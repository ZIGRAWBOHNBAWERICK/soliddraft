//! Release import flows use the real review, kernel, feature tree and saved source.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use soliddraft_app::{
    command_tools::{CommandTools, ProposalBook, Surface},
    document::{Document, FeatureKind, FeatureStatus},
    import::{Preview, Review, commit},
    settings::FrameLayout,
    state::AppAction,
};
use soliddraft_assistant::{Caller, ToolHost};
use soliddraft_kernel_api::ImportFormat;

const BLOCK: &[u8] = include_bytes!("../../../ci/fixtures/import/block-seam-4um.step");
const MESH: &[u8] = include_bytes!("../../../ci/fixtures/import/tetrahedron.obj");

fn call(doc: &mut Document, name: &str, input: Value) -> Result<Value, String> {
    let output = CommandTools { doc, book: &mut ProposalBook::default(), surface: Surface::Mcp, drawing: None }.call(
        name,
        input,
        &Caller::Mcp { client: "release import".into() },
    );
    if output.is_error { Err(output.content) } else { Ok(serde_json::from_str(&output.content).unwrap()) }
}

fn imported() -> (Document, u64) {
    let mut doc = Document::new("Replacement", soliddraft_app::kernel());
    let preview = Preview::from_bytes(&doc, "part.step", BLOCK.to_vec()).unwrap();
    let id = commit(&mut doc, &preview, &Review::default()).unwrap();
    (doc, id)
}

#[test]
fn replaced_source_keeps_identity_and_marks_old_edge_and_face_references_for_repair() {
    let (mut doc, id) = imported();
    call(&mut doc, "fillet", json!({"edges":["Import1 · Edge 10"],"radius":2})).unwrap();
    call(
        &mut doc,
        "sketch",
        json!({"plane":"Import1 · Face 6","shapes":[{"shape":"circle","center":[10,12.5,10.004],"diameter":6}]}),
    )
    .unwrap();
    let revision = doc.revision();
    let same = Preview::from_bytes(&doc, "same.step", BLOCK.to_vec()).unwrap().replacing(&doc, id).unwrap();
    assert_eq!(commit(&mut doc, &same, &Review::default()).unwrap(), id);
    assert!(doc.features.iter().all(|f| f.status == FeatureStatus::Ok));
    assert!(doc.revision() > revision);
    let mut changed = BLOCK.to_vec();
    changed.push(b'\n');
    let preview = Preview::from_bytes(&doc, "new.step", changed.clone()).unwrap().replacing(&doc, id).unwrap();
    let untouched = doc.model().to_bytes().unwrap();
    assert_eq!(commit(&mut doc, &preview, &Review::default()).unwrap(), id);
    assert_eq!(doc.features.len(), 3);
    assert_eq!(doc.features[0].name, "Import1");
    assert_eq!(doc.features[0].status, FeatureStatus::Ok);
    assert!(matches!(doc.features[1].status, FeatureStatus::NeedsRepair(_)));
    assert!(matches!(doc.features[2].status, FeatureStatus::NeedsRepair(_)));
    let FeatureKind::ImportStep(source) = &doc.features[0].kind else { panic!("import") };
    assert_eq!(source.source, changed);
    assert!(source.decisions.is_empty());
    let saved = doc.model().to_bytes().unwrap();
    let reopened =
        soliddraft_app::file::open_bytes("replacement.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
    assert!(matches!(reopened.features[1].status, FeatureStatus::NeedsRepair(_)));
    assert!(matches!(reopened.features[2].status, FeatureStatus::NeedsRepair(_)));
    doc.undo().unwrap();
    assert_eq!(doc.model().to_bytes().unwrap(), untouched);
    assert!(doc.features.iter().all(|f| f.status == FeatureStatus::Ok));
    doc.redo().unwrap();
    assert!(matches!(doc.features[1].status, FeatureStatus::NeedsRepair(_)));
}

#[test]
fn replacement_preview_is_bound_to_target_revision_and_reviewed_bytes() {
    let (mut doc, id) = imported();
    let preview = Preview::from_bytes(&doc, "new.step", BLOCK.to_vec()).unwrap().replacing(&doc, id).unwrap();
    assert_ne!(preview.token(), Preview::from_bytes(&doc, "new.step", BLOCK.to_vec()).unwrap().token());
    assert!(Preview::from_bytes(&doc, "bad.step", b"broken".to_vec()).is_err());
    assert_eq!(doc.features.len(), 1);
    call(&mut doc, "rename", json!({"feature":"Import1","name":"Imported part"})).unwrap();
    assert!(commit(&mut doc, &preview, &Review::default()).unwrap_err().contains("changed"));
    let original = doc.model().to_bytes().unwrap();
    let wide = include_bytes!("../../../ci/fixtures/import/block-seam-20um.step");
    let preview = Preview::from_bytes(&doc, "wide.step", wide.to_vec()).unwrap().replacing(&doc, id).unwrap();
    assert!(commit(&mut doc, &preview, &Review::default()).is_err());
    assert_eq!(doc.model().to_bytes().unwrap(), original);
    assert!(commit(&mut doc, &preview, &Review { decline: vec!["g0".into()], accept: Vec::new() }).is_err());
    assert_eq!(doc.model().to_bytes().unwrap(), original);
}

#[test]
fn closed_mesh_review_accepts_displays_saves_reopens_and_exports_an_exact_solid() {
    let case = ribbon::case("release-mesh-import");
    let source = case.path("tetrahedron.obj");
    std::fs::write(&source, MESH).unwrap();
    let mut app = ribbon::app(Document::new("Mesh solid", soliddraft_app::kernel()));
    app.run_steps(3);
    let command = soliddraft_app::commands::get("import_mesh").unwrap();
    assert!(!(command.shown)(&app.state().state));
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.run_steps(3);
    app.get_by_label("File").click();
    app.run_steps(3);
    app.get_by_label("Import mesh solid…");
    app.key_press(egui::Key::Escape);
    app.run_steps(3);
    app.state_mut().state.push(AppAction::ImportMesh(Some(source.clone())));
    app.run_steps(4);
    app.get_by_label("No repairs proposed.");
    assert!(app.state().state.doc.features.is_empty());
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").click();
    app.run_steps(3);
    assert!(app.state().state.import_dialog.is_none());
    app.state_mut().state.push(AppAction::ImportMesh(Some(source)));
    app.run_steps(4);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
    app.run_steps(8);
    let doc = &app.state().state.doc;
    assert_eq!(doc.features[0].status, FeatureStatus::Ok);
    let body = doc.display_body().unwrap();
    assert_eq!(doc.kernel().topology(body).unwrap().faces.len(), 4);
    assert!((doc.kernel().mass_properties(body).unwrap().volume - 1000.0 / 6.0).abs() < 1e-8);
    assert!(doc.meshes[&body].triangle_count() > 0);
    case.frame(&mut app, "accepted");
    let row = app.query_all_by_label("Import1").next().unwrap().rect().center();
    app.hover_at(row);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos: row,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        app.step();
    }
    app.run_steps(3);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Replace imported file");
    assert!(matches!(
        soliddraft_app::commands::get("replace_import").unwrap().action_now(&app.state().state),
        Some(AppAction::ReplaceImport { .. })
    ));
    case.frame(&mut app, "replace-menu");
    app.key_press(egui::Key::Escape);
    app.run_steps(3);
    let saved = app.state().state.doc.model().to_bytes().unwrap();
    let reopened = soliddraft_app::file::open_bytes("mesh.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
    let FeatureKind::ImportStep(import) = &reopened.features[0].kind else { panic!("import") };
    assert_eq!(import.format, ImportFormat::Mesh);
    assert_eq!(import.source, MESH);
    let export = reopened.kernel().export_step(reopened.display_body().unwrap(), &Default::default()).unwrap();
    assert!(export.report.within_tolerance);
    let preview =
        Preview::from_bytes(&Document::new("check", soliddraft_app::kernel()), "mesh.step", export.text.into_bytes())
            .unwrap();
    assert!(preview.summary()["proposals"].as_array().unwrap().is_empty());
    app.state_mut().state.doc.undo().unwrap();
    assert!(app.state().state.doc.features.is_empty());
}

#[test]
fn source_cleanup_certificate_is_visible_and_survives_a_saved_part() {
    let mut doc = Document::new("Source cleanup", soliddraft_app::kernel());
    let source = include_bytes!("../../soliddraft-kernel-catallax/tests/fixtures/planar-source-incidence.step");
    let preview = Preview::from_bytes(&doc, "incidence.step", source.to_vec()).unwrap();
    let id = commit(&mut doc, &preview, &Review::default()).unwrap();
    let record = doc.model().import_record(soliddraft_model::FeatureId(id as u32)).unwrap();
    assert_eq!(record.format, "catallax-import-repair/3");
    assert_eq!(record.source_incidence.len(), 1);
    let rows = soliddraft_app::import::record_rows(record);
    assert!(rows.iter().any(|(name, _)| name.ends_with("source to result")));
    let saved = doc.model().to_bytes().unwrap();
    let reopened = soliddraft_app::file::open_bytes("cleanup.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
    assert_eq!(reopened.model().import_record(soliddraft_model::FeatureId(id as u32)), Some(record));
    assert_eq!(reopened.features[0].status, FeatureStatus::Ok);
}

#[test]
fn ascii_and_binary_stl_take_the_same_review_and_saved_solid_path() {
    let vertices = [[0.0_f32, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 10.0]];
    let triangles = [[0, 2, 1], [0, 1, 3], [0, 3, 2], [1, 2, 3]];
    let mut ascii = "solid tetrahedron\n".to_owned();
    let mut binary = vec![0_u8; 80];
    binary.extend_from_slice(&4_u32.to_le_bytes());
    for triangle in triangles {
        ascii.push_str("facet normal 0 0 0\nouter loop\n");
        binary.extend_from_slice(&[0; 12]);
        for i in triangle {
            let [x, y, z] = vertices[i];
            ascii.push_str(&format!("vertex {x} {y} {z}\n"));
            for v in [x, y, z] {
                binary.extend_from_slice(&v.to_le_bytes());
            }
        }
        ascii.push_str("endloop\nendfacet\n");
        binary.extend_from_slice(&[0; 2]);
    }
    ascii.push_str("endsolid tetrahedron\n");
    for bytes in [ascii.into_bytes(), binary] {
        let mut doc = Document::new("STL solid", soliddraft_app::kernel());
        let preview = Preview::from_source(&doc, "tetrahedron.stl", bytes.clone(), ImportFormat::Mesh).unwrap();
        commit(&mut doc, &preview, &Review::default()).unwrap();
        assert!(
            (doc.kernel().mass_properties(doc.display_body().unwrap()).unwrap().volume - 1000.0 / 6.0).abs() < 1e-8
        );
        let saved = doc.model().to_bytes().unwrap();
        let reopened = soliddraft_app::file::open_bytes("stl.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
        let FeatureKind::ImportStep(source) = &reopened.features[0].kind else { panic!("import") };
        assert_eq!(source.source, bytes);
        assert_eq!(reopened.features[0].status, FeatureStatus::Ok);
    }
}

#[test]
fn mesh_refusals_are_plain_and_do_not_change_the_part() {
    let doc = Document::new("Refusals", soliddraft_app::kernel());
    let text = std::str::from_utf8(MESH).unwrap();
    for (source, fragment) in [
        (b"broken".to_vec(), "STL or OBJ"),
        (text.replace("f 2 3 4\n", "").into_bytes(), "open edges"),
        (text.replace("f 2 3 4", "f 2 4 3").into_bytes(), "reversed faces"),
        (text.replace("f 2 3 4", "f 2 2 4").into_bytes(), "no area"),
    ] {
        let error = Preview::from_source(&doc, "bad.obj", source, ImportFormat::Mesh).unwrap_err();
        assert!(error.contains(fragment), "{error}");
    }
    let mut oversized = vec![0; 84];
    oversized[80..84].copy_from_slice(&200001_u32.to_le_bytes());
    let truncated = Preview::from_source(&doc, "truncated.stl", oversized.clone(), ImportFormat::Mesh).unwrap_err();
    assert!(truncated.contains("STL or OBJ"), "{truncated}");
    oversized.resize(84 + 200001 * 50, 0);
    let error = Preview::from_source(&doc, "large.stl", oversized, ImportFormat::Mesh).unwrap_err();
    assert!(error.contains("200,000"), "{error}");
    assert!(doc.features.is_empty());
    assert_eq!(doc.revision(), 0);
}

#[test]
fn source_cleanup_outside_the_certified_limit_is_refused_before_editing() {
    let doc = Document::new("Refused cleanup", soliddraft_app::kernel());
    let source = include_str!("../../soliddraft-kernel-catallax/tests/fixtures/planar-source-incidence.step")
        .replace("(0.000001,0,1)", "(0.003,0,1)");
    let original = doc.model().to_bytes().unwrap();
    let error = Preview::from_bytes(&doc, "tilted.step", source.into_bytes()).unwrap_err();
    assert_eq!(error, "This STEP file could not be imported without an unverified repair.");
    assert_eq!(doc.model().to_bytes().unwrap(), original);
    assert!(doc.features.is_empty());
    assert_eq!(doc.revision(), 0);
}

#[test]
fn closed_mesh_downstream_boss_edits_reopens_and_exports_as_a_solid() {
    use soliddraft_kernel_api::Kernel;
    use std::sync::Arc;

    let kernel = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
    let mut doc = Document::new("Mesh with boss", kernel.clone());
    let preview = Preview::from_source(&doc, "tetrahedron.obj", MESH.to_vec(), ImportFormat::Mesh).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    call(
        &mut doc,
        "sketch",
        json!({"plane":"top","shapes":[{"shape":"rectangle","center":[0,0,0],"width":2,"height":2}]}),
    )
    .unwrap();
    call(&mut doc, "extrude", json!({"sketch":"Sketch1","depth":2,"flip":false})).unwrap();
    assert!(doc.features.iter().all(|f| f.status == FeatureStatus::Ok));
    let volume = |doc: &Document| doc.kernel().mass_properties(doc.display_body().unwrap()).unwrap().volume;
    assert!((volume(&doc) - 518.0 / 3.0).abs() < 1e-6);
    let recipe = kernel.body_recipe(doc.display_body().unwrap()).unwrap();
    assert_ne!(recipe["format"], json!("soliddraft-recipe/1"));
    let imported = recipe["steps"].as_array().unwrap().iter().find(|s| s["op"] == "import_mesh").unwrap();
    assert_eq!(imported["file"]["sha256"], soliddraft_model::sha256_hex(MESH));
    let saved = doc.model().to_bytes().unwrap();
    let mut reopened =
        soliddraft_app::file::open_bytes("mesh-boss.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
    assert!(reopened.features.iter().all(|f| f.status == FeatureStatus::Ok));
    assert!((volume(&reopened) - 518.0 / 3.0).abs() < 1e-6);
    call(&mut reopened, "edit_extrude", json!({"feature":"Boss-Extrude1","depth":3,"flip":false})).unwrap();
    assert!((volume(&reopened) - 527.0 / 3.0).abs() < 1e-6);
    reopened.undo().unwrap();
    assert!((volume(&reopened) - 518.0 / 3.0).abs() < 1e-6);
    reopened.redo().unwrap();
    assert!((volume(&reopened) - 527.0 / 3.0).abs() < 1e-6);
    let exported = reopened.kernel().export_step(reopened.display_body().unwrap(), &Default::default()).unwrap();
    assert!(exported.report.within_tolerance);
    assert_eq!(exported.report.solids, 1);
}
