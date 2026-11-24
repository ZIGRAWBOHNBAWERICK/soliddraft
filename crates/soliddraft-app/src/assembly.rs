//! Assemblies in the window: File > New assembly, Make assembly from the open part,
//! Insert component (a file picker, or part files dropped on the window), the component
//! tree, typed placement, and opening and saving `.dwasm` files. The format, loading,
//! edits and undo are `soliddraft-assembly`'s (see `docs/assemblies.md`); this module
//! holds the session the window shows and draws it with the part viewport's renderer.
//!
//! Hooks into the shell are small: `Tab::Assembly` in `app.rs`, one arm each in
//! `ribbon.rs`, the tree and property tabs and the bottom status, one `AppAction::Assembly`
//! variant and one `AssemblyState` field in `state.rs`.

pub(crate) mod tools;

use crate::app::Tab;
use crate::command_line::{Status, StatusNote};
use crate::icons::{self, Icon};
use crate::state::{AppAction, AppState, NoteKind};
use crate::theme;
use crate::tree::{RowSpec, row};
use crate::viewport::ViewportState;
use crate::viewport::assembly::{AssemblyFrame, Detail};
use egui::{Align2, Rect, Sense, Stroke, pos2, vec2};
use egui_dock::DockState;
use glam::{DMat4, DQuat, DVec3, EulerRot};
use soliddraft_assembly::{
    AssemblyDocument, DefinitionIndex, Flags, Freshness, KernelSource, LoadState, MateId, MeshBounds, OccurrenceIndex,
    OccurrencePath, OpenOptions, Opening, Placement, RebuildReport, Scene, SharedMesh,
};
use soliddraft_document::kind::ASSEMBLY_EXTENSION;
use soliddraft_document::{DocKind, DocRef};
use soliddraft_tessellation::{DeviceBudget, Level};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a frame spends taking in meshes the loader found.
const POLL_BUDGET: Duration = Duration::from_millis(4);
/// How many arrived meshes a frame hands the GPU.
const MESHES_PER_FRAME: usize = 256;
/// The gap left between the assembly and a component inserted beside it, in millimetres.
const INSERT_GAP: f64 = 20.0;

/// What the assembly tab does.
#[derive(Clone, Debug, PartialEq)]
pub enum AssemblyAction {
    /// A new, empty assembly in place of the open one.
    New,
    /// A new assembly that places the open part, which must be saved.
    FromPart,
    /// Opens an assembly file in place of the open one.
    Open(PathBuf),
    /// Places part or assembly files; `None` asks with a file dialog.
    Insert(Option<Vec<PathBuf>>),
    /// Saves to the assembly's own file, or to this one; with neither, asks.
    Save(Option<PathBuf>),
    /// Asks where to save.
    SaveAs,
    Select(Option<OccurrencePath>),
    /// Adds an occurrence to the parts chosen in the tree, or takes it out again.
    Choose(OccurrencePath),
    /// Moves an occurrence to a placement relative to the assembly that contains it.
    Place(OccurrencePath, Placement),
    Show,
    ShowPart,
    /// Opens a component's part for editing in place: the given one, or the selected one.
    EditPartInContext(Option<OccurrencePath>),
    /// Ends editing a part in context and shows the assembly.
    ExitInContext,
    Update,
    Undo,
    Redo,
    ZoomFit,
    /// Edits the appearance of a component.
    SetColour(OccurrencePath, Option<soliddraft_materials::Appearance>),
    /// Hides or shows a component.
    SetHidden(OccurrencePath, bool),
    Mate(crate::mates::MateAction),
    Form(crate::subassembly::FormAction),
    Interference(crate::interference::InterferenceAction),
    /// Configurations and the exploded view.
    Configure(crate::configurations::ConfigureAction),
    /// Pack and Go and the exports.
    Files(crate::assembly_files::FilesAction),
}

/// The workspaces of an assembly document in Edit Suite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Workspace {
    #[default]
    Assembly,
    Inspect,
    View,
}

impl Workspace {
    pub const ALL: [Self; 3] = [Self::Assembly, Self::Inspect, Self::View];
}

/// The open assembly, one being opened, and whether the assembly is the active document.
#[derive(Default)]
pub struct AssemblyState {
    /// A per-document choice; the Edit Suite tree starts open.
    pub tree_hidden: bool,
    pub workspace: Workspace,
    pub session: Option<AssemblySession>,
    /// An assembly file whose structure is being read on another thread.
    pub opening: Option<(PathBuf, Opening)>,
    pub active: bool,
    last_part: Option<soliddraft_document::OpenDocument>,
    /// The part and assembly files handed to the page this session, as the browser hands
    /// them over: uploaded to open an assembly with, inserted, or dropped. They count as
    /// open documents, their bytes held in the revision store, because the page has no
    /// folder to find them in.
    pub held: Vec<soliddraft_document::OpenDocument>,
    /// Files uploaded together that hold more than one assembly, by name and bytes, kept
    /// until the visitor chooses which assembly opens.
    pub choice: Option<Vec<(String, Vec<u8>)>>,
    /// The Mate panel's fields.
    pub mate: crate::mates::Draft,
    /// The Form subassembly panel's fields.
    pub form: crate::subassembly::Draft,
    /// The Interference panel's fields and last report.
    pub interference: crate::interference::Draft,
    /// The Configurations panel's fields.
    pub configure: crate::configurations::Draft,
    pub collision_drag: bool,
    pub dynamic_clearance: bool,
    pub clearance_mm: f64,
    /// Physical Dynamics: a guarded drag pushes the parts it meets.
    pub physical_dynamics: bool,
    /// The component whose part is being edited in context.
    pub in_context: Option<crate::in_context::InContext>,
    /// The open part's in-context references are checked at the next frame.
    pub context_check: bool,
    /// What they were last read against: they are read again only against another part
    /// or another revision of the assembly (see `in_context::follow`).
    pub context_seen: Option<crate::in_context::Seen>,
    /// A component whose part is opening to be edited in context, in the browser.
    pub entering: Option<OccurrencePath>,
    /// What the last update of the in-context references found, said at the next frame.
    pub context_note: Option<(String, NoteKind)>,
}

impl AssemblyState {
    /// True while the assembly tab is the document being worked on.
    pub fn is_active(&self) -> bool {
        self.active && self.session.is_some()
    }
}

/// An open assembly as the window shows it.
pub struct AssemblySession {
    pub doc: AssemblyDocument,
    /// The instances the renderer draws, shared with it.
    pub scene: Arc<Mutex<Scene>>,
    /// The assembly's own camera, independent of the part's.
    pub viewport: ViewportState,
    /// The selected occurrence, by path so it outlives a restructure.
    pub selected: Option<OccurrencePath>,
    /// The occurrences chosen in the tree, in the order they were chosen: a click chooses
    /// one, a Ctrl-click adds or takes away one more.
    pub chosen: Vec<OccurrencePath>,
    /// What the last click in the viewport named: the occurrence and the face's persistent
    /// name, whatever it selected.
    pub picked: Option<soliddraft_assembly::Selection>,
    /// The placement fields of the selection as typed.
    pub draft: Option<PlacementDraft>,
    /// Sub-assemblies open in the tree.
    pub expanded: HashSet<OccurrencePath>,
    /// What the scene shows as selected and hovered, by path: an undo or another edit
    /// that rereads the structure renumbers the rows.
    shown: (Option<OccurrencePath>, Option<OccurrencePath>),
    /// Meshes taken from the scene for the renderer, kept until a painted frame took them.
    outbox: Outbox,
    /// The camera frames the parts once the first meshes arrive.
    fit_pending: bool,
    /// A Mate Controller animation under way.
    pub animation: Option<ControllerAnimation>,
}

/// A Mate Controller animation holds each driver's start and end values.
/// Each frame previews a pose. The last frame applies one undo step.
pub struct ControllerAnimation {
    pub(crate) drivers: BTreeMap<MateId, (f64, f64)>,
    pub seconds: f64,
    pub started: web_time::Instant,
}

/// Active controller drivers in display order, keyed by stable mate identity.
pub(crate) fn controller_drivers(doc: &AssemblyDocument) -> impl Iterator<Item = (MateId, &str, Option<f64>)> {
    let values = soliddraft_assembly::controller_values(doc);
    doc.mate_controller().into_iter().flat_map(|c| &c.mates).filter_map(move |&id| {
        let mate = doc.assembly().known_mates().find(|m| m.id() == id)?;
        Some((id, mate.name(), values.get(mate.name()).copied()))
    })
}

/// The selection's placement as the property panel edits it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementDraft {
    pub path: OccurrencePath,
    /// Millimetres.
    pub translation: [f64; 3],
    /// Degrees about x, then y, then z.
    pub rotation: [f64; 3],
    /// The placement the fields were filled from.
    pub source: Placement,
}

impl PlacementDraft {
    pub fn of(path: OccurrencePath, p: &Placement) -> PlacementDraft {
        let (x, y, z) = p.rotation.to_euler(EulerRot::XYZ);
        let tidy = |v: f64| if v.abs() < 1e-9 { 0.0 } else { v };
        PlacementDraft {
            path,
            translation: p.translation.to_array().map(tidy),
            rotation: [x, y, z].map(|a| tidy(a.to_degrees())),
            source: *p,
        }
    }

    /// True when the fields say something other than the placement they came from.
    pub fn edited(&self) -> bool {
        let filled = PlacementDraft::of(self.path.clone(), &self.source);
        filled.translation != self.translation || filled.rotation != self.rotation
    }

    pub fn placement(&self) -> Placement {
        let [x, y, z] = self.rotation.map(f64::to_radians);
        Placement::new(DQuat::from_euler(EulerRot::XYZ, x, y, z), DVec3::from_array(self.translation))
    }
}

/// Meshes on their way to the GPU. egui can discard a pass before its paint callbacks
/// run; what a discarded pass carried goes again with the next.
#[derive(Default)]
struct Outbox {
    meshes: Vec<(DefinitionIndex, SharedMesh)>,
    levels: Vec<(DefinitionIndex, Level, SharedMesh)>,
    deviations: Vec<(DefinitionIndex, [f32; 3])>,
    /// The scene generation the indices above belong to.
    generation: u64,
    /// The pass the last batch went with.
    sent_at: Option<u64>,
}

impl AssemblySession {
    fn new(mut doc: AssemblyDocument, scene: Scene) -> AssemblySession {
        let fit_pending = true;
        let mut viewport = ViewportState::default();
        viewport.display = crate::viewport::render::DisplayMode::Shaded;
        viewport.camera.fit_framed(None);
        let scene = Arc::new(Mutex::new(scene));
        doc.poll(POLL_BUDGET);
        AssemblySession {
            doc,
            scene,
            viewport,
            selected: None,
            chosen: Vec::new(),
            picked: None,
            draft: None,
            expanded: HashSet::new(),
            shown: (None, None),
            outbox: Outbox::default(),
            fit_pending,
            animation: None,
        }
    }

    /// Shows the animation's frame for now, and applies its end once it is over. Returns
    /// whether it runs on.
    pub fn animate(&mut self) -> Result<bool, soliddraft_assembly::MateRefusal> {
        let Some(a) = self.animation.as_ref() else { return Ok(false) };
        let t = a.started.elapsed().as_secs_f64() / a.seconds.max(1e-3);
        let values: Option<Vec<f64>> = controller_drivers(&self.doc)
            .map(|(id, _, current)| {
                let &(from, to) = a.drivers.get(&id)?;
                (current == Some(from)).then_some(if t >= 1.0 { to } else { from + (to - from) * t })
            })
            .collect();
        let Some(values) = values.filter(|v| v.len() == a.drivers.len()) else {
            self.animation = None;
            self.doc.cancel_drag();
            return Ok(false);
        };
        if t >= 1.0 {
            self.animation = None;
            self.doc.cancel_drag();
            self.doc.apply_controller_values(&values)?;
            return Ok(false);
        }
        if let Err(e) = self.doc.preview_controller_values(&values) {
            self.animation = None;
            self.doc.cancel_drag();
            return Err(e);
        }
        Ok(true)
    }

    /// The name the tab and the tree show, with a dot for unsaved edits.
    pub fn title(&self) -> String {
        let name = &self.doc.assembly().name;
        if self.doc.is_dirty() { format!("{name} •") } else { name.clone() }
    }

    /// The selected occurrence's row, if it is still there.
    pub fn selected_row(&self) -> Option<OccurrenceIndex> {
        let path = self.selected.as_ref()?;
        self.doc.table().find(path).filter(|r| !self.doc.table().flags(*r).has(Flags::REMOVED))
    }

    /// The root's own components, in tree order, with their rows.
    pub fn components(&self) -> Vec<OccurrenceIndex> {
        let table = self.doc.table();
        table.children(OccurrenceIndex::ROOT).filter(|r| !table.flags(*r).has(Flags::REMOVED)).collect()
    }

    /// The box around everything loaded, in the assembly's frame.
    fn bounds(&self) -> Option<(glam::Vec3, glam::Vec3)> {
        let b = self.doc.table().bounds(OccurrenceIndex::ROOT);
        (!b.is_empty()).then(|| (b.min.as_vec3(), b.max.as_vec3()))
    }

    /// Takes in what the loader found and what edits changed, once a frame.
    fn update(&mut self) {
        self.doc.poll(POLL_BUDGET);
        self.doc.poll_drag();
        if self.doc.dragging() && self.draft.as_ref().is_some_and(|d| Some(&d.path) != self.selected.as_ref()) {
            self.doc.cancel_drag();
        }
        let contact_faces: Vec<_> = self
            .doc
            .drag_report()
            .filter(|r| r.stopped)
            .into_iter()
            .flat_map(|r| &r.faces)
            .filter_map(|f| {
                let row = self.doc.table().find(&f.path)?;
                let def = self.doc.store().get(self.doc.table().definition(row));
                let mesh = def.mesh.as_ref()?;
                let face = mesh.faces.iter().position(|face| !f.reference.is_empty() && face.name == f.reference)?;
                Some((row, face as u32))
            })
            .collect();
        let mut scene = self.scene.lock().expect("the scene lock");
        scene.update(&mut self.doc);
        scene.set_contact_faces(self.doc.table(), &contact_faces);
        let selected = self.selected_row();
        let path = selected.map(|r| self.doc.table().path(r));
        if self.shown.0 != path {
            scene.set_selection(self.doc.table(), selected.as_slice());
            self.shown.0 = path;
        }
        drop(scene);
        if self.fit_pending && self.bounds().is_some() && !self.doc.is_loading() {
            self.viewport.camera.fit_framed(self.bounds());
            self.fit_pending = false;
        }
        // The fields follow the selection, and its placement after an undo or a move.
        let local = selected.map(|r| (self.doc.table().path(r), self.doc.table().local(r)));
        self.draft = match (local, self.draft.take()) {
            (Some((path, p)), Some(d)) if d.path == path && (d.source == p || self.doc.dragging()) => Some(d),
            (Some((path, p)), _) => Some(PlacementDraft::of(path, &p)),
            (None, _) => None,
        };
    }

    /// What this pass hands the renderer: the meshes that arrived, and again those a
    /// discarded pass carried.
    fn frame(&mut self, ctx: &egui::Context) -> AssemblyFrame {
        let pass = ctx.cumulative_pass_nr();
        let resend = ctx.current_pass_index() > 0 && self.outbox.sent_at.is_some_and(|p| p + 1 == pass);
        let (meshes, levels, generation) = {
            let mut scene = self.scene.lock().expect("the scene lock");
            (scene.take_new_meshes(MESHES_PER_FRAME), scene.take_new_levels(), scene.generation())
        };
        // A batch from before the scene was rebuilt names definitions by old numbers; the
        // rebuild queued every mesh again under the new ones.
        if !resend || self.outbox.generation != generation {
            self.outbox.meshes.clear();
            self.outbox.levels.clear();
            self.outbox.deviations.clear();
            self.outbox.generation = generation;
        }
        let store = self.doc.store();
        for d in meshes {
            if let Some(m) = store.get(d).mesh.clone() {
                self.outbox.deviations.push((d, store.get(d).deviations()));
                self.outbox.meshes.push((d, m));
            }
        }
        for (d, l) in levels {
            if let Some(m) = store.get(d).level_mesh(l).cloned() {
                self.outbox.levels.push((d, l, m));
            }
        }
        if !self.outbox.meshes.is_empty() || !self.outbox.levels.is_empty() {
            ctx.request_repaint();
        }
        self.outbox.sent_at = Some(pass);
        AssemblyFrame {
            scene: self.scene.clone(),
            generation,
            meshes: self.outbox.meshes.clone(),
            // The viewport sets the camera.
            view_proj: DMat4::IDENTITY,
            eye: DVec3::ZERO,
            detail: Detail::default(),
            levels: self.outbox.levels.clone(),
            deviations: self.outbox.deviations.clone(),
            budget: DeviceBudget::default(),
            display: self.doc.assembly().viewport_state().map_or(self.viewport.display, |s| s.mode),
        }
    }
}

/// What opening and inserting may consult: the referenced documents folders, the
/// documents open, the revisions this session has seen, and the part's kernel for parts
/// without display data.
fn options(state: &AppState) -> OpenOptions {
    OpenOptions {
        search: crate::settings::file_locations().search_folders().to_vec(),
        open: documents(state),
        revisions: Some(state.revisions.clone()),
        display: Some(Arc::new(KernelSource::new(state.doc.kernel().clone()))),
        check_freshness: true,
        ..OpenOptions::default()
    }
}

fn open_part(state: &AppState) -> soliddraft_document::OpenDocument {
    soliddraft_document::OpenDocument {
        id: state.doc.model().id(),
        kind: DocKind::Part,
        path: state.doc.path.clone(),
        revision: crate::drawing::part::digest(&state.doc),
    }
}

/// The documents open in this session: the part, then the files handed to the page. The
/// part comes first, so its unsaved edits count over a file held at an older revision.
fn documents(state: &AppState) -> Vec<soliddraft_document::OpenDocument> {
    std::iter::once(open_part(state)).chain(state.assembly.held.iter().cloned()).collect()
}

pub(crate) fn refresh(state: &mut AppState) {
    let part = open_part(state);
    let open = documents(state);
    if let Some(session) = state.assembly.session.as_mut() {
        session.doc.refresh_references(open);
    }
    state.assembly.last_part = Some(part);
}

/// Holds a part or assembly file handed to the page as an open document, its bytes in the
/// revision store, in place of an earlier one of the same document.
fn hold(state: &mut AppState, name: &str, bytes: &[u8]) -> Result<soliddraft_document::Identity, String> {
    let identity = soliddraft_document::identify(std::io::Cursor::new(bytes)).map_err(|e| format!("{name}: {e}"))?;
    if !matches!(identity.kind, DocKind::Part | DocKind::Assembly) {
        return Err(format!("{name}: a {} cannot be placed in an assembly", identity.kind));
    }
    state.revisions.record(identity.id, &identity.revision, bytes);
    state.assembly.held.retain(|d| d.id != identity.id);
    state.assembly.held.push(soliddraft_document::OpenDocument {
        id: identity.id,
        kind: identity.kind,
        path: Some(PathBuf::from(name)),
        revision: identity.revision.clone(),
    });
    Ok(identity)
}

/// Holds the part and assembly files handed over with an assembly to open, so it finds
/// them. Other files are left out, and the note says so.
pub fn hold_all(state: &mut AppState, files: &[(String, Vec<u8>)]) {
    let refused: Vec<String> = files.iter().filter_map(|(name, bytes)| hold(state, name, bytes).err()).collect();
    if let Some(first) = refused.first() {
        state.note(first.clone(), NoteKind::Error);
    }
    refresh(state);
}

fn is_assembly(name: &str) -> bool {
    DocKind::of_path(std::path::Path::new(name)) == Some(DocKind::Assembly)
}

/// Opens uploaded files that hold assemblies. With one assembly it opens, the other files
/// held as the documents it places. With more than one, nothing opens yet: a parent
/// and the child assembly it places can arrive in either order, so the visitor chooses
/// which one opens, in the window [`show_choice`] draws.
pub fn open_uploaded(state: &mut AppState, mut files: Vec<(String, Vec<u8>)>) {
    let assemblies: Vec<usize> = (0..files.len()).filter(|&i| is_assembly(&files[i].0)).collect();
    match assemblies[..] {
        [] => {}
        [only] => {
            let (name, bytes) = files.remove(only);
            hold_all(state, &files);
            open_bytes(state, &name, bytes);
        }
        _ => state.assembly.choice = Some(files),
    }
}

/// Opens the uploaded assembly called `name` that the visitor chose, holding the other
/// files of its upload, the other assemblies among them, as documents it may place.
pub fn open_choice(state: &mut AppState, name: &str) {
    let Some(mut files) = state.assembly.choice.take() else { return };
    let Some(at) = files.iter().position(|(n, _)| n == name && is_assembly(n)) else {
        state.assembly.choice = Some(files);
        return;
    };
    let (name, bytes) = files.remove(at);
    hold_all(state, &files);
    open_bytes(state, &name, bytes);
}

/// The window that asks which of several uploaded assemblies opens. Closing it opens
/// none of them.
pub fn show_choice(ctx: &egui::Context, state: &mut AppState) {
    let Some(files) = &state.assembly.choice else { return };
    let names: Vec<String> = files.iter().map(|(n, _)| n.clone()).filter(|n| is_assembly(n)).collect();
    let mut open = true;
    let mut chosen = None;
    egui::Window::new("Open assembly").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
        ui.label("These files hold more than one assembly. Choose the one to open.");
        ui.label("The other files stay available as the parts and subassemblies it places.");
        for name in &names {
            let button = ui.button(name);
            #[cfg(target_arch = "wasm32")]
            crate::web::show_control(format!("Open assembly: {name}"), button.rect);
            if button.clicked() {
                chosen = Some(name.clone());
            }
        }
    });
    if let Some(name) = chosen {
        open_choice(state, &name);
    } else if !open {
        state.assembly.choice = None;
        state.note("Opened nothing; open the files again to choose an assembly", NoteKind::Info);
    }
}

/// Opens an assembly from the bytes of its file, called `name`, as the browser hands it
/// over. Its parts come from the documents held this session, or show as the copies the
/// file keeps.
pub fn open_bytes(state: &mut AppState, name: &str, bytes: Vec<u8>) {
    #[cfg(target_arch = "wasm32")]
    crate::documents_browser::remember(name, DocKind::Assembly, &bytes);
    state.note(format!("Opening {name}…"), NoteKind::Info);
    // Kept for Pack and Go, which packs the file as it was opened: the page has no folder.
    if let Ok(identity) = soliddraft_document::identify(std::io::Cursor::new(&bytes)) {
        state.revisions.record(identity.id, &identity.revision, &bytes);
    }
    let path = PathBuf::from(name);
    let opening = AssemblyDocument::open_bytes_in_background(bytes, Some(path.clone()), options(state));
    state.assembly.opening = Some((path, opening));
}

/// Places files handed to the page, by name and bytes, in the open assembly.
pub fn insert_bytes(state: &mut AppState, files: Vec<(String, Vec<u8>)>) {
    if state.assembly.session.is_none() {
        return state.note("Open or start an assembly to insert parts in", NoteKind::Info);
    }
    insert(state, files.into_iter().map(|(name, bytes)| Placing::Bytes(name, bytes)).collect());
}

pub(crate) fn warning(def: &soliddraft_assembly::Definition) -> Option<String> {
    match (&def.state, &def.freshness) {
        (LoadState::Stale, _) => def.problem.clone(),
        (_, Freshness::NewerAvailable { .. }) => Some("The part has changed; Update to use it".into()),
        (_, Freshness::Missing) => Some("The file cannot be found; the assembly shows its copy".into()),
        _ => None,
    }
}

fn from_part(state: &mut AppState) -> Result<(), String> {
    const SAVE_FIRST: &str = "Save the part before making an assembly of it";
    if state.doc.dirty {
        return Err(SAVE_FIRST.into());
    }
    // The browser places the part from its bytes: the page has no file to read it from.
    let part = if cfg!(target_arch = "wasm32") {
        let bytes = state.doc.model().to_bytes().map_err(|e| e.to_string())?;
        let name = format!("{}.{}", state.doc.name, crate::file::EXTENSION);
        hold(state, &name, &bytes)?;
        Placing::Bytes(name, bytes)
    } else {
        Placing::File(state.doc.path.clone().ok_or(SAVE_FIRST)?)
    };
    let mut session = new_session(state, format!("{} assembly", state.doc.name));
    session.selected = Some(place(&mut session, &*state.revisions, &part, None)?);
    session.doc.finish_loading();
    install(state, session);
    refresh(state);
    Ok(())
}

/// Runs Update. A refusal comes back as the report that says why.
fn update(state: &mut AppState) -> Result<RebuildReport, Box<RebuildReport>> {
    update_sources(state, None)
}

/// Runs Update for the documents in `only`, or for every source; the others keep the
/// revisions they show.
fn update_sources(
    state: &mut AppState,
    only: Option<&[soliddraft_document::DocumentId]>,
) -> Result<RebuildReport, Box<RebuildReport>> {
    refresh(state);
    let Some(session) = state.assembly.session.as_mut() else {
        let refusal = Some("no assembly is open".into());
        return Err(Box::new(RebuildReport { refusal, ..RebuildReport::default() }));
    };
    let report = match only {
        Some(ids) => session.doc.update_documents(ids),
        None => session.doc.rebuild(),
    };
    if report.refusal.is_some() {
        return Err(Box::new(report));
    }
    session.update();
    refresh(state);
    Ok(report)
}

/// Runs once a frame before the panels: tracks which document is active, takes an
/// assembly that finished opening, and brings the open one up to date.
pub fn frame(state: &mut AppState, dock: &mut DockState<Tab>, ctx: &egui::Context) {
    if let Some((_, tab)) = dock.find_active_focused() {
        match tab {
            Tab::Assembly => {
                state.assembly.active = true;
                state.drawing.active = false;
            }
            Tab::Viewport | Tab::Drawing => state.assembly.active = false,
        }
    }
    if dock.find_tab(&Tab::Assembly).is_none() {
        state.assembly.active = false;
    }
    let opened = state.assembly.opening.as_ref().map(|(path, opening)| (path.clone(), opening.try_take()));
    match opened {
        Some((path, Some(Ok((doc, scene))))) => {
            state.assembly.opening = None;
            let n = doc.assembly().components.len();
            show(state, dock, AssemblySession::new(doc, scene));
            crate::file::remember(&path);
            state.recent = crate::file::recent();
            state.note(format!("Opened {} · {n} component{}", path.display(), plural(n)), NoteKind::Ok);
        }
        Some((path, Some(Err(e)))) => {
            state.assembly.opening = None;
            state.note(format!("{}: {e}", path.display()), NoteKind::Error);
        }
        Some((_, None)) => ctx.request_repaint_after(Duration::from_millis(16)),
        None => {}
    }
    let mut loaded = false;
    if let Some(s) = state.assembly.session.as_mut() {
        loaded = s.doc.is_loading();
        s.update();
        if s.doc.is_loading() || s.doc.drag_pending() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
    let part = open_part(state);
    if loaded || state.assembly.last_part.as_ref() != Some(&part) {
        // Another part opened: its in-context references are read afresh. Edits of the
        // same part do not ask, so undoing an update of its references stays undone.
        if state.assembly.last_part.as_ref().is_none_or(|p| p.id != part.id) {
            state.assembly.context_check = true;
        }
        refresh(state);
    }
    crate::in_context::frame(state);
    if let Some((text, kind)) = state.assembly.context_note.take() {
        state.note(text, kind);
    }
    if state.assembly.context_check {
        crate::in_context::follow(state);
    }
    #[cfg(target_arch = "wasm32")]
    crate::web::show_assembly(shown(state));
}

/// One component as [`shown`] lists it.
fn shown_component(s: &AssemblySession, r: OccurrenceIndex) -> serde_json::Value {
    let table = s.doc.table();
    let def = s.doc.store().get(table.definition(r));
    let local = table.local(r);
    let freshness = match &def.freshness {
        Freshness::Unchecked => "unchecked",
        Freshness::Current => "current",
        Freshness::NewerAvailable { .. } => "newer",
        Freshness::Missing => "missing",
    };
    let inside: Vec<serde_json::Value> =
        table.children(r).filter(|c| !table.flags(*c).has(Flags::REMOVED)).map(|c| shown_component(s, c)).collect();
    serde_json::json!({
        "path": table.path(r).to_string(),
        "label": s.doc.label(r),
        "state": format!("{:?}", def.state),
        "freshness": freshness,
        "warning": warning(def),
        "problem": def.problem,
        "translation": local.translation.to_array(),
        "rotation": local.rotation.to_array(),
        "components": inside,
    })
}

/// The open assembly as the window shows it, as JSON, for browser checks: `{"open": false}`
/// with nothing open, else its `name`, whether it is `active`, `dirty`, `opening` another
/// or `loading`, each of the root's `components` (`path`, `label`, `state` and
/// `freshness` of what it places, a `warning` or `problem`, `translation` in millimetres
/// and `rotation` as `[x, y, z, w]`, and a subassembly's own `components` in the same
/// form), the `selected` component's path, the number of `mates` and their
/// `mate_names`, the `undo` step's label and depth, the window's last `note` (`text`
/// and `kind`), the Mate panel's picks and error, the `file` it came from, the names of
/// the files `held` for it, the assemblies of an upload it is `choosing` between, whether
/// the Configurations panel is open (`configure_open`), the `configurations` by name and
/// the `active_configuration`, how many `explode_steps` it has and whether it is shown
/// `exploded` (a component's `translation` is then where the exploded view shows it).
pub fn shown(state: &AppState) -> serde_json::Value {
    let held: Vec<String> =
        state.assembly.held.iter().filter_map(|d| d.path.as_ref().map(|p| p.display().to_string())).collect();
    let opening = state.assembly.opening.is_some();
    let choosing: Vec<&str> =
        state.assembly.choice.iter().flatten().map(|(n, _)| n.as_str()).filter(|n| is_assembly(n)).collect();
    let Some(s) = state.assembly.session.as_ref() else {
        return serde_json::json!({ "open": false, "opening": opening, "held": held, "choosing": choosing });
    };
    let components: Vec<serde_json::Value> = s.components().into_iter().map(|r| shown_component(s, r)).collect();
    serde_json::json!({
        "open": true,
        "active": state.assembly.is_active(),
        "name": s.doc.assembly().name,
        "file": s.doc.path().map(|p| p.display().to_string()),
        "dirty": s.doc.is_dirty(),
        "opening": opening,
        "loading": s.doc.is_loading(),
        "components": components,
        "selected": s.selected.as_ref().map(|p| p.to_string()),
        "mates": s.doc.assembly().mates.len(),
        "controller": {
            "mates": s.doc.controller_mates(),
            "positions": s.doc.mate_controller().map(|c| c.positions.iter().map(|p| p.name.clone()).collect::<Vec<_>>()).unwrap_or_default(),
            "animating": s.animation.is_some(),
        },
        "mate_names": s.doc.assembly().mates.iter().map(|m| match m {
            soliddraft_assembly::MateEntry::Known(mate) => mate.name(),
            soliddraft_assembly::MateEntry::Future(kept) => kept["name"].as_str().unwrap_or_default(),
        }).collect::<Vec<_>>(),
        "undo": { "label": s.doc.undo_label(), "depth": s.doc.undo_depth() },
        "note": state.note.as_ref().map(|n| serde_json::json!({ "text": n.text, "kind": format!("{:?}", n.kind) })),
        "mate_panel": {
            "open": state.assembly.mate.open,
            "a": state.assembly.mate.a.as_ref().map(|p| serde_json::json!({ "component": p.component.0, "face": p.face.label })),
            "b": state.assembly.mate.b.as_ref().map(|p| serde_json::json!({ "component": p.component.0, "face": p.face.label })),
            "error": state.assembly.mate.error,
        },
        "held": held,
        "choosing": choosing,
        "configure_open": state.assembly.configure.open,
        "configurations": s.doc.assembly().configurations.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        "active_configuration": s.doc.assembly().active_configuration().map(|c| c.name.clone()),
        "explode_steps": s.doc.assembly().explode.len(),
        "exploded": s.doc.is_exploded(),
        "in_context": crate::in_context::shown(state),
    })
}

/// Files dropped on the window: placed in the active assembly, or else the first one
/// that opens is opened.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn dropped(state: &mut AppState, files: Vec<PathBuf>) {
    if state.assembly.is_active() {
        let placeable: Vec<PathBuf> = files
            .into_iter()
            .filter(|p| matches!(DocKind::of_path(p), Some(DocKind::Part | DocKind::Assembly)))
            .collect();
        if placeable.is_empty() {
            state.note("Drop part or assembly files to insert them", NoteKind::Info);
        } else {
            state.push(AppAction::Assembly(AssemblyAction::Insert(Some(placeable))));
        }
    } else if let Some(p) = files.into_iter().find(|p| DocKind::of_path(p).is_some()) {
        state.push(AppAction::Open(Some(p)));
    }
}

/// Puts a session in place of the open assembly and shows its tab.
fn show(state: &mut AppState, dock: &mut DockState<Tab>, session: AssemblySession) {
    install(state, session);
    show_tab(dock);
}

/// Puts an opened assembly in place of the open one and makes it the active document, as
/// File > Open does once the assembly is read.
pub fn replace(state: &mut AppState, doc: AssemblyDocument, scene: Scene) {
    install(state, AssemblySession::new(doc, scene));
}

/// Puts a session in place of the open assembly. A Form subassembly panel belongs to the
/// assembly it was opened on, so it closes.
fn install(state: &mut AppState, mut session: AssemblySession) {
    state.assembly.workspace = Workspace::Assembly;
    state.assembly.tree_hidden = false;

    session.viewport.scene = Some(state.viewport.scene.unwrap_or_else(|| crate::settings::view().scene));
    session.viewport.references = state.viewport.references;

    state.assembly.opening = None;
    state.assembly.form = crate::subassembly::Draft::default();
    state.assembly.last_part = None;
    state.assembly.mate.clear_controller_values();
    state.assembly.in_context = None;
    state.assembly.session = Some(session);
    state.assembly.active = true;
    state.drawing.active = false;
}

/// Opens the assembly tab beside the part and makes it the active document.
pub(crate) fn show_tab(dock: &mut DockState<Tab>) {
    if let Some(path) = dock.find_tab(&Tab::Assembly) {
        let _ = dock.set_active_tab(path);
        dock.set_focused_node_and_surface(egui_dock::NodePath { surface: path.surface, node: path.node });
        return;
    }
    if let Some(path) = dock.find_tab(&Tab::Viewport) {
        dock.set_focused_node_and_surface(egui_dock::NodePath { surface: path.surface, node: path.node });
    }
    dock.push_to_focused_leaf(Tab::Assembly);
}

/// The name a new assembly takes: Assembly1, or the next number not open.
fn new_name(state: &AppState) -> String {
    match state.assembly.session.as_ref().map(|s| s.doc.assembly().name.clone()) {
        Some(open) if open.starts_with("Assembly") => {
            let n = open.trim_start_matches("Assembly").parse::<u32>().unwrap_or(1);
            format!("Assembly{}", n + 1)
        }
        _ => "Assembly1".to_owned(),
    }
}

fn new_session(state: &AppState, name: String) -> AssemblySession {
    let mut doc = AssemblyDocument::new_with(name, options(state));
    let scene = Scene::new(&mut doc);
    AssemblySession::new(doc, scene)
}

pub fn apply(state: &mut AppState, dock: &mut DockState<Tab>, action: AssemblyAction) {
    // Any assembly change may move a face an in-context reference follows.
    state.assembly.context_check = true;
    match action {
        AssemblyAction::New => {
            let session = new_session(state, new_name(state));
            let name = session.doc.assembly().name.clone();
            show(state, dock, session);
            state.note(format!("New assembly {name} · insert a part to start"), NoteKind::Info);
        }
        AssemblyAction::FromPart => match from_part(state) {
            Ok(()) => show_tab(dock),
            Err(e) => state.note(e, NoteKind::Info),
        },
        AssemblyAction::SetColour(path, appearance) => {
            if let Some(s) = state.assembly.session.as_mut()
                && let Err(error) = s.doc.set_colour(&[path], appearance)
            {
                state.note(error.to_string(), crate::state::NoteKind::Error);
            }
        }
        AssemblyAction::SetHidden(path, hidden) => {
            if let Some(s) = state.assembly.session.as_mut()
                && let Err(error) = s.doc.set_hidden(&[path], hidden)
            {
                state.note(error.to_string(), crate::state::NoteKind::Error);
            }
        }
        AssemblyAction::Show => {
            if state.assembly.session.is_some() {
                state.assembly.active = true;
                state.drawing.active = false;
                show_tab(dock);
            }
        }
        AssemblyAction::ShowPart => {
            state.assembly.active = false;
            state.drawing.active = false;
            if let Some(path) = dock.find_tab(&Tab::Viewport) {
                let _ = dock.set_active_tab(path);
                dock.set_focused_node_and_surface(egui_dock::NodePath { surface: path.surface, node: path.node });
            }
        }
        // The window opens the part first (see app::dispatch), then starts the editing.
        AssemblyAction::EditPartInContext(_) => {}
        AssemblyAction::ExitInContext => {
            if let Some(path) = crate::in_context::leave(state) {
                let newer = state.assembly.session.as_ref().and_then(|s| {
                    let r = s.doc.table().find(&path)?;
                    let def = s.doc.table().definition(r);
                    let fresh = matches!(s.doc.store().get(def).freshness, Freshness::NewerAvailable { .. });
                    s.doc.reference(def).and_then(|r| r.id).filter(|_| fresh)
                });
                // Saved edits come into the assembly at once, as SolidWorks shows them; the
                // other components keep the revisions they show.
                if let Some(id) = newer.filter(|_| !state.doc.dirty) {
                    match update_sources(state, Some(&[id])) {
                        Ok(_) => state.note("Updated the assembly with the edited part", NoteKind::Ok),
                        Err(report) => state.note(report.refusal.unwrap_or_default(), NoteKind::Error),
                    }
                }
            }
            if state.assembly.session.is_some() {
                state.assembly.active = true;
                state.drawing.active = false;
                show_tab(dock);
            }
        }
        AssemblyAction::Update => match update(state) {
            Ok(report) => state.note(format!("Updated {} sources", report.updated()), NoteKind::Ok),
            Err(report) => {
                if let Some(r) = report.mate_refusal.clone().filter(|r| !r.mates.is_empty() || !r.free.is_empty()) {
                    state.assembly.mate.conflict = Some(r);
                }
                state.note(report.refusal.unwrap_or_default(), NoteKind::Error)
            }
        },
        AssemblyAction::Mate(action) => crate::mates::apply(state, action),
        AssemblyAction::Form(action) => crate::subassembly::apply(state, action),
        AssemblyAction::Interference(action) => crate::interference::apply(state, action),
        AssemblyAction::Configure(action) => crate::configurations::apply(state, action),
        AssemblyAction::Files(action) => {
            let drawing = action == crate::assembly_files::FilesAction::MakeDrawing;
            crate::assembly_files::apply(state, action);
            if drawing && state.drawing.active {
                crate::drawing::show_drawing_tab(dock);
            }
        }
        AssemblyAction::Open(path) => {
            state.note(format!("Opening {}…", path.display()), NoteKind::Info);
            let opening = AssemblyDocument::open_in_background(&path, options(state));
            state.assembly.opening = Some((path, opening));
        }
        AssemblyAction::Insert(files) => {
            let Some(files) = files.or_else(|| {
                crate::platform::open_paths(
                    "Insert component",
                    "SolidDraft parts and assemblies",
                    &[crate::file::EXTENSION, ASSEMBLY_EXTENSION],
                )
            }) else {
                return;
            };
            insert(state, files.into_iter().map(Placing::File).collect());
        }
        AssemblyAction::Save(path) => save(state, path),
        AssemblyAction::SaveAs => {
            let Some(s) = state.assembly.session.as_ref() else { return };
            let name = format!("{}.{ASSEMBLY_EXTENSION}", s.doc.assembly().name);
            if let Some(path) =
                crate::platform::save_path("Save assembly", "SolidDraft assembly", ASSEMBLY_EXTENSION, &name)
            {
                save(state, Some(path));
            }
        }
        AssemblyAction::Select(path) => {
            if let Some(s) = state.assembly.session.as_mut() {
                s.chosen = path.iter().cloned().collect();
                s.selected = path;
            }
        }
        AssemblyAction::Choose(path) => {
            if let Some(s) = state.assembly.session.as_mut() {
                match s.chosen.iter().position(|p| *p == path) {
                    Some(i) => {
                        s.chosen.remove(i);
                    }
                    None => s.chosen.push(path.clone()),
                }
                s.selected = Some(path);
            }
        }
        AssemblyAction::Place(path, placement) => {
            let Some(s) = state.assembly.session.as_mut() else { return };
            let clearance = state.assembly.dynamic_clearance.then_some(state.assembly.clearance_mm);
            let begin = if state.assembly.physical_dynamics {
                s.doc.begin_dynamic_drag(path, clearance)
            } else if state.assembly.collision_drag || state.assembly.dynamic_clearance {
                s.doc.begin_guarded_drag(path, clearance)
            } else {
                s.doc.begin_drag(path)
            };
            let result = begin
                .map_err(|e| e.to_string())
                .and_then(|_| s.doc.preview_drag(placement).map_err(|e| e.to_string()))
                .and_then(|_| s.doc.commit_drag().map_err(|e| e.to_string()));
            if result.is_err() {
                s.doc.cancel_drag();
            }
            match result {
                Ok(()) => state.note("Moved", NoteKind::Ok),
                Err(e) => state.note(e.to_string(), NoteKind::Error),
            }
        }
        AssemblyAction::Undo | AssemblyAction::Redo => {
            let Some(s) = state.assembly.session.as_mut() else { return };
            let undo = action == AssemblyAction::Undo;
            let label = if undo { s.doc.undo_label() } else { s.doc.redo_label() }.map(str::to_owned);
            let done = if undo { s.doc.undo() } else { s.doc.redo() };
            match (done, label) {
                (true, Some(l)) => state.note(format!("{} {l}", if undo { "Undid" } else { "Redid" }), NoteKind::Info),
                _ => state.note(if undo { "Nothing to undo" } else { "Nothing to redo" }, NoteKind::Info),
            }
        }
        AssemblyAction::ZoomFit => {
            if let Some(s) = state.assembly.session.as_mut() {
                let bounds = s.bounds();
                s.viewport.camera.fit_framed(bounds);
            }
        }
    }
}

/// A file to place: where it is, or, handed to the page, its name and bytes.
enum Placing {
    File(PathBuf),
    Bytes(String, Vec<u8>),
}

impl Placing {
    fn path(&self) -> &Path {
        match self {
            Placing::File(path) => path,
            Placing::Bytes(name, _) => Path::new(name),
        }
    }
}

/// Places each file in the open assembly, pinned at its revision now. The first
/// component goes at the origin and each later one beside what is there, along x. A file
/// handed to the page is held first, so the assembly finds it again.
fn insert(state: &mut AppState, files: Vec<Placing>) {
    let mut problems = Vec::new();
    let mut held = Vec::new();
    for file in files {
        match &file {
            Placing::Bytes(name, bytes) => match hold(state, name, bytes) {
                Ok(_) => held.push(file),
                Err(e) => problems.push(e),
            },
            Placing::File(_) => held.push(file),
        }
    }
    refresh(state);
    let revisions = state.revisions.clone();
    let Some(s) = state.assembly.session.as_mut() else { return };
    let mut placed = Vec::new();
    let files = held;
    for file in &files {
        match place(s, &*revisions, file, None) {
            Ok(path) => placed.push(path),
            Err(e) => problems.push(format!("{}: {}", file.path().display(), readable(&e))),
        }
    }
    if let Some(last) = placed.last() {
        s.selected = Some(last.clone());
        s.fit_pending = true;
    }
    match (placed.len(), problems.first()) {
        (_, Some(p)) => state.note(p.clone(), NoteKind::Error),
        (1, None) => {
            let name = files[0].path().file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            state.note(format!("Inserted {name}"), NoteKind::Ok)
        }
        (n, None) => state.note(format!("Inserted {n} components"), NoteKind::Ok),
    }
}

/// Places one file: reads it for its id and revision, hands its bytes to the revision
/// store so an assembly with no folder yet can show it, and places it at `placement`, or
/// beside what is there. A file handed to the page is referenced by its name.
fn place(
    s: &mut AssemblySession,
    revisions: &dyn soliddraft_document::RevisionStore,
    file: &Placing,
    placement: Option<Placement>,
) -> Result<OccurrencePath, String> {
    let (bytes, absolute) = match file {
        Placing::File(path) => {
            (std::fs::read(path).map_err(|e| e.to_string())?, std::path::absolute(path).map_err(|e| e.to_string())?)
        }
        Placing::Bytes(name, bytes) => (bytes.clone(), PathBuf::from(name)),
    };
    let identity = soliddraft_document::identify(std::io::Cursor::new(&bytes)).map_err(|e| e.to_string())?;
    if !matches!(identity.kind, DocKind::Part | DocKind::Assembly) {
        return Err(format!("a {} cannot be placed in an assembly", identity.kind));
    }
    revisions.record(identity.id, &identity.revision, &bytes);
    let mut r =
        DocRef::new(identity.id, identity.kind, absolute.to_string_lossy().replace('\\', "/"), identity.revision);
    if let Some(here) = s.doc.path() {
        r.set_path(here, &absolute);
    }
    // Beside what is there: the part's own box comes from its saved display data.
    s.doc.finish_loading();
    let there = s.doc.table().bounds(OccurrenceIndex::ROOT);
    let at = if s.components().is_empty() || there.is_empty() {
        DVec3::ZERO
    } else {
        let own_min = soliddraft_model::display::read(&bytes, Level::Standard)
            .ok()
            .flatten()
            .map(|(m, _)| m.bounds().min.x)
            .unwrap_or(0.0);
        DVec3::new(there.max.x + INSERT_GAP - own_min, 0.0, 0.0)
    };
    let id = s.doc.insert(r, placement.unwrap_or(Placement::translation(at))).map_err(insert_refusal)?;
    Ok(OccurrencePath::root().child(soliddraft_assembly::Step::component(id)))
}

/// A refused insert as a tool reports it: a cycle is typed JSON naming the occurrence it
/// runs through; anything else is its message.
fn insert_refusal(e: soliddraft_assembly::EditError) -> String {
    match &e {
        soliddraft_assembly::EditError::Cycle(at) => {
            serde_json::json!({ "refused": "cycle", "message": e.to_string(), "cause": at }).to_string()
        }
        _ => e.to_string(),
    }
}

/// What a refusal says to people: its message when it is typed JSON.
fn readable(refusal: &str) -> String {
    serde_json::from_str::<serde_json::Value>(refusal)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| refusal.to_owned())
}

fn save(state: &mut AppState, path: Option<PathBuf>) {
    match save_file(state, path) {
        Ok(Some(saved)) => state.note(format!("Saved {}", saved.display()), NoteKind::Ok),
        Ok(None) => {}
        Err(e) => state.note(e, NoteKind::Error),
    }
}

/// Saves the open assembly to `path`, or to its own file. `None` when there is nothing
/// to save, or no file yet and Save As was asked for instead.
fn save_file(state: &mut AppState, path: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    let Some(s) = state.assembly.session.as_mut() else { return Ok(None) };
    if path.is_none() && s.doc.path().is_none() {
        apply_save_as(state);
        return Ok(None);
    }
    // The browser downloads the file and keeps it for the recent list.
    let mut written = None;
    let saved = if cfg!(target_arch = "wasm32") {
        s.doc.save_with(path.as_deref(), |path, bytes| {
            crate::platform::write_file(path, bytes).map_err(std::io::Error::other)?;
            written = Some(bytes.to_vec());
            Ok(())
        })
    } else {
        s.doc.save(path.as_deref())
    };
    match saved {
        Ok(saved) => {
            if let Some(bytes) = written.or_else(|| std::fs::read(&saved).ok()) {
                state.revisions.record(s.doc.id(), &s.doc.revision(), &bytes);
                #[cfg(target_arch = "wasm32")]
                crate::documents_browser::remember(
                    &saved.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    DocKind::Assembly,
                    &bytes,
                );
            }
            crate::file::remember(&saved);
            state.recent = crate::file::recent();
            Ok(Some(saved))
        }
        Err(e) => Err(format!("cannot save the assembly: {e}")),
    }
}

/// The component Edit part opens when none is named: the part the last click in the
/// view met, when it lies in the selected component (a click selects the root's
/// component, a sub-assembly for a part inside one), or else the selected component.
pub(crate) fn edit_target(s: &AssemblySession) -> Option<OccurrencePath> {
    let selected = s.selected.as_ref()?;
    match s.picked.as_ref() {
        Some(p) if p.path.starts_with(selected) => Some(p.path.clone()),
        _ => Some(selected.clone()),
    }
}

/// Brings what saving a part edited in context changed up the assembly that shows it:
/// each sub-assembly from the one holding the part up to the open assembly takes the
/// new revision of the document below it, and only that one, and is saved. Returns the
/// files written, innermost first. A refusal or a failed write stops it and says why.
pub(crate) fn save_chain(state: &mut AppState, own: &OccurrencePath) -> Result<Vec<PathBuf>, String> {
    refresh(state);
    let s = state.assembly.session.as_ref().ok_or("no assembly is open")?;
    let table = s.doc.table();
    let mut row = table.find(own).ok_or("the part is no longer in the assembly")?;
    // The sub-assemblies between the open assembly and the part, outermost first.
    let mut subs = Vec::new();
    while let Some(parent) = table.parent(row).filter(|p| *p != OccurrenceIndex::ROOT) {
        subs.push(parent);
        row = parent;
    }
    subs.reverse();
    let (mut written, child) = save_subassemblies(state, &subs)?;
    let session = state.assembly.session.as_mut().ok_or("no assembly is open")?;
    let report = session.doc.update_documents(&[child]);
    if let Some(why) = report.refusal {
        if let Some(r) = report.mate_refusal.filter(|r| !r.mates.is_empty() || !r.free.is_empty()) {
            state.assembly.mate.conflict = Some(r);
        }
        return Err(why);
    }
    session.update();
    refresh(state);
    match save_file(state, None)? {
        Some(saved) => written.push(saved),
        None => return Err("the assembly has no file yet; choose where to save it".into()),
    }
    Ok(written)
}

/// Updates and saves each sub-assembly in `subs`, innermost first, each taking only the
/// new revision of the document below it. Returns the files written and the document the
/// open assembly takes next: the outermost sub-assembly, or the part itself.
#[cfg(not(target_arch = "wasm32"))]
fn save_subassemblies(
    state: &mut AppState,
    subs: &[OccurrenceIndex],
) -> Result<(Vec<PathBuf>, soliddraft_document::DocumentId), String> {
    let mut child = state.doc.model().id();
    let mut written = Vec::new();
    if subs.is_empty() {
        return Ok((written, child));
    }
    // Each sub-assembly's file, found from the file that holds it.
    let mut files = Vec::new();
    {
        let s = state.assembly.session.as_ref().ok_or("no assembly is open")?;
        let settings = crate::settings::file_locations();
        let mut holder = s.doc.path().map(Path::to_path_buf);
        for &sub in subs {
            let reference = s.doc.reference(s.doc.table().definition(sub)).ok_or("a sub-assembly has no file")?;
            let found = soliddraft_document::Resolver::new(holder.as_deref())
                .with_open(documents(state))
                .with_search_folders(settings.search_folders())
                .resolve(reference);
            let file = found
                .target()
                .and_then(|t| t.path())
                .map(Path::to_path_buf)
                .ok_or_else(|| format!("could not find the sub-assembly {}", reference.path))?;
            let id = reference.id.ok_or_else(|| format!("{} has no document id", reference.path))?;
            files.push((file.clone(), id));
            holder = Some(file);
        }
    }
    for (file, id) in files.into_iter().rev() {
        let mut doc = AssemblyDocument::open(&file, options(state)).map_err(|e| format!("{}: {e}", file.display()))?;
        let report = doc.update_documents(&[child]);
        if let Some(why) = report.refusal {
            return Err(format!("{}: {why}", file.display()));
        }
        let saved = doc.save(None).map_err(|e| format!("cannot save {}: {e}", file.display()))?;
        if let Ok(bytes) = std::fs::read(&saved) {
            state.revisions.record(id, &doc.revision(), &bytes);
        }
        written.push(saved);
        child = id;
    }
    refresh(state);
    Ok((written, child))
}

/// The browser holds sub-assemblies as uploads with no file to write back.
#[cfg(target_arch = "wasm32")]
fn save_subassemblies(
    state: &mut AppState,
    subs: &[OccurrenceIndex],
) -> Result<(Vec<PathBuf>, soliddraft_document::DocumentId), String> {
    if subs.is_empty() {
        return Ok((Vec::new(), state.doc.model().id()));
    }
    Err("the browser cannot save a sub-assembly from here yet; open the sub-assembly, Update it and save it".into())
}

fn apply_save_as(state: &mut AppState) {
    state.push(AppAction::Assembly(AssemblyAction::SaveAs));
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// The assembly tab: the viewport with the assembly's parts.
pub fn viewport(ui: &mut egui::Ui, state: &mut AppState) {
    let nav = &state.navigation;
    let Some(s) = state.assembly.session.as_mut() else { return };
    let ctx = ui.ctx().clone();
    let frame = s.frame(&ctx);
    let hover_label = s.shown.1.as_ref().and_then(|p| s.doc.table().find(p)).map(|r| s.doc.label(r));
    let empty = s.components().is_empty();
    let events = s.viewport.show_assembly(ui, frame, hover_label.as_deref(), nav);
    #[cfg(target_arch = "wasm32")]
    crate::web::show_assembly_view(&s.viewport.camera, s.viewport.rect);
    let picked = |s: &AssemblySession, texel: Option<[u32; 2]>| {
        let scene = s.scene.lock().expect("the scene lock");
        texel.and_then(|t| scene.decode(t)).map(|p| p.occurrence)
    };
    let hovered = picked(s, events.hover_texel);
    let mate_open = state.assembly.mate.open;
    let hovered_path = hovered.map(|r| s.doc.table().path(r));
    if hovered_path != s.shown.1 {
        s.scene.lock().expect("the scene lock").set_hover(s.doc.table(), hovered);
        s.shown.1 = hovered_path;
    }
    let rect = s.viewport.rect;
    let mut actions = Vec::new();
    if let Some(texel) = events.clicked_texel {
        let row = picked(s, texel);
        // A click names the face under it; with the Mate panel open it also picks it.
        let face = texel.and_then(|t| s.scene.lock().expect("the scene lock").decode(t));
        s.picked = face.map(|f| s.doc.selection(&f));
        if let Some(selection) = s.picked.clone().filter(|_| mate_open)
            && let Some(name) = selection.face
        {
            actions
                .push(AppAction::Assembly(AssemblyAction::Mate(crate::mates::MateAction::Pick(selection.path, name))));
        }
        // A click selects the root's component the part belongs to, as SolidWorks does.
        let top = row.map(|mut r| {
            while let Some(p) = s.doc.table().parent(r).filter(|p| *p != OccurrenceIndex::ROOT) {
                r = p;
            }
            s.doc.table().path(r)
        });
        actions.push(AppAction::Assembly(AssemblyAction::Select(top)));
    }
    if let Some(v) = events.set_view {
        s.viewport.camera.set_view(v);
    }
    if events.fit {
        actions.push(AppAction::Assembly(AssemblyAction::ZoomFit));
    }
    if let Some(d) = events.set_display {
        actions.push(AppAction::SetDisplay(d));
    }
    if events.toggle_projection {
        s.viewport.camera.orthographic = !s.viewport.camera.orthographic;
    }
    for a in actions {
        state.push(a);
    }
    let dropping = ctx.input(|i| !i.raw.hovered_files.is_empty());
    if empty || dropping {
        let text = if dropping {
            "Drop to insert the parts"
        } else {
            "Insert component, or drop part files here, to place parts"
        };
        let painter = ui.painter().with_clip_rect(rect);
        let mut job = egui::text::LayoutJob::simple(
            text.to_owned(),
            theme::font_medium(13.0),
            theme::muted(),
            (rect.width() - 40.0).max(1.0),
        );
        job.halign = egui::Align::Center;
        let galley = painter.layout_job(job);
        let r = Rect::from_center_size(rect.center(), galley.size() + vec2(28.0, 14.0));
        painter.rect_filled(r, egui::CornerRadius::same(99), theme::hud_bg());
        painter.rect_stroke(r, egui::CornerRadius::same(99), Stroke::new(1.0, theme::line()), egui::StrokeKind::Inside);
        painter.galley(r.center() - galley.rect.center().to_vec2(), galley, theme::muted());
    }
}

/// The assembly tree: the assembly, then a row per component; a sub-assembly opens to
/// show its own.
pub fn tree(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(s) = state.assembly.session.as_mut() else { return };
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().text(
        pos2(head.left() + 12.0, head.center().y),
        Align2::LEFT_CENTER,
        s.title(),
        theme::font_semibold(12.5),
        theme::ink(),
    );
    let counts = s.doc.table().counts();
    let (status, color) = if s.doc.is_loading() {
        ("Loading…".to_owned(), theme::muted())
    } else {
        let failed = s.doc.store().counts()[5];
        if failed > 0 {
            (format!("{failed} not loaded"), theme::error())
        } else {
            (format!("{} part{}", counts.parts, plural(counts.parts)), theme::ok())
        }
    };
    ui.painter().text(
        pos2(head.right() - 12.0, head.center().y),
        Align2::RIGHT_CENTER,
        status,
        theme::font_medium(11.0),
        color,
    );
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, theme::line()));
    let mut actions = Vec::new();
    let mut error = None;
    egui::ScrollArea::vertical().id_salt("assembly_tree").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        let selected = s.selected_row();
        let root = s.doc.assembly().name.clone();
        let r = row(ui, RowSpec { icon: Icon::Tree, name: &root, selected: selected.is_none(), ..RowSpec::plain() });
        if r.clicked() {
            actions.push(AppAction::Assembly(AssemblyAction::Select(None)));
        }
        let components = s.components();
        egui::CollapsingHeader::new(format!("Components ({})", components.len())).default_open(true).show(ui, |ui| {
            let mut stack: Vec<(OccurrenceIndex, u8)> = components.into_iter().rev().map(|r| (r, 0)).collect();
            while let Some((r, indent)) = stack.pop() {
                let table = s.doc.table();
                let flags = table.flags(r);
                let path = table.path(r);
                let def = s.doc.store().get(table.definition(r));
                let sub = flags.has(Flags::ASSEMBLY);
                let open = sub && s.expanded.contains(&path);
                let name = s.doc.label(r);
                let problem = match (&def.state, &def.freshness) {
                    (LoadState::Failed, _) => Some(def.problem.clone().unwrap_or_else(|| "Not loaded".into())),
                    _ => None,
                };
                let warn = warning(def);
                let sub_text = if flags.has(Flags::HIDDEN_SELF) { "hidden" } else { "" };
                let chosen = s.chosen.contains(&path);
                let resp = row(
                    ui,
                    RowSpec {
                        indent,
                        chevron: sub.then_some(open),
                        icon: if sub { Icon::Tree } else { Icon::Extrude },
                        name: &name,
                        sub: sub_text,
                        selected: Some(r) == selected || chosen,
                        dim: flags.has(Flags::HIDDEN) || flags.has(Flags::SUPPRESSED),
                        error: problem.as_deref(),
                        warn: warn.as_deref(),
                        suppressed: flags.has(Flags::SUPPRESSED_SELF),
                        ..RowSpec::plain()
                    },
                );
                #[cfg(target_arch = "wasm32")]
                crate::web::show_control(&name, resp.rect);
                let resp = match problem.as_deref().or(warn.as_deref()) {
                    Some(why) => resp.on_hover_text(why),
                    None => resp,
                };
                if !sub && resp.double_clicked() {
                    actions.push(AppAction::OpenComponentPart(path.clone()));
                } else if resp.clicked() {
                    if sub
                        && resp
                            .interact_pointer_pos()
                            .is_some_and(|p| p.x < resp.rect.left() + 18.0 + indent as f32 * 20.0)
                    {
                        if !s.expanded.remove(&path) {
                            s.expanded.insert(path.clone());
                        }
                    } else if ui.input(|i| i.modifiers.command) {
                        actions.push(AppAction::Assembly(AssemblyAction::Choose(path.clone())));
                    } else {
                        actions.push(AppAction::Assembly(AssemblyAction::Select(Some(path.clone()))));
                    }
                }
                resp.context_menu(|ui| {
                    if !sub {
                        let config = def.key.configuration.as_ref().map_or("Default", |id| id.0.as_str());
                        let display = s
                            .doc
                            .reference(table.definition(r))
                            .and_then(|reference| reference.display_state.as_ref())
                            .map_or("Default", |id| id.0.as_str());
                        ui.label(format!("Configuration: {config}"));
                        ui.label(format!("Display state: {display}"));
                        ui.separator();
                        if ui.button("Open part").clicked() {
                            actions.push(AppAction::OpenComponentPart(path.clone()));
                            ui.close();
                        }
                        if ui.button("Edit part in context").clicked() {
                            actions.push(AppAction::Assembly(AssemblyAction::EditPartInContext(Some(path.clone()))));
                            ui.close();
                        }
                        ui.separator();
                    }
                    component_appearance(ui, s, r, &path, &mut actions);
                    let hidden = flags.has(Flags::HIDDEN_SELF);
                    if ui.button(if hidden { "Show component" } else { "Hide component" }).clicked() {
                        actions.push(AppAction::Assembly(AssemblyAction::SetHidden(path.clone(), !hidden)));
                        ui.close();
                    }
                    ui.separator();
                    crate::subassembly::panel::context_menu(ui, &mut actions);
                });
                if open {
                    let children: Vec<OccurrenceIndex> =
                        table.children(r).filter(|c| !table.flags(*c).has(Flags::REMOVED)).collect();
                    stack.extend(children.into_iter().rev().map(|c| (c, indent + 1)));
                }
            }
        });
        let assembly = s.doc.assembly();
        egui::CollapsingHeader::new(format!("Mates ({})", assembly.mates.len())).default_open(true).show(ui, |ui| {
            for entry in &assembly.mates {
                ui.label(entry.known().map_or("Unsupported mate", |mate| mate.name()));
            }
        });
        egui::CollapsingHeader::new(format!("Assembly features ({})", assembly.patterns.len()))
            .default_open(true)
            .show(ui, |ui| {
                for pattern in &assembly.patterns {
                    ui.label(pattern.name.clone().unwrap_or_else(|| format!("Pattern {}", pattern.id.0)));
                }
            });
        error = crate::display_states::show_assembly(ui, s).err();
        assembly_appearances(ui, s, &mut actions);
    });
    if let Some(error) = error {
        state.note(error, crate::state::NoteKind::Error);
    }
    for a in actions {
        state.push(a);
    }
}

pub(crate) enum ComponentPart {
    Current,
    #[cfg(not(target_arch = "wasm32"))]
    File(PathBuf),
    #[cfg(target_arch = "wasm32")]
    Upload(String, Vec<u8>),
}

pub(crate) fn component_part(state: &AppState, path: &OccurrencePath) -> Result<ComponentPart, String> {
    let session = state.assembly.session.as_ref().ok_or("Open an assembly first.")?;
    let table = session.doc.table();
    let row = table.find(path).ok_or("The component is no longer in the assembly.")?;
    let def = table.definition(row);
    let reference =
        session.doc.reference(def).filter(|r| r.kind == DocKind::Part).ok_or("The component has no source part.")?;
    if reference.id == Some(state.doc.model().id()) {
        return Ok(ComponentPart::Current);
    }
    if state.doc.dirty {
        return Err("Save the open part before opening a different component.".into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let settings = crate::settings::file_locations();
        let source_path = Path::new(&reference.path);
        let referencing = session.doc.path().or_else(|| source_path.is_absolute().then_some(source_path));
        let resolution = soliddraft_document::Resolver::new(referencing)
            .with_open(documents(state))
            .with_search_folders(settings.search_folders())
            .resolve(reference);
        resolution
            .target()
            .and_then(|target| target.path())
            .map(|path| ComponentPart::File(path.to_owned()))
            .ok_or_else(|| format!("Could not find source part {}", reference.path))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let held = state.assembly.held.iter().find(|doc| Some(doc.id) == reference.id);
        let id = reference.id.ok_or("Upload the source part with the assembly first.")?;
        let revision = held.map_or(&reference.pinned, |doc| &doc.revision);
        let bytes = state
            .revisions
            .fetch(id, revision)
            .map_err(|_| format!("Upload {} to open the source part.", reference.path))?;
        let name = held
            .and_then(|doc| doc.path.as_ref())
            .and_then(|path| path.file_name())
            .map_or_else(|| reference.path.clone(), |name| name.to_string_lossy().into_owned());
        Ok(ComponentPart::Upload(name, bytes))
    }
}

fn component_appearance(
    ui: &mut egui::Ui,
    session: &AssemblySession,
    row: OccurrenceIndex,
    path: &OccurrencePath,
    actions: &mut Vec<AppAction>,
) {
    use soliddraft_materials::{Appearance, Srgb};
    ui.menu_button("Component appearance", |ui| {
        let assembly = session.doc.assembly();
        let existing = if let Some(display) = assembly.viewport_state() {
            display.view.appearances.iter().any(|(p, _)| p == path)
        } else {
            assembly.appearances.iter().any(|a| &a.path == path && a.scope == soliddraft_document::ConfigScope::All)
        };
        let shown = session.doc.table().colour(row).cloned().unwrap_or_else(Appearance::default_part);
        let id = ui.id().with(path);
        let c = shown.base_color;
        let mut rgb = ui.data_mut(|d| d.get_temp::<[u8; 3]>(id).unwrap_or([c.r, c.g, c.b]));
        egui::color_picker::color_edit_button_srgb(ui, &mut rgb);
        ui.data_mut(|d| d.insert_temp(id, rgb));
        let colour = Srgb::new(rgb[0], rgb[1], rgb[2]);
        if ui.button("Apply colour").clicked() {
            let appearance = crate::properties::painted(&shown, colour, 1.0);
            actions.push(AppAction::Assembly(AssemblyAction::SetColour(path.clone(), Some(appearance))));
            ui.close();
        }
        if existing && ui.button("Remove colour").clicked() {
            ui.data_mut(|d| d.remove::<[u8; 3]>(id));
            actions.push(AppAction::Assembly(AssemblyAction::SetColour(path.clone(), None)));
            ui.close();
        }
    });
}

fn assembly_appearances(ui: &mut egui::Ui, session: &AssemblySession, actions: &mut Vec<AppAction>) {
    ui.collapsing("Appearances", |ui| {
        let assembly = session.doc.assembly();
        let display = assembly.viewport_state().map(|state| &state.id);
        let choices: Vec<_> =
            assembly
                .appearances
                .iter()
                .filter_map(|a| assembly.appearance(&a.appearance).map(|look| (&a.path, look, None)))
                .chain(
                    assembly.viewport_states.states().iter().flat_map(|state| {
                        state.view.appearances.iter().map(move |(path, look)| (path, look, Some(state)))
                    }),
                )
                .collect();
        if choices.is_empty() {
            ui.label("No component appearance overrides");
        }
        for (i, (path, appearance, in_state)) in choices.into_iter().enumerate() {
            let name = session.doc.table().find(path).map_or_else(|| path.to_string(), |row| session.doc.label(row));
            let scope = in_state.map_or("Default", |state| state.name.as_str());
            ui.push_id(i, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let c = appearance.base_color;
                    ui.colored_label(egui::Color32::from_rgb(c.r, c.g, c.b), "●");
                    ui.label(format!("{} · Component · {name} · {scope}", appearance.name));
                    if in_state.map(|state| &state.id) == display && ui.small_button("Remove colour").clicked() {
                        actions.push(AppAction::Assembly(AssemblyAction::SetColour(path.clone(), None)));
                    }
                })
            });
        }
    });
}

/// The property panel: the Mate panel while it is open, otherwise the selected component's
/// placement, typed; or the assembly's summary.
pub fn properties(ui: &mut egui::Ui, state: &mut AppState) {
    let forming = state.assembly.form.open;
    let checking = state.assembly.interference.open;
    let configuring = state.assembly.configure.open;
    let collision_drag = &mut state.assembly.collision_drag;
    let dynamic_clearance = &mut state.assembly.dynamic_clearance;
    let clearance_mm = &mut state.assembly.clearance_mm;
    let physical_dynamics = &mut state.assembly.physical_dynamics;
    let Some(s) = state.assembly.session.as_mut() else { return };
    let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
    let title = match s.selected_row() {
        Some(r) => s.doc.label(r),
        None => s.doc.assembly().name.clone(),
    };
    icons::paint(
        ui.painter(),
        if s.selected_row().is_some() { Icon::Extrude } else { Icon::Tree },
        Rect::from_center_size(pos2(head.left() + 20.0, head.center().y), vec2(16.0, 16.0)),
        theme::ink(),
    );
    ui.painter().text(
        pos2(head.left() + 34.0, head.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::font_semibold(12.5),
        theme::ink(),
    );
    ui.painter().hline(head.x_range(), head.bottom() - 0.5, Stroke::new(1.0, theme::line()));
    // The Form subassembly panel and the Mate panel take the whole area while open: their
    // fields do not fit under the placement fields.
    if forming {
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(12, 0))
            .show(ui, |ui| crate::subassembly::panel::show(ui, state));
        return;
    }
    if checking {
        egui::ScrollArea::vertical().id_salt("assembly_interference").auto_shrink([false, true]).show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(12, 0))
                .show(ui, |ui| crate::interference::panel::show(ui, state));
        });
        return;
    }
    if configuring {
        egui::ScrollArea::vertical().id_salt("assembly_configurations").auto_shrink([false, true]).show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(12, 0))
                .show(ui, |ui| crate::configurations::show(ui, state));
        });
        return;
    }
    let mut actions = Vec::new();
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        let Some(draft) = s.draft.as_mut() else {
            let counts = s.doc.table().counts();
            theme::section_label(ui, "Assembly");
            ui.label(format!("{} component{}", s.components().len(), plural(s.components().len())));
            ui.label(format!("{} part{} placed", counts.parts, plural(counts.parts)));
            ui.label(match s.doc.path() {
                Some(p) => format!("Saved as {}", p.display()),
                None => "Not saved yet".to_owned(),
            });
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Select a component to place it.").color(theme::muted()));
            return;
        };
        ui.add_enabled_ui(!s.doc.dragging(), |ui| {
            ui.checkbox(collision_drag, "Collision detection (mesh-certified)");
            ui.checkbox(dynamic_clearance, "Dynamic clearance (mesh-certified)");
            if *dynamic_clearance {
                ui.add(egui::DragValue::new(clearance_mm).range(0.0..=1e9).suffix(" mm"));
            }
            ui.checkbox(physical_dynamics, "Physical Dynamics: push what it meets");
        });
        if s.doc.drag_pending() {
            ui.label("Checking motion...");
        }
        if let Some(report) = s.doc.drag_report() {
            if let Some(error) = &report.error {
                ui.colored_label(theme::error(), error);
            }
            if report.stopped {
                ui.colored_label(theme::warn(), "Stopped before the requested pose");
            }
            if let Some(distance) = report.distance {
                ui.label(match distance.hi {
                    Some(hi) => format!("Clearance: {:.4} to {:.4} mm", distance.lo, hi),
                    None => format!("Clearance: at least {:.4} mm", distance.lo),
                });
            }
            for face in &report.faces {
                ui.label(format!("{}: {}", face.path, face.face));
            }
        }
        theme::section_label(ui, "Placement");
        ui.label(
            egui::RichText::new("Relative to the assembly that holds it. Each change is one undo step.")
                .size(11.0)
                .color(theme::muted()),
        );
        ui.add_space(4.0);
        // A change applies once the field lets go: a drag ends or typing leaves it.
        let mut busy = false;
        let mut changed = false;
        let mut started = false;
        let mut released = false;
        egui::Grid::new("assembly_placement").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            let rows = ["X", "Y", "Z"].into_iter().zip(draft.translation.iter_mut()).map(|(a, v)| (a, v, "mm"));
            let turns = ["Rotate X", "Rotate Y", "Rotate Z"].into_iter().zip(draft.rotation.iter_mut());
            for (label, value, unit) in rows.chain(turns.map(|(a, v)| (a, v, "°"))) {
                ui.label(label);
                let resp = ui.add(egui::DragValue::new(value).speed(1.0).max_decimals(4).suffix(format!(" {unit}")));
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, format!("Placement {label}"))
                });
                #[cfg(target_arch = "wasm32")]
                crate::web::show_control(format!("Placement {label}"), resp.rect);
                busy |= resp.dragged() || resp.has_focus();
                changed |= resp.changed();
                started |= resp.drag_started();
                released |= resp.drag_stopped();
                ui.end_row();
            }
        });
        let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if cancel && s.doc.dragging() {
            s.doc.cancel_drag();
            s.draft = None;
            return;
        }
        if started
            && let Err(e) = if *physical_dynamics {
                s.doc.begin_dynamic_drag(draft.path.clone(), dynamic_clearance.then_some(*clearance_mm))
            } else if *collision_drag || *dynamic_clearance {
                s.doc.begin_guarded_drag(draft.path.clone(), dynamic_clearance.then_some(*clearance_mm))
            } else {
                s.doc.begin_drag(draft.path.clone())
            }
        {
            ui.colored_label(theme::muted(), e.to_string());
        }
        if changed
            && s.doc.dragging()
            && let Err(e) = s.doc.preview_drag(draft.placement())
        {
            ui.colored_label(theme::muted(), e.to_string());
        }
        if released && s.doc.dragging() {
            if let Err(e) = s.doc.commit_drag() {
                ui.colored_label(theme::muted(), e.to_string());
            }
            s.draft = None;
            return;
        }
        if !busy && draft.edited() {
            actions.push(AppAction::Assembly(AssemblyAction::Place(draft.path.clone(), draft.placement())));
        }
    });
    // The fields fill again from the moved placement, or the old one when it was refused.
    if !actions.is_empty() {
        s.draft = None;
    }
    for a in actions {
        state.push(a);
    }
}

/// The bottom bar's status while the assembly is active.
pub fn status(state: &AppState) -> Option<Status> {
    let s = state.assembly.session.as_ref()?;
    let headline = if s.doc.is_loading() {
        ("Loading parts…".to_owned(), theme::muted())
    } else {
        ("Assembly".to_owned(), theme::ok())
    };
    let counts = s.doc.table().counts();
    let n = s.components().len();
    Some(Status {
        headline,
        note: state.note.as_ref().map(StatusNote::of),
        details: vec![
            "Editing assembly".to_owned(),
            format!("{n} component{}", plural(n)),
            format!("{} part{} placed", counts.parts, plural(counts.parts)),
        ],
        position: "mm".to_owned(),
    })
}
