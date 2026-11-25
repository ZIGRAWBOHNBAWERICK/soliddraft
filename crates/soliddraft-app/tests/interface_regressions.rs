//! Interface regressions from the batch 360 review: a closed tab asks before it drops
//! unsaved work, a part keeps its own rollback and playback while another tab is active,
//! a drawing's Save and history act on the drawing, the Hole Wizard keeps its chosen end
//! condition until its target is picked, Show shows a body its base view hid, and
//! Physical Analysis measures the active body with its own material, names why a
//! finished preview is not measured, and converts yards as yards.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Rect, pos2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::document::{Command, Document, ExtrudeParams, ProfileKind, Selection, ViewItem};
use soliddraft_app::drawing::DrawingAction;
use soliddraft_app::state::{AppAction, EditorParams};
use soliddraft_materials::{Catalog, MaterialId};
use soliddraft_model::hole_wizard::HoleEndCondition;

fn act(app: &mut App, action: AppAction) {
    app.state_mut().state.push(action);
    app.run_steps(4);
}

/// Opens a blank part in a new tab with the + button, as a user does.
fn new_part_tab(app: &mut App) {
    app.get_by_label("New tab").click();
    app.run_steps(4);
    app.get_by_label("New part (.dwprt)").click();
    app.run_steps(6);
}

/// The button of the unsaved-changes question, told apart from the top bar's Save by
/// sitting on the question's row of buttons.
fn question_button(app: &App, label: &str) -> Rect {
    let row = app.get_by_label("Discard").rect().center().y;
    app.get_all_by_role_and_label(egui::accesskit::Role::Button, label)
        .map(|n| n.rect())
        .find(|r| (r.center().y - row).abs() < 2.0)
        .unwrap_or_else(|| panic!("no {label} beside Discard"))
}

fn answer(app: &mut App, label: &str) {
    let rect = question_button(app, label);
    ribbon::click_at(app, rect.center());
    app.run_steps(6);
}

fn asked(app: &App) -> bool {
    app.query_by_label("Discard").is_some()
}

fn plate_with_two_features() -> Document {
    let mut doc = ribbon::plate();
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams {
            profile: ProfileKind::Rectangle { width: 20.0, height: 20.0 },
            depth: 30.0,
            ..Default::default()
        },
        via_assistant: false,
    })
    .unwrap();
    doc
}

#[test]
fn closing_a_part_tab_with_unsaved_changes_asks_save_discard_or_cancel() {
    let case = ribbon::case("close-unsaved-part");
    let mut app = ribbon::app(ribbon::plate());
    assert!(app.state().state.doc.dirty, "the plate is unsaved");
    new_part_tab(&mut app);
    assert_eq!(app.state().open_parts.total(), 2);

    // The plate waits in its tab: closing it asks, and Cancel keeps it.
    app.get_by_label("Close Ribbon acceptance.dwprt").click();
    app.run_steps(6);
    assert!(asked(&app), "closing an unsaved part asks first");
    assert_eq!(app.state().open_parts.total(), 2, "nothing closes before the answer");
    answer(&mut app, "Cancel");
    assert!(!asked(&app));
    assert_eq!(app.state().open_parts.total(), 2, "Cancel keeps the tab");

    // Save writes the part where it lives, then closes its tab.
    let path = case.path("plate.dwprt");
    let parked = &mut app.state_mut().open_parts.before[0].doc;
    parked.path = Some(path.clone());
    app.run_steps(2);
    app.get_by_label("Close plate.dwprt").click();
    app.run_steps(6);
    assert!(asked(&app));
    answer(&mut app, "Save");
    app.run_steps(6);
    assert!(path.is_file(), "Save wrote the part");
    assert_eq!(app.state().open_parts.total(), 1, "the saved tab closed");
    assert!(!asked(&app));

    // Discard closes an unsaved active tab without saving.
    act(&mut app, AppAction::Command(Command::AddExtrude { params: ExtrudeParams::default(), via_assistant: false }));
    assert!(app.state().state.doc.dirty);
    new_part_tab(&mut app);
    let title = format!("Close {}.dwprt", app.state().open_parts.before[0].doc.name);
    app.get_by_label(&title).click();
    app.run_steps(6);
    assert!(asked(&app));
    answer(&mut app, "Discard");
    assert_eq!(app.state().open_parts.total(), 1, "Discard closed the tab");
    assert!(!asked(&app));
}

#[test]
fn closing_an_unsaved_drawing_or_assembly_tab_asks_first() {
    let case = ribbon::case("close-unsaved-documents");
    let mut app = ribbon::app(ribbon::plate());
    act(&mut app, AppAction::SaveAs(Some(case.path("plate.dwprt"))));
    act(&mut app, AppAction::Drawing(DrawingAction::MakeDrawing));
    act(&mut app, AppAction::Drawing(DrawingAction::CreateDrawing));
    add_note(&mut app, "Break all edges");
    let drawing = app.state().state.drawing.session.as_ref().expect("the drawing opened");
    assert!(drawing.doc.dirty, "the drawing has an unsaved note");
    let title = format!("Close {}.dwdrw", drawing.title());
    app.get_by_label(&title).click();
    app.run_steps(6);
    assert!(asked(&app), "closing an unsaved drawing asks first");
    answer(&mut app, "Cancel");
    assert!(app.state().state.drawing.session.is_some(), "Cancel keeps the drawing");
    app.get_by_label(&title).click();
    app.run_steps(6);
    answer(&mut app, "Discard");
    assert!(app.state().state.drawing.session.is_none(), "Discard closes the drawing");

    act(&mut app, AppAction::Assembly(soliddraft_app::assembly::AssemblyAction::FromPart));
    let assembly = app.state().state.assembly.session.as_ref().expect("the assembly opened");
    assert!(assembly.doc.is_dirty(), "a new assembly is unsaved");
    let title = format!("Close {}.dwasm", assembly.doc.assembly().name);
    app.get_by_label(&title).click();
    app.run_steps(6);
    assert!(asked(&app), "closing an unsaved assembly asks first");
    answer(&mut app, "Cancel");
    assert!(app.state().state.assembly.session.is_some(), "Cancel keeps the assembly");
    app.get_by_label(&title).click();
    app.run_steps(6);
    answer(&mut app, "Discard");
    assert!(app.state().state.assembly.session.is_none(), "Discard closes the assembly");
}

#[test]
fn a_part_keeps_its_own_rollback_and_playback_while_another_tab_is_active() {
    let _case = ribbon::case("tab-rollback");
    let mut app = ribbon::app(plate_with_two_features());
    let first = app.state().state.doc.features[0].id;
    act(&mut app, AppAction::Editor(soliddraft_app::app::EditorAction::EditFeature(first)));
    assert_eq!(app.state().state.doc.rollback, Some(0), "the editor shows the part before its feature");

    // Another tab, then back: the editor's view of the part ended with the switch.
    new_part_tab(&mut app);
    ribbon::document_tab(&mut app, "Ribbon acceptance");
    assert_eq!(app.state().state.doc.name, "Ribbon acceptance");
    assert_eq!(app.state().state.doc.rollback, None, "the editor's rollback stayed in the parked part");

    // The second part is rolled back to its start by the user. Playing the first part's
    // history and switching tabs must not step the second part.
    act(&mut app, AppAction::Command(Command::Rollback(Some(0))));
    ribbon::show_pane(&mut app, soliddraft_app::panels::Pane::History);
    act(&mut app, AppAction::Command(Command::Rollback(Some(0))));
    ribbon::document_tab(&mut app, "Part2");
    for _ in 0..2 {
        act(
            &mut app,
            AppAction::Command(Command::AddExtrude { params: ExtrudeParams::default(), via_assistant: false }),
        );
    }
    act(&mut app, AppAction::Command(Command::Rollback(Some(0))));
    ribbon::document_tab(&mut app, "Ribbon acceptance");
    app.get_by_label("Play history forward").click();
    app.run_steps(2);
    ribbon::document_tab(&mut app, "Part2");
    app.run_steps(120);
    assert_eq!(app.state().state.doc.name, "Part2");
    assert_eq!(app.state().state.doc.rollback, Some(0), "the first part's playback stepped the second part");
}

fn add_note(app: &mut App, text: &str) {
    use soliddraft_drawing::{Annotation, AnnotationKind, AnnotationState};
    let s = app.state().state.drawing.session.as_ref().unwrap();
    let annotation = Annotation {
        id: s.doc.next_annotation_id(),
        view: s.doc.drawing.sheets[0].views[0].id,
        body: AnnotationKind::Note { text: text.into(), leader: Vec::new() },
        anchors: Vec::new(),
        placement: soliddraft_kernel_api::Point2::new(0.0, 0.0),
        style: Default::default(),
        state: AnnotationState::Ok,
        source: None,
    };
    act(app, AppAction::Drawing(DrawingAction::Command(soliddraft_drawing::Command::AddAnnotation { annotation })));
}

fn top_bar(app: &App) -> Rect {
    let window = ribbon::window(app);
    Rect::from_min_max(window.min, pos2(window.right(), window.top() + soliddraft_app::theme::MENU_HEIGHT))
}

#[test]
fn a_drawing_saves_and_names_its_history_from_the_top_bar_and_file_menu() {
    let case = ribbon::case("drawing-save-routing");
    let mut app = ribbon::app(ribbon::plate());
    let part = case.path("plate.dwprt");
    act(&mut app, AppAction::SaveAs(Some(part.clone())));
    act(&mut app, AppAction::Drawing(DrawingAction::MakeDrawing));
    act(&mut app, AppAction::Drawing(DrawingAction::CreateDrawing));
    let sheet = case.path("plate.dwdrw");
    act(&mut app, AppAction::Drawing(DrawingAction::Save(Some(sheet.clone()))));
    add_note(&mut app, "Break all edges");
    let session = app.state().state.drawing.session.as_ref().unwrap();
    assert!(session.doc.dirty && app.state().state.drawing.is_active());
    let drawing_undo = session.doc.undo_label().map(str::to_owned).expect("the note is a step");
    let part_undo = app.state().state.doc.undo_label().map(str::to_owned);
    assert_ne!(Some(drawing_undo.clone()), part_undo);

    // Undo names the drawing's step: it is the step it undoes.
    let undo = ribbon::buttons_in(&app, top_bar(&app)).into_iter().find(|(l, _)| l == "Undo").expect("Undo").1;
    let tip = ribbon::tooltip(&mut app, undo);
    assert!(tip.starts_with(&format!("Undo {drawing_undo}")), "Undo names another document's step: {tip}");

    // The top bar's Save writes the drawing.
    let save =
        ribbon::buttons_in(&app, top_bar(&app)).into_iter().find(|(l, _)| l.starts_with("Save")).expect("Save").1;
    ribbon::click_at(&mut app, save.center());
    app.run_steps(6);
    assert!(
        !app.state().state.drawing.session.as_ref().unwrap().doc.dirty,
        "the top bar's Save left the drawing unsaved"
    );

    // So does the File menu's.
    add_note(&mut app, "Deburr");
    let file = ribbon::button_rect(&app, "File", top_bar(&app));
    ribbon::click_at(&mut app, file.center());
    app.run_steps(4);
    let bar = top_bar(&app);
    let entry = app
        .get_all_by_label_contains("Save drawing")
        .map(|n| n.rect())
        .find(|r| r.top() > bar.bottom())
        .expect("the File menu offers Save drawing");
    ribbon::click_at(&mut app, entry.center());
    app.run_steps(6);
    assert!(!app.state().state.drawing.session.as_ref().unwrap().doc.dirty, "File > Save left the drawing unsaved");
}

fn hole(app: &App) -> soliddraft_app::document::HoleParams {
    match &app.state().state.editor.as_ref().expect("hole editor open").params {
        EditorParams::Hole(h) => (**h).clone(),
        other => panic!("{other:?}"),
    }
}

fn choose_end(app: &mut App, label: &str) {
    app.get_by_label("Wizard End Condition").scroll_to_me();
    app.run_steps(24);
    let rect = app.get_by_label("Wizard End Condition").rect();
    ribbon::click_at(app, rect.center());
    app.run_steps(30);
    // The open list is drawn last, over any other control of that name.
    let rect = app.get_all_by_label(label).map(|n| n.rect()).next_back().expect("the end condition is listed");
    ribbon::click_at(app, rect.center());
    app.run_steps(30);
}

/// The plate's face whose outward normal is `normal`.
fn plate_face(app: &App, normal: soliddraft_kernel_api::Vec3) -> soliddraft_kernel_api::FaceId {
    let doc = &app.state().state.doc;
    let body = doc.model().body().unwrap();
    doc.kernel()
        .topology(body)
        .unwrap()
        .faces
        .into_iter()
        .find(|f| f.normal.is_some_and(|n| n.dot(normal) > 0.9999))
        .unwrap()
        .id
}

#[test]
fn the_hole_wizard_keeps_a_target_mode_until_its_target_is_picked() {
    let _case = ribbon::case("hole-target-mode");
    let mut app = ribbon::app(ribbon::plate());
    app.run_steps(30);
    ribbon::run_command(&mut app, "hole");
    app.run_steps(30);
    assert!(hole(&app).face.is_none(), "the hole starts with no face");

    choose_end(&mut app, "Offset From Surface");
    let h = hole(&app);
    assert!(h.to_model().is_err(), "a hole with no target is refused");

    // The first face picked is the face the holes start on; the mode stays.
    let top = plate_face(&app, soliddraft_kernel_api::Vec3::Z);
    act(&mut app, AppAction::Select(Some(Selection::Face(top))));
    let h = hole(&app);
    assert!(h.face.is_some(), "the first pick did not become the start face");
    assert!(
        !matches!(h.wizard.as_ref().map(|w| &w.end_condition), Some(HoleEndCondition::UpToSurface { .. })),
        "the pick turned Offset From Surface into Up To Surface"
    );
    assert!(h.to_model().is_err(), "a hole with no target is refused");

    // The next face is the target, and the mode is Offset From Surface.
    let bottom = plate_face(&app, -soliddraft_kernel_api::Vec3::Z);
    act(&mut app, AppAction::Select(Some(Selection::Face(bottom))));
    let h = hole(&app);
    let target = app.state().state.doc.face_ref(bottom).unwrap();
    match h.wizard.as_ref().map(|w| &w.end_condition) {
        Some(HoleEndCondition::OffsetFromSurface { face, .. }) => assert_eq!(*face, target),
        other => panic!("the end condition is {other:?}"),
    }
    assert_eq!(h.picking_target, None);
    assert!(h.to_model().is_ok(), "{:?}", h.to_model());

    // A mode without a target drops the pick a target mode was waiting for.
    choose_end(&mut app, "Up To Vertex");
    assert!(hole(&app).picking_target.is_some());
    choose_end(&mut app, "Blind");
    let h = hole(&app);
    assert_eq!(h.picking_target, None, "Blind kept the vertex pick");
    assert!(matches!(h.wizard.as_ref().map(|w| &w.end_condition), Some(HoleEndCondition::Blind { .. })));
    assert!(h.to_model().is_ok(), "{:?}", h.to_model());
}

#[test]
fn show_shows_a_body_its_base_view_hid_under_a_named_display_state() {
    use soliddraft_document::display_state::{DisplayMode, DisplayScope, DisplayState};
    let _case = ribbon::case("show-hidden-body");
    let mut doc = ribbon::plate();
    doc.set_view_choice(ViewItem::Body(0), Some(false));
    let inspection = DisplayState::new("Inspection".into(), DisplayScope::All, DisplayMode::Shaded);
    let id = inspection.id.clone();
    let mut states = doc.display_states().clone();
    states.put(inspection).unwrap();
    states.select(None, Some(id)).unwrap();
    doc.change_display_states(states).unwrap();
    // The body shows when neither its own view choice nor the display state hides it.
    let visible = |app: &App| {
        let state = &app.state().state;
        soliddraft_app::tree_visibility::shown(state, ViewItem::Body(0))
            && !state.doc.display_state().is_some_and(|s| s.view.hidden.contains(&0))
    };
    let mut app = ribbon::app(doc);
    assert!(!visible(&app));
    act(&mut app, AppAction::SetBodyVisible(true));
    assert!(visible(&app), "Show left the body hidden");
    // Hide under the display state stays the display state's own.
    act(&mut app, AppAction::SetBodyVisible(false));
    assert!(!visible(&app));
    assert_eq!(app.state().state.doc.view_choice(ViewItem::Body(0)), Some(true), "the base view still shows it");
}

fn painted_texts(app: &App) -> Vec<String> {
    ribbon::painted(app).into_iter().map(|p| p.text).collect()
}

fn open_physical_analysis(app: &mut App) {
    app.get_by_label("Physical analysis").click();
    app.run_steps(8);
}

#[test]
fn physical_analysis_weighs_the_active_body_with_its_own_material() {
    let _case = ribbon::case("physical-analysis-density");
    let catalog = Catalog::built_in();
    let steel = catalog.material(&MaterialId("steel-generic".into())).unwrap().1.clone();
    let brass = catalog.material(&MaterialId("brass-c26000".into())).unwrap().1.clone();
    let brass_density = brass.density_kg_m3.unwrap();
    let mut doc = ribbon::plate();
    doc.set_material(Some(steel), None).unwrap();
    doc.set_material_for(Some(0), Some(brass.clone()), None).unwrap();
    let mut app = ribbon::app(doc);
    ribbon::until(&mut app, "the proved plate", |app| app.state().state.doc.proof_state().current_proved_display());
    open_physical_analysis(&mut app);
    // 60 x 40 x 20 mm of brass.
    let grams = 48_000.0e-9 * brass_density * 1000.0;
    let expected = format!("{grams:.3} g");
    ribbon::until(&mut app, "the mass", |app| {
        painted_texts(app).iter().any(|t| t.ends_with(" g") && t.chars().next().is_some_and(|c| c.is_ascii_digit()))
    });
    let texts = painted_texts(&app);
    assert!(texts.contains(&expected), "the mass is not the brass body's {expected}: {texts:?}");
    assert!(texts.iter().any(|t| t.contains(&brass.name)), "the density is not named as the body's: {texts:?}");

    // A new part with no material does not show the brass density as one typed for it.
    app.key_press(egui::Key::Escape);
    app.run_steps(4);
    new_part_tab(&mut app);
    open_physical_analysis(&mut app);
    let shown = format!("{brass_density}");
    let texts = painted_texts(&app);
    assert!(!texts.contains(&shown), "the brass density followed into another part: {texts:?}");
}

#[test]
fn physical_analysis_names_why_a_finished_preview_is_not_measured() {
    let _case = ribbon::case("physical-analysis-preview");
    let mut app = ribbon::app(ribbon::plate());
    ribbon::until(&mut app, "the proved plate", |app| app.state().state.doc.proof_state().current_proved_display());
    let feature = app.state().state.doc.features.last().unwrap().id;
    let bound = serde_json::from_value(serde_json::json!({
        "rule": "Imported profile maximum deviation",
        "kept_definition": "Kept profile A",
        "measured_bound": 0.008,
        "allowed_bound": 0.01,
        "unit": "mm"
    }))
    .unwrap();
    app.state_mut().state.doc.record_answer_within_bound(feature, bound).unwrap();
    app.run_steps(10);
    let proof = app.state().state.doc.proof_state();
    assert!(proof.answered_within_bound && !proof.pending, "{proof:?}");
    open_physical_analysis(&mut app);
    app.run_steps(120);
    let texts = painted_texts(&app);
    assert!(
        !texts.iter().any(|t| t.contains("Waiting for the rebuild")),
        "the finished preview waits for a rebuild that never comes: {texts:?}"
    );
    assert!(texts.iter().any(|t| t == "Not measured"), "no refusal is named: {texts:?}");
}

#[test]
fn physical_analysis_converts_yards_as_yards() {
    let _case = ribbon::case("physical-analysis-yards");
    let mut app = ribbon::app(ribbon::plate());
    ribbon::until(&mut app, "the proved plate", |app| app.state().state.doc.proof_state().current_proved_display());
    app.get_by_label("Default units").click();
    app.run_steps(6);
    app.get_by_label("Yards").click();
    app.run_steps(6);
    open_physical_analysis(&mut app);
    ribbon::until(&mut app, "the measurement", |app| {
        painted_texts(app).iter().any(|t| t.starts_with('(') && t.contains(") "))
    });
    let texts = painted_texts(&app);
    let centre = texts.iter().find(|t| t.starts_with('(')).expect("the centre of mass");
    assert!(centre.ends_with(" yd"), "the centre of mass is not in yards: {centre}");
    assert!(texts.iter().any(|t| t.ends_with(" yd²")), "area is not in square yards: {texts:?}");
    assert!(texts.iter().any(|t| t.ends_with(" yd³")), "volume is not in cubic yards: {texts:?}");
}
