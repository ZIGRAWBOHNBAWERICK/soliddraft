//! Mates on real parts through the commands the panel and the MCP tools call: what stays
//! free, how alignment and Origin work, what a failed Update leaves behind, and what old
//! and future files keep. Each holds for Coincident mates as for distances. Parallel,
//! Perpendicular, Width, Lock and mates to reference planes follow at the end.
//!
//! Expected positions are hand-worked from the plane equations of a 40 x 30 x D block A
//! and a 20 x 10 x 6 block B, both centred on the origin in plan and rising along +Z.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{DQuat, DVec3};
use serde_json::{Value, json};
use soliddraft_assembly::{
    Alignment, Assembly, AssemblyDocument, AssemblyPackage, ComponentId, EndpointSelector, FaceSelector, FileError,
    GeneratorChoice, KernelSource, Mate, MateEdit, MateEntry, MateId, MateRefusal, OccurrencePath, OpenOptions,
    Placement, RefusalCode, SolverState, Step, WidthInput,
};
use soliddraft_document::{CacheOnlyStore, ConfigurationId, DocRef, identify_file};
use soliddraft_kernel_api::Kernel;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, ProfileSource, SketchFeature,
    SketchPlane,
};

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-mates-{}-{name}", std::process::id()));
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
    let mut doc = Document::new("Block", kernel());
    let mut sketch = Sketch::new();
    sketch.rectangle(P2::new(0.0, 0.0), w, d).unwrap();
    solve(&mut sketch, None, true);
    let plane = SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 };
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

/// Sets a saved block's height, keeping its id and its extrusion.
fn set_height(path: &Path, h: f64) {
    let mut doc = Document::open(path, kernel()).unwrap();
    let id = doc.features()[1].id;
    let FeatureKind::Extrude(mut extrude) = doc.feature(id).unwrap().kind.clone() else { panic!("an extrusion") };
    extrude.end = EndCondition::Blind { depth: h };
    doc.execute(Command::EditFeature { id, kind: FeatureKind::Extrude(extrude) }).unwrap();
    doc.save(path).unwrap();
}

/// Replaces a saved block's extrusion with a new feature making the same box.
fn regenerate(path: &Path) {
    let mut doc = Document::open(path, kernel()).unwrap();
    let old = doc.features()[1].clone();
    doc.execute(Command::DeleteFeature { id: old.id }).unwrap();
    let kind = old.kind.clone();
    doc.execute(Command::AddFeature { kind, name: Some(old.name.clone()), position: None, via_assistant: false })
        .unwrap();
    assert_ne!(doc.features()[1].id, old.id);
    doc.save(path).unwrap();
}

fn reference(from: &Path, to: &Path) -> DocRef {
    let i = identify_file(to).unwrap();
    let mut r = DocRef::new(i.id, i.kind, "", i.revision);
    r.set_path(from, to);
    r
}

fn face(component: ComponentId, generator: GeneratorChoice, normal: Option<[f64; 3]>) -> EndpointSelector {
    let feature = "Boss-Extrude1".into();
    EndpointSelector { component, face: FaceSelector::Generator { feature, generator, normal } }
}

fn side(component: ComponentId, n: [f64; 3]) -> EndpointSelector {
    face(component, GeneratorChoice::Side, Some(n))
}

fn top(component: ComponentId) -> EndpointSelector {
    face(component, GeneratorChoice::EndCap, None)
}

fn bottom(component: ComponentId) -> EndpointSelector {
    face(component, GeneratorChoice::StartCap, None)
}

const X: [f64; 3] = [1.0, 0.0, 0.0];
const Y: [f64; 3] = [0.0, 1.0, 0.0];

struct Rig {
    dir: PathBuf,
    a_file: PathBuf,
    b_file: PathBuf,
    root: PathBuf,
    doc: AssemblyDocument,
    a: ComponentId,
    b: ComponentId,
}

/// Part A of height `depth` and B inserted at [10, 10, 30], both floating, saved.
fn rig(name: &str, depth: f64) -> Rig {
    let dir = folder(name);
    let (a_file, b_file, root) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("rig.dwasm"));
    new_block(&a_file, 40.0, 30.0, depth);
    new_block(&b_file, 20.0, 10.0, 6.0);
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&root)).unwrap();
    let a = doc.insert(reference(&root, &a_file), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference(&root, &b_file), Placement::translation(DVec3::new(10.0, 10.0, 30.0))).unwrap();
    doc.finish_loading();
    doc.save(None).unwrap();
    Rig { dir, a_file, b_file, root, doc, a, b }
}

impl Rig {
    /// The sides along `n` on one plane: a zero distance, or a Coincident.
    fn sides(&mut self, name: &str, n: [f64; 3], alignment: Alignment, coincident: bool) {
        let (a, b) = (side(self.a, n), side(self.b, n));
        let made = match coincident {
            true => self.doc.mate_coincident(name, &a, &b, alignment, false),
            false => self.doc.mate_distance(name, &a, &b, "0 mm", alignment, false),
        };
        made.unwrap();
    }

    fn x(&mut self, alignment: Alignment) {
        self.sides("X", X, alignment, false);
    }

    fn y(&mut self, alignment: Alignment) {
        self.sides("Y", Y, alignment, false);
    }

    /// A's top and B's underside on one plane.
    fn cz(&mut self, alignment: Alignment, strict: bool) -> Result<MateId, MateRefusal> {
        self.doc.mate_coincident("Cz", &top(self.a), &bottom(self.b), alignment, strict)
    }

    /// O, then Cx, Cy and Cz: B's underside on A's top, its sides on A's.
    fn contact(&mut self) {
        self.doc.mate_origin("O", self.a, true, false).unwrap();
        self.sides("Cx", X, Alignment::Aligned, true);
        self.sides("Cy", Y, Alignment::Aligned, true);
        self.cz(Alignment::Opposed, false).unwrap();
    }

    fn z(&mut self, distance: &str, alignment: Alignment) {
        self.doc.mate_distance("Z", &top(self.a), &bottom(self.b), distance, alignment, false).unwrap();
    }

    /// O, X, Y and Z of the main graph, Z at 5 mm.
    fn graph(&mut self) {
        self.graph_with(false);
    }

    /// The main graph, with X and Y Coincident mates when `coincident` says so.
    fn graph_with(&mut self, coincident: bool) {
        self.doc.mate_origin("O", self.a, true, false).unwrap();
        self.sides("X", X, Alignment::Aligned, coincident);
        self.sides("Y", Y, Alignment::Aligned, coincident);
        self.z("5 mm", Alignment::Opposed);
    }

    fn placement(&self, c: ComponentId) -> Placement {
        self.doc.assembly().component(c).unwrap().placement
    }

    fn row(&self, c: ComponentId) -> soliddraft_assembly::OccurrenceIndex {
        self.doc.table().find(&OccurrencePath::root().child(Step::component(c))).unwrap()
    }

    /// The whole accepted state a refusal must leave alone.
    fn state(&self) -> Value {
        let table = self.doc.table();
        let rows: Vec<_> = (0..table.len() as u32)
            .map(soliddraft_assembly::OccurrenceIndex)
            .map(|r| json!([table.path(r), table.local(r), table.world(r)]))
            .collect();
        let keys: Vec<_> = self.doc.store().iter().map(|(_, d)| json!([d.key.document, d.key.revision])).collect();
        json!({
            "truth": self.doc.assembly(), "revision": self.doc.revision(), "dirty": self.doc.is_dirty(),
            "undo": [self.doc.undo_label(), self.doc.undo_depth()], "redo": [self.doc.redo_label(), self.doc.redo_depth()],
            "rows": rows, "definitions": keys,
        })
    }

    /// The top of B as the occurrence table shows it.
    fn b_top(&self) -> f64 {
        self.doc.table().bounds(self.row(self.b)).max.z
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn close(a: DVec3, b: [f64; 3]) {
    assert!(a.distance(DVec3::from_array(b)) <= 1e-9, "{a} against {b:?}");
}

/// Rotations match by what they do to the basis, allowing q and -q.
fn turned(q: DQuat, x: [f64; 3], y: [f64; 3], z: [f64; 3]) {
    for (axis, wanted) in [(DVec3::X, x), (DVec3::Y, y), (DVec3::Z, z)] {
        assert!((q * axis).distance(DVec3::from_array(wanted)) <= 1e-12, "{q} on {axis}");
    }
}

/// S1: refusal against partial freedom, and no hidden fixing of A.
#[test]
fn partial_freedom_and_strict_failure_are_distinct() {
    let mut r = rig("s1", 10.0);
    r.doc.mate_origin("O", r.a, true, false).unwrap();
    let before = r.state();
    // A held redo survives a refusal too.
    r.doc.mate_origin("Spare", r.b, false, false).unwrap();
    r.doc.undo();
    let held = r.state();
    let a_bytes = std::fs::read(&r.a_file).unwrap();
    let b_bytes = std::fs::read(&r.b_file).unwrap();

    let refused = r.doc.mate_distance("Z", &top(r.a), &bottom(r.b), "5 mm", Alignment::Opposed, true).unwrap_err();
    assert_eq!(refused.code, RefusalCode::UnderConstrained);
    assert_eq!(refused.remaining_dof, Some(3));
    let motions: Vec<_> = refused.free.iter().map(|f| (f.component, f.motion.as_str())).collect();
    assert_eq!(motions, [(r.b, "translate_x"), (r.b, "translate_y"), (r.b, "spin_z")]);
    assert_eq!(r.state(), held, "a refused proposal allocates and records nothing, and keeps redo");
    assert_eq!(std::fs::read(&r.a_file).unwrap(), a_bytes);
    assert_eq!(std::fs::read(&r.b_file).unwrap(), b_bytes);
    assert_eq!(before["truth"], held["truth"]);

    // Ordinary insertion of the same partial mate commits, and keeps authored coordinates.
    r.z("5 mm", Alignment::Opposed);
    let status = r.doc.solver();
    assert_eq!((status.state, status.remaining_dof), (SolverState::Underconstrained, 3));
    assert_eq!(status.free.len(), 3);
    assert_eq!(r.placement(r.b).translation, DVec3::new(10.0, 10.0, 15.0));

    // Without O the six remaining motions are the whole group's, and A is not fixed.
    let mut r = rig("s1-float", 10.0);
    r.x(Alignment::Aligned);
    r.y(Alignment::Aligned);
    r.z("5 mm", Alignment::Opposed);
    assert_eq!(r.doc.solver().remaining_dof, 6);
    assert!(r.doc.solver().free.is_empty(), "no one component moves alone: {:?}", r.doc.solver());
    assert_eq!(r.doc.solver().state, SolverState::Underconstrained);
    let refused = r.doc.mate_distance("Again", &top(r.a), &bottom(r.b), "5 mm", Alignment::Opposed, true).unwrap_err();
    assert_eq!((refused.code, refused.remaining_dof), (RefusalCode::UnderConstrained, Some(6)));

    // A strict Coincident leaves what a strict distance leaves, and commits nothing.
    let mut r = rig("s1-coincident", 10.0);
    r.doc.mate_origin("O", r.a, true, false).unwrap();
    let (before, a_bytes) = (r.state(), std::fs::read(&r.a_file).unwrap());
    let refused = r.cz(Alignment::Opposed, true).unwrap_err();
    assert_eq!((refused.code, refused.remaining_dof), (RefusalCode::UnderConstrained, Some(3)));
    let motions: Vec<_> = refused.free.iter().map(|f| (f.component, f.motion.as_str())).collect();
    assert_eq!(motions, [(r.b, "translate_x"), (r.b, "translate_y"), (r.b, "spin_z")]);
    assert_eq!(r.state(), before);
    assert_eq!(std::fs::read(&r.a_file).unwrap(), a_bytes);

    // Three Coincidents and no O hold nothing to the assembly: six motions stay and A is
    // not fixed.
    let mut r = rig("s1-coincident-float", 10.0);
    r.sides("Cx", X, Alignment::Aligned, true);
    r.sides("Cy", Y, Alignment::Aligned, true);
    let before = r.state();
    let refused = r.cz(Alignment::Opposed, true).unwrap_err();
    assert_eq!((refused.code, refused.remaining_dof), (RefusalCode::UnderConstrained, Some(6)));
    assert_eq!(r.state(), before);
    r.cz(Alignment::Opposed, false).unwrap();
    let status = r.doc.solver();
    assert_eq!((status.state, status.remaining_dof), (SolverState::Underconstrained, 6));
    assert!(status.free.is_empty(), "no one component moves alone: {status:?}");
    assert!(r.placement(r.a).is_identity());
    close(r.placement(r.b).translation, [10.0, 10.0, 10.0]);

    // An Origin on A's position only leaves A's three turns.
    let mut r = rig("s1-coincident-position", 10.0);
    r.doc.mate_origin("O", r.a, false, false).unwrap();
    r.sides("Cx", X, Alignment::Aligned, true);
    r.sides("Cy", Y, Alignment::Aligned, true);
    r.cz(Alignment::Opposed, false).unwrap();
    assert_eq!(r.doc.solver().remaining_dof, 3);
}

/// S2: alignment is a vector relation, and every family goes through one solver.
#[test]
fn alignment_and_origin_use_the_same_solver() {
    let mut r = rig("s2", 14.0);
    r.graph();
    let before = r.state();
    let refused = r.doc.edit_mates(&[("Z".into(), MateEdit::Alignment(Alignment::Aligned))]).unwrap_err();
    assert_eq!(refused.code, RefusalCode::OverConstrained, "{refused}");
    assert_eq!(r.state(), before);

    // Y and Z flip together, as one candidate and one undo step.
    r.doc.edit_mates(&[("Z".into(), MateEdit::Distance("12 mm".into()))]).unwrap();
    let depth = r.doc.undo_depth();
    r.doc
        .edit_mates(&[
            ("Y".into(), MateEdit::Alignment(Alignment::Opposed)),
            ("Z".into(), MateEdit::Alignment(Alignment::Aligned)),
        ])
        .unwrap();
    assert_eq!(r.doc.undo_depth(), depth + 1);
    let p = r.placement(r.b);
    assert_eq!(p.rotation.to_array(), [1.0, 0.0, 0.0, 0.0]);
    turned(p.rotation, [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]);
    close(p.translation, [10.0, 20.0, 26.0]);
    let bounds = r.doc.table().bounds(r.row(r.b));
    close(bounds.min, [0.0, 15.0, 20.0]);
    close(bounds.max, [20.0, 25.0, 26.0]);
    let measured = r.doc.mate_measurements();
    let z = measured.last().unwrap().1.as_ref().unwrap().as_ref().expect("a distance measures");
    assert_eq!((z.value, z.rational.as_str()), (12.0, "12"));

    r.doc.save(None).unwrap();
    let mut reopened = AssemblyDocument::open(&r.root, options()).unwrap();
    reopened.finish_loading();
    reopened.edit_mates(&[("Z".into(), MateEdit::Distance("8 mm".into()))]).unwrap();
    let c = reopened.assembly().component(r.b).unwrap();
    close(c.placement.translation, [10.0, 20.0, 22.0]);
    let row = reopened.table().find(&OccurrencePath::root().child(Step::component(r.b))).unwrap();
    close(reopened.table().bounds(row).min, [0.0, 15.0, 16.0]);
    close(reopened.table().bounds(row).max, [20.0, 25.0, 22.0]);

    // The same flips on Coincident mates: B turns over and stays on A's planes.
    let mut r = rig("s2-coincident", 14.0);
    r.contact();
    close(r.placement(r.b).translation, [10.0, 10.0, 14.0]);
    let zero = |r: &mut Rig| {
        let measured = r.doc.mate_measurements();
        assert_eq!(measured.len(), 3, "each Coincident is measured");
        for (_, m) in measured {
            let m = m.unwrap().expect("a Coincident measures");
            assert_eq!((m.value, m.rational.as_str()), (0.0, "0"));
        }
    };
    zero(&mut r);
    let before = r.state();
    let refused = r.doc.edit_mates(&[("Cz".into(), MateEdit::Alignment(Alignment::Aligned))]).unwrap_err();
    assert_eq!(refused.code, RefusalCode::OverConstrained, "{refused}");
    assert_eq!(r.state(), before);
    let refused = r.doc.edit_mates(&[("Cz".into(), MateEdit::Distance("1 mm".into()))]).unwrap_err();
    assert_eq!(refused.code, RefusalCode::Invalid, "a Coincident has no distance: {refused}");
    assert_eq!(r.state(), before);

    let depth = r.doc.undo_depth();
    r.doc
        .edit_mates(&[
            ("Cy".into(), MateEdit::Alignment(Alignment::Opposed)),
            ("Cz".into(), MateEdit::Alignment(Alignment::Aligned)),
        ])
        .unwrap();
    assert_eq!(r.doc.undo_depth(), depth + 1);
    let flipped = |r: &Rig| {
        let p = r.placement(r.b);
        turned(p.rotation, [1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]);
        close(p.translation, [10.0, 20.0, 14.0]);
        let bounds = r.doc.table().bounds(r.row(r.b));
        close(bounds.min, [0.0, 15.0, 8.0]);
        close(bounds.max, [20.0, 25.0, 14.0]);
    };
    flipped(&r);
    assert_eq!(r.doc.solver().remaining_dof, 0);
    zero(&mut r);
    let mates = r.doc.assembly().mates.clone();
    let kinds: Vec<_> = r
        .doc
        .assembly()
        .known_mates()
        .filter_map(|m| match m {
            Mate::Coincident { id, name, alignment, .. } => Some((*id, name.clone(), *alignment)),
            _ => None,
        })
        .collect();
    let ids: Vec<_> = before["truth"]["mates"].as_array().unwrap().iter().map(|m| m["id"].as_u64().unwrap()).collect();
    assert_eq!(kinds.iter().map(|k| u64::from(k.0.0)).collect::<Vec<_>>(), ids[1..]);
    assert_eq!(
        kinds.iter().map(|k| (k.1.as_str(), k.2)).collect::<Vec<_>>(),
        [("Cx", Alignment::Aligned), ("Cy", Alignment::Opposed), ("Cz", Alignment::Aligned)]
    );
    assert!(r.doc.undo());
    assert_eq!(r.state()["truth"], before["truth"]);
    close(r.placement(r.b).translation, [10.0, 10.0, 14.0]);
    assert!(r.doc.redo());
    flipped(&r);
    assert_eq!(r.doc.assembly().mates, mates);
    let parts = (std::fs::read(&r.a_file).unwrap(), std::fs::read(&r.b_file).unwrap());
    r.doc.save(None).unwrap();
    let saved: Value =
        serde_json::from_slice(&AssemblyPackage::open(&r.root).unwrap().assembly().truth_entries().json).unwrap();
    for m in &saved["mates"].as_array().unwrap()[1..] {
        let mut keys: Vec<_> = m.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["a", "alignment", "b", "id", "kind", "name"], "a Coincident saves no distance: {m}");
        assert_eq!(m["kind"], "coincident");
    }
    let mut reopened = AssemblyDocument::open(&r.root, options()).unwrap();
    reopened.finish_loading();
    assert_eq!(reopened.assembly().mates, mates, "the same ids, kinds and alignments");
    assert_eq!(reopened.assembly(), r.doc.assembly());
    assert_eq!(reopened.solver().remaining_dof, 0);
    assert_eq!(
        (std::fs::read(&r.a_file).unwrap(), std::fs::read(&r.b_file).unwrap()),
        parts,
        "parts are never written"
    );
}

/// S3: an Origin mate anchors what it says it anchors.
#[test]
fn origin_axis_alignment_is_authored() {
    let mut r = rig("s3", 14.0);
    r.doc.mate_origin("O", r.a, false, false).unwrap();
    r.x(Alignment::Aligned);
    r.y(Alignment::Aligned);
    r.z("12 mm", Alignment::Opposed);
    assert_eq!(r.doc.solver().remaining_dof, 3, "A's origin is anchored and its turn is not");
    let before = r.state();
    let refused = r.doc.mate_origin("Strict", r.a, false, true).unwrap_err();
    assert_eq!((refused.code, refused.remaining_dof), (RefusalCode::UnderConstrained, Some(3)));
    assert_eq!(r.state(), before);

    let depth = r.doc.undo_depth();
    r.doc.edit_mates(&[("O".into(), MateEdit::AlignAxes(true))]).unwrap();
    assert_eq!(r.doc.undo_depth(), depth + 1);
    assert_eq!(r.doc.solver().remaining_dof, 0);
    assert!(r.placement(r.a).is_identity());
    close(r.placement(r.b).translation, [10.0, 10.0, 26.0]);
    assert!(r.doc.undo());
    assert_eq!(r.doc.solver().remaining_dof, 3);
    assert!(r.doc.redo());
    assert_eq!(r.doc.solver().remaining_dof, 0);
    r.doc.save(None).unwrap();
    let mut reopened = AssemblyDocument::open(&r.root, options()).unwrap();
    reopened.finish_loading();
    assert_eq!(reopened.solver().remaining_dof, 0);
    assert_eq!(reopened.assembly(), r.doc.assembly());

    // A first component at zero with no Origin mate floats.
    let mut r = rig("s3-float", 14.0);
    assert_eq!(r.doc.solver().remaining_dof, 12);
    assert_eq!(r.doc.solver().state, SolverState::Underconstrained);
}

/// S4: a failed Update rolls back every candidate resource, whichever step failed, with
/// X and Y as zero distances or as Coincidents.
#[test]
fn failed_update_rolls_back_every_candidate_resource() {
    for coincident in [false, true] {
        let mut r = rig(&format!("s4-{coincident}"), 14.0);
        r.graph_with(coincident);
        r.doc.set_value("gap", "12 mm").unwrap();
        r.doc.drive_mate("Z", "gap", false).unwrap();
        close(r.placement(r.b).translation, [10.0, 10.0, 26.0]);
        r.doc.save(None).unwrap();
        let saved_a = std::fs::read(&r.a_file).unwrap();
        let old = Document::open(&r.a_file, kernel()).unwrap().features()[1].id;

        // B has a valid new depth; A's new revision has lost the faces the mates name.
        set_height(&r.b_file, 8.0);
        regenerate(&r.a_file);
        let before = r.state();
        let top_before = r.b_top();
        let report = r.doc.rebuild();
        let refusal = report.mate_refusal.clone().unwrap();
        assert_eq!(refusal.code, RefusalCode::DanglingReference);
        assert_eq!(refusal.mates, ["X", "Y", "Z"]);
        assert!(refusal.message.contains("a.dwprt"), "{refusal}");
        assert!(refusal.message.contains(&format!("of feature {old}")), "{refusal}");
        assert!(report.refusal.as_ref().unwrap().contains("dangling_reference"));
        r.doc.finish_loading();
        assert_eq!(r.state(), before);
        assert_eq!(r.b_top(), top_before, "B's six-mm display stays");

        // Restoring the file repairs it, and the good B comes in: 26 + 8.
        std::fs::write(&r.a_file, &saved_a).unwrap();
        let report = r.doc.rebuild();
        assert!(report.refusal.is_none(), "{report:?}");
        r.doc.finish_loading();
        close(r.placement(r.b).translation, [10.0, 10.0, 26.0]);
        assert!((r.b_top() - 34.0).abs() <= 1e-9, "{}", r.b_top());
        assert_eq!(r.doc.undo_label(), Some("Update"));
        let kinds = r.doc.assembly().known_mates().filter(|m| matches!(m, Mate::Coincident { .. })).count();
        assert_eq!(kinds, if coincident { 2 } else { 0 });
    }

    // A refusal from the solve itself, after the candidate has loaded: B is anchored too,
    // so A's new height leaves the distance mate nothing to move.
    let mut r = rig("s4-solve", 10.0);
    r.doc.mate_origin("OA", r.a, true, false).unwrap();
    r.doc.mate_origin("OB", r.b, true, false).unwrap();
    r.doc.mate_distance("Z", &top(r.a), &bottom(r.b), "-10 mm", Alignment::Opposed, false).unwrap();
    r.doc.finish_loading();
    r.doc.save(None).unwrap();
    set_height(&r.a_file, 14.0);
    let before = r.state();
    let report = r.doc.rebuild();
    assert_eq!(report.mate_refusal.unwrap().code, RefusalCode::OverConstrained);
    r.doc.finish_loading();
    assert_eq!(r.state(), before);
}

/// P1: an old file, and a mate from a later version.
#[test]
fn old_and_future_assembly_truth_survives() {
    // The file V4's qualified scenario wrote, as hosted CI uploaded it. V5 did not build it.
    let dir = folder("p1");
    let root = dir.join("rig.dwasm");
    std::fs::write(&root, include_bytes!("fixtures/v4-unmated/rig.dwasm")).unwrap();
    std::fs::write(dir.join("a.dwprt"), include_bytes!("fixtures/v4-unmated/a.dwprt")).unwrap();
    let written = AssemblyPackage::open(&root).unwrap();
    let (id, revision) = (written.id(), written.manifest().semantic_hash.clone());
    let geometry = written.manifest().geometry_hash.clone().unwrap();
    assert_eq!(id.to_string(), "0f4f3914-b756-4be9-b74d-e4d703894e84");
    assert_eq!(revision.to_string(), "blake3:eadfa6d96ef1d2ff8868d89f54e001bd72daa2c64940f2a5334b9a3c15c1755d");
    assert_eq!(geometry.to_string(), "blake3:3cdcde0e867af058eb07e3e0c53250413d528785a91947655681a5231fe1f103");
    assert_eq!(written.assembly().semantic_hash(), revision, "the hash V4 wrote is the hash V5 computes");
    assert_eq!(written.assembly().geometry_hash(), geometry);
    let truth = written.assembly().truth_entries().json.to_vec();
    let text = String::from_utf8(truth.clone()).unwrap();
    assert!(!text.contains("mates") && !text.contains("values"), "an empty design writes nothing: {text}");

    let mut doc = AssemblyDocument::open(&root, options()).unwrap();
    doc.finish_loading();
    assert_ne!(doc.solver().state, SolverState::Error);
    let floating = doc.assembly().components[0].clone();
    assert!(floating.placement.is_identity());
    assert_eq!(doc.assembly(), &**written.assembly(), "opening solves nothing and moves nothing");
    assert!(!doc.is_dirty());
    doc.save(None).unwrap();
    let saved = AssemblyPackage::open(&root).unwrap();
    assert_eq!((saved.id(), &saved.manifest().semantic_hash), (id, &revision));
    assert_eq!(saved.manifest().geometry_hash.as_ref(), Some(&geometry));
    assert_eq!(saved.assembly().truth_entries().json.to_vec(), truth);
    assert_eq!(saved.assembly().components[0], floating);
    let _ = std::fs::remove_dir_all(&dir);

    // A mate of a later version round-trips untouched, and nothing solves past it.
    let mut v: Value = serde_json::from_str(&text).unwrap();
    let future = json!({ "kind": "gear", "id": 40, "ratio": 2.5, "future": { "nested": [1, 2] } });
    v["mates"] = json!([future]);
    let kept = Assembly::from_truth(v.to_string().as_bytes(), None).unwrap();
    assert_eq!(serde_json::to_value(&kept).unwrap()["mates"][0], future);
    let dir = folder("p1-future");
    let root = dir.join("future.dwasm");
    kept.save(&root, id, None, &[]).unwrap();
    let mut doc = AssemblyDocument::open(&root, options()).unwrap();
    doc.finish_loading();
    let status = doc.solver();
    assert_eq!(status.state, SolverState::Error);
    assert!(status.diagnostics[0].contains("unknown_mate"), "{status:?}");
    let refused = doc.set_value("gap", "5 mm").unwrap_err();
    assert_eq!(refused.code, RefusalCode::UnknownMate);
    assert_eq!(doc.assembly(), &kept);
    assert!(!doc.is_dirty(), "a proof or a status never changes truth");
    let _ = std::fs::remove_dir_all(dir);

    // A known mate that names what is not there is refused, not dropped.
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["mates"] = json!([{ "kind": "origin", "id": 9, "name": "O", "component": 77, "align_axes": true }]);
    v["next_id"] = json!(10);
    assert!(matches!(Assembly::from_truth(v.to_string().as_bytes(), None), Err(FileError::Invalid(_))));
}

/// A configuration a file carries on a mate's endpoint or source is kept and refused, never
/// solved on the default. A save and a load bring it back, as the reserved field always did.
#[test]
fn a_configuration_is_refused_not_ignored() {
    fn nondefault() -> Option<ConfigurationId> {
        Some(ConfigurationId("nondefault".into()))
    }
    type Change = fn(&mut Assembly);
    // Each change, and whether X and Y are Coincidents.
    let cases: [(&str, Change, bool); 4] = [
        (
            "endpoint",
            |truth| {
                for entry in &mut truth.mates {
                    if let MateEntry::Known(Mate::Distance { a, name, .. }) = entry
                        && name == "Z"
                    {
                        a.configuration = nondefault();
                    }
                }
            },
            false,
        ),
        ("source", |truth| truth.sources[0].document.configuration = nondefault(), false),
        (
            "coincident-endpoint",
            |truth| {
                for entry in &mut truth.mates {
                    if let MateEntry::Known(Mate::Coincident { b, name, .. }) = entry
                        && name == "X"
                    {
                        b.configuration = nondefault();
                    }
                }
            },
            true,
        ),
        ("coincident-source", |truth| truth.sources[1].document.configuration = nondefault(), true),
    ];
    let reload = |r: &mut Rig, change: Change| {
        r.doc.save(None).unwrap();
        let package = AssemblyPackage::open(&r.root).unwrap();
        let mut truth = (**package.assembly()).clone();
        change(&mut truth);
        truth.save(&r.root, package.id(), None, &[]).unwrap();
        r.doc = AssemblyDocument::open(&r.root, options()).unwrap();
        r.doc.finish_loading();
        assert_eq!(r.doc.assembly(), &truth, "a save and a load keep it");
    };
    let code = RefusalCode::UnsupportedConfiguration;
    for (name, change, coincident) in cases {
        let mut r = rig(&format!("configuration-{name}"), 10.0);
        r.graph_with(coincident);
        reload(&mut r, change);
        let before = r.state();
        let refused = r.doc.set_value("gap", "5 mm").unwrap_err();
        assert_eq!((refused.code, refused.mates.is_empty()), (code, false), "{name}: {refused:?}");
        assert!(refused.message.contains("nondefault"), "{refused:?}");
        let edit = [("Z".to_owned(), MateEdit::Distance("9 mm".into()))];
        assert_eq!(r.doc.edit_mates(&edit).unwrap_err().code, code, "{name}");
        assert_eq!(r.doc.solver().state, SolverState::Error);
        assert!(r.doc.solver().diagnostics[0].contains("unsupported_configuration"));
        let measured = r.doc.mate_measurements();
        assert!(measured.iter().any(|(_, m)| m.as_ref().is_err_and(|e| e.code == code)), "{name}: {measured:?}");
        set_height(&r.a_file, 14.0);
        assert_eq!(r.doc.rebuild().mate_refusal.unwrap().code, code, "{name}: Update refuses as a whole");
        assert_eq!(r.state(), before, "{name}: a refusal changes nothing");
    }

    // An Origin mate reads no face, and its component's source is refused all the same.
    let mut r = rig("configuration-origin", 10.0);
    r.doc.mate_origin("O", r.a, true, false).unwrap();
    reload(&mut r, cases[1].1);
    assert_eq!(r.doc.set_value("gap", "5 mm").unwrap_err().code, code);

    // Nothing to read a new face from, and no change to find one.
    let mut r = rig("configuration-new", 10.0);
    reload(&mut r, cases[1].1);
    let before = r.state();
    assert_eq!(r.doc.measure_planes(&top(r.a), &bottom(r.b)).unwrap_err().code, code);
    let refused = r.doc.mate_distance("Z", &top(r.a), &bottom(r.b), "5 mm", Alignment::Opposed, false);
    assert_eq!(refused.unwrap_err().code, code);
    assert_eq!(r.state(), before);

    // Outside mates the field stays reserved: an unmated source saves, loads and edits.
    assert_ne!(r.doc.solver().state, SolverState::Error);
    r.doc.set_value("gap", "5 mm").unwrap();
    r.doc.save(None).unwrap();
    let saved = AssemblyPackage::open(&r.root).unwrap();
    assert_eq!(saved.assembly().sources[0].document.configuration, nondefault());
}

/// A proof names the accepted revision, and a part that moved on leaves it retained.
#[test]
fn a_proof_stays_with_its_revision() {
    let mut r = rig("retained", 10.0);
    r.graph();
    let before = r.doc.mate_measurements();
    let pinned = before.last().unwrap().1.as_ref().unwrap().clone().expect("a distance measures");
    assert_eq!((pinned.value, pinned.rational.as_str()), (5.0, "5"));
    assert_eq!(pinned.certainty, soliddraft_kernel_api::Certainty::Exact);
    assert!(!pinned.a.retained);
    set_height(&r.a_file, 14.0);
    r.doc.refresh_references(Vec::new());
    r.doc.finish_loading();
    let later = r.doc.mate_measurements();
    let held = later.last().unwrap().1.as_ref().unwrap().as_ref().expect("a distance measures");
    assert!(held.a.retained && !held.b.retained, "{held:?}");
    assert_eq!((held.value, &held.a.revision), (5.0, &pinned.a.revision));
}

/// A mate that moves a pattern's seed takes the members with it, and undo brings them back.
#[test]
fn a_mated_seed_takes_its_pattern_along() {
    use soliddraft_assembly::PatternRule;
    let mut r = rig("pattern", 10.0);
    let rule = PatternRule::Linear { direction: [0.0, 1.0, 0.0], spacing: 50.0, count: 2, second: None };
    let pattern = r.doc.add_pattern(vec![r.b], rule).unwrap();
    r.doc.finish_loading();
    let member = OccurrencePath::root().child(Step::member(r.b, pattern, 1));
    r.graph();
    r.doc.finish_loading();
    let row = r.doc.table().find(&member).unwrap();
    close(r.doc.table().world(row).translation, [10.0, 60.0, 15.0]);
    while r.doc.undo_label() != Some("Pattern") {
        r.doc.undo();
    }
    r.doc.finish_loading();
    let row = r.doc.table().find(&member).unwrap();
    close(r.doc.table().world(row).translation, [10.0, 60.0, 30.0]);
}

/// A move proposes a pose; a mated component keeps only what its mates leave free.
#[test]
fn moves_and_deletes_respect_mates() {
    let mut r = rig("moves", 10.0);
    r.doc.mate_origin("O", r.a, true, false).unwrap();
    r.z("5 mm", Alignment::Opposed);
    let path = OccurrencePath::root().child(Step::component(r.b));
    r.doc.move_to(&[(path.clone(), Placement::translation(DVec3::new(1.0, 2.0, 99.0)))]).unwrap();
    close(r.placement(r.b).translation, [1.0, 2.0, 15.0]);
    assert_eq!(r.doc.undo_label(), Some("Move"));

    r.x(Alignment::Aligned);
    r.y(Alignment::Aligned);
    let before = r.state();
    assert!(r.doc.move_to(&[(path, Placement::translation(DVec3::new(1.0, 2.0, 99.0)))]).is_err());
    assert_eq!(r.state(), before);

    let refused = r.doc.delete(&[r.b]).unwrap_err();
    assert!(refused.to_string().contains("mates first"), "{refused}");
    assert_eq!(r.state(), before);
    r.doc.delete_mate("Z").unwrap();
    assert_eq!(r.doc.solver().remaining_dof, 1);
    close(r.placement(r.b).translation, [10.0, 10.0, 15.0]);
}

/// A reference plane feature on a saved part, on `datum` moved by `offset`. Returns its name.
fn add_plane(path: &Path, datum: DatumPlane, offset: f64) -> String {
    let mut doc = Document::open(path, kernel()).unwrap();
    let kind = FeatureKind::ReferencePlane(SketchPlane::Datum { datum, offset });
    doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false }).unwrap();
    let name = doc.features().last().unwrap().name.clone();
    doc.save(path).unwrap();
    name
}

fn plane(component: ComponentId, name: &str) -> EndpointSelector {
    EndpointSelector { component, face: FaceSelector::Plane(name.into()) }
}

fn quarter_z() -> DQuat {
    DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2)
}

/// Saves the assembly, opens it again, and checks the truth came back unchanged.
fn reopen(r: &mut Rig) {
    r.doc.save(None).unwrap();
    let truth = r.doc.assembly().clone();
    r.doc = AssemblyDocument::open(&r.root, options()).unwrap();
    r.doc.finish_loading();
    assert_eq!(r.doc.assembly(), &truth, "a save and a load keep every mate");
}

/// Parallel fixes two turns and no translation; Perpendicular fixes one turn and takes the
/// nearest quarter turn when the authored pose does not hold it; together against one
/// another they are named.
#[test]
fn parallel_and_perpendicular_hold_turns_only() {
    let mut r = rig("parallel", 10.0);
    r.doc.mate_origin("O", r.a, true, false).unwrap();
    r.doc.mate_parallel("P", &top(r.a), &bottom(r.b), Alignment::Opposed, false).unwrap();
    assert!(r.placement(r.b) == Placement::translation(DVec3::new(10.0, 10.0, 30.0)), "it held already");
    let status = r.doc.solver();
    assert_eq!(status.remaining_dof, 4);
    let motions: Vec<_> = status.free.iter().map(|f| f.motion.as_str()).collect();
    assert_eq!(motions, ["translate_x", "translate_y", "translate_z", "spin_z"]);
    let measured = r.doc.mate_measurements();
    let gap = measured[0].1.as_ref().unwrap().as_ref().expect("a Parallel measures its gap");
    assert_eq!(gap.rational, "20");
    let refused = r.doc.edit_mates(&[("P".into(), MateEdit::Distance("1 mm".into()))]).unwrap_err();
    assert_eq!(refused.code, RefusalCode::Invalid, "{refused}");
    reopen(&mut r);

    // A strict Parallel says what it leaves: with the sides too, only translations.
    let refused = r.doc.mate_parallel("Q", &side(r.a, X), &side(r.b, X), Alignment::Aligned, true).unwrap_err();
    assert_eq!((refused.code, refused.remaining_dof), (RefusalCode::UnderConstrained, Some(3)));

    // B's top at right angles to A's: a quarter turn.
    let mut r = rig("perpendicular", 10.0);
    r.doc.mate_origin("O", r.a, true, false).unwrap();
    r.doc.mate_perpendicular("R", &top(r.a), &top(r.b), false).unwrap();
    let turned = r.placement(r.b).rotation;
    assert!((turned * DVec3::Z).dot(DVec3::Z).abs() < 1e-12, "{turned}");
    assert!((turned.angle_between(DQuat::IDENTITY) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    assert_eq!(r.doc.solver().remaining_dof, 5);
    let before = r.state();
    let refused = r.doc.mate_parallel("P", &top(r.a), &top(r.b), Alignment::Aligned, false).unwrap_err();
    assert_eq!(refused.code, RefusalCode::OverConstrained);
    assert_eq!(refused.mates, ["P", "R"], "over-constraint names both mates");
    assert_eq!(r.state(), before);
    let refused = r.doc.edit_mates(&[("R".into(), MateEdit::Alignment(Alignment::Aligned))]).unwrap_err();
    assert_eq!(refused.code, RefusalCode::Invalid, "a Perpendicular has no alignment");
    reopen(&mut r);
    assert_eq!(r.doc.solver().remaining_dof, 5);
}

/// A Lock keeps B where it is relative to A, fixes all six motions between them, survives
/// a save, and refuses a relative turn off the quarter turns instead of rounding it.
#[test]
fn a_lock_keeps_the_relative_placement() {
    let mut r = rig("lock", 10.0);
    r.doc.mate_origin("O", r.a, true, false).unwrap();
    r.doc.mate_lock("L", r.a, r.b, false).unwrap();
    assert_eq!(r.placement(r.b), Placement::translation(DVec3::new(10.0, 10.0, 30.0)));
    assert_eq!(r.doc.solver().state, SolverState::Solved);
    let Some(Mate::Lock { relative, .. }) = r.doc.assembly().known_mates().last().cloned() else { panic!("a lock") };
    assert_eq!(relative, Placement::translation(DVec3::new(10.0, 10.0, 30.0)));
    reopen(&mut r);
    assert_eq!(r.doc.solver().state, SolverState::Solved);

    // A distance that disagrees with the lock is refused, naming both.
    let before = r.state();
    let refused = r.doc.mate_distance("Z", &top(r.a), &bottom(r.b), "5 mm", Alignment::Opposed, false).unwrap_err();
    assert_eq!(refused.code, RefusalCode::OverConstrained);
    assert_eq!(refused.mates, ["L", "Z"]);
    assert_eq!(r.state(), before);

    // A quarter turn between them is exact, and the lock carries B when A turns.
    let mut r = rig("lock-quarter", 10.0);
    let path = OccurrencePath::root().child(Step::component(r.b));
    r.doc.move_to(&[(path, Placement::new(quarter_z(), DVec3::new(10.0, 10.0, 30.0)))]).unwrap();
    r.doc.mate_lock("L", r.a, r.b, false).unwrap();
    r.doc.mate_origin("O", r.a, true, false).unwrap();
    assert_eq!(r.doc.solver().state, SolverState::Solved);
    turned(r.placement(r.b).rotation, [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);

    // Thirty degrees between them is refused, and nothing changes.
    let mut r = rig("lock-oblique", 10.0);
    let path = OccurrencePath::root().child(Step::component(r.b));
    let oblique = Placement::new(DQuat::from_rotation_z(30f64.to_radians()), DVec3::new(10.0, 10.0, 30.0));
    r.doc.move_to(&[(path, oblique)]).unwrap();
    let before = r.state();
    let refused = r.doc.mate_lock("L", r.a, r.b, false).unwrap_err();
    assert_eq!(refused.code, RefusalCode::UnsupportedPlacement, "{refused}");
    assert_eq!(refused.mates, ["L"]);
    assert!(refused.message.contains("quarter-turn"), "{refused}");
    assert_eq!(r.state(), before, "a refused lock changes nothing");

    // The same thirty degrees written into a file is refused when it solves.
    let mut r = rig("lock-file", 10.0);
    r.doc.mate_lock("L", r.a, r.b, false).unwrap();
    r.doc.save(None).unwrap();
    let package = AssemblyPackage::open(&r.root).unwrap();
    let mut truth = (**package.assembly()).clone();
    for entry in &mut truth.mates {
        if let MateEntry::Known(Mate::Lock { relative, .. }) = entry {
            relative.rotation = DQuat::from_rotation_z(30f64.to_radians());
        }
    }
    truth.save(&r.root, package.id(), None, &[]).unwrap();
    r.doc = AssemblyDocument::open(&r.root, options()).unwrap();
    r.doc.finish_loading();
    let refused = r.doc.set_value("gap", "5 mm").unwrap_err();
    assert_eq!((refused.code, refused.mates), (RefusalCode::UnsupportedPlacement, vec!["L".to_owned()]));
}

/// Width holds B's x sides between A's: centred, at a gap from one face or the other, or at
/// a share of the play, all exact.
#[test]
fn width_puts_the_tab_in_the_groove() {
    const NX: [f64; 3] = [-1.0, 0.0, 0.0];
    // A's sides at x = 20 and x = -20, B's at 10 and -10 in its frame: the play is 20.
    for (width, x) in [
        (WidthInput::Centered, 0.0),
        (WidthInput::Dimension { distance: "3 mm".into(), flipped: false }, 7.0),
        (WidthInput::Dimension { distance: "3 mm".into(), flipped: true }, -7.0),
        (WidthInput::Percent { percent: 25.0, flipped: false }, 5.0),
        (WidthInput::Percent { percent: 25.0, flipped: true }, -5.0),
    ] {
        let mut r = rig("width", 10.0);
        r.doc.mate_origin("O", r.a, true, false).unwrap();
        let (g0, g1, t0, t1) = (side(r.a, X), side(r.a, NX), side(r.b, X), side(r.b, NX));
        r.doc.mate_width("W", [&g0, &g1], [&t0, &t1], Alignment::Aligned, &width, false).unwrap();
        assert_eq!(r.placement(r.b).translation, DVec3::new(x, 10.0, 30.0), "{width:?}");
        let status = r.doc.solver();
        let motions: Vec<_> = status.free.iter().map(|f| f.motion.as_str()).collect();
        assert_eq!(motions, ["translate_y", "translate_z", "spin_x"], "{width:?}");
        // From A's +x side at 20 to B's at x + 10.
        let measured = r.doc.mate_measurements();
        let gap = measured[0].1.as_ref().unwrap().as_ref().expect("a Width measures its first gap");
        assert_eq!(gap.value, x - 10.0, "{width:?}");
    }

    let mut r = rig("width-edit", 10.0);
    r.doc.mate_origin("O", r.a, true, false).unwrap();
    let (g0, g1, t0, t1) = (side(r.a, X), side(r.a, [-1.0, 0.0, 0.0]), side(r.b, X), side(r.b, [-1.0, 0.0, 0.0]));
    let percent = WidthInput::Percent { percent: 25.0, flipped: false };
    r.doc.mate_width("W", [&g0, &g1], [&t0, &t1], Alignment::Aligned, &percent, false).unwrap();
    r.doc.edit_mates(&[("W".into(), MateEdit::Percent(50.0))]).unwrap();
    assert_eq!(r.placement(r.b).translation.x, 0.0);
    r.doc.edit_mates(&[("W".into(), MateEdit::Percent(0.0)), ("W".into(), MateEdit::Flipped(true))]).unwrap();
    assert_eq!(r.placement(r.b).translation.x, -10.0, "against the second groove face");
    assert_eq!(
        r.doc.edit_mates(&[("W".into(), MateEdit::Percent(120.0))]).unwrap_err().code,
        RefusalCode::InvalidValue
    );
    reopen(&mut r);

    // A distance across the groove disagrees with the width: both are named.
    let before = r.state();
    let refused =
        r.doc.mate_distance("X", &side(r.a, X), &side(r.b, X), "0 mm", Alignment::Aligned, false).unwrap_err();
    assert_eq!(
        (refused.code, refused.mates.clone()),
        (RefusalCode::OverConstrained, vec!["W".to_owned(), "X".to_owned()])
    );
    assert_eq!(r.state(), before);

    // A groove whose faces are not parallel is refused by name.
    let mut r = rig("width-skew", 10.0);
    let (g0, g1, t0, t1) = (side(r.a, X), top(r.a), side(r.b, X), side(r.b, [-1.0, 0.0, 0.0]));
    let before = r.state();
    let refused = r.doc.mate_width("W", [&g0, &g1], [&t0, &t1], Alignment::Aligned, &WidthInput::Centered, false);
    assert_eq!(refused.unwrap_err().code, RefusalCode::NotParallel);
    assert_eq!(r.state(), before);
}

/// A part's datum planes and its plane features mate like faces, proved from their
/// definitions; a plane the part does not have is refused.
#[test]
fn reference_planes_mate_like_faces() {
    let mut r = rig("planes", 10.0);
    let name = add_plane(&r.a_file, DatumPlane::Top, 4.0);
    r.doc.refresh_references(Vec::new());
    r.doc.finish_loading();
    let report = r.doc.rebuild();
    assert!(report.refusal.is_none(), "{report:?}");
    r.doc.mate_origin("O", r.a, true, false).unwrap();
    r.doc.mate_coincident("Mid", &plane(r.a, &name), &bottom(r.b), Alignment::Opposed, false).unwrap();
    assert_eq!(r.placement(r.b).translation, DVec3::new(10.0, 10.0, 4.0), "B's underside on the plane 4 up");
    r.doc
        .mate_coincident("Right", &plane(r.a, "Right plane"), &plane(r.b, "Right plane"), Alignment::Aligned, false)
        .unwrap();
    r.doc
        .mate_distance(
            "Front",
            &plane(r.a, "Front plane"),
            &plane(r.b, "Front plane"),
            "2 mm",
            Alignment::Aligned,
            false,
        )
        .unwrap();
    // The Front plane looks along -Y, so 2 mm along it is y = -2.
    assert_eq!(r.placement(r.b).translation, DVec3::new(0.0, -2.0, 4.0));
    assert_eq!(r.doc.solver().state, SolverState::Solved);
    let text = serde_json::to_string(r.doc.assembly()).unwrap();
    assert!(text.contains(r#""plane":{"on":"datum","datum":"right"}"#), "{text}");
    assert!(text.contains(r#""on":"feature""#), "{text}");
    reopen(&mut r);
    assert_eq!(r.doc.solver().state, SolverState::Solved);

    let before = r.state();
    let refused = r.doc.mate_parallel("No", &plane(r.a, "Nope"), &top(r.b), Alignment::Aligned, false).unwrap_err();
    assert_eq!(refused.code, RefusalCode::NoSuchFace);
    assert_eq!(r.state(), before);
}
