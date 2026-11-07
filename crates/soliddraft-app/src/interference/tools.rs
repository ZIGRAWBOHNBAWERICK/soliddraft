//! Interference and clearance checks over MCP.

use crate::state::AppState;
use serde_json::{Value, json};
use soliddraft_assembly::{CheckRefusal, CheckRequest, CheckScope, OccurrencePath};
use soliddraft_model::ValueTable;

/// A refusal as a tool's error: typed JSON.
pub(crate) fn refused(r: CheckRefusal) -> String {
    serde_json::to_string(&r).unwrap_or_else(|_| r.to_string())
}

/// A length typed as text, such as `0.5 mm`, in millimetres.
pub(crate) fn length(text: &str) -> Result<f64, String> {
    let none = ValueTable::default();
    let expression = none.parse(text).map_err(|e| format!("the clearance: {e}"))?;
    none.length(&expression).map_err(|e| format!("the clearance: {e}"))
}

/// The request a tool call asks for.
fn request(input: &Value, clearance: Option<f64>) -> Result<CheckRequest, String> {
    let scope = match input.get("paths").filter(|p| !p.is_null()) {
        None => CheckScope::Assembly,
        Some(paths) => {
            let paths = paths.as_array().ok_or("paths is a list of occurrence paths")?;
            let parsed = paths
                .iter()
                .map(|p| {
                    let text = p.as_str().ok_or("an occurrence path is text, such as \"4\"")?;
                    text.parse::<OccurrencePath>().map_err(|e| format!("{text}: {e}"))
                })
                .collect::<Result<Vec<_>, String>>()?;
            CheckScope::Occurrences(parsed)
        }
    };
    let mut request = CheckRequest { scope, clearance, ..CheckRequest::default() };
    if let Some(on) = input["include_hidden"].as_bool() {
        request.include_hidden = on;
    }
    if let Some(chord) = input["chord_mm"].as_f64() {
        request.chord = chord;
    }
    Ok(request)
}

/// Runs the check tools. `None` hands the call on.
pub(crate) fn run(state: &mut AppState, name: &str, input: &Value) -> Option<Result<Value, String>> {
    let clearance = match name {
        "assembly_check_interference" => None,
        "assembly_check_clearance" => {
            let text = input["clearance"].as_str().ok_or_else(|| "clearance is required".to_owned());
            match text.and_then(length) {
                Ok(c) => Some(c),
                Err(e) => return Some(Err(e)),
            }
        }
        _ => return None,
    };
    Some(call(state, input, clearance))
}

fn call(state: &mut AppState, input: &Value, clearance: Option<f64>) -> Result<Value, String> {
    let request = request(input, clearance)?;
    let s = state.assembly.session.as_mut().ok_or("no assembly is open")?;
    let report = s.doc.check_interference(&request).map_err(refused)?;
    Ok(json!({ "report": report }))
}
