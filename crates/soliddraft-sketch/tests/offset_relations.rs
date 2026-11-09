//! Offset solver relations track their sources without allowing negative circle radii.

use soliddraft_sketch::{ConstraintKind, EntityKind, P2, PointRef, Sketch, SolveStatus, solve};

fn solved(s: &mut Sketch) {
    solve::solve(s, None, true);
    assert!(matches!(s.status, SolveStatus::FullyDefined | SolveStatus::UnderDefined), "{:?}", s.status);
}

#[test]
fn a_circular_offset_follows_radius_edits_and_refuses_negative_radii() {
    let mut s = Sketch::new();
    let a = s.add(EntityKind::Circle { center: P2::new(5.0, 5.0), radius: 5.0 }, true);
    let b = s.add(EntityKind::Circle { center: P2::new(5.0, 5.0), radius: 3.0 }, false);
    s.constrain(ConstraintKind::Fix { point: PointRef::new(a, 0), at: P2::new(5.0, 5.0) });
    let radius = s.constrain(ConstraintKind::Radius { entity: a, value: 5.0, diameter: false, label: P2::ZERO });
    s.constrain(ConstraintKind::Offset {
        a,
        b,
        group: 3,
        distance: 2.0,
        reverse: false,
        opposite: false,
        ends: [None; 2],
    });
    solved(&mut s);
    assert!((s.entity(b).unwrap().radius().unwrap() - 3.0).abs() < 1e-6);
    s.constraint_mut(radius).unwrap().set_value(6.0);
    solved(&mut s);
    assert!((s.entity(b).unwrap().radius().unwrap() - 4.0).abs() < 1e-6);
    s.constraint_mut(radius).unwrap().set_value(1.0);
    solve::solve(&mut s, None, true);
    assert!(matches!(s.status, SolveStatus::OverDefined { .. } | SolveStatus::Failed(_)), "{:?}", s.status);
}

use soliddraft_kernel_api::Plane;
use soliddraft_sketch::{
    profile,
    tools::{OffsetCap, OffsetOptions, Operation},
};

fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn fixed_line(s: &mut Sketch, reverse: bool) -> u32 {
    let (a, b) = if reverse { (p(20.0, 0.0), P2::ZERO) } else { (P2::ZERO, p(20.0, 0.0)) };
    let id = s.add(EntityKind::Line { a, b }, false);
    for (index, at) in [(0, a), (1, b)] {
        s.constrain(ConstraintKind::Fix { point: PointRef::new(id, index), at });
    }
    id
}
fn group(s: &Sketch) -> u32 {
    s.constraints
        .iter()
        .find_map(|c| match c.kind {
            ConstraintKind::Offset { group, .. } => Some(group),
            _ => None,
        })
        .unwrap()
}

#[test]
fn capped_offsets_solve_in_both_directions_and_edit_without_changing_ids() {
    for reverse in [false, true] {
        for both_sides in [false, true] {
            for cap in [OffsetCap::Lines, OffsetCap::Round] {
                for distance in [-2.0, 2.0] {
                    let mut s = Sketch::new();
                    let source = fixed_line(&mut s, reverse);
                    let options = OffsetOptions { chain: true, both_sides, cap, make_base_construction: both_sides };
                    let ids =
                        s.apply_operation(&Operation::Offset { entities: vec![source], distance, options }).unwrap();
                    assert_eq!(ids.len(), if both_sides { 4 } else { 3 });
                    solved(&mut s);
                    assert!(
                        profile::profiles(&s, Plane::XY).is_ok(),
                        "{reverse} {both_sides} {cap:?} {distance}: {:?}",
                        s.entities
                    );
                    let group = group(&s);
                    let changed =
                        s.apply_operation(&Operation::EditOffset { group, distance: -distance * 1.5 }).unwrap();
                    assert_eq!(changed, ids);
                    solved(&mut s);
                    assert!(profile::profiles(&s, Plane::XY).is_ok());
                    assert_eq!(s.offset_group_distance(group), Some(-distance * 1.5));
                    assert_eq!(s.offset_group_targets(group), ids);
                    if cap == OffsetCap::Round {
                        let expected = if both_sides { 3.0 } else { 1.5 };
                        for id in &ids[ids.len() - 2..] {
                            assert!((s.entity(*id).unwrap().radius().unwrap() - expected).abs() < 1e-5);
                        }
                    }
                    let right = PointRef::new(source, u8::from(!reverse));
                    let fixed = s
                        .constraints
                        .iter_mut()
                        .find(|c| matches!(c.kind, ConstraintKind::Fix { point, .. } if point == right))
                        .unwrap();
                    fixed.kind = ConstraintKind::Fix { point: right, at: p(30.0, 0.0) };
                    solved(&mut s);
                    for id in ids.iter().take(if both_sides { 2 } else { 1 }) {
                        let e = s.entity(*id).unwrap();
                        let right = e.points().into_iter().max_by(|a, b| a.u.total_cmp(&b.u)).unwrap();
                        assert!((right.u - 30.0).abs() < 1e-5, "offset must follow source endpoint: {right:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn automatic_chain_selection_extends_both_ways_and_keeps_the_seed_direction() {
    let mut s = Sketch::new();
    let a = s.add(EntityKind::Line { a: P2::ZERO, b: p(10.0, 0.0) }, false);
    let b = s.add(EntityKind::Line { a: p(20.0, 0.0), b: p(10.0, 0.0) }, false);
    let c = s.add(EntityKind::Line { a: p(20.0, 0.0), b: p(30.0, 0.0) }, false);
    assert_eq!(s.offset_chain_from(b).unwrap(), vec![(c, true), (b, false), (a, true)]);
    let ids = s
        .apply_operation(&Operation::Offset {
            entities: vec![b],
            distance: 2.0,
            options: OffsetOptions { chain: true, ..Default::default() },
        })
        .unwrap();
    assert_eq!(ids.len(), 3);
    solved(&mut s);
    for id in ids {
        assert!(s.entity(id).unwrap().points().iter().all(|p| (p.v + 2.0).abs() < 1e-6));
    }
}

#[test]
fn offset_group_refusals_are_atomic_and_legacy_json_stays_unchanged() {
    let mut s = Sketch::new();
    let a = s.add(EntityKind::Line { a: P2::ZERO, b: p(10.0, 0.0) }, false);
    s.add(EntityKind::Line { a: p(10.0, 0.0), b: p(20.0, 0.0) }, false);
    s.add(EntityKind::Line { a: p(10.0, 0.0), b: p(10.0, 10.0) }, false);
    let before = s.clone();
    assert!(
        s.apply_operation(&Operation::Offset {
            entities: vec![a],
            distance: 2.0,
            options: OffsetOptions { chain: true, ..Default::default() }
        })
        .is_err()
    );
    assert_eq!(s, before);
    let mut s = Sketch::new();
    let id = s.add(EntityKind::Circle { center: p(5.0, 5.0), radius: 5.0 }, false);
    s.apply_operation(&Operation::Offset { entities: vec![id], distance: 2.0, options: Default::default() }).unwrap();
    let group = group(&s);
    for distance in [0.0, 6.0, f64::NAN, f64::INFINITY] {
        let before = s.clone();
        assert!(s.apply_operation(&Operation::EditOffset { group, distance }).is_err());
        assert_eq!(s, before);
    }
    let legacy: Operation =
        serde_json::from_value(serde_json::json!({"tool":"offset", "entities":[1], "distance":2})).unwrap();
    assert_eq!(
        serde_json::to_value(legacy).unwrap(),
        serde_json::json!({"tool":"offset", "entities":[1], "distance":2.0})
    );
    let roundtrip: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(roundtrip.constraints, s.constraints);
}

#[test]
fn distance_edits_respect_target_constraints_and_refuse_conflicts_atomically() {
    let mut s = Sketch::new();
    let source = s.add(EntityKind::Circle { center: p(5.0, 5.0), radius: 5.0 }, false);
    s.constrain(ConstraintKind::Fix { point: PointRef::new(source, 0), at: p(5.0, 5.0) });
    let targets = s
        .apply_operation(&Operation::Offset { entities: vec![source], distance: 2.0, options: Default::default() })
        .unwrap();
    let target = targets[0];
    s.constrain(ConstraintKind::Radius { entity: target, value: 3.0, diameter: false, label: P2::ZERO });
    solved(&mut s);
    let group = group(&s);
    s.apply_operation(&Operation::EditOffset { group, distance: 1.0 }).unwrap();
    assert!((s.entity(source).unwrap().radius().unwrap() - 4.0).abs() < 1e-5);
    assert!((s.entity(target).unwrap().radius().unwrap() - 3.0).abs() < 1e-5);
    s.constrain(ConstraintKind::Radius { entity: source, value: 4.0, diameter: false, label: P2::ZERO });
    solved(&mut s);
    let before = s.clone();
    assert!(s.apply_operation(&Operation::EditOffset { group, distance: 2.0 }).is_err());
    assert_eq!(s, before);
}

#[test]
fn an_arc_line_chain_keeps_its_circle_and_round_caps_after_editing() {
    let mut s = Sketch::new();
    let line = s.add(EntityKind::Line { a: p(-10.0, 0.0), b: P2::ZERO }, false);
    let arc = s.add(EntityKind::Arc { center: p(0.0, 5.0), start: P2::ZERO, end: p(5.0, 5.0) }, false);
    s.constrain(ConstraintKind::Coincident { a: PointRef::new(line, 1), b: PointRef::new(arc, 1) });
    for entity in [line, arc] {
        let e = *s.entity(entity).unwrap();
        for index in 0..e.point_count() {
            s.constrain(ConstraintKind::Fix { point: PointRef::new(entity, index), at: e.point(index).unwrap() });
        }
    }
    let ids = s
        .apply_operation(&Operation::Offset {
            entities: vec![arc],
            distance: 1.0,
            options: OffsetOptions {
                chain: true,
                both_sides: true,
                cap: OffsetCap::Round,
                make_base_construction: true,
            },
        })
        .unwrap();
    assert_eq!(ids.len(), 6);
    solved(&mut s);
    assert!(profile::profiles(&s, Plane::XY).is_ok());
    let group = group(&s);
    s.apply_operation(&Operation::EditOffset { group, distance: 2.0 }).unwrap();
    solved(&mut s);
    assert!(profile::profiles(&s, Plane::XY).is_ok());
    let radii: Vec<_> = ids.iter().filter_map(|id| s.entity(*id).unwrap().radius()).collect();
    assert!(radii.iter().any(|r| (*r - 3.0).abs() < 1e-5));
    assert!(radii.iter().any(|r| (*r - 7.0).abs() < 1e-5));
}
