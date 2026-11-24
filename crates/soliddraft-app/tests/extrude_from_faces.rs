//! Extrude From on a sloping face, picked in the viewport the way a user picks it: the
//! extrusion starts on the slope, Merge result off makes it a new body, Flip turns it
//! round, and the part saves and opens again. A curved face is kept and refused by name.
//! Frames land under `target/ux-tools/extrude-from/`.

use soliddraft_app::document::{
    Command, Document, ExtrudeParams, FeatureStatus, PlaneKind, Selection, SketchPlane, StartKind,
};
use soliddraft_app::state::{AppAction, EditorParams};
use soliddraft_model::PlaneRef;
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

/// A cylinder 20 mm across lying along Y behind the Front plane, its axis 10 mm up, and a
/// 6 mm square on the Top plane under its middle.
fn cylinder_and_square() -> (Document, u64) {
    body_and_square(|drawing| {
        drawing.circle_at(P2::new(0.0, 10.0), 20.0).unwrap();
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
fn a_curved_face_picked_in_the_viewport_is_kept_and_refused_by_name() {
    let mut case = Case::new("extrude-from-curved");
    let (mut doc, square) = cylinder_and_square();
    doc.selection = Some(Selection::Feature(square));
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        rebuilt(app);
        let solid = app.state().state.doc.model().body().expect("the cylinder");
        run(app, AppAction::StartExtrude);
        extrude_params(app).choices.from = StartKind::Surface(None);
        extrude_params(app).depth = 5.0;
        settle(app);
        // A point of the curved face turned toward the camera, halfway along the cylinder.
        let axis = glam::Vec3::new(0.0, -20.0, 10.0);
        let eye = app.state().state.viewport.camera.eye();
        let toward = glam::Vec3::new(eye.x - axis.x, 0.0, eye.z - axis.z).normalize();
        let point = ux_tools::screen(app, axis + toward * 10.0);
        ribbon::render_steps::await_pick(app, point);
        ribbon::render_steps::rendered(app, |app| ribbon::click_at(app, point));
        settle(app);
        let picked = extrude_params(app).choices.from.clone();
        let StartKind::Surface(Some(PlaneRef::Face { face })) = &picked else {
            panic!("the click names the curved face as the start: {picked:?}");
        };
        let model = app.state().state.doc.model();
        let id = model.resolve_face(face, solid).expect("the pick names a face of the cylinder");
        let topology = model.kernel().topology(solid).unwrap();
        assert!(topology.face(id).unwrap().normal.is_none(), "the picked face is the curved one");
        pointer_to_chrome(app);
        case.frame(app, "curved-face-picked");
        // OK refuses it by name, and the form keeps the pick.
        run(app, AppAction::CommitEditor);
        let refusal = app.state().state.editor.as_ref().expect("the form stays open").refusal().map(str::to_owned);
        let refusal = refusal.expect("the form says why");
        // The form shows it in plain words that still name the curved face.
        let shown = soliddraft_app::geometry_message::for_user(&refusal);
        assert!(shown.contains("curved face"), "the form shows: {shown}");
        assert_eq!(extrude_params(app).choices.from, picked, "the curved face stays picked");
        pointer_to_chrome(app);
        case.frame(app, "curved-face-refused");
        // The same extrusion kept as a row of the tree is saved and opened again: it keeps
        // the face and the refusal, for when the kernel can build it.
        let params = extrude_params(app).clone();
        run(app, AppAction::CancelEditor);
        run(app, AppAction::Command(Command::AddExtrude { params, via_assistant: false }));
        let doc = &mut app.state_mut().state.doc;
        let row = doc.features.last().expect("a row");
        assert!(
            matches!(&row.status, FeatureStatus::Error(why) if why.contains("curved face")),
            "the row is refused by name: {:?}",
            row.status
        );
        let file = case.dir.join("curved-start.dwprt");
        soliddraft_app::file::save(doc, &file).expect("the part saves");
        let kernel = doc.model().kernel().clone();
        let back = soliddraft_app::file::open(&file, kernel, None).expect("the part opens");
        assert_eq!(back.model().part_json(), doc.model().part_json(), "the truth round-trips");
        let model = back.model();
        let starts_on = |kind: &soliddraft_model::FeatureKind| {
            matches!(kind, soliddraft_model::FeatureKind::Extrude(e)
                if e.options.from == Some(soliddraft_model::ExtrudeStart::Surface { plane: PlaneRef::Face { face: face.clone() } }))
        };
        let kept = model.features().iter().find(|f| starts_on(&f.kind)).expect("the curved start is kept");
        match model.status(kept.id) {
            Some(soliddraft_model::FeatureStatus::Failed(why)) => assert!(why.contains("curved face"), "{why}"),
            other => panic!("the reopened extrusion is refused by name: {other:?}"),
        }
        case.note("a curved start face picked in the viewport is refused by name and kept through save and reopen");
    });
}
