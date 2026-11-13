//! Read-only profile diagnostics through the real sketch property panel.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2, Shape, pos2, vec2};
use ribbon::App;
use soliddraft_app::{
    document::{Command, FeatureId, PlaneKind, SketchPlane},
    sketch::{Align, ConstraintKind, EntityKind, P2, Sketch, SolveStatus, solve},
    sketcher::Frame,
    state::{AppAction, SketchSel, SketchSession},
};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn dir() -> std::path::PathBuf {
    std::env::var_os("SOLIDDRAFT_DIAGNOSTICS_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-diagnostics-{}", std::process::id())))
}

fn launch(sketch: Sketch) -> (App, FeatureId) {
    launch_at(ribbon::blank(), sketch, 0.0)
}

fn launch_at(mut doc: soliddraft_app::document::Document, mut sketch: Sketch, offset: f64) -> (App, FeatureId) {
    std::fs::create_dir_all(dir().join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir().join("config")));
    solve::solve(&mut sketch, None, true);
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset } })
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

fn control(app: &mut App, label: &str) {
    ribbon::click_option(app, label);
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap()
}

fn marker(shape: &Shape, at: Pos2) -> bool {
    match shape {
        Shape::Vec(shapes) => shapes.iter().any(|s| marker(s, at)),
        Shape::Circle(c) => {
            c.center.distance(at) < 0.5
                && (c.radius - 8.0).abs() < 0.1
                && c.stroke.color == soliddraft_app::theme::error()
        }
        _ => false,
    }
}

fn assert_marker(app: &App) {
    let issue = soliddraft_app::sketch_diagnostics::selected(session(app)).unwrap();
    let at = screen(app, issue.locations[0]);
    assert!(app.state().state.viewport.rect.contains(at), "the fault must be inside the viewport");
    assert!(
        app.output().shapes.iter().any(|s| s.clip_rect.contains(at) && marker(&s.shape, at)),
        "the real painter marks the selected fault"
    );
}

fn frame(app: &mut App, name: &str) {
    if std::env::var_os("SOLIDDRAFT_DIAGNOSTICS_PROOF_DIR").is_some() {
        app.render().unwrap().save(dir().join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn diagnostics_locate_open_ends_without_editing_or_selecting_geometry() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    for (a, b) in [
        (P2::new(-20.0, 0.0), P2::new(20.0, 0.0)),
        (P2::new(20.0, 0.0), P2::new(20.0, 20.0)),
        (P2::new(20.0, 20.0), P2::new(-20.0, 20.0)),
    ] {
        sketch.add(EntityKind::Line { a, b }, false);
    }
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    control(&mut app, "Sketch diagnostics");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Open end"));
    assert_eq!(session(&app).sketch, before);
    assert_eq!(app.state().state.doc.revision(), revision);
    assert!(session(&app).selection.is_empty());
    assert_marker(&app);
    frame(&mut app, "diagnostics-open-end");
    let first = session(&app).diagnostics.as_ref().unwrap().current;
    control(&mut app, "Next fault");
    assert_ne!(session(&app).diagnostics.as_ref().unwrap().current, first);
    assert_marker(&app);
    control(&mut app, "Locate fault");
    app.run_steps(90);
    let issue = soliddraft_app::sketch_diagnostics::selected(session(&app)).unwrap();
    let at = screen(&app, issue.locations[0]);
    assert!(at.distance(app.state().state.viewport.rect.center()) < 3.0, "Locate centres the actual fault");
    assert_marker(&app);
    assert_eq!(app.state().state.doc.revision(), revision);
    frame(&mut app, "diagnostics-located");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert!(soliddraft_app::sketch_diagnostics::selected(session(&app)).is_none());
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Sketch changed. Refresh diagnostics."));
    let changed_revision = app.state().state.doc.revision();
    control(&mut app, "Refresh diagnostics");
    assert!(session(&app).diagnostics.as_ref().unwrap().issues.is_empty());
    assert_eq!(app.state().state.doc.revision(), changed_revision);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    control(&mut app, "Refresh diagnostics");
    assert_eq!(session(&app).diagnostics.as_ref().unwrap().issues.len(), 2);
    let camera = app.state().state.viewport.camera.clone();
    let report = app.state_mut().state.sketch.as_mut().unwrap().diagnostics.as_mut().unwrap();
    report.issues[report.current].locations = vec![P2::new(f64::MAX, f64::MAX)];
    app.state_mut().state.push(AppAction::LocateSketchDiagnostic);
    app.run_steps(6);
    assert_eq!(app.state().state.viewport.camera.target, camera.target);
    assert_eq!(app.state().state.viewport.camera.distance, camera.distance);
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert!(app.state().state.sketch.is_some(), "Escape closes diagnostics before leaving the sketch");
    assert!(session(&app).diagnostics.is_none());
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn overlap_diagnostics_are_read_only_and_reports_do_not_persist_after_reopen() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
    sketch.add(EntityKind::Line { a: P2::new(10.0, 0.0), b: P2::ZERO }, false);
    let (mut app, id) = launch(sketch);
    app.set_size(vec2(960.0, 600.0));
    soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
    app.run_steps(12);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    control(&mut app, "Sketch diagnostics");
    assert_eq!(session(&app).diagnostics.as_ref().unwrap().issues.len(), 2);
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Overlapping curves"));
    assert_marker(&app);
    frame(&mut app, "diagnostics-overlap-small-dark");
    control(&mut app, "Next fault");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Invalid or degenerate curve"));
    control(&mut app, "Previous fault");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Overlapping curves"));
    assert_eq!(session(&app).sketch, before);
    assert_eq!(app.state().state.doc.revision(), revision);
    control(&mut app, "Close diagnostics");
    assert!(session(&app).diagnostics.is_none());
    let path = dir().join("diagnostics.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(10);
    assert!(session(&app).diagnostics.is_none());
    assert_eq!(session(&app).sketch, before);
    control(&mut app, "Sketch diagnostics");
    assert_eq!(session(&app).diagnostics.as_ref().unwrap().issues.len(), 2);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

fn owner_halos(shape: &Shape) -> usize {
    match shape {
        Shape::Vec(shapes) => shapes.iter().map(owner_halos).sum(),
        Shape::Path(path) if (path.stroke.width - 6.0).abs() < 0.1 => 1,
        _ => 0,
    }
}

#[test]
fn branching_junctions_highlight_all_owners_without_making_delete_destructive() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    for end in [P2::new(-10.0, 0.0), P2::new(10.0, 0.0), P2::new(0.0, 10.0)] {
        sketch.add(EntityKind::Line { a: P2::ZERO, b: end }, false);
    }
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    control(&mut app, "Sketch diagnostics");
    for _ in 0..3 {
        control(&mut app, "Next fault");
    }
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Branching junction"));
    assert_eq!(soliddraft_app::sketch_diagnostics::selected(session(&app)).unwrap().entities.len(), 3);
    assert_eq!(app.output().shapes.iter().map(|s| owner_halos(&s.shape)).sum::<usize>(), 3);
    assert_marker(&app);
    frame(&mut app, "diagnostics-branch");
    app.key_press(Key::Delete);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    assert_eq!(app.state().state.doc.revision(), revision);
    assert!(session(&app).selection.is_empty());
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn wrong_kind_relations_are_reported_after_reopen_without_silently_repairing_them() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let circle = sketch.add(EntityKind::Circle { center: P2::new(10.0, 10.0), radius: 10.0 }, false);
    let line = sketch.add(EntityKind::Line { a: P2::new(-5.0, 10.0), b: P2::new(25.0, 10.0) }, true);
    let horizontal = sketch.constrain(ConstraintKind::Horizontal { target: Align::Line(circle) });
    let radius = sketch.constrain(ConstraintKind::Radius {
        entity: line,
        value: 3.0,
        diameter: false,
        label: P2::new(0.0, 15.0),
    });
    let (mut app, id) = launch(sketch);
    let authored = session(&app).sketch.clone();
    let path = dir().join("wrong-kind.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(10);
    assert_eq!(session(&app).sketch, authored);
    assert!(!matches!(session(&app).sketch.status, SolveStatus::OverDefined { .. }));
    control(&mut app, "Selected");
    control(&mut app, "Dangling");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "No dangling references."));
    control(&mut app, "Dangling");
    control(&mut app, "Conflicting");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "No conflicting relations."));
    let revision = app.state().state.doc.revision();
    control(&mut app, "Sketch diagnostics");
    frame(&mut app, "diagnostics-wrong-kind");
    assert!(
        ribbon::painted(&app).iter().any(|p| p.text == "Invalid relation target"),
        "existing wrong-kind targets must not look like satisfied relations"
    );
    assert_eq!(session(&app).sketch, authored);
    assert_eq!(app.state().state.doc.revision(), revision);
    assert_eq!(session(&app).sketch.constraints.iter().map(|c| c.id).collect::<Vec<_>>(), [horizontal, radius]);
    assert_eq!(soliddraft_app::sketch_diagnostics::selected(session(&app)).unwrap().relation, Some(horizontal));
    frame(&mut app, "diagnostics-invalid-relation");
    control(&mut app, "Inspect relation");
    assert_eq!(session(&app).selection, [SketchSel::Constraint(horizontal)]);
    app.key_press(Key::Delete);
    app.run_steps(8);
    assert!(session(&app).sketch.constraint(horizontal).is_none());
    assert!(session(&app).sketch.constraint(radius).is_some());
    assert_eq!(session(&app).sketch.entities, authored.entities);
    let after = session(&app).sketch.clone();
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, authored);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, after);
    control(&mut app, "Refresh diagnostics");
    assert_eq!(soliddraft_app::sketch_diagnostics::selected(session(&app)).unwrap().relation, Some(radius));
    control(&mut app, "Inspect relation");
    assert_eq!(session(&app).selection, [SketchSel::Constraint(radius)]);
    app.key_press(Key::Delete);
    app.run_steps(8);
    assert!(session(&app).sketch.constraints.is_empty());
    assert_eq!(session(&app).sketch.entities, authored.entities);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn intended_extrusion_checks_do_not_create_features_or_change_geometry() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Circle { center: P2::new(10.0, 10.0), radius: 10.0 }, false);
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    let features = app.state().state.doc.features.len();
    control(&mut app, "Sketch diagnostics");
    control(&mut app, "Diagnostics only");
    control(&mut app, "Boss extrude");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Boss extrude: 1 closed region"));
    frame(&mut app, "preflight-boss");
    control(&mut app, "Boss extrude");
    control(&mut app, "Cut extrude");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Cut extrude needs a part body."));
    assert_eq!(session(&app).sketch, before);
    assert_eq!(app.state().state.doc.revision(), revision);
    assert_eq!(app.state().state.doc.features.len(), features);
    assert!(session(&app).selection.is_empty());
    app.set_size(vec2(960.0, 600.0));
    soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
    app.run_steps(12);
    assert!(
        ribbon::painted(&app)
            .iter()
            .any(|p| p.text == "Cut extrude needs a part body." && p.rect.contains_rect(p.full))
    );
    frame(&mut app, "preflight-missing-body-small-dark");
    control(&mut app, "Close diagnostics");
    app.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    app.run_steps(3);
    app.event(egui::Event::Text("Check sketch for feature".into()));
    app.run_steps(3);
    app.key_press(Key::Enter);
    app.run_steps(12);
    assert!(session(&app).diagnostics.is_some());
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Boss extrude: 1 closed region"));
    app.key_press(Key::Escape);
    app.run_steps(8);
    assert!(app.state().state.sketch.is_some());
    assert!(session(&app).diagnostics.is_none());
    assert_eq!(app.state().state.doc.revision(), revision);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn feature_checks_refresh_after_native_edits_undo_redo_and_reset_after_reopen() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
    sketch.add(EntityKind::Line { a: P2::new(0.0, 10.0), b: P2::ZERO }, false);
    let (mut app, id) = launch(sketch);
    control(&mut app, "Sketch diagnostics");
    control(&mut app, "Diagnostics only");
    control(&mut app, "Boss extrude");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Boss extrude needs a closed region."));
    let open = session(&app).sketch.clone();
    frame(&mut app, "preflight-open");
    app.key_press(Key::L);
    app.run_steps(6);
    for p in [P2::new(10.0, 0.0), P2::new(0.0, 10.0)] {
        let at = screen(&app, p);
        // Sketch endpoints use pointer events, not the solid viewport's GPU ID buffer.
        ribbon::click_at(&mut app, at);
    }
    app.key_press(Key::Escape);
    app.run_steps(8);
    assert!(session(&app).diagnostics.is_some());
    assert!(soliddraft_app::sketch_diagnostics::selected(session(&app)).is_none());
    control(&mut app, "Refresh diagnostics");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Boss extrude: 1 closed region"));
    let closed = session(&app).sketch.clone();
    assert_eq!(closed.drawn().count(), 3, "the native clicks add the missing edge");
    frame(&mut app, "preflight-closed");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, open, "undo removes only the closing edge and its snaps");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Sketch changed. Refresh diagnostics."));
    control(&mut app, "Refresh diagnostics");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Boss extrude needs a closed region."));
    frame(&mut app, "preflight-no-region");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    control(&mut app, "Refresh diagnostics");
    assert_eq!(session(&app).sketch, closed);
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Boss extrude: 1 closed region"));
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    let path = dir().join("preflight.dwprt");
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(12);
    assert!(session(&app).diagnostics.is_none());
    control(&mut app, "Sketch diagnostics");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Diagnostics only"));
    assert_eq!(session(&app).sketch, closed);
    frame(&mut app, "preflight-reopened");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn a_valid_region_keeps_unused_curve_warnings_in_feature_check_mode() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Circle { center: P2::new(10.0, 10.0), radius: 5.0 }, false);
    sketch.add(EntityKind::Circle { center: P2::new(10.0, 10.0), radius: 2.0 }, false);
    sketch.add(EntityKind::Line { a: P2::new(30.0, 10.0), b: P2::new(40.0, 10.0) }, false);
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    control(&mut app, "Sketch diagnostics");
    control(&mut app, "Diagnostics only");
    control(&mut app, "Boss extrude");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Boss extrude: 1 closed region"));
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Review 2 sketch warnings."));
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "1 inner loop"));
    assert_marker(&app);
    assert_eq!(session(&app).sketch, before);
    frame(&mut app, "preflight-unused-curves");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn checked_cut_profiles_still_use_the_existing_editor_and_catallax_path() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Circle { center: P2::new(10.0, 10.0), radius: 2.0 }, false);
    let (mut app, _) = launch_at(ribbon::plate(), sketch, 20.0);
    let revision = app.state().state.doc.revision();
    let features = app.state().state.doc.features.len();
    control(&mut app, "Sketch diagnostics");
    control(&mut app, "Diagnostics only");
    control(&mut app, "Cut extrude");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Cut extrude: 1 closed region"));
    assert_eq!(app.state().state.doc.revision(), revision);
    assert_eq!(app.state().state.doc.features.len(), features);
    frame(&mut app, "preflight-cut");
    ribbon::run_command(&mut app, "exit_sketch");
    assert!(app.state().state.editor.is_none());
    ribbon::run_command(&mut app, "cut");
    assert!(app.state().state.sketch.is_none());
    assert!(matches!(
        app.state().state.editor.as_ref().map(|e| &e.params),
        Some(soliddraft_app::state::EditorParams::Cut(_))
    ));
    app.key_press(Key::Enter);
    ribbon::until_rendered(&mut app, "the checked profile commits through the existing cut editor", |app| {
        app.state().state.editor.is_none()
            && app.state().state.doc.features.len() == features + 1
            && app.state().state.doc.proof_state().current_proved_model()
    });
    let doc = app.state().state.doc.model();
    let volume = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
    let expected = 48000.0 - 80.0 * std::f64::consts::PI;
    assert!((volume - expected).abs() < 1e-4, "{volume} != {expected}");
    frame(&mut app, "preflight-cut-result");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn off_axis_circular_holes_keep_their_inner_loop_in_feature_checks() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for (name, center, compact) in
        [("off-axis", P2::new(99.6, 6.5), false), ("rotated", P2::new(99.81187304123692, 0.0), true)]
    {
        let mut sketch = Sketch::new();
        sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 100.0 }, false);
        sketch.add(EntityKind::Circle { center, radius: 0.1 }, false);
        let (mut app, _) = launch(sketch);
        if compact {
            app.set_size(vec2(960.0, 600.0));
            soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
            app.run_steps(12);
        }
        let before = session(&app).sketch.clone();
        let revision = app.state().state.doc.revision();
        control(&mut app, "Sketch diagnostics");
        control(&mut app, "Diagnostics only");
        control(&mut app, "Boss extrude");
        frame(&mut app, &format!("nesting-{name}"));
        assert!(session(&app).diagnostics.as_ref().unwrap().issues.is_empty());
        assert!(
            ribbon::painted(&app).iter().any(|p| p.text == "Boss extrude: 1 closed region"),
            "{name}: one containing circle must not become two outer regions"
        );
        assert!(ribbon::painted(&app).iter().any(|p| p.text == "1 inner loop"));
        assert_eq!(session(&app).sketch, before);
        assert_eq!(app.state().state.doc.revision(), revision);
        drop(app);
        soliddraft_app::file::set_config_dir(None);
    }
}

#[test]
fn circles_below_the_profile_tolerance_are_not_reported_as_valid() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 5e-7 }, false);
    let (mut app, _) = launch(sketch);
    control(&mut app, "Sketch diagnostics");
    assert!(
        ribbon::painted(&app).iter().any(|p| p.text == "Invalid or degenerate curve"),
        "a circle rejected by profile extraction must appear in the diagnostic report"
    );
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn valid_profiles_ignore_construction_and_keep_the_existing_feature_workflow() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Circle { center: P2::new(10.0, 10.0), radius: 10.0 }, false);
    sketch.add(EntityKind::Line { a: P2::new(-5.0, 10.0), b: P2::new(25.0, 10.0) }, true);
    let (mut app, _) = launch(sketch);
    let revision = app.state().state.doc.revision();
    assert!(soliddraft_app::commands::get("sketch_diagnostics").unwrap().is_enabled(&app.state().state));
    control(&mut app, "Sketch diagnostics");
    assert!(session(&app).diagnostics.as_ref().unwrap().issues.is_empty());
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "No listed profile defects found."));
    assert_eq!(app.state().state.doc.revision(), revision);
    assert_eq!(session(&app).sketch.regions().unwrap().len(), 1);
    frame(&mut app, "diagnostics-valid-profile");
    control(&mut app, "Close diagnostics");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
