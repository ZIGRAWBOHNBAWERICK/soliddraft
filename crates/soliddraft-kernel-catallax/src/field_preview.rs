//! Bounded, unproved viewport samples of native lattice definitions.

use std::collections::HashMap;

use catallax_field::{Plan, Workspace, printing, workers};
use catallax_numbers::{Decider, Rational, Refusal};
use soliddraft_kernel_api::{Aabb, Vec3};

const MAX_POINTS: usize = 262_144;
const MAX_VERTICES: usize = 500_000;
const TETRAHEDRA: [[usize; 4]; 6] =
    [[0, 1, 2, 6], [0, 2, 3, 6], [0, 3, 7, 6], [0, 7, 4, 6], [0, 4, 5, 6], [0, 5, 1, 6]];
const CORNERS: [[usize; 3]; 8] =
    [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0], [0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]];

/// This mesh is display only. Field exports still call the native stage's exporter.
pub(super) fn sample(plan: &Plan, bounds: Aabb, radius: f64, dec: &mut Decider) -> Result<printing::Mesh, Refusal> {
    let extent = bounds.max - bounds.min;
    let target = radius.min(extent.x.max(extent.y).max(extent.z) / 32.0);
    if !target.is_finite() || target <= 0.0 {
        return Err(Refusal::new("field.print-options", "the preview needs a positive finite sampling size"));
    }
    let pitch = target.log2().floor().exp2();
    let minimum = [bounds.min.x, bounds.min.y, bounds.min.z];
    let maximum = [bounds.max.x, bounds.max.y, bounds.max.z];
    let origin = minimum.map(|value| (value / pitch).floor() * pitch - pitch);
    let mut counts = [0; 3];
    let mut total = 1usize;
    for axis in 0..3 {
        let count = ((maximum[axis] - origin[axis]) / pitch).ceil() + 1.0;
        if !count.is_finite() || count > MAX_POINTS as f64 {
            return Err(budget());
        }
        counts[axis] = count as usize;
        total = total.checked_mul(counts[axis] + 1).filter(|n| *n <= MAX_POINTS).ok_or_else(budget)?;
    }
    let index = |x, y, z| (x * (counts[1] + 1) + y) * (counts[2] + 1) + z;
    let mut points = Vec::with_capacity(total);
    for x in 0..=counts[0] {
        for y in 0..=counts[1] {
            for z in 0..=counts[2] {
                points.push(std::array::from_fn(|axis| origin[axis] + [x, y, z][axis] as f64 * pitch));
            }
        }
    }
    let mut values = Vec::with_capacity(total);
    for chunk in points.chunks(8192) {
        dec.check_deadline()?;
        for value in
            workers::map_init(chunk, Workspace::default, |storage, &point| plan.unproved_preview(point, storage))
        {
            values.push(value?.value);
        }
    }
    let mut surface = Surface::new(&points, &values);
    for x in 0..counts[0] {
        dec.check_deadline()?;
        for y in 0..counts[1] {
            for z in 0..counts[2] {
                let cube = CORNERS.map(|[dx, dy, dz]| index(x + dx, y + dy, z + dz));
                for tetrahedron in TETRAHEDRA {
                    surface.tetrahedron(tetrahedron.map(|corner| cube[corner]))?;
                }
            }
        }
    }
    // Like the native sampled preview, this records sampling scale, not a proof.
    surface.mesh.max_deviation = Rational::from_f64_exact(pitch * 4.0).ok_or_else(budget)?;
    surface.mesh.cells = counts.iter().product();
    Ok(surface.mesh)
}

fn budget() -> Refusal {
    Refusal::new("field.print-budget", "the viewport sampling grid is too large; increase the beam radius")
}

struct Surface<'a> {
    points: &'a [[f64; 3]],
    values: &'a [f64],
    edges: HashMap<(usize, usize), usize>,
    mesh: printing::Mesh,
}

impl<'a> Surface<'a> {
    fn new(points: &'a [[f64; 3]], values: &'a [f64]) -> Self {
        Self {
            points,
            values,
            edges: HashMap::new(),
            mesh: printing::Mesh { positions: vec![], triangles: vec![], max_deviation: Rational::zero(), cells: 0 },
        }
    }

    fn vertex(&mut self, a: usize, b: usize) -> Result<usize, Refusal> {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        let t = self.values[a] / (self.values[a] - self.values[b]);
        let key = if t == 0.0 {
            (a, a)
        } else if t == 1.0 {
            (b, b)
        } else {
            (a, b)
        };
        if let Some(index) = self.edges.get(&key) {
            return Ok(*index);
        }
        if self.mesh.positions.len() == MAX_VERTICES {
            return Err(Refusal::new("field.print-vertices", "the viewport sampling vertex limit was reached"));
        }
        let point = Vec3::from(self.points[a]) + (Vec3::from(self.points[b]) - Vec3::from(self.points[a])) * t;
        let index = self.mesh.positions.len();
        self.mesh.positions.push([point.x as f32, point.y as f32, point.z as f32]);
        self.edges.insert(key, index);
        Ok(index)
    }

    fn tetrahedron(&mut self, vertices: [usize; 4]) -> Result<(), Refusal> {
        let (mut inside, mut outside) = ([0; 4], [0; 4]);
        let (mut ni, mut no) = (0, 0);
        for vertex in vertices {
            if self.values[vertex] < 0.0 {
                inside[ni] = vertex;
                ni += 1;
            } else {
                outside[no] = vertex;
                no += 1;
            }
        }
        if ni == 0 || no == 0 {
            return Ok(());
        }
        let centre = |ids: &[usize]| {
            ids.iter().fold(Vec3::ZERO, |sum, &id| sum + Vec3::from(self.points[id])) * (1.0 / ids.len() as f64)
        };
        let outward = centre(&outside[..no]) - centre(&inside[..ni]);
        if ni == 2 {
            let a = self.vertex(inside[0], outside[0])?;
            let b = self.vertex(inside[0], outside[1])?;
            let c = self.vertex(inside[1], outside[1])?;
            let d = self.vertex(inside[1], outside[0])?;
            self.triangle([a, b, c], outward);
            self.triangle([a, c, d], outward);
        } else {
            let (one, others) = if ni == 1 { (inside[0], &outside[..no]) } else { (outside[0], &inside[..ni]) };
            let a = self.vertex(one, others[0])?;
            let b = self.vertex(one, others[1])?;
            let c = self.vertex(one, others[2])?;
            self.triangle([a, b, c], outward);
        }
        Ok(())
    }

    fn triangle(&mut self, mut indices: [usize; 3], outward: Vec3) {
        let [a, b, c] = indices.map(|index| Vec3::from(self.mesh.positions[index].map(f64::from)));
        let normal = (b - a).cross(c - a);
        if normal.length() == 0.0 {
            return;
        }
        if normal.dot(outward) < 0.0 {
            indices.swap(1, 2);
        }
        self.mesh.triangles.push(indices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ridealong_sampled_planes_have_their_expected_intersections_and_orientation() {
        let points = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        for (values, direction, triangles) in [
            ([-0.25, 0.75, -0.25, -0.25], Vec3::X, 1),
            ([0.25, -0.75, 0.25, 0.25], -Vec3::X, 1),
            ([-0.5, 0.5, 0.5, -0.5], Vec3::X + Vec3::Y, 2),
        ] {
            let mut surface = Surface::new(&points, &values);
            surface.tetrahedron([0, 1, 2, 3]).unwrap();
            assert_eq!(surface.mesh.triangles.len(), triangles);
            for position in &surface.mesh.positions {
                let point = Vec3::from(position.map(f64::from));
                let expected = if triangles == 2 { 0.5 } else { 0.25 };
                let actual = if triangles == 2 { point.x + point.y } else { point.x };
                assert!((actual - expected).abs() < 1e-6);
            }
            for indices in &surface.mesh.triangles {
                let [a, b, c] = indices.map(|index| Vec3::from(surface.mesh.positions[index].map(f64::from)));
                assert!((b - a).cross(c - a).dot(direction) > 0.0);
            }
        }
    }
}
