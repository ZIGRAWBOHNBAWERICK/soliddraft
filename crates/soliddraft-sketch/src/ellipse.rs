//! Ellipse evaluation in the sketch plane.

use crate::{EntityKind, P2};
use std::f64::consts::TAU;

/// An ellipse with perpendicular semi-axis handles.
#[derive(Clone, Copy, Debug)]
pub struct Ellipse {
    /// Centre.
    pub center: P2,
    /// First semi-axis endpoint.
    pub major: P2,
    /// Second semi-axis endpoint.
    pub minor: P2,
}

impl Ellipse {
    /// Ellipse geometry of a full or partial ellipse entity.
    pub fn of(kind: &EntityKind) -> Option<Self> {
        match kind {
            EntityKind::Ellipse { center, major, minor } | EntityKind::EllipticArc { center, major, minor, .. } => {
                Some(Self { center: *center, major: *major, minor: *minor })
            }
            _ => None,
        }
    }

    /// First semi-axis vector.
    pub fn axis(self) -> P2 {
        self.major - self.center
    }

    /// Positive second-to-first semi-axis ratio.
    pub fn ratio(self) -> f64 {
        self.center.dist(self.minor) / self.axis().len()
    }

    /// Point at eccentric parameter `angle`, in radians.
    pub fn at(self, angle: f64) -> P2 {
        let axis = self.axis();
        self.center + axis.scale(angle.cos()) + axis.perp().scale(self.ratio() * angle.sin())
    }

    /// Eccentric parameter of the radial projection of `p`.
    pub fn parameter(self, p: P2) -> f64 {
        let d = p - self.center;
        let a = self.axis();
        (a.cross(d) / self.ratio()).atan2(a.dot(d))
    }

    /// Counterclockwise start parameter and positive sweep, or one full turn.
    pub fn span(self, kind: &EntityKind) -> (f64, f64) {
        if let EntityKind::EllipticArc { start, end, .. } = kind {
            let start = self.parameter(*start);
            let sweep = (self.parameter(*end) - start).rem_euclid(TAU);
            (start, sweep)
        } else {
            (0.0, TAU)
        }
    }

    /// Nonzero finite perpendicular axes.
    pub fn valid(self) -> bool {
        let a = self.axis();
        let b = self.minor - self.center;
        [self.center, self.major, self.minor].iter().all(|p| p.u.is_finite() && p.v.is_finite())
            && a.len().min(b.len()) > 1e-7
            && a.dot(b).abs() <= 1e-7 * a.len().max(b.len())
    }

    /// Closest point on a full or partial ellipse, found over every stationary interval.
    pub fn nearest(self, kind: &EntityKind, p: P2) -> (P2, f64) {
        let (start, sweep) = self.span(kind);
        let distance = |t| self.at(start + t).dist(p);
        let mut best = (0.0, distance(0.0));
        let steps = 64;
        for i in 0..steps {
            let mut lo = sweep * i as f64 / steps as f64;
            let mut hi = sweep * (i + 1) as f64 / steps as f64;
            for _ in 0..55 {
                let a = lo + (hi - lo) / 3.0;
                let b = hi - (hi - lo) / 3.0;
                if distance(a) < distance(b) {
                    hi = b;
                } else {
                    lo = a;
                }
            }
            for t in [lo, hi] {
                let d = distance(t);
                if d < best.1 {
                    best = (t, d);
                }
            }
        }
        if distance(sweep) < best.1 {
            best = (sweep, distance(sweep));
        }
        (self.at(start + best.0), best.1)
    }
}
