//! The certified-mesh proof between two placed solids.
//!
//! Each solid comes as a closed triangle mesh from the kernel's certified tessellation,
//! placed in the root's frame. The kernel proves a chord bound `h` for it: every mesh
//! triangle covers a patch of one face, and each point of the patch and the matching
//! point of the triangle are at most `h` apart. Sliding every surface point to its mesh
//! point never passes within `d - h` of a point `d` away from the surface, so the mesh
//! and the solid agree on what is inside everywhere farther than `h` from either
//! boundary. Each test below rests on one consequence:
//!
//! - **Clear.** The solid lies within `h` of its mesh. Two meshes farther apart than
//!   `h_a + h_b`, neither inside the other, hold two solids that do not meet.
//! - **Interferes.** A point inside both meshes, farther than `h_a` from the first and
//!   `h_b` from the second, is inside both solids, with a ball around it.
//! - **Shared face point.** Two planar faces on one plane share a point when a point of
//!   that plane lies inside both faces' triangles, farther than each face's bound from
//!   its mesh boundary.
//!
//! Anything else is touching or undecided, and nothing here guesses which. Every margin
//! also carries [`Solid::rounding`], an allowance for binary64 arithmetic on coordinates
//! of the size placed.

use glam::{DVec2, DVec3};

/// Triangles per leaf of a solid's hierarchy.
const LEAF: usize = 4;

/// The allowance for binary64 rounding, as a fraction of the largest coordinate. Placing
/// a point, a distance and a containment ray each lose a few units in the last place,
/// about 1e-16 of the coordinate; this is seven orders of magnitude above that.
pub(crate) const ROUNDING: f64 = 1e-9;

/// Triangle pairs within the cutoff that a pair query keeps.
const PAIRS_KEPT: usize = 4096;

/// Candidate points tried for an interference witness, per pair of solids.
const CANDIDATES: usize = 20_000;

/// A box in the root's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bounds {
    pub min: DVec3,
    pub max: DVec3,
}

impl Bounds {
    const EMPTY: Bounds = Bounds { min: DVec3::splat(f64::INFINITY), max: DVec3::splat(f64::NEG_INFINITY) };

    fn of(points: &[DVec3]) -> Bounds {
        let mut b = Bounds::EMPTY;
        for p in points {
            b.min = b.min.min(*p);
            b.max = b.max.max(*p);
        }
        b
    }

    fn union(self, other: Bounds) -> Bounds {
        Bounds { min: self.min.min(other.min), max: self.max.max(other.max) }
    }

    /// The distance between two boxes, zero when they overlap.
    pub fn distance(&self, other: &Bounds) -> f64 {
        (other.min - self.max).max(self.min - other.max).max(DVec3::ZERO).length()
    }

    fn point_distance(&self, p: DVec3) -> f64 {
        (self.min - p).max(p - self.max).max(DVec3::ZERO).length()
    }

    /// Whether the ray from `origin` with reciprocal direction `inv` meets the box.
    fn hit_by(&self, origin: DVec3, inv: DVec3) -> bool {
        let t0 = (self.min - origin) * inv;
        let t1 = (self.max - origin) * inv;
        let near = t0.min(t1).max_element();
        let far = t0.max(t1).min_element();
        // A NaN from 0 * inf orders as nothing and keeps the box, so a ray along one of
        // its faces is never dropped.
        far.partial_cmp(&near.max(0.0)) != Some(std::cmp::Ordering::Less)
    }
}

/// A hierarchy node: a leaf holds `order[first..first + count]`, an inner node
/// (`count` zero) has children `left` and `right`.
struct Node {
    bounds: Bounds,
    left: u32,
    right: u32,
    first: u32,
    count: u32,
}

/// A placed solid: its certified mesh in the root's frame.
pub(crate) struct Solid {
    tris: Vec<[DVec3; 3]>,
    /// The caller's face index of each triangle.
    faces: Vec<u32>,
    /// The kernel's proven chord bound for this mesh, in millimetres.
    pub chord: f64,
    /// The binary64 allowance for this mesh's coordinates, in millimetres.
    pub rounding: f64,
    /// The box around every triangle.
    pub bounds: Bounds,
    nodes: Vec<Node>,
    order: Vec<u32>,
}

/// The nearest points found on two features, such as two triangles, and a proven lower
/// bound on how far apart the features are. The two are kept apart: a pair of points
/// bounds the distance from above only, and only a search that reaches the true closest
/// points bounds it from below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Closest {
    /// How far apart `a` and `b` are. They lie on the two features, so the features are
    /// at most this far apart.
    pub distance: f64,
    /// The features are at least this far apart, up to the rounding allowance.
    pub lower: f64,
    /// A point of the first feature.
    pub a: DVec3,
    /// A point of the second feature.
    pub b: DVec3,
}

impl Closest {
    /// Two points proven to be the closest of their features, so that their distance
    /// bounds the features' distance both ways.
    fn exact(a: DVec3, b: DVec3) -> Closest {
        let distance = a.distance(b);
        Closest { distance, lower: distance, a, b }
    }

    /// Over two sub-problems whose least distance between them is the whole problem's:
    /// the nearer pair of points, and the lesser lower bound.
    fn or(self, other: Closest) -> Closest {
        let lower = self.lower.min(other.lower);
        let nearer = if other.distance < self.distance { other } else { self };
        Closest { lower, ..nearer }
    }

    /// The same with the two features exchanged.
    fn swapped(self) -> Closest {
        Closest { a: self.b, b: self.a, ..self }
    }
}

/// What two solids' meshes say near each other.
pub(crate) struct Near {
    /// The nearest pair of mesh points found. They are that far apart, so it bounds the
    /// meshes' least distance from above.
    pub closest: Option<Closest>,
    /// The meshes are at least this far apart, up to the rounding allowance: the least
    /// lower bound over the triangle pairs searched. The search reaches every pair that
    /// could be nearer than `closest`, so the nearest pair is among them.
    pub lower: f64,
    /// Triangle pairs within the cutoff, nearest first: triangle of a, triangle of b,
    /// closest points. At most [`PAIRS_KEPT`] of them.
    pub pairs: Vec<(u32, u32, Closest)>,
}

impl Solid {
    /// A solid from triangles in the root's frame, each with the caller's face index,
    /// and the mesh's proven chord bound.
    pub fn new(tris: Vec<[DVec3; 3]>, faces: Vec<u32>, chord: f64) -> Solid {
        assert_eq!(tris.len(), faces.len(), "one face per triangle");
        let extent = tris.iter().flatten().fold(1.0_f64, |m, p| m.max(p.abs().max_element()));
        let mut solid = Solid {
            bounds: Bounds::EMPTY,
            order: (0..tris.len() as u32).collect(),
            tris,
            faces,
            chord,
            rounding: ROUNDING * extent,
            nodes: Vec::new(),
        };
        if !solid.tris.is_empty() {
            solid.build(0, solid.tris.len());
            solid.bounds = solid.nodes[0].bounds;
        }
        solid
    }

    /// The number of triangles.
    pub fn count(&self) -> usize {
        self.tris.len()
    }

    /// Triangle `i`.
    pub fn triangle(&self, i: u32) -> [DVec3; 3] {
        self.tris[i as usize]
    }

    /// The caller's face index of triangle `i`.
    pub fn face(&self, i: u32) -> u32 {
        self.faces[i as usize]
    }

    /// The bound on how far this solid and its mesh disagree, rounding included.
    pub fn slack(&self) -> f64 {
        self.chord + self.rounding
    }

    /// Builds the hierarchy over `order[first..first + count]` and returns its node.
    fn build(&mut self, first: usize, count: usize) -> u32 {
        let index = self.nodes.len() as u32;
        let span = &self.order[first..first + count];
        let bounds = span.iter().fold(Bounds::EMPTY, |b, &t| b.union(Bounds::of(&self.tris[t as usize])));
        self.nodes.push(Node { bounds, left: 0, right: 0, first: first as u32, count: count as u32 });
        if count <= LEAF {
            return index;
        }
        let size = bounds.max - bounds.min;
        let axis = if size.x >= size.y && size.x >= size.z {
            0
        } else if size.y >= size.z {
            1
        } else {
            2
        };
        let centre = |t: u32| {
            let [a, b, c] = self.tris[t as usize];
            (a[axis] + b[axis] + c[axis]) / 3.0
        };
        let mut span: Vec<u32> = span.to_vec();
        span.sort_by(|&p, &q| centre(p).total_cmp(&centre(q)).then(p.cmp(&q)));
        self.order[first..first + count].copy_from_slice(&span);
        let half = count / 2;
        let left = self.build(first, half);
        let right = self.build(first + half, count - half);
        let node = &mut self.nodes[index as usize];
        (node.left, node.right, node.count) = (left, right, 0);
        index
    }

    fn leaf(&self, node: &Node) -> &[u32] {
        &self.order[node.first as usize..(node.first + node.count) as usize]
    }

    /// The least distance from `p` to the mesh, searched no farther than `cutoff`:
    /// `cutoff` or more means nothing nearer.
    pub fn surface_distance(&self, p: DVec3, cutoff: f64) -> f64 {
        let mut best = cutoff;
        if self.nodes.is_empty() {
            return best;
        }
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let node = &self.nodes[i as usize];
            if node.bounds.point_distance(p) >= best {
                continue;
            }
            if node.count == 0 {
                stack.push(node.left);
                stack.push(node.right);
                continue;
            }
            for &t in self.leaf(node) {
                best = best.min(point_triangle(p, &self.tris[t as usize]).distance(p));
            }
        }
        best
    }

    /// Whether `p` is inside the closed mesh, by the parity of crossings along a ray.
    /// A ray that grazes an edge or a vertex, or runs along a triangle, says nothing and
    /// the next direction is tried; `None` when every direction grazed.
    pub fn contains(&self, p: DVec3) -> Option<bool> {
        if self.nodes.is_empty() || self.bounds.point_distance(p) > 0.0 {
            return Some(false);
        }
        // Fixed directions with no simple ratio between their components, so that axis
        // aligned meshes never hold them in a plane of their edges.
        const DIRECTIONS: [[f64; 3]; 6] = [
            [0.613_207_1, 0.338_562_9, 0.713_629_4],
            [-0.417_386_5, 0.702_941_8, -0.576_213_9],
            [0.309_517_3, -0.866_204_7, 0.392_801_6],
            [-0.829_174_3, -0.243_618_2, 0.503_271_9],
            [0.131_472_6, 0.982_517_4, -0.131_895_3],
            [0.657_314_2, -0.437_926_1, -0.613_548_7],
        ];
        DIRECTIONS.iter().find_map(|d| self.parity(p, DVec3::from_array(*d).normalize()))
    }

    fn parity(&self, origin: DVec3, dir: DVec3) -> Option<bool> {
        let inv = DVec3::ONE / dir;
        let mut inside = false;
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let node = &self.nodes[i as usize];
            if !node.bounds.hit_by(origin, inv) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.left);
                stack.push(node.right);
                continue;
            }
            for &t in self.leaf(node) {
                match crossing(origin, dir, &self.tris[t as usize]) {
                    Crossing::Clean => inside = !inside,
                    Crossing::Miss => {}
                    Crossing::Grazes => return None,
                }
            }
        }
        Some(inside)
    }

    /// Whether `p` is proven inside the solid behind this mesh: inside the mesh and
    /// farther than the slack from it.
    pub fn proves_inside(&self, p: DVec3) -> bool {
        let slack = self.slack();
        self.surface_distance(p, slack * 2.0 + 1.0) > slack && self.contains(p) == Some(true)
    }
}

/// The meshes of `a` and `b` near each other: the least distance between them and the
/// triangle pairs within `cutoff`.
pub(crate) fn near(a: &Solid, b: &Solid, cutoff: f64) -> Near {
    let mut out = Near { closest: None, lower: f64::INFINITY, pairs: Vec::new() };
    if a.nodes.is_empty() || b.nodes.is_empty() {
        return out;
    }
    let mut best = f64::INFINITY;
    let mut stack = vec![(0u32, 0u32)];
    while let Some((i, j)) = stack.pop() {
        let (na, nb) = (&a.nodes[i as usize], &b.nodes[j as usize]);
        let gap = na.bounds.distance(&nb.bounds);
        if gap > best && gap > cutoff {
            continue;
        }
        match (na.count == 0, nb.count == 0) {
            (true, true) => {
                stack.extend([(na.left, nb.left), (na.left, nb.right), (na.right, nb.left), (na.right, nb.right)])
            }
            (true, false) => stack.extend([(na.left, j), (na.right, j)]),
            (false, true) => stack.extend([(i, nb.left), (i, nb.right)]),
            (false, false) => {
                for &ta in a.leaf(na) {
                    for &tb in b.leaf(nb) {
                        let c = triangles(&a.tris[ta as usize], &b.tris[tb as usize]);
                        if c.distance < best {
                            best = c.distance;
                            out.closest = Some(c);
                        }
                        out.lower = out.lower.min(c.lower);
                        if c.lower <= cutoff && out.pairs.len() < PAIRS_KEPT {
                            out.pairs.push((ta, tb, c));
                        }
                    }
                }
            }
        }
    }
    // The traversal order depends on the stack only, so the kept pairs are deterministic;
    // sort them so that callers see them nearest first.
    out.pairs.sort_by(|p, q| p.2.distance.total_cmp(&q.2.distance).then((p.0, p.1).cmp(&(q.0, q.1))));
    out
}

/// A point proven inside both solids, searched from the given near pairs and from each
/// solid's own triangles. `None` proves nothing.
pub(crate) fn witness(a: &Solid, b: &Solid, pairs: &[(u32, u32, Closest)]) -> Option<DVec3> {
    let step = 2.0 * (a.slack() + b.slack());
    let reach = a.bounds.union(b.bounds);
    let reach = (reach.max - reach.min).max_element().max(step);
    let steps: Vec<f64> = std::iter::successors(Some(step), |s| Some(s * 2.0)).take_while(|s| *s <= reach).collect();
    // `None` once the budget is spent.
    let mut tried = 0usize;
    let mut proven = |y: DVec3| {
        if tried >= CANDIDATES {
            return None;
        }
        tried += 1;
        Some(a.proves_inside(y) && b.proves_inside(y))
    };
    // Where the two surfaces meet, the overlap opens against both outward normals.
    for (ta, tb, c) in pairs {
        let (na, nb) = (normal(&a.tris[*ta as usize]), normal(&b.tris[*tb as usize]));
        let both = -(na + nb);
        let dirs = [both.normalize_or_zero(), -na, -nb];
        let bases = [(c.a + c.b) * 0.5, c.a, c.b];
        for base in bases {
            for dir in dirs.iter().filter(|d| **d != DVec3::ZERO) {
                for s in &steps {
                    let y = base + *dir * *s;
                    match proven(y) {
                        Some(true) => return Some(y),
                        Some(false) => {}
                        None => return None,
                    }
                }
            }
        }
    }
    // One solid inside the other, or overlaps with no surface pair kept: walk inward from
    // spread-out triangles of each.
    for (s, other) in [(a, b), (b, a)] {
        let stride = (s.count() / 64).max(1);
        for t in (0..s.count()).step_by(stride) {
            let tri = &s.tris[t];
            let base = (tri[0] + tri[1] + tri[2]) / 3.0;
            if other.bounds.point_distance(base) > 0.0 {
                continue;
            }
            let dir = -normal(tri);
            if dir == DVec3::ZERO {
                continue;
            }
            for step in &steps {
                let y = base + dir * *step;
                match proven(y) {
                    Some(true) => return Some(y),
                    Some(false) => {}
                    None => return None,
                }
            }
        }
    }
    None
}

/// A point proven to lie on two planar faces at once: face `fa` of `a` and face `fb` of
/// `b`, both on the plane normal to world axis `axis`, which the caller has proven they
/// share. `None` proves nothing.
pub(crate) fn shared_point(a: &Solid, fa: u32, b: &Solid, fb: u32, axis: usize) -> Option<DVec3> {
    let flat = |p: DVec3| {
        let (u, v) = [(1, 2), (2, 0), (0, 1)][axis];
        DVec2::new(p[u], p[v])
    };
    let face = |s: &Solid, f: u32| -> Vec<[DVec3; 3]> {
        (0..s.count() as u32).filter(|t| s.face(*t) == f).map(|t| s.triangle(t)).collect()
    };
    let (ta, tb) = (face(a, fa), face(b, fb));
    let edges = |tris: &[[DVec3; 3]]| -> Vec<[DVec2; 2]> { boundary(tris).into_iter().map(|e| e.map(flat)).collect() };
    let (ea, eb) = (edges(&ta), edges(&tb));
    let (sa, sb) = (a.slack(), b.slack());
    const PAIRS: usize = 1_000_000;
    let mut tried = 0;
    for x in &ta {
        let px = x.map(flat);
        for y in &tb {
            tried += 1;
            if tried > PAIRS {
                return None;
            }
            let py = y.map(flat);
            let Some(c) = overlap_centre(&px, &py) else { continue };
            let clear =
                |edges: &[[DVec2; 2]], slack: f64| edges.iter().all(|[p, q]| segment_point_2d(*p, *q, c) > slack);
            if clear(&ea, sa) && clear(&eb, sb) {
                // Back on the plane: the axis coordinate of the face, which is the plane's.
                let mut p = DVec3::ZERO;
                let (u, v) = [(1, 2), (2, 0), (0, 1)][axis];
                p[u] = c.x;
                p[v] = c.y;
                p[axis] = (x[0][axis] + x[1][axis] + x[2][axis]) / 3.0;
                return Some(p);
            }
        }
    }
    None
}

/// The edges of a set of triangles that only one of them has: the boundary of the
/// region they cover. Shared vertices are placed by the same arithmetic, so they match
/// bit for bit.
fn boundary(tris: &[[DVec3; 3]]) -> Vec<[DVec3; 2]> {
    use std::collections::BTreeMap;
    /// An edge by its ends' bits, lower end first.
    type EdgeKey = ([u64; 3], [u64; 3]);
    let key = |p: DVec3| p.to_array().map(f64::to_bits);
    let mut count: BTreeMap<EdgeKey, (usize, [DVec3; 2])> = BTreeMap::new();
    for t in tris {
        for k in 0..3 {
            let (p, q) = (t[k], t[(k + 1) % 3]);
            let (kp, kq) = (key(p), key(q));
            let edge = if kp <= kq { (kp, kq) } else { (kq, kp) };
            count.entry(edge).or_insert((0, [p, q])).0 += 1;
        }
    }
    count.into_values().filter(|(n, _)| *n == 1).map(|(_, e)| e).collect()
}

/// The centre of the overlap of two triangles in the plane, when they overlap in area.
fn overlap_centre(a: &[DVec2; 3], b: &[DVec2; 3]) -> Option<DVec2> {
    let ccw = |t: &[DVec2; 3]| {
        let area = (t[1] - t[0]).perp_dot(t[2] - t[0]);
        if area >= 0.0 { *t } else { [t[0], t[2], t[1]] }
    };
    let (a, b) = (ccw(a), ccw(b));
    // Sutherland-Hodgman: clip a by each edge of b.
    let mut poly: Vec<DVec2> = a.to_vec();
    for k in 0..3 {
        let (p, q) = (b[k], b[(k + 1) % 3]);
        let side = |x: DVec2| (q - p).perp_dot(x - p);
        let mut out = Vec::with_capacity(poly.len() + 1);
        for i in 0..poly.len() {
            let (s, e) = (poly[i], poly[(i + 1) % poly.len()]);
            let (ds, de) = (side(s), side(e));
            if de >= 0.0 {
                if ds < 0.0 {
                    out.push(s + (e - s) * (ds / (ds - de)));
                }
                out.push(e);
            } else if ds >= 0.0 {
                out.push(s + (e - s) * (ds / (ds - de)));
            }
        }
        poly = out;
        if poly.len() < 3 {
            return None;
        }
    }
    let mut area = 0.0;
    let mut centre = DVec2::ZERO;
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let w = p.perp_dot(q);
        area += w;
        centre += (p + q) * w;
    }
    (area > 0.0).then(|| centre / (3.0 * area))
}

fn segment_point_2d(p: DVec2, q: DVec2, x: DVec2) -> f64 {
    let d = q - p;
    let len = d.length_squared();
    let t = if len > 0.0 { ((x - p).dot(d) / len).clamp(0.0, 1.0) } else { 0.0 };
    (p + d * t).distance(x)
}

/// The unit outward normal of a triangle wound counter-clockwise seen from outside, or
/// zero for a degenerate one.
fn normal(t: &[DVec3; 3]) -> DVec3 {
    (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero()
}

enum Crossing {
    Clean,
    Miss,
    Grazes,
}

/// How a ray meets a triangle. Anything within a relative 1e-9 of an edge, a vertex, the
/// triangle's plane or the ray's origin grazes.
fn crossing(origin: DVec3, dir: DVec3, t: &[DVec3; 3]) -> Crossing {
    const GRAZE: f64 = 1e-9;
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    let scale = e1.length() * e2.length();
    if scale == 0.0 {
        // A degenerate triangle has no inside to cross.
        return Crossing::Miss;
    }
    if det.abs() <= GRAZE * scale {
        // The ray runs along the triangle's plane: grazing if it is in that plane.
        let off = (origin - t[0]).dot(e1.cross(e2).normalize_or_zero());
        return if off.abs() <= GRAZE * (scale.sqrt() + origin.length()) { Crossing::Grazes } else { Crossing::Miss };
    }
    let s = origin - t[0];
    let u = s.dot(p) / det;
    let q = s.cross(e1);
    let v = dir.dot(q) / det;
    let w = 1.0 - u - v;
    let d = e2.dot(q) / det;
    let margin = GRAZE;
    if u < -margin || v < -margin || w < -margin || d < -margin * scale.sqrt() {
        return Crossing::Miss;
    }
    if u <= margin || v <= margin || w <= margin || d <= margin * scale.sqrt() {
        return Crossing::Grazes;
    }
    Crossing::Clean
}

/// The closest point of triangle `t` to `p` (Ericson, Real-Time Collision Detection 5.1.5).
fn point_triangle(p: DVec3, t: &[DVec3; 3]) -> DVec3 {
    let [a, b, c] = *t;
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = va + vb + vc;
    if denom <= 0.0 {
        // Degenerate: the nearest of the three edges.
        return [segment_point(a, b, p), segment_point(b, c, p), segment_point(c, a, p)]
            .into_iter()
            .min_by(|x, y| x.distance(p).total_cmp(&y.distance(p)))
            .expect("three edges");
    }
    a + ab * (vb / denom) + ac * (vc / denom)
}

/// Where along segment `pq` its point nearest `x` is, from 0 at `p` to 1 at `q`.
fn segment_param(p: DVec3, q: DVec3, x: DVec3) -> f64 {
    let d = q - p;
    let len = d.length_squared();
    if len > 0.0 { ((x - p).dot(d) / len).clamp(0.0, 1.0) } else { 0.0 }
}

fn segment_point(p: DVec3, q: DVec3, x: DVec3) -> DVec3 {
    p + (q - p) * segment_param(p, q, x)
}

/// The closest points of two segments. Segments more than 45 degrees from parallel meet
/// where their lines come nearest, clamped to their ends (Ericson 5.1.9). Finding that
/// point divides by the squared sine of the angle between them, so nearer parallel pairs
/// go to [`alongside`], which never does.
fn segments(p1: DVec3, q1: DVec3, p2: DVec3, q2: DVec3) -> Closest {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.length_squared(), d2.length_squared(), d2.dot(r));
    if a == 0.0 {
        return Closest::exact(p1, segment_point(p2, q2, p1));
    }
    if e == 0.0 {
        return Closest::exact(segment_point(p1, q1, p2), p2);
    }
    let (b, c) = (d1.dot(d2), d1.dot(r));
    let denom = a * e - b * b;
    if 2.0 * denom <= a * e {
        return alongside(p1, q1, p2, q2);
    }
    let mut s = ((b * f - c * e) / denom).clamp(0.0, 1.0);
    let mut t = (b * s + f) / e;
    if t < 0.0 {
        t = 0.0;
        s = (-c / a).clamp(0.0, 1.0);
    } else if t > 1.0 {
        t = 1.0;
        s = ((b - c) / a).clamp(0.0, 1.0);
    }
    Closest::exact(p1 + d1 * s, p2 + d2 * t)
}

/// The closest points of two segments within 45 degrees of parallel, and a lower bound on
/// their distance that does not rest on where their lines come nearest, which is ill
/// conditioned here.
///
/// Their least distance is reached either with an end of one segment, or between inner
/// points of both. An end's candidates are the four distances from an end to the other
/// segment. Between inner points, the first segment's point lies alongside the second,
/// between the planes square to the second through its ends, and its distance to the
/// second is its distance to the second's line. Over the alongside part of the first
/// segment that distance is the distance from the origin to a segment of offsets, which
/// stays well conditioned however near parallel the two are. Near parallel, the four ends
/// alone bound the distance from above only: two long segments crossing a hair apart are
/// nearest in their middles.
fn alongside(p1: DVec3, q1: DVec3, p2: DVec3, q2: DVec3) -> Closest {
    let ends = Closest::exact(p1, segment_point(p2, q2, p1))
        .or(Closest::exact(q1, segment_point(p2, q2, q1)))
        .or(Closest::exact(segment_point(p1, q1, p2), p2))
        .or(Closest::exact(segment_point(p1, q1, q2), q2));
    let (d1, length) = (q1 - p1, p2.distance(q2));
    let u = (q2 - p2) / length;
    // Along u the first segment starts at `start` and moves `rate` per unit of its
    // parameter; within 45 degrees of u, `rate` is at least 0.7 of its length.
    let (start, rate) = ((p1 - p2).dot(u), d1.dot(u));
    let (s0, s1) = (-start / rate, (length - start) / rate);
    let (s0, s1) = (s0.min(s1).max(0.0), s0.max(s1).min(1.0));
    if s0 > s1 {
        return ends;
    }
    // A point's offset from the second segment's line, square to it.
    let offset = |x: DVec3| {
        let v = x - p2;
        v - u * v.dot(u)
    };
    let (x0, x1) = (p1 + d1 * s0, p1 + d1 * s1);
    let x = x0 + (x1 - x0) * segment_param(offset(x0), offset(x1), DVec3::ZERO);
    let y = segment_point(p2, q2, x);
    // The distance to the line is at most the distance to the segment, and equal to it
    // alongside, so it bounds the inner candidate from below wherever rounding puts `x`.
    ends.or(Closest { distance: x.distance(y), lower: offset(x).length(), a: x, b: y })
}

/// The closest points of a segment and a triangle. Their least distance is reached at an
/// end of the segment, on an edge of the triangle, or where the segment crosses it.
fn segment_triangle(p: DVec3, q: DVec3, t: &[DVec3; 3]) -> Closest {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    let (dp, dq) = ((p - t[0]).dot(n), (q - t[0]).dot(n));
    let mut best = Closest::exact(p, point_triangle(p, t)).or(Closest::exact(q, point_triangle(q, t)));
    for k in 0..3 {
        best = best.or(segments(p, q, t[k], t[(k + 1) % 3]));
    }
    // Where the segment crosses the triangle's plane inside it, the crossing point is
    // the nearest, to within rounding.
    if (dp < 0.0) != (dq < 0.0) && dp != dq {
        let x = p + (q - p) * (dp / (dp - dq));
        best = best.or(Closest::exact(x, point_triangle(x, t)));
    }
    best
}

/// The closest points of two triangles. Two triangles' least distance is reached on an
/// edge of one of them, and where they cross, an edge of one passes through the other.
pub(crate) fn triangles(a: &[DVec3; 3], b: &[DVec3; 3]) -> Closest {
    (0..3)
        .map(|k| {
            let from_a = segment_triangle(a[k], a[(k + 1) % 3], b);
            from_a.or(segment_triangle(b[k], b[(k + 1) % 3], a).swapped())
        })
        .reduce(Closest::or)
        .expect("three edges")
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A closed box from `lo` to `hi` as twelve outward triangles, face index per side.
    pub(crate) fn cuboid(lo: DVec3, hi: DVec3) -> (Vec<[DVec3; 3]>, Vec<u32>) {
        let c = |x: usize, y: usize, z: usize| DVec3::new([lo.x, hi.x][x], [lo.y, hi.y][y], [lo.z, hi.z][z]);
        // Each side as four corners counter-clockwise seen from outside.
        let sides = [
            [c(0, 0, 0), c(0, 1, 0), c(1, 1, 0), c(1, 0, 0)], // -z
            [c(0, 0, 1), c(1, 0, 1), c(1, 1, 1), c(0, 1, 1)], // +z
            [c(0, 0, 0), c(1, 0, 0), c(1, 0, 1), c(0, 0, 1)], // -y
            [c(0, 1, 0), c(0, 1, 1), c(1, 1, 1), c(1, 1, 0)], // +y
            [c(0, 0, 0), c(0, 0, 1), c(0, 1, 1), c(0, 1, 0)], // -x
            [c(1, 0, 0), c(1, 1, 0), c(1, 1, 1), c(1, 0, 1)], // +x
        ];
        let mut tris = Vec::new();
        let mut faces = Vec::new();
        for (f, [p, q, r, s]) in sides.into_iter().enumerate() {
            tris.push([p, q, r]);
            tris.push([p, r, s]);
            faces.extend([f as u32, f as u32]);
        }
        (tris, faces)
    }

    /// A placed block with the given chord bound.
    pub(crate) fn block(lo: [f64; 3], hi: [f64; 3], chord: f64) -> Solid {
        let (tris, faces) = cuboid(DVec3::from_array(lo), DVec3::from_array(hi));
        Solid::new(tris, faces, chord)
    }

    /// A closed triangular prism along x from `-half` to `half`, with the cross-section
    /// `section` in (y, z), turned by `angle` about world Z and then moved by `shift`: two
    /// end caps and each side split along the diagonal from its lower-index corner at
    /// `-half` to its other corner at `half`, eight outward triangles in all.
    pub(crate) fn prism(section: [[f64; 2]; 3], half: f64, angle: f64, shift: DVec3, chord: f64) -> Solid {
        let turn = glam::DQuat::from_rotation_z(angle);
        let corner = |i: usize, x: f64| turn * DVec3::new(x, section[i][0], section[i][1]) + shift;
        let (lo, hi): (Vec<DVec3>, Vec<DVec3>) = (0..3).map(|i| (corner(i, -half), corner(i, half))).unzip();
        let mut tris = vec![[lo[0], lo[1], lo[2]], [hi[0], hi[1], hi[2]]];
        for i in 0..3 {
            let j = (i + 1) % 3;
            tris.push([lo[i], lo[j], hi[j]]);
            tris.push([lo[i], hi[j], hi[i]]);
        }
        let centre = (lo.iter().chain(&hi).copied().sum::<DVec3>()) / 6.0;
        for t in &mut tris {
            let mid = (t[0] + t[1] + t[2]) / 3.0;
            if (t[1] - t[0]).cross(t[2] - t[0]).dot(mid - centre) < 0.0 {
                t.swap(1, 2);
            }
        }
        let faces = vec![0, 1, 2, 2, 3, 3, 4, 4];
        Solid::new(tris, faces, chord)
    }

    /// The review's pair of ridged prisms, 20 m long: A's ridge points up along x at the
    /// origin, B's points down, turned by `angle` about Z and lifted 0.01 mm. Their ridges
    /// cross at x = 0, so the parts are exactly 0.01 mm apart at every angle.
    pub(crate) fn ridges(angle: f64, chord: f64) -> (Solid, Solid) {
        let a = prism([[0.0, 0.0], [-10.0, -10.0], [10.0, -10.0]], 10_000.0, 0.0, DVec3::ZERO, chord);
        let b = prism([[0.0, 0.0], [10.0, 10.0], [-10.0, 10.0]], 10_000.0, angle, DVec3::new(0.0, 0.0, 0.01), chord);
        (a, b)
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{block, cuboid};
    use super::*;

    #[test]
    fn outward_winding_of_the_test_box() {
        let (tris, _) = cuboid(DVec3::ZERO, DVec3::ONE);
        let centre = DVec3::splat(0.5);
        for t in &tris {
            let mid = (t[0] + t[1] + t[2]) / 3.0;
            assert!(normal(t).dot(mid - centre) > 0.0, "{t:?} faces inward");
        }
    }

    #[test]
    fn containment_by_parity() {
        let s = block([0.0; 3], [10.0, 20.0, 30.0], 0.01);
        assert_eq!(s.contains(DVec3::new(5.0, 5.0, 5.0)), Some(true));
        assert_eq!(s.contains(DVec3::new(5.0, 5.0, 31.0)), Some(false));
        assert_eq!(s.contains(DVec3::new(-1.0, 5.0, 5.0)), Some(false));
        // On an edge's line and on a vertex the first rays still settle it.
        assert_eq!(s.contains(DVec3::new(5.0, 10.0, 15.0)), Some(true));
        assert!(s.proves_inside(DVec3::new(5.0, 5.0, 5.0)));
        // Within the chord bound of a face proves nothing.
        assert!(!s.proves_inside(DVec3::new(5.0, 5.0, 29.995)));
    }

    #[test]
    fn distances_between_triangles() {
        let a = [DVec3::ZERO, DVec3::X, DVec3::Y];
        let b = [DVec3::new(0.0, 0.0, 2.0), DVec3::new(1.0, 0.0, 2.0), DVec3::new(0.0, 1.0, 2.0)];
        assert_eq!(triangles(&a, &b).distance, 2.0);
        // Crossing triangles are zero apart.
        let c = [DVec3::new(0.2, 0.2, -1.0), DVec3::new(0.2, 0.2, 1.0), DVec3::new(0.3, -1.0, 0.0)];
        assert!(triangles(&a, &c).distance < 1e-15, "{:?}", triangles(&a, &c));
        // Edge to corner: d's upright edge passes (2, 0.5, 0), nearest a's corner (1, 0, 0).
        let d = [DVec3::new(2.0, 0.5, -1.0), DVec3::new(2.0, 0.5, 1.0), DVec3::new(3.0, 0.5, 0.0)];
        assert!((triangles(&a, &d).distance - 1.25_f64.sqrt()).abs() < 1e-15);
    }

    #[test]
    fn near_parallel_segments_are_bounded_from_both_sides() {
        // Two 20 m segments crossing 0.01 mm apart at their middles: the ends are about
        // 0.0135 mm apart, more than the true 0.01, at any angle under a few microradians.
        let (p1, q1) = (DVec3::new(-10_000.0, 0.0, 0.0), DVec3::new(10_000.0, 0.0, 0.0));
        for angle in [0.0, 1e-12, 9e-7, 1.1e-6, 1e-3, 0.5] {
            let turn = glam::DQuat::from_rotation_z(angle);
            let lift = DVec3::new(0.0, 0.0, 0.01);
            let c = segments(p1, q1, turn * p1 + lift, turn * q1 + lift);
            assert!(c.lower <= 0.01 && c.lower > 0.01 - 1e-12, "angle {angle}: {c:?}");
            assert!(c.distance >= c.lower && c.distance < 0.01 + 1e-9, "angle {angle}: {c:?}");
            assert!((c.a.distance(c.b) - c.distance).abs() < 1e-15, "the points realise the distance");
        }
        // End to end on one line, 3 mm apart, the ends are the nearest points.
        let c = segments(DVec3::ZERO, DVec3::X * 10.0, DVec3::X * 13.0, DVec3::X * 20.0);
        assert_eq!((c.distance, c.lower), (3.0, 3.0));
        // Side by side, overlapping along their length, 2 mm apart.
        let c = segments(DVec3::ZERO, DVec3::X * 10.0, DVec3::new(-5.0, 2.0, 0.0), DVec3::new(5.0, 2.0, 0.0));
        assert_eq!((c.distance, c.lower), (2.0, 2.0));
    }

    #[test]
    fn near_parallel_ridges_are_their_true_distance_apart() {
        // The review's closed prisms: the meshes are 0.01 mm apart where the ridges cross.
        for angle in [0.0, 9e-7, 1.1e-6] {
            let (a, b) = tests_support::ridges(angle, 0.001);
            let n = near(&a, &b, 0.1);
            let c = n.closest.expect("closest points");
            assert!(n.lower <= 0.01 && n.lower > 0.01 - 1e-9, "angle {angle}: lower {}", n.lower);
            assert!(c.distance >= n.lower && c.distance < 0.01 + 1e-9, "angle {angle}: {c:?}");
        }
    }

    #[test]
    fn separated_blocks_are_far_and_touching_blocks_are_near() {
        let a = block([0.0; 3], [40.0, 30.0, 10.0], 0.01);
        let b = block([10.0, 5.0, 12.0], [30.0, 25.0, 18.0], 0.01);
        let n = near(&a, &b, 0.1);
        assert_eq!(n.closest.unwrap().distance, 2.0);
        assert_eq!(n.lower, 2.0);
        assert!(n.pairs.is_empty());
        let on = block([10.0, 5.0, 10.0], [30.0, 25.0, 16.0], 0.01);
        let n = near(&a, &on, 0.1);
        assert_eq!(n.closest.unwrap().distance, 0.0);
        assert!(!n.pairs.is_empty());
        assert!(witness(&a, &on, &n.pairs).is_none(), "touching blocks have no common inside");
    }

    #[test]
    fn overlapping_blocks_have_a_witness() {
        let a = block([0.0; 3], [40.0, 30.0, 10.0], 0.01);
        let b = block([10.0, 5.0, 9.0], [30.0, 25.0, 15.0], 0.01);
        let n = near(&a, &b, a.slack() + b.slack());
        let y = witness(&a, &b, &n.pairs).expect("a point inside both");
        assert!(a.proves_inside(y) && b.proves_inside(y));
        assert!(y.z > 9.0 && y.z < 10.0);
        // An overlap thinner than the bounds proves nothing either way.
        let thin = block([10.0, 5.0, 9.995], [30.0, 25.0, 15.0], 0.01);
        let n = near(&a, &thin, a.slack() + thin.slack());
        assert!(witness(&a, &thin, &n.pairs).is_none());
    }

    #[test]
    fn a_block_inside_another_has_a_witness() {
        let a = block([0.0; 3], [40.0, 30.0, 10.0], 0.01);
        let b = block([10.0, 10.0, 2.0], [20.0, 20.0, 8.0], 0.01);
        let n = near(&a, &b, a.slack() + b.slack());
        assert!(n.pairs.is_empty());
        assert!(witness(&a, &b, &n.pairs).is_some());
    }

    #[test]
    fn coincident_faces_share_a_point_only_where_they_overlap() {
        let a = block([0.0; 3], [40.0, 30.0, 10.0], 0.01);
        let b = block([10.0, 5.0, 10.0], [30.0, 25.0, 16.0], 0.01);
        // Face 1 is a's +z, face 0 is b's -z.
        let p = shared_point(&a, 1, &b, 0, 2).expect("a shared point");
        assert_eq!(p.z, 10.0);
        assert!(p.x > 10.0 && p.x < 30.0 && p.y > 5.0 && p.y < 25.0);
        // Side by side on the same plane they meet along an edge at most: nothing proven.
        let beside = block([40.0, 0.0, 10.0], [60.0, 30.0, 16.0], 0.01);
        assert!(shared_point(&a, 1, &beside, 0, 2).is_none());
    }
}
