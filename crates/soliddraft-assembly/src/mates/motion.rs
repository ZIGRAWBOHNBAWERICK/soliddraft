//! Bounded rigid motions. Exact groups keep the original solver and serialization.

mod interval;
mod native;
use super::solver::{
    self, Along, Body, Constraint, FreeMotion, LocalAxis, LocalPlane, LocalPoint, Mirrored, Slide, SlotMode, WidthMode,
};
use super::{MateRefusal, RefusalCode};
use crate::placement::Placement;
use glam::{DQuat, DVec3};
use interval::{Interval, Number, radians};
pub(super) use native::{native_constraint, projection};
use num_rational::BigRational;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const MM: f64 = 1e-9;
const RAD: f64 = 1e-9;
const ITERATIONS: usize = 120;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DistanceEntity {
    Point(LocalPoint),
    Axis(LocalAxis),
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Relation {
    Native { reads: Vec<super::command::Read>, rule: Box<super::bodies::Rule> },
    Angle { a: Body, b: Body, reference: Body, directions: [[i8; 3]; 3], min: f64, max: f64, limit: bool },
    Separation { a: Body, b: Body, ends: Box<[DistanceEntity; 2]>, distance: BigRational, parallel: Option<i8> },
}
impl Relation {
    pub fn bodies(&self) -> Vec<&Body> {
        match self {
            Self::Native { reads, rule } => {
                let mut bodies: Vec<_> = reads.iter().map(|r| &r.body).collect();
                if let super::bodies::Rule::Coupler { references, .. } = rule.as_ref() {
                    bodies.extend(references.iter().flatten());
                }
                bodies
            }
            Self::Angle { a, b, reference, .. } => vec![a, b, reference],
            Self::Separation { a, b, .. } => vec![a, b],
        }
    }
}

/// A residual enclosure at the stored binary64 pose. Bounds are absolute, per scalar row.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BoundedProof {
    /// Mate whose equations were checked.
    pub mate: String,
    /// Always `bounded`; never an exact claim.
    pub certainty: &'static str,
    /// Maximum certified positional residual, in millimetres.
    pub residual_mm: f64,
    /// Conservative angular error bound, in radians.
    pub residual_angle: f64,
    /// Allowed positional residual, in millimetres.
    pub bound_mm: f64,
    /// Allowed angular error, in radians.
    pub bound_angle: f64,
}

type V<N> = [N; 3];
fn add<N: Number>(a: V<N>, b: V<N>) -> V<N> {
    std::array::from_fn(|k| a[k] + b[k])
}
fn neg<N: Number>(a: V<N>) -> V<N> {
    a.map(|x| -x)
}
fn sub<N: Number>(a: V<N>, b: V<N>) -> V<N> {
    add(a, neg(b))
}
fn mul<N: Number>(a: V<N>, b: N) -> V<N> {
    a.map(|x| x * b)
}
fn dot<N: Number>(a: V<N>, b: V<N>) -> N {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross<N: Number>(a: V<N>, b: V<N>) -> V<N> {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm<N: Number>(a: V<N>) -> N {
    dot(a, a).sqrt()
}
fn unit<N: Number>(a: V<N>) -> V<N> {
    mul(a, N::f(1.0) / norm(a))
}
fn direction<N: Number>(a: [i8; 3]) -> V<N> {
    a.map(|x| N::f(x as f64))
}
fn local<N: Number>(a: &LocalPoint) -> V<N> {
    a.each_ref().map(N::q)
}
fn rotate<N: Number>(p: &Placement, v: V<N>) -> V<N> {
    let [x, y, z, w] = p.rotation.to_array().map(N::f);
    let u = [x, y, z];
    let square = x * x + y * y + z * z + w * w;
    // q v q^-1, so even a stored quaternion's normalization rounding is enclosed.
    add(v, mul(add(mul(cross(u, v), w), cross(u, cross(u, v))), N::f(2.0) / square))
}
fn point<N: Number>(p: &Placement, v: &LocalPoint) -> V<N> {
    add(rotate(p, local(v)), p.translation.to_array().map(N::f))
}
fn plane<N: Number>(p: &Placement, v: &LocalPlane) -> (V<N>, N) {
    let n = rotate(p, direction(v.normal));
    (n, N::q(&v.offset) + dot(n, p.translation.to_array().map(N::f)))
}
fn axis<N: Number>(p: &Placement, v: &LocalAxis) -> (V<N>, V<N>) {
    (rotate(p, direction(v.direction)), point(p, &v.through))
}
fn mirror<N: Number>(v: V<N>, n: V<N>) -> V<N> {
    sub(v, mul(n, N::f(2.0) * dot(v, n)))
}
fn reflected<N: Number>(v: V<N>, n: V<N>, offset: N) -> V<N> {
    sub(v, mul(n, N::f(2.0) * (dot(v, n) - offset)))
}

#[derive(Clone, Copy)]
struct Row<N: Number> {
    value: N,
    angular: bool,
}
fn scalar<N: Number>(out: &mut Vec<Row<N>>, v: N, angular: bool) {
    out.push(Row { value: v, angular });
}
fn vector<N: Number>(out: &mut Vec<Row<N>>, v: V<N>, angular: bool) {
    for c in v {
        scalar(out, c, angular);
    }
}
fn range<N: Number>(out: &mut Vec<Row<N>>, v: N, lo: N, hi: N, angular: bool, rank: bool) {
    if rank {
        return;
    }
    // Both violations are evaluated, including during interval certification.
    let positive = |x: N| x.positive();
    scalar(out, positive(lo - v), angular);
    scalar(out, positive(v - hi), angular);
}
fn aligned<N: Number>(out: &mut Vec<Row<N>>, a: V<N>, b: V<N>, sign: i8) {
    if sign == 0 {
        scalar(out, dot(a, b), true);
    } else {
        vector(out, sub(b, mul(a, N::f(sign as f64))), true);
    }
}
fn on_axis<N: Number>(out: &mut Vec<Row<N>>, n: V<N>, a: V<N>, b: V<N>) {
    vector(out, cross(sub(b, a), n), false);
}
fn turn<N: Number>(out: &mut Vec<Row<N>>, a: &Placement, b: &Placement, relative: DQuat) {
    let rel = Placement { rotation: relative, translation: DVec3::ZERO };
    for v in [[1, 0, 0], [0, 1, 0]] {
        vector(out, sub(rotate(b, direction(v)), rotate(a, rotate(&rel, direction(v)))), true);
    }
}
fn slide<N: Number>(s: &Slide, poses: &BTreeMap<Body, Placement>) -> N {
    let p = &poses[&s.body];
    let n = rotate(p, direction(s.direction));
    let from =
        s.from.as_ref().map_or_else(|| s.frame.each_ref().map(N::q), |b| poses[b].translation.to_array().map(N::f));
    N::q(&s.offset) + dot(n, sub(p.translation.to_array().map(N::f), from))
}
fn width<N: Number>(out: &mut Vec<Row<N>>, a: (V<N>, N, N), b: (V<N>, N, N), mode: &WidthMode, rank: bool) {
    let (na, oa, ga) = a;
    let (nb, ob, tb) = b;
    let sign = if dot(na, nb).middle() < 0.0 { -1.0 } else { 1.0 };
    let b = ob * N::f(sign);
    let t = tb * N::f(sign);
    let (glo, ghi) = if oa.middle() < ga.middle() { (oa, ga) } else { (ga, oa) };
    let (tlo, thi) = if b.middle() < t.middle() { (b, t) } else { (t, b) };
    let play = (ghi - glo) - (thi - tlo);
    let gap = tlo - glo;
    match mode {
        WidthMode::Free => range(out, gap, N::f(0.0), play, false, rank),
        WidthMode::Centered => scalar(out, gap - play / N::f(2.0), false),
        WidthMode::Gap { distance, flipped } => {
            let v = if *flipped { play - gap } else { gap };
            scalar(out, v - N::f(*distance), false);
        }
        WidthMode::Share { fraction, flipped } => {
            let v = if *flipped { play - gap } else { gap };
            scalar(out, v - play * N::q(fraction), false);
        }
    }
}
fn distance<N: Number>(ends: &[DistanceEntity; 2], a: &Placement, b: &Placement) -> N {
    let read = |e: &DistanceEntity, p: &Placement| match e {
        DistanceEntity::Point(v) => (point(p, v), None),
        DistanceEntity::Axis(v) => {
            let (n, x) = axis(p, v);
            (x, Some(n))
        }
    };
    let ((p, u), (q, v)) = (read(&ends[0], a), read(&ends[1], b));
    let d = sub(q, p);
    match (u, v) {
        (None, None) => norm(d),
        (Some(n), None) | (None, Some(n)) => norm(cross(d, n)),
        (Some(u), Some(v)) => {
            let n = cross(u, v);
            if norm(n).middle() < 1e-7 {
                norm(cross(d, u))
            } else {
                let signed = dot(d, unit(n));
                if signed.middle() < 0.0 { -signed } else { signed }
            }
        }
    }
}

fn residual<N: Number>(c: &Constraint, poses: &BTreeMap<Body, Placement>, rank: bool) -> Vec<Row<N>> {
    let mut out = Vec::new();
    match c {
        Constraint::Origin { body, axes, at, .. } => {
            let p = &poses[body];
            vector(&mut out, sub(p.translation.to_array().map(N::f), at.translation.to_array().map(N::f)), false);
            if *axes {
                turn(&mut out, at, p, DQuat::IDENTITY);
            }
        }
        Constraint::Lock { a, b, relative, .. } => {
            let (pa, pb) = (&poses[a], &poses[b]);
            let target =
                add(pa.translation.to_array().map(N::f), rotate(pa, relative.translation.to_array().map(N::f)));
            vector(&mut out, sub(pb.translation.to_array().map(N::f), target), false);
            turn(&mut out, pa, pb, relative.rotation);
        }
        Constraint::Turn { a, b, relative, .. } => turn(&mut out, &poses[a], &poses[b], *relative),
        Constraint::Parallel { a, b, plane_a, plane_b, sign, along, .. } => {
            let (na, oa) = plane::<N>(&poses[a], plane_a);
            let (nb, ob) = plane::<N>(&poses[b], plane_b);
            aligned(&mut out, na, nb, *sign);
            let gap = ob * N::f(*sign as f64) - oa;
            match along {
                Some(Along::Distance(d)) => scalar(&mut out, gap - N::f(*d), false),
                Some(Along::Range(lo, hi)) => range(&mut out, gap, N::f(*lo), N::f(*hi), false, rank),
                Some(Along::Width { groove, tab, mode }) => {
                    let (gn, go) = plane::<N>(&poses[a], groove);
                    let (tn, to) = plane::<N>(&poses[b], tab);
                    let ga = go * N::f(if dot(gn, na).middle() < 0.0 { -1.0 } else { 1.0 });
                    let tb = to * N::f(if dot(tn, nb).middle() < 0.0 { -1.0 } else { 1.0 });
                    width(&mut out, (na, oa, ga), (nb, ob, tb), mode, rank);
                }
                None => {}
            }
        }
        Constraint::Perpendicular { a, b, plane_a, plane_b, .. } => {
            let (na, _) = plane::<N>(&poses[a], plane_a);
            let (nb, _) = plane::<N>(&poses[b], plane_b);
            scalar(&mut out, dot(na, nb), true);
        }
        Constraint::Axes { a, b, axis_a, axis_b, sign, coaxial, .. } => {
            let (na, pa) = axis::<N>(&poses[a], axis_a);
            let (nb, pb) = axis::<N>(&poses[b], axis_b);
            aligned(&mut out, na, nb, *sign);
            if *coaxial {
                on_axis(&mut out, na, pa, pb);
            }
        }
        Constraint::OnAxis { a, b, axis: ax, point: p, .. } => {
            let (n, x) = axis::<N>(&poses[a], ax);
            on_axis(&mut out, n, x, point(&poses[b], p));
        }
        Constraint::Points { a, b, point_a, point_b, .. } => {
            vector(&mut out, sub(point(&poses[b], point_b), point(&poses[a], point_a)), false)
        }
        Constraint::FromPlane { a, b, plane: p, point: v, direction: dir, gap, upto, .. } => {
            let (n, o) = plane::<N>(&poses[a], p);
            let d = dot(n, point(&poses[b], v)) - o;
            if let Some(dir) = dir {
                scalar(&mut out, dot(n, rotate(&poses[b], direction(*dir))), true);
            }
            if let Some(hi) = upto {
                range(&mut out, d, N::q(gap), N::q(hi), false, rank);
            } else {
                scalar(&mut out, d - N::q(gap), false);
            }
        }
        Constraint::Coupler { a, b, ratio, reverse, offset, .. } => {
            let sign = if *reverse { 1.0 } else { -1.0 };
            scalar(
                &mut out,
                N::q(&ratio[0]) * slide(b, poses) + N::f(sign) * N::q(&ratio[1]) * slide(a, poses) - N::q(offset),
                false,
            );
        }
        Constraint::Symmetric { a, b, s, plane: p, ends, sign, .. } => {
            let (n, o) = plane::<N>(&poses[s], p);
            match ends {
                Mirrored::Points(p, q) => {
                    vector(&mut out, sub(point(&poses[b], q), reflected(point(&poses[a], p), n, o)), false)
                }
                Mirrored::Axes(p, q) => {
                    let (na, pa) = axis::<N>(&poses[a], p);
                    let (nb, pb) = axis::<N>(&poses[b], q);
                    aligned(&mut out, mirror(na, n), nb, *sign);
                    on_axis(&mut out, nb, reflected(pa, n, o), pb);
                }
                Mirrored::Planes(p, q) => {
                    let (na, oa) = plane::<N>(&poses[a], p);
                    let (nb, ob) = plane::<N>(&poses[b], q);
                    let image = mirror(na, n);
                    aligned(&mut out, image, nb, *sign);
                    let on = reflected(mul(na, oa), n, o);
                    scalar(&mut out, dot(image, on) - ob * N::f(*sign as f64), false);
                }
            }
        }
        Constraint::Slot { a, b, slot, axis: pin, sign, mode, .. } => {
            let pa = &poses[a];
            let pb = &poses[b];
            let n = rotate(pa, direction(slot.depth));
            let start = point(pa, &slot.ends[0]);
            let end = point(pa, &slot.ends[1]);
            let run = sub(end, start);
            let length = norm(run);
            let run = unit(run);
            let (pn, p) = axis::<N>(pb, pin);
            aligned(&mut out, n, pn, *sign);
            let delta = sub(p, start);
            scalar(&mut out, dot(delta, cross(n, run)), false);
            let along = dot(delta, run);
            match mode {
                SlotMode::Free => range(&mut out, along, N::f(0.0), length, false, rank),
                SlotMode::Centre => scalar(&mut out, along - length / N::f(2.0), false),
                SlotMode::Distance { distance, flipped } => scalar(
                    &mut out,
                    if *flipped { length - along - N::q(distance) } else { along - N::q(distance) },
                    false,
                ),
                SlotMode::Share { fraction, flipped } => scalar(
                    &mut out,
                    if *flipped { length - along - length * N::q(fraction) } else { along - length * N::q(fraction) },
                    false,
                ),
            }
        }
        Constraint::Spin { a, b, normal, edges, .. } => {
            let n = rotate(&poses[a], direction(*normal));
            if let Some((ea, eb)) = edges {
                let mut best = None;
                for u in ea {
                    for v in eb {
                        let u = unit(rotate(&poses[a], u.to_array().map(N::f)));
                        let v = unit(rotate(&poses[b], v.to_array().map(N::f)));
                        let r = dot(cross(u, v), n);
                        if best.is_none_or(|x: N| r.middle().abs() < x.middle().abs()) {
                            best = Some(r);
                        }
                    }
                }
                scalar(&mut out, best.unwrap_or(N::f(f64::NAN)), true);
            } else {
                scalar(&mut out, N::f(0.0), true);
            }
        }
        Constraint::Motion { relation, .. } => match relation.as_ref() {
            Relation::Native { reads, rule } => native::residual(&mut out, reads, rule, poses, rank),
            Relation::Separation { a, b, ends, distance: d, parallel } => {
                if let [DistanceEntity::Axis(u), DistanceEntity::Axis(v)] = ends.as_ref() {
                    let (un, up) = axis::<N>(&poses[a], u);
                    let (vn, vp) = axis::<N>(&poses[b], v);
                    if parallel.is_some() || norm(cross(un, vn)).middle() < 1e-7 {
                        let sign = parallel.unwrap_or_else(|| if dot(un, vn).middle() < 0.0 { -1 } else { 1 });
                        aligned(&mut out, un, vn, sign);
                        scalar(&mut out, norm(cross(sub(vp, up), un)) - N::q(d), false);
                    } else {
                        scalar(&mut out, distance::<N>(ends, &poses[a], &poses[b]) - N::q(d), false);
                    }
                } else {
                    scalar(&mut out, distance::<N>(ends, &poses[a], &poses[b]) - N::q(d), false);
                }
            }
            Relation::Angle { a, b, reference, directions, min, max, limit } => {
                let n: V<N> = rotate(&poses[reference], direction(directions[2]));
                let projected = |p: &Placement, d| {
                    let v = rotate(p, direction(d));
                    unit(sub(v, mul(n, dot(v, n))))
                };
                let u = projected(&poses[a], directions[0]);
                let v = projected(&poses[b], directions[1]);
                let sine = dot(n, cross(u, v));
                let cosine = dot(u, v);
                if *limit {
                    range(
                        &mut out,
                        sine.atan2(cosine, ((*min + *max) * 0.5).to_radians()),
                        radians(*min),
                        radians(*max),
                        true,
                        rank,
                    );
                } else {
                    let (s, c) = radians::<N>(*min).sin_cos();
                    scalar(&mut out, sine - s, true);
                    scalar(&mut out, cosine - c, true);
                }
            }
        },
    }
    out
}

fn planes_ready_for_exact(bodies: &BTreeMap<Body, Placement>, cs: &[Constraint]) -> bool {
    let on_axis = |body: &Body, n: [i8; 3]| {
        let v = bodies[body].rotation * DVec3::from_array(n.map(|x| x as f64));
        let candidate = if v.x.abs() > 0.5 && v.y.abs() < 1e-12 && v.z.abs() < 1e-12 {
            DVec3::X * v.x.signum()
        } else if v.y.abs() > 0.5 && v.x.abs() < 1e-12 && v.z.abs() < 1e-12 {
            DVec3::Y * v.y.signum()
        } else if v.z.abs() > 0.5 && v.x.abs() < 1e-12 && v.y.abs() < 1e-12 {
            DVec3::Z * v.z.signum()
        } else {
            return false;
        };
        v.distance(candidate) < 1e-12
    };
    cs.iter().all(|c| match c {
        Constraint::Origin { axes, at, .. } => !*axes || solver::cube_of(at.rotation).is_some(),
        Constraint::Parallel { a, b, plane_a, plane_b, along, .. } => {
            on_axis(a, plane_a.normal)
                && on_axis(b, plane_b.normal)
                && match along {
                    Some(Along::Width { groove, tab, .. }) => on_axis(a, groove.normal) && on_axis(b, tab.normal),
                    _ => true,
                }
        }
        Constraint::Perpendicular { a, b, plane_a, plane_b, .. } => {
            on_axis(a, plane_a.normal) && on_axis(b, plane_b.normal)
        }
        _ => false,
    })
}

pub(super) fn needs_motion(bodies: &BTreeMap<Body, Placement>, cs: &[Constraint]) -> bool {
    cs.iter().any(|c| match c {
        Constraint::Motion { .. } => true,
        Constraint::Lock { relative, .. } => solver::cube_of(relative.rotation).is_none(),
        Constraint::Turn { relative, .. } => solver::cube_of(*relative).is_none(),
        Constraint::Origin { axes: true, at, .. } => solver::cube_of(at.rotation).is_none(),
        _ => false,
    }) || (bodies.values().any(|p| solver::cube_of(p.rotation).is_none()) && !planes_ready_for_exact(bodies, cs))
}
fn groups(bodies: &BTreeMap<Body, Placement>, cs: &[Constraint]) -> Vec<(BTreeMap<Body, Placement>, Vec<Constraint>)> {
    let mut left: BTreeSet<Body> = bodies.keys().cloned().collect();
    let mut out = Vec::new();
    while let Some(seed) = left.pop_first() {
        let mut keys = BTreeSet::from([seed]);
        loop {
            let before = keys.len();
            for c in cs {
                let tied = c.tied();
                if tied.iter().any(|b| keys.contains(*b)) {
                    keys.extend(tied.into_iter().cloned());
                }
            }
            if before == keys.len() {
                break;
            }
        }
        for key in &keys {
            left.remove(key);
        }
        let constraints = cs
            .iter()
            .filter(|c| match c {
                Constraint::Origin { body, .. } => keys.contains(body),
                _ => c.tied().iter().any(|b| keys.contains(*b)),
            })
            .cloned()
            .collect();
        out.push((
            keys.into_iter()
                .map(|b| {
                    let p = bodies[&b];
                    (b, p)
                })
                .collect(),
            constraints,
        ));
    }
    out
}
fn refused(cs: &[Constraint], code: RefusalCode, message: &str) -> MateRefusal {
    let mates: BTreeSet<String> = cs.iter().map(|c| c.mate().to_owned()).collect();
    MateRefusal { mates: mates.into_iter().collect(), ..MateRefusal::new(code, message) }
}
fn values(bodies: &BTreeMap<Body, Placement>, cs: &[Constraint], rank: bool) -> Vec<f64> {
    cs.iter().flat_map(|c| residual::<f64>(c, bodies, rank)).map(|r| r.value).collect()
}
fn perturb(bodies: &BTreeMap<Body, Placement>, body: &Body, k: usize, h: f64) -> BTreeMap<Body, Placement> {
    let mut out = bodies.clone();
    let p = out.get_mut(body).expect("known body");
    if k < 3 {
        p.translation[k] += h;
    } else {
        let mut v = [0.0; 3];
        v[k - 3] = h * 0.5;
        let q = DQuat::from_xyzw(v[0], v[1], v[2], 1.0).normalize();
        p.rotation = (q * p.rotation).normalize();
    }
    out
}
fn jacobian(bodies: &BTreeMap<Body, Placement>, cs: &[Constraint], rank: bool) -> Vec<Vec<f64>> {
    let count = values(bodies, cs, rank).len();
    let mut rows = vec![vec![0.0; bodies.len() * 6]; count];
    for (i, body) in bodies.keys().enumerate() {
        for k in 0..6 {
            let h = if k < 3 { 1e-4 } else { 1e-5 };
            let plus = values(&perturb(bodies, body, k, h), cs, rank);
            let minus = values(&perturb(bodies, body, k, -h), cs, rank);
            for (r, (p, m)) in rows.iter_mut().zip(plus.into_iter().zip(minus)) {
                r[i * 6 + k] = (p - m) / (2.0 * h);
            }
        }
    }
    rows
}
fn linear(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for k in 0..n {
        let pivot = (k..n).max_by(|i, j| a[*i][k].abs().total_cmp(&a[*j][k].abs()))?;
        if !a[pivot][k].is_finite() || a[pivot][k].abs() < 1e-20 {
            return None;
        }
        a.swap(k, pivot);
        b.swap(k, pivot);
        let pivot_tail = a[k][k + 1..].to_vec();
        for i in k + 1..n {
            let factor = a[i][k] / a[k][k];
            for (value, pivot) in a[i][k + 1..].iter_mut().zip(&pivot_tail) {
                *value -= factor * pivot;
            }
            b[i] -= factor * b[k];
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        x[i] = (b[i] - (i + 1..n).map(|j| a[i][j] * x[j]).sum::<f64>()) / a[i][i];
    }
    Some(x)
}
fn advance(bodies: &BTreeMap<Body, Placement>, step: &[f64]) -> BTreeMap<Body, Placement> {
    bodies
        .iter()
        .enumerate()
        .map(|(i, (b, p))| {
            let s = &step[i * 6..i * 6 + 6];
            let mut q = *p;
            q.translation += DVec3::new(s[0], s[1], s[2]);
            let rotation = DQuat::from_xyzw(s[3] * 0.5, s[4] * 0.5, s[5] * 0.5, 1.0).normalize();
            q.rotation = (rotation * q.rotation).normalize();
            (b.clone(), Placement::new(q.rotation, q.translation))
        })
        .collect()
}
fn norm_squared(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum()
}
fn shortest_arc(from: DVec3, to: DVec3) -> DQuat {
    let from = from.normalize();
    let to = to.normalize();
    let dot = from.dot(to);
    if dot < -1.0 + 1e-14 {
        let axis = if from.x.abs() < 0.8 { from.cross(DVec3::X) } else { from.cross(DVec3::Y) }.normalize();
        DQuat::from_xyzw(axis.x, axis.y, axis.z, 0.0)
    } else {
        let axis = from.cross(to);
        DQuat::from_xyzw(axis.x, axis.y, axis.z, 1.0 + dot).normalize()
    }
}
fn seed(bodies: &BTreeMap<Body, Placement>, cs: &[Constraint]) -> BTreeMap<Body, Placement> {
    let mut out = bodies.clone();
    let fixed_rotation: BTreeSet<_> = cs
        .iter()
        .filter_map(|c| match c {
            Constraint::Origin { body, axes: true, .. } => Some(body.clone()),
            _ => None,
        })
        .collect();
    let fixed_position: BTreeSet<_> = cs
        .iter()
        .filter_map(|c| match c {
            Constraint::Origin { body, .. } => Some(body.clone()),
            _ => None,
        })
        .collect();
    for c in cs {
        if let Constraint::Origin { body, axes, at, .. } = c {
            let p = out.get_mut(body).expect("known body");
            p.translation = at.translation;
            if *axes {
                p.rotation = at.rotation;
            }
        }
    }
    // Resolve an antipodal slot axis before the signed-angle seed chooses its spin.
    for c in cs {
        if let Constraint::Slot { a, b, slot, axis, sign, .. } = c
            && !fixed_rotation.contains(b)
        {
            let target = out[a].rotation * DVec3::from_array(slot.depth.map(|x| x as f64)) * (*sign as f64);
            let current = out[b].rotation * DVec3::from_array(axis.direction.map(|x| x as f64));
            let p = out.get_mut(b).expect("known body");
            p.rotation = crate::placement::canonical((shortest_arc(current, target) * p.rotation).normalize());
        }
    }
    for c in cs {
        match c {
            Constraint::Parallel { a, b, plane_a, plane_b, sign, .. } => {
                if !fixed_rotation.contains(b) {
                    let target = out[a].rotation * DVec3::from_array(plane_a.normal.map(|x| x as f64)) * (*sign as f64);
                    let current = out[b].rotation * DVec3::from_array(plane_b.normal.map(|x| x as f64));
                    let p = out.get_mut(b).expect("known body");
                    p.rotation = crate::placement::canonical((shortest_arc(current, target) * p.rotation).normalize());
                }
            }
            Constraint::Axes { a, b, axis_a, axis_b, sign, .. } if *sign != 0 => {
                if !fixed_rotation.contains(b) {
                    let target =
                        out[a].rotation * DVec3::from_array(axis_a.direction.map(|x| x as f64)) * (*sign as f64);
                    let current = out[b].rotation * DVec3::from_array(axis_b.direction.map(|x| x as f64));
                    let p = out.get_mut(b).expect("known body");
                    p.rotation = crate::placement::canonical((shortest_arc(current, target) * p.rotation).normalize());
                }
            }
            Constraint::Motion { relation, .. } => match relation.as_ref() {
                Relation::Angle { a, b, reference, directions, min, max, limit } => {
                    let n = out[reference].rotation * DVec3::from_array(directions[2].map(|x| x as f64));
                    let project = |v: DVec3| (v - n * v.dot(n)).normalize();
                    let u = project(out[a].rotation * DVec3::from_array(directions[0].map(|x| x as f64)));
                    let v = project(out[b].rotation * DVec3::from_array(directions[1].map(|x| x as f64)));
                    let near = if *limit { ((*min + *max) * 0.5).to_radians() } else { 0.0 };
                    let current = <f64 as Number>::atan2(n.dot(u.cross(v)), u.dot(v), near);
                    let target =
                        if *limit { current.clamp(min.to_radians(), max.to_radians()) } else { min.to_radians() };
                    let (body, sign) = if !fixed_rotation.contains(b) {
                        (b, 1.0)
                    } else if !fixed_rotation.contains(a) {
                        (a, -1.0)
                    } else {
                        continue;
                    };
                    let half = sign * (target - current) * 0.5;
                    let s = libm::sin(half);
                    let q = DQuat::from_xyzw(n.x * s, n.y * s, n.z * s, libm::cos(half));
                    if q.is_finite() {
                        let p = out.get_mut(body).expect("known body");
                        p.rotation = crate::placement::canonical((q * p.rotation).normalize());
                    }
                }
                Relation::Separation { a, b, ends, distance, parallel } => {
                    if let Some(sign) = parallel
                        && let [DistanceEntity::Axis(u), DistanceEntity::Axis(v)] = ends.as_ref()
                        && !fixed_rotation.contains(b)
                    {
                        let un = out[a].rotation * DVec3::from_array(u.direction.map(|x| x as f64));
                        let vn = out[b].rotation * DVec3::from_array(v.direction.map(|x| x as f64));
                        let sign = *sign as f64;
                        let p = out.get_mut(b).expect("known body");
                        p.rotation =
                            crate::placement::canonical((shortest_arc(vn, un * sign) * p.rotation).normalize());
                    }
                    let read = |e: &DistanceEntity, p: &Placement| match e {
                        DistanceEntity::Point(v) => (DVec3::from_array(point::<f64>(p, v)), None),
                        DistanceEntity::Axis(v) => {
                            let (n, p) = axis::<f64>(p, v);
                            (DVec3::from_array(p), Some(DVec3::from_array(n)))
                        }
                    };
                    let ((p, u), (q, v)) = (read(&ends[0], &out[a]), read(&ends[1], &out[b]));
                    let mut delta = q - p;
                    match (u, v) {
                        (Some(u), Some(v)) if u.cross(v).length() > 1e-7 => {
                            let n = u.cross(v).normalize();
                            delta = n * delta.dot(n);
                        }
                        (Some(n), _) | (_, Some(n)) => delta -= n * delta.dot(n),
                        _ => {}
                    }
                    let mut direction = delta.try_normalize().unwrap_or_else(|| {
                        u.map_or(DVec3::X, |n| {
                            if n.x.abs() < 0.8 { n.cross(DVec3::X).normalize() } else { n.cross(DVec3::Y).normalize() }
                        })
                    });
                    let correction = solver::nearest(distance) - delta.length();
                    let body = if !fixed_position.contains(b) {
                        b
                    } else if !fixed_position.contains(a) {
                        direction = -direction;
                        a
                    } else {
                        continue;
                    };
                    out.get_mut(body).expect("known body").translation += direction * correction;
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

fn numeric(bodies: &BTreeMap<Body, Placement>, cs: &[Constraint]) -> Result<BTreeMap<Body, Placement>, MateRefusal> {
    if certify(bodies, cs).is_ok() {
        return Ok(bodies.clone());
    }
    let mut poses = seed(bodies, cs);
    let mut origins = BTreeMap::new();
    for c in cs {
        if let Constraint::Origin { body, axes, .. } = c {
            *origins.entry(body).or_insert(false) |= axes;
        }
    }
    let movable: Vec<bool> =
        bodies.keys().flat_map(|body| (0..6).map(|k| origins.get(body).is_none_or(|axes| k >= 3 && !axes))).collect();
    let mut damping = 1e-6;
    for _ in 0..ITERATIONS {
        let r = values(&poses, cs, false);
        if r.iter().any(|x| !x.is_finite()) {
            return Err(refused(
                cs,
                RefusalCode::Uncertified,
                "a reference direction is singular or a residual cannot be enclosed",
            ));
        }
        if r.iter().all(|x| x.abs() < 1e-12) && certify(&poses, cs).is_ok() {
            return Ok(poses);
        }
        let mut j = jacobian(&poses, cs, false);
        // Origin coordinates are fixed data, rather than soft equations in the step.
        for row in &mut j {
            for (value, movable) in row.iter_mut().zip(&movable) {
                if !movable {
                    *value = 0.0;
                }
            }
        }
        let n = poses.len() * 6;
        let mut a = vec![vec![0.0; n]; n];
        let mut b = vec![0.0; n];
        for (row, r) in j.iter().zip(&r) {
            for i in 0..n {
                b[i] -= row[i] * r;
                for k in 0..=i {
                    a[i][k] += row[i] * row[k];
                }
            }
        }
        for i in 0..n {
            let (before, after) = a.split_at_mut(i);
            let row = &mut after[0];
            for (k, previous) in before.iter_mut().enumerate() {
                previous[i] = row[k];
            }
            row[i] += damping;
        }
        let Some(mut step) = linear(a, b) else { break };
        let largest = step.as_chunks::<6>().0.iter().map(|s| DVec3::new(s[3], s[4], s[5]).length()).fold(0.0, f64::max);
        if largest > 0.5 {
            for s in &mut step {
                *s *= 0.5 / largest;
            }
        }
        let next = advance(&poses, &step);
        let next_r = values(&next, cs, false);
        if norm_squared(&next_r) < norm_squared(&r) {
            poses = next;
            damping = (damping * 0.2).max(1e-14);
        } else {
            damping *= 10.0;
        }
        if damping > 1e12 {
            break;
        }
    }
    certify(&poses, cs).map_err(|r| refused(cs, r.code, &r.message))?;
    Ok(poses)
}
fn certify(poses: &BTreeMap<Body, Placement>, cs: &[Constraint]) -> Result<Vec<BoundedProof>, MateRefusal> {
    let mut proofs: BTreeMap<String, BoundedProof> = BTreeMap::new();
    for c in cs {
        let p = proofs.entry(c.mate().into()).or_insert_with(|| BoundedProof {
            mate: c.mate().into(),
            certainty: "bounded",
            residual_mm: 0.0,
            residual_angle: 0.0,
            bound_mm: MM,
            bound_angle: RAD,
        });
        for r in residual::<Interval>(c, poses, false) {
            let bound = if r.angular { RAD } else { MM };
            if !r.value.finite() || r.value.abs_max() > bound {
                return Err(refused(
                    std::slice::from_ref(c),
                    RefusalCode::Uncertified,
                    "the stored placement could not be certified within 1e-9 mm and 1e-9 angular residual; the accepted assembly is unchanged",
                ));
            }
            let squared = (r.value.abs_max() * r.value.abs_max()).next_up();
            if r.angular {
                p.residual_angle = (p.residual_angle + squared).next_up();
            } else {
                p.residual_mm = (p.residual_mm + squared).next_up();
            }
        }
    }
    for p in proofs.values_mut() {
        p.residual_mm = p.residual_mm.sqrt().next_up();
        p.residual_angle = (2.0 * p.residual_angle.sqrt().next_up()).next_up();
        if p.residual_mm > MM || p.residual_angle > RAD {
            return Err(MateRefusal {
                mates: vec![p.mate.clone()],
                ..MateRefusal::new(RefusalCode::Uncertified, "aggregate residual exceeds 1e-9 mm or 1e-9 radians")
            });
        }
    }
    Ok(proofs.into_values().collect())
}

pub(super) fn solve(
    bodies: &BTreeMap<Body, Placement>,
    cs: &[Constraint],
) -> Result<BTreeMap<Body, Placement>, MateRefusal> {
    if bodies.len() > solver::MAX_BODIES {
        return Err(refused(cs, RefusalCode::TooLarge, "at most 128 mated bodies are supported"));
    }
    // Calling the old path unchanged matters for canonical bytes and exact refusals.
    if !needs_motion(bodies, cs) {
        return solver::solve_exact(bodies, cs);
    }
    let mut out = BTreeMap::new();
    for (b, c) in groups(bodies, cs) {
        let placed = if needs_motion(&b, &c) { numeric(&b, &c)? } else { solver::solve_exact(&b, &c)? };
        out.extend(if needs_motion(&placed, &c) { placed } else { solver::solve_exact(&placed, &c)? });
    }
    Ok(out)
}
pub(super) fn certificates(
    bodies: &BTreeMap<Body, Placement>,
    cs: &[Constraint],
) -> Result<Vec<BoundedProof>, MateRefusal> {
    let mut out = Vec::new();
    for (b, c) in groups(bodies, cs) {
        if needs_motion(&b, &c) {
            out.extend(certify(&b, &c)?);
        }
    }
    Ok(out)
}
fn rank(mut rows: Vec<Vec<f64>>, threshold: f64) -> usize {
    if rows.is_empty() {
        return 0;
    }
    for r in &mut rows {
        let scale = r.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
        if scale > 1e-7 {
            for x in r {
                *x /= scale;
            }
        } else {
            r.fill(0.0);
        }
    }
    let mut rank = 0;
    for col in 0..rows[0].len() {
        let Some(pivot) = (rank..rows.len()).max_by(|i, j| rows[*i][col].abs().total_cmp(&rows[*j][col].abs())) else {
            break;
        };
        if rows[pivot][col].abs() < threshold {
            continue;
        }
        rows.swap(rank, pivot);
        for i in rank + 1..rows.len() {
            let f = rows[i][col] / rows[rank][col];
            for k in col..rows[i].len() {
                rows[i][k] -= f * rows[rank][k];
            }
        }
        rank += 1;
    }
    rank
}
pub(super) fn freedoms(
    bodies: &BTreeMap<Body, Placement>,
    cs: &[Constraint],
) -> Result<(usize, Vec<FreeMotion>), MateRefusal> {
    if bodies.len() > solver::MAX_BODIES {
        return Err(refused(cs, RefusalCode::TooLarge, "at most 128 mated bodies are supported"));
    }
    if !needs_motion(bodies, cs) {
        return solver::freedoms_exact(bodies, cs);
    }
    let mut count = 0;
    let mut free = Vec::new();
    for (b, c) in groups(bodies, cs) {
        if !needs_motion(&b, &c) {
            let (d, f) = solver::freedoms_exact(&b, &c)?;
            count += d;
            free.extend(f);
            continue;
        }
        let j = jacobian(&b, &c, true);
        if j.iter().flatten().any(|v| !v.is_finite()) {
            return Err(refused(&c, RefusalCode::UncertainRank, "the constraint Jacobian is singular or nonfinite"));
        }
        let low = rank(j.clone(), 1e-6);
        let high = rank(j.clone(), 1e-9);
        if low != high {
            return Err(refused(
                &c,
                RefusalCode::UncertainRank,
                "the constraint rank is uncertain near a singular pose",
            ));
        }
        count += b.len() * 6 - low;
        for (i, body) in b.keys().enumerate() {
            for (k, name) in
                ["translate_x", "translate_y", "translate_z", "spin_x", "spin_y", "spin_z"].into_iter().enumerate()
            {
                if j.iter().all(|row| row[i * 6 + k].abs() < 1e-7) {
                    free.push(FreeMotion::of(body, name));
                }
            }
        }
    }
    Ok((count, free))
}

pub(super) fn rational_direction(rotation: DQuat, normal: [i8; 3]) -> [num_rational::BigRational; 3] {
    use num_rational::BigRational;
    let [x, y, z, w] = rotation.to_array().map(|v| solver::exact(v).expect("finite placement"));
    let one = BigRational::from_integer(1.into());
    let two = BigRational::from_integer(2.into());
    let norm = &x * &x + &y * &y + &z * &z + &w * &w;
    let m = [
        [
            one.clone() - &two * (&y * &y + &z * &z) / &norm,
            &two * (&x * &y - &z * &w) / &norm,
            &two * (&x * &z + &y * &w) / &norm,
        ],
        [
            &two * (&x * &y + &z * &w) / &norm,
            one.clone() - &two * (&x * &x + &z * &z) / &norm,
            &two * (&y * &z - &x * &w) / &norm,
        ],
        [
            &two * (&x * &z - &y * &w) / &norm,
            &two * (&y * &z + &x * &w) / &norm,
            one - &two * (&x * &x + &y * &y) / &norm,
        ],
    ];
    m.map(|row| row.into_iter().zip(normal).map(|(x, n)| x * BigRational::from_integer(n.into())).sum())
}

pub(super) fn measure_gap(
    a: &LocalPlane,
    b: &LocalPlane,
    poses: &[Placement; 2],
) -> Result<(f64, f64, f64), MateRefusal> {
    let (n, o) = plane::<Interval>(&poses[0], a);
    let (m, _) = plane::<Interval>(&poses[1], b);
    if cross(n, m).into_iter().any(|x| !x.finite() || x.abs_max() > RAD / 4.0) {
        return Err(MateRefusal::new(
            RefusalCode::NotParallel,
            "supporting planes are not parallel within 1e-9 radians",
        ));
    }
    let local_point = b.normal.map(|k| b.offset.clone() * num_rational::BigRational::from_integer(k.into()));
    let gap = dot(n, point::<Interval>(&poses[1], &local_point)) - o;
    if !gap.finite() {
        return Err(MateRefusal::new(RefusalCode::Uncertified, "the plane distance has no finite enclosure"));
    }
    Ok((gap.middle(), gap.lo, gap.hi))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::{ComponentId, Step};
    fn body(i: u32) -> Body {
        crate::path::OccurrencePath(vec![Step::component(ComponentId(i))])
    }
    fn q(v: i64) -> num_rational::BigRational {
        num_rational::BigRational::from_integer(v.into())
    }
    fn p() -> LocalPoint {
        [q(0), q(0), q(0)]
    }
    fn poses() -> BTreeMap<Body, Placement> {
        BTreeMap::from([(body(1), Placement::IDENTITY), (body(2), Placement::IDENTITY)])
    }
    fn origin() -> Constraint {
        Constraint::Origin { mate: "O".into(), body: body(1), axes: true, at: Placement::IDENTITY }
    }
    fn angle(degrees: f64) -> Constraint {
        Constraint::Motion {
            mate: "Angle".into(),
            relation: Box::new(Relation::Angle {
                a: body(1),
                b: body(2),
                reference: body(1),
                directions: [[1, 0, 0], [1, 0, 0], [0, 0, 1]],
                min: degrees,
                max: degrees,
                limit: false,
            }),
        }
    }
    #[test]
    fn signed_angle_and_antipodal_slot_axis_solve_together() {
        let mut at = poses();
        at.get_mut(&body(2)).unwrap().translation = DVec3::new(40.0, 7.0, 30.0);
        let cs = [
            origin(),
            angle(30.0),
            Constraint::Slot {
                mate: "SL".into(),
                a: body(1),
                b: body(2),
                slot: solver::LocalSlot { depth: [0, 0, -1], ends: [[q(10), q(0), q(0)], [q(30), q(0), q(0)]] },
                axis: LocalAxis { direction: [0, 0, 1], through: p() },
                sign: 1,
                mode: SlotMode::Free,
            },
        ];
        let out = solve(&at, &cs).unwrap();
        certificates(&out, &cs).unwrap();
        assert!((out[&body(2)].rotation * DVec3::Z + DVec3::Z).length() < 1e-10);
        let mut locked = cs.to_vec();
        locked.push(Constraint::Turn { mate: "SL".into(), a: body(1), b: body(2), relative: out[&body(2)].rotation });
        assert_eq!(solve(&out, &locked).unwrap(), out);
        solve(&at, &locked).unwrap();
    }

    #[test]
    fn signed_angles_have_labelled_outward_bounds() {
        for degrees in [-180.0, -120.0, -30.0, 0.0, 30.0, 90.0, 120.0, 180.0] {
            let cs = [origin(), angle(degrees)];
            let out = solve(&poses(), &cs).unwrap();
            let n = out[&body(2)].rotation * DVec3::X;
            assert!(
                (libm::atan2(n.y, n.x).to_degrees() - degrees).abs() < 1e-7
                    || (degrees.abs() == 180.0 && (libm::atan2(n.y, n.x).to_degrees().abs() - 180.0).abs() < 1e-7),
                "{degrees}: {n:?}"
            );
            let proof = certificates(&out, &cs).unwrap();
            assert_eq!(proof[0].certainty, "bounded");
            assert!(proof.iter().all(|p| p.residual_mm <= p.bound_mm && p.residual_angle <= p.bound_angle));
            assert_eq!(freedoms(&out, &cs).unwrap().0, 5);
            assert_eq!(out, solve(&out, &cs).unwrap());
        }
    }
    #[test]
    fn limits_snap_to_the_nearest_stop_and_leave_the_turn_free() {
        for (start, min, max, target) in [
            (-90.0_f64, -30.0, 45.0, -30.0_f64),
            (90.0, -30.0, 45.0, 45.0),
            (10.0, -30.0, 45.0, 10.0),
            (180.0, 90.0, 180.0, 180.0),
            (-180.0, -180.0, -90.0, -180.0),
            (170.0, -180.0, -90.0, -180.0),
            (-170.0, 90.0, 180.0, 180.0),
        ] {
            let mut at = poses();
            at.get_mut(&body(2)).unwrap().rotation = DQuat::from_rotation_z(start.to_radians());
            let c = Constraint::Motion {
                mate: "Limit".into(),
                relation: Box::new(Relation::Angle {
                    a: body(1),
                    b: body(2),
                    reference: body(1),
                    directions: [[1, 0, 0], [1, 0, 0], [0, 0, 1]],
                    min,
                    max,
                    limit: true,
                }),
            };
            let cs = [origin(), c];
            let out = solve(&at, &cs).unwrap();
            let n = out[&body(2)].rotation * DVec3::X;
            assert!((libm::atan2(n.y, n.x).to_degrees().abs() - target.abs()).abs() < 1e-7);
            assert_eq!(freedoms(&out, &cs).unwrap().0, 6);
        }
    }
    #[test]
    fn a_reference_parallel_to_a_direction_is_refused_by_name() {
        let c = Constraint::Motion {
            mate: "bad reference".into(),
            relation: Box::new(Relation::Angle {
                a: body(1),
                b: body(2),
                reference: body(1),
                directions: [[0, 0, 1], [1, 0, 0], [0, 0, 1]],
                min: -30.0,
                max: 30.0,
                limit: true,
            }),
        };
        let error = solve(&poses(), &[origin(), c]).unwrap_err();
        assert_eq!(error.code, RefusalCode::Uncertified);
        assert!(error.mates.contains(&"bad reference".into()));
    }
    #[test]
    fn point_distance_is_euclidean_and_certified() {
        let mut at = poses();
        at.get_mut(&body(2)).unwrap().translation = DVec3::new(3.0, 4.0, 0.0);
        let cs = [
            origin(),
            Constraint::Motion {
                mate: "D".into(),
                relation: Box::new(Relation::Separation {
                    a: body(1),
                    b: body(2),
                    ends: Box::new([DistanceEntity::Point(p()), DistanceEntity::Point(p())]),
                    distance: q(10),
                    parallel: None,
                }),
            },
        ];
        let out = solve(&at, &cs).unwrap();
        assert!((out[&body(2)].translation.length() - 10.0).abs() < MM);
        assert_eq!(freedoms(&out, &cs).unwrap().0, 5);
    }
    #[test]
    fn independent_exact_group_keeps_the_old_answer() {
        let mut at = poses();
        at.insert(body(3), Placement::new(DQuat::IDENTITY, DVec3::new(0.0, 0.0, 17.0)));
        let exact = Constraint::Origin { mate: "exact".into(), body: body(3), axes: true, at: at[&body(3)] };
        let cs = [origin(), angle(30.0), exact.clone()];
        let out = solve(&at, &cs).unwrap();
        let baseline = solver::solve_exact(&BTreeMap::from([(body(3), at[&body(3)])]), &[exact]).unwrap();
        assert_eq!(out[&body(3)], baseline[&body(3)]);
        assert!(certificates(&out, &cs).unwrap().iter().all(|p| p.mate != "exact"));
    }
    #[test]
    fn internal_tangent_radius_subtraction_keeps_its_exact_target() {
        let large = q(1_000_000_000_000);
        let radius = &large + q(1) / q(3);
        let rounded_gap = solver::nearest(&radius) - solver::nearest(&large);
        let distance = radius - large;
        let c = Constraint::Motion {
            mate: "T".into(),
            relation: Box::new(Relation::Separation {
                a: body(1),
                b: body(2),
                ends: Box::new([DistanceEntity::Point(p()), DistanceEntity::Point(p())]),
                distance: distance.clone(),
                parallel: None,
            }),
        };
        let cs = [origin(), c];
        let mut at = poses();
        at.get_mut(&body(2)).unwrap().translation = DVec3::X * rounded_gap;
        assert_eq!(certificates(&at, &cs).unwrap_err().code, RefusalCode::Uncertified);
        at.get_mut(&body(2)).unwrap().translation = DVec3::X * solver::nearest(&distance);
        certificates(&at, &cs).unwrap();
    }
    #[test]
    fn certificate_rejects_a_rounded_claim() {
        let cs = [origin(), angle(30.0)];
        let mut out = solve(&poses(), &cs).unwrap();
        out.get_mut(&body(2)).unwrap().rotation = DQuat::from_rotation_z(30.0001_f64.to_radians());
        assert_eq!(certificates(&out, &cs).unwrap_err().code, RefusalCode::Uncertified);
    }
}

#[cfg(test)]
mod family_tests {
    use super::*;
    use crate::path::{ComponentId, OccurrencePath, Step};
    use num_rational::BigRational;
    fn body(i: u32) -> Body {
        OccurrencePath(vec![Step::component(ComponentId(i))])
    }
    fn q(i: i64) -> BigRational {
        BigRational::from_integer(i.into())
    }
    fn p(x: i64, y: i64, z: i64) -> LocalPoint {
        [q(x), q(y), q(z)]
    }
    fn plane(n: [i8; 3], o: i64) -> LocalPlane {
        LocalPlane { normal: n, offset: q(o) }
    }
    fn axis(n: [i8; 3]) -> LocalAxis {
        LocalAxis { direction: n, through: p(0, 0, 0) }
    }
    fn parallel(name: &str, along: Option<Along>) -> Constraint {
        Constraint::Parallel {
            mate: name.into(),
            a: body(1),
            b: body(2),
            plane_a: plane([0, 0, 1], 0),
            plane_b: plane([0, 0, 1], 0),
            sign: 1,
            along,
        }
    }
    fn check(name: &str, mut cs: Vec<Constraint>) {
        let frame = Placement::new(DQuat::from_xyzw(1.0, 2.0, 3.0, 4.0).normalize(), DVec3::new(13.0, -8.0, 5.0));
        let at = BTreeMap::from([
            (body(1), frame),
            (body(2), frame.compose(&Placement::new(DQuat::IDENTITY, DVec3::new(3.0, 4.0, 9.0)))),
            (body(3), frame.compose(&Placement::new(DQuat::IDENTITY, DVec3::new(-10.0, 0.0, 0.0)))),
        ]);
        cs.insert(0, Constraint::Origin { mate: "O".into(), body: body(1), axes: true, at: frame });
        let out = solve(&at, &cs).unwrap_or_else(|e| panic!("{name}: {e}"));
        let proof = certificates(&out, &cs).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(proof.iter().any(|p| p.mate == name), "{name}: {proof:?}");
        freedoms(&out, &cs).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(out, solve(&out, &cs).unwrap(), "{name}: warm start changed a certified pose");
    }
    #[test]
    fn all_existing_families_solve_in_a_rotated_frame() {
        check("distance", vec![parallel("distance", Some(Along::Distance(5.0)))]);
        check("coincident", vec![parallel("coincident", Some(Along::Distance(0.0)))]);
        check("parallel", vec![parallel("parallel", None)]);
        check(
            "perpendicular",
            vec![Constraint::Perpendicular {
                mate: "perpendicular".into(),
                a: body(1),
                b: body(2),
                plane_a: plane([0, 0, 1], 0),
                plane_b: plane([1, 0, 0], 0),
            }],
        );
        for mode in [
            WidthMode::Free,
            WidthMode::Centered,
            WidthMode::Gap { distance: 2.0, flipped: false },
            WidthMode::Share { fraction: q(1) / q(4), flipped: true },
        ] {
            check(
                "width",
                vec![parallel(
                    "width",
                    Some(Along::Width { groove: plane([0, 0, -1], -20), tab: plane([0, 0, -1], -4), mode }),
                )],
            );
        }
        check(
            "lock",
            vec![Constraint::Lock {
                mate: "lock".into(),
                a: body(1),
                b: body(2),
                relative: Placement::new(DQuat::IDENTITY, DVec3::new(0.0, 2.0, 0.0)),
            }],
        );
        let concentric = |name: &str| Constraint::Axes {
            mate: name.into(),
            a: body(1),
            b: body(2),
            axis_a: axis([0, 0, 1]),
            axis_b: axis([0, 0, 1]),
            sign: 1,
            coaxial: true,
        };
        check("concentric", vec![concentric("concentric")]);
        check(
            "locked concentric",
            vec![
                concentric("locked concentric"),
                Constraint::Turn {
                    mate: "locked concentric".into(),
                    a: body(1),
                    b: body(2),
                    relative: DQuat::IDENTITY,
                },
            ],
        );
        for direction in [None, Some([1, 0, 0])] {
            check(
                "tangent",
                vec![Constraint::FromPlane {
                    mate: "tangent".into(),
                    a: body(1),
                    b: body(2),
                    plane: plane([0, 0, 1], 0),
                    point: p(0, 0, 0),
                    direction,
                    gap: q(5),
                    upto: None,
                    says: "tangent".into(),
                }],
            );
        }
        check("hinge", vec![concentric("hinge"), parallel("hinge", Some(Along::Distance(0.0)))]);
        check(
            "point coincident",
            vec![Constraint::Points {
                mate: "point coincident".into(),
                a: body(1),
                b: body(2),
                point_a: p(0, 0, 0),
                point_b: p(0, 0, 0),
            }],
        );
        check(
            "point on axis",
            vec![Constraint::OnAxis {
                mate: "point on axis".into(),
                a: body(1),
                axis: axis([0, 0, 1]),
                b: body(2),
                point: p(0, 0, 0),
            }],
        );
        for ends in [
            Mirrored::Points(p(0, 0, 0), p(0, 0, 0)),
            Mirrored::Axes(axis([0, 0, 1]), axis([0, 0, 1])),
            Mirrored::Planes(plane([0, 0, 1], 0), plane([0, 0, 1], 0)),
        ] {
            check(
                "symmetric",
                vec![Constraint::Symmetric {
                    mate: "symmetric".into(),
                    a: body(1),
                    b: body(2),
                    s: body(3),
                    plane: plane([1, 0, 0], 0),
                    ends,
                    sign: 1,
                }],
            );
        }
        let slide = |i| Slide { body: body(i), direction: [1, 0, 0], offset: q(0), from: None, frame: p(13, -8, 5) };
        check(
            "coupler",
            vec![Constraint::Coupler {
                mate: "coupler".into(),
                a: slide(1),
                b: slide(2),
                ratio: [q(1), q(2)],
                reverse: false,
                offset: q(6),
            }],
        );
        check("limit", vec![parallel("limit", Some(Along::Range(2.0, 5.0)))]);
        check(
            "profile",
            vec![
                parallel("profile", Some(Along::Distance(5.0))),
                Constraint::OnAxis {
                    mate: "profile".into(),
                    a: body(1),
                    axis: axis([0, 0, 1]),
                    b: body(2),
                    point: p(0, 0, 0),
                },
                Constraint::Spin {
                    mate: "profile".into(),
                    a: body(1),
                    b: body(2),
                    normal: [0, 0, 1],
                    edges: Some((vec![DVec3::X, DVec3::Y], vec![DVec3::X, DVec3::Y])),
                },
            ],
        );
        for mode in [
            SlotMode::Free,
            SlotMode::Centre,
            SlotMode::Distance { distance: q(4), flipped: false },
            SlotMode::Share { fraction: q(1) / q(4), flipped: true },
        ] {
            check(
                "slot",
                vec![Constraint::Slot {
                    mate: "slot".into(),
                    a: body(1),
                    slot: solver::LocalSlot { depth: [0, 0, 1], ends: [p(-10, 0, 0), p(10, 0, 0)] },
                    b: body(2),
                    axis: axis([0, 0, 1]),
                    sign: 1,
                    mode,
                }],
            );
        }
    }
}
