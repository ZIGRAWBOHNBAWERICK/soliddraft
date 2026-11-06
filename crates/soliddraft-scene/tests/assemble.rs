//! An imported scene written as documents opens as an assembly with the file's tree,
//! names, placements, colours and joint record, from its display copies and, with those
//! gone, from the mesh reference files themselves.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glam::DVec3;
use soliddraft_assembly::{AssemblyDocument, AssemblyPackage, OccurrenceIndex, OpenOptions};
use soliddraft_document::DocKind;
use soliddraft_scene::{Options, assemble, read};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ci/fixtures/scene-import").join(rel)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-scene-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Every occurrence by its tree path of labels: kind and world box.
fn tree(doc: &AssemblyDocument) -> BTreeMap<String, (DocKind, [DVec3; 2])> {
    let table = doc.table();
    let mut out = BTreeMap::new();
    for row in (1..table.len() as u32).map(OccurrenceIndex) {
        let mut names = vec![doc.label(row)];
        let mut at = table.parent(row);
        while let Some(p) = at.filter(|p| *p != OccurrenceIndex::ROOT) {
            names.push(doc.label(p));
            at = table.parent(p);
        }
        names.reverse();
        let b = table.bounds(row);
        let kind = doc.store().get(table.definition(row)).kind;
        out.insert(names.join("/"), (kind, [b.min, b.max]));
    }
    out
}

fn near(a: DVec3, b: DVec3) -> bool {
    (a - b).abs().max_element() < 0.01
}

#[test]
fn the_washer_opens_as_an_assembly_of_mesh_references() {
    let dir = scratch("washer");
    let source = fixture("sand3d-washer/washer.urdf");
    let scene = read(&source, &Options::default()).unwrap();
    let out = dir.join("washer.dwasm");
    let written = assemble::write(&scene, &source, &out).unwrap();
    assert!(dir.join("washer.parts/door.dwasm").is_file());
    assert!(dir.join("washer.parts/glass.dwmsh").is_file());
    assert_eq!(written.files.last(), Some(&out));

    let mut doc = AssemblyDocument::open(&out, OpenOptions::default()).unwrap();
    doc.finish_loading();
    assert_eq!(doc.assembly().name, "washing_machine");
    let t = tree(&doc);
    assert_eq!(
        t.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "cabinet",
            "dial",
            "dial/dial",
            "door",
            "door/glass",
            "door/handle",
            "door/ring",
            "drawer",
            "drawer/drawer",
            "drum",
            "drum/drum",
        ]
    );
    assert_eq!(t["door"].0, DocKind::Assembly);
    assert_eq!(t["door/glass"].0, DocKind::Mesh);
    // The boxes are the scene's: placements and meshes carried the frames over.
    let scene_boxes: BTreeMap<String, [DVec3; 2]> = (0..scene.nodes.len())
        .filter(|i| scene.nodes[*i].body.is_some())
        .map(|i| {
            let w = scene.world(i);
            let pts: Vec<DVec3> = scene.meshes[scene.nodes[i].body.unwrap().mesh]
                .positions
                .iter()
                .map(|p| w.transform_point3(*p))
                .collect();
            let lo = pts.iter().fold(DVec3::INFINITY, |a, p| a.min(*p));
            let hi = pts.iter().fold(DVec3::NEG_INFINITY, |a, p| a.max(*p));
            (scene.path(i), [lo, hi])
        })
        .collect();
    for (path, [lo, hi]) in &scene_boxes {
        let [l, h] = t[path].1;
        assert!(near(*lo, l) && near(*hi, h), "{path}: {lo} {hi} vs {l} {h}");
    }
    let [lo, hi] = t["cabinet"].1;
    assert!(near(hi - lo, DVec3::new(600.0, 600.0, 850.0)), "{}", hi - lo);

    // The colour of the glass lives in the door's own assembly.
    let door = AssemblyPackage::open(dir.join("washer.parts/door.dwasm")).unwrap();
    let glass = door.assembly().snapshots.appearances.iter().find(|a| a.name.starts_with("glass")).unwrap();
    assert_eq!((glass.base_color.r, glass.base_color.g, glass.base_color.b), (0xcf, 0xe3, 0xe6));
    assert!((glass.opacity - 0.4).abs() < 1e-3);

    // The record keeps the joints in assembly terms, by occurrence path.
    let record: serde_json::Value = serde_json::from_str(doc.assembly().import.as_deref().unwrap()).unwrap();
    assert_eq!(record["format"], assemble::RECORD_FORMAT);
    assert_eq!(record["source"]["format"], "urdf");
    let hinge = record["joints"].as_array().unwrap().iter().find(|j| j["name"] == "door_hinge").unwrap();
    assert_eq!((hinge["kind"].as_str(), hinge["units"].as_str()), (Some("revolute"), Some("deg")));
    assert_eq!(hinge["parent"], "");
    let door_path =
        record["nodes"].as_array().unwrap().iter().find(|n| n["path"] == "door").unwrap()["occurrence"].clone();
    assert_eq!(hinge["child"], door_path);
    assert_eq!(hinge["limits"][1], 143.0);

    // Saving keeps the record; with no display copies the meshes load from their files.
    let bare = dir.join("bare.dwasm");
    let truth = AssemblyPackage::open(&out).unwrap().assembly().as_ref().clone();
    truth.save(&bare, soliddraft_document::DocumentId::new(), None, &[]).unwrap();
    let mut doc = AssemblyDocument::open(&bare, OpenOptions::default()).unwrap();
    doc.finish_loading();
    let again = tree(&doc);
    assert_eq!(again.keys().collect::<Vec<_>>(), t.keys().collect::<Vec<_>>());
    for (path, (_, [l, h])) in &again {
        assert!(near(*l, t[path].1[0]) && near(*h, t[path].1[1]), "{path} without copies");
    }
    assert!(doc.assembly().import.is_some(), "the record survives a save");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_bracket_from_usd_keeps_its_nested_tree() {
    // Written from USD, sand3d's tree nests the Geometry scope and the link.
    let dir = scratch("bracket");
    let source = fixture("sand3d-bracket/bracket.usda");
    let scene = read(&source, &Options::default()).unwrap();
    let out = dir.join("bracket.dwasm");
    let written = assemble::write(&scene, &source, &out).unwrap();
    let meshes = written.files.iter().filter(|f| f.extension().is_some_and(|e| e == "dwmsh")).count();
    assert_eq!(meshes, 1);
    let mut doc = AssemblyDocument::open(&out, OpenOptions::default()).unwrap();
    doc.finish_loading();
    let t = tree(&doc);
    let [lo, hi] = t["Geometry/plate/plate"].1;
    assert!(near(lo, DVec3::new(-40.0, -20.0, 0.0)) && near(hi, DVec3::new(40.0, 20.0, 6.0)), "{lo} {hi}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A USD stage with no units or up axis authored, two instances of one prototype, and a
/// scaled one.
const INSTANCED: &str = r#"#usda 1.0
(
    defaultPrim = "World"
)

def Xform "World"
{
    def Xform "A" (
        instanceable = true
        references = </Proto>
    )
    {
        double3 xformOp:translate = (10, 0, 0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }

    def Xform "B" (
        instanceable = true
        references = </Proto>
    )
    {
        double3 xformOp:translate = (-10, 0, 0)
        uniform token[] xformOpOrder = ["xformOp:translate"]
    }

    def Xform "C" (
        references = </Proto>
    )
    {
        float3 xformOp:scale = (2, 2, 2)
        uniform token[] xformOpOrder = ["xformOp:scale"]
    }
}

class "Proto"
{
    def Mesh "tri"
    {
        int[] faceVertexCounts = [3]
        int[] faceVertexIndices = [0, 1, 2]
        point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]
    }
}
"#;

#[test]
fn usd_fallbacks_instances_and_scale() {
    let dir = scratch("instanced");
    let source = dir.join("instanced.usda");
    std::fs::write(&source, INSTANCED).unwrap();
    let scene = read(&source, &Options::default()).unwrap();
    // Unauthored: USD's fallbacks of centimetres and Y up.
    assert_eq!((scene.meters_per_unit, scene.up_axis), (0.01, soliddraft_scene::UpAxis::Y));
    let paths: Vec<String> = (1..scene.nodes.len()).map(|i| scene.path(i)).collect();
    assert_eq!(paths, ["A", "A/tri", "B", "B/tri", "C", "C/tri"]);
    let out = dir.join("instanced.dwasm");
    let written = assemble::write(&scene, &source, &out).unwrap();
    // A and B share one mesh; C's is scaled into a mesh of its own.
    let meshes = written.files.iter().filter(|f| f.extension().is_some_and(|e| e == "dwmsh")).count();
    assert_eq!(meshes, 2);
    let mut doc = AssemblyDocument::open(&out, OpenOptions::default()).unwrap();
    doc.finish_loading();
    let t = tree(&doc);
    // 10 cm along X stays along X; 1 cm along Y (up) becomes 10 mm along Z.
    let [lo, hi] = t["A/tri"].1;
    assert!(near(lo, DVec3::new(100.0, 0.0, 0.0)) && near(hi, DVec3::new(110.0, 0.0, 10.0)), "{lo} {hi}");
    let [lo, hi] = t["C/tri"].1;
    assert!(near(lo, DVec3::ZERO) && near(hi, DVec3::new(20.0, 0.0, 20.0)), "{lo} {hi}");
    let record = &written.record;
    assert_eq!(record["units"]["from"], "fallback");
    let c = record["nodes"].as_array().unwrap().iter().find(|n| n["path"] == "C/tri").unwrap();
    assert_eq!(c["scale_baked"], serde_json::json!([2.0, 2.0, 2.0]));
    let _ = std::fs::remove_dir_all(dir);
}

/// A counter-clockwise quad notched at its second corner: area 3 mm², and (1, 0.25)
/// lies outside it.
const NOTCH_USD: &str = r#"#usda 1.0
(
    defaultPrim = "World"
    metersPerUnit = 0.001
    upAxis = "Z"
)
def Xform "World"
{
    def Mesh "NotchedQuad"
    {
        uniform token subdivisionScheme = "none"
        int[] faceVertexCounts = [4]
        int[] faceVertexIndices = [0, 1, 2, 3]
        point3f[] points = [(0, 0, 0), (1, 1, 0), (3, 0, 0), (0, 3, 0)]
    }
}
"#;

const NOTCH_OBJ: &str = "# notch, millimetres, Z up\no NotchedQuad\nv 0 0 0\nv 1 1 0\nv 3 0 0\nv 0 3 0\nf 1 2 3 4\n";

/// The same quad 10 km from the origin, at coordinates a binary32 holds exactly.
const FAR_NOTCH_OBJ: &str = "# far notch, millimetres, Z up\no NotchedQuad\nv 10000000 10000000 0\n\
    v 10000001 10000001 0\nv 10000003 10000000 0\nv 10000000 10000003 0\nf 1 2 3 4\n";

/// Each triangle's signed area about +Z, and whether any covers `p`.
fn coverage(scene: &soliddraft_scene::Scene, p: DVec3) -> (Vec<f64>, bool) {
    let m = &scene.meshes[0];
    let tris: Vec<[DVec3; 3]> = m.triangles.iter().map(|t| t.map(|i| m.positions[i as usize])).collect();
    let areas = tris.iter().map(|[a, b, c]| (*b - *a).cross(*c - *a).z / 2.0).collect();
    let inside = tris.iter().any(|[a, b, c]| {
        let s = |u: DVec3, v: DVec3| (v - u).cross(p - u).z;
        let (x, y, z) = (s(*a, *b), s(*b, *c), s(*c, *a));
        (x > 0.0 && y > 0.0 && z > 0.0) || (x < 0.0 && y < 0.0 && z < 0.0)
    });
    (areas, inside)
}

#[test]
fn concave_polygons_keep_their_notch_from_usd_and_obj() {
    let dir = scratch("notch");
    let far_usd = NOTCH_USD.replace(
        "[(0, 0, 0), (1, 1, 0), (3, 0, 0), (0, 3, 0)]",
        "[(10000000, 10000000, 0), (10000001, 10000001, 0), (10000003, 10000000, 0), (10000000, 10000003, 0)]",
    );
    let far = DVec3::new(1.0e7, 1.0e7, 0.0);
    for (name, text, at) in [
        ("notch.usda", NOTCH_USD, DVec3::ZERO),
        ("notch.obj", NOTCH_OBJ, DVec3::ZERO),
        ("far-notch.usda", far_usd.as_str(), far),
        ("far-notch.obj", FAR_NOTCH_OBJ, far),
    ] {
        let source = dir.join(name);
        std::fs::write(&source, text).unwrap();
        let scene = read(&source, &Options::default()).unwrap();
        let (areas, inside) = coverage(&scene, at + DVec3::new(1.0, 0.25, 0.0));
        assert_eq!(areas.len(), 2, "{name}");
        assert!(
            areas.iter().all(|a| (a - 1.5).abs() < 1e-9),
            "{name}: two counter-clockwise halves of 1.5 mm²: {areas:?}"
        );
        assert!(!inside, "{name}: the notch stays open");
        assert!(scene.notes.is_empty(), "{name}: {:?}", scene.notes);
        // Written and reopened, the mesh keeps the cut.
        let out = dir.join(format!("{name}.dwasm"));
        assemble::write(&scene, &source, &out).unwrap();
        let mut doc = AssemblyDocument::open(&out, OpenOptions::default()).unwrap();
        doc.finish_loading();
        let def = doc.store().iter().find(|(_, d)| d.kind == DocKind::Mesh).unwrap().1;
        let mesh = def.mesh.as_ref().expect("the mesh loaded");
        assert_eq!(mesh.triangle_count(), 2, "{name}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_default_prim_that_is_a_mesh_is_the_root_body() {
    let dir = scratch("root-mesh");
    let source = dir.join("tri.usda");
    std::fs::write(
        &source,
        "#usda 1.0\n(\n    defaultPrim = \"Tri\"\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n\
         def Mesh \"Tri\"\n{\n    int[] faceVertexCounts = [3]\n    int[] faceVertexIndices = [0, 1, 2]\n    \
         point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]\n}\n",
    )
    .unwrap();
    let scene = read(&source, &Options::default()).unwrap();
    assert_eq!(scene.nodes.len(), 1);
    assert!(scene.nodes[0].body.is_some());
    let out = dir.join("tri.dwasm");
    assemble::write(&scene, &source, &out).unwrap();
    let mut doc = AssemblyDocument::open(&out, OpenOptions::default()).unwrap();
    doc.finish_loading();
    let t = tree(&doc);
    assert_eq!(t.keys().map(String::as_str).collect::<Vec<_>>(), ["Tri"]);
    let [lo, hi] = t["Tri"].1;
    assert!(near(lo, DVec3::ZERO) && near(hi, DVec3::new(1.0, 1.0, 0.0)), "{lo} {hi}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A root moved 100 mm along X and turned a quarter turn about Z, a body under it, and
/// two revolute joints about X that move the body: `Anchored` names no body0, so its
/// frame is the stage's world frame; `Rooted` names the root as body0, so its frame is
/// the root's.
const ANCHORS_USD: &str = r#"#usda 1.0
(
    defaultPrim = "World"
    metersPerUnit = 0.001
    upAxis = "Z"
)
def Xform "World"
{
    double3 xformOp:translate = (100, 0, 0)
    float xformOp:rotateZ = 90
    uniform token[] xformOpOrder = ["xformOp:translate", "xformOp:rotateZ"]

    def Cube "Body"
    {
        double size = 10
    }

    def PhysicsRevoluteJoint "Anchored"
    {
        rel physics:body1 = </World/Body>
        point3f physics:localPos0 = (0, 0, 0)
        quatf physics:localRot0 = (1, 0, 0, 0)
        point3f physics:localPos1 = (0, 100, 0)
        quatf physics:localRot1 = (1, 0, 0, 0)
        uniform token physics:axis = "X"
    }

    def PhysicsRevoluteJoint "Rooted"
    {
        rel physics:body0 = </World>
        rel physics:body1 = </World/Body>
        point3f physics:localPos0 = (0, 0, 0)
        quatf physics:localRot0 = (1, 0, 0, 0)
        point3f physics:localPos1 = (0, 0, 0)
        quatf physics:localRot1 = (1, 0, 0, 0)
        uniform token physics:axis = "X"
    }
}
"#;

#[test]
fn a_joint_with_no_body0_is_anchored_to_the_world_not_the_root() {
    let dir = scratch("anchors");
    let source = dir.join("anchors.usda");
    std::fs::write(&source, ANCHORS_USD).unwrap();
    let scene = read(&source, &Options::default()).unwrap();
    let out = dir.join("anchors.dwasm");
    assemble::write(&scene, &source, &out).unwrap();
    let doc = AssemblyPackage::open(&out).unwrap();
    let record: serde_json::Value = serde_json::from_str(doc.assembly().import.as_deref().unwrap()).unwrap();
    let joint = |name: &str| {
        let j = record["joints"].as_array().unwrap().iter().find(|j| j["name"] == name).unwrap().clone();
        let v = |k: &str| DVec3::from_array([0, 1, 2].map(|i| j[k][i].as_f64().unwrap()));
        (v("origin"), v("axis"))
    };
    // The world anchor stays at the stage's origin, about the stage's X.
    let (origin, axis) = joint("Anchored");
    assert!(near(origin, DVec3::ZERO) && near(axis, DVec3::X), "Anchored: {origin} {axis}");
    // A joint on the root moves and turns with it.
    let (origin, axis) = joint("Rooted");
    assert!(near(origin, DVec3::new(100.0, 0.0, 0.0)) && near(axis, DVec3::Y), "Rooted: {origin} {axis}");
    let _ = std::fs::remove_dir_all(dir);
}

/// One triangle facing +Z, `orientation` and `indices` as given, with or without an
/// authored constant normal of +Z.
fn handed_triangle(orientation: &str, indices: &str, normals: bool) -> String {
    let normals = if normals {
        "    normal3f[] normals = [(0, 0, 1)] (\n        interpolation = \"constant\"\n    )\n"
    } else {
        ""
    };
    format!(
        "#usda 1.0\n(\n    defaultPrim = \"Tri\"\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n\
         def Mesh \"Tri\"\n{{\n    uniform token orientation = \"{orientation}\"\n    \
         uniform token subdivisionScheme = \"none\"\n    int[] faceVertexCounts = [3]\n    \
         int[] faceVertexIndices = {indices}\n    point3f[] points = [(0, 0, 0), (1, 0, 0), (0, 1, 0)]\n{normals}}}\n"
    )
}

#[test]
fn authored_normals_keep_their_direction_on_left_handed_meshes() {
    let dir = scratch("handed");
    for (name, orientation, indices, normals) in [
        ("right-authored", "rightHanded", "[0, 1, 2]", true),
        ("left-authored", "leftHanded", "[0, 2, 1]", true),
        ("left-derived", "leftHanded", "[0, 2, 1]", false),
    ] {
        let source = dir.join(format!("{name}.usda"));
        std::fs::write(&source, handed_triangle(orientation, indices, normals)).unwrap();
        let scene = read(&source, &Options::default()).unwrap();
        let m = &scene.meshes[0];
        let [a, b, c] = m.triangles[0].map(|i| m.positions[i as usize]);
        assert!((b - a).cross(c - a).normalize().abs_diff_eq(DVec3::Z, 1e-12), "{name}: the triangle faces +Z");
        if let Some(ns) = &m.corner_normals {
            assert!(ns.iter().all(|n| n.abs_diff_eq(DVec3::Z, 1e-12)), "{name}: authored normals {ns:?}");
        }
        let shown = assemble::display_mesh(m, &glam::DMat4::IDENTITY).unwrap();
        assert!(shown.normals.iter().all(|n| *n == [0.0, 0.0, 1.0]), "{name}: shown normals {:?}", shown.normals);
    }
    let _ = std::fs::remove_dir_all(dir);
}
