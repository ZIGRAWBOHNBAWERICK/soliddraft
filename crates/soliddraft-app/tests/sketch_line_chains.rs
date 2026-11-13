//! Order 71: connected line placement and the boundaries between independent chains.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, PointerButton, Pos2};
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityId, EntityKind, P2, PointRef, Sketch},
    sketcher::Frame,
    state::{AppAction, SketchSession, Tool},
};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}

fn launch(name: &str) -> App {
    let dir = std::env::temp_dir().join(format!("soliddraft-line-chains-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir));
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut app = ribbon::app_at(doc, ribbon::EXPANDED_WINDOW, 1.0);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(20);
    ribbon::run_command(&mut app, "sketch_line");
    assert_eq!(session(&app).tool, Tool::Line);
    app
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    let at =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap();
    assert!(st.viewport.rect.contains(at));
    at
}

fn pick(app: &mut App, p: P2) {
    let at = screen(app, p);
    ribbon::click_at(app, at);
}

fn right_click(app: &mut App, p: P2) {
    let pos = screen(app, p);
    app.hover_at(pos);
    app.run_steps(3);
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

fn assert_line(sketch: &Sketch, id: EntityId, start: P2, end: P2) {
    let EntityKind::Line { a, b } = sketch.entity(id).unwrap().kind else { panic!("expected line {id}") };
    assert!(a.dist(start) < 1e-4, "line {id} starts at {a:?}, expected {start:?}");
    assert!(b.dist(end) < 1e-4, "line {id} ends at {b:?}, expected {end:?}");
}

fn assert_join(sketch: &Sketch, a: PointRef, b: PointRef) {
    assert!(
        sketch.constraints.iter().any(|c| matches!(c.kind,
            ConstraintKind::Coincident { a: x, b: y } if (x == a && y == b) || (x == b && y == a)
        )),
        "missing persistent join {a:?} to {b:?}"
    );
}

#[test]
fn start_and_continue_with_coincident_joins_even_without_auto_relations() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = launch("continuation");
    app.state_mut().state.sketch.as_mut().unwrap().auto_relations = false;
    let empty = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    let (a, b, c) = (P2::new(-30.0, -20.0), P2::new(0.0, -10.0), P2::new(25.0, 20.0));
    pick(&mut app, a);
    assert_eq!(session(&app).pending.len(), 1);
    assert!(session(&app).chain_from.is_none());
    assert_eq!(session(&app).sketch, empty, "the start is only a preview");
    assert_eq!(app.state().state.doc.revision(), revision);
    let start = session(&app).pending[0].0;
    pick(&mut app, b);
    let one = session(&app).sketch.clone();
    let join = one.point(PointRef::new(1, 1)).unwrap();
    assert_eq!(one.drawn().count(), 1);
    assert_eq!(session(&app).chain_from, Some(PointRef::new(1, 1)));
    pick(&mut app, c);
    let two = session(&app).sketch.clone();
    let end = session(&app).pending[0].0;
    assert_eq!(two.drawn().count(), 2);
    assert_line(&two, 1, start, join);
    assert_line(&two, 2, join, end);
    assert_join(&two, PointRef::new(1, 1), PointRef::new(2, 0));
    assert_eq!(session(&app).chain_from, Some(PointRef::new(2, 1)));
    assert_eq!(session(&app).tool, Tool::Line);

    // Each placed segment is one history entry; its stored join follows undo and redo.
    for expected in [&one, &empty] {
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        app.run_steps(6);
        assert_eq!(&session(&app).sketch, expected);
    }
    for expected in [&one, &two] {
        app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        app.run_steps(6);
        assert_eq!(&session(&app).sketch, expected);
    }
    let reopened: Sketch = serde_json::from_str(&serde_json::to_string(&two).unwrap()).unwrap();
    assert_eq!(reopened.entities, two.entities);
    assert_eq!(reopened.constraints, two.constraints);
    assert_join(&reopened, PointRef::new(1, 1), PointRef::new(2, 0));
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn right_click_ends_a_chain_and_the_next_click_starts_an_independent_one() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = launch("right-click");
    pick(&mut app, P2::ZERO);
    pick(&mut app, P2::new(30.0, 0.0));
    let one = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    right_click(&mut app, P2::new(40.0, 20.0));
    assert_eq!(session(&app).tool, Tool::Line);
    assert!(session(&app).pending.is_empty());
    assert!(session(&app).chain_from.is_none());
    assert_eq!(session(&app).sketch, one, "right-click must not commit the rubber band");
    assert_eq!(app.state().state.doc.revision(), revision);

    let start = P2::new(-30.0, -20.0);
    let end = P2::new(-10.0, -20.0);
    pick(&mut app, start);
    assert_eq!(session(&app).sketch, one);
    pick(&mut app, end);
    assert_eq!(session(&app).sketch.drawn().count(), 2);
    assert_line(&session(&app).sketch, 2, start, end);
    assert!(
        !session(&app).sketch.constraints.iter().any(|c| matches!(c.kind,
            ConstraintKind::Coincident { a, b } if a.entity != 0 && b.entity != 0
        )),
        "independent chains must not inherit the previous endpoint"
    );
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn double_click_lands_one_final_segment_and_keeps_the_line_tool() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = launch("double-click");
    pick(&mut app, P2::ZERO);
    pick(&mut app, P2::new(25.0, 0.0));
    let one = session(&app).sketch.clone();
    // Let prior placement clicks expire so the helper produces a double-click.
    app.run_steps(30);
    let end = P2::new(25.0, 20.0);
    let revision = app.state().state.doc.revision();
    let at = screen(&app, end);
    ribbon::double_click_at(&mut app, at);
    assert_eq!(session(&app).sketch.drawn().count(), 2, "no zero-length segment on the second click");
    assert_line(&session(&app).sketch, 2, P2::new(25.0, 0.0), end);
    assert_join(&session(&app).sketch, PointRef::new(1, 1), PointRef::new(2, 0));
    assert_eq!(app.state().state.doc.revision(), revision + 1);
    assert_eq!(session(&app).tool, Tool::Line);
    assert!(session(&app).pending.is_empty());
    assert!(session(&app).chain_from.is_none());
    let two = session(&app).sketch.clone();
    pick(&mut app, P2::new(-30.0, -20.0));
    assert_eq!(session(&app).sketch, two, "the next click starts a fresh chain");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, one, "the final segment has exactly one undo step");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn existing_point_closes_a_chain_without_leaving_the_tool() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = launch("closure");
    pick(&mut app, P2::ZERO);
    pick(&mut app, P2::new(30.0, 0.0));
    pick(&mut app, P2::new(30.0, 20.0));
    pick(&mut app, P2::ZERO);
    let closed = session(&app).sketch.clone();
    assert_eq!(closed.drawn().count(), 3);
    assert_eq!(closed.regions().unwrap().len(), 1);
    assert_join(&closed, PointRef::new(3, 1), PointRef::ORIGIN);
    assert!(session(&app).pending.is_empty());
    assert!(session(&app).chain_from.is_none());
    assert_eq!(session(&app).tool, Tool::Line);
    pick(&mut app, P2::new(-30.0, -20.0));
    assert_eq!(session(&app).sketch, closed);
    pick(&mut app, P2::new(-10.0, -20.0));
    assert_line(&session(&app).sketch, 4, P2::new(-30.0, -20.0), P2::new(-10.0, -20.0));
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn cancel_discards_only_uncommitted_placement_and_escape_puts_the_tool_down() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = launch("cancel");
    let empty = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    pick(&mut app, P2::new(-30.0, -20.0));
    right_click(&mut app, P2::new(-10.0, -20.0));
    assert_eq!(session(&app).tool, Tool::Line);
    assert!(session(&app).pending.is_empty());
    assert!(session(&app).chain_from.is_none());
    assert_eq!(session(&app).sketch, empty);
    assert_eq!(app.state().state.doc.revision(), revision);

    for placed in [false, true] {
        pick(&mut app, P2::ZERO);
        if placed {
            pick(&mut app, P2::new(30.0, 0.0));
        }
        let before = session(&app).sketch.clone();
        let revision = app.state().state.doc.revision();
        app.key_press(Key::Escape);
        app.run_steps(6);
        assert_eq!(session(&app).tool, Tool::Select);
        assert!(session(&app).pending.is_empty());
        assert!(session(&app).chain_from.is_none());
        assert_eq!(session(&app).sketch, before);
        assert_eq!(app.state().state.doc.revision(), revision);
        app.key_press(Key::L);
        app.run_steps(6);
    }
    // With no chain in progress, right-click puts the tool down as well.
    right_click(&mut app, P2::new(40.0, 20.0));
    assert_eq!(session(&app).tool, Tool::Select);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
