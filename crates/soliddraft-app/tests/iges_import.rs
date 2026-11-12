//! Corpus IGES files through both frames, the shared review, display and saved part.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::Key;
use egui_kittest::kittest::Queryable;
use soliddraft_app::{
    commands,
    document::{Document, FeatureKind, FeatureStatus, PlaneKind, SketchPlane},
    settings::FrameLayout,
    state::AppAction,
};
use soliddraft_kernel_api::ImportFormat;

const BOX: &[u8] = include_bytes!("../../../ci/fixtures/import/occt-box.igs");
const SPHERE: &[u8] = include_bytes!("../../../ci/fixtures/import/occt-sphere.igs");
const REFUSAL: &str =
    "This IGES file uses a transform that changes lengths or angles. It cannot be imported without changing the shape.";

#[test]
fn corpus_iges_review_cancel_accept_reopen_and_refusal_in_both_frames() {
    let case = ribbon::case("iges-import");
    // The extension deliberately disagrees. Reopening must use the saved format.
    let source = case.path("box.step");
    let refused = case.path("sphere.igs");
    std::fs::write(&source, BOX).unwrap();
    std::fs::write(&refused, SPHERE).unwrap();
    for layout in [FrameLayout::Classic, FrameLayout::EditSuite] {
        let mut app = ribbon::app(Document::new("IGES import", soliddraft_app::kernel()));
        app.state_mut().state.frame_layout = layout;
        app.run_steps(3);
        case.run(&mut app, |app| {
            let command = commands::get("import_iges").unwrap();
            assert!(matches!(command.action_now(&app.state().state), Some(AppAction::ImportIges(None))));
            app.get_by_label("File").click();
            app.run_steps(3);
            app.get_by_label("Import IGES…");
            app.key_press(Key::Escape);
            app.run_steps(3);

            app.state_mut().state.push(AppAction::ImportIges(Some(source.clone())));
            app.run_steps(4);
            app.get_by_label("No repairs proposed.");
            assert!(app.state().state.doc.features.is_empty());
            app.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").click();
            app.run_steps(3);
            assert!(app.state().state.import_dialog.is_none());
            assert!(app.state().state.doc.features.is_empty());

            app.state_mut().state.push(AppAction::ImportIges(Some(source.clone())));
            app.run_steps(4);
            app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
            app.run_steps(8);
            let doc = &app.state().state.doc;
            assert_eq!(doc.features[0].status, FeatureStatus::Ok);
            let FeatureKind::ImportStep(import) = &doc.features[0].kind else { panic!("import feature") };
            assert_eq!(import.format, ImportFormat::Iges);
            assert_eq!(import.source, BOX);
            assert!(doc.meshes.values().any(|mesh| mesh.triangle_count() > 0));
            let body = doc.display_body().unwrap();
            assert_eq!(doc.kernel().topology(body).unwrap().faces.len(), 6);
            assert!((doc.kernel().mass_properties(body).unwrap().volume - 30.0).abs() < 1e-9);
            case.frame(app, &format!("{layout:?}-accepted"));

            let saved = case.path("imported.dwprt");
            app.state_mut().state.push(AppAction::SaveAs(Some(saved.clone())));
            app.run_steps(4);
            app.state_mut().state.push(AppAction::Open(Some(saved)));
            app.run_steps(8);
            let doc = &app.state().state.doc;
            assert_eq!(doc.features[0].status, FeatureStatus::Ok);
            let FeatureKind::ImportStep(import) = &doc.features[0].kind else { panic!("import feature") };
            assert_eq!(import.format, ImportFormat::Iges);
            assert_eq!(import.source, BOX);
            assert_eq!(import.record.as_ref().unwrap().source.sha256, soliddraft_model::sha256_hex(BOX));
            let revision = doc.revision();
            let undo = doc.undo_label().map(str::to_owned);
            app.state_mut().state.push(AppAction::ImportIges(Some(refused.clone())));
            app.run_steps(4);
            assert_eq!(app.state().state.note.as_ref().unwrap().text, REFUSAL);
            assert!(app.state().state.import_dialog.is_none());
            assert_eq!(app.state().state.doc.revision(), revision);
            assert_eq!(app.state().state.doc.undo_label().map(str::to_owned), undo);
            assert_eq!(app.state().state.doc.features.len(), 1);
            case.frame(app, &format!("{layout:?}-refused"));

            app.state_mut()
                .state
                .push(AppAction::StartSketchOn(SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 }));
            app.run_steps(4);
            assert!(app.state().state.in_sketch());
            assert!(!(command.shown)(&app.state().state));
            assert!(
                commands::search_here("iges", &app.state().state.commands, commands::Context::of(&app.state().state))
                    .is_empty()
            );
            assert!(command.action_now(&app.state().state).is_none());
            app.get_by_label("File").click();
            app.run_steps(3);
            assert!(app.query_all_by_label("Import IGES…").next().is_none());
        });
    }
}
