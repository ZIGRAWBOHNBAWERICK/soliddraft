//! Configuration and exploded view command registry rows.

use crate::assembly::AssemblyAction;
use crate::commands::{Agents, CommandSpec, Size, Tab, Undo};
use crate::configurations::ConfigureAction;
use crate::icons::Icon;
use crate::state::AppAction;
use serde_json::Value;
use soliddraft_assistant::tools::schema::{array, boolean, integer, non_empty_array, number, optional, string};

fn name_params() -> Vec<(&'static str, Value)> {
    vec![("name", string("The configuration's name"))]
}

fn rename_params() -> Vec<(&'static str, Value)> {
    vec![("name", string("The configuration's name")), ("to", string("Its new name"))]
}

fn direction() -> Value {
    array(number("A direction component"), "[x, y, z] in the assembly's frame; need not be a unit vector")
}

fn step_params() -> Vec<(&'static str, Value)> {
    vec![
        (
            "paths",
            non_empty_array(
                string("An occurrence's path, such as \"4\" or \"6/2\""),
                "What moves, with everything under it",
            ),
        ),
        ("direction", direction()),
        ("distance", number("How far, in millimetres")),
    ]
}

fn edit_step_params() -> Vec<(&'static str, Value)> {
    vec![
        ("step", integer("The step's id, from model_summary's assembly.explode.steps")),
        ("direction", optional(direction())),
        ("distance", optional(number("How far, in millimetres"))),
    ]
}

fn delete_step_params() -> Vec<(&'static str, Value)> {
    vec![("step", integer("The step's id"))]
}

fn auto_params() -> Vec<(&'static str, Value)> {
    vec![("distance", optional(number("How far each component moves, in millimetres; 50 when left out")))]
}

fn explode_params() -> Vec<(&'static str, Value)> {
    vec![("exploded", boolean("true shows the exploded view, false the assembled positions"))]
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let row = |id, label, icon, doc| CommandSpec::new(id, label, icon, doc).assembly();
    vec![
        row(
            "assembly_configurations",
            "Configurations",
            Icon::Config,
            "Keep named states of the assembly and an exploded view in each",
        )
        .on(Tab::Assembly, "Component", Size::Small)
        .run(|_| AppAction::Assembly(AssemblyAction::Configure(ConfigureAction::Toggle)))
        .lit(|s| s.assembly.configure.open)
        .agents(
            Agents::Mcp,
            "Lists the configurations, the active one and the exploded view's steps; changes nothing. The panel \
             adds, activates and deletes them.",
            Vec::new,
        ),
        row(
            "assembly_explode",
            "Explode",
            Icon::Cad("explode"),
            "Show the exploded view, or the assembled positions again",
        )
        .on(Tab::Assembly, "View", Size::Large)
        .on(Tab::AssemblyView, "Display", Size::Small)
        .run(|s| {
            let on = s.assembly.session.as_ref().is_some_and(|x| x.doc.is_exploded());
            AppAction::Assembly(AssemblyAction::Configure(ConfigureAction::Explode(!on)))
        })
        .lit(|s| s.assembly.session.as_ref().is_some_and(|x| x.doc.is_exploded()))
        .agents(
            Agents::Mcp,
            "Display only: placements, the revision and undo do not change. Answers with where each moved \
             occurrence shows.",
            explode_params,
        ),
        row(
            "assembly_add_configuration",
            "Add configuration",
            Icon::Config,
            "Keep the assembly's state now as a new configuration",
        )
        .agents(Agents::Mcp, "The first one also adds Default, active, for the state before.", name_params)
        .undo(Undo::Step),
        row(
            "assembly_activate_configuration",
            "Activate configuration",
            Icon::Config,
            "Show another configuration's suppression, placements, values and exploded view",
        )
        .agents(Agents::Mcp, "The configuration that was active keeps the state the assembly had.", name_params)
        .undo(Undo::Step),
        row("assembly_rename_configuration", "Rename configuration", Icon::Config, "Rename a configuration")
            .agents(Agents::Mcp, "Names are unique.", rename_params)
            .undo(Undo::Step),
        row("assembly_delete_configuration", "Delete configuration", Icon::Config, "Delete an inactive configuration")
            .agents(Agents::Mcp, "The active one cannot be deleted; with one left the assembly has none.", name_params)
            .undo(Undo::Step),
        row(
            "assembly_explode_step",
            "Explode step",
            Icon::Cad("explode"),
            "Move occurrences apart in the exploded view",
        )
        .agents(
            Agents::Mcp,
            "Adds a step to the active configuration's exploded view, after the others. Display only.",
            step_params,
        )
        .undo(Undo::Step),
        row("assembly_edit_explode_step", "Edit explode step", Icon::Cad("explode"), "Change an explode step")
            .agents(Agents::Mcp, "Changes its direction or distance.", edit_step_params)
            .undo(Undo::Step),
        row("assembly_delete_explode_step", "Delete explode step", Icon::Cad("explode"), "Delete an explode step")
            .agents(Agents::Mcp, "Deletes one step.", delete_step_params)
            .undo(Undo::Step),
        row(
            "assembly_auto_explode",
            "Auto explode",
            Icon::Cad("explode"),
            "Move every shown component straight away from the middle",
        )
        .agents(
            Agents::Mcp,
            "Replaces the exploded view with one step per shown component; the one nearest the middle stays.",
            auto_params,
        )
        .undo(Undo::Step),
    ]
}
