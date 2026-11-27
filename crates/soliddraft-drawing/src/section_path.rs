//! The paper trace and the material-cutting spans of a bent section.

use soliddraft_kernel_api::drawing::ViewFrame;
use soliddraft_kernel_api::{Point2, Vec3};

const MIN_SPAN: f64 = 1e-9;
const ANGLE_TOL: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CutKind {
    Offset,
    Aligned,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CutPathError {
    PointCount,
    NonFinite,
    ShortSpan,
    OffsetDirection,
    AlignedDirection,
}

#[derive(Debug)]
pub(crate) struct CutPath<'a> {
    trace: &'a [Point2],
    spans: Vec<CutSpan>,
}

#[derive(Debug)]
pub(crate) struct CutSpan {
    origin: Point2,
    tangent: Point2,
    arrow: Point2,
    start: f64,
    end: f64,
}

impl<'a> CutPath<'a> {
    pub(crate) fn new(kind: CutKind, trace: &'a [Point2], flip: bool) -> Result<Self, CutPathError> {
        let span_count = match kind {
            CutKind::Offset if trace.len() >= 4 && trace.len().is_multiple_of(2) => trace.len() / 2,
            CutKind::Aligned if trace.len() == 3 => 2,
            _ => return Err(CutPathError::PointCount),
        };
        let mut segments = trace.windows(2);
        let first = segments.next().unwrap();
        let (axis, length) = segment(first[0], first[1])?;
        // Match the straight section's default side, then carry it through each bend.
        let reversed = axis.u < -ANGLE_TOL || (axis.u.abs() <= ANGLE_TOL && axis.v < 0.0);
        let side = if reversed ^ flip { -1.0 } else { 1.0 };
        let mut spans = Vec::with_capacity(span_count);
        spans.push(CutSpan::new(first[0], axis, side, 0.0, length));
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
                CutKind::Aligned => length,
            };
            let end = station + advance;
            if !end.is_finite() {
                return Err(CutPathError::NonFinite);
            }
            if end <= station {
                return Err(CutPathError::ShortSpan);
            }
            let tangent = if kind == CutKind::Offset { axis } else { tangent };
            spans.push(CutSpan::new(pair[0], tangent, side, station, end));
            station = end;
        }
        Ok(Self { trace, spans })
    }

    pub(crate) fn trace(&self) -> &'a [Point2] {
        self.trace
    }

    pub(crate) fn spans(&self) -> &[CutSpan] {
        &self.spans
    }

    /// The joint belongs to the following span, including the pivot of an aligned cut.
    pub(crate) fn owns_station(&self, span: usize, station: f64) -> bool {
        self.spans.get(span).is_some_and(|cut| {
            station >= cut.start && (station < cut.end || (span + 1 == self.spans.len() && station == cut.end))
        })
    }
}

impl CutSpan {
    fn new(origin: Point2, tangent: Point2, side: f64, start: f64, end: f64) -> Self {
        let arrow = Point2::new(-tangent.v * side, tangent.u * side);
        Self { origin, tangent, arrow, start, end }
    }

    /// Distance along the unfolded cut. Aligned legs rotate about their shared pivot;
    /// parallel offset legs translate across the jog without advancing along the cut.
    pub(crate) fn station(&self, p: Point2) -> f64 {
        self.start + (p.u - self.origin.u) * self.tangent.u + (p.v - self.origin.v) * self.tangent.v
    }

    /// Eye direction and common depth axis for the vertical cut through the parent.
    pub(crate) fn axes(&self, parent: &ViewFrame) -> (Vec3, Vec3) {
        (-parent.right * self.arrow.u - parent.up * self.arrow.v, parent.direction)
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

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::drawing::ViewSpec;

    fn points<const N: usize>(p: [[f64; 2]; N]) -> [Point2; N] {
        p.map(Point2::from)
    }

    fn near(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-10, "{actual} != {expected}");
    }

    #[test]
    fn offset_trace_crosses_both_staggered_holes_but_does_not_cut_on_the_jog() {
        let trace = points([[-20.0, -6.0], [0.0, -6.0], [0.0, 6.0], [20.0, 6.0]]);
        let [left_hole, right_hole] = points([[-10.0, -6.0], [10.0, 6.0]]);
        let path = CutPath::new(CutKind::Offset, &trace, false).unwrap();
        assert_eq!(path.trace(), &trace);
        let [left, right] = path.spans() else { panic!("expected two material cuts") };
        near(left.station(left_hole), 10.0);
        near(right.station(right_hole), 30.0);
        near(left.station(trace[1]), right.station(trace[2]));
        assert!(!path.owns_station(0, 20.0));
        assert!(path.owns_station(1, 20.0));
        assert!(path.owns_station(1, 40.0));
        assert!(!path.owns_station(1, 41.0));

        let parent = ViewSpec::new(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)).frame().unwrap();
        assert_eq!(left.axes(&parent), right.axes(&parent));
        let reversed = [trace[3], trace[2], trace[1], trace[0]];
        let reversed = CutPath::new(CutKind::Offset, &reversed, false).unwrap();
        assert_eq!(reversed.spans()[0].axes(&parent), left.axes(&parent));
    }

    #[test]
    fn rotated_radial_trace_unfolds_both_legs_and_owns_the_pivot_once() {
        let trace = points([[-12.0, -16.0], [0.0, 0.0], [-12.0, 9.0]]);
        let [inner, radial] = points([[-6.0, -8.0], [-8.0, 6.0]]);
        let path = CutPath::new(CutKind::Aligned, &trace, false).unwrap();
        assert_eq!(path.trace(), &trace);
        let [fixed, rotated] = path.spans() else { panic!("expected both radial cuts") };
        near(fixed.station(inner), 10.0);
        near(rotated.station(radial), 30.0);
        let joint = rotated.station(trace[1]);
        near(fixed.station(trace[1]), joint);
        assert!(!path.owns_station(0, joint));
        assert!(path.owns_station(1, joint));
        near(rotated.station(trace[2]), 35.0);

        let parent = ViewSpec::new(Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)).frame().unwrap();
        let (first_eye, first_up) = fixed.axes(&parent);
        let (second_eye, second_up) = rotated.axes(&parent);
        near(first_eye.dot(second_eye), 0.0);
        near(first_eye.dot(Vec3::new(0.6, 0.8, 0.0)), 0.0);
        near(second_eye.dot(Vec3::new(-0.8, 0.6, 0.0)), 0.0);
        assert_eq!(first_up, parent.direction);
        assert_eq!(second_up, parent.direction);
        let flipped = CutPath::new(CutKind::Aligned, &trace, true).unwrap();
        assert_eq!(flipped.spans()[0].axes(&parent), (-first_eye, first_up));
        assert_eq!(flipped.spans()[1].axes(&parent), (-second_eye, second_up));
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
}
