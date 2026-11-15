//! Broken sketch dimensions through the rendered app: a dimension the solver cannot hold,
//! one that names geometry the sketch no longer has, one whose label lies on the curve it
//! measures and one a named value drives are each picked by a click on the label or by a
//! box, shown as broken where they are, and removed by Delete as one undo step.

use egui::{Key, Modifiers, PointerButton, Pos2, Rect};
use soliddraft_app::document::{Command, Document, FeatureId, PlaneKind, SketchPlane};
use soliddraft_app::sketch::{
    Align, Constraint, ConstraintKind, DistanceKind, EntityKind, P2, PointRef, Sketch, SolveStatus, solve,
};
use soliddraft_app::sketcher::Frame;
use soliddraft_app::state::{AppAction, SketchSel, SketchSession};

#[path = "support/ui.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use ribbon::App;
use ux_tools::{Case, STANDARD, launch, settle};

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().expect("open sketch")
}

fn sketch(app: &App) -> &Sketch {
    &session(app).sketch
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    let at =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap();
    assert!(st.viewport.rect.contains(at), "{p:?} is off screen at {at:?}");
    at
}

/// Where a dimension's label is drawn.
fn label(app: &App, id: u32) -> Pos2 {
    screen(app, sketch(app).constraint(id).expect("the dimension").label().unwrap())
}

/// What the sketch draws and constrains, without what the solver derives from it.
fn content(s: &Sketch) -> (Vec<soliddraft_app::sketch::Entity>, Vec<Constraint>) {
    (s.entities.clone(), s.constraints.clone())
}

fn part(source: Sketch) -> (Document, FeatureId) {
    let mut doc = Document::new("Broken dimensions", soliddraft_app::kernel());
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch: source }).unwrap();
    (doc, id)
}

fn open(doc: Document, id: FeatureId) -> App {
    let mut app = launch(doc, STANDARD);
    app.state_mut().state.push(AppAction::EditSketch(id));
    settle(&mut app);
    app.key_press(Key::Z);
    settle(&mut app);
    app
}

fn click(app: &mut App, at: Pos2) {
    ribbon::render_steps::rendered(app, |app| ribbon::click_at(app, at));
    settle(app);
}

fn delete(app: &mut App) {
    app.key_press(Key::Delete);
    settle(app);
}

fn undo(app: &mut App) {
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(app);
}

fn redo(app: &mut App) {
    app.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    settle(app);
}

/// A press, a drag and a release in the viewport.
fn drag(app: &mut App, from: Pos2, to: Pos2) {
    ribbon::render_steps::rendered(app, |app| {
        app.hover_at(from);
        app.run_steps(3);
        let none = Modifiers::NONE;
        app.event(egui::Event::PointerButton {
            pos: from,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: none,
        });
        app.step();
        for i in 1..=10 {
            app.hover_at(from + (to - from) * (i as f32 / 10.0));
            app.step();
        }
        app.event(egui::Event::PointerButton {
            pos: to,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: none,
        });
        app.step();
        settle(app);
    });
}

/// How many pixels of the frame near `at` carry the error colour, as broken dimensions do.
fn error_pixels(app: &App, image: &image::RgbaImage, at: Pos2) -> usize {
    let ppp = app.ctx.pixels_per_point();
    let red = soliddraft_app::theme::error();
    let around = Rect::from_center_size(at, egui::vec2(40.0, 16.0));
    let mut count = 0;
    for y in (around.top() * ppp) as u32..(around.bottom() * ppp) as u32 {
        for x in (around.left() * ppp) as u32..(around.right() * ppp) as u32 {
            let p = image.get_pixel(x, y);
            let distance: u32 = [red.r(), red.g(), red.b()].iter().zip(p.0).map(|(a, b)| a.abs_diff(b) as u32).sum();
            count += usize::from(distance < 60);
        }
    }
    count
}

fn length(line: u32, value: f64, label: P2) -> ConstraintKind {
    ConstraintKind::Distance {
        a: PointRef::new(line, 0),
        b: PointRef::new(line, 1),
        kind: DistanceKind::Aligned,
        value,
        label,
    }
}

#[test]
fn a_label_on_its_curve_selects_the_dimension_and_delete_keeps_the_curve() {
    let mut case = Case::new("sketch-dimension-label-on-curve");
    let mut source = Sketch::new();
    let circle = source.add(EntityKind::Circle { center: P2::ZERO, radius: 10.0 }, false);
    // The label sits on the circle and on the line it measures, as a click that places
    // a dimension right on its curve leaves it.
    let diameter = source.constrain(ConstraintKind::Radius {
        entity: circle,
        value: 20.0,
        diameter: true,
        label: P2::new(10.0, 0.0),
    });
    let line = source.add(EntityKind::Line { a: P2::new(20.0, -10.0), b: P2::new(40.0, -10.0) }, false);
    let along = source.constrain(length(line, 20.0, P2::new(30.0, -10.0)));
    solve::solve(&mut source, None, true);
    let (doc, id) = part(source.clone());
    let mut app = open(doc, id);
    case.run(&mut app, |case, app| {
        for (name, dimension, curve) in [("diameter", diameter, circle), ("length", along, line)] {
            click(app, label(app, dimension));
            assert_eq!(session(app).selection, vec![SketchSel::Constraint(dimension)], "the {name} label selects it");
            case.frame(app, &format!("01-{name}-selected"));
            delete(app);
            assert!(sketch(app).constraint(dimension).is_none(), "Delete removed the {name} dimension");
            assert!(sketch(app).entity(curve).is_some(), "Delete kept the curve under the {name} label");
            case.frame(app, &format!("02-{name}-deleted"));
            undo(app);
            assert_eq!(content(sketch(app)), content(&source), "one undo brings the {name} dimension back");
        }

        // A drag that starts on the label moves the label, not the circle under it.
        let from = label(app, diameter);
        drag(app, from, from + egui::vec2(40.0, -30.0));
        let EntityKind::Circle { center, radius } = sketch(app).entity(circle).unwrap().kind else { panic!("circle") };
        assert_eq!((center, radius), (P2::ZERO, 10.0), "the circle stayed where it was");
        let moved = sketch(app).constraint(diameter).unwrap().label().unwrap();
        assert!(moved.dist(P2::new(10.0, 0.0)) > 2.0, "the label moved to {moved:?}");
        case.frame(app, "03-label-dragged-off-the-circle");
    });
}

#[test]
fn a_conflicting_dimension_is_red_and_deletes_by_click_or_box() {
    let mut case = Case::new("sketch-dimension-conflicting");
    let mut source = Sketch::new();
    let line = source.add(EntityKind::Line { a: P2::ZERO, b: P2::new(20.0, 0.0) }, false);
    source.constrain(ConstraintKind::Coincident { a: PointRef::new(line, 0), b: PointRef::ORIGIN });
    source.constrain(ConstraintKind::Horizontal { target: Align::Line(line) });
    let first = source.constrain(length(line, 20.0, P2::new(10.0, 8.0)));
    let second = source.constrain(length(line, 30.0, P2::new(10.0, -8.0)));
    solve::solve(&mut source, None, true);
    assert!(matches!(source.status, SolveStatus::OverDefined { .. }), "{:?}", source.status);
    let (doc, id) = part(source);
    let mut app = open(doc, id);
    case.run(&mut app, |case, app| {
        let before = sketch(app).clone();
        let frame = case.frame(app, "01-conflicting");
        let red = error_pixels(app, &frame, label(app, second));
        assert!(red > 10, "the conflicting label is drawn in the error colour ({red} pixels)");

        click(app, label(app, second));
        assert_eq!(session(app).selection, vec![SketchSel::Constraint(second)]);
        delete(app);
        assert!(sketch(app).constraint(second).is_none());
        assert!(sketch(app).constraint(first).is_some());
        assert_eq!(sketch(app).status, SolveStatus::FullyDefined, "the rest of the sketch holds again");
        let committed = &app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch;
        assert!(committed.constraint(second).is_none(), "the document has the deletion, not only the editor");
        case.frame(app, "02-deleted-by-click");
        undo(app);
        assert_eq!(content(sketch(app)), content(&before), "one undo brings the conflict back");
        redo(app);
        assert!(sketch(app).constraint(second).is_none(), "redo deletes it again");
        undo(app);

        // A window around the label alone takes the dimension and nothing else.
        let at = label(app, second);
        drag(app, at - egui::vec2(40.0, 16.0), at + egui::vec2(40.0, 16.0));
        assert_eq!(session(app).selection, vec![SketchSel::Constraint(second)]);
        delete(app);
        assert!(sketch(app).constraint(second).is_none());
        assert_eq!(sketch(app).drawn().count(), 1, "the line stays");
        case.frame(app, "03-deleted-by-box");
    });
}

#[test]
fn a_dangling_dimension_is_red_and_deletes() {
    let mut case = Case::new("sketch-dimension-dangling");
    let mut source = Sketch::new();
    let line = source.add(EntityKind::Line { a: P2::new(-20.0, 0.0), b: P2::new(20.0, 0.0) }, false);
    let healthy = source.constrain(length(line, 40.0, P2::new(0.0, 8.0)));
    // A file can hold a dimension whose geometry is gone: entity 99 never existed here.
    let dangling = source.constrain(ConstraintKind::Distance {
        a: PointRef::new(line, 1),
        b: PointRef::new(99, 0),
        kind: DistanceKind::Aligned,
        value: 15.0,
        label: P2::new(0.0, -8.0),
    });
    solve::solve(&mut source, None, true);
    let (doc, id) = part(source);
    let mut app = open(doc, id);
    case.run(&mut app, |case, app| {
        assert!(sketch(app).constraint(dangling).unwrap().is_dangling(sketch(app)));
        let frame = case.frame(app, "01-dangling");
        let broken = error_pixels(app, &frame, label(app, dangling));
        let sound = error_pixels(app, &frame, label(app, healthy));
        assert!(broken > 10 && sound == 0, "only the dangling label is red: {broken} and {sound} pixels");

        click(app, label(app, dangling));
        assert_eq!(session(app).selection, vec![SketchSel::Constraint(dangling)]);
        delete(app);
        assert!(sketch(app).constraint(dangling).is_none());
        assert!(sketch(app).constraint(healthy).is_some());
        case.frame(app, "02-deleted");
        undo(app);
        assert!(sketch(app).constraint(dangling).is_some(), "undo restores the dimension as it was");
    });
}

#[test]
fn a_dimension_a_value_drives_deletes_with_its_drive() {
    let mut case = Case::new("sketch-dimension-linked");
    let mut source = Sketch::new();
    let mut dimensions = Vec::new();
    let mut circles = Vec::new();
    for (x, radius) in [(-12.0, 5.0), (12.0, 7.0)] {
        let entity = source.add(EntityKind::Circle { center: P2::new(x, 0.0), radius }, false);
        circles.push(entity);
        dimensions.push(source.constrain(ConstraintKind::Radius {
            entity,
            value: radius * 2.0,
            diameter: true,
            label: P2::new(x, -12.0),
        }));
    }
    solve::solve(&mut source, None, true);
    let (mut doc, id) = part(source);
    doc.apply(Command::LinkSketchValues { id, constraints: dimensions.clone(), name: "Span".into(), value: None })
        .unwrap();
    let mut app = open(doc, id);
    case.run(&mut app, |case, app| {
        let links = |app: &App| soliddraft_app::sketch_values::links(app.state().state.doc.model(), id);
        assert_eq!(links(app).len(), 2);
        let before = sketch(app).clone();
        case.frame(app, "01-linked");

        click(app, label(app, dimensions[1]));
        assert_eq!(session(app).selection, vec![SketchSel::Constraint(dimensions[1])]);
        delete(app);
        assert!(sketch(app).constraint(dimensions[1]).is_none(), "the linked dimension is gone");
        assert_eq!(links(app).keys().copied().collect::<Vec<_>>(), vec![dimensions[0]], "the other keeps Span");
        case.frame(app, "02-deleted");
        undo(app);
        assert_eq!(content(sketch(app)), content(&before));
        assert_eq!(links(app).len(), 2, "one undo restores the dimension and its link");

        // Deleting the circle takes its linked dimension along.
        click(app, screen(app, P2::new(-12.0, 5.0)));
        assert_eq!(session(app).selection, vec![SketchSel::Entity(circles[0])]);
        delete(app);
        assert!(sketch(app).entity(circles[0]).is_none());
        assert!(sketch(app).constraint(dimensions[0]).is_none());
        assert_eq!(links(app).keys().copied().collect::<Vec<_>>(), vec![dimensions[1]]);
        case.frame(app, "03-circle-deleted");
    });
}

/// Double-clicking a dimension opens its value with the old number selected, so the
/// digits typed next replace it. They used to land after it: 120 typed over 40 read
/// "40120", and Enter kept 40.
#[test]
fn typing_over_an_opened_dimension_replaces_its_value() {
    let mut case = Case::new("sketch-dimension-type-over");
    let mut source = Sketch::new();
    let line = source.add(EntityKind::Line { a: P2::ZERO, b: P2::new(40.0, 0.0) }, false);
    source.constrain(ConstraintKind::Coincident { a: PointRef::new(line, 0), b: PointRef::ORIGIN });
    source.constrain(ConstraintKind::Horizontal { target: Align::Line(line) });
    let along = source.constrain(length(line, 40.0, P2::new(20.0, 12.0)));
    solve::solve(&mut source, None, true);
    let (doc, id) = part(source);
    let mut app = open(doc, id);
    case.run(&mut app, |case, app| {
        let at = label(app, along);
        ribbon::render_steps::rendered(app, |app| ribbon::double_click_at(app, at));
        app.run_steps(4);
        assert!(session(app).dim_edit.is_some(), "the double-click opens the value");
        case.frame(app, "01-value-opened-selected");
        // The keys go where a user's would: to whatever holds the keyboard.
        app.event(egui::Event::Text("25".into()));
        app.run_steps(2);
        assert_eq!(session(app).dim_edit.as_ref().map(|e| e.text.as_str()), Some("25"), "the typed digits replace 40");
        case.frame(app, "02-typed-over");
        app.key_press(Key::Enter);
        settle(app);
        assert!(session(app).dim_edit.is_none(), "Enter applies and closes the value");
        let ConstraintKind::Distance { value, .. } = sketch(app).constraint(along).unwrap().kind else {
            panic!("a distance")
        };
        assert_eq!(value, 25.0, "the dimension takes the typed value");
        let EntityKind::Line { a, b } = sketch(app).entity(line).unwrap().kind else { panic!("a line") };
        assert!((a.dist(b) - 25.0).abs() < 1e-6, "the line is {} mm long", a.dist(b));
        case.frame(app, "03-applied");
    });
}

/// The pill in the viewport's corner tells a part's user to hover the model. In an open
/// sketch nothing of the model can be hovered and the sketch has its own status line, so
/// the pill used to give advice that did nothing there.
#[test]
fn the_model_hover_hint_is_not_shown_in_an_open_sketch() {
    let mut case = Case::new("sketch-no-model-hint");
    let mut source = Sketch::new();
    source.add(EntityKind::Line { a: P2::ZERO, b: P2::new(40.0, 0.0) }, false);
    let (doc, id) = part(source);
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        let hint = |app: &App| ribbon::painted(app).iter().any(|p| p.text == "Hover a face, edge or vertex");
        ux_tools::pointer_to_chrome(app);
        assert!(hint(app), "a part with nothing under the pointer shows the hint");
        app.state_mut().state.push(AppAction::EditSketch(id));
        settle(app);
        ux_tools::pointer_to_chrome(app);
        assert!(!hint(app), "an open sketch does not");
        case.frame(app, "01-open-sketch-without-the-model-hint");
    });
}
