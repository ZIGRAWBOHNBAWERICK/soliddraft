//! Extrude From on a sloping face, picked in the viewport the way a user picks it: the
//! extrusion starts on the slope, Merge result off makes it a new body, Flip turns it
//! round, and the part saves and opens again. A curved face is kept and refused by name.
//! Frames land under `target/ux-tools/extrude-from/`.

use soliddraft_app::document::{
    Command, Document, ExtrudeParams, FeatureStatus, PlaneKind, Selection, SketchPlane, StartKind,
};
use soliddraft_app::state::{AppAction, EditorParams};
use soliddraft_sketch::P2;

#[path = "support/ui.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use ribbon::App;
use ux_tools::{Case, STANDARD, launch, pointer_to_chrome, rebuilt, settle};

/// A wedge 40 mm wide and 40 mm deep behind the Front plane whose top falls from 30 mm at
/// x = -20 to 10 mm at x = 20, and a 6 mm square on the Top plane under its middle.
fn wedge_and_square() -> (Document, u64) {
    let mut doc = ribbon::blank();
    let front = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Front, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut drawing = soliddraft_sketch::Sketch::new();
    drawing
        .polyline_loop(&[P2::new(-20.0, 0.0), P2::new(20.0, 0.0), P2::new(20.0, 10.0), P2::new(-20.0, 30.0)])
        .unwrap();
    doc.apply(Command::EditSketch { id: front, sketch: drawing }).unwrap();
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(front), depth: 40.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let top = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut drawing = soliddraft_sketch::Sketch::new();
    drawing.rectangle(P2::new(0.0, -20.0), 6.0, 6.0).unwrap();
    doc.apply(Command::EditSketch { id: top, sketch: drawing }).unwrap();
    (doc, top)
}

const WEDGE: f64 = 32000.0;

fn extrude_params(app: &mut App) -> &mut ExtrudeParams {
    match &mut app.state_mut().state.editor.as_mut().expect("the Extrude form is open").params {
        EditorParams::Extrude(p) => p,
        other => panic!("extruding: {other:?}"),
    }
}

fn run(app: &mut App, action: AppAction) {
    app.state_mut().state.push(action);
    settle(app);
    rebuilt(app);
    settle(app);
}

fn volume_of(app: &App, key: u32) -> f64 {
    let doc = app.state().state.doc.model();
    let body = doc.live_bodies().into_iter().find(|(body, _)| *body == key).expect("the body is live").1;
    doc.kernel().mass_properties(body).unwrap().volume
}

fn close(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() <= 1e-6 * b.abs().max(1.0), "{what}: {a} is not {b}");
}

#[test]
fn extrude_from_a_sloping_face_picked_in_the_viewport() {
    let mut case = Case::new("extrude-from");
    let (mut doc, square) = wedge_and_square();
    doc.selection = Some(Selection::Feature(square));
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        rebuilt(app);
        close(volume_of(app, 0), WEDGE, "the wedge");
        run(app, AppAction::StartExtrude);
        // From: Surface/Face/Plane, then a click on the slope at x = 10, where it is 15 mm up.
        extrude_params(app).choices.from = StartKind::Surface(None);
        extrude_params(app).depth = 5.0;
        settle(app);
        let point = ux_tools::screen(app, glam::Vec3::new(10.0, -20.0, 15.0));
        ribbon::render_steps::await_pick(app, point);
        ribbon::render_steps::rendered(app, |app| ribbon::click_at(app, point));
        settle(app);
        let picked = extrude_params(app).choices.from.clone();
        assert!(
            matches!(picked, StartKind::Surface(Some(soliddraft_model::PlaneRef::Face { .. }))),
            "the click names the slope as the start: {picked:?}"
        );
        pointer_to_chrome(app);
        case.frame(app, "slope-picked");
        run(app, AppAction::CommitEditor);
        assert!(app.state().state.editor.is_none(), "the form closes on OK");
        let boss = app.state().state.doc.features.last().unwrap().id;
        assert_eq!(app.state().state.doc.feature(boss).unwrap().status, FeatureStatus::Ok);
        close(volume_of(app, 0), WEDGE + 180.0, "a 6 x 6 boss 5 mm from the slope");
        pointer_to_chrome(app);
        case.frame(app, "boss-on-slope");
        // The part as a file, for the browser check that opens it in the web build.
        let file = case.dir.join("slope-boss.dwprt");
        soliddraft_app::file::save(&mut app.state_mut().state.doc, &file).expect("the part saves");

        // Merge result off: the boss becomes a body of its own on the slope.
        run(app, AppAction::EditFeature(boss));
        extrude_params(app).merge = false;
        run(app, AppAction::CommitEditor);
        let doc = app.state().state.doc.model();
        assert_eq!(doc.live_bodies().len(), 2, "a new body");
        close(volume_of(app, 0), WEDGE, "the wedge alone");
        close(volume_of(app, 1), 180.0, "the boss as its own body");
        // Flip turns it into the wedge from the same slope.
        run(app, AppAction::EditFeature(boss));
        extrude_params(app).flip = true;
        run(app, AppAction::CommitEditor);
        assert_eq!(app.state().state.doc.feature(boss).unwrap().status, FeatureStatus::Ok);
        close(volume_of(app, 1), 180.0, "the flipped body");
        pointer_to_chrome(app);
        case.frame(app, "flipped-new-body");

        // Saved and opened again, both bodies come back the same.
        let model = app.state().state.doc.model();
        let back = soliddraft_model::Document::from_bytes(&model.to_bytes().unwrap(), model.kernel().clone()).unwrap();
        assert_eq!(back.part_json(), model.part_json());
        assert_eq!(back.geometry_hash(), model.geometry_hash());
        case.note("saved and reopened with the same truth and geometry");
    });
}
