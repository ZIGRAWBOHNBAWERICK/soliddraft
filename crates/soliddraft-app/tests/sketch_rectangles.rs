//! Rectangle placement, persistent relations, refusal, history and reopen.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Pos2};
use ribbon::App;
use soliddraft_app::{
    commands,
    document::{Command, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, PointRef, Sketch, SolveStatus, solve},
    sketcher::Frame,
    state::{AppAction, SketchSession, Tool},
};
use std::{path::PathBuf, sync::Mutex};

static SERIAL: Mutex<()> = Mutex::new(());

fn proof_dir() -> PathBuf {
    std::env::var_os("SOLIDDRAFT_RECTANGLE_PROOF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-rectangles-{}", std::process::id())))
}

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

fn preview_ink(app: &App, image: &image::RgbaImage, p: P2) -> bool {
    let at = screen(app, p) * app.ctx.pixels_per_point();
    (-2..=2).any(|dx| {
        (-2..=2).any(|dy| {
            let x = (at.x.round() as i32 + dx).clamp(0, image.width() as i32 - 1) as u32;
            let y = (at.y.round() as i32 + dy).clamp(0, image.height() as i32 - 1) as u32;
            let [r, g, b, _] = image.get_pixel(x, y).0;
            i16::from(b) - i16::from(r) > 40 && i16::from(b) - i16::from(g) > 20
        })
    })
}

fn start() -> App {
    let config = proof_dir().join("config");
    std::fs::create_dir_all(&config).unwrap();
    soliddraft_app::file::set_config_dir(Some(config));
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(15);
    ribbon::switch_workspace(&mut app, "sketch");
    app
}

#[test]
fn accepted_corner_rectangle_retains_axis_relations_and_refuses_zero_area() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = start();
    ribbon::run_command(&mut app, "sketch_rectangle");
    assert_eq!(session(&app).tool, Tool::Rectangle);
    let before = session(&app).sketch.clone();
    pick(&mut app, P2::new(-30.0, -20.0));
    pick(&mut app, P2::new(-30.0, 20.0));
    assert_eq!(session(&app).sketch, before);
    assert_eq!(session(&app).pending.len(), 1);
    pick(&mut app, P2::new(30.0, 20.0));
    let sketch = &session(&app).sketch;
    assert_eq!(sketch.drawn().count(), 4);
    assert_eq!(sketch.constraints.iter().filter(|c| matches!(c.kind, ConstraintKind::Coincident { .. })).count(), 4);
    assert_eq!(sketch.constraints.iter().filter(|c| matches!(c.kind, ConstraintKind::Horizontal { .. })).count(), 2);
    assert_eq!(sketch.constraints.iter().filter(|c| matches!(c.kind, ConstraintKind::Vertical { .. })).count(), 2);
    assert_eq!(soliddraft_app::sketch::profile::profiles(sketch, session(&app).plane).unwrap().len(), 1);
    let saved = sketch.clone();
    app.state_mut().state.push(AppAction::Undo);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    app.state_mut().state.push(AppAction::Redo);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, saved);
    soliddraft_app::file::set_config_dir(None);
}

fn check_shape(sketch: &Sketch, tool: Tool, area: f64) {
    assert!(matches!(sketch.status, SolveStatus::UnderDefined | SolveStatus::FullyDefined), "{:?}", sketch.status);
    let edges: Vec<_> = sketch.drawn().filter(|e| !e.construction).collect();
    assert_eq!(edges.len(), 4);
    let mut directions = Vec::new();
    for i in 0..4 {
        let EntityKind::Line { a, b } = edges[i].kind else { panic!("a rectangle needs lines") };
        assert!(b.dist(edges[(i + 1) % 4].point(0).unwrap()) < 1e-5);
        directions.push(b - a);
    }
    assert!(directions[0].normalized().cross(directions[2].normalized()).abs() < 1e-6);
    assert!(directions[1].normalized().cross(directions[3].normalized()).abs() < 1e-6);
    if tool != Tool::Parallelogram {
        assert!(directions[0].normalized().dot(directions[1].normalized()).abs() < 1e-6);
    } else {
        assert!(directions[0].normalized().dot(directions[1].normalized()).abs() > 0.1);
    }
    if matches!(tool, Tool::Rectangle | Tool::CentreRectangle) {
        assert!(directions[0].v.abs() < 1e-5);
        assert!(directions[1].u.abs() < 1e-5);
    }
    if matches!(tool, Tool::CentreRectangle | Tool::ThreePointCentreRectangle) {
        let center = sketch.drawn().find(|e| matches!(e.kind, EntityKind::Point { .. })).unwrap();
        assert!(center.construction);
        assert!(edges[0].point(0).unwrap().mid(edges[2].point(0).unwrap()).dist(center.point(0).unwrap()) < 1e-5);
        assert!(center.point(0).unwrap().dist(P2::ZERO) < 1e-5, "the snapped centre remains at the origin");
    }
    assert_eq!(sketch.regions().unwrap().len(), 1);
    if area > 0.0 {
        assert!((sketch.regions().unwrap()[0].area() - area).abs() < 0.05);
    }
}

#[test]
fn all_rectangle_tools_preview_cancel_retain_relations_undo_and_reopen() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let cases = [
        ("sketch_rectangle", Tool::Rectangle, vec![P2::new(-30.0, -20.0), P2::new(30.0, 20.0)], 2400.0),
        ("sketch_centre_rectangle", Tool::CentreRectangle, vec![P2::ZERO, P2::new(30.0, -20.0)], 2400.0),
        (
            "sketch_three_point_corner_rectangle",
            Tool::ThreePointCornerRectangle,
            vec![P2::new(-30.0, -20.0), P2::new(0.0, 20.0), P2::new(24.0, 2.0)],
            1500.0,
        ),
        (
            "sketch_three_point_centre_rectangle",
            Tool::ThreePointCentreRectangle,
            vec![P2::ZERO, P2::new(24.0, 18.0), P2::new(36.0, 2.0)],
            2400.0,
        ),
        (
            "sketch_parallelogram",
            Tool::Parallelogram,
            vec![P2::new(-30.0, -20.0), P2::new(0.0, 20.0), P2::new(35.0, 10.0)],
            1700.0,
        ),
    ];
    for (command, tool, points, area) in cases {
        let mut app = start();
        let id = session(&app).feature;
        let before = session(&app).sketch.clone();
        ribbon::run_command(&mut app, command);
        assert_eq!(session(&app).tool, tool);
        assert!((commands::get(command).unwrap().active)(&app.state().state));
        for p in &points[..points.len() - 1] {
            pick(&mut app, *p);
        }
        let at = screen(&app, *points.last().unwrap());
        app.hover_at(at);
        app.run_steps(4);
        assert_eq!(session(&app).sketch, before);
        let preview = app.render().unwrap();
        preview.save(proof_dir().join(format!("{command}-preview.png"))).unwrap();
        app.key_press(Key::Escape);
        app.run_steps(4);
        assert!(session(&app).pending.is_empty());
        assert_eq!(session(&app).sketch, before);

        ribbon::run_command(&mut app, command);
        for p in &points {
            pick(&mut app, *p);
        }
        assert!(session(&app).pending.is_empty());
        let saved = session(&app).sketch.clone();
        check_shape(&saved, tool, area);
        for edge in saved.drawn().filter(|e| !e.construction) {
            let EntityKind::Line { a, b } = edge.kind else { unreachable!() };
            for t in [0.25, 0.5, 0.75] {
                assert!(preview_ink(&app, &preview, a.lerp(b, t)), "{tool:?}: preview must paint every placed edge");
            }
        }
        app.state_mut().state.push(AppAction::Undo);
        app.run_steps(6);
        assert_eq!(session(&app).sketch, before);
        app.state_mut().state.push(AppAction::Redo);
        app.run_steps(6);
        assert_eq!(session(&app).sketch, saved);

        // Stored equations must preserve the shape after an endpoint edit, rather
        // than describing only the coordinates initially painted.
        let mut edited = saved.clone();
        let edge = edited.drawn().find(|e| !e.construction).unwrap().id;
        let point = PointRef::new(edge, 1);
        let to = edited.point(point).unwrap() + P2::new(5.0, 7.0);
        solve::solve(&mut edited, Some(solve::Drag { point, to }), true);
        assert!(edited.point(point).unwrap().dist(to) < 1e-4);
        check_shape(&edited, tool, 0.0);

        let path = proof_dir().join(format!("{command}.dwprt"));
        for action in [AppAction::ExitSketch, AppAction::SaveAs(Some(path.clone())), AppAction::Open(Some(path))] {
            app.state_mut().state.push(action);
            app.run_steps(10);
        }
        assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, saved);
        app.state_mut().state.push(AppAction::EditSketch(id));
        app.run_steps(15);
        ribbon::run_command(&mut app, command);
        // Construction preference applies to every side and stays outside profiles.
        app.state_mut().state.sketch.as_mut().unwrap().construction = true;
        for p in &points {
            pick(&mut app, *p + P2::new(80.0, 0.0));
        }
        let construction = &session(&app).sketch;
        assert_eq!(construction.drawn().filter(|e| !e.construction).count(), 4);
        assert_eq!(construction.regions().unwrap().len(), 1);
        assert!(construction.drawn().filter(|e| saved.entity(e.id).is_none()).all(|e| e.construction));
        soliddraft_app::file::set_config_dir(None);
    }
}
