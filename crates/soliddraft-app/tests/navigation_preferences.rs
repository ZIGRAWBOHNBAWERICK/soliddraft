//! The Preferences window without a GPU, driven through egui_kittest: search, the
//! category list with reserved categories, and the Navigation > Mouse, Orbit and zoom and
//! View keys pages. Each change is saved as the app saves it, into a config directory of
//! the test's own.

use egui::accesskit::Role;
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use glam::Vec3;
use soliddraft_app::navigation::{
    self, Action, Motion, NavEvent, NavigationSettings, Outcome, Router, ZoomAnchor, view_keys,
};
use soliddraft_app::preferences::{self, Change, Preferences};
use soliddraft_app::settings::{self, InstallKind};
use soliddraft_app::viewport::camera::Camera;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

fn config_dir(name: &str) -> (MutexGuard<'static, ()>, PathBuf) {
    let guard = CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("soliddraft-nav-prefs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.clone()));
    (guard, dir)
}

fn write(dir: &Path, text: &str) {
    std::fs::write(dir.join("settings.json"), text).unwrap();
}

fn bytes(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join("settings.json")).unwrap_or_default()
}

fn file_json(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&bytes(dir)).unwrap()
}

type Window = Harness<'static, Preferences>;

/// The window on the saved settings; changes are saved as the app saves them, and
/// counted.
fn window() -> Window {
    let view = settings::view();
    let prefs = Preferences::new(&settings::navigation(InstallKind::Existing), view.references, view.grid);
    Harness::builder().with_size(vec2(1100.0, 760.0)).build_ui_state(
        |ui, p: &mut Preferences| {
            p.ui(ui);
            for change in p.take_changes() {
                match change {
                    Change::Navigation(s) => {
                        SAVES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        if let Err(e) = settings::save_navigation(&s) {
                            p.revert(&settings::navigation(InstallKind::Existing), e.to_string());
                        }
                    }
                    Change::Shortcuts(s) => {
                        SAVES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let _ = settings::save_shortcuts(&s);
                    }
                    _ => {}
                }
            }
        },
        prefs,
    )
}

static SAVES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn saves() -> usize {
    SAVES.load(std::sync::atomic::Ordering::SeqCst)
}

fn click(w: &mut Window, label: &str) {
    w.get_by_label(label).click();
    w.run_steps(3);
}

fn has(w: &Window, text: &str) -> bool {
    w.query_all_by_label_contains(text).next().is_some()
}

/// The text a node shows, whether egui reports it as its label or its value.
fn text_of(w: &Window, contains: &str) -> String {
    let node = w.get_all_by_label_contains(contains).next().unwrap_or_else(|| panic!("nothing shows {contains}"));
    let n = node.accesskit_node();
    format!("{}{}", n.label().unwrap_or_default(), n.value().unwrap_or_default())
}

fn search_for(w: &mut Window, text: &str) {
    w.state_mut().search = text.to_owned();
    w.run_steps(2);
}

#[test]
fn preferences_window_opens_with_search_and_categories() {
    let (_guard, dir) = config_dir("opens");
    write(&dir, r#"{"theme":"dark"}"#);
    let before = bytes(&dir);
    let mut w = window();
    w.run_steps(2);
    assert!(has(&w, "Search settings"), "a search field");
    for page in ["Mouse", "Orbit and zoom", "View keys"] {
        assert!(w.query_by_label(page).is_some(), "the left list has {page}");
    }
    assert!(w.query_by_label("Navigation").is_some(), "under Navigation");
    assert_eq!(w.state().page, preferences::MOUSE, "it opens on Navigation > Mouse");
    assert!(w.query_by_label("Navigation > Mouse").is_some());
    drop(w);
    assert_eq!(bytes(&dir), before, "opening and closing changes no setting");
}

#[test]
fn reserved_categories_are_not_finished_pages() {
    let (_guard, _dir) = config_dir("reserved");
    let ids: Vec<&str> = preferences::available_pages().iter().map(|p| p.id).collect();
    assert_eq!(
        ids,
        [
            "navigation.mouse",
            "navigation.orbit_zoom",
            "navigation.view_keys",
            "viewport.grid_and_planes",
            "interface.shortcuts",
            "general_documents.autosave"
        ]
    );
    let reserved = preferences::reserved_categories();
    let labels: Vec<&str> = reserved.iter().map(|c| c.label).collect();
    assert!(!labels.contains(&"General and documents"), "General and documents has its Autosave page");
    for label in ["Selection and editing", "Appearance and graphics", "Assistant"] {
        assert!(labels.contains(&label), "{label} is reserved: {labels:?}");
    }
    assert!(preferences::reserved_categories().iter().all(|c| c.status == preferences::Status::Reserved));
    let mut w = window();
    w.run_steps(2);
    let saves_before = saves();
    for category in reserved {
        click(&mut w, &format!("{} · Coming soon", category.label));
        assert_eq!(w.state().page, category.id);
        assert!(w.query_by_label(category.label).is_some(), "the chosen category has a heading");
        assert!(w.query_by_label("Coming soon").is_some());
        assert!(w.query_by_label("Navigation preset").is_none());
    }
    search_for(&mut w, "appearance");
    click(&mut w, "Appearance and graphics settings · Coming soon");
    assert_eq!(w.state().page, "appearance_graphics");
    assert!(w.state().search.is_empty());
    assert!(w.query_by_label("Coming soon").is_some());
    click(&mut w, "Mouse");
    assert_eq!(w.state().page, preferences::MOUSE);
    assert!(w.query_by_label("Navigation preset").is_some());
    assert_eq!(saves(), saves_before, "and writes nothing");
    for absent in ["Selection profile", "Ribbon", "Units", "Provider"] {
        assert!(!has(&w, absent), "no {absent} control from a reserved page");
    }
}

#[test]
fn search_finds_controls_presets_and_bindings() {
    let (_guard, dir) = config_dir("search");
    write(&dir, r#"{"theme":"dark"}"#);
    let before = bytes(&dir);
    let mut w = window();
    w.run_steps(2);
    let saves_before = saves();
    // Typed into the field itself.
    w.get_by_label("Search settings").click();
    w.run_steps(1);
    w.get_by_label("Search settings").type_text("  WHEEL ");
    w.run_steps(3);
    assert_eq!(w.state().search.trim(), "WHEEL");
    let hit = "Navigation > Orbit and zoom: Wheel direction: invert";
    assert!(w.query_by_label(hit).is_some(), "wheel finds the Wheel direction control, case and spaces aside");
    click(&mut w, hit);
    assert_eq!(w.state().page, preferences::ORBIT_ZOOM);
    assert_eq!(w.state().highlight.as_deref(), Some("wheel_direction"), "and highlights it");
    assert!(w.query_by_label("Wheel direction: invert").is_some());

    search_for(&mut w, "Fusion");
    let fusion = "Navigation > Mouse: Fusion 360 navigation preset";
    assert!(w.query_by_label(fusion).is_some());
    click(&mut w, fusion);
    assert_eq!(w.state().page, preferences::MOUSE);
    assert_eq!(w.state().highlight.as_deref(), Some("preset.fusion"));

    search_for(&mut w, "F6");
    let f6 = w.query_all_by_label_contains("Fit F6").count();
    assert!(f6 >= 1, "F6 finds the Fit binding");
    w.get_all_by_label_contains("Fusion 360: Fit F6").next().unwrap().click();
    w.run_steps(2);
    assert_eq!(w.state().page, preferences::MOUSE);

    search_for(&mut w, "normal");
    assert!(w.query_by_label("Navigation > View keys: Normal to and reverse normal keys").is_some());
    assert!(w.query_all_by_label_contains("Normal to").count() > 1, "and the normal-to rows");
    click(&mut w, "Navigation > View keys: Normal to and reverse normal keys");
    assert_eq!(w.state().page, preferences::VIEW_KEYS);

    search_for(&mut w, "ZZZ");
    assert!(w.query_by_label("No settings found").is_some());
    search_for(&mut w, "");
    assert!(w.query_by_label("No settings found").is_none());
    assert!(w.query_by_label("Navigation > View keys").is_some(), "clearing shows the page again");
    assert_eq!(saves(), saves_before, "search changes nothing");
    assert_eq!(bytes(&dir), before);
    assert!(preferences::search("wheel").iter().all(|h| h.label.to_lowercase().contains("wheel")));
}

/// Drags in the practice area with `button` held and `m` down.
fn practice_drag(w: &mut Window, button: PointerButton, m: Modifiers) {
    let area = w.get_by_label("Practice area").rect();
    let from = area.center();
    w.hover_at(from);
    w.step();
    w.event(egui::Event::ModifiersChanged(m));
    w.event(egui::Event::PointerButton { pos: from, button, pressed: true, modifiers: m });
    w.step();
    for i in 1..=4 {
        w.event(egui::Event::PointerMoved(from + vec2(10.0 * i as f32, 5.0 * i as f32)));
        w.step();
    }
    let to = from + vec2(40.0, 20.0);
    w.event(egui::Event::PointerButton { pos: to, button, pressed: false, modifiers: m });
    w.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    w.run_steps(3);
}

fn practice_label(w: &Window) -> String {
    w.state().practice_label.clone()
}

#[test]
fn saved_custom_sketch_keys_survive_the_new_defaults_in_every_preset() {
    use soliddraft_app::commands::ShortcutPreset;

    let (_guard, dir) = config_dir("sketch-key-migration");
    let mut lost = Vec::new();
    for preset in ShortcutPreset::ALL {
        for (custom, key, added_default) in [
            ("sketch_line", "T", "sketch_trim"),
            ("sketch_circle", "O", "sketch_offset"),
            ("smart_dimension", "M", "sketch_move"),
        ] {
            write(
                &dir,
                &serde_json::json!({
                    "shortcuts": {"preset": preset.id(), "overrides": {custom: key}}
                })
                .to_string(),
            );
            let before = bytes(&dir);
            let mut w = window();
            w.run_steps(2);
            let loaded = &w.state().shortcuts;
            if loaded.shortcut_text(custom).as_deref() != Some(key) {
                lost.push(format!("{}: {custom} -> {key}", preset.id()));
                continue;
            }
            assert!(loaded.keys_of(added_default).is_empty(), "the custom assignment owns {key}");
            for (other, default_key) in [("sketch_trim", "T"), ("sketch_offset", "O"), ("sketch_move", "M")] {
                if other != added_default {
                    assert_eq!(loaded.shortcut_text(other).as_deref(), Some(default_key));
                }
            }
            assert_eq!(bytes(&dir), before, "opening Preferences does not write settings");
            settings::save_shortcuts(loaded).unwrap();
            assert_eq!(settings::shortcuts(), *loaded, "migration survives save and reopen");
        }
    }
    assert!(lost.is_empty(), "lost saved mappings: {lost:?}");
}

#[test]
fn mouse_page_preset_switch_updates_list_practice_and_settings() {
    let (_guard, dir) = config_dir("mouse");
    write(&dir, r#"{"theme":"dark"}"#);
    let mut w = window();
    w.run_steps(2);
    let labels: Vec<&str> = navigation::presets().iter().map(|p| p.label).collect();
    assert_eq!(labels.len(), 9);
    let mut ys = Vec::new();
    for label in &labels {
        let node = w.get_by_role_and_label(Role::RadioButton, label);
        ys.push((node.rect().top(), node.rect().left()));
    }
    let mut sorted = ys.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(ys, sorted, "the nine presets in their stable order");

    w.get_by_role_and_label(Role::RadioButton, "Onshape").click();
    w.run_steps(3);
    assert_eq!(w.state().draft.preset_id, "onshape");
    for row in ["Orbit: Right drag", "Pan: Middle drag", "Pan: Ctrl+Right drag", "Fit: Middle double-click"] {
        assert!(w.query_by_label(row).is_some(), "the list reads {row}");
    }
    let fit_key = text_of(&w, "Fit: F - blocked");
    assert!(fit_key.contains("Front") && fit_key.contains("Fillet"), "{fit_key}");
    assert!(has(&w, "Normal to, again to reverse: N"));
    assert!(has(&w, "physical Control key") && has(&w, "right click"), "the limitations name Ctrl and the right click");
    assert_eq!(file_json(&dir)["navigation"]["preset_id"], "onshape");
    assert_eq!(file_json(&dir)["theme"], "dark");
    drop(w);

    // Opened again, it is still Onshape.
    let mut w = window();
    w.run_steps(2);
    assert_eq!(w.state().draft.active_preset().id, "onshape");
    // Change Orbit to a left drag: it shows, saves, and the router uses it after a reload.
    w.get_by_label("Change orbit").click();
    w.run_steps(2);
    assert_eq!(w.state().capture, Some("orbit"));
    practice_drag(&mut w, PointerButton::Primary, Modifiers::NONE);
    assert!(w.state().capture.is_none());
    assert!(w.query_by_label("Orbit: Left drag").is_some(), "{:?}", w.state().message);
    let saved = settings::navigation(InstallKind::Existing);
    assert_eq!(saved.overrides["onshape"]["orbit"].buttons, [navigation::Button::Left]);
    let mut r = Router::new(&saved, false);
    r.handle(NavEvent::Press { button: navigation::Button::Left, pos: Pos2::ZERO, inside: true, time: 0.0 });
    let out = r.handle(NavEvent::Move(Pos2::new(0.0, 20.0)));
    assert_eq!(out.outcome, Outcome::Action(Action::Orbit));
    // A gesture another row has is refused, and nothing is written.
    let before = bytes(&dir);
    w.get_by_label("Change pan").click();
    w.run_steps(2);
    practice_drag(&mut w, PointerButton::Secondary, Modifiers::CTRL);
    let message = w.state().message.clone().expect("the refusal says why");
    assert!(message.error && message.text.contains("same gesture"), "{message:?}");
    assert_eq!(bytes(&dir), before, "nothing was written");
    assert!(w.query_by_label("Pan: Middle drag").is_some(), "the old binding stays");
}

#[test]
fn mouse_page_reset_and_customize_validate_conflicts() {
    let (_guard, dir) = config_dir("reset");
    write(&dir, r#"{"theme":"dark","layout":{"width":300}}"#);
    let mut s = NavigationSettings::new(InstallKind::Existing);
    s.select_preset("solidworks").unwrap();
    s.set_override(
        "fusion",
        "pan",
        navigation::Chord {
            event: navigation::EventKind::Drag,
            buttons: vec![navigation::Button::Right],
            ..Default::default()
        },
    )
    .unwrap();
    s.wheel.invert = true;
    s.camera.orbit_sensitivity = 2.5;
    view_keys::apply_to(&mut s, "onshape").unwrap();
    settings::save_navigation(&s).unwrap();
    let mut w = window();
    w.run_steps(2);
    w.get_by_label("Change orbit").click();
    w.run_steps(2);
    practice_drag(&mut w, PointerButton::Secondary, Modifiers::NONE);
    assert!(settings::navigation(InstallKind::Existing).overrides.contains_key("solidworks"));

    let saves_before = saves();
    click(&mut w, "Reset preset");
    assert_eq!(saves(), saves_before + 1, "a reset writes once");
    let after = settings::navigation(InstallKind::Existing);
    assert!(!after.overrides.contains_key("solidworks"), "the override is gone");
    assert!(after.overrides.contains_key("fusion"), "other presets' changes stay");
    assert!(after.wheel.invert && after.camera.orbit_sensitivity == 2.5, "the wheel and camera stay");
    assert_eq!(after.keyboard_profile, "onshape", "the view keys stay");
    assert_eq!(file_json(&dir)["layout"], serde_json::json!({"width": 300}), "foreign keys stay");

    // Reset page: the Orbit and zoom page's own controls, nothing else.
    click(&mut w, "Orbit and zoom");
    let saves_before = saves();
    click(&mut w, "Reset page");
    assert_eq!(saves(), saves_before + 1);
    let page = settings::navigation(InstallKind::Existing);
    assert!(!page.wheel.invert && page.camera.orbit_sensitivity == 1.0, "the page's controls are back");
    assert_eq!(page.preset_id, "solidworks", "the preset stays");
    assert!(page.overrides.contains_key("fusion"), "and its changes");
    assert_eq!(file_json(&dir)["theme"], "dark");

    // A custom gesture that clashes is refused before anything is saved.
    click(&mut w, "Mouse");
    let before = bytes(&dir);
    w.get_by_label("Change orbit").click();
    w.run_steps(2);
    practice_drag(&mut w, PointerButton::Middle, Modifiers::CTRL);
    assert!(w.state().message.as_ref().is_some_and(|m| m.error), "the clash is reported");
    assert_eq!(bytes(&dir), before);
    let mut r = Router::new(&settings::navigation(InstallKind::Existing), false);
    r.handle(NavEvent::Press { button: navigation::Button::Middle, pos: Pos2::ZERO, inside: true, time: 0.0 });
    assert_eq!(
        r.handle(NavEvent::Move(Pos2::new(0.0, 20.0))).outcome,
        Outcome::Action(Action::Orbit),
        "the old row works"
    );
}

#[test]
fn practice_viewport_reports_the_gesture_without_moving_the_camera() {
    let (_guard, _dir) = config_dir("practice");
    let mut s = NavigationSettings::new(InstallKind::Existing);
    s.select_preset("solidworks").unwrap();
    settings::save_navigation(&s).unwrap();
    let mut w = window();
    w.run_steps(2);
    assert!(w.query_by_label("Practice area").is_some(), "a labelled practice area, drawn by the app");
    practice_drag(&mut w, PointerButton::Middle, Modifiers::NONE);
    assert_eq!(practice_label(&w), "Orbit");
    assert!(w.query_by_label("Practice: Orbit").is_some());
    practice_drag(&mut w, PointerButton::Middle, Modifiers::CTRL);
    assert_eq!(practice_label(&w), "Pan");
    practice_drag(&mut w, PointerButton::Middle, Modifiers::SHIFT);
    assert_eq!(practice_label(&w), "Zoom");
    practice_drag(&mut w, PointerButton::Middle, Modifiers::CTRL);
    let area = w.get_by_label("Practice area").rect();
    w.hover_at(area.center());
    w.step();
    w.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    w.run_steps(2);
    assert_eq!(practice_label(&w), "Zoom", "the wheel zooms");
    w.get_by_role_and_label(Role::RadioButton, "Fusion 360").click();
    w.run_steps(3);
    practice_drag(&mut w, PointerButton::Middle, Modifiers::NONE);
    assert_eq!(practice_label(&w), "Pan");
    practice_drag(&mut w, PointerButton::Middle, Modifiers::SHIFT);
    assert_eq!(practice_label(&w), "Orbit");
}

/// The camera after one routed gesture under `nav`.
fn after(nav: &NavigationSettings, action: Action, motion: Motion, pointer: Option<Pos2>) -> Camera {
    let rect = egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0));
    let router = Router::new(nav, false);
    let mut camera = Camera::default();
    let routed = navigation::Routed { outcome: Outcome::Action(action), binding: None, consumed: true, motion };
    soliddraft_app::viewport::apply_navigation(&mut camera, &routed, &router, nav, rect, pointer);
    camera
}

fn wheel_factor(nav: &NavigationSettings) -> f32 {
    match Router::new(nav, false).handle(NavEvent::Wheel { lines: 1.0 }).motion {
        Motion::Zoom { factor } => factor,
        other => panic!("{other:?}"),
    }
}

#[test]
fn orbit_zoom_page_enabled_controls_change_camera() {
    let (_guard, _dir) = config_dir("orbit-zoom");
    let mut w = window();
    w.run_steps(2);
    click(&mut w, "Orbit and zoom");
    for label in [
        "Orbit sensitivity",
        "Pan sensitivity",
        "Wheel speed",
        "Wheel direction: invert",
        "Zoom toward the cursor",
        "Zoom toward the centre",
        "View centre",
        "Transition duration (s)",
    ] {
        assert!(has(&w, label), "the page has {label}");
    }
    let base = NavigationSettings::new(InstallKind::Existing);
    let drag = Motion::Drag { delta: vec2(10.0, 4.0), origin: Pos2::ZERO };
    let default = Camera::default();

    w.state_mut().edit(|s| s.camera.orbit_sensitivity = 2.0);
    w.state_mut().edit(|s| s.camera.pan_sensitivity = 2.0);
    w.state_mut().edit(|s| s.wheel.speed = 2.0);
    w.run_steps(2);
    let doubled = settings::navigation(InstallKind::Existing);
    let yaw = |c: &Camera| c.yaw - default.yaw;
    let one = after(&base, Action::Orbit, drag, None);
    let two = after(&doubled, Action::Orbit, drag, None);
    assert!((yaw(&two) - 2.0 * yaw(&one)).abs() < 1e-5, "double orbit sensitivity doubles the turn");
    let shift = |c: &Camera| (c.target - default.target).length();
    let one = after(&base, Action::Pan, drag, None);
    let two = after(&doubled, Action::Pan, drag, None);
    assert!((shift(&two) - 2.0 * shift(&one)).abs() < 1e-3, "double pan sensitivity doubles the move");
    assert!((wheel_factor(&doubled).ln() - 2.0 * wheel_factor(&base).ln()).abs() < 1e-5, "double wheel speed");

    // The checkbox and the radio buttons, through the page.
    click(&mut w, "Wheel direction: invert");
    let inverted = settings::navigation(InstallKind::Existing);
    assert!(inverted.wheel.invert);
    assert!(wheel_factor(&inverted) < 1.0 && wheel_factor(&base) > 1.0, "invert turns the wheel around");
    click(&mut w, "Zoom toward the centre");
    let centred = settings::navigation(InstallKind::Existing);
    assert_eq!(centred.camera.zoom_anchor, ZoomAnchor::Center);
    let zoom = Motion::Zoom { factor: 1.3 };
    let cursor = Pos2::new(720.0, 230.0);
    assert_eq!(
        after(&centred, Action::WheelZoom, zoom, Some(cursor)).target,
        default.target,
        "centre: the target stays"
    );
    click(&mut w, "Zoom toward the cursor");
    let at_cursor = settings::navigation(InstallKind::Existing);
    let rect = egui::Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0));
    let (o, d) = default.ray(cursor, rect);
    let point = o + d * (default.distance / d.dot(default.forward()));
    let moved = after(&at_cursor, Action::WheelZoom, zoom, Some(cursor));
    assert!((moved.project(point, rect).unwrap() - cursor).length() < 1.0, "cursor: the point stays under it");

    // A longer transition is farther from its goal after one tick, and still finishes.
    let goal = |seconds: f32, ticks: usize| {
        let mut c = Camera::default();
        c.transition_seconds = seconds;
        c.set_view(soliddraft_app::viewport::camera::StandardView::Top);
        for _ in 0..ticks {
            c.tick(1.0 / 60.0);
        }
        c.direction().dot(Vec3::Z)
    };
    assert!(goal(1.0, 1) < goal(0.2, 1), "after one tick the long one is farther");
    assert!(goal(1.0, 600) > 0.9999 && goal(0.2, 600) > 0.9999, "both arrive");
    w.state_mut().edit(|s| s.camera.transition_seconds = 0.8);
    w.run_steps(2);

    let saved = settings::navigation(InstallKind::Existing);
    assert_eq!(saved, w.state().draft, "every control persisted and reloads");
    assert_eq!(saved.camera.transition_seconds, 0.8);
    click(&mut w, "Reset preset");
    assert_eq!(
        settings::navigation(InstallKind::Existing).camera,
        saved.camera,
        "Reset preset keeps the camera settings"
    );
    click(&mut w, "Reset page");
    let reset = settings::navigation(InstallKind::Existing);
    assert_eq!(reset.camera, navigation::CameraSettings::default());
    assert_eq!(reset.wheel, navigation::WheelSettings::default());
}

#[test]
fn unsupported_camera_modes_are_not_presented_as_working() {
    let (_guard, _dir) = config_dir("unsupported");
    let mut w = window();
    w.run_steps(2);
    click(&mut w, "Orbit and zoom");
    for label in ["Free trackball", "Under the cursor", "Selection", "Origin"] {
        let node = w.get_by_role_and_label(Role::RadioButton, label);
        assert!(node.accesskit_node().is_disabled(), "{label} is not offered as working");
        node.click();
        w.run_steps(2);
        assert!(has(&w, "Coming soon"), "{label}");
    }
    assert_eq!(
        settings::navigation(InstallKind::Existing).camera.pivot,
        navigation::Pivot::Target,
        "nothing else is saved"
    );
    let stored: navigation::CameraSettings =
        serde_json::from_value(serde_json::json!({"pivot": "cursor"})).unwrap_or_default();
    assert_eq!(stored.pivot, navigation::Pivot::Target, "an unsupported pivot cannot be stored as active");

    // Roll rows stay disabled and say Coming soon.
    click(&mut w, "Mouse");
    for preset in ["SOLIDWORKS", "Creo Parametric"] {
        w.get_by_role_and_label(Role::RadioButton, preset).click();
        w.run_steps(3);
        let roll = text_of(&w, "Roll ·");
        assert_eq!(roll, "Roll · Coming soon", "{preset}");
        assert!(w.query_by_label("Change roll").is_none(), "{preset}: roll cannot be edited as if it worked");
    }
    let mut sw = Router::for_preset(navigation::preset("solidworks").unwrap().clone(), false);
    sw.handle(NavEvent::Modifiers(Modifiers::ALT));
    sw.handle(NavEvent::Press { button: navigation::Button::Middle, pos: Pos2::ZERO, inside: true, time: 0.0 });
    assert_eq!(sw.handle(NavEvent::Move(Pos2::new(0.0, 20.0))).outcome, Outcome::Unsupported(Action::Roll));
}

#[test]
fn view_keys_page_shows_profiles_and_conflicts() {
    let (_guard, _dir) = config_dir("view-keys-page");
    let mut w = window();
    w.run_steps(2);
    click(&mut w, "View keys");
    for p in view_keys::profiles() {
        assert!(w.query_by_role_and_label(Role::RadioButton, p.label).is_some(), "{} is listed", p.label);
    }
    assert_eq!(w.state().draft.keyboard_profile, view_keys::SOLIDDRAFT, "SolidDraft's own keys are current");
    w.get_by_role_and_label(Role::RadioButton, "SOLIDWORKS").click();
    w.run_steps(3);
    assert_eq!(settings::navigation(InstallKind::Existing).keyboard_profile, "solidworks");
    assert!(has(&w, "Fit: F - blocked"), "the blocked row shows");
    let lines: Vec<String> = w
        .query_all_by_label_contains("is off; the key stays")
        .map(|n| {
            let n = n.accesskit_node();
            format!("{}{}", n.label().unwrap_or_default(), n.value().unwrap_or_default())
        })
        .collect();
    assert!(lines.iter().any(|l| l.contains("Front") && !l.contains("view_front")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("Fillet") && !l.contains("fillet")), "{lines:?}");
    w.get_by_role_and_label(Role::RadioButton, "Fusion 360").click();
    w.run_steps(3);
    assert!(has(&w, "Fit: F6"));
    assert!(has(&w, "None: this profile takes no command's key"));
    w.get_by_role_and_label(Role::RadioButton, "Blender").click();
    w.run_steps(3);
    assert!(!has(&w, "needs the numeric keypad"));
    assert!(has(&w, "Coming soon"));
}

#[test]
fn preferences_does_not_disturb_existing_settings() {
    let (_guard, dir) = config_dir("undisturbed");
    let start = r#"{"assistant":{"provider":"openrouter"},"layout":{"width":300},"appearance":{"theme":"dark"},"documents":{"autosave":{"interval_seconds":30}},"file_locations":{"entries":[]},"theme":"dark"}"#;
    write(&dir, start);
    let mut w = window();
    w.run_steps(4);
    drop(w);
    assert_eq!(bytes(&dir), start.as_bytes(), "open and close: not a byte changed");
    let before = file_json(&dir);
    let mut w = window();
    w.run_steps(2);
    w.get_by_role_and_label(Role::RadioButton, "Inventor").click();
    w.run_steps(3);
    let after = file_json(&dir);
    for key in ["assistant", "layout", "appearance", "documents", "file_locations", "theme"] {
        assert_eq!(after[key], before[key], "{key} is untouched");
    }
    assert_eq!(after["navigation"]["preset_id"], "inventor");
    assert!(!has(&w, "Export"), "no settings export is added");
    assert!(!has(&w, "API key"), "no assistant secrets are shown");
}

#[test]
fn shortcuts_page_lists_registry_searches_assigns_clears_resets_swaps_and_presets() {
    use egui::Key;

    let (_guard, dir) = config_dir("shortcuts-page");
    write(&dir, r#"{"theme":"dark"}"#);
    let mut w = window();
    w.run_steps(2);
    click(&mut w, "Shortcuts");
    assert_eq!(w.state().page, preferences::SHORTCUTS);
    assert!(w.query_by_label("Interface > Shortcuts").is_some());

    // Search filters the command list in place.
    w.get_by_role_and_label(Role::TextInput, "Search commands").click();
    w.run_steps(2);
    w.get_by_role_and_label(Role::TextInput, "Search commands").type_text("extrude");
    w.run_steps(2);
    assert!(w.query_by_label("Extrude: E").is_some());
    assert!(w.query_by_label("Chamfer: C").is_none());

    click(&mut w, "Assign Extrude");
    assert_eq!(w.state().shortcut_capture, Some("extrude"));
    w.key_press(Key::G);
    w.run_steps(3);
    assert!(w.state().shortcut_capture.is_none());
    assert!(w.query_by_label("Extrude: G").is_some());
    assert_eq!(file_json(&dir)["shortcuts"]["overrides"]["extrude"], "G");

    // Clear and reset a key.
    click(&mut w, "Clear Extrude");
    assert!(w.query_by_label("Extrude: None").is_some());
    assert!(file_json(&dir)["shortcuts"]["overrides"]["extrude"].is_null());
    click(&mut w, "Reset Extrude");
    assert!(w.query_by_label("Extrude: E").is_some());

    // Conflict warning and swap.
    click(&mut w, "Assign Extrude");
    w.key_press(Key::S);
    w.run_steps(3);
    assert!(w.state().shortcut_conflict.is_some(), "conflict with Sketch is recorded");
    assert!(has(&w, "already used by Sketch (Outside a sketch)"), "warning names Sketch and its scope");
    assert!(w.query_by_label("Extrude: E").is_some(), "not overwritten before swap");
    assert_eq!(w.state().shortcuts.shortcut_text("sketch").as_deref(), Some("S"));
    click(&mut w, "Swap");
    assert!(w.state().shortcut_conflict.is_none());
    assert!(w.query_by_label("Extrude: S").is_some());
    assert_eq!(w.state().shortcuts.shortcut_text("sketch").as_deref(), Some("E"));
    assert_eq!(file_json(&dir)["shortcuts"]["overrides"]["extrude"], "S");
    assert_eq!(file_json(&dir)["shortcuts"]["overrides"]["sketch"], "E");

    // Resetting a key that was swapped warns before changing either command.
    click(&mut w, "Reset Extrude");
    assert!(w.state().shortcut_conflict.is_some());
    assert!(w.query_by_label("Extrude: S").is_some());
    assert_eq!(w.state().shortcuts.shortcut_text("sketch").as_deref(), Some("E"));
    click(&mut w, "Cancel");
    assert!(w.query_by_label("Extrude: S").is_some(), "cancel preserves both assignments");

    // Reset all to the chosen preset (SolidDraft, SolidWorks, Fusion).
    w.get_by_role_and_label(Role::RadioButton, "SolidWorks").click();
    w.run_steps(3);
    let shortcut = |w: &Window, id| w.state().shortcuts.shortcut_text(id).unwrap();
    let sketch_keys = |w: &Window| {
        for (id, key) in [
            ("sketch_line", "L"),
            ("sketch_rectangle", "R"),
            ("sketch_circle", "C"),
            ("sketch_arc", "A"),
            ("smart_dimension", "D"),
            ("sketch_trim", "T"),
            ("sketch_offset", "O"),
            ("sketch_move", "M"),
        ] {
            assert_eq!(shortcut(w, id), key);
        }
    };
    sketch_keys(&w);
    assert_eq!(shortcut(&w, "zoom_fit"), "F");
    assert_eq!(shortcut(&w, "view_front"), "Ctrl+1");
    assert_eq!(shortcut(&w, "view_isometric"), "Ctrl+7");
    assert_eq!(shortcut(&w, "normal_to"), "Ctrl+8");
    assert!(w.query_by_label("Extrude: E").is_some(), "overrides cleared by preset reset");
    assert_eq!(file_json(&dir)["shortcuts"]["preset"], "solidworks");

    w.get_by_role_and_label(Role::RadioButton, "Fusion").click();
    w.run_steps(3);
    assert_eq!(shortcut(&w, "zoom_fit"), "F6");
    assert_eq!(shortcut(&w, "view_front"), "F");
    assert_eq!(file_json(&dir)["shortcuts"]["preset"], "fusion");
    sketch_keys(&w);

    click(&mut w, "Clear search");
    w.get_by_role_and_label(Role::TextInput, "Search commands").click();
    w.run_steps(2);
    w.get_by_role_and_label(Role::TextInput, "Search commands").type_text("fit");
    w.run_steps(3);
    click(&mut w, "Clear Fit");
    assert!(w.query_by_label("Fit: None").is_some());
    click(&mut w, "Reset all");
    assert!(w.query_by_label("Fit: F6").is_some(), "Reset all restores the chosen preset");

    w.get_by_role_and_label(Role::RadioButton, "SolidDraft").click();
    w.run_steps(3);
    assert!(w.query_by_label("Fit: Z").is_some());
    assert_eq!(shortcut(&w, "view_front"), "F");
    assert_eq!(shortcut(&w, "normal_to"), "N");
    assert_eq!(file_json(&dir)["shortcuts"]["preset"], "soliddraft");
    sketch_keys(&w);
    assert_eq!(file_json(&dir)["theme"], "dark", "other settings keys stay");
}

#[test]
fn reset_warns_for_secondary_workspace_aliases_and_restores_all_of_them() {
    use soliddraft_app::commands::ShortcutPreset;
    let (_guard, dir) = config_dir("reset-workspace-alias");
    write(&dir, r#"{"shortcuts":{"preset":"soliddraft","overrides":{"zoom_fit":null,"drawing_section":"Z"}}}"#);
    let mut w = window();
    w.run_steps(2);
    click(&mut w, "Shortcuts");
    w.get_by_role_and_label(Role::TextInput, "Search commands").click();
    w.run_steps(2);
    w.get_by_role_and_label(Role::TextInput, "Search commands").type_text("fit");
    w.run_steps(3);
    click(&mut w, "Reset Fit");
    let conflict = w.state().shortcut_conflict.as_ref().expect("drawing Z requires confirmation");
    assert_eq!(conflict.with, ["drawing_section"], "model Front's F does not conflict with drawing Fit's F");
    assert_eq!(file_json(&dir)["shortcuts"]["overrides"]["drawing_section"], "Z", "nothing displaced yet");
    assert!(w.state().shortcuts.keys_of("zoom_fit").is_empty());
    click(&mut w, "Swap");
    let shortcuts = &w.state().shortcuts;
    assert_eq!(shortcuts.keys_of("zoom_fit"), ShortcutPreset::Soliddraft.keys_of("zoom_fit"));
    assert!(shortcuts.keys_of("drawing_section").is_empty());
    assert_eq!(shortcuts.shortcut_text("view_front").as_deref(), Some("F"), "Front keeps its unrelated key");
    assert_eq!(settings::shortcuts(), *shortcuts, "all restored aliases survive a reload");
    click(&mut w, "Reset Fit");
    assert!(w.state().shortcut_conflict.is_none(), "resetting an unchanged default has no false conflict");
}
