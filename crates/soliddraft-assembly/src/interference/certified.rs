//! Certified distances and signs between mesh features.
//!
//! Every distance here comes as an enclosure `[lo, hi]` of the true distance between the
//! features whose corners are the given binary64 points, and every orientation as its
//! true sign. Nothing rests on a rounded value: a denominator that computes positive, say,
//! decides nothing until its error bound keeps it above zero.
//!
//! Each query runs one algorithm twice over. First in binary64 with a running bound on
//! the error of every operation ([`Fast`]), which decides almost every branch and bounds
//! almost every distance to within a few units in the last place. Where a branch stays
//! open, or the enclosure is wider than the caller's tolerance, the same steps run again
//! in exact arithmetic ([`Exact`]). The corners are binary floats, so they are exact
//! dyadic rationals, and every quantity the algorithm forms from them is a polynomial,
//! or a ratio of two, that exact integer arithmetic evaluates without error.
//!
//! The algorithm reads distances as squares of signed quantities divided by positive
//! ones, never as differences of squares, so the binary64 bound stays tight near zero:
//! two triangles touching come out `[0, a few ulps]`, not `[0, sqrt(ulp)]`.

use std::cmp::Ordering;

use glam::DVec3;
use num_bigint::{BigInt, Sign};
use num_traits::{ToPrimitive, Zero};

/// An enclosure of a nonnegative quantity: it lies from `lo` to `hi`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Span {
    pub lo: f64,
    pub hi: f64,
}

impl Span {
    pub const ZERO: Span = Span { lo: 0.0, hi: 0.0 };
    /// Nothing known but that the quantity is nonnegative.
    const OPEN: Span = Span { lo: 0.0, hi: f64::INFINITY };
    /// No candidate: the identity of [`Span::min`].
    const NONE: Span = Span { lo: f64::INFINITY, hi: f64::INFINITY };

    /// The least of two quantities lies from the lesser `lo` to the lesser `hi`.
    pub fn min(self, other: Span) -> Span {
        Span { lo: self.lo.min(other.lo), hi: self.hi.min(other.hi) }
    }

    /// From an enclosure of a square to one of its root.
    fn sqrt(self) -> Span {
        Span { lo: root(self.lo, Ordering::Less), hi: root(self.hi, Ordering::Greater) }
    }

    pub fn width(self) -> f64 {
        self.hi - self.lo
    }
}

/// The square root of `x`, at least zero, rounded towards `side`: a root that is exact
/// stays exact, so that a square of 4 has a root of exactly 2.
fn root(x: f64, side: Ordering) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x.is_infinite() {
        return if side == Ordering::Less { f64::MAX } else { x };
    }
    let r = x.sqrt();
    // The rounded root against the exact one: its square against `x`, exactly.
    let (p, err) = two_product(r, r);
    let over = if (1e-290..1e290).contains(&x) { p.total_cmp(&x).then(err.total_cmp(&0.0)) } else { side.reverse() };
    match (side, over) {
        (Ordering::Less, Ordering::Greater) => r.next_down(),
        (Ordering::Greater, Ordering::Less) => r.next_up(),
        _ => r,
    }
}

/// The rounded product and its exact rounding error (Dekker's TwoProduct), for factors
/// whose product and halves stay in the normal range.
fn two_product(a: f64, b: f64) -> (f64, f64) {
    let split = |x: f64| {
        let c = 134_217_729.0 * x;
        let hi = c - (c - x);
        (hi, x - hi)
    };
    let p = a * b;
    let ((ah, al), (bh, bl)) = (split(a), split(b));
    (p, ((ah * bh - p) + ah * bl + al * bh) + al * bl)
}

/// `a - b`, rounded down: never more than the exact difference.
pub(crate) fn sub_down(a: f64, b: f64) -> f64 {
    let (s, err) = two_sum(a, -b);
    if err < 0.0 || !s.is_finite() { s.next_down() } else { s }
}

/// `a + b`, rounded up: never less than the exact sum.
pub(crate) fn add_up(a: f64, b: f64) -> f64 {
    let (s, err) = two_sum(a, b);
    if err > 0.0 || !s.is_finite() { s.next_up() } else { s }
}

/// The rounded sum and its exact rounding error (Knuth's TwoSum).
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    (s, (a - (s - bb)) + (b - bb))
}

/// The unit roundoff of binary64, round to nearest.
const U: f64 = f64::EPSILON / 2.0;

/// The factor that covers the rounding of the error bounds themselves. Each bound is a
/// sum of products of nonnegative numbers, rounded at most a few dozen times on its way,
/// so it is low by a relative 1e-14 at worst.
const SAFE: f64 = 1.0 + 1e-10;

/// The arithmetic the algorithm runs in.
trait Num: Clone {
    fn of(x: f64) -> Self;
    fn add(&self, other: &Self) -> Self;
    fn sub(&self, other: &Self) -> Self;
    fn mul(&self, other: &Self) -> Self;
    /// The sign of the exact value, or `None` when the arithmetic cannot tell.
    fn sign(&self) -> Option<Ordering>;
    /// An enclosure of the exact value, known to be at least zero.
    fn span(&self) -> Span;
    /// An enclosure of `self / den`, `self` known to be at least zero; `den` must be
    /// proven positive by [`Num::sign`] first.
    fn ratio(&self, den: &Self) -> Span;
}

/// A binary64 value and a bound on how far it is from the exact value of the expression
/// it was computed by: running error analysis. Each rounding is at most `U` of its
/// result, or for a product in the subnormal range at most half the least subnormal; a
/// sum is exact there. Exact zeros stay exact, so axis-aligned meshes decide their
/// many zero signs without the exact fallback.
#[derive(Clone, Copy, Debug)]
struct Fast {
    v: f64,
    e: f64,
}

impl Fast {
    /// The bound, covered for its own rounding.
    fn radius(&self) -> f64 {
        self.e * SAFE
    }

    fn sum(v: f64, e: f64) -> Fast {
        Fast { v, e: e + U * v.abs() }
    }
}

impl Num for Fast {
    fn of(x: f64) -> Fast {
        Fast { v: x, e: 0.0 }
    }

    fn add(&self, o: &Fast) -> Fast {
        Fast::sum(self.v + o.v, self.e + o.e)
    }

    fn sub(&self, o: &Fast) -> Fast {
        Fast::sum(self.v - o.v, self.e + o.e)
    }

    fn mul(&self, o: &Fast) -> Fast {
        let v = self.v * o.v;
        let carried = self.v.abs() * o.e + o.v.abs() * self.e + self.e * o.e;
        let rounding = if self.v == 0.0 || o.v == 0.0 {
            0.0
        } else if v.abs() >= f64::MIN_POSITIVE {
            U * v.abs()
        } else {
            f64::MIN_POSITIVE
        };
        Fast { v, e: carried + rounding }
    }

    fn sign(&self) -> Option<Ordering> {
        let r = self.radius();
        if !(self.v.is_finite() && r.is_finite()) {
            None
        } else if self.v == 0.0 && r == 0.0 {
            Some(Ordering::Equal)
        } else if self.v > r {
            Some(Ordering::Greater)
        } else if self.v < -r {
            Some(Ordering::Less)
        } else {
            None
        }
    }

    fn span(&self) -> Span {
        let r = self.radius();
        if !(self.v.is_finite() && r.is_finite()) {
            return Span::OPEN;
        }
        Span { lo: sub_down(self.v, r).max(0.0), hi: add_up(self.v, r) }
    }

    fn ratio(&self, den: &Fast) -> Span {
        let (num, den) = (self.span(), den.span());
        if !(den.lo > 0.0 && den.hi.is_finite() && num.hi.is_finite()) {
            return Span::OPEN;
        }
        if num.hi == 0.0 {
            return Span::ZERO;
        }
        Span { lo: (num.lo / den.hi).next_down().max(0.0), hi: (num.hi / den.lo).next_up() }
    }
}

/// An exact dyadic rational, `m * 2^e`.
#[derive(Clone, Debug)]
struct Exact {
    m: BigInt,
    e: i64,
}

impl Exact {
    /// `m` shifted left so that its exponent is `e`, at most its own.
    fn at(&self, e: i64) -> BigInt {
        &self.m << (self.e - e) as usize
    }
}

impl Num for Exact {
    fn of(x: f64) -> Exact {
        assert!(x.is_finite(), "a mesh coordinate is a finite number");
        if x == 0.0 {
            return Exact { m: BigInt::zero(), e: 0 };
        }
        let bits = x.to_bits();
        let exp = ((bits >> 52) & 0x7ff) as i64;
        let frac = bits & ((1 << 52) - 1);
        let (mut mant, mut e) = if exp == 0 { (frac, -1074) } else { (frac | 1 << 52, exp - 1075) };
        let tz = mant.trailing_zeros();
        mant >>= tz;
        e += i64::from(tz);
        let m = BigInt::from(mant);
        Exact { m: if x < 0.0 { -m } else { m }, e }
    }

    fn add(&self, o: &Exact) -> Exact {
        if self.m.is_zero() {
            return o.clone();
        }
        if o.m.is_zero() {
            return self.clone();
        }
        let e = self.e.min(o.e);
        Exact { m: self.at(e) + o.at(e), e }
    }

    fn sub(&self, o: &Exact) -> Exact {
        self.add(&Exact { m: -o.m.clone(), e: o.e })
    }

    fn mul(&self, o: &Exact) -> Exact {
        Exact { m: &self.m * &o.m, e: self.e + o.e }
    }

    fn sign(&self) -> Option<Ordering> {
        Some(match self.m.sign() {
            Sign::Minus => Ordering::Less,
            Sign::NoSign => Ordering::Equal,
            Sign::Plus => Ordering::Greater,
        })
    }

    fn span(&self) -> Span {
        bounds(&self.m, self.e, &BigInt::from(1u8))
    }

    fn ratio(&self, den: &Exact) -> Span {
        let Span { lo, hi } = bounds(&self.m, 0, &den.m);
        let shift = self.e - den.e;
        Span { lo: scale_down(lo, shift), hi: scale_up(hi, shift) }
    }
}

/// An enclosure of `num * 2^e / den`, for `num` at least zero and `den` positive.
fn bounds(num: &BigInt, e: i64, den: &BigInt) -> Span {
    if num.is_zero() {
        return Span::ZERO;
    }
    // A quotient of at least 64 bits: num / den lies in [q, q + 1] * 2^-k.
    let k = (den.bits() as i64 - num.bits() as i64 + 64).max(0);
    let q = (num << k as usize) / den;
    // Its top 64 bits: q lies in [t, t + 1] * 2^s.
    let s = q.bits().saturating_sub(64);
    let t = (q >> s as usize).to_u64().expect("64 bits");
    let shift = e - k + s as i64;
    // `as` rounds to nearest, so one step down covers `t`. An ulp at 2^63 and above is
    // at least 2048, so one step up covers `t + 1`.
    let lo = (t as f64).next_down().max(0.0);
    let hi = (t as f64).next_up();
    Span { lo: scale_down(lo, shift), hi: scale_up(hi, shift) }
}

/// `x * 2^k` for `x` at least zero, rounded down.
fn scale_down(x: f64, k: i64) -> f64 {
    let r = scale(x, k);
    if r < f64::MIN_POSITIVE {
        0.0
    } else if r.is_infinite() {
        f64::MAX
    } else {
        r
    }
}

/// `x * 2^k` for `x` at least zero, rounded up.
fn scale_up(x: f64, k: i64) -> f64 {
    let r = scale(x, k);
    if r < f64::MIN_POSITIVE && x > 0.0 { 2.0 * f64::MIN_POSITIVE } else { r }
}

/// `x * 2^k`, exact unless it leaves the normal range.
fn scale(x: f64, k: i64) -> f64 {
    let pow = |k: i64| f64::from_bits(((k + 1023) as u64) << 52);
    match k {
        -1022..=1023 => x * pow(k),
        -2044..=2046 => x * pow(k / 2) * pow(k - k / 2),
        _ if k < 0 => 0.0,
        _ => f64::INFINITY,
    }
}

type V<N> = [N; 3];

fn point<N: Num>(p: DVec3) -> V<N> {
    [N::of(p.x), N::of(p.y), N::of(p.z)]
}

fn sub<N: Num>(a: &V<N>, b: &V<N>) -> V<N> {
    [a[0].sub(&b[0]), a[1].sub(&b[1]), a[2].sub(&b[2])]
}

fn dot<N: Num>(a: &V<N>, b: &V<N>) -> N {
    a[0].mul(&b[0]).add(&a[1].mul(&b[1])).add(&a[2].mul(&b[2]))
}

fn cross<N: Num>(a: &V<N>, b: &V<N>) -> V<N> {
    [
        a[1].mul(&b[2]).sub(&a[2].mul(&b[1])),
        a[2].mul(&b[0]).sub(&a[0].mul(&b[2])),
        a[0].mul(&b[1]).sub(&a[1].mul(&b[0])),
    ]
}

/// The signed volume of `p, q, u, v`, six times over.
fn orient<N: Num>(p: &V<N>, q: &V<N>, u: &V<N>, v: &V<N>) -> Option<Ordering> {
    dot(&cross(&sub(q, p), &sub(u, p)), &sub(v, p)).sign()
}

use Ordering::{Equal, Greater, Less};

/// The squared distance from `p` to segment `ab`.
fn point_segment_sq<N: Num>(p: &V<N>, a: &V<N>, b: &V<N>) -> Span {
    let (d, ap) = (sub(b, a), sub(p, a));
    let along = dot(&ap, &d);
    let dd = dot(&d, &d);
    let past = along.sub(&dd);
    let to_a = || dot(&ap, &ap).span();
    let to_b = || {
        let bp = sub(p, b);
        dot(&bp, &bp).span()
    };
    match (along.sign(), past.sign()) {
        (Some(Less | Equal), _) => return to_a(),
        (_, Some(Greater | Equal)) => return to_b(),
        _ => {}
    }
    // Square to the line, divided by the length: never more than the distance to the
    // segment, and equal to it when `p` lies alongside.
    let c = cross(&ap, &d);
    let line = if dd.sign() == Some(Greater) { dot(&c, &c).ratio(&dd) } else { Span::OPEN };
    match (along.sign(), past.sign()) {
        (Some(Greater), Some(Less)) => line,
        _ => Span { lo: line.lo, hi: to_a().hi.min(to_b().hi) },
    }
}

/// The squared distance from `p` to the plane of triangle `t`, when `p` lies over the
/// triangle: [`Span::NONE`] when it is proven not to, so that the triangle's edges give
/// its distance, and a lower bound only while that is open. A triangle with no area has
/// no plane and gives nothing.
fn point_face_sq<N: Num>(p: &V<N>, t: &[V<N>; 3]) -> Span {
    let [a, b, c] = t;
    let n = cross(&sub(b, a), &sub(c, a));
    let nn = dot(&n, &n);
    match nn.sign() {
        Some(Equal) => return Span::NONE,
        Some(Greater) => {}
        _ => return Span::OPEN,
    }
    // Which side of each edge the point lies, seen along the normal: over the triangle
    // where none is negative.
    let mut open = false;
    for (u, v) in [(a, b), (b, c), (c, a)] {
        match dot(&cross(&sub(v, u), &sub(p, u)), &n).sign() {
            Some(Less) => return Span::NONE,
            None => open = true,
            _ => {}
        }
    }
    let h = dot(&sub(p, a), &n);
    let plane = h.mul(&h).ratio(&nn);
    // The plane is never farther than the triangle, so it bounds it from below either way.
    if open { Span { lo: plane.lo, hi: f64::INFINITY } } else { plane }
}

/// The squared distance between the inner points of two segments, when their nearest
/// points are inner points of both: [`Span::NONE`] when proven otherwise, since an end
/// then gives the distance, and a lower bound only while that is open.
fn edge_edge_sq<N: Num>(p1: &V<N>, q1: &V<N>, p2: &V<N>, q2: &V<N>) -> Span {
    let (d1, d2, r) = (sub(q1, p1), sub(q2, p2), sub(p1, p2));
    let c = cross(&d1, &d2);
    let denom = dot(&c, &c);
    match denom.sign() {
        // Parallel segments are nearest at an end of one.
        Some(Equal) => return Span::NONE,
        Some(Greater) => {}
        _ => return Span::OPEN,
    }
    let (a, b, e) = (dot(&d1, &d1), dot(&d1, &d2), dot(&d2, &d2));
    let (cr, f) = (dot(&d1, &r), dot(&d2, &r));
    // The lines are nearest at s = s_num / denom along the first and t = t_num / denom
    // along the second (Ericson 5.1.9); strictly inside both is the inner case.
    let s_num = b.mul(&f).sub(&cr.mul(&e));
    let t_num = a.mul(&f).sub(&b.mul(&cr));
    let mut open = false;
    for x in [&s_num, &t_num] {
        match (x.sign(), x.sub(&denom).sign()) {
            (Some(Greater), Some(Less)) => {}
            (Some(Less | Equal), _) | (_, Some(Greater | Equal)) => return Span::NONE,
            _ => open = true,
        }
    }
    // The lines' distance: never more than the segments', and theirs in the inner case.
    let h = dot(&r, &c);
    let lines = h.mul(&h).ratio(&denom);
    if open { Span { lo: lines.lo, hi: f64::INFINITY } } else { lines }
}

enum Pierce {
    Yes,
    No,
    Open,
}

/// Whether segment `pq` passes through the inside of triangle `t`, from strictly one side
/// of its plane to strictly the other. Touching it otherwise, with an end on it or
/// through its edge, is left to the distances to that end or edge, which are then zero.
fn pierces<N: Num>(p: &V<N>, q: &V<N>, t: &[V<N>; 3]) -> Pierce {
    let [a, b, c] = t;
    let n = cross(&sub(b, a), &sub(c, a));
    let (sp, sq) = (dot(&n, &sub(p, a)).sign(), dot(&n, &sub(q, a)).sign());
    if sp == Some(Equal) || sq == Some(Equal) || (sp.is_some() && sp == sq) {
        return Pierce::No;
    }
    let sides = [orient(p, q, a, b), orient(p, q, b, c), orient(p, q, c, a)];
    let has = |o: Ordering| sides.contains(&Some(o));
    // Mixed signs pass the line outside the triangle; a zero puts it through an edge.
    if (has(Greater) && has(Less)) || has(Equal) {
        return Pierce::No;
    }
    if sides.iter().all(Option::is_some) && sp.is_some() && sq.is_some() { Pierce::Yes } else { Pierce::Open }
}

/// The squared distance between two triangles. Crossing triangles are zero apart, and
/// an edge of one then passes through the other. Otherwise they are nearest from a corner
/// to the other's face or edge, or between two edges' inner points.
fn triangles_sq<N: Num>(a: &[V<N>; 3], b: &[V<N>; 3]) -> Span {
    let mut best = Span::NONE;
    for (s, t) in [(a, b), (b, a)] {
        for k in 0..3 {
            match pierces(&s[k], &s[(k + 1) % 3], t) {
                Pierce::Yes => return Span::ZERO,
                Pierce::Open => best.lo = 0.0,
                Pierce::No => {}
            }
        }
    }
    for (s, t) in [(a, b), (b, a)] {
        for p in s {
            best = best.min(point_face_sq(p, t));
            for k in 0..3 {
                best = best.min(point_segment_sq(p, &t[k], &t[(k + 1) % 3]));
            }
        }
    }
    for i in 0..3 {
        for j in 0..3 {
            best = best.min(edge_edge_sq(&a[i], &a[(i + 1) % 3], &b[j], &b[(j + 1) % 3]));
        }
    }
    best
}

fn point_triangle_sq<N: Num>(p: &V<N>, t: &[V<N>; 3]) -> Span {
    let mut best = point_face_sq(p, t);
    for k in 0..3 {
        best = best.min(point_segment_sq(p, &t[k], &t[(k + 1) % 3]));
    }
    best
}

#[cfg(test)]
fn segments_sq<N: Num>(p1: &V<N>, q1: &V<N>, p2: &V<N>, q2: &V<N>) -> Span {
    point_segment_sq(p1, p2, q2)
        .min(point_segment_sq(q1, p2, q2))
        .min(point_segment_sq(p2, p1, q1))
        .min(point_segment_sq(q2, p1, q1))
        .min(edge_edge_sq(p1, q1, p2, q2))
}

/// The binary64 enclosure, or the exact one when that is wider than `tol` and its lower
/// end is under `enough`, below which the caller needs it tight.
fn certify(tol: f64, enough: f64, fast: impl FnOnce() -> Span, exact: impl FnOnce() -> Span) -> Span {
    let span = fast().sqrt();
    if span.width() <= tol || span.lo >= enough { span } else { exact().sqrt() }
}

fn tri<N: Num>(t: &[DVec3; 3]) -> [V<N>; 3] {
    t.map(point)
}

/// The distance between two triangles. `tol` is the widest enclosure wanted; at or above
/// `enough`, any lower bound will do.
pub(crate) fn triangles(a: &[DVec3; 3], b: &[DVec3; 3], tol: f64, enough: f64) -> Span {
    certify(
        tol,
        enough,
        || triangles_sq::<Fast>(&tri(a), &tri(b)),
        || triangles_sq::<Exact>(&tri(a), &tri(b)),
    )
}

/// The distance from a point to a triangle.
pub(crate) fn point_triangle(p: DVec3, t: &[DVec3; 3], tol: f64, enough: f64) -> Span {
    certify(
        tol,
        enough,
        || point_triangle_sq::<Fast>(&point(p), &tri(t)),
        || point_triangle_sq::<Exact>(&point(p), &tri(t)),
    )
}

/// The distance from a point to a segment.
pub(crate) fn point_segment(p: DVec3, a: DVec3, b: DVec3, tol: f64) -> Span {
    certify(
        tol,
        f64::INFINITY,
        || point_segment_sq::<Fast>(&point(p), &point(a), &point(b)),
        || point_segment_sq::<Exact>(&point(p), &point(a), &point(b)),
    )
}

/// The distance between two segments.
#[cfg(test)]
pub(crate) fn segments(p1: DVec3, q1: DVec3, p2: DVec3, q2: DVec3, tol: f64) -> Span {
    certify(
        tol,
        f64::INFINITY,
        || segments_sq::<Fast>(&point(p1), &point(q1), &point(p2), &point(q2)),
        || segments_sq::<Exact>(&point(p1), &point(q1), &point(p2), &point(q2)),
    )
}

/// How a containment ray meets a triangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Crossing {
    /// Through the inside, from strictly one side to strictly the other.
    Clean,
    /// Not at all.
    Miss,
    /// Through an edge or a corner, along the plane, or from a start on the triangle:
    /// the ray says nothing.
    Grazes,
}

fn crossing_in<N: Num>(p: &V<N>, q: &V<N>, t: &[V<N>; 3]) -> Option<Crossing> {
    let [a, b, c] = t;
    let n = cross(&sub(b, a), &sub(c, a));
    match dot(&n, &n).sign()? {
        // A triangle with no area has no inside to cross; a ray through its line passes
        // through the edges of the triangles beside it, and grazes there.
        Equal => return Some(Crossing::Miss),
        _ => {}
    }
    let (sp, sq) = (dot(&n, &sub(p, a)).sign(), dot(&n, &sub(q, a)).sign());
    if sp.is_some() && sp == sq && sp != Some(Equal) {
        return Some(Crossing::Miss);
    }
    let sides = [orient(p, q, a, b), orient(p, q, b, c), orient(p, q, c, a)];
    let has = |o: Ordering| sides.contains(&Some(o));
    if has(Greater) && has(Less) {
        return Some(Crossing::Miss);
    }
    let (sp, sq) = (sp?, sq?);
    if sp == Equal && sq == Equal {
        return Some(Crossing::Grazes);
    }
    if sides.iter().any(Option::is_none) {
        return None;
    }
    // The line meets the plane once, inside the triangle or on its edge.
    if has(Equal) || sp == Equal || sq == Equal { Some(Crossing::Grazes) } else { Some(Crossing::Clean) }
}

/// How segment `pq` meets triangle `t`, decided exactly.
pub(crate) fn crossing(p: DVec3, q: DVec3, t: &[DVec3; 3]) -> Crossing {
    crossing_in::<Fast>(&point(p), &point(q), &tri(t))
        .unwrap_or_else(|| crossing_in::<Exact>(&point(p), &point(q), &tri(t)).expect("exact signs decide"))
}

/// The sign of the turn `a, b, c` in the plane.
fn turn<N: Num>(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<Ordering> {
    let d = |p: [f64; 2], q: [f64; 2], i: usize| N::of(p[i]).sub(&N::of(q[i]));
    d(b, a, 0).mul(&d(c, a, 1)).sub(&d(b, a, 1).mul(&d(c, a, 0))).sign()
}

/// Whether `x` lies in the closed plane triangle `t`, which must have area.
pub(crate) fn in_triangle_2d(x: [f64; 2], t: [[f64; 2]; 3]) -> bool {
    let exact = |a, b, c| turn::<Fast>(a, b, c).unwrap_or_else(|| turn::<Exact>(a, b, c).expect("exact"));
    let whole = exact(t[0], t[1], t[2]);
    if whole == Equal {
        return false;
    }
    (0..3).all(|k| {
        let s = exact(t[k], t[(k + 1) % 3], x);
        s == whole || s == Equal
    })
}

#[cfg(test)]
pub(crate) mod oracle {
    //! An independent exact distance between two triangles, in rationals, for tests: it
    //! clamps closest-point parameters as a textbook does rather than reading signs.

    use glam::DVec3;
    use num_rational::BigRational;
    use num_traits::{One, Signed, Zero};

    type Q = BigRational;
    type P = [Q; 3];

    fn q(x: f64) -> Q {
        BigRational::from_float(x).expect("finite")
    }

    pub fn point(p: DVec3) -> P {
        [q(p.x), q(p.y), q(p.z)]
    }

    fn sub(a: &P, b: &P) -> P {
        [&a[0] - &b[0], &a[1] - &b[1], &a[2] - &b[2]]
    }

    fn add_scaled(a: &P, d: &P, t: &Q) -> P {
        [&a[0] + &d[0] * t, &a[1] + &d[1] * t, &a[2] + &d[2] * t]
    }

    fn dot(a: &P, b: &P) -> Q {
        &a[0] * &b[0] + &a[1] * &b[1] + &a[2] * &b[2]
    }

    fn cross(a: &P, b: &P) -> P {
        [&a[1] * &b[2] - &a[2] * &b[1], &a[2] * &b[0] - &a[0] * &b[2], &a[0] * &b[1] - &a[1] * &b[0]]
    }

    fn sq(a: &P, b: &P) -> Q {
        let d = sub(a, b);
        dot(&d, &d)
    }

    fn clamp(t: Q) -> Q {
        if t.is_negative() {
            Q::zero()
        } else if t > Q::one() {
            Q::one()
        } else {
            t
        }
    }

    pub fn point_segment(p: &P, a: &P, b: &P) -> Q {
        let d = sub(b, a);
        let dd = dot(&d, &d);
        if dd.is_zero() {
            return sq(p, a);
        }
        let t = clamp(dot(&sub(p, a), &d) / dd);
        sq(p, &add_scaled(a, &d, &t))
    }

    /// Barycentric weights `(u, v)` of the projection of `p` on the plane of `abc`, by
    /// the normal equations, or `None` for a triangle with no area.
    fn project(p: &P, a: &P, b: &P, c: &P) -> Option<(Q, Q)> {
        let (e1, e2, w) = (sub(b, a), sub(c, a), sub(p, a));
        let (g11, g12, g22) = (dot(&e1, &e1), dot(&e1, &e2), dot(&e2, &e2));
        let det = &g11 * &g22 - &g12 * &g12;
        if det.is_zero() {
            return None;
        }
        let (r1, r2) = (dot(&w, &e1), dot(&w, &e2));
        Some(((&g22 * &r1 - &g12 * &r2) / &det, (&g11 * &r2 - &g12 * &r1) / det))
    }

    pub fn point_triangle(p: &P, t: &[P; 3]) -> Q {
        let [a, b, c] = t;
        if let Some((u, v)) = project(p, a, b, c)
            && !u.is_negative()
            && !v.is_negative()
            && &u + &v <= Q::one()
        {
            let x = add_scaled(&add_scaled(a, &sub(b, a), &u), &sub(c, a), &v);
            return sq(p, &x);
        }
        [point_segment(p, a, b), point_segment(p, b, c), point_segment(p, c, a)].into_iter().min().unwrap()
    }

    pub fn segments(p1: &P, q1: &P, p2: &P, q2: &P) -> Q {
        let ends = [point_segment(p1, p2, q2), point_segment(q1, p2, q2), point_segment(p2, p1, q1), point_segment(q2, p1, q1)]
            .into_iter()
            .min()
            .unwrap();
        let (d1, d2, r) = (sub(q1, p1), sub(q2, p2), sub(p1, p2));
        let (a, b, e, c, f) = (dot(&d1, &d1), dot(&d1, &d2), dot(&d2, &d2), dot(&d1, &r), dot(&d2, &r));
        let denom = &a * &e - &b * &b;
        if denom.is_zero() {
            return ends;
        }
        let s = (&b * &f - &c * &e) / &denom;
        let t = (&a * &f - &b * &c) / &denom;
        let inside = |x: &Q| !x.is_negative() && *x <= Q::one();
        if inside(&s) && inside(&t) {
            ends.min(sq(&add_scaled(p1, &d1, &s), &add_scaled(p2, &d2, &t)))
        } else {
            ends
        }
    }

    /// Whether segment `pq` meets triangle `t` where it crosses the triangle's plane.
    fn meets(p: &P, q: &P, t: &[P; 3]) -> bool {
        let [a, b, c] = t;
        let n = cross(&sub(b, a), &sub(c, a));
        let den = dot(&n, &sub(q, p));
        if den.is_zero() {
            return false;
        }
        let l = dot(&n, &sub(a, p)) / den;
        if l.is_negative() || l > Q::one() {
            return false;
        }
        let x = add_scaled(p, &sub(q, p), &l);
        project(&x, a, b, c).is_some_and(|(u, v)| !u.is_negative() && !v.is_negative() && &u + &v <= Q::one())
    }

    /// The exact squared distance between two triangles.
    pub fn triangles(a: &[P; 3], b: &[P; 3]) -> Q {
        for (s, t) in [(a, b), (b, a)] {
            for k in 0..3 {
                if meets(&s[k], &s[(k + 1) % 3], t) {
                    return Q::zero();
                }
            }
        }
        let mut best: Option<Q> = None;
        let mut take = |x: Q| {
            if best.as_ref().is_none_or(|b| x < *b) {
                best = Some(x);
            }
        };
        for (s, t) in [(a, b), (b, a)] {
            for p in s {
                take(point_triangle(p, t));
            }
        }
        for i in 0..3 {
            for j in 0..3 {
                take(segments(&a[i], &a[(i + 1) % 3], &b[j], &b[(j + 1) % 3]));
            }
        }
        best.unwrap()
    }

    /// Whether `span` encloses the root of `exact_sq`.
    pub fn encloses(span: super::Span, exact_sq: &Q) -> bool {
        let lo_ok = span.lo <= 0.0 || q(span.lo) * q(span.lo) <= *exact_sq;
        let hi_ok = span.hi.is_infinite() || q(span.hi) * q(span.hi) >= *exact_sq;
        lo_ok && hi_ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z)
    }

    #[test]
    fn exact_dyadics_round_trip_and_enclose() {
        for x in [0.0, 1.0, -3.5, 0.015625, 21000.068267152124, 1e-300, 5e-324, 1.7e308] {
            let s = Exact::of(x).mul(&Exact::of(x)).span();
            assert!(s.lo <= x * x && x * x <= s.hi, "{x}: {s:?}");
            assert!(s.hi - s.lo <= 4.0 * f64::EPSILON * x * x + 4.0 * f64::MIN_POSITIVE, "{x}: {s:?}");
        }
        // 1/3 is no float: the enclosure brackets it within an ulp or two.
        let r = Exact::of(1.0).ratio(&Exact::of(3.0));
        assert!(r.lo < 1.0 / 3.0 + 1e-16 && r.hi > 1.0 / 3.0 - 1e-16 && r.hi - r.lo < 3e-16, "{r:?}");
    }

    #[test]
    fn exact_zeros_stay_decided_in_binary64() {
        // Axis-aligned edges cross to an exact zero, decided without the exact fallback.
        let (p, q) = (point::<Fast>(d(0.0, 0.0, 0.0)), point::<Fast>(d(10.0, 0.0, 0.0)));
        let (u, v) = (point::<Fast>(d(0.0, 2.0, 0.0)), point::<Fast>(d(5.0, 2.0, 0.0)));
        let c = cross(&sub(&q, &p), &sub(&v, &u));
        assert_eq!(dot(&c, &c).sign(), Some(Equal));
    }

    #[test]
    fn the_review_skinny_triangle_is_its_true_distance_from_the_point() {
        // The closest point is (21000, 0.015625, 0), inside the triangle, 0.015625 away.
        let t = [d(0.0, 0.0, 0.0), d(30000.0, 0.0, 0.0), d(30000.0, 0.0625, 0.0)];
        let p = d(21000.0, 0.015625, 0.015625);
        let s = point_triangle(p, &t, 1e-12, f64::INFINITY);
        assert!(s.lo <= 0.015625 && s.hi >= 0.015625 && s.width() < 1e-15, "{s:?}");
    }

    #[test]
    fn crossings_are_decided_exactly() {
        let t = [d(0.0, 0.0, 0.0), d(1.0, 0.0, 0.0), d(0.0, 1.0, 0.0)];
        let up = |x: f64, y: f64| crossing(d(x, y, -1.0), d(x, y, 1.0), &t);
        assert_eq!(up(0.25, 0.25), Crossing::Clean);
        assert_eq!(up(2.0, 0.25), Crossing::Miss);
        assert_eq!(up(0.5, 0.0), Crossing::Grazes, "through an edge");
        assert_eq!(up(0.5, 0.5), Crossing::Grazes, "through the hypotenuse");
        assert_eq!(up(0.5, 0.5 + f64::EPSILON), Crossing::Miss, "a hair outside");
        assert_eq!(up(0.5, 0.5 - f64::EPSILON / 2.0), Crossing::Clean, "a hair inside");
        assert_eq!(crossing(d(0.25, 0.25, 0.0), d(0.25, 0.25, 1.0), &t), Crossing::Grazes, "from the triangle");
        assert_eq!(crossing(d(5.0, 5.0, 0.0), d(5.0, 5.0, 1.0), &t), Crossing::Miss, "from its plane, outside");
        // A sliver far thinner than any relative tolerance is still crossed.
        let sliver = [d(0.0, 0.0, 0.0), d(1e4, 0.0, 0.0), d(1e4, 1e-9, 0.0)];
        assert_eq!(crossing(d(5e3, 2.5e-10, -1.0), d(5e3, 2.5e-10, 1.0), &sliver), Crossing::Clean);
    }

    #[test]
    fn plane_triangles_hold_their_closed_area() {
        let t = [[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]];
        assert!(in_triangle_2d([1.0, 1.0], t));
        assert!(in_triangle_2d([2.0, 2.0], t), "on the hypotenuse");
        assert!(!in_triangle_2d([2.0, 2.0 + 4.0 * f64::EPSILON], t));
        assert!(!in_triangle_2d([1.0, 1.0], [[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]]), "no area");
    }
}

#[cfg(test)]
mod adversarial {
    //! Triangle pairs built to break binary64: long and thin, a hair off parallel, nearly
    //! without area, and far from the origin. An independent rational computation gives
    //! each pair's exact distance, and every enclosure must hold it: the binary64 one alone,
    //! the exact one alone, and the certified one the meshes use.

    use super::*;
    use proptest::prelude::*;

    /// A coordinate as the kernel hands it over, in f32.
    fn f(x: f64) -> f64 {
        x as f32 as f64
    }

    fn v(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(f(x), f(y), f(z))
    }

    /// A magnitude spread evenly in its exponent, from `10^lo` to `10^hi`.
    fn magnitude(lo: f64, hi: f64) -> impl Strategy<Value = f64> {
        (lo..hi).prop_map(|e| 10f64.powf(e))
    }

    fn signed(lo: f64, hi: f64) -> impl Strategy<Value = f64> {
        (magnitude(lo, hi), any::<bool>()).prop_map(|(m, neg)| if neg { -m } else { m })
    }

    fn unit() -> impl Strategy<Value = DVec3> {
        (-1.0..1.0f64, -1.0..1.0f64, -1.0..1.0f64)
            .prop_map(|(x, y, z)| DVec3::new(x, y, z).normalize_or(DVec3::X))
    }

    type Pair = ([DVec3; 3], [DVec3; 3]);

    /// A long sliver, and a small triangle over its inside, just off its plane.
    fn long_thin() -> impl Strategy<Value = Pair> {
        (magnitude(2.0, 5.0), magnitude(-4.0, 0.0), 0.02..0.98f64, 0.05..0.95f64, signed(-7.0, -1.0), unit(), unit(), magnitude(-4.0, 1.0))
            .prop_map(|(len, width, u, w, gap, e1, e2, size)| {
                let a = [v(0.0, 0.0, 0.0), v(len, 0.0, 0.0), v(len, width, 0.0)];
                let p = DVec3::new(len * u, width * u * w, gap);
                let b = [v(p.x, p.y, p.z), v(p.x + e1.x * size, p.y + e1.y * size, p.z + e1.z.abs() * size * gap.signum()),
                    v(p.x + e2.x * size, p.y + e2.y * size, p.z + e2.z.abs() * size * gap.signum())];
                (a, b)
            })
    }

    /// Two long thin triangles whose long edges cross a hair apart, a hair off parallel.
    fn near_parallel() -> impl Strategy<Value = Pair> {
        (magnitude(1.0, 4.5), signed(-14.0, -3.0), signed(-6.0, -1.0), magnitude(-3.0, 1.0), -0.5..0.5f64)
            .prop_map(|(half, angle, gap, width, slide)| {
                let (c, s) = (angle.cos(), angle.sin());
                let a = [v(-half, 0.0, 0.0), v(half, 0.0, 0.0), v(0.0, -width, -width)];
                let x0 = slide * half;
                let b = [
                    v(x0 - half * c, -half * s, gap),
                    v(x0 + half * c, half * s, gap),
                    v(x0, width, gap + width * gap.signum()),
                ];
                (a, b)
            })
    }

    /// A triangle whose third corner sits a hair off the line of the other two, and a
    /// triangle near it.
    fn near_degenerate() -> impl Strategy<Value = Pair> {
        (unit(), magnitude(-1.0, 3.0), 0.0..1.0f64, signed(-12.0, -2.0), unit(), unit(), signed(-6.0, 0.0), magnitude(-3.0, 1.0))
            .prop_map(|(dir, len, t, off, e1, e2, gap, size)| {
                let q = dir * len;
                let side = dir.any_orthonormal_vector();
                let r = q * t + side * (off * len);
                let a = [v(0.0, 0.0, 0.0), v(q.x, q.y, q.z), v(r.x, r.y, r.z)];
                let p = q * 0.5 + side.cross(dir) * gap;
                let b = [v(p.x, p.y, p.z), v(p.x + e1.x * size, p.y + e1.y * size, p.z + e1.z * size),
                    v(p.x + e2.x * size, p.y + e2.y * size, p.z + e2.z * size)];
                (a, b)
            })
    }

    /// Any of the above, or two small triangles that may cross, moved far from the origin.
    fn far() -> impl Strategy<Value = Pair> {
        let small = (unit(), unit(), unit(), unit(), unit(), unit(), unit(), magnitude(-2.0, 1.0))
            .prop_map(|(a0, a1, a2, b0, b1, b2, off, s)| {
                let w = |p: DVec3, o: DVec3| v(p.x * s + o.x, p.y * s + o.y, p.z * s + o.z);
                ([w(a0, DVec3::ZERO), w(a1, DVec3::ZERO), w(a2, DVec3::ZERO)], [w(b0, off * s), w(b1, off * s), w(b2, off * s)])
            });
        (prop_oneof![small, long_thin(), near_parallel(), near_degenerate()], signed(3.0, 7.0), signed(3.0, 7.0), signed(3.0, 7.0))
            .prop_map(|((a, b), x, y, z)| {
                let o = DVec3::new(x, y, z);
                let at = |p: DVec3| v(p.x + o.x, p.y + o.y, p.z + o.z);
                (a.map(at), b.map(at))
            })
    }

    fn pairs() -> impl Strategy<Value = Pair> {
        prop_oneof![long_thin(), near_parallel(), near_degenerate(), far()]
    }

    /// Whether a squared enclosure holds the exact square.
    fn holds_sq(span: Span, exact: &num_rational::BigRational) -> bool {
        use num_rational::BigRational;
        let q = |x: f64| BigRational::from_float(x).expect("finite");
        (span.lo <= 0.0 || q(span.lo) <= *exact) && (span.hi.is_infinite() || q(span.hi) >= *exact)
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 1500, ..ProptestConfig::default() })]

        #[test]
        fn every_enclosure_holds_the_exact_distance((a, b) in pairs()) {
            let exact = oracle::triangles(&a.map(oracle::point), &b.map(oracle::point));
            let fast = triangles_sq::<Fast>(&tri(&a), &tri(&b));
            prop_assert!(holds_sq(fast, &exact), "binary64 {fast:?} misses {exact}");
            let exact_span = triangles_sq::<Exact>(&tri(&a), &tri(&b));
            prop_assert!(holds_sq(exact_span, &exact), "exact {exact_span:?} misses {exact}");
            let tol = 1e-12 * (1.0 + a.iter().chain(&b).map(|p| p.abs().max_element()).fold(0.0, f64::max));
            let span = triangles(&a, &b, tol, f64::INFINITY);
            prop_assert!(oracle::encloses(span, &exact), "certified {span:?} misses {exact}");
            prop_assert!(span.width() <= tol.max(1e-15 * span.hi), "certified {span:?} wider than {tol}");
            // Either order.
            let swapped = triangles(&b, &a, tol, f64::INFINITY);
            prop_assert!(oracle::encloses(swapped, &exact), "swapped {swapped:?} misses {exact}");
        }

        #[test]
        fn points_and_segments_are_enclosed((a, b) in pairs()) {
            let (pa, pb) = (a.map(oracle::point), b.map(oracle::point));
            for k in 0..3 {
                let exact = oracle::point_triangle(&pb[k], &pa);
                let fast = point_triangle_sq::<Fast>(&point(b[k]), &tri(&a));
                prop_assert!(holds_sq(fast, &exact), "binary64 point {fast:?} misses {exact}");
                let span = point_triangle(b[k], &a, 1e-9, f64::INFINITY);
                prop_assert!(oracle::encloses(span, &exact), "point {span:?} misses {exact}");
                for j in 0..3 {
                    let (p1, q1, p2, q2) = (a[k], a[(k + 1) % 3], b[j], b[(j + 1) % 3]);
                    let exact = oracle::segments(&pa[k], &pa[(k + 1) % 3], &pb[j], &pb[(j + 1) % 3]);
                    let fast = segments_sq::<Fast>(&point(p1), &point(q1), &point(p2), &point(q2));
                    prop_assert!(holds_sq(fast, &exact), "binary64 segments {fast:?} miss {exact}");
                    let span = segments(p1, q1, p2, q2, 1e-9);
                    prop_assert!(oracle::encloses(span, &exact), "segments {span:?} miss {exact}");
                }
            }
        }

        #[test]
        fn binary64_signs_agree_with_exact_ones((a, b) in pairs()) {
            // Four corners close to one plane: whenever binary64 decides, it is right.
            for p in &b {
                let (fast, exact) = (
                    orient(&point::<Fast>(a[0]), &point(a[1]), &point(a[2]), &point(*p)),
                    orient(&point::<Exact>(a[0]), &point(a[1]), &point(a[2]), &point(*p)),
                );
                prop_assert!(fast.is_none() || fast == exact, "{fast:?} against {exact:?}");
            }
        }
    }
}
