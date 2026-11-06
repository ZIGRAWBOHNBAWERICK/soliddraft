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
