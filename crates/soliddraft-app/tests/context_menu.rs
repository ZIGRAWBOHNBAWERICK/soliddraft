//! Right-click picking and commands through the real Edit Suite frame.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::{NodeT, Queryable};
use glam::Vec3;
use ribbon::{App, render_steps};
use soliddraft_app::{
    document::{Command, PlaneKind, Selection, SketchPlane},
    sketch::P2,
    sketcher::Frame,
    state::{AppAction, Tool},
    viewport::scenes::Scene,
};

fn set_frame(case: &ribbon::Case, frame: &str) {
    std::fs::write(case.path("config/settings.json"), format!(r#"{{"frame_layout":"{frame}"}}"#)).unwrap();
}

fn right_click(app: &mut App, pos: Pos2, model: bool) {
    if model {
        render_steps::await_pick(app, pos);
    } else {
        app.hover_at(pos);
        app.run_steps(3);
    }
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        app.step();
    }
    app.run_steps(3);
}

fn tree_row(app: &mut App, name: &str) -> Pos2 {
    let visible =
        app.query_all_by_label(name).any(|node| node.accesskit_node().role() == egui::accesskit::Role::Button);
    if !visible {
        ribbon::run_command(app, "toggle_tree");
        app.run_steps(12);
    }
    ribbon::control(app, name, ribbon::window(app)).rect.center()
}

fn menu_command(app: &mut App, id: &str) {
    let command = soliddraft_app::commands::get(id).unwrap();
    let state = &app.state().state;
    let label = command.label_in(state);
    // The popup follows same-named property buttons in the accessibility tree.
    let point =
        app.query_all_by_label(&label).last().unwrap_or_else(|| panic!("missing menu command {label}")).rect().center();
    ribbon::click_at(app, point);
}

fn groups(app: &App, expected: &[&str]) {
    let painted = ribbon::painted(app);
    for label in expected {
        assert!(painted.iter().any(|text| text.text == label.to_uppercase()), "missing group {label}");
    }
}

fn dismiss(app: &mut App) {
    app.key_press(Key::Escape);
    app.run_steps(4);
    assert!(app.query_all_by_label("Plain white").next().is_none());
}

fn world(app: &App, point: Vec3) -> Pos2 {
    let st = &app.state().state;
    st.viewport.camera.project(point, st.viewport.rect).unwrap()
}

#[test]
fn secondary_pick_targets_the_face_edge_feature_body_and_empty_space() {
    let case = ribbon::case("context-model");
    set_frame(&case, "edit_suite");
    let mut app = ribbon::app(ribbon::plate());
    case.run(&mut app, |app| {
        assert!(!app.state().state.tree_shown());
        let tree = app.get_by_label("Tree").rect().center();
        ribbon::click_at(app, tree);
        app.run_steps(6);
        assert!(app.state().state.tree_shown());
        let feature = app.state().state.doc.features[0].id;
        let name = app.state().state.doc.features[0].name.clone();
        let face = world(app, Vec3::new(0.0, 0.0, 20.0));
        right_click(app, face, true);
        assert!(matches!(app.state().state.doc.selection, Some(Selection::Face(_))));
        groups(app, &["Face", "Feature", "Body"]);
        case.frame(app, "face");
        menu_command(app, "suppress");
        app.run_steps(6);
        assert!(app.state().state.doc.feature(feature).unwrap().suppressed);

        let row = tree_row(app, &name);
        right_click(app, row, false);
        assert_eq!(app.state().state.doc.selection, Some(Selection::Feature(feature)));
        groups(app, &["Feature", "Body"]);
        menu_command(app, "suppress");
        app.run_steps(6);
        assert!(!app.state().state.doc.feature(feature).unwrap().suppressed);

        let edge = world(app, Vec3::new(30.0, 0.0, 20.0));
        right_click(app, edge, true);
        assert!(matches!(app.state().state.doc.selection, Some(Selection::Edge(_))));
        groups(app, &["Edge", "Feature", "Body"]);
        case.frame(app, "edge");
        dismiss(app);

        let row = tree_row(app, "Solid body");
        right_click(app, row, false);
        assert_eq!(app.state().state.doc.selection, Some(Selection::Body));
        groups(app, &["Body"]);
        dismiss(app);

        let empty = app.state().state.viewport.rect.left_bottom() + egui::vec2(45.0, -50.0);
        right_click(app, empty, true);
        assert_eq!(app.state().state.doc.selection, None, "empty space clears the old target");
        groups(app, &["View", "Scene", "Display", "Show"]);
        case.frame(app, "empty");
        menu_command(app, "scene_graphite");
        app.run_steps(5);
        assert_eq!(app.state().state.viewport.scene, Some(Scene::Graphite));
        assert!(app.query_all_by_label("Plain white").next().is_none(), "a command closes the menu");
        right_click(app, empty, true);
        dismiss(app);
    });
}

fn sketch_point(app: &App, point: P2) -> Pos2 {
    let st = &app.state().state;
    let session = st.sketch.as_ref().unwrap();
    Frame { plane: session.plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(point).unwrap()
}

#[test]
fn sketch_right_click_ends_the_chain_before_offering_entity_or_empty_menus() {
    let case = ribbon::case("context-sketch");
    set_frame(&case, "edit_suite");
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(20);
    ribbon::run_command(&mut app, "sketch_line");
    case.run(&mut app, |app| {
        for point in [P2::new(-20.0, -10.0), P2::new(20.0, -10.0)] {
            let at = sketch_point(app, point);
            ribbon::click_at(app, at);
        }
        let revision = app.state().state.doc.revision();
        let empty = sketch_point(app, P2::new(-25.0, 25.0));
        right_click(app, empty, false);
        let s = app.state().state.sketch.as_ref().unwrap();
        assert_eq!(s.tool, Tool::Line);
        assert!(s.pending.is_empty() && s.chain_from.is_none());
        assert_eq!(s.sketch.drawn().count(), 1);
        assert_eq!(app.state().state.doc.revision(), revision);
        assert!(app.query_all_by_label("Sketch tools").next().is_none());
        right_click(app, empty, false);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Select);
        assert!(app.query_all_by_label("Sketch tools").next().is_none());

        let line = sketch_point(app, P2::new(0.0, -10.0));
        right_click(app, line, false);
        groups(app, &["Line", "Sketch"]);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().selection.len(), 1);
        case.frame(app, "line");
        menu_command(app, "delete");
        app.run_steps(5);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().sketch.drawn().count(), 0);

        right_click(app, empty, false);
        groups(app, &["Sketch tools", "Sketch", "Scene"]);
        case.frame(app, "empty-sketch");
        dismiss(app);
        assert!(app.state().state.sketch.is_some(), "Escape closes the menu without closing the sketch");
    });
}

#[test]
fn classic_keeps_its_viewport_and_feature_menu_behavior() {
    let case = ribbon::case("context-classic");
    set_frame(&case, "classic");
    let mut app = ribbon::app(ribbon::plate());
    case.run(&mut app, |app| {
        let original = app.state().state.doc.selection;
        let face = world(app, Vec3::new(0.0, 0.0, 20.0));
        right_click(app, face, true);
        assert_eq!(app.state().state.doc.selection, original);
        assert!(app.query_all_by_label("Face").next().is_none());
        let name = app.state().state.doc.features[0].name.clone();
        let row = tree_row(app, &name);
        right_click(app, row, false);
        assert!(app.query_all_by_label("Roll back to here").next().is_some());
        assert!(app.query_all_by_label("Repair references…").next().is_none());
    });
}

#[test]
fn secondary_pick_waits_for_the_camera_and_viewport_that_were_clicked() {
    for change in ["camera", "resize"] {
        let case = ribbon::case(&format!("context-stale-{change}"));
        set_frame(&case, "edit_suite");
        let mut doc = ribbon::plate();
        doc.selection = None;
        let mut app = ribbon::app(doc);
        case.run(&mut app, |app| {
            let at = world(app, Vec3::new(0.0, 0.0, 20.0));
            render_steps::await_pick(app, at);
            let state = &app.state().state;
            let old_sample = state.viewport.pick_sample();
            let old_target = soliddraft_app::viewport::pick::selection(&state.doc, state.viewport.hover.unwrap());
            assert!(matches!(old_target, Some(Selection::Face(_))));

            if change == "camera" {
                ribbon::run_command(app, "view_front");
            } else {
                app.set_size(ribbon::WINDOW + egui::vec2(180.0, 0.0));
                app.run_steps(3);
            }
            let state = &app.state().state;
            assert_eq!(state.viewport.pick_sample().revision, old_sample.revision, "the model did not change");
            assert_ne!(state.viewport.pick_view(state.viewport.rect, app.ctx.pixels_per_point()), old_sample.seen);
            right_click(app, at, false);
            assert_eq!(app.state().state.doc.selection, None, "a stale {change} sample must not select the old entity");
            assert!(!egui::Popup::is_any_open(&app.ctx), "an unknown target must not open an empty-space menu");

            render_steps::await_pick(app, at);
            assert!(egui::Popup::is_any_open(&app.ctx), "the pending click opens after its current sample arrives");
            if change == "camera" {
                assert_ne!(app.state().state.doc.selection, old_target, "the old face is no longer under the pointer");
            }
            case.frame(app, "resolved");
            dismiss(app);
        });
    }
}
