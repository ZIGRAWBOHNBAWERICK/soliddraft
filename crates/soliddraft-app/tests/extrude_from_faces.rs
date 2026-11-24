//! Extrude From on a sloping face, picked in the viewport the way a user picks it: the
//! extrusion starts on the slope, Merge result off makes it a new body, Flip turns it
//! round, and the part saves and opens again. A curved face starts one the same way, and
//! Up to next from it is refused in plain words. Up to face stops on the same curved face,
//! and a curved start that no longer resolves is repaired by picking the face again. Frames
//! land under `target/ux-tools/<case>/`.

use soliddraft_app::document::{
    Command, Document, EndKind, ExtrudeParams, FeatureStatus, PlaneKind, Selection, SketchPlane, StartKind,
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
        case.note("saved and reopened with the same truth and geometry");
    });
}

/// The rod's round top in a part's truth: every face fingerprint of the last feature that
/// names a cylinder now names a sphere, so the reference no longer resolves, as when the
/// face it named is gone.
fn lose_curved_reference(value: &mut serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => {
            let mut lost = 0;
            if let Some(surface) = map.get_mut("surface")
                && surface == "cylinder"
            {
                *surface = "sphere".into();
                lost += 1;
            }
            lost + map.values_mut().map(lose_curved_reference).sum::<usize>()
        }
        serde_json::Value::Array(items) => items.iter_mut().map(lose_curved_reference).sum(),
        _ => 0,
    }
}

/// The area under the rod's top between x = 2 and x = 8, per millimetre along y.
fn under_rod() -> f64 {
    let f = |x: f64| (x * (400.0 - x * x).sqrt() + 400.0 * (x / 20.0).asin()) / 2.0;
    f(8.0) - f(2.0)
}

#[test]
fn up_to_a_curved_face_picked_in_the_viewport() {
    let mut case = Case::new("extrude-up-to-curved");
    let (mut doc, square) = rod_and_square();
    doc.selection = Some(Selection::Feature(square));
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        rebuilt(app);
        run(app, AppAction::StartExtrude);
        // Down from 30 mm, Up to face, then a click on the rod's top at x = -10.
        extrude_params(app).flip = true;
        extrude_params(app).end = EndKind::UpToFace(None);
        settle(app);
        let point = ux_tools::screen(app, glam::Vec3::new(-10.0, -20.0, 300f32.sqrt()));
        ribbon::render_steps::await_pick(app, point);
        ribbon::render_steps::rendered(app, |app| ribbon::click_at(app, point));
        settle(app);
        let end = extrude_params(app).end.clone();
        assert!(matches!(&end, EndKind::UpToFace(Some(_))), "the click names the curved top as the limit: {end:?}");
        pointer_to_chrome(app);
        case.frame(app, "curved-limit-picked");
        run(app, AppAction::CommitEditor);
        assert!(app.state().state.editor.is_none(), "the form closes on OK");
        let column = app.state().state.doc.features.last().unwrap().id;
        assert_eq!(app.state().state.doc.feature(column).unwrap().status, FeatureStatus::Ok);
        close(volume_of(app, 0), ROD + 6.0 * (6.0 * 30.0 - under_rod()), "the column from 30 mm down to the rod");
        pointer_to_chrome(app);
        case.frame(app, "column-on-curved-top");
        let model = app.state().state.doc.model();
        let back = soliddraft_model::Document::from_bytes(&model.to_bytes().unwrap(), model.kernel().clone()).unwrap();
        assert_eq!(back.part_json(), model.part_json());
        assert_eq!(back.geometry_hash(), model.geometry_hash());
        case.note("Up to face on the rod's top: the column stops on the curve, saved and reopened the same");
    });
}

#[test]
fn a_lost_curved_start_is_repaired_by_picking_the_face_again() {
    let mut case = Case::new("extrude-from-curved-repair");
    let (mut doc, square) = rod_and_square();
    doc.selection = Some(Selection::Feature(square));
    let mut app = launch(doc, STANDARD);
    case.run(&mut app, |case, app| {
        rebuilt(app);
        run(app, AppAction::StartExtrude);
        extrude_params(app).choices.from = StartKind::Surface(None);
        extrude_params(app).depth = 5.0;
        extrude_params(app).flip = true;
        extrude_params(app).choices.direction2 =
            Some(soliddraft_app::document::Direction2 { end: EndKind::Blind, depth: 5.0 });
        settle(app);
        let point = ux_tools::screen(app, glam::Vec3::new(-10.0, -20.0, 300f32.sqrt()));
        ribbon::render_steps::await_pick(app, point);
        ribbon::render_steps::rendered(app, |app| ribbon::click_at(app, point));
        settle(app);
        run(app, AppAction::CommitEditor);
        let boss = app.state().state.doc.features.last().unwrap().id;
        assert_eq!(app.state().state.doc.feature(boss).unwrap().status, FeatureStatus::Ok);

        // The saved part's start face no longer resolves; it opens needing repair.
        let mut truth = app.state().state.doc.model().truth();
        let features = truth["features"].as_array_mut().unwrap();
        assert_eq!(lose_curved_reference(features.last_mut().unwrap()), 1, "the boss names one curved face");
        let model = app.state().state.doc.model();
        let lost = soliddraft_model::Document::from_part_json(&truth.to_string(), model.kernel().clone()).unwrap();
        let file = case.dir.join("lost-curved-start.dwprt");
        std::fs::write(&file, lost.to_bytes().unwrap()).unwrap();
        run(app, AppAction::Open(Some(file)));
        let status = app.state().state.doc.feature(boss).unwrap().status.clone();
        let FeatureStatus::NeedsRepair(items) = status else { panic!("the boss needs repair: {status:?}") };
        assert_eq!(items.len(), 1, "{items:?}");
        run(app, AppAction::StartRepair(boss));
        pointer_to_chrome(app);
        case.frame(app, "repair-open");

        // The same curved face, picked in the viewport, is taken as the replacement.
        let point = ux_tools::screen(app, glam::Vec3::new(-10.0, -20.0, 300f32.sqrt()));
        ribbon::render_steps::await_pick(app, point);
        ribbon::render_steps::rendered(app, |app| ribbon::click_at(app, point));
        settle(app);
        let editor = app.state().state.editor.as_ref().expect("the repair form is open");
        assert_eq!(editor.error, None, "the curved face is accepted");
        let EditorParams::Repair { chosen, .. } = &editor.params else { panic!("repairing: {:?}", editor.params) };
        assert!(
            matches!(chosen.as_slice(), [Some(soliddraft_app::document::EntityRef::Face(_))]),
            "the curved face is the replacement: {chosen:?}"
        );
        pointer_to_chrome(app);
        case.frame(app, "curved-replacement-picked");
        run(app, AppAction::CommitEditor);
        assert_eq!(app.state().state.doc.feature(boss).unwrap().status, FeatureStatus::Ok);
        close(volume_of(app, 0), ROD + 180.0, "the repaired boss stands on the rod's top again");
        pointer_to_chrome(app);
        case.frame(app, "repaired");
        case.note("a lost curved From face repaired by picking it in the viewport");
    });
}

/// The curved-start part the browser check opens, built without a renderer: the web job
/// that makes browser fixtures has none. The rendered test above builds the same part by
/// picking the rod's top in the viewport; this one names that face through the model.
#[test]
fn a_headless_curved_start_part_produces_the_browser_fixture() {
    use soliddraft_model::sketch::{P2 as ModelP2, Sketch, solve::solve};
    use soliddraft_model::{
        DatumPlane, EndCondition, ExtrudeFeature, ExtrudeOp, ExtrudeOptions, ExtrudeStart, FeatureId, FeatureKind,
        ProfileSource, SketchFeature,
    };
    let mut doc = soliddraft_model::Document::new("Part", soliddraft_app::kernel());
    let add = |doc: &mut soliddraft_model::Document, kind: FeatureKind| -> FeatureId {
        doc.execute(soliddraft_model::Command::AddFeature { kind, name: None, position: None, via_assistant: false })
            .unwrap()
            .created
            .unwrap()
    };
    let sketch = |datum: DatumPlane, offset: f64, draw: &dyn Fn(&mut Sketch)| {
        let mut s = Sketch::new();
        draw(&mut s);
        solve(&mut s, None, true);
        FeatureKind::Sketch(SketchFeature { plane: soliddraft_model::SketchPlane::Datum { datum, offset }, sketch: s })
    };
    let extrude = |sketch: FeatureId, depth: f64, reverse: bool, options: ExtrudeOptions| {
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: sketch },
            operation: ExtrudeOp::Boss,
            end: EndCondition::Blind { depth },
            reverse,
            options,
        })
    };
    // The rod and the square of rod_and_square().
    let circle = add(
        &mut doc,
        sketch(DatumPlane::Front, 0.0, &|s| {
            s.circle_at(ModelP2::new(0.0, 0.0), 40.0).unwrap();
        }),
    );
    add(&mut doc, extrude(circle, 40.0, false, ExtrudeOptions::default()));
    let square = add(
        &mut doc,
        sketch(DatumPlane::Top, 30.0, &|s| {
            s.rectangle(ModelP2::new(5.0, -20.0), 6.0, 6.0).unwrap();
        }),
    );
    // The boss the rendered test ends with: from the rod's round top, flipped into the rod
    // 5 mm and 7 mm up as Direction 2, merged.
    let topology = doc.kernel().topology(doc.body().unwrap()).unwrap();
    let round = topology.faces.iter().find(|f| f.normal.is_none()).expect("the rod's round face").id;
    let face = doc.face_ref(round).unwrap();
    let options = ExtrudeOptions {
        from: Some(ExtrudeStart::Surface { plane: soliddraft_model::PlaneRef::Face { face } }),
        direction2: Some(EndCondition::Blind { depth: 7.0 }),
        ..Default::default()
    };
    add(&mut doc, extrude(square, 5.0, true, options));
    let volume = |doc: &soliddraft_model::Document| doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
    close(volume(&doc), ROD + 252.0, "the boss 7 mm out of the rod's top");
    // The later 4 mm post from the Top plane up through the rod.
    let post = add(
        &mut doc,
        sketch(DatumPlane::Top, 0.0, &|s| {
            s.rectangle(ModelP2::new(-10.0, -20.0), 4.0, 4.0).unwrap();
        }),
    );
    add(&mut doc, extrude(post, 45.0, false, ExtrudeOptions::default()));
    assert!(doc.statuses().into_iter().all(|(_, status)| status.is_ok()), "every feature builds");
    assert!(volume(&doc) > ROD + 252.0, "the later post adds material above the rod");
    let names: Vec<&str> = doc.features().iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names[3], "Boss-Extrude2", "the browser check edits this feature by name: {names:?}");
    let bytes = doc.to_bytes().unwrap();
    let back = soliddraft_model::Document::from_bytes(&bytes, soliddraft_app::kernel()).unwrap();
    assert_eq!(back.part_json(), doc.part_json());
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_BROWSER_FIXTURES") {
        std::fs::write(std::path::PathBuf::from(dir).join("extrude-from-curved.dwprt"), &bytes).unwrap();
    }
}
