//! An MCP client driving the headless command line over stdio, as Claude Code does
//! with `claude mcp add soliddraft -- soliddraft mcp`.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

struct Client {
    child: std::process::Child,
    out: BufReader<std::process::ChildStdout>,
    next: u64,
    config: std::path::PathBuf,
}

impl Client {
    fn start(args: &[&str]) -> Self {
        let config =
            std::env::temp_dir().join(format!("soliddraft-mcp-config-{}", soliddraft_document::DocumentId::new()));
        let mut child = Command::new(env!("CARGO_BIN_EXE_soliddraft"))
            .args(args)
            .env("SOLIDDRAFT_CONFIG_DIR", &config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start soliddraft mcp");
        let out = BufReader::new(child.stdout.take().unwrap());
        Self { child, out, next: 0, config }
    }

    fn send(&mut self, msg: &Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": self.next, "method": method, "params": params }));
        let mut line = String::new();
        self.out.read_line(&mut line).unwrap();
        // Every line on stdout must be protocol: anything else breaks the client.
        let reply: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON-RPC: {line:?}: {e}"));
        assert_eq!(reply["id"], self.next, "{reply}");
        reply
    }

    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let r = self.request("tools/call", json!({ "name": name, "arguments": arguments }));
        r["result"].clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.config);
    }
}

#[test]
fn mcp_over_stdio_builds_measures_and_undoes() {
    let mut c = Client::start(&["mcp"]);
    let init = c.request(
        "initialize",
        json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "stdio-test", "version": "1" } }),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    c.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));

    let tools = c.request("tools/list", json!({}));
    let names: Vec<&str> =
        tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    let registry: Vec<String> = soliddraft_app::command_tools::tool_specs(soliddraft_app::command_tools::Surface::Mcp)
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, registry, "the MCP tool list is the command registry's");
    assert!(names.contains(&"extrude"));

    let sketch = c.call(
        "sketch",
        json!({ "plane": "top", "shapes": [{ "shape": "rectangle", "center": [0, 0, 0], "width": 40, "height": 30 }] }),
    );
    assert_eq!(sketch["isError"], false, "{sketch}");
    let made = c.call("extrude", json!({ "sketch": "Sketch1", "depth": 10, "flip": false }));
    assert_eq!(made["isError"], false, "{made}");

    let m = c.call("measure", json!({ "a": "Boss-Extrude1 · Start cap", "b": "Boss-Extrude1 · End cap" }));
    assert_eq!(m["structuredContent"]["gap_mm"], 10.0, "{m}");

    // A proposal that names a missing feature is rejected before anything else happens.
    let bad =
        c.call("propose", json!({ "summary": "x", "commands": [{ "op": "delete_feature", "feature": "Fillet1" }] }));
    assert_eq!(bad["isError"], true);

    for _ in 0..2 {
        let undone = c.call("undo", json!({}));
        assert_eq!(undone["isError"], false, "{undone}");
    }
    let summary = c.call("model_summary", json!({}));
    assert_eq!(summary["structuredContent"]["features"], json!([]));
}

#[test]
fn mcp_bundle_tags_proofs_with_the_request_and_revision_they_answer() {
    let dir = std::env::temp_dir().join(format!("soliddraft-mcp-revisions-{}", std::process::id()));
    let mut c = Client::start(&["mcp", "--bundle", dir.to_str().unwrap()]);
    c.request(
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"evidence-test","version":"1"}}),
    );
    let sketch = c.call(
        "sketch",
        json!({"plane":"top","shapes":[{"shape":"rectangle","center":[0,0,0],"width":40,"height":30}]}),
    );
    assert_eq!(sketch["isError"], false, "{sketch}");
    let extruded = c.call("extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    assert_eq!(extruded["isError"], false, "{extruded}");
    let measured = c.call("scenario_measure", json!({}));
    assert_eq!(measured["isError"], false, "{measured}");
    drop(c);
    let read =
        |path: &str| -> Value { serde_json::from_str(&std::fs::read_to_string(dir.join(path)).unwrap()).unwrap() };
    let (first, second, measurement) =
        (read("0001/command.json"), read("0002/command.json"), read("0002/measurement.json"));
    assert_eq!((first["request"].as_u64(), second["request"].as_u64()), (Some(1), Some(2)));
    assert!(second["revision"].as_u64().unwrap() > first["revision"].as_u64().unwrap(), "{first} {second}");
    assert_eq!(measurement["request"], second["request"]);
    assert_eq!(measurement["revision"], second["revision"]);
}

#[test]
fn mcp_bundle_keeps_the_part_and_question_after_a_refused_extrusion() {
    let dir = std::env::temp_dir().join(format!("soliddraft-mcp-evidence-{}", std::process::id()));
    let mut c = Client::start(&["mcp", "--bundle", dir.to_str().unwrap(), "--max-work", "0"]);
    c.request(
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"evidence-test","version":"1"}}),
    );
    let sketch = c.call(
        "sketch",
        json!({"plane":"top","shapes":[
            {"shape":"rectangle","center":[0,0,0],"width":40,"height":30}
        ]}),
    );
    assert_eq!(sketch["isError"], false, "{sketch}");
    let refused = c.call("extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    assert_eq!(refused["isError"], true, "{refused}");
    drop(c);
    let read = |path: &str| std::fs::read_to_string(dir.join(path)).unwrap();
    assert_eq!(read("0001/part.json"), read("0002/part.json"));
    let command: Value = serde_json::from_str(&read("0002/command.json")).unwrap();
    assert_eq!(command["backend"]["name"], "catallax");
    assert!(command["output"].as_str().unwrap().contains("numbers.operation-budget"));
    let question: Value = serde_json::from_str(&read("0002/question-0000.json")).unwrap();
    assert_eq!(question["format"], "catallax-question/1");
    assert_eq!(question["max_work"], 0);
    assert_eq!(question["source"], "recipe");
    let kernel: Value = serde_json::from_str(&read("0002/kernel.json")).unwrap();
    assert!(kernel["calls"].as_array().unwrap().iter().any(|call| {
        call["operation_log"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["refusal"]["code"] == "numbers.operation-budget")
    }));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn snap_scenario_builds_and_reopens_through_the_application_protocol() {
    replay_sketch_volume_scenario(include_str!("../../../ci/scenarios/sketch-snaps.json"));
}

#[test]
fn relation_scenario_builds_and_reopens_through_the_application_protocol() {
    replay_sketch_volume_scenario(include_str!("../../../ci/scenarios/sketch-relations.json"));
}

fn replay_sketch_volume_scenario(source: &str) {
    let mut c = Client::start(&["mcp"]);
    c.request(
        "initialize",
        json!({"protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"sketch-volume-test","version":"1"}}),
    );
    let source = source.replace("{bundle}", c.config.to_str().unwrap());
    let scenario: Value = serde_json::from_str(&source).unwrap();
    for step in scenario["steps"].as_array().unwrap() {
        let name = step["tool"].as_str().unwrap();
        let reply = c.call(name, step["arguments"].clone());
        assert_eq!(reply["isError"], false, "{name}: {reply}");
        if step.get("checks").is_some() {
            let measured = c.call("mass_properties", json!({"name":"Boss-Extrude1"}));
            assert_eq!(measured["isError"], false, "{measured}");
            let volume = measured["structuredContent"]["volume_mm3"].as_f64().unwrap();
            for check in step["checks"].as_array().unwrap() {
                assert_eq!(check["pointer"], "/measurement/volume_mm3");
                let expected = check["approx"].as_f64().unwrap();
                let tolerance = check["tolerance"].as_f64().unwrap();
                assert!((volume - expected).abs() < tolerance, "{volume} != {expected}");
            }
        }
    }
}

#[test]
fn ellipse_scenario_profiles_extrude_and_measure_over_stdio() {
    for source in [
        include_str!("../../../ci/scenarios/sketch-ellipse.json"),
        include_str!("../../../ci/scenarios/sketch-partial-ellipse.json"),
    ] {
        let scenario: Value = serde_json::from_str(source).unwrap();
        let mut client = Client::start(&["mcp"]);
        client.request(
            "initialize",
            json!({
                "protocolVersion":"2025-06-18", "capabilities":{},
                "clientInfo":{"name":"ellipse-scenario-test","version":"1"}
            }),
        );
        client.send(&json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        for step in scenario["steps"].as_array().unwrap() {
            let result = client.call(step["tool"].as_str().unwrap(), step["arguments"].clone());
            assert_eq!(result["isError"], false, "{}: {result}", scenario["name"]);
            if let Some(checks) = step["checks"].as_array() {
                let measured = client.call("mass_properties", json!({"name":"Boss-Extrude1"}));
                assert_eq!(measured["isError"], false, "{measured}");
                let actual = measured["structuredContent"]["volume_mm3"].as_f64().unwrap();
                let expected = checks[0]["approx"].as_f64().unwrap();
                let tolerance = checks[0]["tolerance"].as_f64().unwrap();
                assert!((actual - expected).abs() <= tolerance, "{}: {actual} != {expected}", scenario["name"]);
            }
        }
    }
}
