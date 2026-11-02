//! The windowless Shell commands use the same model feature as the editor.

use serde_json::json;
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Command, Document, ExtrudeParams, FeatureKind, FeatureStatus, ProfileKind};
use soliddraft_app::state::AppState;
use soliddraft_assistant::{Caller, ToolOutput, tools::call_checked};
use soliddraft_exchange::Format;
use soliddraft_model::ShellOpenings;

fn blank() -> AppState {
    let mut doc = Document::new("tray", soliddraft_app::kernel());
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams {
            profile: ProfileKind::Rectangle { width: 40.0, height: 30.0 },
            depth: 12.0,
            ..Default::default()
        },
        via_assistant: false,
    })
    .unwrap();
    AppState::new(doc)
}

fn volume(state: &AppState) -> f64 {
    state.doc.kernel().mass_properties(state.doc.display_body().unwrap()).unwrap().volume
}

#[test]
fn shell_mcp_creates_edits_and_preserves_the_body_on_bad_requests() {
    let mut state = blank();
    let caller = Caller::Mcp { client: "shell-test".into() };
    let open_faces = json!(["Boss-Extrude1 · End cap"]);
    let out = call_checked(
        &mut AppTools { state: &mut state },
        "shell",
        json!({"open_faces":open_faces,"thickness":2}),
        &caller,
    );
    assert!(!out.is_error, "{}", out.content);
    assert!((volume(&state) - 5040.0).abs() < 1e-6);
    let FeatureKind::Shell(original) = state.doc.features.last().unwrap().kind.clone() else { panic!("shell feature") };
    let out = call_checked(
        &mut AppTools { state: &mut state },
        "edit_shell",
        json!({"feature":"Shell1","thickness":1.5}),
        &caller,
    );
    assert!(!out.is_error, "{}", out.content);
    assert!((volume(&state) - 3910.5).abs() < 1e-6);
    let FeatureKind::Shell(edited) = &state.doc.features.last().unwrap().kind else { panic!("shell feature") };
    assert_eq!(original.open_faces, edited.open_faces);
    let body = state.doc.display_body();
    let undo_depth = state.doc.undo_depth();
    let feature_count = state.doc.features.len();
    for (command, arguments) in [
        ("shell", json!({"open_faces":["missing face"],"thickness":2})),
        ("shell", json!({"open_faces":[],"thickness":0})),
        ("shell", json!({"thickness":2})),
        ("edit_shell", json!({"feature":"Shell1","thickness":-1})),
        ("edit_shell", json!({"feature":"Boss-Extrude1","thickness":2})),
        ("edit_shell", json!({"feature":"Shell1","thickness":"thin"})),
    ] {
        let out = call_checked(&mut AppTools { state: &mut state }, command, arguments, &caller);
        assert!(out.is_error, "{command} must refuse");
        assert_eq!(state.doc.display_body(), body, "{command} changed the body");
        assert_eq!(state.doc.undo_depth(), undo_depth);
        assert_eq!(state.doc.features.len(), feature_count);
    }
    state.doc.undo().unwrap();
    assert!((volume(&state) - 5040.0).abs() < 1e-6);
}

#[test]
fn shell_mcp_accepts_an_explicit_empty_opening_list_for_a_closed_shell() {
    let mut state = blank();
    let out = call_checked(
        &mut AppTools { state: &mut state },
        "shell",
        json!({"open_faces":[],"thickness":2}),
        &Caller::Mcp { client: "shell-test".into() },
    );
    assert!(!out.is_error, "{}", out.content);
    assert!((volume(&state) - (14400.0 - 36.0 * 26.0 * 8.0)).abs() < 1e-6);
}

fn call(state: &mut AppState, tool: &str, arguments: serde_json::Value) -> ToolOutput {
    call_checked(&mut AppTools { state }, tool, arguments, &Caller::Mcp { client: "shell-test".into() })
}

fn accepted(state: &mut AppState, tool: &str, arguments: serde_json::Value) {
    let out = call(state, tool, arguments);
    assert!(!out.is_error, "{tool}: {}", out.content);
}

fn shell(state: &AppState) -> ShellOpenings {
    let found = state.doc.features.iter().rev().find_map(|f| match &f.kind {
        FeatureKind::Shell(shell) => Some(shell.open_faces.clone()),
        _ => None,
    });
    found.expect("a shell feature")
}

fn saved_openings(state: &AppState) -> serde_json::Value {
    let part: serde_json::Value = serde_json::from_str(&state.doc.model().part_json()).unwrap();
    let features = part["features"].as_array().unwrap();
    features.iter().rev().find(|f| f["kind"]["type"] == "shell").unwrap()["kind"]["open_faces"].clone()
}

/// Stays a part with no body: a success, not a failed feature, and nothing to export.
fn nothing_left(state: &AppState) {
    assert_eq!(state.doc.features.last().unwrap().status, FeatureStatus::Ok);
    assert_eq!(state.doc.display_body(), None);
    let Err(refusal) = state.doc.model().export(Format::Step) else { panic!("no body to export") };
    assert_eq!(refusal, "the part has no body to export");
}

#[test]
fn all_faces_after_a_cut_and_a_fillet_stays_empty_through_edits_undo_save_and_reopen() {
    let mut state = blank();
    accepted(&mut state, "sketch", json!({"plane":"top","shapes":[{"shape":"circle","center":[0,0,0],"diameter":6}]}));
    accepted(&mut state, "cut", json!({"sketch":"Sketch1","depth":1,"through_all":true,"flip":false}));
    accepted(&mut state, "fillet", json!({"edges":["Boss-Extrude1 · Edge 1"],"radius":1}));
    let solid = volume(&state);
    let out =
        call(&mut state, "shell", json!({"open_faces":["Boss-Extrude1 · End cap"],"all_faces":true,"thickness":2}));
    assert!(out.is_error, "picked faces and every face are one choice or the other");
    accepted(&mut state, "shell", json!({"open_faces":[],"all_faces":true,"thickness":2}));
    nothing_left(&state);
    assert_eq!(shell(&state), ShellOpenings::All);
    assert_eq!(saved_openings(&state), json!([{}]));

    accepted(&mut state, "edit_shell", json!({"feature":"Shell1","thickness":1.5}));
    nothing_left(&state);
    assert_eq!(shell(&state), ShellOpenings::All, "editing the wall keeps the opening");
    accepted(&mut state, "edit_extrude", json!({"feature":"Boss-Extrude1","depth":20,"flip":false}));
    nothing_left(&state);

    state.doc.undo().unwrap();
    state.doc.undo().unwrap();
    state.doc.undo().unwrap();
    assert!((volume(&state) - solid).abs() < 1e-6, "undo brings the whole part back");
    state.doc.redo().unwrap();
    nothing_left(&state);

    let dir = std::env::temp_dir().join(format!("soliddraft-all-faces-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("nothing-left.dwprt");
    accepted(&mut state, "save", json!({"path":path}));
    accepted(&mut state, "open", json!({"path":path}));
    nothing_left(&state);
    assert_eq!(shell(&state), ShellOpenings::All);
    assert_eq!(saved_openings(&state), json!([{}]));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn closed_stays_empty_list_and_a_lost_pick_never_widens_to_every_face() {
    let mut state = blank();
    accepted(&mut state, "shell", json!({"open_faces":[],"thickness":2}));
    assert_eq!(saved_openings(&state), json!([]));
    assert_eq!(shell(&state), ShellOpenings::Faces(Vec::new()));
    state.doc.undo().unwrap();

    accepted(
        &mut state,
        "sketch",
        json!({"plane":"top","offset":12,"shapes":[{"shape":"rectangle","center":[0,0,12],"width":40,"height":30}]}),
    );
    accepted(&mut state, "extrude", json!({"sketch":"Sketch1","depth":4,"flip":false}));
    accepted(&mut state, "shell", json!({"open_faces":["Boss-Extrude2 · End cap"],"thickness":1}));
    assert_eq!(shell(&state).picked().len(), 1);
    accepted(&mut state, "delete_feature", json!({"feature":"Boss-Extrude2"}));
    assert_eq!(shell(&state).picked().len(), 1, "the picked reference stays as it was");
    let status = &state.doc.features.last().unwrap().status;
    assert!(!matches!(status, FeatureStatus::Ok), "a lost pick reports itself: {status:?}");
    assert!(state.doc.display_body().is_some(), "the part before the shell stays");
}
