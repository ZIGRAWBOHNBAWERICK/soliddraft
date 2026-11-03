//! The reviewed import's visible buttons run the shared command on real Catallax.

use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use soliddraft_app::{document::Document, import::Preview, state::AppState};

const BLOCK: &[u8] = include_bytes!("../../../ci/fixtures/import/block-seam-4um.step");
const WIDE: &[u8] = include_bytes!("../../../ci/fixtures/import/block-seam-20um.step");

#[test]
fn preview_cancel_decline_and_accept() {
    let doc = Document::new("Import test", soliddraft_app::kernel());
    let original_revision = doc.revision();
    let mut state = AppState::new(doc);
    state.import_dialog = Some(Preview::from_bytes(&state.doc, "block-seam-4um.step", BLOCK.to_vec()).unwrap());
    let mut app = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(|ui, state| soliddraft_app::import::window(ui.ctx(), state), state);
    app.run_steps(3);
    app.get_by_label("Gap 1: seam");
    app.get_by_label("Measured 0.004 mm");
    app.get_by_label("Repaired automatically, within 0.01 mm");
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").click();
    app.run_steps(3);
    assert!(app.state().import_dialog.is_none());
    assert_eq!(app.state().doc.revision(), original_revision);
    assert!(app.state().doc.features.is_empty());

    let state = app.state_mut();
    state.import_dialog = Some(Preview::from_bytes(&state.doc, "block-seam-4um.step", BLOCK.to_vec()).unwrap());
    app.run_steps(3);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Decline").click();
    app.run_steps(3);
    let feature = &app.state().doc.features[0];
    assert_eq!(feature.name, "Import1");
    let soliddraft_app::document::FeatureKind::ImportStep(import) = &feature.kind else { panic!("import feature") };
    assert_eq!(import.decisions["repair:1a602addaa4b:g0"], "decline");
    assert!(
        matches!(&feature.status, soliddraft_app::document::FeatureStatus::Error(message) if message.contains("not imported until the gap is resolved"))
    );
    assert!(app.state().doc.display_body().is_none());

    let state = app.state_mut();
    state.doc.undo().unwrap();
    let path = std::env::temp_dir().join(format!("soliddraft-import-ui-binding-{}.step", std::process::id()));
    std::fs::write(&path, BLOCK).unwrap();
    state.import_dialog = Some(Preview::read(&state.doc, &path).unwrap());
    let mut replacement = BLOCK.to_vec();
    replacement.push(b'\n');
    std::fs::write(&path, replacement).unwrap();
    app.run_steps(3);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
    app.run_steps(3);
    let feature = &app.state().doc.features[0];
    assert_eq!(feature.name, "Import1");
    let soliddraft_app::document::FeatureKind::ImportStep(import) = &feature.kind else { panic!("import feature") };
    assert!(import.decisions.is_empty());
    assert_eq!(import.source, BLOCK, "the UI commits the bytes it previewed, even after path replacement");
    assert_eq!(feature.status, soliddraft_app::document::FeatureStatus::Ok);
    assert!(app.state().doc.display_body().is_some());
    std::fs::remove_file(path).unwrap();
}

fn button_enabled(app: &Harness<'_, soliddraft_app::state::AppState>, label: &str) -> bool {
    !app.get_by_role_and_label(egui::accesskit::Role::Button, label).accesskit_node().is_disabled()
}

/// Replaces a field's text as a person does.
fn typed(app: &mut Harness<'_, soliddraft_app::state::AppState>, label: &str, text: &str) {
    app.get_by_role_and_label(egui::accesskit::Role::TextInput, label).click();
    app.run_steps(2);
    app.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    app.key_press(egui::Key::Delete);
    app.run_steps(2);
    if !text.is_empty() {
        app.get_by_role_and_label(egui::accesskit::Role::TextInput, label).type_text(text);
    }
    app.run_steps(3);
}

#[test]
fn an_asked_repair_needs_a_limit_and_an_approver_and_is_saved_with_both() {
    let doc = Document::new("Approval test", soliddraft_app::kernel());
    let mut state = AppState::new(doc);
    state.import_dialog = Some(Preview::from_bytes(&state.doc, "block-seam-20um.step", WIDE.to_vec()).unwrap());
    let mut app = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(|ui, state| soliddraft_app::import::window(ui.ctx(), state), state);
    app.run_steps(3);
    app.get_by_label("Gap 1: seam");
    app.get_by_label("Measured 0.02 mm");

    typed(&mut app, "Approved by", "");
    assert!(!button_enabled(&app, "Accept"), "an approval names who gave it");
    typed(&mut app, "Approved by", "Ana Reviewer");
    assert!(button_enabled(&app, "Accept"));
    typed(&mut app, "Allowed movement", "0.01");
    app.get_by_label("Enter at least 0.02 mm");
    assert!(!button_enabled(&app, "Accept"), "a limit below the gap cannot be accepted");
    typed(&mut app, "Allowed movement", "0.03");
    assert!(button_enabled(&app, "Accept"));
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
    app.run_steps(3);

    let state = app.state();
    assert!(state.import_dialog.is_none());
    let feature = &state.doc.features[0];
    assert_eq!(feature.status, soliddraft_app::document::FeatureStatus::Ok);
    let soliddraft_app::document::FeatureKind::ImportStep(import) = &feature.kind else { panic!("import feature") };
    let (question, answer) = import.decisions.iter().next().unwrap();
    assert_eq!(answer, "accept:0.03:Ana Reviewer");
    assert!(question.starts_with("repair:") && question.ends_with(":g0"));
    assert_eq!(import.source, WIDE);

    let details: std::collections::BTreeMap<_, _> = soliddraft_app::import::record_rows(
        state.doc.model().import_record(soliddraft_model::FeatureId(feature.id as u32)).unwrap(),
    )
    .into_iter()
    .collect();
    assert_eq!(details["Gap 1"], "Repaired seam, approved by Ana Reviewer");
    assert_eq!(details["Gap 1 approved limit"], "0.03 mm", "the limit the user approved");
    assert_eq!(details["Gap 1 vertex movement"], "0.02 mm", "the movement measured on the result");
    assert_eq!(details["Face #40 from source"], "up to 0.02 mm", "the attained tolerance, apart from the limit");
    for (label, text) in &details {
        for internal in ["Catallax", "kernel", "stage", "seam_close", "g0"] {
            assert!(!label.contains(internal) && !text.contains(internal), "{label}: {text}");
        }
    }
}

#[test]
fn a_repair_past_the_tenth_of_the_nearest_feature_is_refused_after_approval() {
    let source = std::str::from_utf8(BLOCK).unwrap().replace("10.004", "0.055001").replace(",10.)", ",0.05)");
    let mut doc = Document::new("Limit test", soliddraft_app::kernel());
    let preview = Preview::from_bytes(&doc, "thin.step", source.into_bytes()).unwrap();
    let revision = doc.revision();
    let review = soliddraft_app::import::Review {
        decline: Vec::new(),
        accept: vec![soliddraft_app::import::Approved {
            id: "g0".into(),
            tolerance_mm: "0.005001".into(),
            approver: "Ana Reviewer".into(),
        }],
    };
    let error = soliddraft_app::import::commit(&mut doc, &preview, &review).unwrap_err();
    assert!(error.contains("a tenth of the nearest feature"), "{error}");
    assert!(doc.features.is_empty());
    assert_eq!(doc.revision(), revision);
}

#[test]
fn mcp_refuses_source_substitution_after_preview() {
    use serde_json::{Value, json};
    use soliddraft_app::command_tools::{CommandTools, ProposalBook, Surface};
    use soliddraft_assistant::{Caller, ToolHost};

    fn call(doc: &mut Document, book: &mut ProposalBook, name: &str, input: Value) -> soliddraft_assistant::ToolOutput {
        CommandTools { doc, book, surface: Surface::Mcp, drawing: None }.call(
            name,
            input,
            &Caller::Mcp { client: "review binding test".into() },
        )
    }

    let temporary = std::env::temp_dir().join(format!("soliddraft-import-binding-{}", std::process::id()));
    std::fs::create_dir_all(&temporary).unwrap();
    let path = temporary.join("source.step");
    let mut replacement = BLOCK.to_vec();
    replacement.push(b'\n');
    let mut observed = Vec::new();
    for decline in [json!(["g0"]), json!([])] {
        std::fs::write(&path, BLOCK).unwrap();
        let kernel = std::sync::Arc::new(soliddraft_kernel_api::counting::Counting::new(soliddraft_app::kernel()));
        let mut doc = Document::new("Import binding", kernel);
        let mut book = ProposalBook::default();
        let before = (doc.revision(), doc.undo_depth());
        let preview = call(&mut doc, &mut book, "preview_import", json!({"path":path}));
        assert!(!preview.is_error, "{}", preview.content);
        let preview: Value = serde_json::from_str(&preview.content).unwrap();
        let token = preview.get("token").cloned().unwrap_or(json!("unbound legacy preview"));
        std::fs::write(&path, &replacement).unwrap();
        let answer =
            call(&mut doc, &mut book, "import_step", json!({"path":path,"preview_token":token,"decline":decline}));
        observed.push((answer.is_error, doc.features.len(), doc.revision(), doc.undo_depth()));
        if answer.is_error {
            assert!(answer.content.contains("review"), "{}", answer.content);
            assert_eq!((doc.revision(), doc.undo_depth()), before);
            let missing = call(&mut doc, &mut book, "import_step", json!({"path":path,"decline":[]}));
            assert!(missing.is_error);
            assert!(doc.features.is_empty());
            let fresh = call(&mut doc, &mut book, "preview_import", json!({"path":path}));
            assert!(!fresh.is_error, "{}", fresh.content);
            let fresh: Value = serde_json::from_str(&fresh.content).unwrap();
            assert_eq!(fresh["proposals"][0]["id"], "g0", "replacement is a supported part with the same gap id");
            let accepted = call(
                &mut doc,
                &mut book,
                "import_step",
                json!({"path":path,"preview_token":fresh["token"],"decline":[]}),
            );
            assert!(!accepted.is_error, "{}", accepted.content);
            let soliddraft_app::document::FeatureKind::ImportStep(import) = &doc.features[0].kind else {
                panic!("import")
            };
            assert_eq!(import.source, replacement);
            let summary: Value = serde_json::from_str(&accepted.content).unwrap();
            let top = summary["body"]["bounding_box"]["max"][2].as_f64().unwrap();
            assert!((top - 10.004).abs() <= 0.0005, "the repaired top must survive the summary: {top}");
        }
    }
    std::fs::remove_dir_all(temporary).unwrap();
    assert_eq!(
        observed,
        [(true, 0, 0, 0), (true, 0, 0, 0)],
        "neither decline nor acceptance can commit a substituted source"
    );
}

/// The real-use pass answered an approval with "abc" through import_step: the refusal
/// was "the import made no feature", which says nothing about the answer.
#[test]
fn an_allowed_movement_that_is_not_a_number_says_so() {
    let mut doc = Document::new("Wide seam", soliddraft_app::kernel());
    let preview = Preview::from_bytes(&doc, "wide.step", WIDE.to_vec()).unwrap();
    let revision = doc.revision();
    for tolerance in ["abc", "", "0.0.3", "-0.03"] {
        let review = soliddraft_app::import::Review {
            decline: Vec::new(),
            accept: vec![soliddraft_app::import::Approved {
                id: "g0".into(),
                tolerance_mm: tolerance.into(),
                approver: "Ana Reviewer".into(),
            }],
        };
        let error = soliddraft_app::import::commit(&mut doc, &preview, &review).unwrap_err();
        assert!(error.contains("must be a number of millimetres"), "{tolerance:?}: {error}");
        assert!(doc.features.is_empty());
        assert_eq!(doc.revision(), revision);
    }
}

/// A declined repair names the unbuilt solid. A file that gives the solid no name left
/// its STEP entity number in the tree, "#178 not imported until the gap is resolved".
#[test]
fn a_declined_import_names_no_entity_number() {
    let mut doc = Document::new("Declined", soliddraft_app::kernel());
    let preview = Preview::from_bytes(&doc, "block.step", BLOCK.to_vec()).unwrap();
    let review = soliddraft_app::import::Review { decline: vec!["g0".into()], accept: Vec::new() };
    soliddraft_app::import::commit(&mut doc, &preview, &review).unwrap();
    let soliddraft_app::document::FeatureStatus::Error(message) = &doc.features[0].status else {
        panic!("a declined import is an error: {:?}", doc.features[0].status)
    };
    assert!(!message.contains('#'), "{message}");
    assert!(message.starts_with("The solid is not imported until the gap is resolved"), "{message}");
}

/// The real-use pass could not measure, sketch on or round anything of an imported body:
/// its faces were listed as `Body · Face N`, a name no command took, and every edge
/// "cannot be referenced". Imported faces now carry names by their order in the source
/// file. Features that build on them are in `import_features.rs`.
#[test]
fn an_imported_body_has_faces_to_measure_and_sketch_on() {
    use serde_json::{Value, json};
    use soliddraft_app::command_tools::{CommandTools, ProposalBook, Surface};
    use soliddraft_assistant::{Caller, ToolHost};

    fn call(doc: &mut Document, book: &mut ProposalBook, name: &str, input: Value) -> soliddraft_assistant::ToolOutput {
        CommandTools { doc, book, surface: Surface::Mcp, drawing: None }.call(
            name,
            input,
            &Caller::Mcp { client: "imported faces".into() },
        )
    }

    let dir = std::env::temp_dir().join(format!("soliddraft-import-faces-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("block.step");
    std::fs::write(&source, BLOCK).unwrap();
    let mut doc = Document::new("Imported faces", soliddraft_app::kernel());
    let mut book = ProposalBook::default();
    let preview = call(&mut doc, &mut book, "preview_import", json!({"path":source}));
    let token = serde_json::from_str::<Value>(&preview.content).unwrap()["token"].clone();
    let out = call(&mut doc, &mut book, "import_step", json!({"path":source,"preview_token":token,"decline":[]}));
    assert!(!out.is_error, "{}", out.content);

    let feature: Value =
        serde_json::from_str(&call(&mut doc, &mut book, "feature", json!({"name":"Import1"})).content).unwrap();
    let faces: Vec<&str> = feature["faces"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(faces.len(), 6, "{feature}");
    assert!(faces.iter().all(|f| f.starts_with("Import1 · Face ")), "{faces:?}");
    for edge in feature["edges"].as_array().unwrap() {
        for face in edge["faces"].as_array().unwrap() {
            assert!(faces.contains(&face.as_str().unwrap()), "{edge}");
        }
    }
    let top = feature["faces"].as_array().unwrap().iter().find(|f| f["normal"] == json!([0.0, 0.0, 1.0])).unwrap();
    let top = top["name"].as_str().unwrap().to_owned();
    let area = call(&mut doc, &mut book, "measure_entity", json!({"entity":top}));
    assert!(!area.is_error, "{}", area.content);
    assert_eq!(serde_json::from_str::<Value>(&area.content).unwrap()["value"], 1000.0);
    let length = call(&mut doc, &mut book, "measure_entity", json!({"entity":"Import1 · Edge 1"}));
    assert!(!length.is_error, "{}", length.content);

    let circle = json!({"shape":"circle","center":[10,10,10.004],"diameter":4});
    let sketch = call(&mut doc, &mut book, "sketch", json!({"plane":top,"shapes":[circle]}));
    assert!(!sketch.is_error, "{}", sketch.content);

    let saved = dir.join("imported.dwprt");
    soliddraft_app::file::save(&mut doc, &saved).unwrap();
    let mut reopened = soliddraft_app::file::open(&saved, soliddraft_app::kernel(), None).unwrap();
    let sketch: Value =
        serde_json::from_str(&call(&mut reopened, &mut book, "feature", json!({"name":"Sketch1"})).content).unwrap();
    assert_eq!(sketch["status"], "ok", "{sketch}");
    assert_eq!(sketch["plane"], top, "{sketch}");
    std::fs::remove_dir_all(dir).unwrap();
}
