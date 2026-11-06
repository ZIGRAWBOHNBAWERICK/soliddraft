//! Profile-warning locations, ownership and read-only analysis.

use soliddraft_sketch::{
    Align, ConstraintKind, EntityKind, P2, PointRef, Sketch,
    profile::{
        self,
        diagnostics::{DiagnosticKind, inspect},
    },
};

fn p(x: f64, y: f64) -> P2 {
    P2::new(x, y)
}
fn line(s: &mut Sketch, a: P2, b: P2) -> u32 {
    s.add(EntityKind::Line { a, b }, false)
}
fn rectangle() -> Sketch {
    let mut s = Sketch::new();
    for (a, b) in [
        (p(0.0, 0.0), p(10.0, 0.0)),
        (p(10.0, 0.0), p(10.0, 10.0)),
        (p(10.0, 10.0), p(0.0, 10.0)),
        (p(0.0, 10.0), p(0.0, 0.0)),
    ] {
        line(&mut s, a, b);
    }
    s
}

#[test]
fn valid_profiles_ignore_guides_and_inspection_changes_nothing() {
    let mut s = rectangle();
    s.add(EntityKind::Line { a: p(-5.0, 5.0), b: p(15.0, 5.0) }, true);
    s.add(EntityKind::Point { p: p(5.0, 5.0) }, false);
    let before = s.clone();
    let regions = profile::regions(&s).unwrap();
    assert!(inspect(&s).is_empty());
    assert_eq!(s, before);
    assert_eq!(profile::regions(&s).unwrap(), regions);
    assert!(inspect(&Sketch::new()).is_empty());
}

#[test]
fn every_open_end_has_an_owner_and_exact_plane_location() {
    let mut s = Sketch::new();
    let a = line(&mut s, p(0.0, 0.0), p(10.0, 0.0));
    let b = line(&mut s, p(10.0, 0.0), p(10.0, 10.0));
    let issues = inspect(&s);
    assert_eq!(issues.len(), 2);
    assert!(issues.iter().all(|i| i.kind == DiagnosticKind::OpenEnd));
    assert_eq!(issues[0].entities, [a]);
    assert_eq!(issues[0].locations, [p(0.0, 0.0)]);
    assert_eq!(issues[1].entities, [b]);
    assert_eq!(issues[1].locations, [p(10.0, 10.0)]);
}

#[test]
fn coincidence_and_profile_tolerance_join_ends_in_the_same_way() {
    for gap in [0.5e-6, 0.01] {
        let mut s = rectangle();
        let last = s.drawn().last().unwrap().id;
        s.entity_mut(last).unwrap().set_point(1, p(gap, 0.0));
        if gap > 1e-6 {
            s.constrain(ConstraintKind::Coincident { a: PointRef::new(1, 0), b: PointRef::new(last, 1) });
        }
        let before = s.clone();
        assert!(inspect(&s).is_empty());
        assert_eq!(s, before);
        assert_eq!(profile::regions(&s).unwrap().len(), 1);
    }
}

#[test]
fn endpoint_and_interior_branches_are_found_without_duplicate_markers() {
    let mut s = Sketch::new();
    let ids: Vec<_> =
        [p(-10.0, 0.0), p(10.0, 0.0), p(0.0, 10.0)].into_iter().map(|q| line(&mut s, P2::ZERO, q)).collect();
    let issues = inspect(&s);
    let branches: Vec<_> = issues.iter().filter(|i| i.kind == DiagnosticKind::Branch).collect();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0].entities, ids);
    assert_eq!(branches[0].locations, [P2::ZERO]);
    for diagonal in [false, true] {
        let mut s = Sketch::new();
        line(&mut s, p(-10.0, 0.0), p(10.0, 0.0));
        line(&mut s, if diagonal { p(0.0, -10.0) } else { P2::ZERO }, p(0.0, 10.0));
        let issues = inspect(&s);
        assert_eq!(issues.iter().filter(|i| i.kind == DiagnosticKind::Branch).count(), 1);
    }
}

#[test]
fn line_overlap_is_direction_independent_and_endpoint_contact_is_not_overlap() {
    for reverse in [false, true] {
        let mut s = Sketch::new();
        let a = line(&mut s, p(0.0, 0.0), p(10.0, 0.0));
        let (start, end) = if reverse { (p(15.0, 0.0), p(5.0, 0.0)) } else { (p(5.0, 0.0), p(15.0, 0.0)) };
        let b = line(&mut s, start, end);
        let issues = inspect(&s);
        let overlap = issues.iter().find(|i| i.kind == DiagnosticKind::Overlap).unwrap();
        assert_eq!(overlap.entities, [a, b]);
        assert_eq!(overlap.locations, [p(5.0, 0.0), p(10.0, 0.0)]);
    }
    let mut s = Sketch::new();
    line(&mut s, p(0.0, 0.0), p(10.0, 0.0));
    line(&mut s, p(10.0, 0.0), p(20.0, 0.0));
    assert!(!inspect(&s).iter().any(|i| i.kind == DiagnosticKind::Overlap));
}

#[test]
fn circular_overlap_keeps_the_angle_seam_out_of_the_result() {
    let mut s = Sketch::new();
    let at = |degrees: f64| P2::from_angle(degrees.to_radians()).scale(10.0);
    let a = s.add(EntityKind::Arc { center: P2::ZERO, start: at(330.0), end: at(90.0) }, false);
    let b = s.add(EntityKind::Arc { center: P2::ZERO, start: at(300.0), end: at(30.0) }, false);
    let issues = inspect(&s);
    let overlap = issues.iter().find(|i| i.kind == DiagnosticKind::Overlap).unwrap();
    assert_eq!(overlap.entities, [a, b]);
    assert!(overlap.locations.iter().all(|q| (q.len() - 10.0).abs() < 1e-6 && q.u >= 8.6));
    s.add(EntityKind::Circle { center: P2::ZERO, radius: 10.0 }, false);
    assert_eq!(inspect(&s).iter().filter(|i| i.kind == DiagnosticKind::Overlap).count(), 3);
}

#[test]
fn disjoint_circles_and_tangent_contacts_are_not_coincident_overlaps() {
    for x in [20.0, 30.0] {
        let mut s = Sketch::new();
        s.add(EntityKind::Circle { center: P2::ZERO, radius: 10.0 }, false);
        s.add(EntityKind::Circle { center: p(x, 0.0), radius: 10.0 }, false);
        assert!(!inspect(&s).iter().any(|i| i.kind == DiagnosticKind::Overlap));
    }
}

#[test]
fn wrong_kind_relation_targets_remain_authored_and_have_explicit_diagnostics() {
    let mut s = Sketch::new();
    let circle = s.add(EntityKind::Circle { center: p(10.0, 10.0), radius: 10.0 }, false);
    let line = s.add(EntityKind::Line { a: p(-5.0, 10.0), b: p(25.0, 10.0) }, true);
    let horizontal = s.constrain(ConstraintKind::Horizontal { target: Align::Line(circle) });
    let radius = s.constrain(ConstraintKind::Radius { entity: line, value: 3.0, diameter: false, label: p(0.0, 15.0) });
    let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    reopened.normalise();
    let before = reopened.clone();
    let issues = inspect(&reopened);
    assert_eq!(issues.len(), 2);
    assert!(issues.iter().all(|i| i.kind == DiagnosticKind::InvalidRelation));
    assert_eq!(issues.iter().filter_map(|i| i.relation).collect::<Vec<_>>(), [horizontal, radius]);
    assert!(issues.iter().all(|i| i.message.is_some()));
    assert_eq!(reopened, before);
}

#[test]
fn relation_reference_checks_follow_the_solver_target_families() {
    let mut base = Sketch::new();
    let a = line(&mut base, p(0.0, 0.0), p(10.0, 0.0));
    let b = line(&mut base, p(0.0, 10.0), p(10.0, 10.0));
    let circle = base.add(EntityKind::Circle { center: p(100.0, 0.0), radius: 5.0 }, false);
    let arc = base.add(EntityKind::Arc { center: p(200.0, 0.0), start: p(205.0, 0.0), end: p(200.0, 5.0) }, false);
    let point = base.add(EntityKind::Point { p: p(50.0, 50.0) }, false);
    let pr = PointRef::new(point, 0);
    let invalid = [
        ConstraintKind::Horizontal { target: Align::Line(circle) },
        ConstraintKind::Vertical { target: Align::Line(arc) },
        ConstraintKind::Parallel { a, b: circle },
        ConstraintKind::Perpendicular { a: point, b },
        ConstraintKind::Angle { a: arc, b, value: 30.0, label: P2::ZERO },
        ConstraintKind::Midpoint { point: pr, line: circle },
        ConstraintKind::PointLineDistance { point: pr, line: circle, value: 2.0, label: P2::ZERO },
        ConstraintKind::Radius { entity: a, value: 2.0, diameter: false, label: P2::ZERO },
        ConstraintKind::Concentric { a: point, b: circle },
        ConstraintKind::Equal { a, b: circle },
        ConstraintKind::Tangent { a, b },
    ];
    for kind in invalid {
        let mut s = base.clone();
        let id = s.constrain(kind);
        assert!(inspect(&s).iter().any(|i| i.relation == Some(id)), "{kind:?}");
    }
    let valid = [
        ConstraintKind::Horizontal { target: Align::Line(a) },
        ConstraintKind::Vertical { target: Align::Points(pr, PointRef::new(a, 0)) },
        ConstraintKind::Parallel { a, b },
        ConstraintKind::Perpendicular { a, b },
        ConstraintKind::Angle { a, b, value: 30.0, label: P2::ZERO },
        ConstraintKind::Midpoint { point: pr, line: a },
        ConstraintKind::PointLineDistance { point: pr, line: a, value: 2.0, label: P2::ZERO },
        ConstraintKind::Radius { entity: arc, value: 5.0, diameter: false, label: P2::ZERO },
        ConstraintKind::Concentric { a: arc, b: circle },
        ConstraintKind::Equal { a, b },
        ConstraintKind::Equal { a: arc, b: circle },
        ConstraintKind::Tangent { a, b: circle },
        ConstraintKind::Tangent { a: circle, b: arc },
        ConstraintKind::PointOn { point: PointRef::new(a, 0), entity: point },
    ];
    for kind in valid {
        let mut s = base.clone();
        s.constrain(kind);
        assert!(!inspect(&s).iter().any(|i| i.kind == DiagnosticKind::InvalidRelation), "{kind:?}");
    }
}

#[test]
fn missing_entities_and_point_indices_are_invalid_references_not_satisfied_rows() {
    let mut s = Sketch::new();
    let a = line(&mut s, p(0.0, 0.0), p(10.0, 0.0));
    let missing = s.constrain(ConstraintKind::Horizontal { target: Align::Line(999) });
    let point = s.constrain(ConstraintKind::Coincident { a: PointRef::new(a, 9), b: PointRef::new(0, 0) });
    let issues = inspect(&s);
    let refs: Vec<_> = issues.iter().filter(|i| i.kind == DiagnosticKind::InvalidRelation).collect();
    assert_eq!(refs.len(), 2);
    assert!(refs.iter().any(|i| i.relation == Some(missing) && i.message.as_deref() == Some("Missing entity 999")));
    assert!(refs.iter().any(|i| i.relation == Some(point) && i.message.as_deref() == Some("Missing point 1:9")));
}

#[test]
fn invalid_curves_are_reported_without_nonfinite_marker_coordinates() {
    let mut s = Sketch::new();
    line(&mut s, P2::ZERO, P2::ZERO);
    s.add(EntityKind::Circle { center: p(f64::NAN, 0.0), radius: 2.0 }, false);
    s.add(EntityKind::Circle { center: P2::ZERO, radius: -2.0 }, false);
    s.add(EntityKind::Arc { center: P2::ZERO, start: p(2.0, 0.0), end: p(0.0, 3.0) }, false);
    let issues = inspect(&s);
    assert_eq!(issues.len(), 4);
    assert!(issues.iter().all(|i| i.kind == DiagnosticKind::Degenerate));
    assert!(issues.iter().flat_map(|i| &i.locations).all(|p| p.u.is_finite() && p.v.is_finite()));
}
