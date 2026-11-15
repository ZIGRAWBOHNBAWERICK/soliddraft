//! Drag previews: while the user drags a feature's dimension, intermediate results come
//! from a worker off the interface thread, and the exact result of the final value is
//! applied as one command, so one drag is one undo step.
//!
//! The worker holds a fork of the part, which shares the part's feature builds on the
//! same kernel. For each value it edits the dragged feature in the fork and lets the
//! model rebuild from that feature down, so an intermediate result is the whole part as the edit would leave it. It
//! answers only the newest value it has been asked for, so a fast drag gets results at
//! the worker's pace and none for values the drag has already passed. Results arrive as
//! display meshes without picks: preview state stays in the feature controls and tree, never saved, exported,
//! measured or used for a reference. Those all read the document, which a drag leaves
//! alone until its final value is applied.
//!
//! A drag's [`Setup`] is the document revision, the selection, the feature and the
//! dimension. When any of them changes the drag is obsolete: its results and errors are
//! dropped and nothing is applied. Within one drag, a result older than the one shown is
//! dropped, and an error is shown only for the newest value; an error for a value the
//! drag has passed is dropped.
//!
//! The meshes of values already built are kept for the rest of the drag, starting with
//! the document's own mesh at the starting value, so dragging back over them shows them
//! at once without asking the worker.

use crate::document::{
    CandidateDisplay, Command, DisplayMesh, Document, EndKind, FeatureId, FeatureKind, FeatureStatus, ReadyDisplay,
    Selection, ViewToken, construction_view, fmt_mm, ready_display,
};
use soliddraft_kernel_api::{ProofWork, TessellationParams};
use soliddraft_model as model;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
    mpsc,
};

/// A feature dimension a drag can change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dimension {
    /// A boss's depth, or a blind cut's.
    Depth,
    Radius,
    Distance,
}

impl Dimension {
    /// The dimension a feature offers for dragging, if it has one.
    pub fn of(kind: &FeatureKind) -> Option<Dimension> {
        match kind {
            FeatureKind::Extrude(p) if p.end == EndKind::Blind => Some(Dimension::Depth),
            FeatureKind::Cut(p) if !p.through_all && p.up_to.is_none() => Some(Dimension::Depth),
            FeatureKind::Fillet(_) => Some(Dimension::Radius),
            FeatureKind::Chamfer(_) => Some(Dimension::Distance),
            FeatureKind::Combine { .. }
            | FeatureKind::Extrude(_)
            | FeatureKind::Cut(_)
            | FeatureKind::ReferencePlane(_)
            | FeatureKind::Sketch(_)
            | FeatureKind::Hole(_)
            | FeatureKind::Pattern(_)
            | FeatureKind::Mirror(_)
            | FeatureKind::Draft(_)
            | FeatureKind::Shell(_)
            | FeatureKind::Offset(_)
            | FeatureKind::Thicken(_)
            | FeatureKind::Scan(_)
            | FeatureKind::ImportStep(_)
            | FeatureKind::Lattice(_)
            | FeatureKind::Gyroid(_)
            | FeatureKind::Revolve(_)
            | FeatureKind::Loft(_)
            | FeatureKind::Sweep(_) => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Dimension::Depth => "Depth",
            Dimension::Radius => "Radius",
            Dimension::Distance => "Distance",
        }
    }

    /// The dimension's value in `kind`, when `kind` has it.
    pub fn get(self, kind: &FeatureKind) -> Option<f64> {
        match (self, kind) {
            (Dimension::Depth, FeatureKind::Extrude(p)) => Some(p.depth),
            (Dimension::Depth, FeatureKind::Cut(p)) if !p.through_all => Some(p.depth),
            (Dimension::Radius, FeatureKind::Fillet(p)) => Some(p.radius),
            (Dimension::Distance, FeatureKind::Chamfer(p)) => Some(p.distance),
            _ => None,
        }
    }

    /// `kind` with the dimension set to `value`.
    pub fn with(self, kind: &FeatureKind, value: f64) -> FeatureKind {
        let mut kind = kind.clone();
        match (self, &mut kind) {
            (Dimension::Depth, FeatureKind::Extrude(p)) => p.depth = value,
            (Dimension::Depth, FeatureKind::Cut(p)) => p.depth = value,
            (Dimension::Radius, FeatureKind::Fillet(p)) => p.radius = value,
            (Dimension::Distance, FeatureKind::Chamfer(p)) => p.distance = value,
            _ => {}
        }
        kind
    }
}

/// The command that edits feature `id` to `kind`.
pub fn edit_command(id: FeatureId, kind: FeatureKind) -> Option<Command> {
    Some(match kind {
        FeatureKind::Extrude(params) => Command::EditExtrude { id, params },
        FeatureKind::Cut(params) => Command::EditCut { id, params },
        FeatureKind::Fillet(params) => Command::EditFillet { id, params },
        FeatureKind::Chamfer(params) => Command::EditChamfer { id, params },
        kind @ (FeatureKind::Hole(_)
        | FeatureKind::Pattern(_)
        | FeatureKind::Mirror(_)
        | FeatureKind::Draft(_)
        | FeatureKind::Shell(_)
        | FeatureKind::Offset(_)
        | FeatureKind::Thicken(_)
        | FeatureKind::Revolve(_)
        | FeatureKind::Loft(_)
        | FeatureKind::Sweep(_)) => Command::EditFeature { id, kind },
        FeatureKind::Combine { .. }
        | FeatureKind::ReferencePlane(_)
        | FeatureKind::Sketch(_)
        | FeatureKind::Scan(_)
        | FeatureKind::ImportStep(_)
        | FeatureKind::Lattice(_)
        | FeatureKind::Gyroid(_) => {
            return None;
        }
    })
}

/// What a drag's results depend on. Any change makes them obsolete.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Setup {
    document: soliddraft_document::DocumentId,
    basis: Option<soliddraft_kernel_api::BodyId>,
    pub revision: u64,
    pub selection: Option<Selection>,
    pub feature: FeatureId,
    pub dimension: Dimension,
}

impl Setup {
    /// The setup of a drag of `dimension` on `feature` in `doc` as it is now.
    pub fn of(doc: &Document, feature: FeatureId, dimension: Dimension) -> Setup {
        Setup {
            document: doc.model().id(),
            basis: doc.display_body(),
            revision: doc.revision(),
            selection: doc.selection,
            feature,
            dimension,
        }
    }
}

/// One displayed value. Its geometry is not a saved or exportable body.
#[derive(Clone, Debug)]
pub struct Intermediate {
    pub value: f64,
    pub mesh: Arc<DisplayMesh>,
    pub proved: bool,
    candidate: Option<CandidateDisplay>,
}

#[derive(Clone, Debug)]
pub enum Shown {
    Waiting,
    Result(Intermediate),
    Failed { value: f64, message: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Finish {
    Applied { value: f64 },
    Pending { value: f64 },
    Failed { value: f64, message: String },
    Refused { value: f64, message: String },
    Unchanged,
    Obsolete,
}

const CACHED: usize = 16;

#[derive(Clone)]
struct Cached {
    shown: Intermediate,
    proof: Option<ProvedBuild>,
}

/// The existing drag lifecycle owns the document's worker until commit/cancel.
pub struct DragPreview {
    setup: Setup,
    base: FeatureKind,
    source: Arc<model::Document>,
    start: f64,
    value: f64,
    latest: u64,
    shown_request: u64,
    proof_request: u64,
    shown: Shown,
    proof: Option<Result<ProvedBuild, String>>,
    notice: Option<String>,
    cache: Vec<Cached>,
    worker: Worker,
}

impl DragPreview {
    pub fn start(doc: &mut Document, feature: FeatureId, dimension: Dimension) -> Result<Self, String> {
        let item = doc.feature(feature).ok_or("no such feature")?;
        let start = dimension.get(&item.kind).ok_or_else(|| format!("{} has no {}", item.name, dimension.label()))?;
        if item.status == FeatureStatus::Inactive {
            return Err(format!("{} is not built; unsuppress it or move the rollback bar below it", item.name));
        }
        let base = item.kind.clone();
        let setup = Setup::of(doc, feature, dimension);
        let mut cache = Vec::new();
        if !doc.proof_pending()
            && item.status == FeatureStatus::Ok
            && let Some(mesh) = doc.display_body().and_then(|body| doc.meshes.get(&body))
        {
            cache.push(Cached {
                shown: Intermediate { value: start, mesh: mesh.clone(), proved: true, candidate: None },
                proof: None,
            });
        }
        let worker = doc.take_worker()?;
        let source = Arc::new(doc.model().fork());
        Ok(Self {
            setup,
            base,
            source,
            start,
            value: start,
            latest: 0,
            shown_request: 0,
            proof_request: 0,
            shown: Shown::Waiting,
            proof: None,
            notice: None,
            cache,
            worker,
        })
    }

    pub fn setup(&self) -> Setup {
        self.setup
    }
    pub fn feature(&self) -> FeatureId {
        self.setup.feature
    }
    pub fn dimension(&self) -> Dimension {
        self.setup.dimension
    }
    pub fn value(&self) -> f64 {
        self.value
    }
    pub fn start_value(&self) -> f64 {
        self.start
    }
    pub fn shown(&self) -> &Shown {
        &self.shown
    }
    pub fn mesh(&self) -> Option<Arc<DisplayMesh>> {
        match &self.shown {
            Shown::Result(value) => Some(value.mesh.clone()),
            Shown::Waiting | Shown::Failed { .. } => None,
        }
    }
    pub fn pending(&self) -> bool {
        self.proof_request < self.latest
    }
    pub fn current_request(&self) -> Option<u64> {
        (self.latest != 0).then_some(self.latest)
    }
    pub fn displayed_request(&self) -> Option<u64> {
        matches!(self.shown, Shown::Result(_)).then_some(self.shown_request)
    }
    pub fn proved_request(&self) -> Option<u64> {
        let proved = match &self.proof {
            Some(Ok(proof)) => proof.failure.is_none() && !proof.model.is_dirty(),
            _ => matches!(&self.shown, Shown::Result(shown) if shown.proved),
        };
        (proved && self.proof_request == self.latest).then_some(self.latest)
    }
    pub fn is_current(&self, doc: &Document) -> bool {
        Setup::of(doc, self.setup.feature, self.setup.dimension) == self.setup
    }
    fn token(&self) -> ViewToken {
        ViewToken { revision: self.setup.revision, request: self.latest, transient: true }
    }

    pub fn request(&mut self, value: f64, doc: &mut Document) {
        if !value.is_finite() || value == self.value {
            return;
        }
        doc.discard_provisional(self.token());
        self.value = value;
        self.latest = self.worker.invalidate();
        self.proof = None;
        self.notice = None;
        if let Some(hit) = self.cache.iter().find(|entry| entry.shown.value == value) {
            self.shown = Shown::Result(hit.shown.clone());
            self.shown_request = self.latest;
            self.proof_request = self.latest;
            self.proof = hit.proof.clone().map(Ok);
            if let Some(candidate) = &hit.shown.candidate {
                doc.show_provisional(self.token(), candidate.clone());
            }
            return;
        }
        self.shown = Shown::Waiting;
        let edit = ScalarDraft { feature: model::FeatureId(self.feature() as u32), dimension: self.dimension(), value };
        let result = self.worker.enqueue(self.latest, doc.build_job(self.source.clone(), Some(edit)));
        if let Err(message) = result {
            self.shown = Shown::Failed { value, message: message.clone() };
            self.proof = Some(Err(message));
            self.proof_request = self.latest;
        }
    }

    pub fn poll(&mut self, doc: &mut Document) -> bool {
        let mut changed = false;
        for answer in self.worker.answers() {
            if answer.request != self.latest {
                doc.hold_proof_answer(answer);
                continue;
            }
            // The early display runs beside the exact proof and may answer after it.
            if self.proof_request == answer.request && matches!(answer.kind, AnswerKind::Construction(_)) {
                continue;
            }
            match answer.kind {
                AnswerKind::Construction(Ok(candidate)) => {
                    self.shown = Shown::Result(Intermediate {
                        value: self.value,
                        mesh: candidate.mesh(),
                        proved: false,
                        candidate: Some(candidate.clone()),
                    });
                    self.shown_request = answer.request;
                    doc.show_provisional(self.token(), candidate);
                    changed = true;
                }
                AnswerKind::Construction(Err(message)) => self.notice = Some(message),
                AnswerKind::Proved(result) => {
                    self.proof_request = answer.request;
                    match &result {
                        Ok(proof) if proof.failure.is_none() => {
                            if let Some(display) = &proof.display {
                                let candidate = CandidateDisplay::Proved(display.clone());
                                let shown = Intermediate {
                                    value: self.value,
                                    mesh: candidate.mesh(),
                                    proved: true,
                                    candidate: Some(candidate.clone()),
                                };
                                self.shown = Shown::Result(shown.clone());
                                self.shown_request = self.latest;
                                if self.cache.len() == CACHED {
                                    self.cache.remove(0);
                                }
                                self.cache.push(Cached { shown, proof: Some(proof.clone()) });
                                doc.show_provisional(self.token(), candidate);
                            }
                            self.notice = proof.display_error.clone();
                        }
                        Ok(proof) => {
                            self.shown = Shown::Failed {
                                value: self.value,
                                message: proof.failure.clone().expect("matched failed proof"),
                            };
                            doc.discard_provisional(self.token());
                        }
                        Err(message) => {
                            self.shown = Shown::Failed { value: self.value, message: message.clone() };
                            doc.discard_provisional(self.token());
                        }
                    }
                    self.proof = Some(result);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Rendering evidence for the current completed drag answer, never model proof.
    pub fn display_bound(&self) -> Option<soliddraft_kernel_api::DisplayBound> {
        self.proved_request()?;
        let proof = self.proof.as_ref()?.as_ref().ok()?;
        if proof.failure.is_some() {
            return None;
        }
        proof.display.as_ref()?;
        proof.model.kernel().display_bound(proof.model.body()?).ok().flatten()
    }

    pub fn label(&self) -> (String, bool) {
        if let Shown::Failed { value, message } = &self.shown {
            return (format!("Edit failed · {} {} mm: {message}", self.dimension().label(), fmt_mm(*value)), true);
        }
        let phase = if self.pending() { "Proof pending" } else { "Preview" };
        let mut label = format!("{phase} · {} {} mm", self.dimension().label(), fmt_mm(self.value));
        if let Some(notice) = &self.notice {
            label.push_str(&format!(" · {notice}"));
        }
        (label, false)
    }

    pub fn cancel(self, doc: &mut Document) {
        if self.setup.document != doc.model().id()
            || self.setup.revision != doc.revision()
            || self.setup.basis != doc.display_body()
        {
            return;
        }
        doc.discard_provisional(self.token());
        doc.return_worker(self.worker);
    }

    pub fn finish(self, doc: &mut Document) -> Finish {
        if !self.is_current(doc) {
            self.cancel(doc);
            return Finish::Obsolete;
        }
        let value = self.value;
        if value == self.start {
            self.cancel(doc);
            return Finish::Unchanged;
        }
        let Some(command) = edit_command(self.feature(), self.dimension().with(&self.base, value)) else {
            self.cancel(doc);
            return Finish::Unchanged;
        };
        let feature = self.feature();
        let token = self.token();
        let candidate = if self.proof.is_none() {
            match &self.shown {
                Shown::Result(value) => value.candidate.clone(),
                _ => None,
            }
        } else {
            None
        };
        let prepared = PreparedBuild { worker: self.worker, request: self.latest, completed: self.proof, candidate };
        doc.discard_provisional(token);
        drop(self.cache);
        drop(self.shown);
        match doc.apply_prepared(command, prepared) {
            Err(message) => Finish::Refused { value, message },
            Ok(false) => Finish::Pending { value },
            Ok(true) => match doc.feature(feature).map(|feature| &feature.status) {
                Some(FeatureStatus::Error(message)) => Finish::Failed { value, message: message.clone() },
                _ => match doc.display_failure() {
                    Some(message) => Finish::Failed { value, message: message.to_owned() },
                    None => Finish::Applied { value },
                },
            },
        }
    }
}

#[derive(Clone)]
pub(crate) struct ProvedBuild {
    pub model: Arc<model::Document>,
    pub display: Option<Arc<ReadyDisplay>>,
    pub failure: Option<String>,
    pub display_error: Option<String>,
}

pub(crate) struct PreparedBuild {
    pub worker: Worker,
    pub request: u64,
    pub completed: Option<Result<ProvedBuild, String>>,
    pub candidate: Option<CandidateDisplay>,
}

#[derive(Clone, Copy)]
pub(crate) struct ScalarDraft {
    pub feature: model::FeatureId,
    pub dimension: Dimension,
    pub value: f64,
}

pub(crate) struct Job {
    pub source: Arc<model::Document>,
    pub edit: Option<ScalarDraft>,
    pub baseline: Option<Arc<model::Document>>,
    pub params: TessellationParams,
    pub level: soliddraft_tessellation::Level,
}

pub(crate) enum AnswerKind {
    Construction(Result<CandidateDisplay, String>),
    Proved(Result<ProvedBuild, String>),
}

pub(crate) struct Answer {
    pub request: u64,
    pub kind: AnswerKind,
}

struct Queued {
    request: u64,
    job: Job,
}

struct ProofTask {
    request: u64,
    model: model::Document,
    level: soliddraft_tessellation::Level,
    allowance: Box<dyn ProofWork>,
}

/// An early display of one request, built beside that request's exact proof.
struct ConstructionTask {
    request: u64,
    baseline: Arc<model::Document>,
    edits: Vec<soliddraft_kernel_api::ScalarEdit>,
    params: TessellationParams,
    model: model::Document,
}

/// Sends answers and wakes the interface, which draws no frames while it only waits.
#[derive(Clone)]
struct Answered {
    sender: mpsc::Sender<Answer>,
    repaint: Option<egui::Context>,
}

impl Answered {
    fn send(&self, answer: Answer) -> Result<(), ()> {
        self.sender.send(answer).map_err(|_| ())?;
        if let Some(ctx) = &self.repaint {
            ctx.request_repaint();
        }
        Ok(())
    }
}

/// Applies a request's edit and reserves its exact proof. Neither builds anything,
/// so the proof starts without waiting for the early display.
fn prepare(queued: Queued) -> Result<(ProofTask, Option<ConstructionTask>), Answer> {
    let Queued { request, job: Job { source, edit, baseline, params, level } } = queued;
    let refused = |message: String| Answer { request, kind: AnswerKind::Proved(Err(message)) };
    let mut model = Arc::try_unwrap(source).unwrap_or_else(|source| source.fork());
    if let Some(edit) = edit {
        crate::document::scalar_draft_command(&model, edit)
            .and_then(|command| model.execute_deferred(command).map(|_| ()).map_err(|error| error.to_string()))
            .map_err(refused)?;
    }
    // No operation budget is active on the worker, so this is the same per-build
    // allowance that a construction request reserves for its proof.
    let allowance = model.kernel().reserve_proof_work().map_err(|error| refused(error.to_string()))?;
    let construction = baseline.and_then(|baseline| {
        let edits = crate::document::scalar_edits(&baseline, &model).filter(|edits| !edits.is_empty())?;
        if baseline.live_bodies().len() != 1 {
            return None;
        }
        baseline.body()?;
        Some(ConstructionTask { request, baseline, edits, params, model: model.fork() })
    });
    Ok((ProofTask { request, model, level, allowance }, construction))
}

fn construct(task: ConstructionTask, answered: &Answered) -> Result<(), ()> {
    let ConstructionTask { request, baseline, edits, params, model } = task;
    let Some(body) = baseline.body() else {
        return Ok(());
    };
    // The exact proof already holds its own allowance; this request's is dropped unused.
    let candidate = model
        .kernel()
        .construction_edit(body, &edits, &params)
        .and_then(|preview| preview.display)
        .map_err(|error| error.to_string())
        .and_then(|display| construction_view(display, &model))
        .map(|view| CandidateDisplay::Construction(Arc::new(view)));
    answered.send(Answer { request, kind: AnswerKind::Construction(candidate) })
}

/// A feature that shapes the body did not build. A sketch that did not build blocks
/// each active feature that uses it, so it is not a failure of the body itself.
pub(crate) fn model_failure(model: &model::Document) -> Option<String> {
    let mut shaping = model.features().iter().filter(|feature| !matches!(feature.kind, model::FeatureKind::Sketch(_)));
    shaping.find_map(|feature| match model.status(feature.id) {
        Some(model::FeatureStatus::Failed(message)) => Some(format!("{}: {message}", feature.name)),
        Some(status @ (model::FeatureStatus::RepairNeeded(_) | model::FeatureStatus::Blocked(_))) => {
            Some(format!("{}: {status:?}", feature.name))
        }
        _ => None,
    })
}

fn prove(
    task: ProofTask,
    newest: &AtomicU64,
    protected: &AtomicU64,
    previous: Option<&model::Document>,
) -> ProvedBuild {
    let ProofTask { request, mut model, level, allowance } = task;
    if let Some(previous) = previous {
        model.reuse_builds_from(previous);
    }
    let mut display = None;
    let mut display_error = None;
    let mut failure = None;
    let result = allowance.run(&mut || {
        model.rebuild();
        failure = model_failure(&model);
        if failure.is_none() && wanted(request, newest, protected) {
            match ready_display(&model, level) {
                Ok(view) => display = view.map(Arc::new),
                Err(error) => display_error = Some(error),
            }
        }
        Ok(())
    });
    if let Err(error) = result
        && failure.is_none()
    {
        display_error = Some(error.to_string());
    }
    ProvedBuild { model: Arc::new(model), display, failure, display_error }
}

fn wanted(request: u64, newest: &AtomicU64, protected: &AtomicU64) -> bool {
    newest.load(Ordering::Acquire) == request || protected.load(Ordering::Acquire) == request
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn next_work<T>(pending: &mut Vec<T>, newest: u64, protected: u64, key: impl Fn(&T) -> u64) -> Option<T> {
    pending.retain(|work| {
        let request = key(work);
        request == newest || request == protected
    });
    let index = pending.iter().position(|work| key(work) == newest).or_else(|| (!pending.is_empty()).then_some(0))?;
    Some(pending.swap_remove(index))
}

/// The oldest wanted work. Early displays keep request order, so a committed
/// request's display is ready before a newer transient request replaces it.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn oldest_work<T>(pending: &mut Vec<T>, newest: u64, protected: u64, key: impl Fn(&T) -> u64) -> Option<T> {
    pending.retain(|work| {
        let request = key(work);
        request == newest || request == protected
    });
    (!pending.is_empty()).then(|| pending.remove(0))
}

/// One scheduler, with independent construction and exact jobs. A long proof
/// cannot hold the next edit's construction preview behind it, and a slow
/// construction preview cannot hold its own proof behind it.
pub(crate) struct Worker {
    answers: mpsc::Receiver<Answer>,
    newest: Arc<AtomicU64>,
    protected: Arc<AtomicU64>,
    request: u64,
    #[cfg(not(target_arch = "wasm32"))]
    jobs: mpsc::Sender<Queued>,
    #[cfg(target_arch = "wasm32")]
    answered: Answered,
    #[cfg(target_arch = "wasm32")]
    remote: remote::Remote,
}

impl Worker {
    /// `repaint` is woken after each answer, so the interface need not poll for one.
    pub(crate) fn new(repaint: Option<egui::Context>) -> Result<Self, String> {
        let (sender, answers) = mpsc::channel();
        let answered = Answered { sender, repaint };
        let newest = Arc::new(AtomicU64::new(0));
        let protected = Arc::new(AtomicU64::new(0));
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (jobs, queue) = mpsc::channel::<Queued>();
            let (proofs, proof_queue) = mpsc::channel::<ProofTask>();
            let (constructions, construction_queue) = mpsc::channel::<ConstructionTask>();
            let current = newest.clone();
            let retained = protected.clone();
            let finished = answered.clone();
            let refusals = answered.clone();
            std::thread::Builder::new()
                .name("edit-proof".into())
                .spawn(move || {
                    let mut previous: Option<Arc<model::Document>> = None;
                    let mut pending = Vec::new();
                    loop {
                        if pending.is_empty() {
                            match proof_queue.recv() {
                                Ok(task) => pending.push(task),
                                Err(_) => break,
                            }
                        }
                        pending.extend(proof_queue.try_iter());
                        let Some(task) = next_work(
                            &mut pending,
                            current.load(Ordering::Acquire),
                            retained.load(Ordering::Acquire),
                            |task| task.request,
                        ) else {
                            continue;
                        };
                        let request = task.request;
                        if !wanted(request, &current, &retained) {
                            continue;
                        }
                        let result = prove(task, &current, &retained, previous.as_deref());
                        if result.failure.is_none() && !result.model.is_dirty() {
                            previous = Some(result.model.clone());
                        }
                        if wanted(request, &current, &retained)
                            && finished.send(Answer { request, kind: AnswerKind::Proved(Ok(result)) }).is_err()
                        {
                            break;
                        }
                    }
                })
                .map_err(|error| format!("cannot start edit proof: {error}"))?;
            let current = newest.clone();
            let retained = protected.clone();
            std::thread::Builder::new()
                .name("edit-construction".into())
                .spawn(move || {
                    let mut pending = Vec::new();
                    loop {
                        if pending.is_empty() {
                            match construction_queue.recv() {
                                Ok(task) => pending.push(task),
                                Err(_) => break,
                            }
                        }
                        pending.extend(construction_queue.try_iter());
                        let Some(task) = oldest_work(
                            &mut pending,
                            current.load(Ordering::Acquire),
                            retained.load(Ordering::Acquire),
                            |task| task.request,
                        ) else {
                            continue;
                        };
                        if !wanted(task.request, &current, &retained) {
                            continue;
                        }
                        if construct(task, &answered).is_err() {
                            break;
                        }
                    }
                })
                .map_err(|error| format!("cannot start edit construction: {error}"))?;
            let current = newest.clone();
            let retained = protected.clone();
            std::thread::Builder::new()
                .name("edit-prepare".into())
                .spawn(move || {
                    for queued in queue {
                        if !wanted(queued.request, &current, &retained) {
                            continue;
                        }
                        match prepare(queued) {
                            Ok((proof, construction)) => {
                                if proofs.send(proof).is_err()
                                    || construction.is_some_and(|task| constructions.send(task).is_err())
                                {
                                    break;
                                }
                            }
                            Err(answer) => {
                                if refusals.send(answer).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                })
                .map_err(|error| format!("cannot start edit preparation: {error}"))?;
            Ok(Self { answers, newest, protected, request: 0, jobs })
        }
        #[cfg(target_arch = "wasm32")]
        Ok(Self { answers, newest, protected, request: 0, answered, remote: remote::Remote::default() })
    }

    pub(crate) fn protect(&mut self, request: Option<u64>) {
        self.protected.store(request.unwrap_or(0), Ordering::Release);
    }

    pub(crate) fn activate(&mut self, request: u64) {
        self.newest.store(request, Ordering::Release);
    }

    pub(crate) fn invalidate(&mut self) -> u64 {
        self.request =
            self.request.checked_add(1).filter(|request| *request != u64::MAX).expect("edit request space exhausted");
        self.newest.store(self.request, Ordering::Release);
        self.request
    }

    pub(crate) fn enqueue(&mut self, request: u64, job: Job) -> Result<(), String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.jobs.send(Queued { request, job }).map_err(|_| "the edit proof worker stopped".into())
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.remote.enqueue(Queued { request, job }, &self.answered)
        }
    }

    pub(crate) fn answers(&mut self) -> Vec<Answer> {
        #[cfg(target_arch = "wasm32")]
        self.remote.replay(&self.newest, &self.protected, &self.answered);
        self.answers.try_iter().collect()
    }
}

/// The browser's half of the scheduler. Each request's definition goes to the kernel
/// worker, which builds it and answers with its kernel calls. The page then runs the same
/// construction and proof as the desktop's worker threads do, on the replay kernel, so
/// revision tokens and adoption are unchanged. A rebuild that asks for something the
/// worker did not answer is never adopted: it is built once more from nothing on both
/// sides, and refused if that misses too.
#[cfg(target_arch = "wasm32")]
mod remote {
    use super::*;
    use crate::kernel_worker::{self, Job, SUPERSEDED, calls::Answered as Recorded};
    use soliddraft_kernel_api::Kernel;
    use std::collections::HashMap;
    use std::sync::Mutex;

    enum Reply {
        Constructed(Result<Vec<Recorded>, String>),
        Proved(Result<Vec<Recorded>, String>),
    }

    struct Waiting {
        part: Vec<u8>,
        /// The request's definition, unbuilt, sharing the page's feature builds.
        model: model::Document,
        level: soliddraft_tessellation::Level,
        construction: Option<ConstructionTask>,
        cold: bool,
        generation: u64,
    }

    #[derive(Default)]
    pub(super) struct Remote {
        replies: Arc<Mutex<Vec<(u64, Reply)>>>,
        waiting: HashMap<u64, Waiting>,
        previous: Option<Arc<model::Document>>,
    }

    fn decode(answer: Result<serde_json::Value, String>) -> Result<Vec<Recorded>, String> {
        answer.and_then(|answer| serde_json::from_value(answer).map_err(|error| error.to_string()))
    }

    impl Remote {
        pub(super) fn enqueue(&mut self, queued: Queued, answered: &Answered) -> Result<(), String> {
            let request = queued.request;
            let (proof, construction) = match prepare(queued) {
                Ok(tasks) => tasks,
                Err(answer) => {
                    let _ = answered.send(answer);
                    return Ok(());
                }
            };
            let (_, part) = proof.model.snapshot().encode().map_err(|error| error.to_string())?;
            let waiting = Waiting {
                part,
                model: proof.model,
                level: proof.level,
                construction,
                cold: false,
                generation: 0,
            };
            let ctx = answered.repaint.clone().unwrap_or_default();
            if let Some(task) = &waiting.construction
                && let Some(body) = task.baseline.body()
            {
                // Exactly the call `construct` makes, so its answer replays.
                let call = kernel_worker::calls::Call {
                    method: "construction_edit".into(),
                    args: serde_json::json!([body, task.edits, task.params]),
                };
                let replies = self.replies.clone();
                kernel_worker::submit(Vec::new(), &Job::Calls { calls: vec![call] }, "construction", &ctx, move |answer| {
                    replies.lock().expect("worker replies").push((request, Reply::Constructed(decode(answer))));
                })?;
            }
            let mut waiting = waiting;
            self.prove(request, &waiting, &ctx)?;
            // Read after sending: the first job starts the first worker.
            waiting.generation = kernel_worker::generation();
            self.waiting.insert(request, waiting);
            Ok(())
        }

        fn prove(&self, request: u64, waiting: &Waiting, ctx: &egui::Context) -> Result<(), String> {
            let job = Job::Prove {
                document: waiting.model.id(),
                level: waiting.level,
                budget: waiting.model.rebuild_cache_usage().1,
                cold: waiting.cold,
            };
            let slot = format!("prove:{:?}", waiting.model.id());
            let replies = self.replies.clone();
            kernel_worker::submit(waiting.part.clone(), &job, &slot, ctx, move |answer| {
                replies.lock().expect("worker replies").push((request, Reply::Proved(decode(answer))));
            })
        }

        /// Replays every answer that arrived, in arrival order.
        pub(super) fn replay(&mut self, newest: &AtomicU64, protected: &AtomicU64, answered: &Answered) {
            let replies = std::mem::take(&mut *self.replies.lock().expect("worker replies"));
            let ctx = answered.repaint.clone().unwrap_or_default();
            let kernel = kernel_worker::replay();
            for (request, reply) in replies {
                if !wanted(request, newest, protected) {
                    self.waiting.remove(&request);
                    continue;
                }
                let Some(mut waiting) = self.waiting.remove(&request) else {
                    continue;
                };
                match reply {
                    Reply::Constructed(result) => {
                        if let Some(task) = waiting.construction.take() {
                            match result {
                                Ok(recorded) => {
                                    kernel.learn(recorded);
                                    let _ = construct(task, answered);
                                }
                                Err(error) if error == SUPERSEDED => {}
                                Err(error) => {
                                    let _ = answered
                                        .send(Answer { request, kind: AnswerKind::Construction(Err(error)) });
                                }
                            }
                        }
                        self.waiting.insert(request, waiting);
                    }
                    Reply::Proved(Err(error)) if error == SUPERSEDED => self.retry(request, waiting, false, &ctx, answered),
                    Reply::Proved(Err(error)) => {
                        let _ = answered.send(Answer { request, kind: AnswerKind::Proved(Err(error)) });
                    }
                    Reply::Proved(Ok(_)) if waiting.generation != kernel_worker::generation() => {
                        // A new worker answered: the page's builds name another worker's bodies.
                        self.retry(request, waiting, true, &ctx, answered);
                    }
                    Reply::Proved(Ok(recorded)) => {
                        log::info!("DEBUG proof answer {request}: {} calls, cold {}", recorded.len(), waiting.cold);
                        kernel.learn(recorded);
                        let cold = waiting.cold;
                        let model = if cold { waiting.model.fork_unbuilt() } else { waiting.model.fork() };
                        let allowance = match kernel.reserve_proof_work() {
                            Ok(allowance) => allowance,
                            Err(error) => {
                                let _ = answered
                                    .send(Answer { request, kind: AnswerKind::Proved(Err(error.to_string())) });
                                continue;
                            }
                        };
                        let task = ProofTask { request, model, level: waiting.level, allowance };
                        let misses = kernel.misses();
                        let previous = if cold { None } else { self.previous.as_deref() };
                        let result = prove(task, newest, protected, previous);
                        if kernel.misses() != misses {
                            // These were the page's own calls, not queries to fetch.
                            kernel.take_missed();
                            if !cold {
                                self.retry(request, waiting, true, &ctx, answered);
                                continue;
                            }
                            let error = "The page's rebuild asked for kernel answers the worker did not give".to_owned();
                            let _ = answered.send(Answer { request, kind: AnswerKind::Proved(Err(error)) });
                            continue;
                        }
                        log::info!("DEBUG proved {request}: failure {:?} display {} error {:?}", result.failure, result.display.is_some(), result.display_error);
                        if result.failure.is_none() && !result.model.is_dirty() {
                            self.previous = Some(result.model.clone());
                        }
                        let _ = answered.send(Answer { request, kind: AnswerKind::Proved(Ok(result)) });
                    }
                }
            }
        }

        fn retry(&mut self, request: u64, mut waiting: Waiting, cold: bool, ctx: &egui::Context, answered: &Answered) {
            waiting.cold |= cold;
            if waiting.cold {
                self.previous = None;
            }
            match self.prove(request, &waiting, ctx) {
                Ok(()) => {
                    waiting.generation = kernel_worker::generation();
                    self.waiting.insert(request, waiting);
                }
                Err(error) => {
                    let _ = answered.send(Answer { request, kind: AnswerKind::Proved(Err(error)) });
                }
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.protected.store(u64::MAX, Ordering::Release);
        self.newest.store(u64::MAX, Ordering::Release);
    }
}
