//! Application preferences for viewport meshes and rendering.

use serde::{Deserialize, Serialize};
use crate::viewport::look::{Look, Preset, Tier};
use soliddraft_tessellation::{Advanced, Levels, Quality, policy};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsPreset {
    Performance,
    Quality,
    #[default]
    Custom,
}

impl GraphicsPreset {
    pub const ALL: [Self; 3] = [Self::Performance, Self::Quality, Self::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Self::Performance => "Performance",
            Self::Quality => "Quality",
            Self::Custom => "Custom",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurvedSurfaces {
    Coarse,
    #[default]
    Normal,
    Fine,
    Ultra,
}

impl CurvedSurfaces {
    pub const ALL: [Self; 4] = [Self::Coarse, Self::Normal, Self::Fine, Self::Ultra];

    pub fn label(self) -> &'static str {
        match self {
            Self::Coarse => "Coarse",
            Self::Normal => "Normal",
            Self::Fine => "Fine",
            Self::Ultra => "Ultra",
        }
    }

    fn controls(self) -> (u8, f64) {
        match self {
            Self::Coarse => (1, policy::ANGLE_DEG),
            Self::Normal => (5, policy::ANGLE_DEG),
            Self::Fine => (8, 10.0),
            Self::Ultra => (10, 7.5),
        }
    }

    pub fn description(self) -> String {
        let (slider, angle) = self.controls();
        format!("Standard detail: {:.3}% of the body's diagonal, {angle}°", policy::slider_fraction(slider) * 100.0)
    }
}

/// An unset preference keeps the quality from the part file. An explicit choice
/// changes the displayed mesh without writing to that file.
pub(crate) fn mesh_levels(choice: Option<CurvedSurfaces>, part: &Quality, diagonal: f64) -> Levels {
    let quality = match choice {
        None => *part,
        Some(choice) => {
            let (slider, angle_deg) = choice.controls();
            let chord_height = (policy::slider_fraction(slider) * diagonal).max(policy::MIN_CHORD_MM);
            Quality::Custom(Advanced { chord_height, angle_deg, max_edge_length: 0.0, max_aspect_ratio: 0.0 })
        }
    };
    Levels::new(&quality, diagonal)
}

/// The pinned kernel uses a separate certified display query for B-spline bodies.
pub(crate) fn supports_curved_surfaces(surface: soliddraft_kernel_api::SurfaceKind) -> bool {
    surface != soliddraft_kernel_api::SurfaceKind::Spline
}

pub(crate) fn topology_supports_curved_surfaces(topology: &soliddraft_kernel_api::Topology) -> bool {
    topology.faces.iter().all(|face| supports_curved_surfaces(face.surface))
}

pub(crate) fn body_mesh_levels(
    choice: Option<CurvedSurfaces>,
    part: &Quality,
    topology: &soliddraft_kernel_api::Topology,
) -> Levels {
    mesh_levels(choice.filter(|_| topology_supports_curved_surfaces(topology)), part, topology.bbox.diagonal())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Graphics {
    pub preset: GraphicsPreset,
    pub curved_surfaces: Option<CurvedSurfaces>,
    pub samples: u32,
    pub still_supersampling: bool,
    pub show_view_angles: bool,
    pub show_vertex_markers: bool,
    pub quick_measurement: bool,
    pub show_frame_rate: bool,
    pub frame_rate_details: bool,
    pub cosmetic_threads: bool,
    pub sketch_halo: bool,
    pub section_hatch: SectionHatch,
    pub tint_preview: bool,
    pub red_removal: bool,
    pub hidden_selected_edges: bool,
    pub xray_references: bool,
    pub view_aligned_grid: bool,
}

impl Default for Graphics {
    fn default() -> Self {
        Self {
            preset: GraphicsPreset::Custom,
            curved_surfaces: None,
            samples: 4,
            still_supersampling: false,
            show_view_angles: false,
            show_vertex_markers: false,
            quick_measurement: false,
            show_frame_rate: false,
            frame_rate_details: false,
            cosmetic_threads: true,
            sketch_halo: false,
            section_hatch: SectionHatch::Solid,
            tint_preview: true,
            red_removal: true,
            hidden_selected_edges: false,
            xray_references: false,
            view_aligned_grid: false,
        }
    }
}

impl Graphics {
    pub fn choose_preset(&mut self, preset: GraphicsPreset, device: &GraphicsDevice) {
        self.preset = preset;
        match preset {
            GraphicsPreset::Performance => {
                self.curved_surfaces = Some(CurvedSurfaces::Coarse);
                self.samples = 1;
                self.still_supersampling = false;
            }
            GraphicsPreset::Quality => {
                self.curved_surfaces = Some(CurvedSurfaces::Fine);
                self.samples = device.samples.iter().copied().filter(|n| *n <= 4).max().unwrap_or(1);
                self.still_supersampling = device.supersampling();
            }
            GraphicsPreset::Custom => {}
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionHatch {
    #[default]
    Solid,
    Diagonal,
    Crosshatch,
}

impl SectionHatch {
    pub const ALL: [Self; 3] = [Self::Solid, Self::Diagonal, Self::Crosshatch];

    pub fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Diagonal => "Diagonal",
            Self::Crosshatch => "Crosshatch",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GraphicsDevice {
    pub gpu: String,
    pub renderer: String,
    pub samples: Vec<u32>,
    pub active_samples: u32,
    pub tier: crate::viewport::look::Tier,
}

/// The 3D view on screen, which the Current view controls change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewTarget {
    Part,
    /// The assembly view draws no ground shadow and no ambient occlusion.
    Assembly,
}

impl ViewTarget {
    /// Whether the view draws the ground shadow and ambient occlusion.
    pub fn lit(self) -> bool {
        self == Self::Part
    }
}

/// Controls shared by Preferences and the viewport's own menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurrentView {
    pub references: crate::settings::ReferenceDisplay,
    pub display: crate::viewport::render::DisplayMode,
    pub perspective: bool,
    pub field_of_view: f32,
    pub lighting: Lighting,
}

impl CurrentView {
    pub fn of(view: &crate::viewport::ViewportState) -> Self {
        Self {
            references: view.references,
            display: view.display,
            perspective: !view.camera.orthographic,
            field_of_view: view.camera.fov_y.to_degrees(),
            lighting: view.lighting,
        }
    }

    pub fn apply(self, view: &mut crate::viewport::ViewportState, camera_editable: bool) {
        view.lighting = self.lighting;
        if camera_editable {
            view.camera.orthographic = !self.perspective;
            view.camera.fov_y = self.field_of_view.to_radians();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lighting {
    /// Leave the existing lighting preset in control until explicitly overridden.
    pub ambient_occlusion: Option<bool>,
    pub shadow_follows_model: bool,
    /// The ground shadow as chosen for this view. Until one is chosen, the floor setting
    /// shows it where the look casts shadows. Every floor and ground shadow command
    /// chooses it, so a look change keeps it.
    pub ground_shadow: Option<bool>,
}

impl Default for Lighting {
    fn default() -> Self {
        Self { ambient_occlusion: None, shadow_follows_model: true, ground_shadow: None }
    }
}

impl Lighting {
    /// What a view with these choices draws: `preset`'s look on `tier` and whether it
    /// shows the ground shadow. A ground shadow chosen under a look without shadows takes
    /// Studio's. The Current view controls and the renderer both read this.
    pub fn look(&self, preset: Preset, tier: Tier, floor: bool) -> (Look, bool) {
        let mut look = preset.look(tier);
        let ground = self.ground_shadow.unwrap_or(floor && look.shadows);
        if ground && !look.shadows {
            let studio = Preset::Studio.look(tier);
            look.shadows = true;
            look.shadow_strength = studio.shadow_strength;
            look.shadow_softness = studio.shadow_softness;
        }
        if let Some(ao) = self.ambient_occlusion {
            look.ambient_occlusion = if ao && tier.has_ambient_occlusion() {
                look.ambient_occlusion.max(Preset::Studio.look(tier).ambient_occlusion)
            } else {
                0.0
            };
        }
        (look, ground)
    }
}

impl Default for GraphicsDevice {
    fn default() -> Self {
        Self {
            gpu: "Unavailable".into(),
            renderer: "Unavailable".into(),
            samples: vec![1],
            active_samples: 1,
            tier: crate::viewport::look::Tier::Basic,
        }
    }
}

impl GraphicsDevice {
    pub fn supersampling(&self) -> bool {
        !cfg!(target_arch = "wasm32") && self.tier.has_ambient_occlusion()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chosen_ground_shadow_holds_under_every_look() {
        let chosen = |on| Lighting { ground_shadow: Some(on), ..Default::default() };
        for tier in [Tier::Basic, Tier::Full] {
            for preset in Preset::ALL {
                let (look, ground) = chosen(true).look(preset, tier, true);
                assert!(ground && look.shadows, "{preset:?} on {tier:?} drops a chosen ground shadow");
                assert!(!chosen(false).look(preset, tier, true).1, "{preset:?} on {tier:?} shows a refused one");
            }
            // Until one is chosen, the floor shows the shadow where the look casts one.
            assert!(!Lighting::default().look(Preset::Fast, tier, true).1);
            assert!(Lighting::default().look(Preset::Studio, tier, true).1);
            assert!(!Lighting::default().look(Preset::Studio, tier, false).1);
        }
    }
}
