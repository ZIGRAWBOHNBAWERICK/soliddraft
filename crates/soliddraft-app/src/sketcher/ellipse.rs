//! Pointer placement shared by full and partial ellipses.

use super::*;
use crate::sketch::ellipse::Ellipse;

fn geometry(s: &SketchSession, p: P2) -> Option<Ellipse> {
    let (center, _) = s.pending.first()?;
    let (major, _) = s.pending.get(1)?;
    let axis = *major - *center;
    if axis.len() <= 1e-6 {
        return None;
    }
    let minor_pick = s.pending.get(2).map_or(p, |(p, _)| *p);
    let length = (minor_pick - *center).dot(axis.perp().normalized()).abs();
    if length <= 1e-6 {
        return None;
    }
    Some(Ellipse { center: *center, major: *major, minor: *center + axis.perp().normalized().scale(length) })
}

pub(super) fn preview(s: &SketchSession, p: P2) -> Option<EntityKind> {
    let ellipse = geometry(s, p)?;
    let Ellipse { center, major, minor } = ellipse;
    if s.tool == Tool::PartialEllipse
        && let Some((start, _)) = s.pending.get(3)
    {
        let end = ellipse.at(ellipse.parameter(p));
        Some(EntityKind::EllipticArc { center, major, minor, start: *start, end })
    } else {
        Some(EntityKind::Ellipse { center, major, minor })
    }
}

pub(super) fn click(s: &mut SketchSession, p: P2, snaps: Vec<Snap>, actions: &mut Vec<AppAction>) {
    if s.pending.len() < 2 {
        if s.pending.first().is_none_or(|(c, _)| c.dist(p) > 1e-6) {
            s.pending.push((p, snaps));
        }
        return;
    }
    let Some(ellipse) = geometry(s, p) else { return };
    if s.tool == Tool::PartialEllipse && s.pending.len() < 4 {
        let p = if s.pending.len() == 2 { ellipse.minor } else { ellipse.at(ellipse.parameter(p)) };
        s.pending.push((p, snaps));
        return;
    }
    let Ellipse { center, major, minor } = ellipse;
    let kind = if s.tool == Tool::Ellipse {
        EntityKind::Ellipse { center, major, minor }
    } else {
        let start = s.pending[3].0;
        let end = ellipse.at(ellipse.parameter(p));
        if start.dist(end) <= 1e-6 {
            return;
        }
        EntityKind::EllipticArc { center, major, minor, start, end }
    };
    let before = s.sketch.clone();
    let id = s.sketch.add(kind, s.construction);
    let mut picks = s.pending.clone();
    picks.push((p, snaps));
    for (index, (_, snaps)) in picks.iter().enumerate() {
        // Axis projection can move the pick. Keep only exact point snaps there.
        let point = s.sketch.point(PointRef::new(id, index as u8)).unwrap();
        let snaps: Vec<_> = snaps
            .iter()
            .filter(|snap| match snap {
                Snap::Point(r) => s.sketch.point(*r).is_some_and(|q| point.dist(q) < 1e-6),
                _ => false,
            })
            .cloned()
            .collect();
        apply_snaps(&mut s.sketch, PointRef::new(id, index as u8), &snaps, None);
    }
    solve::solve(&mut s.sketch, None, true);
    if matches!(s.sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }) {
        s.sketch = before;
        return;
    }
    s.pending.clear();
    commit(s, actions);
}
