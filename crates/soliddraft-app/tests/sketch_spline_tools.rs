//! Spline tool requests preserve the runtime point minimum and batch undo boundary.

use serde_json::json;
use soliddraft_app::{app_tools::AppTools, document::Document, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};

#[test]
fn spline_point_minimum_is_atomic_for_both_modes() {
    for mode in ["fit", "control"] {
        let mut state = AppState::new(Document::new("Spline tools", soliddraft_app::kernel()));
        let caller = Caller::Mcp { client: "spline tools".into() };
        let out = call_checked(
            &mut AppTools { state: &mut state },
            "sketch_geometry",
            json!({"plane":"top","operations":[
                {"tool":"add","kind":{"type":"spline","mode":mode,
                    "points":[{"u":0,"v":0},{"u":2,"v":1}]},"construction":false}
            ]}),
            &caller,
        );
        assert!(!out.is_error, "{}", out.content);
        let before = state.doc.model().part_json();
        let revision = state.doc.revision();
        let depth = state.doc.undo_depth();
        for points in [json!([]), json!([{"u":0,"v":0}])] {
            let out = call_checked(
                &mut AppTools { state: &mut state },
                "sketch_geometry",
                json!({"plane":"top","operations":[
                    {"tool":"add","kind":{"type":"line","a":{"u":0,"v":0},
                        "b":{"u":1,"v":0}},"construction":false},
                    {"tool":"add","kind":{"type":"spline","mode":mode,
                        "points":points},"construction":false}
                ]}),
                &caller,
            );
            assert!(out.is_error, "{mode}: {}", out.content);
            assert_eq!(state.doc.model().part_json(), before);
            assert_eq!(state.doc.revision(), revision);
            assert_eq!(state.doc.undo_depth(), depth);
        }
    }
}
