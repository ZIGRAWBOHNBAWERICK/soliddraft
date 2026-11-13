//! Native spline rendering retains its measured kind without granting export proof.

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_catallax::{APP_REQUEST_WORK, with_request_allowance};

fn edge_at(state: &AppState, wanted: soliddraft_kernel_api::Vec3) -> String {
    let body = state.doc.model().body().unwrap();
    let kernel = state.doc.model().kernel();
    let edge = state
        .doc
        .edges
        .iter()
        .find(|edge| {
            kernel.edge_midpoint(body, edge.id).is_ok_and(|point| {
                let [x, y, z] = point.coordinates.map(|coordinate| coordinate.value);
                soliddraft_kernel_api::Vec3::new(x, y, z).distance(wanted) < 1e-10
            })
        })
        .expect("the intended longitudinal loft edge");
    let name = format!("Loft1 · {}", edge.label);
    eprintln!("longitudinal selector at {wanted:?}: {name}");
    name
}

#[test]
fn native_loft_fillet_display_keeps_the_bound_and_plain_trimmed_face_refusal() {
    let dir =
        std::env::temp_dir().join(format!("soliddraft-spline-display-{}", soliddraft_document::DocumentId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut state = AppState::new(soliddraft_app::new_document());
    let scenario: Value = serde_json::from_str(include_str!("../../../ci/scenarios/loft-edge-fillet.json")).unwrap();
    let caller = Caller::Mcp { client: "spline-display-test".into() };
    for step in scenario["steps"].as_array().unwrap().iter().take(4) {
        let name = step["tool"].as_str().unwrap();
        let mut arguments = step["arguments"].clone();
        if name == "fillet" {
            arguments["edges"] = json!([edge_at(&state, soliddraft_kernel_api::Vec3::new(0.1, -0.05, 0.1))]);
        }
        let (out, work) = with_request_allowance(APP_REQUEST_WORK, || {
            call_checked(&mut AppTools { state: &mut state }, name, arguments, &caller)
        });
        eprintln!("{name}: {work:?}; {}", out.content);
        assert!(!out.is_error && !work.exhausted, "{name}: {}; {work:?}", out.content);
        if name == "loft" {
            for edge in &state.doc.edges {
                eprintln!("named loft edge: {} {:?}", edge.label, edge.midpoint);
            }
        }
    }
    let body = state.doc.model().body().unwrap();
    let bound = state.doc.model().kernel().display_bound(body).unwrap().expect("the native bounded spline display");
    bound.validate().unwrap();
    assert_eq!(bound.kind, "spline display mesh");
    assert_eq!(bound.level_mm, 0.01);
    assert!(bound.method.contains("derivative"));
    let proof = state.doc.proof_state();
    assert!(proof.answered_within_bound && proof.display_only_bound);
    assert!(proof.current_proved_model(), "the exact retained body has its independent proof");
    assert!(!proof.current_proved_display());
    let feature = state.doc.features.iter().find(|feature| feature.name == "Fillet1").unwrap();
    assert!(matches!(feature.status, soliddraft_app::document::FeatureStatus::AnsweredWithinBound(_)));
    let before = state.doc.model().to_bytes().unwrap();
    let undo = state.doc.undo_depth();
    let mut refusal = scenario["steps"][7]["arguments"].clone();
    refusal["edges"] = json!([edge_at(&state, soliddraft_kernel_api::Vec3::new(0.05, 0.15, 0.1))]);
    let out = call_checked(&mut AppTools { state: &mut state }, "fillet", refusal, &caller);
    assert!(out.is_error && out.content.contains("already trimmed"), "{}", out.content);
    assert_eq!(state.doc.model().to_bytes().unwrap(), before);
    assert_eq!(state.doc.undo_depth(), undo);
    assert!(state.doc.proof_state().answered_within_bound);
    // Export asks the certified writer independently, never reuses bounded display geometry.
    let path = dir.join("part.step");
    let out = call_checked(&mut AppTools { state: &mut state }, "export_step", json!({"path":path}), &caller);
    assert!(!out.is_error, "{}", out.content);
    assert!(std::fs::metadata(path).unwrap().len() > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_excessive_native_display_record_cannot_be_admitted() {
    let valid = soliddraft_kernel_api::DisplayBound {
        kind: "spline display mesh".into(),
        rule: "spline-display-evaluator-v1".into(),
        level_mm: 0.01,
        measured_mm: 0.008,
        method: "complete source-span derivative residual enclosures".into(),
        definition: "retained exact boundary revision".into(),
    };
    valid.validate().unwrap();
    for measured_mm in [0.010000001, f64::INFINITY, f64::NAN, -0.001] {
        let refused = soliddraft_kernel_api::DisplayBound { measured_mm, ..valid.clone() };
        assert!(refused.validate().is_err());
    }
}
