//! Bent cutting lines: offset sections, whose parallel cuts are joined by jogs that
//! cut no material, and aligned sections, whose second cut is revolved about the pivot
//! into the plane of the first (ISO 128-44, ASME Y14.3).
//!
//! Each cut is an ordinary planar section: the kernel's certified section for an export,
//! its mesh section for the screen. This module keeps each cut's line work to the cut's
//! own stretch of the unfolded line, its *band*, and turns a revolved cut into the first
//! cut's plane. A curve in two bands is split exactly where it leaves one; a curve on
//! the boundary between two bands belongs to the later one only, so nothing is drawn
//! twice. The jogs of an offset cut and the pivot of an aligned cut draw no line.

use soliddraft_kernel_api::drawing::{Curve3, CutRegion, SectionView, ViewCurve, ViewCurves, ViewFrame, perpendicular};
use soliddraft_kernel_api::{KernelResult, Point2, Vec3};

const MIN_SPAN: f64 = 1e-9;
const ANGLE_TOL: f64 = 1e-9;
/// Stations closer than this to a band boundary, relative to the cut's length, are on
/// it.
const BOUNDARY_TOL: f64 = 1e-9;

/// What a cutting line is, by its number of points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CutKind {
    /// Two points: one plane.
    Straight,
    /// An even number of points from four: parallel cuts joined by perpendicular jogs.
    Offset,
    /// Three points: two cuts meeting at the middle point, the second revolved into the
    /// plane of the first.
    Aligned,
}

impl CutKind {
    /// Two points make a straight cut, three an aligned cut about the middle point, and
    /// an even number from four an offset cut.
    pub(crate) fn of(points: usize) -> Option<CutKind> {
        match points {
            2 => Some(CutKind::Straight),
            3 => Some(CutKind::Aligned),
            n if n >= 4 && n.is_multiple_of(2) => Some(CutKind::Offset),
            _ => None,
        }
    }
}

/// Why a bent cutting line cannot make a section.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CutPathError {
    PointCount,
    NonFinite,
    ShortSpan,
    OffsetDirection,
    AlignedDirection,
}

impl std::fmt::Display for CutPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CutPathError::PointCount => {
                "a bent cutting line has three points (aligned) or an even number from four (offset)"
            }
            CutPathError::NonFinite => "a cutting line point is not finite",
            CutPathError::ShortSpan => "a stretch of the cutting line has no length",
            CutPathError::OffsetDirection => {
                "an offset cutting line alternates parallel cuts, all one way, with jogs square to them"
            }
            CutPathError::AlignedDirection => "the two legs of an aligned cutting line must not be in line",
        })
    }
}

/// A bent cutting line in a parent view's plane, split into the stretches that cut
/// material.
#[derive(Debug)]
pub(crate) struct CutPath {
    kind: CutKind,
    spans: Vec<CutSpan>,
}

/// One stretch of a bent cutting line that cuts material.
#[derive(Debug)]
pub(crate) struct CutSpan {
    /// The trace point it starts at.
    from: usize,
    tangent: Point2,
    arrow: Point2,
    start: f64,
    end: f64,
}

impl CutPath {
    /// The cut along `trace`, its kind told by its number of points. `flip` turns the
    /// arrows round, as for a straight section.
    pub(crate) fn of_line(trace: &[Point2], flip: bool) -> Result<Self, CutPathError> {
        match CutKind::of(trace.len()) {
            Some(kind @ (CutKind::Offset | CutKind::Aligned)) => Self::new(kind, trace, flip),
            _ => Err(CutPathError::PointCount),
        }
    }

    pub(crate) fn new(kind: CutKind, trace: &[Point2], flip: bool) -> Result<Self, CutPathError> {
        let span_count = match kind {
            CutKind::Offset if trace.len() >= 4 && trace.len().is_multiple_of(2) => trace.len() / 2,
            CutKind::Aligned if trace.len() == 3 => 2,
            _ => return Err(CutPathError::PointCount),
        };
        let mut segments = trace.windows(2);
        let first = segments.next().expect("at least three points");
        let (axis, length) = segment(first[0], first[1])?;
        // Match the straight section's default side, then carry it through each bend.
        let reversed = axis.u < -ANGLE_TOL || (axis.u.abs() <= ANGLE_TOL && axis.v < 0.0);
        let side = if reversed ^ flip { -1.0 } else { 1.0 };
        let mut spans = Vec::with_capacity(span_count);
        spans.push(CutSpan::new(0, axis, side, 0.0, length));
        let mut station = length;
        for (i, pair) in segments.enumerate() {
            let (tangent, length) = segment(pair[0], pair[1])?;
            let dot = axis.u * tangent.u + axis.v * tangent.v;
            let cross = axis.u * tangent.v - axis.v * tangent.u;
            let advance = match kind {
                CutKind::Offset if i.is_multiple_of(2) => {
                    if dot.abs() > ANGLE_TOL {
                        return Err(CutPathError::OffsetDirection);
                    }
                    continue; // A jog is visible on paper but cuts no material.
                }
                CutKind::Offset if dot <= 0.0 || cross.abs() > ANGLE_TOL => {
                    return Err(CutPathError::OffsetDirection);
                }
                CutKind::Offset => length * dot,
                CutKind::Aligned if cross.abs() <= ANGLE_TOL => return Err(CutPathError::AlignedDirection),
                _ => length,
            };
            let end = station + advance;
            if !end.is_finite() {
                return Err(CutPathError::NonFinite);
            }
            if end <= station {
                return Err(CutPathError::ShortSpan);
            }
            let tangent = if kind == CutKind::Offset { axis } else { tangent };
            spans.push(CutSpan::new(i + 1, tangent, side, station, end));
            station = end;
        }
        Ok(Self { kind, spans })
    }

    pub(crate) fn spans(&self) -> &[CutSpan] {
        &self.spans
    }

    /// Distance of `p` along the unfolded cut, measured on stretch `span` of `trace`.
    /// Aligned legs rotate about their shared pivot; parallel offset legs translate
    /// across the jog without advancing along the cut.
    #[cfg(test)]
    fn station(&self, trace: &[Point2], span: usize, p: Point2) -> f64 {
        let cut = &self.spans[span];
        let o = trace[cut.from];
        cut.start + (p.u - o.u) * cut.tangent.u + (p.v - o.v) * cut.tangent.v
    }

    /// The joint belongs to the following span, including the pivot of an aligned cut.
    #[cfg(test)]
    fn owns_station(&self, span: usize, station: f64) -> bool {
        self.spans.get(span).is_some_and(|cut| {
            station >= cut.start && (station < cut.end || (span + 1 == self.spans.len() && station == cut.end))
        })
    }
}

impl CutSpan {
    fn new(from: usize, tangent: Point2, side: f64, start: f64, end: f64) -> Self {
        let arrow = Point2::new(-tangent.v * side, tangent.u * side);
        Self { from, tangent, arrow, start, end }
    }

    /// The way this stretch's arrow points on the parent: the way its cut looks.
    pub(crate) fn arrow(&self) -> Point2 {
        self.arrow
    }

    /// Direction towards the eye and up vector of this stretch's cut through the parent,
    /// turned about the stretch as a straight section along it is.
    pub(crate) fn axes(&self, parent: &ViewFrame) -> (Vec3, Vec3) {
        let a = self.arrow;
        crate::project::fold_about(parent, Point2::new(a.v, -a.u), Point2::new(-a.u, -a.v))
    }
}

fn segment(a: Point2, b: Point2) -> Result<(Point2, f64), CutPathError> {
    let (u, v) = (b.u - a.u, b.v - a.v);
    let length = u.hypot(v);
    if !length.is_finite() {
        return Err(CutPathError::NonFinite);
    }
    if length < MIN_SPAN {
        return Err(CutPathError::ShortSpan);
    }
    Ok((Point2::new(u / length, v / length), length))
}

/// A stretch of the unfolded cut, in model space: the points whose station along
/// `axis` from `origin` (where the station is `start`) lies in `[lo, hi)`. The
/// first band reaches back without end and the last forward, as a straight cut's plane
/// does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Band {
    origin: Vec3,
    axis: Vec3,
    start: f64,
    lo: f64,
    hi: f64,
    tol: f64,
}

impl Band {
    fn station(&self, p: Vec3) -> f64 {
        self.start + (p - self.origin).dot(self.axis)
    }

    /// True for a station in the band. A station on a boundary belongs to the later
    /// band only.
    fn owns(&self, s: f64) -> bool {
        s >= self.lo - self.tol && s < self.hi - self.tol
    }

    /// The band turned by `turn`.
    fn turned(&self, turn: &Turn) -> Band {
        Band { origin: turn.point(self.origin), axis: turn.vector(self.axis), ..*self }
    }

    /// The band on paper: a quadrilateral reaching far past any view, for clipping the
    /// cut regions. `point` and `vector` take model space to view millimetres.
    pub(crate) fn window(&self, point: impl Fn(Vec3) -> Point2, vector: impl Fn(Vec3) -> Point2) -> Vec<Point2> {
        const FAR: f64 = 1e5;
        let o = point(self.origin);
        let a = vector(self.axis);
        let length = a.u.hypot(a.v);
        if length < 1e-12 {
            return Vec::new();
        }
        let scale = length;
        let (a, n) = (Point2::new(a.u / length, a.v / length), Point2::new(-a.v / length, a.u / length));
        let at = |s: f64| (s - self.start) * scale;
        let lo = if self.lo.is_finite() { at(self.lo) } else { -FAR };
        let hi = if self.hi.is_finite() { at(self.hi) } else { FAR };
        let corner =
            |along: f64, across: f64| Point2::new(o.u + a.u * along + n.u * across, o.v + a.v * along + n.v * across);
        vec![corner(lo, -FAR), corner(hi, -FAR), corner(hi, FAR), corner(lo, FAR)]
    }

    /// The pieces of `curve` in the band, each exact where the curve is: lines stay
    /// lines, circles and arcs become arcs and splines are cut by knot insertion.
    pub(crate) fn clip(&self, curve: &Curve3) -> Vec<Curve3> {
        let mut cuts: Vec<f64> = Vec::new();
        for boundary in [self.lo, self.hi].into_iter().filter(|b| b.is_finite()) {
            cuts.extend(self.crossings(curve, boundary));
        }
        cuts.retain(|t| *t > 1e-12 && *t < 1.0 - 1e-12);
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() <= 1e-12);
        let mut bounds = vec![0.0];
        bounds.extend(cuts);
        bounds.push(1.0);
        let mut kept: Vec<(f64, f64)> = Vec::new();
        for pair in bounds.windows(2) {
            let (t0, t1) = (pair[0], pair[1]);
            if !self.owns(self.station(curve.point_at((t0 + t1) / 2.0))) {
                continue;
            }
            match kept.last_mut() {
                Some(last) if last.1 == t0 => last.1 = t1,
                _ => kept.push((t0, t1)),
            }
        }
        // A circle kept across its start is one arc.
        if let Curve3::Circle { center, normal, radius } = curve
            && let [(first, a), .., (b, last)] = kept[..]
            && first <= 0.0
            && last >= 1.0
        {
            let mut pieces = vec![Curve3::Arc {
                center: *center,
                normal: *normal,
                x_axis: perpendicular(*normal),
                radius: *radius,
                start_deg: 360.0 * b,
                end_deg: 360.0 * (1.0 + a),
            }];
            pieces.extend(kept[1..kept.len() - 1].iter().map(|(t0, t1)| curve.between(*t0, *t1)));
            return pieces;
        }
        kept.into_iter()
            .map(|(t0, t1)| if t0 <= 0.0 && t1 >= 1.0 { curve.clone() } else { curve.between(t0, t1) })
            .collect()
    }

    /// The parameters in (0, 1) where `curve` crosses the station `boundary`.
    fn crossings(&self, curve: &Curve3, boundary: f64) -> Vec<f64> {
        match curve {
            Curve3::Line { a, b } => {
                let (s0, s1) = (self.station(*a), self.station(*b));
                if (s1 - s0).abs() <= self.tol {
                    return Vec::new();
                }
                vec![(boundary - s0) / (s1 - s0)]
            }
            Curve3::Arc { center, normal, x_axis, radius, start_deg, end_deg } => {
                self.circle_crossings(*center, *normal, *x_axis, *radius, *start_deg, *end_deg, boundary)
            }
            Curve3::Circle { center, normal, radius } => {
                self.circle_crossings(*center, *normal, perpendicular(*normal), *radius, 0.0, 360.0, boundary)
            }
            Curve3::Nurbs { .. } | Curve3::Polyline { .. } => {
                let samples = curve.sample_params(1e-4);
                let side = |p: Vec3| self.station(p) >= boundary;
                let mut out = Vec::new();
                for pair in samples.windows(2) {
                    let ((mut t0, p0), (mut t1, p1)) = (pair[0], pair[1]);
                    if side(p0) == side(p1) {
                        continue;
                    }
                    let low = side(p0);
                    for _ in 0..64 {
                        let mid = (t0 + t1) / 2.0;
                        if side(curve.point_at(mid)) == low {
                            t0 = mid;
                        } else {
                            t1 = mid;
                        }
                    }
                    out.push((t0 + t1) / 2.0);
                }
                out
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn circle_crossings(
        &self,
        center: Vec3,
        normal: Vec3,
        x_axis: Vec3,
        radius: f64,
        start_deg: f64,
        end_deg: f64,
        boundary: f64,
    ) -> Vec<f64> {
        // The station along the circle is c + p cos t + q sin t = c + r cos(t - phase).
        let y_axis = normal.cross(x_axis);
        let (c, p, q) = (self.station(center), radius * x_axis.dot(self.axis), radius * y_axis.dot(self.axis));
        let amplitude = p.hypot(q);
        if amplitude <= self.tol {
            return Vec::new();
        }
        let ratio = (boundary - c) / amplitude;
        if ratio.abs() >= 1.0 {
            return Vec::new();
        }
        let phase = q.atan2(p);
        let (a0, a1) = (start_deg.to_radians(), end_deg.to_radians());
        let sweep = a1 - a0;
        let mut out = Vec::new();
        for angle in [phase + ratio.acos(), phase - ratio.acos()] {
            let offset = (angle - a0).rem_euclid(std::f64::consts::TAU);
            if offset < sweep {
                out.push(offset / sweep);
            }
        }
        out
    }
}

/// A rotation about an axis through a point: how an aligned section's second cut is
/// revolved into the plane of the first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Turn {
    pivot: Vec3,
    axis: Vec3,
    cos: f64,
    sin: f64,
}

impl Turn {
    const NONE: Turn = Turn { pivot: Vec3::ZERO, axis: Vec3::Z, cos: 1.0, sin: 0.0 };

    fn vector(&self, v: Vec3) -> Vec3 {
        let k = self.axis;
        v * self.cos + k.cross(v) * self.sin + k * (k.dot(v) * (1.0 - self.cos))
    }

    fn point(&self, p: Vec3) -> Vec3 {
        self.pivot + self.vector(p - self.pivot)
    }

    fn curve(&self, curve: &Curve3) -> Curve3 {
        match curve {
            Curve3::Line { a, b } => Curve3::Line { a: self.point(*a), b: self.point(*b) },
            Curve3::Arc { center, normal, x_axis, radius, start_deg, end_deg } => Curve3::Arc {
                center: self.point(*center),
                normal: self.vector(*normal),
                x_axis: self.vector(*x_axis),
                radius: *radius,
                start_deg: *start_deg,
                end_deg: *end_deg,
            },
            Curve3::Circle { center, normal, radius } => {
                Curve3::Circle { center: self.point(*center), normal: self.vector(*normal), radius: *radius }
            }
            Curve3::Nurbs { degree, knots, points, weights } => Curve3::Nurbs {
                degree: *degree,
                knots: knots.clone(),
                points: points.iter().map(|p| self.point(*p)).collect(),
                weights: weights.clone(),
            },
            Curve3::Polyline { points } => Curve3::Polyline { points: points.iter().map(|p| self.point(*p)).collect() },
        }
    }

    fn view_curve(&self, c: &ViewCurve) -> ViewCurve {
        ViewCurve { curve: self.curve(&c.curve), ..c.clone() }
    }
}

/// A bent section through a parent view: its cut path in the parent's plane and the
/// model points of its trace.
#[derive(Debug)]
pub(crate) struct BentSection {
    parent: ViewFrame,
    path: CutPath,
    model: Vec<Vec3>,
}

/// One cut of a bent section, unfolded: the kernel's planar section along one stretch,
/// its curves kept to the stretch's band, and all of it turned into the first cut's
/// plane. The regions are whole; [`Band::window`] clips them on paper.
#[derive(Debug)]
pub(crate) struct UnfoldedCut {
    pub(crate) curves: Vec<ViewCurve>,
    pub(crate) regions: Vec<CutRegion>,
    pub(crate) band: Band,
}

/// What the cuts of a bent section show together, in the first cut's frame.
#[derive(Debug)]
pub(crate) struct Unfolded {
    pub(crate) curves: ViewCurves,
    pub(crate) cuts: Vec<UnfoldedCut>,
}

impl BentSection {
    /// The section of a parent with frame `parent` along the model points `model`,
    /// which the parent shows as its cutting line.
    pub(crate) fn new(parent: &ViewFrame, model: &[Vec3], flip: bool) -> Result<Self, CutPathError> {
        if model.iter().any(|p| !p.is_finite()) {
            return Err(CutPathError::NonFinite);
        }
        let trace: Vec<Point2> = model.iter().map(|p| parent.project(*p)).collect();
        let path = CutPath::of_line(&trace, flip)?;
        Ok(Self { parent: *parent, path, model: model.to_vec() })
    }

    /// Direction towards the eye and up of the section view: the first cut's.
    pub(crate) fn axes(&self) -> (Vec3, Vec3) {
        self.path.spans[0].axes(&self.parent)
    }

    fn in_model(&self, v: Point2) -> Vec3 {
        self.parent.right * v.u + self.parent.up * v.v
    }

    /// Each cut sectioned with `section(direction, up, origin)`, kept to its band and
    /// unfolded into the first cut's frame.
    pub(crate) fn unfold(
        &self,
        mut section: impl FnMut(Vec3, Vec3, Vec3) -> KernelResult<SectionView>,
    ) -> KernelResult<Unfolded> {
        let spans = &self.path.spans;
        let first = &spans[0];
        let (direction, up) = self.axes();
        let frame =
            soliddraft_kernel_api::drawing::ViewSpec::new(direction, up).frame().expect("a quarter turn of a frame");
        let length = spans.last().map_or(1.0, |s| s.end).max(1.0);
        let mut cuts = Vec::with_capacity(spans.len());
        let (mut unproved, mut catallax) = (false, true);
        let mut marks: Vec<String> = Vec::new();
        for (i, span) in spans.iter().enumerate() {
            let (eye, eye_up) = span.axes(&self.parent);
            let origin = self.model[span.from];
            let cut = section(eye, eye_up, origin)?;
            unproved |= cut.curves.unproved;
            catallax &= cut.curves.catallax;
            for mark in &cut.curves.approximate {
                if !marks.contains(mark) {
                    marks.push(mark.clone());
                }
            }
            let band = Band {
                origin,
                axis: self.in_model(span.tangent),
                start: span.start,
                lo: if i == 0 { f64::NEG_INFINITY } else { span.start },
                hi: if i + 1 == spans.len() { f64::INFINITY } else { span.end },
                tol: BOUNDARY_TOL * length,
            };
            // An aligned cut's later leg revolves about the pivot onto the first leg's line.
            let turn = if self.path.kind == CutKind::Aligned && i > 0 {
                let (t, t0) = (span.tangent, first.tangent);
                Turn {
                    pivot: origin,
                    axis: self.parent.right.cross(self.parent.up),
                    cos: t.u * t0.u + t.v * t0.v,
                    sin: t.u * t0.v - t.v * t0.u,
                }
            } else {
                Turn::NONE
            };
            let curves = cut
                .curves
                .curves
                .iter()
                .flat_map(|c| {
                    band.clip(&c.curve)
                        .into_iter()
                        .map(|piece| turn.view_curve(&ViewCurve { curve: piece, ..c.clone() }))
                })
                .collect();
            let regions = cut
                .regions
                .iter()
                .map(|r| CutRegion {
                    outer: r.outer.iter().map(|c| turn.view_curve(c)).collect(),
                    holes: r.holes.iter().map(|h| h.iter().map(|c| turn.view_curve(c)).collect()).collect(),
                })
                .collect();
            cuts.push(UnfoldedCut { curves, regions, band: band.turned(&turn) });
        }
        let all: Vec<ViewCurve> = cuts.iter().flat_map(|c| c.curves.iter().cloned()).collect();
        let curves = ViewCurves { unproved, catallax, approximate: marks, ..ViewCurves::new(all, &frame) };
        Ok(Unfolded { curves, cuts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::drawing::{CurveSource, ViewSpec};

    fn points<const N: usize>(p: [[f64; 2]; N]) -> [Point2; N] {
        p.map(Point2::from)
    }

    fn near(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
    }

    fn top() -> ViewFrame {
        ViewSpec::new(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)).frame().unwrap()
    }

    #[test]
    fn point_counts_name_the_kind_of_cut() {
        assert_eq!(CutKind::of(2), Some(CutKind::Straight));
        assert_eq!(CutKind::of(3), Some(CutKind::Aligned));
        assert_eq!(CutKind::of(4), Some(CutKind::Offset));
        assert_eq!(CutKind::of(6), Some(CutKind::Offset));
        assert_eq!(CutKind::of(5), None);
        assert_eq!(CutKind::of(1), None);
        let straight = points([[0.0, 0.0], [1.0, 0.0]]);
        assert_eq!(CutPath::of_line(&straight, false).unwrap_err(), CutPathError::PointCount);
    }

    #[test]
    fn offset_trace_crosses_both_staggered_holes_but_does_not_cut_on_the_jog() {
        let trace = points([[-20.0, -6.0], [0.0, -6.0], [0.0, 6.0], [20.0, 6.0]]);
        let [left_hole, right_hole] = points([[-10.0, -6.0], [10.0, 6.0]]);
        let path = CutPath::new(CutKind::Offset, &trace, false).unwrap();
        let [left, right] = path.spans() else { panic!("expected two material cuts") };
        near(path.station(&trace, 0, left_hole), 10.0);
        near(path.station(&trace, 1, right_hole), 30.0);
        near(path.station(&trace, 0, trace[1]), path.station(&trace, 1, trace[2]));
        assert!(!path.owns_station(0, 20.0));
        assert!(path.owns_station(1, 20.0));
        assert!(path.owns_station(1, 40.0));
        assert!(!path.owns_station(1, 41.0));

        let parent = top();
        assert_eq!(left.axes(&parent), right.axes(&parent));
        let reversed = [trace[3], trace[2], trace[1], trace[0]];
        let reversed = CutPath::new(CutKind::Offset, &reversed, false).unwrap();
        assert_eq!(reversed.spans()[0].axes(&parent), left.axes(&parent));
    }

    #[test]
    fn a_stretch_looks_the_way_a_straight_section_along_it_does() {
        let parent = top();
        for (trace, flip) in [
            (points([[-20.0, -6.0], [0.0, -6.0], [0.0, 6.0], [20.0, 6.0]]), false),
            (points([[-20.0, -6.0], [0.0, -6.0], [0.0, 6.0], [20.0, 6.0]]), true),
            (points([[5.0, -20.0], [5.0, 0.0], [-5.0, 0.0], [-5.0, 20.0]]), false),
            (points([[5.0, 20.0], [5.0, 0.0], [-5.0, 0.0], [-5.0, -20.0]]), true),
        ] {
            let path = CutPath::new(CutKind::Offset, &trace, flip).unwrap();
            let straight = crate::project::section_axes(&parent, &trace[..2], flip).unwrap();
            assert_eq!(path.spans()[0].axes(&parent), straight, "{trace:?} {flip}");
            assert_eq!(path.spans()[0].arrow(), crate::project::section_arrow(&trace[..2], flip).unwrap());
        }
    }

    #[test]
    fn rotated_radial_trace_unfolds_both_legs_and_owns_the_pivot_once() {
        let trace = points([[-12.0, -16.0], [0.0, 0.0], [-12.0, 9.0]]);
        let [inner, radial] = points([[-6.0, -8.0], [-8.0, 6.0]]);
        let path = CutPath::new(CutKind::Aligned, &trace, false).unwrap();
        let [fixed, rotated] = path.spans() else { panic!("expected both radial cuts") };
        near(path.station(&trace, 0, inner), 10.0);
        near(path.station(&trace, 1, radial), 30.0);
        let joint = path.station(&trace, 1, trace[1]);
        near(path.station(&trace, 0, trace[1]), joint);
        assert!(!path.owns_station(0, joint));
        assert!(path.owns_station(1, joint));
        near(path.station(&trace, 1, trace[2]), 35.0);

        let parent = top();
        let (first_eye, _) = fixed.axes(&parent);
        let (second_eye, _) = rotated.axes(&parent);
        near(first_eye.dot(second_eye), 0.0);
        near(first_eye.dot(Vec3::new(0.6, 0.8, 0.0)), 0.0);
        near(second_eye.dot(Vec3::new(-0.8, 0.6, 0.0)), 0.0);
        let flipped = CutPath::new(CutKind::Aligned, &trace, true).unwrap();
        assert_eq!(flipped.spans()[0].axes(&parent).0, -first_eye);
        assert_eq!(flipped.spans()[1].axes(&parent).0, -second_eye);
    }

    #[test]
    fn invalid_paths_cannot_produce_cut_frames() {
        use CutKind::{Aligned, Offset};
        use CutPathError::{AlignedDirection, NonFinite, OffsetDirection, PointCount, ShortSpan};
        for (kind, trace, error) in [
            (Offset, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]][..], PointCount),
            (Offset, &[[0.0, 0.0], [1.0, 0.0], [2.0, 1.0], [3.0, 1.0]], OffsetDirection),
            (Offset, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], OffsetDirection),
            (Aligned, &[[0.0, 0.0], [0.0, 1.0], [0.0, -1.0]], AlignedDirection),
            (Aligned, &[[0.0, 0.0], [0.0, 1.0], [0.0, 2.0]], AlignedDirection),
            (Aligned, &[[0.0, 0.0], [0.0, 1.0], [0.0, 1.0]], ShortSpan),
            (Aligned, &[[0.0, 0.0], [0.0, 1.0], [f64::NAN, 1.0]], NonFinite),
        ] {
            let trace: Vec<_> = trace.iter().copied().map(Point2::from).collect();
            assert_eq!(CutPath::new(kind, &trace, false).unwrap_err(), error);
        }
    }

    fn band(lo: f64, hi: f64) -> Band {
        Band { origin: Vec3::ZERO, axis: Vec3::X, start: 0.0, lo, hi, tol: 1e-9 }
    }

    #[test]
    fn bands_split_lines_exactly_and_give_a_boundary_line_to_the_later_band() {
        let line = Curve3::Line { a: Vec3::new(-10.0, 0.0, 0.0), b: Vec3::new(10.0, 0.0, 0.0) };
        let first = band(f64::NEG_INFINITY, 0.0).clip(&line);
        let second = band(0.0, f64::INFINITY).clip(&line);
        assert_eq!(first, [Curve3::Line { a: Vec3::new(-10.0, 0.0, 0.0), b: Vec3::ZERO }]);
        assert_eq!(second, [Curve3::Line { a: Vec3::ZERO, b: Vec3::new(10.0, 0.0, 0.0) }]);
        let on_boundary = Curve3::Line { a: Vec3::ZERO, b: Vec3::new(0.0, 5.0, 0.0) };
        assert!(band(f64::NEG_INFINITY, 0.0).clip(&on_boundary).is_empty());
        assert_eq!(band(0.0, f64::INFINITY).clip(&on_boundary), [on_boundary]);
    }

    #[test]
    fn bands_cut_circles_into_exact_arcs_that_meet_on_the_boundary() {
        let circle = Curve3::Circle { center: Vec3::ZERO, normal: Vec3::Z, radius: 5.0 };
        let left = band(f64::NEG_INFINITY, 0.0).clip(&circle);
        let right = band(0.0, f64::INFINITY).clip(&circle);
        let length = |pieces: &[Curve3]| pieces.iter().map(Curve3::length).sum::<f64>();
        near(length(&left), 5.0 * std::f64::consts::PI);
        near(length(&right), 5.0 * std::f64::consts::PI);
        for piece in left.iter().chain(&right) {
            let Curve3::Arc { radius, center, .. } = piece else { panic!("an arc: {piece:?}") };
            assert_eq!((*radius, *center), (5.0, Vec3::ZERO));
            for end in [piece.start(), piece.end()] {
                assert!(end.x.abs() < 1e-12, "the pieces meet on the boundary: {end:?}");
            }
            assert!(piece.point_at(0.5).x.signum() == if left.contains(piece) { -1.0 } else { 1.0 });
        }
        // A circle wholly inside stays the same circle.
        assert_eq!(band(-10.0, 10.0).clip(&circle), [circle]);
    }

    #[test]
    fn bands_cut_polylines_where_they_leave() {
        let polyline = Curve3::Polyline { points: vec![Vec3::new(-4.0, 0.0, 0.0), Vec3::new(4.0, 1.0, 0.0)] };
        let [piece] = &band(0.0, 2.0).clip(&polyline)[..] else { panic!("one piece") };
        near(piece.start().x, 0.0);
        near(piece.end().x, 2.0);
    }

    #[test]
    fn a_turn_revolves_the_second_leg_onto_the_first() {
        let parent = top();
        let trace_model = [Vec3::new(30.0, 0.0, 0.0), Vec3::ZERO, Vec3::new(-15.0, 15.0 * 3f64.sqrt(), 0.0)];
        let bent = BentSection::new(&parent, &trace_model, false).unwrap();
        let hole = Vec3::new(-7.5, 7.5 * 3f64.sqrt(), 0.0); // 15 mm out along the second leg
        let mut asked = Vec::new();
        let unfolded = bent
            .unfold(|eye, up, origin| {
                asked.push((eye, up, origin));
                let curve = Curve3::Circle {
                    center: if asked.len() == 1 { Vec3::new(15.0, 0.0, 0.0) } else { hole },
                    normal: Vec3::Z,
                    radius: 2.0,
                };
                let frame = ViewSpec::new(eye, up).frame().unwrap();
                Ok(SectionView {
                    curves: ViewCurves::new(vec![ViewCurve::new(curve, CurveSource::Unknown, true)], &frame),
                    regions: Vec::new(),
                })
            })
            .unwrap();
        assert_eq!(asked.len(), 2);
        assert_eq!(asked[1].2, Vec3::ZERO, "the second cut goes through the pivot");
        let centres: Vec<Vec3> = unfolded
            .curves
            .curves
            .iter()
            .map(|c| match c.curve {
                Curve3::Circle { center, .. } | Curve3::Arc { center, .. } => center,
                _ => panic!("{c:?}"),
            })
            .collect();
        assert_eq!(centres.len(), 2);
        // The first cut runs from x = 30 towards the pivot; the revolved hole lies 15 mm
        // beyond the pivot on the same line.
        assert!(centres[0].distance(Vec3::new(15.0, 0.0, 0.0)) < 1e-12);
        assert!(centres[1].distance(Vec3::new(-15.0, 0.0, 0.0)) < 1e-12, "{:?}", centres[1]);
        let (eye, _) = bent.axes();
        assert_eq!(eye, asked[0].0);
    }
}
