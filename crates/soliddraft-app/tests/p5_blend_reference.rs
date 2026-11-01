//! One blend reference across the P5 housing's fillet states. Track cannot hold a fitted
//! blend, which is several faces, so this stores the reference where a set is allowed: a
//! face appearance. Taken once on radius 2.5's four torus patches, it must follow the
//! blend through the radius 1 contact, undo, redo, a save and a fresh open.

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::{ToolOutput, call_checked};
use soliddraft_kernel_api::{FaceId, FaceInfo, Origin, SurfaceKind};
use soliddraft_materials::{Appearance, Srgb};
use soliddraft_model::{AppearanceTarget, FaceFingerprint, FaceRef, FeatureId, Generator};

const SCENARIO: &str = include_str!("../../../ci/scenarios/p5-flanged-housing.json");

fn call(state: &mut AppState, name: &str, arguments: Value) -> ToolOutput {
    let caller = Caller::Mcp { client: "test".into() };
    call_checked(&mut AppTools { state }, name, arguments, &caller)
}

fn run(state: &mut AppState, name: &str, arguments: Value) {
    let out = call(state, name, arguments);
    assert!(!out.is_error, "{name}: {}", out.content);
}

fn faces(state: &AppState) -> Vec<FaceInfo> {
    let doc = &state.doc;
    doc.kernel().topology(doc.display_body().expect("the housing")).unwrap().faces
}

/// The feature with this name, as the model numbers it.
fn feature(state: &AppState, name: &str) -> FeatureId {
    FeatureId(state.doc.features.iter().find(|f| f.name == name).expect("the feature").id as u32)
}

/// What a feature's generator stands for in this rebuild.
fn origin(state: &AppState, feature: FeatureId, label: &str) -> Origin {
    let made = state.doc.model().generated_faces(feature);
    made.iter().find(|n| n.ordinal == 0 && n.generator.to_string() == label).expect("the generated name").origin.clone()
}

fn carriers(state: &AppState, origin: &Origin, surface: SurfaceKind) -> Vec<FaceId> {
    let mut found: Vec<FaceId> =
        faces(state).into_iter().filter(|f| f.surface == surface && f.origins.contains(origin)).map(|f| f.id).collect();
    found.sort();
    found
}

/// The faces that show an appearance, from the production resolver. Nothing may be lost.
fn shown(state: &AppState, look: &Appearance) -> Vec<FaceId> {
    let table = state.doc.model().appearance_table(None);
    assert!(table.lost.is_empty(), "{:?}", table.lost);
    let index = table.appearances.iter().position(|a| a.id == look.id).expect("the appearance") as u32;
    let mut found: Vec<FaceId> = table.faces.iter().filter(|(_, i)| **i == index).map(|(face, _)| *face).collect();
    found.sort();
    found
}

#[test]
fn a_blend_reference_taken_on_the_fitted_patches_follows_the_blend_to_the_contact_and_back() {
    let dir = std::env::temp_dir().join(format!("soliddraft-p5-reference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut state = AppState::new(soliddraft_app::new_document());
    let scenario: Value = serde_json::from_str(SCENARIO).unwrap();
    for step in &scenario["steps"].as_array().unwrap()[..4] {
        run(&mut state, step["tool"].as_str().unwrap(), step["arguments"].clone());
    }
    run(&mut state, "edit_fillet", json!({"feature": "Fillet1", "radius": 2.5}));

    // One real reference, from the generated name and an actual torus.
    let fillet = feature(&state, "Fillet1");
    let blend_name = Generator::Blend { edge: 0 };
    let blend = origin(&state, fillet, &blend_name.to_string());
    let patches = carriers(&state, &blend, SurfaceKind::Torus);
    assert_eq!(patches.len(), 4, "the fitted blend's torus patches");
    let torus = faces(&state).into_iter().find(|f| f.id == patches[0]).unwrap();
    let blend_reference = FaceRef {
        feature: fillet,
        generator: blend_name,
        ordinal: 0,
        fingerprint: FaceFingerprint::of(&torus),
        label: "Fillet1 · Blend of edge 1".into(),
    };
    let hole = feature(&state, "Hole1");
    let mouth = origin(&state, hole, "Hole 1 countersink");
    let mouth_face = carriers(&state, &mouth, SurfaceKind::Cone);
    assert_eq!(mouth_face.len(), 1);
    let mouth_reference = state.doc.model().face_ref(mouth_face[0]).expect("the mouth is one named face");

    let blue = Appearance::new("blend-blue", "Blend blue", Srgb::new(30, 60, 200));
    let green = Appearance::new("mouth-green", "Mouth green", Srgb::new(30, 180, 60));
    let blend_target = AppearanceTarget::Face { face: blend_reference };
    let mouth_target = AppearanceTarget::Face { face: mouth_reference };
    state.doc.set_appearance(blend_target.clone(), Some(blue.clone())).unwrap();
    state.doc.set_appearance(mouth_target.clone(), Some(green.clone())).unwrap();
    assert_eq!(shown(&state, &blue), patches, "exactly the four tori, no spline and nothing else");
    assert_eq!(shown(&state, &green), mouth_face);

    // The same stored references, never recreated, against the current geometry.
    let check = |state: &AppState, stage: &str| {
        let stored: Vec<_> = state.doc.model().materials().appearances.iter().map(|a| a.target.clone()).collect();
        assert_eq!(stored, [blend_target.clone(), mouth_target.clone()], "{stage}");
        let blend = origin(state, feature(state, "Fillet1"), "Blend of edge 1");
        let mouth = origin(state, feature(state, "Hole1"), "Hole 1 countersink");
        let tori = carriers(state, &blend, SurfaceKind::Torus);
        assert_eq!(tori.len(), 1, "{stage}: one torus away from the fitted radii");
        assert_eq!(shown(state, &blue), tori, "{stage}");
        assert_eq!(shown(state, &green), carriers(state, &mouth, SurfaceKind::Cone), "{stage}");
    };
    let edit = |state: &mut AppState, radius: f64| {
        run(state, "edit_fillet", json!({"feature": "Fillet1", "radius": radius}));
    };

    edit(&mut state, 1.0);
    check(&state, "radius 1");
    edit(&mut state, 0.5);
    check(&state, "radius 0.5");
    run(&mut state, "undo", json!({}));
    check(&state, "undo to radius 1");
    run(&mut state, "redo", json!({}));
    check(&state, "redo to radius 0.5");
    edit(&mut state, 1.0);
    check(&state, "return to radius 1");

    let path = dir.join("p5-reference.dwprt");
    run(&mut state, "save", json!({"path": path}));
    drop(state);
    let doc = soliddraft_app::file::open(&path, soliddraft_app::kernel(), None).unwrap();
    let mut reopened = AppState::new(doc);
    check(&reopened, "fresh open");
    let refused = call(&mut reopened, "edit_fillet", json!({"feature": "Fillet1", "radius": 3.5}));
    assert!(refused.is_error && refused.content.contains("shapes.rolling-envelope"), "{}", refused.content);
    check(&reopened, "refused radius 3.5");
    std::fs::remove_dir_all(dir).unwrap();
}
