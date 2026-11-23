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
//! The pack is made in memory first ([`gather`]) and then written: into a folder
//! ([`pack_and_go`]), into one zip ([`pack_zip`], [`Pack::zip`]), or, in the browser,
//! from the files the page holds ([`HeldFiles`]) into a zip the page downloads. Nothing
//! is written unless every reference resolves, and nothing in the destination is
//! replaced.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use soliddraft_document::resolve::Resolver;
use soliddraft_document::{ContentHash, DocKind, DocRef, DocumentId, Identity, identify, identify_file};

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

/// Where Pack and Go finds and reads the files it packs: folders on disk ([`Folders`]),
/// or the files a browser page holds ([`HeldFiles`]).
pub trait PackSource {
    /// What `r`, held by the file at `from`, resolves to: the document's id, its file
    /// and its revision now, or `None` when it cannot be found.
    fn resolve(&self, from: &Path, r: &DocRef) -> Option<(DocumentId, PathBuf, Option<ContentHash>)>;
    /// The bytes of the file at `path`: the root, or one `resolve` named.
    fn read(&self, path: &Path) -> Result<Vec<u8>, String>;
    /// The drawings to look through for ones whose model is packed, each with its path
    /// and identity. `base` is the assembly's folder.
    fn drawings(&self, base: &Path) -> Vec<(PathBuf, Identity)>;
}

/// Files on disk, found as the assembly finds them: the stored path, the assembly's
/// folder and the search folders. Drawings come from beside the root assembly.
#[derive(Clone, Copy, Debug)]
pub struct Folders<'a> {
    /// Other folders to look in.
    pub search: &'a [PathBuf],
}

impl PackSource for Folders<'_> {
    fn resolve(&self, from: &Path, r: &DocRef) -> Option<(DocumentId, PathBuf, Option<ContentHash>)> {
        let resolution = Resolver::new(Some(from)).with_search_folders(self.search).resolve(r);
        let t = resolution.target()?;
        Some((t.id()?, t.path()?.to_path_buf(), t.revision().cloned()))
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn drawings(&self, base: &Path) -> Vec<(PathBuf, Identity)> {
        let Ok(entries) = std::fs::read_dir(base) else { return Vec::new() };
        entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if DocKind::of_path(&path) != Some(DocKind::Drawing)
                    || entry.file_name().to_string_lossy().starts_with('.')
                    || !entry.file_type().ok()?.is_file()
                {
                    return None;
                }
                let identity = identify_file(&path).ok()?;
                (identity.kind == DocKind::Drawing).then_some((path, identity))
            })
            .collect()
    }
}

/// The files a browser page holds, which has no folders: each known by its name and
/// bytes. A reference is found by its target's id, the file at the revision it pins
/// first, and by its file name when it has no id.
#[derive(Clone, Debug, Default)]
pub struct HeldFiles {
    files: Vec<(PathBuf, Vec<u8>, Identity)>,
}

impl HeldFiles {
    /// The documents among `files`, each a name and its bytes. Files that are not
    /// documents this version reads are left out.
    pub fn new(files: impl IntoIterator<Item = (PathBuf, Vec<u8>)>) -> HeldFiles {
        let files = files
            .into_iter()
            .filter_map(|(name, bytes)| identify(Cursor::new(&bytes)).ok().map(|i| (name, bytes, i)))
            .collect();
        HeldFiles { files }
    }
}

impl PackSource for HeldFiles {
    fn resolve(&self, _from: &Path, r: &DocRef) -> Option<(DocumentId, PathBuf, Option<ContentHash>)> {
        let name = Path::new(&r.path).file_name();
        let found = match r.id {
            Some(id) => {
                let same: Vec<_> = self.files.iter().filter(|(_, _, i)| i.id == id && i.kind == r.kind).collect();
                same.iter().find(|(_, _, i)| i.revision == r.pinned).or(same.first()).copied()
            }
            None => self.files.iter().find(|(p, _, i)| i.kind == r.kind && p.file_name() == name),
        }?;
        Some((found.2.id, found.0.clone(), Some(found.2.revision.clone())))
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
        self.files
            .iter()
            .find(|(p, _, _)| p == path)
            .map(|(_, bytes, _)| bytes.clone())
            .ok_or_else(|| format!("{} was not opened with the assembly", path.display()))
    }

    fn drawings(&self, _base: &Path) -> Vec<(PathBuf, Identity)> {
        self.files
            .iter()
            .filter(|(_, _, i)| i.kind == DocKind::Drawing)
            .map(|(p, _, i)| (p.clone(), i.clone()))
            .collect()
    }
}

/// A pack made in memory, before it is written: every file's report and bytes, the
/// assembly last. Each file's `to` is relative to where the pack goes.
#[derive(Clone, Debug)]
pub struct Pack {
    /// The packed assembly's place in the pack.
    pub root: PathBuf,
    /// Every file and its bytes.
    pub files: Vec<(PackedFile, Vec<u8>)>,
}

impl Pack {
    /// What writing the pack under `into` does: every file's `to` joined to it.
    pub fn report(&self, into: &Path) -> PackReport {
        let files = self.files.iter().map(|(f, _)| PackedFile { to: into.join(&f.to), ..f.clone() }).collect();
        PackReport { root: into.join(&self.root), files }
    }

    /// The pack as one zip: every file at its place in the pack, stored as it is (the
    /// documents are compressed already), with a fixed date, so a pack gives the same
    /// bytes each time.
    pub fn zip(&self) -> Result<Vec<u8>, PackError> {
        use std::io::Write as _;
        let io = |e: &dyn std::fmt::Display| PackError::Io(format!("cannot make the zip: {e}"));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default());
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (f, bytes) in &self.files {
            check_output_path(&f.to)?;
            zip.start_file(entry_name(&f.to), options).map_err(|e| io(&e))?;
            zip.write_all(bytes).map_err(|e| io(&e))?;
        }
        Ok(zip.finish().map_err(|e| io(&e))?.into_inner())
    }
}

/// A place in the pack as a zip entry's name: forward slashes.
fn entry_name(path: &Path) -> String {
    path.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// The folder a pack is laid out in before it is written anywhere. References between
/// packed files are worked out lexically inside it, so it need not exist.
const LAYOUT: &str = "/soliddraft-pack";

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

/// Packs `request.root` into `request.into`, a new or empty folder, with drawings of
/// packed documents when `drawings` is given.
pub fn pack_and_go(request: &PackRequest, drawings: Option<Repoint>) -> Result<PackReport, PackError> {
    let io = |what: &Path, e: &dyn std::fmt::Display| PackError::Io(format!("{}: {e}", what.display()));
    let root = std::path::absolute(&request.root).map_err(|e| io(&request.root, &e))?;
    let into = std::path::absolute(&request.into).map_err(|e| io(&request.into, &e))?;
    check_destination(&into, root.parent().unwrap_or(Path::new("")))?;
    let pack = gather(&PackRequest { root, ..request.clone() }, &Folders { search: &request.search }, drawings)?;
    std::fs::create_dir_all(&into).map_err(|e| io(&into, &e))?;
    let metadata = std::fs::symlink_metadata(&into).map_err(|e| io(&into, &e))?;
    if !metadata.is_dir() {
        return Err(PackError::Destination(format!("{} must be a directory", into.display())));
    }
    let destination = into.canonicalize().map_err(|e| io(&into, &e))?;
    for (f, bytes) in &pack.files {
        let mut dir = destination.clone();
        let parent = f.to.parent().expect("validated output path");
        for component in parent.components() {
            dir.push(component);
            if let Err(e) = std::fs::create_dir(&dir)
                && e.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(io(&dir, &e));
            }
            let resolved = dir.canonicalize().map_err(|e| io(&dir, &e))?;
            if !resolved.starts_with(&destination) || !resolved.is_dir() {
                return Err(PackError::Destination(format!(
                    "{} must resolve to a directory inside the pack",
                    dir.display()
                )));
            }
            dir = resolved;
        }
        let to = dir.join(f.to.file_name().expect("validated output path"));
        soliddraft_document::save_new(&to, bytes).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                PackError::Destination(format!("{} already exists; Pack and Go replaces nothing", to.display()))
            } else {
                io(&to, &e)
            }
        })?;
    }
    Ok(pack.report(&into))
}

/// Packs `request.root` into one zip file, `request.into`, which must not exist. Each
/// file's `to` in the report is its place in the zip.
pub fn pack_zip(request: &PackRequest, drawings: Option<Repoint>) -> Result<PackReport, PackError> {
    let io = |what: &Path, e: &dyn std::fmt::Display| PackError::Io(format!("{}: {e}", what.display()));
    let root = std::path::absolute(&request.root).map_err(|e| io(&request.root, &e))?;
    if !request.into.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
        return Err(PackError::Destination(format!("{} does not end in .zip", request.into.display())));
    }
    if request.into.exists() {
        return Err(PackError::Destination(format!(
            "{} already exists; Pack and Go replaces nothing",
            request.into.display()
        )));
    }
    let pack = gather(&PackRequest { root, ..request.clone() }, &Folders { search: &request.search }, drawings)?;
    let bytes = pack.zip()?;
    soliddraft_document::save_new(&request.into, &bytes).map_err(|e| io(&request.into, &e))?;
    Ok(pack.report(Path::new("")))
}

/// Gathers `request.root` and every document it reaches through `source`, and lays
/// them out as `request` says, writing nothing. `request.into` is not used.
pub fn gather(request: &PackRequest, source: &dyn PackSource, drawings: Option<Repoint>) -> Result<Pack, PackError> {
    if !safe_name_fragment(&request.prefix) || !safe_name_fragment(&request.suffix) {
        return Err(PackError::Destination("Pack and Go prefixes and suffixes must be filename fragments".into()));
    }
    let io = |what: &Path, e: &dyn std::fmt::Display| PackError::Io(format!("{}: {e}", what.display()));
    let layout = Path::new(LAYOUT);
    let root = request.root.clone();
    let base = root.parent().unwrap_or(Path::new("")).to_path_buf();
    let read = |path: &Path| source.read(path).map_err(PackError::Io);
    let open = |path: &Path| AssemblyPackage::from_bytes(read(path)?).map_err(|e| io(path, &e));

    // 1. Every document the assembly reaches, by id.
    let first = identify(Cursor::new(read(&root)?)).map_err(|e| io(&root, &e))?;
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
            let package = open(&path)?;
            for s in &package.assembly().sources {
                let (found, at, now) =
                    source.resolve(&path, &s.document).ok_or_else(|| PackError::Missing(s.document.path.clone()))?;
                let now = now.unwrap_or_else(|| s.document.pinned.clone());
                sources.push((found, at.clone()));
                queue.push((found, at, s.document.kind, now));
            }
        }
        docs.insert(id, Doc { kind, from: path, to: PathBuf::new(), revision, sources });
    }

    // 2. Where each lands.
    let place = |from: &Path| -> Result<PathBuf, PackError> {
        let relative = match from.strip_prefix(&base) {
            Ok(r) if !request.flatten => r.to_path_buf(),
            _ => PathBuf::from(from.file_name().unwrap_or_default()),
        };
        let to = renamed(&relative, &request.prefix, &request.suffix);
        check_output_path(&to)?;
        Ok(layout.join(to))
    };
    let mut taken: HashMap<PathBuf, PathBuf> = HashMap::new();
    for doc in docs.values_mut() {
        let to = place(&doc.from)?;
        if let Some(other) = taken.insert(to.clone(), doc.from.clone()) {
            return Err(PackError::Clash(other, doc.from.clone(), unlaid(&to)));
        }
        doc.to = to;
    }

    // 3. Drawings of packed documents, found before anything is made.
    let mut drawn: Vec<(PathBuf, PathBuf, DocRef)> = Vec::new();
    if drawings.is_some() {
        // One copy of each drawing: the one nearest the assembly's folder. Another, such as
        // the copy an earlier pack put in a subfolder, is the same document.
        let mut found: BTreeMap<DocumentId, (PathBuf, Identity)> = BTreeMap::new();
        for (path, identity) in source.drawings(&base) {
            let depth = |p: &Path| (p.components().count(), p.to_path_buf());
            match found.get(&identity.id) {
                Some((kept, _)) if depth(kept) <= depth(&path) => {}
                _ => {
                    found.insert(identity.id, (path, identity));
                }
            }
        }
        let mut found: Vec<(PathBuf, Identity)> = found.into_values().collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, identity) in found {
            let Some(model) = identity.references.first() else { continue };
            if !model.id.is_some_and(|m| docs.contains_key(&m)) {
                continue;
            }
            let to = place(&path)?;
            if let Some(other) = taken.insert(to.clone(), path.clone()) {
                return Err(PackError::Clash(other, path.clone(), unlaid(&to)));
            }
            drawn.push((path, to, model.clone()));
        }
    }

    // 4. Parts and meshes as they are; assemblies deepest first, each with its references
    // moved and its sub-assemblies' new revisions.
    let mut files: Vec<(PackedFile, Vec<u8>)> = Vec::new();
    let mut new_revision: HashMap<DocumentId, ContentHash> = HashMap::new();
    let mut packed_bytes: HashMap<DocumentId, Vec<u8>> = HashMap::new();
    for (id, doc) in docs.iter().filter(|(_, d)| d.kind != DocKind::Assembly) {
        new_revision.insert(*id, doc.revision.clone());
        let file = PackedFile {
            id: *id,
            kind: doc.kind,
            from: doc.from.clone(),
            to: unlaid(&doc.to),
            revision: doc.revision.clone(),
            rewritten: false,
        };
        files.push((file, read(&doc.from)?));
    }
    // Every sub-assembly before each assembly that places it, however deep or shared.
    let assemblies = post_order(first.id, &docs);
    let mut packed_root = None;
    for id in &assemblies {
        let doc = &docs[id];
        let package = open(&doc.from)?;
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
                    let to = &docs[&cid].to;
                    let bytes = packed_bytes.get(&cid).cloned().ok_or_else(|| io(to, &"not packed yet"))?;
                    let packed = AssemblyPackage::from_bytes(bytes).map_err(|e| io(to, &e))?;
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
        let revision = truth.semantic_hash();
        new_revision.insert(*id, revision.clone());
        packed_bytes.insert(*id, bytes.clone());
        let file = PackedFile {
            id: *id,
            kind: DocKind::Assembly,
            from: doc.from.clone(),
            to: unlaid(&doc.to),
            revision,
            rewritten: true,
        };
        files.push((file, bytes));
        if *id == first.id {
            packed_root = Some(unlaid(&doc.to));
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
            let out = repoint(&read(&from)?, &r).map_err(|e| io(&from, &e))?;
            let identity = identify(Cursor::new(&out)).map_err(|e| io(&to, &e))?;
            let file = PackedFile {
                id: identity.id,
                kind: DocKind::Drawing,
                from,
                to: unlaid(&to),
                revision: identity.revision,
                rewritten: true,
            };
            files.push((file, out));
        }
    }
    // The assembly last, as the report promises.
    files.sort_by_key(|(f, _)| (f.id == first.id, f.kind == DocKind::Assembly));
    Ok(Pack { root: packed_root.expect("the root is an assembly"), files })
}

/// A place in the layout folder as a path relative to the pack.
fn unlaid(path: &Path) -> PathBuf {
    path.strip_prefix(LAYOUT).map(Path::to_path_buf).unwrap_or_else(|_| path.to_path_buf())
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

fn safe_name_fragment(fragment: &str) -> bool {
    !fragment.contains(['/', '\\', ':', '\0']) && !fragment.contains("..")
}

fn check_output_path(path: &Path) -> Result<(), PackError> {
    if path.file_name().is_some()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(name) if safe_name_fragment(&name.to_string_lossy())))
    {
        return Ok(());
    }
    Err(PackError::Destination(format!("{} is not a safe path inside the pack", path.display())))
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
