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

fn unsupported_motion_params() -> Vec<(&'static str, Value)> {
    Vec::new()
}

fn angle_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("First plane normal or axis")),
        ("b", face("Second plane normal or axis")),
        ("reference", face("Explicit positive rotation direction: a plane normal or axis")),
        ("degrees", number("Signed degrees from -180 to 180")),
        ("require_fully_constrained", optional(boolean("Refuse remaining freedom"))),
    ]
}
fn limit_angle_params() -> Vec<(&'static str, Value)> {
    let mut out = angle_params();
    out.retain(|(key, _)| *key != "degrees");
    out.extend([
        ("min_degrees", number("Inclusive least signed degrees")),
        ("max_degrees", number("Inclusive greatest signed degrees")),
    ]);
    out
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
        (
            "mode",
            optional(one_of(
                &["free", "centered", "dimension", "percent"],
                "Where the tab sits: anywhere between the groove's faces, centred (the default), at a gap or a percentage",
            )),
        ),
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
        ("angle_limits", optional(array(number("Signed degrees"), "Optional inclusive [min,max] angular stops"))),
        ("alignment", optional(one_of(&["aligned", "opposed"], AXES))),
        ("plane_alignment", optional(one_of(&["aligned", "opposed"], ALIGNMENT))),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn symmetric_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("The first planar face, plane, axis or point")),
        ("b", face("Its mirror image on another component: the same kind")),
        ("plane", face("The symmetry plane: a planar face or a reference plane of any component")),
        (
            "alignment",
            optional(one_of(
                &["aligned", "opposed"],
                "For planes or axes: aligned when b's normal or direction is a's mirrored, opposed when it is reversed; aligned by default",
            )),
        ),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn linear_coupler_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("The first part's planar face or axis: its normal or direction is the way it slides")),
        ("b", face("The second part's, on another component")),
        ("reference_a", optional(integer("The component a slides relative to; the assembly by default"))),
        ("reference_b", optional(integer("The component b slides relative to; the assembly by default"))),
        (
            "ratio",
            optional(array(
                number("A distance"),
                "[first, second]: a slides first while b slides second; [1, 1] by default",
            )),
        ),
        ("reverse", optional(boolean("true slides b the other way"))),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn limit_distance_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("The face the distance is measured from, along its outward normal")),
        ("b", face("The face, axis or point the distance is measured to")),
        ("min", string("The least distance, such as 0 mm")),
        ("max", string("The greatest distance, such as 10 mm")),
        ("alignment", one_of(&["aligned", "opposed"], ALIGNMENT)),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn profile_center_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("a", face("A planar face bounded by a rectangle, a regular polygon or a circle, or a circular edge")),
        ("b", face("Another of the same kind, centred on the first")),
        ("alignment", one_of(&["aligned", "opposed"], ALIGNMENT)),
        ("distance", optional(string("The distance between their planes, such as 2 mm; 0 mm by default"))),
        ("lock_rotation", optional(boolean("For circles: true holds the turn where the solve leaves it"))),
        ("require_fully_constrained", optional(boolean("true refuses a result with any freedom left"))),
    ]
}

fn slot_params() -> Vec<(&'static str, Value)> {
    vec![
        ("name", string("A name for the mate")),
        ("slot", face("A wall or a round end of a straight slot")),
        ("pin", face("The pin's cylinder, or a circular or straight edge, on another component")),
        (
            "alignment",
            optional(one_of(
                &["aligned", "opposed"],
                "How the pin's direction relates to the slot's round ends' axes; aligned by default",
            )),
        ),
        (
            "mode",
            optional(one_of(
                &["free", "center", "distance", "percent"],
                "Where the pin sits along the slot: anywhere between the ends (the default), centred, at a distance or a percentage",
            )),
        ),
        ("distance", optional(string("With distance: from the first end's centre, such as 4 mm"))),
        ("percent", optional(number("With percent: of the run between the ends' centres, 0 to 100"))),
        ("flipped", optional(boolean("true measures from the second end"))),
        ("lock_rotation", optional(boolean("true holds the pin's turn where the solve leaves it"))),
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
        ("degrees", optional(number("New signed Angle degrees"))),
        ("angle_limits", optional(array(number("Signed degrees"), "New [min_degrees, max_degrees]"))),
        ("mate", string("The mate's name")),
        ("distance", optional(string("A new literal length"))),
        ("alignment", optional(one_of(&["aligned", "opposed"], ALIGNMENT))),
        ("align_axes", optional(boolean("For an Origin mate: whether the axes match too"))),
        ("percent", optional(number("For a Width or a Slot by percent: the new percentage, 0 to 100"))),
        (
            "flipped",
            optional(boolean(
                "For a Width or a Slot by dimension, distance or percent: true measures from the second face or end",
            )),
        ),
        ("plane_alignment", optional(one_of(&["aligned", "opposed"], "For a Hinge: how its planes relate"))),
        (
            "lock_rotation",
            optional(boolean(
                "For a Concentric between axes, a Profile Center between circles or a Slot: whether the turn is held",
            )),
        ),
        ("min", optional(string("For a Limit Distance: the new least distance"))),
        ("max", optional(string("For a Limit Distance: the new greatest distance"))),
        (
            "ratio",
            optional(array(
                number("A distance"),
                "For a Linear Coupler: the new ratio; the parts' places become its zero",
            )),
        ),
        ("reverse", optional(boolean("For a Linear Coupler: whether b slides the other way"))),
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
        row("assembly_drag", "Move and rotate under mates", "Preview a component placement while mates hold")
            .agents(Agents::Mcp, "Begin, update and commit one drag, or cancel it. Move performs one complete drag. Limits act as stops.", drag_params).undo(Undo::Carried),
        row("assembly_mate_gear", "Gear mate", "Unavailable in the static mate solver").agents(Agents::Mcp, "Returns a named unsupported_motion refusal.", unsupported_motion_params),
        row("assembly_mate_rack_and_pinion", "Rack and Pinion mate", "Unavailable in the static mate solver").agents(Agents::Mcp, "Returns a named unsupported_motion refusal.", unsupported_motion_params),
        row("assembly_mate_screw", "Screw mate", "Unavailable in the static mate solver").agents(Agents::Mcp, "Returns a named unsupported_motion refusal.", unsupported_motion_params),
        row("assembly_mate_path", "Path mate", "Unavailable in the static mate solver").agents(Agents::Mcp, "Returns a named unsupported_motion refusal.", unsupported_motion_params),
        row("assembly_mate_cam", "Cam mate", "Unavailable in the static mate solver").agents(Agents::Mcp, "Returns a named unsupported_motion refusal.", unsupported_motion_params),
        row("assembly_mate_universal_joint", "Universal Joint mate", "Unavailable in the static mate solver").agents(Agents::Mcp, "Returns a named unsupported_motion refusal.", unsupported_motion_params),
        row("assembly_physical_dynamics", "Physical Dynamics", "Unavailable in the static mate solver").agents(Agents::Mcp, "Returns a named unsupported_motion refusal.", unsupported_motion_params),
        row("assembly_mate_angle", "Angle mate", "Set a signed angle about an explicit reference direction")
            .agents(Agents::Mcp, "Rotates projected directions about the reference; certified bounds are reported per mate.", angle_params).undo(Undo::Step),
        row("assembly_mate_limit_angle", "Limit angle mate", "Bound a signed angle about an explicit reference direction")
            .agents(Agents::Mcp, "Leaves angular motion free between inclusive bounds.", limit_angle_params).undo(Undo::Step),
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
        row("assembly_mate_symmetric", "Symmetric mate", "Mirror a face, axis or point of one component onto another's")
            .agents(
                Agents::Mcp,
                "About a plane of any component; two planes, axes or points of one kind, exact or refused.",
                symmetric_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_linear_coupler", "Linear Coupler mate", "Couple one component's slide to another's by a ratio")
            .agents(
                Agents::Mcp,
                "The parts' places when it is made are its zero; the relation is solved exactly, across axes too.",
                linear_coupler_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_limit_distance", "Limit Distance mate", "Hold a distance between a least and a greatest value")
            .agents(
                Agents::Mcp,
                "Keeps the faces parallel and leaves the slide free within the range; a move past an end stops there.",
                limit_distance_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_profile_center", "Profile Center mate", "Centre two rectangles, regular polygons or circles")
            .agents(
                Agents::Mcp,
                "The centre is read exactly from the outline's corners or circle; a rounded outline is refused.",
                profile_center_params,
            )
            .undo(Undo::Step),
        row("assembly_mate_slot", "Slot mate", "Put a pin in a straight slot")
            .agents(
                Agents::Mcp,
                "Free, centred, at a distance or a percentage along the slot; arc slots are refused.",
                slot_params,
            )
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
        row("assembly_edit_mate", "Edit mate", "Change a mate's distance, limits, ratio, alignment, axes, percentage, side or rotation lock")
            .agents(Agents::Mcp, "A driven distance is refused: edit the value that drives it.", edit_params)
            .undo(Undo::Step),
        row("assembly_delete_mate", "Delete mate", "Remove a mate and leave the components where they are")
            .agents(Agents::Mcp, "One undo step.", delete_params)
            .undo(Undo::Step),
    ]
}

fn drag_params() -> Vec<(&'static str, Value)> {
    vec![
        ("phase", optional(one_of(&["move", "begin", "update", "commit", "cancel"], "Drag phase; defaults to move"))),
        ("path", optional(string("Occurrence path, required for begin or move"))),
        (
            "placement",
            optional(
                serde_json::json!({"type":"object", "description":"Target in the parent frame, required for update or move", "properties":{"translation":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"rotation":{"type":"array","items":{"type":"number"},"minItems":4,"maxItems":4}},"required":["translation","rotation"],"additionalProperties":false}),
            ),
        ),
    ]
}
