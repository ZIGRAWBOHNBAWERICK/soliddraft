//! Pack and Go and export command registry rows.

use crate::assembly::AssemblyAction;
use crate::assembly_files::FilesAction;
use crate::commands::{Agents, CommandSpec, Size, Tab};
use crate::icons::Icon;
use crate::state::AppAction;
use serde_json::Value;
use soliddraft_assistant::tools::schema::{boolean, optional, string};

fn pack_params() -> Vec<(&'static str, Value)> {
    vec![
        ("folder", string("The destination folder; it must not exist or be empty")),
        ("prefix", optional(string("Put before every copied file's name"))),
        ("suffix", optional(string("Put after every copied file's name, before its extension"))),
        ("flatten", optional(boolean("true puts every file in the folder itself"))),
        ("drawings", optional(boolean("true also packs drawings in the assembly's folder whose model is packed"))),
    ]
}

fn urdf_params() -> Vec<(&'static str, Value)> {
    vec![("path", string("The file to write, ending in .urdf; its meshes go in a folder beside it"))]
}

fn usd_params() -> Vec<(&'static str, Value)> {
    vec![("path", string("The file to write, ending in .usda"))]
}

const JOINTS: &str = "Every part is a link. A Hinge becomes a revolute joint (continuous without limits), a \
                      Concentric between axes a continuous one, a Gear a mimic joint, anything else a fixed joint \
                      at the assembly's pose; notes name what was not carried. The assembly does not change.";

pub(crate) fn commands() -> Vec<CommandSpec> {
    let row = |id, label, icon, doc| CommandSpec::new(id, label, icon, doc).assembly();
    vec![
        row(
            "assembly_pack_and_go",
            "Pack and Go",
            Icon::Save,
            "Copy the assembly and every file it references into one new folder",
        )
        .on(Tab::Assembly, "File", Size::Small)
        .run(|_| AppAction::Assembly(AssemblyAction::Files(FilesAction::PackAndGo)))
        .hint(|_| cfg!(target_arch = "wasm32").then(|| super::BROWSER_PACK.to_owned()))
        .agents(
            Agents::Mcp,
            "The assembly must be saved. Parts are copied as they are, assemblies written again pointing at the \
             copies, ids kept. Nothing is written unless every reference is found; nothing is replaced.",
            pack_params,
        ),
        row(
            "assembly_export_urdf",
            "Export URDF",
            Icon::Import,
            "Write the assembly as a URDF robot with joints from its mates",
        )
        .on(Tab::Assembly, "File", Size::Small)
        .run(|_| AppAction::Assembly(AssemblyAction::Files(FilesAction::Export("urdf"))))
        .agents(Agents::Mcp, JOINTS, urdf_params),
        row(
            "assembly_export_usd",
            "Export USD",
            Icon::Import,
            "Write the assembly as OpenUSD with UsdPhysics joints from its mates",
        )
        .on(Tab::Assembly, "File", Size::Small)
        .run(|_| AppAction::Assembly(AssemblyAction::Files(FilesAction::Export("usda"))))
        .agents(Agents::Mcp, JOINTS, usd_params),
    ]
}
