//! MCP callers use the same transforms and atomic refusal policy as the editor.

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, document::Document, sketch::P2, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};

fn call(state: &mut AppState, name: &str, arguments: Value, refused: bool) {
    let out = call_checked(&mut AppTools { state }, name, arguments, &Caller::Mcp { client: "transforms".into() });
    assert_eq!(out.is_error, refused, "{}", out.content);
}

#[test]
fn headless_transforms_validate_schema_and_undo_the_whole_batch() {
    let mut state = AppState::new(Document::new("Transforms", soliddraft_app::kernel()));
    call(
        &mut state,
        "sketch_geometry",
        json!({"plane":"top","operations":[
            {"tool":"add","kind":{"type":"circle","center":{"u":2,"v":3},"radius":2},"construction":true},
            {"tool":"move","entities":[1],"displacement":{"u":4,"v":1}},
            {"tool":"copy","entities":[1],"base":{"u":6,"v":4},"displacement":{"u":10,"v":0}},
            {"tool":"rotate","entities":[1],"center":{"u":0,"v":0},"angle_degrees":90,"copy":true},
            {"tool":"scale","entities":[2],"base":{"u":1,"v":1},"factor":0.5},
            {"tool":"rotate","entities":[3],"center":{"u":1,"v":1},"angle_degrees":-90}
        ]}),
        false,
    );
    let sketch = &state.doc.features[0].sketch().unwrap().sketch;
    for (id, center, radius) in [(1, P2::new(6.0, 4.0), 2.0), (2, P2::new(8.5, 2.5), 1.0), (3, P2::new(6.0, 6.0), 2.0)]
    {
        let e = sketch.entity(id).unwrap();
        assert!(e.point(0).unwrap().dist(center) < 1e-6);
        assert!((e.radius().unwrap() - radius).abs() < 1e-6);
        assert!(e.construction);
    }
    let after = state.doc.model().part_json();
    call(&mut state, "undo", json!({}), false);
    assert!(state.doc.features.is_empty(), "one undo removes the whole transformed sketch");
    assert_eq!(state.doc.undo_depth(), 0);
    call(&mut state, "redo", json!({}), false);
    assert_eq!(state.doc.model().part_json(), after);

    let depth = state.doc.undo_depth();
    call(
        &mut state,
        "sketch_geometry",
        json!({"plane":"top","operations":[
            {"tool":"add","kind":{"type":"circle","center":{"u":2,"v":3},"radius":2},"construction":false},
            {"tool":"relate","relation":{"type":"fix_entity","entity":1}},
            {"tool":"scale","entities":[1],"base":{"u":0,"v":0},"factor":2}
        ]}),
        true,
    );
    assert_eq!(state.doc.model().part_json(), after);
    assert_eq!(state.doc.undo_depth(), depth);
}
