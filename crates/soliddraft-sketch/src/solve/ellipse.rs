//! Focal construction keeps ellipse endpoints and point-on relations in the solver.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct EllipseDatum {
    pub center: DatumPoint,
    pub first: DatumLineSegment,
    pub second: DatumLineSegment,
    foci: Option<(DatumPoint, DatumPoint, DatumPoint, P2, P2, P2, P2)>,
}

impl Vars {
    fn auxiliary(&mut self, at: P2) -> DatumPoint {
        let p = DatumPoint::new(&mut self.ids);
        self.guesses.extend([(p.id_x(), at.u), (p.id_y(), at.v)]);
        p
    }
}

pub(super) fn datum(vars: &mut Vars, e: &Entity, need_foci: bool) -> (EllipseDatum, Vec<Constraint>) {
    let geometry = crate::ellipse::Ellipse::of(e.kind).unwrap();
    let c = vars.point(PointRef::new(e.id, 0), geometry.center);
    let a = vars.point(PointRef::new(e.id, 1), geometry.major);
    let b = vars.point(PointRef::new(e.id, 2), geometry.minor);
    let first = DatumLineSegment::new(c, a);
    let second = DatumLineSegment::new(c, b);
    let mut reqs = vec![Constraint::LinesAtAngle(first, second, AngleKind::Perpendicular)];
    let mut result = EllipseDatum { center: c, first, second, foci: None };
    if need_foci {
        let (long, short, axis, minor) = if geometry.center.dist(geometry.major) >= geometry.center.dist(geometry.minor)
        {
            (geometry.major, geometry.minor, a, b)
        } else {
            (geometry.minor, geometry.major, b, a)
        };
        let direction = (long - geometry.center).normalized();
        let focal = (geometry.center.dist(long).powi(2) - geometry.center.dist(short).powi(2)).max(0.0).sqrt();
        let f1_at = geometry.center + direction.scale(focal);
        let f2_at = geometry.center - direction.scale(focal);
        let double_at = long + (long - geometry.center);
        let f1 = vars.auxiliary(f1_at);
        let f2 = vars.auxiliary(f2_at);
        let double = vars.auxiliary(double_at);
        reqs.extend([
            Constraint::PointLineDistance(f1, DatumLineSegment::new(c, axis), 0.0),
            Constraint::LinesEqualLength(DatumLineSegment::new(minor, f1), DatumLineSegment::new(c, axis)),
            Constraint::Midpoint(DatumLineSegment::new(f1, f2), c),
            Constraint::Midpoint(DatumLineSegment::new(c, double), axis),
        ]);
        result.foci = Some((f1, f2, double, geometry.center, long, f1_at, f2_at));
        if let EntityKind::EllipticArc { start, end, .. } = e.kind {
            for (i, at) in [(3, start), (4, end)] {
                let p = vars.point(PointRef::new(e.id, i), at);
                reqs.extend(result.point_on(vars, p, at));
            }
        }
    }
    (result, reqs)
}

impl EllipseDatum {
    pub fn point_on(self, vars: &mut Vars, p: DatumPoint, at: P2) -> Vec<Constraint> {
        let Some((f1, f2, double, c, long, f1_at, _)) = self.foci else { return Vec::new() };
        let u_at = c + (long - c).normalized().scale(at.dist(f1_at));
        let u = vars.auxiliary(u_at);
        vec![
            Constraint::PointLineDistance(u, DatumLineSegment::new(self.center, double), 0.0),
            Constraint::LinesEqualLength(DatumLineSegment::new(self.center, u), DatumLineSegment::new(p, f1)),
            Constraint::LinesEqualLength(DatumLineSegment::new(u, double), DatumLineSegment::new(p, f2)),
        ]
    }
}
