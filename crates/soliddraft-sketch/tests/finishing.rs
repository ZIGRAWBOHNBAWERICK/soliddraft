//! Corner treatments and splits retain analytic geometry and solve their joins.

use soliddraft_sketch::{
    Align, ConstraintKind, DistanceKind, EntityKind, P2, PointRef, Sketch, SolveStatus, solve,
    tools::{ChamferSize, Operation},
};

fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn line(s: &mut Sketch, a: P2, b: P2) -> u32 {
    s.add(EntityKind::Line { a, b }, false)
}
fn near(a: P2, b: P2) {
    assert!(a.dist(b) < 1e-5, "{a:?} != {b:?}");
}
fn solved(s: &mut Sketch) {
    solve::solve(s, None, true);
    assert!(matches!(s.status, SolveStatus::UnderDefined | SolveStatus::FullyDefined), "{:?}", s.status);
}
fn corner() -> (Sketch, u32, u32) {
    let mut s = Sketch::new();
    let a = line(&mut s, p(-10.0, 0.0), P2::ZERO);
    let b = line(&mut s, P2::ZERO, p(0.0, 10.0));
    s.constrain(ConstraintKind::Coincident { a: PointRef::new(a, 1), b: PointRef::new(b, 0) });
    (s, a, b)
}

#[test]
fn fillet_replaces_a_joined_corner_and_keeps_far_endpoint_anchors() {
    let (mut s, a, b) = corner();
    for (entity, index, at) in [(a, 0, p(-10.0, 0.0)), (b, 1, p(0.0, 10.0))] {
        s.constrain(ConstraintKind::Fix { point: PointRef::new(entity, index), at });
    }
    s.constrain(ConstraintKind::Horizontal { target: Align::Line(a) });
    s.constrain(ConstraintKind::Vertical { target: Align::Line(b) });
    let before = s.clone();
    let preview = s.fillet_preview(a, b, p(-8.0, 0.0), p(0.0, 8.0), 2.0).unwrap();
    assert_eq!(s, before);
    let EntityKind::Arc { center, start, end } = preview.bridge else { panic!() };
    near(center, p(-2.0, 2.0));
    near(start, p(-2.0, 0.0));
    near(end, p(0.0, 2.0));
    let ids =
        s.apply_operation(&Operation::Fillet { a, b, keep_a: p(-8.0, 0.0), keep_b: p(0.0, 8.0), radius: 2.0 }).unwrap();
    assert_eq!(s.entity(ids[2]).unwrap().kind, preview.bridge);
    solved(&mut s);
    assert_eq!(s.constraints.iter().filter(|c| matches!(c.kind, ConstraintKind::Tangent { .. })).count(), 2);
    assert!((s.entity(ids[2]).unwrap().radius().unwrap() - 2.0).abs() < 1e-5);
    near(s.point(PointRef::new(a, 0)).unwrap(), p(-10.0, 0.0));
    near(s.point(PointRef::new(b, 1)).unwrap(), p(0.0, 10.0));
    let radius = s.constraints.iter_mut().find(|c| matches!(c.kind, ConstraintKind::Radius { .. })).unwrap();
    radius.set_value(1.0);
    solved(&mut s);
    assert!((s.entity(ids[2]).unwrap().radius().unwrap() - 1.0).abs() < 1e-5);
}

#[test]
fn fillets_support_line_arc_and_arc_arc_corners_without_tangent_cusps() {
    for arcs in [false, true] {
        let mut s = Sketch::new();
        let a = if arcs {
            s.add(EntityKind::Arc { center: p(-5.0, 0.0), start: p(-5.0, -5.0), end: P2::ZERO }, false)
        } else {
            line(&mut s, p(-10.0, 0.0), P2::ZERO)
        };
        let b = if arcs {
            s.add(EntityKind::Arc { center: p(0.0, 5.0), start: P2::ZERO, end: p(5.0, 5.0) }, false)
        } else {
            s.add(EntityKind::Arc { center: p(5.0, 0.0), start: p(5.0, -5.0), end: P2::ZERO }, false)
        };
        let (keep_a, keep_b, radius) =
            if arcs { (p(-2.0, -4.0), p(3.0, 1.0), 0.5) } else { (p(-8.0, 0.0), p(1.0, 3.0), 1.0) };
        let preview = s.fillet_preview(a, b, keep_a, keep_b, radius).unwrap();
        let ids = s.apply_operation(&Operation::Fillet { a, b, keep_a, keep_b, radius }).unwrap();
        assert_eq!(s.entity(ids[2]).unwrap().kind, preview.bridge);
        solved(&mut s);
        assert!((s.entity(ids[2]).unwrap().radius().unwrap() - radius).abs() < 1e-5);
        for i in 0..2 {
            near(s.point(preview.joint[i]).unwrap(), s.point(PointRef::new(ids[2], preview.bridge_points[i])).unwrap());
        }
    }
}

#[test]
fn chamfers_accept_two_setbacks_or_a_setback_and_angle() {
    for size in [
        ChamferSize::Distances { first: 2.0, second: 3.0 },
        ChamferSize::DistanceAngle { distance: 2.0, angle_degrees: 45.0 },
    ] {
        let (mut s, a, b) = corner();
        let preview = s.chamfer_preview(a, b, p(-8.0, 0.0), p(0.0, 8.0), size).unwrap();
        let EntityKind::Line { a: start, b: end } = preview.bridge else { panic!() };
        near(start, p(-2.0, 0.0));
        near(end, p(0.0, if matches!(size, ChamferSize::Distances { .. }) { 3.0 } else { 2.0 }));
        s.apply_operation(&Operation::Chamfer { a, b, keep_a: p(-8.0, 0.0), keep_b: p(0.0, 8.0), size }).unwrap();
        solved(&mut s);
        assert_eq!(s.drawn().count(), 3);
    }
}

#[test]
fn splitting_a_line_remaps_far_endpoint_constraints_and_keeps_total_dimension() {
    let mut s = Sketch::new();
    let a = line(&mut s, P2::ZERO, p(10.0, 0.0));
    let b = line(&mut s, p(10.0, 0.0), p(10.0, 5.0));
    s.constrain(ConstraintKind::Coincident { a: PointRef::new(a, 1), b: PointRef::new(b, 0) });
    s.constrain(ConstraintKind::Distance {
        a: PointRef::new(a, 0),
        b: PointRef::new(a, 1),
        kind: DistanceKind::Aligned,
        value: 10.0,
        label: p(5.0, -2.0),
    });
    let before = s.clone();
    let preview = s.split_preview(a, p(4.0, 0.1), None).unwrap();
    assert_eq!(s, before);
    let ids = s.apply_operation(&Operation::Split { entity: a, at: p(4.0, 0.1), second: None }).unwrap();
    assert_eq!(ids[0], a);
    assert_eq!(s.entity(ids[1]).unwrap().kind, preview.pieces[1]);
    assert!(
        s.constraints
            .iter()
            .any(|c| matches!(c.kind, ConstraintKind::Distance { b, .. } if b == PointRef::new(ids[1], 1)))
    );
    solved(&mut s);
    near(s.point(PointRef::new(ids[1], 1)).unwrap(), s.point(PointRef::new(b, 0)).unwrap());
    let first = s.entity(a).unwrap();
    let second = s.entity(ids[1]).unwrap();
    assert!(
        (first.point(0).unwrap().dist(first.point(1).unwrap())
            + second.point(0).unwrap().dist(second.point(1).unwrap())
            - 10.0)
            .abs()
            < 1e-5
    );
}

#[test]
fn splitting_arcs_and_circles_retains_the_analytic_circle_and_construction_state() {
    for circle in [false, true] {
        let mut s = Sketch::new();
        let kind = if circle {
            EntityKind::Circle { center: P2::ZERO, radius: 5.0 }
        } else {
            EntityKind::Arc { center: P2::ZERO, start: p(5.0, 0.0), end: p(-5.0, 0.0) }
        };
        let id = s.add(kind, true);
        let operation = Operation::Split { entity: id, at: p(0.0, 5.1), second: circle.then_some(p(0.0, -5.0)) };
        let ids = s.apply_operation(&operation).unwrap();
        solved(&mut s);
        for id in ids {
            let e = s.entity(id).unwrap();
            assert!(e.construction);
            assert!(matches!(e.kind, EntityKind::Arc { .. }));
            near(e.point(0).unwrap(), P2::ZERO);
            assert!((e.radius().unwrap() - 5.0).abs() < 1e-5);
        }
        let copy: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(copy.constraints, s.constraints);
    }
}

#[test]
fn finishing_refusals_are_atomic_and_operations_roundtrip() {
    let (mut s, a, b) = corner();
    for operation in [
        Operation::Fillet { a, b, keep_a: p(-8.0, 0.0), keep_b: p(0.0, 8.0), radius: 100.0 },
        Operation::Fillet { a, b: a, keep_a: p(-8.0, 0.0), keep_b: p(0.0, 8.0), radius: 1.0 },
        Operation::Chamfer {
            a,
            b,
            keep_a: p(-8.0, 0.0),
            keep_b: p(0.0, 8.0),
            size: ChamferSize::DistanceAngle { distance: 2.0, angle_degrees: 100.0 },
        },
        Operation::Split { entity: a, at: P2::ZERO, second: None },
        Operation::Split { entity: a, at: p(-5.0, 0.0), second: Some(p(-4.0, 0.0)) },
    ] {
        let before = s.clone();
        assert!(s.apply_operation(&operation).is_err(), "{operation:?}");
        assert_eq!(s, before);
        assert_eq!(serde_json::from_str::<Operation>(&serde_json::to_string(&operation).unwrap()).unwrap(), operation);
    }
}

fn fixed_midpoint_corner(reverse_a: bool, reverse_b: bool) -> (Sketch, u32, u32, u32) {
    let mut s = Sketch::new();
    let ends = |a, b, reverse| if reverse { (b, a) } else { (a, b) };
    let (pa, qa) = ends(p(-10.0, 0.0), P2::ZERO, reverse_a);
    let (pb, qb) = ends(P2::ZERO, p(0.0, 10.0), reverse_b);
    let a = line(&mut s, pa, qa);
    let b = line(&mut s, pb, qb);
    s.constrain(ConstraintKind::Coincident {
        a: PointRef::new(a, usize::from(!reverse_a)),
        b: PointRef::new(b, usize::from(reverse_b)),
    });
    for (entity, index, at) in [(a, usize::from(reverse_a), p(-10.0, 0.0)), (b, usize::from(!reverse_b), p(0.0, 10.0))]
    {
        s.constrain(ConstraintKind::Fix { point: PointRef::new(entity, index), at });
    }
    for (line, at) in [(a, p(-5.0, 0.0)), (b, p(0.0, 5.0))] {
        let id = s.add(EntityKind::Point { p: at }, true);
        s.constrain(ConstraintKind::Midpoint { point: PointRef::new(id, 0), line });
        s.constrain(ConstraintKind::Fix { point: PointRef::new(id, 0), at });
    }
    let unrelated = line(&mut s, p(8.0, -4.0), p(16.0, -4.0));
    let marker = s.add(EntityKind::Point { p: p(12.0, -4.0) }, true);
    s.constrain(ConstraintKind::Midpoint { point: PointRef::new(marker, 0), line: unrelated });
    s.constrain(ConstraintKind::Fix { point: PointRef::new(marker, 0), at: p(12.0, -4.0) });
    solved(&mut s);
    (s, a, b, unrelated)
}

fn check_fixed_midpoint_treatment(fillet: bool) {
    for reverse_a in [false, true] {
        for reverse_b in [false, true] {
            let (mut s, a, b, unrelated) = fixed_midpoint_corner(reverse_a, reverse_b);
            let old = s.clone();
            let (operation, preview) = if fillet {
                (
                    Operation::Fillet { a, b, keep_a: p(-8.0, 0.0), keep_b: p(0.0, 8.0), radius: 2.0 },
                    s.fillet_preview(a, b, p(-8.0, 0.0), p(0.0, 8.0), 2.0).unwrap(),
                )
            } else {
                let size = ChamferSize::Distances { first: 2.0, second: 2.0 };
                (
                    Operation::Chamfer { a, b, keep_a: p(-8.0, 0.0), keep_b: p(0.0, 8.0), size },
                    s.chamfer_preview(a, b, p(-8.0, 0.0), p(0.0, 8.0), size).unwrap(),
                )
            };
            assert_eq!(s, old, "preview must not change constraints");
            s.apply_operation(&operation).unwrap();
            solved(&mut s);
            near(s.point(preview.joint[0]).unwrap(), p(-2.0, 0.0));
            near(s.point(preview.joint[1]).unwrap(), p(0.0, 2.0));
            assert!(
                !s.constraints
                    .iter()
                    .any(|c| matches!(c.kind, ConstraintKind::Midpoint { line, .. } if line == a || line == b))
            );
            assert!(
                s.constraints
                    .iter()
                    .any(|c| matches!(c.kind, ConstraintKind::Midpoint { line, .. } if line == unrelated))
            );
            for c in old.constraints.iter().filter(|c| matches!(c.kind, ConstraintKind::Fix { .. })) {
                assert_eq!(s.constraint(c.id), Some(c), "unrelated and far-end anchors survive");
                let ConstraintKind::Fix { point, at } = c.kind else { unreachable!() };
                near(s.point(point).unwrap(), at);
            }
        }
    }
}

#[test]
fn fillet_releases_fixed_external_midpoints_in_all_endpoint_orders() {
    check_fixed_midpoint_treatment(true);
}

#[test]
fn chamfer_releases_fixed_external_midpoints_in_all_endpoint_orders() {
    check_fixed_midpoint_treatment(false);
}

#[test]
fn split_preserves_symmetry_in_both_point_slots_and_tracks_the_original_far_end() {
    for arc in [false, true] {
        for swapped in [false, true] {
            let mut s = Sketch::new();
            let (kind, at, index, far, changed) = if arc {
                (
                    EntityKind::Arc { center: p(4.0, 0.0), start: p(8.0, 0.0), end: p(4.0, 4.0) },
                    p(4.0 + 8.0_f64.sqrt(), 8.0_f64.sqrt()),
                    2,
                    p(4.0, 4.0),
                    p(6.0, 12.0_f64.sqrt()),
                )
            } else {
                (EntityKind::Line { a: p(2.0, 0.0), b: p(10.0, 0.0) }, p(6.0, 0.0), 1, p(10.0, 0.0), p(12.0, 0.0))
            };
            let source = s.add(kind, false);
            let axis_kind = EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) };
            let axis = s.add(axis_kind.clone(), true);
            s.constrain(ConstraintKind::FixEntity { entity: axis, at: axis_kind });
            let partner = s.add(EntityKind::Point { p: p(-far.u, far.v) }, true);
            let partner_ref = PointRef::new(partner, 0);
            let partner_fix = s.constrain(ConstraintKind::Fix { point: partner_ref, at: p(-far.u, far.v) });
            let far_ref = PointRef::new(source, index);
            let (a, b) = if swapped { (partner_ref, far_ref) } else { (far_ref, partner_ref) };
            let symmetry = s.constrain(ConstraintKind::Symmetric { a, b, axis });
            let first = PointRef::new(source, if arc { 1 } else { 0 });
            let first_at = s.point(first).unwrap();
            let control = s.add(EntityKind::Point { p: p(-first_at.u, first_at.v) }, true);
            let unaffected = s.constrain(ConstraintKind::Symmetric { a: first, b: PointRef::new(control, 0), axis });
            s.constrain(ConstraintKind::Fix { point: first, at: first_at });
            if arc {
                s.constrain(ConstraintKind::Fix { point: PointRef::new(source, 0), at: p(4.0, 0.0) });
                s.constrain(ConstraintKind::Radius { entity: source, value: 4.0, diameter: false, label: P2::ZERO });
            }
            let marker = s.add(EntityKind::Point { p: far }, true);
            let coincidence = s.constrain(ConstraintKind::Coincident { a: far_ref, b: PointRef::new(marker, 0) });
            solved(&mut s);
            let control = s.constraint(unaffected).unwrap().clone();
            let ids = s.apply_operation(&Operation::Split { entity: source, at, second: None }).unwrap();
            let replacement = PointRef::new(ids[1], index);
            let expected = if swapped {
                ConstraintKind::Symmetric { a: partner_ref, b: replacement, axis }
            } else {
                ConstraintKind::Symmetric { a: replacement, b: partner_ref, axis }
            };
            assert_eq!(
                s.constraint(symmetry).map(|c| c.kind.clone()),
                Some(expected.clone()),
                "arc={arc} swapped={swapped}"
            );
            assert_eq!(s.constraint(unaffected), Some(&control));
            assert_eq!(
                s.constraint(coincidence).unwrap().kind,
                ConstraintKind::Coincident { a: replacement, b: PointRef::new(marker, 0) }
            );
            solved(&mut s);
            s.constraint_mut(partner_fix).unwrap().kind =
                ConstraintKind::Fix { point: partner_ref, at: p(-changed.u, changed.v) };
            solved(&mut s);
            near(s.point(replacement).unwrap(), changed);
            near(s.point(PointRef::new(marker, 0)).unwrap(), changed);
            let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
            solved(&mut reopened);
            near(reopened.point(replacement).unwrap(), changed);
            assert_eq!(reopened.constraint(symmetry).map(|c| c.kind.clone()), Some(expected));
        }
    }
}
