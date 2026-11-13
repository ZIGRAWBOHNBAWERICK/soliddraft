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
    let mut reopened =
        soliddraft_app::file::open_bytes("fitted.dwprt", &saved, soliddraft_app::kernel(), None).unwrap();
    assert_eq!(reopened.features[0].status, FeatureStatus::Ok);
    let FeatureKind::Scan(scan) = &reopened.features[0].kind else { panic!("scan") };
    assert_eq!(scan.source.bytes, bytes);
    assert_eq!(scan.fit.as_ref().unwrap().part_map.len(), fit.part_map.len());
    assert_eq!(reopened.faces.len(), 6);
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
