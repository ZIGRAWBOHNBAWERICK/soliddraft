//! Circular placement geometry, refusal, tangency and persistence.

use soliddraft_sketch::{
    ConstraintKind, EntityKind, P2, PointRef, Sketch, solve,
    tools::{Operation, circular::*},
};

fn sweep(kind: EntityKind) -> f64 {
    let EntityKind::Arc { center, start, end } = kind else { panic!("expected arc") };
    ((end - center).angle() - (start - center).angle()).rem_euclid(std::f64::consts::TAU)
}

#[test]
fn three_picks_retain_circle_and_minor_major_arc_geometry() {
    for offset in [P2::ZERO, P2::new(1e6, -1e6)] {
        let a = offset + P2::new(5.0, 0.0);
        let b = offset + P2::new(0.0, -5.0);
        let c = offset + P2::new(-5.0, 0.0);
        let EntityKind::Circle { center, radius } = perimeter_circle(a, b, c).unwrap() else { panic!() };
        assert!(center.dist(offset) < 1e-8);
        assert!((radius - 5.0).abs() < 1e-8);
        assert!((sweep(three_point_arc(a, b, c).unwrap()) - 1.5 * std::f64::consts::PI).abs() < 1e-8);
        assert!((sweep(centrepoint_arc(offset, a, b, false).unwrap()) - 1.5 * std::f64::consts::PI).abs() < 1e-8);
        assert!((sweep(centrepoint_arc(offset, a, b, true).unwrap()) - 0.5 * std::f64::consts::PI).abs() < 1e-8);
        assert!((sweep(three_point_arc(a, c, b).unwrap()) - std::f64::consts::PI).abs() < 1e-8);
    }
}

#[test]
fn invalid_picks_refuse_without_consuming_ids_or_changing_the_sketch() {
    let mut sketch = Sketch::new();
    for p in [P2::ZERO, P2::new(1.0, 0.0), P2::new(f64::INFINITY, 1.0), P2::new(f64::NAN, 1.0)] {
        for operation in [
            Operation::PerimeterCircle { a: P2::ZERO, b: P2::new(1.0, 0.0), c: p, construction: false },
            Operation::ThreePointArc { start: P2::ZERO, end: P2::new(1.0, 0.0), through: p, construction: false },
        ] {
            let before = sketch.clone();
            assert!(sketch.apply_operation(&operation).is_err());
            assert_eq!(sketch, before);
        }
    }
    assert!(centrepoint_arc(P2::ZERO, P2::new(2.0, 0.0), P2::new(4.0, 0.0), false).is_err());
    assert!(centrepoint_arc(P2::ZERO, P2::ZERO, P2::new(0.0, 2.0), false).is_err());
}

#[test]
fn tangent_arcs_continue_both_ends_of_lines_and_arcs_on_either_side() {
    for source in [
        EntityKind::Line { a: P2::new(-5.0, 0.0), b: P2::new(5.0, 0.0) },
        EntityKind::Arc { center: P2::ZERO, start: P2::new(5.0, 0.0), end: P2::new(0.0, 5.0) },
    ] {
        for index in if matches!(source, EntityKind::Line { .. }) { [0, 1] } else { [1, 2] } {
            for side in [-1.0, 1.0] {
                let mut sketch = Sketch::new();
                let id = sketch.add(source.clone(), false);
                let from = PointRef::new(id, index);
                let start = sketch.point(from).unwrap();
                let tangent = match (&source, index) {
                    (EntityKind::Line { a, b }, 0) => (*a - *b).normalized(),
                    (EntityKind::Line { a, b }, _) => (*b - *a).normalized(),
                    (EntityKind::Arc { center, .. }, 1) => (*center - start).perp().normalized(),
                    (EntityKind::Arc { center, .. }, _) => (start - *center).perp().normalized(),
                    _ => unreachable!(),
                };
                let end = start + tangent.scale(4.0) + tangent.perp().scale(side * 4.0);
                let (kind, join) = tangent_arc(&sketch, from, end).unwrap();
                let ids = sketch.apply_operation(&Operation::TangentArc { from, end, construction: true }).unwrap();
                assert_eq!(sketch.entity(ids[0]).unwrap().kind, kind);
                assert_eq!(sketch.point(PointRef::new(ids[0], join)), Some(start));
                assert!(sketch.entity(ids[0]).unwrap().construction);
                assert!(sketch.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Tangent { .. })));
                solve::solve(&mut sketch, None, true);
                assert_eq!(sketch.entity(id).unwrap().kind, source);
                assert!(sketch.point(PointRef::new(ids[0], join)).unwrap().dist(start) < 1e-6);
                assert!(!matches!(
                    sketch.status,
                    soliddraft_sketch::SolveStatus::Failed(_) | soliddraft_sketch::SolveStatus::OverDefined { .. }
                ));
                let json = serde_json::to_string(&sketch).unwrap();
                let reopened = serde_json::from_str::<Sketch>(&json).unwrap();
                assert_eq!(reopened.entities, sketch.entities);
                assert_eq!(reopened.constraints, sketch.constraints);
                let before = sketch.clone();
                assert!(
                    sketch
                        .apply_operation(&Operation::TangentArc { from, end: start + tangent, construction: false })
                        .is_err()
                );
                assert_eq!(sketch, before);
            }
        }
    }
    let mut sketch = Sketch::new();
    let circle = sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 2.0 }, false);
    for from in [PointRef::ORIGIN, PointRef::new(circle, 0), PointRef::new(99, 1)] {
        assert!(tangent_arc(&sketch, from, P2::new(4.0, 4.0)).is_err());
    }
}
