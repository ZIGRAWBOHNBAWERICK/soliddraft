//! The P1 bracket's fourteen items of manufacturing information put on the part by hand
//! in the real app: the Manufacturing information panel is opened from the Inspect tab,
//! every face is picked in the viewport and every value is typed. The panel then lists
//! each item as a drawing writes it, with the distances that locate the bore and the
//! holes, and the part writes all fourteen into its STEP file. Frames land under
//! `target/ribbon-acceptance/ux-pmi-panel/`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::Pos2;
use egui_kittest::kittest::Queryable;
use glam::Vec3;
use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Document, Selection};
use soliddraft_app::state::{AppAction, AppState};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_api::{FaceId, SurfaceKind};

#[path = "support/ui.rs"]
mod ribbon;

use ribbon::App;

const HANG_GUARD: Duration = Duration::from_secs(600);
const VIEWS: [&str; 7] =
    ["view_isometric", "view_front", "view_back", "view_left", "view_right", "view_top", "view_bottom"];

/// The bracket, built with the canonical scenario's steps.
fn bracket() -> Document {
    let kernel = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(20_000_000));
    let mut state = AppState::new(Document::new("P1 bracket", kernel.clone()));
    let scenario: Value = serde_json::from_str(include_str!("../../../ci/scenarios/canonical-p1.json")).unwrap();
    for step in scenario["steps"].as_array().unwrap() {
        let tool = step["tool"].as_str().unwrap();
        let caller = Caller::Mcp { client: "ux-pmi-panel".into() };
        let out = call_checked(&mut AppTools { state: &mut state }, tool, step["arguments"].clone(), &caller);
        assert!(!out.is_error, "{tool}: {}", out.content);
        if step["label"] == "original" {
            break;
        }
    }
    std::mem::replace(&mut state.doc, Document::new("Empty", kernel))
}

fn settle(app: &mut App) {
    let start = Instant::now();
    while app.state().state.doc.proof_pending() {
        assert!(start.elapsed() < HANG_GUARD, "still rebuilding");
        app.step();
    }
    app.run_steps(20);
}

/// The one face of `surface` whose centroid is at `at`; `None` is any value.
fn face(app: &App, surface: SurfaceKind, at: [Option<f64>; 3]) -> FaceId {
    let found: Vec<FaceId> = app
        .state()
        .state
        .doc
        .faces
        .iter()
        .filter(|f| f.surface == surface)
        .filter(|f| {
            let c = [f.centroid.x, f.centroid.y, f.centroid.z].map(f64::from);
            at.iter().zip(c).all(|(want, have)| want.is_none_or(|want| (want - have).abs() < 1e-3))
        })
        .map(|f| f.id)
        .collect();
    assert_eq!(found.len(), 1, "one {surface:?} face at {at:?}, found {}", found.len());
    found[0]
}

/// A face to pick: which face, and points of it to try clicking.
struct Target {
    face: FaceId,
    points: Vec<Vec3>,
}

fn plane(app: &App, at: [Option<f64>; 3], points: &[[f32; 3]]) -> Target {
    Target { face: face(app, SurfaceKind::Plane, at), points: points.iter().copied().map(Vec3::from).collect() }
}

/// A round face about an axis along `axis` (0 for x, 1 for y, 2 for z) through `centre`:
/// points all round its wall at a few places along the axis.
fn round(app: &App, at: [Option<f64>; 3], axis: usize, centre: [f32; 3], radius: f32, along: &[f32]) -> Target {
    let mut points = Vec::new();
    for &place in along {
        for step in 0..12 {
            let angle = step as f32 * std::f32::consts::TAU / 12.0;
            let (a, b) = (radius * angle.cos(), radius * angle.sin());
            let mut point = centre;
            point[axis] = place;
            point[(axis + 1) % 3] += a;
            point[(axis + 2) % 3] += b;
            points.push(Vec3::from(point));
        }
    }
    Target { face: face(app, SurfaceKind::Cylinder, at), points }
}

/// A way to look at the part: a standard view, or the part turned by hand to a yaw and
/// a pitch in degrees.
#[derive(Clone, Copy, Debug)]
enum Look {
    View(&'static str),
    Turned(f32, f32),
}

fn look(app: &mut App, look: Look) {
    match look {
        Look::View(view) => ribbon::run_command(app, view),
        Look::Turned(yaw, pitch) => {
            let camera = &mut app.state_mut().state.viewport.camera;
            camera.orbit(0.0, 0.0);
            camera.yaw = yaw.to_radians();
            camera.pitch = pitch.to_radians();
        }
    }
    ribbon::run_command(app, "zoom_fit");
    settle(app);
}

/// Clicks a point of the part if it is on screen, and says whether that selected `face`.
fn try_point(app: &mut App, face: FaceId, world: Vec3) -> Option<(Pos2, bool)> {
    // Nothing selected: a selection shows its feature's dimensions over the part, and a
    // click on one of those is not a click on a face. The pause keeps two clicks from
    // reading as a double click.
    app.state_mut().state.push(AppAction::Select(None));
    app.run_steps(30);
    let rect = app.state().state.viewport.rect;
    let at = app.state().state.viewport.camera.project(world, rect).filter(|at| rect.contains(*at))?;
    ribbon::pick_at(app, at);
    app.run_steps(2);
    Some((at, app.state().state.doc.selection == Some(Selection::Face(face))))
}

thread_local! {
    /// Where each face was found last time, so a face picked twice is searched once.
    static FOUND: std::cell::RefCell<std::collections::BTreeMap<FaceId, (Look, Vec3)>> = Default::default();
}

/// Finds a view and a screen point where a click selects the target's face, and leaves
/// the app in that view.
fn find(app: &mut App, target: &Target) -> Pos2 {
    if let Some((known, world)) = FOUND.with(|found| found.borrow().get(&target.face).copied()) {
        look(app, known);
        if let Some((at, true)) = try_point(app, target.face, world) {
            return at;
        }
    }
    // The standard views, then the part turned by hand to look into what they hide,
    // such as the underside of a slot.
    let turned = [-90.0_f32, 90.0, -45.0, -135.0, 45.0, 135.0, 0.0, 180.0]
        .into_iter()
        .flat_map(|yaw| [-25.0_f32, 25.0].map(|pitch| Look::Turned(yaw, pitch)));
    let mut seen = Vec::new();
    for way in VIEWS.into_iter().map(Look::View).chain(turned) {
        look(app, way);
        for world in &target.points {
            match try_point(app, target.face, *world) {
                Some((at, true)) => {
                    FOUND.with(|found| found.borrow_mut().insert(target.face, (way, *world)));
                    return at;
                }
                Some((_, false)) => {
                    let doc = &app.state().state.doc;
                    let title = doc.selection.and_then(|s| doc.describe(s)).map_or("nothing".into(), |d| d.title);
                    seen.push(format!("{way:?}: {title}"));
                }
                None => {}
            }
        }
    }
    let wanted = app.state().state.doc.describe(Selection::Face(target.face)).map(|d| d.title);
    panic!("no view shows a point of {wanted:?}; the clicks selected {seen:?}");
}

/// Picks the target's face in the viewport, as a person does.
fn pick(app: &mut App, target: &Target) {
    // The face is found with the panel closed, so a miss on the way picks nothing.
    let open = app.state().state.pmi.open;
    if open {
        ribbon::run_command(app, "manufacturing_information");
    }
    let at = find(app, target);
    app.state_mut().state.push(AppAction::Select(None));
    app.run_steps(4);
    if open {
        ribbon::run_command(app, "manufacturing_information");
    }
    ribbon::pick_at(app, at);
    app.run_steps(4);
    assert_eq!(app.state().state.pmi.picks.last(), Some(&target.face), "the face is picked");
}

fn press(app: &mut App, label: &str) {
    ribbon::click_button(app, label, ribbon::window(app));
    app.run_steps(4);
}

fn add(app: &mut App) {
    let before = app.state().state.doc.model().pmi().items.len();
    press(app, "Add");
    let state = &app.state().state;
    assert_eq!(state.doc.model().pmi().items.len(), before + 1, "the item is added");
    assert!(state.pmi.picks.is_empty(), "the form is empty again");
}

fn callouts(app: &App) -> Vec<(String, String)> {
    app.state()
        .state
        .doc
        .model()
        .pmi_rows()
        .into_iter()
        .map(|row| {
            assert_eq!(row.problem, None, "{}", row.item.id);
            (row.item.id.clone(), row.callout())
        })
        .collect()
}

#[test]
fn a_person_puts_the_bracket_s_fourteen_items_on_its_faces() {
    let case = ribbon::case("ux-pmi-panel");
    let mut app = ribbon::app(bracket());
    case.run(&mut app, |app| {
        settle(app);
        ribbon::switch_workspace(app, "inspect");
        ribbon::run_command(app, "manufacturing_information");
        assert!(app.state().state.pmi.open);

        let a = plane(app, [None, None, Some(0.0)], &[[20.0, 20.0, 0.0], [42.0, 8.0, 0.0]]);
        let b = plane(app, [None, Some(0.0), None], &[[10.0, 0.0, 15.0], [50.0, 0.0, 26.0], [8.0, 0.0, 2.0]]);
        let c = plane(
            app,
            [Some(0.0), None, None],
            &[[0.0, 20.0, 2.5], [0.0, 31.0, 3.5], [0.0, 2.5, 22.0], [0.0, 1.5, 12.0], [0.0, 36.0, 1.5]],
        );
        let bore = round(app, [Some(30.0), Some(25.0), Some(7.5)], 2, [30.0, 25.0, 0.0], 4.0, &[13.0, 9.0]);
        let boss = round(app, [Some(30.0), Some(25.0), Some(10.0)], 2, [30.0, 25.0, 0.0], 8.0, &[10.0]);
        let boss_top = plane(
            app,
            [Some(30.0), Some(25.0), Some(15.0)],
            &[[36.0, 25.0, 15.0], [30.0, 31.0, 15.0], [24.5, 25.0, 15.0]],
        );
        let holes: Vec<Target> = [[10.0, 12.0], [10.0, 32.0], [50.0, 12.0], [50.0, 32.0]]
            .into_iter()
            .map(|[x, y]| round(app, [Some(x), Some(y), Some(2.5)], 2, [x as f32, y as f32, 0.0], 2.0, &[4.0, 1.0]))
            .collect();
        let across = |z: f32| [[30.0, 0.6, z], [26.0, 2.5, z], [34.0, 4.2, z], [30.0, 4.4, z]];
        let slot_low = plane(app, [Some(30.0), Some(2.5), Some(15.0)], &across(15.0));
        let slot_high = plane(app, [Some(30.0), Some(2.5), Some(21.0)], &across(21.0));
        // The slot's ends are half cylinders about x = 23 and x = 37 at z = 18.
        let end = |app: &App, low: bool| {
            let found: Vec<FaceId> = app
                .state()
                .state
                .doc
                .faces
                .iter()
                .filter(|f| f.surface == SurfaceKind::Cylinder && (f.centroid.z - 18.0).abs() < 1e-3)
                .filter(|f| (f.centroid.x < 30.0) == low)
                .map(|f| f.id)
                .collect();
            assert_eq!(found.len(), 1, "one slot end");
            let x = if low { 23.0 } else { 37.0 };
            let points =
                [1.0_f32, 4.0].into_iter().map(|y| Vec3::new(if low { x - 3.0 } else { x + 3.0 }, y, 18.0)).collect();
            Target { face: found[0], points }
        };
        let slot_ends = [end(app, true), end(app, false)];

        // Datums A, B and C. The letter is the next free one.
        for datum in [&a, &b, &c] {
            pick(app, datum);
            add(app);
        }
        case.frame(app, "01-datums");

        // The bore: its diameter with a fit, its finish and its position.
        press(app, "Diameter");
        pick(app, &bore);
        ribbon::type_field(app, "Tolerance", "H7");
        ribbon::type_field(app, "Item name", "Bore diameter");
        add(app);
        press(app, "Surface finish");
        pick(app, &bore);
        ribbon::type_field(app, "Roughness", "Ra 1.6");
        ribbon::type_field(app, "Item name", "Bore finish");
        add(app);
        press(app, "Geometric tolerance");
        pick(app, &bore);
        ribbon::type_field(app, "Tolerance value", "0.05");
        app.get_by_label("Cylindrical zone (\u{d8})").click();
        app.run_steps(3);
        ribbon::type_field(app, "Datums", "A B C");
        ribbon::type_field(app, "Item name", "Bore position");
        add(app);

        // The boss: its diameter, and its top from datum A.
        press(app, "Diameter");
        pick(app, &boss);
        ribbon::type_field(app, "Tolerance", "\u{b1}0.1");
        ribbon::type_field(app, "Item name", "Boss diameter");
        add(app);
        press(app, "Distance");
        pick(app, &a);
        press(app, "From");
        pick(app, &boss_top);
        ribbon::type_field(app, "Tolerance", "\u{b1}0.1");
        ribbon::type_field(app, "Item name", "Boss height");
        add(app);
        case.frame(app, "02-bore-and-boss");

        // The four holes as a pattern: their diameter, through, and their position.
        press(app, "Diameter");
        for hole in &holes {
            pick(app, hole);
        }
        ribbon::type_field(app, "Tolerance", "+0.1/0");
        app.get_by_label("Through hole").click();
        app.run_steps(3);
        ribbon::type_field(app, "Item name", "Hole diameter");
        add(app);
        press(app, "Geometric tolerance");
        for hole in &holes {
            pick(app, hole);
        }
        ribbon::type_field(app, "Tolerance value", "0.2");
        app.get_by_label("Cylindrical zone (\u{d8})").click();
        app.run_steps(3);
        ribbon::type_field(app, "Datums", "A B C");
        ribbon::type_field(app, "Item name", "Hole position");
        add(app);

        // The slot as one item, and its centre from datum A.
        press(app, "Slot");
        for side in [&slot_low, &slot_high, &slot_ends[0], &slot_ends[1]] {
            pick(app, side);
        }
        ribbon::type_field(app, "Tolerance", "+0.1/0");
        ribbon::type_field(app, "Item name", "Slot");
        add(app);
        press(app, "Distance");
        pick(app, &a);
        press(app, "From");
        pick(app, &slot_low);
        pick(app, &slot_high);
        press(app, "Middle");
        ribbon::type_field(app, "Tolerance", "\u{b1}0.1");
        ribbon::type_field(app, "Item name", "Slot height");
        add(app);

        // Flatness of A and perpendicularity of B to A.
        press(app, "Geometric tolerance");
        pick(app, &a);
        app.get_by_label("Characteristic").click();
        app.run_steps(4);
        app.get_by_label("Flatness").click();
        app.run_steps(4);
        ribbon::type_field(app, "Tolerance value", "0.05");
        ribbon::type_field(app, "Item name", "Flatness of A");
        add(app);
        press(app, "Geometric tolerance");
        pick(app, &b);
        app.get_by_label("Characteristic").click();
        app.run_steps(4);
        app.get_by_label("Perpendicularity").click();
        app.run_steps(4);
        ribbon::type_field(app, "Tolerance value", "0.1");
        ribbon::type_field(app, "Datums", "A");
        ribbon::type_field(app, "Item name", "Perpendicularity of B");
        add(app);

        // The notes of the part.
        ribbon::type_field(app, "General tolerance", "ISO 2768-m");
        ribbon::type_field(app, "Material note", "6061-T6");
        ribbon::type_field(app, "Part notes", "Break sharp edges");
        press(app, "Apply notes");

        ribbon::run_command(app, "view_isometric");
        ribbon::run_command(app, "zoom_fit");
        settle(app);
        case.frame(app, "03-fourteen-items");

        let expected = [
            ("Datum A", "Datum A"),
            ("Datum B", "Datum B"),
            ("Datum C", "Datum C"),
            ("Bore diameter", "\u{d8}8 H7"),
            ("Bore finish", "Ra 1.6"),
            ("Bore position", "Position \u{d8}0.05 A|B|C"),
            ("Boss diameter", "\u{d8}16 \u{b1}0.1"),
            ("Boss height", "15 \u{b1}0.1"),
            ("Hole diameter", "4\u{d7} \u{d8}4 +0.1/0 THRU"),
            ("Hole position", "Position \u{d8}0.2 A|B|C"),
            ("Slot", "20\u{d7}6 +0.1/0 R3"),
            ("Slot height", "18 \u{b1}0.1"),
            ("Flatness of A", "Flatness 0.05"),
            ("Perpendicularity of B", "Perpendicularity 0.1 A"),
        ]
        .map(|(name, callout)| (name.to_owned(), callout.to_owned()));
        assert_eq!(callouts(app), expected);
        let doc = app.state().state.doc.model();
        let notes = doc.pmi().part.clone();
        assert_eq!(notes.general_tolerance.as_deref(), Some("ISO 2768-m"));
        assert_eq!(notes.material.as_deref(), Some("6061-T6"));
        assert_eq!(notes.notes, ["Break sharp edges"]);

        // All fourteen go into the STEP file, with the ten locating distances.
        let on_body = doc.pmi_on_body();
        assert_eq!(on_body.left_out, Vec::new());
        assert_eq!((on_body.items, on_body.pmi.items.len()), (14, 24));

        // C2 by hand is the same edit as anywhere else: the bore goes to 10 mm and the
        // panel reads it with its fit.
        let caller = Caller::Mcp { client: "ux-pmi-panel".into() };
        let edit = json!({"sketch": "Sketch4", "dimension": 1, "expression": "10"});
        let out = call_checked(&mut AppTools { state: &mut app.state_mut().state }, "drive", edit, &caller);
        assert!(!out.is_error, "{}", out.content);
        settle(app);
        assert_eq!(callouts(app)[3], ("Bore diameter".to_owned(), "\u{d8}10 H7".to_owned()));
        case.frame(app, "04-bore-edited");
    });
}
