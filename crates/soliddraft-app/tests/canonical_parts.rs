//! The canonical target parts through the application's commands, naming and exchange.

use std::{collections::BTreeSet, path::Path};

use serde_json::Value;
use soliddraft_app::{app_tools::AppTools, document::FeatureStatus, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_exchange::{Format, Report};
use soliddraft_model::{EdgeRef, FaceRef};

#[path = "support/ribbon.rs"]
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
        Self { faces, edges, labels }
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
        assert_eq!(Self::take(state).labels, self.labels, "{stage}: generator names changed");
    }
}

fn volume(state: &AppState) -> f64 {
    let doc = state.doc.model();
    doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume
}

fn exports(state: &AppState, dir: &Path, label: &str) {
    for format in [Format::Step, Format::Stl] {
        let output = state.doc.model().export(format).unwrap();
        match &output.report {
            Report::Step(r) => {
                assert!(r.within_tolerance, "{r:?}");
                assert_eq!(r.solids, 1);
            }
            Report::Mesh(r) => {
                assert!(r.closed && r.triangles > 0, "{r:?}");
                assert!(r.chord_height.is_finite() && r.chord_height > 0.0);
            }
            other => panic!("unexpected report: {other:?}"),
        }
        assert!(!output.bytes.is_empty());
        std::fs::write(dir.join(format!("{label}.{}", format.extension())), &output.bytes).unwrap();
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

#[test]
fn canonical_parts_build_edit_keep_names_reopen_and_export_in_edit_suite() {
    let case = ribbon::case("canonical-parts");
    let pi = std::f64::consts::PI;
    let expected = [
        (19080.0 + 275.0 * pi, 19080.0 + 445.0 * pi),
        (3998.20025046861, 4029.9761511627316),
        (14428.640126047918, 14455.207561827563),
        (tray_area() * 12.0 - 9360.0, tray_area() * 12.0 - 10489.5),
        (12558.681699123754, 12591.90560450861),
    ];
    for (source, (original_volume, edited_volume)) in PARTS.iter().zip(expected) {
        let scenario: Value = serde_json::from_str(source).unwrap();
        let name = scenario["name"].as_str().unwrap();
        let dir = case.path(name);
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = AppState::new(soliddraft_app::new_document());
        let mut names = None;
        for (i, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
            let tool = step["tool"].as_str().unwrap();
            let arguments: Value =
                serde_json::from_str(&step["arguments"].to_string().replace("{bundle}", dir.to_str().unwrap()))
                    .unwrap();
            if step["fresh_process"] == true {
                drop(state);
                state = AppState::new(soliddraft_app::new_document());
            }
            eprintln!("{name} step {} {tool}", i + 1);
            let (out, work) = soliddraft_kernel_catallax::with_request_allowance(20_000_000, || {
                call_checked(
                    &mut AppTools { state: &mut state },
                    tool,
                    arguments,
                    &Caller::Mcp { client: "canonical-parts".into() },
                )
            });
            assert!(!work.exhausted, "{name} step {} exceeded application allowance: {work:?}", i + 1);
            assert!(!out.is_error, "{name} step {} {tool}: {}", i + 1, out.content);
            assert!(state.doc.features.iter().all(|f| f.status == FeatureStatus::Ok));
            let Some(label) =
                step["label"].as_str().filter(|l| ["original", "edited", "undo", "redo", "reopened"].contains(l))
            else {
                continue;
            };
            assert_volume(
                volume(&state),
                if matches!(label, "original" | "undo") { original_volume } else { edited_volume },
                label,
            );
            if label == "original" {
                names = Some(Names::take(&state));
            }
            names.as_ref().unwrap().check(&state, label);
            exports(&state, &dir, label);
            let mut app = ribbon::app(state.doc);
            app.state_mut().state.frame_layout = soliddraft_app::settings::FrameLayout::EditSuite;
            app.key_press(egui::Key::Z);
            app.run_steps(5);
            app.render().unwrap().save(dir.join(format!("{label}.png"))).unwrap();
            state = AppState::new(std::mem::replace(&mut app.state_mut().state.doc, soliddraft_app::new_document()));
        }
    }
}
