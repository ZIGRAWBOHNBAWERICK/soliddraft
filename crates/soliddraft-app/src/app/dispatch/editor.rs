//! The feature editor life cycle and its live controls.

use super::SolidDraftApp;
use crate::document::{Command, DraftParams, FeatureId, FeatureKind, FeatureStatus, HoleParams, Selection};
use crate::state::{AppAction, EditorParams, NoteKind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditorAction {
    EditFeature(FeatureId),
    Cancel,
    Commit,
    ApplySuggestedFilletRadius(f64),
    /// The open Extrude's or Cut's depth, from its arrow on the preview.
    Depth(f64),
    /// Opens the open form's Depth for typing.
    FocusDepth,
}

impl SolidDraftApp {
    pub(super) fn dispatch_editor(&mut self, ctx: &egui::Context, action: EditorAction) {
        match action {
            EditorAction::EditFeature(id) => {
                let Some(f) = self.state.doc.feature(id) else { return };
                let params = match &f.kind {
                    FeatureKind::Combine { operation, tool } => {
                        self.state.doc.combine_form =
                            Some(crate::multibody::CombineForm::editing(id, f.body_key, *tool, *operation));
                        return;
                    }
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
                    FeatureKind::Hole(h) => {
                        let sketch_body = h
                            .sketch
                            .and_then(|sid| self.state.doc.feature(u64::from(sid.0)))
                            .and_then(|sf| sf.sketch())
                            .map(|sf| sf.sketch.clone());
                        EditorParams::Hole(Box::new(HoleParams::from_model(h, sketch_body)))
                    }
                    FeatureKind::Pattern(p) => EditorParams::Pattern(p.clone()),
                    FeatureKind::ReferencePlane(p) => EditorParams::ReferencePlane(p.clone()),
                    FeatureKind::Mirror(m) => EditorParams::Mirror(m.clone()),
                    FeatureKind::Shell(s) => EditorParams::Shell(s.clone()),
                    FeatureKind::Offset(s) => EditorParams::Offset(s.clone()),
                    FeatureKind::Thicken(s) => EditorParams::Thicken(s.clone()),
                    FeatureKind::Draft(d) => EditorParams::Draft(Box::new(DraftParams::of(d))),
                    // An import has no editor; its repair questions have their own dialog.
                    FeatureKind::ImportStep(_) => return,
                    FeatureKind::Scan(_) => {
                        if !crate::scan::part_active(&self.state) {
                            return;
                        }
                        if self.state.in_sketch() {
                            self.apply(ctx, AppAction::ExitSketch);
                        }
                        self.close_editor();
                        self.state.doc.selection = Some(Selection::Feature(id));
                        self.state.scan.open = true;
                        return;
                    }
                    // The lattice's controls arrive with its editor slice.
                    FeatureKind::Lattice(l) => EditorParams::Lattice(l.clone()),
                    FeatureKind::Gyroid(g) => EditorParams::Gyroid(g.clone()),
                };
                if self.state.in_sketch() {
                    self.apply(ctx, AppAction::ExitSketch);
                }
                let merge = self.state.doc.merges(id);
                self.open_editor(Some(id), params);
                if let Some(editor) = self.state.editor.as_mut() {
                    editor.merge = merge;
                }
                self.state.doc.selection = Some(Selection::Feature(id));
            }
            EditorAction::Commit => {
                // A refused size stays open for correction: OK cannot apply the last
                // accepted one in its place.
                if self
                    .state
                    .editor
                    .as_ref()
                    .is_some_and(|e| e.feedback.size.is_some() || e.feedback.second_size.is_some())
                {
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
                // A From or Direction 2 still waiting for its pick keeps the form open.
                if let Some(editor) = self.state.editor.as_mut() {
                    let choices = match &editor.params {
                        EditorParams::Extrude(p) => Some(&p.choices),
                        EditorParams::Cut(p) => Some(&p.choices),
                        _ => None,
                    };
                    if let Some(Err(why)) = choices.map(|c| c.to_model()) {
                        editor.refuse(why);
                        return;
                    }
                }
                if let Some(session) =
                    self.state.sketch.take_if(|s| s.feature == crate::hole_wizard_panel::INLINE_POSITION_SKETCH_ID)
                {
                    self.state.viewport.camera.orthographic = session.was_orthographic;
                    self.state.ribbon_tab = crate::state::RibbonTab::Solid;
                    if let Some(editor) = self.state.editor.as_mut()
                        && let EditorParams::Hole(h) = &mut editor.params
                    {
                        h.position_sketch = Some(session.sketch);
                        h.sync_points_from_sketch();
                    }
                }
                // The extrusion or cut the preview built, while the part is as it was then.
                let prepared = self.state.editor.as_mut().and_then(|e| e.take_prepared(&self.state.doc));
                let Some(e) = self.close_editor() else { return };
                let reopen = (e.target, e.params.clone(), e.then_feature);
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
                        Some(id) => Command::EditRevolve { id, revolve: r, merge: e.merge },
                        None => Command::AddRevolve { revolve: r, merge: e.merge },
                    },
                    (target, params @ (EditorParams::Extrude(_) | EditorParams::Cut(_))) => {
                        crate::state::extrusion_command(target, &params).expect("an extrusion or cut has its edit")
                    }
                    (Some(id), EditorParams::Fillet(p)) => Command::EditFillet { id, params: p },
                    (None, EditorParams::Fillet(p)) => Command::AddFillet { params: p },
                    (Some(id), EditorParams::Chamfer(p)) => Command::EditChamfer { id, params: p },
                    (None, EditorParams::Chamfer(p)) => Command::AddChamfer { params: p },
                    (target, EditorParams::Hole(mut h)) => {
                        if let Some(session) = self.state.sketch.take()
                            && session.feature == crate::hole_wizard_panel::INLINE_POSITION_SKETCH_ID
                        {
                            self.state.viewport.camera.orthographic = session.was_orthographic;
                            self.state.ribbon_tab = crate::state::RibbonTab::Solid;
                            h.position_sketch = Some(session.sketch);
                            h.sync_points_from_sketch();
                        }
                        match h.to_model() {
                            Ok(hole) => match (target, h.sketch_feature, h.position_sketch) {
                                (Some(id), sketch_id, Some(sketch)) => {
                                    let sketch_id = sketch_id
                                        .unwrap_or_else(|| u64::from(self.state.doc.model().next_feature_id().0));
                                    Command::EditHoleWithSketch { id, sketch_id, sketch, hole }
                                }
                                (None, None, Some(sketch)) => Command::AddHoleWithSketch { sketch, hole },
                                (Some(id), _, _) => Command::EditFeature { id, kind: FeatureKind::Hole(hole) },
                                (None, _, _) => Command::AddFeature(FeatureKind::Hole(hole)),
                            },
                            Err(why) => {
                                self.refuse(reopen, why);
                                return;
                            }
                        }
                    }
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
                    (target, EditorParams::Offset(s)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Offset(s) },
                        None => Command::AddFeature(FeatureKind::Offset(s)),
                    },
                    (target, EditorParams::Thicken(s)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Thicken(s) },
                        None => Command::AddFeature(FeatureKind::Thicken(s)),
                    },
                    (target, EditorParams::Shell(s)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Shell(s) },
                        None => Command::AddFeature(FeatureKind::Shell(s)),
                    },
                    (target, EditorParams::Draft(d)) => match target {
                        Some(id) => Command::EditFeature { id, kind: FeatureKind::Draft(d.to_model()) },
                        None => Command::AddFeature(FeatureKind::Draft(d.to_model())),
                    },
                    (_, EditorParams::PickPlane | EditorParams::PickSketch(_)) => return,
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
                // Every OK builds before the form closes, a dimension edit too, so a refused
                // one keeps its form, and the part and its history stay as they were.
                if let Some(drag) = self.state.drag_preview.take() {
                    drag.cancel(&mut self.state.doc);
                }
                if let Some(prepared) = prepared {
                    prepared.offer(&mut self.state.doc, &cmd);
                }
                match self.state.doc.apply_form(cmd) {
                    Ok(Some(id)) => {
                        self.state.doc.selection = Some(Selection::Feature(id));
                        self.state.note("Feature added", NoteKind::Ok);
                    }
                    Ok(None) => self.state.note("Rebuilt", NoteKind::Ok),
                    // The kernel refused: nothing was added, and the form stays open.
                    Err(why) => self.refuse(reopen, why),
                }
                self.sync_sketch_session();
            }
            EditorAction::Cancel => {
                self.close_editor();
            }
            EditorAction::ApplySuggestedFilletRadius(radius) => {
                let Some(editor) = self.state.editor.as_ref().filter(|e| e.suggested_fillet_radius() == Some(radius))
                else {
                    return;
                };
                let EditorParams::Fillet(mut params) = editor.params.clone() else { return };
                let target = editor.target;
                params.radius = radius;
                let editor = self.close_editor().expect("the fillet editor is open");
                match self.state.chat.book.accept_fillet_radius(&mut self.state.doc, target, params) {
                    Ok(()) => self.state.note(format!("Applied {radius} mm fillet radius"), NoteKind::Ok),
                    Err(why) => {
                        self.refuse(
                            (target, editor.params, editor.then_feature),
                            format!(
                                "The suggested radius could not build. {}",
                                crate::geometry_message::for_user(&why)
                            ),
                        );
                    }
                }
                self.sync_sketch_session();
            }
            EditorAction::Depth(depth) => {
                if let Some(e) = self.state.editor.as_mut() {
                    match &mut e.params {
                        EditorParams::Extrude(p) => p.depth = depth,
                        EditorParams::Cut(p) if !p.through_all && p.up_to.is_none() => p.depth = depth,
                        _ => {}
                    }
                }
            }
            EditorAction::FocusDepth => {
                if let Some(e) = self.state.editor.as_mut() {
                    e.focus_depth = true;
                }
            }
        }
    }
}
