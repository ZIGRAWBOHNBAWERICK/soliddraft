//! Placing components from proven planes, and counting what stays free.
//!
//! Plane normals are exact signed axes here, which splits the problem in two:
//!
//! 1. **Rotations.** Each mate between planes relates two normals: equal, opposite or at
//!    right angles. An Origin mate with axes fixes a rotation outright, and a Lock fixes one
//!    body's from another's. Rotations spread outward from what is fixed. A body held by two
//!    perpendicular normals turns exactly; one held by a single normal turns as little as
//!    it can, keeping its spin. A right angle the smallest turn does not keep is met by the
//!    nearest quarter-turn rotation that keeps every relation.
//! 2. **Translations.** With rotations known, each distance, width and lock is a difference
//!    of one world coordinate between two bodies, so each axis is a graph of exact
//!    differences. Walking it places what is connected to the assembly's origin, or to the
//!    graph's first body, and a closing edge that disagrees is an over-constrained set of
//!    mates.
//!
//! What is not constrained keeps its authored value. Nothing is fixed behind the user's
//! back: an unanchored group keeps its first body's position only because something must
//! stay put, and it still counts its freedoms ([`freedoms`] works on the whole Jacobian of
//! the rigid-body motions, so a shared rotation about an anchored origin counts too).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use glam::{DMat3, DQuat, DVec3};
use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Signed, ToPrimitive, Zero};
use serde::Serialize;
use soliddraft_kernel_api::ExactRational;

use crate::mates::{MateRefusal, RefusalCode};
use crate::path::{ComponentId, OccurrencePath};
use crate::placement::Placement;

/// Two mates agree within this, in millimetres; a difference above it is a conflict.
const CONFLICT_MM: f64 = 1e-9;
/// How far a placed normal may lie from an axis and still be that axis.
const AXIS_TOLERANCE: f64 = 1e-12;
/// A pivot smaller than this, in a row scaled to one, is zero.
const RANK_TOLERANCE: f64 = 1e-9;
/// The most mated components one assembly may hold.
pub(crate) const MAX_BODIES: usize = 128;

type Normal = [i8; 3];
/// A body is a part occurrence, named by its path from the root. A nested one moves inside
/// its sub-assembly's frame, which stays where it is authored.
pub(crate) type Body = OccurrencePath;

/// A plane in its part's frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LocalPlane {
    /// The outward normal.
    pub normal: Normal,
    /// Where it lies along the normal.
    pub offset: BigRational,
}

/// What fixes the translation between two parallel planes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Along {
    /// The signed distance from `a`'s plane to `b`'s, along `a`'s outward normal.
    Distance(f64),
    /// A tab between a groove's faces: `plane_a` and `groove` are the groove's, on `a`;
    /// `plane_b` and `tab` the tab's, on `b`.
    Width { groove: LocalPlane, tab: LocalPlane, mode: WidthMode },
}

/// Where a width puts the tab.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WidthMode {
    Centered,
    /// The gap from the first groove face, or the second when `flipped`, to the tab face
    /// nearest it.
    Gap {
        distance: f64,
        flipped: bool,
    },
    /// That gap as a fraction of the play.
    Share {
        fraction: BigRational,
        flipped: bool,
    },
}

/// One mate as the solver sees it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Constraint {
    /// The body's origin on a frame: the root's origin for the root's own mates, a
    /// sub-assembly's frame for its mates.
    Origin { mate: String, body: Body, axes: bool, at: Placement },
    /// Two parallel planes, their outward normals related by `sign`. `along` fixes the
    /// translation between them when there is one.
    Parallel {
        mate: String,
        a: Body,
        b: Body,
        plane_a: LocalPlane,
        plane_b: LocalPlane,
        /// +1 when the outward normals agree, -1 when they oppose.
        sign: i8,
        along: Option<Along>,
    },
    /// Two planes at right angles.
    Perpendicular { mate: String, a: Body, b: Body, plane_a: LocalPlane, plane_b: LocalPlane },
    /// `b`'s placement is `a`'s composed with `relative`.
    Lock { mate: String, a: Body, b: Body, relative: Placement },
}

impl Constraint {
    fn mate(&self) -> &str {
        match self {
            Constraint::Origin { mate, .. }
            | Constraint::Parallel { mate, .. }
            | Constraint::Perpendicular { mate, .. }
            | Constraint::Lock { mate, .. } => mate,
        }
    }

    /// The two bodies a constraint between bodies ties.
    fn pair(&self) -> Option<(&Body, &Body)> {
        match self {
            Constraint::Parallel { a, b, .. }
            | Constraint::Perpendicular { a, b, .. }
            | Constraint::Lock { a, b, .. } => Some((a, b)),
            Constraint::Origin { .. } => None,
        }
    }
}

/// A motion a component keeps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FreeMotion {
    /// The component: the root's own, or for a nested part the step that places it.
    pub component: ComponentId,
    /// The nested part's path from the root; absent for the root's own components.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<OccurrencePath>,
    /// The motion: `translate_x`, `spin_z` and so on.
    pub motion: String,
}

impl FreeMotion {
    /// The motion for people, such as "component 4 spin about Z".
    pub fn describe(&self) -> String {
        describe(self)
    }

    fn of(body: &Body, motion: &str) -> FreeMotion {
        let component = body.steps().last().map_or(ComponentId(0), |s| s.component);
        let path = (body.steps().len() > 1).then(|| body.clone());
        FreeMotion { component, path, motion: motion.to_owned() }
    }
}

/// How constrained an assembly is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SolverState {
    /// Some freedom remains. A valid state: mates go on one at a time.
    Underconstrained,
    /// Nothing can move.
    Solved,
    /// The mates cannot be read or do not hold.
    Error,
}

/// What the solver says of the accepted assembly.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SolverStatus {
    /// The verdict.
    pub state: SolverState,
    /// How many independent rigid-body motions remain.
    pub remaining_dof: usize,
    /// The motions one component can make alone. Group motions are only counted.
    pub free: Vec<FreeMotion>,
    /// Problems, for people.
    pub diagnostics: Vec<String>,
}

impl SolverStatus {
    pub(crate) fn error(diagnostic: String) -> SolverStatus {
        SolverStatus { state: SolverState::Error, remaining_dof: 0, free: Vec::new(), diagnostics: vec![diagnostic] }
    }
}

pub(crate) fn rational(value: &ExactRational) -> Option<BigRational> {
    let n: BigInt = value.numerator.parse().ok()?;
    let d: BigInt = value.denominator.parse().ok()?;
    (!d.is_zero()).then(|| BigRational::new(n, d))
}

pub(crate) fn exact(value: f64) -> Option<BigRational> {
    BigRational::from_float(value)
}

pub(crate) fn nearest(value: &BigRational) -> f64 {
    value.to_f64().unwrap_or(f64::NAN)
}

/// Text for a rational: `5`, or `381/5`.
pub(crate) fn text(value: &BigRational) -> String {
    if value.is_integer() { value.numer().to_string() } else { format!("{}/{}", value.numer(), value.denom()) }
}

fn vector(n: Normal) -> DVec3 {
    DVec3::new(n[0] as f64, n[1] as f64, n[2] as f64)
}

/// The signed axis a direction lies on, if it lies on one.
fn snapped(v: DVec3) -> Option<Normal> {
    let n = v.to_array().map(|c| c.round());
    let on_axis = n.iter().map(|c| c.abs()).sum::<f64>() == 1.0;
    (on_axis && v.distance(DVec3::from_array(n)) <= AXIS_TOLERANCE).then(|| n.map(|c| c as i8))
}

fn world(rotation: DQuat, n: Normal, mate: &str) -> Result<Normal, MateRefusal> {
    snapped(rotation * vector(n)).ok_or_else(|| {
        let message = format!(
            "{mate}: a face is placed off the world axes; this version proves planes on axis-aligned placements only"
        );
        MateRefusal { mates: vec![mate.to_owned()], ..MateRefusal::new(RefusalCode::UnsupportedPlacement, message) }
    })
}

fn negate(n: Normal) -> Normal {
    n.map(|c| -c)
}

fn dot(a: Normal, b: Normal) -> i8 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Normal, b: Normal) -> Normal {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn conflict(mates: Vec<String>, message: String) -> MateRefusal {
    MateRefusal { mates, ..MateRefusal::new(RefusalCode::OverConstrained, message) }
}

/// The rotation taking each local direction to its world direction, if one exists,
/// staying as close to `authored` as the pairs allow.
fn orient(authored: DQuat, pairs: &[(Normal, Normal)]) -> Option<DQuat> {
    let Some(&(u0, v0)) = pairs.first() else { return Some(authored) };
    let second = pairs.iter().find(|(u, _)| dot(*u, u0) == 0).copied();
    let agrees = |(u, v): &(Normal, Normal), (x, y): (Normal, Normal)| dot(*u, x) == dot(*v, y);
    if !pairs.iter().all(|p| agrees(p, (u0, v0)) && second.is_none_or(|s| agrees(p, s))) {
        return None;
    }
    let Some((u1, v1)) = second else {
        return Some(shortest_arc(authored * vector(u0), vector(v0)) * authored);
    };
    let frame = |a, b| DMat3::from_cols(vector(a), vector(b), vector(cross(a, b)));
    Some(DQuat::from_mat3(&(frame(v0, v1) * frame(u0, u1).transpose())))
}

/// Whether `rotation` takes each `(u, w)` local direction `u` to a world axis at right
/// angles to `w`.
fn square(rotation: DQuat, perpendicular: &[(Normal, Normal)]) -> bool {
    perpendicular.iter().all(|(u, w)| snapped(rotation * vector(*u)).is_some_and(|n| dot(n, *w) == 0))
}

/// A rotation as the integer matrix it is when it takes axes to axes.
type Cube = [[i8; 3]; 3];

fn apply(m: &Cube, n: Normal) -> Normal {
    [0, 1, 2].map(|i| m[i][0] * n[0] + m[i][1] * n[1] + m[i][2] * n[2])
}

/// The 24 rotations that take axes to axes, in a fixed order.
fn cube_rotations() -> Vec<Cube> {
    let mut out = Vec::with_capacity(24);
    for perm in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        for signs in
            [[1, 1, 1], [1, 1, -1], [1, -1, 1], [1, -1, -1], [-1, 1, 1], [-1, 1, -1], [-1, -1, 1], [-1, -1, -1]]
        {
            let mut m = [[0i8; 3]; 3];
            for (col, row) in perm.iter().enumerate() {
                m[*row][col] = signs[col];
            }
            let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
            if det == 1 {
                out.push(m);
            }
        }
    }
    out
}

fn quaternion(m: &Cube) -> DQuat {
    let col = |c: usize| DVec3::new(m[0][c] as f64, m[1][c] as f64, m[2][c] as f64);
    DQuat::from_mat3(&DMat3::from_cols(col(0), col(1), col(2)))
}

/// The rotation's integer matrix, when it takes every axis to an axis.
pub(crate) fn cube_of(rotation: DQuat) -> Option<Cube> {
    let mut m = [[0i8; 3]; 3];
    for (c, axis) in [[1, 0, 0], [0, 1, 0], [0, 0, 1]].into_iter().enumerate() {
        let n = snapped(rotation * vector(axis))?;
        for r in 0..3 {
            m[r][c] = n[r];
        }
    }
    Some(m)
}

/// The quarter-turn rotation nearest `authored` that takes each pair's local direction to
/// its world one and keeps each right angle, first in a fixed order among equals.
fn nearest_cube(authored: DQuat, pairs: &[(Normal, Normal)], perpendicular: &[(Normal, Normal)]) -> Option<DQuat> {
    let mut best: Option<(f64, DQuat)> = None;
    for m in cube_rotations() {
        let holds = pairs.iter().all(|(u, v)| apply(&m, *u) == *v)
            && perpendicular.iter().all(|(u, w)| dot(apply(&m, *u), *w) == 0);
        if !holds {
            continue;
        }
        let q = quaternion(&m);
        let near = q.dot(authored).abs();
        if best.is_none_or(|(b, _)| near > b + AXIS_TOLERANCE) {
            best = Some((near, q));
        }
    }
    best.map(|(_, q)| q)
}

/// The smallest turn taking unit `from` to the axis direction `to`. A half turn goes about
/// a world axis, so it stays exact when `from` lies on an axis too.
fn shortest_arc(from: DVec3, to: DVec3) -> DQuat {
    if from.distance(to) <= AXIS_TOLERANCE {
        return DQuat::IDENTITY;
    }
    if from.distance(-to) <= AXIS_TOLERANCE {
        let helper = [DVec3::X, DVec3::Y, DVec3::Z].into_iter().find(|a| a.dot(to).abs() < 0.5).expect("an axis");
        let axis = to.cross(helper).normalize();
        return DQuat::from_xyzw(axis.x, axis.y, axis.z, 0.0);
    }
    DQuat::from_rotation_arc(from, to)
}

/// What one mate says about the body being turned, given another it ties it to.
enum Tie {
    /// The local normal on the body being turned goes where the other's does, by sign.
    Pair { own: Normal, theirs: Normal, sign: i8 },
    /// The local normal on the body being turned is at right angles to the other's.
    Square { own: Normal, theirs: Normal },
    /// The body's rotation is the other's times this.
    Locked(DQuat),
}

struct Link<'a> {
    other: &'a Body,
    tie: Tie,
    mate: &'a str,
}

fn links<'a>(constraints: &'a [Constraint], body: &Body) -> Vec<Link<'a>> {
    let mut out = Vec::new();
    for c in constraints {
        let Some((a, b)) = c.pair() else { continue };
        let (other, mine_is_b) = if b == body {
            (a, true)
        } else if a == body {
            (b, false)
        } else {
            continue;
        };
        let tie = match c {
            Constraint::Parallel { plane_a, plane_b, sign, .. } => {
                let (own, theirs) = if mine_is_b { (plane_b, plane_a) } else { (plane_a, plane_b) };
                Tie::Pair { own: own.normal, theirs: theirs.normal, sign: *sign }
            }
            Constraint::Perpendicular { plane_a, plane_b, .. } => {
                let (own, theirs) = if mine_is_b { (plane_b, plane_a) } else { (plane_a, plane_b) };
                Tie::Square { own: own.normal, theirs: theirs.normal }
            }
            Constraint::Lock { relative, .. } => {
                Tie::Locked(if mine_is_b { relative.rotation } else { relative.rotation.inverse() })
            }
            Constraint::Origin { .. } => continue,
        };
        out.push(Link { other, tie, mate: c.mate() });
    }
    out
}

/// The names of every mate between `a` and `b`, sorted.
fn between(constraints: &[Constraint], a: &Body, b: &Body) -> Vec<String> {
    let mut names: Vec<String> =
        links(constraints, a).iter().filter(|l| l.other == b).map(|l| l.mate.to_owned()).collect();
    names.sort();
    names.dedup();
    names
}

fn incompatible(names: Vec<String>) -> MateRefusal {
    let message = format!("{}: their faces would need incompatible directions", names.join(", "));
    conflict(names, message)
}

fn rotations(
    bodies: &BTreeMap<Body, Placement>,
    constraints: &[Constraint],
) -> Result<BTreeMap<Body, DQuat>, MateRefusal> {
    let mut known = BTreeMap::new();
    for c in constraints {
        if let Constraint::Origin { body, axes: true, at, .. } = c {
            known.insert(body.clone(), at.rotation);
        }
    }
    let mut pending: BTreeSet<Body> = bodies.keys().filter(|b| !known.contains_key(*b)).cloned().collect();
    while !pending.is_empty() {
        // The body most tied to what is known: a lock decides a rotation outright, and a
        // body held by one normal must not take a spin before a second normal reaches it.
        let reach = |body: &Body| {
            let tied: Vec<_> = links(constraints, body).into_iter().filter(|l| known.contains_key(l.other)).collect();
            (tied.iter().any(|l| matches!(l.tie, Tie::Locked(_))), tied.len())
        };
        let best = pending.iter().max_by_key(|b| (reach(b), std::cmp::Reverse(*b))).expect("not empty").clone();
        pending.remove(&best);
        let authored = bodies[&best].rotation;
        let tied: Vec<Link> = links(constraints, &best).into_iter().filter(|l| known.contains_key(l.other)).collect();
        if let Some((other, relative)) = tied.iter().find_map(|l| match l.tie {
            Tie::Locked(r) => Some((l.other, r)),
            _ => None,
        }) {
            known.insert(best, (known[other] * relative).normalize());
            continue;
        }
        let (mut pairs, mut right) = (Vec::new(), Vec::new());
        let mut names: Vec<String> = Vec::new();
        for l in &tied {
            match l.tie {
                Tie::Pair { own, theirs, sign } => {
                    let there = world(known[l.other], theirs, l.mate)?;
                    pairs.push((own, if sign > 0 { there } else { negate(there) }));
                }
                Tie::Square { own, theirs } => right.push((own, world(known[l.other], theirs, l.mate)?)),
                Tie::Locked(_) => unreachable!("a lock is taken above"),
            }
            names.push(l.mate.to_owned());
        }
        let turned = orient(authored, &pairs)
            .filter(|r| square(*r, &right))
            .or_else(|| (!right.is_empty()).then(|| nearest_cube(authored, &pairs, &right)).flatten())
            .ok_or_else(|| {
                names.sort();
                names.dedup();
                incompatible(names.clone())
            })?;
        known.insert(best, turned);
    }
    for c in constraints {
        match c {
            Constraint::Parallel { mate, a, b, plane_a, plane_b, sign, .. } => {
                let (na, nb) = (world(known[a], plane_a.normal, mate)?, world(known[b], plane_b.normal, mate)?);
                let wanted = if *sign > 0 { na } else { negate(na) };
                if nb != wanted {
                    return Err(incompatible(between(constraints, a, b)));
                }
            }
            Constraint::Perpendicular { mate, a, b, plane_a, plane_b } => {
                let (na, nb) = (world(known[a], plane_a.normal, mate)?, world(known[b], plane_b.normal, mate)?);
                if dot(na, nb) != 0 {
                    return Err(incompatible(between(constraints, a, b)));
                }
            }
            Constraint::Lock { a, b, relative, .. } => {
                let wanted = known[a] * relative.rotation;
                let off = [DVec3::X, DVec3::Y].iter().any(|v| (wanted * *v).distance(known[b] * *v) > AXIS_TOLERANCE);
                if off {
                    return Err(incompatible(between(constraints, a, b)));
                }
            }
            Constraint::Origin { .. } => {}
        }
    }
    Ok(known)
}

/// `t[to] - t[from] = gap` along one world axis. `from == None` is the assembly's origin.
struct Edge<'a> {
    from: Option<&'a Body>,
    to: &'a Body,
    gap: BigRational,
    mate: &'a str,
    /// What the mate asks for, for a message.
    says: String,
}

fn edges<'a>(
    constraints: &'a [Constraint],
    turned: &BTreeMap<Body, DQuat>,
    axis: usize,
) -> Result<Vec<Edge<'a>>, MateRefusal> {
    let mut out = Vec::new();
    for c in constraints {
        match c {
            Constraint::Origin { mate, body, at, .. } => {
                let gap = exact(at.translation[axis])
                    .ok_or_else(|| MateRefusal::invalid(format!("{mate}: the frame is not finite")))?;
                out.push(Edge { from: None, to: body, gap, mate, says: "at the origin".into() })
            }
            Constraint::Parallel { mate, a, b, plane_a, plane_b, sign, along: Some(along) } => {
                let n = world(turned[a], plane_a.normal, mate)?;
                if n[axis] == 0 {
                    continue;
                }
                let s = BigRational::from_integer(BigInt::from(*sign));
                let side = BigRational::from_integer(BigInt::from(n[axis]));
                let (gap, says) = match along {
                    Along::Distance(distance) => {
                        let d = exact(*distance)
                            .ok_or_else(|| MateRefusal::invalid(format!("{mate}: the distance is not finite")))?;
                        (side * (d + &plane_a.offset - s * &plane_b.offset), format!("{distance} mm"))
                    }
                    Along::Width { groove, tab, mode } => {
                        // Where each plane lies along this axis, from its body's origin.
                        let at = |rotation: DQuat, p: &LocalPlane| -> Result<BigRational, MateRefusal> {
                            let w = world(rotation, p.normal, mate)?;
                            if w[axis] == 0 {
                                return Err(MateRefusal {
                                    mates: vec![mate.clone()],
                                    ..MateRefusal::new(
                                        RefusalCode::NotParallel,
                                        format!("{mate}: its faces are not parallel"),
                                    )
                                });
                            }
                            Ok(BigRational::from_integer(BigInt::from(w[axis])) * &p.offset)
                        };
                        let g = [at(turned[a], plane_a)?, at(turned[a], groove)?];
                        let h = [at(turned[b], plane_b)?, at(turned[b], tab)?];
                        let two = BigRational::from_integer(BigInt::from(2));
                        let (h_lo, h_hi) = if h[0] <= h[1] { (&h[0], &h[1]) } else { (&h[1], &h[0]) };
                        let gap_from = |from: &BigRational, other: &BigRational, gap: BigRational| {
                            if from <= other { from + gap - h_lo } else { from - gap - h_hi }
                        };
                        match mode {
                            WidthMode::Centered => ((&g[0] + &g[1] - &h[0] - &h[1]) / two, "centred".to_owned()),
                            WidthMode::Gap { distance, flipped } => {
                                let d = exact(*distance).ok_or_else(|| {
                                    MateRefusal::invalid(format!("{mate}: the distance is not finite"))
                                })?;
                                let (from, other) = if *flipped { (&g[1], &g[0]) } else { (&g[0], &g[1]) };
                                (gap_from(from, other, d), format!("{distance} mm from a groove face"))
                            }
                            WidthMode::Share { fraction, flipped } => {
                                let (from, other) = if *flipped { (&g[1], &g[0]) } else { (&g[0], &g[1]) };
                                let play = (from - other).abs() - (h_hi - h_lo);
                                let percent = nearest(&(fraction * BigRational::from_integer(BigInt::from(100))));
                                (gap_from(from, other, fraction * play), format!("{percent}% of the play"))
                            }
                        }
                    }
                };
                out.push(Edge { from: Some(a), to: b, gap, mate, says });
            }
            Constraint::Lock { mate, a, b, relative } => {
                let m = cube_of(turned[a]).ok_or_else(|| unsupported_lock(mate))?;
                let t = relative.translation.to_array();
                let j = (0..3).find(|j| m[axis][*j] != 0).expect("a signed permutation");
                let offset =
                    exact(t[j]).ok_or_else(|| MateRefusal::invalid(format!("{mate}: the lock is not finite")))?;
                let gap = BigRational::from_integer(BigInt::from(m[axis][j])) * offset;
                out.push(Edge { from: Some(a), to: b, gap, mate, says: "locked".into() });
            }
            Constraint::Parallel { along: None, .. } | Constraint::Perpendicular { .. } => {}
        }
    }
    Ok(out)
}

/// The refusal for a lock whose parts are turned off the quarter turns.
pub(crate) fn unsupported_lock(mate: &str) -> MateRefusal {
    let message = format!(
        "{mate}: a locked part is turned off the quarter turns; this version locks quarter-turn rotations only"
    );
    MateRefusal { mates: vec![mate.to_owned()], ..MateRefusal::new(RefusalCode::UnsupportedPlacement, message) }
}

/// Every world coordinate of every body along one axis.
fn walk<'a>(
    bodies: &'a BTreeMap<Body, Placement>,
    edges: &[Edge<'a>],
    axis: usize,
) -> Result<BTreeMap<&'a Body, BigRational>, MateRefusal> {
    type Node<'b> = Option<&'b Body>;
    let mut adjacent: BTreeMap<Node, Vec<usize>> = BTreeMap::new();
    for (i, e) in edges.iter().enumerate() {
        adjacent.entry(e.from).or_default().push(i);
        adjacent.entry(Some(e.to)).or_default().push(i);
    }
    let mut at: BTreeMap<Node, BigRational> = BTreeMap::new();
    let mut parent: BTreeMap<Node, usize> = BTreeMap::new();
    let roots = std::iter::once(None).chain(bodies.keys().map(Some));
    for root in roots {
        if at.contains_key(&root) {
            continue;
        }
        let start = match root {
            None => BigRational::zero(),
            Some(b) => exact(bodies[b].translation[axis])
                .ok_or_else(|| MateRefusal::invalid(format!("occurrence {b} has no finite placement")))?,
        };
        at.insert(root, start);
        let mut queue = VecDeque::from([root]);
        while let Some(node) = queue.pop_front() {
            for &i in adjacent.get(&node).into_iter().flatten() {
                let e = &edges[i];
                let (other, value) =
                    if e.from == node { (Some(e.to), &at[&node] + &e.gap) } else { (e.from, &at[&node] - &e.gap) };
                match at.get(&other) {
                    None => {
                        at.insert(other, value);
                        parent.insert(other, i);
                        queue.push_back(other);
                    }
                    Some(placed) => {
                        let apart = (placed - &value).abs();
                        if nearest(&apart) > CONFLICT_MM {
                            return Err(disagreement(edges, &parent, node, other, i));
                        }
                    }
                }
            }
        }
    }
    Ok(at.into_iter().filter_map(|(node, value)| Some((node?, value))).collect())
}

/// The refusal for an edge that closes a cycle the wrong way: every mate on the cycle.
fn disagreement<'a>(
    edges: &[Edge<'a>],
    parent: &BTreeMap<Option<&'a Body>, usize>,
    a: Option<&'a Body>,
    b: Option<&'a Body>,
    closing: usize,
) -> MateRefusal {
    let climb = |mut node: Option<&'a Body>| {
        let mut path = Vec::new();
        while let Some(&i) = parent.get(&node) {
            path.push(i);
            let e = &edges[i];
            node = if Some(e.to) == node { e.from } else { Some(e.to) };
        }
        path
    };
    let (up_a, up_b) = (climb(a), climb(b));
    let mut cycle = vec![closing];
    cycle.extend(up_a.iter().filter(|i| !up_b.contains(i)));
    cycle.extend(up_b.iter().filter(|i| !up_a.contains(i)));
    cycle.sort_by_key(|i| edges[*i].mate);
    cycle.dedup();
    let names: Vec<String> = cycle.iter().map(|i| edges[*i].mate.to_owned()).collect();
    let says: Vec<String> = cycle.iter().map(|i| format!("{} ({})", edges[*i].mate, edges[*i].says)).collect();
    conflict(names, format!("{} cannot all hold", says.join(", ")))
}

/// The placements that satisfy `constraints`, starting from `bodies`.
pub(crate) fn solve(
    bodies: &BTreeMap<Body, Placement>,
    constraints: &[Constraint],
) -> Result<BTreeMap<Body, Placement>, MateRefusal> {
    let turned = rotations(bodies, constraints)?;
    let mut coordinates = Vec::with_capacity(3);
    for axis in 0..3 {
        coordinates.push(walk(bodies, &edges(constraints, &turned, axis)?, axis)?);
    }
    Ok(bodies
        .iter()
        .map(|(body, authored)| {
            let t = DVec3::new(
                nearest(&coordinates[0][body]),
                nearest(&coordinates[1][body]),
                nearest(&coordinates[2][body]),
            );
            let rotation = turned[body];
            let kept = rotation == authored.rotation && t == authored.translation;
            (body.clone(), if kept { *authored } else { Placement::new(rotation, t) })
        })
        .collect())
}

/// One row of the constraint Jacobian over six motions per body, translation first.
fn row(size: usize, entries: &[(usize, f64)]) -> Vec<f64> {
    let mut r = vec![0.0; size];
    for (i, v) in entries {
        r[*i] += v;
    }
    let scale = r.iter().fold(0.0_f64, |m, v| m.max(v.abs()));
    if scale > 0.0 {
        r.iter_mut().for_each(|v| *v /= scale);
    }
    r
}

/// The Jacobian of the constraints at `bodies`. Each body moves by a translation and a
/// turn about its own origin. A plane's point can be any point of the body: the choices
/// differ by what the normal rows already cover, so each body's origin stands in.
fn jacobian(
    index: &BTreeMap<Body, usize>,
    bodies: &BTreeMap<Body, Placement>,
    constraints: &[Constraint],
) -> Result<Vec<Vec<f64>>, MateRefusal> {
    let size = index.len() * 6;
    let mut rows = Vec::new();
    for c in constraints {
        match c {
            Constraint::Origin { body, axes, .. } => {
                let at = index[body] * 6;
                for k in 0..3 {
                    rows.push(row(size, &[(at + k, 1.0)]));
                    if *axes {
                        rows.push(row(size, &[(at + 3 + k, 1.0)]));
                    }
                }
            }
            Constraint::Parallel { mate, a, b, plane_a, along, .. } => {
                let (pa, pb) = (&bodies[a], &bodies[b]);
                let n = world(pa.rotation, plane_a.normal, mate)?;
                let (at_a, at_b) = (index[a] * 6, index[b] * 6);
                for e in (0..3).filter(|k| n[*k] == 0) {
                    rows.push(row(size, &[(at_b + 3 + e, 1.0), (at_a + 3 + e, -1.0)]));
                }
                if along.is_none() {
                    continue;
                }
                // d/dt of n · (t_b - t_a), with n turning with a.
                let normal = vector(n);
                let lever = normal.cross(pb.translation - pa.translation);
                let mut entries = Vec::new();
                for k in 0..3 {
                    entries.push((at_b + k, normal[k]));
                    entries.push((at_a + k, -normal[k]));
                    entries.push((at_a + 3 + k, lever[k]));
                }
                rows.push(row(size, &entries));
            }
            Constraint::Perpendicular { mate, a, b, plane_a, plane_b } => {
                // d/dt of n_a · n_b, each turning with its body: (w_a - w_b) · (n_a x n_b).
                let na = vector(world(bodies[a].rotation, plane_a.normal, mate)?);
                let nb = vector(world(bodies[b].rotation, plane_b.normal, mate)?);
                let c = na.cross(nb);
                let (at_a, at_b) = (index[a] * 6, index[b] * 6);
                let entries: Vec<_> = (0..3).flat_map(|k| [(at_a + 3 + k, c[k]), (at_b + 3 + k, -c[k])]).collect();
                rows.push(row(size, &entries));
            }
            Constraint::Lock { a, b, .. } => {
                // The turns agree, and d/dt of t_b - t_a - R_a r is v_b - v_a - w_a x (t_b - t_a).
                let r = bodies[b].translation - bodies[a].translation;
                let (at_a, at_b) = (index[a] * 6, index[b] * 6);
                for k in 0..3 {
                    rows.push(row(size, &[(at_b + 3 + k, 1.0), (at_a + 3 + k, -1.0)]));
                    let mut entries = vec![(at_b + k, 1.0), (at_a + k, -1.0)];
                    for (i, e) in [DVec3::X, DVec3::Y, DVec3::Z].into_iter().enumerate() {
                        entries.push((at_a + 3 + i, -e.cross(r)[k]));
                    }
                    rows.push(row(size, &entries));
                }
            }
        }
    }
    Ok(rows)
}

fn rank(mut rows: Vec<Vec<f64>>) -> usize {
    let columns = rows.first().map_or(0, Vec::len);
    let mut rank = 0;
    for column in 0..columns {
        let Some(pivot) = (rank..rows.len()).max_by(|a, b| rows[*a][column].abs().total_cmp(&rows[*b][column].abs()))
        else {
            break;
        };
        if rows[pivot][column].abs() <= RANK_TOLERANCE {
            continue;
        }
        rows.swap(rank, pivot);
        let top = rows[rank].clone();
        for r in rows.iter_mut().skip(rank + 1) {
            let factor = r[column] / top[column];
            if factor != 0.0 {
                r.iter_mut().zip(&top).for_each(|(v, t)| *v -= factor * t);
            }
        }
        rank += 1;
    }
    rank
}

const MOTIONS: [&str; 6] = ["translate_x", "translate_y", "translate_z", "spin_x", "spin_y", "spin_z"];

/// How many motions remain of the mated `bodies` at their placements, and those one
/// body can make alone.
pub(crate) fn freedoms(
    bodies: &BTreeMap<Body, Placement>,
    constraints: &[Constraint],
) -> Result<(usize, Vec<FreeMotion>), MateRefusal> {
    if bodies.len() > MAX_BODIES {
        return Err(MateRefusal::new(
            RefusalCode::TooLarge,
            format!("{} mated components; this version solves at most {MAX_BODIES}", bodies.len()),
        ));
    }
    let index: BTreeMap<Body, usize> = bodies.keys().cloned().zip(0..).collect();
    let rows = jacobian(&index, bodies, constraints)?;
    let mut free = Vec::new();
    for (body, at) in &index {
        for (k, motion) in MOTIONS.iter().enumerate() {
            let column = at * 6 + k;
            if rows.iter().all(|r| r[column] == 0.0) {
                free.push(FreeMotion::of(body, motion));
            }
        }
    }
    Ok((index.len() * 6 - rank(rows), free))
}

/// Names a motion for people.
pub(crate) fn describe(m: &FreeMotion) -> String {
    let (kind, axis) = m.motion.split_once('_').unwrap_or((&m.motion, ""));
    let axis = axis.to_uppercase();
    let who = match &m.path {
        Some(path) => format!("occurrence {path}"),
        None => format!("component {}", m.component),
    };
    match kind {
        "translate" => format!("{who} translation along {axis}"),
        _ => format!("{who} spin about {axis}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(id: u32) -> Body {
        OccurrencePath::root().child(crate::path::Step::component(ComponentId(id)))
    }

    fn a() -> Body {
        body(2)
    }

    fn b() -> Body {
        body(4)
    }

    fn q(n: i64) -> BigRational {
        BigRational::from_integer(BigInt::from(n))
    }

    fn plane(normal: Normal, offset: i64) -> LocalPlane {
        LocalPlane { normal, offset: q(offset) }
    }

    fn distance(mate: &str, pa: LocalPlane, pb: LocalPlane, sign: i8, d: f64) -> Constraint {
        let along = Some(Along::Distance(d));
        Constraint::Parallel { mate: mate.into(), a: a(), b: b(), plane_a: pa, plane_b: pb, sign, along }
    }

    fn x() -> Constraint {
        distance("X", plane([1, 0, 0], 20), plane([1, 0, 0], 10), 1, 0.0)
    }

    fn y() -> Constraint {
        distance("Y", plane([0, 1, 0], 15), plane([0, 1, 0], 5), 1, 0.0)
    }

    fn z() -> Constraint {
        distance("Z", plane([0, 0, 1], 10), plane([0, 0, -1], 0), -1, 5.0)
    }

    fn origin(axes: bool) -> Constraint {
        Constraint::Origin { mate: "O".into(), body: a(), axes, at: Placement::IDENTITY }
    }

    fn bodies() -> BTreeMap<Body, Placement> {
        [(a(), Placement::IDENTITY), (b(), Placement::translation(DVec3::new(10.0, 10.0, 30.0)))].into()
    }

    #[test]
    fn freedoms_match_the_hand_ranks() {
        let at = bodies();
        for (constraints, expected) in [
            (vec![], 12),
            (vec![origin(true)], 6),
            (vec![origin(true), z()], 3),
            (vec![origin(true), x()], 3),
            (vec![origin(true), x(), y()], 1),
            (vec![origin(true), x(), y(), z()], 0),
            (vec![x(), y(), z()], 6),
            (vec![origin(false), x(), y(), z()], 3),
        ] {
            let solved = solve(&at, &constraints).unwrap();
            assert_eq!(freedoms(&solved, &constraints).unwrap().0, expected, "{constraints:?}");
        }
    }

    #[test]
    fn the_free_motions_of_one_body_are_named() {
        let constraints = [origin(true), z()];
        let solved = solve(&bodies(), &constraints).unwrap();
        let (_, free) = freedoms(&solved, &constraints).unwrap();
        let names: Vec<_> = free.iter().map(|f| (f.component, f.motion.as_str())).collect();
        let b = ComponentId(4);
        assert_eq!(names, [(b, "translate_x"), (b, "translate_y"), (b, "spin_z")]);
        assert!(free.iter().all(|f| f.path.is_none()), "the root's own components carry no path");
    }

    #[test]
    fn a_literal_mate_places_the_second_part_exactly() {
        let solved = solve(&bodies(), &[origin(true), x(), y(), z()]).unwrap();
        assert_eq!(solved[&a()], Placement::IDENTITY);
        assert_eq!(solved[&b()], Placement::translation(DVec3::new(10.0, 10.0, 15.0)));
    }

    #[test]
    fn a_partial_mate_keeps_authored_coordinates() {
        let solved = solve(&bodies(), &[origin(true), x()]).unwrap();
        assert_eq!(solved[&b()], Placement::translation(DVec3::new(10.0, 10.0, 30.0)));
        let mut at = bodies();
        at.get_mut(&b()).unwrap().translation = DVec3::new(1.0, 2.0, 3.0);
        let solved = solve(&at, &[origin(true), x()]).unwrap();
        assert_eq!(solved[&b()].translation, DVec3::new(10.0, 2.0, 3.0));
    }

    #[test]
    fn a_half_turn_comes_out_exact() {
        let constraints = [
            origin(true),
            x(),
            distance("Y", plane([0, 1, 0], 15), plane([0, 1, 0], 5), -1, 0.0),
            distance("Z", plane([0, 0, 1], 14), plane([0, 0, -1], 0), 1, 12.0),
        ];
        let solved = solve(&bodies(), &constraints).unwrap();
        assert_eq!(solved[&b()].rotation.to_array(), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(solved[&b()].translation, DVec3::new(10.0, 20.0, 26.0));
        assert_eq!(freedoms(&solved, &constraints).unwrap().0, 0);
    }

    #[test]
    fn disagreeing_mates_are_named_with_what_they_ask() {
        let two = distance("Conflict", plane([0, 0, 1], 10), plane([0, 0, -1], 0), -1, 6.0);
        let refused = solve(&bodies(), &[origin(true), x(), y(), z(), two]).unwrap_err();
        assert_eq!(refused.code, RefusalCode::OverConstrained);
        assert_eq!(refused.mates, ["Conflict", "Z"]);
        assert!(refused.message.contains("Z (5 mm)") && refused.message.contains("Conflict (6 mm)"), "{refused}");
        let flipped = distance("Z", plane([0, 0, 1], 10), plane([0, 0, -1], 0), 1, 5.0);
        assert_eq!(
            solve(&bodies(), &[origin(true), x(), y(), flipped]).unwrap_err().code,
            RefusalCode::OverConstrained
        );
    }

    #[test]
    fn a_redundant_mate_that_agrees_is_accepted() {
        let again = distance("Again", plane([0, 0, 1], 10), plane([0, 0, -1], 0), -1, 5.0);
        let constraints = [origin(true), x(), y(), z(), again];
        let solved = solve(&bodies(), &constraints).unwrap();
        assert_eq!(freedoms(&solved, &constraints).unwrap().0, 0);
    }

    #[test]
    fn an_exact_offset_survives_the_solve() {
        let plate = LocalPlane { normal: [0, 0, 1], offset: BigRational::new(BigInt::from(381), BigInt::from(5)) };
        let constraints = [origin(true), distance("Z", plate, plane([0, 0, -1], 0), -1, 5.0)];
        let solved = solve(&bodies(), &constraints).unwrap();
        assert_eq!(solved[&b()].translation.z, 81.2);
        assert_eq!(
            rational(&ExactRational { numerator: "381".into(), denominator: "5".into() }).map(|r| text(&r)),
            Some("381/5".into())
        );
    }

    fn parallel(mate: &str, pa: LocalPlane, pb: LocalPlane, sign: i8) -> Constraint {
        Constraint::Parallel { mate: mate.into(), a: a(), b: b(), plane_a: pa, plane_b: pb, sign, along: None }
    }

    fn perpendicular(mate: &str, pa: LocalPlane, pb: LocalPlane) -> Constraint {
        Constraint::Perpendicular { mate: mate.into(), a: a(), b: b(), plane_a: pa, plane_b: pb }
    }

    fn lock(relative: Placement) -> Constraint {
        Constraint::Lock { mate: "L".into(), a: a(), b: b(), relative }
    }

    /// B's groove from x = -10 to x = 10, A's tab from x = 2 to x = 6 in its own frame.
    fn width(mode: WidthMode) -> Constraint {
        let along = Some(Along::Width { groove: plane([-1, 0, 0], 10), tab: plane([-1, 0, 0], -2), mode });
        Constraint::Parallel {
            mate: "W".into(),
            a: a(),
            b: b(),
            plane_a: plane([1, 0, 0], 10),
            plane_b: plane([1, 0, 0], 6),
            sign: 1,
            along,
        }
    }

    fn q90z() -> DQuat {
        DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2)
    }

    #[test]
    fn parallel_turns_and_leaves_every_translation() {
        let mut at = bodies();
        at.get_mut(&b()).unwrap().rotation = DQuat::from_rotation_x(std::f64::consts::PI);
        let constraints = [origin(true), parallel("P", plane([0, 0, 1], 10), plane([0, 0, -1], 0), -1)];
        let solved = solve(&at, &constraints).unwrap();
        assert_eq!(snapped(solved[&b()].rotation * DVec3::NEG_Z), Some([0, 0, -1]), "the underside looks down");
        assert_eq!(solved[&b()].translation, DVec3::new(10.0, 10.0, 30.0), "nothing moves along the normal");
        let (dof, free) = freedoms(&solved, &constraints).unwrap();
        assert_eq!(dof, 4);
        let names: Vec<_> = free.iter().map(|f| f.motion.as_str()).collect();
        assert_eq!(names, ["translate_x", "translate_y", "translate_z", "spin_z"]);
    }

    #[test]
    fn perpendicular_keeps_the_authored_turn_when_it_holds() {
        let constraints = [origin(true), perpendicular("R", plane([0, 0, 1], 10), plane([1, 0, 0], 10))];
        let solved = solve(&bodies(), &constraints).unwrap();
        assert_eq!(solved[&b()], bodies()[&b()], "x is already at right angles to z");
        assert_eq!(freedoms(&solved, &constraints).unwrap().0, 5);
    }

    #[test]
    fn perpendicular_takes_the_smallest_quarter_turn() {
        // B's top would have to stand at right angles to A's: a quarter turn, about the
        // first axis in order.
        let constraints = [origin(true), perpendicular("R", plane([0, 0, 1], 10), plane([0, 0, 1], 6))];
        let solved = solve(&bodies(), &constraints).unwrap();
        let turned = solved[&b()].rotation;
        assert_eq!(snapped(turned * DVec3::Z).map(|n| dot(n, [0, 0, 1])), Some(0));
        assert!((turned.angle_between(DQuat::IDENTITY) - std::f64::consts::FRAC_PI_2).abs() < 1e-12, "{turned}");
        assert!(cube_of(turned).is_some());
        assert_eq!(freedoms(&solved, &constraints).unwrap().0, 5);
    }

    #[test]
    fn perpendicular_and_parallel_together_pick_a_turn_that_keeps_both() {
        // B's z parallel to A's x and B's x at right angles to A's y: B's x goes along z.
        let constraints = [
            origin(true),
            parallel("P", plane([1, 0, 0], 0), plane([0, 0, 1], 0), 1),
            perpendicular("R", plane([0, 1, 0], 0), plane([1, 0, 0], 0)),
        ];
        let solved = solve(&bodies(), &constraints).unwrap();
        let r = solved[&b()].rotation;
        assert_eq!(snapped(r * DVec3::Z), Some([1, 0, 0]));
        assert_eq!(snapped(r * DVec3::X).map(|n| n[1]), Some(0));
        assert_eq!(freedoms(&solved, &constraints).unwrap().0, 3, "every turn is held, no translation");
    }

    #[test]
    fn perpendicular_against_a_parallel_is_named() {
        let constraints = [
            origin(true),
            parallel("P", plane([0, 0, 1], 0), plane([0, 0, 1], 0), 1),
            perpendicular("R", plane([0, 0, 1], 0), plane([0, 0, 1], 0)),
        ];
        let refused = solve(&bodies(), &constraints).unwrap_err();
        assert_eq!(refused.code, RefusalCode::OverConstrained);
        assert_eq!(refused.mates, ["P", "R"]);
    }

    #[test]
    fn a_lock_holds_every_motion_and_keeps_its_offset() {
        let relative = Placement::new(q90z(), DVec3::new(1.0, 2.0, 3.0));
        let constraints = [origin(true), lock(relative)];
        let solved = solve(&bodies(), &constraints).unwrap();
        let placed = solved[&b()];
        assert!(placed.rotation.angle_between(q90z()) < 1e-12);
        assert_eq!(placed.translation, DVec3::new(1.0, 2.0, 3.0));
        assert_eq!(freedoms(&solved, &constraints).unwrap().0, 0);
        // Without the origin the pair moves as one body.
        let solved = solve(&bodies(), &[lock(relative)]).unwrap();
        assert_eq!(freedoms(&solved, &[lock(relative)]).unwrap().0, 6);
    }

    #[test]
    fn a_lock_follows_a_quarter_turned_first_part_exactly() {
        let at = Placement::new(q90z(), DVec3::new(5.0, 0.0, 0.0));
        let frame = Constraint::Origin { mate: "O".into(), body: a(), axes: true, at };
        let constraints = [frame, lock(Placement::translation(DVec3::new(1.0, 2.0, 3.0)))];
        let solved = solve(&bodies(), &constraints).unwrap();
        // A's x is world y, so B's offset [1, 2, 3] lands at [5 - 2, 1, 3].
        assert_eq!(solved[&b()].translation, DVec3::new(3.0, 1.0, 3.0));
    }

    #[test]
    fn a_lock_on_a_part_turned_off_the_quarter_turns_is_refused() {
        let at = Placement::new(DQuat::from_rotation_z(0.5), DVec3::ZERO);
        let frame = Constraint::Origin { mate: "O".into(), body: a(), axes: true, at };
        let refused = solve(&bodies(), &[frame, lock(Placement::IDENTITY)]).unwrap_err();
        assert_eq!((refused.code, refused.mates), (RefusalCode::UnsupportedPlacement, vec!["L".to_owned()]));
    }

    #[test]
    fn a_lock_against_a_distance_names_both() {
        let constraints = [origin(true), lock(Placement::translation(DVec3::new(0.0, 0.0, 20.0))), z()];
        let refused = solve(&bodies(), &constraints).unwrap_err();
        assert_eq!(refused.code, RefusalCode::OverConstrained);
        assert_eq!(refused.mates, ["L", "Z"]);
    }

    #[test]
    fn width_places_the_tab_exactly() {
        // Groove -10..10 on A, tab 2..6 on B, play 16. The first groove face is the one at
        // 10, so a gap from it puts the tab's top 1 below it, and a quarter of the play 4.
        for (mode, x) in [
            (WidthMode::Centered, -4.0),
            (WidthMode::Gap { distance: 1.0, flipped: false }, 3.0),
            (WidthMode::Gap { distance: 1.0, flipped: true }, -11.0),
            (WidthMode::Share { fraction: BigRational::new(BigInt::from(1), BigInt::from(4)), flipped: false }, 0.0),
            (WidthMode::Share { fraction: BigRational::new(BigInt::from(1), BigInt::from(4)), flipped: true }, -8.0),
        ] {
            let constraints = [origin(true), width(mode.clone())];
            let solved = solve(&bodies(), &constraints).unwrap();
            assert_eq!(solved[&b()].translation, DVec3::new(x, 10.0, 30.0), "{mode:?}");
            assert_eq!(freedoms(&solved, &constraints).unwrap().0, 3, "{mode:?}");
        }
    }

    #[test]
    fn a_width_against_a_distance_names_both() {
        let constraints = [origin(true), width(WidthMode::Centered), x()];
        let refused = solve(&bodies(), &constraints).unwrap_err();
        assert_eq!(refused.code, RefusalCode::OverConstrained);
        assert_eq!(refused.mates, ["W", "X"]);
    }

    #[test]
    fn the_cube_rotations_are_the_twenty_four() {
        let all = cube_rotations();
        assert_eq!(all.len(), 24);
        for m in &all {
            assert_eq!(cube_of(quaternion(m)), Some(*m));
        }
        assert_eq!(cube_of(DQuat::from_rotation_z(0.5)), None);
    }
}
