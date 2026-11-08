//! Assemblies of real parts: insert, pattern, save, reopen from the display copies,
//! override occurrences inside sub-assemblies, undo, and Rebuild after a part changes.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::DVec3;
use soliddraft_assembly::{
    AssemblyDocument, Freshness, KernelSource, LoadState, OccurrencePath, OpenOptions, PatternRule, Placement,
    RefChange,
};
use soliddraft_document::{DocKind, DocRef, identify_file};
use soliddraft_kernel_api::Kernel;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, ProfileSource, SketchFeature,
    SketchPlane,
};
use soliddraft_tessellation::Quality;

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-assembly-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A block `w` x 20 x `h` centred on the origin in plan, saved at `path`, keeping its id when the file exists.
fn save_block(path: &Path, w: f64, h: f64) {
    let mut doc = match path.exists() {
        true => Document::open(path, kernel()).unwrap(),
        false => Document::new("Block", kernel()),
    };
    let ids: Vec<_> = doc.features().iter().map(|f| f.id).collect();
    for id in ids.into_iter().rev() {
        doc.execute(Command::DeleteFeature { id }).unwrap();
    }
    let mut sketch = Sketch::new();
    sketch.rectangle(P2::new(0.0, 0.0), w, 20.0).unwrap();
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

fn reference(from: &Path, to: &Path) -> DocRef {
    let i = identify_file(to).unwrap();
    let mut r = DocRef::new(i.id, i.kind, "", i.revision);
    r.set_path(from, to);
    r
}

fn with_kernel() -> OpenOptions {
    OpenOptions {
        display: Some(Arc::new(KernelSource::new(kernel()))),
        check_freshness: true,
        ..OpenOptions::default()
    }
}

#[test]
fn assemblies_open_from_their_copies_and_rebuild_when_parts_change() {
    let dir = folder("rebuild");
    let part = dir.join("parts/block.dwprt");
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    save_block(&part, 40.0, 10.0);

    // Build: two blocks and a pattern of the second; meshes come from the kernel.
    let root = dir.join("rig.dwasm");
    let mut doc = AssemblyDocument::new("Rig");
    doc.save(Some(&root)).unwrap();
    let mut doc = AssemblyDocument::open(&root, with_kernel()).unwrap();
    let a = doc.insert(reference(&root, &part), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference(&root, &part), Placement::translation(DVec3::new(100.0, 0.0, 0.0))).unwrap();
    doc.add_pattern(vec![b], PatternRule::Linear { direction: [0.0, 1.0, 0.0], spacing: 50.0, count: 3, second: None })
        .unwrap();
    doc.finish_loading();
    assert_eq!(doc.assembly().sources.len(), 1, "two placements of one part share a source");
    assert_eq!(doc.table().counts().parts, 4);
    assert_eq!(doc.store().counts()[2], 1, "one display-ready definition");
    let member: OccurrencePath = format!("{b}:{}.2", doc.assembly().patterns[0].id).parse().unwrap();
    let row = doc.table().find(&member).unwrap();
    assert_eq!(doc.table().world(row).translation, DVec3::new(100.0, 100.0, 0.0));
    let bounds = doc.table().bounds(row);
    assert!((bounds.max.x - 120.0).abs() < 1e-3 && (bounds.max.z - 10.0).abs() < 1e-3, "{bounds:?}");
    doc.save(None).unwrap();

    // Reopen with no kernel at all: everything shows from the file's own copies.
    let mut doc =
        AssemblyDocument::open(&root, OpenOptions { check_freshness: true, ..OpenOptions::default() }).unwrap();
    doc.finish_loading();
    let d = doc.store().iter().find(|(_, d)| d.kind == DocKind::Part).unwrap().1;
    assert_eq!(d.state, LoadState::DisplayReady);
    assert_eq!(d.freshness, Freshness::Current);

    // The part changes: the assembly still shows the pinned revision and says so.
    save_block(&part, 60.0, 10.0);
    let mut doc = AssemblyDocument::open(&root, with_kernel()).unwrap();
    doc.finish_loading();
    let row = doc.table().find(&member).unwrap();
    assert!((doc.table().bounds(row).max.x - 120.0).abs() < 1e-3, "still the pinned revision");
    let d = doc.store().iter().find(|(_, d)| d.kind == DocKind::Part).unwrap().1;
    assert!(matches!(d.freshness, Freshness::NewerAvailable { .. }), "{:?}", d.freshness);

    // Rebuild moves it on, and undo takes it back.
    let report = doc.rebuild();
    assert_eq!(report.updated(), 1);
    assert_eq!(report.sources[0].status, RefChange::Updated);
    doc.finish_loading();
    let row = doc.table().find(&member).unwrap();
    assert!((doc.table().bounds(row).max.x - 130.0).abs() < 1e-3, "{:?}", doc.table().bounds(row));
    doc.save(None).unwrap();
    assert!(doc.undo());
    doc.finish_loading();
    let row = doc.table().find(&member).unwrap();
    assert!((doc.table().bounds(row).max.x - 120.0).abs() < 1e-3);
    let _ = a;
}

#[test]
fn overrides_change_occurrences_inside_sub_assemblies() {
    let dir = folder("overrides");
    let part = dir.join("block.dwprt");
    save_block(&part, 10.0, 10.0);
    let sub = dir.join("sub.dwasm");
    let mut s = AssemblyDocument::new("Sub");
    s.save(Some(&sub)).unwrap();
    let mut s = AssemblyDocument::open(&sub, with_kernel()).unwrap();
    let inner = s.insert(reference(&sub, &part), Placement::translation(DVec3::new(0.0, 0.0, 5.0))).unwrap();
    s.finish_loading();
    s.save(None).unwrap();

    let root = dir.join("top.dwasm");
    let mut t = AssemblyDocument::new("Top");
    t.save(Some(&root)).unwrap();
    let mut t = AssemblyDocument::open(&root, with_kernel()).unwrap();
    let left = t.insert(reference(&root, &sub), Placement::IDENTITY).unwrap();
    let right = t.insert(reference(&root, &sub), Placement::translation(DVec3::new(50.0, 0.0, 0.0))).unwrap();
    t.finish_loading();
    assert_eq!(t.table().counts().parts, 2);
    assert_eq!(t.table().counts().assemblies, 3, "the root and two sub-assemblies");

    // Hide and move one part of one sub-assembly; the other is untouched.
    let p: OccurrencePath = format!("{right}/{inner}").parse().unwrap();
    t.set_hidden(std::slice::from_ref(&p), true).unwrap();
    t.move_to(&[(p.clone(), Placement::translation(DVec3::new(0.0, 7.0, 0.0)))]).unwrap();
    let row = t.table().find(&p).unwrap();
    assert_eq!(t.table().world(row).translation, DVec3::new(50.0, 7.0, 0.0));
    assert_eq!(t.table().counts().hidden, 1);
    assert!(t.assembly().override_at(&p).is_some(), "the change lives in the top assembly");
    let other: OccurrencePath = format!("{left}/{inner}").parse().unwrap();
    assert_eq!(t.table().world(t.table().find(&other).unwrap()).translation, DVec3::new(0.0, 0.0, 5.0));

    // Moving the sub-assembly carries its part along.
    t.move_by(&[format!("{right}").parse().unwrap()], &Placement::translation(DVec3::new(0.0, 0.0, 100.0))).unwrap();
    assert_eq!(t.table().world(row).translation, DVec3::new(50.0, 7.0, 100.0));
    assert!(t.undo());
    assert_eq!(t.table().world(row).translation, DVec3::new(50.0, 7.0, 0.0));
    assert!(t.redo());
    assert_eq!(t.table().world(row).translation, DVec3::new(50.0, 7.0, 100.0));

    // Saved and reopened, the overrides apply again, and the file opens on its own.
    t.save(None).unwrap();
    let mut back = AssemblyDocument::open(&root, OpenOptions::default()).unwrap();
    back.finish_loading();
    let row = back.table().find(&p).unwrap();
    assert_eq!(back.table().world(row).translation, DVec3::new(50.0, 7.0, 100.0));
    assert_eq!(back.table().counts().hidden, 1);
    assert_eq!(back.store().counts()[2], 1, "the part's copy came along with the sub-assembly's");
    assert_eq!(back.store().counts()[5], 0, "nothing failed");

    // Undo the hide: back to visible, and the override goes away with the move undone.
    let mut t = AssemblyDocument::open(&root, OpenOptions::default()).unwrap();
    t.set_hidden(std::slice::from_ref(&p), false).unwrap();
    assert_eq!(t.table().counts().hidden, 0);
    t.undo();
    assert_eq!(t.table().counts().hidden, 1);
}

#[test]
fn deleting_and_suppressing_keep_the_rest_in_place() {
    let dir = folder("delete");
    let part = dir.join("block.dwprt");
    save_block(&part, 10.0, 10.0);
    let root = dir.join("a.dwasm");
    let mut doc = AssemblyDocument::new("A");
    doc.save(Some(&root)).unwrap();
    let mut doc = AssemblyDocument::open(&root, with_kernel()).unwrap();
    let ids: Vec<_> = (0..5)
        .map(|i| {
            doc.insert(reference(&root, &part), Placement::translation(DVec3::new(i as f64 * 20.0, 0.0, 0.0))).unwrap()
        })
        .collect();
    doc.finish_loading();
    let last: OccurrencePath = format!("{}", ids[4]).parse().unwrap();
    let before = doc.table().world(doc.table().find(&last).unwrap());
    doc.delete(&ids[1..3]).unwrap();
    assert_eq!(doc.table().counts().parts, 3);
    assert_eq!(doc.table().world(doc.table().find(&last).unwrap()), before);
    doc.set_suppressed(std::slice::from_ref(&last), true).unwrap();
    assert_eq!(doc.table().counts().drawn, 2);
    assert!(doc.undo());
    assert_eq!(doc.table().counts().drawn, 3);
    assert!(doc.undo());
    doc.finish_loading();
    assert_eq!(doc.table().counts().parts, 5, "undo brings the deleted components back");
    assert!(doc.is_dirty());
}

/// Rig with parts A and B, each its own source. Deleting B takes its source along, so a
/// B file that has since gone cannot block A's Update. Undo brings the source back with B,
/// Redo drops it again, and the file saved after the delete no longer names B.
#[test]
fn deleting_the_last_placement_of_a_part_drops_its_source() {
    let dir = folder("delete-source");
    let elsewhere = folder("delete-source-elsewhere");
    let (a, b, root) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("rig.dwasm"));
    save_block(&a, 40.0, 10.0);
    save_block(&b, 10.0, 10.0);
    let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
    doc.save(Some(&root)).unwrap();
    let ca = doc.insert(reference(&root, &a), Placement::IDENTITY).unwrap();
    let cb = doc.insert(reference(&root, &b), Placement::translation(DVec3::new(50.0, 0.0, 0.0))).unwrap();
    doc.finish_loading();
    let source_b = doc.assembly().component(cb).unwrap().source;
    let paths =
        |doc: &AssemblyDocument| doc.assembly().sources.iter().map(|s| s.document.path.clone()).collect::<Vec<_>>();
    assert_eq!(paths(&doc), ["a.dwprt", "b.dwprt"]);

    doc.delete(&[cb]).unwrap();
    assert_eq!(paths(&doc), ["a.dwprt"], "B's source goes with its last component");
    assert!(doc.undo());
    doc.finish_loading();
    assert_eq!(paths(&doc), ["a.dwprt", "b.dwprt"], "undo brings B's source back");
    assert_eq!(doc.assembly().component(cb).unwrap().source, source_b);
    assert_eq!(doc.table().counts().parts, 2);
    assert!(doc.redo());
    doc.finish_loading();
    assert_eq!(paths(&doc), ["a.dwprt"], "redo drops it again");
    doc.save(None).unwrap();

    let mut doc = AssemblyDocument::open(&root, with_kernel()).unwrap();
    doc.finish_loading();
    assert_eq!(paths(&doc), ["a.dwprt"], "the saved file no longer names B");
    std::fs::rename(&b, elsewhere.join("b.dwprt")).unwrap();
    save_block(&a, 60.0, 10.0);
    let report = doc.rebuild();
    assert!(report.refusal.is_none(), "{report:?}");
    assert_eq!((report.updated(), report.missing()), (1, 0));
    doc.finish_loading();
    let path = OccurrencePath::root().child(soliddraft_assembly::Step::component(ca));
    let row = doc.table().find(&path).unwrap();
    assert!((doc.table().bounds(row).max.x - 30.0).abs() < 1e-3, "A takes its new revision");
}

/// The control: deleting one of two placements of A keeps A's source, which the other
/// still places, so Update still needs A's file and still takes its new revision.
#[test]
fn deleting_one_placement_of_a_shared_part_keeps_its_source() {
    let dir = folder("delete-shared-source");
    let elsewhere = folder("delete-shared-source-elsewhere");
    let (a, b, root) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("rig.dwasm"));
    save_block(&a, 40.0, 10.0);
    save_block(&b, 10.0, 10.0);
    let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
    doc.save(Some(&root)).unwrap();
    let first = doc.insert(reference(&root, &a), Placement::IDENTITY).unwrap();
    let second = doc.insert(reference(&root, &a), Placement::translation(DVec3::new(0.0, 50.0, 0.0))).unwrap();
    doc.insert(reference(&root, &b), Placement::translation(DVec3::new(50.0, 0.0, 0.0))).unwrap();
    doc.finish_loading();
    // The other placement is suppressed, but it is still a component placing A.
    let other = OccurrencePath::root().child(soliddraft_assembly::Step::component(second));
    doc.set_suppressed(std::slice::from_ref(&other), true).unwrap();
    doc.delete(&[first]).unwrap();
    let paths: Vec<_> = doc.assembly().sources.iter().map(|s| s.document.path.clone()).collect();
    assert_eq!(paths, ["a.dwprt", "b.dwprt"], "A's source is still placed");
    doc.set_suppressed(std::slice::from_ref(&other), false).unwrap();
    doc.save(None).unwrap();

    std::fs::rename(&a, elsewhere.join("a.dwprt")).unwrap();
    let report = doc.rebuild();
    assert!(report.refusal.as_ref().is_some_and(|r| r.contains("a.dwprt")), "{report:?}");
    std::fs::rename(elsewhere.join("a.dwprt"), &a).unwrap();
    save_block(&a, 60.0, 10.0);
    let report = doc.rebuild();
    assert!(report.refusal.is_none(), "{report:?}");
    assert_eq!(report.updated(), 1);
    doc.finish_loading();
    let path = OccurrencePath::root().child(soliddraft_assembly::Step::component(second));
    let row = doc.table().find(&path).unwrap();
    assert!((doc.table().bounds(row).max.x - 30.0).abs() < 1e-3, "the other A takes the new revision");
}

/// A file saved before deletes dropped sources still names B's after B is gone. Update
/// leaves that source out instead of looking for B, and Undo puts it back.
#[test]
fn an_update_drops_a_source_no_component_places() {
    let dir = folder("orphan-source");
    let elsewhere = folder("orphan-source-elsewhere");
    let (a, b, root) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("rig.dwasm"));
    save_block(&a, 40.0, 10.0);
    save_block(&b, 10.0, 10.0);
    let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
    doc.save(Some(&root)).unwrap();
    let ca = doc.insert(reference(&root, &a), Placement::IDENTITY).unwrap();
    doc.finish_loading();
    doc.save(None).unwrap();
    // The old file: A placed, and B's source left behind with no component.
    let package = soliddraft_assembly::AssemblyPackage::open(&root).unwrap();
    let mut old = (**package.assembly()).clone();
    let id = soliddraft_assembly::SourceId(old.take_id());
    old.sources.push(soliddraft_assembly::Source { id, document: reference(&root, &b) });
    old.save(&root, package.id(), None, &[]).unwrap();

    let mut doc = AssemblyDocument::open(&root, with_kernel()).unwrap();
    doc.finish_loading();
    assert_eq!(doc.assembly().sources.len(), 2);
    std::fs::rename(&b, elsewhere.join("b.dwprt")).unwrap();
    save_block(&a, 60.0, 10.0);
    let report = doc.rebuild();
    assert!(report.refusal.is_none(), "{report:?}");
    assert_eq!((report.updated(), report.missing(), report.sources.len()), (1, 0, 1));
    doc.finish_loading();
    let paths: Vec<_> = doc.assembly().sources.iter().map(|s| s.document.path.clone()).collect();
    assert_eq!(paths, ["a.dwprt"]);
    let path = OccurrencePath::root().child(soliddraft_assembly::Step::component(ca));
    let row = doc.table().find(&path).unwrap();
    assert!((doc.table().bounds(row).max.x - 30.0).abs() < 1e-3);
    assert!(doc.undo());
    assert_eq!(doc.assembly().sources.len(), 2, "undo restores the file as it was");
}

/// Placing a part or a sub-assembly again shares its source. Undoing that insert takes
/// only the new component: every earlier placement keeps the source, the undone state
/// saves and reopens whole, and Redo brings the new one back. Undoing the first insert,
/// the control, does take the source.
#[test]
fn undoing_a_repeated_insert_keeps_the_shared_source() {
    let dir = folder("insert-shared-source");
    let part = dir.join("a.dwprt");
    save_block(&part, 40.0, 10.0);
    let sub = dir.join("sub.dwasm");
    let mut s = AssemblyDocument::new_with("Sub", with_kernel());
    s.save(Some(&sub)).unwrap();
    s.insert(reference(&sub, &part), Placement::IDENTITY).unwrap();
    s.finish_loading();
    s.save(None).unwrap();
    for (what, file) in [("part", &part), ("sub-assembly", &sub)] {
        for earlier in [1, 2, 5] {
            let root = dir.join(format!("rig-{what}-{earlier}.dwasm"));
            let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
            doc.save(Some(&root)).unwrap();
            let at = |i: usize| Placement::translation(DVec3::new(i as f64 * 50.0, 0.0, 0.0));
            let placed: Vec<_> = (0..earlier).map(|i| doc.insert(reference(&root, file), at(i)).unwrap()).collect();
            doc.insert(reference(&root, file), at(earlier)).unwrap();
            doc.finish_loading();
            assert_eq!(doc.assembly().sources.len(), 1, "{what}, {earlier}");
            assert_eq!(doc.table().counts().parts, earlier + 1, "{what}, {earlier}");

            assert!(doc.undo());
            doc.finish_loading();
            let check = |doc: &AssemblyDocument| {
                assert_eq!(doc.assembly().sources.len(), 1, "{what}, {earlier}: the shared source stays");
                for (i, c) in placed.iter().enumerate() {
                    let source = doc.assembly().component(*c).unwrap().source;
                    assert!(doc.assembly().source(source).is_some(), "{what}, {earlier}: {c} keeps its source");
                    let path = OccurrencePath::root().child(soliddraft_assembly::Step::component(*c));
                    let row = doc.table().find(&path).unwrap_or_else(|| panic!("{what}, {earlier}: {c} is shown"));
                    assert_eq!(doc.table().world(row), at(i));
                }
                assert_eq!(doc.table().counts().parts, earlier, "{what}, {earlier}");
            };
            check(&doc);
            doc.save(None).unwrap();
            let mut back = AssemblyDocument::open(&root, with_kernel()).unwrap();
            back.finish_loading();
            check(&back);
            assert!(doc.redo());
            doc.finish_loading();
            assert_eq!((doc.assembly().sources.len(), doc.table().counts().parts), (1, earlier + 1));

            // The control: undo back past the first insert, which takes the source.
            while doc.undo() {}
            doc.finish_loading();
            assert!(doc.assembly().sources.is_empty() && doc.assembly().components.is_empty(), "{what}");
            doc.save(None).unwrap();
        }
    }
}

/// Every shown occurrence's world placement, by path.
fn layout(doc: &AssemblyDocument) -> Vec<(String, Placement)> {
    let t = doc.table();
    let mut out: Vec<_> = t
        .subtree(soliddraft_assembly::OccurrenceIndex::ROOT)
        .into_iter()
        .skip(1)
        .filter(|r| !t.flags(*r).has(soliddraft_assembly::Flags::REMOVED))
        .map(|r| (t.path(r).to_string(), t.world(r)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn same_layout(live: &[(String, Placement)], other: &[(String, Placement)], what: &str) {
    assert_eq!(live.len(), other.len(), "{what}");
    for ((p, a), (q, b)) in live.iter().zip(other) {
        assert_eq!(p, q, "{what}");
        let off = a.translation.distance(b.translation)
            + (a.rotation * DVec3::X).distance(b.rotation * DVec3::X)
            + (a.rotation * DVec3::Y).distance(b.rotation * DVec3::Y);
        assert!(off <= 1e-9, "{what}: {p} is at {a:?} live but {b:?} read back");
    }
}

/// Moving a pattern's seed with no mate moves its copies at once, as the saved pattern
/// says: the shown layout after Move, Undo and Redo is the one the file reopens with.
/// It holds for linear and circular patterns, a part and a sub-assembly seed, Move and
/// Move by. A copy with a placement of its own stays put.
#[test]
fn moving_a_pattern_seed_moves_its_copies() {
    let dir = folder("seed-move");
    let part = dir.join("block.dwprt");
    save_block(&part, 10.0, 10.0);
    let sub = dir.join("sub.dwasm");
    let mut s = AssemblyDocument::new_with("Sub", with_kernel());
    s.save(Some(&sub)).unwrap();
    s.insert(reference(&sub, &part), Placement::translation(DVec3::new(0.0, 0.0, 5.0))).unwrap();
    s.finish_loading();
    s.save(None).unwrap();
    let linear = PatternRule::Linear { direction: [1.0, 0.0, 0.0], spacing: 20.0, count: 3, second: None };
    let circular = PatternRule::Circular { origin: [0.0, 0.0, 0.0], axis: [0.0, 0.0, 1.0], angle: 90.0, count: 4 };
    let turn = Placement::new(glam::DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2), DVec3::new(0.0, 0.0, 3.0));
    for (case, file, rule, by) in [
        ("linear part", &part, &linear, false),
        ("circular part", &part, &circular, true),
        ("linear sub-assembly", &sub, &linear, false),
        ("circular sub-assembly", &sub, &circular, true),
    ] {
        let root = dir.join(format!("{}.dwasm", case.replace(' ', "-")));
        let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
        doc.save(Some(&root)).unwrap();
        let seed = doc.insert(reference(&root, file), Placement::translation(DVec3::new(10.0, 0.0, 0.0))).unwrap();
        let pattern = doc.add_pattern(vec![seed], rule.clone()).unwrap();
        doc.finish_loading();
        // The second copy has a placement of its own, which a seed move must not change.
        let own: OccurrencePath = format!("{seed}:{pattern}.2").parse().unwrap();
        let fixed = Placement::translation(DVec3::new(-50.0, -50.0, 0.0));
        doc.move_to(&[(own.clone(), fixed)]).unwrap();
        let start = layout(&doc);

        let path: OccurrencePath = format!("{seed}").parse().unwrap();
        match by {
            true => doc.move_by(std::slice::from_ref(&path), &turn).unwrap(),
            false => doc.move_to(&[(path.clone(), Placement::translation(DVec3::new(100.0, 0.0, 0.0)))]).unwrap(),
        }
        let moved = layout(&doc);
        assert_ne!(moved, start, "{case}");
        let first: OccurrencePath = format!("{seed}:{pattern}.1").parse().unwrap();
        let seed_at = doc.assembly().component(seed).unwrap().placement;
        let row = doc.table().find(&first).unwrap();
        let wanted = rule.member(1).compose(&seed_at);
        same_layout(&[("copy".into(), doc.table().world(row))], &[("copy".into(), wanted)], case);
        let row = doc.table().find(&own).unwrap();
        assert_eq!(doc.table().world(row), fixed, "{case}: the copy placed on its own stays");

        doc.save(None).unwrap();
        let mut back = AssemblyDocument::open(&root, with_kernel()).unwrap();
        back.finish_loading();
        same_layout(&moved, &layout(&back), &format!("{case}, reopened"));
        assert!(doc.undo());
        same_layout(&start, &layout(&doc), &format!("{case}, undone"));
        assert!(doc.redo());
        same_layout(&moved, &layout(&doc), &format!("{case}, redone"));
    }
}

/// A seed with twenty thousand copies moves them all in time proportional to the copies,
/// and so do Undo and Redo of the move: the root's bounds are redone once, not once per
/// copy.
#[test]
fn moving_a_seed_with_many_copies_scales() {
    const COPIES: u32 = 20_000;
    let dir = folder("seed-move-many");
    let part = dir.join("block.dwprt");
    save_block(&part, 10.0, 10.0);
    let root = dir.join("rig.dwasm");
    let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
    doc.save(Some(&root)).unwrap();
    let seed = doc.insert(reference(&root, &part), Placement::IDENTITY).unwrap();
    let rule = PatternRule::Linear { direction: [1.0, 0.0, 0.0], spacing: 20.0, count: COPIES, second: None };
    doc.add_pattern(vec![seed], rule).unwrap();
    doc.finish_loading();
    assert_eq!(doc.table().counts().parts, COPIES as usize);
    let path: OccurrencePath = format!("{seed}").parse().unwrap();
    let up = Placement::translation(DVec3::new(0.0, 0.0, 100.0));

    let started = std::time::Instant::now();
    doc.move_to(&[(path, up)]).unwrap();
    let moved = layout(&doc);
    assert!(doc.undo());
    assert!(doc.redo());
    let took = started.elapsed();
    same_layout(&moved, &layout(&doc), "redone");
    let all = doc.table().bounds(soliddraft_assembly::OccurrenceIndex::ROOT);
    assert!((all.min.z - 100.0).abs() < 1e-6 && (all.max.z - 110.0).abs() < 1e-6, "every copy rose: {all:?}");
    // Redoing the root's bounds per copy is 3 x 4e8 sibling visits, which takes far longer.
    assert!(took < std::time::Duration::from_secs(10), "{took:?}");
}

/// Every occurrence that is not suppressed, with its world placement, in path order.
/// Rows under a suppressed sub-assembly may or may not exist; they are never shown.
fn shown_layout(doc: &AssemblyDocument) -> Vec<(String, Placement)> {
    use soliddraft_assembly::Flags;
    let t = doc.table();
    let mut out: Vec<_> = t
        .subtree(soliddraft_assembly::OccurrenceIndex::ROOT)
        .into_iter()
        .skip(1)
        .filter(|r| !t.flags(*r).has(Flags::REMOVED) && !t.flags(*r).has(Flags::SUPPRESSED))
        .map(|r| (t.path(r).to_string(), t.world(r)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A child of a sub-assembly is moved, the sub-assembly suppressed, then the viewport
/// quality changed, which rereads the structure without the suppressed sub-assembly's
/// rows. Undoing all three and redoing them must show, after every step, exactly what the
/// saved file reopens with: the children come back when the suppression is undone, and
/// undoing the move finds the child it moved, whatever row it now has.
#[test]
fn undo_after_the_structure_is_reread_shows_what_the_file_holds() {
    let dir = folder("undo-reread");
    let part = dir.join("block.dwprt");
    save_block(&part, 10.0, 10.0);
    for children in [1, 2, 5] {
        let sub = dir.join(format!("sub-{children}.dwasm"));
        let mut s = AssemblyDocument::new_with("Sub", with_kernel());
        s.save(Some(&sub)).unwrap();
        let parts: Vec<_> = (0..children)
            .map(|i| {
                let at = Placement::translation(DVec3::new(0.0, f64::from(i) * 30.0, 0.0));
                s.insert(reference(&sub, &part), at).unwrap()
            })
            .collect();
        s.finish_loading();
        s.save(None).unwrap();

        let root = dir.join(format!("rig-{children}.dwasm"));
        let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
        doc.save(Some(&root)).unwrap();
        let s_id = doc.insert(reference(&root, &sub), Placement::translation(DVec3::new(100.0, 0.0, 0.0))).unwrap();
        doc.finish_loading();
        let s_path: OccurrencePath = format!("{s_id}").parse().unwrap();
        let child: OccurrencePath = format!("{s_id}/{}", parts[children as usize - 1]).parse().unwrap();

        let mut states = vec![shown_layout(&doc)];
        doc.move_to(&[(child.clone(), Placement::translation(DVec3::new(5.0, 0.0, 7.0)))]).unwrap();
        states.push(shown_layout(&doc));
        doc.set_suppressed(std::slice::from_ref(&s_path), true).unwrap();
        states.push(shown_layout(&doc));
        doc.set_viewport_quality(Quality::Slider { value: 10 }).unwrap();
        doc.finish_loading();
        states.push(shown_layout(&doc));

        let check = |doc: &mut AssemblyDocument, state: usize, step: &str| {
            doc.finish_loading();
            let live = shown_layout(doc);
            doc.save(None).unwrap();
            let mut back = AssemblyDocument::open(&root, with_kernel()).unwrap();
            back.finish_loading();
            assert_eq!(live, shown_layout(&back), "{children} children, {step}: live and saved differ");
            assert_eq!(live, states[state], "{children} children, {step}");
        };
        check(&mut doc, 3, "after the quality change");
        for (state, step) in [(2, "undo quality"), (1, "undo suppress"), (0, "undo move")] {
            assert!(doc.undo(), "{step}");
            check(&mut doc, state, step);
        }
        for (state, step) in [(1, "redo move"), (2, "redo suppress"), (3, "redo quality")] {
            assert!(doc.redo(), "{step}");
            check(&mut doc, state, step);
        }
    }
}

/// Root, sub-assembly S, sub-assembly T inside it, and parts in T. S is suppressed, then
/// T through S, then S shown again, and the viewport quality changed, which rereads the
/// structure without T's parts. Undoing all four clears T's own suppression while S still
/// hides it, then S's: T's parts must come back. After every Undo and Redo the document
/// shows exactly what the saved file reopens with.
#[test]
fn undo_regrows_a_nested_sub_assembly_once_nothing_above_it_is_suppressed() {
    let dir = folder("undo-nested-regrow");
    let part = dir.join("block.dwprt");
    save_block(&part, 10.0, 10.0);
    let assembly = |path: &Path, inner: &Path, count: u32| {
        let mut a = AssemblyDocument::new_with("Sub", with_kernel());
        a.save(Some(path)).unwrap();
        let ids: Vec<_> = (0..count)
            .map(|i| {
                let at = Placement::translation(DVec3::new(0.0, f64::from(i) * 30.0, 0.0));
                a.insert(reference(path, inner), at).unwrap()
            })
            .collect();
        a.finish_loading();
        a.save(None).unwrap();
        ids
    };
    for children in [1, 2, 5] {
        let (t, s) = (dir.join(format!("t-{children}.dwasm")), dir.join(format!("s-{children}.dwasm")));
        assembly(&t, &part, children);
        let t_id = assembly(&s, &t, 1)[0];
        let root = dir.join(format!("rig-{children}.dwasm"));
        let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
        doc.save(Some(&root)).unwrap();
        let s_id = doc.insert(reference(&root, &s), Placement::translation(DVec3::new(100.0, 0.0, 0.0))).unwrap();
        doc.finish_loading();
        let s_path: OccurrencePath = format!("{s_id}").parse().unwrap();
        let t_path: OccurrencePath = format!("{s_id}/{t_id}").parse().unwrap();
        assert_eq!(doc.table().counts().parts, children as usize);

        let mut states = vec![shown_layout(&doc)];
        doc.set_suppressed(std::slice::from_ref(&s_path), true).unwrap();
        states.push(shown_layout(&doc));
        doc.set_suppressed(std::slice::from_ref(&t_path), true).unwrap();
        states.push(shown_layout(&doc));
        doc.set_suppressed(std::slice::from_ref(&s_path), false).unwrap();
        states.push(shown_layout(&doc));
        doc.set_viewport_quality(Quality::Slider { value: 10 }).unwrap();
        doc.finish_loading();
        states.push(shown_layout(&doc));

        let check = |doc: &mut AssemblyDocument, state: usize, step: &str| {
            doc.finish_loading();
            let live = shown_layout(doc);
            doc.save(None).unwrap();
            let mut back = AssemblyDocument::open(&root, with_kernel()).unwrap();
            back.finish_loading();
            assert_eq!(live, shown_layout(&back), "{children} children, {step}: live and saved differ");
            assert_eq!(live, states[state], "{children} children, {step}");
        };
        check(&mut doc, 4, "after the quality change");
        for (state, step) in
            [(3, "undo quality"), (2, "undo unsuppress S"), (1, "undo suppress T"), (0, "undo suppress S")]
        {
            assert!(doc.undo(), "{step}");
            check(&mut doc, state, step);
        }
        assert_eq!(doc.table().counts().parts, children as usize, "T's parts are back");
        for (state, step) in
            [(1, "redo suppress S"), (2, "redo suppress T"), (3, "redo unsuppress S"), (4, "redo quality")]
        {
            assert!(doc.redo(), "{step}");
            check(&mut doc, state, step);
        }
    }
}

/// Every occurrence's world placement, by path.
fn world_layout(doc: &AssemblyDocument) -> Vec<(OccurrencePath, Placement)> {
    let t = doc.table();
    let mut out: Vec<_> = t
        .subtree(soliddraft_assembly::OccurrenceIndex::ROOT)
        .into_iter()
        .skip(1)
        .filter(|r| !t.flags(*r).has(soliddraft_assembly::Flags::REMOVED))
        .map(|r| (t.path(r), t.world(r)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn same_pose(a: &Placement, b: &Placement) -> bool {
    a.translation.distance(b.translation) <= 1e-9
        && [DVec3::X, DVec3::Y].iter().all(|v| (a.rotation * *v).distance(b.rotation * *v) <= 1e-12)
}

/// One rigid motion in the root's frame moves everything it selects once, even when a
/// selected sub-assembly holds other selected occurrences: S, S/T and S/T/C in any order
/// and combination, translated or turned, all end where the motion takes them. Undo,
/// Redo and a save and reopen agree.
#[test]
fn moving_a_sub_assembly_and_its_parts_together_moves_each_once() {
    let dir = folder("move-nested-selection");
    let part = dir.join("block.dwprt");
    save_block(&part, 10.0, 10.0);
    let make = |path: &Path, inner: &Path, at: Placement| {
        let mut a = AssemblyDocument::new_with("Sub", with_kernel());
        a.save(Some(path)).unwrap();
        let id = a.insert(reference(path, inner), at).unwrap();
        a.finish_loading();
        a.save(None).unwrap();
        id
    };
    let (t, s) = (dir.join("t.dwasm"), dir.join("s.dwasm"));
    let c_id = make(&t, &part, Placement::translation(DVec3::new(20.0, 0.0, 0.0)));
    let t_id = make(&s, &t, Placement::translation(DVec3::new(0.0, 5.0, 0.0)));
    let turn = Placement::new(glam::DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2), DVec3::new(1.0, 2.0, 3.0));
    let shift = Placement::translation(DVec3::new(10.0, 0.0, 0.0));
    let selections: [&[usize]; 7] = [&[0, 1], &[1, 0], &[0, 2], &[2, 0], &[1, 2], &[0, 1, 2], &[2, 1, 0]];
    for (n, picked) in selections.iter().enumerate() {
        for (m, delta) in [shift, turn].into_iter().enumerate() {
            let root = dir.join(format!("rig-{n}-{m}.dwasm"));
            let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
            doc.save(Some(&root)).unwrap();
            let s_id = doc.insert(reference(&root, &s), Placement::translation(DVec3::new(100.0, 0.0, 0.0))).unwrap();
            let other =
                doc.insert(reference(&root, &part), Placement::translation(DVec3::new(0.0, 50.0, 0.0))).unwrap();
            doc.finish_loading();
            let paths: Vec<OccurrencePath> =
                [format!("{s_id}"), format!("{s_id}/{t_id}"), format!("{s_id}/{t_id}/{c_id}")]
                    .iter()
                    .map(|p| p.parse().unwrap())
                    .collect();
            let chosen: Vec<OccurrencePath> = picked.iter().map(|i| paths[*i].clone()).collect();
            let top = chosen.iter().min_by_key(|p| p.steps().len()).unwrap().clone();
            let before = world_layout(&doc);
            doc.move_by(&chosen, &delta).unwrap();
            let after = world_layout(&doc);
            let what = format!("{chosen:?} by {delta:?}");
            for ((path, was), (again, now)) in before.iter().zip(&after) {
                assert_eq!(path, again, "{what}");
                let wanted = if path.starts_with(&top) { delta.compose(was) } else { *was };
                assert!(same_pose(now, &wanted), "{what}: {path} is at {now:?}, not {wanted:?}");
            }
            let unmoved: OccurrencePath = format!("{other}").parse().unwrap();
            assert!(after.iter().any(|(p, w)| *p == unmoved && same_pose(w, &before.iter().find(|(q, _)| *q == unmoved).unwrap().1)));

            let matches = |a: &[(OccurrencePath, Placement)], b: &[(OccurrencePath, Placement)], step: &str| {
                assert_eq!(a.len(), b.len(), "{what}, {step}");
                for ((p, x), (q, y)) in a.iter().zip(b) {
                    assert!(p == q && same_pose(x, y), "{what}, {step}: {p} at {x:?} against {y:?}");
                }
            };
            doc.save(None).unwrap();
            let mut back = AssemblyDocument::open(&root, with_kernel()).unwrap();
            back.finish_loading();
            matches(&after, &world_layout(&back), "reopened");
            assert!(doc.undo());
            matches(&before, &world_layout(&doc), "undone");
            assert!(doc.redo());
            matches(&after, &world_layout(&doc), "redone");
        }
    }
}

#[test]
fn references_resolve_from_the_file_that_holds_them() {
    // The sub-assembly and its part live outside the root's folder: the part's path is
    // relative to the sub-assembly, so it must be followed from there.
    let dir = folder("nested-paths");
    let part = dir.join("lib/parts/block.dwprt");
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    save_block(&part, 10.0, 10.0);
    let sub = dir.join("lib/sub.dwasm");
    let mut s = AssemblyDocument::new("Sub");
    s.save(Some(&sub)).unwrap();
    let mut s = AssemblyDocument::open(&sub, with_kernel()).unwrap();
    s.insert(reference(&sub, &part), Placement::IDENTITY).unwrap();
    s.finish_loading();
    s.save(None).unwrap();
    let root = dir.join("top/top.dwasm");
    std::fs::create_dir_all(root.parent().unwrap()).unwrap();
    let mut t = AssemblyDocument::new("Top");
    t.save(Some(&root)).unwrap();
    let mut t = AssemblyDocument::open(&root, with_kernel()).unwrap();
    t.insert(reference(&root, &sub), Placement::IDENTITY).unwrap();
    t.finish_loading();
    t.save(None).unwrap();

    let mut back =
        AssemblyDocument::open(&root, OpenOptions { check_freshness: true, ..OpenOptions::default() }).unwrap();
    back.finish_loading();
    for (_, d) in back.store().iter().filter(|(i, _)| *i != back.root()) {
        assert_eq!(d.freshness, Freshness::Current, "{}", d.label);
    }
}

#[test]
fn an_update_is_refused_whole_while_one_part_is_missing() {
    let dir = folder("atomic-update");
    let elsewhere = folder("missing-update-source");
    let a = dir.join("a.dwprt");
    let b = dir.join("b.dwprt");
    let root = dir.join("rig.dwasm");
    save_block(&a, 40.0, 10.0);
    save_block(&b, 10.0, 10.0);
    let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
    doc.save(Some(&root)).unwrap();
    let component = doc.insert(reference(&root, &a), Placement::IDENTITY).unwrap();
    doc.insert(reference(&root, &b), Placement::translation(DVec3::new(50.0, 0.0, 0.0))).unwrap();
    doc.finish_loading();
    doc.save(None).unwrap();
    let revision = doc.revision();
    let pins: Vec<_> = doc.assembly().sources.iter().map(|s| s.document.pinned.clone()).collect();
    let undo = doc.undo_label().map(str::to_owned);
    save_block(&a, 60.0, 10.0);
    std::fs::rename(&b, elsewhere.join("b.dwprt")).unwrap();
    let report = doc.rebuild();
    assert_eq!(report.missing(), 1, "{report:?}");
    assert!(report.refusal.as_ref().is_some_and(|r| r.contains("b.dwprt")), "{report:?}");
    assert!(report.sources.iter().any(|s| s.label == "b" && s.status == RefChange::Missing));
    assert_eq!(doc.revision(), revision, "a missing source must refuse the whole Update");
    assert_eq!(doc.assembly().sources.iter().map(|s| s.document.pinned.clone()).collect::<Vec<_>>(), pins);
    assert_eq!(doc.undo_label(), undo.as_deref());
    assert!(!doc.is_dirty());
    doc.finish_loading();
    let path = OccurrencePath::root().child(soliddraft_assembly::Step::component(component));
    let row = doc.table().find(&path).unwrap();
    assert!((doc.table().bounds(row).max.x - 20.0).abs() < 1e-3);
    std::fs::rename(elsewhere.join("b.dwprt"), &b).unwrap();
    let report = doc.rebuild();
    assert_eq!(report.updated(), 1);
    assert_eq!(report.sources.iter().filter(|s| s.status == RefChange::Unchanged).count(), 1);
    doc.finish_loading();
    let row = doc.table().find(&path).unwrap();
    assert!((doc.table().bounds(row).max.x - 30.0).abs() < 1e-3);
    assert_eq!(doc.undo_label(), Some("Update"));
}

#[test]
fn an_update_whose_new_revision_cannot_be_shown_keeps_the_old_one() {
    let dir = folder("unshowable-update");
    let a = dir.join("a.dwprt");
    let root = dir.join("rig.dwasm");
    save_block(&a, 40.0, 10.0);
    let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
    doc.save(Some(&root)).unwrap();
    let component = doc.insert(reference(&root, &a), Placement::IDENTITY).unwrap();
    doc.finish_loading();
    doc.save(None).unwrap();
    let mut doc =
        AssemblyDocument::open(&root, OpenOptions { check_freshness: true, ..OpenOptions::default() }).unwrap();
    doc.finish_loading();
    let revision = doc.revision();
    let pin = doc.assembly().sources[0].document.pinned.clone();
    let undo = doc.undo_label().map(str::to_owned);
    save_block(&a, 60.0, 10.0);
    let part = Document::open(&a, kernel()).unwrap();
    let bytes =
        part.to_bytes_with(&soliddraft_model::file::Extras { no_display_data: true, ..Default::default() }).unwrap();
    std::fs::write(&a, bytes).unwrap();
    let report = doc.rebuild();
    assert!(
        report.refusal.as_ref().is_some_and(|r| r.contains("a.dwprt") && r.contains("cannot be shown")),
        "{report:?}"
    );
    assert_eq!(doc.revision(), revision, "an unshowable revision must refuse Update: {report:?}");
    assert_eq!(doc.assembly().sources[0].document.pinned, pin);
    assert_eq!(doc.undo_label(), undo.as_deref());
    assert!(!doc.is_dirty());
    let path = OccurrencePath::root().child(soliddraft_assembly::Step::component(component));
    let row = doc.table().find(&path).unwrap();
    assert_eq!(doc.store().get(doc.table().definition(row)).state, LoadState::DisplayReady);
    assert!((doc.table().bounds(row).max.x - 20.0).abs() < 1e-3);
}

#[test]
fn the_first_save_changes_only_the_stored_path() {
    use soliddraft_document::{CacheOnlyStore, RevisionStore};
    let dir = folder("save-revision");
    let part = dir.join("a.dwprt");
    let root = dir.join("rig.dwasm");
    save_block(&part, 40.0, 10.0);
    let identity = identify_file(&part).unwrap();
    let revisions = Arc::new(CacheOnlyStore::new());
    revisions.record(identity.id, &identity.revision, &std::fs::read(&part).unwrap());
    let mut doc = AssemblyDocument::new_with("Rig", OpenOptions { revisions: Some(revisions), ..with_kernel() });
    let reference = DocRef::new(identity.id, identity.kind, part.to_string_lossy(), identity.revision);
    doc.insert(reference, Placement::IDENTITY).unwrap();
    doc.finish_loading();
    assert!(doc.store().iter().any(|(_, d)| d.kind == DocKind::Part && d.state == LoadState::DisplayReady));
    let before = doc.assembly().clone();
    let start = doc.revision();
    doc.save(Some(&root)).unwrap();
    let source = &doc.assembly().sources[0];
    assert_eq!(source.document.path, "a.dwprt");
    assert_eq!(source.id, before.sources[0].id);
    assert_eq!(source.document.id, before.sources[0].document.id);
    assert_eq!(source.document.pinned, before.sources[0].document.pinned);
    assert_eq!(doc.assembly().geometry_hash(), before.geometry_hash());
    assert_ne!(doc.revision(), start);
    assert!(!doc.is_dirty());
    assert_eq!(identify_file(&root).unwrap().revision, doc.revision());
    let mut restored = doc.assembly().clone();
    restored.sources[0].document.path = before.sources[0].document.path.clone();
    assert_eq!(restored.semantic_hash(), start);
    let saved = doc.revision();
    doc.save(None).unwrap();
    assert_eq!(doc.revision(), saved);
    doc.save(Some(&dir.join("copy.dwasm"))).unwrap();
    assert_eq!(doc.revision(), saved);
    let subfolder = dir.join("nested");
    std::fs::create_dir_all(&subfolder).unwrap();
    doc.save(Some(&subfolder.join("rig.dwasm"))).unwrap();
    assert_eq!(doc.assembly().sources[0].document.path, "../a.dwprt");
    assert_ne!(doc.revision(), saved);
    assert_eq!(doc.assembly().geometry_hash(), before.geometry_hash());
}

/// Deleting many placements of one part takes time in proportion to the placements, not
/// their square: twenty thousand copies, all but the first deleted at once, keep the
/// shared source, and Undo brings every copy back.
#[test]
fn deleting_many_placements_of_one_part_scales() {
    const COPIES: u32 = 20_000;
    let dir = folder("delete-many");
    let part = dir.join("a.dwprt");
    let root = dir.join("rig.dwasm");
    save_block(&part, 10.0, 10.0);
    let mut truth = soliddraft_assembly::Assembly::new("Rig");
    let source = truth.add_source(reference(&root, &part));
    let ids: Vec<_> = (0..COPIES)
        .map(|i| truth.add_component(source, Placement::translation(DVec3::new(f64::from(i) * 20.0, 0.0, 0.0))))
        .collect();
    truth.save(&root, soliddraft_document::DocumentId::new(), None, &[]).unwrap();
    let mut doc = AssemblyDocument::open(&root, with_kernel()).unwrap();
    doc.finish_loading();
    assert_eq!(doc.table().counts().parts, COPIES as usize);

    let started = std::time::Instant::now();
    doc.delete(&ids[1..]).unwrap();
    let took = started.elapsed();
    assert_eq!(doc.assembly().sources.len(), 1, "the first copy still places the part");
    assert_eq!(doc.table().counts().parts, 1);
    let all = doc.table().bounds(soliddraft_assembly::OccurrenceIndex::ROOT);
    assert!(
        (all.max.x - 5.0).abs() < 1e-3 && (all.min.x + 5.0).abs() < 1e-3,
        "only the first copy is bounded: {all:?}"
    );
    // Checking every copy against every other is 4e8 steps, which takes far longer.
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
    assert!(doc.undo());
    doc.finish_loading();
    assert_eq!((doc.assembly().sources.len(), doc.table().counts().parts), (1, COPIES as usize));
}

/// Every shown occurrence, the root included, with its bounds, in path order.
fn shown_bounds(doc: &AssemblyDocument) -> Vec<(String, soliddraft_assembly::Aabb)> {
    use soliddraft_assembly::Flags;
    let t = doc.table();
    let mut out: Vec<_> = t
        .subtree(soliddraft_assembly::OccurrenceIndex::ROOT)
        .into_iter()
        .filter(|r| !t.flags(*r).has(Flags::REMOVED) && !t.flags(*r).has(Flags::SUPPRESSED))
        .map(|r| (t.path(r).to_string(), t.bounds(r)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Root R places S, S places T (at depth 3) or the part C directly (at depth 2). S is
/// suppressed, then the part's own parent through S's retained rows, and both undone.
/// Suppressing the inner one used to redo S's bounds from children that S itself kept
/// suppressed, leaving it empty; undoing S's suppression then redid only R from that
/// empty box, so R and S stayed empty while C was drawn. After every step the bounds
/// of every shown occurrence match the saved file's.
#[test]
fn undoing_a_nested_suppression_restores_the_bounds_above_the_parts() {
    let dir = folder("undo-nested-suppress-bounds");
    let part = dir.join("block.dwprt");
    save_block(&part, 10.0, 10.0);
    let assembly = |path: &Path, inner: &Path, x: f64| {
        let mut a = AssemblyDocument::new_with("Sub", with_kernel());
        a.save(Some(path)).unwrap();
        let id = a.insert(reference(path, inner), Placement::translation(DVec3::new(x, 0.0, 0.0))).unwrap();
        a.finish_loading();
        a.save(None).unwrap();
        id
    };
    for depth in [2, 3] {
        let s = dir.join(format!("s-{depth}.dwasm"));
        let mut inner = vec![assembly(&s, &part, 10.0)];
        if depth == 3 {
            let t = dir.join("t.dwasm");
            let c = assembly(&t, &part, 10.0);
            inner = vec![assembly(&s, &t, 5.0), c];
        }
        let root = dir.join(format!("rig-{depth}.dwasm"));
        let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
        doc.save(Some(&root)).unwrap();
        let s_id = doc.insert(reference(&root, &s), Placement::translation(DVec3::new(100.0, 0.0, 0.0))).unwrap();
        doc.finish_loading();
        let s_path: OccurrencePath = format!("{s_id}").parse().unwrap();
        let below: OccurrencePath = format!("{s_id}/{}", inner[0]).parse().unwrap();
        let start = shown_bounds(&doc);
        let root_box = start[0].1;
        assert!(!root_box.is_empty() && (root_box.min.x - if depth == 2 { 105.0 } else { 110.0 }).abs() < 1e-6);

        let check = |doc: &mut AssemblyDocument, step: &str| {
            doc.finish_loading();
            let live = shown_bounds(doc);
            doc.save(None).unwrap();
            let mut back = AssemblyDocument::open(&root, with_kernel()).unwrap();
            back.finish_loading();
            assert_eq!(live, shown_bounds(&back), "depth {depth}, {step}: live and saved differ");
        };
        doc.set_suppressed(std::slice::from_ref(&s_path), true).unwrap();
        check(&mut doc, "suppress S");
        doc.set_suppressed(std::slice::from_ref(&below), true).unwrap();
        check(&mut doc, "suppress below S");
        assert!(doc.undo());
        check(&mut doc, "undo suppress below S");
        assert!(doc.undo());
        check(&mut doc, "undo suppress S");
        assert_eq!(shown_bounds(&doc), start, "depth {depth}: back where it started");
        assert!(doc.redo() && doc.redo());
        check(&mut doc, "redo both");
        // Shown again directly, not by Undo.
        doc.set_suppressed(std::slice::from_ref(&s_path), false).unwrap();
        doc.set_suppressed(std::slice::from_ref(&below), false).unwrap();
        check(&mut doc, "unsuppress S, then below it");
        assert_eq!(shown_bounds(&doc), start, "depth {depth}: shown again");
    }
}

/// S is suppressed while its part's mesh is still loading. The mesh arrives while S is
/// suppressed, and S is then shown again: S and the root take the part's box.
#[test]
fn a_mesh_that_arrives_under_a_suppressed_assembly_counts_once_it_is_shown() {
    let dir = folder("suppressed-mesh-arrival");
    let part = dir.join("block.dwprt");
    save_block(&part, 10.0, 10.0);
    let s = dir.join("s.dwasm");
    let mut a = AssemblyDocument::new_with("Sub", with_kernel());
    a.save(Some(&s)).unwrap();
    a.insert(reference(&s, &part), Placement::translation(DVec3::new(10.0, 0.0, 0.0))).unwrap();
    a.finish_loading();
    a.save(None).unwrap();
    let root = dir.join("rig.dwasm");
    let mut doc = AssemblyDocument::new_with("Rig", with_kernel());
    doc.save(Some(&root)).unwrap();
    let s_id = doc.insert(reference(&root, &s), Placement::translation(DVec3::new(100.0, 0.0, 0.0))).unwrap();
    let s_path: OccurrencePath = format!("{s_id}").parse().unwrap();
    doc.set_suppressed(std::slice::from_ref(&s_path), true).unwrap();
    doc.finish_loading();
    doc.set_suppressed(std::slice::from_ref(&s_path), false).unwrap();
    doc.finish_loading();
    let live = shown_bounds(&doc);
    doc.save(None).unwrap();
    let mut back = AssemblyDocument::open(&root, with_kernel()).unwrap();
    back.finish_loading();
    assert_eq!(live, shown_bounds(&back));
    assert!((live[0].1.min.x - 105.0).abs() < 1e-6, "{live:?}");
}

/// The x extent of the first definition's display mesh and of the root's bounds, with
/// the definition's state and problem.
fn shown_block(doc: &AssemblyDocument) -> (f64, f64, LoadState, Option<String>) {
    let row = doc.table().children(soliddraft_assembly::OccurrenceIndex::ROOT).next().unwrap();
    let d = doc.store().get(doc.table().definition(row));
    assert!(d.mesh.is_some(), "a display mesh");
    let mesh = d.bounds;
    let all = doc.table().bounds(soliddraft_assembly::OccurrenceIndex::ROOT);
    (mesh.max.x - mesh.min.x, all.max.x - all.min.x, d.state, d.problem.clone())
}

/// The assembly pins revision R1 of a block 10 wide and has no display copy of it; the
/// file holds R2, 30 wide, so the block is shown stale from R2. R1's file comes back
/// and the exact body is built at the pin. The display is then made at R1 too: the
/// block shows 10 wide and is no longer stale, and Save writes R1's picture under R1's
/// revision. Before, exact left R2's mesh in place, marked it exact-ready, and Save
/// stored it as R1's copy, so a reopened assembly showed R2 with no warning.
#[test]
fn building_the_exact_body_of_a_stale_part_shows_and_saves_the_pinned_revision() {
    use soliddraft_assembly::Assembly;
    use soliddraft_document::DocumentId;
    let source = KernelSource::new(kernel());
    for release in [false, true] {
        let dir = folder(&format!("stale-exact-{release}"));
        let part = dir.join("block.dwprt");
        save_block(&part, 10.0, 10.0);
        let r1 = std::fs::read(&part).unwrap();
        let root = dir.join("rig.dwasm");
        let mut truth = Assembly::new("Rig");
        let s = truth.add_source(reference(&root, &part));
        truth.add_component(s, Placement::IDENTITY);
        std::fs::write(&root, truth.to_bytes(DocumentId::new(), None, &[]).unwrap()).unwrap();
        save_block(&part, 30.0, 10.0);

        let mut doc = AssemblyDocument::open(&root, with_kernel()).unwrap();
        doc.finish_loading();
        let (mesh, all, state, problem) = shown_block(&doc);
        assert_eq!((mesh.round(), all.round(), state), (30.0, 30.0, LoadState::Stale), "R2, stale");
        assert!(problem.is_some());
        let row = doc.table().children(soliddraft_assembly::OccurrenceIndex::ROOT).next().unwrap();
        let def = doc.table().definition(row);
        assert!(doc.exact(def, &source).is_err(), "R1 is not available yet");
        assert_eq!(shown_block(&doc).2, LoadState::Stale);

        std::fs::write(&part, &r1).unwrap();
        doc.exact(def, &source).unwrap();
        let (mesh, all, state, problem) = shown_block(&doc);
        assert_eq!((mesh.round(), all.round(), state, problem), (10.0, 10.0, LoadState::ExactReady, None), "R1");
        if release {
            doc.release_exact(def);
            assert_eq!(shown_block(&doc).2, LoadState::DisplayReady);
        }
        // Reread the structure: the R1 display is kept, not marked stale.
        doc.set_viewport_quality(Quality::Slider { value: 10 }).unwrap();
        doc.finish_loading();
        doc.set_viewport_quality(Quality::default()).unwrap();
        doc.finish_loading();
        doc.save(None).unwrap();

        // Reopen with R2 in the file again: the saved copy is R1's own picture.
        save_block(&part, 30.0, 10.0);
        let mut back = AssemblyDocument::open(&root, with_kernel()).unwrap();
        back.finish_loading();
        let (mesh, _, state, problem) = shown_block(&back);
        assert_eq!((mesh.round(), state, problem), (10.0, LoadState::DisplayReady, None), "release {release}");
    }
}

/// While the part cannot be rebuilt at its pin, releasing nothing keeps it stale: a
/// stale definition never reads as display-ready, and Save writes no copy of it.
#[test]
fn a_stale_part_stays_stale_and_unsaved_without_its_pinned_revision() {
    use soliddraft_assembly::Assembly;
    use soliddraft_document::DocumentId;
    let dir = folder("stale-unsaved");
    let part = dir.join("block.dwprt");
    save_block(&part, 10.0, 10.0);
    let root = dir.join("rig.dwasm");
    let mut truth = Assembly::new("Rig");
    let s = truth.add_source(reference(&root, &part));
    truth.add_component(s, Placement::IDENTITY);
    std::fs::write(&root, truth.to_bytes(DocumentId::new(), None, &[]).unwrap()).unwrap();
    save_block(&part, 30.0, 10.0);
    let mut doc = AssemblyDocument::open(&root, with_kernel()).unwrap();
    doc.finish_loading();
    let row = doc.table().children(soliddraft_assembly::OccurrenceIndex::ROOT).next().unwrap();
    let def = doc.table().definition(row);
    assert!(doc.exact(def, &KernelSource::new(kernel())).is_err());
    doc.release_exact(def);
    assert_eq!(shown_block(&doc).2, LoadState::Stale);
    assert!(doc.copies().is_empty(), "no copy of R1 to write");
}
