//! Pack and Go on real files: an assembly with a part, and a sub-assembly that places
//! another part, packed into a new folder with a prefix, then opened on its own after
//! the originals are gone, and worked on.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::DVec3;
use soliddraft_assembly::{
    AssemblyDocument, Freshness, KernelSource, LoadState, OpenOptions, PackError, PackRequest, Placement, pack_and_go,
};
use soliddraft_document::{DocKind, DocRef, identify_file};
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
    let dir = std::env::temp_dir().join(format!("soliddraft-pack-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn block(path: &Path, w: f64, h: f64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut doc = Document::new("Block", kernel());
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
    OpenOptions { display: Some(Arc::new(KernelSource::new(kernel()))), check_freshness: true, ..Default::default() }
}

/// rig.dwasm places parts/a.dwprt, sub/sub.dwasm (which places parts/b.dwprt) and the
/// same sub-assembly again, each saved.
fn tree(dir: &Path) -> PathBuf {
    let (a, b) = (dir.join("parts/a.dwprt"), dir.join("parts/b.dwprt"));
    block(&a, 40.0, 10.0);
    block(&b, 20.0, 5.0);
    let sub = dir.join("sub/sub.dwasm");
    std::fs::create_dir_all(sub.parent().unwrap()).unwrap();
    let mut s = AssemblyDocument::new("Sub");
    s.save(Some(&sub)).unwrap();
    let mut s = AssemblyDocument::open(&sub, with_kernel()).unwrap();
    s.insert(reference(&sub, &b), Placement::IDENTITY).unwrap();
    s.finish_loading();
    s.save(None).unwrap();
    let root = dir.join("rig.dwasm");
    let mut r = AssemblyDocument::new("Rig");
    r.save(Some(&root)).unwrap();
    let mut r = AssemblyDocument::open(&root, with_kernel()).unwrap();
    r.insert(reference(&root, &a), Placement::IDENTITY).unwrap();
    r.insert(reference(&root, &sub), Placement::translation(DVec3::new(60.0, 0.0, 0.0))).unwrap();
    r.insert(reference(&root, &sub), Placement::translation(DVec3::new(0.0, 60.0, 0.0))).unwrap();
    r.finish_loading();
    r.save(None).unwrap();
    root
}

#[test]
fn a_packed_assembly_opens_on_its_own_and_keeps_working() {
    let dir = folder("whole");
    let root = tree(&dir.join("src"));
    let into = dir.join("packed");
    let request = PackRequest { root: root.clone(), into: into.clone(), prefix: "P-".into(), ..Default::default() };
    let report = pack_and_go(&request, None).unwrap();
    assert_eq!(report.root, into.join("P-rig.dwasm"));
    assert_eq!(report.files.len(), 4, "two parts, the sub-assembly and the assembly: {:#?}", report.files);
    assert!(into.join("parts/P-a.dwprt").exists() && into.join("sub/P-sub.dwasm").exists());
    let last = report.files.last().unwrap();
    assert_eq!((last.kind, last.to.clone()), (DocKind::Assembly, report.root.clone()));
    let original = AssemblyDocument::open(&root, OpenOptions::default()).unwrap();

    // The originals go; the copy opens from itself, every part shown and current.
    std::fs::rename(dir.join("src"), dir.join("gone")).unwrap();
    let mut doc =
        AssemblyDocument::open(&report.root, OpenOptions { check_freshness: true, ..Default::default() }).unwrap();
    doc.finish_loading();
    assert_eq!(doc.table().counts().parts, 3);
    let root_def = doc.root();
    for (i, d) in doc.store().iter().filter(|(i, _)| *i != root_def) {
        if d.kind == DocKind::Part {
            assert_eq!(d.state, LoadState::DisplayReady, "{}", d.label);
        }
        assert_eq!(d.freshness, Freshness::Current, "{} ({i:?}) pins its packed copy", d.label);
    }
    assert_eq!(doc.id(), original.id(), "the same document in a new place");
    // The sub-assembly's references moved, so it pins a new revision; what is placed where
    // does not change.
    let placed = |d: &AssemblyDocument| -> Vec<(String, [f64; 3])> {
        let t = d.table();
        let mut v: Vec<_> = (1..t.len() as u32)
            .map(soliddraft_assembly::OccurrenceIndex)
            .map(|r| (t.path(r).to_string(), t.world(r).translation.to_array()))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    assert_eq!(placed(&doc), placed(&original));
    assert!(doc.assembly().sources.iter().all(|s| s.document.path.contains("P-")), "{:?}", doc.assembly().sources);

    // A later feature on top: Update finds nothing to move, a part is placed again, saved.
    let mut doc = AssemblyDocument::open(&report.root, with_kernel()).unwrap();
    doc.finish_loading();
    let rebuilt = doc.rebuild();
    assert_eq!(rebuilt.updated(), 0, "{rebuilt:?}");
    let a = into.join("parts/P-a.dwprt");
    doc.insert(reference(&report.root, &a), Placement::translation(DVec3::new(0.0, 0.0, 50.0))).unwrap();
    doc.finish_loading();
    doc.save(None).unwrap();
    let mut back = AssemblyDocument::open(&report.root, OpenOptions::default()).unwrap();
    back.finish_loading();
    assert_eq!(back.table().counts().parts, 4);
}

#[test]
fn pack_and_go_refuses_what_it_cannot_do_and_writes_nothing() {
    let dir = folder("refusals");
    let root = tree(&dir.join("src"));
    // Into the assembly's own folder.
    let same = PackRequest { root: root.clone(), into: dir.join("src"), ..Default::default() };
    assert!(matches!(pack_and_go(&same, None), Err(PackError::Destination(_))));
    // Into a folder that holds something.
    std::fs::create_dir_all(dir.join("full")).unwrap();
    std::fs::write(dir.join("full/x.txt"), b"x").unwrap();
    let full = PackRequest { root: root.clone(), into: dir.join("full"), ..Default::default() };
    assert!(matches!(pack_and_go(&full, None), Err(PackError::Destination(_))));
    // Flattened, two documents named alike would collide.
    block(&dir.join("src/sub/a.dwprt"), 5.0, 5.0);
    let mut s = AssemblyDocument::open(dir.join("src/sub/sub.dwasm"), with_kernel()).unwrap();
    let sub = dir.join("src/sub/sub.dwasm");
    s.insert(reference(&sub, &dir.join("src/sub/a.dwprt")), Placement::IDENTITY).unwrap();
    s.finish_loading();
    s.save(None).unwrap();
    let flat = PackRequest { root: root.clone(), into: dir.join("flat"), flatten: true, ..Default::default() };
    assert!(matches!(pack_and_go(&flat, None), Err(PackError::Clash(..))));
    assert!(!dir.join("flat").exists(), "nothing written");
    // A part gone missing.
    std::fs::remove_file(dir.join("src/parts/b.dwprt")).unwrap();
    let gone = PackRequest { root, into: dir.join("gone"), ..Default::default() };
    assert!(matches!(pack_and_go(&gone, None), Err(PackError::Missing(_))));
    assert!(!dir.join("gone").exists());
}
