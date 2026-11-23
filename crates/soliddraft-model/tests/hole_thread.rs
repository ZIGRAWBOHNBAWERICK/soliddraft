//! A hole with a cosmetic thread on the Catallax backend: the hole is the tap drill's
//! cylinder, and the part states the thread. The thread is checked as a user meets it:
//! built, edited, undone, saved and reopened, under a later cut that shortens the wall,
//! and through STEP to another part and back.

use std::sync::Arc;

use soliddraft_kernel_api::pmi::PmiKind;
use soliddraft_kernel_api::{FaceId, HoleDepth, HoleHead, HoleTip, StepOptions, Vec3};
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

/// A threaded wall without the kernel's handles: its hole, the thread, where it is, its
/// stretches, and whether it runs through.
type WallThread = (FeatureId, usize, HoleThread, [f64; 7], Vec<(f64, f64)>, bool);

/// What a part's threaded walls say about their threads, without the kernel's handles
/// for the body and faces, which a reopened part numbers afresh.
fn wall_threads(d: &Document) -> Vec<WallThread> {
    d.threaded_walls()
        .into_iter()
        .map(|w| {
            let at = [w.entry.x, w.entry.y, w.entry.z, w.direction.z, w.radius, w.top, w.faces.len() as f64];
            (w.feature, w.placement, w.thread, at, w.spans, w.through)
        })
        .collect()
}

/// The part's thread items are one, which applies, reads `callout` and measures Ø5.
fn stated(d: &Document, callout: &str) {
    let listed = threads(d);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!((listed[0].0.as_str(), &listed[0].2), (callout, &None), "{listed:?}");
    assert!(close(listed[0].1, 5.0), "{listed:?}");
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
        assert!(wall.spans.len() == 1 && close(wall.start(), 0.0) && close(wall.end(), 6.0), "{wall:?}");
        assert!(close(wall.top, 0.0) && !wall.through && wall.depth() == Some(6.0), "{wall:?}");
        assert_eq!(Some(wall.body), d.body());
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
    assert!(d.threaded_walls().iter().all(|w| close(w.radius, 3.4) && close(w.end(), 7.0)));

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
    assert_eq!(wall_threads(&reopened), wall_threads(&d));
    assert!(!wall_threads(&d).is_empty());

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
        assert!(wall.spans.len() == 1 && close(wall.start(), 3.0) && close(wall.end(), 6.0), "{wall:?}");
        assert!(close(wall.top, 3.0), "{wall:?}");
    }
    // The callout measures from the face the holes start on now: 3 mm of thread is left.
    let listed = threads(&d);
    assert_eq!(listed[0].0, "2\u{d7} M6x1-6H DEEP 3");
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
    // A class that is no tolerance class of an internal thread does not become a
    // designation, from a script or from a saved part.
    let class = add(&mut d, hole(5.0, HoleDepth::Through, HoleThread { class: "6NOPE".into(), ..m6(None) }));
    assert_eq!(
        d.status(class),
        Some(&FeatureStatus::Failed(
            "6NOPE is not a tolerance class of an internal thread: a grade from 4 to 8 and G or H, such as 6H, or the \
             pitch and crest classes, such as 5H6H."
                .into()
        ))
    );
    let reopened = Document::from_bytes(&d.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    assert!(matches!(reopened.status(class), Some(FeatureStatus::Failed(why)) if why.starts_with("6NOPE is not")));
    assert!(d.threaded_walls().is_empty() && d.thread_items().is_empty());
}

/// A rectangle `width` x `height` centred at (`x`, `y`) on the top datum, `offset` above it,
/// extruded `depth` upwards and cut from the part.
fn pocket(doc: &mut Document, x: f64, y: f64, width: f64, height: f64, offset: f64, depth: f64) {
    let mut s = Sketch::new();
    s.rectangle(P2::new(x, y), width, height).unwrap();
    solve(&mut s, None, true);
    let sketch = add(
        doc,
        FeatureKind::Sketch(SketchFeature { plane: SketchPlane::Datum { datum: DatumPlane::Top, offset }, sketch: s }),
    );
    add(
        doc,
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: sketch },
            operation: ExtrudeOp::Cut,
            end: EndCondition::Blind { depth },
            reverse: false,
        }),
    );
    for (id, status) in doc.statuses() {
        assert!(status.is_ok(), "{id}: {status:?}");
    }
}

/// One hole at (-15, 0) through the 10 mm plate, drilled 5 mm with `head`, tapped M6.
fn through_plate(thread: HoleThread, head: Option<HoleHead>) -> (Document, FeatureId) {
    let mut d = doc();
    slab(&mut d, 0.0, 60.0, 40.0, 10.0, ExtrudeOp::Boss);
    let face = d.face_ref(top_face(&d, 10.0)).unwrap();
    let hole = add(
        &mut d,
        FeatureKind::Hole(HoleFeature {
            face,
            points: vec![P2::new(-15.0, 0.0)],
            diameter: thread.tap_drill(),
            depth: HoleDepth::Through,
            tip: HoleTip::Flat,
            head,
            thread: Some(thread),
        }),
    );
    (d, hole)
}

#[test]
fn a_thread_longer_than_a_through_hole_runs_out_through_it() {
    // 20 mm of thread asked for in a 10 mm plate: the thread goes through, and says so.
    let (d, hole) = through_plate(m6(Some(20.0)), None);
    assert_eq!(d.status(hole), Some(&FeatureStatus::Ok));
    let walls = d.threaded_walls();
    assert_eq!(walls.len(), 1, "{walls:?}");
    assert!(walls[0].through && walls[0].depth().is_none(), "{walls:?}");
    assert!(close(walls[0].start(), 0.0) && close(walls[0].end(), 10.0), "{walls:?}");
    stated(&d, "M6x1-6H THRU");
}

#[test]
fn a_cut_that_takes_the_thread_leaves_the_plain_wall_unthreaded() {
    // 4 mm of thread in a through hole; 5 mm milled off the top leaves 5 mm of plain wall.
    let (mut d, hole) = through_plate(m6(Some(4.0)), None);
    assert_eq!(threads(&d)[0].0, "M6x1-6H DEEP 4");
    pocket(&mut d, 0.0, 0.0, 80.0, 60.0, 5.0, 5.0);
    assert!(d.status(hole).unwrap().is_ok());
    assert!(d.threaded_walls().is_empty(), "{:?}", d.threaded_walls());
    let listed = threads(&d);
    assert_eq!(listed.len(), 1, "{listed:?}");
    let problem = listed[0].2.clone().expect("the thread item does not apply");
    assert!(problem.contains("No wall of Hole1 is threaded any more"), "{problem}");
    // Neither a STEP file nor a drawing gets a thread on the plain wall.
    let on_body = d.pmi_on_body();
    assert_eq!(on_body.items, 0, "{:?}", on_body.pmi);
    assert_eq!(on_body.left_out, [("Hole1 thread".to_owned(), problem)]);
}

#[test]
fn a_milled_through_hole_states_the_thread_that_is_left() {
    let (mut d, _) = through_plate(m6(Some(4.0)), None);
    pocket(&mut d, 0.0, 0.0, 80.0, 60.0, 8.0, 2.0);
    let walls = d.threaded_walls();
    assert_eq!(walls.len(), 1);
    assert!(close(walls[0].top, 2.0) && close(walls[0].start(), 2.0) && close(walls[0].end(), 4.0), "{walls:?}");
    stated(&d, "M6x1-6H DEEP 2");
    let reopened = Document::from_bytes(&d.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(threads(&reopened), threads(&d));
}

#[test]
fn a_countersunk_thread_starts_where_the_straight_wall_does() {
    // A Ø12 90° countersink on the Ø5 drill reaches 3.5 mm down.
    let sink = HoleHead::Countersink { diameter: 12.0, angle_deg: 90.0 };
    let (d, hole) = through_plate(m6(Some(3.0)), Some(sink));
    assert_eq!(d.status(hole), Some(&FeatureStatus::Failed("The thread ends inside the 3.5 mm countersink.".into())));
    assert!(d.threaded_walls().is_empty() && d.thread_items().is_empty());

    let (mut d, hole) = through_plate(m6(Some(5.0)), Some(sink));
    assert_eq!(d.status(hole), Some(&FeatureStatus::Ok));
    let walls = d.threaded_walls();
    assert_eq!(walls.len(), 1);
    assert!(close(walls[0].top, 0.0) && close(walls[0].start(), 3.5) && close(walls[0].end(), 5.0), "{walls:?}");
    stated(&d, "M6x1-6H DEEP 5");
    // Milled 1 mm, the countersink starts 1 mm lower and so does the callout.
    pocket(&mut d, 0.0, 0.0, 80.0, 60.0, 9.0, 1.0);
    let walls = d.threaded_walls();
    assert!(close(walls[0].top, 1.0) && close(walls[0].start(), 3.5) && close(walls[0].end(), 5.0), "{walls:?}");
    stated(&d, "M6x1-6H DEEP 4");
}

#[test]
fn holes_a_cut_left_different_threads_are_called_out_apart() {
    let (mut d, hole) = tapped_plate();
    // Mill 3 mm off over the hole at x -15 only.
    pocket(&mut d, -20.0, 0.0, 30.0, 60.0, 7.0, 3.0);
    let walls = d.threaded_walls();
    assert_eq!(walls.len(), 2);
    let rows: Vec<_> = d.pmi_rows().into_iter().filter(|row| row.feature == Some(hole)).collect();
    let listed: Vec<_> = rows.iter().map(|row| (row.item.id.as_str(), row.callout(), row.placements.clone())).collect();
    assert_eq!(
        listed,
        [
            ("Hole1 thread", "M6x1-6H DEEP 3".to_owned(), vec![0]),
            ("Hole1 thread 2", "M6x1-6H DEEP 6".to_owned(), vec![1])
        ]
    );
    assert!(rows.iter().all(|row| row.applies() && close(row.measured[0], 5.0)), "{rows:?}");
    let on_body = d.pmi_on_body();
    assert_eq!(on_body.items, 2, "{:?}", on_body.left_out);
}

#[test]
fn a_thread_stays_on_its_body_in_a_part_of_several_bodies() {
    let (mut d, hole) = tapped_plate();
    let plate = d.body().unwrap();
    // A separate block, a new body above the plate.
    let mut s = Sketch::new();
    s.rectangle(P2::new(0.0, 0.0), 10.0, 10.0).unwrap();
    solve(&mut s, None, true);
    let in_body = |kind| Command::AddFeatureInBody { body: 1, via_assistant: false, kind, position: None };
    let sketch = d
        .execute(in_body(FeatureKind::Sketch(SketchFeature {
            plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 20.0 },
            sketch: s,
        })))
        .unwrap()
        .created
        .unwrap();
    d.execute(in_body(FeatureKind::Extrude(ExtrudeFeature {
        profile: ProfileSource::Sketch { feature: sketch },
        operation: ExtrudeOp::Boss,
        end: EndCondition::Blind { depth: 5.0 },
        reverse: false,
    })))
    .unwrap();
    for (id, status) in d.statuses() {
        assert!(status.is_ok(), "{id}: {status:?}");
    }
    assert_eq!(d.live_bodies().len(), 2);
    assert_eq!(d.body(), None, "the part has several bodies");
    let walls = d.threaded_walls();
    assert_eq!(walls.len(), 2, "{walls:?}");
    assert!(walls.iter().all(|wall| wall.body == plate && close(wall.end(), 6.0)), "{walls:?}");
    let rows: Vec<_> = d.pmi_rows().into_iter().filter(|row| row.feature == Some(hole)).collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].problem, None);
    assert_eq!(rows[0].callout(), "2\u{d7} M6x1-6H DEEP 6");
    assert_eq!(rows[0].body, Some(plate));
    assert!(close(rows[0].measured[0], 5.0), "{:?}", rows[0].measured);
}

#[test]
fn a_pocket_across_the_bore_splits_the_thread() {
    let (mut d, _) = through_plate(m6(None), None);
    // A pocket from the plate's side, 4 to 6 mm up, through the bore all round; the plate
    // stays one piece beside it.
    pocket(&mut d, -15.0, -12.5, 10.0, 30.0, 4.0, 2.0);
    let walls = d.threaded_walls();
    assert_eq!(walls.len(), 1, "{walls:?}");
    let spans = &walls[0].spans;
    assert_eq!(spans.len(), 2, "{walls:?}");
    assert!(close(spans[0].0, 0.0) && close(spans[0].1, 4.0), "{spans:?}");
    assert!(close(spans[1].0, 6.0) && close(spans[1].1, 10.0), "{spans:?}");
    assert_eq!(walls[0].faces.len(), 2);
    assert!(walls[0].through);
    stated(&d, "M6x1-6H THRU");
}
