//! Linked and dynamic mirror through the rendered sketch editor.
#[path = "support/ribbon.rs"]
mod ribbon;
use egui::{Key, Modifiers, Pos2};
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, Sketch, solve},
    sketcher::Frame,
    state::{AppAction, SketchSession, Tool},
};
fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}
fn screen(app: &App, at: P2) -> Pos2 {
    let st = &app.state().state;
    let pos =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(at).unwrap();
    assert!(st.viewport.rect.contains(pos), "{at:?} inside viewport");
    pos
}
fn hover(app: &mut App, at: P2) {
    let pos = screen(app, at);
    app.hover_at(pos);
    app.run_steps(5);
}
fn pick(app: &mut App, at: P2, ctrl: bool) {
    let pos = screen(app, at);
    if ctrl {
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
        app.run_steps(5);
    } else {
        ribbon::click_at(app, pos);
    }
}
fn control(app: &mut App, label: &str) {
    for attempt in 0..14 {
        if let Some(p) = ribbon::painted(app).into_iter().find(|p| p.text == label && p.rect.contains_rect(p.full)) {
            ribbon::click_at(app, p.rect.center());
            app.run_steps(6);
            return;
        }
        let view = app.state().state.viewport.rect;
        app.hover_at(egui::pos2(view.left() - 15.0, view.bottom() - 50.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, if attempt == 0 { 10000.0 } else { -100.0 }),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        app.run_steps(6);
    }
    proof(app, "missing-control");
    panic!("visible {label}");
}
fn proof(app: &mut App, name: &str) {
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_MIRROR_PROOF_DIR") {
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
fn launch(sketch: Sketch) -> (App, u64) {
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
fn axis(s: &mut Sketch) {
    let at = EntityKind::Line { a: p(0.0, -20.0), b: p(0.0, 20.0) };
    let entity = s.add(at, true);
    assert_eq!(entity, 1);
    s.constrain(ConstraintKind::FixEntity { entity, at });
}
fn reopen(app: &mut App, id: u64, name: &str) -> Sketch {
    let expected = session(app).sketch.clone();
    let path = std::env::temp_dir().join(format!("mirror-{}-{name}.dwprt", std::process::id()));
    app.state_mut().state.push(AppAction::ExitSketch(None));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, expected);
    expected
}
#[test]
fn picked_axis_and_dynamic_drawing_preview_cancel_undo_and_reopen() {
    let dir = std::env::temp_dir().join(format!("mirror-config-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir));
    let mut sketch = Sketch::new();
    axis(&mut sketch);
    for (a, b) in [(p(0.0, -6.0), p(8.0, -6.0)), (p(8.0, -6.0), p(8.0, 6.0)), (p(8.0, 6.0), p(0.0, 6.0))] {
        sketch.add(EntityKind::Line { a, b }, false);
    }
    let (mut app, id) = launch(sketch);
    pick(&mut app, p(4.0, -6.0), false);
    pick(&mut app, p(8.0, 0.0), true);
    pick(&mut app, p(4.0, 6.0), true);
    ribbon::click_ribbon(&mut app, "Mirror entities");
    control(&mut app, "Two points");
    control(&mut app, "Line 1");
    assert_eq!(session(&app).modify.mirror_axis, Some(1));
    assert!(session(&app).modify.keep_symmetry);
    let before = session(&app).sketch.clone();
    let preview = session(&app).preview.as_ref().unwrap();
    assert_eq!(preview.added.len(), 3);
    let mut expected = before.clone();
    expected.apply_operation(preview.operation.as_ref().unwrap()).unwrap();
    solve::solve(&mut expected, None, true);
    proof(&mut app, "picked-axis-preview");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    ribbon::click_ribbon(&mut app, "Mirror entities");
    control(&mut app, "Apply");
    assert_eq!(session(&app).sketch.entities, expected.entities);
    assert!((session(&app).sketch.regions().unwrap()[0].area() - 192.0).abs() < 1e-6);
    let after = session(&app).sketch.clone();
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    redo(&mut app);
    assert_eq!(session(&app).sketch, after);
    reopen(&mut app, id, "picked");
    drop(app);

    let mut guide = Sketch::new();
    axis(&mut guide);
    guide.add(EntityKind::Circle { center: p(12.0, 0.0), radius: 3.0 }, true);
    let (mut app, id) = launch(guide);
    pick(&mut app, p(15.0, 0.0), false);
    ribbon::click_ribbon(&mut app, "Mirror entities");
    control(&mut app, "Two points");
    control(&mut app, "Line 1");
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 1);
    proof(&mut app, "construction-circle-preview");
    let image = app.render().unwrap();
    let ppp = app.ctx.pixels_per_point();
    let covered = (0..120)
        .filter(|i| {
            let angle = f64::from(*i) * std::f64::consts::TAU / 120.0;
            let at = screen(&app, p(-12.0 + 3.0 * angle.cos(), 3.0 * angle.sin())) * ppp;
            let pixel = image.get_pixel(at.x.floor() as u32, at.y.floor() as u32);
            i16::from(pixel[2]) > i16::from(pixel[0]) + 40 && i16::from(pixel[2]) > i16::from(pixel[1]) + 20
        })
        .count();
    assert!((31..105).contains(&covered), "a construction preview has visible dash gaps, blue samples={covered}/120");
    control(&mut app, "Apply");
    assert!(session(&app).sketch.entity(3).unwrap().construction);
    reopen(&mut app, id, "construction");
    drop(app);

    let mut sketch = Sketch::new();
    axis(&mut sketch);
    let (mut app, id) = launch(sketch);
    let before = session(&app).sketch.clone();
    ribbon::click_ribbon(&mut app, "Mirror entities");
    control(&mut app, "Two points");
    control(&mut app, "Line 1");
    control(&mut app, "Start dynamic mirror");
    assert_eq!(session(&app).dynamic_mirror, Some(1));
    assert_eq!(session(&app).tool, Tool::Line);
    control(&mut app, "Auto relations");
    pick(&mut app, p(5.0, -8.0), false);
    hover(&mut app, p(12.0, -8.0));
    let preview = &session(&app).dynamic_preview;
    assert_eq!(preview.len(), 1);
    let EntityKind::Line { a, b } = preview[0].0 else { panic!("reflected line") };
    assert!(a.dist(p(-5.0, -8.0)) < 1e-4 && b.dist(p(-12.0, -8.0)) < 1e-4);
    assert!(!preview[0].1);
    assert_eq!(session(&app).sketch, before);
    proof(&mut app, "dynamic-line-preview");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).dynamic_mirror, None);
    assert_eq!(session(&app).sketch, before);
    ribbon::click_ribbon(&mut app, "Mirror entities");
    control(&mut app, "Start dynamic mirror");
    pick(&mut app, p(5.0, -8.0), false);
    pick(&mut app, p(12.0, -8.0), false);
    assert_eq!(session(&app).sketch.drawn().count(), 3);
    let line = session(&app).sketch.clone();
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    redo(&mut app);
    assert_eq!(session(&app).sketch, line);
    ribbon::click_ribbon(&mut app, "Circle");
    assert_eq!(session(&app).dynamic_mirror, Some(1));
    pick(&mut app, p(8.0, 4.0), false);
    hover(&mut app, p(11.0, 4.0));
    let preview = session(&app).dynamic_preview.clone();
    assert_eq!(preview.len(), 1);
    proof(&mut app, "dynamic-circle-preview");
    pick(&mut app, p(11.0, 4.0), false);
    assert_eq!(session(&app).sketch.drawn().count(), 5);
    assert_eq!(session(&app).sketch.drawn().last().unwrap().kind, preview[0].0);
    undo(&mut app);
    assert_eq!(session(&app).sketch, line);
    redo(&mut app);
    ribbon::click_ribbon(&mut app, "Arc");
    pick(&mut app, p(8.0, 10.0), false);
    pick(&mut app, p(11.0, 10.0), false);
    hover(&mut app, p(8.0, 13.0));
    let preview = session(&app).dynamic_preview.clone();
    assert_eq!(preview.len(), 1);
    pick(&mut app, p(8.0, 13.0), false);
    assert_eq!(session(&app).sketch.drawn().last().unwrap().kind, preview[0].0);
    ribbon::click_ribbon(&mut app, "Rectangle");
    pick(&mut app, p(16.0, -2.0), false);
    hover(&mut app, p(22.0, 4.0));
    assert_eq!(session(&app).dynamic_preview.len(), 4);
    pick(&mut app, p(22.0, 4.0), false);
    assert_eq!(session(&app).sketch.drawn().count(), 15);
    proof(&mut app, "dynamic-pairs-applied");
    ribbon::click_ribbon(&mut app, "Point");
    hover(&mut app, p(15.0, -15.0));
    assert_eq!(session(&app).dynamic_preview.len(), 1);
    pick(&mut app, p(15.0, -15.0), false);
    assert_eq!(session(&app).sketch.drawn().count(), 17);
    let before_slot = session(&app).sketch.clone();
    ribbon::click_ribbon(&mut app, "Slot");
    pick(&mut app, p(6.0, -16.0), false);
    pick(&mut app, p(12.0, -16.0), false);
    hover(&mut app, p(6.0, -14.0));
    assert_eq!(session(&app).dynamic_preview.len(), 5);
    pick(&mut app, p(6.0, -14.0), false);
    assert_eq!(session(&app).sketch.drawn().count(), 27);
    let slot = session(&app).sketch.clone();
    undo(&mut app);
    assert_eq!(session(&app).sketch, before_slot);
    redo(&mut app);
    assert_eq!(session(&app).sketch, slot);
    ribbon::click_ribbon(&mut app, "Polygon");
    control(&mut app, "Draw construction");
    pick(&mut app, p(20.0, -12.0), false);
    hover(&mut app, p(23.0, -12.0));
    assert_eq!(session(&app).dynamic_preview.len(), 6);
    assert!(session(&app).dynamic_preview.iter().all(|(_, construction)| *construction));
    pick(&mut app, p(23.0, -12.0), false);
    assert_eq!(session(&app).sketch.drawn().count(), 39);
    assert!(session(&app).sketch.entities.iter().rev().take(12).all(|e| e.construction));
    let polygon = session(&app).sketch.clone();
    undo(&mut app);
    assert_eq!(session(&app).sketch, slot);
    redo(&mut app);
    assert_eq!(session(&app).sketch, polygon);
    proof(&mut app, "dynamic-slot-polygon-applied");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).dynamic_mirror, None);
    assert!(session(&app).dynamic_preview.is_empty());
    let expected = reopen(&mut app, id, "dynamic");
    assert!(expected.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Symmetric { axis: 1, .. })));
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
