//! Application actions and editor sessions.

use crate::document::{
    AxisKind, ChamferParams, Command, CutParams, Document, DraftParams, EndKind, ExtrudeParams, FeatureKind,
    FeatureStatus, FilletParams, HoleParams, Reference, Selection, SketchPlane,
};
use crate::file;
use crate::panels::{Layout, Side};
use crate::state::{AppAction, EditorParams, FeatureEditor, NoteKind, SketchSession};
use crate::theme;
use crate::viewport::dimensions::DimensionTarget;
use crate::viewport::render::{DisplayMode, Picked};

use super::Tab;
use super::presentation::docks::{center_dock, tree_dock};
use super::workspace::SolidDraftApp;

impl SolidDraftApp {
    pub(super) fn apply_actions(&mut self, ctx: &egui::Context) {
        for action in self.state.take_actions() {
            self.apply(ctx, action);
        }
    }

    /// Closes the open editor, restoring the display the editor rolled back.
    pub(super) fn close_editor(&mut self) -> Option<FeatureEditor> {
        let e = self.state.editor.take()?;
        if e.target.is_some() && self.state.doc.rollback != e.saved_rollback {
            self.state.doc.rollback = e.saved_rollback;
            let _ = self.state.doc.rebuild();
        }
        Some(e)
    }

    /// Opens an editor; an existing feature is shown as the body before it.
    pub(super) fn open_editor(&mut self, target: Option<crate::document::FeatureId>, params: EditorParams) {
        self.close_editor();
        self.state.chat.preview = None;
        let saved = self.state.doc.rollback;
        if let Some(id) = target
            && let Some(i) = self.state.doc.index_of(id)
            && self.state.doc.rollback.is_none_or(|r| r > i)
        {
            self.state.doc.rollback = Some(i);
            let _ = self.state.doc.rebuild();
        }
        self.state.editor = Some(FeatureEditor::new(target, params, saved));
        self.state.ribbon_tab = crate::state::RibbonTab::Solid;
    }

    pub(super) fn open_sketch(&mut self, id: crate::document::FeatureId) {
        self.close_editor();
        let Some(f) = self.state.doc.feature(id) else { return };
        let (Some(plane), Some(sk)) = (f.plane, f.sketch()) else {
            self.state.note("The sketch plane could not be resolved; repair it first", NoteKind::Error);
            return;
        };
        let was_orthographic = self.state.viewport.camera.orthographic;
        let session = SketchSession::new(id, plane, sk.sketch.clone(), was_orthographic);
        self.state.sketch = Some(session);
        self.state.doc.selection = Some(Selection::Feature(id));
        self.state.ribbon_tab = crate::state::RibbonTab::Sketch;
        self.state.viewport.camera.orthographic = true;
        self.state.viewport.camera.look_along(glam::Vec3::new(
            plane.normal.x as f32,
            plane.normal.y as f32,
            plane.normal.z as f32,
        ));
        self.state.viewport.camera.fit(self.fit_bounds());
    }

    /// What "zoom to fit" frames: the body, the open sketch, or with nothing
    /// built yet a 200 mm workspace around the sketch origin.
    pub(super) fn fit_bounds(&self) -> Option<(glam::Vec3, glam::Vec3)> {
        let mut acc = self.state.doc.bounds();
        if let Some(s) = &self.state.sketch {
            let f = crate::sketcher::Frame {
                plane: s.plane,
                camera: &self.state.viewport.camera,
                rect: self.state.viewport.rect,
            };
            let (lo, hi) = s
                .sketch
                .bounds()
                .unwrap_or((crate::sketch::P2::new(-100.0, -100.0), crate::sketch::P2::new(100.0, 100.0)));
            let corners = [
                f.world(lo),
                f.world(hi),
                f.world(crate::sketch::P2::new(lo.u, hi.v)),
                f.world(crate::sketch::P2::new(hi.u, lo.v)),
            ];
            for c in corners {
                acc = Some(match acc {
                    Some((a, b)) => (a.min(c), b.max(c)),
                    None => (c, c),
                });
            }
        }
        acc
    }

    pub(super) fn apply(&mut self, ctx: &egui::Context, action: AppAction) {
        match action {
            AppAction::Command(cmd) => {
                if let Some(drag) = self.state.drag_preview.take() {
                    drag.cancel(&mut self.state.doc);
                }
                // A sketch edit from the sketcher must not close the editor state.
                let is_sketch_edit = matches!(cmd, Command::EditSketch { .. });
                match self.state.doc.apply_interactive(cmd) {
                    Ok(Some(id)) => {
                        self.state.doc.selection = Some(Selection::Feature(id));
                        self.state.note("Feature added", NoteKind::Ok);
                    }
                    Ok(None) => {
                        if !is_sketch_edit {
                            if self.state.doc.proof_pending() {
                                self.state.note("Edit requested", NoteKind::Info);
                            } else {
                                self.state.note("Rebuilt", NoteKind::Ok);
                            }
                        }
                    }
                    Err(e) => self.state.note(e, NoteKind::Error),
                }
                self.sync_sketch_session();
            }
            AppAction::Undo if self.state.drawing.is_active() => {
                self.apply(ctx, AppAction::Drawing(crate::drawing::DrawingAction::Undo));
            }
            AppAction::Redo if self.state.drawing.is_active() => {
                self.apply(ctx, AppAction::Drawing(crate::drawing::DrawingAction::Redo));
            }
            AppAction::Undo if self.state.assembly.is_active() => {
                self.apply(ctx, AppAction::Assembly(crate::assembly::AssemblyAction::Undo));
            }
            AppAction::Redo if self.state.assembly.is_active() => {
                self.apply(ctx, AppAction::Assembly(crate::assembly::AssemblyAction::Redo));
            }
            AppAction::Drawing(a) => crate::drawing::apply(&mut self.state, &mut self.dock, ctx, a),
            AppAction::Assembly(crate::assembly::AssemblyAction::FromPart) => {
                // The assembly references the part's file, so the part is saved first.
                if self.state.doc.dirty || self.state.doc.path.is_none() {
                    self.apply(ctx, AppAction::Save);
                }
                crate::assembly::apply(&mut self.state, &mut self.dock, crate::assembly::AssemblyAction::FromPart);
            }
            AppAction::Assembly(a) => crate::assembly::apply(&mut self.state, &mut self.dock, a),
            AppAction::Undo => {
                if let Some(s) = self.state.sketch.as_mut()
                    && let Some(drag) = s.trim_drag.as_mut()
                {
                    if drag.extend.is_some() {
                        s.sketch = drag.before.clone();
                    } else if let Some(cut) = drag.checkpoints.pop() {
                        cut.restore(&mut s.sketch);
                    }
                    return;
                }
                if let Some(drag) = self.state.drag_preview.take() {
                    drag.cancel(&mut self.state.doc);
                }
                self.close_editor();
                match self.state.doc.undo_interactive() {
                    Ok(l) => self.state.note(format!("Undid {l}"), NoteKind::Info),
                    Err(e) => self.state.note(e, NoteKind::Error),
                }
                self.sync_sketch_session();
            }
            AppAction::Redo => {
                if let Some(s) = self.state.sketch.as_mut()
                    && s.trim_drag.is_some()
                {
                    s.say("Finish or cancel the trim drag before redoing");
                    return;
                }
                if let Some(drag) = self.state.drag_preview.take() {
                    drag.cancel(&mut self.state.doc);
                }
                self.close_editor();
                match self.state.doc.redo_interactive() {
                    Ok(l) => self.state.note(format!("Redid {l}"), NoteKind::Info),
                    Err(e) => self.state.note(e, NoteKind::Error),
                }
                self.sync_sketch_session();
            }
            AppAction::StartSketch => {
                if self.state.in_sketch() {
                    return;
                }
                let plane = match self.state.doc.selection {
                    Some(Selection::Feature(id))
                        if matches!(
                            self.state.doc.feature(id).map(|f| &f.kind),
                            Some(FeatureKind::ReferencePlane(_))
                        ) =>
                    {
                        Some(SketchPlane::Feature { feature: id, offset: 0.0 })
                    }
                    Some(Selection::Face(f)) => self.state.doc.face_ref(f).map(|face| SketchPlane::Face { face }),
                    Some(Selection::Reference(Reference::Plane(plane))) => {
                        Some(SketchPlane::Reference { plane, offset: 0.0 })
                    }
                    _ => None,
                };
                match plane {
                    Some(plane) => self.apply(ctx, AppAction::StartSketchOn(plane)),
                    None => {
                        self.open_editor(None, EditorParams::PickPlane);
                        self.state.origin_open = true;
                        if self.state.frame_layout == crate::settings::FrameLayout::EditSuite {
                            self.state.ribbon_tab = crate::state::RibbonTab::Sketch;
                            self.state.note("Choose an origin plane or click a flat face", NoteKind::Info);
                        } else {
                            self.state.note("Choose a plane in the tree or click a planar face", NoteKind::Info);
                        }
                    }
                }
            }
            AppAction::StartSketchOn(plane) => {
                let tool = self.close_editor().and_then(|editor| editor.pending_sketch_tool);
                match self.state.doc.apply(Command::AddSketch { plane }) {
                    Ok(Some(id)) => {
                        self.open_sketch(id);
                        if let Some(tool) = tool
                            && let Some(session) = &mut self.state.sketch
                        {
                            session.set_tool(tool);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => self.state.note(e, NoteKind::Error),
                }
            }
            AppAction::EditSketch(id) => {
                if self.state.sketch.as_ref().is_some_and(|s| s.feature == id) {
                    return;
                }
                self.apply(ctx, AppAction::ExitSketch);
                self.open_sketch(id);
            }
            AppAction::ExitSketch => {
                let Some(session) = self.state.sketch.take() else { return };
                self.state.viewport.camera.orthographic = session.was_orthographic;
                self.state.ribbon_tab = crate::state::RibbonTab::Solid;
                let id = session.feature;
                let empty = self.state.doc.feature(id).and_then(|f| f.sketch()).is_some_and(|s| s.sketch.is_empty());
                if empty && self.state.doc.consumer_of(id).is_none() {
                    // An empty new sketch leaves nothing behind.
                    let _ = self.state.doc.apply(Command::DeleteFeature(id));
                    self.state.doc.selection = None;
                    return;
                }
                self.state.doc.selection = Some(Selection::Feature(id));
            }
            AppAction::StartExtrude => {
                if self.state.in_sketch() {
                    return;
                }
                let sketch = self.state.selected_sketch().filter(|s| self.state.doc.consumer_of(*s).is_none());
                let params = match sketch {
                    Some(id) => ExtrudeParams { sketch: Some(id), depth: 10.0, ..Default::default() },
                    None => ExtrudeParams::default(),
                };
                self.open_editor(None, EditorParams::Extrude(params));
            }
            AppAction::StartLoft => {
                if self.state.in_sketch() {
                    return;
                }
                let profiles = match self.state.doc.selection {
                    Some(Selection::Feature(id)) if self.state.doc.sketch_of(id).is_some() => {
                        vec![soliddraft_model::LoftProfile::Sketch { feature: soliddraft_model::FeatureId(id as u32) }]
                    }
                    Some(Selection::Face(face)) => self
                        .state
                        .doc
                        .any_face_ref(face)
                        .map(|face| vec![soliddraft_model::LoftProfile::Face { face }])
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                self.open_editor(
                    None,
                    EditorParams::Loft(soliddraft_model::LoftFeature {
                        profiles,
                        shape: soliddraft_kernel_api::LoftShape::Ruled,
                    }),
                );
            }
            AppAction::StartSweep => {
                if self.state.in_sketch() {
                    return;
                }
                match self.state.selected_sketch().filter(|id| self.state.doc.consumer_of(*id).is_none()) {
                    Some(id) => {
                        let normal = self
                            .state
                            .doc
                            .feature(id)
                            .and_then(|f| f.plane)
                            .map(|p| p.normal)
                            .unwrap_or(soliddraft_kernel_api::Vec3::Z);
                        // Arcs turn about model axes, so the path starts on the profile.
                        let start = self.state.doc.sweep_anchor(id).map_or([0.0; 3], |(start, _)| start);
                        let end = soliddraft_kernel_api::Vec3::from(start) + normal * 10.0;
                        self.open_editor(
                            None,
                            EditorParams::Sweep(soliddraft_model::SweepFeature {
                                sketch: soliddraft_model::FeatureId(id as u32),
                                path: soliddraft_kernel_api::Sweep::line(start, end.into(), "travel"),
                            }),
                        );
                    }
                    None => self.state.note("Select a closed polygon sketch to sweep", NoteKind::Info),
                }
            }
            AppAction::StartRevolve => {
                if self.state.in_sketch() {
                    return;
                }
                match self.state.selected_sketch().filter(|id| self.state.doc.consumer_of(*id).is_none()) {
                    Some(id) => self.open_editor(
                        None,
                        EditorParams::Revolve(soliddraft_model::RevolveFeature {
                            sketch: soliddraft_model::FeatureId(id as u32),
                            axis_origin: [0.0; 3],
                            axis_direction: [0.0, 1.0, 0.0],
                            angle_deg: 360.0,
                        }),
                    ),
                    None => self.state.note("Select a closed meridian sketch to revolve", NoteKind::Info),
                }
            }
            AppAction::StartCut => {
                if self.state.in_sketch() {
                    return;
                }
                match self.state.selected_sketch() {
                    Some(id) if self.state.doc.consumer_of(id).is_none() => {
                        self.open_editor(None, EditorParams::Cut(CutParams::new(id)));
                    }
                    _ => self.state.note("Select a sketch to cut with, or sketch on a face first", NoteKind::Info),
                }
            }
            AppAction::StartFillet | AppAction::StartChamfer => {
                let chamfer = matches!(action, AppAction::StartChamfer);
                let params = if chamfer {
                    EditorParams::Chamfer(ChamferParams { edges: Vec::new(), distance: 2.0 })
                } else {
                    EditorParams::Fillet(FilletParams { edges: Vec::new(), radius: 4.0 })
                };
                self.open_editor(None, params);
                // A picked edge or face is the panel's first pick, as if made with it open.
                if let Some(pick @ (Selection::Edge(_) | Selection::Face(_) | Selection::Vertex(_))) =
                    self.state.doc.selection
                    && let Some(editor) = self.state.editor.as_mut()
                {
                    crate::fillet_collect::collect(editor, &self.state.doc, pick);
                }
                self.state.doc.selection = None;
            }
            AppAction::StartHole => {
                let face = match self.state.doc.selection {
                    Some(Selection::Face(f)) => self.state.doc.face_ref(f),
                    _ => None,
                };
                self.open_editor(None, EditorParams::Hole(HoleParams { face, ..Default::default() }));
                self.state.doc.selection = None;
            }
            AppAction::StartPattern { circular } => {
                let layout = if circular {
                    soliddraft_model::PatternSpec::Circular {
                        axis_origin: soliddraft_kernel_api::Vec3::ZERO,
                        axis_direction: soliddraft_kernel_api::Vec3::Z,
                        count: 4,
                        angle: 90.0,
                    }
                } else {
                    soliddraft_model::PatternSpec::Linear {
                        direction: soliddraft_kernel_api::Vec3::X,
                        spacing: 20.0,
                        count: 3,
                    }
                };
                self.open_editor(None, EditorParams::Pattern(soliddraft_model::PatternFeature { layout }));
            }
            AppAction::StartReferencePlane => {
                let source = match self.state.doc.selection {
                    Some(Selection::Feature(id))
                        if matches!(
                            self.state.doc.feature(id).map(|f| &f.kind),
                            Some(FeatureKind::ReferencePlane(_))
                        ) =>
                    {
                        soliddraft_model::SketchPlane::Feature {
                            feature: soliddraft_model::FeatureId(id as u32),
                            offset: 10.0,
                        }
                    }
                    Some(Selection::Face(id)) => match self.state.doc.face_ref(id) {
                        Some(face) => soliddraft_model::SketchPlane::FaceOffset { face, offset: 10.0 },
                        None => {
                            self.state.note("Choose a planar face or reference plane", NoteKind::Info);
                            return;
                        }
                    },
                    Some(Selection::Reference(Reference::Plane(plane))) => {
                        soliddraft_model::SketchPlane::Datum { datum: plane.to_model(), offset: 10.0 }
                    }
                    _ => {
                        soliddraft_model::SketchPlane::Datum { datum: soliddraft_model::DatumPlane::Top, offset: 10.0 }
                    }
                };
                self.open_editor(None, EditorParams::ReferencePlane(source));
            }
            AppAction::StartMirror => {
                let mut plane = match self.state.doc.selection.and_then(|s| match s {
                    Selection::Face(f) => self.state.doc.face_ref(f),
                    _ => None,
                }) {
                    Some(face) => soliddraft_model::PlaneRef::Face { face },
                    None => {
                        soliddraft_model::PlaneRef::Datum { datum: soliddraft_model::DatumPlane::Right, offset: 0.0 }
                    }
                };
                if let Some(Selection::Feature(id)) = self.state.doc.selection
                    && matches!(self.state.doc.feature(id).map(|f| &f.kind), Some(FeatureKind::ReferencePlane(_)))
                {
                    plane = soliddraft_model::PlaneRef::Feature { feature: soliddraft_model::FeatureId(id as u32) };
                }
                self.open_editor(
                    None,
                    EditorParams::Mirror(soliddraft_model::MirrorFeature { plane, keep_original: true }),
                );
                self.state.doc.selection = None;
            }
            AppAction::StartLattice => {
                self.open_editor(None, EditorParams::Lattice(soliddraft_model::LatticeFeature::default()));
                self.state.doc.selection = None;
            }
            AppAction::StartGyroid => {
                self.open_editor(None, EditorParams::Gyroid(soliddraft_model::GyroidFeature::default()));
                self.state.doc.selection = None;
            }
            AppAction::StartShell => {
                let open_faces = match self.state.doc.selection {
                    Some(Selection::Face(face)) => self.state.doc.any_face_ref(face).into_iter().collect(),
                    _ => Vec::new(),
                };
                self.open_editor(
                    None,
                    EditorParams::Shell(soliddraft_model::ShellFeature {
                        open_faces: soliddraft_model::ShellOpenings::Faces(open_faces),
                        thickness: 2.0,
                    }),
                );
                self.state.doc.selection = None;
            }
            AppAction::StartDraft => {
                let faces = match self.state.doc.selection {
                    Some(Selection::Face(f)) => self.state.doc.any_face_ref(f).into_iter().collect(),
                    _ => Vec::new(),
                };
                self.open_editor(None, EditorParams::Draft(DraftParams { faces, ..Default::default() }));
                self.state.doc.selection = None;
            }
            AppAction::StartRepair(id) => {
                let Some(FeatureStatus::NeedsRepair(items)) = self.state.doc.feature(id).map(|f| f.status.clone())
                else {
                    return;
                };
                let n = items.len();
                self.open_editor(Some(id), EditorParams::Repair { items, chosen: vec![None; n], active: 0 });
                self.state.note(
                    "Choose a replacement plane in the tree, or an edge or planar face in the viewport",
                    NoteKind::Info,
                );
            }
            AppAction::EditFeature(id) => {
                let Some(f) = self.state.doc.feature(id) else { return };
                let params = match &f.kind {
                    FeatureKind::Sketch(_) => {
                        self.apply(ctx, AppAction::EditSketch(id));
                        return;
                    }
                    FeatureKind::Extrude(p) => EditorParams::Extrude(p.clone()),
                    FeatureKind::Revolve(r) => EditorParams::Revolve(r.clone()),
                    FeatureKind::Sweep(s) => EditorParams::Sweep(s.clone()),
                    FeatureKind::Loft(l) => EditorParams::Loft(l.clone()),
                    FeatureKind::Cut(p) => EditorParams::Cut(p.clone()),
                    FeatureKind::Fillet(p) => EditorParams::Fillet(p.clone()),
                    FeatureKind::Chamfer(p) => EditorParams::Chamfer(p.clone()),
                    FeatureKind::Hole(h) => EditorParams::Hole(HoleParams {
                        face: Some(h.face.clone()),
                        points: h.points.clone(),
                        diameter: h.diameter,
                        depth: h.depth,
                        tip: h.tip,
                        head: h.head,
                    }),
                    FeatureKind::Pattern(p) => EditorParams::Pattern(p.clone()),
                    FeatureKind::ReferencePlane(p) => EditorParams::ReferencePlane(p.clone()),
                    FeatureKind::Mirror(m) => EditorParams::Mirror(m.clone()),
                    FeatureKind::Shell(s) => EditorParams::Shell(s.clone()),
                    FeatureKind::Draft(d) => EditorParams::Draft(DraftParams {
                        faces: d.faces.clone(),
                        neutral: d.neutral.clone(),
                        angle: d.angle,
                        reverse: d.reverse,
                        picking_neutral: false,
                    }),
                    // An import has no editor; its repair questions have their own dialog.
                    FeatureKind::ImportStep(_) => return,
                    // The lattice's controls arrive with its editor slice.
                    FeatureKind::Lattice(l) => EditorParams::Lattice(*l),
                    FeatureKind::Gyroid(g) => EditorParams::Gyroid(*g),
                };
                if self.state.in_sketch() {
                    self.apply(ctx, AppAction::ExitSketch);
                }
                self.open_editor(Some(id), params);
                self.state.doc.selection = Some(Selection::Feature(id));
            }
            AppAction::CommitEditor => {
                // A refused size stays open for correction: OK cannot apply the last
                // accepted one in its place.
                if let Some(why) = self.state.editor.as_ref().and_then(|e| e.feedback.size.clone()) {
                    self.state.note(format!("Not applied: {why}"), NoteKind::Error);
                    return;
                }
                if let Some(why) = self
                    .state
                    .editor
                    .as_ref()
                    .filter(|e| matches!(e.params, EditorParams::Loft(_)))
                    .and_then(|e| e.error.clone())
                {
                    self.state.note(why, NoteKind::Error);
                    return;
                }
                let Some(e) = self.close_editor() else { return };
                let cmd = match (e.target, e.params) {
                    (target, EditorParams::Loft(l)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Loft(l) },
                        None => Command::AddFeature(FeatureKind::Loft(l)),
                    },
                    (target, EditorParams::Sweep(s)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Sweep(s) },
                        None => Command::AddFeature(FeatureKind::Sweep(s)),
                    },
                    (target, EditorParams::Revolve(r)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Revolve(r) },
                        None => Command::AddFeature(FeatureKind::Revolve(r)),
                    },
                    (Some(id), EditorParams::Extrude(p)) => Command::EditExtrude { id, params: p },
                    (None, EditorParams::Extrude(p)) => Command::AddExtrude { params: p, via_assistant: false },
                    (Some(id), EditorParams::Cut(p)) => Command::EditCut { id, params: p },
                    (None, EditorParams::Cut(p)) => Command::AddCut { params: p },
                    (Some(id), EditorParams::Fillet(p)) => Command::EditFillet { id, params: p },
                    (None, EditorParams::Fillet(p)) => Command::AddFillet { params: p },
                    (Some(id), EditorParams::Chamfer(p)) => Command::EditChamfer { id, params: p },
                    (None, EditorParams::Chamfer(p)) => Command::AddChamfer { params: p },
                    (target, EditorParams::Hole(h)) => match h.to_model() {
                        Ok(h) => match target {
                            Some(id) => Command::EditFeature { id, kind: FeatureKind::Hole(h) },
                            None => Command::AddFeature(FeatureKind::Hole(h)),
                        },
                        Err(e) => {
                            self.state.note(e, NoteKind::Error);
                            return;
                        }
                    },
                    (target, EditorParams::Pattern(p)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Pattern(p) },
                        None => Command::AddFeature(FeatureKind::Pattern(p)),
                    },
                    (target, EditorParams::ReferencePlane(p)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::ReferencePlane(p) },
                        None => Command::AddFeature(FeatureKind::ReferencePlane(p)),
                    },
                    (target, EditorParams::Mirror(m)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Mirror(m) },
                        None => Command::AddFeature(FeatureKind::Mirror(m)),
                    },
                    (target, EditorParams::Lattice(l)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Lattice(l) },
                        None => Command::AddFeature(FeatureKind::Lattice(l)),
                    },
                    (target, EditorParams::Gyroid(g)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Gyroid(g) },
                        None => Command::AddFeature(FeatureKind::Gyroid(g)),
                    },
                    (target, EditorParams::Shell(s)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Shell(s) },
                        None => Command::AddFeature(FeatureKind::Shell(s)),
                    },
                    (target, EditorParams::Draft(d)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Draft(d.to_model()) },
                        None => Command::AddFeature(FeatureKind::Draft(d.to_model())),
                    },
                    (_, EditorParams::PickPlane) => return,
                    (Some(id), EditorParams::Repair { items, chosen, .. }) => {
                        let mut applied = 0;
                        for (item, choice) in items.iter().zip(chosen) {
                            if let Some(r) = choice {
                                match self.state.doc.apply(Command::Repair { id, index: item.index, replacement: r }) {
                                    Ok(_) => applied += 1,
                                    Err(e) => self.state.note(e, NoteKind::Error),
                                }
                            }
                        }
                        let status = self.state.doc.feature(id).map(|f| f.status.clone());
                        match status {
                            Some(FeatureStatus::Ok) => self.state.note(
                                format!("Repaired {applied} reference{}", if applied == 1 { "" } else { "s" }),
                                NoteKind::Ok,
                            ),
                            Some(FeatureStatus::NeedsRepair(_)) => {
                                self.state.note("Some references are still missing", NoteKind::Error)
                            }
                            Some(FeatureStatus::Error(e)) => self.state.note(e, NoteKind::Error),
                            _ => {}
                        }
                        return;
                    }
                    (None, EditorParams::Repair { .. }) => return,
                };
                self.apply(ctx, AppAction::Command(cmd));
            }
            AppAction::CancelEditor => {
                self.close_editor();
            }
            AppAction::DragDimension(step) => self.state.drag_dimension(step),
            AppAction::EditDimension(target) => match target {
                DimensionTarget::Feature(id) => self.apply(ctx, AppAction::EditFeature(id)),
                DimensionTarget::Sketch { sketch, constraint } => {
                    self.apply(ctx, AppAction::EditSketch(sketch));
                    if crate::sketch_values::links(self.state.doc.model(), sketch).contains_key(&constraint) {
                        if let Some(s) = self.state.sketch.as_mut() {
                            s.selection = vec![crate::state::SketchSel::Constraint(constraint)];
                        }
                        crate::sketch_values::open(&mut self.state);
                        return;
                    }
                    // The sketch opens with the value editor on the dimension, as a
                    // double-click on it inside the sketch does.
                    if let Some(s) = self.state.sketch.as_mut().filter(|s| s.feature == sketch)
                        && let Some(value) = s.sketch.constraint(constraint).and_then(|c| c.value())
                    {
                        s.dim_edit = Some(crate::state::DimEdit::new(constraint, value));
                    } else if let Some(s) = self.state.sketch.as_mut().filter(|s| s.feature == sketch)
                        && s.sketch.constraint(constraint).is_some_and(|c| c.is_reference())
                    {
                        s.selection = vec![crate::state::SketchSel::Constraint(constraint)];
                        s.say("Reference dimensions are read-only; change to driving to edit");
                    }
                }
            },
            AppAction::Dimension(action) => self.state.dimension(action),
            AppAction::Select(sel) => {
                // Editors that collect picks take the click instead of selecting.
                if let Some(e) = self.state.editor.as_mut() {
                    match (&mut e.params, sel) {
                        (
                            EditorParams::Fillet(_) | EditorParams::Chamfer(_),
                            Some(pick @ (Selection::Edge(_) | Selection::Face(_) | Selection::Vertex(_))),
                        ) => {
                            crate::fillet_collect::collect(e, &self.state.doc, pick);
                            return;
                        }
                        (EditorParams::Hole(h), Some(Selection::Face(f))) => {
                            match self.state.doc.face_ref(f) {
                                Some(r) => h.face = Some(r),
                                None => e.error = Some("Holes start on a planar face".into()),
                            }
                            return;
                        }
                        (EditorParams::ReferencePlane(p), Some(Selection::Reference(Reference::Plane(plane)))) => {
                            *p = soliddraft_model::SketchPlane::Datum { datum: plane.to_model(), offset: p.offset() };
                        }
                        (EditorParams::Mirror(m), Some(Selection::Reference(Reference::Plane(plane)))) => {
                            m.plane = soliddraft_model::PlaneRef::Datum { datum: plane.to_model(), offset: 0.0 };
                        }
                        (EditorParams::ReferencePlane(p), Some(Selection::Face(f))) => {
                            if let Some(face) = self.state.doc.face_ref(f) {
                                *p = soliddraft_model::SketchPlane::FaceOffset { face, offset: p.offset() };
                            }
                        }
                        (EditorParams::ReferencePlane(p), Some(Selection::Feature(id)))
                            if self
                                .state
                                .doc
                                .model()
                                .reference_plane(soliddraft_model::FeatureId(id as u32))
                                .is_some() =>
                        {
                            *p = soliddraft_model::SketchPlane::Feature {
                                feature: soliddraft_model::FeatureId(id as u32),
                                offset: p.offset(),
                            };
                        }
                        (EditorParams::Mirror(m), Some(Selection::Feature(id)))
                            if self
                                .state
                                .doc
                                .model()
                                .reference_plane(soliddraft_model::FeatureId(id as u32))
                                .is_some() =>
                        {
                            m.plane =
                                soliddraft_model::PlaneRef::Feature { feature: soliddraft_model::FeatureId(id as u32) };
                        }
                        (EditorParams::PickPlane, Some(Selection::Feature(id)))
                            if self
                                .state
                                .doc
                                .model()
                                .reference_plane(soliddraft_model::FeatureId(id as u32))
                                .is_some() =>
                        {
                            self.apply(
                                ctx,
                                AppAction::StartSketchOn(SketchPlane::Feature { feature: id, offset: 0.0 }),
                            );
                        }
                        (EditorParams::Loft(l), Some(Selection::Feature(id)))
                            if self.state.doc.sketch_of(id).is_some() =>
                        {
                            let source = soliddraft_model::LoftProfile::Sketch {
                                feature: soliddraft_model::FeatureId(id as u32),
                            };
                            if !l.profiles.contains(&source) {
                                l.profiles.push(source);
                            }
                            return;
                        }
                        (EditorParams::Loft(l), Some(Selection::Face(f))) => {
                            if let Some(face) = self.state.doc.face_ref(f) {
                                let source = soliddraft_model::LoftProfile::Face { face };
                                if !l.profiles.contains(&source) {
                                    l.profiles.push(source);
                                }
                            } else {
                                e.error = Some("Loft needs a planar face or a closed sketch".into());
                            }
                            return;
                        }
                        (EditorParams::Mirror(m), Some(Selection::Face(f))) => {
                            match self.state.doc.face_ref(f) {
                                Some(face) => m.plane = soliddraft_model::PlaneRef::Face { face },
                                None => e.error = Some("Mirror in a planar face or a reference plane".into()),
                            }
                            return;
                        }
                        (EditorParams::Shell(s), Some(Selection::Face(f))) => {
                            if let soliddraft_model::ShellOpenings::Faces(faces) = &mut s.open_faces
                                && let Some(reference) = self.state.doc.any_face_ref(f)
                            {
                                if let Some(i) = faces.iter().position(|face| face == &reference) {
                                    faces.remove(i);
                                } else {
                                    faces.push(reference);
                                }
                            }
                            return;
                        }
                        (EditorParams::Draft(d), Some(Selection::Face(f))) => {
                            if d.picking_neutral {
                                match self.state.doc.face_ref(f) {
                                    Some(face) => {
                                        d.neutral = soliddraft_model::PlaneRef::Face { face };
                                        d.picking_neutral = false;
                                    }
                                    None => e.error = Some("The neutral plane must be planar".into()),
                                }
                            } else if let Some(r) = self.state.doc.any_face_ref(f) {
                                if let Some(i) = d.faces.iter().position(|x| x.label == r.label) {
                                    d.faces.remove(i);
                                } else {
                                    d.faces.push(r);
                                }
                            }
                            return;
                        }
                        (
                            EditorParams::Extrude(ExtrudeParams { end: EndKind::UpToFace(face), .. }),
                            Some(Selection::Face(f)),
                        )
                        | (
                            EditorParams::Cut(CutParams { up_to: Some(EndKind::UpToFace(face)), .. }),
                            Some(Selection::Face(f)),
                        ) => {
                            *face = self.state.doc.any_face_ref(f);
                            return;
                        }
                        (EditorParams::PickPlane, Some(Selection::Face(f))) => {
                            match self.state.doc.face_ref(f) {
                                Some(r) => self.apply(ctx, AppAction::StartSketchOn(SketchPlane::Face { face: r })),
                                None => self.state.note("Sketches need a planar face", NoteKind::Error),
                            }
                            return;
                        }
                        (EditorParams::PickPlane, Some(Selection::Reference(Reference::Plane(plane)))) => {
                            self.apply(ctx, AppAction::StartSketchOn(SketchPlane::Reference { plane, offset: 0.0 }));
                            return;
                        }
                        // The axis is selected as well, so the origin axes show which one.
                        (
                            params @ (EditorParams::Revolve(_) | EditorParams::Pattern(_)),
                            Some(Selection::Reference(Reference::Axis(axis))),
                        ) => use_axis(params, axis),
                        (EditorParams::Repair { items, chosen, active }, Some(s)) => {
                            let wanted = items.get(*active).map(|i| i.kind);
                            let r = match (wanted, s) {
                                (Some(crate::document::RefKind::Edge), Selection::Edge(edge)) => {
                                    self.state.doc.edge_ref(edge).map(crate::document::EntityRef::Edge)
                                }
                                (Some(crate::document::RefKind::Plane), Selection::Feature(id))
                                    if self
                                        .state
                                        .doc
                                        .model()
                                        .reference_plane(soliddraft_model::FeatureId(id as u32))
                                        .is_some() =>
                                {
                                    Some(crate::document::EntityRef::Plane {
                                        plane: soliddraft_model::SketchPlane::Feature {
                                            feature: soliddraft_model::FeatureId(id as u32),
                                            offset: 0.0,
                                        },
                                        label: self.state.doc.feature(id).unwrap().name.clone(),
                                    })
                                }
                                (Some(crate::document::RefKind::Plane), Selection::Reference(Reference::Plane(p))) => {
                                    Some(crate::document::EntityRef::Plane {
                                        plane: soliddraft_model::SketchPlane::Datum {
                                            datum: p.to_model(),
                                            offset: 0.0,
                                        },
                                        label: self.state.doc.model().datum_name(p.to_model()).to_owned(),
                                    })
                                }
                                (
                                    Some(crate::document::RefKind::Face | crate::document::RefKind::Plane),
                                    Selection::Face(face),
                                ) => self.state.doc.face_ref(face).map(crate::document::EntityRef::Face),
                                _ => None,
                            };
                            match r {
                                Some(r) => {
                                    if let Some(slot) = chosen.get_mut(*active) {
                                        *slot = Some(r);
                                    }
                                    if let Some(next) = chosen.iter().position(|c| c.is_none()) {
                                        *active = next;
                                    }
                                    e.error = None;
                                }
                                None => {
                                    e.error = Some(match wanted {
                                        Some(crate::document::RefKind::Edge) => {
                                            "Pick an edge for this reference".into()
                                        }
                                        Some(crate::document::RefKind::Plane) => {
                                            "Choose a reference plane or planar face".into()
                                        }
                                        _ => "Pick a planar face for this reference".into(),
                                    })
                                }
                            }
                            return;
                        }
                        _ => {}
                    }
                }
                self.state.doc.selection = sel;
                self.state.rename = None;
            }
            AppAction::MeasureEntity => self.state.measure.open = !self.state.measure.open,
            AppAction::MeasureMassProperties => {
                let feature = self.state.doc.selection.and_then(|sel| self.state.doc.selection_feature(sel));
                if let Some(id) = feature {
                    match self.state.mass.request(&self.state.doc, id, ctx) {
                        Ok(()) => self.state.doc.selection = Some(Selection::Feature(id)),
                        Err(message) => self.state.note(message, NoteKind::Error),
                    }
                } else {
                    self.state.note("Select a solid feature with measurable properties", NoteKind::Error);
                }
            }
            AppAction::FinishSpline => {
                let mut actions = Vec::new();
                if let Some(s) = self.state.sketch.as_mut() {
                    crate::sketcher::finish_spline(s, &mut actions);
                }
                for action in actions {
                    self.apply(ctx, action);
                }
            }
            AppAction::SketchTool(tool) => {
                if !self.state.in_sketch()
                    && self.state.frame_layout == crate::settings::FrameLayout::EditSuite
                    && crate::commands::Context::of(&self.state).part()
                {
                    self.apply(ctx, AppAction::StartSketch);
                    if let Some(editor) = &mut self.state.editor
                        && matches!(editor.params, EditorParams::PickPlane)
                    {
                        editor.pending_sketch_tool = Some(tool);
                    }
                }
                if let Some(s) = self.state.sketch.as_mut() {
                    s.set_tool(tool);
                }
            }
            AppAction::SketchRelations(open) => {
                if let Some(s) = self.state.sketch.as_mut() {
                    s.relations_open = open;
                    if open {
                        self.state.ribbon_tab = crate::state::RibbonTab::Sketch;
                    }
                }
            }
            AppAction::SketchShading(show) => {
                if let Some(s) = self.state.sketch.as_mut() {
                    s.show_regions = show;
                }
            }
            AppAction::SketchDimensions(show) => {
                if let Some(s) = self.state.sketch.as_mut() {
                    s.show_dimensions = show;
                    if !show {
                        s.selection.retain(|sel| {
                            !matches!(sel, crate::state::SketchSel::Constraint(id)
                            if s.sketch.constraint(*id).is_some_and(|c| c.is_dimension()))
                        });
                        if matches!(s.drag, Some(crate::state::SketchDrag::Label { .. })) {
                            s.drag = None;
                        }
                        s.dim_edit = None;
                    }
                }
            }
            AppAction::SketchGridSnap(on) => {
                if let Some(s) = self.state.sketch.as_mut() {
                    s.grid_snap = on;
                }
            }
            AppAction::SketchDiagnostics(open) => {
                if let Some(s) = self.state.sketch.as_mut() {
                    if open {
                        s.set_tool(crate::state::Tool::Select);
                        s.selection.clear();
                        s.diagnostics = Some(crate::sketch_diagnostics::Inspection::new(&s.sketch));
                        self.state.ribbon_tab = crate::state::RibbonTab::Sketch;
                    } else {
                        s.diagnostics = None;
                    }
                }
            }
            AppAction::CheckSketchFeature => {
                self.apply(ctx, AppAction::SketchDiagnostics(true));
                if let Some(report) = self.state.sketch.as_mut().and_then(|s| s.diagnostics.as_mut()) {
                    report.usage = crate::sketch_diagnostics::Usage::Boss;
                }
            }
            AppAction::LocateSketchDiagnostic => {
                let bounds = self.state.sketch.as_ref().and_then(|s| {
                    let issue = crate::sketch_diagnostics::selected(s)?;
                    let frame = crate::sketcher::Frame {
                        plane: s.plane,
                        camera: &self.state.viewport.camera,
                        rect: self.state.viewport.rect,
                    };
                    let owners = issue
                        .relation
                        .into_iter()
                        .flat_map(|_| issue.entities.iter())
                        .filter_map(|id| s.sketch.entity(*id))
                        .flat_map(|e| e.polyline(8));
                    let mut points =
                        issue.locations.iter().copied().chain(owners).map(|p| frame.world(p)).filter(|p| p.is_finite());
                    let first = points.next()?;
                    Some(points.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
                });
                if let Some((lo, hi)) =
                    bounds.filter(|(lo, hi)| (*lo + *hi).is_finite() && (*hi - *lo).length().is_finite())
                {
                    self.state.viewport.camera.fit(Some((lo, hi)));
                } else if let Some(s) = self.state.sketch.as_mut() {
                    s.say("Fault coordinates cannot be displayed at this scale");
                }
            }
            AppAction::ToggleConstruction => {
                if let Some(s) = self.state.sketch.as_mut() {
                    let ids = s.selected_entities();
                    if ids.is_empty() {
                        s.say("Select entities to toggle construction");
                        return;
                    }
                    let all = ids.iter().all(|e| s.sketch.entity(*e).is_some_and(|e| e.construction));
                    for id in ids {
                        if let Some(e) = s.sketch.entity_mut(id) {
                            e.construction = !all;
                        }
                    }
                    crate::sketch::solve::solve(&mut s.sketch, None, true);
                    let cmd = Command::EditSketch { id: s.feature, sketch: s.sketch.clone() };
                    self.apply(ctx, AppAction::Command(cmd));
                }
            }
            AppAction::DeleteSelection => {
                if let Some(s) = self.state.sketch.as_mut() {
                    if s.tool.spline_mode().is_some() && !s.pending.is_empty() {
                        s.pending.pop();
                        s.snaps.clear();
                        return;
                    }
                    if s.selection.is_empty() {
                        return;
                    }
                    for sel in s.selection.clone() {
                        match sel {
                            crate::state::SketchSel::Entity(e) => s.sketch.remove_entity(e),
                            crate::state::SketchSel::Constraint(c) => s.sketch.remove_constraint(c),
                            crate::state::SketchSel::Point(_) => {}
                        }
                    }
                    s.selection.clear();
                    crate::sketch::solve::solve(&mut s.sketch, None, true);
                    let cmd = Command::EditSketch { id: s.feature, sketch: s.sketch.clone() };
                    self.apply(ctx, AppAction::Command(cmd));
                } else if self.state.editor.is_none()
                    && let Some(id) = self.state.selected_feature()
                {
                    self.apply(ctx, AppAction::Command(Command::DeleteFeature(id)));
                }
            }
            AppAction::RenameSelected => {
                if let Some(id) = self.state.selected_feature()
                    && let Some(f) = self.state.doc.feature(id)
                {
                    let rename = Some(crate::state::Rename { feature: id, text: f.name.clone(), focus: true });
                    if self.state.frame_layout == crate::settings::FrameLayout::EditSuite
                        && !self.state.drawing.is_active()
                        && !self.state.assembly.is_active()
                    {
                        self.state.timeline.rename = rename;
                    } else {
                        self.state.rename = rename;
                    }
                }
            }
            AppAction::SetView(v) => self.state.viewport.camera.set_view(v),
            AppAction::NormalTo => {
                let normal = match (&self.state.sketch, self.state.doc.selection) {
                    (Some(s), _) => Some(s.plane.normal),
                    (None, Some(Selection::Face(f))) => self.state.doc.face_plane(f).map(|p| p.normal),
                    (None, _) => {
                        self.state.selected_sketch().and_then(|id| self.state.doc.feature(id)?.plane).map(|p| p.normal)
                    }
                };
                match normal {
                    Some(n) => {
                        self.state.viewport.camera.look_along(glam::Vec3::new(n.x as f32, n.y as f32, n.z as f32))
                    }
                    None => self.state.note("Select a planar face or a sketch first", NoteKind::Info),
                }
            }
            AppAction::ZoomFit => {
                let bounds = self.fit_bounds();
                self.state.viewport.camera.fit(bounds);
            }
            AppAction::SetDisplay(d) => self.state.viewport.display = d,
            AppAction::SetPreset(p) => {
                let viewport = &mut self.state.viewport;
                // Realistic leaves edges off; switching to or from it moves the shaded
                // modes along, and a wireframe stays a wireframe.
                if p.shows_edges() != viewport.preset.shows_edges() && viewport.display != DisplayMode::Wireframe {
                    viewport.display = if p.shows_edges() { DisplayMode::ShadedWithEdges } else { DisplayMode::Shaded };
                }
                viewport.preset = p;
            }
            AppAction::SetBodyVisible(show) => {
                if crate::tree_visibility::visible(&self.state).body != show {
                    let feature = self
                        .state
                        .doc
                        .features
                        .iter()
                        .find(|feature| !feature.is_sketch() && !matches!(feature.kind, FeatureKind::ReferencePlane(_)))
                        .map(|feature| feature.id);
                    if let Some(feature) = feature {
                        crate::tree_visibility::toggle(&mut self.state, crate::document::ViewItem::Feature(feature));
                    }
                }
            }
            AppAction::SetReferenceDisplay(display) => {
                let _ = self.state.set_reference_display(display);
            }
            AppAction::SketchRelationSet(kinds) => {
                if let Some(sketch) = self.state.sketch.as_mut() {
                    let action = crate::sketcher::add_relations(sketch, &kinds);
                    self.apply(ctx, action);
                }
            }
            AppAction::SetScene(scene) => {
                if self.state.frame_layout == crate::settings::FrameLayout::EditSuite {
                    self.state.viewport.scene = Some(scene);
                    if let Err(e) = crate::settings::save_scene(scene) {
                        self.state.note(format!("Scene changed, but could not be saved: {e}"), NoteKind::Error);
                    }
                }
            }
            AppAction::SetTheme(mode) => {
                theme::set_mode(ctx, mode);
                if let Err(e) = crate::settings::save_appearance(&crate::settings::Appearance { theme: mode }) {
                    log::warn!("cannot save the appearance: {e}");
                }
            }
            AppAction::ToggleProjection => {
                self.state.viewport.camera.orthographic = !self.state.viewport.camera.orthographic
            }
            AppAction::PreviewProposal(id) => {
                self.close_editor();
                self.state.preview_proposal(&id);
            }
            AppAction::ApplyProposal(id) => {
                self.close_editor();
                self.state.apply_proposal(&id);
            }
            AppAction::UndoProposal(id) => self.state.undo_proposal(&id),
            AppAction::DiscardProposal(id) => self.state.discard_proposal(&id),
            AppAction::FocusChat => self.state.command_line.open_session(),
            AppAction::ToggleSession => self.state.command_line.toggle_session(),
            AppAction::SendChat => self.state.send_chat(),
            AppAction::StopChat => self.state.stop_chat(),
            AppAction::Ask(text) => {
                self.state.command_line.open_session();
                self.state.chat.draft = text;
                self.state.send_chat();
            }
            AppAction::ToggleAutoApply => self.state.toggle_auto_apply(),
            AppAction::Values(action) => crate::values_tab::apply(&mut self.state, action),
            AppAction::SketchValues(crate::sketch_values::Action::Open) => crate::sketch_values::open(&mut self.state),
            AppAction::SketchValues(crate::sketch_values::Action::Cancel) => {
                crate::sketch_values::cancel(&mut self.state)
            }
            AppAction::SketchValues(crate::sketch_values::Action::Apply { unlink }) => {
                if crate::sketch_values::apply(&mut self.state, unlink) {
                    self.sync_sketch_session();
                }
            }
            // The web build's provider, model and key are set on its server.
            #[cfg(target_arch = "wasm32")]
            AppAction::ShowProviderSettings => {
                self.state.note("The assistant's provider and key are set on the SolidDraft server", NoteKind::Info)
            }
            #[cfg(not(target_arch = "wasm32"))]
            AppAction::ShowProviderSettings => {
                self.state.provider_dialog = Some(crate::chat::ProviderDialog::new(&crate::settings::assistant()))
            }
            AppAction::NewPart => {
                self.state.start = None;
                self.state.drawing_after_open = false;
                self.close_editor();
                self.state.sketch = None;
                self.state.chat.preview = None;
                self.state.import_dialog = None;
                self.state.doc = crate::new_document();
                self.state.viewport.camera.fit(None);
                self.state.note("New part", NoteKind::Info);
            }
            AppAction::ImportStep(path) => {
                let path = path.or_else(|| crate::platform::open_path("Import STEP", "STEP", &["step", "stp"]));
                if let Some(path) = path {
                    match crate::import::Preview::read(&self.state.doc, &path) {
                        Ok(preview) => self.state.import_dialog = Some(preview),
                        Err(message) => self.state.note(message, NoteKind::Error),
                    }
                }
            }
            AppAction::ImportIges(path) => {
                if self.state.in_sketch() || self.state.doc.proof_pending() {
                    return;
                }
                let path = path.or_else(|| crate::platform::open_path("Import IGES", "IGES", &["iges", "igs"]));
                if let Some(path) = path {
                    match crate::import::Preview::read_source(
                        &self.state.doc,
                        &path,
                        soliddraft_kernel_api::ImportFormat::Iges,
                    ) {
                        Ok(preview) => self.state.import_dialog = Some(preview),
                        Err(message) => self.state.note(message, NoteKind::Error),
                    }
                }
            }
            AppAction::ImportScene(path) => {
                let path = path.or_else(|| {
                    crate::platform::open_path(
                        "Import scene",
                        "OpenUSD, glTF, URDF, OBJ and STL",
                        crate::scene_import::EXTENSIONS,
                    )
                });
                let Some(path) = path else { return };
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
                let default = format!("{stem}.{}", soliddraft_document::kind::ASSEMBLY_EXTENSION);
                let Some(out) = crate::platform::save_path(
                    "Save the imported assembly",
                    "SolidDraft assembly",
                    soliddraft_document::kind::ASSEMBLY_EXTENSION,
                    &default,
                ) else {
                    return;
                };
                match crate::scene_import::import(&path, &out, crate::scene_import::Choices::default()) {
                    Ok(record) => {
                        self.state.note(crate::scene_import::said(&record), NoteKind::Ok);
                        self.apply(ctx, AppAction::Assembly(crate::assembly::AssemblyAction::Open(out)));
                    }
                    Err(message) => self.state.note(message, NoteKind::Error),
                }
            }
            AppAction::Open(path) => {
                let path =
                    path.or_else(|| crate::platform::open_path("Open", "SolidDraft documents", &file::OPEN_EXTENSIONS));
                let Some(path) = path else { return };
                match soliddraft_document::DocKind::of_path(&path) {
                    Some(soliddraft_document::DocKind::Drawing) => {
                        self.apply(ctx, AppAction::Drawing(crate::drawing::DrawingAction::OpenFile(path)));
                    }
                    Some(soliddraft_document::DocKind::Assembly) => {
                        self.apply(ctx, AppAction::Assembly(crate::assembly::AssemblyAction::Open(path)));
                    }
                    _ => {
                        let opened = file::open(&path, self.state.doc.kernel().clone(), Some(self.state.doc.model()));
                        if opened.is_ok() {
                            self.state.import_dialog = None;
                        }
                        self.opened(opened);
                    }
                }
            }
            AppAction::NewDrawing => {
                if self.state.doc.display_body().is_some() {
                    self.apply(ctx, AppAction::Drawing(crate::drawing::DrawingAction::MakeDrawing));
                    return;
                }
                // A drawing shows a part: ask for one, and start the drawing once it opens.
                self.state.drawing_after_open = true;
                let parts = [file::EXTENSION, file::LEGACY_EXTENSION];
                match crate::platform::open_path("Open the part to draw", "SolidDraft part", &parts) {
                    Some(path) => self.apply(ctx, AppAction::Open(Some(path))),
                    // The browser's upload arrives later; on the desktop nothing was chosen.
                    None => self.state.drawing_after_open = cfg!(target_arch = "wasm32"),
                }
            }
            AppAction::OpenBytes { name, bytes } => self.open_bytes(&name, &bytes),
            AppAction::ShowStartPage => {
                let ctx = ctx.clone();
                self.state.start = Some(crate::start_page::StartPage::new(&self.state, move || ctx.request_repaint()));
            }
            AppAction::Save => match self.state.doc.path.clone() {
                Some(p) => self.apply(ctx, AppAction::SaveAs(Some(p))),
                None => self.apply(ctx, AppAction::SaveAs(None)),
            },
            AppAction::SaveAs(path) => {
                let path = path.or_else(|| {
                    let name = format!("{}.{}", self.state.doc.name, file::EXTENSION);
                    crate::platform::save_path("Save part", "SolidDraft part", file::EXTENSION, &name)
                });
                let Some(path) = path else { return };
                let path = file::with_extension(&path);
                let thumbnail = self.thumbnail();
                let extras = file::Extras { thumbnail: thumbnail.as_deref(), ..Default::default() };
                match file::save_with(&mut self.state.doc, &path, &extras) {
                    Ok(warning) => {
                        self.state.remember_revision();
                        self.state.recent = file::recent();
                        #[cfg(target_arch = "wasm32")]
                        {
                            let id = self.state.doc.model().id();
                            let key = crate::documents_browser::key(id, soliddraft_document::DocKind::Part);
                            self.picture_for_recent(ctx, key);
                        }
                        match warning {
                            Some(w) => self.state.note(format!("Saved {} · {w}", path.display()), NoteKind::Error),
                            None => self.state.note(format!("Saved {}", path.display()), NoteKind::Ok),
                        }
                    }
                    Err(e) => self.state.note(e, NoteKind::Error),
                }
            }
            AppAction::Export { format, path } => {
                let path = path.or_else(|| {
                    let name = format!("{}.{}", self.state.doc.name, format.extension());
                    let title = format!("Export {}", format.name());
                    crate::platform::save_path(&title, format.name(), format.extension(), &name)
                });
                let Some(path) = path else { return };
                let exported = self.state.export_part(format);
                match exported.and_then(|out| crate::platform::write_file(&path, &out.bytes).map(|()| out.report)) {
                    Ok(report) => {
                        let kind = match &report {
                            soliddraft_exchange::Report::Step(r) if !r.within_tolerance => NoteKind::Error,
                            soliddraft_exchange::Report::Mesh(r) if !r.closed => NoteKind::Error,
                            _ => NoteKind::Ok,
                        };
                        self.state.note(format!("Exported {} · {}", path.display(), report.summary()), kind);
                    }
                    Err(e) => self.state.note(format!("Export: {e}"), NoteKind::Error),
                }
            }
            AppAction::ClearRecent => {
                file::forget_all();
                #[cfg(target_arch = "wasm32")]
                crate::documents_browser::forget_recent();
                self.state.recent.clear();
                if let Some(page) = self.state.start.as_mut() {
                    page.docs = Some(Vec::new());
                }
            }
            AppAction::ResetLayout => {
                let drawing_open = self.dock.find_tab(&Tab::Drawing).is_some();
                let assembly_open = self.dock.find_tab(&Tab::Assembly).is_some();
                self.dock = center_dock();
                if drawing_open {
                    self.dock.push_to_focused_leaf(Tab::Drawing);
                }
                if assembly_open {
                    self.dock.push_to_focused_leaf(Tab::Assembly);
                }
                self.tree_dock = tree_dock();
                self.suite_tree_dock = egui_dock::DockState::new(vec![Tab::Tree, Tab::Values]);
                self.state.layout = Layout::default();
                self.state.command_line.settings.session_height = crate::command_line::SESSION_DEFAULT_HEIGHT;
                // Forget the sizes egui kept, so the defaults apply again.
                ctx.data_mut(|d| {
                    for side in Side::ALL {
                        d.remove::<egui::PanelState>(side.panel_id());
                    }
                    d.remove::<egui::PanelState>(crate::command_line::session_id());
                });
            }
            AppAction::TogglePanel(_) => {
                let shown = self.state.tree_shown();
                self.state.show_tree(!shown);
            }
            AppAction::ShowFileLocations(kind) => {
                self.state.locations_dialog = Some(crate::locations::LocationsDialog::new(kind));
            }
            AppAction::ShowPreferences => {
                self.preferences.get_or_insert_with(|| {
                    crate::preferences::Preferences::new(
                        &self.state.navigation,
                        self.state.viewport.references,
                        self.state.viewport.grid_style,
                    )
                    .with_shortcuts(&self.state.shortcuts)
                    .with_autosave(self.state.autosave_policy())
                });
            }
            AppAction::ShowShortcuts => self.state.show_shortcuts = true,
            AppAction::TogglePalette => crate::palette::toggle(&mut self.state),
            AppAction::ShowAbout => self.state.show_about = true,
            AppAction::ShowRecovery => {
                if let Some(r) = self.state.recovery.as_mut() {
                    r.show = true;
                }
                #[cfg(target_arch = "wasm32")]
                crate::journal_browser::show();
            }
            AppAction::RestoreRecovery(file) => {
                self.restore(&file);
                self.close_recovery_when_done();
            }
            AppAction::DiscardRecovery(file) => {
                if let Some(Err(e)) = self.state.recovery.as_mut().map(|r| r.discard(&file)) {
                    self.state.note(e, NoteKind::Error);
                }
                self.close_recovery_when_done();
            }
            AppAction::ExportCrashJournal(source) => {
                if let Some(path) = crate::platform::save_path(
                    "Save crash report journal",
                    "Command journal",
                    "jsonl",
                    "crash-report.jsonl",
                ) {
                    match soliddraft_document::journal::export(&source, &path) {
                        Ok(()) => self.state.note(
                            "Crash report journal saved with personal paths replaced. Review it before sharing.",
                            NoteKind::Ok,
                        ),
                        Err(e) => self.state.note(format!("Cannot save crash report journal: {e}"), NoteKind::Error),
                    }
                }
            }
            AppAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// After the document changed under an open sketch (undo, redo, a command),
    /// the session takes the document's copy so both agree.
    pub(super) fn sync_sketch_session(&mut self) {
        let Some(s) = self.state.sketch.as_mut() else { return };
        s.cancel_trim();
        if s.corner_first.take().is_some() {
            s.selection.clear();
        }
        match self.state.doc.feature(s.feature).and_then(|f| f.sketch()) {
            Some(sk) => {
                if sk.sketch != s.sketch {
                    s.link_values = None;
                    s.text_edit = None;
                    if !s.trim_bounds.is_empty() {
                        s.trim_bounds.clear();
                        s.selection.clear();
                        s.preview = None;
                    }
                    s.sketch = sk.sketch.clone();
                    s.selection.retain(|sel| match sel {
                        crate::state::SketchSel::Entity(e) => s.sketch.entity(*e).is_some(),
                        crate::state::SketchSel::Point(p) => s.sketch.point(*p).is_some(),
                        crate::state::SketchSel::Constraint(c) => s.sketch.constraint(*c).is_some(),
                    });
                }
                if let Some(plane) = self.state.doc.feature(s.feature).and_then(|f| f.plane) {
                    s.plane = plane;
                }
            }
            None => {
                // The sketch is gone (undone past its creation).
                self.state.sketch = None;
                self.state.ribbon_tab = crate::state::RibbonTab::Solid;
            }
        }
    }
}

/// Points the open revolve or pattern along an origin axis, through the origin.
fn use_axis(params: &mut EditorParams, axis: AxisKind) {
    let d = axis.direction();
    let dir = soliddraft_kernel_api::Vec3::new(d.x.into(), d.y.into(), d.z.into());
    match params {
        EditorParams::Revolve(r) => {
            r.axis_origin = [0.0; 3];
            r.axis_direction = [dir.x, dir.y, dir.z];
        }
        EditorParams::Pattern(p) => match &mut p.layout {
            soliddraft_model::PatternSpec::Linear { direction, .. } => *direction = dir,
            soliddraft_model::PatternSpec::Circular { axis_origin, axis_direction, .. } => {
                *axis_origin = soliddraft_kernel_api::Vec3::ZERO;
                *axis_direction = dir;
            }
        },
        _ => {}
    }
}

/// Maps an id-buffer pick to the document entity it stands for.
pub(super) fn selection_of(doc: &Document, picked: Picked) -> Option<Selection> {
    crate::viewport::pick::selection(doc, picked)
}
