//! Several-solid imports through review, selection, replacement and saved replay.

#[path = "support/ui.rs"]
mod ribbon;

use egui_kittest::kittest::Queryable;
use soliddraft_app::{
    document::{Command, Document, FeatureKind, FeatureStatus},
    state::AppAction,
};
use soliddraft_kernel_api::{BodyId, ImportFormat};
use std::collections::BTreeMap;

const STEP: &[u8] = include_bytes!("../../../ci/fixtures/import/two-blocks-5um.step");
const IGES: &[u8] = include_bytes!("../../../ci/fixtures/import/two-blocks.igs");

fn reviewed(app: &mut ribbon::App, path: std::path::PathBuf, format: ImportFormat) {
    app.state_mut().state.push(match format {
        ImportFormat::Step => AppAction::ImportStep(Some(path)),
        ImportFormat::Iges => AppAction::ImportIges(Some(path)),
        ImportFormat::Mesh => unreachable!(),
    });
    app.run_steps(4);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
    app.run_steps(8);
    assert!(app.state().state.import_dialog.is_none(), "{:?}", app.state().state.note.as_ref().map(|n| &n.text));
}

type BodyMembership = BTreeMap<String, (u32, BodyId, f64, [f64; 3])>;

fn membership(doc: &Document) -> BodyMembership {
    assert!(doc.features.iter().all(|f| f.status == FeatureStatus::Ok));
    doc.features
        .iter()
        .filter_map(|feature| {
            let FeatureKind::ImportStep(import) = &feature.kind else { return None };
            let body = doc.model().live_bodies().into_iter().find(|(key, _)| *key == feature.body_key).unwrap().1;
            let mass = doc.kernel().mass_properties(body).unwrap();
            Some((
                import.source_solid.clone().expect("source selector"),
                (feature.body_key, body, mass.volume, mass.centroid.into()),
            ))
        })
        .collect()
}

fn geometry(doc: &Document) -> BTreeMap<String, (f64, [f64; 3])> {
    membership(doc).into_iter().map(|(key, (_, _, volume, centre))| (key, (volume, centre))).collect()
}

fn flow(format: ImportFormat, source: &[u8]) {
    let case = ribbon::case(&format!("multisolid-{}", format.label()));
    let path = case.path("original.source");
    std::fs::write(&path, source).unwrap();
    let mut app = ribbon::app(Document::new("Several imported solids", soliddraft_app::kernel()));
    case.run(&mut app, |app| {
        reviewed(app, path.clone(), format);
        let doc = &app.state().state.doc;
        assert_eq!(doc.features.len(), 2);
        assert_eq!(doc.meshes.len(), 2);
        assert_eq!(doc.undo_depth(), 1, "the reviewed file is one undo step");
        let before = membership(doc);
        assert_eq!(before.len(), 2);
        assert_ne!(before.values().next().unwrap().3, before.values().last().unwrap().3);
        for feature in &doc.features {
            let FeatureKind::ImportStep(import) = &feature.kind else { unreachable!() };
            assert_eq!(import.format, format);
            assert_eq!(import.source, source);
            assert_eq!(import.sha256, soliddraft_model::sha256_hex(source));
            assert_eq!(
                doc.model().import_record(soliddraft_model::FeatureId(feature.id as u32)).unwrap().source.sha256,
                import.sha256
            );
        }
        app.state_mut().state.push(AppAction::Undo);
        app.run_steps(4);
        ribbon::until(app, "undo rebuild", |app| !app.state().state.doc.proof_pending());
        assert!(app.state().state.doc.features.is_empty());
        app.state_mut().state.push(AppAction::Redo);
        app.run_steps(6);
        ribbon::until(app, "redo rebuild", |app| !app.state().state.doc.proof_pending());
        assert_eq!(membership(&app.state().state.doc), before);

        let saved = case.path("imported.dwprt");
        app.state_mut().state.push(AppAction::SaveAs(Some(saved.clone())));
        app.run_steps(4);
        app.state_mut().state.push(AppAction::Open(Some(saved)));
        app.run_steps(8);
        let reopened = membership(&app.state().state.doc);
        for (selector, (key, _, volume, centre)) in &before {
            let (new_key, _, new_volume, new_centre) = reopened[selector];
            assert_eq!(new_key, *key);
            assert!((new_volume - volume).abs() < 1e-6);
            assert_eq!(new_centre, *centre);
        }

        let doc = &mut app.state_mut().state.doc;
        let first = doc.features[0].id;
        let second = doc.features[1].id;
        doc.apply(Command::Reorder { id: second, index: 0 }).unwrap();
        assert_eq!(membership(doc), reopened, "feature order cannot swap source solids");
        doc.undo().unwrap();

        app.state_mut().state.push(AppAction::SelectBody(0));
        app.run_steps(3);
        let untouched = app.state().state.doc.model().live_bodies()[1].1;
        app.state_mut().state.push(AppAction::Command(Command::AddFeature(FeatureKind::Mirror(
            soliddraft_model::MirrorFeature {
                plane: soliddraft_model::PlaneRef::Datum { datum: soliddraft_model::DatumPlane::Right, offset: 0.0 },
                keep_original: false,
            },
        ))));
        app.run_steps(6);
        let doc = &app.state().state.doc;
        assert_eq!(doc.features.last().unwrap().body_key, 0);
        assert!(doc.features.iter().all(|f| f.status == FeatureStatus::Ok));
        assert_eq!(doc.model().live_bodies()[1].1, untouched, "editing one import keeps the other solid");
        app.state_mut().state.push(AppAction::Undo);
        app.run_steps(4);
        ribbon::until(app, "undo rebuild", |app| !app.state().state.doc.proof_pending());

        app.state_mut().state.push(AppAction::StartCombine);
        app.run_steps(4);
        app.get_by_label("Combine bodies").get_by_label("Combine").click();
        app.run_steps(8);
        assert!(app.state().state.doc.combine_form.is_none());
        let doc = &app.state().state.doc;
        assert_eq!(doc.model().live_bodies().len(), 1);
        let volume = doc.kernel().mass_properties(doc.display_body().unwrap()).unwrap().volume;
        assert!((volume - before.values().map(|v| v.2).sum::<f64>()).abs() < 1e-6);
        case.frame(app, "combined-imports");
        app.state_mut().state.push(AppAction::Undo);
        app.run_steps(4);
        ribbon::until(app, "undo rebuild", |app| !app.state().state.doc.proof_pending());

        let original = app.state().state.doc.model().to_bytes().unwrap();
        let revision = app.state().state.doc.revision();
        let depth = app.state().state.doc.undo_depth();
        let replacement = case.path("replacement.source");
        let different = if format == ImportFormat::Step {
            String::from_utf8(source.to_vec()).unwrap().replace("(#150,#300,#309)", "(#300,#150,#309)").into_bytes()
        } else {
            include_bytes!("../../../ci/fixtures/import/occt-box.igs").to_vec()
        };
        assert_ne!(different, source);
        std::fs::write(&replacement, different).unwrap();
        app.state_mut().state.push(AppAction::ReplaceImport { feature: first, path: Some(replacement) });
        app.run_steps(4);
        let state = &app.state().state;
        assert!(state.import_dialog.is_none());
        assert!(state.note.as_ref().unwrap().text.contains("Import a changed file as new bodies"));
        assert_eq!(state.doc.model().to_bytes().unwrap(), original);
        assert_eq!(state.doc.revision(), revision);
        assert_eq!(state.doc.undo_depth(), depth);

        app.state_mut().state.push(AppAction::ReplaceImport { feature: first, path: Some(path) });
        app.run_steps(4);
        app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
        app.run_steps(8);
        for (selector, (key, _, volume, centre)) in &reopened {
            let (new_key, _, new_volume, new_centre) = membership(&app.state().state.doc)[selector];
            assert_eq!(new_key, *key);
            assert!((new_volume - volume).abs() < 1e-6);
            assert_eq!(new_centre, *centre);
        }
    });
}

#[test]
fn step_solids_review_replay_reorder_edit_combine_and_refuse_replacement() {
    flow(ImportFormat::Step, STEP);
}

#[test]
fn iges_solids_review_replay_reorder_edit_combine_and_refuse_replacement() {
    flow(ImportFormat::Iges, IGES);
}

#[test]
fn reordering_source_solid_declarations_preserves_entity_to_geometry_membership() {
    let case = ribbon::case("multisolid-source-order");
    let original = case.path("original.step");
    std::fs::write(&original, STEP).unwrap();
    let text = std::str::from_utf8(STEP).unwrap();
    let mut lines: Vec<_> = text.lines().collect();
    let solids: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| line.contains("=MANIFOLD_SOLID_BREP(").then_some(index))
        .collect();
    assert_eq!(solids.len(), 2);
    lines.swap(solids[0], solids[1]);
    let reordered = lines.join("\n").replace("(#150,#300,#309)", "(#300,#150,#309)");
    let path = case.path("reordered.step");
    std::fs::write(&path, reordered.as_bytes()).unwrap();
    let mut app = ribbon::app(Document::new("Original order", soliddraft_app::kernel()));
    case.run(&mut app, |app| {
        reviewed(app, original, ImportFormat::Step);
        let expected = geometry(&app.state().state.doc);
        app.state_mut().state.doc = Document::new("Reordered source", soliddraft_app::kernel());
        reviewed(app, path, ImportFormat::Step);
        assert_eq!(geometry(&app.state().state.doc), expected);
        for feature in &app.state().state.doc.features {
            let FeatureKind::ImportStep(import) = &feature.kind else { unreachable!() };
            assert_eq!(import.source, reordered.as_bytes());
            assert_eq!(
                app.state()
                    .state
                    .doc
                    .model()
                    .import_record(soliddraft_model::FeatureId(feature.id as u32))
                    .unwrap()
                    .source
                    .sha256,
                soliddraft_model::sha256_hex(reordered.as_bytes())
            );
        }
    });
}

#[test]
fn release_mcp_scenario_imports_two_bodies_undoes_and_previews_the_next_file() {
    use serde_json::Value;
    use soliddraft_app::command_tools::{CommandTools, ProposalBook, Surface};
    use soliddraft_assistant::{Caller, ToolHost};

    let scenario: Value = serde_json::from_str(include_str!("../../../ci/scenarios/import-block.json")).unwrap();
    let mut doc = Document::new("MCP solids", soliddraft_app::kernel());
    let mut book = ProposalBook::default();
    let mut token = Value::Null;
    let mut imported = Vec::new();
    let mut revisions = Vec::new();
    for (index, step) in scenario["steps"].as_array().unwrap().iter().take(5).enumerate() {
        let mut input = step["arguments"].clone();
        if let Some(path) = input["path"].as_str() {
            input["path"] =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(path).to_str().unwrap().into();
        }
        if index == 1 {
            input["preview_token"] = token.clone();
        }
        let out = CommandTools { doc: &mut doc, book: &mut book, surface: Surface::Mcp, drawing: None }.call(
            step["tool"].as_str().unwrap(),
            input,
            &Caller::Mcp { client: "multi-solid import scenario".into() },
        );
        if let Some(message) = step["refused"].as_str() {
            assert!(out.is_error, "{}", out.content);
            assert!(out.content.contains(message), "{}", out.content);
            assert_eq!(doc.model().to_bytes().unwrap(), imported);
        } else {
            assert!(!out.is_error, "{}", out.content);
            let result: Value = serde_json::from_str(&out.content).unwrap();
            if index == 0 {
                token = result["token"].clone();
            }
            if index == 1 {
                assert_eq!(membership(&doc).len(), 2);
                assert_eq!(result["features"][0]["source_solid"], "#150");
                assert_eq!(result["features"][1]["source_solid"], "#300");
                imported = doc.model().to_bytes().unwrap();
            }
        }
        for check in step["checks"].as_array().unwrap() {
            if check["pointer"] == "/revision" {
                let earlier = check["same_as_step"].as_u64().unwrap() as usize;
                assert_eq!(doc.revision(), revisions[earlier - 1], "scenario step {}", index + 1);
            }
        }
        revisions.push(doc.revision());
    }
    assert!(doc.features.is_empty());
}

fn saved_stock_with_body_counter(next: u32) -> Vec<u8> {
    let stock = ribbon::plate();
    let mut truth: serde_json::Value = serde_json::from_str(&stock.model().part_json()).unwrap();
    truth["format"] = "soliddraft-part/3".into();
    truth["next_body"] = next.into();
    soliddraft_model::Document::from_part_json(&truth.to_string(), soliddraft_app::kernel())
        .unwrap()
        .to_bytes()
        .unwrap()
}

#[test]
fn saved_exhausted_body_id_range_refuses_import_without_changes() {
    for counter in [u32::MAX, u32::MAX - 1] {
        let case = ribbon::case(&format!("import-exhausted-body-ids-{counter}"));
        let saved = saved_stock_with_body_counter(counter);
        let doc = soliddraft_app::file::open_bytes("stock.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
        assert_eq!(doc.model().next_body(), counter);
        assert_eq!(doc.model().live_bodies().len(), 1);
        let source = case.path("two-solids.step");
        std::fs::write(&source, STEP).unwrap();
        let mut app = ribbon::app(doc);
        case.run(&mut app, |app| {
            let before = app.state().state.doc.model().to_bytes().unwrap();
            let revision = app.state().state.doc.revision();
            let depth = app.state().state.doc.undo_depth();
            let bodies = app.state().state.doc.model().live_bodies();
            let meshes = app.state().state.doc.meshes.clone();
            app.state_mut().state.push(AppAction::ImportStep(Some(source)));
            app.run_steps(4);
            app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
            app.run_steps(8);
            let state = &app.state().state;
            assert_eq!(
                state.note.as_ref().unwrap().text,
                "This part cannot hold more bodies. Import the file into a new part."
            );
            assert!(state.import_dialog.is_some(), "the reviewed source remains available after refusal");
            assert_eq!(state.doc.model().to_bytes().unwrap(), before);
            assert_eq!(state.doc.revision(), revision);
            assert_eq!(state.doc.undo_depth(), depth);
            assert_eq!(state.doc.model().live_bodies(), bodies);
            assert_eq!(state.doc.meshes, meshes);
        });
    }
}

#[test]
fn last_available_body_id_range_imports_saves_and_undoes_without_reusing_ids() {
    let case = ribbon::case("import-last-body-ids");
    let saved = saved_stock_with_body_counter(u32::MAX - 2);
    let doc = soliddraft_app::file::open_bytes("stock.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
    let source = case.path("two-solids.step");
    std::fs::write(&source, STEP).unwrap();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        reviewed(app, source, ImportFormat::Step);
        let doc = &app.state().state.doc;
        assert_eq!(doc.model().next_body(), u32::MAX);
        assert_eq!(doc.features.iter().map(|f| f.body_key).collect::<Vec<_>>(), [0, u32::MAX - 2, u32::MAX - 1]);
        let saved = doc.model().to_bytes().unwrap();
        let reopened =
            soliddraft_app::file::open_bytes("last-ids.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
        assert_eq!(reopened.model().next_body(), u32::MAX);
        assert_eq!(reopened.model().live_bodies().len(), 3);
        app.state_mut().state.push(AppAction::Undo);
        app.run_steps(4);
        ribbon::until(app, "undo rebuild", |app| !app.state().state.doc.proof_pending());
        assert_eq!(app.state().state.doc.features.len(), 1);
        assert_eq!(app.state().state.doc.model().next_body(), u32::MAX);
        app.state_mut().state.push(AppAction::Redo);
        app.run_steps(4);
        ribbon::until(app, "redo rebuild", |app| !app.state().state.doc.proof_pending());
        assert_eq!(app.state().state.doc.model().live_bodies().len(), 3);
        assert_eq!(app.state().state.doc.model().next_body(), u32::MAX);
    });
}
