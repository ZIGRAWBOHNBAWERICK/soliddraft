//! Offset choices and linked-distance editing through the rendered sketch editor.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, PointRef, Sketch, solve},
    sketcher::Frame,
    state::{AppAction, SketchSession, Tool},
};

static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}
fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn pick(app: &mut App, at: P2) {
    let st = &app.state().state;
    let pos =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(at).unwrap();
    ribbon::click_at(app, pos);
}
fn pane(app: &App) -> egui::Rect {
    let view = app.state().state.viewport.rect;
    egui::Rect::from_min_max(egui::pos2(0.0, view.top()), egui::pos2(view.left(), view.bottom()))
}
fn button(app: &mut App, label: &str) {
    for _ in 0..10 {
        let pane = pane(app);
        if app.get_all_by_role_and_label(egui::accesskit::Role::Button, label).any(|n| pane.contains_rect(n.rect())) {
            ribbon::click_button(app, label, pane);
            app.run_steps(6);
            return;
        }
        app.hover_at(Pos2::new(pane.right() - 15.0, pane.bottom() - 40.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -100.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        app.run_steps(6);
    }
    panic!("visible button {label}");
}
fn text(app: &mut App, label: &str) {
    for _ in 0..12 {
        if let Some(control) =
            ribbon::painted(app).into_iter().find(|p| p.text == label && p.rect.contains_rect(p.full))
        {
            ribbon::click_at(app, control.rect.center());
            app.run_steps(6);
            return;
        }
        let pane = pane(app);
        app.hover_at(Pos2::new(pane.right() - 15.0, pane.bottom() - 40.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -100.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        app.run_steps(6);
    }
    proof(app, "missing-control");
    panic!("visible {label}");
}
fn distance(app: &mut App, value: f64) {
    let rect = app.get_by_role(egui::accesskit::Role::SpinButton).rect();
    proof(app, "distance-field");
    assert!(pane(app).contains_rect(rect), "the distance field must be fully visible before clicking it");
    assert!(
        rect.bottom() <= ribbon::band(app).top(),
        "the distance field must be visible above the command bar: {rect:?}"
    );
    ribbon::double_click_at(app, rect.center());
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text(value.to_string()));
    app.key_press(Key::Enter);
    app.run_steps(6);
    assert_eq!(session(app).modify.distance, value);
}
fn proof(app: &mut App, name: &str) {
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_OFFSET_PROOF_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        app.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
    }
}
fn undo(app: &mut App) {
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
}
fn redo(app: &mut App) {
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
}

#[test]
fn chains_caps_guides_and_linked_distance_use_preview_one_undo_and_reopen() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("soliddraft-offsets-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut sketch = Sketch::new();
    for (a, b) in [(p(-20.0, 0.0), P2::ZERO), (P2::ZERO, p(20.0, 0.0))] {
        let id = sketch.add(EntityKind::Line { a, b }, false);
        for (index, at) in [(0, a), (1, b)] {
            sketch.constrain(ConstraintKind::Fix { point: PointRef::new(id, index), at });
        }
    }
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
    pick(&mut app, p(-10.0, 0.0));
    ribbon::run_command(&mut app, "sketch_offset");
    distance(&mut app, 4.0);
    assert!(session(&app).modify.offset.chain);
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 2);
    text(&mut app, "Chain selection");
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 1);
    text(&mut app, "Chain selection");
    button(&mut app, "Reverse");
    assert_eq!(session(&app).modify.distance, -4.0);
    text(&mut app, "Both sides");
    text(&mut app, "None");
    text(&mut app, "Round");
    assert!(session(&app).modify.offset.make_base_construction);
    let preview = session(&app).preview.as_ref().unwrap();
    assert_eq!(preview.added.len(), 8, "six additions and two construction replacements");
    assert_eq!(preview.construction.iter().filter(|&&c| c).count(), 2);
    assert_eq!(preview.replaced, vec![1, 2]);
    let mut expected = before.clone();
    expected.apply_operation(preview.operation.as_ref().unwrap()).unwrap();
    solve::solve(&mut expected, None, true);
    assert_eq!(session(&app).sketch, before);
    proof(&mut app, "round-both-sides-preview");
    button(&mut app, "Apply");
    assert_eq!(session(&app).sketch.entities, expected.entities);
    assert_eq!(session(&app).sketch.constraints, expected.constraints);
    assert_eq!(session(&app).sketch.regions().unwrap().len(), 1);
    let after = session(&app).sketch.clone();
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    redo(&mut app);
    assert_eq!(session(&app).sketch, after);

    app.key_press(Key::Escape);
    app.run_steps(6);
    pick(&mut app, p(24.0, 0.0));
    button(&mut app, "Edit offset distance");
    assert!(session(&app).modify.offset_group.is_some());
    distance(&mut app, 3.0);
    assert!(session(&app).preview.as_ref().unwrap().operation.is_some());
    proof(&mut app, "linked-distance-preview");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).tool, Tool::Select);
    assert_eq!(session(&app).sketch, after);
    ribbon::run_command(&mut app, "sketch_offset");
    assert_eq!(session(&app).modify.offset_group, None, "a new command does not resume a cancelled edit");
    assert!(matches!(
        soliddraft_app::sketch_tools::operation_for(session(&app)),
        Some(soliddraft_app::sketch::tools::Operation::Offset { .. })
    ));
    app.key_press(Key::Escape);
    app.run_steps(6);
    pick(&mut app, p(24.0, 0.0));
    button(&mut app, "Edit offset distance");
    distance(&mut app, 3.0);
    button(&mut app, "Apply");
    let edited = session(&app).sketch.clone();
    assert_eq!(
        edited.entities.iter().map(|e| e.id).collect::<Vec<_>>(),
        after.entities.iter().map(|e| e.id).collect::<Vec<_>>()
    );
    assert!(
        edited
            .drawn()
            .filter(|e| matches!(e.kind, EntityKind::Arc { .. }))
            .all(|e| (e.radius().unwrap() - 3.0).abs() < 1e-6)
    );
    undo(&mut app);
    assert_eq!(session(&app).sketch, after);
    redo(&mut app);
    assert_eq!(session(&app).sketch, edited);
    proof(&mut app, "linked-distance-applied");
    let path = dir.join("offsets.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(12);
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, edited);

    drop(app);
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch: before.clone() }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    pick(&mut app, p(-10.0, 0.0));
    ribbon::run_command(&mut app, "sketch_offset");
    distance(&mut app, 2.0);
    text(&mut app, "None");
    text(&mut app, "Lines");
    button(&mut app, "Apply");
    assert_eq!(session(&app).sketch.drawn().count(), 6);
    assert!(session(&app).sketch.drawn().all(|e| !e.construction));
    assert_eq!(session(&app).sketch.regions().unwrap().len(), 1);
    proof(&mut app, "one-sided-line-caps");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

fn fixed_segment(sketch: &mut Sketch, a: P2, b: P2) {
    let id = sketch.add(EntityKind::Line { a, b }, false);
    for (index, at) in [(0, a), (1, b)] {
        sketch.constrain(ConstraintKind::Fix { point: PointRef::new(id, index), at });
    }
}
fn review_app(sketch: Sketch, name: &str) -> App {
    let dir = std::env::temp_dir().join(format!("offset-review-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir));
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    app
}
fn review_screen(app: &App, at: P2) -> Pos2 {
    let st = &app.state().state;
    let pos =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(at).unwrap();
    assert!(st.viewport.rect.contains(pos));
    pos
}
fn ctrl_pick(app: &mut App, at: P2) {
    let pos = review_screen(app, at);
    app.hover_at(pos);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event_modifiers(
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::CTRL,
            },
            Modifiers::CTRL,
        );
        app.step();
    }
    app.run_steps(6);
}
fn review_drag(app: &mut App, a: P2, b: P2) {
    let (a, b) = (review_screen(app, a), review_screen(app, b));
    app.hover_at(a);
    app.run_steps(4);
    app.event(egui::Event::PointerButton {
        pos: a,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.step();
    for i in 1..=8 {
        app.hover_at(a + (b - a) * (i as f32 / 8.0));
        app.step();
    }
    app.event(egui::Event::PointerButton {
        pos: b,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.step();
    app.run_steps(6);
}
#[test]
fn auto_chain_drag_uses_the_reversed_nonadjacent_selected_member() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    fixed_segment(&mut sketch, p(-10.0, -6.0), p(0.0, -6.0));
    fixed_segment(&mut sketch, p(0.0, -6.0), p(0.0, 6.0));
    fixed_segment(&mut sketch, p(10.0, 6.0), p(0.0, 6.0));
    let mut app = review_app(sketch, "direction");
    let before = session(&app).sketch.clone();
    pick(&mut app, p(-5.0, -6.0));
    ribbon::run_command(&mut app, "sketch_offset");
    ctrl_pick(&mut app, p(5.0, 6.0));
    assert_eq!(soliddraft_app::sketch_tools::selected_in_order(session(&app)), vec![1, 3]);
    assert!(session(&app).modify.offset.chain);
    review_drag(&mut app, p(5.0, 6.0), p(5.0, 4.0));
    proof(&mut app, "auto-chain-reversed-drag");
    assert!(
        (session(&app).modify.distance + 2.0).abs() < 1e-4,
        "drag must use the expanded chain direction: {}",
        session(&app).modify.distance
    );
    let preview = session(&app).preview.as_ref().unwrap();
    assert_eq!(preview.added.len(), 3);
    let EntityKind::Line { a, b } = preview.added[2] else { panic!("offset member") };
    assert!((a.v - 4.0).abs() < 1e-4 && (b.v - 4.0).abs() < 1e-4);
    button(&mut app, "Apply");
    assert_eq!(session(&app).sketch.drawn().count(), 6);
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
#[test]
fn collinear_offset_join_cannot_be_dragged_beyond_its_neighbor() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    fixed_segment(&mut sketch, p(0.0, 0.0), p(10.0, 0.0));
    fixed_segment(&mut sketch, p(10.0, 0.0), p(20.0, 0.0));
    let mut app = review_app(sketch, "join");
    pick(&mut app, p(5.0, 0.0));
    ribbon::run_command(&mut app, "sketch_offset");
    distance(&mut app, 2.0);
    button(&mut app, "Apply");
    app.key_press(Key::Escape);
    app.run_steps(6);
    let point = PointRef::new(3, 1);
    assert!((session(&app).sketch.point(point).unwrap().u - 10.0).abs() < 1e-6);
    review_drag(&mut app, p(10.0, 2.0), p(25.0, 2.0));
    proof(&mut app, "collinear-join-drag");
    let join = session(&app).sketch.point(point).unwrap();
    assert!((join.u - 10.0).abs() < 1e-6, "the internal offset join must stay on its source normal: {join:?}");
    assert!((session(&app).sketch.point(PointRef::new(4, 0)).unwrap().u - 10.0).abs() < 1e-6);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn a_collinear_offset_accepts_a_constraint_driven_source_corner() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    fixed_segment(&mut sketch, p(0.0, 0.0), p(10.0, 0.0));
    fixed_segment(&mut sketch, p(10.0, 0.0), p(20.0, 0.0));
    let mut app = review_app(sketch, "source-corner");
    pick(&mut app, p(5.0, 0.0));
    ribbon::run_command(&mut app, "sketch_offset");
    distance(&mut app, 2.0);
    button(&mut app, "Apply");
    app.key_press(Key::Escape);
    app.run_steps(6);
    let before = session(&app).sketch.clone();
    let mut edited = before.clone();
    for c in &mut edited.constraints {
        if let ConstraintKind::Fix { point, .. } = c.kind
            && point == PointRef::new(2, 1)
        {
            c.kind = ConstraintKind::Fix { point, at: p(10.0, 10.0) };
        }
    }
    // Change only the driving constraint. The solver must discover the new source angle.
    solve::solve(&mut edited, None, true);
    let id = session(&app).feature;
    app.state_mut().state.sketch.as_mut().unwrap().sketch = edited.clone();
    app.state_mut().state.push(AppAction::Command(Command::EditSketch { id, sketch: edited }));
    app.run_steps(6);
    proof(&mut app, "straight-to-corner");
    let s = &session(&app).sketch;
    assert!(
        !matches!(
            s.status,
            soliddraft_app::sketch::SolveStatus::Failed(_) | soliddraft_app::sketch::SolveStatus::OverDefined { .. }
        ),
        "valid source corner must solve: {:?}",
        s.status
    );
    for point in [PointRef::new(3, 1), PointRef::new(4, 0)] {
        assert!(
            s.point(point).unwrap().dist(p(8.0, 2.0)) < 1e-6,
            "miter follows changed source angle: {:?}",
            s.point(point)
        );
    }
    let after = s.clone();
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    redo(&mut app);
    assert_eq!(session(&app).sketch, after);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
