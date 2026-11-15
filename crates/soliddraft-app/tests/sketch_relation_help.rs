//! Relation glyphs explain themselves in the rendered app. Hovering any glyph the sketcher
//! draws names the relation and the geometry it holds. A right-click on a glyph lists every
//! constraint on that geometry; hovering a row lights it up on the sketch, Delete on a row
//! removes it as one undo step, and a click on a row selects it. A dimension label over a
//! glyph still comes first, as it does for a click.

use egui::{Key, Modifiers, PointerButton, Pos2, Rect};
use egui_kittest::kittest::{NodeT, Queryable, by};
use soliddraft_app::document::{Command, Document, FeatureId, PlaneKind, SketchPlane};
use soliddraft_app::sketch::tools::{OffsetCap, OffsetOptions, Operation, PatternSpec};
use soliddraft_app::sketch::{
    Align, ConstraintId, ConstraintKind, DistanceKind, EntityKind, P2, PointRef, Sketch, SolveStatus, solve,
};
use soliddraft_app::sketcher::{Frame, glyph_rects, relations};
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

fn frame(app: &App) -> Frame<'_> {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }
}

/// Where the glyph of relation `id` is drawn; the first one for a relation with two.
fn glyph(app: &App, id: ConstraintId) -> Pos2 {
    let rects = glyph_rects(&frame(app), sketch(app));
    let at = rects.iter().find(|(c, _)| *c == id).unwrap_or_else(|| panic!("relation {id} has a glyph")).1.center();
    assert!(app.state().state.viewport.rect.contains(at), "glyph {id} is off screen at {at:?}");
    at
}

fn texts(app: &App) -> Vec<String> {
    app.query_all(by()).flat_map(|n| [n.accesskit_node().label(), n.accesskit_node().value()]).flatten().collect()
}

fn shows(app: &App, text: &str) -> bool {
    texts(app).iter().any(|t| t == text)
}

/// Rests the pointer on `at` long enough for a tooltip, coming from somewhere else.
fn rest(app: &mut App, at: Pos2) {
    let away = app.state().state.viewport.rect.left_bottom() + egui::vec2(30.0, -60.0);
    app.hover_at(away);
    app.run_steps(4);
    app.hover_at(at);
    app.run_steps(60);
}

fn right_click(app: &mut App, at: Pos2) {
    ribbon::render_steps::rendered(app, |app| {
        app.hover_at(at);
        app.run_steps(3);
        for pressed in [true, false] {
            app.event(egui::Event::PointerButton {
                pos: at,
                button: PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            app.step();
        }
        settle(app);
    });
}

fn click(app: &mut App, at: Pos2) {
    ribbon::render_steps::rendered(app, |app| ribbon::click_at(app, at));
    settle(app);
}

/// The bounds of the button assistive technology reads as `label`.
fn button(app: &App, label: &str) -> Rect {
    let found: Vec<Rect> =
        app.query_all_by_role_and_label(egui::accesskit::Role::Button, label).map(|n| n.rect()).collect();
    let named: Vec<String> = app
        .query_all(by().role(egui::accesskit::Role::Button))
        .filter_map(|n| n.accesskit_node().label())
        .filter(|l| l.contains(" on "))
        .collect();
    assert_eq!(found.len(), 1, "one button {label}: {found:?}; buttons named \"on\": {named:?}");
    found[0]
}

fn undo(app: &mut App) {
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    settle(app);
}

fn line(s: &mut Sketch, a: (f64, f64), b: (f64, f64)) -> u32 {
    s.add(EntityKind::Line { a: P2::new(a.0, a.1), b: P2::new(b.0, b.1) }, false)
}

fn circle(s: &mut Sketch, c: (f64, f64), radius: f64) -> u32 {
    s.add(EntityKind::Circle { center: P2::new(c.0, c.1), radius }, false)
}

fn point(s: &mut Sketch, p: (f64, f64)) -> u32 {
    s.add(EntityKind::Point { p: P2::new(p.0, p.1) }, false)
}

/// Ids of the relations the test picks out of the sketch.
struct Picks {
    /// The two lines of the corner, and the two parallel lines.
    corner: [u32; 2],
    pair: [u32; 2],
    meet: ConstraintId,
    level: ConstraintId,
    plumb: ConstraintId,
    length: ConstraintId,
    parallel: ConstraintId,
}

/// One of every relation glyph the sketcher draws, each on its own geometry, already true.
fn every_glyph() -> (Sketch, Picks) {
    let mut s = Sketch::new();
    let h = line(&mut s, (-80.0, 40.0), (-60.0, 40.0));
    s.constrain(ConstraintKind::Horizontal { target: Align::Line(h) });
    let v = line(&mut s, (-50.0, 30.0), (-50.0, 50.0));
    s.constrain(ConstraintKind::Vertical { target: Align::Line(v) });
    // A corner: Coincident, with a Horizontal, a length and a Vertical on its two lines.
    let c1 = line(&mut s, (-35.0, 35.0), (-15.0, 35.0));
    let c2 = line(&mut s, (-15.0, 35.0), (-15.0, 50.0));
    let level = s.constrain(ConstraintKind::Horizontal { target: Align::Line(c1) });
    let meet = s.constrain(ConstraintKind::Coincident { a: PointRef::new(c1, 1), b: PointRef::new(c2, 0) });
    let length = s.constrain(ConstraintKind::Distance {
        a: PointRef::new(c1, 0),
        b: PointRef::new(c1, 1),
        kind: DistanceKind::Aligned,
        value: 20.0,
        label: P2::new(-25.0, 28.0),
    });
    let plumb = s.constrain(ConstraintKind::Vertical { target: Align::Line(c2) });
    let p1 = line(&mut s, (-80.0, 15.0), (-65.0, 22.0));
    let p2 = line(&mut s, (-80.0, 5.0), (-65.0, 12.0));
    let parallel = s.constrain(ConstraintKind::Parallel { a: p1, b: p2 });
    let q1 = line(&mut s, (-50.0, 5.0), (-40.0, 15.0));
    let q2 = line(&mut s, (-30.0, 15.0), (-20.0, 5.0));
    s.constrain(ConstraintKind::Perpendicular { a: q1, b: q2 });
    let t = line(&mut s, (0.0, 45.0), (20.0, 45.0));
    let round = circle(&mut s, (10.0, 39.0), 6.0);
    s.constrain(ConstraintKind::Tangent { a: t, b: round });
    let e1 = circle(&mut s, (35.0, 40.0), 4.0);
    let e2 = circle(&mut s, (48.0, 40.0), 4.0);
    s.constrain(ConstraintKind::Equal { a: e1, b: e2 });
    let k1 = circle(&mut s, (70.0, 40.0), 7.0);
    let k2 = circle(&mut s, (70.0, 40.0), 3.0);
    s.constrain(ConstraintKind::Concentric { a: k1, b: k2 });
    let m = line(&mut s, (0.0, 15.0), (20.0, 15.0));
    let middle = point(&mut s, (10.0, 15.0));
    s.constrain(ConstraintKind::Midpoint { point: PointRef::new(middle, 0), line: m });
    let o = line(&mut s, (30.0, 15.0), (50.0, 15.0));
    let on = point(&mut s, (35.0, 15.0));
    s.constrain(ConstraintKind::PointOn { point: PointRef::new(on, 0), entity: o });
    let axis = line(&mut s, (70.0, 5.0), (70.0, 25.0));
    let left = point(&mut s, (63.0, 15.0));
    let right = point(&mut s, (77.0, 15.0));
    s.constrain(ConstraintKind::Symmetric { a: PointRef::new(left, 0), b: PointRef::new(right, 0), axis });
    let pin = point(&mut s, (-80.0, -15.0));
    s.constrain(ConstraintKind::Fix { point: PointRef::new(pin, 0), at: P2::new(-80.0, -15.0) });
    let held = line(&mut s, (-65.0, -15.0), (-50.0, -15.0));
    let at = s.entity(held).unwrap().kind.clone();
    s.constrain(ConstraintKind::FixEntity { entity: held, at });
    // Offset with line caps, and a linear pattern, from the sketch tools.
    let source = line(&mut s, (-30.0, -15.0), (-10.0, -15.0));
    let options = OffsetOptions { cap: OffsetCap::Lines, ..OffsetOptions::default() };
    s.apply_operation(&Operation::Offset { entities: vec![source], distance: -6.0, options }).expect("offset");
    let seed = line(&mut s, (10.0, -15.0), (20.0, -15.0));
    let spec = PatternSpec::Linear { step: P2::new(0.0, -8.0), count: 2, second: None };
    s.apply_operation(&Operation::Pattern { entities: vec![seed], spec, skipped: Vec::new() }).expect("pattern");
    solve::solve(&mut s, None, true);
    (s, Picks { corner: [c1, c2], pair: [p1, p2], meet, level, plumb, length, parallel })
}

fn part(source: Sketch) -> (Document, FeatureId) {
    let mut doc = Document::new("Relation glyphs", soliddraft_app::kernel());
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

/// The name of every kind of relation glyph, as the tooltip and the menu show it.
const GLYPHS: [&str; 16] = [
    "Coincident",
    "On entity",
    "Horizontal",
    "Vertical",
    "Parallel",
    "Perpendicular",
    "Tangent",
    "Equal",
    "Midpoint",
    "Concentric",
    "Symmetric",
    "Fixed",
    "Offset",
    "Offset cap",
    "Pattern instance",
    // FixEntity shares the name Fixed; its glyph is checked through the same name.
    "Fixed",
];

#[test]
fn hovering_any_relation_glyph_names_it_and_its_geometry() {
    let mut case = Case::new("sketch-relation-tooltips");
    let (source, picks) = every_glyph();
    assert!(!matches!(source.status, SolveStatus::OverDefined { .. } | SolveStatus::Failed(_)), "{:?}", source.status);
    let (doc, id) = part(source.clone());
    let mut app = open(doc, id);
    case.run(&mut app, |case, app| {
        case.frame(app, "00-every-glyph");
        let glyphs: Vec<_> = sketch(app).constraints.iter().filter(|c| !c.is_dimension()).cloned().collect();
        let mut named: Vec<&str> = glyphs.iter().map(|c| c.name()).collect();
        named.sort_unstable();
        named.dedup();
        let mut wanted = GLYPHS.to_vec();
        wanted.sort_unstable();
        wanted.dedup();
        assert_eq!(named, wanted, "the sketch has one of every relation glyph");

        for c in &glyphs {
            let at = glyph(app, c.id);
            rest(app, at);
            assert_eq!(
                session(app).hover,
                Some(SketchSel::Constraint(c.id)),
                "{} glyph is under the pointer",
                c.name()
            );
            let sentence = relations::sentence(sketch(app), c);
            assert!(shows(app, c.name()) && shows(app, &sentence), "the {} tooltip says \"{sentence}\"", c.name());
        }

        // Three in full, with the words a user reads.
        let ([c1, c2], [p1, p2]) = (picks.corner, picks.pair);
        for (id, name, words) in [
            (picks.parallel, "parallel", format!("Line {p1} and Line {p2} stay parallel.")),
            (picks.meet, "coincident", format!("Line {c1} end and Line {c2} start meet at one point.")),
            (picks.level, "horizontal", format!("Line {c1} stays horizontal.")),
        ] {
            rest(app, glyph(app, id));
            assert!(shows(app, &words), "{name}: {words}");
            case.frame(app, &format!("01-hover-{name}"));
        }
    });
}

#[test]
fn right_click_on_a_glyph_lists_the_constraints_on_its_geometry() {
    let mut case = Case::new("sketch-relation-menu");
    let (source, picks) = every_glyph();
    let (doc, id) = part(source.clone());
    let mut app = open(doc, id);
    case.run(&mut app, |case, app| {
        right_click(app, glyph(app, picks.meet));
        let menu = session(app).relations_menu.clone().expect("the right-click opened the relations menu");
        assert_eq!(menu.glyph, picks.meet);
        assert!(app.query_by_label("Smart dim").is_none(), "the command ring stays closed");
        case.frame(app, "01-menu");
        // Every constraint on the two lines of the corner, in sketch order, each with Delete.
        let ([c1, c2], [p1, p2]) = (picks.corner, picks.pair);
        let length = relations::title(sketch(app), sketch(app).constraint(picks.length).unwrap());
        assert_eq!(length, "Distance 20");
        let rows = [
            format!("Horizontal on Line {c1}"),
            format!("Coincident on Line {c1}, Line {c2}"),
            format!("Distance 20 on Line {c1}"),
            format!("Vertical on Line {c2}"),
        ];
        for row in &rows {
            button(app, row);
            button(app, &format!("Delete {row}"));
        }
        let listed = app
            .query_all(by().role(egui::accesskit::Role::Button))
            .filter_map(|n| n.accesskit_node().label())
            .filter(|l| l.starts_with("Delete ") && l.contains(" on Line "))
            .count();
        assert_eq!(listed, rows.len(), "nothing else is listed");

        // Hovering a row lights its constraint and geometry up on the sketch.
        let distance = button(app, &rows[2]);
        app.hover_at(distance.left_center() + egui::vec2(60.0, 0.0));
        app.run_steps(6);
        assert_eq!(session(app).relations_menu.as_ref().and_then(|m| m.hovered), Some(picks.length));
        assert_eq!(session(app).hover, Some(SketchSel::Constraint(picks.length)), "the sketch shows the hovered row");
        case.frame(app, "02-row-hovered");

        // Delete on a row removes that constraint alone, and the list stays open.
        let before = sketch(app).clone();
        click(app, button(app, &format!("Delete {}", rows[3])).center());
        assert!(sketch(app).constraint(picks.plumb).is_none(), "Vertical is gone");
        for kept in [picks.meet, picks.level, picks.length] {
            assert!(sketch(app).constraint(kept).is_some(), "constraint {kept} stays");
        }
        let committed = &app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch;
        assert!(committed.constraint(picks.plumb).is_none(), "the document has the deletion");
        assert!(session(app).relations_menu.is_some(), "the menu stays open after Delete");
        assert!(app.query_by_label(&rows[3]).is_none(), "its row is gone");
        case.frame(app, "03-row-deleted");
        undo(app);
        assert_eq!(sketch(app).constraints, before.constraints, "one undo brings the relation back");

        // A click on a row selects it and closes the menu.
        right_click(app, glyph(app, picks.meet));
        click(app, button(app, &rows[2]).left_center() + egui::vec2(60.0, 0.0));
        assert_eq!(session(app).selection, vec![SketchSel::Constraint(picks.length)]);
        assert!(session(app).relations_menu.is_none(), "choosing a row closes the menu");
        case.frame(app, "04-row-selected");

        // A right-click on another glyph moves the list there.
        right_click(app, glyph(app, picks.meet));
        right_click(app, glyph(app, picks.parallel));
        assert_eq!(session(app).relations_menu.as_ref().map(|m| m.glyph), Some(picks.parallel));
        button(app, &format!("Parallel on Line {p1}, Line {p2}"));
        assert!(app.query_by_label(&rows[1]).is_none(), "the corner's list is gone");

        // Escape closes the menu, and so does a right-click elsewhere.
        app.key_press(Key::Escape);
        settle(app);
        assert!(session(app).relations_menu.is_none(), "Escape closes the menu");
        right_click(app, glyph(app, picks.parallel));
        let empty = app.state().state.viewport.rect.right_bottom() - egui::vec2(120.0, 160.0);
        right_click(app, empty);
        assert!(session(app).relations_menu.is_none(), "a right-click elsewhere closes it");
    });
}

/// A primary press or release at `at`, without moving the pointer.
fn primary(app: &mut App, at: Pos2, pressed: bool) {
    app.event(egui::Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(3);
}

#[test]
fn an_undo_while_delete_is_held_does_not_delete_the_row_that_moved_under_it() {
    let mut case = Case::new("sketch-relation-held-delete");
    let (source, picks) = every_glyph();
    let (doc, id) = part(source);
    let mut app = open(doc, id);
    case.run(&mut app, |case, app| {
        let [c1, c2] = picks.corner;
        let level = format!("Horizontal on Line {c1}");
        let length = format!("Distance 20 on Line {c1}");
        let plumb = format!("Vertical on Line {c2}");
        right_click(app, glyph(app, picks.meet));
        click(app, button(app, &format!("Delete {level}")).center());
        assert!(sketch(app).constraint(picks.level).is_none(), "the first row is deleted");

        // Hold Delete on the last row, Vertical, and undo the first deletion meanwhile:
        // Horizontal comes back at the top and Distance moves under the pointer.
        let at = button(app, &format!("Delete {plumb}")).center();
        ribbon::render_steps::rendered(app, |app| {
            app.hover_at(at);
            app.run_steps(3);
            primary(app, at, true);
            app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            app.run_steps(6);
            assert!(sketch(app).constraint(picks.level).is_some(), "undo restored Horizontal");
            assert!(button(app, &format!("Delete {length}")).contains(at), "Distance's Delete is under the pointer");
            primary(app, at, false);
            settle(app);
        });
        case.frame(app, "01-released-after-undo");
        assert!(
            sketch(app).constraint(picks.length).is_some(),
            "the release does not delete Distance, which moved under it"
        );
        for kept in [picks.level, picks.meet] {
            assert!(sketch(app).constraint(kept).is_some(), "constraint {kept} stays");
        }
    });
}

#[test]
fn a_dimension_label_over_a_glyph_comes_first() {
    let mut case = Case::new("sketch-relation-label-first");
    let (source, picks) = every_glyph();
    let (doc, id) = part(source);
    let mut app = open(doc, id);
    case.run(&mut app, |case, app| {
        // Lay the length label right over the Parallel glyph.
        let at = glyph(app, picks.parallel);
        let spot = frame(app).plane_at(at).unwrap();
        let s = app.state_mut().state.sketch.as_mut().unwrap();
        s.sketch.constraint_mut(picks.length).unwrap().set_label(spot);
        settle(app);
        rest(app, at);
        assert_eq!(session(app).hover, Some(SketchSel::Constraint(picks.length)), "the label is on top");
        let [p1, p2] = picks.pair;
        assert!(!shows(app, &format!("Line {p1} and Line {p2} stay parallel.")), "no relation tooltip under a label");
        case.frame(app, "01-label-over-glyph");
        right_click(app, at);
        assert!(session(app).relations_menu.is_none(), "the label's own menu opens, not the relation list");
        assert_eq!(session(app).selection, vec![SketchSel::Constraint(picks.length)]);
        case.frame(app, "02-label-menu");
    });
}
