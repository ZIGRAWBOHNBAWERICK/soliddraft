//! Feature form fields, keyboard input, pick lists and rendered fillet previews.

use egui::{Key, Rect};
use egui_kittest::kittest::{By, NodeT, Queryable};
use glam::Vec3;
use soliddraft_app::document::{Command, Document, PlaneKind, Selection, SketchPlane};
use soliddraft_app::state::{AppAction, EditorParams};
use soliddraft_app::theme;

#[path = "support/ui.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use ribbon::App;
use ux_tools::{Case, Look, SMALL_DARK, STANDARD, launch, pointer_to_chrome, rebuilt, settle};

/// The plate: 60 by 40 by 20 on Top, its top face at z = 20.
const TOP: f32 = 20.0;

/// The plate with a 16 mm circle sketched on its top face, ready to cut.
fn plate_with_circle() -> (Document, u64) {
    let mut doc = ribbon::plate();
    let sketch = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: TOP as f64 } })
        .unwrap()
        .unwrap();
    let mut drawing = soliddraft_sketch::Sketch::new();
    drawing.circle_at(soliddraft_sketch::P2::new(0.0, 0.0), 16.0).unwrap();
    doc.apply(Command::EditSketch { id: sketch, sketch: drawing }).unwrap();
    doc.selection = Some(Selection::Feature(sketch));
    (doc, sketch)
}

fn open(app: &mut App, action: AppAction) {
    app.state_mut().state.push(action);
    settle(app);
    pointer_to_chrome(app);
}

fn editor_title(app: &App) -> String {
    app.state().state.editor.as_ref().expect("a feature panel is open").title()
}

/// The value fields of the open form, top to bottom.
fn value_fields(app: &App) -> Vec<(String, Rect)> {
    let mut fields: Vec<(String, Rect)> = app
        .query_all(By::new().predicate(|n| n.label().is_some_and(|l| l.ends_with(" value"))))
        .map(|n| (n.accesskit_node().label().unwrap_or_default(), n.rect()))
        .filter(|(_, r)| ribbon::window(app).contains_rect(*r))
        .collect();
    fields.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
    fields
}

/// The active form exposes each expected field inside the window.
fn assert_fields(app: &App, what: &str, wanted: &[&str]) {
    let fields = value_fields(app);
    let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
    for w in wanted {
        assert!(names.contains(w), "{what}: the form has {w:?}: {names:?}");
    }
    for (name, rect) in fields.iter().filter(|(name, _)| wanted.contains(&name.as_str())) {
        assert!(rect.width() >= 36.0 && rect.height() >= 18.0, "{what}: readable {name}: {rect:?}");
    }
}

/// Whether a node of the tree reads `text`, as its label or its value.
fn shows(app: &App, text: &str) -> bool {
    app.query_all(By::new().predicate(|n| n.label().as_deref() == Some(text) || n.value().as_deref() == Some(text)))
        .next()
        .is_some()
}

fn focused_label(app: &App) -> Option<String> {
    app.query_all(By::new().predicate(|n| n.is_focused())).find_map(|n| n.accesskit_node().label())
}

/// The nearest of the plate's top edges to the eye, by their midpoints.
fn near_edges(app: &App) -> (Vec3, Vec3) {
    let eye = app.state().state.viewport.camera.eye();
    let nearest = |a: Vec3, b: Vec3| if a.distance(eye) < b.distance(eye) { a } else { b };
    (
        nearest(Vec3::new(0.0, 20.0, TOP), Vec3::new(0.0, -20.0, TOP)),
        nearest(Vec3::new(30.0, 0.0, TOP), Vec3::new(-30.0, 0.0, TOP)),
    )
}

fn color_distance(a: [u8; 4], b: egui::Color32) -> f32 {
    let d = |x: u8, y: u8| (x as f32 - y as f32).powi(2);
    (d(a[0], b.r()) + d(a[1], b.g()) + d(a[2], b.b())).sqrt()
}

/// How far a pixel is toward the accent: its distance to the accent, for comparisons.
fn pixel(image: &image::RgbaImage, app: &App, world: Vec3) -> [u8; 4] {
    let at = ux_tools::screen(app, world) * app.ctx.pixels_per_point();
    image.get_pixel(at.x as u32, at.y as u32).0
}

#[test]
fn extrude_numbers_accept_keyboard_input() {
    let mut case = Case::new("feature-panel/extrude");
    let mut app = launch(ribbon::blank(), STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartExtrude);
        assert_eq!(editor_title(app), "Boss-Extrude");
        case.frame(app, "extrude");
        assert_fields(app, "Boss-Extrude", &["Width value", "Height value", "Depth value"]);
        ribbon::open_options(app);
        assert!(app.query_by_label("Flip direction").is_some(), "the flip stays a labelled checkbox");
        assert!(app.query_by_label("OK").is_some() && app.query_by_label("Cancel").is_some());

        // Keyboard: Tab walks the numbers in reading order.
        let width = app.get_by_label("Width value").rect();
        ribbon::click_at(app, width.center());
        app.run_steps(2);
        assert_eq!(focused_label(app).as_deref(), Some("Width value"));
        app.key_press(Key::Tab);
        app.run_steps(2);
        assert_eq!(focused_label(app).as_deref(), Some("Height value"), "Tab moves to the next number");

        // A typed number lands in the parameters.
        ribbon::type_field(app, "Depth value", "35");
        settle(app);
        let depth = match &app.state().state.editor.as_ref().unwrap().params {
            EditorParams::Extrude(p) => p.depth,
            other => panic!("still extruding: {other:?}"),
        };
        assert_eq!(depth, 35.0, "Depth reads the typed value");
        pointer_to_chrome(app);
        case.frame(app, "extrude-typed");
    });
}

#[test]
fn cut_shell_draft_and_pattern_forms_expose_their_fields() {
    let mut case = Case::new("feature-panel/forms");
    let (doc, _) = plate_with_circle();
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartCut);
        assert_eq!(editor_title(app), "Cut-Extrude");
        case.frame(app, "cut");
        ribbon::open_options(app);
        assert!(app.query_by_label("Flip direction").is_some(), "the cut keeps its flip");
        assert!(shows(app, "Through all"), "a cut starts through all");
        open(app, AppAction::CancelEditor);

        open(app, AppAction::StartChamfer);
        case.frame(app, "chamfer");
        assert!(app.query_by_label("Distance value").is_some());
        open(app, AppAction::CancelEditor);

        open(app, AppAction::StartHole);
        case.frame(app, "hole");
        assert_fields(app, "Hole", &["Diameter value"]);
        open(app, AppAction::CancelEditor);

        open(app, AppAction::StartShell);
        case.frame(app, "shell");
        assert!(app.query_by_label("Wall thickness value").is_some());
        open(app, AppAction::CancelEditor);

        open(app, AppAction::StartDraft);
        case.frame(app, "draft");
        assert_fields(app, "Draft", &["Angle value"]);
        open(app, AppAction::CancelEditor);

        open(app, AppAction::StartPattern { circular: false });
        case.frame(app, "pattern");
        assert_fields(app, "Linear Pattern", &["Spacing value", "Copies value"]);
        open(app, AppAction::CancelEditor);
    });
}

#[test]
fn fillet_preview_tints_new_faces_and_names_the_radius() {
    let mut case = Case::new("feature-panel/fillet");
    let mut app = launch(ribbon::plate(), STANDARD);
    case.run(&mut app, |case, app| {
        let before = ux_tools::facts(app);
        open(app, AppAction::StartFillet);
        case.frame(app, "fillet-empty");
        assert!(app.query_by_label("Fillet preview").is_none(), "no preview without a pick");

        let (long, short) = near_edges(app);
        ribbon::click_world(app, long);
        ribbon::click_world(app, short);
        settle(app);
        assert_eq!(app.state().state.editor.as_ref().unwrap().edges().len(), 2, "both edges are held");
        ribbon::type_field(app, "Radius value", "5");
        settle(app);
        pointer_to_chrome(app);
        let image = case.frame(app, "fillet");

        // The preview names what it shows, over the viewport.
        let hud = app.get_by_label("Fillet preview");
        let said = hud.accesskit_node().value().unwrap_or_default();
        assert_eq!(said, "R 5 mm · 2 edges", "the viewport names the radius and the edges");
        assert!(ux_tools::viewport(app).contains_rect(hud.rect()), "the label sits over the viewport");

        // The rounded faces carry the accent; the untouched top face does not.
        let editor = app.state().state.editor.as_ref().unwrap();
        let mesh = editor.preview.result.clone().expect("R5 previews");
        assert!(mesh.face_features.contains(&soliddraft_app::document::PREVIEW_MADE), "new faces are marked");
        // On the rounded face over the long edge: halfway round the R5 arc.
        let r = 5.0 / 2f32.sqrt();
        let round = long + Vec3::new(0.0, -long.y.signum() * (5.0 - r), -(5.0 - r));
        let flat = Vec3::new(0.0, 0.0, TOP);
        let (made, kept) = (pixel(&image, app, round), pixel(&image, app, flat));
        let accent = theme::accent();
        case.note(format!("rounded face {made:?}, top face {kept:?}, accent {accent:?}"));
        assert!(
            color_distance(made, accent) + 20.0 < color_distance(kept, accent),
            "the rounded face reads nearer the accent than the top face: {made:?} vs {kept:?}"
        );

        // A refused radius shows no body, so the picked edges themselves are marked.
        ribbon::type_field(app, "Radius value", "abc");
        app.run_steps(4);
        pointer_to_chrome(app);
        assert!(app.state().state.editor.as_ref().unwrap().preview.result.is_none(), "no preview for abc");
        let image = case.frame(app, "fillet-refused");
        let mark = pixel(&image, app, long);
        case.note(format!("picked edge without a preview {mark:?}"));
        assert!(color_distance(mark, accent) < 60.0, "the picked edge is drawn in the accent: {mark:?}");
        let hud = app.get_by_label("Fillet preview").accesskit_node().value().unwrap_or_default();
        assert_eq!(hud, "No preview · 2 edges", "the label says the preview is off");

        // Escape leaves the part as it was.
        app.key_press(Key::Escape);
        settle(app);
        rebuilt(app);
        assert!(app.state().state.editor.is_none(), "Escape closes the panel");
        assert_eq!(ux_tools::facts(app), before, "nothing was committed");
        assert!(app.query_by_label("Fillet preview").is_none(), "the label goes with the panel");
    });
}

#[test]
fn a_long_pick_list_scrolls_and_keeps_the_radius_in_view() {
    let mut case = Case::new("feature-panel/fillet-faces");
    let mut app = launch(ribbon::plate(), STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartFillet);
        // Two faces add every edge on their boundaries: seven, the shared one once.
        let (long, _) = near_edges(app);
        ribbon::click_world(app, Vec3::new(0.0, 0.0, TOP));
        ribbon::click_world(app, Vec3::new(0.0, long.y, TOP / 2.0));
        settle(app);
        let held = app.state().state.editor.as_ref().unwrap().edges().len();
        assert_eq!(held, 7, "the two faces hold seven edges");
        app.get_by_label(&format!("{held} edges")).click();
        app.run_steps(8);
        let chips = app.query_all(By::new().predicate(|n| n.label().is_some_and(|l| l.starts_with("Remove ")))).count();
        assert_eq!(chips, held, "every pick keeps its chip, in the list's own scroll box");
        app.get_by_label(&format!("{held} edges")).click();
        app.run_steps(8);

        // The list scrolls in its box, so the radius stays above the status bar.
        let radius = app.get_by_label("Radius value").rect();
        let bar = ribbon::input_rect(app).top();
        assert!(radius.bottom() < bar, "the radius {radius:?} shows above the status bar at {bar}");
        ribbon::type_field(app, "Radius value", "2");
        settle(app);
        pointer_to_chrome(app);
        case.frame(app, "fillet-faces");
        let hud = app.get_by_label("Fillet preview").accesskit_node().value().unwrap_or_default();
        assert_eq!(hud, "R 2 mm · 7 edges", "the typed radius reaches the preview");
    });
}

#[test]
fn fillet_panel_in_dark_at_double_scale() {
    let mut case = Case::new("feature-panel/fillet-dark");
    let look: Look = SMALL_DARK;
    let mut app = launch(ribbon::plate(), look);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartFillet);
        let (long, _) = near_edges(app);
        ribbon::click_world(app, long);
        settle(app);
        pointer_to_chrome(app);
        case.frame(app, "fillet-dark");
        assert_fields(app, "Fillet", &["Radius value"]);
        let hud = app.get_by_label("Fillet preview").accesskit_node().value().unwrap_or_default();
        assert_eq!(hud, "R 4 mm · 1 edge");
    });
}

/// How many tree nodes have `text` as their label or value.
fn nodes_reading(app: &App, text: &str) -> usize {
    app.query_all(By::new().predicate(|n| n.label().as_deref() == Some(text) || n.value().as_deref() == Some(text)))
        .count()
}

/// The fillet form asks only for the edges and the radius, as in SolidWorks. Overflow,
/// corners and the rest are the kernel's to settle, so the form offers no choice of them
/// and nothing in it waits as Coming soon.
#[test]
fn the_fillet_form_asks_only_for_edges_and_radius() {
    let mut case = Case::new("feature-panel/fillet-options");
    let mut app = launch(ribbon::plate(), STANDARD);
    case.run(&mut app, |case, app| {
        let soon_before = nodes_reading(app, "Coming soon");
        open(app, AppAction::StartFillet);
        let (long, _) = near_edges(app);
        ribbon::click_world(app, long);
        settle(app);
        pointer_to_chrome(app);
        case.frame(app, "fillet-options");

        app.get_by_label("1 edges").click();
        app.run_steps(8);
        assert!(app.query_by_label_contains("Remove ").is_some(), "the picked edge can be removed");
        app.get_by_label("1 edges").click();
        app.run_steps(8);
        for gone in ["Setback parameters", "Fillet options"] {
            assert!(!shows(app, gone), "the form has no {gone} card");
        }
        for gone in ["Constant", "Variable", "Roll over", "Keep neighbours", "Round corners", "Setback distance value"]
        {
            assert!(app.query_by_label(gone).is_none(), "the form offers no {gone} control");
        }
        assert_eq!(nodes_reading(app, "Coming soon"), soon_before, "nothing in the form waits as Coming soon");
        assert_fields(app, "Fillet", &["Radius value"]);

        // The radius previews and commits.
        ribbon::type_field(app, "Radius value", "5");
        settle(app);
        let hud = app.get_by_label("Fillet preview").accesskit_node().value().unwrap_or_default();
        assert_eq!(hud, "R 5 mm · 1 edge");
        open(app, AppAction::CommitEditor);
        rebuilt(app);
        let doc = &app.state().state.doc;
        match &doc.features.last().expect("a feature").kind {
            soliddraft_app::document::FeatureKind::Fillet(f) => assert_eq!(f.radius, 5.0, "the fillet keeps R5"),
            other => panic!("the last feature is the fillet, not {other:?}"),
        }
        assert!(doc.display_failure().is_none(), "the fillet builds");
    });
}
