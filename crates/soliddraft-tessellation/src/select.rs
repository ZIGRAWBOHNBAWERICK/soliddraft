//! Screen-space level choice, per occurrence and per face of a single part.
//!
//! Each placement of a definition picks one of the definition's three levels from how
//! large its deviation looks on screen. An occurrence moves to a finer level as soon as
//! its error covers more than about a pixel, and back to a coarser one only after its
//! error has stayed under about a tenth of a pixel for a while. The gap between the two
//! thresholds, with levels a factor of four apart, keeps an occurrence from flickering
//! between levels while the camera moves.
//!
//! The adapter's display budget sets a ceiling from the meshes already held. It can
//! change which level is shown, not the parameters or mesh of any level.
//!
//! A single part chooses per face. Each face has a [`Choice`] of its own and moves by
//! the same rule at the scale of its own bounding box, while any part of that box is in
//! the [`View`]. A face out of view is never moved to a finer level: it counts as quiet and
//! steps down after the wait. Zooming in on one end of a part therefore refines the
//! faces on screen and leaves the rest as they were.

use soliddraft_kernel_api::{Aabb, Vec3};

use crate::level::Level;
use crate::policy::Deviations;

/// How many screen pixels one millimetre covers, for the camera of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// A perspective camera.
    Perspective {
        /// Where the eye is, in the same frame as the occurrences.
        eye: Vec3,
        /// The vertical field of view, in radians.
        fov_y: f64,
        /// The viewport's height in pixels.
        height_px: f64,
    },
    /// A parallel camera.
    Orthographic {
        /// Pixels per millimetre everywhere in the view.
        pixels_per_mm: f64,
    },
}

impl Projection {
    /// Pixels per millimetre at the part of a bounding sphere nearest the eye. A sphere
    /// around the eye counts as filling the screen at one millimetre's distance.
    pub fn pixels_per_mm(&self, center: Vec3, radius: f64) -> f64 {
        match *self {
            Projection::Orthographic { pixels_per_mm } => pixels_per_mm,
            Projection::Perspective { eye, fov_y, height_px } => {
                let distance = (center.distance(eye) - radius.max(0.0)).max(1.0);
                height_px / (2.0 * (fov_y / 2.0).tan() * distance)
            }
        }
    }
}

/// One side of what a camera shows: a point `p` is inside when
/// `normal · p + offset >= 0`. The normal is a unit vector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipPlane {
    /// The unit normal, pointing into the view.
    pub normal: Vec3,
    /// The plane's offset along the normal.
    pub offset: f64,
}

impl ClipPlane {
    /// The plane through `point` with `inward` pointing into the view. `None` when
    /// `inward` has no direction.
    pub fn through(point: Vec3, inward: Vec3) -> Option<ClipPlane> {
        let normal = inward.normalized()?;
        Some(ClipPlane { normal, offset: -normal.dot(point) })
    }
}

/// The camera of one frame and the part of space it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// The camera's scale.
    pub projection: Projection,
    /// The sides of the view volume. A view with no planes shows everything.
    pub planes: Vec<ClipPlane>,
}

impl View {
    /// A perspective camera at `eye` looking at `target`, with `up` towards the top of
    /// a viewport `width_px` by `height_px`. Its four sides and the plane through the eye
    /// bound the view; nothing is too far to show. A camera with no direction shows
    /// everything.
    pub fn perspective(eye: Vec3, target: Vec3, up: Vec3, fov_y: f64, width_px: f64, height_px: f64) -> View {
        let projection = Projection::Perspective { eye, fov_y, height_px };
        let Some((forward, right, up)) = frame(target - eye, up) else {
            return View { projection, planes: Vec::new() };
        };
        let tan_y = (fov_y / 2.0).tan();
        let tan_x = tan_y * width_px / height_px;
        let sides = [forward * tan_x - right, forward * tan_x + right, forward * tan_y - up, forward * tan_y + up];
        let planes = sides.into_iter().chain([forward]).filter_map(|inward| ClipPlane::through(eye, inward)).collect();
        View { projection, planes }
    }

    /// A parallel camera centred on `center`, looking along `forward`, with `up` towards
    /// the top of a viewport `width_px` by `height_px`. A camera with no direction shows
    /// everything.
    pub fn orthographic(
        center: Vec3,
        forward: Vec3,
        up: Vec3,
        pixels_per_mm: f64,
        width_px: f64,
        height_px: f64,
    ) -> View {
        let projection = Projection::Orthographic { pixels_per_mm };
        let Some((_, right, up)) = frame(forward, up) else {
            return View { projection, planes: Vec::new() };
        };
        let (half_width, half_height) = (width_px / (2.0 * pixels_per_mm), height_px / (2.0 * pixels_per_mm));
        let sides = [(right, half_width), (up, half_height)];
        let planes = sides
            .into_iter()
            .flat_map(|(axis, half)| [(axis, half), (-axis, half)])
            .filter_map(|(axis, half)| ClipPlane::through(center + axis * half, -axis))
            .collect();
        View { projection, planes }
    }

    /// True when any part of a sphere is in view.
    pub fn shows(&self, center: Vec3, radius: f64) -> bool {
        self.planes.iter().all(|plane| plane.normal.dot(center) + plane.offset >= -radius.max(0.0))
    }

    /// True when a box may be in view: no side of the view has the whole box outside it.
    pub fn shows_box(&self, bbox: &Aabb) -> bool {
        self.planes.iter().all(|plane| {
            let n = plane.normal;
            let pick = |along: f64, lo: f64, hi: f64| if along >= 0.0 { hi } else { lo };
            let inmost = Vec3::new(
                pick(n.x, bbox.min.x, bbox.max.x),
                pick(n.y, bbox.min.y, bbox.max.y),
                pick(n.z, bbox.min.z, bbox.max.z),
            );
            n.dot(inmost) + plane.offset >= 0.0
        })
    }

    /// How many pixels a millimetre covers at a face with this bounding box, taken at
    /// the near side of the sphere around the box, or zero when the box is out of view.
    pub fn pixels_per_mm(&self, face: &Aabb) -> f64 {
        if self.shows_box(face) { self.projection.pixels_per_mm(face.center(), face.diagonal() / 2.0) } else { 0.0 }
    }
}

/// The camera's forward, right and up directions, or `None` when `forward` has no
/// direction or `up` lies along it.
fn frame(forward: Vec3, up: Vec3) -> Option<(Vec3, Vec3, Vec3)> {
    let forward = forward.normalized()?;
    let right = forward.cross(up).normalized()?;
    Some((forward, right, right.cross(forward)))
}

/// When to change level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    /// Promote when the current level's error on screen is more than this, in pixels.
    pub promote_px: f64,
    /// Demote when the current level's error has stayed under this, in pixels...
    pub demote_px: f64,
    /// ...for this long, in seconds.
    pub demote_after_s: f32,
}

impl Default for Thresholds {
    fn default() -> Thresholds {
        Thresholds { promote_px: 1.0, demote_px: 0.1, demote_after_s: 2.0 }
    }
}

/// One occurrence's level and how long it has been a candidate for demotion. Eight
/// bytes, so a million occurrences keep their choices in 8 MB.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Choice {
    /// The level drawn.
    pub level: Level,
    /// Seconds the error has stayed under the demotion threshold.
    pub quiet_s: f32,
}

impl Thresholds {
    /// The level a newly shown occurrence starts at: the coarsest whose error is within
    /// the promotion threshold, at most `ceiling`.
    pub fn initial(&self, levels: &impl Deviations, pixels_per_mm: f64, ceiling: Level) -> Choice {
        Choice { level: self.coarsest_sharp(levels, pixels_per_mm, Level::Coarse, ceiling), quiet_s: 0.0 }
    }

    /// Moves `choice` on by one frame `dt_s` seconds long. Returns true when the level
    /// changed.
    pub fn update(
        &self,
        choice: &mut Choice,
        levels: &impl Deviations,
        pixels_per_mm: f64,
        dt_s: f32,
        ceiling: Level,
    ) -> bool {
        let before = choice.level;
        let error = |l: Level| levels.deviation(l).map(|d| d * pixels_per_mm);
        let Some(now) = error(choice.level) else {
            // Nothing bounds the error, so the levels are the same mesh.
            *choice = Choice { level: Level::Standard.min(ceiling), quiet_s: 0.0 };
            return choice.level != before;
        };
        if choice.level > ceiling {
            *choice = Choice { level: ceiling, quiet_s: 0.0 };
        } else if now > self.promote_px && choice.level < ceiling {
            *choice = Choice { level: self.coarsest_sharp(levels, pixels_per_mm, choice.level, ceiling), quiet_s: 0.0 };
        } else if now < self.demote_px
            && let Some(coarser) = choice.level.coarser()
        {
            choice.quiet_s += dt_s.max(0.0);
            if choice.quiet_s >= self.demote_after_s && error(coarser).is_some_and(|e| e <= self.promote_px) {
                *choice = Choice { level: coarser, quiet_s: 0.0 };
            }
        } else {
            choice.quiet_s = 0.0;
        }
        choice.level != before
    }

    /// The coarsest level from `from` to `ceiling` whose error is within the promotion
    /// threshold, or `ceiling` when none is. Standard when no chord bounds the error.
    fn coarsest_sharp(&self, levels: &impl Deviations, pixels_per_mm: f64, from: Level, ceiling: Level) -> Level {
        if levels.deviation(Level::Standard).is_none() {
            return Level::Standard.min(ceiling);
        }
        Level::ALL
            .into_iter()
            .filter(|l| *l >= from && *l <= ceiling)
            .find(|l| levels.deviation(*l).is_some_and(|d| d * pixels_per_mm <= self.promote_px))
            .unwrap_or(ceiling)
    }

    /// The levels the faces of a newly shown part start at, one per bounding box in
    /// `faces`, as a body's topology gives them: a face in view at the coarsest level
    /// whose error is within the promotion threshold, a face out of view at the coarsest
    /// of all.
    pub fn initial_faces(&self, levels: &impl Deviations, faces: &[Aabb], view: &View, ceiling: Level) -> Vec<Choice> {
        faces.iter().map(|face| self.initial(levels, view.pixels_per_mm(face), ceiling)).collect()
    }

    /// Moves the choice of each face on by one frame `dt_s` seconds long, each at its own
    /// scale in `view`. A face out of view is not promoted. Returns how many faces
    /// changed level. `choices` and `faces` are in the same order.
    pub fn update_faces(
        &self,
        choices: &mut [Choice],
        levels: &impl Deviations,
        faces: &[Aabb],
        view: &View,
        dt_s: f32,
        ceiling: Level,
    ) -> usize {
        debug_assert_eq!(choices.len(), faces.len(), "one choice per face");
        let mut changed = 0;
        for (choice, face) in choices.iter_mut().zip(faces) {
            if self.update(choice, levels, view.pixels_per_mm(face), dt_s, ceiling) {
                changed += 1;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{FaceLevels, Levels};
    use crate::quality::{Advanced, Quality};

    /// A body with a 100 mm diagonal: deviations 0.8, 0.2 and 0.05 mm.
    fn levels() -> Levels {
        Levels::new(&Quality::Auto, 100.0)
    }

    #[test]
    fn perspective_scale_falls_with_distance() {
        let p = Projection::Perspective { eye: Vec3::new(0.0, 0.0, 0.0), fov_y: 90f64.to_radians(), height_px: 1000.0 };
        let near = p.pixels_per_mm(Vec3::new(0.0, 0.0, 100.0), 0.0);
        let far = p.pixels_per_mm(Vec3::new(0.0, 0.0, 1000.0), 0.0);
        assert!((near - 5.0).abs() < 1e-9, "{near}");
        assert!((near / far - 10.0).abs() < 1e-9);
        // The near side of the sphere decides, and the eye inside a sphere is a close-up.
        assert!((p.pixels_per_mm(Vec3::new(0.0, 0.0, 150.0), 50.0) - near).abs() < 1e-9);
        assert!((p.pixels_per_mm(Vec3::new(0.0, 0.0, 1.0), 50.0) - 500.0).abs() < 1e-9);
        assert_eq!(Projection::Orthographic { pixels_per_mm: 3.0 }.pixels_per_mm(Vec3::new(9.0, 9.0, 9.0), 1.0), 3.0);
    }

    #[test]
    fn a_new_occurrence_starts_at_the_coarsest_sharp_level() {
        let t = Thresholds::default();
        let l = levels();
        assert_eq!(t.initial(&l, 1.0, Level::Fine).level, Level::Coarse);
        assert_eq!(t.initial(&l, 2.0, Level::Fine).level, Level::Standard);
        assert_eq!(t.initial(&l, 10.0, Level::Fine).level, Level::Fine);
        assert_eq!(t.initial(&l, 1000.0, Level::Fine).level, Level::Fine, "the finest when none is sharp enough");
        assert_eq!(t.initial(&l, 10.0, Level::Standard).level, Level::Standard, "the ceiling holds");
    }

    #[test]
    fn promotion_is_immediate_and_demotion_waits() {
        let t = Thresholds::default();
        let l = levels();
        let mut c = t.initial(&l, 1.0, Level::Fine);
        assert_eq!(c.level, Level::Coarse);
        // Zooming in: coarse's 0.8 mm covers 8 px, standard's 2 px, fine's 0.5 px.
        assert!(t.update(&mut c, &l, 10.0, 0.016, Level::Fine));
        assert_eq!(c.level, Level::Fine);
        // Zooming out a little stays put: fine is 0.05 px, under the demotion threshold,
        // but only for a moment.
        assert!(!t.update(&mut c, &l, 1.0, 1.0, Level::Fine));
        assert_eq!(c.level, Level::Fine);
        // Back in before the wait is over resets it.
        assert!(!t.update(&mut c, &l, 5.0, 1.5, Level::Fine));
        assert_eq!(c.quiet_s, 0.0);
        // Out for two seconds: one level down.
        assert!(!t.update(&mut c, &l, 1.0, 1.0, Level::Fine));
        assert!(t.update(&mut c, &l, 1.0, 1.0, Level::Fine));
        assert_eq!(c.level, Level::Standard);
        // Standard's 0.2 px is above the demotion threshold, so it stays.
        for _ in 0..100 {
            assert!(!t.update(&mut c, &l, 1.0, 1.0, Level::Fine));
        }
        // Far away it steps down once more.
        assert!(!t.update(&mut c, &l, 0.1, 1.0, Level::Fine));
        assert!(t.update(&mut c, &l, 0.1, 1.0, Level::Fine));
        assert_eq!(c.level, Level::Coarse);
    }

    #[test]
    fn the_ceiling_demotes_at_once() {
        let t = Thresholds::default();
        let l = levels();
        let mut c = t.initial(&l, 10.0, Level::Fine);
        assert_eq!(c.level, Level::Fine);
        assert!(t.update(&mut c, &l, 10.0, 0.0, Level::Coarse));
        assert_eq!(c.level, Level::Coarse);
        assert!(!t.update(&mut c, &l, 10.0, 0.0, Level::Coarse), "no promotion past the ceiling");
        assert!(t.update(&mut c, &l, 10.0, 0.0, Level::Fine));
        assert_eq!(c.level, Level::Fine);
    }

    #[test]
    fn levels_without_a_chord_stay_standard() {
        let t = Thresholds::default();
        let l = Levels::new(&Quality::Custom(Advanced { chord_height: 0.0, ..Advanced::default() }), 100.0);
        let mut c = Choice { level: Level::Fine, quiet_s: 0.0 };
        assert!(t.update(&mut c, &l, 1000.0, 0.0, Level::Fine));
        assert_eq!(c.level, Level::Standard);
        assert_eq!(t.initial(&l, 1000.0, Level::Fine).level, Level::Standard);
    }

    /// A cube `size` across, centred on the x axis at `x`.
    fn cube(x: f64, size: f64) -> Aabb {
        let half = Vec3::new(size / 2.0, size / 2.0, size / 2.0);
        Aabb { min: Vec3::new(x, 0.0, 0.0) - half, max: Vec3::new(x, 0.0, 0.0) + half }
    }

    /// A camera `distance` above the point `x` on the x axis, looking down at it through
    /// a 1500 by 1000 viewport with a 45 degree field of view.
    fn above(x: f64, distance: f64) -> View {
        let eye = Vec3::new(x, 0.0, distance);
        View::perspective(eye, Vec3::new(x, 0.0, 0.0), Vec3::Y, 45f64.to_radians(), 1500.0, 1000.0)
    }

    #[test]
    fn a_view_shows_what_is_inside_its_sides() {
        let v = above(0.0, 100.0);
        assert_eq!(v.planes.len(), 5);
        // At the origin's depth the view is 82.8 mm high and 124.3 mm wide.
        let half_height = 100.0 * 22.5f64.to_radians().tan();
        let half_width = half_height * 1.5;
        for (point, inside) in [
            (Vec3::new(0.0, 0.0, 0.0), true),
            (Vec3::new(half_width - 0.1, 0.0, 0.0), true),
            (Vec3::new(half_width + 0.1, 0.0, 0.0), false),
            (Vec3::new(0.0, half_height - 0.1, 0.0), true),
            (Vec3::new(0.0, -half_height - 0.1, 0.0), false),
            (Vec3::new(0.0, 0.0, 100.1), false),
            (Vec3::new(0.0, 0.0, -1e6), true),
        ] {
            assert_eq!(v.shows(point, 0.0), inside, "{point:?}");
        }
        // A sphere or a box reaching into the view counts.
        assert!(v.shows(Vec3::new(half_width + 5.0, 0.0, 0.0), 10.0));
        assert!(!v.shows(Vec3::new(half_width + 50.0, 0.0, 0.0), 10.0));
        let beside = cube(half_width + 5.0, 12.0);
        assert!(v.shows_box(&beside));
        // A face's scale is that of the near side of the sphere around its box.
        let around = v.projection.pixels_per_mm(beside.center(), beside.diagonal() / 2.0);
        assert!(around > 0.0 && (v.pixels_per_mm(&beside) - around).abs() < 1e-12);
        // A box is judged by its own corners: this one's sphere reaches the view, the box does not.
        let slab = Aabb { min: Vec3::new(half_width + 1.0, -30.0, -0.5), max: Vec3::new(half_width + 2.0, 30.0, 0.5) };
        assert!(v.shows(slab.center(), slab.diagonal() / 2.0) && !v.shows_box(&slab));
        assert_eq!(v.pixels_per_mm(&slab), 0.0);
        assert_eq!(v.pixels_per_mm(&cube(half_width + 50.0, 12.0)), 0.0);

        let flat = View::orthographic(Vec3::new(10.0, 0.0, 0.0), -Vec3::Z, Vec3::Y, 10.0, 1000.0, 500.0);
        assert_eq!(flat.planes.len(), 4);
        assert!(flat.shows(Vec3::new(59.9, 24.9, 7.0), 0.0));
        assert!(!flat.shows(Vec3::new(60.1, 0.0, 7.0), 0.0));
        assert!(!flat.shows(Vec3::new(10.0, -25.1, 7.0), 0.0));
        assert_eq!(flat.pixels_per_mm(&cube(10.0, 1.0)), 10.0);
        assert!(flat.shows_box(&cube(60.5, 2.0)) && !flat.shows_box(&cube(61.5, 2.0)));

        // A camera with no direction hides nothing.
        let lost = View::perspective(Vec3::ZERO, Vec3::ZERO, Vec3::Y, 1.0, 100.0, 100.0);
        assert!(lost.planes.is_empty() && lost.shows(Vec3::new(1e9, 0.0, 0.0), 0.0));
    }

    #[test]
    fn zooming_in_raises_the_faces_in_view_and_not_the_rest() {
        let t = Thresholds::default();
        // A part 100 mm across: a face is sampled to 0.5, 0.1 or 0.05 mm.
        let levels = FaceLevels::new(&Quality::Auto, 100.0);
        // Ten faces 3 mm across in a row along x, 20 mm apart.
        let faces: Vec<Aabb> = (0..10).map(|i| cube(-90.0 + 20.0 * f64::from(i), 3.0)).collect();
        // From far away every face is in view and coarse is sharp enough.
        let far = above(0.0, 2000.0);
        let mut choices = t.initial_faces(&levels, &faces, &far, Level::Fine);
        assert!(faces.iter().all(|f| far.shows_box(f)));
        assert!(choices.iter().all(|c| c.level == Level::Coarse));

        // Zoom in on the right end: three faces stay in view and go fine.
        let near = above(70.0, 50.0);
        let in_view: Vec<bool> = faces.iter().map(|f| near.shows_box(f)).collect();
        assert_eq!(in_view.iter().filter(|shown| **shown).count(), 3);
        let raised = t.update_faces(&mut choices, &levels, &faces, &near, 0.016, Level::Fine);
        assert_eq!(raised, 3);
        for (choice, shown) in choices.iter().zip(&in_view) {
            assert_eq!(choice.level, if *shown { Level::Fine } else { Level::Coarse });
        }

        // Panning to the left end raises the faces there at once; the ones left behind
        // wait out the demotion delay, then step down a level at a time.
        let left = above(-70.0, 50.0);
        assert_eq!(t.update_faces(&mut choices, &levels, &faces, &left, 0.016, Level::Fine), 3);
        assert_eq!(choices.iter().filter(|c| c.level == Level::Fine).count(), 6);
        assert_eq!(t.update_faces(&mut choices, &levels, &faces, &left, 1.0, Level::Fine), 0);
        assert_eq!(t.update_faces(&mut choices, &levels, &faces, &left, 1.0, Level::Fine), 3);
        assert_eq!(choices[7..].iter().map(|c| c.level).collect::<Vec<_>>(), [Level::Standard; 3]);
        assert_eq!(choices[..3].iter().map(|c| c.level).collect::<Vec<_>>(), [Level::Fine; 3]);
        assert_eq!(choices[3..7].iter().map(|c| c.level).collect::<Vec<_>>(), [Level::Coarse; 4]);

        // The ceiling caps every face.
        assert_eq!(t.update_faces(&mut choices, &levels, &faces, &left, 0.0, Level::Coarse), 6);
        assert!(choices.iter().all(|c| c.level == Level::Coarse));
    }

    #[test]
    fn a_choice_is_eight_bytes() {
        assert_eq!(std::mem::size_of::<Choice>(), 8);
    }
}
