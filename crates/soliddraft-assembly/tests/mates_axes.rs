//! Mates on axes and points, on real parts through the commands the panel and the MCP tools
//! call: a shaft in a bore and a pin in a hole solve exactly, Lock rotation holds the spin,
//! a hole from a 60 degree circular pattern is refused by name, a shaft lies tangent on a
//! plane, a hinge seats a shaft, and vertices and edges mate to planes and to each other.
//!
//! Block A is 40 x 30 x 10, centred in plan and rising from z = 0, with a bore of
//! diameter 8 through it at (10, 5). Shaft B has diameter 8 and length 30 along its +Z.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{DQuat, DVec3};
use serde_json::json;
use soliddraft_assembly::{
    Alignment, AssemblyDocument, ComponentId, EndpointSelector, Entity, FaceSelector, KernelSource, Mate, MateEdit,
    MateEntry, OpenOptions, Placement, RefusalCode, StoredSupport,
};
use soliddraft_document::{CacheOnlyStore, DocRef, identify_file};
use soliddraft_kernel_api::{CurveKind, Kernel, SurfaceKind, Topology, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EdgeRef, EndCondition, ExtrudeFeature, ExtrudeOp, FaceRef, FeatureId, FeatureKind,
    PatternFeature, PatternSpec, ProfileSource, SketchFeature, SketchPlane, VertexRef,
};

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-mates-axes-{}-{name}", std::process::id()));
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

/// A 40 x 30 x 10 block with a bore of diameter 8 through it at (10, 5).
fn new_block_with_bore(path: &Path) {
    let mut doc = Document::new("Block", kernel());
    let s = sketch(&mut doc, |s| {
        s.rectangle(P2::new(0.0, 0.0), 40.0, 30.0).unwrap();
    });
    extrude(&mut doc, s, ExtrudeOp::Boss, 10.0);
    let s = sketch(&mut doc, |s| {
        s.circle_at(P2::new(10.0, 5.0), 8.0).unwrap();
    });
    extrude(&mut doc, s, ExtrudeOp::Cut, 10.0);
    doc.save(path).unwrap();
}

/// A shaft of diameter `d` and length `h` along +Z, its axis through the origin.
fn new_shaft(path: &Path, d: f64, h: f64) {
    let mut doc = Document::new("Shaft", kernel());
    let s = sketch(&mut doc, |s| {
        s.circle_at(P2::new(0.0, 0.0), d).unwrap();
    });
    extrude(&mut doc, s, ExtrudeOp::Boss, h);
    doc.save(path).unwrap();
}

/// A 10 x 10 x 5 tab at (20, 0) with a hole of diameter 4 through its middle, and a copy of
/// it turned `angle` degrees about Z.
fn new_tabs(path: &Path, angle: f64) {
    let mut doc = Document::new("Tabs", kernel());
    let s = sketch(&mut doc, |s| {
        s.rectangle(P2::new(20.0, 0.0), 10.0, 10.0).unwrap();
    });
    extrude(&mut doc, s, ExtrudeOp::Boss, 5.0);
    let s = sketch(&mut doc, |s| {
        s.circle_at(P2::new(20.0, 0.0), 4.0).unwrap();
    });
    extrude(&mut doc, s, ExtrudeOp::Cut, 5.0);
    let layout = PatternSpec::Circular { axis_origin: Vec3::ZERO, axis_direction: Vec3::Z, count: 2, angle };
    add(&mut doc, FeatureKind::Pattern(PatternFeature { layout, source: None }));
    doc.save(path).unwrap();
}

/// A `w` x `d` x `h` block, centred in plan and rising from z = 0.
fn new_block(path: &Path, w: f64, d: f64, h: f64) {
    let mut doc = Document::new("Block", kernel());
    let s = sketch(&mut doc, |s| {
        s.rectangle(P2::new(0.0, 0.0), w, d).unwrap();
    });
    extrude(&mut doc, s, ExtrudeOp::Boss, h);
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

    /// The vertex at `at`.
    fn vertex(&self, at: [f64; 3]) -> VertexRef {
        let at = Vec3::new(at[0], at[1], at[2]);
        let v = self.topology.vertices.iter().find(|v| v.position.distance(at) < 1e-9).expect("a vertex there");
        self.doc.vertex_ref(v.id).unwrap()
    }

    /// The straight edge whose midpoint is `at`.
    fn line(&self, at: [f64; 3]) -> EdgeRef {
        let at = Vec3::new(at[0], at[1], at[2]);
        let e = self
            .topology
            .edges
            .iter()
            .find(|e| e.curve == CurveKind::Line && e.midpoint.distance(at) < 1e-6)
            .expect("an edge there");
        self.doc.edge_ref(e.id).unwrap()
    }
}

fn picked(component: ComponentId, face: FaceRef) -> EndpointSelector {
    EndpointSelector { component, face: FaceSelector::Picked(face) }
}

struct Rig {
    dir: PathBuf,
    root: PathBuf,
    doc: AssemblyDocument,
    a: ComponentId,
    b: ComponentId,
    pa: Part,
    pb: Part,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A made by `make_a`, held at the origin by O, and B made by `make_b`, inserted at
/// (3, 7, 30).
fn rig(name: &str, make_a: impl FnOnce(&Path), make_b: impl FnOnce(&Path)) -> Rig {
    let dir = folder(name);
    let (a_file, b_file, root) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("rig.dwasm"));
    make_a(&a_file);
    make_b(&b_file);
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&root)).unwrap();
    let a = doc.insert(reference(&root, &a_file), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference(&root, &b_file), Placement::translation(DVec3::new(3.0, 7.0, 30.0))).unwrap();
    doc.finish_loading();
    doc.mate_origin("O", a, true, false).unwrap();
    doc.save(None).unwrap();
    let (pa, pb) = (Part::open(&a_file), Part::open(&b_file));
    Rig { dir, root, doc, a, b, pa, pb }
}

fn shaft_rig(name: &str) -> Rig {
    rig(name, new_block_with_bore, |p| new_shaft(p, 8.0, 30.0))
}

impl Rig {
    fn placement(&self, c: ComponentId) -> Placement {
        self.doc.assembly().component(c).unwrap().placement
    }

    fn bore(&self) -> EndpointSelector {
        let [wall] = &self.pa.faces(SurfaceKind::Cylinder)[..] else { panic!("one bore") };
        picked(self.a, wall.clone())
    }

    fn shaft(&self) -> EndpointSelector {
        let [side] = &self.pb.faces(SurfaceKind::Cylinder)[..] else { panic!("one side") };
        picked(self.b, side.clone())
    }

    fn free(&mut self) -> (usize, Vec<String>) {
        let s = self.doc.solver();
        (s.remaining_dof, s.free.iter().map(|f| f.motion.clone()).collect())
    }

    fn reopened(&self) -> AssemblyDocument {
        let mut doc = AssemblyDocument::open(&self.root, options()).unwrap();
        doc.finish_loading();
        doc
    }
}

fn close(a: DVec3, b: [f64; 3]) {
    assert!(a.distance(DVec3::from_array(b)) <= 1e-12, "{a} against {b:?}");
}

/// A shaft in a bore lands on the bore's axis exactly, keeps its height, and keeps the
/// slide and the spin along the axis. Seated flush with a Coincident, only the spin is left.
/// The status reads the stored axes after a save and a reopen.
#[test]
fn a_shaft_in_a_bore_solves_exactly() {
    let mut r = shaft_rig("shaft");
    let (bore, shaft) = (r.bore(), r.shaft());
    r.doc.mate_concentric("C", &bore, &shaft, Alignment::Aligned, false, false).unwrap();
    let p = r.placement(r.b);
    assert_eq!(p.rotation, DQuat::IDENTITY);
    assert_eq!(p.translation, DVec3::new(10.0, 5.0, 30.0));
    assert_eq!(r.free(), (2, vec!["translate_z".into(), "spin_z".into()]));

    let Some(MateEntry::Known(Mate::Concentric { a, b, alignment, lock_rotation, .. })) = r.doc.assembly().mates.last()
    else {
        panic!("a concentric mate")
    };
    assert_eq!((*alignment, *lock_rotation), (Some(Alignment::Aligned), None));
    assert_eq!(
        a.support,
        Some(StoredSupport::Axis {
            direction: [0, 0, 1],
            through: [10.0, 5.0, 0.0],
            radius: Some(4.0),
            outward: Some(false)
        })
    );
    assert!(matches!(
        b.support,
        Some(StoredSupport::Axis { direction: [0, 0, 1], radius: Some(4.0), outward: Some(true), .. })
    ));
    let saved = serde_json::to_value(r.doc.assembly()).unwrap();
    let mate = saved["mates"].as_array().unwrap().iter().find(|m| m["kind"] == "concentric").unwrap().clone();
    assert_eq!(mate["a"]["support"]["kind"], json!("axis"));

    let bottoms = (picked(r.a, r.pa.plane([0.0, 0.0, -1.0])), picked(r.b, r.pb.plane([0.0, 0.0, -1.0])));
    r.doc.mate_coincident("F", &bottoms.0, &bottoms.1, Alignment::Aligned, false).unwrap();
    assert_eq!(r.placement(r.b).translation, DVec3::new(10.0, 5.0, 0.0));
    assert_eq!(r.free(), (1, vec!["spin_z".into()]));

    r.doc.save(None).unwrap();
    let mut reopened = r.reopened();
    assert_eq!(reopened.assembly(), r.doc.assembly());
    let status = reopened.solver();
    assert_eq!((status.remaining_dof, status.free.len()), (1, 1), "{status:?}");
    // A rebuild proves the stored axes again and moves nothing.
    let _ = reopened.rebuild();
    reopened.finish_loading();
    assert_eq!(reopened.assembly(), r.doc.assembly());
    assert!(reopened.mate_measurements().iter().all(|(_, m)| m.is_ok()));
}

/// Lock rotation holds the turn about the axis where the solve leaves it, as a quarter-turn
/// matrix, and turning it off frees the spin again.
#[test]
fn lock_rotation_holds_the_spin() {
    let mut r = shaft_rig("lock");
    let (bore, shaft) = (r.bore(), r.shaft());
    r.doc.mate_concentric("C", &bore, &shaft, Alignment::Aligned, true, false).unwrap();
    assert_eq!(r.free(), (1, vec!["translate_z".into()]));
    let Some(MateEntry::Known(Mate::Concentric { lock_rotation, .. })) = r.doc.assembly().mates.last() else {
        panic!("a concentric mate")
    };
    assert_eq!(*lock_rotation, Some(soliddraft_assembly::StoredTurn::Cube([[1, 0, 0], [0, 1, 0], [0, 0, 1]])));
    r.doc.edit_mates(&[("C".into(), MateEdit::LockRotation(false))]).unwrap();
    assert_eq!(r.free().0, 2);
    r.doc.edit_mates(&[("C".into(), MateEdit::LockRotation(true))]).unwrap();
    assert_eq!(r.free().0, 1);
    let reopened = {
        r.doc.save(None).unwrap();
        r.reopened()
    };
    assert_eq!(reopened.assembly(), r.doc.assembly());
}

/// A pin goes into the first tab's hole exactly. The copy turned 60 degrees has an
/// algebraic axis. Asking the pin to occupy both holes is refused without an edit;
/// either hole alone is supported, with a bounded proof for the algebraic copy.
#[test]
fn a_pin_in_a_hole_and_a_sixty_degree_copy() {
    let mut r = rig("pin", |p| new_tabs(p, 60.0), |p| new_shaft(p, 4.0, 12.0));
    let holes = r.pa.faces(SurfaceKind::Cylinder);
    assert_eq!(holes.len(), 2);
    let (first, copy) = (holes[1].clone(), holes[0].clone());
    let pin = r.shaft();
    r.doc.mate_concentric("C1", &picked(r.a, first), &pin, Alignment::Aligned, false, false).unwrap();
    assert_eq!(r.placement(r.b).translation, DVec3::new(20.0, 0.0, 30.0));

    let before = serde_json::to_value(r.doc.assembly()).unwrap();
    let depth = r.doc.undo_depth();
    let copied = picked(r.a, copy);
    let refused = r.doc.mate_concentric("C2", &copied, &pin, Alignment::Aligned, false, false).unwrap_err();
    assert_eq!(refused.code, RefusalCode::Uncertified, "{refused:?}");
    assert!(refused.mates.contains(&"C1".into()) && refused.mates.contains(&"C2".into()));
    assert_eq!(serde_json::to_value(r.doc.assembly()).unwrap(), before);
    assert_eq!(r.doc.undo_depth(), depth);
    r.doc.delete_mate("C1").unwrap();
    r.doc.mate_concentric("C2", &copied, &pin, Alignment::Aligned, false, false).unwrap();
    assert!(r.doc.solver().bounded.iter().any(|p| p.mate == "C2"));

    let mut q = rig("pin-quarter", |p| new_tabs(p, 90.0), |p| new_shaft(p, 4.0, 12.0));
    let holes = q.pa.faces(SurfaceKind::Cylinder);
    // The copy turned a quarter turn sits at (0, 20).
    let copy = holes[0].clone();
    let pin = q.shaft();
    q.doc.mate_concentric("C", &picked(q.a, copy), &pin, Alignment::Aligned, false, false).unwrap();
    close(q.placement(q.b).translation, [0.0, 20.0, 30.0]);
}

/// A shaft laid on the block's top turns flat and sits one radius above it; a Concentric
/// with a plane is refused by name.
#[test]
fn a_shaft_lies_tangent_on_a_plane() {
    let mut r = shaft_rig("tangent");
    let top = picked(r.a, r.pa.plane([0.0, 0.0, 1.0]));
    let shaft = r.shaft();
    r.doc.mate_tangent("T", &top, &shaft, Alignment::Opposed, false).unwrap();
    let p = r.placement(r.b);
    let axis = p.rotation * DVec3::Z;
    assert!(axis.z.abs() < 1e-12, "the axis lies flat: {axis}");
    assert_eq!(p.translation.z, 14.0, "the axis is one radius above the top");
    assert_eq!(r.free().0, 4);

    let refused = r.doc.mate_concentric("C", &top, &shaft, Alignment::Aligned, false, false).unwrap_err();
    assert_eq!((refused.code, refused.mates.clone()), (RefusalCode::UnsupportedReference, vec!["C".to_owned()]));
}

/// A hinge seats the shaft in the bore, flush at the bottom, and leaves the spin. Flipping
/// its planes alone asks the shaft to face both ways, which is refused and named.
#[test]
fn a_hinge_seats_a_shaft_and_leaves_the_spin() {
    let mut r = shaft_rig("hinge");
    let (bore, shaft) = (r.bore(), r.shaft());
    let bottoms = (picked(r.a, r.pa.plane([0.0, 0.0, -1.0])), picked(r.b, r.pb.plane([0.0, 0.0, -1.0])));
    r.doc
        .mate_hinge("H", [&bore, &shaft], [&bottoms.0, &bottoms.1], Alignment::Aligned, Alignment::Aligned, false)
        .unwrap();
    assert_eq!(r.placement(r.b).translation, DVec3::new(10.0, 5.0, 0.0));
    assert_eq!(r.free(), (1, vec!["spin_z".into()]));
    let refused = r.doc.edit_mates(&[("H".into(), MateEdit::PlaneAlignment(Alignment::Opposed))]).unwrap_err();
    assert_eq!((refused.code, refused.mates), (RefusalCode::OverConstrained, vec!["H".to_owned()]));
    // Flipping both turns the shaft over: it hangs from the bottom face instead.
    r.doc
        .edit_mates(&[
            ("H".into(), MateEdit::Alignment(Alignment::Opposed)),
            ("H".into(), MateEdit::PlaneAlignment(Alignment::Opposed)),
        ])
        .unwrap();
    let p = r.placement(r.b);
    close(p.rotation * DVec3::Z, [0.0, 0.0, -1.0]);
    close(p.translation, [10.0, 5.0, 0.0]);
}

/// Vertices and edges: a corner on a corner, an edge in a plane, a corner a distance from
/// a plane; an incompatible distance between two corners is refused by name.
#[test]
fn vertices_and_edges_mate_exactly() {
    let mut r = rig("points", |p| new_block(p, 40.0, 30.0, 10.0), |p| new_block(p, 20.0, 10.0, 6.0));
    let corner_a = picked_vertex(r.a, r.pa.vertex([20.0, 15.0, 10.0]));
    let corner_b = picked_vertex(r.b, r.pb.vertex([-10.0, -5.0, 0.0]));
    r.doc.mate_coincident("P", &corner_a, &corner_b, Alignment::Aligned, false).unwrap();
    assert_eq!(r.placement(r.b).translation, DVec3::new(30.0, 20.0, 10.0));
    assert_eq!(r.free().0, 3, "a point on a point leaves the three turns");
    let Some(MateEntry::Known(Mate::Coincident { a, .. })) = r.doc.assembly().mates.last() else { panic!() };
    assert!(matches!(a.entity, Entity::Vertex(_)));
    assert_eq!(a.support, Some(StoredSupport::Point { at: [20.0, 15.0, 10.0], radius: None, outward: None }));

    // B's bottom edge along X, at y = -5, in A's front face plane y = -15.
    let edge = EndpointSelector { component: r.b, face: FaceSelector::PickedEdge(r.pb.line([0.0, -5.0, 0.0])) };
    let front = picked(r.a, r.pa.plane([0.0, -1.0, 0.0]));
    r.doc.delete_mate("P").unwrap();
    r.doc.mate_coincident("E", &front, &edge, Alignment::Aligned, false).unwrap();
    assert_eq!(r.placement(r.b).translation.y, -10.0);
    r.doc
        .mate_distance("D", &picked(r.a, r.pa.plane([0.0, 0.0, 1.0])), &corner_b, "5 mm", Alignment::Aligned, false)
        .unwrap();
    assert_eq!(r.placement(r.b).translation.z, 15.0);

    let refused = r.doc.mate_distance("Bad", &corner_a, &corner_b, "5 mm", Alignment::Aligned, false).unwrap_err();
    assert_eq!(refused.code, RefusalCode::Uncertified);
    assert!(
        refused.mates.contains(&"Bad".into())
            && refused.mates.contains(&"D".into())
            && refused.mates.contains(&"E".into())
    );

    r.doc.save(None).unwrap();
    let reopened = r.reopened();
    assert_eq!(reopened.assembly(), r.doc.assembly());
}

fn picked_vertex(component: ComponentId, v: VertexRef) -> EndpointSelector {
    EndpointSelector { component, face: FaceSelector::PickedVertex(v) }
}

/// Two axes held parallel or square fix turns only; the shaft keeps its place.
#[test]
fn axes_held_parallel_and_square() {
    let mut r = shaft_rig("axes");
    let (bore, shaft) = (r.bore(), r.shaft());
    r.doc.mate_parallel("P", &bore, &shaft, Alignment::Opposed, false).unwrap();
    let p = r.placement(r.b);
    close(p.rotation * DVec3::Z, [0.0, 0.0, -1.0]);
    assert_eq!(r.free().0, 4);
    r.doc.delete_mate("P").unwrap();
    r.doc.mate_perpendicular("S", &bore, &shaft, false).unwrap();
    let p = r.placement(r.b);
    assert!((p.rotation * DVec3::Z).z.abs() < 1e-12);
    assert_eq!(r.free().0, 5);
}

/// The bore's axis as the Concentric `C` stores it on A's end.
fn stored_bore(doc: &AssemblyDocument) -> [f64; 3] {
    let c = doc.assembly().known_mates().find(|m| m.name() == "C").expect("C");
    let soliddraft_assembly::Mate::Concentric { a, .. } = c else { panic!("a Concentric") };
    match &a.support {
        Some(soliddraft_assembly::StoredSupport::Axis { through, .. }) => *through,
        other => panic!("{other:?}"),
    }
}

/// A's bore moves 5 mm along x in a new revision of A's file. Update proves the new axis
/// to put the shaft on it, and stores that axis too, so the status and a reopened file
/// read the axis where it now is. Undo and Redo carry the stored axis with the revision.
#[test]
fn update_stores_the_axis_it_proved() {
    let mut r = shaft_rig("update-axis");
    let (bore, shaft) = (r.bore(), r.shaft());
    r.doc.mate_concentric("C", &bore, &shaft, Alignment::Aligned, false, false).unwrap();
    assert_eq!(stored_bore(&r.doc), [10.0, 5.0, 0.0]);
    let a_file = r.dir.join("a.dwprt");
    let mut part = Document::open(&a_file, kernel()).unwrap();
    let id = part.features()[2].id;
    let FeatureKind::Sketch(mut s) = part.feature(id).unwrap().kind.clone() else { panic!("the bore's sketch") };
    // The circle and the point its centre is fixed at, 5 mm along x.
    let along = P2::new(5.0, 0.0);
    for e in s.sketch.entities.iter_mut().filter(|e| !e.construction) {
        e.translate(along);
    }
    for c in &mut s.sketch.constraints {
        if let soliddraft_model::sketch::ConstraintKind::Fix { at, .. } = &mut c.kind {
            *at = P2::new(at.u + along.u, at.v + along.v);
        }
    }
    solve(&mut s.sketch, None, true);
    part.execute(Command::EditFeature { id, kind: FeatureKind::Sketch(s) }).unwrap();
    part.save(&a_file).unwrap();

    let report = r.doc.rebuild();
    assert!(report.refusal.is_none(), "{report:?}");
    assert_eq!(stored_bore(&r.doc), [15.0, 5.0, 0.0], "the stored axis is the one Update proved");
    close(r.placement(r.b).translation, [15.0, 5.0, 30.0]);
    r.doc.save(None).unwrap();
    assert_eq!(stored_bore(&r.reopened()), [15.0, 5.0, 0.0]);

    // Undo puts back the axis Update replaced along with the old revision, Redo the new one,
    // and a file saved after Undo stores the old axis.
    assert_eq!(r.doc.undo_label(), Some("Update"));
    assert!(r.doc.undo());
    assert_eq!(stored_bore(&r.doc), [10.0, 5.0, 0.0], "Undo restores the stored axis");
    close(r.placement(r.b).translation, [10.0, 5.0, 30.0]);
    assert!(r.doc.redo());
    assert_eq!(stored_bore(&r.doc), [15.0, 5.0, 0.0], "Redo stores the proved axis again");
    close(r.placement(r.b).translation, [15.0, 5.0, 30.0]);
    assert!(r.doc.undo());
    r.doc.save(None).unwrap();
    assert_eq!(stored_bore(&r.reopened()), [10.0, 5.0, 0.0], "a file saved after Undo");
}
