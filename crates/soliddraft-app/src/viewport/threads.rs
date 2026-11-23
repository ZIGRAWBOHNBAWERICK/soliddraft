//! The marks of cosmetic threads: lines on the walls of threaded holes.
//!
//! A threaded hole keeps the plain cylinder of its drill, so nothing in its faces shows
//! the thread. The viewport draws one: a helix at the thread's pitch along the part of
//! the wall the thread covers, a circle where the thread ends, and a circle of the
//! thread's major diameter where it starts, which lies on the face a plain hole opens
//! onto. The lines are tested against the faces like edges and take no picks, so the
//! wall under them stays pickable.

use crate::document::{DisplayEdge, DisplayMesh, Document};
use glam::Vec3;
use soliddraft_model::thread::ThreadedWall;
use std::sync::Arc;

/// Line segments per turn of the helix and round each circle.
const SEGMENTS: usize = 48;
/// At most this many turns are drawn of one wall; a finer thread is drawn at a coarser
/// pitch rather than as a solid band.
const MAX_TURNS: f64 = 400.0;

fn v(p: soliddraft_kernel_api::Vec3) -> Vec3 {
    Vec3::new(p.x as f32, p.y as f32, p.z as f32)
}

/// Two unit vectors square to `axis` and to each other.
fn frame(axis: Vec3) -> (Vec3, Vec3) {
    let helper = if axis.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = axis.cross(helper).normalize();
    (u, axis.cross(u))
}

/// The segments, as pairs of points, of one wall's mark.
pub fn wall_segments(wall: &ThreadedWall) -> Vec<Vec3> {
    let axis = v(wall.direction).normalize_or_zero();
    if axis == Vec3::ZERO {
        return Vec::new();
    }
    let (u, w) = frame(axis);
    let entry = v(wall.entry);
    let (start, end) = (wall.span.0 as f32, wall.span.1 as f32);
    let radius = wall.radius as f32;
    let length = end - start;
    let turns = (f64::from(length) / wall.thread.pitch).min(MAX_TURNS) as f32;
    let at = |along: f32, angle: f32, r: f32| entry + axis * along + (u * angle.cos() + w * angle.sin()) * r;
    let mut points = Vec::new();
    let mut polyline = |samples: Vec<Vec3>| {
        for pair in samples.windows(2) {
            points.extend_from_slice(pair);
        }
    };
    // The helix turns right-handed about the drilling direction, as a right-hand
    // thread does.
    let steps = ((turns * SEGMENTS as f32).ceil() as usize).max(1);
    polyline(
        (0..=steps)
            .map(|i| {
                let t = i as f32 / steps as f32;
                at(start + length * t, std::f32::consts::TAU * turns * t, radius)
            })
            .collect(),
    );
    let circle = |along: f32, r: f32| {
        (0..=SEGMENTS).map(|i| at(along, std::f32::consts::TAU * i as f32 / SEGMENTS as f32, r)).collect::<Vec<_>>()
    };
    polyline(circle(end, radius));
    polyline(circle(start, wall.thread.size as f32 / 2.0));
    points
}

/// The marks of every threaded wall of the part as one line-only mesh, or `None` when
/// the part has no thread to show.
pub fn marks(doc: &Document) -> Option<Arc<DisplayMesh>> {
    let walls = doc.model().threaded_walls();
    if walls.is_empty() {
        return None;
    }
    let edges = walls.iter().map(|wall| DisplayEdge { pick: 0, segments: wall_segments(wall) }).collect();
    Some(Arc::new(DisplayMesh { edges, ..Default::default() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::Vec3 as K;
    use soliddraft_model::thread::{HoleThread, metric};

    #[test]
    fn the_mark_runs_along_the_span_on_the_wall() {
        let wall = ThreadedWall {
            feature: soliddraft_model::FeatureId(1),
            placement: 0,
            thread: HoleThread { length: Some(12.0), ..HoleThread::coarse(metric(6.0).unwrap()) },
            entry: K::new(10.0, 0.0, 20.0),
            direction: K::new(0.0, 0.0, -1.0),
            radius: 2.5,
            span: (0.0, 12.0),
        };
        let points = wall_segments(&wall);
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
}
