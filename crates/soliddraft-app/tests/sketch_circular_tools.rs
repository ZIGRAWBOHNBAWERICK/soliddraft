//! MCP callers share circular placement and atomic refusal with the editor.

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, document::Document, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};

fn call(state: &mut AppState, name: &str, arguments: Value, refused: bool) {
    let out = call_checked(&mut AppTools { state }, name, arguments, &Caller::Mcp { client: "circular".into() });
    assert_eq!(out.is_error, refused, "{}", out.content);
}

#[test]
fn circular_operations_use_strict_schema_and_atomic_batch_history() {
    let mut state = AppState::new(Document::new("Circular", soliddraft_app::kernel()));
    call(
        &mut state,
        "sketch_geometry",
        json!({"plane":"top","operations":[
            {"tool":"perimeter_circle","a":{"u":5,"v":0},"b":{"u":0,"v":5},"c":{"u":-5,"v":0}},
            {"tool":"centrepoint_arc","center":{"u":20,"v":0},"start":{"u":25,"v":0},"end":{"u":20,"v":-5},"clockwise":false},
            {"tool":"three_point_arc","start":{"u":45,"v":0},"end":{"u":40,"v":-5},"through":{"u":35,"v":0},"construction":true},
            {"tool":"add","kind":{"type":"line","a":{"u":-20,"v":0},"b":{"u":-15,"v":0}},"construction":false},
            {"tool":"tangent_arc","from":{"entity":4,"index":1},"end":{"u":-10,"v":5}}
        ]}),
        false,
    );
    let sketch = &state.doc.features[0].sketch().unwrap().sketch;
    assert_eq!(sketch.drawn().count(), 5);
    assert!(sketch.entity(3).unwrap().construction);
    assert!(
        sketch
            .constraints
            .iter()
            .any(|c| matches!(c.kind, soliddraft_app::sketch::ConstraintKind::Tangent { a: 4, b: 5 }))
    );
    let after = state.doc.model().part_json();
    call(&mut state, "undo", json!({}), false);
    assert!(state.doc.features.is_empty());
    call(&mut state, "redo", json!({}), false);
    assert_eq!(state.doc.model().part_json(), after);
    for bad in [
        json!({"tool":"perimeter_circle","a":{"u":0,"v":0},"b":{"u":1,"v":0},"c":{"u":2,"v":0}}),
        json!({"tool":"three_point_arc","start":{"u":0,"v":0},"end":{"u":1,"v":0},"through":{"u":0,"v":0}}),
        json!({"tool":"tangent_arc","from":{"entity":1,"index":0},"end":{"u":10,"v":10}}),
        json!({"tool":"centrepoint_arc","center":{"u":0,"v":0},"start":{"u":5,"v":0},"end":{"u":0,"v":5},"extra":true}),
    ] {
        let depth = state.doc.undo_depth();
        call(
            &mut state,
            "sketch_geometry",
            json!({"plane":"top","operations":[
                {"tool":"add","kind":{"type":"circle","center":{"u":0,"v":0},"radius":5},"construction":false}, bad
            ]}),
            true,
        );
        assert_eq!(state.doc.model().part_json(), after);
        assert_eq!(state.doc.undo_depth(), depth);
    }
}
