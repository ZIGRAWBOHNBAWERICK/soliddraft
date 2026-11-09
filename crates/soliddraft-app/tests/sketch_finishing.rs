//! Native corner treatments and splits preview, cancel, commit and reopen.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2};
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, Sketch, tools::ChamferSize},
    sketcher::{self, Frame},
    state::{AppAction, SketchSession, Tool},
};

fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}
fn screen(app: &App, p: P2) -> Pos2 {
    let state = &app.state().state;
    Frame { plane: session(app).plane, camera: &state.viewport.camera, rect: state.viewport.rect }.screen(p).unwrap()
}
fn hover(app: &mut App, at: P2) {
    app.hover_at(screen(app, at));
    app.run_steps(4);
}
fn pick(app: &mut App, at: P2) {
    let pos = screen(app, at);
    ribbon::click_at(app, pos);
}
fn tool(app: &mut App, label: &str) {
    let window = ribbon::window(app);
    let top = egui::Rect::from_min_max(window.min, egui::pos2(window.right(), app.state().state.viewport.rect.top()));
    if ribbon::buttons_in(app, top).iter().any(|(name, _)| name == label) {
        ribbon::click_button(app, label, top);
    } else {
        ribbon::click_button(app, "Modify group", top);
        let popup = egui::Rect::from_min_max(egui::pos2(app.state().state.viewport.rect.left(), 0.0), window.max);
        ribbon::click_button(app, label, popup);
    }
    app.run_steps(4);
}
fn panel_click(app: &mut App, label: &str) {
    let left = app.state().state.viewport.rect.left();
    for _ in 0..10 {
        if let Some(row) = ribbon::painted(app)
            .into_iter()
            .find(|p| p.text == label && p.rect.right() < left && p.rect.contains_rect(p.full))
        {
            ribbon::click_at(app, row.rect.center());
            return;
        }
        app.hover_at(egui::pos2(left - 20.0, 750.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -180.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        app.run_steps(10);
    }
    panic!("property control {label} must be reachable");
}
fn proof(app: &mut App, name: &str) {
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_FINISHING_PROOF_DIR") {
        let path = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&path).unwrap();
        app.render().unwrap().save(path.join(format!("{name}.png"))).unwrap();
    }
}
fn undo(app: &mut App) {
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
}

#[test]
fn native_finishing_keeps_analytic_previews_and_one_undo_step() {
    let dir = std::env::temp_dir().join(format!("soliddraft-finishing-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut sketch = Sketch::new();
    sketcher::add_rectangle(&mut sketch, p(-15.0, -10.0), p(15.0, 10.0));
    sketch.add(EntityKind::Circle { center: p(8.0, 3.0), radius: 3.0 }, true);
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    let before = session(&app).sketch.clone();
    tool(&mut app, "Sketch fillet");
    pick(&mut app, p(-10.0, -10.0));
    hover(&mut app, p(-15.0, -5.0));
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 3);
    assert_eq!(session(&app).sketch, before);
    proof(&mut app, "fillet-preview");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).tool, Tool::Select);
    assert!(session(&app).corner_first.is_none());
    assert_eq!(session(&app).sketch, before);
    tool(&mut app, "Sketch fillet");
    pick(&mut app, p(-10.0, -10.0));
    pick(&mut app, p(-15.0, -5.0));
    assert_eq!(session(&app).sketch.drawn().count(), 6);
    assert!(
        session(&app)
            .sketch
            .constraints
            .iter()
            .any(|c| matches!(c.kind, ConstraintKind::Radius { value, .. } if (value - 2.0).abs() < 1e-6))
    );
    let fillet = session(&app).sketch.clone();
    proof(&mut app, "fillet-applied");
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, fillet);
    undo(&mut app);
    tool(&mut app, "Sketch chamfer");
    panel_click(&mut app, "Distance and angle");
    assert!(matches!(session(&app).modify.chamfer, ChamferSize::DistanceAngle { .. }));
    pick(&mut app, p(-10.0, -10.0));
    hover(&mut app, p(-15.0, -5.0));
    assert_eq!(session(&app).sketch, before);
    proof(&mut app, "chamfer-preview");
    pick(&mut app, p(-15.0, -5.0));
    assert_eq!(session(&app).sketch.drawn().count(), 6);
    assert!(session(&app).sketch.drawn().last().unwrap().is_line());
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    tool(&mut app, "Split entities");
    hover(&mut app, p(0.0, -10.0));
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 2);
    assert_eq!(session(&app).sketch, before);
    proof(&mut app, "split-line-preview");
    pick(&mut app, p(0.0, -10.0));
    assert_eq!(session(&app).sketch.drawn().count(), 6);
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    tool(&mut app, "Split entities");
    pick(&mut app, p(11.0, 3.0));
    assert!(session(&app).corner_first.is_some());
    hover(&mut app, p(8.0, 6.0));
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 2);
    proof(&mut app, "split-circle-preview");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    tool(&mut app, "Split entities");
    pick(&mut app, p(11.0, 3.0));
    pick(&mut app, p(8.0, 6.0));
    let after = session(&app).sketch.clone();
    assert_eq!(after.drawn().filter(|e| matches!(e.kind, EntityKind::Arc { .. }) && e.construction).count(), 2);
    let path = dir.join("finishing.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch(None));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    assert_eq!(session(&app).sketch.constraints, after.constraints);
    assert_eq!(session(&app).sketch.drawn().count(), after.drawn().count());
    proof(&mut app, "split-reopened");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}
