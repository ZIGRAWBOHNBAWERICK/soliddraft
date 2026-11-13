//! Relation inspection through property rows and canvas glyphs.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2, Rect, Shape, pos2, vec2};
use egui_kittest::kittest::{By, Queryable};
use ribbon::App;
use soliddraft_app::{
    document::{Command, FeatureId, PlaneKind, SketchPlane},
    sketch::{
        Align, ConstraintKind, DistanceKind, EntityId, EntityKind, ORIGIN, P2, PointRef, Sketch, SolveStatus,
        fmt_value, solve,
    },
    sketcher::Frame,
    state::{AppAction, RelationFilter, SketchSel, SketchSession},
    theme,
};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap()
}

fn launch(mut sketch: Sketch) -> (App, FeatureId) {
    let dir = proof_dir().join("config");
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir));
    solve::solve(&mut sketch, None, true);
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app(doc);
    theme::set_mode(&app.ctx, theme::Mode::Light);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    (app, id)
}

fn proof_dir() -> std::path::PathBuf {
    std::env::var_os("SOLIDDRAFT_RELATION_INSPECTION_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-relation-inspection-{}", std::process::id())))
}

fn frame(app: &mut App, name: &str) {
    if std::env::var_os("SOLIDDRAFT_RELATION_INSPECTION_PROOF_DIR").is_some() {
        std::fs::create_dir_all(proof_dir()).unwrap();
        app.render().unwrap().save(proof_dir().join(format!("{name}.png"))).unwrap();
    }
}

fn click(app: &mut App, at: Pos2, modifiers: Modifiers) {
    app.hover_at(at);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event_modifiers(
            egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers },
            modifiers,
        );
        app.step();
    }
    app.run_steps(6);
}

fn filter_button(app: &mut App) -> (String, Rect) {
    for _ in 0..6 {
        let painted = ribbon::painted(app);
        let left = app.state().state.viewport.rect.left();
        if let Some(head) = painted.iter().find(|p| p.text == "Existing relations" && p.rect.right() < left)
            && let Some(current) = painted.iter().find(|p| {
                ["Selected", "All", "Conflicting", "Dangling"].contains(&p.text.as_str())
                    && p.full.top() >= head.full.bottom()
                    && p.rect.right() < left
                    && p.rect.contains_rect(p.full)
            })
        {
            return (current.text.clone(), current.rect);
        }
        let head = ribbon::painted(app).into_iter().find(|p| p.text == "Properties").unwrap().rect;
        app.hover_at(pos2(left - 15.0, (head.bottom() + ribbon::band(app).top()) / 2.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -160.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        app.run_steps(10);
    }
    panic!("relation scope filters must be available without selecting geometry");
}

fn filter(app: &mut App, wanted: &str) {
    let (current, rect) = filter_button(app);
    if current == wanted {
        return;
    }
    ribbon::click_at(app, rect.center());
    let choice = ribbon::painted(app)
        .into_iter()
        .find(|p| p.text == wanted && p.rect.contains_rect(p.full))
        .unwrap_or_else(|| panic!("relation filter must offer {wanted}"));
    ribbon::click_at(app, choice.rect.center());
}

#[test]
fn whole_sketch_relations_can_be_inspected_without_selecting_geometry() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let a = sketch.add(EntityKind::Line { a: P2::new(-20.0, 0.0), b: P2::new(20.0, 0.0) }, false);
    let b = sketch.add(EntityKind::Line { a: P2::new(30.0, -10.0), b: P2::new(30.0, 10.0) }, false);
    let horizontal = sketch.constrain(ConstraintKind::Horizontal { target: Align::Line(a) });
    sketch.constrain(ConstraintKind::Vertical { target: Align::Line(b) });
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    filter(&mut app, "All");
    assert!(session(&app).selection.is_empty());
    assert_eq!(session(&app).sketch, before);
    assert_eq!(app.state().state.doc.revision(), revision);
    inspect(&mut app, "Horizontal");
    assert_eq!(session(&app).selection, [SketchSel::Constraint(horizontal)]);
    assert!(halo(&app, a));
    assert!(!halo(&app, b));
    filter(&mut app, "Selected");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "1 of 2 shown"));
    filter(&mut app, "All");
    ribbon::run_command(&mut app, "relations");
    app.get_by_label("Show all relations").click();
    app.run_steps(6);
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert!(!session(&app).show_relations);
    inspect(&mut app, "Horizontal");
    assert_eq!(session(&app).selection, [SketchSel::Constraint(horizontal)]);
    frame(&mut app, "filter-all-hidden-relations");
    app.key_press(Key::Delete);
    app.run_steps(6);
    assert!(session(&app).sketch.constraint(horizontal).is_none());
    assert_eq!(session(&app).sketch.drawn().count(), before.drawn().count());
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn conflict_filter_locates_solver_relations_and_row_deletion_resolves_them() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let a = P2::new(-20.0, -10.0);
    let b = P2::new(20.0, -10.0);
    let line = sketch.add(EntityKind::Line { a, b }, false);
    sketch.constrain(ConstraintKind::Fix { point: PointRef::new(line, 0), at: a });
    sketch.constrain(ConstraintKind::Fix { point: PointRef::new(line, 1), at: b });
    sketch.constrain(ConstraintKind::Vertical { target: Align::Line(line) });
    let other = sketch.add(EntityKind::Line { a: P2::new(-20.0, 20.0), b: P2::new(20.0, 20.0) }, false);
    sketch.constrain(ConstraintKind::Horizontal { target: Align::Line(other) });
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    let SolveStatus::OverDefined { conflicting } = &before.status else { panic!("the fixture must conflict") };
    let chosen = before.constraints.iter().find(|c| conflicting.contains(&c.id)).unwrap().clone();
    let revision = app.state().state.doc.revision();
    let window = ribbon::window(&app);
    ribbon::click_button(&mut app, "Inspect conflicts", window);
    assert_eq!(session(&app).relations_filter, RelationFilter::Conflicting);
    assert!(session(&app).selection.is_empty());
    assert_eq!(app.state().state.doc.revision(), revision);
    inspect(&mut app, chosen.name());
    assert_eq!(session(&app).selection, [SketchSel::Constraint(chosen.id)]);
    assert!(halo(&app, line));
    assert!(!halo(&app, other));
    let top = panel_heading(&app, "Existing relations").bottom();
    let row = ribbon::painted(&app)
        .into_iter()
        .find(|p| {
            p.text == chosen.name()
                && p.full.top() >= top
                && p.rect.right() < app.state().state.viewport.rect.left()
                && p.rect.contains_rect(p.full)
        })
        .unwrap()
        .rect;
    let remove = app
        .query_all(By::new().role(egui::accesskit::Role::Button).label("×"))
        .find(|n| {
            (n.rect().center().y - row.center().y).abs() < 8.0
                && n.rect().right() < app.state().state.viewport.rect.left()
        })
        .unwrap()
        .rect();
    frame(&mut app, "filter-conflict-owner");
    ribbon::click_at(&mut app, remove.center());
    assert!(session(&app).sketch.constraint(chosen.id).is_none());
    assert!(!matches!(session(&app).sketch.status, SolveStatus::OverDefined { .. }));
    assert_eq!(session(&app).sketch.drawn().count(), before.drawn().count());
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "No conflicting relations."));
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    assert_eq!(session(&app).relations_filter, RelationFilter::Conflicting);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert!(session(&app).sketch.constraint(chosen.id).is_none());
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn dangling_filter_recovers_missing_entities_and_points_after_reopen() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let line = sketch.add(EntityKind::Line { a: P2::new(-20.0, 0.0), b: P2::new(20.0, 0.0) }, false);
    let valid = sketch.constrain(ConstraintKind::Horizontal { target: Align::Line(line) });
    let missing = sketch.constrain(ConstraintKind::Horizontal { target: Align::Line(999) });
    let point = sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(line, 9), b: PointRef::new(ORIGIN, 0) });
    let (mut app, id) = launch(sketch);
    filter(&mut app, "Dangling");
    assert_eq!(session(&app).relations_filter, RelationFilter::Dangling);
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "2 of 3 shown"));
    frame(&mut app, "filter-dangling");
    let path = proof_dir().join("dangling.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(10);
    assert_eq!(session(&app).relations_filter, RelationFilter::Selected, "filters are session-local");
    filter(&mut app, "Dangling");
    inspect(&mut app, "Coincident");
    assert_eq!(session(&app).selection, [SketchSel::Constraint(point)]);
    assert!(halo(&app, line));
    app.key_press(Key::Delete);
    app.run_steps(6);
    inspect(&mut app, "Horizontal");
    assert_eq!(session(&app).selection, [SketchSel::Constraint(missing)]);
    app.key_press(Key::Delete);
    app.run_steps(6);
    assert_eq!(session(&app).sketch.constraints.iter().map(|c| c.id).collect::<Vec<_>>(), [valid]);
    assert_eq!(session(&app).sketch.drawn().count(), 1);
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "No dangling references."));
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn empty_filter_states_and_popup_escape_do_not_leave_the_sketch() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (mut app, _) = launch(Sketch::new());
    app.set_size(vec2(960.0, 600.0));
    theme::set_mode(&app.ctx, theme::Mode::Dark);
    app.run_steps(12);
    let revision = app.state().state.doc.revision();
    for (scope, hint) in [
        ("All", "No stored relations in this sketch."),
        ("Conflicting", "No conflicting relations."),
        ("Dangling", "No dangling references."),
    ] {
        filter(&mut app, scope);
        frame(&mut app, &format!("filter-empty-{scope}-baseline"));
        assert!(ribbon::painted(&app).iter().any(|p| p.text == hint), "{scope} empty state must be visible");
    }
    let (_, rect) = filter_button(&mut app);
    ribbon::click_at(&mut app, rect.center());
    frame(&mut app, "filter-popup-small-dark");
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert!(app.state().state.sketch.is_some(), "Escape must close only the relation filter popup");
    assert_eq!(session(&app).relations_filter, RelationFilter::Dangling);
    assert_eq!(app.state().state.doc.revision(), revision);
    assert!(session(&app).selection.is_empty());
    frame(&mut app, "filter-empty-small-dark");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

fn panel_heading(app: &App, name: &str) -> Rect {
    ribbon::painted(app)
        .into_iter()
        .find(|t| t.text == name && t.rect.right() < app.state().state.viewport.rect.left())
        .unwrap_or_else(|| panic!("property panel must show {name}"))
        .rect
}

fn inspect(app: &mut App, label: &str) {
    for _ in 0..4 {
        let text = ribbon::painted(app);
        let left = app.state().state.viewport.rect.left();
        let top =
            text.iter().find(|t| t.text == "Existing relations" && t.rect.right() < left).map(|t| t.rect.bottom());
        let bottom = text
            .iter()
            .find(|t| t.text == "Add relation" && t.rect.right() < left)
            .map_or(ribbon::window(app).bottom(), |t| t.rect.top());
        if let Some(top) = top
            && let Some(row) = text.iter().find(|t| {
                t.text == label
                    && t.full.top() >= top
                    && t.full.bottom() <= bottom
                    && t.rect.right() < left
                    && t.rect.contains_rect(t.full)
            })
        {
            ribbon::click_at(app, row.rect.center());
            return;
        }
        let head = panel_heading(app, "Properties");
        app.hover_at(pos2(left - 15.0, (head.bottom() + ribbon::band(app).top()) / 2.0));
        app.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -180.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        app.run_steps(12);
    }
    panic!("existing relation {label} must be listed and reachable by scrolling");
}

fn curve_strokes(app: &App, id: EntityId) -> Vec<(f32, egui::Color32)> {
    fn collect(shape: &Shape, expected: &[Pos2], out: &mut Vec<(f32, egui::Color32)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, expected, out)),
            Shape::Path(p)
                if p.points.len() == expected.len()
                    && p.points.iter().zip(expected).all(|(a, b)| a.distance(*b) < 0.1) =>
            {
                if let egui::epaint::ColorMode::Solid(color) = p.stroke.color {
                    out.push((p.stroke.width, color));
                }
            }
            _ => {}
        }
    }
    let expected: Vec<_> =
        session(app).sketch.entity(id).unwrap().polyline(96).into_iter().map(|p| screen(app, p)).collect();
    assert!(expected.iter().all(|p| app.state().state.viewport.rect.contains(*p)), "the owner is visible");
    let mut out = Vec::new();
    for clipped in &app.output().shapes {
        collect(&clipped.shape, &expected, &mut out);
    }
    out
}

fn halo(app: &App, id: EntityId) -> bool {
    curve_strokes(app, id).iter().any(|(width, color)| (*width - 6.0).abs() < 0.01 && *color == theme::selection_bg())
}

fn point_halo(app: &App, point: PointRef) -> bool {
    fn contains(shape: &Shape, at: Pos2) -> bool {
        match shape {
            Shape::Vec(shapes) => shapes.iter().any(|s| contains(s, at)),
            Shape::Circle(c) => c.center.distance(at) < 0.1 && c.radius == 7.0 && c.fill == theme::selection_bg(),
            _ => false,
        }
    }
    let at = screen(app, session(app).sketch.point(point).unwrap());
    assert!(
        app.state().state.viewport.rect.contains(at),
        "the referenced point is visible, not merely painted outside the clip"
    );
    app.output().shapes.iter().any(|s| contains(&s.shape, at))
}

fn glyph_near(app: &App, p: P2) -> Pos2 {
    fn collect(shape: &Shape, viewport: Rect, out: &mut Vec<Rect>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, viewport, out)),
            Shape::Rect(r)
                if r.rect.size() == vec2(14.0, 14.0)
                    && r.corner_radius == egui::CornerRadius::same(3)
                    && viewport.contains_rect(r.rect) =>
            {
                out.push(r.rect)
            }
            _ => {}
        }
    }
    let mut glyphs = Vec::new();
    for shape in &app.output().shapes {
        collect(&shape.shape, app.state().state.viewport.rect, &mut glyphs);
    }
    let at = screen(app, p);
    let glyph = glyphs.into_iter().min_by(|a, b| a.center().distance(at).total_cmp(&b.center().distance(at))).unwrap();
    assert!(glyph.center().distance(at) < 30.0);
    glyph.center()
}

#[test]
fn relation_rows_and_glyphs_highlight_owners_without_selecting_geometry() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let a = sketch.add(EntityKind::Line { a: P2::new(-35.0, -20.0), b: P2::new(-10.0, -20.0) }, false);
    let b = sketch.add(EntityKind::Line { a: P2::new(5.0, 5.0), b: P2::new(30.0, 5.0) }, false);
    let unrelated = sketch.add(EntityKind::Line { a: P2::new(-30.0, 25.0), b: P2::new(-5.0, 30.0) }, false);
    let horizontal = sketch.constrain(ConstraintKind::Horizontal { target: Align::Line(a) });
    let parallel = sketch.constrain(ConstraintKind::Parallel { a, b });
    let (mut app, _) = launch(sketch);
    let before = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    let at = screen(&app, P2::new(-30.0, -20.0));
    click(&mut app, at, Modifiers::NONE);
    assert_eq!(session(&app).selection, vec![SketchSel::Entity(a)]);
    assert!(halo(&app, a), "the existing geometry-selection halo is detectable");
    frame(&mut app, "entity-selected");
    let glyph = glyph_near(&app, session(&app).sketch.entity(a).unwrap().midpoint());
    click(&mut app, glyph, Modifiers::NONE);
    assert_eq!(session(&app).selection, vec![SketchSel::Constraint(horizontal)]);
    assert!(session(&app).selected_entities().is_empty());
    frame(&mut app, "single-owner");
    assert!(halo(&app, a), "a selected relation must highlight its owner");
    assert!(!halo(&app, b));
    assert!(!halo(&app, unrelated));
    inspect(&mut app, "Parallel");
    assert_eq!(session(&app).selection, vec![SketchSel::Constraint(parallel)]);
    assert!(halo(&app, a) && halo(&app, b));
    assert!(!halo(&app, unrelated));
    inspect(&mut app, "Horizontal");
    let glyph = glyph_near(&app, session(&app).sketch.entity(b).unwrap().midpoint());
    click(&mut app, glyph, Modifiers::CTRL);
    assert!(session(&app).selection.contains(&SketchSel::Constraint(horizontal)));
    assert!(session(&app).selection.contains(&SketchSel::Constraint(parallel)));
    assert!(halo(&app, a) && halo(&app, b));
    click(&mut app, glyph, Modifiers::NONE);
    assert_eq!(session(&app).selection, vec![SketchSel::Constraint(parallel)]);
    assert_eq!(session(&app).sketch, before);
    assert_eq!(app.state().state.doc.revision(), revision, "inspection is not an edit");
    frame(&mut app, "two-owners-light");
    app.set_size(vec2(960.0, 600.0));
    theme::set_mode(&app.ctx, theme::Mode::Dark);
    app.run_steps(12);
    inspect(&mut app, "Parallel");
    assert!(halo(&app, a) && halo(&app, b));
    frame(&mut app, "two-owners-small-dark");
    app.set_size(vec2(1440.0, 900.0));
    theme::set_mode(&app.ctx, theme::Mode::Light);
    app.run_steps(12);
    let window = ribbon::window(&app);
    if app.query_all(By::new().role(egui::accesskit::Role::Button).label("Relations")).next().is_none() {
        ribbon::click_button(&mut app, "Constrain group", window);
    }
    ribbon::click_button(&mut app, "Relations", window);
    app.get_by_label("Show all relations").click();
    app.run_steps(6);
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert!(!session(&app).show_relations);
    let at = screen(&app, P2::new(-30.0, -20.0));
    click(&mut app, at, Modifiers::NONE);
    inspect(&mut app, "Parallel");
    assert!(halo(&app, a) && halo(&app, b), "hidden glyphs still allow inspection through the property panel");
    app.key_press(Key::Delete);
    app.run_steps(8);
    assert!(session(&app).sketch.constraint(parallel).is_none());
    assert!(session(&app).sketch.constraint(horizontal).is_some());
    assert_eq!(session(&app).sketch.entities, before.entities, "Delete removes only the chosen relation");
    assert!(!halo(&app, a) && !halo(&app, b));
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    assert!(session(&app).sketch.constraint(parallel).is_none());
    assert_eq!(session(&app).sketch.entities, before.entities);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn relation_inspection_includes_selected_points_and_the_origin() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let point = sketch.add(EntityKind::Point { p: P2::new(10.0, 10.0) }, false);
    let guide = sketch.add(EntityKind::Line { a: P2::new(-10.0, -20.0), b: P2::new(30.0, -20.0) }, true);
    let relation = sketch.constrain(ConstraintKind::Distance {
        a: PointRef::new(point, 0),
        b: PointRef::new(ORIGIN, 0),
        kind: DistanceKind::Aligned,
        value: 200.0_f64.sqrt(),
        label: P2::new(25.0, -15.0),
    });
    let (mut app, id) = launch(sketch);
    let before = session(&app).sketch.clone();
    let c = before.constraint(relation).unwrap();
    let name = format!("{} {}", c.name(), fmt_value(c.value().unwrap()));
    let at = screen(&app, before.point(PointRef::new(point, 0)).unwrap());
    click(&mut app, at, Modifiers::NONE);
    assert_eq!(session(&app).selection, vec![SketchSel::Point(PointRef::new(point, 0))]);
    inspect(&mut app, &name);
    assert_eq!(session(&app).selection, vec![SketchSel::Constraint(relation)]);
    assert!(point_halo(&app, PointRef::new(point, 0)));
    assert!(point_halo(&app, PointRef::new(ORIGIN, 0)), "the fixed origin is also an owner");
    assert!(!halo(&app, guide));
    assert!(session(&app).selected_points().is_empty() && session(&app).selected_entities().is_empty());
    frame(&mut app, "point-and-origin");
    let top = ribbon::painted(&app).into_iter().find(|t| t.text.starts_with("Selection (")).unwrap().rect.bottom();
    let bottom = panel_heading(&app, "Existing relations").top();
    let row = Rect::from_min_max(pos2(0.0, top), pos2(app.state().state.viewport.rect.left(), bottom));
    ribbon::click_button(&mut app, "×", row);
    assert!(session(&app).sketch.constraint(relation).is_none());
    assert_eq!(session(&app).sketch.entities, before.entities);
    assert!(!point_halo(&app, PointRef::new(ORIGIN, 0)));
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    let saved = session(&app).sketch.clone();
    let path = proof_dir().join("point-inspection.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, saved);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn relation_owner_highlight_keeps_definition_and_conflict_colours() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let a = P2::new(-20.0, -10.0);
    let b = P2::new(20.0, -10.0);
    let line = sketch.add(EntityKind::Line { a, b }, false);
    let fixed = sketch.constrain(ConstraintKind::Fix { point: PointRef::new(line, 0), at: a });
    sketch.constrain(ConstraintKind::Fix { point: PointRef::new(line, 1), at: b });
    let vertical = sketch.constrain(ConstraintKind::Vertical { target: Align::Line(line) });
    let (mut app, _) = launch(sketch);
    assert!(matches!(session(&app).sketch.status, SolveStatus::OverDefined { .. }));
    frame(&mut app, "conflict-before-selection");
    let EntityKind::Line { a, b } = session(&app).sketch.entity(line).unwrap().kind else { unreachable!() };
    let at = screen(&app, a.lerp(b, 0.25));
    assert!(app.state().state.viewport.rect.contains(at), "the conflict target is visible");
    click(&mut app, at, Modifiers::NONE);
    assert_eq!(session(&app).selection, vec![SketchSel::Entity(line)]);
    inspect(&mut app, "Vertical");
    assert_eq!(session(&app).selection, vec![SketchSel::Constraint(vertical)]);
    assert!(halo(&app, line));
    assert!(
        curve_strokes(&app, line).iter().any(|(width, color)| (*width - 1.8).abs() < 0.01 && *color == theme::error()),
        "inspection must not hide the conflict colour"
    );
    frame(&mut app, "conflicting-owner");
    app.key_press(Key::Delete);
    app.run_steps(8);
    assert!(matches!(session(&app).sketch.status, SolveStatus::FullyDefined));
    app.key_press(Key::Z);
    app.run_steps(30);
    let glyph = glyph_near(&app, session(&app).sketch.entity(line).unwrap().point(0).unwrap());
    click(&mut app, glyph, Modifiers::NONE);
    assert_eq!(session(&app).selection, vec![SketchSel::Constraint(fixed)]);
    assert!(halo(&app, line));
    assert!(
        curve_strokes(&app, line).iter().any(|(width, color)| (*width - 1.8).abs() < 0.01 && *color == theme::ink()),
        "inspection must not recolour fully defined geometry"
    );
    frame(&mut app, "fully-defined-owner");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
