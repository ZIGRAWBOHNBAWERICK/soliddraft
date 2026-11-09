//! Analytic snap candidates retain their intended relations after placement and solving.

use soliddraft_sketch::{
    ConstraintKind, EntityKind, P2, PointRef, Sketch,
    inference::{InferenceOptions, Snap, SnapFilter, apply_snaps, infer},
    solve,
    tools::Operation,
};

fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn line(s: &mut Sketch, a: P2, b: P2) -> u32 {
    s.add(EntityKind::Line { a, b }, true)
}
fn preview(s: &Sketch, at: P2, start: Option<P2>, filter: SnapFilter) -> (P2, Vec<Snap>) {
    infer(s, at, start, InferenceOptions::uniform(0.2, filter), P2::dist)
}

#[test]
fn bounded_crossings_are_nearest_and_keep_both_owners() {
    let mut s = Sketch::new();
    let a = line(&mut s, p(1.0, 1.0), p(9.0, 9.0));
    let b = line(&mut s, p(1.0, 9.0), p(9.0, 1.0));
    let before = s.clone();
    let (at, snaps) = preview(&s, p(5.08, 5.09), None, SnapFilter::Intersection);
    assert_eq!(s, before);
    assert_eq!(at, p(5.0, 5.0));
    assert_eq!(snaps, vec![Snap::Intersection(a, b)]);
    let id = s.add(EntityKind::Point { p: at }, false);
    apply_snaps(&mut s, PointRef::new(id, 0), &snaps, None);
    assert_eq!(s.constraints.len(), 2);
    solve::solve(&mut s, None, true);
    assert!(s.point(PointRef::new(id, 0)).unwrap().dist(at) < 1e-6);
    assert!(
        matches!(s.status, soliddraft_sketch::SolveStatus::UnderDefined | soliddraft_sketch::SolveStatus::FullyDefined),
        "{:?}",
        s.status
    );
    let mut s = Sketch::new();
    line(&mut s, p(1.0, 1.0), p(2.0, 2.0));
    line(&mut s, p(1.0, 9.0), p(9.0, 1.0));
    assert!(preview(&s, at, None, SnapFilter::Intersection).1.is_empty());
}

#[test]
fn circle_and_wrapping_arc_crossings_use_the_analytic_bounds() {
    let mut s = Sketch::new();
    let circle = s.add(EntityKind::Circle { center: p(5.0, 5.0), radius: 3.0 }, true);
    let axis = line(&mut s, p(0.0, 5.0), p(10.0, 5.0));
    assert_eq!(
        preview(&s, p(8.05, 5.05), None, SnapFilter::Intersection),
        (p(8.0, 5.0), vec![Snap::Intersection(circle, axis)])
    );
    s.entity_mut(circle).unwrap().kind = EntityKind::Arc { center: p(5.0, 5.0), start: p(5.0, 2.0), end: p(5.0, 8.0) };
    assert!(preview(&s, p(2.0, 5.0), None, SnapFilter::Intersection).1.is_empty());
    assert!(!preview(&s, p(8.0, 5.0), None, SnapFilter::Intersection).1.is_empty());
    let other = s.add(EntityKind::Circle { center: p(11.0, 5.0), radius: 3.0 }, true);
    s.entities.retain(|e| e.id != axis);
    assert_eq!(preview(&s, p(8.0, 5.0), None, SnapFilter::Intersection).1, vec![Snap::Intersection(circle, other)]);
    s.entity_mut(other).unwrap().kind = EntityKind::Circle { center: p(8.0, 5.0), radius: 3.0 };
    let upper = p(6.5, 5.0 + 6.75f64.sqrt());
    let (at, snaps) = preview(&s, upper + p(0.03, 0.04), None, SnapFilter::Intersection);
    assert!(at.dist(upper) < 1e-9);
    assert_eq!(snaps, vec![Snap::Intersection(circle, other)]);
    s.entity_mut(other).unwrap().kind = s.entity(circle).unwrap().kind.clone();
    assert!(preview(&s, p(8.0, 5.0), None, SnapFilter::Intersection).1.is_empty());
}

#[test]
fn parallel_and_perpendicular_previews_persist_and_solve() {
    for (filter, raw, expected) in
        [(SnapFilter::Parallel, p(8.0, 6.05), p(8.02, 6.01)), (SnapFilter::Perpendicular, p(2.05, 8.0), p(2.01, 7.98))]
    {
        let mut s = Sketch::new();
        let guide = line(&mut s, p(-10.0, -10.0), p(0.0, -5.0));
        let start = p(4.0, 4.0);
        let (end, snaps) = preview(&s, raw, Some(start), filter);
        assert_eq!(preview(&s, raw, Some(start), SnapFilter::All), (end, snaps.clone()));
        assert!(end.dist(expected) < 1e-9, "{end:?}");
        assert_eq!(
            snaps,
            vec![if filter == SnapFilter::Parallel { Snap::Parallel(guide) } else { Snap::Perpendicular(guide) }]
        );
        let id = s.add(EntityKind::Line { a: start, b: end }, false);
        apply_snaps(&mut s, PointRef::new(id, 1), &snaps, Some(id));
        solve::solve(&mut s, None, true);
        assert!(
            matches!(
                s.status,
                soliddraft_sketch::SolveStatus::UnderDefined | soliddraft_sketch::SolveStatus::FullyDefined
            ),
            "{:?}",
            s.status
        );
        let EntityKind::Line { a, b } = s.entity(id).unwrap().kind else { panic!() };
        let EntityKind::Line { a: c, b: d } = s.entity(guide).unwrap().kind else { panic!() };
        let residual = if filter == SnapFilter::Parallel { (b - a).cross(d - c) } else { (b - a).dot(d - c) };
        assert!(residual.abs() < 1e-5);
    }
}

#[test]
fn tangent_inference_covers_circle_contact_and_external_tangents() {
    let mut s = Sketch::new();
    let id = s.add(EntityKind::Circle { center: p(4.0, 4.0), radius: 3.0 }, true);
    assert_eq!(
        preview(&s, p(7.08, 10.0), Some(p(7.0, 4.0)), SnapFilter::Tangent),
        (p(7.0, 10.0), vec![Snap::Tangent(id)])
    );
    // The external tangent from (9,4) touches the circle at (5.8,6.4).
    let start = p(9.0, 4.0);
    let (end, snaps) = preview(&s, p(5.82, 6.42), Some(start), SnapFilter::Tangent);
    assert_eq!(snaps, vec![Snap::Tangent(id)]);
    assert_eq!(preview(&s, p(5.82, 6.42), Some(start), SnapFilter::All), (end, snaps.clone()));
    assert!(((p(4.0, 4.0) - start).cross(end - start).abs() / (end - start).len() - 3.0).abs() < 1e-9);
    let line = s.add(EntityKind::Line { a: start, b: end }, false);
    apply_snaps(&mut s, PointRef::new(line, 1), &snaps, Some(line));
    solve::solve(&mut s, None, true);
    assert!(
        matches!(s.status, soliddraft_sketch::SolveStatus::UnderDefined | soliddraft_sketch::SolveStatus::FullyDefined),
        "{:?}",
        s.status
    );
    assert!(preview(&s, p(5.0, 10.0), Some(p(4.0, 4.0)), SnapFilter::Tangent).1.is_empty());
    s.entity_mut(id).unwrap().kind = EntityKind::Arc { center: p(4.0, 4.0), start: p(1.0, 4.0), end: p(4.0, 1.0) };
    assert!(preview(&s, p(5.82, 6.42), Some(start), SnapFilter::Tangent).1.is_empty());
}

#[test]
fn filters_exclude_other_classes_and_non_line_tools_exclude_directions() {
    let mut s = Sketch::new();
    let id = line(&mut s, p(1.0, 1.0), p(9.0, 5.0));
    assert_eq!(preview(&s, p(5.01, 3.01), None, SnapFilter::All).1, vec![Snap::Midpoint(id)]);
    assert_eq!(preview(&s, p(0.01, 0.01), None, SnapFilter::All).1, vec![Snap::Point(PointRef::new(0, 0))]);
    assert_eq!(preview(&s, p(1.01, 1.01), None, SnapFilter::All).1, vec![Snap::Point(PointRef::new(id, 0))]);
    assert_eq!(preview(&s, p(15.0, 20.05), Some(p(10.0, 20.0)), SnapFilter::All).1, vec![Snap::Horizontal]);
    assert_eq!(preview(&s, p(5.01, 3.01), None, SnapFilter::On).1, vec![Snap::On(id)]);
    assert!(preview(&s, p(5.01, 3.01), None, SnapFilter::None).1.is_empty());
    let options = InferenceOptions { directions: false, ..InferenceOptions::uniform(0.2, SnapFilter::Parallel) };
    assert!(infer(&s, p(8.0, 6.05), Some(p(4.0, 4.0)), options, P2::dist).1.is_empty());
    assert!(preview(&s, p(4.05, 4.01), Some(p(4.0, 4.0)), SnapFilter::Parallel).1.is_empty());
}

#[test]
fn snapped_line_is_atomic_serializable_and_uses_preview_relations() {
    let mut s = Sketch::new();
    line(&mut s, p(1.0, 1.0), p(9.0, 9.0));
    line(&mut s, p(1.0, 9.0), p(9.0, 1.0));
    let operation = Operation::SnappedLine {
        start: p(5.08, 5.09),
        end: p(10.0, 5.05),
        reach: 0.2,
        start_filter: SnapFilter::Intersection,
        end_filter: SnapFilter::Axes,
        construction: false,
    };
    let encoded = serde_json::to_string(&operation).unwrap();
    assert_eq!(serde_json::from_str::<Operation>(&encoded).unwrap(), operation);
    let ids = s.apply_operation(&operation).unwrap();
    assert_eq!(s.entity(ids[0]).unwrap().kind, EntityKind::Line { a: p(5.0, 5.0), b: p(10.0, 5.0) });
    assert!(s.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Horizontal { .. })));
    let before = s.clone();
    let bad = Operation::SnappedLine {
        start: p(5.0, 5.0),
        end: p(5.0, 5.0),
        reach: 0.2,
        start_filter: SnapFilter::All,
        end_filter: SnapFilter::All,
        construction: false,
    };
    assert!(s.apply_operation(&bad).is_err());
    assert_eq!(s, before);
    let mut bad = operation;
    if let Operation::SnappedLine { reach, .. } = &mut bad {
        *reach = f64::NAN;
    }
    assert!(s.apply_operation(&bad).is_err());
    assert_eq!(s, before);
}
