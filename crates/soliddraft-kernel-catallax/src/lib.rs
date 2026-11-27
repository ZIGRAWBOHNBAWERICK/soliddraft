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
mod face_areas;
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
mod pmi;
mod proof_clock;
mod recognition;
mod request;
mod scan;
mod shell;
mod step;
mod sweep;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use catallax_app::{Error, EvaluatedApplicationOutcome, NumericalApplicationBody};

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

use request::RequestPolicy;
pub use request::{APP_REQUEST_WORK, APP_WORK_TARGET, EDIT_STOPGAP_WORK, RequestWork, with_request_allowance};

/// The pinned Catallax application entry revision.
pub const CATALLAX_REVISION: &str = "e15c27e19bab4a2f341682138e7c9ab3be3b181e";

/// An in-process Catallax backend. There is no global kernel state or kernel thread.
#[derive(Default)]
pub struct CatallaxKernel {
    state: Mutex<State>,
    diagnostics: Option<diagnostics::Diagnostics>,
    recent_areas: face_areas::Recent,
}

#[derive(Clone, Copy)]
struct RebuildSource {
    kernel: *const CatallaxKernel,
    previous: BodyId,
    input: Option<BodyId>,
    after: u64,
}

thread_local! {
    static REBUILD_SOURCE: std::cell::Cell<Option<RebuildSource>> = const { std::cell::Cell::new(None) };
}

#[derive(Default)]
struct State {
    next: u64,
    bodies: BTreeMap<BodyId, Stored>,
    fields: BTreeMap<BodyId, Arc<lattice::Field>>,
    field_source_support: BTreeMap<BodyId, bool>,
    capture: bool,
    questions: Vec<Value>,
    /// Imported STEP sources by SHA-256. Catallax reads them again for every build
    /// that continues an imported body, even when the import itself is reused.
    files: BTreeMap<String, Arc<[u8]>>,
    /// Default STEP admission shared by the file's body imports and repair preview.
    tolerant_imports: BTreeMap<String, Arc<catallax_import::tolerant::opened::OpenedStep>>,
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
    /// The early display meshes handed over already, by recipe key and display key.
    sent_previews: std::collections::BTreeSet<(String, String)>,
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

impl Stored {
    fn legacy_recipe(&self) -> Value {
        json!({"format":"soliddraft-recipe/1", "name":"SolidDraft",
            "steps":self.steps.values().collect::<Vec<_>>(), "result":self.result})
    }
}

struct Solid {
    app: AppBody,
    /// The recipe key of this build, for a hand-over of its topology.
    key: String,
    /// Early display meshes another kernel sampled for this build, by display key. More
    /// arrive when that kernel samples the build at other display settings.
    handed_previews: Mutex<handover::Previews>,
    /// The kernel instance that handed this build over, if one did.
    source: Option<u64>,
    ids: convert::Ids,
    display: MeshCache<catallax_mesh::Mesh>,
    print: MeshCache<PrintMesh>,
    /// `None` where Catallax refused to sample a preview, so it is not asked again.
    previews: MeshCache<Option<PreviewMesh>>,
    rendering: MeshCache<catallax_numbers::OperationResult<catallax_mesh::DisplayMesh>>,
    area_prefix: Weak<Solid>,
    areas: face_areas::Areas,
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

fn screen_deviation(display: &catallax_mesh::DisplayMesh) -> Option<Rational> {
    if display.geometry.positions.iter().flatten().any(|value| !(*value as f32).is_finite()) {
        return None;
    }
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
    let rounding = Rational::from_f64_exact(rounding)?;
    Some(&display.measurement.maximum_deviation + &rounding)
}

impl Solid {
    /// The early display meshes another kernel handed over for this build.
    fn handed_previews(&self) -> MutexGuard<'_, handover::Previews> {
        self.handed_previews.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn has_splines(&self) -> bool {
        if let Some(handed) = self.app.unread() {
            return handed.splines;
        }
        let built = self.app.checkpoint.built();
        built.model.faces_of_body(built.result).iter().any(|face| {
            matches!(built.model.geom().surface(built.model.face(*face).surface), catallax_shapes::Surface::BSpline(_))
        })
    }

    fn rendering_answer(
        &self,
        dec: &mut Decider,
    ) -> Result<catallax_numbers::OperationResult<catallax_mesh::DisplayMesh>, Error> {
        use catallax_numbers::OperationResult;
        let mut options = MeshOptions::default();
        let answer = self.app.display_result(&options, dec)?;
        let OperationResult::Proven(proven) = &answer else { return Ok(answer) };
        if screen_deviation(&proven.value).is_none_or(|bound| bound <= options.chord_tolerance) {
            return Ok(answer);
        }

        let extent = proven.value.geometry.positions.iter().flatten().fold(0.0_f64, |max, p| max.max(p.abs()));
        let extent = Rational::from_f64_exact(extent).expect("screen deviation requires finite coordinates");
        // Allow one chord on either side of the first mesh. The L1 rounding error
        // of a vertex uses at most three binary32 half ulps at that extent.
        let reserve = (&extent + &options.chord_tolerance.mul_pow2(1)).mul_pow2(-22);
        let chord = &options.chord_tolerance - &reserve;
        if chord <= Rational::zero() {
            return Ok(answer);
        }
        options.chord_tolerance = chord;
        self.app.display_result(&options, dec)
    }

    fn rendering(&self) -> KernelResult<Arc<catallax_numbers::OperationResult<catallax_mesh::DisplayMesh>>> {
        use catallax_numbers::{Answer, OperationResult, Refusal};
        self.rendering.get(display_key(&MeshOptions::default()), || {
            let run = || numerical("display_result", |dec| match self.rendering_answer(dec) {
                Ok(OperationResult::Refused(refusal)) => Err(refusal),
                Ok(answer) => Ok(Answer::exact(answer)),
                Err(Error::Display(refusal)) => Err(refusal),
                Err(error) => Err(Refusal::new(error.code(), error.to_string())),
            });
            let (result, work) = RequestPolicy::Display.run(run);
            let (used, limit) = (work.total_used, work.request.limit);
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
        let bound = screen_deviation(display).ok_or_else(|| {
            KernelError::Unsupported("The display coordinates cannot be represented on screen".into())
        })?;
        let level = MeshOptions::default().chord_tolerance;
        if bound > level {
            return Err(KernelError::Unsupported(
                "Spline display bound (SD-DISPLAY-03): The measured deviation exceeds 0.01 mm after screen-coordinate rounding. The existing part is retained".into(),
            ));
        }
        let mut mesh = convert::preview(&display.geometry, &self.ids);
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
            recent_areas: face_areas::Recent::default(),
        }
    }

    /// A backend whose builds of `ops` come back as numerical bodies, as Catallax will
    /// return rolling-ball fillets. It stands in for Catallax's numerical results in
    /// tests until Catallax returns them itself.
    #[cfg(feature = "numerical-stand-in")]
    pub fn with_numerical_stand_in(ops: &[OpKind]) -> Self {
        Self { state: Mutex::new(State { stand_in: ops.to_vec(), ..State::default() }), ..Self::default() }
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
        let recipe = stored.legacy_recipe();
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

    fn rebuild_source(&self) -> Option<RebuildSource> {
        REBUILD_SOURCE.get().filter(|source| std::ptr::eq(source.kernel, self))
    }

    fn advance_rebuild_source(&self, seq: u64) {
        if let Some(mut source) = self.rebuild_source() {
            source.after = seq;
            REBUILD_SOURCE.set(Some(source));
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
        let Evaluated {
            app,
            key,
            handed,
            names,
            steps,
            mut history,
            parents,
            approximate_parent,
            area_prefix,
            seq,
            result,
        } = evaluated;
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
                let before = approximate_parent.as_deref();
                let approximate =
                    approximate::Approximate::new(outcome, &parent_ids, before, &tags, selected_edges, &mut state.next);
                let record = approximate.record(seq, tag, op, &parent_ids, before);
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
            && let Some(refusal) = stopped_short(app.checkpoint.built(), &result)
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
            handed_previews: Mutex::new(handed_previews),
            ids,
            display,
            print: MeshCache::new(),
            previews: MeshCache::new(),
            rendering: MeshCache::new(),
            area_prefix,
            areas: face_areas::Areas::default(),
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
        // An approximate body's recipe could replay here as an exact build, so neither an
        // input nor a context body may be one. Profiles store no body and pass. A hole in a
        // numerical build is the exception: Catallax drills it through its construction for
        // bodies with tolerant entities, and a replay that comes back exact is refused below.
        let approximate_of = |body: &BodyId| state.bodies.get(body).and_then(|s| s.approximate.as_ref());
        if context.iter().any(|body| approximate_of(body).is_some())
            || inputs.iter().any(|body| approximate_of(body).is_some_and(|a| op != OpKind::Hole || !a.numerical()))
        {
            return Err(approximate::not_yet());
        }
        let approximate_parent = inputs.iter().find_map(|body| approximate_of(body).cloned());
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
        self.advance_rebuild_source(seq);
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
        let retained = self
            .rebuild_source()
            .and_then(|source| state.bodies.get(&source.previous))
            .and_then(|stored| stored.solid.clone());
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
            &parents,
            prefix.as_ref().map(|solid| &solid.app),
            retained.as_ref().map(|solid| &solid.app),
            &files,
            capture.then_some(&mut questions),
            &offered,
            &stand_in,
        );
        // An approximate input never yields a body passed off as exact.
        let built = built.and_then(|built| match built.0 {
            Outcome::Exact(_) if approximate_parent.is_some() => Err(approximate::not_yet()),
            _ => Ok(built),
        });
        // A build made here is kept for hand-over outside the lock; an offered one is not.
        let kept = match &built {
            Ok((Outcome::Exact(app), _, key, _, _)) if keeping && matches!(app.checkpoint, Exact::Checkpoint(_)) => {
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
        if matches!(&built, Ok((Outcome::Exact(app), _, _, _, _)) if matches!(app.checkpoint, Exact::Checkpoint(_)))
            || matches!(&built, Ok((Outcome::Approximate(_), _, _, _, _)))
        {
            state.fresh += 1;
        }
        let (app, names, key, handed, continued) = built?;
        let area_prefix = if continued { prefix.as_ref().map(Arc::downgrade).unwrap_or_default() } else { Weak::new() };
        Ok((
            state,
            Evaluated {
                app,
                key,
                handed,
                names,
                steps,
                history,
                parents,
                approximate_parent,
                area_prefix,
                seq,
                result,
            },
        ))
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
    approximate_parent: Option<Arc<approximate::Approximate>>,
    area_prefix: Weak<Solid>,
    seq: u64,
    result: String,
}

const NOTHING_LEFT: &str = "this would leave nothing of the part";

/// A fillet whose rolling contact ended below the radius it was asked for. Catallax
/// then builds the strip at the smaller radius where the ball reached a face boundary
/// and keeps the request in `requested_radius`. SolidDraft never builds a radius other
/// than the one the feature records, so it refuses as Catallax did before, naming the
/// largest radius that fits. A radius that consumes a face exactly still builds.
fn stopped_short(built: &catallax_features::Built, step: &str) -> Option<KernelError> {
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
    catallax_recipe::FORMAT_V6.clone_into(&mut converted.recipe.format);
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
            | catallax_recipe::Operation::ExtrudeFrom { direction: d, .. }
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

/// A build's outcome, step ids, hand-over key and topology, and whether it
/// preserved the selected checkpoint's arena.
type BuildAnswer =
    (Outcome, BTreeMap<String, catallax_recipe::StepId>, String, Option<Arc<handover::HandedTopology>>, bool);

/// Builds a legacy recipe, returning the body and the legacy step names' Catallax step ids.
#[allow(clippy::too_many_arguments)]
fn build(
    input: &Value,
    op: OpKind,
    parents: &[Arc<Solid>],
    prefix: Option<&AppBody>,
    retained: Option<&AppBody>,
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
            return Ok((Outcome::Exact(app), converted.steps, key, offer.topology.clone(), false));
        }
        // An offer whose bytes cannot be read is passed over; the build is evaluated instead.
        if let Ok(built) = offer.built()
            && let Some(app) = AppBody::adopted(built, &converted.recipe)
        {
            return Ok((Outcome::Exact(app), converted.steps, key, offer.topology.clone(), false));
        }
    }
    // An offered or adopted body has no local continuation; don't read its geometry for one.
    // Locally built checkpoints use the same default decider policy below.
    let prefix = prefix.and_then(|app| app.checkpoint.continuation());
    let (leading, keeps_arena) = prefix.map_or((0, false), |checkpoint| {
        let prior = checkpoint.built().recipe();
        if converted.recipe.format != prior.format {
            return (0, false);
        }
        let shared = prior.steps.iter().zip(&converted.recipe.steps).take_while(|(old, new)| old == new).count();
        (shared, shared == prior.steps.len())
    });
    let retained = retained.and_then(|app| app.checkpoint.continuation()).filter(|checkpoint| {
        let prior = checkpoint.built().recipe();
        prior.format == converted.recipe.format
            && prior.steps.len().min(converted.recipe.steps.len()) > leading
            && prior.steps.iter().zip(&converted.recipe.steps).take(leading + 1).all(|(old, new)| old == new)
    });
    // Face areas can follow the ordinary input's arena, not a rewound edit source.
    let continued = retained.is_none() && keeps_arena;
    let prefix = retained.or(prefix).cloned();
    // Catallax tries the exact construction first and returns a numerical body, built
    // within its stated tolerance, where only that construction succeeds. Either is kept
    // as Catallax built it, never turned into a refusal or the other kind.
    let mut decider = Decider::default();
    let (outcome, work) = RequestPolicy::Edit.run(|| {
        catallax_app::build_incremental_with_files_without_display(&converted.recipe, prefix, files, &mut decider)
    });
    if work.exhausted_here() && work.request.limit == request::EDIT_STOPGAP_WORK {
        return Err(KernelError::Failed { op, code: 0, message: request::EDIT_STOPGAP_REFUSAL.into() });
    }
    let outcome = if work.exhausted_here() { Err(Error::ResourceLimit) } else { outcome }.map_err(|e| {
        let spline = input["steps"].as_array()
            .and_then(|steps| steps.iter().find(|step| step["id"] == input["result"]))
            .and_then(|step| step["path"]["segments"].as_array())
            .is_some_and(|segments| segments.iter().any(|s| s["kind"] == "spline"));
        if op == OpKind::Sweep && spline && e.code() == "features.unsupported-path" {
            KernelError::InvalidInput("This spline sweep was refused because it needs one spline segment and a profile that keeps its direction".into())
        } else if op == OpKind::Fillet
            && let Some(reaches) = fillet_reaches(&e, parents)
        {
            reaches
        } else {
            build_error(op, e)
        }
    })?;
    // Checked before anything else reads the build, so no stand-in hides it.
    if op == OpKind::Fillet
        && let EvaluatedApplicationOutcome::Exact(app) = &outcome
        && let Some(refusal) = stopped_short(app.checkpoint.built(), input["result"].as_str().unwrap_or_default())
    {
        return Err(refusal);
    }
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
    Ok((outcome.into(), converted.steps, key, None, continued))
}

/// What a Catallax fillet refusal writes before each face of the input body the fillet
/// would run into, followed by the face's handle as Catallax prints it: "; reaches face #12".
const REACHES: &str = "; reaches face #";

/// What a Catallax `fillet.radius-too-large` refusal writes before the faces a ball of
/// that radius cannot touch at once, as a JSON list of each face's origins:
/// `; limiting contact supports [[{"step":"op_3",...}],[{"step":"op_45",...}]]`.
const LIMITING_SUPPORTS: &str = "; limiting contact supports ";

/// A fillet Catallax refused because it would run into faces other than those beside
/// its edges, or because no ball of its radius touches two faces at once, with those
/// faces as the input body's ids. `None` when the refusal names no such face, or names
/// one the input body does not have.
fn fillet_reaches(error: &Error, parents: &[Arc<Solid>]) -> Option<KernelError> {
    let Error::Build(catallax_features::BuildError::Geometry { refusal, .. }) = error else { return None };
    let handles = reached_handles(&refusal.detail);
    let (faces, between) = if handles.is_empty() {
        let supports = limiting_supports(&refusal.detail)?;
        let mut faces = Vec::new();
        for support in &supports {
            let made = parents.iter().map(|parent| parent.ids.faces_made_by(support)).find(|made| !made.is_empty())?;
            for face in made {
                if !faces.contains(&face) {
                    faces.push(face);
                }
            }
        }
        (faces, true)
    } else {
        let faces = handles
            .iter()
            .map(|index| {
                parents.iter().find_map(|parent| {
                    parent.ids.faces.iter().find_map(|(cat, face)| (cat.index() == *index).then_some(*face))
                })
            })
            .collect::<Option<Vec<FaceId>>>()?;
        (faces, false)
    };
    if faces.is_empty() {
        return None;
    }
    let radius_limit_mm = refusal
        .detail
        .split("; radius limit ")
        .skip(1)
        .filter_map(|rest| rest.split_once(" mm")?.0.parse::<f64>().ok())
        .filter(|limit| limit.is_finite() && *limit > 0.0)
        .min_by(f64::total_cmp);
    Some(KernelError::FilletReaches { faces, between, radius_limit_mm, message: format!("{}: {error}", error.code()) })
}

/// The origins of each face a refusal names after [`LIMITING_SUPPORTS`], first list only.
fn limiting_supports(detail: &str) -> Option<Vec<Vec<catallax_recipe::Origin>>> {
    let (_, rest) = detail.split_once(LIMITING_SUPPORTS)?;
    let mut list = serde_json::Deserializer::from_str(rest).into_iter::<Vec<Vec<catallax_recipe::Origin>>>();
    list.next()?.ok().filter(|supports| !supports.is_empty())
}

/// The face handles a refusal names after [`REACHES`], in order, each once.
fn reached_handles(detail: &str) -> Vec<u32> {
    let mut handles = Vec::new();
    for rest in detail.split(REACHES).skip(1) {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(index) = digits.parse()
            && !handles.contains(&index)
        {
            handles.push(index);
        }
    }
    handles
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

fn edge_proof_run(
    report: catallax_app::EdgeProofReport,
    started: Option<u64>,
    id: impl Fn(catallax_app::EdgeProofKey) -> Option<EdgeId>,
) -> KernelResult<EdgeProofRun> {
    use catallax_app::EdgeProofOutcome;

    let cpu_ns = started.zip(proof_clock::proof_cpu_ns()).and_then(|(start, end)| end.checked_sub(start));
    let edges = report
        .edges
        .into_iter()
        .map(|row| {
            let edge = id(row.edge).ok_or_else(|| KernelError::QueryFailed {
                query: "prove_edges".into(),
                code: -1,
                message: "A reported edge has no interface identity".into(),
            })?;
            let bound_mm = match row.outcome {
                EdgeProofOutcome::Proved { upper_mm } => {
                    let mut bound = upper_mm.to_f64_nearest();
                    if Rational::from_f64_exact(bound).is_some_and(|value| value < upper_mm) {
                        bound = bound.next_up();
                    }
                    Some(bound)
                }
                EdgeProofOutcome::CouldNotProve(_) => None,
            };
            Ok(EdgeProof { edge, bound_mm })
        })
        .collect::<KernelResult<_>>()?;
    // Both Catallax whole-edge routines run serially on the calling thread.
    Ok(EdgeProofRun { edges, work_used: report.work, cpu_ns, threads_used: 1 })
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
        self.observe("body_relation", || RequestPolicy::Query.run(|| body_relation::relation(self, a, b, clearance)).0)
    }

    fn propose_import(&self, source: &[u8], options: &ImportOptions) -> KernelResult<ProposalSet> {
        self.observe("propose_import", || import::propose(self, ImportFormat::Step, source, options))
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
        self.observe("propose_import", || import::propose(self, format, source, options))
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
            if solid.key.is_empty() || topologies.contains_key(&solid.key) {
                continue;
            }
            // The early display meshes sampled so far travel with the topology. Once it
            // has gone, the ones sampled since go with it again, as a display change asks.
            let previews: Vec<_> = solid
                .previews
                .computed()
                .into_iter()
                .filter_map(|(key, preview)| {
                    let key = preview_key(&key);
                    if state.sent_previews.contains(&(solid.key.clone(), key.clone())) {
                        return None;
                    }
                    let mesh = preview.as_ref().as_ref()?;
                    Some(handover::HandedPreview {
                        key,
                        mesh: convert::preview(mesh, &solid.ids),
                        edges: preview_edges(mesh, &solid.ids),
                    })
                })
                .collect();
            if state.sent_topologies.contains(&solid.key) && previews.is_empty() {
                continue;
            }
            let (faces, edges, vertices) = solid.ids.by_index();
            let pairs = |ids: Vec<(u32, u64)>| ids.into_iter().map(|(index, raw)| (raw, index)).collect();
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
            if let Some(topology) = &entry.topology {
                state.sent_topologies.insert(entry.key.clone());
                for preview in &topology.previews {
                    state.sent_previews.insert((entry.key.clone(), preview.key.clone()));
                }
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
                // One sent again carries early display meshes sampled since, which join the
                // offer's and those of every body already built from the same recipe.
                handover::Received::Topology(topology) => {
                    for solid in state.bodies.values().filter_map(|stored| stored.solid.as_deref()) {
                        if solid.key != key {
                            continue;
                        }
                        let (faces, edges, _) = solid.ids.by_index();
                        if let Some(previews) = topology.renamed_previews(&faces, &edges) {
                            solid.handed_previews().extend(previews);
                        }
                    }
                    if let Some(offer) = state.offered.get_mut(&key) {
                        match &mut offer.topology {
                            Some(earlier) => Arc::make_mut(earlier).previews.extend(topology.previews.iter().cloned()),
                            None => offer.topology = Some(topology),
                        }
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
        let unread: Vec<handover::Offer> =
            self.state()?.offered.values().filter(|offer| !offer.is_read() && !offer.has_identity()).cloned().collect();
        for offer in unread.iter().take(at_most) {
            let _ = offer.built();
        }
        Ok(unread.len().saturating_sub(at_most))
    }

    fn source_solids(&self, format: ImportFormat, source: &[u8]) -> KernelResult<Vec<String>> {
        self.observe("source_solids", || import::source_solids(self, format, source))
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
            let mut step = json!({
                "op":"profile", "plane":profile.plane,
                "loops":{"outer":profile.outer,"holes":profile.holes}
            });
            let (seq, result) = construction::profile_identity(self, &mut state, tag, &mut step);
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
            let mut step = json!({"op":"native_profile","native":native});
            let (seq, result) = construction::profile_identity(self, &mut state, tag, &mut step);
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

    fn extrude_from(
        &self,
        tag: &OpTag,
        profile: BodyId,
        target: BodyId,
        direction: Option<Vec3>,
        start: FaceId,
        distance: f64,
    ) -> KernelResult<OpResult> {
        self.observe("extrude_from", || {
            let state = self.state()?;
            if state.get(profile)?.solid.is_some() {
                return Err(KernelError::InvalidInput("extrude_from needs a profile".into()));
            }
            let solid = state.solid(target)?;
            let (target_step, profile_step) = (state.step(target)?, state.step(profile)?);
            let exact = direction.map(|d| native::vector(d, "sweep direction")).transpose()?;
            let native = catallax_recipe::Operation::ExtrudeFrom {
                target: target_step,
                profile: profile_step,
                direction: exact,
                start: solid.face_ref(start, target)?,
                distance: native::number(distance, "extrusion distance")?,
            };
            let sweep = json!({"profile": state.get(profile)?.result, "direction": direction});
            let base = state.get(target)?.result.clone();
            let step = json!({"op":"native","base":base,"native":native,"sweep":sweep});
            // As for a limited sweep, the start face's body is only read.
            self.operation_reading(state, tag, OpKind::ExtrudeFrom, &[profile], &[target], &[], step)
        })
    }

    fn hole(&self, tag: &OpTag, body: BodyId, params: &Hole) -> KernelResult<OpResult> {
        self.observe("hole", || {
            let state = self.state()?;
            // A field body is refused by `solid` as a terminal field.
            let approximate = if state.fields.contains_key(&body) {
                None
            } else {
                state.get(body)?.approximate.clone().filter(|approximate| approximate.numerical())
            };
            let native = match &approximate {
                Some(approximate) => {
                    native::hole(state.step(body)?, native::Drilled::Approximate(approximate), body, params)?
                }
                None => {
                    let solid = state.solid(body)?;
                    native::hole(state.step(body)?, native::Drilled::Exact(&solid), body, params)?
                }
            };
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

    fn supports_field_source(&self, body: BodyId) -> bool {
        self.check_field_source(body).unwrap_or(false)
    }

    fn lattice(&self, tag: &OpTag, body: BodyId, params: &Lattice) -> KernelResult<OpResult> {
        self.observe("lattice", || self.build_lattice(tag, body, params))
    }

    fn gyroid(&self, tag: &OpTag, body: BodyId, params: &Gyroid) -> KernelResult<OpResult> {
        self.observe("gyroid", || self.build_gyroid(tag, body, params))
    }

    fn field_grade(&self, field: GradedField, kind: GradeKind) -> KernelResult<()> {
        lattice::grade_support(field, kind)
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
                return Err(KernelError::Unsupported("field meshes come only from the labelled field export".into()));
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
        if !request::active()
            && let Some(diagnostics) = &self.diagnostics
        {
            let before = diagnostics.work_used();
            run();
            return Ok(before.zip(diagnostics.work_used()).map(|(before, after)| after.saturating_sub(before)));
        }
        let (_, work) = RequestPolicy::Edit.run(run);
        if work.request.exhausted {
            Err(KernelError::Unavailable("numbers.operation-budget: operation work allowance exhausted".into()))
        } else {
            Ok(Some(work.request.used))
        }
    }

    fn with_rebuild_source(&self, body: BodyId, input: Option<BodyId>, run: &mut dyn FnMut()) {
        struct Restore(Option<RebuildSource>);
        impl Drop for Restore {
            fn drop(&mut self) {
                REBUILD_SOURCE.set(self.0);
            }
        }
        let source = RebuildSource { kernel: self, previous: body, input, after: 0 };
        let _restore = Restore(REBUILD_SOURCE.replace(Some(source)));
        run();
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
            if let Some((mesh, _)) = solid.handed_previews().get(&preview_key(&display_key(&options))) {
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
            if let Some((_, edges)) = solid.handed_previews().get(&preview_key(&display_key(&options))) {
                return Ok(edges.clone());
            }
            let preview = solid.preview(&options, "preview_edges")?;
            let Some(mesh) = preview.as_ref() else {
                return self.tessellate_edges(body, params);
            };
            Ok(preview_edges(mesh, &solid.ids))
        })
    }

    fn prove_edges(&self, body: BodyId, work_limit: u64, interrupt: &Interrupt) -> KernelResult<EdgeProofRun> {
        use std::sync::atomic::{AtomicBool, Ordering};

        use catallax_topology::numerical::edge_proof::{EdgeProofKey, prove_exact_edges, prove_numerical_edges};

        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let _registration = interrupt.register(Box::new(move || stop.store(true, Ordering::Relaxed)));
        let started = proof_clock::proof_cpu_ns();
        if let Some(approximate) = self.approximate(body)? {
            let Ok(numerical) = approximate.body() else {
                // A checked tolerant import is not a proof of its source geometry.
                return Ok(EdgeProofRun { edges: Vec::new(), work_used: 0, cpu_ns: Some(0), threads_used: 0 });
            };
            let limit = catallax_topology::Tolerance::new(Rational::parse("0.01").expect("constant decimal"))
                .expect("positive whole-edge proof limit");
            let report = prove_numerical_edges(numerical, &limit, work_limit, &cancelled);
            return edge_proof_run(report, started, |key| match key {
                EdgeProofKey::Numerical(key) => approximate.edges.get(&key).copied(),
                EdgeProofKey::Exact(_) => None,
            });
        }
        // Use the retained model directly: a body handed over from the browser worker
        // may not have an evaluation checkpoint in this kernel.
        let solid = self.solid(body)?;
        let built = solid.app.checkpoint.built();
        let report = prove_exact_edges(&built.model, built.result, work_limit, &cancelled);
        edge_proof_run(report, started, |key| match key {
            EdgeProofKey::Exact(key) => solid.ids.edges.get(&key).copied(),
            EdgeProofKey::Numerical(_) => None,
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
        let reuse = !request::active() && self.diagnostics.as_ref().is_none_or(diagnostics::Diagnostics::uncapped);
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
                    .get_or_init(|| {
                        let areas = reuse.then(|| self.recent_areas.query(&solid));
                        convert::rendering_topology(&solid.app, &solid.ids, mesh, areas.as_ref())
                    })
                    .clone();
            }
            let mesh = solid.display(&MeshOptions::default(), "topology")?;
            solid
                .topology
                .get_or_init(|| {
                    let areas = reuse.then(|| self.recent_areas.query(&solid));
                    convert::topology(&solid.app, &solid.ids, &mesh, false, areas.as_ref())
                })
                .clone()
        })
    }

    fn face_carriers(&self, body: BodyId) -> KernelResult<Vec<FaceCarrier>> {
        self.observe("face_carriers", || {
            let solid = self.solid(body)?;
            convert::face_carriers(&solid.app, &solid.ids)
        })
    }

    fn measured_topology(&self, body: BodyId) -> KernelResult<Topology> {
        let reuse = !request::active() && self.diagnostics.as_ref().is_none_or(diagnostics::Diagnostics::uncapped);
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
            solid
                .measured
                .get_or_init(|| {
                    let areas = reuse.then(|| self.recent_areas.query(&solid));
                    convert::topology(&solid.app, &solid.ids, &mesh, true, areas.as_ref())
                })
                .clone()
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
            state.field_source_support.remove(&body);
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
                let mut section = drawing::section::by_mesh_slice(self, body, &drawing::SectionSpec { view, ..*spec })?;
                section.curves.unproved = true;
                Ok(section)
            } else {
                drawing_queries::check_exact_tolerance(&spec.view)?;
                drawing_queries::drawing_solid(self, body, "section_view")?;
                drawing::section::by_half_space(self, body, spec)
            }
        })
    }
    fn silhouette_curves(&self, body: BodyId, direction: Vec3) -> KernelResult<Vec<drawing::ViewCurve>> {
        self.observe("silhouette_curves", || drawing_queries::silhouette_curves(self, body, direction))
    }
    fn section_curves(&self, body: BodyId, plane: &Plane) -> KernelResult<Vec<drawing::ViewCurve>> {
        self.observe("section_curves", || drawing_queries::section_curves(self, body, plane))
    }
    fn edge_curve(&self, body: BodyId, edge: EdgeId) -> KernelResult<drawing::Curve3> {
        Ok(self.edge_drawing(body, edge)?.curve)
    }
    fn edge_drawing(&self, body: BodyId, edge: EdgeId) -> KernelResult<drawing::ViewCurve> {
        self.observe("edge_curve", || drawing_queries::edge_drawing(self.solid(body)?.as_ref(), body, edge))
    }
    fn face_loops(&self, body: BodyId, face: FaceId) -> KernelResult<Vec<Vec<drawing::Curve3>>> {
        Ok(self
            .face_drawing_loops(body, face)?
            .into_iter()
            .map(|chain| chain.into_iter().map(|edge| edge.curve).collect())
            .collect())
    }
    fn face_drawing_loops(&self, body: BodyId, face: FaceId) -> KernelResult<Vec<Vec<drawing::ViewCurve>>> {
        self.observe("face_loops", || drawing_queries::face_drawing_loops(self.solid(body)?.as_ref(), body, face))
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
                None => step::export(&self.solid(body)?.app, options, None),
            };
            lattice::export_allowance("export_step", write).map(|(export, _)| export)
        })
    }

    fn recognise(&self, body: BodyId) -> KernelResult<soliddraft_kernel_api::recognition::Recognised> {
        self.observe("recognise", || recognition::recognise(&*self.solid(body)?))
    }

    fn pmi_nominals(&self, body: BodyId, pmi: &soliddraft_kernel_api::pmi::Pmi<FaceId>) -> KernelResult<Vec<Vec<f64>>> {
        let solid = self.solid(body)?;
        pmi::nominals(&solid, body, pmi)
    }

    fn export_step_with_pmi(
        &self,
        body: BodyId,
        options: &StepOptions,
        pmi: &soliddraft_kernel_api::pmi::Pmi<FaceId>,
    ) -> KernelResult<StepExport> {
        self.observe_export("export_step", || {
            // A body built within a tolerance has no exact faces to attach to yet.
            if self.approximate(body)?.is_some() {
                return Err(KernelError::Unsupported("export_step_with_pmi".into()));
            }
            let solid = self.solid(body)?;
            let on_model = pmi::on_model(&solid, body, pmi)?;
            let write = || step::export(&solid.app, options, Some(&on_model));
            lattice::export_allowance("export_step", write).map(|(export, _)| export)
        })
    }

    fn read_step_pmi(&self, source: &[u8]) -> KernelResult<soliddraft_kernel_api::pmi::StepPmi> {
        pmi::read(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fillet_refusal_names_each_face_it_reaches_once() {
        let detail = "exact path refused fillet.radius-too-large: the ball reaches a face; radius limit 4 mm; \
                      reaches face #12; numerical continuation refused fillet.radius-too-large: the same face; \
                      reaches face #12; reaches face #3.";
        assert_eq!(reached_handles(detail), [12, 3]);
        assert!(reached_handles("fillet.unsupported-overflow: a face; reaches face #").is_empty());
    }

    /// The faces a ball cannot touch at once, as Catallax names them by origin before
    /// the terminal radius limit.
    #[test]
    fn a_fillet_refusal_names_the_faces_it_does_not_fit_between() {
        let detail = r#"no ball of this radius rolls along the edge against both faces; maximum unknown; limiting contact supports [[{"step":"op_3","role":{"side":{"segment":"l0s2"}}}],[{"step":"op_45","role":{"side":{"segment":"l0s1"}}}]]; radius limit 3.999 mm"#;
        let supports = limiting_supports(detail).unwrap();
        assert_eq!(supports.len(), 2);
        assert_eq!(supports[0][0].step.as_str(), "op_3");
        assert_eq!(supports[1][0].step.as_str(), "op_45");
        assert!(limiting_supports("fillet.radius-too-large: a; radius limit 4 mm").is_none());
        assert!(limiting_supports("a; limiting contact supports not json; radius limit 4 mm").is_none());
    }

    /// Checkpoint 2, Q32: the field draws only field-only features, lattices and gyroids.
    /// Every other body, early preview included, is drawn from the boundary model.
    #[test]
    fn only_field_only_features_preview_from_the_field() {
        use std::collections::BTreeSet;
        let kernel = CatallaxKernel::new();
        let params = TessellationParams::default();
        let block = contract::block(&kernel);
        let pin = contract::cylinder(&kernel, Vec3::new(1.0, 1.0, 1.0), 0.5, 3.0);
        let cut = kernel.boolean(&OpTag::new("cut"), BooleanOp::Subtract, block, pin).unwrap().body;
        let top: Vec<EdgeId> =
            kernel.topology(block).unwrap().edges.iter().filter(|e| e.bbox.min.z > 10.0 - 1e-6).map(|e| e.id).collect();
        let rounded = kernel.fillet(&OpTag::new("round"), block, &top, 2.0).unwrap().body;
        for body in [block, pin, cut, rounded] {
            assert!(kernel.field(body).unwrap().is_none(), "{body:?} has no field stage");
            let preview = kernel.preview(body, &params).unwrap();
            let drawn: BTreeSet<FaceId> = preview.triangle_faces.iter().copied().collect();
            let faces: BTreeSet<FaceId> = kernel.topology(body).unwrap().faces.iter().map(|f| f.id).collect();
            assert_eq!(drawn, faces, "{body:?} is previewed face by face from its boundary model");
        }
        let lattice = kernel
            .lattice(
                &OpTag::new("lattice"),
                block,
                &Lattice {
                    cell: LatticeCell::Cubic,
                    cell_size_mm: 5.0,
                    radius_mm: Grade::Constant(1.0),
                    frame: FieldFrame::Part,
                    accuracy_mm: 0.1,
                    min_detail_mm: 0.5,
                },
            )
            .unwrap()
            .body;
        let gyroid = kernel
            .gyroid(
                &OpTag::new("gyroid"),
                block,
                &Gyroid {
                    kind: GyroidKind::Network,
                    period_mm: 10.0,
                    level: Grade::Constant(0.0),
                    frame: FieldFrame::Part,
                    accuracy_mm: 0.1,
                    min_detail_mm: 0.5,
                },
            )
            .unwrap()
            .body;
        for body in [lattice, gyroid] {
            let field = kernel.field(body).unwrap().expect("a field-only feature keeps its field stage");
            let preview = kernel.preview(body, &params).unwrap();
            assert!(preview.unproved && preview.triangle_count() > 0);
            assert_eq!(preview.positions, field.preview().positions, "{body:?} is drawn from its field sample");
        }
    }

    #[test]
    fn an_exact_view_of_a_cold_cylinder_builds_no_display_mesh() {
        let kernel = CatallaxKernel::new();
        let body = contract::cylinder(&kernel, Vec3::ZERO, 5.0, 10.0);
        let view = kernel.view_curves(body, &drawing::ViewSpec::standard(drawing::StandardView::Top)).unwrap();
        assert!(view.catallax && !view.unproved);
        assert_eq!(kernel.state().unwrap().fresh_topologies, 0);
        let mut meshes = 0;
        kernel.solid(body).unwrap().display.each(|_| meshes += 1);
        assert_eq!(meshes, 0);
    }

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

    fn area_proofs(kernel: &CatallaxKernel) -> usize {
        kernel.take_diagnostics().unwrap()["calls"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|call| call["operation_log"]["operations"].as_array().unwrap())
            .filter(|entry| entry["operation"] == "face_area")
            .count()
    }

    fn uncached_rendering_topology(solid: &Solid) -> Topology {
        let mesh = solid.rendering().unwrap();
        let mesh = match mesh.as_ref() {
            catallax_numbers::OperationResult::Proven(answer) => &answer.value,
            catallax_numbers::OperationResult::Bounded(answer) => answer.value(),
            catallax_numbers::OperationResult::Refused(refusal) => panic!("{refusal}"),
        };
        convert::rendering_topology(&solid.app, &solid.ids, mesh, None).unwrap()
    }

    #[test]
    fn unchanged_face_areas_survive_a_cut_and_feed_both_topology_views() {
        fn cut(kernel: &CatallaxKernel) -> (BodyId, BodyId) {
            let block = contract::block(kernel);
            let pin = contract::cylinder(kernel, Vec3::new(1.0, 1.0, 8.0), 0.5, 3.0);
            let cut = kernel.boolean(&OpTag::new("cut"), BooleanOp::Subtract, block, pin).unwrap().body;
            (block, cut)
        }

        let kernel = CatallaxKernel::with_operation_log();
        let (stock, body) = cut(&kernel);
        let solid = kernel.solid(body).unwrap();
        let mesh = solid.display(&MeshOptions::default(), "test").unwrap();
        let expected = convert::topology(&solid.app, &solid.ids, &mesh, false, None).unwrap();
        let expected_measured = convert::topology(&solid.app, &solid.ids, &mesh, true, None).unwrap();
        kernel.topology(stock).unwrap();
        area_proofs(&kernel);

        let actual = kernel.topology(body).unwrap();
        assert_eq!(serde_json::to_vec(&actual).unwrap(), serde_json::to_vec(&expected).unwrap());
        let recomputed = area_proofs(&kernel);
        assert!(recomputed > 0 && recomputed < actual.faces.len());
        let measured = kernel.measured_topology(body).unwrap();
        assert_eq!(serde_json::to_vec(&measured).unwrap(), serde_json::to_vec(&expected_measured).unwrap());
        assert_eq!(area_proofs(&kernel), 0);
    }

    #[test]
    fn unchanged_face_areas_reuse_the_start_cap_of_a_rebuilt_spline_sweep() {
        let kernel = CatallaxKernel::with_operation_log();
        let profile = Profile::rectangle(Plane::XY, [9.0, -2.0].into(), 4.0, 2.0);
        let sheet = kernel.create_body_from_profile(&OpTag::new("profile"), &profile).unwrap().body;
        let sweep = |height| Sweep {
            start: [0.0; 3],
            orientation: SweepOrientation::Fixed,
            segments: vec![PathSegment::Spline {
                id: "rail".into(),
                degree: 3,
                control_points: vec![[0.0; 3], [2.0, 0.0, height], [2.0, 2.0, 2.0 * height], [0.0, 2.0, 3.0 * height]],
                weights: vec![1.0; 4],
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            }],
        };
        let run = |height| kernel.sweep(&OpTag::new("sweep"), sheet, &sweep(height)).unwrap().body;
        let before = run(10.0);
        kernel.topology(before).unwrap();
        assert_eq!(area_proofs(&kernel), 6);

        let after = run(20.0);
        let solid = kernel.solid(after).unwrap();
        let expected = uncached_rendering_topology(&solid);
        let actual = kernel.topology(after).unwrap();
        assert_eq!(area_proofs(&kernel), 5, "only the start cap of the revised sweep is unchanged");
        assert!(serde_json::to_vec(&actual).unwrap() == serde_json::to_vec(&expected).unwrap());
    }

    #[test]
    fn unchanged_face_areas_keep_loft_patch_enclosures_through_a_fillet() {
        let kernel = CatallaxKernel::with_operation_log();
        let section = |height, points: [Point2; 4]| Profile {
            plane: Plane { origin: Vec3::new(0.0, 0.0, height), ..Plane::XY },
            outer: Loop((0..4).map(|i| Segment::Line { from: points[i], to: points[(i + 1) % 4] }).collect()),
            holes: vec![],
        };
        let point = |(x, y)| Point2::new(x, y);
        let bottom = section(0.0, [(-0.1, -0.1), (0.1, -0.1), (0.1, 0.1), (-0.1, 0.1)].map(point));
        let top = section(0.2, [(0.0, -0.1), (0.1, 0.0), (0.0, 0.2), (-0.2, 0.0)].map(point));
        let a = kernel.create_body_from_profile(&OpTag::new("bottom"), &bottom).unwrap().body;
        let b = kernel.create_body_from_profile(&OpTag::new("top"), &top).unwrap().body;
        let loft = kernel.loft(&OpTag::new("loft"), &[a, b], LoftShape::Ruled).unwrap();
        kernel.topology(loft.body).unwrap();
        assert_eq!(area_proofs(&kernel), 6);

        let edge = loft
            .record
            .edges
            .created
            .iter()
            .copied()
            .find(|&edge| {
                kernel.edge_midpoint(loft.body, edge).is_ok_and(|point| {
                    let [x, y, z] = point.coordinates.map(|coordinate| coordinate.value);
                    Vec3::new(x, y, z).distance(Vec3::new(0.1, -0.05, 0.1)) < 1e-10
                })
            })
            .unwrap();
        let body = kernel.fillet(&OpTag::new("round"), loft.body, &[edge], 0.01).unwrap().body;
        let solid = kernel.solid(body).unwrap();
        let expected = uncached_rendering_topology(&solid);
        let actual = kernel.topology(body).unwrap();
        assert_eq!(area_proofs(&kernel), 5, "the fillet leaves two of the loft's side faces intact");
        assert!(serde_json::to_vec(&actual).unwrap() == serde_json::to_vec(&expected).unwrap());
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
            let (Outcome::Exact(replayed), steps, ..) =
                build(&recipe, OpKind::Subtract, &[], None, None, &Default::default(), None, &Default::default(), &[])
                    .unwrap()
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
            assert_eq!(kernel.export_step(cut, &options).unwrap(), step::export(&replayed, &options, None).unwrap());
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
    fn edited_cut_reuses_a_profile_beyond_the_ordinary_input_checkpoint() {
        let kernel = CatallaxKernel::with_operation_log();
        let base = contract::block(&kernel);
        let profile = Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 4.0, 6.0);
        let build_cut = |profile, distance| {
            let sheet = kernel.create_body_from_profile(&OpTag::new("profile"), profile).unwrap();
            let tool = kernel.extrude(&OpTag::new("tool"), sheet.body, &Extrude { distance, direction: None }).unwrap();
            let cut = kernel.boolean(&OpTag::new("cut"), BooleanOp::Subtract, base, tool.body).unwrap();
            kernel.release(tool.body).unwrap();
            kernel.release(sheet.body).unwrap();
            (cut.body, sheet.record.seq)
        };
        let (previous, profile_seq) = build_cut(&profile, 4.0);
        let (uncontinued, fresh_seq) = build_cut(&profile, 5.0);
        assert_ne!(fresh_seq, profile_seq);
        kernel.take_diagnostics().unwrap();
        let mut edited = None;
        kernel.with_rebuild_source(previous, Some(base), &mut || edited = Some(build_cut(&profile, 5.0)));
        let (edited, edited_seq) = edited.unwrap();
        assert_eq!(edited_seq, profile_seq);

        let evidence = kernel.take_diagnostics().unwrap();
        let profile_step = format!("op_{profile_seq}");
        let cut = evidence["calls"].as_array().unwrap().iter().find(|c| c["operation"] == "boolean").unwrap();
        let operations = cut["operation_log"]["operations"].as_array().unwrap();
        assert!(!operations.iter().any(|op| op["id"] == profile_step), "replayed the unchanged profile: {evidence}");

        let recipe = kernel.body_recipe(edited).unwrap();
        let (Outcome::Exact(fresh), ..) =
            build(&recipe, OpKind::Subtract, &[], None, None, &Default::default(), None, &Default::default(), &[])
                .unwrap()
        else {
            panic!("an exact cut")
        };
        let options = StepOptions::default();
        assert_eq!(kernel.export_step(edited, &options).unwrap(), step::export(&fresh, &options, None).unwrap());
        assert_eq!(kernel.export_step(edited, &options).unwrap(), kernel.export_step(uncontinued, &options).unwrap());

        let changed = Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 3.0, 6.0);
        let mut edited = None;
        kernel.with_rebuild_source(previous, Some(base), &mut || edited = Some(build_cut(&changed, 5.0)));
        let (edited, edited_seq) = edited.unwrap();
        assert_ne!(edited_seq, profile_seq);
        assert_ne!(kernel.mass_properties(edited).unwrap().volume, kernel.mass_properties(uncontinued).unwrap().volume);
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
            let (Outcome::Exact(fresh), ..) =
                build(&recipe, OpKind::ExtrudeTo, &[], None, None, &Default::default(), None, &Default::default(), &[])
                    .unwrap()
            else {
                panic!("an exact sweep")
            };
            assert_eq!(kernel.solid(swept.body).unwrap().app.names, fresh.names);
            assert_eq!(
                kernel.export_step(swept.body, &StepOptions::default()).unwrap(),
                step::export(&fresh, &StepOptions::default(), None).unwrap()
            );
            assert!((kernel.mass_properties(swept.body).unwrap().volume - 45.0 * std::f64::consts::PI).abs() < 1e-6);
            let history = kernel.history(swept.body).unwrap();
            assert_eq!(history.iter().map(|record| record.tag.as_str()).collect::<Vec<_>>(), ["post", "sweep"]);
            assert!(swept.record.faces.deleted.is_empty());
            assert_eq!(kernel.export_step(target, &StepOptions::default()).unwrap(), original);
        }
    }
}

#[cfg(test)]
mod format_doc {
    /// docs/needs-catallax.md says which recipe format the adapter writes; it must name the
    /// format the adapter really writes.
    #[test]
    fn the_doc_names_the_recipe_format_the_adapter_writes() {
        let doc = include_str!("../../../docs/needs-catallax.md");
        let written = catallax_recipe::FORMAT_V6.rsplit('/').next().expect("a numbered format");
        let claim = doc.split("The adapter emits recipe format ").nth(1).expect("the doc names the adapter's format");
        assert!(
            claim.starts_with(&format!("{written} ")),
            "the doc says format {}, the adapter writes {written}",
            claim.split_whitespace().next().unwrap_or_default()
        );
    }
}
