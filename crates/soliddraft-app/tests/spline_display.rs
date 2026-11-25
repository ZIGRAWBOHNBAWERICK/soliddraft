//! Native spline displays certify within the requested tolerance and preserve the part on refusal.

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_catallax::{APP_WORK_TARGET, with_request_allowance};

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
fn native_scaled_fillet_display_certifies_at_the_application_work_target() {
    check_fillet_display(
        include_str!("../../../ci/scenarios/loft-edge-fillet.json"),
        soliddraft_kernel_api::Vec3::new(0.1, -0.05, 0.1),
        None,
    );
}

#[test]
fn native_scaled_fillet_display_preserves_the_part_on_reduced_budget_refusal() {
    check_fillet_display(
        include_str!("../../../ci/scenarios/loft-edge-fillet.json"),
        soliddraft_kernel_api::Vec3::new(0.1, -0.05, 0.1),
        Some(9_600_000),
    );
}

#[test]
fn native_small_warp_fillet_display_certifies_within_the_stated_bound() {
    check_fillet_display(
        include_str!("../../../ci/scenarios/loft-edge-fillet-small-warp.json"),
        soliddraft_kernel_api::Vec3::new(1.0, -0.9995, 1.0),
        None,
    );
}

fn check_fillet_display(source: &str, rail: soliddraft_kernel_api::Vec3, refusal_budget: Option<u64>) {
    static CONFIG: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _config = CONFIG.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir =
        std::env::temp_dir().join(format!("soliddraft-spline-display-{}", soliddraft_document::DocumentId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut state = AppState::new(soliddraft_app::startup::new_document());
    let scenario: Value = serde_json::from_str(source).unwrap();
    let caller = Caller::Mcp { client: "spline-display-test".into() };
    for step in scenario["steps"].as_array().unwrap().iter().take(3) {
        let name = step["tool"].as_str().unwrap();
        let (out, work) = with_request_allowance(APP_WORK_TARGET, || {
            call_checked(&mut AppTools { state: &mut state }, name, step["arguments"].clone(), &caller)
        });
        assert!(!out.is_error && !work.exhausted, "{name}: {}; {work:?}", out.content);
    }
    let before = state.doc.model().to_bytes().unwrap();
    let geometry = state.doc.model().geometry_hash();
    let retained_body = state.doc.display_body();
    let names = |state: &AppState| {
        json!({
            "faces": state.doc.faces.iter().map(|face| state.doc.any_face_ref(face.id).unwrap()).collect::<Vec<_>>(),
            "edges": state.doc.edges.iter().map(|edge| state.doc.edge_ref(edge.id).unwrap()).collect::<Vec<_>>(),
        })
    };
    let retained_names = names(&state);
    let undo = state.doc.undo_depth();
    let mut arguments = scenario["steps"][3]["arguments"].clone();
    let selected = edge_at(&state, rail);
    assert_eq!(arguments["edges"], json!([selected]), "the scenario names the exact intended rail");
    arguments["edges"] = json!([selected]);
    let fillet = |state: &mut AppState, limit| {
        with_request_allowance(limit, || call_checked(&mut AppTools { state }, "fillet", arguments.clone(), &caller))
    };
    // This allowance reaches display and runs out before certification.
    if let Some(limit) = refusal_budget {
        let (out, work) = fillet(&mut state, limit);
        assert!(out.is_error && work.exhausted, "{}; {work:?}", out.content);
        assert!(work.used <= work.limit && work.limit == limit, "{work:?}");
        assert_eq!(state.doc.display_failure(), Some(out.content.as_str()));
        let measured = format!("{} work units", work.used);
        for expected in ["SD-DISPLAY-01", "0.01 mm", measured.as_str(), &limit.to_string()] {
            assert!(out.content.contains(expected), "missing {expected}: {}", out.content);
        }
        for forbidden in ["kernel", "step op_", "mesh.spline", "numbers.operation"] {
            assert!(!out.content.contains(forbidden), "internal detail {forbidden}: {}", out.content);
        }
        assert_eq!(state.doc.model().to_bytes().unwrap(), before);
        assert_eq!(state.doc.undo_depth(), undo);
        assert!(!state.doc.features.iter().any(|feature| feature.name == "Fillet1"));
        assert!(!state.doc.proof_state().answered_within_bound);
        std::fs::remove_dir_all(dir).unwrap();
        return;
    }
    let (out, work) = fillet(&mut state, APP_WORK_TARGET);
    assert!(!out.is_error && !work.exhausted && work.used <= APP_WORK_TARGET, "{}; {work:?}", out.content);
    assert_ne!(state.doc.model().to_bytes().unwrap(), before);
    assert_eq!(state.doc.undo_depth(), undo + 1);
    let feature = state.doc.features.last().unwrap().id;
    let check = |state: &AppState| {
        let doc = &state.doc;
        let proof = doc.proof_state();
        assert!(proof.current_proved_model() && proof.current_proved_display());
        assert!(!proof.answered_within_bound && !proof.display_only_bound);
        assert_eq!(doc.features.last().unwrap().name, "Fillet1");
        assert!(doc.display_answer(feature).is_none());
        assert!(state.edit_status().is_none(), "a certified display lights nothing");
        let body = doc.display_body().unwrap();
        assert!(!doc.meshes[&body].indices.is_empty(), "the app retains the selectable certified mesh");
        assert_eq!(doc.faces.len(), 7);
        let tolerance = doc.kernel().topology(body).unwrap().display_chord_height.unwrap();
        assert!((0.0..=0.01).contains(&tolerance), "{tolerance}");
        let mesh = doc.kernel().rendering(body, &soliddraft_kernel_api::TessellationParams::default()).unwrap();
        assert!(!mesh.unproved && mesh.bounded.is_none() && !mesh.indices.is_empty());
    };
    check(&state);
    let body = state.doc.display_body().unwrap();
    for chord_height in [0.01, 0.005] {
        let (certified, work) = with_request_allowance(0, || {
            state
                .doc
                .kernel()
                .tessellate(body, &soliddraft_kernel_api::TessellationParams { chord_height, ..Default::default() })
        });
        assert!(
            certified.is_err() && work.exhausted,
            "display evidence cannot answer an independent export request at {chord_height} mm: {work:?}"
        );
    }
    for (tool, arguments) in [
        ("undo", json!({})),
        ("redo", json!({})),
        ("save", json!({"path": dir.join("fillet.dwprt")})),
        ("open", json!({"path": dir.join("fillet.dwprt")})),
    ] {
        let out = call_checked(&mut AppTools { state: &mut state }, tool, arguments, &caller);
        assert!(!out.is_error, "{tool}: {}", out.content);
        if tool == "undo" {
            assert_eq!(state.doc.model().geometry_hash(), geometry);
            assert_eq!(state.doc.display_body(), retained_body);
            assert_eq!(names(&state), retained_names);
            assert!(!state.doc.proof_state().answered_within_bound);
        } else {
            check(&state);
        }
    }
    let path = dir.join("independent-exact.step");
    let out = call_checked(&mut AppTools { state: &mut state }, "export_step", json!({"path":path}), &caller);
    assert!(!out.is_error, "independent exact STEP: {}", out.content);
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
