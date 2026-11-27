//! Manufacturing information on the part: datums, dimensions with limits, geometric
//! tolerances, surface finishes and the part's notes. What an item is and how it follows
//! its faces is `soliddraft-model`'s (see `docs/manufacturing-information.md`); this
//! module holds the commands, the tools, and what reading a STEP file's information into
//! the part reports.

pub(crate) mod assistant;
pub mod panel;
pub(crate) mod registry;
pub(crate) mod tools;

use soliddraft_kernel_api::ImportFormat;
use soliddraft_model::pmi::PmiImport;
use soliddraft_model::{Command, FeatureKind};

use crate::document::{Document, FeatureId, model_id};

/// Reads the manufacturing information of the STEP file the features `added` were just
/// imported from and puts it on their faces, as one undo entry. A file without any
/// leaves the part as it is. What could not be placed is kept for the panel and the
/// tools to show, by name.
pub(crate) fn after_import(doc: &mut Document, added: &[FeatureId]) {
    let source = added.iter().find_map(|id| match &doc.model().feature(model_id(*id))?.kind {
        FeatureKind::ImportStep(import) if import.format == ImportFormat::Step => Some(import.source.clone()),
        _ => None,
    });
    let Some(source) = source else { return };
    // A kernel that reads none, or a file it cannot parse here, has already imported
    // the body: the information is reported as not read, never guessed.
    let read = match doc.kernel().read_step_pmi(&source) {
        Ok(read) => read,
        Err(_) => return,
    };
    if read.pmi.is_empty() && read.skipped.is_empty() {
        return;
    }
    let features: Vec<_> = added.iter().map(|id| model_id(*id)).collect();
    let (items, mut report) = doc.model().pmi_from_step(&features, &read);
    let mut commands: Vec<Command> =
        items.into_iter().map(|item| Command::PutPmiItem { item: Box::new(item), replaces: None }).collect();
    // The file's notes fill only what the part does not say itself.
    let mut notes = doc.model().pmi().part.clone();
    let before = notes.clone();
    if notes.general_tolerance.is_none() {
        notes.general_tolerance.clone_from(&read.pmi.part.general_tolerance);
    }
    if notes.material.is_none() {
        notes.material.clone_from(&read.pmi.part.material);
    }
    for note in &read.pmi.part.notes {
        if !notes.notes.contains(note) {
            notes.notes.push(note.clone());
        }
    }
    if notes != before {
        commands.push(Command::SetPartNotes { notes });
    }
    if !commands.is_empty()
        && let Err(error) = doc.run_pmi("manufacturing information from the file", commands)
    {
        report.left_out.extend(report.added.drain(..).map(|name| (name, error.clone())));
    }
    doc.pmi_import = Some(report);
}

/// What the last STEP import's manufacturing information became, in a line for the
/// status bar: `14 manufacturing items read`, with the count left out if any were.
pub(crate) fn import_summary(report: &PmiImport) -> String {
    let read = match report.added.len() {
        1 => "1 manufacturing item read".to_owned(),
        n => format!("{n} manufacturing items read"),
    };
    match report.left_out.len() {
        0 => read,
        n => format!("{read}; {n} left out"),
    }
}

/// The ribbon's Drawing and files action on the path the user picked: the package with
/// the tool's defaults, and a sentence for the status line.
pub fn ribbon_package(state: &mut crate::state::AppState, path: &std::path::Path) -> Result<String, String> {
    let report = crate::app_tools::AppTools { state }.manufacturing_package(&serde_json::json!({ "path": path }))?;
    Ok(package_summary(&report))
}

/// What the Drawing and files action did, in a sentence for the status line.
pub fn package_summary(report: &serde_json::Value) -> String {
    let count = |value: &serde_json::Value| value.as_array().map_or(0, Vec::len);
    let drawing = &report["drawing"];
    let inspection = &report["inspection"];
    let written = if inspection.is_null() { "STEP, PDF and DXF" } else { "STEP, PDF, DXF, CSV and AS9102 workbook" };
    let mut text = format!(
        "{written} written: {} items in the file, {} on the drawing",
        report["manufacturing_information"]["items"].as_u64().unwrap_or(0),
        count(&drawing["items"]),
    );
    for (key, what) in [("not_shown", "not shown"), ("overlaps", "overlapping"), ("not_in_file", "not in the file")] {
        if count(&drawing[key]) > 0 {
            text.push_str(&format!(", {} {what}", count(&drawing[key])));
        }
    }
    if let Some(n) = drawing["accuracy"]["approximate"].as_u64().filter(|&n| n > 0) {
        text.push_str(&format!(", {n} approximate drawing curves"));
    }
    if let Some(n) = inspection["inspect"].as_u64() {
        text.push_str(&format!(", {n} characteristics to inspect"));
    }
    if count(&inspection["unresolved"]) > 0 {
        text.push_str(&format!(", {} unresolved", count(&inspection["unresolved"])));
    }
    text
}
