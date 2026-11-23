//! D1 Clear Bench shell acceptance tests: top bar & single File menu, document tabs & +,
//! stacked single side column with all-off clear, History rollback controls, Zoo bottom bar
//! popovers, floating display style menu, D1 Preferences Cancel/Apply/OK, and a full
//! build -> edit -> save -> reopen -> later-feature round-trip.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, vec2};
use egui_kittest::kittest::Queryable;
use soliddraft_app::document::{Command, ExtrudeParams, PlaneKind, ProfileKind, Reference, Selection};
use soliddraft_app::left_bar;
use soliddraft_app::panels::Pane;
use soliddraft_app::state::{AppAction, DisplayUnit};
use soliddraft_app::theme;
use soliddraft_app::viewport::render::DisplayMode;

#[test]
fn d1_top_bar_single_file_menu_tabs_and_utilities() {
    let case = ribbon::case("d1-top-bar");
    let mut app = ribbon::app(soliddraft_app::demo_document());
    case.run(&mut app, |app| {
        // Legacy 7-item menu row is gone; single File button remains.
        assert!(app.query_by_label("File").is_some(), "single File button present");
        for removed in ["Edit", "Insert", "Tools", "Window", "Help"] {
            assert!(
                app.query_all_by_role_and_label(egui::accesskit::Role::Button, removed).next().is_none(),
                "legacy top menu `{removed}` must be removed"
            );
        }
        for required in ["Home", "Save", "Undo", "Redo", "New tab", "Donate", "Preferences", "Report a bug"] {
            assert!(app.query_by_label(required).is_some(), "top bar has `{required}`");
        }
        let version = concat!("v", env!("CARGO_PKG_VERSION"));
        assert!(app.query_by_label(version).is_some(), "top bar shows crate version {version}");

        // Open single File dropdown and verify 17.png structure.
        app.get_by_label("File").click();
        app.run_steps(6);
        for item in [
            "New Part",
            "New Assembly",
            "New Drawing",
            "Open…",
            "Export STEP AP242…",
            "Export STL…",
            "Design Properties…",
            "Customize App Bar…",
            "Keyboard Shortcuts…",
            "Preferences…",
        ] {
            assert!(app.query_by_label_contains(item).is_some(), "File menu has `{item}`");
        }
        app.key_press(Key::Escape);
        app.run_steps(4);

        // Open Donate popover and confirm real links are shown.
        app.get_by_label("Donate").click();
        app.run_steps(6);
        assert!(app.query_by_label("Support on Ko-fi").is_some());
        assert!(app.query_by_label("GitHub Sponsors").is_some());
        app.key_press(Key::Escape);
        app.run_steps(4);

        // Document tabs + `+` button: open a second Part tab, switch back, then close it.
        assert_eq!(app.state().open_parts.total(), 1);
        app.get_by_label("New tab").click();
        app.run_steps(6);
        app.get_by_label("New Part (.dwprt)").click();
        app.run_steps(8);
        assert_eq!(app.state().open_parts.total(), 2);
        assert_eq!(app.state().state.doc.name, "Part2");
        assert!(app.state().state.doc.features.is_empty());

        // Switch back to Part1.dwprt and verify its features stayed intact.
        app.get_by_label("Part1.dwprt").click();
        app.run_steps(8);
        assert_eq!(app.state().state.doc.name, "Part1");
        assert_eq!(app.state().state.doc.features.len(), 3);

        // Close Part2.dwprt via its tab X.
        app.get_by_label("Close Part2.dwprt").click();
        app.run_steps(6);
        assert_eq!(app.state().open_parts.total(), 1);
    });
}

#[test]
fn d1_stacked_side_column_history_rollback_and_all_off_clear() {
    let case = ribbon::case("d1-side-column");
    let mut app = ribbon::app(soliddraft_app::demo_document());
    case.run(&mut app, |app| {
        // Default D1 Clear Bench opens Browser (Tree) + History stacked in one column.
        assert!(app.state().state.pane_shown(Pane::Tree));
        assert!(app.state().state.pane_shown(Pane::History));
        assert!(!app.state().state.pane_shown(Pane::Values));
        assert!(!app.state().state.pane_shown(Pane::Props));
        let vp_two_stacked = app.state().state.viewport.rect;

        // Toggle Values and Props on — all 4 stack vertically in the SAME column!
        app.get_by_label("Values").click();
        app.run_steps(6);
        app.get_by_label("Props").click();
        app.run_steps(6);
        assert!(app.state().state.pane_shown(Pane::Values));
        assert!(app.state().state.pane_shown(Pane::Props));
        assert!(
            (app.state().state.viewport.rect.left() - vp_two_stacked.left()).abs() < 2.0,
            "stacking panes uses ONE column, never adding a second side column"
        );

        // Close Props and Values via each pane's own close X.
        app.get_by_label("Close Props").click();
        app.run_steps(6);
        assert!(!app.state().state.pane_shown(Pane::Props));
        app.get_by_label("Close Values").click();
        app.run_steps(6);
        assert!(!app.state().state.pane_shown(Pane::Values));

        // History rollback step controls actually drive Command::Rollback on the model.
        assert_eq!(app.state().state.doc.rollback, None);
        app.get_by_label("Step back").click();
        app.run_steps(8);
        assert_eq!(app.state().state.doc.rollback, Some(2));
        app.get_by_label("Roll to start").click();
        app.run_steps(8);
        assert_eq!(app.state().state.doc.rollback, Some(0));
        app.get_by_label("Step forward").click();
        app.run_steps(8);
        assert_eq!(app.state().state.doc.rollback, Some(1));
        app.get_by_label("Roll to end").click();
        app.run_steps(8);
        assert_eq!(app.state().state.doc.rollback, None);

        // Clicking remaining panes off (or Clear) clears the column completely to the 46px rail.
        app.get_by_label("Close History").click();
        app.run_steps(6);
        app.get_by_label("Close Browser").click();
        app.run_steps(6);
        assert!(!app.state().state.tree_shown());
        assert!(
            (app.state().state.viewport.rect.left() - left_bar::WIDTH).abs() <= 2.0,
            "all-off clears the side column completely to the 46px left rail: got {}",
            app.state().state.viewport.rect.left()
        );
    });
}

#[test]
fn d1_bottom_bar_popovers_floating_view_bar_preferences_and_sketch_mode() {
    let case = ribbon::case("d1-bottom-and-prefs");
    let mut app = ribbon::app(ribbon::plate());
    ribbon::until(&mut app, "plate proof settles", |app| !app.state().state.doc.proof_pending());
    case.run(&mut app, |app| {
        assert!(!theme::is_dark(), "D1 Clear Bench daylight is default");
        assert_eq!(theme::active_theme_id(), "clear_bench");

        // Floating view bar display style menu (captain-attachment.png model).
        app.get_by_label("Display style").click();
        app.run_steps(6);
        assert!(app.query_by_label("Shaded with edges").is_some());
        assert!(app.query_by_label("Shaded").is_some());
        assert!(app.query_by_label("Wireframe").is_some());
        assert!(app.query_by_label("Shaded with hidden edges").is_some());
        assert!(app.query_by_label("Translucent").is_some());
        app.get_by_label("Wireframe").click();
        app.run_steps(6);
        assert_eq!(app.state().state.viewport.display, DisplayMode::Wireframe);

        // Bottom bar: Selection filter popover (Zoo 27.png).
        app.get_by_label("Selection filter").click();
        app.run_steps(6);
        app.get_by_label("Faces").click();
        app.run_steps(6);
        assert!(!app.state().state.pick_filter.faces);
        assert!(app.state().state.pick_filter.edges);
        app.key_press(Key::Escape);
        app.run_steps(4);

        // Bottom bar: Default units popover (Zoo 29.png).
        assert_eq!(app.state().state.display_unit, DisplayUnit::Mm);
        app.get_by_label("Default units").click();
        app.run_steps(6);
        app.get_by_label("Inches").click();
        app.run_steps(6);
        assert_eq!(app.state().state.display_unit, DisplayUnit::In);

        // Bottom bar: Physical analysis popover (Zoo 28.png) computes real mass properties.
        app.get_by_label("Physical analysis").click();
        app.run_steps(10);
        ribbon::until(app, "mass properties computed", |app| {
            let body = app.state().state.doc.display_body().unwrap();
            app.state().state.doc.kernel().cached_mass_properties(body).is_some()
        });
        app.run_steps(6);
        assert!(ribbon::painted(app).iter().any(|p| p.text.contains("Physical analysis")));
        app.key_press(Key::Escape);
        app.run_steps(4);

        // Preferences gear opens D1 Preferences modal with Cancel revert & OK commit.
        app.get_by_label("Preferences").click();
        app.run_steps(6);
        assert!(app.state().preferences.is_some());
        assert!(app.query_by_label("Restore page defaults").is_some());
        assert!(app.query_by_label("Profile: Default").is_some());
        assert!(app.query_by_label("Cancel").is_some());
        assert!(app.query_by_label("Apply").is_some());
        assert!(app.query_by_label("OK").is_some());

        let initial_preset = app.state().state.navigation.preset_id.clone();
        app.get_by_label("SOLIDWORKS").click();
        app.run_steps(4);
        assert_eq!(app.state().state.navigation.preset_id, "solidworks");
        app.get_by_label("Cancel").click();
        app.run_steps(6);
        assert!(app.state().preferences.is_none());
        assert_eq!(app.state().state.navigation.preset_id, initial_preset, "Cancel reverts changes");

        // Sketch mode: top strip switches to Sketch with red Close sketch button.
        app.state_mut().state.doc.selection = Some(Selection::Reference(Reference::Plane(PlaneKind::Top)));
        app.get_by_label("Start sketch").click();
        app.run_steps(8);
        assert!(app.state().state.in_sketch());
        assert!(app.query_by_label("Close sketch").is_some());
        app.get_by_label("Close sketch").click();
        app.run_steps(8);
        assert!(!app.state().state.in_sketch());
    });
}

#[test]
fn d1_build_edit_save_reopen_and_feature_on_top_round_trip() {
    let case = ribbon::case("d1-round-trip");
    let mut app = ribbon::app_at(ribbon::blank(), vec2(1440.0, 900.0), 1.0);
    case.run(&mut app, |app| {
        // 1. Build an extrude (60 x 40 x 20 = 48000 mm^3).
        app.state_mut().state.push(AppAction::Command(Command::AddExtrude {
            params: ExtrudeParams {
                profile: ProfileKind::Rectangle { width: 60.0, height: 40.0 },
                depth: 20.0,
                ..Default::default()
            },
            via_assistant: false,
        }));
        app.run_steps(10);
        ribbon::until(app, "initial extrude settles", |app| !app.state().state.doc.proof_pending());
        let id = app.state().state.doc.features[0].id;

        // 2. Edit the feature depth to 30 mm (60 x 40 x 30 = 72000 mm^3).
        app.state_mut().state.push(AppAction::Command(Command::EditExtrude {
            id,
            params: ExtrudeParams {
                profile: ProfileKind::Rectangle { width: 60.0, height: 40.0 },
                depth: 30.0,
                ..Default::default()
            },
        }));
        app.run_steps(10);
        ribbon::until(app, "edited extrude settles", |app| !app.state().state.doc.proof_pending());

        // 3. Save to a real `.dwprt` file.
        let path = case.path("d1-round-trip.dwprt");
        app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
        app.run_steps(10);
        assert!(path.exists(), "saved .dwprt exists");

        // 4. Reopen the file.
        app.state_mut().state.push(AppAction::NewPart);
        app.run_steps(8);
        assert!(app.state().state.doc.features.is_empty());
        app.state_mut().state.push(AppAction::Open(Some(path.clone())));
        app.run_steps(10);
        ribbon::until(app, "reopened part settles", |app| !app.state().state.doc.proof_pending());
        assert_eq!(app.state().state.doc.features.len(), 1);
        let body = app.state().state.doc.display_body().expect("reopened body");
        let v1 = app.state().state.doc.kernel().mass_properties(body).unwrap().volume;
        assert!((v1 - 72_000.0).abs() < 1e-6, "reopened volume: {v1}");

        // 5. Add a later feature on top (a 20 mm tall, 20 mm diameter boss on top).
        app.state_mut().state.push(AppAction::Command(Command::AddExtrude {
            params: ExtrudeParams {
                profile: ProfileKind::Circle { diameter: 20.0 },
                plane: PlaneKind::Top,
                depth: 40.0,
                ..Default::default()
            },
            via_assistant: false,
        }));
        app.run_steps(10);
        ribbon::until(app, "second feature settles", |app| !app.state().state.doc.proof_pending());
        assert_eq!(app.state().state.doc.features.len(), 2);
        let body2 = app.state().state.doc.display_body().expect("body after second feature");
        let v2 = app.state().state.doc.kernel().mass_properties(body2).unwrap().volume;
        let expected = 72_000.0 + std::f64::consts::PI * 100.0 * 10.0;
        assert!((v2 - expected).abs() < 1e-3, "volume after boss on top: {v2} vs {expected}");

        // 6. Open the saved file the way + > Open Existing File does: it takes a new tab
        // and the part with the boss stays open beside it.
        let tabs = app.state().open_parts.total();
        app.state_mut().state.open_in_new_tab = true;
        app.state_mut().state.push(AppAction::Open(Some(path.clone())));
        app.run_steps(10);
        ribbon::until(app, "part in the new tab settles", |app| !app.state().state.doc.proof_pending());
        assert_eq!(app.state().open_parts.total(), tabs + 1, "the file opened in a new tab");
        assert!(!app.state().state.open_in_new_tab, "the request is spent");
        assert_eq!(app.state().state.doc.features.len(), 1, "the new tab shows the saved part");
        let parked = app.state().open_parts.before.last().expect("the earlier part is parked");
        assert_eq!(parked.doc.features.len(), 2, "the part with the boss kept its unsaved feature");
        assert!(parked.doc.dirty);
    });
}
