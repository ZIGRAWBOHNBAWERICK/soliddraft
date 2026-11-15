//! Kernel calls answered in the kernel worker and replayed on the page.
//!
//! The worker's kernel is wrapped in a [`RecordingKernel`], which notes each call's
//! method, arguments and answer. The page's parts live on a [`ReplayKernel`] that holds
//! those answers. When the page's model rebuilds a definition the worker already built,
//! it makes the same calls with the same body ids and gets the worker's answers back,
//! without evaluating any geometry on the interface thread. A call the worker never
//! answered is a miss: the replay kernel refuses it as unavailable for now, and the page
//! can ask the worker to [`dispatch`] it.
//!
//! A call is keyed by its method and the SHA-256 of its arguments as JSON, so equal
//! requests meet whatever order they arrive in.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use soliddraft_kernel_api::scan::{ScanFit, ScanReference, ScanSource};
use soliddraft_kernel_api::{drawing, *};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// What the replay kernel answers for a call the worker has not answered yet.
pub const WAITING: &str = "The kernel worker has not answered this yet";

/// One answered call: its key and its answer as JSON.
pub type Answered = (String, Value);

/// A call the page could not replay, to be sent to the worker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Call {
    pub method: String,
    pub args: Value,
}

pub fn key(method: &str, args: &Value) -> String {
    let digest = Sha256::digest(args.to_string().as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{method}:{hex}")
}

/// An import request with owned fields, for the arguments of the two import calls.
#[derive(Serialize, Deserialize)]
struct ImportRequest {
    source_solid: Option<String>,
    name: String,
    #[serde(with = "hex_bytes")]
    source: Vec<u8>,
    sha256: String,
    options: ImportOptions,
    decisions: Vec<Decision>,
    record: Option<RepairRecord>,
}

impl ImportRequest {
    fn of(request: &ImportStepRequest<'_>) -> Self {
        Self {
            source_solid: request.source_solid.map(str::to_owned),
            name: request.name.to_owned(),
            source: request.source.to_vec(),
            sha256: request.sha256.to_owned(),
            options: request.options.clone(),
            decisions: request.decisions.to_vec(),
            record: request.record.cloned(),
        }
    }

    fn request(&self) -> ImportStepRequest<'_> {
        ImportStepRequest {
            source_solid: self.source_solid.as_deref(),
            name: &self.name,
            source: &self.source,
            sha256: &self.sha256,
            options: &self.options,
            decisions: &self.decisions,
            record: self.record.as_ref(),
        }
    }
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(text.get(i..i + 2).unwrap_or("zz"), 16))
            .collect::<Result<_, _>>()
            .map_err(serde::de::Error::custom)
    }
}

/// Bytes as hex in call arguments, so a source file does not become a JSON number array.
#[derive(Serialize, Deserialize)]
struct Bytes(#[serde(with = "hex_bytes")] Vec<u8>);

// Parameter passing in the call list: `r` by reference, `v` by value, `s` as a slice.
macro_rules! param {
    (r $t:ty) => { &$t };
    (v $t:ty) => { $t };
    (s $t:ty) => { &[$t] };
}
macro_rules! owned {
    (r $t:ty) => { $t };
    (v $t:ty) => { $t };
    (s $t:ty) => { Vec<$t> };
}
macro_rules! pass {
    (r $x:ident) => { &$x };
    (v $x:ident) => { $x };
    (s $x:ident) => { &$x };
}

/// Every kernel call whose arguments and answer are plain data. `$then` receives them.
macro_rules! with_calls {
    ($then:ident) => {
        $then! {
            fn scan_reference(source: r ScanSource) -> ScanReference;
            fn fit_scan(source: r ScanSource) -> ScanFit;
            fn build_scan_fit(tag: r OpTag, source: r ScanSource, fit: r ScanFit) -> OpResult;
            fn create_body_from_profile(tag: r OpTag, profile: r Profile) -> OpResult;
            fn extrude(tag: r OpTag, profile: v BodyId, params: r Extrude) -> OpResult;
            fn extrude_on_face(tag: r OpTag, profile: r Profile, support: v FaceSupport, distance: v f64) -> OpResult;
            fn revolve(tag: r OpTag, profile: v BodyId, params: r Revolve) -> OpResult;
            fn create_profile_from_face(tag: r OpTag, body: v BodyId, face: v FaceId) -> OpResult;
            fn loft(tag: r OpTag, profiles: s BodyId, shape: v LoftShape) -> OpResult;
            fn sweep(tag: r OpTag, profile: v BodyId, params: r Sweep) -> OpResult;
            fn boolean(tag: r OpTag, op: v BooleanOp, target: v BodyId, tool: v BodyId) -> OpResult;
            fn fillet(tag: r OpTag, body: v BodyId, edges: s EdgeId, radius: v f64) -> OpResult;
            fn chamfer(tag: r OpTag, body: v BodyId, edges: s EdgeId, distance: v f64) -> OpResult;
            fn chamfer_distances(tag: r OpTag, body: v BodyId, edges: s EdgeId, distance: v f64, other_distance: v Option<f64>) -> OpResult;
            fn shell(tag: r OpTag, body: v BodyId, open_faces: s FaceId, thickness: v f64) -> OpResult;
            fn shell_all_faces(body: v BodyId, thickness: v f64) -> ();
            fn offset(tag: r OpTag, body: v BodyId, distance: v f64) -> OpResult;
            fn thicken(tag: r OpTag, body: v BodyId, params: r Thicken) -> OpResult;
            fn pattern_feature(tag: r OpTag, body: v BodyId, source: v BodyId, before: v Option<BodyId>, layout: r PatternLayout) -> OpResult;
            fn extrude_to(tag: r OpTag, profile: v BodyId, target: v BodyId, direction: v Option<Vec3>, limit: r ExtrudeLimit) -> OpResult;
            fn hole(tag: r OpTag, body: v BodyId, params: r Hole) -> OpResult;
            fn pattern(tag: r OpTag, body: v BodyId, layout: r PatternLayout) -> OpResult;
            fn mirror(tag: r OpTag, body: v BodyId, params: r Mirror) -> OpResult;
            fn draft(tag: r OpTag, body: v BodyId, faces: s FaceId, params: r Draft) -> OpResult;
            fn lattice(tag: r OpTag, body: v BodyId, params: r Lattice) -> OpResult;
            fn gyroid(tag: r OpTag, body: v BodyId, params: r Gyroid) -> OpResult;
            fn field_mass(body: v BodyId, accuracy: r FieldMassAccuracy) -> FieldMass;
            fn independent_check(body: v BodyId) -> IndependentCheck;
            fn print_mesh(body: v BodyId, params: r TessellationParams) -> Option<PrintMesh>;
            fn tessellate(body: v BodyId, params: r TessellationParams) -> Mesh;
            fn tessellate_levels(body: v BodyId, levels: s TessellationParams) -> Vec<Mesh>;
            fn tessellate_edges(body: v BodyId, params: r TessellationParams) -> Vec<EdgePolyline>;
            fn rendering(body: v BodyId, params: r TessellationParams) -> Mesh;
            fn rendering_edges(body: v BodyId, params: r TessellationParams) -> Vec<EdgePolyline>;
            fn display_bound(body: v BodyId) -> Option<DisplayBound>;
            fn preview(body: v BodyId, params: r TessellationParams) -> Mesh;
            fn preview_levels(body: v BodyId, levels: s TessellationParams) -> Vec<Mesh>;
            fn preview_edges(body: v BodyId, params: r TessellationParams) -> Vec<EdgePolyline>;
            fn mass_properties(body: v BodyId) -> MassProperties;
            fn validate(body: v BodyId) -> Validation;
            fn topology(body: v BodyId) -> Topology;
            fn face_carriers(body: v BodyId) -> Vec<FaceCarrier>;
            fn measured_topology(body: v BodyId) -> Topology;
            fn edge_length(body: v BodyId, edge: v EdgeId) -> Measurement;
            fn face_area(body: v BodyId, face: v FaceId) -> Measurement;
            fn edge_midpoint(body: v BodyId, edge: v EdgeId) -> PointProof;
            fn measure_pair(body: v BodyId, a: v PairEndpoint, b: v PairEndpoint, quantity: v PairQuantity) -> Measurement;
            fn face_support_plane(body: v BodyId, face: v FaceId) -> ExactAxisPlane;
            fn entity_support(body: v BodyId, entity: v SupportEntity) -> ExactSupport;
            fn entity_support_enclosed(body: v BodyId, entity: v SupportEntity) -> EnclosedSupport;
            fn history(body: v BodyId) -> Vec<HistoryRecord>;
            fn retained_bytes(body: v BodyId) -> u64;
            fn view_curves(body: v BodyId, spec: r drawing::ViewSpec) -> drawing::ViewCurves;
            fn silhouette_curves(body: v BodyId, direction: v Vec3) -> Vec<drawing::ViewCurve>;
            fn section_curves(body: v BodyId, plane: r Plane) -> Vec<drawing::ViewCurve>;
            fn section_view(body: v BodyId, spec: r drawing::SectionSpec) -> drawing::SectionView;
            fn edge_curve(body: v BodyId, edge: v EdgeId) -> drawing::Curve3;
            fn face_loops(body: v BodyId, face: v FaceId) -> Vec<Vec<drawing::Curve3>>;
            fn lines_and_arcs(curve: r drawing::Curve3, tolerance: v f64) -> Vec<drawing::Curve3>;
            fn export_iges(body: v BodyId, options: r IgesOptions) -> IgesExport;
            fn export_step(body: v BodyId, options: r StepOptions) -> StepExport;
        }
    };
}

macro_rules! recorded {
    ($( fn $name:ident($($arg:ident : $mode:ident $ty:ty),*) -> $ret:ty; )*) => {
        $(
            fn $name(&self $(, $arg: param!($mode $ty))*) -> KernelResult<$ret> {
                let args = serde_json::json!([$($arg),*]);
                let result = self.inner.$name($($arg),*);
                self.record(stringify!($name), &args, &result);
                result
            }
        )*
    };
}

macro_rules! replayed {
    ($( fn $name:ident($($arg:ident : $mode:ident $ty:ty),*) -> $ret:ty; )*) => {
        $(
            fn $name(&self $(, $arg: param!($mode $ty))*) -> KernelResult<$ret> {
                self.answer(stringify!($name), serde_json::json!([$($arg),*]))
            }
        )*
    };
}

macro_rules! dispatched {
    ($( fn $name:ident($($arg:ident : $mode:ident $ty:ty),*) -> $ret:ty; )*) => {
        /// Runs a call the page could not replay, so the recording kernel notes its answer.
        pub fn dispatch(kernel: &dyn Kernel, call: &Call) -> Result<(), String> {
            let unreadable = |error: serde_json::Error| format!("{}: unreadable arguments: {error}", call.method);
            match call.method.as_str() {
                $(
                    stringify!($name) => {
                        #[allow(unused_parens)]
                        let ($($arg,)*): ($(owned!($mode $ty),)*) =
                            serde_json::from_value(call.args.clone()).map_err(unreadable)?;
                        let _ = kernel.$name($(pass!($mode $arg)),*);
                        Ok(())
                    }
                )*
                "import_step" | "import_source" => {
                    let (format, tag, request): (ImportFormat, OpTag, ImportRequest) =
                        serde_json::from_value(call.args.clone()).map_err(unreadable)?;
                    let _ = kernel.import_source(format, &tag, &request.request());
                    Ok(())
                }
                "propose_import_source" => {
                    let (format, source, options): (ImportFormat, Bytes, ImportOptions) =
                        serde_json::from_value(call.args.clone()).map_err(unreadable)?;
                    let _ = kernel.propose_import_source(format, &source.0, &options);
                    Ok(())
                }
                "source_solids" => {
                    let (format, source): (ImportFormat, Bytes) =
                        serde_json::from_value(call.args.clone()).map_err(unreadable)?;
                    let _ = kernel.source_solids(format, &source.0);
                    Ok(())
                }
                "body_notice" => {
                    let (body,): (BodyId,) = serde_json::from_value(call.args.clone()).map_err(unreadable)?;
                    let _ = kernel.body_notice(body);
                    Ok(())
                }
                "construction_edit" => {
                    let (body, edits, params): (BodyId, Vec<ScalarEdit>, TessellationParams) =
                        serde_json::from_value(call.args.clone()).map_err(unreadable)?;
                    let _ = kernel.construction_edit(body, &edits, &params);
                    Ok(())
                }
                other => Err(format!("The kernel worker cannot run {other}")),
            }
        }
    };
}

with_calls!(dispatched);

/// Work for a replayed rebuild. The worker already spent the real allowance.
struct Replayed;

impl ProofWork for Replayed {
    fn run(self: Box<Self>, rebuild: &mut dyn FnMut() -> KernelResult<()>) -> KernelResult<()> {
        rebuild()
    }
}

/// Keys of the first call inside each open `measure_work`, innermost last. A work
/// count is filed under the first call its closure made, which both sides share.
#[derive(Default)]
struct Measuring(Mutex<Vec<Option<String>>>);

impl Measuring {
    fn open(&self) {
        self.0.lock().expect("measure stack").push(None);
    }

    fn close(&self) -> Option<String> {
        self.0.lock().expect("measure stack").pop().flatten()
    }

    fn saw(&self, key: &str) {
        for first in self.0.lock().expect("measure stack").iter_mut().filter(|first| first.is_none()) {
            *first = Some(key.to_owned());
        }
    }
}

/// The worker's kernel, noting every plain-data call and its answer.
pub struct RecordingKernel {
    inner: Arc<dyn Kernel>,
    log: Mutex<Vec<Answered>>,
    measuring: Measuring,
}

impl RecordingKernel {
    pub fn new(inner: Arc<dyn Kernel>) -> Self {
        Self { inner, log: Mutex::new(Vec::new()), measuring: Measuring::default() }
    }

    /// The calls answered since the last take.
    pub fn take(&self) -> Vec<Answered> {
        std::mem::take(&mut *self.log.lock().expect("recording log"))
    }

    fn record<T: Serialize>(&self, method: &str, args: &Value, result: &T) {
        let key = key(method, args);
        self.measuring.saw(&key);
        if let Ok(answer) = serde_json::to_value(result) {
            self.log.lock().expect("recording log").push((key, answer));
        }
    }
}

impl Kernel for RecordingKernel {
    with_calls!(recorded);

    fn backend(&self) -> BackendInfo {
        self.inner.backend()
    }

    fn body_notice(&self, body: BodyId) -> Option<&'static str> {
        let notice = self.inner.body_notice(body);
        self.record("body_notice", &serde_json::json!([body]), &notice);
        notice
    }

    fn cached_mass_properties(&self, body: BodyId) -> Option<MassProperties> {
        let properties = self.inner.cached_mass_properties(body);
        self.record("cached_mass_properties", &serde_json::json!([body]), &properties);
        properties
    }

    fn measure_work(&self, run: &mut dyn FnMut()) -> KernelResult<Option<u64>> {
        self.measuring.open();
        let work = self.inner.measure_work(run);
        if let Some(first) = self.measuring.close() {
            self.record("measure_work", &serde_json::json!([first]), &work);
        }
        work
    }

    fn reserve_proof_work(&self) -> KernelResult<Box<dyn ProofWork>> {
        self.inner.reserve_proof_work()
    }

    fn construction_edit(
        &self,
        body: BodyId,
        edits: &[ScalarEdit],
        params: &TessellationParams,
    ) -> KernelResult<ConstructionEdit> {
        let edit = self.inner.construction_edit(body, edits, params);
        let display = edit.as_ref().map(|edit| &edit.display);
        self.record("construction_edit", &serde_json::json!([body, edits, params]), &display);
        edit
    }

    fn propose_import(&self, source: &[u8], options: &ImportOptions) -> KernelResult<ProposalSet> {
        self.propose_import_source(ImportFormat::Step, source, options)
    }

    fn propose_import_source(
        &self,
        format: ImportFormat,
        source: &[u8],
        options: &ImportOptions,
    ) -> KernelResult<ProposalSet> {
        let result = self.inner.propose_import_source(format, source, options);
        let args = serde_json::json!([format, Bytes(source.to_vec()), options]);
        self.record("propose_import_source", &args, &result);
        result
    }

    fn source_solids(&self, format: ImportFormat, source: &[u8]) -> KernelResult<Vec<String>> {
        let result = self.inner.source_solids(format, source);
        self.record("source_solids", &serde_json::json!([format, Bytes(source.to_vec())]), &result);
        result
    }

    fn import_step(&self, tag: &OpTag, request: &ImportStepRequest<'_>) -> KernelResult<ImportStepResult> {
        self.import_source(ImportFormat::Step, tag, request)
    }

    fn import_source(
        &self,
        format: ImportFormat,
        tag: &OpTag,
        request: &ImportStepRequest<'_>,
    ) -> KernelResult<ImportStepResult> {
        let result = self.inner.import_source(format, tag, request);
        let args = serde_json::json!([format, tag, ImportRequest::of(request)]);
        self.record("import_source", &args, &result);
        result
    }

    // Queries the page does not replay yet: they run here and are not noted.
    fn body_relation(&self, a: PlacedBody, b: PlacedBody, clearance: Option<f64>) -> KernelResult<BodyRelation> {
        self.inner.body_relation(a, b, clearance)
    }

    fn export_field_stl(&self, body: BodyId) -> KernelResult<FieldExport> {
        self.inner.export_field_stl(body)
    }

    fn export_field_stand_in(&self, body: BodyId, accuracy: &FieldMassAccuracy) -> KernelResult<StandInExport> {
        self.inner.export_field_stand_in(body, accuracy)
    }

    fn export_field_stl_at(&self, body: BodyId, accuracy_mm: f64) -> KernelResult<FieldExport> {
        self.inner.export_field_stl_at(body, accuracy_mm)
    }

    fn release(&self, body: BodyId) -> KernelResult<()> {
        self.inner.release(body)
    }
}

/// The page's kernel in the browser: answers what the worker answered.
pub struct ReplayKernel {
    backend: BackendInfo,
    answers: Mutex<HashMap<String, Value>>,
    missed: Mutex<Vec<Call>>,
    misses: std::sync::atomic::AtomicU64,
    measuring: Measuring,
    notices: Mutex<HashSet<&'static str>>,
}

impl ReplayKernel {
    pub fn new(backend: BackendInfo) -> Self {
        Self {
            backend,
            answers: Mutex::new(HashMap::new()),
            missed: Mutex::new(Vec::new()),
            misses: std::sync::atomic::AtomicU64::new(0),
            measuring: Measuring::default(),
            notices: Mutex::new(HashSet::new()),
        }
    }

    /// Takes in answers the worker recorded.
    pub fn learn(&self, answered: Vec<Answered>) {
        self.answers.lock().expect("replay answers").extend(answered);
    }

    /// Forgets every answer: the worker that gave them has gone, and a new one numbers
    /// its bodies from the start again.
    pub fn forget(&self) {
        self.answers.lock().expect("replay answers").clear();
        self.missed.lock().expect("replay misses").clear();
    }

    /// Calls missed since the last take, each once.
    pub fn take_missed(&self) -> Vec<Call> {
        let mut seen = HashSet::new();
        let mut missed = std::mem::take(&mut *self.missed.lock().expect("replay misses"));
        missed.retain(|call| seen.insert(key(&call.method, &call.args)));
        missed
    }

    /// How many calls have missed so far. A replayed rebuild compares it before and after.
    pub fn misses(&self) -> u64 {
        self.misses.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn answers(&self) -> usize {
        self.answers.lock().expect("replay answers").len()
    }

    fn lookup(&self, method: &str, args: Value) -> Option<Value> {
        let key = key(method, &args);
        self.measuring.saw(&key);
        let answer = self.answers.lock().expect("replay answers").get(&key).cloned();
        if answer.is_none() {
            self.misses.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.missed.lock().expect("replay misses").push(Call { method: method.to_owned(), args });
        }
        answer
    }

    fn answer<T: DeserializeOwned>(&self, method: &str, args: Value) -> KernelResult<T> {
        match self.lookup(method, args) {
            Some(answer) => serde_json::from_value::<KernelResult<T>>(answer).unwrap_or_else(|error| {
                Err(KernelError::Unavailable(format!("{method}: the kernel worker's answer is unreadable: {error}")))
            }),
            None => Err(KernelError::Unavailable(WAITING.into())),
        }
    }
}

impl Kernel for ReplayKernel {
    with_calls!(replayed);

    fn backend(&self) -> BackendInfo {
        self.backend.clone()
    }

    fn body_notice(&self, body: BodyId) -> Option<&'static str> {
        let notice: String = self
            .lookup("body_notice", serde_json::json!([body]))
            .and_then(|answer| serde_json::from_value::<Option<String>>(answer).ok().flatten())?;
        let mut notices = self.notices.lock().expect("replay notices");
        if let Some(known) = notices.get(notice.as_str()) {
            return Some(known);
        }
        // Notices are a handful of fixed sentences, so each is kept once for good.
        let kept: &'static str = Box::leak(notice.into_boxed_str());
        notices.insert(kept);
        Some(kept)
    }

    fn cached_mass_properties(&self, body: BodyId) -> Option<MassProperties> {
        let args = serde_json::json!([body]);
        let answer = self.answers.lock().expect("replay answers").get(&key("cached_mass_properties", &args)).cloned();
        answer.and_then(|answer| serde_json::from_value(answer).ok()).flatten()
    }

    fn measure_work(&self, run: &mut dyn FnMut()) -> KernelResult<Option<u64>> {
        self.measuring.open();
        run();
        let Some(first) = self.measuring.close() else {
            return Ok(None);
        };
        let answer = self.answers.lock().expect("replay answers").get(&key("measure_work", &serde_json::json!([first]))).cloned();
        Ok(answer.and_then(|answer| serde_json::from_value::<KernelResult<Option<u64>>>(answer).ok()?.ok()).flatten())
    }

    fn reserve_proof_work(&self) -> KernelResult<Box<dyn ProofWork>> {
        Ok(Box::new(Replayed))
    }

    fn construction_edit(
        &self,
        body: BodyId,
        edits: &[ScalarEdit],
        params: &TessellationParams,
    ) -> KernelResult<ConstructionEdit> {
        let display = self.answer::<KernelResult<ConstructionMesh>>(
            "construction_edit",
            serde_json::json!([body, edits, params]),
        )?;
        Ok(ConstructionEdit { display, proof: Box::new(Replayed) })
    }

    fn propose_import(&self, source: &[u8], options: &ImportOptions) -> KernelResult<ProposalSet> {
        self.propose_import_source(ImportFormat::Step, source, options)
    }

    fn propose_import_source(
        &self,
        format: ImportFormat,
        source: &[u8],
        options: &ImportOptions,
    ) -> KernelResult<ProposalSet> {
        self.answer("propose_import_source", serde_json::json!([format, Bytes(source.to_vec()), options]))
    }

    fn source_solids(&self, format: ImportFormat, source: &[u8]) -> KernelResult<Vec<String>> {
        self.answer("source_solids", serde_json::json!([format, Bytes(source.to_vec())]))
    }

    fn import_step(&self, tag: &OpTag, request: &ImportStepRequest<'_>) -> KernelResult<ImportStepResult> {
        self.import_source(ImportFormat::Step, tag, request)
    }

    fn import_source(
        &self,
        format: ImportFormat,
        tag: &OpTag,
        request: &ImportStepRequest<'_>,
    ) -> KernelResult<ImportStepResult> {
        self.answer("import_source", serde_json::json!([format, tag, ImportRequest::of(request)]))
    }

    fn body_relation(&self, _: PlacedBody, _: PlacedBody, _: Option<f64>) -> KernelResult<BodyRelation> {
        Err(KernelError::Unavailable("Body relations run in the desktop app".into()))
    }

    fn export_field_stl(&self, _: BodyId) -> KernelResult<FieldExport> {
        Err(KernelError::Unavailable("Field export in the browser is coming soon".into()))
    }

    fn export_field_stand_in(&self, _: BodyId, _: &FieldMassAccuracy) -> KernelResult<StandInExport> {
        Err(KernelError::Unavailable("Field export in the browser is coming soon".into()))
    }

    fn export_field_stl_at(&self, _: BodyId, _: f64) -> KernelResult<FieldExport> {
        Err(KernelError::Unavailable("Field export in the browser is coming soon".into()))
    }

    /// The worker keeps or releases its own bodies; the page only stops asking.
    fn release(&self, _: BodyId) -> KernelResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recorded_demo() -> (Arc<RecordingKernel>, crate::document::Document) {
        let recording = Arc::new(RecordingKernel::new(crate::kernel()));
        let mut doc = crate::document::Document::new("Part1", recording.clone());
        crate::populate_demo(&mut doc);
        (recording, doc)
    }

    #[test]
    fn a_replayed_rebuild_makes_the_same_part_without_evaluating_it() {
        let (recording, doc) = recorded_demo();
        let (_, part) = doc.model().snapshot().encode().unwrap();
        let replay = Arc::new(ReplayKernel::new(recording.backend()));
        replay.learn(recording.take());
        let replayed = soliddraft_model::Document::from_bytes(&part, replay.clone()).unwrap();
        assert_eq!(replay.misses(), 0, "{:?}", replay.take_missed());
        assert_eq!(replayed.body(), doc.model().body(), "the page sees the worker's body ids");
        for feature in replayed.features() {
            assert_eq!(replayed.status(feature.id), doc.model().status(feature.id));
        }
        let body = replayed.body().unwrap();
        assert_eq!(replay.topology(body).unwrap(), doc.kernel().topology(body).unwrap());
    }

    #[test]
    fn a_miss_is_refused_and_the_worker_answers_it_on_dispatch() {
        let (recording, doc) = recorded_demo();
        recording.take();
        let replay = ReplayKernel::new(recording.backend());
        let body = doc.model().body().unwrap();
        let error = replay.mass_properties(body).unwrap_err();
        assert_eq!(error, KernelError::Unavailable(WAITING.into()));
        let missed = replay.take_missed();
        assert_eq!(missed.len(), 1);
        dispatch(recording.as_ref(), &missed[0]).unwrap();
        replay.learn(recording.take());
        assert_eq!(replay.mass_properties(body).unwrap(), doc.kernel().mass_properties(body).unwrap());
        assert!(replay.take_missed().is_empty());
    }

    #[test]
    fn work_counts_follow_the_first_call_inside_them() {
        let (recording, doc) = recorded_demo();
        let (_, part) = doc.model().snapshot().encode().unwrap();
        recording.take();
        let rebuilt = soliddraft_model::Document::from_bytes(&part, recording.clone()).unwrap();
        let replay = Arc::new(ReplayKernel::new(recording.backend()));
        replay.learn(recording.take());
        let replayed = soliddraft_model::Document::from_bytes(&part, replay).unwrap();
        for feature in replayed.features() {
            assert_eq!(replayed.work_used(feature.id), rebuilt.work_used(feature.id), "{}", feature.name);
        }
    }

    #[test]
    fn import_arguments_round_trip() {
        let options = ImportOptions::default();
        let request = ImportStepRequest {
            source_solid: Some("solid-1"),
            name: "part.step",
            source: b"ISO-10303-21;",
            sha256: "abc",
            options: &options,
            decisions: &[],
            record: None,
        };
        let args = serde_json::json!([ImportFormat::Step, OpTag::new("f1"), ImportRequest::of(&request)]);
        let (_, _, owned): (ImportFormat, OpTag, ImportRequest) = serde_json::from_value(args).unwrap();
        assert_eq!(owned.request().source, request.source);
        assert_eq!(owned.request().source_solid, Some("solid-1"));
    }
}
