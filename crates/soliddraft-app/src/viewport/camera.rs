//! Z-up orbit camera with smooth transitions to the standard views.

use egui::{Pos2, Rect};
use glam::{Mat4, Vec3, Vec4};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandardView {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Isometric,
}

impl StandardView {
    pub fn label(&self) -> &'static str {
        match self {
            StandardView::Front => "Front",
            StandardView::Back => "Back",
            StandardView::Left => "Left",
            StandardView::Right => "Right",
            StandardView::Top => "Top",
            StandardView::Bottom => "Bottom",
            StandardView::Isometric => "Isometric",
        }
    }

    /// Yaw and pitch in degrees. Yaw is measured from +X toward +Y; the
    /// viewer stands at `target + distance * dir(yaw, pitch)`.
    pub fn angles(&self) -> (f32, f32) {
        match self {
            StandardView::Front => (-90.0, 0.0),
            StandardView::Back => (90.0, 0.0),
            StandardView::Right => (0.0, 0.0),
            StandardView::Left => (180.0, 0.0),
            StandardView::Top => (-90.0, 90.0),
            StandardView::Bottom => (-90.0, -90.0),
            StandardView::Isometric => (-45.0, 35.264),
        }
    }
}

/// See [`Camera::depth_view`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DepthView {
    Isometric,
    /// Looks from this direction, named "Oblique".
    Oblique(Vec3),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Goal {
    yaw: f32,
    pitch: f32,
    target: Vec3,
    distance: f32,
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub target: Vec3,
    pub distance: f32,
    /// Radians, around +Z, measured from +X toward +Y.
    pub yaw: f32,
    /// Radians above the XY plane.
    pub pitch: f32,
    pub fov_y: f32,
    pub orthographic: bool,
    goal: Option<Goal>,
    pub view_name: &'static str,
    /// How long a move to a standard view takes, near enough: it is 99 % there by then.
    pub transition_seconds: f32,
    /// The bands of the viewport that overlays cover, top and bottom, in points, with
    /// the viewport's height: a fit frames the part between them. Set by
    /// [`Camera::set_framing`].
    framing: Framing,
    /// The bounds the last fit framed and the goal it set, so that a framing first known,
    /// or changed, while the fit is still under way can reframe them.
    fitted: Option<(Option<(Vec3, Vec3)>, Goal)>,
}

/// What a fit leaves clear: the bands at the viewport's top and bottom that the heads-up
/// pills, the navigation bar and a sketch's status cover.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Framing {
    pub top: f32,
    pub bottom: f32,
    pub height: f32,
}

/// The natural log of how far from its goal a transition may be when its time is up.
const SETTLE: f32 = 4.62;

impl Default for Camera {
    fn default() -> Self {
        let (yaw, pitch) = StandardView::Isometric.angles();
        Self {
            target: Vec3::ZERO,
            distance: 260.0,
            yaw: yaw.to_radians(),
            pitch: pitch.to_radians(),
            fov_y: 38.0_f32.to_radians(),
            orthographic: false,
            goal: None,
            view_name: "Isometric",
            transition_seconds: 0.33,
            framing: Framing::default(),
            fitted: None,
        }
    }
}

impl Camera {
    pub fn direction(&self) -> Vec3 {
        Vec3::new(self.pitch.cos() * self.yaw.cos(), self.pitch.cos() * self.yaw.sin(), self.pitch.sin())
    }

    /// Up vector that keeps the horizon level and stays well defined at the poles.
    pub fn up(&self) -> Vec3 {
        Vec3::new(-self.pitch.sin() * self.yaw.cos(), -self.pitch.sin() * self.yaw.sin(), self.pitch.cos()).normalize()
    }

    pub fn right(&self) -> Vec3 {
        self.forward().cross(self.up()).normalize()
    }

    pub fn forward(&self) -> Vec3 {
        -self.direction()
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.direction() * self.distance
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(self.eye(), self.target, self.up())
    }

    pub fn near_far(&self) -> (f32, f32) {
        (self.distance * 0.02, self.distance * 50.0)
    }

    /// Projection with reversed depth: the near plane maps to 1 and far (infinity for
    /// perspective) to 0, which spreads a float depth buffer's precision evenly.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        let (near, far) = self.near_far();
        if self.orthographic {
            let half_h = self.distance * (self.fov_y / 2.0).tan();
            let half_w = half_h * aspect;
            glam::camera::rh::proj::directx::orthographic(-half_w, half_w, -half_h, half_h, far, -far)
        } else {
            glam::camera::rh::proj::directx::perspective_infinite_reverse(self.fov_y, aspect, near)
        }
    }

    pub fn view_projection(&self, aspect: f32) -> Mat4 {
        self.projection(aspect) * self.view()
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        self.goal = None;
        self.view_name = "Custom";
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.008).clamp(-1.553, 1.553);
    }

    /// How many millimetres a point covers at the orbit target.
    pub fn mm_per_point(&self, viewport_height: f32) -> f32 {
        2.0 * self.distance * (self.fov_y / 2.0).tan() / viewport_height.max(1.0)
    }

    pub fn pan(&mut self, dx: f32, dy: f32, viewport_height: f32) {
        self.goal = None;
        let world_per_px = self.mm_per_point(viewport_height);
        self.target -= self.right() * dx * world_per_px;
        self.target += self.up() * dy * world_per_px;
    }

    /// Zoom by `factor` (> 1 zooms in) keeping the point under `ndc` fixed.
    pub fn zoom(&mut self, factor: f32, ndc: Option<(f32, f32)>, aspect: f32) {
        self.goal = None;
        let old = self.distance;
        let new = (old / factor).clamp(0.5, 100_000.0);
        if let Some((x, y)) = ndc {
            let t = (self.fov_y / 2.0).tan();
            let shift = (self.right() * x * aspect + self.up() * y) * (old - new) * t;
            self.target += shift;
        }
        self.distance = new;
    }

    pub fn set_view(&mut self, view: StandardView) {
        let (yaw, pitch) = view.angles();
        self.turn_to(view.label(), yaw, pitch);
    }

    /// Turns, animated, to look from `yaw` and `pitch` degrees, and names the view.
    pub fn turn_to(&mut self, name: &'static str, yaw: f32, pitch: f32) {
        self.view_name = name;
        self.goal = Some(Goal {
            yaw: yaw.to_radians(),
            pitch: pitch.to_radians(),
            target: self.target,
            distance: self.distance,
        });
    }

    /// Where the view looks from once any transition ends: [`Camera::direction`] at the
    /// goal.
    pub fn heading(&self) -> Vec3 {
        let (yaw, pitch) = self.goal.map_or((self.yaw, self.pitch), |g| (g.yaw, g.pitch));
        Vec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin())
    }

    /// Looks straight at a plane from the side its normal points to, animated.
    pub fn look_along(&mut self, normal: Vec3) {
        let n = normal.normalize_or_zero();
        if n == Vec3::ZERO {
            return;
        }
        let pitch = n.z.clamp(-1.0, 1.0).asin();
        let yaw = if n.x.abs() < 1e-6 && n.y.abs() < 1e-6 { -std::f32::consts::FRAC_PI_2 } else { n.y.atan2(n.x) };
        self.view_name = "Normal to";
        self.goal = Some(Goal { yaw, pitch: pitch.clamp(-1.553, 1.553), target: self.target, distance: self.distance });
    }

    /// Where a standard view looks from, as [`Camera::heading`] gives it.
    pub fn direction_of(view: StandardView) -> Vec3 {
        let (yaw, pitch) = view.angles();
        let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
        Vec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin())
    }

    /// The view that shows the depth of a feature growing along `normal`, a sketch plane's
    /// unit normal with `u` its U axis, when the view looks from `heading` within about
    /// five degrees of that normal: isometric, or, when the normal lies near the
    /// isometric line of sight itself, a view tilted off the normal towards both sketch
    /// axes. `None` when the view already shows depth.
    pub fn depth_view(heading: Vec3, normal: Vec3, u: Vec3) -> Option<DepthView> {
        /// The largest cosine to the normal at which depth shows well: about 26 degrees.
        const OBLIQUE: f32 = 0.9;
        let normal = normal.normalize_or_zero();
        let along = heading.normalize_or_zero().dot(normal);
        if along.abs() <= 0.996 {
            return None;
        }
        if Self::direction_of(StandardView::Isometric).dot(normal).abs() < OBLIQUE {
            return Some(DepthView::Isometric);
        }
        let u = u.normalize_or_zero();
        let side = if along < 0.0 { -1.0 } else { 1.0 };
        Some(DepthView::Oblique(side * normal + 0.7 * u + 0.5 * normal.cross(u)))
    }

    /// Turns, animated, to look from `from`, and names the view `name`.
    pub fn look_from(&mut self, name: &'static str, from: Vec3) {
        let n = from.normalize_or_zero();
        if n == Vec3::ZERO {
            return;
        }
        let pitch = n.z.clamp(-1.0, 1.0).asin().clamp(-1.553, 1.553);
        let yaw = if n.x.abs() < 1e-6 && n.y.abs() < 1e-6 { -std::f32::consts::FRAC_PI_2 } else { n.y.atan2(n.x) };
        self.turn_to(name, yaw.to_degrees(), pitch.to_degrees());
    }

    /// Frames `bounds` in the whole viewport, as an offscreen picture wants it. Kept as main
    /// had it: the edit speed job compares a picture through this path with main's pixel
    /// for pixel, and a release build folds this arithmetic differently once it changes.
    pub fn fit(&mut self, bounds: Option<(Vec3, Vec3)>) {
        let (center, radius) = match bounds {
            Some((lo, hi)) => ((lo + hi) / 2.0, ((hi - lo).length() / 2.0).max(1.0)),
            None => (Vec3::ZERO, 60.0),
        };
        let distance = radius / (self.fov_y / 2.0).sin() * 1.12;
        let (yaw, pitch) = self.goal.map(|g| (g.yaw, g.pitch)).unwrap_or((self.yaw, self.pitch));
        self.goal = Some(Goal { yaw, pitch, target: center, distance });
    }

    /// Frames `bounds` in the band of the viewport that the overlays leave clear (see
    /// [`Camera::set_framing`]), for the app's own viewport.
    pub fn fit_framed(&mut self, bounds: Option<(Vec3, Vec3)>) {
        // With nothing to show there is nothing to keep clear of the bands: an empty
        // document keeps the view a plain fit gives it.
        if bounds.is_none() {
            self.fitted = None;
            return self.fit(None);
        }
        let (center, radius) = match bounds {
            Some((lo, hi)) => ((lo + hi) / 2.0, ((hi - lo).length() / 2.0).max(1.0)),
            None => (Vec3::ZERO, 60.0),
        };
        let mut distance = radius / (self.fov_y / 2.0).sin() * 1.12;
        let (yaw, pitch) = self.goal.map(|g| (g.yaw, g.pitch)).unwrap_or((self.yaw, self.pitch));
        // Frame the part in the band the overlays leave clear, centred in it.
        let Framing { top, bottom, height } = self.framing;
        let clear = height - top - bottom;
        let mut target = center;
        if height > 0.0 && clear > height * 0.3 {
            let up = Vec3::new(-pitch.sin() * yaw.cos(), -pitch.sin() * yaw.sin(), pitch.cos()).normalize_or_zero();
            // How far the part reaches up and down the screen from its middle. The fit
            // above holds the part's whole sphere; it zooms out further only when that
            // reach would not fit the clear band, as with a tall sketch.
            let reach = match bounds {
                Some((lo, hi)) => (0..8)
                    .map(|i| {
                        let corner = Vec3::new(
                            if i & 1 == 0 { lo.x } else { hi.x },
                            if i & 2 == 0 { lo.y } else { hi.y },
                            if i & 4 == 0 { lo.z } else { hi.z },
                        );
                        (corner - center).dot(up).abs()
                    })
                    .fold(0.0, f32::max),
                None => radius,
            };
            let tan = (self.fov_y / 2.0).tan();
            distance = distance.max(reach * 1.1 * height / clear / tan);
            let mm_per_point = 2.0 * distance * tan / height;
            // The band's centre is (top - bottom) / 2 points below the viewport's.
            target += up * (top - bottom) / 2.0 * mm_per_point;
        }
        let goal = Goal { yaw, pitch, target, distance };
        self.goal = Some(goal);
        self.fitted = Some((bounds, goal));
    }

    /// What a fit leaves clear, from the frame just laid out. A fit made before the
    /// viewport's first frame, or before a sketch's status showed, reframes while its move
    /// is still under way; once it has landed, or the view has moved since, it stands.
    pub fn set_framing(&mut self, framing: Framing) {
        if framing == self.framing {
            return;
        }
        self.framing = framing;
        if let Some((bounds, goal)) = self.fitted
            && self.goal == Some(goal)
        {
            self.fit_framed(bounds);
        }
    }

    pub fn framing(&self) -> Framing {
        self.framing
    }

    /// Advances any view transition. Returns true while still animating.
    pub fn tick(&mut self, dt: f32) -> bool {
        let Some(goal) = self.goal else { return false };
        let k = if self.transition_seconds > 0.0 { 1.0 - (-dt * SETTLE / self.transition_seconds).exp() } else { 1.0 };
        let dyaw = wrap_angle(goal.yaw - self.yaw);
        self.yaw += dyaw * k;
        self.pitch += (goal.pitch - self.pitch) * k;
        self.target += (goal.target - self.target) * k;
        self.distance += (goal.distance - self.distance) * k;
        let done = dyaw.abs() < 0.0005
            && (goal.pitch - self.pitch).abs() < 0.0005
            && (goal.target - self.target).length() < 0.01 * self.distance
            && (goal.distance - self.distance).abs() < 0.001 * self.distance;
        if done {
            self.yaw = wrap_angle(goal.yaw);
            self.pitch = goal.pitch;
            self.target = goal.target;
            self.distance = goal.distance;
            self.goal = None;
        }
        true
    }

    /// World-space ray through a viewport pixel.
    pub fn ray(&self, pos: Pos2, rect: Rect) -> (Vec3, Vec3) {
        let ndc_x = (pos.x - rect.left()) / rect.width() * 2.0 - 1.0;
        let ndc_y = 1.0 - (pos.y - rect.top()) / rect.height() * 2.0;
        let aspect = rect.width() / rect.height().max(1.0);
        let t = (self.fov_y / 2.0).tan();
        let (right, up, fwd) = (self.right(), self.up(), self.forward());
        if self.orthographic {
            let origin = self.eye() + right * ndc_x * t * self.distance * aspect + up * ndc_y * t * self.distance;
            (origin - fwd * self.distance * 10.0, fwd)
        } else {
            let dir = (fwd + right * ndc_x * t * aspect + up * ndc_y * t).normalize();
            (self.eye(), dir)
        }
    }

    /// Projects a world point to viewport pixels. `None` when behind the eye.
    pub fn project(&self, p: Vec3, rect: Rect) -> Option<Pos2> {
        let aspect = rect.width() / rect.height().max(1.0);
        let clip = self.view_projection(aspect) * Vec4::new(p.x, p.y, p.z, 1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Pos2::new(
            rect.left() + (ndc.x + 1.0) / 2.0 * rect.width(),
            rect.top() + (1.0 - ndc.y) / 2.0 * rect.height(),
        ))
    }
}

fn wrap_angle(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut a = a % tau;
    if a > std::f32::consts::PI {
        a -= tau;
    } else if a < -std::f32::consts::PI {
        a += tau;
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_view_looks_along_plus_y_with_z_up() {
        let mut c = Camera::default();
        c.set_view(StandardView::Front);
        for _ in 0..200 {
            c.tick(0.016);
        }
        assert!((c.forward() - Vec3::Y).length() < 1e-3);
        assert!((c.up() - Vec3::Z).length() < 1e-3);
    }

    #[test]
    fn top_view_keeps_y_up_on_screen() {
        let mut c = Camera::default();
        c.set_view(StandardView::Top);
        for _ in 0..200 {
            c.tick(0.016);
        }
        assert!((c.forward() + Vec3::Z).length() < 1e-3);
        assert!((c.up() - Vec3::Y).length() < 1e-3);
    }

    #[test]
    fn depth_is_reversed_in_both_projections() {
        let mut c = Camera::default();
        for ortho in [false, true] {
            c.orthographic = ortho;
            let m = c.view_projection(1.5);
            let depth = |p: Vec3| {
                let v = m * Vec4::new(p.x, p.y, p.z, 1.0);
                v.z / v.w
            };
            let near = depth(c.eye() + c.forward() * c.near_far().0 * 1.01);
            let target = depth(c.target);
            let far = depth(c.target + c.forward() * c.distance * 10.0);
            assert!(near > target && target > far && far > 0.0 && near <= 1.0, "{ortho}: {near} {target} {far}");
        }
    }

    #[test]
    fn ray_through_centre_hits_target() {
        let c = Camera::default();
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0));
        let (o, d) = c.ray(Pos2::new(400.0, 300.0), rect);
        let closest = o + d * (c.target - o).dot(d);
        assert!((closest - c.target).length() < 1e-3);
    }

    /// Looking straight down at a flat part: a wide one keeps the zoom a fit gave it before
    /// the bands, only moved up into the clear band; a tall one zooms out until its top and
    /// bottom sit inside that band.
    #[test]
    fn a_fit_zooms_out_only_as_far_as_the_bands_need() {
        let framing = Framing { top: 60.0, bottom: 84.0, height: 600.0 };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, framing.height));
        let fitted = |bounds: (Vec3, Vec3), framed: bool| {
            let mut c = Camera { orthographic: true, ..Camera::default() };
            c.look_along(Vec3::Z);
            while c.tick(1.0 / 60.0) {}
            if framed {
                c.set_framing(framing);
            }
            c.fit_framed(Some(bounds));
            while c.tick(1.0 / 60.0) {}
            c
        };
        let wide = (Vec3::new(-100.0, -20.0, 0.0), Vec3::new(100.0, 20.0, 0.0));
        let (plain, framed) = (fitted(wide, false), fitted(wide, true));
        assert!((plain.distance - framed.distance).abs() < 1e-3, "a wide part keeps its zoom");
        let tall = (Vec3::new(-20.0, -100.0, 0.0), Vec3::new(20.0, 100.0, 0.0));
        let framed = fitted(tall, true);
        assert!(framed.distance > fitted(tall, false).distance, "a tall part zooms out");
        for y in [-100.0, 100.0] {
            let at = framed.project(Vec3::new(0.0, y, 0.0), rect).unwrap();
            assert!(
                at.y > framing.top && at.y < framing.height - framing.bottom,
                "{y} lands at {} outside the clear band",
                at.y
            );
        }
    }

    /// The app fits the part before its first frame, when the viewport's bands are not
    /// known yet: the framing that arrives during the move still applies, and a later
    /// fit of the same part has nowhere to go.
    #[test]
    fn a_fit_made_before_the_first_frame_takes_the_framing_that_follows() {
        let bounds = Some((Vec3::splat(-20.0), Vec3::splat(20.0)));
        let framing = Framing { top: 8.0, bottom: 84.0, height: 600.0 };
        let mut early = Camera::default();
        early.fit_framed(bounds);
        early.tick(1.0 / 60.0);
        early.set_framing(framing);
        while early.tick(1.0 / 60.0) {}
        let mut late = Camera::default();
        late.set_framing(framing);
        late.fit_framed(bounds);
        while late.tick(1.0 / 60.0) {}
        assert!((early.target - late.target).length() < 1e-3 && (early.distance - late.distance).abs() < 1e-3);
        let landed = (early.target, early.distance);
        early.fit_framed(bounds);
        assert!(!early.tick(1.0 / 60.0) || (early.target - landed.0).length() < 1e-3, "a second fit stays put");
        // Once a fit has landed a new framing leaves the view where the user sees it.
        early.set_framing(Framing { top: 60.0, ..framing });
        assert!(!early.tick(1.0 / 60.0), "a landed fit does not move again");
    }

    /// A sketch normal on the isometric line of sight must not turn the view to
    /// isometric, which would still look straight down it.
    #[test]
    fn a_feature_on_any_plane_gets_a_view_oblique_to_its_normal() {
        let iso = Camera::direction_of(StandardView::Isometric);
        let tilted = (Vec3::new(1.0, -1.0, 1.0) + Vec3::new(0.03, 0.02, -0.01)).normalize();
        let planes = [
            (Vec3::Z, Vec3::X),
            (-Vec3::Y, Vec3::X),
            (Vec3::new(1.0, -1.0, 1.0).normalize(), Vec3::new(1.0, 1.0, 0.0).normalize()),
            (tilted, tilted.any_orthonormal_vector()),
            (Vec3::new(0.0, -1.0, 1.0).normalize(), Vec3::X),
        ];
        for (normal, u) in planes {
            for side in [1.0, -1.0] {
                let view = Camera::depth_view(side * normal, normal, u).expect("the view on the normal turns");
                let from = match view {
                    DepthView::Isometric => iso,
                    DepthView::Oblique(from) => from.normalize(),
                };
                let mut camera = Camera::default();
                camera.look_from("Oblique", from);
                let cos = camera.heading().dot(normal).abs();
                assert!(
                    cos < 0.9,
                    "{normal} from side {side}: the view stays within {:.1} degrees",
                    cos.acos().to_degrees()
                );
                if let DepthView::Oblique(from) = view {
                    assert!(from.dot(side * normal) > 0.0, "the view stays on the side the sketch was seen from");
                }
            }
            assert_eq!(Camera::depth_view(iso.cross(normal).normalize_or(Vec3::X), normal, u), None);
        }
        assert_eq!(Camera::depth_view(Vec3::Z, Vec3::Z, Vec3::X), Some(DepthView::Isometric));
        assert!(matches!(
            Camera::depth_view(iso, Vec3::new(1.0, -1.0, 1.0).normalize(), Vec3::X.cross(iso).normalize()),
            Some(DepthView::Oblique(_))
        ));
    }
}
