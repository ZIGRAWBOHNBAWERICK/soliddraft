//! Point placement and previews for the two spline drawing tools.

use super::*;

pub(super) fn preview(s: &SketchSession, cursor: P2) -> Option<EntityKind> {
    let mode = s.tool.spline_mode()?;
    let mut points: Vec<_> = s.pending.iter().map(|(point, _)| *point).collect();
    if points.last().is_none_or(|point| point.dist(cursor) > 1e-9) {
        points.push(cursor);
    }
    (points.len() >= 2).then_some(EntityKind::Spline { mode, points })
}

pub(super) fn paint(painter: &egui::Painter, frame: &Frame<'_>, s: &SketchSession, cursor: P2, stroke: Stroke) {
    let Some(kind) = preview(s, cursor) else {
        return;
    };
    let entity = Entity { id: 0, kind, construction: false };
    let curve: Vec<_> = entity.polyline(96).into_iter().filter_map(|point| frame.screen(point)).collect();
    if curve.len() >= 2 {
        painter.add(egui::Shape::line(curve, stroke));
    }
    let points: Vec<_> = entity.points().into_iter().filter_map(|point| frame.screen(point)).collect();
    if s.tool == Tool::ControlSpline {
        dashed(painter, &points, Stroke::new(0.8, theme::muted()));
    }
    for point in points {
        painter.circle_stroke(point, 3.0, stroke);
    }
}

pub(super) fn click(s: &mut SketchSession, point: P2, snaps: Vec<Snap>, double: bool, actions: &mut Vec<AppAction>) {
    if double {
        finish(s, actions);
        return;
    }
    let closes = s.pending.len() >= 3 && s.pending[0].0.dist(point) <= 1e-6;
    s.pending.push((point, snaps));
    if closes {
        finish(s, actions);
    }
}

pub(super) fn finish(s: &mut SketchSession, actions: &mut Vec<AppAction>) {
    let Some(mode) = s.tool.spline_mode() else {
        return;
    };
    if s.pending.len() < 2 || s.pending.iter().all(|(point, _)| point.dist(s.pending[0].0) <= 1e-6) {
        s.say("Place at least two distinct spline points");
        return;
    }
    let before = s.sketch.clone();
    let points: Vec<_> = s.pending.iter().map(|(point, _)| *point).collect();
    let closed = points.len() >= 3 && points[0].dist(*points.last().unwrap()) <= 1e-6;
    let last = points.len() - 1;
    let id = s.sketch.add(EntityKind::Spline { mode, points }, s.construction);
    for (index, (_, snaps)) in s.pending.iter().enumerate() {
        apply_snaps(&mut s.sketch, PointRef::new(id, index), snaps, None);
    }
    if closed {
        s.sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(id, 0), b: PointRef::new(id, last) });
    }
    solve::solve(&mut s.sketch, None, true);
    if matches!(s.sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }) {
        s.sketch = before;
        s.say("The spline points conflict with existing sketch relations");
        return;
    }
    s.pending.clear();
    s.chain_from = None;
    commit(s, actions);
}
