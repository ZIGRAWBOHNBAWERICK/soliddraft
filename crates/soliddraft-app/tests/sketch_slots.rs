//! Slot commands through native pointer input, including cancellation and history.

#[path = "support/ui.rs"]
mod ribbon;

use egui::Key;
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, Sketch, SolveStatus, solve},
    sketcher::Frame,
    state::{AppAction, SketchSession, Tool},
};

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}

fn pick(app: &mut App, p: P2) {
    let state = &app.state().state;
    let frame = Frame { plane: session(app).plane, camera: &state.viewport.camera, rect: state.viewport.rect };
    let at = frame.screen(p).unwrap();
    ribbon::click_at(app, at);
}

fn edit_width(sketch: &Sketch, circular: bool) {
    let mut edited = sketch.clone();
    let boundary: Vec<_> = sketch.drawn().filter(|e| !e.construction).map(|e| e.id).collect();
    edited.constrain(ConstraintKind::Radius { entity: boundary[1], value: 3.0, diameter: false, label: P2::ZERO });
    solve::solve(&mut edited, None, true);
    assert!(!matches!(edited.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }), "{:?}", edited.status);
    assert_eq!(edited.regions().unwrap().len(), 1);
    for cap in [boundary[1], boundary[3]] {
        assert!((edited.entity(cap).unwrap().radius().unwrap() - 3.0).abs() < 1e-5);
    }
    if circular {
        let outer = edited.entity(boundary[0]).unwrap();
        let inner = edited.entity(boundary[2]).unwrap();
        let EntityKind::Arc { center: a, .. } = outer.kind else { panic!("circular side") };
        let EntityKind::Arc { center: b, .. } = inner.kind else { panic!("circular side") };
        assert!(a.dist(b) < 1e-5);
        assert!((outer.radius().unwrap() - inner.radius().unwrap() - 6.0).abs() < 1e-5);
    }
}

fn exercise(command: &str, tool: Tool, picks: &[P2], area: f64) {
    let dir = std::env::var_os("SOLIDDRAFT_SLOT_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-slots-{}", std::process::id())))
        .join(format!("{tool:?}"));
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
    app.state_mut().state.sketch.as_mut().unwrap().auto_relations = false;
    ribbon::switch_workspace(&mut app, "sketch");
    let original = session(&app).sketch.clone();

    // Every partially collected path cancels without document geometry or history.
    for count in 1..picks.len() {
        ribbon::run_command(&mut app, command);
        assert_eq!(session(&app).tool, tool);
        for &p in &picks[..count] {
            pick(&mut app, p);
        }
        assert_eq!(session(&app).pending.len(), count);
        assert_eq!(session(&app).sketch, original);
        if count == 1 {
            pick(&mut app, picks[0]);
            assert_eq!(session(&app).pending.len(), 1, "coincident defining picks are refused");
        } else if count == 2 && matches!(tool, Tool::ThreePointArcSlot | Tool::CentrepointArcSlot) {
            let invalid = if tool == Tool::ThreePointArcSlot { picks[0].mid(picks[1]) } else { picks[1] };
            pick(&mut app, invalid);
            assert_eq!(session(&app).pending.len(), 2, "degenerate arc is refused");
        }
        assert_eq!(session(&app).sketch, original);
        if count == 1 {
            let at = app.state().state.viewport.rect.center();
            app.hover_at(at);
            app.run_steps(3);
            for pressed in [true, false] {
                app.event(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                });
                app.step();
            }
            app.run_steps(4);
            assert!(session(&app).pending.is_empty());
            assert_eq!(session(&app).tool, tool, "right-click cancels the pending slot and keeps its tool");
        }
        app.key_press(Key::Escape);
        app.run_steps(4);
        assert!(session(&app).pending.is_empty());
        assert_eq!(session(&app).sketch, original);
    }

    ribbon::run_command(&mut app, command);
    for &p in &picks[..picks.len() - 1] {
        pick(&mut app, p);
    }
    if tool != Tool::Slot {
        let zero_width = if tool == Tool::CentrepointArcSlot { picks[1] } else { picks[0] };
        pick(&mut app, zero_width);
        assert_eq!(session(&app).pending.len(), picks.len() - 1, "zero width is refused");
        assert_eq!(session(&app).sketch, original);
    }
    let width = *picks.last().unwrap();
    app.hover_at({
        let state = &app.state().state;
        Frame { plane: session(&app).plane, camera: &state.viewport.camera, rect: state.viewport.rect }
            .screen(width)
            .unwrap()
    });
    app.run_steps(4);
    app.render().unwrap().save(dir.join("preview.png")).unwrap();
    assert_eq!(session(&app).sketch, original, "preview must not edit the document");
    pick(&mut app, width);
    assert!(session(&app).pending.is_empty(), "{tool:?}: {:?}", session(&app).message);
    assert!(session(&app).message.is_none(), "a successful retry clears the refusal hint");
    let saved = session(&app).sketch.clone();
    assert_eq!(saved.drawn().filter(|e| !e.construction).count(), 4);
    let regions = saved.regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert!(regions[0].holes.is_empty());
    // Pointer projection passes through f32, so use a relative geometric tolerance.
    assert!((regions[0].area() - area).abs() < area * 1e-4, "{} != {area}", regions[0].area());
    edit_width(&saved, matches!(tool, Tool::ThreePointArcSlot | Tool::CentrepointArcSlot));
    app.render().unwrap().save(dir.join("placed.png")).unwrap();

    app.state_mut().state.push(AppAction::Undo);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, original, "one undo removes the complete slot");
    app.state_mut().state.push(AppAction::Redo);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, saved);

    // A tool change discards a pending slot as well.
    pick(&mut app, picks[0]);
    ribbon::run_command(&mut app, "sketch_point");
    assert!(session(&app).pending.is_empty());
    assert_eq!(session(&app).sketch, saved);

    let path = dir.join("slot.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(12);
    let reopened: &Sketch = &app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch;
    ribbon::assert_sketch_close(reopened, &saved);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn slot_variants_create_preview_cancel_undo_and_reopen() {
    let pi = std::f64::consts::PI;
    for (command, tool, picks, area) in [
        (
            "sketch_slot",
            Tool::Slot,
            vec![P2::new(-20.0, 0.0), P2::new(20.0, 0.0), P2::new(0.0, 4.0)],
            320.0 + 16.0 * pi,
        ),
        (
            "sketch_centrepoint_slot",
            Tool::CentrepointSlot,
            vec![P2::ZERO, P2::new(20.0, 0.0), P2::new(0.0, -4.0)],
            320.0 + 16.0 * pi,
        ),
        (
            "sketch_three_point_arc_slot",
            Tool::ThreePointArcSlot,
            vec![P2::new(20.0, 0.0), P2::new(0.0, 20.0), P2::new(12.0, 16.0), P2::new(22.0, 0.0)],
            44.0 * pi,
        ),
        (
            "sketch_centrepoint_arc_slot",
            Tool::CentrepointArcSlot,
            vec![P2::ZERO, P2::new(20.0, 0.0), P2::new(0.0, 20.0), P2::new(0.0, 18.0)],
            44.0 * pi,
        ),
    ] {
        exercise(command, tool, &picks, area);
    }
}
