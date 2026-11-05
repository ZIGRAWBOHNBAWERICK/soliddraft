use super::*;
use crate::{AssemblyDocument, Placement};
use glam::DVec3;
use soliddraft_document::{
    DocumentId,
    manifest::{Entry, Manifest, Section, write_package},
};

struct Display;
impl DisplaySource for Display {
    fn part_mesh(&self, _: &[u8]) -> Result<(DisplayMesh, String), String> {
        Ok((DisplayMesh::cuboid(DVec3::ZERO, DVec3::ONE), "test".into()))
    }
    fn part_mesh_level(&self, bytes: &[u8], _: Option<&Quality>, _: Level) -> Result<(DisplayMesh, String), String> {
        self.part_mesh(bytes)
    }
}

fn folder(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("soliddraft-load-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn part(path: &Path, id: DocumentId, truth: &[u8]) -> ContentHash {
    let revision = ContentHash::of_bytes(truth);
    let mut manifest = Manifest::new(id, DocKind::Part, revision.clone());
    manifest.sections = vec![Section::required("part.json")];
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_package(&manifest, &[Entry::new("part.json", truth)]).unwrap()).unwrap();
    revision
}

fn assembly(path: &Path, target: &Path, count: usize) -> (Assembly, DocumentId) {
    let identity = identify_file(target).unwrap();
    let mut reference = DocRef::new(identity.id, identity.kind, "", identity.revision);
    reference.set_path(path, target);
    let mut assembly = Assembly::new("Assembly");
    let source = assembly.add_source(reference);
    for _ in 0..count {
        assembly.add_component(source, Placement::IDENTITY);
    }
    let id = DocumentId::new();
    assembly.save(path, id, None, &[]).unwrap();
    (assembly, id)
}

#[test]
fn nested_cache_free_loads_use_each_containing_assembly_for_display_and_levels() {
    for (name, library) in [("nested-parts", "lib"), ("nested-structure", "middle/nested")] {
        check_nested(name, library);
    }
}

fn check_nested(name: &str, library: &str) {
    let dir = folder(name);
    let block = dir.join(library).join("parts/block.dwprt");
    part(&block, DocumentId::new(), b"block");
    let sub = dir.join(library).join("sub.dwasm");
    assembly(&sub, &block, 1);
    let middle = dir.join("middle/middle.dwasm");
    assembly(&middle, &sub, 1);
    let root = dir.join("top/top.dwasm");
    assembly(&root, &middle, 1);
    let options = OpenOptions { display: Some(Arc::new(Display)), check_freshness: true, ..OpenOptions::default() };
    let package = AssemblyPackage::open(&root).unwrap();
    let mut lookup = Lookup::new(&package, Some(&root), &options);
    let structure =
        structure(package.id(), package.manifest().semantic_hash.clone(), package.assembly().clone(), &mut lookup)
            .unwrap();
    let mut doc = AssemblyDocument::open(&root, options).unwrap();
    doc.finish_loading();
    let definition = doc.store().iter().find(|(_, d)| d.kind == DocKind::Part).unwrap().1;
    assert_eq!(definition.freshness, Freshness::Current);
    assert_eq!(definition.state, crate::LoadState::DisplayReady);
    let reference = structure.refs.values().find(|r| r.kind == DocKind::Part).unwrap();
    for level in [Level::Coarse, Level::Fine] {
        assert!(lookup.level_mesh(reference, None, level, &mut []).is_ok(), "{level}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_cached_subassembly_bulk_falls_back_to_intact_source() {
    let dir = folder("cached-bulk");
    let block = dir.join("block.dwprt");
    part(&block, DocumentId::new(), b"block");
    let sub = dir.join("sub.dwasm");
    let (subassembly, id) = assembly(&sub, &block, crate::bulk::BULK_THRESHOLD + 1);
    let mut copy = crate::file::RefCopy::assembly(id, subassembly.semantic_hash(), &subassembly.truth_entries());
    copy.entries.retain(|(name, _)| name != crate::bulk::PLACEMENTS);
    let root = dir.join("root.dwasm");
    let (top, top_id) = assembly(&root, &sub, 1);
    top.save(&root, top_id, None, &[copy]).unwrap();
    let package = AssemblyPackage::open(&root).unwrap();
    let lookup = Lookup::new(&package, Some(&root), &OpenOptions::default());
    let fetched = lookup.fetch_structure(&top.sources[0].document).unwrap();
    assert_eq!(fetched.assembly.components.len(), subassembly.components.len());
    assert!(fetched.stale.is_none());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn failed_save_as_preserves_paths_and_retry_keeps_the_original_target() {
    let dir = folder("save-as");
    let block = dir.join("parts/block.dwprt");
    part(&block, DocumentId::new(), b"block");
    let old = dir.join("top/root.dwasm");
    let mut doc = AssemblyDocument::new("Root");
    doc.save(Some(&old)).unwrap();
    let identity = identify_file(&block).unwrap();
    let mut reference = DocRef::new(identity.id, identity.kind, "", identity.revision);
    reference.set_path(&old, &block);
    doc.insert(reference, Placement::IDENTITY).unwrap();
    let before = doc.assembly().sources.clone();
    let new = dir.join("new/deep/root.dwasm");
    std::fs::create_dir_all(&new).unwrap();
    assert!(doc.save(Some(&new)).is_err());
    assert_eq!(doc.assembly().sources, before, "failed Save As must not mutate live references");
    std::fs::remove_dir(&new).unwrap();
    doc.save(Some(&new)).unwrap();
    let reopened = AssemblyPackage::open(&new).unwrap();
    assert_eq!(reopened.assembly().sources[0].document.stored_path(&new), block);
    assert!(doc.undo());
    assert!(doc.redo());
    assert_eq!(doc.assembly().sources[0].document.stored_path(&new), block, "redo must keep rebased references");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unreadable_current_bytes_do_not_hide_a_valid_pinned_store_copy() {
    let dir = folder("unreadable-current");
    let moved = dir.join("moved.dwprt");
    let id = DocumentId::new();
    let pinned = part(&moved, id, b"old");
    let bytes = std::fs::read(&moved).unwrap();
    let root = dir.join("root.dwasm");
    Assembly::new("Root").save(&root, DocumentId::new(), None, &[]).unwrap();
    let package = AssemblyPackage::open(&root).unwrap();
    let store = Arc::new(soliddraft_document::CacheOnlyStore::new());
    store.record(id, &pinned, &bytes);
    let options =
        OpenOptions { revisions: Some(store.clone()), display: Some(Arc::new(Display)), ..OpenOptions::default() };
    let lookup = Lookup::new(&package, Some(&root), &options);
    let reference = DocRef::new(id, DocKind::Part, "old-name.dwprt", pinned.clone());
    assert!(lookup.resolve(&reference).is_some());
    std::fs::write(&moved, b"cut short").unwrap();
    assert!(lookup.mesh(&reference, None, &mut []).unwrap().1.is_none());
    assert!(lookup.level_mesh(&reference, None, Level::Fine, &mut []).is_ok());
    let wrong = dir.join("wrong.dwprt");
    part(&wrong, DocumentId::new(), b"old");
    store.record(id, &pinned, &std::fs::read(&wrong).unwrap());
    assert!(lookup.mesh(&reference, None, &mut []).is_err());
    assert!(lookup.level_mesh(&reference, None, Level::Fine, &mut []).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn indexed_revision_is_rechecked_against_the_bytes_used_for_display_and_levels() {
    let dir = folder("indexed-pin");
    let moved = dir.join("moved.dwprt");
    let id = DocumentId::new();
    let pinned = part(&moved, id, b"old");
    let root = dir.join("root.dwasm");
    let empty = Assembly::new("Root");
    empty.save(&root, DocumentId::new(), None, &[]).unwrap();
    let package = AssemblyPackage::open(&root).unwrap();
    let lookup =
        Lookup::new(&package, Some(&root), &OpenOptions { display: Some(Arc::new(Display)), ..OpenOptions::default() });
    let reference = DocRef::new(id, DocKind::Part, "old-name.dwprt", pinned.clone());
    assert_eq!(lookup.resolve(&reference).unwrap().1, Some(pinned));
    part(&moved, id, b"new");
    assert!(lookup.mesh(&reference, None, &mut []).unwrap().1.is_some(), "changed bytes must be stale, not pinned");
    assert!(lookup.level_mesh(&reference, None, Level::Fine, &mut []).is_err(), "never promote with another revision");
    part(&moved, DocumentId::new(), b"old");
    assert!(lookup.mesh(&reference, None, &mut []).is_err(), "another document is not a stale stand-in");
    std::fs::remove_dir_all(dir).unwrap();
}
