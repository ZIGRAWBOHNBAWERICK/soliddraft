//! Path and Cam residuals: the signed distance from a point to a planar outline of lines
//! and circular arcs.
//!
//! Outside the outline the distance is positive, inside negative. The distance to each piece
//! is evaluated in the same arithmetic as every other residual, so the certificate encloses
//! it; the least over the pieces is enclosed piece by piece. Which piece is nearest and on
//! which side the point lies are decided once, in doubles: away from a corner a piece's own
//! side answers, and at a corner a fine polygon round the outline does.

use super::super::bodies::Rule;
use super::super::command::{Local, LocalOutline, Read};
use super::interval::Number;
use super::{Body, Placement, Row, V, add, aligned, cross, direction, dot, local, mul, norm, rotate, scalar, sub};
use crate::mates::StoredSupport;
use std::collections::BTreeMap;

/// One piece placed in the world: a line from `from` to `to`, or an arc about `centre`.
struct Piece<N: Number> {
    from: V<N>,
    to: V<N>,
    /// Centre, radius, anticlockwise about the outline's normal, whole circle.
    arc: Option<(V<N>, N, bool, bool)>,
}

struct Placed<N: Number> {
    normal: V<N>,
    offset: N,
    pieces: Vec<Piece<N>>,
}

fn placed_point<N: Number>(p: &Placement, v: V<N>) -> V<N> {
    add(rotate(p, v), p.translation.to_array().map(N::f))
}

fn place<N: Number>(outline: &LocalOutline, at: &Placement) -> Placed<N> {
    let normal = rotate(at, direction(outline.plane.normal));
    let offset = N::q(&outline.plane.offset) + dot(normal, at.translation.to_array().map(N::f));
    let n = outline.pieces.len();
    let pieces = (0..n)
        .map(|i| {
            let (from, arc) = &outline.pieces[i];
            let to = &outline.pieces[(i + 1) % n].0;
            let whole = n == 1;
            Piece {
                from: placed_point(at, local(from)),
                to: placed_point(at, local(to)),
                arc: arc.as_ref().map(|(c, r2, ccw)| (placed_point(at, local(c)), N::q(r2).sqrt(), *ccw, whole)),
            }
        })
        .collect();
    Placed { normal, offset, pieces }
}

/// The signed angle from `u` to `v` about `n`, in [0, 2π), in doubles.
fn sweep(u: [f64; 3], v: [f64; 3], n: [f64; 3]) -> f64 {
    let a = f64::atan2(dot(cross(u, v), n), dot(u, v));
    if a < 0.0 { a + std::f64::consts::TAU } else { a }
}

fn middle<N: Number>(v: V<N>) -> [f64; 3] {
    v.map(|x| x.middle())
}

/// A fine polygon round the outline in doubles, for the side a point lies on at a corner
/// and for which way the outline runs.
fn polygon(p: &Placed<f64>) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    for piece in &p.pieces {
        match piece.arc {
            None => out.push(piece.from),
            Some((c, r, ccw, whole)) => {
                let u = sub(piece.from, c);
                let total = if whole { std::f64::consts::TAU } else { sweep(u, sub(piece.to, c), p.normal) };
                let total = if ccw || whole { total } else { total - std::f64::consts::TAU };
                let u = if whole {
                    // Any point on the circle starts it.
                    let k = if p.normal[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
                    let t = cross(p.normal, k);
                    mul(t, r / norm(t))
                } else {
                    u
                };
                for k in 0..32 {
                    let a = total * k as f64 / 32.0;
                    let (s, co) = (a.sin(), a.cos());
                    let w = add(mul(u, co), mul(cross(p.normal, u), s));
                    out.push(add(c, w));
                }
            }
        }
    }
    out
}

/// Twice the signed area of the polygon about `n`: positive when it runs anticlockwise.
fn area(poly: &[[f64; 3]], n: [f64; 3]) -> f64 {
    let mut sum = [0.0; 3];
    for i in 0..poly.len() {
        sum = add(sum, cross(poly[i], poly[(i + 1) % poly.len()]));
    }
    dot(sum, n)
}

/// Whether `c` lies inside the polygon, by its winding about `n`.
fn inside(poly: &[[f64; 3]], n: [f64; 3], c: [f64; 3]) -> bool {
    let mut total = 0.0;
    for i in 0..poly.len() {
        let (a, b) = (sub(poly[i], c), sub(poly[(i + 1) % poly.len()], c));
        total += f64::atan2(dot(cross(a, b), n), dot(a, b));
    }
    total.abs() > std::f64::consts::PI
}

/// The distance from `c` (in the outline's plane) to one piece, and the side the piece
/// alone says `c` is on when its nearest point is inside the piece: `Some(true)` outside.
fn piece_distance<N: Number>(piece: &Piece<N>, n: V<N>, anticlockwise: bool, c: V<N>) -> (N, Option<bool>) {
    match piece.arc {
        None => {
            let d = sub(piece.to, piece.from);
            let t = dot(sub(c, piece.from), d) / dot(d, d);
            let foot = add(piece.from, mul(d, t.clamp01()));
            let tm = t.middle();
            let side = (tm > 0.0 && tm < 1.0).then(|| {
                // Left of the direction of travel is inside an anticlockwise outline.
                let left = dot(cross(middle(d), sub(middle(c), middle(piece.from))), middle(n)) > 0.0;
                left != anticlockwise
            });
            (norm(sub(c, foot)), side)
        }
        Some((centre, r, ccw, whole)) => {
            let u = sub(c, centre);
            let radial = norm(u) - r;
            let distance = (radial * radial).sqrt();
            let within = whole || {
                let (a, b, m) = (middle(sub(piece.from, centre)), middle(sub(piece.to, centre)), middle(u));
                let (span, at) = if ccw {
                    (sweep(a, b, middle(n)), sweep(a, m, middle(n)))
                } else {
                    (sweep(b, a, middle(n)), sweep(b, m, middle(n)))
                };
                at <= span
            };
            if within {
                // An arc turning the way the outline runs bulges outward.
                let convex = ccw == anticlockwise;
                let beyond = radial.middle() > 0.0;
                (distance, Some(beyond == convex))
            } else {
                let ends = norm(sub(c, piece.from)).min(norm(sub(c, piece.to)));
                (ends, None)
            }
        }
    }
}

/// The signed distance from `c` to the outline, measured in its plane: positive outside.
fn signed<N: Number>(p: &Placed<N>, c: V<N>) -> N {
    // Into the plane.
    let c = sub(c, mul(p.normal, dot(p.normal, c) - p.offset));
    let doubles = Placed::<f64> {
        normal: middle(p.normal),
        offset: p.offset.middle(),
        pieces: p
            .pieces
            .iter()
            .map(|x| Piece {
                from: middle(x.from),
                to: middle(x.to),
                arc: x.arc.map(|(c, r, ccw, whole)| (middle(c), r.middle(), ccw, whole)),
            })
            .collect(),
    };
    let poly = polygon(&doubles);
    let anticlockwise = area(&poly, doubles.normal) > 0.0;
    let mut least: Option<N> = None;
    let mut nearest = (f64::INFINITY, None);
    for piece in &p.pieces {
        let (d, side) = piece_distance(piece, p.normal, anticlockwise, c);
        least = Some(match least {
            Some(l) => l.min(d),
            None => d,
        });
        if d.middle() < nearest.0 {
            nearest = (d.middle(), side);
        }
    }
    let outside = nearest.1.unwrap_or_else(|| !inside(&poly, doubles.normal, middle(c)));
    let d = least.unwrap_or(N::f(f64::NAN));
    if outside { d } else { -d }
}

fn outline_of(read: &Read) -> Option<&LocalOutline> {
    match &read.local {
        Local::Outline(o) => Some(o),
        _ => None,
    }
}

/// A follower's centre (a point, or a point on its axis), its axis direction when it has
/// one, and its radius (zero for a vertex).
fn follower<N: Number>(read: &Read, at: &Placement) -> (V<N>, Option<V<N>>, N) {
    let radius = read.radius.as_ref().map_or(N::f(0.0), |r| match &r.value {
        Ok(r) => N::q(r),
        Err(s) => N::q(s).sqrt(),
    });
    match &read.local {
        Local::Point(p) => (placed_point(at, local(p)), None, radius),
        Local::Axis(a) => (placed_point(at, local(&a.through)), Some(rotate(at, direction(a.direction))), radius),
        _ => ([N::f(f64::NAN); 3], None, radius),
    }
}

/// The residual rows of a Path or a Cam.
pub(super) fn rows<N: Number>(out: &mut Vec<Row<N>>, reads: &[Read], rule: &Rule, poses: &BTreeMap<Body, Placement>) {
    let Some(outline) = outline_of(&reads[0]) else {
        scalar(out, N::f(f64::NAN), false);
        return;
    };
    let placed = place::<N>(outline, &poses[&reads[0].body]);
    let (c, axis, r) = follower::<N>(&reads[1], &poses[&reads[1].body]);
    match rule {
        Rule::Path => {
            scalar(out, dot(placed.normal, c) - placed.offset, false);
            scalar(out, signed(&placed, c), false);
        }
        Rule::Cam { outside } => {
            if let Some(u) = axis {
                let sign = if dot(placed.normal, u).middle() < 0.0 { -1 } else { 1 };
                aligned(out, placed.normal, u, sign);
            }
            let contact = if *outside { r } else { -r };
            scalar(out, signed(&placed, c) - contact, false);
        }
        _ => scalar(out, N::f(f64::NAN), false),
    }
}

/// Whether a follower lies outside a cam's outline at the given placements, from what the
/// mate stores: the side a new Cam keeps it on.
pub(in crate::mates) fn outline_side(
    cam: Option<&StoredSupport>,
    follower: Option<&StoredSupport>,
    cam_at: &Placement,
    follower_at: &Placement,
) -> Option<bool> {
    let Some(StoredSupport::Outline { normal, offset, pieces }) = cam else { return None };
    let to = |x: &[f64; 3]| glam::DVec3::from_array(*x);
    let at = |v: glam::DVec3, p: &Placement| p.apply(v).to_array();
    let n = rotate(cam_at, direction(*normal));
    let placed = Placed::<f64> {
        normal: n,
        offset: *offset + dot(n, cam_at.translation.to_array()),
        pieces: (0..pieces.len())
            .map(|i| {
                let (p, next) = (&pieces[i], &pieces[(i + 1) % pieces.len()]);
                Piece {
                    from: at(to(&p.from), cam_at),
                    to: at(to(&next.from), cam_at),
                    arc: p
                        .centre
                        .zip(p.radius)
                        .map(|(c, r)| (at(to(&c), cam_at), r, p.anticlockwise, pieces.len() == 1)),
                }
            })
            .collect(),
    };
    let centre = match follower? {
        StoredSupport::Point { at: p, .. } => at(to(p), follower_at),
        StoredSupport::Axis { through, .. } => at(to(through), follower_at),
        _ => return None,
    };
    Some(signed(&placed, centre) >= 0.0)
}
