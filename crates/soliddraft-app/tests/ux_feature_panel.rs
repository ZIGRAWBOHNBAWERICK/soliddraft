//! The feature panel: the forms of Boss-Extrude, Cut-Extrude, Fillet, Chamfer, Hole,
//! Shell, Draft and Linear Pattern, and the viewport's preview of a fillet while its panel
//! is open. The real app on the real wgpu pipeline and the real kernel, driven by ribbon
//! actions, pointer picks and keys.
//!
//! The panel lays its numbers out as one table: every value field of a form starts at
//! the same x, ends at the same x and has the same height, so the values read as a
//! column. Tab walks the fields in order. The fillet preview tints the faces the fillet
//! makes, names its radius and edge count over the viewport, and marks the picked edges
//! when no rounded body can be shown.
//!
//! Frames go to `target/ux-tools/feature-panel/`; the PR's before and after pictures
//! come from them.

use egui::{Key, Rect};
use egui_kittest::kittest::{By, NodeT, Queryable};
use glam::Vec3;
use soliddraft_app::document::{Command, Document, PlaneKind, Selection, SketchPlane};
use soliddraft_app::state::{AppAction, EditorParams};
use soliddraft_app::theme;

#[path = "support/ribbon.rs"]
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
    let left = ux_tools::viewport(app).left();
    let mut fields: Vec<(String, Rect)> = app
        .query_all(By::new().predicate(|n| n.label().is_some_and(|l| l.ends_with(" value"))))
        .map(|n| (n.accesskit_node().label().unwrap_or_default(), n.rect()))
        .filter(|(_, r)| r.right() <= left + ribbon::TOLERANCE)
        .collect();
    fields.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
    fields
}

/// The form's value fields line up as a column of one width and one height.
fn assert_table(app: &App, what: &str, wanted: &[&str]) {
    let fields = value_fields(app);
    let names: Vec<&str> = fields.iter().map(|(n, _)| n.as_str()).collect();
    for w in wanted {
        assert!(names.contains(w), "{what}: the form has {w:?}: {names:?}");
    }
    let table: Vec<&(String, Rect)> = fields.iter().filter(|(n, _)| wanted.contains(&n.as_str())).collect();
    let first = table[0].1;
    for (name, rect) in &table {
        assert!(
            (rect.left() - first.left()).abs() <= ribbon::TOLERANCE
                && (rect.right() - first.right()).abs() <= ribbon::TOLERANCE,
            "{what}: {name} spans the value column {:?}, not {:?}",
            first.x_range(),
            rect.x_range()
        );
        assert!(
            (rect.height() - first.height()).abs() <= ribbon::TOLERANCE,
            "{what}: {name} is {} tall like the others, not {}",
            first.height(),
            rect.height()
        );
        assert!(rect.width() >= 80.0, "{what}: {name} is wide enough to read a value: {rect:?}");
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
fn extrude_numbers_line_up_and_tab_walks_them() {
    let mut case = Case::new("feature-panel/extrude");
    let mut app = launch(ribbon::blank(), STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartExtrude);
        assert_eq!(editor_title(app), "Boss-Extrude");
        case.frame(app, "extrude");
        assert_table(app, "Boss-Extrude", &["Width value", "Height value", "Depth value"]);
        assert!(app.query_by_label("Flip direction").is_some(), "the flip stays a labelled checkbox");
        assert!(app.query_by_label("OK (Enter)").is_some() && app.query_by_label("Cancel (Esc)").is_some());

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
fn cut_shell_draft_and_pattern_forms_share_the_table() {
    let mut case = Case::new("feature-panel/forms");
    let (doc, _) = plate_with_circle();
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        open(app, AppAction::StartCut);
        assert_eq!(editor_title(app), "Cut-Extrude");
        case.frame(app, "cut");
        assert!(app.query_by_label("Flip direction").is_some(), "the cut keeps its flip");
        assert!(shows(app, "Through all"), "a cut starts through all");
        open(app, AppAction::CancelEditor);

        open(app, AppAction::StartChamfer);
        case.frame(app, "chamfer");
        assert!(app.query_by_label("Distance value").is_some());
        open(app, AppAction::CancelEditor);

        open(app, AppAction::StartHole);
        case.frame(app, "hole");
        assert_table(app, "Hole", &["Diameter value"]);
        open(app, AppAction::CancelEditor);

        open(app, AppAction::StartShell);
        case.frame(app, "shell");
        assert!(app.query_by_label("Wall thickness value").is_some());
        open(app, AppAction::CancelEditor);

        open(app, AppAction::StartDraft);
        case.frame(app, "draft");
        assert_table(app, "Draft", &["Angle value"]);
        open(app, AppAction::CancelEditor);

        open(app, AppAction::StartPattern { circular: false });
        case.frame(app, "pattern");
        assert_table(app, "Linear Pattern", &["Spacing value", "Copies value"]);
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
        let chips = app.query_all(By::new().predicate(|n| n.label().is_some_and(|l| l.starts_with("Remove ")))).count();
        assert_eq!(chips, held, "every pick keeps its chip, in the list's own scroll box");

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
        assert_table(app, "Fillet", &["Radius value"]);
        let hud = app.get_by_label("Fillet preview").accesskit_node().value().unwrap_or_default();
        assert_eq!(hud, "R 4 mm · 1 edge");
    });
}

/// Whether each tree node whose label or value is `text` is disabled.
fn nodes_reading(app: &App, text: &str) -> Vec<bool> {
    app.query_all(By::new().predicate(|n| n.label().as_deref() == Some(text) || n.value().as_deref() == Some(text)))
        .map(|n| n.accesskit_node().is_disabled())
        .collect()
}

#[test]
fn fillet_options_catallax_cannot_build_yet_show_disabled() {
    let mut case = Case::new("feature-panel/fillet-options");
    let mut app = launch(ribbon::plate(), STANDARD);
    case.run(&mut app, |case, app| {
        let soon_before = nodes_reading(app, "Coming soon").len();
        open(app, AppAction::StartFillet);
        let (long, _) = near_edges(app);
        ribbon::click_world(app, long);
        settle(app);
        pointer_to_chrome(app);
        case.frame(app, "fillet-options");

        // Constant radius is the choice; Variable shows beside it but takes no click.
        let chosen = |app: &App, label: &str| {
            app.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
        };
        assert!(chosen(app, "Constant") && !chosen(app, "Variable"), "Constant radius is chosen");
        assert!(!app.get_by_label("Constant").accesskit_node().is_disabled(), "Constant stays a live choice");
        let variable = app.get_by_label("Variable").rect();
        assert!(app.get_by_label("Variable").accesskit_node().is_disabled(), "Variable waits for Catallax");
        ribbon::click_at(app, variable.center());
        settle(app);
        assert!(chosen(app, "Constant") && !chosen(app, "Variable"), "the click changed nothing");
        // Resting on it brings up the tip that says what it waits for.
        pointer_to_chrome(app);
        app.hover_at(variable.center());
        app.run_steps(60);
        let tip = app
            .query_all(By::new().predicate(|n| {
                [n.label(), n.value()]
                    .into_iter()
                    .flatten()
                    .any(|t| t.starts_with("Coming soon: a start and end radius"))
            }))
            .count();
        case.frame(app, "fillet-variable-tip");
        assert_eq!(tip, 1, "the tip says what Variable waits for");
        pointer_to_chrome(app);

        // The overflow choice is Catallax's: Roll over builds and is chosen; Keep neighbours and its
        // setback stations wait, so their controls are disabled and the setback card says so.
        for title in ["Fillet parameters", "Setback parameters", "Fillet options"] {
            assert!(shows(app, title), "the form has the {title} card");
        }
        assert_eq!(nodes_reading(app, "Coming soon").len(), soon_before + 1, "the setback card carries the badge");
        assert!(chosen(app, "Roll over"), "Roll over is the fillet's overflow");
        assert!(!app.get_by_label("Roll over").accesskit_node().is_disabled(), "Roll over is a live choice");
        for control in ["Setback distance value", "Keep neighbours", "Round corners"] {
            let disabled = app.get_by_label(control).accesskit_node().is_disabled();
            assert!(disabled, "{control} is drawn disabled");
        }
        assert_table(app, "Fillet", &["Radius value", "Setback distance value"]);
        app.get_by_label("Round corners").scroll_to_me();
        app.run_steps(24);
        pointer_to_chrome(app);
        case.frame(app, "fillet-options-lower");
        app.get_by_label("Radius value").scroll_to_me();
        app.run_steps(24);

        // The live parts of the form still work: the radius previews and commits.
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

        // A chamfer has none of these.
        open(app, AppAction::StartChamfer);
        assert!(!shows(app, "Setback parameters") && !shows(app, "Fillet options"), "the chamfer form is unchanged");
        assert!(app.query_by_label("Variable").is_none(), "a chamfer has no variable radius choice");
        open(app, AppAction::CancelEditor);
    });
}

/// The plate's back top edge, along x at y = 20, and the end face it runs into at x = 30.
fn back_edge_and_end(doc: &Document) -> (soliddraft_app::document::EdgeRef, soliddraft_app::document::FaceRef) {
    let t = doc.kernel().topology(doc.model().body().expect("the plate")).unwrap();
    let edge = t
        .edges
        .iter()
        .find(|e| (e.midpoint.y - 20.0).abs() < 1e-6 && (e.midpoint.z - TOP as f64).abs() < 1e-6)
        .expect("the back top edge");
    let end = t.faces.iter().find(|f| f.normal.is_some_and(|n| (n.x - 1.0).abs() < 1e-9)).expect("the face at x = 30");
    (doc.edge_ref(edge.id).expect("edge name"), doc.any_face_ref(end.id).expect("face name"))
}

#[test]
fn a_fillet_that_keeps_its_neighbours_opens_with_its_stations_and_can_roll_over() {
    use soliddraft_model::{FilletOverflow, FilletSetback};
    // The app adds no feature that fails, so a fillet that keeps its neighbours arrives in a
    // saved part, such as one written where Catallax builds it. The model stores it as is.
    let plate = ribbon::plate();
    let (edge, end) = back_edge_and_end(&plate);
    let keep = FilletOverflow::KeepNeighbours(vec![FilletSetback { edge: 0, at: end.clone(), setback: 10.0 }]);
    let mut model =
        soliddraft_model::Document::from_bytes(&plate.model().to_bytes().unwrap(), plate.kernel().clone()).unwrap();
    let id = model
        .execute(soliddraft_model::Command::AddFeature {
            kind: soliddraft_model::FeatureKind::Fillet(soliddraft_model::BlendFeature {
                edges: vec![edge],
                size: 4.0,
                overflow: Some(keep),
                laws: Vec::new(),
            }),
            name: None,
            position: None,
            via_assistant: false,
        })
        .unwrap()
        .created
        .unwrap()
        .0 as u64;
    let mut case = Case::new("feature-panel/fillet-keep-neighbours");
    let mut app = launch(Document::from_model(model), STANDARD);
    case.run(&mut app, |case, app| {
        rebuilt(app);
        match &app.state().state.doc.feature(id).expect("the fillet").status {
            soliddraft_app::document::FeatureStatus::Error(why) => {
                assert!(why.to_lowercase().contains("coming soon"), "the fillet is refused as coming soon: {why}")
            }
            other => panic!("the fillet is refused, not {other:?}"),
        }

        // Opened for editing, the form shows the policy it holds and lists the station.
        open(app, AppAction::EditFeature(id));
        let chosen = |app: &App, label: &str| {
            app.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
        };
        assert!(chosen(app, "Keep neighbours") && !chosen(app, "Roll over"), "the form shows the stored policy");
        let station = format!("10 mm from {}", end.label);
        let listed = app
            .query_all(By::new().predicate(|n| [n.label(), n.value()].into_iter().flatten().any(|t| t == station)))
            .count();
        assert!(listed >= 1, "the station is listed as {station:?}");
        let hud = app.get_by_label("Fillet preview").accesskit_node().value().unwrap_or_default();
        assert_eq!(hud, "No preview · 1 edge", "Catallax cannot preview it yet");
        app.get_by_label("Round corners").scroll_to_me();
        app.run_steps(24);
        pointer_to_chrome(app);
        case.frame(app, "keep-neighbours");

        // Roll over builds: choosing it previews and commits the rounded body.
        let roll = app.get_by_label("Roll over").rect();
        ribbon::click_at(app, roll.center());
        settle(app);
        pointer_to_chrome(app);
        assert!(chosen(app, "Roll over"), "Roll over is chosen");
        let hud = app.get_by_label("Fillet preview").accesskit_node().value().unwrap_or_default();
        assert_eq!(hud, "R 4 mm · 1 edge", "the rolled-over fillet previews");
        case.frame(app, "rolled-over");
        open(app, AppAction::CommitEditor);
        rebuilt(app);
        let doc = &app.state().state.doc;
        let status = &doc.feature(id).expect("the fillet").status;
        assert!(
            matches!(
                status,
                soliddraft_app::document::FeatureStatus::Ok | soliddraft_app::document::FeatureStatus::Pending
            ),
            "the fillet builds: {status:?}"
        );
        match &doc.feature(id).expect("the fillet").kind {
            soliddraft_app::document::FeatureKind::Fillet(f) => {
                assert_eq!(f.overflow, Some(FilletOverflow::RollOver), "the choice is stored")
            }
            other => panic!("still a fillet: {other:?}"),
        }
    });
}

#[test]
fn a_variable_fillet_opens_with_its_laws_and_can_go_constant() {
    use soliddraft_kernel_api::RadiusLaw;
    use soliddraft_model::FilletLaw;
    // As with keep-neighbours, a variable fillet arrives in a saved part; the model stores it.
    let plate = ribbon::plate();
    let (edge, end) = back_edge_and_end(&plate);
    let laws = vec![FilletLaw { edge: 0, from: end.clone(), radius: RadiusLaw::Linear { start: 2.0, end: 6.0 } }];
    let mut model =
        soliddraft_model::Document::from_bytes(&plate.model().to_bytes().unwrap(), plate.kernel().clone()).unwrap();
    let id = model
        .execute(soliddraft_model::Command::AddFeature {
            kind: soliddraft_model::FeatureKind::Fillet(soliddraft_model::BlendFeature {
                edges: vec![edge],
                size: 4.0,
                overflow: None,
                laws,
            }),
            name: None,
            position: None,
            via_assistant: false,
        })
        .unwrap()
        .created
        .unwrap()
        .0 as u64;
    let mut case = Case::new("feature-panel/fillet-variable");
    let mut app = launch(Document::from_model(model), STANDARD);
    case.run(&mut app, |case, app| {
        rebuilt(app);
        match &app.state().state.doc.feature(id).expect("the fillet").status {
            soliddraft_app::document::FeatureStatus::Error(why) => {
                assert!(why.to_lowercase().contains("coming soon"), "the fillet is refused as coming soon: {why}")
            }
            other => panic!("the fillet is refused, not {other:?}"),
        }

        // Opened for editing, the form shows Variable and lists the law with its start end.
        open(app, AppAction::EditFeature(id));
        let chosen = |app: &App, label: &str| {
            app.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
        };
        assert!(chosen(app, "Variable") && !chosen(app, "Constant"), "the form shows the stored radius kind");
        let listed_law = format!("R2 to R6 mm from {}", end.label);
        let listed = app
            .query_all(By::new().predicate(|n| [n.label(), n.value()].into_iter().flatten().any(|t| t == listed_law)))
            .count();
        assert!(listed >= 1, "the law is listed as {listed_law:?}");
        let hud = app.get_by_label("Fillet preview").accesskit_node().value().unwrap_or_default();
        assert_eq!(hud, "No preview · 1 edge", "Catallax cannot preview it yet");
        pointer_to_chrome(app);
        case.frame(app, "variable");

        // Constant builds: choosing it drops the law, previews R4 and commits.
        let constant = app.get_by_label("Constant").rect();
        ribbon::click_at(app, constant.center());
        settle(app);
        pointer_to_chrome(app);
        assert!(chosen(app, "Constant"), "Constant is chosen");
        let hud = app.get_by_label("Fillet preview").accesskit_node().value().unwrap_or_default();
        assert_eq!(hud, "R 4 mm · 1 edge", "the constant fillet previews");
        case.frame(app, "constant");
        open(app, AppAction::CommitEditor);
        rebuilt(app);
        let doc = &app.state().state.doc;
        let status = &doc.feature(id).expect("the fillet").status;
        assert!(
            matches!(
                status,
                soliddraft_app::document::FeatureStatus::Ok | soliddraft_app::document::FeatureStatus::Pending
            ),
            "the fillet builds: {status:?}"
        );
        match &doc.feature(id).expect("the fillet").kind {
            soliddraft_app::document::FeatureKind::Fillet(f) => assert!(f.laws.is_empty(), "the laws are gone"),
            other => panic!("still a fillet: {other:?}"),
        }
    });
}
