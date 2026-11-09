//! Concentric-circle projections keep ellipse points in the constraint solver.

use super::*;
use crate::ellipse::Ellipse;

#[derive(Clone, Copy)]
pub(super) struct EllipseDatum {
    pub center: DatumPoint,
    pub first: DatumLineSegment,
    pub second: DatumLineSegment,
    geometry: Ellipse,
}

impl Vars {
    fn auxiliary(&mut self, at: P2) -> DatumPoint {
        let p = DatumPoint::new(&mut self.ids);
        self.guesses.extend([(p.id_x(), at.u), (p.id_y(), at.v)]);
        p
    }
}

pub(super) fn datum(vars: &mut Vars, e: &Entity) -> (EllipseDatum, Vec<Constraint>) {
    let geometry = Ellipse::of(&e.kind).unwrap();
    let c = vars.point(PointRef::new(e.id, 0), geometry.center);
    let a = vars.point(PointRef::new(e.id, 1), geometry.major);
    let b = vars.point(PointRef::new(e.id, 2), geometry.minor);
    let first = DatumLineSegment::new(c, a);
    let second = DatumLineSegment::new(c, b);
    let result = EllipseDatum { center: c, first, second, geometry };
    let mut reqs = vec![Constraint::LinesAtAngle(first, second, AngleKind::Perpendicular)];
    if let EntityKind::EllipticArc { start, end, .. } = e.kind {
        for (i, at) in [(3, start), (4, end)] {
            let p = vars.point(PointRef::new(e.id, i), at);
            reqs.extend(result.point_on(vars, p, at));
        }
    }
    (result, reqs)
}

impl EllipseDatum {
    pub fn point_on(self, vars: &mut Vars, p: DatumPoint, at: P2) -> Vec<Constraint> {
        let g = self.geometry;
        let angle = g.parameter(at);
        let radial = g.axis().normalized().scale(angle.cos()) + g.axis().perp().normalized().scale(angle.sin());
        let q_at = g.center + radial.scale(g.center.dist(g.major));
        let r_at = g.center + radial.scale(g.center.dist(g.minor));
        let q = vars.auxiliary(q_at);
        let r = vars.auxiliary(r_at);
        let q_shift = vars.auxiliary(q_at + (g.minor - g.center));
        let r_shift = vars.auxiliary(r_at + g.axis());
        let first_circle = DatumLineSegment::new(self.center, q);
        let second_circle = DatumLineSegment::new(self.center, r);
        let vertical = DatumLineSegment::new(q, q_shift);
        let horizontal = DatumLineSegment::new(r, r_shift);
        // Q and R share a radial line on circles with the two semi-axis radii.
        // Their projections give P=(a*cos(t), b*sin(t)) in the ellipse frame.
        // Shifted reference lines stay nonzero at axis endpoints and when a=b.
        vec![
            Constraint::LinesEqualLength(first_circle, self.first),
            Constraint::LinesEqualLength(second_circle, self.second),
            Constraint::PointLineDistance(r, first_circle, 0.0),
            Constraint::LinesAtAngle(vertical, self.second, AngleKind::Parallel),
            Constraint::LinesEqualLength(vertical, self.second),
            Constraint::PointLineDistance(p, vertical, 0.0),
            Constraint::LinesAtAngle(horizontal, self.first, AngleKind::Parallel),
            Constraint::LinesEqualLength(horizontal, self.first),
            Constraint::PointLineDistance(p, horizontal, 0.0),
        ]
    }
}
