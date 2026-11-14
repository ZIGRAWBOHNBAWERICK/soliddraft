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
            assert_eq!(import.record.as_ref().unwrap().source.sha256, import.sha256);
        }
        app.state_mut().state.push(AppAction::Undo);
        app.run_steps(4);
        assert!(app.state().state.doc.features.is_empty());
        app.state_mut().state.push(AppAction::Redo);
        app.run_steps(6);
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
                import.record.as_ref().unwrap().source.sha256,
                soliddraft_model::sha256_hex(reordered.as_bytes())
            );
        }
    });
}
