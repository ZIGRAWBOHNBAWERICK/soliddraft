//! Pack and Go and export command registry rows.

use crate::assembly::AssemblyAction;
use crate::assembly_files::FilesAction;
use crate::commands::{Agents, CommandSpec, Size, Tab, Undo};
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

fn drawing_params() -> Vec<(&'static str, Value)> {
    vec![
        ("template", optional(string("A drawing template id, such as iso-a3; iso-a3 when left out"))),
        (
            "views",
            optional(soliddraft_assistant::tools::schema::array(
                string("isometric, front, top, right, left, back or bottom"),
                "The views, side by side, the first ballooned; the isometric alone when exploded, else isometric and front",
            )),
        ),
        ("exploded", optional(boolean("Show the exploded view; whenever the assembly has one when left out"))),
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
            "assembly_make_drawing",
            "Make drawing",
            Icon::Cad("drawing"),
            "Make a drawing of the assembly with a bill of materials and balloons",
        )
        .on(Tab::Assembly, "File", Size::Small)
        .run(|_| AppAction::Assembly(AssemblyAction::Files(FilesAction::MakeDrawing)))
        .agents(
            Agents::Mcp,
            "The assembly must be saved. Views come from the parts' display meshes with hidden lines removed across \
             parts; the BOM lists parts with quantities; each row gets a balloon on the first view. The drawing \
             opens; Rebuild brings it to the assembly later.",
            drawing_params,
        )
        .undo(Undo::Replaces),
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
        .when(|s| s.assembly.is_active() && !cfg!(target_arch = "wasm32"))
        .hint(|_| cfg!(target_arch = "wasm32").then(|| super::BROWSER_EXPORT.to_owned()))
        .agents(Agents::Mcp, JOINTS, urdf_params),
        row(
            "assembly_export_usd",
            "Export USD",
            Icon::Import,
            "Write the assembly as OpenUSD with UsdPhysics joints from its mates",
        )
        .on(Tab::Assembly, "File", Size::Small)
        .run(|_| AppAction::Assembly(AssemblyAction::Files(FilesAction::Export("usda"))))
        .when(|s| s.assembly.is_active() && !cfg!(target_arch = "wasm32"))
        .hint(|_| cfg!(target_arch = "wasm32").then(|| super::BROWSER_EXPORT.to_owned()))
        .agents(Agents::Mcp, JOINTS, usd_params),
    ]
}
