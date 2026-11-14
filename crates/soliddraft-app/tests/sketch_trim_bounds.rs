//! Selected-boundary trim through the existing Trim panel and native pointer input.

#[path = "support/ui.rs"]
mod ribbon;

use egui_kittest::kittest::Queryable;

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

fn dir() -> std::path::PathBuf {
    std::env::var_os("SOLIDDRAFT_BOUNDED_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-bounded-{}", std::process::id())))
}

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
    let pos =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap();
    assert!(st.viewport.rect.contains(pos));
    pos
}

fn pick(app: &mut App, p: P2) {
    ribbon::close_options(app);
    let at = screen(app, p);
    ribbon::click_at(app, at);
}

fn mode(app: &mut App, label: &str) {
    ribbon::run_command(app, "sketch_trim");
    choose_mode(app, label);
}

fn choose_mode(app: &mut App, label: &str) {
    ribbon::click_option(app, label);
    assert_eq!(session(app).tool.label(), format!("Trim {}", label.to_lowercase()));
}

#[test]
fn inside_trim_uses_the_two_chosen_bounds_not_other_crossings() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let target = sketch.add(EntityKind::Line { a: P2::new(-10.0, 0.0), b: P2::new(10.0, 0.0) }, false);
    for x in [-3.0, 3.0, 0.0] {
        sketch.add(EntityKind::Line { a: P2::new(x, -6.0), b: P2::new(x, 6.0) }, true);
    }
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    mode(&mut app, "Inside");
    pick(&mut app, P2::new(-3.0, 4.0));
    pick(&mut app, P2::new(3.0, 4.0));
    let at = screen(&app, P2::new(-6.0, 0.0));
    app.hover_at(at);
    app.run_steps(6);
    assert!(session(&app).preview.as_ref().is_some_and(|p| p.operation.is_some()));
    assert_eq!(session(&app).sketch, before, "bounds and preview do not edit geometry");
    pick(&mut app, P2::new(-6.0, 0.0));
    assert_eq!(
        session(&app).sketch.entity(target).unwrap().kind,
        EntityKind::Line { a: P2::new(-10.0, 0.0), b: P2::new(-3.0, 0.0) }
    );
    assert!(
        session(&app)
            .sketch
            .drawn()
            .any(|e| e.kind == EntityKind::Line { a: P2::new(3.0, 0.0), b: P2::new(10.0, 0.0) })
    );
    assert_eq!(session(&app).sketch.drawn().count(), 5);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

fn frame(app: &mut App, name: &str) {
    if std::env::var_os("SOLIDDRAFT_BOUNDED_PROOF_DIR").is_some() {
        app.render().unwrap().save(dir().join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn bounded_modes_keep_the_bounds_across_edits_and_share_atomic_history() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let first = sketch.add(EntityKind::Line { a: P2::new(-10.0, 2.0), b: P2::new(10.0, 2.0) }, false);
    let second = sketch.add(EntityKind::Line { a: P2::new(-10.0, -2.0), b: P2::new(10.0, -2.0) }, false);
    for x in [-3.0, 3.0, 0.0] {
        sketch.add(EntityKind::Line { a: P2::new(x, -6.0), b: P2::new(x, 6.0) }, true);
    }
    let (mut app, id) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    mode(&mut app, "Inside");
    pick(&mut app, P2::new(-3.0, 4.0));
    pick(&mut app, P2::new(3.0, 4.0));
    let bounds = session(&app).trim_bounds.clone();
    assert_eq!(bounds.len(), 2);
    let at = screen(&app, P2::new(-6.0, 2.0));
    app.hover_at(at);
    app.run_steps(6);
    assert_eq!(session(&app).preview.as_ref().unwrap().removed.len(), 1);
    frame(&mut app, "inside-preview");
    assert_eq!(session(&app).sketch, before);
    assert_eq!(app.state().state.doc.revision(), revision);
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).tool.label(), "Select");
    assert!(session(&app).trim_bounds.is_empty() && session(&app).selection.is_empty());
    assert_eq!(session(&app).sketch, before);
    mode(&mut app, "Inside");
    pick(&mut app, P2::new(-3.0, 4.0));
    pick(&mut app, P2::new(3.0, 4.0));
    pick(&mut app, P2::new(-6.0, 2.0));
    let after_first = session(&app).sketch.clone();
    assert_eq!(session(&app).trim_bounds, bounds, "own target edits retain the chosen bounds");
    assert_eq!(after_first.drawn().count(), before.drawn().count() + 1);
    choose_mode(&mut app, "Outside");
    assert_eq!(session(&app).trim_bounds, bounds, "changing inside/outside retains the bounds");
    let at = screen(&app, P2::new(-6.0, -2.0));
    app.hover_at(at);
    app.run_steps(6);
    assert_eq!(session(&app).preview.as_ref().unwrap().removed.len(), 2);
    frame(&mut app, "outside-preview");
    pick(&mut app, P2::new(-6.0, -2.0));
    let after = session(&app).sketch.clone();
    assert_eq!(after.entity(second).unwrap().kind, EntityKind::Line { a: P2::new(-3.0, -2.0), b: P2::new(3.0, -2.0) });
    assert_eq!(after.entity(first), after_first.entity(first));
    assert_eq!(session(&app).trim_bounds, bounds);
    let revision = app.state().state.doc.revision();
    pick(&mut app, P2::new(1.0, -2.0));
    assert_eq!(session(&app).sketch, after, "repeating an outside trim is a refused no-op");
    assert_eq!(app.state().state.doc.revision(), revision);
    frame(&mut app, "bounded-after");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, after_first);
    assert!(session(&app).trim_bounds.is_empty(), "external history changes discard stale bounds");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, after);
    let path = dir().join("bounded.dwprt");
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
fn bounded_trim_refusals_right_click_and_document_changes_are_safe() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Line { a: P2::new(-10.0, 0.0), b: P2::new(10.0, 0.0) }, false);
    for x in [-3.0, 3.0] {
        sketch.add(EntityKind::Line { a: P2::new(x, -6.0), b: P2::new(x, 6.0) }, true);
    }
    sketch.add(EntityKind::Circle { center: P2::new(15.0, 10.0), radius: 3.0 }, false);
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    mode(&mut app, "Inside");
    pick(&mut app, P2::new(-3.0, 4.0));
    pick(&mut app, P2::new(-3.0, 4.0));
    assert_eq!(session(&app).trim_bounds.len(), 1, "the same bound cannot be picked twice");
    pick(&mut app, P2::new(3.0, 4.0));
    for p in [P2::new(-3.0, 4.0), P2::new(15.0, 13.0)] {
        pick(&mut app, p);
        assert_eq!(session(&app).sketch, before);
        assert_eq!(app.state().state.doc.revision(), revision);
    }
    ribbon::open_options(&mut app);
    app.get_by_label("Choose bounds again").scroll_to_me();
    app.run_steps(24);
    frame(&mut app, "bounded-refused");
    assert!(
        ribbon::painted(&app).iter().any(|p| p.text == "Choose bounds again" && p.rect.contains_rect(p.full)),
        "the reset control must be visible before clicking it"
    );
    let window = ribbon::window(&app);
    ribbon::click_button(&mut app, "Choose bounds again", window);
    assert!(session(&app).tool.bounded_trim(), "reset keeps the current trim mode");
    assert!(session(&app).trim_bounds.is_empty() && session(&app).selection.is_empty());
    assert_eq!(session(&app).sketch, before);
    pick(&mut app, P2::new(-3.0, 4.0));
    let at = screen(&app, P2::new(0.0, 4.0));
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
    assert!(session(&app).trim_bounds.is_empty() && session(&app).selection.is_empty());
    assert_eq!(session(&app).sketch, before);
    mode(&mut app, "Outside");
    pick(&mut app, P2::new(-3.0, 4.0));
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert!(session(&app).trim_bounds.is_empty());
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn outside_trim_keeps_an_analytic_arc_in_a_compact_dark_view() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let arc =
        sketch.add(EntityKind::Arc { center: P2::ZERO, start: P2::new(10.0, 0.0), end: P2::new(0.0, 10.0) }, false);
    sketch.add(EntityKind::Line { a: P2::new(1.0, 5.0), b: P2::new(12.0, 5.0) }, true);
    sketch.add(EntityKind::Line { a: P2::new(5.0, 1.0), b: P2::new(5.0, 12.0) }, true);
    let (mut app, _) = launch(sketch);
    app.set_size(vec2(960.0, 600.0));
    soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
    app.run_steps(12);
    mode(&mut app, "Outside");
    pick(&mut app, P2::new(11.0, 5.0));
    pick(&mut app, P2::new(5.0, 11.0));
    ribbon::open_options(&mut app);
    app.get_by_label("Choose bounds again").scroll_to_me();
    app.run_steps(24);
    frame(&mut app, "bounded-arc-small-dark-controls");
    assert!(
        ribbon::painted(&app).iter().any(|p| p.text == "Choose bounds again" && p.rect.contains_rect(p.full)),
        "the bounds reset control remains visible in the compact panel"
    );
    ribbon::close_options(&mut app);
    let d = 50.0_f64.sqrt();
    let at = screen(&app, P2::new(d, d));
    app.hover_at(at);
    app.run_steps(6);
    assert_eq!(session(&app).preview.as_ref().unwrap().removed.len(), 2);
    frame(&mut app, "bounded-arc-small-dark-preview");
    pick(&mut app, P2::new(d, d));
    assert!(
        (session(&app).sketch.entity(arc).unwrap().arc_angles().unwrap().1 - std::f64::consts::PI / 6.0).abs() < 1e-6
    );
    frame(&mut app, "bounded-arc-small-dark");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
