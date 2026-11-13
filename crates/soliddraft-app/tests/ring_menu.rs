//! The approved ring through the real Edit Suite frame and renderer.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::kittest::{By, Queryable};
use glam::Vec3;
use ribbon::{App, render_steps};
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    settings::FrameLayout,
    sketch::{ConstraintKind, P2},
    sketcher::Frame,
    state::{AppAction, SketchSel, Tool},
};

fn edit_suite(case: &ribbon::Case) {
    std::fs::write(case.path("config/settings.json"), r#"{"frame_layout":"edit_suite"}"#).unwrap();
}

fn secondary(app: &mut App, at: Pos2, model: bool) {
    if model {
        render_steps::await_pick(app, at);
    }
    app.hover_at(at);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        app.step();
    }
    app.run_steps(4);
}

fn quick_rect(app: &App) -> Rect {
    app.get_by_label("Quick commands").rect()
}

fn quick_count(app: &App) -> usize {
    app.get_by_label("Quick commands").query_all(By::new().role(egui::accesskit::Role::Button)).count()
}

fn click_quick(app: &mut App, label: &str) {
    let at = app.get_by_label("Quick commands").get_by_label(label).rect().center();
    ribbon::click_at(app, at);
}

fn world(app: &App, point: Vec3) -> Pos2 {
    let state = &app.state().state;
    state.viewport.camera.project(point, state.viewport.rect).unwrap()
}

fn sketch_point(app: &App, point: P2) -> Pos2 {
    let state = &app.state().state;
    let sketch = state.sketch.as_ref().unwrap();
    Frame { plane: sketch.plane, camera: &state.viewport.camera, rect: state.viewport.rect }.screen(point).unwrap()
}

#[test]
fn face_ring_has_eight_commands_and_body_visibility_uses_the_existing_view_choice() {
    let case = ribbon::case("ring-face");
    edit_suite(&case);
    let mut app = ribbon::app(ribbon::plate());
    case.run(&mut app, |app| {
        let revision = app.state().state.doc.revision();
        let at = world(app, Vec3::new(0.0, 0.0, 20.0));
        secondary(app, at, true);
        assert_eq!(quick_count(app), 8);
        assert!(quick_rect(app).center().distance(at) <= ribbon::TOLERANCE);
        for group in ["Face", "Feature", "Body"] {
            assert!(app.query_all_by_label(group).next().is_some());
        }
        case.frame(app, "open");
        click_quick(app, "Hide");
        assert!(!soliddraft_app::tree_visibility::visible(&app.state().state).body);
        assert_eq!(app.state().state.doc.revision(), revision, "hiding does not change geometry");
        assert!(!egui::Popup::is_any_open(&app.ctx));

        let at = app.get_by_role_and_label(egui::accesskit::Role::Button, "Solid body").rect().center();
        secondary(app, at, false);
        assert_eq!(quick_count(app), 8);
        click_quick(app, "Show");
        assert!(soliddraft_app::tree_visibility::visible(&app.state().state).body);
        assert_eq!(app.state().state.doc.revision(), revision);
    });
}

#[test]
fn empty_ring_clamps_at_viewport_corners_in_regular_and_compact_windows() {
    let case = ribbon::case("ring-corners");
    edit_suite(&case);
    for size in [ribbon::WINDOW, vec2(1280.0, 720.0)] {
        let mut app = ribbon::app_at(ribbon::plate(), size, 1.0);
        if app.state().state.layout.tree.shown {
            ribbon::run_command(&mut app, "toggle_tree");
        }
        case.run(&mut app, |app| {
            let view = app.state().state.viewport.rect.shrink(8.0);
            for (name, at) in [
                ("top-left", view.left_top()),
                ("top-right", view.right_top()),
                ("bottom-left", view.left_bottom()),
                ("bottom-right", view.right_bottom()),
            ] {
                secondary(app, at, true);
                assert_eq!(quick_count(app), 8);
                let window = Rect::from_min_size(Pos2::ZERO, size);
                assert!(window.contains_rect(quick_rect(app)), "{name}");
                for node in app.get_by_label("Quick commands").query_all(By::new().role(egui::accesskit::Role::Button))
                {
                    assert!(window.contains_rect(node.rect()), "{name}: quick command stays inside");
                }
                let mut column_top = None;
                for group in ["View", "Scene", "Display", "Show"] {
                    // Popup headings follow the main-window labels in the accessibility tree.
                    let heading = app
                        .query_all(By::new().role(egui::accesskit::Role::Label).label(group))
                        .last()
                        .expect("the popup has this command group")
                        .rect();
                    assert!(window.contains_rect(heading));
                    if let Some(top) = column_top {
                        assert!((heading.top() - top).abs() <= ribbon::TOLERANCE, "menu columns align");
                    }
                    column_top = Some(heading.top());
                }
                case.frame(app, &format!("{}-{name}", size.x));
                app.key_press(Key::Escape);
                app.run_steps(4);
                assert!(!egui::Popup::is_any_open(&app.ctx));
                assert!(app.query_all_by_label("Quick commands").next().is_none());
            }
        });
    }
}

#[test]
fn sketch_ring_applies_a_relation_as_one_edit_and_keeps_solid_commands_hidden() {
    let case = ribbon::case("ring-sketch");
    edit_suite(&case);
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
        for point in [P2::new(-20.0, -10.0), P2::new(20.0, 15.0)] {
            let at = sketch_point(app, point);
            ribbon::click_at(app, at);
        }
        let empty = sketch_point(app, P2::new(-30.0, 30.0));
        secondary(app, empty, false);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Line);
        assert!(app.query_all_by_label("Quick commands").next().is_none());
        secondary(app, empty, false);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Select);
        let line = app.state().state.sketch.as_ref().unwrap().sketch.entities.iter().find(|e| e.is_line()).unwrap();
        let id = line.id;
        let a = line.point(0).unwrap();
        let b = line.point(1).unwrap();
        let at = sketch_point(app, P2::new(0.75 * a.u + 0.25 * b.u, 0.75 * a.v + 0.25 * b.v));
        secondary(app, at, false);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().selection, [SketchSel::Entity(id)]);
        assert!(soliddraft_app::commands::get("sketch_vertical").unwrap().is_enabled(&app.state().state));
        assert_eq!(quick_count(app), 8);
        for hidden in ["Extrude", "Cut", "Edit feature", "Delete feature", "Face", "Feature", "Body"] {
            assert!(app.query_all_by_label(hidden).next().is_none(), "{hidden} stays hidden in sketch mode");
        }
        let before = app.state().state.sketch.as_ref().unwrap().sketch.clone();
        let revision = app.state().state.doc.revision();
        case.frame(app, "line");
        click_quick(app, "Make vertical");
        let sketch = &app.state().state.sketch.as_ref().unwrap().sketch;
        assert!(sketch.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Vertical { .. })));
        assert_eq!(app.state().state.doc.revision(), revision + 1);
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        app.run_steps(5);
        let sketch = &app.state().state.sketch.as_ref().unwrap().sketch;
        assert_eq!(sketch.entities, before.entities);
        assert_eq!(sketch.constraints, before.constraints);

        secondary(app, empty, false);
        assert_eq!(quick_count(app), 8);
        case.frame(app, "empty");
        app.key_press(Key::Escape);
        app.run_steps(4);
        assert!(app.state().state.sketch.is_some());
        assert!(app.query_all_by_label("Quick commands").next().is_none());
    });
}

#[test]
fn classic_has_no_ring() {
    let case = ribbon::case("ring-classic");
    let mut app = ribbon::app(ribbon::plate());
    assert_eq!(app.state().state.frame_layout, FrameLayout::Classic);
    let at = world(&app, Vec3::new(0.0, 0.0, 20.0));
    secondary(&mut app, at, true);
    assert!(app.query_all_by_label("Quick commands").next().is_none());
    drop(app);
    drop(case);
}
