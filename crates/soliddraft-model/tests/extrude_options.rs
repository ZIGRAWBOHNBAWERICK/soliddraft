//! An extrusion's From, Direction 2, thin wall and selected contours: each builds, edits,
//! saves in format 6, reopens to the same part and carries a later feature.

use std::sync::Arc;

use soliddraft_kernel_api::{BooleanOp, FaceId, Kernel, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, Contour, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, ExtrudeOptions, ExtrudeStart,
    FeatureId, FeatureKind, FeatureStatus, Generator, PlaneRef, ProfileSource, SketchFeature, SketchPlane, ThinFeature,
    ThinSide,
};

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn doc() -> Document {
    Document::new("Part1", kernel())
}

fn add(doc: &mut Document, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
        .unwrap()
        .created
        .unwrap()
}

fn edit(doc: &mut Document, id: FeatureId, kind: FeatureKind) {
    doc.execute(Command::EditFeature { id, kind }).unwrap();
}

fn sketch(plane: SketchPlane, build: impl FnOnce(&mut Sketch)) -> FeatureKind {
    let mut s = Sketch::new();
    build(&mut s);
    solve(&mut s, None, true);
    FeatureKind::Sketch(SketchFeature { plane, sketch: s })
}

fn top(offset: f64) -> SketchPlane {
    SketchPlane::Datum { datum: DatumPlane::Top, offset }
}

fn extrude(
    sketch: FeatureId,
    operation: ExtrudeOp,
    end: EndCondition,
    reverse: bool,
    options: ExtrudeOptions,
) -> FeatureKind {
    FeatureKind::Extrude(ExtrudeFeature {
        profile: ProfileSource::Sketch { feature: sketch },
        operation,
        end,
        reverse,
        options,
    })
}

fn blind(depth: f64) -> EndCondition {
    EndCondition::Blind { depth }
}

fn volume(doc: &Document) -> f64 {
    doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume
}

/// The lowest and highest planar faces facing down and up, by their centroids: exact,
/// where the kernel's bounding box is padded.
fn bounds(doc: &Document) -> (Vec3, Vec3) {
    let t = doc.kernel().topology(doc.body().unwrap()).unwrap();
    let caps = |dir: f64| {
        t.faces
            .iter()
            .filter(|f| f.normal.is_some_and(|n| n.z * dir > 1.0 - 1e-9))
            .map(|f| f.centroid)
            .collect::<Vec<_>>()
    };
    let lo = caps(-1.0).into_iter().min_by(|a, b| a.z.total_cmp(&b.z)).expect("a face down");
    let hi = caps(1.0).into_iter().max_by(|a, b| a.z.total_cmp(&b.z)).expect("a face up");
    (lo, hi)
}

fn all_ok(doc: &Document) {
    for (id, status) in doc.statuses() {
        assert!(status.is_ok(), "{id}: {status:?}");
    }
}

fn close(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() <= 1e-6 * b.abs().max(1.0), "{what}: {a} is not {b}");
}

/// The part saved and opened again: it must say format 6, keep its truth and its shape.
fn reopened(doc: &Document) -> Document {
    assert_eq!(doc.truth()["format"], "soliddraft-part/6", "extrusion options need format 6");
    assert_eq!(doc.manifest().format, "soliddraft-part/6");
    let back = Document::from_bytes(&doc.to_bytes().unwrap(), kernel()).unwrap();
    assert_eq!(back.part_json(), doc.part_json(), "the truth round-trips");
    assert_eq!(back.geometry_hash(), doc.geometry_hash());
    all_ok(&back);
    close(volume(&back), volume(doc), "the reopened volume");
    back
}

fn face_facing(doc: &Document, normal: Vec3, z: f64) -> FaceId {
    let t = doc.kernel().topology(doc.body().unwrap()).unwrap();
    t.faces
        .iter()
        .find(|f| f.normal.is_some_and(|n| n.dot(normal) > 1.0 - 1e-9) && (f.centroid.z - z).abs() < 1e-6)
        .unwrap()
        .id
}

/// A cut through all from a circle on the Top plane: the later feature on top.
fn later_cut(doc: &mut Document, diameter: f64) -> FeatureId {
    let s = add(
        doc,
        sketch(top(0.0), |s| {
            s.circle_at(P2::new(0.0, 0.0), diameter).unwrap();
        }),
    );
    let both = ExtrudeOptions { direction2: Some(EndCondition::ThroughAll), ..Default::default() };
    add(doc, extrude(s, ExtrudeOp::Cut, EndCondition::ThroughAll, false, both))
}

#[test]
fn an_offset_start_moves_the_extrusion_off_its_sketch_plane_and_flips() {
    let mut d = doc();
    let s = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 20.0, 10.0).unwrap();
        }),
    );
    let from =
        |reverse| ExtrudeOptions { from: Some(ExtrudeStart::Offset { distance: 5.0, reverse }), ..Default::default() };
    let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(10.0), false, from(false)));
    all_ok(&d);
    close(volume(&d), 2000.0, "20 x 10 x 10");
    let (lo, hi) = bounds(&d);
    close(lo.z, 5.0, "starts 5 mm up");
    close(hi.z, 15.0, "ends 10 mm later");
    // The offset measured the other way starts below the sketch.
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(10.0), false, from(true)));
    let (lo, hi) = bounds(&d);
    close(lo.z, -5.0, "a reversed offset starts below");
    close(hi.z, 5.0, "and goes up 10");
    // Flipping the extrusion flips the offset with it: the start stays 5 mm along it.
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(10.0), true, from(false)));
    let (lo, hi) = bounds(&d);
    close(hi.z, -5.0, "a flipped extrusion starts 5 mm below");
    close(lo.z, -15.0, "and goes down 10");
    let mut back = reopened(&d);
    // A later feature on the reopened part: a hole through all of it.
    let cut = later_cut(&mut back, 4.0);
    assert_eq!(back.status(cut), Some(&FeatureStatus::Ok));
    close(volume(&back), 2000.0 - std::f64::consts::PI * 4.0 * 10.0, "the later cut removes a 4 mm hole");
    reopened(&back);
}

#[test]
fn an_offset_start_from_a_face_sketch_begins_inside_the_block() {
    let mut d = doc();
    let base = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 40.0, 40.0).unwrap();
        }),
    );
    add(&mut d, extrude(base, ExtrudeOp::Boss, blind(10.0), false, Default::default()));
    let face = d.face_ref(face_facing(&d, Vec3::Z, 10.0)).unwrap();
    let s = add(
        &mut d,
        sketch(SketchPlane::Face { face }, |s| {
            s.circle_at(P2::new(0.0, 0.0), 10.0).unwrap();
        }),
    );
    // 4 mm into the block, then 10 mm out: 6 mm stands above the top face.
    let from =
        ExtrudeOptions { from: Some(ExtrudeStart::Offset { distance: 4.0, reverse: true }), ..Default::default() };
    let boss = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(10.0), false, from));
    all_ok(&d);
    close(volume(&d), 16000.0 + std::f64::consts::PI * 25.0 * 6.0, "the block and 6 mm of boss");
    close(bounds(&d).1.z, 16.0, "the boss ends 6 mm above the top face");
    let mut back = reopened(&d);
    // A later feature reads the boss's end cap.
    let cap = back.face_ref(face_facing(&back, Vec3::Z, 16.0)).unwrap();
    assert_eq!(cap.feature, boss, "the end cap is the boss's");
    let s2 = add(
        &mut back,
        sketch(SketchPlane::Face { face: cap }, |s| {
            s.circle_at(P2::new(0.0, 0.0), 4.0).unwrap();
        }),
    );
    add(&mut back, extrude(s2, ExtrudeOp::Boss, blind(3.0), false, Default::default()));
    all_ok(&back);
    close(bounds(&back).1.z, 19.0, "the later boss stands on the end cap");
    reopened(&back);
}

#[test]
fn direction_2_goes_the_other_way_from_the_same_start() {
    let mut d = doc();
    let s = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 20.0, 10.0).unwrap();
        }),
    );
    let second = |end| ExtrudeOptions { direction2: Some(end), ..Default::default() };
    let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(10.0), false, second(blind(4.0))));
    all_ok(&d);
    close(volume(&d), 2800.0, "20 x 10 x (10 + 4)");
    let (lo, hi) = bounds(&d);
    close(lo.z, -4.0, "Direction 2 goes 4 mm down");
    close(hi.z, 10.0, "Direction 1 goes 10 mm up");
    let names: Vec<Generator> = d.generated_faces(e).iter().map(|n| n.generator.clone()).collect();
    assert!(names.contains(&Generator::StartCap) && names.contains(&Generator::EndCap), "{names:?}");
    // Off again, the extrusion is as it was.
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(10.0), false, Default::default()));
    close(volume(&d), 2000.0, "without Direction 2");
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(10.0), false, second(blind(4.0))));
    let mut back = reopened(&d);
    let cut = later_cut(&mut back, 2.0);
    assert_eq!(back.status(cut), Some(&FeatureStatus::Ok));
    close(volume(&back), 2800.0 - std::f64::consts::PI * 14.0, "the later cut goes through both directions");
    reopened(&back);
}

#[test]
fn direction_2_with_an_up_to_next_first_direction_sweeps_each_way() {
    let mut d = doc();
    let base = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 40.0, 40.0).unwrap();
        }),
    );
    add(&mut d, extrude(base, ExtrudeOp::Boss, blind(10.0), false, Default::default()));
    // A post sketched above the block: down to the block, and 5 mm up as well.
    let s = add(
        &mut d,
        sketch(top(20.0), |s| {
            s.circle_at(P2::new(0.0, 0.0), 6.0).unwrap();
        }),
    );
    let second = ExtrudeOptions { direction2: Some(blind(5.0)), ..Default::default() };
    let post = add(&mut d, extrude(s, ExtrudeOp::Boss, EndCondition::UpToNext, true, second));
    all_ok(&d);
    close(volume(&d), 16000.0 + std::f64::consts::PI * 9.0 * 15.0, "10 mm down to the block and 5 mm up");
    close(bounds(&d).1.z, 25.0, "Direction 2 ends 5 mm above the sketch");
    // The far end of Direction 2 is the post's start cap, as for one sweep through the start.
    let top_cap = d.face_ref(face_facing(&d, Vec3::Z, 25.0)).unwrap();
    assert_eq!((top_cap.feature, top_cap.generator), (post, Generator::StartCap));
    reopened(&d);
}

#[test]
fn a_cut_through_all_in_both_directions_from_a_mid_plane() {
    let mut d = doc();
    let base = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 40.0, 40.0).unwrap();
        }),
    );
    add(&mut d, extrude(base, ExtrudeOp::Boss, blind(10.0), false, Default::default()));
    let s = add(
        &mut d,
        sketch(top(5.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 10.0, 10.0).unwrap();
        }),
    );
    let both = ExtrudeOptions { direction2: Some(EndCondition::ThroughAll), ..Default::default() };
    add(&mut d, extrude(s, ExtrudeOp::Cut, EndCondition::ThroughAll, false, both));
    all_ok(&d);
    close(volume(&d), 16000.0 - 1000.0, "a square hole through the whole block");
    reopened(&d);
}

#[test]
fn thin_walls_follow_closed_and_open_sketches() {
    let mut d = doc();
    let s = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 20.0, 10.0).unwrap();
        }),
    );
    let thin = |thickness, side| ExtrudeOptions { thin: Some(ThinFeature { thickness, side }), ..Default::default() };
    let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(5.0), false, thin(1.0, ThinSide::One { reverse: false })));
    all_ok(&d);
    close(volume(&d), (22.0 * 12.0 - 200.0) * 5.0, "a 1 mm wall outside the rectangle");
    let names: Vec<Generator> = d.generated_faces(e).iter().map(|n| n.generator.clone()).collect();
    assert!(names.iter().any(|g| matches!(g, Generator::ThinWall { second: false, .. })), "{names:?}");
    assert!(names.iter().any(|g| matches!(g, Generator::Side { .. })), "the sketch's own side keeps its name");
    for (side, expected, what) in [
        (ThinSide::One { reverse: true }, (200.0 - 18.0 * 8.0) * 5.0, "inside"),
        (ThinSide::MidPlane, (21.0 * 11.0 - 19.0 * 9.0) * 5.0, "centred"),
        (ThinSide::Two { other: 0.5 }, (22.0 * 12.0 - 19.0 * 9.0) * 5.0, "1 mm out and 0.5 mm in"),
    ] {
        edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), false, thin(1.0, side)));
        all_ok(&d);
        close(volume(&d), expected, what);
    }
    let mut back = reopened(&d);
    // A later cut through the wall.
    let c = add(
        &mut back,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(10.0, 0.0), 4.0, 2.0).unwrap();
        }),
    );
    let both = ExtrudeOptions { direction2: Some(EndCondition::ThroughAll), ..Default::default() };
    let cut = add(&mut back, extrude(c, ExtrudeOp::Cut, EndCondition::ThroughAll, false, both));
    assert_eq!(back.status(cut), Some(&FeatureStatus::Ok));
    assert!(volume(&back) < volume(&d), "the cut opens the wall");
    reopened(&back);

    // An open L: a wall with two straight ends, on one side or the other.
    let mut d = doc();
    let l = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.add(EntityKind::Line { a: P2::new(0.0, 0.0), b: P2::new(20.0, 0.0) }, false);
            s.add(EntityKind::Line { a: P2::new(20.0, 0.0), b: P2::new(20.0, 10.0) }, false);
        }),
    );
    let wall = add(&mut d, extrude(l, ExtrudeOp::Boss, blind(5.0), false, thin(1.0, ThinSide::One { reverse: false })));
    all_ok(&d);
    let one = volume(&d);
    edit(&mut d, wall, extrude(l, ExtrudeOp::Boss, blind(5.0), false, thin(1.0, ThinSide::One { reverse: true })));
    all_ok(&d);
    let other = volume(&d);
    let mut sides = [one, other];
    sides.sort_by(f64::total_cmp);
    close(sides[0], 29.0 * 5.0, "the wall inside the corner");
    close(sides[1], 31.0 * 5.0, "the wall outside the corner");
    let names: Vec<Generator> = d.generated_faces(wall).iter().map(|n| n.generator.clone()).collect();
    assert!(names.iter().any(|g| matches!(g, Generator::ThinEnd { last: false, .. })), "{names:?}");
    assert!(names.iter().any(|g| matches!(g, Generator::ThinEnd { last: true, .. })), "{names:?}");
    reopened(&d);
}

#[test]
fn selected_contours_extrude_only_the_chosen_regions() {
    let mut d = doc();
    let s = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(-15.0, 0.0), 10.0, 10.0).unwrap();
            s.rectangle(P2::new(15.0, 0.0), 10.0, 10.0).unwrap();
        }),
    );
    let regions = d.built_sketch(s).unwrap().regions.clone().unwrap();
    assert_eq!(regions.len(), 2);
    let pick = |contours: Vec<Contour>| ExtrudeOptions { contours, ..Default::default() };
    let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(5.0), false, pick(vec![Contour::of(&regions[0])])));
    all_ok(&d);
    close(volume(&d), 500.0, "one 10 x 10 square");
    let centre = |d: &Document| d.kernel().mass_properties(d.body().unwrap()).unwrap().centroid.x;
    let first = centre(&d);
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), false, pick(vec![Contour::of(&regions[1])])));
    close(volume(&d), 500.0, "the other square");
    assert!((centre(&d) + first).abs() < 1e-6, "the other square is the mirror one");
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), false, pick(regions.iter().map(Contour::of).collect())));
    close(volume(&d), 1000.0, "both squares");
    // A contour the sketch no longer has stops the extrusion with a reason.
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), false, pick(vec![Contour(vec![999])])));
    match d.status(e) {
        Some(FeatureStatus::Failed(message)) => assert!(message.contains("choose the contours again"), "{message}"),
        other => panic!("a lost contour fails: {other:?}"),
    }
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), false, pick(vec![Contour::of(&regions[0])])));
    all_ok(&d);
    let mut back = reopened(&d);
    let cut = later_cut(&mut back, 1.0);
    assert_eq!(back.status(cut), Some(&FeatureStatus::Ok));
    reopened(&back);
}

/// A wedge 40 mm wide and 40 mm deep whose top falls from 30 mm at x = -20 to 10 mm
/// at x = 20: 32000 cubic millimetres.
fn wedge(doc: &mut Document) -> FeatureId {
    let s = add(
        doc,
        sketch(SketchPlane::Datum { datum: DatumPlane::Front, offset: 0.0 }, |s| {
            s.polyline_loop(&[P2::new(-20.0, 0.0), P2::new(20.0, 0.0), P2::new(20.0, 10.0), P2::new(-20.0, 30.0)])
                .unwrap();
        }),
    );
    let both = ExtrudeOptions { direction2: Some(blind(20.0)), ..Default::default() };
    add(doc, extrude(s, ExtrudeOp::Boss, blind(20.0), false, both))
}

const WEDGE: f64 = 32000.0;

/// A face leaning like the wedge's top, its centroid above or below `z`.
fn sloping(doc: &Document, above: bool, z: f64) -> FaceId {
    let t = doc.kernel().topology(doc.body().unwrap()).unwrap();
    t.faces
        .iter()
        .find(|f| {
            f.normal.is_some_and(|n| (n.x - 0.5 / 1.25f64.sqrt()).abs() < 1e-9 && n.z > 0.0)
                && (f.centroid.z > z) == above
        })
        .unwrap()
        .id
}

fn from_surface(plane: PlaneRef) -> ExtrudeOptions {
    ExtrudeOptions { from: Some(ExtrudeStart::Surface { plane }), ..Default::default() }
}

fn top_square(doc: &mut Document) -> FeatureId {
    add(
        doc,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 6.0, 6.0).unwrap();
        }),
    )
}

#[test]
fn a_start_on_a_sloping_face_follows_it_flips_and_carries_a_later_feature() {
    let mut d = doc();
    wedge(&mut d);
    close(volume(&d), WEDGE, "the wedge");
    let face = d.face_ref(sloping(&d, false, 22.0)).unwrap();
    let s = top_square(&mut d);
    let from = from_surface(PlaneRef::Face { face });
    let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(5.0), false, from.clone()));
    all_ok(&d);
    // Whatever the slope, a sweep 5 mm along the direction from it holds area x depth.
    close(volume(&d), WEDGE + 180.0, "a 6 x 6 boss 5 mm from the slope");
    let t = d.kernel().topology(d.body().unwrap()).unwrap();
    let cap = t.face(sloping(&d, true, 22.0)).unwrap().centroid;
    close(cap.z, 25.0, "the end cap leans with the slope, 5 mm above it");
    close(cap.x, 0.0, "over the square");
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(8.0), false, from.clone()));
    close(volume(&d), WEDGE + 288.0, "an 8 mm boss");
    // Flipped as a cut, it goes into the wedge from the slope.
    edit(&mut d, e, extrude(s, ExtrudeOp::Cut, blind(8.0), true, from.clone()));
    all_ok(&d);
    close(volume(&d), WEDGE - 288.0, "an 8 mm pocket under the slope");
    // Through all from the slope down: the wedge is 20 mm tall on average under the square.
    edit(&mut d, e, extrude(s, ExtrudeOp::Cut, EndCondition::ThroughAll, true, from.clone()));
    all_ok(&d);
    close(volume(&d), WEDGE - 720.0, "a pocket from the slope to the bottom");
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), false, from));
    let mut back = reopened(&d);
    let cap = back.face_ref(sloping(&back, true, 22.0)).unwrap();
    assert_eq!(cap.feature, e, "the end cap is the boss's");
    // A later hole through all of it: 20 mm of wedge and 5 mm of boss on average.
    let cut = later_cut(&mut back, 4.0);
    assert_eq!(back.status(cut), Some(&FeatureStatus::Ok));
    close(volume(&back), WEDGE + 180.0 - std::f64::consts::PI * 4.0 * 25.0, "the later hole");
    reopened(&back);
}

#[test]
fn a_tilted_plane_or_another_bodys_face_starts_a_new_body() {
    let mut d = doc();
    wedge(&mut d);
    let face = d.face_ref(sloping(&d, false, 22.0)).unwrap();
    let plane = add(&mut d, FeatureKind::ReferencePlane(SketchPlane::FaceOffset { face: face.clone(), offset: 3.0 }));
    let s = top_square(&mut d);
    // Merge result off: the extrusion is the first feature of body 1.
    let own = |d: &mut Document, kind| {
        d.execute(Command::AddFeatureInBody { body: 1, kind, via_assistant: false, position: None })
            .unwrap()
            .created
            .unwrap()
    };
    let in_body = |d: &Document, key: u32| {
        let body = d.live_bodies().into_iter().find(|(body, _)| *body == key).unwrap().1;
        d.kernel().mass_properties(body).unwrap().volume
    };
    let e =
        own(&mut d, extrude(s, ExtrudeOp::Boss, blind(5.0), false, from_surface(PlaneRef::Feature { feature: plane })));
    all_ok(&d);
    close(in_body(&d, 1), 180.0, "a new body 5 mm from the tilted plane");
    close(in_body(&d, 0), WEDGE, "the wedge is untouched");
    // Direction 2 and Flip keep working from a tilted start.
    let both = ExtrudeOptions { direction2: Some(blind(2.0)), ..from_surface(PlaneRef::Feature { feature: plane }) };
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), true, both));
    all_ok(&d);
    close(in_body(&d, 1), 252.0, "5 mm one way and 2 mm the other");
    // A face of the other body starts it too.
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), false, from_surface(PlaneRef::Face { face })));
    all_ok(&d);
    close(in_body(&d, 1), 180.0, "a new body on the wedge's slope");
    // So does a vertex of the other body: the wedge's highest corner, 30 mm up.
    let t = d.kernel().topology(d.body_after(plane).unwrap()).unwrap();
    let top = t.vertices.iter().find(|v| (v.position.z - 30.0).abs() < 1e-9).unwrap().id;
    let vertex = d.vertex_ref(top).unwrap();
    let at_vertex = ExtrudeOptions { from: Some(ExtrudeStart::Vertex { vertex }), ..Default::default() };
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), false, at_vertex));
    all_ok(&d);
    close(in_body(&d, 1), 180.0, "a new body from the corner's height");
    let lowest = d.kernel().topology(d.body_after(e).unwrap()).unwrap();
    let floor = lowest.faces.iter().find(|f| f.normal.is_some_and(|n| n.z < -1.0 + 1e-9)).unwrap();
    close(floor.centroid.z, 30.0, "the new body starts at the corner's height");
    let back = Document::from_bytes(&d.to_bytes().unwrap(), kernel()).unwrap();
    assert_eq!(back.part_json(), d.part_json(), "the truth round-trips");
    assert_eq!(back.geometry_hash(), d.geometry_hash());
    all_ok(&back);
    close(in_body(&back, 1), 180.0, "the reopened new body");
    close(in_body(&back, 0), WEDGE, "the reopened wedge");
}

#[test]
fn starts_that_cannot_be_built_are_refused_by_name() {
    let mut d = doc();
    wedge(&mut d);
    let face = d.face_ref(sloping(&d, false, 22.0)).unwrap();
    let s = top_square(&mut d);
    let refused = |d: &Document, id: FeatureId, says: &str| match d.status(id) {
        Some(FeatureStatus::Failed(message)) => assert!(message.contains(says), "{message}"),
        other => panic!("refused, saying {says}: {other:?}"),
    };
    // The Front plane holds the Top sketch's normal, so the extrusion never leaves it.
    let front = from_surface(PlaneRef::Datum { datum: DatumPlane::Front, offset: 3.0 });
    let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(5.0), false, front));
    refused(&d, e, "contains the extrusion direction");
    // Up to next is found only from a parallel start.
    let slope = from_surface(PlaneRef::Face { face });
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, EndCondition::UpToNext, false, slope.clone()));
    refused(&d, e, "Up to next");
    // A curved face starts a set length, but not Up to next or Up to face.
    let mut c = doc();
    let round = add(
        &mut c,
        sketch(SketchPlane::Datum { datum: DatumPlane::Front, offset: 0.0 }, |s| {
            s.circle_at(P2::new(0.0, 0.0), 20.0).unwrap();
        }),
    );
    let both = ExtrudeOptions { direction2: Some(blind(20.0)), ..Default::default() };
    add(&mut c, extrude(round, ExtrudeOp::Boss, blind(20.0), false, both));
    let t = c.kernel().topology(c.body().unwrap()).unwrap();
    let curved = c.face_ref(t.faces.iter().find(|f| f.normal.is_none()).unwrap().id).unwrap();
    let s2 = top_square(&mut c);
    let on_curve = add(
        &mut c,
        extrude(
            s2,
            ExtrudeOp::Boss,
            EndCondition::UpToNext,
            false,
            from_surface(PlaneRef::Face { face: curved.clone() }),
        ),
    );
    refused(&c, on_curve, "Up to next and Up to face cannot start on a curved face");
    let back = Document::from_bytes(&c.to_bytes().unwrap(), kernel()).unwrap();
    assert_eq!(back.part_json(), c.part_json(), "the refused start is saved as it was");
    // A torus takes only a ring-shaped profile around its axis: the reason is the kernel's.
    let mut r = doc();
    let ring = add(
        &mut r,
        sketch(SketchPlane::Datum { datum: DatumPlane::Front, offset: 0.0 }, |s| {
            s.circle_at(P2::new(30.0, 0.0), 10.0).unwrap();
        }),
    );
    add(&mut r, revolve(ring));
    let torus = r.face_ref(curved_faces(&r)[0]).unwrap();
    let s3 = add(
        &mut r,
        sketch(top(-20.0), |s| {
            s.rectangle(P2::new(30.0, 0.0), 4.0, 4.0).unwrap();
        }),
    );
    let on_torus =
        add(&mut r, extrude(s3, ExtrudeOp::Boss, blind(2.0), false, from_surface(PlaneRef::Face { face: torus })));
    refused(&r, on_torus, "features.");
    // A parallel plane still starts it as before.
    let parallel = from_surface(PlaneRef::Datum { datum: DatumPlane::Top, offset: 31.0 });
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), false, parallel));
    all_ok(&d);
    close(bounds(&d).1.z, 36.0, "a parallel plane 31 mm up is the start");
}

/// A rod of radius 20 along y from 0 to -40, made from one circle: Catallax proves a
/// start or limit on its round face.
fn rod(doc: &mut Document) -> FeatureId {
    let s = add(
        doc,
        sketch(SketchPlane::Datum { datum: DatumPlane::Front, offset: 0.0 }, |s| {
            s.circle_at(P2::new(0.0, 0.0), 40.0).unwrap();
        }),
    );
    add(doc, extrude(s, ExtrudeOp::Boss, blind(40.0), false, ExtrudeOptions::default()))
}

const ROD: f64 = 16000.0 * std::f64::consts::PI;

/// The area under the rod's top over 2 <= x <= 8, above its axis.
fn under_rod() -> f64 {
    let f = |x: f64| (x * (400.0 - x * x).sqrt() + 400.0 * (x / 20.0).asin()) / 2.0;
    f(8.0) - f(2.0)
}

fn curved_faces(doc: &Document) -> Vec<FaceId> {
    let t = doc.kernel().topology(doc.body().unwrap()).unwrap();
    t.faces.iter().filter(|f| f.normal.is_none()).map(|f| f.id).collect()
}

/// A 6 mm square on a plane `z` above the Top plane, 2 to 8 mm along x, over the rod.
fn square_over(doc: &mut Document, z: f64) -> FeatureId {
    add(
        doc,
        sketch(top(z), |s| {
            s.rectangle(P2::new(5.0, -20.0), 6.0, 6.0).unwrap();
        }),
    )
}

fn revolve(sketch: FeatureId) -> FeatureKind {
    FeatureKind::Revolve(soliddraft_model::RevolveFeature {
        sketch,
        axis_origin: [0.0, 0.0, 0.0],
        axis_direction: [0.0, 0.0, 1.0],
        angle_deg: 360.0,
    })
}

fn top_z(doc: &Document) -> f64 {
    doc.kernel().topology(doc.body().unwrap()).unwrap().bbox.max.z
}

#[test]
fn a_start_on_a_curved_face_follows_it_flips_merges_and_carries_a_later_feature() {
    let mut d = doc();
    rod(&mut d);
    close(volume(&d), ROD, "the rod");
    let face = d.face_ref(curved_faces(&d)[0]).unwrap();
    let s = square_over(&mut d, 30.0);
    let from = from_surface(PlaneRef::Face { face: face.clone() });
    // Flipped, the lines go down and first cross the rod's top; Direction 2 goes up from it.
    let out = ExtrudeOptions { direction2: Some(blind(5.0)), ..from.clone() };
    let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(5.0), true, out.clone()));
    all_ok(&d);
    // Between the face and the face moved 5 mm lie 36 x 5; the downward half is inside.
    close(volume(&d), ROD + 180.0, "a 6 x 6 boss 5 mm out of the rod's top");
    let peak = 396f64.sqrt() + 5.0;
    assert!((top_z(&d) - peak).abs() <= 0.011, "the boss top follows the rod: {}", top_z(&d));
    // As a cut the flipped sweep takes a 5 mm layer under the top.
    edit(&mut d, e, extrude(s, ExtrudeOp::Cut, blind(5.0), true, from.clone()));
    all_ok(&d);
    close(volume(&d), ROD - 180.0, "a flipped cut 5 mm under the top");
    // Unflipped, the lines go up and first cross the rod's underside.
    edit(&mut d, e, extrude(s, ExtrudeOp::Cut, blind(5.0), false, from.clone()));
    all_ok(&d);
    close(volume(&d), ROD - 180.0, "a cut 5 mm above the underside");
    // Through all from the top goes down through the whole rod.
    edit(&mut d, e, extrude(s, ExtrudeOp::Cut, EndCondition::ThroughAll, true, from.clone()));
    all_ok(&d);
    close(volume(&d), ROD - 12.0 * under_rod(), "the column through the rod");
    // Up to next and Up to face from a curved face are refused by name.
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, EndCondition::UpToNext, true, from.clone()));
    match d.status(e) {
        Some(FeatureStatus::Failed(message)) => {
            assert!(message.contains("cannot start on a curved face"), "{message}");
            assert!(!message.contains("Catallax") && !message.contains("kernel"), "plain words: {message}");
        }
        other => panic!("refused: {other:?}"),
    }
    // Back to the boss, then a later cut through the part, then a deeper boss under it.
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), true, out.clone()));
    let c = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.circle_at(P2::new(-10.0, -20.0), 2.0).unwrap();
        }),
    );
    let both = ExtrudeOptions { direction2: Some(EndCondition::ThroughAll), ..Default::default() };
    let later = add(&mut d, extrude(c, ExtrudeOp::Cut, EndCondition::ThroughAll, false, both));
    all_ok(&d);
    // The hole at x = -10 is 33.4 to 35.7 mm long across its 2 mm.
    let removed = ROD + 180.0 - volume(&d);
    let pi = std::f64::consts::PI;
    assert!(removed > pi * 33.0 && removed < pi * 36.0, "the later cut takes a 2 mm hole: {removed}");
    let deeper = ExtrudeOptions { direction2: Some(blind(7.0)), ..from.clone() };
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), true, deeper));
    all_ok(&d);
    close(volume(&d), ROD + 252.0 - removed, "the deeper boss beside the later cut");
    let back = reopened(&d);
    assert_eq!(back.status(later), Some(&FeatureStatus::Ok));
}

#[test]
fn a_curved_start_on_another_body_makes_a_new_body() {
    let mut d = doc();
    rod(&mut d);
    let face = d.face_ref(curved_faces(&d)[0]).unwrap();
    let s = square_over(&mut d, 30.0);
    let out = ExtrudeOptions { direction2: Some(blind(5.0)), ..from_surface(PlaneRef::Face { face }) };
    // Merge result off: the extrusion is the first feature of body 1.
    let e = d
        .execute(Command::AddFeatureInBody {
            body: 1,
            kind: extrude(s, ExtrudeOp::Boss, blind(5.0), true, out),
            via_assistant: false,
            position: None,
        })
        .unwrap()
        .created
        .unwrap();
    all_ok(&d);
    let in_body = |d: &Document, key: u32| {
        let body = d.live_bodies().into_iter().find(|(body, _)| *body == key).unwrap().1;
        d.kernel().mass_properties(body).unwrap().volume
    };
    close(in_body(&d, 1), 360.0, "a new body 5 mm each side of the rod's top");
    close(in_body(&d, 0), ROD, "the rod is untouched");
    let back = Document::from_bytes(&d.to_bytes().unwrap(), kernel()).unwrap();
    assert_eq!(back.part_json(), d.part_json(), "the truth round-trips");
    assert_eq!(back.geometry_hash(), d.geometry_hash());
    assert_eq!(back.status(e), Some(&FeatureStatus::Ok));
    close(in_body(&back, 1), 360.0, "the reopened new body");
}

#[test]
fn up_to_a_curved_face_stops_on_it() {
    let mut d = doc();
    rod(&mut d);
    let face = d.face_ref(curved_faces(&d)[0]).unwrap();
    let s = square_over(&mut d, 30.0);
    add(&mut d, extrude(s, ExtrudeOp::Boss, EndCondition::UpToFace { face }, true, ExtrudeOptions::default()));
    all_ok(&d);
    close(volume(&d), ROD + 6.0 * (6.0 * 30.0 - under_rod()), "the column from 30 mm down to the rod");
    let back = Document::from_bytes(&d.to_bytes().unwrap(), kernel()).unwrap();
    assert_eq!(back.part_json(), d.part_json(), "the truth round-trips");
    assert_eq!(back.geometry_hash(), d.geometry_hash());
    all_ok(&back);
}

#[test]
fn spline_faces_start_extrusions_and_revolved_faces_are_refused_with_the_reason() {
    // A roof whose top is a quadratic spline from (10, 10) over (0, 20) to (-10, 10).
    let mut d = doc();
    let roof = add(
        &mut d,
        sketch(SketchPlane::Datum { datum: DatumPlane::Front, offset: 0.0 }, |s| {
            s.add(EntityKind::Line { a: P2::new(-10.0, 10.0), b: P2::new(-10.0, 0.0) }, false);
            s.add(EntityKind::Line { a: P2::new(-10.0, 0.0), b: P2::new(10.0, 0.0) }, false);
            s.add(EntityKind::Line { a: P2::new(10.0, 0.0), b: P2::new(10.0, 10.0) }, false);
            let points = vec![P2::new(10.0, 10.0), P2::new(0.0, 20.0), P2::new(-10.0, 10.0)];
            s.add(EntityKind::Spline { mode: soliddraft_model::sketch::SplineMode::Control, points }, false);
        }),
    );
    add(&mut d, extrude(roof, ExtrudeOp::Boss, blind(20.0), false, ExtrudeOptions::default()));
    let before = volume(&d);
    let face = d.face_ref(curved_faces(&d)[0]).unwrap();
    let s = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(0.0, -10.0), 4.0, 4.0).unwrap();
        }),
    );
    let from = from_surface(PlaneRef::Face { face });
    // Merge result off: the boss is a body of its own on the spline face.
    d.execute(Command::AddFeatureInBody {
        body: 1,
        kind: extrude(s, ExtrudeOp::Boss, blind(4.0), false, from.clone()),
        via_assistant: false,
        position: None,
    })
    .unwrap();
    all_ok(&d);
    let in_body = |d: &Document, key: u32| {
        let body = d.live_bodies().into_iter().find(|(body, _)| *body == key).unwrap().1;
        d.kernel().mass_properties(body).unwrap().volume
    };
    close(in_body(&d, 1), 64.0, "a 4 x 4 boss 4 mm out of the spline roof");
    close(in_body(&d, 0), before, "the roof is untouched");
    let back = Document::from_bytes(&d.to_bytes().unwrap(), kernel()).unwrap();
    assert_eq!(back.part_json(), d.part_json(), "the truth round-trips");
    assert_eq!(back.geometry_hash(), d.geometry_hash());
    all_ok(&back);
    // Joined to the roof, the union along the spline face is refused with its reason.
    let merged = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(4.0), false, from));
    match d.status(merged) {
        Some(FeatureStatus::Failed(message)) => {
            assert!(message.contains("intersect.bezier-plane-oblique"), "{message}")
        }
        other => panic!("refused with the reason: {other:?}"),
    }
    // A dome and a cone turned about z: Catallax has no proof for their faces yet.
    let dome = |s: &mut Sketch| {
        s.add(EntityKind::Line { a: P2::new(0.0, 20.0), b: P2::new(0.0, 0.0) }, false);
        s.add(EntityKind::Line { a: P2::new(0.0, 0.0), b: P2::new(20.0, 0.0) }, false);
        s.add(EntityKind::Arc { center: P2::new(0.0, 0.0), start: P2::new(20.0, 0.0), end: P2::new(0.0, 20.0) }, false);
    };
    let cone = |s: &mut Sketch| {
        s.polyline_loop(&[P2::new(0.0, 0.0), P2::new(20.0, 0.0), P2::new(0.0, 20.0)]).unwrap();
    };
    for (what, shape) in [("dome", &dome as &dyn Fn(&mut Sketch)), ("cone", &cone)] {
        let mut d = doc();
        let m = add(&mut d, sketch(SketchPlane::Datum { datum: DatumPlane::Front, offset: 0.0 }, shape));
        add(&mut d, revolve(m));
        let face = d.face_ref(curved_faces(&d)[0]).unwrap();
        let s = add(
            &mut d,
            sketch(top(0.0), |s| {
                s.rectangle(P2::new(5.0, 0.0), 6.0, 6.0).unwrap();
            }),
        );
        let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(5.0), false, from_surface(PlaneRef::Face { face })));
        match d.status(e) {
            Some(FeatureStatus::Failed(message)) => {
                assert!(message.contains("features.unsupported-curved-limit"), "{what}: {message}")
            }
            other => panic!("{what}: refused with the reason: {other:?}"),
        }
        let back = Document::from_bytes(&d.to_bytes().unwrap(), kernel()).unwrap();
        assert_eq!(back.part_json(), d.part_json(), "{what}: the refused start is kept");
    }
}

#[test]
fn format_6_is_written_only_for_options_and_older_files_still_open() {
    // Plain parts keep format 2, and several bodies format 3.
    let mut d = doc();
    let s = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 20.0, 10.0).unwrap();
        }),
    );
    let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(10.0), false, Default::default()));
    assert_eq!(d.truth()["format"], "soliddraft-part/2");
    let saved = d.truth();
    let kind = saved["features"][1]["kind"].as_object().unwrap();
    for key in ["from", "direction2", "thin", "contours"] {
        assert!(!kind.contains_key(key), "unset options write nothing: {key}");
    }
    let options = ExtrudeOptions { direction2: Some(blind(2.0)), ..Default::default() };
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(10.0), false, options));
    assert!(d.part_json().contains("\"direction2\""));
    let back = reopened(&d);
    // A file from a later version is refused rather than misread.
    let next = format!("soliddraft-part/{}", soliddraft_model::file::NEWEST_FORMAT + 1);
    let later = back.part_json().replace(soliddraft_model::file::OPTIONS_FORMAT, &next);
    assert!(Document::from_part_json(&later, kernel()).is_err(), "a {next} part is refused");
    // A format 2 file from before the options reads as an extrusion without them.
    let legacy = serde_json::json!({
        "format":"soliddraft-part/2", "name":"Legacy part", "units":"mm", "next_feature":2,
        "features":[{"id":1,"name":"Boss-Extrude1","kind":{"type":"extrude",
            "profile":{"from":"shape","datum":"top","shape":{"type":"rectangle","width":10.0,"height":10.0}},
            "operation":"boss","end":{"type":"blind","depth":10.0}}}]
    });
    let old = Document::from_part_json(&legacy.to_string(), kernel()).unwrap();
    let FeatureKind::Extrude(x) = &old.features()[0].kind else { panic!("an extrusion") };
    assert!(x.options.is_default());
    assert_eq!(serde_json::from_str::<serde_json::Value>(&old.part_json()).unwrap(), legacy, "written back unchanged");
    close(volume(&old), 1000.0, "the legacy block");
}

/// Adds a feature to body `key`: a body of its own when no feature has used the key.
fn own(doc: &mut Document, key: u32, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeatureInBody { body: key, kind, via_assistant: false, position: None })
        .unwrap()
        .created
        .unwrap()
}

fn live(doc: &Document, key: u32) -> soliddraft_kernel_api::BodyId {
    doc.live_bodies().into_iter().find(|(body, _)| *body == key).expect("the body is live").1
}

/// The volume and centre of mass of body `key`.
fn mass(doc: &Document, key: u32) -> (f64, Vec3) {
    let m = doc.kernel().mass_properties(live(doc, key)).unwrap();
    (m.volume, m.centroid)
}

/// How far above the slope, measured straight up, a plane `offset` mm off it lies: the
/// slope's normal leans 1 in 2 from the vertical.
fn above_slope(offset: f64) -> f64 {
    offset * 1.25f64.sqrt()
}

#[test]
fn editing_a_named_start_plane_moves_the_extrusion_before_the_part_is_saved() {
    let mut d = doc();
    wedge(&mut d);
    let face = d.face_ref(sloping(&d, false, 22.0)).unwrap();
    let plane_at = |offset| FeatureKind::ReferencePlane(SketchPlane::FaceOffset { face: face.clone(), offset });
    let plane = add(&mut d, plane_at(3.0));
    let s = top_square(&mut d);
    let from = from_surface(PlaneRef::Feature { feature: plane });
    let e = own(&mut d, 1, extrude(s, ExtrudeOp::Boss, blind(5.0), false, from.clone()));
    all_ok(&d);
    // Over the square the plane averages 20 mm plus its rise; the boss goes 5 mm up from it.
    let (v, c) = mass(&d, 1);
    close(v, 180.0, "a 6 x 6 boss 5 mm long");
    close(c.z, 20.0 + above_slope(3.0) + 2.5, "the boss starts on the plane 3 mm off the slope");
    // Only the plane changes. The sketch, the extrusion and its input stay as they were.
    edit(&mut d, plane, plane_at(5.0));
    all_ok(&d);
    let (v, c) = mass(&d, 1);
    close(v, 180.0, "the same boss");
    close(c.z, 20.0 + above_slope(5.0) + 2.5, "the boss follows the plane to 5 mm");
    let back = Document::from_bytes(&d.to_bytes().unwrap(), kernel()).unwrap();
    assert_eq!(back.geometry_hash(), d.geometry_hash(), "the part on screen is the part saved");
    close(mass(&back, 1).1.z, c.z, "the reopened boss");
    // Flip sends it down from the same plane: the centre moves 5 mm, through save and reopen.
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), true, from));
    all_ok(&d);
    let (v, c) = mass(&d, 1);
    close(v, 180.0, "the flipped boss");
    close(c.z, 20.0 + above_slope(5.0) - 2.5, "the flipped boss hangs under the plane");
    let back = Document::from_bytes(&d.to_bytes().unwrap(), kernel()).unwrap();
    assert_eq!(back.part_json(), d.part_json(), "the truth round-trips");
    assert_eq!(back.geometry_hash(), d.geometry_hash());
    all_ok(&back);
    close(mass(&back, 1).1.z, c.z, "the reopened flipped boss");
}

#[test]
fn up_to_a_face_that_crosses_a_tilted_start_is_refused_by_name() {
    let mut d = doc();
    wedge(&mut d);
    let face = d.face_ref(sloping(&d, false, 22.0)).unwrap();
    // On the slope: 18.5 to 21.5 mm up over the square. 5 mm under it: wholly below 20 mm.
    let on = add(&mut d, FeatureKind::ReferencePlane(SketchPlane::FaceOffset { face: face.clone(), offset: 0.0 }));
    let under = add(&mut d, FeatureKind::ReferencePlane(SketchPlane::FaceOffset { face, offset: -5.0 }));
    // A block 50 x 50 mm from 5 mm below the Top plane, cut off flat at 20 mm.
    let block = add(
        &mut d,
        sketch(top(-5.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 50.0, 50.0).unwrap();
        }),
    );
    add(&mut d, extrude(block, ExtrudeOp::Boss, blind(30.0), false, Default::default()));
    let lid = add(
        &mut d,
        sketch(top(20.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 60.0, 60.0).unwrap();
        }),
    );
    add(&mut d, extrude(lid, ExtrudeOp::Cut, blind(20.0), false, Default::default()));
    all_ok(&d);
    const BLOCK: f64 = 50.0 * 50.0 * 25.0;
    close(volume(&d), BLOCK, "the block cut flat at 20 mm");
    let flat = d.face_ref(face_facing(&d, Vec3::Z, 20.0)).unwrap();
    let up_to = EndCondition::UpToFace { face: flat };
    let s = top_square(&mut d);
    // From the plane under the slope up to the flat top: a pocket 5 mm deep across the
    // slope's normal, so area times the plane's rise below 20 mm.
    let below = from_surface(PlaneRef::Feature { feature: under });
    let e = add(&mut d, extrude(s, ExtrudeOp::Cut, up_to.clone(), false, below));
    all_ok(&d);
    close(volume(&d), BLOCK - 36.0 * above_slope(5.0), "a pocket from the tilted plane up to the top");
    // From the slope itself the top crosses the start: half the square starts above it.
    let refused = |d: &Document| match d.status(e) {
        Some(FeatureStatus::Failed(message)) => {
            assert!(message.contains("crosses the start"), "{message}");
            close(volume(d), BLOCK, "nothing is built");
        }
        other => panic!("refused by name: {other:?}"),
    };
    let crossing = from_surface(PlaneRef::Feature { feature: on });
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, up_to.clone(), false, crossing.clone()));
    refused(&d);
    // Direction 2 meets the same limit the other way round.
    let second = ExtrudeOptions { direction2: Some(up_to), ..crossing };
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(2.0), true, second));
    refused(&d);
    let back = Document::from_bytes(&d.to_bytes().unwrap(), kernel()).unwrap();
    assert_eq!(back.part_json(), d.part_json(), "the refused extrusion is kept");
    refused(&back);
}

#[test]
fn a_start_on_a_face_split_into_coplanar_pieces_uses_their_plane() {
    let mut d = doc();
    wedge(&mut d);
    let whole = d.face_ref(sloping(&d, false, 22.0)).unwrap();
    // A slot 2 mm wide across the slope at x = 10, from 14 mm up, splits it in two
    // pieces in one plane: 80 cubic millimetres.
    let slot = add(
        &mut d,
        sketch(top(14.0), |s| {
            s.rectangle(P2::new(10.0, 0.0), 2.0, 60.0).unwrap();
        }),
    );
    add(&mut d, extrude(slot, ExtrudeOp::Cut, blind(20.0), false, Default::default()));
    all_ok(&d);
    close(volume(&d), WEDGE - 80.0, "the slot");
    // A piece is named as the viewport names a pick to start from: by the plane it shares.
    let left = d.plane_ref(sloping(&d, true, 14.0)).unwrap();
    let right = d.plane_ref(sloping(&d, false, 14.0)).unwrap();
    // The slope picked before the split, or either piece picked after it, starts the boss.
    let s = top_square(&mut d);
    let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(5.0), false, from_surface(PlaneRef::Face { face: whole })));
    for face in [None, Some(left), Some(right)] {
        if let Some(face) = face {
            edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(5.0), false, from_surface(PlaneRef::Face { face })));
        }
        all_ok(&d);
        close(volume(&d), WEDGE - 80.0 + 180.0, "a 6 x 6 boss 5 mm from the slope");
        let t = d.kernel().topology(d.body().unwrap()).unwrap();
        let leaning = |n: Vec3| (n.x - 0.5 / 1.25f64.sqrt()).abs() < 1e-9 && n.z > 0.0;
        let cap = t.faces.iter().find(|f| f.normal.is_some_and(leaning) && f.centroid.x.abs() < 1e-6).unwrap();
        close(cap.centroid.z, 25.0, "the end cap leans with the slope, 5 mm above it");
    }
    reopened(&d);
}

#[test]
fn a_face_that_a_combine_carried_onto_another_body_starts_a_new_body() {
    let mut d = doc();
    wedge(&mut d);
    // A block 20 x 20 x 8 mm as body 1, half in the wedge, then joined to it.
    let block = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(25.0, 0.0), 20.0, 20.0).unwrap();
        }),
    );
    own(&mut d, 1, extrude(block, ExtrudeOp::Boss, blind(8.0), false, Default::default()));
    let combine = own(&mut d, 0, FeatureKind::Combine { operation: BooleanOp::Union, tool: 1 });
    all_ok(&d);
    assert_eq!(d.live_bodies().len(), 1, "the block is part of the wedge");
    close(mass(&d, 0).0, WEDGE + 15.0 * 20.0 * 8.0, "the wedge and the block outside it");
    // The block's top, on the joined body, starts a new body 8 mm up.
    let t = d.kernel().topology(live(&d, 0)).unwrap();
    let top_face = t
        .faces
        .iter()
        .find(|f| f.normal.is_some_and(|n| n.z > 1.0 - 1e-9) && (f.centroid.z - 8.0).abs() < 1e-9)
        .unwrap();
    let face = d.face_ref(top_face.id).unwrap();
    let s = top_square(&mut d);
    let e = own(&mut d, 2, extrude(s, ExtrudeOp::Boss, blind(5.0), false, from_surface(PlaneRef::Face { face })));
    assert_eq!(d.status(e), Some(&FeatureStatus::Ok), "the start face is on the joined body");
    let (v, c) = mass(&d, 2);
    close(v, 180.0, "a new body 5 mm long");
    close(c.z, 10.5, "from the block's top at 8 mm");
    close(mass(&d, 0).0, WEDGE + 2400.0, "the joined body is untouched");
    assert!(d.body_features(0).contains(&combine));
    let back = Document::from_bytes(&d.to_bytes().unwrap(), kernel()).unwrap();
    assert_eq!(back.part_json(), d.part_json(), "the truth round-trips");
    assert_eq!(back.geometry_hash(), d.geometry_hash());
    all_ok(&back);
    close(mass(&back, 2).1.z, 10.5, "the reopened new body");
}
