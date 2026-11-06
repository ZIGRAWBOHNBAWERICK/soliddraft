//! STL, binary or ASCII: one mesh, one node.

use std::io::Read;
use std::path::Path;

use glam::DVec3;

use crate::{Body, Format, ImportError, Mesh, Origin, Scene, UpAxis, header_hints, stem};

/// The mesh of an STL file, in the file's units, with the name its header gives.
pub(crate) fn mesh(path: &Path) -> Result<(Mesh, String), ImportError> {
    let bytes = std::fs::read(path).map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
    let header = header_text(&bytes);
    let indexed = stl_io::read_stl(&mut std::io::Cursor::new(&bytes))
        .map_err(|e| ImportError(format!("{} is not a valid STL file: {e}", path.display())))?;
    let mut mesh = Mesh { name: stem(path), ..Mesh::default() };
    mesh.positions =
        indexed.vertices.iter().map(|v| DVec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2]))).collect();
    mesh.triangles = indexed
        .faces
        .iter()
        .map(|f| f.vertices.map(|i| i as u32))
        .filter(|[a, b, c]| a != b && b != c && a != c)
        .collect();
    Ok((mesh, header))
}

/// The text of the header: the 80 bytes of a binary file, or the `solid` line of an
/// ASCII one, without the keyword.
fn header_text(bytes: &[u8]) -> String {
    let mut head = Vec::new();
    let _ = (&bytes[..bytes.len().min(80)]).read_to_end(&mut head);
    let text = String::from_utf8_lossy(&head);
    let line = text.lines().next().unwrap_or_default();
    line.trim_start_matches("solid").trim_matches(|c: char| c.is_whitespace() || c == '\0').to_owned()
}

pub(crate) fn read(path: &Path) -> Result<Scene, ImportError> {
    let (mut mesh, header) = mesh(path)?;
    let (units, up) = header_hints(&header);
    // A sand3d header starts with the object's name: "plate — written by sand3d".
    if let Some(name) = header.split(" — ").next().filter(|n| !n.is_empty() && header.contains(" — ")) {
        mesh.name = name.to_owned();
    }
    let mut scene = Scene::new(stem(path), Format::Stl, units.unwrap_or(0.001), up.unwrap_or(UpAxis::Z));
    scene.units_from = if units.is_some() { Origin::Header } else { Origin::Default };
    scene.up_from = if up.is_some() { Origin::Header } else { Origin::Default };
    let name = mesh.name.clone();
    let node = scene.add(0, name, "solid", glam::DMat4::IDENTITY);
    let m = scene.add_mesh(mesh);
    scene.nodes[node].body = Some(Body { mesh: m, material: None });
    Ok(scene)
}
