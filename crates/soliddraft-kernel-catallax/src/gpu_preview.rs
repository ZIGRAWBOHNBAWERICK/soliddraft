//! The GPU preview: float descriptions of retained bodies for the renderer's GPU
//! mesher, that mesher on the renderer's device, and the point a pick on a GPU preview
//! asks for. Nothing here is proven, exported or measured.

use std::collections::{HashMap, VecDeque};
use std::f64::consts::TAU;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use catallax_mesh::float::{
    DisplayDescription, DisplayEdge, DisplayFace, DisplayOptions, DisplaySpline, DisplaySurface,
};
use soliddraft_kernel_api::{
    BodyId, EdgeId, EdgePolyline, FaceId, GpuPreview, KernelError, KernelResult, TessellationParams, Vec3,
};

/// A description keyed by interface ids, as a [`GpuPreview`] carries it.
pub(crate) type Description = DisplayDescription<FaceId, EdgeId>;

/// The description controls for a display request: the edge chord target follows the
/// requested chord height, and the curved-face grids follow the chord and angle.
pub(crate) fn options(params: &TessellationParams) -> DisplayOptions {
    let mut options = DisplayOptions::default();
    if params.chord_height.is_finite() && params.chord_height > 0.0 {
        options.chord_tolerance = params.chord_height;
    }
    options
}

/// Re-keys a Catallax description by interface ids. An entity without an id refuses.
pub(crate) fn rekey<F, E>(
    description: DisplayDescription<F, E>,
    face: impl Fn(&F) -> Option<FaceId>,
    edge: impl Fn(&E) -> Option<EdgeId>,
) -> KernelResult<Description> {
    let unnamed = || KernelError::Unsupported("a GPU preview face or edge has no interface id".into());
    let edges = description
        .edges
        .into_iter()
        .map(|e| Ok(DisplayEdge { edge: edge(&e.edge).ok_or_else(unnamed)?, points: e.points }))
        .collect::<KernelResult<Vec<_>>>()?;
    let faces = description
        .faces
        .into_iter()
        .map(|f| {
            Ok(DisplayFace {
                face: face(&f.face).ok_or_else(unnamed)?,
                reversed: f.reversed,
                surface: f.surface,
                bounds: f.bounds,
                periods: f.periods,
                loops: f.loops,
            })
        })
        .collect::<KernelResult<Vec<_>>>()?;
    Ok(DisplayDescription { version: description.version, origin: description.origin, edges, faces })
}

/// The interface form of `description`: its faces, its edges in world coordinates, and
/// the description itself for the GPU mesher.
pub(crate) fn preview(description: Arc<Description>) -> GpuPreview {
    let o = description.origin;
    GpuPreview {
        origin: o,
        faces: description.faces.iter().map(|face| face.face).collect(),
        edges: description
            .edges
            .iter()
            .map(|edge| EdgePolyline {
                edge: edge.edge,
                points: edge.points.iter().map(|p| Vec3::new(p[0] + o[0], p[1] + o[1], p[2] + o[2])).collect(),
            })
            .collect(),
        source: description,
    }
}

/// The last few descriptions read, for the point queries of picks on them.
#[derive(Default)]
pub(crate) struct Recent(Mutex<VecDeque<(BodyId, Arc<Description>)>>);

impl Recent {
    const KEPT: usize = 8;

    pub(crate) fn keep(&self, body: BodyId, description: Arc<Description>) {
        let mut kept = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        kept.retain(|(b, _)| *b != body);
        if kept.len() == Self::KEPT {
            kept.pop_front();
        }
        kept.push_back((body, description));
    }

    pub(crate) fn get(&self, body: BodyId) -> Option<Arc<Description>> {
        let kept = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        kept.iter().rev().find(|(b, _)| *b == body).map(|(_, d)| d.clone())
    }
}

/// Segments per full turn of a circle of `radius` within the chord height and angle.
fn per_turn(radius: f64, params: &TessellationParams) -> u32 {
    let angle = if params.surface_angle_deg > 0.0 { params.surface_angle_deg } else { 15.0 };
    let by_angle = (360.0 / angle).ceil();
    let chord = options(params).chord_tolerance;
    let by_chord =
        if radius.is_finite() && radius > chord { (TAU / (2.0 * (1.0 - chord / radius).acos())).ceil() } else { 8.0 };
    by_angle.max(by_chord).clamp(8.0, 512.0) as u32
}

/// Grid divisions for one face: none inside a plane, the turns of an analytic face at
/// the requested chord and angle, and a density from the control net of a spline.
fn detail(face: &DisplayFace<FaceId>, params: &TessellationParams) -> [u32; 2] {
    let turns = |axis: usize| (face.bounds[axis][1] - face.bounds[axis][0]).abs();
    let around = |axis: usize, radius: f64| ((turns(axis) * f64::from(per_turn(radius, params))).ceil() as u32).max(1);
    match &face.surface {
        DisplaySurface::Plane(_) => [1, 1],
        DisplaySurface::Cylinder { radius, .. } => [around(0, *radius), 1],
        DisplaySurface::Cone { slope, .. } => {
            let reach = face.bounds[1][0].abs().max(face.bounds[1][1].abs()) * slope.abs();
            [around(0, reach), 1]
        }
        DisplaySurface::Sphere { radius, .. } => [around(0, *radius), around(1, *radius)],
        DisplaySurface::Torus { major, minor, .. } => [around(0, major + minor), around(1, *minor)],
        DisplaySurface::EllipticCylinder { radii, .. } => [around(0, radii[0].max(radii[1])), 1],
        DisplaySurface::BSpline(spline) => {
            std::array::from_fn(|axis| ((spline.size[axis] - spline.degree[axis]).max(1) as u32 * 8).clamp(8, 128))
        }
        DisplaySurface::Grid(grid) => grid.size.map(|n| (n.saturating_sub(1) as u32).clamp(1, 128)),
    }
}

/// Vertex and index buffers on the renderer's device, as the Catallax mesher writes them.
pub use catallax_preview::mesher::{MeshBuffers, PreviewVertex};

/// One face of a GPU mesh: its index range, or why the CPU preview must draw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeshedFace {
    /// The interface face.
    pub face: FaceId,
    /// The face's range in [`MeshBuffers::index`], or the stable code of its refusal.
    pub draw: Result<Range<u32>, &'static str>,
    /// The GPU interpolated sampled points rather than evaluating the carrier.
    pub sampled: bool,
}

/// A GPU preview meshed on the renderer's device.
pub struct Meshed {
    /// Positions in the buffers are relative to this world origin.
    pub origin: [f64; 3],
    /// `None` when every face refused.
    pub buffers: Option<MeshBuffers>,
    /// One entry per face of the preview, in its order.
    pub faces: Vec<MeshedFace>,
}

/// The Catallax GPU preview mesher, compiled once on the renderer's device.
pub struct Mesher(catallax_preview::mesher::Mesher);

impl Mesher {
    /// Compiles the mesher on `device`. `queue` must belong to `device`.
    pub async fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Result<Self, String> {
        catallax_preview::mesher::Mesher::new(device, queue).await.map(Self).map_err(|refusal| refusal.to_string())
    }

    /// Meshes `preview` at the grid density `params` asks for. The buffers belong to the
    /// device the mesher was made on; nothing is read back.
    pub async fn mesh(&self, preview: &GpuPreview, params: &TessellationParams) -> Result<Meshed, String> {
        let description = preview
            .source
            .downcast_ref::<Description>()
            .ok_or("preview.foreign-description: the GPU preview did not come from the Catallax kernel")?;
        let details: HashMap<FaceId, [u32; 2]> =
            description.faces.iter().map(|face| (face.face, detail(face, params))).collect();
        let mesh = self
            .0
            .mesh(description, |face| catallax_preview::mesher::FaceDetail(details[face]))
            .await
            .map_err(|refusal| refusal.to_string())?;
        Ok(Meshed {
            origin: mesh.origin,
            buffers: mesh.buffers,
            faces: mesh
                .faces
                .into_iter()
                .map(|face| MeshedFace {
                    face: face.face,
                    sampled: face.draw.as_ref().is_ok_and(|draw| draw.sampled_grid),
                    draw: face.draw.map(|draw| draw.indices).map_err(|refusal| refusal.code()),
                })
                .collect(),
        })
    }
}

/// Where a ray in world coordinates first meets `face` of `description`, by Newton's
/// method from the nearest crossing of a sampled grid over the face's chart. The
/// screen's id buffer has already said the ray hits this face.
pub(crate) fn ray_point(description: &Description, face: FaceId, origin: Vec3, direction: Vec3) -> Option<Vec3> {
    let face = description.faces.iter().find(|f| f.face == face)?;
    let o = description.origin;
    let ray_origin = [origin.x - o[0], origin.y - o[1], origin.z - o[2]];
    let d = [direction.x, direction.y, direction.z];
    let length = dot(d, d).sqrt();
    if !length.is_finite() || length == 0.0 {
        return None;
    }
    let d = d.map(|x| x / length);
    const N: usize = 32;
    let [[u0, u1], [v0, v1]] = face.bounds;
    let uv = |i: usize, j: usize| [u0 + (u1 - u0) * i as f64 / N as f64, v0 + (v1 - v0) * j as f64 / N as f64];
    let grid: Vec<Option<[f64; 3]>> =
        (0..=N).flat_map(|j| (0..=N).map(move |i| (i, j))).map(|(i, j)| evaluate(face, uv(i, j))).collect();
    let at = |i: usize, j: usize| grid[j * (N + 1) + i];
    let mut hits = Vec::new();
    for j in 0..N {
        for i in 0..N {
            let corners = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            let Some(points) = corners.iter().map(|&(a, b)| at(a, b)).collect::<Option<Vec<_>>>() else { continue };
            for tri in [[0, 1, 2], [0, 2, 3]] {
                if let Some((t, b1, b2)) = triangle_hit(ray_origin, d, points[tri[0]], points[tri[1]], points[tri[2]]) {
                    let [p, q, r] = tri.map(|k| uv(corners[k].0, corners[k].1));
                    let guess =
                        std::array::from_fn(|axis| p[axis] + b1 * (q[axis] - p[axis]) + b2 * (r[axis] - p[axis]));
                    hits.push((t, guess));
                }
            }
        }
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (t, guess) = *hits.first()?;
    let point =
        refine(face, ray_origin, d, guess, t).unwrap_or_else(|| std::array::from_fn(|k| ray_origin[k] + t * d[k]));
    Some(Vec3::new(point[0] + o[0], point[1] + o[1], point[2] + o[2]))
}

/// Solves S(u, v) = o + t d near `guess` and `t`.
fn refine(face: &DisplayFace<FaceId>, o: [f64; 3], d: [f64; 3], guess: [f64; 2], t: f64) -> Option<[f64; 3]> {
    let (mut u, mut v, mut t) = (guess[0], guess[1], t);
    let span = face.bounds.map(|[lo, hi]| (hi - lo).abs().max(f64::MIN_POSITIVE));
    for _ in 0..24 {
        let p = evaluate(face, [u, v])?;
        let f: [f64; 3] = std::array::from_fn(|k| p[k] - (o[k] + t * d[k]));
        if dot(f, f).sqrt() < 1e-9 * (1.0 + dot(p, p).sqrt()) {
            return Some(p);
        }
        let (hu, hv) = (span[0] * 1e-6, span[1] * 1e-6);
        let pu = evaluate(face, [u + hu, v])?;
        let pv = evaluate(face, [u, v + hv])?;
        let su: [f64; 3] = std::array::from_fn(|k| (pu[k] - p[k]) / hu);
        let sv: [f64; 3] = std::array::from_fn(|k| (pv[k] - p[k]) / hv);
        // [su sv -d] [du dv dt]^T = -f
        let m = [su, sv, d.map(|x| -x)];
        let det = dot(m[0], cross(m[1], m[2]));
        if !det.is_finite() || det.abs() < 1e-300 {
            return None;
        }
        let rhs = f.map(|x| -x);
        let du = dot(rhs, cross(m[1], m[2])) / det;
        let dv = dot(m[0], cross(rhs, m[2])) / det;
        let dt = dot(m[0], cross(m[1], rhs)) / det;
        u += du;
        v += dv;
        t += dt;
        if ![u, v, t].iter().all(|x| x.is_finite()) {
            return None;
        }
    }
    None
}

fn triangle_hit(o: [f64; 3], d: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Option<(f64, f64, f64)> {
    let e1 = sub(b, a);
    let e2 = sub(c, a);
    let p = cross(d, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-18 {
        return None;
    }
    let s = sub(o, a);
    let u = dot(s, p) / det;
    let q = cross(s, e1);
    let v = dot(d, q) / det;
    let t = dot(e2, q) / det;
    ((0.0..=1.0).contains(&u) && v >= 0.0 && u + v <= 1.0 && t > 0.0).then_some((t, u, v))
}

/// The face's carrier at chart coordinates `uv`, body-relative.
fn evaluate(face: &DisplayFace<FaceId>, [u, v]: [f64; 2]) -> Option<[f64; 3]> {
    let at = |frame: &catallax_mesh::float::DisplayFrame, a: f64, b: f64, c: f64| -> [f64; 3] {
        std::array::from_fn(|k| frame.origin[k] + a * frame.x[k] + b * frame.y[k] + c * frame.z[k])
    };
    let (s, c) = (u * TAU).sin_cos();
    let p = match &face.surface {
        DisplaySurface::Plane(frame) => at(frame, u, v, 0.0),
        DisplaySurface::Cylinder { frame, radius } => at(frame, radius * c, radius * s, v),
        DisplaySurface::Cone { frame, slope } => at(frame, v * slope * c, v * slope * s, v),
        DisplaySurface::Sphere { frame, radius } => {
            let (sv, cv) = (v * TAU).sin_cos();
            at(frame, radius * cv * c, radius * cv * s, radius * sv)
        }
        DisplaySurface::Torus { frame, major, minor } => {
            let (sv, cv) = (v * TAU).sin_cos();
            let r = major + minor * cv;
            at(frame, r * c, r * s, minor * sv)
        }
        DisplaySurface::EllipticCylinder { frame, radii } => at(frame, radii[0] * c, radii[1] * s, v),
        DisplaySurface::BSpline(spline) => spline_point(spline, u, v)?,
        DisplaySurface::Grid(grid) => {
            let [[u0, u1], [v0, v1]] = face.bounds;
            let [nu, nv] = grid.size;
            if nu < 2 || nv < 2 || grid.points.len() != nu * nv {
                return None;
            }
            let x = ((u - u0) / (u1 - u0)).clamp(0.0, 1.0) * (nu - 1) as f64;
            let y = ((v - v0) / (v1 - v0)).clamp(0.0, 1.0) * (nv - 1) as f64;
            let (i, j) = ((x as usize).min(nu - 2), (y as usize).min(nv - 2));
            let (fx, fy) = (x - i as f64, y - j as f64);
            let q = |a: usize, b: usize| grid.points[b * nu + a];
            std::array::from_fn(|k| {
                let bottom = q(i, j)[k] + (q(i + 1, j)[k] - q(i, j)[k]) * fx;
                let top = q(i, j + 1)[k] + (q(i + 1, j + 1)[k] - q(i, j + 1)[k]) * fx;
                bottom + (top - bottom) * fy
            })
        }
    };
    p.iter().all(|x| x.is_finite()).then_some(p)
}

fn spline_point(spline: &DisplaySpline, u: f64, v: f64) -> Option<[f64; 3]> {
    let (su, bu) = basis(&spline.knots[0], spline.degree[0], spline.size[0], u)?;
    let (sv, bv) = basis(&spline.knots[1], spline.degree[1], spline.size[1], v)?;
    let mut p = [0.0; 3];
    let mut sum = 0.0;
    for (j, b) in bv.iter().enumerate() {
        for (i, a) in bu.iter().enumerate() {
            let index = (sv + j) * spline.size[0] + su + i;
            let w = a * b * spline.weights.as_ref().map_or(1.0, |w| w[index]);
            let point = spline.points.get(index)?;
            for k in 0..3 {
                p[k] += w * point[k];
            }
            sum += w;
        }
    }
    (sum != 0.0).then(|| p.map(|x| x / sum))
}

/// The first control index and the nonzero basis values at `t`.
fn basis(knots: &[f64], degree: usize, n: usize, t: f64) -> Option<(usize, Vec<f64>)> {
    if degree >= n || knots.len() != n + degree + 1 {
        return None;
    }
    let t = t.clamp(knots[degree], knots[n]);
    let span = knots.partition_point(|&k| k <= t).saturating_sub(1).clamp(degree, n - 1);
    let mut values = vec![0.0; degree + 1];
    let mut left = vec![0.0; degree + 1];
    let mut right = vec![0.0; degree + 1];
    values[0] = 1.0;
    for j in 1..=degree {
        left[j] = t - knots[span + 1 - j];
        right[j] = knots[span + j] - t;
        let mut saved = 0.0;
        for r in 0..j {
            let denom = right[r + 1] + left[j - r];
            let term = if denom > 0.0 { values[r] / denom } else { 0.0 };
            values[r] = saved + right[r + 1] * term;
            saved = left[j - r] * term;
        }
        values[j] = saved;
    }
    Some((span - degree, values))
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k] - b[k])
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use catallax_mesh::float::{DisplayFrame, DisplayLoop};

    fn frame() -> DisplayFrame {
        DisplayFrame { origin: [0.0; 3], x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0], z: [0.0, 0.0, 1.0] }
    }

    fn face(surface: DisplaySurface, bounds: [[f64; 2]; 2]) -> Description {
        DisplayDescription {
            version: catallax_mesh::float::DISPLAY_DESCRIPTION_VERSION,
            origin: [10.0, 0.0, 0.0],
            edges: Vec::new(),
            faces: vec![DisplayFace {
                face: FaceId::from_raw(7),
                reversed: false,
                surface,
                bounds,
                periods: [None, None],
                loops: Vec::<DisplayLoop>::new(),
            }],
        }
    }

    #[test]
    fn a_ray_meets_a_plane_and_a_cylinder_where_the_carrier_is() {
        let plane = face(DisplaySurface::Plane(frame()), [[-5.0, 5.0], [-5.0, 5.0]]);
        let hit = ray_point(&plane, FaceId::from_raw(7), Vec3::new(11.0, 2.0, 9.0), Vec3::new(0.0, 0.0, -2.0)).unwrap();
        assert!((hit.x - 11.0).abs() < 1e-9 && (hit.y - 2.0).abs() < 1e-9 && hit.z.abs() < 1e-9, "{hit:?}");

        let cylinder = face(DisplaySurface::Cylinder { frame: frame(), radius: 3.0 }, [[0.0, 1.0], [0.0, 4.0]]);
        let hit =
            ray_point(&cylinder, FaceId::from_raw(7), Vec3::new(10.0, -20.0, 1.0), Vec3::new(0.0, 1.0, 0.0)).unwrap();
        assert!((hit.x - 10.0).abs() < 1e-9 && (hit.y + 3.0).abs() < 1e-9 && (hit.z - 1.0).abs() < 1e-9, "{hit:?}");
        assert_eq!(ray_point(&cylinder, FaceId::from_raw(8), Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)), None);
    }

    #[test]
    fn faces_get_no_grid_inside_planes_and_turns_on_cylinders() {
        let params = TessellationParams::default();
        let plane = face(DisplaySurface::Plane(frame()), [[0.0, 1.0], [0.0, 1.0]]);
        assert_eq!(detail(&plane.faces[0], &params), [1, 1]);
        let half = face(DisplaySurface::Cylinder { frame: frame(), radius: 1.0 }, [[0.0, 0.5], [0.0, 2.0]]);
        let [u, v] = detail(&half.faces[0], &params);
        assert_eq!(v, 1);
        assert!(u >= 12, "{u}");
    }
}
