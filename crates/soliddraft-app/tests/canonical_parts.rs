//! The canonical target parts through the application's commands, naming and exchange.

use std::{collections::BTreeSet, path::Path, sync::Arc};

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, document::FeatureStatus, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_exchange::{Format, Report};
use soliddraft_model::{EdgeRef, FaceRef};

#[path = "support/ui.rs"]
mod ribbon;

const PARTS: [&str; 5] = [
    include_str!("../../../ci/scenarios/canonical-p1.json"),
    include_str!("../../../ci/scenarios/canonical-p2.json"),
    include_str!("../../../ci/scenarios/canonical-p3.json"),
    include_str!("../../../ci/scenarios/canonical-p4.json"),
    include_str!("../../../ci/scenarios/canonical-p5.json"),
];

struct Names {
    faces: Vec<FaceRef>,
    edges: Vec<EdgeRef>,
    labels: BTreeSet<String>,
    counts: (usize, usize),
}

impl Names {
    fn take(state: &AppState) -> Self {
        let doc = state.doc.model();
        let topology = doc.kernel().topology(doc.body().unwrap()).unwrap();
        let faces: Vec<_> = topology.faces.iter().filter_map(|f| doc.face_ref(f.id).ok()).collect();
        let edges: Vec<_> = topology.edges.iter().filter_map(|e| doc.edge_ref(e.id).ok()).collect();
        assert!(!faces.is_empty() && !edges.is_empty());
        // Split blend patches have one set-valued name; their existing appearance test
        // exercises that resolver. Every displayed face must still have a generator.
        let labels = topology
            .faces
            .iter()
            .map(|f| {
                let named = state.doc.named_face(f.id).expect("every face has a generator");
                assert!(state.doc.feature(named.feature).is_some(), "no anonymous Body face");
                format!("{} · {}", state.doc.feature(named.feature).unwrap().name, named.label)
            })
            .collect();
        Self { faces, edges, labels, counts: (topology.faces.len(), topology.edges.len()) }
    }

    fn check(&self, state: &AppState, stage: &str) {
        let doc = state.doc.model();
        let body = doc.body().unwrap();
        for reference in &self.faces {
            doc.resolve_face(reference, body).unwrap_or_else(|e| panic!("{stage}: {reference:?}: {e:?}"));
        }
        for reference in &self.edges {
            doc.resolve_edge(reference, body).unwrap_or_else(|e| panic!("{stage}: {reference:?}: {e:?}"));
        }
        let current = Self::take(state);
        assert_eq!(current.labels, self.labels, "{stage}: generator names changed");
        assert_eq!(current.counts, self.counts, "{stage}: topology counts changed");
    }
}

fn mass(state: &AppState) -> soliddraft_kernel_api::MassProperties {
    let doc = state.doc.model();
    // Use the desktop mass worker's allowance, separate from construction and export.
    let (result, work) =
        soliddraft_kernel_catallax::with_request_allowance(soliddraft_app::measure::MASS_WORK_LIMIT, || {
            doc.kernel().mass_properties(doc.body().unwrap())
        });
    assert!(!work.exhausted, "mass oracle exhausted the desktop allowance: {work:?}");
    result.unwrap()
}

fn exports(state: &mut AppState, dir: &Path, label: &str, mesh_accuracy_mm: Option<f64>) {
    for format in [Format::Step, Format::Stl] {
        let accuracy = (format == Format::Stl).then_some(mesh_accuracy_mm).flatten();
        let settings = state.doc.model().settings().clone();
        let output = state
            .doc
            .model()
            .export_at(format, accuracy)
            .unwrap_or_else(|error| panic!("{label} {}: {error}", format.name()));
        match &output.report {
            Report::Step(r) => {
                assert!(r.within_tolerance, "{r:?}");
                assert_eq!(r.solids, 1);
            }
            Report::Mesh(r) => {
                assert!(r.closed && r.triangles > 0, "{r:?}");
                let requested = accuracy.unwrap_or(settings.export.chord_height);
                let proof = r.print_check.as_ref().expect("native print certificate");
                assert_eq!(r.chord_height, requested);
                assert_eq!(proof.requested_accuracy_mm, requested);
                assert!(proof.certified_accuracy_mm.is_finite());
                assert!(0.0 <= proof.certified_accuracy_mm && proof.certified_accuracy_mm <= requested);
            }
            other => panic!("unexpected report: {other:?}"),
        }
        assert!(!output.bytes.is_empty());
        let path = dir.join(format!("{label}.{}", format.extension()));
        let tool = if format == Format::Step { "export_step" } else { "export_stl" };
        let mut arguments = json!({"path": path});
        if let Some(mm) = accuracy {
            arguments["accuracy_mm"] = json!(mm);
        }
        let out =
            call_checked(&mut AppTools { state }, tool, arguments, &Caller::Mcp { client: "canonical-exports".into() });
        assert!(!out.is_error, "{tool}: {}", out.content);
        assert_eq!(state.doc.model().settings(), &settings, "export changed saved settings");
        assert_eq!(std::fs::read(&path).unwrap(), output.bytes, "public export differs from the model export");
        if let Report::Step(report) = &output.report {
            let doc = state.doc.model();
            let body = doc.body().unwrap();
            let topology = doc.kernel().topology(body).unwrap();
            let mass = mass(state);
            let evidence = json!({"report":report,"body":{"volume":mass.volume,"area":mass.area,"faces":topology.faces.len(),"edges":topology.edges.len()}});
            std::fs::write(
                dir.join(format!("{label}.step-report.json")),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
        std::fs::write(
            dir.join(format!("{label}.{}.json", format.extension())),
            serde_json::to_vec_pretty(&output.report).unwrap(),
        )
        .unwrap();
    }
}

fn assert_volume(actual: f64, expected: f64, stage: &str) {
    assert!((actual - expected).abs() < 0.005, "{stage}: {actual} != {expected}");
}

fn tray_area() -> f64 {
    let radius: f64 = 23.0;
    let a = (radius * radius - 15.0_f64.powi(2)).sqrt();
    let integral = |x: f64| (x * (radius * radius - x * x).sqrt() + radius * radius * (x / radius).asin()) / 2.0;
    1200.0 - 4.0 * (15.0 * (20.0 - a) - integral(20.0) + integral(a))
}

fn exercise(index: usize) {
    let pi = std::f64::consts::PI;
    let expected = [
        (19080.0 + 275.0 * pi, 19080.0 + 445.0 * pi),
        (3998.20025046861, 4029.9761511627316),
        (14428.640126047918, 14455.207561827563),
        (tray_area() * 12.0 - 9360.0, tray_area() * 12.0 - 10489.5),
        (12558.681699123754, 12591.90560450861),
    ];
    let (original_volume, edited_volume) = expected[index];
    let scenario: Value = serde_json::from_str(PARTS[index]).unwrap();
    let name = scenario["name"].as_str().unwrap();
    let case = ribbon::case(&format!("canonical-parts/{name}"));
    let dir = case.path("");
    std::fs::create_dir_all(&dir).unwrap();
    let mut kernel = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(20_000_000));
    let mut state = AppState::new(soliddraft_app::document::Document::new(name, kernel.clone()));
    let mut names = None;
    let mut mesh_accuracy_mm = None;
    for (i, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
        let tool = step["tool"].as_str().unwrap();
        let mut arguments = step["arguments"].clone();
        if let Some(path) = arguments.get_mut("path") {
            *path = path.as_str().unwrap().replace("{bundle}", dir.to_str().unwrap()).into();
        }
        if step["fresh_process"] == true {
            drop(state);
            kernel = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(20_000_000));
            state = AppState::new(soliddraft_app::document::Document::new(name, kernel.clone()));
        }
        eprintln!("{name} step {} {tool}", i + 1);
        let before = state.doc.model().part_json();
        let revision = state.doc.revision();
        // Match MCP's per-call allowance; the scenario runner checks whole CLI exports.
        kernel.take_diagnostics().unwrap();
        let out = call_checked(
            &mut AppTools { state: &mut state },
            tool,
            arguments,
            &Caller::Mcp { client: "canonical-parts".into() },
        );
        std::fs::write(
            dir.join(format!("{:04}-kernel.json", i + 1)),
            serde_json::to_vec_pretty(&kernel.take_diagnostics().unwrap()).unwrap(),
        )
        .unwrap();
        if let Some(reason) = step["refused"].as_str() {
            assert!(out.is_error && out.content.contains(reason), "{name}: {}", out.content);
            assert_eq!(state.doc.model().part_json(), before, "refused edit changed the part");
            assert_eq!(state.doc.revision(), revision, "refused edit changed the revision");
            std::fs::write(dir.join(format!("{}.txt", step["label"].as_str().unwrap())), &out.content).unwrap();
        } else {
            assert!(!out.is_error, "{name} step {} {tool}: {}", i + 1, out.content);
        }
        assert!(state.doc.features.iter().all(|f| f.status == FeatureStatus::Ok));
        let Some(label) = step["label"]
            .as_str()
            .filter(|l| ["original", "edited", "undo", "redo", "reopened", "refused"].contains(l))
        else {
            continue;
        };
        assert_volume(
            mass(&state).volume,
            if matches!(label, "original" | "undo") { original_volume } else { edited_volume },
            label,
        );
        if label == "original" {
            names = Some(Names::take(&state));
        }
        names.as_ref().unwrap().check(&state, label);
        assert_eq!(state.doc.displayed_revision(), state.doc.revision(), "{label}: stale display");
        assert!(state.doc.triangle_count() > 0, "{label}: empty display");
        assert!(!state.doc.proof_pending(), "{label}: proof is still pending");
        // Repeat the scenario's explicit print choice through the history checks.
        if let Some(export) = step["exports"].as_array().and_then(|exports| {
            exports.iter().find(|export| export["format"] == "stl" && export.get("refused").is_none())
        }) {
            mesh_accuracy_mm = export["accuracy_mm"].as_f64();
        }
        exports(&mut state, &dir, label, mesh_accuracy_mm);
        let mut app = ribbon::app(state.doc);

        app.key_press(egui::Key::Z);
        app.run_steps(5);
        app.render().unwrap().save(dir.join(format!("{label}.png"))).unwrap();
        state = AppState::new(std::mem::replace(&mut app.state_mut().state.doc, soliddraft_app::new_document()));
    }
}

#[test]
fn p1_builds_edits_keeps_names_reopens_and_exports_in_edit_suite() {
    exercise(0);
}

#[test]
fn p2_builds_edits_keeps_names_reopens_and_exports_in_edit_suite() {
    exercise(1);
}

#[test]
fn p3_builds_edits_keeps_names_reopens_and_exports_in_edit_suite() {
    exercise(2);
}

#[test]
fn p4_builds_edits_keeps_names_reopens_and_exports_in_edit_suite() {
    exercise(3);
}

#[test]
fn p5_builds_edits_keeps_names_reopens_and_exports_in_edit_suite() {
    exercise(4);
}
