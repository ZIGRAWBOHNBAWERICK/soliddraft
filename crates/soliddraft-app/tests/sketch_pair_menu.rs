//! Relations for a sketch selection of two or more entities, through the real Edit Suite
//! frame. Letting go of Ctrl after a Ctrl pick opens a strip of the relations the
//! selection can take, each named by its tooltip; a right-click on the selection opens
//! the columns with a Relations column for the whole selection. Both add a relation as
//! one undo step, and a right-click on one entity keeps its own menu.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::{By, NodeT, Queryable};
use ribbon::App;
use soliddraft_app::{
    document::{Command, FeatureId, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityId, EntityKind, P2, Sketch, solve},
    sketcher::Frame,
    state::{AppAction, SketchSel, SketchSession, Tool},
};

const STRIP: &str = "Relations for the selection";

struct Geometry {
    a: EntityId,
    b: EntityId,
    arc: EntityId,
}

/// Two lines that are neither parallel nor at right angles, and an arc on their right.
fn sketch() -> (Sketch, Geometry) {
    let mut sketch = Sketch::new();
    let a = sketch.add(EntityKind::Line { a: P2::new(-20.0, -10.0), b: P2::new(10.0, -10.0) }, false);
    let b = sketch.add(EntityKind::Line { a: P2::new(-15.0, 0.0), b: P2::new(5.0, 15.0) }, false);
    let arc = sketch
        .add(EntityKind::Arc { center: P2::new(25.0, 0.0), start: P2::new(30.0, 0.0), end: P2::new(25.0, 5.0) }, false);
    (sketch, Geometry { a, b, arc })
}

/// The Edit Suite frame on an open sketch holding the geometry of [`sketch`].
fn launch(case: &ribbon::Case) -> (App, FeatureId, Geometry) {
    std::fs::write(case.path("config/settings.json"), r#"{"frame_layout":"edit_suite"}"#).unwrap();
    let (mut sketch, geometry) = sketch();
    solve::solve(&mut sketch, None, true);
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    (app, id, geometry)
}

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().expect("open sketch")
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    let at =
        Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap();
    assert!(st.viewport.rect.contains(at), "{p:?} is off screen at {at:?}");
    at
}

/// A point a quarter of the way along line `a`, or the middle of the arc.
fn on(app: &App, id: EntityId) -> Pos2 {
    let entity = session(app).sketch.entity(id).unwrap().clone();
    let p = match entity.kind {
        EntityKind::Line { a, b } => P2::new(0.75 * a.u + 0.25 * b.u, 0.75 * a.v + 0.25 * b.v),
        EntityKind::Arc { center, .. } => {
            let half = std::f64::consts::FRAC_1_SQRT_2 * 5.0;
            P2::new(center.u + half, center.v + half)
        }
        _ => unreachable!(),
    };
    screen(app, p)
}

fn press(app: &mut App, at: Pos2, button: PointerButton, modifiers: Modifiers) {
    app.hover_at(at);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers });
        app.step();
    }
    app.run_steps(3);
}

fn click(app: &mut App, at: Pos2) {
    press(app, at, PointerButton::Primary, Modifiers::NONE);
}

/// A click with Ctrl held, which stays held until [`release_ctrl`].
fn ctrl_click(app: &mut App, at: Pos2) {
    app.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    app.step();
    press(app, at, PointerButton::Primary, Modifiers::CTRL);
}

fn release_ctrl(app: &mut App) {
    app.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    app.run_steps(4);
}

fn right_click(app: &mut App, at: Pos2) {
    press(app, at, PointerButton::Secondary, Modifiers::NONE);
    app.run_steps(2);
}

/// The buttons of the open strip, by name; `None` when it is closed.
fn strip(app: &App) -> Option<Vec<String>> {
    let strip = app.query_by_label(STRIP)?;
    Some(
        strip
            .query_all(By::new().role(egui::accesskit::Role::Button))
            .filter_map(|node| node.accesskit_node().label())
            .collect(),
    )
}

fn click_in_strip(app: &mut App, name: &str) {
    let at = app.get_by_label(STRIP).get_by_label(name).rect().center();
    click(app, at);
}

/// How many labels read `label`. A column heading adds one; "Relations" is also a
/// palette group, so a heading is told by the count going up.
fn labels(app: &App, label: &str) -> usize {
    app.query_all(By::new().role(egui::accesskit::Role::Label).label(label)).count()
}

/// The menu row `label` in the columns that opened at `at`: the one nearest the pointer,
/// as the columns open beside it and flip at the viewport's edges.
fn menu_row(app: &App, label: &str, at: Pos2) -> Pos2 {
    let rect = app
        .query_all(By::new().role(egui::accesskit::Role::Button).label(label))
        .map(|node| node.rect())
        .min_by(|a, b| a.distance_to_pos(at).total_cmp(&b.distance_to_pos(at)))
        .unwrap_or_else(|| panic!("no menu row {label}"));
    assert!(rect.distance_to_pos(at) < 400.0, "{label} at {rect:?} is not in the menu at {at:?}");
    rect.center()
}

fn has(app: &App, wanted: impl Fn(&ConstraintKind) -> bool) -> bool {
    session(app).sketch.constraints.iter().any(|c| wanted(&c.kind))
}

#[test]
fn ctrl_release_on_two_lines_offers_their_relations_and_adds_one_as_one_undo_step() {
    let case = ribbon::case("pair-menu-lines");
    let (mut app, _, g) = launch(&case);
    case.run(&mut app, |app| {
        let at_a = on(app, g.a);
        let at_b = on(app, g.b);
        click(app, at_a);
        ctrl_click(app, at_b);
        assert_eq!(session(app).selection, [SketchSel::Entity(g.a), SketchSel::Entity(g.b)]);
        assert_eq!(strip(app), None, "the strip waits for Ctrl to come up");
        release_ctrl(app);
        assert_eq!(
            strip(app).expect("releasing Ctrl opens the strip"),
            ["Parallel", "Perpendicular", "Equal", "Collinear", "Smart Dimension"]
        );
        let button = app.get_by_label(STRIP).get_by_label("Perpendicular").rect();
        assert!(button.left() > at_b.x && button.bottom() < at_b.y, "the strip opens up and right of the last pick");
        case.frame(app, "two-lines");

        let revision = app.state().state.doc.revision();
        click_in_strip(app, "Perpendicular");
        assert!(has(app, |k| matches!(k, ConstraintKind::Perpendicular { a, b } if (*a, *b) == (g.a, g.b))));
        assert_eq!(app.state().state.doc.revision(), revision + 1, "one undo step");
        assert_eq!(strip(app), None, "a chosen relation closes the strip");
        assert_eq!(session(app).selection, [SketchSel::Entity(g.a), SketchSel::Entity(g.b)]);
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        app.run_steps(5);
        assert!(!has(app, |k| matches!(k, ConstraintKind::Perpendicular { .. })), "one undo removes it");
    });
}

#[test]
fn ctrl_release_on_a_line_and_an_arc_offers_tangent_and_esc_closes_the_strip() {
    let case = ribbon::case("pair-menu-tangent");
    let (mut app, _, g) = launch(&case);
    case.run(&mut app, |app| {
        let at_a = on(app, g.a);
        let at_arc = on(app, g.arc);
        click(app, at_a);
        ctrl_click(app, at_arc);
        release_ctrl(app);
        assert_eq!(strip(app).expect("the strip opens"), ["Tangent"]);
        case.frame(app, "line-and-arc");
        app.key_press(Key::Escape);
        app.run_steps(4);
        assert_eq!(strip(app), None, "Esc closes the strip");
        assert_eq!(
            session(app).selection,
            [SketchSel::Entity(g.a), SketchSel::Entity(g.arc)],
            "Esc closes the strip and leaves the selection"
        );

        // Ctrl held and let go with nothing picked opens nothing.
        app.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
        app.run_steps(3);
        release_ctrl(app);
        assert_eq!(strip(app), None);

        // A click elsewhere closes it too.
        ctrl_click(app, at_arc);
        ctrl_click(app, at_arc);
        release_ctrl(app);
        assert_eq!(strip(app).expect("a fresh Ctrl pick opens it again"), ["Tangent"]);
        let empty = screen(app, P2::new(-25.0, -20.0));
        click(app, empty);
        assert_eq!(strip(app), None, "a click elsewhere closes the strip");

        let revision = app.state().state.doc.revision();
        click(app, at_a);
        ctrl_click(app, at_arc);
        release_ctrl(app);
        click_in_strip(app, "Tangent");
        assert!(has(app, |k| matches!(k, ConstraintKind::Tangent { .. })));
        assert_eq!(app.state().state.doc.revision(), revision + 1);
    });
}

#[test]
fn smart_dimension_in_the_strip_takes_both_lines_and_places_their_angle() {
    let case = ribbon::case("pair-menu-angle");
    let (mut app, _, g) = launch(&case);
    case.run(&mut app, |app| {
        let at_a = on(app, g.a);
        let at_b = on(app, g.b);
        click(app, at_a);
        ctrl_click(app, at_b);
        release_ctrl(app);
        click_in_strip(app, "Smart Dimension");
        assert_eq!(session(app).tool, Tool::Dimension);
        assert_eq!(session(app).dim_first, Some(SketchSel::Entity(g.a)));
        assert_eq!(session(app).dim_second, Some(SketchSel::Entity(g.b)));
        let place = screen(app, P2::new(-25.0, 10.0));
        click(app, place);
        assert!(has(app, |k| matches!(k, ConstraintKind::Angle { a, b, .. } if (*a, *b) == (g.a, g.b))));
    });
}

#[test]
fn right_click_on_two_selected_entities_lists_relations_for_the_whole_selection() {
    let case = ribbon::case("pair-menu-right-click");
    let (mut app, _, g) = launch(&case);
    case.run(&mut app, |app| {
        let at_a = on(app, g.a);
        let at_b = on(app, g.b);
        click(app, at_a);
        ctrl_click(app, at_b);
        release_ctrl(app);
        assert!(strip(app).is_some());
        let before: Vec<usize> = ["Relations", "Selection", "Sketch", "Line"].map(|l| labels(app, l)).into();
        right_click(app, at_b);
        assert_eq!(strip(app), None, "the right-click closes the strip");
        assert_eq!(session(app).selection, [SketchSel::Entity(g.a), SketchSel::Entity(g.b)]);
        for (index, heading) in ["Relations", "Selection", "Sketch"].into_iter().enumerate() {
            assert_eq!(labels(app, heading), before[index] + 1, "the {heading} column is there");
        }
        assert_eq!(labels(app, "Line"), before[3], "the one-line column gives way to the selection's");
        for relation in ["Parallel", "Perpendicular", "Equal", "Collinear"] {
            menu_row(app, relation, at_b);
        }
        for single in ["Make vertical", "Make horizontal", "Fix"] {
            assert!(
                app.query_all(By::new().role(egui::accesskit::Role::Button).label(single)).all(|node| !(node
                    .rect()
                    .left()
                    >= at_b.x - 1.0
                    && node.rect().top() >= at_b.y - 1.0)),
                "{single} applies to one entity and stays out"
            );
        }
        case.frame(app, "two-lines-right-click");

        let revision = app.state().state.doc.revision();
        let row = menu_row(app, "Parallel", at_b);
        click(app, row);
        assert!(has(app, |k| matches!(k, ConstraintKind::Parallel { a, b } if (*a, *b) == (g.a, g.b))));
        assert_eq!(app.state().state.doc.revision(), revision + 1, "one undo step");
        assert_eq!(labels(app, "Relations"), before[0], "choosing a relation closes the menu");

        // A line and an arc: Tangent. Parallel moved the lines, so find them again.
        let at_a = on(app, g.a);
        let at_arc = on(app, g.arc);
        click(app, at_a);
        ctrl_click(app, at_arc);
        release_ctrl(app);
        right_click(app, at_arc);
        assert_eq!(labels(app, "Relations"), before[0] + 1);
        menu_row(app, "Tangent", at_arc);
        case.frame(app, "line-and-arc-right-click");
    });
}

#[test]
fn right_click_on_one_line_keeps_the_line_menu() {
    let case = ribbon::case("pair-menu-single");
    let (mut app, _, g) = launch(&case);
    case.run(&mut app, |app| {
        let at_a = on(app, g.a);
        click(app, at_a);
        let before: Vec<usize> = ["Line", "Sketch", "Relations", "Selection"].map(|l| labels(app, l)).into();
        right_click(app, at_a);
        assert_eq!(session(app).selection, [SketchSel::Entity(g.a)]);
        assert_eq!(labels(app, "Line"), before[0] + 1);
        assert_eq!(labels(app, "Sketch"), before[1] + 1);
        assert_eq!(labels(app, "Relations"), before[2], "no Relations column for one line");
        assert_eq!(labels(app, "Selection"), before[3]);
        menu_row(app, "Make vertical", at_a);
    });
}
