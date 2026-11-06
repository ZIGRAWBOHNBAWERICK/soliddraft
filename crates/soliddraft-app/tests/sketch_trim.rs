//! Trim-to-closest and power trim through real pointer input, including sparse samples.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());
use ribbon::App;
use soliddraft_app::{
    document::{Command, Document, FeatureId, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, PointRef, Sketch, solve},
    sketcher::Frame,
    state::{AppAction, SketchSession},
};

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}

fn screen(app: &App, point: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(point).unwrap()
}

fn launch(sketch: Sketch) -> (App, FeatureId) {
    let config = std::env::temp_dir().join(format!("soliddraft-trim-{}", std::process::id()));
    std::fs::create_dir_all(&config).unwrap();
    soliddraft_app::file::set_config_dir(Some(config));
    let mut doc = Document::new("Trim regressions", soliddraft_app::kernel());
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    ribbon::click_ribbon(&mut app, "Trim");
    (app, id)
}

fn frame(app: &mut App, name: &str) {
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_TRIM_PROOF_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        app.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
    }
}

fn option(app: &mut App, label: &str) {
    for _ in 0..12 {
        if let Some(p) = ribbon::painted(app).into_iter().find(|p| p.text == label && p.rect.contains_rect(p.full)) {
            ribbon::click_at(app, p.rect.center());
            return;
        }
        let head = ribbon::painted(app).into_iter().find(|p| p.text == "Properties").unwrap().rect;
        app.hover_at(pos2(
            app.state().state.viewport.rect.left() - 15.0,
            (head.bottom() + ribbon::band(app).top()) / 2.0,
        ));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -80.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        app.run_steps(12);
    }
    panic!("Trim must expose the {label} checkbox");
}

#[test]
fn trim_can_keep_removed_pieces_as_non_profile_guides() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let target = sketch.add(EntityKind::Line { a: P2::new(-10.0, 0.0), b: P2::new(10.0, 0.0) }, false);
    for x in [-3.0, 3.0] {
        sketch.add(EntityKind::Line { a: P2::new(x, -6.0), b: P2::new(x, 6.0) }, true);
    }
    let (mut app, id) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    option(&mut app, "Keep trimmed pieces as construction");
    assert!(session(&app).trim_options.keep_as_construction);
    let at = screen(&app, P2::ZERO);
    app.hover_at(at);
    app.run_steps(6);
    frame(&mut app, "keep-construction-preview");
    assert_eq!(session(&app).sketch, before);
    assert_eq!(app.state().state.doc.revision(), revision);
    let samples: Vec<_> = (0..64).map(|i| P2::new(-2.5 + 5.0 * i as f64 / 63.0, 0.0)).collect();
    let count = stroke_samples(&mut app, &samples, soliddraft_app::theme::accent());
    assert!((10..55).contains(&count), "the guide preview needs visible dashes, got {count}/64");
    ribbon::click_at(&mut app, at);
    assert!(session(&app).sketch.entity(target).is_some());
    assert_eq!(session(&app).sketch.drawn().filter(|e| !e.construction).count(), 2);
    assert!(
        session(&app)
            .sketch
            .drawn()
            .any(|e| e.construction && e.kind == EntityKind::Line { a: P2::new(-3.0, 0.0), b: P2::new(3.0, 0.0) })
    );
    frame(&mut app, "keep-construction-after");
    let after = session(&app).sketch.clone();
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, after);
    let path = std::env::temp_dir().join(format!("soliddraft-keep-{}.dwprt", std::process::id()));
    app.state_mut().state.push(AppAction::ExitSketch(None));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, after);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(6);
    assert!(!session(&app).trim_options.keep_as_construction, "options are session-local, guide flags are saved");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

fn stroke_samples(app: &mut App, points: &[P2], color: egui::Color32) -> usize {
    let image = app.render().unwrap();
    let scale = app.ctx.pixels_per_point();
    let expected = color.to_array();
    assert!(points.len() > 2);
    (0..points.len())
        .filter(|&i| {
            let p = screen(app, points[i]);
            assert!(app.state().state.viewport.rect.contains(p));
            let previous = screen(app, points[(i + points.len() - 1) % points.len()]);
            let next = screen(app, points[(i + 1) % points.len()]);
            let tangent = next - previous;
            let normal = vec2(-tangent.y, tangent.x).normalized();
            // Sample across the stroke, not along it, so nearby dashes cannot fill a gap.
            [-1.0, 0.0, 1.0].into_iter().any(|offset| {
                let at = (p + normal * offset) * scale;
                let (x, y) = (at.x.round() as i32, at.y.round() as i32);
                x >= 0 && y >= 0 && x < image.width() as i32 && y < image.height() as i32 && {
                    let pixel = image.get_pixel(x as u32, y as u32).0;
                    (0..3).all(|i| pixel[i].abs_diff(expected[i]) <= 75)
                }
            })
        })
        .count()
}

#[test]
fn small_kept_circles_have_visible_construction_dashes() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let target = sketch.add(EntityKind::Circle { center: P2::new(5.0, 5.0), radius: 2.0 }, false);
    let (mut app, _) = launch(sketch);
    option(&mut app, "Keep trimmed pieces as construction");
    let at = screen(&app, P2::new(7.0, 5.0));
    ribbon::click_at(&mut app, at);
    assert!(session(&app).sketch.entity(target).unwrap().construction);
    let at = screen(&app, P2::new(5.0, 5.0));
    app.hover_at(at);
    app.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, -14.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(12);
    let radius = screen(&app, P2::new(7.0, 5.0)).distance(screen(&app, P2::new(5.0, 5.0)));
    assert!((35.0..75.0).contains(&radius), "prove the zoomed-out fixture, radius {radius}");
    frame(&mut app, "keep-small-circle");
    let points: Vec<_> = (0..100)
        .map(|i| P2::new(5.0, 5.0) + P2::from_angle((i as f64 + 0.25) * std::f64::consts::TAU / 100.0).scale(2.0))
        .collect();
    let count = stroke_samples(&mut app, &points, soliddraft_app::theme::muted());
    assert!((10..85).contains(&count), "a construction circle needs visible strokes and gaps, got {count}/100");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn ignored_guides_still_bound_profile_trimming_and_cannot_be_grabbed() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let guide = sketch.add(EntityKind::Line { a: P2::new(-10.0, 0.0), b: P2::new(10.0, 0.0) }, true);
    sketch.add(EntityKind::Line { a: P2::new(-10.0, 5.0), b: P2::new(10.0, 5.0) }, false);
    for x in [-3.0, 3.0] {
        sketch.add(EntityKind::Line { a: P2::new(x, -6.0), b: P2::new(x, 10.0) }, true);
    }
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    app.set_size(vec2(960.0, 600.0));
    soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
    app.run_steps(12);
    option(&mut app, "Ignore construction targets");
    assert!(session(&app).trim_options.ignore_construction);
    let at = screen(&app, P2::new(-5.0, 0.0));
    ribbon::click_at(&mut app, at);
    assert_eq!(session(&app).sketch, before);
    assert_eq!(app.state().state.doc.revision(), revision);
    press(&mut app, P2::ZERO);
    move_to(&mut app, P2::new(0.0, 8.0));
    assert_eq!(session(&app).sketch.entity(guide), before.entity(guide));
    assert_eq!(session(&app).sketch.drawn().filter(|e| !e.construction).count(), 2);
    release(&mut app, P2::new(0.0, 8.0));
    let after = session(&app).sketch.clone();
    frame(&mut app, "ignore-construction-after");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, after);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn kept_power_trim_restores_whole_entities_and_relations_without_duplicates() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let target = sketch.add(EntityKind::Line { a: P2::new(-10.0, 0.0), b: P2::new(10.0, 0.0) }, false);
    sketch.constrain(ConstraintKind::Fix { point: PointRef::new(target, 0), at: P2::new(-10.0, 0.0) });
    let (mut app, _) = launch(sketch);
    option(&mut app, "Keep trimmed pieces as construction");
    option(&mut app, "Ignore construction targets");
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    press(&mut app, P2::new(0.0, -4.0));
    move_to(&mut app, P2::new(0.0, 4.0));
    assert!(session(&app).sketch.entity(target).unwrap().construction);
    assert_eq!(session(&app).sketch.constraints.len(), before.constraints.len());
    frame(&mut app, "keep-power-live");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    release(&mut app, P2::new(0.0, 4.0));
    assert_eq!(app.state().state.doc.revision(), revision);
    press(&mut app, P2::new(0.0, -4.0));
    move_to(&mut app, P2::new(0.0, 4.0));
    move_to(&mut app, P2::new(0.0, -4.0));
    release(&mut app, P2::new(0.0, -4.0));
    assert_eq!(session(&app).sketch, before, "retracing restores flags, identities and relations exactly");
    assert_eq!(app.state().state.doc.revision(), revision);
    press(&mut app, P2::new(0.0, -4.0));
    move_to(&mut app, P2::new(0.0, 4.0));
    app.key_press(Key::Escape);
    app.run_steps(6);
    release(&mut app, P2::new(0.0, 4.0));
    assert_eq!(session(&app).sketch, before);
    assert_eq!(app.state().state.doc.revision(), revision);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

fn lines() -> Sketch {
    let mut sketch = Sketch::new();
    for (a, b) in [
        (P2::new(-30.0, -10.0), P2::new(30.0, -10.0)),
        (P2::new(-30.0, 10.0), P2::new(30.0, 10.0)),
        (P2::new(-15.0, -25.0), P2::new(-15.0, 25.0)),
        (P2::new(15.0, -25.0), P2::new(15.0, 25.0)),
    ] {
        sketch.add(EntityKind::Line { a, b }, false);
    }
    solve::solve(&mut sketch, None, true);
    sketch
}

fn density_lines() -> Sketch {
    let mut sketch = Sketch::new();
    for y in [-30.0, -10.0, 10.0] {
        sketch.add(EntityKind::Line { a: P2::new(-30.0, y), b: P2::new(30.0, y) }, false);
    }
    for x in [-15.0, 15.0] {
        sketch.add(EntityKind::Line { a: P2::new(x, -50.0), b: P2::new(x, 50.0) }, true);
    }
    solve::solve(&mut sketch, None, true);
    sketch
}

fn press(app: &mut App, at: P2) {
    let at = screen(app, at);
    app.hover_at(at);
    app.run_steps(3);
    app.event(egui::Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.step();
}

fn move_to(app: &mut App, at: P2) {
    let at = screen(app, at);
    app.hover_at(at);
    app.run_steps(4);
}

fn release(app: &mut App, at: P2) {
    let at = screen(app, at);
    app.event(egui::Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(6);
}

#[test]
fn cancelled_power_trim_restores_the_committed_sketch() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let before = lines();
    let (mut app, id) = launch(before.clone());
    let revision = app.state().state.doc.revision();
    press(&mut app, P2::new(0.0, -20.0));
    move_to(&mut app, P2::new(0.0, 20.0));
    assert_eq!(session(&app).sketch.drawn().count(), 6);
    assert_eq!(
        app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch,
        before,
        "the drag is only a preview until release"
    );
    frame(&mut app, "power-live-trail");
    app.key_press(Key::Escape);
    app.run_steps(6);
    release(&mut app, P2::new(0.0, 20.0));
    assert_eq!(session(&app).sketch, before);
    assert!(session(&app).trim_drag.is_none());
    assert_eq!(app.state().state.doc.revision(), revision);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn retracing_and_undo_restore_each_cut_without_undoing_prior_geometry() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let before = lines();
    let (mut app, _) = launch(before.clone());
    let revision = app.state().state.doc.revision();
    press(&mut app, P2::new(0.0, -20.0));
    move_to(&mut app, P2::new(0.0, 20.0));
    assert_eq!(session(&app).trim_drag.as_ref().unwrap().checkpoints.len(), 2);
    move_to(&mut app, P2::new(0.0, -20.0));
    assert_eq!(session(&app).sketch, before, "retracing both red markers restores both cuts");
    assert!(session(&app).trim_drag.as_ref().unwrap().checkpoints.is_empty());
    release(&mut app, P2::new(0.0, -20.0));
    assert_eq!(app.state().state.doc.revision(), revision, "a fully retraced gesture records no edit");
    press(&mut app, P2::new(0.0, -20.0));
    move_to(&mut app, P2::new(0.0, 20.0));
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch.drawn().count(), 5);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(app.state().state.doc.revision(), revision, "Undo during a drag never touches earlier document history");
    release(&mut app, P2::new(0.0, 20.0));
    assert_eq!(session(&app).sketch, before);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn retracing_trim_does_not_depend_on_pointer_sample_density() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let before = density_lines();
    let mut results = Vec::new();
    let paths: &[&[f64]] = &[&[-40.0], &[-20.0, -40.0], &[10.5, 0.0, -9.5, -20.0, -40.0], &[10.0, -10.0, -40.0]];
    for stops in paths {
        let (mut app, _) = launch(before.clone());
        press(&mut app, P2::new(0.0, -20.0));
        move_to(&mut app, P2::new(0.0, 20.0));
        for &y in *stops {
            move_to(&mut app, P2::new(0.0, y));
        }
        release(&mut app, P2::new(0.0, -40.0));
        results.push(session(&app).sketch.clone());
        drop(app);
        soliddraft_app::file::set_config_dir(None);
    }
    assert_eq!(
        results[0].drawn().count(),
        results[1].drawn().count(),
        "sparse and dense reversal must both trim the new crossing after restoring the old cuts"
    );
    assert_eq!(results[0].drawn().count(), 6);
    for result in &results[1..] {
        assert_eq!(&results[0], result);
    }
}

#[test]
fn a_new_crossing_before_an_older_marker_keeps_trim_traversal_order() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut before = density_lines();
    let short = before.add(EntityKind::Line { a: P2::new(4.0, 15.0), b: P2::new(6.0, 15.0) }, false);
    solve::solve(&mut before, None, true);
    let mut results = Vec::new();
    for dense in [false, true] {
        let (mut app, _) = launch(before.clone());
        press(&mut app, P2::new(0.0, -20.0));
        move_to(&mut app, P2::new(0.0, 20.0));
        move_to(&mut app, P2::new(10.0, 20.0));
        if dense {
            move_to(&mut app, P2::new(4.0, 14.0));
        }
        move_to(&mut app, P2::new(-10.0, 0.0));
        release(&mut app, P2::new(-10.0, 0.0));
        assert!(session(&app).sketch.entity(short).is_none(), "the new crossing occurs before the old undo marker");
        results.push(session(&app).sketch.clone());
        drop(app);
        soliddraft_app::file::set_config_dir(None);
    }
    assert_eq!(results[0], results[1]);
}

#[test]
fn removing_a_circle_makes_its_tangent_line_trimmable_in_the_same_pointer_sample() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut before = Sketch::new();
    before.add(EntityKind::Circle { center: P2::new(0.0, 10.0), radius: 10.0 }, false);
    before.add(EntityKind::Line { a: P2::new(-10.0, 0.0), b: P2::new(10.0, 0.0) }, false);
    for x in [-30.0, 30.0] {
        before.add(EntityKind::Line { a: P2::new(x, -20.0), b: P2::new(x, 40.0) }, true);
    }
    solve::solve(&mut before, None, true);
    let mut results = Vec::new();
    for split in [false, true] {
        let (mut app, _) = launch(before.clone());
        let from = P2::new(0.0, 30.0);
        assert!(app.state().state.viewport.rect.contains(screen(&app, from)));
        press(&mut app, from);
        if split {
            move_to(&mut app, P2::new(0.0, 5.0));
        }
        move_to(&mut app, P2::new(0.0, -5.0));
        release(&mut app, P2::new(0.0, -5.0));
        results.push(session(&app).sketch.clone());
        drop(app);
        soliddraft_app::file::set_config_dir(None);
    }
    assert_eq!(
        results[0].drawn().count(),
        results[1].drawn().count(),
        "splitting the stroke must not change which curves trim"
    );
    assert!(results[0].drawn().all(|e| e.construction));
    assert_eq!(results[0], results[1]);
}

#[test]
fn live_trim_undo_works_in_a_reopened_part_without_document_history() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, id) = launch(lines());
    let path = std::env::temp_dir().join(format!("soliddraft-trim-loaded-{}.dwprt", std::process::id()));
    app.state_mut().state.push(AppAction::ExitSketch(None));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    assert!(!app.state().state.doc.can_undo(), "reopening clears document history");
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(10);
    ribbon::click_ribbon(&mut app, "Trim");
    press(&mut app, P2::new(0.0, -20.0));
    move_to(&mut app, P2::new(0.0, 20.0));
    assert_eq!(session(&app).sketch.drawn().count(), 6);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch.drawn().count(), 5, "the Undo command must be enabled for an uncommitted cut");
    app.key_press(Key::Escape);
    app.run_steps(6);
    release(&mut app, P2::new(0.0, 20.0));
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn power_trim_drag_from_a_line_extends_along_its_natural_path() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let target = sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
    sketch.add(EntityKind::Point { p: P2::new(-30.0, -20.0) }, true);
    sketch.add(EntityKind::Point { p: P2::new(40.0, 20.0) }, true);
    solve::solve(&mut sketch, None, true);
    let before = sketch.clone();
    let (mut app, _) = launch(sketch);
    press(&mut app, P2::new(5.0, 0.0));
    move_to(&mut app, P2::new(30.0, 5.0));
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before, "Undo during extension does not remove the original line");
    move_to(&mut app, P2::new(31.0, 5.0));
    move_to(&mut app, P2::new(30.0, 5.0));
    release(&mut app, P2::new(30.0, 5.0));
    let EntityKind::Line { a, b } = session(&app).sketch.entity(target).expect("extension keeps the line").kind else {
        panic!()
    };
    assert!(
        a.dist(P2::ZERO) < 1e-6 && b.dist(P2::new(30.0, 0.0)) < 0.2,
        "the line follows its supporting line: {a:?} {b:?}"
    );
    frame(&mut app, "power-extended-line");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn power_trim_drag_from_an_arc_keeps_its_radius() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let target =
        sketch.add(EntityKind::Arc { center: P2::ZERO, start: P2::new(20.0, 0.0), end: P2::new(0.0, 20.0) }, false);
    sketch.add(EntityKind::Point { p: P2::new(-40.0, -30.0) }, true);
    sketch.add(EntityKind::Point { p: P2::new(40.0, 30.0) }, true);
    solve::solve(&mut sketch, None, true);
    let (mut app, _) = launch(sketch);
    let q = 20.0 / 2.0_f64.sqrt();
    press(&mut app, P2::new(q, q));
    move_to(&mut app, P2::new(-30.0, 0.0));
    release(&mut app, P2::new(-30.0, 0.0));
    let EntityKind::Arc { center, start, end } = session(&app).sketch.entity(target).unwrap().kind else { panic!() };
    assert!(
        center.dist(P2::ZERO) < 1e-6 && start.dist(P2::new(20.0, 0.0)) < 0.2 && end.dist(P2::new(-20.0, 0.0)) < 0.2
    );
    frame(&mut app, "power-extended-arc");
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn trim_circle_to_closest_keeps_the_other_arc() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let circle = sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 20.0 }, false);
    sketch.add(EntityKind::Line { a: P2::new(-30.0, 0.0), b: P2::new(30.0, 0.0) }, true);
    solve::solve(&mut sketch, None, true);
    let (mut app, _) = launch(sketch);
    frame(&mut app, "circle-before");
    let at = screen(&app, P2::new(0.0, 20.0));
    ribbon::click_at(&mut app, at);
    frame(&mut app, "circle-after");
    assert!(
        matches!(session(&app).sketch.entity(circle).unwrap().kind, EntityKind::Arc { .. }),
        "trimming a circle between two crossings must leave an arc"
    );
    assert!(session(&app).sketch.entity(circle).unwrap().midpoint().v < 0.0);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert!(matches!(session(&app).sketch.entity(circle).unwrap().kind, EntityKind::Circle { .. }));
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn power_trim_crosses_entities_between_pointer_samples() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let before = lines();
    let (mut app, id) = launch(before.clone());
    let start = screen(&app, P2::new(0.0, -20.0));
    let end = screen(&app, P2::new(0.0, 20.0));
    frame(&mut app, "power-before");
    app.hover_at(start);
    app.run_steps(3);
    app.event(egui::Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.step();
    app.hover_at(end);
    app.run_steps(4);
    app.event(egui::Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(6);
    frame(&mut app, "power-after");
    assert_eq!(
        session(&app).sketch.drawn().count(),
        6,
        "one sparse drag cuts both middle intervals, leaving four pieces and the two boundaries"
    );
    let after = session(&app).sketch.clone();
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before, "the whole gesture is one undo step");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, after);
    let path = std::env::temp_dir().join(format!("soliddraft-trim-{}.dwprt", std::process::id()));
    app.state_mut().state.push(AppAction::ExitSketch(None));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, after);
    soliddraft_app::file::set_config_dir(None);
}
