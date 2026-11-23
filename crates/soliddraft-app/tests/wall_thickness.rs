//! Graded-wall refusals, saved grades and the constant wall edit chain.

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Document, FeatureStatus};
use soliddraft_app::state::AppState;
use soliddraft_assistant::{Caller, tools::call_checked};

fn call(state: &mut AppState, name: &str, args: Value) -> Result<(), String> {
    let output = call_checked(&mut AppTools { state }, name, args, &Caller::Mcp { client: "wall-thickness".into() });
    if output.is_error { Err(output.content) } else { Ok(()) }
}

fn volume(state: &AppState) -> f64 {
    state.doc.kernel().mass_properties(state.doc.display_body().unwrap()).unwrap().volume
}

fn assert_volume(state: &AppState, expected: f64) {
    let actual = volume(state);
    assert!((actual - expected).abs() < 1e-6, "{actual} mm³, expected {expected} mm³");
}

#[test]
fn pinned_kernel_refuses_graded_walls_and_keeps_the_constant_feature_editable() {
    let grade = json!({"linear":{"axis":"x", "from":-5, "to":5, "start":1, "end":3}});
    for (verb, selection, name, volumes, mirrored_centroid_z) in [
        (
            "shell",
            json!({"open_faces":["Boss-Extrude1 · End cap"]}),
            "Shell1",
            [384.0, 322.5],
            -(480.0 * 3.0 - 157.5 * 3.75) / 322.5,
        ),
        ("thicken", json!({"faces":["Boss-Extrude1 · End cap"], "side":"front"}), "Thicken1", [160.0, 120.0], -6.75),
    ] {
        let mut state = AppState::new(Document::new(name, soliddraft_app::kernel()));
        call(
            &mut state,
            "sketch",
            json!({"plane":"top", "shapes":[{"shape":"rectangle", "center":[0,0,0], "width":10, "height":8}]}),
        )
        .unwrap();
        call(&mut state, "extrude", json!({"sketch":"Sketch1", "depth":6, "flip":false})).unwrap();
        let body = state.doc.display_body();
        let history = state.doc.undo_depth();
        let mut create = selection;
        create["thickness"] = grade.clone();
        let reason = call(&mut state, verb, create.clone()).unwrap_err();
        assert!(reason.contains("Field-driven thickness is unavailable"), "{verb}: {reason}");
        assert_eq!(state.doc.display_body(), body);
        assert_eq!(state.doc.undo_depth(), history);
        assert_eq!(state.doc.features.len(), 2);
        let mut invalid = create.clone();
        invalid["thickness"]["linear"]["to"] = json!(-5);
        assert!(call(&mut state, verb, invalid).unwrap_err().contains("distinct grade anchors"));
        let mut invalid = create.clone();
        invalid["thickness"]["linear"]["end"] = json!(0);
        let reason = call(&mut state, verb, invalid).unwrap_err();
        assert!(reason.contains("positive") || reason.contains("greater than 0"), "{verb}: {reason}");
        assert_eq!(state.doc.display_body(), body);
        assert_eq!(state.doc.undo_depth(), history);

        create["thickness"] = json!(2);
        call(&mut state, verb, create).unwrap();
        assert_volume(&state, volumes[0]);

        let edit = format!("edit_{verb}");
        let mut changes = json!({"feature":name, "thickness":grade});
        if verb == "thicken" {
            changes["side"] = json!("front");
        }
        let history = state.doc.undo_depth();
        let reason = call(&mut state, &edit, changes.clone()).unwrap_err();
        assert!(reason.contains("Field-driven thickness is unavailable"), "{edit}: {reason}");
        assert_eq!(state.doc.undo_depth(), history);
        assert_volume(&state, volumes[0]);

        changes["thickness"] = json!(1.5);
        call(&mut state, &edit, changes).unwrap();
        assert_volume(&state, volumes[1]);

        let path = std::env::temp_dir().join(format!("soliddraft-wall-{verb}-{}.dwprt", std::process::id()));
        call(&mut state, "save", json!({"path":path})).unwrap();
        call(&mut state, "open", json!({"path":path})).unwrap();
        assert_volume(&state, volumes[1]);

        let before = state.doc.kernel().topology(state.doc.display_body().unwrap()).unwrap().bbox;
        call(&mut state, "mirror", json!({"plane":"top", "keep_original":false})).unwrap();
        let kernel = state.doc.kernel();
        let reflected = state.doc.display_body().unwrap();
        let bounds = kernel.topology(reflected).unwrap().bbox;
        let mass = kernel.mass_properties(reflected).unwrap();
        assert!(
            (bounds.min.z + before.max.z).abs() < 1e-6 && (bounds.max.z + before.min.z).abs() < 1e-6,
            "{verb} after Mirror has {bounds:?}, before had {before:?}",
        );
        assert!(
            (mass.centroid.z - mirrored_centroid_z).abs() < 1e-6,
            "{verb} after Mirror has centroid {:?}, expected z = {mirrored_centroid_z}",
            mass.centroid,
        );
        assert_volume(&state, volumes[1]);
        assert_eq!(state.doc.features.last().unwrap().status, FeatureStatus::Ok);
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn saved_walls_keep_the_grade_or_legacy_number() {
    let constant = json!(2.0);
    let grade = json!({"linear":{"axis":"x", "from":-5.0, "to":5.0, "start":1.0, "end":3.0}});
    for thickness in [constant, grade] {
        let shell = json!({"open_faces":[], "thickness":thickness});
        let thicken = json!({"faces":[], "thickness":thickness, "side":"back"});
        let saved_shell = serde_json::from_value::<soliddraft_model::ShellFeature>(shell.clone()).unwrap();
        let saved_thicken = serde_json::from_value::<soliddraft_model::ThickenFeature>(thicken.clone()).unwrap();
        assert_eq!(serde_json::to_value(saved_shell).unwrap(), shell);
        assert_eq!(serde_json::to_value(saved_thicken).unwrap(), thicken);
    }
}

#[test]
fn a_saved_graded_wall_reopens_with_its_grade_and_a_named_refusal() {
    let kernel = soliddraft_app::kernel();
    let part = soliddraft_model::Document::from_part_json(
        include_str!("../../../ci/fixtures/graded-wall.json"),
        kernel.clone(),
    )
    .unwrap();
    let bytes = part.to_bytes().unwrap();
    let reopened = Document::from_model(soliddraft_model::Document::from_bytes(&bytes, kernel).unwrap());
    let error = &reopened.features.last().unwrap().status;
    assert!(
        matches!(error, FeatureStatus::Error(message) if message.contains("Field-driven thickness is unavailable")),
        "{error:?}"
    );
    let saved: Value = serde_json::from_str(&reopened.model().part_json()).unwrap();
    let fixture: Value = serde_json::from_str(include_str!("../../../ci/fixtures/graded-wall.json")).unwrap();
    let thickness = |part: &Value| {
        serde_json::from_value::<soliddraft_kernel_api::Grade>(part["features"][1]["kind"]["thickness"].clone())
            .unwrap()
    };
    assert_eq!(thickness(&saved), thickness(&fixture));
    assert_volume(&AppState::new(reopened), 480.0);
}
