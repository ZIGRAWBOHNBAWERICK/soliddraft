//! A refused export leaves its destination as it found it: the agent tools write only
//! after the kernel has answered, for a B-rep part's STEP and a field part's labelled STL.

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::{ToolOutput, call_checked};
use soliddraft_kernel_catallax::{APP_REQUEST_WORK, with_request_allowance};

static CONFIG: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn call(state: &mut AppState, name: &str, arguments: Value) -> ToolOutput {
    let caller = Caller::Mcp { client: "test".into() };
    call_checked(&mut AppTools { state }, name, arguments, &caller)
}

fn run(state: &mut AppState, name: &str, arguments: Value) {
    let out = call(state, name, arguments);
    assert!(!out.is_error, "{name}: {}", out.content);
}

/// Refuses `tool` on an exhausted allowance over an existing file, then lets it write.
fn assert_refused_then_written(state: &mut AppState, tool: &str, path: &std::path::Path) {
    let sentinel = b"the file from before";
    std::fs::write(path, sentinel).unwrap();
    let arguments = json!({ "path": path });
    let (refused, work) = with_request_allowance(0, || call(state, tool, arguments.clone()));
    assert!(refused.is_error && work.exhausted, "{tool}: {}", refused.content);
    assert!(refused.content.contains("numbers.operation-budget"), "{tool}: {}", refused.content);
    assert_eq!(std::fs::read(path).unwrap(), sentinel, "{tool} touched its destination");
    run(state, tool, arguments);
    assert_ne!(std::fs::read(path).unwrap(), sentinel, "{tool} wrote nothing");
}

#[test]
fn a_refused_export_leaves_its_destination_alone() {
    let _config = CONFIG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = std::env::temp_dir().join(format!("soliddraft-export-allowance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut state = AppState::new(soliddraft_app::new_document());
    let rectangle = json!([{ "shape": "rectangle", "center": [0, 0, 20], "width": 6, "height": 6 }]);
    run(&mut state, "sketch", json!({ "plane": "top", "offset": 20, "shapes": rectangle }));
    run(&mut state, "extrude", json!({ "sketch": "Sketch1", "depth": 10, "flip": false }));
    assert_refused_then_written(&mut state, "export_step", &dir.join("part.step"));
    assert_refused_then_written(&mut state, "export_iges", &dir.join("part.iges"));
    let lattice = json!({ "cell": "cubic", "cell_size": 100, "radius": 1, "accuracy": 0.5, "min_detail": 0.1 });
    run(&mut state, "lattice", lattice);
    assert_refused_then_written(&mut state, "export_stl", &dir.join("field.stl"));
    let path = dir.join("field.iges");
    std::fs::write(&path, b"previous IGES").unwrap();
    let refusal = call(&mut state, "export_iges", json!({ "path": path }));
    assert!(refusal.is_error && refusal.content.contains("field parts export as meshes"), "{}", refusal.content);
    assert_eq!(std::fs::read(path).unwrap(), b"previous IGES");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn smooth_rendering_cannot_satisfy_an_independent_certified_export() {
    let _config = CONFIG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(APP_REQUEST_WORK, 20_000_000);
    let dir = std::env::temp_dir().join(format!("soliddraft-smooth-export-{}", soliddraft_document::DocumentId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut state = AppState::new(soliddraft_app::new_document());
    for (size, z) in [(2, 0), (4, 3), (2, 6)] {
        run(
            &mut state,
            "sketch",
            json!({"plane":"top","offset":z,"shapes":[{"shape":"rectangle","center":[0,0,z],"width":size,"height":size}]}),
        );
    }
    run(&mut state, "loft", json!({"profiles":["Sketch1","Sketch2","Sketch3"],"shape":"smooth"}));
    let before = state.doc.model().to_bytes().unwrap();
    let undo = state.doc.undo_depth();
    let path = dir.join("smooth.stl");
    let sentinel = b"previous STL";
    std::fs::write(&path, sentinel).unwrap();
    let arguments = json!({"path":path});
    let (refused, work) = with_request_allowance(0, || call(&mut state, "export_stl", arguments.clone()));
    assert!(refused.is_error && work.exhausted, "{}; {work:?}", refused.content);
    assert!(refused.content.contains("numbers.operation-budget"), "{}", refused.content);
    assert_eq!(std::fs::read(&path).unwrap(), sentinel);
    let (refused, work) = with_request_allowance(APP_REQUEST_WORK, || call(&mut state, "export_stl", arguments));
    eprintln!("three-section Smooth certified export: {work:?}; {}", refused.content);
    assert!(refused.is_error && work.exhausted, "{}; {work:?}", refused.content);
    assert!(refused.content.contains("numbers.operation-budget"), "{}", refused.content);
    assert_eq!(std::fs::read(&path).unwrap(), sentinel, "a screen result cannot replace certified export");
    assert_eq!(state.doc.model().to_bytes().unwrap(), before);
    assert_eq!(state.doc.undo_depth(), undo);
    std::fs::remove_dir_all(dir).unwrap();
}
