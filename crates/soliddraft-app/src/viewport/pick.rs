//! CPU ray queries against the display mesh. The id buffer decides what was
//! hit; these give the exact 3D point for the status bar and for later
//! features such as placing a sketch on a face.
//!
//! A face a GPU preview draws has no CPU triangles: its point comes from the kernel
//! ([`Kernel::face_ray_point`](soliddraft_kernel_api::Kernel::face_ray_point)).

use crate::document::{DisplayMesh, Document, PickEntry, Selection};
use crate::viewport::render::Picked;
use glam::Vec3;

/// Resolve an id-buffer pick for either primary or secondary selection.
pub fn selection(doc: &Document, picked: Picked) -> Option<Selection> {
    match doc.pick(picked.pick())? {
        PickEntry::Face(_, face) => Some(Selection::Face(face)),
        PickEntry::Edge(_, edge) => Some(Selection::Edge(edge)),
        PickEntry::Vertex(key, _) => Some(Selection::Vertex(key)),
    }
}

pub fn hit_point(doc: &Document, picked: Picked, origin: Vec3, dir: Vec3) -> Option<Vec3> {
    if let Picked::Face(pick) = picked {
        let shown = doc.provisional_mesh().into_iter().chain(doc.meshes.values().cloned());
        if let Some(gpu) = shown.filter_map(|mesh| mesh.gpu.clone()).find(|gpu| gpu.draws(pick)) {
            return gpu_point(doc, &gpu, pick, origin, dir);
        }
    }
    if let Some(mesh) = doc.provisional_mesh() {
        return match picked {
            Picked::Face(pick) => ray_face(&mesh, pick, origin, dir).map(|(_, point)| point),
            Picked::Edge(_) | Picked::Vertex(_) => None,
        };
    }
    match picked {
        Picked::Face(pick) => doc
            .meshes
            .values()
            .filter_map(|m| ray_face(m, pick, origin, dir))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p)| p),
        Picked::Edge(pick) => doc
            .meshes
            .values()
            .filter_map(|m| ray_edge(m, pick, origin, dir))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p)| p),
        Picked::Vertex(pick) => {
            doc.meshes.values().flat_map(|m| m.corners.iter()).find(|c| c.pick == pick).map(|c| c.position)
        }
    }
}

/// Where the ray meets a face a GPU preview draws, from the kernel's description of it.
fn gpu_point(
    doc: &Document,
    gpu: &crate::viewport::gpu_preview::GpuFaces,
    pick: u32,
    origin: Vec3,
    dir: Vec3,
) -> Option<Vec3> {
    let face = gpu.face(pick)?;
    let v = |p: Vec3| soliddraft_kernel_api::Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z));
    let point = doc.kernel().face_ray_point(gpu.body, face, v(origin), v(dir)).ok()??;
    Some(Vec3::new(point.x as f32, point.y as f32, point.z as f32))
}

fn ray_face(mesh: &DisplayMesh, pick: u32, origin: Vec3, dir: Vec3) -> Option<(f32, Vec3)> {
    let mut best: Option<(f32, Vec3)> = None;
    for tri in mesh.indices.as_chunks::<3>().0 {
        let i0 = tri[0] as usize;
        if mesh.face_picks[i0] != pick {
            continue;
        }
        let (a, b, c) = (mesh.positions[i0], mesh.positions[tri[1] as usize], mesh.positions[tri[2] as usize]);
        if let Some(t) = ray_triangle(origin, dir, a, b, c)
            && best.is_none_or(|(bt, _)| t < bt)
        {
            best = Some((t, origin + dir * t));
        }
    }
    best
}

/// Möller–Trumbore, both-sided.
fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-9 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

fn ray_edge(mesh: &DisplayMesh, pick: u32, origin: Vec3, dir: Vec3) -> Option<(f32, Vec3)> {
    let edge = mesh.edges.iter().find(|e| e.pick == pick)?;
    let mut best: Option<(f32, Vec3)> = None;
    for seg in edge.segments.as_chunks::<2>().0 {
        let (d, p) = closest_on_segment(origin, dir, seg[0], seg[1]);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, p));
        }
    }
    best
}

/// Distance between a ray and a segment, and the point on the segment.
fn closest_on_segment(o: Vec3, d: Vec3, a: Vec3, b: Vec3) -> (f32, Vec3) {
    let u = b - a;
    let w = a - o;
    let (aa, bb, cc) = (d.dot(d), d.dot(u), u.dot(u));
    let (dd, ee) = (d.dot(w), u.dot(w));
    let denom = aa * cc - bb * bb;
    let t = if denom.abs() < 1e-9 { ee / cc.max(1e-9) } else { (aa * ee - bb * dd) / denom };
    let t = t.clamp(0.0, 1.0);
    let s = ((bb * t + dd) / aa.max(1e-9)).max(0.0);
    let p_ray = o + d * s;
    let p_seg = a + u * t;
    ((p_ray - p_seg).length(), p_seg)
}

/// Position of a picked corner.
pub fn vertex_position(doc: &Document, pick: u32) -> Option<Vec3> {
    doc.meshes.values().flat_map(|m| m.corners.iter()).find(|c| c.pick == pick).map(|c| c.position)
}
