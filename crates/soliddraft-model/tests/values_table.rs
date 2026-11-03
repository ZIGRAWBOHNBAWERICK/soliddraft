//! The values table: every typed number is a row, cells take numbers, names and
//! formulas, and a refusal leaves the part as it was.

use std::sync::Arc;

use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::configs::{ValueRow, ValuesTable};
use soliddraft_model::{Command, Document, EndCondition, FeatureKind};

/// An old file with three sketches and extrusions, saved before parameters had names.
fn bracket() -> Document {
    Document::from_bytes(include_bytes!("fixtures/bracket-140.dwprt"), Arc::new(CatallaxKernel::new())).unwrap()
}

fn row<'a>(table: &'a ValuesTable, name: &str) -> &'a ValueRow {
    table
        .globals
        .iter()
        .chain(table.groups.iter().flat_map(|g| &g.rows))
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no row {name}"))
}

fn cell(doc: &mut Document, column: &str, name: &str, text: Option<&str>) -> Result<(), String> {
    let command =
        Command::SetValueCell { configuration: column.into(), name: name.into(), expression: text.map(str::to_owned) };
    doc.execute(command).map(|_| ()).map_err(|e| e.to_string())
}

fn depth(doc: &Document, feature: &str) -> f64 {
    let f = doc.features().iter().find(|f| f.name == feature).unwrap();
    let FeatureKind::Extrude(e) = &f.kind else { panic!("{feature} is not an extrusion") };
    let EndCondition::Blind { depth } = e.end else { panic!("{feature} is not blind") };
    depth
}

fn value(doc: &Document, name: &str, column: usize) -> f64 {
    row(&doc.values_table(), name).values[column].unwrap()
}

#[test]
fn an_old_file_lists_every_typed_number_under_a_default_name() {
    let doc = bracket();
    let table = doc.values_table();
    assert_eq!(table.columns, ["Default"]);
    assert!(table.globals.is_empty());
    let extrude = row(&table, "BossExtrude1.Depth");
    assert_eq!((extrude.label.as_str(), extrude.unit.as_str()), ("Depth", "mm"));
    assert_eq!(extrude.cells, [None], "a typed number has no formula");
    assert_eq!(extrude.values, [Some(10.0)]);
    let first = &table.groups[0];
    assert_eq!(first.name, "Sketch1");
    assert!(first.rows.iter().all(|r| r.name.starts_with("Sketch1.D")), "{:?}", first.rows);
    // Listing names nothing in the file.
    assert!(doc.values().is_empty());
}

#[test]
fn a_global_drives_a_depth_and_a_formula_on_it_drives_a_dimension() {
    let mut doc = bracket();
    let width = value(&doc, "Sketch1.D1", 0);
    doc.execute(Command::SetValue { name: "Plate".into(), expression: "12".into() }).unwrap();
    cell(&mut doc, "Default", "BossExtrude1.Depth", Some("Plate")).unwrap();
    assert_eq!(depth(&doc, "Boss-Extrude1"), 12.0);
    cell(&mut doc, "Default", "Sketch1.D1", Some("BossExtrude1.Depth * 10 + 5")).unwrap();
    assert_eq!(value(&doc, "Sketch1.D1", 0), 125.0);
    assert_ne!(width, 125.0);
    // A dimension feeds a global too.
    doc.execute(Command::SetValue { name: "Twice".into(), expression: "Sketch1.D1 * 2".into() }).unwrap();
    assert_eq!(value(&doc, "Twice", 0), 250.0);
    let table = doc.values_table();
    assert_eq!(row(&table, "BossExtrude1.Depth").cells, [Some("Plate".to_owned())]);

    // Saved and opened again, the names, globals and formulas stay.
    let reopened = Document::from_bytes(&doc.to_bytes().unwrap(), doc.kernel().clone()).unwrap();
    assert_eq!(reopened.values_table(), table);
    assert_eq!(reopened.semantic_hash(), doc.semantic_hash());
}

#[test]
fn a_plain_number_edits_the_feature_and_clearing_a_formula_keeps_its_number() {
    let mut doc = bracket();
    cell(&mut doc, "Default", "BossExtrude1.Depth", Some("1 cm")).unwrap();
    assert_eq!(depth(&doc, "Boss-Extrude1"), 10.0);
    cell(&mut doc, "Default", "BossExtrude1.Depth", Some("14")).unwrap();
    assert_eq!(depth(&doc, "Boss-Extrude1"), 14.0);
    assert_eq!(row(&doc.values_table(), "BossExtrude1.Depth").cells, [None], "a number leaves no formula");
    doc.execute(Command::SetValue { name: "Plate".into(), expression: "16".into() }).unwrap();
    cell(&mut doc, "Default", "BossExtrude1.Depth", Some("Plate + 1")).unwrap();
    cell(&mut doc, "Default", "BossExtrude1.Depth", None).unwrap();
    assert_eq!(depth(&doc, "Boss-Extrude1"), 17.0);
    assert_eq!(row(&doc.values_table(), "BossExtrude1.Depth").cells, [None]);
    // Undo puts the formula back.
    doc.undo().unwrap();
    assert_eq!(row(&doc.values_table(), "BossExtrude1.Depth").cells, [Some("Plate + 1".to_owned())]);
}

fn refused(doc: &mut Document, name: &str, text: &str, words: &str) {
    let truth = doc.part_json();
    let geometry = doc.geometry_hash();
    let undo = doc.undo_depth();
    let error = cell(doc, "Default", name, Some(text)).unwrap_err();
    assert!(error.contains(words), "{text}: {error}");
    assert_eq!(doc.part_json(), truth, "{text} left a mark");
    assert_eq!(doc.geometry_hash(), geometry);
    assert_eq!(doc.undo_depth(), undo);
}

#[test]
fn a_bad_formula_is_refused_and_the_part_keeps_its_last_good_state() {
    let mut doc = bracket();
    cell(&mut doc, "Default", "Sketch1.D1", Some("BossExtrude1.Depth * 12")).unwrap();
    refused(&mut doc, "BossExtrude1.Depth", "Wdth * 2", "unknown value Wdth");
    refused(&mut doc, "BossExtrude1.Depth", "Sketch1.D1 / 12", "cycle");
    refused(&mut doc, "BossExtrude1.Depth", "10 / (Sketch2.D1 - Sketch2.D1)", "division by zero");
    refused(&mut doc, "BossExtrude1.Depth", "30 deg", "length");
    refused(&mut doc, "BossExtrude1.Depth", "0", "positive");
}

#[test]
fn renaming_a_parameter_or_a_global_rewrites_every_formula() {
    let mut doc = bracket();
    doc.execute(Command::SetValue { name: "Plate".into(), expression: "12".into() }).unwrap();
    cell(&mut doc, "Default", "BossExtrude1.Depth", Some("Plate")).unwrap();
    cell(&mut doc, "Default", "Sketch1.D1", Some("BossExtrude1.Depth * 10")).unwrap();
    doc.execute(Command::RenameValue { name: "BossExtrude1.Depth".into(), to: "PlateDepth".into() }).unwrap();
    doc.execute(Command::RenameValue { name: "Plate".into(), to: "Thick".into() }).unwrap();
    let table = doc.values_table();
    assert_eq!(row(&table, "PlateDepth").cells, [Some("Thick".to_owned())]);
    assert_eq!(row(&table, "Sketch1.D1").cells, [Some("PlateDepth * 10".to_owned())]);
    // A renamed feature does not break a formula that names its parameter.
    let id = doc.features().iter().find(|f| f.name == "Sketch1").unwrap().id;
    doc.execute(Command::RenameFeature { id, name: "Base sketch".into() }).unwrap();
    assert_eq!(value(&doc, "Sketch1.D1", 0), 120.0);
    let error = doc.execute(Command::RemoveValue { name: "Thick".into() }).unwrap_err().to_string();
    assert!(error.contains("Thick is used by PlateDepth"), "{error}");
}

#[test]
fn each_configuration_column_overrides_globals_and_parameters() {
    let mut doc = bracket();
    doc.execute(Command::SetValue { name: "Plate".into(), expression: "12".into() }).unwrap();
    cell(&mut doc, "Default", "BossExtrude1.Depth", Some("Plate")).unwrap();
    doc.execute(Command::AddConfiguration { name: "Thin".into() }).unwrap();
    cell(&mut doc, "Thin", "Plate", Some("6")).unwrap();
    cell(&mut doc, "Thin", "BossExtrude2.Depth", Some("Plate * 5")).unwrap();
    let table = doc.values_table();
    assert_eq!(table.columns, ["Default", "Thin"]);
    assert_eq!(row(&table, "BossExtrude1.Depth").values, [Some(12.0), Some(6.0)]);
    assert_eq!(row(&table, "BossExtrude2.Depth").cells[1].as_deref(), Some("Plate * 5"));
    assert_eq!(row(&table, "BossExtrude2.Depth").values[1], Some(30.0));
    doc.execute(Command::ActivateConfiguration { name: "Thin".into() }).unwrap();
    assert_eq!((depth(&doc, "Boss-Extrude1"), depth(&doc, "Boss-Extrude2")), (6.0, 30.0));
    assert_eq!(doc.values_table().active, 1);
    // A blank cell inherits again.
    cell(&mut doc, "Thin", "BossExtrude2.Depth", None).unwrap();
    assert_ne!(depth(&doc, "Boss-Extrude2"), 30.0);
    let reopened = Document::from_bytes(&doc.to_bytes().unwrap(), doc.kernel().clone()).unwrap();
    assert_eq!(reopened.values_table(), doc.values_table());
    // The table of globals keeps a parameter's override out and leaves it in place.
    cell(&mut doc, "Thin", "BossExtrude2.Depth", Some("40")).unwrap();
    let grid = doc.configuration_grid();
    assert!(grid.rows[1].values.keys().all(|k| k == "Plate"), "{:?}", grid.rows[1].values);
    let checked = doc.check_configuration_grid(&grid).unwrap();
    doc.execute(Command::SetConfigurationGrid { grid: Box::new(checked) }).unwrap();
    assert_eq!(depth(&doc, "Boss-Extrude2"), 40.0);
}
