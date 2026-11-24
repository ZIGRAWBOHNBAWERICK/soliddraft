//! The viewport's display choices in `settings.json`: the `view` section's keys, how each
//! falls back on its own, and that saving one thing keeps everything else.
//!
//! The design is `sdc-d-ux-viewport`, the unit-level checks and merge A.

#[path = "support/viewport_acceptance.rs"]
mod support;

use egui_kittest::kittest::Queryable;
use serde_json::json;
use soliddraft_app::settings::{self, InstallKind, ReferenceDisplay};
use soliddraft_app::viewport::look::Preset;
use support::{ALWAYS_SHOW_AXES, Case, empty, launch, set_preference};

#[test]
fn graphics_presets_change_the_drawn_part() {
    use egui_kittest::kittest::Queryable;
    use soliddraft_app::document::{Command, ExtrudeParams, ProfileKind};
    use std::time::Instant;

    let mut case = Case::gpu("graphics_presets");
    let mut doc = empty();
    let params = ExtrudeParams { profile: ProfileKind::Circle { diameter: 200.0 }, depth: 50.0, ..Default::default() };
    doc.apply(Command::AddExtrude { params, via_assistant: false }).unwrap();
    let mut app = launch(doc);
    let mut observed = Vec::new();
    for (preset, angle) in [("Performance", 15.0), ("Quality", 10.0)] {
        support::run_command(&mut app, "settings");
        app.get_by_label("Graphics").click();
        app.run_steps(3);
        app.get_by_label(preset).click();
        app.run_steps(3);
        support::close_preferences(&mut app);
        let start = Instant::now();
        loop {
            let doc = &app.state().state.doc;
            if doc.tess_params.surface_angle_deg == angle && !doc.display_quality().pending() {
                break;
            }
            assert!(start.elapsed() < support::render_steps::HANG_GUARD, "{preset} did not draw");
            std::thread::sleep(std::time::Duration::from_millis(10));
            app.step();
        }
        // The viewport takes the worker's reply after submitting that frame's mesh.
        app.step();
        observed.push((app.state().state.doc.triangle_count(), case.frame(&mut app, preset)));
    }
    let [(coarse_count, coarse), (fine_count, fine)] = observed.as_slice() else { unreachable!() };
    assert!(fine_count > coarse_count, "Quality drew {fine_count} triangles, Performance drew {coarse_count}");
    let rect = support::viewport(&app);
    let at = rect.center() - rect.size() / 4.0;
    let image = |frame| {
        image::imageops::crop_imm(
            frame,
            at.x as u32,
            at.y as u32,
            (rect.width() / 2.0) as u32,
            (rect.height() / 2.0) as u32,
        )
        .to_image()
    };
    assert!(image(coarse) != image(fine), "the rendered model changed");
    case.note(format!("Performance drew {coarse_count} triangles. Quality drew {fine_count} triangles."));
}

const DEFAULTS: ReferenceDisplay =
    ReferenceDisplay { show_grid: true, show_floor: true, show_origin_planes: true, always_show_origin_axes: true };

/// Reads the settings and checks that reading wrote nothing.
fn view_unchanged(case: &Case) -> settings::ViewSettings {
    let before = std::fs::read(case.settings_file()).ok();
    let view = settings::view();
    assert_eq!(std::fs::read(case.settings_file()).ok(), before, "reading leaves the file as it was");
    view
}

#[test]
fn an_absent_or_legacy_view_section_gets_the_defaults() {
    let case = Case::settings("view_absent_or_legacy");
    let view = view_unchanged(&case);
    assert_eq!((view.preset, view.references), (Preset::Studio, DEFAULTS), "no file");
    assert!(!case.settings_file().exists(), "reading does not create the file");

    case.seed_settings(r#"{"theme":"dark"}"#);
    let view = view_unchanged(&case);
    assert_eq!((view.preset, view.references), (Preset::Studio, DEFAULTS), "no view section");

    case.seed_settings(r#"{"view":{"preset":"realistic"}}"#);
    let view = view_unchanged(&case);
    assert_eq!((view.preset, view.references), (Preset::Realistic, DEFAULTS), "a file from before the keys");

    case.seed_settings(r#"{"view":"not an object"}"#);
    let view = view_unchanged(&case);
    assert_eq!((view.preset, view.references), (Preset::Studio, DEFAULTS), "an unreadable section");
}

#[test]
fn a_missing_show_grid_falls_back_alone() {
    let case = Case::settings("view_missing_show_grid");
    case.seed_settings(r#"{"view":{"preset":"fast","show_origin_planes":false,"always_show_origin_axes":true}}"#);
    let view = view_unchanged(&case);
    let expected = ReferenceDisplay {
        show_grid: true,
        show_floor: true,
        show_origin_planes: false,
        always_show_origin_axes: true,
    };
    assert_eq!((view.preset, view.references), (Preset::Fast, expected));
}

#[test]
fn a_missing_or_wrong_typed_show_floor_falls_back_alone() {
    let case = Case::settings("view_missing_show_floor");
    let expected = ReferenceDisplay { show_floor: true, show_grid: false, ..DEFAULTS };
    for seed in [r#"{"view":{"show_grid":false}}"#, r#"{"view":{"show_grid":false,"show_floor":"no"}}"#] {
        case.seed_settings(seed);
        assert_eq!(view_unchanged(&case).references, expected, "{seed}");
    }
    case.seed_settings(r#"{"view":{"show_floor":false}}"#);
    assert_eq!(view_unchanged(&case).references, ReferenceDisplay { show_floor: false, ..DEFAULTS });
}

#[test]
fn a_missing_show_origin_planes_falls_back_alone() {
    let case = Case::settings("view_missing_show_origin_planes");
    case.seed_settings(r#"{"view":{"preset":"fast","show_grid":false,"always_show_origin_axes":true}}"#);
    let view = view_unchanged(&case);
    let expected = ReferenceDisplay {
        show_grid: false,
        show_floor: true,
        show_origin_planes: true,
        always_show_origin_axes: true,
    };
    assert_eq!((view.preset, view.references), (Preset::Fast, expected));
}

#[test]
fn a_missing_always_show_origin_axes_falls_back_alone() {
    let case = Case::settings("view_missing_always_show_origin_axes");
    case.seed_settings(r#"{"view":{"preset":"fast","show_grid":false,"show_origin_planes":false}}"#);
    let view = view_unchanged(&case);
    let expected = ReferenceDisplay {
        show_grid: false,
        show_floor: true,
        show_origin_planes: false,
        always_show_origin_axes: true,
    };
    assert_eq!((view.preset, view.references), (Preset::Fast, expected));
}

#[test]
fn a_wrong_typed_show_grid_falls_back_alone() {
    let case = Case::settings("view_wrong_show_grid");
    case.seed_settings(
        r#"{"view":{"preset":"fast","show_grid":"no","show_origin_planes":false,"always_show_origin_axes":true}}"#,
    );
    let view = view_unchanged(&case);
    let expected = ReferenceDisplay {
        show_grid: true,
        show_floor: true,
        show_origin_planes: false,
        always_show_origin_axes: true,
    };
    assert_eq!((view.preset, view.references), (Preset::Fast, expected));
}

#[test]
fn a_wrong_typed_show_origin_planes_falls_back_alone() {
    let case = Case::settings("view_wrong_show_origin_planes");
    case.seed_settings(
        r#"{"view":{"preset":"fast","show_grid":false,"show_origin_planes":0,"always_show_origin_axes":true}}"#,
    );
    let view = view_unchanged(&case);
    let expected = ReferenceDisplay {
        show_grid: false,
        show_floor: true,
        show_origin_planes: true,
        always_show_origin_axes: true,
    };
    assert_eq!((view.preset, view.references), (Preset::Fast, expected));
}

#[test]
fn a_wrong_typed_always_show_origin_axes_falls_back_alone() {
    let case = Case::settings("view_wrong_always_show_origin_axes");
    case.seed_settings(
        r#"{"view":{"preset":"fast","show_grid":false,"show_origin_planes":false,"always_show_origin_axes":"yes"}}"#,
    );
    let view = view_unchanged(&case);
    let expected = ReferenceDisplay {
        show_grid: false,
        show_floor: true,
        show_origin_planes: false,
        always_show_origin_axes: true,
    };
    assert_eq!((view.preset, view.references), (Preset::Fast, expected));
}

#[test]
fn an_unknown_look_falls_back_without_costing_a_display_choice() {
    let case = Case::settings("view_unknown_look");
    case.seed_settings(r#"{"view":{"preset":"cinema","always_show_origin_axes":true}}"#);
    let view = view_unchanged(&case);
    assert_eq!(view.preset, Preset::Studio);
    assert!(view.references.always_show_origin_axes);
}

#[test]
fn look_and_display_writes_preserve_each_other() {
    let case = Case::settings("look_and_display_writes");
    let navigation = json!({"version": 99, "preset_id": "from_the_future", "future": "keep"});
    let seed = json!({
        "view": {
            "preset": "studio",
            "show_grid": false,
            "show_origin_planes": false,
            "always_show_origin_axes": true,
            "future": {"nested": [1, 2]},
        },
        "navigation": navigation,
        "assistant": {"provider": "openrouter"},
        "appearance": {"theme": "dark"},
        "documents": {"autosave": {"interval_seconds": 30}},
        "command_line": {"session_height": 410.0},
        "unknown_section": [true],
    });
    case.seed_settings(&seed.to_string());

    // Changing the look keeps the display choices.
    settings::save_default_preset(Preset::Realistic).unwrap();
    let mut expected = seed.clone();
    expected["view"]["preset"] = json!("realistic");
    assert_eq!(case.settings_json(), expected, "only the look changed");
    let view = view_unchanged(&case);
    assert_eq!(view.preset, Preset::Realistic);
    let seeded = ReferenceDisplay {
        show_grid: false,
        show_floor: true,
        show_origin_planes: false,
        always_show_origin_axes: true,
    };
    assert_eq!(view.references, seeded);

    // Changing a display choice keeps the look and everything foreign.
    settings::save_reference_display(&ReferenceDisplay { always_show_origin_axes: false, ..seeded }).unwrap();
    expected["view"]["always_show_origin_axes"] = json!(false);
    expected["view"]["show_floor"] = json!(true);
    assert_eq!(case.settings_json(), expected, "only the display choice changed");

    // And back the other way.
    settings::save_default_preset(Preset::Fast).unwrap();
    settings::save_reference_display(&seeded).unwrap();
    expected["view"]["preset"] = json!("fast");
    expected["view"]["always_show_origin_axes"] = json!(true);
    assert_eq!(case.settings_json(), expected, "interleaved writes keep each other's keys");

    // The same through the window: only the choice clicked changes.
    let mut app = launch(empty());
    set_preference(&mut app, ALWAYS_SHOW_AXES, false);
    expected["view"]["always_show_origin_axes"] = json!(false);
    assert_eq!(case.settings_json(), expected, "a toggle in Preferences keeps everything else");
    app.render().expect("the last frame renders");
}

#[test]
fn the_first_display_write_of_a_new_install_makes_the_view_section() {
    let case = Case::settings("first_display_write");
    let chosen = ReferenceDisplay { show_grid: false, ..DEFAULTS };
    settings::save_reference_display(&chosen).unwrap();
    assert_eq!(view_unchanged(&case).references, chosen);
    assert_eq!(case.settings_json()["view"]["show_grid"], json!(false));
}

#[test]
fn navigation_and_view_writes_keep_each_other() {
    let case = Case::settings("navigation_and_view_writes");
    case.seed_settings(
        r#"{"view":{"preset":"studio","show_grid":false,"future":1},"navigation":{"preset_id":"onshape","future":"keep"}}"#,
    );

    let mut navigation = settings::navigation(InstallKind::Existing);
    navigation.select_preset("solidworks").unwrap();
    settings::save_navigation(&navigation).unwrap();
    let saved = case.settings_json();
    assert_eq!(saved["navigation"]["preset_id"], json!("solidworks"));
    assert_eq!(saved["navigation"]["future"], json!("keep"));
    assert_eq!(saved["view"], json!({"preset":"studio","show_grid":false,"future":1}), "a navigation write");

    settings::save_reference_display(&ReferenceDisplay { show_grid: true, ..DEFAULTS }).unwrap();
    settings::save_default_preset(Preset::Fast).unwrap();
    let written = case.settings_json();
    assert_eq!(written["navigation"], saved["navigation"], "display and look writes");
    assert_eq!(written["view"]["future"], json!(1));
    assert_eq!(written["view"]["preset"], json!("fast"));
    assert_eq!(written["view"]["show_grid"], json!(true));
}

#[test]
fn a_display_write_does_not_stop_at_a_newer_navigation_section() {
    let case = Case::settings("newer_navigation");
    let navigation = json!({"version": 99, "preset_id": "from_the_future", "future": {"keep": [1]}});
    case.seed_settings(&json!({ "navigation": navigation }).to_string());
    settings::capture_install_kind();
    settings::save_reference_display(&ReferenceDisplay { always_show_origin_axes: true, ..DEFAULTS }).unwrap();
    let written = case.settings_json();
    assert_eq!(written["navigation"], navigation, "exactly as it was");
    assert_eq!(written["view"]["always_show_origin_axes"], json!(true));
}

#[test]
fn a_new_install_that_writes_display_first_keeps_its_starting_preset() {
    let case = Case::settings("new_install_display_first");
    assert_eq!(settings::capture_install_kind(), InstallKind::New);
    settings::save_reference_display(&ReferenceDisplay { show_grid: false, ..DEFAULTS }).unwrap();

    // The next start finds a file, so it detects an existing install.
    let restarted = settings::capture_install_kind();
    assert_eq!(restarted, InstallKind::Existing);
    assert_eq!(
        settings::navigation(restarted).active_preset().id,
        soliddraft_app::navigation::default_preset(InstallKind::New)
    );
    assert!(!view_unchanged(&case).references.show_grid);
}

#[test]
fn edit_suite_starts_with_origin_axes_and_keeps_an_explicit_choice() {
    let case = Case::settings("edit_suite_axes_default");
    case.seed_settings(r#"{"frame_layout":"edit_suite"}"#);
    assert!(view_unchanged(&case).references.always_show_origin_axes);
    case.seed_settings(r#"{"frame_layout":"edit_suite","view":{"always_show_origin_axes":false}}"#);
    assert!(!view_unchanged(&case).references.always_show_origin_axes);
    case.seed_settings(r#"{"frame_layout":"classic"}"#);
    assert!(view_unchanged(&case).references.always_show_origin_axes);
}

/// Toggles a checkbox of the open Preferences window.
fn toggle(app: &mut support::App, label: &str) {
    app.get_by_role_and_label(egui::accesskit::Role::CheckBox, label).click();
    app.run_steps(3);
}

/// Opens Preferences at Appearance and graphics > Graphics.
fn open_graphics(app: &mut support::App) {
    app.state_mut().state.push(soliddraft_app::state::AppAction::ShowPreferences);
    app.run_steps(4);
    app.get_by_label("Graphics").click();
    app.run_steps(3);
}

/// The view the Current view controls change, as the window shows it.
fn current_view(app: &support::App) -> soliddraft_app::graphics::CurrentView {
    let state = &app.state().state;
    soliddraft_app::graphics::CurrentView {
        display: state.display_mode(),
        ..soliddraft_app::graphics::CurrentView::of(state.active_viewport())
    }
}

#[test]
fn graphics_settings_carry_their_format_and_read_older_records() {
    use soliddraft_app::graphics::{CurvedSurfaces, Graphics, GraphicsPreset};

    let case = Case::settings("graphics_format");
    // A record saved before the format existed keeps its choices.
    case.seed_settings(r#"{"graphics":{"preset":"performance","curved_surfaces":"coarse","samples":1}}"#);
    let old = settings::graphics();
    assert_eq!(
        (old.preset, old.curved_surfaces, old.samples),
        (GraphicsPreset::Performance, Some(CurvedSurfaces::Coarse), 1)
    );
    settings::save_graphics(&old).unwrap();
    assert_eq!(case.settings_json()["graphics"]["format"], "soliddraft-graphics/1");
    assert_eq!(settings::graphics(), old, "the saved record reads back");

    // A later format this build cannot read is not taken for its own.
    case.seed_settings(r#"{"graphics":{"format":"soliddraft-graphics/2","preset":"performance","samples":1}}"#);
    assert_eq!(settings::graphics(), Graphics::default());
}

#[test]
fn appearance_changes_outside_preferences_keep_the_preview_unsaved() {
    use soliddraft_app::state::AppAction;
    use soliddraft_app::theme::{self, Background, BuiltIn, Mode, Selection};
    use soliddraft_app::viewport::scenes::Scene;

    let _case = Case::gpu("appearance_preview_unsaved");
    let mut app = launch(empty());
    let preview = |app: &mut support::App| {
        app.state_mut().state.push(AppAction::ShowPreferences);
        app.run_steps(4);
        app.get_by_label("Appearance").click();
        app.run_steps(3);
        app.get_by_label("Gruvbox Light").click();
        app.run_steps(3);
        assert_eq!(theme::active_theme_id(), "gruvbox-light", "the window previews the theme");
    };

    // A scene chosen in the viewport while the window previews a theme.
    preview(&mut app);
    app.state_mut().state.push(AppAction::SetScene(Scene::Night));
    app.run_steps(3);
    let saved = settings::theme_preferences();
    assert_eq!(saved.selected, Selection::BuiltIn(BuiltIn::ClearBench), "the previewed theme is not saved");
    assert_eq!(saved.background, Background::Scene);
    assert_eq!(settings::view().scene, Scene::Night, "the scene is saved");
    app.get_by_label("Cancel").click();
    app.run_steps(4);
    assert!(app.state().preferences.is_none());
    assert_eq!(theme::active_theme_id(), "clear_bench", "Cancel shows the saved theme");
    assert_eq!(settings::theme_preferences().selected, Selection::BuiltIn(BuiltIn::ClearBench));

    // Light or dark from the top bar while the window previews a theme saves only that choice.
    preview(&mut app);
    app.state_mut().state.push(AppAction::SetTheme(Mode::Dark));
    app.run_steps(3);
    assert_eq!(settings::theme_preferences().selected, Selection::BuiltIn(BuiltIn::ClearBenchDark));
    app.get_by_label("Cancel").click();
    app.run_steps(4);
    assert_eq!(theme::active_theme_id(), "clear-bench-dark");
    assert_eq!(settings::theme_preferences().selected, Selection::BuiltIn(BuiltIn::ClearBenchDark));
}

#[test]
fn cancel_in_graphics_restores_the_current_view() {
    let _case = Case::gpu("graphics_cancel");
    let mut app = launch(support::box_part());
    let before = current_view(&app);
    for close in ["Cancel", "Escape"] {
        open_graphics(&mut app);
        toggle(&mut app, "Perspective");
        toggle(&mut app, "Ground shadow follows the model");
        toggle(&mut app, "Show grid");
        assert_ne!(current_view(&app), before, "the window shows each edit at once");
        if close == "Cancel" {
            app.get_by_label("Cancel").click();
        } else {
            app.key_press(egui::Key::Escape);
        }
        app.run_steps(4);
        assert!(app.state().preferences.is_none(), "{close} closed the window");
        assert_eq!(current_view(&app), before, "{close} restored the current view");
    }
}

#[test]
fn the_ground_shadow_box_follows_the_floor_commands_and_survives_a_look_change() {
    use soliddraft_app::state::AppAction;

    let _case = Case::gpu("ground_shadow_box");
    let mut app = launch(support::box_part());
    let shadow_box = |app: &mut support::App| {
        open_graphics(app);
        let shown = support::checked(app, "Ground shadow");
        support::close_preferences(app);
        shown
    };

    // Under Fast, turning the floor off and on again shows its shadow.
    app.state_mut().state.push(AppAction::SetPreset(Preset::Fast));
    support::settle(&mut app);
    set_preference(&mut app, support::SHOW_FLOOR, false);
    set_preference(&mut app, support::SHOW_FLOOR, true);
    assert!(shadow_box(&mut app), "Show floor turned the ground shadow on under Fast");

    // Turned on under Studio, the shadow stays on under Fast.
    app.state_mut().state.push(AppAction::SetPreset(Preset::Studio));
    support::settle(&mut app);
    open_graphics(&mut app);
    if support::checked(&app, "Ground shadow") {
        toggle(&mut app, "Ground shadow");
    }
    toggle(&mut app, "Ground shadow");
    assert!(support::checked(&app, "Ground shadow"));
    support::close_preferences(&mut app);
    app.state_mut().state.push(AppAction::SetPreset(Preset::Fast));
    support::settle(&mut app);
    assert!(shadow_box(&mut app), "the ground shadow chosen under Studio stays on under Fast");
}

#[test]
fn current_view_controls_change_only_the_view_on_screen() {
    use egui_kittest::kittest::NodeT;
    use soliddraft_app::assembly::AssemblyAction;
    use soliddraft_app::state::AppAction;

    let _case = Case::gpu("current_view_scope");
    let mut app = launch(support::box_part());
    // The occlusion box names the GPU when the adapter has no occlusion pass.
    let enabled = |app: &support::App, label: &str| {
        let boxes: Vec<_> = app
            .get_all_by_role(egui::accesskit::Role::CheckBox)
            .filter(|node| {
                node.accesskit_node()
                    .label()
                    .is_some_and(|text| text == label || text.starts_with(&format!("{label} ·")))
            })
            .collect();
        assert_eq!(boxes.len(), 1, "one box is {label}");
        !boxes[0].accesskit_node().is_disabled()
    };
    app.state_mut().state.push(AppAction::Assembly(AssemblyAction::New));
    support::settle(&mut app);
    assert!(app.state().state.assembly.is_active());

    // The assembly view draws no ground shadow or occlusion, so it offers neither.
    open_graphics(&mut app);
    for label in ["Ground shadow", "Ground shadow follows the model", "Ambient occlusion"] {
        assert!(!enabled(&app, label), "{label} is offered for an assembly");
    }
    assert!(enabled(&app, "Show grid"));

    // Its grid switch changes the assembly's view and no default.
    toggle(&mut app, "Show grid");
    support::close_preferences(&mut app);
    let state = &app.state().state;
    assert!(!state.assembly.session.as_ref().unwrap().viewport.references.show_grid, "the assembly hid its grid");
    assert!(state.viewport.references.show_grid, "the part keeps its grid");
    assert!(settings::view().references.show_grid, "the saved default keeps the grid");

    // With no 3D view on screen there is no current view to change.
    app.state_mut().state.push(AppAction::ShowStartPage);
    support::settle(&mut app);
    assert!(app.state().state.start.is_some());
    let camera = app.state().state.viewport.camera.orthographic;
    open_graphics(&mut app);
    assert!(app.query_by_label("Open a part or an assembly to change its view.").is_some());
    assert!(app.query_by_role_and_label(egui::accesskit::Role::CheckBox, "Perspective").is_none());
    support::close_preferences(&mut app);
    assert_eq!(app.state().state.viewport.camera.orthographic, camera);
}
