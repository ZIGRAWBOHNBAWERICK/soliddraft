//! The marks of cosmetic threads: lines on the walls of threaded holes.
//!
//! A threaded hole keeps the plain cylinder of its drill, so nothing in its faces shows
//! the thread. The viewport draws one: a helix at the thread's pitch along the part of
//! the wall the thread covers, a circle where the thread ends, and a circle of the
//! thread's major diameter where it starts, which lies on the face a plain hole opens
//! onto. The lines stay on what is left of the wall: where a later cut took the wall
//! away, all round or part of the way round, no line is drawn. They are tested against
//! the faces like edges and take no picks, so the wall under them stays pickable.

use crate::document::{DisplayEdge, DisplayMesh, Document};
use glam::{Vec2, Vec3};
use soliddraft_model::thread::ThreadedWall;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

/// Line segments per turn of the helix and round each circle.
const SEGMENTS: usize = 48;
/// At most this many turns are drawn of one wall; a finer thread is drawn at a coarser
/// pitch rather than as a solid band.
const MAX_TURNS: f64 = 400.0;
/// Slices of the turn that [`Cover`] sorts the wall's triangles into.
const SLICES: usize = 72;
/// How far inside the thread's ends a circle is tested against the wall, in millimetres.
const INSIDE: f32 = 0.05;

fn v(p: soliddraft_kernel_api::Vec3) -> Vec3 {
    Vec3::new(p.x as f32, p.y as f32, p.z as f32)
}

/// Two unit vectors square to `axis` and to each other.
fn frame(axis: Vec3) -> (Vec3, Vec3) {
    let helper = if axis.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = axis.cross(helper).normalize();
    (u, axis.cross(u))
}

/// A wall's axis frame: where a point is round the axis and along it.
#[derive(Clone, Copy)]
struct Axis {
    entry: Vec3,
    axis: Vec3,
    u: Vec3,
    w: Vec3,
}

impl Axis {
    fn of(wall: &ThreadedWall) -> Option<Axis> {
        let axis = v(wall.direction).normalize_or_zero();
        if axis == Vec3::ZERO {
            return None;
        }
        let (u, w) = frame(axis);
        Some(Axis { entry: v(wall.entry), axis, u, w })
    }

    fn at(&self, along: f32, angle: f32, r: f32) -> Vec3 {
        self.entry + self.axis * along + (self.u * angle.cos() + self.w * angle.sin()) * r
    }

    /// The angle round the axis, in (-π, π], and the length along it.
    fn place(&self, p: Vec3) -> Vec2 {
        let d = p - self.entry;
        Vec2::new(d.dot(self.w).atan2(d.dot(self.u)), d.dot(self.axis))
    }
}

/// An angle moved by whole turns to lie within half a turn of `near`.
fn unwrap(angle: f32, near: f32) -> f32 {
    angle - TAU * ((angle - near) / TAU).round()
}

/// What is left of a wall, as the display draws it: the triangles of the wall's faces,
/// laid out by angle round the hole's axis and length along it.
pub struct Cover {
    slices: Vec<Vec<[Vec2; 3]>>,
}

impl Cover {
    /// The triangles of `mesh` on the faces whose pick ids are `picks`.
    fn new(axis: &Axis, mesh: &DisplayMesh, picks: &[u32]) -> Cover {
        let mut slices = vec![Vec::new(); SLICES];
        for triangle in mesh.indices.as_chunks::<3>().0 {
            let corner = |i: usize| triangle[i] as usize;
            if !mesh.face_picks.get(corner(0)).is_some_and(|pick| picks.contains(pick)) {
                continue;
            }
            let Some(points) = (0..3).map(|i| mesh.positions.get(corner(i)).copied()).collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let mut laid = points.iter().map(|p| axis.place(*p)).collect::<Vec<_>>();
            let first = laid[0].x;
            for p in laid.iter_mut().skip(1) {
                p.x = unwrap(p.x, first);
            }
            let laid = [laid[0], laid[1], laid[2]];
            let low = laid.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
            let high = laid.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
            let first = ((low + PI) / TAU * SLICES as f32).floor() as i64;
            let last = ((high + PI) / TAU * SLICES as f32).floor() as i64;
            for slice in first..=last {
                slices[slice.rem_euclid(SLICES as i64) as usize].push(laid);
            }
        }
        Cover { slices }
    }

    /// True when the wall is there at `angle` round the axis and `along` it.
    fn holds(&self, angle: f32, along: f32) -> bool {
        let angle = unwrap(angle, 0.0);
        let slice = (((angle + PI) / TAU * SLICES as f32).floor() as i64).rem_euclid(SLICES as i64) as usize;
        self.slices[slice].iter().any(|[a, b, c]| {
            let p = Vec2::new(unwrap(angle, a.x), along);
            inside(p, *a, *b, *c)
        })
    }
}

/// True when `p` lies in the triangle, or within a hair of its sides.
fn inside(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let area = (b - a).perp_dot(c - a);
    if area.abs() < 1e-12 {
        return false;
    }
    let s = (b - p).perp_dot(c - p) / area;
    let t = (c - p).perp_dot(a - p) / area;
    let u = 1.0 - s - t;
    const HAIR: f32 = 1e-4;
    s >= -HAIR && t >= -HAIR && u >= -HAIR
}

/// The segments, as pairs of points, of one wall's mark, on the part of the wall that
/// `holds` says is there: `holds(angle, along)`.
pub fn wall_segments(wall: &ThreadedWall, holds: impl Fn(f32, f32) -> bool) -> Vec<Vec3> {
    let Some(axis) = Axis::of(wall) else { return Vec::new() };
    let (Some(first), Some(last)) = (wall.spans.first(), wall.spans.last()) else { return Vec::new() };
    let (start, end) = (first.0 as f32, last.1 as f32);
    let radius = wall.radius as f32;
    // The helix keeps one pitch over the whole thread, gaps and all.
    let turns = (f64::from(end - start) / wall.thread.pitch).min(MAX_TURNS) as f32;
    let pitch = (end - start) / turns;
    let mut points = Vec::new();
    let mut polyline = |samples: Vec<(f32, f32)>, r: f32, test: &dyn Fn(f32, f32) -> (f32, f32)| {
        for pair in samples.windows(2) {
            let (angle, along) = ((pair[0].0 + pair[1].0) / 2.0, (pair[0].1 + pair[1].1) / 2.0);
            let (angle, along) = test(angle, along);
            if holds(angle, along) {
                points.push(axis.at(pair[0].1, pair[0].0, r));
                points.push(axis.at(pair[1].1, pair[1].0, r));
            }
        }
    };
    // The helix turns right-handed about the drilling direction, as a right-hand
    // thread does.
    for (low, high) in &wall.spans {
        let (low, high) = (*low as f32, *high as f32);
        let steps = (((high - low) / pitch * SEGMENTS as f32).ceil() as usize).max(1);
        let samples = (0..=steps)
            .map(|i| {
                let along = low + (high - low) * i as f32 / steps as f32;
                (TAU * (along - start) / pitch, along)
            })
            .collect();
        polyline(samples, radius, &|angle, along| (angle, along));
    }
    let circle = |along: f32| (0..=SEGMENTS).map(|i| (TAU * i as f32 / SEGMENTS as f32, along)).collect::<Vec<_>>();
    // Each circle is drawn where the wall is there just inside the thread's end.
    let inside_end = (end - INSIDE).max(start);
    polyline(circle(end), radius, &|angle, _| (angle, inside_end));
    let inside_start = (start + INSIDE).min(end);
    polyline(circle(start), wall.thread.size as f32 / 2.0, &|angle, _| (angle, inside_start));
    points
}

/// The marks of every threaded wall of the part as one line-only mesh, or `None` when
/// the part has no thread to show. A wall is marked where its body's display mesh has it.
pub fn marks(doc: &Document) -> Option<Arc<DisplayMesh>> {
    let edges: Vec<DisplayEdge> = doc
        .model()
        .threaded_walls()
        .iter()
        .filter_map(|wall| {
            let axis = Axis::of(wall)?;
            let mesh = doc.meshes.get(&wall.body)?;
            let picks: Vec<u32> = wall.faces.iter().filter_map(|face| doc.pick_of_face(*face)).collect();
            let cover = Cover::new(&axis, mesh, &picks);
            let segments = wall_segments(wall, |angle, along| cover.holds(angle, along));
            (!segments.is_empty()).then_some(DisplayEdge { pick: 0, segments })
        })
        .collect();
    (!edges.is_empty()).then(|| Arc::new(DisplayMesh { edges, ..Default::default() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::Vec3 as K;
    use soliddraft_model::thread::{HoleThread, metric};

    fn wall(spans: Vec<(f64, f64)>, length: Option<f64>) -> ThreadedWall {
        ThreadedWall {
            feature: soliddraft_model::FeatureId(1),
            placement: 0,
            thread: HoleThread { length, ..HoleThread::coarse(metric(6.0).unwrap()) },
            entry: K::new(10.0, 0.0, 20.0),
            direction: K::new(0.0, 0.0, -1.0),
            radius: 2.5,
            body: soliddraft_kernel_api::BodyId::from_raw(1),
            faces: Vec::new(),
            top: 0.0,
            spans,
            through: false,
        }
    }

    #[test]
    fn the_mark_runs_along_the_span_on_the_wall() {
        let wall = wall(vec![(0.0, 12.0)], Some(12.0));
        let points = wall_segments(&wall, |_, _| true);
        assert!(!points.is_empty() && points.len().is_multiple_of(2));
        let (helix, circles) = points.split_at(points.len() - 4 * SEGMENTS);
        for p in helix {
            let r = (p.truncate() - glam::Vec2::new(10.0, 0.0)).length();
            assert!((r - 2.5).abs() < 1e-4, "{p}");
            assert!(p.z <= 20.0 + 1e-4 && p.z >= 8.0 - 1e-4, "{p}");
        }
        // Twelve turns of a 1 mm pitch, ending where the thread ends.
        assert_eq!(helix.len(), 2 * 12 * SEGMENTS);
        assert!((helix.last().unwrap().z - 8.0).abs() < 1e-4);
        // The major diameter's circle lies in the face the hole starts on.
        let major = &circles[2 * SEGMENTS..];
        assert!(major.iter().all(|p| (p.z - 20.0).abs() < 1e-4));
        assert!(major.iter().all(|p| ((p.truncate() - glam::Vec2::new(10.0, 0.0)).length() - 3.0).abs() < 1e-4));
    }

    /// A display mesh of a wall of radius 2.5 round the axis of `wall`, from 0 to 10 mm
    /// along it, with the triangles that `gone` holds taken out.
    fn wall_mesh(wall: &ThreadedWall, gone: impl Fn(f32, f32) -> bool) -> DisplayMesh {
        let axis = Axis::of(wall).unwrap();
        let mut mesh = DisplayMesh::default();
        let (around, down) = (96, 40);
        for i in 0..around {
            for j in 0..down {
                let (a0, a1) = (TAU * i as f32 / around as f32, TAU * (i + 1) as f32 / around as f32);
                let (t0, t1) = (10.0 * j as f32 / down as f32, 10.0 * (j + 1) as f32 / down as f32);
                if gone(unwrap((a0 + a1) / 2.0, 0.0), (t0 + t1) / 2.0) {
                    continue;
                }
                for (a, t) in [(a0, t0), (a1, t0), (a1, t1), (a0, t0), (a1, t1), (a0, t1)] {
                    mesh.indices.push(mesh.positions.len() as u32);
                    mesh.positions.push(axis.at(t, a, 2.5));
                    mesh.face_picks.push(7);
                }
            }
        }
        mesh
    }

    #[test]
    fn the_mark_leaves_out_what_a_later_cut_took_from_the_wall() {
        // A pocket from the side took the wall away all round from 4 to 6 mm, and a slot
        // took a quarter of the way round from 7 to 9 mm.
        let pocket = |_: f32, along: f32| (4.0..6.0).contains(&along);
        let slot = |angle: f32, along: f32| (7.0..9.0).contains(&along) && (0.0..PI / 2.0).contains(&angle);
        let wall = wall(vec![(0.0, 4.0), (6.0, 10.0)], None);
        let axis = Axis::of(&wall).unwrap();
        let mesh = wall_mesh(&wall, |angle, along| pocket(angle, along) || slot(angle, along));
        let cover = Cover::new(&axis, &mesh, &[7]);
        let points = wall_segments(&wall, |angle, along| cover.holds(angle, along));
        assert!(!points.is_empty());
        let mut above = 0;
        let mut below = 0;
        let mut beside_slot = 0;
        for pair in points.as_chunks::<2>().0 {
            let place = axis.place((pair[0] + pair[1]) / 2.0);
            let (angle, along) = (place.x, place.y);
            assert!(!pocket(angle, along), "a line crosses the pocket at {angle} rad, {along} mm");
            // Away from the slot's sides by more than one segment's reach.
            assert!(
                !((7.1..8.9).contains(&along) && (0.1..PI / 2.0 - 0.1).contains(&angle)),
                "a line crosses the slot at {angle} rad, {along} mm"
            );
            above += usize::from(along < 4.0);
            below += usize::from(along > 6.0);
            beside_slot += usize::from((7.1..8.9).contains(&along) && !(-0.1..PI / 2.0 + 0.1).contains(&angle));
        }
        assert!(above > 0 && below > 0 && beside_slot > 0, "{above} {below} {beside_slot}");
        // The whole wall is marked whole.
        let whole = wall_mesh(&wall, |_, _| false);
        let cover = Cover::new(&axis, &whole, &[7]);
        let all = wall_segments(&wall, |angle, along| cover.holds(angle, along));
        assert_eq!(all.len(), wall_segments(&wall, |_, _| true).len());
        // Another face's triangles are not this wall's.
        assert!(wall_segments(&wall, |a, t| Cover::new(&axis, &whole, &[8]).holds(a, t)).is_empty());
    }
}
