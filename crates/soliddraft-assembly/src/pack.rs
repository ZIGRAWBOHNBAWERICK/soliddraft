//! Pack and Go: copies an assembly into a new folder with every document it references,
//! at any depth, so the folder opens anywhere on its own.
//!
//! Parts and mesh references are copied byte for byte. Each assembly is written again
//! with its references pointing at the copies; a sub-assembly whose file changes that way
//! gets a new revision, so the assemblies above it pin the new one and their display
//! copies of it are replaced. A pin that was already behind its file stays as it was:
//! the packed assembly shows what the original showed. Document ids are kept: a packed
//! file is the same document in a new place. With a prefix or a suffix, every copied
//! file's name gets it; with `flatten`, every file lands in the destination folder
//! itself. Drawings in the assembly's folder whose model is packed come along when asked,
//! through the caller's `repoint`.
//!
//! Nothing is written unless every reference resolves, and nothing in the destination
//! is replaced.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use soliddraft_document::resolve::Resolver;
use soliddraft_document::{ContentHash, DocKind, DocRef, DocumentId, FolderIndex, identify_file};

use crate::file::{AssemblyPackage, RefCopy};

/// What to pack and where.
#[derive(Clone, Debug, Default)]
pub struct PackRequest {
    /// The assembly file.
    pub root: PathBuf,
    /// The destination folder. It must not exist, or be empty.
    pub into: PathBuf,
    /// Put before every copied file's name.
    pub prefix: String,
    /// Put after every copied file's name, before its extension.
    pub suffix: String,
    /// Every file in the destination folder itself, rather than in the folders it was in
    /// below the assembly's.
    pub flatten: bool,
    /// Other folders to look for references in.
    pub search: Vec<PathBuf>,
}

/// One file Pack and Go wrote.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PackedFile {
    /// The document.
    pub id: DocumentId,
    /// What it is.
    pub kind: DocKind,
    /// Where it was.
    pub from: PathBuf,
    /// Where the copy is.
    pub to: PathBuf,
    /// The copy's revision.
    pub revision: ContentHash,
    /// True when the copy was written again with new references rather than copied.
    pub rewritten: bool,
}

/// What Pack and Go did.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PackReport {
    /// The packed assembly's copy.
    pub root: PathBuf,
    /// Every file written, the assembly last.
    pub files: Vec<PackedFile>,
}

/// Why Pack and Go wrote nothing.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// A reference could not be found.
    #[error("{0} cannot be found; repair the reference or add a search folder before packing")]
    Missing(String),
    /// The destination is not usable.
    #[error("{0}")]
    Destination(String),
    /// Two documents would land on one file name.
    #[error("{0} and {1} would both be packed as {2}; pack without flatten, or rename one")]
    Clash(PathBuf, PathBuf, PathBuf),
    /// Reading or writing failed.
    #[error("{0}")]
    Io(String),
}

/// Rewrites a drawing's bytes so its model is the given reference. The drawing crate
/// supplies it; Pack and Go only finds the drawings and works out the reference.
pub type Repoint<'a> = &'a dyn Fn(&[u8], &DocRef) -> Result<Vec<u8>, String>;

#[derive(Clone, Debug)]
struct Doc {
    kind: DocKind,
    from: PathBuf,
    to: PathBuf,
    /// The file's revision when packing started.
    revision: ContentHash,
    /// For an assembly, the documents its sources name, resolved.
    sources: Vec<(DocumentId, PathBuf)>,
}

/// Packs `request.root` into `request.into`, with drawings of packed documents when
/// `drawings` is given.
pub fn pack_and_go(request: &PackRequest, drawings: Option<Repoint>) -> Result<PackReport, PackError> {
    let io = |what: &Path, e: &dyn std::fmt::Display| PackError::Io(format!("{}: {e}", what.display()));
    let root = std::path::absolute(&request.root).map_err(|e| io(&request.root, &e))?;
    let into = std::path::absolute(&request.into).map_err(|e| io(&request.into, &e))?;
    let base = root.parent().unwrap_or(Path::new("")).to_path_buf();
    check_destination(&into, &base)?;

    // 1. Every document the assembly reaches, by id.
    let first = identify_file(&root).map_err(|e| io(&root, &e))?;
    if first.kind != DocKind::Assembly {
        return Err(PackError::Destination(format!("{} is not an assembly", root.display())));
    }
    let mut docs: BTreeMap<DocumentId, Doc> = BTreeMap::new();
    let mut queue = vec![(first.id, root.clone(), first.kind, first.revision)];
    while let Some((id, path, kind, revision)) = queue.pop() {
        if docs.contains_key(&id) {
            continue;
        }
        let mut sources = Vec::new();
        if kind == DocKind::Assembly {
            let package = AssemblyPackage::open(&path).map_err(|e| io(&path, &e))?;
            let resolver = Resolver::new(Some(&path)).with_search_folders(&request.search);
            for s in &package.assembly().sources {
                let target = resolver.resolve(&s.document);
                let t = target.target().ok_or_else(|| PackError::Missing(s.document.path.clone()))?;
                let (Some(found), Some(at)) = (t.id(), t.path()) else {
                    return Err(PackError::Missing(s.document.path.clone()));
                };
                let now = t.revision().cloned().unwrap_or_else(|| s.document.pinned.clone());
                sources.push((found, at.to_path_buf()));
                queue.push((found, at.to_path_buf(), s.document.kind, now));
            }
        }
        docs.insert(id, Doc { kind, from: path, to: PathBuf::new(), revision, sources });
    }

    // 2. Where each lands.
    let mut taken: HashMap<PathBuf, PathBuf> = HashMap::new();
    for doc in docs.values_mut() {
        let relative = match doc.from.strip_prefix(&base) {
            Ok(r) if !request.flatten => r.to_path_buf(),
            _ => PathBuf::from(doc.from.file_name().unwrap_or_default()),
        };
        let to = into.join(renamed(&relative, &request.prefix, &request.suffix));
        if let Some(other) = taken.insert(to.clone(), doc.from.clone()) {
            return Err(PackError::Clash(other, doc.from.clone(), to));
        }
        doc.to = to;
    }

    // 3. Drawings of packed documents, found before anything is written.
    let mut drawn: Vec<(PathBuf, PathBuf, DocRef)> = Vec::new();
    if drawings.is_some() {
        let mut roots = vec![base.clone()];
        roots.extend(request.search.iter().cloned());
        let index = FolderIndex::scan(roots);
        for e in index.of_kind(DocKind::Drawing) {
            let Some(model) = e.identity.references.first() else { continue };
            if !model.id.is_some_and(|m| docs.contains_key(&m)) {
                continue;
            }
            let relative = match e.path.strip_prefix(&base) {
                Ok(r) if !request.flatten => r.to_path_buf(),
                _ => PathBuf::from(e.path.file_name().unwrap_or_default()),
            };
            let to = into.join(renamed(&relative, &request.prefix, &request.suffix));
            if let Some(other) = taken.insert(to.clone(), e.path.clone()) {
                return Err(PackError::Clash(other, e.path.clone(), to));
            }
            drawn.push((e.path.clone(), to, model.clone()));
        }
    }

    // 4. Parts and meshes as they are; assemblies deepest first, each with its references
    // moved and its sub-assemblies' new revisions.
    let mut report = Vec::new();
    let mut new_revision: HashMap<DocumentId, ContentHash> = HashMap::new();
    let make_parent = |to: &Path| {
        let dir = to.parent().unwrap_or(Path::new(""));
        std::fs::create_dir_all(dir).map_err(|e| io(dir, &e))
    };
    for (id, doc) in docs.iter().filter(|(_, d)| d.kind != DocKind::Assembly) {
        make_parent(&doc.to)?;
        std::fs::copy(&doc.from, &doc.to).map_err(|e| io(&doc.from, &e))?;
        new_revision.insert(*id, doc.revision.clone());
        report.push(PackedFile {
            id: *id,
            kind: doc.kind,
            from: doc.from.clone(),
            to: doc.to.clone(),
            revision: doc.revision.clone(),
            rewritten: false,
        });
    }
    // Every sub-assembly before each assembly that places it, however deep or shared.
    let assemblies = post_order(first.id, &docs);
    let mut packed_root = None;
    for id in &assemblies {
        let doc = &docs[id];
        let package = AssemblyPackage::open(&doc.from).map_err(|e| io(&doc.from, &e))?;
        let mut truth = (**package.assembly()).clone();
        // Sub-assembly revisions that change, old to new, for this file's copies.
        let mut moved_revisions: HashMap<(DocumentId, ContentHash), ContentHash> = HashMap::new();
        for (source, (found, _)) in truth.sources.iter_mut().zip(&doc.sources) {
            let target = &docs[found];
            source.document.set_path(&doc.to, &target.to);
            if source.document.kind == DocKind::Assembly
                && source.document.pinned == target.revision
                && let Some(now) = new_revision.get(found)
            {
                moved_revisions.insert((*found, source.document.pinned.clone()), now.clone());
                source.document.pinned = now.clone();
            }
        }
        // Nested sub-assemblies' copies sit in this file's cache too, at the pins their
        // own assemblies held; those move the same way.
        for (sub, sub_doc) in docs.iter().filter(|(_, d)| d.kind == DocKind::Assembly) {
            if let Some(now) = new_revision.get(sub)
                && *now != sub_doc.revision
            {
                moved_revisions.entry((*sub, sub_doc.revision.clone())).or_insert_with(|| now.clone());
            }
        }
        let mut reader = package.reader();
        let mut copies: Vec<RefCopy> = Vec::new();
        for (cid, rev) in package.copies() {
            match moved_revisions.get(&(cid, rev.clone())) {
                Some(now) => {
                    let packed = AssemblyPackage::open(&docs[&cid].to).map_err(|e| io(&docs[&cid].to, &e))?;
                    copies.push(RefCopy::assembly(cid, now.clone(), &packed.assembly().truth_entries()));
                }
                None => {
                    if let Some(c) = reader.copy(cid, &rev).map_err(|e| io(&doc.from, &e))? {
                        copies.push(c);
                    }
                }
            }
        }
        let kernel = package.manifest().kernel.clone();
        truth.normalize();
        let bytes = truth.to_bytes(*id, kernel, &copies).map_err(|e| io(&doc.to, &e))?;
        make_parent(&doc.to)?;
        soliddraft_document::save_atomic(&doc.to, &bytes).map_err(|e| io(&doc.to, &e))?;
        let revision = truth.semantic_hash();
        new_revision.insert(*id, revision.clone());
        report.push(PackedFile {
            id: *id,
            kind: DocKind::Assembly,
            from: doc.from.clone(),
            to: doc.to.clone(),
            revision,
            rewritten: true,
        });
        if *id == first.id {
            packed_root = Some(doc.to.clone());
        }
    }

    // 5. Drawings, pointed at their models' copies.
    if let Some(repoint) = drawings {
        for (from, to, model) in drawn {
            let target = &docs[&model.id.expect("matched by id")];
            let mut r = model.clone();
            r.set_path(&to, &target.to);
            if r.pinned == target.revision
                && let Some(now) = new_revision.get(&model.id.expect("matched by id"))
            {
                r.pinned = now.clone();
            }
            let bytes = std::fs::read(&from).map_err(|e| io(&from, &e))?;
            let out = repoint(&bytes, &r).map_err(|e| io(&from, &e))?;
            make_parent(&to)?;
            soliddraft_document::save_atomic(&to, &out).map_err(|e| io(&to, &e))?;
            let identity = identify_file(&to).map_err(|e| io(&to, &e))?;
            report.push(PackedFile {
                id: identity.id,
                kind: DocKind::Drawing,
                from,
                to,
                revision: identity.revision,
                rewritten: true,
            });
        }
    }
    // The assembly last, as the report promises.
    report.sort_by_key(|f| (f.id == first.id, f.kind == DocKind::Assembly));
    Ok(PackReport { root: packed_root.expect("the root is an assembly"), files: report })
}

/// The assemblies under `root`, each after every assembly it places.
fn post_order(root: DocumentId, docs: &BTreeMap<DocumentId, Doc>) -> Vec<DocumentId> {
    fn visit(
        id: DocumentId,
        docs: &BTreeMap<DocumentId, Doc>,
        seen: &mut HashSet<DocumentId>,
        out: &mut Vec<DocumentId>,
    ) {
        if docs[&id].kind != DocKind::Assembly || !seen.insert(id) {
            return;
        }
        for (child, _) in &docs[&id].sources {
            visit(*child, docs, seen, out);
        }
        out.push(id);
    }
    let mut out = Vec::new();
    visit(root, docs, &mut HashSet::new(), &mut out);
    out
}

/// `relative` with `prefix` before its file name and `suffix` after its stem.
fn renamed(relative: &Path, prefix: &str, suffix: &str) -> PathBuf {
    if prefix.is_empty() && suffix.is_empty() {
        return relative.to_path_buf();
    }
    let stem = relative.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match relative.extension() {
        Some(ext) => format!("{prefix}{stem}{suffix}.{}", ext.to_string_lossy()),
        None => format!("{prefix}{stem}{suffix}"),
    };
    relative.with_file_name(name)
}

fn check_destination(into: &Path, base: &Path) -> Result<(), PackError> {
    if into == base {
        return Err(PackError::Destination("pack into another folder than the assembly's own".into()));
    }
    match std::fs::read_dir(into) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(PackError::Destination(format!(
                    "{} is not empty; Pack and Go replaces nothing",
                    into.display()
                )));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PackError::Io(format!("{}: {e}", into.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_take_the_prefix_and_suffix() {
        assert_eq!(renamed(Path::new("parts/bolt.dwprt"), "A-", "-v2"), PathBuf::from("parts/A-bolt-v2.dwprt"));
        assert_eq!(renamed(Path::new("rig.dwasm"), "", ""), PathBuf::from("rig.dwasm"));
    }
}
