//! Where Preferences keep what the user chose: the settings file on the desktop, the
//! browser's own storage on the web. Each case runs the real Preferences window against
//! the app's state, then starts a new state from the same store, as a restart or a page
//! reload does. The browser's storage is stood in for by an in-memory shelf, one per
//! browser; `ci/browser-preferences.mjs` runs the same flow in Chromium on the web build.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use soliddraft_app::navigation::{self, Action, NavEvent, Outcome, Router};
use soliddraft_app::preferences::{self, Preferences};
use soliddraft_app::settings::store::{self, Memory, NotKept, Store};
use soliddraft_app::settings::{self, GridStyle};
use soliddraft_app::state::AppState;
use std::sync::{Arc, Mutex, MutexGuard};

/// The store is global to the process.
static SERIAL: Mutex<()> = Mutex::new(());

struct Session {
    _serial: MutexGuard<'static, ()>,
}

impl Session {
    /// Starts the app on `store`, as a launch or a page load does.
    fn on(store: Store) -> Session {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // The recent-files list and the rest of the config directory are the test's own.
        let config = match &store {
            Store::File(file) => file.parent().unwrap().to_owned(),
            Store::Shelf(_) => std::env::temp_dir().join(format!("soliddraft-settings-storage-{}", std::process::id())),
        };
        std::fs::create_dir_all(&config).unwrap();
        soliddraft_app::file::set_config_dir(Some(config));
        store::use_store(Some(store));
        settings::capture_install_kind();
        Session { _serial: serial }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        store::use_store(None);
        soliddraft_app::file::set_config_dir(None);
    }
}

/// A store in a folder of the test's own.
fn file_store(name: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("soliddraft-settings-storage-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Store::File(dir.join("settings.json"))
}

fn browser(shelf: &Arc<Memory>) -> Store {
    Store::Shelf(shelf.clone())
}

type App = Harness<'static, (Option<Preferences>, AppState)>;

/// The app's state as a start reads it, with Preferences open as the app opens it.
fn launch() -> App {
    let mut state = AppState::new(soliddraft_app::new_document());
    let view = settings::view();
    state.viewport.references = view.references;
    state.viewport.grid_style = view.grid;
    let prefs = Preferences::new(&state.navigation, state.viewport.references, state.viewport.grid_style)
        .with_shortcuts(&state.shortcuts);
    let mut app = Harness::builder().with_size(vec2(1200.0, 820.0)).build_ui_state(
        |ui, (prefs, state): &mut (Option<Preferences>, AppState)| preferences::window(ui.ctx(), prefs, state),
        (Some(prefs), state),
    );
    app.run_steps(2);
    app
}

fn state(app: &App) -> &AppState {
    &app.state().1
}

fn message(app: &App) -> Option<String> {
    app.state().0.as_ref().and_then(|p| p.message.as_ref()).map(|m| m.text.clone())
}

fn choose_preset(app: &mut App, label: &str) {
    app.get_by_role_and_label(Role::RadioButton, label).click();
    app.run_steps(3);
}

/// What a plain middle drag does under `nav`.
fn middle_drag(nav: &navigation::NavigationSettings) -> Outcome {
    let mut r = Router::new(nav, false);
    r.handle(NavEvent::Press { button: navigation::Button::Middle, pos: egui::Pos2::ZERO, inside: true, time: 0.0 });
    r.handle(NavEvent::Move(egui::Pos2::new(0.0, 20.0))).outcome
}

/// The stored settings, read back as text the way the store holds them.
fn stored(store: &Store) -> serde_json::Value {
    store.load()
}

/// Chooses SOLIDWORKS: it applies at once, is stored, and a new start keeps it.
fn solidworks_survives_a_restart(store: Store) {
    let session = Session::on(store.clone());
    let mut app = launch();
    assert_eq!(state(&app).navigation.active_preset().id, "fusion", "a new installation starts on Fusion");
    assert_eq!(middle_drag(&state(&app).navigation), Outcome::Action(Action::Pan), "Fusion pans on a middle drag");

    choose_preset(&mut app, "SOLIDWORKS");
    assert_eq!(state(&app).navigation.active_preset().id, "solidworks", "{:?}", message(&app));
    assert_eq!(middle_drag(&state(&app).navigation), Outcome::Action(Action::Orbit), "SOLIDWORKS orbits at once");
    assert_eq!(message(&app).as_deref(), Some("Navigation now follows SOLIDWORKS."));
    assert_eq!(stored(&store)["navigation"]["preset_id"], "solidworks");
    drop(app);
    drop(session);

    let _session = Session::on(store.clone());
    let app = launch();
    assert_eq!(state(&app).navigation.active_preset().id, "solidworks", "the start after keeps SOLIDWORKS");
    assert_eq!(middle_drag(&state(&app).navigation), Outcome::Action(Action::Orbit));
}

#[test]
fn a_preset_survives_a_restart_on_the_desktop() {
    solidworks_survives_a_restart(file_store("preset"));
}

#[test]
fn a_preset_survives_a_reload_in_the_browser() {
    let shelf = Memory::new();
    solidworks_survives_a_restart(browser(&shelf));
    let text = shelf.text().expect("the browser holds the settings");
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(json["navigation"]["preset_id"], "solidworks", "one JSON text, as the file holds it");
}

#[test]
fn fusion_view_keys_survive_a_reload_in_the_browser() {
    let shelf = Memory::new();
    {
        let _session = Session::on(browser(&shelf));
        let mut app = launch();
        choose_preset(&mut app, "SOLIDWORKS");
        app.get_by_label("View keys").click();
        app.run_steps(3);
        choose_preset(&mut app, "Fusion 360");
        assert_eq!(state(&app).navigation.keyboard_profile, "fusion", "{:?}", message(&app));
        assert_eq!(message(&app).as_deref(), Some("View keys now follow Fusion 360."));
    }
    let _session = Session::on(browser(&shelf));
    let app = launch();
    assert_eq!(state(&app).navigation.keyboard_profile, "fusion");
    assert_eq!(state(&app).navigation.active_preset().id, "solidworks", "the mouse preset stays its own choice");
    let keys = navigation::view_keys::in_use(&state(&app).navigation);
    assert!(keys.rows.iter().any(|(b, _)| b.action == Action::Fit && b.text(false) == "F6"), "Fusion fits on F6");
}

#[test]
fn two_browsers_never_see_each_others_settings() {
    let (first, second) = (Memory::new(), Memory::new());
    {
        let _session = Session::on(browser(&first));
        let mut app = launch();
        choose_preset(&mut app, "SOLIDWORKS");
    }
    {
        let _session = Session::on(browser(&second));
        let mut app = launch();
        assert_eq!(state(&app).navigation.active_preset().id, "fusion", "the other browser starts afresh");
        assert_eq!(second.text(), None, "and has nothing stored");
        choose_preset(&mut app, "Onshape");
    }
    let _session = Session::on(browser(&first));
    let app = launch();
    assert_eq!(state(&app).navigation.active_preset().id, "solidworks", "the second browser changed nothing here");
    let second: serde_json::Value = serde_json::from_str(&second.text().unwrap()).unwrap();
    assert_eq!(second["navigation"]["preset_id"], "onshape");
}

/// A change that cannot be kept still applies, and the window says so in a sentence.
fn refused(why: NotKept, says: &str) {
    let _session = Session::on(Store::Shelf(Memory::refusing(why)));
    let mut app = launch();
    choose_preset(&mut app, "SOLIDWORKS");
    assert_eq!(state(&app).navigation.active_preset().id, "solidworks", "the change applies for the session");
    let text = message(&app).expect("the window says what happened");
    assert!(text.starts_with("Not saved: ") && text.contains(says), "{text}");
    assert!(text.ends_with(store::lasts()), "{text}");
    assert!(!text.contains("config"), "no internal words: {text}");
    let note = state(&app).note.as_ref().map(|n| n.text.clone()).unwrap_or_default();
    assert!(note.contains(says) && !note.contains("config"), "{note}");
}

#[test]
fn storage_that_is_blocked_or_full_says_so_and_the_change_still_applies() {
    refused(NotKept::Nowhere, &NotKept::Nowhere.to_string());
    refused(NotKept::Full, "the browser's storage for this site is full");
}

#[test]
fn nowhere_to_keep_settings_reads_as_a_sentence() {
    let desktop = NotKept::Nowhere.to_string();
    assert_eq!(desktop, "SolidDraft cannot find a folder to keep its settings in");
    assert_eq!(store::lasts(), "The change lasts until you close SolidDraft.");
    assert_eq!(navigation::SaveError::NotKept(NotKept::Nowhere).to_string(), desktop);
}

#[test]
fn the_grid_style_applies_at_once_and_survives_a_reload() {
    let shelf = Memory::new();
    {
        let _session = Session::on(browser(&shelf));
        let mut app = launch();
        assert_eq!(state(&app).viewport.grid_style, GridStyle::default());
        app.get_by_label("Grid and planes").click();
        app.run_steps(3);
        app.get_by_role_and_label(Role::RadioButton, "Fixed").click();
        app.run_steps(3);
        app.get_by_role_and_label(Role::RadioButton, "Custom").click();
        app.run_steps(3);
        let style = state(&app).viewport.grid_style;
        assert!(!style.follow_zoom, "the fixed spacing applies at once");
        assert!(style.color.is_some(), "the custom colour applies at once");
        let saved = GridStyle::from_json(&stored(&browser(&shelf))["view"]["grid"]);
        assert_eq!(saved, style, "and is stored");
    }
    let _session = Session::on(browser(&shelf));
    let mut app = launch();
    let style = state(&app).viewport.grid_style;
    assert!(!style.follow_zoom && style.color.is_some(), "the reload keeps the grid: {style:?}");
    app.get_by_label("Grid and planes").click();
    app.run_steps(3);
    app.get_by_label("Reset grid").click();
    app.run_steps(3);
    assert_eq!(state(&app).viewport.grid_style, GridStyle::default());
    assert_eq!(GridStyle::from_json(&stored(&browser(&shelf))["view"]["grid"]), GridStyle::default());
}

#[test]
fn a_strength_drag_shows_at_once_and_is_stored_when_let_go() {
    let shelf = Memory::new();
    let _session = Session::on(browser(&shelf));
    let mut app = launch();
    app.get_by_label("Grid and planes").click();
    app.run_steps(3);
    let slider = app.get_by_role_and_label(Role::Slider, "Strength").rect();
    let (from, to) = (slider.left_center() + vec2(4.0, 0.0), slider.right_center() - vec2(4.0, 0.0));
    app.hover_at(from);
    app.run_steps(1);
    app.drag_at(from);
    app.run_steps(1);
    app.hover_at(to);
    app.run_steps(2);
    let dragging = state(&app).viewport.grid_style.strength;
    assert!(dragging > 0.9, "the grid follows the drag: {dragging}");
    let before = GridStyle::from_json(&stored(&browser(&shelf))["view"]["grid"]);
    assert_eq!(before.strength, GridStyle::default().strength, "nothing is stored mid-drag");
    app.drop_at(to);
    app.run_steps(3);
    let after = GridStyle::from_json(&stored(&browser(&shelf))["view"]["grid"]);
    assert_eq!(after.strength, state(&app).viewport.grid_style.strength, "the value let go of is stored");
}

fn shortcuts_survive_a_restart(store: Store) {
    {
        let _session = Session::on(store.clone());
        let mut app = launch();
        app.get_by_label("Shortcuts").click();
        app.run_steps(3);
        app.get_by_label("Search commands").type_text("extrude");
        app.run_steps(3);
        app.get_by_label("Assign Extrude").click();
        app.run_steps(2);
        app.key_press(egui::Key::G);
        app.run_steps(3);
        assert_eq!(
            state(&app).shortcuts.shortcut_text("extrude").as_deref(),
            Some("G"),
            "assigned shortcut applies to AppState at once"
        );
        assert_eq!(stored(&store)["shortcuts"]["overrides"]["extrude"], "G");
    }
    let _session = Session::on(store);
    let app = launch();
    assert_eq!(state(&app).shortcuts.shortcut_text("extrude").as_deref(), Some("G"), "survives restart/reload");
}

#[test]
fn shortcuts_survive_a_restart_on_the_desktop_and_per_browser() {
    shortcuts_survive_a_restart(file_store("shortcuts"));
    let (first, second) = (Memory::new(), Memory::new());
    shortcuts_survive_a_restart(browser(&first));
    {
        let _session = Session::on(browser(&second));
        let app = launch();
        assert_eq!(
            state(&app).shortcuts.shortcut_text("extrude").as_deref(),
            Some("E"),
            "another browser keeps its own default shortcuts"
        );
    }
}
