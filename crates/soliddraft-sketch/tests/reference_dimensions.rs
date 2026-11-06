//! Reference measurements, solver exclusion and atomic mode conversion.

use soliddraft_sketch::{
    Constraint, ConstraintKind, DistanceKind, EntityKind, P2, PointRef, ReferenceDimension, Sketch, SolveStatus, solve,
};

fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn reference(s: &mut Sketch, kind: ConstraintKind) -> u32 {
    s.constrain(kind.as_reference().unwrap())
}
fn value(s: &Sketch, id: u32) -> f64 {
    s.constraint(id).unwrap().display_value(s).unwrap()
}

#[test]
fn every_reference_family_measures_current_geometry_without_mutating_it() {
    let mut s = Sketch::new();
    let a = s.add(EntityKind::Point { p: p(3.0, 4.0) }, false);
    let x = s.add(EntityKind::Line { a: P2::ZERO, b: p(10.0, 0.0) }, false);
    let y = s.add(EntityKind::Line { a: P2::ZERO, b: p(-10.0, 10.0) }, false);
    let circle = s.add(EntityKind::Circle { center: p(20.0, 20.0), radius: 5.0 }, false);
    let arc = s.add(EntityKind::Arc { center: p(30.0, 30.0), start: p(33.0, 30.0), end: p(30.0, 33.0) }, false);
    let mut cases = Vec::new();
    for (kind, expected) in
        [(DistanceKind::Aligned, 5.0), (DistanceKind::Horizontal, 3.0), (DistanceKind::Vertical, 4.0)]
    {
        cases.push((
            reference(
                &mut s,
                ConstraintKind::Distance {
                    a: PointRef::new(a, 0),
                    b: PointRef::new(0, 0),
                    kind,
                    value: 99.0,
                    label: p(1.0, 2.0),
                },
            ),
            expected,
        ));
    }
    cases.push((
        reference(
            &mut s,
            ConstraintKind::PointLineDistance { point: PointRef::new(a, 0), line: x, value: 99.0, label: P2::ZERO },
        ),
        4.0,
    ));
    cases.push((reference(&mut s, ConstraintKind::Angle { a: x, b: y, value: 99.0, label: P2::ZERO }), 135.0));
    for (entity, diameter, expected) in
        [(circle, false, 5.0), (circle, true, 10.0), (arc, false, 3.0), (arc, true, 6.0)]
    {
        cases.push((
            reference(&mut s, ConstraintKind::Radius { entity, diameter, value: 99.0, label: P2::ZERO }),
            expected,
        ));
    }
    let before = s.clone();
    for (id, expected) in cases {
        assert!((value(&s, id) - expected).abs() < 1e-9);
        assert_eq!(s.constraint(id).unwrap().value(), None);
        let mut driving = s.clone();
        driving.set_dimension_reference(id, false).unwrap();
        assert!((driving.constraint(id).unwrap().value().unwrap() - expected).abs() < 1e-9);
        assert_eq!(driving.entities, s.entities);
    }
    assert_eq!(s, before);
    s.entity_mut(a).unwrap().kind = EntityKind::Point { p: p(6.0, 8.0) };
    assert_eq!(value(&s, 1), 10.0);
    assert_eq!(value(&s, 4), 8.0);
}

#[test]
fn angle_mode_changes_preserve_geometry_in_every_quadrant() {
    for degrees in [45.0_f64, 135.0, 225.0, 315.0] {
        let mut s = Sketch::new();
        let a = s.add(EntityKind::Line { a: P2::ZERO, b: p(10.0, 0.0) }, false);
        let theta = degrees.to_radians();
        let b = s.add(EntityKind::Line { a: P2::ZERO, b: p(10.0 * theta.cos(), 10.0 * theta.sin()) }, false);
        let id = reference(&mut s, ConstraintKind::Angle { a, b, value: 0.0, label: p(3.0, 3.0) });
        let before = s.entities.clone();
        s.set_dimension_reference(id, false).unwrap();
        assert_eq!(s.entities, before, "{degrees} degree direction");
    }
}

#[test]
fn perpendicular_measurements_use_the_supporting_line_in_both_directions() {
    let mut s = Sketch::new();
    let line = s.add(EntityKind::Line { a: P2::ZERO, b: p(10.0, 0.0) }, false);
    let point = s.add(EntityKind::Point { p: p(20.0, 5.0) }, false);
    let c = Constraint {
        id: 1,
        kind: ConstraintKind::PointLineDistance { point: PointRef::new(point, 0), line, value: 99.0, label: P2::ZERO },
    };
    for (a, b) in [(P2::ZERO, p(10.0, 0.0)), (p(10.0, 0.0), P2::ZERO)] {
        s.entity_mut(line).unwrap().kind = EntityKind::Line { a, b };
        for at in [p(20.0, 5.0), p(-20.0, 5.0), p(5.0, 5.0)] {
            s.entity_mut(point).unwrap().kind = EntityKind::Point { p: at };
            assert_eq!(c.measured_value(&s), Some(5.0));
        }
    }
}

#[test]
fn mode_changes_preserve_identity_and_release_only_the_dimension_equation() {
    let mut s = Sketch::new();
    let circle = s.add(EntityKind::Circle { center: P2::ZERO, radius: 5.0 }, false);
    s.constrain(ConstraintKind::Coincident { a: PointRef::new(circle, 0), b: PointRef::new(0, 0) });
    let id = s.constrain(ConstraintKind::Radius { entity: circle, diameter: true, value: 10.0, label: p(9.0, 7.0) });
    solve::solve(&mut s, None, true);
    assert_eq!(s.status, SolveStatus::FullyDefined);
    let entities = s.entities.clone();
    s.set_dimension_reference(id, true).unwrap();
    assert_eq!(s.status, SolveStatus::UnderDefined);
    assert_eq!(s.entities, entities);
    assert_eq!(s.constraint(id).unwrap().label(), Some(p(9.0, 7.0)));
    assert_eq!(s.constraint(id).unwrap().name(), "Diameter");
    assert_eq!(s.constraint(id).unwrap().entities(), [circle]);
    s.entity_mut(circle).unwrap().kind = EntityKind::Circle { center: P2::ZERO, radius: 8.0 };
    solve::solve(&mut s, None, true);
    assert_eq!(value(&s, id), 16.0);
    s.set_dimension_reference(id, false).unwrap();
    assert_eq!(s.constraint(id).unwrap().value(), Some(16.0));
    assert_eq!(s.status, SolveStatus::FullyDefined);
}

#[test]
fn reference_creation_is_value_free_and_legacy_driving_json_is_unchanged() {
    let legacy =
        r#"{"id":9,"kind":{"type":"radius","entity":1,"value":10.0,"diameter":true,"label":{"u":9.0,"v":7.0}}}"#;
    let c: Constraint = serde_json::from_str(legacy).unwrap();
    assert_eq!(serde_json::to_value(c).unwrap(), serde_json::from_str::<serde_json::Value>(legacy).unwrap());
    let r = Constraint { id: c.id, kind: c.kind.as_reference().unwrap() };
    let json = serde_json::to_value(r).unwrap();
    assert_eq!(json["kind"]["type"], "reference");
    assert!(json["kind"].get("value").is_none());
    assert!(json["kind"]["dimension"].get("value").is_none());
    assert_eq!(serde_json::from_value::<Constraint>(json).unwrap(), r);
    assert!(ConstraintKind::Horizontal { target: soliddraft_sketch::Align::Line(1) }.as_reference().is_none());
    assert!(serde_json::from_str::<Constraint>(r#"{"id":1,"kind":{"type":"reference","dimension":{"type":"horizontal","target":{"line":1}},"label":{"u":0,"v":0}}}"#).is_err());
}

#[test]
fn zero_and_unavailable_measurements_refuse_driving_atomically() {
    let mut s = Sketch::new();
    let a = s.add(EntityKind::Point { p: P2::ZERO }, false);
    let zero = reference(
        &mut s,
        ConstraintKind::Distance {
            a: PointRef::new(a, 0),
            b: PointRef::new(0, 0),
            kind: DistanceKind::Aligned,
            value: 1.0,
            label: P2::ZERO,
        },
    );
    assert_eq!(value(&s, zero), 0.0);
    let before = s.clone();
    assert!(s.set_dimension_reference(zero, false).is_err());
    assert_eq!(s, before);
    let missing = s.constrain(ConstraintKind::Reference {
        dimension: ReferenceDimension::Radius { entity: 999, diameter: false },
        label: P2::ZERO,
    });
    assert_eq!(s.constraint(missing).unwrap().display_value(&s), None);
    let before = s.clone();
    assert!(s.set_dimension_reference(missing, false).is_err());
    assert_eq!(s, before);
    let wrong = s.constrain(ConstraintKind::Reference {
        dimension: ReferenceDimension::Radius { entity: a, diameter: false },
        label: P2::ZERO,
    });
    assert_eq!(s.constraint(wrong).unwrap().display_value(&s), None);
    let arc = s.add(EntityKind::Arc { center: P2::ZERO, start: p(5.0, 0.0), end: p(0.0, 10.0) }, false);
    let bad_arc =
        reference(&mut s, ConstraintKind::Radius { entity: arc, value: 5.0, diameter: false, label: P2::ZERO });
    assert_eq!(s.constraint(bad_arc).unwrap().display_value(&s), None);
    s.entity_mut(a).unwrap().kind = EntityKind::Point { p: p(f64::NAN, 0.0) };
    assert_eq!(s.constraint(zero).unwrap().display_value(&s), None);
}

#[test]
fn driving_conversion_refuses_existing_conflicts_and_diagnostics_checks_reference_targets() {
    let mut s = Sketch::new();
    let circle = s.add(EntityKind::Circle { center: P2::ZERO, radius: 5.0 }, false);
    s.constrain(ConstraintKind::Radius { entity: circle, value: 5.0, diameter: false, label: P2::ZERO });
    s.constrain(ConstraintKind::Radius { entity: circle, value: 10.0, diameter: false, label: P2::ZERO });
    solve::solve(&mut s, None, true);
    assert!(matches!(s.status, SolveStatus::OverDefined { .. }));
    let id =
        reference(&mut s, ConstraintKind::Radius { entity: circle, value: 50.0, diameter: true, label: p(10.0, 10.0) });
    let before = s.clone();
    assert!(s.set_dimension_reference(id, false).is_err());
    assert_eq!(s, before);
    let bad = reference(&mut s, ConstraintKind::Angle { a: circle, b: circle, value: 30.0, label: P2::ZERO });
    let issues = soliddraft_sketch::profile::diagnostics::inspect(&s);
    assert!(issues.iter().any(|i| i.relation == Some(bad)));
}
