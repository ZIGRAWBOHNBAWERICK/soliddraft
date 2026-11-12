//! Selection transforms through the rendered app, with pointer placement and numeric input.

use egui::{Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use soliddraft_app::document::{Command, Document, PlaneKind, SketchPlane};
use soliddraft_app::sketch::{EntityKind, P2, Sketch};
use soliddraft_app::sketcher::Frame;
use soliddraft_app::state::{SketchSession, Tool};

#[path = "support/ribbon.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use ribbon::App;
use ux_tools::{STANDARD, launch, settle};

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().expect("open sketch")
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap()
}

fn click(app: &mut App, p: P2, modifiers: Modifiers) {
    let pos = screen(app, p);
    ribbon::render_steps::rendered(app, |app| {
        app.hover_at(pos);
        app.run_steps(3);
        for pressed in [true, false] {
            app.event_modifiers(
                egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers },
                modifiers,
            );
            app.step();
        }
        settle(app);
    });
}

fn hover(app: &mut App, p: P2) {
    let pos = screen(app, p);
    ribbon::render_steps::rendered(app, |app| {
        app.hover_at(pos);
        app.run_steps(3);
    });
}

fn button(app: &mut App, label: &str) {
    let view = app.state().state.viewport.rect;
    let pane = egui::Rect::from_min_max(egui::pos2(0.0, view.top()), egui::pos2(view.left(), view.bottom()));
    for _ in 0..6 {
        if app.get_all_by_role_and_label(egui::accesskit::Role::Button, label).any(|n| pane.contains_rect(n.rect())) {
            break;
        }
        app.hover_at(egui::pos2(pane.right() - 18.0, pane.bottom() - 60.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        settle(app);
    }
    ribbon::click_button(app, label, pane);
    settle(app);
}

fn near(a: P2, b: P2) {
    assert!(a.dist(b) < 0.08, "{a:?} expected {b:?}");
}

fn number(app: &mut App, index: usize, text: &str) {
    let view = app.state().state.viewport.rect;
    let header = ribbon::painted(app).into_iter().find(|p| p.text == "Properties").expect("properties panel");
    let fields =
        egui::Rect::from_min_max(egui::pos2(0.0, header.rect.bottom() + 40.0), egui::pos2(view.left(), view.bottom()));
    // Accessibility retains clipped controls. Never click their offscreen bounds,
    // which can overlap the command bar's Cancel button.
    for _ in 0..8 {
        if app
            .query_all(egui_kittest::kittest::By::new().role(egui::accesskit::Role::SpinButton))
            .nth(index)
            .is_some_and(|n| fields.contains_rect(n.rect()))
        {
            break;
        }
        app.hover_at(egui::pos2(view.left() - 18.0, view.bottom() - 60.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        settle(app);
    }
    let rect = app.get_all_by_role(egui::accesskit::Role::SpinButton).nth(index).expect("numeric input").rect();
    assert!(fields.contains_rect(rect), "numeric field must be fully visible: {rect:?}");
    ribbon::double_click_at(app, rect.center());
    app.run_steps(2);
    assert!(app.ctx.egui_wants_keyboard_input(), "double click must focus the numeric editor");
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text(text.into()));
    app.key_press(Key::Enter);
    settle(app);
}

fn tool(app: &mut App, label: &str) {
    let band = ribbon::ribbon_region(app);
    if !app.get_all_by_role_and_label(egui::accesskit::Role::Button, label).any(|n| band.intersects(n.rect())) {
        let window = ribbon::window(app);
        ribbon::click_button(app, "Transform group", window);
    }
    let controls = egui::Rect::from_min_max(band.min, egui::pos2(band.right(), band.bottom() + 120.0));
    ribbon::click_button(app, label, controls);
    settle(app);
    assert_eq!(session(app).tool.label(), label);
}

fn undo_redo(app: &mut App, before: &Sketch, after: &Sketch) {
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(app);
    assert_eq!(&session(app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    settle(app);
    assert_eq!(&session(app).sketch, after);
}

#[test]
fn transforms_preview_apply_cancel_undo_and_reopen() {
    let dir = std::env::var_os("SOLIDDRAFT_TRANSFORM_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-transforms-{}", std::process::id())));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut doc = Document::new("Transforms", soliddraft_app::kernel());
    doc.apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } }).unwrap();
    let id = doc.features[0].id;
    let name = doc.features[0].name.clone();
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Line { a: P2::new(-20.0, -10.0), b: P2::new(0.0, -10.0) }, false);
    sketch.add(EntityKind::Circle { center: P2::new(20.0, 10.0), radius: 5.0 }, true);
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = launch(doc, STANDARD);
    let view = app.state().state.viewport.rect;
    let left = egui::Rect::from_min_max(egui::pos2(0.0, view.top()), egui::pos2(view.left(), view.bottom()));
    let row = ribbon::button_rect(&app, &name, left);
    ribbon::double_click_at(&mut app, row.center());
    settle(&mut app);
    app.key_press(Key::Z);
    settle(&mut app);
    let original = session(&app).sketch.clone();

    // Whole-entity transforms accept point picks, deduplicate owners and move a selection numerically.
    click(&mut app, P2::new(-20.0, -10.0), Modifiers::NONE);
    click(&mut app, P2::new(25.0, 10.0), Modifiers::CTRL);
    tool(&mut app, "Move entities");
    assert_eq!(soliddraft_app::sketch_tools::selected_in_order(session(&app)), vec![1, 2]);
    number(&mut app, 2, "4");
    number(&mut app, 3, "6");
    assert_eq!(session(&app).sketch, original);
    let preview = session(&app).preview.as_ref().unwrap().added.clone();
    assert_eq!(preview.len(), 2);
    app.render().unwrap().save(dir.join("01-move-preview.png")).unwrap();
    button(&mut app, "Apply");
    let moved = session(&app).sketch.clone();
    near(moved.entity(1).unwrap().point(0).unwrap(), P2::new(-16.0, -4.0));
    near(moved.entity(2).unwrap().point(0).unwrap(), P2::new(24.0, 16.0));
    assert_eq!(moved.drawn().map(|e| e.kind.clone()).collect::<Vec<_>>(), preview);
    undo_redo(&mut app, &original, &moved);

    // Copy placement follows the destination until the second click and then freezes.
    tool(&mut app, "Copy entities");
    click(&mut app, P2::new(3.0, 2.0), Modifiers::NONE);
    hover(&mut app, P2::new(11.0, 7.0));
    near(session(&app).modify.step, P2::new(8.0, 5.0));
    assert_eq!(session(&app).sketch, moved);
    click(&mut app, P2::new(11.0, 7.0), Modifiers::NONE);
    let preview = session(&app).preview.as_ref().unwrap().added.clone();
    hover(&mut app, P2::new(0.0, 30.0));
    assert_eq!(session(&app).preview.as_ref().unwrap().added, preview);
    app.render().unwrap().save(dir.join("02-copy-preview.png")).unwrap();
    button(&mut app, "Apply");
    let copied = session(&app).sketch.clone();
    assert_eq!(copied.drawn().count(), 4);
    assert_eq!(&copied.entities[..moved.entities.len()], &moved.entities);
    assert!(copied.entity(4).unwrap().construction);
    undo_redo(&mut app, &moved, &copied);

    // Rotate copies about a pointer centre with an editable signed angle.
    app.key_press(Key::Escape);
    settle(&mut app);
    click(&mut app, P2::new(2.0, 1.0), Modifiers::NONE);
    click(&mut app, P2::new(37.0, 21.0), Modifiers::CTRL);
    tool(&mut app, "Rotate entities");
    click(&mut app, P2::new(5.0, 0.0), Modifiers::NONE);
    number(&mut app, 2, "-90");
    let in_place_preview = session(&app).preview.as_ref().unwrap().added.clone();
    button(&mut app, "Apply");
    let in_place = session(&app).sketch.clone();
    assert_eq!(in_place.drawn().count(), 4);
    assert_eq!(in_place.drawn().skip(2).map(|e| e.kind.clone()).collect::<Vec<_>>(), in_place_preview);
    undo_redo(&mut app, &copied, &in_place);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(&mut app);
    let checkbox = app.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Copy entities").rect();
    ribbon::click_at(&mut app, checkbox.center());
    settle(&mut app);
    assert!(session(&app).modify.copy);
    let preview = session(&app).preview.as_ref().unwrap().added.clone();
    button(&mut app, "Apply");
    let rotated = session(&app).sketch.clone();
    assert_eq!(rotated.drawn().count(), 6);
    assert_eq!(rotated.drawn().skip(4).map(|e| e.kind.clone()).collect::<Vec<_>>(), preview);
    undo_redo(&mut app, &copied, &rotated);

    // Uniform scale includes circle radii, and Cancel/Escape discard preview-only edits.
    app.key_press(Key::Escape);
    settle(&mut app);
    click(&mut app, P2::new(6.0, 5.0), Modifiers::NONE);
    click(&mut app, P2::new(31.0, -27.0), Modifiers::CTRL);
    tool(&mut app, "Scale entities");
    click(&mut app, P2::new(5.0, 0.0), Modifiers::NONE);
    number(&mut app, 2, "0");
    assert!(session(&app).preview.as_ref().unwrap().operation.is_none());
    assert_eq!(session(&app).sketch, rotated, "invalid scaling cannot mutate geometry");
    number(&mut app, 2, "0.5");
    let preview = session(&app).preview.as_ref().unwrap().added.clone();
    button(&mut app, "Cancel");
    assert_eq!(session(&app).tool, Tool::Select);
    assert_eq!(session(&app).sketch, rotated);
    tool(&mut app, "Scale entities");
    // Reactivation starts live base placement; a collapsed ribbon popup can leave
    // the cursor over the viewport. Place the base again before comparing previews.
    click(&mut app, P2::new(5.0, 0.0), Modifiers::NONE);
    assert_eq!(session(&app).preview.as_ref().unwrap().added, preview);
    app.render().unwrap().save(dir.join("03-scale-preview.png")).unwrap();
    button(&mut app, "Apply");
    let scaled = session(&app).sketch.clone();
    assert!((scaled.entity(6).unwrap().radius().unwrap() - 2.5).abs() < 1e-6);
    undo_redo(&mut app, &rotated, &scaled);
    tool(&mut app, "Move entities");
    click(&mut app, P2::new(3.0, 2.0), Modifiers::NONE);
    app.key_press(Key::Escape);
    settle(&mut app);
    assert_eq!(session(&app).tool, Tool::Select);
    assert!(session(&app).pending.is_empty());
    assert_eq!(session(&app).sketch, scaled);

    app.render().unwrap().save(dir.join("04-applied.png")).unwrap();
    let path = dir.join("transforms.dwprt");
    app.state_mut().state.push(soliddraft_app::state::AppAction::ExitSketch);
    settle(&mut app);
    app.state_mut().state.push(soliddraft_app::state::AppAction::SaveAs(Some(path.clone())));
    settle(&mut app);
    app.state_mut().state.push(soliddraft_app::state::AppAction::Open(Some(path)));
    settle(&mut app);
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, scaled);
    soliddraft_app::file::set_config_dir(None);
}
