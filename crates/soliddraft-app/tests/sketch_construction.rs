//! Construction retention and dedicated centreline placement through native input.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2};
use egui_kittest::kittest::{NodeT, Queryable};
use ribbon::App;
use soliddraft_app::{
    commands,
    document::{Command, FeatureId, PlaneKind, SketchPlane},
    sketch::{Align, ConstraintKind, EntityKind, P2, PointRef, Sketch, solve},
    sketcher::Frame,
    state::{AppAction, SketchSession, Tool},
};
use std::{path::PathBuf, sync::Mutex};

static SERIAL: Mutex<()> = Mutex::new(());

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
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

fn launch(sketch: Sketch, name: &str) -> (App, FeatureId, PathBuf) {
    let dir = std::env::var_os("SOLIDDRAFT_CONSTRUCTION_PROOF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-construction-{}", std::process::id())))
        .join(name);
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app_at(doc, ribbon::EXPANDED_WINDOW, 1.0);
    app.state_mut().state.viewport.references.always_show_origin_axes = false;
    assert!(commands::get("sketch_centreline").unwrap().is_enabled(&app.state().state));
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(20);
    (app, id, dir)
}

fn history(app: &mut App, before: &Sketch, after: &Sketch) {
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(&session(app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(&session(app).sketch, after);
}

fn reopen(app: &mut App, id: FeatureId, path: PathBuf, expected: &Sketch) {
    for action in [AppAction::ExitSketch, AppAction::SaveAs(Some(path.clone())), AppAction::Open(Some(path))] {
        app.state_mut().state.push(action);
        app.run_steps(10);
    }
    assert_eq!(&app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, expected);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(20);
    assert_eq!(&session(app).sketch, expected);
    assert!(!session(app).construction, "drawing preference is session-local");
}

/// Counts painted samples across a straight stroke; gaps must survive rasterization.
fn assert_dashed(app: &mut App, a: P2, b: P2, color: egui::Color32) {
    let image = app.render().unwrap();
    let expected = color.to_array();
    let (a, b) = (screen(app, a), screen(app, b));
    let normal = egui::vec2(-(b - a).y, (b - a).x).normalized();
    let count = (0..100)
        .filter(|i| {
            let p = a.lerp(b, *i as f32 / 99.0);
            [-1.0, 0.0, 1.0].into_iter().any(|offset| {
                let at = (p + normal * offset) * app.ctx.pixels_per_point();
                let pixel = image.get_pixel(at.x.round() as u32, at.y.round() as u32).0;
                if color == soliddraft_app::theme::accent() {
                    i16::from(pixel[0]) > i16::from(pixel[1]) + 40 && i16::from(pixel[1]) > i16::from(pixel[2]) + 20
                } else {
                    (0..3).all(|channel| pixel[channel].abs_diff(expected[channel]) <= 75)
                }
            })
        })
        .count();
    assert!((20..85).contains(&count), "construction stroke needs ink and gaps, got {count}/100");
}

#[test]
fn x_keeps_construction_constraints_profiles_history_and_persistence() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 30.0 }, false);
    let inner = sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 10.0 }, false);
    sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(inner, 0), b: PointRef::ORIGIN });
    sketch.constrain(ConstraintKind::Radius {
        entity: inner,
        value: 10.0,
        diameter: false,
        label: P2::new(15.0, 15.0),
    });
    solve::solve(&mut sketch, None, true);
    let (mut app, id, dir) = launch(sketch, "toggle");
    let before = session(&app).sketch.clone();
    assert_eq!(before.regions().unwrap()[0].holes.len(), 1);

    // Empty-selection X is not a sticky drawing preference.
    let revision = app.state().state.doc.revision();
    app.key_press(Key::X);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    assert!(!session(&app).construction);
    assert_eq!(app.state().state.doc.revision(), revision);

    pick(&mut app, P2::new(-10.0, 0.0));
    app.key_press(Key::X);
    app.run_steps(6);
    let after = session(&app).sketch.clone();
    assert!(after.entity(inner).unwrap().construction);
    assert_eq!(after.constraints, before.constraints);
    assert_eq!(after.entity(inner).unwrap().kind, before.entity(inner).unwrap().kind);
    assert!(after.regions().unwrap()[0].holes.is_empty());
    history(&mut app, &before, &after);

    // Construction dimensions remain solver equations, rather than just retained labels.
    let mut driven = after.clone();
    let dimension = driven.constraints.iter_mut().find(|c| c.is_dimension()).unwrap();
    if let ConstraintKind::Radius { value, .. } = &mut dimension.kind {
        *value = 12.0;
    }
    solve::solve(&mut driven, None, true);
    assert!((driven.entity(inner).unwrap().radius().unwrap() - 12.0).abs() < 1e-6);
    assert_eq!(driven.entity(inner).unwrap().point(0), Some(P2::ZERO));
    assert!(driven.regions().unwrap()[0].holes.is_empty());

    app.key_press(Key::X);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before, "X restores a guide to profile geometry");
    app.key_press(Key::X);
    app.run_steps(6);
    reopen(&mut app, id, dir.join("construction.dwprt"), &after);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn centreline_is_a_dashed_constrained_chain_without_changing_ordinary_drawing() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, id, dir) = launch(Sketch::new(), "centreline");
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    ribbon::run_command(&mut app, "sketch_centreline");
    assert_eq!(session(&app).tool, Tool::Centreline);
    assert!(!session(&app).construction);
    assert!((commands::get("sketch_centreline").unwrap().active)(&app.state().state));
    assert!(!(commands::get("sketch_line").unwrap().active)(&app.state().state));
    let checkbox = app.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Draw construction");
    assert!(checkbox.accesskit_node().is_disabled());
    assert_eq!(app.state().state.doc.revision(), revision);

    pick(&mut app, P2::ZERO);
    let end = screen(&app, P2::new(30.0, 0.0));
    app.hover_at(end);
    app.run_steps(5);
    assert_eq!(session(&app).sketch, before);
    app.render().unwrap().save(dir.join("preview.png")).unwrap();
    assert_dashed(&mut app, P2::new(5.0, 0.0), P2::new(25.0, 0.0), soliddraft_app::theme::accent());
    pick(&mut app, P2::new(30.0, 0.0));
    let one = session(&app).sketch.clone();
    assert_eq!(one.drawn().count(), 1);
    assert!(one.entity(1).unwrap().construction);
    assert!(one.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Coincident { b: PointRef::ORIGIN, .. })));
    assert!(one.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Horizontal { target: Align::Line(1) })));
    history(&mut app, &before, &one);

    // Continue the shared line chain, then close onto its existing start.
    ribbon::run_command(&mut app, "sketch_centreline");
    pick(&mut app, P2::new(30.0, 0.0));
    pick(&mut app, P2::new(30.0, 20.0));
    assert!(session(&app).chain_from.is_some());
    let two = session(&app).sketch.clone();
    pick(&mut app, P2::ZERO);
    assert!(session(&app).pending.is_empty());
    let guides = session(&app).sketch.clone();
    assert_eq!(guides.drawn().count(), 3);
    assert!(guides.drawn().all(|e| e.construction));
    assert!(guides.regions().is_err(), "a closed construction loop is not a profile");
    history(&mut app, &two, &guides);

    // Escape and right-click cancel unfinished placement, without editing the guides.
    pick(&mut app, P2::new(-30.0, -20.0));
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).tool, Tool::Select);
    assert!(session(&app).pending.is_empty());
    assert_eq!(session(&app).sketch, guides);
    ribbon::run_command(&mut app, "sketch_centreline");
    pick(&mut app, P2::new(-30.0, -20.0));
    let pos = screen(&app, P2::new(-15.0, -20.0));
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        app.step();
    }
    app.run_steps(6);
    assert_eq!(session(&app).tool, Tool::Centreline, "right-click ends the chain and keeps the tool");
    assert!(session(&app).pending.is_empty());
    assert_eq!(session(&app).sketch, guides);
    app.key_press(Key::Escape);
    app.run_steps(6);
    app.render().unwrap().save(dir.join("guides.png")).unwrap();
    assert_dashed(
        &mut app,
        P2::new(5.0, 0.0),
        P2::new(25.0, 0.0),
        soliddraft_app::theme::edit_suite::CONSTRUCTION,
    );

    // The ordinary Line shortcut and other drawing tools retain their previous mode.
    app.key_press(Key::L);
    app.run_steps(6);
    assert_eq!(session(&app).tool, Tool::Line);
    assert!(!session(&app).construction);
    pick(&mut app, P2::new(-30.0, -20.0));
    pick(&mut app, P2::new(-10.0, -20.0));
    assert!(!session(&app).sketch.entity(4).unwrap().construction);
    app.key_press(Key::Escape);
    app.run_steps(6);

    let checkbox = app.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Draw construction").rect();
    ribbon::click_at(&mut app, checkbox.center());
    assert!(session(&app).construction);
    let revision = app.state().state.doc.revision();
    ribbon::run_command(&mut app, "sketch_centreline");
    app.key_press(Key::C);
    app.run_steps(6);
    assert_eq!(session(&app).tool, Tool::Circle);
    assert!(session(&app).construction, "a pre-existing construction preference survives Centreline");
    assert_eq!(app.state().state.doc.revision(), revision);
    pick(&mut app, P2::new(-35.0, 20.0));
    pick(&mut app, P2::new(-30.0, 20.0));
    assert!(session(&app).sketch.entity(5).unwrap().construction);
    let saved = session(&app).sketch.clone();
    reopen(&mut app, id, dir.join("centrelines.dwprt"), &saved);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
