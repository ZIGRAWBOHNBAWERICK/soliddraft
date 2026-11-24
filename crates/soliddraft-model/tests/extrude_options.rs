//! An extrusion's From, Direction 2, thin wall and selected contours: each builds, edits,
//! saves in format 6, reopens to the same part and carries a later feature.

use std::sync::Arc;

use soliddraft_kernel_api::{FaceId, Kernel, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, Contour, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, ExtrudeOptions, ExtrudeStart,
    FeatureId, FeatureKind, FeatureStatus, Generator, ProfileSource, SketchFeature, SketchPlane, ThinFeature, ThinSide,
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

#[test]
fn a_start_on_a_plane_that_is_not_parallel_is_refused_by_name() {
    let mut d = doc();
    let s = add(
        &mut d,
        sketch(top(0.0), |s| {
            s.rectangle(P2::new(0.0, 0.0), 20.0, 10.0).unwrap();
        }),
    );
    let start = |datum| ExtrudeOptions {
        from: Some(ExtrudeStart::Surface { plane: soliddraft_model::PlaneRef::Datum { datum, offset: 3.0 } }),
        ..Default::default()
    };
    let e = add(&mut d, extrude(s, ExtrudeOp::Boss, blind(10.0), false, start(DatumPlane::Top)));
    all_ok(&d);
    close(bounds(&d).0.z, 3.0, "a parallel plane 3 mm up is the start");
    edit(&mut d, e, extrude(s, ExtrudeOp::Boss, blind(10.0), false, start(DatumPlane::Front)));
    match d.status(e) {
        Some(FeatureStatus::Failed(message)) => assert!(message.contains("coming soon"), "{message}"),
        other => panic!("a tilted start is refused: {other:?}"),
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
