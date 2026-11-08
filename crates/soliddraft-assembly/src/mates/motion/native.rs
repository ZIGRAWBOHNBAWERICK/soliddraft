//! Residuals for off-axis or algebraic analytic definitions read from the kernel.
use super::super::bodies::Rule;
use super::super::command::{Local, Read};
use super::*;
use soliddraft_kernel_api::EnclosedSupport;

#[derive(Clone, Copy)]
enum World<N: Number> {
    Plane { normal: V<N>, offset: N },
    Axis { direction: V<N>, through: V<N>, radius: Option<(N, bool)> },
    Point { at: V<N>, radius: Option<(N, bool)> },
}
impl<N: Number> World<N> {
    fn direction(self) -> V<N> {
        match self {
            Self::Plane { normal, .. } => normal,
            Self::Axis { direction, .. } => direction,
            _ => [N::f(f64::NAN); 3],
        }
    }
    fn point(self) -> V<N> {
        match self {
            Self::Plane { normal, offset } => mul(normal, offset),
            Self::Axis { through, .. } => through,
            Self::Point { at, .. } => at,
        }
    }
    fn radius(self) -> Option<(N, bool)> {
        match self {
            Self::Axis { radius, .. } | Self::Point { radius, .. } => radius,
            _ => None,
        }
    }
}
fn world<N: Number>(read: &Read, poses: &BTreeMap<Body, Placement>) -> World<N> {
    let p = &poses[&read.body];
    let radius = read.radius.as_ref().and_then(|r| {
        r.outward.map(|o| {
            let value = match &r.value {
                Ok(r) => N::q(r),
                Err(s) => N::q(s).sqrt(),
            };
            (value, o)
        })
    });
    match &read.local {
        Local::Plane(v) => {
            let (normal, offset) = plane(p, v);
            World::Plane { normal, offset }
        }
        Local::Axis(v) => {
            let (direction, through) = axis(p, v);
            World::Axis { direction, through, radius }
        }
        Local::Point(v) => World::Point { at: point(p, v), radius },
        Local::Native(s) => {
            let radius = |square: &Option<soliddraft_kernel_api::SupportBounds>, outward: &Option<bool>| {
                square.as_ref().zip(*outward).map(|(s, o)| (N::bounds(s).sqrt(), o))
            };
            match s.as_ref() {
                EnclosedSupport::Plane { normal, through } => {
                    let normal = rotate(p, unit(normal.each_ref().map(N::bounds)));
                    let through = add(rotate(p, through.each_ref().map(N::bounds)), p.translation.to_array().map(N::f));
                    World::Plane { normal, offset: dot(normal, through) }
                }
                EnclosedSupport::Axis { direction, through, radius_squared, outward } => {
                    let direction = rotate(p, unit(direction.each_ref().map(N::bounds)));
                    let through = add(rotate(p, through.each_ref().map(N::bounds)), p.translation.to_array().map(N::f));
                    World::Axis { direction, through, radius: radius(radius_squared, outward) }
                }
                EnclosedSupport::Point { at, radius_squared, outward } => {
                    let at = add(rotate(p, at.each_ref().map(N::bounds)), p.translation.to_array().map(N::f));
                    World::Point { at, radius: radius(radius_squared, outward) }
                }
            }
        }
        _ => World::Point { at: [N::f(f64::NAN); 3], radius: None },
    }
}

pub(in crate::mates) fn projection(read: &Read, at: Placement, from: DVec3) -> Option<f64> {
    let poses = BTreeMap::from([(read.body.clone(), at)]);
    let support = world::<f64>(read, &poses);
    if matches!(support, World::Point { .. }) {
        return None;
    }
    Some(dot(support.direction(), sub(support.point(), from.to_array())))
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plane,
    Axis,
    Point,
    Other,
}
fn kind(r: &Read) -> Kind {
    match &r.local {
        Local::Plane(_) => Kind::Plane,
        Local::Axis(_) => Kind::Axis,
        Local::Point(_) => Kind::Point,
        Local::Native(s) => match s.as_ref() {
            EnclosedSupport::Plane { .. } => Kind::Plane,
            EnclosedSupport::Axis { .. } => Kind::Axis,
            EnclosedSupport::Point { .. } => Kind::Point,
        },
        _ => Kind::Other,
    }
}
fn curved(r: &Read) -> bool {
    match &r.local {
        Local::Native(s) => matches!(
            s.as_ref(),
            EnclosedSupport::Axis { radius_squared: Some(_), outward: Some(_), .. }
                | EnclosedSupport::Point { radius_squared: Some(_), outward: Some(_), .. }
        ),
        _ => r.radius.as_ref().is_some_and(|r| r.outward.is_some()),
    }
}
pub(in crate::mates) fn native_constraint(
    mate: String,
    reads: Vec<Read>,
    rule: &Rule,
) -> Result<Vec<Constraint>, MateRefusal> {
    let kinds: Vec<_> = reads.iter().map(kind).collect();
    let directed = |k: Kind| matches!(k, Kind::Plane | Kind::Axis);
    if let Rule::Parallel { distance: Some(d), .. } = rule
        && kinds.iter().all(|k| *k != Kind::Plane)
        && *d < 0.0
    {
        return Err(MateRefusal {
            mates: vec![mate.clone()],
            ..MateRefusal::new(RefusalCode::InvalidValue, "point-point and axis-axis distances are nonnegative")
        });
    }
    if let Rule::Tangent { sign } = rule
        && reads.iter().all(curved)
    {
        let poses = reads.iter().map(|r| (r.body.clone(), Placement::IDENTITY)).collect();
        if let (Some((ra, oa)), Some((rb, ob))) =
            (world::<Interval>(&reads[0], &poses).radius(), world::<Interval>(&reads[1], &poses).radius())
        {
            let difference = ra - rb;
            if (oa == ob) != (*sign < 0) && difference.lo <= 0.0 && difference.hi >= 0.0 {
                return Err(MateRefusal {
                    mates: vec![mate.clone()],
                    ..MateRefusal::new(
                        RefusalCode::UnsupportedReference,
                        "internal Tangent requires radii proved different; use Concentric for equal radii",
                    )
                });
            }
        }
    }
    let valid = match rule {
        Rule::Mechanical { mechanism, .. } => match mechanism {
            super::super::Mechanism::Gear => kinds == [Kind::Axis; 4],
            super::super::Mechanism::Screw => kinds == [Kind::Axis; 2],
            super::super::Mechanism::RackAndPinion => {
                kinds.len() == 3 && kinds[..2] == [Kind::Axis; 2] && directed(kinds[2])
            }
        },
        Rule::Angle { min, max, .. } => {
            if !min.is_finite() || !max.is_finite() || *min < -180.0 || *max > 180.0 || min > max {
                return Err(MateRefusal {
                    mates: vec![mate.clone()],
                    ..MateRefusal::new(
                        RefusalCode::InvalidValue,
                        "signed angular limits must be ordered within [-180,180]",
                    )
                });
            }
            kinds.len() == 3 && kinds.iter().all(|k| directed(*k))
        }
        Rule::Parallel { distance, coincident, .. } => match kinds.as_slice() {
            [Kind::Plane, Kind::Plane] | [Kind::Axis, Kind::Axis] => true,
            [Kind::Plane, Kind::Axis | Kind::Point] | [Kind::Axis | Kind::Point, Kind::Plane] => distance.is_some(),
            [Kind::Point, Kind::Point] => distance.is_some(),
            [Kind::Axis, Kind::Point] | [Kind::Point, Kind::Axis] => *coincident,
            _ => false,
        },
        Rule::Perpendicular => matches!(kinds.as_slice(), [Kind::Plane, Kind::Plane] | [Kind::Axis, Kind::Axis]),
        Rule::Concentric { .. } => kinds.len() == 2 && kinds.iter().all(|k| matches!(k, Kind::Axis | Kind::Point)),
        Rule::Tangent { .. } => {
            kinds.len() == 2
                && ((kinds[0] == Kind::Plane && curved(&reads[1])) || (curved(&reads[0]) && curved(&reads[1])))
        }
        Rule::Hinge { limits, .. } => {
            if let Some([lo, hi]) = limits
                && (!lo.is_finite() || !hi.is_finite() || *lo < -180.0 || *hi > 180.0 || lo > hi)
            {
                return Err(MateRefusal::new(
                    RefusalCode::InvalidValue,
                    "hinge stops must be ordered within [-180,180]",
                ));
            }
            kinds == [Kind::Axis, Kind::Axis, Kind::Plane, Kind::Plane]
        }
        Rule::Symmetric { .. } => {
            kinds.len() == 3 && kinds[2] == Kind::Plane && kinds[0] == kinds[1] && kinds[0] != Kind::Other
        }
        Rule::Coupler { .. } => kinds.len() == 2 && kinds.iter().all(|k| directed(*k)),
        Rule::Limit { .. } => matches!(
            kinds.as_slice(),
            [Kind::Plane, Kind::Plane | Kind::Axis | Kind::Point] | [Kind::Axis | Kind::Point, Kind::Plane]
        ),
        Rule::Width { mode, .. } => {
            if !matches!(mode, WidthMode::Centered) {
                return Err(MateRefusal {
                    mates: vec![mate.clone()],
                    ..MateRefusal::new(
                        RefusalCode::UnsupportedReference,
                        "Width between nonparallel faces supports Centered for now",
                    )
                });
            }
            kinds == [Kind::Plane; 4]
                && reads[0].body == reads[1].body
                && reads[2].body == reads[3].body
                && reads[0].body != reads[2].body
        }
        Rule::Profile { .. } | Rule::Slot { .. } => false,
    };
    if !valid {
        return Err(MateRefusal {
            mates: vec![mate],
            ..MateRefusal::new(
                RefusalCode::UnsupportedReference,
                "this mate does not accept the selected native support kinds",
            )
        });
    }
    Ok(vec![Constraint::Motion { mate, relation: Box::new(Relation::Native { reads, rule: Box::new(rule.clone()) }) }])
}
fn clock<N: Number>(read: &Read, poses: &BTreeMap<Body, Placement>) -> V<N> {
    let local = world::<N>(read, &BTreeMap::from([(read.body.clone(), Placement::IDENTITY)])).direction();
    let index = (0..3).min_by(|i, j| local[*i].middle().abs().total_cmp(&local[*j].middle().abs())).unwrap();
    let basis = std::array::from_fn(|k| N::f(if k == index { 1.0 } else { 0.0 }));
    rotate(&poses[&read.body], unit(cross(local, basis)))
}
fn joint_angle<N: Number>(reads: &[Read], poses: &BTreeMap<Body, Placement>, center: f64) -> N {
    let (u, v) = (clock(&reads[0], poses), clock(&reads[1], poses));
    let n = world::<N>(&reads[0], poses).direction();
    dot(n, cross(u, v)).atan2(dot(u, v), center)
}
fn mechanical<N: Number>(reads: &[Read], rule: &Rule, poses: &BTreeMap<Body, Placement>) -> (N, [N; 2]) {
    let Rule::Mechanical { mechanism, factor, reverse, angles, reference, frame, .. } = rule else { unreachable!() };
    let a = joint_angle::<N>(reads, poses, angles[0]);
    let sign = N::f(if *reverse { -1.0 } else { 1.0 });
    match mechanism {
        super::super::Mechanism::Gear => {
            let b = joint_angle::<N>(&reads[2..], poses, angles[1]);
            (N::f(factor[0]) * b + sign * N::f(factor[1]) * a, [a, b])
        }
        super::super::Mechanism::Screw => {
            let (a_axis, b_axis) = (world::<N>(&reads[0], poses), world::<N>(&reads[1], poses));
            let travel = dot(a_axis.direction(), sub(b_axis.point(), a_axis.point()));
            (travel - sign * N::f(factor[0]) * a / radians::<N>(360.0), [a, N::f(0.0)])
        }
        super::super::Mechanism::RackAndPinion => {
            let rack = world::<N>(&reads[2], poses);
            let from = reference
                .as_ref()
                .map_or_else(|| frame.each_ref().map(N::q), |body| poses[body].translation.to_array().map(N::f));
            let travel = dot(rack.direction(), sub(rack.point(), from));
            (travel - sign * N::f(factor[0]) * a / radians::<N>(360.0), [a, N::f(0.0)])
        }
    }
}
pub(in crate::mates) fn mechanical_value(
    reads: &[Read],
    rule: &Rule,
    poses: &BTreeMap<Body, Placement>,
) -> (f64, [f64; 2]) {
    mechanical::<f64>(reads, rule, poses)
}

fn separation<N: Number>(a: World<N>, b: World<N>) -> N {
    let delta = sub(b.point(), a.point());
    match (a, b) {
        (World::Point { .. }, World::Point { .. }) => norm(delta),
        (World::Axis { direction, .. }, World::Point { .. }) | (World::Point { .. }, World::Axis { direction, .. }) => {
            norm(cross(delta, direction))
        }
        (World::Axis { direction: u, .. }, World::Axis { direction: v, .. }) => {
            let n = cross(u, v);
            if norm(n).middle() < 1e-7 {
                norm(cross(delta, u))
            } else {
                let distance = dot(delta, unit(n));
                if distance.middle() < 0.0 { -distance } else { distance }
            }
        }
        _ => N::f(f64::NAN),
    }
}
fn coincident<N: Number>(out: &mut Vec<Row<N>>, a: World<N>, b: World<N>, sign: i8) {
    match (a, b) {
        (World::Axis { direction: u, through: p, .. }, World::Axis { direction: v, through: q, .. }) => {
            aligned(out, u, v, sign);
            on_axis(out, u, p, q);
        }
        (World::Axis { direction, through, .. }, World::Point { at, .. })
        | (World::Point { at, .. }, World::Axis { direction, through, .. }) => on_axis(out, direction, through, at),
        (World::Point { at: p, .. }, World::Point { at: q, .. }) => vector(out, sub(p, q), false),
        _ => scalar(out, N::f(f64::NAN), false),
    }
}
fn from_plane<N: Number>(out: &mut Vec<Row<N>>, a: World<N>, b: World<N>, sign: i8, lo: N, hi: Option<N>, rank: bool) {
    let (a, b) = if matches!(a, World::Plane { .. }) { (a, b) } else { (b, a) };
    let World::Plane { normal, offset } = a else {
        scalar(out, N::f(f64::NAN), false);
        return;
    };
    let gap = match b {
        World::Plane { normal: n, offset: o } => {
            aligned(out, normal, n, sign);
            o * N::f(sign as f64) - offset
        }
        World::Axis { direction, through, .. } => {
            scalar(out, dot(normal, direction), true);
            dot(normal, through) - offset
        }
        World::Point { at, .. } => dot(normal, at) - offset,
    };
    if let Some(hi) = hi {
        range(out, gap, lo, hi, false, rank);
    } else {
        scalar(out, gap - lo, false);
    }
}
fn bisector_orientation(a: &Read, b: &Read) -> i8 {
    let normal = |r: &Read| match &r.local {
        Local::Plane(p) => DVec3::from_array(p.normal.map(|x| x as f64)),
        Local::Native(s) => match s.as_ref() {
            EnclosedSupport::Plane { normal, .. } => DVec3::from_array(normal.each_ref().map(f64::bounds)).normalize(),
            _ => DVec3::NAN,
        },
        _ => DVec3::NAN,
    };
    // Choose in the part frame. A world-frame dot at 90 degrees changes sign as
    // numerical rotation roundoff changes, which would switch bisectors mid-solve.
    if normal(a).dot(normal(b)) > 0.0 { 1 } else { -1 }
}
fn bisector<N: Number>(a: World<N>, b: World<N>, orientation: i8) -> (V<N>, N) {
    let (World::Plane { normal: n, offset: o }, World::Plane { normal: m, offset: q }) = (a, b) else {
        return ([N::f(f64::NAN); 3], N::f(f64::NAN));
    };
    let sign = N::f(orientation as f64);
    let sum = add(n, mul(m, sign));
    let length = norm(sum);
    (mul(sum, N::f(1.0) / length), (o + sign * q) / length)
}
pub(super) fn residual<N: Number>(
    out: &mut Vec<Row<N>>,
    reads: &[Read],
    rule: &Rule,
    poses: &BTreeMap<Body, Placement>,
    rank: bool,
) {
    let a = world::<N>(&reads[0], poses);
    let b = world::<N>(&reads[1], poses);
    match rule {
        Rule::Mechanical { mechanism, offset, .. } => {
            let (value, _) = mechanical::<N>(reads, rule, poses);
            scalar(out, value - N::f(*offset), *mechanism == super::super::Mechanism::Gear);
        }
        Rule::Angle { min, max, limit } => {
            let n = world::<N>(&reads[2], poses).direction();
            let projection = |u: V<N>| unit(sub(u, mul(n, dot(u, n))));
            let (u, v) = (projection(a.direction()), projection(b.direction()));
            let (sine, cosine) = (dot(n, cross(u, v)), dot(u, v));
            if *limit {
                range(
                    out,
                    sine.atan2(cosine, ((*min + *max) * 0.5).to_radians()),
                    radians(*min),
                    radians(*max),
                    true,
                    rank,
                );
            } else {
                let (s, c) = radians::<N>(*min).sin_cos();
                scalar(out, sine - s, true);
                scalar(out, cosine - c, true);
            }
        }
        Rule::Parallel { sign, distance, coincident: is_coincident } => {
            if matches!(a, World::Plane { .. }) || matches!(b, World::Plane { .. }) {
                if let Some(d) = distance {
                    from_plane(out, a, b, *sign, N::f(*d), None, rank);
                } else {
                    aligned(out, a.direction(), b.direction(), *sign);
                }
            } else if *is_coincident {
                coincident(out, a, b, *sign);
            } else if let Some(d) = distance {
                if *d == 0.0 {
                    coincident(out, a, b, *sign);
                } else if let (
                    World::Axis { direction: u, through: p, .. },
                    World::Axis { direction: v, through: q, .. },
                ) = (a, b)
                {
                    aligned(out, u, v, *sign);
                    scalar(out, norm(cross(sub(q, p), u)) - N::f(*d), false);
                } else {
                    scalar(out, separation(a, b) - N::f(*d), false);
                }
            } else {
                aligned(out, a.direction(), b.direction(), *sign);
            }
        }
        Rule::Perpendicular => scalar(out, dot(a.direction(), b.direction()), true),
        Rule::Concentric { sign, lock } => {
            coincident(out, a, b, sign.unwrap_or(1));
            if let Some(lock) = lock {
                turn(out, &poses[&reads[0].body], &poses[&reads[1].body], lock.rotation().unwrap_or(DQuat::IDENTITY));
            }
        }
        Rule::Limit { sign, min, max } => from_plane(out, a, b, *sign, N::f(*min), Some(N::f(*max)), rank),
        Rule::Tangent { sign } => {
            if let World::Plane { normal, offset } = a {
                let Some((r, outward)) = b.radius() else {
                    scalar(out, N::f(f64::NAN), false);
                    return;
                };
                let gap = if outward == (*sign < 0) { r } else { -r };
                if let World::Axis { direction, .. } = b {
                    scalar(out, dot(normal, direction), true);
                }
                scalar(out, dot(normal, b.point()) - offset - gap, false);
            } else {
                let (Some((ra, oa)), Some((rb, ob))) = (a.radius(), b.radius()) else {
                    scalar(out, N::f(f64::NAN), false);
                    return;
                };
                let external = (oa == ob) == (*sign < 0);
                let difference = ra - rb;
                let d = if external {
                    ra + rb
                } else if difference.middle() < 0.0 {
                    -difference
                } else {
                    difference
                };
                if let (World::Axis { direction: u, through: p, .. }, World::Axis { direction: v, through: q, .. }) =
                    (a, b)
                    && norm(cross(u, v)).middle() < 1e-7
                {
                    aligned(out, u, v, if dot(u, v).middle() < 0.0 { -1 } else { 1 });
                    scalar(out, norm(cross(sub(q, p), u)) - d, false);
                } else {
                    scalar(out, separation(a, b) - d, false);
                }
            }
        }
        Rule::Hinge { sign, plane_sign, limits } => {
            coincident(out, a, b, *sign);
            if let Some([lo, hi]) = limits {
                let clock = |read: &Read| {
                    let local =
                        world::<N>(read, &BTreeMap::from([(read.body.clone(), Placement::IDENTITY)])).direction();
                    let index =
                        (0..3).min_by(|i, j| local[*i].middle().abs().total_cmp(&local[*j].middle().abs())).unwrap();
                    let basis = std::array::from_fn(|k| N::f(if k == index { 1.0 } else { 0.0 }));
                    rotate(&poses[&read.body], unit(cross(local, basis)))
                };
                let (u, v) = (clock(&reads[0]), clock(&reads[1]));
                let n = a.direction();
                range(
                    out,
                    dot(n, cross(u, v)).atan2(dot(u, v), ((*lo + *hi) * 0.5).to_radians()),
                    radians(*lo),
                    radians(*hi),
                    true,
                    rank,
                );
            }
            from_plane(out, world(&reads[2], poses), world(&reads[3], poses), *plane_sign, N::f(0.0), None, rank);
        }
        Rule::Width { sign, .. } => {
            let (na, oa) = bisector(a, b, bisector_orientation(&reads[0], &reads[1]));
            let (nb, ob) =
                bisector(world(&reads[2], poses), world(&reads[3], poses), bisector_orientation(&reads[2], &reads[3]));
            aligned(out, na, nb, *sign);
            scalar(out, ob * N::f(*sign as f64) - oa, false);
        }
        Rule::Symmetric { sign } => {
            let World::Plane { normal: n, offset: o } = world(&reads[2], poses) else {
                scalar(out, N::f(f64::NAN), false);
                return;
            };
            let sign = sign.unwrap_or(1);
            if let (Some((ra, _)), Some((rb, _))) = (a.radius(), b.radius()) {
                scalar(out, ra - rb, false);
            }
            match (a, b) {
                (World::Plane { normal: u, offset: p }, World::Plane { normal: v, offset: q }) => {
                    let mirrored = mirror(u, n);
                    aligned(out, mirrored, v, sign);
                    scalar(out, dot(mirrored, reflected(mul(u, p), n, o)) - q * N::f(sign as f64), false);
                }
                (World::Axis { direction: u, through: p, .. }, World::Axis { direction: v, through: q, .. }) => {
                    aligned(out, mirror(u, n), v, sign);
                    on_axis(out, v, reflected(p, n, o), q);
                }
                (World::Point { at: p, .. }, World::Point { at: q, .. }) => {
                    vector(out, sub(reflected(p, n, o), q), false)
                }
                _ => scalar(out, N::f(f64::NAN), false),
            }
        }
        Rule::Coupler { ratio, reverse, offset, references, frame } => {
            let slide = |i: usize, s: World<N>| {
                let p = &poses[&reads[i].body];
                let n = s.direction();
                let from = references[i]
                    .as_ref()
                    .map_or_else(|| frame.each_ref().map(N::q), |b| poses[b].translation.to_array().map(N::f));
                let local_offset = dot(n, sub(s.point(), p.translation.to_array().map(N::f)));
                local_offset + dot(n, sub(p.translation.to_array().map(N::f), from))
            };
            let sign = N::f(if *reverse { 1.0 } else { -1.0 });
            scalar(out, N::q(&ratio[0]) * slide(1, b) + sign * N::q(&ratio[1]) * slide(0, a) - N::q(offset), false);
        }
        Rule::Profile { .. } | Rule::Slot { .. } => scalar(out, N::f(f64::NAN), false),
    }
}

pub(super) fn seed_hinge(reads: &[Read], rule: &Rule, poses: &mut BTreeMap<Body, Placement>, fixed: &BTreeSet<Body>) {
    let Rule::Hinge { sign, plane_sign, limits } = rule else { return };
    let (a, b) = (&reads[0].body, &reads[1].body);
    let moving = if !fixed.contains(b) {
        b
    } else if !fixed.contains(a) {
        a
    } else {
        return;
    };
    let u = DVec3::from_array(world::<f64>(&reads[0], poses).direction());
    let v = DVec3::from_array(world::<f64>(&reads[1], poses).direction());
    let (current, target) = if moving == b { (v, u * (*sign as f64)) } else { (u, v * (*sign as f64)) };
    if current.try_normalize().is_none() || target.try_normalize().is_none() {
        return;
    }
    let p = poses.get_mut(moving).expect("known hinge body");
    p.rotation = crate::placement::canonical((shortest_arc(current, target) * p.rotation).normalize());
    let n = DVec3::from_array(world::<f64>(&reads[0], poses).direction()).normalize();
    let target = DVec3::from_array(world::<f64>(&reads[2], poses).direction()) * (*plane_sign as f64);
    let current = DVec3::from_array(world::<f64>(&reads[3], poses).direction());
    let (target, current) = (target - n * target.dot(n), current - n * current.dot(n));
    if target.length_squared() > 1e-12 && current.length_squared() > 1e-12 {
        let (target, current) = (target.normalize(), current.normalize());
        let angle = libm::atan2(n.dot(current.cross(target)), current.dot(target));
        let half = angle * if moving == b { 0.5 } else { -0.5 };
        let sine = libm::sin(half);
        let q = DQuat::from_xyzw(n.x * sine, n.y * sine, n.z * sine, libm::cos(half));
        let p = poses.get_mut(moving).expect("known hinge body");
        p.rotation = crate::placement::canonical((q * p.rotation).normalize());
    }
    if let Some([lo, hi]) = limits {
        let clock = |read: &Read| {
            let local = world::<f64>(read, &BTreeMap::from([(read.body.clone(), Placement::IDENTITY)])).direction();
            let index = (0..3).min_by(|i, j| local[*i].abs().total_cmp(&local[*j].abs())).unwrap();
            let basis = std::array::from_fn(|k| if k == index { 1.0 } else { 0.0 });
            rotate(&poses[&read.body], unit(cross(local, basis)))
        };
        let (u, v) = (clock(&reads[0]), clock(&reads[1]));
        let n = world::<f64>(&reads[0], poses).direction();
        let near = ((*lo + *hi) * 0.5).to_radians();
        let angle = <f64 as Number>::atan2(dot(n, cross(u, v)), dot(u, v), near);
        let target = angle.clamp(lo.to_radians(), hi.to_radians());
        let half = (target - angle) * if moving == b { 0.5 } else { -0.5 };
        let sine = libm::sin(half);
        let q = DQuat::from_xyzw(n[0] * sine, n[1] * sine, n[2] * sine, libm::cos(half));
        if q.is_finite() {
            let p = poses.get_mut(moving).expect("known hinge body");
            p.rotation = crate::placement::canonical((q * p.rotation).normalize());
        }
    }
}
