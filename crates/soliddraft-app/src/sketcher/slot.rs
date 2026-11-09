//! Slot placement uses ordinary editable curves and retained sketch relations.

use super::*;
use std::f64::consts::{PI, TAU};

const EPS: f64 = 1e-6;
type Result<T> = std::result::Result<T, &'static str>;

struct Geometry {
    // Side, end cap, side, start cap, in the existing straight slot's order.
    boundary: [EntityKind; 4],
    axis: EntityKind,
}

#[derive(Clone, Copy)]
struct Path {
    center: P2,
    start: P2,
    end: P2,
    start_pick: usize,
    end_pick: usize,
}

impl Path {
    fn sweep(self) -> f64 {
        (self.end.sub(self.center).angle() - self.start.sub(self.center).angle()).rem_euclid(TAU)
    }

    fn kind(self) -> EntityKind {
        EntityKind::Arc { center: self.center, start: self.start, end: self.end }
    }

    fn slot(self, width_pick: P2) -> Result<Geometry> {
        let radius = self.center.dist(self.start);
        if !radius.is_finite() || radius <= EPS || self.start.dist(self.end) <= EPS {
            return Err("Choose a finite, open arc path");
        }
        let half_width = (self.center.dist(width_pick) - radius).abs();
        if !half_width.is_finite() || half_width <= EPS {
            return Err("Choose a nonzero slot width");
        }
        if radius - half_width <= EPS {
            return Err("Slot width would consume the inner radius");
        }
        if self.sweep() > PI && self.start.dist(self.end) <= 2.0 * half_width + EPS {
            return Err("Slot end caps would touch or overlap");
        }
        let a = self.start.sub(self.center).normalized().scale(half_width);
        let b = self.end.sub(self.center).normalized().scale(half_width);
        Ok(Geometry {
            boundary: [
                EntityKind::Arc { center: self.center, start: self.start + a, end: self.end + b },
                EntityKind::Arc { center: self.end, start: self.end + b, end: self.end - b },
                EntityKind::Arc { center: self.center, start: self.start - a, end: self.end - b },
                EntityKind::Arc { center: self.start, start: self.start - a, end: self.start + a },
            ],
            axis: self.kind(),
        })
    }
}

fn arc_path(tool: Tool, picks: &[P2]) -> Result<Path> {
    let [a, b, p] = picks else { return Err("Pick three points for the arc") };
    if tool == Tool::CentrepointArcSlot {
        let radius = a.dist(*b);
        if radius <= EPS || a.dist(*p) <= EPS {
            return Err("Choose a nonzero arc radius");
        }
        let end = *a + p.sub(*a).normalized().scale(radius);
        if b.dist(end) <= EPS {
            return Err("Choose different arc endpoints");
        }
        return Ok(Path { center: *a, start: *b, end, start_pick: 1, end_pick: 2 });
    }
    let d = *b - *a;
    let e = *p - *a;
    let cross = d.u * e.v - d.v * e.u;
    // Plane picks pass through the camera's f32 projection. A relative angular
    // tolerance keeps a visually collinear pick from producing an enormous arc.
    let collinear = (EPS * d.len().max(e.len())).max(1e-5 * d.len() * e.len());
    if d.len() <= EPS || e.len() <= EPS || b.dist(*p) <= EPS || cross.abs() <= collinear {
        return Err("Choose three distinct, non-collinear arc points");
    }
    // Work relative to the first pick to avoid cancellation far from the origin.
    let dd = d.dot(d);
    let ee = e.dot(e);
    let center = *a + P2::new(dd * e.v - ee * d.v, d.u * ee - e.u * dd).scale(0.5 / cross);
    let mut path = Path { center, start: *a, end: *b, start_pick: 0, end_pick: 1 };
    let through = (p.sub(center).angle() - a.sub(center).angle()).rem_euclid(TAU);
    if through > path.sweep() {
        std::mem::swap(&mut path.start, &mut path.end);
        std::mem::swap(&mut path.start_pick, &mut path.end_pick);
    }
    Ok(path)
}

fn straight_centres(tool: Tool, first: P2, end: P2) -> (P2, P2) {
    (if tool == Tool::CentrepointSlot { first.scale(2.0) - end } else { first }, end)
}

fn straight(tool: Tool, first: P2, end: P2, width_pick: P2) -> Result<Geometry> {
    let (c0, c1) = straight_centres(tool, first, end);
    let d = c1 - c0;
    if d.len() <= EPS {
        return Err("Choose different slot centres");
    }
    let n = d.normalized().perp();
    let r = (width_pick - c0).dot(n).abs();
    // Retain the original Slot command's minimum-width behaviour.
    let r = if tool == Tool::Slot { r.max(d.len() * 0.05) } else { r };
    if !r.is_finite() || r <= EPS {
        return Err("Choose a nonzero slot width");
    }
    Ok(Geometry {
        boundary: [
            EntityKind::Line { a: c0 + n.scale(r), b: c1 + n.scale(r) },
            EntityKind::Arc { center: c1, start: c1 - n.scale(r), end: c1 + n.scale(r) },
            EntityKind::Line { a: c1 - n.scale(r), b: c0 - n.scale(r) },
            EntityKind::Arc { center: c0, start: c0 + n.scale(r), end: c0 - n.scale(r) },
        ],
        axis: EntityKind::Line { a: c0, b: c1 },
    })
}

fn curved(tool: Tool) -> bool {
    matches!(tool, Tool::ThreePointArcSlot | Tool::CentrepointArcSlot)
}

fn geometry(s: &SketchSession, p: P2) -> Result<Geometry> {
    let picks: Vec<_> = s.pending.iter().map(|(p, _)| *p).collect();
    if curved(s.tool) { arc_path(s.tool, &picks)?.slot(p) } else { straight(s.tool, picks[0], picks[1], p) }
}

pub(super) fn preview(s: &SketchSession, p: P2) -> Vec<(EntityKind, bool)> {
    if s.pending.is_empty() {
        return Vec::new();
    }
    if s.pending.len() == 1 {
        let (a, b) = straight_centres(s.tool, s.pending[0].0, p);
        return vec![(EntityKind::Line { a, b }, true)];
    }
    if curved(s.tool) && s.pending.len() == 2 {
        let picks = [s.pending[0].0, s.pending[1].0, p];
        return arc_path(s.tool, &picks).map(|path| vec![(path.kind(), true)]).unwrap_or_default();
    }
    let Ok(geometry) = geometry(s, p) else { return Vec::new() };
    geometry.boundary.into_iter().map(|kind| (kind, s.construction)).chain([(geometry.axis, true)]).collect()
}

pub(super) fn click(s: &mut SketchSession, p: P2, snaps: Vec<Snap>, actions: &mut Vec<AppAction>, analyse: bool) {
    if !p.u.is_finite() || !p.v.is_finite() {
        s.say("Choose finite slot coordinates");
        return;
    }
    let count = if curved(s.tool) { 3 } else { 2 };
    if s.pending.len() < count {
        if s.pending.len() == 1 && s.pending[0].0.dist(p) <= EPS {
            s.say("Choose a different point");
            return;
        }
        if s.pending.len() == 2 {
            let picks = [s.pending[0].0, s.pending[1].0, p];
            if let Err(why) = arc_path(s.tool, &picks) {
                s.say(why);
                return;
            }
        }
        s.pending.push((p, snaps));
        s.message = None;
        return;
    }
    let geometry = match geometry(s, p) {
        Ok(geometry) => geometry,
        Err(why) => {
            s.say(why);
            return;
        }
    };
    let before = s.sketch.clone();
    let ids = geometry.boundary.map(|kind| s.sketch.add(kind, s.construction));
    let [side, end, other, start] = ids;
    let axis = s.sketch.add(geometry.axis, true);
    let joins = if curved(s.tool) {
        [(side, 2, end, 1), (end, 2, other, 2), (other, 1, start, 1), (start, 2, side, 1)]
    } else {
        [(side, 1, end, 2), (end, 1, other, 0), (other, 1, start, 2), (start, 1, side, 0)]
    };
    for (a, ai, b, bi) in joins {
        s.sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(a, ai), b: PointRef::new(b, bi) });
    }
    let axis_start = u8::from(curved(s.tool));
    for (index, cap) in [(axis_start, start), (axis_start + 1, end)] {
        s.sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(axis, index), b: PointRef::new(cap, 0) });
    }
    for (a, b) in [(side, end), (other, end), (side, start), (other, start)] {
        s.sketch.constrain(ConstraintKind::Tangent { a, b });
    }
    s.sketch.constrain(ConstraintKind::Equal { a: start, b: end });
    if curved(s.tool) {
        for a in [axis, other] {
            s.sketch.constrain(ConstraintKind::Concentric { a, b: side });
        }
        let picks: Vec<_> = s.pending.iter().map(|(p, _)| *p).collect();
        let path = arc_path(s.tool, &picks).expect("validated arc path");
        for (index, pick, actual) in [(1, path.start_pick, path.start), (2, path.end_pick, path.end)] {
            // A radius-projected pick retains snaps only when it actually lands on them.
            if actual.dist(s.pending[pick].0) <= EPS {
                apply_snaps(&mut s.sketch, PointRef::new(axis, index), &s.pending[pick].1, None);
            }
        }
        if s.tool == Tool::CentrepointArcSlot {
            apply_snaps(&mut s.sketch, PointRef::new(axis, 0), &s.pending[0].1, None);
        } else {
            let through = s.sketch.add(EntityKind::Point { p: s.pending[2].0 }, true);
            s.sketch.constrain(ConstraintKind::PointOn { point: PointRef::new(through, 0), entity: axis });
            apply_snaps(&mut s.sketch, PointRef::new(through, 0), &s.pending[2].1, None);
        }
    } else if s.tool == Tool::CentrepointSlot {
        let midpoint = s.sketch.add(EntityKind::Point { p: s.pending[0].0 }, true);
        s.sketch.constrain(ConstraintKind::Midpoint { point: PointRef::new(midpoint, 0), line: axis });
        apply_snaps(&mut s.sketch, PointRef::new(midpoint, 0), &s.pending[0].1, None);
        apply_snaps(&mut s.sketch, PointRef::new(axis, 1), &s.pending[1].1, Some(axis));
    } else {
        apply_snaps(&mut s.sketch, PointRef::new(axis, 0), &s.pending[0].1, Some(axis));
        apply_snaps(&mut s.sketch, PointRef::new(axis, 1), &s.pending[1].1, Some(axis));
    }
    solve::solve(&mut s.sketch, None, analyse);
    if s.tool != Tool::Slot && matches!(s.sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }) {
        s.sketch = before;
        s.say("Slot relations conflict with the snapped geometry");
        return;
    }
    s.pending.clear();
    s.message = None;
    commit(s, actions);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(tool: Tool) -> SketchSession {
        let mut s = SketchSession::new(1, Plane::XY, Sketch::new(), true);
        s.tool = tool;
        s
    }

    fn place(s: &mut SketchSession, picks: &[P2]) -> Vec<AppAction> {
        let mut actions = Vec::new();
        for &p in picks {
            click(s, p, Vec::new(), &mut actions, true);
        }
        actions
    }

    fn close(a: P2, b: P2) {
        assert!(a.dist(b) < EPS, "{a:?} != {b:?}");
    }

    fn valid_profile(s: &SketchSession) {
        assert!(
            !matches!(s.sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }),
            "{:?}",
            s.sketch.status
        );
        let regions = s.sketch.regions().expect("one closed profile");
        assert_eq!(regions.len(), 1);
        assert!(regions[0].holes.is_empty());
        assert_eq!(s.sketch.constraints.iter().filter(|c| matches!(c.kind, ConstraintKind::Tangent { .. })).count(), 4);
        assert_eq!(s.sketch.constraints.iter().filter(|c| matches!(c.kind, ConstraintKind::Equal { .. })).count(), 1);
    }

    #[test]
    fn original_straight_slot_retains_geometry_relations_and_width_floor() {
        for width in [0.0, 6.0, -6.0] {
            let mut s = session(Tool::Slot);
            let actions = place(&mut s, &[P2::ZERO, P2::new(40.0, 0.0), P2::new(15.0, width)]);
            assert_eq!(actions.len(), 1);
            valid_profile(&s);
            assert_eq!(s.sketch.drawn().count(), 5);
            assert_eq!(s.sketch.constraints.len(), 11);
            let cap = s.sketch.drawn().find(|e| matches!(e.kind, EntityKind::Arc { .. })).unwrap();
            assert!((cap.radius().unwrap() - width.abs().max(2.0)).abs() < EPS);
            assert_eq!(s.sketch.drawn().filter(|e| e.construction).count(), 1);
        }
    }

    #[test]
    fn centrepoint_slot_is_symmetric_in_every_quadrant_and_keeps_its_midpoint_snap() {
        for end in [P2::new(20.0, 0.0), P2::new(0.0, -20.0), P2::new(-12.0, 16.0), P2::new(12.0, -16.0)] {
            let mut s = session(Tool::CentrepointSlot);
            let mut actions = Vec::new();
            click(&mut s, P2::ZERO, vec![Snap::Point(PointRef::ORIGIN)], &mut actions, true);
            let width_pick = end.normalized().perp().scale(4.0);
            actions.extend(place(&mut s, &[end, width_pick]));
            assert_eq!(actions.len(), 1);
            valid_profile(&s);
            let axis = s.sketch.drawn().find(|e| e.construction && e.is_line()).unwrap();
            close(axis.point(0).unwrap(), end.scale(-1.0));
            close(axis.point(1).unwrap(), end);
            let midpoint =
                s.sketch.drawn().find(|e| e.construction && matches!(e.kind, EntityKind::Point { .. })).unwrap();
            assert!(s.sketch.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Midpoint { point, line } if point.entity == midpoint.id && line == axis.id)));
            let mut edited = s.sketch.clone();
            solve::solve(&mut edited, Some(solve::Drag { point: PointRef::new(axis.id, 1), to: end.scale(1.2) }), true);
            let axis = edited.entity(axis.id).unwrap();
            close(axis.point(0).unwrap().mid(axis.point(1).unwrap()), P2::ZERO);
            assert_eq!(edited.regions().unwrap().len(), 1);
        }
    }

    #[test]
    fn straight_refusals_keep_pending_picks_and_do_not_edit_the_document() {
        let mut s = session(Tool::CentrepointSlot);
        let original = s.sketch.clone();
        assert!(place(&mut s, &[P2::ZERO, P2::ZERO]).is_empty());
        assert_eq!(s.pending.len(), 1);
        assert!(place(&mut s, &[P2::new(20.0, 0.0), P2::new(5.0, 0.0)]).is_empty());
        assert_eq!(s.pending.len(), 2);
        assert_eq!(s.sketch, original);
        assert_eq!(place(&mut s, &[P2::new(5.0, -1.0)]).len(), 1);
        valid_profile(&s);
    }

    #[test]
    fn arc_variants_keep_minor_major_wrapping_and_reversed_paths() {
        let r = 20.0;
        let at = |degrees: f64| P2::new(r * degrees.to_radians().cos(), r * degrees.to_radians().sin());
        for (tool, picks, sweep) in [
            (Tool::CentrepointArcSlot, vec![P2::ZERO, at(350.0), at(40.0).scale(1.6), at(20.0).scale(1.1)], 50.0),
            (Tool::CentrepointArcSlot, vec![P2::ZERO, at(0.0), at(240.0), at(120.0).scale(1.1)], 240.0),
            (Tool::CentrepointArcSlot, vec![P2::ZERO, at(0.0), at(180.0), at(90.0).scale(0.9)], 180.0),
            (Tool::ThreePointArcSlot, vec![at(0.0), at(90.0), at(45.0), at(45.0).scale(1.1)], 90.0),
            (Tool::ThreePointArcSlot, vec![at(90.0), at(0.0), at(45.0), at(45.0).scale(0.9)], 90.0),
            (Tool::ThreePointArcSlot, vec![at(0.0), at(90.0), at(240.0), at(240.0).scale(1.1)], 270.0),
        ] {
            let mut s = session(tool);
            let actions = place(&mut s, &picks);
            assert_eq!(actions.len(), 1, "{tool:?} {picks:?}");
            valid_profile(&s);
            let axis = s.sketch.drawn().find(|e| e.construction && matches!(e.kind, EntityKind::Arc { .. })).unwrap();
            let EntityKind::Arc { center, start, end } = axis.kind else { unreachable!() };
            close(center, P2::ZERO);
            assert!(
                ((end.sub(center).angle() - start.sub(center).angle()).rem_euclid(TAU).to_degrees() - sweep).abs()
                    < EPS
            );
            let radii: Vec<_> = s.sketch.drawn().filter(|e| !e.construction).map(|e| e.radius().unwrap()).collect();
            for (actual, expected) in radii.into_iter().zip([22.0, 2.0, 18.0, 2.0]) {
                assert!((actual - expected).abs() < EPS, "{actual} != {expected}");
            }
            let roundtrip: Sketch = serde_json::from_str(&serde_json::to_string(&s.sketch).unwrap()).unwrap();
            assert_eq!(serde_json::to_value(&roundtrip).unwrap(), serde_json::to_value(&s.sketch).unwrap());
        }
    }

    #[test]
    fn arc_boundary_refusals_are_atomic_and_recoverable() {
        assert!(
            arc_path(Tool::ThreePointArcSlot, &[P2::new(20.0, 0.0), P2::new(0.0, 20.0), P2::new(10.0, 10.0 + 1e-5)],)
                .is_err(),
            "screen projection noise must not turn collinear picks into a shallow arc"
        );
        for tool in [Tool::ThreePointArcSlot, Tool::CentrepointArcSlot] {
            let mut s = session(tool);
            let original = s.sketch.clone();
            assert!(place(&mut s, &[P2::ZERO, P2::ZERO]).is_empty());
            assert_eq!(s.pending.len(), 1);
            assert!(place(&mut s, &[P2::new(20.0, 0.0), P2::new(10.0, 0.0)]).is_empty());
            assert_eq!(s.pending.len(), 2);
            assert_eq!(s.sketch, original);
        }
        let mut s = session(Tool::CentrepointArcSlot);
        let original = s.sketch.clone();
        place(&mut s, &[P2::ZERO, P2::new(20.0, 0.0), P2::new(0.0, 20.0)]);
        for invalid in [P2::new(20.0, 0.0), P2::ZERO, P2::new(40.0, 0.0), P2::new(f64::NAN, 0.0)] {
            assert!(place(&mut s, &[invalid]).is_empty());
            assert_eq!(s.pending.len(), 3);
            assert_eq!(s.sketch, original);
            assert!(preview(&s, invalid).is_empty());
        }
        assert_eq!(place(&mut s, &[P2::new(22.0, 0.0)]).len(), 1);
        valid_profile(&s);
        let mut s = session(Tool::CentrepointArcSlot);
        place(&mut s, &[P2::ZERO, P2::new(20.0, 0.0), P2::new(19.0, -1.0)]);
        assert!(place(&mut s, &[P2::new(22.0, 0.0)]).is_empty(), "overlapping major arc caps");
        assert_eq!(s.sketch, original);
    }

    #[test]
    fn construction_and_preview_match_the_committed_boundaries_for_each_tool() {
        for (tool, picks) in [
            (Tool::Slot, vec![P2::ZERO, P2::new(20.0, 0.0), P2::new(10.0, 3.0)]),
            (Tool::CentrepointSlot, vec![P2::ZERO, P2::new(20.0, 0.0), P2::new(10.0, -3.0)]),
            (
                Tool::ThreePointArcSlot,
                vec![P2::new(20.0, 0.0), P2::new(0.0, 20.0), P2::new(12.0, 16.0), P2::new(22.0, 0.0)],
            ),
            (Tool::CentrepointArcSlot, vec![P2::ZERO, P2::new(20.0, 0.0), P2::new(0.0, 20.0), P2::new(22.0, 0.0)]),
        ] {
            for construction in [false, true] {
                let mut s = session(tool);
                s.construction = construction;
                let (last, first) = picks.split_last().unwrap();
                assert!(place(&mut s, first).is_empty());
                let expected = preview(&s, *last);
                assert_eq!(expected.len(), 5);
                assert_eq!(place(&mut s, &[*last]).len(), 1);
                for ((expected, construction), actual) in expected.iter().zip(s.sketch.drawn()) {
                    assert_eq!(*construction, actual.construction);
                    for (a, b) in (Entity { id: 0, kind: *expected, construction: false })
                        .polyline(16)
                        .iter()
                        .zip(actual.polyline(16))
                    {
                        close(*a, b);
                    }
                }
                if construction {
                    assert!(s.sketch.regions().is_err());
                } else {
                    valid_profile(&s);
                }
            }
        }
    }

    #[test]
    fn projected_arc_end_drops_a_snap_that_is_not_on_the_starting_radius() {
        let mut s = session(Tool::CentrepointArcSlot);
        let target = s.sketch.add(EntityKind::Point { p: P2::new(0.0, 40.0) }, true);
        let mut actions = place(&mut s, &[P2::ZERO, P2::new(20.0, 0.0)]);
        click(&mut s, P2::new(0.0, 40.0), vec![Snap::Point(PointRef::new(target, 0))], &mut actions, true);
        actions.extend(place(&mut s, &[P2::new(22.0, 0.0)]));
        assert_eq!(actions.len(), 1);
        valid_profile(&s);
        let axis = s.sketch.drawn().find(|e| e.construction && matches!(e.kind, EntityKind::Arc { .. })).unwrap();
        close(axis.point(2).unwrap(), P2::new(0.0, 20.0));
        assert!(s.sketch.constraints.iter().all(|c| !c.entities().contains(&target)));
        close(s.sketch.point(PointRef::new(target, 0)).unwrap(), P2::new(0.0, 40.0));
    }

    #[test]
    fn conflicting_snaps_restore_the_entire_sketch_and_allow_retry() {
        for tool in [Tool::CentrepointSlot, Tool::ThreePointArcSlot, Tool::CentrepointArcSlot] {
            let mut s = session(tool);
            let target = s.sketch.add(EntityKind::Point { p: P2::new(10.0, 0.0) }, true);
            s.sketch.constrain(ConstraintKind::Fix { point: PointRef::new(target, 0), at: P2::new(10.0, 0.0) });
            let before = s.sketch.clone();
            let mut actions = Vec::new();
            let snaps = vec![Snap::Point(PointRef::ORIGIN), Snap::Point(PointRef::new(target, 0))];
            click(&mut s, P2::ZERO, snaps, &mut actions, true);
            let rest = match tool {
                Tool::CentrepointSlot => vec![P2::new(20.0, 0.0), P2::new(0.0, 4.0)],
                Tool::ThreePointArcSlot => vec![P2::new(20.0, 0.0), P2::new(10.0, 10.0), P2::new(10.0, 12.0)],
                _ => vec![P2::new(20.0, 0.0), P2::new(0.0, 20.0), P2::new(22.0, 0.0)],
            };
            actions.extend(place(&mut s, &rest));
            assert!(actions.is_empty());
            assert_eq!(s.sketch, before);
            assert_eq!(s.pending.len(), rest.len());
            s.pending[0].1.clear();
            assert_eq!(place(&mut s, &[*rest.last().unwrap()]).len(), 1);
            valid_profile(&s);
        }
    }
}
