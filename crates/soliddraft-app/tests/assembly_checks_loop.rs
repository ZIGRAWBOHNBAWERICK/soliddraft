//! Replay the B7 application-loop specs through the same AppTools route as MCP.

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};

fn call(state: &mut AppState, name: &str, args: Value) -> (bool, Value) {
    let out = call_checked(&mut AppTools { state }, name, args, &Caller::Mcp { client: "B7-loop".into() });
    (out.is_error, serde_json::from_str(&out.content).unwrap())
}

fn decimal_clearance() -> Value {
    let mut spec: Value =
        serde_json::from_str(include_str!("../../../ci/scenarios/exact-body-relations.json")).unwrap();
    spec["name"] = json!("exact-decimal-clearance");
    let steps = spec["steps"].as_array_mut().unwrap();
    steps.truncate(11);
    steps[1]["arguments"]["depth"] = json!(0.7);
    steps[10]["arguments"]["placement"]["translation"] = json!([3, 7, 1]);
    // The part ends at exactly 0.7 mm; the second starts at exactly 1 mm.
    for (clearance, below, met) in [("0.3 mm", 1, 0), ("0.299 mm", 0, 1), ("0.301 mm", 1, 0)] {
        steps.push(json!({
            "tool": "assembly_check_clearance", "arguments": {"clearance": clearance},
            "checks": [
                {"pointer":"/result/report/below", "equals": below},
                {"pointer":"/result/report/met", "equals": met},
                {"pointer":"/result/report/undecided", "equals": 0}
            ]
        }));
    }
    spec
}

#[test]
fn interference_clearance_and_hole_alignment_application_loops() {
    for spec in [
        include_str!("../../../ci/scenarios/interference-detection.json"),
        include_str!("../../../ci/scenarios/exact-body-relations.json"),
        include_str!("../../../ci/scenarios/clearance-verification.json"),
        include_str!("../../../ci/scenarios/hole-alignment.json"),
    ]
    .into_iter()
    .map(|source| serde_json::from_str::<Value>(source).unwrap())
    .chain([decimal_clearance()])
    {
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
        if name == "exact-body-relations" {
            let (_, before) = call(&mut state, "model_summary", json!({}));
            let ((error, refused), work) = soliddraft_kernel_catallax::with_request_allowance(0, || {
                call(&mut state, "assembly_check_interference", json!({}))
            });
            assert!(!error, "{refused}");
            assert!(work.exhausted);
            let refusal = &refused["report"]["results"][0]["exact_refused"][0];
            assert_eq!(refusal["code"], "numbers.operation-budget", "{refused}");
            assert!(refusal["message"].as_str().unwrap().contains("calculation limit"));
            let (_, after) = call(&mut state, "model_summary", json!({}));
            assert_eq!(before["assembly"]["accepted_state"], after["assembly"]["accepted_state"]);

            let part: soliddraft_assembly::OccurrencePath = "4".parse().unwrap();
            let at = soliddraft_assembly::Placement::translation(glam::DVec3::new(3.0, 7.0, 11.0));
            state.assembly.session.as_mut().unwrap().doc.move_to(&[(part.clone(), at)]).unwrap();
            let (error, warm) = call(&mut state, "assembly_check_clearance", json!({"clearance":"0.5 mm"}));
            assert!(!error && warm["report"]["met"] == 1, "{warm}");
            let ((error, refined), work) = soliddraft_kernel_catallax::with_request_allowance(0, || {
                call(&mut state, "assembly_check_clearance", json!({"clearance":"1 mm"}))
            });
            assert!(!error && work.exhausted, "{refined}");
            let pair = &refined["report"]["results"][0];
            assert_eq!(pair["outcome"], "clear", "{pair}");
            assert_eq!(pair["clearance"], "undecided", "{pair}");
            assert!(pair["distance"]["lo"].as_f64().unwrap() > 0.0, "{pair}");
            assert!(pair["why"].as_str().unwrap().contains("separated"), "{pair}");
            assert!(pair["exact_refused"].as_array().unwrap().iter().any(|r| r["code"] == "numbers.operation-budget"));

            // A gap below the mesh chord bound is still decided from the exact solids.
            let at = soliddraft_assembly::Placement::translation(glam::DVec3::new(3.0, 7.0, 10.0001));
            state.assembly.session.as_mut().unwrap().doc.move_to(&[(part, at)]).unwrap();
            let (error, met) = call(&mut state, "assembly_check_clearance", json!({"clearance":"0.00005 mm"}));
            assert!(!error, "{met}");
            assert_eq!(met["report"]["met"], 1, "{met}");
            assert_eq!(met["report"]["undecided"], 0, "{met}");
            let (error, below) = call(&mut state, "assembly_check_clearance", json!({"clearance":"0.0002 mm"}));
            assert!(!error, "{below}");
            assert_eq!(below["report"]["below"], 1, "{below}");
            let pair = &below["report"]["results"][0];
            assert_eq!(pair["outcome"], "clear", "{below}");
            assert!(pair["distance"]["lo"].as_f64().unwrap() > 0.00005, "{pair}");
            assert!(pair["distance"]["hi"].as_f64().unwrap() < 0.0002, "{pair}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn p3_copies_report_proved_overlap_and_unresolved_tangency() {
    let dir = std::env::temp_dir().join(format!("soliddraft-p3-interference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut state = AppState::new(soliddraft_app::new_document());
    let scenario: Value = serde_json::from_str(include_str!("../../../ci/scenarios/p3-rounded-boss.json")).unwrap();
    for step in scenario["steps"].as_array().unwrap().iter().take(8) {
        let tool = step["tool"].as_str().unwrap();
        let (error, result) = call(&mut state, tool, step["arguments"].clone());
        assert!(!error, "{tool}: {result}");
    }
    let part_file = dir.join("p3.dwprt");
    for (tool, arguments) in [
        ("save", json!({"path": part_file})),
        ("make_assembly", json!({})),
        ("assembly_save", json!({"path": dir.join("rig.dwasm")})),
        (
            "insert_component",
            json!({
                "paths": [part_file],
                "placement": {"translation": [20, 0, 0], "rotation": [0, 0, 0, 1]}
            }),
        ),
    ] {
        let (error, result) = call(&mut state, tool, arguments);
        assert!(!error, "{tool}: {result}");
    }
    let (error, result) = call(&mut state, "assembly_check_interference", json!({}));
    assert!(!error, "{result}");
    let pair = &result["report"]["results"][0];
    assert_eq!(pair["outcome"], "interferes", "{pair}");
    let overlap = &pair["overlap_mm3"];
    let lo = overlap["lo"].as_f64().unwrap();
    assert!(lo > 0.0 && overlap["hi"].as_f64().unwrap() > 2.0 * lo, "{pair}");

    let assembly = &mut state.assembly.session.as_mut().unwrap().doc;
    let second = assembly.assembly().components.last().unwrap().id;
    let path = soliddraft_assembly::OccurrencePath::root().child(soliddraft_assembly::Step::component(second));
    // The same pair touching end to end: the Boolean refuses the fillets' torus pairs and
    // nothing settles the pair, which must stay open and never read as clear.
    let tangent = soliddraft_assembly::Placement::translation(glam::DVec3::new(40.0, 0.0, 0.0));
    assembly.move_to(&[(path, tangent)]).unwrap();
    let (error, result) = call(&mut state, "assembly_check_interference", json!({}));
    assert!(!error, "{result}");
    assert_eq!(result["report"]["clear"], 0, "{result}");
    let pair = &result["report"]["results"][0];
    assert_eq!(pair["outcome"], "touching_or_undecided", "{pair}");
    assert!(pair["exact_refused"].as_array().unwrap().iter().any(|r| r["query"] == "Overlap volume"), "{pair}");
    std::fs::remove_dir_all(dir).unwrap();
}
