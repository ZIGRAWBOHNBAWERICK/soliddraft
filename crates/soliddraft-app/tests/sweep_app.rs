//! Straight sweep commands, rendered property controls and part persistence.

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
    harness.get_by_label("OK (Enter)").click();
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
    harness.get_by_label("sweep end Z value").click();
    harness.run_steps(2);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_label("sweep end Z value").type_text("6");
    harness.run_steps(3);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(4);
    harness.get_by_label("OK (Enter)").click();
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
