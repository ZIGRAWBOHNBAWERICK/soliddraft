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
        // With no sketch, the extrude asks for one; its sized rectangle needs none.
        let rectangle = app.get_by_label("Rectangle").rect();
        ribbon::click_at(app, rectangle.center());
        settle(app);
        pointer_to_chrome(app);
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
        assert_asks_for_edges(app, "chamfer");
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
        assert!(shows(app, "Click faces to draft"), "an empty draft asks for faces, not \"0 faces\"");
        // One picked face reads "1 face": the count used to read "1 faces".
        // A point of the top face, clear of the circle sketched on it.
        ribbon::click_world(app, Vec3::new(22.0, 12.0, TOP));
        settle(app);
        pointer_to_chrome(app);
        let Some(EditorParams::Draft(draft)) = app.state().state.editor.as_ref().map(|e| &e.params) else {
            panic!("the draft form is open")
        };
        assert_eq!(draft.faces.len(), 1, "the click picked one face");
        assert!(shows(app, "1 face"), "one face is counted in the singular");
        assert!(!shows(app, "1 faces"));
        case.frame(app, "draft-one-face");
        // The top face lies across the pull, so OK is refused. The form stays open with
        // the reason in its message area, right of the palette, and the tree gains no
        // failed row.
        let count = app.state().state.doc.features.len();
        open(app, AppAction::CommitEditor);
        rebuilt(app);
        assert!(
            matches!(app.state().state.editor.as_ref().map(|e| &e.params), Some(EditorParams::Draft(_))),
            "a refused draft keeps its form open"
        );
        assert_eq!(app.state().state.doc.features.len(), count, "a refused draft adds no row");
        let message = ribbon::painted(app)
            .into_iter()
            .find(|p| p.text.starts_with("Draft cannot"))
            .expect("the form says why Draft refused");
        let palette = egui::PanelState::load(&app.ctx, egui::Id::new("tool_palette")).unwrap().outer_rect;
        assert!(message.rect.left() >= palette.right(), "the message clears the palette: {message:?}");
        assert!(
            app.state().state.note.as_ref().is_none_or(|n| n.kind != soliddraft_app::state::NoteKind::Error),
            "a refusal is no error note"
        );
        case.frame(app, "draft-refused");
        open(app, AppAction::CancelEditor);

        // A side face drafts. A draft in the tree names its value Angle. It read
        // "Layout", the word for a pattern's copies.
        open(app, AppAction::StartDraft);
        ribbon::click_world(app, Vec3::new(30.0, 0.0, TOP / 2.0));
        settle(app);
        pointer_to_chrome(app);
        open(app, AppAction::CommitEditor);
        rebuilt(app);
        let doc = &app.state().state.doc;
        let draft = doc.features.iter().find(|f| f.kind.noun() == "draft").expect("the draft is in the tree").id;
        let details = doc.describe(Selection::Feature(draft)).expect("the draft describes itself").details;
        assert!(details.iter().any(|(label, value)| label == "Angle" && value == "3°"), "{details:?}");
        assert!(!details.iter().any(|(label, _)| label == "Layout"), "{details:?}");
        if app.state().state.editor.is_some() {
            open(app, AppAction::CancelEditor);
        }
        app.state_mut().state.push(AppAction::Select(None));
        settle(app);

        open(app, AppAction::StartPattern { circular: false });
        case.frame(app, "pattern");
        assert_fields(app, "Linear Pattern", &["Spacing value", "Copies value"]);
        open(app, AppAction::CancelEditor);
    });
}

/// An OK that only changes a dimension is refused as any other: a radius the part
/// cannot take keeps the form open, and the fillet keeps the radius it had.
#[test]
fn a_refused_radius_keeps_the_fillet_form_open() {
    let mut case = Case::new("feature-panel/fillet-radius-refused");
    let mut app = launch(ribbon::plate(), STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartFillet);
        let (long, short) = near_edges(app);
        ribbon::click_world(app, long);
        ribbon::click_world(app, short);
        settle(app);
        ribbon::type_field(app, "Radius value", "5");
        settle(app);
        pointer_to_chrome(app);
        open(app, AppAction::CommitEditor);
        rebuilt(app);
        let fillet = app.state().state.doc.features.last().expect("the fillet is in the tree").id;
        let before = ux_tools::facts(app);

        open(app, AppAction::EditFeature(fillet));
        ribbon::type_field(app, "Radius value", "35");
        settle(app);
        pointer_to_chrome(app);
        open(app, AppAction::CommitEditor);
        rebuilt(app);
        assert!(
            matches!(app.state().state.editor.as_ref().map(|e| &e.params), Some(EditorParams::Fillet(_))),
            "a refused radius keeps its form open"
        );
        case.frame(app, "fillet-radius-refused");
        // The open form shows the part rolled back to before the fillet. Cancel brings the
        // fillet back as it was.
        open(app, AppAction::CancelEditor);
        rebuilt(app);
        let status = app.state().state.doc.feature(fillet).unwrap().status.clone();
        assert_eq!(status, soliddraft_app::document::FeatureStatus::Ok, "the fillet keeps its radius");
        assert_eq!(ux_tools::facts(app), before, "the part is as it was");
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

/// A fillet or chamfer form that has no edges yet asks for them and refuses nothing: the
/// form used to open with a red "select at least one edge" before the first click.
fn assert_asks_for_edges(app: &App, what: &str) {
    let editor = app.state().state.editor.as_ref().expect("a feature form is open");
    assert_eq!(editor.error, None, "{what}: no refusal before the first pick");
    assert!(shows(app, "Click edges or faces"), "{what}: the field asks for edges");
    assert!(!shows(app, "select at least one edge"), "{what}: no error text is shown");
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
        case.frame(app, "fillet-nothing-picked");
        assert_asks_for_edges(app, "fillet");
        let (long, _) = near_edges(app);
        ribbon::click_world(app, long);
        settle(app);
        pointer_to_chrome(app);
        case.frame(app, "fillet-options");

        app.get_by_label("1 edge").click();
        app.run_steps(8);
        assert!(app.query_by_label_contains("Remove ").is_some(), "the picked edge can be removed");
        app.get_by_label("1 edge").click();
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

/// The centre of the one tick box named `label`, opening the form's More sheet when the
/// bar has no room for it.
fn check_box(app: &mut App, label: &str) -> egui::Pos2 {
    let role = egui::accesskit::Role::CheckBox;
    if app.query_by_role_and_label(role, label).is_none() {
        let more = app
            .query_all(By::new().predicate(|n| n.label().is_some_and(|l| l.starts_with("More ("))))
            .next()
            .expect("the form's More button")
            .rect()
            .center();
        ribbon::click_at(app, more);
        settle(app);
    }
    app.get_by_role_and_label(role, label).rect().center()
}

fn editor_params(app: &App) -> EditorParams {
    app.state().state.editor.as_ref().expect("a form is open").params.clone()
}

/// A drag from `from` to `to`, a few points a frame, as a hand moves.
fn drag(app: &mut App, from: egui::Pos2, to: egui::Pos2) {
    app.hover_at(from);
    app.run_steps(3);
    app.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    app.step();
    for i in 1..=10 {
        app.hover_at(from + (to - from) * (i as f32 / 10.0));
        app.step();
    }
    app.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    app.run_steps(4);
}

/// The Extrude preview is amber material with a depth arrow on the profile: dragging the
/// arrow sets the depth, and a double-click on its value opens Depth for typing. Draft is
/// a tick box of the form, as in SOLIDWORKS. Ticked, the preview is the tapered body, and
/// OK adds one row to the tree: the extrusion, built with a draft linked to it.
#[test]
fn extrude_previews_its_depth_arrow_and_takes_a_draft_option() {
    let mut case = Case::new("feature-panel/extrude-arrow-and-draft");
    let mut app = launch(ribbon::sketch(), STANDARD);
    case.run(&mut app, |case, app| {
        let sketch = app.state().state.doc.features[0].id;
        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(sketch))));
        settle(app);
        ribbon::run_command(app, "extrude");
        settle(app);
        pointer_to_chrome(app);
        let EditorParams::Extrude(before) = editor_params(app) else { panic!("the Extrude form is open") };
        let preview = app.state().state.editor.as_ref().unwrap().preview.clone();
        assert!(preview.tool.is_some(), "the material Extrude adds shows");
        let arrow = preview.guides.arrow.expect("a depth arrow");
        assert!(arrow.drags && arrow.text == soliddraft_app::document::fmt_mm(before.depth), "{arrow:?}");
        assert!((arrow.to - arrow.from).length() as f64 - before.depth < 1e-3);
        case.frame(app, "1-extrude-arrow");

        // Drag the arrow's handle out along the arrow: the depth grows.
        let handle = app.get_by_label("Depth arrow").rect().center();
        let state = &app.state().state;
        let tip = state.viewport.camera.project(arrow.to, state.viewport.rect).unwrap();
        let along = (tip - handle).normalized();
        drag(app, handle, handle + along * 40.0);
        settle(app);
        let EditorParams::Extrude(dragged) = editor_params(app) else { panic!() };
        assert!(dragged.depth > before.depth + 1.0, "the drag deepened {} to {}", before.depth, dragged.depth);
        case.frame(app, "2-extrude-arrow-dragged");

        // A double-click on the value opens Depth for typing.
        pointer_to_chrome(app);
        let value = app.get_by_label("Depth on the preview").rect().center();
        ribbon::double_click_at(app, value);
        app.run_steps(4);
        assert!(app.get_by_label("Depth value").is_focused(), "the form's Depth takes the keyboard");
        app.key_press(Key::Escape);
        settle(app);
        if app.state().state.editor.is_none() {
            ribbon::run_command(app, "extrude");
            settle(app);
        }

        // Tick Draft: the angle and Outward appear, and the preview tapers.
        pointer_to_chrome(app);
        let tick = check_box(app, "Draft");
        ribbon::click_at(app, tick);
        settle(app);
        let EditorParams::Extrude(drafted) = editor_params(app) else { panic!() };
        assert_eq!(drafted.draft, Some(soliddraft_app::document::ExtrudeDraft::default()));
        assert!(app.query_by_label("Draft angle value").is_some(), "the angle shows once Draft is ticked");
        // The sheet grows to hold them: a field it clipped would not take the click.
        let outward = check_box(app, "Outward");
        ribbon::click_at(app, outward);
        settle(app);
        let EditorParams::Extrude(drafted) = editor_params(app) else { panic!() };
        assert!(drafted.draft.is_some_and(|d| d.outward), "Outward takes the click: {drafted:?}");
        let preview = app.state().state.editor.as_ref().unwrap().preview.clone();
        assert!(preview.result.is_some(), "the preview is the tapered body");
        assert!(preview.guides.arrow.is_some(), "the tapered preview keeps its depth arrow");
        let made = preview.result.as_ref().unwrap().face_features.iter().filter(|f| **f == u32::MAX).count();
        assert!(made > 0, "the new material is marked for its amber");
        pointer_to_chrome(app);
        case.frame(app, "3-extrude-draft-preview");

        open(app, AppAction::CommitEditor);
        rebuilt(app);
        assert!(app.state().state.editor.is_none(), "{:?}", app.state().state.note.as_ref().map(|n| &n.text));
        let doc = &app.state().state.doc;
        let extrude = doc.features.iter().find(|f| f.kind.noun() == "extrude").expect("the extrusion").id;
        let draft = doc.linked_draft(extrude).expect("the extrusion carries its draft");
        let name = doc.feature(draft).unwrap().name.clone();
        ribbon::open_tree(app);
        assert!(app.query_all_by_label(&name).next().is_none(), "the draft has no row of its own");
        assert!(shows_with(app, "draft"), "the extrusion's row says it is drafted");
        case.frame(app, "4-extrude-draft-built");
    });
}

/// Whether some node's label or value contains `text`.
fn shows_with(app: &App, text: &str) -> bool {
    app.query_all(
        By::new()
            .predicate(|n| n.label().is_some_and(|l| l.contains(text)) || n.value().is_some_and(|v| v.contains(text))),
    )
    .next()
    .is_some()
}

/// Pattern, Mirror and Draft show their result before OK, as built: the copies or the
/// tilted faces marked, the copies numbered, the draft's neutral plane and pull drawn.
#[test]
fn pattern_mirror_and_draft_preview_before_ok() {
    let mut case = Case::new("feature-panel/built-previews");
    let (doc, _) = plate_with_circle();
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        // A boss to copy: the circle on the plate, extruded.
        open(app, AppAction::StartExtrude);
        open(app, AppAction::CommitEditor);
        rebuilt(app);
        let boss = app.state().state.doc.features.last().unwrap().id;
        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(boss))));
        settle(app);

        open(app, AppAction::StartPattern { circular: false });
        let preview = app.state().state.editor.as_ref().expect("the Pattern form").preview.clone();
        assert!(
            preview.result.is_some(),
            "the pattern shows its copies: {:?}",
            app.state().state.editor.as_ref().unwrap().error
        );
        assert!(!preview.guides.labels.is_empty(), "the copies are numbered");
        case.frame(app, "linear-pattern");
        open(app, AppAction::CancelEditor);

        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(boss))));
        settle(app);
        open(app, AppAction::StartMirror);
        let preview = app.state().state.editor.as_ref().expect("the Mirror form").preview.clone();
        assert!(
            preview.result.is_some(),
            "the mirror shows its copy: {:?}",
            app.state().state.editor.as_ref().unwrap().error
        );
        case.frame(app, "mirror");
        open(app, AppAction::CancelEditor);

        app.state_mut().state.push(AppAction::Select(None));
        open(app, AppAction::StartDraft);
        ribbon::click_world(app, Vec3::new(30.0, 0.0, TOP / 2.0));
        settle(app);
        pointer_to_chrome(app);
        let preview = app.state().state.editor.as_ref().expect("the Draft form").preview.clone();
        assert!(preview.result.is_some(), "the draft shows the tilted face");
        assert_eq!(preview.guides.outline.len(), 4, "the neutral plane is outlined");
        assert!(preview.guides.arrow.as_ref().is_some_and(|a| !a.drags), "the pull is an arrow");
        case.frame(app, "draft");
        open(app, AppAction::CancelEditor);

        // A hole before its face: a ghost follows the pointer over a planar face.
        open(app, AppAction::StartHole);
        let state = &app.state().state;
        let top = state.viewport.camera.project(Vec3::new(-20.0, -12.0, TOP), state.viewport.rect).unwrap();
        ribbon::render_steps::await_pick(app, top);
        app.run_steps(3);
        case.frame(app, "hole-ghost");
        open(app, AppAction::CancelEditor);
    });
}
