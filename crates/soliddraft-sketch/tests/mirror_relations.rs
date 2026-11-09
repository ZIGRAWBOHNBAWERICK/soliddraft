//! Linked mirrors retain their axis, curve pairing and profile incidence.
use soliddraft_sketch::{
    ConstraintKind, EntityKind, P2, Sketch, SolveStatus, SplineMode, solve,
    tools::{MirrorAxis, Operation, reflected},
};
fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn fixed(s: &mut Sketch, kind: EntityKind, construction: bool) -> u32 {
    let id = s.add(kind.clone(), construction);
    s.constrain(ConstraintKind::FixEntity { entity: id, at: kind });
    id
}
fn solved(s: &mut Sketch) {
    solve::solve(s, None, true);
    assert!(matches!(s.status, SolveStatus::FullyDefined | SolveStatus::UnderDefined), "{:?}", s.status);
}

#[test]
fn spline_mirrors_retain_every_point_after_source_edits_and_reopen() {
    for mode in [SplineMode::Fit, SplineMode::Control] {
        for retained in [None, Some(false), Some(true)] {
            let mut s = Sketch::new();
            let start = p(0.0, -10.0);
            let end = p(0.0, 10.0);
            let axis = fixed(&mut s, EntityKind::Line { a: start, b: end }, true);
            let mut points = vec![p(2.0, 0.0), p(4.0, 2.0), p(5.0, 5.0), p(3.0, 6.0)];
            let source = fixed(&mut s, EntityKind::Spline { mode, points: points.clone() }, false);
            let mut entities = vec![source];
            if let Some(reverse) = retained {
                let mut partner = reflected(s.entity(source).unwrap().kind.clone(), start, end).unwrap();
                if reverse && let EntityKind::Spline { ref mut points, .. } = partner {
                    points.reverse();
                }
                entities.push(s.add(partner, false));
            }
            let copies = s
                .apply_operation(&Operation::MirrorLinked {
                    entities: entities.clone(),
                    axis: MirrorAxis::Entity { entity: axis },
                })
                .unwrap();
            assert_eq!(copies.len(), usize::from(retained.is_none()));
            let partner = if retained.is_some() { entities[1] } else { copies[0] };
            for _ in 0..2 {
                points[1] = points[1] + p(1.0, 2.0);
                s.constraints
                    .iter_mut()
                    .find(|c| matches!(c.kind, ConstraintKind::FixEntity { entity, .. } if entity == source))
                    .unwrap()
                    .kind = ConstraintKind::FixEntity {
                    entity: source,
                    at: EntityKind::Spline { mode, points: points.clone() },
                };
                solved(&mut s);
                let actual = s.entity(partner).unwrap().points();
                for (i, point) in points.iter().enumerate() {
                    let j = if retained == Some(true) { points.len() - 1 - i } else { i };
                    assert!(p(-point.u, point.v).dist(actual[j]) < 1e-6);
                }
                let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
                solved(&mut reopened);
                assert_eq!(s.constraints, reopened.constraints);
                assert_eq!(s.entities, reopened.entities);
                s = reopened;
                let once = s.clone();
                assert!(
                    s.apply_operation(&Operation::MirrorLinked {
                        entities: vec![source, partner],
                        axis: MirrorAxis::Entity { entity: axis },
                    })
                    .unwrap()
                    .is_empty()
                );
                assert_eq!(s, once, "retained spline mirrors do not duplicate relations");
            }
        }
    }
}

#[test]
fn self_symmetric_splines_retain_their_points_without_duplicate_geometry() {
    for mode in [SplineMode::Fit, SplineMode::Control] {
        for points in [vec![p(-3.0, 1.0), p(0.0, 4.0), p(3.0, 1.0)], vec![p(0.0, 1.0), p(0.0, 4.0), p(0.0, 6.0)]] {
            let mut s = Sketch::new();
            let axis = fixed(&mut s, EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
            let source = s.add(EntityKind::Spline { mode, points }, false);
            let operation =
                Operation::MirrorLinked { entities: vec![source], axis: MirrorAxis::Entity { entity: axis } };
            assert!(s.apply_operation(&operation).unwrap().is_empty());
            assert_eq!(s.drawn().count(), 2);
            solved(&mut s);
            let once = s.clone();
            assert!(s.apply_operation(&operation).unwrap().is_empty());
            assert_eq!(s, once);
            s.constraints[0].kind =
                ConstraintKind::FixEntity { entity: axis, at: EntityKind::Line { a: p(1.0, -10.0), b: p(1.0, 10.0) } };
            solved(&mut s);
            let points = s.entity(source).unwrap().points();
            let matches = |reverse| {
                points.iter().enumerate().all(|(i, point)| {
                    let j = if reverse { points.len() - 1 - i } else { i };
                    p(2.0 - point.u, point.v).dist(points[j]) < 1e-6
                })
            };
            assert!(matches(false) || matches(true));
        }
    }
}
#[test]
fn curve_pairs_follow_source_and_axis_edits_with_reversed_arcs() {
    for source in [
        EntityKind::Point { p: p(4.0, 4.0) },
        EntityKind::Line { a: p(3.0, 0.0), b: p(5.0, 1.0) },
        EntityKind::Circle { center: p(5.0, 2.0), radius: 2.0 },
        EntityKind::Arc { center: p(4.0, 2.0), start: p(5.0, 2.0), end: p(4.0, 3.0) },
        EntityKind::Spline { mode: SplineMode::Fit, points: vec![p(3.0, 0.0), p(5.0, 2.0), p(4.0, 4.0)] },
        EntityKind::Spline { mode: SplineMode::Control, points: vec![p(3.0, 0.0), p(5.0, 2.0), p(4.0, 4.0)] },
    ] {
        let mut s = Sketch::new();
        let axis = fixed(&mut s, EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
        let seed = fixed(&mut s, source.clone(), false);
        let ids = s
            .apply_operation(&Operation::MirrorLinked {
                entities: vec![seed],
                axis: MirrorAxis::Entity { entity: axis },
            })
            .unwrap();
        assert_eq!(ids.len(), 1);
        solved(&mut s);
        assert_eq!(s.entity(ids[0]).unwrap().kind, reflected(source.clone(), p(0.0, -10.0), p(0.0, 10.0)).unwrap());
        let shifted = EntityKind::Line { a: p(1.0, -10.0), b: p(1.0, 10.0) };
        s.constraints
            .iter_mut()
            .find(|c| matches!(c.kind,ConstraintKind::FixEntity{entity,..} if entity==axis))
            .unwrap()
            .kind = ConstraintKind::FixEntity { entity: axis, at: shifted };
        solved(&mut s);
        let expected = reflected(source.clone(), p(1.0, -10.0), p(1.0, 10.0)).unwrap();
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
        assert!(error.to_string().contains("does not support ellipses or partial ellipses"));
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
        let expected = reflected(e.kind.clone(), p(1.0, -10.0), p(1.0, 10.0)).unwrap();
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

#[test]
fn construction_partners_do_not_suppress_profile_copies() {
    let mut s = Sketch::new();
    let axis = fixed(&mut s, EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
    let a = fixed(&mut s, EntityKind::Circle { center: p(4.0, 0.0), radius: 1.0 }, false);
    let b = fixed(&mut s, EntityKind::Circle { center: p(-4.0, 0.0), radius: 1.0 }, true);
    let copies = s
        .apply_operation(&Operation::MirrorLinked { entities: vec![a, b], axis: MirrorAxis::Entity { entity: axis } })
        .unwrap();
    solved(&mut s);
    let area: f64 = s.regions().unwrap().iter().map(|r| r.area()).sum();
    assert!((area - 2.0 * std::f64::consts::PI).abs() < 1e-6, "ordinary copies must produce two profiles, area={area}");
    assert_eq!(copies.len(), 2);
    assert!(!s.entity(copies[0]).unwrap().construction);
    assert!(s.entity(copies[1]).unwrap().construction);
    let reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(s.entities, reopened.entities);
}

#[test]
fn retained_complementary_arcs_keep_shared_axis_ends_after_source_edits_and_reopen() {
    let mut s = Sketch::new();
    let axis = fixed(&mut s, EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
    let a = s.add(EntityKind::Arc { center: P2::ZERO, start: p(0.0, -2.0), end: p(0.0, 2.0) }, false);
    let b = s.add(EntityKind::Arc { center: P2::ZERO, start: p(0.0, 2.0), end: p(0.0, -2.0) }, false);
    let center = s.constrain(ConstraintKind::Fix { point: soliddraft_sketch::PointRef::new(a, 0), at: P2::ZERO });
    s.constrain(ConstraintKind::Radius { entity: a, value: 2.0, diameter: false, label: p(5.0, 5.0) });
    assert!(
        s.apply_operation(&Operation::MirrorLinked { entities: vec![a, b], axis: MirrorAxis::Entity { entity: axis } })
            .unwrap()
            .is_empty()
    );
    solved(&mut s);
    assert_eq!(s.regions().unwrap().len(), 1);
    for x in [1.0, 0.5] {
        s.constraint_mut(center).unwrap().kind =
            ConstraintKind::Fix { point: soliddraft_sketch::PointRef::new(a, 0), at: p(x, 0.0) };
        solved(&mut s);
        assert!(s.entity(a).unwrap().point(0).unwrap().dist(p(x, 0.0)) < 1e-6);
        for (i, j) in [(1, 2), (2, 1)] {
            let p = s.entity(a).unwrap().point(i).unwrap();
            let q = s.entity(b).unwrap().point(j).unwrap();
            assert!(p.dist(q) < 1e-6, "retained junction must remain shared: {p:?} {q:?}");
            assert!(p.u.abs() < 1e-6);
        }
        assert_eq!(s.regions().unwrap().len(), 1);
        let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        solved(&mut reopened);
        assert_eq!(reopened.constraints, s.constraints);
        s = reopened;
    }
}

#[test]
fn retained_lines_use_the_stored_endpoint_correspondence_at_the_axis() {
    use soliddraft_sketch::PointRef;
    for reverse_source in [false, true] {
        for reverse_partner in [false, true] {
            let mut s = Sketch::new();
            let axis = fixed(&mut s, EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
            let (a, b) = if reverse_source { (p(4.0, 0.0), P2::ZERO) } else { (P2::ZERO, p(4.0, 0.0)) };
            let source = s.add(EntityKind::Line { a, b }, false);
            let (a, b) = if reverse_partner { (P2::ZERO, p(-4.0, 0.0)) } else { (p(-4.0, 0.0), P2::ZERO) };
            let partner = s.add(EntityKind::Line { a, b }, false);
            let joint = PointRef::new(source, usize::from(reverse_source));
            let other = PointRef::new(partner, usize::from(!reverse_partner));
            let outer = PointRef::new(source, 1 - joint.index);
            let anchor = s.constrain(ConstraintKind::Fix { point: outer, at: p(4.0, 0.0) });
            assert!(
                s.apply_operation(&Operation::MirrorLinked {
                    entities: vec![source, partner],
                    axis: MirrorAxis::Entity { entity: axis }
                })
                .unwrap()
                .is_empty()
            );
            assert!(s.constraints.iter().any(|c| c.kind == ConstraintKind::Coincident { a: joint, b: other }));
            s.constraint_mut(anchor).unwrap().kind = ConstraintKind::Fix { point: outer, at: p(5.0, 1.0) };
            solved(&mut s);
            assert!(s.point(joint).unwrap().dist(s.point(other).unwrap()) < 1e-6);
            assert!(s.point(joint).unwrap().u.abs() < 1e-6);
        }
    }
}

fn retained_circle_pair() -> (Sketch, Operation, u32, u32) {
    let mut s = Sketch::new();
    let axis = fixed(&mut s, EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) }, true);
    let a = s.add(EntityKind::Circle { center: p(4.0, 0.0), radius: 1.0 }, false);
    let b = s.add(EntityKind::Circle { center: p(-4.0, 0.0), radius: 1.0 }, false);
    let op = Operation::MirrorLinked { entities: vec![a, b], axis: MirrorAxis::Entity { entity: axis } };
    s.apply_operation(&op).unwrap();
    solved(&mut s);
    (s, op, a, b)
}
#[test]
fn retained_mirror_pair_reapplication_keeps_one_relation_and_stable_ids() {
    let (mut s, op, _, _) = retained_circle_pair();
    let before = s.clone();
    for _ in 0..3 {
        assert!(s.apply_operation(&op).unwrap().is_empty());
        assert_eq!(s, before, "reapplying a retained pair is a complete no-op");
    }
}
#[test]
fn retained_mirror_pair_single_removal_releases_the_centre_pairing() {
    let (mut s, op, a, b) = retained_circle_pair();
    s.apply_operation(&op).unwrap();
    let id = s.constraints.iter().find(|c| matches!(c.kind, ConstraintKind::Symmetric { .. })).unwrap().id;
    s.constraints.retain(|c| c.id != id);
    assert!(
        !s.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Symmetric { .. })),
        "one removal releases symmetry"
    );
    for (entity, at) in [(a, p(6.0, 0.0)), (b, p(-4.0, 0.0))] {
        s.constrain(ConstraintKind::Fix { point: soliddraft_sketch::PointRef::new(entity, 0), at });
    }
    solved(&mut s);
    for (entity, at) in [(a, p(6.0, 0.0)), (b, p(-4.0, 0.0))] {
        assert!(s.entity(entity).unwrap().point(0).unwrap().dist(at) < 1e-6);
    }
}
