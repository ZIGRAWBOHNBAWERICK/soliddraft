//! Scan review, fitting and deviation maps through the real application.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui_kittest::kittest::Queryable;
use soliddraft_app::{
    document::{Document, FeatureKind, FeatureStatus},
    state::AppAction,
};
use std::time::{Duration, Instant};

fn cloud() -> Vec<u8> {
    // Independent samples of six planes, with small normal noise. No part recipe is input.
    let mut seed = 41_u64;
    let mut random = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (seed >> 11) as f64 / ((1_u64 << 53) as f64)
    };
    let size = [6.0, 5.0, 4.0];
    let mut obj = String::new();
    for axis in 0..3 {
        for side in [0.0, 1.0] {
            for _ in 0..1000 {
                let mut p = [random() * size[0], random() * size[1], random() * size[2]];
                p[axis] = side * size[axis] + (random() - 0.5) * 0.004;
                obj.push_str(&format!("v {:.9} {:.9} {:.9}\n", p[0], p[1], p[2]));
            }
        }
    }
    obj.into_bytes()
}

#[test]
fn open_scan_review_cancel_accept_save_reopen_and_fit_refusal_leave_reference_intact() {
    let case = ribbon::case("scan-reference");
    let path = case.path("open.obj");
    let bytes = b"v 0 0 0\nv 10 0 0\nv 0 10 0\nv 5 5 3\nf 1 2 3\n";
    std::fs::write(&path, bytes).unwrap();
    let mut app = ribbon::app(Document::new("Reference", soliddraft_app::kernel()));
    app.state_mut().state.push(AppAction::ImportScan(Some(path.clone())));
    app.run_steps(4);
    app.get_by_label("3 open edges · 0 reversed edge pairs · 0 nonmanifold edges");
    assert!(app.state().state.doc.features.is_empty());
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel").click();
    app.run_steps(3);
    assert!(app.state().state.doc.features.is_empty());
    app.state_mut().state.push(AppAction::ImportScan(Some(path.clone())));
    app.run_steps(3);
    app.state_mut().state.doc = Document::new("Another part", soliddraft_app::kernel());
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept reference").click();
    app.run_steps(3);
    assert!(app.state().state.doc.features.is_empty());
    app.state_mut().state.push(AppAction::ImportScan(Some(path)));
    app.run_steps(3);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept reference").click();
    app.run_steps(5);
    assert_eq!(app.state().state.doc.features[0].status, FeatureStatus::Ok);
    assert!(app.state().state.doc.display_body().is_none());
    case.frame(&mut app, "reference");
    let saved = app.state().state.doc.model().to_bytes().unwrap();
    let reopened = soliddraft_app::file::open_bytes("scan.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
    let FeatureKind::Scan(scan) = &reopened.features[0].kind else { panic!("scan") };
    assert_eq!(scan.source.bytes, bytes);
    assert!(scan.fit.is_none());
    assert!(reopened.display_body().is_none());
    app.state_mut().state.scan.open = false;
    app.run_steps(2);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Edit feature").click();
    app.run_steps(3);
    assert!(app.state().state.scan.open, "editing a scan must reopen its workspace");
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Fit scan to part").click();
    let start = Instant::now();
    loop {
        app.run_steps(2);
        if app.query_all_by_label("Fit scan to part needs at least 100 points.").next().is_some() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(app.state().state.doc.model().to_bytes().unwrap(), saved);
    app.state_mut().state.doc.undo().unwrap();
    assert!(app.state().state.doc.features.is_empty());
    app.state_mut().state.doc.redo().unwrap();
    assert_eq!(app.state().state.doc.features.len(), 1);
}

#[test]
fn fit_scan_reviews_both_numbers_and_maps_then_reopens_an_editable_solid() {
    use soliddraft_app::document::{Command, ExtrudeParams, PlaneKind, ProfileKind};
    let case = ribbon::case("scan-fit");
    let path = case.path("cloud.obj");
    let bytes = cloud();
    std::fs::write(&path, &bytes).unwrap();
    let mut app = ribbon::app(Document::new("Fit", soliddraft_app::kernel()));
    app.state_mut().state.frame_layout = soliddraft_app::settings::FrameLayout::EditSuite;
    app.run_steps(3);
    app.state_mut().state.push(AppAction::ImportScan(Some(path)));
    app.run_steps(4);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept reference").click();
    app.run_steps(4);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Fit scan to part").click();
    let start = Instant::now();
    loop {
        app.run_steps(2);
        if app.query_all_by_label("Accept fitted part").next().is_some() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(120), "fit did not return a part");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(app.state().state.doc.display_body().is_none());
    app.get_by_label("Sampled distances, not a bound on the whole surface.");
    app.get_by_label("Scan to part map").click();
    app.run_steps(3);
    case.frame(&mut app, "scan-to-part");
    app.get_by_label("Part to scan map").click();
    app.run_steps(3);
    case.frame(&mut app, "part-to-scan");
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept fitted part").click();
    app.run_steps(6);
    let doc = &app.state().state.doc;
    assert_eq!(doc.features[0].status, FeatureStatus::Ok);
    let body = doc.display_body().expect("solid");
    let volume = doc.kernel().mass_properties(body).unwrap().volume;
    assert!((volume - 120.0).abs() < 0.2, "{volume}");
    let FeatureKind::Scan(scan) = &doc.features[0].kind else { panic!("scan") };
    let fit = scan.fit.as_ref().unwrap();
    assert_eq!(fit.scan_to_part.samples, 6000);
    assert_eq!(fit.scan_map.len(), 6000);
    assert_eq!(fit.part_map.len(), fit.part_to_scan.samples);
    assert!(fit.scan_to_part.max_mm < 0.02);
    assert!(fit.part_to_scan.max_mm > fit.scan_to_part.max_mm);
    assert!(fit.chord_mm <= 0.001);
    let saved = doc.model().to_bytes().unwrap();
    let part_sample_count = fit.part_map.len();
    let work = doc.proof_report().features[0].work_used.expect("fitted scan build work");
    let export = app.state().state.export_part(soliddraft_exchange::Format::Step).unwrap();
    let path = case.path("fitted.step");
    export.write_with(&path, |path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())).unwrap();
    let proof: serde_json::Value =
        serde_json::from_slice(&std::fs::read(soliddraft_exchange::proof::sidecar_path(&path, "json")).unwrap())
            .unwrap();
    assert_eq!(proof["features"][0]["work_used"], work);
    case.frame(&mut app, "accepted-fit");
    let mut reopened =
        soliddraft_app::file::open_bytes("fitted.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
    assert_eq!(reopened.features[0].status, FeatureStatus::Ok);
    assert!(reopened.proof_report().features[0].work_used.is_some());
    let FeatureKind::Scan(scan) = &reopened.features[0].kind else { panic!("scan") };
    assert_eq!(scan.source.bytes, bytes);
    assert_eq!(scan.fit.as_ref().unwrap().part_map.len(), part_sample_count);
    assert_eq!(reopened.faces.len(), 6);
    let face = reopened.face_ref(reopened.faces[0].id).expect("named fitted face");
    reopened
        .apply(Command::AddFeature(FeatureKind::ReferencePlane(soliddraft_model::SketchPlane::Face { face })))
        .unwrap();
    let saved = reopened.model().to_bytes().unwrap();
    let kernel = soliddraft_app::kernel();
    let mut unrelated = Document::new("Unrelated", kernel.clone());
    soliddraft_app::populate_demo(&mut unrelated);
    reopened = soliddraft_app::file::open_bytes("named-fit.dwprt", &saved, kernel, None).unwrap();
    assert!(reopened.features.iter().all(|f| f.status == FeatureStatus::Ok));
    reopened
        .apply(Command::AddExtrude {
            params: ExtrudeParams {
                sketch: None,
                profile: ProfileKind::Rectangle { width: 1.0, height: 1.0 },
                plane: PlaneKind::Top,
                depth: 1.0,
                flip: false,
                end: Default::default(),
            },
            via_assistant: false,
        })
        .unwrap();
    assert!(reopened.features.iter().all(|f| f.status == FeatureStatus::Ok));
    let export = reopened.kernel().export_step(reopened.display_body().unwrap(), &Default::default()).unwrap();
    assert!(export.report.within_tolerance);
    app.state_mut().state.doc.undo().unwrap();
    assert!(app.state().state.doc.display_body().is_none());
    app.state_mut().state.doc.redo().unwrap();
    assert!(app.state().state.doc.display_body().is_some());
}

fn open_other_document(app: &mut ribbon::App, case: &ribbon::Case, assembly: bool) {
    if assembly {
        app.state_mut().state.push(AppAction::Assembly(soliddraft_app::assembly::AssemblyAction::New));
    } else {
        let path = case.path("other.dwdrw");
        soliddraft_app::drawing::DrawingDoc::new(soliddraft_drawing::fixture::bracket_a3())
            .save(&path, "catallax")
            .unwrap();
        app.state_mut().state.push(AppAction::Drawing(soliddraft_app::drawing::DrawingAction::OpenFile(path)));
    }
    app.run_steps(4);
    assert!(if assembly { app.state().state.assembly.is_active() } else { app.state().state.drawing.is_active() });
}

fn return_to_part(app: &mut ribbon::App, assembly: bool) {
    app.state_mut().state.push(if assembly {
        AppAction::Assembly(soliddraft_app::assembly::AssemblyAction::ShowPart)
    } else {
        AppAction::Drawing(soliddraft_app::drawing::DrawingAction::ShowPart)
    });
    app.run_steps(4);
}

fn reference_review_on_other_document(assembly: bool) {
    let case = ribbon::case(if assembly { "scan-assembly-isolation" } else { "scan-drawing-isolation" });
    let path = case.path("reference.obj");
    std::fs::write(&path, b"v 0 0 0\nv 10 0 0\nv 0 10 0\nf 1 2 3\n").unwrap();
    let mut app = ribbon::app(Document::new("Retained part", soliddraft_app::kernel()));
    app.state_mut().state.push(AppAction::ImportScan(Some(path.clone())));
    app.run_steps(4);
    app.get_by_label("Accept reference");
    let before = app.state().state.doc.model().to_bytes().unwrap();
    open_other_document(&mut app, &case, assembly);
    if let Some(accept) = app.query_all_by_label("Accept reference").next() {
        accept.click();
    }
    app.run_steps(3);
    assert!(
        app.state().state.doc.model().to_bytes().unwrap() == before,
        "accepting a scan must not edit an inactive part"
    );
    assert!(app.query_all_by_label("Accept reference").next().is_none());
    for id in ["import_scan", "fit_scan", "scan_workspace"] {
        let command = soliddraft_app::commands::get(id).unwrap();
        assert!(!(command.shown)(&app.state().state), "{id} must be hidden outside the part");
        assert!(command.action_now(&app.state().state).is_none(), "{id} must be disabled outside the part");
    }
    app.state_mut().state.push(AppAction::ImportScan(Some(path)));
    app.state_mut().state.push(AppAction::ShowScan);
    app.run_steps(3);
    assert!(app.query_all_by_label("Accept reference").next().is_none());
    return_to_part(&mut app, assembly);
    app.get_by_label("Accept reference").click();
    app.run_steps(4);
    assert_eq!(app.state().state.doc.features.len(), 1);
}

#[test]
fn review_reference_cannot_edit_the_part_from_a_new_assembly() {
    reference_review_on_other_document(true);
}

#[test]
fn review_reference_cannot_edit_the_part_from_an_open_drawing() {
    reference_review_on_other_document(false);
}

#[test]
fn review_fitted_part_cannot_be_accepted_from_a_drawing() {
    let case = ribbon::case("scan-fit-drawing-isolation");
    let path = case.path("cloud.obj");
    std::fs::write(&path, cloud()).unwrap();
    let mut app = ribbon::app(Document::new("Retained fit", soliddraft_app::kernel()));
    app.state_mut().state.push(AppAction::ImportScan(Some(path)));
    app.run_steps(4);
    app.get_by_label("Accept reference").click();
    app.run_steps(4);
    app.get_by_label("Fit scan to part").click();
    let start = Instant::now();
    loop {
        app.run_steps(2);
        if app.query_all_by_label("Accept fitted part").next().is_some() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(120));
        std::thread::sleep(Duration::from_millis(10));
    }
    let before = app.state().state.doc.model().to_bytes().unwrap();
    open_other_document(&mut app, &case, false);
    if let Some(accept) = app.query_all_by_label("Accept fitted part").next() {
        accept.click();
    }
    app.run_steps(3);
    assert!(app.state().state.doc.model().to_bytes().unwrap() == before, "a fit must not replace an inactive part");
    assert!(app.query_all_by_label("Accept fitted part").next().is_none());
    return_to_part(&mut app, false);
    app.get_by_label("Accept fitted part").click();
    app.run_steps(4);
    assert!(app.state().state.doc.display_body().is_some());
}

#[test]
fn review_counting_preserves_scan_admission_fit_and_saved_replay() {
    use soliddraft_kernel_api::{
        Kernel,
        counting::{Counting, Work},
        scan::ScanSource,
    };
    use std::sync::Arc;
    let mut bytes = cloud();
    bytes.extend_from_slice(b"f 1 2 3\n");
    let source = ScanSource { name: "mixed-scan.obj".into(), sha256: soliddraft_model::sha256_hex(&bytes), bytes };
    let raw = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
    let counting = Arc::new(Counting::new(raw.clone()));
    let reference = counting.scan_reference(&source).expect("Counting must forward scan admission");
    assert_eq!(reference.points, raw.scan_reference(&source).unwrap().points);
    let fit = counting.fit_scan(&source).expect("Counting must forward fitting");
    assert_eq!(fit.scan_map.len(), 6000);
    assert_eq!(fit.scan_to_part.samples, 6003, "statistics also include triangle-interior probes");
    assert_eq!(fit.part_map.len(), fit.part_to_scan.samples);
    assert!(counting.take().is_empty(), "read and fit do not publish a body");
    let mut doc = Document::new("Counted fit", counting.clone());
    doc.apply(soliddraft_app::document::Command::AddFeature(FeatureKind::Scan(soliddraft_model::ScanFeature {
        source,
        fit: Some(fit),
    })))
    .unwrap();
    assert_eq!(doc.features[0].status, FeatureStatus::Ok);
    assert_eq!(
        counting.take().into_iter().filter(|work| matches!(work, Work::Operation(_))).collect::<Vec<_>>(),
        vec![Work::Operation("F1.scan".into())]
    );
    assert!((counting.mass_properties(doc.display_body().unwrap()).unwrap().volume - 120.0).abs() < 0.2);
    let saved = doc.model().to_bytes().unwrap();
    let fresh = Arc::new(Counting::new(soliddraft_app::kernel()));
    let reopened = soliddraft_app::file::open_bytes("counted-fit.dwprt", &saved, fresh.clone(), None).unwrap();
    assert_eq!(reopened.features[0].status, FeatureStatus::Ok);
    assert!(reopened.display_body().is_some());
    assert_eq!(
        fresh.take().into_iter().filter(|work| matches!(work, Work::Operation(_))).collect::<Vec<_>>(),
        vec![Work::Operation("F1.scan".into())]
    );
}
