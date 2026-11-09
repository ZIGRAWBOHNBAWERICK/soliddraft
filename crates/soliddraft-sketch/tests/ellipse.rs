//! Solver dimensions, endpoints, persistence and refusal checks for ellipse sketches.

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
    let mut reopened: Sketch = serde_json::from_str(&json).unwrap();
    assert_eq!(reopened.entities, sketch.entities);
    assert_eq!(reopened.constraints, sketch.constraints);
    solve::solve(&mut reopened, None, true);
    assert_eq!(reopened.status, SolveStatus::FullyDefined);
}

#[test]
fn solving_preserves_a_fixed_minor_handle_on_either_side() {
    for v in [-2.0, 2.0] {
        let mut sketch = Sketch::new();
        let id = sketch
            .add(EntityKind::Ellipse { center: P2::ZERO, major: P2::new(4.0, 0.0), minor: P2::new(0.0, v) }, false);
        for (index, at) in [(0, P2::ZERO), (1, P2::new(4.0, 0.0)), (2, P2::new(0.0, v))] {
            sketch.constrain(ConstraintKind::Fix { point: PointRef::new(id, index), at });
        }
        solve::solve(&mut sketch, None, true);
        assert_eq!(sketch.status, SolveStatus::FullyDefined);
        assert!(sketch.point(PointRef::new(id, 2)).unwrap().dist(P2::new(0.0, v)) < 1e-6);
    }
}

#[test]
fn partial_ellipse_endpoints_follow_axes_and_coincidence_closes_the_profile() {
    for (first, second) in [(8.0, 3.0), (3.0, 8.0), (4.0, 4.0)] {
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
        constrain_axes(&mut sketch, id, first, second);
        solve::solve(&mut sketch, None, true);
        assert!(
            !matches!(sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }),
            "{:?}",
            sketch.status
        );
        let regions = sketch.regions().unwrap();
        assert_eq!(regions.len(), 1);
        assert!(
            (regions[0].area() - 0.5 * first * second * std::f64::consts::PI).abs() < 1e-4,
            "{}",
            regions[0].area()
        );
    }
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

#[test]
fn equal_and_concentric_ellipses_share_axes_and_center() {
    let mut sketch = Sketch::new();
    let a =
        sketch.add(EntityKind::Ellipse { center: P2::ZERO, major: P2::new(4.0, 0.0), minor: P2::new(0.0, 2.0) }, false);
    let b = sketch.add(
        EntityKind::Ellipse { center: P2::new(10.0, 10.0), major: P2::new(16.0, 10.0), minor: P2::new(10.0, 13.0) },
        true,
    );
    constrain_axes(&mut sketch, a, 4.0, 2.0);
    sketch.constrain(ConstraintKind::Equal { a, b });
    sketch.constrain(ConstraintKind::Concentric { a, b });
    solve::solve(&mut sketch, None, true);
    assert!(!matches!(sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }), "{:?}", sketch.status);
    let e = sketch.entity(b).unwrap();
    let c = e.point(0).unwrap();
    assert!(c.dist(P2::ZERO) < 1e-6);
    assert!((c.dist(e.point(1).unwrap()) - 4.0).abs() < 1e-6);
    assert!((c.dist(e.point(2).unwrap()) - 2.0).abs() < 1e-6);
}

fn check_copied_profile_joins(elliptic: bool) {
    use soliddraft_sketch::tools::Operation;
    for tool in 0..3 {
        let mut sketch = Sketch::new();
        let start = P2::new(4.0, 0.0);
        let end = P2::new(-4.0, 0.0);
        let (kind, indices) = if elliptic {
            (EntityKind::EllipticArc { center: P2::ZERO, major: start, minor: P2::new(0.0, 2.0), start, end }, [3, 4])
        } else {
            (EntityKind::Arc { center: P2::ZERO, start, end }, [1, 2])
        };
        let curve = sketch.add(kind, false);
        let line = sketch.add(EntityKind::Line { a: end, b: start }, false);
        for (a, b) in [(indices[0], 1), (indices[1], 0)] {
            sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(curve, a), b: PointRef::new(line, b) });
        }
        let entities = vec![curve, line];
        let operation = match tool {
            0 => Operation::Mirror { entities, axis_start: P2::new(10.0, 0.0), axis_end: P2::new(10.0, 1.0) },
            1 => Operation::LinearPattern { entities, step: P2::new(20.0, 0.0), count: 3 },
            _ => Operation::CircularPattern { entities, center: P2::new(10.0, 0.0), angle_degrees: 90.0, count: 3 },
        };
        let copies = sketch.apply_operation(&operation).unwrap();
        assert_eq!(copies.len(), if tool == 0 { 2 } else { 4 });
        let regions = 1 + copies.len() / 2;
        assert_eq!(sketch.regions().unwrap().len(), regions, "{operation:?}");
        for &[curve, line] in copies.as_chunks::<2>().0 {
            let expected: Vec<_> = indices
                .into_iter()
                .map(|index| {
                    let a = PointRef::new(curve, index);
                    let b = (0..2)
                        .map(|i| PointRef::new(line, i))
                        .find(|b| sketch.point(a).unwrap().dist(sketch.point(*b).unwrap()) < 1e-8)
                        .unwrap();
                    (a, b)
                })
                .collect();
            let actual: Vec<_> = sketch
                .constraints
                .iter()
                .filter_map(|constraint| match constraint.kind {
                    ConstraintKind::Coincident { a, b }
                        if (a.entity == curve && b.entity == line) || (a.entity == line && b.entity == curve) =>
                    {
                        Some((a, b))
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(actual.len(), 2, "copied {operation:?}: {actual:?}");
            for &(a, b) in &expected {
                assert!(actual.contains(&(a, b)) || actual.contains(&(b, a)), "{operation:?}: missing {a:?} = {b:?}");
            }
            let moved = PointRef::new(line, 0);
            let before = sketch.point(moved).unwrap();
            solve::solve(&mut sketch, Some(solve::Drag { point: moved, to: before + P2::new(0.75, 0.5) }), true);
            assert!(
                !matches!(sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }),
                "{operation:?}: {:?}",
                sketch.status
            );
            assert!(sketch.point(moved).unwrap().dist(before) > 0.05, "{operation:?}: drag did not move its endpoint");
            for &(a, b) in &expected {
                assert!(
                    sketch.point(a).unwrap().dist(sketch.point(b).unwrap()) < 1e-6,
                    "{operation:?}: drag opened the join"
                );
            }
            assert_eq!(sketch.regions().unwrap().len(), regions, "{operation:?}: drag opened the profile");
            let dimension = if elliptic {
                ConstraintKind::Distance {
                    a: PointRef::new(curve, 0),
                    b: PointRef::new(curve, 1),
                    kind: DistanceKind::Aligned,
                    value: 6.0,
                    label: P2::ZERO,
                }
            } else {
                ConstraintKind::Radius { entity: curve, value: 6.0, diameter: false, label: P2::ZERO }
            };
            sketch.constrain(dimension);
            solve::solve(&mut sketch, None, true);
            assert!(
                !matches!(sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }),
                "{operation:?}: {:?}",
                sketch.status
            );
            for &(a, b) in &expected {
                assert!(
                    sketch.point(a).unwrap().dist(sketch.point(b).unwrap()) < 1e-6,
                    "{operation:?}: dimension opened the join"
                );
            }
            assert_eq!(sketch.regions().unwrap().len(), regions, "{operation:?}: dimension opened the profile");
        }
    }
}

#[test]
fn partial_ellipse_copies_keep_joins_through_drags_and_dimensions() {
    check_copied_profile_joins(true);
}

#[test]
fn circular_arc_copies_keep_joins_through_drags_and_dimensions() {
    check_copied_profile_joins(false);
}

#[test]
fn partial_ellipse_trim_extend_and_offset_refuse_atomically() {
    use soliddraft_sketch::tools::{End, Operation, TrimOptions};
    let mut sketch = Sketch::new();
    let curve = sketch.add(
        EntityKind::EllipticArc {
            center: P2::ZERO,
            major: P2::new(4.0, 0.0),
            minor: P2::new(0.0, 2.0),
            start: P2::new(4.0, 0.0),
            end: P2::new(-4.0, 0.0),
        },
        false,
    );
    let boundary = sketch.add(EntityKind::Line { a: P2::new(10.0, -5.0), b: P2::new(10.0, 5.0) }, true);
    for operation in [
        Operation::Trim { entity: curve, at: P2::new(0.0, 2.0), options: TrimOptions::default() },
        Operation::Extend { entity: curve, end: End::Start, boundary },
        Operation::Offset { entities: vec![curve], distance: 1.0, options: Default::default() },
    ] {
        let before = sketch.clone();
        assert!(sketch.apply_operation(&operation).is_err(), "{operation:?}");
        assert_eq!(sketch, before, "{operation:?} mutated the refused sketch");
    }
}
