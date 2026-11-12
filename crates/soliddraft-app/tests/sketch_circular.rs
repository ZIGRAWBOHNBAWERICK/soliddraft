//! Circle and arc placement through native input.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2};
use egui_kittest::kittest::{By, Queryable};
use ribbon::App;
use soliddraft_app::{
    commands,
    document::{Command, FeatureId, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, PointRef, Sketch},
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
    let dir = std::env::var_os("SOLIDDRAFT_CIRCULAR_PROOF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-circular-{}", std::process::id())))
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
    assert!(!commands::get("sketch_centreline").unwrap().is_enabled(&app.state().state));
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

fn assert_geometry_eq(actual: &Sketch, expected: &Sketch) {
    let mut reopened = actual.clone();
    assert_eq!(reopened.entities.len(), expected.entities.len());
    for (actual, saved) in reopened.entities.iter_mut().zip(&expected.entities) {
        assert_eq!(std::mem::discriminant(&actual.kind), std::mem::discriminant(&saved.kind));
        for index in 0..saved.point_count() {
            assert!(actual.point(index).unwrap().dist(saved.point(index).unwrap()) < 1e-8);
        }
        if let (Some(actual), Some(saved)) = (actual.radius(), saved.radius()) {
            assert!((actual - saved).abs() < 1e-8);
        }
        // Only coordinates tolerate solver roundoff; metadata stays exact.
        actual.kind = saved.kind.clone();
    }
    assert_eq!(&reopened, expected);
}

fn reopen(app: &mut App, id: FeatureId, path: PathBuf, expected: &Sketch) {
    for action in [AppAction::ExitSketch, AppAction::SaveAs(Some(path.clone())), AppAction::Open(Some(path))] {
        app.state_mut().state.push(action);
        app.run_steps(10);
    }
    assert_geometry_eq(&app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, expected);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(20);
    assert_geometry_eq(&session(app).sketch, expected);
    assert!(!session(app).construction, "drawing preference is session-local");
}

#[test]
fn centrepoint_arc_can_place_a_major_sweep() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for clockwise in [false, true] {
        let (mut app, id, dir) = launch(Sketch::new(), if clockwise { "major-cw" } else { "major-ccw" });
        disable_inference(&mut app);
        app.key_press(Key::A);
        app.run_steps(5);
        if clockwise {
            app.set_size(egui::vec2(1280.0, 760.0));
            app.run_steps(10);
            let rect = app.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Clockwise arc").rect();
            assert!(rect.bottom() <= app.state().state.viewport.rect.bottom(), "active arc input stays visible");
            ribbon::click_at(&mut app, rect.center());
        }
        assert_eq!(session(&app).arc_clockwise, clockwise);
        let before = session(&app).sketch.clone();
        pick(&mut app, P2::ZERO);
        pick(&mut app, P2::ZERO);
        assert_eq!(session(&app).pending.len(), 1);
        pick(&mut app, P2::new(20.0, 0.0));
        let end = P2::new(0.0, if clockwise { 20.0 } else { -20.0 });
        preview(&mut app, end, P2::new(-20.0, 0.0), &dir.join("preview.png"));
        pick(&mut app, P2::new(20.0, 0.0));
        assert_eq!(session(&app).sketch, before);
        assert_eq!(session(&app).pending.len(), 2);
        pick(&mut app, end);
        let after = session(&app).sketch.clone();
        let e = after.drawn().next().unwrap();
        let EntityKind::Arc { start, end: stored_end, .. } = e.kind else { panic!("arc expected") };
        let (expected_start, expected_end) =
            if clockwise { (end, P2::new(20.0, 0.0)) } else { (P2::new(20.0, 0.0), end) };
        assert!(start.dist(expected_start) < 1e-4, "CCW start was swapped");
        assert!(stored_end.dist(expected_end) < 1e-4);
        history(&mut app, &before, &after);
        pick(&mut app, P2::new(30.0, 0.0));
        pick(&mut app, P2::new(30.0, 10.0));
        app.key_press(Key::Escape);
        app.run_steps(5);
        assert!(session(&app).pending.is_empty());
        assert_eq!(session(&app).sketch, after);
        reopen(&mut app, id, dir.join("major.dwprt"), &after);
        drop(app);
        soliddraft_app::file::set_config_dir(None);
    }
}

fn property_button(app: &App, label: &str) -> egui::Rect {
    let band = ribbon::ribbon_region(app);
    app.query_all(By::new().role(egui::accesskit::Role::Button).label(label))
        .find(|n| n.rect().top() >= band.bottom())
        .unwrap_or_else(|| panic!("property tool {label}"))
        .rect()
}

fn disable_inference(app: &mut App) {
    app.state_mut().state.sketch.as_mut().unwrap().auto_relations = false;
}

fn preview(app: &mut App, at: P2, on_curve: P2, path: &std::path::Path) {
    let before = session(app).sketch.clone();
    app.hover_at(screen(app, at));
    app.run_steps(5);
    assert_eq!(session(app).sketch, before);
    let image = app.render().unwrap();
    image.save(path).unwrap();
    let pixel = screen(app, on_curve) * app.ctx.pixels_per_point();
    let color = soliddraft_app::theme::accent().to_array();
    assert!(
        (-3..=3).any(|x| (-3..=3).any(|y| {
            let p = image.get_pixel((pixel.x.round() as i32 + x) as u32, (pixel.y.round() as i32 + y) as u32).0;
            (0..3).all(|i| p[i].abs_diff(color[i]) < 65)
        })),
        "expected the shared curve preview at {on_curve:?}"
    );
}

#[test]
fn centrepoint_circle_retains_history_cancellation_construction_and_reopen() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, id, dir) = launch(Sketch::new(), "circle");
    disable_inference(&mut app);
    app.key_press(Key::C);
    app.run_steps(5);
    app.state_mut().state.sketch.as_mut().unwrap().construction = true;
    let before = session(&app).sketch.clone();
    pick(&mut app, P2::ZERO);
    preview(&mut app, P2::new(20.0, 0.0), P2::new(-20.0, 0.0), &dir.join("preview.png"));
    pick(&mut app, P2::new(20.0, 0.0));
    let after = session(&app).sketch.clone();
    let e = after.drawn().next().unwrap();
    assert!(e.construction);
    assert!((e.radius().unwrap() - 20.0).abs() < 1e-4);
    history(&mut app, &before, &after);
    pick(&mut app, P2::new(30.0, 10.0));
    app.key_press(Key::Escape);
    app.run_steps(5);
    assert!(session(&app).pending.is_empty());
    assert_eq!(session(&app).sketch, after);
    reopen(&mut app, id, dir.join("circle.dwprt"), &after);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn perimeter_and_three_point_tools_preview_refuse_cancel_and_commit_once() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for (label, tool, picks, on_curve) in [
        (
            "Perimeter circle",
            Tool::PerimeterCircle,
            [P2::new(20.0, 0.0), P2::new(0.0, -20.0), P2::new(-20.0, 0.0)],
            P2::new(0.0, 20.0),
        ),
        (
            "Three-point arc",
            Tool::ThreePointArc,
            [P2::new(20.0, 0.0), P2::new(0.0, -20.0), P2::new(-20.0, 0.0)],
            P2::new(0.0, 20.0),
        ),
    ] {
        let (mut app, id, dir) = launch(Sketch::new(), label);
        disable_inference(&mut app);
        assert!(commands::search(label, &Default::default()).iter().any(|m| m.command.label == label));
        ribbon::click_ribbon(&mut app, label);
        assert_eq!(session(&app).tool, tool);
        let _ = property_button(&app, label);
        let before = session(&app).sketch.clone();
        for count in [1, 2] {
            for p in &picks[..count] {
                pick(&mut app, *p);
            }
            app.key_press(Key::Escape);
            app.run_steps(5);
            assert!(session(&app).pending.is_empty());
            assert_eq!(session(&app).sketch, before);
            // Escape selects the selection tool; the property entry can restart placement.
            let at = property_button(&app, label).center();
            ribbon::click_at(&mut app, at);
            app.run_steps(5);
        }
        pick(&mut app, picks[0]);
        pick(&mut app, picks[1]);
        preview(&mut app, picks[2], on_curve, &dir.join("preview.png"));
        pick(&mut app, picks[1]);
        assert_eq!(session(&app).pending.len(), 2, "invalid last pick stays editable");
        assert_eq!(session(&app).sketch, before);
        pick(&mut app, picks[2]);
        let after = session(&app).sketch.clone();
        assert_eq!(after.drawn().count(), 1);
        let e = after.drawn().next().unwrap();
        assert!((e.radius().unwrap() - 20.0).abs() < 1e-4);
        if let EntityKind::Arc { start, end, .. } = e.kind {
            assert!(start.dist(picks[0]) < 1e-4);
            assert!(end.dist(picks[1]) < 1e-4);
        }
        history(&mut app, &before, &after);
        reopen(&mut app, id, dir.join("round.dwprt"), &after);
        drop(app);
        soliddraft_app::file::set_config_dir(None);
    }
}

#[test]
fn tangent_endpoint_pick_overrides_origin_and_keeps_persistent_relations() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let line = sketch.add(EntityKind::Line { a: P2::new(-20.0, 0.0), b: P2::ZERO }, false);
    let (mut app, id, dir) = launch(sketch, "tangent");
    app.state_mut().state.viewport.camera.zoom(0.25, None, 1.0);
    app.run_steps(5);
    disable_inference(&mut app);
    ribbon::click_ribbon(&mut app, "Tangent arc");
    pick(&mut app, P2::new(-10.0, 0.0));
    assert!(session(&app).pending.is_empty(), "curve interiors are not endpoints");
    pick(&mut app, P2::ZERO);
    assert_eq!(session(&app).pending.len(), 1);
    let before = session(&app).sketch.clone();
    preview(&mut app, P2::new(20.0, 20.0), P2::new(14.1421356, 5.8578644), &dir.join("preview.png"));
    pick(&mut app, P2::new(20.0, 0.0));
    assert_eq!(session(&app).sketch, before, "straight tangent refuses atomically");
    pick(&mut app, P2::new(20.0, 20.0));
    let after = session(&app).sketch.clone();
    assert_eq!(after.drawn().count(), 2);
    assert!(after.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Tangent { a, b:2 } if a==line)));
    assert!(after.constraints.iter().any(
        |c| matches!(c.kind, ConstraintKind::Coincident { a, b } if a==PointRef::new(line,1) && b==PointRef::new(2,1))
    ));
    history(&mut app, &before, &after);
    pick(&mut app, P2::new(20.0, 20.0));
    app.key_press(Key::Escape);
    app.run_steps(5);
    assert_eq!(session(&app).sketch, after);
    assert!(session(&app).pending.is_empty());
    reopen(&mut app, id, dir.join("tangent.dwprt"), &after);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn perimeter_picks_keep_point_on_relations_to_existing_points() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let picks = [P2::new(20.0, 0.0), P2::new(0.0, 20.0), P2::new(-20.0, 0.0)];
    let mut sketch = Sketch::new();
    for p in picks {
        sketch.add(EntityKind::Point { p }, false);
    }
    let (mut app, _, _) = launch(sketch, "snapped-circle");
    ribbon::click_ribbon(&mut app, "Perimeter circle");
    for p in picks {
        pick(&mut app, p);
    }
    let sketch = &session(&app).sketch;
    assert_eq!(sketch.drawn().count(), 4);
    for id in 1..=3 {
        assert!(
            sketch
                .constraints
                .iter()
                .any(|c| matches!(c.kind, ConstraintKind::PointOn { point, entity:4 } if point==PointRef::new(id,0)))
        );
        assert!(sketch.point(PointRef::new(id, 0)).unwrap().dist(picks[(id - 1) as usize]) < 1e-4);
    }
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
