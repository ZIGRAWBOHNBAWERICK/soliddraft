//! Stored relation sets solve after edits and reject invalid selections atomically.

use soliddraft_sketch::{
    ConstraintKind, EntityKind, P2, PointRef, Sketch, SolveStatus, relations::Relation, solve, tools::Operation,
};

fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn line(s: &mut Sketch, a: P2, b: P2) -> u32 {
    s.add(EntityKind::Line { a, b }, true)
}
fn relate(s: &mut Sketch, relation: Relation) {
    s.apply_operation(&Operation::Relate { relation }).unwrap();
}
fn fix(s: &mut Sketch, entity: u32) {
    relate(s, Relation::FixEntity { entity });
}
fn solved(s: &mut Sketch) {
    solve::solve(s, None, true);
    assert!(matches!(s.status, SolveStatus::UnderDefined | SolveStatus::FullyDefined), "{:?}", s.status);
}
fn near(a: P2, b: P2) {
    assert!(a.dist(b) < 1e-5, "{a:?} != {b:?}");
}
fn point(s: &Sketch, e: u32, i: usize) -> P2 {
    s.point(PointRef::new(e, i)).unwrap()
}

#[test]
fn composite_relations_solve_and_do_not_duplicate_partial_sets() {
    let mut s = Sketch::new();
    let a = line(&mut s, p(-10.0, -5.0), p(10.0, 5.0));
    let b = line(&mut s, p(12.0, 7.0), p(20.0, 11.0));
    fix(&mut s, a);
    let req = Relation::Collinear { a, b };
    let kinds = s.relation_constraints(&req).unwrap();
    s.constrain(kinds[0].clone());
    relate(&mut s, req);
    assert_eq!(s.constraints.len(), 3);
    relate(&mut s, req);
    assert_eq!(s.constraints.len(), 3);
    solved(&mut s);
    for i in 0..2 {
        assert!((point(&s, b, i).v - 0.5 * point(&s, b, i).u).abs() < 1e-5);
    }
    let cross = line(&mut s, p(4.0, -5.0), p(4.0, 10.0));
    fix(&mut s, cross);
    let mark = s.add(EntityKind::Point { p: p(4.2, 2.3) }, true);
    relate(&mut s, Relation::Intersection { point: PointRef::new(mark, 0), a, b: cross });
    solved(&mut s);
    near(point(&s, mark, 0), p(4.0, 2.0));
    let c = s.add(EntityKind::Circle { center: p(-3.0, 6.0), radius: 3.0 }, true);
    let d = s.add(EntityKind::Arc { center: p(-2.5, 6.5), start: p(1.5, 6.5), end: p(-2.5, 10.5) }, true);
    fix(&mut s, c);
    relate(&mut s, Relation::Coradial { a: c, b: d });
    solved(&mut s);
    near(point(&s, c, 0), point(&s, d, 0));
    assert!((s.entity(d).unwrap().radius().unwrap() - 3.0).abs() < 1e-5);
}

#[test]
fn point_symmetry_tracks_an_oblique_axis_and_a_changed_fixed_source() {
    let mut s = Sketch::new();
    let axis = line(&mut s, p(-10.0, -10.0), p(10.0, 10.0));
    let a = s.add(EntityKind::Point { p: p(2.0, 7.0) }, false);
    let b = s.add(EntityKind::Point { p: p(7.2, 2.1) }, false);
    fix(&mut s, axis);
    fix(&mut s, a);
    relate(&mut s, Relation::SymmetricPoints { a: PointRef::new(a, 0), b: PointRef::new(b, 0), axis });
    solved(&mut s);
    near(point(&s, b, 0), p(7.0, 2.0));
    let fixed = s
        .constraints
        .iter_mut()
        .find(|c| matches!(c.kind, ConstraintKind::FixEntity { entity, .. } if entity == a))
        .unwrap();
    fixed.kind = ConstraintKind::FixEntity { entity: a, at: EntityKind::Point { p: p(3.0, 8.0) } };
    solved(&mut s);
    near(point(&s, b, 0), p(8.0, 3.0));
    let roundtrip: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(roundtrip.entities, s.entities);
    assert_eq!(roundtrip.constraints, s.constraints);
}

#[test]
fn entity_symmetry_keeps_endpoint_correspondence_and_circle_radius() {
    let mut s = Sketch::new();
    let axis = line(&mut s, p(0.0, -20.0), p(0.0, 20.0));
    let a = line(&mut s, p(-5.0, -4.0), p(-2.0, 3.0));
    let b = line(&mut s, p(2.1, 3.2), p(5.2, -4.1));
    fix(&mut s, axis);
    fix(&mut s, a);
    relate(&mut s, Relation::SymmetricEntities { a, b, axis });
    solved(&mut s);
    near(point(&s, b, 0), p(2.0, 3.0));
    near(point(&s, b, 1), p(5.0, -4.0));
    let a = s.add(EntityKind::Circle { center: p(-6.0, 7.0), radius: 2.0 }, true);
    let b = s.add(EntityKind::Circle { center: p(6.2, 7.1), radius: 2.2 }, true);
    fix(&mut s, a);
    relate(&mut s, Relation::SymmetricEntities { a, b, axis });
    solved(&mut s);
    near(point(&s, b, 0), p(6.0, 7.0));
    assert!((s.entity(b).unwrap().radius().unwrap() - 2.0).abs() < 1e-5);
}

#[test]
fn reflected_arcs_reverse_their_endpoints() {
    let mut s = Sketch::new();
    let axis = line(&mut s, p(0.0, -20.0), p(0.0, 20.0));
    let a = s.add(EntityKind::Arc { center: p(-6.0, 0.0), start: p(-4.0, 0.0), end: p(-6.0, 2.0) }, false);
    let b = s.add(EntityKind::Arc { center: p(6.1, 0.0), start: p(6.1, 2.1), end: p(4.0, 0.0) }, false);
    fix(&mut s, axis);
    fix(&mut s, a);
    relate(&mut s, Relation::SymmetricEntities { a, b, axis });
    solved(&mut s);
    near(point(&s, b, 0), p(6.0, 0.0));
    near(point(&s, b, 1), p(6.0, 2.0));
    near(point(&s, b, 2), p(4.0, 0.0));
}

#[test]
fn whole_circle_fix_restores_radius_and_releases_when_removed() {
    let mut s = Sketch::new();
    let id = s.add(EntityKind::Circle { center: p(5.0, 8.0), radius: 4.0 }, false);
    fix(&mut s, id);
    let stored = s.constraints[0].clone();
    s.entity_mut(id).unwrap().kind = EntityKind::Circle { center: p(5.5, 8.5), radius: 4.5 };
    fix(&mut s, id);
    assert_eq!(s.constraints, vec![stored]);
    solved(&mut s);
    near(point(&s, id, 0), p(5.0, 8.0));
    assert!((s.entity(id).unwrap().radius().unwrap() - 4.0).abs() < 1e-5);
    assert_eq!(s.status, SolveStatus::FullyDefined);
    s.constraints.clear();
    solved(&mut s);
    assert_eq!(s.status, SolveStatus::UnderDefined);
}

#[test]
fn invalid_requests_refuse_atomically_and_schema_roundtrips() {
    let mut s = Sketch::new();
    let a = line(&mut s, p(-2.0, 0.0), p(2.0, 0.0));
    let b = s.add(EntityKind::Circle { center: p(4.0, 4.0), radius: 2.0 }, false);
    for relation in [
        Relation::FixEntity { entity: 0 },
        Relation::FixEntity { entity: 100 },
        Relation::Collinear { a, b },
        Relation::Collinear { a, b: a },
        Relation::Coradial { a, b },
        Relation::Intersection { point: PointRef::new(b, 4), a, b: a },
        Relation::SymmetricPoints { a: PointRef::new(a, 0), b: PointRef::new(b, 0), axis: a },
        Relation::SymmetricEntities { a, b, axis: a },
    ] {
        let before = s.clone();
        let op = Operation::Relate { relation };
        assert!(s.apply_operation(&op).is_err(), "{op:?}");
        assert_eq!(s, before);
        assert_eq!(serde_json::from_str::<Operation>(&serde_json::to_string(&op).unwrap()).unwrap(), op);
    }
}

#[test]
fn intersection_can_use_an_endpoint_of_one_selected_line() {
    let mut s = Sketch::new();
    let a = line(&mut s, p(0.0, 0.0), p(5.0, 1.0));
    let b = line(&mut s, p(4.0, -10.0), p(4.0, 10.0));
    fix(&mut s, b);
    s.constrain(ConstraintKind::Fix { point: PointRef::new(a, 0), at: P2::ZERO });
    let request = Relation::Intersection { point: PointRef::new(a, 1), a, b };
    assert_eq!(
        s.relation_constraints(&request).unwrap(),
        vec![ConstraintKind::PointOn { point: PointRef::new(a, 1), entity: b }]
    );
    relate(&mut s, request);
    solved(&mut s);
    assert!((point(&s, a, 1).u - 4.0).abs() < 1e-5);
}

#[test]
fn reflected_ellipses_keep_their_axes_and_partial_span_after_edits_and_reopening() {
    for partial in [false, true] {
        let ellipse = |center, major, minor, start, end| {
            if partial {
                EntityKind::EllipticArc { center, major, minor, start, end }
            } else {
                EntityKind::Ellipse { center, major, minor }
            }
        };
        let mut s = Sketch::new();
        let axis = line(&mut s, p(0.0, -20.0), p(0.0, 20.0));
        let a = s.add(ellipse(p(-6.0, 0.0), p(-2.0, 0.0), p(-6.0, 2.0), p(-2.0, 0.0), p(-10.0, 0.0)), false);
        let b = s.add(ellipse(p(6.2, 0.1), p(2.1, 0.1), p(6.2, 2.2), p(10.3, 0.1), p(2.1, 0.1)), false);
        fix(&mut s, axis);
        fix(&mut s, a);
        relate(&mut s, Relation::SymmetricEntities { a, b, axis });
        solved(&mut s);
        for (index, expected) in [(0, p(6.0, 0.0)), (1, p(2.0, 0.0)), (2, p(6.0, 2.0))] {
            near(point(&s, b, index), expected);
        }
        if partial {
            near(point(&s, b, 3), p(10.0, 0.0));
            near(point(&s, b, 4), p(2.0, 0.0));
        }
        let json = serde_json::to_string(&s).unwrap();
        let mut s: Sketch = serde_json::from_str(&json).unwrap();
        let edited = ellipse(p(-7.0, 1.0), p(-2.0, 1.0), p(-7.0, 4.0), p(-2.0, 1.0), p(-12.0, 1.0));
        s.entity_mut(a).unwrap().kind = edited.clone();
        for constraint in &mut s.constraints {
            if let ConstraintKind::FixEntity { entity, at } = &mut constraint.kind
                && *entity == a
            {
                *at = edited.clone();
            }
        }
        solved(&mut s);
        for (index, expected) in [(0, p(7.0, 1.0)), (1, p(2.0, 1.0)), (2, p(7.0, 4.0))] {
            near(point(&s, b, index), expected);
        }
        if partial {
            near(point(&s, b, 3), p(12.0, 1.0));
            near(point(&s, b, 4), p(2.0, 1.0));
            let kind = &s.entity(b).unwrap().kind;
            let geometry = soliddraft_sketch::ellipse::Ellipse::of(kind).unwrap();
            let (start, sweep) = geometry.span(kind);
            assert!(geometry.at(start + sweep / 2.0).v > geometry.center.v);
        }
    }
}

#[test]
fn spline_symmetry_keeps_every_point_pair_after_fixed_source_edits_and_reopening() {
    use soliddraft_sketch::SplineMode;
    for mode in [SplineMode::Fit, SplineMode::Control] {
        for reversed in [false, true] {
            let mut s = Sketch::new();
            let axis = line(&mut s, p(0.0, -10.0), p(0.0, 10.0));
            fix(&mut s, axis);
            let points = vec![p(1.0, 0.0), p(2.0, 3.0), p(4.0, 2.0), p(5.0, 0.0)];
            let a = s.add(EntityKind::Spline { mode, points: points.clone() }, false);
            let mut reflected: Vec<_> = points.iter().map(|q| p(-q.u, q.v)).collect();
            if reversed {
                reflected.reverse();
            }
            let b = s.add(EntityKind::Spline { mode, points: reflected }, false);
            fix(&mut s, a);
            relate(&mut s, Relation::SymmetricEntities { a, b, axis });
            solved(&mut s);
            let mut edited = points;
            edited[1] = p(3.0, 5.0);
            let snapshot = EntityKind::Spline { mode, points: edited.clone() };
            for c in &mut s.constraints {
                if let ConstraintKind::FixEntity { entity, at } = &mut c.kind
                    && *entity == a
                {
                    *at = snapshot.clone();
                }
            }
            solved(&mut s);
            let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
            solved(&mut reopened);
            for (index, q) in edited.iter().enumerate() {
                let other = if reversed { edited.len() - 1 - index } else { index };
                near(point(&reopened, b, other), p(-q.u, q.v));
            }
        }
    }
}

#[test]
fn spline_copies_transform_the_whole_polynomial_in_both_modes() {
    use soliddraft_sketch::{SplineMode, spline};
    for mode in [SplineMode::Fit, SplineMode::Control] {
        let points = vec![p(1.0, 0.0), p(2.0, 4.0), p(3.0, 5.0), p(5.0, 2.0), p(6.0, 0.0)];
        for transform in 0..3 {
            let mut s = Sketch::new();
            let seed = s.add(EntityKind::Spline { mode, points: points.clone() }, false);
            let operation = match transform {
                0 => Operation::Mirror { entities: vec![seed], axis_start: p(0.0, -1.0), axis_end: p(0.0, 1.0) },
                1 => Operation::LinearPattern { entities: vec![seed], step: p(10.0, 20.0), count: 2 },
                _ => {
                    Operation::CircularPattern { entities: vec![seed], center: P2::ZERO, angle_degrees: 90.0, count: 2 }
                }
            };
            let copied = s.apply_operation(&operation).unwrap();
            assert_eq!(copied.len(), 1);
            let EntityKind::Spline { mode: actual_mode, points: actual } = &s.entity(copied[0]).unwrap().kind else {
                panic!()
            };
            assert_eq!(*actual_mode, mode);
            let map = |q: P2| match transform {
                0 => p(-q.u, q.v),
                1 => q + p(10.0, 20.0),
                _ => p(-q.v, q.u),
            };
            let before = spline::spans(mode, &points).unwrap();
            let after = spline::spans(mode, actual).unwrap();
            assert_eq!(before.len(), after.len());
            for (a, b) in before.iter().zip(&after) {
                for t in [0.0, 0.17, 0.5, 0.83, 1.0] {
                    near(b.at(t), map(a.at(t)));
                }
            }
        }
    }
}

#[test]
fn point_on_spline_keeps_constant_spans_solvable_after_dimension_edits() {
    use soliddraft_sketch::{DistanceKind, SplineMode};
    for solve_first in [false, true] {
        for reversed in [false, true] {
            let mut s = Sketch::new();
            let mut points = vec![P2::ZERO; 4];
            points.push(p(1.0, 0.0));
            if reversed {
                points.reverse();
            }
            let curve = s.add(EntityKind::Spline { mode: SplineMode::Control, points }, false);
            fix(&mut s, curve);
            let marker = PointRef::new(s.add(EntityKind::Point { p: P2::ZERO }, true), 0);
            s.constrain(ConstraintKind::PointOn { point: marker, entity: curve });
            if solve_first {
                solved(&mut s);
            }
            let dimension = s.constrain(ConstraintKind::Distance {
                a: marker,
                b: PointRef::ORIGIN,
                kind: DistanceKind::Horizontal,
                value: 0.5,
                label: P2::ZERO,
            });
            solved(&mut s);
            near(s.point(marker).unwrap(), p(0.5, 0.0));
            let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
            for value in [0.125, 0.875, 0.5] {
                reopened.constraint_mut(dimension).unwrap().set_value(value);
                solved(&mut reopened);
                near(reopened.point(marker).unwrap(), p(value, 0.0));
            }
            reopened.constraint_mut(dimension).unwrap().set_value(1.25);
            solve::solve(&mut reopened, None, true);
            assert!(matches!(reopened.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }));
        }
    }
}

#[test]
fn point_on_spline_retries_a_parallel_return_span_after_two_dimensions() {
    use soliddraft_sketch::{DistanceKind, SplineMode};
    for mode in [SplineMode::Fit, SplineMode::Control] {
        for reversed in [false, true] {
            let mut s = Sketch::new();
            let mut points = vec![
                p(0.0, 0.0),
                p(1.0, 0.0),
                p(2.0, 0.0),
                p(3.0, 0.0),
                p(3.0, 1.0),
                p(2.0, 1.0),
                p(1.0, 1.0),
                p(0.0, 1.0),
            ];
            if reversed {
                points.reverse();
            }
            let curve = s.add(EntityKind::Spline { mode, points }, false);
            fix(&mut s, curve);
            near(s.entity(curve).unwrap().nearest(p(1.0, 1.0)).0, p(1.0, 1.0));
            let marker = PointRef::new(s.add(EntityKind::Point { p: p(1.0, 0.01) }, true), 0);
            s.constrain(ConstraintKind::PointOn { point: marker, entity: curve });
            for kind in [DistanceKind::Horizontal, DistanceKind::Vertical] {
                s.constrain(ConstraintKind::Distance {
                    a: marker,
                    b: PointRef::ORIGIN,
                    kind,
                    value: 1.0,
                    label: P2::ZERO,
                });
            }
            solve::solve(&mut s, None, true);
            assert!(
                matches!(s.status, SolveStatus::UnderDefined | SolveStatus::FullyDefined),
                "{mode:?}, reversed={reversed}: {:?}",
                s.status
            );
            near(s.point(marker).unwrap(), p(1.0, 1.0));
            let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
            solved(&mut reopened);
            near(reopened.point(marker).unwrap(), p(1.0, 1.0));
        }
    }
}
