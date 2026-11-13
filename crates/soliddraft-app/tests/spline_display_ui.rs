//! Native spline display retains its bounded answer in both application frames.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui_kittest::kittest::Queryable;
use serde_json::Value;
use soliddraft_app::{app_tools::AppTools, settings::FrameLayout};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_catallax::{APP_REQUEST_WORK, with_request_allowance};

#[test]
fn native_scaled_spline_fillet_shows_its_numbered_refusal_and_work_in_both_frames() {
    check_display("scaled", include_str!("../../../ci/scenarios/loft-edge-fillet.json"), false);
}

#[test]
fn native_small_warp_spline_fillet_shows_its_bounded_answer_in_both_frames() {
    check_display("small-warp", include_str!("../../../ci/scenarios/loft-edge-fillet-small-warp.json"), true);
}

fn check_display(name: &str, source: &str, bounded: bool) {
    let case = ribbon::case(&format!("native-spline-display-{name}"));
    let scenario: Value = serde_json::from_str(source).unwrap();
    let caller = Caller::Mcp { client: "native-spline-display-ui".into() };
    for layout in [FrameLayout::Classic, FrameLayout::EditSuite] {
        let mut app = ribbon::app(soliddraft_app::new_document());
        app.state_mut().state.frame_layout = layout;
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
            let (out, work) = with_request_allowance(APP_REQUEST_WORK, || {
                call_checked(
                    &mut AppTools { state: &mut app.state_mut().state },
                    "fillet",
                    scenario["steps"][3]["arguments"].clone(),
                    &caller,
                )
            });
            eprintln!("{layout:?}: {work:?}; {}", out.content);
            let probe = |app: &ribbon::App, stage: &str| {
                let state = &app.state().state;
                eprintln!(
                    "[DEBUG-f04-ui] {stage}: meshes={}, faces={}, failure={:?}, note={:?}, topology={:?}",
                    state.doc.meshes.len(),
                    state.doc.faces.len(),
                    state.doc.display_failure(),
                    state.note.as_ref().map(|n| &n.text),
                    state.doc.display_body().map(|body| state
                        .doc
                        .kernel()
                        .topology(body)
                        .map(|t| (t.faces.len(), t.edges.len())))
                );
            };
            probe(app, "before frames");
            app.run_steps(8);
            probe(app, "after frames");
            if bounded {
                assert!(!out.is_error && !work.exhausted && work.used <= APP_REQUEST_WORK, "{}; {work:?}", out.content);
                assert_ne!(app.state().state.doc.model().to_bytes().unwrap(), before);
                assert_eq!(app.state().state.doc.undo_depth(), undo + 1);
                let doc = &app.state().state.doc;
                assert_eq!(doc.features.last().unwrap().name, "Fillet1");
                let proof = doc.proof_state();
                assert!(proof.answered_within_bound && proof.display_only_bound);
                assert!(!proof.current_proved_display());
                let mesh = doc
                    .kernel()
                    .rendering(doc.display_body().unwrap(), &soliddraft_kernel_api::TessellationParams::default())
                    .unwrap();
                let bound = mesh.bounded.as_ref().unwrap();
                assert!(mesh.unproved && bound.measured_mm <= 0.01);
                assert!(!bound.method.is_empty() && !bound.definition.is_empty());
                assert!(!app.state().state.doc.meshes.is_empty(), "the bounded fillet is visible");
                assert_eq!(app.state().state.edit_status().unwrap().0, soliddraft_app::document::WITHIN_BOUND_LABEL);
                if layout == FrameLayout::EditSuite {
                    assert!(app.query_all_by_label("Fillet1 proof: Answered within a stated bound").next().is_some());
                    assert!(app.query_all_by_label("Fillet1 proof: Proved").next().is_none());
                }
            } else {
                assert!(out.is_error && work.exhausted, "{}; {work:?}", out.content);
                assert_eq!(work.used, 19_999_956);
                assert_eq!(app.state().state.doc.model().to_bytes().unwrap(), before);
                assert_eq!(app.state().state.doc.undo_depth(), undo);
                assert!(!app.state().state.doc.features.iter().any(|feature| feature.name == "Fillet1"));
                assert!(!app.state().state.doc.proof_state().answered_within_bound);
            }
            if !bounded {
                app.get_by_label("Status details").click();
                app.run_steps(4);
                let painted = ribbon::painted(app);
                for expected in ["SD-DISPLAY-01", "19999956 work units", "20000000"] {
                    assert!(
                        painted.iter().any(|text| text.text.contains(expected)),
                        "missing {expected} in status chip details"
                    );
                }
            }
            case.frame(app, &format!("{layout:?}-fillet"));
        });
    }
}
