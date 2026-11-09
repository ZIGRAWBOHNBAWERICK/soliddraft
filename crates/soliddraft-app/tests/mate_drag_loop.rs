//! Replay the B6 application-loop specs through the same AppTools route as MCP.

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};

fn call(state: &mut AppState, name: &str, args: Value) -> (bool, Value) {
    let out = call_checked(&mut AppTools { state }, name, args, &Caller::Mcp { client: "B6-loop".into() });
    let result = serde_json::from_str(&out.content)
        .unwrap_or_else(|error| panic!("{name} returned invalid JSON ({error}): {}", out.content));
    (out.is_error, result)
}

#[test]
fn drag_and_hinge_limit_application_loops() {
    for source in [
        include_str!("../../../ci/scenarios/mate-bounded.json"),
        include_str!("../../../ci/scenarios/mate-drag.json"),
        include_str!("../../../ci/scenarios/mate-gear-motion.json"),
        include_str!("../../../ci/scenarios/mate-rack-motion.json"),
        include_str!("../../../ci/scenarios/mate-screw-motion.json"),
        include_str!("../../../ci/scenarios/mate-linear-motion.json"),
        include_str!("../../../ci/scenarios/mate-drag-hinge.json"),
    ] {
        let spec: Value = serde_json::from_str(source).unwrap();
        let name = spec["name"].as_str().unwrap();
        let dir = std::env::temp_dir().join(format!("soliddraft-{name}-loop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("work")).unwrap();
        soliddraft_app::file::set_config_dir(Some(dir.join("config")));
        let mut state = AppState::new(soliddraft_app::new_document());
        let mut history: Vec<Value> = Vec::new();
        for (index, step) in spec["steps"].as_array().unwrap().iter().enumerate() {
            if step["fresh_process"] == true {
                state = AppState::new(soliddraft_app::new_document());
            }
            let tool = step["tool"].as_str().unwrap();
            let args = serde_json::to_string(&step["arguments"]).unwrap().replace("{bundle}", dir.to_str().unwrap());
            let (error, result) = call(&mut state, tool, serde_json::from_str(&args).unwrap());
            assert_eq!(error, step.get("refused").is_some(), "{name} step {} {tool}: {result}", index + 1);
            let (error, summary) = call(&mut state, "model_summary", json!({}));
            assert!(!error, "{summary}");
            let output = json!({ "result": result, "summary": summary });
            for check in step["checks"].as_array().unwrap() {
                let pointer = check["pointer"].as_str().unwrap();
                let actual = output
                    .pointer(pointer)
                    .unwrap_or_else(|| panic!("{name} step {}: missing {pointer}: {output}", index + 1));
                if let Some(expected) = check.get("equals") {
                    assert!(
                        actual == expected
                            || (actual.is_number() && expected.is_number() && actual.as_f64() == expected.as_f64()),
                        "{name} step {} {pointer}: {actual} != {expected}",
                        index + 1
                    );
                } else if let Some(expected) = check.get("approx") {
                    let delta = (actual.as_f64().unwrap() - expected.as_f64().unwrap()).abs();
                    assert!(
                        delta <= check["tolerance"].as_f64().unwrap(),
                        "{name} step {} {pointer}: {actual}",
                        index + 1
                    );
                } else if let Some(previous) = check.get("same_as") {
                    let prior = &history[previous["step"].as_u64().unwrap() as usize - 1];
                    assert_eq!(
                        Some(actual),
                        prior.pointer(previous["pointer"].as_str().unwrap()),
                        "{name} step {} {pointer}",
                        index + 1
                    );
                } else {
                    panic!("unhandled check: {check}");
                }
            }
            history.push(output);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn poll_collision(state: &mut AppState) -> Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let (error, result) = call(state, "assembly_drag", json!({"phase":"poll"}));
        assert!(!error, "{result}");
        if result["pending"] == false {
            return result;
        }
        assert!(std::time::Instant::now() < deadline, "{result}");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn collision_and_dynamic_clearance_application_loops() {
    let shader = naga::front::wgsl::parse_str(include_str!("../src/viewport/assembly.wgsl")).unwrap();
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&shader)
        .unwrap();
    let spec: Value = serde_json::from_str(include_str!("../../../ci/scenarios/clearance-verification.json")).unwrap();
    for (name, guard, stop) in
        [("collision", json!({"collision":true}), 10.02), ("dynamic-clearance", json!({"clearance":"2 mm"}), 12.02)]
    {
        let dir = std::env::temp_dir().join(format!("soliddraft-b8-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("work")).unwrap();
        soliddraft_app::file::set_config_dir(Some(dir.join("config")));
        let mut state = AppState::new(soliddraft_app::new_document());
        for step in &spec["steps"].as_array().unwrap()[..12] {
            let args = serde_json::to_string(&step["arguments"]).unwrap().replace("{bundle}", dir.to_str().unwrap());
            let (error, result) =
                call(&mut state, step["tool"].as_str().unwrap(), serde_json::from_str(&args).unwrap());
            assert!(!error, "{result}");
        }
        let before = state.assembly.session.as_ref().unwrap().doc.assembly().clone();
        let depth = state.assembly.session.as_ref().unwrap().doc.undo_depth();
        let mut begin = guard;
        begin["phase"] = json!("begin");
        begin["path"] = json!("4");
        assert!(!call(&mut state, "assembly_drag", begin).0);
        let (error, _) = call(
            &mut state,
            "assembly_drag",
            json!({"phase":"update","placement":{"translation":[3,7,-30],"rotation":[0,0,0,1]}}),
        );
        assert!(!error);
        let result = poll_collision(&mut state);
        assert_eq!(result["motion"]["tier"], "certified_mesh");
        assert_eq!(result["motion"]["stopped"], true);
        assert!(result["motion"]["error"].is_null(), "{result}");
        let mut dock = egui_dock::DockState::new(vec![soliddraft_app::app::Tab::Assembly]);
        soliddraft_app::assembly::frame(&mut state, &mut dock, &egui::Context::default());
        let session = state.assembly.session.as_ref().unwrap();
        assert_eq!(
            session.scene.lock().unwrap().instances().iter().filter(|i| i.flags >> 4 != 0).count(),
            2,
            "both nearest faces must be highlighted"
        );
        assert_eq!(session.doc.assembly(), &before);
        assert_eq!(session.doc.undo_depth(), depth);
        let row = session.doc.table().find(&"4".parse().unwrap()).unwrap();
        let z = session.doc.table().world(row).translation.z;
        assert!((z - stop).abs() < 0.002, "{name}: z={z}, {result}");
        assert!(!call(&mut state, "assembly_save", json!({"path":dir.join("work/preview.dwasm")})).0);
        assert_eq!(state.assembly.session.as_ref().unwrap().doc.assembly(), &before);
        assert!(!call(&mut state, "assembly_drag", json!({"phase":"commit"})).0);
        poll_collision(&mut state);
        let doc = &mut state.assembly.session.as_mut().unwrap().doc;
        assert_eq!(doc.undo_depth(), depth + 1);
        assert!(doc.undo());
        assert_eq!(doc.assembly(), &before);
        assert!(doc.redo());
        assert!(!call(&mut state, "assembly_save", json!({"path":dir.join("work/committed.dwasm")})).0);
        let (error, result) = call(&mut state, "open", json!({"path":dir.join("work/committed.dwasm")}));
        assert!(!error, "{result}");
        let doc = &state.assembly.session.as_ref().unwrap().doc;
        assert_eq!(doc.assembly().component(soliddraft_assembly::ComponentId(4)).unwrap().placement.translation.z, z);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
