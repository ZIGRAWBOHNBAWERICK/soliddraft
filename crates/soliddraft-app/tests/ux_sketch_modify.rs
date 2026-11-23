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
use egui_kittest::kittest::Queryable;
use soliddraft_app::document::{Command, Document, PlaneKind, SketchPlane};
use soliddraft_app::sketch::tools::{End, Operation};
use soliddraft_app::sketch::{Entity, EntityId, EntityKind, P2, profile};
use soliddraft_app::sketcher::Frame;
use soliddraft_app::state::{SketchSession, Tool};

#[path = "support/ui.rs"]
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
    session(app).sketch.entity(id).cloned()
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
    ribbon::open_tree(app);
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
        for id in [
            "sketch_polygon",
            "sketch_trim",
            "sketch_extend",
            "sketch_offset",
            "sketch_mirror",
            "sketch_linear_pattern",
            "sketch_circular_pattern",
        ] {
            let button = ribbon::reveal_command(app, id);
            assert!(button.enabled, "{id} is enabled in a sketch");
            ribbon::click_at(app, button.rect.center());
            assert_eq!(
                session(app).tool.label(),
                soliddraft_app::commands::get(id).unwrap().label,
                "{id} runs its registered tool"
            );
        }
        case.frame(app, "01-sketch-tab");

        // Polygon: the centre on the origin, the vertex under the pointer, previewed live.
        ribbon::run_command(app, "sketch_polygon");
        assert_eq!(session(app).tool, Tool::Polygon);
        assert!(ribbon::reveal_command(app, "sketch_polygon").selected, "the tile is lit");

        assert_eq!(session(app).tool, Tool::Polygon, "inspecting the tile keeps the tool active");
        assert_eq!(session(app).tool, Tool::Polygon, "inspecting the tile keeps the tool active");
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

        ribbon::run_command(app, "sketch_polygon");
        sketch_click(app, P2::ZERO);
        sketch_click(app, P2::new(R, 0.0));
        assert_eq!(drawn(app), 7, "six sides and their construction circle");
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
        ribbon::run_command(app, "sketch_line");
        sketch_click(app, P2::new(-10.0, CHORD_V));
        sketch_click(app, P2::new(10.0, CHORD_V));
        right_click(app, P2::new(10.0, CHORD_V));
        assert_eq!(drawn(app), 8);
        assert_eq!(session(app).tool, Tool::Line, "the right-click ends the chain and keeps the tool");
        let (a, b) = line_ends(entity(app, 8).expect("the chord").kind);
        assert!(near(a, P2::new(-10.0, CHORD_V), 0.05) && near(b, P2::new(10.0, CHORD_V), 0.05), "{a:?} {b:?}");

        // Extend: the end nearer the pointer reaches the slanted edge, previewed first.
        ribbon::run_command(app, "sketch_extend");
        hover_sketch(app, P2::new(-9.0, CHORD_V));
        let preview = session(app).preview.clone().expect("a preview at the chord");
        assert_eq!(preview.operation, Some(Operation::Extend { entity: 8, end: End::Start, boundary: 3 }));
        assert_eq!(preview.boundary, Some(3));
        let (pa, _) = line_ends(preview.added[0].clone());
        assert!(near(pa, P2::new(-chord_reach(), CHORD_V), 0.05), "the preview reaches the edge: {pa:?}");
        case.frame(app, "04-extend-preview");
        sketch_click(app, P2::new(-9.0, CHORD_V));
        let (a, _) = line_ends(entity(app, 8).unwrap().kind);
        assert!(near(a, P2::new(-chord_reach(), CHORD_V), 0.05), "the start moved: {a:?}");
        hover_sketch(app, P2::new(9.0, CHORD_V));
        let preview = session(app).preview.clone().expect("a preview at the other end");
        assert_eq!(preview.operation, Some(Operation::Extend { entity: 8, end: End::End, boundary: 1 }));
        sketch_click(app, P2::new(9.0, CHORD_V));
        let (_, b) = line_ends(entity(app, 8).unwrap().kind);
        assert!(near(b, P2::new(chord_reach(), CHORD_V), 0.05), "the end moved: {b:?}");
        assert_eq!(drawn(app), 8, "extend keeps the entity");
        case.frame(app, "05-extended");

        // A copy tool previews from the selection and the panel's values; Escape drops it.
        app.key_press(Key::V);
        app.run_steps(3);
        assert_eq!(session(app).tool, Tool::Select, "V picks Select");
        sketch_click(app, P2::new(3.0, CHORD_V));
        assert_eq!(session(app).selected_entities(), vec![8], "the chord is selected");
        ribbon::run_command(app, "sketch_linear_pattern");
        assert_eq!(session(app).selected_entities(), vec![8], "a copy tool keeps the selection");
        app.run_steps(2);
        let preview = session(app).preview.clone().expect("a pattern preview");
        assert_eq!(preview.added.len(), 2, "two copies of the chord at the default count");
        let (ca, _) = line_ends(preview.added[0].clone());
        assert!(near(ca, P2::new(-chord_reach() + 20.0, CHORD_V), 0.05), "the copy sits one step over: {ca:?}");
        case.frame(app, "06-linear-pattern-preview");
        escape(app);
        assert_eq!(session(app).tool, Tool::Select);
        assert!(session(app).preview.is_none());
        assert_eq!(drawn(app), 8, "a cancelled pattern adds nothing");

        // Trim: the piece under the pointer is shown before it goes.
        ribbon::run_command(app, "sketch_trim");
        let apothem = R * 0.75f64.sqrt();
        hover_sketch(app, P2::new(0.0, apothem));
        let preview = session(app).preview.clone().expect("a trim preview on the top side");
        assert_eq!(preview.target, Some(2));
        assert_eq!(preview.removed.len(), 1);
        let (ra, rb) = line_ends(preview.removed[0].clone());
        assert!((ra.v - apothem).abs() < 0.05 && (rb.v - apothem).abs() < 0.05, "the whole top side goes");
        assert!(preview.added.is_empty(), "nothing of it stays");
        case.frame(app, "07-trim-preview");
        sketch_click(app, P2::new(0.0, apothem));
        assert!(entity(app, 2).is_none(), "the top side is gone");
        assert_eq!(drawn(app), 7);
        for (u, side) in [(12.5, 1), (-12.5, 3)] {
            hover_sketch(app, P2::new(u, 13.0));
            let preview = session(app).preview.clone().expect("a trim preview on a slanted side");
            assert_eq!(preview.target, Some(side));
            let (ra, rb) = line_ends(preview.removed[0].clone());
            let (lo, hi) = if ra.v < rb.v { (ra, rb) } else { (rb, ra) };
            assert!((lo.v - CHORD_V).abs() < 0.05, "the piece starts at the chord: {lo:?}");
            assert!((hi.v - apothem).abs() < 0.05, "and ends at the top: {hi:?}");
            sketch_click(app, P2::new(u, 13.0));
            assert!(entity(app, side).is_some(), "the lower piece keeps the side's id");
        }
        assert_eq!(drawn(app), 7, "six profile sides and a construction circle");
        let regions = profile::regions(&session(app).sketch).expect("the profile closes");
        assert_eq!(regions.len(), 1);
        let area: f64 = regions.iter().map(|r| r.area()).sum();
        assert!((area - expected_area()).abs() < 0.2, "area {area}, expected {}", expected_area());
        case.frame(app, "08-profile");

        // Put Trim down, close the sketch, then start the boss and build it.
        escape(app);
        assert_eq!(session(app).tool, Tool::Select);
        assert!(app.state().state.sketch.is_some());
        ribbon::run_command(app, "exit_sketch");
        assert!(app.state().state.editor.is_none());
        ribbon::run_command(app, "extrude");
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

fn options_bar(app: &App) -> egui::Rect {
    soliddraft_app::options_bar::shown_rect(&app.ctx).expect("the options bar is drawn")
}

/// The sketch walk-over's fixes, as a user meets them. The Polygon tool shows its sides
/// and circle in the options bar while it is active, and the next polygon takes them.
/// The sketch switches sit in that bar, never over the sketch. Escape drops the tool,
/// then clears the selection, and never leaves the sketch; Close sketch does. Smart
/// Dimension marks its pick and draws the dimension at the pointer before the placing
/// click. The options panel lists no icon grid of the tools the palette already names.
#[test]
fn sketch_options_escape_and_smart_dimension_preview() {
    let mut case = Case::new("sketch-modify/walkover");
    let mut app = launch(empty_sketch(), STANDARD);
    case.run(&mut app, |case, app| {
        open_sketch(app);
        let bar = options_bar(app);
        let view = app.state().state.viewport.rect;
        for switch in ["Auto relations", "Draw construction", "Snap to grid"] {
            let nodes: Vec<_> = app.query_all_by_label(switch).filter(|n| bar.contains_rect(n.rect())).collect();
            assert_eq!(nodes.len(), 1, "{switch} is in the options bar");
            assert!(
                app.query_all_by_label(switch).all(|n| !view.contains(n.rect().center())),
                "{switch} no longer covers the sketch"
            );
        }

        // Polygon: five sides from the options bar.
        ribbon::run_command(app, "sketch_polygon");
        settle(app);
        let sides = app.get_by_label("Polygon sides");
        assert!(bar.contains_rect(sides.rect()), "the sides are in the options bar while Polygon is active");
        assert!(app.query_all_by_label("Circumscribed").any(|n| bar.contains_rect(n.rect())));
        app.get_by_label("Polygon sides").focus();
        app.run_steps(2);
        app.key_press_modifiers(Modifiers::COMMAND, Key::A);
        app.get_by_label("Polygon sides").type_text("5");
        app.key_press(Key::Enter);
        app.run_steps(4);
        assert_eq!(session(app).modify.sides, 5);
        case.frame(app, "01-polygon-sides-in-the-bar");
        sketch_click(app, P2::ZERO);
        sketch_click(app, P2::new(R, 0.0));
        assert_eq!(drawn(app), 6, "five sides and their construction circle");

        // Escape: the tool goes, then the selection; the sketch stays open.
        escape(app);
        assert_eq!(session(app).tool, Tool::Select);
        assert!(app.state().state.sketch.is_some(), "the first Escape keeps the sketch");
        sketch_click(app, P2::new(R * 0.9, R * 0.3));
        assert!(!session(app).selection.is_empty(), "a side is selected");
        escape(app);
        assert!(session(app).selection.is_empty(), "the second Escape clears the selection");
        escape(app);
        assert!(app.state().state.sketch.is_some(), "Escape never closes the sketch");
        case.frame(app, "02-escape-keeps-the-sketch");

        // Smart Dimension: the pick is marked and the value follows the pointer.
        ribbon::run_command(app, "smart_dimension");
        settle(app);
        let side = session(app).sketch.entities.iter().find(|e| e.is_line() && !e.construction).unwrap().clone();
        let (a, b) = line_ends(side.kind.clone());
        let middle = P2::new((a.u + b.u) / 2.0, (a.v + b.v) / 2.0);
        sketch_click(app, middle);
        assert!(session(app).dim_first.is_some(), "the side is picked");
        let away = P2::new(middle.u * 1.6, middle.v * 1.6);
        hover_sketch(app, away);
        let at = sketch_screen(app, away);
        // A value pill at the pointer: the dimension the placing click would make.
        let pill = ribbon::painted(app)
            .into_iter()
            .find(|p| p.text.parse::<f64>().is_ok_and(|v| v > 0.0) && p.rect.center().distance(at) < 40.0)
            .unwrap_or_else(|| panic!("a dimension follows the pointer to {at:?}: {:?}", ribbon::painted(app)));
        case.frame(app, "03-smart-dimension-follows-the-pointer");
        let length = ((b.u - a.u).powi(2) + (b.v - a.v).powi(2)).sqrt();
        assert!(pill.text.parse::<f64>().unwrap() <= length + 0.01, "{} for a side of {length}", pill.text);
        escape(app);
        escape(app);

        // The options panel: the palette names the tools, so the panel has no icon grid.
        ribbon::open_options(app);
        assert!(app.query_all_by_label("Line (L)").next().is_none(), "no unlabelled tool grid in the options panel");
        assert!(app.query_by_label("Sketch diagnostics").is_some());
        case.frame(app, "04-options-panel");
        ribbon::close_options(app);

        // Close sketch leaves.
        let close = ribbon::control(app, "Close sketch", options_bar(app)).rect.center();
        ribbon::click_at(app, close);
        settle(app);
        assert!(app.state().state.sketch.is_none(), "Close sketch leaves the sketch");
    });
}
