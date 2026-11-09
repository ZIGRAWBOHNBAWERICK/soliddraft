//! Reflection with a stored axis and symmetry relations.

use super::*;
use crate::relations::Relation;

/// The persistent axis of a linked mirror.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MirrorAxis {
    /// Follow an existing line as it changes.
    Entity {
        /// Existing axis line.
        entity: EntityId,
    },
    /// Create a fixed construction axis from two placed points.
    Points {
        /// First axis point.
        start: P2,
        /// Second axis point.
        end: P2,
    },
}

/// Reflect geometry, reversing arc endpoints to retain counter-clockwise storage.
pub fn reflected(kind: EntityKind, start: P2, end: P2) -> Result<EntityKind> {
    if !valid(&kind) || !finite(start) || !finite(end) || start.dist(end) <= EPS {
        return refuse("reflection needs valid geometry and two distinct finite axis points");
    }
    let dir = (end - start).normalized();
    let map = |p: P2| start + dir.scale(2.0 * (p - start).dot(dir)) - (p - start);
    let kind = match kind {
        EntityKind::Text { text } => EntityKind::Text { text: text.mapped(map) },
        EntityKind::Spline { mode, points } => {
            EntityKind::Spline { mode, points: points.into_iter().map(map).collect() }
        }
        EntityKind::Point { p } => EntityKind::Point { p: map(p) },
        EntityKind::Line { a, b } => EntityKind::Line { a: map(a), b: map(b) },
        EntityKind::Circle { center, radius } => EntityKind::Circle { center: map(center), radius },
        EntityKind::Arc { center, start, end } => {
            EntityKind::Arc { center: map(center), start: map(end), end: map(start) }
        }
        EntityKind::Ellipse { .. } | EntityKind::EllipticArc { .. } => {
            return refuse("linked mirror does not support ellipses or partial ellipses; turn off Keep symmetry");
        }
    };
    if !valid(&kind) {
        return refuse("reflection would produce invalid geometry");
    }
    Ok(kind)
}

fn same(a: &EntityKind, b: &EntityKind) -> bool {
    match (a, b) {
        (EntityKind::Spline { mode, points }, EntityKind::Spline { mode: other_mode, points: other_points }) => {
            mode == other_mode
                && points.len() == other_points.len()
                && (points.iter().zip(other_points).all(|(a, b)| a.dist(*b) <= EPS)
                    || points.iter().zip(other_points.iter().rev()).all(|(a, b)| a.dist(*b) <= EPS))
        }
        (EntityKind::Point { p }, EntityKind::Point { p: q }) => p.dist(*q) <= EPS,
        (EntityKind::Line { a, b }, EntityKind::Line { a: c, b: d }) => {
            (a.dist(*c) <= EPS && b.dist(*d) <= EPS) || (a.dist(*d) <= EPS && b.dist(*c) <= EPS)
        }
        (EntityKind::Circle { center, radius }, EntityKind::Circle { center: c, radius: r }) => {
            center.dist(*c) <= EPS && (radius - r).abs() <= EPS
        }
        (EntityKind::Arc { center, start, end }, EntityKind::Arc { center: c, start: a, end: b }) => {
            center.dist(*c) <= EPS && start.dist(*a) <= EPS && end.dist(*b) <= EPS
        }
        _ => false,
    }
}

impl Sketch {
    pub(super) fn mirror_linked(&mut self, ids: &[EntityId], axis: MirrorAxis) -> Result<Vec<EntityId>> {
        let selected = self.tool_selection(ids)?;
        if selected.iter().any(|e| matches!(e.kind, EntityKind::Text { .. })) {
            return refuse(
                "Linked mirror does not support sketch text; turn off Keep symmetry for an independent copy",
            );
        }
        let (start, end, existing) = match axis {
            MirrorAxis::Entity { entity } => match self.tool_entity(entity)?.kind {
                EntityKind::Line { a, b } => (a, b, Some(entity)),
                _ => return refuse("the mirror axis must be a line"),
            },
            MirrorAxis::Points { start, end } => (start, end, None),
        };
        let mut pairs = Vec::new();
        let mut retained = Vec::new();
        for e in &selected {
            if Some(e.id) == existing {
                return refuse("the mirror axis cannot also be copied");
            }
            let kind = reflected(e.kind.clone(), start, end)?;
            if let Some(partner) =
                selected.iter().find(|source| source.construction == e.construction && same(&source.kind, &kind))
            {
                if e.id <= partner.id {
                    retained.push((e.clone(), partner.id));
                }
            } else {
                pairs.push((e.clone(), kind));
            }
        }
        let mut trial = self.clone();
        let axis = match existing {
            Some(id) => id,
            None => {
                let kind = EntityKind::Line { a: start, b: end };
                let id = trial.add(kind.clone(), true);
                trial.constrain(ConstraintKind::FixEntity { entity: id, at: kind });
                id
            }
        };
        for (source, partner) in retained {
            if source.id != partner {
                trial.mirror_pair(source, partner, axis, start, end)?;
                continue;
            }
            let kinds = {
                let point = |index| PointRef::new(source.id, index);
                match source.kind {
                    EntityKind::Spline { ref points, .. } => {
                        let direction = (end - start).normalized();
                        let reflect = |p: P2| start + direction.scale(2.0 * (p - start).dot(direction)) - (p - start);
                        let on_axis = points.iter().all(|p| p.dist(reflect(*p)) <= EPS);
                        (0..points.len())
                            .filter_map(|i| {
                                let j = if on_axis { i } else { points.len() - 1 - i };
                                if i == j {
                                    Some(ConstraintKind::PointOn { point: point(i), entity: axis })
                                } else {
                                    (i < j).then_some(ConstraintKind::Symmetric { a: point(i), b: point(j), axis })
                                }
                            })
                            .collect()
                    }
                    EntityKind::Point { .. } | EntityKind::Circle { .. } => {
                        vec![ConstraintKind::PointOn { point: point(0), entity: axis }]
                    }
                    EntityKind::Line { a, .. } if (a - start).cross(end - start).abs() / start.dist(end) <= EPS => {
                        vec![
                            ConstraintKind::PointOn { point: point(0), entity: axis },
                            ConstraintKind::PointOn { point: point(1), entity: axis },
                        ]
                    }
                    EntityKind::Line { .. } => vec![ConstraintKind::Symmetric { a: point(0), b: point(1), axis }],
                    EntityKind::Arc { .. } => vec![
                        ConstraintKind::PointOn { point: point(0), entity: axis },
                        ConstraintKind::Symmetric { a: point(1), b: point(2), axis },
                    ],
                    _ => unreachable!("reflection validates supported curves"),
                }
            };
            for kind in kinds {
                if !trial.constraints.iter().any(|c| c.kind == kind) {
                    trial.constrain(kind);
                }
            }
        }
        let copies = trial
            .add_tool_entities(pairs.iter().map(|(source, kind)| (kind.clone(), source.construction)).collect())?;
        for ((source, _), &copy) in pairs.iter().zip(&copies) {
            trial.mirror_pair(source.clone(), copy, axis, start, end)?;
        }
        *self = trial;
        Ok(copies)
    }

    fn mirror_pair(&mut self, source: Entity, partner: EntityId, axis: EntityId, start: P2, end: P2) -> Result<()> {
        let kinds = self.relation_constraints(&Relation::SymmetricEntities { a: source.id, b: partner, axis })?;
        for kind in &kinds {
            if !self.constraints.iter().any(|c| c.kind == *kind) {
                self.constrain(kind.clone());
            }
        }
        for kind in kinds {
            let ConstraintKind::Symmetric { a, b, .. } = kind else { continue };
            let endpoint = match source.kind {
                EntityKind::Spline { ref points, .. } => a.index == 0 || a.index == points.len() - 1,
                EntityKind::Line { .. } => a.index < 2,
                EntityKind::Arc { .. } => a.index == 1 || a.index == 2,
                _ => false,
            };
            if endpoint
                && let Some(p) = source.point(a.index)
                && (p - start).cross(end - start).abs() / start.dist(end) <= EPS
            {
                self.constrain(ConstraintKind::PointOn { point: a, entity: axis });
                self.constrain(ConstraintKind::Coincident { a, b });
            }
        }
        Ok(())
    }
}
