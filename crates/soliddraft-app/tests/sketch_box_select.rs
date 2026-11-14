//! Selecting sketch lines through the rendered app: box and crossing selection as in
//! SolidWorks, Delete on what the box took, a hover tint that shows, and a shown sketch
//! picked by its line outside sketch editing.

use egui::{Key, Modifiers, PointerButton, Pos2};
use soliddraft_app::document::{Command, Document, FeatureId, PlaneKind, Selection, SketchPlane};
use soliddraft_app::sketch::{ConstraintKind, DistanceKind, EntityKind, P2, PointRef, Sketch};
use soliddraft_app::sketcher::Frame;
use soliddraft_app::sketcher::select_box::BoxKind;
use soliddraft_app::state::{AppAction, SketchSel, SketchSession};

#[path = "support/ui.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use ribbon::App;
use ux_tools::{Case, STANDARD, launch, settle};

const LINE: SketchSel = SketchSel::Entity(1);
const CROSSING_LINE: SketchSel = SketchSel::Entity(2);
const CIRCLE: SketchSel = SketchSel::Entity(3);

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().expect("open sketch")
}

/// A line with its length dimension, a vertical line to the right of it and a circle.
fn document() -> (Document, FeatureId, Sketch) {
    let mut doc = Document::new("Box selection", soliddraft_app::kernel());
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut sketch = Sketch::new();
    let line = sketch.add(EntityKind::Line { a: P2::new(-30.0, 5.0), b: P2::new(-10.0, 5.0) }, false);
    sketch.add(EntityKind::Line { a: P2::new(-5.0, -15.0), b: P2::new(-5.0, 15.0) }, false);
    sketch.add(EntityKind::Circle { center: P2::new(25.0, 0.0), radius: 5.0 }, false);
    sketch.constrain(ConstraintKind::Distance {
        a: PointRef::new(line, 0),
        b: PointRef::new(line, 1),
        kind: DistanceKind::Aligned,
        value: 20.0,
        label: P2::new(-20.0, 9.0),
    });
    doc.apply(Command::EditSketch { id, sketch: sketch.clone() }).unwrap();
    (doc, id, sketch)
}

fn dimension(app: &App) -> SketchSel {
    let c = session(app).sketch.constraints.iter().find(|c| c.is_dimension()).expect("the length dimension");
    SketchSel::Constraint(c.id)
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    let at =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap();
    assert!(st.viewport.rect.contains(at), "{p:?} is off screen at {at:?}");
    at
}

/// A press at `from`, a drag to `to` and a release, with `modifiers` held throughout.
/// `during` sees the app with the button still down.
fn drag(app: &mut App, from: Pos2, to: Pos2, modifiers: Modifiers, during: impl FnOnce(&mut App)) {
    ribbon::render_steps::rendered(app, |app| {
        app.event(egui::Event::ModifiersChanged(modifiers));
        app.hover_at(from);
        app.run_steps(3);
        app.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers });
        app.step();
        for i in 1..=10 {
            app.hover_at(from + (to - from) * (i as f32 / 10.0));
            app.step();
        }
        during(app);
        app.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers });
        app.step();
        app.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        settle(app);
    });
}

/// What the sketch draws and constrains, without what the solver derives from it.
fn content(s: &Sketch) -> (Vec<soliddraft_app::sketch::Entity>, Vec<soliddraft_app::sketch::Constraint>) {
    (s.entities.clone(), s.constraints.clone())
}

fn selection(app: &App) -> Vec<SketchSel> {
    session(app).selection.clone()
}

/// The screen corners of a plane-space box, as (left top, right bottom).
fn corners(app: &App, min: P2, max: P2) -> (Pos2, Pos2) {
    let (a, b) = (screen(app, P2::new(min.u, max.v)), screen(app, P2::new(max.u, min.v)));
    (Pos2::new(a.x.min(b.x), a.y.min(b.y)), Pos2::new(a.x.max(b.x), a.y.max(b.y)))
}

fn open(case: &mut Case) -> (App, Sketch) {
    let (doc, id, sketch) = document();
    let mut app = launch(doc, STANDARD);
    app.state_mut().state.push(AppAction::EditSketch(id));
    settle(&mut app);
    app.key_press(Key::Z);
    settle(&mut app);
    case.note(format!("viewport {:?}", app.state().state.viewport.rect));
    (app, sketch)
}

#[test]
fn a_box_selects_inside_left_to_right_and_crossing_right_to_left() {
    let mut case = Case::new("sketch-box-select");
    let (mut app, source) = open(&mut case);
    case.run(&mut app, |case, app| {
        let dim = dimension(app);
        let (left_top, right_bottom) = corners(app, P2::new(-35.0, -2.0), P2::new(-3.0, 13.0));

        // Left to right: a solid window that takes the line and its dimension, which lie
        // wholly inside, and leaves the vertical line it only crosses.
        drag(app, left_top, right_bottom, Modifiers::NONE, |app| {
            let b = session(app).selection_box.clone().expect("a drag from empty space draws a box");
            assert_eq!(b.kind(), BoxKind::Window);
            assert_eq!(b.picks, vec![LINE, dim], "the window previews what it will take");
            case.frame(app, "01-window-dragging");
        });
        assert!(session(app).selection_box.is_none());
        assert_eq!(selection(app), vec![LINE, dim]);
        assert_eq!(content(&session(app).sketch), content(&source), "a box selects and never moves anything");
        case.frame(app, "02-window-selected");

        // Right to left over the same area: a dotted crossing box that also takes the
        // line it crosses. Without a modifier it replaces the selection.
        drag(app, right_bottom, left_top, Modifiers::NONE, |app| {
            let b = session(app).selection_box.clone().unwrap();
            assert_eq!(b.kind(), BoxKind::Crossing);
            case.frame(app, "03-crossing-dragging");
        });
        assert_eq!(selection(app), vec![LINE, CROSSING_LINE, dim]);
        case.frame(app, "04-crossing-selected");

        // Ctrl inverts within the box, Shift adds; what lies outside the box stays.
        let (circle_lt, circle_rb) = corners(app, P2::new(18.0, -7.0), P2::new(32.0, 7.0));
        drag(app, circle_lt, circle_rb, Modifiers::CTRL, |_| {});
        assert_eq!(selection(app), vec![LINE, CROSSING_LINE, dim, CIRCLE], "Ctrl adds what was not selected");
        drag(app, left_top, right_bottom, Modifiers::CTRL, |_| {});
        assert_eq!(selection(app), vec![CROSSING_LINE, CIRCLE], "Ctrl drops what was selected");
        drag(app, left_top, right_bottom, Modifiers::SHIFT, |_| {});
        drag(app, left_top, right_bottom, Modifiers::SHIFT, |_| {});
        assert_eq!(selection(app), vec![CROSSING_LINE, CIRCLE, LINE, dim], "Shift only adds");

        // A box over nothing clears the selection, as a click on nothing does.
        let (empty_lt, empty_rb) = corners(app, P2::new(5.0, 20.0), P2::new(15.0, 30.0));
        drag(app, empty_lt, empty_rb, Modifiers::NONE, |_| {});
        assert!(selection(app).is_empty());

        // Escape drops a box that is still being dragged, and the selection stays.
        drag(app, right_bottom, left_top, Modifiers::NONE, |app| {
            app.key_press(Key::Escape);
            app.run_steps(2);
            assert!(session(app).selection_box.is_none(), "Escape cancels the box");
        });
        assert!(selection(app).is_empty());
        assert!(app.state().state.sketch.is_some(), "the first Escape stays in the sketch");

        // A drag that starts on a curve still moves it rather than drawing a box.
        let on_circle = screen(app, P2::new(30.0, 0.0));
        let beside = screen(app, P2::new(33.0, 0.0));
        drag(app, on_circle, beside, Modifiers::NONE, |app| {
            assert!(session(app).selection_box.is_none());
        });
        let EntityKind::Circle { center, .. } = session(app).sketch.entity(3).unwrap().kind else { panic!("circle") };
        assert!(center.dist(P2::new(25.0, 0.0)) > 1.0, "the circle moved to {center:?}");
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        settle(app);
        assert_eq!(content(&session(app).sketch), content(&source));

        // Delete removes everything a crossing box took, as one undo step.
        drag(app, right_bottom, left_top, Modifiers::NONE, |_| {});
        assert_eq!(selection(app), vec![LINE, CROSSING_LINE, dim]);
        app.key_press(Key::Delete);
        settle(app);
        let left: Vec<_> = session(app).sketch.drawn().map(|e| e.id).collect();
        assert_eq!(left, vec![3], "only the circle is left");
        assert!(session(app).sketch.constraints.iter().all(|c| !c.is_dimension()));
        assert!(selection(app).is_empty());
        case.frame(app, "05-deleted");
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        settle(app);
        assert_eq!(content(&session(app).sketch), content(&source), "one undo brings back what Delete removed");
    });
}

#[test]
fn a_box_ends_where_the_button_comes_up() {
    let mut case = Case::new("sketch-box-release-move");
    let (mut app, _) = open(&mut case);
    case.run(&mut app, |case, app| {
        let dim = dimension(app);
        let (left_top, right_bottom) = corners(app, P2::new(-35.0, -2.0), P2::new(-3.0, 13.0));
        ribbon::render_steps::rendered(app, |app| {
            app.hover_at(left_top);
            app.run_steps(3);
            let pressed = Modifiers::NONE;
            app.event(egui::Event::PointerButton {
                pos: left_top,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: pressed,
            });
            app.step();
            // A short drag that holds nothing yet.
            app.hover_at(left_top + egui::vec2(12.0, 12.0));
            app.step();
            let b = session(app).selection_box.clone().expect("a box is being dragged");
            assert!(b.picks.is_empty(), "the short box holds {:?}", b.picks);
            // The last move and the release arrive in one frame, as a quick flick can.
            app.event(egui::Event::PointerMoved(right_bottom));
            app.event(egui::Event::PointerButton {
                pos: right_bottom,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: pressed,
            });
            app.step();
            settle(app);
        });
        assert!(session(app).selection_box.is_none());
        assert_eq!(selection(app), vec![LINE, dim], "the box reached the release point");
        case.frame(app, "01-selected");
    });
}

/// How far a pixel is from another, summed over the colour channels.
fn difference(a: &image::Rgba<u8>, b: &image::Rgba<u8>) -> u32 {
    (0..3).map(|i| a[i].abs_diff(b[i]) as u32).sum()
}

#[test]
fn the_curve_under_the_pointer_shows_a_hover_tint() {
    let mut case = Case::new("sketch-hover-tint");
    let (mut app, _) = open(&mut case);
    case.run(&mut app, |case, app| {
        let ppp = app.ctx.pixels_per_point();
        let on_line = screen(app, P2::new(-20.0, 5.0));
        // Beside the line, under where the halo reaches but not the line's own ink.
        let beside: Vec<Pos2> =
            (-4..=4).map(|i| screen(app, P2::new(-20.0 + i as f64, 5.0)) + egui::vec2(0.0, 2.2)).collect();
        let away = screen(app, P2::new(10.0, -25.0));
        app.hover_at(away);
        settle(app);
        let plain = case.frame(app, "01-not-hovered");
        app.hover_at(on_line);
        settle(app);
        assert_eq!(session(app).hover, Some(LINE));
        let hovered = case.frame(app, "02-hovered");
        let pixel = |image: &image::RgbaImage, p: Pos2| *image.get_pixel((p.x * ppp) as u32, (p.y * ppp) as u32);
        let weakest = beside.iter().map(|p| difference(&pixel(&plain, *p), &pixel(&hovered, *p))).min().unwrap();
        case.note(format!("hover tint changes the pixels beside the line by at least {weakest}"));
        assert!(weakest >= 70, "the hover tint is too faint to see: {weakest}");
    });
}

#[test]
fn a_shown_sketch_is_picked_by_its_line_outside_the_sketch() {
    let mut case = Case::new("sketch-pick-in-part");
    let (doc, id, _) = document();
    let plane = doc.feature(id).unwrap().plane.unwrap();
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(id))));
        settle(app);
        let on_line = {
            let st = &app.state().state;
            let at = Frame { plane, camera: &st.viewport.camera, rect: st.viewport.rect }
                .screen(P2::new(-5.0, -10.0))
                .unwrap();
            assert!(st.viewport.rect.contains(at));
            at
        };
        app.hover_at(on_line);
        settle(app);
        // On the vertical line, away from the dimension label, which has gestures of its own.
        assert!(app.state().state.viewport.dimensions.iter().all(|d| !d.rect.contains(on_line)));
        assert_eq!(app.state().state.viewport.hover_sketch, Some(id), "the shown sketch's line is under the pointer");
        case.frame(app, "01-hovered");
        ribbon::click_at(app, on_line);
        settle(app);
        assert_eq!(
            app.state().state.doc.selection,
            Some(Selection::Feature(id)),
            "a click on the line keeps the sketch"
        );
        case.frame(app, "02-clicked");
    });
}
