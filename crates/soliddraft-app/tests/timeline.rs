//! Timeline acceptance through the app, pointer events and software Vulkan.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2, Rect, pos2, vec2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::document::{Command, Document, ExtrudeParams, PlaneKind, Selection, SketchPlane};
use soliddraft_app::settings::FrameLayout;
use soliddraft_app::theme::{self, Mode};

fn bounds(app: &App) -> Rect {
    let view = app.state().state.viewport.rect;
    Rect::from_min_max(pos2(view.left(), view.bottom()), ribbon::window(app).max - vec2(0.0, 32.0))
}

fn chip(app: &App, name: &str) -> Rect {
    ribbon::control(app, &format!("{name} in history"), bounds(app)).rect
}

fn open_menu(app: &mut App, name: &str) {
    let pos = chip(app, name).center();
    app.hover_at(pos);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        app.step();
    }
    app.run_steps(4);
}

fn menu(app: &mut App, name: &str, command: &str) {
    open_menu(app, name);
    let control = ribbon::control(app, command, ribbon::window(app));
    assert!(control.enabled, "{command} is enabled");
    ribbon::click_at(app, control.rect.center());
    app.run_steps(8);
}

fn drag(app: &mut App, from: Pos2, to: Pos2) {
    app.hover_at(from);
    app.run_steps(2);
    app.drag_at(from);
    app.run_steps(2);
    for step in 1..=10 {
        app.hover_at(from + (to - from) * (step as f32 / 10.0));
        app.run_steps(2);
    }
    app.drop_at(to);
    app.run_steps(8);
}

fn sketches() -> Document {
    let mut doc = ribbon::sketch();
    for plane in [PlaneKind::Front, PlaneKind::Right] {
        doc.apply(Command::AddSketch { plane: SketchPlane::Reference { plane, offset: 0.0 } }).unwrap();
    }
    doc
}

fn edit_suite(doc: Document) -> App {
    let mut app = ribbon::app(doc);
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.run_steps(8);
    app
}

#[test]
fn chips_include_absorbed_sketches_and_only_appear_in_edit_suite() {
    let case = ribbon::case("timeline-layout");
    let mut doc = ribbon::sketch();
    let sketch = doc.features[0].id;
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(sketch), ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let mut app = ribbon::app(doc);
    assert!(app.query_all_by_label("Sketch1 in history").next().is_none());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.run_steps(8);
    case.run(&mut app, |app| {
        for name in ["Sketch1", "Boss-Extrude1"] {
            let rect = chip(app, name);
            assert!(bounds(app).contains_rect(rect));
            let painted = ribbon::painted_in(app, rect);
            assert!(painted.iter().any(|text| text.text == name));
            assert!(painted.len() >= 2, "the chip has a summary");
            ribbon::click_at(app, rect.center());
            let expected = app.state().state.doc.features.iter().find(|f| f.name == name).unwrap().id;
            assert_eq!(app.state().state.doc.selection, Some(Selection::Feature(expected)));
        }
        case.frame(app, "absorbed-sketch");
    });
}

#[test]
fn context_actions_rename_suppress_rollback_edit_and_delete() {
    let case = ribbon::case("timeline-actions");
    let mut app = edit_suite(sketches());
    case.run(&mut app, |app| {
        open_menu(app, "Sketch2");
        for label in ["Edit", "Repair", "Rename", "Suppress", "Roll back to here", "Delete"] {
            let control = ribbon::control(app, label, ribbon::window(app));
            assert_eq!(control.enabled, label != "Repair");
        }
        case.frame(app, "context-menu");
        app.key_press(Key::Escape);
        app.run_steps(4);
        menu(app, "Sketch2", "Rename");
        app.key_press_modifiers(Modifiers::COMMAND, Key::A);
        app.event(egui::Event::Text("Construction".into()));
        app.key_press(Key::Enter);
        app.run_steps(8);
        assert_eq!(app.state().state.doc.features[1].name, "Construction");
        menu(app, "Construction", "Suppress");
        assert!(app.state().state.doc.features[1].suppressed);
        menu(app, "Construction", "Unsuppress");
        assert!(!app.state().state.doc.features[1].suppressed);
        menu(app, "Sketch1", "Roll back to here");
        assert_eq!(app.state().state.doc.rollback, Some(1));
        let bar = app.get_by_label("History rollback bar").rect();
        ribbon::double_click_at(app, bar.center());
        app.run_steps(8);
        assert_eq!(app.state().state.doc.rollback, None);
        menu(app, "Sketch1", "Edit");
        assert_eq!(app.state().state.sketch.as_ref().unwrap().feature, app.state().state.doc.features[0].id);
        app.state_mut().state.push(soliddraft_app::state::AppAction::ExitSketch);
        app.run_steps(8);
        menu(app, "Sketch3", "Delete");
        assert_eq!(app.state().state.doc.features.len(), 2);
        case.frame(app, "after-actions");
    });
}

#[test]
fn chip_and_playhead_drags_change_document_order_and_rollback() {
    let case = ribbon::case("timeline-drag");
    let mut app = edit_suite(sketches());
    case.run(&mut app, |app| {
        let from = chip(app, "Sketch3").center();
        let to = chip(app, "Sketch1").left_center() + vec2(2.0, 0.0);
        drag(app, from, to);
        assert_eq!(
            app.state().state.doc.features.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(),
            ["Sketch3", "Sketch1", "Sketch2"]
        );
        let from = chip(app, "Sketch3").center();
        let to = chip(app, "Sketch2").right_center() - vec2(2.0, 0.0);
        drag(app, from, to);
        assert_eq!(app.state().state.doc.features[2].name, "Sketch3");
        let from = app.get_by_label("History rollback bar").rect().center();
        let to = chip(app, "Sketch2").left_center() + vec2(2.0, 0.0);
        drag(app, from, to);
        assert_eq!(app.state().state.doc.rollback, Some(1));
        case.frame(app, "rolled-back");
        let from = app.get_by_label("History rollback bar").rect().center();
        let to = chip(app, "Sketch1").left_center() + vec2(2.0, 0.0);
        drag(app, from, to);
        assert_eq!(app.state().state.doc.rollback, Some(0));
        let from = app.get_by_label("History rollback bar").rect().center();
        let to = chip(app, "Sketch3").right_center() - vec2(2.0, 0.0);
        drag(app, from, to);
        assert_eq!(app.state().state.doc.rollback, None);
    });
}

#[test]
fn sketch_double_click_dims_later_chips_in_both_themes() {
    let case = ribbon::case("timeline-sketch");
    let mut app = edit_suite(sketches());
    for mode in [Mode::Light, Mode::Dark] {
        theme::set_mode(&app.ctx, mode);
        app.run_steps(4);
        let rect = chip(&app, "Sketch1");
        ribbon::double_click_at(&mut app, rect.center());
        app.run_steps(8);
        assert!(app.state().state.in_sketch());
        let later = chip(&app, "Sketch2");
        let text = ribbon::painted_in(&app, later).into_iter().find(|p| p.text == "Sketch2").unwrap();
        assert_eq!(text.color, theme::muted());
        open_menu(&mut app, "Sketch2");
        assert!(!ribbon::control(&app, "Delete", ribbon::window(&app)).enabled);
        app.key_press(Key::Escape);
        app.run_steps(4);
        case.frame(&mut app, &format!("{mode:?}-editing"));
        app.state_mut().state.push(soliddraft_app::state::AppAction::ExitSketch);
        app.run_steps(8);
        let later = chip(&app, "Sketch2");
        let text = ribbon::painted_in(&app, later).into_iter().find(|p| p.text == "Sketch2").unwrap();
        assert_eq!(text.color, theme::ink());
    }
}

#[test]
fn long_histories_scroll_at_1280_and_empty_history_has_a_hint() {
    let case = ribbon::case("timeline-scroll");
    let mut app = edit_suite(ribbon::blank());
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "No features yet"));
    for _ in 0..12 {
        app.state_mut()
            .state
            .doc
            .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
            .unwrap();
    }
    app.set_size(vec2(1280.0, 800.0));
    app.run_steps(8);
    let area = bounds(&app);
    app.hover_at(area.center());
    app.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(-2000.0, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(20);
    assert!(area.contains_rect(chip(&app, "Sketch12")), "the last feature scrolls into view");
    assert!(app.state().state.viewport.rect.width() >= 300.0);
    case.frame(&mut app, "1280-scrolled");
}
