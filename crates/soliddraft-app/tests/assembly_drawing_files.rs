//! Drawings of saved assemblies through the tools, as an agent or the CLI drives them:
//! opening one beside unsaved work, views at the revision the drawing shows, the bill of
//! materials of placements in one configuration, and the ids the configuration tools take.

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::command_tools::board_bracket;
use soliddraft_app::drawing::LinkState;
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use std::path::PathBuf;

struct Session {
    state: AppState,
    dir: PathBuf,
}

impl Session {
    fn new(name: &str) -> Session {
        let dir = std::env::temp_dir().join(format!("soliddraft-assembly-drawing-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Session { state: AppState::new(board_bracket()), dir }
    }

    fn try_call(&mut self, name: &str, input: Value) -> Result<Value, String> {
        let caller = Caller::Mcp { client: "test".into() };
        let out = call_checked(&mut AppTools { state: &mut self.state }, name, input, &caller);
        if out.is_error { Err(out.content) } else { Ok(serde_json::from_str(&out.content).unwrap()) }
    }

    fn call(&mut self, name: &str, input: Value) -> Value {
        self.try_call(name, input.clone()).unwrap_or_else(|e| panic!("{name} {input}: {e}"))
    }

    fn path(&self, name: &str) -> String {
        self.dir.join(name).to_string_lossy().into_owned()
    }

    /// A new part: a block `w` x 20 x 10, saved as `name` and left open.
    fn block(&mut self, name: &str, w: f64) -> String {
        self.call("assembly_show_part", json!({}));
        self.call("new_part", json!({}));
        let rectangle = json!({ "shape": "rectangle", "center": [0, 0, 0], "width": w, "height": 20 });
        self.call("sketch", json!({ "plane": "top", "shapes": [rectangle] }));
        self.call("extrude", json!({ "sketch": "Sketch1", "depth": 10, "flip": false }));
        let path = self.path(name);
        self.call("save", json!({ "path": path }));
        path
    }

    fn assembly(&self) -> &soliddraft_assembly::AssemblyDocument {
        &self.state.assembly.session.as_ref().expect("an assembly is open").doc
    }

    fn link(&mut self) -> LinkState {
        soliddraft_app::drawing::reference::refresh(&mut self.state);
        self.state.drawing.session.as_ref().expect("a drawing is open").link_state()
    }
}

/// A saved assembly of one block, `name`.dwasm, with its saved drawing `name`.dwdrw open.
fn drawn(s: &mut Session, name: &str) -> Value {
    s.block(&format!("{name}.dwprt"), 40.0);
    s.call("make_assembly", json!({}));
    s.call("assembly_save", json!({ "path": s.path(&format!("{name}.dwasm")) }));
    let drawn = s.call("assembly_make_drawing", json!({}));
    s.call("drawing_save", json!({ "path": s.path(&format!("{name}.dwdrw")) }));
    drawn
}

#[test]
fn opening_a_drawing_never_discards_unsaved_edits_to_another_assembly() {
    let mut s = Session::new("unsaved");
    drawn(&mut s, "a");
    let a = s.assembly().id();
    s.call("drawing_close", json!({}));

    // A second assembly, with an edit not saved yet.
    s.block("b.dwprt", 20.0);
    s.call("make_assembly", json!({}));
    s.call("assembly_save", json!({ "path": s.path("b.dwasm") }));
    s.call("assembly_set_value", json!({ "name": "gap", "expression": "5 mm" }));
    let b = s.assembly().id();
    assert_ne!(a, b);
    assert!(s.assembly().is_dirty());

    // Opening A's drawing does not open A over B's edit, and says why.
    let refused = s.try_call("open", json!({ "path": s.path("a.dwdrw") })).unwrap_err();
    assert!(refused.contains("unsaved"), "{refused}");
    assert_eq!(s.assembly().id(), b, "B stays open");
    assert!(s.assembly().is_dirty(), "with its edit");
    assert!(s.assembly().assembly().values.id_of("gap").is_some());
    // Rebuild, which opens the drawing's assembly first, holds back the same way.
    let refused = s.try_call("drawing_rebuild", json!({})).unwrap_err();
    assert!(refused.contains("unsaved"), "{refused}");
    assert_eq!(s.assembly().id(), b);
    assert!(s.assembly().is_dirty());

    // Once B is saved, Rebuild opens A and brings the drawing to it.
    s.call("assembly_save", json!({ "path": s.path("b.dwasm") }));
    s.call("drawing_rebuild", json!({}));
    assert_eq!(s.assembly().id(), a);
}

#[test]
fn a_drawing_of_an_assembly_that_shows_a_mesh_is_refused() {
    let mut s = Session::new("mesh");
    let block = s.block("block.dwprt", 40.0);
    let stl = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ci/fixtures/scene-import/sand3d-bracket/bracket.stl");
    s.call("import_scene", json!({ "path": stl, "out": s.path("scene.dwasm") }));
    s.call("insert_component", json!({ "paths": [block] }));
    s.call("assembly_save", json!({ "path": s.path("scene.dwasm") }));
    let refused = s.try_call("assembly_make_drawing", json!({})).unwrap_err();
    assert!(refused.contains("mesh"), "{refused}");
    assert!(s.state.drawing.session.is_none(), "no drawing leaves the mesh out");
}

#[test]
fn derived_views_wait_for_a_rebuild_when_the_assembly_moved_on() {
    let mut s = Session::new("revision");
    let made = drawn(&mut s, "rig");
    let front = made["drawing"]["sheets"][0]["views"][1]["id"].clone();
    assert_eq!(s.link(), LinkState::Current);

    // Another configuration, made active: the drawing still shows the first revision.
    s.call("show_assembly", json!({}));
    s.call("assembly_add_configuration", json!({ "name": "Wide" }));
    s.call("assembly_activate_configuration", json!({ "name": "Wide" }));
    let refused = s.try_call("drawing_projected", json!({ "view": front, "direction": "right" })).unwrap_err();
    assert!(refused.contains("Rebuild"), "{refused}");
    let at = s.call("model_summary", json!({}))["assembly"]["components"][0]["translation"].clone();
    let refused = s.try_call("drawing_detail", json!({ "view": front, "center": at, "radius": 10, "factor": 2 }));
    assert!(refused.unwrap_err().contains("Rebuild"));

    // Saved, the drawing sees a newer revision and still waits for its Rebuild.
    s.call("assembly_save", json!({ "path": s.path("rig.dwasm") }));
    assert!(matches!(s.link(), LinkState::Newer { .. }), "the saved revision is newer than the drawing's");
    assert!(s.try_call("drawing_projected", json!({ "view": front, "direction": "right" })).is_err());
    s.call("drawing_rebuild", json!({}));
    assert_eq!(s.link(), LinkState::Current);
    let placed = s.call("drawing_projected", json!({ "view": front, "direction": "right" }));
    assert!(placed["placed"].is_string(), "{placed}");
}
