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
    let length = (minor_pick - *center).dot(axis.perp().normalized());
    if length.abs() <= 1e-6 {
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
        let point = PointRef::new(id, index);
        apply_snaps(&mut s.sketch, point, snaps, None);
        if index == 1 {
            let target = Align::Points(PointRef::new(id, 0), point);
            for snap in snaps {
                match snap {
                    Snap::Horizontal => {
                        s.sketch.constrain(ConstraintKind::Horizontal { target });
                    }
                    Snap::Vertical => {
                        s.sketch.constrain(ConstraintKind::Vertical { target });
                    }
                    _ => {}
                }
            }
        }
    }
    solve::solve(&mut s.sketch, None, true);
    let kind = s.sketch.entity(id).unwrap().kind.clone();
    let valid = Ellipse::of(&kind).is_some_and(Ellipse::valid)
        && !matches!(kind, EntityKind::EllipticArc { start, end, .. } if start.dist(end) <= 1e-6);
    if !valid || matches!(s.sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }) {
        s.sketch = before;
        return;
    }
    s.pending.clear();
    commit(s, actions);
}
