//! Sweep commands, rendered property controls and part persistence.

use serde_json::{Value, json};
use soliddraft_app::command_tools::{CommandTools, ProposalBook, Surface};
use soliddraft_app::document::{Document, FeatureKind, Selection};
use soliddraft_assistant::{Caller, ToolHost};

fn call(doc: &mut Document, name: &str, input: Value) -> Value {
    let mut book = ProposalBook::default();
    let result = CommandTools { doc, book: &mut book, surface: Surface::Mcp, drawing: None }.call(
        name,
        input,
        &Caller::Mcp { client: "sweep regression".into() },
    );
    assert!(!result.is_error, "{name}: {}", result.content);
    serde_json::from_str(&result.content).unwrap()
}

fn profile() -> Document {
    let mut doc = Document::new("Straight sweep", soliddraft_app::kernel());
    call(
        &mut doc,
        "sketch",
        json!({"plane":"top","offset":11,"shapes":[{"shape":"rectangle","center":[9,-2,11],"width":4,"height":2}]}),
    );
    doc
}

fn check(doc: &Document, volume: f64) {
    assert!(doc.features.iter().all(|f| matches!(f.status, soliddraft_app::document::FeatureStatus::Ok)));
    assert_eq!((doc.faces.len(), doc.edges.len()), (6, 12));
    let body = doc.display_body().unwrap();
    assert!((doc.kernel().mass_properties(body).unwrap().volume - volume).abs() < 1e-8);
    let mesh = &doc.meshes[&body];
    assert!(!mesh.indices.is_empty());
    for face in &doc.faces {
        assert!(doc.face_ref(face.id).is_some());
        assert!(mesh.face_picks.contains(&doc.pick_of_face(face.id).unwrap()));
    }
    for edge in &doc.edges {
        assert!(doc.edge_ref(edge.id).is_some());
        assert!(mesh.edges.iter().any(|e| e.pick == doc.pick_of_edge(edge.id).unwrap() && !e.segments.is_empty()));
    }
}

#[test]
fn sweep_commands_edit_undo_reopen_and_export_the_saved_path() {
    let mut doc = profile();
    call(&mut doc, "sweep", json!({"sketch":"Sketch1","start":[100,200,300],"end":[102,201,303]}));
    check(&doc, 24.0);
    let names = |doc: &Document| {
        doc.faces
            .iter()
            .map(|f| {
                let reference = doc.face_ref(f.id).unwrap();
                (reference.feature, reference.generator, reference.ordinal, reference.label)
            })
            .collect::<Vec<_>>()
    };
    let before = names(&doc);
    call(&mut doc, "edit_sweep", json!({"feature":"Sweep1","start":[100,200,300],"end":[102,201,306]}));
    check(&doc, 48.0);
    assert_eq!(before, names(&doc));
    call(&mut doc, "undo", json!({}));
    check(&doc, 24.0);
    call(&mut doc, "redo", json!({}));
    check(&doc, 48.0);
    let path = std::env::temp_dir().join(format!("soliddraft-sweep-{}.dwprt", std::process::id()));
    call(&mut doc, "save", json!({"path":path}));
    let mut reopened = Document::new("Reopened", soliddraft_app::kernel());
    call(&mut reopened, "open", json!({"path":path}));
    check(&reopened, 48.0);
    assert_eq!(before, names(&reopened));
    assert_eq!(doc.features[1].kind, reopened.features[1].kind);
    let step = reopened.kernel().export_step(reopened.display_body().unwrap(), &Default::default()).unwrap();
    assert!(step.text.contains("MANIFOLD_SOLID_BREP"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn sweep_ribbon_opens_property_page_commits_and_edits() {
    use egui_kittest::{Harness, kittest::Queryable};
    let instance = egui_wgpu::wgpu::Instance::default();
    assert!(
        !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty(),
        "Sweep UI requires a GPU adapter"
    );
    let config = std::env::temp_dir().join(format!("soliddraft-sweep-config-{}", std::process::id()));
    std::fs::create_dir_all(&config).unwrap();
    soliddraft_app::file::set_config_dir(Some(config));
    let mut doc = profile();
    let sketch = doc.features[0].id;
    doc.selection = Some(Selection::Feature(sketch));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_render_every_step(true)
        .wgpu()
        .build_eframe(move |cc| soliddraft_app::app::SolidDraftApp::new(cc, doc, Default::default()));
    harness.run_steps(4);
    harness.get_by_label("Sweep").click();
    harness.run_steps(4);
    assert!(matches!(
        harness.state().state.editor.as_ref().unwrap().params,
        soliddraft_app::state::EditorParams::Sweep(_)
    ));
    harness.get_by_label("OK").click();
    harness.run_steps(4);
    assert!(harness.state().state.editor.is_none());
    check(&harness.state().state.doc, 80.0);
    assert!(matches!(harness.state().state.doc.features[1].kind, FeatureKind::Sweep(_)));
    harness.key_press(egui::Key::Z);
    harness.run_steps(4);
    harness.get_all_by_role_and_label(egui::accesskit::Role::Button, "Sweep1").next().unwrap().click();
    harness.run_steps(4);
    harness.get_by_label("Edit feature").click();
    harness.run_steps(4);
    assert!(matches!(
        harness.state().state.editor.as_ref().unwrap().params,
        soliddraft_app::state::EditorParams::Sweep(_)
    ));
    let proof = std::env::var_os("SOLIDDRAFT_SWEEP_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-sweep-proof-{}", std::process::id())));
    std::fs::create_dir_all(&proof).unwrap();
    harness.render().unwrap().save(proof.join("sweep-property-page.png")).unwrap();
    // The property page scrolls; bring the control into view as a user would.
    harness.get_by_label("sweep end Z value").scroll_to_me();
    harness.run_steps(24);
    harness.get_by_label("sweep end Z value").click();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    // The path starts on the profile, at Z 11.
    harness.get_by_label("sweep end Z value").type_text("17");
    harness.run_steps(3);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(4);
    harness.get_by_label("OK").click();
    harness.run_steps(4);
    check(&harness.state().state.doc, 48.0);
    harness.render().unwrap().save(proof.join("sweep-edited.png")).unwrap();
    let path = proof.join("sweep-edited.step");
    let result = soliddraft_app::app_tools::AppTools { state: &mut harness.state_mut().state }.call(
        "export_step",
        json!({"path":path}),
        &Caller::Mcp { client: "sweep regression".into() },
    );
    assert!(!result.is_error, "{}", result.content);
    assert!(std::fs::read_to_string(path).unwrap().contains("MANIFOLD_SOLID_BREP"));
}

#[test]
fn refused_paths_and_curved_profiles_leave_the_document_unchanged() {
    let mut doc = profile();
    call(&mut doc, "sweep", json!({"sketch":"Sketch1","start":[0,0,0],"end":[2,1,3]}));
    let before = doc.features[1].kind.clone();
    for (end, message) in
        [(json!([0, 0, 0]), "different finite endpoints"), (json!([2, 1, 0]), "leave the profile plane")]
    {
        let result =
            CommandTools { doc: &mut doc, book: &mut ProposalBook::default(), surface: Surface::Mcp, drawing: None }
                .call(
                    "edit_sweep",
                    json!({"feature":"Sweep1","start":[0,0,0],"end":end}),
                    &Caller::Mcp { client: "sweep regression".into() },
                );
        assert!(result.is_error, "{}", result.content);
        assert!(result.content.contains(message), "{}", result.content);
        assert_eq!(doc.features[1].kind, before);
        check(&doc, 24.0);
    }
    for input in [
        json!({"op":"sweep","sketch":"Sketch1","start":[0,0,0],"end":[2,1,3],"segments":[{"kind":"arc"}]}),
        json!({"op":"edit_sweep","feature":"Sweep1","start":[0,0,0],"end":[2,1,3],"segments":[{},{}]}),
    ] {
        assert!(serde_json::from_value::<soliddraft_app::command_tools::Line>(input).is_err());
    }
    let mut circle = Document::new("Curved profile", soliddraft_app::kernel());
    call(&mut circle, "sketch", json!({"plane":"top","shapes":[{"shape":"circle","center":[0,0,0],"diameter":2}]}));
    let result =
        CommandTools { doc: &mut circle, book: &mut ProposalBook::default(), surface: Surface::Mcp, drawing: None }
            .call(
                "sweep",
                json!({"sketch":"Sketch1","start":[0,0,0],"end":[2,1,3]}),
                &Caller::Mcp { client: "sweep regression".into() },
            );
    assert!(result.is_error);
    assert!(result.content.contains("closed polygon profile with straight sides"), "{}", result.content);
    assert_eq!(circle.features.len(), 1);
}

#[test]
fn sweep_work_refusal_is_plain_and_preserves_the_sketch() {
    let mut doc = Document::new(
        "Sweep refusal",
        std::sync::Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(0)),
    );
    call(
        &mut doc,
        "sketch",
        json!({"plane":"top","shapes":[{"shape":"rectangle","center":[0,0,0],"width":4,"height":2}]}),
    );
    let result =
        CommandTools { doc: &mut doc, book: &mut ProposalBook::default(), surface: Surface::Mcp, drawing: None }.call(
            "sweep",
            json!({"sketch":"Sketch1","start":[0,0,0],"end":[2,1,3]}),
            &Caller::Mcp { client: "sweep regression".into() },
        );
    assert!(result.is_error);
    for internal in ["Catallax", "stage", "numbers.", "kernel"] {
        assert!(!result.content.contains(internal), "{}", result.content);
    }
    assert_eq!(doc.features.len(), 1);
    assert!(doc.display_body().is_none());
}

fn beside_the_path() -> Document {
    let mut doc = Document::new("Chain sweep", soliddraft_app::kernel());
    call(
        &mut doc,
        "sketch",
        json!({"plane":"top","shapes":[{"shape":"rectangle","center":[1,-1.5,0],"width":2,"height":1}]}),
    );
    doc
}

fn chain(run: f64) -> Value {
    json!([
        {"kind":"line","id":"rise","to":[0,0,4]},
        {"kind":"arc","id":"bend","centre":[0,5,4],"axis":[-1,0,0],"turn":"quarter"},
        {"kind":"line","id":"run","to":[0,run,9]},
    ])
}

fn selectable(doc: &Document, volume: f64) {
    assert!(doc.features.iter().all(|f| matches!(f.status, soliddraft_app::document::FeatureStatus::Ok)));
    let body = doc.display_body().unwrap();
    assert!((doc.kernel().mass_properties(body).unwrap().volume - volume).abs() < 1e-6);
    let mesh = &doc.meshes[&body];
    for face in &doc.faces {
        assert!(doc.any_face_ref(face.id).is_some(), "face {} has no saved name", face.id);
        assert!(mesh.face_picks.contains(&doc.pick_of_face(face.id).unwrap()));
    }
    for edge in &doc.edges {
        assert!(mesh.edges.iter().any(|e| e.pick == doc.pick_of_edge(edge.id).unwrap() && !e.segments.is_empty()));
    }
}

#[test]
fn line_arc_chain_edits_undoes_reopens_selects_and_exports() {
    use std::f64::consts::PI;
    let at = |run: f64| 16.0 + 6.5 * PI + 2.0 * (run - 9.0);
    let mut doc = beside_the_path();
    call(&mut doc, "sweep", json!({"sketch":"Sketch1","start":[0,0,0],"segments":chain(9.0)}));
    selectable(&doc, at(9.0));
    let names = |doc: &Document| {
        let mut names: Vec<_> = doc
            .faces
            .iter()
            .map(|f| {
                let reference = doc.any_face_ref(f.id).unwrap();
                (reference.feature, reference.generator, reference.ordinal, reference.label)
            })
            .collect();
        names.sort_by(|a, b| a.3.cmp(&b.3));
        names
    };
    let before = names(&doc);
    assert!(before.iter().any(|n| n.3.contains("along bend")), "{before:?}");
    assert!(!before.iter().any(|n| n.3.contains("Catallax")));
    call(&mut doc, "edit_sweep", json!({"feature":"Sweep1","start":[0,0,0],"segments":chain(12.0)}));
    selectable(&doc, at(12.0));
    assert_eq!(before, names(&doc), "editing a run keeps every face's name");
    call(&mut doc, "undo", json!({}));
    selectable(&doc, at(9.0));
    call(&mut doc, "redo", json!({}));
    selectable(&doc, at(12.0));
    let path = std::env::temp_dir().join(format!("soliddraft-chain-{}.dwprt", std::process::id()));
    call(&mut doc, "save", json!({"path":path}));
    let mut reopened = Document::new("Reopened", soliddraft_app::kernel());
    call(&mut reopened, "open", json!({"path":path}));
    selectable(&reopened, at(12.0));
    assert_eq!(before, names(&reopened));
    assert_eq!(doc.features[1].kind, reopened.features[1].kind);
    let saved = call(&mut reopened, "feature", json!({"name":"Sweep1"}));
    assert!(saved.to_string().contains("three_quarter") || saved.to_string().contains("quarter"), "{saved}");
    let step = reopened.kernel().export_step(reopened.display_body().unwrap(), &Default::default()).unwrap();
    assert!(step.text.contains("MANIFOLD_SOLID_BREP") && step.text.contains("CYLINDRICAL_SURFACE"));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn removing_a_chain_segment_keeps_the_others_and_edits_stay_undoable() {
    let mut doc = beside_the_path();
    call(&mut doc, "sweep", json!({"sketch":"Sketch1","start":[0,0,0],"segments":chain(9.0)}));
    call(
        &mut doc,
        "edit_sweep",
        json!({"feature":"Sweep1","start":[0,0,0],"segments":[{"kind":"line","id":"rise","to":[0,0,4]}]}),
    );
    selectable(&doc, 8.0);
    call(&mut doc, "undo", json!({}));
    let FeatureKind::Sweep(sweep) = &doc.features[1].kind else { panic!("a sweep") };
    assert_eq!(sweep.path.segments.len(), 3);
    assert_eq!(sweep.path.segments[2].id(), "run");
}

#[test]
fn unnamed_segments_keep_the_identity_saved_at_their_place() {
    let mut doc = beside_the_path();
    call(&mut doc, "sweep", json!({"sketch":"Sketch1","start":[0,0,0],"end":[0,0,4]}));
    call(
        &mut doc,
        "edit_sweep",
        json!({"feature":"Sweep1","start":[0,0,0],"segments":[
            {"kind":"line","to":[0,0,5]},
            {"kind":"line","to":[0,0,6]},
        ]}),
    );
    let FeatureKind::Sweep(sweep) = &doc.features[1].kind else { panic!("a sweep") };
    assert_eq!(sweep.path.segments[0].id(), "travel");
    assert_eq!(sweep.path.segments[1].id(), "segment1");
}

#[test]
fn a_path_is_an_end_or_segments_and_refusals_leave_the_part() {
    let mut doc = beside_the_path();
    call(&mut doc, "sweep", json!({"sketch":"Sketch1","start":[0,0,0],"segments":chain(9.0)}));
    let before = doc.features[1].kind.clone();
    for (input, message) in [
        (json!({"feature":"Sweep1","start":[0,0,0]}), "`end` or as `segments`"),
        (json!({"feature":"Sweep1","start":[0,0,0],"end":[0,0,1],"segments":chain(9.0)}), "`end` or as `segments`"),
        (
            json!({"feature":"Sweep1","start":[0,0,0],"segments":[
                {"kind":"line","to":[0,0,4]},{"kind":"line","to":[0,3,7]},
            ]}),
            "coming soon",
        ),
        (json!({"feature":"Sweep1","start":[0,0,0],"orientation":"fixed","segments":chain(9.0)}), "coming soon"),
        (
            json!({"feature":"Sweep1","start":[0,0,0],"segments":[
                {"kind":"line","to":[0,0,4]},
                {"kind":"arc","centre":[0,5,3],"axis":[0,0,1],"turn":"half"},
            ]}),
            "square to the line",
        ),
    ] {
        let result =
            CommandTools { doc: &mut doc, book: &mut ProposalBook::default(), surface: Surface::Mcp, drawing: None }
                .call("edit_sweep", input, &Caller::Mcp { client: "sweep regression".into() });
        assert!(result.is_error, "{}", result.content);
        assert!(result.content.contains(message), "{}", result.content);
        for internal in ["Catallax", "stage", "kernel", "features."] {
            assert!(!result.content.contains(internal), "{}", result.content);
        }
        assert_eq!(doc.features[1].kind, before);
    }
}

#[test]
fn property_page_builds_a_chain_and_offers_only_what_works() {
    use egui_kittest::{Harness, kittest::Queryable};
    use soliddraft_kernel_api::{PathSegment, SweepOrientation};
    let instance = egui_wgpu::wgpu::Instance::default();
    assert!(
        !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty(),
        "Sweep UI requires a GPU adapter"
    );
    let config = std::env::temp_dir().join(format!("soliddraft-chain-config-{}", std::process::id()));
    std::fs::create_dir_all(&config).unwrap();
    soliddraft_app::file::set_config_dir(Some(config));
    let mut doc = profile();
    let sketch = doc.features[0].id;
    doc.selection = Some(Selection::Feature(sketch));
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 1100.0))
        .with_render_every_step(true)
        .wgpu()
        .build_eframe(move |cc| soliddraft_app::app::SolidDraftApp::new(cc, doc, Default::default()));
    harness.run_steps(4);
    harness.get_by_label("Sweep").click();
    harness.run_steps(4);
    assert!(harness.query_by_label("Spline · Coming soon").is_none());
    assert!(harness.query_by_label("Use spline path").is_some());
    assert!(harness.query_by_label("Helix · Coming soon").is_some());
    assert!(harness.query_by_label("Keep direction").is_some());
    // The property page scrolls; bring the control into view as a user would.
    harness.get_by_label("Add arc").scroll_to_me();
    harness.run_steps(24);
    harness.get_by_label("Add arc").click();
    harness.run_steps(4);
    let path = |harness: &Harness<'_, soliddraft_app::app::SolidDraftApp>| {
        let Some(soliddraft_app::state::EditorParams::Sweep(s)) =
            harness.state().state.editor.as_ref().map(|e| e.params.clone())
        else {
            panic!("the sweep page is open")
        };
        s.path
    };
    assert_eq!(path(&harness).orientation, SweepOrientation::FollowPath);
    assert!(matches!(path(&harness).segments[1], PathSegment::Arc { .. }));
    assert!(harness.query_by_label("Keep direction · Coming soon").is_some());
    // The property page scrolls; bring the control into view as a user would.
    harness.get_by_label("Half").scroll_to_me();
    harness.run_steps(24);
    harness.get_by_label("Half").click();
    harness.run_steps(4);
    // The property page scrolls; bring the control into view as a user would.
    harness.get_by_label("Add line").scroll_to_me();
    harness.run_steps(24);
    harness.get_by_label("Add line").click();
    harness.run_steps(6);
    assert_eq!(path(&harness).segments.len(), 3);
    let proof = std::env::var_os("SOLIDDRAFT_SWEEP_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-chain-proof-{}", std::process::id())));
    std::fs::create_dir_all(&proof).unwrap();
    harness.render().unwrap().save(proof.join("sweep-chain-page.png")).unwrap();
    harness.get_by_label("OK").click();
    harness.run_steps(6);
    assert!(harness.state().state.editor.is_none());
    let doc = &harness.state().state.doc;
    assert!(matches!(doc.features[1].kind, FeatureKind::Sweep(_)));
    assert!(doc.features.iter().all(|f| matches!(f.status, soliddraft_app::document::FeatureStatus::Ok)));
    assert!(!doc.faces.is_empty());
    harness.render().unwrap().save(proof.join("sweep-chain-done.png")).unwrap();
}

#[test]
fn sweeps_saved_before_chains_read_back_with_their_names() {
    let saved = json!({
        "sketch": soliddraft_model::FeatureId(1),
        "path": {"start":[1,2,3],"end":[1,2,9],"segment_id":"travel"},
    });
    let sweep: soliddraft_model::SweepFeature = serde_json::from_value(saved).unwrap();
    assert_eq!(sweep.path, soliddraft_kernel_api::Sweep::line([1.0, 2.0, 3.0], [1.0, 2.0, 9.0], "travel"));
    let name = soliddraft_model::Generator::Side { entity: 2, path: None };
    let text = serde_json::to_value(&name).unwrap();
    assert_eq!(text, json!({"kind":"side","entity":2}), "a one-piece path keeps the saved name");
    assert_eq!(serde_json::from_value::<soliddraft_model::Generator>(text).unwrap(), name);
}

fn check_spline(doc: &Document, volume: f64) {
    selectable(doc, volume);
    assert_eq!((doc.faces.len(), doc.edges.len()), (6, 12));
    assert!(!doc.meshes[&doc.display_body().unwrap()].indices.is_empty());
    for edge in &doc.edges {
        assert!(doc.edge_ref(edge.id).is_some(), "edge {} has no saved name", edge.id);
    }
}

fn spline_controls(end_z: f64) -> Value {
    json!([{"kind":"spline", "id":"rail", "degree":3,
        "control_points":[[0,0,0],[1,0,end_z / 3.0],[1,1,end_z * 2.0 / 3.0],[0,1,end_z]],
        "weights":[1,1,1,1], "knots":[0,0,0,0,1,1,1,1]}])
}

#[test]
fn spline_sweep_edits_reopens_and_refuses_a_return_without_changing_the_part() {
    let mut doc = profile();
    call(
        &mut doc,
        "sweep",
        json!({"sketch":"Sketch1", "start":[0,0,0], "orientation":"fixed", "segments":spline_controls(3.0)}),
    );
    check_spline(&doc, 24.0);
    let names = |doc: &Document| {
        doc.faces
            .iter()
            .map(|f| {
                let reference = doc.any_face_ref(f.id).unwrap();
                (reference.feature, reference.generator, reference.ordinal, reference.label)
            })
            .collect::<Vec<_>>()
    };
    let before_names = names(&doc);
    call(&mut doc, "edit_sweep", json!({"feature":"Sweep1", "start":[0,0,0], "segments":spline_controls(6.0)}));
    check_spline(&doc, 48.0);
    assert_eq!(before_names, names(&doc));
    call(&mut doc, "undo", json!({}));
    check_spline(&doc, 24.0);
    call(&mut doc, "redo", json!({}));
    check_spline(&doc, 48.0);
    let before = doc.features[1].kind.clone();
    let mut returning = spline_controls(3.0);
    returning[0]["control_points"][3] = json!([0, 0, 0]);
    {
        let segments = returning;
        let message = "path may turn back or cross itself";
        let refused =
            CommandTools { doc: &mut doc, book: &mut ProposalBook::default(), surface: Surface::Mcp, drawing: None }
                .call(
                    "edit_sweep",
                    json!({"feature":"Sweep1", "start":[0,0,0], "segments":segments}),
                    &Caller::Mcp { client: "spline sweep regression".into() },
                );
        assert!(refused.is_error, "{}", refused.content);
        assert!(refused.content.contains(message), "{}", refused.content);
        for internal in ["sweep.spline", "mesh.vertex-limit", "Catallax", "stack", "kernel", "tessellate", "status 0"] {
            assert!(!refused.content.contains(internal), "{}", refused.content);
        }
        assert_eq!(doc.features[1].kind, before);
        check_spline(&doc, 48.0);
    }
    // Owner decision 13 Nov: certified-first display now accepts this former display refusal as proved.
    let mut detailed = spline_controls(3.0);
    detailed[0]["control_points"][3] = json!([0, 1, 6]);
    let (accepted, work) =
        soliddraft_kernel_catallax::with_request_allowance(soliddraft_kernel_catallax::APP_REQUEST_WORK, || {
            call(&mut doc, "edit_sweep", json!({"feature":"Sweep1", "start":[0,0,0], "segments":detailed}))
        });
    let body = doc.display_body().unwrap();
    let mesh = doc.kernel().rendering(body, &Default::default()).unwrap();
    eprintln!(
        "detailed spline sweep: {work:?}; vertices={}; bound={:?}; {accepted}",
        mesh.positions.len(),
        mesh.bounded.as_ref().map(|bound| (&bound.kind, bound.measured_mm, bound.level_mm))
    );
    assert!(!work.exhausted && work.used > 0 && work.used <= 20_000_000, "{work:?}");
    assert!(mesh.bounded.is_none() && !mesh.unproved, "the native proved answer must stay proved");
    assert!(!mesh.positions.is_empty() && !mesh.indices.is_empty());
    assert!(doc.proof_state().current_proved_display());
    assert!(doc.display_answer(doc.features[1].id).is_none());
    let (export, _) = soliddraft_kernel_catallax::with_request_allowance(20_000_000, || {
        doc.kernel().tessellate(body, &Default::default())
    });
    assert!(export.is_err(), "a named screen result cannot replace an independent certified request");
    call(&mut doc, "undo", json!({}));
    assert_eq!(doc.features[1].kind, before);
    check_spline(&doc, 48.0);
    let path = std::env::temp_dir().join(format!("soliddraft-spline-sweep-{}.dwprt", std::process::id()));
    call(&mut doc, "save", json!({"path":path}));
    let mut reopened = Document::new("Reopened spline", soliddraft_app::kernel());
    call(&mut reopened, "open", json!({"path":path}));
    check_spline(&reopened, 48.0);
    assert_eq!(reopened.features[1].kind, before);
    let step = reopened.kernel().export_step(reopened.display_body().unwrap(), &Default::default()).unwrap();
    assert!(step.text.contains("B_SPLINE_SURFACE"));
    std::fs::remove_file(path).unwrap();
}

#[path = "support/ui.rs"]
mod ribbon;

#[test]
fn spline_controls_build_through_the_registry_in_edit_suite() {
    use egui_kittest::kittest::Queryable;
    use soliddraft_app::commands;
    let case = ribbon::case("spline-sweep");
    {
        let layout = "edit_suite";
        let mut doc = profile();
        doc.selection = Some(Selection::Feature(doc.features[0].id));
        let mut app = ribbon::app(doc);

        app.run_steps(4);
        ribbon::run_command(&mut app, "sweep");

        let bar = egui::PanelState::load(&app.ctx, egui::Id::new("options_bar")).unwrap().outer_rect;
        let (_, rect) = ribbon::buttons_in(&app, bar).into_iter().find(|(name, _)| name.starts_with("More (")).unwrap();
        ribbon::click_at(&mut app, rect.center());
        app.run_steps(6);

        app.get_by_label("Use spline path").scroll_to_me();
        app.run_steps(24);
        app.get_by_label("Use spline path").click();
        app.run_steps(6);
        assert!(app.query_by_label("Turn with the path · Coming soon").is_some());
        case.frame(&mut app, &format!("{layout:?}-controls"));
        app.get_by_label("OK").click();
        app.run_steps(6);
        assert!(app.state().state.editor.is_none());
        check_spline(&app.state().state.doc, 240.0);
        case.frame(&mut app, &format!("{layout:?}-result"));
        assert!(!commands::get("sweep").unwrap().available(commands::Context { sketch: true, ..Default::default() }));
    }
}

#[test]
fn pinned_sweep_hides_during_sketch_and_returns_in_edit_suite() {
    use egui_kittest::kittest::Queryable;
    use soliddraft_app::command_line::BarSettings;
    let case = ribbon::case("pinned-sweep-sketch");
    let pins = BarSettings { tools: vec!["sweep".into(), "zoom_fit".into()], ..Default::default() };
    soliddraft_app::settings::save_command_line(&pins).unwrap();
    {
        let layout = "edit_suite";
        let mut doc = ribbon::sketch();
        doc.selection = Some(Selection::Feature(doc.features[0].id));
        let mut app = ribbon::app_at(doc, ribbon::EXPANDED_WINDOW, 1.0);

        app.run_steps(4);
        case.run(&mut app, |app| {
            app.get_by_label("Sweep on the command bar");
            ribbon::run_command(app, "edit_feature");
            assert!(app.state().state.sketch.is_some());
            assert!(
                app.query_all_by_label("Sweep on the command bar").next().is_none(),
                "pinned Sweep must be hidden while a sketch is open in {layout:?}"
            );
            app.get_by_label("Fit on the command bar");
            assert!(!soliddraft_app::commands::get("sweep").unwrap().is_enabled(&app.state().state));
            assert_eq!(app.state().state.command_line.settings.tools, pins.tools);
            assert_eq!(soliddraft_app::settings::command_line().tools, pins.tools);
            case.frame(app, &format!("{layout:?}-open-sketch"));

            ribbon::run_command(app, "exit_sketch");
            assert!(app.state().state.sketch.is_none() && app.state().state.editor.is_none());
            assert_eq!(app.state().state.command_line.settings.tools, pins.tools);
            assert_eq!(soliddraft_app::settings::command_line().tools, pins.tools);
            app.get_by_label("Sweep on the command bar").click();
            app.run_steps(6);
            assert!(matches!(
                app.state().state.editor.as_ref().map(|e| &e.params),
                Some(soliddraft_app::state::EditorParams::Sweep(_))
            ));
            assert_eq!(app.state().state.doc.features.len(), 1);
            case.frame(app, &format!("{layout:?}-sweep-after-close"));
        });
    }
}
