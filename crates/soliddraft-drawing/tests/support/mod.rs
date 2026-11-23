//! A kernel of axis-aligned boxes for testing drawings without the C++ kernel. It has
//! topology, history, exact edge curves and tessellation, so views come from the mesh
//! method; every operation is unsupported.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Mutex;

use soliddraft_drawing::{ContentHash, ModelDimension, ModelMeasure, Part};
use soliddraft_kernel_api::drawing::{
    Curve3, CurveSource, CutRegion, SectionSpec, SectionView, ViewCurve, ViewCurves, ViewSpec, mesh_hlr,
};
use soliddraft_kernel_api::{
    Aabb, BackendInfo, BodyId, BodyKind, BooleanOp, CurveKind, EdgeId, EdgeInfo, EdgePolyline, EntityChanges, Extrude,
    FaceId, FaceInfo, HistoryRecord, Kernel, KernelError, KernelResult, MassProperties, Mesh, OpKind, OpResult, OpTag,
    Profile, Revolve, SurfaceKind, TessellationParams, Topology, Validation, Vec3, VertexId, VertexInfo,
};

#[derive(Clone)]
struct BoxBody {
    tag: String,
    min: Vec3,
    max: Vec3,
    /// Raw id of the first face; edges follow the six faces.
    first_id: u64,
}

/// Boxes made by [`BoxKernel::add`], each the result of one tagged "extrude".
#[derive(Default)]
pub struct BoxKernel {
    bodies: Mutex<HashMap<u64, BoxBody>>,
    next: Mutex<u64>,
    pub detail_strokes: bool,
    pub unsupported_view: bool,
    pub refuse_direction: Option<Vec3>,
    /// Cuts sections square to an axis itself; without it sections take the trait's default.
    pub sections: bool,
}

/// The faces of a box: outward normal, and in-plane axes u and v with u x v = normal.
const FACES: [(Vec3, Vec3, Vec3); 6] = [
    (Vec3::new(-1.0, 0.0, 0.0), Vec3::Z, Vec3::Y),
    (Vec3::X, Vec3::Y, Vec3::Z),
    (Vec3::new(0.0, -1.0, 0.0), Vec3::X, Vec3::Z),
    (Vec3::Y, Vec3::Z, Vec3::X),
    (Vec3::new(0.0, 0.0, -1.0), Vec3::Y, Vec3::X),
    (Vec3::Z, Vec3::X, Vec3::Y),
];

impl BoxKernel {
    pub fn new() -> BoxKernel {
        BoxKernel { next: Mutex::new(1), ..Self::default() }
    }

    /// A box from `min` to `max` made by the operation tagged `tag`.
    pub fn add(&self, tag: &str, min: Vec3, max: Vec3) -> BodyId {
        let mut next = self.next.lock().unwrap();
        let id = *next;
        *next += 100;
        self.bodies.lock().unwrap().insert(id, BoxBody { tag: tag.to_owned(), min, max, first_id: id });
        BodyId::from_raw(id)
    }

    fn with<T>(&self, body: BodyId, f: impl FnOnce(&BoxBody) -> T) -> KernelResult<T> {
        self.bodies.lock().unwrap().get(&body.raw()).map(f).ok_or(KernelError::UnknownBody(body))
    }
}

fn corner(b: &BoxBody, i: usize) -> Vec3 {
    Vec3::new(
        if i & 1 == 0 { b.min.x } else { b.max.x },
        if i & 2 == 0 { b.min.y } else { b.max.y },
        if i & 4 == 0 { b.min.z } else { b.max.z },
    )
}

/// The twelve edges as corner index pairs: four along X, four along Y, four along Z.
fn edge_corners() -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for bit in [1usize, 2, 4] {
        for i in 0..8 {
            if i & bit == 0 {
                out.push((i, i | bit));
            }
        }
    }
    out
}

fn face_of(b: &BoxBody, f: usize) -> (Vec3, f64) {
    let (n, ..) = FACES[f];
    let c = (b.min + b.max) * 0.5;
    let half = (b.max - b.min) * 0.5;
    let offset = Vec3::new(n.x * half.x, n.y * half.y, n.z * half.z);
    (c + offset, offset.length())
}

/// Faces an edge touches: the two faces whose outward side its corners share.
fn edge_faces(b: &BoxBody, a: usize, c: usize) -> Vec<FaceId> {
    let (pa, pc) = (corner(b, a), corner(b, c));
    (0..6)
        .filter(|f| {
            let (centre, _) = face_of(b, *f);
            let n = FACES[*f].0;
            ((pa - centre).dot(n)).abs() < 1e-9 && ((pc - centre).dot(n)).abs() < 1e-9
        })
        .map(|f| FaceId::from_raw(b.first_id + f as u64))
        .collect()
}

impl Kernel for BoxKernel {
    fn backend(&self) -> BackendInfo {
        BackendInfo { name: "boxes".into(), version: "test".into() }
    }

    fn create_body_from_profile(&self, _: &OpTag, _: &Profile) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("profiles".into()))
    }

    fn extrude(&self, _: &OpTag, _: BodyId, _: &Extrude) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("extrude".into()))
    }

    fn revolve(&self, _: &OpTag, _: BodyId, _: &Revolve) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("revolve".into()))
    }

    fn boolean(&self, _: &OpTag, _: BooleanOp, _: BodyId, _: BodyId) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("boolean".into()))
    }

    fn fillet(&self, _: &OpTag, _: BodyId, _: &[EdgeId], _: f64) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("fillet".into()))
    }

    fn chamfer(&self, _: &OpTag, _: BodyId, _: &[EdgeId], _: f64) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("chamfer".into()))
    }

    fn shell(&self, _: &OpTag, _: BodyId, _: &[FaceId], _: f64) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("shell".into()))
    }

    fn view_curves(&self, body: BodyId, spec: &ViewSpec) -> KernelResult<ViewCurves> {
        if self.refuse_direction.is_some_and(|direction| direction.distance(spec.direction) < 1e-9) {
            return Err(KernelError::InvalidInput("test direction refused".into()));
        }
        if self.unsupported_view {
            return Err(KernelError::Unsupported("drawing view".into()));
        }
        let view = mesh_hlr::view_curves(self, body, spec)?;
        let (max, face) = self.with(body, |b| (b.max, FaceId::from_raw(b.first_id)))?;
        let near = Vec3::new(max.x - 1.0, 0.0, max.z - 1.0);
        let mut curves = view.curves;
        if self.detail_strokes && spec.smooth_edges {
            let mut tangent = ViewCurve::new(Curve3::Line { a: near, b: near - Vec3::X }, CurveSource::Unknown, true);
            tangent.smooth = true;
            curves.push(tangent);
        }
        if self.detail_strokes && spec.seams {
            curves.push(ViewCurve::new(Curve3::Line { a: near, b: near + Vec3::Z }, CurveSource::Seam(face), true));
        }
        Ok(ViewCurves {
            catallax: !spec.preview,
            unproved: view.unproved || spec.preview,
            ..ViewCurves::new(curves, &spec.frame().unwrap())
        })
    }

    /// With `sections` on, a box cut square to an axis is a shorter box: its view, with the face the cut
    /// leaves drawn as section curves and hatched. Other cuts are unsupported.
    fn section_view(&self, body: BodyId, spec: &SectionSpec) -> KernelResult<SectionView> {
        if !self.sections {
            return soliddraft_kernel_api::drawing::section::by_half_space(self, body, spec);
        }
        let frame = spec.view.frame().ok_or_else(|| KernelError::InvalidInput("view direction".into()))?;
        let n = frame.direction;
        let axis = [Vec3::X, Vec3::Y, Vec3::Z]
            .into_iter()
            .position(|a| n.dot(a).abs() > 1.0 - 1e-12)
            .ok_or_else(|| KernelError::Unsupported("a box is cut square to an axis only".into()))?;
        let get = |v: Vec3| [v.x, v.y, v.z][axis];
        let set = |v: &mut Vec3, x: f64| match axis {
            0 => v.x = x,
            1 => v.y = x,
            _ => v.z = x,
        };
        let b = self.with(body, BoxBody::clone)?;
        let cut = get(spec.origin);
        // The eye is towards n: the material on its side of the plane goes.
        let towards = get(n) > 0.0;
        let mut kept = b.clone();
        if towards && get(b.max) > cut {
            set(&mut kept.max, cut);
        } else if !towards && get(b.min) < cut {
            set(&mut kept.min, cut);
        }
        if get(kept.max) <= get(kept.min) {
            return Err(KernelError::InvalidInput("the cutting plane is behind the whole body".into()));
        }
        let key = u64::MAX / 2 + body.raw();
        self.bodies.lock().unwrap().insert(key, kept.clone());
        let view = self.view_curves(BodyId::from_raw(key), &spec.view);
        self.bodies.lock().unwrap().remove(&key);
        let mut view = view?;
        let through = get(kept.max) != get(b.max) || get(kept.min) != get(b.min);
        if !through {
            return Ok(SectionView { curves: view, regions: Vec::new() });
        }
        let cap = FaceId::from_raw(b.first_id + 2 * axis as u64 + u64::from(towards));
        let on = |p: Vec3| (get(p) - cut).abs() < 1e-9;
        for c in &mut view.curves {
            if on(c.curve.start()) && on(c.curve.end()) {
                c.source = CurveSource::Section(cap);
            }
        }
        // The cap's corners in loop order.
        let (u, w) = ((axis + 1) % 3, (axis + 2) % 3);
        let corner = |a: bool, c: bool| {
            let mut p = [0.0; 3];
            p[axis] = cut;
            p[u] = if a { [kept.max.x, kept.max.y, kept.max.z][u] } else { [kept.min.x, kept.min.y, kept.min.z][u] };
            p[w] = if c { [kept.max.x, kept.max.y, kept.max.z][w] } else { [kept.min.x, kept.min.y, kept.min.z][w] };
            Vec3::new(p[0], p[1], p[2])
        };
        let ring = [corner(false, false), corner(true, false), corner(true, true), corner(false, true)];
        let outer = (0..4)
            .map(|i| {
                let line = Curve3::Line { a: ring[i], b: ring[(i + 1) % 4] };
                ViewCurve::new(line, CurveSource::Section(cap), true)
            })
            .collect();
        Ok(SectionView { curves: view, regions: vec![CutRegion { outer, holes: Vec::new() }] })
    }

    fn tessellate(&self, body: BodyId, _: &TessellationParams) -> KernelResult<Mesh> {
        self.with(body, |b| {
            let mut m = Mesh::default();
            let size = b.max - b.min;
            for (f, (n, u, v)) in FACES.iter().enumerate() {
                let (centre, _) = face_of(b, f);
                let (lu, lv) = (size.dot(*u).abs(), size.dot(*v).abs());
                let origin = centre - *u * (lu / 2.0) - *v * (lv / 2.0);
                let base = m.positions.len() as u32;
                for p in [origin, origin + *u * lu, origin + *u * lu + *v * lv, origin + *v * lv] {
                    m.positions.push([p.x as f32, p.y as f32, p.z as f32]);
                    m.normals.push([n.x as f32, n.y as f32, n.z as f32]);
                }
                m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                let face = FaceId::from_raw(b.first_id + f as u64);
                m.triangle_faces.extend([face, face]);
            }
            m
        })
    }

    fn tessellate_edges(&self, body: BodyId, _: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        let topo = self.topology(body)?;
        topo.edges
            .iter()
            .map(|e| {
                Ok(EdgePolyline {
                    edge: e.id,
                    points: vec![self.edge_curve(body, e.id)?.start(), self.edge_curve(body, e.id)?.end()],
                })
            })
            .collect()
    }

    fn mass_properties(&self, body: BodyId) -> KernelResult<MassProperties> {
        self.with(body, |b| {
            let s = b.max - b.min;
            MassProperties {
                volume: s.x * s.y * s.z,
                area: 2.0 * (s.x * s.y + s.y * s.z + s.z * s.x),
                centroid: (b.min + b.max) * 0.5,
                inertia: Some([[0.0; 3]; 3]),
            }
        })
    }

    fn validate(&self, body: BodyId) -> KernelResult<Validation> {
        self.with(body, |_| Validation { valid: true, kind: BodyKind::Solid, manifold_solid: true, issues: vec![] })
    }

    fn topology(&self, body: BodyId) -> KernelResult<Topology> {
        self.with(body, |b| {
            let edges: Vec<EdgeInfo> = edge_corners()
                .into_iter()
                .enumerate()
                .map(|(i, (a, c))| {
                    let (pa, pc) = (corner(b, a), corner(b, c));
                    EdgeInfo {
                        id: EdgeId::from_raw(b.first_id + 6 + i as u64),
                        length: pa.distance(pc),
                        midpoint: (pa + pc) * 0.5,
                        bbox: Aabb { min: pa, max: pc },
                        faces: edge_faces(b, a, c),
                        construction_origins: None,
                        curve: CurveKind::Line,
                        vertices: vec![
                            VertexId::from_raw(b.first_id + 20 + a as u64),
                            VertexId::from_raw(b.first_id + 20 + c as u64),
                        ],
                    }
                })
                .collect();
            let faces = (0..6)
                .map(|f| {
                    let (centre, _) = face_of(b, f);
                    let id = FaceId::from_raw(b.first_id + f as u64);
                    let s = b.max - b.min;
                    let (_, u, v) = FACES[f];
                    FaceInfo {
                        id,
                        surface: SurfaceKind::Plane,
                        accuracy: soliddraft_kernel_api::FaceAccuracy::Unknown,
                        area: s.dot(u).abs() * s.dot(v).abs(),
                        centroid: centre,
                        bbox: Aabb { min: b.min, max: b.max },
                        edges: edges.iter().filter(|e| e.faces.contains(&id)).map(|e| e.id).collect(),
                        normal: Some(FACES[f].0),
                        origins: Vec::new(),
                    }
                })
                .collect();
            let vertices = (0..8)
                .map(|i| VertexInfo {
                    id: VertexId::from_raw(b.first_id + 20 + i as u64),
                    position: corner(b, i),
                    edges: edges
                        .iter()
                        .filter(|e| e.vertices.contains(&VertexId::from_raw(b.first_id + 20 + i as u64)))
                        .map(|e| e.id)
                        .collect(),
                })
                .collect();
            Topology {
                display_chord_height: None,
                kind: BodyKind::Solid,
                faces,
                edges,
                vertices,
                vertex_count: 8,
                bbox: Aabb { min: b.min, max: b.max },
            }
        })
    }

    fn history(&self, body: BodyId) -> KernelResult<Vec<HistoryRecord>> {
        let topo = self.topology(body)?;
        self.with(body, |b| {
            vec![HistoryRecord {
                seq: 1,
                tag: OpTag::new(&b.tag),
                op: OpKind::Extrude,
                faces: EntityChanges { created: topo.faces.iter().map(|f| f.id).collect(), ..Default::default() },
                edges: EntityChanges { created: topo.edges.iter().map(|e| e.id).collect(), ..Default::default() },
                vertices: EntityChanges { created: topo.vertices.iter().map(|v| v.id).collect(), ..Default::default() },
                face_roles: Vec::new(),
            }]
        })
    }

    fn release(&self, body: BodyId) -> KernelResult<()> {
        self.bodies.lock().unwrap().remove(&body.raw()).map(|_| ()).ok_or(KernelError::UnknownBody(body))
    }

    fn edge_curve(&self, body: BodyId, edge: EdgeId) -> KernelResult<Curve3> {
        self.with(body, |b| {
            let i = edge.raw().checked_sub(b.first_id + 6).filter(|i| *i < 12)?;
            let (a, c) = edge_corners()[i as usize];
            Some(Curve3::Line { a: corner(b, a), b: corner(b, c) })
        })?
        .ok_or(KernelError::UnknownEdge { body, edge })
    }
}

/// The part of a box from the origin to `size`, made by `tag`, with its feature
/// dimensions: the sketch's width and height on XY and the depth along Z.
pub fn box_part<'k>(k: &'k BoxKernel, tag: &str, size: Vec3) -> Part<'k> {
    let body = k.add(tag, Vec3::ZERO, size);
    let distance = |name: &str, axis: Vec3, to: f64, sketch: bool| ModelDimension {
        feature: tag.to_owned(),
        name: name.to_owned(),
        measure: ModelMeasure::Distance {
            axis,
            from: Vec3::ZERO,
            to: axis * to,
            sketch_normal: sketch.then_some(Vec3::Z),
        },
        count: 1,
    };
    Part {
        kernel: k,
        body,
        digest: ContentHash::of_bytes(format!("{tag} {} {} {}", size.x, size.y, size.z).as_bytes()),
        dimensions: vec![
            distance("width", Vec3::X, size.x, true),
            distance("height", Vec3::Y, size.y, true),
            distance("depth", Vec3::Z, size.z, false),
            // The same width again: Model items brings each dimension in once.
            distance("width again", Vec3::X, size.x, true),
        ],
        namer: None,
        preview: false,
        source: Default::default(),
        requirements: Default::default(),
    }
}
