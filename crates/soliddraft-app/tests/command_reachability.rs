//! Proves that removing the legacy 7-item top menu row (File/Edit/View/Insert/Tools/Window/Help)
//! leaves every single app command reachable via the D1 Clear Bench surfaces:
//! the single File menu, top bar, top ribbon, context menus, or command search/palette.

use soliddraft_app::command_line;
use soliddraft_app::commands::{self, Context, History, Tab};
use soliddraft_app::context_menu;
use soliddraft_app::top_bar::{FILE_MENU_COMMANDS, TOP_BAR_COMMANDS};
use std::collections::HashSet;

/// Every command id that the removed 7-item `File / Edit / View / Insert / Tools / Window / Help`
/// menu bar reached before D1 Clear Bench.
const LEGACY_MENU_COMMANDS: &[&str] = &[
    // Legacy File
    "new_part",
    "open",
    "import_step",
    "import_iges",
    "import_mesh",
    "import_report",
    "import_scan",
    "scan_workspace",
    "make_drawing",
    "new_assembly",
    "make_assembly",
    "assembly_save",
    "assembly_save_as",
    "insert_component",
    "assembly_pack_and_go",
    "assembly_pack_and_go_zip",
    "assembly_export_urdf",
    "assembly_export_usd",
    "save",
    "save_as",
    "export_step",
    "export_iges",
    "export_stl",
    "export_3mf",
    "export_obj",
    "quit",
    // Legacy Edit
    "drawing_rebuild",
    "undo",
    "redo",
    "edit_feature",
    "rename",
    "suppress",
    "delete_feature",
    "roll_to_end",
    // Legacy View
    "view_isometric",
    "view_front",
    "view_top",
    "view_right",
    "view_back",
    "view_left",
    "view_bottom",
    "normal_to",
    "zoom_fit",
    "orthographic",
    "toggle_tree",
    // Legacy Tools
    "measure",
    "check",
    "ask",
    "check_model",
    "explain",
    "summarise",
    "find_issues",
    "provider_settings",
    "auto_apply",
    "file_locations",
    "settings",
    // Legacy Window
    "reset_layout",
    "show_drawing",
    "command_palette",
    // Legacy Help
    "shortcuts",
    "give_feedback",
    "about",
];

fn all_surface_commands() -> HashSet<&'static str> {
    let mut set = HashSet::new();
    set.extend(FILE_MENU_COMMANDS.iter().copied());
    set.extend(TOP_BAR_COMMANDS.iter().copied());
    for tab in Tab::all() {
        for (_, items) in commands::ribbon(tab) {
            for (cmd, _) in items {
                set.insert(cmd.id);
            }
        }
    }
    for (_, spec) in &context_menu::MENUS {
        for group in spec.groups {
            set.extend(group.commands.iter().copied());
        }
        for quick in &spec.quick {
            set.insert(quick.command);
        }
    }
    set
}

#[test]
fn every_app_command_is_reachable_without_legacy_menu_bar() {
    let ui_surfaces = all_surface_commands();
    let history = History::default();

    // 1. Every command listed in the new File menu and top bar exists in the registry.
    for id in FILE_MENU_COMMANDS.iter().chain(TOP_BAR_COMMANDS.iter()) {
        assert!(commands::get(id).is_some(), "top bar / File menu references unknown command {id}");
    }

    // 2. Every single app command in the registry is reachable through command search /
    // command line in at least one valid context.
    for cmd in commands::all().iter().filter(|c| c.in_app()) {
        let by_name =
            Context::all().into_iter().any(|ctx| command_line::named(cmd.id, ctx).is_some_and(|c| c.id == cmd.id));
        let by_search = Context::all()
            .into_iter()
            .any(|ctx| commands::search_here(cmd.id, &history, ctx).iter().any(|m| m.command.id == cmd.id));
        assert!(
            by_name || by_search || ui_surfaces.contains(cmd.id),
            "app command `{}` ({}) is unreachable without the legacy menu bar",
            cmd.id,
            cmd.label
        );
    }

    // 3. Every command from the removed 7-item File/Edit/View/Insert/Tools/Window/Help row
    // stays reachable via File menu, top bar, ribbon, context menus, or command search.
    for id in LEGACY_MENU_COMMANDS {
        let cmd = commands::get(id).unwrap_or_else(|| panic!("legacy menu command `{id}` missing from registry"));
        assert!(cmd.in_app(), "legacy menu command `{id}` must remain an app command");
        let in_search = Context::all().into_iter().any(|ctx| command_line::named(id, ctx).is_some_and(|c| c.id == *id));
        assert!(
            ui_surfaces.contains(id) || in_search,
            "legacy menu command `{id}` became unreachable after removing the 7-item menu row"
        );
    }
}
