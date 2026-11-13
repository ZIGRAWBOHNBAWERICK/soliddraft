//! The relation list, its edits and glyph visibility through the rendered app.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2, Rect, Shape, pos2, vec2};
use egui_kittest::kittest::{By, Queryable};
use ribbon::App;
use soliddraft_app::{
    document::{Command, Document, FeatureId, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, DistanceKind, EntityKind, P2, PointRef, Sketch, solve},
    sketcher::Frame,
    state::{AppAction, SketchSel, SketchSession},
};

fn fixture() -> (Document, FeatureId, [u32; 5]) {
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut sketch = Sketch::new();
    let a = sketch.add(EntityKind::Line { a: P2::new(-30.0, -20.0), b: P2::new(0.0, -10.0) }, false);
    let b = sketch.add(EntityKind::Line { a: P2::new(10.0, -15.0), b: P2::new(40.0, -5.0) }, false);
    let c = sketch.add(EntityKind::Circle { center: P2::new(-25.0, 20.0), radius: 6.0 }, false);
    let d = sketch.add(EntityKind::Circle { center: P2::new(15.0, 25.0), radius: 8.0 }, false);
    let p = sketch.add(EntityKind::Point { p: P2::new(-10.0, 15.0) }, false);
    sketch.constrain(ConstraintKind::Distance {
        a: PointRef::new(a, 0),
        b: PointRef::new(a, 1),
        kind: DistanceKind::Aligned,
        value: 1000.0_f64.sqrt(),
        label: P2::new(-15.0, -35.0),
    });
    solve::solve(&mut sketch, None, true);
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    (doc, id, [a, b, c, d, p])
}

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().expect("sketch stays open")
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap()
}

fn pick(app: &mut App, p: P2, modifiers: Modifiers) {
    let pos = screen(app, p);
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

fn relations_control(app: &mut App) -> ribbon::Control {
    let window = ribbon::window(app);
    if app.query_all(By::new().role(egui::accesskit::Role::Button).label("Relations")).next().is_none() {
        ribbon::click_button(app, "Constrain group", window);
    }
    ribbon::control(app, "Relations", window)
}

fn open_menu(app: &mut App) {
    let control = relations_control(app);
    ribbon::click_at(app, control.rect.center());
    app.run_steps(4);
    assert!(session(app).relations_open);
}

fn menu_region(app: &App) -> Rect {
    let checkbox = app.get_by_label("Show all relations").rect();
    Rect::from_min_max(
        pos2(checkbox.left() - 8.0, checkbox.bottom()),
        pos2(checkbox.right() + 8.0, ribbon::window(app).bottom()),
    )
}

fn choose(app: &mut App, name: &str) {
    let region = menu_region(app);
    ribbon::click_button(app, name, region);
    app.run_steps(6);
    assert!(!session(app).relations_open);
}

fn close_menu(app: &mut App) {
    app.key_press(Key::Escape);
    app.run_steps(5);
    assert!(!session(app).relations_open);
}

fn glyphs(app: &App) -> Vec<Rect> {
    fn collect(shape: &Shape, viewport: Rect, out: &mut Vec<Rect>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, viewport, out)),
            Shape::Rect(r)
                if r.rect.size() == vec2(14.0, 14.0)
                    && r.corner_radius == egui::CornerRadius::same(3)
                    && viewport.contains_rect(r.rect) =>
            {
                out.push(r.rect);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &app.output().shapes {
        collect(&clipped.shape, app.state().state.viewport.rect, &mut out);
    }
    out
}

fn dimension_labels(app: &App) -> Vec<String> {
    let view = app.state().state.viewport.rect;
    ribbon::painted(app)
        .into_iter()
        .filter(|t| view.contains_rect(t.rect) && t.text == "31.62")
        .map(|t| t.text)
        .collect()
}

fn frame(app: &mut App, name: &str) {
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_RELATIONS_PROOF_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        app.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn sketch_relations_are_accessible_from_the_ribbon() {
    let dir = std::env::var_os("SOLIDDRAFT_RELATIONS_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-relations-{}", std::process::id())));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let (doc, id, [a, b, c, d, p]) = fixture();
    let mut app = ribbon::app(doc);
    ribbon::switch_workspace(&mut app, "sketch");
    assert!(!relations_control(&mut app).enabled, "unavailable outside a sketch");
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    assert!(relations_control(&mut app).enabled, "Relations must be an actionable sketch command");
    frame(&mut app, "before-menu");
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    open_menu(&mut app);
    let names = [
        "Horizontal",
        "Vertical",
        "Parallel",
        "Perpendicular",
        "Coincident",
        "On line",
        "On curve",
        "Tangent",
        "Equal",
        "Concentric",
        "Midpoint",
        "Fix",
    ];
    for name in names {
        assert!(!ribbon::control(&app, name, menu_region(&app)).enabled, "no selection: {name}");
    }
    frame(&mut app, "all-relation-types");
    close_menu(&mut app);
    assert_eq!(app.state().state.doc.revision(), revision, "opening and closing changes no geometry");

    // Real pointer selection, then the ribbon edit and a single undo/redo step.
    pick(&mut app, P2::new(-15.0, -15.0), Modifiers::NONE);
    pick(&mut app, P2::new(25.0, -10.0), Modifiers::CTRL);
    assert_eq!(session(&app).selection, [SketchSel::Entity(a), SketchSel::Entity(b)]);
    open_menu(&mut app);
    assert!(!ribbon::control(&app, "Tangent", menu_region(&app)).enabled);
    choose(&mut app, "Parallel");
    let added = session(&app).sketch.clone();
    let relation = added.constraints.last().unwrap().id;
    assert!(matches!(added.constraints.last().unwrap().kind, ConstraintKind::Parallel { .. }));
    assert_eq!(added.constraints.len(), before.constraints.len() + 1);
    assert_eq!(glyphs(&app).len(), 2, "one parallel glyph per owning line");
    assert_eq!(dimension_labels(&app), ["31.62"]);
    let glyph = glyphs(&app)[0].center();
    frame(&mut app, "parallel-visible");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before, "one undo removes the relation");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, added);

    // Hiding removes paint and hits, not dimensions, inference or stored constraints.
    open_menu(&mut app);
    assert!(!ribbon::control(&app, "Parallel", menu_region(&app)).enabled, "already applied");
    app.get_by_label("Show all relations").click();
    app.run_steps(4);
    close_menu(&mut app);
    assert!(!session(&app).show_relations && session(&app).auto_relations);
    assert!(glyphs(&app).is_empty());
    assert_eq!(dimension_labels(&app), ["31.62"]);
    assert_eq!(session(&app).sketch, added);
    ribbon::click_at(&mut app, glyph);
    assert!(!session(&app).selection.contains(&SketchSel::Constraint(relation)), "hidden glyph cannot be picked");
    frame(&mut app, "relations-hidden");
    open_menu(&mut app);
    app.get_by_label("Show all relations").click();
    app.run_steps(4);
    close_menu(&mut app);
    ribbon::click_at(&mut app, glyph);
    assert_eq!(session(&app).selection, [SketchSel::Constraint(relation)]);
    open_menu(&mut app);
    app.get_by_label("Show all relations").click();
    app.run_steps(4);
    assert!(session(&app).selection.is_empty(), "hiding clears a selected relation glyph");
    app.get_by_label("Show all relations").click();
    app.run_steps(4);
    close_menu(&mut app);

    // Each supported kind uses the same menu and model command. Reset between cases.
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    let entity = SketchSel::Entity;
    let point = |id, index| SketchSel::Point(PointRef::new(id, index));
    for (name, selection) in [
        ("Horizontal", vec![entity(a)]),
        ("Vertical", vec![entity(a)]),
        ("Parallel", vec![entity(a), entity(b)]),
        ("Perpendicular", vec![entity(a), entity(b)]),
        ("Coincident", vec![point(a, 0), point(b, 0)]),
        ("On line", vec![entity(p), entity(a)]),
        ("On curve", vec![entity(p), entity(c)]),
        ("Tangent", vec![entity(a), entity(c)]),
        ("Equal", vec![entity(a), entity(b)]),
        ("Concentric", vec![entity(c), entity(d)]),
        ("Midpoint", vec![entity(p), entity(a)]),
        ("Fix", vec![entity(p)]),
    ] {
        app.state_mut().state.sketch.as_mut().unwrap().selection = selection;
        app.run_steps(3);
        open_menu(&mut app);
        choose(&mut app, name);
        assert_eq!(session(&app).sketch.constraints.len(), before.constraints.len() + 1, "{name}");
        let expected = match name {
            "On line" | "On curve" => "On entity",
            "Fix" => "Fixed",
            name => name,
        };
        assert_eq!(session(&app).sketch.constraints.last().unwrap().name(), expected);
        assert!(
            matches!(
                session(&app).sketch.status,
                soliddraft_app::sketch::SolveStatus::UnderDefined | soliddraft_app::sketch::SolveStatus::FullyDefined
            ),
            "{name}: {:?}",
            session(&app).sketch.status
        );
        assert!(!glyphs(&app).is_empty(), "{name} has a visible glyph");
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        app.run_steps(6);
        assert_eq!(session(&app).sketch, before, "{name} is one undo step");
    }

    // The collapsed group still opens the complete list in a small dark window.
    app.set_size(vec2(960.0, 600.0));
    soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
    app.run_steps(12);
    open_menu(&mut app);
    for name in names {
        let control = ribbon::control(&app, name, menu_region(&app));
        assert!(ribbon::window(&app).contains_rect(control.rect), "{name} stays on screen");
    }
    frame(&mut app, "small-dark-menu");
    close_menu(&mut app);

    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    let saved = session(&app).sketch.clone();
    let path = dir.join("relations.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(12);
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, saved);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(6);
    assert!(session(&app).show_relations, "a new edit session starts with glyphs shown");
    soliddraft_app::file::set_config_dir(None);
}
