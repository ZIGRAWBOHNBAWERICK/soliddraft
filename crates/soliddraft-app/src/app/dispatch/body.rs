//! Body operations and their starting selections.

use super::SolidDraftApp;
use crate::document::{
    ChamferParams, DraftParams, FeatureId, FeatureKind, FeatureStatus, FilletParams, Reference, Selection,
};
use crate::state::{EditorParams, NoteKind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BodyAction {
    StartFillet,
    StartChamfer,
    StartPattern { circular: bool },
    StartMirror,
    StartReferencePlane,
    StartDraft,
    StartShell,
    NewBody,
    SelectBody(u32),
    RenameBody,
    StartCombine,
    StartOffset,
    StartThicken,
    StartLattice,
    StartGyroid,
    StartRepair(FeatureId),
}

impl SolidDraftApp {
    pub(super) fn dispatch_body(&mut self, action: BodyAction) {
        match action {
            BodyAction::StartFillet | BodyAction::StartChamfer => {
                let chamfer = matches!(action, BodyAction::StartChamfer);
                let params = if chamfer {
                    EditorParams::Chamfer(ChamferParams { edges: Vec::new(), distance: 2.0, other_distance: None })
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
            BodyAction::StartPattern { circular } => {
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
                self.open_editor(
                    None,
                    EditorParams::Pattern(soliddraft_model::PatternFeature {
                        layout,
                        source: self.state.doc.selection.and_then(|selection| match selection {
                            Selection::Feature(id) => self
                                .state
                                .doc
                                .feature(id)
                                .filter(|f| f.kind.to_model().can_pattern_feature())
                                .map(|_| soliddraft_model::FeatureId(id as u32)),
                            _ => None,
                        }),
                    }),
                );
            }
            BodyAction::StartReferencePlane => {
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
            BodyAction::StartMirror => {
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
            BodyAction::StartLattice => {
                self.open_editor(None, EditorParams::Lattice(soliddraft_model::LatticeFeature::default()));
                self.state.doc.selection = None;
            }
            BodyAction::StartGyroid => {
                self.open_editor(None, EditorParams::Gyroid(soliddraft_model::GyroidFeature::default()));
                self.state.doc.selection = None;
            }
            BodyAction::NewBody => self.state.doc.new_body(),
            BodyAction::SelectBody(key) => self.state.doc.select_body(key),
            BodyAction::RenameBody => crate::tree::rename_body(&mut self.state),
            BodyAction::StartCombine => {
                self.state.doc.combine_form = crate::multibody::CombineForm::new(&self.state.doc);
            }
            BodyAction::StartOffset => {
                self.open_editor(None, EditorParams::Offset(soliddraft_model::OffsetFeature { distance: 1.0 }));
                self.state.doc.selection = None;
            }
            BodyAction::StartThicken => {
                let faces = match self.state.doc.selection {
                    Some(Selection::Face(face)) => self.state.doc.any_face_ref(face).into_iter().collect(),
                    _ => Vec::new(),
                };
                self.open_editor(
                    None,
                    EditorParams::Thicken(soliddraft_model::ThickenFeature {
                        faces,
                        thickness: 2.0.into(),
                        side: soliddraft_kernel_api::ThickenSide::Front,
                    }),
                );
                self.state.doc.selection = None;
            }
            BodyAction::StartShell => {
                let open_faces = match self.state.doc.selection {
                    Some(Selection::Face(face)) => self.state.doc.any_face_ref(face).into_iter().collect(),
                    _ => Vec::new(),
                };
                self.open_editor(
                    None,
                    EditorParams::Shell(soliddraft_model::ShellFeature {
                        open_faces: soliddraft_model::ShellOpenings::Faces(open_faces),
                        thickness: 2.0.into(),
                    }),
                );
                self.state.doc.selection = None;
            }
            BodyAction::StartDraft => {
                let faces = match self.state.doc.selection {
                    Some(Selection::Face(f)) => self.state.doc.any_face_ref(f).into_iter().collect(),
                    _ => Vec::new(),
                };
                self.open_editor(None, EditorParams::Draft(Box::new(DraftParams { faces, ..Default::default() })));
                self.state.doc.selection = None;
            }
            BodyAction::StartRepair(id) => {
                let Some(FeatureStatus::NeedsRepair(items)) = self.state.doc.feature(id).map(|f| f.status.clone())
                else {
                    return;
                };
                let n = items.len();
                self.open_editor(Some(id), EditorParams::Repair { items, chosen: vec![None; n], active: 0 });
                self.state
                    .note("Choose a replacement plane in the tree, or an edge or face in the viewport", NoteKind::Info);
            }
        }
    }
}
