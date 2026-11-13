//! Copy placement through the rendered app, using pointer and keyboard input.

use egui::{Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::Queryable;
use soliddraft_app::document::{Command, Document, PlaneKind, SketchPlane};
use soliddraft_app::sketch::{EntityKind, P2, Sketch};
use soliddraft_app::sketcher::Frame;
use soliddraft_app::state::{SketchSession, Tool};

#[path = "support/ui.rs"]
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
    ribbon::close_options(app);
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
    ribbon::close_options(app);
    let pos = screen(app, p);
    ribbon::render_steps::rendered(app, |app| {
        app.hover_at(pos);
        app.run_steps(3);
    });
}

fn drag(app: &mut App, a: P2, b: P2) {
    ribbon::close_options(app);
    let (a, b) = (screen(app, a), screen(app, b));
    ribbon::render_steps::rendered(app, |app| {
        app.hover_at(a);
        app.run_steps(3);
        app.event(egui::Event::PointerButton {
            pos: a,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::default(),
        });
        app.step();
        for i in 1..=8 {
            app.hover_at(a + (b - a) * (i as f32 / 8.0));
            app.step();
        }
        // The live preview must exist while the button is still down.
        assert!(session(app).preview.is_some());
        app.event(egui::Event::PointerButton {
            pos: b,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::default(),
        });
        app.step();
        settle(app);
    });
}

fn button(app: &mut App, label: &str) {
    ribbon::click_option(app, label);
    settle(app);
}

fn select(app: &mut App, p: P2) {
    app.key_press(Key::Escape);
    settle(app);
    click(app, p, Modifiers::default());
}

fn near(a: P2, b: P2) {
    assert!(a.dist(b) < 0.08, "{a:?} expected {b:?}");
}

fn count(app: &App) -> usize {
    session(app).sketch.drawn().count()
}

fn undo(app: &mut App, expected: usize) {
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(app);
    assert_eq!(count(app), expected, "one undo removes all copies");
}

#[test]
fn copy_tools_place_values_without_moving_sources_and_apply_as_one_undo() {
    let dir = std::env::var_os("SOLIDDRAFT_COPY_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-copy-{}", std::process::id())));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let frame = |app: &mut App, name: &str| {
        app.render().expect("the app renders").save(dir.join(format!("{name}.png"))).unwrap();
    };
    let mut doc = Document::new("Copy placement", soliddraft_app::kernel());
    doc.apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } }).unwrap();
    let id = doc.features[0].id;
    let name = doc.features[0].name.clone();
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Line { a: P2::new(-20.0, -10.0), b: P2::new(0.0, -10.0) }, false);
    // Stored backwards, but the selection walks upwards after the first line.
    sketch.add(EntityKind::Line { a: P2::new(0.0, 10.0), b: P2::new(0.0, -10.0) }, false);
    sketch.add(EntityKind::Circle { center: P2::new(20.0, 10.0), radius: 5.0 }, true);
    sketch.add(
        EntityKind::Arc { center: P2::new(15.0, -12.0), start: P2::new(20.0, -12.0), end: P2::new(15.0, -7.0) },
        false,
    );
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = launch(doc, STANDARD);
    ribbon::open_tree(&mut app);
    let view = app.state().state.viewport.rect;
    let left = egui::Rect::from_min_max(egui::pos2(0.0, view.top()), egui::pos2(view.left(), view.bottom()));
    let row = ribbon::button_rect(&app, &name, left);
    ribbon::double_click_at(&mut app, row.center());
    settle(&mut app);
    assert!(app.state().state.sketch.is_some());
    app.key_press(Key::Z);
    settle(&mut app);
    let source = session(&app).sketch.clone();

    // Select the chain in order. Dragging its reversed member keeps the signed side.
    click(&mut app, P2::new(-12.0, -10.0), Modifiers::default());
    ribbon::run_command(&mut app, "sketch_offset");
    click(&mut app, P2::new(0.0, 3.0), Modifiers::CTRL);
    assert_eq!(soliddraft_app::sketch_tools::selected_in_order(session(&app)), vec![1, 2]);
    drag(&mut app, P2::new(0.0, 3.0), P2::new(-4.0, 3.0));
    assert!((session(&app).modify.distance - 4.0).abs() < 0.08);
    assert_eq!(session(&app).sketch, source, "dragging a copy tool never moves its source");
    frame(&mut app, "01-offset-left-preview");
    drag(&mut app, P2::new(0.0, 3.0), P2::new(4.0, 3.0));
    assert!((session(&app).modify.distance + 4.0).abs() < 0.08);
    let preview = session(&app).preview.as_ref().unwrap().added.clone();
    assert_eq!(preview.len(), 2);
    if let EntityKind::Line { a, .. } = preview[0] {
        near(a, P2::new(-20.0, -14.0));
    } else {
        panic!("line copy");
    }
    frame(&mut app, "02-offset-right-preview");
    button(&mut app, "Apply");
    assert_eq!(count(&app), 6);
    frame(&mut app, "02b-offset-applied");
    for (e, kind) in session(&app).sketch.drawn().skip(4).zip(preview) {
        assert_eq!(e.kind, kind);
    }
    undo(&mut app, 4);
    app.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    settle(&mut app);
    assert_eq!(count(&app), 6, "redo restores both copies");
    undo(&mut app, 4);

    // A circle's offset is inward positive. An excessive numeric distance refuses without editing.
    select(&mut app, P2::new(25.0, 10.0));
    ribbon::run_command(&mut app, "sketch_offset");
    drag(&mut app, P2::new(25.0, 10.0), P2::new(27.0, 10.0));
    let preview = session(&app).preview.as_ref().unwrap();
    assert!(matches!(preview.added[0], EntityKind::Circle { radius, .. } if (radius - 7.0).abs() < 0.08));
    drag(&mut app, P2::new(25.0, 10.0), P2::new(20.0, 10.0));
    assert!((session(&app).modify.distance - 5.0).abs() < 0.08);
    let distance = app.get_by_role(egui::accesskit::Role::SpinButton).rect();
    ribbon::double_click_at(&mut app, distance.center());
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text("6".into()));
    app.key_press(Key::Enter);
    settle(&mut app);
    assert_eq!(session(&app).modify.distance, 6.0);
    assert!(session(&app).preview.as_ref().unwrap().operation.is_none());
    button(&mut app, "Apply");
    assert_eq!(session(&app).sketch, source);
    frame(&mut app, "03-offset-refusal");

    // An arc uses the same radial distance and keeps its angular extent.
    let radial = 0.5f64.sqrt();
    let arc_at = |r| P2::new(15.0 + r * radial, -12.0 + r * radial);
    select(&mut app, arc_at(5.0));
    ribbon::run_command(&mut app, "sketch_offset");
    drag(&mut app, arc_at(5.0), arc_at(7.0));
    assert!((session(&app).modify.distance + 2.0).abs() < 0.08);
    match session(&app).preview.as_ref().unwrap().added[0] {
        EntityKind::Arc { center, start, end } => {
            near(center, P2::new(15.0, -12.0));
            near(start, P2::new(22.0, -12.0));
            near(end, P2::new(15.0, -5.0));
        }
        _ => panic!("arc copy"),
    }
    assert_eq!(session(&app).sketch, source);
    frame(&mut app, "03b-arc-offset-preview");

    // Two mirror points, with the second point previewed before it is clicked.
    select(&mut app, P2::new(-12.0, -10.0));
    ribbon::run_command(&mut app, "sketch_mirror");
    click(&mut app, P2::new(-5.0, -15.0), Modifiers::default());
    hover(&mut app, P2::new(-5.0, 15.0));
    assert_eq!(count(&app), 4);
    near(session(&app).modify.axis_end, P2::new(-5.0, 15.0));
    match session(&app).preview.as_ref().unwrap().added[0] {
        EntityKind::Line { a, b } => {
            near(a, P2::new(10.0, -10.0));
            near(b, P2::new(-10.0, -10.0));
        }
        _ => panic!("mirrored line"),
    }
    frame(&mut app, "04-mirror-live-preview");
    click(&mut app, P2::new(-5.0, 15.0), Modifiers::default());
    let preview = session(&app).preview.as_ref().unwrap().added.clone();
    hover(&mut app, P2::new(10.0, 20.0));
    assert_eq!(session(&app).preview.as_ref().unwrap().added, preview, "placed axis stays put");
    button(&mut app, "Apply");
    assert_eq!(count(&app), 6, "the linked mirror also retains its construction axis");
    assert_eq!(session(&app).sketch.drawn().last().unwrap().kind, preview[0]);
    undo(&mut app, 4);

    // Place a translation between two arbitrary points, snapping its start to the centre.
    select(&mut app, P2::new(25.0, 10.0));
    ribbon::run_command(&mut app, "sketch_linear_pattern");
    click(&mut app, P2::new(20.0, 10.0), Modifiers::default());
    hover(&mut app, P2::new(28.0, 15.0));
    near(session(&app).modify.step, P2::new(8.0, 5.0));
    frame(&mut app, "05-linear-live-preview");
    click(&mut app, P2::new(28.0, 15.0), Modifiers::default());
    let preview = session(&app).preview.as_ref().unwrap().added.clone();
    assert_eq!(preview.len(), 2);
    if let EntityKind::Circle { center, .. } = preview[0] {
        near(center, P2::new(28.0, 15.0));
    } else {
        panic!("circle copy");
    }
    button(&mut app, "Apply");
    assert_eq!(count(&app), 6);
    assert!(session(&app).sketch.drawn().skip(4).all(|e| e.construction));
    undo(&mut app, 4);

    // A circular pattern follows the centre, freezes on click, and cancels without edits.
    select(&mut app, P2::new(25.0, 10.0));
    ribbon::run_command(&mut app, "sketch_circular_pattern");
    hover(&mut app, P2::new(5.0, 0.0));
    let preview = session(&app).preview.as_ref().unwrap().added.clone();
    if let EntityKind::Circle { center, .. } = preview[0] {
        near(center, P2::new(-5.0, 15.0));
    } else {
        panic!("circle copy");
    }
    frame(&mut app, "06-circular-live-preview");
    click(&mut app, P2::new(5.0, 0.0), Modifiers::default());
    hover(&mut app, P2::new(10.0, 20.0));
    assert_eq!(session(&app).preview.as_ref().unwrap().added, preview);
    button(&mut app, "Cancel");
    assert_eq!(session(&app).tool, Tool::Select);
    assert_eq!(session(&app).sketch, source);
    ribbon::run_command(&mut app, "sketch_circular_pattern");
    click(&mut app, P2::new(5.0, 0.0), Modifiers::default());
    button(&mut app, "Apply");
    assert_eq!(count(&app), 6);
    undo(&mut app, 4);
    assert_eq!(session(&app).sketch, source);

    // Escape drops a half-placed axis and keeps the sketch open.
    select(&mut app, P2::new(-12.0, -10.0));
    ribbon::run_command(&mut app, "sketch_mirror");
    click(&mut app, P2::new(-5.0, -15.0), Modifiers::default());
    app.key_press(Key::Escape);
    settle(&mut app);
    assert_eq!(session(&app).tool, Tool::Select);
    assert!(session(&app).pending.is_empty());
    assert_eq!(session(&app).sketch, source);
    frame(&mut app, "07-cancelled");
    soliddraft_app::file::set_config_dir(None);
}
