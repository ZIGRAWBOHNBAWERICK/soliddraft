//! The closed ring where a boss crosses a pipe rounds into one band through the commands a
//! user has, measures, and exports as STEP under its certificate. What the band cannot
//! answer yet comes back as the ordinary refusal, and the part stays as it was.

use serde_json::json;
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Document, FeatureStatus};
use soliddraft_app::state::AppState;
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_exchange::{Format, Report};
use soliddraft_kernel_api::{CurveKind, SurfaceKind};

fn accepted(state: &mut AppState, tool: &str, arguments: serde_json::Value) {
    let out = call_checked(&mut AppTools { state }, tool, arguments, &Caller::Mcp { client: "ring-fillet".into() });
    assert!(!out.is_error, "{tool}: {}", out.content);
}

/// A pipe of radius 10 along z, with a boss of radius 4 leaving it along y.
fn tee() -> AppState {
    let mut state = AppState::new(Document::new("Tee", soliddraft_app::kernel()));
    accepted(&mut state, "sketch", json!({"plane":"top","shapes":[{"shape":"circle","center":[0,0,0],"diameter":20}]}));
    accepted(&mut state, "extrude", json!({"sketch":"Sketch1","depth":40,"flip":false}));
    accepted(
        &mut state,
        "sketch",
        json!({"plane":"front","shapes":[{"shape":"circle","center":[0,0,20],"diameter":8}]}),
    );
    accepted(&mut state, "extrude", json!({"sketch":"Sketch2","depth":20,"flip":false}));
    state
}

fn ring(state: &AppState) -> String {
    let doc = &state.doc;
    let topology = doc.kernel().topology(doc.display_body().unwrap()).unwrap();
    let edge = topology.edges.iter().find(|e| e.curve == CurveKind::Other).expect("the ring between the cylinders");
    let named = doc.named_edge(edge.id).expect("the ring has a name");
    format!("{} · {}", doc.feature(named.feature).unwrap().name, named.label)
}

fn bands(state: &AppState) -> usize {
    let doc = &state.doc;
    let topology = doc.kernel().topology(doc.display_body().unwrap()).unwrap();
    topology.faces.iter().filter(|f| f.surface == SurfaceKind::Spline).count()
}

#[test]
fn a_ring_rounds_exports_edits_undoes_and_reopens_and_its_refusals_are_the_ordinary_ones() {
    let mut state = tee();
    let plain = state.doc.kernel().mass_properties(state.doc.display_body().unwrap()).unwrap().volume;
    let edges = json!([ring(&state)]);
    accepted(&mut state, "fillet", json!({"edges":edges,"radius":1}));
    let doc = &state.doc;
    assert_eq!(doc.features.last().unwrap().status, FeatureStatus::Ok);
    assert_eq!(bands(&state), 1);
    let body = doc.display_body().unwrap();
    assert!(!doc.kernel().tessellate(body, &Default::default()).unwrap().indices.is_empty());

    let mass = doc.kernel().mass_properties(body).unwrap();
    assert!(mass.volume > plain && mass.volume < plain + 10.0, "{mass:?}");
    assert!(mass.inertia.is_none(), "a fitted band has no second moments");
    let step = doc.model().export(Format::Step).unwrap();
    let Report::Step(report) = &step.report else { panic!("a STEP report") };
    eprintln!("{report:?}");
    assert!(report.within_tolerance, "{report:?}");
    assert_eq!((report.solids, report.faces), (1, 6), "{report:?}");
    assert!(report.fit_mm > 0.0, "the band's fit is reported beside the file's own bound: {report:?}");
    assert!(report.fit_mm < 1e-3, "{report:?}");
    assert!(String::from_utf8_lossy(&step.bytes).contains("B_SPLINE_SURFACE"));

    accepted(&mut state, "edit_fillet", json!({"feature":"Fillet1","radius":1.5}));
    assert_eq!(state.doc.features.last().unwrap().status, FeatureStatus::Ok);
    assert_eq!(bands(&state), 1, "the band keeps its name through a radius edit");

    state.doc.undo().unwrap();
    state.doc.undo().unwrap();
    assert_eq!(bands(&state), 0);
    let back = state.doc.kernel().mass_properties(state.doc.display_body().unwrap()).unwrap().volume;
    assert_eq!(back, plain);
    state.doc.redo().unwrap();
    state.doc.redo().unwrap();
    assert_eq!(bands(&state), 1);

    let dir = std::env::temp_dir().join(format!("soliddraft-ring-fillet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tee.dwprt");
    accepted(&mut state, "save", json!({"path":path}));
    accepted(&mut state, "open", json!({"path":path}));
    assert_eq!(state.doc.features.last().unwrap().status, FeatureStatus::Ok);
    assert_eq!(bands(&state), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A second, unrounded boss on the same wall leaves an edge the certificate cannot bound, so
/// the part writes no STEP and the refusal stays typed.
#[test]
fn a_ring_beside_an_unrounded_boss_has_no_certificate_and_writes_no_step() {
    let mut state = tee();
    accepted(
        &mut state,
        "sketch",
        json!({"plane":"front","shapes":[{"shape":"circle","center":[0,0,33],"diameter":6}]}),
    );
    accepted(&mut state, "extrude", json!({"sketch":"Sketch3","depth":20,"flip":false}));
    let edges = json!([ring(&state)]);
    accepted(&mut state, "fillet", json!({"edges":edges,"radius":1}));
    assert_eq!(state.doc.features.last().unwrap().status, FeatureStatus::Ok);
    assert_eq!(bands(&state), 1);
    let refused = state.doc.model().export(Format::Step).unwrap_err();
    assert!(refused.contains("numbers.geometric-gap-unproven"), "{refused}");
}
