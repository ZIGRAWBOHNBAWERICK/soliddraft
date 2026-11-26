//! Render settings: the three looks (Fast, Studio, Realistic) and the capability tier
//! of the device they run on. A preset is a bundle of lighting values the renderer
//! reads each frame; the tier decides which passes the device can run at all.

use egui_wgpu::wgpu;
use serde::{Deserialize, Serialize};

/// How much effort the viewport spends on the look. Studio is the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Colour, environment lighting and edges; no shadows or occlusion.
    Fast,
    /// Fast plus soft shadows from the fixed light and light ambient occlusion.
    #[default]
    Studio,
    /// Studio with a stronger shadow light, stronger reflections and occlusion.
    Realistic,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Fast, Preset::Studio, Preset::Realistic];

    pub fn label(&self) -> &'static str {
        match self {
            Preset::Fast => "Fast",
            Preset::Studio => "Studio",
            Preset::Realistic => "Realistic",
        }
    }

    /// Parses a preset name as typed on the command line or in a page address.
    pub fn parse(name: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.label().eq_ignore_ascii_case(name.trim()))
    }

    /// Whether the preset draws edges unless the user asks for them. Realistic is for
    /// presentation and leaves them off.
    pub fn shows_edges(&self) -> bool {
        !matches!(self, Preset::Realistic)
    }

    /// The lighting values of this preset on a device of `tier`.
    pub fn look(&self, tier: Tier) -> Look {
        let ao = tier.has_ambient_occlusion();
        match self {
            Preset::Fast => Look {
                shadows: false,
                shadow_strength: 0.0,
                shadow_softness: 0.0,
                ambient_occlusion: 0.0,
                environment: 0.55,
                reflections: 1.0,
                rig: 1.0,
                sun: 0.0,
            },
            Preset::Studio => Look {
                shadows: true,
                shadow_strength: 0.45,
                shadow_softness: 1.0,
                ambient_occlusion: if ao { 0.6 } else { 0.0 },
                environment: 0.36,
                reflections: 1.0,
                rig: 0.85,
                sun: 0.95,
            },
            Preset::Realistic => Look {
                shadows: true,
                shadow_strength: 0.7,
                shadow_softness: 0.6,
                ambient_occlusion: if ao { 1.0 } else { 0.0 },
                environment: 0.3,
                reflections: 1.6,
                rig: 0.7,
                sun: 1.25,
            },
        }
    }
}

/// Lighting values the renderer reads each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Whether the fixed light casts shadows on the ground and the parts.
    pub shadows: bool,
    /// How dark a fully shadowed spot on the ground gets, 0 to 1.
    pub shadow_strength: f32,
    /// Filter radius of the shadow edge, relative to Studio's.
    pub shadow_softness: f32,
    /// Strength of the screen-space ambient occlusion, 0 for none.
    pub ambient_occlusion: f32,
    /// Scale of the light from the environment.
    pub environment: f32,
    /// Scale of the environment's specular reflections.
    pub reflections: f32,
    /// Scale of the studio rig, the key, fill and rim lights that follow the camera.
    pub rig: f32,
    /// Intensity of the fixed light above the model; 0 leaves the studio rig alone.
    pub sun: f32,
}

/// What the device can run. Basic keeps the WebGL2-compatible passes; the full tier
/// adds ambient occlusion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Shading, shadows and wide lines, without the occlusion pass.
    Basic,
    /// Every viewport lighting pass.
    Full,
}

impl Tier {
    /// Selects the tier from the active backend and its enabled limits. Keep OpenGL
    /// on the basic path, which avoids loading depth texels in the occlusion shader.
    pub fn of(backend: wgpu::Backend, limits: &wgpu::Limits) -> Tier {
        if matches!(backend, wgpu::Backend::Gl | wgpu::Backend::Noop)
            || limits.max_compute_invocations_per_workgroup == 0
        {
            Tier::Basic
        } else {
            Tier::Full
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Tier::Basic => "basic",
            Tier::Full => "full",
        }
    }

    pub fn has_ambient_occlusion(&self) -> bool {
        *self == Tier::Full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn studio_is_the_default_and_names_parse() {
        assert_eq!(Preset::default(), Preset::Studio);
        for p in Preset::ALL {
            assert_eq!(Preset::parse(&p.label().to_lowercase()), Some(p));
        }
        assert_eq!(Preset::parse("cinema"), None);
    }

    #[test]
    fn the_basic_tier_drops_occlusion_and_keeps_shadows() {
        let full = Preset::Studio.look(Tier::Full);
        let basic = Preset::Studio.look(Tier::Basic);
        assert!(full.ambient_occlusion > 0.0);
        assert_eq!(basic.ambient_occlusion, 0.0);
        assert!(basic.shadows);
        assert!(!Preset::Fast.look(Tier::Full).shadows);
        let webgl = wgpu::Limits::downlevel_webgl2_defaults();
        assert_eq!(Tier::of(wgpu::Backend::Gl, &webgl), Tier::Basic);
        assert_eq!(Tier::of(wgpu::Backend::Vulkan, &webgl), Tier::Basic);
        assert_eq!(Tier::of(wgpu::Backend::BrowserWebGpu, &wgpu::Limits::default()), Tier::Full);
    }

    #[test]
    fn recorded_adapter_limits_change_the_level_for_the_same_view() {
        use soliddraft_tessellation::{DeviceBudget, Level, Levels, Quality, Thresholds, Usage};

        // Linux Vulkan: RTX 2070 Max-Q and UHD 630. wgpu clamps the Intel buffer limit
        // to i32::MAX, and reports the NVIDIA limit of 1 TiB. Both support compute.
        let discrete = wgpu::Limits {
            max_buffer_size: 1 << 40,
            max_compute_invocations_per_workgroup: 1024,
            ..wgpu::Limits::default()
        };
        let integrated = wgpu::Limits { max_buffer_size: i32::MAX as u64, ..discrete.clone() };
        let levels = Levels::new(&Quality::Auto, 100.0);
        let usage = Usage { triangles: 10_000_000, bytes: 640_000_000 };
        let [strong, weak] = [&discrete, &integrated].map(|limits| {
            assert_eq!(Tier::of(wgpu::Backend::Vulkan, limits), Tier::Full);
            let budget = DeviceBudget::from_buffer_limit(limits.max_buffer_size);
            Thresholds::default().initial(&levels, 10.0, budget.ceiling(usage)).level
        });
        assert_eq!((strong, weak), (Level::Fine, Level::Coarse));
    }

    #[test]
    fn realistic_is_stronger_than_studio() {
        let (s, r) = (Preset::Studio.look(Tier::Full), Preset::Realistic.look(Tier::Full));
        assert!(r.shadow_strength > s.shadow_strength);
        assert!(r.reflections > s.reflections);
        assert!(r.ambient_occlusion > s.ambient_occlusion);
        assert!(!Preset::Realistic.shows_edges());
    }
}
