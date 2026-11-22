//! The Pack and Go and export tools.

use crate::assembly::tools::summary;
use crate::state::AppState;
use serde_json::{Value, json};
use soliddraft_assembly::PackRequest;
use std::path::PathBuf;

fn text<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    input[key].as_str().filter(|t| !t.trim().is_empty()).ok_or_else(|| format!("{key} is required"))
}

/// Runs the tools. `None` hands the call on.
pub(crate) fn run(state: &mut AppState, name: &str, input: &Value) -> Option<Result<Value, String>> {
    Some(match name {
        "assembly_pack_and_go" => (|| {
            let request = PackRequest {
                into: PathBuf::from(text(input, "folder")?),
                prefix: input["prefix"].as_str().unwrap_or_default().to_owned(),
                suffix: input["suffix"].as_str().unwrap_or_default().to_owned(),
                flatten: input["flatten"].as_bool().unwrap_or(false),
                ..PackRequest::default()
            };
            let drawings = input["drawings"].as_bool().unwrap_or(false);
            let report = super::pack(state, request, drawings)?;
            Ok(json!({ "packed": report, "assembly": summary(state) }))
        })(),
        "assembly_export_urdf" | "assembly_export_usd" => (|| {
            let path = PathBuf::from(text(input, "path")?);
            let want = if name == "assembly_export_urdf" { "urdf" } else { "usda" };
            if path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() != Some(want) {
                return Err(format!("the path must end in .{want}"));
            }
            let (articulation, files) = super::export(state, &path)?;
            Ok(json!({ "exported": articulation, "files": files, "assembly": summary(state) }))
        })(),
        _ => return None,
    })
}
