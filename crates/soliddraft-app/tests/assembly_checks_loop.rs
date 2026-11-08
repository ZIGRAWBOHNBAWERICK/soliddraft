//! Replay the B7 application-loop specs through the same AppTools route as MCP.

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};

fn call(state: &mut AppState, name: &str, args: Value) -> (bool, Value) {
    let out = call_checked(&mut AppTools { state }, name, args, &Caller::Mcp { client: "B7-loop".into() });
    (out.is_error, serde_json::from_str(&out.content).unwrap())
}

#[test]
fn interference_clearance_and_hole_alignment_application_loops() {
    for source in [
        include_str!("../../../ci/scenarios/interference-detection.json"),
        include_str!("../../../ci/scenarios/clearance-verification.json"),
        include_str!("../../../ci/scenarios/hole-alignment.json"),
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
                    assert_eq!(actual, expected, "{name} step {} {pointer}", index + 1);
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
