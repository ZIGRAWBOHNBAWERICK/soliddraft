//! The Values tab beside the feature tree, with the mouse and the keyboard: every number
//! typed into the bracket's features is already a row, a typed depth becomes a global, a
//! second dimension follows a formula on the first, a bad formula stays on its cell, a
//! configuration column changes the part, and a save and reopen keeps it all. The real
//! app, the real wgpu pipeline and the real kernel. Frames go to `target/ux-tools/values-tab/`
//! and `target/ux-tools/values-excel/`.

use egui::{Key, Modifiers};
use egui_kittest::kittest::{NodeT, Queryable};

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Document, Selection};
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use soliddraft_model::{EndCondition, FeatureKind};

#[path = "support/ui.rs"]
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
        type_in(app, "Default BossExtrude2.Depth", "=ROUND(BossExtrude1.Depth*5,0)");
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
        assert!(texts(app).iter().any(|t| t == "#NAME?"));
        assert_eq!((app.state().state.doc.model().semantic_hash(), volume(app)), before);
        case.frame(app, "04-error-on-cell");
        app.key_press(Key::Escape);
        type_in(app, "Default BossExtrude2.Depth", "=ROUND(BossExtrude1.Depth*5,0)");
        settle(app);

        // The selected tab edits its own values and suppression state.
        press(app, "New configuration");
        type_in(app, "New configuration name", "Thin");
        settle(app);
        press(app, "Configuration Thin");
        type_in(app, "Thin Plate", "6");
        rebuilt(app);
        assert_eq!((depth(app, "Boss-Extrude1"), depth(app, "Boss-Extrude2")), (6.0, 30.0));
        let cut_volume = volume(app);
        press(app, "Feature suppression");
        press(app, "Suppress Cut-Extrude1");
        rebuilt(app);
        let thin_volume = volume(app);
        assert!(thin_volume > cut_volume);
        case.frame(app, "05-thin-configuration");
        press(app, "New configuration");
        type_in(app, "New configuration name", "Tall");
        press(app, "Configuration Tall");
        type_in(app, "Tall Plate", "14");
        rebuilt(app);
        let tall_volume = volume(app);
        assert_eq!(depth(app, "Boss-Extrude2"), 70.0);
        press(app, "Configuration Default");
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
        assert_eq!(value(app, "Default BossExtrude2.Depth").as_deref(), Some("=ROUND(BossExtrude1.Depth*5,0)"));
        press(app, "Configuration Thin");
        rebuilt(app);
        assert_eq!(value(app, "Thin Plate").as_deref(), Some("6"));
        assert_eq!(depth(app, "Boss-Extrude2"), 30.0);
        assert_eq!(volume(app), thin_volume);
        press(app, "Configuration Tall");
        rebuilt(app);
        assert_eq!(value(app, "Tall Plate").as_deref(), Some("14"));
        assert_eq!(volume(app), tall_volume);
        call(&mut app.state_mut().state, "sketch", json!({ "plane": "top", "shapes": [{ "shape": "rectangle", "center": [30, 30, 0], "width": 10, "height": 10 }] }));
        call(&mut app.state_mut().state, "extrude", json!({ "sketch": "Sketch4", "depth": 22, "flip": false }));
        rebuilt(app);
        assert!(volume(app) > tall_volume);
        case.frame(app, "06-reopened");
    });
}

fn global(app: &App, name: &str) -> (Option<f64>, bool) {
    let table = app.state().state.doc.model().values_table();
    let row = table.globals.iter().find(|row| row.name == name).unwrap_or_else(|| panic!("no global {name}"));
    (row.values[table.active], row.logical[table.active])
}

fn columns(app: &App) -> Vec<String> {
    app.state().state.doc.model().values_table().columns
}

/// Long's own Gap formula, from the part.
fn long_gap(app: &App) -> Option<String> {
    let table = app.state().state.doc.model().values_table();
    let column = table.columns.iter().position(|c| c == "Long")?;
    table.globals.iter().find(|row| row.name == "Gap")?.cells[column].clone()
}

/// A middle click on a control, the press and the release in separate frames.
fn middle_click(app: &mut App, label: &str) {
    let at = app.get_by_label(label).rect().center();
    app.hover_at(at);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Middle,
            pressed,
            modifiers: Modifiers::default(),
        });
        app.step();
    }
    app.run_steps(3);
}

/// Opens a tab's menu with the right button and picks an entry.
fn tab_menu(app: &mut App, tab: &str, entry: &str) {
    app.get_by_label(&format!("Configuration {tab}")).click_secondary();
    app.run_steps(4);
    app.get_by_label(entry).click();
    app.run_steps(4);
}

#[test]
fn excel_formulas_and_browser_tabs_drive_the_bracket() {
    let mut case = Case::new("values-excel");
    let mut app = launch(bracket(), STANDARD);
    case.run(&mut app, |case, app| {
        ribbon::document_tab(app, "Values");
        settle(app);
        press(app, "Expand");
        settle(app);

        // Globals typed as a person types Excel formulas.
        for (name, formula) in [("Gap", "=MROUND(7.3,2)"), ("Thick", "=IFERROR(1/0,5 mm)"), ("Fits", "=0.1+0.2=0.3")] {
            type_in(app, "New global name", name);
            type_in(app, "New global formula", formula);
            settle(app);
            assert_eq!(value(app, &format!("Default {name}")).as_deref(), Some(formula), "{name}");
        }
        assert_eq!(global(app, "Gap"), (Some(8.0), false));
        assert_eq!(global(app, "Thick"), (Some(5.0), false));
        assert_eq!(global(app, "Fits"), (Some(1.0), true));
        assert!(texts(app).iter().any(|t| t == "TRUE"), "a comparison shows TRUE, as in Excel");

        // The plate and the flange follow formulas over them.
        type_in(app, "Default BossExtrude1.Depth", "=IF(AND(Fits,Gap>6),Thick*2,Thick)");
        rebuilt(app);
        assert_eq!(depth(app, "Boss-Extrude1"), 10.0);
        type_in(app, "Default BossExtrude2.Depth", "=ROUND(Gap*Thick*1.6,)");
        rebuilt(app);
        assert_eq!(depth(app, "Boss-Extrude2"), 64.0);

        // An error value shows on its cell as Excel shows it, in words beside, and the part stays.
        let before = app.state().state.doc.model().semantic_hash();
        type_in(app, "Default BossExtrude2.Depth", "=NA()");
        settle(app);
        let cell = soliddraft_app::values_tab::Field::Cell("BossExtrude2.Depth".into(), 0);
        assert_eq!(app.state().state.values.error(&cell), Some("No value is available (#N/A)"));
        assert!(texts(app).iter().any(|t| t == "#N/A"));
        assert_eq!(app.state().state.doc.model().semantic_hash(), before);
        case.frame(app, "01-excel-formulas");
        app.key_press(Key::Escape);
        app.run_steps(2);

        // A new tab opens with its name field, as a browser's new tab opens with its address bar.
        press(app, "New configuration");
        type_in(app, "New configuration name", "Long");
        settle(app);
        press(app, "Configuration Long");
        type_in(app, "Long Gap", "=MROUND(11,4)");
        rebuilt(app);
        assert_eq!(depth(app, "Boss-Extrude2"), 96.0);
        press(app, "New configuration");
        type_in(app, "New configuration name", "Spare");
        settle(app);
        assert_eq!(columns(app), ["Default", "Long", "Spare"]);
        case.frame(app, "02-browser-tabs");

        // A tab closes from its own button, and Undo brings it back.
        press(app, "Close Spare");
        settle(app);
        assert_eq!(columns(app), ["Default", "Long"]);
        ux_tools::undo(app);
        settle(app);
        assert_eq!(columns(app), ["Default", "Long", "Spare"]);

        // A middle click closes a tab as a browser's does, and Undo brings it back with
        // its values; so does its Close button.
        let table = app.state().state.doc.model().values_table();
        assert_eq!(long_gap(app).as_deref(), Some("=MROUND(11,4)"));
        middle_click(app, "Configuration Long");
        settle(app);
        assert_eq!(columns(app), ["Default", "Spare"]);
        ux_tools::undo(app);
        rebuilt(app);
        assert_eq!(app.state().state.doc.model().values_table(), table);
        press(app, "Close Long");
        settle(app);
        assert_eq!(columns(app), ["Default", "Spare"]);
        ux_tools::undo(app);
        rebuilt(app);
        assert_eq!(app.state().state.doc.model().values_table(), table);
        assert_eq!(long_gap(app).as_deref(), Some("=MROUND(11,4)"));

        // The tab's menu renames it in place and moves it.
        tab_menu(app, "Spare", "Rename");
        type_in(app, "Configuration name", "Short");
        settle(app);
        tab_menu(app, "Short", "Move left");
        settle(app);
        assert_eq!(columns(app), ["Default", "Short", "Long"]);
        case.frame(app, "03-renamed-and-moved");

        // Saved and opened again, then a later feature on top follows a formula too.
        let path = case.dir.join("bracket-excel.dwprt");
        call(&mut app.state_mut().state, "save", json!({ "path": path }));
        let table = app.state().state.doc.model().values_table();
        call(&mut app.state_mut().state, "new_part", json!({}));
        call(&mut app.state_mut().state, "open", json!({ "path": path }));
        rebuilt(app);
        assert_eq!(app.state().state.doc.model().values_table(), table);
        assert_eq!(columns(app), ["Default", "Short", "Long"]);
        press(app, "Configuration Long");
        rebuilt(app);
        assert_eq!(value(app, "Long Gap").as_deref(), Some("=MROUND(11,4)"));
        assert_eq!(depth(app, "Boss-Extrude2"), 96.0);
        let volume_before = volume(app);
        let post = json!({ "shape": "rectangle", "center": [30, 30, 0], "width": 10, "height": 10 });
        call(&mut app.state_mut().state, "sketch", json!({ "plane": "top", "shapes": [post] }));
        call(&mut app.state_mut().state, "extrude", json!({ "sketch": "Sketch4", "depth": 22, "flip": false }));
        rebuilt(app);
        type_in(app, "Long BossExtrude3.Depth", "=CEILING.MATH(Gap*1.9,5)");
        rebuilt(app);
        assert_eq!(depth(app, "Boss-Extrude3"), 25.0);
        assert!(volume(app) > volume_before);
        case.frame(app, "04-reopened-later-feature");
    });
}
