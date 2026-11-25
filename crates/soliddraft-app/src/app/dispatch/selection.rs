//! Viewport selection and the picks used by feature editors.

use super::{SolidDraftApp, sketch_in_use};
use crate::document::{AxisKind, CutParams, Document, EndKind, ExtrudeParams, Reference, Selection, SketchPlane};
use crate::state::{AppAction, EditorParams, NoteKind};

impl SolidDraftApp {
    pub(super) fn select(&mut self, ctx: &egui::Context, sel: Option<Selection>) {
        // The point of the viewport click that made this selection, if one did.
        let click_point = self.state.click_point.take().filter(|(clicked, _)| Some(*clicked) == sel).map(|(_, at)| at);
        // A plain selection shows the dimensions it always did; only a double-click
        // narrows them to one feature.
        self.state.viewport.dimensions_of = None;
        // Editors that collect picks take the click instead of selecting.
        if let Some(e) = self.state.editor.as_mut() {
            match (&mut e.params, sel) {
                (EditorParams::PickSketch(feature), Some(pick)) => {
                    let feature = *feature;
                    match sketch_pick(&self.state.doc, pick) {
                        Ok(SketchPick::Use(id)) => {
                            self.state.doc.selection = Some(Selection::Feature(id));
                            self.apply(ctx, feature.start());
                        }
                        Ok(SketchPick::DrawOn(plane)) => self.apply(ctx, AppAction::StartSketchOn(plane)),
                        Err(why) => e.error = Some(why),
                    }
                    return;
                }
                (
                    EditorParams::Fillet(_) | EditorParams::Chamfer(_),
                    Some(pick @ (Selection::Edge(_) | Selection::Face(_) | Selection::Vertex(_))),
                ) => {
                    crate::fillet_collect::collect(e, &self.state.doc, pick);
                    return;
                }
                (EditorParams::Hole(h), Some(Selection::Face(f))) => {
                    use soliddraft_model::hole_wizard::HoleEndCondition;
                    // A target face is taken only for an end condition that has one,
                    // and only once the holes have a face to start on: until then the
                    // first face picked is the face they start on, and the target pick
                    // waits for the next one.
                    let surface_end =
                        h.unresolved_end.is_some_and(|end| end.pick() == crate::document::HolePickTarget::Surface)
                            || matches!(
                                h.wizard.as_ref().map(|w| &w.end_condition),
                                Some(HoleEndCondition::UpToSurface { .. } | HoleEndCondition::OffsetFromSurface { .. })
                            );
                    if h.picking_target == Some(crate::document::HolePickTarget::Surface) && !surface_end {
                        h.picking_target = None;
                    }
                    if h.picking_target == Some(crate::document::HolePickTarget::Surface) && h.face.is_some() {
                        match self.state.doc.face_ref(f) {
                            Some(target_face) => {
                                match h.unresolved_end.and_then(|end| end.with_face(target_face.clone())) {
                                    Some(end) => {
                                        h.unresolved_end = None;
                                        h.ensure_wizard().end_condition = end;
                                    }
                                    None => {
                                        if let HoleEndCondition::UpToSurface { face }
                                        | HoleEndCondition::OffsetFromSurface { face, .. } =
                                            &mut h.ensure_wizard().end_condition
                                        {
                                            *face = target_face;
                                        }
                                    }
                                }
                                h.depth = soliddraft_kernel_api::HoleDepth::Through;
                                h.picking_target = None;
                                e.refresh(&self.state.doc);
                            }
                            None => {
                                e.error = Some("Up to surface on a non-parallel or curved face is coming soon.".into());
                            }
                        }
                        return;
                    }
                    match self.state.doc.face_ref(f) {
                        Some(r) => {
                            // Where the viewport click that picked the face met it.
                            let click_uv = click_point.zip(self.state.doc.face_plane(f)).map(|(at, plane)| {
                                let at =
                                    soliddraft_kernel_api::Vec3::new(f64::from(at.x), f64::from(at.y), f64::from(at.z));
                                let d = at - plane.origin;
                                let tenth = |v: f64| (v * 10.0).round() / 10.0 + 0.0;
                                crate::sketch::P2::new(tenth(d.dot(plane.x_dir)), tenth(d.dot(plane.y_dir())))
                            });
                            if h.face.as_ref() == Some(&r)
                                && h.wizard_tab == crate::document::HoleWizardTab::Positions
                                && let Some(uv) = click_uv
                            {
                                h.add_position_point(uv);
                                e.refresh(&self.state.doc);
                                return;
                            }
                            // A new hole's first face click also places its first
                            // hole where that click met the face, to a tenth of a mm.
                            let first = h.face.is_none() && h.points == [crate::sketch::P2::ZERO];
                            h.face = Some(r);
                            if first && let Some(uv) = click_uv {
                                h.set_position_point(0, uv);
                            }
                            e.refresh(&self.state.doc);
                        }
                        None => {
                            let label = self
                                .state
                                .doc
                                .faces
                                .iter()
                                .find(|face| face.id == f)
                                .map(|face| face.label.as_str())
                                .unwrap_or("selected face");
                            e.error =
                                Some(format!("Holes can only be placed on a planar face; {label} is not planar."));
                        }
                    }
                    return;
                }
                (EditorParams::Hole(h), Some(Selection::Vertex(v)))
                    if h.picking_target == Some(crate::document::HolePickTarget::Vertex) =>
                {
                    let resolved = self.state.doc.vertex_position(v).and_then(|pos| {
                        let target =
                            soliddraft_kernel_api::Vec3::new(f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
                        let model = self.state.doc.model();
                        let body = model.body()?;
                        let topo = model.kernel().topology(body).ok()?;
                        let best = topo
                            .vertices
                            .iter()
                            .min_by(|a, b| (a.position - target).length().total_cmp(&(b.position - target).length()))?;
                        ((best.position - target).length() < 1e-2).then_some(best.id)
                    });
                    match resolved
                        .ok_or_else(|| "Selected vertex could not be resolved.".to_owned())
                        .and_then(|vid| self.state.doc.model().vertex_ref(vid).map_err(|err| err.to_string()))
                    {
                        Ok(vertex) => {
                            h.ensure_wizard().end_condition =
                                soliddraft_model::hole_wizard::HoleEndCondition::UpToVertex { vertex };
                            h.unresolved_end = None;
                            h.depth = soliddraft_kernel_api::HoleDepth::Through;
                            h.picking_target = None;
                            e.refresh(&self.state.doc);
                        }
                        Err(err) => e.error = Some(err),
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
                    if self.state.doc.model().reference_plane(soliddraft_model::FeatureId(id as u32)).is_some() =>
                {
                    *p = soliddraft_model::SketchPlane::Feature {
                        feature: soliddraft_model::FeatureId(id as u32),
                        offset: p.offset(),
                    };
                }
                (EditorParams::Mirror(m), Some(Selection::Feature(id)))
                    if self.state.doc.model().reference_plane(soliddraft_model::FeatureId(id as u32)).is_some() =>
                {
                    m.plane = soliddraft_model::PlaneRef::Feature { feature: soliddraft_model::FeatureId(id as u32) };
                }
                (EditorParams::PickPlane, Some(Selection::Feature(id)))
                    if self.state.doc.model().reference_plane(soliddraft_model::FeatureId(id as u32)).is_some() =>
                {
                    self.apply(ctx, AppAction::StartSketchOn(SketchPlane::Feature { feature: id, offset: 0.0 }));
                }
                (EditorParams::Loft(l), Some(Selection::Feature(id))) if self.state.doc.sketch_of(id).is_some() => {
                    let source =
                        soliddraft_model::LoftProfile::Sketch { feature: soliddraft_model::FeatureId(id as u32) };
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
                // A grade by distance from picked faces takes each clicked face, or
                // gives it back.
                (
                    EditorParams::Lattice(soliddraft_model::LatticeFeature { radius_mm: Some(grade), .. })
                    | EditorParams::Gyroid(soliddraft_model::GyroidFeature { level: grade, .. }),
                    Some(Selection::Face(f)),
                ) if grade.picks_faces() => {
                    if let Some(reference) = self.state.doc.any_face_ref(f)
                        && let Some(faces) = grade.picked_faces_mut()
                    {
                        if let Some(i) = faces.iter().position(|face| face == &reference) {
                            faces.remove(i);
                        } else {
                            faces.push(reference);
                        }
                    }
                    return;
                }
                (EditorParams::Thicken(s), Some(Selection::Face(f))) => {
                    if let Some(reference) = self.state.doc.any_face_ref(f) {
                        if let Some(i) = s.faces.iter().position(|face| face == &reference) {
                            s.faces.remove(i);
                        } else {
                            s.faces.push(reference);
                        }
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
                (EditorParams::Draft(d), Some(Selection::Face(f))) if d.picking_pull => {
                    match self.state.doc.face_ref(f) {
                        Some(face) => {
                            d.pull = Some(soliddraft_model::PullRef::Face { face });
                            d.picking_pull = false;
                        }
                        None => e.error = Some("Pull along a planar face's normal, an edge or an axis".into()),
                    }
                    return;
                }
                // A draft takes no edges or axes otherwise, so either sets the pull.
                (EditorParams::Draft(d), Some(Selection::Edge(edge))) => {
                    match self.state.doc.edge_ref(edge) {
                        Some(edge) => {
                            d.pull = Some(soliddraft_model::PullRef::Edge { edge });
                            d.picking_pull = false;
                        }
                        None => e.error = Some("This edge cannot be referenced".into()),
                    }
                    return;
                }
                (EditorParams::Draft(d), Some(Selection::Reference(Reference::Axis(axis)))) => {
                    d.pull = Some(soliddraft_model::PullRef::Axis { axis: axis.to_model() });
                    d.picking_pull = false;
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
                // A From or Direction 2 field waiting for its pick takes the click first.
                (
                    EditorParams::Extrude(ExtrudeParams { choices, .. }) | EditorParams::Cut(CutParams { choices, .. }),
                    Some(pick),
                ) if choices.from.picking()
                    || matches!(
                        choices.direction2,
                        Some(crate::document::Direction2 { end: EndKind::UpToFace(None), .. })
                    ) =>
                {
                    let doc = &self.state.doc;
                    // A curved face is kept too: an extrusion can start on it.
                    let plane = |pick: Selection| match pick {
                        Selection::Face(f) => doc
                            .face_ref(f)
                            .or_else(|| doc.any_face_ref(f))
                            .map(|face| soliddraft_model::PlaneRef::Face { face }),
                        Selection::Reference(Reference::Plane(p)) => {
                            Some(soliddraft_model::PlaneRef::Datum { datum: p.to_model(), offset: 0.0 })
                        }
                        Selection::Feature(id)
                            if doc.model().reference_plane(soliddraft_model::FeatureId(id as u32)).is_some() =>
                        {
                            Some(soliddraft_model::PlaneRef::Feature {
                                feature: soliddraft_model::FeatureId(id as u32),
                            })
                        }
                        _ => None,
                    };
                    match (&mut choices.from, pick) {
                        (crate::document::StartKind::Surface(slot @ None), pick) => match plane(pick) {
                            Some(p) => {
                                *slot = Some(p);
                                e.error = None;
                            }
                            None if matches!(pick, Selection::Face(_)) => {
                                e.error = Some("This face cannot be named. Pick another".into());
                            }
                            None => e.error = Some("Pick a planar face or a plane to start from".into()),
                        },
                        (crate::document::StartKind::Vertex(slot @ None), Selection::Vertex(v)) => {
                            match doc.vertex_ref(v) {
                                Some(r) => {
                                    *slot = Some(r);
                                    e.error = None;
                                }
                                None => e.error = Some("This vertex cannot be named. Pick another".into()),
                            }
                        }
                        (crate::document::StartKind::Vertex(None), _) => {
                            e.error = Some("Pick a vertex to start from".into());
                        }
                        (_, Selection::Face(f)) => {
                            if let Some(crate::document::Direction2 { end: EndKind::UpToFace(face), .. }) =
                                &mut choices.direction2
                            {
                                *face = doc.any_face_ref(f);
                            }
                        }
                        _ => {}
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
                                plane: soliddraft_model::SketchPlane::Datum { datum: p.to_model(), offset: 0.0 },
                                label: self.state.doc.model().datum_name(p.to_model()).to_owned(),
                            })
                        }
                        (
                            Some(crate::document::RefKind::Face | crate::document::RefKind::Plane),
                            Selection::Face(face),
                        ) => self.state.doc.face_ref(face).map(crate::document::EntityRef::Face),
                        // An extrusion's start and limits take curved faces as well.
                        (Some(crate::document::RefKind::AnyFace), Selection::Face(face)) => {
                            self.state.doc.any_face_ref(face).map(crate::document::EntityRef::Face)
                        }
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
                                Some(crate::document::RefKind::Edge) => "Pick an edge for this reference".into(),
                                Some(crate::document::RefKind::Plane) => {
                                    "Choose a reference plane or planar face".into()
                                }
                                Some(crate::document::RefKind::AnyFace) => "Pick a face for this reference".into(),
                                _ => "Pick a planar face for this reference".into(),
                            })
                        }
                    }
                    return;
                }
                _ => {}
            }
        }
        self.state.doc.target_selection(sel);
        self.state.doc.selection = sel;
        self.state.rename = None;
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

/// What a click gives a feature asking for its sketch.
enum SketchPick {
    /// An existing sketch no other feature uses.
    Use(crate::document::FeatureId),
    /// A plane or planar face to draw a new sketch on.
    DrawOn(SketchPlane),
}

fn sketch_pick(doc: &Document, pick: Selection) -> Result<SketchPick, String> {
    match pick {
        Selection::Feature(id) if doc.sketch_of(id).is_some() => match sketch_in_use(doc, id) {
            Some(why) => Err(why),
            None => Ok(SketchPick::Use(id)),
        },
        Selection::Feature(id) if doc.model().reference_plane(soliddraft_model::FeatureId(id as u32)).is_some() => {
            Ok(SketchPick::DrawOn(SketchPlane::Feature { feature: id, offset: 0.0 }))
        }
        Selection::Reference(Reference::Plane(plane)) => {
            Ok(SketchPick::DrawOn(SketchPlane::Reference { plane, offset: 0.0 }))
        }
        Selection::Face(f) => match doc.face_ref(f) {
            Some(face) => Ok(SketchPick::DrawOn(SketchPlane::Face { face })),
            None => Err("Sketches need a planar face".into()),
        },
        _ => Err("Pick a sketch, or a plane or flat face to sketch on".into()),
    }
}
