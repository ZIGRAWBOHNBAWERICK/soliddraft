//! Options bar acceptance through the real app, pointer, command dispatcher and renderer.

#[path = "support/ui.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use egui::{Key, Rect, vec2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::document::{PlaneKind, Reference, Selection};

use soliddraft_app::state::{EditorParams, Tool};
use soliddraft_app::theme::Mode;
use soliddraft_app::viewport::{PickFilter, render::DisplayMode};
use ux_tools::{Case, Look, STANDARD, launch, settle};

fn bar(app: &App) -> Rect {
    soliddraft_app::options_bar::shown_rect(&app.ctx).expect("the options bar is drawn")
}

fn click(app: &mut App, label: &str) {
    let button = ribbon::control(app, label, bar(app));
    assert!(button.enabled, "{label} is enabled");
    ribbon::click_at(app, button.rect.center());
    settle(app);
}

fn text(app: &App, expected: &str) {
    let painted = ribbon::painted_in(app, bar(app));
    assert!(painted.iter().any(|p| p.text == expected), "{expected} is visible: {painted:?}");
}

fn edit_suite(app: &mut App) {
    app.state_mut().state.doc.selection = None;
    settle(app);
}

#[test]
fn empty_selection_has_pick_filters_and_views_with_display_on_navigation() {
    let mut case = Case::new("options-bar-views");
    let mut app = launch(ribbon::plate(), STANDARD);
    case.run(&mut app, |case, app| {
        edit_suite(app);
        assert!(
            soliddraft_app::options_bar::shown_rect(&app.ctx).is_none(),
            "a part with nothing selected has no options bar"
        );
        let before = ribbon::fingerprint(app);
        // The pick filters are in the bottom bar's Selection filter.
        assert_eq!(app.state().state.pick_filter, PickFilter::default());
        let filter = app.get_by_role_and_label(egui::accesskit::Role::Button, "Selection filter").rect();
        ribbon::click_at(app, filter.center());
        settle(app);
        for label in ["All", "Faces", "Edges & vertices", "Bodies"] {
            assert!(app.query_all_by_label(label).next().is_some(), "the Selection filter lists {label}");
        }
        app.key_press(Key::Escape);
        settle(app);
        let display = app.state().state.viewport.display;
        let navigation_display = app.get_by_label("Display style").rect();
        assert!(app.state().state.viewport.rect.contains_rect(navigation_display));
        ribbon::click_at(app, navigation_display.center());
        settle(app);
        assert_eq!(app.state().state.viewport.display, display, "opening the menu preserves the display");
        let wireframe = app.get_by_label("Wireframe").rect();
        ribbon::click_at(app, wireframe.center());
        settle(app);
        assert_eq!(
            app.state().state.viewport.display,
            DisplayMode::Wireframe,
            "navigation applies the selected display style"
        );
        assert_ne!(app.state().state.viewport.display, display, "navigation changes the selected display mode");
        // The named views are on the View tab.
        ribbon::switch_workspace(app, "view");
        strip(app, "Front");
        let front = ribbon::fingerprint(app).camera;
        strip(app, "Top");
        assert_ne!(ribbon::fingerprint(app).camera, front, "view controls move the camera");
        strip(app, "Isometric");
        strip(app, "Fit");
        let after = ribbon::fingerprint(app);
        assert_eq!(before.features, after.features);
        assert_eq!(before.undo_depth, after.undo_depth);
        assert_eq!(before.dirty, after.dirty);
        assert_eq!(after.selection, None);
        case.frame(app, "no-selection");
    });
}

/// Clicks a tool of the strip, in its group's More menu when the strip has no room for it.
fn strip(app: &mut App, label: &str) {
    ribbon::click_button(app, label, ribbon::palette_region(app));
    settle(app);
}

/// Whether the bottom bar's Selection filter is open.
fn filter_open(app: &App) -> bool {
    app.query_all_by_label("Edges & vertices").next().is_some()
}

/// Turns a pick filter on or off in the bottom bar's Selection filter, and closes it again.
fn toggle_filter(app: &mut App, label: &str) {
    for _ in 0..2 {
        if filter_open(app) {
            break;
        }
        let filter = app.get_by_role_and_label(egui::accesskit::Role::Button, "Selection filter").rect();
        ribbon::click_at(app, filter.center());
        settle(app);
    }
    let checkbox = app.get_by_label(label).rect();
    ribbon::click_at(app, checkbox.center());
    settle(app);
    if filter_open(app) {
        let filter = app.get_by_role_and_label(egui::accesskit::Role::Button, "Selection filter").rect();
        ribbon::click_at(app, filter.center());
        settle(app);
    }
}

#[test]
fn pick_filters_control_real_picks_without_restricting_tools() {
    let mut case = Case::new("options-bar-pick-filter");
    let mut app = launch(ribbon::plate(), STANDARD);
    edit_suite(&mut app);
    case.run(&mut app, |case, app| {
        ribbon::run_command(app, "zoom_fit");
        settle(app);
        let before = ribbon::fingerprint(app);
        let face = glam::Vec3::new(0.0, 0.0, 20.0);
        let edge = app.state().state.doc.edges.iter().find(|e| (e.midpoint.z - 20.0).abs() < 1e-9).unwrap().clone();

        toggle_filter(app, "Faces");
        toggle_filter(app, "Bodies");
        ribbon::click_world(app, face);
        settle(app);
        assert_eq!(app.state().state.doc.selection, None, "a disabled face does not pick a plane behind it");
        ribbon::click_world(app, edge.midpoint);
        settle(app);
        assert_eq!(app.state().state.doc.selection, Some(Selection::Edge(edge.id)));
        app.state_mut().state.doc.selection = None;
        settle(app);

        toggle_filter(app, "Edges & vertices");
        toggle_filter(app, "Bodies");
        ribbon::click_world(app, face);
        settle(app);
        assert_eq!(app.state().state.doc.selection, Some(Selection::Body));
        case.frame(app, "body-only");
        app.state_mut().state.doc.selection = None;
        settle(app);

        toggle_filter(app, "Faces");
        ribbon::click_world(app, edge.midpoint);
        settle(app);
        assert!(matches!(app.state().state.doc.selection, Some(Selection::Face(_))), "faces remain reachable at an edge");
        app.state_mut().state.doc.selection = None;
        settle(app);
        toggle_filter(app, "Bodies");
        let at = app.state().state.viewport.camera.project(face, app.state().state.viewport.rect).unwrap();
        ribbon::render_steps::await_pick(app, at);
        ribbon::double_click_at(app, at);
        settle(app);
        assert!(matches!(app.state().state.doc.selection, Some(Selection::Face(_))), "double-click respects Bodies off");
        app.state_mut().state.doc.selection = None;
        settle(app);

        toggle_filter(app, "Faces");
        assert_eq!(app.state().state.pick_filter, PickFilter { faces: false, edges: false, bodies: false });
        ribbon::click_world(app, face);
        settle(app);
        assert_eq!(app.state().state.doc.selection, None);
        ribbon::run_command(app, "fillet");
        settle(app);
        ribbon::click_world(app, edge.midpoint);
        settle(app);
        assert!(matches!(app.state().state.editor.as_ref().map(|e| &e.params), Some(EditorParams::Fillet(p)) if p.edges.len() == 1));
        app.key_press(Key::Escape);
        settle(app);

        let after = ribbon::fingerprint(app);
        assert_eq!(before.features, after.features);
        assert_eq!(before.undo_depth, after.undo_depth);
        assert_eq!(before.dirty, after.dirty);
    });
}

#[test]
fn picked_geometry_shows_dimensions_and_starts_rounding_with_the_pick() {
    let mut case = Case::new("options-bar-selection");
    let mut app = launch(ribbon::plate(), STANDARD);
    edit_suite(&mut app);
    // The bar's whole width, without the side column beside it, for every value of a selection.
    ribbon::close_tree(&mut app);
    case.run(&mut app, |case, app| {
        ribbon::run_command(app, "zoom_fit");
        settle(app);
        ribbon::click_world(app, glam::Vec3::new(0.0, 0.0, 20.0));
        settle(app);
        assert!(matches!(app.state().state.doc.selection, Some(Selection::Face(_))));
        let feature = app.state().state.doc.features[0].name.clone();
        for expected in [format!("{feature} · Width"), "60 mm".into(), format!("{feature} · Height"), "40 mm".into(), format!("{feature} · Depth"), "20 mm".into(), "Area".into()] {
            text(app, &expected);
        }
        assert!(ribbon::buttons_in(app, bar(app)).iter().all(|(name, _)| name != "Wireframe"));
        case.frame(app, "face");

        let edge = app.state().state.doc.edges.iter().find(|e| (e.midpoint.z - 20.0).abs() < 1e-9).unwrap().clone();
        ribbon::click_world(app, edge.midpoint);
        settle(app);
        assert_eq!(app.state().state.doc.selection, Some(Selection::Edge(edge.id)));
        text(app, "Length");
        text(app, &format!("{:.2} mm", edge.length));
        case.frame(app, "edge");
        click(app, "Fillet");
        assert!(matches!(app.state().state.editor.as_ref().map(|e| &e.params), Some(EditorParams::Fillet(p)) if p.edges.len() == 1));
        assert!(app.query_all_by_label("Active tool: Fillet").next().is_some());
        case.frame(app, "fillet");
        app.key_press(Key::Escape);
        settle(app);
        ribbon::click_world(app, edge.midpoint);
        settle(app);
        click(app, "Chamfer");
        assert!(matches!(app.state().state.editor.as_ref().map(|e| &e.params), Some(EditorParams::Chamfer(p)) if p.edges.len() == 1));
        assert!(app.query_all_by_label("Active tool: Chamfer").next().is_some());
        app.key_press(Key::Escape);
        settle(app);

        app.state_mut().state.doc.selection = Some(Selection::Body);
        settle(app);
        for expected in ["Solid body", "60 mm", "40 mm", "20 mm"] {
            text(app, expected);
        }
        case.frame(app, "body");
        app.state_mut().state.doc.selection = None;
        settle(app);
        assert!(
            soliddraft_app::options_bar::shown_rect(&app.ctx).is_none(),
            "the bar and its selection values go with the selection"
        );
    });
}

#[test]
fn sketch_tools_follow_the_palette_and_the_bar_fits_small_windows() {
    let mut case = Case::new("options-bar-sketch");
    for mode in [Mode::Light, Mode::Dark] {
        let look = Look { size: vec2(960.0, 600.0), pixels_per_point: 2.0, theme: Some(mode) };
        let mut app = launch(ribbon::blank(), look);
        edit_suite(&mut app);
        case.run(&mut app, |case, app| {
            case.frame(app, &format!("small-{mode:?}"));
            app.state_mut().state.doc.selection = Some(Selection::Reference(Reference::Plane(PlaneKind::Top)));
            ribbon::run_command(app, "sketch");
            settle(app);
            // An open sketch gives the bar a row of its own above the viewport.
            assert!(ribbon::window(app).contains_rect(bar(app)));
            assert!(bar(app).bottom() <= app.state().state.viewport.rect.top());
            for (id, label, tool) in [
                ("sketch_line", "Line", Tool::Line),
                ("sketch_move", "Move entities", Tool::Move),
                ("smart_dimension", "Smart Dimension", Tool::Dimension),
            ] {
                ribbon::run_command(app, id);
                settle(app);
                assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, tool);
                assert!(app.query_all_by_label(&format!("Active tool: {label}")).next().is_some());
                text(app, label);
                assert!(
                    ribbon::painted_in(app, ribbon::palette_region(app)).iter().any(|p| p.text == "Close sketch"),
                    "Close sketch is on the tool strip"
                );
                for forbidden in ["Fillet", "Chamfer", "Display"] {
                    assert!(!ribbon::painted_in(app, bar(app)).iter().any(|p| p.text == forbidden));
                }
            }
            case.frame(app, &format!("dimension-{mode:?}"));
        });
    }
}
