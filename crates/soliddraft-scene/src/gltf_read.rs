//! glTF 2.0, binary (`.glb`) or JSON (`.gltf`): the default scene's node tree.
//!
//! glTF is metres with Y up. A node keeps its transform; a mesh of one primitive is the
//! node's body, and a mesh of several gives the node a child per primitive. Morph
//! targets are applied at the node's weights, or else the mesh's. Materials keep their
//! metallic-roughness factors; alpha counts only for blended ones. A file that requires
//! an extension this reader does not know, such as compressed geometry, is refused. sand3d writes each joint in the extras of a
//! node that holds what the joint moves (`s3d_joint`): that node's frame is the joint's.

use std::path::Path;

use base64::Engine;
use glam::{DMat4, DVec3};
use gltf::mesh::Mode;

use crate::{Body, Format, ImportError, Joint, JointKind, Material, Mesh, Mimic, Scene, UpAxis, srgb, stem};

pub(crate) fn read(path: &Path) -> Result<Scene, ImportError> {
    let bytes = std::fs::read(path).map_err(|e| ImportError(format!("{}: {e}", path.display())))?;
    let gltf = gltf::Gltf::from_slice(&bytes)
        .map_err(|e| ImportError(format!("{} is not a valid glTF file: {e}", path.display())))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let mut buffers = Vec::new();
    for buffer in gltf.buffers() {
        let data = match buffer.source() {
            gltf::buffer::Source::Bin => gltf
                .blob
                .clone()
                .ok_or_else(|| ImportError(format!("{} names a binary chunk it does not have", path.display())))?,
            gltf::buffer::Source::Uri(uri) => load_uri(base, uri)?,
        };
        if data.len() < buffer.length() {
            return Err(ImportError(format!(
                "buffer {} of {} is shorter than it says",
                buffer.index(),
                path.display()
            )));
        }
        buffers.push(data);
    }
    let scene_def = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .ok_or_else(|| ImportError(format!("{} has no scene", path.display())))?;
    let name = scene_def.name().map(str::to_owned).unwrap_or_else(|| stem(path));
    let mut scene = Scene::new(name, Format::Gltf, 1.0, UpAxis::Y);
    scene.materials = gltf.materials().map(|m| material(&m)).collect();
    if gltf.scenes().len() > 1 {
        scene.note("only the default scene is imported");
    }
    if gltf.extensions_used().any(|e| e == "EXT_mesh_gpu_instancing") {
        scene.note("EXT_mesh_gpu_instancing instances are not imported; each mesh is shown once, at its node");
    }
    // Each mesh read once per set of morph weights its nodes give it.
    let mut meshes: std::collections::HashMap<(usize, Vec<u32>), Vec<Primitive>> = std::collections::HashMap::new();
    let mut stack: Vec<(gltf::Node<'_>, usize)> =
        scene_def.nodes().collect::<Vec<_>>().into_iter().rev().map(|n| (n, 0)).collect();
    while let Some((node, parent)) = stack.pop() {
        let local = DMat4::from_cols_array_2d(&node.transform().matrix().map(|c| c.map(f64::from)));
        let label = node.name().map(str::to_owned).unwrap_or_else(|| format!("node {}", node.index()));
        let me = scene.add(parent, label.clone(), format!("nodes[{}]", node.index()), local);
        if let Some(mesh) = node.mesh() {
            let weights: Vec<f32> = node.weights().or(mesh.weights()).unwrap_or_default().to_vec();
            let key = (mesh.index(), weights.iter().map(|w| w.to_bits()).collect());
            let bodies = match meshes.get(&key) {
                Some(b) => b.clone(),
                None => {
                    let b = read_mesh(&mut scene, &mesh, &weights, &buffers)?;
                    meshes.insert(key, b.clone());
                    b
                }
            };
            match bodies.as_slice() {
                [] => {}
                [(m, material)] => scene.nodes[me].body = Some(Body { mesh: *m, material: *material }),
                many => {
                    for (k, (m, material)) in many.iter().enumerate() {
                        let c = scene.add(
                            me,
                            format!("{label} {}", k + 1),
                            format!("nodes[{}] primitive {k}", node.index()),
                            DMat4::IDENTITY,
                        );
                        scene.nodes[c].body = Some(Body { mesh: *m, material: *material });
                    }
                }
            }
        }
        if node.skin().is_some() {
            scene.note("skins are not imported; skinned meshes are shown at rest");
        }
        if let Some(extras) = node.extras() {
            joint(&mut scene, extras.get(), parent, me, local);
        }
        stack.extend(node.children().collect::<Vec<_>>().into_iter().rev().map(|c| (c, me)));
    }
    if gltf.animations().len() > 0 {
        scene.note("animations are not imported; the scene is shown at its rest pose");
    }
    Ok(scene)
}

/// A primitive's mesh and material, as indices into the scene.
type Primitive = (usize, Option<usize>);

/// A buffer's bytes from a data URI or a file beside the glTF.
fn load_uri(base: &Path, uri: &str) -> Result<Vec<u8>, ImportError> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (_, data) = rest
            .split_once(";base64,")
            .ok_or_else(|| ImportError("a glTF data URI that is not base64 is not read".into()))?;
        return base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| ImportError(format!("a glTF data URI is not valid base64: {e}")));
    }
    let decoded = percent_decode(uri);
    let file = base.join(decoded);
    std::fs::read(&file).map_err(|e| ImportError(format!("the glTF buffer {} cannot be read: {e}", file.display())))
}

/// `%20` and the like in a relative URI.
fn percent_decode(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The material's constant factors; glTF colours are linear, SolidDraft's sRGB.
fn material(m: &gltf::Material<'_>) -> Material {
    let pbr = m.pbr_metallic_roughness();
    let [r, g, b, a] = pbr.base_color_factor().map(f64::from);
    // Alpha is coverage only when the material blends; a mask cuts by texture, which is
    // not read, so it is shown opaque.
    let a = if m.alpha_mode() == gltf::material::AlphaMode::Blend { a } else { 1.0 };
    Material {
        name: m.name().map(str::to_owned).unwrap_or_else(|| format!("material {}", m.index().unwrap_or(0))),
        base_color: [srgb(r), srgb(g), srgb(b), a],
        metallic: f64::from(pbr.metallic_factor()),
        roughness: f64::from(pbr.roughness_factor()),
        textured: pbr.base_color_texture().is_some(),
    }
}

/// The meshes of each triangle primitive of `mesh` with its morph targets applied at
/// `weights`, with their materials.
fn read_mesh(
    scene: &mut Scene,
    mesh: &gltf::Mesh<'_>,
    weights: &[f32],
    buffers: &[Vec<u8>],
) -> Result<Vec<Primitive>, ImportError> {
    let name = mesh.name().map(str::to_owned).unwrap_or_else(|| format!("mesh {}", mesh.index()));
    let mut out = Vec::new();
    for primitive in mesh.primitives() {
        let reader = primitive.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
        let Some(positions) = reader.read_positions() else {
            scene.note("primitives with no positions are not imported");
            continue;
        };
        let mut positions: Vec<DVec3> = positions.map(|p| DVec3::from_array(p.map(f64::from))).collect();
        let mut normals: Option<Vec<DVec3>> =
            reader.read_normals().map(|n| n.map(|v| DVec3::from_array(v.map(f64::from))).collect());
        for (k, (dp, dn, _)) in reader.read_morph_targets().enumerate() {
            let w = f64::from(weights.get(k).copied().unwrap_or(0.0));
            if w == 0.0 {
                continue;
            }
            for (p, d) in positions.iter_mut().zip(dp.into_iter().flatten()) {
                *p += w * DVec3::from_array(d.map(f64::from));
            }
            if let Some(normals) = &mut normals {
                for (n, d) in normals.iter_mut().zip(dn.into_iter().flatten()) {
                    *n += w * DVec3::from_array(d.map(f64::from));
                }
            }
        }
        if reader.read_colors(0).is_some() {
            scene.note("vertex colours are not imported");
        }
        let indices: Vec<u32> = match reader.read_indices() {
            Some(i) => i.into_u32().collect(),
            None => (0..positions.len() as u32).collect(),
        };
        let triangles: Vec<[u32; 3]> = match primitive.mode() {
            Mode::Triangles => indices.as_chunks::<3>().0.to_vec(),
            Mode::TriangleStrip => (2..indices.len())
                .map(|i| {
                    if i % 2 == 0 {
                        [indices[i - 2], indices[i - 1], indices[i]]
                    } else {
                        [indices[i - 1], indices[i - 2], indices[i]]
                    }
                })
                .collect(),
            Mode::TriangleFan => (2..indices.len()).map(|i| [indices[0], indices[i - 1], indices[i]]).collect(),
            _ => {
                scene.note("points and lines are not imported");
                continue;
            }
        };
        if triangles.iter().flatten().any(|i| *i as usize >= positions.len()) {
            return Err(ImportError(format!("mesh {name} has an index past its points")));
        }
        let corner_normals = normals.map(|normals| {
            triangles
                .iter()
                .flat_map(|t| t.map(|i| normals.get(i as usize).copied().unwrap_or(DVec3::Z).normalize_or(DVec3::Z)))
                .collect()
        });
        let part =
            if mesh.primitives().len() == 1 { name.clone() } else { format!("{name} {}", primitive.index() + 1) };
        let m = scene.add_mesh(Mesh { name: part, positions, triangles, corner_normals });
        out.push((m, primitive.material().index()));
    }
    Ok(out)
}

/// The joint sand3d writes in a node's extras: `{"s3d_joint": {"name", "type", "axis",
/// "lower", "upper", "mimic"}}`, in metres and radians, its axis in the node's frame.
fn joint(scene: &mut Scene, extras: &str, parent: usize, node: usize, frame: DMat4) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(extras) else { return };
    let j = &value["s3d_joint"];
    if !j.is_object() {
        return;
    }
    let kind = match j["type"].as_str() {
        Some("revolute") => JointKind::Revolute,
        Some("continuous") => JointKind::Continuous,
        Some("prismatic") => JointKind::Prismatic,
        Some("fixed") => JointKind::Fixed,
        Some("floating") => JointKind::Floating,
        Some("planar") => JointKind::Planar,
        Some("spherical") => JointKind::Spherical,
        _ => JointKind::Other,
    };
    let axis = j["axis"]
        .as_array()
        .filter(|a| a.len() == 3)
        .map(|a| DVec3::new(a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0), a[2].as_f64().unwrap_or(0.0)))
        .unwrap_or(DVec3::Z)
        .normalize_or_zero();
    let turn = matches!(kind, JointKind::Revolute | JointKind::Continuous);
    let limits = match (j["lower"].as_f64(), j["upper"].as_f64()) {
        (Some(lo), Some(hi)) if turn => Some([lo.to_degrees(), hi.to_degrees()]),
        (Some(lo), Some(hi)) => Some([lo, hi]),
        _ => None,
    };
    let mimic = j["mimic"].as_object().and_then(|m| {
        Some(Mimic {
            joint: m.get("joint")?.as_str()?.to_owned(),
            multiplier: m.get("multiplier").and_then(serde_json::Value::as_f64).unwrap_or(1.0),
            offset: m
                .get("offset")
                .and_then(serde_json::Value::as_f64)
                .map(|o| if turn { o.to_degrees() } else { o })
                .unwrap_or(0.0),
        })
    });
    let name = j["name"].as_str().map(str::to_owned).unwrap_or_else(|| scene.nodes[node].name.clone());
    scene.joints.push(Joint { name, kind, parent, child: node, frame, axis, limits, mimic });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_escapes_decode() {
        assert_eq!(percent_decode("a%20b.bin"), "a b.bin");
        assert_eq!(percent_decode("plain.bin"), "plain.bin");
        assert_eq!(percent_decode("bad%2"), "bad%2");
    }
}
