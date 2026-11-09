//! Shared preview and placement for the rectangle family.

use super::*;

const MIN_EDGE: f64 = 1e-6;

/// Project the last pick onto the side being placed. Inference at an unrelated
/// raw pointer position must not attach a relation to a different corner.
pub(super) fn project_pick(s: &SketchSession, p: P2) -> P2 {
    if let [(a, _), (b, _)] = s.pending.as_slice() {
        let normal = (*b - *a).normalized().perp();
        match s.tool {
            Tool::ThreePointCornerRectangle | Tool::ThreePointCentreRectangle => {
                return *b + normal.scale((p - *b).dot(normal));
            }
            _ => {}
        }
    }
    p
}

pub(super) fn corners(s: &SketchSession, p: P2) -> Option<[P2; 4]> {
    let a = s.pending.first()?.0;
    let c = match s.tool {
        Tool::Rectangle | Tool::CentreRectangle => {
            let a = if s.tool == Tool::CentreRectangle { a.scale(2.0) - p } else { a };
            [a, P2::new(p.u, a.v), p, P2::new(a.u, p.v)]
        }
        Tool::ThreePointCornerRectangle | Tool::Parallelogram => {
            let b = s.pending.get(1)?.0;
            let c = project_pick(s, p);
            [a, b, c, a + (c - b)]
        }
        Tool::ThreePointCentreRectangle => {
            let b = s.pending.get(1)?.0;
            let width = project_pick(s, p) - b;
            let opposite = a.scale(2.0) - b;
            [b + width, b - width, opposite - width, opposite + width]
        }
        _ => return None,
    };
    let edge = c[1] - c[0];
    let side = c[3] - c[0];
    (c.iter().all(|p| p.u.is_finite() && p.v.is_finite())
        && edge.len() >= MIN_EDGE
        && side.len() >= MIN_EDGE
        && edge.normalized().cross(side).abs() >= MIN_EDGE)
        .then_some(c)
}

/// A diagonal for the axis-aligned centre tool, or a centreline joining opposite
/// side midpoints for the three-point centre tool.
pub(super) fn guide(tool: Tool, c: [P2; 4]) -> Option<(P2, P2)> {
    match tool {
        Tool::CentreRectangle => Some((c[0], c[2])),
        Tool::ThreePointCentreRectangle => Some((c[0].mid(c[1]), c[2].mid(c[3]))),
        _ => None,
    }
}

pub(super) fn add_loop(sketch: &mut Sketch, c: [P2; 4], tool: Tool, construction: bool) -> [EntityId; 4] {
    let ids = std::array::from_fn(|i| sketch.add(EntityKind::Line { a: c[i], b: c[(i + 1) % 4] }, construction));
    for i in 0..4 {
        sketch.constrain(ConstraintKind::Coincident {
            a: PointRef::new(ids[i], 1),
            b: PointRef::new(ids[(i + 1) % 4], 0),
        });
    }
    if matches!(tool, Tool::Rectangle | Tool::CentreRectangle) {
        for i in [0, 2] {
            sketch.constrain(ConstraintKind::Horizontal { target: Align::Line(ids[i]) });
        }
        for i in [1, 3] {
            sketch.constrain(ConstraintKind::Vertical { target: Align::Line(ids[i]) });
        }
    } else {
        sketch.constrain(ConstraintKind::Parallel { a: ids[0], b: ids[2] });
        sketch.constrain(ConstraintKind::Parallel { a: ids[1], b: ids[3] });
        if tool != Tool::Parallelogram {
            sketch.constrain(ConstraintKind::Perpendicular { a: ids[0], b: ids[1] });
        }
    }
    ids
}

pub(super) fn click(s: &mut SketchSession, p: P2, snaps: Vec<Snap>, actions: &mut Vec<AppAction>, analyse: bool) {
    if !p.u.is_finite() || !p.v.is_finite() {
        return;
    }
    let picks = if matches!(s.tool, Tool::Rectangle | Tool::CentreRectangle) { 2 } else { 3 };
    if s.pending.len() < picks - 1 {
        if s.pending.first().is_none_or(|(a, _)| a.dist(p) >= MIN_EDGE) {
            s.pending.push((p, snaps));
        }
        return;
    }
    let Some(c) = corners(s, p) else { return };
    let before = s.sketch.clone();
    let ids = add_loop(&mut s.sketch, c, s.tool, s.construction);
    let corner = |i| PointRef::new(ids[i], 0);
    let (first, second) = if let Some((a, b)) = guide(s.tool, c) {
        let axis = s.sketch.add(EntityKind::Line { a, b }, true);
        let center = s.sketch.add(EntityKind::Point { p: s.pending[0].0 }, true);
        for i in 0..2 {
            let point = PointRef::new(axis, i);
            let kind = if s.tool == Tool::CentreRectangle {
                ConstraintKind::Coincident { a: point, b: corner(i * 2) }
            } else {
                ConstraintKind::Midpoint { point, line: ids[i * 2] }
            };
            s.sketch.constrain(kind);
        }
        s.sketch.constrain(ConstraintKind::Midpoint { point: PointRef::new(center, 0), line: axis });
        (PointRef::new(center, 0), PointRef::new(axis, 0))
    } else {
        (corner(0), corner(1))
    };
    apply_snaps(&mut s.sketch, first, &s.pending[0].1, None);
    if picks == 3 {
        apply_snaps(&mut s.sketch, second, &s.pending[1].1, None);
    }
    // A projected pick can only retain a snap when that snap describes the corner
    // actually shown. Off-side pointer snaps are discarded before painting too.
    if project_pick(s, p).dist(p) < MIN_EDGE {
        let final_corner = if s.tool == Tool::ThreePointCentreRectangle { corner(0) } else { corner(2) };
        apply_snaps(&mut s.sketch, final_corner, &snaps, None);
    }
    solve::solve(&mut s.sketch, None, analyse);
    let valid = ids.iter().all(|id| {
        s.sketch.entity(*id).is_some_and(|e| {
            matches!(e.kind, EntityKind::Line { a, b } if a.u.is_finite() && a.v.is_finite()
                && b.u.is_finite() && b.v.is_finite() && a.dist(b) >= MIN_EDGE)
        })
    });
    let height = match (s.sketch.entity(ids[0]), s.sketch.entity(ids[1])) {
        (Some(a), Some(b)) => match (&a.kind, &b.kind) {
            (EntityKind::Line { a, b }, EntityKind::Line { a: c, b: d }) => (*b - *a).normalized().cross(*d - *c).abs(),
            _ => 0.0,
        },
        _ => 0.0,
    };
    if !valid
        || height < MIN_EDGE
        || matches!(s.sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. })
    {
        s.sketch = before;
        s.say("Those references cannot form this rectangle; choose another point");
        return;
    }
    s.pending.clear();
    commit(s, actions);
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOOLS: [Tool; 5] = [
        Tool::Rectangle,
        Tool::CentreRectangle,
        Tool::ThreePointCornerRectangle,
        Tool::ThreePointCentreRectangle,
        Tool::Parallelogram,
    ];

    fn session(tool: Tool) -> SketchSession {
        let mut s = SketchSession::new(1, Plane::XY, Sketch::new(), false);
        s.tool = tool;
        s
    }

    fn place(s: &mut SketchSession, picks: &[P2]) -> Vec<AppAction> {
        let mut actions = Vec::new();
        for p in picks {
            click(s, *p, Vec::new(), &mut actions, true);
        }
        actions
    }

    #[test]
    fn rectangle_degenerate_and_nonfinite_picks_are_atomic_and_retryable() {
        for tool in TOOLS {
            let mut s = session(tool);
            let before = s.sketch.clone();
            let a = P2::new(10.0, 10.0);
            let b = P2::new(30.0, 20.0);
            assert!(place(&mut s, &[P2::new(f64::NAN, 0.0), P2::new(0.0, f64::INFINITY)]).is_empty());
            assert!(s.pending.is_empty());
            assert!(place(&mut s, &[a, a]).is_empty());
            assert_eq!(s.pending.len(), 1);
            if matches!(tool, Tool::Rectangle | Tool::CentreRectangle) {
                assert!(place(&mut s, &[P2::new(a.u, 40.0), P2::new(40.0, a.v), P2::new(a.u + 1e-8, 40.0)]).is_empty());
                assert_eq!(s.pending.len(), 1);
            } else {
                assert!(place(&mut s, &[b, b + (b - a)]).is_empty());
                assert!(place(&mut s, &[b + (b - a).normalized().perp().scale(1e-8)]).is_empty());
                assert_eq!(s.pending.len(), 2);
            }
            assert_eq!(s.sketch, before, "{tool:?}");
            let actions = place(&mut s, &[P2::new(-20.0, 40.0)]);
            assert_eq!(actions.len(), 1, "{tool:?}");
            assert!(s.pending.is_empty());
            assert_eq!(s.sketch.regions().unwrap().len(), 1);
        }
    }

    #[test]
    fn rectangle_projected_height_does_not_inherit_an_off_side_snap() {
        for tool in [Tool::ThreePointCornerRectangle, Tool::ThreePointCentreRectangle] {
            let mut s = session(tool);
            assert!(place(&mut s, &[P2::ZERO, P2::new(20.0, 0.0)]).is_empty());
            let mut actions = Vec::new();
            click(&mut s, P2::new(40.0, 10.0), vec![Snap::Point(PointRef::ORIGIN)], &mut actions, true);
            assert_eq!(actions.len(), 1);
            assert!(s.sketch.regions().is_ok());
            let edge = if tool == Tool::ThreePointCentreRectangle { 1 } else { 3 };
            assert!(s.sketch.entity(edge).unwrap().point(0).unwrap().dist(P2::new(20.0, 10.0)) < 1e-6);
        }
    }

    #[test]
    fn rectangle_signed_sides_and_rotated_references_match_the_documented_shapes() {
        for sign in [-1.0, 1.0] {
            let cases = [
                (Tool::Rectangle, vec![P2::ZERO, P2::new(30.0, sign * 20.0)], 600.0),
                (Tool::CentreRectangle, vec![P2::ZERO, P2::new(30.0, sign * 20.0)], 2400.0),
                (
                    Tool::ThreePointCornerRectangle,
                    vec![P2::ZERO, P2::new(24.0, 18.0), P2::new(24.0 - sign * 12.0, 18.0 + sign * 16.0)],
                    600.0,
                ),
                (
                    Tool::ThreePointCentreRectangle,
                    vec![P2::ZERO, P2::new(24.0, 18.0), P2::new(24.0 - sign * 12.0, 18.0 + sign * 16.0)],
                    2400.0,
                ),
                (Tool::Parallelogram, vec![P2::ZERO, P2::new(30.0, 0.0), P2::new(40.0, sign * 20.0)], 600.0),
            ];
            for (tool, points, expected_area) in cases {
                let mut s = session(tool);
                assert_eq!(place(&mut s, &points).len(), 1, "{tool:?}");
                assert!((s.sketch.regions().unwrap()[0].area() - expected_area).abs() < 1e-5, "{tool:?}");
                let actual: Vec<_> = s.sketch.drawn().filter(|e| !e.construction).collect();
                assert_eq!(actual.len(), 4);
                let last =
                    if tool == Tool::ThreePointCentreRectangle { actual[0].point(0) } else { actual[2].point(0) };
                assert!(last.unwrap().dist(*points.last().unwrap()) < 1e-6, "{tool:?}");
                if matches!(tool, Tool::CentreRectangle | Tool::ThreePointCentreRectangle) {
                    assert!(actual[0].point(0).unwrap().mid(actual[2].point(0).unwrap()).dist(P2::ZERO) < 1e-6);
                }
            }
        }
    }

    #[test]
    fn rectangle_incompatible_corner_snaps_refuse_without_geometry_or_history() {
        for tool in TOOLS {
            let mut s = session(tool);
            let before = s.sketch.clone();
            let mut actions = Vec::new();
            click(&mut s, P2::ZERO, vec![Snap::Point(PointRef::ORIGIN)], &mut actions, true);
            if !matches!(tool, Tool::Rectangle | Tool::CentreRectangle) {
                click(&mut s, P2::new(20.0, 0.0), vec![Snap::Point(PointRef::ORIGIN)], &mut actions, true);
            }
            click(&mut s, P2::new(30.0, 20.0), vec![Snap::Point(PointRef::ORIGIN)], &mut actions, true);
            assert_eq!(s.sketch, before, "{tool:?}");
            assert!(actions.is_empty());
            assert!(!s.pending.is_empty());
        }
    }

    #[test]
    fn rectangle_family_keeps_dynamic_mirror_as_one_closed_profile_placement() {
        for tool in TOOLS {
            let mut s = session(tool);
            let kind = EntityKind::Line { a: P2::new(-80.0, -50.0), b: P2::new(-80.0, 50.0) };
            let axis = s.sketch.add(kind.clone(), true);
            s.sketch.constrain(ConstraintKind::FixEntity { entity: axis, at: kind });
            s.dynamic_mirror = Some(axis);
            let picks = if matches!(tool, Tool::Rectangle | Tool::CentreRectangle) {
                vec![P2::ZERO, P2::new(30.0, 20.0)]
            } else {
                vec![P2::ZERO, P2::new(20.0, 0.0), P2::new(30.0, 20.0)]
            };
            let mut actions = Vec::new();
            for p in picks {
                mirrored_placement(&mut s, p, Vec::new(), false, &mut actions, true);
            }
            assert_eq!(actions.len(), 1, "{tool:?}: {:?}", s.message);
            assert_eq!(s.sketch.drawn().filter(|e| !e.construction).count(), 8);
            assert_eq!(s.sketch.regions().unwrap().len(), 2);
            assert!(matches!(s.sketch.status, SolveStatus::UnderDefined | SolveStatus::FullyDefined));
        }
    }
}
