//! Meshes for the primitive shapes URDF and USD can name instead of a mesh.

use std::f64::consts::TAU;

use glam::DVec3;

use crate::Mesh;

/// Segments around a cylinder or sphere.
const SEGMENTS: u32 = 32;

/// A box centred on the origin with edge lengths `size`.
pub(crate) fn cuboid(name: &str, size: DVec3) -> Mesh {
    let h = size / 2.0;
    let mut m = Mesh { name: name.to_owned(), ..Mesh::default() };
    // Each face has its own four corners so its normals stay flat.
    let faces = [
        (DVec3::X, DVec3::Y, DVec3::Z),
        (-DVec3::X, DVec3::Z, DVec3::Y),
        (DVec3::Y, DVec3::Z, DVec3::X),
        (-DVec3::Y, DVec3::X, DVec3::Z),
        (DVec3::Z, DVec3::X, DVec3::Y),
        (-DVec3::Z, DVec3::Y, DVec3::X),
    ];
    let mut normals = Vec::new();
    for (n, u, v) in faces {
        let base = m.positions.len() as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            m.positions.push((n + u * a + v * b) * h);
        }
        m.triangles.push([base, base + 1, base + 2]);
        m.triangles.push([base, base + 2, base + 3]);
        normals.extend([n; 6]);
    }
    m.corner_normals = Some(normals);
    m
}

/// A cylinder along Z, centred on the origin.
pub(crate) fn cylinder(name: &str, radius: f64, length: f64) -> Mesh {
    let mut m = Mesh { name: name.to_owned(), ..Mesh::default() };
    let h = length / 2.0;
    let ring = |z: f64| {
        (0..SEGMENTS).map(move |i| {
            let a = TAU * f64::from(i) / f64::from(SEGMENTS);
            DVec3::new(radius * a.cos(), radius * a.sin(), z)
        })
    };
    m.positions.extend(ring(-h));
    m.positions.extend(ring(h));
    m.positions.push(DVec3::new(0.0, 0.0, -h));
    m.positions.push(DVec3::new(0.0, 0.0, h));
    let (bottom, top) = (2 * SEGMENTS, 2 * SEGMENTS + 1);
    for i in 0..SEGMENTS {
        let j = (i + 1) % SEGMENTS;
        m.triangles.push([i, j, SEGMENTS + j]);
        m.triangles.push([i, SEGMENTS + j, SEGMENTS + i]);
        m.triangles.push([bottom, j, i]);
        m.triangles.push([top, SEGMENTS + i, SEGMENTS + j]);
    }
    m
}

/// A sphere centred on the origin.
pub(crate) fn sphere(name: &str, radius: f64) -> Mesh {
    let mut m = Mesh { name: name.to_owned(), ..Mesh::default() };
    let rings = SEGMENTS / 2;
    for r in 0..=rings {
        let polar = std::f64::consts::PI * f64::from(r) / f64::from(rings);
        for s in 0..SEGMENTS {
            let a = TAU * f64::from(s) / f64::from(SEGMENTS);
            m.positions.push(DVec3::new(polar.sin() * a.cos(), polar.sin() * a.sin(), polar.cos()) * radius);
        }
    }
    for r in 0..rings {
        for s in 0..SEGMENTS {
            let t = (s + 1) % SEGMENTS;
            let (a, b) = (r * SEGMENTS + s, r * SEGMENTS + t);
            let (c, d) = ((r + 1) * SEGMENTS + s, (r + 1) * SEGMENTS + t);
            if r > 0 {
                m.triangles.push([a, c, b]);
            }
            if r + 1 < rings {
                m.triangles.push([b, c, d]);
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(m: &Mesh) -> f64 {
        m.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| m.positions[i as usize]);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    #[test]
    fn shapes_are_closed_and_face_outwards() {
        assert!((volume(&cuboid("b", DVec3::new(2.0, 3.0, 4.0))) - 24.0).abs() < 1e-9);
        let c = volume(&cylinder("c", 1.0, 2.0));
        assert!(c > 6.0 && c < std::f64::consts::TAU, "{c}");
        let s = volume(&sphere("s", 1.0));
        assert!(s > 4.0 && s < 4.0 * std::f64::consts::PI / 3.0, "{s}");
    }
}
