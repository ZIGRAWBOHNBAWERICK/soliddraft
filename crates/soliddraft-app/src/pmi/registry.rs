//! Manufacturing information command registry rows.

use crate::commands::{Agents, CommandSpec, Size, Undo};
use crate::icons::Icon;
use crate::state::{AppAction, RibbonTab};
use serde_json::Value;
use soliddraft_assistant::tools::schema::{
    any_of, array, boolean, described, integer, number, object, one_of, optional, string,
};

fn segment() -> Value {
    object(&[
        ("value_mm", string("Explicit total zone diameter in mm, as a decimal")),
        ("diameter_zone", boolean("Must be true for this axis control")),
        ("material", optional(one_of(&["regardless", "maximum", "least"], "Feature material condition"))),
        (
            "datums",
            optional(array(
                object(&[
                    ("letter", string("A declared datum")),
                    ("material", optional(one_of(&["regardless", "maximum", "least"], "Datum material boundary"))),
                ]),
                "Ordered datum references, primary first",
            )),
        ),
    ])
}

fn vector(label: &str) -> Value {
    array(number("Coordinate in part axes"), label)
}

fn target() -> Value {
    object(&[
        ("datum", string("Declared datum letter")),
        ("number", integer("Explicit positive target number for this datum")),
        ("face", string("One face in the datum declaration")),
        ("point_mm", vector("Target center: three part coordinates in mm")),
        (
            "kind",
            object(&[
                ("shape", one_of(&["point", "line", "area"], "Target shape")),
                ("length_mm", optional(string("Line only: required full length in mm"))),
                ("direction", optional(vector("Line only: required tangent direction in part axes"))),
                (
                    "area",
                    optional(object(&[
                        ("shape", one_of(&["circular", "rectangular"], "Required area shape")),
                        ("diameter_mm", optional(string("Circular only: required full diameter in mm"))),
                        ("length_mm", optional(string("Rectangular only: required full length in mm"))),
                        ("width_mm", optional(string("Rectangular only: required full width in mm"))),
                        ("direction", optional(vector("Rectangular only: required tangent length axis"))),
                    ])),
                ),
            ]),
        ),
    ])
}

fn position_frame() -> Value {
    object(&[
        ("frame", one_of(&["single", "composite"], "Position frame type")),
        ("segment", optional(described(segment(), "Required for a single frame; omit for a composite"))),
        (
            "projected_zone",
            optional(described(
                object(&[
                    ("height_mm", string("Explicit positive height along the outward axis, in mm")),
                    ("direction", vector("Explicit outward direction in part axes")),
                    ("entry_faces", array(string("Planar entry face name"), "One per feature, in order")),
                ]),
                "Single frames only",
            )),
        ),
        ("upper", optional(described(segment(), "Required upper segment of a composite; omit for a single frame"))),
        ("lower", optional(described(segment(), "Required lower segment of a composite; omit for a single frame"))),
    ])
}

fn add_params() -> Vec<(&'static str, Value)> {
    let faces = || array(string("A face name, such as Boss-Extrude1 · End cap"), "The faces of one feature");
    let two = |what| array(faces(), what);
    vec![
        (
            "kind",
            string(
                "datum, datum_target, diameter, radius, hole, distance, slot, width, chamfer, edge_break, basic_angle, tolerance, position_control, finish, knurl or face_note",
            ),
        ),
        ("name", optional(string("The item's name; one is made up when left out"))),
        ("replaces", optional(string("The name of the item this one takes the place of"))),
        ("faces", optional(described(faces(), "Exactly one face name for knurl or face_note"))),
        ("features", optional(two("A pattern: the faces of each feature, such as each of four holes"))),
        ("letter", optional(string("A datum's letter; the next free one when left out"))),
        ("target", optional(target())),
        ("frame", optional(position_frame())),
        ("designation", optional(string("Knurl designation as stated by the user; required for knurl"))),
        ("text", optional(string("Note as written by the user; required for face_note"))),
        ("tolerance", optional(string("A dimension's tolerance: H7, ±0.1, +0.1/0, +0.2/-0.1 or basic"))),
        ("through", optional(boolean("true for a drill that goes through the part"))),
        ("thread", optional(string("A threaded diameter or hole's designation, such as M6x1-6H"))),
        ("pilot_faces", optional(faces())),
        ("head_kind", optional(string("For a compound hole: counterbore or countersink"))),
        ("head_faces", optional(faces())),
        ("drill_depth", optional(string("Explicit blind drill depth in mm, as a decimal; omit to measure"))),
        (
            "thread_depth",
            optional(string("Explicit usable thread depth in mm, as a decimal; requires a thread designation")),
        ),
        ("from_faces", optional(faces())),
        ("from_at", optional(string("Where a distance starts: feature, middle, inner or outer"))),
        ("to_faces", optional(faces())),
        ("to_at", optional(string("Where a distance ends: feature, middle, inner or outer"))),
        ("flats", optional(two("The two flat sides of a slot or width"))),
        ("ends", optional(two("A slot's two round ends"))),
        ("bevel", optional(faces())),
        ("sides", optional(two("The two supports of a chamfer or edge break, reference side first"))),
        ("planes", optional(two("The two planar groups for a basic angle"))),
        ("min_mm", optional(string("Edge break minimum setback in mm, explicitly stated"))),
        ("max_mm", optional(string("Edge break maximum setback in mm, explicitly stated"))),
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

fn answer_params() -> Vec<(&'static str, Value)> {
    vec![
        (
            "accept",
            optional(any_of(vec![
                string("\"all\" takes every proposal"),
                array(string("A question id from pmi_interview"), "The questions whose proposals to take"),
            ])),
        ),
        ("material", optional(string("The part's material, which no rule can propose"))),
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
        CommandSpec::new(
            "manufacturing_package",
            "Drawing and files",
            Icon::Cad("drawing"),
            "Write the STEP file with its manufacturing information and a drawing of it as PDF and DXF",
        )
        .on(RibbonTab::Inspect, "Annotate", Size::Large)
        .shown(|s| !s.in_sketch())
        .run(|_| AppAction::ManufacturingPackage)
        .when(|s| s.doc.display_body().is_some() && !s.doc.proof_pending() && s.doc.model().live_bodies().len() == 1)
        .agents(
            Agents::Tool,
            "Writes path (.step) with the part's manufacturing information, and a drawing made from it as .pdf and .dxf \
             beside it. The answer lists what the drawing shows, what it could not show, what overlaps and what is on \
             the drawing but not in the file. The drawing becomes the active document, in place of an open drawing.",
            package_params,
        )
        .undo(Undo::Replaces),
        row("pmi_add", "Add manufacturing information", "Put a dimension, tolerance or annotation on faces").agents(
            Agents::Tool,
            "Adds an item or replaces the item named in replaces. Nominal sizes are measured from the part. \
             Only specify drill depth, thread depth or limits when the user supplies them. For datum_target and \
             position_control, use only stated intent. Ask for a missing target, datum, material modifier, zone \
             height or direction; do not make it up. Omitted segment and datum materials mean RFS and RMB. \
             For knurl or face_note, ask for the face and the user's designation or text if missing.",
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
        row(
            "pmi_recognise",
            "Recognise features",
            "List the holes, hole patterns, bosses and slots the part's faces show",
        )
        .agents(
            Agents::Tool,
            "Reads the features off the body's faces, on a modelled or an imported part: each with its faces, whether \
             it goes through, what it opens onto and its sizes. It is geometry, not a proposal.",
            Vec::new,
        ),
        row("pmi_interview", "Interview", "List what the part's manufacturing information still lacks").agents(
            Agents::Tool,
            "The decisions still open, in order: material, general tolerance, datums, then each hole, boss and slot. \
             Each has a proposal from rules and standards tables and the reason for it. Nothing is added to the part.",
            Vec::new,
        ),
        row("pmi_answer", "Answer the interview", "Take the interview's proposals as the part's items").agents(
            Agents::Tool,
            "Adds the proposals of the questions named in accept, or of every question with accept \"all\", and sets \
             the material when it is given, as one undo step. Answer a question another way with pmi_add or pmi_notes.",
            answer_params,
        )
        .undo(Undo::Step),
        row("pmi_list", "List manufacturing information", "List the part's items as a drawing writes them").agents(
            Agents::Tool,
            "Every item with its callout, its faces and what is wrong with it if it does not apply, and the part's notes.",
            Vec::new,
        ),
    ]
}
