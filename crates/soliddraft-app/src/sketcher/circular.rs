//! Circular tools share operation geometry between painting and placement.

use super::*;

pub(super) fn endpoint(s: &SketchSession, frame: &Frame<'_>, pos: Pos2) -> Option<(P2, Vec<Snap>)> {
    s.sketch
        .drawn()
        .flat_map(|e| {
            let indices: &[usize] = match &e.kind {
                EntityKind::Line { .. } => &[0, 1],
                EntityKind::Arc { .. } => &[1, 2],
                _ => &[],
            };
            indices.iter().filter_map(move |&index| {
                let p = e.point(index)?;
                let distance = frame.screen(p)?.distance(pos);
                (distance <= POINT_REACH).then_some((distance, p, PointRef::new(e.id, index)))
            })
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, p, r)| (p, vec![Snap::Point(r)]))
}

fn source(s: &SketchSession) -> Option<PointRef> {
    s.pending.first()?.1.iter().find_map(|snap| if let Snap::Point(r) = snap { Some(*r) } else { None })
}

fn operation(s: &SketchSession, p: P2) -> Option<Operation> {
    let construction = s.construction;
    let a = s.pending.first()?.0;
    Some(match s.tool {
        Tool::PerimeterCircle => Operation::PerimeterCircle { a, b: s.pending.get(1)?.0, c: p, construction },
        Tool::Arc => Operation::CentrepointArc {
            center: a,
            start: s.pending.get(1)?.0,
            end: p,
            clockwise: s.arc_clockwise,
            construction,
        },
        Tool::ThreePointArc => {
            Operation::ThreePointArc { start: a, end: s.pending.get(1)?.0, through: p, construction }
        }
        Tool::TangentArc => Operation::TangentArc { from: source(s)?, end: p, construction },
        _ => return None,
    })
}

pub(super) fn preview(s: &SketchSession, p: P2) -> Option<EntityKind> {
    let operation = operation(s, p)?;
    let mut sketch = s.sketch.clone();
    let ids = sketch.apply_operation(&operation).ok()?;
    Some(sketch.entity(ids[0])?.kind.clone())
}

fn point_on_snap(sketch: &mut Sketch, id: EntityId, snaps: &[Snap]) {
    for snap in snaps {
        if let Snap::Point(point) = snap {
            sketch.constrain(ConstraintKind::PointOn { point: *point, entity: id });
        }
    }
}

pub(super) fn click(s: &mut SketchSession, p: P2, snaps: Vec<Snap>, actions: &mut Vec<AppAction>, analyse: bool) {
    let needed = if s.tool == Tool::TangentArc { 1 } else { 2 };
    if s.pending.len() < needed {
        if s.tool == Tool::TangentArc {
            let from = snaps.iter().find_map(|snap| if let Snap::Point(r) = snap { Some(*r) } else { None });
            if from.is_none_or(|from| {
                let Some(e) = s.sketch.entity(from.entity) else { return true };
                !matches!((&e.kind, from.index), (EntityKind::Line { .. }, 0 | 1) | (EntityKind::Arc { .. }, 1 | 2))
            }) {
                s.say("Pick a line or circular arc endpoint");
                return;
            }
        }
        if s.pending.first().is_none_or(|(a, _)| a.dist(p) > 1e-6) {
            s.pending.push((p, snaps));
        }
        return;
    }
    let Some(operation) = operation(s, p) else { return };
    let before = s.sketch.clone();
    let ids = match s.sketch.apply_operation(&operation) {
        Ok(ids) => ids,
        Err(error) => {
            s.say(error.to_string());
            return;
        }
    };
    let id = ids[0];
    match s.tool {
        Tool::PerimeterCircle => {
            for (_, snaps) in &s.pending {
                point_on_snap(&mut s.sketch, id, snaps);
            }
            point_on_snap(&mut s.sketch, id, &snaps);
        }
        Tool::Arc => {
            apply_snaps(&mut s.sketch, PointRef::new(id, 0), &s.pending[0].1, None);
            let (first, last) = if s.arc_clockwise { (2, 1) } else { (1, 2) };
            apply_snaps(&mut s.sketch, PointRef::new(id, first), &s.pending[1].1, None);
            if s.sketch.point(PointRef::new(id, last)).is_some_and(|end| end.dist(p) <= 1e-6) {
                apply_snaps(&mut s.sketch, PointRef::new(id, last), &snaps, None);
            }
        }
        Tool::ThreePointArc => {
            let first = if s.sketch.point(PointRef::new(id, 1)).unwrap().dist(s.pending[0].0) <= 1e-6 { 1 } else { 2 };
            apply_snaps(&mut s.sketch, PointRef::new(id, first), &s.pending[0].1, None);
            apply_snaps(&mut s.sketch, PointRef::new(id, 3 - first), &s.pending[1].1, None);
            point_on_snap(&mut s.sketch, id, &snaps);
        }
        Tool::TangentArc => {
            let first = if s.sketch.point(PointRef::new(id, 1)).unwrap().dist(s.pending[0].0) <= 1e-6 { 1 } else { 2 };
            apply_snaps(&mut s.sketch, PointRef::new(id, 3 - first), &snaps, None);
        }
        _ => unreachable!(),
    }
    solve::solve(&mut s.sketch, None, analyse);
    let kind = &s.sketch.entity(id).unwrap().kind;
    if !crate::sketch::tools::valid(kind)
        || matches!(s.sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. })
    {
        s.sketch = before;
        s.say("The inferred relations do not fit this circle or arc");
        return;
    }
    s.pending.clear();
    commit(s, actions);
}
