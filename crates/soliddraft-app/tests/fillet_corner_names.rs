//! The six spherical corners of a triangular prism with all nine edges filleted carry
//! persistent names. Each name stays on the corner it started on through an upstream depth
//! edit and a radius edit, read through the same feature query an agent uses.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;

fn run(state: &mut AppState, name: &str, arguments: Value) -> Value {
    let caller = Caller::Mcp { client: "test".into() };
    let out = call_checked(&mut AppTools { state }, name, arguments, &caller);
    assert!(!out.is_error, "{name}: {}", out.content);
    serde_json::from_str(&out.content).unwrap()
}

/// One corner as its name reports it: the blends it borders and its centroid.
#[derive(Debug)]
struct Corner {
    blends: BTreeSet<String>,
    centroid: [f64; 3],
}

fn corners(state: &mut AppState) -> BTreeMap<String, Corner> {
    let feature = run(state, "feature", json!({ "name": "Fillet1" }));
    let names = |face: &Value| face["name"].as_str().unwrap().to_owned();
    let spheres = feature["faces"].as_array().unwrap().iter().filter(|f| f["surface"] == "sphere");
    let found: BTreeMap<_, _> = spheres
        .map(|face| {
            let centroid = face["centroid"].as_array().unwrap();
            let centroid = [0, 1, 2].map(|i| centroid[i].as_f64().unwrap());
            let name = names(face);
            let edges = feature["edges"].as_array().unwrap().iter();
            let sides = edges.filter_map(|edge| {
                let faces: Vec<_> = edge["faces"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
                faces.contains(&name.as_str()).then(|| faces.iter().find(|f| **f != name).unwrap().to_string())
            });
            (name.clone(), Corner { blends: sides.collect(), centroid })
        })
        .collect();
    let expected: BTreeSet<_> = (1..=6).map(|n| format!("Fillet1 · Face {n}")).collect();
    assert_eq!(found.keys().cloned().collect::<BTreeSet<_>>(), expected, "{feature}");
    assert!(found.values().all(|c| c.blends.len() == 3 && c.blends.iter().all(|b| b.contains("Blend"))), "{found:?}");
    found
}

#[test]
fn spherical_corners_keep_their_names_through_upstream_and_radius_edits() {
    let dir = std::env::temp_dir().join(format!("soliddraft-corner-names-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut state = AppState::new(soliddraft_app::new_document());
    let line = |a: [f64; 2], b: [f64; 2]| {
        let (a, b) = (json!({ "u": a[0], "v": a[1] }), json!({ "u": b[0], "v": b[1] }));
        json!({ "tool": "add", "kind": { "type": "line", "a": a, "b": b }, "construction": false })
    };
    let triangle = [line([0.0, 0.0], [8.0, 0.0]), line([8.0, 0.0], [0.0, 6.0]), line([0.0, 6.0], [0.0, 0.0])];
    run(&mut state, "sketch_geometry", json!({ "plane": "top", "operations": triangle }));
    run(&mut state, "extrude", json!({ "sketch": "Sketch1", "depth": 10, "flip": false }));
    let edges: Vec<_> = (1..=9).map(|n| format!("Boss-Extrude1 · Edge {n}")).collect();
    run(&mut state, "fillet", json!({ "edges": edges, "radius": 1 }));
    let first = corners(&mut state);

    run(&mut state, "edit_extrude", json!({ "feature": "Boss-Extrude1", "depth": 12, "flip": false }));
    let deeper = corners(&mut state);
    for (name, corner) in &first {
        let now = &deeper[name];
        assert_eq!(now.blends, corner.blends, "{name}");
        let lift = if corner.centroid[2] > 5.0 { 2.0 } else { 0.0 };
        let moved = [corner.centroid[0], corner.centroid[1], corner.centroid[2] + lift];
        assert!((0..3).all(|i| (now.centroid[i] - moved[i]).abs() < 0.02), "{name}: {now:?} from {corner:?}");
    }

    run(&mut state, "edit_fillet", json!({ "feature": "Fillet1", "radius": 0.5 }));
    let smaller = corners(&mut state);
    for (name, corner) in &deeper {
        let now = &smaller[name];
        assert_eq!(now.blends, corner.blends, "{name}");
        // Halving the radius moves a corner less than 2 mm, and the next corner stands at
        // least 2.5 mm further off, so a name that changed corners fails here.
        let moved = (0..3).map(|i| (now.centroid[i] - corner.centroid[i]).powi(2)).sum::<f64>().sqrt();
        assert!(moved < 2.0, "{name}: {now:?} from {corner:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
