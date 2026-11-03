//! Autosave and crash recovery.
//!
//! Autosave writes the open part's and the open drawing's recovery copies
//! (`soliddraft_document::recovery`) when [`Trigger`] says one is due. Saving a
//! document deletes its copy, and so does replacing it with another (New, Open). The
//! user's own files are never written. Where the copies go depends on the platform:
//!
//! - **Desktop.** Each running app keeps a session folder in the recovery folder,
//!   `<recovery folder>/session-<pid>-<nanos>/`, and holds a lock on its `session.lock`
//!   while it runs. The bytes are serialized and written on a worker without the slow
//!   entries (display meshes, the drawing's thumbnail); the interface only hands over an
//!   immutable snapshot. Quitting deletes the session folder. At start, a session folder
//!   whose lock is free belonged to an app that ended without quitting. Its copies that
//!   are newer than their documents ([`soliddraft_document::Standing::offered`]) move
//!   into this session's `offered/` folder and are listed for restore; the others are
//!   deleted. Damaged copies move to `<recovery folder>/damaged/` and are reported once.
//!   An offer still undecided at quit stays in `offered/`, so the next start finds it.
//! - **Browser.** The copies stay in the browser's storage for the site
//!   ([`crate::documents_browser`]), under a session id the tab holds a Web Lock on. The
//!   browser has no worker thread here, so a copy is serialized between frames and
//!   stored in the background. Closing the tab keeps the copies of unsaved work: the
//!   next visit offers them, as the desktop does after a crash. Nothing is sent to the
//!   server, so two people on the site never see each other's copies.

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{Receiver, Sender, channel};
#[cfg(not(target_arch = "wasm32"))]
use std::thread::JoinHandle;
use std::time::Duration;

use soliddraft_document::recovery::{self, Record, RecoveryCopy};
use soliddraft_document::{AutosavePolicy, DocKind, DocumentId, Trigger};
use web_time::Instant;

use crate::document::Document;
use crate::drawing::document::DrawingDoc;
use crate::state::{AppAction, AppState};

#[cfg(not(target_arch = "wasm32"))]
const LOCK: &str = "session.lock";
#[cfg(not(target_arch = "wasm32"))]
const OFFERED: &str = "offered";
#[cfg(not(target_arch = "wasm32"))]
const DAMAGED: &str = "damaged";
#[cfg(not(target_arch = "wasm32"))]
const SESSION_PREFIX: &str = "session-";

enum Job {
    Part {
        snapshot: Box<soliddraft_model::file::Snapshot>,
        name: String,
        path: Option<PathBuf>,
    },
    Drawing {
        drawing: Box<soliddraft_drawing::Drawing>,
        id: DocumentId,
        path: Option<PathBuf>,
        kernel: String,
    },
    Remove(DocumentId, DocKind),
    #[cfg(not(target_arch = "wasm32"))]
    Flush(Sender<()>),
}

/// Where a job's copy goes: a session folder, or the browser's storage.
trait Shelf {
    fn put(&self, record: &Record, document: &[u8]) -> Result<(), String>;
    fn remove(&self, id: DocumentId, kind: DocKind) -> Result<(), String>;
}

/// Makes the copy a job asks for and hands it to `shelf`. Returns what went wrong, as a
/// sentence.
fn perform(job: Job, shelf: &impl Shelf) -> Option<String> {
    match job {
        Job::Part { snapshot, name, path } => snapshot
            .encode()
            .map_err(|e| e.to_string())
            .and_then(|(manifest, bytes)| {
                let record = Record::new(manifest.id, DocKind::Part, &name, path.as_deref(), manifest.semantic_hash);
                shelf.put(&record, &bytes)
            })
            .err()
            .map(|e| format!("Autosave could not copy {name}: {e}")),
        Job::Drawing { drawing, id, path, kernel } => {
            let name = &drawing.meta.title;
            let meta = soliddraft_drawing::file::FileMeta {
                id,
                ..soliddraft_drawing::file::FileMeta::new(&kernel, drawing.meta.units)
            };
            let copy = || {
                let revision = drawing.semantic_hash().map_err(|e| e.to_string())?;
                let record = Record::new(id, DocKind::Drawing, name, path.as_deref(), revision);
                let bytes =
                    soliddraft_drawing::file::to_bytes_with(&drawing, &meta, false).map_err(|e| e.to_string())?;
                shelf.put(&record, &bytes)
            };
            copy().err().map(|e| format!("Autosave could not copy {name}: {e}"))
        }
        Job::Remove(id, kind) => {
            shelf.remove(id, kind).err().map(|e| format!("Autosave could not delete a recovery copy: {e}"))
        }
        #[cfg(not(target_arch = "wasm32"))]
        Job::Flush(done) => {
            let _ = done.send(());
            None
        }
    }
}

/// One document autosave watches: which it is, the edit count last seen and whether a
/// copy of it is on disk.
#[derive(Default)]
struct Watch {
    id: Option<DocumentId>,
    edits: u64,
    trigger: Trigger,
    written: bool,
}

/// The desktop's session folder and its writer thread.
#[cfg(not(target_arch = "wasm32"))]
struct Folder {
    root: PathBuf,
    dir: PathBuf,
    _lock: File,
    jobs: Option<Sender<Job>>,
    reports: Receiver<String>,
    writer: Option<JoinHandle<()>>,
}

/// The running session's recovery copies, their writer and what it found at start.
pub struct Recovery {
    #[cfg(not(target_arch = "wasm32"))]
    folder: Folder,
    /// The browser has not answered yet with what earlier visits left.
    #[cfg(target_arch = "wasm32")]
    starting: bool,
    pub policy: AutosavePolicy,
    part: Watch,
    drawing: Watch,
    /// Copies from sessions that ended without quitting, newest first. In the browser a
    /// copy's `file` is its key in the browser's storage.
    pub offers: Vec<RecoveryCopy>,
    /// Journals from sessions that ended without quitting. Only redacted copies may be shared.
    pub journals: Vec<PathBuf>,
    /// Damaged copies found at start, as sentences.
    pub problems: Vec<String>,
    /// Copies that could not be made or written since the app last looked, as sentences.
    pub failures: Vec<String>,
    /// The Recover documents window is open.
    pub show: bool,
}

impl Recovery {
    /// Looks at the open part and drawing after a frame: counts their edits, writes a
    /// copy of each that is due and deletes the copy of one that was saved or closed.
    /// Returns how long until the next copy is due, so an idle app can wake for it.
    pub fn tick(
        &mut self,
        part: &Document,
        drawing: Option<&DrawingDoc>,
        kernel: &str,
        now: Instant,
    ) -> Option<Duration> {
        self.collect_failures();
        if let Some(error) = soliddraft_document::journal::take_failure() {
            self.failures.push(error);
        }
        let policy = self.policy;
        let mut jobs = Vec::new();
        let id = part.model().id();
        if watch(&mut self.part, DocKind::Part, Some((id, part.edits, part.dirty)), &policy, now, &mut jobs) {
            jobs.push(Job::Part {
                snapshot: Box::new(part.model().snapshot()),
                name: part.name.clone(),
                path: part.path.clone(),
            });
        }
        let seen = drawing.map(|d| (d.id, d.edits, d.dirty));
        if watch(&mut self.drawing, DocKind::Drawing, seen, &policy, now, &mut jobs)
            && let Some(d) = drawing
        {
            jobs.push(Job::Drawing {
                drawing: Box::new(d.drawing.clone()),
                id: d.id,
                path: d.path.clone(),
                kernel: kernel.to_owned(),
            });
        }
        for job in jobs {
            self.send(job);
        }
        [&self.part, &self.drawing].into_iter().filter_map(|w| w.trigger.wait(&policy, now)).min()
    }

    /// The restored document is open with `edits` edits counted; its copy is on disk.
    pub fn restored(&mut self, id: DocumentId, kind: DocKind, edits: u64) {
        let watch = if kind == DocKind::Drawing { &mut self.drawing } else { &mut self.part };
        if let Some(old) = watch.id.filter(|old| *old != id && watch.written) {
            self.send(Job::Remove(old, kind));
        }
        let watch = if kind == DocKind::Drawing { &mut self.drawing } else { &mut self.part };
        *watch = Watch { id: Some(id), edits, trigger: Trigger::default(), written: true };
    }

    /// Whether anything waits for the user: copies to restore or journals to keep.
    pub fn has_offers(&self) -> bool {
        !self.offers.is_empty() || !self.journals.is_empty()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Recovery {
    /// Starts this session's folder under `root` and takes over the folders of sessions
    /// that ended without quitting.
    pub fn start(root: &Path, policy: AutosavePolicy) -> std::io::Result<Recovery> {
        std::fs::create_dir_all(root)?;
        let (dir, lock) = claim(root)?;
        let (jobs, reports, writer) = writer(dir.clone());
        let mut recovery = Recovery {
            folder: Folder {
                root: root.to_path_buf(),
                dir,
                _lock: lock,
                jobs: Some(jobs),
                reports,
                writer: Some(writer),
            },
            policy,
            part: Watch::default(),
            drawing: Watch::default(),
            offers: Vec::new(),
            journals: soliddraft_document::journal::abandoned(&root.join("journals")),
            problems: Vec::new(),
            failures: Vec::new(),
            show: false,
        };
        recovery.adopt_ended_sessions();
        recovery.show = !recovery.offers.is_empty() || !recovery.journals.is_empty() || !recovery.problems.is_empty();
        Ok(recovery)
    }

    /// This session's folder.
    pub fn folder(&self) -> &Path {
        &self.folder.dir
    }

    /// Takes an offered copy back: returns its record and document bytes, and moves the
    /// copy into this session's folder as the document's own, so it stays protected
    /// until the restored document is saved. The caller opens the bytes and then
    /// reports the edit count with [`Recovery::restored`].
    pub fn restore(&mut self, file: &Path) -> Result<(Record, Vec<u8>), String> {
        let (record, bytes) = recovery::read(file).map_err(|e| format!("cannot restore {}: {e}", file.display()))?;
        let own = self.folder.dir.join(recovery::file_name(record.id, record.kind));
        // The writer may hold an older copy of the same document; this one wins.
        self.flush();
        let _ = std::fs::remove_file(&own);
        std::fs::rename(file, &own).map_err(|e| format!("cannot restore {}: {e}", file.display()))?;
        self.offers.retain(|c| c.file != file);
        Ok((record, bytes))
    }

    /// Deletes an offered copy the user does not want.
    pub fn discard(&mut self, file: &Path) -> Result<(), String> {
        self.offers.retain(|c| c.file != file);
        self.journals.retain(|p| p != file);
        match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("cannot discard {}: {e}", file.display()))
            }
            _ => Ok(()),
        }
    }

    /// Waits until every copy asked for so far is on disk.
    pub fn flush(&mut self) {
        let (done, wait) = channel();
        self.send(Job::Flush(done));
        let _ = wait.recv();
        self.collect_failures();
    }

    /// The app is quitting: finishes the writes, then deletes this session's copies and
    /// its folder. Offers nobody decided on stay for the next start.
    pub fn close(mut self) {
        self.folder.jobs = None;
        if let Some(writer) = self.folder.writer.take() {
            let _ = writer.join();
        }
        let dir = &self.folder.dir;
        let scan = recovery::scan(dir);
        for file in scan.copies.iter().map(|c| &c.file).chain(scan.damaged.iter().map(|d| &d.file)) {
            let _ = std::fs::remove_file(file);
        }
        let offered = dir.join(OFFERED);
        let _ = std::fs::remove_dir(&offered);
        if !offered.exists() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn send(&self, job: Job) {
        if let Some(jobs) = &self.folder.jobs {
            let _ = jobs.send(job);
        }
    }

    fn collect_failures(&mut self) {
        while let Ok(failure) = self.folder.reports.try_recv() {
            log::warn!("{failure}");
            self.failures.push(failure);
        }
    }

    /// Takes over every session folder whose app is gone, and lists what it offers.
    fn adopt_ended_sessions(&mut self) {
        let root = self.folder.root.clone();
        let offered = self.folder.dir.join(OFFERED);
        let Ok(entries) = std::fs::read_dir(&root) else { return };
        for entry in entries.filter_map(Result::ok) {
            let folder = entry.path();
            let is_session = entry.file_name().to_string_lossy().starts_with(SESSION_PREFIX);
            if !is_session || folder == self.folder.dir || !folder.is_dir() {
                continue;
            }
            let Ok(lock) = OpenOptions::new().read(true).write(true).open(folder.join(LOCK)) else {
                // A session that stopped before it made its lock left an empty folder.
                let _ = std::fs::remove_dir(&folder);
                continue;
            };
            if lock.try_lock().is_err() {
                continue; // That app is still running.
            }
            for sub in [folder.join(OFFERED), folder.clone()] {
                let scan = recovery::scan(&sub);
                for damaged in scan.damaged {
                    let name = damaged.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.problems
                        .push(format!("A recovery copy is damaged and was skipped: {name} ({})", damaged.error));
                    let kept = root.join(DAMAGED);
                    let _ = std::fs::create_dir_all(&kept);
                    let _ = std::fs::rename(&damaged.file, kept.join(name));
                }
                for copy in scan.copies {
                    if copy.standing().offered() {
                        let _ = std::fs::create_dir_all(&offered);
                        let to = offered.join(copy.file.file_name().unwrap_or_default());
                        // Two ended sessions with the same document: the newer copy wins.
                        let newer = recovery::read(&to).is_ok_and(|(r, _)| r.written_ms > copy.record.written_ms);
                        if newer || std::fs::rename(&copy.file, &to).is_err() {
                            let _ = std::fs::remove_file(&copy.file);
                        }
                    } else {
                        let _ = std::fs::remove_file(&copy.file);
                    }
                }
            }
            drop(lock);
            let _ = std::fs::remove_dir_all(&folder);
        }
        // Offers kept from an earlier start are weighed again: the file may have been saved since.
        for copy in recovery::scan(&offered).copies {
            if copy.standing().offered() {
                self.offers.push(copy);
            } else {
                let _ = std::fs::remove_file(&copy.file);
            }
        }
    }
}

/// A session folder as a shelf, for the writer thread.
#[cfg(not(target_arch = "wasm32"))]
struct FolderShelf(PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl Shelf for FolderShelf {
    fn put(&self, record: &Record, document: &[u8]) -> Result<(), String> {
        recovery::write(&self.0, record, document).map(|_| ()).map_err(|e| e.to_string())
    }

    fn remove(&self, id: DocumentId, kind: DocKind) -> Result<(), String> {
        recovery::remove(&self.0, id, kind).map_err(|e| e.to_string())
    }
}

/// Makes and locks a new session folder under `root`.
#[cfg(not(target_arch = "wasm32"))]
fn claim(root: &Path) -> std::io::Result<(PathBuf, File)> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir = root.join(format!("{SESSION_PREFIX}{}-{nanos}", std::process::id()));
        let made = std::fs::create_dir(&dir).and_then(|()| File::create(dir.join(LOCK)));
        match made {
            Ok(lock) => match lock.try_lock() {
                Ok(()) => return Ok((dir, lock)),
                // Another app starting at the same moment took it for an ended session.
                Err(_) if attempt < 5 => continue,
                Err(e) => return Err(std::io::Error::other(e.to_string())),
            },
            Err(_) if attempt < 5 => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Starts the writer thread for the session folder `dir`.
#[cfg(not(target_arch = "wasm32"))]
fn writer(dir: PathBuf) -> (Sender<Job>, Receiver<String>, JoinHandle<()>) {
    let (jobs, inbox) = channel::<Job>();
    let (report, failures) = channel();
    let thread = std::thread::Builder::new()
        .name("autosave".into())
        .spawn(move || {
            let shelf = FolderShelf(dir);
            for job in inbox {
                if let Some(failed) = perform(job, &shelf) {
                    let _ = report.send(failed);
                }
            }
        })
        .expect("the autosave thread starts");
    (jobs, failures, thread)
}

#[cfg(target_arch = "wasm32")]
impl Recovery {
    /// Starts autosave into the browser's storage for this site. What earlier visits left
    /// arrives a few frames later; the Recover documents window opens then if there is
    /// any.
    pub fn start_in_browser(policy: AutosavePolicy) -> Recovery {
        crate::documents_browser::start_recovery();
        Recovery {
            starting: true,
            policy,
            part: Watch::default(),
            drawing: Watch::default(),
            offers: Vec::new(),
            journals: Vec::new(),
            problems: Vec::new(),
            failures: Vec::new(),
            show: false,
        }
    }

    /// Takes an offered copy back: returns its record and document bytes, and keeps the
    /// copy as the document's own, so it stays protected until the restored document is
    /// saved.
    pub fn restore(&mut self, key: &Path) -> Result<(Record, Vec<u8>), String> {
        let name = key.to_string_lossy().into_owned();
        let copy = crate::documents_browser::offered(&name).ok_or("this recovery copy is no longer here")?;
        let (record, bytes) =
            recovery::decode(std::io::Cursor::new(copy)).map_err(|e| format!("cannot restore {name}: {e}"))?;
        crate::documents_browser::adopt(&name, &recovery::file_name(record.id, record.kind));
        self.offers.retain(|c| c.file != key);
        Ok((record, bytes))
    }

    /// Deletes an offered copy the user does not want.
    pub fn discard(&mut self, key: &Path) -> Result<(), String> {
        self.offers.retain(|c| c.file != key);
        crate::documents_browser::discard(&key.to_string_lossy());
        Ok(())
    }

    /// The page is closing. Its copies stay: a tab closed with unsaved work is offered at
    /// the next visit, as a crash is.
    pub fn close(self) {}

    fn send(&self, job: Job) {
        if let Some(failed) = perform(job, &crate::documents_browser::RecoveryShelf) {
            crate::documents_browser::fail(failed);
        }
    }

    fn collect_failures(&mut self) {
        if self.starting
            && let Some(found) = crate::documents_browser::take_found()
        {
            self.starting = false;
            match found {
                Ok(found) => {
                    self.offers = found.offers;
                    self.problems = found.problems;
                    self.show = !self.offers.is_empty() || !self.problems.is_empty();
                }
                Err(e) => self.failures.push(format!("Autosave is off: {e}")),
            }
        }
        for failure in crate::documents_browser::take_failures() {
            log::warn!("{failure}");
            self.failures.push(failure);
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Shelf for crate::documents_browser::RecoveryShelf {
    fn put(&self, record: &Record, document: &[u8]) -> Result<(), String> {
        let bytes = recovery::encode(record, document).map_err(|e| e.to_string())?;
        crate::documents_browser::put(&recovery::file_name(record.id, record.kind), bytes, record.written_ms);
        Ok(())
    }

    fn remove(&self, id: DocumentId, kind: DocKind) -> Result<(), String> {
        crate::documents_browser::remove(&recovery::file_name(id, kind));
        Ok(())
    }
}

/// Follows one document slot. `seen` is the document in it now: id, edit count and
/// whether it has unsaved changes. Returns true when a copy is due; a copy no longer
/// wanted is queued on `jobs`.
fn watch(
    w: &mut Watch,
    kind: DocKind,
    seen: Option<(DocumentId, u64, bool)>,
    policy: &AutosavePolicy,
    now: Instant,
    jobs: &mut Vec<Job>,
) -> bool {
    let id = seen.map(|(id, ..)| id);
    if w.id != id {
        // Another document took the slot, or it closed: the old copy is not wanted.
        if let Some(old) = w.id.filter(|_| w.written) {
            jobs.push(Job::Remove(old, kind));
        }
        *w = Watch { id, edits: seen.map_or(0, |(_, edits, _)| edits), ..Watch::default() };
        if seen.is_some_and(|(_, _, dirty)| dirty) {
            w.trigger.edited(now, 1);
        }
    }
    let Some((id, edits, dirty)) = seen else { return false };
    if !dirty {
        // Saved, or every edit undone back to the file.
        if w.written {
            jobs.push(Job::Remove(id, kind));
            w.written = false;
        }
        w.trigger.reset();
        w.edits = edits;
        return false;
    }
    if edits != w.edits {
        // A frame can carry several edits, such as a script's batch of commands.
        w.trigger.edited(now, u32::try_from(edits.abs_diff(w.edits)).unwrap_or(u32::MAX));
        w.edits = edits;
    }
    if w.trigger.due(policy, now) {
        w.trigger.reset();
        w.written = true;
        return true;
    }
    false
}

/// The Recover documents window: one row per copy found after a crash, with what it is,
/// where its file lives and when the copy was made, Restore and Discard; then the damaged
/// copies that were skipped.
pub fn window(ctx: &egui::Context, state: &mut AppState) {
    let Some(r) = state.recovery.as_mut().filter(|r| r.show) else { return };
    let mut open = true;
    let mut later = false;
    let mut actions = Vec::new();
    egui::Window::new("Recover documents").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
        if r.offers.is_empty() {
            ui.label("No documents to recover.");
        } else if cfg!(target_arch = "wasm32") {
            ui.label("This browser kept changes from an earlier visit that were not saved:");
        } else {
            ui.label("SolidDraft did not close properly. These documents have changes that were not saved:");
        }
        egui::Grid::new("recovery_offers").num_columns(5).spacing([14.0, 6.0]).striped(true).show(ui, |ui| {
            if !r.offers.is_empty() {
                for heading in ["Document", "Location", "Last copy", "", ""] {
                    ui.label(egui::RichText::new(heading).strong());
                }
                ui.end_row();
            }
            for copy in &r.offers {
                let record = &copy.record;
                ui.label(format!("{} · {}", record.name, crate::recent::kind_name(Some(record.kind))));
                ui.label(place(record));
                ui.label(format!("{} ({})", crate::recent::when_text(record.written_ms), ago(record.written())));
                if ui.button("Restore").clicked() {
                    actions.push(AppAction::RestoreRecovery(copy.file.clone()));
                }
                if ui.button("Discard").clicked() {
                    actions.push(AppAction::DiscardRecovery(copy.file.clone()));
                }
                ui.end_row();
            }
        });
        if !r.journals.is_empty() {
            ui.separator();
            ui.label("Crash report journals are available. Personal paths are replaced in the report copy.");
            for (index, journal) in r.journals.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("Session {}", index + 1));
                    if ui.button("Save crash report journal").clicked() {
                        actions.push(AppAction::ExportCrashJournal(journal.clone()));
                    }
                    if ui.button("Discard journal").clicked() {
                        actions.push(AppAction::DiscardRecovery(journal.clone()));
                    }
                });
            }
        }
        if !r.problems.is_empty() {
            ui.separator();
            for problem in &r.problems {
                ui.label(problem);
            }
        }
        ui.separator();
        later = ui.button("Later").on_hover_text("Keep the copies for the next start").clicked();
    });
    if !open || later {
        r.show = false;
        // Damaged copies are reported once; they stay in the damaged folder.
        r.problems.clear();
    }
    for action in actions {
        state.push(action);
    }
}

/// Where the document of a copy lives: its file, or that it was never saved.
fn place(record: &Record) -> String {
    match &record.path {
        None => "Never saved".to_owned(),
        // The browser saves by downloading: the name is all it knows.
        Some(path) if cfg!(target_arch = "wasm32") => {
            format!("Downloaded as {}", path.file_name().unwrap_or_default().to_string_lossy())
        }
        Some(path) => path.display().to_string(),
    }
}

/// "just now", "5 min ago", "3 h ago" or "2 days ago".
fn ago(t: web_time::SystemTime) -> String {
    let s = t.elapsed().unwrap_or_default().as_secs();
    match s {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        _ => format!("{} days ago", s / 86400),
    }
}
