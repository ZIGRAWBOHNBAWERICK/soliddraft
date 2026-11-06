//! Import scene: an OpenUSD, glTF, URDF, OBJ or STL file opened as an assembly of mesh
//! reference bodies.
//!
//! `soliddraft_scene` reads the file into one tree in millimetres with Z up and writes it
//! as an assembly, its groups as sub-assemblies and its meshes as `.dwmsh` mesh reference
//! bodies in a `.parts` folder beside it. The app then opens the assembly like any other.
//! Meshes stay meshes: measure, mass, mates and drawing projection refuse them, and
//! converting one to a solid waits for the kernel.

use std::path::Path;

use serde_json::{Value, json};
use soliddraft_assistant::tools::schema::{one_of, optional, string};

use crate::state::AppState;

/// The MCP and command line parameters.
pub(crate) fn params() -> Vec<(&'static str, Value)> {
    vec![
        ("path", string("The file to import: .usd, .usda, .usdc, .usdz, .glb, .gltf, .urdf, .obj or .stl")),
        (
            "out",
            string(
                "The assembly to write, ending in .dwasm. Its sub-assemblies and mesh bodies go in a folder \
                 named after it with .parts, beside it",
            ),
        ),
        (
            "units",
            optional(one_of(
                &["m", "cm", "mm", "in"],
                "The file's length unit, when the file says none or says it wrongly",
            )),
        ),
        ("up", optional(one_of(&["y", "z"], "The file's up axis, when the file says none or says it wrongly"))),
    ]
}

/// The file extensions the import opens, for the file dialog.
pub(crate) const EXTENSIONS: &[&str] = &["usd", "usda", "usdc", "usdz", "glb", "gltf", "urdf", "obj", "stl"];

/// What the caller decides instead of the file.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Choices {
    meters_per_unit: Option<f64>,
    y_up: Option<bool>,
}

impl Choices {
    fn from_input(input: &Value) -> Result<Choices, String> {
        let meters_per_unit = match input["units"].as_str() {
            None => None,
            Some("m") => Some(1.0),
            Some("cm") => Some(0.01),
            Some("mm") => Some(0.001),
            Some("in") => Some(0.0254),
            Some(other) => return Err(format!("units must be m, cm, mm or in, not {other}")),
        };
        let y_up = match input["up"].as_str().map(str::to_ascii_lowercase).as_deref() {
            None => None,
            Some("y") => Some(true),
            Some("z") => Some(false),
            Some(other) => return Err(format!("up must be y or z, not {other}")),
        };
        Ok(Choices { meters_per_unit, y_up })
    }
}

/// Reads `source` and writes it as the assembly `out`. Returns the import record.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn import(source: &Path, out: &Path, choices: Choices) -> Result<Value, String> {
    use soliddraft_scene::{Options, UpAxis};
    if soliddraft_document::DocKind::of_path(out) != Some(soliddraft_document::DocKind::Assembly) {
        return Err("the assembly to write must end in .dwasm".into());
    }
    let options = Options {
        meters_per_unit: choices.meters_per_unit,
        up_axis: choices.y_up.map(|y| if y { UpAxis::Y } else { UpAxis::Z }),
    };
    let scene = soliddraft_scene::read(source, &options).map_err(|e| e.to_string())?;
    let written = soliddraft_scene::assemble::write(&scene, source, out).map_err(|e| e.to_string())?;
    Ok(written.record)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn import(_: &Path, _: &Path, _: Choices) -> Result<Value, String> {
    Err("Scene import runs in the desktop app".into())
}

/// One line for the status bar: what came in, and in what units.
pub(crate) fn said(record: &Value) -> String {
    let nodes = record["nodes"].as_array().map(Vec::as_slice).unwrap_or_default();
    let bodies = nodes.iter().filter(|n| !n["body"].is_null()).count();
    let joints = record["joints"].as_array().map_or(0, Vec::len);
    let unit = match record["units"]["meters_per_unit"].as_f64() {
        Some(1.0) => "metres".to_owned(),
        Some(0.01) => "centimetres".to_owned(),
        Some(0.001) => "millimetres".to_owned(),
        Some(0.0254) => "inches".to_owned(),
        Some(m) => format!("{m} m units"),
        None => "unknown units".to_owned(),
    };
    let up = record["up_axis"]["axis"].as_str().unwrap_or("?");
    let mut line = format!(
        "Imported {}: {bodies} mesh {}, {joints} {}, {unit}, {up} up",
        record["source"]["file"].as_str().unwrap_or_default(),
        if bodies == 1 { "body" } else { "bodies" },
        if joints == 1 { "joint" } else { "joints" },
    );
    let notes = record["notes"].as_array().map_or(0, Vec::len);
    if notes > 0 {
        line.push_str(&format!(" · {notes} left out (see import.json)"));
    }
    line
}

/// MCP's `import_scene`: imports, opens the assembly and reports both.
pub(crate) fn run(state: &mut AppState, input: &Value) -> Result<Value, String> {
    let source = input["path"].as_str().filter(|p| !p.is_empty()).ok_or("path names the file to import")?;
    let out = input["out"].as_str().filter(|p| !p.is_empty()).ok_or("out names the assembly to write")?;
    let choices = Choices::from_input(input)?;
    let record = import(Path::new(source), Path::new(out), choices)?;
    let opened = crate::assembly::tools::open(state, out.into())?;
    state.assembly.active = true;
    state.drawing.active = false;
    Ok(json!({ "done": said(&record), "import": record, "assembly": opened["assembly"] }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_take_units_and_axes() {
        let c = Choices::from_input(&json!({ "units": "mm", "up": "Y" })).unwrap();
        assert_eq!(c, Choices { meters_per_unit: Some(0.001), y_up: Some(true) });
        assert_eq!(Choices::from_input(&json!({})).unwrap(), Choices::default());
        assert!(Choices::from_input(&json!({ "units": "furlong" })).is_err());
        assert!(Choices::from_input(&json!({ "up": "x" })).is_err());
    }

    #[test]
    fn the_status_line_counts_what_came_in() {
        let record = json!({
            "source": { "file": "washer.glb" },
            "units": { "meters_per_unit": 1.0 },
            "up_axis": { "axis": "Y" },
            "nodes": [{ "body": null }, { "body": "1" }, { "body": "2" }],
            "joints": [{}],
            "notes": [],
        });
        assert_eq!(said(&record), "Imported washer.glb: 2 mesh bodies, 1 joint, metres, Y up");
    }
}
