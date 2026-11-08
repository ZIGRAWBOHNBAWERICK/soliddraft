//! Interference command registry rows.

use crate::assembly::AssemblyAction;
use crate::commands::{Agents, CommandSpec, Size, Tab};
use crate::icons::Icon;
use crate::interference::InterferenceAction;
use crate::state::AppAction;
use serde_json::Value;
use soliddraft_assistant::tools::schema::{array, boolean, number, optional, string};

fn scope_params() -> Vec<(&'static str, Value)> {
    vec![
        (
            "paths",
            optional(array(
                string("An occurrence's path, such as \"4\" or \"6/2\""),
                "Compare only the parts at or under these occurrences; without it, every part",
            )),
        ),
        ("include_hidden", optional(boolean("true compares hidden parts too; suppressed parts never take part"))),
        ("chord_mm", optional(number("The chord bound asked of the kernel, 0.001 to 1 mm; 0.01 when left out"))),
    ]
}

fn clearance_params() -> Vec<(&'static str, Value)> {
    let mut params = vec![("clearance", string("The least distance wanted, such as 0.5 mm"))];
    params.extend(scope_params());
    params
}

fn hole_params() -> Vec<(&'static str, Value)> {
    let mut params = vec![("max_deviation", string("Maximum parallel hole-axis deviation, such as 0.5 mm"))];
    params.extend(scope_params().into_iter().filter(|(name, _)| *name != "chord_mm"));
    params
}

pub(crate) fn commands() -> Vec<CommandSpec> {
    let row = |id, label, icon, doc| CommandSpec::new(id, label, icon, doc).assembly();
    vec![
        row(
            "assembly_check_hole_alignment",
            "Check hole alignment",
            Icon::Cad("interference"),
            "Compare internal cylinder axes on the pinned parts without moving them",
        )
        .agents(Agents::Mcp, "Exact coaxial or parallel offset axes within max_deviation; unsupported cylinders are listed as unchecked. Does not test diameter or fit.", hole_params),
        row(
            "assembly_interference",
            "Interference",
            Icon::Cad("interference"),
            "Check parts for interference or a clearance, proven on certified meshes",
        )
        .on(Tab::Assembly, "Component", Size::Large)
        .run(|_| AppAction::Assembly(AssemblyAction::Interference(InterferenceAction::Toggle)))
        .lit(|s| s.assembly.interference.open)
        .app_only("It opens the Interference panel; the assembly_check tools run the same checks without it"),
        row(
            "assembly_check_interference",
            "Check interference",
            Icon::Cad("interference"),
            "Prove each pair of parts clear, interfering, or touching or undecided",
        )
        .agents(
            Agents::Mcp,
            "Rebuilds each part at its pinned revision and proves on its certified mesh; changes nothing.",
            scope_params,
        ),
        row(
            "assembly_check_clearance",
            "Check clearance",
            Icon::Cad("interference"),
            "Prove which pairs of parts are at or under a clearance",
        )
        .agents(
            Agents::Mcp,
            "Each pair is proven to meet the clearance, proven at or under it, or undecided; changes nothing.",
            clearance_params,
        ),
    ]
}
