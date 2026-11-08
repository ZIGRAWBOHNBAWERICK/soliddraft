//! A drawing's model reference survives history after first Save and Save As.

use egui_kittest::Harness;
use serde_json::json;
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Command as PartCommand, Document, ExtrudeParams};
use soliddraft_app::drawing::{DrawingAction, DrawingSession, LinkState};
use soliddraft_app::state::AppAction;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use soliddraft_document::{FileLocations, LocationKind};
use soliddraft_drawing::{Annotation, AnnotationKind, AnnotationState, Command};
use soliddraft_kernel_api::Point2;
use std::sync::Mutex;

type App = Harness<'static, SolidDraftApp>;
static CONFIG: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy)]
enum Control {
    None,
    OpenPart,
    SearchFolder,
    SameFolder,
}

fn action(app: &mut App, action: AppAction) {
    app.state_mut().state.push(action);
    app.run_steps(2);
}

fn drawing(app: &mut App, command: DrawingAction) {
    action(app, AppAction::Drawing(command));
}

fn session(app: &App) -> &DrawingSession {
    app.state().state.drawing.session.as_ref().unwrap()
}

fn saved_history(name: &str, save_as: bool, redo: bool, control: Control) {
    let _config = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("soliddraft-save-history-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for folder in ["parts", "drawings", "config"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    soliddraft_app::file::set_config_dir(Some(root.join("config")));
    if matches!(control, Control::SearchFolder) {
        let mut locations = FileLocations::default();
        locations.add(LocationKind::ReferencedDocuments, root.join("parts"));
        soliddraft_app::settings::save_file_locations(&locations).unwrap();
    }
    let mut doc = Document::new("Bracket", soliddraft_app::kernel());
    doc.apply(PartCommand::AddExtrude { params: ExtrudeParams::default(), via_assistant: false }).unwrap();
    let mut app = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| SolidDraftApp::new(cc, doc, Automation::default()));
    let part_path = root.join("parts/bracket.dwprt");
    action(&mut app, AppAction::SaveAs(Some(part_path.clone())));
    drawing(&mut app, DrawingAction::MakeDrawing);
    drawing(&mut app, DrawingAction::CreateDrawing);
    assert_eq!(session(&app).doc.drawing.model.path, "bracket.dwprt");
    if save_as {
        drawing(&mut app, DrawingAction::Save(Some(root.join("parts/original.dwdrw"))));
    }

    let s = session(&app);
    let annotation = Annotation {
        id: s.doc.next_annotation_id(),
        view: s.doc.drawing.sheets[0].views[0].id,
        body: AnnotationKind::Note { text: "Keep this history".into(), leader: Vec::new() },
        anchors: Vec::new(),
        placement: Point2::new(0.0, 0.0),
        style: Default::default(),
        state: AnnotationState::Ok,
    };
    drawing(&mut app, DrawingAction::Command(Command::AddAnnotation { annotation }));
    if redo {
        drawing(&mut app, DrawingAction::Undo);
    }
    let path =
        root.join(if matches!(control, Control::SameFolder) { "parts/sheet.dwdrw" } else { "drawings/sheet.dwdrw" });
    if save_as {
        let result = call_checked(
            &mut AppTools { state: &mut app.state_mut().state },
            "drawing_save",
            json!({ "path": path }),
            &Caller::Mcp { client: "save-history-test".into() },
        );
        assert!(!result.is_error, "{}", result.content);
        app.run_steps(2);
    } else {
        drawing(&mut app, DrawingAction::Save(Some(path.clone())));
    }
    assert_eq!(session(&app).doc.drawing.model.stored_path(&path), part_path);
    let pinned = session(&app).doc.drawing.model.pinned.clone();
    let model_id = session(&app).doc.drawing.model.id;

    if !matches!(control, Control::OpenPart) {
        action(&mut app, AppAction::NewPart);
    }
    drawing(&mut app, DrawingAction::ShowDrawing);
    assert_eq!(session(&app).link_state(), LinkState::Current, "the saved path resolves before history");
    drawing(&mut app, if redo { DrawingAction::Redo } else { DrawingAction::Undo });
    let after_history = session(&app).link_state();
    assert_eq!(session(&app).doc.path.as_ref(), Some(&path));
    assert_eq!(session(&app).doc.drawing.model.id, model_id);
    assert_eq!(session(&app).doc.drawing.model.pinned, pinned);
    drawing(&mut app, DrawingAction::Save(None));
    drawing(&mut app, DrawingAction::CloseDrawing);
    drawing(&mut app, DrawingAction::OpenFile(path.clone()));
    assert_eq!(after_history, LinkState::Current, "history must keep the saved model reference resolvable");
    assert_eq!(session(&app).link_state(), LinkState::Current, "Save/reopen must retain the reference");
    assert_eq!(session(&app).doc.drawing.model.stored_path(&path), part_path);
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn first_save_undo_keeps_the_model_reference_through_save_and_reopen() {
    saved_history("first-undo", false, false, Control::None);
}

#[test]
fn first_save_redo_keeps_the_model_reference_through_save_and_reopen() {
    saved_history("first-redo", false, true, Control::None);
}

#[test]
fn drawing_save_as_undo_keeps_the_model_reference() {
    saved_history("save-as-undo", true, false, Control::None);
}

#[test]
fn drawing_save_as_redo_keeps_the_model_reference() {
    saved_history("save-as-redo", true, true, Control::None);
}

#[test]
fn open_part_search_folder_and_same_folder_are_resolution_controls() {
    for (name, control) in [
        ("open-part", Control::OpenPart),
        ("search-folder", Control::SearchFolder),
        ("same-folder", Control::SameFolder),
    ] {
        saved_history(name, false, false, control);
    }
}

#[test]
fn save_as_rebases_each_historical_reference_without_copying_its_identity_or_pin() {
    use soliddraft_app::drawing::{self, DrawingDoc};
    use soliddraft_app::state::AppState;
    use soliddraft_document::{ContentHash, DocKind, DocRef};

    let _config = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("soliddraft-save-history-identities-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for dir in ["old", "new", "config"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    soliddraft_app::file::set_config_dir(Some(root.join("config")));
    let mut part = Document::new("B", soliddraft_app::kernel());
    let b = root.join("old/b.dwprt");
    soliddraft_app::file::save(&mut part, &b).unwrap();
    let old = root.join("old/sheet.dwdrw");
    let new = root.join("new/sheet.dwdrw");
    let mut doc = DrawingDoc::new(soliddraft_drawing::fixture::bracket_a3());
    doc.path = Some(old);
    let mut reference_a = DocRef::new(
        soliddraft_document::DocumentId::new(),
        DocKind::Part,
        "a.dwprt",
        ContentHash::of_bytes(b"older revision of A"),
    );
    reference_a.locator = Some("historic-a".into());
    doc.drawing.model = reference_a.clone();
    doc.apply(Command::SetMeta { meta: doc.drawing.meta.clone() }, None).unwrap();
    let mut reference_b = DocRef::new(part.model().id(), DocKind::Part, "b.dwprt", part.model().semantic_hash());
    reference_b.locator = Some("historic-b".into());
    doc.drawing.model = reference_b.clone();
    doc.apply(Command::SetMeta { meta: doc.drawing.meta.clone() }, None).unwrap();
    doc.undo().unwrap();
    let mut state = AppState::new(part);
    state.drawing.session = Some(DrawingSession::new(doc));
    drawing::follow_part(&mut state);
    let result = call_checked(
        &mut AppTools { state: &mut state },
        "drawing_save",
        json!({ "path": new }),
        &Caller::Mcp { client: "save-history-identities".into() },
    );
    assert!(!result.is_error, "{}", result.content);
    reference_a.set_path(&new, &root.join("old/a.dwprt"));
    reference_b.set_path(&new, &b);
    let doc = &mut state.drawing.session.as_mut().unwrap().doc;
    doc.undo().unwrap();
    assert_eq!(doc.drawing.model, reference_a);
    doc.redo().unwrap();
    assert_eq!(doc.drawing.model, reference_b);
    doc.redo().unwrap();
    assert_eq!(doc.drawing.model, reference_b);
    std::fs::remove_dir_all(root).unwrap();
}
