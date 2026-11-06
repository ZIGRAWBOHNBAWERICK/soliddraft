//! Reference dimensions through native sketch input and the existing property panel.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2, pos2, vec2};
use ribbon::App;
use soliddraft_app::{
    document::{Command, FeatureId, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, PointRef, Sketch, SolveStatus, solve},
    sketcher::Frame,
    state::{AppAction, SketchSel, SketchSession},
};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn dir() -> std::path::PathBuf {
    std::env::var_os("SOLIDDRAFT_DRIVEN_PROOF_DIR")
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
    for direction in [-1.0, 1.0] {
        for _ in 0..12 {
            let left = app.state().state.viewport.rect.left();
            if let Some(p) = ribbon::painted(app)
                .into_iter()
                .find(|p| p.text == label && p.rect.right() < left && p.rect.contains_rect(p.full))
            {
                ribbon::click_at(app, p.rect.center());
                return;
            }
            let head = ribbon::painted(app).into_iter().find(|p| p.text == "Properties").unwrap().rect;
            app.hover_at(pos2(left - 15.0, (head.bottom() + ribbon::band(app).top()) / 2.0));
            app.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, direction * 80.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            });
            app.run_steps(10);
        }
    }
    panic!("the sketch must expose {label} in its diagnostics workflow");
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap()
}

fn frame(app: &mut App, name: &str) {
    app.render().unwrap().save(dir().join(format!("{name}.png"))).unwrap();
}

fn circle() -> (Sketch, u32) {
    let mut sketch = Sketch::new();
    let entity = sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 5.0 }, false);
    sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(entity, 0), b: PointRef::new(0, 0) });
    let dim =
        sketch.constrain(ConstraintKind::Radius { entity, value: 10.0, diameter: true, label: P2::new(9.0, 7.0) });
    (sketch, dim)
}

#[test]
fn perpendicular_defaults_use_supporting_lines_without_moving_geometry() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let p = P2::new;
    for reference in [false, true] {
        for (name, parallel, reverse, swap, point) in [
            ("point-after", false, false, false, p(20.0, 5.0)),
            ("point-before", false, false, true, p(-20.0, 5.0)),
            ("point-interior", false, false, false, p(5.0, 5.0)),
            ("parallel-a-b", true, false, false, p(20.0, 5.0)),
            ("parallel-b-a", true, false, true, p(20.0, 5.0)),
            ("parallel-reversed", true, true, false, p(20.0, 5.0)),
        ] {
            let mut sketch = Sketch::new();
            sketch.add(EntityKind::Line { a: p(0.0, 0.0), b: p(10.0, 0.0) }, false);
            if parallel {
                let (a, b) = if reverse { (p(30.0, 5.0), p(20.0, 5.0)) } else { (p(20.0, 5.0), p(30.0, 5.0)) };
                sketch.add(EntityKind::Line { a, b }, false);
            } else {
                sketch.add(EntityKind::Point { p: point }, false);
            }
            let (mut app, _) = launch(sketch);
            let before = session(&app).sketch.entities.clone();
            app.key_press(Key::D);
            app.run_steps(6);
            if reference {
                control(&mut app, "Reference dimensions");
            }
            let a = if parallel { p(25.0, 5.0) } else { point };
            let b = p(5.0, 0.0);
            let picks = if swap { [b, a] } else { [a, b] };
            for at in picks {
                let at = screen(&app, at);
                ribbon::click_at(&mut app, at);
            }
            let label = app.state().state.viewport.rect.center() + vec2(0.0, 80.0);
            ribbon::click_at(&mut app, label);
            let dimension = session(&app)
                .sketch
                .constraints
                .iter()
                .find(|c| matches!(c.geometry_kind(), ConstraintKind::PointLineDistance { .. }))
                .copied()
                .unwrap_or_else(|| panic!("{name}, reference {reference}: perpendicular dimension placed"));
            frame(&mut app, &format!("perpendicular-{name}-{}", if reference { "reference" } else { "driving" }));
            assert_eq!(
                dimension.display_value(&session(&app).sketch),
                Some(5.0),
                "{name}: placement must measure the supporting line"
            );
            assert_eq!(dimension.is_reference(), reference);
            assert_eq!(
                session(&app).sketch.entities,
                before,
                "{name}: placing a measured value must not move geometry"
            );
            drop(app);
            soliddraft_app::file::set_config_dir(None);
        }
    }
}

#[test]
fn reference_creation_is_read_only_updates_with_driving_edits_and_survives_reopen() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (sketch, driving) = circle();
    let (mut app, id) = launch(sketch);
    let entities = session(&app).sketch.entities.clone();
    app.key_press(Key::D);
    app.run_steps(6);
    control(&mut app, "Reference dimensions");
    for p in [P2::new(-5.0, 0.0), P2::new(-9.0, -7.0)] {
        let at = screen(&app, p);
        ribbon::click_at(&mut app, at);
    }
    let reference =
        *session(&app).sketch.constraints.iter().find(|c| c.is_reference()).expect("reference annotation created");
    assert!(session(&app).dim_edit.is_none());
    assert_eq!(session(&app).sketch.status, SolveStatus::FullyDefined);
    assert_eq!(session(&app).sketch.entities, entities);
    app.key_press(Key::Escape);
    app.run_steps(8);
    let at = screen(&app, reference.label().unwrap());
    app.run_steps(30);
    ribbon::double_click_at(&mut app, at);
    assert!(session(&app).dim_edit.is_none(), "a reference label must not open a numeric editor");
    assert!(session(&app).message.as_ref().is_some_and(|m| m.0.contains("read-only")));
    app.run_steps(30);
    let at = screen(&app, session(&app).sketch.constraint(driving).unwrap().label().unwrap());
    ribbon::double_click_at(&mut app, at);
    frame(&mut app, "reference-driving-editor");
    assert!(
        session(&app).dim_edit.is_some(),
        "tool {:?}, hover {:?}, selected {:?}, click {at:?}",
        session(&app).tool,
        session(&app).hover,
        session(&app).selection
    );
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text("20".into()));
    app.key_press(Key::Enter);
    app.run_steps(12);
    assert!(session(&app).dim_edit.is_none());
    assert_eq!(session(&app).sketch.constraint(reference.id).unwrap().display_value(&session(&app).sketch), Some(20.0));
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "(Ø20)"));
    app.set_size(vec2(960.0, 600.0));
    soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
    app.run_steps(12);
    frame(&mut app, "reference-created-small-dark");
    let before = session(&app).sketch.clone();
    app.state_mut().state.push(AppAction::ExitSketch(None));
    app.run_steps(6);
    let path = dir().join("reference.dwprt");
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(60);
    assert_eq!(session(&app).sketch, before);
    assert!(!session(&app).reference_dimensions, "the creation choice is session-local");
    let at = screen(&app, reference.label().unwrap());
    ribbon::click_at(&mut app, at);
    assert_eq!(
        session(&app).selection,
        [SketchSel::Constraint(reference.id)],
        "the reopened reference label is the delete target"
    );
    control(&mut app, "Reference dimension");
    control(&mut app, "Reference dimension");
    assert_eq!(session(&app).sketch, before);
    frame(&mut app, "reference-reopened");
    app.key_press(Key::Delete);
    app.run_steps(10);
    assert!(session(&app).sketch.constraint(reference.id).is_none());
    assert_eq!(session(&app).sketch.entities, before.entities);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(10);
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn reference_label_drag_moves_only_the_annotation_and_undo_restores_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (sketch, dim) = circle();
    let (mut app, _) = launch(sketch);
    let at = screen(&app, session(&app).sketch.constraint(dim).unwrap().label().unwrap());
    ribbon::click_at(&mut app, at);
    control(&mut app, "Reference dimension");
    let before = session(&app).sketch.clone();
    let from = screen(&app, before.constraint(dim).unwrap().label().unwrap());
    let to = from + vec2(-30.0, 25.0);
    app.hover_at(from);
    app.run_steps(4);
    app.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.step();
    for t in [0.25, 0.5, 0.75, 1.0] {
        app.hover_at(from + (to - from) * t);
        app.run_steps(3);
    }
    app.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(10);
    assert_eq!(session(&app).sketch.entities, before.entities);
    let label = session(&app).sketch.constraint(dim).unwrap().label().unwrap();
    assert_ne!(label, before.constraint(dim).unwrap().label().unwrap());
    assert!(screen(&app, label).distance(to) < 2.0);
    frame(&mut app, "reference-label-moved");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn reference_values_follow_a_live_point_drag_without_becoming_equations() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let line = sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
    let id = sketch.constrain(
        ConstraintKind::Distance {
            a: PointRef::new(line, 0),
            b: PointRef::new(line, 1),
            kind: soliddraft_app::sketch::DistanceKind::Aligned,
            value: 999.0,
            label: P2::new(5.0, -5.0),
        }
        .as_reference()
        .unwrap(),
    );
    let (mut app, _) = launch(sketch);
    app.run_steps(60);
    let from = screen(&app, P2::new(10.0, 0.0));
    let to = screen(&app, P2::new(14.0, 3.0));
    app.hover_at(from);
    app.run_steps(4);
    app.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.step();
    for t in [0.25, 0.5, 0.75, 1.0] {
        app.hover_at(from + (to - from) * t);
        app.run_steps(3);
    }
    let measured = session(&app).sketch.constraint(id).unwrap().display_value(&session(&app).sketch).unwrap();

    assert!(
        (measured - 205.0_f64.sqrt()).abs() < 1e-5,
        "measurement {measured}, points {:?}, drag {:?}, from {from:?}, to {to:?}",
        session(&app).sketch.entities,
        session(&app).drag
    );
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "(14.32)"));
    assert_eq!(session(&app).sketch.constraint(id).unwrap().value(), None);
    frame(&mut app, "reference-live-point-drag");
    app.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(10);
    assert!((session(&app).sketch.point(PointRef::new(line, 1)).unwrap().dist(P2::new(14.0, 3.0))) < 1e-5);
    assert_eq!(session(&app).sketch.status, SolveStatus::UnderDefined);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn reference_mode_releases_only_the_dimension_and_keeps_its_identity() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (sketch, dim) = circle();
    let (mut app, _) = launch(sketch);
    assert_eq!(session(&app).sketch.status, SolveStatus::FullyDefined);
    let before = session(&app).sketch.clone();
    let at = screen(&app, before.constraint(dim).unwrap().label().unwrap());
    ribbon::click_at(&mut app, at);
    assert_eq!(session(&app).selection, [SketchSel::Constraint(dim)]);
    control(&mut app, "Reference dimension");
    assert_eq!(session(&app).sketch.status, SolveStatus::UnderDefined);
    assert_eq!(session(&app).sketch.entities, before.entities);
    assert_eq!(session(&app).sketch.constraint(dim).unwrap().label(), before.constraint(dim).unwrap().label());
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "(Ø10)"));
    frame(&mut app, "reference-converted");
    control(&mut app, "Reference dimension");
    assert_eq!(session(&app).sketch.status, SolveStatus::FullyDefined);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert_eq!(session(&app).sketch.status, SolveStatus::UnderDefined);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
