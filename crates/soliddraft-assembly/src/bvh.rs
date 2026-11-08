//! A bounding volume hierarchy over occurrences, on the CPU.
//!
//! The id buffer says which occurrence is under the cursor; the hierarchy answers the
//! questions that follow and the ones the id buffer cannot: the exact point a ray hits,
//! every occurrence in a box, every occurrence a camera can see. It holds part
//! occurrences only (sub-assemblies are drawn through their parts).
//!
//! Boxes are `f32`, relative to an origin chosen when the hierarchy is built, so a
//! million occurrences cost about 32 MB of nodes. Built with binned surface-area splits;
//! after a move the boxes are refitted without rebuilding the tree, which stays correct
//! (every box still holds its contents) and degrades in quality only after large moves.

use glam::{DVec3, Vec3};

use crate::occurrence::{OccurrenceIndex, OccurrenceTable};
use crate::placement::Aabb;

/// How many occurrences a leaf holds at most.
const LEAF: usize = 4;
/// Split candidates per axis.
const BINS: usize = 12;

#[derive(Clone, Copy, Debug)]
struct Node {
    min: Vec3,
    /// For a leaf, the first item; for an inner node, the left child (the right one
    /// follows it).
    first: u32,
    max: Vec3,
    /// Items in a leaf; 0 for an inner node.
    count: u32,
}

/// A hierarchy over the part occurrences of an [`OccurrenceTable`].
#[derive(Clone, Debug, Default)]
pub struct Bvh {
    origin: DVec3,
    nodes: Vec<Node>,
    /// Occurrence rows, in leaf order.
    items: Vec<u32>,
    /// For each row in the table, its leaf node, or `u32::MAX` when it is not in the
    /// hierarchy.
    leaf_of: Vec<u32>,
    parent: Vec<u32>,
}

/// A ray in the root's frame.
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    /// Where it starts.
    pub origin: DVec3,
    /// Its direction, not necessarily unit.
    pub direction: DVec3,
}

/// Six planes bounding what a camera sees, each `(normal, d)` with the inside where
/// `normal . p + d >= 0`, in the root's frame.
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    /// The planes.
    pub planes: [(DVec3, f64); 6],
}

impl Frustum {
    /// The frustum of a view-projection matrix (clip depth 0 to 1, as wgpu uses).
    pub fn from_view_proj(m: glam::DMat4) -> Frustum {
        let r = |i: usize| m.row(i);
        let planes = [r(3) + r(0), r(3) - r(0), r(3) + r(1), r(3) - r(1), r(2), r(3) - r(2)].map(|p| {
            let n = p.truncate();
            let len = n.length().max(1e-300);
            (n / len, p.w / len)
        });
        Frustum { planes }
    }
}

/// The box from `min` to `max`, relative to `origin` in f32, rounded outward: read back
/// as `stored + origin` in f64, as [`Bvh::node_box`] does, it holds the original. Rounding
/// to nearest could shrink it, and a query would then prune a row it meets.
fn to_f32(min: DVec3, max: DVec3, origin: DVec3) -> (Vec3, Vec3) {
    let (mut lo, mut hi) = ((min - origin).as_vec3(), (max - origin).as_vec3());
    for i in 0..3 {
        // Each step moves one f32 ulp, and the nearest f32 is within one ulp of the
        // f64 offset, so this loops at most twice for finite inputs.
        while lo[i] > f32::MIN && lo[i] as f64 + origin[i] > min[i] {
            lo[i] = lo[i].next_down();
        }
        while hi[i] < f32::MAX && (hi[i] as f64 + origin[i]) < max[i] {
            hi[i] = hi[i].next_up();
        }
    }
    (lo, hi)
}

impl Bvh {
    /// Builds the hierarchy over every part occurrence that is not removed and has
    /// bounds. Hidden and suppressed ones are included, so showing or unsuppressing them
    /// needs no rebuild: queries leave out what is suppressed or removed by its flags at
    /// the time, and skip hidden ones by flag when they want to.
    pub fn build(table: &OccurrenceTable) -> Bvh {
        Bvh::build_from(table.all_bounds(), table.all_flags(), table.bounds(OccurrenceIndex::ROOT).center())
    }

    /// Builds from a copy of a table's bounds and flags, as a background build does,
    /// with boxes relative to `origin`.
    pub fn build_from(bounds: &[Aabb], flags: &[crate::occurrence::Flags], origin: DVec3) -> Bvh {
        let len = bounds.len();
        let rows: Vec<u32> = (0..len as u32)
            .filter(|&r| {
                let f = flags[r as usize];
                !f.has(crate::occurrence::Flags::ASSEMBLY)
                    && !f.has(crate::occurrence::Flags::REMOVED)
                    && !bounds[r as usize].is_empty()
            })
            .collect();
        let origin = if origin.is_finite() { origin } else { DVec3::ZERO };
        let mut b = Bvh { origin, nodes: Vec::with_capacity(rows.len() / 2 + 1), items: rows, ..Bvh::default() };
        let boxes: Vec<(Vec3, Vec3)> = (0..len)
            .map(|r| {
                let a = bounds[r];
                if a.is_empty() { (Vec3::ZERO, Vec3::ZERO) } else { to_f32(a.min, a.max, origin) }
            })
            .collect();
        if !b.items.is_empty() {
            b.nodes.push(Node { min: Vec3::ZERO, first: 0, max: Vec3::ZERO, count: 0 });
            b.parent.push(u32::MAX);
            let n = b.items.len();
            b.split(0, 0, n, &boxes);
        }
        b.leaf_of = vec![u32::MAX; len];
        for (i, node) in b.nodes.iter().enumerate() {
            if node.count > 0 {
                for &r in &b.items[node.first as usize..(node.first + node.count) as usize] {
                    b.leaf_of[r as usize] = i as u32;
                }
            }
        }
        b
    }

    fn split(&mut self, node: usize, start: usize, end: usize, boxes: &[(Vec3, Vec3)]) {
        let (mut lo, mut hi) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        let (mut clo, mut chi) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        for &r in &self.items[start..end] {
            let (a, b) = boxes[r as usize];
            lo = lo.min(a);
            hi = hi.max(b);
            let c = (a + b) * 0.5;
            clo = clo.min(c);
            chi = chi.max(c);
        }
        self.nodes[node].min = lo;
        self.nodes[node].max = hi;
        let n = end - start;
        let extent = chi - clo;
        if n <= LEAF || extent.max_element() <= 0.0 {
            self.nodes[node].first = start as u32;
            self.nodes[node].count = n as u32;
            return;
        }
        // Binned surface-area heuristic on the axis of largest centroid spread.
        let axis = if extent.x >= extent.y && extent.x >= extent.z {
            0
        } else if extent.y >= extent.z {
            1
        } else {
            2
        };
        let bin_of = |r: u32| {
            let (a, b) = boxes[r as usize];
            let c = ((a + b) * 0.5)[axis];
            (((c - clo[axis]) / extent[axis] * BINS as f32) as usize).min(BINS - 1)
        };
        let mut bins = [(0usize, Vec3::INFINITY, Vec3::NEG_INFINITY); BINS];
        for &r in &self.items[start..end] {
            let (a, b) = boxes[r as usize];
            let k = bin_of(r);
            bins[k].0 += 1;
            bins[k].1 = bins[k].1.min(a);
            bins[k].2 = bins[k].2.max(b);
        }
        let area = |lo: Vec3, hi: Vec3| {
            let s = (hi - lo).max(Vec3::ZERO);
            s.x * s.y + s.y * s.z + s.z * s.x
        };
        let mut best = (f32::INFINITY, BINS / 2);
        for split in 1..BINS {
            let (mut nl, mut ll, mut lh) = (0, Vec3::INFINITY, Vec3::NEG_INFINITY);
            for b in &bins[..split] {
                nl += b.0;
                ll = ll.min(b.1);
                lh = lh.max(b.2);
            }
            let (mut nr, mut rl, mut rh) = (0, Vec3::INFINITY, Vec3::NEG_INFINITY);
            for b in &bins[split..] {
                nr += b.0;
                rl = rl.min(b.1);
                rh = rh.max(b.2);
            }
            if nl == 0 || nr == 0 {
                continue;
            }
            let cost = nl as f32 * area(ll, lh) + nr as f32 * area(rl, rh);
            if cost < best.0 {
                best = (cost, split);
            }
        }
        let mut mid = start;
        if best.0.is_finite() {
            for i in start..end {
                if bin_of(self.items[i]) < best.1 {
                    self.items.swap(i, mid);
                    mid += 1;
                }
            }
        }
        if mid == start || mid == end {
            // Every centre in one bin: split by count.
            mid = start + n / 2;
            self.items[start..end].select_nth_unstable_by(n / 2, |a, b| {
                let ca = boxes[*a as usize].0[axis] + boxes[*a as usize].1[axis];
                let cb = boxes[*b as usize].0[axis] + boxes[*b as usize].1[axis];
                ca.total_cmp(&cb)
            });
        }
        let left = self.nodes.len();
        let blank = Node { min: Vec3::ZERO, first: 0, max: Vec3::ZERO, count: 0 };
        self.nodes.push(blank);
        self.nodes.push(blank);
        self.parent.push(node as u32);
        self.parent.push(node as u32);
        self.nodes[node].first = left as u32;
        self.nodes[node].count = 0;
        self.split(left, start, mid, boxes);
        self.split(left + 1, mid, end, boxes);
    }

    /// How many occurrences the hierarchy holds.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when it holds none.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Roughly how many bytes it holds.
    pub fn heap_bytes(&self) -> usize {
        self.nodes.len() * (std::mem::size_of::<Node>() + 4) + self.items.len() * 4 + self.leaf_of.len() * 4
    }

    /// Updates the boxes after the rows in `moved` changed bounds: each one's leaf and
    /// its ancestors. With many rows moved a full pass is cheaper and is used instead.
    pub fn refit(&mut self, table: &OccurrenceTable, moved: &[OccurrenceIndex]) {
        self.refit_from(table.all_bounds(), moved);
    }

    fn refit_from(&mut self, bounds: &[Aabb], moved: &[OccurrenceIndex]) {
        if self.nodes.is_empty() {
            return;
        }
        if moved.len() * 16 > self.nodes.len() {
            self.refit_all_from(bounds);
            return;
        }
        let mut dirty: Vec<u32> =
            moved.iter().filter_map(|r| self.leaf_of.get(r.0 as usize).copied()).filter(|&l| l != u32::MAX).collect();
        dirty.sort_unstable();
        dirty.dedup();
        // Children always come after their parents, so going from the highest index
        // down visits every node after the nodes below it.
        let mut queue = std::collections::BTreeSet::from_iter(dirty);
        while let Some(n) = queue.pop_last() {
            self.fit(n as usize, bounds);
            let p = self.parent[n as usize];
            if p != u32::MAX {
                queue.insert(p);
            }
        }
    }

    /// Updates every box from the table.
    pub fn refit_all(&mut self, table: &OccurrenceTable) {
        self.refit_all_from(table.all_bounds());
    }

    fn refit_all_from(&mut self, bounds: &[Aabb]) {
        for n in (0..self.nodes.len()).rev() {
            self.fit(n, bounds);
        }
    }

    fn fit(&mut self, n: usize, bounds: &[Aabb]) {
        let node = self.nodes[n];
        let (mut lo, mut hi) = (Vec3::INFINITY, Vec3::NEG_INFINITY);
        if node.count > 0 {
            for &r in &self.items[node.first as usize..(node.first + node.count) as usize] {
                let b = bounds[r as usize];
                if !b.is_empty() {
                    let (a, b) = to_f32(b.min, b.max, self.origin);
                    lo = lo.min(a);
                    hi = hi.max(b);
                }
            }
        } else {
            for c in [node.first as usize, node.first as usize + 1] {
                lo = lo.min(self.nodes[c].min);
                hi = hi.max(self.nodes[c].max);
            }
        }
        self.nodes[n].min = lo;
        self.nodes[n].max = hi;
    }

    fn node_box(&self, n: usize) -> Aabb {
        let node = &self.nodes[n];
        Aabb { min: node.min.as_dvec3() + self.origin, max: node.max.as_dvec3() + self.origin }
    }

    /// Visits the rows in leaves whose boxes `keep` accepts, pruning subtrees it
    /// rejects.
    fn visit(&self, mut keep: impl FnMut(&Aabb) -> bool, mut each: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            if !keep(&self.node_box(n)) {
                continue;
            }
            let node = &self.nodes[n];
            if node.count > 0 {
                for &r in &self.items[node.first as usize..(node.first + node.count) as usize] {
                    each(r);
                }
            } else {
                stack.push(node.first as usize);
                stack.push(node.first as usize + 1);
            }
        }
    }

    /// Occurrences whose bounds overlap `region`.
    pub fn in_box(&self, table: &OccurrenceTable, region: &Aabb) -> Vec<OccurrenceIndex> {
        let mut out = Vec::new();
        self.visit(
            |b| b.intersects(region),
            |r| {
                if live(table, r) && table.bounds(OccurrenceIndex(r)).intersects(region) {
                    out.push(OccurrenceIndex(r));
                }
            },
        );
        out
    }

    /// Occurrences whose bounds a camera may see.
    pub fn in_frustum(&self, table: &OccurrenceTable, frustum: &Frustum) -> Vec<OccurrenceIndex> {
        let inside = |b: &Aabb| {
            frustum.planes.iter().all(|(n, d)| {
                // The box corner furthest along the plane's normal.
                let p = DVec3::new(
                    if n.x >= 0.0 { b.max.x } else { b.min.x },
                    if n.y >= 0.0 { b.max.y } else { b.min.y },
                    if n.z >= 0.0 { b.max.z } else { b.min.z },
                );
                n.dot(p) + d >= 0.0
            })
        };
        let mut out = Vec::new();
        self.visit(inside, |r| {
            if live(table, r) && inside(&table.bounds(OccurrenceIndex(r))) {
                out.push(OccurrenceIndex(r));
            }
        });
        out
    }

    /// Occurrences whose bounds `ray` passes through, nearest entry first, with the
    /// ray parameter where it enters each box.
    pub fn along_ray(&self, table: &OccurrenceTable, ray: &Ray) -> Vec<(OccurrenceIndex, f64)> {
        let mut out = Vec::new();
        self.visit(
            |b| ray_box(ray, b).is_some(),
            |r| {
                if let Some(t) = ray_box(ray, &table.bounds(OccurrenceIndex(r))).filter(|_| live(table, r)) {
                    out.push((OccurrenceIndex(r), t));
                }
            },
        );
        out.sort_by(|a, b| a.1.total_cmp(&b.1));
        out
    }
}

/// Whether a leaf's row counts for a query now: not suppressed or removed since the
/// hierarchy was built. A row past the table's end belongs to a table it was not built
/// for and counts as gone.
fn live(table: &OccurrenceTable, r: u32) -> bool {
    (r as usize) < table.len() && {
        let f = table.flags(OccurrenceIndex(r));
        !f.has(crate::occurrence::Flags::SUPPRESSED) && !f.has(crate::occurrence::Flags::REMOVED)
    }
}

/// Where a ray enters a box (0 when it starts inside), if it meets it ahead. The box is
/// closed: a ray along a face or an edge meets it.
pub fn ray_box(ray: &Ray, b: &Aabb) -> Option<f64> {
    if b.is_empty() || !ray.origin.is_finite() || !ray.direction.is_finite() {
        return None;
    }
    let (mut near, mut far) = (0.0f64, f64::INFINITY);
    for i in 0..3 {
        let (o, d) = (ray.origin[i], ray.direction[i]);
        if d == 0.0 {
            // Parallel to this slab: inside it for the whole ray, or never. Dividing
            // would give 0 / 0 for an origin on a face.
            if o < b.min[i] || o > b.max[i] {
                return None;
            }
            continue;
        }
        // Divide rather than multiply by the reciprocal, which overflows for a tiny
        // component and turns an origin on a face into 0 * infinity.
        let (t1, t2) = ((b.min[i] - o) / d, (b.max[i] - o) / d);
        let (t1, t2) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
        near = near.max(t1);
        far = far.min(t2);
    }
    (near <= far).then_some(near)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_enter_boxes() {
        let b = Aabb { min: DVec3::ZERO, max: DVec3::ONE };
        let r = Ray { origin: DVec3::new(-1.0, 0.5, 0.5), direction: DVec3::X };
        assert_eq!(ray_box(&r, &b), Some(1.0));
        let away = Ray { origin: DVec3::new(-1.0, 0.5, 0.5), direction: -DVec3::X };
        assert_eq!(ray_box(&away, &b), None);
        let inside = Ray { origin: DVec3::splat(0.5), direction: DVec3::Z };
        assert_eq!(ray_box(&inside, &b), Some(0.0));
    }

    /// A ray along one axis that lies on a face or an edge of the box, or runs through
    /// it, enters it; one a hair outside a face does not. Both signs of zero in the
    /// other components, and both directions along each axis.
    #[test]
    fn rays_parallel_to_faces_enter_boxes() {
        let b = Aabb { min: DVec3::ZERO, max: DVec3::ONE };
        let outside = [-f64::EPSILON, 1.0 + f64::EPSILON];
        for axis in 0..3 {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            for sign in [1.0, -1.0] {
                for zero in [0.0, -0.0] {
                    let mut direction = DVec3::splat(zero);
                    direction[axis] = sign;
                    // On the min face, through the middle, on the max face.
                    for cu in [0.0, 0.5, 1.0] {
                        for cv in [0.0, 0.5, 1.0] {
                            let mut origin = DVec3::ZERO;
                            origin[u] = cu;
                            origin[v] = cv;
                            origin[axis] = if sign > 0.0 { -1.0 } else { 2.0 };
                            let ray = Ray { origin, direction };
                            assert_eq!(ray_box(&ray, &b), Some(1.0), "{ray:?}");
                            origin[axis] = 0.5;
                            assert_eq!(ray_box(&Ray { origin, direction }, &b), Some(0.0), "starts inside, {ray:?}");
                            origin[axis] = if sign > 0.0 { 0.0 } else { 1.0 };
                            assert_eq!(ray_box(&Ray { origin, direction }, &b), Some(0.0), "starts on a face, {ray:?}");
                            origin[axis] = if sign > 0.0 { 2.0 } else { -1.0 };
                            assert_eq!(ray_box(&Ray { origin, direction }, &b), None, "past the box, {ray:?}");
                        }
                    }
                    for off in outside {
                        let mut origin = DVec3::splat(0.5);
                        origin[axis] = if sign > 0.0 { -1.0 } else { 2.0 };
                        origin[u] = off;
                        let ray = Ray { origin, direction };
                        assert_eq!(ray_box(&ray, &b), None, "outside a face, {ray:?}");
                    }
                }
            }
        }
        // A ray in a face plane but slanted across it still meets the face.
        let slant = Ray { origin: DVec3::new(-1.0, 0.0, -1.0), direction: DVec3::new(1.0, 0.0, 1.0) };
        assert_eq!(ray_box(&slant, &b), Some(1.0));
        // A direction so small its reciprocal overflows, from a face.
        let tiny = Ray { origin: DVec3::new(-1.0, 0.0, 0.5), direction: DVec3::new(1.0, 1e-320, 0.0) };
        assert_eq!(ray_box(&tiny, &b), Some(1.0));
    }

    /// Every node's box, as the queries rebuild it in f64, holds the f64 bounds of every
    /// row below it.
    fn assert_encloses(bvh: &Bvh, bounds: &[Aabb]) {
        let mut stack = vec![(0usize, Vec::new())];
        while let Some((n, mut path)) = stack.pop() {
            path.push(n);
            let node = bvh.nodes[n];
            if node.count > 0 {
                for &r in &bvh.items[node.first as usize..(node.first + node.count) as usize] {
                    for &a in &path {
                        let b = bvh.node_box(a);
                        let row = bounds[r as usize];
                        assert!(
                            b.min.cmple(row.min).all() && row.max.cmple(b.max).all(),
                            "node {a} {b:?} does not hold row {r} {row:?}"
                        );
                    }
                }
            } else {
                stack.push((node.first as usize, path.clone()));
                stack.push((node.first as usize + 1, path));
            }
        }
    }

    fn rows_on_ray(bvh: &Bvh, bounds: &[Aabb], ray: &Ray) -> Vec<u32> {
        let mut out = Vec::new();
        bvh.visit(|b| ray_box(ray, b).is_some(), |r| out.extend(ray_box(ray, &bounds[r as usize]).map(|_| r)));
        out.sort_unstable();
        out
    }

    fn rows_in_box(bvh: &Bvh, bounds: &[Aabb], region: &Aabb) -> Vec<u32> {
        let mut out = Vec::new();
        bvh.visit(|b| b.intersects(region), |r| out.extend(bounds[r as usize].intersects(region).then_some(r)));
        out.sort_unstable();
        out
    }

    /// A box whose f64 bounds f32 rounds inward: x from -h to h with h = 1 + 2^-25,
    /// stored relative to the origin as -1 to 1. A ray strictly inside the true x range
    /// but outside the rounded one must still find it, after a build and after a refit.
    /// Unit boxes further along y make enough nodes for a refit of one row to touch only
    /// its own leaf and ancestors.
    #[test]
    fn stored_boxes_hold_their_rows_after_rounding() {
        let unit = Aabb { min: -DVec3::ONE, max: DVec3::ONE };
        let others = (1..64).map(|k| {
            let y = DVec3::new(0.0, 10.0 * k as f64, 0.0);
            Aabb { min: unit.min + y, max: unit.max + y }
        });
        let flags = vec![crate::occurrence::Flags::default(); 64];
        for h in [1.0 + 2f64.powi(-25), 2f64.powi(20) + 2f64.powi(-5)] {
            let row = Aabb { min: DVec3::new(-h, -1.0, -1.0), max: DVec3::new(h, 1.0, 1.0) };
            let start: Vec<Aabb> = std::iter::once(unit).chain(others.clone()).collect();
            let bounds: Vec<Aabb> = std::iter::once(row).chain(others.clone()).collect();
            let x = (h + h as f32 as f64) / 2.0;
            assert!(x < h && x > h as f32 as f64, "between the rounded and the true bound");
            let ray = Ray { origin: DVec3::new(x, 0.0, 2.0), direction: -DVec3::Z };
            let region = Aabb { min: DVec3::new(x, -0.5, -0.5), max: DVec3::new(x, 0.5, 0.5) };
            let mirrored = Ray { origin: DVec3::new(-x, 0.0, 2.0), ..ray };
            let check = |bvh: &Bvh| {
                assert_encloses(bvh, &bounds);
                assert_eq!(rows_on_ray(bvh, &bounds, &ray), vec![0], "h = {h}");
                assert_eq!(rows_on_ray(bvh, &bounds, &mirrored), vec![0], "h = {h}, min side");
                assert_eq!(rows_in_box(bvh, &bounds, &region), vec![0], "h = {h}");
            };
            check(&Bvh::build_from(&bounds, &flags, DVec3::ZERO));
            // Built around a unit box, then refitted to the awkward one, by its own
            // path up the tree and by a full pass.
            let mut one = Bvh::build_from(&start, &flags, DVec3::ZERO);
            assert!(one.nodes.len() > 16, "a refit of one row takes the partial path");
            one.refit_from(&bounds, &[OccurrenceIndex(0)]);
            check(&one);
            let mut all = Bvh::build_from(&start, &flags, DVec3::ZERO);
            all.refit_all_from(&bounds);
            check(&all);
        }
    }

    mod generated {
        use super::*;
        use proptest::prelude::*;

        fn config() -> ProptestConfig {
            let cases = std::env::var("PROPTEST_CASES").ok().and_then(|c| c.parse().ok()).unwrap_or(200);
            ProptestConfig { cases, ..ProptestConfig::default() }
        }

        /// A coordinate at a random scale, so that f32 cannot hold most of them.
        fn coordinate() -> impl Strategy<Value = f64> {
            (-1.0e7f64..1.0e7, 0u32..4).prop_map(|(x, scale)| x / 10f64.powi(scale as i32 * 2))
        }

        fn aabb() -> impl Strategy<Value = Aabb> {
            (prop::array::uniform3(coordinate()), prop::array::uniform3(0.0f64..50.0)).prop_map(|(c, e)| {
                let min = DVec3::from_array(c);
                Aabb { min, max: min + DVec3::from_array(e) }
            })
        }

        proptest! {
            #![proptest_config(config())]

            /// Queries through the hierarchy find exactly the rows a check of every row
            /// finds, with rays and boxes placed on the rows' own faces and edges.
            #[test]
            fn queries_match_a_check_of_every_row(
                bounds in prop::collection::vec(aabb(), 1..40),
                moved in prop::collection::vec((any::<prop::sample::Index>(), aabb()), 0..6),
                origin in prop::array::uniform3(coordinate()),
                picks in prop::collection::vec((any::<prop::sample::Index>(), 0usize..3, any::<bool>(), 0u8..4), 1..12),
            ) {
                let flags = vec![crate::occurrence::Flags::default(); bounds.len()];
                let mut bvh = Bvh::build_from(&bounds, &flags, DVec3::from_array(origin));
                assert_encloses(&bvh, &bounds);
                let mut bounds = bounds;
                let rows: Vec<OccurrenceIndex> = moved
                    .iter()
                    .map(|(i, b)| {
                        let r = i.index(bounds.len());
                        bounds[r] = *b;
                        OccurrenceIndex(r as u32)
                    })
                    .collect();
                bvh.refit_from(&bounds, &rows);
                assert_encloses(&bvh, &bounds);
                for (i, axis, max_side, kind) in picks {
                    let b = bounds[i.index(bounds.len())];
                    // A point on a face or edge of a row's box.
                    let mut p = (b.min + b.max) * 0.5;
                    p[axis] = if max_side { b.max[axis] } else { b.min[axis] };
                    if kind == 1 {
                        p[(axis + 1) % 3] = b.min[(axis + 1) % 3];
                    }
                    let travel = (axis + 1 + kind as usize % 2) % 3;
                    let mut direction = DVec3::ZERO;
                    direction[travel] = if kind < 2 { 1.0 } else { -1.0 };
                    let mut origin = p;
                    origin[travel] -= direction[travel] * 100.0;
                    let ray = Ray { origin, direction };
                    let brute: Vec<u32> =
                        (0..bounds.len() as u32).filter(|&r| ray_box(&ray, &bounds[r as usize]).is_some()).collect();
                    prop_assert!(brute.contains(&(i.index(bounds.len()) as u32)), "the ray meets its own row");
                    prop_assert_eq!(rows_on_ray(&bvh, &bounds, &ray), brute);
                    let region = Aabb { min: p, max: p };
                    let brute: Vec<u32> =
                        (0..bounds.len() as u32).filter(|&r| bounds[r as usize].intersects(&region)).collect();
                    prop_assert_eq!(rows_in_box(&bvh, &bounds, &region), brute);
                }
            }
        }
    }

    #[test]
    fn frustum_planes_hold_what_the_camera_sees() {
        let proj = glam::camera::rh::proj::directx::perspective(1.0, 1.0, 1.0, 100.0);
        let view = glam::camera::rh::view::look_at_mat4(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, Vec3::Y);
        let f = Frustum::from_view_proj((proj * view).as_dmat4());
        let ahead = DVec3::ZERO;
        let behind = DVec3::new(0.0, 0.0, 20.0);
        assert!(f.planes.iter().all(|(n, d)| n.dot(ahead) + d >= 0.0));
        assert!(f.planes.iter().any(|(n, d)| n.dot(behind) + d < 0.0));
    }
}
