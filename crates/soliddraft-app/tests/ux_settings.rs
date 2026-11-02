//! Settings through the app's command line, menu and saved navigation choices.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::vec2;
use egui_kittest::kittest::{NodeT, Queryable};
use soliddraft_app::{command_line, commands, preferences, settings};

#[test]
fn settings_command_opens_preferences_and_saves_navigation() {
    let dir = std::env::temp_dir().join(format!("soliddraft-ux-settings-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.clone()));
    let mut app = ribbon::app(ribbon::blank());
    for context in commands::Context::all() {
        for name in ["settings", "Preferences"] {
            let command = command_line::named(name, context).unwrap();
            assert!(command.available(context), "{name} works in every document");
        }
    }

    ribbon::type_command(&mut app, "settings");
    app.run_steps(3);
    assert_eq!(app.state().preferences.as_ref().unwrap().page, preferences::MOUSE);
    app.get_by_label("Onshape").click();
    app.run_steps(3);
    assert_eq!(app.state().state.navigation.preset_id, "onshape");
    assert_eq!(settings::navigation(settings::InstallKind::Existing).preset_id, "onshape");
    assert!(app.state().state.doc.features.is_empty());
    if let Some(proof) = std::env::var_os("SDC_SETTINGS_PROOF_DIR") {
        app.render().unwrap().save(std::path::Path::new(&proof).join("settings-desktop.png")).unwrap();
    }

    app.get_by_label("Interface · Coming soon").click();
    app.run_steps(3);
    assert_eq!(app.state().preferences.as_ref().unwrap().page, "interface");
    assert!(app.query_by_label("Coming soon").is_some());
    assert!(app.query_by_label("Navigation preset").is_none());

    app.get_by_label("Close window").click();
    app.run_steps(3);
    assert!(app.state().preferences.is_none());
    ribbon::type_command(&mut app, "Preferences");
    app.run_steps(3);
    assert_eq!(app.state().preferences.as_ref().unwrap().draft.preset_id, "onshape");
    app.get_by_label("Close window").click();
    app.run_steps(3);
    ribbon::menu(&mut app, "Tools", "Preferences…");
    app.run_steps(3);
    assert!(app.state().preferences.is_some());
    app.set_size(vec2(1024.0, 768.0));
    app.run_steps(5);
    assert!(app.query_by_label("Search settings").is_some());
    if let Some(proof) = std::env::var_os("SDC_SETTINGS_PROOF_DIR") {
        app.render().unwrap().save(std::path::Path::new(&proof).join("settings-compact.png")).unwrap();
    }
    app.get_by_label("Search settings").click();
    app.run_steps(2);
    app.get_by_label("Search settings").type_text("assistant");
    app.run_steps(3);
    app.get_by_label("Assistant settings · Coming soon").click();
    app.run_steps(3);
    assert_eq!(app.state().preferences.as_ref().unwrap().page, "assistant");
    assert!(app.query_by_label("Coming soon").is_some());
    if let Some(proof) = std::env::var_os("SDC_SETTINGS_PROOF_DIR") {
        app.render().unwrap().save(std::path::Path::new(&proof).join("settings-coming-soon.png")).unwrap();
    }
    app.render().expect("settings render in the real app");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
