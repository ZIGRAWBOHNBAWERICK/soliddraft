//! Pattern requests keep runtime position validation and the batch undo boundary.

use serde_json::json;
use soliddraft_app::{app_tools::AppTools, document::Document, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};

#[test]
fn skipped_pattern_positions_are_validated_atomically_for_creation_and_edits() {
    let mut state = AppState::new(Document::new("Pattern tools", soliddraft_app::kernel()));
    let caller = Caller::Mcp { client: "pattern tools".into() };
    let spec = json!({"type":"linear","step":{"u":10,"v":0},"count":3});
    let circle = json!({"tool":"add","kind":{"type":"circle","center":{"u":0,"v":0},"radius":1},"construction":false});
    let pattern = json!({"tool":"pattern","entities":[1],"spec":spec,"skipped":[[1,0]]});
    for skipped in [json!([]), json!([[1, 0]])] {
        let out = call_checked(
            &mut AppTools { state: &mut state },
            "sketch_geometry",
            json!({"plane":"top","operations":[circle,pattern,
                {"tool":"edit_pattern","group":1,"spec":spec,"skipped":skipped}
            ]}),
            &caller,
        );
        assert!(!out.is_error, "{}", out.content);
    }
    let before = state.doc.model().part_json();
    let revision = state.doc.revision();
    let depth = state.doc.undo_depth();
    for tool in ["pattern", "edit_pattern"] {
        for skipped in [
            json!([[]]),
            json!([[1]]),
            json!([[1, 0, 0]]),
            json!([[-1, 0]]),
            json!([[1.5, 0]]),
            json!([[4294967296_u64, 0]]),
            json!([[0, 0]]),
            json!([[3, 0]]),
            json!([[1, 1]]),
            json!([[1, 0], [1, 0]]),
        ] {
            let mut operation = json!({"tool":tool,"spec":spec,"skipped":skipped});
            if tool == "pattern" {
                operation["entities"] = json!([1]);
            } else {
                operation["group"] = json!(1);
            }
            let mut operations = vec![circle.clone()];
            if tool == "edit_pattern" {
                operations.push(pattern.clone());
            }
            operations.push(operation);
            let out = call_checked(
                &mut AppTools { state: &mut state },
                "sketch_geometry",
                json!({"plane":"top","operations":operations}),
                &caller,
            );
            assert!(out.is_error, "{tool}, {skipped}: {}", out.content);
            assert_eq!(state.doc.model().part_json(), before);
            assert_eq!(state.doc.revision(), revision);
            assert_eq!(state.doc.undo_depth(), depth);
        }
    }
}
