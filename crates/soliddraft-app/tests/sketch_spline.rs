//! Native point placement, completion, closure and saved history for splines.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Pos2};
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, PointRef, SplineMode},
    sketcher::Frame,
    state::{AppAction, SketchSel, SketchSession, Tool},
};

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}
fn screen(app: &App, point: P2) -> Pos2 {
    let state = &app.state().state;
    Frame { plane: session(app).plane, camera: &state.viewport.camera, rect: state.viewport.rect }
        .screen(point)
        .unwrap()
}
fn pick(app: &mut App, point: P2) {
    let at = screen(app, point);
    ribbon::click_at(app, at);
}

#[test]
fn spline_tools_place_finish_close_undo_and_reopen_through_the_pointer() {
    for (mode, tool, label, name, points) in [
        (
            SplineMode::Fit,
            Tool::FitSpline,
            "Fit-point spline",
            "spline-fit-pointer",
            vec![P2::ZERO, P2::new(20.0, 30.0), P2::new(40.0, 0.0)],
        ),
        (
            SplineMode::Control,
            Tool::ControlSpline,
            "Control-point spline",
            "spline-control-pointer",
            vec![P2::ZERO, P2::new(0.0, 40.0), P2::new(40.0, 40.0), P2::new(40.0, 0.0)],
        ),
    ] {
        let case = ribbon::case(name);
        let mut document = ribbon::blank();
        let id = document
            .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
            .unwrap()
            .unwrap();
        let mut app = ribbon::app(document);
        case.run(&mut app, |app| {
            app.state_mut().state.push(AppAction::EditSketch(id));
            app.run_steps(15);
            ribbon::click_tab(app, "Sketch");
            ribbon::click_ribbon(app, label);
            assert_eq!(session(app).tool, tool);
            let before = session(app).sketch.clone();
            pick(app, P2::ZERO);
            pick(app, P2::new(20.0, 20.0));
            assert_eq!(session(app).pending.len(), 2);
            assert_eq!(session(app).sketch, before);
            app.key_press(Key::Backspace);
            app.run_steps(4);
            assert_eq!(session(app).pending.len(), 1);
            app.key_press(Key::Escape);
            app.run_steps(4);
            assert!(session(app).pending.is_empty());
            assert_eq!(session(app).tool, Tool::Select);
            assert_eq!(session(app).sketch, before);

            ribbon::click_ribbon(app, label);
            let count = if mode == SplineMode::Control { points.len() - 1 } else { points.len() };
            for point in &points[..count] {
                pick(app, *point);
            }
            case.frame(app, "preview");
            if mode == SplineMode::Control {
                let last = screen(app, *points.last().unwrap());
                ribbon::double_click_at(app, last);
            } else {
                app.key_press(Key::Enter);
                app.run_steps(5);
            }
            assert!(session(app).pending.is_empty(), "{mode:?}: {:?}", session(app).message);
            assert_eq!(session(app).sketch.drawn().count(), 1);
            let entity = session(app).sketch.drawn().next().unwrap();
            let curve = entity.id;
            assert!(matches!(&entity.kind, EntityKind::Spline { mode: actual, points: actual_points }
                if *actual == mode && actual_points.len() == points.len()));
            let saved = session(app).sketch.clone();
            app.state_mut().state.push(AppAction::Undo);
            app.run_steps(6);
            assert_eq!(session(app).sketch, before);
            app.state_mut().state.push(AppAction::Redo);
            app.run_steps(6);
            assert_eq!(session(app).sketch, saved);

            app.key_press(Key::Escape);
            app.run_steps(4);
            let handle = session(app).sketch.entity(curve).unwrap().point(1).unwrap();
            pick(app, handle);
            assert!(session(app).selection.contains(&SketchSel::Point(PointRef::new(curve, 1))));
            case.frame(app, "selected-handle");
            let from = screen(app, handle);
            let moved = handle + P2::new(5.0, 5.0);
            let to = screen(app, moved);
            app.run_steps(30);
            app.event(egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            app.step();
            app.hover_at(to);
            app.run_steps(5);
            app.event(egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            app.run_steps(5);
            let actual = session(app).sketch.entity(curve).unwrap().point(1).unwrap();
            assert!(actual.dist(moved) < 1e-3, "{mode:?} handle drag: {actual:?} != {moved:?}");
            app.state_mut().state.push(AppAction::Undo);
            app.run_steps(6);
            assert_eq!(session(app).sketch, saved);

            ribbon::click_ribbon(app, "Line");
            let end = session(app).sketch.entity(curve).unwrap().point(points.len() - 1).unwrap();
            pick(app, end);
            pick(app, P2::ZERO);
            assert_eq!(session(app).sketch.regions().unwrap().len(), 1);

            ribbon::click_ribbon(app, label);
            let close =
                [P2::new(60.0, 0.0), P2::new(80.0, 0.0), P2::new(80.0, 20.0), P2::new(60.0, 20.0), P2::new(60.0, 0.0)];
            for point in close {
                pick(app, point);
            }
            assert!(session(app).pending.is_empty());
            let closed = session(app).sketch.drawn().last().unwrap();
            assert!(matches!(&closed.kind, EntityKind::Spline { points, .. } if points.len() == 5));
            assert!(closed.point(0).unwrap().dist(closed.point(4).unwrap()) < 1e-9);
            assert!(session(app).sketch.constraints.iter().any(|constraint| matches!(constraint.kind,
                ConstraintKind::Coincident { a, b }
                    if a == PointRef::new(closed.id, 0) && b == PointRef::new(closed.id, 4))));
            assert_eq!(session(app).sketch.regions().unwrap().len(), 2);
            case.frame(app, "closed-profiles");
            let saved = session(app).sketch.clone();
            let path = case.path("splines.dwprt");
            app.state_mut().state.push(AppAction::ExitSketch(None));
            app.run_steps(6);
            app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
            app.run_steps(6);
            app.state_mut().state.push(AppAction::Open(Some(path)));
            app.run_steps(12);
            assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, saved);
        });
    }
}
