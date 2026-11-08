use soliddraft_sketch::{Align, ConstraintKind, DistanceKind, EntityKind, P2, PointRef, Sketch, SolveStatus, solve};

fn constrain_axes(sketch: &mut Sketch, id: u32, a: f64, b: f64) {
    sketch.constrain(ConstraintKind::Fix { point: PointRef::new(id, 0), at: P2::ZERO });
    sketch.constrain(ConstraintKind::Horizontal { target: Align::Points(PointRef::new(id, 0), PointRef::new(id, 1)) });
    for (index, value) in [(1, a), (2, b)] {
        sketch.constrain(ConstraintKind::Distance {
            a: PointRef::new(id, 0),
            b: PointRef::new(id, index),
            kind: DistanceKind::Aligned,
            value,
            label: P2::ZERO,
        });
    }
}

#[test]
fn axes_are_solver_dimensions_and_the_full_ellipse_is_fully_defined() {
    let mut sketch = Sketch::new();
    let id =
        sketch.add(EntityKind::Ellipse { center: P2::ZERO, major: P2::new(4.0, 0.0), minor: P2::new(0.0, 2.0) }, false);
    constrain_axes(&mut sketch, id, 8.0, 3.0);
    solve::solve(&mut sketch, None, true);
    assert_eq!(sketch.status, SolveStatus::FullyDefined);
    let e = sketch.entity(id).unwrap();
    assert!((e.point(1).unwrap().dist(P2::ZERO) - 8.0).abs() < 1e-6);
    assert!((e.point(2).unwrap().dist(P2::ZERO) - 3.0).abs() < 1e-6);
    assert!((sketch.regions().unwrap()[0].area() - 24.0 * std::f64::consts::PI).abs() < 1e-5);
    let json = serde_json::to_string(&sketch).unwrap();
    let reopened: Sketch = serde_json::from_str(&json).unwrap();
    assert_eq!(reopened, sketch);
}

#[test]
fn partial_ellipse_endpoints_follow_axes_and_coincidence_closes_the_profile() {
    let mut sketch = Sketch::new();
    let id = sketch.add(
        EntityKind::EllipticArc {
            center: P2::ZERO,
            major: P2::new(4.0, 0.0),
            minor: P2::new(0.0, 2.0),
            start: P2::new(4.0, 0.0),
            end: P2::new(-4.0, 0.0),
        },
        false,
    );
    let line = sketch.add(EntityKind::Line { a: P2::new(-4.0, 0.0), b: P2::new(4.0, 0.0) }, false);
    for (arc_index, line_index) in [(3, 1), (4, 0)] {
        sketch.constrain(ConstraintKind::Coincident {
            a: PointRef::new(id, arc_index),
            b: PointRef::new(line, line_index),
        });
        sketch.constrain(ConstraintKind::Horizontal {
            target: Align::Points(PointRef::new(id, 0), PointRef::new(id, arc_index)),
        });
    }
    constrain_axes(&mut sketch, id, 8.0, 3.0);
    solve::solve(&mut sketch, None, true);
    assert!(!matches!(sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }), "{:?}", sketch.status);
    let regions = sketch.regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert!((regions[0].area() - 12.0 * std::f64::consts::PI).abs() < 1e-4, "{}", regions[0].area());
}

#[test]
fn point_on_ellipse_is_solved() {
    let mut sketch = Sketch::new();
    let id =
        sketch.add(EntityKind::Ellipse { center: P2::ZERO, major: P2::new(4.0, 0.0), minor: P2::new(0.0, 2.0) }, false);
    constrain_axes(&mut sketch, id, 4.0, 2.0);
    let p = sketch.add(EntityKind::Point { p: P2::new(3.0, 1.0) }, false);
    sketch.constrain(ConstraintKind::PointOn { point: PointRef::new(p, 0), entity: id });
    solve::solve(&mut sketch, None, true);
    assert!(!matches!(sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }), "{:?}", sketch.status);
    let p = sketch.point(PointRef::new(p, 0)).unwrap();
    assert!((p.u.powi(2) / 16.0 + p.v.powi(2) / 4.0 - 1.0).abs() < 1e-6);
}

#[test]
fn ellipse_trim_refuses_without_changing_the_sketch() {
    use soliddraft_sketch::tools::{Operation, TrimOptions};
    let mut sketch = Sketch::new();
    let id =
        sketch.add(EntityKind::Ellipse { center: P2::ZERO, major: P2::new(4.0, 0.0), minor: P2::new(0.0, 2.0) }, false);
    let before = sketch.clone();
    assert!(
        sketch
            .apply_operation(&Operation::Trim { entity: id, at: P2::new(0.0, 2.0), options: TrimOptions::default() })
            .is_err()
    );
    assert_eq!(sketch, before);
}
