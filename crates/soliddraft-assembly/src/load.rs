//! Opening an assembly without waiting for it.
//!
//! Opening runs in three phases, and only the first blocks:
//!
//! 1. **Structure.** The root's truth, then every sub-assembly's structure at its pinned
//!    revision (from the file's display copies), then the flat occurrence table. This is
//!    milliseconds to a second even for a million occurrences, because it reads no
//!    meshes and opens no other file while the copies are there.
//! 2. **Display.** Each part definition's display mesh at its pinned revision, decoded
//!    on worker threads and handed over in batches ([`LoadEvent::Meshes`]). The
//!    occurrences of a definition appear as its mesh arrives.
//! 3. **Freshness.** Each referenced document is resolved and its revision compared
//!    with the pin ([`LoadEvent::Freshness`]), for the "newer version available" marks.
//!
//! When a copy is missing, the loader falls back, in order, to: the copies held by
//! other assembly files it has opened (sub-assemblies) or one of those files itself when
//! it is the sub-assembly at the pinned revision, the referenced file when it is
//! at the pinned revision, the revision store, and finally the referenced file at its
//! current revision, which marks the definition stale. Without any of those the
//! definition fails, and the rest of the assembly still opens.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use web_time::Instant;

use soliddraft_document::resolve::{Location, Resolution, Resolver};
use soliddraft_document::{ContentHash, DocKind, DocRef, OpenDocument, RevisionStore, identify_file};

use crate::file::{AssemblyPackage, FileError};
use crate::mesh::{DisplayMesh, SharedMesh};
use crate::occurrence::{FlattenError, OccurrenceTable, label_of};
use crate::schema::Assembly;
use crate::source::DisplaySource;
use crate::store::{DefinitionIndex, DefinitionKey, DefinitionStore, Freshness};
use soliddraft_tessellation::{Level, Quality};

#[cfg(test)]
mod regression_tests;

/// Why an assembly could not be opened at all.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The root file could not be read.
    #[error("{0}")]
    File(#[from] FileError),
    /// The tree could not be flattened.
    #[error("{0}")]
    Flatten(#[from] FlattenError),
}

/// What an open may consult beyond the assembly file itself.
#[derive(Clone, Default)]
pub struct OpenOptions {
    /// Folders to search for moved documents, such as File locations' referenced
    /// documents folders.
    pub search: Vec<PathBuf>,
    /// Documents open in this session.
    pub open: Vec<OpenDocument>,
    /// Old revisions' bytes.
    pub revisions: Option<Arc<dyn RevisionStore>>,
    /// Display meshes for parts without a copy. Without one, such parts fail to load
    /// (or, from a file, read only their structure).
    pub display: Option<Arc<dyn DisplaySource>>,
    /// Worker threads for decoding meshes; 0 for one per core, at most 8.
    pub threads: usize,
    /// Resolve every reference after loading to fill in freshness.
    pub check_freshness: bool,
    /// Load on the caller's thread, a step each time an event is asked for, instead of
    /// on threads of the loader's own. The browser always loads this way: a page has one
    /// thread. The document, its meshes and its freshness come out the same either way.
    pub same_thread: bool,
}

impl OpenOptions {
    /// True when loading runs on the caller's thread: asked for, or in the browser.
    pub fn loads_here(&self) -> bool {
        self.same_thread || cfg!(target_arch = "wasm32")
    }
}

/// Where the loader looks for display copies and referenced documents.
pub(crate) struct Lookup {
    /// Assembly files whose copies are searched, the root's first.
    pub packages: Vec<AssemblyPackage>,
    pub resolver: Mutex<Resolver>,
    pub revisions: Option<Arc<dyn RevisionStore>>,
    pub display: Option<Arc<dyn DisplaySource>>,
    /// The root's file.
    pub path: Option<PathBuf>,
    pub search: Vec<PathBuf>,
    pub open: Vec<OpenDocument>,
    /// Load on the caller's thread ([`OpenOptions::loads_here`]).
    pub here: bool,
}

impl Lookup {
    pub fn new(root: &AssemblyPackage, path: Option<&Path>, options: &OpenOptions) -> Lookup {
        Lookup::with_packages(vec![root.clone()], path, options)
    }

    /// A lookup searching `packages` first.
    pub fn with_packages(packages: Vec<AssemblyPackage>, path: Option<&Path>, options: &OpenOptions) -> Lookup {
        let mut l = Lookup {
            packages,
            resolver: Mutex::new(Resolver::new(None)),
            revisions: options.revisions.clone(),
            display: options.display.clone(),
            path: path.map(Path::to_path_buf),
            search: options.search.clone(),
            open: options.open.clone(),
            here: options.loads_here(),
        };
        l.resolver = Mutex::new(l.resolver_for(path));
        l
    }

    /// A resolver for references held by the file at `referencing`.
    pub fn resolver_for(&self, referencing: Option<&Path>) -> Resolver {
        Resolver::new(referencing).with_search_folders(self.search.iter()).with_open(self.open.iter().cloned())
    }

    /// The file a reference resolves to now, with its current revision.
    fn resolve(&self, r: &DocRef) -> Option<(PathBuf, Option<ContentHash>)> {
        let resolution = self.resolver.lock().ok()?.resolve(r);
        self.found(resolution)
    }

    /// The bytes of an open document whose file cannot be read, from the revision store
    /// at the document's revision. In the browser the documents open are the files handed
    /// to the page, and the page holds their bytes there: it has no folder to read.
    fn held(&self, d: &OpenDocument) -> Option<Vec<u8>> {
        self.revisions.as_ref()?.fetch(d.id, &d.revision).ok()
    }

    /// Where a resolution found its target, with the target's revision now.
    fn found(&self, resolution: Resolution) -> Option<(PathBuf, Option<ContentHash>)> {
        let Resolution::Found(t) = resolution else { return None };
        match &t.location {
            Location::File(p) => Some((p.clone(), t.revision().cloned())),
            Location::Open(d) => {
                let path = d.path.clone()?;
                // An open document's unsaved edits are not in its file. A document held
                // in the revision store is at the revision it is held at.
                let revision = identify_file(&path).ok().map(|i| i.revision).or_else(|| {
                    self.revisions.as_ref().filter(|s| s.has(d.id, &d.revision)).map(|_| d.revision.clone())
                });
                Some((path, revision))
            }
        }
    }

    fn current(&self, r: &DocRef) -> Result<Option<(Vec<u8>, ContentHash)>, String> {
        let Ok(resolver) = self.resolver.lock() else { return Ok(None) };
        let Resolution::Found(target) = resolver.resolve(r) else { return Ok(None) };
        drop(resolver);
        let read = target.path().map(std::fs::read);
        let held = match &target.location {
            Location::Open(d) if !matches!(read, Some(Ok(_))) => self.held(d),
            _ => None,
        };
        let bytes = match (read, held) {
            (_, Some(bytes)) | (Some(Ok(bytes)), None) => bytes,
            (Some(Err(e)), None) => return Err(e.to_string()),
            (None, None) => return Ok(None),
        };
        let identity = soliddraft_document::identify(std::io::Cursor::new(&bytes)).map_err(|e| e.to_string())?;
        if Some(identity.id) != r.id || identity.kind != r.kind {
            return Ok(None);
        }
        Ok(Some((bytes, identity.revision)))
    }
}

pub(crate) fn matches_reference(bytes: &[u8], reference: &DocRef) -> bool {
    soliddraft_document::identify(std::io::Cursor::new(bytes)).is_ok_and(|identity| {
        Some(identity.id) == reference.id && identity.kind == reference.kind && identity.revision == reference.pinned
    })
}

impl Lookup {
    /// A sub-assembly's structure at the pinned revision, with a problem when it is of
    /// another revision, and the file it came from when that was not one of the
    /// lookup's own.
    pub fn fetch_structure(&self, r: &DocRef) -> Result<Fetched, String> {
        let id = r.id.ok_or("the reference has no document id")?;
        // A file the lookup already holds may be the sub-assembly itself at that revision.
        if let Some(p) = self.packages.iter().find(|p| p.id() == id && p.manifest().semantic_hash == r.pinned) {
            return Ok(Fetched { assembly: p.assembly().clone(), stale: None, package: None });
        }
        for p in &self.packages {
            if let Some(t) = p.reader().assembly_truth(id, &r.pinned).map_err(|e| e.to_string())? {
                let a = Assembly::from_truth(&t.json, t.bulk.as_deref()).map_err(|e| e.to_string())?;
                return Ok(Fetched { assembly: Arc::new(a), stale: None, package: None });
            }
        }
        let opened = |p: AssemblyPackage, stale| Fetched { assembly: p.assembly().clone(), stale, package: Some(p) };
        let found = match self.current(r) {
            Ok(Some((bytes, now))) if now == r.pinned => {
                return Ok(opened(AssemblyPackage::from_bytes(bytes).map_err(|e| e.to_string())?, None));
            }
            found => found,
        };
        if let Some(store) = &self.revisions
            && let Ok(bytes) = store.fetch(id, &r.pinned)
            && matches_reference(&bytes, r)
        {
            return Ok(opened(AssemblyPackage::from_bytes(bytes).map_err(|e| e.to_string())?, None));
        }
        match found? {
            Some((bytes, now)) => {
                let now = now.short();
                let stale =
                    format!("revision {} has no copy here; showing the current revision {now}", r.pinned.short());
                Ok(opened(AssemblyPackage::from_bytes(bytes).map_err(|e| e.to_string())?, Some(stale)))
            }
            None => Err(format!(
                "no copy of revision {} and {} cannot be found; Rebuild or repair the reference",
                r.pinned.short(),
                r.path
            )),
        }
    }

    /// A part's display mesh at the pinned revision, and a problem when it is of
    /// another revision.
    pub fn mesh(
        &self,
        r: &DocRef,
        quality: Option<&Quality>,
        readers: &mut [crate::file::CopyReader],
    ) -> Result<(DisplayMesh, Option<String>), String> {
        let id = r.id.ok_or("the reference has no document id")?;
        for reader in readers.iter_mut() {
            if let Some((m, _)) = reader.display_mesh_at(id, &r.pinned, quality).map_err(|e| e.to_string())? {
                return Ok((m, None));
            }
        }
        if r.kind == DocKind::Mesh {
            return self.reference_mesh(r);
        }
        let made = |display: &dyn DisplaySource, bytes: &[u8]| match quality {
            None => display.part_mesh(bytes),
            Some(q) => display.part_mesh_at(bytes, q),
        };
        let display = self.display.as_ref().ok_or_else(|| {
            format!("the file has no display copy of revision {}; Rebuild to make one", r.pinned.short())
        })?;
        let found = self.current(r);
        if let Ok(Some((bytes, now))) = &found
            && *now == r.pinned
        {
            return made(display.as_ref(), bytes).map(|(m, _)| (m, None));
        }
        if let Some(store) = &self.revisions
            && let Ok(bytes) = store.fetch(id, &r.pinned)
            && matches_reference(&bytes, r)
        {
            return made(display.as_ref(), &bytes).map(|(m, _)| (m, None));
        }
        match found? {
            Some((bytes, now)) => {
                let (m, _) = made(display.as_ref(), &bytes)?;
                let now = now.short();
                Ok((
                    m,
                    Some(format!("revision {} has no copy here; showing the current revision {now}", r.pinned.short())),
                ))
            }
            None => Err(format!("no copy of revision {} and {} cannot be found", r.pinned.short(), r.path)),
        }
    }
}

impl Lookup {
    /// A mesh reference body's mesh from its file, which needs no kernel: at the pinned
    /// revision, or the current one with a problem saying so.
    fn reference_mesh(&self, r: &DocRef) -> Result<(DisplayMesh, Option<String>), String> {
        let id = r.id.ok_or("the reference has no document id")?;
        if let Some(store) = &self.revisions
            && let Ok(bytes) = store.fetch(id, &r.pinned)
            && matches_reference(&bytes, r)
        {
            return crate::reference_mesh::display(&bytes).map(|(m, _)| (m, None));
        }
        match self.current(r)? {
            Some((bytes, now)) => {
                let (m, _) = crate::reference_mesh::display(&bytes)?;
                let problem = (now != r.pinned).then(|| {
                    format!(
                        "revision {} has no copy here; showing the current revision {}",
                        r.pinned.short(),
                        now.short()
                    )
                });
                Ok((m, problem))
            }
            None => Err(format!("no copy of revision {} and {} cannot be found", r.pinned.short(), r.path)),
        }
    }

    /// A part's mesh at `level` of `quality` (`None` for its own) at the pinned
    /// revision: from the copies first, then from the referenced file while it is at
    /// that revision, then from the revision store. Unlike the standard level there is
    /// no stand-in from another revision: an occurrence that cannot have the level
    /// keeps the one it has.
    pub fn level_mesh(
        &self,
        r: &DocRef,
        quality: Option<&Quality>,
        level: Level,
        readers: &mut [crate::file::CopyReader],
    ) -> Result<DisplayMesh, String> {
        let id = r.id.ok_or("the reference has no document id")?;
        let name = crate::file::level_entry(quality, level);
        for reader in readers.iter_mut() {
            if let Some(bytes) = reader.entry(id, &r.pinned, &name).map_err(|e| e.to_string())? {
                return DisplayMesh::decode(&bytes).map(|(m, _)| m).map_err(|e| e.to_string());
            }
        }
        let display = self.display.as_ref().ok_or("no display source can make other levels")?;
        let found = self.current(r);
        if let Ok(Some((bytes, now))) = &found
            && *now == r.pinned
        {
            return display.part_mesh_level(bytes, quality, level).map(|(m, _)| m);
        }
        if let Some(store) = &self.revisions
            && let Ok(bytes) = store.fetch(id, &r.pinned)
            && matches_reference(&bytes, r)
        {
            return display.part_mesh_level(&bytes, quality, level).map(|(m, _)| m);
        }
        found?;
        Err(format!("revision {} of {} is not available", r.pinned.short(), r.path))
    }
}

/// One coarse or fine mesh to load.
pub(crate) struct LevelJob {
    pub def: DefinitionIndex,
    pub reference: DocRef,
    pub quality: Option<Quality>,
    pub level: Level,
}

/// A loaded level, or why it could not be.
pub(crate) type LevelDone = (DefinitionIndex, Level, Result<SharedMesh, String>);

/// Loads coarse and fine meshes on request, one at a time, for one structure generation:
/// definition indices mean nothing after a restructure. On a thread of its own, or, when
/// the lookup loads here, one job each time the levels are taken.
pub(crate) struct LevelWorker {
    backend: Levels,
    pub generation: u64,
    pub pending: HashSet<(DefinitionIndex, Level)>,
    /// Levels that could not be loaded: not asked for again in this generation.
    pub failed: HashSet<(DefinitionIndex, Level)>,
}

enum Levels {
    Thread { jobs: Sender<LevelJob>, done: Receiver<LevelDone> },
    Here { lookup: Box<Lookup>, readers: Vec<crate::file::CopyReader>, queue: VecDeque<LevelJob> },
}

impl LevelWorker {
    pub fn spawn(lookup: Lookup, generation: u64) -> LevelWorker {
        let backend = if lookup.here {
            let readers = lookup.packages.iter().map(|p| p.reader()).collect();
            Levels::Here { lookup: Box::new(lookup), readers, queue: VecDeque::new() }
        } else {
            let (jobs, queue) = channel::<LevelJob>();
            let (done_tx, done) = channel();
            std::thread::Builder::new()
                .name("assembly levels".into())
                .spawn(move || {
                    let mut readers: Vec<_> = lookup.packages.iter().map(|p| p.reader()).collect();
                    for job in queue {
                        let mesh = lookup.level_mesh(&job.reference, job.quality.as_ref(), job.level, &mut readers);
                        if done_tx.send((job.def, job.level, mesh.map(Arc::new))).is_err() {
                            return;
                        }
                    }
                })
                .expect("the level loader thread starts");
            Levels::Thread { jobs, done }
        };
        LevelWorker { backend, generation, pending: HashSet::new(), failed: HashSet::new() }
    }

    /// Queues a job unless the same one is queued or has failed.
    pub fn request(&mut self, job: LevelJob) {
        if !self.failed.contains(&(job.def, job.level)) && self.pending.insert((job.def, job.level)) {
            match &mut self.backend {
                Levels::Thread { jobs, .. } => {
                    let _ = jobs.send(job);
                }
                Levels::Here { queue, .. } => queue.push_back(job),
            }
        }
    }

    /// The levels loaded since the last call. Loading here, this loads the next one.
    pub fn take(&mut self) -> Vec<LevelDone> {
        let out: Vec<LevelDone> = match &mut self.backend {
            Levels::Thread { done, .. } => done.try_iter().collect(),
            Levels::Here { lookup, readers, queue } => queue
                .pop_front()
                .map(|job| {
                    let mesh = lookup.level_mesh(&job.reference, job.quality.as_ref(), job.level, readers);
                    (job.def, job.level, mesh.map(Arc::new))
                })
                .into_iter()
                .collect(),
        };
        for (d, l, mesh) in &out {
            self.pending.remove(&(*d, *l));
            if mesh.is_err() {
                self.failed.insert((*d, *l));
            }
        }
        out
    }
}

/// A sub-assembly's structure, fetched.
pub(crate) struct Fetched {
    pub assembly: Arc<Assembly>,
    /// Why it is of another revision than the pinned one.
    pub stale: Option<String>,
    /// The file it was read from, when it was not one the lookup already had.
    pub package: Option<AssemblyPackage>,
}

/// An assembly's structure, read.
pub struct Structure {
    /// The definitions, sub-assemblies' structures loaded, parts unresolved.
    pub store: DefinitionStore,
    /// The occurrence table.
    pub table: OccurrenceTable,
    /// The root's definition.
    pub root: DefinitionIndex,
    /// The reference each definition was first reached by.
    pub refs: HashMap<DefinitionIndex, DocRef>,
    /// The assembly whose file holds that reference.
    pub parents: HashMap<DefinitionIndex, DefinitionIndex>,
}

/// Reads the structure under `assembly`, the truth of document `id` at `revision`.
pub(crate) fn structure(
    id: soliddraft_document::DocumentId,
    revision: ContentHash,
    assembly: Arc<Assembly>,
    lookup: &mut Lookup,
) -> Result<Structure, LoadError> {
    let mut store = DefinitionStore::new();
    let key = DefinitionKey::new(id, None, revision);
    let root = store.intern(key, DocKind::Assembly, assembly.name.clone());
    store.set_assembly(root, assembly);
    let mut refs: HashMap<DefinitionIndex, DocRef> = HashMap::new();
    let mut parents: HashMap<DefinitionIndex, DefinitionIndex> = HashMap::new();
    let mut locations = HashMap::from([(root, lookup.path.clone())]);
    // Level by level, so the sub-assemblies of one level are read on every core.
    let mut level = vec![root];
    let mut seen = HashSet::from([root]);
    while !level.is_empty() {
        let mut wanted: Vec<(DefinitionIndex, DocRef)> = Vec::new();
        for a in level.drain(..) {
            let Some(assembly) = store.get(a).assembly.clone() else { continue };
            for s in &assembly.sources {
                let Some(doc) = s.document.id else { continue };
                let key = DefinitionKey::new(doc, s.document.configuration.clone(), s.document.pinned.clone());
                let i = store.intern(key, s.document.kind, label_of(&s.document));
                let mut reference = s.document.clone();
                if !reference.path.is_empty()
                    && let Some(holder) = locations.get(&a).and_then(|path| path.as_deref())
                {
                    let target = reference.stored_path(holder);
                    reference.path = std::path::absolute(&target).unwrap_or(target).to_string_lossy().into_owned();
                }
                refs.entry(i).or_insert_with(|| reference.clone());
                parents.entry(i).or_insert(a);
                if s.document.kind == DocKind::Assembly && seen.insert(i) {
                    wanted.push((i, reference));
                }
            }
        }
        let fetched = fetch_all(lookup, &wanted);
        for ((i, reference), f) in wanted.into_iter().zip(fetched) {
            match f {
                Ok(f) => {
                    let location = lookup
                        .resolve(&reference)
                        .map(|(path, _)| path)
                        .or_else(|| (!reference.path.is_empty()).then(|| PathBuf::from(&reference.path)));
                    locations.insert(i, location);
                    store.set_assembly(i, f.assembly);
                    if let Some(p) = f.stale {
                        let d = store.get_mut(i);
                        d.state = crate::store::LoadState::Stale;
                        d.problem = Some(p);
                    }
                    lookup.packages.extend(f.package);
                    level.push(i);
                }
                Err(e) => store.fail(i, e),
            }
        }
    }
    let table = OccurrenceTable::flatten(root, &mut store)?;
    // A part shown at a quality of its own is a definition flattening added: it is
    // reached by its base definition's reference.
    for (i, d) in store.iter() {
        if d.quality.is_some()
            && let Some(base) = store.find(&d.key.at(None))
        {
            if let Some(r) = refs.get(&base).cloned() {
                refs.entry(i).or_insert(r);
            }
            if let Some(p) = parents.get(&base).copied() {
                parents.entry(i).or_insert(p);
            }
        }
    }
    Ok(Structure { store, table, root, refs, parents })
}

/// Fetches sub-assembly structures on every core, or in turn when the lookup loads here.
fn fetch_all(lookup: &Lookup, wanted: &[(DefinitionIndex, DocRef)]) -> Vec<Result<Fetched, String>> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8).min(wanted.len());
    if threads <= 1 || lookup.here {
        return wanted.iter().map(|(_, r)| lookup.fetch_structure(r)).collect();
    }
    let chunk = wanted.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let parts: Vec<_> = wanted
            .chunks(chunk)
            .map(|c| scope.spawn(move || c.iter().map(|(_, r)| lookup.fetch_structure(r)).collect::<Vec<_>>()))
            .collect();
        parts.into_iter().flat_map(|p| p.join().expect("a structure reader")).collect()
    })
}

/// A loaded display mesh with the problem that makes it stale, if any, or why it could
/// not be loaded.
pub type MeshResult = Result<(SharedMesh, Option<String>), String>;

/// Something the background loader found.
#[derive(Debug)]
pub enum LoadEvent {
    /// Display meshes, or why a definition could not be loaded. A mesh with a problem
    /// is of another revision than the pinned one (the definition is stale).
    Meshes(Vec<(DefinitionIndex, MeshResult)>),
    /// How pinned revisions compare with the documents now.
    Freshness(Vec<(DefinitionIndex, Freshness)>),
    /// Loading finished.
    Finished {
        /// How long the display phase took.
        display: Duration,
        /// How long the freshness phase took.
        freshness: Duration,
    },
}

/// The background half of an open: meshes and freshness, arriving as events.
pub struct LoadHandle {
    events: Events,
    /// Part definitions to load.
    pub total: usize,
    done: bool,
}

/// Where a load's events come from.
enum Events {
    /// The loader's threads.
    Threads(Receiver<LoadEvent>),
    /// Steps run on the caller's thread, one each time an event is asked for.
    Here(Box<Steps>),
}

impl LoadHandle {
    /// The next event if one has arrived, without waiting. Loading here, this runs the
    /// next step and answers its event.
    pub fn try_event(&mut self) -> Option<LoadEvent> {
        let e = match &mut self.events {
            Events::Threads(events) => events.try_recv().ok()?,
            Events::Here(steps) => steps.next()?,
        };
        self.done |= matches!(e, LoadEvent::Finished { .. });
        Some(e)
    }

    /// Every event that has arrived, without waiting. Loading here, this runs every
    /// step that is left.
    pub fn try_events(&mut self) -> Vec<LoadEvent> {
        std::iter::from_fn(|| self.try_event()).collect()
    }

    /// The next event, waiting for it; `None` once the loader is done.
    pub fn next_event(&mut self) -> Option<LoadEvent> {
        if self.done {
            return None;
        }
        let e = match &mut self.events {
            Events::Threads(events) => events.recv().ok()?,
            Events::Here(steps) => steps.next()?,
        };
        self.done |= matches!(e, LoadEvent::Finished { .. });
        Some(e)
    }

    /// True once every event has been handed out.
    pub fn is_done(&self) -> bool {
        self.done
    }
}

const BATCH: usize = 256;

/// How many display meshes one step makes when loading here. A part without a display
/// copy is rebuilt on a kernel, so a step makes one, and a frame waits for at most one
/// rebuild beyond its budget.
const STEP: usize = 1;

/// Starts the background phases: on a thread of their own, or, when the lookup loads
/// here, as steps the handle runs when asked for events.
pub(crate) fn spawn(
    lookup: Lookup,
    parts: Vec<(DefinitionIndex, DocRef, Option<Quality>)>,
    freshness: Freshen,
    threads: usize,
) -> LoadHandle {
    let total = parts.len();
    if lookup.here {
        return LoadHandle { events: Events::Here(Box::new(Steps::new(lookup, parts, freshness))), total, done: false };
    }
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("assembly loader".into())
        .spawn(move || run(lookup, parts, freshness, threads, tx))
        .expect("the loader thread starts");
    LoadHandle { events: Events::Threads(rx), total, done: false }
}

/// The references whose freshness to check: each definition with its reference and the
/// assembly whose file holds it, and the root's definition.
pub(crate) struct Freshen {
    pub refs: Vec<(DefinitionIndex, DocRef, DefinitionIndex)>,
    pub root: DefinitionIndex,
}

/// A reference to check: the definition, its reference, and the assembly holding it.
type Held = (DefinitionIndex, DocRef, DefinitionIndex);

impl Freshen {
    /// The sub-assemblies' references, parents before children, so their files are known
    /// when their parts are resolved; then the parts'.
    fn split(self) -> (Vec<Held>, Vec<Held>) {
        let (mut assemblies, parts): (Vec<_>, Vec<_>) =
            self.refs.into_iter().partition(|(_, r, _)| r.kind == DocKind::Assembly);
        let parent_of: HashMap<DefinitionIndex, DefinitionIndex> =
            assemblies.iter().map(|(i, _, p)| (*i, *p)).collect();
        let depth = |mut i: DefinitionIndex| {
            let mut d = 0;
            while let Some(p) = parent_of.get(&i) {
                i = *p;
                d += 1;
            }
            d
        };
        assemblies.sort_by_key(|(i, _, _)| depth(*i));
        (assemblies, parts)
    }
}

fn freshness_of(found: &Option<(PathBuf, Option<ContentHash>)>, r: &DocRef) -> Freshness {
    match found {
        None => Freshness::Missing,
        Some((_, Some(now))) if *now != r.pinned => Freshness::NewerAvailable { now: now.clone() },
        Some(_) => Freshness::Current,
    }
}

/// The sub-assemblies' freshness, each resolved from the file that holds it, in order;
/// the file each was found at goes into `path_of`.
fn assembly_freshness(
    lookup: &Lookup,
    assemblies: &[Held],
    path_of: &mut HashMap<DefinitionIndex, PathBuf>,
) -> Vec<(DefinitionIndex, Freshness)> {
    let mut resolvers: HashMap<Option<PathBuf>, Resolver> = HashMap::new();
    let mut batch = Vec::new();
    for (i, r, parent) in assemblies {
        let referencing = path_of.get(parent).cloned();
        let resolver =
            resolvers.entry(referencing.clone()).or_insert_with(|| lookup.resolver_for(referencing.as_deref()));
        let hit = lookup.found(resolver.resolve(r));
        if let Some((p, _)) = &hit {
            path_of.insert(*i, p.clone());
        }
        batch.push((*i, freshness_of(&hit, r)));
    }
    batch
}

/// Resolves parts' references from the files that hold them, each document once.
#[derive(Default)]
struct PartFreshness {
    resolvers: HashMap<Option<PathBuf>, Resolver>,
    seen: HashMap<soliddraft_document::DocumentId, Option<(PathBuf, Option<ContentHash>)>>,
}

impl PartFreshness {
    fn batch(
        &mut self,
        lookup: &Lookup,
        path_of: &HashMap<DefinitionIndex, PathBuf>,
        parts: &[Held],
    ) -> Vec<(DefinitionIndex, Freshness)> {
        let mut batch = Vec::with_capacity(parts.len());
        for (i, r, parent) in parts {
            let Some(id) = r.id else { continue };
            let hit = self.seen.entry(id).or_insert_with(|| {
                let referencing = path_of.get(parent).cloned();
                let resolver = self
                    .resolvers
                    .entry(referencing.clone())
                    .or_insert_with(|| lookup.resolver_for(referencing.as_deref()));
                lookup.found(resolver.resolve(r))
            });
            batch.push((*i, freshness_of(hit, r)));
        }
        batch
    }
}

/// Display meshes for `parts`, as [`Lookup::mesh`] makes them.
fn meshes(
    lookup: &Lookup,
    parts: &[(DefinitionIndex, DocRef, Option<Quality>)],
    readers: &mut [crate::file::CopyReader],
) -> Vec<(DefinitionIndex, MeshResult)> {
    parts.iter().map(|(i, r, q)| (*i, lookup.mesh(r, q.as_ref(), readers).map(|(m, p)| (Arc::new(m), p)))).collect()
}

fn run(
    lookup: Lookup,
    parts: Vec<(DefinitionIndex, DocRef, Option<Quality>)>,
    freshness: Freshen,
    threads: usize,
    tx: Sender<LoadEvent>,
) {
    let started = Instant::now();
    let threads = if threads == 0 {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).clamp(1, 8)
    } else {
        threads
    };
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads.min(parts.len().div_ceil(BATCH)).max(1) {
            let (tx, next, parts, lookup) = (tx.clone(), &next, &parts, &lookup);
            scope.spawn(move || {
                let mut readers: Vec<_> = lookup.packages.iter().map(|p| p.reader()).collect();
                loop {
                    let start = next.fetch_add(BATCH, Ordering::Relaxed);
                    if start >= parts.len() {
                        break;
                    }
                    let batch = meshes(lookup, &parts[start..(start + BATCH).min(parts.len())], &mut readers);
                    if tx.send(LoadEvent::Meshes(batch)).is_err() {
                        return;
                    }
                }
            });
        }
    });
    let display = started.elapsed();
    let started = Instant::now();
    // Every reference is resolved from the file that holds it: sub-assemblies first,
    // parents before children, so their files are known when their parts are resolved;
    // then the parts, on every worker. Each document is resolved once.
    let root = freshness.root;
    let (assemblies, parts) = freshness.split();
    let mut path_of: HashMap<DefinitionIndex, PathBuf> = HashMap::new();
    if let Some(p) = &lookup.path {
        path_of.insert(root, p.clone());
    }
    let batch = assembly_freshness(&lookup, &assemblies, &mut path_of);
    if !batch.is_empty() && tx.send(LoadEvent::Freshness(batch)).is_err() {
        return;
    }
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads.min(parts.len().div_ceil(BATCH)).max(1) {
            let (tx, next, parts, lookup, path_of) = (tx.clone(), &next, &parts, &lookup, &path_of);
            scope.spawn(move || {
                let mut resolved = PartFreshness::default();
                loop {
                    let start = next.fetch_add(BATCH, Ordering::Relaxed);
                    if start >= parts.len() {
                        break;
                    }
                    let batch = resolved.batch(lookup, path_of, &parts[start..(start + BATCH).min(parts.len())]);
                    if tx.send(LoadEvent::Freshness(batch)).is_err() {
                        return;
                    }
                }
            });
        }
    });
    let _ = tx.send(LoadEvent::Finished { display, freshness: started.elapsed() });
}

/// The loader's phases as steps on the caller's thread, in the order the threads run
/// them: the display meshes, then the sub-assemblies' freshness, then the parts'.
struct Steps {
    lookup: Lookup,
    parts: Vec<(DefinitionIndex, DocRef, Option<Quality>)>,
    readers: Vec<crate::file::CopyReader>,
    /// The next part to load, or to check.
    next: usize,
    phase: Phase,
    started: Instant,
    display: Duration,
    freshen: Option<Freshen>,
    checked: Vec<Held>,
    path_of: HashMap<DefinitionIndex, PathBuf>,
    resolved: PartFreshness,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Display,
    Assemblies,
    Parts,
    Done,
}

impl Steps {
    fn new(lookup: Lookup, parts: Vec<(DefinitionIndex, DocRef, Option<Quality>)>, freshen: Freshen) -> Steps {
        let readers = lookup.packages.iter().map(|p| p.reader()).collect();
        Steps {
            lookup,
            parts,
            readers,
            next: 0,
            phase: Phase::Display,
            started: Instant::now(),
            display: Duration::ZERO,
            freshen: Some(freshen),
            checked: Vec::new(),
            path_of: HashMap::new(),
            resolved: PartFreshness::default(),
        }
    }

    /// Runs the next step and answers its event; `None` once finished.
    fn next(&mut self) -> Option<LoadEvent> {
        loop {
            match self.phase {
                Phase::Display => {
                    if self.next < self.parts.len() {
                        let end = (self.next + STEP).min(self.parts.len());
                        let batch = meshes(&self.lookup, &self.parts[self.next..end], &mut self.readers);
                        self.next = end;
                        return Some(LoadEvent::Meshes(batch));
                    }
                    self.display = self.started.elapsed();
                    self.started = Instant::now();
                    self.phase = Phase::Assemblies;
                }
                Phase::Assemblies => {
                    let freshen = self.freshen.take().expect("checked once");
                    if let Some(p) = &self.lookup.path {
                        self.path_of.insert(freshen.root, p.clone());
                    }
                    let (assemblies, parts) = freshen.split();
                    self.checked = parts;
                    self.next = 0;
                    self.phase = Phase::Parts;
                    let batch = assembly_freshness(&self.lookup, &assemblies, &mut self.path_of);
                    if !batch.is_empty() {
                        return Some(LoadEvent::Freshness(batch));
                    }
                }
                Phase::Parts => {
                    if self.next < self.checked.len() {
                        let end = (self.next + BATCH).min(self.checked.len());
                        let batch = self.resolved.batch(&self.lookup, &self.path_of, &self.checked[self.next..end]);
                        self.next = end;
                        return Some(LoadEvent::Freshness(batch));
                    }
                    self.phase = Phase::Done;
                    return Some(LoadEvent::Finished { display: self.display, freshness: self.started.elapsed() });
                }
                Phase::Done => return None,
            }
        }
    }
}
