//! Edit Suite shows a proved spline display and the measured work when display runs out of budget.

#[path = "support/ui.rs"]
mod ribbon;

use egui_kittest::kittest::Queryable;
use serde_json::Value;
use soliddraft_app::app_tools::AppTools;
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_catallax::{APP_WORK_TARGET, with_request_allowance};

#[test]
fn native_scaled_spline_fillet_shows_a_proved_display_at_the_application_work_target() {
    check_display("scaled", include_str!("../../../ci/scenarios/loft-edge-fillet.json"), None);
}

#[test]
fn native_scaled_spline_fillet_shows_its_reduced_budget_refusal_in_edit_suite() {
    check_display("scaled-refused", include_str!("../../../ci/scenarios/loft-edge-fillet.json"), Some(9_600_000));
}

#[test]
fn native_small_warp_spline_fillet_shows_its_proved_answer_in_edit_suite() {
    check_display("small-warp", include_str!("../../../ci/scenarios/loft-edge-fillet-small-warp.json"), None);
}

fn check_display(name: &str, source: &str, refusal_budget: Option<u64>) {
    let case = ribbon::case(&format!("native-spline-display-{name}"));
    let scenario: Value = serde_json::from_str(source).unwrap();
    let caller = Caller::Mcp { client: "native-spline-display-ui".into() };
    {
        let layout = "edit_suite";
        let mut app = ribbon::app(soliddraft_app::startup::new_document());
        case.run(&mut app, |app| {
            for step in scenario["steps"].as_array().unwrap().iter().take(3) {
                let name = step["tool"].as_str().unwrap();
                let out = call_checked(
                    &mut AppTools { state: &mut app.state_mut().state },
                    name,
                    step["arguments"].clone(),
                    &caller,
                );
                assert!(!out.is_error, "{name}: {}", out.content);
            }
            app.run_steps(8);
            ribbon::run_command(app, "zoom_fit");
            app.run_steps(8);
            let before = app.state().state.doc.model().to_bytes().unwrap();
            let undo = app.state().state.doc.undo_depth();
            let fillet = |app: &mut ribbon::App, limit| {
                with_request_allowance(limit, || {
                    call_checked(
                        &mut AppTools { state: &mut app.state_mut().state },
                        "fillet",
                        scenario["steps"][3]["arguments"].clone(),
                        &caller,
                    )
                })
            };
            if let Some(limit) = refusal_budget {
                let (out, work) = fillet(app, limit);
                assert!(out.is_error && work.exhausted, "{}; {work:?}", out.content);
                assert!(work.used <= work.limit && work.limit == limit, "{work:?}");
                assert_eq!(app.state().state.doc.model().to_bytes().unwrap(), before);
                assert_eq!(app.state().state.doc.undo_depth(), undo);
                assert!(!app.state().state.doc.features.iter().any(|feature| feature.name == "Fillet1"));
                assert!(!app.state().state.doc.proof_state().answered_within_bound);
                app.run_steps(8);
                app.get_by_label("Status details").click();
                app.run_steps(4);
                let painted = ribbon::painted(app);
                let measured = format!("{} work units", work.used);
                for expected in ["SD-DISPLAY-01", measured.as_str(), &limit.to_string()] {
                    assert!(
                        painted.iter().any(|text| text.text.contains(expected)),
                        "missing {expected} in status chip details: {}",
                        out.content
                    );
                }
                case.frame(app, "edit-suite-fillet-refused");
                return;
            }
            let (out, work) = fillet(app, APP_WORK_TARGET);
            assert!(!out.is_error && !work.exhausted && work.used <= APP_WORK_TARGET, "{}; {work:?}", out.content);
            app.run_steps(8);
            let doc = &app.state().state.doc;
            assert_ne!(doc.model().to_bytes().unwrap(), before);
            assert_eq!(doc.undo_depth(), undo + 1);
            let fillet = doc.features.last().unwrap();
            assert_eq!(fillet.name, "Fillet1");
            let proof = doc.proof_state();
            assert!(proof.current_proved_model() && proof.current_proved_display());
            assert!(!proof.answered_within_bound && !proof.display_only_bound);
            assert!(doc.display_answer(fillet.id).is_none());
            let body = doc.display_body().unwrap();
            assert!(!doc.meshes[&body].indices.is_empty(), "the proved fillet is visible");
            let mesh = doc.kernel().rendering(body, &Default::default()).unwrap();
            assert!(!mesh.unproved && mesh.bounded.is_none() && !mesh.indices.is_empty());
            let tolerance = doc.kernel().topology(body).unwrap().display_chord_height.unwrap();
            assert!((0.0..=0.01).contains(&tolerance), "{tolerance}");
            assert!(app.state().state.edit_status().is_none());
            assert!(app.query_all_by_label_contains("Fillet1 status:").next().is_none());
            case.frame(app, &format!("{layout:?}-fillet"));
        });
    }
}
