//! Native polygon placement, retained parameter editing, history and persistence.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{EntityKind, P2, Sketch, tools::PolygonMode},
    sketcher::Frame,
    state::{AppAction, SketchSession, Tool},
};

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}

fn pick(app: &mut App, point: P2) {
    let state = &app.state().state;
    let at = Frame { plane: session(app).plane, camera: &state.viewport.camera, rect: state.viewport.rect }
        .screen(point)
        .unwrap();
    ribbon::click_at(app, at);
}

fn preview(app: &mut App, point: P2) {
    let state = &app.state().state;
    let at = Frame { plane: session(app).plane, camera: &state.viewport.camera, rect: state.viewport.rect }
        .screen(point)
        .unwrap();
    app.hover_at(at);
    app.run_steps(5);
    fn polygon(shape: &egui::Shape, view: egui::Rect) -> bool {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().any(|s| polygon(s, view)),
            egui::Shape::Path(path) => path.points.len() == 6 && path.points.iter().all(|p| view.contains(*p)),
            _ => false,
        }
    }
    assert!(
        app.output().shapes.iter().any(|s| polygon(&s.shape, app.state().state.viewport.rect)),
        "a five-sided closed preview follows the pointer"
    );
}

fn control(app: &mut App, text: &str) {
    for _ in 0..12 {
        let left = app.state().state.viewport.rect.left();
        if let Some(row) = ribbon::painted(app)
            .into_iter()
            .find(|p| p.text == text && p.rect.right() < left && p.rect.contains_rect(p.full))
        {
            ribbon::click_at(app, row.rect.center());
            app.run_steps(4);
            return;
        }
        app.hover_at(egui::pos2(left - 20.0, 750.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -100.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        app.run_steps(6);
    }
    panic!("property control {text} must be reachable");
}

fn number(app: &mut App, index: usize, text: &str) {
    let controls: Vec<_> = app.get_all_by_role(egui::accesskit::Role::SpinButton).map(|n| n.rect()).collect();
    assert!(controls[index].bottom() < ribbon::band(app).top(), "numeric input must be above the command bar");
    ribbon::double_click_at(app, controls[index].center());
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text(text.into()));
    app.key_press(Key::Enter);
    app.run_steps(5);
}

fn history(app: &mut App, before: &Sketch, after: &Sketch) {
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(&session(app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(&session(app).sketch, after);
}

#[test]
fn native_modes_parameters_cancellation_undo_and_reopen() {
    let dir = std::env::var_os("SOLIDDRAFT_POLYGON_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-polygon-{}", std::process::id())));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut app = ribbon::app_at(doc, ribbon::EXPANDED_WINDOW, 1.0);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(20);
    for mode in [PolygonMode::Inscribed, PolygonMode::Circumscribed] {
        ribbon::run_command(&mut app, "sketch_polygon");
        control(&mut app, if mode == PolygonMode::Inscribed { "Inscribed" } else { "Circumscribed" });
        number(&mut app, 0, "5");
        let before = session(&app).sketch.clone();
        let center = if mode == PolygonMode::Inscribed { P2::ZERO } else { P2::new(-50.0, -35.0) };
        pick(&mut app, center);
        assert_eq!(session(&app).pending.len(), 1);
        app.key_press(Key::Escape);
        app.run_steps(5);
        assert_eq!(session(&app).tool, Tool::Select);
        assert_eq!(session(&app).sketch, before);
        ribbon::run_command(&mut app, "sketch_polygon");
        pick(&mut app, center);
        preview(&mut app, center + P2::new(24.0, 12.0));
        assert_eq!(session(&app).sketch, before);
        app.render().unwrap().save(dir.join(format!("{mode:?}-preview.png"))).unwrap();
        pick(&mut app, center + P2::new(24.0, 12.0));
        let placed = session(&app).sketch.clone();
        assert_eq!(placed.polygons.len(), before.polygons.len() + 1);
        let group = placed.polygons.last().unwrap().clone();
        assert_eq!(group.mode, mode);
        assert_eq!(group.edges.len(), 5);
        assert!(placed.entity(group.circle).unwrap().construction);
        assert_eq!(placed.regions().unwrap().len(), placed.polygons.len());
        history(&mut app, &before, &placed);
        app.key_press(Key::Escape);
        app.run_steps(4);
        let EntityKind::Line { a, b } = placed.entity(group.edges[0]).unwrap().kind else { panic!() };
        pick(&mut app, (a + b).scale(0.5));
        let edited_mode =
            if mode == PolygonMode::Inscribed { PolygonMode::Circumscribed } else { PolygonMode::Inscribed };
        // Reveal the mode row first, bringing both preceding numeric inputs into view.
        control(&mut app, if edited_mode == PolygonMode::Inscribed { "Inscribed" } else { "Circumscribed" });
        number(&mut app, 0, "7");
        number(&mut app, 1, "18");
        assert_eq!(session(&app).sketch, placed, "draft inputs do not edit the document");
        control(&mut app, "Apply polygon");
        assert_eq!(session(&app).selection.len(), 1, "Apply keeps the inspected side selected");
        let edited = session(&app).sketch.clone();
        let group = edited.polygon_of(group.circle).unwrap();
        assert_eq!(group.edges.len(), 7);
        assert_eq!(group.mode, edited_mode);
        assert!((edited.entity(group.circle).unwrap().radius().unwrap() - 18.0).abs() < 1e-6);
        history(&mut app, &placed, &edited);
        number(&mut app, 1, "-1");
        control(&mut app, "Apply polygon");
        assert_eq!(session(&app).sketch, edited, "an invalid radius refuses atomically");
        number(&mut app, 1, "18");
        app.render().unwrap().save(dir.join(format!("{mode:?}-edited.png"))).unwrap();
    }
    let expected = session(&app).sketch.clone();
    let path = dir.join("polygon.sldraft");
    for action in [AppAction::ExitSketch(None), AppAction::SaveAs(Some(path.clone())), AppAction::Open(Some(path))] {
        app.state_mut().state.push(action);
        app.run_steps(12);
    }
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(20);
    assert_eq!(session(&app).sketch, expected);
    assert_eq!(session(&app).sketch.polygons.len(), 2);
    soliddraft_app::file::set_config_dir(None);
}
