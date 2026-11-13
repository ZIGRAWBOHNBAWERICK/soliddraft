//! Native spline refusal preserves the rendered part in both application frames.

#[path = "support/ribbon.rs"]
mod ribbon;

use serde_json::Value;
use soliddraft_app::{app_tools::AppTools, settings::FrameLayout};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_catallax::{APP_REQUEST_WORK, with_request_allowance};

#[test]
fn native_spline_fillet_refusal_keeps_the_visible_loft_in_both_frames() {
    let case = ribbon::case("native-spline-display-refusal");
    let scenario: Value = serde_json::from_str(include_str!("../../../ci/scenarios/loft-edge-fillet.json")).unwrap();
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
            assert!(out.is_error && work.exhausted, "{}; {work:?}", out.content);
            assert!(out.content.contains("SD-DISPLAY-01") && out.content.contains("20000000"), "{}", out.content);
            for forbidden in ["kernel", "step op_", "mesh.spline", "numbers.operation"] {
                assert!(!out.content.contains(forbidden), "{forbidden}: {}", out.content);
            }
            assert_eq!(app.state().state.doc.model().to_bytes().unwrap(), before);
            assert_eq!(app.state().state.doc.undo_depth(), undo);
            assert!(!app.state().state.doc.features.iter().any(|feature| feature.name == "Fillet1"));
            assert!(!app.state().state.doc.proof_state().answered_within_bound);
            app.run_steps(8);
            assert!(!app.state().state.doc.meshes.is_empty(), "the accepted loft remains visible");
            case.frame(app, &format!("{layout:?}-retained-loft"));
        });
    }
}
