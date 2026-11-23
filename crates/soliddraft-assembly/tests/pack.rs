//! Pack and Go on real files: an assembly with a part, and a sub-assembly that places
//! another part, packed into a new folder with a prefix, then opened on its own after
//! the originals are gone, and worked on; the same pack as one zip; and the files a
//! browser page holds, packed into a zip with no folder to look in.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::DVec3;
use soliddraft_assembly::{
    AssemblyDocument, Folders, Freshness, HeldFiles, KernelSource, LoadState, OpenOptions, PackError, PackRequest,
    Placement, gather, pack_and_go, pack_zip,
};
use soliddraft_document::manifest::{Manifest, write_package};
use soliddraft_document::{ContentHash, DocKind, DocRef, DocumentId, identify_file};
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

/// Unpacks the zip `bytes` into `dir`, its entries' names as paths.
fn unzip(bytes: &[u8], dir: &Path) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut names = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().unwrap().into_owned();
        let to = dir.join(&name);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut out).unwrap();
        std::fs::write(&to, out).unwrap();
        names.push(name);
    }
    names.sort();
    names
}

/// Opens the assembly at `path` and checks every part shows and every reference pins
/// the copy beside it.
fn opens_on_its_own(path: &Path, parts: usize) -> AssemblyDocument {
    let mut doc = AssemblyDocument::open(path, OpenOptions { check_freshness: true, ..Default::default() }).unwrap();
    doc.finish_loading();
    assert_eq!(doc.table().counts().parts, parts);
    let root_def = doc.root();
    for (_, d) in doc.store().iter().filter(|(i, _)| *i != root_def) {
        if d.kind == DocKind::Part {
            assert_eq!(d.state, LoadState::DisplayReady, "{}", d.label);
        }
        assert_eq!(d.freshness, Freshness::Current, "{} pins its packed copy", d.label);
    }
    doc
}

#[test]
fn a_zip_holds_the_folder_pack_and_opens_on_its_own() {
    let dir = folder("zip");
    let root = tree(&dir.join("src"));
    let before = std::fs::read(&root).unwrap();
    let zip = dir.join("rig.zip");
    let request = PackRequest { root: root.clone(), into: zip.clone(), ..Default::default() };
    let report = pack_zip(&request, None).unwrap();
    assert_eq!(report.root, PathBuf::from("rig.dwasm"), "the assembly sits at the top of the zip");
    let mut places: Vec<String> = report.files.iter().map(|f| f.to.display().to_string()).collect();
    places.sort();
    assert_eq!(places, ["parts/a.dwprt", "parts/b.dwprt", "rig.dwasm", "sub/sub.dwasm"]);

    // The same documents, at the same revisions, as Pack and Go into a folder.
    let folder_report =
        pack_and_go(&PackRequest { root: root.clone(), into: dir.join("folder"), ..Default::default() }, None).unwrap();
    let revisions = |r: &soliddraft_assembly::PackReport| {
        let mut v: Vec<_> = r.files.iter().map(|f| (f.id, f.revision.clone())).collect();
        v.sort();
        v
    };
    assert_eq!(revisions(&report), revisions(&folder_report));
    let bytes = std::fs::read(&zip).unwrap();
    assert_eq!(bytes, gather(&request, &Folders { search: &[] }, None).unwrap().zip().unwrap(), "the same bytes again");

    // Unpacked anywhere once the originals are gone, it opens on its own and keeps
    // working: a part placed again, saved, reopened.
    std::fs::rename(dir.join("src"), dir.join("gone")).unwrap();
    let out = dir.join("unpacked");
    assert_eq!(unzip(&bytes, &out), places);
    let mut doc = opens_on_its_own(&out.join("rig.dwasm"), 3);
    let a = out.join("parts/a.dwprt");
    doc.insert(reference(&out.join("rig.dwasm"), &a), Placement::translation(DVec3::new(0.0, 0.0, 50.0))).unwrap();
    doc.finish_loading();
    doc.save(None).unwrap();
    opens_on_its_own(&out.join("rig.dwasm"), 4);
    assert_eq!(std::fs::read(dir.join("gone/rig.dwasm")).unwrap(), before, "the original is unchanged");

    // A zip that exists, or a name that is not a zip, is refused and nothing changes.
    std::fs::rename(dir.join("gone"), dir.join("src")).unwrap();
    assert!(matches!(pack_zip(&request, None), Err(PackError::Destination(_))));
    assert_eq!(std::fs::read(&zip).unwrap(), bytes);
    let not_zip = PackRequest { into: dir.join("rig.tar"), ..request.clone() };
    assert!(matches!(pack_zip(&not_zip, None), Err(PackError::Destination(_))));
    assert!(!dir.join("rig.tar").exists());
    // A part gone missing writes no zip.
    std::fs::remove_file(dir.join("src/parts/b.dwprt")).unwrap();
    let gone = PackRequest { into: dir.join("gone.zip"), ..request };
    assert!(matches!(pack_zip(&gone, None), Err(PackError::Missing(_))));
    assert!(!dir.join("gone.zip").exists());
}

/// The browser has no folders: the page holds each file by its name and bytes, as they
/// were handed to it. Pack and Go finds every reference among them by id and makes one
/// zip, which opens on its own once unpacked.
#[test]
fn files_a_page_holds_pack_into_a_zip_with_no_folder() {
    let dir = folder("held");
    tree(&dir.join("src"));
    let held = |rel: &str| {
        (PathBuf::from(Path::new(rel).file_name().unwrap()), std::fs::read(dir.join("src").join(rel)).unwrap())
    };
    let files = ["parts/a.dwprt", "parts/b.dwprt", "sub/sub.dwasm", "rig.dwasm"].map(held);
    let source = HeldFiles::new(files.clone());
    let request = PackRequest { root: PathBuf::from("rig.dwasm"), ..Default::default() };
    let pack = gather(&request, &source, None).unwrap();
    assert_eq!(pack.root, PathBuf::from("rig.dwasm"));
    let mut names: Vec<String> = pack.files.iter().map(|(f, _)| f.to.display().to_string()).collect();
    names.sort();
    assert_eq!(names, ["a.dwprt", "b.dwprt", "rig.dwasm", "sub.dwasm"], "the page has no folders to keep");
    let bytes = pack.zip().unwrap();
    std::fs::remove_dir_all(dir.join("src")).unwrap();
    let out = dir.join("unpacked");
    assert_eq!(unzip(&bytes, &out), names);
    let doc = opens_on_its_own(&out.join("rig.dwasm"), 3);
    assert!(doc.assembly().sources.iter().all(|s| !s.document.path.contains('/')), "{:?}", doc.assembly().sources);

    // A file the page was not handed is named, and nothing is packed.
    let without = HeldFiles::new(files[1..].to_vec());
    assert!(matches!(gather(&request, &without, None), Err(PackError::Missing(p)) if p == "parts/a.dwprt"));
}

#[test]
fn pack_names_reject_absolute_paths_and_parent_components() {
    let dir = folder("unsafe-names");
    let root = dir.join("rig.dwasm");
    AssemblyDocument::new("Rig").save(Some(&root)).unwrap();
    let cases = [
        ("../parent-".to_owned(), ""),
        (dir.join("absolute-").to_string_lossy().into_owned(), ""),
        (r"C:\absolute-".to_owned(), ""),
        (String::new(), "/../../parent"),
        (String::new(), r"\..\parent"),
    ];
    let mut accepted = Vec::new();
    for (index, (prefix, suffix)) in cases.into_iter().enumerate() {
        let into = dir.join(format!("pack-{index}"));
        let request = PackRequest {
            root: root.clone(),
            into: into.clone(),
            prefix,
            suffix: suffix.into(),
            flatten: true,
            ..Default::default()
        };
        let folder_result = pack_and_go(&request, None);
        let zip = into.with_extension("zip");
        let zip_result = pack_zip(&PackRequest { into: zip.clone(), ..request.clone() }, None);
        if !matches!(folder_result, Err(PackError::Destination(_)))
            || into.exists()
            || !matches!(zip_result, Err(PackError::Destination(_)))
            || zip.exists()
        {
            accepted.push((request.prefix, request.suffix, folder_result, zip_result));
        }
    }
    assert!(accepted.is_empty(), "{accepted:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn pack_folder_rejects_a_symlink_as_the_destination() {
    let dir = folder("symlink-destination");
    let root = dir.join("rig.dwasm");
    AssemblyDocument::new("Rig").save(Some(&root)).unwrap();
    let other = dir.join("other");
    std::fs::create_dir(&other).unwrap();
    let into = dir.join("packed");
    std::os::unix::fs::symlink(&other, &into).unwrap();
    let result = pack_and_go(&PackRequest { root, into, ..Default::default() }, None);
    assert!(matches!(result, Err(PackError::Destination(_))), "{result:?}");
    assert!(std::fs::read_dir(other).unwrap().next().is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

fn drawing_manifest(path: &Path, model: DocRef) {
    let mut manifest = Manifest::new(DocumentId::new(), DocKind::Drawing, ContentHash::of_bytes(b"drawing"));
    manifest.references.push(model);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_package(&manifest, &[]).unwrap()).unwrap();
}

#[test]
fn a_second_pack_cannot_replace_a_file_written_after_preflight() {
    let dir = folder("concurrent");
    let root = dir.join("src/rig.dwasm");
    AssemblyDocument::new("First").save(Some(&root)).unwrap();
    let other = dir.join("other/rig.dwasm");
    AssemblyDocument::new("Second").save(Some(&other)).unwrap();
    let drawing = dir.join("src/rig.dwdrw");
    drawing_manifest(&drawing, reference(&drawing, &root));
    let into = dir.join("packed");
    let complete_other_pack = |bytes: &[u8], _: &DocRef| {
        pack_and_go(&PackRequest { root: other.clone(), into: into.clone(), ..Default::default() }, None).unwrap();
        Ok(bytes.to_vec())
    };
    let result =
        pack_and_go(&PackRequest { root, into: into.clone(), ..Default::default() }, Some(&complete_other_pack));
    let winner = AssemblyDocument::open(into.join("rig.dwasm"), OpenOptions::default()).unwrap();
    assert_eq!(winner.assembly().name, "Second", "the first file published wins");
    assert!(matches!(result, Err(PackError::Destination(_))), "the pack that arrived late must report the collision");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn automatic_drawings_are_only_taken_from_beside_the_assembly() {
    let dir = folder("drawing-scope");
    let root = dir.join("src/rig.dwasm");
    AssemblyDocument::new("Rig").save(Some(&root)).unwrap();
    let local = dir.join("src/local.dwdrw");
    let nested = dir.join("src/nested/nested.dwdrw");
    let private = dir.join("private/private.dwdrw");
    for drawing in [&local, &nested, &private] {
        drawing_manifest(drawing, reference(drawing, &root));
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&private, dir.join("src/linked.dwdrw")).unwrap();
    let request =
        PackRequest { root, into: dir.join("packed.zip"), search: vec![dir.join("private")], ..Default::default() };
    let report = pack_zip(&request, Some(&|bytes, _| Ok(bytes.to_vec()))).unwrap();
    let bytes = std::fs::read(&request.into).unwrap();
    assert_eq!(unzip(&bytes, &dir.join("unpacked")), ["local.dwdrw", "rig.dwasm"]);
    assert_eq!(report.files.iter().filter(|file| file.kind == DocKind::Drawing).count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn browser_zip_keeps_the_referenced_upload_when_names_match() {
    let dir = folder("duplicate-uploads");
    let root = tree(&dir.join("src"));
    let unrelated = dir.join("unrelated/a.dwprt");
    block(&unrelated, 73.0, 12.0);
    let read = |file: &Path| (PathBuf::from(file.file_name().unwrap()), std::fs::read(file).unwrap());
    let wanted = dir.join("src/parts/a.dwprt");
    let files = [&unrelated, &wanted, &dir.join("src/parts/b.dwprt"), &dir.join("src/sub/sub.dwasm"), &root];
    let held = HeldFiles::new(files.into_iter().map(|file| read(file)));
    let request = PackRequest { root: "rig.dwasm".into(), ..Default::default() };
    let zip = gather(&request, &held, None).unwrap().zip().unwrap();
    let out = dir.join("unpacked");
    assert_eq!(unzip(&zip, &out), ["a.dwprt", "b.dwprt", "rig.dwasm", "sub.dwasm"]);
    assert!(
        std::fs::read(out.join("a.dwprt")).unwrap() == std::fs::read(wanted).unwrap(),
        "the zip must contain the bytes of the referenced upload"
    );
    opens_on_its_own(&out.join("rig.dwasm"), 3);
    std::fs::remove_dir_all(dir).unwrap();
}
