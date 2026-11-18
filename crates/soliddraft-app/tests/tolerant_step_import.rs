//! Default STEP admission keeps sampled imports distinct from certified solids.

use egui_kittest::{Harness, kittest::Queryable};
use soliddraft_app::{
    app::{Automation, SolidDraftApp},
    document::{Command, Document, ExtrudeParams, ProfileKind, Selection, TOLERANT_BODY_NAME},
    import::report::{self, Tier},
    state::AppAction,
};
use soliddraft_kernel_api::{BodyAccuracy, StepOptions};

#[path = "support/render_steps.rs"]
mod render_steps;

fn box_step() -> String {
    let mut doc = Document::new("Source", soliddraft_app::kernel());
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams {
            profile: ProfileKind::Rectangle { width: 10.0, height: 8.0 },
            depth: 6.0,
            ..Default::default()
        },
        via_assistant: false,
    })
    .unwrap();
    doc.kernel().export_step(doc.display_body().unwrap(), &StepOptions::default()).unwrap().text
}

#[test]
fn tolerant_and_certified_imports_display_with_their_own_authority_and_reopen() {
    let kept = std::env::var_os("SOLIDDRAFT_TOLERANT_IMPORT_PROOF_DIR").map(std::path::PathBuf::from);
    let root = kept
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-tolerant-import-{}", std::process::id())));
    std::fs::create_dir_all(&root).unwrap();
    soliddraft_app::file::set_config_dir(Some(root.join("config")));
    let certified = box_step();
    let tolerant = include_str!("../../../ci/fixtures/import/block-seam-4um.step").to_owned();
    for (name, source, tier) in [("certified", certified, Tier::Exact), ("tolerant", tolerant, Tier::Tolerant)] {
        let path = root.join(format!("{name}.step"));
        std::fs::write(&path, &source).unwrap();
        let mut app = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_step_dt(0.1)
            .renderer(render_steps::renderer())
            .build_eframe(|cc| {
                SolidDraftApp::new(cc, Document::new("Imported", soliddraft_app::kernel()), Automation::default())
            });
        app.run_steps(3);
        app.state_mut().state.push(AppAction::ImportStep(Some(path.clone())));
        app.run_steps(3);
        assert!(app.state().state.import_dialog.is_some());
        app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
        app.run_steps(3);
        let doc = &app.state().state.doc;
        let body = doc.display_body().unwrap_or_else(|| {
            panic!(
                "{name}: {:?}; refusals: {:?}; review: {:?}",
                doc.proof_message(),
                app.state().state.import_refusals,
                app.state().state.import_dialog.as_ref().map(|preview| preview.summary())
            )
        });
        let topology = doc.kernel().topology(body).unwrap();
        assert_eq!((topology.faces.len(), topology.edges.len(), topology.vertices.len()), (6, 12, 8));
        assert_eq!(report::rows(doc, &[])[0].tier, tier);
        assert!(doc.meshes[&body].triangle_count() > 0);
        let imported_volume = doc.kernel().mass_properties(body).unwrap().volume;
        if tier == Tier::Tolerant {
            assert_eq!(doc.kernel().body_accuracy(body).unwrap(), BodyAccuracy::Tolerant);
            assert_eq!(doc.body_name(0), TOLERANT_BODY_NAME);
            assert!(doc.body_tolerance(0).is_none(), "local fits are not a whole-body deviation bound");
            let details = doc.describe(Selection::Body).unwrap();
            assert!(details.details.iter().any(|(key, value)| key == "Accuracy" && value.contains("Tolerant")));
            assert!(
                doc.model()
                    .proof_report()
                    .features
                    .iter()
                    .all(|feature| feature.answer_kind != soliddraft_exchange::proof::AnswerKind::Proved)
            );
            let export = doc.model().export_at(soliddraft_exchange::Format::Step, None).unwrap();
            let soliddraft_exchange::Report::Step(report) = &export.report else { panic!("STEP report") };
            assert!(report.tolerant && report.approximate.is_none() && !report.within_tolerance);
            assert_eq!((report.faces, report.edges, report.vertices), (6, 12, 8));
            assert!(export.report.summary().contains("not certified"));
        } else {
            assert_eq!(doc.kernel().body_accuracy(body).unwrap(), BodyAccuracy::Exact);
            assert!(report::rows(doc, &[])[0].tier.label().contains("Certified"));
            assert!((doc.kernel().mass_properties(body).unwrap().volume - 480.0).abs() < 1e-6);
        }
        app.render().unwrap().save(root.join(format!("{name}.png"))).unwrap();
        let part = root.join(format!("{name}.dwprt"));
        soliddraft_app::file::save(&mut app.state_mut().state.doc, &part).unwrap();
        let reopened = soliddraft_app::file::open(&part, soliddraft_app::kernel(), None).unwrap();
        assert_eq!(report::rows(&reopened, &[])[0].tier, tier);
        assert!(
            (reopened.kernel().mass_properties(reopened.display_body().unwrap()).unwrap().volume - imported_volume)
                .abs()
                < 1e-6
        );
        let soliddraft_app::document::FeatureKind::ImportStep(import) = &reopened.features[0].kind else {
            panic!("import feature")
        };
        assert_eq!(import.source, source.as_bytes());
        assert_eq!(std::fs::read(&path).unwrap(), source.as_bytes());
        assert_eq!(reopened.faces.len(), 6);
        app.state_mut().state.doc = reopened;
        app.run_steps(3);
        app.render().unwrap().save(root.join(format!("{name}-reopened.png"))).unwrap();
        if tier == Tier::Exact {
            app.state_mut()
                .state
                .doc
                .apply(Command::AddExtrude {
                    params: ExtrudeParams {
                        profile: ProfileKind::Rectangle { width: 12.0, height: 6.0 },
                        depth: 8.0,
                        ..Default::default()
                    },
                    via_assistant: false,
                })
                .unwrap();
            app.run_steps(3);
            let doc = &app.state().state.doc;
            assert!((doc.kernel().mass_properties(doc.display_body().unwrap()).unwrap().volume - 696.0).abs() < 1e-6);
            app.render().unwrap().save(root.join("certified-follow-on.png")).unwrap();
        }
    }
    if kept.is_none() {
        std::fs::remove_dir_all(root).unwrap();
    }
}
