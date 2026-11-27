//! Default STEP admission keeps sampled imports distinct from certified solids.

use egui_kittest::{Harness, kittest::Queryable};
use soliddraft_app::{
    app::{Automation, SolidDraftApp},
    document::{Command, Document, ExtrudeParams, ProfileKind, Selection},
    import::report::{self, Tier},
    state::AppAction,
};
use soliddraft_kernel_api::{BodyAccuracy, StepOptions};

#[path = "support/import_open.rs"]
mod import_open;
#[path = "support/render_steps.rs"]
mod render_steps;

fn box_step() -> String {
    let mut doc = Document::new("Source", soliddraft_app::startup::kernel());
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
    // This generated box has one vertex 0.1 mm off its unchanged curves. It needs the
    // native tolerant path, not an exact-model cast or a whole-body certificate.
    let native = include_str!("data/native-tolerant-vertex.step").to_owned();
    let healed = include_str!("../../../ci/fixtures/import/healed-and-tolerant.step").to_owned();
    for (name, source, tier, body_count) in [
        ("certified", certified, Tier::Exact, 1),
        ("certified-repaired", tolerant, Tier::Repaired, 1),
        ("native-tolerant", native, Tier::Tolerant, 1),
        ("healed-checked", healed, Tier::Tolerant, 2),
    ] {
        let path = root.join(format!("{name}.step"));
        std::fs::write(&path, &source).unwrap();
        let mut app = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_step_dt(0.1)
            .renderer(render_steps::renderer())
            .build_eframe(|cc| {
                SolidDraftApp::new(
                    cc,
                    Document::new("Imported", soliddraft_app::startup::kernel()),
                    Automation::default(),
                )
            });
        app.run_steps(3);
        app.state_mut().state.push(AppAction::ImportStep(Some(path.clone())));
        import_open::settle(&mut app);
        assert!(app.state().state.import_dialog.is_some());
        app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
        import_open::settle(&mut app);
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
        assert_eq!(doc.features.len(), body_count);
        let rows = report::rows(doc, &[]);
        assert_eq!(rows.len(), body_count);
        assert!(rows.iter().all(|row| row.tier == tier));
        assert_eq!(doc.meshes.len(), body_count);
        assert!(doc.meshes.values().all(|mesh| mesh.triangle_count() > 0));
        let imported_volumes: std::collections::BTreeMap<_, _> = doc
            .model()
            .live_bodies()
            .into_iter()
            .map(|(key, body)| (key, doc.kernel().mass_properties(body).unwrap().volume))
            .collect();
        if tier == Tier::Tolerant {
            assert_eq!(doc.kernel().body_accuracy(body).unwrap(), BodyAccuracy::Tolerant);
            for (key, body) in doc.model().live_bodies() {
                assert_eq!(doc.kernel().body_accuracy(body).unwrap(), BodyAccuracy::Tolerant);
                assert!(doc.body_name(key).starts_with("Tolerant"));
                assert!(doc.body_tolerance(key).is_none(), "local fits are not a whole-body deviation bound");
            }
            let details = doc.describe(Selection::Body).unwrap();
            assert!(details.details.iter().any(|(key, value)| key == "Accuracy" && value.contains("Tolerant")));
            assert!(
                doc.model()
                    .proof_report()
                    .features
                    .iter()
                    .all(|feature| feature.answer_kind != soliddraft_exchange::proof::AnswerKind::Proved)
            );
            if body_count == 1 {
                let export = doc.model().export_at(soliddraft_exchange::Format::Step, None).unwrap();
                let soliddraft_exchange::Report::Step(report) = &export.report else { panic!("STEP report") };
                assert!(report.tolerant && report.approximate.is_none() && !report.within_tolerance);
                assert_eq!((report.faces, report.edges, report.vertices), (6, 12, 8));
                assert!(export.report.summary().contains("not certified"));
            } else {
                for (_, body) in doc.model().live_bodies() {
                    let export = doc.kernel().export_step(body, &StepOptions::default()).unwrap();
                    assert!(export.report.tolerant && !export.report.within_tolerance);
                    assert!(export.report.approximate.is_none());
                    assert_eq!((export.report.faces, export.report.edges, export.report.vertices), (6, 12, 8));
                }
            }
        } else {
            assert_eq!(doc.kernel().body_accuracy(body).unwrap(), BodyAccuracy::Exact);
            assert!(report::rows(doc, &[])[0].tier.label().contains("Certified"));
            let expected_volume = if name == "certified" { 480.0 } else { 10004.0 };
            assert!((doc.kernel().mass_properties(body).unwrap().volume - expected_volume).abs() < 1e-6);
        }
        app.render().unwrap().save(root.join(format!("{name}.png"))).unwrap();
        let part = root.join(format!("{name}.dwprt"));
        soliddraft_app::file::save(&mut app.state_mut().state.doc, &part).unwrap();
        let reopened = soliddraft_app::file::open(&part, soliddraft_app::startup::kernel(), None).unwrap();
        let rows = report::rows(&reopened, &[]);
        assert_eq!(rows.len(), body_count);
        assert!(rows.iter().all(|row| row.tier == tier));
        let soliddraft_app::document::FeatureKind::ImportStep(import) = &reopened.features[0].kind else {
            panic!("import feature")
        };
        assert_eq!(import.source, source.as_bytes());
        assert_eq!(std::fs::read(&path).unwrap(), source.as_bytes());
        let bodies = reopened.model().live_bodies();
        assert_eq!(bodies.len(), body_count);
        for (key, body) in bodies {
            let topology = reopened.kernel().topology(body).unwrap();
            assert_eq!((topology.faces.len(), topology.edges.len(), topology.vertices.len()), (6, 12, 8));
            let accuracy = if tier == Tier::Tolerant { BodyAccuracy::Tolerant } else { BodyAccuracy::Exact };
            assert_eq!(reopened.kernel().body_accuracy(body).unwrap(), accuracy);
            let before = imported_volumes[&key];
            let after = reopened.kernel().mass_properties(body).unwrap().volume;
            assert!((after - before).abs() < 1e-6, "{name} body {key}: {before} -> {after}");
        }
        app.state_mut().state.doc = reopened;
        app.run_steps(3);
        app.render().unwrap().save(root.join(format!("{name}-reopened.png"))).unwrap();
        if name == "certified" {
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
