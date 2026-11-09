//! Selection transforms preserve identity and relations, or refuse without mutation.

use soliddraft_sketch::{Align, ConstraintKind, DistanceKind, EntityKind, P2, PointRef, Sketch, tools::Operation};

fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}

fn near(a: P2, b: P2) {
    assert!(a.dist(b) < 1e-6, "{a:?} != {b:?}");
}

fn refused(s: &mut Sketch, op: Operation) {
    let before = s.clone();
    assert!(s.apply_operation(&op).is_err(), "{op:?}");
    assert_eq!(*s, before, "refusal must retain geometry, constraints and id counters");
}

#[test]
fn move_retains_dimensions_and_shared_endpoints_but_rejects_partial_connections() {
    let mut s = Sketch::new();
    let a = s.add(EntityKind::Line { a: p(1.0, 2.0), b: p(5.0, 2.0) }, false);
    let b = s.add(EntityKind::Line { a: p(5.0, 2.0), b: p(5.0, 6.0) }, true);
    s.constrain(ConstraintKind::Horizontal { target: Align::Line(a) });
    s.constrain(ConstraintKind::Coincident { a: PointRef::new(a, 1), b: PointRef::new(b, 0) });
    s.constrain(ConstraintKind::Distance {
        a: PointRef::new(a, 0),
        b: PointRef::new(a, 1),
        kind: DistanceKind::Aligned,
        value: 4.0,
        label: p(3.0, 1.0),
    });
    let relations = s.constraints.clone();
    refused(&mut s, Operation::Move { entities: vec![a], displacement: p(3.0, 7.0) });
    assert_eq!(
        s.apply_operation(&Operation::Move { entities: vec![a, b], displacement: p(3.0, 7.0) }).unwrap(),
        vec![a, b]
    );
    near(s.entity(a).unwrap().point(0).unwrap(), p(4.0, 9.0));
    near(s.entity(b).unwrap().point(0).unwrap(), p(8.0, 9.0));
    assert!(s.entity(b).unwrap().construction);
    assert_eq!(s.constraints, relations);
    refused(&mut s, Operation::Scale { entities: vec![a, b], base: P2::ZERO, factor: 2.0 });
    refused(&mut s, Operation::Rotate { entities: vec![a, b], center: P2::ZERO, angle_degrees: 90.0, copy: false });
    s.apply_operation(&Operation::Rotate { entities: vec![a, b], center: P2::ZERO, angle_degrees: 180.0, copy: false })
        .unwrap();
    near(s.entity(a).unwrap().point(0).unwrap(), p(-4.0, -9.0));
    assert_eq!(s.constraints, relations, "compatible rotation retains driving dimensions too");
}

#[test]
fn copies_of_fixed_geometry_are_independent_and_join_only_copied_endpoints() {
    let mut s = Sketch::new();
    let a = s.add(EntityKind::Line { a: p(1.0, 2.0), b: p(5.0, 2.0) }, false);
    let b = s.add(EntityKind::Line { a: p(5.0, 2.0), b: p(5.0, 6.0) }, true);
    for id in [a, b] {
        s.constrain(ConstraintKind::FixEntity { entity: id, at: s.entity(id).unwrap().kind.clone() });
    }
    let source = s.entities.clone();
    refused(&mut s, Operation::Move { entities: vec![a, b], displacement: p(3.0, 7.0) });
    refused(&mut s, Operation::Rotate { entities: vec![a, b], center: p(4.0, 1.0), angle_degrees: -90.0, copy: false });
    refused(&mut s, Operation::Scale { entities: vec![a, b], base: p(1.0, 2.0), factor: 0.5 });
    let ids = s
        .apply_operation(&Operation::Copy { entities: vec![a, b], base: p(10.0, 5.0), displacement: p(3.0, 7.0) })
        .unwrap();
    assert_eq!(&s.entities[..source.len()], source);
    near(s.entity(ids[0]).unwrap().point(0).unwrap(), p(4.0, 9.0));
    assert!(s.entity(ids[1]).unwrap().construction);
    assert_eq!(s.constraints.len(), 3);
    assert!(
        matches!(s.constraints[2].kind, ConstraintKind::Coincident { a, b } if a.entity == ids[0] && b.entity == ids[1])
    );
    let rotated = s
        .apply_operation(&Operation::Rotate {
            entities: vec![a],
            center: p(1.0, 2.0),
            angle_degrees: -90.0,
            copy: true,
        })
        .unwrap();
    near(s.entity(rotated[0]).unwrap().point(1).unwrap(), p(1.0, -2.0));
    assert_eq!(&s.entities[..source.len()], source);
}

#[test]
fn rotation_and_scaling_cover_every_entity_family_and_roundtrip() {
    let mut s = Sketch::new();
    let kinds = [
        EntityKind::Point { p: p(2.0, 1.0) },
        EntityKind::Line { a: p(2.0, 1.0), b: p(4.0, 1.0) },
        EntityKind::Circle { center: p(3.0, 2.0), radius: 2.0 },
        EntityKind::Arc { center: p(3.0, 2.0), start: p(5.0, 2.0), end: p(3.0, 4.0) },
        EntityKind::Ellipse { center: p(3.0, 2.0), major: p(7.0, 2.0), minor: p(3.0, 4.0) },
        EntityKind::EllipticArc {
            center: p(3.0, 2.0),
            major: p(7.0, 2.0),
            minor: p(3.0, 4.0),
            start: p(7.0, 2.0),
            end: p(3.0, 4.0),
        },
    ];
    let ids: Vec<_> = kinds.into_iter().map(|kind| s.add(kind, true)).collect();
    let before = s.clone();
    let ops = [
        Operation::Rotate { entities: ids.clone(), center: p(1.0, 1.0), angle_degrees: 90.0, copy: false },
        Operation::Scale { entities: ids.clone(), base: p(1.0, 1.0), factor: 0.5 },
    ];
    for op in ops {
        let json = serde_json::to_string(&op).unwrap();
        assert_eq!(serde_json::from_str::<Operation>(&json).unwrap(), op);
        assert_eq!(s.apply_operation(&op).unwrap(), ids);
    }
    for id in ids {
        let old = before.entity(id).unwrap();
        let new = s.entity(id).unwrap();
        for (a, b) in old.points().into_iter().zip(new.points()) {
            let d = a - p(1.0, 1.0);
            near(b, p(1.0, 1.0) + p(-d.v, d.u).scale(0.5));
        }
        if let Some(r) = old.radius() {
            assert!((new.radius().unwrap() - r * 0.5).abs() < 1e-6);
        }
        assert!(new.construction);
    }
    let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    soliddraft_sketch::solve::solve(&mut reopened, None, true);
    assert_eq!(reopened, s);
}

#[test]
fn invalid_selection_parameters_and_overflow_are_atomic() {
    let mut s = Sketch::new();
    let a = s.add(EntityKind::Circle { center: p(3.0, 2.0), radius: 2.0 }, false);
    for entities in [vec![], vec![a, a], vec![0], vec![999]] {
        refused(&mut s, Operation::Move { entities, displacement: p(1.0, 0.0) });
    }
    for factor in [0.0, -1.0, 1.0, f64::NAN, f64::INFINITY, f64::MAX, 1e-10] {
        refused(&mut s, Operation::Scale { entities: vec![a], base: P2::ZERO, factor });
    }
    for displacement in [P2::ZERO, p(f64::NAN, 0.0), p(f64::INFINITY, 0.0)] {
        refused(&mut s, Operation::Copy { entities: vec![a], base: P2::ZERO, displacement });
    }
    for angle_degrees in [0.0, 360.0, f64::NAN, f64::INFINITY] {
        refused(&mut s, Operation::Rotate { entities: vec![a], center: P2::ZERO, angle_degrees, copy: true });
    }
}
