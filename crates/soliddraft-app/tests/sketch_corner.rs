//! Corner trim through the existing Trim tool panel and real pointer picks.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2, vec2};
use ribbon::App;
use soliddraft_app::{
    document::{Command, FeatureId, PlaneKind, SketchPlane},
    sketch::{EntityKind, P2, Sketch, solve},
    sketcher::Frame,
    state::{AppAction, SketchSession},
};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn launch(mut sketch: Sketch) -> (App, FeatureId) {
    std::fs::create_dir_all(dir().join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir().join("config")));
    solve::solve(&mut sketch, None, true);
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    (app, id)
}

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap()
}

fn pick(app: &mut App, p: P2) {
    let at = screen(app, p);
    assert!(app.state().state.viewport.rect.contains(at));
    ribbon::click_at(app, at);
}

fn dir() -> std::path::PathBuf {
    std::env::var_os("SOLIDDRAFT_CORNER_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-corner-{}", std::process::id())))
}

fn frame(app: &mut App, name: &str) {
    if std::env::var_os("SOLIDDRAFT_CORNER_PROOF_DIR").is_some() {
        std::fs::create_dir_all(dir()).unwrap();
        app.render().unwrap().save(dir().join(format!("{name}.png"))).unwrap();
    }
}

fn corner_mode(app: &mut App) {
    ribbon::run_command(app, "sketch_trim");
    ribbon::click_option(app, "Corner");
    assert_eq!(session(app).tool.label(), "Trim corner");
}

#[test]
fn corner_preview_cancel_undo_and_reopen_use_one_geometry_edit() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let a = sketch.add(EntityKind::Line { a: P2::new(-20.0, 0.0), b: P2::new(0.0, 0.0) }, false);
    let b = sketch.add(EntityKind::Line { a: P2::new(10.0, -10.0), b: P2::new(10.0, 20.0) }, false);
    sketch.add(EntityKind::Line { a: P2::new(10.0, 20.0), b: P2::new(-20.0, 20.0) }, false);
    sketch.add(EntityKind::Line { a: P2::new(-20.0, 20.0), b: P2::new(-20.0, 0.0) }, false);
    let (mut app, id) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    corner_mode(&mut app);
    pick(&mut app, P2::new(-10.0, 0.0));
    let at = screen(&app, P2::new(10.0, 10.0));
    app.hover_at(at);
    app.run_steps(6);
    assert!(session(&app).preview.as_ref().is_some_and(|p| p.operation.is_some()));
    assert_eq!(session(&app).sketch, before, "the first pick and preview are not edits");
    assert_eq!(app.state().state.doc.revision(), revision);
    frame(&mut app, "corner-preview");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).tool.label(), "Select");
    assert!(session(&app).corner_first.is_none());
    assert_eq!(session(&app).sketch, before);
    corner_mode(&mut app);
    pick(&mut app, P2::new(-10.0, 0.0));
    pick(&mut app, P2::new(10.0, 10.0));
    assert_eq!(
        session(&app).sketch.entity(a).unwrap().kind,
        EntityKind::Line { a: P2::new(-20.0, 0.0), b: P2::new(10.0, 0.0) }
    );
    assert_eq!(
        session(&app).sketch.entity(b).unwrap().kind,
        EntityKind::Line { a: P2::new(10.0, 0.0), b: P2::new(10.0, 20.0) }
    );
    assert_eq!(session(&app).sketch.regions().unwrap().len(), 1);
    assert!((session(&app).sketch.regions().unwrap()[0].area() - 600.0).abs() < 1e-6);
    frame(&mut app, "corner-after");
    let after = session(&app).sketch.clone();
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before, "both changes undo together");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, after);
    let path = dir().join("corner.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, after);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn corner_refusals_right_click_and_document_undo_clear_pending_picks() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Line { a: P2::new(-20.0, 0.0), b: P2::ZERO }, false);
    sketch.add(EntityKind::Line { a: P2::new(-20.0, 10.0), b: P2::new(0.0, 10.0) }, false);
    sketch.add(EntityKind::Circle { center: P2::new(20.0, 20.0), radius: 5.0 }, false);
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    corner_mode(&mut app);
    pick(&mut app, P2::new(-10.0, 0.0));
    for at in [P2::new(-10.0, 10.0), P2::new(-10.0, 0.0), P2::new(20.0, 25.0)] {
        pick(&mut app, at);
        assert_eq!(session(&app).sketch, before);
        assert_eq!(app.state().state.doc.revision(), revision);
        assert!(session(&app).corner_first.is_some());
    }
    frame(&mut app, "corner-refused");
    let at = screen(&app, P2::new(10.0, 0.0));
    app.hover_at(at);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        app.step();
    }
    app.run_steps(6);
    assert_eq!(session(&app).tool.label(), "Select");
    assert!(session(&app).corner_first.is_none());
    assert_eq!(session(&app).sketch, before);
    corner_mode(&mut app);
    pick(&mut app, P2::new(-10.0, 0.0));
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert!(session(&app).corner_first.is_none(), "document changes cancel an outstanding pick");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn corner_joins_an_arc_and_line_in_a_compact_dark_view() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let arc =
        sketch.add(EntityKind::Arc { center: P2::ZERO, start: P2::new(10.0, 0.0), end: P2::new(0.0, 10.0) }, false);
    sketch.add(EntityKind::Line { a: P2::new(0.0, 15.0), b: P2::ZERO }, false);
    sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
    let (mut app, _) = launch(sketch);
    app.set_size(vec2(960.0, 600.0));
    soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
    app.run_steps(12);
    let before = session(&app).sketch.entity(arc).unwrap().kind.clone();
    corner_mode(&mut app);
    let d = 50.0_f64.sqrt();
    pick(&mut app, P2::new(d, d));
    let at = screen(&app, P2::new(0.0, 5.0));
    app.hover_at(at);
    app.run_steps(6);
    assert!(session(&app).preview.as_ref().is_some_and(|p| p.operation.is_some()));
    frame(&mut app, "corner-arc-small-dark-preview");
    pick(&mut app, P2::new(0.0, 5.0));
    assert_eq!(session(&app).sketch.entity(arc).unwrap().kind, before);
    let regions = session(&app).sketch.regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert!((regions[0].area() - 25.0 * std::f64::consts::PI).abs() < 1e-6);
    frame(&mut app, "corner-arc-small-dark");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
