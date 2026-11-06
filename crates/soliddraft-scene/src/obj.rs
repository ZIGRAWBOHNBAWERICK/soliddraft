//! Wavefront OBJ: a node per object, with the MTL file's diffuse colours.

use std::path::Path;

use glam::{DMat4, DVec3};

use crate::{Body, Format, ImportError, Material, Mesh, Origin, Scene, UpAxis, header_hints, stem, triangulate};

/// The objects of a file with their materials, and the count of polygons fanned.
pub(crate) type Objects = (Vec<(Mesh, Option<Material>)>, usize);

/// The objects of an OBJ file in the file's units, each with its material, and how many
/// polygons crossed themselves or had no area. Polygons are cut by ear clipping
/// ([`triangulate`]), not by the loader's fan, so concave faces keep their notches.
pub(crate) fn objects(path: &Path) -> Result<Objects, ImportError> {
    let options = tobj::LoadOptions { triangulate: false, single_index: true, ignore_points: true, ignore_lines: true };
    let (models, materials) = tobj::load_obj(path, &options)
        .map_err(|e| ImportError(format!("{} is not a valid OBJ file: {e}", path.display())))?;
    // A missing or broken MTL file leaves the objects without colour, not the import.
    let materials = materials.unwrap_or_default();
    let mut out = Vec::new();
    let mut crossed = 0;
    for model in models {
        let m = &model.mesh;
        let positions: Vec<DVec3> = m
            .positions
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
            .collect();
        if m.indices.iter().any(|i| *i as usize >= positions.len()) {
            return Err(ImportError(format!(
                "object {} of {} has an index past its points",
                model.name,
                path.display()
            )));
        }
        // No arities means every face is a triangle.
        let counts: Vec<usize> = if m.face_arities.is_empty() {
            vec![3; m.indices.len() / 3]
        } else {
            m.face_arities.iter().map(|a| *a as usize).collect()
        };
        let (triangles, corners, bad) = triangulate(&positions, &counts, &m.indices, false).ok_or_else(|| {
            ImportError(format!("object {} of {} has fewer indices than its faces need", model.name, path.display()))
        })?;
        crossed += bad;
        let corner_normals = (m.normals.len() == m.positions.len() && !m.normals.is_empty()).then(|| {
            corners
                .iter()
                .flat_map(|t| {
                    t.map(|c| {
                        let k = m.indices[c] as usize * 3;
                        DVec3::new(f64::from(m.normals[k]), f64::from(m.normals[k + 1]), f64::from(m.normals[k + 2]))
                    })
                })
                .collect()
        });
        let material = m.material_id.and_then(|i| materials.get(i)).map(|mtl| {
            let d = mtl.diffuse.unwrap_or([0.8, 0.8, 0.8]);
            Material {
                name: mtl.name.clone(),
                base_color: [f64::from(d[0]), f64::from(d[1]), f64::from(d[2]), f64::from(mtl.dissolve.unwrap_or(1.0))],
                metallic: 0.0,
                roughness: 0.5,
                textured: mtl.diffuse_texture.is_some(),
            }
        });
        out.push((Mesh { name: model.name.clone(), positions, triangles, corner_normals }, material));
    }
    Ok((out, crossed))
}

pub(crate) fn read(path: &Path) -> Result<Scene, ImportError> {
    let text = std::fs::read(path).map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
    let header: String = String::from_utf8_lossy(&text)
        .lines()
        .take_while(|l| l.trim_start().starts_with('#') || l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let (units, up) = header_hints(&header);
    let mut scene = Scene::new(stem(path), Format::Obj, units.unwrap_or(0.001), up.unwrap_or(UpAxis::Y));
    scene.units_from = if units.is_some() { Origin::Header } else { Origin::Default };
    scene.up_from = if up.is_some() { Origin::Header } else { Origin::Default };
    let (objects, crossed) = objects(path)?;
    if crossed > 0 {
        scene.note("polygons that cross themselves or have no area are split into fans from their first corner");
    }
    for (i, (mesh, material)) in objects.into_iter().enumerate() {
        if mesh.triangles.is_empty() {
            continue;
        }
        let name = if mesh.name.is_empty() { format!("object {}", i + 1) } else { mesh.name.clone() };
        let material = material.map(|m| {
            scene.materials.push(m);
            scene.materials.len() - 1
        });
        let node = scene.add(0, name.clone(), format!("o {name}"), DMat4::IDENTITY);
        let mesh = scene.add_mesh(mesh);
        scene.nodes[node].body = Some(Body { mesh, material });
    }
    Ok(scene)
}
