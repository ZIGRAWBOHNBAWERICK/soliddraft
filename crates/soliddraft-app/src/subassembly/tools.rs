//! The Form subassembly command, for the panel and over MCP.

use crate::assembly::{AssemblySession, tools::summary};
use crate::state::AppState;
use serde_json::{Value, json};
use soliddraft_assembly::{
    AssemblyPackage, ComponentId, FormCode, FormRefusal, FormRequest, Formed, OccurrencePath, Step,
};
use std::path::PathBuf;

/// A refusal as a tool's error: typed JSON with its cause.
fn refused(r: &FormRefusal) -> String {
    serde_json::to_string(r).unwrap_or_else(|_| r.to_string())
}

fn invalid(message: impl Into<String>) -> String {
    let r = FormRefusal { code: FormCode::Invalid, message: message.into(), cause: None, expected: None, actual: None };
    refused(&r)
}

/// An occurrence as a tool names it: its path as text, such as "4" or "16/4", or a root
/// component's id.
fn occurrence(v: &Value) -> Result<OccurrencePath, String> {
    match v {
        Value::String(text) => text.parse().map_err(|e: soliddraft_assembly::path::PathError| invalid(e.to_string())),
        Value::Number(n) => n
            .as_u64()
            .and_then(|c| u32::try_from(c).ok())
            .map(|c| OccurrencePath::root().child(Step::component(ComponentId(c))))
            .ok_or_else(|| invalid("a component id is a whole number")),
        _ => Err(invalid("an occurrence is a path such as \"4\", or a component id")),
    }
}

fn request(input: &Value) -> Result<FormRequest, String> {
    let paths = input["paths"].as_array().ok_or_else(|| invalid("paths lists the parts to form"))?;
    let paths = paths.iter().map(occurrence).collect::<Result<Vec<_>, _>>()?;
    let path =
        input["path"].as_str().filter(|p| !p.trim().is_empty()).ok_or_else(|| invalid("path names the new file"))?;
    let origin = occurrence(&input["origin"])?;
    Ok(FormRequest { paths, path: PathBuf::from(path), origin })
}

/// The selection follows the parts into the new file, and the tree opens on it.
fn follow(s: &mut AssemblySession, formed: &Formed) {
    s.selected = s.selected.take().map(|p| formed.migration.map(&p));
    s.chosen = s.chosen.iter().map(|p| formed.migration.map(p)).collect();
    s.expanded.insert(OccurrencePath::root().child(Step::component(formed.migration.component)));
}

/// Forms the open assembly's parts as `request` says, for the panel and the tool alike:
/// the selection follows the parts, and the session keeps the new file's bytes so it can
/// show and prove its pinned revision whatever happens to the file.
pub(crate) fn form(state: &mut AppState, request: &FormRequest) -> Result<Formed, FormRefusal> {
    let Some(s) = state.assembly.session.as_mut() else {
        return Err(FormRefusal {
            code: FormCode::Invalid,
            message: "no assembly is open".into(),
            cause: None,
            expected: None,
            actual: None,
        });
    };
    let formed = s.doc.form_subassembly(request)?;
    follow(s, &formed);
    if let Ok(bytes) = std::fs::read(&formed.path) {
        state.revisions.record(formed.migration.child, &formed.migration.child_revision, &bytes);
    }
    Ok(formed)
}

fn form_tool(state: &mut AppState, input: &Value) -> Result<Value, String> {
    let request = request(input)?;
    if state.assembly.session.is_none() {
        return Err("no assembly is open".into());
    }
    let formed = form(state, &request).map_err(|r| refused(&r))?;
    let bytes = std::fs::read(&formed.path).map_err(|e| e.to_string())?;
    let child = AssemblyPackage::from_bytes(bytes).map_err(|e| e.to_string())?;
    let selected = state.assembly.session.as_ref().and_then(|s| s.selected.clone());
    Ok(json!({
        "formed": formed,
        "child": {
            "path": formed.path, "id": child.id(), "revision": child.manifest().semantic_hash,
            "truth": &**child.assembly(), "solver": formed.status.child,
        },
        "selected": selected,
        "assembly": summary(state),
    }))
}

/// Runs the subassembly tools. `None` hands the call on.
pub(crate) fn run(state: &mut AppState, name: &str, input: &Value) -> Option<Result<Value, String>> {
    (name == "assembly_form_subassembly").then(|| form_tool(state, input))
}
