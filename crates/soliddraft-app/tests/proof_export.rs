//! STEP exports through the application tool used by the command line and assistant.

mod gated;

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{AnswerBound, Command, Document, ExtrudeParams};
use soliddraft_app::state::AppState;
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_exchange::{Format, proof::sidecar_path};
use std::{path::Path, sync::Arc};

static CONFIG: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn call(state: &mut AppState, tool: &str, args: Value) -> soliddraft_assistant::tools::ToolOutput {
    call_checked(&mut AppTools { state }, tool, args, &Caller::Mcp { client: "proof-export".into() })
}

fn run(state: &mut AppState, tool: &str, args: Value) {
    let out = call(state, tool, args);
    assert!(!out.is_error, "{tool}: {}", out.content);
}

fn report(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(sidecar_path(path, "json")).unwrap()).unwrap()
}

#[test]
fn step_exports_ship_feature_answers_and_retained_work_in_both_reports() {
    let _config = CONFIG.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("soliddraft-proof-export-{}", soliddraft_document::DocumentId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut state = AppState::new(soliddraft_app::new_document());
    run(
        &mut state,
        "sketch",
        json!({"plane":"top", "shapes":[{"shape":"rectangle", "center":[0,0,0], "width":6,"height":6}]}),
    );
    run(&mut state, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    let path = dir.join("plate.stp");
    run(&mut state, "export_step", json!({"path":path}));
    assert!(std::fs::read(&path).unwrap().starts_with(b"ISO-10303-21"));
    let original = report(&path);
    assert_eq!(original["format"], "soliddraft-proof/1");
    assert_eq!(original["file"], "plate.stp");
    assert_eq!(original["sha256"].as_str().unwrap().len(), 64);
    let features = original["features"].as_array().unwrap();
    assert_eq!(features.len(), 2);
    assert_eq!(features[0]["name"], "Sketch1");
    assert_eq!(features[1]["name"], "Boss-Extrude1");
    for feature in features {
        assert_eq!(feature["answer_kind"], "proved");
        assert!(feature["model_bound"].is_null());
        assert!(feature["display_bound"].is_null());
        assert!(feature["work_used"].as_u64().is_some(), "missing work: {feature}");
    }
    assert!(features[1]["work_used"].as_u64().unwrap() > 0);
    let text = std::fs::read_to_string(sidecar_path(&path, "txt")).unwrap();
    for expected in ["Sketch1", "Boss-Extrude1", "Proved", "Work used:", original["sha256"].as_str().unwrap()] {
        assert!(text.contains(expected), "missing {expected}");
    }
    let out = state.export_part(Format::Step).unwrap();
    let mut delivered = Vec::new();
    out.write_with(Path::new("download.step"), |path, bytes| {
        delivered.push((path.to_path_buf(), bytes.to_vec()));
        Ok(())
    })
    .unwrap();
    assert_eq!(delivered.len(), 3);
    assert_eq!(delivered[0].0, Path::new("download.step.proof.json"));
    assert_eq!(delivered[1].0, Path::new("download.step.proof.txt"));
    assert_eq!(delivered[2].0, Path::new("download.step"));
    run(&mut state, "edit_extrude", json!({"feature":"Boss-Extrude1","depth":12,"flip":false}));
    run(&mut state, "undo", json!({}));
    run(&mut state, "rename", json!({"feature":"Boss-Extrude1","name":"Plate"}));
    run(&mut state, "export_step", json!({"path":path}));
    let restored = report(&path);
    assert_eq!(restored["features"][1]["name"], "Plate");
    assert_eq!(restored["features"][1]["work_used"], original["features"][1]["work_used"]);
    // A report destination that cannot be written must prevent a new STEP publication.
    let blocked = dir.join("blocked.step");
    std::fs::write(&blocked, b"previous STEP").unwrap();
    std::fs::create_dir(sidecar_path(&blocked, "json")).unwrap();
    let out = call(&mut state, "export_step", json!({"path":blocked}));
    assert!(out.is_error, "{}", out.content);
    assert_eq!(std::fs::read(&blocked).unwrap(), b"previous STEP");
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn model_bounds_refuse_publication_and_display_bounds_keep_independent_model_proof() {
    let _config = CONFIG.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("soliddraft-bound-report-{}", soliddraft_document::DocumentId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let kernel = Arc::new(gated::Gated::default());
    let mut doc = Document::new("Display plate", kernel.clone());
    let id =
        doc.apply(Command::AddExtrude { params: ExtrudeParams::default(), via_assistant: false }).unwrap().unwrap();
    let mut state = AppState::new(doc);
    kernel.rules.lock().unwrap().display_bound = Some(soliddraft_kernel_api::DisplayBound {
        kind: "spline display mesh".into(),
        rule: "surface-cell-measurement".into(),
        level_mm: 0.01,
        measured_mm: 0.007,
        method: "complete boundary measurement".into(),
        definition: "kept boundary".into(),
    });
    run(&mut state, "rename", json!({"feature":"Boss-Extrude1","name":"Display plate"}));
    let path = dir.join("display.step");
    run(&mut state, "export_step", json!({"path":path}));
    let proof = report(&path);
    assert_eq!(proof["features"][0]["answer_kind"], "proved");
    assert!(proof["features"][0]["model_bound"].is_null());
    assert_eq!(proof["features"][0]["display_bound"]["measured_bound"], 0.007);
    let kept = [path.clone(), sidecar_path(&path, "json"), sidecar_path(&path, "txt")].map(|path| {
        let bytes = std::fs::read(&path).unwrap();
        (path, bytes)
    });
    let bound = AnswerBound::new("profile deviation".into(), "kept profile".into(), 0.008, 0.01, "mm".into()).unwrap();
    state.doc.record_answer_within_bound(id, bound).unwrap();
    let proof = serde_json::to_value(state.doc.proof_report()).unwrap();
    assert_eq!(proof["features"][0]["answer_kind"], "within_stated_bound");
    assert_eq!(proof["features"][0]["model_bound"]["measured_bound"], 0.008);
    assert_eq!(proof["features"][0]["display_bound"]["measured_bound"], 0.007);
    let refused = call(&mut state, "export_step", json!({"path":path}));
    assert!(refused.is_error && refused.content.contains("stated bound"), "{}", refused.content);
    for (path, bytes) in kept {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_first_feature_waiting_for_proof_explains_why_no_export_is_written() {
    let _config = CONFIG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = std::env::temp_dir().join(format!("soliddraft-first-proof-{}", soliddraft_document::DocumentId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let kernel = Arc::new(gated::Gated::default());
    struct Release(Arc<gated::Gated>);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0.rules.lock().unwrap().hold = None;
            self.0.changed.notify_all();
        }
    }
    let release = Release(kernel.clone());
    kernel.rules.lock().unwrap().hold = Some(10.0);
    let mut model = soliddraft_model::Document::new("First feature proof", kernel);
    model
        .execute_deferred(soliddraft_model::Command::AddFeature {
            kind: soliddraft_model::FeatureKind::Extrude(soliddraft_model::ExtrudeFeature {
                profile: soliddraft_model::ProfileSource::Shape {
                    datum: soliddraft_model::DatumPlane::Top,
                    shape: soliddraft_model::BuiltinShape::Rectangle { width: 6.0, height: 6.0 },
                },
                operation: soliddraft_model::ExtrudeOp::Boss,
                end: soliddraft_model::EndCondition::Blind { depth: 10.0 },
                reverse: false,
            }),
            name: None,
            position: None,
            via_assistant: false,
        })
        .unwrap();
    let mut state = AppState::new(Document::from_model(model));
    assert!(state.doc.proof_pending());
    let path = dir.join("pending.step");
    let out = call(&mut state, "export_step", json!({"path":path}));
    assert!(out.is_error && out.content.contains("pending"), "{}", out.content);
    for path in [&path, &sidecar_path(&path, "json"), &sidecar_path(&path, "txt")] {
        assert!(!path.exists());
    }
    drop(release);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while state.doc.proof_pending() {
        assert!(std::time::Instant::now() < deadline, "proof did not settle");
        state.doc.poll_proof();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    run(&mut state, "export_step", json!({"path":path}));
    assert_eq!(report(&path)["features"][0]["answer_kind"], "proved");
    soliddraft_app::file::set_config_dir(None);
    std::fs::remove_dir_all(dir).unwrap();
}
