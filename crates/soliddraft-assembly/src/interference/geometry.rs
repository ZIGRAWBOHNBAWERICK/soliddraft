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
//! also carries [`Solid::rounding`], an allowance for placing the mesh's coordinates in
//! the root's frame in binary64.
//!
//! On the placed mesh itself nothing is estimated. Every distance between mesh features is
//! a proven enclosure and every containment ray's crossing a proven sign, from
//! [`certified`]: binary64 with a running error bound, and exact arithmetic on the
//! coordinates wherever that bound leaves the answer open. Floating-point nearest points
//! appear only as places to start the witness search, which proves its own candidates.

use glam::{DVec2, DVec3};

use super::certified::{self, Crossing, Span};

/// Triangles per leaf of a solid's hierarchy.
const LEAF: usize = 4;

/// The allowance for binary64 rounding in placing a mesh, as a fraction of the largest
/// coordinate. Turning and moving a point loses a few units in the last place, about 1e-16
/// of the coordinate; this is seven orders of magnitude above that.
pub(crate) const ROUNDING: f64 = 1e-9;

/// The widest distance enclosure kept from binary64 before exact arithmetic tightens it,
/// as a fraction of the rounding allowance: far below anything a margin can notice.
const TIGHT: f64 = 1e-3;

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

    /// A lower bound on the distance between two boxes, zero when they overlap.
    pub fn distance(&self, other: &Bounds) -> f64 {
        gap_length((0..3).map(|i| {
            certified::sub_down(other.min[i], self.max[i]).max(certified::sub_down(self.min[i], other.max[i]))
        }))
    }

    /// A lower bound on the distance from `p` to the box, zero inside it.
    fn point_distance(&self, p: DVec3) -> f64 {
        gap_length((0..3).map(|i| certified::sub_down(self.min[i], p[i]).max(certified::sub_down(p[i], self.max[i]))))
    }

    /// Whether `p` lies in the closed box.
    fn holds(&self, p: DVec3) -> bool {
        (0..3).all(|i| self.min[i] <= p[i] && p[i] <= self.max[i])
    }

    /// Whether segment `pq` may meet the box. The slab test runs on the box widened by far
    /// more than its rounding, so a segment that meets it is never dropped.
    fn hit_by(&self, p: DVec3, q: DVec3) -> bool {
        let reach = p.abs().max(q.abs()).max(self.min.abs()).max(self.max.abs()).max_element();
        let pad = DVec3::splat(1e-9 * reach + f64::MIN_POSITIVE);
        let (min, max) = (self.min - pad, self.max + pad);
        let d = q - p;
        let (mut near, mut far) = (0.0_f64, 1.0_f64);
        for i in 0..3 {
            if d[i] == 0.0 {
                if p[i] < min[i] || p[i] > max[i] {
                    return false;
                }
                continue;
            }
            let (t0, t1) = ((min[i] - p[i]) / d[i], (max[i] - p[i]) / d[i]);
            near = near.max(t0.min(t1));
            far = far.min(t0.max(t1));
        }
        near <= far
    }
}

/// The length of a vector of nonnegative gaps, rounded down. A gap of zero or less is no
/// gap.
fn gap_length(gaps: impl Iterator<Item = f64>) -> f64 {
    let sum = gaps.map(|g| if g > 0.0 { g * g } else { 0.0 }).sum::<f64>();
    // Three squares and their sum lose at most four ulps of the sum together, unless a
    // square underflows, which leaves nothing worth bounding.
    if sum < 1e-290 {
        return 0.0;
    }
    (sum * (1.0 - 4.0 * f64::EPSILON)).sqrt().next_down().max(0.0)
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
    /// The binary64 allowance for placing this mesh's coordinates, in millimetres.
    pub rounding: f64,
    /// The box around every triangle.
    pub bounds: Bounds,
    nodes: Vec<Node>,
    order: Vec<u32>,
    /// The first triangle of each connected piece of the mesh, in triangle order.
    pieces: Vec<u32>,
}

/// What two solids' meshes say near each other.
pub(crate) struct Near {
    /// The meshes are at most this far apart: the least upper bound over the triangle
    /// pairs measured. `None` when no pair was.
    pub upper: Option<f64>,
    /// The meshes are at least this far apart: the least lower bound over the triangle
    /// pairs searched. The search reaches every pair that could be nearer than `upper`, so
    /// the nearest pair is among them.
    pub lower: f64,
    /// Triangle pairs whose lower bound is within the cutoff, nearest first: triangle of
    /// a, triangle of b, and the pair's distance. At most [`PAIRS_KEPT`] of them.
    pub pairs: Vec<(u32, u32, Span)>,
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
            pieces: Vec::new(),
        };
        solid.pieces = pieces(&solid.tris);
        if !solid.tris.is_empty() {
            solid.build(0, solid.tris.len());
            solid.bounds = solid.nodes[0].bounds;
        }
        solid
    }

    /// The first triangle of each connected piece of the mesh. A part's mesh holds one
    /// piece per shell, so a patterned part or a part with a cavity has several.
    pub fn pieces(&self) -> &[u32] {
        &self.pieces
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
        certified::add_up(self.chord, self.rounding)
    }

    /// The widest distance enclosure this mesh's queries keep without exact arithmetic.
    fn tight(&self) -> f64 {
        TIGHT * self.rounding
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

    /// A lower bound on the least distance from `p` to the mesh, searched no farther than
    /// `cutoff`: `cutoff` means nothing nearer.
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
                best = best.min(certified::point_triangle(p, &self.tris[t as usize], self.tight(), best).lo);
            }
        }
        best
    }

    /// Whether `p` is inside the closed mesh, by the parity of crossings along a ray. Each
    /// crossing is decided exactly. A ray through an edge or a corner, along a triangle's
    /// plane, or from a point on a triangle says nothing and the next direction is tried;
    /// `None` when every direction did.
    pub fn contains(&self, p: DVec3) -> Option<bool> {
        if self.nodes.is_empty() || !self.bounds.holds(p) {
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
        let reach = (self.bounds.max - self.bounds.min).length() + 1.0;
        DIRECTIONS.iter().find_map(|d| {
            // The ray as a segment to a point beyond the box, whatever its rounding.
            let dir = DVec3::from_array(*d);
            let mut length = reach;
            let mut q = p + dir * length;
            while self.bounds.holds(q) {
                length *= 2.0;
                q = p + dir * length;
            }
            self.parity(p, q)
        })
    }

    fn parity(&self, p: DVec3, q: DVec3) -> Option<bool> {
        let mut inside = false;
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let node = &self.nodes[i as usize];
            if !node.bounds.hit_by(p, q) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.left);
                stack.push(node.right);
                continue;
            }
            for &t in self.leaf(node) {
                match certified::crossing(p, q, &self.tris[t as usize]) {
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

/// The meshes of `a` and `b` near each other: proven bounds on the least distance between
/// them and the triangle pairs whose lower bound is within `cutoff`.
pub(crate) fn near(a: &Solid, b: &Solid, cutoff: f64) -> Near {
    let mut out = Near { upper: None, lower: f64::INFINITY, pairs: Vec::new() };
    if a.nodes.is_empty() || b.nodes.is_empty() {
        return out;
    }
    let tight = a.tight().min(b.tight());
    let mut best = f64::INFINITY;
    let mut stack = vec![(0u32, 0u32)];
    while let Some((i, j)) = stack.pop() {
        let (na, nb) = (&a.nodes[i as usize], &b.nodes[j as usize]);
        // A lower bound on every pair below: past `best`, none can be nearest or lower
        // the least lower bound, which is at most `best`.
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
                        // A pair whose lower bound reaches `best` changes neither bound,
                        // so its enclosure need not be tight.
                        let span = certified::triangles(&a.tris[ta as usize], &b.tris[tb as usize], tight, best);
                        if span.hi < best {
                            best = span.hi;
                            out.upper = Some(span.hi);
                        }
                        out.upper.get_or_insert(span.hi);
                        out.lower = out.lower.min(span.lo);
                        if span.lo <= cutoff && out.pairs.len() < PAIRS_KEPT {
                            out.pairs.push((ta, tb, span));
                        }
                    }
                }
            }
        }
    }
    // The traversal order depends on the stack only, so the kept pairs are deterministic;
    // sort them so that callers see them nearest first.
    out.pairs.sort_by(|p, q| p.2.hi.total_cmp(&q.2.hi).then((p.0, p.1).cmp(&(q.0, q.1))));
    out
}

/// A point proven inside both solids, searched from the given near pairs and from each
/// solid's own triangles. `None` proves nothing.
pub(crate) fn witness(a: &Solid, b: &Solid, pairs: &[(u32, u32, Span)]) -> Option<DVec3> {
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
    for (ta, tb, _) in pairs {
        let (x, y) = (&a.tris[*ta as usize], &b.tris[*tb as usize]);
        let (na, nb) = (normal(x), normal(y));
        let both = -(na + nb);
        let dirs = [both.normalize_or_zero(), -na, -nb];
        let (pa, pb) = nearest(x, y);
        let bases = [(pa + pb) * 0.5, pa, pb];
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
    // a triangle of every piece of each, then from spread-out triangles.
    for (s, other) in [(a, b), (b, a)] {
        let stride = (s.count() / 64).max(1);
        let firsts = s.pieces.iter().map(|t| *t as usize);
        for t in firsts.chain((0..s.count()).step_by(stride)) {
            let tri = &s.tris[t];
            let base = (tri[0] + tri[1] + tri[2]) / 3.0;
            if !other.bounds.holds(base) {
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
            // The centre is computed in binary64, so it is proven anew: inside a triangle of
            // each face, and farther than each face's bound from its boundary.
            let flat3 = |p: DVec2| DVec3::new(p.x, p.y, 0.0);
            let clear = |edges: &[[DVec2; 2]], slack: f64, tight: f64| {
                edges.iter().all(|[p, q]| certified::point_segment(flat3(c), flat3(*p), flat3(*q), tight).lo > slack)
            };
            let inside = |t: &[DVec2; 3]| certified::in_triangle_2d(c.to_array(), t.map(|p| p.to_array()));
            if inside(&px) && inside(&py) && clear(&ea, sa, a.tight()) && clear(&eb, sb, b.tight()) {
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

/// The first triangle of each connected piece of a mesh, in triangle order. Triangles
/// join where they share a corner, matched bit for bit, since the kernel places each
/// shared vertex by the same arithmetic. A piece the matching misses splits in two, which
/// only adds tests; two pieces never join unless their surfaces meet.
fn pieces(tris: &[[DVec3; 3]]) -> Vec<u32> {
    use std::collections::HashMap;
    use std::collections::hash_map::Entry;
    // Each triangle points towards the least triangle of its piece.
    let mut parent: Vec<u32> = (0..tris.len() as u32).collect();
    fn root(parent: &mut [u32], mut i: u32) -> u32 {
        while parent[i as usize] != i {
            parent[i as usize] = parent[parent[i as usize] as usize];
            i = parent[i as usize];
        }
        i
    }
    let mut first: HashMap<[u64; 3], u32> = HashMap::new();
    for (t, tri) in tris.iter().enumerate() {
        for p in tri {
            // Adding zero folds -0 into 0.
            match first.entry((*p + DVec3::ZERO).to_array().map(f64::to_bits)) {
                Entry::Vacant(e) => {
                    e.insert(t as u32);
                }
                Entry::Occupied(e) => {
                    let (x, y) = (root(&mut parent, *e.get()), root(&mut parent, t as u32));
                    parent[x.max(y) as usize] = x.min(y);
                }
            }
        }
    }
    (0..tris.len() as u32).filter(|t| parent[*t as usize] == *t).collect()
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

/// The unit outward normal of a triangle wound counter-clockwise seen from outside, or
/// zero for a degenerate one.
fn normal(t: &[DVec3; 3]) -> DVec3 {
    (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero()
}

/// Points near where two triangles come nearest, from binary64 arithmetic: places to start
/// a search, never a proof of a distance.
fn nearest(a: &[DVec3; 3], b: &[DVec3; 3]) -> (DVec3, DVec3) {
    let mut best = (f64::INFINITY, a[0], b[0]);
    let mut take = |x: DVec3, y: DVec3| {
        let d = x.distance(y);
        if d < best.0 {
            best = (d, x, y);
        }
    };
    for k in 0..3 {
        let (p, q) = (a[k], a[(k + 1) % 3]);
        take(p, on_triangle(p, b));
        take(on_triangle(b[k], a), b[k]);
        for j in 0..3 {
            let (x, y) = on_segments(p, q, b[j], b[(j + 1) % 3]);
            take(x, y);
        }
        // Where each triangle's edges cross the other's plane.
        for (s, t, flip) in [(a, b, false), (b, a, true)] {
            let (p, q) = (s[k], s[(k + 1) % 3]);
            let n = (t[1] - t[0]).cross(t[2] - t[0]);
            let (dp, dq) = ((p - t[0]).dot(n), (q - t[0]).dot(n));
            if (dp < 0.0) != (dq < 0.0) && dp != dq {
                let x = p + (q - p) * (dp / (dp - dq));
                let y = on_triangle(x, t);
                if flip { take(y, x) } else { take(x, y) }
            }
        }
    }
    (best.1, best.2)
}

/// A point of triangle `t` near `p` (Ericson, Real-Time Collision Detection 5.1.5).
fn on_triangle(p: DVec3, t: &[DVec3; 3]) -> DVec3 {
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
        return [on_segment(a, b, p), on_segment(b, c, p), on_segment(c, a, p)]
            .into_iter()
            .min_by(|x, y| x.distance(p).total_cmp(&y.distance(p)))
            .expect("three edges");
    }
    a + ab * (vb / denom) + ac * (vc / denom)
}

/// The point of segment `pq` nearest `x`, near enough.
fn on_segment(p: DVec3, q: DVec3, x: DVec3) -> DVec3 {
    let d = q - p;
    let len = d.length_squared();
    p + d * if len > 0.0 { ((x - p).dot(d) / len).clamp(0.0, 1.0) } else { 0.0 }
}

/// Points of two segments near where they come nearest (Ericson 5.1.9).
fn on_segments(p1: DVec3, q1: DVec3, p2: DVec3, q2: DVec3) -> (DVec3, DVec3) {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.length_squared(), d2.length_squared(), d2.dot(r));
    if a == 0.0 || e == 0.0 {
        return if a == 0.0 { (p1, on_segment(p2, q2, p1)) } else { (on_segment(p1, q1, p2), p2) };
    }
    let (b, c) = (d1.dot(d2), d1.dot(r));
    let denom = a * e - b * b;
    let mut s = if denom > 0.0 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
    let mut t = (b * s + f) / e;
    if t < 0.0 {
        t = 0.0;
        s = (-c / a).clamp(0.0, 1.0);
    } else if t > 1.0 {
        t = 1.0;
        s = ((b - c) / a).clamp(0.0, 1.0);
    }
    (p1 + d1 * s, p2 + d2 * t)
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

    /// One solid of several disjoint boxes, each `(lo, hi)`, their meshes in the order
    /// given: a single part with one piece per box.
    pub(crate) fn boxes(boxes: &[([f64; 3], [f64; 3])], chord: f64) -> Solid {
        let (mut tris, mut faces) = (Vec::new(), Vec::new());
        for (lo, hi) in boxes {
            let (t, f) = cuboid(DVec3::from_array(*lo), DVec3::from_array(*hi));
            tris.extend(t);
            faces.extend(f);
        }
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

    /// The certified distance between two triangles, tight to 1e-12 mm.
    fn between(a: &[DVec3; 3], b: &[DVec3; 3]) -> Span {
        certified::triangles(a, b, 1e-12, f64::INFINITY)
    }

    #[test]
    fn distances_between_triangles() {
        let a = [DVec3::ZERO, DVec3::X, DVec3::Y];
        let b = [DVec3::new(0.0, 0.0, 2.0), DVec3::new(1.0, 0.0, 2.0), DVec3::new(0.0, 1.0, 2.0)];
        let s = between(&a, &b);
        assert!(s.lo <= 2.0 && s.hi >= 2.0 && s.width() < 1e-13, "{s:?}");
        // Crossing triangles are zero apart.
        let c = [DVec3::new(0.2, 0.2, -1.0), DVec3::new(0.2, 0.2, 1.0), DVec3::new(0.3, -1.0, 0.0)];
        assert_eq!(between(&a, &c), Span::ZERO);
        // Edge to corner: d's upright edge passes (2, 0.5, 0), nearest a's corner (1, 0, 0).
        let d = [DVec3::new(2.0, 0.5, -1.0), DVec3::new(2.0, 0.5, 1.0), DVec3::new(3.0, 0.5, 0.0)];
        let s = between(&a, &d);
        assert!(s.lo <= 1.25_f64.sqrt() && s.hi >= 1.25_f64.sqrt() && s.width() < 1e-14, "{s:?}");
        // Touching along a shared edge: exactly zero.
        let e = [DVec3::X, DVec3::Y, DVec3::new(1.0, 1.0, 0.0)];
        assert_eq!(between(&a, &e), Span::ZERO);
    }

    #[test]
    fn near_parallel_segments_are_bounded_from_both_sides() {
        // Two 20 m segments crossing 0.01 mm apart at their middles: the ends are about
        // 0.0135 mm apart, more than the true 0.01, at any angle under a few microradians.
        let (p1, q1) = (DVec3::new(-10_000.0, 0.0, 0.0), DVec3::new(10_000.0, 0.0, 0.0));
        for angle in [0.0, 1e-12, 9e-7, 1.1e-6, 1e-3, 0.5] {
            let turn = glam::DQuat::from_rotation_z(angle);
            let lift = DVec3::new(0.0, 0.0, 0.01);
            let s = certified::segments(p1, q1, turn * p1 + lift, turn * q1 + lift, 1e-12);
            // The turned ends are rounded, so the lift is 0.01 to within an ulp of 1e4.
            assert!(s.lo <= 0.01 + 1e-11 && s.hi >= 0.01 - 1e-11 && s.width() <= 1e-12, "angle {angle}: {s:?}");
        }
        // End to end on one line, 3 mm apart.
        let s = certified::segments(DVec3::ZERO, DVec3::X * 10.0, DVec3::X * 13.0, DVec3::X * 20.0, 1e-12);
        assert!(s.lo <= 3.0 && s.hi >= 3.0 && s.width() < 1e-14, "{s:?}");
        // Side by side, overlapping along their length, 2 mm apart.
        let s = certified::segments(
            DVec3::ZERO,
            DVec3::X * 10.0,
            DVec3::new(-5.0, 2.0, 0.0),
            DVec3::new(5.0, 2.0, 0.0),
            1e-12,
        );
        assert!(s.lo <= 2.0 && s.hi >= 2.0 && s.width() < 1e-14, "{s:?}");
    }

    #[test]
    fn near_parallel_ridges_are_their_true_distance_apart() {
        // The review's closed prisms: the meshes are 0.01 mm apart where the ridges cross.
        for angle in [0.0, 9e-7, 1.1e-6] {
            let (a, b) = tests_support::ridges(angle, 0.001);
            let n = near(&a, &b, 0.1);
            let upper = n.upper.expect("a pair measured");
            assert!(n.lower <= 0.01 + 1e-11 && n.lower > 0.01 - 1e-9, "angle {angle}: lower {}", n.lower);
            assert!(upper >= n.lower && upper < 0.01 + 1e-9, "angle {angle}: upper {upper}");
        }
    }

    #[test]
    fn a_long_skinny_face_is_its_true_distance_from_a_block_over_it() {
        // The review's case: A's top face is a 30 m by 0.0625 mm strip in two slivers, and
        // B's underside is 0.015625 mm over its middle. Binary64 projection onto a sliver
        // lands 0.07 mm away; the true distance is the gap.
        let a = block([0.0, 0.0, -1.0], [30_000.0, 0.0625, 0.0], 0.001);
        let b = block([21_000.0, 0.015625, 0.015625], [21_001.0, 0.0234375, 1.0], 0.001);
        for (x, y) in [(&a, &b), (&b, &a)] {
            let n = near(x, y, 0.1);
            let upper = n.upper.expect("a pair measured");
            assert!(n.lower <= 0.015625 && upper >= 0.015625, "{} to {upper}", n.lower);
            assert!(upper - n.lower < 1e-12, "{} to {upper}", n.lower);
        }
        // The point over the sliver's middle, as the review gives it.
        let t = [DVec3::ZERO, DVec3::new(30_000.0, 0.0, 0.0), DVec3::new(30_000.0, 0.0625, 0.0)];
        let p = DVec3::new(21_000.0, 0.015625, 0.015625);
        let s = certified::point_triangle(p, &t, 1e-12, f64::INFINITY);
        assert!(s.lo <= 0.015625 && s.hi >= 0.015625 && s.width() < 1e-15, "{s:?}");
        let s = block([21_000.0, 0.0, -1.0], [21_001.0, 1.0, 0.0], 0.001);
        assert!(s.surface_distance(p, 1.0) <= 0.015625);
    }

    #[test]
    fn a_sliver_triangle_in_a_closed_mesh_still_counts_its_crossing() {
        // A box 30 m long and 1e-6 mm across in y: its long sides are slivers whose angle
        // at the corner is about 3e-11 rad. A point inside it is inside.
        let s = block([0.0, 0.0, 0.0], [30_000.0, 1e-6, 1.0], 0.001);
        assert_eq!(s.contains(DVec3::new(12_345.0, 5e-7, 0.5)), Some(true));
        assert_eq!(s.contains(DVec3::new(12_345.0, 2e-6, 0.5)), Some(false));
    }

    #[test]
    fn each_connected_piece_of_a_mesh_is_found() {
        let one = block([0.0; 3], [1.0; 3], 0.001);
        assert_eq!(one.pieces(), &[0]);
        let three = tests_support::boxes(
            &[([0.0; 3], [1.0; 3]), ([5.0, 0.0, 0.0], [6.0, 1.0, 1.0]), ([10.0, 0.0, 0.0], [11.0, 1.0, 1.0])],
            0.001,
        );
        assert_eq!(three.pieces(), &[0, 12, 24]);
    }

    #[test]
    fn separated_blocks_are_far_and_touching_blocks_are_near() {
        let a = block([0.0; 3], [40.0, 30.0, 10.0], 0.01);
        let b = block([10.0, 5.0, 12.0], [30.0, 25.0, 18.0], 0.01);
        let n = near(&a, &b, 0.1);
        let upper = n.upper.unwrap();
        assert!(n.lower <= 2.0 && upper >= 2.0 && upper - n.lower < 1e-12, "{} to {upper}", n.lower);
        assert!(n.pairs.is_empty());
        let on = block([10.0, 5.0, 10.0], [30.0, 25.0, 16.0], 0.01);
        let n = near(&a, &on, 0.1);
        assert_eq!((n.lower, n.upper), (0.0, Some(0.0)));
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
