//! Modelling on an imported STEP body in the real app: the reviewed import, then a fillet
//! on one of its edges and a shell through its top, each picked in the viewport and
//! committed with OK, then saved and reopened. The imported faces keep their
//! `Import1 · Face N` names throughout. Frames land under `target/ribbon-acceptance/ux-import-features/`.

use egui::{Key, Pos2, pos2};
use egui_kittest::kittest::Queryable;
use glam::Vec3;
use soliddraft_app::document::{Document, FeatureKind, FeatureStatus, Selection};
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

/// The isometric view, fitted, from the View tab, then back to the Solid tab.
fn isometric(app: &mut App) {
    ribbon::click_tab(app, "View");
    ribbon::click_ribbon(app, "Isometric");
    ribbon::click_ribbon(app, "Fit");
    settle(app);
    ribbon::click_tab(app, "Solid");
    pointer_off_model(app);
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

/// The selected face's name, as Properties shows it.
fn selected_face(app: &App) -> Option<String> {
    let doc = &app.state().state.doc;
    let Some(Selection::Face(id)) = doc.selection else { return None };
    let face = doc.named_face(id)?;
    let feature = doc.features.iter().find(|f| f.id == face.feature)?;
    Some(format!("{} · {}", feature.name, face.label))
}

fn all_ok(app: &App) {
    for f in &app.state().state.doc.features {
        assert_eq!(f.status, FeatureStatus::Ok, "{} is {:?}", f.name, f.status);
    }
}

/// Imports the block through the reviewed import's dialog and shows it isometric.
fn import(case: &ribbon::Case, app: &mut App) {
    let source = case.path("block.step");
    std::fs::write(&source, BLOCK).unwrap();
    app.state_mut().state.push(AppAction::ImportStep(Some(source)));
    app.run_steps(6);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Accept").click();
    settle(app);
    isometric(app);
    all_ok(app);
    assert!((volume(app) - 10004.0).abs() < 1e-6);
}

/// Saves, reopens and checks the part comes back whole, with its imported names.
fn save_and_reopen(case: &ribbon::Case, app: &mut App) {
    let before = volume(app);
    let saved = case.path("imported.dwprt");
    app.state_mut().state.push(AppAction::SaveAs(Some(saved.clone())));
    app.run_steps(4);
    app.state_mut().state.push(AppAction::Open(Some(saved)));
    settle(app);
    all_ok(app);
    assert!((volume(app) - before).abs() < 1e-9);
    isometric(app);
    // The right wall is still an imported face.
    pick(app, Vec3::new(40.0, 12.5, 3.0));
    let side = selected_face(app).expect("the right wall is a named face");
    assert!(side.starts_with("Import1 · Face "), "{side}");
}

fn names(app: &App) -> Vec<String> {
    app.state().state.doc.features.iter().map(|f| f.name.clone()).collect()
}

#[test]
fn fillet_and_chamfer_an_imported_block_in_the_app() {
    let case = ribbon::case("ux-import-features-edges");
    let mut app = ribbon::app(Document::new("Imported block", soliddraft_app::kernel()));
    case.run(&mut app, |app| {
        import(&case, app);
        case.frame(app, "1-imported");

        // The top face reads its imported name.
        pick(app, Vec3::new(30.0, 20.0, TOP));
        assert_eq!(selected_face(app).as_deref(), Some("Import1 · Face 6"));
        case.frame(app, "2-top-picked");
        app.key_press(Key::Escape);
        app.run_steps(4);

        // A 3 mm round on the front top edge removes (9 - 9 pi / 4) * 40.
        ribbon::click_ribbon(app, "Fillet");
        pick(app, Vec3::new(20.0, 0.0, TOP));
        ribbon::type_field(app, "Radius value", "3");
        settle(app);
        pointer_off_model(app);
        case.frame(app, "3-fillet-preview");
        ok(app);
        all_ok(app);
        let rounded = volume(app);
        let expected = 10004.0 - (9.0 - 9.0 * std::f64::consts::PI / 4.0) * 40.0;
        assert!((rounded - expected).abs() < 1e-3, "{rounded} vs {expected}");

        // A 2 mm chamfer on the back top edge removes 2 * 2 / 2 * 40 = 80 mm3.
        ribbon::click_ribbon(app, "Chamfer");
        pick(app, Vec3::new(20.0, 25.0, TOP));
        ribbon::type_field(app, "Distance value", "2");
        settle(app);
        ok(app);
        all_ok(app);
        assert!((rounded - volume(app) - 80.0).abs() < 1e-3, "{}", volume(app));
        assert_eq!(names(app), ["Import1", "Fillet1", "Chamfer1"]);
        pointer_off_model(app);
        case.frame(app, "4-fillet-and-chamfer");

        save_and_reopen(&case, app);
        case.frame(app, "5-reopened");
    });
}

#[test]
fn shell_an_imported_block_in_the_app() {
    let case = ribbon::case("ux-import-features-shell");
    let mut app = ribbon::app(Document::new("Imported block", soliddraft_app::kernel()));
    case.run(&mut app, |app| {
        import(&case, app);
        ribbon::click_ribbon(app, "Shell");
        pick(app, Vec3::new(30.0, 20.0, TOP));
        ribbon::type_field(app, "Wall thickness value", "1");
        settle(app);
        ok(app);
        all_ok(app);
        // 40 x 25 x 10.004 less a 38 x 23 x 9.004 hollow.
        let expected = 40.0 * 25.0 * 10.004 - 38.0 * 23.0 * 9.004;
        assert!((volume(app) - expected).abs() < 1e-6, "{}", volume(app));
        let shell = app.state().state.doc.features.last().unwrap();
        assert!(matches!(shell.kind, FeatureKind::Shell(_)));
        pointer_off_model(app);
        case.frame(app, "1-shell");

        save_and_reopen(&case, app);
        case.frame(app, "2-reopened");
    });
}
