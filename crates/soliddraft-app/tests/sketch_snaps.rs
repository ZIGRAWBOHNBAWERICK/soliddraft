//! Native pointer placement uses the shared analytic snap and retains its relations.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Pos2};
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, Sketch},
    sketcher::Frame,
    state::{AppAction, SketchSession, Snap, SnapFilter, Tool},
};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}
fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    let at =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap();
    assert!(st.viewport.rect.contains(at), "{p:?} must be inside the viewport, got {at:?}");
    at
}
fn hover(app: &mut App, p: P2) {
    app.hover_at(screen(app, p));
    app.run_steps(5);
}
fn pick(app: &mut App, p: P2) {
    let at = screen(app, p);
    ribbon::click_at(app, at);
}
fn text_click(app: &mut App, label: &str) {
    let Some(control) = ribbon::painted(app).into_iter().find(|p| p.text == label && p.rect.contains_rect(p.full))
    else {
        proof(app, "missing-control");
        panic!("visible control {label}");
    };
    let at = control.rect.center();
    ribbon::click_at(app, at);
}
fn quick(app: &mut App, filter: SnapFilter) {
    let label = session(app).quick_snap.map_or("Quick snap", SnapFilter::label);
    text_click(app, label);
    if filter == SnapFilter::None {
        proof(app, "quick-snap-menu");
    }
    text_click(app, filter.label());
    assert_eq!(session(app).quick_snap, Some(filter));
}
fn launch(sketch: Sketch, tool: Tool) -> App {
    let dir = std::env::temp_dir().join(format!("soliddraft-snaps-{}", std::process::id()));
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
    app.state_mut().state.push(AppAction::SketchTool(tool));
    app.run_steps(5);
    app
}
fn proof(app: &mut App, name: &str) {
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_SNAP_PROOF_DIR") {
        let path = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&path).unwrap();
        app.render().unwrap().save(path.join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn intersection_filter_is_one_shot_and_keeps_two_point_on_relations() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = Sketch::new();
    let a = s.add(EntityKind::Line { a: p(-12.0, -8.0), b: p(12.0, 8.0) }, true);
    let b = s.add(EntityKind::Line { a: p(-12.0, 8.0), b: p(12.0, -8.0) }, true);
    let mut app = launch(s, Tool::Point);
    // Explicit quick snaps also work when ordinary inference is off.
    text_click(&mut app, "Auto relations");
    assert!(!session(&app).auto_relations);
    // The default point snap at the origin must not mask an explicit intersection choice.
    quick(&mut app, SnapFilter::Intersection);
    let before = session(&app).sketch.clone();
    hover(&mut app, p(0.08, 0.09));
    proof(&mut app, "intersection-preview");
    assert_eq!(session(&app).snaps, vec![Snap::Intersection(a, b)]);
    assert_eq!(session(&app).sketch, before);
    pick(&mut app, p(0.08, 0.09));
    assert_eq!(session(&app).quick_snap, None);
    let after = session(&app).sketch.clone();
    assert_eq!(after.constraints.iter().filter(|c| matches!(c.kind, ConstraintKind::PointOn { .. })).count(), 2);
    let last = after.drawn().last().unwrap();
    assert!(last.point(0).unwrap().dist(P2::ZERO) < 1e-5);
    app.key_press_modifiers(egui::Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert_eq!(session(&app).sketch.entities, before.entities);
    app.key_press_modifiers(egui::Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    assert_eq!(session(&app).sketch.entities, after.entities);
    let id = session(&app).feature;
    let path = std::env::temp_dir().join(format!("soliddraft-snaps-{}.dwprt", std::process::id()));
    app.state_mut().state.push(AppAction::ExitSketch(None));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path.clone())));
    app.run_steps(10);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    assert_eq!(session(&app).sketch.constraints, after.constraints);
    assert_eq!(session(&app).quick_snap, None);
    std::fs::remove_file(path).unwrap();
    app.state_mut().state.push(AppAction::SketchTool(Tool::Point));
    app.run_steps(4);
    quick(&mut app, SnapFilter::None);
    hover(&mut app, P2::ZERO);
    assert!(session(&app).snaps.is_empty());
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).tool, Tool::Select);
    assert_eq!(session(&app).quick_snap, None);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn line_direction_filters_preview_commit_and_cancel_through_native_input() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = Sketch::new();
    let guide = s.add(EntityKind::Line { a: p(-14.0, -8.0), b: p(-4.0, -3.0) }, true);
    let circle = s.add(EntityKind::Circle { center: p(6.0, -6.0), radius: 3.0 }, true);
    let mut app = launch(s, Tool::Line);
    for (filter, start, end, snap) in [
        (SnapFilter::Parallel, p(-8.0, -1.0), p(2.0, 4.05), Snap::Parallel(guide)),
        (SnapFilter::Perpendicular, p(-5.0, -3.0), p(-8.95, 5.0), Snap::Perpendicular(guide)),
        (SnapFilter::Tangent, p(9.0, -6.0), p(9.05, 4.0), Snap::Tangent(circle)),
    ] {
        app.state_mut().state.push(AppAction::SketchTool(Tool::Line));
        app.run_steps(4);
        quick(&mut app, if filter == SnapFilter::Tangent { SnapFilter::On } else { SnapFilter::None });
        pick(&mut app, start);
        quick(&mut app, filter);
        let before = session(&app).sketch.clone();
        hover(&mut app, end);
        proof(&mut app, filter.label());
        assert_eq!(session(&app).snaps, vec![snap]);
        assert_eq!(session(&app).sketch, before);
        pick(&mut app, end);
        assert_eq!(session(&app).quick_snap, None);
        assert!(session(&app).sketch.constraints.iter().any(|c| match (filter, &c.kind) {
            (SnapFilter::Parallel, ConstraintKind::Parallel { a, .. }) => *a == guide,
            (SnapFilter::Perpendicular, ConstraintKind::Perpendicular { a, .. }) => *a == guide,
            (SnapFilter::Tangent, ConstraintKind::Tangent { a, .. }) => *a == circle,
            _ => false,
        }));
        let committed = session(&app).sketch.clone();
        quick(&mut app, filter);
        app.key_press(Key::Escape);
        app.run_steps(6);
        assert_eq!(session(&app).quick_snap, None);
        assert_eq!(session(&app).sketch, committed);
        app.key_press_modifiers(egui::Modifiers::COMMAND, Key::Z);
        app.run_steps(8);
        assert_eq!(session(&app).sketch.entities, before.entities);
        assert_eq!(session(&app).sketch.constraints, before.constraints);
    }
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
