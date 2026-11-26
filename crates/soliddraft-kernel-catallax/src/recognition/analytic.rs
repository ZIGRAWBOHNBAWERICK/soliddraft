//! Conservative rules for connected analytic boundaries. A refused or inexact predicate
//! leaves the candidate unclassified; numerical values are projected only after a match.

use std::cmp::Ordering::{self, Equal, Greater, Less};
use std::collections::{BTreeMap, BTreeSet};

use catallax_numbers::{Decider, Outcome, Real, StrictSign};
use catallax_shapes::{Circle, Curve, Plane, Point3, Surface, Vector3};
use catallax_topology::{CoedgeId, EdgeId, FaceId, Loop, Model, Sense};
use soliddraft_kernel_api::recognition::{
    CompoundHead, CompoundHole, EdgeTreatment, EdgeTreatmentKind, Recess, Recognised,
};
use soliddraft_kernel_api::{FaceId as AppFace, Vec3};

use crate::convert::Ids;

pub(super) fn recognise(model: &Model, ids: &Ids, dec: &mut Decider) -> Recognised {
    let mut query = Query { model, ids, dec };
    let mut seen = BTreeSet::new();
    let mut rounds = Vec::new();
    let mut out = Recognised::default();
    for &face in ids.faces.keys() {
        if !query.exact(face) {
            continue;
        }
        if !seen.contains(&face) && query.round_axis(face).is_some() {
            let faces = query.connected_round(face, &mut seen);
            if let Some(rims) = query.rims(&faces) {
                rounds.push(Round { faces, rims });
            }
        }
        if let Some(recess) = query.recess(face) {
            out.recesses.push(recess);
        }
        if let Some(treatment) = query.edge_treatment(face) {
            out.edge_treatments.push(treatment);
        }
    }

    let mut by_wall = BTreeMap::new();
    for (index, round) in rounds.iter().enumerate() {
        for &face in &round.faces {
            by_wall.insert(face, index);
        }
    }
    let mut candidates = Vec::new();
    let mut uses = vec![0; rounds.len()];
    for (pilot_index, pilot) in rounds.iter().enumerate() {
        if !matches!(query.surface(pilot.faces[0]), Surface::Cylinder(_)) {
            continue;
        }
        for join in 0..2 {
            let Some((head_index, feature)) = query.compound_hole(&rounds, &by_wall, pilot_index, join) else {
                continue;
            };
            uses[pilot_index] += 1;
            uses[head_index] += 1;
            candidates.push((pilot_index, head_index, feature));
        }
    }
    // A wall that belongs to two proposals does not determine a unique head and pilot.
    out.compound_holes =
        candidates.into_iter().filter_map(|(p, h, f)| (uses[p] == 1 && uses[h] == 1).then_some(f)).collect();
    out
}

struct Query<'a> {
    model: &'a Model,
    ids: &'a Ids,
    dec: &'a mut Decider,
}

struct Round {
    faces: Vec<FaceId>,
    rims: [Rim; 2],
}

struct Rim {
    circle: Circle,
    edges: BTreeSet<EdgeId>,
    neighbors: BTreeSet<FaceId>,
}

impl Query<'_> {
    fn sign(&mut self, n: &Real) -> Option<Ordering> {
        match self.dec.sign(n) {
            Outcome::Proven(StrictSign::Positive) => Some(Greater),
            Outcome::Proven(StrictSign::Negative) => Some(Less),
            Outcome::ProvenCoincident(_) => Some(Equal),
            Outcome::Refused(_) | Outcome::NotExact(_) => None,
        }
    }

    fn zero(&mut self, n: &Real) -> bool {
        self.sign(n) == Some(Equal)
    }

    fn parallel(&mut self, a: &Vector3, b: &Vector3) -> bool {
        let across = a.cross(b);
        self.zero(&across.x) && self.zero(&across.y) && self.zero(&across.z)
    }

    fn id(&self, face: FaceId) -> AppFace {
        self.ids.faces[&face]
    }

    fn exact(&self, face: FaceId) -> bool {
        self.model.face(face).tolerance.is_none()
    }

    fn surface(&self, face: FaceId) -> &'_ Surface {
        self.model.geom().surface(self.model.face(face).surface)
    }

    fn plane(&self, face: FaceId) -> Option<(&'_ Plane, Vector3)> {
        let Surface::Plane(plane) = self.surface(face) else { return None };
        let normal = if self.model.face(face).sense == Sense::Same {
            plane.normal.clone()
        } else {
            plane.normal.scaled(&Real::from_i64(-1))
        };
        Some((plane, normal))
    }

    fn curve(&self, edge: EdgeId) -> &'_ Curve {
        self.model.geom().curve(self.model.edge(edge).curve)
    }

    fn edges(&self, face: FaceId) -> impl Iterator<Item = EdgeId> + '_ {
        self.model
            .face(face)
            .loops
            .iter()
            .flat_map(|&id| match self.model.loop_(id) {
                Loop::Coedges(coedges) => coedges.as_slice(),
                Loop::Vertex(_) => &[],
            })
            .map(|&coedge| self.model.coedge(coedge).edge)
    }

    fn neighbor(&self, face: FaceId, edge: EdgeId) -> Option<FaceId> {
        if self.model.edge(edge).tolerance.is_some() {
            return None;
        }
        let uses = self.model.coedges_of_edge(edge);
        let &[a, b] = uses.as_slice() else { return None };
        let (_, a) = self.model.owner_of_coedge(a)?;
        let (_, b) = self.model.owner_of_coedge(b)?;
        let other = if face == a {
            b
        } else if face == b {
            a
        } else {
            return None;
        };
        (self.ids.faces.contains_key(&other) && self.exact(other)).then_some(other)
    }

    fn endpoints(&self, edge: EdgeId) -> Option<[Point3; 2]> {
        let edge = self.model.edge(edge);
        let point = |vertex| self.model.geom().position(self.model.vertex(vertex).point);
        Some([point(edge.start?), point(edge.end?)])
    }

    fn round_axis(&self, face: FaceId) -> Option<(&'_ Point3, &'_ Vector3)> {
        if self.model.face(face).sense != Sense::Reversed {
            return None;
        }
        match self.surface(face) {
            Surface::Cylinder(c) => Some((&c.origin, &c.axis)),
            Surface::Cone(c) => Some((&c.apex, &c.axis)),
            _ => None,
        }
    }

    fn same_round(&mut self, a: FaceId, b: FaceId) -> bool {
        let model = self.model;
        let surface = |f| model.geom().surface(model.face(f).surface);
        if model.face(b).sense != Sense::Reversed {
            return false;
        }
        match (surface(a), surface(b)) {
            (Surface::Cylinder(a), Surface::Cylinder(b)) => {
                self.zero(&(&a.radius_sq - &b.radius_sq))
                    && self.parallel(&a.axis, &b.axis)
                    && self.parallel(&b.origin.minus(&a.origin), &a.axis)
            }
            (Surface::Cone(a), Surface::Cone(b)) => {
                self.zero(&(&a.slope - &b.slope))
                    && self.zero(&b.apex.minus(&a.apex).norm_squared())
                    && self.parallel(&a.axis, &b.axis)
                    && self.sign(&a.axis.dot(&b.axis)) == Some(Greater)
            }
            _ => false,
        }
    }

    fn connected_round(&mut self, seed: FaceId, seen: &mut BTreeSet<FaceId>) -> Vec<FaceId> {
        let mut faces = vec![seed];
        seen.insert(seed);
        let mut next = 0;
        while next < faces.len() {
            let face = faces[next];
            let neighbors: Vec<_> = self.edges(face).filter_map(|e| self.neighbor(face, e)).collect();
            for other in neighbors {
                if !seen.contains(&other) && self.same_round(seed, other) {
                    seen.insert(other);
                    faces.push(other);
                }
            }
            next += 1;
        }
        faces.sort_unstable();
        faces
    }

    fn rims(&mut self, faces: &[FaceId]) -> Option<[Rim; 2]> {
        let (axis_point, axis) = self.round_axis(faces[0])?;
        let (axis_point, axis) = (axis_point.clone(), axis.clone());
        let mut boundary = Vec::new();
        for &face in faces {
            for edge in self.edges(face) {
                let other = self.neighbor(face, edge)?;
                if faces.binary_search(&other).is_err() {
                    boundary.push((edge, other));
                }
            }
        }
        let mut rings: [Option<Rim>; 2] = [None, None];
        let model = self.model;
        for (edge, other) in boundary {
            let Curve::Circle(circle) = model.geom().curve(model.edge(edge).curve) else { return None };
            if !self.parallel(&circle.normal, &axis) || !self.parallel(&circle.center.minus(&axis_point), &axis) {
                return None;
            }
            let mut placed = false;
            for ring in &mut rings {
                if let Some(rim) = ring {
                    if !self.zero(&rim.circle.center.minus(&circle.center).norm_squared()) {
                        continue;
                    }
                    if !self.zero(&(&rim.circle.radius_sq - &circle.radius_sq)) {
                        return None;
                    }
                }
                let rim = ring.get_or_insert_with(|| Rim {
                    circle: circle.clone(),
                    edges: BTreeSet::new(),
                    neighbors: BTreeSet::new(),
                });
                rim.edges.insert(edge);
                rim.neighbors.insert(other);
                placed = true;
                break;
            }
            if !placed {
                return None;
            }
        }
        let [a, b] = rings;
        Some([a?, b?])
    }

    fn loop_matches(&self, coedges: &[CoedgeId], rim: &Rim) -> bool {
        coedges.len() == rim.edges.len() && coedges.iter().all(|&c| rim.edges.contains(&self.model.coedge(c).edge))
    }

    /// A planar circular boundary is an aperture if its loop winds against the outward normal.
    fn aperture(&mut self, face: FaceId, rim: &Rim, normal: &Vector3) -> Option<bool> {
        let model = self.model;
        let coedges = model.face(face).loops.iter().find_map(|&id| match model.loop_(id) {
            Loop::Coedges(coedges) if self.loop_matches(coedges, rim) => Some(coedges.as_slice()),
            _ => None,
        })?;
        let mut aperture = None;
        for &coedge in coedges {
            let coedge = model.coedge(coedge);
            let edge = model.edge(coedge.edge);
            let Curve::Circle(circle) = model.geom().curve(edge.curve) else { return None };
            let order = self.sign(&circle.normal.dot(normal))?;
            if order == Equal {
                return None;
            }
            let inner = (order == Less) == (coedge.sense == edge.sense);
            if aperture.is_some_and(|was| was != inner) {
                return None;
            }
            aperture = Some(inner);
        }
        aperture
    }

    fn compound_hole(
        &mut self,
        rounds: &[Round],
        by_wall: &BTreeMap<FaceId, usize>,
        pilot_index: usize,
        join: usize,
    ) -> Option<(usize, CompoundHole)> {
        let pilot = &rounds[pilot_index];
        let [inner, end] = [&pilot.rims[join], &pilot.rims[1 - join]];
        let end_face = only(&end.neighbors)?;
        let (_, end_normal) = self.plane(end_face)?;
        let through = self.aperture(end_face, end, &end_normal)?;

        let (head_index, at, shoulder) =
            if let Some(shoulder) = only(&inner.neighbors).filter(|&f| self.plane(f).is_some()) {
                let [a, b] = self.model.face(shoulder).loops.as_slice() else { return None };
                let (Loop::Coedges(a), Loop::Coedges(b)) = (self.model.loop_(*a), self.model.loop_(*b)) else {
                    return None;
                };
                let outer = if self.loop_matches(a, inner) {
                    b
                } else if self.loop_matches(b, inner) {
                    a
                } else {
                    return None;
                };
                let c = self.model.coedge(*outer.first()?);
                let head_index = *by_wall.get(&self.neighbor(shoulder, c.edge)?)?;
                if !matches!(self.surface(rounds[head_index].faces[0]), Surface::Cylinder(_)) {
                    return None;
                }
                let at = rounds[head_index]
                    .rims
                    .iter()
                    .position(|rim| only(&rim.neighbors) == Some(shoulder) && self.loop_matches(outer, rim))?;
                (head_index, at, Some(shoulder))
            } else {
                let head_index = *by_wall.get(inner.neighbors.first()?)?;
                if !matches!(self.surface(rounds[head_index].faces[0]), Surface::Cone(_)) {
                    return None;
                }
                let at = rounds[head_index].rims.iter().position(|rim| {
                    rim.edges == inner.edges && rim.neighbors.iter().all(|f| by_wall.get(f) == Some(&pilot_index))
                })?;
                (head_index, at, None)
            };
        if pilot_index == head_index {
            return None;
        }
        let head = &rounds[head_index];
        let [near, mouth] = [&head.rims[at], &head.rims[1 - at]];
        let entry = only(&mouth.neighbors)?;
        let (_, entry_normal) = self.plane(entry)?;
        let dir = end.circle.center.minus(&mouth.circle.center);
        let neck = inner.circle.center.minus(&mouth.circle.center);
        if !self.zero(&near.circle.center.minus(&inner.circle.center).norm_squared())
            || !self.parallel(&mouth.circle.normal, &inner.circle.normal)
            || !self.parallel(&mouth.circle.normal, &dir)
            || !self.parallel(&entry_normal, &dir)
            || !self.parallel(&end_normal, &dir)
            || !self.aperture(entry, mouth, &entry_normal)?
            || self.sign(&entry_normal.dot(&dir)) != Some(Less)
            || self.sign(&end_normal.dot(&dir)) != Some(if through { Greater } else { Less })
            || self.sign(&neck.dot(&dir)) != Some(Greater)
            || self.sign(&end.circle.center.minus(&inner.circle.center).dot(&dir)) != Some(Greater)
            || self.sign(&(&mouth.circle.radius_sq - &inner.circle.radius_sq)) != Some(Greater)
        {
            return None;
        }
        if let Some(shoulder) = shoulder {
            let (_, normal) = self.plane(shoulder)?;
            if !self.parallel(&normal, &dir)
                || self.sign(&normal.dot(&dir)) != Some(Less)
                || !self.aperture(shoulder, inner, &normal)?
                || self.aperture(shoulder, near, &normal)?
            {
                return None;
            }
        }
        let depth = length(&dir)?;
        let head_depth = length(&neck)?;
        let pilot_radius = root(&inner.circle.radius_sq)?;
        let head_radius = root(&mouth.circle.radius_sq)?;
        let kind = if let Some(shoulder) = shoulder {
            CompoundHead::Counterbore { shoulder: self.id(shoulder) }
        } else {
            CompoundHead::Countersink { angle_deg: 2.0 * (head_radius - pilot_radius).atan2(head_depth).to_degrees() }
        };
        Some((
            head_index,
            CompoundHole {
                pilot_faces: pilot.faces.iter().map(|&f| self.id(f)).collect(),
                head_faces: head.faces.iter().map(|&f| self.id(f)).collect(),
                entry: self.id(entry),
                end: self.id(end_face),
                mouth_center: Vec3::new(
                    mouth.circle.center.x.to_f64(),
                    mouth.circle.center.y.to_f64(),
                    mouth.circle.center.z.to_f64(),
                ),
                direction: Vec3::new(dir.x.to_f64() / depth, dir.y.to_f64() / depth, dir.z.to_f64() / depth),
                pilot_diameter: 2.0 * pilot_radius,
                head_diameter: 2.0 * head_radius,
                depth,
                head_depth,
                through,
                head: kind,
            },
        ))
    }

    fn one_boundary(&self, face: FaceId) -> Option<&'_ [CoedgeId]> {
        let &[id] = self.model.face(face).loops.as_slice() else { return None };
        match self.model.loop_(id) {
            Loop::Coedges(edges) => Some(edges),
            Loop::Vertex(_) => None,
        }
    }

    /// Which side of a plane contains a straight face, ignoring points on the plane.
    fn side(&mut self, points: &[[Point3; 2]], origin: &Point3, normal: &Vector3) -> Option<Ordering> {
        let mut side = Equal;
        for point in points.iter().flatten() {
            let at = self.sign(&point.minus(origin).dot(normal))?;
            if at != Equal {
                if side != Equal && at != side {
                    return None;
                }
                side = at;
            }
        }
        Some(side)
    }

    fn recess(&mut self, floor: FaceId) -> Option<Recess> {
        let (_, up) = self.plane(floor)?;
        let model = self.model;
        let boundary: Vec<_> = self.one_boundary(floor)?.iter().map(|&c| model.coedge(c).edge).collect();
        if boundary.len() < 3 || boundary.iter().any(|&e| !matches!(self.curve(e), Curve::Line(_))) {
            return None;
        }
        let points: Vec<_> = boundary.iter().map(|&edge| self.endpoints(edge)).collect::<Option<_>>()?;
        let floor_point = &points[0][0];
        let mut walls = BTreeSet::new();
        let mut openings = BTreeSet::new();
        let mut height = None;
        let mut open_sided = false;
        for (edge, [start, end]) in boundary.into_iter().zip(&points) {
            let wall = self.neighbor(floor, edge)?;
            let (_, normal) = self.plane(wall)?;
            if !self.zero(&normal.dot(&up)) {
                return None;
            }
            match self.side(&points, start, &normal)? {
                Less => {
                    // The wall may extend above the floor beyond this edge's endpoints.
                    // Its oriented coedge identifies the local material side.
                    let uses = model.coedges_of_edge(edge);
                    let &coedge =
                        uses.iter().find(|&&c| model.owner_of_coedge(c).is_some_and(|(_, face)| face == wall))?;
                    let interior = normal.cross(&end.minus(start)).dot(&up);
                    let expected = if model.coedge(coedge).sense == Sense::Same { Less } else { Greater };
                    if self.sign(&interior) != Some(expected) {
                        return None;
                    }
                    open_sided = true;
                }
                Greater => {
                    if !walls.insert(wall) {
                        return None;
                    }
                    let &[a, b, c, d] = self.one_boundary(wall)? else { return None };
                    let edges = [a, b, c, d].map(|c| model.coedge(c).edge);
                    let at = edges.iter().position(|&e| e == edge)?;
                    if edges.iter().any(|&e| !matches!(self.curve(e), Curve::Line(_))) {
                        return None;
                    }
                    let top = edges[(at + 2) % 4];
                    let opening = self.neighbor(wall, top)?;
                    let (_, top_normal) = self.plane(opening)?;
                    if !self.parallel(&up, &top_normal) || self.sign(&up.dot(&top_normal)) != Some(Greater) {
                        return None;
                    }
                    let [top_point, _] = self.endpoints(top)?;
                    let rise = top_point.minus(floor_point).dot(&up);
                    if self.sign(&rise) != Some(Greater) || height.as_ref().is_some_and(|h| !self.zero(&(&rise - h))) {
                        return None;
                    }
                    height = Some(rise);
                    openings.insert(opening);
                }
                Equal => return None,
            }
        }
        let height = height?;
        let depth_sq = height.square() / up.norm_squared();
        // The same notch can resemble a step viewed from the side. Require the
        // depth to be smaller than the floor's width across each boundary edge.
        for [start, end] in &points {
            let across = up.cross(&end.minus(start));
            let min_span_sq = &depth_sq * &across.norm_squared();
            if !points.iter().flatten().any(|point| {
                let span_sq = point.minus(start).dot(&across).square();
                self.sign(&(&span_sq - &min_span_sq)) == Some(Greater)
            }) {
                return None;
            }
        }
        let depth = root(&depth_sq)?;
        Some(Recess {
            floor: self.id(floor),
            walls: walls.into_iter().map(|f| self.id(f)).collect(),
            opens_onto: openings.into_iter().map(|f| self.id(f)).collect(),
            depth,
            open_sided,
        })
    }

    fn edge_treatment(&mut self, face: FaceId) -> Option<EdgeTreatment> {
        let model = self.model;
        let &[a, b, c, d] = self.one_boundary(face)? else { return None };
        let edges = [a, b, c, d].map(|c| model.coedge(c).edge);
        let surface = model.geom().surface(model.face(face).surface);
        let mut found = None;
        for i in 0..2 {
            let [a, b, c, d] = [edges[i], edges[i + 1], edges[(i + 2) % 4], edges[(i + 3) % 4]];
            let curve = |e| model.geom().curve(model.edge(e).curve);
            let (Curve::Line(rail), Curve::Line(other)) = (curve(a), curve(c)) else { continue };
            if !self.parallel(&rail.dir, &other.dir) {
                continue;
            }
            let [Some(p), Some(q), Some(r), Some(s)] = [a, b, c, d].map(|e| self.neighbor(face, e)) else { continue };
            let (Some((_, n)), Some((_, m)), Some((_, cap_b)), Some((_, cap_d))) =
                (self.plane(p), self.plane(r), self.plane(q), self.plane(s))
            else {
                continue;
            };
            if !self.zero(&n.dot(&m)) || !self.parallel(&rail.dir, &cap_b) || !self.parallel(&rail.dir, &cap_d) {
                continue;
            }
            let kind = match surface {
                Surface::Plane(_) if matches!((curve(b), curve(d)), (Curve::Line(_), Curve::Line(_))) => {
                    let (_, bevel) = self.plane(face)?;
                    let (nn, mm) = (bevel.dot(&n), bevel.dot(&m));
                    if self.sign(&nn) != Some(Greater)
                        || self.sign(&mm) != Some(Greater)
                        || !self.zero(&(&(&nn.square() * &m.norm_squared()) - &(&mm.square() * &n.norm_squared())))
                    {
                        continue;
                    }
                    let gap = other.origin.minus(&rail.origin).dot(&n);
                    EdgeTreatmentKind::Chamfer { setback: root(&(gap.square() / n.norm_squared()))? }
                }
                Surface::Cylinder(cyl) if self.parallel(&cyl.axis, &rail.dir) => {
                    let sense = model.face(face).sense;
                    let radial_n = rail.origin.minus(&cyl.origin).cross(&cyl.axis);
                    let radial_m = other.origin.minus(&cyl.origin).cross(&cyl.axis);
                    let outward = if sense == Sense::Same { Greater } else { Less };
                    let n_cross = n.cross(&cyl.axis);
                    let m_cross = m.cross(&cyl.axis);
                    if !self.parallel(&radial_n, &n_cross)
                        || !self.parallel(&radial_m, &m_cross)
                        || self.sign(&radial_n.dot(&n_cross)) != Some(outward)
                        || self.sign(&radial_m.dot(&m_cross)) != Some(outward)
                        || !self.quarter_arc(b)
                        || !self.quarter_arc(d)
                    {
                        continue;
                    }
                    EdgeTreatmentKind::Fillet { radius: root(&cyl.radius_sq)? }
                }
                _ => continue,
            };
            if found.is_some() {
                return None;
            }
            found = Some(EdgeTreatment { face: self.id(face), supports: [self.id(p), self.id(r)], kind });
        }
        found
    }

    fn quarter_arc(&mut self, edge: EdgeId) -> bool {
        let (Curve::Circle(arc), Some([a, b])) = (self.curve(edge), self.endpoints(edge)) else { return false };
        let from = a.minus(&arc.center);
        let to = b.minus(&arc.center);
        let sweep = from.cross(&to).dot(&arc.normal);
        let minor = if self.model.edge(edge).sense == Sense::Same { Greater } else { Less };
        self.zero(&from.dot(&to)) && self.sign(&sweep) == Some(minor)
    }
}

fn only(set: &BTreeSet<FaceId>) -> Option<FaceId> {
    (set.len() == 1).then(|| *set.first().expect("one face"))
}

fn root(value: &Real) -> Option<f64> {
    let result = value.to_f64().sqrt();
    (result.is_finite() && result > 0.0).then_some(result)
}

fn length(vector: &Vector3) -> Option<f64> {
    root(&vector.norm_squared())
}
