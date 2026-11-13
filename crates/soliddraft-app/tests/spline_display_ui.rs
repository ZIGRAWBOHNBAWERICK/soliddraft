//! Native bounded rendering and its proof light through the real application.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui_kittest::kittest::Queryable;
use serde_json::Value;
use soliddraft_app::{app_tools::AppTools, document::WITHIN_BOUND_LABEL, settings::FrameLayout};
use soliddraft_assistant::{Caller, tools::call_checked};

#[test]
fn native_spline_fillet_shows_its_measured_kind_and_keeps_the_refused_part() {
    let case = ribbon::case("native-spline-display");
    let mut app = ribbon::app(soliddraft_app::new_document());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.state_mut().state.layout.tree.shown = false;
    let scenario: Value = serde_json::from_str(include_str!("../../../ci/scenarios/loft-edge-fillet.json")).unwrap();
    let caller = Caller::Mcp { client: "native-spline-display-ui".into() };
    case.run(&mut app, |app| {
        for step in scenario["steps"].as_array().unwrap().iter().take(4) {
            let name = step["tool"].as_str().unwrap();
            let out = call_checked(
                &mut AppTools { state: &mut app.state_mut().state },
                name,
                step["arguments"].clone(),
                &caller,
            );
            assert!(!out.is_error, "{name}: {}", out.content);
        }
        app.run_steps(12);
        let proof = app.state().state.doc.proof_state();
        assert!(proof.answered_within_bound && proof.display_only_bound);
        assert!(!proof.current_proved_display());
        assert_eq!(app.state().state.edit_status().unwrap().0, WITHIN_BOUND_LABEL);
        let badge = app.get_by_label("Fillet1 proof: Answered within a stated bound").rect();
        app.hover_at(badge.center());
        app.run_steps(90);
        let text = ribbon::painted(app).into_iter().map(|paint| paint.text).collect::<Vec<_>>().join("\n");
        for detail in ["0.01 mm", "spline display mesh", "Measurement:"] {
            assert!(text.contains(detail), "missing {detail} in the native proof-light tooltip");
        }
        case.frame(app, "within-stated-bound");
        let before = app.state().state.doc.model().to_bytes().unwrap();
        let out = call_checked(
            &mut AppTools { state: &mut app.state_mut().state },
            "fillet",
            scenario["steps"][7]["arguments"].clone(),
            &caller,
        );
        assert!(out.is_error && out.content.contains("already trimmed"), "{}", out.content);
        assert_eq!(app.state().state.doc.model().to_bytes().unwrap(), before);
        assert!(app.state().state.doc.proof_state().answered_within_bound);
        app.run_steps(8);
        case.frame(app, "trimmed-neighbour-refused");
    });
}
