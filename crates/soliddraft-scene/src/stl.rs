//! STL, binary or ASCII: a node per solid.

use std::io::Read;
use std::path::Path;

use glam::DVec3;

use crate::{Body, Format, ImportError, Mesh, Origin, Scene, UpAxis, header_hints, stem};

/// The solids of an STL file in the file's units, the header text, and notes on what
/// the file holds that is not read.
pub(crate) type Solids = (Vec<Mesh>, String, Vec<String>);

/// Reads every solid of an STL file: a binary file has one; an ASCII file may hold
/// several, each named on its `solid` line. Facet normals are checked against the
/// winding, which is kept. Colours (in a binary file's attribute bytes or a `COLOR=`
/// header) are not read.
pub(crate) fn solids(path: &Path) -> Result<Solids, ImportError> {
    let bytes = std::fs::read(path).map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
    let header = header_text(&bytes);
    let invalid = |e: std::io::Error| ImportError(format!("{} is not a valid STL file: {e}", path.display()));
    let mut notes = Vec::new();
    let binary = bytes.len() >= 84 && {
        let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        count.checked_mul(50).and_then(|n| n.checked_add(84)) == Some(bytes.len())
    };
    // Each solid's bytes and name.
    let mut parts: Vec<(Vec<u8>, String)> = Vec::new();
    if binary {
        let attributes = bytes[84..].as_chunks::<50>().0.iter().any(|f| f[48] != 0 || f[49] != 0);
        if attributes || header.contains("COLOR=") {
            notes.push("colours in the file are not imported".to_owned());
        }
        parts.push((bytes.clone(), stem(path)));
    } else {
        let text = String::from_utf8_lossy(&bytes);
        let mut current: Option<(String, String)> = None;
        for line in text.lines() {
            let word = line.split_whitespace().next().unwrap_or_default();
            if word == "solid" && current.is_none() {
                let name = line.trim_start().trim_start_matches("solid").trim().to_owned();
                current = Some((format!("{line}\n"), name));
            } else if let Some((block, name)) = current.as_mut() {
                block.push_str(line);
                block.push('\n');
                if word == "endsolid" {
                    let name = if name.is_empty() { stem(path) } else { name.clone() };
                    parts.push((block.clone().into_bytes(), name));
                    current = None;
                }
            }
        }
        if parts.is_empty() {
            parts.push((bytes.clone(), stem(path)));
        }
    }
    let mut meshes = Vec::new();
    let mut disagree = 0usize;
    for (part, name) in parts {
        let indexed = stl_io::read_stl(&mut std::io::Cursor::new(&part)).map_err(invalid)?;
        let mut mesh = Mesh { name, ..Mesh::default() };
        mesh.positions =
            indexed.vertices.iter().map(|v| DVec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2]))).collect();
        for f in &indexed.faces {
            let [a, b, c] = f.vertices.map(|i| i as u32);
            if a == b || b == c || a == c {
                continue;
            }
            let [pa, pb, pc] = [a, b, c].map(|i| mesh.positions[i as usize]);
            let normal = DVec3::new(f64::from(f.normal[0]), f64::from(f.normal[1]), f64::from(f.normal[2]));
            if (pb - pa).cross(pc - pa).dot(normal) < 0.0 {
                disagree += 1;
            }
            mesh.triangles.push([a, b, c]);
        }
        meshes.push(mesh);
    }
    if disagree > 0 {
        notes.push(format!("{disagree} facets have a normal that disagrees with their winding; the winding is kept"));
    }
    Ok((meshes, header, notes))
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
    let (mut meshes, header, notes) = solids(path)?;
    let (units, up) = header_hints(&header);
    // A sand3d header starts with the object's name: "plate — written by sand3d".
    if let ([mesh], Some(name)) =
        (meshes.as_mut_slice(), header.split(" — ").next().filter(|n| !n.is_empty() && header.contains(" — ")))
    {
        mesh.name = name.to_owned();
    } else if let [mesh] = meshes.as_mut_slice() {
        mesh.name = stem(path);
    }
    let mut scene = Scene::new(stem(path), Format::Stl, units.unwrap_or(0.001), up.unwrap_or(UpAxis::Z));
    scene.units_from = if units.is_some() { Origin::Header } else { Origin::Default };
    scene.up_from = if up.is_some() { Origin::Header } else { Origin::Default };
    for n in notes {
        scene.note(n);
    }
    for mesh in meshes.into_iter().filter(|m| !m.triangles.is_empty()) {
        let node = scene.add(0, mesh.name.clone(), format!("solid {}", mesh.name), glam::DMat4::IDENTITY);
        let m = scene.add_mesh(mesh);
        scene.nodes[node].body = Some(Body { mesh: m, material: None });
    }
    Ok(scene)
}
