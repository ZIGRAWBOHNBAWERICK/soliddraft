//! Modelling on an imported STEP body through the MCP tools: fillet, chamfer, boss, cut,
//! hole and shell each build on the imported block, keep its faces' `Import1 · Face N`
//! names, and survive undo, an edit of the later feature, save and reopen.

use serde_json::{Value, json};
use soliddraft_app::command_tools::{CommandTools, ProposalBook, Surface};
use soliddraft_app::document::{Document, FeatureStatus};
use soliddraft_assistant::{Caller, ToolHost};

/// A 40 x 25 x 10.004 mm block from the origin, with one 4 micrometre seam that the
/// import closes on its own.
const BLOCK: &[u8] = include_bytes!("../../../ci/fixtures/import/block-seam-4um.step");
const TOP_Z: f64 = 10.004;

fn call(doc: &mut Document, book: &mut ProposalBook, name: &str, input: Value) -> Result<Value, String> {
    let out = CommandTools { doc, book, surface: Surface::Mcp, drawing: None }.call(
        name,
        input,
        &Caller::Mcp { client: "imported body features".into() },
    );
    if out.is_error { Err(out.content) } else { Ok(serde_json::from_str(&out.content).unwrap_or(Value::Null)) }
}

struct Part {
    doc: Document,
    book: ProposalBook,
    dir: std::path::PathBuf,
}

impl Part {
    fn imported(case: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("soliddraft-import-features-{case}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("block.step");
        std::fs::write(&source, BLOCK).unwrap();
        let mut part =
            Self { doc: Document::new("Imported block", soliddraft_app::kernel()), book: <_>::default(), dir };
        let preview = part.call("preview_import", json!({"path":source})).unwrap();
        let input = json!({"path":source,"preview_token":preview["token"],"decline":[]});
        part.call("import_step", input).unwrap();
        part
    }

    fn call(&mut self, name: &str, input: Value) -> Result<Value, String> {
        call(&mut self.doc, &mut self.book, name, input)
    }

    fn ok(&mut self, name: &str, input: Value) {
        if let Err(e) = self.call(name, input.clone()) {
            panic!("{name} {input}: {e}");
        }
        for f in &self.doc.features {
            assert_eq!(f.status, FeatureStatus::Ok, "{name}: {} is {:?}", f.name, f.status);
        }
    }

    fn feature(&mut self, name: &str) -> Value {
        self.call("feature", json!({"name":name})).unwrap()
    }

    /// The imported faces left in the part, by name.
    fn imported_faces(&mut self) -> Vec<Value> {
        self.feature("Import1")["faces"].as_array().unwrap().clone()
    }

    /// The imported face whose outward normal is `normal`.
    fn face(&mut self, normal: [f64; 3]) -> String {
        let faces = self.imported_faces();
        let face = faces.iter().find(|f| f["normal"] == json!(normal)).unwrap_or_else(|| panic!("{faces:?}"));
        face["name"].as_str().unwrap().to_owned()
    }

    /// The imported edge between the faces with these outward normals.
    fn edge(&mut self, a: [f64; 3], b: [f64; 3]) -> String {
        let (a, b) = (self.face(a), self.face(b));
        let edges = self.feature("Import1")["edges"].as_array().unwrap().clone();
        let edge = edges
            .iter()
            .find(|e| {
                let faces = e["faces"].as_array().unwrap();
                faces.contains(&json!(a)) && faces.contains(&json!(b))
            })
            .unwrap_or_else(|| panic!("no edge between {a} and {b}: {edges:?}"));
        edge["name"].as_str().unwrap().to_owned()
    }

    fn volume(&mut self, feature: &str) -> f64 {
        self.call("mass_properties", json!({"name":feature})).unwrap()["volume_mm3"].as_f64().unwrap()
    }

    /// Every imported face is still named `Import1 · Face N`, and the names that remain
    /// are the ones the block had.
    fn assert_names_kept(&mut self, before: &[String]) {
        let names: Vec<String> = self.imported_faces().iter().map(|f| f["name"].as_str().unwrap().to_owned()).collect();
        assert!(!names.is_empty());
        for name in &names {
            assert!(name.starts_with("Import1 · Face "), "{names:?}");
            assert!(before.contains(name), "{name} is new: {before:?}");
        }
    }

    /// Saves, reopens in a fresh document and checks every feature still builds to the
    /// same volume.
    fn reopen(&mut self, last: &str) {
        let volume = self.volume(last);
        let saved = self.dir.join("part.dwprt");
        soliddraft_app::file::save(&mut self.doc, &saved).unwrap();
        self.doc = soliddraft_app::file::open(&saved, soliddraft_app::kernel(), None).unwrap();
        for f in &self.doc.features {
            assert_eq!(f.status, FeatureStatus::Ok, "reopened {} is {:?}", f.name, f.status);
        }
        assert!((self.volume(last) - volume).abs() < 1e-6, "{last} changed on reopen");
    }
}

impl Drop for Part {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const UP: [f64; 3] = [0.0, 0.0, 1.0];
const FRONT: [f64; 3] = [0.0, -1.0, 0.0];
const RIGHT: [f64; 3] = [1.0, 0.0, 0.0];
const BACK: [f64; 3] = [0.0, 1.0, 0.0];
const LEFT: [f64; 3] = [-1.0, 0.0, 0.0];

#[test]
fn fillet_and_chamfer_round_imported_edges_and_edit_undo_and_reopen() {
    let mut part = Part::imported("edges");
    let block = part.volume("Import1");
    let names: Vec<String> = part.imported_faces().iter().map(|f| f["name"].as_str().unwrap().to_owned()).collect();
    assert_eq!(names.len(), 6);

    let edge = part.edge(UP, FRONT);
    part.ok("fillet", json!({"edges":[edge],"radius":2}));
    let rounded = part.volume("Fillet1");
    assert!(rounded < block, "{rounded} >= {block}");
    part.assert_names_kept(&names);

    let edge = part.edge(UP, BACK);
    part.ok("chamfer", json!({"edges":[edge],"distance":1.5}));
    let chamfered = part.volume("Chamfer1");
    // A 1.5 mm chamfer along the 40 mm back edge removes 1.5 * 1.5 / 2 * 40 = 45 mm3.
    assert!((rounded - chamfered - 45.0).abs() < 0.01, "{rounded} - {chamfered}");
    part.assert_names_kept(&names);

    part.ok("edit_fillet", json!({"feature":"Fillet1","radius":3}));
    let larger = part.volume("Fillet1");
    assert!(larger < rounded, "a larger round removes more: {larger} vs {rounded}");
    part.doc.undo().unwrap();
    assert!((part.volume("Fillet1") - rounded).abs() < 0.01, "undo restores the 2 mm round");
    assert!((part.volume("Chamfer1") - chamfered).abs() < 0.01);
    part.reopen("Chamfer1");
    part.assert_names_kept(&names);
    part.ok("edit_chamfer", json!({"feature":"Chamfer1","distance":1}));
    // 1 * 1 / 2 * 40 = 20 mm3 off the rounded block.
    assert!((rounded - part.volume("Chamfer1") - 20.0).abs() < 0.01);
}

#[test]
fn boss_cut_and_hole_on_an_imported_face_build_and_reopen() {
    let mut part = Part::imported("solids");
    let block = part.volume("Import1");
    let names: Vec<String> = part.imported_faces().iter().map(|f| f["name"].as_str().unwrap().to_owned()).collect();
    let top = part.face(UP);

    let boss = json!({"shape":"circle","center":[10.0, 12.5, TOP_Z],"diameter":6});
    part.ok("sketch", json!({"plane":top,"shapes":[boss]}));
    part.ok("extrude", json!({"sketch":"Sketch1","depth":5,"flip":false}));
    let bossed = part.volume("Boss-Extrude1");
    assert!(bossed > block);
    part.assert_names_kept(&names);

    let slot = json!({"shape":"rectangle","center":[25.0, 12.5, TOP_Z],"width":8,"height":4});
    part.ok("sketch", json!({"plane":top,"shapes":[slot]}));
    part.ok("cut", json!({"sketch":"Sketch2","through_all":false,"depth":3,"flip":false}));
    let cut = part.volume("Cut-Extrude1");
    assert!((bossed - cut - 96.0).abs() < 0.01, "an 8 x 4 x 3 pocket: {bossed} - {cut}");
    part.assert_names_kept(&names);

    part.ok("hole", json!({"face":top,"points":[[35.0, 6.0, TOP_Z]],"diameter":3,"depth":4}));
    let drilled = part.volume("Hole1");
    assert!(drilled < cut);
    part.assert_names_kept(&names);

    part.ok("edit_cut", json!({"feature":"Cut-Extrude1","through_all":false,"depth":2,"flip":false}));
    assert!((part.volume("Cut-Extrude1") - (bossed - 64.0)).abs() < 0.01);
    part.doc.undo().unwrap();
    assert!((part.volume("Cut-Extrude1") - cut).abs() < 0.01, "undo restores the 3 mm pocket");
    assert!((part.volume("Hole1") - drilled).abs() < 0.01);
    part.reopen("Hole1");
    part.assert_names_kept(&names);
    part.ok("edit_hole", json!({"feature":"Hole1","diameter":2,"depth":4}));
    assert!(part.volume("Hole1") > drilled);
}

#[test]
fn shell_hollows_an_imported_block_through_its_top() {
    let mut part = Part::imported("shell");
    let names: Vec<String> = part.imported_faces().iter().map(|f| f["name"].as_str().unwrap().to_owned()).collect();
    let top = part.face(UP);
    part.ok("shell", json!({"open_faces":[top],"thickness":1}));
    let shelled = part.volume("Shell1");
    // 40 x 25 x 10.004 less a 38 x 23 x 9.004 hollow.
    let expected = 40.0 * 25.0 * TOP_Z - 38.0 * 23.0 * (TOP_Z - 1.0);
    assert!((shelled - expected).abs() < 0.01, "{shelled} vs {expected}");
    part.assert_names_kept(&names);
    for side in [FRONT, RIGHT, BACK, LEFT] {
        part.face(side);
    }
    part.ok("edit_shell", json!({"feature":"Shell1","thickness":2}));
    part.reopen("Shell1");
}

/// What else a user can ask of an imported body either builds or reads Coming soon and
/// leaves the part as it was. No answer names the import step or the kernel.
#[test]
fn other_features_on_an_imported_body_build_or_say_coming_soon() {
    let mut part = Part::imported("others");
    let right = part.face(RIGHT);
    let top = part.face(UP);
    for (tool, input) in [
        ("linear_pattern", json!({"direction":[0.0, 1.0, 0.0],"spacing":30,"count":2})),
        ("mirror", json!({"plane":right,"keep_original":true})),
        ("draft", json!({"faces":[right],"neutral":top,"angle":3,"reverse":false})),
        ("lattice", json!({"cell":"cubic","cell_size":5,"radius":1,"accuracy":0.1,"min_detail":0.5})),
        ("gyroid", json!({"kind":"network","period":10,"level":0,"accuracy":0.1,"min_detail":0.5})),
    ] {
        let depth = part.doc.undo_depth();
        match part.call(tool, input) {
            Ok(_) => {
                assert!(part.doc.features.iter().all(|f| f.status == FeatureStatus::Ok), "{tool}");
                println!("{tool}: built");
                part.doc.undo().unwrap();
            }
            Err(e) => {
                println!("{tool}: {e}");
                for internal in ["import_step", "Catallax", "kernel", "stage", "phase"] {
                    assert!(!e.contains(internal), "{tool}: {e}");
                }
                // The interface shows any "... is coming soon." as Coming soon alone.
                assert!(e.trim_end().ends_with("is coming soon."), "{tool}: {e}");
            }
        }
        assert_eq!(part.doc.undo_depth(), depth, "{tool} left the part changed");
    }
}
