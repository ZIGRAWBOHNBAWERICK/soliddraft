//! V3 increment 2, from the independent acceptance report: G-REFUSE, G-TOKEN and the title
//! stacks run through the production tools. The Values tab that replaced the window is
//! driven in `ux_values_tab.rs`.
//!
//! The files in `fixtures/configuration_grid` come from `ci/check-configuration-grid.py
//! fixtures`, never from this application.

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::Document;
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use soliddraft_drawing::display::{Item, layout_sheet};
use std::path::{Path, PathBuf};

const FAMILY: &str = "Bracket, \"family\"";
const WIDE: &str = "Bracket, \"wide\"";
const WIDE_ROW: &str = "Wide, \"shop\"";
const WIDTH: &str = "value:plate_w [mm]";
const TITLE: &str = "property:title [text]";
/// The report's expected volume of Narrow at 130 mm, digit for digit.
#[allow(clippy::excessive_precision)]
const VOLUME_130: f64 = 165369.02664470769;

fn fixture(name: &str) -> String {
    format!("{}/tests/fixtures/configuration_grid/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-grid-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn tool(state: &mut AppState, name: &str, arguments: Value) -> Result<Value, String> {
    let caller = Caller::Mcp { client: "test".into() };
    let out = call_checked(&mut AppTools { state }, name, arguments, &caller);
    if out.is_error { Err(out.content) } else { Ok(serde_json::from_str(&out.content).unwrap_or(Value::Null)) }
}

fn call(state: &mut AppState, name: &str, arguments: Value) -> Value {
    tool(state, name, arguments).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn parameter_dependent_grid(name: &str, default: &str, expression: &str) -> AppState {
    let model = soliddraft_model::Document::from_bytes(
        include_bytes!("../../soliddraft-model/tests/fixtures/bracket-140.dwprt"),
        soliddraft_app::kernel(),
    )
    .unwrap();
    let mut state = AppState::new(Document::from_model(model));
    call(&mut state, "set_value", json!({ "name": name, "expression": default }));
    call(&mut state, "add_configuration", json!({ "name": "Thin" }));
    call(
        &mut state,
        "set_configuration_value",
        json!({ "configuration": "Thin", "name": "BossExtrude1.Depth", "expression": "20 mm" }),
    );
    call(
        &mut state,
        "set_configuration_value",
        json!({ "configuration": "Thin", "name": name, "expression": expression }),
    );
    state
}

fn resolved_thin(report: &Value, name: &str) -> Value {
    report["resolved"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "Thin")
        .and_then(|row| row["values"].as_array())
        .and_then(|values| values.iter().find(|v| v["name"] == name))
        .map(|v| v["value"].clone())
        .unwrap_or(Value::Null)
}

#[test]
fn grid_retained_parameter_overrides_do_not_block_values_tab_text_property_edits() {
    use soliddraft_app::values_tab::{self, Field, ValuesAction};
    let mut state = parameter_dependent_grid("Ratio", "1 mm / 1 mm", "10 mm / (BossExtrude1.Depth - 10 mm)");
    let field = Field::Property("note".into(), 0);
    let undo = state.doc.undo_depth();
    values_tab::apply(&mut state, ValuesAction::Commit(field.clone(), "Stock item".into()));
    assert!(state.values.error(&field).is_none(), "{:?}", state.values.error(&field));
    assert_eq!(state.doc.model().configuration_grid().rows[0].properties["note"], "Stock item");
    assert_eq!(state.doc.undo_depth(), undo + 1);
    let read = call(&mut state, "configuration_grid", json!({}));
    assert_eq!(resolved_thin(&read, "Ratio"), json!(1.0));
}

#[test]
fn grid_retained_parameter_overrides_survive_unchanged_csv_and_xlsx_imports() {
    let mut state = parameter_dependent_grid("Ratio", "1 mm / 1 mm", "10 mm / (BossExtrude1.Depth - 10 mm)");
    let dir = scratch("retained-parameter-import");
    for format in ["csv", "xlsx"] {
        let file = dir.join(format!("unchanged.{format}"));
        let before = state.doc.model().configuration_grid();
        call(&mut state, "export_configuration_grid", json!({ "path": file, "format": format }));
        let preview = call(&mut state, "preview_configuration_grid", json!({ "path": file, "format": format }));
        assert_eq!(resolved_thin(&preview, "Ratio"), json!(1.0));
        apply(&mut state, preview["preview_id"].as_str().unwrap()).unwrap();
        assert_eq!(state.doc.model().configuration_grid(), before);
        let read = call(&mut state, "configuration_grid", json!({}));
        assert_eq!(resolved_thin(&read, "Ratio"), json!(1.0));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn grid_retained_parameter_overrides_are_used_by_live_and_preview_readouts() {
    let mut state = parameter_dependent_grid("Twice", "20 mm", "BossExtrude1.Depth * 2");
    let live = call(&mut state, "configuration_grid", json!({}));
    assert_eq!(resolved_thin(&live, "Twice"), json!(40.0));
    let dir = scratch("retained-parameter-read");
    let file = dir.join("unchanged.csv");
    call(&mut state, "export_configuration_grid", json!({ "path": file, "format": "csv" }));
    let preview = call(&mut state, "preview_configuration_grid", json!({ "path": file, "format": "csv" }));
    assert_eq!(resolved_thin(&preview, "Twice"), json!(40.0));
    assert_eq!(live["resolved"], preview["resolved"]);
    std::fs::remove_dir_all(dir).unwrap();
}

fn preview_file(state: &mut AppState, name: &str) -> Result<Value, String> {
    let format = name.rsplit('.').next().unwrap();
    tool(state, "preview_configuration_grid", json!({ "path": fixture(name), "format": format }))
}

fn preview_cells(state: &mut AppState, rows: Value) -> Value {
    let columns = call(state, "configuration_grid", json!({}))["columns"].clone();
    let arguments = json!({ "columns": columns, "rows": rows });
    call(state, "preview_configuration_grid", arguments)
}

fn apply(state: &mut AppState, id: &str) -> Result<Value, String> {
    tool(state, "apply_configuration_grid", json!({ "preview_id": id }))
}

/// The scenario's bracket with Default, Narrow and the quoted row, without text properties.
fn bracket(state: &mut AppState) {
    let rectangle =
        |x: f64, width: f64| json!({ "shape": "rectangle", "center": [x, 45, 0], "width": width, "height": 90 });
    call(state, "sketch", json!({ "plane": "top", "shapes": [rectangle(60.0, 140.0)] }));
    call(state, "extrude", json!({ "sketch": "Sketch1", "depth": 10, "flip": false }));
    call(state, "sketch", json!({ "plane": "top", "shapes": [rectangle(5.0, 10.0)] }));
    call(state, "extrude", json!({ "sketch": "Sketch2", "depth": 70, "flip": false }));
    let hole = json!({ "shape": "circle", "center": [70, 45, 10], "diameter": 12 });
    call(state, "sketch", json!({ "plane": "Boss-Extrude1 · End cap", "shapes": [hole] }));
    let cut = json!({ "sketch": "Sketch3", "through_all": true, "depth": 1, "flip": false });
    call(state, "cut", cut);
    for (name, expression) in [("plate_w", "140"), ("flange_d", "plate_w / 2"), ("hole_d", "12")] {
        call(state, "set_value", json!({ "name": name, "expression": expression }));
    }
    call(state, "drive", json!({ "sketch": "Sketch1", "dimension": 1, "expression": "plate_w" }));
    call(state, "drive", json!({ "feature": "Boss-Extrude2", "parameter": "depth", "expression": "flange_d" }));
    call(state, "drive", json!({ "sketch": "Sketch3", "dimension": 1, "expression": "hole_d" }));
    for (row, width) in [("Narrow", "120"), (WIDE_ROW, "6 in")] {
        call(state, "add_configuration", json!({ "name": row }));
        call(state, "set_configuration_value", json!({ "configuration": row, "name": "plate_w", "expression": width }));
    }
}

fn g0_rows() -> Value {
    json!([
        ["Default", "140", "plate_w / 2", "12", FAMILY],
        ["Narrow", "120", "", "", ""],
        [WIDE_ROW, "152.4 mm", "", "", WIDE]
    ])
}

/// G0 with Narrow selected.
fn g0(state: &mut AppState) {
    bracket(state);
    let columns = json!(["configuration", WIDTH, "value:flange_d [mm]", "value:hole_d [mm]", TITLE]);
    let arguments = json!({ "columns": columns, "rows": g0_rows() });
    let id = call(state, "preview_configuration_grid", arguments)["preview_id"].as_str().unwrap().to_owned();
    apply(state, &id).unwrap();
    call(state, "activate_configuration", json!({ "name": "Narrow" }));
}

/// Everything a refused or cancelled table must leave alone. Previews, diagnostics and
/// evidence files are outside it.
#[derive(Debug, PartialEq)]
struct Accepted {
    journal: Value,
    revision: String,
    geometry: String,
    statuses: Vec<String>,
    active: Option<String>,
    resolved: Value,
    grid: Value,
    dirty: bool,
    mesh: u64,
    displayed: u64,
    triangles: usize,
    saved: Option<Vec<u8>>,
}

fn accepted(state: &mut AppState, saved: Option<&Path>) -> Accepted {
    let doc = &state.doc;
    let model = doc.model();
    let statuses = doc.features.iter().map(|f| format!("{}: {:?}", f.name, f.status)).collect();
    let result = Accepted {
        journal: model.journal_state(),
        revision: model.semantic_hash().to_string(),
        geometry: model.geometry_hash().to_string(),
        statuses,
        active: model.active_configuration().map(|id| id.0.clone()),
        resolved: model.values().summary(),
        grid: serde_json::to_value(model.configuration_grid()).unwrap(),
        dirty: doc.dirty,
        mesh: doc.mesh_version,
        displayed: doc.displayed_revision(),
        triangles: doc.triangle_count(),
        saved: saved.map(|path| std::fs::read(path).unwrap()),
    };
    assert!(!doc.proof_pending());
    result
}

fn volume(state: &mut AppState) -> f64 {
    let doc = &state.doc;
    let body = doc.model().body().unwrap();
    doc.kernel().mass_properties(body).unwrap().volume
}

#[test]
fn g_refuse_every_bad_file_changes_nothing_and_a_good_one_still_imports() {
    let dir = scratch("refuse");
    let mut state = AppState::new(Document::new("Part1", soliddraft_app::kernel()));
    g0(&mut state);
    // Nonempty undo and redo history and a saved file.
    call(&mut state, "activate_configuration", json!({ "name": "Default" }));
    call(&mut state, "undo", json!({}));
    let saved = dir.join("grid-base.dwprt");
    call(&mut state, "save", json!({ "path": saved }));
    assert!(state.doc.undo_depth() > 0 && state.doc.redo_depth() > 0);
    let before = accepted(&mut state, Some(&saved));

    let mut cases: Vec<(String, Vec<&str>)> = vec![
        ("short-record.csv".into(), vec!["short-record.csv", "CSV row 4 has 4 fields"]),
        ("broken.xlsx".into(), vec!["broken.xlsx", "XLSX workbook cannot be read"]),
        ("formula.xlsx".into(), vec!["Configurations!B3", "Narrow, plate_w", "formula"]),
    ];
    for extension in ["csv", "xlsx"] {
        let wrong_unit = vec![WIDE_ROW, "plate_w", "expected length (mm)"];
        cases.push((format!("bad-unit.{extension}"), wrong_unit));
        cases.push((format!("duplicate-row.{extension}"), vec!["duplicate configuration Narrow"]));
        cases.push((format!("duplicate-header.{extension}"), vec!["duplicate column plate_w"]));
        cases.push((format!("unknown-value.{extension}"), vec!["unknown named value missing_w"]));
        cases.push((format!("cycle.{extension}"), vec![WIDE_ROW, "cycle"]));
        cases.push((format!("empty-active.{extension}"), vec!["Narrow", "Cut-Extrude1", "leave nothing of the part"]));
    }
    for (file, words) in &cases {
        let error = match preview_file(&mut state, file) {
            Err(error) => error,
            // The one row whose graph is valid reaches the kernel, on a scratch copy.
            Ok(report) => apply(&mut state, report["preview_id"].as_str().unwrap()).expect_err(file),
        };
        for word in words {
            assert!(error.contains(word), "{file}: {error}");
        }
        assert_eq!(accepted(&mut state, Some(&saved)), before, "{file} left a mark");
        assert!(state.doc.redo_depth() > 0, "{file} cleared redo");
    }

    let id = preview_file(&mut state, "g1-inch.xlsx").unwrap()["preview_id"].as_str().unwrap().to_owned();
    apply(&mut state, &id).unwrap();
    let wide = call(&mut state, "configuration_grid", json!({}));
    assert_eq!(wide["rows"][1]["cells"][0], "130 mm");
    assert!((volume(&mut state) - VOLUME_130).abs() < 0.005);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn g_token_a_preview_is_bound_to_its_document_and_revision_and_used_once() {
    let mut state = AppState::new(Document::new("Part1", soliddraft_app::kernel()));
    g0(&mut state);
    let old = preview_file(&mut state, "g1-inch.csv").unwrap()["preview_id"].as_str().unwrap().to_owned();

    // An ordinary accepted edit that keeps the geometry.
    let geometry = state.doc.model().geometry_hash();
    let mut rows = g0_rows();
    rows[0][4] = json!("Family changed");
    let edit = preview_cells(&mut state, rows.clone())["preview_id"].as_str().unwrap().to_owned();
    apply(&mut state, &edit).unwrap();
    assert_eq!(state.doc.model().geometry_hash(), geometry);
    let after_edit = accepted(&mut state, None);

    let error = apply(&mut state, &old).unwrap_err();
    assert!(error.contains("stale preview"), "{error}");
    assert_eq!(accepted(&mut state, None), after_edit, "the stale token overwrote the edit");

    // A fresh preview of current truth applies once.
    rows[1][1] = json!("130 mm");
    let fresh = preview_cells(&mut state, rows)["preview_id"].as_str().unwrap().to_owned();
    apply(&mut state, &fresh).unwrap();
    let applied = accepted(&mut state, None);
    let error = apply(&mut state, &fresh).unwrap_err();
    assert!(error.contains("consumed preview"), "{error}");
    assert_eq!(accepted(&mut state, None), applied, "a used token made a second entry");

    // A token from another document, even a twin with the same cells, does not apply.
    let mut twin = AppState::new(Document::new("Part1", soliddraft_app::kernel()));
    g0(&mut twin);
    let token = preview_file(&mut twin, "g1-inch.csv").unwrap()["preview_id"].as_str().unwrap().to_owned();
    std::mem::swap(&mut twin.doc, &mut state.doc);
    let before = accepted(&mut twin, None);
    let error = apply(&mut twin, &token).unwrap_err();
    assert!(error.contains("another document"), "{error}");
    assert_eq!(accepted(&mut twin, None), before);
}

fn title_text(state: &AppState) -> Vec<String> {
    let session = state.drawing.session.as_ref().expect("a drawing");
    let list = layout_sheet(&session.doc.drawing, 0);
    list.items.iter().filter_map(|i| if let Item::Text(t) = i { Some(t.text.clone()) } else { None }).collect()
}

#[test]
fn d_title_the_two_undo_stacks_stay_apart_and_title_and_views_move_together() {
    let mut state = AppState::new(Document::new("Part1", soliddraft_app::kernel()));
    g0(&mut state);
    call(&mut state, "make_drawing", json!({ "template": "iso-a3" }));
    call(&mut state, "drawing_auto_update", json!({ "on": false }));
    let bind = json!({ "op": "execute", "command": { "type": "bind_title_property", "sheet": 0, "key": "title", "property": "title" } });
    let part = (state.doc.model().semantic_hash(), state.doc.undo_depth(), state.doc.revision());
    call(&mut state, "drawing_command", json!({ "command_json": bind.to_string() }));
    assert!(title_text(&state).iter().any(|t| t == FAMILY));
    assert_eq!((state.doc.model().semantic_hash(), state.doc.undo_depth(), state.doc.revision()), part);
    call(&mut state, "undo", json!({}));
    assert!(!title_text(&state).iter().any(|t| t == FAMILY));
    assert_eq!(state.doc.undo_depth(), part.1, "the drawing's undo stays in the drawing");
    call(&mut state, "redo", json!({}));

    let pinned = |state: &AppState| {
        let drawing = &state.drawing.session.as_ref().unwrap().doc.drawing;
        (drawing.model.pinned.clone(), drawing.model.configuration.clone(), drawing.sheets.clone())
    };
    let narrow = pinned(&state);
    call(&mut state, "drawing_show_part", json!({}));
    call(&mut state, "activate_configuration", json!({ "name": WIDE_ROW }));
    assert_eq!(pinned(&state), narrow, "a part switch moves neither the views nor the title");
    assert!(title_text(&state).iter().any(|t| t == FAMILY));

    call(&mut state, "show_drawing", json!({}));
    call(&mut state, "drawing_rebuild", json!({}));
    let wide = pinned(&state);
    assert!(title_text(&state).iter().any(|t| t == WIDE));
    assert_eq!(wide.0, state.doc.model().semantic_hash());
    assert_eq!(wide.1.as_ref(), state.doc.model().active_configuration());
    let part_undo = state.doc.undo_depth();
    call(&mut state, "undo", json!({}));
    assert_eq!(pinned(&state), narrow, "one undo restores the pin, the views and the title");
    call(&mut state, "redo", json!({}));
    assert_eq!(pinned(&state), wide);
    assert_eq!(state.doc.undo_depth(), part_undo);
}
