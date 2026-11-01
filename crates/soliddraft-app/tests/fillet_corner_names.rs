//! The six spherical corners of a triangular prism with all nine edges filleted carry
//! persistent names. Each name stays on the corner it started on through an upstream depth
//! edit and a radius edit, read through the same feature query an agent uses.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use soliddraft_kernel_api::{FaceId, FaceInfo, SurfaceKind};
use soliddraft_materials::{Appearance, Srgb};
use soliddraft_model::AppearanceTarget;

const SCENARIO: &str = include_str!("../../../ci/scenarios/skew-corner-fillet.json");

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

fn faces(state: &AppState) -> Vec<FaceInfo> {
    let doc = &state.doc;
    doc.kernel().topology(doc.display_body().expect("the prism")).unwrap().faces
}

/// The faces that show an appearance, from the production resolver. Nothing may be lost.
fn shown(state: &AppState, look: &Appearance) -> Vec<FaceId> {
    let table = state.doc.model().appearance_table(None);
    assert!(table.lost.is_empty(), "{:?}", table.lost);
    let index = table.appearances.iter().position(|a| a.id == look.id).expect("the appearance") as u32;
    table.faces.iter().filter(|(_, i)| **i == index).map(|(face, _)| *face).collect()
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}

/// The centroids at radius r of the strip along the acute vertical edge and of the corner where
/// it meets the start cap. The contact normals (0,-1,0) and (3/5,4/5,0) turn by `theta` around
/// the rolling centre (8-3r, r), and the mean of the unit normal over that arc is `along`.
fn acute_centroids(r: f64) -> ([f64; 3], [f64; 3]) {
    let theta = (-0.8f64).acos();
    let bisector = [3.0 / 10f64.sqrt(), -1.0 / 10f64.sqrt()];
    let along = (theta / 2.0).sin() / (theta / 2.0);
    let centre = [8.0 - 3.0 * r, r];
    let strip = [centre[0] + r * along * bisector[0], centre[1] + r * along * bisector[1], 5.0];
    let sector = std::f64::consts::FRAC_PI_4 * along;
    let corner = [centre[0] + r * sector * bisector[0], centre[1] + r * sector * bisector[1], r / 2.0];
    (strip, corner)
}

/// A stored reference is taken once on a skew strip and on one sphere whose twin at the other
/// end of the prism has the same area, so a value alone could not tell them apart. It must
/// resolve to the same regions through the edit, the history, a save and a fresh open.
#[test]
fn a_stored_corner_reference_stays_on_its_own_sphere() {
    let dir = std::env::temp_dir().join(format!("soliddraft-corner-reference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut state = AppState::new(soliddraft_app::new_document());
    let scenario: Value = serde_json::from_str(SCENARIO).unwrap();
    let steps = scenario["steps"].as_array().unwrap();
    for step in steps[..4].iter().filter(|s| s["tool"] != "feature") {
        run(&mut state, step["tool"].as_str().unwrap(), step["arguments"].clone());
    }

    let (strip, corner) = acute_centroids(1.0);
    let found = faces(&state);
    let nearest = |surface: SurfaceKind, at: [f64; 3]| {
        let face = found.iter().find(|f| {
            let c = f.centroid;
            f.surface == surface && distance([c.x, c.y, c.z], at) < 0.02
        });
        face.expect("the face at its derived centroid").id
    };
    let references = [
        state.doc.model().face_ref(nearest(SurfaceKind::Cylinder, strip)).expect("the strip is one named face"),
        state.doc.model().face_ref(nearest(SurfaceKind::Sphere, corner)).expect("the corner is one named face"),
    ];
    let looks = [
        Appearance::new("strip-blue", "Strip blue", Srgb::new(30, 60, 200)),
        Appearance::new("corner-green", "Corner green", Srgb::new(30, 180, 60)),
    ];
    let targets = references.map(|face| AppearanceTarget::Face { face });
    for (target, look) in targets.iter().zip(&looks) {
        state.doc.set_appearance(target.clone(), Some(look.clone())).unwrap();
    }

    // The same stored references, never recreated, against the current geometry.
    let check = |state: &AppState, r: f64, stage: &str| {
        let stored: Vec<_> = state.doc.model().materials().appearances.iter().map(|a| a.target.clone()).collect();
        assert_eq!(stored, targets, "{stage}");
        let (strip, corner) = acute_centroids(r);
        let all = faces(state);
        for (look, surface, at) in [(&looks[0], SurfaceKind::Cylinder, strip), (&looks[1], SurfaceKind::Sphere, corner)]
        {
            let [id] = shown(state, look)[..] else { panic!("{stage}: one face for {}", look.id) };
            let face = all.iter().find(|f| f.id == id).unwrap();
            let c = face.centroid;
            assert_eq!(face.surface, surface, "{stage}");
            assert!(distance([c.x, c.y, c.z], at) < 0.02, "{stage}: {} at {c:?}, expected {at:?}", look.id);
        }
    };

    check(&state, 1.0, "build");
    run(&mut state, "edit_fillet", json!({ "feature": "Fillet1", "radius": 0.5 }));
    check(&state, 0.5, "edit");
    run(&mut state, "undo", json!({}));
    check(&state, 1.0, "undo");
    run(&mut state, "redo", json!({}));
    check(&state, 0.5, "redo");

    let path = dir.join("skew-corner.dwprt");
    run(&mut state, "save", json!({ "path": path }));
    drop(state);
    let doc = soliddraft_app::file::open(&path, soliddraft_app::kernel(), None).unwrap();
    let mut reopened = AppState::new(doc);
    check(&reopened, 0.5, "fresh open");
    let caller = Caller::Mcp { client: "test".into() };
    let refused = call_checked(
        &mut AppTools { state: &mut reopened },
        "edit_fillet",
        json!({ "feature": "Fillet1", "radius": 0 }),
        &caller,
    );
    assert!(refused.is_error, "{}", refused.content);
    check(&reopened, 0.5, "refused radius 0");
    std::fs::remove_dir_all(&dir).unwrap();
}
