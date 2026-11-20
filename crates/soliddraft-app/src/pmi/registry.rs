//! Manufacturing information command registry rows.

use crate::commands::{Agents, CommandSpec, Size, Undo};
use crate::icons::Icon;
use crate::state::{AppAction, RibbonTab};
use serde_json::Value;
use soliddraft_assistant::tools::schema::{array, boolean, optional, string};

fn add_params() -> Vec<(&'static str, Value)> {
    let faces = || array(string("A face name, such as Boss-Extrude1 · End cap"), "The faces of one feature");
    let two = |what| array(faces(), what);
    vec![
        ("kind", string("datum, diameter, radius, distance, slot, tolerance or finish")),
        ("name", optional(string("The item's name; one is made up when left out"))),
        ("replaces", optional(string("The name of the item this one takes the place of"))),
        ("faces", optional(faces())),
        ("features", optional(two("A pattern: the faces of each feature, such as each of four holes"))),
        ("letter", optional(string("A datum's letter; the next free one when left out"))),
        ("tolerance", optional(string("A dimension's tolerance: H7, ±0.1, +0.1/0, +0.2/-0.1 or basic"))),
        ("through", optional(boolean("true for the diameter of a hole that goes through the part"))),
        ("from_faces", optional(faces())),
        ("from_at", optional(string("Where a distance starts: feature, middle, inner or outer"))),
        ("to_faces", optional(faces())),
        ("to_at", optional(string("Where a distance ends: feature, middle, inner or outer"))),
        ("flats", optional(two("A slot's two flat sides"))),
        ("ends", optional(two("A slot's two round ends"))),
        ("characteristic", optional(string("A geometric tolerance's characteristic, such as position or flatness"))),
        ("value", optional(string("A geometric tolerance in mm, such as 0.05"))),
        ("diameter_zone", optional(boolean("true when the tolerance zone is a cylinder"))),
        ("material", optional(string("regardless, maximum or least, for a tolerance at a material condition"))),
        ("datums", optional(array(string("A datum letter"), "The datums a tolerance refers to, primary first"))),
        ("roughness", optional(string("A finish's requirement, such as Ra 1.6"))),
        ("process", optional(string("any, removal or no_removal"))),
        ("method", optional(string("A finish's production method, such as Ground"))),
        ("lay", optional(string("The direction of a finish's lay, such as parallel or circular"))),
        ("allowance", optional(string("A finish's machining allowance, such as 0.5"))),
        ("all_around", optional(boolean("true when the finish holds all around the outline"))),
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

fn package_params() -> Vec<(&'static str, Value)> {
    vec![
        ("path", string("The STEP file to write, ending in .step; the drawing goes beside it as .pdf and .dxf")),
        ("template", optional(string("The sheet template's id; iso-a3 when left out"))),
    ]
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let row = |id, label, doc| CommandSpec::new(id, label, Icon::Tolerance, doc);
    vec![
        row(
            "manufacturing_information",
            "Manufacturing information",
            "Put datums, dimensions, tolerances and finishes on the part's faces",
        )
        .on(RibbonTab::Inspect, "Annotate", Size::Large)
        .shown(|s| !s.in_sketch())
        .run(|_| AppAction::ToggleManufacturingInformation)
        .lit(|s| s.pmi.open)
        .app_only("It opens the Manufacturing information panel; the pmi tools change the same items without it"),
        row(
            "manufacturing_package",
            "Drawing and files",
            "Write the STEP file with its manufacturing information and a drawing of it as PDF and DXF",
        )
        .on(RibbonTab::Inspect, "Annotate", Size::Large)
        .shown(|s| !s.in_sketch())
        .run(|_| AppAction::ManufacturingPackage)
        .when(|s| s.doc.display_body().is_some() && !s.doc.proof_pending() && s.doc.model().live_bodies().len() == 1)
        .agents(
            Agents::Mcp,
            "Writes path (.step) with the part's manufacturing information, and a drawing made from it as .pdf and .dxf \
             beside it. The answer lists what the drawing shows, what it could not show, what overlaps and what is on \
             the drawing but not in the file.",
            package_params,
        ),
        row("pmi_add", "Add manufacturing information", "Put a datum, dimension, tolerance or finish on faces").agents(
            Agents::Mcp,
            "Adds one item to the part, or replaces the item named in replaces. A dimension takes no size: it reads the part's.",
            add_params,
        )
        .undo(Undo::Step),
        row("pmi_remove", "Remove manufacturing information", "Remove one item from the part").agents(
            Agents::Mcp,
            "Removes the named item. A datum a tolerance refers to stays until that tolerance lets go of it.",
            remove_params,
        )
        .undo(Undo::Step),
        row("pmi_notes", "Part notes", "Set the general tolerance, the material note and other notes").agents(
            Agents::Mcp,
            "Sets the notes that apply to the whole part. A field left out keeps its value.",
            notes_params,
        )
        .undo(Undo::Step),
        row("pmi_list", "List manufacturing information", "List the part's items as a drawing writes them").agents(
            Agents::Mcp,
            "Every item with its callout, its faces and what is wrong with it if it does not apply, and the part's notes.",
            Vec::new,
        ),
    ]
}
