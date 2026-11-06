//! Value-free reference dimensions measured from current geometry.

use crate::{
    Constraint, ConstraintId, ConstraintKind, DistanceKind, EntityId, EntityKind, P2, PointRef, Sketch, SolveStatus,
};
use serde::{Deserialize, Serialize};

/// Geometry measured by a reference dimension. It stores no driving value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReferenceDimension {
    /// Separation of two points.
    Distance {
        /// First point.
        a: PointRef,
        /// Second point.
        b: PointRef,
        /// Aligned or projected separation.
        kind: DistanceKind,
    },
    /// Perpendicular distance to the infinite line.
    PointLineDistance {
        /// Measured point.
        point: PointRef,
        /// Line target.
        line: EntityId,
    },
    /// Angle between two line directions, modulo 180 degrees.
    Angle {
        /// First line.
        a: EntityId,
        /// Second line.
        b: EntityId,
    },
    /// Radius or diameter of a circle or arc.
    Radius {
        /// Circular target.
        entity: EntityId,
        /// Measure diameter when true.
        diameter: bool,
    },
}

impl ReferenceDimension {
    fn driving(self, value: f64, label: P2) -> ConstraintKind {
        match self {
            Self::Distance { a, b, kind } => ConstraintKind::Distance { a, b, kind, value, label },
            Self::PointLineDistance { point, line } => ConstraintKind::PointLineDistance { point, line, value, label },
            Self::Angle { a, b } => ConstraintKind::Angle { a, b, value, label },
            Self::Radius { entity, diameter } => ConstraintKind::Radius { entity, diameter, value, label },
        }
    }

    fn measure(self, sketch: &Sketch) -> Option<f64> {
        let line = |id| match sketch.entity(id)?.kind {
            EntityKind::Line { a, b } if a.dist(b).is_finite() && a.dist(b) > 0.0 => Some((a, b)),
            _ => None,
        };
        let value = match self {
            Self::Distance { a, b, kind } => {
                let (a, b) = (sketch.point(a)?, sketch.point(b)?);
                match kind {
                    DistanceKind::Aligned => a.dist(b),
                    DistanceKind::Horizontal => (a.u - b.u).abs(),
                    DistanceKind::Vertical => (a.v - b.v).abs(),
                }
            }
            Self::PointLineDistance { point, line: id } => {
                let (a, b) = line(id)?;
                (sketch.point(point)? - a).cross(b - a).abs() / a.dist(b)
            }
            Self::Angle { a, b } => {
                let ((a0, a1), (b0, b1)) = (line(a)?, line(b)?);
                ((b1 - b0).angle() - (a1 - a0).angle()).to_degrees().rem_euclid(180.0)
            }
            Self::Radius { entity, diameter } => {
                let target = sketch.entity(entity)?;
                if target.points().iter().any(|p| !p.u.is_finite() || !p.v.is_finite()) {
                    return None;
                }
                let radius = match target.kind {
                    EntityKind::Circle { radius, .. } => radius,
                    EntityKind::Arc { center, start, end } => {
                        let radius = center.dist(start);
                        if (radius - center.dist(end)).abs() > 1e-6_f64.max(radius * 1e-9) {
                            return None;
                        }
                        radius
                    }
                    _ => return None,
                };
                if radius <= 0.0 {
                    return None;
                }
                radius * if diameter { 2.0 } else { 1.0 }
            }
        };
        value.is_finite().then_some(value)
    }
}

impl ConstraintKind {
    /// The same numeric annotation without a driving value, or `None` for a relation.
    pub fn as_reference(self) -> Option<Self> {
        let c = Constraint { id: 0, kind: self };
        Some(Self::Reference { dimension: c.dimension()?, label: c.label()? })
    }
}

impl Constraint {
    fn dimension(&self) -> Option<ReferenceDimension> {
        match self.kind {
            ConstraintKind::Distance { a, b, kind, .. } => Some(ReferenceDimension::Distance { a, b, kind }),
            ConstraintKind::PointLineDistance { point, line, .. } => {
                Some(ReferenceDimension::PointLineDistance { point, line })
            }
            ConstraintKind::Angle { a, b, .. } => Some(ReferenceDimension::Angle { a, b }),
            ConstraintKind::Radius { entity, diameter, .. } => Some(ReferenceDimension::Radius { entity, diameter }),
            ConstraintKind::Reference { dimension, .. } => Some(dimension),
            _ => None,
        }
    }

    /// Whether this annotation measures instead of driving geometry.
    pub fn is_reference(&self) -> bool {
        matches!(self.kind, ConstraintKind::Reference { .. })
    }

    /// The annotation's geometry in the driving representation. Its zero placeholder
    /// is not a measured value; use `display_value` for a value to show.
    pub fn geometry_kind(&self) -> ConstraintKind {
        match self.kind {
            ConstraintKind::Reference { dimension, label } => dimension.driving(0.0, label),
            kind => kind,
        }
    }

    /// The current geometric measurement, independent of any driving target.
    pub fn measured_value(&self, sketch: &Sketch) -> Option<f64> {
        self.dimension()?.measure(sketch)
    }

    /// The authored driving target or the current reference measurement.
    pub fn display_value(&self, sketch: &Sketch) -> Option<f64> {
        if self.is_reference() { self.measured_value(sketch) } else { self.value() }
    }
}

impl Sketch {
    /// Switch dimension mode atomically, preserving its ID and label. Driving uses
    /// the current measurement and must leave a satisfiable sketch.
    pub fn set_dimension_reference(&mut self, id: ConstraintId, reference: bool) -> Result<(), String> {
        let c = *self.constraint(id).ok_or("No such dimension")?;
        let dimension = c.dimension().ok_or("Only a numeric dimension can be reference")?;
        if c.is_reference() == reference {
            return Ok(());
        }
        let value = c.measured_value(self).ok_or("Cannot measure the dimension targets")?;
        if !reference && !matches!(dimension, ReferenceDimension::Angle { .. }) && value <= 0.0 {
            return Err("A driving length must be greater than zero".into());
        }
        let label = c.label().expect("dimension label");
        let kind =
            if reference { ConstraintKind::Reference { dimension, label } } else { dimension.driving(value, label) };
        let mut trial = self.clone();
        trial.constraint_mut(id).expect("existing dimension").kind = kind;
        crate::solve::solve(&mut trial, None, true);
        if matches!(trial.status, SolveStatus::Failed(_))
            || (!reference && !matches!(trial.status, SolveStatus::UnderDefined | SolveStatus::FullyDefined))
        {
            return Err("The dimension mode change leaves an unsolved or conflicting sketch".into());
        }
        *self = trial;
        Ok(())
    }
}
