//! Analytic nesting must not depend on display chords or curve direction.

use soliddraft_sketch::{EntityKind, P2, Sketch, profile};
use std::f64::consts::{PI, TAU};

fn circle(s: &mut Sketch, center: P2, radius: f64) -> u32 {
    s.add(EntityKind::Circle { center, radius }, false)
}

fn cap(s: &mut Sketch, end: P2, reverse: bool) {
    let start = P2::new(100.0, 0.0);
    let (a, b) = if reverse { (start, end) } else { (end, start) };
    s.add(EntityKind::Line { a, b }, false);
    s.add(EntityKind::Arc { center: P2::ZERO, start, end }, false);
}

#[test]
fn circular_holes_keep_their_identity_under_rotation_and_insertion_order() {
    for center in [P2::new(99.6, 6.5), P2::new(99.81187304123692, 0.0)] {
        for hole_first in [false, true] {
            let mut s = Sketch::new();
            let (outer, hole) = if hole_first {
                let hole = circle(&mut s, center, 0.1);
                (circle(&mut s, P2::ZERO, 100.0), hole)
            } else {
                let outer = circle(&mut s, P2::ZERO, 100.0);
                (outer, circle(&mut s, center, 0.1))
            };
            let before = s.clone();
            let regions = s.regions().unwrap();
            assert_eq!(regions.len(), 1, "{center:?}, hole first {hole_first}");
            assert_eq!(regions[0].outer.entities, [outer]);
            assert_eq!(regions[0].holes[0].entities, [hole]);
            assert!((regions[0].area() - PI * (10000.0 - 0.01)).abs() < 1e-8);
            assert_eq!(s, before);
        }
    }
}

#[test]
fn near_boundary_inside_and_disjoint_controls_are_rotation_independent() {
    for i in 0..96 {
        let direction = P2::from_angle(TAU * i as f64 / 96.0);
        for inside in [true, false] {
            let mut s = Sketch::new();
            circle(&mut s, P2::ZERO, 100.0);
            let distance = if inside { 99.8999 } else { 100.1001 };
            circle(&mut s, direction.scale(distance), 0.1);
            let regions = s.regions().unwrap();
            assert_eq!(regions.len(), if inside { 1 } else { 2 }, "angle {i}, inside {inside}");
            assert_eq!(regions.iter().map(|r| r.holes.len()).sum::<usize>(), usize::from(inside));
        }
    }
}

#[test]
fn semicircle_and_major_arc_boundaries_use_the_curve_in_both_walk_directions() {
    for end in [P2::new(-100.0, 0.0), P2::new(0.0, -100.0)] {
        for reverse in [false, true] {
            for inside in [true, false] {
                let mut s = Sketch::new();
                cap(&mut s, end, reverse);
                circle(&mut s, P2::new(99.6, if inside { 6.5 } else { -6.5 }), 0.1);
                let regions = s.regions().unwrap();
                assert_eq!(regions.len(), if inside { 1 } else { 2 }, "end {end:?}, reverse {reverse}");
                assert_eq!(regions.iter().map(|r| r.holes.len()).sum::<usize>(), usize::from(inside));
                assert!(regions.iter().all(|r| r.outer.signed_area() > 0.0));
            }
        }
    }
}

#[test]
fn rays_through_arc_joins_and_vertical_extrema_do_not_double_count() {
    let corners = [P2::new(100.0, 0.0), P2::new(0.0, 100.0), P2::new(-100.0, 0.0), P2::new(0.0, -100.0)];
    for center in [P2::new(99.6, 6.5), P2::new(0.0, 99.8), P2::new(-99.8, 0.0), P2::new(0.0, -99.8)] {
        let mut s = Sketch::new();
        for i in 0..4 {
            s.add(EntityKind::Arc { center: P2::ZERO, start: corners[i], end: corners[(i + 1) % 4] }, false);
        }
        circle(&mut s, center, 0.1);
        let regions = s.regions().unwrap();
        assert_eq!(regions.len(), 1, "{center:?}");
        assert_eq!(regions[0].holes.len(), 1);
        profile::profiles(&s, soliddraft_kernel_api::Plane::XY).unwrap();
    }
}

#[test]
fn nested_islands_keep_their_depth_area_and_source_ids() {
    let mut s = Sketch::new();
    let outer = circle(&mut s, P2::ZERO, 100.0);
    let hole = circle(&mut s, P2::new(99.2, 6.5), 0.4);
    let island = circle(&mut s, P2::new(99.2, 6.5), 0.1);
    let regions = s.regions().unwrap();
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].outer.entities, [outer]);
    assert_eq!(regions[0].holes[0].entities, [hole]);
    assert_eq!(regions[1].outer.entities, [island]);
    assert!(regions[1].holes.is_empty());
    assert!((regions.iter().map(|r| r.area()).sum::<f64>() - PI * (10000.0 - 0.16 + 0.01)).abs() < 1e-8);
}

#[test]
fn concave_line_regions_do_not_fill_their_notch() {
    let corners = [
        P2::new(-100.0, -100.0),
        P2::new(100.0, -100.0),
        P2::new(100.0, -50.0),
        P2::new(-50.0, -50.0),
        P2::new(-50.0, 50.0),
        P2::new(100.0, 50.0),
        P2::new(100.0, 100.0),
        P2::new(-100.0, 100.0),
    ];
    let mut s = Sketch::new();
    for i in 0..corners.len() {
        s.add(EntityKind::Line { a: corners[i], b: corners[(i + 1) % corners.len()] }, false);
    }
    let hole = circle(&mut s, P2::new(-75.0, 0.0), 1.0);
    let outside = circle(&mut s, P2::ZERO, 1.0);
    let regions = s.regions().unwrap();
    assert_eq!(regions.len(), 2);
    assert_eq!(regions[0].holes[0].entities, [hole]);
    assert_eq!(regions[1].outer.entities, [outside]);
}
