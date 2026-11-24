//! Extrude From on a sloping face, picked in the viewport the way a user picks it: the
//! extrusion starts on the slope, Merge result off makes it a new body, Flip turns it
//! round, and the part saves and opens again. A curved face starts one the same way, and
//! Up to next from it is refused in plain words. Frames land under
//! `target/ux-tools/extrude-from/` and `target/ux-tools/extrude-from-curved/`.

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
    body_and_square(|drawing| {
        drawing
            .polyline_loop(&[P2::new(-20.0, 0.0), P2::new(20.0, 0.0), P2::new(20.0, 10.0), P2::new(-20.0, 30.0)])
            .unwrap();
    })
}

/// A body 40 mm deep behind the Front plane from `profile`, and a 6 mm square on the Top
/// plane under its middle.
fn body_and_square(profile: impl FnOnce(&mut soliddraft_sketch::Sketch)) -> (Document, u64) {
    let mut doc = ribbon::blank();
    let front = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Front, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut drawing = soliddraft_sketch::Sketch::new();
    profile(&mut drawing);
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

/// A rod of radius 20 along y from 0 to -40, made from one circle on the Front plane,
/// and a 6 mm square on a plane 30 mm above the Top plane, 2 to 8 mm along x.
fn rod_and_square() -> (Document, u64) {
    let mut doc = ribbon::blank();
    let front = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Front, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut drawing = soliddraft_sketch::Sketch::new();
    drawing.circle_at(P2::new(0.0, 0.0), 40.0).unwrap();
    doc.apply(Command::EditSketch { id: front, sketch: drawing }).unwrap();
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(front), depth: 40.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let top = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 30.0 } })
        .unwrap()
        .unwrap();
    let mut drawing = soliddraft_sketch::Sketch::new();
    drawing.rectangle(P2::new(5.0, -20.0), 6.0, 6.0).unwrap();
    doc.apply(Command::EditSketch { id: top, sketch: drawing }).unwrap();
    (doc, top)
}

const ROD: f64 = 16000.0 * std::f64::consts::PI;

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

fn mass(doc: &soliddraft_model::Document, key: u32) -> soliddraft_kernel_api::MassProperties {
    let body = doc.live_bodies().into_iter().find(|(body, _)| *body == key).expect("the body is live").1;
    doc.kernel().mass_properties(body).unwrap()
}

fn volume_of(app: &App, key: u32) -> f64 {
    mass(app.state().state.doc.model(), key).volume
}

/// How high body `key`'s centre of mass is.
fn height_of(app: &App, key: u32) -> f64 {
    mass(app.state().state.doc.model(), key).centroid.z
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
        // The slope is 20 mm up over the square's middle: the boss reaches 5 mm above it.
        close(height_of(app, 1), 22.5, "the boss stands on the slope");
        // Flip turns it into the wedge from the same slope.
        run(app, AppAction::EditFeature(boss));
        extrude_params(app).flip = true;
        run(app, AppAction::CommitEditor);
        assert_eq!(app.state().state.doc.feature(boss).unwrap().status, FeatureStatus::Ok);
        close(volume_of(app, 1), 180.0, "the flipped body");
        close(height_of(app, 1), 17.5, "the flipped body hangs under the slope");
        pointer_to_chrome(app);
        case.frame(app, "flipped-new-body");

        // Saved and opened again, both bodies come back the same.
        let model = app.state().state.doc.model();
        let back = soliddraft_model::Document::from_bytes(&model.to_bytes().unwrap(), model.kernel().clone()).unwrap();
        assert_eq!(back.part_json(), model.part_json());
        assert_eq!(back.geometry_hash(), model.geometry_hash());
        close(mass(&back, 1).centroid.z, 17.5, "the reopened body is still flipped");
        case.note("saved and reopened with the same truth and geometry");
    });
}

#[test]
fn extrude_from_a_curved_face_picked_in_the_viewport() {
    let mut case = Case::new("extrude-from-curved");
    let (mut doc, square) = rod_and_square();
    doc.selection = Some(Selection::Feature(square));
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        rebuilt(app);
        close(volume_of(app, 0), ROD, "the rod");
        run(app, AppAction::StartExtrude);
        // From: Surface/Face/Plane, then a click on the rod's top at x = -10. Flipped, the
        // lines go down and first cross the top; Direction 2 goes 5 mm up from it.
        extrude_params(app).choices.from = StartKind::Surface(None);
        extrude_params(app).depth = 5.0;
        extrude_params(app).flip = true;
        extrude_params(app).choices.direction2 =
            Some(soliddraft_app::document::Direction2 { end: soliddraft_app::document::EndKind::Blind, depth: 5.0 });
        settle(app);
        let point = ux_tools::screen(app, glam::Vec3::new(-10.0, -20.0, 300f32.sqrt()));
        ribbon::render_steps::await_pick(app, point);
        ribbon::render_steps::rendered(app, |app| ribbon::click_at(app, point));
        settle(app);
        let picked = extrude_params(app).choices.from.clone();
        assert!(
            matches!(picked, StartKind::Surface(Some(soliddraft_model::PlaneRef::Face { .. }))),
            "the click names the curved top as the start: {picked:?}"
        );
        pointer_to_chrome(app);
        case.frame(app, "curved-top-picked");
        run(app, AppAction::CommitEditor);
        assert!(app.state().state.editor.is_none(), "the form closes on OK");
        let boss = app.state().state.doc.features.last().unwrap().id;
        assert_eq!(app.state().state.doc.feature(boss).unwrap().status, FeatureStatus::Ok);
        close(volume_of(app, 0), ROD + 180.0, "a 6 x 6 boss 5 mm out of the rod's top");
        pointer_to_chrome(app);
        case.frame(app, "boss-on-curved-top");

        // Up to next cannot start on a curved face: the form stays open with a plain reason.
        run(app, AppAction::EditFeature(boss));
        extrude_params(app).end = soliddraft_app::document::EndKind::UpToNext;
        run(app, AppAction::CommitEditor);
        let refusal = app.state().state.editor.as_ref().and_then(|e| e.refusal()).map(str::to_owned);
        let shown = soliddraft_app::geometry_message::for_user(refusal.as_deref().expect("the edit is refused"));
        assert!(shown.contains("cannot start on a curved face"), "the user reads {shown}");
        pointer_to_chrome(app);
        case.frame(app, "up-to-next-refused");
        run(app, AppAction::CancelEditor);
        close(volume_of(app, 0), ROD + 180.0, "the refused edit leaves the boss");

        // Unflipped, the lines go up and first cross the rod's underside: both 5 mm sweeps
        // from there stay inside the rod or hang under it.
        run(app, AppAction::EditFeature(boss));
        extrude_params(app).flip = false;
        run(app, AppAction::CommitEditor);
        assert_eq!(app.state().state.doc.feature(boss).unwrap().status, FeatureStatus::Ok);
        close(volume_of(app, 0), ROD + 180.0, "5 mm hang under the rod");
        // Merge result off: both sweeps become a body of their own.
        run(app, AppAction::EditFeature(boss));
        extrude_params(app).flip = true;
        extrude_params(app).merge = false;
        run(app, AppAction::CommitEditor);
        assert_eq!(app.state().state.doc.model().live_bodies().len(), 2, "a new body");
        close(volume_of(app, 0), ROD, "the rod alone");
        close(volume_of(app, 1), 360.0, "5 mm each side of the rod's top as its own body");
        pointer_to_chrome(app);
        case.frame(app, "curved-new-body");
        // Back into the rod and 7 mm up.
        run(app, AppAction::EditFeature(boss));
        extrude_params(app).merge = true;
        extrude_params(app).choices.direction2 =
            Some(soliddraft_app::document::Direction2 { end: soliddraft_app::document::EndKind::Blind, depth: 7.0 });
        run(app, AppAction::CommitEditor);
        close(volume_of(app, 0), ROD + 252.0, "the taller boss");

        // A later feature: a 4 mm square post from the Top plane up through the rod.
        let doc = &mut app.state_mut().state.doc;
        let top = doc
            .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
            .unwrap()
            .unwrap();
        let mut drawing = soliddraft_sketch::Sketch::new();
        drawing.rectangle(P2::new(-10.0, -20.0), 4.0, 4.0).unwrap();
        doc.apply(Command::EditSketch { id: top, sketch: drawing }).unwrap();
        doc.apply(Command::AddExtrude {
            params: ExtrudeParams { sketch: Some(top), depth: 45.0, ..Default::default() },
            via_assistant: false,
        })
        .unwrap();
        settle(app);
        rebuilt(app);
        let later = app.state().state.doc.features.last().unwrap().id;
        assert_eq!(app.state().state.doc.feature(later).unwrap().status, FeatureStatus::Ok);
        assert!(volume_of(app, 0) > ROD + 252.0, "the later post adds material above the rod");
        pointer_to_chrome(app);
        case.frame(app, "later-feature");

        // Saved and opened again, the part comes back the same; the web build opens it too.
        let model = app.state().state.doc.model();
        let bytes = model.to_bytes().unwrap();
        let back = soliddraft_model::Document::from_bytes(&bytes, model.kernel().clone()).unwrap();
        assert_eq!(back.part_json(), model.part_json());
        assert_eq!(back.geometry_hash(), model.geometry_hash());
        assert!(back.statuses().into_iter().all(|(_, status)| status.is_ok()), "every feature builds after reopening");
        std::fs::write(case.dir.join("extrude-from-curved.dwprt"), &bytes).unwrap();
        if let Some(dir) = std::env::var_os("SOLIDDRAFT_BROWSER_FIXTURES") {
            std::fs::write(std::path::PathBuf::from(dir).join("extrude-from-curved.dwprt"), &bytes).unwrap();
        }
        case.note("saved and reopened with the same truth and geometry");
    });
}
