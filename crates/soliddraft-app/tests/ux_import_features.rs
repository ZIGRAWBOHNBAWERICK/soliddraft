//! Modelling on an imported STEP body in the real app: the reviewed import, then a fillet
//! on one of its edges and a shell through its top, each picked in the viewport and
//! committed with OK, then saved and reopened. The picked faces read `Import1 · Face N`
//! throughout. Frames land under `target/ribbon-acceptance/ux-import-features/`.

use egui::{Key, Pos2, pos2};
use egui_kittest::kittest::{By, NodeT, Queryable};
use glam::Vec3;
use soliddraft_app::document::{Document, FeatureKind, FeatureStatus};
use soliddraft_app::state::AppAction;
use std::time::{Duration, Instant};

#[path = "support/ribbon.rs"]
mod ribbon;

use ribbon::App;

/// A 40 x 25 x 10.004 mm block from the origin.
const BLOCK: &[u8] = include_bytes!("../../../ci/fixtures/import/block-seam-4um.step");
const TOP: f32 = 10.004;
const HANG_GUARD: Duration = Duration::from_secs(600);

fn settle(app: &mut App) {
    let start = Instant::now();
    while app.state().state.doc.proof_pending() {
        assert!(start.elapsed() < HANG_GUARD, "still rebuilding");
        app.step();
    }
    app.run_steps(20);
}

fn project(app: &App, world: Vec3) -> Pos2 {
    let state = &app.state().state;
    state.viewport.camera.project(world, state.viewport.rect).expect("in front of the camera")
}

fn pick(app: &mut App, world: Vec3) {
    let at = project(app, world);
    assert!(app.state().state.viewport.rect.contains(at), "{world} is on screen");
    ribbon::pick_at(app, at);
    app.run_steps(2);
}

fn ok(app: &mut App) {
    let rect = ribbon::button_rect(app, "OK (Enter)", ribbon::window(app));
    ribbon::click_at(app, rect.center());
    settle(app);
}

fn pointer_off_model(app: &mut App) {
    let rect = app.state().state.viewport.rect;
    app.hover_at(pos2(rect.center().x, 4.0));
    app.run_steps(4);
}

fn volume(app: &App) -> f64 {
    let doc = &app.state().state.doc;
    doc.kernel().mass_properties(doc.display_body().unwrap()).unwrap().volume
}

/// Every text the window shows, labels and values alike.
fn texts(app: &App) -> Vec<String> {
    app.query_all(By::new())
        .flat_map(|node| [node.accesskit_node().label(), node.accesskit_node().value()])
        .flatten()
        .collect()
}

fn all_ok(app: &App) {
    for f in &app.state().state.doc.features {
        assert_eq!(f.status, FeatureStatus::Ok, "{} is {:?}", f.name, f.status);
    }
}

#[test]
fn fillet_and_shell_an_imported_block_in_the_app() {
    let case = ribbon::case("ux-import-features");
    let source = case.path("block.step");
    std::fs::write(&source, BLOCK).unwrap();
    let mut app = ribbon::app(Document::new("Imported block", soliddraft_app::kernel()));
    case.run(&mut app, |app| {
        // The reviewed import, through its own dialog.
        app.state_mut().state.push(AppAction::ImportStep(Some(source.clone())));
        app.run_steps(6);
        app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
        settle(app);
        ribbon::click_tab(app, "View");
        ribbon::click_ribbon(app, "Isometric");
        ribbon::click_ribbon(app, "Fit");
        settle(app);
        ribbon::click_tab(app, "Solid");
        pointer_off_model(app);
        all_ok(app);
        let block = volume(app);
        assert!((block - 10004.0).abs() < 1e-6, "{block}");
        case.frame(app, "1-imported");

        // A fillet on the front top edge.
        ribbon::click_ribbon(app, "Fillet");
        pick(app, Vec3::new(20.0, 0.0, TOP));
        ribbon::type_field(app, "Radius value", "3");
        settle(app);
        pointer_off_model(app);
        case.frame(app, "2-fillet-preview");
        ok(app);
        all_ok(app);
        let rounded = volume(app);
        // A 3 mm round along 40 mm removes (9 - 9 pi / 4) * 40.
        let expected = block - (9.0 - 9.0 * std::f64::consts::PI / 4.0) * 40.0;
        assert!((rounded - expected).abs() < 1e-3, "{rounded} vs {expected}");
        let names: Vec<String> = app.state().state.doc.features.iter().map(|f| f.name.clone()).collect();
        assert_eq!(names, ["Import1", "Fillet1"]);
        pointer_off_model(app);
        case.frame(app, "3-fillet");

        // A shell open at the top face, which keeps its imported name.
        pick(app, Vec3::new(30.0, 20.0, TOP));
        let selected = texts(app).into_iter().find(|t| t.contains("Import1 · Face "));
        assert!(selected.is_some(), "the picked top reads Import1 · Face N");
        app.key_press(Key::Escape);
        app.run_steps(4);
        ribbon::click_ribbon(app, "Shell");
        pick(app, Vec3::new(30.0, 20.0, TOP));
        ribbon::type_field(app, "Wall thickness value", "1");
        settle(app);
        ok(app);
        all_ok(app);
        let shelled = volume(app);
        assert!(shelled < rounded / 2.0, "{shelled}");
        assert!(!texts(app).iter().any(|t| t.contains("oming soon")), "nothing reads Coming soon");
        let kinds: Vec<bool> =
            app.state().state.doc.features.iter().map(|f| matches!(f.kind, FeatureKind::Shell(_))).collect();
        assert_eq!(kinds, [false, false, true]);
        pointer_off_model(app);
        case.frame(app, "4-shell");

        // Save and reopen.
        let saved = case.path("imported.dwprt");
        app.state_mut().state.push(AppAction::SaveAs(Some(saved.clone())));
        app.run_steps(4);
        app.state_mut().state.push(AppAction::Open(Some(saved)));
        settle(app);
        all_ok(app);
        assert!((volume(app) - shelled).abs() < 1e-9);
        pointer_off_model(app);
        case.frame(app, "5-reopened");
    });
}
