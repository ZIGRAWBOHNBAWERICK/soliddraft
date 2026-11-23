//! Path and Cam residuals: the signed distance from a point to a planar outline of lines,
//! circular arcs and splines.
//!
//! Outside the outline the distance is positive, inside negative. The distance to each piece
//! is evaluated in the same arithmetic as every other residual, so the certificate encloses
//! it; the least over the pieces is enclosed piece by piece. Which piece is nearest and on
//! which side the point lies are decided once, in doubles: away from a corner a piece's own
//! side answers, and at a corner a fine polygon round the outline does.
//!
//! A spline piece's distance is the distance to its point at the foot parameter found in
//! doubles, evaluated by de Boor's algorithm at that exact parameter. That is the distance to
//! the spline where the foot is its nearest point, and an outline with a spline adds a row
//! for the foot condition (the offset along the tangent there), certified with the rest.

use super::super::bodies::Rule;
use super::super::command::{Local, LocalOutline, Read};
use super::super::solver::SlotMode;
use super::interval::Number;
use super::native::{World, world};
use super::{
    Body, Placement, Row, V, add, aligned, cross, direction, dot, local, mul, norm, range, rotate, scalar, sub, turn,
    vector,
};
use crate::mates::{MateRefusal, RefusalCode, StoredSupport};
use std::collections::BTreeMap;

/// One piece placed in the world: a line from `from` to `to`, or an arc about `centre`.
struct Piece<N: Number> {
    from: V<N>,
    to: V<N>,
    /// Centre, radius, anticlockwise about the outline's normal, whole circle.
    arc: Option<(V<N>, N, bool, bool)>,
    /// A spline from `from` to `to`.
    spline: Option<Spline<N>>,
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
            let (from, arc, spline) = &outline.pieces[i];
            let to = &outline.pieces[(i + 1) % n].0;
            let whole = n == 1;
            Piece {
                from: placed_point(at, local(from)),
                to: placed_point(at, local(to)),
                arc: arc.as_ref().map(|(c, r2, ccw)| (placed_point(at, local(c)), N::q(r2).sqrt(), *ccw, whole)),
                spline: spline.as_ref().map(|s| Spline::placed(s, at)),
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
        if let Some(s) = &piece.spline {
            let (lo, hi) = s.range();
            out.extend((0..32).map(|k| s.at(lo + (hi - lo) * k as f64 / 32.0).0));
            continue;
        }
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
    if let Some(s) = &piece.spline {
        // The distance to the point at the foot, found in doubles: an upper bound on the
        // distance to the spline, and the distance itself where the foot is the nearest point.
        let doubles = s.doubles();
        let u = doubles.foot(middle(c));
        let (at, dp) = s.at(u);
        let (lo, hi) = doubles.range();
        let side = (u > lo && u < hi).then(|| {
            let left = dot(cross(middle(dp), sub(middle(c), middle(at))), middle(n)) > 0.0;
            left != anticlockwise
        });
        return (norm(sub(c, at)), side);
    }
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
    let doubles = doubles_of(p);
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

/// A spline piece placed in the world: its control points placed, its knots and weights.
#[derive(Clone)]
struct Spline<N: Number> {
    degree: usize,
    knots: Vec<f64>,
    points: Vec<V<N>>,
    weights: Vec<f64>,
}

impl<N: Number> Spline<N> {
    fn placed(s: &crate::mates::StoredSpline, at: &Placement) -> Spline<N> {
        Spline {
            degree: s.degree as usize,
            knots: s.knots.clone(),
            points: s.points.iter().map(|p| placed_point(at, p.map(N::f))).collect(),
            weights: s.weights.clone(),
        }
    }

    fn doubles(&self) -> Spline<f64> {
        Spline {
            degree: self.degree,
            knots: self.knots.clone(),
            points: self.points.iter().map(|p| middle(*p)).collect(),
            weights: self.weights.clone(),
        }
    }

    /// Its parameter range.
    fn range(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.points.len()])
    }

    /// The point and the derivative at the exact parameter `u`, by de Boor's algorithm on
    /// homogeneous points: the derivative from the last level's two points.
    fn at(&self, u: f64) -> (V<N>, V<N>) {
        let (p, n) = (self.degree, self.points.len());
        let (lo, hi) = self.range();
        let u = u.clamp(lo, hi);
        let mut k = p;
        while k + 1 < n && self.knots[k + 1] <= u {
            k += 1;
        }
        let w = |i: usize| N::f(self.weights.get(i).copied().unwrap_or(1.0));
        let mut d: Vec<[N; 4]> = (0..=p)
            .map(|j| {
                let i = k + j - p;
                let (q, wi) = (self.points[i], w(i));
                [q[0] * wi, q[1] * wi, q[2] * wi, wi]
            })
            .collect();
        let at = N::f(u);
        let mut last = (d[p], d[p]);
        for r in 1..=p {
            if r == p {
                last = (d[p - 1], d[p]);
            }
            for j in (r..=p).rev() {
                let i = k + j - p;
                let denom = self.knots[i + p + 1 - r] - self.knots[i];
                let a = if denom == 0.0 { N::f(0.0) } else { (at - N::f(self.knots[i])) / N::f(denom) };
                let prev = d[j - 1];
                for c in 0..4 {
                    d[j][c] = (N::f(1.0) - a) * prev[c] + a * d[j][c];
                }
            }
        }
        let h = d[p];
        let span = self.knots[k + 1] - self.knots[k];
        let scale = N::f(if span == 0.0 { 0.0 } else { p as f64 / span });
        let dh: [N; 4] = std::array::from_fn(|c| (last.1[c] - last.0[c]) * scale);
        let point = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
        let derivative = std::array::from_fn(|c| (dh[c] - dh[3] * point[c]) / h[3]);
        (point, derivative)
    }
}

impl Spline<f64> {
    /// The parameter of the point nearest `c`, in doubles: the best of a fine sampling,
    /// refined by Newton's method on the foot condition.
    fn foot(&self, c: [f64; 3]) -> f64 {
        let (lo, hi) = self.range();
        let samples = 8 * self.points.len().max(2);
        let mut best = (f64::INFINITY, lo);
        for i in 0..=samples {
            let u = lo + (hi - lo) * i as f64 / samples as f64;
            let d = norm(sub(self.at(u).0, c));
            if d < best.0 {
                best = (d, u);
            }
        }
        let mut u = best.1;
        for _ in 0..16 {
            let (p, dp) = self.at(u);
            let h = dot(dp, dp);
            if h == 0.0 {
                break;
            }
            let next = (u + dot(sub(c, p), dp) / h).clamp(lo, hi);
            let done = (next - u).abs() <= 1e-15 * (hi - lo).abs().max(1.0);
            u = next;
            if done {
                break;
            }
        }
        u
    }

    /// The length from parameter `a` to `b`, by Gauss-Legendre quadrature on eight
    /// pieces of each knot span.
    fn length(&self, a: f64, b: f64) -> f64 {
        const X: [f64; 5] =
            [0.0, -0.538_469_310_105_683, 0.538_469_310_105_683, -0.906_179_845_938_664, 0.906_179_845_938_664];
        const W: [f64; 5] = [
            0.568_888_888_888_889,
            0.478_628_670_499_366,
            0.478_628_670_499_366,
            0.236_926_885_056_189,
            0.236_926_885_056_189,
        ];
        let mut total = 0.0;
        let mut cuts: Vec<f64> = self.knots.iter().copied().filter(|k| *k > a && *k < b).collect();
        cuts.insert(0, a);
        cuts.push(b);
        for pair in cuts.windows(2) {
            for m in 0..8 {
                let (x0, x1) = (
                    pair[0] + (pair[1] - pair[0]) * m as f64 / 8.0,
                    pair[0] + (pair[1] - pair[0]) * (m + 1) as f64 / 8.0,
                );
                let (mid, half) = ((x0 + x1) / 2.0, (x1 - x0) / 2.0);
                total += X.iter().zip(W).map(|(x, w)| w * norm(self.at(mid + half * x).1)).sum::<f64>() * half;
            }
        }
        total
    }

    /// The parameter `len` along the spline from its start, by bisection on its length.
    fn parameter(&self, len: f64) -> f64 {
        let (mut lo, mut hi) = self.range();
        let start = lo;
        for _ in 0..80 {
            let mid = (lo + hi) / 2.0;
            if self.length(start, mid) < len {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo + hi) / 2.0
    }
}

/// The outline in doubles, for the choices made once.
fn doubles_of<N: Number>(p: &Placed<N>) -> Placed<f64> {
    Placed {
        normal: middle(p.normal),
        offset: p.offset.middle(),
        pieces: p
            .pieces
            .iter()
            .map(|x| Piece {
                from: middle(x.from),
                to: middle(x.to),
                arc: x.arc.map(|(c, r, ccw, whole)| (middle(c), r.middle(), ccw, whole)),
                spline: x.spline.as_ref().map(Spline::doubles),
            })
            .collect(),
    }
}

/// The distance from `c` along the outline's tangent at the nearest spline piece's foot:
/// zero there by the foot's definition, and certified with the other rows. Zero when the
/// nearest piece is a line or an arc, whose distances are enclosed directly.
fn tangential<N: Number>(p: &Placed<N>, c: V<N>) -> N {
    let d = doubles_of(p);
    let cm = middle(c);
    // Which way the outline runs changes no distance.
    let anticlockwise = true;
    let nearest = p.pieces.iter().zip(&d.pieces).min_by(|a, b| {
        let f = |x: &Piece<f64>| piece_distance(x, d.normal, anticlockwise, cm).0;
        f(a.1).total_cmp(&f(b.1))
    });
    match nearest {
        Some((piece, doubles)) if piece.spline.is_some() => {
            let s = piece.spline.as_ref().expect("checked");
            let u = doubles.spline.as_ref().expect("checked").foot(cm);
            let (at, dp) = s.at(u);
            dot(sub(c, at), mul(dp, N::f(1.0) / norm(dp)))
        }
        _ => N::f(0.0),
    }
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

/// Where a whole circle starts: the point its polygon starts from.
fn circle_start<N: Number>(normal: V<N>, centre: V<N>, r: N) -> V<N> {
    let k = if normal[0].middle().abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let t = cross(normal, k.map(N::f));
    add(centre, mul(t, r / norm(t)))
}

/// Each piece's length, in turn round the outline.
fn lengths<N: Number>(p: &Placed<N>) -> Vec<N> {
    p.pieces
        .iter()
        .map(|piece| match piece.arc {
            None if piece.spline.is_some() => {
                let s = piece.spline.as_ref().expect("checked").doubles();
                let (lo, hi) = s.range();
                N::f(s.length(lo, hi))
            }
            None => norm(sub(piece.to, piece.from)),
            Some((_, r, _, true)) => r * N::pi() * N::f(2.0),
            Some((c, r, ccw, false)) => {
                let n = if ccw { p.normal } else { mul(p.normal, N::f(-1.0)) };
                let (u, v) = (sub(piece.from, c), sub(piece.to, c));
                let near = sweep(middle(u), middle(v), middle(n));
                r * dot(cross(u, v), n).atan2(dot(u, v), near)
            }
        })
        .collect()
}

/// The point `s` along the outline from its first corner, and the unit tangent there the
/// way the outline runs. Which piece holds it is decided in doubles.
fn position<N: Number>(p: &Placed<N>, lengths: &[N], s: N) -> (V<N>, V<N>) {
    let mut start = N::f(0.0);
    for (i, piece) in p.pieces.iter().enumerate() {
        let end = start + lengths[i];
        if s.middle() <= end.middle() || i + 1 == p.pieces.len() {
            let t = s - start;
            if let Some(spline) = &piece.spline {
                let u = spline.doubles().parameter(t.middle());
                let (at, d) = spline.at(u);
                return (at, mul(d, N::f(1.0) / norm(d)));
            }
            return match piece.arc {
                None => {
                    let d = sub(piece.to, piece.from);
                    (add(piece.from, mul(d, t / lengths[i])), mul(d, N::f(1.0) / lengths[i]))
                }
                Some((c, r, ccw, whole)) => {
                    let n = if ccw || whole { p.normal } else { mul(p.normal, N::f(-1.0)) };
                    let from = if whole { circle_start(p.normal, c, r) } else { piece.from };
                    let u = sub(from, c);
                    let (sin, cos) = (t / r).sin_cos();
                    let w = add(mul(u, cos), mul(cross(n, u), sin));
                    (add(c, w), mul(cross(n, w), N::f(1.0) / r))
                }
            };
        }
        start = end;
    }
    ([N::f(f64::NAN); 3], [N::f(f64::NAN); 3])
}

/// The unit tangent, the way the outline runs, at the point of the outline nearest `c`.
fn tangent_near<N: Number>(p: &Placed<N>, c: V<N>) -> V<N> {
    let doubles = doubles_of(p);
    let (nearest, near) = p
        .pieces
        .iter()
        .zip(&doubles.pieces)
        .min_by(|a, b| {
            let d = |x: &Piece<f64>| piece_distance(x, doubles.normal, true, middle(c)).0;
            d(a.1).total_cmp(&d(b.1))
        })
        .expect("an outline has a piece");
    if let (Some(s), Some(d)) = (&nearest.spline, &near.spline) {
        let (_, dp) = s.at(d.foot(middle(c)));
        return mul(dp, N::f(1.0) / norm(dp));
    }
    match nearest.arc {
        None => {
            let d = sub(nearest.to, nearest.from);
            mul(d, N::f(1.0) / norm(d))
        }
        Some((centre, _, ccw, whole)) => {
            let n = if ccw || whole { p.normal } else { mul(p.normal, N::f(-1.0)) };
            let w = sub(c, centre);
            let w = sub(w, mul(p.normal, dot(p.normal, w)));
            mul(cross(n, w), N::f(1.0) / norm(w))
        }
    }
}

/// How far along a slot or path `mode` puts its point, given the run's length.
fn along<N: Number>(mode: &SlotMode, total: N) -> N {
    match mode {
        SlotMode::Free | SlotMode::Centre => total * N::f(0.5),
        SlotMode::Distance { distance, flipped } => {
            if *flipped {
                total - N::q(distance)
            } else {
                N::q(distance)
            }
        }
        SlotMode::Share { fraction, flipped } => {
            if *flipped {
                total * (N::f(1.0) - N::q(fraction))
            } else {
                total * N::q(fraction)
            }
        }
    }
}

/// The length of a path's outline or an arc slot's run, in doubles.
fn run_length(read: &Read) -> Option<f64> {
    match &read.local {
        Local::Outline(o) => Some(lengths(&place::<f64>(o, &Placement::IDENTITY)).iter().sum()),
        Local::ArcSlot(s) => {
            let (c, e0, e1) = (local::<f64>(&s.centre), local::<f64>(&s.ends[0]), local::<f64>(&s.ends[1]));
            Some(super::super::solver::nearest(&s.radius) * sweep(sub(e0, c), sub(e1, c), direction(s.depth)))
        }
        _ => None,
    }
}

/// Refuses a distance that passes the end of a path or a slot, naming the mate.
pub(super) fn check_position(mate: &str, read: &Read, mode: &SlotMode, what: &str) -> Result<(), MateRefusal> {
    let SlotMode::Distance { distance, .. } = mode else { return Ok(()) };
    let Some(total) = run_length(read) else { return Ok(()) };
    let d = super::super::solver::nearest(distance);
    if d > total {
        return Err(MateRefusal {
            mates: vec![mate.to_owned()],
            ..MateRefusal::new(
                RefusalCode::InvalidValue,
                format!("{mate}: {d} mm passes the end of the {what}, {total:.3} mm long"),
            )
        });
    }
    Ok(())
}

/// The residual rows of a pin in an arc slot: the pin's axis along the slot's, through the
/// slot's centre line, and along it between the ends or where `mode` puts it.
pub(super) fn arc_slot_rows<N: Number>(
    out: &mut Vec<Row<N>>,
    reads: &[Read],
    rule: &Rule,
    poses: &BTreeMap<Body, Placement>,
    rank: bool,
) {
    let (Rule::Slot { sign, mode, lock }, Local::ArcSlot(slot)) = (rule, &reads[0].local) else {
        scalar(out, N::f(f64::NAN), false);
        return;
    };
    let World::Axis { direction: u, through: p, .. } = world::<N>(&reads[1], poses) else {
        scalar(out, N::f(f64::NAN), false);
        return;
    };
    let at = &poses[&reads[0].body];
    let d = rotate(at, direction(slot.depth));
    let c = placed_point(at, local(&slot.centre));
    let (u0, u1) = (sub(placed_point(at, local(&slot.ends[0])), c), sub(placed_point(at, local(&slot.ends[1])), c));
    let r = N::q(&slot.radius);
    aligned(out, d, u, *sign);
    // Where the pin's axis crosses the slot's plane, from the slot's centre.
    let w = sub(sub(p, mul(d, dot(d, sub(p, c)))), c);
    let near = sweep(middle(u0), middle(u1), middle(d));
    let total = dot(cross(u0, u1), d).atan2(dot(u0, u1), near);
    match mode {
        SlotMode::Free => {
            scalar(out, norm(w) - r, false);
            let half = total * N::f(0.5);
            let (sin, cos) = half.sin_cos();
            let m = add(mul(u0, cos), mul(cross(d, u0), sin));
            range(out, dot(cross(m, w), d).atan2(dot(m, w), 0.0), -half, half, true, rank);
        }
        _ => {
            let angle = along(mode, r * total) / r;
            let (sin, cos) = angle.sin_cos();
            vector(out, sub(w, add(mul(u0, cos), mul(cross(d, u0), sin))), false);
        }
    }
    if let Some(lock) = lock {
        match lock.rotation() {
            Ok(relative) => turn(out, at, &poses[&reads[1].body], relative),
            Err(_) => scalar(out, N::f(f64::NAN), true),
        }
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
        Rule::Path { mode, follow, up } => {
            let splines = placed.pieces.iter().any(|x| x.spline.is_some());
            let tangent = if *mode == SlotMode::Free {
                scalar(out, dot(placed.normal, c) - placed.offset, false);
                scalar(out, signed(&placed, c), false);
                if splines {
                    scalar(out, tangential(&placed, c), false);
                }
                follow.map(|_| tangent_near(&placed, c))
            } else {
                let lengths = lengths(&placed);
                let total = lengths.iter().fold(N::f(0.0), |a, b| a + *b);
                let (at, tangent) = position(&placed, &lengths, along(mode, total));
                vector(out, sub(c, at), false);
                Some(tangent)
            };
            let mut k = 2;
            if let (Some(sign), Some(tangent)) = (follow, tangent) {
                aligned(out, tangent, world::<N>(&reads[k], poses).direction(), *sign);
                k += 1;
            }
            if let Some(sign) = up {
                aligned(out, placed.normal, world::<N>(&reads[k], poses).direction(), *sign);
            }
        }
        Rule::Cam { outside } => {
            if let Some(u) = axis {
                let sign = if dot(placed.normal, u).middle() < 0.0 { -1 } else { 1 };
                aligned(out, placed.normal, u, sign);
            }
            let contact = if *outside { r } else { -r };
            scalar(out, signed(&placed, c) - contact, false);
            if placed.pieces.iter().any(|x| x.spline.is_some()) {
                scalar(out, tangential(&placed, c), false);
            }
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
                    spline: p.spline.as_ref().map(|s| Spline::placed(s, cam_at)),
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
