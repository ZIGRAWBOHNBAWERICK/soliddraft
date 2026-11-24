//! Where an extrusion meets a start plane that is not parallel to its sketch.
//!
//! The extrusion is built as a prism of the profile along the extrusion direction,
//! clipped by a slab: a rectangle on the start plane swept the same way. This module
//! finds how far the start plane lies from the sketch along the direction over the
//! profile, and the rectangle that covers the profile's shadow on the start plane.

use soliddraft_kernel_api::{Plane, Point2, Profile, Segment, Vec3};

/// How a profile meets a tilted start plane along a sweep direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Span {
    /// The least distance from the sketch plane to the start plane along the direction,
    /// over the profile.
    pub near: f64,
    /// The greatest such distance.
    pub far: f64,
    /// The start plane with its origin at the centre of the slab's rectangle.
    pub slab: Plane,
    /// The rectangle's size along the start plane's U and V axes.
    pub width: f64,
    pub height: f64,
}

/// The profile's bounding box in its plane's coordinates. Arcs and ellipses count
/// their whole curve, which only makes the box larger.
pub(crate) fn bounds(profile: &Profile) -> Option<(Point2, Point2)> {
    let mut points: Vec<Point2> = Vec::new();
    let round = |points: &mut Vec<Point2>, centre: Point2, r: f64| {
        points.push(Point2::new(centre.u - r, centre.v - r));
        points.push(Point2::new(centre.u + r, centre.v + r));
    };
    for segment in profile.loops().flat_map(|l| l.0.iter()) {
        match *segment {
            Segment::Line { from, to } => points.extend([from, to]),
            Segment::Bezier { from, controls, to } => {
                points.extend([from, to]);
                points.extend(controls.as_slice().iter().copied());
            }
            Segment::Arc { center, from, .. } => {
                round(&mut points, center, (from.u - center.u).hypot(from.v - center.v));
            }
            Segment::Circle { center, radius } => round(&mut points, center, radius),
            Segment::Ellipse { center, axis, ratio } | Segment::EllipticArc { center, axis, ratio, .. } => {
                round(&mut points, center, axis.u.hypot(axis.v) * ratio.max(1.0));
            }
        }
    }
    let first = *points.first()?;
    let (lo, hi) = points.iter().fold((first, first), |(lo, hi), p| {
        (Point2::new(lo.u.min(p.u), lo.v.min(p.v)), Point2::new(hi.u.max(p.u), hi.v.max(p.v)))
    });
    (lo.u.is_finite() && lo.v.is_finite() && hi.u.is_finite() && hi.v.is_finite()).then_some((lo, hi))
}

/// How `profile` meets `start` along `along`, a unit direction. None when the direction
/// lies in the start plane or the profile is empty.
pub(crate) fn span(profile: &Profile, start: &Plane, along: Vec3) -> Option<Span> {
    let rate = along.dot(start.normal);
    if rate.abs() <= 1e-9 {
        return None;
    }
    let (lo, hi) = bounds(profile)?;
    let corners = [(lo.u, lo.v), (hi.u, lo.v), (lo.u, hi.v), (hi.u, hi.v)]
        .map(|(u, v)| profile.plane.to_world(Point2::new(u, v)));
    // The distance along the direction is linear over the sketch plane, so the box's
    // corners hold its least and greatest values, and their shadows on the start
    // plane enclose the profile's shadow.
    let heights = corners.map(|c| (start.origin - c).dot(start.normal) / rate);
    let (y, x) = (start.y_dir(), start.x_dir);
    let mut box_u = (f64::INFINITY, f64::NEG_INFINITY);
    let mut box_v = (f64::INFINITY, f64::NEG_INFINITY);
    for (corner, h) in corners.iter().zip(heights) {
        let q = *corner + along * h - start.origin;
        let (u, v) = (q.dot(x), q.dot(y));
        box_u = (box_u.0.min(u), box_u.1.max(u));
        box_v = (box_v.0.min(v), box_v.1.max(v));
    }
    let pad = 1.0 + 0.05 * (box_u.1 - box_u.0).max(box_v.1 - box_v.0);
    let centre = start.origin + x * ((box_u.0 + box_u.1) / 2.0) + y * ((box_v.0 + box_v.1) / 2.0);
    let near = heights.into_iter().fold(f64::INFINITY, f64::min);
    let far = heights.into_iter().fold(f64::NEG_INFINITY, f64::max);
    let span = Span {
        near,
        far,
        slab: start.with_origin(centre),
        width: box_u.1 - box_u.0 + 2.0 * pad,
        height: box_v.1 - box_v.0 + 2.0 * pad,
    };
    [near, far, span.width, span.height].iter().all(|v| v.is_finite()).then_some(span)
}

/// The margin the clipped prism reaches past the slab at each end, in millimetres.
pub(crate) fn margin(span: &Span) -> f64 {
    1.0 + 0.05 * (span.far - span.near)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tilted(deg: f64) -> Plane {
        let (s, c) = (deg.to_radians().sin(), deg.to_radians().cos());
        Plane { origin: Vec3::new(0.0, 0.0, 20.0), normal: Vec3::new(0.0, s, c), x_dir: Vec3::X }
    }

    #[test]
    fn a_square_under_a_plane_tilted_45_degrees_spans_its_rise() {
        let square = Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 6.0, 6.0);
        let span = span(&square, &tilted(45.0), Vec3::Z).unwrap();
        // z = 20 - y on the plane, so over y in [-3, 3] the plane lies 17 to 23 above.
        assert!((span.near - 17.0).abs() < 1e-9 && (span.far - 23.0).abs() < 1e-9, "{span:?}");
        // The shadow is 6 wide and 6 * sqrt 2 long on the plane, plus the padding.
        assert!(span.width > 6.0 && span.height > 6.0 * 2f64.sqrt(), "{span:?}");
        assert!((span.slab.origin - Vec3::new(0.0, 0.0, 20.0)).length() < 1e-9);
        // Against the plane's normal the distances turn negative.
        let down = super::span(&square, &tilted(45.0), -Vec3::Z).unwrap();
        assert!((down.near + 23.0).abs() < 1e-9 && (down.far + 17.0).abs() < 1e-9, "{down:?}");
    }

    #[test]
    fn a_direction_in_the_start_plane_has_no_span() {
        let square = Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 6.0, 6.0);
        let upright = Plane { origin: Vec3::ZERO, normal: Vec3::X, x_dir: Vec3::Y };
        assert_eq!(span(&square, &upright, Vec3::Z), None);
    }

    #[test]
    fn arcs_and_circles_count_their_whole_curve() {
        let circle = Profile::circle(Plane::XY, Point2::new(1.0, 2.0), 3.0);
        let (lo, hi) = bounds(&circle).unwrap();
        assert_eq!((lo, hi), (Point2::new(-2.0, -1.0), Point2::new(4.0, 5.0)));
    }
}
