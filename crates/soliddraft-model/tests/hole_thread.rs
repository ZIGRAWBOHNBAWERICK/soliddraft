//! A hole with a cosmetic thread on the Catallax backend: the hole is the tap drill's
//! cylinder, and the part states the thread. The thread is checked as a user meets it:
//! built, edited, undone, saved and reopened, under a later cut that shortens the wall,
//! and through STEP to another part and back.

use std::sync::Arc;

use soliddraft_kernel_api::pmi::PmiKind;
use soliddraft_kernel_api::{FaceId, HoleDepth, HoleTip, StepOptions, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, SolveStatus, solve::solve};
use soliddraft_model::thread::{HoleThread, metric};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureId, FeatureKind, FeatureStatus,
    HoleFeature, ImportStepFeature, ProfileSource, SketchFeature, SketchPlane,
};

fn doc() -> Document {
    Document::new("Part1", Arc::new(CatallaxKernel::new()))
}

fn add(doc: &mut Document, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
        .unwrap()
        .created
        .unwrap()
}

/// A rectangle on the top datum, `offset` above it, extruded `depth` upwards.
fn slab(doc: &mut Document, offset: f64, width: f64, height: f64, depth: f64, operation: ExtrudeOp) -> FeatureId {
    let mut s = Sketch::new();
    s.rectangle(P2::new(0.0, 0.0), width, height).unwrap();
    solve(&mut s, None, true);
    assert_eq!(s.status, SolveStatus::FullyDefined);
    let sketch = add(
        doc,
        FeatureKind::Sketch(SketchFeature { plane: SketchPlane::Datum { datum: DatumPlane::Top, offset }, sketch: s }),
    );
    add(
        doc,
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: sketch },
            operation,
            end: EndCondition::Blind { depth },
            reverse: false,
        }),
    )
}

fn top_face(doc: &Document, z: f64) -> FaceId {
    let topology = doc.kernel().topology(doc.body().unwrap()).unwrap();
    topology
        .faces
        .iter()
        .find(|f| f.normal.is_some_and(|n| n.dot(Vec3::Z) > 1.0 - 1e-9) && (f.centroid.z - z).abs() < 1e-6)
        .expect("the plate's top face")
        .id
}

fn m6(length: Option<f64>) -> HoleThread {
    HoleThread { length, ..HoleThread::coarse(metric(6.0).unwrap()) }
}

/// Two blind holes 7 mm deep in the top of a 60 x 40 x 10 plate, tapped M6 for 6 mm. The
/// drill tips of every size these tests take stay inside the plate.
fn tapped_plate() -> (Document, FeatureId) {
    let mut d = doc();
    slab(&mut d, 0.0, 60.0, 40.0, 10.0, ExtrudeOp::Boss);
    let face = d.face_ref(top_face(&d, 10.0)).unwrap();
    let thread = m6(Some(6.0));
    let hole = add(
        &mut d,
        FeatureKind::Hole(HoleFeature {
            face,
            points: vec![P2::new(-15.0, 0.0), P2::new(15.0, 0.0)],
            diameter: thread.tap_drill(),
            depth: HoleDepth::Blind { depth: 7.0 },
            tip: HoleTip::Cone { angle_deg: 118.0 },
            head: None,
            thread: Some(thread),
        }),
    );
    for (id, status) in d.statuses() {
        assert!(status.is_ok(), "{id}: {status:?}");
    }
    (d, hole)
}

fn hole_of(d: &Document, id: FeatureId) -> HoleFeature {
    match &d.feature(id).unwrap().kind {
        FeatureKind::Hole(h) => h.clone(),
        other => panic!("{other:?}"),
    }
}

/// The thread items the part lists: callout, measured diameter, problem.
fn threads(d: &Document) -> Vec<(String, f64, Option<String>)> {
    d.pmi_rows()
        .into_iter()
        .filter(|row| row.feature.is_some())
        .map(|row| (row.callout(), row.measured.first().copied().unwrap_or(f64::NAN), row.problem.clone()))
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn a_tapped_hole_is_its_drill_and_states_its_thread() {
    let (d, hole) = tapped_plate();
    // The wall is the 5 mm tap drill's cylinder; the thread is not modelled.
    let listed = threads(&d);
    assert_eq!(listed.len(), 1, "{listed:?}");
    let (callout, measured, problem) = &listed[0];
    assert_eq!(callout, "2\u{d7} M6x1-6H DEEP 6");
    assert!(close(*measured, 5.0), "{measured}");
    assert_eq!(problem, &None);
    let row = d.pmi_rows().into_iter().find(|row| row.feature == Some(hole)).unwrap();
    assert_eq!(row.item.id, format!("{} thread", d.feature(hole).unwrap().name));
    // The part stores no item: the hole states it.
    assert!(d.pmi().items.is_empty());

    let walls = d.threaded_walls();
    assert_eq!(walls.len(), 2);
    for wall in &walls {
        assert!(close(wall.radius, 2.5), "{wall:?}");
        assert!(close(wall.entry.z, 10.0) && close(wall.direction.z, -1.0), "{wall:?}");
        assert!(close(wall.span.0, 0.0) && close(wall.span.1, 6.0), "{wall:?}");
    }
}

#[test]
fn the_thread_survives_edit_undo_save_and_reopen() {
    let (mut d, hole) = tapped_plate();
    let mut edited = hole_of(&d, hole);
    let m8 = HoleThread { length: Some(7.0), ..HoleThread::coarse(metric(8.0).unwrap()) };
    edited.diameter = m8.tap_drill();
    edited.thread = Some(m8);
    d.execute(Command::EditFeature { id: hole, kind: FeatureKind::Hole(edited.clone()) }).unwrap();
    assert!(d.status(hole).unwrap().is_ok());
    let listed = threads(&d);
    assert_eq!(listed[0].0, "2\u{d7} M8x1.25-6H DEEP 7");
    assert!(close(listed[0].1, 6.8), "{listed:?}");
    assert!(d.threaded_walls().iter().all(|w| close(w.radius, 3.4) && close(w.span.1, 7.0)));

    // A fine pitch is a pitch of the same size.
    let mut fine = edited.clone();
    fine.thread = Some(HoleThread { pitch: 1.0, ..fine.thread.clone().unwrap() });
    fine.diameter = fine.thread.as_ref().unwrap().tap_drill();
    d.execute(Command::EditFeature { id: hole, kind: FeatureKind::Hole(fine) }).unwrap();
    assert_eq!(threads(&d)[0].0, "2\u{d7} M8x1-6H DEEP 7");

    d.undo().unwrap();
    assert_eq!(hole_of(&d, hole), edited);
    d.undo().unwrap();
    assert_eq!(hole_of(&d, hole).thread, Some(m6(Some(6.0))));
    d.redo().unwrap();

    let reopened = Document::from_bytes(&d.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(hole_of(&reopened, hole), edited);
    assert_eq!(threads(&reopened), threads(&d));
    assert_eq!(reopened.threaded_walls(), d.threaded_walls());

    // Taken out, the hole is plain and states nothing.
    let mut plain = edited;
    plain.thread = None;
    d.execute(Command::EditFeature { id: hole, kind: FeatureKind::Hole(plain) }).unwrap();
    assert!(threads(&d).is_empty() && d.threaded_walls().is_empty());
}

#[test]
fn a_later_cut_leaves_the_thread_on_what_is_left_of_the_wall() {
    let (mut d, hole) = tapped_plate();
    // Mill 3 mm off the top: the walls now start 3 mm below where the holes started.
    slab(&mut d, 7.0, 80.0, 60.0, 3.0, ExtrudeOp::Cut);
    for (id, status) in d.statuses() {
        assert!(status.is_ok(), "{id}: {status:?}");
    }
    let walls = d.threaded_walls();
    assert_eq!(walls.len(), 2);
    for wall in &walls {
        assert!(close(wall.span.0, 3.0) && close(wall.span.1, 6.0), "{wall:?}");
    }
    let listed = threads(&d);
    assert_eq!(listed[0].0, "2\u{d7} M6x1-6H DEEP 6");
    assert_eq!(listed[0].2, None);
    // Milled below the thread, no wall is left threaded and the item says why.
    slab(&mut d, 1.0, 80.0, 60.0, 6.0, ExtrudeOp::Cut);
    assert!(d.threaded_walls().is_empty());
    assert!(d.status(hole).unwrap().is_ok());
    assert!(threads(&d)[0].2.is_some(), "{:?}", threads(&d));
}

#[test]
fn step_carries_the_designation_to_another_part_and_back() {
    let (d, _) = tapped_plate();
    let on_body = d.pmi_on_body();
    assert_eq!(on_body.items, 1, "{:?}", on_body.left_out);
    let step = d.kernel().export_step_with_pmi(d.body().unwrap(), &StepOptions::default(), &on_body.pmi).unwrap();
    assert!(step.text.contains("'M6x1-6H'"), "the designation is in the file");

    let mut other = doc();
    let import =
        add(&mut other, FeatureKind::ImportStep(ImportStepFeature::new("plate.step", step.text.clone().into_bytes())));
    let read = other.kernel().read_step_pmi(step.text.as_bytes()).unwrap();
    let (items, report) = other.pmi_from_step(&[import], &read);
    assert!(report.left_out.is_empty(), "{:?}", report.left_out);
    assert_eq!(items.len(), 1);
    match &items[0].kind {
        PmiKind::Size { features, thread, through, .. } => {
            assert_eq!(thread.as_deref(), Some("M6x1-6H"));
            assert_eq!(features.len(), 2);
            assert!(!through);
        }
        other => panic!("{other:?}"),
    }
    for item in items {
        other.execute(Command::PutPmiItem { item: Box::new(item), replaces: None }).unwrap();
    }
    let rows = other.pmi_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].callout(), "2\u{d7} M6x1-6H");
    assert!(close(rows[0].measured[0], 5.0));
}

#[test]
fn a_thread_that_does_not_fit_its_hole_is_refused_by_name() {
    let mut d = doc();
    slab(&mut d, 0.0, 60.0, 40.0, 10.0, ExtrudeOp::Boss);
    let face = d.face_ref(top_face(&d, 10.0)).unwrap();
    let hole = |diameter, depth, thread| {
        FeatureKind::Hole(HoleFeature {
            face: face.clone(),
            points: vec![P2::new(0.0, 0.0)],
            diameter,
            depth,
            tip: HoleTip::Flat,
            head: None,
            thread: Some(thread),
        })
    };
    let clearance = add(&mut d, hole(6.6, HoleDepth::Through, m6(None)));
    assert_eq!(
        d.status(clearance),
        Some(&FeatureStatus::Failed(
            "A M6x1-6H thread is cut in a hole from Ø4.917 to under Ø6; this hole is Ø6.6.".into()
        ))
    );
    let deep = add(&mut d, hole(5.0, HoleDepth::Blind { depth: 8.0 }, m6(Some(9.0))));
    assert_eq!(
        d.status(deep),
        Some(&FeatureStatus::Failed("The thread is 9 mm long but the hole is 8 mm deep.".into()))
    );
    assert!(d.threaded_walls().is_empty() && d.thread_items().is_empty());
}
