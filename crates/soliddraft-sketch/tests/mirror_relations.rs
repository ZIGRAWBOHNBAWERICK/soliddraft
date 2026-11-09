//! Linked mirrors retain their axis, curve pairing and profile incidence.
use soliddraft_sketch::{
    ConstraintKind, EntityKind, P2, Sketch, SolveStatus, solve,
    tools::{MirrorAxis, Operation, reflected},
};
fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn fixed(s: &mut Sketch, kind: EntityKind, construction: bool) -> u32 {
    let id = s.add(kind, construction);
    s.constrain(ConstraintKind::FixEntity { entity: id, at: kind });
    id
}
fn solved(s: &mut Sketch) {
    solve::solve(s, None, true);
    assert!(matches!(s.status, SolveStatus::FullyDefined | SolveStatus::UnderDefined), "{:?}", s.status);
}
#[test]
fn curve_pairs_follow_source_and_axis_edits_with_reversed_arcs() {
    for source in [
        EntityKind::Point { p: p(4.0, 4.0) },
        EntityKind::Line { a: p(3.0, 0.0), b: p(5.0, 1.0) },
        EntityKind::Circle { center: p(5.0, 2.0), radius: 2.0 },
        EntityKind::Arc { center: p(4.0, 2.0), start: p(5.0, 2.0), end: p(4.0, 3.0) },
    ] {
        let mut s = Sketch::new();
        let axis = fixed(&mut s, EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
        let seed = fixed(&mut s, source, false);
        let ids = s
            .apply_operation(&Operation::MirrorLinked {
                entities: vec![seed],
                axis: MirrorAxis::Entity { entity: axis },
            })
            .unwrap();
        assert_eq!(ids.len(), 1);
        solved(&mut s);
        assert_eq!(s.entity(ids[0]).unwrap().kind, reflected(source, p(0.0, -10.0), p(0.0, 10.0)).unwrap());
        let shifted = EntityKind::Line { a: p(1.0, -10.0), b: p(1.0, 10.0) };
        s.constraints
            .iter_mut()
            .find(|c| matches!(c.kind,ConstraintKind::FixEntity{entity,..} if entity==axis))
            .unwrap()
            .kind = ConstraintKind::FixEntity { entity: axis, at: shifted };
        solved(&mut s);
        let expected = reflected(source, p(1.0, -10.0), p(1.0, 10.0)).unwrap();
        for (a, b) in s
            .entity(ids[0])
            .unwrap()
            .points()
            .iter()
            .zip(soliddraft_sketch::Entity { id: 0, kind: expected, construction: false }.points())
        {
            assert!(a.dist(b) < 1e-6);
        }
        if matches!(source, EntityKind::Circle { .. }) {
            let changed = EntityKind::Circle { center: p(5.0, 2.0), radius: 3.0 };
            s.constraints
                .iter_mut()
                .find(|c| matches!(c.kind,ConstraintKind::FixEntity{entity,..} if entity==seed))
                .unwrap()
                .kind = ConstraintKind::FixEntity { entity: seed, at: changed };
            solved(&mut s);
            assert!((s.entity(ids[0]).unwrap().radius().unwrap() - 3.0).abs() < 1e-6);
        }
    }
}
#[test]
fn placed_axis_is_a_fixed_guide_and_invariant_seeds_do_not_duplicate() {
    let mut s = Sketch::new();
    let seed = s.add(EntityKind::Circle { center: p(5.0, 0.0), radius: 2.0 }, true);
    let op = Operation::MirrorLinked {
        entities: vec![seed],
        axis: MirrorAxis::Points { start: p(0.0, -10.0), end: p(0.0, 10.0) },
    };
    let ids = s.apply_operation(&op).unwrap();
    assert_eq!(ids, vec![3]);
    assert!(s.entity(2).unwrap().construction);
    assert!(s.entity(3).unwrap().construction);
    solved(&mut s);
    let roundtrip: Operation = serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
    assert_eq!(roundtrip, op);
    let mut s = Sketch::new();
    let seed = s.add(EntityKind::Line { a: p(-4.0, 0.0), b: p(4.0, 0.0) }, false);
    assert!(
        s.apply_operation(&Operation::MirrorLinked {
            entities: vec![seed],
            axis: MirrorAxis::Points { start: p(0.0, -10.0), end: p(0.0, 10.0) }
        })
        .unwrap()
        .is_empty()
    );
    assert_eq!(s.drawn().count(), 2);
    assert!(s.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Symmetric { axis: 2, .. })));
    solved(&mut s);
}
#[test]
fn reflected_half_profile_joins_exactly_and_reopens() {
    let mut s = Sketch::new();
    let axis = fixed(&mut s, EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
    let ids: Vec<_> = [(p(0.0, 0.0), p(4.0, 0.0)), (p(4.0, 0.0), p(4.0, 3.0)), (p(4.0, 3.0), p(0.0, 3.0))]
        .into_iter()
        .map(|(a, b)| fixed(&mut s, EntityKind::Line { a, b }, false))
        .collect();
    s.apply_operation(&Operation::MirrorLinked { entities: ids, axis: MirrorAxis::Entity { entity: axis } }).unwrap();
    solved(&mut s);
    assert_eq!(s.regions().unwrap().len(), 1);
    assert!((s.regions().unwrap()[0].area() - 24.0).abs() < 1e-6);
    let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    solved(&mut reopened);
    assert_eq!(s.entities, reopened.entities);
    assert_eq!(s.constraints, reopened.constraints);
}
#[test]
fn invalid_axis_and_selections_refuse_atomically() {
    let mut s = Sketch::new();
    let seed = s.add(EntityKind::Circle { center: p(5.0, 0.0), radius: 2.0 }, false);
    let axis = s.add(EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
    for op in [
        Operation::MirrorLinked { entities: vec![axis, seed], axis: MirrorAxis::Entity { entity: axis } },
        Operation::MirrorLinked { entities: vec![seed], axis: MirrorAxis::Entity { entity: seed } },
        Operation::MirrorLinked { entities: vec![seed], axis: MirrorAxis::Points { start: P2::ZERO, end: P2::ZERO } },
        Operation::MirrorLinked { entities: vec![seed, seed], axis: MirrorAxis::Entity { entity: axis } },
    ] {
        let before = s.clone();
        assert!(s.apply_operation(&op).is_err());
        assert_eq!(s, before);
    }
}

#[test]
fn linked_mirror_refuses_ellipses_without_changing_the_independent_path() {
    for kind in [
        EntityKind::Ellipse { center: p(5.0, 0.0), major: p(8.0, 0.0), minor: p(5.0, 2.0) },
        EntityKind::EllipticArc {
            center: p(5.0, 0.0),
            major: p(8.0, 0.0),
            minor: p(5.0, 2.0),
            start: p(8.0, 0.0),
            end: p(5.0, 2.0),
        },
    ] {
        let mut s = Sketch::new();
        let seed = s.add(kind, false);
        let before = s.clone();
        let start = p(0.0, -10.0);
        let end = p(0.0, 10.0);
        let error = s
            .apply_operation(&Operation::MirrorLinked { entities: vec![seed], axis: MirrorAxis::Points { start, end } })
            .unwrap_err();
        assert!(error.to_string().contains("supports points, lines, circles and circular arcs"));
        assert_eq!(s, before);
        assert_eq!(
            s.apply_operation(&Operation::Mirror { entities: vec![seed], axis_start: start, axis_end: end })
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn existing_symmetric_geometry_keeps_intent_without_duplicate_curves() {
    for kind in [
        EntityKind::Point { p: p(0.0, 4.0) },
        EntityKind::Circle { center: p(0.0, 4.0), radius: 2.0 },
        EntityKind::Line { a: p(-3.0, 4.0), b: p(3.0, 4.0) },
        EntityKind::Line { a: p(0.0, 3.0), b: p(0.0, 5.0) },
        EntityKind::Arc { center: p(0.0, 4.0), start: p(2.0, 4.0), end: p(-2.0, 4.0) },
    ] {
        let mut s = Sketch::new();
        let axis = fixed(&mut s, EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
        let seed = s.add(kind, false);
        let op = Operation::MirrorLinked { entities: vec![seed], axis: MirrorAxis::Entity { entity: axis } };
        assert!(s.apply_operation(&op).unwrap().is_empty());
        assert_eq!(s.drawn().count(), 2);
        assert!(s.constraints.len() > 1);
        solved(&mut s);
        let once = s.clone();
        s.apply_operation(&op).unwrap();
        assert_eq!(s, once, "reapplying does not duplicate constraints");
        s.constraints[0].kind =
            ConstraintKind::FixEntity { entity: axis, at: EntityKind::Line { a: p(1.0, -10.0), b: p(1.0, 10.0) } };
        solved(&mut s);
        let e = s.entity(seed).unwrap();
        let expected = reflected(e.kind, p(1.0, -10.0), p(1.0, 10.0)).unwrap();
        let points = soliddraft_sketch::Entity { id: 0, kind: expected, construction: false }.points();
        let actual = e.points();
        if e.is_line() && actual[0].dist(points[0]) > 1e-6 {
            assert!(actual[0].dist(points[1]) < 1e-6 && actual[1].dist(points[0]) < 1e-6);
        } else {
            assert!(actual.iter().zip(points).all(|(a, b)| a.dist(b) < 1e-6));
        }
    }
    let mut s = Sketch::new();
    let axis = fixed(&mut s, EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
    let a = s.add(EntityKind::Circle { center: p(4.0, 3.0), radius: 2.0 }, false);
    let b = s.add(EntityKind::Circle { center: p(-4.0, 3.0), radius: 2.0 }, false);
    assert!(
        s.apply_operation(&Operation::MirrorLinked { entities: vec![a, b], axis: MirrorAxis::Entity { entity: axis } })
            .unwrap()
            .is_empty()
    );
    assert_eq!(s.drawn().count(), 3);
    assert!(s.constraints.iter().any(|c| matches!(c.kind,ConstraintKind::Equal{a: x,b:y} if x==a && y==b)));
    solved(&mut s);
}
