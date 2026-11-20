//! The P1 bracket with its fourteen items of manufacturing information, built and
//! annotated through the application's tools.
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, document::Document, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_api::SurfaceKind;

pub fn state(name: &str) -> AppState {
    let kernel = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(20_000_000));
    AppState::new(Document::new(name, kernel))
}

pub fn call(state: &mut AppState, tool: &str, arguments: Value) -> Value {
    let out = call_checked(&mut AppTools { state }, tool, arguments, &Caller::Mcp { client: "pmi-bracket".into() });
    assert!(!out.is_error, "{tool}: {}", out.content);
    serde_json::from_str(&out.content).unwrap_or(Value::Null)
}

/// The bracket's build steps of the canonical scenario: everything before its edits.
pub fn build(state: &mut AppState) {
    let scenario: Value = serde_json::from_str(include_str!("../../../ci/scenarios/canonical-p1.json")).unwrap();
    for step in scenario["steps"].as_array().unwrap() {
        call(state, step["tool"].as_str().unwrap(), step["arguments"].clone());
        if step["label"] == "original" {
            return;
        }
    }
    panic!("the scenario has no step labelled original");
}

/// The name of the one face of `surface` whose centroid is at `at`; `None` is any value.
pub fn face(state: &AppState, surface: SurfaceKind, at: [Option<f64>; 3]) -> String {
    let found: Vec<_> = state
        .doc
        .faces
        .iter()
        .filter(|f| f.surface == surface)
        .filter(|f| {
            // The app keeps display centroids in single precision.
            let c = [f.centroid.x, f.centroid.y, f.centroid.z].map(f64::from);
            at.iter().zip(c).all(|(want, have)| want.is_none_or(|want| (want - have).abs() < 1e-3))
        })
        .collect();
    assert_eq!(found.len(), 1, "one {surface:?} face at {at:?}, found {}", found.len());
    let feature = state.doc.feature(found[0].feature).unwrap();
    format!("{} \u{b7} {}", feature.name, found[0].label)
}

/// The faces the bracket's items go on, by name.
pub struct Faces {
    pub a: String,
    pub b: String,
    pub c: String,
    pub bore: String,
    pub boss: String,
    pub boss_top: String,
    pub holes: Vec<String>,
    pub slot_low: String,
    pub slot_high: String,
    pub slot_ends: [String; 2],
}

pub fn faces(state: &AppState, bore_x: f64) -> Faces {
    let plane = |at| face(state, SurfaceKind::Plane, at);
    let cylinder = |at| face(state, SurfaceKind::Cylinder, at);
    // The slot's rounded ends are half cylinders about x = 23 and x = 37, so their
    // centroids lie outside those axes.
    let ends = |low: bool| {
        let found: Vec<_> = state
            .doc
            .faces
            .iter()
            .filter(|f| f.surface == SurfaceKind::Cylinder && (f.centroid.z - 18.0).abs() < 1e-3)
            .filter(|f| (f.centroid.x < 30.0) == low)
            .collect();
        assert_eq!(found.len(), 1, "one slot end");
        format!("{} \u{b7} {}", state.doc.feature(found[0].feature).unwrap().name, found[0].label)
    };
    Faces {
        a: plane([None, None, Some(0.0)]),
        b: plane([None, Some(0.0), None]),
        c: plane([Some(0.0), None, None]),
        bore: cylinder([Some(bore_x), Some(25.0), Some(7.5)]),
        boss: cylinder([Some(30.0), Some(25.0), Some(10.0)]),
        boss_top: plane([Some(30.0), Some(25.0), Some(15.0)]),
        holes: [[10.0, 12.0], [10.0, 32.0], [50.0, 12.0], [50.0, 32.0]]
            .into_iter()
            .map(|[x, y]| cylinder([Some(x), Some(y), Some(2.5)]))
            .collect(),
        slot_low: plane([Some(30.0), Some(2.5), Some(15.0)]),
        slot_high: plane([Some(30.0), Some(2.5), Some(21.0)]),
        slot_ends: [ends(true), ends(false)],
    }
}

/// Adds the fourteen items and the part's notes.
pub fn annotate(state: &mut AppState) {
    let f = faces(state, 30.0);
    let pattern: Vec<Value> = f.holes.iter().map(|hole| json!([hole])).collect();
    let abc = json!(["A", "B", "C"]);
    let items = [
        json!({"kind": "datum", "letter": "A", "faces": [f.a]}),
        json!({"kind": "datum", "letter": "B", "faces": [f.b]}),
        json!({"kind": "datum", "letter": "C", "faces": [f.c]}),
        json!({"kind": "diameter", "name": "Bore diameter", "faces": [f.bore], "tolerance": "H7"}),
        json!({"kind": "finish", "name": "Bore finish", "faces": [f.bore], "roughness": "Ra 1.6"}),
        json!({"kind": "tolerance", "name": "Bore position", "characteristic": "position", "value": "0.05",
               "diameter_zone": true, "datums": abc, "faces": [f.bore]}),
        json!({"kind": "diameter", "name": "Boss diameter", "faces": [f.boss], "tolerance": "\u{b1}0.1"}),
        json!({"kind": "distance", "name": "Boss height", "from_faces": [f.a], "to_faces": [f.boss_top],
               "tolerance": "\u{b1}0.1"}),
        json!({"kind": "diameter", "name": "Hole diameter", "features": pattern, "tolerance": "+0.1/0",
               "through": true}),
        json!({"kind": "tolerance", "name": "Hole position", "characteristic": "position", "value": "0.2",
               "diameter_zone": true, "datums": abc, "features": pattern}),
        json!({"kind": "slot", "name": "Slot", "flats": [[f.slot_low], [f.slot_high]],
               "ends": [[f.slot_ends[0]], [f.slot_ends[1]]], "tolerance": "+0.1/0"}),
        json!({"kind": "distance", "name": "Slot height", "from_faces": [f.a],
               "to_faces": [f.slot_low, f.slot_high], "to_at": "middle", "tolerance": "\u{b1}0.1"}),
        json!({"kind": "tolerance", "name": "Flatness of A", "characteristic": "flatness", "value": "0.05",
               "faces": [f.a]}),
        json!({"kind": "tolerance", "name": "Perpendicularity of B", "characteristic": "perpendicularity",
               "value": "0.1", "datums": ["A"], "faces": [f.b]}),
    ];
    for item in items {
        call(state, "pmi_add", item);
    }
    call(
        state,
        "pmi_notes",
        json!({"general_tolerance": "ISO 2768-m", "material": "6061-T6", "notes": ["Break sharp edges"]}),
    );
}
