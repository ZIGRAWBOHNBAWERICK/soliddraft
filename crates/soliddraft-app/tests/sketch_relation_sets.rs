//! Composite and whole-entity relations use native selection, one undo step and saved geometry.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2, Rect};
use ribbon::App;
use soliddraft_app::{
    document::{Command, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, Sketch, SolveStatus},
    sketcher::Frame,
    state::{AppAction, SketchSel, SketchSession},
};

fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}
fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap()
}
fn pick(app: &mut App, at: P2, add: bool) {
    ribbon::close_options(app);
    let pos = screen(app, at);
    let modifiers = if add { Modifiers::CTRL } else { Modifiers::NONE };
    app.hover_at(pos);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event_modifiers(
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers },
            modifiers,
        );
        app.step();
    }
    app.run_steps(4);
}
fn open(app: &mut App) {
    let window = ribbon::window(app);
    if !ribbon::painted(app).iter().any(|p| p.text == "Relations") {
        ribbon::click_button(app, "Constrain group", window);
    }
    ribbon::click_button(app, "Relations", window);
    app.run_steps(4);
    assert!(session(app).relations_open);
}
fn menu(app: &App) -> Rect {
    let control = ribbon::painted(app).into_iter().find(|p| p.text == "Show all relations").unwrap();
    Rect::from_min_max(
        control.rect.left_top() - egui::vec2(8.0, 8.0),
        control.rect.left_top() + egui::vec2(260.0, 500.0),
    )
}
fn proof(app: &mut App, name: &str) {
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_RELATION_SETS_PROOF_DIR") {
        let path = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&path).unwrap();
        app.render().unwrap().save(path.join(format!("{name}.png"))).unwrap();
    }
}

fn panel_click(app: &mut App, label: &str) {
    ribbon::click_option(app, label);
}

#[test]
fn relation_sets_apply_once_undo_together_and_survive_reopening() {
    let dir = std::env::temp_dir().join(format!("soliddraft-relation-sets-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut s = Sketch::new();
    let left = s.add(EntityKind::Line { a: p(-16.0, -12.0), b: p(-10.0, -4.0) }, false);
    let right = s.add(EntityKind::Line { a: p(10.2, -4.1), b: p(16.1, -12.2) }, false);
    let axis = s.add(EntityKind::Line { a: p(0.0, -20.0), b: p(0.0, 20.0) }, true);
    let c = s.add(EntityKind::Circle { center: p(-12.0, 12.0), radius: 3.0 }, true);
    let d = s.add(EntityKind::Circle { center: p(12.0, 12.0), radius: 4.0 }, true);
    let mark = s.add(EntityKind::Point { p: p(3.0, -3.0) }, true);
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch: s }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    let base = session(&app).sketch.clone();
    for (name, picks, expected, count) in [
        ("Symmetric", vec![p(-13.0, -8.0), p(13.15, -8.15), p(0.0, -15.0)], vec![left, right, axis], 2),
        ("Collinear", vec![p(-13.0, -8.0), p(13.15, -8.15)], vec![left, right], 2),
        ("Coradial", vec![p(-15.0, 12.0), p(16.0, 12.0)], vec![c, d], 2),
        ("Intersection", vec![p(3.0, -3.0), p(-13.0, -8.0), p(0.0, -15.0)], vec![mark, left, axis], 2),
        ("Fix", vec![p(-15.0, 12.0)], vec![c], 1),
    ] {
        for (i, at) in picks.into_iter().enumerate() {
            pick(&mut app, at, i != 0);
        }
        assert_eq!(
            session(&app)
                .selection
                .iter()
                .map(|sel| match sel {
                    SketchSel::Entity(id) => *id,
                    SketchSel::Point(p) => p.entity,
                    SketchSel::Constraint(_) => panic!("expected a geometry selection"),
                })
                .collect::<Vec<_>>(),
            expected,
            "{name}"
        );
        open(&mut app);
        proof(&mut app, &format!("{name}-menu"));
        if name == "Coradial" {
            app.key_press(Key::Escape);
            app.run_steps(4);
            panel_click(&mut app, name);
        } else {
            let region = menu(&app);
            ribbon::click_button(&mut app, name, region);
        }
        app.run_steps(8);
        let after = session(&app).sketch.clone();
        assert_eq!(after.constraints.len(), base.constraints.len() + count, "{name}");
        assert!(
            matches!(after.status, SolveStatus::UnderDefined | SolveStatus::FullyDefined),
            "{name}: {:?}",
            after.status
        );
        proof(&mut app, &format!("{name}-applied"));
        open(&mut app);
        assert!(!ribbon::control(&app, name, menu(&app)).enabled, "{name}: already applied");
        app.key_press(Key::Escape);
        app.run_steps(4);
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        app.run_steps(8);
        assert_eq!(session(&app).sketch, base, "{name}: one undo");
        app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        app.run_steps(8);
        assert_eq!(session(&app).sketch, after, "{name}: one redo");
        if name == "Fix" {
            let path = dir.join("relations.dwprt");
            app.state_mut().state.push(AppAction::ExitSketch);
            app.run_steps(5);
            app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
            app.run_steps(5);
            app.state_mut().state.push(AppAction::Open(Some(path)));
            app.run_steps(10);
            app.state_mut().state.push(AppAction::EditSketch(id));
            app.run_steps(30);
            assert_eq!(session(&app).sketch.constraints, after.constraints);
            assert!(
                matches!(session(&app).sketch.constraints.last().unwrap().kind, ConstraintKind::FixEntity { entity, .. } if entity == c)
            );
            pick(&mut app, p(-15.0, 12.0), false);
            // Select the stored relation in its inspector, then release it.
            panel_click(&mut app, "Fixed");
            assert!(matches!(session(&app).selection.as_slice(), [SketchSel::Constraint(_)]));
            app.key_press(Key::Delete);
            app.run_steps(6);
            assert!(session(&app).sketch.constraints.is_empty());
            break;
        }
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        app.run_steps(8);
    }
    drop(app);
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}
