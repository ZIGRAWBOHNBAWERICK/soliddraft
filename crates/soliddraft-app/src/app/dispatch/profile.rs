//! Operations that start from a sketch or ordered profile picks.

use super::SolidDraftApp;
use crate::document::{CutParams, ExtrudeParams, Selection};
use crate::state::{EditorParams, ProfileFeature};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProfileAction {
    Extrude,
    Revolve,
    Sweep,
    /// Opens a loft between ordered sketch or face profiles.
    Loft,
    Cut,
}

impl SolidDraftApp {
    pub(super) fn dispatch_profile(&mut self, action: ProfileAction) {
        match action {
            ProfileAction::Extrude => {
                if self.state.in_sketch() {
                    return;
                }
                if let Some(id) = self.sketch_for(ProfileFeature::Extrude) {
                    let params = ExtrudeParams { sketch: Some(id), depth: 10.0, ..Default::default() };
                    self.open_editor(None, EditorParams::Extrude(params));
                    self.show_depth_of(id);
                }
            }
            ProfileAction::Loft => {
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
            ProfileAction::Sweep => {
                if self.state.in_sketch() {
                    return;
                }
                if let Some(id) = self.sketch_for(ProfileFeature::Sweep) {
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
            }
            ProfileAction::Revolve => {
                if self.state.in_sketch() {
                    return;
                }
                if let Some(id) = self.sketch_for(ProfileFeature::Revolve) {
                    self.open_editor(
                        None,
                        EditorParams::Revolve(soliddraft_model::RevolveFeature {
                            sketch: soliddraft_model::FeatureId(id as u32),
                            axis_origin: [0.0; 3],
                            axis_direction: [0.0, 1.0, 0.0],
                            angle_deg: 360.0,
                        }),
                    );
                }
            }
            ProfileAction::Cut => {
                if self.state.in_sketch() {
                    return;
                }
                if let Some(id) = self.sketch_for(ProfileFeature::Cut) {
                    self.open_editor(None, EditorParams::Cut(CutParams::new(id)));
                    self.show_depth_of(id);
                }
            }
        }
    }
}
