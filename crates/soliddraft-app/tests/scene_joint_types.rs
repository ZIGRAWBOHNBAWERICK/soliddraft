//! Author new kinds of scene joints through the same tools as the scenario runner.

use std::path::Path;

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};

fn call(state: &mut AppState, name: &str, args: Value) -> Value {
    let caller = Caller::Mcp { client: "scene joint types".into() };
    let out = call_checked(&mut AppTools { state }, name, args, &caller);
    assert!(!out.is_error, "{name}: {}", out.content);
    serde_json::from_str(&out.content).unwrap()
}

fn part(state: &mut AppState, path: &Path, shape: Value) {
    call(state, "new_part", json!({}));
    call(state, "sketch", json!({ "plane": "top", "shapes": [shape] }));
    call(state, "extrude", json!({ "sketch": "Sketch1", "depth": 10, "flip": false }));
    call(state, "save", json!({ "path": path }));
}

fn place(state: &mut AppState, path: &Path, at: [f64; 3]) -> u64 {
    call(state, "insert_component", json!({ "paths": [path], "placement": { "translation": at } }))["inserted"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

#[test]
fn authored_slider_plane_and_ball_round_trip_through_both_scene_importers() {
    let dir = std::env::temp_dir().join(format!("sd-joint-types-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup<'a>(&'a Path);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0);
        }
    }
    let _cleanup = Cleanup(&dir);
    let (block, shaft, rig) = (dir.join("block.dwprt"), dir.join("shaft.dwprt"), dir.join("rig.dwasm"));
    let mut state = AppState::new(soliddraft_app::new_document());
    part(&mut state, &block, json!({ "shape": "rectangle", "center": [0, 0, 0], "width": 20, "height": 10 }));
    part(&mut state, &shaft, json!({ "shape": "circle", "center": [0, 0, 0], "diameter": 10 }));
    call(&mut state, "make_assembly", json!({}));
    let root = 2;
    call(&mut state, "assembly_mate_origin", json!({ "name": "Ground", "component": root, "align_axes": true }));

    let slider = place(&mut state, &shaft, [0.0, 0.0, 25.0]);
    let cylinder = |component| json!({ "component": component, "feature": "Boss-Extrude1", "generator": "side" });
    let slide = call(
        &mut state,
        "assembly_mate_concentric",
        json!({ "name": "Slide", "a": cylinder(root), "b": cylinder(slider), "lock_rotation": true }),
    );
    assert_eq!(slide["assembly"]["solver"]["remaining_dof"], 1);
    let platen = place(&mut state, &block, [0.0, 15.0, 0.0]);
    let right = |component| json!({ "component": component, "plane": "Right plane" });
    let plane = call(
        &mut state,
        "assembly_mate_coincident",
        json!({ "name": "Plane", "a": right(root), "b": right(platen), "alignment": "aligned" }),
    );
    assert_eq!(plane["assembly"]["solver"]["remaining_dof"], 4);
    call(&mut state, "assembly_save", json!({ "path": rig }));

    // The same assembly reopened in a fresh session takes a point-to-point mate.
    let mut state = AppState::new(soliddraft_app::new_document());
    call(&mut state, "open", json!({ "path": rig }));
    let ball = place(&mut state, &block, [20.0, 2.0, 0.0]);
    let corner = |component| {
        json!({
            "component": component,
            "vertex": ["Boss-Extrude1 · End cap", "Boss-Extrude1 · Side of line 2", "Boss-Extrude1 · Side of line 3"]
        })
    };
    let edited = call(
        &mut state,
        "assembly_mate_coincident",
        json!({ "name": "Ball", "a": corner(platen), "b": corner(ball), "alignment": "aligned" }),
    );
    assert_eq!(edited["assembly"]["solver"]["remaining_dof"], 7);
    let accepted = &edited["assembly"]["accepted_state"];

    for (tool, extension) in [("assembly_export_urdf", "urdf"), ("assembly_export_usd", "usda")] {
        let file = dir.join(format!("rig.{extension}"));
        let exported = call(&mut state, tool, json!({ "path": file }));
        assert_eq!(&exported["assembly"]["accepted_state"], accepted);
        let joints = exported["exported"]["joints"].as_array().unwrap();
        let exported_joint = |name: &str| {
            joints
                .iter()
                .find(|joint| joint["mates"].as_array().unwrap().contains(&json!(name)))
                .unwrap_or_else(|| panic!("{name}: {joints:?}"))
        };

        let imported_path = dir.join(format!("imported-{extension}.dwasm"));
        let mut reopened = AppState::new(soliddraft_app::new_document());
        let imported = call(&mut reopened, "import_scene", json!({ "path": file, "out": imported_path }));
        let record = &imported["import"];
        assert_eq!(record["nodes"].as_array().unwrap().iter().filter(|n| !n["body"].is_null()).count(), 4);
        assert_eq!(record["bounds"]["max"][2], 35.0, "{record}");
        let imported_joints = record["joints"].as_array().unwrap();
        for (name, kind) in [("Slide", "prismatic"), ("Plane", "planar"), ("Ball", "spherical")] {
            let exported = exported_joint(name);
            assert_eq!(exported["kind"], kind, "{exported}");
            let read = imported_joints.iter().find(|joint| joint["name"] == exported["name"]).unwrap();
            assert_eq!(read["kind"], kind, "{read}");
            assert_eq!(read["limits"], Value::Null, "{read}");
            if name == "Ball" {
                // The planar link's X is the plane normal and the selected corner is
                // 10 mm above the block's origin.
                assert_eq!(read["origin"][2], 10.0, "{read}");
            } else {
                let normal = if name == "Slide" { json!([0.0, 0.0, 1.0]) } else { json!([1.0, 0.0, 0.0]) };
                assert_eq!(read["axis"], normal, "{read}");
                assert_eq!(exported["axis"], normal, "{exported}");
            }
        }

        call(&mut reopened, "assembly_save", json!({ "path": imported_path }));
        let mut fresh = AppState::new(soliddraft_app::new_document());
        let saved = call(&mut fresh, "open", json!({ "path": imported_path }));
        let names = saved["assembly"]["import"]["joints"].as_array().unwrap();
        assert_eq!(names.len(), 4);
        assert!(names.contains(&exported_joint("Ball")["name"]));
    }
}
