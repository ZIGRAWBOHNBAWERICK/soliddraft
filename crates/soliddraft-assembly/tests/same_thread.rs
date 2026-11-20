//! Assemblies as the browser opens them: loaded on the caller's thread, a step per poll,
//! from documents held in memory rather than files in a folder. The page has one thread
//! and no folder, so every phase of an open, a Rebuild and exact work must run there.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use glam::DVec3;
use soliddraft_assembly::{
    AssemblyDocument, DefinitionKey, Freshness, KernelSource, LoadState, OpenOptions, PatternRule, Placement, RefChange,
};
use soliddraft_document::{CacheOnlyStore, DocKind, DocRef, OpenDocument, RevisionStore, identify, identify_file};
use soliddraft_kernel_api::Kernel;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, ProfileSource, SketchFeature,
    SketchPlane,
};
use soliddraft_tessellation::Level;

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-same-thread-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A block `w` x 20 x `h` centred on the origin in plan, saved at `path`, keeping its id
/// when the file exists.
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

fn options(same_thread: bool) -> OpenOptions {
    OpenOptions {
        display: Some(Arc::new(KernelSource::new(kernel()))),
        check_freshness: true,
        threads: 4,
        same_thread,
        ..OpenOptions::default()
    }
}

/// What an opened assembly holds: each definition's state, freshness, problem and mesh,
/// and every occurrence's box.
fn image(doc: &AssemblyDocument) -> (Vec<String>, Vec<String>) {
    let mut definitions: Vec<(DefinitionKey, String)> = doc
        .store()
        .iter()
        .map(|(_, d)| {
            let mesh = d.mesh.as_ref().map(|m| (m.positions.clone(), m.indices.clone(), m.faces.len()));
            (d.key.clone(), format!("{:?} {:?} {:?} {:?} {mesh:?}", d.kind, d.state, d.freshness, d.problem))
        })
        .collect();
    definitions.sort_by(|a, b| a.0.cmp(&b.0));
    let table = doc.table();
    let rows = (0..table.len())
        .map(|i| {
            let r = soliddraft_assembly::OccurrenceIndex(i as u32);
            format!("{} {:?} {:?}", table.path(r), table.world(r), table.bounds(r))
        })
        .collect();
    (definitions.into_iter().map(|(_, d)| d).collect(), rows)
}

#[test]
fn an_assembly_opened_on_the_callers_thread_is_the_one_the_loader_threads_open() {
    let dir = folder("same");
    let (small, large) = (dir.join("parts/small.dwprt"), dir.join("parts/large.dwprt"));
    std::fs::create_dir_all(small.parent().unwrap()).unwrap();
    save_block(&small, 30.0, 8.0);
    save_block(&large, 70.0, 12.0);

    // A sub-assembly of the small block, and a root that places both blocks, a pattern of
    // the large one and the sub-assembly.
    let sub = dir.join("sub.dwasm");
    let mut doc = AssemblyDocument::new_with("Sub", options(false));
    doc.save(Some(&sub)).unwrap();
    doc.insert(reference(&sub, &small), Placement::translation(DVec3::new(0.0, 40.0, 0.0))).unwrap();
    doc.save(None).unwrap();
    let root = dir.join("rig.dwasm");
    let mut doc = AssemblyDocument::new_with("Rig", options(false));
    doc.save(Some(&root)).unwrap();
    doc.insert(reference(&root, &small), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference(&root, &large), Placement::translation(DVec3::new(100.0, 0.0, 0.0))).unwrap();
    doc.add_pattern(vec![b], PatternRule::Linear { direction: [0.0, 1.0, 0.0], spacing: 50.0, count: 3, second: None })
        .unwrap();
    doc.insert(reference(&root, &sub), Placement::translation(DVec3::new(0.0, 0.0, 30.0))).unwrap();
    doc.finish_loading();
    doc.save(None).unwrap();

    // The large block changes, so freshness has something to find.
    save_block(&large, 80.0, 12.0);

    let mut threads = AssemblyDocument::open(&root, options(false)).unwrap();
    threads.finish_loading();
    // On the caller's thread, opened in the background as the window does and polled a
    // frame at a time until it is done.
    let opening = AssemblyDocument::open_in_background(&root, options(true));
    let (mut here, _scene) = opening.try_take().expect("opening here reads the structure when asked").unwrap();
    let mut frames = 0;
    while here.poll(Duration::ZERO) {
        frames += 1;
        assert!(frames < 1_000, "the steps end");
    }
    assert!(frames >= 2, "a poll with no budget takes one step, so the parts arrive over {frames} frames");
    assert!(!here.is_loading());
    assert_eq!(image(&here), image(&threads));
    let large_def = here.store().iter().find(|(_, d)| d.label.contains("large")).map(|(i, _)| i).unwrap();
    assert!(matches!(here.store().get(large_def).freshness, Freshness::NewerAvailable { .. }));
    assert_eq!(here.bvh_ready().map(|b| b.len()), Some(threads.bvh().len()), "the hierarchy is built at once");

    // A coarse level, asked for and taken a job at a time. The small block's file is at
    // the pinned revision, so its coarse level comes from the display data it keeps; the
    // large block's pinned revision is nowhere now, so it has none on either loader.
    let small_def = here.store().iter().find(|(_, d)| d.label.contains("small")).map(|(i, _)| i).unwrap();
    for doc in [&mut here, &mut threads] {
        doc.request_levels(&[(small_def, Level::Coarse), (large_def, Level::Coarse)]);
        doc.finish_levels();
    }
    assert_eq!(here.levels_pending(), 0);
    let coarse =
        |doc: &AssemblyDocument, def| doc.store().get(def).level_mesh(Level::Coarse).map(|m| m.positions.clone());
    assert!(coarse(&here, small_def).is_some());
    assert!(coarse(&here, large_def).is_none());
    assert_eq!(coarse(&here, small_def), coarse(&threads, small_def));

    // Rebuild on the caller's thread moves the large block on, as with threads.
    let (a, b) = (here.rebuild(), threads.rebuild());
    assert_eq!((a.updated(), a.refusal.clone()), (b.updated(), b.refusal.clone()));
    assert_eq!(a.updated(), 1);
    assert_eq!(image(&here), image(&threads));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Holds a part file's bytes as the browser holds a file handed to it: in the revision
/// store, and open under its name.
fn hold(store: &CacheOnlyStore, open: &mut Vec<OpenDocument>, name: &str, bytes: &[u8]) -> DocRef {
    let i = identify(std::io::Cursor::new(bytes)).unwrap();
    store.record(i.id, &i.revision, bytes);
    open.retain(|d| d.id != i.id);
    open.push(OpenDocument { id: i.id, kind: i.kind, path: Some(PathBuf::from(name)), revision: i.revision.clone() });
    DocRef::new(i.id, i.kind, name, i.revision)
}

#[test]
fn documents_held_without_a_folder_load_save_reopen_update_and_build_exactly() {
    // The part files exist only as bytes: the folder they were made in is gone.
    let dir = folder("held");
    let part = dir.join("block.dwprt");
    save_block(&part, 40.0, 10.0);
    let first = std::fs::read(&part).unwrap();
    save_block(&part, 60.0, 10.0);
    let second = std::fs::read(&part).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let store = Arc::new(CacheOnlyStore::new());
    let mut open = Vec::new();
    let r = hold(&store, &mut open, "held-block.dwprt", &first);
    let held = |open: &Vec<OpenDocument>| OpenOptions {
        open: open.clone(),
        revisions: Some(store.clone() as Arc<dyn RevisionStore>),
        ..options(true)
    };

    // Inserted into a new assembly: shown from the held bytes.
    let mut doc = AssemblyDocument::new_with("Rig", held(&open));
    doc.insert(r.clone(), Placement::IDENTITY).unwrap();
    doc.insert(r, Placement::translation(DVec3::new(100.0, 0.0, 0.0))).unwrap();
    doc.finish_loading();
    let part_def = |doc: &AssemblyDocument| doc.store().iter().find(|(_, d)| d.kind == DocKind::Part).unwrap().0;
    assert_eq!(doc.store().get(part_def(&doc)).state, LoadState::DisplayReady);

    // Saved as bytes, as the browser downloads them; nothing is written.
    let mut saved = None;
    let path = doc
        .save_with(Some(Path::new("Rig.dwasm")), |_, bytes| {
            saved = Some(bytes.to_vec());
            Ok(())
        })
        .unwrap();
    assert_eq!(path, PathBuf::from("Rig.dwasm"));
    assert!(!Path::new("Rig.dwasm").exists());
    assert!(!doc.is_dirty());
    let saved = saved.unwrap();

    // Reopened from those bytes: drawn from its copies, the part found among the held
    // documents at the revision it pins.
    let (mut doc, _) =
        AssemblyDocument::open_bytes_in_background(saved.clone(), Some(PathBuf::from("Rig.dwasm")), held(&open))
            .wait()
            .unwrap();
    doc.finish_loading();
    let def = part_def(&doc);
    assert_eq!(doc.store().get(def).state, LoadState::DisplayReady);
    assert_eq!(doc.store().get(def).freshness, Freshness::Current);
    let width = |doc: &AssemblyDocument| {
        let first = doc.table().children(soliddraft_assembly::OccurrenceIndex::ROOT).next().unwrap();
        let b = doc.table().bounds(first);
        b.max.x - b.min.x
    };
    assert!((width(&doc) - 40.0).abs() < 1e-3, "{}", width(&doc));

    // Exact work reads the held bytes at the pinned revision.
    let source = KernelSource::new(kernel());
    let exact = doc.exact(def, &source).unwrap();
    assert_eq!(exact.lock().unwrap().live_bodies().len(), 1);

    // A newer revision of the part is handed over: the assembly says so, and Update
    // moves it on from the held bytes alone.
    hold(&store, &mut open, "held-block.dwprt", &second);
    doc.refresh_references(open.clone());
    assert!(matches!(doc.store().get(part_def(&doc)).freshness, Freshness::NewerAvailable { .. }));
    let report = doc.rebuild();
    assert_eq!(report.refusal, None);
    assert_eq!(report.sources[0].status, RefChange::Updated);
    doc.finish_loading();
    assert!((width(&doc) - 60.0).abs() < 1e-3, "{}", width(&doc));
    assert_eq!(doc.store().get(part_def(&doc)).freshness, Freshness::Current);

    // Without the held documents the reopened assembly still shows its copies, and says
    // the part cannot be found.
    let (mut alone, _) =
        AssemblyDocument::open_bytes_in_background(saved, Some(PathBuf::from("Rig.dwasm")), options(true))
            .wait()
            .unwrap();
    alone.finish_loading();
    let def = part_def(&alone);
    assert_eq!(alone.store().get(def).state, LoadState::DisplayReady);
    assert_eq!(alone.store().get(def).freshness, Freshness::Missing);
}
