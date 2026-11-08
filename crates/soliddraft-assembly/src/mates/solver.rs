//! Placing components from proven planes, and counting what stays free.
//!
//! Plane normals are exact signed axes here, which splits the problem in two:
//!
//! 1. **Rotations.** Each mate between planes relates two normals: equal, opposite or at
//!    right angles. An Origin mate with axes fixes a rotation outright. Locked bodies turn
//!    as one group. Every turn this solver can prove takes the normals it reads onto axes,
//!    so the groups' turns are found together by a search over the 24 quarter-turn
//!    rotations; a body held along a single axis may also keep its own spin. Candidates
//!    nearest the authored turn come first, so the answer does not depend on mate order,
//!    and a refusal names a smallest set of mates that cannot hold together. A body whose
//!    axes or points a mate places takes quarter turns only, so those stay rational.
//! 2. **Translations.** With rotations known, each distance, width and lock is a difference
//!    of one world coordinate between two bodies, and so is each coordinate an axis or a
//!    point fixes across its direction, so each axis is a graph of exact differences. Walking it places what is connected to the assembly's origin, or to the
//!    graph's first body, and a closing edge that disagrees is an over-constrained set of
//!    mates.
//!
//! The two steps are not independent: which axis a distance's planes face decides which
//! coordinate it moves. So the search accepts a set of turns only when the translations
//! they leave can hold, checking each mate's translation as soon as its turns are chosen,
//! and the assembly is over-constrained only when no supported turns work. Turns that take
//! a group's read normals onto the same axes decide the same, so the search tries one of
//! them only. A group no mate between planes turns may still need a turn when a lock in it
//! places a part along an axis, so such a group tries every quarter turn after its
//! authored one.
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
use crate::placement::{Placement, canonical};

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

/// A line in its part's frame: an axis-aligned direction through a rational point.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LocalAxis {
    /// The direction, a signed axis.
    pub direction: Normal,
    /// A point on the line.
    pub through: [BigRational; 3],
}

/// A point in its part's frame.
pub(crate) type LocalPoint = [BigRational; 3];

/// What fixes the translation between two parallel planes.
#[allow(clippy::large_enum_variant, reason = "a solve holds a few of these, read one by one")]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Along {
    /// The signed distance from `a`'s plane to `b`'s, along `a`'s outward normal.
    Distance(f64),
    /// That signed distance anywhere from the first value to the second.
    Range(f64, f64),
    /// A tab between a groove's faces: `plane_a` and `groove` are the groove's, on `a`;
    /// `plane_b` and `tab` the tab's, on `b`.
    Width { groove: LocalPlane, tab: LocalPlane, mode: WidthMode },
}

/// What a Symmetric mirrors about its plane: two planes, two lines or two points.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Mirrored {
    Planes(LocalPlane, LocalPlane),
    Axes(LocalAxis, LocalAxis),
    Points(LocalPoint, LocalPoint),
}

/// One side of a coupler: how far an entity of `body` lies along its own direction,
/// measured from the origin of `from`, or from `frame` when there is no reference part.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Slide {
    pub body: Body,
    /// The plane's outward normal or the axis's direction, in the part's frame.
    pub direction: Normal,
    /// How far the entity lies along that direction from the part's origin.
    pub offset: BigRational,
    /// The reference part, when there is one.
    pub from: Option<Body>,
    /// The origin of the frame the mate is held in, when there is no reference part.
    pub frame: [BigRational; 3],
}

/// A straight slot in its part's frame: the direction its round ends' axes run, and a point
/// on each end's axis, first end first.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LocalSlot {
    pub depth: Normal,
    pub ends: [LocalPoint; 2],
}

/// Where a Slot puts the pin along the slot.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SlotMode {
    /// Anywhere between the ends' centres.
    Free,
    /// Halfway between them.
    Centre,
    /// This far from the first end's centre towards the second, or from the second when
    /// `flipped`.
    Distance { distance: BigRational, flipped: bool },
    /// This share of the way from the first end's centre, or from the second when `flipped`.
    Share { fraction: BigRational, flipped: bool },
}

/// Where a width puts the tab.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WidthMode {
    /// Anywhere between the groove's faces.
    Free,
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
#[allow(clippy::large_enum_variant, reason = "a solve holds one per mate, read one by one")]
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
    /// Two axes: their directions equal (`sign` 1), opposite (-1) or at right angles (0).
    /// `coaxial` also puts `b`'s line on `a`'s.
    Axes { mate: String, a: Body, b: Body, axis_a: LocalAxis, axis_b: LocalAxis, sign: i8, coaxial: bool },
    /// `b`'s point on `a`'s line.
    OnAxis { mate: String, a: Body, axis: LocalAxis, b: Body, point: LocalPoint },
    /// `b`'s point on `a`'s.
    Points { mate: String, a: Body, point_a: LocalPoint, b: Body, point_b: LocalPoint },
    /// `b`'s point at `gap` from `a`'s plane, along the plane's outward normal, or anywhere
    /// from `gap` to `upto` when that is given. When `b` reads a line through the point,
    /// `direction` is the line's, and it lies parallel to the plane.
    FromPlane {
        mate: String,
        a: Body,
        plane: LocalPlane,
        b: Body,
        point: LocalPoint,
        direction: Option<Normal>,
        gap: BigRational,
        upto: Option<BigRational>,
        /// What the mate asks for, for a message.
        says: String,
    },
    /// `b`'s turn is `a`'s composed with `relative`; translations stay free.
    Turn { mate: String, a: Body, b: Body, relative: DQuat },
    /// `b`'s entity is `a`'s mirrored about `s`'s plane. Between planes or lines `sign`
    /// relates `b`'s normal or direction to the mirror image of `a`'s.
    Symmetric { mate: String, a: Body, b: Body, s: Body, plane: LocalPlane, ends: Mirrored, sign: i8 },
    /// `ratio[0]` times how far `b` slides, less `ratio[1]` times how far `a` does (plus, when
    /// `reverse`), is `offset`.
    Coupler { mate: String, a: Slide, b: Slide, ratio: [BigRational; 2], reverse: bool, offset: BigRational },
    /// `b`'s line, a pin, in `a`'s straight slot: parallel to the ends' axes by `sign`, on the
    /// slot's mid-plane, and along it as `mode` says.
    Slot { mate: String, a: Body, slot: LocalSlot, b: Body, axis: LocalAxis, sign: i8, mode: SlotMode },
    /// `b`'s profile turned so its edges lie along `a`'s, about `a`'s plane normal `normal`.
    /// `edges` holds each profile's edge directions in its part's frame; the status, which
    /// reads no part, has none and counts the spin only.
    Spin { mate: String, a: Body, b: Body, normal: Normal, edges: Option<(Vec<DVec3>, Vec<DVec3>)> },
}

impl Constraint {
    fn mate(&self) -> &str {
        match self {
            Constraint::Origin { mate, .. }
            | Constraint::Parallel { mate, .. }
            | Constraint::Perpendicular { mate, .. }
            | Constraint::Lock { mate, .. }
            | Constraint::Axes { mate, .. }
            | Constraint::OnAxis { mate, .. }
            | Constraint::Points { mate, .. }
            | Constraint::FromPlane { mate, .. }
            | Constraint::Turn { mate, .. }
            | Constraint::Symmetric { mate, .. }
            | Constraint::Coupler { mate, .. }
            | Constraint::Slot { mate, .. }
            | Constraint::Spin { mate, .. } => mate,
        }
    }

    /// The bodies a constraint between bodies ties.
    fn tied(&self) -> Vec<&Body> {
        match self {
            Constraint::Parallel { a, b, .. }
            | Constraint::Perpendicular { a, b, .. }
            | Constraint::Lock { a, b, .. }
            | Constraint::Axes { a, b, .. }
            | Constraint::OnAxis { a, b, .. }
            | Constraint::Points { a, b, .. }
            | Constraint::FromPlane { a, b, .. }
            | Constraint::Turn { a, b, .. }
            | Constraint::Slot { a, b, .. }
            | Constraint::Spin { a, b, .. } => vec![a, b],
            Constraint::Symmetric { a, b, s, .. } => vec![a, b, s],
            Constraint::Coupler { a, b, .. } => [&a.body, &b.body].into_iter().chain(&a.from).chain(&b.from).collect(),
            Constraint::Origin { .. } => Vec::new(),
        }
    }

    /// The bodies whose axes or points it places: those take quarter turns only, so the
    /// coordinates they read stay rational.
    fn positioned(&self) -> Vec<&Body> {
        match self {
            Constraint::Axes { a, b, coaxial: true, .. }
            | Constraint::OnAxis { a, b, .. }
            | Constraint::Points { a, b, .. }
            | Constraint::Slot { a, b, .. }
            | Constraint::Symmetric { a, b, ends: Mirrored::Axes(..) | Mirrored::Points(..), .. } => vec![a, b],
            Constraint::FromPlane { b, .. } => vec![b],
            _ => Vec::new(),
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

/// The signed axis `rotation` takes the axis `n` to, exactly, or `None`.
///
/// A placement stores a binary64 quaternion, and each component is read as the rational
/// it is. Its turn takes `v` to `((w² - b·b) v + 2 (b·v) b + 2 w (b × v)) / (w² + b·b)`,
/// with `b` its vector part, and that is compared with the axis in rationals. A turn that
/// lands within a tolerance of an axis is not on it: far from the origin a tilt of 2^-42
/// opens a real gap. Every quarter turn's quaternion has nonzero parts of one magnitude,
/// and a spin about an axis has its vector part on that axis, so both are exact however
/// their parts round.
///
/// A solve asks this of the same few dozen turns many times over, so each answer is kept,
/// by the quaternion's bits and the axis, for the rest of the thread's solves.
pub(crate) fn turned(rotation: DQuat, n: Normal) -> Option<Normal> {
    /// How many answers a thread keeps before it starts again.
    const KEPT: usize = 4096;
    type Key = ([u64; 4], Normal);
    thread_local! {
        static KNOWN: std::cell::RefCell<std::collections::HashMap<Key, Option<Normal>>> = Default::default();
    }
    let axis = snapped(rotation * vector(n))?;
    let key = (rotation.to_array().map(f64::to_bits), n);
    if let Some(known) = KNOWN.with(|k| k.borrow().get(&key).copied()) {
        return known;
    }
    let answer = exactly_turned(rotation, n, axis);
    KNOWN.with(|k| {
        let mut k = k.borrow_mut();
        if k.len() >= KEPT {
            k.clear();
        }
        k.insert(key, answer);
    });
    answer
}

/// Whether `rotation` takes `n` exactly onto `axis`, in rationals: [`turned`]'s proof.
fn exactly_turned(rotation: DQuat, n: Normal, axis: Normal) -> Option<Normal> {
    let [x, y, z, w] = [rotation.x, rotation.y, rotation.z, rotation.w].map(exact);
    let (x, y, z, w) = (x?, y?, z?, w?);
    let int = |c: i8| BigRational::from_integer(BigInt::from(c));
    let v = n.map(int);
    let b = [x, y, z];
    let bb = &b[0] * &b[0] + &b[1] * &b[1] + &b[2] * &b[2];
    let ww = &w * &w;
    let bv = &b[0] * &v[0] + &b[1] * &v[1] + &b[2] * &v[2];
    let cross = [&b[1] * &v[2] - &b[2] * &v[1], &b[2] * &v[0] - &b[0] * &v[2], &b[0] * &v[1] - &b[1] * &v[0]];
    let two = int(2);
    let scale = &ww + &bb;
    let along = &ww - &bb;
    (0..3).all(|i| &along * &v[i] + &two * &bv * &b[i] + &two * &w * &cross[i] == &scale * int(axis[i])).then_some(axis)
}

fn world(rotation: DQuat, n: Normal, mate: &str) -> Result<Normal, MateRefusal> {
    turned(rotation, n).ok_or_else(|| {
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

fn conflict(mates: Vec<String>, message: String) -> MateRefusal {
    MateRefusal { mates, ..MateRefusal::new(RefusalCode::OverConstrained, message) }
}

/// A rotation as the integer matrix it is when it takes axes to axes.
pub(crate) type Cube = [[i8; 3]; 3];

/// The 24 rotations that take axes to axes, in a fixed order.
pub(crate) fn cube_rotations() -> Vec<Cube> {
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

pub(crate) fn quaternion(m: &Cube) -> DQuat {
    let col = |c: usize| DVec3::new(m[0][c] as f64, m[1][c] as f64, m[2][c] as f64);
    DQuat::from_mat3(&DMat3::from_cols(col(0), col(1), col(2)))
}

/// The rotation's integer matrix, when it takes every axis to an axis.
pub(crate) fn cube_of(rotation: DQuat) -> Option<Cube> {
    let mut m = [[0i8; 3]; 3];
    for (c, axis) in [[1, 0, 0], [0, 1, 0], [0, 0, 1]].into_iter().enumerate() {
        let n = turned(rotation, axis)?;
        for r in 0..3 {
            m[r][c] = n[r];
        }
    }
    Some(m)
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

/// The turn nearest `authored` that takes the axis `u` exactly onto the axis `v`, keeping
/// the spin about `v` that `authored` has: the quarter turn `c` taking `u` to `v`, then
/// the turn about `v` nearest `authored` once `c` is undone. A turn about `v` has its
/// vector part on `v` alone, so `u` lands on `v` exactly however the parts round; a
/// shortest arc from where `authored` puts `u` would leave residuals off it. `None` if
/// rounding still leaves it off.
fn spun(authored: DQuat, u: Normal, v: Normal) -> Option<DQuat> {
    let c = canonical(shortest_arc(vector(u), vector(v)));
    let rest = authored * c.inverse();
    let k = v.iter().position(|x| *x != 0)?;
    let mut parts = [0.0; 4];
    parts[k] = rest.xyz()[k];
    parts[3] = rest.w;
    let about = DQuat::from_array(parts);
    if about.length() == 0.0 {
        return None;
    }
    let q = canonical((about.normalize() * c).normalize());
    (turned(q, u) == Some(v)).then_some(q)
}

/// The names of every mate between `a` and `b`, sorted.
fn between(constraints: &[Constraint], a: &Body, b: &Body) -> Vec<String> {
    let mut names: Vec<String> = constraints
        .iter()
        .filter(|c| {
            let tied = c.tied();
            tied.contains(&a) && tied.contains(&b)
        })
        .map(|c| c.mate().to_owned())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn incompatible(names: Vec<String>) -> MateRefusal {
    let message = format!("{}: their faces would need incompatible directions", names.join(", "));
    conflict(names, message)
}

/// Each body's lock group: the group's first body, its root, and the turn `offset` with
/// `rotation(body) = rotation(root) * offset`. A body no lock reaches is a group of its own.
type Groups = BTreeMap<Body, (Body, DQuat)>;

/// The rigid groups locks make.
fn lock_groups(bodies: &BTreeMap<Body, Placement>, constraints: &[Constraint]) -> Groups {
    let mut group = Groups::new();
    for root in bodies.keys() {
        if group.contains_key(root) {
            continue;
        }
        group.insert(root.clone(), (root.clone(), DQuat::IDENTITY));
        let mut queue = VecDeque::from([root.clone()]);
        while let Some(body) = queue.pop_front() {
            let offset = group[&body].1;
            for c in constraints {
                // A lock says rotation(b) = rotation(a) * relative, and so does a turn.
                let (a, b, relative) = match c {
                    Constraint::Lock { a, b, relative, .. } => (a, b, relative.rotation),
                    Constraint::Turn { a, b, relative, .. } => (a, b, *relative),
                    _ => continue,
                };
                let next = if *a == body {
                    Some((b, offset * relative))
                } else if *b == body {
                    Some((a, offset * relative.inverse()))
                } else {
                    None
                };
                if let Some((other, turn)) = next
                    && !group.contains_key(other)
                {
                    group.insert(other.clone(), (root.clone(), turn.normalize()));
                    queue.push_back(other.clone());
                }
            }
        }
    }
    group
}

/// The three axes: a group that reads all of them is decided by its whole turn.
const ALL: [Normal; 3] = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];

/// The most candidate turns one solve may try before it refuses.
const SEARCH_STEPS: usize = 200_000;

/// What one mate asks of the turns of the lock groups it ties, by group index. Normals are
/// in each group's root frame.
#[derive(Clone)]
enum Rule {
    /// Group `a`'s normal `u` and group `b`'s `w`: equal when `sign` is 1, opposite when
    /// -1, at right angles when 0.
    Relate { a: usize, u: Normal, b: usize, w: Normal, sign: i8 },
    /// Group `g`'s root takes `rotation`.
    Fixed { g: usize, rotation: DQuat },
    /// Group `b`'s normal `w` is `sign` times group `a`'s `u` mirrored across group `s`'s
    /// `m`.
    Mirror { a: usize, u: Normal, b: usize, w: Normal, s: usize, m: Normal, sign: i8 },
    /// One of group `b`'s edge directions lies along one of group `a`'s, each in its
    /// group's root frame.
    Spin { a: usize, ea: Vec<DVec3>, b: usize, eb: Vec<DVec3> },
}

/// `n` mirrored across a plane whose normal is the signed axis `m`.
fn mirror(n: Normal, m: Normal) -> Normal {
    let d = dot(n, m);
    std::array::from_fn(|k| n[k] - 2 * d * m[k])
}

/// How far apart two directions may turn and still be read as lying along each other: a
/// profile's edges come from sketched corners, which a polygon's turns round.
const EDGE_TOLERANCE: f64 = 1e-9;

/// Whether some direction of `a` lies along some direction of `b`, either way.
fn along_any(a: &[DVec3], b: &[DVec3]) -> bool {
    a.iter().any(|x| b.iter().any(|y| x.normalize().cross(y.normalize()).length() <= EDGE_TOLERANCE))
}

impl Rule {
    fn groups(&self) -> Vec<usize> {
        match self {
            Rule::Relate { a, b, .. } | Rule::Spin { a, b, .. } => vec![*a, *b],
            Rule::Fixed { g, .. } => vec![*g],
            Rule::Mirror { a, b, s, .. } => vec![*a, *b, *s],
        }
    }

    /// Whether the rule holds on the turns given so far; `None` while one is open.
    fn holds(&self, at: &[Option<DQuat>]) -> Option<bool> {
        match self {
            Rule::Relate { a, u, b, w, sign } => {
                let (ra, rb) = (at[*a]?, at[*b]?);
                Some(match (turned(ra, *u), turned(rb, *w)) {
                    (Some(na), Some(nb)) if *sign == 0 => dot(na, nb) == 0,
                    (Some(na), Some(nb)) => nb == na.map(|c| c * sign),
                    _ => false,
                })
            }
            Rule::Fixed { g, rotation } => Some(same_turn(at[*g]?, *rotation)),
            Rule::Mirror { a, u, b, w, s, m, sign } => {
                let (ra, rb, rs) = (at[*a]?, at[*b]?, at[*s]?);
                Some(match (turned(ra, *u), turned(rb, *w), turned(rs, *m)) {
                    (Some(na), Some(nb), Some(nm)) => nb == mirror(na, nm).map(|c| c * sign),
                    _ => false,
                })
            }
            Rule::Spin { a, ea, b, eb } => {
                let (ra, rb) = (at[*a]?, at[*b]?);
                let turn = |q: DQuat, e: &[DVec3]| e.iter().map(|v| q * *v).collect::<Vec<_>>();
                Some(along_any(&turn(ra, ea), &turn(rb, eb)))
            }
        }
    }
}

fn same_turn(p: DQuat, q: DQuat) -> bool {
    [DVec3::X, DVec3::Y].iter().all(|v| (p * *v).distance(q * *v) <= AXIS_TOLERANCE)
}

/// The search over the groups' turns. Every turn the exact solver can prove takes the
/// normals it reads onto axes, so each group's candidates are the 24 quarter-turn
/// rotations; one unlocked part held along a single axis may also keep its own spin.
///
/// A turn is accepted only when the translations it leaves can hold too: a distance whose
/// planes face one way may be impossible and the same distance facing another way exact.
struct Search<'a> {
    bodies: &'a BTreeMap<Body, Placement>,
    constraints: &'a [Constraint],
    /// Each body's group, and the turn `offset` with `rotation(body) = rotation(group) *
    /// offset`.
    member: BTreeMap<Body, (usize, DQuat)>,
    authored: Vec<DQuat>,
    locked: Vec<bool>,
    /// Groups whose axes or points a mate places: quarter turns only.
    positioned: Vec<bool>,
    /// For each constraint that measures from a plane without relating a normal of the
    /// other part to it, the plane's group and normal in its root frame. Which world axis
    /// that normal takes decides the translation, so the group's turn is searched for it.
    from_planes: Vec<Option<(usize, Normal)>>,
    /// Each rule with the mate that asks for it and the constraint it comes from.
    rules: Vec<(&'a str, usize, Rule)>,
    /// For each constraint, the groups whose turns decide the translations it asks for;
    /// `None` when it asks for none.
    placed_by: Vec<Option<Vec<usize>>>,
    /// Each group's normals, in its root frame, whose world directions decide everything
    /// its mates ask: two turns that take them onto the same axes are interchangeable.
    reads: Vec<Vec<Normal>>,
    cubes: Vec<DQuat>,
    steps: usize,
}

/// Why a search stopped before deciding.
enum Stop {
    /// It ran past [`SEARCH_STEPS`].
    TooLong,
    /// A mate cannot be read.
    Refused(MateRefusal),
}

impl From<MateRefusal> for Stop {
    fn from(refusal: MateRefusal) -> Stop {
        Stop::Refused(refusal)
    }
}

/// One search over some of the constraints.
struct Pass {
    /// Which constraints take part.
    active: Vec<bool>,
    domains: Vec<Vec<DQuat>>,
    /// Each group's active rules.
    touching: Vec<Vec<usize>>,
    /// Each group's active constraints that ask for translations, when a turn must also
    /// leave the translations solvable; empty otherwise.
    moves: Vec<Vec<usize>>,
}

impl<'a> Search<'a> {
    fn new(bodies: &'a BTreeMap<Body, Placement>, constraints: &'a [Constraint]) -> Result<Search<'a>, MateRefusal> {
        // Locked parts turn as one, so the unknowns are the lock groups' turns, solved
        // together as one finite search.
        let group = lock_groups(bodies, constraints);
        let roots: Vec<Body> = group.values().map(|(r, _)| r.clone()).collect::<BTreeSet<_>>().into_iter().collect();
        let index: BTreeMap<&Body, usize> = roots.iter().zip(0..).collect();
        let member: BTreeMap<Body, (usize, DQuat)> =
            group.iter().map(|(body, (root, offset))| (body.clone(), (index[root], *offset))).collect();
        let of = |body: &Body| member[body];
        let own = |offset: DQuat, n: Normal, mate: &str| turned(offset, n).ok_or_else(|| unsupported_lock(mate));
        let mut rules = Vec::new();
        let mut placed_by = Vec::new();
        let mut reads = vec![Vec::new(); roots.len()];
        let mut from_planes = vec![None; constraints.len()];
        for (i, c) in constraints.iter().enumerate() {
            match c {
                Constraint::Origin { mate, body, axes, at } => {
                    if *axes {
                        let (g, offset) = of(body);
                        let rotation = canonical((at.rotation * offset.inverse()).normalize());
                        rules.push((mate.as_str(), i, Rule::Fixed { g, rotation }));
                    }
                    placed_by.push(Some(Vec::new()));
                }
                Constraint::Parallel { mate, a, b, plane_a, plane_b, sign, along } => {
                    let ((ga, oa), (gb, ob)) = (of(a), of(b));
                    let (u, w) = (own(oa, plane_a.normal, mate)?, own(ob, plane_b.normal, mate)?);
                    rules.push((mate.as_str(), i, Rule::Relate { a: ga, u, b: gb, w, sign: *sign }));
                    reads[ga].push(u);
                    reads[gb].push(w);
                    if let Some(Along::Width { groove, tab, .. }) = along {
                        for (g, offset, n) in [(ga, oa, groove.normal), (gb, ob, tab.normal)] {
                            match turned(offset, n) {
                                Some(n) => reads[g].push(n),
                                None => reads[g].extend([[1, 0, 0], [0, 1, 0], [0, 0, 1]]),
                            }
                        }
                    }
                    placed_by.push(along.is_some().then(|| vec![ga, gb]));
                }
                Constraint::Perpendicular { mate, a, b, plane_a, plane_b } => {
                    let ((ga, oa), (gb, ob)) = (of(a), of(b));
                    let (u, w) = (own(oa, plane_a.normal, mate)?, own(ob, plane_b.normal, mate)?);
                    rules.push((mate.as_str(), i, Rule::Relate { a: ga, u, b: gb, w, sign: 0 }));
                    reads[ga].push(u);
                    reads[gb].push(w);
                    placed_by.push(None);
                }
                Constraint::Lock { a, .. } => {
                    // A lock's offset turns with its first part, whole.
                    let g = of(a).0;
                    reads[g].extend(ALL);
                    placed_by.push(Some(vec![g]));
                }
                Constraint::Axes { mate, a, b, axis_a, axis_b, sign, coaxial } => {
                    let ((ga, oa), (gb, ob)) = (of(a), of(b));
                    let (u, w) = (own(oa, axis_a.direction, mate)?, own(ob, axis_b.direction, mate)?);
                    rules.push((mate.as_str(), i, Rule::Relate { a: ga, u, b: gb, w, sign: *sign }));
                    if *coaxial {
                        // Each line's point turns with its part, whole.
                        reads[ga].extend(ALL);
                        reads[gb].extend(ALL);
                        placed_by.push(Some(vec![ga, gb]));
                    } else {
                        reads[ga].push(u);
                        reads[gb].push(w);
                        placed_by.push(None);
                    }
                }
                Constraint::OnAxis { a, b, .. } | Constraint::Points { a, b, .. } => {
                    let (ga, gb) = (of(a).0, of(b).0);
                    reads[ga].extend(ALL);
                    reads[gb].extend(ALL);
                    placed_by.push(Some(vec![ga, gb]));
                }
                Constraint::FromPlane { mate, a, plane, b, direction, .. } => {
                    let ((ga, oa), (gb, ob)) = (of(a), of(b));
                    let u = own(oa, plane.normal, mate)?;
                    match direction {
                        Some(d) => {
                            let w = own(ob, *d, mate)?;
                            rules.push((mate.as_str(), i, Rule::Relate { a: ga, u, b: gb, w, sign: 0 }));
                        }
                        None => from_planes[i] = Some((ga, u)),
                    }
                    reads[ga].push(u);
                    reads[gb].extend(ALL);
                    placed_by.push(Some(vec![ga, gb]));
                }
                Constraint::Turn { .. } => placed_by.push(None),
                Constraint::Symmetric { mate, a, b, s, plane, ends, sign } => {
                    let ((ga, oa), (gb, ob), (gs, os)) = (of(a), of(b), of(s));
                    let m = own(os, plane.normal, mate)?;
                    reads[gs].push(m);
                    let directions = match ends {
                        Mirrored::Planes(pa, pb) => Some((pa.normal, pb.normal)),
                        Mirrored::Axes(xa, xb) => Some((xa.direction, xb.direction)),
                        Mirrored::Points(..) => None,
                    };
                    match directions {
                        Some((na, nb)) => {
                            let (u, w) = (own(oa, na, mate)?, own(ob, nb, mate)?);
                            rules.push((mate.as_str(), i, Rule::Mirror { a: ga, u, b: gb, w, s: gs, m, sign: *sign }));
                            reads[ga].push(u);
                            reads[gb].push(w);
                        }
                        // Nothing turns the mirror for two points: only the translation does.
                        None => from_planes[i] = Some((gs, m)),
                    }
                    if !matches!(ends, Mirrored::Planes(..)) {
                        reads[ga].extend(ALL);
                        reads[gb].extend(ALL);
                    }
                    placed_by.push(Some(vec![ga, gb, gs]));
                }
                Constraint::Coupler { mate, a, b, .. } => {
                    let mut groups = Vec::new();
                    for side in [a, b] {
                        let (g, offset) = of(&side.body);
                        reads[g].push(own(offset, side.direction, mate)?);
                        groups.push(g);
                        groups.extend(side.from.as_ref().map(|f| of(f).0));
                    }
                    placed_by.push(Some(groups));
                }
                Constraint::Slot { mate, a, slot, b, axis, sign, .. } => {
                    let ((ga, oa), (gb, ob)) = (of(a), of(b));
                    let (u, w) = (own(oa, slot.depth, mate)?, own(ob, axis.direction, mate)?);
                    rules.push((mate.as_str(), i, Rule::Relate { a: ga, u, b: gb, w, sign: *sign }));
                    reads[ga].extend(ALL);
                    reads[gb].extend(ALL);
                    placed_by.push(Some(vec![ga, gb]));
                }
                Constraint::Spin { mate, a, b, edges, .. } => {
                    if let Some((ea, eb)) = edges {
                        let ((ga, oa), (gb, ob)) = (of(a), of(b));
                        let (ea, eb) = (ea.iter().map(|e| oa * *e).collect(), eb.iter().map(|e| ob * *e).collect());
                        rules.push((mate.as_str(), i, Rule::Spin { a: ga, ea, b: gb, eb }));
                        reads[ga].extend(ALL);
                        reads[gb].extend(ALL);
                    }
                    placed_by.push(None);
                }
            }
        }
        let positioned: BTreeSet<usize> =
            constraints.iter().flat_map(Constraint::positioned).map(|body| member[body].0).collect();
        for r in &mut reads {
            r.sort();
            r.dedup();
        }
        // A normal a fixed frame puts off the axes cannot be proved: refused as before.
        for (mate, _, rule) in &rules {
            if let Rule::Relate { a, u, b, w, .. } = rule {
                for (g, n) in [(*a, *u), (*b, *w)] {
                    let fixed = rules.iter().find_map(|(_, _, r)| match r {
                        Rule::Fixed { g: f, rotation } if *f == g => Some(*rotation),
                        _ => None,
                    });
                    if let Some(rotation) = fixed {
                        world(rotation, n, mate)?;
                    }
                }
            }
        }
        Ok(Search {
            bodies,
            constraints,
            authored: roots.iter().map(|r| bodies[r].rotation).collect(),
            locked: roots.iter().map(|r| group.values().filter(|(x, _)| x == r).count() > 1).collect(),
            positioned: (0..roots.len()).map(|g| positioned.contains(&g)).collect(),
            from_planes,
            member,
            rules,
            placed_by,
            reads,
            cubes: cube_rotations().iter().map(quaternion).collect(),
            steps: 0,
        })
    }

    /// Group `g`'s candidate turns under `active` rules, nearest its authored turn first
    /// and in a fixed order among equals. `locked_in` says an active lock places parts of
    /// the group by its whole turn; `planes` are its normals that a point is measured from.
    fn domain(&self, g: usize, active: &[usize], locked_in: bool, planes: &[Normal]) -> Vec<DQuat> {
        let mut normals = planes.to_vec();
        for &i in active {
            match &self.rules[i].2 {
                Rule::Fixed { g: f, rotation } if *f == g => return vec![*rotation],
                Rule::Relate { a, u, b, w, .. } => {
                    if *a == g {
                        normals.push(*u);
                    }
                    if *b == g {
                        normals.push(*w);
                    }
                }
                Rule::Mirror { a, u, b, w, s, m, .. } => {
                    for (h, n) in [(a, u), (b, w), (s, m)] {
                        if *h == g {
                            normals.push(*n);
                        }
                    }
                }
                // A profile's edges lie across its normal, so they read the whole turn.
                Rule::Spin { a, b, .. } if *a == g || *b == g => normals.extend(ALL),
                Rule::Fixed { .. } | Rule::Spin { .. } => {}
            }
        }
        let authored = self.authored[g];
        let mut out = Vec::new();
        // A group whose axes or points a mate places takes quarter turns only.
        let positioned = self.positioned[g];
        match normals.first() {
            // Nothing asks anything of the group's turn: it keeps the authored one.
            None if !locked_in && !positioned => return vec![authored],
            // No mate relates its normals, but a lock's offset turns with it, so the
            // translations may need any quarter turn. The authored turn still comes first.
            None if !positioned => out.push(authored),
            Some(&u0) if !self.locked[g] && !positioned && normals.iter().all(|n| dot(*n, u0).abs() == 1) => {
                for v in [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]] {
                    out.extend(spun(authored, u0, v));
                }
            }
            _ => {}
        }
        out.extend(self.cubes.iter().copied());
        let near = |q: &DQuat| q.dot(authored).abs();
        // A stable sort keeps the fixed order among equally near turns.
        out.sort_by(|p, q| near(q).total_cmp(&near(p)));
        let mut unique: Vec<DQuat> = Vec::new();
        for q in out {
            if !unique.iter().any(|u| same_turn(*u, q)) {
                unique.push(q);
            }
        }
        unique
    }

    /// Where `q` takes group `g`'s read normals, or `None` when one leaves the axes.
    fn signature(&self, g: usize, q: DQuat) -> Option<Vec<Normal>> {
        self.reads[g].iter().map(|n| turned(q, *n)).collect()
    }

    /// Turns for every group that keep every `active` constraint: its rules, and, when
    /// `placing`, its translations too. `None` when there are none. The answer does not
    /// depend on the order of the mates.
    fn run(&mut self, active: &[bool], placing: bool) -> Result<Option<Vec<DQuat>>, MateRefusal> {
        self.steps = 0;
        let groups = self.authored.len();
        let rules: Vec<usize> = (0..self.rules.len()).filter(|i| active[self.rules[*i].1]).collect();
        let mut locked_in = vec![false; groups];
        for (c, on) in self.constraints.iter().zip(active) {
            if let (Constraint::Lock { a, .. }, true) = (c, on) {
                locked_in[self.member[a].0] = true;
            }
        }
        // A plane a point is measured from asks for its group's turn only through the
        // translation, so only a search that checks translations tries turns for it.
        let mut planes = vec![Vec::new(); groups];
        for (from, on) in self.from_planes.iter().zip(active) {
            if let (Some((g, n)), true) = (from, placing && *on) {
                planes[*g].push(*n);
            }
        }
        let pass = Pass {
            active: active.to_vec(),
            domains: (0..groups).map(|g| self.domain(g, &rules, placing && locked_in[g], &planes[g])).collect(),
            touching: (0..groups)
                .map(|g| rules.iter().copied().filter(|i| self.rules[*i].2.groups().contains(&g)).collect())
                .collect(),
            moves: (0..groups)
                .map(|g| {
                    (0..self.constraints.len())
                        .filter(|c| {
                            placing && active[*c] && self.placed_by[*c].as_ref().is_some_and(|gs| gs.contains(&g))
                        })
                        .collect()
                })
                .collect(),
        };
        let mut at = vec![None; groups];
        // Origin mates ask for translations before any turn is chosen.
        if placing && self.conflict(active, &at)?.is_some() {
            return Ok(None);
        }
        match self.fill(&pass, &mut at) {
            Ok(true) => Ok(Some(at.into_iter().map(|q| q.expect("every group is turned")).collect())),
            Ok(false) => Ok(None),
            Err(Stop::TooLong) => Err(self.too_long()),
            Err(Stop::Refused(refusal)) => Err(refusal),
        }
    }

    /// Backtracking with forward checking: the open group with the fewest candidates left,
    /// first by index among equals, tries each in order.
    fn fill(&mut self, pass: &Pass, at: &mut [Option<DQuat>]) -> Result<bool, Stop> {
        let mut best: Option<(usize, Vec<DQuat>)> = None;
        for g in 0..at.len() {
            if at[g].is_some() {
                continue;
            }
            let mut left = Vec::new();
            for q in &pass.domains[g] {
                at[g] = Some(*q);
                if pass.touching[g].iter().all(|i| self.rules[*i].2.holds(at) != Some(false)) {
                    left.push(*q);
                }
            }
            at[g] = None;
            if left.is_empty() {
                return Ok(false);
            }
            if best.as_ref().is_none_or(|(_, b)| left.len() < b.len()) {
                best = Some((g, left));
            }
        }
        let Some((g, left)) = best else { return Ok(true) };
        // Turns that take the group's normals onto the same axes decide the same: once
        // one has failed, the rest are not tried again.
        let mut failed: Vec<Vec<Normal>> = Vec::new();
        for q in left {
            let signature = self.signature(g, q);
            if signature.as_ref().is_some_and(|s| failed.contains(s)) {
                continue;
            }
            self.steps += 1;
            if self.steps > SEARCH_STEPS {
                return Err(Stop::TooLong);
            }
            at[g] = Some(q);
            // A translation can only start to disagree when a mate asking for one has all
            // its turns.
            let placed = pass.moves[g].iter().any(|c| self.placed_by[*c].iter().flatten().all(|h| at[*h].is_some()));
            if (!placed || self.conflict(&pass.active, at)?.is_none()) && self.fill(pass, at)? {
                return Ok(true);
            }
            failed.extend(signature);
        }
        at[g] = None;
        Ok(false)
    }

    /// The first disagreement among the translations the `active` constraints ask for,
    /// counting only those whose turns are all chosen in `at`.
    fn conflict(&self, active: &[bool], at: &[Option<DQuat>]) -> Result<Option<MateRefusal>, MateRefusal> {
        let turn = |body: &Body| {
            let (g, offset) = self.member[body];
            canonical((at[g].expect("a mate is placed only once its turns are chosen") * offset).normalize())
        };
        let mut ready: Vec<&Constraint> = Vec::new();
        let mut waiting = false;
        for ((c, by), on) in self.constraints.iter().zip(&self.placed_by).zip(active) {
            match by {
                Some(gs) if *on && gs.iter().all(|g| at[*g].is_some()) => ready.push(c),
                Some(_) if *on => waiting = true,
                _ => {}
            }
        }
        // While a mate still waits for its turns, a bound may read a ratio that the mate
        // would cancel: it is left out here and decided once every mate is in.
        let scaled = if waiting { Scaled::Skip } else { Scaled::Refuse };
        match place(self.bodies, &asks(ready.iter().copied(), &turn)?, scaled) {
            Ok(_) => Ok(None),
            Err(r) if r.code == RefusalCode::OverConstrained => Ok(Some(r)),
            Err(r) => Err(r),
        }
    }

    fn too_long(&self) -> MateRefusal {
        let mut names: Vec<String> = self.rules.iter().map(|(m, _, _)| (*m).to_owned()).collect();
        names.sort();
        names.dedup();
        MateRefusal {
            mates: names,
            ..MateRefusal::new(
                RefusalCode::TooLarge,
                format!("these mates' turns take more than {SEARCH_STEPS} steps to decide"),
            )
        }
    }

    /// Why no turns let every mate hold: a smallest set of mates that cannot hold
    /// together, found by dropping each mate in name order while what is left still fails.
    /// When the turns alone cannot hold, it names a set whose turns conflict; otherwise
    /// one that no choice of turns lets hold with its translations.
    fn refusal(&mut self) -> Result<MateRefusal, MateRefusal> {
        let all = vec![true; self.constraints.len()];
        let placing = self.run(&all, false)?.is_some();
        let mut names: Vec<&str> = self.constraints.iter().map(Constraint::mate).collect();
        names.sort();
        names.dedup();
        let mut keep = all;
        for name in names {
            let trial: Vec<bool> = keep.iter().zip(self.constraints).map(|(k, c)| *k && c.mate() != name).collect();
            if self.run(&trial, placing)?.is_none() {
                keep = trial;
            }
        }
        let mut named: Vec<String> =
            self.constraints.iter().zip(&keep).filter(|(_, k)| **k).map(|(c, _)| c.mate().to_owned()).collect();
        named.sort();
        named.dedup();
        if !placing {
            return Ok(incompatible(named));
        }
        // Say what the mates ask for in the pose nearest the authored one.
        let nearest = match self.run(&keep, false)? {
            Some(turns) => self.conflict(&keep, &turns.into_iter().map(Some).collect::<Vec<_>>())?,
            None => None,
        };
        Ok(match nearest {
            Some(refusal) if refusal.mates == named => refusal,
            Some(refusal) => {
                let message = format!(
                    "{}: no quarter-turn pose lets them all hold; nearest the authored pose, {}",
                    named.join(", "),
                    refusal.message
                );
                conflict(named, message)
            }
            None => {
                let message = format!("{}: no quarter-turn pose lets them all hold", named.join(", "));
                conflict(named, message)
            }
        })
    }
}

/// Each body's turn, chosen so that every mate holds, the translations included.
fn rotations(
    bodies: &BTreeMap<Body, Placement>,
    constraints: &[Constraint],
) -> Result<BTreeMap<Body, DQuat>, MateRefusal> {
    let mut search = Search::new(bodies, constraints)?;
    let Some(turns) = search.run(&vec![true; constraints.len()], true)? else { return Err(search.refusal()?) };
    let known: BTreeMap<Body, DQuat> = search
        .member
        .iter()
        .map(|(body, (g, offset))| (body.clone(), canonical((turns[*g] * *offset).normalize())))
        .collect();
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
            Constraint::Lock { a, b, relative: Placement { rotation: relative, .. }, .. }
            | Constraint::Turn { a, b, relative, .. } => {
                let wanted = known[a] * *relative;
                let off = [DVec3::X, DVec3::Y].iter().any(|v| (wanted * *v).distance(known[b] * *v) > AXIS_TOLERANCE);
                if off {
                    return Err(incompatible(between(constraints, a, b)));
                }
            }
            Constraint::Axes { mate, a, b, axis_a, axis_b, sign, .. } => {
                let (na, nb) = (world(known[a], axis_a.direction, mate)?, world(known[b], axis_b.direction, mate)?);
                let holds = if *sign == 0 { dot(na, nb) == 0 } else { nb == na.map(|c| c * sign) };
                if !holds {
                    return Err(incompatible(between(constraints, a, b)));
                }
            }
            Constraint::FromPlane { mate, a, plane, b, direction: Some(d), .. } => {
                let (na, nb) = (world(known[a], plane.normal, mate)?, world(known[b], *d, mate)?);
                if dot(na, nb) != 0 {
                    return Err(incompatible(between(constraints, a, b)));
                }
            }
            Constraint::Symmetric { mate, a, b, s, plane, ends, sign } => {
                let m = world(known[s], plane.normal, mate)?;
                let directions = match ends {
                    Mirrored::Planes(pa, pb) => Some((pa.normal, pb.normal)),
                    Mirrored::Axes(xa, xb) => Some((xa.direction, xb.direction)),
                    Mirrored::Points(..) => None,
                };
                if let Some((u, w)) = directions {
                    let (na, nb) = (world(known[a], u, mate)?, world(known[b], w, mate)?);
                    if nb != mirror(na, m).map(|c| c * sign) {
                        return Err(incompatible(vec![mate.clone()]));
                    }
                }
            }
            Constraint::Slot { mate, a, slot, b, axis, sign, .. } => {
                let (na, nb) = (world(known[a], slot.depth, mate)?, world(known[b], axis.direction, mate)?);
                if nb != na.map(|c| c * sign) {
                    return Err(incompatible(between(constraints, a, b)));
                }
            }
            Constraint::Spin { a, b, edges: Some((ea, eb)), .. } => {
                let turn = |q: DQuat, e: &[DVec3]| e.iter().map(|v| q * *v).collect::<Vec<_>>();
                if !along_any(&turn(known[a], ea), &turn(known[b], eb)) {
                    return Err(incompatible(between(constraints, a, b)));
                }
            }
            Constraint::Origin { mate, body, axes: true, at } => {
                // Two Origin mates on one locked group must agree on its turn.
                let off =
                    [DVec3::X, DVec3::Y].iter().any(|v| (at.rotation * *v).distance(known[body] * *v) > AXIS_TOLERANCE);
                if off {
                    let g = search.member[body].0;
                    let mut names: Vec<String> = constraints
                        .iter()
                        .filter(|c| match c {
                            Constraint::Origin { body, axes: true, .. } => search.member[body].0 == g,
                            Constraint::Lock { a, .. } | Constraint::Turn { a, .. } => search.member[a].0 == g,
                            _ => false,
                        })
                        .map(|c| c.mate().to_owned())
                        .collect();
                    names.push(mate.clone());
                    names.sort();
                    names.dedup();
                    return Err(incompatible(names));
                }
            }
            Constraint::Origin { .. }
            | Constraint::OnAxis { .. }
            | Constraint::Points { .. }
            | Constraint::FromPlane { direction: None, .. }
            | Constraint::Coupler { .. }
            | Constraint::Spin { edges: None, .. } => {}
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

/// `lo <= t[to] - t[from] <= hi` along one world axis.
struct Bound<'a> {
    from: Option<&'a Body>,
    to: &'a Body,
    lo: BigRational,
    hi: BigRational,
    mate: &'a str,
    says: String,
}

/// The sum of each coefficient times a body's coordinate along a world axis is `value`: a
/// relation no single difference holds, such as a mirror's or a coupler's.
struct Row<'a> {
    terms: Vec<(&'a Body, usize, BigRational)>,
    value: BigRational,
    mate: &'a str,
    says: String,
}

/// What the translations are asked once every turn is known: differences and bounds along
/// each world axis, and relations across bodies and axes.
struct Asks<'a> {
    edges: [Vec<Edge<'a>>; 3],
    bounds: [Vec<Bound<'a>>; 3],
    rows: Vec<Row<'a>>,
}

fn int(n: i64) -> BigRational {
    BigRational::from_integer(BigInt::from(n))
}

/// The world axis a signed axis lies on.
fn axis_of(n: Normal) -> usize {
    n.iter().position(|c| *c != 0).expect("a signed axis")
}

fn finite(value: f64, mate: &str, what: &str) -> Result<BigRational, MateRefusal> {
    exact(value).ok_or_else(|| MateRefusal::invalid(format!("{mate}: {what} is not finite")))
}

/// The lesser and the greater of two values.
fn ordered(p: BigRational, q: BigRational) -> (BigRational, BigRational) {
    if p <= q { (p, q) } else { (q, p) }
}

fn asks<'a>(
    constraints: impl IntoIterator<Item = &'a Constraint>,
    turned: &impl Fn(&Body) -> DQuat,
) -> Result<Asks<'a>, MateRefusal> {
    let mut out = Asks { edges: Default::default(), bounds: Default::default(), rows: Vec::new() };
    for c in constraints {
        match c {
            Constraint::Origin { mate, body, at, .. } => {
                for axis in 0..3 {
                    let gap = finite(at.translation[axis], mate, "the frame")?;
                    out.edges[axis].push(Edge { from: None, to: body, gap, mate, says: "at the origin".into() });
                }
            }
            Constraint::Parallel { mate, a, b, plane_a, plane_b, sign, along: Some(along) } => {
                let n = world(turned(a), plane_a.normal, mate)?;
                let axis = axis_of(n);
                let s = int(i64::from(*sign));
                let side = int(i64::from(n[axis]));
                let gap_at = |d: &BigRational| &side * (d + &plane_a.offset - &s * &plane_b.offset);
                match along {
                    Along::Distance(distance) => {
                        let gap = gap_at(&finite(*distance, mate, "the distance")?);
                        out.edges[axis].push(Edge { from: Some(a), to: b, gap, mate, says: format!("{distance} mm") });
                    }
                    Along::Range(min, max) => {
                        let (lo, hi) = ordered(
                            gap_at(&finite(*min, mate, "the least distance")?),
                            gap_at(&finite(*max, mate, "the greatest distance")?),
                        );
                        let says = format!("from {min} mm to {max} mm");
                        out.bounds[axis].push(Bound { from: Some(a), to: b, lo, hi, mate, says });
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
                            Ok(int(i64::from(w[axis])) * &p.offset)
                        };
                        let g = [at(turned(a), plane_a)?, at(turned(a), groove)?];
                        let h = [at(turned(b), plane_b)?, at(turned(b), tab)?];
                        let (h_lo, h_hi) = if h[0] <= h[1] { (&h[0], &h[1]) } else { (&h[1], &h[0]) };
                        let gap_from = |from: &BigRational, other: &BigRational, gap: BigRational| {
                            if from <= other { from + gap - h_lo } else { from - gap - h_hi }
                        };
                        let (gap, says) = match mode {
                            WidthMode::Free => {
                                // The tab's faces stay within the groove's.
                                let (g_lo, g_hi) = ordered(g[0].clone(), g[1].clone());
                                let (lo, hi) = (g_lo - h_lo, g_hi - h_hi);
                                let says = "within the groove".to_owned();
                                out.bounds[axis].push(Bound { from: Some(a), to: b, lo, hi, mate, says });
                                continue;
                            }
                            WidthMode::Centered => ((&g[0] + &g[1] - &h[0] - &h[1]) / int(2), "centred".to_owned()),
                            WidthMode::Gap { distance, flipped } => {
                                let d = finite(*distance, mate, "the distance")?;
                                let (from, other) = if *flipped { (&g[1], &g[0]) } else { (&g[0], &g[1]) };
                                (gap_from(from, other, d), format!("{distance} mm from a groove face"))
                            }
                            WidthMode::Share { fraction, flipped } => {
                                let (from, other) = if *flipped { (&g[1], &g[0]) } else { (&g[0], &g[1]) };
                                let play = (from - other).abs() - (h_hi - h_lo);
                                let percent = nearest(&(fraction * int(100)));
                                (gap_from(from, other, fraction * play), format!("{percent}% of the play"))
                            }
                        };
                        out.edges[axis].push(Edge { from: Some(a), to: b, gap, mate, says });
                    }
                }
            }
            Constraint::Lock { mate, a, b, relative } => {
                let m = cube_of(turned(a)).ok_or_else(|| unsupported_lock(mate))?;
                let t = relative.translation.to_array();
                for (row, edges) in m.iter().zip(&mut out.edges) {
                    let j = (0..3).find(|j| row[*j] != 0).expect("a signed permutation");
                    let offset = finite(t[j], mate, "the lock")?;
                    let gap = int(i64::from(row[j])) * offset;
                    edges.push(Edge { from: Some(a), to: b, gap, mate, says: "locked".into() });
                }
            }
            Constraint::Axes { mate, a, b, axis_a, axis_b, coaxial: true, .. } => {
                let d = world(turned(a), axis_a.direction, mate)?;
                let (pa, pb) = (placed(turned(a), &axis_a.through, mate)?, placed(turned(b), &axis_b.through, mate)?);
                for axis in (0..3).filter(|k| d[*k] == 0) {
                    let gap = &pa[axis] - &pb[axis];
                    out.edges[axis].push(Edge { from: Some(a), to: b, gap, mate, says: "on one axis".into() });
                }
            }
            Constraint::OnAxis { mate, a, axis: line, b, point } => {
                let d = world(turned(a), line.direction, mate)?;
                let (pa, pb) = (placed(turned(a), &line.through, mate)?, placed(turned(b), point, mate)?);
                for axis in (0..3).filter(|k| d[*k] == 0) {
                    let gap = &pa[axis] - &pb[axis];
                    out.edges[axis].push(Edge { from: Some(a), to: b, gap, mate, says: "on the axis".into() });
                }
            }
            Constraint::Points { mate, a, point_a, b, point_b } => {
                let (pa, pb) = (placed(turned(a), point_a, mate)?, placed(turned(b), point_b, mate)?);
                for axis in 0..3 {
                    let gap = &pa[axis] - &pb[axis];
                    out.edges[axis].push(Edge { from: Some(a), to: b, gap, mate, says: "on one point".into() });
                }
            }
            Constraint::FromPlane { mate, a, plane, b, point, gap, upto, says, .. } => {
                let n = world(turned(a), plane.normal, mate)?;
                let axis = axis_of(n);
                let side = int(i64::from(n[axis]));
                let pb = placed(turned(b), point, mate)?;
                let at = |gap: &BigRational| &side * (gap + &plane.offset) - &pb[axis];
                match upto {
                    None => out.edges[axis].push(Edge { from: Some(a), to: b, gap: at(gap), mate, says: says.clone() }),
                    Some(upto) => {
                        let (lo, hi) = ordered(at(gap), at(upto));
                        out.bounds[axis].push(Bound { from: Some(a), to: b, lo, hi, mate, says: says.clone() });
                    }
                }
            }
            Constraint::Symmetric { mate, a, b, s, plane, ends, .. } => {
                let m = world(turned(s), plane.normal, mate)?;
                let k = axis_of(m);
                // Where the mirror lies along its normal's axis, from its body's origin.
                let mirror_at = int(i64::from(m[k])) * &plane.offset;
                let says = "mirrored".to_owned();
                // a + b = 2 s along the mirror's normal, given each one's place from its origin.
                let mut halves = |pa: BigRational, pb: BigRational| {
                    let terms = vec![(a, k, int(1)), (b, k, int(1)), (s, k, int(-2))];
                    out.rows.push(Row { terms, value: int(2) * &mirror_at - pa - pb, mate, says: says.clone() });
                };
                match ends {
                    Mirrored::Planes(pa, pb) => {
                        let (na, nb) = (world(turned(a), pa.normal, mate)?, world(turned(b), pb.normal, mate)?);
                        let place_a = |axis: usize| int(i64::from(na[axis])) * &pa.offset;
                        let place_b = |axis: usize| int(i64::from(nb[axis])) * &pb.offset;
                        if na[k] != 0 {
                            halves(place_a(k), place_b(k));
                        } else {
                            // A plane square to the mirror is its own image: `b`'s lies on it.
                            let axis = axis_of(na);
                            let gap = place_a(axis) - place_b(axis);
                            out.edges[axis].push(Edge { from: Some(a), to: b, gap, mate, says: says.clone() });
                        }
                    }
                    Mirrored::Points(pa, pb) => {
                        let (pa, pb) = (placed(turned(a), pa, mate)?, placed(turned(b), pb, mate)?);
                        halves(pa[k].clone(), pb[k].clone());
                        for axis in (0..3).filter(|j| *j != k) {
                            let gap = &pa[axis] - &pb[axis];
                            out.edges[axis].push(Edge { from: Some(a), to: b, gap, mate, says: says.clone() });
                        }
                    }
                    Mirrored::Axes(xa, xb) => {
                        let d = world(turned(a), xa.direction, mate)?;
                        let (pa, pb) = (placed(turned(a), &xa.through, mate)?, placed(turned(b), &xb.through, mate)?);
                        // A line along the mirror's normal is its own image; one across it
                        // lands as far beyond the mirror as it lies before it.
                        if d[k] == 0 {
                            halves(pa[k].clone(), pb[k].clone());
                        }
                        for axis in (0..3).filter(|j| d[*j] == 0 && *j != k) {
                            let gap = &pa[axis] - &pb[axis];
                            out.edges[axis].push(Edge { from: Some(a), to: b, gap, mate, says: says.clone() });
                        }
                    }
                }
            }
            Constraint::Coupler { mate, a, b, ratio, reverse, offset } => {
                // Each side reads `c * (sigma t[body] - sigma t[from] + offset)` along its axis.
                let mut terms = Vec::new();
                let mut value = offset.clone();
                let ca = if *reverse { ratio[1].clone() } else { -&ratio[1] };
                for (side, c) in [(b, ratio[0].clone()), (a, ca)] {
                    let n = world(turned(&side.body), side.direction, mate)?;
                    let k = axis_of(n);
                    let sigma = int(i64::from(n[k]));
                    terms.push((&side.body, k, &c * &sigma));
                    let mut constant = side.offset.clone();
                    match &side.from {
                        Some(from) => terms.push((from, k, -(&c * &sigma))),
                        None => constant -= &sigma * &side.frame[k],
                    }
                    value -= c * constant;
                }
                out.rows.push(Row { terms, value, mate, says: "coupled".into() });
            }
            Constraint::Slot { mate, a, slot, b, axis: line, mode, .. } => {
                let d = world(turned(a), slot.depth, mate)?;
                let ends = [placed(turned(a), &slot.ends[0], mate)?, placed(turned(a), &slot.ends[1], mate)?];
                let p = placed(turned(b), &line.through, mate)?;
                let across: Vec<usize> = (0..3).filter(|k| d[*k] == 0).collect();
                let Some(&along) = across.iter().find(|k| ends[0][**k] != ends[1][**k]) else {
                    return Err(MateRefusal::invalid(format!("{mate}: the slot's ends are at one place")));
                };
                for &k in across.iter().filter(|k| **k != along) {
                    let gap = &ends[0][k] - &p[k];
                    out.edges[k].push(Edge { from: Some(a), to: b, gap, mate, says: "on the slot's mid-plane".into() });
                }
                let (first, second) = (&ends[0][along], &ends[1][along]);
                let towards = |from: &BigRational, to: &BigRational| if from <= to { int(1) } else { int(-1) };
                let (gap, says) = match mode {
                    SlotMode::Free => {
                        let (lo, hi) = ordered(first - &p[along], second - &p[along]);
                        let says = "between the slot's ends".to_owned();
                        out.bounds[along].push(Bound { from: Some(a), to: b, lo, hi, mate, says });
                        continue;
                    }
                    SlotMode::Centre => ((first + second) / int(2), "centred in the slot".to_owned()),
                    SlotMode::Distance { distance, flipped } => {
                        let (from, to) = if *flipped { (second, first) } else { (first, second) };
                        (from + towards(from, to) * distance, format!("{} mm along the slot", nearest(distance)))
                    }
                    SlotMode::Share { fraction, flipped } => {
                        let (from, to) = if *flipped { (second, first) } else { (first, second) };
                        let percent = nearest(&(fraction * int(100)));
                        (from + fraction * (to - from), format!("{percent}% along the slot"))
                    }
                };
                out.edges[along].push(Edge { from: Some(a), to: b, gap: gap - &p[along], mate, says });
            }
            Constraint::Parallel { along: None, .. }
            | Constraint::Perpendicular { .. }
            | Constraint::Axes { coaxial: false, .. }
            | Constraint::Turn { .. }
            | Constraint::Spin { .. } => {}
        }
    }
    Ok(out)
}

/// A point of a body turned by `rotation`, relative to the body's origin, exactly. Only a
/// quarter-turn rotation keeps it rational, and the rotation search gives such bodies no
/// other.
fn placed(rotation: DQuat, p: &LocalPoint, mate: &str) -> Result<LocalPoint, MateRefusal> {
    let m = cube_of(rotation).ok_or_else(|| {
        let message = format!(
            "{mate}: a part whose axis or point it reads is turned off the quarter turns; this version places axes and points on quarter-turn placements only"
        );
        MateRefusal { mates: vec![mate.to_owned()], ..MateRefusal::new(RefusalCode::UnsupportedPlacement, message) }
    })?;
    Ok(std::array::from_fn(|k| {
        let j = (0..3).find(|j| m[k][*j] != 0).expect("a signed permutation");
        int(i64::from(m[k][j])) * &p[j]
    }))
}

/// The refusal for a lock whose parts are turned off the quarter turns.
pub(crate) fn unsupported_lock(mate: &str) -> MateRefusal {
    let message = format!(
        "{mate}: a locked part is turned off the quarter turns; this version locks quarter-turn rotations only"
    );
    MateRefusal { mates: vec![mate.to_owned()], ..MateRefusal::new(RefusalCode::UnsupportedPlacement, message) }
}

/// A body, or the assembly's origin.
type Node<'a> = Option<&'a Body>;

/// One axis's graph of differences, walked from each graph's root.
struct Tree<'a> {
    /// Each node's coordinate, from its root's authored one along the edges.
    at: BTreeMap<Node<'a>, BigRational>,
    /// Each node's root: the origin, or the first body of its graph.
    root: BTreeMap<Node<'a>, Node<'a>>,
    /// The edge each node was reached by.
    parent: BTreeMap<Node<'a>, usize>,
}

impl<'a> Tree<'a> {
    /// The edges from `node` up to its root.
    fn path(&self, edges: &[Edge<'a>], mut node: Node<'a>) -> Vec<usize> {
        let mut out = Vec::new();
        while let Some(&i) = self.parent.get(&node) {
            out.push(i);
            let e = &edges[i];
            node = if Some(e.to) == node { e.from } else { Some(e.to) };
        }
        out
    }
}

/// Every world coordinate of every body along one axis, by its graph of differences.
fn walk<'a>(bodies: &'a BTreeMap<Body, Placement>, edges: &[Edge<'a>], axis: usize) -> Result<Tree<'a>, MateRefusal> {
    let mut adjacent: BTreeMap<Node, Vec<usize>> = BTreeMap::new();
    for (i, e) in edges.iter().enumerate() {
        adjacent.entry(e.from).or_default().push(i);
        adjacent.entry(Some(e.to)).or_default().push(i);
    }
    let mut tree = Tree { at: BTreeMap::new(), root: BTreeMap::new(), parent: BTreeMap::new() };
    let roots = std::iter::once(None).chain(bodies.keys().map(Some));
    for root in roots {
        if tree.at.contains_key(&root) {
            continue;
        }
        let start = match root {
            None => BigRational::zero(),
            Some(b) => exact(bodies[b].translation[axis])
                .ok_or_else(|| MateRefusal::invalid(format!("occurrence {b} has no finite placement")))?,
        };
        tree.at.insert(root, start);
        tree.root.insert(root, root);
        let mut queue = VecDeque::from([root]);
        while let Some(node) = queue.pop_front() {
            for &i in adjacent.get(&node).into_iter().flatten() {
                let e = &edges[i];
                let (other, value) = if e.from == node {
                    (Some(e.to), &tree.at[&node] + &e.gap)
                } else {
                    (e.from, &tree.at[&node] - &e.gap)
                };
                match tree.at.get(&other) {
                    None => {
                        tree.at.insert(other, value);
                        tree.root.insert(other, root);
                        tree.parent.insert(other, i);
                        queue.push_back(other);
                    }
                    Some(placed) => {
                        let apart = (placed - &value).abs();
                        if nearest(&apart) > CONFLICT_MM {
                            return Err(disagreement(edges, &tree.parent, node, other, i));
                        }
                    }
                }
            }
        }
    }
    Ok(tree)
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

/// A sum of coefficients times unknowns, plus a constant.
#[derive(Clone, Default)]
struct Affine {
    terms: BTreeMap<usize, BigRational>,
    constant: BigRational,
}

impl Affine {
    fn add(&mut self, other: &Affine, factor: &BigRational) {
        for (k, v) in &other.terms {
            let sum = self.terms.get(k).cloned().unwrap_or_default() + factor * v;
            if sum.is_zero() {
                self.terms.remove(k);
            } else {
                self.terms.insert(*k, sum);
            }
        }
        self.constant += factor * &other.constant;
    }
}

/// `x[to] - x[from] <= w`, over the unknowns a bound reads, node 0 being zero.
struct Leq {
    from: usize,
    to: usize,
    w: BigRational,
    /// The bound it comes from; `None` for a value already chosen.
    bound: Option<usize>,
}

/// The shortest distance to every node from `source`, or from every node at once when there
/// is none; a negative cycle comes back as the bounds on it.
fn shortest(nodes: usize, leqs: &[Leq], source: Option<usize>) -> Result<Vec<Option<BigRational>>, Vec<usize>> {
    let mut dist: Vec<Option<BigRational>> = match source {
        Some(s) => (0..nodes).map(|n| (n == s).then(BigRational::zero)).collect(),
        None => vec![Some(BigRational::zero()); nodes],
    };
    let mut pred: Vec<Option<usize>> = vec![None; nodes];
    for round in 0..=nodes {
        let mut changed = None;
        for (i, e) in leqs.iter().enumerate() {
            let Some(d) = &dist[e.from] else { continue };
            let through = d + &e.w;
            if dist[e.to].as_ref().is_none_or(|t| through < *t) {
                dist[e.to] = Some(through);
                pred[e.to] = Some(i);
                changed = Some(e.to);
            }
        }
        let Some(mut node) = changed else { return Ok(dist) };
        if round == nodes {
            // Still relaxing after every round: walk back into the cycle and read it off.
            for _ in 0..nodes {
                node = leqs[pred[node].expect("a relaxed node has a predecessor")].from;
            }
            let (start, mut on) = (node, Vec::new());
            loop {
                let e = &leqs[pred[node].expect("a node on the cycle has a predecessor")];
                on.extend(e.bound);
                node = e.from;
                if node == start {
                    break;
                }
            }
            on.sort();
            on.dedup();
            return Err(on);
        }
    }
    Ok(dist)
}

/// What [`place`] does with a bound that reduces to a coordinate moving by a ratio.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scaled {
    /// Refuse it by name: every mate is in, so nothing will reduce it further.
    Refuse,
    /// Leave it out: some mates are not in yet, and one may make it a difference or a
    /// constant. Leaving a bound out only lets more through, so a conflict found without
    /// it is still a conflict.
    Skip,
}

/// Every body's coordinates along the three world axes, exactly, or why they cannot hold.
///
/// Differences are walked per axis as a graph, each graph keeping its root where it is.
/// Relations across bodies and axes are then solved over the graphs' roots by exact
/// elimination, a later body's root moving before an earlier one's, so whatever stays free
/// keeps its authored place. Bounds on what is still free are differences, decided by
/// Bellman-Ford: each free coordinate keeps its authored value where the bounds allow it and
/// otherwise takes the nearest one they allow, earlier bodies first.
fn place<'a>(
    bodies: &'a BTreeMap<Body, Placement>,
    asks: &Asks<'a>,
    scaled: Scaled,
) -> Result<[BTreeMap<&'a Body, BigRational>; 3], MateRefusal> {
    let mut trees = Vec::with_capacity(3);
    for axis in 0..3 {
        trees.push(walk(bodies, &asks.edges[axis], axis)?);
    }
    let coordinates = |trees: &[Tree<'a>], values: &dyn Fn(usize, Node<'a>) -> Option<BigRational>| {
        std::array::from_fn(|axis| {
            let tree: &Tree<'a> = &trees[axis];
            bodies
                .keys()
                .map(|body| {
                    let node = Some(body);
                    let root = tree.root[&node];
                    let value = match values(axis, root) {
                        Some(u) => u + &tree.at[&node] - &tree.at[&root],
                        None => tree.at[&node].clone(),
                    };
                    (body, value)
                })
                .collect()
        })
    };
    if asks.rows.is_empty() && asks.bounds.iter().all(Vec::is_empty) {
        return Ok(coordinates(&trees, &|_, _| None));
    }
    // The unknowns are the roots' coordinates the relations and bounds read, a later body
    // first, so a relation moves it and an earlier body keeps its authored place.
    let mut keys: BTreeSet<(std::cmp::Reverse<&'a Body>, usize)> = BTreeSet::new();
    let root_of = |axis: usize, body: Node<'a>| trees[axis].root[&body];
    for row in &asks.rows {
        for (body, axis, _) in &row.terms {
            if let Some(r) = root_of(*axis, Some(body)) {
                keys.insert((std::cmp::Reverse(r), *axis));
            }
        }
    }
    for (axis, bounds) in asks.bounds.iter().enumerate() {
        for bound in bounds {
            for node in [bound.from, Some(bound.to)] {
                if let Some(r) = root_of(axis, node) {
                    keys.insert((std::cmp::Reverse(r), axis));
                }
            }
        }
    }
    let keys: Vec<(&'a Body, usize)> = keys.into_iter().map(|(std::cmp::Reverse(b), a)| (b, a)).collect();
    let column: BTreeMap<(&'a Body, usize), usize> = keys.iter().copied().zip(0..).collect();
    // A node's coordinate as an affine form over the unknowns.
    let express = |axis: usize, node: Node<'a>| -> Affine {
        let tree = &trees[axis];
        match tree.root[&node] {
            None => Affine { terms: BTreeMap::new(), constant: tree.at[&node].clone() },
            Some(r) => Affine {
                terms: BTreeMap::from([(column[&(r, axis)], int(1))]),
                constant: &tree.at[&node] - &tree.at[&Some(r)],
            },
        }
    };
    // The mates behind a node's place: the differences from its root.
    let behind = |axis: usize, node: Node<'a>| -> Vec<&'a str> {
        trees[axis].path(&asks.edges[axis], node).into_iter().map(|i| asks.edges[axis][i].mate).collect()
    };
    // Each relation as `sum = 0`, with the relations it was combined from.
    let mut rows: Vec<(Affine, BTreeSet<usize>)> = Vec::new();
    for (i, row) in asks.rows.iter().enumerate() {
        let mut form = Affine { constant: -&row.value, ..Affine::default() };
        for (body, axis, c) in &row.terms {
            form.add(&express(*axis, Some(body)), c);
        }
        rows.push((form, BTreeSet::from([i])));
    }
    // Reduced row echelon form, column by column: unique for this order of the unknowns.
    let mut pivots: BTreeMap<usize, usize> = BTreeMap::new();
    for col in 0..keys.len() {
        let Some(r) = (0..rows.len())
            .find(|r| !pivots.values().any(|p| p == r) && rows[*r].0.terms.get(&col).is_some_and(|c| !c.is_zero()))
        else {
            continue;
        };
        let scale = BigRational::from_integer(BigInt::from(1)) / &rows[r].0.terms[&col];
        let mut top = Affine::default();
        top.add(&rows[r].0, &scale);
        rows[r].0 = top.clone();
        let sources = rows[r].1.clone();
        for (other, (form, from)) in rows.iter_mut().enumerate() {
            if other == r {
                continue;
            }
            if let Some(c) = form.terms.get(&col).cloned() {
                form.add(&top, &-c);
                from.extend(sources.iter().copied());
            }
        }
        pivots.insert(col, r);
    }
    let named = |sources: &BTreeSet<usize>, extra: &[&'a str]| -> Vec<String> {
        let mut names: Vec<String> = extra.iter().map(|m| (*m).to_owned()).collect();
        for &i in sources {
            names.push(asks.rows[i].mate.to_owned());
            for (body, axis, _) in &asks.rows[i].terms {
                names.extend(behind(*axis, Some(body)).into_iter().map(str::to_owned));
            }
        }
        names.sort();
        names.dedup();
        names
    };
    for (form, sources) in &rows {
        if form.terms.is_empty() && nearest(&form.constant.abs()) > CONFLICT_MM {
            let names = named(sources, &[]);
            let says: Vec<String> =
                sources.iter().map(|i| format!("{} ({})", asks.rows[*i].mate, asks.rows[*i].says)).collect();
            return Err(conflict(names, format!("{} cannot all hold", says.join(", "))));
        }
    }
    // A pivot unknown as an affine form over the free ones.
    let solved = |col: usize| -> Option<Affine> {
        let r = *pivots.get(&col)?;
        let mut out = Affine::default();
        let mut rest = rows[r].0.clone();
        rest.terms.remove(&col);
        out.add(&rest, &int(-1));
        Some(out)
    };
    // Bounds over the free unknowns, as `x[to] - x[from] <= w` with node 0 being zero.
    let free: Vec<usize> = (0..keys.len()).filter(|c| !pivots.contains_key(c)).collect();
    let node_of: BTreeMap<usize, usize> = free.iter().zip(1..).map(|(c, n)| (*c, n)).collect();
    let mut leqs = Vec::new();
    let mut bound_list: Vec<(&Bound<'a>, Vec<String>)> = Vec::new();
    for (axis, bounds) in asks.bounds.iter().enumerate() {
        for bound in bounds {
            let mut form = express(axis, Some(bound.to));
            form.add(&express(axis, bound.from), &int(-1));
            let mut sources = BTreeSet::new();
            let mut reduced = Affine { constant: form.constant.clone(), ..Affine::default() };
            for (col, c) in &form.terms {
                match solved(*col) {
                    Some(pivot) => {
                        reduced.add(&pivot, c);
                        sources.extend(rows[pivots[col]].1.iter().copied());
                    }
                    None => reduced.add(&Affine { terms: BTreeMap::from([(*col, int(1))]), ..Affine::default() }, c),
                }
            }
            let mut extra = vec![bound.mate];
            extra.extend(behind(axis, Some(bound.to)));
            extra.extend(behind(axis, bound.from));
            let names = named(&sources, &extra);
            let index = bound_list.len();
            let (lo, hi) = (&bound.lo - &reduced.constant, &bound.hi - &reduced.constant);
            let one = int(1);
            let terms: Vec<(usize, BigRational)> = reduced.terms.iter().map(|(k, v)| (*k, v.clone())).collect();
            let (to, from) = match &terms[..] {
                [] => {
                    let (lo, hi) = (nearest(&lo), nearest(&hi));
                    if lo > CONFLICT_MM || hi < -CONFLICT_MM {
                        let message = format!("{} ({}) cannot hold", bound.mate, bound.says);
                        return Err(conflict(names, message));
                    }
                    continue;
                }
                [(k, c)] if *c == one => (node_of[k], 0),
                [(k, c)] if *c == -&one => (0, node_of[k]),
                [(j, c), (k, d)] if *c == one && *d == -&one => (node_of[j], node_of[k]),
                [(j, c), (k, d)] if *c == -&one && *d == one => (node_of[k], node_of[j]),
                _ if scaled == Scaled::Skip => continue,
                _ => {
                    let message = format!(
                        "{}: it bounds a coordinate that {} also move by a ratio; this version bounds only coordinates that move one for one",
                        bound.mate,
                        names.iter().filter(|n| *n != bound.mate).cloned().collect::<Vec<_>>().join(", ")
                    );
                    return Err(MateRefusal {
                        mates: names,
                        ..MateRefusal::new(RefusalCode::UnsupportedReference, message)
                    });
                }
            };
            // lo <= x[to] - x[from] <= hi.
            leqs.push(Leq { from, to, w: hi, bound: Some(index) });
            leqs.push(Leq { from: to, to: from, w: -lo, bound: Some(index) });
            bound_list.push((bound, names));
        }
    }
    let nodes = free.len() + 1;
    let cycle_refusal = |on: Vec<usize>| {
        let mut names: Vec<String> = on.iter().flat_map(|i| bound_list[*i].1.iter().cloned()).collect();
        names.sort();
        names.dedup();
        let says: Vec<String> =
            on.iter().map(|i| format!("{} ({})", bound_list[*i].0.mate, bound_list[*i].0.says)).collect();
        conflict(names, format!("{} cannot all hold", says.join(", ")))
    };
    shortest(nodes, &leqs, None).map_err(cycle_refusal)?;
    // Each free unknown keeps its authored value where the bounds allow it, earlier bodies
    // first; otherwise it takes the nearest value they allow.
    let authored = |col: usize| {
        let (body, axis) = keys[col];
        trees[axis].at[&Some(body)].clone()
    };
    let mut values: BTreeMap<usize, BigRational> = BTreeMap::new();
    let mut order = free.clone();
    order.sort_by_key(|c| keys[*c]);
    for col in order {
        let node = node_of[&col];
        let mut value = authored(col);
        if leqs.iter().any(|e| e.from == node || e.to == node) {
            let upper = shortest(nodes, &leqs, Some(0)).map_err(cycle_refusal)?[node].clone();
            let lower = shortest(nodes, &leqs, Some(node)).map_err(cycle_refusal)?[0].clone().map(|d| -d);
            if let Some(hi) = upper.filter(|hi| value > *hi) {
                value = hi;
            }
            if let Some(lo) = lower.filter(|lo| value < *lo) {
                value = lo;
            }
            leqs.push(Leq { from: 0, to: node, w: value.clone(), bound: None });
            leqs.push(Leq { from: node, to: 0, w: -&value, bound: None });
        }
        values.insert(col, value);
    }
    for col in pivots.keys() {
        let form = solved(*col).expect("a pivot");
        let mut value = form.constant.clone();
        for (k, c) in &form.terms {
            value += c * &values[k];
        }
        values.insert(*col, value);
    }
    Ok(coordinates(&trees, &|axis, root| root.and_then(|r| values.get(column.get(&(r, axis))?).cloned())))
}

/// The placements that satisfy `constraints`, starting from `bodies`.
pub(crate) fn solve(
    bodies: &BTreeMap<Body, Placement>,
    constraints: &[Constraint],
) -> Result<BTreeMap<Body, Placement>, MateRefusal> {
    let turned = rotations(bodies, constraints)?;
    let coordinates = place(bodies, &asks(constraints, &|b: &Body| turned[b])?, Scaled::Refuse)?;
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
                // A range or a free width bounds the translation and fixes none of it.
                if matches!(along, None | Some(Along::Range(..) | Along::Width { mode: WidthMode::Free, .. })) {
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
            Constraint::Turn { a, b, .. } => {
                let (at_a, at_b) = (index[a] * 6, index[b] * 6);
                for k in 0..3 {
                    rows.push(row(size, &[(at_b + 3 + k, 1.0), (at_a + 3 + k, -1.0)]));
                }
            }
            Constraint::Axes { mate, a, b, axis_a, axis_b, sign, coaxial } => {
                let (pa, pb) = (&bodies[a], &bodies[b]);
                let da = world(pa.rotation, axis_a.direction, mate)?;
                let (at_a, at_b) = (index[a] * 6, index[b] * 6);
                if *sign == 0 {
                    let c = vector(da).cross(vector(world(pb.rotation, axis_b.direction, mate)?));
                    let entries: Vec<_> = (0..3).flat_map(|k| [(at_a + 3 + k, c[k]), (at_b + 3 + k, -c[k])]).collect();
                    rows.push(row(size, &entries));
                } else {
                    for e in (0..3).filter(|k| da[*k] == 0) {
                        rows.push(row(size, &[(at_b + 3 + e, 1.0), (at_a + 3 + e, -1.0)]));
                    }
                }
                if *coaxial {
                    let x = point(pb, &axis_b.through);
                    rows.extend(meet(size, (at_a, pa), (at_b, pb), x, (0..3).filter(|k| da[*k] == 0)));
                }
            }
            Constraint::OnAxis { mate, a, axis: line, b, point: p } => {
                let (pa, pb) = (&bodies[a], &bodies[b]);
                let da = world(pa.rotation, line.direction, mate)?;
                let x = point(pb, p);
                rows.extend(meet(size, (index[a] * 6, pa), (index[b] * 6, pb), x, (0..3).filter(|k| da[*k] == 0)));
            }
            Constraint::Points { a, b, point_b, .. } => {
                let (pa, pb) = (&bodies[a], &bodies[b]);
                let x = point(pb, point_b);
                rows.extend(meet(size, (index[a] * 6, pa), (index[b] * 6, pb), x, 0..3));
            }
            Constraint::FromPlane { mate, a, plane, b, point: p, direction, upto, .. } => {
                // d/dt of n · (x - t_a), with n turning with a and x moving with b. A range
                // bounds it and fixes nothing.
                let (pa, pb) = (&bodies[a], &bodies[b]);
                let n = vector(world(pa.rotation, plane.normal, mate)?);
                let x = point(pb, p);
                let (at_a, at_b) = (index[a] * 6, index[b] * 6);
                if upto.is_none() {
                    let (lever_a, lever_b) = (n.cross(x - pa.translation), (x - pb.translation).cross(n));
                    let mut entries = Vec::new();
                    for k in 0..3 {
                        entries.push((at_b + k, n[k]));
                        entries.push((at_a + k, -n[k]));
                        entries.push((at_a + 3 + k, lever_a[k]));
                        entries.push((at_b + 3 + k, lever_b[k]));
                    }
                    rows.push(row(size, &entries));
                }
                if let Some(d) = direction {
                    let c = n.cross(vector(world(pb.rotation, *d, mate)?));
                    let entries: Vec<_> = (0..3).flat_map(|k| [(at_a + 3 + k, c[k]), (at_b + 3 + k, -c[k])]).collect();
                    rows.push(row(size, &entries));
                }
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
            Constraint::Symmetric { mate, a, b, s, plane, ends, sign } => {
                rows.extend(mirrored(size, index, bodies, (mate, a, b, s), plane, ends, f64::from(*sign))?);
            }
            Constraint::Coupler { mate, a, b, ratio, reverse, .. } => {
                // The rate of c n · (x - t_from) for each side, with n and x on its body: v
                // along n, and a turn about n x (t_body - t_from).
                let ca = if *reverse { nearest(&ratio[1]) } else { -nearest(&ratio[1]) };
                let mut entries = Vec::new();
                for (side, c) in [(b, nearest(&ratio[0])), (a, ca)] {
                    let at = &bodies[&side.body];
                    let n = vector(world(at.rotation, side.direction, mate)?);
                    let origin = match &side.from {
                        Some(from) => bodies[from].translation,
                        None => DVec3::from_array(side.frame.each_ref().map(nearest)),
                    };
                    let (e, lever) = (index[&side.body] * 6, n.cross(at.translation - origin));
                    for k in 0..3 {
                        entries.push((e + k, c * n[k]));
                        entries.push((e + 3 + k, c * lever[k]));
                    }
                    if let Some(from) = &side.from {
                        let f = index[from] * 6;
                        entries.extend((0..3).map(|k| (f + k, -c * n[k])));
                    }
                }
                rows.push(row(size, &entries));
            }
            Constraint::Slot { mate, a, slot, b, axis: line, mode, .. } => {
                let (pa, pb) = (&bodies[a], &bodies[b]);
                let d = world(pa.rotation, slot.depth, mate)?;
                let (at_a, at_b) = (index[a] * 6, index[b] * 6);
                for e in (0..3).filter(|k| d[*k] == 0) {
                    rows.push(row(size, &[(at_b + 3 + e, 1.0), (at_a + 3 + e, -1.0)]));
                }
                // Across the slot the pin is held; along it only a position mode holds it.
                let (first, second) = (point(pa, &slot.ends[0]), point(pa, &slot.ends[1]));
                let across: Vec<usize> = (0..3).filter(|k| d[*k] == 0).collect();
                let along = *across
                    .iter()
                    .max_by(|j, k| (first[**j] - second[**j]).abs().total_cmp(&(first[**k] - second[**k]).abs()))
                    .expect("two axes across a signed axis");
                let held = across.into_iter().filter(|k| *k != along || *mode != SlotMode::Free);
                rows.extend(meet(size, (at_a, pa), (at_b, pb), point(pb, &line.through), held));
            }
            Constraint::Spin { mate, a, b, normal, .. } => {
                // The turn about the shared normal: (w_b - w_a) · n.
                let n = vector(world(bodies[a].rotation, *normal, mate)?);
                let (at_a, at_b) = (index[a] * 6, index[b] * 6);
                let entries: Vec<_> = (0..3).flat_map(|k| [(at_b + 3 + k, n[k]), (at_a + 3 + k, -n[k])]).collect();
                rows.push(row(size, &entries));
            }
        }
    }
    Ok(rows)
}

/// Adds `c` times the rate of `n · x` to `entries`, as the body at column `at`, placed at
/// `p`, carries the point `x` along: its translation along `n` and its turn about
/// `(x - t) x n`.
fn carried(entries: &mut Vec<(usize, f64)>, c: f64, n: DVec3, x: DVec3, at: usize, p: &Placement) {
    let lever = (x - p.translation).cross(n);
    for k in 0..3 {
        entries.push((at + k, c * n[k]));
        entries.push((at + 3 + k, c * lever[k]));
    }
}

/// Adds the rate of `n · y` to `entries` as the body at column `at` turns `n`: its turn
/// about `n x y`.
fn turning(entries: &mut Vec<(usize, f64)>, n: DVec3, y: DVec3, at: usize) {
    let lever = n.cross(y);
    entries.extend((0..3).map(|k| (at + 3 + k, lever[k])));
}

/// The rows of a Symmetric: `b`'s normal or direction is `sign` times `a`'s mirrored across
/// `s`'s plane normal `m`, `M n_a` with `M = I - 2 m mᵀ`, and `b`'s plane, line or point is
/// `a`'s mirrored.
fn mirrored(
    size: usize,
    index: &BTreeMap<Body, usize>,
    bodies: &BTreeMap<Body, Placement>,
    (mate, a, b, s): (&str, &Body, &Body, &Body),
    plane: &LocalPlane,
    ends: &Mirrored,
    sign: f64,
) -> Result<Vec<Vec<f64>>, MateRefusal> {
    let (pa, pb, ps) = (&bodies[a], &bodies[b], &bodies[s]);
    let (at_a, at_b, at_s) = (index[a] * 6, index[b] * 6, index[s] * 6);
    let m = vector(world(ps.rotation, plane.normal, mate)?);
    let reflect = |v: DVec3| v - 2.0 * v.dot(m) * m;
    let unit = [DVec3::X, DVec3::Y, DVec3::Z];
    let mut rows = Vec::new();
    let directions = match ends {
        Mirrored::Planes(x, y) => Some((x.normal, y.normal)),
        Mirrored::Axes(x, y) => Some((x.direction, y.direction)),
        Mirrored::Points(..) => None,
    };
    let mut normals = None;
    if let Some((u, w)) = directions {
        // The rate of n_b - sign M n_a, one row per world axis e_k: w_b · (n_b x e_k)
        // - sign w_a · (n_a x M e_k) + 2 sign w_s · ((m · n_a) (m x e_k) + m_k (m x n_a)).
        let (na, nb) = (vector(world(pa.rotation, u, mate)?), vector(world(pb.rotation, w, mate)?));
        for (k, e) in unit.into_iter().enumerate() {
            let (cb, ca) = (nb.cross(e), -sign * na.cross(reflect(e)));
            let cs = 2.0 * sign * (m.dot(na) * m.cross(e) + m[k] * m.cross(na));
            let entries: Vec<_> =
                (0..3).flat_map(|i| [(at_b + 3 + i, cb[i]), (at_a + 3 + i, ca[i]), (at_s + 3 + i, cs[i])]).collect();
            rows.push(row(size, &entries));
        }
        normals = Some((na, nb));
    }
    let on_mirror = ps.translation + m * nearest(&plane.offset);
    match ends {
        Mirrored::Planes(x, y) => {
            let (na, nb) = normals.expect("planes have normals");
            let (xa, xb) = (pa.translation + na * nearest(&x.offset), pb.translation + nb * nearest(&y.offset));
            let mut entries = Vec::new();
            if na.dot(m).abs() > 0.5 {
                // The rate of m · (x_a + x_b - 2 x_s), with m turning with s.
                carried(&mut entries, 1.0, m, xa, at_a, pa);
                carried(&mut entries, 1.0, m, xb, at_b, pb);
                carried(&mut entries, -2.0, m, on_mirror, at_s, ps);
                turning(&mut entries, m, xa + xb - 2.0 * on_mirror, at_s);
            } else {
                // A plane square to the mirror is its own image: n_a · (x_b - x_a) with n_a
                // turning with a.
                carried(&mut entries, 1.0, na, xb, at_b, pb);
                carried(&mut entries, -1.0, na, xa, at_a, pa);
                turning(&mut entries, na, xb - xa, at_a);
            }
            rows.push(row(size, &entries));
        }
        Mirrored::Points(x, y) => rows.extend(reflected(
            size,
            (at_a, pa, point(pa, x)),
            (at_b, pb, point(pb, y)),
            (at_s, ps, on_mirror),
            m,
            0..3,
        )),
        Mirrored::Axes(x, y) => {
            let (na, nb) = normals.expect("lines have directions");
            let (xa, mut xb) = (point(pa, &x.through), point(pb, &y.through));
            let across = (0..3).filter(move |k| nb[*k].abs() < 0.5);
            if na.dot(m).abs() > 0.5 {
                // A line along the mirror's normal is its own image.
                rows.extend(meet(size, (at_a, pa), (at_b, pb), xb, across));
            } else {
                // The point of b's line nearest a's mirrored point, so nothing is left along it.
                let image = xa - 2.0 * m.dot(xa - on_mirror) * m;
                xb += nb * nb.dot(image - xb);
                rows.extend(reflected(size, (at_a, pa, xa), (at_b, pb, xb), (at_s, ps, on_mirror), m, across));
            }
        }
    }
    Ok(rows)
}

/// The rows keeping `x_b` on `x_a` mirrored across the plane through `x_s` with normal `m`,
/// along each world axis in `along`: the rate of x_a - 2 (m · (x_a - x_s)) m - x_b.
fn reflected(
    size: usize,
    (at_a, pa, xa): (usize, &Placement, DVec3),
    (at_b, pb, xb): (usize, &Placement, DVec3),
    (at_s, ps, xs): (usize, &Placement, DVec3),
    m: DVec3,
    along: impl Iterator<Item = usize>,
) -> Vec<Vec<f64>> {
    let h = m.dot(xa - xs);
    let unit = [DVec3::X, DVec3::Y, DVec3::Z];
    along
        .map(|k| {
            let mut entries = Vec::new();
            carried(&mut entries, 1.0, unit[k] - 2.0 * m[k] * m, xa, at_a, pa);
            carried(&mut entries, -1.0, unit[k], xb, at_b, pb);
            carried(&mut entries, 2.0 * m[k], m, xs, at_s, ps);
            turning(&mut entries, m, -2.0 * m[k] * (xa - xs) - 2.0 * h * unit[k], at_s);
            row(size, &entries)
        })
        .collect()
}

/// A point of a body at its placement, in the root's frame.
fn point(at: &Placement, p: &LocalPoint) -> DVec3 {
    at.rotation * DVec3::new(nearest(&p[0]), nearest(&p[1]), nearest(&p[2])) + at.translation
}

/// The rows that keep a point `x` of `b` on the matching point of `a` along each world
/// axis in `along`: d/dt of x_b - x_a is v_b + w_b x (x - t_b) - v_a - w_a x (x - t_a).
fn meet(
    size: usize,
    (at_a, pa): (usize, &Placement),
    (at_b, pb): (usize, &Placement),
    x: DVec3,
    along: impl Iterator<Item = usize>,
) -> Vec<Vec<f64>> {
    let (ra, rb) = (x - pa.translation, x - pb.translation);
    along
        .map(|k| {
            let mut entries = vec![(at_b + k, 1.0), (at_a + k, -1.0)];
            for (m, e) in [DVec3::X, DVec3::Y, DVec3::Z].into_iter().enumerate() {
                entries.push((at_b + 3 + m, e.cross(rb)[k]));
                entries.push((at_a + 3 + m, -e.cross(ra)[k]));
            }
            row(size, &entries)
        })
        .collect()
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

    /// A part tilted off the axis it is held along is turned onto it exactly, and keeps
    /// the spin about it, an eighth of a turn here, that nothing holds; one already on the
    /// axis keeps its spin as it is.
    #[test]
    fn a_tilted_part_keeps_its_free_spin_exactly() {
        let mut at = bodies();
        let eighth = DQuat::from_rotation_y(0.1) * DQuat::from_rotation_z(std::f64::consts::FRAC_PI_4);
        at.get_mut(&b()).unwrap().rotation = eighth;
        let constraints = [origin(true), parallel("P", plane([0, 0, 1], 10), plane([0, 0, -1], 0), -1)];
        let solved = solve(&at, &constraints).unwrap();
        let r = solved[&b()].rotation;
        assert_eq!(turned(r, [0, 0, -1]), Some([0, 0, -1]), "{r}");
        let h = std::f64::consts::FRAC_1_SQRT_2;
        assert!((r * DVec3::X).distance(DVec3::new(h, h, 0.0)) < 1e-12, "{r}: the eighth turn stays");
        assert_eq!(freedoms(&solved, &constraints).unwrap().0, 4);
        let spin = DQuat::from_rotation_z(0.5);
        at.get_mut(&b()).unwrap().rotation = spin;
        let r = solve(&at, &constraints).unwrap()[&b()].rotation;
        assert!([DVec3::X, DVec3::Y].iter().all(|v| (r * *v).distance(spin * *v) < 1e-15), "{r}");
        assert_eq!(turned(r, [0, 0, 1]), Some([0, 0, 1]));
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
        assert!(same_turn(placed.rotation, q90z()));
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

    /// A turn is on an axis only when its quaternion, read as rationals, puts it there:
    /// every quarter turn and every spin about its own axis is, and a turn within the old
    /// tolerance of an axis is not.
    #[test]
    fn only_exact_turns_land_on_axes() {
        for m in &cube_rotations() {
            for q in [quaternion(m), -quaternion(m), quaternion(m).normalize()] {
                for (c, axis) in [[1, 0, 0], [0, 1, 0], [0, 0, 1]].into_iter().enumerate() {
                    assert_eq!(turned(q, axis), Some([m[0][c], m[1][c], m[2][c]]), "{q} on {axis:?}");
                }
            }
        }
        for angle in [0.1, 0.5, 1.0, 2.5] {
            let spin = DQuat::from_rotation_z(angle);
            assert_eq!(turned(spin, [0, 0, 1]), Some([0, 0, 1]));
            assert_eq!(turned(spin, [1, 0, 0]), None);
        }
        // Tilted 2^-42 about y: -z lands 4.5e-13 from -z, which the old test took as -z.
        let tilt = DQuat::from_xyzw(0.0, -(2f64.powi(-42)), 0.0, 1.0).normalize();
        assert!(snapped(tilt * DVec3::NEG_Z).is_some());
        assert_eq!(turned(tilt, [0, 0, -1]), None);
        assert_eq!(turned(tilt, [0, 1, 0]), Some([0, 1, 0]), "y is the turn's own axis");
        assert_eq!(cube_of(tilt), None);
        let refused = world(tilt, [0, 0, -1], "M").unwrap_err();
        assert_eq!(refused.code, RefusalCode::UnsupportedPlacement);
    }

    /// A locked pair turns as one: a Parallel on one member and a Perpendicular on the
    /// other leave it a quarter turn about z, whichever member comes first.
    #[test]
    fn a_locked_group_turns_as_one_whatever_the_order() {
        for (b_id, c_id) in [(4, 6), (6, 3)] {
            let (pa, pb, pc) = (body(2), body(b_id), body(c_id));
            let at: BTreeMap<Body, Placement> = [
                (pa.clone(), Placement::IDENTITY),
                (pb.clone(), Placement::translation(DVec3::new(10.0, 0.0, 0.0))),
                (pc.clone(), Placement::translation(DVec3::new(10.0, 5.0, 0.0))),
            ]
            .into();
            let lock = Constraint::Lock {
                mate: "L".into(),
                a: pb.clone(),
                b: pc.clone(),
                relative: Placement::translation(DVec3::new(0.0, 5.0, 0.0)),
            };
            let p = Constraint::Parallel {
                mate: "P".into(),
                a: pa.clone(),
                b: pb.clone(),
                plane_a: plane([0, 0, 1], 0),
                plane_b: plane([0, 0, 1], 0),
                sign: 1,
                along: None,
            };
            let r = Constraint::Perpendicular {
                mate: "R".into(),
                a: pa.clone(),
                b: pc.clone(),
                plane_a: plane([1, 0, 0], 0),
                plane_b: plane([1, 0, 0], 0),
            };
            let origin = Constraint::Origin { mate: "O".into(), body: pa.clone(), axes: true, at: Placement::IDENTITY };
            let constraints = [origin, lock, p, r];
            let solved = solve(&at, &constraints).unwrap_or_else(|e| panic!("B {b_id}, C {c_id}: {e}"));
            let (qb, qc) = (solved[&pb].rotation, solved[&pc].rotation);
            assert!(same_turn(qb, qc), "B {b_id}, C {c_id}: the lock keeps them together");
            assert_eq!(snapped(qb * DVec3::Z), Some([0, 0, 1]));
            assert_eq!(snapped(qc * DVec3::X).map(|n| n[0]), Some(0), "C's x is at right angles to A's");
            // C sits at B's placement composed with the lock: [0, 5, 0] turned a quarter.
            let offset = solved[&pc].translation - solved[&pb].translation;
            assert!(offset.distance(qb * DVec3::new(0.0, 5.0, 0.0)) < 1e-12, "{offset}");
            assert_eq!(snapped(offset / 5.0).map(|n| n[1]), Some(0));
            // The pair moves as one body: P takes two of its turns and R the third, so its
            // three translations stay.
            assert_eq!(freedoms(&solved, &constraints).unwrap().0, 3, "B {b_id}, C {c_id}");
        }
    }

    /// A locked group whose members' mates ask for incompatible turns is still refused.
    #[test]
    fn a_locked_group_with_incompatible_mates_is_refused() {
        let (pa, pb, pc) = (body(2), body(4), body(6));
        let at: BTreeMap<Body, Placement> =
            [(pa.clone(), Placement::IDENTITY), (pb.clone(), Placement::IDENTITY), (pc.clone(), Placement::IDENTITY)]
                .into();
        let parallel = |mate: &str, b: &Body, sign| Constraint::Parallel {
            mate: mate.into(),
            a: pa.clone(),
            b: b.clone(),
            plane_a: plane([0, 0, 1], 0),
            plane_b: plane([0, 0, 1], 0),
            sign,
            along: None,
        };
        let lock = Constraint::Lock { mate: "L".into(), a: pb.clone(), b: pc.clone(), relative: Placement::IDENTITY };
        let origin = Constraint::Origin { mate: "O".into(), body: pa.clone(), axes: true, at: Placement::IDENTITY };
        let constraints = [origin, lock, parallel("Up", &pb, 1), parallel("Down", &pc, -1)];
        let refused = solve(&at, &constraints).unwrap_err();
        assert_eq!(refused.code, RefusalCode::OverConstrained);
        assert_eq!(refused.mates, ["Down", "Up"]);
    }

    /// Every order of a list, for the permutation checks.
    fn orders<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
        if items.len() <= 1 {
            return vec![items.to_vec()];
        }
        let mut out = Vec::new();
        for i in 0..items.len() {
            let mut rest = items.to_vec();
            let first = rest.remove(i);
            for mut tail in orders(&rest) {
                tail.insert(0, first.clone());
                out.push(tail);
            }
        }
        out
    }

    /// A on the origin, B's top on A's, C's right side on B's, and C's right side at right
    /// angles to A's: B and C turn a quarter about z together. No mate order or id order
    /// may make the solver freeze B's spin first and refuse.
    #[test]
    fn a_free_spin_is_solved_jointly_in_every_order() {
        for (b_id, c_id) in [(4, 6), (6, 3)] {
            let (pa, pb, pc) = (body(2), body(b_id), body(c_id));
            let at: BTreeMap<Body, Placement> = [
                (pa.clone(), Placement::IDENTITY),
                (pb.clone(), Placement::translation(DVec3::new(10.0, 0.0, 0.0))),
                (pc.clone(), Placement::translation(DVec3::new(20.0, 0.0, 0.0))),
            ]
            .into();
            let between = |mate: &str, a: &Body, b: &Body, na, nb, sign| Constraint::Parallel {
                mate: mate.into(),
                a: a.clone(),
                b: b.clone(),
                plane_a: plane(na, 0),
                plane_b: plane(nb, 0),
                sign,
                along: None,
            };
            let mates = [
                Constraint::Origin { mate: "O".into(), body: pa.clone(), axes: true, at: Placement::IDENTITY },
                between("P", &pa, &pb, [0, 0, 1], [0, 0, 1], 1),
                between("Q", &pb, &pc, [1, 0, 0], [1, 0, 0], 1),
                Constraint::Perpendicular {
                    mate: "R".into(),
                    a: pa.clone(),
                    b: pc.clone(),
                    plane_a: plane([1, 0, 0], 0),
                    plane_b: plane([1, 0, 0], 0),
                },
            ];
            let mut answers = Vec::new();
            for order in orders(&mates) {
                let solved = solve(&at, &order).unwrap_or_else(|e| panic!("B {b_id}, C {c_id}, {order:?}: {e}"));
                let (qb, qc) = (solved[&pb].rotation, solved[&pc].rotation);
                assert_eq!(snapped(qb * DVec3::Z), Some([0, 0, 1]));
                assert_eq!(snapped(qc * DVec3::X), snapped(qb * DVec3::X), "Q holds");
                assert_eq!(snapped(qc * DVec3::X).map(|n| n[0]), Some(0), "R holds");
                answers.push((qb.to_array(), qc.to_array()));
            }
            answers.dedup();
            assert_eq!(answers.len(), 1, "B {b_id}, C {c_id}: one answer for every order: {answers:?}");
        }
    }

    /// A conflict is named by a smallest set of mates that cannot hold, the same in every
    /// order: here P and R, while Q, which agrees with both, is left out.
    #[test]
    fn a_conflict_names_the_same_smallest_set_in_every_order() {
        let (pa, pb) = (body(2), body(4));
        let at: BTreeMap<Body, Placement> =
            [(pa.clone(), Placement::IDENTITY), (pb.clone(), Placement::IDENTITY)].into();
        let p = |mate: &str, n: Normal| Constraint::Parallel {
            mate: mate.into(),
            a: pa.clone(),
            b: pb.clone(),
            plane_a: plane(n, 0),
            plane_b: plane(n, 0),
            sign: 1,
            along: None,
        };
        let mates = [
            Constraint::Origin { mate: "O".into(), body: pa.clone(), axes: true, at: Placement::IDENTITY },
            p("P", [0, 0, 1]),
            p("Q", [1, 0, 0]),
            Constraint::Perpendicular {
                mate: "R".into(),
                a: pa.clone(),
                b: pb.clone(),
                plane_a: plane([0, 0, 1], 0),
                plane_b: plane([0, 0, 1], 0),
            },
        ];
        for order in orders(&mates) {
            let refused = solve(&at, &order).unwrap_err();
            assert_eq!(
                (refused.code, refused.mates.clone()),
                (RefusalCode::OverConstrained, vec!["P".to_owned(), "R".to_owned()])
            );
        }
    }

    /// Two parts on their own frames' origins, the second frame 10 mm along x, and an
    /// aligned distance between their z = 0 planes.
    fn frames_apart(gap: f64, a_id: u32, b_id: u32) -> (BTreeMap<Body, Placement>, Vec<Constraint>) {
        let (pa, pb) = (body(a_id), body(b_id));
        let frame = Placement::translation(DVec3::new(10.0, 0.0, 0.0));
        let at: BTreeMap<Body, Placement> = [(pa.clone(), Placement::IDENTITY), (pb.clone(), frame)].into();
        let mates = vec![
            Constraint::Origin { mate: "OA".into(), body: pa.clone(), axes: false, at: Placement::IDENTITY },
            Constraint::Origin { mate: "OB".into(), body: pb.clone(), axes: false, at: frame },
            Constraint::Parallel {
                mate: "D".into(),
                a: pa,
                b: pb,
                plane_a: plane([0, 0, 1], 0),
                plane_b: plane([0, 0, 1], 0),
                sign: 1,
                along: Some(Along::Distance(gap)),
            },
        ];
        (at, mates)
    }

    /// Kept as they are, both origins put z at 0 and the distance cannot hold. Turning both
    /// parts so their z faces world x holds all three exactly, so the solver must find that
    /// turn rather than stop at the first one that keeps the normals.
    #[test]
    fn a_distance_turns_the_parts_when_their_turn_alone_cannot_place_them() {
        for (a_id, b_id) in [(2, 4), (4, 2)] {
            let (at, mates) = frames_apart(10.0, a_id, b_id);
            let mut answers = Vec::new();
            for order in orders(&mates) {
                let solved = solve(&at, &order).unwrap_or_else(|e| panic!("A {a_id}, B {b_id}, {order:?}: {e}"));
                let (qa, qb) = (solved[&body(a_id)].rotation, solved[&body(b_id)].rotation);
                assert_eq!(snapped(qa * DVec3::Z), Some([1, 0, 0]), "A's top faces world x");
                assert_eq!(snapped(qb * DVec3::Z), Some([1, 0, 0]), "B's top faces world x");
                assert_eq!(solved[&body(a_id)].translation, DVec3::ZERO, "OA holds");
                assert_eq!(solved[&body(b_id)].translation, DVec3::new(10.0, 0.0, 0.0), "OB holds");
                answers.push((qa.to_array(), qb.to_array()));
            }
            answers.dedup();
            assert_eq!(answers.len(), 1, "A {a_id}, B {b_id}: one answer for every order: {answers:?}");
        }
    }

    /// The frames are 10 mm apart, so no turn holds an 11 mm distance: still refused, with
    /// every mate on the cycle named.
    #[test]
    fn a_distance_no_turn_can_hold_is_still_over_constrained() {
        for (a_id, b_id) in [(2, 4), (4, 2)] {
            let (at, mates) = frames_apart(11.0, a_id, b_id);
            for order in orders(&mates) {
                let refused = solve(&at, &order).unwrap_err();
                assert_eq!(refused.code, RefusalCode::OverConstrained, "{order:?}");
                assert_eq!(refused.mates, ["D", "OA", "OB"], "{order:?}");
                assert!(refused.message.contains("D (11 mm)"), "{refused}");
            }
        }
    }

    /// Once a third part holds A's top up, the only turn that would place the distance is
    /// gone. The refusal names every mate it needs, and says what the nearest pose asks.
    #[test]
    fn a_distance_whose_turn_another_mate_forbids_names_both() {
        let (mut at, mut mates) = frames_apart(10.0, 2, 4);
        let pc = body(6);
        at.insert(pc.clone(), Placement::translation(DVec3::new(0.0, 50.0, 0.0)));
        mates.push(Constraint::Origin {
            mate: "OC".into(),
            body: pc.clone(),
            axes: true,
            at: Placement::translation(DVec3::new(0.0, 50.0, 0.0)),
        });
        mates.push(Constraint::Parallel {
            mate: "P".into(),
            a: pc,
            b: body(2),
            plane_a: plane([0, 0, 1], 0),
            plane_b: plane([0, 0, 1], 0),
            sign: 1,
            along: None,
        });
        let refused = solve(&at, &mates).unwrap_err();
        assert_eq!(refused.code, RefusalCode::OverConstrained);
        assert_eq!(refused.mates, ["D", "OA", "OB", "OC", "P"]);
        assert!(refused.message.contains("no quarter-turn pose") && refused.message.contains("D (10 mm)"), "{refused}");
    }

    /// A on the root's origin, B on its frame's origin `frame_y` along y, and B locked
    /// 10 mm along A's x. No mate relates their normals; `parallel` adds one that the lock
    /// already implies.
    fn locked_apart(
        frame_y: f64,
        parallel: bool,
        a_id: u32,
        b_id: u32,
    ) -> (BTreeMap<Body, Placement>, Vec<Constraint>) {
        let (pa, pb) = (body(a_id), body(b_id));
        let frame = Placement::translation(DVec3::new(0.0, frame_y, 0.0));
        let at: BTreeMap<Body, Placement> = [(pa.clone(), Placement::IDENTITY), (pb.clone(), frame)].into();
        let mut mates = vec![
            Constraint::Origin { mate: "OA".into(), body: pa.clone(), axes: false, at: Placement::IDENTITY },
            Constraint::Origin { mate: "OB".into(), body: pb.clone(), axes: false, at: frame },
            Constraint::Lock {
                mate: "L".into(),
                a: pa.clone(),
                b: pb.clone(),
                relative: Placement::translation(DVec3::new(10.0, 0.0, 0.0)),
            },
        ];
        if parallel {
            mates.push(Constraint::Parallel {
                mate: "P".into(),
                a: pa,
                b: pb,
                plane_a: plane([0, 0, 1], 0),
                plane_b: plane([0, 0, 1], 0),
                sign: 1,
                along: None,
            });
        }
        (at, mates)
    }

    /// The lock's 10 mm along A's x can only land on B's frame, 10 mm along y, once the
    /// pair turns a quarter. No mate relates their normals, so only the lock asks for that
    /// turn; the search must still try it, with or without a redundant Parallel.
    #[test]
    fn a_lock_turns_its_group_when_only_a_turn_places_it() {
        for parallel in [false, true] {
            for (a_id, b_id) in [(2, 4), (4, 2)] {
                let (at, mates) = locked_apart(10.0, parallel, a_id, b_id);
                let mut answers = Vec::new();
                for order in orders(&mates) {
                    let solved = solve(&at, &order).unwrap_or_else(|e| panic!("A {a_id}, B {b_id}, {order:?}: {e}"));
                    let (qa, qb) = (solved[&body(a_id)].rotation, solved[&body(b_id)].rotation);
                    assert_eq!(snapped(qa * DVec3::X), Some([0, 1, 0]), "A's x faces world y");
                    assert!(same_turn(qa, qb), "the lock keeps them together");
                    assert_eq!(solved[&body(a_id)].translation, DVec3::ZERO, "OA holds");
                    assert_eq!(solved[&body(b_id)].translation, DVec3::new(0.0, 10.0, 0.0), "OB holds");
                    answers.push((qa.to_array(), qb.to_array()));
                }
                answers.dedup();
                assert_eq!(answers.len(), 1, "A {a_id}, B {b_id}: one answer for every order: {answers:?}");
            }
        }
    }

    /// No turn puts a 10 mm lock offset on a frame 11 mm away: still refused.
    #[test]
    fn a_lock_no_turn_can_place_is_still_over_constrained() {
        for parallel in [false, true] {
            for (a_id, b_id) in [(2, 4), (4, 2)] {
                let (at, mates) = locked_apart(11.0, parallel, a_id, b_id);
                for order in orders(&mates) {
                    let refused = solve(&at, &order).unwrap_err();
                    assert_eq!(refused.code, RefusalCode::OverConstrained, "{order:?}");
                    assert_eq!(refused.mates, ["L", "OA", "OB"], "{order:?}");
                }
            }
        }
    }

    /// Many parts that keep a free spin each, then a conflict found only when the last part
    /// is turned. Spins that read the same axes are tried once, so the search still ends
    /// in a few steps and names the conflict instead of giving up as too large.
    #[test]
    fn a_late_conflict_behind_many_free_spins_is_named_quickly() {
        // The last part in path order, so it is turned after every spinning one.
        let last = body(200);
        let mut at: BTreeMap<Body, Placement> =
            [(a(), Placement::IDENTITY), (last.clone(), Placement::translation(DVec3::new(0.0, 0.0, 30.0)))].into();
        let gap = |mate: &str, d: f64| Constraint::Parallel {
            mate: mate.into(),
            a: a(),
            b: last.clone(),
            plane_a: plane([0, 0, 1], 10),
            plane_b: plane([0, 0, -1], 0),
            sign: -1,
            along: Some(Along::Distance(d)),
        };
        let mut mates = vec![origin(true), gap("Z", 5.0), gap("Conflict", 6.0)];
        for id in 100..112 {
            let spin = body(id);
            at.insert(spin.clone(), Placement::translation(DVec3::new(f64::from(id), 0.0, 0.0)));
            mates.push(Constraint::Parallel {
                mate: format!("S{id}"),
                a: a(),
                b: spin,
                plane_a: plane([0, 0, 1], 0),
                plane_b: plane([0, 0, 1], 0),
                sign: 1,
                along: None,
            });
        }
        let refused = solve(&at, &mates).unwrap_err();
        assert_eq!(
            (refused.code, refused.mates.clone()),
            (RefusalCode::OverConstrained, vec!["Conflict".to_owned(), "Z".to_owned()])
        );
    }

    fn line(direction: Normal, through: [i64; 3]) -> LocalAxis {
        LocalAxis { direction, through: through.map(q) }
    }

    /// A bore along +Z through (10, 5) on A and a shaft along +Z through B's origin.
    fn concentric(mate: &str, bore: [i64; 3], shaft: [i64; 3]) -> Constraint {
        Constraint::Axes {
            mate: mate.into(),
            a: a(),
            b: b(),
            axis_a: line([0, 0, 1], bore),
            axis_b: line([0, 0, 1], shaft),
            sign: 1,
            coaxial: true,
        }
    }

    #[test]
    fn a_shaft_in_a_bore_slides_and_spins_on_the_axis_exactly() {
        let constraints = [origin(true), concentric("C", [10, 5, 0], [0, 0, 0])];
        let solved = solve(&bodies(), &constraints).unwrap();
        assert_eq!(solved[&b()], Placement::translation(DVec3::new(10.0, 5.0, 30.0)));
        let (dof, free) = freedoms(&solved, &constraints).unwrap();
        assert_eq!(dof, 2);
        let names: Vec<_> = free.iter().map(|f| f.motion.as_str()).collect();
        assert_eq!(names, ["translate_z", "spin_z"]);
    }

    #[test]
    fn a_locked_rotation_leaves_only_the_slide() {
        let turn = Constraint::Turn { mate: "C".into(), a: a(), b: b(), relative: DQuat::IDENTITY };
        let constraints = [origin(true), concentric("C", [10, 5, 0], [0, 0, 0]), turn];
        let solved = solve(&bodies(), &constraints).unwrap();
        let (dof, free) = freedoms(&solved, &constraints).unwrap();
        assert_eq!(dof, 1);
        assert_eq!(free.iter().map(|f| f.motion.as_str()).collect::<Vec<_>>(), ["translate_z"]);
    }

    #[test]
    fn a_locked_rotation_turns_the_part_as_stored() {
        let turn = Constraint::Turn { mate: "C".into(), a: a(), b: b(), relative: q90z() };
        let constraints = [origin(true), concentric("C", [10, 5, 0], [0, 0, 0]), turn];
        let solved = solve(&bodies(), &constraints).unwrap();
        assert!(same_turn(solved[&b()].rotation, q90z()));
        assert_eq!(solved[&b()].translation, DVec3::new(10.0, 5.0, 30.0));
    }

    #[test]
    fn two_pins_leave_one_slide_and_one_pin_twice_leaves_the_spin() {
        let two = [origin(true), concentric("C1", [10, 5, 0], [0, 0, 0]), concentric("C2", [-10, 5, 0], [-20, 0, 0])];
        let solved = solve(&bodies(), &two).unwrap();
        assert_eq!(solved[&b()].translation, DVec3::new(10.0, 5.0, 30.0));
        assert_eq!(freedoms(&solved, &two).unwrap().0, 1);
        let same = [origin(true), concentric("C1", [10, 5, 0], [0, 0, 0]), concentric("C2", [10, 5, 9], [0, 0, 4])];
        let solved = solve(&bodies(), &same).unwrap();
        assert_eq!(freedoms(&solved, &same).unwrap().0, 2, "a redundant axis that agrees removes nothing");
    }

    #[test]
    fn a_second_pin_that_disagrees_is_named() {
        let constraints =
            [origin(true), concentric("C1", [10, 5, 0], [0, 0, 0]), concentric("C2", [-10, 5, 0], [-21, 0, 0])];
        let refused = solve(&bodies(), &constraints).unwrap_err();
        assert_eq!((refused.code, refused.mates), (RefusalCode::OverConstrained, vec!["C1".into(), "C2".into()]));
    }

    #[test]
    fn concentric_turns_a_cross_shaft_onto_the_bore() {
        // B's shaft runs along its own X; the bore runs along Z, so B takes a quarter turn.
        let c = Constraint::Axes {
            mate: "C".into(),
            a: a(),
            b: b(),
            axis_a: line([0, 0, 1], [10, 5, 0]),
            axis_b: line([1, 0, 0], [0, 2, 3]),
            sign: 1,
            coaxial: true,
        };
        let constraints = [origin(true), c];
        let solved = solve(&bodies(), &constraints).unwrap();
        let p = solved[&b()];
        assert_eq!(world(p.rotation, [1, 0, 0], "C").unwrap(), [0, 0, 1]);
        let through = p.rotation * DVec3::new(0.0, 2.0, 3.0) + p.translation;
        assert_eq!((through.x, through.y), (10.0, 5.0));
        assert_eq!(freedoms(&solved, &constraints).unwrap().0, 2);
    }

    #[test]
    fn axes_held_parallel_or_square_fix_turns_only() {
        let parallel = Constraint::Axes {
            mate: "P".into(),
            a: a(),
            b: b(),
            axis_a: line([0, 0, 1], [10, 5, 0]),
            axis_b: line([0, 0, 1], [0, 0, 0]),
            sign: -1,
            coaxial: false,
        };
        let solved = solve(&bodies(), &[origin(true), parallel.clone()]).unwrap();
        assert_eq!(world(solved[&b()].rotation, [0, 0, 1], "P").unwrap(), [0, 0, -1]);
        assert_eq!(solved[&b()].translation, DVec3::new(10.0, 10.0, 30.0));
        assert_eq!(freedoms(&solved, &[origin(true), parallel]).unwrap().0, 4);
        let square = Constraint::Axes {
            mate: "S".into(),
            a: a(),
            b: b(),
            axis_a: line([0, 0, 1], [10, 5, 0]),
            axis_b: line([0, 0, 1], [0, 0, 0]),
            sign: 0,
            coaxial: false,
        };
        let solved = solve(&bodies(), &[origin(true), square.clone()]).unwrap();
        assert_eq!(world(solved[&b()].rotation, [0, 0, 1], "S").unwrap()[2], 0);
        assert_eq!(freedoms(&solved, &[origin(true), square]).unwrap().0, 5);
    }

    #[test]
    fn a_point_on_a_point_and_on_an_axis() {
        let points = Constraint::Points {
            mate: "P".into(),
            a: a(),
            point_a: [20, 15, 10].map(q),
            b: b(),
            point_b: [-10, -5, 0].map(q),
        };
        let solved = solve(&bodies(), &[origin(true), points.clone()]).unwrap();
        assert_eq!(solved[&b()].translation, DVec3::new(30.0, 20.0, 10.0));
        assert_eq!(freedoms(&solved, &[origin(true), points]).unwrap().0, 3);
        let on = Constraint::OnAxis {
            mate: "L".into(),
            a: a(),
            axis: line([0, 0, 1], [10, 5, 0]),
            b: b(),
            point: [-10, -5, 0].map(q),
        };
        let solved = solve(&bodies(), &[origin(true), on.clone()]).unwrap();
        assert_eq!(solved[&b()].translation, DVec3::new(20.0, 10.0, 30.0));
        assert_eq!(freedoms(&solved, &[origin(true), on]).unwrap().0, 4);
    }

    #[test]
    fn a_point_or_an_axis_at_a_distance_from_a_plane() {
        let point = Constraint::FromPlane {
            mate: "D".into(),
            a: a(),
            plane: plane([0, 0, 1], 10),
            b: b(),
            point: [0, 0, 6].map(q),
            direction: None,
            gap: q(5),
            upto: None,
            says: "5 mm".into(),
        };
        let solved = solve(&bodies(), &[origin(true), point.clone()]).unwrap();
        assert_eq!(solved[&b()].translation, DVec3::new(10.0, 10.0, 9.0));
        assert_eq!(freedoms(&solved, &[origin(true), point]).unwrap().0, 5);
        // A shaft of radius 4 along B's Z lying on A's top: turned flat, its axis 4 above.
        let tangent = Constraint::FromPlane {
            mate: "T".into(),
            a: a(),
            plane: plane([0, 0, 1], 10),
            b: b(),
            point: [0, 0, 0].map(q),
            direction: Some([0, 0, 1]),
            gap: q(4),
            upto: None,
            says: "tangent".into(),
        };
        let constraints = [origin(true), tangent];
        let solved = solve(&bodies(), &constraints).unwrap();
        let p = solved[&b()];
        assert_eq!(world(p.rotation, [0, 0, 1], "T").unwrap()[2], 0);
        assert_eq!(p.translation.z, 14.0);
        assert_eq!(freedoms(&solved, &constraints).unwrap().0, 4);
    }

    #[test]
    fn a_part_whose_point_is_placed_takes_quarter_turns_only() {
        let mut at = bodies();
        let off = DQuat::from_rotation_z(0.4);
        at.get_mut(&b()).unwrap().rotation = off;
        let points = Constraint::Points {
            mate: "P".into(),
            a: a(),
            point_a: [20, 15, 10].map(q),
            b: b(),
            point_b: [-10, -5, 0].map(q),
        };
        let solved = solve(&at, &[origin(true), points]).unwrap();
        assert_eq!(solved[&b()].rotation, DQuat::IDENTITY);
        assert_eq!(solved[&b()].translation, DVec3::new(30.0, 20.0, 10.0));
        // A Parallel alone keeps the spin, as before.
        let parallel = Constraint::Parallel {
            mate: "Q".into(),
            a: a(),
            b: b(),
            plane_a: plane([0, 0, 1], 0),
            plane_b: plane([0, 0, 1], 0),
            sign: 1,
            along: None,
        };
        let solved = solve(&at, &[origin(true), parallel]).unwrap();
        assert!(same_turn(solved[&b()].rotation, off));
    }

    /// A's top is 10 mm from B's corner. Both origins hold, B's turn too, so A's top cannot
    /// face B's corner while A keeps its turn: only turning A so its top faces world x
    /// places the corner 10 mm out. No mate relates a normal of B to A's top, so the turn
    /// comes from the distance alone.
    #[test]
    fn a_point_measured_from_a_plane_turns_the_plane() {
        let corner = Placement::translation(DVec3::new(10.0, 0.0, 0.0));
        let at: BTreeMap<Body, Placement> = [(a(), Placement::IDENTITY), (b(), corner)].into();
        let mates = vec![
            Constraint::Origin { mate: "OA".into(), body: a(), axes: false, at: Placement::IDENTITY },
            Constraint::Origin { mate: "OB".into(), body: b(), axes: true, at: corner },
            Constraint::FromPlane {
                mate: "D".into(),
                a: a(),
                plane: plane([0, 0, 1], 0),
                b: b(),
                point: [0, 0, 0].map(q),
                direction: None,
                gap: q(10),
                upto: None,
                says: "10 mm".into(),
            },
        ];
        for order in orders(&mates) {
            let solved = solve(&at, &order).unwrap_or_else(|e| panic!("{order:?}: {e}"));
            assert_eq!(turned(solved[&a()].rotation, [0, 0, 1]), Some([1, 0, 0]), "{order:?}");
            assert_eq!(solved[&a()].translation, DVec3::ZERO);
            assert_eq!(solved[&b()], corner);
        }
        // 11 mm cannot hold whichever way A's top faces.
        let mut far = mates.clone();
        if let Constraint::FromPlane { gap, .. } = &mut far[2] {
            *gap = q(11);
        }
        let refused = solve(&at, &far).unwrap_err();
        assert_eq!(refused.code, RefusalCode::OverConstrained, "{refused}");
    }

    fn c() -> Body {
        body(6)
    }

    fn origin_of(mate: &str, body: Body) -> Constraint {
        Constraint::Origin { mate: mate.into(), body, axes: true, at: Placement::IDENTITY }
    }

    /// A mirror plane on C, the YZ plane through C's origin, and A and B either side of it.
    fn three(a_at: [f64; 3], b_at: [f64; 3]) -> BTreeMap<Body, Placement> {
        [
            (a(), Placement::translation(DVec3::from_array(a_at))),
            (b(), Placement::translation(DVec3::from_array(b_at))),
            (c(), Placement::IDENTITY),
        ]
        .into()
    }

    fn symmetric(ends: Mirrored, sign: i8) -> Constraint {
        Constraint::Symmetric { mate: "S".into(), a: a(), b: b(), s: c(), plane: plane([1, 0, 0], 0), ends, sign }
    }

    /// A's face 10 left of A's origin looks along -X; B's face 10 right of B's looks along +X.
    /// Mirrored about x = 0 the faces lie at -40 and +40, so B's origin lands at x = 30. B
    /// is the later body, so it is the one that moves.
    #[test]
    fn symmetric_planes_mirror_the_later_part_exactly() {
        let at = three([-30.0, 0.0, 0.0], [20.0, 5.0, -2.0]);
        let mates =
            vec![origin_of("O", c()), symmetric(Mirrored::Planes(plane([-1, 0, 0], 10), plane([1, 0, 0], 10)), 1)];
        for order in orders(&mates) {
            let solved = solve(&at, &order).unwrap();
            assert_eq!(solved[&a()], at[&a()]);
            assert_eq!(solved[&b()].translation, DVec3::new(30.0, 5.0, -2.0));
            assert_eq!(solved[&b()].rotation, DQuat::IDENTITY);
        }
        // B keeps two turns about Y and Z and its slides along Y and Z; A keeps all six.
        let solved = solve(&at, &mates).unwrap();
        assert_eq!(freedoms(&solved, &mates).unwrap().0, 9);
        // Opposed asks B's face to look along -X: B turns over.
        let flipped =
            [origin_of("O", c()), symmetric(Mirrored::Planes(plane([-1, 0, 0], 10), plane([1, 0, 0], 10)), -1)];
        let solved = solve(&at, &flipped).unwrap();
        assert_eq!(world(solved[&b()].rotation, [1, 0, 0], "S").unwrap(), [-1, 0, 0]);
        assert_eq!(solved[&b()].translation.x, 50.0);
    }

    /// Faces square to the mirror are their own images: B's face lies on A's plane, and the
    /// mirror may slide along its normal.
    #[test]
    fn symmetric_planes_square_to_the_mirror_coincide() {
        let at = three([-30.0, 0.0, 0.0], [20.0, 5.0, -2.0]);
        let mates = [origin_of("O", a()), symmetric(Mirrored::Planes(plane([0, 0, 1], 4), plane([0, 0, 1], 1)), 1)];
        let solved = solve(&at, &mates).unwrap();
        assert_eq!(solved[&b()].translation, DVec3::new(20.0, 5.0, 3.0));
    }

    #[test]
    fn symmetric_points_mirror_across_and_match_along_the_plane() {
        let at = three([-12.0, 3.0, 4.0], [9.0, -1.0, 2.0]);
        let mates = [
            origin_of("O", c()),
            origin_of("OA", a()),
            symmetric(Mirrored::Points([2, 0, 0].map(q), [0, 0, 0].map(q)), 1),
        ];
        let at = {
            let mut at = at;
            at.insert(a(), Placement::translation(DVec3::new(-12.0, 3.0, 4.0)));
            at
        };
        let held = Constraint::Origin { mate: "OA".into(), body: a(), axes: true, at: at[&a()] };
        let mates = [mates[0].clone(), held, mates[2].clone()];
        let solved = solve(&at, &mates).unwrap();
        // A's point at (-10, 3, 4) mirrors to (10, 3, 4): B's origin, since B's point is it.
        assert_eq!(solved[&b()].translation, DVec3::new(10.0, 3.0, 4.0));
        assert_eq!(freedoms(&solved, &mates).unwrap().0, 3, "B keeps its three turns");
    }

    /// A slides along X, B along Y: one mm of A is two of B, and B is the later body.
    #[test]
    fn a_coupler_moves_the_later_part_by_the_ratio_across_axes() {
        let slide = |body: Body, direction: Normal| Slide {
            body,
            direction,
            offset: q(0),
            from: None,
            frame: [0, 0, 0].map(q),
        };
        let coupler = |mate: &str, reverse: bool, offset: i64| Constraint::Coupler {
            mate: mate.into(),
            a: slide(a(), [1, 0, 0]),
            b: slide(b(), [0, 1, 0]),
            ratio: [q(1), q(2)],
            reverse,
            offset: q(offset),
        };
        let at: BTreeMap<Body, Placement> = [
            (a(), Placement::translation(DVec3::new(5.0, 0.0, 0.0))),
            (b(), Placement::translation(DVec3::new(1.0, 1.0, 1.0))),
        ]
        .into();
        let solved = solve(&at, &[coupler("K", false, 0)]).unwrap();
        assert_eq!(solved[&a()], at[&a()]);
        assert_eq!(solved[&b()].translation, DVec3::new(1.0, 10.0, 1.0));
        let solved = solve(&at, &[coupler("K", true, 4)]).unwrap();
        assert_eq!(solved[&b()].translation, DVec3::new(1.0, -6.0, 1.0));
        // It fixes one motion.
        assert_eq!(freedoms(&solved, &[coupler("K", true, 4)]).unwrap().0, 11);
        // Two couplers with different offsets cannot both hold, wherever A is.
        let held = Constraint::Origin { mate: "O".into(), body: a(), axes: true, at: at[&a()] };
        let refused = solve(&at, &[held, coupler("K", false, 0), coupler("K2", false, 3)]).unwrap_err();
        assert_eq!(refused.code, RefusalCode::OverConstrained, "{refused}");
        assert_eq!(refused.mates, ["K", "K2"]);
    }

    fn limit(mate: &str, min: f64, max: f64) -> Constraint {
        let along = Some(Along::Range(min, max));
        Constraint::Parallel {
            mate: mate.into(),
            a: a(),
            b: b(),
            plane_a: plane([0, 0, 1], 10),
            plane_b: plane([0, 0, -1], 0),
            sign: -1,
            along,
        }
    }

    /// B's bottom is 20 above A's top. A limit that allows 20 keeps it; one from 5 to 10
    /// brings it to 10, the nearest value it allows.
    #[test]
    fn a_limit_keeps_the_authored_gap_or_takes_the_nearest_end() {
        let solved = solve(&bodies(), &[origin(true), limit("L", 0.0, 25.0)]).unwrap();
        assert_eq!(solved[&b()], bodies()[&b()]);
        let solved = solve(&bodies(), &[origin(true), limit("L", 5.0, 10.0)]).unwrap();
        assert_eq!(solved[&b()].translation, DVec3::new(10.0, 10.0, 20.0));
        let solved = solve(&bodies(), &[origin(true), limit("L", 30.0, 40.0)]).unwrap();
        assert_eq!(solved[&b()].translation.z, 40.0);
        // It turns the faces parallel and fixes no translation.
        let mates = [origin(true), limit("L", 5.0, 10.0)];
        assert_eq!(freedoms(&solved, &mates).unwrap().0, 4);
        // Two limits that leave nothing between them are named together.
        let refused = solve(&bodies(), &[origin(true), limit("L1", 0.0, 5.0), limit("L2", 10.0, 20.0)]).unwrap_err();
        assert_eq!(refused.code, RefusalCode::OverConstrained, "{refused}");
        assert_eq!(refused.mates, ["L1", "L2"]);
        // A distance inside a limit holds; one outside it is named with it.
        let d = distance("D", plane([0, 0, 1], 10), plane([0, 0, -1], 0), -1, 7.0);
        let solved = solve(&bodies(), &[origin(true), limit("L", 5.0, 10.0), d.clone()]).unwrap();
        assert_eq!(solved[&b()].translation.z, 17.0);
        let d = distance("D", plane([0, 0, 1], 10), plane([0, 0, -1], 0), -1, 12.0);
        let refused = solve(&bodies(), &[origin(true), limit("L", 5.0, 10.0), d]).unwrap_err();
        assert_eq!(refused.mates, ["D", "L"]);
    }

    /// The groove runs from x = -10 to x = 10 on A; the tab from 2 to 6 on B's frame.
    #[test]
    fn a_free_width_keeps_the_tab_within_the_groove() {
        let mates = |x: f64| {
            let at: BTreeMap<Body, Placement> =
                [(a(), Placement::IDENTITY), (b(), Placement::translation(DVec3::new(x, 3.0, 4.0)))].into();
            let solved = solve(&at, &[origin(true), width(WidthMode::Free)]).unwrap();
            solved[&b()].translation.x
        };
        // Inside it stays; past either wall it comes back to touch it.
        assert_eq!(mates(-3.0), -3.0);
        assert_eq!(mates(9.0), 4.0);
        assert_eq!(mates(-20.0), -12.0);
        let solved = solve(&bodies(), &[origin(true), width(WidthMode::Free)]).unwrap();
        assert_eq!(freedoms(&solved, &[origin(true), width(WidthMode::Free)]).unwrap().0, 4);
        // A tab wider than the groove cannot fit.
        let mut wide = width(WidthMode::Free);
        if let Constraint::Parallel { along: Some(Along::Width { tab, .. }), .. } = &mut wide {
            *tab = plane([-1, 0, 0], 20);
        }
        let refused = solve(&bodies(), &[origin(true), wide]).unwrap_err();
        assert_eq!((refused.code, refused.mates.clone()), (RefusalCode::OverConstrained, vec!["W".to_owned()]));
    }

    /// A's slot runs along X from (-10, 0) to (10, 0), its round ends' axes along Z; B's pin
    /// runs along Z through B's origin.
    fn slot(mode: SlotMode) -> Constraint {
        Constraint::Slot {
            mate: "P".into(),
            a: a(),
            slot: LocalSlot { depth: [0, 0, 1], ends: [[-10, 0, 0].map(q), [10, 0, 5].map(q)] },
            b: b(),
            axis: line([0, 0, 1], [0, 0, 0]),
            sign: 1,
            mode,
        }
    }

    #[test]
    fn a_pin_in_a_slot_stays_on_its_mid_plane_and_between_its_ends() {
        let at: BTreeMap<Body, Placement> =
            [(a(), Placement::IDENTITY), (b(), Placement::translation(DVec3::new(15.0, 3.0, 7.0)))].into();
        let x = |mode: SlotMode| {
            let solved = solve(&at, &[origin(true), slot(mode)]).unwrap();
            let t = solved[&b()].translation;
            assert_eq!((t.y, t.z), (0.0, 7.0));
            t.x
        };
        assert_eq!(x(SlotMode::Free), 10.0);
        assert_eq!(x(SlotMode::Centre), 0.0);
        assert_eq!(x(SlotMode::Distance { distance: q(4), flipped: false }), -6.0);
        assert_eq!(x(SlotMode::Distance { distance: q(4), flipped: true }), 6.0);
        assert_eq!(x(SlotMode::Share { fraction: BigRational::new(1.into(), 4.into()), flipped: false }), -5.0);
        // Free leaves the slide along it, the slide along the pin and the spin; a position
        // mode holds the slide along it too.
        let solved = solve(&at, &[origin(true), slot(SlotMode::Free)]).unwrap();
        assert_eq!(freedoms(&solved, &[origin(true), slot(SlotMode::Free)]).unwrap().0, 3);
        assert_eq!(freedoms(&solved, &[origin(true), slot(SlotMode::Centre)]).unwrap().0, 2);
        // A pin lying across the slot turns to stand in it.
        let mut lying = at.clone();
        lying.get_mut(&b()).unwrap().rotation = DQuat::from_rotation_x(std::f64::consts::FRAC_PI_2);
        let solved = solve(&lying, &[origin(true), slot(SlotMode::Centre)]).unwrap();
        assert_eq!(world(solved[&b()].rotation, [0, 0, 1], "P").unwrap(), [0, 0, 1]);
    }

    /// Two square profiles, A's edges along X and Y and B's turned 45 degrees in its own
    /// frame: no quarter turn about their normal lines them up.
    #[test]
    fn a_profile_spin_takes_a_quarter_turn_that_lines_the_edges_up() {
        let profile = |eb: DVec3| {
            vec![
                origin(true),
                Constraint::Parallel {
                    mate: "PC".into(),
                    a: a(),
                    b: b(),
                    plane_a: plane([0, 0, 1], 10),
                    plane_b: plane([0, 0, -1], 0),
                    sign: -1,
                    along: Some(Along::Distance(0.0)),
                },
                Constraint::OnAxis {
                    mate: "PC".into(),
                    a: a(),
                    axis: line([0, 0, 1], [3, 4, 10]),
                    b: b(),
                    point: [0, 0, 0].map(q),
                },
                Constraint::Spin {
                    mate: "PC".into(),
                    a: a(),
                    b: b(),
                    normal: [0, 0, 1],
                    edges: Some((vec![DVec3::X, DVec3::Y], vec![eb])),
                },
            ]
        };
        let mut at = bodies();
        at.get_mut(&b()).unwrap().rotation = DQuat::from_rotation_z(0.3);
        let mates = profile(DVec3::new(0.0, 1.0, 0.0));
        let solved = solve(&at, &mates).unwrap();
        assert_eq!(solved[&b()].translation, DVec3::new(3.0, 4.0, 10.0));
        assert!(cube_of(solved[&b()].rotation).is_some(), "a quarter turn");
        assert_eq!(freedoms(&solved, &mates).unwrap().0, 0);
        let refused = solve(&at, &profile(DVec3::new(1.0, 1.0, 0.0))).unwrap_err();
        assert_eq!((refused.code, refused.mates.clone()), (RefusalCode::OverConstrained, vec!["PC".to_owned()]));
    }

    /// The mirror is the latest body, so it is the one the Symmetric moves, halfway between
    /// A and B. A limit on it moves by half of B: not a difference, so refused by name.
    #[test]
    fn a_limit_on_a_coordinate_a_relation_scales_is_refused_by_name() {
        let at = three([-30.0, 0.0, 0.0], [20.0, 0.0, 0.0]);
        let mates = [
            origin_of("O", a()),
            symmetric(Mirrored::Points([0, 0, 0].map(q), [0, 0, 0].map(q)), 1),
            Constraint::FromPlane {
                mate: "L".into(),
                a: a(),
                plane: plane([1, 0, 0], 0),
                b: c(),
                point: [0, 0, 0].map(q),
                direction: None,
                gap: q(0),
                upto: Some(q(100)),
                says: "limited".into(),
            },
        ];
        let refused = solve(&at, &mates).unwrap_err();
        assert_eq!(refused.code, RefusalCode::UnsupportedReference, "{refused}");
        assert!(refused.message.contains("one for one"), "{}", refused.message);
    }
}
