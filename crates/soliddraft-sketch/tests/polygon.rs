//! Retained polygons, solver behavior and atomic parameter boundaries.

use soliddraft_sketch::{
    ConstraintKind, EntityKind, P2, PointRef, Sketch, SolveStatus, profile, solve,
    tools::{Operation, PolygonMode, regular_polygon_corners},
};

fn create(s: &mut Sketch, sides: u32, mode: PolygonMode) -> Vec<u32> {
    s.apply_operation(&Operation::RegularPolygon { center: P2::ZERO, point: P2::new(20.0, 0.0), sides, mode }).unwrap()
}

fn check(s: &Sketch, circle: u32, sides: usize, radius: f64, mode: PolygonMode) {
    let group = s.polygon_of(circle).unwrap();
    assert_eq!(group.edges.len(), sides);
    assert_eq!(group.mode, mode);
    let EntityKind::Circle { center, radius: actual } = s.entity(circle).unwrap().kind else { panic!() };
    assert!((actual - radius).abs() < 1e-5);
    assert!(s.entity(circle).unwrap().construction);
    let circumradius =
        radius / if mode == PolygonMode::Inscribed { 1.0 } else { (std::f64::consts::PI / sides as f64).cos() };
    let length = 2.0 * circumradius * (std::f64::consts::PI / sides as f64).sin();
    for (i, id) in group.edges.iter().enumerate() {
        let EntityKind::Line { a, b } = s.entity(*id).unwrap().kind else { panic!() };
        assert!((a.dist(b) - length).abs() < 1e-5, "side {i}: {a:?}, {b:?}");
        assert!((center.dist(a) - circumradius).abs() < 1e-5);
        assert!(b.dist(s.point(PointRef::new(group.edges[(i + 1) % sides], 0)).unwrap()) < 1e-5);
    }
    assert!(!matches!(s.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }), "{:?}", s.status);
}

#[test]
fn modes_share_preview_geometry_and_keep_the_guide_out_of_profiles() {
    for mode in [PolygonMode::Inscribed, PolygonMode::Circumscribed] {
        for n in [3, 4, 7, 1000] {
            let mut s = Sketch::new();
            let ids = create(&mut s, n, mode);
            let circle = *ids.last().unwrap();
            let preview = regular_polygon_corners(P2::ZERO, P2::new(20.0, 0.0), n, mode).unwrap();
            for (id, point) in ids.iter().zip(preview) {
                assert_eq!(s.entity(*id).unwrap().point(0), Some(point));
            }
            check(&s, circle, n as usize, 20.0, mode);
            let regions = profile::regions(&s).unwrap();
            assert_eq!(regions.len(), 1);
            assert!(regions[0].holes.is_empty(), "the construction circle must not become a hole");
            let expected = if mode == PolygonMode::Inscribed {
                0.5 * n as f64 * 400.0 * (std::f64::consts::TAU / n as f64).sin()
            } else {
                n as f64 * 400.0 * (std::f64::consts::PI / n as f64).tan()
            };
            assert!((regions[0].area() - expected).abs() < 1e-6);
        }
    }
    let square = regular_polygon_corners(P2::ZERO, P2::new(10.0, 0.0), 4, PolygonMode::Inscribed).unwrap();
    assert_eq!(square, vec![P2::new(10.0, 0.0), P2::new(0.0, 10.0), P2::new(-10.0, 0.0), P2::new(0.0, -10.0)]);
    let square = regular_polygon_corners(P2::ZERO, P2::new(10.0, 0.0), 4, PolygonMode::Circumscribed).unwrap();
    assert!((square[0].u - 10.0).abs() < 1e-12 && (square[0].v + 10.0).abs() < 1e-12);
}

#[test]
fn driving_radius_changes_keep_both_modes_regular_after_reopen() {
    for mode in [PolygonMode::Inscribed, PolygonMode::Circumscribed] {
        let mut s = Sketch::new();
        let ids = create(&mut s, 5, mode);
        let circle = *ids.last().unwrap();
        s.constrain(ConstraintKind::Coincident { a: PointRef::new(circle, 0), b: PointRef::ORIGIN });
        let dimension = s.constrain(ConstraintKind::Radius {
            entity: circle,
            value: 20.0,
            diameter: false,
            label: P2::new(30.0, 30.0),
        });
        solve::solve(&mut s, None, true);
        check(&s, circle, 5, 20.0, mode);
        s.constraint_mut(dimension).unwrap().kind =
            ConstraintKind::Radius { entity: circle, value: 24.0, diameter: false, label: P2::new(30.0, 30.0) };
        solve::solve(&mut s, None, true);
        check(&s, circle, 5, 24.0, mode);
        let mut reopened: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        reopened.normalise();
        solve::solve(&mut reopened, None, true);
        check(&reopened, circle, 5, 24.0, mode);
    }
}

#[test]
fn parameters_preserve_group_identity_centre_snaps_and_construction() {
    let mut s = Sketch::new();
    let ids = create(&mut s, 4, PolygonMode::Inscribed);
    let circle = *ids.last().unwrap();
    let snap = s.constrain(ConstraintKind::Coincident { a: PointRef::new(circle, 0), b: PointRef::ORIGIN });
    for id in &ids {
        s.entity_mut(*id).unwrap().construction = true;
    }
    for (n, radius, mode) in [(8, 10.0, PolygonMode::Circumscribed), (3, 30.0, PolygonMode::Inscribed)] {
        s.apply_operation(&Operation::EditPolygon { circle, sides: n, radius, mode }).unwrap();
        solve::solve(&mut s, None, true);
        check(&s, circle, n as usize, radius, mode);
        assert_eq!(&s.polygon_of(circle).unwrap().edges[..3], &ids[..3]);
        assert!(s.constraint(snap).is_some());
        assert!(s.drawn().all(|e| e.construction));
        assert!(profile::regions(&s).is_err());
    }
}

#[test]
fn invalid_inputs_and_external_edge_relations_refuse_without_mutation() {
    let mut s = Sketch::new();
    for (point, sides) in [
        (P2::ZERO, 4),
        (P2::new(f64::INFINITY, 0.0), 4),
        (P2::new(1.0, 0.0), 2),
        (P2::new(1.0, 0.0), 1001),
        (P2::new(1e-5, 0.0), 1000),
        (P2::new(f64::MAX, f64::MAX), 4),
    ] {
        let before = s.clone();
        assert!(
            s.apply_operation(&Operation::RegularPolygon {
                center: P2::ZERO,
                point,
                sides,
                mode: PolygonMode::Inscribed
            })
            .is_err()
        );
        assert_eq!(s, before);
    }
    let ids = create(&mut s, 4, PolygonMode::Inscribed);
    let circle = *ids.last().unwrap();
    for radius in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e-8] {
        let before = s.clone();
        assert!(
            s.apply_operation(&Operation::EditPolygon { circle, sides: 5, radius, mode: PolygonMode::Inscribed })
                .is_err()
        );
        assert_eq!(s, before);
    }
    let external = s.constrain(ConstraintKind::Fix { point: PointRef::new(ids[0], 0), at: P2::new(20.0, 0.0) });
    let before = s.clone();
    assert!(
        s.apply_operation(&Operation::EditPolygon { circle, sides: 5, radius: 30.0, mode: PolygonMode::Circumscribed })
            .is_err()
    );
    assert_eq!(s, before);
    s.remove_constraint(external);
    let defining = s.polygon_of(circle).unwrap().relations[0];
    s.remove_constraint(defining);
    assert!(s.polygon_of(circle).is_none(), "removing a defining relation releases retained editing");
    let before = s.clone();
    assert!(
        s.apply_operation(&Operation::EditPolygon { circle, sides: 5, radius: 30.0, mode: PolygonMode::Inscribed })
            .is_err()
    );
    assert_eq!(s, before);
}

#[test]
fn old_sketch_json_has_no_polygon_groups() {
    let mut json = serde_json::to_value(Sketch::new()).unwrap();
    json.as_object_mut().unwrap().remove("polygons");
    let old: Sketch = serde_json::from_value(json).unwrap();
    assert!(old.polygons.is_empty());
}

#[test]
fn guide_drag_and_driving_diameter_keep_the_polygon_regular() {
    for mode in [PolygonMode::Inscribed, PolygonMode::Circumscribed] {
        let mut s = Sketch::new();
        let ids = create(&mut s, 6, mode);
        let circle = *ids.last().unwrap();
        s.constrain(ConstraintKind::Radius { entity: circle, value: 40.0, diameter: true, label: P2::new(30.0, 30.0) });
        solve::solve(&mut s, Some(solve::Drag { point: PointRef::new(circle, 0), to: P2::new(10.0, 12.0) }), true);
        assert!(s.point(PointRef::new(circle, 0)).unwrap().dist(P2::new(10.0, 12.0)) < 1e-5);
        check(&s, circle, 6, 20.0, mode);
        // Side count and mode edits preserve an unchanged guide dimension.
        s.apply_operation(&Operation::EditPolygon { circle, sides: 4, radius: 20.0, mode }).unwrap();
        solve::solve(&mut s, None, true);
        check(&s, circle, 4, 20.0, mode);
        let before = s.clone();
        assert!(s.apply_operation(&Operation::EditPolygon { circle, sides: 4, radius: 25.0, mode }).is_err());
        assert_eq!(s, before);
        s.remove_entity(ids[0]);
        assert!(s.polygon_of(circle).is_none());
    }
}
