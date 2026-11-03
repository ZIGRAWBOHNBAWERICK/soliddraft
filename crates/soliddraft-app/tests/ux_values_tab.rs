//! The Values tab beside the feature tree, with the mouse and the keyboard: every number
//! typed into the bracket's features is already a row, a typed depth becomes a global, a
//! second dimension follows a formula on the first, a bad formula stays on its cell, a
//! configuration column changes the part, and a save and reopen keeps it all. The real
//! app, the real wgpu pipeline and the real kernel. Frames go to `target/ux-tools/values-tab/`.

use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Document, Selection};
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use soliddraft_model::{EndCondition, FeatureKind};

#[path = "support/ribbon.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use ribbon::App;
use ux_tools::{Case, STANDARD, launch, settle};

fn call(state: &mut AppState, name: &str, arguments: Value) -> Value {
    let caller = Caller::Mcp { client: "test".into() };
    let out = call_checked(&mut AppTools { state }, name, arguments, &caller);
    assert!(!out.is_error, "{name}: {}", out.content);
    serde_json::from_str(&out.content).unwrap_or(Value::Null)
}

/// A plate, a flange on it and a hole through the plate: six features, numbers typed in.
fn bracket() -> Document {
    let mut state = AppState::new(Document::new("Bracket", soliddraft_app::kernel()));
    let rectangle =
        |x: f64, width: f64| json!({ "shape": "rectangle", "center": [x, 45, 0], "width": width, "height": 90 });
    call(&mut state, "sketch", json!({ "plane": "top", "shapes": [rectangle(60.0, 140.0)] }));
    call(&mut state, "extrude", json!({ "sketch": "Sketch1", "depth": 10, "flip": false }));
    call(&mut state, "sketch", json!({ "plane": "top", "shapes": [rectangle(5.0, 10.0)] }));
    call(&mut state, "extrude", json!({ "sketch": "Sketch2", "depth": 70, "flip": false }));
    let hole = json!({ "shape": "circle", "center": [70, 45, 10], "diameter": 12 });
    call(&mut state, "sketch", json!({ "plane": "Boss-Extrude1 · End cap", "shapes": [hole] }));
    call(&mut state, "cut", json!({ "sketch": "Sketch3", "through_all": true, "depth": 1, "flip": false }));
    state.doc
}

fn depth(app: &App, feature: &str) -> f64 {
    let f = app.state().state.doc.model().features().iter().find(|f| f.name == feature).unwrap().clone();
    let FeatureKind::Extrude(e) = f.kind else { panic!("{feature} is not an extrusion") };
    let EndCondition::Blind { depth } = e.end else { panic!("{feature} is not blind") };
    depth
}

fn volume(app: &App) -> f64 {
    let doc = &app.state().state.doc;
    doc.kernel().mass_properties(doc.model().body().unwrap()).unwrap().volume
}

/// Waits for the rebuilt part to show.
fn rebuilt(app: &mut App) {
    ux_tools::rebuilt(app);
    settle(app);
}

/// Replaces a field's text as a person does and presses Enter.
fn type_in(app: &mut App, label: &str, text: &str) {
    app.get_by_label(label).focus();
    app.run_steps(2);
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.get_by_label(label).type_text(text);
    app.run_steps(2);
    app.key_press(Key::Enter);
    app.run_steps(4);
}

/// Presses a button from the keyboard, as a person who tabbed to it does.
fn press(app: &mut App, label: &str) {
    app.get_by_label(label).focus();
    app.run_steps(2);
    app.key_press(Key::Enter);
    app.run_steps(4);
}

fn value(app: &App, label: &str) -> Option<String> {
    app.get_by_label(label).value()
}

/// Every text on screen, labels and values alike.
fn texts(app: &App) -> Vec<String> {
    app.query_all(egui_kittest::kittest::By::new())
        .flat_map(|n| [n.accesskit_node().label(), n.value()])
        .flatten()
        .collect()
}

#[test]
fn the_values_tab_drives_the_bracket_from_globals_and_formulas() {
    let mut case = Case::new("values-tab");
    let mut app = launch(bracket(), STANDARD);
    case.run(&mut app, |case, app| {
        // The tree keeps its selection, expansion and scroll through a round trip.
        let sketch = app.state().state.doc.features.iter().find(|f| f.name == "Sketch2").unwrap().id;
        app.state_mut().state.doc.selection = Some(Selection::Feature(sketch));
        app.state_mut().state.expanded.insert(sketch);
        settle(app);
        let tree = (app.state().state.doc.selection, app.state().state.expanded.clone());
        case.frame(app, "01-tree");

        ribbon::document_tab(app, "Values");
        settle(app);
        // Every typed number is a row, named after its feature, with nothing to add first.
        for (label, shown) in
            [("Default BossExtrude1.Depth", "10"), ("Default BossExtrude2.Depth", "70"), ("Default Sketch1.D1", "140")]
        {
            assert_eq!(value(app, label).as_deref(), Some(shown), "{label}");
        }
        case.frame(app, "02-values-tab");

        ribbon::document_tab(app, "Tree");
        settle(app);
        assert_eq!((app.state().state.doc.selection, app.state().state.expanded.clone()), tree);
        ribbon::document_tab(app, "Values");
        settle(app);

        // A global, then the plate's typed depth replaced with it.
        type_in(app, "New global name", "Plate");
        type_in(app, "New global formula", "12");
        settle(app);
        assert_eq!(app.state().state.values.error(&soliddraft_app::values_tab::Field::NewGlobalFormula), None);
        assert_eq!(value(app, "Default Plate").as_deref(), Some("12"));
        type_in(app, "Default BossExtrude1.Depth", "=Plate");
        rebuilt(app);
        assert_eq!(depth(app, "Boss-Extrude1"), 12.0);
        assert_eq!(value(app, "Default BossExtrude1.Depth").as_deref(), Some("=Plate"));

        // The flange's height follows the plate: a formula on the first dimension.
        type_in(app, "Default BossExtrude2.Depth", "=BossExtrude1.Depth*5");
        rebuilt(app);
        assert_eq!(depth(app, "Boss-Extrude2"), 60.0);
        case.frame(app, "03-formulas");

        // A misspelt name stays on its cell, in words, and the part does not move.
        let before = (app.state().state.doc.model().semantic_hash(), volume(app));
        type_in(app, "Default BossExtrude2.Depth", "=Plat*5");
        settle(app);
        let error =
            app.state().state.values.error(&soliddraft_app::values_tab::Field::Cell("BossExtrude2.Depth".into(), 0));
        assert_eq!(error, Some("Nothing is named Plat"));
        assert!(texts(app).iter().any(|t| t == "Nothing is named Plat"));
        assert_eq!((app.state().state.doc.model().semantic_hash(), volume(app)), before);
        case.frame(app, "04-error-on-cell");
        app.key_press(Key::Escape);
        type_in(app, "Default BossExtrude2.Depth", "=BossExtrude1.Depth*5");
        settle(app);

        // A configuration column: Thin overrides the global, and the part follows it.
        type_in(app, "New configuration name", "Thin");
        settle(app);
        type_in(app, "Thin Plate", "6");
        settle(app);
        press(app, "Column Thin");
        rebuilt(app);
        assert_eq!((depth(app, "Boss-Extrude1"), depth(app, "Boss-Extrude2")), (6.0, 30.0));
        case.frame(app, "05-thin-configuration");
        press(app, "Column Default");
        rebuilt(app);
        assert_eq!(depth(app, "Boss-Extrude1"), 12.0);

        // Saved and opened again: globals, names, formulas and the column all come back.
        let path = case.dir.join("bracket-values.dwprt");
        call(&mut app.state_mut().state, "save", json!({ "path": path }));
        let table = app.state().state.doc.model().values_table();
        call(&mut app.state_mut().state, "new_part", json!({}));
        call(&mut app.state_mut().state, "open", json!({ "path": path }));
        rebuilt(app);
        assert_eq!(app.state().state.doc.model().values_table(), table);
        assert_eq!(value(app, "Default BossExtrude2.Depth").as_deref(), Some("=BossExtrude1.Depth*5"));
        assert_eq!(value(app, "Thin Plate").as_deref(), Some("6"));
        case.frame(app, "06-reopened");
    });
}
