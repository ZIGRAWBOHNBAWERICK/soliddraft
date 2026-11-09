//! Retained pattern parameters and solver links survive source edits and omitted copies.
use soliddraft_sketch::{
    ConstraintKind, Entity, EntityKind, P2, Sketch, SolveStatus, SplineMode, solve,
    tools::{Operation, PatternDirection, PatternSpec},
};
fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn solved(s: &mut Sketch) {
    solve::solve(s, None, true);
    assert!(matches!(s.status, SolveStatus::FullyDefined | SolveStatus::UnderDefined), "{:?}", s.status);
}
fn fixed(s: &mut Sketch, kind: EntityKind, construction: bool) -> u32 {
    let id = s.add(kind.clone(), construction);
    s.constrain(ConstraintKind::FixEntity { entity: id, at: kind });
    id
}
fn linear() -> PatternSpec {
    PatternSpec::Linear {
        step: p(10.0, 0.0),
        count: 3,
        second: Some(PatternDirection { step: p(0.0, 12.0), count: 2 }),
    }
}
#[test]
fn every_curve_family_follows_source_edits_through_translation_and_rotation() {
    for (n, kind) in [
        EntityKind::Point { p: P2::ZERO },
        EntityKind::Line { a: p(3.0, 0.0), b: p(5.0, 1.0) },
        EntityKind::Circle { center: p(4.0, 0.0), radius: 2.0 },
        EntityKind::Arc { center: p(4.0, 0.0), start: p(6.0, 0.0), end: p(4.0, 2.0) },
        EntityKind::Ellipse { center: p(5.0, 2.0), major: p(8.0, 2.0), minor: p(5.0, 3.0) },
        EntityKind::EllipticArc {
            center: p(5.0, 2.0),
            major: p(8.0, 2.0),
            minor: p(5.0, 3.0),
            start: p(8.0, 2.0),
            end: p(5.0, 3.0),
        },
        EntityKind::Spline { mode: SplineMode::Fit, points: vec![p(3.0, 0.0), p(6.0, 2.0), p(4.0, 5.0), p(1.0, 3.0)] },
        EntityKind::Spline {
            mode: SplineMode::Control,
            points: vec![p(3.0, 0.0), p(6.0, 2.0), p(4.0, 5.0), p(1.0, 3.0)],
        },
    ]
    .into_iter()
    .enumerate()
    {
        for circular in [false, true] {
            let mut s = Sketch::new();
            let seed = fixed(&mut s, kind.clone(), n % 2 == 0);
            let spec = if circular {
                PatternSpec::Circular { center: P2::ZERO, angle_degrees: 360.0, count: 4, total_angle: true }
            } else {
                PatternSpec::Linear { step: p(20.0, 5.0), count: 3, second: None }
            };
            s.apply_operation(&Operation::Pattern { entities: vec![seed], spec, skipped: vec![] }).unwrap();
            solved(&mut s);
            let mut shifted = Entity { id: seed, kind: kind.clone(), construction: n % 2 == 0 };
            for (i, at) in shifted.points().into_iter().enumerate() {
                shifted.set_point(i, at + p(2.0, 3.0));
            }
            if let EntityKind::Circle { radius, .. } = &mut shifted.kind {
                *radius = 3.0;
            }
            s.constraints
                .iter_mut()
                .find(|c| matches!(c.kind,ConstraintKind::FixEntity{entity,..} if entity==seed))
                .unwrap()
                .kind = ConstraintKind::FixEntity { entity: seed, at: shifted.kind.clone() };
            solved(&mut s);
            for instance in &s.patterns[0].instances {
                let index = f64::from(instance.position[0]);
                let target = s.entity(instance.entities[0]).unwrap();
                for (a, b) in shifted.points().into_iter().zip(target.points()) {
                    let expected = if circular {
                        match instance.position[0] {
                            1 => p(-a.v, a.u),
                            2 => p(-a.u, -a.v),
                            3 => p(a.v, -a.u),
                            _ => unreachable!(),
                        }
                    } else {
                        a + p(20.0 * index, 5.0 * index)
                    };
                    assert!(
                        b.dist(expected) < 1e-6,
                        "family={n}, circular={circular}, instance={index}: {b:?} expected {expected:?}"
                    );
                }
                assert_eq!(target.construction, shifted.construction);
                if circular && matches!(kind, EntityKind::Circle { .. }) {
                    assert!((target.radius().unwrap() - 3.0).abs() < 1e-6);
                }
            }
        }
    }
}
#[test]
fn spline_grid_edits_keep_point_transforms_and_surviving_ids_after_reopen() {
    for mode in [SplineMode::Fit, SplineMode::Control] {
        let mut s = Sketch::new();
        let points = vec![p(1.0, 0.0), p(3.0, 2.0), p(4.0, 5.0), p(2.0, 6.0)];
        let seed = fixed(&mut s, EntityKind::Spline { mode, points: points.clone() }, false);
        s.apply_operation(&Operation::Pattern { entities: vec![seed], spec: linear(), skipped: vec![[1, 1]] }).unwrap();
        solved(&mut s);
        let before = s.patterns[0].clone();
        let spec = PatternSpec::Linear {
            step: p(12.0, 0.0),
            count: 4,
            second: Some(PatternDirection { step: p(0.0, 15.0), count: 2 }),
        };
        s.apply_operation(&Operation::EditPattern { group: before.id, spec, skipped: vec![[1, 1], [2, 0]] }).unwrap();
        solved(&mut s);
        for instance in &s.patterns[0].instances {
            if let Some(prior) = before.instances.iter().find(|old| old.position == instance.position) {
                assert_eq!(instance.entities, prior.entities);
            }
            let offset = p(12.0 * f64::from(instance.position[0]), 15.0 * f64::from(instance.position[1]));
            for (source, actual) in points.iter().zip(s.entity(instance.entities[0]).unwrap().points()) {
                assert!((*source + offset).dist(actual) < 1e-6);
            }
        }
        let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        reopened.normalise();
        solved(&mut reopened);
        assert_eq!(s.entities, reopened.entities);
        assert_eq!(s.constraints, reopened.constraints);
        assert_eq!(s.patterns, reopened.patterns);
    }
}

#[test]
fn grid_edits_preserve_surviving_ids_and_skipped_definitions_reopen() {
    let mut s = Sketch::new();
    let seed = fixed(&mut s, EntityKind::Circle { center: P2::ZERO, radius: 2.0 }, false);
    s.apply_operation(&Operation::Pattern { entities: vec![seed], spec: linear(), skipped: vec![[1, 1]] }).unwrap();
    solved(&mut s);
    let unchanged = s.clone();
    s.apply_operation(&Operation::EditPattern { group: 1, spec: linear(), skipped: vec![[1, 1]] }).unwrap();
    assert_eq!(s, unchanged, "unchanged edits retain relation ids and counters");
    let old = s.patterns[0].clone();
    assert_eq!(old.instances.len(), 4);
    let spec = PatternSpec::Linear {
        step: p(12.0, 0.0),
        count: 4,
        second: Some(PatternDirection { step: p(0.0, 15.0), count: 2 }),
    };
    s.apply_operation(&Operation::EditPattern { group: old.id, spec, skipped: vec![[1, 1], [2, 0]] }).unwrap();
    for instance in &s.patterns[0].instances {
        if let Some(prior) = old.instances.iter().find(|i| i.position == instance.position) {
            assert_eq!(instance.entities, prior.entities);
        }
        let EntityKind::Circle { center, .. } = s.entity(instance.entities[0]).unwrap().kind else { panic!("circle") };
        assert!(center.dist(p(12.0 * f64::from(instance.position[0]), 15.0 * f64::from(instance.position[1]))) < 1e-6);
    }
    let all = spec.positions().unwrap().into_iter().filter(|p| *p != [0, 0]).collect();
    s.apply_operation(&Operation::EditPattern { group: old.id, spec, skipped: all }).unwrap();
    assert_eq!(s.drawn().count(), 1);
    assert_eq!(s.patterns.len(), 1);
    assert!(s.patterns[0].instances.is_empty());
    let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    reopened.normalise();
    solved(&mut reopened);
    assert_eq!(s.patterns, reopened.patterns);
    reopened.apply_operation(&Operation::EditPattern { group: old.id, spec, skipped: vec![] }).unwrap();
    assert_eq!(reopened.patterns[0].instances.len(), 7);
}
#[test]
fn total_spans_include_partial_endpoints_and_do_not_duplicate_a_full_turn() {
    for (angle, expected) in
        [(180.0, [60.0_f64, 120.0, 180.0]), (360.0, [90.0, 180.0, 270.0]), (-360.0, [-90.0, -180.0, -270.0])]
    {
        let mut s = Sketch::new();
        let seed = fixed(&mut s, EntityKind::Point { p: p(10.0, 0.0) }, false);
        s.apply_operation(&Operation::Pattern {
            entities: vec![seed],
            spec: PatternSpec::Circular { center: P2::ZERO, angle_degrees: angle, count: 4, total_angle: true },
            skipped: vec![],
        })
        .unwrap();
        solved(&mut s);
        for (instance, degrees) in s.patterns[0].instances.iter().zip(expected) {
            let radians = degrees.to_radians();
            let at = s.entity(instance.entities[0]).unwrap().point(0).unwrap();
            assert!(at.dist(p(10.0 * radians.cos(), 10.0 * radians.sin())) < 1e-6);
        }
    }
}
#[test]
fn conflicting_edits_and_invalid_skips_refuse_without_mutation() {
    let mut s = Sketch::new();
    let seed = fixed(&mut s, EntityKind::Circle { center: P2::ZERO, radius: 2.0 }, false);
    s.apply_operation(&Operation::Pattern { entities: vec![seed], spec: linear(), skipped: vec![] }).unwrap();
    solved(&mut s);
    let instance = s.patterns[0].instances[0].entities[0];
    let at = s.entity(instance).unwrap().kind.clone();
    s.constrain(ConstraintKind::FixEntity { entity: instance, at });
    solved(&mut s);
    for operation in [
        Operation::EditPattern { group: 1, spec: linear(), skipped: vec![[1, 0]] },
        Operation::EditPattern {
            group: 1,
            spec: PatternSpec::Linear { step: p(14.0, 0.0), count: 3, second: None },
            skipped: vec![],
        },
        Operation::Pattern { entities: vec![seed], spec: linear(), skipped: vec![[0, 0]] },
        Operation::Pattern { entities: vec![seed], spec: linear(), skipped: vec![[3, 0]] },
        Operation::Pattern { entities: vec![seed], spec: linear(), skipped: vec![[1, 0], [1, 0]] },
        Operation::Pattern {
            entities: vec![seed],
            spec: PatternSpec::Linear {
                step: p(10.0, 0.0),
                count: 3,
                second: Some(PatternDirection { step: p(20.0, 0.0), count: 2 }),
            },
            skipped: vec![],
        },
        Operation::Pattern {
            entities: vec![seed],
            spec: PatternSpec::Circular { center: P2::ZERO, angle_degrees: 180.0, count: 4, total_angle: false },
            skipped: vec![],
        },
    ] {
        let before = s.clone();
        assert!(s.apply_operation(&operation).is_err(), "{operation:?}");
        assert_eq!(s, before);
    }
}
#[test]
fn direct_link_deletion_dissolves_the_definition_without_deleting_other_geometry() {
    let mut s = Sketch::new();
    let seed = s.add(EntityKind::Line { a: p(1.0, 0.0), b: p(3.0, 0.0) }, false);
    s.apply_operation(&Operation::Pattern { entities: vec![seed], spec: linear(), skipped: vec![] }).unwrap();
    solved(&mut s);
    let before = s.entities.clone();
    let id = s.constraints.iter().find(|c| matches!(c.kind, ConstraintKind::PatternTransform { .. })).unwrap().id;
    s.remove_constraint(id);
    solved(&mut s);
    assert!(s.patterns.is_empty());
    assert_eq!(s.entities, before);
    s.apply_operation(&Operation::Pattern { entities: vec![seed], spec: linear(), skipped: vec![] }).unwrap();
    assert_eq!(s.patterns[0].id, 2);
    let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    reopened.normalise();
    reopened.remove_entity(seed);
    assert!(reopened.patterns.is_empty());
    let empty = serde_json::to_value(Sketch::new()).unwrap();
    assert!(empty.get("patterns").is_none());
    assert!(empty.get("next_pattern").is_none());
}

fn restore_pattern_after_power_trim(index: u32) {
    let mut sketch = Sketch::new();
    let seed = sketch.add(EntityKind::Line { a: P2::ZERO, b: p(2.0, 0.0) }, false);
    sketch
        .apply_operation(&Operation::Pattern {
            entities: vec![seed],
            spec: PatternSpec::Linear { step: p(0.0, 10.0), count: 4, second: None },
            skipped: vec![],
        })
        .unwrap();
    solved(&mut sketch);
    let before = sketch.clone();
    let y = f64::from(index) * 10.0;
    let cuts = sketch.trim_stroke(p(1.0, y - 1.0), p(1.0, y + 1.0), Default::default()).unwrap();
    assert_eq!(cuts.len(), 1);
    assert!(sketch.patterns.is_empty(), "cutting a copy dissolves its definition");
    for cut in cuts.into_iter().rev() {
        cut.restore(&mut sketch);
    }
    assert_eq!(sketch, before, "retrace restores the entire pattern, relations and counters");
}
#[test]
fn last_pattern_copy_power_trim_retrace_restores_the_complete_sketch() {
    restore_pattern_after_power_trim(3);
}
#[test]
fn first_pattern_copy_power_trim_retrace_restores_the_complete_sketch() {
    restore_pattern_after_power_trim(1);
}
