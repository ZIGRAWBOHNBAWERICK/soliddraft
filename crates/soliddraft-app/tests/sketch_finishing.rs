//! Native corner treatments and splits preview, cancel, commit and reopen.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2};
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, Entity, EntityKind, P2, PointRef, Sketch, SolveStatus, tools::ChamferSize},
    sketcher::{self, Frame},
    state::{AppAction, SketchSession, Tool},
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
    let state = &app.state().state;
    Frame { plane: session(app).plane, camera: &state.viewport.camera, rect: state.viewport.rect }.screen(p).unwrap()
}
fn hover(app: &mut App, at: P2) {
    app.hover_at(screen(app, at));
    app.run_steps(4);
}
fn pick(app: &mut App, at: P2) {
    let pos = screen(app, at);
    ribbon::click_at(app, pos);
}
fn tool(app: &mut App, id: &str) {
    let point = ribbon::reveal_ribbon_command(app, id).rect.center();
    ribbon::click_at(app, point);
    app.run_steps(4);
}
fn panel_click(app: &mut App, label: &str) {
    let left = app.state().state.viewport.rect.left();
    for _ in 0..10 {
        if let Some(row) = ribbon::painted(app)
            .into_iter()
            .find(|p| p.text == label && p.rect.right() < left && p.rect.contains_rect(p.full))
        {
            ribbon::click_at(app, row.rect.center());
            return;
        }
        app.hover_at(egui::pos2(left - 20.0, 750.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -180.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        app.run_steps(10);
    }
    panic!("property control {label} must be reachable");
}
fn proof(app: &mut App, name: &str) {
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_FINISHING_PROOF_DIR") {
        let path = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&path).unwrap();
        app.render().unwrap().save(path.join(format!("{name}.png"))).unwrap();
    }
}
fn undo(app: &mut App) {
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
}

#[test]
fn native_finishing_keeps_analytic_previews_and_one_undo_step() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("soliddraft-finishing-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut sketch = Sketch::new();
    sketcher::add_rectangle(&mut sketch, p(-15.0, -10.0), p(15.0, 10.0));
    sketch.add(EntityKind::Circle { center: p(8.0, 3.0), radius: 3.0 }, true);
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    let before = session(&app).sketch.clone();
    tool(&mut app, "sketch_fillet");
    pick(&mut app, p(-10.0, -10.0));
    hover(&mut app, p(-15.0, -5.0));
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 3);
    assert_eq!(session(&app).sketch, before);
    proof(&mut app, "fillet-preview");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).tool, Tool::Select);
    assert!(session(&app).corner_first.is_none());
    assert_eq!(session(&app).sketch, before);
    tool(&mut app, "sketch_fillet");
    pick(&mut app, p(-10.0, -10.0));
    pick(&mut app, p(-15.0, -5.0));
    assert_eq!(session(&app).sketch.drawn().count(), 6);
    assert!(
        session(&app)
            .sketch
            .constraints
            .iter()
            .any(|c| matches!(c.kind, ConstraintKind::Radius { value, .. } if (value - 2.0).abs() < 1e-6))
    );
    let fillet = session(&app).sketch.clone();
    proof(&mut app, "fillet-applied");
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, fillet);
    undo(&mut app);
    tool(&mut app, "sketch_chamfer");
    panel_click(&mut app, "Distance and angle");
    assert!(matches!(session(&app).modify.chamfer, ChamferSize::DistanceAngle { .. }));
    pick(&mut app, p(-10.0, -10.0));
    hover(&mut app, p(-15.0, -5.0));
    assert_eq!(session(&app).sketch, before);
    proof(&mut app, "chamfer-preview");
    pick(&mut app, p(-15.0, -5.0));
    assert_eq!(session(&app).sketch.drawn().count(), 6);
    assert!(session(&app).sketch.drawn().last().unwrap().is_line());
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    tool(&mut app, "sketch_split");
    hover(&mut app, p(0.0, -10.0));
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 2);
    assert_eq!(session(&app).sketch, before);
    proof(&mut app, "split-line-preview");
    pick(&mut app, p(0.0, -10.0));
    assert_eq!(session(&app).sketch.drawn().count(), 6);
    undo(&mut app);
    assert_eq!(session(&app).sketch, before);
    tool(&mut app, "sketch_split");
    pick(&mut app, p(11.0, 3.0));
    assert!(session(&app).corner_first.is_some());
    hover(&mut app, p(8.0, 6.0));
    assert_eq!(session(&app).preview.as_ref().unwrap().added.len(), 2);
    proof(&mut app, "split-circle-preview");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    tool(&mut app, "sketch_split");
    pick(&mut app, p(11.0, 3.0));
    pick(&mut app, p(8.0, 6.0));
    let after = session(&app).sketch.clone();
    assert_eq!(after.drawn().filter(|e| matches!(e.kind, EntityKind::Arc { .. }) && e.construction).count(), 2);
    let path = dir.join("finishing.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch(None));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    assert_eq!(session(&app).sketch.constraints, after.constraints);
    assert_eq!(session(&app).sketch.drawn().count(), after.drawn().count());
    proof(&mut app, "split-reopened");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fixed_midpoint_treatments_match_preview_undo_redo_and_reopen() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for (command, fillet) in [("sketch_fillet", true), ("sketch_chamfer", false)] {
        for reverse_a in [false, true] {
            for reverse_b in [false, true] {
                let name = format!("midpoint-{}-{reverse_a}-{reverse_b}", if fillet { "fillet" } else { "chamfer" });
                let dir = std::env::temp_dir().join(format!("soliddraft-{name}-{}", std::process::id()));
                std::fs::create_dir_all(dir.join("config")).unwrap();
                soliddraft_app::file::set_config_dir(Some(dir.join("config")));
                let mut sketch = Sketch::new();
                let ends = |a, b, reverse| if reverse { (b, a) } else { (a, b) };
                let (a, b) = ends(p(-10.0, 0.0), P2::ZERO, reverse_a);
                let first = sketch.add(EntityKind::Line { a, b }, false);
                let (a, b) = ends(P2::ZERO, p(0.0, 10.0), reverse_b);
                let second = sketch.add(EntityKind::Line { a, b }, false);
                sketch.constrain(ConstraintKind::Coincident {
                    a: PointRef::new(first, usize::from(!reverse_a)),
                    b: PointRef::new(second, usize::from(reverse_b)),
                });
                for (entity, index, at) in
                    [(first, usize::from(reverse_a), p(-10.0, 0.0)), (second, usize::from(!reverse_b), p(0.0, 10.0))]
                {
                    sketch.constrain(ConstraintKind::Fix { point: PointRef::new(entity, index), at });
                }
                for (line, at) in [(first, p(-5.0, 0.0)), (second, p(0.0, 5.0))] {
                    let id = sketch.add(EntityKind::Point { p: at }, true);
                    sketch.constrain(ConstraintKind::Midpoint { point: PointRef::new(id, 0), line });
                    sketch.constrain(ConstraintKind::Fix { point: PointRef::new(id, 0), at });
                }
                let unrelated = sketch.add(EntityKind::Line { a: p(8.0, -4.0), b: p(16.0, -4.0) }, true);
                let marker = sketch.add(EntityKind::Point { p: p(12.0, -4.0) }, true);
                sketch.constrain(ConstraintKind::Midpoint { point: PointRef::new(marker, 0), line: unrelated });
                sketch.constrain(ConstraintKind::Fix { point: PointRef::new(marker, 0), at: p(12.0, -4.0) });
                let mut doc = ribbon::blank();
                let id = doc
                    .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
                    .unwrap()
                    .unwrap();
                doc.apply(Command::EditSketch { id, sketch }).unwrap();
                let mut app = ribbon::app(doc);
                app.state_mut().state.push(AppAction::EditSketch(id));
                app.run_steps(30);
                let before = session(&app).sketch.clone();
                tool(&mut app, command);
                pick(&mut app, p(-8.0, 0.0));
                hover(&mut app, p(0.0, 8.0));
                let preview = session(&app).preview.as_ref().expect("preview").added.clone();
                assert_eq!(preview.len(), 3, "{name}");
                assert_eq!(session(&app).sketch, before);
                pick(&mut app, p(0.0, 8.0));
                let after = session(&app).sketch.clone();
                proof(&mut app, &name);
                assert!(
                    matches!(after.status, SolveStatus::UnderDefined | SolveStatus::FullyDefined),
                    "{name}: {:?}",
                    after.status
                );
                for (id, kind) in [(first, preview[0].clone()), (second, preview[1].clone())] {
                    let expected = Entity { id, kind, construction: false };
                    for index in 0..2 {
                        assert!(
                            after.point(PointRef::new(id, index)).unwrap().dist(expected.point(index).unwrap()) < 1e-5,
                            "{name}: applied geometry must match preview"
                        );
                    }
                }
                assert!(!after.constraints.iter().any(
                    |c| matches!(c.kind, ConstraintKind::Midpoint { line, .. } if line == first || line == second)
                ));
                assert!(
                    after
                        .constraints
                        .iter()
                        .any(|c| matches!(c.kind, ConstraintKind::Midpoint { line, .. } if line == unrelated))
                );
                assert!(
                    session(&app).message.as_ref().unwrap().0.contains("released 3 relations"),
                    "{name}: {:?}",
                    session(&app).message
                );
                for c in before.constraints.iter().filter(|c| matches!(c.kind, ConstraintKind::Fix { .. })) {
                    assert_eq!(after.constraint(c.id), Some(c));
                    let ConstraintKind::Fix { point, at } = c.kind else { unreachable!() };
                    assert!(after.point(point).unwrap().dist(at) < 1e-5, "{name}: fixed geometry must stay put");
                }
                undo(&mut app);
                assert_eq!(session(&app).sketch.constraints, before.constraints, "{name}: one undo");
                for entity in before.drawn() {
                    for index in 0..entity.point_count() {
                        assert!(
                            session(&app)
                                .sketch
                                .point(PointRef::new(entity.id, index))
                                .unwrap()
                                .dist(entity.point(index).unwrap())
                                < 1e-5,
                            "{name}: undo restores geometry"
                        );
                    }
                }
                app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
                app.run_steps(8);
                assert_eq!(session(&app).sketch.constraints, after.constraints, "{name}: one redo");
                let path = dir.join("midpoints.dwprt");
                app.state_mut().state.push(AppAction::ExitSketch(None));
                app.run_steps(6);
                app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
                app.run_steps(6);
                app.state_mut().state.push(AppAction::Open(Some(path)));
                app.run_steps(10);
                app.state_mut().state.push(AppAction::EditSketch(id));
                app.run_steps(30);
                assert_eq!(session(&app).sketch.constraints, after.constraints, "{name}: saved relations");
                for entity in after.drawn() {
                    for index in 0..entity.point_count() {
                        assert!(
                            session(&app)
                                .sketch
                                .point(PointRef::new(entity.id, index))
                                .unwrap()
                                .dist(entity.point(index).unwrap())
                                < 1e-5,
                            "{name}: reopen restores geometry"
                        );
                    }
                }
                drop(app);
                soliddraft_app::file::set_config_dir(None);
                std::fs::remove_dir_all(dir).unwrap();
            }
        }
    }
}

#[test]
fn split_retargets_symmetry_and_keeps_it_through_undo_redo_and_reopen() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("soliddraft-symmetric-split-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    for arc in [false, true] {
        for swapped in [false, true] {
            let mut sketch = Sketch::new();
            let (kind, at, index, far) = if arc {
                (
                    EntityKind::Arc { center: p(4.0, 0.0), start: p(8.0, 0.0), end: p(4.0, 4.0) },
                    p(4.0 + 8.0_f64.sqrt(), 8.0_f64.sqrt()),
                    2,
                    p(4.0, 4.0),
                )
            } else {
                (EntityKind::Line { a: p(2.0, 0.0), b: p(10.0, 0.0) }, p(6.0, 0.0), 1, p(10.0, 0.0))
            };
            let source = sketch.add(kind, false);
            let axis_kind = EntityKind::Line { a: p(0.0, -10.0), b: p(0.0, 10.0) };
            let axis = sketch.add(axis_kind.clone(), true);
            sketch.constrain(ConstraintKind::FixEntity { entity: axis, at: axis_kind });
            let partner = sketch.add(EntityKind::Point { p: p(-far.u, far.v) }, true);
            let partner_ref = PointRef::new(partner, 0);
            let far_ref = PointRef::new(source, index);
            let (a, b) = if swapped { (partner_ref, far_ref) } else { (far_ref, partner_ref) };
            let symmetry = sketch.constrain(ConstraintKind::Symmetric { a, b, axis });
            let mut doc = ribbon::blank();
            let id = doc
                .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
                .unwrap()
                .unwrap();
            doc.apply(Command::EditSketch { id, sketch }).unwrap();
            let mut app = ribbon::app(doc);
            app.state_mut().state.push(AppAction::EditSketch(id));
            app.run_steps(30);
            let before = session(&app).sketch.clone();
            tool(&mut app, "sketch_split");
            hover(&mut app, at);
            assert!(session(&app).preview.as_ref().unwrap().operation.is_some());
            pick(&mut app, at);
            let after = session(&app).sketch.clone();
            let new = after.drawn().last().unwrap().id;
            let replacement = PointRef::new(new, index);
            let expected = if swapped {
                ConstraintKind::Symmetric { a: partner_ref, b: replacement, axis }
            } else {
                ConstraintKind::Symmetric { a: replacement, b: partner_ref, axis }
            };
            assert_eq!(
                after.constraint(symmetry).map(|c| c.kind.clone()),
                Some(expected),
                "arc={arc} swapped={swapped}"
            );
            proof(&mut app, &format!("split-symmetry-{arc}-{swapped}"));
            undo(&mut app);
            assert_eq!(session(&app).sketch, before);
            app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
            app.run_steps(8);
            assert_eq!(session(&app).sketch, after);
            let path = dir.join(format!("split-{arc}-{swapped}.dwprt"));
            app.state_mut().state.push(AppAction::ExitSketch(None));
            app.run_steps(6);
            app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
            app.run_steps(6);
            app.state_mut().state.push(AppAction::Open(Some(path)));
            app.run_steps(10);
            assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, after);
        }
    }
    soliddraft_app::file::set_config_dir(None);
}
