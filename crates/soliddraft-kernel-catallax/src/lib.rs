//! Catallax's application entry behind SolidDraft's existing kernel interface.
//!
//! Profiles are deferred recipe fragments, not sheet geometry. Each solid owns an
//! immutable Catallax application build. Legacy history is derived from Catallax's
//! origin sets; display estimates never decide topology or operation success.

mod approximate;
mod assembly_measure;
mod body_relation;
mod checks;
mod construction;
mod convert;
mod diagnostics;
mod distance;
mod drawing_queries;
mod edge_control;
mod feature_pattern;
mod field_preview;
mod frames;
mod identity;
mod iges;
mod import;
mod lattice;
mod loft;
mod measure;
mod native;
mod scan;
mod shell;
mod step;
mod sweep;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use catallax_app::{Error, EvaluatedApplicationBody, EvaluatedApplicationOutcome, NumericalApplicationBody};

mod evaluated;
mod handover;
#[cfg(feature = "numerical-stand-in")]
mod stand_in;
use catallax_mesh::{MeshOptions, PreviewMesh, PreviewOptions, preview_mesh_body};
use catallax_numbers::{Decider, Rational};
use catallax_recipe::StepId;
pub(crate) use evaluated::{AppBody, Exact};
use serde_json::{Value, json};
use soliddraft_kernel_api::*;

pub use diagnostics::{APP_REQUEST_WORK, APP_WORK_TARGET, RequestWork, with_request_allowance};

/// The pinned Catallax application entry revision.
pub const CATALLAX_REVISION: &str = "9566aa24011d9ba3dde38d717f66be6be97c5015";

/// An in-process Catallax backend. There is no global kernel state or kernel thread.
#[derive(Default)]
pub struct CatallaxKernel {
    state: Mutex<State>,
    diagnostics: Option<diagnostics::Diagnostics>,
}

#[derive(Default)]
struct State {
    next: u64,
    bodies: BTreeMap<BodyId, Stored>,
    fields: BTreeMap<BodyId, Arc<lattice::Field>>,
    capture: bool,
    questions: Vec<Value>,
    /// Imported STEP sources by SHA-256. Catallax reads them again for every build
    /// that continues an imported body, even when the import itself is reused.
    files: BTreeMap<String, Arc<[u8]>>,
    /// Builds this kernel made since [`Kernel::keep_builds`], by recipe key, as bytes.
    kept: Option<Vec<Kept>>,
    /// Every body stored while keeping builds, so a hand-over finds the topology of one
    /// a later feature released.
    kept_solids: Vec<Arc<Solid>>,
    /// Builds another kernel handed over, by recipe key; a build of the same
    /// definition takes one instead of evaluating it.
    offered: BTreeMap<String, handover::Offer>,
    /// Bodies Catallax evaluated here rather than took from an offer.
    fresh: u64,
    /// Topologies computed here rather than taken from an offer.
    fresh_topologies: u64,
    /// Recipe keys whose topology has been handed over already.
    sent_topologies: std::collections::BTreeSet<String>,
    /// Early display requests answered by sampling here rather than from an offer.
    fresh_previews: u64,
    /// The interface id this kernel gave each entity another kernel's id named, for
    /// builds adopted unread.
    handed_ids: BTreeMap<(u64, u64), u64>,
    /// How many offered builds were read from their bytes.
    offer_reads: Arc<std::sync::atomic::AtomicU64>,
    /// Operations whose exact results this kernel retains as numerical bodies, standing
    /// in for Catallax until it returns numerical results itself.
    stand_in: Vec<OpKind>,
}

/// A build kept for a hand-over: its recipe key, its bytes, and what adopting it unread
/// needs, once the body it built is stored.
type Kept = (String, Vec<u8>, Option<handover::HandedIdentity>);

struct Stored {
    steps: BTreeMap<u64, Value>,
    result: String,
    history: Vec<HistoryRecord>,
    solid: Option<Arc<Solid>>,
    /// A body Catallax built within a stated tolerance, which has no exact solid.
    approximate: Option<Arc<approximate::Approximate>>,
}

struct Solid {
    app: AppBody,
    /// The recipe key of this build, for a hand-over of its topology.
    key: String,
    /// Early display meshes another kernel sampled for this build, by display key.
    handed_previews: handover::Previews,
    /// The kernel instance that handed this build over, if one did.
    source: Option<u64>,
    ids: convert::Ids,
    display: MeshCache<catallax_mesh::Mesh>,
    print: MeshCache<PrintMesh>,
    /// `None` where Catallax refused to sample a preview, so it is not asked again.
    previews: MeshCache<Option<PreviewMesh>>,
    rendering: MeshCache<catallax_numbers::OperationResult<catallax_mesh::DisplayMesh>>,
    topology: OnceLock<KernelResult<Topology>>,
    measured: OnceLock<KernelResult<Topology>>,
    mass: OnceLock<KernelResult<MassProperties>>,
}

/// A display key as text, which a hand-over carries between kernels.
fn preview_key(key: &DisplayKey) -> String {
    format!("{key:?}")
}

/// The edges of an early display mesh, named in this kernel's ids.
fn preview_edges(mesh: &PreviewMesh, ids: &convert::Ids) -> Vec<EdgePolyline> {
    mesh.edges
        .iter()
        .map(|edge| EdgePolyline {
            edge: ids.edges[&edge.edge],
            points: edge.vertices.iter().map(|v| mesh.positions[*v].into()).collect(),
        })
        .collect()
}

/// Every display control of a mesh request.
type DisplayKey = (Rational, Option<Rational>, Option<Rational>, Option<Rational>);

fn display_key(options: &MeshOptions) -> DisplayKey {
    (
        options.chord_tolerance.clone(),
        options.angular_deviation.clone(),
        options.max_edge_length.clone(),
        options.max_aspect_ratio.clone(),
    )
}

/// One body's meshes of one kind by their controls, the most recently asked last.
struct MeshCache<T>(Mutex<VecDeque<Arc<CachedMesh<T>>>>);

struct CachedMesh<T> {
    key: DisplayKey,
    value: Mutex<Option<Arc<T>>>,
}

impl<T> MeshCache<T> {
    fn new() -> Self {
        Self(Mutex::new(VecDeque::new()))
    }

    /// The values computed so far, with their keys.
    fn computed(&self) -> Vec<(DisplayKey, Arc<T>)> {
        let entries: Vec<_> = self.0.lock().map(|cached| cached.iter().cloned().collect()).unwrap_or_default();
        entries.into_iter().filter_map(|entry| Some((entry.key.clone(), entry.value.lock().ok()?.clone()?))).collect()
    }

    fn get(&self, key: DisplayKey, make: impl FnOnce() -> KernelResult<T>) -> KernelResult<Arc<T>> {
        let entry = {
            let mut cached =
                self.0.lock().map_err(|_| KernelError::Unavailable("Catallax display cache lock poisoned".into()))?;
            let entry = if let Some(i) = cached.iter().position(|entry| entry.key == key) {
                cached.remove(i).expect("the cached controls exist")
            } else {
                // Three viewport levels, the default mesh and a drawing tolerance coexist.
                const MESHES: usize = 5;
                if cached.len() == MESHES {
                    cached.pop_front();
                }
                Arc::new(CachedMesh { key, value: Mutex::new(None) })
            };
            cached.push_back(entry.clone());
            entry
        };
        // Only requests for these controls wait for their build. Background refinement
        // must not lock out a viewport or file query for an already available mesh.
        let mut value =
            entry.value.lock().map_err(|_| KernelError::Unavailable("Catallax mesh cache lock poisoned".into()))?;
        if let Some(mesh) = value.as_ref() {
            return Ok(mesh.clone());
        }
        let mesh = Arc::new(make()?);
        *value = Some(mesh.clone());
        Ok(mesh)
    }

    /// Visits every finished mesh, skipping any still being made.
    fn each(&self, mut visit: impl FnMut(&T)) {
        if let Ok(cached) = self.0.lock() {
            for entry in cached.iter() {
                if let Ok(value) = entry.value.try_lock()
                    && let Some(mesh) = value.as_ref()
                {
                    visit(mesh);
                }
            }
        }
    }
}

impl Solid {
    fn has_splines(&self) -> bool {
        if let Some(handed) = self.app.unread() {
            return handed.splines;
        }
        let built = self.app.checkpoint.built();
        built.model.faces_of_body(built.result).iter().any(|face| {
            matches!(built.model.geom().surface(built.model.face(*face).surface), catallax_shapes::Surface::BSpline(_))
        })
    }

    fn rendering(&self) -> KernelResult<Arc<catallax_numbers::OperationResult<catallax_mesh::DisplayMesh>>> {
        use catallax_numbers::{Answer, OperationResult, Refusal};
        self.rendering.get(display_key(&MeshOptions::default()), || {
            let run = || numerical("display_result", |dec| match self.app.display_result(&MeshOptions::default(), dec) {
                Ok(OperationResult::Refused(refusal)) => Err(refusal),
                Ok(answer) => Ok(Answer::exact(answer)),
                Err(Error::Display(refusal)) => Err(refusal),
                Err(error) => Err(Refusal::new(error.code(), error.to_string())),
            });
            let (result, used, limit) = if catallax_numbers::operation_budget_active() {
                let result = run();
                (result, catallax_numbers::current_operation_work().used, diagnostics::request_limit())
            } else {
                let (result, work) = catallax_numbers::with_operation_budget(APP_REQUEST_WORK, run);
                (result, work.used, APP_REQUEST_WORK)
            };
            result.map(|answer| answer.value).map_err(|error| {
                if matches!(error.code, "mesh.spline-display-budget" | "numbers.operation-budget" | "numbers.budget-exhausted" | "application.work-limit") {
                    KernelError::DisplayLimit { used, limit }
                } else {
                    KernelError::Unsupported("Spline display bound (SD-DISPLAY-02): A measured deviation within 0.01 mm could not be established over the entire boundary. The existing part is retained".into())
                }
            })
        })
    }

    fn rendering_mesh(&self) -> KernelResult<Mesh> {
        use catallax_numbers::OperationResult;
        let answer = self.rendering()?;
        let (display, evidence) = match answer.as_ref() {
            OperationResult::Proven(answer) => (&answer.value, None),
            OperationResult::Bounded(answer) => (answer.value(), Some(answer.evidence())),
            OperationResult::Refused(refusal) => return Err(query_error("display_result", refusal.clone())),
        };
        if display.geometry.positions.iter().flatten().any(|value| !(*value as f32).is_finite()) {
            return Err(KernelError::Unsupported("The display coordinates cannot be represented on screen".into()));
        }
        let mut mesh = convert::preview(&display.geometry, &self.ids);
        let rounding = display
            .geometry
            .positions
            .iter()
            .map(|point| {
                point
                    .iter()
                    .fold(0.0_f64, |sum, value| (sum + (*value - f64::from(*value as f32)).abs().next_up()).next_up())
            })
            .fold(0.0_f64, f64::max);
        let rounding = Rational::from_f64_exact(rounding).ok_or_else(|| {
            KernelError::Unsupported("The display coordinates cannot be represented on screen".into())
        })?;
        let bound = &display.measurement.maximum_deviation + &rounding;
        let level = MeshOptions::default().chord_tolerance;
        if bound > level {
            return Err(KernelError::Unsupported(
                "Spline display bound (SD-DISPLAY-03): The measured deviation exceeds 0.01 mm after screen-coordinate rounding. The existing part is retained".into(),
            ));
        }
        if let Some(evidence) = evidence {
            let mut measured = bound.to_f64_nearest();
            if Rational::from_f64_exact(measured).is_some_and(|value| value < bound) {
                measured = measured.next_up();
            }
            let definition = evidence.definition();
            mesh.bounded = Some(DisplayBound {
                kind: evidence.rule().case_kind.into(),
                rule: evidence.rule().name.into(),
                level_mm: evidence.level().to_f64_nearest(),
                measured_mm: measured,
                method: format!("{}; outward L1 bound for f32 screen-coordinate rounding", display.measurement.method),
                definition: json!({"revision":definition.revision,"representation":definition.representation,
                    "target":definition.target,"scope":format!("{:?}", definition.scope)})
                .to_string(),
            });
        } else {
            mesh.unproved = false;
        }
        if let Some(bound) = &mesh.bounded {
            bound.validate().map_err(KernelError::Unsupported)?;
        }
        Ok(mesh)
    }

    /// The certified mesh for these controls, for exports and proven results.
    fn display(&self, options: &MeshOptions, query: &str) -> KernelResult<Arc<catallax_mesh::Mesh>> {
        let key = display_key(options);
        let default = key == display_key(&MeshOptions::default());
        self.display.get(key, || {
            if default {
                return self.app.display(options, &mut Decider::default()).map(|answer| answer.value).map_err(
                    |error| match error {
                        Error::Display(refusal) => query_error(query, refusal),
                        other => KernelError::QueryFailed { query: query.into(), code: 0, message: other.to_string() },
                    },
                );
            }
            let native = if options.max_edge_length.is_some() {
                let built = self.app.checkpoint.built();
                let bounds = numerical("display_bounding_box", |dec| {
                    catallax_measure::bounding_box(
                        &built.model,
                        built.result,
                        &catallax_measure::Accuracy::default(),
                        dec,
                    )
                })
                .map_err(|e| query_error(query, e))?
                .value
                .ok_or_else(|| KernelError::InvalidInput("an empty body has no display bounds".into()))?;
                let extent = edge_control::display_extent(&bounds.min, &bounds.max, &options.chord_tolerance)?;
                edge_control::native_options(options, extent)?
            } else {
                options.clone()
            };
            let mesh = numerical(query, |dec| {
                self.app.display(&native, dec).map_err(|error| match error {
                    Error::Display(refusal) => refusal,
                    other => catallax_numbers::Refusal::new(other.code(), other.to_string()),
                })
            })
            .map_err(|e| query_error(query, e))?
            .value;
            edge_control::check(options, &mesh)?;
            Ok(mesh)
        })
    }

    /// An unproved mesh for the screen, sampled to the same targets with no claimed
    /// bounds. Catallax keeps it apart from the certified mesh and never exports it.
    /// `None` when Catallax refuses to sample one; the operation log keeps the refusal.
    fn preview(&self, options: &MeshOptions, query: &str) -> KernelResult<Arc<Option<PreviewMesh>>> {
        self.previews.get(display_key(options), || {
            let built = self.app.checkpoint.built();
            let mut decider = Decider::default();
            let span = catallax_numbers::OperationSpan::begin("brep", &query, query, &decider);
            let result = preview_mesh_body(&built.model, built.result, &PreviewOptions::from(options), &mut decider);
            span.finish(&decider, result.as_ref().err(), None);
            Ok(result.ok())
        })
    }
}

impl State {
    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    fn get(&self, body: BodyId) -> KernelResult<&Stored> {
        self.bodies.get(&body).ok_or(KernelError::UnknownBody(body))
    }

    /// The immutable sources that the import steps among `steps` read.
    fn sources(&self, steps: &BTreeMap<u64, Value>) -> KernelResult<BTreeMap<String, Arc<[u8]>>> {
        steps
            .values()
            .filter(|step| {
                matches!(step["op"].as_str(), Some("import_step" | "import_iges" | "import_mesh" | "scan_fit"))
            })
            .map(|step| {
                let sha = step["file"]["sha256"].as_str().unwrap_or_default();
                let bytes = self.files.get(sha).ok_or_else(|| {
                    KernelError::InvalidInput("the imported file of this body is no longer loaded".into())
                })?;
                Ok((sha.to_owned(), bytes.clone()))
            })
            .collect()
    }

    fn solid(&self, body: BodyId) -> KernelResult<Arc<Solid>> {
        if self.fields.contains_key(&body) {
            return Err(terminal_field(body));
        }
        let stored = self.get(body)?;
        if stored.approximate.is_some() {
            return Err(approximate::not_yet());
        }
        stored.solid.clone().ok_or_else(|| gap("Profile sheet queries"))
    }

    fn step(&self, body: BodyId) -> KernelResult<StepId> {
        Ok(StepId::new(self.get(body)?.result.clone()).expect("generated step ids are valid"))
    }
}

impl CatallaxKernel {
    /// Starts an empty backend with its own body arena.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many feature bodies Catallax has evaluated in this kernel, not counting builds
    /// taken from an offered hand-over.
    pub fn fresh_builds(&self) -> u64 {
        self.state().map(|state| state.fresh).unwrap_or_default()
    }

    /// How many body topologies this kernel computed, not counting ones taken from an
    /// offered hand-over.
    pub fn fresh_topologies(&self) -> u64 {
        self.state().map(|state| state.fresh_topologies).unwrap_or_default()
    }

    /// How many offered builds this kernel read from their bytes.
    pub fn bodies_read(&self) -> u64 {
        self.state().map(|state| state.offer_reads.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or_default()
    }

    /// How many early display requests this kernel answered without a handed-over mesh.
    pub fn fresh_previews(&self) -> u64 {
        self.state().map(|state| state.fresh_previews).unwrap_or_default()
    }

    /// Enables synchronous per-call evidence and deterministic work accounting.
    pub fn with_diagnostics(max_work: u64) -> Self {
        Self::capture(Some(max_work))
    }

    /// Captures evidence without imposing a work allowance on normal application commands.
    pub fn with_operation_log() -> Self {
        Self::capture(None)
    }

    fn capture(max_work: Option<u64>) -> Self {
        Self {
            state: Mutex::new(State { capture: true, ..State::default() }),
            diagnostics: Some(diagnostics::Diagnostics::new(max_work)),
        }
    }

    /// A backend whose builds of `ops` come back as numerical bodies, as Catallax will
    /// return rolling-ball fillets. It stands in for Catallax's numerical results in
    /// tests until Catallax returns them itself.
    #[cfg(feature = "numerical-stand-in")]
    pub fn with_numerical_stand_in(ops: &[OpKind]) -> Self {
        Self { state: Mutex::new(State { stand_in: ops.to_vec(), ..State::default() }), diagnostics: None }
    }

    /// Takes completed kernel calls and their portable recipe questions.
    pub fn take_diagnostics(&self) -> KernelResult<Value> {
        let mut state = self.state()?;
        let questions = std::mem::take(&mut state.questions);
        Ok(json!({"calls": self.diagnostics.as_ref().map(|d| d.take()).unwrap_or_default(),
            "questions":questions}))
    }

    /// The retained definition, using native recipes for fields and path sweeps.
    pub fn body_recipe(&self, body: BodyId) -> KernelResult<Value> {
        if let Some(field) = self.field(body)? {
            return Ok(field.recipe());
        }
        let state = self.state()?;
        let stored = state.get(body)?;
        let recipe = json!({"format":"soliddraft-recipe/1", "name":"SolidDraft",
            "steps":stored.steps.values().collect::<Vec<_>>(), "result":stored.result});
        let swept = stored.steps.values().any(|s| s["op"] == "path_sweep");
        // Retained-prefix callers read ordinary native wrappers in the legacy envelope.
        if swept
            || stored.steps.values().any(|s| {
                matches!(
                    s["op"].as_str(),
                    Some(
                        "scan_fit"
                            | "import_step"
                            | "import_iges"
                            | "import_mesh"
                            | "loft"
                            | "native_profile"
                            | "feature_pattern"
                    )
                )
            })
        {
            let op = stored.history.last().map_or(OpKind::ImportStep, |record| record.op);
            return serde_json::to_value(converted_recipe(&recipe, op)?.recipe)
                .map_err(|e| KernelError::InvalidInput(e.to_string()));
        }
        Ok(recipe)
    }

    fn observe<T>(&self, operation: &str, run: impl FnOnce() -> KernelResult<T>) -> KernelResult<T> {
        match &self.diagnostics {
            Some(d) => d.observe(operation, run),
            None => run(),
        }
    }

    fn observe_export<T>(&self, operation: &str, run: impl FnOnce() -> KernelResult<T>) -> KernelResult<T> {
        match &self.diagnostics {
            Some(d) => d.observe_export(operation, run),
            None => run(),
        }
    }

    /// Builds a body from `inputs` and one more step. The Catallax build runs without the
    /// body arena lock, so other threads keep reading and building bodies meanwhile.
    fn operation(
        &self,
        state: MutexGuard<'_, State>,
        tag: &OpTag,
        op: OpKind,
        inputs: &[BodyId],
        step: Value,
    ) -> KernelResult<OpResult> {
        self.operation_reading(state, tag, op, inputs, &[], &[], step)
    }

    /// Like [`CatallaxKernel::operation`], also reading the steps of `context` bodies
    /// whose faces the operation only looks at, so their history stays out of the result's.
    #[allow(clippy::too_many_arguments)]
    fn operation_reading(
        &self,
        state: MutexGuard<'_, State>,
        tag: &OpTag,
        op: OpKind,
        inputs: &[BodyId],
        context: &[BodyId],
        selected_edges: &[EdgeId],
        step: Value,
    ) -> KernelResult<OpResult> {
        let (mut state, evaluated) = self.evaluate(state, op, inputs, context, step)?;
        let Evaluated { app, key, handed, names, steps, mut history, parents, seq, result } = evaluated;
        // Catallax names faces by recipe step; the interface uses the caller's tag.
        let mut tags: BTreeMap<String, OpTag> = names
            .into_iter()
            .filter_map(|(name, id)| {
                let n: u64 = name.strip_prefix("op_")?.split('_').next()?.parse().ok()?;
                let tag = if n == seq { tag.clone() } else { history.get(&n)?.tag.clone() };
                Some((id.as_str().to_owned(), tag))
            })
            .collect();
        for (key, source) in &steps {
            if source["op"] == "scan_fit" {
                let prefix = format!("op_{key}");
                let fitted_tag = if *key == seq { tag.clone() } else { history[key].tag.clone() };
                let scan_ids: Vec<_> = tags
                    .keys()
                    .filter(|id| id.as_str() == prefix || id.starts_with(&format!("{prefix}_scan_")))
                    .cloned()
                    .collect();
                for id in scan_ids {
                    tags.insert(format!("{id}__scan_origin"), fitted_tag.clone());
                }
            }
        }
        let app = match app {
            Outcome::Exact(app) => app,
            Outcome::Approximate(outcome) => {
                let parent_ids = parents.iter().map(|p| &p.ids).collect::<Vec<_>>();
                if outcome.body().definition().faces.is_empty() {
                    return Err(KernelError::Unsupported(NOTHING_LEFT.into()));
                }
                let approximate = approximate::Approximate::new(outcome, &parent_ids, &tags, &mut state.next);
                let record = approximate.record(seq, tag, op, &result, &parent_ids, &tags);
                history.insert(seq, record.clone());
                let body = BodyId::from_raw(state.id());
                state.bodies.insert(
                    body,
                    Stored {
                        steps,
                        result,
                        history: history.into_values().collect(),
                        solid: None,
                        approximate: Some(Arc::new(approximate)),
                    },
                );
                return Ok(OpResult { body, record });
            }
        };
        if op == OpKind::Fillet
            && let Some(refusal) = stopped_short(&app, &result)
        {
            return Err(refusal);
        }
        if is_empty(&app) {
            if op == OpKind::Sweep {
                return Err(KernelError::InvalidInput("Sweep path must leave the profile plane".into()));
            }
            if op == OpKind::Loft {
                return Err(KernelError::InvalidInput(
                    "Loft profiles must enclose a solid. Move them onto different planes".into(),
                ));
            }
            return Err(KernelError::Unsupported(NOTHING_LEFT.into()));
        }
        let parent_ids = parents.iter().map(|p| &p.ids).collect::<Vec<_>>();
        let source = app.handed_source();
        // The handing kernel's lineage holds only where every parent came from the same
        // kernel instance; a parent built here or handed over by an earlier one has ids it
        // never saw, so the exact keys decide instead.
        let lineage = source.filter(|source| parents.iter().all(|parent| parent.source == Some(*source)));
        let identity = app.handed_identity().cloned();
        let (ids, record) = if let (Some(identity), Some(source)) = (&identity, lineage) {
            // A body adopted unread takes the handing kernel's ids and record.
            let (faces, edges, vertices) = identity.worker_ids();
            let state = &mut *state;
            let mut handed =
                convert::Handed { faces, edges, vertices, source, seen: &mut state.handed_ids, next: &mut state.next };
            let ids = convert::Ids::handed(&app, &parent_ids, &tags, &mut handed, selected_edges);
            let record = ids.handed_record(&identity.record, &state.handed_ids, source, seq, tag, &result);
            (ids, record)
        } else {
            let ids = convert::Ids::new(&app, &parent_ids, &tags, &mut state.next, selected_edges);
            let record = ids.record(seq, tag, op, &result, &parent_ids);
            // Later builds of the same source find these ids through the ones it handed.
            if let (Some(identity), Some(source)) = (&identity, source) {
                let (faces, edges, vertices) = identity.worker_ids();
                let (ours_f, ours_e, ours_v) = ids.by_index();
                let pairs = faces
                    .iter()
                    .filter_map(|(index, theirs)| Some((*theirs, ours_f.get(index)?.raw())))
                    .chain(edges.iter().filter_map(|(index, theirs)| Some((*theirs, ours_e.get(index)?.raw()))))
                    .chain(vertices.iter().filter_map(|(index, theirs)| Some((*theirs, ours_v.get(index)?.raw()))));
                for (theirs, ours) in pairs.collect::<Vec<_>>() {
                    state.handed_ids.insert((source, theirs), ours);
                }
            }
            (ids, record)
        };
        history.insert(seq, record.clone());
        let body = BodyId::from_raw(state.id());
        let display = MeshCache::new();
        let topology = OnceLock::new();
        let mut handed_previews = handover::Previews::new();
        // A handed-over topology and early display are renamed into this kernel's ids, so
        // neither is computed again.
        if let Some(handed) = handed {
            let (faces, edges, vertices) = ids.by_index();
            if let Some(renamed) = handed.renamed(&faces, &edges, &vertices) {
                let _ = topology.set(Ok(renamed));
            }
            handed_previews = handed.renamed_previews(&faces, &edges).unwrap_or_default();
        }
        let solid = Arc::new(Solid {
            source: app.handed_source(),
            app,
            key,
            handed_previews,
            ids,
            display,
            print: MeshCache::new(),
            previews: MeshCache::new(),
            rendering: MeshCache::new(),
            topology,
            measured: OnceLock::new(),
            mass: OnceLock::new(),
        });
        // A build kept for a hand-over takes what adopting it unread needs, while the
        // body that made it is at hand: later features may release it.
        if let Some(kept) = state.kept.as_mut()
            && let Some(entry) = kept.iter_mut().find(|(key, _, identity)| *key == solid.key && identity.is_none())
        {
            entry.2 = handed_identity(&solid, &record);
        }
        if state.kept.is_some() {
            state.kept_solids.push(solid.clone());
        }
        state.bodies.insert(
            body,
            Stored { steps, result, history: history.into_values().collect(), solid: Some(solid), approximate: None },
        );
        Ok(OpResult { body, record })
    }

    /// Builds the chain of `inputs` and `context` with `step` on the end, stored nowhere.
    fn evaluate<'a>(
        &'a self,
        mut state: MutexGuard<'a, State>,
        op: OpKind,
        inputs: &[BodyId],
        context: &[BodyId],
        mut step: Value,
    ) -> KernelResult<(MutexGuard<'a, State>, Evaluated)> {
        if let Some(field) = inputs.iter().chain(context).find(|body| state.fields.contains_key(body)) {
            return Err(terminal_field(*field));
        }
        // An approximate body's recipe would replay here as an exact build, so neither an
        // input nor a context body may be one. Profiles store no body and pass.
        if inputs.iter().chain(context).any(|body| state.bodies.get(body).is_some_and(|s| s.approximate.is_some())) {
            return Err(approximate::not_yet());
        }
        let mut steps = BTreeMap::new();
        let mut history = BTreeMap::new();
        let mut parents = Vec::new();
        for body in context {
            steps.extend(state.get(*body)?.steps.clone());
        }
        for input in inputs {
            let stored = state.get(*input)?;
            steps.extend(stored.steps.clone());
            history.extend(stored.history.iter().map(|r| (r.seq, r.clone())));
            if let Some(solid) = &stored.solid {
                parents.push(solid.clone());
            }
        }
        let seq = state.id();
        let result = format!("op_{seq}");
        step["id"] = result.clone().into();
        steps.insert(seq, step);
        // Read-only targets can supply geometry checkpoints without becoming history
        // parents. Independent tools still require an exact matching leading prefix.
        let mut prefix: Option<&Stored> = None;
        for body in inputs.iter().chain(context) {
            let stored = state.get(*body)?;
            if stored.solid.is_some()
                && prefix.is_none_or(|best| stored.steps.len() > best.steps.len())
                && stored.steps.iter().eq(steps.iter().take(stored.steps.len()))
            {
                prefix = Some(stored);
            }
        }
        let prefix = prefix.and_then(|stored| stored.solid.clone());
        let input = json!({"format":"soliddraft-recipe/1", "name":"SolidDraft", "steps": steps.values().collect::<Vec<_>>(), "result": result});
        let sources = state.sources(&steps)?;
        let capture = state.capture;
        let offered = state.offered.clone();
        let keeping = state.kept.is_some();
        let stand_in = state.stand_in.clone();
        drop(state);
        let mut questions = Vec::new();
        let files = sources.iter().map(|(sha, bytes)| (sha.clone(), &bytes[..])).collect();
        let built = build(
            &input,
            op,
            prefix.as_ref().map(|solid| &solid.app),
            &files,
            capture.then_some(&mut questions),
            &offered,
            &stand_in,
        );
        // A build made here is kept for hand-over outside the lock; an offered one is not.
        let kept = match &built {
            Ok((Outcome::Exact(app), _, key, _)) if keeping && matches!(app.checkpoint, Exact::Checkpoint(_)) => {
                Some((key.clone(), handover::write(app.checkpoint.built())?))
            }
            _ => None,
        };
        let mut state = self.state()?;
        // A refused build keeps its question too.
        state.questions.extend(questions);
        if let (Some((key, bytes)), Some(list)) = (kept, state.kept.as_mut()) {
            list.push((key, bytes, None));
        }
        if matches!(&built, Ok((Outcome::Exact(app), _, _, _)) if matches!(app.checkpoint, Exact::Checkpoint(_)))
            || matches!(&built, Ok((Outcome::Approximate(_), _, _, _)))
        {
            state.fresh += 1;
        }
        let (app, names, key, handed) = built?;
        Ok((state, Evaluated { app, key, handed, names, steps, history, parents, seq, result }))
    }

    /// Stores a Catallax-native step that reads the solid `base` and builds the chain.
    #[allow(clippy::too_many_arguments)]
    fn native(
        &self,
        state: MutexGuard<'_, State>,
        tag: &OpTag,
        op: OpKind,
        base: BodyId,
        inputs: &[BodyId],
        native: catallax_recipe::Operation,
        sweep: Option<Value>,
        selected_edges: &[EdgeId],
    ) -> KernelResult<OpResult> {
        let base = state.get(base)?.result.clone();
        let mut step = json!({"op":"native","base":base,"native":native});
        if let Some(sweep) = sweep {
            step["sweep"] = sweep;
        }
        self.operation_reading(state, tag, op, inputs, &[], selected_edges, step)
    }

    fn state(&self) -> KernelResult<MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| KernelError::Unavailable("Catallax body arena lock poisoned".into()))
    }

    fn solid(&self, body: BodyId) -> KernelResult<Arc<Solid>> {
        self.state()?.solid(body)
    }

    /// The numerical body stored as `body`, if it is one.
    fn approximate(&self, body: BodyId) -> KernelResult<Option<Arc<approximate::Approximate>>> {
        Ok(self.state()?.get(body)?.approximate.clone())
    }
}

/// What another kernel needs to adopt `solid` without reading its bytes: its names and
/// ids by Catallax handle index, the record of the operation that made it and the facts
/// an edit asks for. `None` if an entity has no id.
fn handed_identity(solid: &Solid, record: &HistoryRecord) -> Option<handover::HandedIdentity> {
    let record = record.clone();
    let names = &solid.app.names;
    Some(handover::HandedIdentity {
        steps: solid.app.recipe().steps.iter().map(|step| step.id.as_str().to_owned()).collect(),
        faces: names
            .faces
            .iter()
            .map(|(cat, origins)| Some((cat.index(), solid.ids.faces.get(cat)?.raw(), origins.clone())))
            .collect::<Option<_>>()?,
        edges: names
            .edges
            .iter()
            .map(|(cat, name)| Some((cat.index(), solid.ids.edges.get(cat)?.raw(), name.clone())))
            .collect::<Option<_>>()?,
        vertices: names
            .vertices
            .iter()
            .map(|(cat, name)| Some((cat.index(), solid.ids.vertices.get(cat)?.raw(), name.clone())))
            .collect::<Option<_>>()?,
        record,
        splines: solid.has_splines(),
        surface: solid.app.is_surface(),
        planes: native::planes(&solid.app),
    })
}

/// A terminal field is last: no CAD step or B-rep query reads it.
fn terminal_field(body: BodyId) -> KernelError {
    KernelError::InvalidInput(format!(
        "lattice.cad-after-lattice: {body} is a terminal field; return before it to edit"
    ))
}

/// The operation's result, before it becomes a stored body.
struct Evaluated {
    app: Outcome,
    key: String,
    /// The topology another kernel computed for an offered build.
    handed: Option<Arc<handover::HandedTopology>>,
    names: BTreeMap<String, catallax_recipe::StepId>,
    steps: BTreeMap<u64, Value>,
    history: BTreeMap<u64, HistoryRecord>,
    parents: Vec<Arc<Solid>>,
    seq: u64,
    result: String,
}

const NOTHING_LEFT: &str = "this would leave nothing of the part";

/// A fillet whose rolling contact ended below the radius it was asked for. Catallax
/// then builds the strip at the smaller radius where the ball reached a face boundary
/// and keeps the request in `requested_radius`. SolidDraft never builds a radius other
/// than the one the feature records, so it refuses as Catallax did before, naming the
/// largest radius that fits. A radius that consumes a face exactly still builds.
fn stopped_short(app: &AppBody, step: &str) -> Option<KernelError> {
    let built = app.checkpoint.built();
    let limit = built
        .fillets
        .iter()
        .filter_map(|f| {
            let requested = f.requested_radius.as_ref()?.to_f64();
            let radius = f.radius.to_f64();
            (radius < requested).then_some(radius)
        })
        .min_by(f64::total_cmp)?;
    // Round down, so the advice is a radius that fits.
    let shown = (limit * 1000.0).floor() / 1000.0;
    Some(KernelError::Failed {
        op: OpKind::Fillet,
        code: 0,
        message: format!(
            "{}: step {step}: the ball of this radius reaches a face boundary before it can roll; radius limit {shown} mm",
            catallax_features::fillet::RADIUS_TOO_LARGE
        ),
    })
}

fn is_empty(app: &AppBody) -> bool {
    app.is_empty()
}

fn gap(what: &str) -> KernelError {
    KernelError::Unsupported(format!("{what} is coming soon."))
}

/// Converts retained application definitions, preserving native steps and frames.
fn converted_recipe(input: &Value, op: OpKind) -> KernelResult<catallax_app::soliddraft::Converted> {
    // The shared converter reads `soliddraft-recipe/1` only. Replace unsupported
    // steps during conversion, then restore their native definitions before building.
    let mut legacy = input.clone();
    let mut natives = Vec::new();
    let mut patterns = Vec::new();
    // An import has no counterpart there. A unit block stands in for it, built from a
    // stand-in profile that is dropped again once the import step is back.
    let mut imports = Vec::new();
    let mut scan_fits = Vec::new();
    let steps = legacy["steps"].as_array_mut().expect("generated recipe steps");
    for index in (0..steps.len()).rev() {
        if steps[index]["op"] == "scan_fit" {
            let tag = steps[index]["id"].as_str().expect("generated fit id").to_owned();
            scan_fits.push((tag.clone(), steps[index]["recipe"].clone()));
            let profile = format!("{tag}_fit_profile");
            steps[index] = json!({"id":tag,"op":"extrude","profile":profile,"distance":1.0});
            steps.insert(
                index,
                json!({"id":profile,"op":"profile","plane":"XY","rectangle":{"width":1,"height":1,"center":[0,0]}}),
            );
            continue;
        }
        if !matches!(steps[index]["op"].as_str(), Some("import_step" | "import_iges" | "import_mesh")) {
            continue;
        }
        let mut native = steps[index].clone();
        let tag = native.as_object_mut().and_then(|o| o.remove("id")).and_then(|id| id.as_str().map(str::to_owned));
        let tag = tag.expect("generated import id");
        let native: catallax_recipe::Operation = serde_json::from_value(native)
            .map_err(|e| KernelError::InvalidInput(format!("stored import step: {e}")))?;
        let profile = format!("{tag}_import_profile");
        steps[index] = json!({"id":tag, "op":"extrude", "profile":profile, "distance":1.0});
        let rectangle = json!({"center":[0.0, 0.0], "width":1.0, "height":1.0});
        steps.insert(index, json!({"id":profile, "op":"profile", "plane":"XY", "rectangle":rectangle}));
        imports.push((tag, profile, native));
    }
    for step in legacy["steps"].as_array_mut().expect("generated recipe steps") {
        if step["op"] == "feature_pattern" {
            patterns.push(step.clone());
            *step =
                json!({"id":step["id"], "op":"boolean", "kind":"union", "target":step["base"], "tool":step["base"]});
            continue;
        }
        if step["op"] == "native_profile" {
            let tag = step["id"].as_str().expect("stored profile id").to_owned();
            let native: catallax_recipe::Operation = serde_json::from_value(step["native"].clone())
                .map_err(|_| loft::refusal("loft.incompatible-sections"))?;
            natives.push((tag.clone(), native, Value::Null));
            *step = json!({"id":tag,"op":"profile","plane":"XY","rectangle":{"width":1,"height":1,"center":[0,0]}});
            continue;
        }
        if step["op"] == "loft" {
            let tag = step["id"].as_str().expect("stored loft id").to_owned();
            let first = step["profiles"][0].clone();
            *step = json!({"id":tag,"op":"extrude","profile":first,"distance":1.0});
            continue;
        }
        if step["op"] == "path_sweep" {
            let tag = step["id"].as_str().expect("stored sweep id").to_owned();
            let profile = step["profile"].clone();
            let path = step["path"].clone();
            natives.push((tag.clone(), sweep::operation(&tag, &profile, &path)?, path));
            *step = json!({"id":tag,"op":"extrude","profile":profile,"distance":1.0});
            continue;
        }
        if step["op"] != "native" {
            continue;
        }
        let tag = step["id"].as_str().expect("generated step id").to_owned();
        let native: catallax_recipe::Operation = serde_json::from_value(step["native"].clone())
            .map_err(|e| KernelError::InvalidInput(format!("stored Catallax step: {e}")))?;
        natives.push((tag.clone(), native, step["sweep"].clone()));
        let base = step["base"].clone();
        *step = json!({"id":tag, "op":"boolean", "kind":"union", "target":base, "tool":base});
    }
    let mut converted =
        catallax_app::soliddraft::convert(&legacy.to_string()).map_err(|e| build_error(op, Error::Conversion(e)))?;
    for (tag, profile, native) in imports {
        let stand_in = converted.steps.remove(&profile).expect("converted stand-in profile");
        converted.recipe.steps.retain(|s| s.id != stand_in);
        let id = &converted.steps[&tag];
        let step = converted.recipe.steps.iter_mut().find(|s| &s.id == id).expect("converted import step");
        step.op = native;
    }
    for (tag, recipe) in scan_fits {
        scan::expand(&mut converted, &tag, recipe)?;
    }
    // Start in the newest format so adding a limited sweep or an all-faces shell does not
    // discard an otherwise identical target checkpoint merely to upgrade its recipe version.
    catallax_recipe::FORMAT_V5.clone_into(&mut converted.recipe.format);
    let sources = input["steps"].as_array().expect("generated recipe steps");
    let mut planes = BTreeMap::new();
    for source in sources {
        if source["op"] != "profile" {
            continue;
        }
        let plane: Plane = serde_json::from_value(source["plane"].clone())
            .map_err(|e| KernelError::InvalidInput(format!("stored profile frame: {e}")))?;
        let name = source["id"].as_str().expect("generated step id");
        let id = &converted.steps[name];
        let step = converted.recipe.steps.iter_mut().find(|s| &s.id == id).expect("converted profile step");
        if let catallax_recipe::Operation::Sketch { plane: pose, .. } = &mut step.op {
            if let Some(rotation) = frames::rotation(plane) {
                pose.rotation = rotation;
            }
            planes.insert(name.to_owned(), (plane, pose.rotation.clone()));
        }
    }
    // A sweep along the sketch normal uses the normal of the same certified frame.
    let sweep = |profile: &str, direction: &Value| -> KernelResult<Option<catallax_recipe::Vector3>> {
        let direction: Vec3 = serde_json::from_value(direction.clone())
            .map_err(|e| KernelError::InvalidInput(format!("stored sweep direction: {e}")))?;
        let (plane, rotation) = &planes[profile];
        Ok(frames::normal_sweep(*plane, rotation, direction))
    };
    for source in sources {
        if source["op"] != "extrude" || source["direction"].is_null() {
            continue;
        }
        let profile = source["profile"].as_str().expect("generated profile id");
        if let Some(direction) = sweep(profile, &source["direction"])? {
            let id = &converted.steps[source["id"].as_str().expect("generated step id")];
            let step = converted.recipe.steps.iter_mut().find(|s| &s.id == id).expect("converted extrusion step");
            if let catallax_recipe::Operation::Extrude { direction: d, .. } = &mut step.op {
                *d = Some(direction);
            }
        }
    }
    for (tag, mut native, stored_sweep) in natives {
        let id = converted.steps[&tag].clone();
        // Generated tags are already valid step ids, so native inputs name converted steps.
        debug_assert_eq!(id.as_str(), tag);
        if let Some(profile) = stored_sweep["profile"].as_str()
            && !stored_sweep["direction"].is_null()
            && let Some(direction) = sweep(profile, &stored_sweep["direction"])?
            && let catallax_recipe::Operation::ExtrudeTo { direction: d, .. }
            | catallax_recipe::Operation::ExtrudeNext { direction: d, .. } = &mut native
        {
            *d = Some(direction);
        }
        let index = converted.recipe.steps.iter().position(|s| s.id == id).expect("converted native step");
        converted.recipe.steps[index].op = native;
        if stored_sweep["segments"].is_array() {
            let path = serde_json::from_value(sweep::stored_path(&tag, &stored_sweep)?)
                .map_err(|e| KernelError::InvalidInput(format!("stored sweep path: {e}")))?;
            converted.recipe.steps.insert(index, path);
        }
    }
    for source in sources.iter().filter(|s| s["op"] == "loft") {
        let profiles: Vec<String> = serde_json::from_value(source["profiles"].clone())
            .map_err(|_| loft::refusal("loft.incompatible-sections"))?;
        let shape: LoftShape =
            serde_json::from_value(source["shape"].clone()).map_err(|_| loft::refusal("loft.incompatible-sections"))?;
        let native = loft::operation(&profiles, shape, &converted.recipe)?;
        let id = &converted.steps[source["id"].as_str().expect("stored loft id")];
        converted.recipe.steps.iter_mut().find(|s| &s.id == id).expect("converted loft").op = native;
    }
    for pattern in patterns {
        feature_pattern::expand(&mut converted, &pattern)?;
    }
    Ok(converted)
}

/// What a build made: exact geometry, or geometry built within a stated tolerance.
enum Outcome {
    Exact(AppBody),
    Approximate(Arc<NumericalApplicationBody>),
}

impl From<EvaluatedApplicationOutcome> for Outcome {
    fn from(outcome: EvaluatedApplicationOutcome) -> Self {
        match outcome {
            EvaluatedApplicationOutcome::Exact(app) => Outcome::Exact(app.into()),
            EvaluatedApplicationOutcome::Numerical(body) => Outcome::Approximate(body),
        }
    }
}

/// A build: its result, the step ids of the caller's tags, its recipe key for a
/// hand-over, and the topology another kernel computed when the build was offered.
type BuildAnswer = (Outcome, BTreeMap<String, catallax_recipe::StepId>, String, Option<Arc<handover::HandedTopology>>);

/// Builds a legacy recipe, returning the body and the legacy step names' Catallax step ids.
fn build(
    input: &Value,
    op: OpKind,
    prefix: Option<&AppBody>,
    files: &catallax_features::ImportFiles<'_>,
    questions: Option<&mut Vec<Value>>,
    offered: &BTreeMap<String, handover::Offer>,
    stand_in: &[OpKind],
) -> KernelResult<BuildAnswer> {
    let converted = converted_recipe(input, op)?;
    if let Some(questions) = questions {
        let problem = serde_json::to_value(&converted.recipe).expect("Catallax recipes serialize");
        questions.push(json!({"format":"catallax-question/1", "source":"recipe", "problem":problem}));
    }
    let key = handover::key(&converted.recipe);
    if let Some(offer) = offered.get(&key) {
        // A build that reads no files is adopted unread when its names came with it.
        if files.is_empty()
            && let Some(app) = AppBody::offered(offer.clone(), &converted.recipe)
        {
            return Ok((Outcome::Exact(app), converted.steps, key, offer.topology.clone()));
        }
        // An offer whose bytes cannot be read is passed over; the build is evaluated instead.
        if let Ok(built) = offer.built()
            && let Some(app) = AppBody::adopted(built, &converted.recipe)
        {
            return Ok((Outcome::Exact(app), converted.steps, key, offer.topology.clone()));
        }
    }
    // Earlier bodies remain immutable for undo, branches and concurrent queries.
    // Clone only the selected checkpoint, after releasing the body arena lock.
    // A handed-over body has no continuation; its prefix is rebuilt from the recipe.
    let prefix = prefix.and_then(|app| app.checkpoint.continuation());
    let app = if files.is_empty() {
        catallax_app::build_incremental_without_display(&converted.recipe, prefix, &mut Decider::default())
    } else {
        build_with_files(&converted.recipe, prefix, files)
    }
    .map_err(|e| {
        let spline = input["steps"].as_array()
            .and_then(|steps| steps.iter().find(|step| step["id"] == input["result"]))
            .and_then(|step| step["path"]["segments"].as_array())
            .is_some_and(|segments| segments.iter().any(|s| s["kind"] == "spline"));
        if op == OpKind::Sweep && spline && e.code() == "features.unsupported-path" {
            KernelError::InvalidInput("This spline sweep was refused because it needs one spline segment and a profile that keeps its direction".into())
        } else {
            build_error(op, e)
        }
    })?;
    // Catallax's application entry returns exact bodies only, for now. A numerical one
    // is retained as Catallax built it, never turned into a refusal or an exact body.
    let outcome = EvaluatedApplicationOutcome::Exact(app);
    #[cfg(feature = "numerical-stand-in")]
    let outcome = if stand_in.contains(&op) {
        stand_in::numerical(outcome, &converted.recipe).map_err(|e| KernelError::Failed {
            op,
            code: 0,
            message: format!("{}: {}", e.code, e.detail),
        })?
    } else {
        outcome
    };
    #[cfg(not(feature = "numerical-stand-in"))]
    let _ = stand_in;
    Ok((outcome.into(), converted.steps, key, None))
}

/// [`catallax_app::build_incremental_without_display`] for a recipe that reads STEP
/// sources: the same default allowance, result check and picking names.
fn build_with_files(
    recipe: &catallax_recipe::Recipe,
    prefix: Option<catallax_features::EvaluationCheckpoint>,
    files: &catallax_features::ImportFiles<'_>,
) -> Result<EvaluatedApplicationBody, Error> {
    let run = || {
        let checkpoint =
            catallax_features::evaluate_incremental_with_files(recipe, prefix, files, &mut Decider::default())
                .map_err(Error::Build)?;
        let built = checkpoint.built();
        let (model, body) = (&built.model, built.result);
        let faults = model.check(body);
        if !faults.is_empty() {
            return Err(Error::InvalidBody(faults));
        }
        let names = catallax_app::Names {
            faces: model.faces_of_body(body).into_iter().map(|f| (f, model.face_origins(f).to_vec())).collect(),
            edges: model.edges_of_body(body).into_iter().map(|e| (e, model.edge_name(body, e))).collect(),
            vertices: model.vertices_of_body(body).into_iter().map(|v| (v, model.vertex_name(body, v))).collect(),
        };
        Ok(EvaluatedApplicationBody { checkpoint, names })
    };
    if catallax_numbers::operation_budget_active() {
        return run();
    }
    let (result, work) = catallax_numbers::with_operation_budget(catallax_app::DEFAULT_BUILD_WORK, run);
    if work.exhausted { Err(Error::ResourceLimit) } else { result }
}

fn loft_fillet_refusal(error: &Error) -> Option<&'static str> {
    let Error::Build(catallax_features::BuildError::Geometry { refusal, .. }) = error else { return None };
    if refusal.code != "fillet.unsupported-spline" {
        return None;
    }
    Some(if refusal.detail.starts_with("a spline fillet's side face must be its whole patch, with no other trims") {
        "Fillet cannot round this loft edge because a side face has already been trimmed. Choose an edge on an untrimmed face"
    } else {
        "Fillet cannot round this loft edge. Pick an outside edge with untrimmed side faces and flat end faces"
    })
}

fn build_error(op: OpKind, error: Error) -> KernelError {
    if op == OpKind::Sweep {
        return sweep::refusal(error.code());
    }
    if matches!(op, OpKind::Offset | OpKind::Thicken) {
        return distance::refusal(op, error.code());
    }
    if op == OpKind::Pattern {
        return feature_pattern::refusal(error.code());
    }
    if op == OpKind::Loft {
        return loft::refusal(error.code());
    }
    if op == OpKind::Fillet
        && let Some(message) = loft_fillet_refusal(&error)
    {
        return KernelError::InvalidInput(message.into());
    }
    let code = error.code();
    if matches!(error, Error::Conversion(catallax_app::soliddraft::ConversionError::NotYet { .. })) {
        return gap(&format!("{op:?}"));
    }
    KernelError::Failed { op, code: 0, message: format!("{code}: {error}") }
}

fn query_error(query: &str, error: catallax_numbers::Refusal) -> KernelError {
    if error.code == "mesh.vertex-limit" {
        return KernelError::InvalidInput(
            "This shape needs too much detail to display accurately. Try a smoother path or a simpler profile.".into(),
        );
    }
    KernelError::QueryFailed { query: query.into(), code: 0, message: format!("{}: {}", error.code, error.detail) }
}

fn numerical<T>(
    operation: &str,
    run: impl FnOnce(&mut Decider) -> Result<catallax_numbers::Answer<T>, catallax_numbers::Refusal>,
) -> Result<catallax_numbers::Answer<T>, catallax_numbers::Refusal> {
    let mut decider = Decider::default();
    let span = catallax_numbers::OperationSpan::begin("brep", &operation, operation, &decider);
    let result = run(&mut decider);
    span.finish(&decider, result.as_ref().err(), result.as_ref().ok().map(|a| &a.marks));
    result
}

/// A rational lower bound of pi, so a requested angle in degrees becomes a bound in
/// radians that is never looser than asked.
const PI_BELOW: (i64, i64) = (314_159_265, 100_000_000);

fn options(params: &TessellationParams) -> KernelResult<MeshOptions> {
    let values = [
        params.chord_height,
        params.curve_angle_deg,
        params.surface_angle_deg,
        params.max_edge_length,
        params.max_aspect_ratio,
    ];
    if values.iter().any(|v| !v.is_finite() || *v < 0.0) {
        return Err(KernelError::InvalidInput("tessellation controls must be finite and nonnegative".into()));
    }
    let exact = |v: f64| Rational::parse(&v.to_string()).map_err(|e| KernelError::InvalidInput(e.to_string()));
    let mut options = MeshOptions::default();
    if params.chord_height > 0.0 {
        // Only an explicit bound needs room for the trait's f32 positions.
        // Zero disables this control; keep the already-certified native default.
        options.chord_tolerance = &exact(params.chord_height)? / &Rational::from_i64(2);
    }
    if params.surface_angle_deg > 0.0 {
        let pi = Rational::from_ratio(PI_BELOW.0, PI_BELOW.1).expect("pi bound");
        options.angular_deviation = Some(&(&exact(params.surface_angle_deg)? * &pi) / &Rational::from_i64(180));
    }
    if params.max_edge_length > 0.0 {
        options.max_edge_length = Some(exact(params.max_edge_length)?);
    }
    if params.max_aspect_ratio > 0.0 {
        options.max_aspect_ratio = Some(exact(params.max_aspect_ratio)?);
    }
    Ok(options)
}

impl Kernel for CatallaxKernel {
    fn scan_reference(
        &self,
        source: &soliddraft_kernel_api::scan::ScanSource,
    ) -> KernelResult<soliddraft_kernel_api::scan::ScanReference> {
        self.observe("scan_reference", || scan::reference(source))
    }

    fn fit_scan(
        &self,
        source: &soliddraft_kernel_api::scan::ScanSource,
    ) -> KernelResult<soliddraft_kernel_api::scan::ScanFit> {
        self.observe("fit_scan", || scan::fit(source))
    }

    fn build_scan_fit(
        &self,
        tag: &OpTag,
        source: &soliddraft_kernel_api::scan::ScanSource,
        fit: &soliddraft_kernel_api::scan::ScanFit,
    ) -> KernelResult<OpResult> {
        self.observe("build_scan_fit", || {
            scan::validate(source, fit)?;
            let mut state = self.state()?;
            state.files.entry(source.sha256.clone()).or_insert_with(|| source.bytes.clone().into());
            self.operation(
                state,
                tag,
                OpKind::ScanFit,
                &[],
                json!({"op":"scan_fit","file":{"sha256":source.sha256},"recipe":fit.recipe}),
            )
        })
    }

    fn body_relation(&self, a: PlacedBody, b: PlacedBody, clearance: Option<f64>) -> KernelResult<BodyRelation> {
        self.observe("body_relation", || body_relation::relation(self, a, b, clearance))
    }

    fn propose_import(&self, source: &[u8], options: &ImportOptions) -> KernelResult<ProposalSet> {
        self.observe("propose_import", || import::propose(ImportFormat::Step, source, options))
    }

    fn import_step(&self, tag: &OpTag, request: &ImportStepRequest<'_>) -> KernelResult<ImportStepResult> {
        self.observe("import_step", || self.import_body(ImportFormat::Step, tag, request))
    }

    fn propose_import_source(
        &self,
        format: ImportFormat,
        source: &[u8],
        options: &ImportOptions,
    ) -> KernelResult<ProposalSet> {
        self.observe("propose_import", || import::propose(format, source, options))
    }

    fn replace_import_file(&self, request: &FileReplacementRequest<'_>) -> KernelResult<FileReplacement> {
        import::replace_file(request)
    }

    fn keep_builds(&self) -> KernelResult<()> {
        self.state()?.kept.get_or_insert_with(Vec::new);
        Ok(())
    }

    fn hand_over(&self) -> KernelResult<Vec<u8>> {
        let mut state = self.state()?;
        let kept = state.kept.take().unwrap_or_default();
        // Topologies computed here and not handed over yet, by recipe key: a kept build's,
        // or one of a build handed over before its topology was computed.
        let mut topologies: BTreeMap<String, handover::HandedTopology> = BTreeMap::new();
        let kept_solids = std::mem::take(&mut state.kept_solids);
        let solids = state.bodies.values().filter_map(|stored| stored.solid.as_deref());
        for solid in solids.chain(kept_solids.iter().map(|solid| solid.as_ref())) {
            let Some(Ok(topology)) = solid.topology.get() else { continue };
            if solid.key.is_empty() || state.sent_topologies.contains(&solid.key) || topologies.contains_key(&solid.key)
            {
                continue;
            }
            let (faces, edges, vertices) = solid.ids.by_index();
            let pairs = |ids: Vec<(u32, u64)>| ids.into_iter().map(|(index, raw)| (raw, index)).collect();
            // The early display meshes sampled so far travel with the topology.
            let previews = solid
                .previews
                .computed()
                .into_iter()
                .filter_map(|(key, preview)| {
                    let mesh = preview.as_ref().as_ref()?;
                    Some(handover::HandedPreview {
                        key: preview_key(&key),
                        mesh: convert::preview(mesh, &solid.ids),
                        edges: preview_edges(mesh, &solid.ids),
                    })
                })
                .collect();
            topologies.insert(
                solid.key.clone(),
                handover::HandedTopology {
                    previews,
                    topology: topology.clone(),
                    faces: pairs(faces.into_iter().map(|(index, id)| (index, id.raw())).collect()),
                    edges: pairs(edges.into_iter().map(|(index, id)| (index, id.raw())).collect()),
                    vertices: pairs(vertices.into_iter().map(|(index, id)| (index, id.raw())).collect()),
                },
            );
        }
        let mut handed: Vec<handover::Handed> = kept
            .into_iter()
            .map(|(key, body, identity)| {
                let topology = topologies.remove(&key);
                handover::Handed { key, body: Some(body), topology, identity }
            })
            .collect();
        handed.extend(topologies.into_iter().map(|(key, topology)| handover::Handed {
            key,
            body: None,
            topology: Some(topology),
            identity: None,
        }));
        for entry in &handed {
            if entry.topology.is_some() {
                state.sent_topologies.insert(entry.key.clone());
            }
        }
        drop(state);
        handover::encode(&handed)
    }

    fn offer_builds(&self, source: u64, bytes: &[u8]) -> KernelResult<usize> {
        let received = handover::decode(bytes)?;
        let count = received.len();
        let mut state = self.state()?;
        for (key, received) in received {
            match received {
                handover::Received::Build(mut offer) => {
                    offer.reads = state.offer_reads.clone();
                    offer.source = source;
                    state.offered.insert(key, offer);
                }
                // A topology for a build offered before: the body built from it next takes it.
                handover::Received::Topology(topology) => {
                    if let Some(offer) = state.offered.get_mut(&key) {
                        offer.topology = Some(topology);
                    }
                }
            }
        }
        Ok(count)
    }

    fn body_kind(&self, body: BodyId) -> KernelResult<BodyKind> {
        if self.field(body)?.is_some() {
            return Ok(BodyKind::Solid);
        }
        if let Some(approximate) = self.approximate(body)? {
            return Ok(approximate.kind());
        }
        Ok(if self.solid(body)?.app.is_surface() { BodyKind::Sheet } else { BodyKind::Solid })
    }

    fn body_accuracy(&self, body: BodyId) -> KernelResult<BodyAccuracy> {
        if self.field(body)?.is_some() {
            return Ok(BodyAccuracy::Exact);
        }
        Ok(self.approximate(body)?.map_or(BodyAccuracy::Exact, |approximate| approximate.accuracy()))
    }

    fn exact_unread(&self, body: BodyId) -> KernelResult<bool> {
        let state = self.state()?;
        Ok(state.get(body)?.solid.as_ref().is_some_and(|solid| !solid.app.checkpoint.is_read()))
    }

    fn read_exact(&self, body: BodyId) -> KernelResult<()> {
        let solid = self.state()?.get(body)?.solid.clone();
        // Read outside the lock, so other queries are not held for it.
        if let Some(solid) = solid {
            let _ = solid.app.checkpoint.built();
        }
        Ok(())
    }

    fn read_offers(&self, at_most: usize) -> KernelResult<usize> {
        // Read outside the lock; offers share their read result, so the build that takes
        // one later finds it read. An offer that came with its names is adopted unread,
        // so it is read only when exact geometry is asked of it.
        let unread: Vec<handover::Offer> = self
            .state()?
            .offered
            .values()
            .filter(|offer| !offer.is_read() && offer.identity.is_none())
            .cloned()
            .collect();
        for offer in unread.iter().take(at_most) {
            let _ = offer.built();
        }
        Ok(unread.len().saturating_sub(at_most))
    }

    fn source_solids(&self, format: ImportFormat, source: &[u8]) -> KernelResult<Vec<String>> {
        self.observe("source_solids", || import::source_solids(format, source))
    }

    fn import_source(
        &self,
        format: ImportFormat,
        tag: &OpTag,
        request: &ImportStepRequest<'_>,
    ) -> KernelResult<ImportStepResult> {
        let op = match format {
            ImportFormat::Step => "import_step",
            ImportFormat::Iges => "import_iges",
            ImportFormat::Mesh => "import_mesh",
        };
        self.observe(op, || self.import_body(format, tag, request))
    }

    fn backend(&self) -> BackendInfo {
        BackendInfo { name: "catallax".into(), version: CATALLAX_REVISION.into() }
    }

    fn create_body_from_profile(&self, tag: &OpTag, profile: &Profile) -> KernelResult<OpResult> {
        self.observe("create_body_from_profile", || {
            profile.check()?;
            let mut state = self.state()?;
            let seq = state.id();
            let result = format!("op_{seq}");
            let step = json!({
                "id":result, "op":"profile", "plane":profile.plane,
                "loops":{"outer":profile.outer,"holes":profile.holes}
            });
            // Conversion validates the exact frame and joined profile without manufacturing a sheet.
            // The converter requires a body result, so validate with a structural extrusion only.
            let input = json!({
                "format":"soliddraft-recipe/1","name":"profile",
                "steps":[step, {"id":"check_body","op":"extrude","profile":result,"distance":1}],
                "result":"check_body"
            });
            catallax_app::soliddraft::convert(&input.to_string())
                .map_err(|e| build_error(OpKind::Profile, Error::Conversion(e)))?;
            let record = HistoryRecord {
                seq,
                tag: tag.clone(),
                op: OpKind::Profile,
                faces: EntityChanges::default(),
                edges: EntityChanges::default(),
                vertices: EntityChanges::default(),
                face_roles: vec![],
            };
            let body = BodyId::from_raw(state.id());
            state.bodies.insert(
                body,
                Stored {
                    steps: BTreeMap::from([(seq, step)]),
                    result,
                    history: vec![record.clone()],
                    solid: None,
                    approximate: None,
                },
            );
            Ok(OpResult { body, record })
        })
    }

    fn extrude(&self, tag: &OpTag, profile: BodyId, params: &Extrude) -> KernelResult<OpResult> {
        self.observe("extrude", || {
            let state = self.state()?;
            let p = state.get(profile)?;
            if p.solid.is_some() {
                return Err(KernelError::InvalidInput("extrude needs a profile".into()));
            }
            let step =
                json!({"op":"extrude","profile":p.result,"distance":params.distance,"direction":params.direction});
            self.operation(state, tag, OpKind::Extrude, &[profile], step)
        })
    }

    fn extrude_on_face(
        &self,
        tag: &OpTag,
        profile: &Profile,
        support: soliddraft_kernel_api::FaceSupport,
        distance: f64,
    ) -> KernelResult<OpResult> {
        profile.check()?;
        let local = Profile { plane: Plane::XY, ..profile.clone() };
        let sheet = self.create_body_from_profile(&OpTag::new(format!("{}.profile", tag.0)), &local)?;
        let result = self.observe("extrude_on_face", || {
            let state = self.state()?;
            let solid = state.solid(support.body)?;
            let face = solid.cat_face(support.face, support.body)?;
            let origin = solid.app.names.faces[&face]
                .first()
                .ok_or_else(|| KernelError::InvalidInput("the supporting face has no origin".into()))?;
            let native = catallax_recipe::Operation::ExtrudeOnFace {
                profile: state.step(sheet.body)?,
                body: state.step(support.body)?,
                face: native::pattern(origin),
                normal_direction: native::vector(profile.plane.normal, "sketch normal direction")?,
                x_direction: native::vector(profile.plane.x_dir, "sketch X direction")?,
                offset: native::number(support.offset, "sketch offset")?,
                distance: native::number(distance, "extrusion distance")?,
                placement: catallax_recipe::Pose::identity(),
            };
            self.native(state, tag, OpKind::Extrude, support.body, &[support.body, sheet.body], native, None, &[])
        });
        let _ = self.release(sheet.body);
        result
    }

    fn create_profile_from_face(&self, tag: &OpTag, body: BodyId, face: FaceId) -> KernelResult<OpResult> {
        self.observe("create_profile_from_face", || {
            let native = loft::face_profile(self.solid(body)?.as_ref(), body, face)?;
            let mut state = self.state()?;
            let seq = state.id();
            let result = format!("op_{seq}");
            let step = json!({"id":result,"op":"native_profile","native":native});
            let record = HistoryRecord {
                seq,
                tag: tag.clone(),
                op: OpKind::Profile,
                faces: EntityChanges::default(),
                edges: EntityChanges::default(),
                vertices: EntityChanges::default(),
                face_roles: vec![],
            };
            let body = BodyId::from_raw(state.id());
            state.bodies.insert(
                body,
                Stored {
                    steps: BTreeMap::from([(seq, step)]),
                    result,
                    history: vec![record.clone()],
                    solid: None,
                    approximate: None,
                },
            );
            Ok(OpResult { body, record })
        })
    }

    fn loft(&self, tag: &OpTag, profiles: &[BodyId], shape: LoftShape) -> KernelResult<OpResult> {
        self.observe("loft", || {
            if profiles.len() < 2 {
                return Err(loft::refusal("loft.sections"));
            }
            let state = self.state()?;
            let mut names = Vec::new();
            for profile in profiles {
                let p = state.get(*profile)?;
                if p.solid.is_some() {
                    return Err(loft::refusal("loft.incompatible-sections"));
                }
                names.push(p.result.clone());
            }
            self.operation(state, tag, OpKind::Loft, profiles, json!({"op":"loft","profiles":names,"shape":shape}))
        })
        .map_err(|error| match error {
            KernelError::Unavailable(_) => loft::refusal(""),
            other => other,
        })
    }

    fn sweep(&self, tag: &OpTag, profile: BodyId, params: &soliddraft_kernel_api::Sweep) -> KernelResult<OpResult> {
        self.observe("sweep", || {
            sweep::path_step("checked", params)?;
            let state = self.state()?;
            let p = state.get(profile)?;
            if p.solid.is_some() {
                return Err(KernelError::InvalidInput("Select a closed polygon sketch to sweep".into()));
            }
            let source = p.steps.values().find(|step| step["id"] == p.result).expect("stored profile");
            let shape: Profile = serde_json::from_value(
                json!({"plane":source["plane"], "outer":source["loops"]["outer"], "holes":source["loops"]["holes"]}),
            )
            .map_err(|e| KernelError::InvalidInput(e.to_string()))?;
            if std::iter::once(&shape.outer)
                .chain(&shape.holes)
                .flat_map(|l| &l.0)
                .any(|s| !matches!(s, soliddraft_kernel_api::Segment::Line { .. }))
            {
                return Err(KernelError::InvalidInput(
                    "Sweep currently needs a closed polygon profile with straight sides".into(),
                ));
            }
            let result = p.result.clone();
            self.operation(
                state,
                tag,
                OpKind::Sweep,
                &[profile],
                json!({
                    "op":"path_sweep", "profile":result, "path":params
                }),
            )
        })
        .map_err(|error| match error {
            KernelError::Unavailable(_) => sweep::refusal(""),
            other => other,
        })
    }

    fn revolve(&self, tag: &OpTag, profile: BodyId, params: &Revolve) -> KernelResult<OpResult> {
        self.observe("revolve", || {
            let state = self.state()?;
            let p = state.get(profile)?;
            if p.solid.is_some() {
                return Err(KernelError::InvalidInput("revolve needs a profile".into()));
            }
            let step = json!({
                "op":"revolve","profile":p.result,"axis_origin":params.axis_origin,
                "axis_direction":params.axis_direction,"angle_deg":params.angle_deg
            });
            self.operation(state, tag, OpKind::Revolve, &[profile], step)
        })
    }

    fn boolean(&self, tag: &OpTag, op: BooleanOp, target: BodyId, tool: BodyId) -> KernelResult<OpResult> {
        self.observe("boolean", || {
            let state = self.state()?;
            state.solid(target)?;
            state.solid(tool)?;
            let step =
                json!({"op":"boolean","kind":op,"target":state.get(target)?.result,"tool":state.get(tool)?.result});
            self.operation(state, tag, op.kind(), &[target, tool], step)
        })
    }

    fn fillet(&self, tag: &OpTag, body: BodyId, edges: &[EdgeId], radius: f64) -> KernelResult<OpResult> {
        self.observe("fillet", || {
            let state = self.state()?;
            let solid = state.solid(body)?;
            if edges.is_empty() {
                return Err(KernelError::InvalidInput("a fillet needs at least one edge".into()));
            }
            let refs = edges.iter().map(|e| solid.edge_ref(*e, body)).collect::<KernelResult<Vec<_>>>()?;
            let native = catallax_recipe::Operation::Fillet {
                body: state.step(body)?,
                edges: catallax_recipe::EdgeSelection::Refs(refs),
                radius: native::number(radius, "fillet radius")?,
                // The user picks only edges and a radius. Continuation and corner
                // closure stay at the recipe's defaults: roll over and cliff.
                overflow: None,
                corner: None,
            };
            self.native(state, tag, OpKind::Fillet, body, &[body], native, None, edges)
        })
    }

    fn chamfer(&self, tag: &OpTag, body: BodyId, edges: &[EdgeId], distance: f64) -> KernelResult<OpResult> {
        self.chamfer_distances(tag, body, edges, distance, None)
    }

    fn chamfer_distances(
        &self,
        tag: &OpTag,
        body: BodyId,
        edges: &[EdgeId],
        distance: f64,
        other_distance: Option<f64>,
    ) -> KernelResult<OpResult> {
        self.observe("chamfer", || {
            let state = self.state()?;
            let solid = state.solid(body)?;
            if edges.is_empty() {
                return Err(KernelError::InvalidInput("a chamfer needs at least one edge".into()));
            }
            let refs = edges.iter().map(|e| solid.edge_ref(*e, body)).collect::<KernelResult<Vec<_>>>()?;
            let native = catallax_recipe::Operation::Chamfer {
                body: state.step(body)?,
                edges: catallax_recipe::EdgeSelection::Refs(refs),
                distance: native::number(distance, "chamfer distance")?,
                other_distance: other_distance.map(|d| native::number(d, "second chamfer distance")).transpose()?,
            };
            self.native(state, tag, OpKind::Chamfer, body, &[body], native, None, edges)
        })
    }

    fn shell_all_faces(&self, body: BodyId, thickness: f64) -> KernelResult<()> {
        self.observe("shell", || {
            let state = self.state()?;
            state.solid(body)?;
            let base = state.get(body)?.result.clone();
            let native = shell::all_faces(state.step(body)?, thickness)?;
            let step = json!({"op":"native","base":base,"native":native});
            let (state, evaluated) = self.evaluate(state, OpKind::Shell, &[body], &[], step)?;
            drop(state);
            let empty = match &evaluated.app {
                Outcome::Exact(app) => is_empty(app),
                Outcome::Approximate(outcome) => outcome.body().definition().faces.is_empty(),
            };
            if empty {
                Ok(())
            } else {
                Err(KernelError::Failed {
                    op: OpKind::Shell,
                    code: 0,
                    message: "opening every face left material behind".into(),
                })
            }
        })
    }

    fn shell(&self, tag: &OpTag, body: BodyId, open: &[FaceId], thickness: f64) -> KernelResult<OpResult> {
        self.observe("shell", || {
            let state = self.state()?;
            let solid = state.solid(body)?;
            let operation = shell::operation(state.step(body)?, open, thickness, |face| solid.face_ref(face, body))?;
            self.native(state, tag, OpKind::Shell, body, &[body], operation, None, &[])
        })
    }

    fn extrude_to(
        &self,
        tag: &OpTag,
        profile: BodyId,
        target: BodyId,
        direction: Option<Vec3>,
        limit: &ExtrudeLimit,
    ) -> KernelResult<OpResult> {
        self.observe("extrude_to", || {
            let state = self.state()?;
            if state.get(profile)?.solid.is_some() {
                return Err(KernelError::InvalidInput("extrude_to needs a profile".into()));
            }
            let solid = state.solid(target)?;
            let (target_step, profile_step) = (state.step(target)?, state.step(profile)?);
            let exact = direction.map(|d| native::vector(d, "sweep direction")).transpose()?;
            let native = match limit {
                ExtrudeLimit::UpToNext => catallax_recipe::Operation::ExtrudeNext {
                    target: target_step,
                    profile: profile_step,
                    direction: exact,
                },
                ExtrudeLimit::UpToFace { face } => catallax_recipe::Operation::ExtrudeTo {
                    target: target_step,
                    profile: profile_step,
                    direction: exact,
                    limit: solid.face_ref(*face, target)?,
                },
            };
            let sweep = json!({"profile": state.get(profile)?.result, "direction": direction});
            let base = state.get(target)?.result.clone();
            let step = json!({"op":"native","base":base,"native":native,"sweep":sweep});
            // The swept solid's history starts at the profile: the target is only read.
            self.operation_reading(state, tag, OpKind::ExtrudeTo, &[profile], &[target], &[], step)
        })
    }

    fn hole(&self, tag: &OpTag, body: BodyId, params: &Hole) -> KernelResult<OpResult> {
        self.observe("hole", || {
            let state = self.state()?;
            let solid = state.solid(body)?;
            let native = native::hole(state.step(body)?, &solid, body, params)?;
            self.native(state, tag, OpKind::Hole, body, &[body], native, None, &[])
        })
    }

    fn offset(&self, tag: &OpTag, body: BodyId, distance: f64) -> KernelResult<OpResult> {
        self.observe("offset", || {
            let state = self.state()?;
            state.solid(body)?;
            let operation = distance::offset(state.step(body)?, distance)?;
            self.native(state, tag, OpKind::Offset, body, &[body], operation, None, &[])
        })
        .map_err(|error| match error {
            KernelError::Unavailable(_) => distance::refusal(OpKind::Offset, ""),
            KernelError::Unsupported(message) if message == NOTHING_LEFT => KernelError::InvalidInput(
                "Offset at this distance would remove the whole body. Use a smaller inward distance".into(),
            ),
            other => other,
        })
    }

    fn thicken(&self, tag: &OpTag, body: BodyId, params: &Thicken) -> KernelResult<OpResult> {
        self.observe("thicken", || {
            let state = self.state()?;
            let solid = state.solid(body)?;
            let operation = distance::thicken(state.step(body)?, params, |face| solid.face_ref(face, body))?;
            self.native(state, tag, OpKind::Thicken, body, &[body], operation, None, &[])
        })
        .map_err(|error| match error {
            KernelError::Unavailable(_) => distance::refusal(OpKind::Thicken, ""),
            other => other,
        })
    }

    fn pattern_feature(
        &self,
        tag: &OpTag,
        body: BodyId,
        source: BodyId,
        before: Option<BodyId>,
        layout: &PatternLayout,
    ) -> KernelResult<OpResult> {
        self.observe("pattern_feature", || {
            let state = self.state()?;
            state.solid(body)?;
            let source_body = state.get(source)?;
            let before_body = before.map(|id| state.get(id)).transpose()?;
            let graph = source_body
                .steps
                .keys()
                .filter(|id| before_body.is_none_or(|b| !b.steps.contains_key(id)))
                .map(|id| format!("op_{id}"))
                .collect::<Vec<_>>();
            if graph.is_empty() {
                return Err(feature_pattern::refusal("pattern.expansion"));
            }
            let mut context = vec![source];
            context.extend(before);
            let step = json!({"op":"feature_pattern", "base":state.get(body)?.result,
                "source":source_body.result, "before":before_body.map(|b| &b.result), "graph":graph, "layout":layout});
            self.operation_reading(state, tag, OpKind::Pattern, &[body], &context, &[], step)
        })
        .map_err(|error| match error {
            KernelError::Unavailable(_) => feature_pattern::refusal(""),
            other => other,
        })
    }

    fn pattern(&self, tag: &OpTag, body: BodyId, layout: &PatternLayout) -> KernelResult<OpResult> {
        self.observe("pattern", || {
            let state = self.state()?;
            state.solid(body)?;
            let native = catallax_recipe::Operation::Pattern {
                body: state.step(body)?,
                layout: native::pattern_layout(layout)?,
            };
            self.native(state, tag, OpKind::Pattern, body, &[body], native, None, &[])
        })
    }

    fn mirror(&self, tag: &OpTag, body: BodyId, params: &Mirror) -> KernelResult<OpResult> {
        self.observe("mirror", || {
            let state = self.state()?;
            state.solid(body)?;
            let native = catallax_recipe::Operation::Mirror {
                body: state.step(body)?,
                plane: native::plane(params.plane_origin, params.plane_normal, "mirror plane")?,
                keep_original: params.keep_original,
            };
            self.native(state, tag, OpKind::Mirror, body, &[body], native, None, &[])
        })
    }

    fn draft(&self, tag: &OpTag, body: BodyId, faces: &[FaceId], params: &Draft) -> KernelResult<OpResult> {
        self.observe("draft", || {
            let state = self.state()?;
            let solid = state.solid(body)?;
            if faces.is_empty() {
                return Err(KernelError::InvalidInput("a draft needs at least one face".into()));
            }
            let native = catallax_recipe::Operation::Draft {
                body: state.step(body)?,
                faces: faces.iter().map(|f| solid.face_ref(*f, body)).collect::<KernelResult<Vec<_>>>()?,
                pull: native::vector(params.pull, "pull direction")?,
                neutral: native::plane(params.neutral_origin, params.neutral_normal, "neutral plane")?,
                angle: native::number(params.angle_deg, "draft angle")?,
                keep_regions: None,
            };
            self.native(state, tag, OpKind::Draft, body, &[body], native, None, &[])
        })
    }

    fn lattice(&self, tag: &OpTag, body: BodyId, params: &Lattice) -> KernelResult<OpResult> {
        self.observe("lattice", || self.build_lattice(tag, body, params))
    }

    fn gyroid(&self, tag: &OpTag, body: BodyId, params: &Gyroid) -> KernelResult<OpResult> {
        self.observe("gyroid", || self.build_gyroid(tag, body, params))
    }

    fn export_field_stl(&self, body: BodyId) -> KernelResult<FieldExport> {
        self.observe_export("export_field_stl", || self.field(body)?.ok_or(KernelError::UnknownBody(body))?.export())
    }

    fn field_mass(&self, body: BodyId, accuracy: &FieldMassAccuracy) -> KernelResult<FieldMass> {
        self.observe("field_mass", || self.field(body)?.ok_or(KernelError::UnknownBody(body))?.mass(accuracy))
    }

    fn export_field_stand_in(&self, body: BodyId, accuracy: &FieldMassAccuracy) -> KernelResult<StandInExport> {
        self.observe_export("export_field_stand_in", || {
            self.field(body)?.ok_or(KernelError::UnknownBody(body))?.stand_in(accuracy)
        })
    }

    fn independent_check(&self, body: BodyId) -> KernelResult<IndependentCheck> {
        self.check_independently(body)
    }

    fn print_mesh(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Option<PrintMesh>> {
        self.observe_export("print_mesh", || self.qualified_print(body, params).map(Some))
    }

    fn export_field_stl_at(&self, body: BodyId, accuracy_mm: f64) -> KernelResult<FieldExport> {
        self.observe_export("export_field_stl", || {
            self.field(body)?.ok_or(KernelError::UnknownBody(body))?.export_at(Some(accuracy_mm))
        })
    }

    fn tessellate(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Mesh> {
        self.observe("tessellate", || {
            if self.field(body)?.is_some() {
                return Err(KernelError::Unsupported("field meshes require labelled certified export".into()));
            }
            if let Some(approximate) = self.approximate(body)? {
                return approximate.mesh(&options(params)?);
            }
            let solid = self.solid(body)?;
            let options = options(params)?;
            let mesh = solid.display(&options, "tessellate")?;
            let rounding_budget = options.chord_tolerance.to_f64_nearest();
            if mesh.positions.iter().any(|p| {
                let rounded = p.map(|v| f64::from(v as f32));
                !rounded.iter().all(|v| v.is_finite()) || Vec3::from(*p).distance(rounded.into()) > rounding_budget
            }) {
                return Err(KernelError::Unsupported(
                    "requested chord tolerance cannot survive f32 display rounding is not available in the Catallax application entry".into(),
                ));
            }
            Ok(convert::mesh(&mesh, &solid.ids, params))
        })
    }

    fn tessellate_edges(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        self.observe("tessellate_edges", || {
            if self.field(body)?.is_some() {
                return Ok(vec![]);
            }
            if let Some(approximate) = self.approximate(body)? {
                return approximate.edges(&options(params)?);
            }
            let solid = self.solid(body)?;
            let mesh = solid.display(&options(params)?, "tessellate_edges")?;
            Ok(mesh
                .edges
                .iter()
                .map(|edge| EdgePolyline {
                    edge: solid.ids.edges[&edge.edge],
                    points: edge.vertices.iter().map(|v| mesh.positions[*v].into()).collect(),
                })
                .collect())
        })
    }

    fn measure_work(&self, run: &mut dyn FnMut()) -> KernelResult<Option<u64>> {
        let (used, exhausted) = if catallax_numbers::operation_budget_active() {
            let before = catallax_numbers::current_operation_work().used;
            run();
            let after = catallax_numbers::current_operation_work();
            (after.used.saturating_sub(before), after.exhausted)
        } else if let Some(diagnostics) = &self.diagnostics {
            let before = diagnostics.work_used();
            run();
            return Ok(before.zip(diagnostics.work_used()).map(|(before, after)| after.saturating_sub(before)));
        } else {
            let (_, work) = catallax_numbers::with_operation_budget(catallax_app::DEFAULT_BUILD_WORK, run);
            (work.used, work.exhausted)
        };
        if exhausted {
            Err(KernelError::Unavailable("numbers.operation-budget: operation work allowance exhausted".into()))
        } else {
            Ok(Some(used))
        }
    }

    fn reserve_proof_work(&self) -> KernelResult<Box<dyn ProofWork>> {
        construction::reserve_proof_work()
    }

    fn construction_edit(
        &self,
        body: BodyId,
        edits: &[ScalarEdit],
        params: &TessellationParams,
    ) -> KernelResult<ConstructionEdit> {
        self.observe("construction_edit", || construction::prepare(self, body, edits, params))
    }

    fn rendering(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Mesh> {
        if let Some(approximate) = self.approximate(body)? {
            return self.observe("preview", || approximate.mesh(&options(params)?));
        }
        if self.field(body)?.is_none() {
            let solid = self.solid(body)?;
            if solid.has_splines() {
                options(params)?;
                return self.observe("preview", || solid.rendering_mesh());
            }
        }
        self.preview(body, params)
    }

    fn rendering_edges(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        if self.field(body)?.is_some() {
            return self.preview_edges(body, params);
        }
        if let Some(approximate) = self.approximate(body)? {
            return self.observe("preview_edges", || approximate.edges(&options(params)?));
        }
        let solid = self.solid(body)?;
        if solid.has_splines() {
            options(params)?;
            return self.observe("preview_edges", || {
                let answer = solid.rendering()?;
                let mesh = match answer.as_ref() {
                    catallax_numbers::OperationResult::Proven(answer) => &answer.value.geometry,
                    catallax_numbers::OperationResult::Bounded(answer) => &answer.value().geometry,
                    catallax_numbers::OperationResult::Refused(refusal) => {
                        return Err(query_error("preview_edges", refusal.clone()));
                    }
                };
                Ok(mesh
                    .edges
                    .iter()
                    .map(|edge| EdgePolyline {
                        edge: solid.ids.edges[&edge.edge],
                        points: edge.vertices.iter().map(|v| mesh.positions[*v].into()).collect(),
                    })
                    .collect())
            });
        }

        self.preview_edges(body, params)
    }

    fn display_bound(&self, body: BodyId) -> KernelResult<Option<DisplayBound>> {
        self.observe("display_bound", || {
            if self.field(body)?.is_some() || self.approximate(body)?.is_some() {
                return Ok(None);
            }
            let solid = self.solid(body)?;
            if !solid.has_splines() {
                return Ok(None);
            }
            Ok(solid.rendering_mesh()?.bounded)
        })
    }

    fn preview(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Mesh> {
        self.observe("preview", || {
            if let Some(field) = self.field(body)? {
                return Ok(field.preview());
            }
            if let Some(approximate) = self.approximate(body)? {
                return approximate.mesh(&options(params)?);
            }
            let solid = self.solid(body)?;
            let options = options(params)?;
            if let Some((mesh, _)) = solid.handed_previews.get(&preview_key(&display_key(&options))) {
                return Ok(mesh.clone());
            }
            self.state()?.fresh_previews += 1;
            // Where Catallax cannot sample a preview, the screen shows the certified mesh.
            let preview = solid.preview(&options, "preview")?;
            let Some(mesh) = preview.as_ref() else {
                return self.tessellate(body, params);
            };
            if mesh.positions.iter().flatten().any(|v| !(*v as f32).is_finite()) {
                return Err(KernelError::Unsupported(
                    "preview coordinates beyond f32 display range is not available in the Catallax application entry"
                        .into(),
                ));
            }
            Ok(convert::preview(mesh, &solid.ids))
        })
    }

    fn preview_edges(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        self.observe("preview_edges", || {
            if self.field(body)?.is_some() {
                return Ok(vec![]);
            }
            if let Some(approximate) = self.approximate(body)? {
                return approximate.edges(&options(params)?);
            }
            let solid = self.solid(body)?;
            let options = options(params)?;
            if let Some((_, edges)) = solid.handed_previews.get(&preview_key(&display_key(&options))) {
                return Ok(edges.clone());
            }
            let preview = solid.preview(&options, "preview_edges")?;
            let Some(mesh) = preview.as_ref() else {
                return self.tessellate_edges(body, params);
            };
            Ok(preview_edges(mesh, &solid.ids))
        })
    }

    fn mass_properties(&self, body: BodyId) -> KernelResult<MassProperties> {
        self.observe("mass_properties", || {
            if self.field(body)?.is_some() {
                return Err(KernelError::Unsupported("a field's moments need an explicit field mass request".into()));
            }
            if let Some(approximate) = self.approximate(body)? {
                return approximate.mass_properties();
            }
            let solid = self.solid(body)?;
            if solid.app.is_surface() {
                return Err(KernelError::Unsupported(
                    "A surface body encloses no volume, so it has no mass properties.".into(),
                ));
            }
            solid
                .mass
                .get_or_init(|| {
                    let built = solid.app.checkpoint.built();
                    let accuracy = catallax_measure::Accuracy::default();
                    let mass = numerical("mass_properties", |dec| {
                        // A fitted blend has no second moments, so it reports no inertia.
                        if built.model.blend_base(built.result).is_some() {
                            let measured =
                                catallax_measure::volume_area_centroid(&built.model, built.result, &accuracy, dec)?;
                            return Ok(catallax_numbers::Answer::with_marks(
                                catallax_measure::MassProperties {
                                    volume: measured.value.volume,
                                    area: measured.value.area,
                                    centroid: measured.value.centroid,
                                    inertia: None,
                                },
                                measured.marks,
                            ));
                        }
                        catallax_measure::mass_properties(&built.model, built.result, &accuracy, dec)
                    })
                    .map_err(|e| query_error("mass_properties", e))?
                    .value;
                    Ok(MassProperties {
                        volume: convert::midpoint(&mass.volume),
                        area: convert::midpoint(&mass.area),
                        centroid: mass.centroid.map(|c| Vec3::from(c.each_ref().map(convert::midpoint))).ok_or_else(
                            || {
                                KernelError::Unsupported(
                                    "centroid of an empty body is not available in the Catallax application entry"
                                        .into(),
                                )
                            },
                        )?,
                        inertia: mass.inertia.map(|i| i.map(|row| row.each_ref().map(convert::midpoint))),
                    })
                })
                .clone()
        })
    }

    fn cached_mass_properties(&self, body: BodyId) -> Option<MassProperties> {
        if let Some(approximate) = self.approximate(body).ok()? {
            return approximate.cached_mass();
        }
        self.solid(body).ok()?.mass.get()?.as_ref().ok().copied()
    }

    fn validate(&self, body: BodyId) -> KernelResult<Validation> {
        self.observe("validate", || {
            if let Some(approximate) = self.approximate(body)? {
                return Ok(approximate.validation());
            }
            let solid = self.solid(body)?;
            let built = solid.app.checkpoint.built();
            let faults = built.model.check(built.result);
            Ok(Validation {
                valid: faults.is_empty(),
                kind: if solid.app.is_surface() { BodyKind::Sheet } else { BodyKind::Solid },
                manifold_solid: faults.is_empty() && !solid.app.is_surface(),
                issues: faults.iter().map(|f| format!("{f:?}")).collect(),
            })
        })
    }

    fn topology(&self, body: BodyId) -> KernelResult<Topology> {
        self.observe("topology", || {
            if let Some(field) = self.field(body)? {
                return Ok(field.topology());
            }
            if let Some(approximate) = self.approximate(body)? {
                return approximate.topology();
            }
            let solid = self.solid(body)?;
            // Logical topology uses the initial screen mesh, independent of later refinement requests.
            if let Some(value) = solid.topology.get() {
                return value.clone();
            }
            self.state()?.fresh_topologies += 1;
            if solid.has_splines() {
                let answer = solid.rendering()?;
                let mesh = match answer.as_ref() {
                    catallax_numbers::OperationResult::Proven(answer) => &answer.value,
                    catallax_numbers::OperationResult::Bounded(answer) => answer.value(),
                    catallax_numbers::OperationResult::Refused(refusal) => {
                        return Err(query_error("topology", refusal.clone()));
                    }
                };
                return solid
                    .topology
                    .get_or_init(|| convert::rendering_topology(&solid.app, &solid.ids, mesh))
                    .clone();
            }
            let mesh = solid.display(&MeshOptions::default(), "topology")?;
            solid.topology.get_or_init(|| convert::topology(&solid.app, &solid.ids, &mesh, false)).clone()
        })
    }

    fn face_carriers(&self, body: BodyId) -> KernelResult<Vec<FaceCarrier>> {
        self.observe("face_carriers", || {
            let solid = self.solid(body)?;
            convert::face_carriers(&solid.app, &solid.ids)
        })
    }

    fn measured_topology(&self, body: BodyId) -> KernelResult<Topology> {
        self.observe("measured_topology", || {
            // An approximate body's measures are estimates on its display either way.
            if let Some(approximate) = self.approximate(body)? {
                return approximate.topology();
            }
            let solid = self.solid(body)?;
            if let Some(value) = solid.measured.get() {
                return value.clone();
            }
            let mesh = solid.display(&MeshOptions::default(), "measured_topology")?;
            solid.measured.get_or_init(|| convert::topology(&solid.app, &solid.ids, &mesh, true)).clone()
        })
    }

    fn edge_length(&self, body: BodyId, edge: EdgeId) -> KernelResult<Measurement> {
        self.observe("edge_length", || match self.approximate(body)? {
            Some(approximate) => approximate.edge_length(edge),
            None => measure::edge_length(self, body, edge),
        })
    }

    fn face_area(&self, body: BodyId, face: FaceId) -> KernelResult<Measurement> {
        self.observe("face_area", || match self.approximate(body)? {
            Some(approximate) => approximate.face_area(face),
            None => measure::face_area(self, body, face),
        })
    }

    fn edge_midpoint(&self, body: BodyId, edge: EdgeId) -> KernelResult<PointProof> {
        self.observe("edge_midpoint", || measure::edge_midpoint(self, body, edge))
    }

    fn measure_pair(
        &self,
        body: BodyId,
        a: PairEndpoint,
        b: PairEndpoint,
        quantity: PairQuantity,
    ) -> KernelResult<Measurement> {
        self.observe("measure_pair", || measure::measure_pair(self, body, a, b, quantity))
    }

    fn face_support_plane(&self, body: BodyId, face: FaceId) -> KernelResult<ExactAxisPlane> {
        self.observe("face_support_plane", || assembly_measure::face_support_plane(self, body, face))
    }

    fn entity_support(&self, body: BodyId, entity: SupportEntity) -> KernelResult<ExactSupport> {
        self.observe("entity_support", || assembly_measure::entity_support(self, body, entity))
    }

    fn entity_support_enclosed(&self, body: BodyId, entity: SupportEntity) -> KernelResult<EnclosedSupport> {
        self.observe("entity_support_enclosed", || assembly_measure::enclosed_support(self, body, entity))
    }

    fn history(&self, body: BodyId) -> KernelResult<Vec<HistoryRecord>> {
        self.observe("history", || Ok(self.state()?.get(body)?.history.clone()))
    }

    fn release(&self, body: BodyId) -> KernelResult<()> {
        self.observe("release", || {
            let mut state = self.state()?;
            state.fields.remove(&body);
            state.bodies.remove(&body).map(|_| ()).ok_or(KernelError::UnknownBody(body))
        })
    }

    /// The recipe text, the history, cached certified meshes and previews, plus a
    /// fixed allowance per B-rep face and edge of every body the build kept.
    /// Catallax does not report its model's size, so the allowance is an estimate.
    fn retained_bytes(&self, body: BodyId) -> KernelResult<u64> {
        if let Some(field) = self.field(body)? {
            return Ok(field.retained_bytes());
        }
        const FACE: usize = 2048;
        const EDGE: usize = 512;
        let mesh_bytes = |m: &catallax_mesh::Mesh| {
            m.positions.len() * 24
                + m.triangles.len() * 32
                + m.faces.len() * 64
                + m.edges.iter().map(|e| 64 + e.vertices.len() * 8).sum::<usize>()
        };
        let state = self.state()?;
        let stored = state.get(body)?;
        let recipe: usize = stored.steps.values().map(|s| s.to_string().len()).sum();
        let history: usize = stored.history.iter().map(|r| 256 + r.face_roles.len() * 32).sum();
        let mut bytes = recipe + history;
        if let Some(approximate) = &stored.approximate {
            bytes += approximate.retained_bytes();
        }
        if let Some(solid) = &stored.solid {
            let brep = match solid.app.unread_bytes() {
                Some(unread) => unread,
                None => {
                    solid.app.checkpoint.built().bodies.len().max(1)
                        * (solid.ids.faces.len() * FACE + solid.ids.edges.len() * EDGE)
                }
            };
            bytes += brep;
            solid.display.each(|mesh| bytes += mesh_bytes(mesh));
            solid.print.each(|output| {
                let mesh = &output.mesh;
                bytes += output.stl.len()
                    + (mesh.positions.len() + mesh.normals.len()) * 12
                    + mesh.indices.len() * 4
                    + mesh.triangle_faces.len() * 8;
            });
            solid.previews.each(|m| {
                let Some(m) = m else { return };
                bytes += m.positions.len() * 24
                    + m.triangles.len() * 32
                    + m.faces.len() * 64
                    + m.edges.iter().map(|e| 64 + e.vertices.len() * 8).sum::<usize>()
            });
        }
        Ok(bytes as u64)
    }

    fn view_curves(&self, body: BodyId, spec: &drawing::ViewSpec) -> KernelResult<drawing::ViewCurves> {
        self.observe("view_curves", || drawing_queries::view_curves(self, body, spec))
    }
    /// A section for the screen is cut from the preview mesh, with no boolean; a
    /// certified one subtracts a half-space, as every backend's default does. Either is
    /// observed as one drawing query: its mesh or scratch boolean and the cut body's
    /// view are nested in this call's log and work scope, so a drawing's section costs
    /// what it costs on the display side.
    fn section_view(&self, body: BodyId, spec: &drawing::SectionSpec) -> KernelResult<drawing::SectionView> {
        self.observe("section_view", || {
            if spec.view.preview {
                let view = drawing_queries::resolved_view(self, body, &spec.view);
                drawing::section::by_mesh_slice(self, body, &drawing::SectionSpec { view, ..*spec })
            } else {
                drawing::section::by_half_space(self, body, spec)
            }
        })
    }
    fn silhouette_curves(&self, _: BodyId, _: Vec3) -> KernelResult<Vec<drawing::ViewCurve>> {
        self.observe("silhouette_curves", || Err(gap("Exact silhouettes")))
    }
    fn section_curves(&self, _: BodyId, _: &Plane) -> KernelResult<Vec<drawing::ViewCurve>> {
        self.observe("section_curves", || Err(gap("Section curves")))
    }
    fn edge_curve(&self, body: BodyId, edge: EdgeId) -> KernelResult<drawing::Curve3> {
        self.observe("edge_curve", || drawing_queries::edge_curve(self.solid(body)?.as_ref(), body, edge))
    }
    fn face_loops(&self, body: BodyId, face: FaceId) -> KernelResult<Vec<Vec<drawing::Curve3>>> {
        self.observe("face_loops", || drawing_queries::face_loops(self.solid(body)?.as_ref(), body, face))
    }
    fn lines_and_arcs(&self, _: &drawing::Curve3, _: f64) -> KernelResult<Vec<drawing::Curve3>> {
        self.observe("lines_and_arcs", || Err(gap("Curve approximation for drawings")))
    }

    fn export_iges(&self, body: BodyId, options: &IgesOptions) -> KernelResult<IgesExport> {
        self.observe_export("export_iges", || {
            let write = || iges::export(&self.solid(body)?.app, options);
            lattice::export_allowance("export_iges", write).map(|(export, _)| export)
        })
    }

    fn export_step(&self, body: BodyId, options: &StepOptions) -> KernelResult<StepExport> {
        self.observe_export("export_step", || {
            let write = || match self.approximate(body)? {
                Some(approximate) => approximate.export_step(options),
                None => step::export(&self.solid(body)?.app, options),
            };
            lattice::export_allowance("export_step", write).map(|(export, _)| export)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certified_display_is_cached_after_a_refused_request_retries() {
        let kernel = CatallaxKernel::new();
        let body = contract::cylinder(&kernel, Vec3::ZERO, 5.0, 10.0);
        let solid = kernel.solid(body).unwrap();
        let retained = kernel.retained_bytes(body).unwrap();
        let mut meshes = 0;
        solid.display.each(|_| meshes += 1);
        assert_eq!(meshes, 0, "building and accounting must not request a mesh");
        let options = MeshOptions::default();
        let (refused, work) = catallax_numbers::with_operation_budget(0, || kernel.topology(body));
        assert!(work.exhausted);
        assert!(matches!(refused, Err(KernelError::QueryFailed { message, .. })
            if message.contains("numbers.operation-budget")));
        let (mesh, work) = catallax_numbers::with_operation_budget(100_000_000, || solid.display(&options, "test"));
        assert!(!work.exhausted);
        assert!(work.used > 0);
        let mesh = mesh.unwrap();
        let (cached, work) = catallax_numbers::with_operation_budget(0, || solid.display(&options, "test"));
        assert!(!work.exhausted);
        assert_eq!(work.used, 0);
        assert!(Arc::ptr_eq(&mesh, &cached.unwrap()));
        assert!(kernel.retained_bytes(body).unwrap() > retained);
        assert!(kernel.topology(body).is_ok(), "a mesh refusal must not poison the topology cache");
    }

    #[test]
    fn face_carriers_match_the_topology_and_build_no_mesh() {
        let kernel = CatallaxKernel::new();
        let block = contract::block(&kernel);
        let pin = contract::cylinder(&kernel, Vec3::new(1.0, 1.0, 1.0), 0.5, 3.0);
        let cut = kernel.boolean(&OpTag::new("cut"), BooleanOp::Subtract, block, pin).unwrap().body;
        for body in [block, pin, cut] {
            let carriers = kernel.face_carriers(body).unwrap();
            let mut meshes = 0;
            kernel.solid(body).unwrap().display.each(|_| meshes += 1);
            assert_eq!(meshes, 0, "carriers come from the model");
            let topology = kernel.topology(body).unwrap();
            let ids = |mut faces: Vec<FaceId>| {
                faces.sort();
                faces
            };
            assert_eq!(
                ids(carriers.iter().map(|f| f.id).collect()),
                ids(topology.faces.iter().map(|f| f.id).collect())
            );
            for carrier in &carriers {
                let face = topology.face(carrier.id).unwrap();
                assert_eq!(
                    (carrier.surface, carrier.normal, &carrier.origins),
                    (face.surface, face.normal, &face.origins)
                );
                // The offset a sketch plane takes, bit for bit, wherever a rebuild reads it here.
                if let (Some(normal), Some(point)) = (carrier.normal, carrier.on_plane) {
                    let (exact, display) = (point.dot(normal), face.centroid.dot(normal));
                    assert!((exact - display).abs() < 1e-9, "{}", carrier.id);
                    if exact != 0.0 {
                        assert_eq!(exact.to_bits(), display.to_bits(), "{}", carrier.id);
                    }
                } else {
                    assert_eq!((carrier.normal, carrier.on_plane), (None, None));
                }
            }
        }
    }

    #[test]
    fn retained_prefixes_branch_and_refuse_without_replaying_or_changing_earlier_bodies() {
        let kernel = CatallaxKernel::with_operation_log();
        let base = contract::block(&kernel);
        let base_recipe = kernel.body_recipe(base).unwrap();
        let options = StepOptions::default();
        let original = kernel.export_step(base, &options).unwrap();
        for x in [-8.0, 8.0] {
            let profile = kernel
                .create_body_from_profile(
                    &OpTag::new("tool_sketch"),
                    &Profile::rectangle(Plane::XY.with_origin(Vec3::new(x, 0.0, 0.0)), Point2::new(0.0, 0.0), 4.0, 6.0),
                )
                .unwrap();
            let tool = kernel
                .extrude(&OpTag::new("tool"), profile.body, &Extrude { distance: 10.0, direction: None })
                .unwrap()
                .body;
            kernel.take_diagnostics().unwrap();
            let cut = kernel.boolean(&OpTag::new("cut"), BooleanOp::Subtract, base, tool).unwrap().body;
            let evidence = kernel.take_diagnostics().unwrap();
            let recipe = kernel.body_recipe(cut).unwrap();
            let (Outcome::Exact(replayed), steps, _, _) =
                build(&recipe, OpKind::Subtract, None, &Default::default(), None, &Default::default(), &[]).unwrap()
            else {
                panic!("an exact cut")
            };
            let operations = evidence["calls"][0]["operation_log"]["operations"].as_array().unwrap();
            let result = steps[recipe["result"].as_str().unwrap()].as_str();
            assert!(operations.iter().any(|op| op["id"] == result), "{evidence}");
            for step in base_recipe["steps"].as_array().unwrap() {
                let old = steps[step["id"].as_str().unwrap()].as_str();
                assert!(!operations.iter().any(|op| op["id"] == old), "replayed {old}: {evidence}");
            }
            let solid = kernel.solid(cut).unwrap();
            assert_eq!(solid.app.names, replayed.names);
            assert_eq!(kernel.export_step(cut, &options).unwrap(), step::export(&replayed, &options).unwrap());
            assert!((kernel.mass_properties(cut).unwrap().volume - 11760.0).abs() < 1e-6);

            // The later-created target is not the leading operand. This refusal
            // must neither consume nor mutate the earlier body's checkpoint.
            let error = kernel.boolean(&OpTag::new("empty"), BooleanOp::Subtract, tool, base).unwrap_err();
            assert!(matches!(error, KernelError::Unsupported(_)), "{error}");
            assert_eq!(kernel.export_step(base, &options).unwrap(), original);
        }
        assert_eq!(kernel.mass_properties(base).unwrap().volume, 12000.0);
    }

    #[test]
    fn limited_sweeps_reuse_read_only_targets_without_claiming_their_history() {
        let kernel = CatallaxKernel::with_operation_log();
        let target = contract::block(&kernel);
        let original = kernel.export_step(target, &StepOptions::default()).unwrap();
        let target_recipe = kernel.body_recipe(target).unwrap();
        let bottom =
            kernel.topology(target).unwrap().faces.iter().find(|face| face.normal == Some(-Vec3::Z)).unwrap().id;
        let profile = kernel
            .create_body_from_profile(
                &OpTag::new("post"),
                &Profile::circle(Plane::XY.with_origin(Vec3::new(0.0, 0.0, -5.0)), Point2::new(0.0, 0.0), 3.0),
            )
            .unwrap()
            .body;

        for limit in [ExtrudeLimit::UpToNext, ExtrudeLimit::UpToFace { face: bottom }] {
            kernel.take_diagnostics().unwrap();
            let swept = kernel.extrude_to(&OpTag::new("sweep"), profile, target, None, &limit).unwrap();
            let evidence = kernel.take_diagnostics().unwrap();
            let operations = evidence["calls"][0]["operation_log"]["operations"].as_array().unwrap();
            for step in target_recipe["steps"].as_array().unwrap() {
                let id = &step["id"];
                assert!(!operations.iter().any(|op| &op["id"] == id), "rebuilt read-only target {id}: {evidence}");
            }
            let recipe = kernel.body_recipe(swept.body).unwrap();
            let (Outcome::Exact(fresh), _, _, _) =
                build(&recipe, OpKind::ExtrudeTo, None, &Default::default(), None, &Default::default(), &[]).unwrap()
            else {
                panic!("an exact sweep")
            };
            assert_eq!(kernel.solid(swept.body).unwrap().app.names, fresh.names);
            assert_eq!(
                kernel.export_step(swept.body, &StepOptions::default()).unwrap(),
                step::export(&fresh, &StepOptions::default()).unwrap()
            );
            assert!((kernel.mass_properties(swept.body).unwrap().volume - 45.0 * std::f64::consts::PI).abs() < 1e-6);
            let history = kernel.history(swept.body).unwrap();
            assert_eq!(history.iter().map(|record| record.tag.as_str()).collect::<Vec<_>>(), ["post", "sweep"]);
            assert!(swept.record.faces.deleted.is_empty());
            assert_eq!(kernel.export_step(target, &StepOptions::default()).unwrap(), original);
        }
    }
}
