//! The Sketch tab's Modify group with the mouse: a polygon placed by two clicks, a line
//! extended to the polygon's edges, the edges trimmed back to it, and the profile that
//! results extruded through Catallax. Every tool shows its preview before the click and
//! Escape puts it down. The real app, the real wgpu pipeline and the real kernel, driven
//! by pointer and keyboard events only. Frames go to `target/ux-tools/sketch-modify/`.
//!
//! The same profile is built headlessly by `ci/scenarios/sketch-polygon-trim-extend.json`,
//! which checks the exact volume. The pointer places the polygon's vertex to a pixel, so
//! this case holds the volume to a millimetre cubed.

use egui::{Key, Modifiers, PointerButton, Pos2, Shape};
use soliddraft_app::document::{Command, Document, PlaneKind, SketchPlane};
use soliddraft_app::sketch::tools::{End, Operation};
use soliddraft_app::sketch::{Entity, EntityId, EntityKind, P2, profile};
use soliddraft_app::sketcher::Frame;
use soliddraft_app::state::{SketchSession, Tool};

#[path = "support/ribbon.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use ribbon::App;
use ux_tools::{Case, STANDARD, facts, launch, rebuilt, settle};

/// The hexagon's circumradius, the chord's height and what the tools make of them.
const R: f64 = 20.0;
const CHORD_V: f64 = 5.0;

/// Where the chord meets the slanted edges, by the hexagon's geometry.
fn chord_reach() -> f64 {
    let apothem = R * 0.75f64.sqrt();
    R - (R / 2.0) * (CHORD_V / apothem)
}

/// The truncated hexagon's area: the hexagon less the cap above the chord.
fn expected_area() -> f64 {
    let apothem = R * 0.75f64.sqrt();
    let hexagon = 1.5 * 3f64.sqrt() * R * R;
    let cap = (R + 2.0 * chord_reach()) / 2.0 * (apothem - CHORD_V);
    hexagon - cap
}

/// A part with one empty sketch on the Top plane and nothing built.
fn empty_sketch() -> Document {
    let mut doc = Document::new("Modify tools", soliddraft_app::kernel());
    let plane = SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 };
    doc.apply(Command::AddSketch { plane }).unwrap();
    doc.selection = None;
    doc
}

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().expect("a sketch is open")
}

fn entity(app: &App, id: EntityId) -> Option<Entity> {
    session(app).sketch.entity(id).copied()
}

fn drawn(app: &App) -> usize {
    session(app).sketch.drawn().count()
}

/// Screen position of a point of the open sketch's plane.
fn sketch_screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    let s = st.sketch.as_ref().expect("a sketch is open");
    Frame { plane: s.plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).expect("point on screen")
}

fn sketch_click(app: &mut App, p: P2) {
    let pos = sketch_screen(app, p);
    ribbon::click_at(app, pos);
}

fn hover_sketch(app: &mut App, p: P2) {
    let pos = sketch_screen(app, p);
    ribbon::render_steps::rendered(app, |app| {
        app.hover_at(pos);
        app.run_steps(3);
    });
}

/// A right-click where the pointer is, as a user ends a chain or puts a tool down.
fn right_click(app: &mut App, p: P2) {
    let pos = sketch_screen(app, p);
    app.hover_at(pos);
    app.run_steps(2);
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::default(),
        });
        app.step();
    }
    app.run_steps(3);
}

fn escape(app: &mut App) {
    app.key_press(Key::Escape);
    app.run_steps(4);
}

fn open_sketch(app: &mut App) {
    let name = app.state().state.doc.features[0].name.clone();
    let view = app.state().state.viewport.rect;
    let left = egui::Rect::from_min_max(egui::pos2(0.0, view.top()), egui::pos2(view.left(), view.bottom()));
    let row = ribbon::button_rect(app, &name, left);
    ribbon::double_click_at(app, row.center());
    app.run_steps(12);
    settle(app);
    assert!(app.state().state.sketch.is_some(), "the sketch is open");
}

fn near(a: P2, b: P2, tolerance: f64) -> bool {
    a.dist(b) <= tolerance
}

fn line_ends(kind: EntityKind) -> (P2, P2) {
    match kind {
        EntityKind::Line { a, b } => (a, b),
        other => panic!("not a line: {other:?}"),
    }
}

/// The polylines the last frame drew inside the viewport, with their point counts.
fn viewport_paths(app: &App) -> Vec<usize> {
    fn walk(shape: &Shape, clip: egui::Rect, out: &mut Vec<usize>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, clip, out)),
            Shape::Path(path) if path.points.iter().all(|p| clip.contains(*p)) => out.push(path.points.len()),
            _ => {}
        }
    }
    let view = app.state().state.viewport.rect;
    let mut out = Vec::new();
    for clipped in &app.output().shapes {
        walk(&clipped.shape, view, &mut out);
    }
    out
}

#[test]
fn polygon_extend_and_trim_by_pointer_make_a_profile_that_extrudes() {
    let mut case = Case::new("sketch-modify");
    let mut app = launch(empty_sketch(), STANDARD);
    case.run(&mut app, |case, app| {
        open_sketch(app);
        assert_eq!(app.state().state.ribbon_tab, soliddraft_app::state::RibbonTab::Sketch, "the Sketch tab opens");
        for label in ["Polygon", "Trim", "Extend", "Offset", "Mirror entities", "Linear pattern", "Circular pattern"] {
            let button = ribbon::ribbon_button(app, label);
            assert!(button.enabled, "{label} is enabled in a sketch");
        }
        case.frame(app, "01-sketch-tab");

        // Polygon: the centre on the origin, the vertex under the pointer, previewed live.
        ribbon::click_ribbon(app, "Polygon");
        assert_eq!(session(app).tool, Tool::Polygon);
        assert!(ribbon::ribbon_button(app, "Polygon").selected, "the tile is lit");
        sketch_click(app, P2::ZERO);
        assert_eq!(session(app).pending.len(), 1, "the centre is placed");
        hover_sketch(app, P2::new(R, 0.0));
        assert!(viewport_paths(app).contains(&7), "a closed six-sided preview follows the pointer");
        case.frame(app, "02-polygon-preview");

        // Escape puts the tool down before it leaves the sketch, and nothing was drawn.
        escape(app);
        assert_eq!(session(app).tool, Tool::Select, "Escape returns to Select");
        assert!(session(app).pending.is_empty());
        assert_eq!(drawn(app), 0, "a cancelled polygon adds nothing");
        assert!(app.state().state.sketch.is_some(), "the sketch stays open");

        ribbon::click_ribbon(app, "Polygon");
        sketch_click(app, P2::ZERO);
        sketch_click(app, P2::new(R, 0.0));
        assert_eq!(drawn(app), 6, "six sides");
        assert_eq!(session(app).tool, Tool::Polygon, "the tool stays for the next polygon");
        let (a, _) = line_ends(entity(app, 1).expect("the first side").kind);
        assert!(near(a, P2::new(R, 0.0), 0.05), "the first vertex is under the pointer: {a:?}");
        let (top_a, top_b) = line_ends(entity(app, 2).expect("the top side").kind);
        assert!((top_a.v - top_b.v).abs() < 1e-6, "the second side is level");
        case.frame(app, "03-polygon");
        // Zoom to fit frames the hexagon, so the tools work at a readable scale.
        app.key_press(Key::Z);
        settle(app);

        // A chord inside the hexagon, by the Line tool; the right-click ends the chain.
        ribbon::click_ribbon(app, "Line");
        sketch_click(app, P2::new(-10.0, CHORD_V));
        sketch_click(app, P2::new(10.0, CHORD_V));
        right_click(app, P2::new(10.0, CHORD_V));
        assert_eq!(drawn(app), 7);
        assert_eq!(session(app).tool, Tool::Line, "the right-click ends the chain and keeps the tool");
        let (a, b) = line_ends(entity(app, 7).expect("the chord").kind);
        assert!(near(a, P2::new(-10.0, CHORD_V), 0.05) && near(b, P2::new(10.0, CHORD_V), 0.05), "{a:?} {b:?}");

        // Extend: the end nearer the pointer reaches the slanted edge, previewed first.
        ribbon::click_ribbon(app, "Extend");
        hover_sketch(app, P2::new(-9.0, CHORD_V));
        let preview = session(app).preview.clone().expect("a preview at the chord");
        assert_eq!(preview.operation, Some(Operation::Extend { entity: 7, end: End::Start, boundary: 3 }));
        assert_eq!(preview.boundary, Some(3));
        let (pa, _) = line_ends(preview.added[0]);
        assert!(near(pa, P2::new(-chord_reach(), CHORD_V), 0.05), "the preview reaches the edge: {pa:?}");
        case.frame(app, "04-extend-preview");
        sketch_click(app, P2::new(-9.0, CHORD_V));
        let (a, _) = line_ends(entity(app, 7).unwrap().kind);
        assert!(near(a, P2::new(-chord_reach(), CHORD_V), 0.05), "the start moved: {a:?}");
        hover_sketch(app, P2::new(9.0, CHORD_V));
        let preview = session(app).preview.clone().expect("a preview at the other end");
        assert_eq!(preview.operation, Some(Operation::Extend { entity: 7, end: End::End, boundary: 1 }));
        sketch_click(app, P2::new(9.0, CHORD_V));
        let (_, b) = line_ends(entity(app, 7).unwrap().kind);
        assert!(near(b, P2::new(chord_reach(), CHORD_V), 0.05), "the end moved: {b:?}");
        assert_eq!(drawn(app), 7, "extend keeps the entity");
        case.frame(app, "05-extended");

        // A copy tool previews from the selection and the panel's values; Escape drops it.
        app.key_press(Key::V);
        app.run_steps(3);
        assert_eq!(session(app).tool, Tool::Select, "V picks Select");
        sketch_click(app, P2::new(3.0, CHORD_V));
        assert_eq!(session(app).selected_entities(), vec![7], "the chord is selected");
        ribbon::click_ribbon(app, "Linear pattern");
        assert_eq!(session(app).selected_entities(), vec![7], "a copy tool keeps the selection");
        app.run_steps(2);
        let preview = session(app).preview.clone().expect("a pattern preview");
        assert_eq!(preview.added.len(), 2, "two copies of the chord at the default count");
        let (ca, _) = line_ends(preview.added[0]);
        assert!(near(ca, P2::new(-chord_reach() + 20.0, CHORD_V), 0.05), "the copy sits one step over: {ca:?}");
        case.frame(app, "06-linear-pattern-preview");
        escape(app);
        assert_eq!(session(app).tool, Tool::Select);
        assert!(session(app).preview.is_none());
        assert_eq!(drawn(app), 7, "a cancelled pattern adds nothing");

        // Trim: the piece under the pointer is shown before it goes.
        ribbon::click_ribbon(app, "Trim");
        let apothem = R * 0.75f64.sqrt();
        hover_sketch(app, P2::new(0.0, apothem));
        let preview = session(app).preview.clone().expect("a trim preview on the top side");
        assert_eq!(preview.target, Some(2));
        assert_eq!(preview.removed.len(), 1);
        let (ra, rb) = line_ends(preview.removed[0]);
        assert!((ra.v - apothem).abs() < 0.05 && (rb.v - apothem).abs() < 0.05, "the whole top side goes");
        assert!(preview.added.is_empty(), "nothing of it stays");
        case.frame(app, "07-trim-preview");
        sketch_click(app, P2::new(0.0, apothem));
        assert!(entity(app, 2).is_none(), "the top side is gone");
        assert_eq!(drawn(app), 6);
        for (u, side) in [(12.5, 1), (-12.5, 3)] {
            hover_sketch(app, P2::new(u, 13.0));
            let preview = session(app).preview.clone().expect("a trim preview on a slanted side");
            assert_eq!(preview.target, Some(side));
            let (ra, rb) = line_ends(preview.removed[0]);
            let (lo, hi) = if ra.v < rb.v { (ra, rb) } else { (rb, ra) };
            assert!((lo.v - CHORD_V).abs() < 0.05, "the piece starts at the chord: {lo:?}");
            assert!((hi.v - apothem).abs() < 0.05, "and ends at the top: {hi:?}");
            sketch_click(app, P2::new(u, 13.0));
            assert!(entity(app, side).is_some(), "the lower piece keeps the side's id");
        }
        assert_eq!(drawn(app), 6, "six sides again, one of them the chord");
        let regions = profile::regions(&session(app).sketch).expect("the profile closes");
        assert_eq!(regions.len(), 1);
        let area: f64 = regions.iter().map(|r| r.area()).sum();
        assert!((area - expected_area()).abs() < 0.2, "area {area}, expected {}", expected_area());
        case.frame(app, "08-profile");

        // Escape puts Trim down; Extrude leaves the sketch into the boss, Enter builds it.
        escape(app);
        assert_eq!(session(app).tool, Tool::Select);
        assert!(app.state().state.sketch.is_some());
        ribbon::click_ribbon(app, "Extrude");
        settle(app);
        assert!(app.state().state.sketch.is_none(), "the sketch is left");
        assert!(app.state().state.editor.is_some(), "the extrude editor is open");
        app.key_press(Key::Enter);
        settle(app);
        rebuilt(app);
        let found = facts(app);
        let expected = expected_area() * 10.0;
        assert!((found.volume - expected).abs() < 1.0, "volume {} for {expected}", found.volume);
        assert!((found.max[2] - 10.0).abs() < 1e-3, "height {:?}", found.max);
        assert_eq!(found.topology, (8, 18, 12), "six sides, two caps");
        let doc = &app.state().state.doc;
        assert_eq!(doc.error_count(), 0);
        assert_eq!(doc.features.len(), 2, "the sketch and its boss");
        case.note(format!("volume {} mm^3, expected {expected}", found.volume));
        settle(app);
        case.frame(app, "09-extruded");
    });
}
