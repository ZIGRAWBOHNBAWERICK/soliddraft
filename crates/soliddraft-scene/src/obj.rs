//! Wavefront OBJ: a node per object, with the MTL file's diffuse colours.

use std::path::Path;

use glam::{DMat4, DVec3};

use crate::{Body, Format, ImportError, Material, Mesh, Origin, Scene, UpAxis, header_hints, stem};

/// The objects of an OBJ file in the file's units, each with its material.
pub(crate) fn objects(path: &Path) -> Result<Vec<(Mesh, Option<Material>)>, ImportError> {
    let options = tobj::LoadOptions { triangulate: true, single_index: true, ..tobj::LoadOptions::default() };
    let (models, materials) = tobj::load_obj(path, &options)
        .map_err(|e| ImportError(format!("{} is not a valid OBJ file: {e}", path.display())))?;
    // A missing or broken MTL file leaves the objects without colour, not the import.
    let materials = materials.unwrap_or_default();
    let mut out = Vec::new();
    for model in models {
        let m = &model.mesh;
        let positions: Vec<DVec3> = m
            .positions
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
            .collect();
        let triangles: Vec<[u32; 3]> = m.indices.as_chunks::<3>().0.to_vec();
        let corner_normals = (m.normals.len() == m.positions.len() && !m.normals.is_empty()).then(|| {
            triangles
                .iter()
                .flat_map(|t| {
                    t.map(|i| {
                        let k = i as usize * 3;
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
    Ok(out)
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
    for (i, (mesh, material)) in objects(path)?.into_iter().enumerate() {
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
