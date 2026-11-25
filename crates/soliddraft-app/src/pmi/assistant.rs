//! The assistant's manufacturing information tools.
//!
//! Rules decide every value. The assistant reads the part with `pmi_recognise`,
//! `pmi_interview` and `pmi_list`, which run at once, and relays what they say. What it
//! asks to change becomes a card the user applies, as a proposal does: `pmi_answer`
//! takes the interview's proposals, `pmi_add` puts an item the user stated, and
//! `manufacturing_package` writes the files. With auto-apply on they run at once.

use serde_json::{Value, json};
use soliddraft_exchange::Format as ExportFormat;

use super::tools;
use crate::command_tools::ProposedCall;
use crate::document::Document;
use crate::state::AppState;

/// The tools that change the part or write files: from the assistant each makes a card.
pub(crate) const CHANGES: [&str; 3] = ["pmi_answer", "pmi_add", PACKAGE];

/// The tool that writes files and opens a drawing, which only the app can apply.
pub(crate) const PACKAGE: &str = "manufacturing_package";

/// What a card shows before the user applies it.
pub(crate) struct Card {
    /// One line, the card's title.
    pub summary: String,
    /// What it changes, row by row.
    pub rows: Vec<(String, String)>,
}

/// The card for a manufacturing information call, checked first: `pmi_answer` and
/// `pmi_add` run on a scratch copy of the part, and the card lists what they would
/// change; `manufacturing_package` is checked as the action checks it before it writes.
pub(crate) fn card(doc: &Document, call: &ProposedCall) -> Result<Card, String> {
    if call.tool == PACKAGE {
        return package_card(doc, &call.input);
    }
    let mut scratch = doc.scratch();
    let answer = tools::run(&mut scratch, &call.tool, &call.input)
        .ok_or_else(|| format!("`{}` does not change manufacturing information", call.tool))??;
    // The scratch copy's faces are the part's: name them from the part.
    let rows = changes(&tools::rows(doc), &tools::rows_named(&scratch, doc));
    if rows.is_empty() {
        return Err("This changes nothing on the part.".into());
    }
    let summary = match call.tool.as_str() {
        "pmi_answer" => answer_summary(&call.input),
        _ => {
            let name = answer["added"]["name"].as_str().unwrap_or("the item");
            match call.input["replaces"].as_str().map(str::trim).filter(|old| !old.is_empty()) {
                Some(old) if old != name => format!("Replace {old} with {name}"),
                Some(_) => format!("Change {name}"),
                None => format!("Add {name}"),
            }
        }
    };
    Ok(Card { summary, rows })
}

fn answer_summary(input: &Value) -> String {
    let material = input["material"].as_str().map(str::trim).filter(|text| !text.is_empty());
    let taken = match &input["accept"] {
        Value::String(all) if all == "all" => Some("Take every proposal of the interview".to_owned()),
        Value::Array(ids) if !ids.is_empty() => {
            let ids: Vec<&str> = ids.iter().filter_map(Value::as_str).collect();
            Some(format!("Take the interview's proposals for {}", ids.join(", ")))
        }
        _ => None,
    };
    match (taken, material) {
        (Some(taken), Some(material)) => format!("{taken}, with material {material}"),
        (Some(taken), None) => taken,
        (None, Some(material)) => format!("Set the material to {material}"),
        (None, None) => "Answer the interview".to_owned(),
    }
}

/// The rows of a card: each item added, changed or removed, and each note that changes.
fn changes(before: &Value, after: &Value) -> Vec<(String, String)> {
    let items = |listed: &Value| listed["items"].as_array().cloned().unwrap_or_default();
    let (old, new) = (items(before), items(after));
    let mut rows = Vec::new();
    for item in &new {
        match old.iter().find(|o| o["name"] == item["name"]) {
            Some(was) if was == item => {}
            Some(_) => rows.push(("Changes".to_owned(), described(item))),
            None => rows.push(("Adds".to_owned(), described(item))),
        }
    }
    for item in &old {
        if !new.iter().any(|n| n["name"] == item["name"]) {
            rows.push(("Removes".to_owned(), item["name"].as_str().unwrap_or_default().to_owned()));
        }
    }
    for (key, label) in [("material", "Material"), ("general_tolerance", "General tolerance")] {
        let (was, now) = (&before["notes"][key], &after["notes"][key]);
        if was != now {
            rows.push((label.to_owned(), now.as_str().unwrap_or("none").to_owned()));
        }
    }
    rows
}

/// An item as a card row: its name, its callout and its faces.
fn described(item: &Value) -> String {
    let faces: Vec<String> = item["faces"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|feature| {
            feature.as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
        })
        .filter(|faces| !faces.is_empty())
        .collect();
    let mut text =
        format!("{}: {}", item["name"].as_str().unwrap_or_default(), item["callout"].as_str().unwrap_or_default());
    if !faces.is_empty() {
        text.push_str(&format!(" on {}", faces.join("; ")));
    }
    if let Some(problem) = item["problem"].as_str() {
        text.push_str(&format!(" ({problem})"));
    }
    text
}

fn package_card(doc: &Document, input: &Value) -> Result<Card, String> {
    let step = std::path::PathBuf::from(input["path"].as_str().unwrap_or_default().trim());
    if !ExportFormat::Step.suits(&step) {
        return Err("the path must end in .step or .stp".into());
    }
    let template = input["template"].as_str().unwrap_or("iso-a3");
    crate::app_tools::check_drawing(doc, template)?;
    let name = step.file_name().map_or_else(|| step.display().to_string(), |name| name.to_string_lossy().into_owned());
    Ok(Card {
        summary: format!("Write {name} with its drawing as PDF and DXF"),
        rows: vec![
            ("Writes".to_owned(), step.display().to_string()),
            (
                "Drawing".to_owned(),
                format!("{} and {}", step.with_extension("pdf").display(), step.with_extension("dxf").display()),
            ),
            ("Template".to_owned(), template.to_owned()),
            ("Opens".to_owned(), "the drawing, in place of any open drawing".to_owned()),
        ],
    })
}

/// Whether the proposal `id` writes the manufacturing package.
pub(crate) fn is_package(state: &AppState, id: &str) -> bool {
    state.chat.book.get(id).and_then(|p| p.call.as_ref()).is_some_and(|call| call.tool == PACKAGE)
}

/// Writes a pending package proposal's files and opens its drawing, as the ribbon's
/// action does, and marks the proposal applied. Answers the action's report.
pub(crate) fn apply_package(state: &mut AppState, id: &str) -> Result<Value, String> {
    let call = state.chat.book.pending_call(id)?;
    let report = crate::app_tools::AppTools { state: &mut *state }.manufacturing_package(&call.input)?;
    state.chat.book.applied_by_app(&mut state.doc, id)?;
    Ok(json!({ "proposal_id": id, "status": "applied", "report": report }))
}
