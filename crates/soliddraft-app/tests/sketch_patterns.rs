//! Retained pattern controls through the rendered sketch editor.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::tools::{Operation, PatternSpec},
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
        app.run_steps(36);
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
        app.run_steps(36);
    }
    proof(app, "missing-control");
    panic!("visible {label}");
}
fn number(app: &mut App, label: &str, value: f64) {
    let mut attempts = 0;
    let rect = loop {
        attempts += 1;
        assert!(attempts <= 12, "visible numeric field {label}");
        let rect = app.get_by_role_and_label(egui::accesskit::Role::SpinButton, label).rect();
        if pane(app).contains_rect(rect) {
            break rect;
        }
        let pane = pane(app);
        app.hover_at(Pos2::new(pane.right() - 15.0, pane.bottom() - 40.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -100.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        app.run_steps(36);
    };
    ribbon::double_click_at(app, rect.center());
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text(value.to_string()));
    app.run_steps(2);
    app.key_press(Key::Enter);
    app.run_steps(6);
    assert!(app.state().state.sketch.is_some(), "stayed in sketch after {label}");
    let m = &session(app).modify;
    let actual = match label {
        "First step u" => m.step.u,
        "First step v" => m.step.v,
        "First count" => f64::from(m.count),
        "Second step u" => m.second_step.u,
        "Second step v" => m.second_step.v,
        "Second count" => f64::from(m.second_count),
        "Centre u" => m.center.u,
        "Centre v" => m.center.v,
        "Pattern angle" => m.angle,
        "Pattern count" => f64::from(m.count),
        _ => panic!("known numeric control"),
    };
    assert_eq!(actual, value, "committed {label}");
}
fn scroll_top(app: &mut App) {
    let pane = pane(app);
    app.hover_at(Pos2::new(pane.right() - 15.0, pane.bottom() - 40.0));
    app.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, 2000.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(36);
}
fn setup(name: &str) -> (App, std::path::PathBuf, soliddraft_app::document::FeatureId) {
    let mut sketch = Sketch::new();
    let id = sketch.add(EntityKind::Circle { center: p(10.0, 10.0), radius: 2.0 }, false);
    sketch.constrain(ConstraintKind::Fix { point: PointRef::new(id, 0), at: p(10.0, 10.0) });
    sketch.constrain(ConstraintKind::Radius { entity: id, value: 2.0, diameter: false, label: p(14.0, 14.0) });
    solve::solve(&mut sketch, None, true);
    setup_custom(sketch, name)
}
fn setup_custom(sketch: Sketch, name: &str) -> (App, std::path::PathBuf, soliddraft_app::document::FeatureId) {
    let dir = std::env::temp_dir().join(format!("soliddraft-patterns-{name}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    let st = &mut app.state_mut().state;
    st.viewport.camera.zoom(0.1, None, st.viewport.rect.aspect_ratio());
    app.run_steps(6);
    (app, dir, id)
}
fn apply(app: &mut App) {
    button(app, "Apply");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(app).tool, Tool::Select);
    scroll_top(app);
}
fn reopen(app: &mut App, dir: &std::path::Path, id: soliddraft_app::document::FeatureId) {
    let expected = session(app).sketch.clone();
    let path = dir.join("patterns.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(12);
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, expected);
}
fn pixel(app: &mut App, at: P2) -> [u8; 4] {
    let st = &app.state().state;
    let pos =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(at).unwrap();
    app.render().unwrap().get_pixel(pos.x.floor() as u32, pos.y.floor() as u32).0
}
fn proof(app: &mut App, name: &str) {
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_PATTERN_PROOF_DIR") {
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
fn grid_skips_retained_edits_cancel_history_and_reopen() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, dir, id) = setup("grid");
    let before = session(&app).sketch.clone();
    pick(&mut app, p(12.0, 10.0));
    ribbon::run_command(&mut app, "sketch_linear_pattern");
    number(&mut app, "First step u", 12.0);
    number(&mut app, "First step v", 0.0);
    number(&mut app, "First count", 3.0);
    text(&mut app, "Second direction");
    number(&mut app, "Second step u", 0.0);
    number(&mut app, "Second step v", 12.0);
    number(&mut app, "Second count", 2.0);
    assert!(session(&app).modify.two_directions);
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 5);
    text(&mut app, "Skip instances");
    text(&mut app, "Instance 3, 2");
    assert_eq!(session(&app).modify.skipped, vec![[2, 1]]);
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 4);
    assert_eq!(session(&app).sketch, before);
    proof(&mut app, "grid-skipped-preview");
    apply(&mut app);
    let created = session(&app).sketch.clone();
    assert_eq!(created.drawn().count(), 5);
    assert_eq!(created.patterns.len(), 1);
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    redo(&mut app);
    assert_eq!(session(&app).sketch, created);
    button(&mut app, "Edit pattern 1");
    scroll_top(&mut app);
    number(&mut app, "First step u", 15.0);
    let preview = session(&app).preview.as_ref().unwrap();
    assert_eq!(preview.replaced.len(), 3, "unchanged copies remain in the original drawing");
    proof(&mut app, "grid-edit-preview");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, created);
    button(&mut app, "Edit pattern 1");
    scroll_top(&mut app);
    number(&mut app, "First step u", 15.0);
    apply(&mut app);
    let edited = session(&app).sketch.clone();
    assert_eq!(
        edited.entities.iter().map(|e| e.id).collect::<Vec<_>>(),
        created.entities.iter().map(|e| e.id).collect::<Vec<_>>()
    );
    undo(&mut app);
    assert_eq!(session(&app).sketch, created);
    redo(&mut app);
    assert_eq!(session(&app).sketch, edited);
    button(&mut app, "Edit pattern 1");
    scroll_top(&mut app);
    // The collapsing panel remembers its open state across editing.
    for label in ["Instance 2, 1", "Instance 3, 1", "Instance 1, 2", "Instance 2, 2"] {
        text(&mut app, label);
    }
    apply(&mut app);
    assert_eq!(session(&app).sketch.drawn().count(), 1);
    assert!(session(&app).sketch.patterns[0].instances.is_empty());
    pick(&mut app, p(-15.0, -15.0));
    assert!(session(&app).selection.is_empty());
    button(&mut app, "Edit pattern 1");
    scroll_top(&mut app);
    text(&mut app, "Instance 2, 1");
    apply(&mut app);
    assert_eq!(session(&app).sketch.drawn().count(), 2);
    proof(&mut app, "grid-restored-instance");
    reopen(&mut app, &dir, id);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn circular_total_spans_skips_and_reopen() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, dir, id) = setup("circular");
    let omitted_background = pixel(&mut app, p(-9.3, 10.7));
    let omitted_centre_background = pixel(&mut app, p(-10.0, 10.0));
    pick(&mut app, p(12.0, 10.0));
    ribbon::run_command(&mut app, "sketch_circular_pattern");
    number(&mut app, "Centre u", 0.0);
    number(&mut app, "Centre v", 10.0);
    text(&mut app, "Total angle");
    number(&mut app, "Pattern angle", 180.0);
    number(&mut app, "Pattern count", 3.0);
    assert!(session(&app).modify.total_angle);
    let centers = session(&app)
        .preview
        .as_ref()
        .unwrap()
        .added
        .iter()
        .map(|k| match k {
            EntityKind::Circle { center, .. } => *center,
            _ => panic!("circle"),
        })
        .collect::<Vec<_>>();
    assert!((centers[0] - p(0.0, 20.0)).len() < 1e-6);
    assert!((centers[1] - p(-10.0, 10.0)).len() < 1e-6);
    proof(&mut app, "circular-half-span-preview");
    apply(&mut app);
    let half = session(&app).sketch.clone();
    button(&mut app, "Edit pattern 1");
    scroll_top(&mut app);
    number(&mut app, "Pattern angle", 360.0);
    number(&mut app, "Pattern count", 4.0);
    text(&mut app, "Skip instances");
    text(&mut app, "Instance 3");
    assert_eq!(
        session(&app).preview.as_ref().unwrap().added.len(),
        1,
        "unchanged copies remain in the original drawing"
    );
    assert_eq!(pixel(&mut app, p(-9.3, 10.7)), omitted_background, "skipped instance has no stale region fill");
    assert_eq!(
        pixel(&mut app, p(-10.0, 10.0)),
        omitted_centre_background,
        "skipped instance has no stale centre marker"
    );
    proof(&mut app, "circular-full-span-skipped-preview");
    apply(&mut app);
    let full = session(&app).sketch.clone();
    assert_eq!(full.drawn().count(), 3);
    assert_eq!(full.patterns[0].skipped, vec![[2, 0]]);
    undo(&mut app);
    assert_eq!(session(&app).sketch, half);
    redo(&mut app);
    assert_eq!(session(&app).sketch, full);
    reopen(&mut app, &dir, id);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}

fn hover(app: &mut App, at: P2) {
    let st = &app.state().state;
    let pos =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(at).unwrap();
    app.hover_at(pos);
    app.run_steps(6);
}
#[test]
fn retained_circular_pivot_ignores_hover_and_moves_only_on_click() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let kind = EntityKind::Circle { center: p(10.0, 0.0), radius: 2.0 };
    let seed = sketch.add(kind.clone(), false);
    sketch.constrain(ConstraintKind::FixEntity { entity: seed, at: kind });
    sketch
        .apply_operation(&Operation::Pattern {
            entities: vec![seed],
            spec: PatternSpec::Circular { center: P2::ZERO, angle_degrees: 180.0, count: 2, total_angle: false },
            skipped: vec![],
        })
        .unwrap();
    solve::solve(&mut sketch, None, true);
    let (mut app, dir, id) = setup_custom(sketch, "pivot");
    let before = session(&app).sketch.clone();
    button(&mut app, "Edit pattern 1");
    hover(&mut app, p(0.0, 10.0));
    proof(&mut app, "saved-pivot-hover");
    assert_eq!(session(&app).modify.center, P2::ZERO, "hover keeps the saved pivot");
    assert_eq!(session(&app).sketch, before);
    assert!(session(&app).preview.as_ref().unwrap().operation.is_none());
    pick(&mut app, p(0.0, 10.0));
    assert!((session(&app).modify.center - p(0.0, 10.0)).len() < 1e-3, "explicit click relocates pivot");
    hover(&mut app, p(0.0, -10.0));
    assert!((session(&app).modify.center - p(0.0, 10.0)).len() < 1e-3);
    proof(&mut app, "relocated-pivot-preview");
    button(&mut app, "Apply");
    hover(&mut app, p(0.0, -10.0));
    assert!((session(&app).modify.center - p(0.0, 10.0)).len() < 1e-3, "Apply keeps the pivot placed");
    app.key_press(Key::Escape);
    app.run_steps(6);
    let after = session(&app).sketch.clone();
    let EntityKind::Circle { center, .. } = after.entity(2).unwrap().kind else { panic!("circle") };
    assert!((center - p(-10.0, 20.0)).len() < 2e-3);
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    redo(&mut app);
    assert_eq!(session(&app).sketch, after);
    reopen(&mut app, &dir, id);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn constrained_copy_edit_previews_moved_seed_and_dependent_geometry() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let seed = sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 2.0 }, false);
    for step in [p(10.0, 0.0), p(0.0, 5.0)] {
        sketch
            .apply_operation(&Operation::Pattern {
                entities: vec![seed],
                spec: PatternSpec::Linear { step, count: 2, second: None },
                skipped: vec![],
            })
            .unwrap();
    }
    let fixed = sketch.entity(2).unwrap().kind.clone();
    sketch.constrain(ConstraintKind::FixEntity { entity: 2, at: fixed });
    sketch.entity_mut(3).unwrap().construction = true;
    solve::solve(&mut sketch, None, true);
    let (mut app, dir, id) = setup_custom(sketch, "solved-preview");
    let before = session(&app).sketch.clone();
    button(&mut app, "Edit pattern 1");
    scroll_top(&mut app);
    number(&mut app, "First step u", 20.0);
    let preview = session(&app).preview.as_ref().unwrap().clone();
    proof(&mut app, "solved-source-dependent-preview");
    assert!(preview.replaced.contains(&seed), "preview replaces the moved source");
    assert!(preview.replaced.contains(&3), "preview replaces moved dependent geometry");
    let mut expected = before.clone();
    expected.apply_operation(preview.operation.as_ref().unwrap()).unwrap();
    let changed: Vec<_> = expected
        .drawn()
        .filter(|e| before.entity(e.id).is_none_or(|old| old.kind != e.kind || old.construction != e.construction))
        .collect();
    assert_eq!(preview.added, changed.iter().map(|e| e.kind.clone()).collect::<Vec<_>>());
    assert_eq!(preview.construction, changed.iter().map(|e| e.construction).collect::<Vec<_>>());
    assert_eq!(session(&app).sketch, before);
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before, "Escape preserves source,copy and dependencies");
    button(&mut app, "Edit pattern 1");
    scroll_top(&mut app);
    number(&mut app, "First step u", 20.0);
    apply(&mut app);
    let after = session(&app).sketch.clone();
    assert_eq!(after.entities.len(), expected.entities.len());
    for (actual, shown) in after.entities.iter().zip(&expected.entities) {
        assert_eq!(actual.id, shown.id);
        assert_eq!(std::mem::discriminant(&actual.kind), std::mem::discriminant(&shown.kind));
        assert_eq!(actual.construction, shown.construction);
        for (a, b) in actual.points().into_iter().zip(shown.points()) {
            assert!(a.dist(b) < 1e-6, "Apply matches the complete solved preview");
        }
        if let (Some(a), Some(b)) = (actual.radius(), shown.radius()) {
            assert!((a - b).abs() < 1e-6);
        }
    }
    for (entity, at) in [(seed, p(-10.0, 0.0)), (3, p(-10.0, 5.0))] {
        let EntityKind::Circle { center, .. } = after.entity(entity).unwrap().kind else { panic!("circle") };
        assert!((center - at).len() < 1e-6);
    }
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    redo(&mut app);
    assert_eq!(session(&app).sketch, after);
    reopen(&mut app, &dir, id);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn power_trim_retrace_restores_pattern_without_a_document_edit() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let seed = sketch.add(EntityKind::Line { a: P2::ZERO, b: p(2.0, 0.0) }, false);
    sketch
        .apply_operation(&Operation::Pattern {
            entities: vec![seed],
            spec: PatternSpec::Linear { step: p(0.0, 10.0), count: 4, second: None },
            skipped: vec![],
        })
        .unwrap();
    solve::solve(&mut sketch, None, true);
    let (mut app, dir, _id) = setup_custom(sketch, "trim-retrace");
    let st = &mut app.state_mut().state;
    st.viewport.camera.zoom(10.0, None, st.viewport.rect.aspect_ratio());
    app.run_steps(6);
    ribbon::run_command(&mut app, "sketch_trim");
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    hover(&mut app, p(1.0, 29.0));
    let at = {
        let st = &app.state().state;
        Frame { plane: session(&app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }
            .screen(p(1.0, 29.0))
            .unwrap()
    };
    app.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.step();
    hover(&mut app, p(1.0, 31.0));
    assert_eq!(session(&app).sketch.drawn().count(), 3);
    assert!(session(&app).sketch.patterns.is_empty());
    assert_eq!(session(&app).trim_drag.as_ref().unwrap().checkpoints.len(), 1);
    proof(&mut app, "pattern-last-copy-trimmed");
    hover(&mut app, p(1.0, 29.0));
    assert_eq!(session(&app).sketch, before, "pointer retrace restores every pattern link");
    app.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(6);
    assert_eq!(app.state().state.doc.revision(), revision, "fully retraced gesture records no edit");
    assert!(session(&app).trim_drag.is_none());
    proof(&mut app, "pattern-retrace-restored");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}
