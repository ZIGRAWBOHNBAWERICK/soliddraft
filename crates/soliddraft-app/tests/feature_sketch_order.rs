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

/// Extrude, New sketch, then the palette's Rectangle before a plane: the feature still waits
/// for the sketch through the plane picker.
fn rectangle_before_the_plane(app: &mut App) {
    ribbon::run_command(app, "extrude");
    settle(app);
    click_bar(app, "New sketch");
    let rectangle = ribbon::reveal_command(app, "sketch_rectangle").rect;
    ribbon::click_at(app, rectangle.center());
    settle(app);
    let editor = app.state().state.editor.as_ref().expect("the plane picker is still open");
    assert_eq!(editor.params, EditorParams::PickPlane);
    assert_eq!(editor.then_feature, Some(ProfileFeature::Extrude), "the tool keeps the waiting extrude");
    click_bar(app, "Top plane");
    let session = app.state().state.sketch.as_ref().expect("a sketch on the Top plane");
    assert_eq!(session.tool, soliddraft_app::state::Tool::Rectangle);
    assert_eq!(session.then_feature, Some(ProfileFeature::Extrude));
}

#[test]
fn a_sketch_tool_before_the_plane_keeps_the_feature_waiting() {
    let mut case = Case::new("feature-sketch-order/tool-before-plane");
    let mut app = launch(ribbon::blank(), STANDARD);
    case.run(&mut app, |case, app| {
        rectangle_before_the_plane(app);
        let sketch = app.state().state.sketch.as_ref().unwrap().feature;
        click_on_sketch(app, glam::Vec3::new(-20.0, -10.0, 0.0));
        click_on_sketch(app, glam::Vec3::new(20.0, 10.0, 0.0));
        settle(app);
        case.frame(app, "1-rectangle-drawn");

        ribbon::run_command(app, "exit_sketch");
        settle(app);
        assert!(
            matches!(params(app), Some(EditorParams::Extrude(p)) if p.sketch == Some(sketch)),
            "the extrude continues with the rectangle's sketch: {:?}",
            params(app)
        );
        case.frame(app, "2-extrude-continues");
    });
}

#[test]
fn a_sketch_tool_before_the_plane_then_an_empty_sketch_asks_again() {
    let mut case = Case::new("feature-sketch-order/tool-before-plane-empty");
    let mut app = launch(ribbon::blank(), STANDARD);
    case.run(&mut app, |case, app| {
        rectangle_before_the_plane(app);
        ribbon::run_command(app, "exit_sketch");
        settle(app);
        assert!(app.state().state.doc.features.is_empty(), "the empty sketch is gone");
        assert_eq!(params(app), Some(EditorParams::PickSketch(ProfileFeature::Extrude)), "the extrude asks again");
        case.frame(app, "1-extrude-asks-again");
    });
}

/// Closing a sketch leaves the view looking straight at its plane. An extrusion grows
/// along that line of sight, so its preview was a flat tint there and a typed depth
/// changed nothing on screen. Opening the feature now turns the view to isometric. A view
/// the user already turned stays as it is.
#[test]
fn a_feature_opened_from_the_sketch_view_turns_to_show_its_depth() {
    let mut case = Case::new("feature-sketch-order/depth-shows");
    let mut app = launch(ribbon::sketch(), STANDARD);
    case.run(&mut app, |case, app| {
        let sketch = sketch_id(app);
        let normal = {
            let plane = app.state().state.doc.feature(sketch).unwrap().plane.unwrap();
            glam::Vec3::new(plane.normal.x as f32, plane.normal.y as f32, plane.normal.z as f32)
        };
        let along = |app: &App| app.state().state.viewport.camera.heading().dot(normal).abs();
        app.state_mut().state.push(AppAction::EditSketch(sketch));
        settle(app);
        app.state_mut().state.push(AppAction::ExitSketch);
        settle(app);
        assert!(along(app) > 0.999, "the closed sketch leaves the view on its normal: {}", along(app));
        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(sketch))));
        settle(app);
        case.frame(app, "1-view-on-the-sketch-normal");

        ribbon::run_command(app, "extrude");
        app.run_steps(90);
        assert!(matches!(params(app), Some(EditorParams::Extrude(_))), "{:?}", params(app));
        assert_eq!(app.state().state.viewport.camera.view_name, "Isometric");
        assert!(along(app) < 0.9, "the view left the normal, so the depth shows: {}", along(app));
        case.frame(app, "2-extrude-preview-in-isometric");
        app.state_mut().state.push(AppAction::CancelEditor);
        settle(app);

        // A view the user chose is kept: the feature opens without turning it.
        app.state_mut().state.viewport.camera.set_view(soliddraft_app::viewport::camera::StandardView::Front);
        app.run_steps(90);
        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(sketch))));
        settle(app);
        ribbon::run_command(app, "extrude");
        app.run_steps(90);
        assert!(matches!(params(app), Some(EditorParams::Extrude(_))), "{:?}", params(app));
        assert_eq!(app.state().state.viewport.camera.view_name, "Front", "a turned view is left alone");
    });
}

/// A Cut sketched on a chamfer, a face no reference plane lies along, turns the view off
/// that face's own normal too. A fixed view could stay on the normal of such a face; the
/// unit test `a_feature_on_any_plane_gets_a_view_oblique_to_its_normal` covers a normal
/// on the isometric line of sight.
#[test]
fn a_cut_on_an_oblique_face_turns_the_view_off_its_normal() {
    use soliddraft_app::document::{ChamferParams, Command, SketchPlane};
    let mut doc = ribbon::plate();
    // The plate's top front edge, along X at y = -20 and z = 20.
    let edge = doc
        .edges
        .iter()
        .find(|e| (e.midpoint.z - 20.0).abs() < 1e-6 && (e.midpoint.y + 20.0).abs() < 1e-6)
        .expect("the top front edge")
        .id;
    let edge = doc.edge_ref(edge).unwrap();
    doc.apply(Command::AddChamfer { params: ChamferParams { edges: vec![edge], distance: 8.0, other_distance: None } })
        .unwrap();
    let body = doc.display_body().unwrap();
    let chamfer = doc
        .kernel()
        .topology(body)
        .unwrap()
        .faces
        .into_iter()
        .find(|f| f.normal.is_some_and(|n| n.y < -0.5 && n.z > 0.5))
        .expect("the chamfer face")
        .id;
    let face = doc.face_ref(chamfer).unwrap();
    let sketch = doc.apply(Command::AddSketch { plane: SketchPlane::Face { face } }).unwrap().unwrap();
    let mut drawing = soliddraft_app::sketch::Sketch::new();
    drawing.rectangle(soliddraft_app::sketch::P2::ZERO, 10.0, 4.0).unwrap();
    doc.apply(Command::EditSketch { id: sketch, sketch: drawing }).unwrap();

    let mut case = Case::new("feature-sketch-order/oblique-cut");
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        let normal = {
            let plane = app.state().state.doc.feature(sketch).unwrap().plane.unwrap();
            glam::Vec3::new(plane.normal.x as f32, plane.normal.y as f32, plane.normal.z as f32)
        };
        assert!(normal.x.abs() < 1e-3 && normal.y < -0.5 && normal.z > 0.5, "an oblique face: {normal}");
        let along = |app: &App| app.state().state.viewport.camera.heading().dot(normal).abs();
        app.state_mut().state.push(AppAction::EditSketch(sketch));
        settle(app);
        app.state_mut().state.push(AppAction::ExitSketch);
        settle(app);
        assert!(along(app) > 0.999, "the closed sketch leaves the view on its normal: {}", along(app));
        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(sketch))));
        settle(app);

        ribbon::run_command(app, "cut");
        app.run_steps(90);
        assert!(matches!(params(app), Some(EditorParams::Cut(_))), "{:?}", params(app));
        assert!(along(app) < 0.9, "the view left the chamfer's normal, so the cut's depth shows: {}", along(app));
        case.frame(app, "1-cut-preview-off-the-normal");
    });
}

/// Editing an early sketch shows the part as it was when the sketch was made, as a
/// feature's editor does: later features roll back until the sketch closes. A sketch on a
/// face has its sheet on that face, not round the world origin.
#[test]
fn an_early_sketch_opens_on_the_part_as_it_was_with_its_sheet_on_the_face() {
    use soliddraft_app::document::{Command, CutParams, ExtrudeParams, PlaneKind, SketchPlane};
    use soliddraft_app::sketch::{P2, Sketch};
    let mut doc = ribbon::blank();
    // A block away from the origin, then a circle on its top face, cut through.
    let base = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut drawing = Sketch::new();
    drawing.rectangle(P2::new(100.0, 50.0), 60.0, 40.0).unwrap();
    doc.apply(Command::EditSketch { id: base, sketch: drawing }).unwrap();
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(base), depth: 20.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let body = doc.display_body().unwrap();
    let block = doc.kernel().mass_properties(body).unwrap().volume;
    let top = doc
        .kernel()
        .topology(body)
        .unwrap()
        .faces
        .into_iter()
        .find(|f| f.normal.is_some_and(|n| n.z > 0.99) && f.centroid.z > 19.9)
        .unwrap()
        .id;
    let face = doc.face_ref(top).unwrap();
    let hole = doc.apply(Command::AddSketch { plane: SketchPlane::Face { face } }).unwrap().unwrap();
    let mut circle = Sketch::new();
    // A face's sketch plane keeps the world origin's projection as its own origin, so
    // the block's middle is at (100, 50) on it too.
    circle.circle_at(P2::new(100.0, 50.0), 10.0).unwrap();
    doc.apply(Command::EditSketch { id: hole, sketch: circle }).unwrap();
    doc.apply(Command::AddCut { params: CutParams::new(hole) }).unwrap();
    let cut = doc.kernel().mass_properties(doc.display_body().unwrap()).unwrap().volume;
    assert!(cut < block - 1.0, "the cut removes material");

    let mut case = Case::new("feature-sketch-order/early-sketch");
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        app.state_mut().state.push(AppAction::EditSketch(hole));
        settle(app);
        let state = &app.state().state;
        let shown = state.doc.kernel().mass_properties(state.doc.display_body().unwrap()).unwrap().volume;
        assert!((shown - block).abs() < 1e-3, "the part shows without the later cut: {shown} against {block}");
        // The sheet's middle lies over the block, not over the world origin.
        let session = state.sketch.as_ref().unwrap();
        let centre = soliddraft_app::sketcher::sheet_centre(&state.doc, &session.sketch, session.plane);
        let p = session.plane;
        let v = p.normal.cross(p.x_dir);
        let world = p.origin + p.x_dir * centre.u + v * centre.v;
        assert!((world.x - 100.0).abs() < 1.0 && (world.y - 50.0).abs() < 1.0, "the sheet is centred at {world:?}");
        case.frame(app, "1-early-sketch-on-the-part-as-it-was");

        app.state_mut().state.push(AppAction::ExitSketch);
        settle(app);
        let state = &app.state().state;
        assert_eq!(state.doc.rollback, None, "closing the sketch rolls the part forward again");
        let shown = state.doc.kernel().mass_properties(state.doc.display_body().unwrap()).unwrap().volume;
        assert!((shown - cut).abs() < 1e-3, "the cut is back: {shown} against {cut}");
        case.frame(app, "2-closed-the-part-is-whole");
    });
}

/// With a face selected its base sketch shows over the body. Where the sketch's outline
/// lies on the body's own bottom edges, the edge under the pointer still takes the pick.
#[test]
fn a_shown_base_sketch_leaves_the_body_edges_under_it_pickable() {
    use soliddraft_app::document::{Command, ExtrudeParams};
    let mut doc = ribbon::sketch();
    let sketch = doc.features[0].id;
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(sketch), depth: 20.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let mut case = Case::new("feature-sketch-order/sketch-over-edges");
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        let top = app
            .state()
            .state
            .doc
            .faces
            .iter()
            .find(|f| f.centroid.z > 19.9 && f.surface == soliddraft_kernel_api::SurfaceKind::Plane)
            .expect("the top face")
            .id;
        app.state_mut().state.push(AppAction::Select(Some(Selection::Face(top))));
        settle(app);
        assert!(
            soliddraft_app::tree_visibility::visible(&app.state().state).sketches.contains(&sketch),
            "the face's base sketch shows"
        );
        // The middle of the bottom front edge, where the sketch's line lies too.
        let state = &app.state().state;
        let at = state.viewport.camera.project(glam::Vec3::new(0.0, -20.0, 0.0), state.viewport.rect).unwrap();
        ribbon::render_steps::await_pick(app, at);
        app.run_steps(3);
        let state = &app.state().state;
        let picked = state.viewport.hover.and_then(|p| soliddraft_app::viewport::pick::selection(&state.doc, p));
        assert!(matches!(picked, Some(Selection::Edge(_))), "the body's edge is under the pointer: {picked:?}");
        assert_eq!(state.viewport.hover_sketch, None, "the sketch outline does not take the edge's pick");
        case.frame(app, "1-edge-under-the-sketch");
    });
}
