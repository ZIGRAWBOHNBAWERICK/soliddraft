//! Whole-edge proof is an inspection of a part revision, not a rebuild or a display operation.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use serde::{Deserialize, Serialize};
use soliddraft_document::DocumentId;
use soliddraft_kernel_api::{
    BodyId, CurveKind, EdgeId, EdgeProofRun, FaceAccuracy, Interrupt, Kernel, MassProperties, SurfaceKind, Topology,
};
use soliddraft_model as model;

use crate::document::Document;

const WORK_LIMIT: u64 = 4_096;
const CANCELLED: &str = "Cancelled.";

type Answer<T> = Receiver<Result<T, String>>;

#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) struct Estimate {
    edges: usize,
    complex_faces: usize,
    work_hint: u64,
}

impl Estimate {
    fn heavy(self) -> bool {
        self.complex_faces != 0 || self.work_hint >= WORK_LIMIT / 2
    }
}

struct Body {
    key: u32,
    id: BodyId,
    topology: Topology,
}

fn bodies(doc: &model::Document) -> Result<Vec<Body>, String> {
    use model::FeatureStatus::{Blocked, Failed, RepairNeeded};
    if doc.is_dirty()
        || doc.statuses().iter().any(|(_, status)| matches!(status, Blocked(_) | Failed(_) | RepairNeeded(_)))
    {
        return Err("Finish rebuilding the part first.".into());
    }
    doc.live_bodies()
        .into_iter()
        .map(|(key, id)| {
            let topology = doc.kernel().topology(id).map_err(|error| error.to_string())?;
            if topology.edges.is_empty() {
                return Err("The part has a body with no edges.".into());
            }
            Ok(Body { key, id, topology })
        })
        .collect()
}

pub(crate) fn estimate(doc: &model::Document) -> Result<Estimate, String> {
    let mut estimate = Estimate { edges: 0, complex_faces: 0, work_hint: 0 };
    for body in bodies(doc)? {
        estimate.edges += body.topology.edges.len();
        // A screening heuristic, not a promise about duration or subdivision depth.
        estimate.work_hint = estimate.work_hint.saturating_add(body.topology.edges.len() as u64 * 2);
        for face in body.topology.faces {
            let factor = if matches!(face.accuracy, FaceAccuracy::Fitted { .. })
                || matches!(face.surface, SurfaceKind::Spline | SurfaceKind::Other)
            {
                estimate.complex_faces += 1;
                64
            } else if face.surface == SurfaceKind::Plane {
                0
            } else {
                1
            };
            estimate.work_hint = estimate.work_hint.saturating_add(face.edges.len() as u64 * factor);
        }
    }
    if estimate.edges == 0 {
        return Err("The part has no edges.".into());
    }
    Ok(estimate)
}

#[derive(Serialize, Deserialize)]
struct BodyReport {
    body: u32,
    edges: Vec<(EdgeId, Option<f64>)>,
    surfaces: BTreeMap<SurfaceKind, usize>,
    curves: BTreeMap<CurveKind, usize>,
    vertices: usize,
    vertex_valence: BTreeMap<usize, usize>,
    mass: Result<MassProperties, String>,
    density_kg_m3: Option<f64>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Report {
    bodies: Vec<BodyReport>,
    unproved_edges: usize,
    wall_seconds: f64,
    cpu_ns: Option<u64>,
    threads: usize,
    work_used: u64,
}

impl Report {
    fn fully_proven(&self) -> bool {
        self.unproved_edges == 0 && !self.bodies.is_empty()
    }
}

pub(crate) fn kernel_prover(
    kernel: &dyn Kernel,
    body: BodyId,
    work_limit: u64,
    interrupt: &Interrupt,
) -> Result<EdgeProofRun, String> {
    kernel.prove_edges(body, work_limit, interrupt).map_err(|error| error.to_string())
}

pub(crate) fn prove(
    doc: &model::Document,
    interrupt: &Interrupt,
    prover: impl Fn(&dyn Kernel, BodyId, u64, &Interrupt) -> Result<EdgeProofRun, String>,
) -> Result<Report, String> {
    interrupt.run(|| {
        let started = web_time::Instant::now();
        let kernel = &**doc.kernel();
        let mut report = Report {
            bodies: Vec::new(),
            unproved_edges: 0,
            wall_seconds: 0.0,
            cpu_ns: Some(0),
            threads: 0,
            work_used: 0,
        };
        for Body { key, id, topology } in bodies(doc)? {
            check_cancel(interrupt)?;
            let work_left = WORK_LIMIT - report.work_used;
            let mut bounds = HashMap::new();
            if work_left != 0 {
                let run = prover(kernel, id, work_left, interrupt)?;
                check_cancel(interrupt)?;
                if run.work_used > work_left {
                    return Err("The calculation exceeded its work limit.".into());
                }
                report.work_used += run.work_used;
                report.cpu_ns = report.cpu_ns.zip(run.cpu_ns).map(|(total, cpu)| total.saturating_add(cpu));
                report.threads = report.threads.max(run.threads_used);
                for edge in run.edges {
                    let bound = edge.bound_mm.filter(|b| b.is_finite() && *b >= 0.0);
                    if bounds.insert(edge.edge, bound).is_some() {
                        return Err("The kernel reported an edge twice.".into());
                    }
                }
            }

            let mut surfaces = BTreeMap::new();
            for face in &topology.faces {
                *surfaces.entry(face.surface).or_insert(0) += 1;
            }
            let mut curves = BTreeMap::new();
            let edges = topology
                .edges
                .iter()
                .map(|edge| {
                    *curves.entry(edge.curve).or_insert(0) += 1;
                    let bound = bounds.remove(&edge.id).flatten();
                    report.unproved_edges += usize::from(bound.is_none());
                    (edge.id, bound)
                })
                .collect();
            if !bounds.is_empty() {
                return Err("The kernel reported an edge outside the part.".into());
            }
            let mut vertex_valence = BTreeMap::new();
            for vertex in &topology.vertices {
                *vertex_valence.entry(vertex.edges.len()).or_insert(0) += 1;
            }
            let (mass, work) = soliddraft_kernel_catallax::with_request_allowance(super::MASS_WORK_LIMIT, || {
                kernel.mass_properties(id)
            });
            check_cancel(interrupt)?;
            let mass = if work.exhausted {
                Err("Mass properties exceeded their work limit.".into())
            } else {
                mass.map_err(|error| error.to_string())
            };
            report.bodies.push(BodyReport {
                body: key,
                edges,
                surfaces,
                curves,
                vertices: topology.vertex_count,
                vertex_valence,
                mass,
                density_kg_m3: doc.body_material(key, doc.active_configuration()).and_then(|m| m.density_kg_m3),
            });
        }
        report.wall_seconds = started.elapsed().as_secs_f64();
        Ok(report)
    })
}

fn check_cancel(interrupt: &Interrupt) -> Result<(), String> {
    if interrupt.stopped() { Err(CANCELLED.into()) } else { Ok(()) }
}

/// Kernel bodies are immutable. A new revision, an authored edit, or a new open copy
/// invalidates both the report and the verdict, even when undo reuses the same body.
#[derive(Clone, Copy, PartialEq)]
struct Revision {
    document: DocumentId,
    revision: u64,
    edits: u64,
}

impl From<&Document> for Revision {
    fn from(doc: &Document) -> Self {
        Self { document: doc.instance(), revision: doc.revision(), edits: doc.edits }
    }
}

enum Phase {
    Estimating(Answer<Estimate>),
    Confirm(Estimate),
    Proving(Answer<Report>),
}

struct Active {
    phase: Phase,
    #[cfg(not(target_arch = "wasm32"))]
    interrupt: Interrupt,
    #[cfg(not(target_arch = "wasm32"))]
    model: std::sync::Arc<model::Document>,
    #[cfg(target_arch = "wasm32")]
    worker: crate::kernel_worker::ProofWorker,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Active {
    fn drop(&mut self) {
        self.interrupt.stop();
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
type TestProver =
    std::sync::Arc<dyn Fn(&dyn Kernel, BodyId, u64, &Interrupt) -> Result<EdgeProofRun, String> + Send + Sync>;

#[derive(Default)]
pub(crate) struct State {
    revision: Option<Revision>,
    active: Option<Active>,
    report: Option<Report>,
    report_open: bool,
    message: Option<String>,
    #[cfg(all(test, not(target_arch = "wasm32")))]
    prover: Option<TestProver>,
}

impl State {
    fn current(&self, doc: &Document) -> bool {
        self.revision == Some(Revision::from(doc))
    }

    fn start(&mut self, doc: &Document, ctx: &egui::Context) -> Result<(), String> {
        self.revision = Some(doc.into());
        if !doc.proof_state().current_proved_model() || doc.model().live_bodies().is_empty() {
            return Err("Select a part with a completed build.".into());
        }
        self.message = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let interrupt = Interrupt::new();
            let model = std::sync::Arc::new(doc.model().fork());
            let answer = native_job(&model, ctx, &interrupt, |doc, _| estimate(doc))?;
            self.active = Some(Active { phase: Phase::Estimating(answer), interrupt, model });
        }
        #[cfg(target_arch = "wasm32")]
        {
            let (_, part) = doc.model().snapshot().encode().map_err(|error| error.to_string())?;
            let worker = crate::kernel_worker::ProofWorker::new(part, doc.kernel().clone())?;
            let answer = browser_job(&worker, ctx, true)?;
            self.active = Some(Active { phase: Phase::Estimating(answer), worker });
        }
        Ok(())
    }

    fn confirm(&mut self, ctx: &egui::Context) -> Result<(), String> {
        let Some(active) = self.active.as_mut() else { return Ok(()) };
        #[cfg(not(target_arch = "wasm32"))]
        let answer = {
            #[cfg(test)]
            let fake = self.prover.clone();
            native_job(&active.model, ctx, &active.interrupt, move |doc, interrupt| {
                #[cfg(test)]
                if let Some(fake) = fake {
                    return prove(doc, interrupt, &*fake);
                }
                prove(doc, interrupt, kernel_prover)
            })?
        };
        #[cfg(target_arch = "wasm32")]
        let answer = browser_job(&active.worker, ctx, false)?;
        active.phase = Phase::Proving(answer);
        Ok(())
    }

    fn finish(&mut self, message: String) {
        self.active = None;
        self.message = Some(message);
    }

    /// Accept answers only for the revision that started the job.
    pub(crate) fn poll(&mut self, doc: &Document, ctx: &egui::Context) {
        if !self.current(doc) {
            self.active = None;
            self.report = None;
            self.report_open = false;
            self.message = None;
            return;
        }
        match self.active.as_ref().map(|run| &run.phase) {
            Some(Phase::Estimating(answer)) => match try_answer(answer) {
                Some(Ok(estimate)) if estimate.heavy() => {
                    self.active.as_mut().unwrap().phase = Phase::Confirm(estimate);
                }
                Some(Ok(_)) => {
                    if let Err(error) = self.confirm(ctx) {
                        self.finish(error);
                    }
                }
                Some(Err(error)) => self.finish(error),
                None => {}
            },
            Some(Phase::Proving(answer)) => match try_answer(answer) {
                Some(Ok(report)) => {
                    self.active = None;
                    self.report = Some(report);
                }
                Some(Err(error)) => self.finish(error),
                None => {}
            },
            _ => {}
        }
    }

    pub(crate) fn controls(&mut self, ui: &mut egui::Ui, doc: &Document) {
        if let Some(report) = &self.report {
            ui.label(if report.fully_proven() {
                "Every edge has a precision bound"
            } else {
                "Some edges have no precision bound"
            });
            if ui.button("Open precision report").clicked() {
                self.report_open = true;
            }
        }
        if self.active.is_some() {
            ui.label(match self.active.as_ref().map(|run| &run.phase) {
                Some(Phase::Estimating(_)) => "Estimating the work…",
                Some(Phase::Confirm(_)) => "Waiting for confirmation…",
                _ => "Measuring edge precision…",
            });
            if ui.button("Cancel").clicked() {
                self.finish(CANCELLED.into());
            }
        } else {
            if ui.add_enabled(doc.proof_state().current_proved_model(), egui::Button::new("Show precision")).clicked()
                && let Err(error) = self.start(doc, ui.ctx())
            {
                self.finish(error);
            }
            if let Some(message) = &self.message {
                ui.label(message);
            }
        }
    }

    pub(crate) fn dialogs(&mut self, doc: &Document, ctx: &egui::Context) {
        if !self.current(doc) {
            return;
        }
        if let Some(Phase::Confirm(estimate)) = self.active.as_ref().map(|run| &run.phase) {
            let mut proceed = false;
            let mut cancel = false;
            let modal = egui::Modal::new(egui::Id::new("confirm_whole_edge_proof")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.heading("Show precision for this complex part?");
                ui.label("This may take a long time and use a lot of computing power.");
                ui.label(format!(
                    "{} edges · {} complex faces · limit: {WORK_LIMIT} work units",
                    estimate.edges, estimate.complex_faces
                ));
                ui.horizontal(|ui| {
                    cancel = ui.button("Cancel").clicked();
                    proceed = ui.button("Continue").clicked();
                });
            });
            if proceed {
                if let Err(error) = self.confirm(ctx) {
                    self.finish(error);
                }
            } else if cancel || modal.should_close() {
                self.finish(CANCELLED.into());
            }
        }
        if self.report_open
            && let Some(report) = &self.report
        {
            egui::Window::new("Part precision report")
                .open(&mut self.report_open)
                .default_width(480.0)
                .default_height(440.0)
                .vscroll(true)
                .show(ctx, |ui| show_report(ui, report));
        }
    }
}

fn try_answer<T>(answer: &Answer<T>) -> Option<Result<T, String>> {
    match answer.try_recv() {
        Ok(answer) => Some(answer),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => Some(Err("The precision worker stopped.".into())),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn native_job<T: Send + 'static>(
    model: &std::sync::Arc<model::Document>,
    ctx: &egui::Context,
    interrupt: &Interrupt,
    work: impl FnOnce(&model::Document, &Interrupt) -> Result<T, String> + Send + 'static,
) -> Result<Answer<T>, String> {
    let model = model.clone();
    let ctx = ctx.clone();
    let interrupt = interrupt.clone();
    let (sent, answer) = mpsc::channel();
    std::thread::Builder::new()
        .name("part-proof".into())
        .spawn(move || {
            let result = interrupt.run(|| check_cancel(&interrupt).and_then(|()| work(&model, &interrupt)));
            let _ = sent.send(result);
            ctx.request_repaint();
        })
        .map_err(|error| format!("The precision worker could not start: {error}"))?;
    Ok(answer)
}

#[cfg(target_arch = "wasm32")]
fn browser_job<T: serde::de::DeserializeOwned + 'static>(
    worker: &crate::kernel_worker::ProofWorker,
    ctx: &egui::Context,
    estimate: bool,
) -> Result<Answer<T>, String> {
    let (sent, answer) = mpsc::channel();
    worker.submit(estimate, ctx, move |value| {
        let _ = sent.send(value.and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string())));
    })?;
    Ok(answer)
}

fn show_report(ui: &mut egui::Ui, report: &Report) {
    ui.heading(if report.fully_proven() {
        "Every edge has a precision bound"
    } else {
        "Some edges have no precision bound"
    });
    ui.label("Bounds measure the distance from the whole edge curve to its incident surfaces.");
    ui.label(format!("Wall time: {:.3} s", report.wall_seconds));
    if let Some(cpu) = report.cpu_ns {
        ui.label(format!("CPU time: {:.3} s", cpu as f64 * 1e-9));
    } else {
        ui.label("CPU time: unavailable on this platform");
    }
    ui.label(format!("Threads used (peak): {}", report.threads));
    ui.label(format!("Work units: {} / {WORK_LIMIT}", report.work_used));
    for body in &report.bodies {
        ui.separator();
        if report.bodies.len() > 1 {
            ui.heading(format!("Body {}", body.body));
        }
        ui.label(format!("Faces: {}", body.surfaces.values().sum::<usize>()));
        counts(ui, &body.surfaces);
        ui.label(format!("Edges: {}", body.edges.len()));
        counts(ui, &body.curves);
        ui.label(format!("Vertices: {}", body.vertices));
        for (valence, count) in &body.vertex_valence {
            ui.label(format!("{valence} incident edges: {count}"));
        }
        ui.separator();
        ui.strong("Mass properties");
        match &body.mass {
            Ok(mass) => {
                ui.label(format!("Volume: {:.6} mm³", mass.volume));
                ui.label(format!("Surface area: {:.6} mm²", mass.area));
                let c = mass.centroid;
                ui.label(format!("Centroid: ({:.6}, {:.6}, {:.6}) mm", c.x, c.y, c.z));
                if let Some(density) = body.density_kg_m3 {
                    let mass = soliddraft_materials::mass::at_density(mass, density);
                    ui.label(format!("Mass: {:.6} kg", mass.mass_kg));
                    tensor(ui, mass.inertia_kg_mm2, "kg·mm²");
                } else {
                    ui.label("Mass: unknown (no material density)");
                    tensor(ui, mass.inertia, "mm⁵ at unit density");
                }
            }
            Err(error) => {
                ui.label(error);
            }
        }
        ui.separator();
        ui.strong("Edge results");
        for (edge, bound) in &body.edges {
            if let Some(bound) = bound {
                // Debug formatting round-trips the conservative value without rounding it down to a display precision.
                ui.label(format!("Edge {}: ≤ {bound:?} mm", edge.raw()));
            } else {
                ui.label(format!("Edge {}: no bound", edge.raw()));
            }
        }
    }
}

fn counts<K: std::fmt::Debug>(ui: &mut egui::Ui, kinds: &BTreeMap<K, usize>) {
    for (kind, count) in kinds {
        ui.label(format!("{kind:?}: {count}"));
    }
}

fn tensor(ui: &mut egui::Ui, tensor: Option<[[f64; 3]; 3]>, units: &str) {
    if let Some(tensor) = tensor {
        ui.label(format!("Inertia about centroid ({units})"));
        for [x, y, z] in tensor {
            ui.monospace(format!("{x:>14.6e} {y:>14.6e} {z:>14.6e}"));
        }
    } else {
        ui.label("Inertia about centroid: unavailable");
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
