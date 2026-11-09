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
    if !valid(kind) || !finite(start) || !finite(end) || start.dist(end) <= EPS {
        return refuse("reflection needs valid geometry and two distinct finite axis points");
    }
    let dir = (end - start).normalized();
    let map = |p: P2| start + dir.scale(2.0 * (p - start).dot(dir)) - (p - start);
    let kind = match kind {
        EntityKind::Point { p } => EntityKind::Point { p: map(p) },
        EntityKind::Line { a, b } => EntityKind::Line { a: map(a), b: map(b) },
        EntityKind::Circle { center, radius } => EntityKind::Circle { center: map(center), radius },
        EntityKind::Arc { center, start, end } => {
            EntityKind::Arc { center: map(center), start: map(end), end: map(start) }
        }
        EntityKind::Ellipse { .. } | EntityKind::EllipticArc { .. } => {
            return refuse("linked mirror supports points, lines, circles and circular arcs");
        }
    };
    if !valid(kind) {
        return refuse("reflection would produce invalid geometry");
    }
    Ok(kind)
}

fn same(a: EntityKind, b: EntityKind) -> bool {
    match (a, b) {
        (EntityKind::Point { p }, EntityKind::Point { p: q }) => p.dist(q) <= EPS,
        (EntityKind::Line { a, b }, EntityKind::Line { a: c, b: d }) => {
            (a.dist(c) <= EPS && b.dist(d) <= EPS) || (a.dist(d) <= EPS && b.dist(c) <= EPS)
        }
        (EntityKind::Circle { center, radius }, EntityKind::Circle { center: c, radius: r }) => {
            center.dist(c) <= EPS && (radius - r).abs() <= EPS
        }
        (EntityKind::Arc { center, start, end }, EntityKind::Arc { center: c, start: a, end: b }) => {
            center.dist(c) <= EPS && start.dist(a) <= EPS && end.dist(b) <= EPS
        }
        _ => false,
    }
}

impl Sketch {
    pub(super) fn mirror_linked(&mut self, ids: &[EntityId], axis: MirrorAxis) -> Result<Vec<EntityId>> {
        let selected = self.tool_selection(ids)?;
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
            let kind = reflected(e.kind, start, end)?;
            if let Some(partner) = selected.iter().find(|source| same(source.kind, kind)) {
                if e.id <= partner.id {
                    retained.push((*e, partner.id));
                }
            } else {
                pairs.push((*e, kind));
            }
        }
        let mut trial = self.clone();
        let axis = match existing {
            Some(id) => id,
            None => {
                let kind = EntityKind::Line { a: start, b: end };
                let id = trial.add(kind, true);
                trial.constrain(ConstraintKind::FixEntity { entity: id, at: kind });
                id
            }
        };
        for (source, partner) in retained {
            let kinds = if source.id != partner {
                trial.relation_constraints(&Relation::SymmetricEntities { a: source.id, b: partner, axis })?
            } else {
                let point = |index| PointRef::new(source.id, index);
                match source.kind {
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
        let copies =
            trial.add_tool_entities(pairs.iter().map(|(source, kind)| (*kind, source.construction)).collect())?;
        for ((source, _), &copy) in pairs.iter().zip(&copies) {
            for kind in trial.relation_constraints(&Relation::SymmetricEntities { a: source.id, b: copy, axis })? {
                trial.constrain(kind);
            }
            // A shared boundary on the axis keeps a mirrored profile closed during later edits.
            if !source.is_point() && !matches!(source.kind, EntityKind::Circle { .. }) {
                for index in if source.is_line() { 0..2 } else { 1..3 } {
                    let p = source.point(index).expect("curve endpoint");
                    if (p - start).cross(end - start).abs() / start.dist(end) <= EPS {
                        let original = PointRef::new(source.id, index);
                        let target_index = if source.is_line() { index } else { 3 - index };
                        let kind = ConstraintKind::PointOn { point: original, entity: axis };
                        if !trial.constraints.iter().any(|c| c.kind == kind) {
                            trial.constrain(kind);
                        }
                        trial.constrain(ConstraintKind::Coincident {
                            a: original,
                            b: PointRef::new(copy, target_index),
                        });
                    }
                }
            }
        }
        *self = trial;
        Ok(copies)
    }
}
