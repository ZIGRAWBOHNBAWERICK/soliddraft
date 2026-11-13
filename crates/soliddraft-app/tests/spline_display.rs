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
fn native_scaled_fillet_display_refuses_with_a_numbered_reason_and_preserves_the_part() {
    check_fillet_refusal(
        include_str!("../../../ci/scenarios/loft-edge-fillet.json"),
        soliddraft_kernel_api::Vec3::new(0.1, -0.05, 0.1),
    );
}

#[test]
fn native_small_warp_fillet_display_refuses_with_a_numbered_reason_and_preserves_the_part() {
    check_fillet_refusal(
        include_str!("../../../ci/scenarios/loft-edge-fillet-small-warp.json"),
        soliddraft_kernel_api::Vec3::new(1.0, -0.9995, 1.0),
    );
}

fn check_fillet_refusal(source: &str, rail: soliddraft_kernel_api::Vec3) {
    static CONFIG: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _config = CONFIG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir =
        std::env::temp_dir().join(format!("soliddraft-spline-refusal-{}", soliddraft_document::DocumentId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let kernel = std::sync::Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_operation_log());
    let mut state = AppState::new(soliddraft_app::document::Document::new("Spline display", kernel.clone()));
    let scenario: Value = serde_json::from_str(source).unwrap();
    let caller = Caller::Mcp { client: "spline-display-test".into() };
    for step in scenario["steps"].as_array().unwrap().iter().take(3) {
        let name = step["tool"].as_str().unwrap();
        let (out, work) = with_request_allowance(APP_REQUEST_WORK, || {
            call_checked(&mut AppTools { state: &mut state }, name, step["arguments"].clone(), &caller)
        });
        assert!(!out.is_error && !work.exhausted, "{name}: {}; {work:?}", out.content);
    }
    let before = state.doc.model().to_bytes().unwrap();
    let undo = state.doc.undo_depth();
    let mut arguments = scenario["steps"][3]["arguments"].clone();
    let selected = edge_at(&state, rail);
    assert_eq!(arguments["edges"], json!([selected]), "the scenario names the exact intended rail");
    arguments["edges"] = json!([selected]);
    kernel.take_diagnostics().unwrap();
    let (out, work) = with_request_allowance(APP_REQUEST_WORK, || {
        call_checked(&mut AppTools { state: &mut state }, "fillet", arguments, &caller)
    });
    eprintln!("F04 display refusal: {work:?}; {}", out.content);
    assert!(out.is_error && work.exhausted && work.used <= APP_REQUEST_WORK, "{}; {work:?}", out.content);
    for expected in ["SD-DISPLAY-01", "0.01 mm", "work units", "20000000"] {
        assert!(out.content.contains(expected), "missing {expected}: {}", out.content);
    }
    for forbidden in ["kernel", "step op_", "mesh.spline", "numbers.operation"] {
        assert!(!out.content.contains(forbidden), "internal detail {forbidden}: {}", out.content);
    }
    let evidence = kernel.take_diagnostics().unwrap();
    let refusals: Vec<_> = evidence["calls"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|call| call["operation_log"]["operations"].as_array().unwrap())
        .filter_map(|operation| operation["refusal"]["code"].as_str())
        .collect();
    assert!(refusals.contains(&"mesh.spline-display-budget"), "{evidence}");
    assert_eq!(state.doc.model().to_bytes().unwrap(), before);
    assert_eq!(state.doc.undo_depth(), undo);
    assert!(!state.doc.features.iter().any(|feature| feature.name == "Fillet1"));
    assert!(!state.doc.proof_state().answered_within_bound);
    let path = dir.join("retained-loft.step");
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
