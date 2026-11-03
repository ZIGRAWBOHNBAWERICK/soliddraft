//! The modelling grid: a lattice on the ground, or on the open sketch's plane, whose
//! spacing follows the zoom or stays fixed, as the user chose. The renderer draws a
//! [`GridFrame`] and the viewport's pill names the same [`Spacing`], so the two cannot
//! disagree. [`GridStyle`] is the user's choice of strength, spacing and colour.

use super::camera::Camera;
use glam::{DVec3, Vec3, dvec3};
use serde_json::{Value, json};
use soliddraft_kernel_api::Plane;
use std::ops::RangeInclusive;

/// The major lines' opacity at full strength. The grid of the first releases drew them at
/// 0.36, which is a strength of 0.6.
const FULL_OPACITY: f32 = 0.6;
/// Minor lines are drawn at this share of the major lines' opacity.
const MINOR_SHARE: f32 = 0.45;
/// A line over the dark theme's background stands out about twice as much as the same
/// opacity over the light one, so the dark theme draws at this share, and one strength
/// looks alike in both.
const DARK_SHARE: f32 = 0.45;
/// Line widths in points: minor, then major.
pub const LINE_WIDTH: (f32, f32) = (1.0, 1.5);

/// How the grid looks: the user's choice, kept in the `grid` keys of the settings' `view`
/// section. Each key falls back on its own, so a value this build cannot read costs only
/// that value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridStyle {
    /// How strongly the lines show, in [`GridStyle::STRENGTH`].
    pub strength: f32,
    /// Whether the spacing follows the zoom. When it does not, minor lines are
    /// `fixed_mm` apart.
    pub follow_zoom: bool,
    /// While the spacing follows the zoom: the least gap between minor lines on screen, in
    /// points. The grid takes the finest step of 1, 2 or 5 times a power of ten that keeps
    /// them this far apart.
    pub gap: f32,
    /// The minor lines' spacing in millimetres when it does not follow the zoom.
    pub fixed_mm: f64,
    /// Every this-many minor lines is a major line.
    pub major_every: u32,
    /// The lines' colour; `None` follows the interface theme.
    pub color: Option<egui::Color32>,
}

impl Default for GridStyle {
    fn default() -> Self {
        Self { strength: 0.3, follow_zoom: true, gap: 16.0, fixed_mm: 10.0, major_every: 5, color: None }
    }
}

impl GridStyle {
    pub const STRENGTH: RangeInclusive<f32> = 0.05..=1.0;
    pub const GAP: RangeInclusive<f32> = 8.0..=64.0;
    pub const FIXED_MM: RangeInclusive<f64> = 0.01..=10_000.0;
    pub const MAJOR_EVERY: RangeInclusive<u32> = 2..=10;

    /// The minor and the major lines' opacity over a light background.
    pub fn opacity(&self) -> (f32, f32) {
        let major = self.strength * FULL_OPACITY;
        (major * MINOR_SHARE, major)
    }

    /// The minor and the major lines' opacity in the light or the dark theme.
    pub fn opacity_in(&self, dark: bool) -> (f32, f32) {
        let (minor, major) = self.opacity();
        if dark { (minor * DARK_SHARE, major * DARK_SHARE) } else { (minor, major) }
    }

    /// The style as the settings file keeps it.
    pub fn to_json(&self) -> Value {
        json!({
            "strength": self.strength,
            "follow_zoom": self.follow_zoom,
            "gap": self.gap,
            "fixed_mm": self.fixed_mm,
            "major_every": self.major_every,
            "color": self.color.map(|c| c.to_hex()[..7].to_owned()),
        })
    }

    /// The style in `value`, each key falling back to its default on its own when it is
    /// missing or not a value this build accepts; numbers are kept within their ranges.
    pub fn from_json(value: &Value) -> GridStyle {
        let d = GridStyle::default();
        let number = |key: &str| value[key].as_f64().filter(|n| n.is_finite());
        let clamp = |n: f64, r: RangeInclusive<f32>| (n as f32).clamp(*r.start(), *r.end());
        GridStyle {
            strength: number("strength").map_or(d.strength, |n| clamp(n, Self::STRENGTH)),
            follow_zoom: value["follow_zoom"].as_bool().unwrap_or(d.follow_zoom),
            gap: number("gap").map_or(d.gap, |n| clamp(n, Self::GAP)),
            fixed_mm: number("fixed_mm")
                .map_or(d.fixed_mm, |n| n.clamp(*Self::FIXED_MM.start(), *Self::FIXED_MM.end())),
            major_every: value["major_every"].as_u64().map_or(d.major_every, |n| n.clamp(2, 10) as u32),
            color: match &value["color"] {
                Value::String(hex) if hex.len() == 7 => egui::Color32::from_hex(hex).ok(),
                _ => None,
            },
        }
    }
}

/// The lattice's spacing in millimetres. With the spacing following the zoom it is a
/// function of the view's scale alone, so the same zoom always gives the same grid and a
/// resting camera cannot make it flicker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    minor: f64,
    every: u32,
    /// Decimals the readout needs to name a minor step exactly.
    decimals: usize,
}

impl Spacing {
    /// The spacing `style` gives at `mm_per_point`. `None` for a scale that is not a
    /// positive number.
    pub fn for_scale(mm_per_point: f32, style: &GridStyle) -> Option<Spacing> {
        if !mm_per_point.is_finite() || mm_per_point <= 0.0 {
            return None;
        }
        let every = style.major_every.clamp(*GridStyle::MAJOR_EVERY.start(), *GridStyle::MAJOR_EVERY.end());
        if !style.follow_zoom {
            let minor = style.fixed_mm.clamp(*GridStyle::FIXED_MM.start(), *GridStyle::FIXED_MM.end());
            let decimals = (0..4).find(|d| ((minor * 10f64.powi(*d)).round() - minor * 10f64.powi(*d)).abs() < 1e-6);
            return Some(Spacing { minor, every, decimals: decimals.unwrap_or(4) as usize });
        }
        let wanted = f64::from(mm_per_point) * f64::from(style.gap.max(1.0));
        let mut exponent = wanted.log10().floor() as i32;
        let digit = match [1, 2, 5].into_iter().find(|digit| f64::from(*digit) * 10f64.powi(exponent) >= wanted) {
            Some(digit) => digit,
            None => {
                exponent += 1;
                1
            }
        };
        Some(Spacing { minor: f64::from(digit) * 10f64.powi(exponent), every, decimals: (-exponent).max(0) as usize })
    }

    pub fn minor(&self) -> f64 {
        self.minor
    }

    pub fn major(&self) -> f64 {
        self.minor * f64::from(self.every)
    }

    fn mm(&self, value: f64) -> String {
        let text = format!("{:.*}", self.decimals, value);
        if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_owned() } else { text }
    }

    /// What the viewport's pill says.
    pub fn label(&self) -> String {
        format!("Grid {} mm / {} mm", self.mm(self.minor()), self.mm(self.major()))
    }

    /// The same spacing in words, for screen readers and the pill's tooltip.
    pub fn description(&self) -> String {
        format!("Grid spacing: minor {} mm, major {} mm", self.mm(self.minor()), self.mm(self.major()))
    }
}

/// The plane a grid lies in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridPlane {
    origin: DVec3,
    u: DVec3,
    v: DVec3,
    /// How far the grid is moved toward the viewer, in the edges' pull. The ground goes
    /// behind a face lying on it; a sketch comes in front of the face it is on.
    pull: f32,
}

impl GridPlane {
    pub const GROUND: GridPlane = GridPlane { origin: DVec3::ZERO, u: DVec3::X, v: DVec3::Y, pull: -0.5 };

    pub fn of_sketch(plane: &Plane) -> GridPlane {
        let dvec = |v: soliddraft_kernel_api::Vec3| dvec3(v.x, v.y, v.z);
        GridPlane { origin: dvec(plane.origin), u: dvec(plane.x_dir), v: dvec(plane.y_dir()), pull: 0.5 }
    }
}

/// One frame's grid as the renderer draws it, in view space: all the shader's arithmetic
/// is relative to the eye and to a lattice point near the view, so it keeps its precision
/// wherever the camera is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridFrame {
    /// A major intersection of the lattice near the view's target.
    pub anchor: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub minor: f32,
    pub major: f32,
    pub pull: f32,
    pub ink: egui::Color32,
    /// The minor and the major lines' opacity.
    pub opacity: (f32, f32),
}

impl GridFrame {
    /// `None` when the camera is not at a finite place.
    pub fn new(
        plane: &GridPlane,
        spacing: Spacing,
        camera: &Camera,
        ink: egui::Color32,
        opacity: (f32, f32),
    ) -> Option<GridFrame> {
        let major = spacing.major();
        let from = camera.target.as_dvec3() - plane.origin;
        let snap = |axis: DVec3| (from.dot(axis) / major).round() * major;
        let anchor = plane.origin + plane.u * snap(plane.u) + plane.v * snap(plane.v);
        let (right, up, back) = (camera.right().as_dvec3(), camera.up().as_dvec3(), camera.direction().as_dvec3());
        let in_view = |d: DVec3| dvec3(d.dot(right), d.dot(up), d.dot(back)).as_vec3();
        let frame = GridFrame {
            anchor: in_view(anchor - camera.eye().as_dvec3()),
            u: in_view(plane.u),
            v: in_view(plane.v),
            minor: spacing.minor() as f32,
            major: major as f32,
            pull: plane.pull,
            ink,
            opacity,
        };
        (frame.anchor.is_finite() && frame.u.is_finite() && frame.v.is_finite()).then_some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: egui::Color32 = egui::Color32::BLACK;

    /// The world point a view-space point of `frame` stands for.
    fn world(camera: &Camera, p: Vec3) -> Vec3 {
        camera.eye() + camera.right() * p.x + camera.up() * p.y + camera.direction() * p.z
    }

    /// A step of `digit` times ten to the `exponent`, major every five.
    fn step(digit: u32, exponent: i32) -> Spacing {
        Spacing { minor: f64::from(digit) * 10f64.powi(exponent), every: 5, decimals: (-exponent).max(0) as usize }
    }

    fn frame(plane: &GridPlane, spacing: Spacing, camera: &Camera) -> Option<GridFrame> {
        GridFrame::new(plane, spacing, camera, INK, GridStyle::default().opacity())
    }

    #[test]
    fn spacing_stays_in_range_over_every_zoom() {
        // The camera's whole range, from 0.5 mm away on a tall view to 100 m on a tiny one.
        let scales = [0.5 * 0.7 / 2000.0, 1e-4, 0.03, 0.2, 1.0, 37.0, 100_000.0 * 0.7 / 10.0];
        for gap in [8.0, 16.0, 40.0, 64.0] {
            let style = GridStyle { gap, ..Default::default() };
            for scale in scales {
                let s = Spacing::for_scale(scale, &style).unwrap();
                let points = s.minor() / f64::from(scale);
                let most = f64::from(gap) * 2.5;
                assert!((f64::from(gap)..=most).contains(&points), "gap {gap}, {scale} mm per point: {points} points");
                assert!(s.minor().is_finite() && s.minor() > 0.0);
                assert_eq!(s.major() / s.minor(), 5.0);
            }
        }
    }

    #[test]
    fn spacing_on_both_sides_of_a_step_stays_in_range() {
        let style = GridStyle::default();
        // 5 mm minors are exactly 16 points apart at this scale.
        let at = 5.0 / 16.0_f32;
        for scale in [at * 0.999, at, at * 1.001] {
            let s = Spacing::for_scale(scale, &style).unwrap();
            let points = s.minor() / f64::from(scale);
            assert!((15.9..=40.1).contains(&points), "{scale}: {points}");
        }
        assert_ne!(Spacing::for_scale(at * 0.99, &style), Spacing::for_scale(at * 1.01, &style));
        assert_eq!(
            Spacing::for_scale(at * 0.99, &style),
            Spacing::for_scale(at * 0.99, &style),
            "the same scale, the same grid"
        );
    }

    #[test]
    fn a_wider_gap_gives_a_coarser_grid() {
        let scale = 0.1;
        let fine = Spacing::for_scale(scale, &GridStyle { gap: 8.0, ..Default::default() }).unwrap();
        let coarse = Spacing::for_scale(scale, &GridStyle { gap: 64.0, ..Default::default() }).unwrap();
        assert!(coarse.minor() > fine.minor() * 4.0, "{fine:?} {coarse:?}");
    }

    #[test]
    fn a_fixed_spacing_ignores_the_zoom() {
        let style = GridStyle { follow_zoom: false, fixed_mm: 2.5, major_every: 4, ..Default::default() };
        for scale in [1e-3, 0.2, 50.0] {
            let s = Spacing::for_scale(scale, &style).unwrap();
            assert_eq!(s.minor(), 2.5);
            assert_eq!(s.major(), 10.0);
            assert_eq!(s.label(), "Grid 2.5 mm / 10 mm");
        }
        let tiny = GridStyle { follow_zoom: false, fixed_mm: 0.0, ..Default::default() };
        assert_eq!(Spacing::for_scale(1.0, &tiny).unwrap().minor(), 0.01, "a spacing is kept within its range");
    }

    #[test]
    fn a_scale_that_is_not_a_positive_number_gives_no_grid() {
        for style in [GridStyle::default(), GridStyle { follow_zoom: false, ..Default::default() }] {
            for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert_eq!(Spacing::for_scale(scale, &style), None, "{scale}");
            }
        }
    }

    #[test]
    fn the_readout_names_both_families_without_float_noise() {
        assert_eq!(step(5, 0).label(), "Grid 5 mm / 25 mm");
        assert_eq!(step(2, 1).label(), "Grid 20 mm / 100 mm");
        assert_eq!(step(1, -1).label(), "Grid 0.1 mm / 0.5 mm");
        assert_eq!(step(2, -3).label(), "Grid 0.002 mm / 0.01 mm");
        assert_eq!(step(5, 2).description(), "Grid spacing: minor 500 mm, major 2500 mm");
    }

    #[test]
    fn the_default_grid_is_lighter_than_the_first_one() {
        let (minor, major) = GridStyle::default().opacity();
        assert!(major < 0.36 * 0.6 && minor < 0.16 * 0.6, "{minor} {major}");
        assert!(minor > 0.0 && minor < major);
        let (_, full) = GridStyle { strength: 1.0, ..Default::default() }.opacity();
        assert!(full > 0.36, "full strength is stronger than the first grid");
        let (dark_minor, dark_major) = GridStyle::default().opacity_in(true);
        assert!(dark_major < major && dark_minor < minor, "the dark theme draws lighter lines");
        assert_eq!(GridStyle::default().opacity_in(false), (minor, major));
    }

    #[test]
    fn the_style_round_trips_and_each_key_falls_back_alone() {
        let style = GridStyle {
            strength: 0.55,
            follow_zoom: false,
            gap: 30.0,
            fixed_mm: 12.5,
            major_every: 4,
            color: Some(egui::Color32::from_rgb(0x2a, 0x6f, 0xdb)),
        };
        let json = style.to_json();
        assert_eq!(json["color"], "#2a6fdb");
        assert_eq!(GridStyle::from_json(&json), style);
        let d = GridStyle::default();
        assert_eq!(GridStyle::from_json(&Value::Null), d);
        let mixed = json!({"strength": "loud", "follow_zoom": false, "gap": 1000, "major_every": 1, "color": "blue"});
        let back = GridStyle::from_json(&mixed);
        assert_eq!(back.strength, d.strength, "a wrong type falls back");
        assert!(!back.follow_zoom, "a good key next to bad ones is kept");
        assert_eq!(back.gap, *GridStyle::GAP.end(), "a number out of range is clamped");
        assert_eq!(back.major_every, 2);
        assert_eq!(back.color, None);
    }

    #[test]
    fn the_anchor_is_on_the_lattice_whatever_the_sign_of_the_coordinates() {
        let spacing = step(5, 0);
        for target in [
            Vec3::ZERO,
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(-1.0, -2.0, 0.0),
            Vec3::new(-73.0, 41.0, 9.0),
            Vec3::new(-1e6, 1e6, 0.0),
        ] {
            let mut camera = Camera::default();
            camera.target = target;
            let frame = frame(&GridPlane::GROUND, spacing, &camera).unwrap();
            let anchor = world(&camera, frame.anchor);
            let tolerance = 1e-5 * target.length().max(10.0);
            assert!(anchor.z.abs() < tolerance, "{target}: {anchor}");
            for along in [anchor.x, anchor.y] {
                let off = along.rem_euclid(25.0);
                assert!(off.min(25.0 - off) < tolerance, "{target}: {anchor}");
            }
        }
    }

    #[test]
    fn a_sketch_grid_keeps_the_plane_frame_and_origin() {
        let plane = Plane {
            origin: soliddraft_kernel_api::Vec3::new(25.0, 0.0, 0.0),
            normal: soliddraft_kernel_api::Vec3::X,
            x_dir: soliddraft_kernel_api::Vec3::Y,
        };
        let camera = Camera::default();
        let frame = frame(&GridPlane::of_sketch(&plane), step(2, 0), &camera).unwrap();
        let direction = |v: Vec3| camera.right() * v.x + camera.up() * v.y + camera.direction() * v.z;
        assert!(direction(frame.u).distance(Vec3::Y) < 1e-6);
        assert!(direction(frame.v).distance(Vec3::Z) < 1e-6);
        assert!((world(&camera, frame.anchor).x - 25.0).abs() < 1e-3, "the anchor is in the plane");
    }

    #[test]
    fn a_camera_that_is_not_finite_gives_no_grid() {
        let mut camera = Camera::default();
        camera.target = Vec3::NAN;
        assert_eq!(frame(&GridPlane::GROUND, step(1, 0), &camera), None);
    }
}
