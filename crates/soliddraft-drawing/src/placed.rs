//! A drawing of several placed bodies: an assembly's parts.
//!
//! [`PlacedBodies`] implements [`DrawingGeometry`] for an assembly's placed parts, so an
//! assembly drawing is made, rebuilt and exported by the same code as a part drawing and
//! keeps its refusal rules. Each drawing query is asked of each part's geometry in the
//! part's own frame, and the answers are placed in the assembly's frame: a part's line
//! work is its own hidden-line view, and a part that refuses refuses the view, named.
//!
//! No single-part query sees two parts at once, so where one part hides another the
//! visibility is measured against the other parts' meshes: certified meshes for an
//! exported view, the screen's meshes for a preview. A view where that measurement
//! decided a curve carries the code [`PART_VISIBILITY`] and is labelled approximate, as
//! a view is whose cuts or visibility the kernel measured.
//!
//! Entities get ids of their own. The parts' history comes back with each tag prefixed
//! by its component, so a name in the drawing says which component an entity is on:
//! `c2/f3` is the operation tagged `f3` in component `c2`.

use std::collections::HashMap;
use std::sync::Arc;

use soliddraft_kernel_api::drawing::mesh_hlr::MeshOccluder;
use soliddraft_kernel_api::drawing::{
    Curve3, CurveSource, CutRegion, SectionSpec, SectionView, ViewCurve, ViewCurves, ViewFrame, ViewSpec, sheet_bounds,
};
use soliddraft_kernel_api::{
    Aabb, BodyId, BodyKind, EdgeId, EdgeInfo, EdgePolyline, EntityChanges, FaceAxis, FaceCarrier, FaceId, FaceInfo,
    HistoryRecord, Kernel, KernelError, KernelResult, Mesh, OpTag, Origin, TessellationParams, Topology, Vec3,
    VertexId, VertexInfo,
};

use crate::geometry::{DrawingGeometry, KernelBody};

/// The code a view carries when visibility between parts was measured on meshes.
pub const PART_VISIBILITY: &str = "assembly.part-visibility";

/// Bisection steps used to place each change between visible and hidden.
const REFINE_STEPS: u32 = 16;

/// A rigid placement: a unit rotation quaternion and a translation, applied in that
/// order. A point of the part at `p` is at `rotation * p + translation` in the assembly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rigid {
    /// The rotation as a unit quaternion `[x, y, z, w]`.
    pub rotation: [f64; 4],
    /// The translation in millimetres.
    pub translation: Vec3,
}

impl Rigid {
    /// No movement.
    pub const IDENTITY: Rigid = Rigid { rotation: [0.0, 0.0, 0.0, 1.0], translation: Vec3::ZERO };

    /// A direction turned by the rotation.
    pub fn rotate(&self, v: Vec3) -> Vec3 {
        let [x, y, z, w] = self.rotation;
        turn(Vec3::new(x, y, z), w, v)
    }

    /// A direction turned back into the part's frame.
    pub fn unrotate(&self, v: Vec3) -> Vec3 {
        let [x, y, z, w] = self.rotation;
        turn(Vec3::new(-x, -y, -z), w, v)
    }

    /// A point of the part in the assembly.
    pub fn apply(&self, p: Vec3) -> Vec3 {
        self.rotate(p) + self.translation
    }

    /// A point of the assembly in the part's frame.
    pub fn unapply(&self, p: Vec3) -> Vec3 {
        self.unrotate(p - self.translation)
    }

    /// A curve of the part in the assembly. Every kind keeps its kind: a spline's
    /// control points move, which moves the spline exactly.
    pub fn curve(&self, curve: &Curve3) -> Curve3 {
        match curve {
            Curve3::Line { a, b } => Curve3::Line { a: self.apply(*a), b: self.apply(*b) },
            Curve3::Arc { center, normal, x_axis, radius, start_deg, end_deg } => Curve3::Arc {
                center: self.apply(*center),
                normal: self.rotate(*normal),
                x_axis: self.rotate(*x_axis),
                radius: *radius,
                start_deg: *start_deg,
                end_deg: *end_deg,
            },
            Curve3::Circle { center, normal, radius } => {
                Curve3::Circle { center: self.apply(*center), normal: self.rotate(*normal), radius: *radius }
            }
            Curve3::Nurbs { degree, knots, points, weights } => Curve3::Nurbs {
                degree: *degree,
                knots: knots.clone(),
                points: points.iter().map(|p| self.apply(*p)).collect(),
                weights: weights.clone(),
            },
            Curve3::Polyline { points } => Curve3::Polyline { points: points.iter().map(|p| self.apply(*p)).collect() },
        }
    }

    /// A mesh of the part in the assembly.
    pub fn mesh(&self, mut mesh: Mesh) -> Mesh {
        let f = |p: [f32; 3]| Vec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
        let back = |v: Vec3| [v.x as f32, v.y as f32, v.z as f32];
        for p in &mut mesh.positions {
            *p = back(self.apply(f(*p)));
        }
        for n in &mut mesh.normals {
            *n = back(self.rotate(f(*n)));
        }
        mesh
    }
}

/// `v` turned by the unit quaternion with vector part `q` and scalar part `w`.
fn turn(q: Vec3, w: f64, v: Vec3) -> Vec3 {
    let t = q.cross(v) * 2.0;
    v + t * w + q.cross(t)
}

/// One part of an assembly as a drawing sees it.
#[derive(Clone)]
pub struct PlacedPart<'k> {
    /// The part's read-only drawing geometry, in its own frame.
    pub geometry: Arc<dyn DrawingGeometry + 'k>,
    /// Where the assembly places it.
    pub placement: Rigid,
    /// The component's occurrence path, unique in the assembly, such as `c2` or
    /// `c4/c1`. Names of the part's entities start with it.
    pub component: String,
    /// The name the interface shows, such as "Bracket<1>". Refusals name the part with it.
    pub label: String,
}

impl<'k> PlacedPart<'k> {
    /// A placed part backed by any read-only [`DrawingGeometry`].
    pub fn new(
        geometry: Arc<dyn DrawingGeometry + 'k>,
        placement: Rigid,
        component: impl Into<String>,
        label: impl Into<String>,
    ) -> PlacedPart<'k> {
        PlacedPart { geometry, placement, component: component.into(), label: label.into() }
    }

    /// A placed part backed by a body in a kernel session.
    pub fn from_kernel(
        kernel: &'k dyn Kernel,
        body: BodyId,
        placement: Rigid,
        component: impl Into<String>,
        label: impl Into<String>,
    ) -> PlacedPart<'k> {
        PlacedPart::new(Arc::new(KernelBody::new(kernel, body)), placement, component, label)
    }
}

/// An assembly's parts as a read-only [`DrawingGeometry`]. See the module documentation.
pub struct PlacedBodies<'k> {
    parts: Vec<PlacedPart<'k>>,
    faces: Vec<(usize, FaceId)>,
    edges: Vec<(usize, EdgeId)>,
    face_ids: Vec<HashMap<FaceId, FaceId>>,
    edge_ids: Vec<HashMap<EdgeId, EdgeId>>,
    /// Each part's box in the assembly's frame.
    boxes: Vec<Aabb>,
    topology: Topology,
    history: Vec<HistoryRecord>,
}

impl<'k> PlacedBodies<'k> {
    /// The parts as one drawing geometry, reading each part's topology and history once.
    /// A part whose body is not a solid is refused by name, as is an assembly with no part.
    pub fn new(parts: Vec<PlacedPart<'k>>) -> KernelResult<PlacedBodies<'k>> {
        if parts.is_empty() {
            return Err(KernelError::InvalidInput("the assembly places no part to draw".into()));
        }
        let mut out = PlacedBodies {
            parts: Vec::new(),
            faces: Vec::new(),
            edges: Vec::new(),
            face_ids: Vec::new(),
            edge_ids: Vec::new(),
            boxes: Vec::new(),
            topology: Topology {
                kind: BodyKind::Solid,
                faces: Vec::new(),
                edges: Vec::new(),
                vertex_count: 0,
                vertices: Vec::new(),
                bbox: Aabb { min: Vec3::ZERO, max: Vec3::ZERO },
                display_chord_height: None,
            },
            history: Vec::new(),
        };
        let mut all: Option<Aabb> = None;
        let mut next_vertex = 0u64;
        for (i, part) in parts.iter().enumerate() {
            let topology = part.geometry.topology().map_err(|e| named(&part.label, e))?;
            if topology.kind != BodyKind::Solid {
                return Err(KernelError::Unsupported(format!("{} is not a solid, so it cannot be drawn", part.label)));
            }
            let history = part.geometry.history().map_err(|e| named(&part.label, e))?;
            let mut faces = HashMap::new();
            for f in &topology.faces {
                out.faces.push((i, f.id));
                faces.insert(f.id, FaceId::from_raw(out.faces.len() as u64));
            }
            let mut edges = HashMap::new();
            for e in &topology.edges {
                out.edges.push((i, e.id));
                edges.insert(e.id, EdgeId::from_raw(out.edges.len() as u64));
            }
            let mut vertices = HashMap::new();
            for v in &topology.vertices {
                next_vertex += 1;
                vertices.insert(v.id, VertexId::from_raw(next_vertex));
            }
            let at = &part.placement;
            let prefix = |tag: &OpTag| OpTag(format!("{}/{}", part.component, tag.0));
            let origin = |o: &Origin| Origin { tag: prefix(&o.tag), role: o.role.clone() };
            for f in &topology.faces {
                out.topology.faces.push(FaceInfo {
                    id: faces[&f.id],
                    surface: f.surface,
                    accuracy: f.accuracy,
                    area: f.area,
                    centroid: at.apply(f.centroid),
                    bbox: place_box(at, &f.bbox),
                    edges: f.edges.iter().filter_map(|e| edges.get(e).copied()).collect(),
                    normal: f.normal.map(|n| at.rotate(n)),
                    origins: f.origins.iter().map(origin).collect(),
                });
            }
            for e in &topology.edges {
                out.topology.edges.push(EdgeInfo {
                    id: edges[&e.id],
                    curve: e.curve,
                    length: e.length,
                    midpoint: at.apply(e.midpoint),
                    bbox: place_box(at, &e.bbox),
                    faces: e.faces.iter().filter_map(|f| faces.get(f).copied()).collect(),
                    construction_origins: e.construction_origins.as_ref().map(|[a, b]| [origin(a), origin(b)]),
                    vertices: e.vertices.iter().filter_map(|v| vertices.get(v).copied()).collect(),
                });
            }
            for v in &topology.vertices {
                out.topology.vertices.push(VertexInfo {
                    id: vertices[&v.id],
                    position: at.apply(v.position),
                    edges: v.edges.iter().filter_map(|e| edges.get(e).copied()).collect(),
                });
            }
            for r in &history {
                let faces_made = r.faces.generated().filter_map(|f| faces.get(&f).copied()).collect();
                let edges_made = r.edges.generated().filter_map(|e| edges.get(&e).copied()).collect();
                out.history.push(HistoryRecord {
                    seq: r.seq,
                    tag: prefix(&r.tag),
                    op: r.op,
                    faces: EntityChanges { created: faces_made, ..Default::default() },
                    edges: EntityChanges { created: edges_made, ..Default::default() },
                    vertices: EntityChanges::default(),
                    face_roles: Vec::new(),
                });
            }
            let placed = place_box(at, &topology.bbox);
            all = Some(match all {
                None => placed,
                Some(b) => Aabb { min: min3(b.min, placed.min), max: max3(b.max, placed.max) },
            });
            out.boxes.push(placed);
            out.topology.display_chord_height = match (out.topology.display_chord_height, topology.display_chord_height)
            {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            out.face_ids.push(faces);
            out.edge_ids.push(edges);
        }
        out.topology.vertex_count = out.topology.vertices.len();
        out.topology.bbox = all.expect("at least one part");
        out.parts = parts;
        Ok(out)
    }

    /// The parts, in order.
    pub fn parts(&self) -> &[PlacedPart<'k>] {
        &self.parts
    }

    /// The part a face of this assembly geometry is on, and its id in that part.
    pub fn face_of(&self, face: FaceId) -> Option<(usize, FaceId)> {
        self.faces.get((face.raw() as usize).checked_sub(1)?).copied()
    }

    /// The part an edge of this assembly geometry is on, and its id in that part.
    pub fn edge_of(&self, edge: EdgeId) -> Option<(usize, EdgeId)> {
        self.edges.get((edge.raw() as usize).checked_sub(1)?).copied()
    }

    fn source(&self, i: usize, source: CurveSource) -> CurveSource {
        let face = |f: FaceId| self.face_ids[i].get(&f).copied();
        match source {
            CurveSource::Edge(e) => self.edge_ids[i].get(&e).map_or(CurveSource::Unknown, |e| CurveSource::Edge(*e)),
            CurveSource::Silhouette(f) => face(f).map_or(CurveSource::Unknown, CurveSource::Silhouette),
            CurveSource::Seam(f) => face(f).map_or(CurveSource::Unknown, CurveSource::Seam),
            CurveSource::Section(f) => face(f).map_or(CurveSource::Unknown, CurveSource::Section),
            CurveSource::Unknown => CurveSource::Unknown,
        }
    }

    fn place(&self, i: usize, mut c: ViewCurve) -> ViewCurve {
        c.curve = self.parts[i].placement.curve(&c.curve);
        c.source = self.source(i, c.source);
        c
    }

    /// The view seen from the part's own frame.
    fn local_spec(&self, i: usize, spec: &ViewSpec) -> ViewSpec {
        let at = &self.parts[i].placement;
        ViewSpec { direction: at.unrotate(spec.direction), up: at.unrotate(spec.up), ..*spec }
    }

    /// A part's mesh in the assembly's frame, with the accuracy it was asked for.
    fn occluder_mesh(&self, i: usize, spec: &ViewSpec) -> KernelResult<(Mesh, f64)> {
        let part = &self.parts[i];
        let size = self.boxes[i].diagonal().max(1.0);
        let chord = if spec.tolerance > 0.0 { spec.tolerance } else { 2e-5 * size };
        let angle = if spec.preview { TessellationParams::default().surface_angle_deg } else { 2.0 };
        let params = TessellationParams {
            chord_height: chord,
            curve_angle_deg: angle,
            surface_angle_deg: angle,
            ..Default::default()
        };
        let mesh = part.geometry.occlusion_mesh(&params, spec.preview).map_err(|e| named(&part.label, e))?;
        Ok((part.placement.mesh(mesh), chord))
    }

    /// The nearest and farthest depth of each part towards the eye.
    fn depths(&self, frame: &ViewFrame) -> Vec<(f64, f64)> {
        self.boxes
            .iter()
            .map(|b| {
                let ds = corners(b).map(|c| frame.depth(c));
                (ds.iter().copied().fold(f64::INFINITY, f64::min), ds.iter().copied().fold(f64::NEG_INFINITY, f64::max))
            })
            .collect()
    }

    /// Splits each part's visible curves where another part hides them. `limit` is the
    /// depth of a section's cutting plane: nothing nearer the eye than it hides anything.
    /// Parts marked in `gone` hide nothing. Returns the curves, whether a mesh decided
    /// any, and whether a mesh used was unproved.
    fn occlude(
        &self,
        frame: &ViewFrame,
        spec: &ViewSpec,
        curves: Vec<Vec<ViewCurve>>,
        limit: Option<f64>,
        gone: &[bool],
    ) -> KernelResult<(Vec<ViewCurve>, bool, bool)> {
        let n = self.parts.len();
        let boxes: Vec<Option<[f64; 4]>> =
            curves.iter().map(|c| (!c.is_empty()).then(|| sheet_bounds(c, frame))).collect();
        let span = boxes.iter().flatten().fold(0.0f64, |s, b| s.max(b[2] - b[0]).max(b[3] - b[1])).max(1e-6);
        let margin = 1e-6 * span.max(1.0);
        let depths = self.depths(frame);
        let overlaps = |a: &[f64; 4], b: &[f64; 4]| {
            a[0] <= b[2] + margin && b[0] <= a[2] + margin && a[1] <= b[3] + margin && b[1] <= a[3] + margin
        };
        let hides = |j: usize, i: usize| {
            if j == i || gone[j] {
                return false;
            }
            let (Some(a), Some(b)) = (&boxes[i], &boxes[j]) else { return false };
            let front = limit.map_or(depths[j].1, |l| depths[j].1.min(l));
            overlaps(a, b) && front > depths[i].0
        };
        let mut occluders: HashMap<usize, MeshOccluder> = HashMap::new();
        let mut unproved = false;
        for j in 0..n {
            if (0..n).any(|i| hides(j, i)) {
                let (mesh, chord) = self.occluder_mesh(j, spec)?;
                unproved |= mesh.unproved;
                occluders.insert(j, MeshOccluder::new(&mesh, frame, chord));
            }
        }
        let step = (span / 400.0).max(spec.tolerance);
        let mut measured = false;
        let mut out = Vec::new();
        for (i, part_curves) in curves.into_iter().enumerate() {
            let front: Vec<&MeshOccluder> = (0..n).filter(|&j| hides(j, i)).map(|j| &occluders[&j]).collect();
            for curve in part_curves {
                if !curve.visible || front.is_empty() {
                    out.push(curve);
                    continue;
                }
                measured = true;
                let hidden = |t: f64| {
                    let p = curve.curve.point_at(t);
                    front.iter().any(|o| o.hides(frame, p, limit))
                };
                let samples = ((curve.curve.length() / step).ceil() as usize).clamp(1, 20_000);
                let mut cuts = vec![0.0];
                let mut states = vec![hidden(0.0)];
                let mut t_prev = 0.0;
                for k in 1..=samples {
                    let t = k as f64 / samples as f64;
                    let h = hidden(t);
                    if h != *states.last().expect("a first state") {
                        let (mut lo, mut hi) = (t_prev, t);
                        for _ in 0..REFINE_STEPS {
                            let mid = 0.5 * (lo + hi);
                            if hidden(mid) == h {
                                hi = mid;
                            } else {
                                lo = mid;
                            }
                        }
                        cuts.push(0.5 * (lo + hi));
                        states.push(h);
                    }
                    t_prev = t;
                }
                cuts.push(1.0);
                if states.len() == 1 {
                    out.push(ViewCurve { visible: !states[0], approximate: true, ..curve });
                    continue;
                }
                for (k, h) in states.iter().enumerate() {
                    let (t0, t1) = (cuts[k], cuts[k + 1]);
                    if t1 <= t0 {
                        continue;
                    }
                    out.push(ViewCurve {
                        curve: curve.curve.between(t0, t1),
                        source: curve.source,
                        visible: !h,
                        smooth: curve.smooth,
                        approximate: true,
                    });
                }
            }
        }
        Ok((out, measured, unproved))
    }

    fn view(&self, spec: &ViewSpec) -> KernelResult<ViewCurves> {
        let frame =
            spec.frame().ok_or_else(|| KernelError::InvalidInput("view direction is zero or parallel to up".into()))?;
        let mut curves = Vec::with_capacity(self.parts.len());
        let (mut unproved, mut catallax, mut codes) = (false, true, Vec::new());
        for (i, part) in self.parts.iter().enumerate() {
            let view = part.geometry.view_curves(&self.local_spec(i, spec)).map_err(|e| named(&part.label, e))?;
            unproved |= view.unproved;
            catallax &= view.catallax;
            merge_codes(&mut codes, view.approximate);
            curves.push(view.curves.into_iter().map(|c| self.place(i, c)).collect());
        }
        let gone = vec![false; self.parts.len()];
        let (curves, measured, mesh_unproved) = self.occlude(&frame, spec, curves, None, &gone)?;
        if measured {
            merge_codes(&mut codes, vec![PART_VISIBILITY.to_owned()]);
        }
        Ok(ViewCurves {
            unproved: unproved || mesh_unproved,
            catallax,
            approximate: codes,
            ..ViewCurves::new(curves, &frame)
        })
    }

    fn section(&self, spec: &SectionSpec) -> KernelResult<SectionView> {
        let frame = spec
            .view
            .frame()
            .ok_or_else(|| KernelError::InvalidInput("view direction is zero or parallel to up".into()))?;
        let cut = frame.depth(spec.origin);
        let depths = self.depths(&frame);
        let mut curves = Vec::with_capacity(self.parts.len());
        let mut regions = Vec::new();
        let mut gone = vec![false; self.parts.len()];
        let (mut unproved, mut catallax, mut codes) = (false, true, Vec::new());
        for (i, part) in self.parts.iter().enumerate() {
            let local = self.local_spec(i, &spec.view);
            let (near, far) = depths[i];
            if near >= cut {
                // The whole part is on the eye's side of the plane: the cut removes it.
                gone[i] = true;
                curves.push(Vec::new());
                continue;
            }
            let view = if far <= cut {
                part.geometry.view_curves(&local)
            } else {
                let section = SectionSpec { view: local, origin: part.placement.unapply(spec.origin) };
                part.geometry.section_view(&section).map(|s| {
                    regions.extend(s.regions.into_iter().map(|r| CutRegion {
                        outer: r.outer.into_iter().map(|c| self.place(i, c)).collect(),
                        holes: r.holes.into_iter().map(|h| h.into_iter().map(|c| self.place(i, c)).collect()).collect(),
                    }));
                    s.curves
                })
            }
            .map_err(|e| named(&part.label, e))?;
            unproved |= view.unproved;
            catallax &= view.catallax;
            merge_codes(&mut codes, view.approximate);
            curves.push(view.curves.into_iter().map(|c| self.place(i, c)).collect());
        }
        if gone.iter().all(|g| *g) {
            return Err(KernelError::InvalidInput("the cutting plane leaves no part behind it".into()));
        }
        let (curves, measured, mesh_unproved) = self.occlude(&frame, &spec.view, curves, Some(cut), &gone)?;
        if measured {
            merge_codes(&mut codes, vec![PART_VISIBILITY.to_owned()]);
        }
        Ok(SectionView {
            curves: ViewCurves {
                unproved: unproved || mesh_unproved,
                catallax,
                approximate: codes,
                ..ViewCurves::new(curves, &frame)
            },
            regions,
        })
    }

    fn placed_polylines(&self, polylines: Vec<Vec<EdgePolyline>>) -> Vec<EdgePolyline> {
        let mut out = Vec::new();
        for (i, lines) in polylines.into_iter().enumerate() {
            let at = &self.parts[i].placement;
            for p in lines {
                if let Some(edge) = self.edge_ids[i].get(&p.edge) {
                    out.push(EdgePolyline { edge: *edge, points: p.points.into_iter().map(|q| at.apply(q)).collect() });
                }
            }
        }
        out
    }

    fn each<T>(&self, f: impl Fn(&PlacedPart<'k>) -> KernelResult<T>) -> KernelResult<Vec<T>> {
        self.parts.iter().map(|p| f(p).map_err(|e| named(&p.label, e))).collect()
    }

    fn meshes(&self, f: impl Fn(&PlacedPart<'k>) -> KernelResult<Mesh>) -> KernelResult<Mesh> {
        let mut out = Mesh::default();
        for (i, part) in self.parts.iter().enumerate() {
            let mesh = part.placement.mesh(f(part).map_err(|e| named(&part.label, e))?);
            let base = out.positions.len() as u32;
            out.positions.extend(mesh.positions);
            out.normals.extend(mesh.normals);
            out.indices.extend(mesh.indices.iter().map(|k| k + base));
            out.triangle_faces
                .extend(mesh.triangle_faces.iter().map(|f| self.face_ids[i].get(f).copied().unwrap_or(*f)));
            out.unproved |= mesh.unproved;
            for c in mesh.unavailable_controls {
                if !out.unavailable_controls.contains(&c) {
                    out.unavailable_controls.push(c);
                }
            }
        }
        Ok(out)
    }
}

/// The component a name in an assembly drawing starts with: `c2` for `c2/f3`.
pub fn component_of(tag: &str) -> Option<&str> {
    tag.rsplit_once('/').map(|(component, _)| component)
}

/// A part's refusal with the part's name in front, so a refused view says which part
/// refused it.
fn named(label: &str, e: KernelError) -> KernelError {
    match e {
        KernelError::QueryFailed { query, code, message } => {
            KernelError::QueryFailed { query, code, message: format!("{label}: {message}") }
        }
        KernelError::Unsupported(m) => KernelError::Unsupported(format!("{label}: {m}")),
        KernelError::InvalidInput(m) => KernelError::InvalidInput(format!("{label}: {m}")),
        other => other,
    }
}

fn merge_codes(codes: &mut Vec<String>, more: Vec<String>) {
    for c in more {
        if !codes.contains(&c) {
            codes.push(c);
        }
    }
}

fn min3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z))
}

fn max3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z))
}

fn corners(b: &Aabb) -> [Vec3; 8] {
    let (l, h) = (b.min, b.max);
    [
        Vec3::new(l.x, l.y, l.z),
        Vec3::new(h.x, l.y, l.z),
        Vec3::new(l.x, h.y, l.z),
        Vec3::new(h.x, h.y, l.z),
        Vec3::new(l.x, l.y, h.z),
        Vec3::new(h.x, l.y, h.z),
        Vec3::new(l.x, h.y, h.z),
        Vec3::new(h.x, h.y, h.z),
    ]
}

/// The box around a placed box.
fn place_box(at: &Rigid, b: &Aabb) -> Aabb {
    let placed = corners(b).map(|c| at.apply(c));
    let mut out = Aabb { min: placed[0], max: placed[0] };
    for p in placed {
        out.min = min3(out.min, p);
        out.max = max3(out.max, p);
    }
    out
}

impl DrawingGeometry for PlacedBodies<'_> {
    fn topology(&self) -> KernelResult<Topology> {
        Ok(self.topology.clone())
    }

    fn history(&self) -> KernelResult<Vec<HistoryRecord>> {
        Ok(self.history.clone())
    }

    fn face_carriers(&self) -> KernelResult<Vec<FaceCarrier>> {
        let mut out = Vec::new();
        for (i, carriers) in self.each(|p| p.geometry.face_carriers())?.into_iter().enumerate() {
            let part = &self.parts[i];
            let at = &part.placement;
            for c in carriers {
                let Some(id) = self.face_ids[i].get(&c.id) else { continue };
                out.push(FaceCarrier {
                    id: *id,
                    surface: c.surface,
                    normal: c.normal.map(|n| at.rotate(n)),
                    on_plane: c.on_plane.map(|p| at.apply(p)),
                    axis: c.axis.map(|a| FaceAxis {
                        point: at.apply(a.point),
                        direction: at.rotate(a.direction),
                        radius: a.radius,
                    }),
                    origins: c
                        .origins
                        .iter()
                        .map(|o| Origin { tag: OpTag(format!("{}/{}", part.component, o.tag.0)), role: o.role.clone() })
                        .collect(),
                });
            }
        }
        Ok(out)
    }

    fn edge_drawing(&self, edge: EdgeId) -> KernelResult<ViewCurve> {
        let (i, real) = self.edge_of(edge).ok_or(KernelError::UnknownEdge { body: BodyId::from_raw(0), edge })?;
        let part = &self.parts[i];
        let drawn = part.geometry.edge_drawing(real).map_err(|e| named(&part.label, e))?;
        Ok(self.place(i, drawn))
    }

    fn rendering_edges(&self, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        Ok(self.placed_polylines(self.each(|p| p.geometry.rendering_edges(params))?))
    }

    fn view_curves(&self, spec: &ViewSpec) -> KernelResult<ViewCurves> {
        self.view(spec)
    }

    fn section_view(&self, spec: &SectionSpec) -> KernelResult<SectionView> {
        self.section(spec)
    }

    fn occlusion_mesh(&self, params: &TessellationParams, preview: bool) -> KernelResult<Mesh> {
        self.meshes(|p| p.geometry.occlusion_mesh(params, preview))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quarter_turn_about_z_takes_x_to_y_and_back() {
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let at = Rigid { rotation: [0.0, 0.0, h, h], translation: Vec3::new(10.0, 0.0, 0.0) };
        let p = at.apply(Vec3::X);
        assert!(p.distance(Vec3::new(10.0, 1.0, 0.0)) < 1e-12, "{p:?}");
        let back = at.unapply(p);
        assert!(back.distance(Vec3::X) < 1e-12, "{back:?}");
        assert!(at.unrotate(at.rotate(Vec3::Z)).distance(Vec3::Z) < 1e-12);
    }

    #[test]
    fn a_placed_arc_keeps_its_kind_and_radius() {
        let at = Rigid { rotation: [0.0, 0.0, 0.0, 1.0], translation: Vec3::new(0.0, 0.0, 5.0) };
        let arc = Curve3::Arc {
            center: Vec3::ZERO,
            normal: Vec3::Z,
            x_axis: Vec3::X,
            radius: 2.0,
            start_deg: 0.0,
            end_deg: 90.0,
        };
        let placed = at.curve(&arc);
        assert!(matches!(placed, Curve3::Arc { radius, .. } if radius == 2.0));
        assert!(placed.end().distance(Vec3::new(0.0, 2.0, 5.0)) < 1e-12);
    }

    #[test]
    fn names_say_their_component() {
        assert_eq!(component_of("c4/c1/f3"), Some("c4/c1"));
        assert_eq!(component_of("f3"), None);
    }
}
