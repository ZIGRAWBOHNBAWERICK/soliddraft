//! A sketch-based feature starts in either order: pick the sketch and then the feature, or
//! pick the feature and let it ask for the sketch.

#[path = "support/ui.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use egui::Rect;
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::document::{FeatureKind, Selection};
use soliddraft_app::state::{AppAction, EditorParams, ProfileFeature};
use ux_tools::{Case, STANDARD, launch, settle};

fn bar(app: &App) -> Rect {
    egui::PanelState::load(&app.ctx, egui::Id::new("options_bar")).unwrap().outer_rect
}

fn click_bar(app: &mut App, label: &str) {
    let rect = ribbon::control(app, label, bar(app)).rect;
    ribbon::click_at(app, rect.center());
    settle(app);
}

fn params(app: &App) -> Option<EditorParams> {
    app.state().state.editor.as_ref().map(|e| e.params.clone())
}

fn sketch_id(app: &App) -> u64 {
    app.state().state.doc.features.iter().find(|f| f.is_sketch()).expect("a sketch").id
}

/// Clicks the feature tree's row for `name`, not the options bar's button of that name.
fn click_tree_row(app: &mut App, name: &str) {
    ribbon::open_tree(app);
    let row = app
        .get_all_by_label(name)
        .map(|n| n.rect())
        .find(|r| !bar(app).intersects(*r))
        .unwrap_or_else(|| panic!("a tree row for {name}"));
    ribbon::click_at(app, row.center());
    settle(app);
}

/// Clicks a point on the open sketch's plane, as a drawing tool takes it.
fn click_on_sketch(app: &mut App, world: glam::Vec3) {
    let point = {
        let state = &app.state().state;
        state.viewport.camera.project(world, state.viewport.rect).expect("the point is on screen")
    };
    ribbon::click_at(app, point);
}

/// The volume of the part after its last feature.
fn volume(app: &App) -> f64 {
    let doc = &app.state().state.doc;
    doc.mass_properties(doc.features.last().expect("a feature").id).expect("a body").volume
}

/// Whether the options bar shows a button of that name.
fn on_bar(app: &App, label: &str) -> bool {
    app.query_all_by_role_and_label(egui::accesskit::Role::Button, label).any(|n| bar(app).intersects(n.rect()))
}

#[test]
fn sketch_first_then_feature_still_opens_the_feature() {
    let mut case = Case::new("feature-sketch-order/sketch-first");
    let mut app = launch(ribbon::sketch(), STANDARD);
    case.run(&mut app, |case, app| {
        let sketch = sketch_id(app);
        click_tree_row(app, "Sketch1");
        assert_eq!(app.state().state.doc.selection, Some(Selection::Feature(sketch)));
        case.frame(app, "1-sketch-selected");

        ribbon::run_command(app, "extrude");
        settle(app);
        assert!(
            matches!(params(app), Some(EditorParams::Extrude(p)) if p.sketch == Some(sketch)),
            "the selected sketch goes straight into the extrude: {:?}",
            params(app)
        );
        case.frame(app, "2-extrude-from-selected-sketch");
        click_bar(app, "OK");
        assert!(app.state().state.editor.is_none());
        assert!(matches!(
            &app.state().state.doc.features.last().unwrap().kind,
            FeatureKind::Extrude(p) if p.sketch == Some(sketch)
        ));
    });
}

#[test]
fn feature_first_asks_for_a_sketch_and_continues_with_the_one_picked() {
    let mut case = Case::new("feature-sketch-order/feature-first-existing");
    let mut app = launch(ribbon::sketch(), STANDARD);
    case.run(&mut app, |case, app| {
        let sketch = sketch_id(app);
        assert_eq!(app.state().state.doc.selection, None);
        for (id, feature) in [("revolve", ProfileFeature::Revolve), ("sweep", ProfileFeature::Sweep)] {
            let command = soliddraft_app::commands::get(id).unwrap();
            assert!(command.is_enabled(&app.state().state), "{id} is available with nothing selected");
            ribbon::run_command(app, id);
            settle(app);
            assert_eq!(params(app), Some(EditorParams::PickSketch(feature)), "{id} asks for its sketch");
            click_bar(app, "Cancel");
            assert!(app.state().state.editor.is_none());
        }

        ribbon::run_command(app, "extrude");
        settle(app);
        assert_eq!(params(app), Some(EditorParams::PickSketch(ProfileFeature::Extrude)));
        assert!(app.query_all_by_label("OK").next().is_none(), "nothing to confirm before the sketch is chosen");
        let shown = |app: &App, id| {
            soliddraft_app::tree_visibility::shown(&app.state().state, soliddraft_app::document::ViewItem::Feature(id))
        };
        assert!(shown(app, sketch), "the sketch it could use shows in the viewport");
        for label in ["New sketch", "Sketch1", "Cancel"] {
            assert!(bar(app).contains_rect(ribbon::control(app, label, bar(app)).rect), "{label} is on the bar");
        }
        case.frame(app, "1-extrude-asks-for-sketch");

        // The sketch's row in the tree answers the question.
        click_tree_row(app, "Sketch1");
        assert!(
            matches!(params(app), Some(EditorParams::Extrude(p)) if p.sketch == Some(sketch)),
            "the extrude continues with the picked sketch: {:?}",
            params(app)
        );
        case.frame(app, "2-extrude-continues-with-sketch");
        click_bar(app, "OK");
        assert!(matches!(
            &app.state().state.doc.features.last().unwrap().kind,
            FeatureKind::Extrude(p) if p.sketch == Some(sketch)
        ));

        // The used sketch is no longer offered; picking it says why.
        ribbon::run_command(app, "revolve");
        settle(app);
        assert_eq!(params(app), Some(EditorParams::PickSketch(ProfileFeature::Revolve)));
        assert!(!on_bar(app, "Sketch1"), "a used sketch is not offered");
        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(sketch))));
        settle(app);
        let error = app.state().state.editor.as_ref().and_then(|e| e.error.clone()).unwrap_or_default();
        assert!(error.contains("already used"), "{error}");
        case.frame(app, "3-used-sketch-refused");
    });
}

#[test]
fn feature_first_draws_a_new_sketch_on_a_face_and_then_continues() {
    let mut case = Case::new("feature-sketch-order/feature-first-new");
    let mut app = launch(ribbon::plate(), STANDARD);
    case.run(&mut app, |case, app| {
        let before = volume(app);
        ribbon::run_command(app, "cut");
        settle(app);
        assert_eq!(params(app), Some(EditorParams::PickSketch(ProfileFeature::Cut)));
        case.frame(app, "1-cut-asks-for-sketch");

        // A flat face starts a new sketch on it for the cut.
        ribbon::click_world(app, glam::Vec3::new(10.0, 5.0, 20.0));
        settle(app);
        let session = app.state().state.sketch.as_ref().expect("a sketch on the face");
        assert_eq!(session.then_feature, Some(ProfileFeature::Cut));
        let sketch = session.feature;
        ribbon::run_command(app, "sketch_circle");
        click_on_sketch(app, glam::Vec3::new(0.0, 0.0, 20.0));
        click_on_sketch(app, glam::Vec3::new(8.0, 0.0, 20.0));
        settle(app);
        case.frame(app, "2-profile-drawn");

        // Leaving the sketch picks the cut up again with it.
        ribbon::run_command(app, "exit_sketch");
        settle(app);
        assert!(
            matches!(params(app), Some(EditorParams::Cut(p)) if p.sketch == sketch),
            "the cut continues with the new sketch: {:?}",
            params(app)
        );
        case.frame(app, "3-cut-continues-with-new-sketch");
        click_bar(app, "OK");
        assert!(app.state().state.editor.is_none());
        assert!(volume(app) < before - 1.0, "the cut removed material: {before} -> {}", volume(app));
    });
}

#[test]
fn new_sketch_button_asks_for_a_plane_and_an_empty_sketch_asks_again() {
    let mut case = Case::new("feature-sketch-order/new-sketch-button");
    let mut app = launch(ribbon::blank(), STANDARD);
    case.run(&mut app, |case, app| {
        ribbon::run_command(app, "extrude");
        settle(app);
        click_bar(app, "New sketch");
        assert_eq!(params(app), Some(EditorParams::PickPlane));
        assert_eq!(app.state().state.editor.as_ref().unwrap().then_feature, Some(ProfileFeature::Extrude));
        case.frame(app, "1-plane-picker");
        click_bar(app, "Top plane");
        assert_eq!(app.state().state.sketch.as_ref().and_then(|s| s.then_feature), Some(ProfileFeature::Extrude));

        // Nothing drawn: the sketch goes and the extrude asks again.
        ribbon::run_command(app, "exit_sketch");
        settle(app);
        assert!(app.state().state.doc.features.is_empty());
        assert_eq!(params(app), Some(EditorParams::PickSketch(ProfileFeature::Extrude)));

        // The sized rectangle still extrudes without a sketch.
        click_bar(app, "Rectangle");
        assert!(matches!(params(app), Some(EditorParams::Extrude(p)) if p.sketch.is_none()));
        click_bar(app, "OK");
        assert!((volume(app) - 48_000.0).abs() < 1.0);
    });
}
