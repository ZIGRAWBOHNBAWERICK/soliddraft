//! Interference and clearance checks on real parts, rebuilt on Catallax at their pinned
//! revisions: the three outcomes, exact touching from a Coincident mate, clearances proven
//! both ways, and what is refused.
//!
//! A is a 40 x 30 x D block and B a 20 x 10 x 6 block, both centred on the origin in plan
//! and rising along +Z. B starts at [3, 7, 30], 30 - D mm above A.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{DQuat, DVec3};
use soliddraft_assembly::{
    Alignment, AssemblyDocument, CheckCode, CheckRequest, CheckScope, ClearanceOutcome, ComponentId, EndpointSelector,
    FaceSelector, GeneratorChoice, KernelSource, OccurrencePath, OpenOptions, Outcome, Placement, Step,
};
use soliddraft_document::{CacheOnlyStore, DocRef, identify_file};
use soliddraft_kernel_api::{Kernel, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, PatternFeature, PatternSpec,
    ProfileSource, SketchFeature, SketchPlane,
};

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-interference-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn options() -> OpenOptions {
    OpenOptions {
        display: Some(Arc::new(KernelSource::new(kernel()))),
        revisions: Some(Arc::new(CacheOnlyStore::new())),
        check_freshness: true,
        ..OpenOptions::default()
    }
}

/// A `w` x `d` x `h` block, centred in plan and rising from z = 0.
fn new_block(path: &Path, w: f64, d: f64, h: f64) {
    let mut sketch = Sketch::new();
    sketch.rectangle(P2::new(0.0, 0.0), w, d).unwrap();
    new_extrusion(path, DatumPlane::Top, sketch, h);
}

/// A prism with the cross-section `corners` in (y, z), running along x from 0 to `length`.
fn new_prism(path: &Path, corners: &[[f64; 2]], length: f64) {
    let mut sketch = Sketch::new();
    let points: Vec<P2> = corners.iter().map(|[y, z]| P2::new(*y, *z)).collect();
    sketch.polyline_loop(&points).unwrap();
    new_extrusion(path, DatumPlane::Right, sketch, length);
}

/// A part made of one sketch on `datum` extruded by `h`.
fn new_extrusion(path: &Path, datum: DatumPlane, mut sketch: Sketch, h: f64) {
    let mut doc = Document::new("Block", kernel());
    solve(&mut sketch, None, true);
    let plane = SketchPlane::Datum { datum, offset: 0.0 };
    let add = |doc: &mut Document, kind| {
        doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
            .unwrap()
            .created
            .unwrap()
    };
    let s = add(&mut doc, FeatureKind::Sketch(SketchFeature { plane, sketch }));
    add(
        &mut doc,
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: s },
            operation: ExtrudeOp::Boss,
            end: EndCondition::Blind { depth: h },
            reverse: false,
        }),
    );
    doc.save(path).unwrap();
}

/// A hundred 1 mm cubes 20 mm apart along x, centred on x = 0, 20, ... 1980: one part,
/// made by a linear pattern of the first cube, whose body has a hundred separate pieces.
fn new_cubes(path: &Path) {
    new_block(path, 1.0, 1.0, 1.0);
    let mut doc = Document::open(path, kernel()).unwrap();
    let layout = PatternSpec::Linear { direction: Vec3::X, spacing: 20.0, count: 100 };
    let kind = FeatureKind::Pattern(PatternFeature { layout });
    doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false }).unwrap();
    doc.save(path).unwrap();
}

/// Sets a saved block's height, keeping its id and its extrusion.
fn set_height(path: &Path, h: f64) {
    let mut doc = Document::open(path, kernel()).unwrap();
    let id = doc.features()[1].id;
    let FeatureKind::Extrude(mut extrude) = doc.feature(id).unwrap().kind.clone() else { panic!("an extrusion") };
    extrude.end = EndCondition::Blind { depth: h };
    doc.execute(Command::EditFeature { id, kind: FeatureKind::Extrude(extrude) }).unwrap();
    doc.save(path).unwrap();
}

fn reference(from: &Path, to: &Path) -> DocRef {
    let i = identify_file(to).unwrap();
    let mut r = DocRef::new(i.id, i.kind, "", i.revision);
    r.set_path(from, to);
    r
}

fn face(component: ComponentId, generator: GeneratorChoice) -> EndpointSelector {
    EndpointSelector {
        component,
        face: FaceSelector::Generator { feature: "Boss-Extrude1".into(), generator, normal: None },
    }
}

struct Rig {
    a_file: PathBuf,
    doc: AssemblyDocument,
    a: ComponentId,
    b: ComponentId,
}

fn path(c: ComponentId) -> OccurrencePath {
    OccurrencePath::root().child(Step::component(c))
}

/// A of height `depth` at the origin and B at [3, 7, 30], saved.
fn rig(name: &str, depth: f64) -> Rig {
    let dir = folder(name);
    let (a_file, b_file, root) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("rig.dwasm"));
    new_block(&a_file, 40.0, 30.0, depth);
    new_block(&b_file, 20.0, 10.0, 6.0);
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&root)).unwrap();
    let a = doc.insert(reference(&root, &a_file), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference(&root, &b_file), Placement::translation(DVec3::new(3.0, 7.0, 30.0))).unwrap();
    doc.finish_loading();
    doc.save(None).unwrap();
    Rig { a_file, doc, a, b }
}

impl Rig {
    /// The origin on A, and B's underside on A's top, opposed.
    fn seat(&mut self) {
        self.doc.mate_origin("O", self.a, true, false).unwrap();
        let (top, bottom) = (face(self.a, GeneratorChoice::EndCap), face(self.b, GeneratorChoice::StartCap));
        self.doc.mate_coincident("Cz", &top, &bottom, Alignment::Opposed, false).unwrap();
    }

    fn check(&mut self, clearance: Option<f64>) -> soliddraft_assembly::CheckReport {
        self.doc.check_interference(&CheckRequest { clearance, ..CheckRequest::default() }).unwrap()
    }
}

#[test]
fn parts_apart_are_clear_on_their_pinned_revisions() {
    let mut rig = rig("apart", 10.0);
    let before = rig.doc.assembly().clone();
    let report = rig.check(None);
    assert_eq!((report.kind, report.tier), ("interference", "certified_mesh"));
    assert_eq!((report.pairs, report.clear, report.interferes, report.touching_or_undecided), (1, 1, 0, 0));
    assert!(report.results.is_empty() && report.complete && report.skipped.is_empty());
    assert_eq!(report.parts.len(), 2);
    for (part, c) in report.parts.iter().zip([rig.a, rig.b]) {
        assert_eq!(part.path, path(c));
        let pinned = rig.doc.assembly().source(rig.doc.assembly().component(c).unwrap().source).unwrap();
        assert_eq!(part.revision, pinned.document.pinned);
        assert!(part.triangles >= 12, "a block's certified mesh has every side");
    }
    assert!(rig.doc.parts_rebuilt_for_proofs() >= 2, "the parts are rebuilt from their pinned bytes");
    assert_eq!(rig.doc.assembly(), &before, "a check changes nothing");
    assert!(report.refused.iter().any(|r| r.starts_with("volume")));
}

#[test]
fn an_exact_coincident_mate_is_exact_touching() {
    let mut rig = rig("touching", 10.0);
    rig.seat();
    let report = rig.check(None);
    assert_eq!((report.interferes, report.touching_or_undecided), (0, 1));
    let r = &report.results[0];
    assert_eq!(r.outcome, Outcome::TouchingOrUndecided);
    assert_eq!((r.a.clone(), r.b.clone()), (path(rig.a), path(rig.b)));
    assert_eq!(r.contact.len(), 1, "{r:#?}");
    let contact = &r.contact[0];
    assert_eq!(contact.mate, "Cz");
    assert_eq!(contact.point[2], 10.0);
    // The shared point is on both faces: A's top spans [-20, 20] x [-15, 15], B's
    // underside [-7, 13] x [2, 12].
    assert!((-7.0..=13.0).contains(&contact.point[0]) && (2.0..=12.0).contains(&contact.point[1]));
    assert_eq!((r.distance.lo, r.distance.hi), (0.0, Some(0.0)));
    assert!(r.why.starts_with("Exactly touching"), "{}", r.why);
    assert!(!r.faces.is_empty());
    // Exact touching is at or under any clearance.
    let report = rig.check(Some(0.5));
    assert_eq!(report.results[0].clearance, Some(ClearanceOutcome::Below));
}

#[test]
fn a_part_sunk_into_another_interferes_at_a_proven_point() {
    let mut rig = rig("sunk", 10.0);
    rig.doc.mate_origin("O", rig.a, true, false).unwrap();
    let (top, bottom) = (face(rig.a, GeneratorChoice::EndCap), face(rig.b, GeneratorChoice::StartCap));
    rig.doc.mate_distance("Sink", &top, &bottom, "-1 mm", Alignment::Opposed, false).unwrap();
    let report = rig.check(None);
    assert_eq!(report.interferes, 1);
    let r = &report.results[0];
    assert_eq!(r.outcome, Outcome::Interferes);
    let w = r.witness.expect("a witness");
    assert!(w[2] > 9.0 && w[2] < 10.0, "{w:?}");
    assert!((-7.0..=13.0).contains(&w[0]) && (2.0..=12.0).contains(&w[1]), "{w:?}");
    assert!(r.contact.is_empty(), "a distance of -1 mm holds no faces on one plane");
}

#[test]
fn clearances_are_proven_met_below_or_left_undecided() {
    let mut rig = rig("clearance", 10.0);
    // B's underside is 20 mm above A's top.
    let below = rig.check(Some(25.0));
    assert_eq!((below.below, below.met, below.undecided), (Some(1), Some(0), Some(0)));
    let r = &below.results[0];
    assert_eq!((r.outcome, r.clearance), (Outcome::Clear, Some(ClearanceOutcome::Below)));
    assert!(r.distance.lo < 20.0 && r.distance.lo > 19.95, "{:?}", r.distance);
    assert!(r.distance.hi.is_some_and(|hi| hi > 20.0 && hi < 20.05), "{:?}", r.distance);
    let met = rig.check(Some(15.0));
    assert_eq!((met.met, met.below), (Some(1), Some(0)));
    assert!(met.results.is_empty());
    let undecided = rig.check(Some(20.0));
    assert_eq!(undecided.undecided, Some(1));
    assert_eq!(undecided.results[0].clearance, Some(ClearanceOutcome::Undecided));
}

#[test]
fn a_rotated_part_is_checked_where_it_is_placed() {
    let mut rig = rig("rotated", 10.0);
    // A quarter turn about x stands B on its side: it spans z in [25, 35] around [3, 7, 30],
    // and a quarter turn about z then lays it across A's top, still 15 mm above it.
    let turn =
        DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2) * DQuat::from_rotation_x(std::f64::consts::FRAC_PI_2);
    rig.doc.move_to(&[(path(rig.b), Placement::new(turn, DVec3::new(3.0, 7.0, 30.0)))]).unwrap();
    let report = rig.check(Some(16.0));
    let r = &report.results[0];
    assert_eq!(r.clearance, Some(ClearanceOutcome::Below));
    assert!(r.distance.lo > 14.9 && r.distance.lo < 15.0, "{:?}", r.distance);
    // Turned down into A, the same placement 10 mm lower interferes.
    rig.doc.move_to(&[(path(rig.b), Placement::new(turn, DVec3::new(3.0, 7.0, 12.0)))]).unwrap();
    assert_eq!(rig.check(None).interferes, 1);
}

#[test]
fn long_parts_a_hair_off_parallel_never_prove_a_clearance_they_miss() {
    // Two 20 m ridged bars, A's ridge up along x and B's down, crossed at the origin with
    // B turned 9e-7 rad about z and lifted 0.01 mm. The ridges meet at mid-length only, so
    // the parts are 0.01 mm apart there; their ridges' ends are about 0.013 mm apart.
    let dir = folder("ridges");
    let (a_file, b_file, root) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("ridges.dwasm"));
    const LENGTH: f64 = 20_000.0;
    new_prism(&a_file, &[[0.0, 0.0], [-10.0, -10.0], [10.0, -10.0]], LENGTH);
    new_prism(&b_file, &[[0.0, 0.0], [10.0, 10.0], [-10.0, 10.0]], LENGTH);
    let mut doc = AssemblyDocument::new_with("Ridges", options());
    doc.save(Some(&root)).unwrap();
    let mid = DVec3::new(LENGTH / 2.0, 0.0, 0.0);
    doc.insert(reference(&root, &a_file), Placement::translation(-mid)).unwrap();
    let b = doc.insert(reference(&root, &b_file), Placement::IDENTITY).unwrap();
    doc.finish_loading();
    for angle in [0.0, 9e-7, 1.1e-6] {
        let turn = DQuat::from_rotation_z(angle);
        let at = Placement::new(turn, DVec3::new(0.0, 0.0, 0.01) - turn * mid);
        doc.move_to(&[(path(b), at)]).unwrap();
        let check = |doc: &mut AssemblyDocument, clearance| {
            doc.check_interference(&CheckRequest {
                clearance: Some(clearance),
                chord: 0.001,
                ..CheckRequest::default()
            })
            .unwrap()
        };
        // The margin is twice 0.001 mm plus rounding, so 0.011 mm cannot be told apart.
        let report = check(&mut doc, 0.011);
        assert_eq!(report.undecided, Some(1), "angle {angle}: {report:#?}");
        let r = &report.results[0];
        assert!(r.distance.lo <= 0.01 && r.distance.hi.is_some_and(|hi| hi >= 0.01), "angle {angle}: {r:#?}");
        assert_eq!(check(&mut doc, 0.007).met, Some(1), "angle {angle}");
    }
}

#[test]
fn a_long_skinny_part_never_proves_a_clearance_it_misses() {
    // A is a 30 m bar 0.0625 mm wide and 1 mm deep, its top at z = 0 from x = 0 to 30000.
    // B is a 1 mm block 0.0078125 mm wide whose underside is 0.015625 mm over A's top
    // around x = 21000. Every coordinate is exact in f32, and the gap fails 0.018 mm.
    let dir = folder("skinny");
    let (a_file, b_file, root) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("skinny.dwasm"));
    new_block(&a_file, 30_000.0, 0.0625, 1.0);
    new_block(&b_file, 1.0, 0.0078125, 0.984375);
    let mut doc = AssemblyDocument::new_with("Skinny", options());
    doc.save(Some(&root)).unwrap();
    doc.insert(reference(&root, &a_file), Placement::translation(DVec3::new(15_000.0, 0.03125, -1.0))).unwrap();
    doc.insert(reference(&root, &b_file), Placement::translation(DVec3::new(21_000.5, 0.01953125, 0.015625))).unwrap();
    doc.finish_loading();
    let mut check = |clearance| {
        doc.check_interference(&CheckRequest { clearance: Some(clearance), chord: 0.001, ..CheckRequest::default() })
            .unwrap()
    };
    // The margin is twice 0.001 mm plus rounding, about 0.00205 mm: 0.015625 mm plus that is
    // under 0.018 mm, so the pair is proven below it.
    let report = check(0.018);
    assert_eq!((report.below, report.met), (Some(1), Some(0)), "{report:#?}");
    let r = &report.results[0];
    assert!(r.distance.lo <= 0.015625 && r.distance.hi.is_some_and(|hi| hi >= 0.015625), "{r:#?}");
    // And 0.015625 mm less the margin is over 0.012 mm.
    assert_eq!(check(0.012).met, Some(1));
}

#[test]
fn a_patterned_part_with_one_copy_inside_another_part_is_never_clear() {
    let dir = folder("pieces");
    let (a_file, b_file, root) = (dir.join("cubes.dwprt"), dir.join("b.dwprt"), dir.join("pieces.dwasm"));
    new_cubes(&a_file);
    new_block(&b_file, 11.0, 11.0, 11.0);
    let mut doc = AssemblyDocument::new_with("Pieces", options());
    doc.save(Some(&root)).unwrap();
    doc.insert(reference(&root, &a_file), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference(&root, &b_file), Placement::IDENTITY).unwrap();
    doc.finish_loading();
    let check = |doc: &mut AssemblyDocument| {
        doc.check_interference(&CheckRequest { clearance: Some(0.5), chord: 0.001, ..CheckRequest::default() }).unwrap()
    };
    // B, 11 mm on a side, around the first, a middle and the last copy: its surface is
    // 5 mm from the cube inside it and farther from the rest.
    for x in [0.0, 40.0, 1000.0, 1980.0] {
        doc.move_to(&[(path(b), Placement::translation(DVec3::new(x, 0.0, -5.0)))]).unwrap();
        let report = check(&mut doc);
        assert_eq!((report.interferes, report.below), (1, Some(1)), "B at x = {x}: {report:#?}");
        let w = report.results[0].witness.expect("a witness");
        assert!((w[0] - x).abs() < 0.5 && w[2] > 0.0 && w[2] < 1.0, "B at x = {x}: {w:?}");
    }
    // Between the first two copies, 4 mm from each, every piece is outside: clear.
    doc.move_to(&[(path(b), Placement::translation(DVec3::new(10.0, 0.0, -5.0)))]).unwrap();
    let report = check(&mut doc);
    assert_eq!((report.clear, report.met), (1, Some(1)), "{report:#?}");
}

#[test]
fn an_edit_after_the_pin_changes_nothing_until_update() {
    let mut rig = rig("pinned", 10.0);
    rig.seat();
    // A grows to 14 mm in its own file. The assembly still pins the 10 mm revision, so B
    // still rests on it, touching, and does not sink into the new A.
    set_height(&rig.a_file, 14.0);
    let report = rig.check(None);
    assert_eq!((report.interferes, report.touching_or_undecided), (0, 1));
    assert_eq!(report.results[0].contact.len(), 1);
}

#[test]
fn hidden_parts_join_only_when_asked() {
    let mut rig = rig("hidden", 10.0);
    rig.doc.set_hidden(&[path(rig.b)], true).unwrap();
    let refused = rig.doc.check_interference(&CheckRequest::default()).unwrap_err();
    assert_eq!(refused.code, CheckCode::TooFewParts);
    let report = rig.doc.check_interference(&CheckRequest { include_hidden: true, ..CheckRequest::default() }).unwrap();
    assert_eq!(report.pairs, 1);
}

#[test]
fn bad_requests_are_refused_by_name() {
    let mut rig = rig("refused", 10.0);
    let before = rig.doc.assembly().clone();
    let one = CheckRequest { scope: CheckScope::Occurrences(vec![path(rig.a)]), ..CheckRequest::default() };
    assert_eq!(rig.doc.check_interference(&one).unwrap_err().code, CheckCode::TooFewParts);
    let missing =
        CheckRequest { scope: CheckScope::Occurrences(vec![path(ComponentId(99))]), ..CheckRequest::default() };
    assert_eq!(rig.doc.check_interference(&missing).unwrap_err().code, CheckCode::Invalid);
    let coarse = CheckRequest { chord: 5.0, ..CheckRequest::default() };
    assert_eq!(rig.doc.check_interference(&coarse).unwrap_err().code, CheckCode::Invalid);
    let negative = CheckRequest { clearance: Some(-1.0), ..CheckRequest::default() };
    assert_eq!(rig.doc.check_interference(&negative).unwrap_err().code, CheckCode::Invalid);
    assert_eq!(rig.doc.assembly(), &before);
}

/// A 2^24 x 16 x 16 slab from x = 2^24 to 2^25, y = 0 to 16 and z = 0 to 16.
fn new_slab(path: &Path) {
    let mut sketch = Sketch::new();
    let (w, x) = (16_777_216.0, 25_165_824.0);
    sketch.rectangle(P2::new(x, 8.0), w, 16.0).unwrap();
    new_extrusion(path, DatumPlane::Top, sketch, 16.0);
}

/// B sits on A's top, turned 2^-42 about y: its underside's normal is 4.5e-13 from -z,
/// and far out along x every point of B would be above A, by at least 2^-17 mm. A proof
/// that read such a turn as the axis while the meshes kept the tilt would call a real gap
/// exact touching. So a turn this near a quarter turn is stored as the quarter turn
/// itself, and the proof, the solve and the meshes all read that one placement. The tilt
/// is written into the component's fields directly here, as no constructor would store it.
#[test]
fn a_turn_near_an_axis_is_not_exactly_touching() {
    let dir = folder("near-axis");
    let (a_file, b_file, root) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("rig.dwasm"));
    new_slab(&a_file);
    new_slab(&b_file);
    let mut doc = AssemblyDocument::new_with("Near", options());
    doc.save(Some(&root)).unwrap();
    let a = doc.insert(reference(&root, &a_file), Placement::translation(DVec3::new(0.0, 0.0, -16.0))).unwrap();
    let tilt = DQuat::from_xyzw(0.0, -(2f64.powi(-42)), 0.0, 1.0).normalize();
    assert_eq!(Placement::new(tilt, DVec3::ZERO), Placement::IDENTITY, "a constructor stores the quarter turn");
    let b = doc.insert(reference(&root, &b_file), Placement { rotation: tilt, translation: DVec3::ZERO }).unwrap();
    assert_eq!(doc.assembly().component(b).unwrap().placement.rotation, tilt);
    doc.finish_loading();
    let (top, bottom) = (face(a, GeneratorChoice::EndCap), face(b, GeneratorChoice::StartCap));
    // Every world placement is composed, and composing stores the quarter turn: the
    // proof reads B exactly level, as the meshes do.
    let measured = doc.measure_planes(&top, &bottom).unwrap();
    assert_eq!(measured.placements[1].rotation, DQuat::IDENTITY);
    assert_eq!(measured.rational, "0");
    let world = doc.check_interference(&CheckRequest::default()).unwrap();
    assert!(world.results.iter().all(|r| r.contact.is_empty()), "no mate, so no exact contact");

    // A Coincident turns B exactly onto the axes, where it does touch A. The Origin mate
    // brings A's base to z = 0, so the faces meet at z = 16.
    let clearance = Some(2f64.powi(-18));
    doc.mate_origin("O", a, true, false).unwrap();
    doc.mate_coincident("Cz", &top, &bottom, Alignment::Opposed, false).unwrap();
    let placed = doc.assembly().component(b).unwrap().placement;
    assert_eq!(placed.rotation, DQuat::IDENTITY, "the solve keeps no tilt it cannot prove");
    assert_eq!(placed.translation, DVec3::new(0.0, 0.0, 16.0));
    let measured = doc.measure_planes(&top, &bottom).unwrap();
    assert_eq!(measured.rational, "0");
    let report = doc.check_interference(&CheckRequest { clearance, ..CheckRequest::default() }).unwrap();
    let r = &report.results[0];
    assert_eq!(r.contact.len(), 1, "{r:#?}");
    assert_eq!(r.contact[0].point[2], 16.0);
    assert_eq!(r.clearance, Some(ClearanceOutcome::Below));

/// A root with A at the origin, B 100 mm along x and a sub-assembly S holding C 200 mm
/// along x, all apart, saved. Returns the files and the root's components.
struct Nested {
    dir: PathBuf,
    root: PathBuf,
    sub: PathBuf,
    b_file: PathBuf,
}

fn nested(name: &str) -> Nested {
    let dir = folder(name);
    let (a_file, b_file, c_file) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("c.dwprt"));
    let (sub, root) = (dir.join("sub.dwasm"), dir.join("root.dwasm"));
    for f in [&a_file, &b_file, &c_file] {
        new_block(f, 10.0, 10.0, 10.0);
    }
    let mut s = AssemblyDocument::new_with("Sub", options());
    s.save(Some(&sub)).unwrap();
    s.insert(reference(&sub, &c_file), Placement::translation(DVec3::new(200.0, 0.0, 0.0))).unwrap();
    s.finish_loading();
    s.save(None).unwrap();
    let mut doc = AssemblyDocument::new_with("Root", options());
    doc.save(Some(&root)).unwrap();
    doc.insert(reference(&root, &a_file), Placement::IDENTITY).unwrap();
    doc.insert(reference(&root, &b_file), Placement::translation(DVec3::new(100.0, 0.0, 0.0))).unwrap();
    doc.insert(reference(&root, &sub), Placement::IDENTITY).unwrap();
    doc.finish_loading();
    doc.save(None).unwrap();
    Nested { dir, root, sub, b_file }
}

impl Nested {
    fn open(&self) -> AssemblyDocument {
        AssemblyDocument::open(&self.root, options()).unwrap()
    }

    /// The root's sub-assembly occurrence.
    fn branch(doc: &AssemblyDocument) -> OccurrencePath {
        let c = doc.assembly().components.iter().find(|c| {
            doc.assembly().source(c.source).is_some_and(|s| s.document.kind == soliddraft_document::DocKind::Assembly)
        });
        path(c.expect("the sub-assembly").id)
    }
}

impl Drop for Nested {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A whole-assembly check is complete only when every part in scope is known: a
/// sub-assembly whose structure cannot be read is listed by path and cause, never dropped,
/// unless it is suppressed, or hidden and hidden parts are not asked for.
#[test]
fn an_unreadable_sub_assembly_leaves_the_check_incomplete() {
    // Healthy control: all three parts, nested C among them, are compared.
    let fx = nested("unread-sub");
    let mut doc = fx.open();
    let report = doc.check_interference(&CheckRequest::default()).unwrap();
    assert_eq!((report.parts.len(), report.pairs, report.clear), (3, 3, 3));
    assert!(report.complete && report.skipped.is_empty(), "{:?}", report.skipped);

    // The sub-assembly's file is gone, and the root is written without the copy of its
    // structure a save carries, so nothing holds it.
    let id = doc.id();
    doc.assembly().save(&fx.root, id, None, &[]).unwrap();
    std::fs::remove_file(&fx.sub).unwrap();
    let mut doc = fx.open();
    let branch = Nested::branch(&doc);
    let report = doc.check_interference(&CheckRequest::default()).unwrap();
    assert_eq!((report.parts.len(), report.pairs, report.clear), (2, 1, 1));
    assert!(!report.complete);
    assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
    assert_eq!(report.skipped[0].path, branch);
    assert_eq!(report.skipped[0].code, CheckCode::ProofUnavailable);
    assert!(report.skipped[0].message.contains("structure could not be read"), "{}", report.skipped[0].message);
    let clearance = CheckRequest { clearance: Some(1.0), ..CheckRequest::default() };
    let report = doc.check_interference(&clearance).unwrap();
    assert!(!report.complete && report.skipped[0].path == branch);

    // Hidden, it is out of scope like any hidden part, until hidden parts are asked for.
    doc.set_hidden(std::slice::from_ref(&branch), true).unwrap();
    let report = doc.check_interference(&CheckRequest::default()).unwrap();
    assert!(report.complete && report.skipped.is_empty());
    let hidden = CheckRequest { include_hidden: true, ..CheckRequest::default() };
    let report = doc.check_interference(&hidden).unwrap();
    assert!(!report.complete && report.skipped[0].path == branch);
    doc.set_hidden(std::slice::from_ref(&branch), false).unwrap();

    // Suppressed, it is not in the assembly's scope at all.
    doc.set_suppressed(std::slice::from_ref(&branch), true).unwrap();
    let report = doc.check_interference(&CheckRequest::default()).unwrap();
    assert!(report.complete && report.skipped.is_empty());
}

/// Control: a missing leaf part was already listed, and still is.
#[test]
fn a_missing_part_leaves_the_check_incomplete() {
    let fx = nested("missing-leaf");
    std::fs::remove_file(&fx.b_file).unwrap();
    let mut doc = fx.open();
    let report = doc.check_interference(&CheckRequest::default()).unwrap();
    assert!(!report.complete);
    assert_eq!(report.skipped.len(), 1, "{:?}", report.skipped);
    assert_eq!(report.skipped[0].path, path(ComponentId(report.skipped[0].path.steps()[0].component.0)));
    assert_eq!(report.parts.len(), 2);
}
