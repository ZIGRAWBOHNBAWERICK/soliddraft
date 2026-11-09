//! Selection-level relations shared by the menu, property panel and sketch operations.

use serde::{Deserialize, Serialize};

use crate::{ConstraintKind, Entity, EntityId, EntityKind, ORIGIN, P2, PointRef, Sketch, tools};
use tools::ToolError;

/// A relation request. Composite choices expand into ordinary stored constraints.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Relation {
    /// Keep two lines on one infinite line.
    Collinear {
        /// First line.
        a: EntityId,
        /// Second line.
        b: EntityId,
    },
    /// Keep two circular curves at one centre and radius.
    Coradial {
        /// First curve.
        a: EntityId,
        /// Second curve.
        b: EntityId,
    },
    /// Keep a point on two infinite lines.
    Intersection {
        /// Point at the crossing.
        point: PointRef,
        /// First line.
        a: EntityId,
        /// Second line.
        b: EntityId,
    },
    /// Reflect two points across a line.
    SymmetricPoints {
        /// First point.
        a: PointRef,
        /// Second point.
        b: PointRef,
        /// Reflection axis, selected last.
        axis: EntityId,
    },
    /// Reflect matching entities across a line.
    SymmetricEntities {
        /// First entity.
        a: EntityId,
        /// Second entity of the same type.
        b: EntityId,
        /// Reflection axis, selected last.
        axis: EntityId,
    },
    /// Fix all coordinates and the radius of one entity.
    FixEntity {
        /// Entity to hold in place.
        entity: EntityId,
    },
}

/// Whether these entity types have a tangent relation. Spline tangency needs
/// joined endpoints, so its derivative is evaluated at a specified contact.
pub fn tangent_supported(sketch: &Sketch, a: EntityId, b: EntityId) -> bool {
    let (Some(a), Some(b)) = (sketch.entity(a), sketch.entity(b)) else {
        return false;
    };
    if a.id == b.id {
        return false;
    }
    let spline = |entity: &Entity| matches!(entity.kind, EntityKind::Spline { .. });
    let circular = |entity: &Entity| entity.is_curve();
    if spline(a) || spline(b) {
        let endpoint_curve = |entity: &Entity| {
            matches!(entity.kind, EntityKind::Spline { .. } | EntityKind::Line { .. } | EntityKind::Arc { .. })
        };
        endpoint_curve(a) && endpoint_curve(b) && crate::solve::shared_point(sketch, a.id, b.id).is_some()
    } else {
        (a.is_line() && circular(b)) || (circular(a) && b.is_line()) || (circular(a) && circular(b))
    }
}

impl Sketch {
    /// Validate a selection and produce its relations without changing the sketch.
    pub fn relation_constraints(&self, relation: &Relation) -> Result<Vec<ConstraintKind>, ToolError> {
        let entity = |id| -> Result<&Entity, ToolError> {
            self.entity(id)
                .filter(|e| tools::valid(&e.kind))
                .ok_or_else(|| ToolError("Select valid existing geometry".into()))
        };
        let line = |id| -> Result<(P2, P2), ToolError> {
            match entity(id)?.kind {
                EntityKind::Line { a, b } => Ok((a, b)),
                _ => Err(ToolError("Select a line".into())),
            }
        };
        let distinct = |a, b| {
            if a == b { Err(ToolError("Select distinct entities".into())) } else { Ok(()) }
        };
        let point = |p: PointRef| -> Result<(), ToolError> {
            entity(p.entity)?;
            self.point(p).map(|_| ()).ok_or_else(|| ToolError("Select an existing point".into()))
        };
        let symmetry = |a: PointRef, b: PointRef, axis| -> Result<ConstraintKind, ToolError> {
            line(axis)?;
            point(a)?;
            point(b)?;
            if a == b || a.entity == axis || b.entity == axis {
                return Err(ToolError("Select two distinct points, then a separate axis line".into()));
            }
            Ok(ConstraintKind::Symmetric { a, b, axis })
        };
        Ok(match *relation {
            Relation::Collinear { a, b } => {
                distinct(a, b)?;
                line(a)?;
                line(b)?;
                vec![
                    ConstraintKind::PointOn { point: PointRef::new(b, 0), entity: a },
                    ConstraintKind::PointOn { point: PointRef::new(b, 1), entity: a },
                ]
            }
            Relation::Coradial { a, b } => {
                distinct(a, b)?;
                if tools::circle(&entity(a)?.kind).is_none() || tools::circle(&entity(b)?.kind).is_none() {
                    return Err(ToolError("Select two arcs or circles".into()));
                }
                vec![ConstraintKind::Concentric { a, b }, ConstraintKind::Equal { a, b }]
            }
            Relation::Intersection { point: p, a, b } => {
                distinct(a, b)?;
                line(a)?;
                line(b)?;
                point(p)?;
                [a, b]
                    .into_iter()
                    .filter(|id| *id != p.entity)
                    .map(|entity| ConstraintKind::PointOn { point: p, entity })
                    .collect()
            }
            Relation::SymmetricPoints { a, b, axis } => vec![symmetry(a, b, axis)?],
            Relation::SymmetricEntities { a, b, axis } => {
                distinct(a, b)?;
                let first = entity(a)?;
                let second = entity(b)?;
                let (start, end) = line(axis)?;
                if std::mem::discriminant(&first.kind) != std::mem::discriminant(&second.kind) {
                    return Err(ToolError("Select two entities of the same type, then the axis line".into()));
                }
                let direction = (end - start).normalized();
                let reflect = |p: P2| start + direction.scale(2.0 * (p - start).dot(direction)) - (p - start);
                let pairs: Vec<(usize, usize)> = match first.kind {
                    EntityKind::Spline { mode, ref points } => {
                        let EntityKind::Spline { mode: other_mode, points: ref other_points } = second.kind else {
                            unreachable!()
                        };
                        if mode != other_mode || points.len() != other_points.len() {
                            return Err(ToolError("Select splines with the same mode and point count".into()));
                        }
                        let direct = points.iter().zip(other_points).map(|(a, b)| reflect(*a).dist(*b)).sum::<f64>();
                        let reversed = points
                            .iter()
                            .zip(other_points.iter().rev())
                            .map(|(a, b)| reflect(*a).dist(*b))
                            .sum::<f64>();
                        (0..points.len())
                            .map(|i| (i, if direct <= reversed { i } else { points.len() - 1 - i }))
                            .collect()
                    }
                    EntityKind::Point { .. } | EntityKind::Circle { .. } => vec![(0, 0)],
                    EntityKind::Arc { .. } => vec![(0, 0), (1, 2), (2, 1)],
                    EntityKind::Ellipse { .. } => vec![(0, 0), (1, 1), (2, 2)],
                    EntityKind::EllipticArc { .. } => vec![(0, 0), (1, 1), (2, 2), (3, 4), (4, 3)],
                    EntityKind::Line { a, b } => {
                        let (c, d) = line(second.id)?;
                        // Store the nearest endpoint correspondence once, so later drags cannot swap it.
                        if reflect(a).dist(c) + reflect(b).dist(d) <= reflect(a).dist(d) + reflect(b).dist(c) {
                            vec![(0, 0), (1, 1)]
                        } else {
                            vec![(0, 1), (1, 0)]
                        }
                    }
                };
                let mut kinds = pairs
                    .into_iter()
                    .map(|(i, j)| symmetry(PointRef::new(a, i), PointRef::new(b, j), axis))
                    .collect::<Result<Vec<_>, _>>()?;
                if matches!(first.kind, EntityKind::Circle { .. }) {
                    kinds.push(ConstraintKind::Equal { a, b });
                }
                kinds
            }
            Relation::FixEntity { entity: id } => {
                if id == ORIGIN {
                    return Err(ToolError("The origin is already fixed".into()));
                }
                let e = entity(id)?;
                vec![
                    self.constraints
                        .iter()
                        .find_map(|c| match c.kind {
                            ConstraintKind::FixEntity { entity, .. } if entity == id => Some(c.kind.clone()),
                            _ => None,
                        })
                        .unwrap_or(ConstraintKind::FixEntity { entity: id, at: e.kind.clone() }),
                ]
            }
        })
    }
}
