//! The distance from a point to a face as trimmed: the binary64 carrier the kernel
//! describes, restricted to the face's trim.
//!
//! The kernel's ray query meets the carrier in binary64 inside the face's chart, and
//! refines the hit with Newton steps. A hit counts as on the trimmed face when it lies
//! within the checked face's measured sag of the checked face; the checked mesh comes
//! from the kernel's certified path, not from the description the GPU mesher reads. A
//! point whose carrier hit lies off the trimmed face, or that meets no carrier, is
//! measured to the face's boundary edges instead. So a preview drawn on the right
//! carrier but over the wrong part of it, such as the wrong arc of a torus, stays far
//! from its face.

#![allow(dead_code)]

use std::collections::HashMap;

use soliddraft_kernel_api::{BodyId, FaceId, Kernel, Vec3};

pub type Point = [f64; 3];

/// One face's trimmed carrier, measured against its checked mesh.
pub struct TrimmedCarrier<'a> {
    kernel: &'a dyn Kernel,
    body: BodyId,
    face: FaceId,
    checked: Triangles,
    /// Segments of the face's boundary edges.
    boundary: Vec<[Point; 2]>,
    /// How far the checked face lies from the carrier, at most, as measured.
    pub sag: f64,
    /// Checked samples whose carrier the ray query did not meet.
    pub sag_misses: usize,
    /// How far along a normal to look for the carrier, either way.
    reach: f64,
}

/// A point measured against a trimmed carrier.
#[derive(Clone, Copy, Debug)]
pub struct Measured {
    /// The distance to the trimmed face.
    pub distance: f64,
    /// Whether the carrier hit lay on the trimmed face. When false, `distance` is to the
    /// boundary edges.
    pub on_face: bool,
    /// Whether the ray query met the carrier at all.
    pub hit: bool,
}

impl<'a> TrimmedCarrier<'a> {
    /// `checked` holds the face's checked triangles and `boundary` its edges' samples,
    /// all in world coordinates. `tolerance` is the preview tolerance being checked.
    pub fn new(
        kernel: &'a dyn Kernel,
        body: BodyId,
        face: FaceId,
        checked: Vec<[Point; 3]>,
        boundary: Vec<Vec<Point>>,
        tolerance: f64,
    ) -> Self {
        let checked = Triangles::new(checked, tolerance);
        let boundary = boundary.iter().flat_map(|line| line.windows(2).map(|pair| [pair[0], pair[1]])).collect();
        let mut carrier = TrimmedCarrier {
            kernel,
            body,
            face,
            checked,
            boundary,
            sag: 0.0,
            sag_misses: 0,
            reach: (tolerance * 10.0).max(0.05),
        };
        carrier.measure_sag();
        carrier
    }

    /// The checked face's distance from the carrier, at each triangle's centroid and edge
    /// midpoints, where a flat triangle strays most from a curved carrier. At most 512
    /// samples per face, evenly spread.
    fn measure_sag(&mut self) {
        let samples: Vec<(Point, Point)> = self
            .checked
            .triangles
            .iter()
            .flat_map(|&[a, b, c]| {
                let normal = unit(cross(sub(b, a), sub(c, a)));
                let mid = |p: Point, q: Point| -> Point { std::array::from_fn(|i| (p[i] + q[i]) / 2.0) };
                let centroid: Point = std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.0);
                normal.map(|n| [(centroid, n), (mid(a, b), n), (mid(b, c), n), (mid(c, a), n)]).into_iter().flatten()
            })
            .collect();
        let step = samples.len().div_ceil(512).max(1);
        for &(point, normal) in samples.iter().step_by(step) {
            match self.carrier_hit(point, normal) {
                Some(hit) => self.sag = self.sag.max(length(sub(hit, point))),
                None => self.sag_misses += 1,
            }
        }
    }

    /// The carrier point nearest `point` along `normal`, looking both ways. A hit past the
    /// reach is another part of the carrier, not this point's foot on it.
    fn carrier_hit(&self, point: Point, normal: Point) -> Option<Point> {
        let mut best: Option<Point> = None;
        for sign in [1.0, -1.0] {
            let direction: Point = normal.map(|x| x * sign);
            let origin: Point = std::array::from_fn(|i| point[i] - direction[i] * self.reach);
            let hit = self
                .kernel
                .face_ray_point(self.body, self.face, vec3(origin), vec3(direction))
                .ok()
                .flatten()
                .map(|v| [v.x, v.y, v.z]);
            if let Some(hit) = hit
                && length(sub(hit, point)) <= self.reach
                && best.is_none_or(|b| length(sub(hit, point)) < length(sub(b, point)))
            {
                best = Some(hit);
            }
        }
        best
    }

    /// The distance from `point`, which has `normal`, to the trimmed face.
    pub fn measure(&self, point: Point, normal: Point, tolerance: f64) -> Measured {
        let hit = unit(normal).and_then(|normal| self.carrier_hit(point, normal));
        if let Some(hit) = hit
            && self.checked.distance(hit) <= self.sag + tolerance
        {
            return Measured { distance: length(sub(hit, point)), on_face: true, hit: true };
        }
        let boundary = self.boundary.iter().map(|&[a, b]| segment_distance(point, a, b)).fold(f64::INFINITY, f64::min);
        Measured { distance: boundary, on_face: false, hit: hit.is_some() }
    }
}

fn vec3(p: Point) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn dot(a: Point, b: Point) -> f64 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn cross(a: Point, b: Point) -> Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: Point) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: Point) -> Option<Point> {
    let l = length(a);
    (l.is_finite() && l > 1e-12).then(|| a.map(|x| x / l))
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (u, d) = (sub(b, a), sub(p, a));
    let len = dot(u, u);
    let t = if len > 0.0 { (dot(u, d) / len).clamp(0.0, 1.0) } else { 0.0 };
    length(std::array::from_fn(|i| d[i] - t * u[i]))
}

/// Triangles bucketed in a grid, for the distance from a point to the nearest.
pub struct Triangles {
    pub triangles: Vec<[Point; 3]>,
    cell: f64,
    reach: f64,
    cells: HashMap<[i64; 3], Vec<usize>>,
}

impl Triangles {
    pub fn new(triangles: Vec<[Point; 3]>, tolerance: f64) -> Self {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in triangles.iter().flatten() {
            for i in 0..3 {
                lo[i] = lo[i].min(p[i]);
                hi[i] = hi[i].max(p[i]);
            }
        }
        let diagonal = (0..3).map(|i| (hi[i] - lo[i]).max(0.0).powi(2)).sum::<f64>().sqrt();
        let cell = (diagonal / 128.0).max(0.5);
        // Look past the tolerance, so a deviation over it is measured rather than capped.
        let reach = tolerance * 8.0;
        let mut cells = HashMap::<_, Vec<_>>::new();
        for (i, t) in triangles.iter().enumerate() {
            let low: [i64; 3] =
                std::array::from_fn(|j| ((t[0][j].min(t[1][j]).min(t[2][j]) - reach) / cell).floor() as i64);
            let high: [i64; 3] =
                std::array::from_fn(|j| ((t[0][j].max(t[1][j]).max(t[2][j]) + reach) / cell).floor() as i64);
            for z in low[2]..=high[2] {
                for y in low[1]..=high[1] {
                    for x in low[0]..=high[0] {
                        cells.entry([x, y, z]).or_default().push(i);
                    }
                }
            }
        }
        Triangles { triangles, cell, reach, cells }
    }

    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty()
    }

    /// The distance from `p` to the nearest triangle, or to every triangle when none is
    /// within reach.
    pub fn distance(&self, p: Point) -> f64 {
        let near = self
            .cells
            .get(&p.map(|x| (x / self.cell).floor() as i64))
            .map(|near| near.iter().map(|&i| distance_sq(p, self.triangles[i])).fold(f64::INFINITY, f64::min))
            .unwrap_or(f64::INFINITY);
        if near.sqrt() <= self.reach {
            return near.sqrt();
        }
        self.triangles.iter().map(|&t| distance_sq(p, t)).fold(f64::INFINITY, f64::min).sqrt()
    }
}

fn distance_sq(p: Point, [a, b, c]: [Point; 3]) -> f64 {
    let (u, v, d) = (sub(b, a), sub(c, a), sub(p, a));
    let (uu, uv, vv, du, dv) = (dot(u, u), dot(u, v), dot(v, v), dot(d, u), dot(d, v));
    let det = uu * vv - uv * uv;
    if det > 0.0 {
        let s = (vv * du - uv * dv) / det;
        let t = (uu * dv - uv * du) / det;
        if s >= 0.0 && t >= 0.0 && s + t <= 1.0 {
            let offset: Point = std::array::from_fn(|i| d[i] - s * u[i] - t * v[i]);
            return dot(offset, offset);
        }
    }
    [(a, b), (b, c), (c, a)]
        .into_iter()
        .map(|(a, b)| {
            let (u, d) = (sub(b, a), sub(p, a));
            let len = dot(u, u);
            let t = if len > 0.0 { (dot(u, d) / len).clamp(0.0, 1.0) } else { 0.0 };
            let offset: Point = std::array::from_fn(|i| d[i] - t * u[i]);
            dot(offset, offset)
        })
        .fold(f64::INFINITY, f64::min)
}
