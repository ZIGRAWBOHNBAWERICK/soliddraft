//! Proves that removing the legacy 7-item top menu row (File/Edit/View/Insert/Tools/Window/Help)
//! leaves app commands reachable through command search in the workspace where they run.

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::commands;
use soliddraft_app::document::{Command, FeatureKind, FeatureStatus, Selection};
use soliddraft_app::drawing::{self, DrawingSel, DrawingTool};
use soliddraft_app::palette::Palette;
use soliddraft_app::state::{AppState, SketchSel, SketchSession};
use soliddraft_app::top_bar::{FILE_MENU_COMMANDS, TOP_BAR_COMMANDS};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_api::{Plane, Point2};
use soliddraft_sketch::{ConstraintKind, DistanceKind, EntityKind, P2, PointRef, Sketch};
use std::collections::BTreeSet;

/// Every command id that the removed 7-item `File / Edit / View / Insert / Tools / Window / Help`
/// menu bar reached before D1 Clear Bench.
const LEGACY_MENU_COMMANDS: &[&str] = &[
    // Legacy File
    "new_part",
    "open",
    "import_step",
    "import_iges",
    "import_mesh",
    "import_report",
    "import_scan",
    "scan_workspace",
    "make_drawing",
    "new_assembly",
    "make_assembly",
    "assembly_save",
    "assembly_save_as",
    "insert_component",
    "assembly_pack_and_go",
    "assembly_pack_and_go_zip",
    "assembly_export_urdf",
    "assembly_export_usd",
    "save",
    "save_as",
    "export_step",
    "export_iges",
    "export_stl",
    "export_3mf",
    "export_obj",
    "quit",
    // Legacy Edit
    "drawing_rebuild",
    "undo",
    "redo",
    "edit_feature",
    "rename",
    "suppress",
    "delete_feature",
    "roll_to_end",
    // Legacy View
    "view_isometric",
    "view_front",
    "view_top",
    "view_right",
    "view_back",
    "view_left",
    "view_bottom",
    "normal_to",
    "zoom_fit",
    "orthographic",
    "toggle_tree",
    // Legacy Tools
    "measure",
    "check",
    "ask",
    "check_model",
    "explain",
    "summarise",
    "find_issues",
    "provider_settings",
    "auto_apply",
    "file_locations",
    "settings",
    // Legacy Window
    "reset_layout",
    "show_drawing",
    "command_palette",
    // Legacy Help
    "shortcuts",
    "give_feedback",
    "about",
];

fn tool(state: &mut AppState, name: &str, args: Value) -> Value {
    let caller = Caller::Mcp { client: "command-reachability".into() };
    let out = call_checked(&mut AppTools { state }, name, args, &caller);
    assert!(!out.is_error, "{name}: {}", out.content);
    serde_json::from_str(&out.content).unwrap()
}

fn find_and_dispatch(state: &mut AppState, pending: &mut BTreeSet<&str>) {
    let _ = state.take_actions();
    pending.retain(|id| {
        let mut palette = Palette::default();
        palette.query = (*id).to_owned();
        let Some(found) = palette.results(state).into_iter().find(|result| result.command.id == *id) else {
            return true;
        };
        let command = found.command;
        if !command.is_working() {
            return false;
        }
        state.run_command(command);
        state.take_actions().is_empty()
    });
}

fn part_contexts(pending: &mut BTreeSet<&str>) {
    let mut state = AppState::new(soliddraft_app::command_tools::board_bracket());
    find_and_dispatch(&mut state, pending);
    state.doc.selection = Some(Selection::Body);
    find_and_dispatch(&mut state, pending);
    state.doc.selection = Some(Selection::Edge(state.doc.edges[0].id));
    find_and_dispatch(&mut state, pending);
    let feature = state.doc.features.last().unwrap().id;
    state.doc.selection = Some(Selection::Feature(feature));
    find_and_dispatch(&mut state, pending);

    let mut state = AppState::new(soliddraft_app::new_document());
    tool(
        &mut state,
        "sketch",
        json!({"plane":"top","shapes":[{"shape":"rectangle","center":[30,0,0],"width":4,"height":4}]}),
    );
    let sketch_id = state.doc.features[0].id;
    state.doc.selection = Some(Selection::Feature(sketch_id));
    find_and_dispatch(&mut state, pending);
    tool(&mut state, "extrude", json!({"sketch":"Sketch1","depth":4,"flip":false}));
    state.doc.new_body();
    tool(
        &mut state,
        "sketch",
        json!({"plane":"top","shapes":[{"shape":"rectangle","center":[0,0,0],"width":4,"height":4}]}),
    );
    tool(&mut state, "extrude", json!({"sketch":"Sketch2","depth":4,"flip":false}));
    find_and_dispatch(&mut state, pending);
    let boss = state.doc.features.last().unwrap().id;
    tool(
        &mut state,
        "sketch",
        json!({"plane":"Boss-Extrude2 · End cap","shapes":[{"shape":"rectangle","center":[0,0,4],"width":2,"height":2}]}),
    );
    state.doc.apply(Command::Suppress { id: boss, suppressed: true }).unwrap();
    let broken = state.doc.features.last().unwrap();
    assert!(matches!(broken.status, FeatureStatus::NeedsRepair(_)));
    state.doc.selection = Some(Selection::Feature(broken.id));
    find_and_dispatch(&mut state, pending);
    state.doc.undo().unwrap();
    find_and_dispatch(&mut state, pending);
    state.doc.rollback = Some(1);
    find_and_dispatch(&mut state, pending);

    let mut sketch = Sketch::new();
    let line = sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 5.0) }, false);
    let length = sketch.constrain(ConstraintKind::Distance {
        a: PointRef::new(line, 0),
        b: PointRef::new(line, 1),
        kind: DistanceKind::Aligned,
        value: 125.0_f64.sqrt(),
        label: P2::new(5.0, 8.0),
    });
    let mut edit = SketchSession::new(sketch_id, Plane::XY, sketch, false);
    edit.selection.push(SketchSel::Entity(line));
    state.sketch = Some(edit);
    find_and_dispatch(&mut state, pending);
    state.sketch.as_mut().unwrap().selection = vec![SketchSel::Constraint(length)];
    find_and_dispatch(&mut state, pending);
}

fn file_contexts(dir: &std::path::Path, pending: &mut BTreeSet<&str>) {
    let mut state = AppState::new(soliddraft_app::new_document());
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ci/fixtures/import");
    let source = fixtures.join("block-closed.step");
    let preview = tool(&mut state, "preview_import", json!({"path":source}));
    tool(&mut state, "import_step", json!({"path":source,"preview_token":preview["token"],"decline":[]}));
    state.doc.selection = Some(Selection::Feature(state.doc.features[0].id));
    find_and_dispatch(&mut state, pending);

    let mut state = AppState::new(soliddraft_app::new_document());
    let bytes = std::fs::read(fixtures.join("tetrahedron.obj")).unwrap();
    let source = soliddraft_kernel_api::scan::ScanSource {
        name: "tetrahedron.obj".into(),
        sha256: soliddraft_model::sha256_hex(&bytes),
        bytes,
    };
    state
        .doc
        .apply(Command::AddFeature(FeatureKind::Scan(soliddraft_model::ScanFeature { source, fit: None })))
        .unwrap();
    find_and_dispatch(&mut state, pending);

    let mut state = AppState::new(soliddraft_app::new_document());
    let rectangle = json!([{"shape":"rectangle","center":[0,0,20],"width":6,"height":6}]);
    tool(&mut state, "sketch", json!({"plane":"top","offset":20,"shapes":rectangle}));
    tool(&mut state, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    tool(&mut state, "lattice", json!({"cell":"cubic","cell_size":100,"radius":1,"accuracy":0.5,"min_detail":0.1}));
    find_and_dispatch(&mut state, pending);

    let mut state = AppState::new(soliddraft_app::command_tools::board_bracket());
    let part = dir.join("bracket.dwprt");
    tool(&mut state, "save", json!({"path":part}));
    tool(&mut state, "make_drawing", json!({"template":"iso-a3"}));
    let session = state.drawing.session.as_mut().unwrap();
    let child = session.doc.drawing.sheets[0].views.iter().find(|view| view.alignment.is_some()).unwrap();
    let child_id = child.id.0;
    session.selection = Some(DrawingSel::View(child.id));
    find_and_dispatch(&mut state, pending);
    tool(&mut state, "drawing_break_alignment", json!({"view":child_id}));
    let session = state.drawing.session.as_mut().unwrap();
    session.tool = DrawingTool::Detail;
    session.draft.profile = true;
    session.draft.points = vec![Point2::new(0.0, 0.0), Point2::new(4.0, 0.0), Point2::new(2.0, 3.0)];
    find_and_dispatch(&mut state, pending);
    let sheet = dir.join("bracket.dwdrw");
    tool(&mut state, "drawing_save", json!({"path":sheet}));
    tool(&mut state, "drawing_show_part", json!({}));
    find_and_dispatch(&mut state, pending);
    tool(&mut state, "make_assembly", json!({}));
    find_and_dispatch(&mut state, pending);
    tool(&mut state, "assembly_make_drawing", json!({}));
    find_and_dispatch(&mut state, pending);

    std::fs::remove_file(part).unwrap();
    let mut state = AppState::new(soliddraft_app::new_document());
    tool(&mut state, "open", json!({"path":sheet}));
    drawing::reference::refresh(&mut state);
    find_and_dispatch(&mut state, pending);
}

#[test]
fn naming_a_command_outside_its_workspace_or_with_a_disabled_action_does_not_reach_it() {
    let mut state = AppState::new(soliddraft_app::new_document());
    let mut pending = BTreeSet::from(["assembly_mate", "make_assembly"]);
    for id in &pending {
        let named = soliddraft_app::command_line::named(id, commands::Context::of(&state));
        assert_eq!(named.unwrap().id, *id);
    }
    find_and_dispatch(&mut state, &mut pending);
    assert_eq!(pending, BTreeSet::from(["assembly_mate", "make_assembly"]));
}

#[test]
fn every_app_command_is_reachable_without_legacy_menu_bar() {
    let dir = std::env::temp_dir()
        .join(format!("soliddraft-command-reachability-{}", soliddraft_document::DocumentId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    for id in FILE_MENU_COMMANDS.iter().chain(TOP_BAR_COMMANDS.iter()).chain(LEGACY_MENU_COMMANDS) {
        let command = commands::get(id).unwrap_or_else(|| panic!("missing app command {id}"));
        assert!(command.in_app(), "{id} must remain an app command");
    }
    let mut pending: BTreeSet<_> = commands::all().iter().filter(|c| c.in_app()).map(|c| c.id).collect();
    // The feedback form belongs to the web page and is covered in the browser.
    assert!(pending.remove("give_feedback"));
    part_contexts(&mut pending);
    file_contexts(&dir, &mut pending);
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
    assert!(pending.is_empty(), "commands never reached and dispatched: {pending:?}");
}
