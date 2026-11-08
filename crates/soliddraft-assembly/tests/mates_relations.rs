//! Symmetric, Linear Coupler, Limit Distance, a free Width, Profile Center and Slot mates on
//! real parts, through the commands the panel and the MCP tools call. Each places exactly or
//! is refused by name, counts what it leaves free, and survives a save and a reopen.
//!
//! A is held at the origin by O. Blocks are centred in plan and rise from z = 0; shafts
//! run along their +Z from the origin.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{DQuat, DVec3};
use soliddraft_assembly::{
    Alignment, AssemblyDocument, ComponentId, EndpointSelector, FaceSelector, KernelSource, Mate, MateEdit, MateEntry,
    OccurrencePath, OpenOptions, Placement, RefusalCode, SlotInput, Step, StoredSupport, WidthInput,
};
use soliddraft_document::{CacheOnlyStore, DocRef, identify_file};
use soliddraft_kernel_api::{Kernel, SurfaceKind, Topology, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::tools::polygon_corners;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FaceRef, FeatureId, FeatureKind,
    ProfileSource, SketchFeature, SketchPlane,
};

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-mates-relations-{}-{name}", std::process::id()));
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

fn add(doc: &mut Document, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
        .unwrap()
        .created
        .unwrap()
}

fn sketch(doc: &mut Document, draw: impl FnOnce(&mut Sketch)) -> FeatureId {
    let mut sketch = Sketch::new();
    draw(&mut sketch);
    solve(&mut sketch, None, true);
    let plane = SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 };
    add(doc, FeatureKind::Sketch(SketchFeature { plane, sketch }))
}

fn extrude(doc: &mut Document, s: FeatureId, operation: ExtrudeOp, depth: f64) {
    add(
        doc,
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: s },
            operation,
            end: EndCondition::Blind { depth },
            reverse: false,
        }),
    );
}

/// A part made of one sketch extruded `h` up from the top plane.
fn prism(path: &Path, h: f64, draw: impl FnOnce(&mut Sketch)) {
    let mut doc = Document::new("Part", kernel());
    let s = sketch(&mut doc, draw);
    extrude(&mut doc, s, ExtrudeOp::Boss, h);
    doc.save(path).unwrap();
}

fn block(w: f64, d: f64, h: f64) -> impl Fn(&Path) {
    move |p| prism(p, h, |s| drop(s.rectangle(P2::new(0.0, 0.0), w, d).unwrap()))
}

fn shaft(d: f64, h: f64) -> impl Fn(&Path) {
    move |p| prism(p, h, |s| drop(s.circle_at(P2::new(0.0, 0.0), d).unwrap()))
}

/// A regular polygon of `sides` with a corner `r` out along +X.
fn polygon(sides: u32, r: f64, h: f64) -> impl Fn(&Path) {
    move |p| {
        let corners = polygon_corners(P2::new(0.0, 0.0), P2::new(r, 0.0), sides).unwrap();
        prism(p, h, |s| drop(s.polyline_loop(&corners).unwrap()))
    }
}

/// A 60 x 30 x 10 block with a straight slot cut through it along X: 30 long and 8 wide,
/// so its round ends' axes stand at x = -11 and x = 11.
fn slotted(path: &Path) {
    let mut doc = Document::new("Slotted", kernel());
    let s = sketch(&mut doc, |s| drop(s.rectangle(P2::new(0.0, 0.0), 60.0, 30.0).unwrap()));
    extrude(&mut doc, s, ExtrudeOp::Boss, 10.0);
    let s = sketch(&mut doc, |s| drop(s.slot(P2::new(0.0, 0.0), 30.0, 8.0).unwrap()));
    extrude(&mut doc, s, ExtrudeOp::Cut, 10.0);
    doc.save(path).unwrap();
}

fn reference(from: &Path, to: &Path) -> DocRef {
    let i = identify_file(to).unwrap();
    let mut r = DocRef::new(i.id, i.kind, "", i.revision);
    r.set_path(from, to);
    r
}

/// A part, read the way a pick reads it.
struct Part {
    doc: Document,
    topology: Topology,
}

impl Part {
    fn open(path: &Path) -> Part {
        let doc = Document::open(path, kernel()).unwrap();
        let topology = doc.kernel().topology(doc.body().unwrap()).unwrap();
        Part { doc, topology }
    }

    /// The faces of a kind, by centroid x then y.
    fn faces(&self, kind: SurfaceKind) -> Vec<FaceRef> {
        let mut found: Vec<_> = self.topology.faces.iter().filter(|f| f.surface == kind).collect();
        found.sort_by(|a, b| (a.centroid.x, a.centroid.y).partial_cmp(&(b.centroid.x, b.centroid.y)).unwrap());
        found.iter().map(|f| self.doc.face_ref(f.id).unwrap()).collect()
    }

    /// The planar face whose outward normal is `n`.
    fn plane(&self, n: [f64; 3]) -> FaceRef {
        let n = Vec3::new(n[0], n[1], n[2]);
        let found: Vec<_> = self.topology.faces.iter().filter(|f| f.normal == Some(n)).collect();
        assert_eq!(found.len(), 1, "one face looks along {n:?}");
        self.doc.face_ref(found[0].id).unwrap()
    }
}

const X: [f64; 3] = [1.0, 0.0, 0.0];
const NX: [f64; 3] = [-1.0, 0.0, 0.0];
const Y: [f64; 3] = [0.0, 1.0, 0.0];
const Z: [f64; 3] = [0.0, 0.0, 1.0];
const NZ: [f64; 3] = [0.0, 0.0, -1.0];

struct Rig {
    dir: PathBuf,
    root: PathBuf,
    doc: AssemblyDocument,
    ids: Vec<ComponentId>,
    parts: Vec<Part>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Each part made by its maker and inserted where it says; the first is held at the
/// origin by O.
fn rig(name: &str, parts: &[(&dyn Fn(&Path), Placement)]) -> Rig {
    let dir = folder(name);
    let root = dir.join("rig.dwasm");
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&root)).unwrap();
    let (mut ids, mut opened) = (Vec::new(), Vec::new());
    for (i, (make, at)) in parts.iter().enumerate() {
        let file = dir.join(format!("p{i}.dwprt"));
        make(&file);
        ids.push(doc.insert(reference(&root, &file), *at).unwrap());
        opened.push(Part::open(&file));
    }
    doc.finish_loading();
    doc.mate_origin("O", ids[0], true, false).unwrap();
    doc.save(None).unwrap();
    Rig { dir, root, doc, ids, parts: opened }
}

fn at(x: f64, y: f64, z: f64) -> Placement {
    Placement::translation(DVec3::new(x, y, z))
}

impl Rig {
    fn placement(&self, i: usize) -> Placement {
        self.doc.assembly().component(self.ids[i]).unwrap().placement
    }

    fn face(&self, i: usize, n: [f64; 3]) -> EndpointSelector {
        EndpointSelector { component: self.ids[i], face: FaceSelector::Picked(self.parts[i].plane(n)) }
    }

    fn curved(&self, i: usize, kind: SurfaceKind, k: usize) -> EndpointSelector {
        EndpointSelector { component: self.ids[i], face: FaceSelector::Picked(self.parts[i].faces(kind)[k].clone()) }
    }

    fn datum(&self, i: usize, name: &str) -> EndpointSelector {
        EndpointSelector { component: self.ids[i], face: FaceSelector::Plane(name.into()) }
    }

    fn dof(&mut self) -> usize {
        self.doc.solver().remaining_dof
    }

    fn path(&self, i: usize) -> OccurrencePath {
        OccurrencePath::root().child(Step::component(self.ids[i]))
    }

    /// Saves, reopens and checks the reopened truth and freedoms match.
    fn reopen(&mut self) {
        self.doc.save(None).unwrap();
        let mut reopened = AssemblyDocument::open(&self.root, options()).unwrap();
        reopened.finish_loading();
        assert_eq!(reopened.assembly(), self.doc.assembly());
        assert_eq!(reopened.solver(), self.doc.solver());
        let _ = reopened.rebuild();
        reopened.finish_loading();
        assert_eq!(reopened.assembly(), self.doc.assembly(), "a rebuild proves again and moves nothing");
    }
}

fn last(doc: &AssemblyDocument) -> Mate {
    match doc.assembly().mates.last() {
        Some(MateEntry::Known(m)) => m.clone(),
        other => panic!("a known mate: {other:?}"),
    }
}

/// Where the local axis `n` of a placement points in the world.
fn turned(p: &Placement, n: [f64; 3]) -> DVec3 {
    (p.rotation * DVec3::from_array(n)).round()
}

/// C's +X face mirrors B's -X face about A's Right plane; two shafts mirror as lines.
#[test]
fn symmetric_mirrors_planes_and_axes_about_a_plane() {
    let small = block(10.0, 10.0, 10.0);
    let mut r = rig(
        "symmetric",
        &[(&block(40.0, 30.0, 10.0), Placement::IDENTITY), (&small, at(3.0, 7.0, 30.0)), (&small, at(50.0, -5.0, 2.0))],
    );
    let (b, c, mirror) = (r.face(1, NX), r.face(2, X), r.datum(0, "Right plane"));
    r.doc.mate_symmetric("S", [&b, &c, &mirror], Alignment::Aligned, false).unwrap();
    // B's face at x = -2 mirrors to x = 2: C, the later part, moves; B stays.
    assert_eq!(r.placement(1), at(3.0, 7.0, 30.0));
    assert_eq!(r.placement(2), at(-3.0, -5.0, 2.0));
    assert_eq!(r.dof(), 9, "B keeps six, C three");
    assert!(matches!(last(&r.doc), Mate::Symmetric { alignment: Some(Alignment::Aligned), .. }));
    r.reopen();
    // Opposed turns C over: its +X face looks along -X, its origin 5 the other side.
    r.doc.edit_mates(&[("S".into(), MateEdit::Alignment(Alignment::Opposed))]).unwrap();
    assert_eq!(turned(&r.placement(2), X), DVec3::NEG_X);
    assert_eq!(r.placement(2).translation, DVec3::new(7.0, -5.0, 2.0));
    // A cylinder is no symmetry plane, and a plane does not mirror an axis.
    let mut s = rig(
        "symmetric-refused",
        &[(&shaft(8.0, 30.0), Placement::IDENTITY), (&small, at(3.0, 7.0, 30.0)), (&small, at(50.0, -5.0, 2.0))],
    );
    let side = s.curved(0, SurfaceKind::Cylinder, 0);
    let before = s.doc.assembly().clone();
    let refused =
        s.doc.mate_symmetric("S", [&s.face(1, NX), &s.face(2, X), &side], Alignment::Aligned, false).unwrap_err();
    assert_eq!((refused.code, refused.mates.clone()), (RefusalCode::UnsupportedReference, vec!["S".to_owned()]));
    let refused = s.doc.mate_symmetric("S", [&s.face(1, NX), &s.face(1, X), &s.face(2, X)], Alignment::Aligned, false);
    assert_eq!(refused.unwrap_err().code, RefusalCode::Invalid, "the entity and its image are on two parts");
    assert_eq!(s.doc.assembly(), &before);

    let rod = shaft(8.0, 20.0);
    let mut r = rig(
        "symmetric-axes",
        &[(&block(40.0, 30.0, 10.0), Placement::IDENTITY), (&rod, at(3.0, 7.0, 30.0)), (&rod, at(50.0, -5.0, 2.0))],
    );
    let (b, c, mirror) =
        (r.curved(1, SurfaceKind::Cylinder, 0), r.curved(2, SurfaceKind::Cylinder, 0), r.datum(0, "Right plane"));
    r.doc.mate_symmetric("S", [&b, &c, &mirror], Alignment::Aligned, false).unwrap();
    // B's axis at x = 3 mirrors to x = -3; along the mirror C's axis matches B's y.
    assert_eq!(r.placement(2), at(-3.0, 7.0, 2.0));
    assert_eq!(r.dof(), 6 + 2, "C keeps its slide and spin along the axis");
    r.reopen();
}

/// B slides along X and C along Y, one to two. A distance that moves B moves C by twice as
/// much; reversing takes the parts where they are as the new zero.
#[test]
fn a_linear_coupler_moves_one_part_by_a_ratio_of_another() {
    let small = block(10.0, 10.0, 10.0);
    let mut r = rig(
        "coupler",
        &[(&block(40.0, 30.0, 10.0), Placement::IDENTITY), (&small, at(3.0, 7.0, 30.0)), (&small, at(50.0, -5.0, 2.0))],
    );
    let (b, c) = (r.face(1, X), r.face(2, Y));
    r.doc.mate_linear_coupler("K", &b, &c, [None, None], [1.0, 2.0], false, false).unwrap();
    assert_eq!((r.placement(1), r.placement(2)), (at(3.0, 7.0, 30.0), at(50.0, -5.0, 2.0)), "made where they are");
    // C's face lies 0 along Y and B's 8 along X: 1 x 0 less 2 x 8 is the exact offset.
    let Mate::LinearCoupler { offset, ratio, reverse, .. } = last(&r.doc) else { panic!("a coupler") };
    assert_eq!((offset.as_str(), ratio, reverse), ("-16", [1.0, 2.0], false));
    assert_eq!(r.dof(), 6 + 6 - 1);
    let (side, back) = (r.face(0, X), r.face(1, NX));
    r.doc.mate_distance("D", &side, &back, "0 mm", Alignment::Opposed, false).unwrap();
    assert_eq!(r.placement(1).translation.x, 25.0);
    assert_eq!(r.placement(2), at(50.0, 39.0, 2.0), "B slid 22, so C slides 44");
    r.reopen();
    // Reversed, the parts' places are the zero: nothing moves, and B sliding back slides C on.
    r.doc.edit_mates(&[("K".into(), MateEdit::Reverse(true))]).unwrap();
    assert_eq!(r.placement(2), at(50.0, 39.0, 2.0));
    r.doc.delete_mate("D").unwrap();
    r.doc.move_to(&[(r.path(1), at(20.0, 7.0, 30.0))]).unwrap();
    assert_eq!(r.placement(2), at(50.0, 49.0, 2.0));
    // A ratio of zero is no ratio.
    let refused = r.doc.edit_mates(&[("K".into(), MateEdit::Ratio([0.0, 1.0]))]).unwrap_err();
    assert_eq!(refused.code, RefusalCode::InvalidValue);
    // A part slides relative to another part: B along X from A's origin, C from B's.
    let mut s = rig(
        "coupler-reference",
        &[(&block(40.0, 30.0, 10.0), Placement::IDENTITY), (&small, at(3.0, 7.0, 30.0)), (&small, at(50.0, -5.0, 2.0))],
    );
    let (b, c) = (s.face(1, X), s.face(2, X));
    s.doc.mate_linear_coupler("K", &b, &c, [Some(s.ids[0]), Some(s.ids[1])], [1.0, 1.0], false, false).unwrap();
    s.doc.move_to(&[(s.path(1), at(13.0, 7.0, 30.0))]).unwrap();
    // C rides along with B (10) and slides on by as much again (10).
    assert_eq!(s.placement(2), at(70.0, -5.0, 2.0));
    s.reopen();
}

/// B's bottom stays 5 to 10 above A's top: brought down to 10, kept where a move inside the
/// range puts it, and stopped at an end by a move past it.
#[test]
fn a_limit_distance_keeps_a_gap_within_its_range() {
    let mut r = rig(
        "limit",
        &[(&block(40.0, 30.0, 10.0), Placement::IDENTITY), (&block(10.0, 10.0, 10.0), at(3.0, 7.0, 30.0))],
    );
    let (top, bottom) = (r.face(0, Z), r.face(1, NZ));
    r.doc.mate_limit_distance("L", &top, &bottom, ["5 mm", "10 mm"], Alignment::Opposed, false).unwrap();
    assert_eq!(r.placement(1), at(3.0, 7.0, 20.0));
    assert_eq!(r.dof(), 4, "the faces stay parallel; the gap is bounded, not fixed");
    let measured = r.doc.mate_measurements();
    assert_eq!(measured[0].1.as_ref().unwrap().as_ref().unwrap().value, 10.0);
    r.doc.move_to(&[(r.path(1), at(3.0, 7.0, 17.0))]).unwrap();
    assert_eq!(r.placement(1), at(3.0, 7.0, 17.0), "inside the range a move stands");
    r.doc.move_to(&[(r.path(1), at(3.0, 7.0, 2.0))]).unwrap();
    assert_eq!(r.placement(1), at(3.0, 7.0, 15.0), "past an end it stops there");
    r.reopen();
    r.doc.edit_mates(&[("L".into(), MateEdit::Max("25 mm".into()))]).unwrap();
    assert_eq!(r.placement(1), at(3.0, 7.0, 15.0));
    let refused = r.doc.edit_mates(&[("L".into(), MateEdit::Min("30 mm".into()))]).unwrap_err();
    assert_eq!(refused.code, RefusalCode::InvalidValue);
    // A distance outside the range is named with it.
    let before = r.doc.assembly().clone();
    let refused = r.doc.mate_distance("D", &top, &bottom, "40 mm", Alignment::Opposed, false).unwrap_err();
    assert_eq!(
        (refused.code, refused.mates.clone()),
        (RefusalCode::OverConstrained, vec!["D".to_owned(), "L".to_owned()])
    );
    assert_eq!(r.doc.assembly(), &before);
}

/// A free Width keeps B's X sides between A's, wherever B is moved; a tab wider than the
/// groove cannot fit.
#[test]
fn a_free_width_keeps_the_tab_between_the_groove_faces() {
    let mut r = rig(
        "width-free",
        &[(&block(40.0, 30.0, 10.0), Placement::IDENTITY), (&block(20.0, 10.0, 10.0), at(3.0, 7.0, 30.0))],
    );
    let (g0, g1, t0, t1) = (r.face(0, X), r.face(0, NX), r.face(1, X), r.face(1, NX));
    r.doc.mate_width("W", [&g0, &g1], [&t0, &t1], Alignment::Aligned, &WidthInput::Free, false).unwrap();
    assert_eq!(r.placement(1), at(3.0, 7.0, 30.0));
    assert_eq!(r.dof(), 4);
    r.doc.move_to(&[(r.path(1), at(15.0, 7.0, 30.0))]).unwrap();
    assert_eq!(r.placement(1), at(10.0, 7.0, 30.0));
    r.reopen();
    let mut wide = rig(
        "width-wide",
        &[(&block(40.0, 30.0, 10.0), Placement::IDENTITY), (&block(50.0, 10.0, 10.0), at(3.0, 7.0, 30.0))],
    );
    let (g0, g1, t0, t1) = (wide.face(0, X), wide.face(0, NX), wide.face(1, X), wide.face(1, NX));
    let refused = wide.doc.mate_width("W", [&g0, &g1], [&t0, &t1], Alignment::Aligned, &WidthInput::Free, false);
    let refused = refused.unwrap_err();
    assert_eq!((refused.code, refused.mates), (RefusalCode::OverConstrained, vec!["W".to_owned()]));
}

/// Rectangles, hexagons and circles centre on each other; polygons line their edges up and
/// circles keep their spin unless Lock rotation holds it.
#[test]
fn profile_center_centres_rectangles_polygons_and_circles() {
    let quarter = Placement::new(DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2), DVec3::new(3.0, 7.0, 30.0));
    let mut r = rig("profile", &[(&block(40.0, 30.0, 10.0), Placement::IDENTITY), (&block(10.0, 20.0, 5.0), quarter)]);
    let (top, bottom) = (r.face(0, Z), r.face(1, NZ));
    r.doc.mate_profile_center("P", &top, &bottom, Alignment::Opposed, "0 mm", false, false).unwrap();
    assert_eq!(r.placement(1).translation, DVec3::new(0.0, 0.0, 10.0));
    assert_eq!(turned(&r.placement(1), X), DVec3::Y, "it keeps its quarter turn: edges still line up");
    assert_eq!(r.dof(), 0);
    let Mate::ProfileCenter { b, .. } = last(&r.doc) else { panic!("a profile center") };
    assert_eq!(b.support, Some(StoredSupport::Profile { normal: [0, 0, -1], centre: [0.0, 0.0, 0.0], sides: Some(4) }));
    r.doc.edit_mates(&[("P".into(), MateEdit::Distance("2 mm".into()))]).unwrap();
    assert_eq!(r.placement(1).translation, DVec3::new(0.0, 0.0, 12.0));
    r.reopen();
    let refused = r.doc.edit_mates(&[("P".into(), MateEdit::LockRotation(true))]).unwrap_err();
    assert_eq!(refused.code, RefusalCode::Invalid, "a polygon is held by its edges");

    // Hexagons turned a quarter apart cannot line up that way: B takes a turn that does.
    let mut h =
        rig("profile-hexagon", &[(&polygon(6, 10.0, 10.0), Placement::IDENTITY), (&polygon(6, 6.0, 5.0), quarter)]);
    let (top, bottom) = (h.face(0, Z), h.face(1, NZ));
    h.doc.mate_profile_center("P", &top, &bottom, Alignment::Opposed, "0 mm", false, false).unwrap();
    let p = h.placement(1);
    // A hexagon's centre is the exact average of its corners as the part stores them, and the
    // sketch rounded those: it lies within 1e-15 of the origin, not on it.
    assert!(p.translation.distance(DVec3::new(0.0, 0.0, 10.0)) < 1e-15, "{p:?}");
    assert_eq!(p.translation.z, 10.0);
    assert!(turned(&p, X) == DVec3::X || turned(&p, X) == DVec3::NEG_X, "{p:?}");
    assert_eq!(h.dof(), 0);
    h.reopen();
    // A hexagon is not a rectangle.
    let mut m =
        rig("profile-mixed", &[(&polygon(6, 10.0, 10.0), Placement::IDENTITY), (&block(10.0, 20.0, 5.0), quarter)]);
    let (top, bottom) = (m.face(0, Z), m.face(1, NZ));
    let refused = m.doc.mate_profile_center("P", &top, &bottom, Alignment::Opposed, "0 mm", false, false).unwrap_err();
    assert_eq!(refused.code, RefusalCode::UnsupportedReference);
    assert!(refused.message.contains("a 6-sided polygon and a rectangle"), "{}", refused.message);
    // A rounded rectangle is none of the three.
    let rounded = |p: &Path| prism(p, 5.0, |s| drop(s.rounded_rectangle(P2::new(0.0, 0.0), 20.0, 10.0, 2.0).unwrap()));
    let mut o = rig("profile-rounded", &[(&block(40.0, 30.0, 10.0), Placement::IDENTITY), (&rounded, quarter)]);
    let (top, bottom) = (o.face(0, Z), o.face(1, NZ));
    let refused = o.doc.mate_profile_center("P", &top, &bottom, Alignment::Opposed, "0 mm", false, false).unwrap_err();
    assert_eq!(refused.code, RefusalCode::UnsupportedReference);
    assert!(refused.message.contains("rounded or chamfered"), "{}", refused.message);

    // Two shafts' end caps: B stacks on A coaxially, 2 mm up, spinning until locked.
    let mut c =
        rig("profile-circle", &[(&shaft(8.0, 30.0), Placement::IDENTITY), (&shaft(8.0, 20.0), at(3.0, 7.0, 40.0))]);
    let (top, bottom) = (c.face(0, Z), c.face(1, NZ));
    c.doc.mate_profile_center("P", &top, &bottom, Alignment::Opposed, "2 mm", false, false).unwrap();
    assert_eq!(c.placement(1), at(0.0, 0.0, 32.0));
    assert_eq!(c.dof(), 1, "the spin about the shared axis");
    c.doc.edit_mates(&[("P".into(), MateEdit::LockRotation(true))]).unwrap();
    assert_eq!(c.dof(), 0);
    c.reopen();
}

/// A pin of diameter 8 in A's slot: on the slot's mid-plane, and along it as the mode says.
#[test]
fn a_pin_in_a_slot_takes_each_position_mode() {
    let pin = shaft(8.0, 20.0);
    for (mode, x, dof) in [
        (SlotInput::Free, 3.0, 3),
        (SlotInput::Center, 0.0, 2),
        (SlotInput::Distance { distance: "4 mm".into(), flipped: false }, -7.0, 2),
        (SlotInput::Distance { distance: "4 mm".into(), flipped: true }, 7.0, 2),
        (SlotInput::Percent { percent: 25.0, flipped: false }, -5.5, 2),
    ] {
        let mut r = rig("slot", &[(&slotted, Placement::IDENTITY), (&pin, at(3.0, 7.0, 30.0))]);
        let (end, side) = (r.curved(0, SurfaceKind::Cylinder, 0), r.curved(1, SurfaceKind::Cylinder, 0));
        r.doc.mate_slot("P", &end, &side, Alignment::Aligned, &mode, false, false).unwrap();
        assert_eq!(r.placement(1), at(x, 0.0, 30.0), "{mode:?}");
        assert_eq!(r.dof(), dof, "{mode:?}");
    }
    let mut r = rig("slot-free", &[(&slotted, Placement::IDENTITY), (&pin, at(40.0, 7.0, 30.0))]);
    let (end, side) = (r.curved(0, SurfaceKind::Cylinder, 1), r.curved(1, SurfaceKind::Cylinder, 0));
    r.doc.mate_slot("P", &end, &side, Alignment::Aligned, &SlotInput::Free, true, false).unwrap();
    assert_eq!(r.placement(1), at(11.0, 0.0, 30.0), "stopped at the far end");
    assert_eq!(r.dof(), 2, "Lock rotation holds the spin");
    let Mate::Slot { slot, .. } = last(&r.doc) else { panic!("a slot") };
    let Some(StoredSupport::Slot { depth, ends, radius }) = &slot.support else { panic!("a stored slot") };
    assert_eq!(
        (depth[2].abs(), ends[0][0], ends[1][0], ends[0][1], ends[1][1], *radius),
        (1, -11.0, 11.0, 0.0, 0.0, 4.0)
    );
    r.doc.move_to(&[(r.path(1), at(-30.0, 0.0, 30.0))]).unwrap();
    assert_eq!(r.placement(1), at(-11.0, 0.0, 30.0));
    r.reopen();
    // A wall reads the same slot; the block's top is no slot.
    let mut w = rig("slot-wall", &[(&slotted, Placement::IDENTITY), (&pin, at(3.0, 7.0, 30.0))]);
    let wall = {
        let walls: Vec<_> = w.parts[0]
            .topology
            .faces
            .iter()
            .filter(|f| f.normal == Some(Vec3::new(0.0, -1.0, 0.0)) && f.centroid.y.abs() < 5.0)
            .collect();
        EndpointSelector {
            component: w.ids[0],
            face: FaceSelector::Picked(w.parts[0].doc.face_ref(walls[0].id).unwrap()),
        }
    };
    let side = w.curved(1, SurfaceKind::Cylinder, 0);
    w.doc.mate_slot("P", &wall, &side, Alignment::Aligned, &SlotInput::Center, false, false).unwrap();
    assert_eq!(w.placement(1), at(0.0, 0.0, 30.0));
    let before = w.doc.assembly().clone();
    let refused =
        w.doc.mate_slot("Q", &w.face(0, Z), &side, Alignment::Aligned, &SlotInput::Center, false, false).unwrap_err();
    assert_eq!(refused.code, RefusalCode::UnsupportedReference, "{}", refused.message);
    assert_eq!(w.doc.assembly(), &before);
}
