//! Mate command registry rows.

use crate::assembly::AssemblyAction;
use crate::commands::{Agents, CommandSpec, Size, Tab, Undo};
use crate::icons::Icon;
use crate::state::AppAction;
use serde_json::Value;
use soliddraft_assistant::tools::schema::{
    array, boolean, described, integer, number, object, one_of, optional, string,
};

/// A face, plane, edge or vertex of a root component, as a tool takes it.
fn face(what: &str) -> Value {
    described(
        object(&[
            ("component", integer("The component's id: the last number of its path in model_summary")),
            ("face", optional(string("The face's label, such as Boss-Extrude1 · End cap"))),
            (
                "plane",
                optional(string("Or a reference plane: Top plane, Front plane, Right plane or a plane feature's name")),
            ),
            ("feature", optional(string("Or the name of the feature that made it"))),
            ("generator", optional(one_of(&["start_cap", "end_cap", "side"], "With feature: which of its faces"))),
            ("normal", optional(array(number("A direction"), "With a side: its outward normal [x, y, z]"))),
            ("edge", optional(array(string("A face label"), "Or the edge between two faces, by their labels"))),
            (
                "vertex",
                optional(array(string("A face label"), "Or the vertex where these faces meet, by their labels")),
            ),
        ]),
        what,
    )
}

fn measure_params() -> Vec<(&'static str, Value)> {
    vec![("a", face("The first face")), ("b", face("The second face"))]
}

fn origin_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("component", integer("The component's id")),
        ("align_axes", boolean("true matches its axes to the assembly's too")),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

const ALIGNMENT: &str = "aligned when the outward normals agree, opposed when they oppose";

fn distance_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("The face the distance is measured from, along its outward normal")),
        ("b", face("The face, axis or point the distance is measured to; an axis lies parallel to the plane")),
        ("distance", string("A length such as 5 mm or 1.2 cm")),
        ("alignment", one_of(&["aligned", "opposed"], ALIGNMENT)),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn coincident_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("The first planar face, plane, axis or point")),
        ("b", face("The one put on the first: in its plane, on its axis or on its point")),
        ("alignment", one_of(&["aligned", "opposed"], ALIGNMENT)),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn parallel_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("The first planar face or plane, or axis")),
        ("b", face("The one kept parallel to the first: two planes or two axes")),
        ("alignment", one_of(&["aligned", "opposed"], ALIGNMENT)),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn perpendicular_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("The first planar face or plane, or axis")),
        ("b", face("The one kept at right angles to the first: two planes or two axes")),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn width_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("groove", array(face("A face of the groove"), "The groove's two parallel faces, on one component")),
        ("tab", array(face("A face of the tab"), "The tab's two parallel faces, on another component")),
        (
            "alignment",
            one_of(&["aligned", "opposed"], "How the tab's first face's outward normal relates to the groove's first"),
        ),
        ("mode", optional(one_of(&["centered", "dimension", "percent"], "Where the tab sits; centered by default"))),
        ("distance", optional(string("With dimension: the gap from the first groove face to the tab face nearest it"))),
        ("percent", optional(number("With percent: that gap as a percentage of the play, 0 to 100"))),
        ("flipped", optional(boolean("true measures from the second groove face"))),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn concentric_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("The first cylinder, cone, sphere, circular or straight edge, or vertex")),
        ("b", face("The second, put on the first one's axis or centre")),
        ("alignment", optional(one_of(&["aligned", "opposed"], AXES))),
        ("lock_rotation", optional(boolean("true holds the turn about the axis where the solve leaves it"))),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

const AXES: &str = "For two axes: aligned when their directions agree, opposed when they oppose; aligned by default";

fn tangent_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("The planar face or plane")),
        ("b", face("The cylinder or sphere laid against it")),
        (
            "alignment",
            one_of(
                &["aligned", "opposed"],
                "How the curved face's outward normal at the contact relates to the plane's: opposed for a shaft lying on a table",
            ),
        ),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn hinge_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        (
            "axes",
            array(face("A cylinder, cone, circular or straight edge"), "Two axes put on one line, as a Concentric"),
        ),
        ("planes", array(face("A planar face or plane"), "Two planes put on one plane, as a Coincident")),
        ("alignment", optional(one_of(&["aligned", "opposed"], AXES))),
        ("plane_alignment", optional(one_of(&["aligned", "opposed"], ALIGNMENT))),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn lock_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", integer("The first component's id")),
        ("b", integer("The component locked where it is relative to the first")),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn value_params() -> Vec<(&'static str, Value)> {
    vec![("name", string("The value's name")), ("expression", string("An expression such as 5 mm or gap * 2"))]
}

fn drive_params() -> Vec<(&'static str, Value)> {
    vec![
        ("mate", string("The mate's name")),
        ("parameter", one_of(&["distance"], "What to drive")),
        ("expression", string("An expression over assembly values")),
        ("replace", optional(boolean("true replaces the driver the mate has; without it a second driver is refused"))),
    ]
}

fn edit_params() -> Vec<(&'static str, Value)> {
    vec![
        ("mate", string("The mate's name")),
        ("distance", optional(string("A new literal length"))),
        ("alignment", optional(one_of(&["aligned", "opposed"], ALIGNMENT))),
        ("align_axes", optional(boolean("For an Origin mate: whether the axes match too"))),
        ("percent", optional(number("For a Width by percent: the new percentage, 0 to 100"))),
        (
            "flipped",
            optional(boolean("For a Width by dimension or percent: true measures from the second groove face")),
        ),
        ("plane_alignment", optional(one_of(&["aligned", "opposed"], "For a Hinge: how its planes relate"))),
        ("lock_rotation", optional(boolean("For a Concentric between axes: whether the turn about the axis is held"))),
    ]
}

fn delete_params() -> Vec<(&'static str, Value)> {
    vec![("mate", string("The mate's name"))]
}

fn rename_params() -> Vec<(&'static str, Value)> {
    vec![("name", string("The value's name")), ("to", string("Its new name"))]
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let row = |id, label, doc| CommandSpec::new(id, label, Icon::Tree, doc).assembly();
    vec![
        row("assembly_mate", "Mate", "Tie two components by their faces, planes, axes, points or a lock")
            .on(Tab::Assembly, "Component", Size::Large)
            .run(|_| AppAction::Assembly(AssemblyAction::Mate(crate::mates::MateAction::Toggle)))
            .app_only("It opens the Mate panel; the assembly_mate tools add mates without it"),
        row("assembly_measure_planes", "Measure planes", "Prove the signed distance between two faces' planes").agents(
            Agents::Mcp,
            "Reads exact plane equations at the accepted revisions and changes nothing.",
            measure_params,
        ),
        row("assembly_mate_origin", "Origin mate", "Put a component's origin on the assembly's")
            .agents(
                Agents::Mcp,
                "An authored mate, not a fixed flag; a partial result is accepted unless strict.",
                origin_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_distance", "Distance mate", "Set the distance between two planar faces")
            .agents(
                Agents::Mcp,
                "Proves, solves and commits together, or refuses and changes nothing.",
                distance_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_coincident", "Coincident mate", "Put two planar faces on one plane")
            .agents(
                Agents::Mcp,
                "No distance of its own; proves, solves and commits together, or refuses and changes nothing.",
                coincident_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_parallel", "Parallel mate", "Keep two planar faces parallel")
            .agents(
                Agents::Mcp,
                "Fixes two turns and no translation; proves, solves and commits together, or refuses and changes nothing.",
                parallel_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_perpendicular", "Perpendicular mate", "Keep two planar faces at right angles")
            .agents(
                Agents::Mcp,
                "Fixes one turn; a turn it needs is the nearest quarter turn. Refuses and changes nothing when it cannot hold.",
                perpendicular_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_width", "Width mate", "Hold a tab between the two faces of a groove")
            .agents(
                Agents::Mcp,
                "Centred, at a gap or at a percentage of the play; the four faces must be parallel planes.",
                width_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_lock", "Lock mate", "Lock a component where it is relative to another")
            .agents(
                Agents::Mcp,
                "Stores the relative placement; a relative turn off the quarter turns is refused, never rounded.",
                lock_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_concentric", "Concentric mate", "Put two axes on one line, or a point on an axis")
            .agents(
                Agents::Mcp,
                "Axes and points are read exactly from the part's definitions; one off the world axes or not rational is refused.",
                concentric_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_tangent", "Tangent mate", "Lay a cylinder or sphere against a plane")
            .agents(
                Agents::Mcp,
                "The axis or centre sits one radius from the plane; the radius must be rational.",
                tangent_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_hinge", "Hinge mate", "Put two axes on one line and two planes on one plane")
            .agents(Agents::Mcp, "A Concentric and a Coincident under one name; no limits.", hinge_params)
            .undo(Undo::Step),
        row("assembly_set_value", "Set value", "Add or change an assembly value")
            .agents(Agents::Mcp, "Solves everything the value drives in the same step.", value_params)
            .undo(Undo::Step),
        row("assembly_rename_value", "Rename value", "Rename an assembly value")
            .agents(Agents::Mcp, "What reads the value holds its id, so nothing moves.", rename_params)
            .undo(Undo::Step),
        row("assembly_drive_mate", "Drive mate", "Bind a mate's distance to assembly values")
            .agents(Agents::Mcp, "A distance has one driver.", drive_params)
            .undo(Undo::Step),
        row("assembly_edit_mate", "Edit mate", "Change a mate's distance, alignment, axes, percentage, side or rotation lock")
            .agents(Agents::Mcp, "A driven distance is refused: edit the value that drives it.", edit_params)
            .undo(Undo::Step),
        row("assembly_delete_mate", "Delete mate", "Remove a mate and leave the components where they are")
            .agents(Agents::Mcp, "One undo step.", delete_params)
            .undo(Undo::Step),
    ]
}
