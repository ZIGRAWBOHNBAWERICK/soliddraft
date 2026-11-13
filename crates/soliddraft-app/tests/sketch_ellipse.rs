//! Pointer creation, preview, cancellation, history and reopen for ellipse tools.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Pos2};
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{EntityKind, P2},
    sketcher::Frame,
    state::{AppAction, SketchSession, Tool},
};

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}
fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap()
}
fn pick(app: &mut App, p: P2) {
    let at = screen(app, p);
    ribbon::click_at(app, at);
}

#[test]
fn ellipse_tools_create_cancel_undo_and_reopen_through_the_pointer() {
    let dir = std::env::var_os("SOLIDDRAFT_ELLIPSE_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-ellipse-{}", std::process::id())));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(15);
    ribbon::switch_workspace(&mut app, "sketch");
    ribbon::run_command(&mut app, "sketch_ellipse");
    assert_eq!(session(&app).tool, Tool::Ellipse);
    let before = session(&app).sketch.clone();
    pick(&mut app, P2::ZERO);
    pick(&mut app, P2::new(30.0, 0.0));
    assert_eq!(session(&app).pending.len(), 2);
    assert_eq!(session(&app).sketch, before);
    app.key_press(Key::Escape);
    app.run_steps(4);
    assert!(session(&app).pending.is_empty());
    assert_eq!(session(&app).sketch, before);

    ribbon::run_command(&mut app, "sketch_ellipse");
    for p in [P2::ZERO, P2::new(30.0, 0.0), P2::new(0.0, -12.0)] {
        pick(&mut app, p);
    }
    assert_eq!(session(&app).sketch.drawn().count(), 1);
    assert!(matches!(session(&app).sketch.drawn().next().unwrap().kind, EntityKind::Ellipse { .. }));
    assert!(session(&app).sketch.drawn().next().unwrap().point(2).unwrap().v < 0.0);
    let saved = session(&app).sketch.clone();
    app.state_mut().state.push(AppAction::Undo);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    app.state_mut().state.push(AppAction::Redo);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, saved);

    app.key_press(Key::D);
    app.run_steps(4);
    assert_eq!(session(&app).tool, Tool::Dimension);
    for label in [P2::new(-45.0, 0.0), P2::new(0.0, 25.0)] {
        pick(&mut app, P2::new(-30.0, 0.0));
        pick(&mut app, label);
    }
    assert_eq!(session(&app).sketch.constraints.iter().filter(|c| c.is_dimension()).count(), 2);
    assert_eq!(session(&app).sketch.status, soliddraft_app::sketch::SolveStatus::FullyDefined);
    assert!(session(&app).sketch.drawn().next().unwrap().point(2).unwrap().v < 0.0);

    ribbon::run_command(&mut app, "sketch_partial_ellipse");
    for p in [P2::new(60.0, 0.0), P2::new(80.0, 0.0), P2::new(60.0, 10.0), P2::new(80.0, 0.0), P2::new(40.0, 0.0)] {
        pick(&mut app, p);
    }
    assert_eq!(session(&app).sketch.drawn().count(), 2);
    assert!(session(&app).sketch.drawn().any(|e| matches!(e.kind, EntityKind::EllipticArc { .. })));
    let saved = session(&app).sketch.clone();
    let path = dir.join("ellipses.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(12);
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, saved);
    soliddraft_app::file::set_config_dir(None);
}
