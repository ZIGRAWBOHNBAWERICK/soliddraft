//! Named viewport backgrounds from the Edit Suite prototype kit.

use egui::Color32;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scene {
    #[default]
    White,
    Studio,
    Faded,
    Spot,
    Graphite,
    Night,
    Beige,
    Blueprint,
    Lightbox,
}

impl Scene {
    pub const ALL: [Self; 9] = [
        Self::White,
        Self::Studio,
        Self::Faded,
        Self::Spot,
        Self::Graphite,
        Self::Night,
        Self::Beige,
        Self::Blueprint,
        Self::Lightbox,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::White => "white",
            Self::Studio => "studio",
            Self::Faded => "faded",
            Self::Spot => "spot",
            Self::Graphite => "graphite",
            Self::Night => "night",
            Self::Beige => "beige",
            Self::Blueprint => "blueprint",
            Self::Lightbox => "lightbox",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::White => "Plain white",
            Self::Studio => "Studio fade",
            Self::Faded => "3 point faded",
            Self::Spot => "Soft spotlight",
            Self::Graphite => "Graphite",
            Self::Night => "Night grid",
            Self::Beige => "3 point beige",
            Self::Blueprint => "Blueprint",
            Self::Lightbox => "Lightbox studio",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|s| s.id().eq_ignore_ascii_case(name.trim()) || s.label().eq_ignore_ascii_case(name.trim()))
    }

    pub fn is_dark(self) -> bool {
        matches!(self, Self::Graphite | Self::Night | Self::Blueprint)
    }

    pub fn backdrop(self) -> Backdrop {
        let mut bg = match self {
            Self::White => Backdrop::linear(&[(0xffffff, 0.0), (0xffffff, 1.0)]),
            Self::Studio => Backdrop::linear(&[(0xf2f4f7, 0.0), (0xdde2e9, 0.46), (0xa5aebb, 1.0)]),
            Self::Faded => Backdrop::linear(&[
                (0xffffff, 0.0),
                (0xe6eaf2, 0.34),
                (0xdde3ed, 0.52),
                (0xf4f6f9, 0.78),
                (0xffffff, 1.0),
            ]),
            Self::Spot => Backdrop::linear(&[(0xffffff, 0.0), (0xeceff3, 0.48), (0xc2c9d3, 1.0)]),
            Self::Graphite => Backdrop::linear(&[(0x434a54, 0.0), (0x2a2f36, 0.55), (0x16191d, 1.0)]),
            Self::Night => Backdrop::linear(&[(0x232830, 0.0), (0x101317, 1.0)]),
            Self::Beige => Backdrop::linear(&[(0xfbf8f1, 0.0), (0xece3d1, 0.5), (0xc9b998, 1.0)]),
            Self::Blueprint => Backdrop::linear(&[(0x1e4f8f, 0.0), (0x123763, 1.0)]),
            Self::Lightbox => Backdrop::linear(&[(0xe9edf2, 0.0), (0xf9fafb, 0.58), (0xd5dae1, 0.59), (0xeef1f4, 1.0)]),
        };
        match self {
            Self::Spot => bg.radial = [0.5, 0.46, 0.62, 0.58],
            Self::Night => bg.grid = [32.0, 0.055],
            Self::Blueprint => bg.grid = [28.0, 0.09],
            _ => {}
        }
        bg
    }
}

/// Up to five gradient stops, an optional radial ellipse and a screen grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backdrop {
    pub stops: [(Color32, f32); 5],
    /// Centre and radii as fractions of viewport size; zero radii mean a vertical fade.
    pub radial: [f32; 4],
    /// Grid spacing in logical pixels and white line opacity; zero spacing disables it.
    pub grid: [f32; 2],
}

impl Backdrop {
    fn linear(stops: &[(u32, f32)]) -> Self {
        let mut result = Self { stops: [(Color32::WHITE, 1.0); 5], radial: [0.0; 4], grid: [0.0; 2] };
        for (i, &(rgb, at)) in stops.iter().enumerate() {
            result.stops[i] = (Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8), at);
        }
        let last = result.stops[stops.len() - 1];
        result.stops[stops.len()..].fill(last);
        result
    }
}

impl From<[Color32; 2]> for Backdrop {
    fn from([top, bottom]: [Color32; 2]) -> Self {
        Self {
            stops: [(top, 0.0), (bottom, 1.0), (bottom, 1.0), (bottom, 1.0), (bottom, 1.0)],
            radial: [0.0; 4],
            grid: [0.0; 2],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_names_and_stops_are_stable() {
        for scene in Scene::ALL {
            assert_eq!(Scene::parse(scene.id()), Some(scene));
            assert_eq!(Scene::parse(&scene.label().to_uppercase()), Some(scene));
            assert_eq!(serde_json::to_value(scene).unwrap(), scene.id());
            let bg = scene.backdrop();
            assert_eq!(bg.stops[0].1, 0.0);
            assert_eq!(bg.stops[4].1, 1.0);
            assert!(bg.stops.windows(2).all(|s| s[0].1 <= s[1].1));
        }
        assert_eq!(Scene::parse("unknown"), None);
        assert_eq!(Scene::ALL.into_iter().filter(|s| s.is_dark()).count(), 3);
    }
}
