//! A body built within a stated tolerance in the real app: it draws, a face picks in the
//! viewport, and the tree and Details call it approximate in plain words. Frames land
//! under `target/ribbon-acceptance/ux-numerical-body/`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::Pos2;
use egui_kittest::kittest::{By, NodeT, Queryable};
use glam::Vec3;
use soliddraft_app::document::{Command, Document, ExtrudeParams, ProfileKind, Selection};
use soliddraft_kernel_api::OpKind;
use soliddraft_kernel_catallax::CatallaxKernel;

#[path = "support/ui.rs"]
mod ribbon;

use ribbon::App;

const HANG_GUARD: Duration = Duration::from_secs(600);
const BODY: &str = "Approximate solid body";
const WORDS: &str = "Approximate, built within 0.01 mm. Its display, measurements and mass are estimates";

/// A 10 x 8 x 6 mm block whose extrusion Catallax returns as a numerical body.
fn block() -> Document {
    let kernel = Arc::new(CatallaxKernel::with_numerical_stand_in(&[OpKind::Extrude]));
    let mut doc = Document::new("Approximate block", kernel);
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams {
            profile: ProfileKind::Rectangle { width: 10.0, height: 8.0 },
            depth: 6.0,
            ..Default::default()
        },
        via_assistant: false,
    })
    .unwrap()
    .unwrap();
    doc
}

fn settle(app: &mut App) {
    let start = Instant::now();
    while app.state().state.doc.proof_pending() {
        assert!(start.elapsed() < HANG_GUARD, "still rebuilding");
        app.step();
    }
    app.run_steps(20);
}

/// Every label and value the accessibility tree shows: a label's text is its value.
fn texts(app: &App) -> Vec<String> {
    app.query_all(By::new())
        .flat_map(|node| {
            let node = node.accesskit_node();
            [node.label(), node.value()]
        })
        .flatten()
        .collect()
}

/// Opens the Details panel from its button, unless it is open.
fn open_details(app: &mut App) {
    if !app.state().state.options_details {
        app.get_by_label("Details").click();
        app.run_steps(8);
    }
}

fn project(app: &App, world: Vec3) -> Pos2 {
    let state = &app.state().state;
    state.viewport.camera.project(world, state.viewport.rect).expect("in front of the camera")
}

#[test]
fn an_approximate_body_draws_picks_and_says_what_it_is() {
    let case = ribbon::case("ux-numerical-body");
    let mut app = ribbon::app(block());
    case.run(&mut app, |app| {
        ribbon::switch_workspace(app, "view");
        ribbon::run_command(app, "view_isometric");
        ribbon::run_command(app, "zoom_fit");
        ribbon::switch_workspace(app, "solid");
        settle(app);
        assert!(app.state().state.doc.proof_state().current_proved_display());
        case.frame(app, "01-approximate-block");

        // The tree names the body; its row selects it and Details says what it is.
        let row = app
            .query_all(By::new())
            .find(|node| {
                let node = node.accesskit_node();
                node.label().as_deref() == Some(BODY) || node.value().as_deref() == Some(BODY)
            })
            .expect("the tree names the approximate body")
            .rect();
        ribbon::click_at(app, row.center());
        settle(app);
        assert_eq!(app.state().state.doc.selection, Some(Selection::Body));
        open_details(app);
        let shown = texts(app);
        assert!(!shown.iter().any(|t| t.starts_with("Boss-Extrude1 status:")), "no light on a built body: {shown:?}");
        assert!(shown.iter().any(|t| t == WORDS), "Details does not say the body is approximate: {shown:?}");
        assert!(!shown.iter().any(|t| t.to_lowercase().contains("numerical")), "{shown:?}");
        case.frame(app, "02-body-details");

        // The top face picks in the viewport, by its own face of the numerical body.
        let top = project(app, Vec3::new(0.0, 0.0, 6.0));
        ribbon::pick_at(app, top);
        settle(app);
        let Some(Selection::Face(face)) = app.state().state.doc.selection else {
            panic!("the top face did not pick: {:?}", app.state().state.doc.selection)
        };
        let named = app.state().state.doc.named_face(face).expect("the picked face is named").clone();
        assert!((named.centroid.z - 6.0).abs() < 1e-6, "{named:?}");
        open_details(app);
        let shown = texts(app);
        assert!(shown.iter().any(|t| t == "Approximate, built within 0.01 mm"), "{shown:?}");
        case.frame(app, "03-face-details");
    });
}
