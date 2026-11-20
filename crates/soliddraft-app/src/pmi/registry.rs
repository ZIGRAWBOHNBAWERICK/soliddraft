//! Manufacturing information command registry rows.

use crate::commands::{Agents, CommandSpec};
use crate::icons::Icon;
use serde_json::Value;
use soliddraft_assistant::tools::schema::{array, boolean, optional, string};

fn add_params() -> Vec<(&'static str, Value)> {
    let faces = || array(string("A face name, such as Boss-Extrude1 · End cap"), "The faces of one feature");
    vec![
        ("kind", string("datum, diameter, radius, distance, tolerance or finish")),
        ("name", optional(string("The item's name; one is made up when left out"))),
        ("replaces", optional(string("The name of the item this one takes the place of"))),
        ("faces", optional(faces())),
        ("features", optional(array(faces(), "A pattern: the faces of each feature, such as each of four holes"))),
        ("letter", optional(string("A datum's letter; the next free one when left out"))),
        ("limits", optional(string("A dimension's limits: H7, ±0.1, +0.1/0 or +0.2/-0.1"))),
        ("from_faces", optional(faces())),
        ("from_at", optional(string("Where a distance starts: feature, middle, inner or outer"))),
        ("to_faces", optional(faces())),
        ("to_at", optional(string("Where a distance ends: feature, middle, inner or outer"))),
        (
            "characteristic",
            optional(string("A tolerance's characteristic, such as position, flatness or perpendicularity")),
        ),
        ("value", optional(string("A tolerance in mm, such as 0.05, or a roughness in micrometres, such as 1.6"))),
        ("diameter_zone", optional(boolean("true when the tolerance zone is a cylinder"))),
        ("material", optional(string("maximum or least, for a tolerance at a material condition"))),
        ("datums", optional(array(string("A datum letter"), "The datums a tolerance refers to, primary first"))),
        ("parameter", optional(string("A finish's roughness parameter; Ra when left out"))),
        ("process", optional(string("any, removal or no_removal"))),
    ]
}

fn remove_params() -> Vec<(&'static str, Value)> {
    vec![("name", string("The item's name"))]
}

fn notes_params() -> Vec<(&'static str, Value)> {
    vec![
        ("general_tolerance", optional(string("Such as ISO 2768-m; an empty text clears it"))),
        ("material", optional(string("The material as the drawing names it; an empty text clears it"))),
        ("notes", optional(array(string("A note, such as Break sharp edges"), "Every other note of the part"))),
    ]
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let row = |id, label, doc| CommandSpec::new(id, label, Icon::SmartDim, doc);
    vec![
        row("pmi_add", "Add manufacturing information", "Put a datum, dimension, tolerance or finish on faces").agents(
            Agents::Mcp,
            "Adds one item to the part, or replaces the item named in replaces. A dimension takes no size: it reads the part's.",
            add_params,
        ),
        row("pmi_remove", "Remove manufacturing information", "Remove one item from the part").agents(
            Agents::Mcp,
            "Removes the named item. A datum a tolerance refers to stays until that tolerance lets go of it.",
            remove_params,
        ),
        row("pmi_notes", "Part notes", "Set the general tolerance, the material note and other notes").agents(
            Agents::Mcp,
            "Sets the notes that apply to the whole part. A field left out keeps its value.",
            notes_params,
        ),
        row("pmi_list", "List manufacturing information", "List the part's items as a drawing writes them").agents(
            Agents::Mcp,
            "Every item with its callout, its faces and what is wrong with it if it does not apply, and the part's notes.",
            Vec::new,
        ),
    ]
}
