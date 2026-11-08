//! Clean analytic STEP imports through the review dialog, rendered view and saved part.

use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};
use soliddraft_app::{
    app::{Automation, SolidDraftApp},
    app_tools::AppTools,
    document::{Command, Document, ExtrudeParams, PlaneKind, ProfileKind},
    state::AppAction,
};
use soliddraft_assistant::{Caller, ToolHost};
use soliddraft_kernel_api::StepOptions;
use soliddraft_tessellation::Level;
use std::time::{Duration, Instant};

#[path = "support/render_steps.rs"]
mod render_steps;

fn tool(state: &mut soliddraft_app::state::AppState, name: &str, input: Value) -> Value {
    let out = AppTools { state }.call(name, input, &Caller::Mcp { client: "clean STEP proof".into() });
    assert!(!out.is_error, "{name}: {}", out.content);
    serde_json::from_str(&out.content).unwrap()
}

fn check_cylinder_mesh(mesh: &soliddraft_app::document::DisplayMesh, chord: f64, angle_deg: f64) -> Value {
    let mut worst_chord = 0.0_f64;
    let mut worst_angle = 0.0_f64;
    for triangle in mesh.indices.as_chunks::<3>().0 {
        let ids = [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize];
        let points = ids.map(|i| mesh.positions[i].as_dvec3());
        if mesh.normals[ids[0]].z.abs() > 0.5 {
            continue;
        }
        let normal = (points[1] - points[0]).cross(points[2] - points[0]).normalize();
        assert!(normal.is_finite());
        for i in 0..3 {
            let radial = points[i].truncate();
            assert!((radial.length() - 3.0).abs() < 1e-6);
            worst_angle = worst_angle.max(
                normal
                    .dot(glam::DVec3::new(radial.x, radial.y, 0.0).normalize())
                    .abs()
                    .clamp(-1.0, 1.0)
                    .acos()
                    .to_degrees(),
            );
            let next = points[(i + 1) % 3].truncate();
            let edge = next - radial;
            let t = if edge.length_squared() == 0.0 {
                0.0
            } else {
                (-radial.dot(edge) / edge.length_squared()).clamp(0.0, 1.0)
            };
            worst_chord = worst_chord.max(3.0 - (radial + t * edge).length());
        }
    }
    assert!(worst_chord <= chord + 1e-6, "rounded mesh chord {worst_chord} exceeds {chord}");
    assert!(worst_angle <= angle_deg + 1e-4, "rounded mesh angle {worst_angle} exceeds {angle_deg}");
    json!({"triangles":mesh.triangle_count(), "max_chord_mm":worst_chord, "max_angle_deg":worst_angle, "requested_chord_mm":chord, "requested_angle_deg":angle_deg})
}

#[test]
fn clean_block_and_cylinder_import_render_save_reopen_and_checked_export() {
    let root = std::env::var_os("SOLIDDRAFT_STEP_IMPORT_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-step-import-{}", std::process::id())));
    std::fs::create_dir_all(&root).unwrap();
    soliddraft_app::file::set_config_dir(Some(root.join("config")));
    for (name, profile, height, faces, volume) in [
        ("block", ProfileKind::Rectangle { width: 2.0, height: 3.0 }, 5.0, 6, 30.0),
        ("cylinder", ProfileKind::Circle { diameter: 6.0 }, 7.0, 3, 63.0 * std::f64::consts::PI),
    ] {
        let kernel = soliddraft_app::kernel();
        let mut original = Document::new("Source", kernel.clone());
        original
            .apply(Command::AddExtrude {
                params: ExtrudeParams {
                    sketch: None,
                    profile,
                    plane: PlaneKind::Top,
                    depth: height,
                    flip: false,
                    end: Default::default(),
                },
                via_assistant: false,
            })
            .unwrap();
        let source = kernel.export_step(original.display_body().unwrap(), &StepOptions::default()).unwrap();
        assert!(source.report.within_tolerance);
        let bytes = source.text.into_bytes();
        let hash = soliddraft_model::sha256_hex(&bytes);
        let path = root.join(format!("{name}.step"));
        std::fs::write(&path, &bytes).unwrap();
        let mut app = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_step_dt(0.1)
            .renderer(render_steps::renderer())
            .build_eframe(|cc| {
                SolidDraftApp::new(cc, Document::new("Imported", kernel.clone()), Automation::default())
            });
        app.run_steps(3);
        app.state_mut().state.push(AppAction::ImportStep(Some(path.clone())));
        app.run_steps(3);
        assert!(app.state().state.import_dialog.is_some());
        assert!(app.state().state.doc.features.is_empty(), "preview must not edit the part");
        app.get_by_label("No repairs proposed.");
        app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
        app.run_steps(3);
        let doc = &app.state().state.doc;
        assert_eq!(doc.features[0].status, soliddraft_app::document::FeatureStatus::Ok);
        let body = doc.display_body().unwrap();
        assert_eq!(kernel.topology(body).unwrap().faces.len(), faces);
        assert!((kernel.mass_properties(body).unwrap().volume - volume).abs() < 1e-6);
        let record = doc.model().import_record(soliddraft_model::FeatureId(doc.features[0].id as u32)).unwrap().clone();
        assert!(record.changes.is_empty());
        assert!(record.derivative.is_none());
        assert_eq!(record.source.sha256, hash);
        let chord = doc.display_quality().levels().params(Level::Standard).chord_height;
        // Leave face pixels between the renderer's seven-pixel vertex pick targets at coarse zoom.
        for (level, ratio) in [(Level::Coarse, 0.08), (Level::Standard, 0.6), (Level::Fine, 2.0)] {
            let pixels_per_point = app.ctx.pixels_per_point();
            let state = &mut app.state_mut().state;
            let bounds = state.doc.meshes[&body].bounds().unwrap();
            state.viewport.camera.pan(0.0, 0.0, state.viewport.rect.height());
            state.viewport.camera.target = (bounds.0 + bounds.1) * 0.5;
            state.viewport.camera.orthographic = true;
            let pixels_per_mm = ratio / chord;
            state.viewport.camera.distance = state.viewport.rect.height() * pixels_per_point
                / (2.0 * (state.viewport.camera.fov_y / 2.0).tan() * pixels_per_mm as f32);
            let start = Instant::now();
            loop {
                app.run_steps(1);
                if app.state().state.doc.display_quality().shown() == Some(level) {
                    break;
                }
                assert!(
                    start.elapsed() < Duration::from_secs(120),
                    "{name}: {level:?} did not display, shown {:?}, wanted {:?}",
                    app.state().state.doc.display_quality().shown(),
                    app.state().state.doc.display_quality().wanted()
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            let doc = &app.state().state.doc;
            assert_eq!(doc.tess_params, *doc.display_quality().levels().params(level));
            assert!(doc.meshes[&body].triangle_count() > 0);
            assert!(doc.meshes[&body].positions.iter().all(|p| p.is_finite()));
            if name == "cylinder" {
                let stats = check_cylinder_mesh(
                    &doc.meshes[&body],
                    doc.tess_params.chord_height,
                    doc.tess_params.surface_angle_deg,
                );
                std::fs::write(
                    root.join(format!("{name}-{level}-mesh.json")),
                    serde_json::to_vec_pretty(&stats).unwrap(),
                )
                .unwrap();
            }
            let center = app.state().state.viewport.rect.center();
            render_steps::await_pick(&mut app, center);
            assert!(
                app.state().state.viewport.pick_sample().data.iter().any(|pixel| {
                    let Some(soliddraft_app::viewport::render::Picked::Face(id)) =
                        soliddraft_app::viewport::render::Picked::decode(pixel[1])
                    else {
                        return false;
                    };
                    matches!(app.state().state.doc.pick(id), Some(soliddraft_app::document::PickEntry::Face(_, _)))
                }),
                "the rendered body must answer picks"
            );
            app.render().unwrap().save(root.join(format!("{name}-{}.png", level.as_str()))).unwrap();
        }
        let saved = root.join(format!("{name}.dwprt"));
        tool(&mut app.state_mut().state, "save", json!({"path":saved}));
        tool(&mut app.state_mut().state, "open", json!({"path":saved}));
        app.run_steps(3);
        let doc = &app.state().state.doc;
        let soliddraft_app::document::FeatureKind::ImportStep(import) = &doc.features[0].kind else {
            panic!("import feature")
        };
        assert_eq!(import.source, bytes);
        assert_eq!(import.sha256, hash);
        assert_eq!(import.record.as_ref(), Some(&record));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(doc.display_body().is_some(), "reopen must verify and rebuild the import");
        let exported = root.join(format!("{name}-reexport.step"));
        let answer = tool(&mut app.state_mut().state, "export_step", json!({"path":exported}));
        std::fs::write(root.join(format!("{name}-export-report.json")), serde_json::to_vec_pretty(&answer).unwrap())
            .unwrap();
        assert!(!answer["report"].as_str().unwrap().contains("over the requested tolerance"));
        let checked = app
            .state()
            .state
            .doc
            .kernel()
            .export_step(app.state().state.doc.display_body().unwrap(), &StepOptions::default())
            .unwrap();
        assert!(checked.report.within_tolerance);
        let reexport = std::fs::read(&exported).unwrap();
        let preview =
            soliddraft_app::import::Preview::from_bytes(&app.state().state.doc, "check.step", reexport).unwrap();
        assert!(preview.summary()["proposals"].as_array().unwrap().is_empty());
        let mut reimported = Document::new("Re-export check", kernel.clone());
        let preview = soliddraft_app::import::Preview::read(&reimported, &exported).unwrap();
        soliddraft_app::import::commit(&mut reimported, &preview, &Default::default()).unwrap();
        assert!((kernel.mass_properties(reimported.display_body().unwrap()).unwrap().volume - volume).abs() < 1e-6);
        app.render().unwrap().save(root.join(format!("{name}-reopened.png"))).unwrap();
    }
}

/// An approved seam through the real dialog, renderer, saved part and checked export.
#[test]
fn approved_seam_repair_renders_saves_reopens_and_exports_with_its_record() {
    let root = std::env::var_os("SOLIDDRAFT_STEP_IMPORT_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-step-repair-{}", std::process::id())));
    std::fs::create_dir_all(&root).unwrap();
    soliddraft_app::file::set_config_dir(Some(root.join("repair-config")));
    let bytes = include_bytes!("../../../ci/fixtures/import/block-seam-20um.step").to_vec();
    let path = root.join("block-seam-20um.step");
    std::fs::write(&path, &bytes).unwrap();
    let kernel = soliddraft_app::kernel();
    let mut app = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_step_dt(0.1)
        .renderer(render_steps::renderer())
        .build_eframe(|cc| SolidDraftApp::new(cc, Document::new("Repaired", kernel.clone()), Automation::default()));
    app.run_steps(3);
    app.state_mut().state.push(AppAction::ImportStep(Some(path.clone())));
    app.run_steps(3);
    app.get_by_label("Measured 0.02 mm");
    app.render().unwrap().save(root.join("repair-review.png")).unwrap();
    app.get_by_role_and_label(egui::accesskit::Role::TextInput, "Approved by").click();
    app.run_steps(2);
    app.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    app.key_press(egui::Key::Delete);
    app.run_steps(2);
    app.get_by_role_and_label(egui::accesskit::Role::TextInput, "Approved by").type_text("Ana Reviewer");
    app.run_steps(3);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
    app.run_steps(3);

    let doc = &app.state().state.doc;
    assert_eq!(doc.features[0].status, soliddraft_app::document::FeatureStatus::Ok);
    let id = soliddraft_model::FeatureId(doc.features[0].id as u32);
    let record = doc.model().import_record(id).unwrap().clone();
    assert_eq!(record.changes[0].approval, soliddraft_kernel_api::Approval::User);
    assert_eq!(record.changes[0].approver.as_deref(), Some("Ana Reviewer"));
    assert_eq!(record.face_tolerances.len(), 4);
    assert!(record.derivative.is_some());
    let body = doc.display_body().unwrap();
    assert_eq!(kernel.topology(body).unwrap().faces.len(), 6);
    assert!((kernel.mass_properties(body).unwrap().volume - 10020.0).abs() < 1e-6);
    let centre = app.state().state.viewport.rect.center();
    render_steps::await_pick(&mut app, centre);
    let selected = soliddraft_app::document::Selection::Feature(id.0.into());
    app.state_mut().state.doc.selection = Some(selected);
    app.run_steps(3);
    app.get_by_label("Gap 1 approved limit");
    app.get_by_label("Face #40 from source");
    app.render().unwrap().save(root.join("repair-record.png")).unwrap();

    let saved = root.join("repaired.dwprt");
    tool(&mut app.state_mut().state, "save", json!({"path":saved}));
    tool(&mut app.state_mut().state, "open", json!({"path":saved}));
    app.run_steps(3);
    let doc = &app.state().state.doc;
    let soliddraft_app::document::FeatureKind::ImportStep(import) = &doc.features[0].kind else {
        panic!("import feature")
    };
    assert_eq!(import.source, bytes, "the original file is kept as it was");
    assert_eq!(import.sha256, soliddraft_model::sha256_hex(&bytes));
    assert_eq!(import.record.as_ref(), Some(&record));
    assert_eq!(import.decisions.values().next().unwrap(), "accept:0.02:Ana Reviewer");
    assert!(doc.display_body().is_some(), "reopen verifies and rebuilds the repaired import");

    let exported = root.join("repaired-export.step");
    let answer = tool(&mut app.state_mut().state, "export_step", json!({"path":exported}));
    std::fs::write(root.join("repaired-export-report.json"), serde_json::to_vec_pretty(&answer).unwrap()).unwrap();
    assert!(!answer["report"].as_str().unwrap().contains("over the requested tolerance"));
    let mut reimported = Document::new("Re-export check", kernel.clone());
    let preview = soliddraft_app::import::Preview::read(&reimported, &exported).unwrap();
    assert!(preview.summary()["proposals"].as_array().unwrap().is_empty(), "the export is closed");
    soliddraft_app::import::commit(&mut reimported, &preview, &Default::default()).unwrap();
    let volume = kernel.mass_properties(reimported.display_body().unwrap()).unwrap().volume;
    assert!((volume - 10020.0).abs() < 1e-6, "{volume}");
    app.render().unwrap().save(root.join("repair-reopened.png")).unwrap();
}

#[test]
fn unsupported_step_file_gets_a_plain_message_without_editing_the_part() {
    let doc = soliddraft_app::new_document();
    let source = std::str::from_utf8(include_bytes!("../../../ci/fixtures/import/block-seam-4um.step")).unwrap();
    // Catallax reads B-spline surfaces since its PR 316; it does not read swept surfaces yet.
    let swept = source.replace("PLANE(", "SURFACE_OF_LINEAR_EXTRUSION(");
    let error = soliddraft_app::import::Preview::from_bytes(&doc, "swept.step", swept.into_bytes()).unwrap_err();
    assert_eq!(error, "Coming soon");
    assert!(doc.features.is_empty());
    assert_eq!(doc.revision(), 0);
}
