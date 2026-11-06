//! What the glTF, OBJ, STL and URDF readers do with the parts of their formats beyond
//! plain meshes: each is read, or left out with a note in the import record, never read
//! wrong without a word. Every case has a control that reads cleanly.

use std::path::{Path, PathBuf};

use base64::Engine;
use glam::DVec3;
use serde_json::{Value, json};
use soliddraft_scene::{Options, Scene, read};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-readers-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn noted(scene: &Scene, what: &str) -> bool {
    scene.notes.iter().any(|n| n.contains(what))
}

/// The X extent of the node named `name`'s body in the scene's root frame, in mm.
fn x_extent(scene: &Scene, name: &str) -> [f64; 2] {
    let i = scene.nodes.iter().position(|n| n.name == name).unwrap_or_else(|| panic!("no node {name}"));
    let mesh = &scene.meshes[scene.nodes[i].body.unwrap().mesh];
    let w = scene.world(i);
    mesh.positions
        .iter()
        .map(|p| w.transform_point3(*p).x)
        .fold([f64::MAX, f64::MIN], |[lo, hi], x| [lo.min(x), hi.max(x)])
}

/// Little-endian f32 bytes of `values`.
fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A glTF of one triangle (0,0,0), (1,0,0), (0,1,0) in metres on one node named `Tri`,
/// with `extra` buffer data appended after the 36 bytes of positions; `edit` changes the
/// JSON.
fn gltf(extra: &[u8], edit: impl FnOnce(&mut Value)) -> String {
    let mut data = floats(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    data.extend_from_slice(extra);
    let uri =
        format!("data:application/octet-stream;base64,{}", base64::engine::general_purpose::STANDARD.encode(&data));
    let mut v = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "Tri", "mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 4}]}],
        "buffers": [{"uri": uri, "byteLength": data.len()}],
        "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                       "min": [0, 0, 0], "max": [1, 1, 0]}],
    });
    edit(&mut v);
    v.to_string()
}

/// Adds a VEC3 float accessor over `extra` bytes at `offset` after the positions, and
/// returns its index.
fn accessor(v: &mut Value, offset: usize, count: usize) -> usize {
    let views = v["bufferViews"].as_array_mut().unwrap();
    views.push(json!({"buffer": 0, "byteOffset": 36 + offset, "byteLength": count * 12}));
    let view = views.len() - 1;
    let accessors = v["accessors"].as_array_mut().unwrap();
    accessors.push(json!({"bufferView": view, "componentType": 5126, "count": count, "type": "VEC3"}));
    accessors.len() - 1
}

#[test]
fn gltf_morph_targets_take_their_node_or_mesh_weights() {
    let dir = scratch("morph");
    // The reviewer's witness: one target moves every corner 10 m along X; the mesh's
    // default weight is 1, and one of its two nodes overrides it with 0.
    let shift = floats(&[10.0, 0.0, 0.0, 10.0, 0.0, 0.0, 10.0, 0.0, 0.0]);
    let text = gltf(&shift, |v| {
        let target = accessor(v, 0, 3);
        v["meshes"][0]["weights"] = json!([1.0]);
        v["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION": target}]);
        v["nodes"] = json!([{"name": "ZeroOverride", "mesh": 0, "weights": [0.0]}, {"name": "DefaultOne", "mesh": 0}]);
        v["scenes"][0]["nodes"] = json!([0, 1]);
    });
    let scene = read(&write(&dir, "static-morph.gltf", text), &Options::default()).unwrap();
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6;
    assert!(near(x_extent(&scene, "ZeroOverride"), [0.0, 1000.0]), "{:?}", x_extent(&scene, "ZeroOverride"));
    assert!(near(x_extent(&scene, "DefaultOne"), [10000.0, 11000.0]), "{:?}", x_extent(&scene, "DefaultOne"));
    assert!(scene.notes.is_empty(), "{:?}", scene.notes);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn gltf_alpha_counts_only_for_blended_materials() {
    let dir = scratch("alpha");
    for (mode, alpha) in [(None, 1.0), (Some("OPAQUE"), 1.0), (Some("MASK"), 1.0), (Some("BLEND"), 0.5)] {
        let text = gltf(&[], |v| {
            let mut m = json!({"name": "half", "pbrMetallicRoughness": {"baseColorFactor": [1.0, 1.0, 1.0, 0.5]}});
            if let Some(mode) = mode {
                m["alphaMode"] = json!(mode);
            }
            v["materials"] = json!([m]);
            v["meshes"][0]["primitives"][0]["material"] = json!(0);
        });
        let scene = read(&write(&dir, "alpha.gltf", text), &Options::default()).unwrap();
        assert_eq!(scene.materials[0].base_color[3], alpha, "{mode:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn gltf_features_it_cannot_read_are_refused_or_noted() {
    let dir = scratch("gltf-features");
    // The control reads with no note.
    let scene = read(&write(&dir, "plain.gltf", gltf(&[], |_| {})), &Options::default()).unwrap();
    assert!(scene.notes.is_empty(), "{:?}", scene.notes);
    // An extension the file requires and this reader does not know, such as compressed
    // or quantised geometry, refuses the file by name.
    for ext in
        ["KHR_draco_mesh_compression", "EXT_meshopt_compression", "KHR_mesh_quantization", "KHR_texture_transform"]
    {
        let text = gltf(&[], |v| {
            v["extensionsUsed"] = json!([ext]);
            v["extensionsRequired"] = json!([ext]);
        });
        let err = read(&write(&dir, "required.gltf", text), &Options::default()).unwrap_err();
        assert!(err.to_string().contains(ext), "{err}");
    }
    // GPU instancing, vertex colours and further scenes are noted.
    let colours = floats(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let text = gltf(&colours, |v| {
        let c = accessor(v, 0, 3);
        v["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(c);
        v["extensionsUsed"] = json!(["EXT_mesh_gpu_instancing"]);
        v["scenes"] = json!([{"nodes": [0]}, {"nodes": [0]}]);
    });
    let scene = read(&write(&dir, "noted.gltf", text), &Options::default()).unwrap();
    for what in ["vertex colours", "EXT_mesh_gpu_instancing", "default scene"] {
        assert!(noted(&scene, what), "{what}: {:?}", scene.notes);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn obj_lines_points_free_form_and_missing_materials_are_noted() {
    let dir = scratch("obj");
    let tri = "# millimetres, Z up\no Tri\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
    let scene = read(&write(&dir, "plain.obj", tri), &Options::default()).unwrap();
    assert!(scene.notes.is_empty(), "{:?}", scene.notes);
    let text =
        format!("{tri}l 1 2\np 3\nmtllib missing.mtl\nusemtl red\ncstype bspline\ndeg 3\ncurv 0 1 1 2 3 3\nend\n");
    let scene = read(&write(&dir, "extras.obj", text), &Options::default()).unwrap();
    assert_eq!(scene.meshes[0].triangles.len(), 1);
    for what in ["lines and points", "free-form", "missing.mtl"] {
        assert!(noted(&scene, what), "{what}: {:?}", scene.notes);
    }
    let coloured = "o Tri\nv 0 0 0 1 0 0\nv 1 0 0 1 0 0\nv 0 1 0 1 0 0\nf 1 2 3\n";
    let scene = read(&write(&dir, "coloured.obj", coloured), &Options::default()).unwrap();
    assert!(noted(&scene, "vertex colours"), "{:?}", scene.notes);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stl_solids_normals_and_colours() {
    let dir = scratch("stl");
    let facet = |n: &str, z: f64| {
        format!(
            "  facet normal {n}\n    outer loop\n      vertex 0 0 {z}\n      vertex 1 0 {z}\n      vertex 0 1 {z}\n    \
             endloop\n  endfacet\n"
        )
    };
    // One solid, normals that agree with the winding: no note.
    let one = format!("solid a\n{}endsolid a\n", facet("0 0 1", 0.0));
    let scene = read(&write(&dir, "one.stl", one), &Options::default()).unwrap();
    assert!(scene.notes.is_empty(), "{:?}", scene.notes);
    // Two solids in one ASCII file: both come in.
    let two = format!("solid a\n{}endsolid a\nsolid b\n{}endsolid b\n", facet("0 0 1", 0.0), facet("0 0 1", 5.0));
    let scene = read(&write(&dir, "two.stl", two), &Options::default()).unwrap();
    let triangles: usize = scene.meshes.iter().map(|m| m.triangles.len()).sum();
    assert_eq!(triangles, 2, "{:?}", scene.nodes.iter().map(|n| &n.name).collect::<Vec<_>>());
    // A facet whose normal disagrees with its winding keeps the winding, said so.
    let flipped = format!("solid a\n{}endsolid a\n", facet("0 0 -1", 0.0));
    let scene = read(&write(&dir, "flipped.stl", flipped), &Options::default()).unwrap();
    assert!(noted(&scene, "winding"), "{:?}", scene.notes);
    // Colours in a binary file's attribute bytes are noted.
    let mut binary = vec![0u8; 80];
    binary.extend_from_slice(&1u32.to_le_bytes());
    binary.extend(floats(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]));
    binary.extend_from_slice(&0x801fu16.to_le_bytes());
    let scene = read(&write(&dir, "colour.stl", &binary), &Options::default()).unwrap();
    assert!(noted(&scene, "colour"), "{:?}", scene.notes);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn urdf_meshes_it_cannot_read_are_left_out_not_fatal() {
    let dir = scratch("urdf");
    write(
        &dir,
        "base.stl",
        "solid base\n  facet normal 0 0 1\n    outer loop\n      vertex 0 0 0\n      vertex 1 0 0\n      vertex 0 1 0\n    endloop\n  endfacet\nendsolid base\n",
    );
    write(&dir, "arm.dae", "<COLLADA/>");
    let text = r#"<robot name="bot">
  <link name="base"><visual><geometry><mesh filename="base.stl"/></geometry></visual></link>
  <link name="arm"><visual><geometry><mesh filename="arm.dae"/></geometry></visual></link>
  <joint name="j" type="fixed"><parent link="base"/><child link="arm"/></joint>
</robot>"#;
    let scene = read(&write(&dir, "bot.urdf", text), &Options::default()).unwrap();
    assert!(scene.nodes.iter().any(|n| n.name == "base" && n.body.is_some()));
    assert!(noted(&scene, "arm.dae"), "{:?}", scene.notes);
    let _ = std::fs::remove_dir_all(dir);
    let _ = DVec3::ZERO;
}
