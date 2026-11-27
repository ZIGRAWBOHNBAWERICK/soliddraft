//! Kernel work the browser runs off its interface thread.
//!
//! The page has one thread, and a long Catallax query on it freezes the camera and every
//! control until it ends. The browser build therefore starts a dedicated Web Worker
//! (`kernel_worker.js`) on the same compiled WebAssembly module. The worker has its own
//! memory and its own Catallax kernel: it never sees the page's bodies. Each job carries
//! the part's saved bytes, the worker rebuilds them, reusing what it built for the
//! previous job, and answers with plain data.
//!
//! The interface stays the owner of its part. A job's answer is accepted only for the
//! document and revision that asked for it; the caller keeps that check. A newer job of
//! the same kind replaces one that has not started. A worker that traps answers the job
//! it was running with the reason, and the next job starts a new worker, so a failure
//! never takes the part on screen with it.
//!
//! [`Host`] is the worker's half. It is plain Rust, so native tests run it directly.

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::Kernel;
use soliddraft_model as model;
use std::sync::Arc;

/// One request to the kernel worker. The part travels beside it as saved bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "job", rename_all = "snake_case")]
pub enum Job {
    /// Volume, area, centroid and inertia of the body after `feature`.
    MassProperties { feature: model::FeatureId },
    /// The part after an optional dimension edit, rebuilt, proved and meshed as the
    /// desktop's proof worker does it.
    Build {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edit: Option<crate::drag::ScalarDraft>,
        #[serde(default)]
        level: soliddraft_tessellation::Level,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        curved_surfaces: Option<crate::graphics::CurvedSurfaces>,
        /// The page shows GPU previews: the worker hands the rebuilt part over as soon as
        /// it is built, so the page can mesh its preview, and then makes the checked
        /// meshes the page shows in its place.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        checked: bool,
    },
    /// Warm the viewport meshes for a display setting while the open document stays usable.
    /// For a page that shows GPU previews, these are the checked meshes.
    Remesh {
        curved_surfaces: Option<crate::graphics::CurvedSurfaces>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        checked: bool,
    },
}

impl Job {
    /// Jobs of one slot replace each other while they wait.
    pub fn slot(&self) -> &'static str {
        match self {
            Job::MassProperties { .. } => "mass_properties",
            Job::Build { .. } => "build",
            Job::Remesh { .. } => "remesh",
        }
    }
}

/// The worker's kernel and the part it built last.
pub struct Host {
    kernel: Arc<dyn Kernel>,
    part: Option<(Vec<u8>, model::Document)>,
}

impl Host {
    /// A host keeps every build its kernel makes, for [`Host::take_hand_over`]. A kernel
    /// without hand-over still answers jobs; its answers just carry no builds.
    pub fn new(kernel: Arc<dyn Kernel>) -> Self {
        let _ = kernel.keep_builds();
        Self { kernel, part: None }
    }

    /// The builds made since the last call, as bytes the page's kernel can be offered,
    /// so it rebuilds the same part without evaluating it again. Keeps keeping builds.
    pub fn take_hand_over(&mut self) -> Result<Vec<u8>, String> {
        let bytes = self.kernel.hand_over().map_err(|error| error.to_string())?;
        self.kernel.keep_builds().map_err(|error| error.to_string())?;
        Ok(bytes)
    }

    /// Runs `job` against the part saved as `part` and answers with plain data.
    pub fn run(&mut self, part: &[u8], job: &Job) -> Result<serde_json::Value, String> {
        self.run_with(part, job, &|_| {})
    }

    /// [`Host::run`], handing a checked build's part over to `early` as soon as it is
    /// rebuilt, before its checked meshes are made.
    pub fn run_with(&mut self, part: &[u8], job: &Job, early: &dyn Fn(Vec<u8>)) -> Result<serde_json::Value, String> {
        let kernel = self.kernel.clone();
        let doc = self.open(part)?;
        match job {
            Job::MassProperties { feature } => {
                built(doc, *feature)?;
                let body = doc.body_after(*feature).ok_or("The feature has no body to measure")?;
                let (result, work) =
                    soliddraft_kernel_catallax::with_request_allowance(crate::measure::MASS_WORK_LIMIT, || {
                        doc.kernel().mass_properties(body)
                    });
                if work.exhausted {
                    return Err(format!(
                        "mass_properties: numbers.operation-budget: {} work units exhausted",
                        work.limit
                    ));
                }
                let properties = result.map_err(|error| error.to_string())?;
                serde_json::to_value(properties).map_err(|error| error.to_string())
            }
            Job::Build { edit, level, curved_surfaces, checked } => {
                // The rebuilt part goes to the page at once, with the topology of each of
                // its bodies, so the page meshes its GPU preview while this worker makes
                // the checked meshes.
                let hand_early = |model: &model::Document| {
                    if !*checked {
                        return;
                    }
                    for (_, body) in model.live_bodies() {
                        let _ = kernel.topology(body);
                    }
                    match kernel.hand_over() {
                        Ok(bytes) => {
                            let _ = kernel.keep_builds();
                            if !bytes.is_empty() {
                                early(bytes);
                            }
                        }
                        Err(error) => log::warn!("the kernel worker could not hand its rebuilt part over: {error}"),
                    }
                };
                let build = || -> Result<serde_json::Value, String> {
                    let source = std::sync::Arc::new(doc.fork());
                    let proved = crate::drag::prove_now(source, *edit, *level, *curved_surfaces, &hand_early)?;
                    let model = &proved.model;
                    let statuses: Vec<_> = model
                        .features()
                        .iter()
                        .map(|feature| serde_json::json!({ "feature": feature.id, "status": built(model, feature.id).err() }))
                        .collect();
                    let triangles = proved.display.as_ref().map(|display| display.mesh.indices.len() / 3);
                    sample_display_levels(model, *curved_surfaces);
                    Ok(serde_json::json!({
                        "failure": proved.failure,
                        "display_error": proved.display_error,
                        "statuses": statuses,
                        "triangles": triangles,
                    }))
                };
                if *checked { crate::viewport::gpu_preview::checked_route(build) } else { build() }
            }
            Job::Remesh { curved_surfaces, checked } => {
                for (_, body) in doc.live_bodies() {
                    let kernel = doc.kernel();
                    let topology = kernel.topology(body).map_err(|error| error.to_string())?;
                    if curved_surfaces.is_some() && !crate::graphics::topology_supports_curved_surfaces(&topology) {
                        continue;
                    }
                    let levels =
                        crate::graphics::body_mesh_levels(*curved_surfaces, &doc.settings().viewport, &topology);
                    for params in levels.all() {
                        if *checked {
                            // A refused checked mesh leaves the page its preview.
                            let _ = kernel.checked_rendering(body, params);
                            let _ = kernel.checked_rendering_edges(body, params);
                        } else {
                            kernel.preview(body, params).map_err(|error| error.to_string())?;
                            kernel.preview_edges(body, params).map_err(|error| error.to_string())?;
                        }
                    }
                }
                Ok(serde_json::Value::Null)
            }
        }
    }

    /// The part these bytes save, rebuilt on the worker's kernel. The same bytes reuse the
    /// last part whole; other bytes reuse its feature builds wherever their inputs match.
    fn open(&mut self, part: &[u8]) -> Result<&model::Document, String> {
        if self.part.as_ref().is_none_or(|(bytes, _)| bytes.as_slice() != part) {
            let previous = self.part.take().map(|(_, doc)| doc);
            let doc = model::Document::from_bytes_with_input(part, self.kernel.clone(), previous.as_ref(), |_| {})
                .map_err(|error| format!("The kernel worker could not read the part: {error}"))?;
            self.part = Some((part.to_vec(), doc));
        }
        Ok(&self.part.as_ref().expect("the part was just opened").1)
    }
}

/// Samples the early display of the part's shown body at every display level, as the
/// page's display quality asks for them, so the hand-over carries each and the page
/// samples none. For a page that shows GPU previews, these are the checked meshes.
fn sample_display_levels(model: &model::Document, curved_surfaces: Option<crate::graphics::CurvedSurfaces>) {
    use soliddraft_tessellation::Level;
    let Some((_, body)) = model.live_bodies().last().copied() else { return };
    let kernel = model.kernel();
    let Ok(topology) = kernel.topology(body) else { return };
    if curved_surfaces.is_some() && !crate::graphics::topology_supports_curved_surfaces(&topology) {
        return;
    }
    let levels = crate::graphics::body_mesh_levels(curved_surfaces, &model.settings().viewport, &topology);
    let checked = crate::viewport::gpu_preview::route_active();
    for level in [Level::Coarse, Level::Standard, Level::Fine] {
        let params = *levels.params(level);
        if checked {
            let _ = kernel.checked_rendering(body, &params);
            let _ = kernel.checked_rendering_edges(body, &params);
        } else {
            let _ = kernel.preview(body, &params);
            let _ = kernel.preview_edges(body, &params);
        }
    }
}

/// Refuses a feature the worker's rebuild did not build, rather than measuring the body
/// it passed through.
fn built(doc: &model::Document, feature: model::FeatureId) -> Result<(), String> {
    match doc.status(feature) {
        Some(status) if status.is_ok() => Ok(()),
        Some(status) => Err(format!("The kernel worker could not build the feature: {status:?}")),
        None => Err("The kernel worker's part has no such feature".into()),
    }
}

#[cfg(target_arch = "wasm32")]
pub use browser::{abandon_unwanted, submit, submit_with_early, trace};

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::{Host, Job};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::Arc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    /// What a job's answer is handed to on the interface thread.
    type Answered = Box<dyn FnOnce(Result<serde_json::Value, String>)>;

    thread_local! {
        static CLIENT: RefCell<Option<Client>> = const { RefCell::new(None) };
        static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
        /// Each worker the page starts, numbered: a restarted worker numbers its entities
        /// from the start again, so the page's kernel keeps each worker's ids apart.
        static SESSIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
        static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    }

    /// Whether the page still wants a job's builds: a cancelled or replaced job's answer
    /// still arrives, and its builds must not reach the kernel a later job reads.
    type Live = Box<dyn Fn() -> bool>;

    /// What a job was sent as, so a restarted worker can be sent it again.
    struct Sent {
        slot: &'static str,
        header: String,
        part: Vec<u8>,
    }

    /// Who waits for a job: its answer's handler, the context to repaint, the kernel of
    /// the part that asked, which is offered the builds the worker made for the job,
    /// whether that job is still wanted, and the job as sent.
    type Waiting = (Answered, egui::Context, Arc<dyn soliddraft_kernel_api::Kernel>, Live, Sent);

    #[derive(Default)]
    struct Shared {
        /// Which worker the page started this is, for [`soliddraft_kernel_api::Kernel::offer_builds`].
        session: u64,
        waiting: HashMap<u64, Waiting>,
        /// What runs when a job hands its rebuilt part over before its answer.
        early: HashMap<u64, Box<dyn FnMut()>>,
        /// The last panic the worker reported, the reason for the trap that follows it.
        panic: Option<String>,
        stopped: bool,
    }

    impl Shared {
        /// Whether job `id`'s builds are still wanted, before they are copied off the message.
        fn live(&self, id: u64) -> bool {
            self.waiting.get(&id).is_some_and(|(_, _, _, live, _)| live())
        }

        /// Offers a job's rebuilt part to the page's kernel ahead of its answer, and tells
        /// whoever waits for it.
        fn early(&mut self, id: u64, hand_over: &[u8]) {
            let Some((_, ctx, kernel, _, _)) = self.waiting.get(&id) else { return };
            if let Err(error) = kernel.offer_builds(self.session, hand_over) {
                log::warn!("the page could not take the kernel worker's rebuilt part: {error}");
                return;
            }
            ctx.request_repaint();
            if let Some(early) = self.early.get_mut(&id) {
                early();
            }
        }

        fn answer(&mut self, id: u64, result: Result<serde_json::Value, String>, hand_over: &[u8]) {
            self.early.remove(&id);
            if let Some((answered, ctx, kernel, _, _)) = self.waiting.remove(&id) {
                // The builds come first, so a rebuild the answer starts takes them.
                if !hand_over.is_empty()
                    && let Err(error) = kernel.offer_builds(self.session, hand_over)
                {
                    log::warn!("the page could not take the kernel worker's builds: {error}");
                }
                answered(result);
                ctx.request_repaint();
            }
        }

        fn stop(&mut self, reason: &str) {
            self.stopped = true;
            self.early.clear();
            for (_, (answered, ctx, _, _, _)) in self.waiting.drain() {
                answered(Err(format!("The kernel worker stopped: {reason}. Try again.")));
                ctx.request_repaint();
            }
        }
    }

    struct Client {
        worker: web_sys::Worker,
        shared: Rc<RefCell<Shared>>,
        _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
        _error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
    }

    impl Drop for Client {
        fn drop(&mut self) {
            self.worker.terminate();
        }
    }

    impl Client {
        fn start() -> Result<Self, String> {
            let glue = glue_url().ok_or("The kernel worker could not find the application module")?;
            let script = wasm_bindgen::link_to!(module = "/src/kernel_worker.js");
            let options = web_sys::WorkerOptions::new();
            options.set_type(web_sys::WorkerType::Module);
            options.set_name("soliddraft-kernel");
            let worker = web_sys::Worker::new_with_options(&script, &options)
                .map_err(|error| format!("The kernel worker could not start: {error:?}"))?;
            let session = SESSIONS.with(|sessions| sessions.replace(sessions.get() + 1));
            let shared = Rc::new(RefCell::new(Shared { session, ..Shared::default() }));
            let on_message = shared.clone();
            let message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                received(&mut on_message.borrow_mut(), &event.data());
            });
            let on_error = shared.clone();
            let error = Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |event: web_sys::ErrorEvent| {
                event.prevent_default();
                on_error.borrow_mut().stop(&event.message());
            });
            worker.set_onmessage(Some(message.as_ref().unchecked_ref()));
            worker.set_onerror(Some(error.as_ref().unchecked_ref()));
            let init = js_sys::Object::new();
            set(&init, "glue", &glue.into());
            set(&init, "module", &wasm_bindgen::module());
            let message_init = js_sys::Object::new();
            set(&message_init, "init", &init);
            worker
                .post_message(&message_init)
                .map_err(|error| format!("The kernel worker could not start: {error:?}"))?;
            Ok(Self { worker, shared, _message: message, _error: error })
        }
    }

    fn received(shared: &mut Shared, data: &JsValue) {
        if let Some(id) = get(data, "id").as_f64() {
            let kind =
                ["answer", "early", "fatal", "superseded"].into_iter().find(|key| !get(data, key).is_undefined());
            trace("page received", id as u64, kind.unwrap_or("other").into());
        }
        if let Some(panic) = get(data, "panic").as_string() {
            shared.panic = Some(panic);
            return;
        }
        let Some(id) = get(data, "id").as_f64().map(|id| id as u64) else {
            if let Some(fatal) = get(data, "fatal").as_string() {
                let reason = shared.panic.take().unwrap_or(fatal);
                shared.stop(&reason);
            }
            return;
        };
        if get(data, "superseded").is_truthy() {
            // The interface already asked again; nobody waits for this answer.
            shared.waiting.remove(&id);
            shared.early.remove(&id);
        } else if let Ok(early) = get(data, "early").dyn_into::<js_sys::Uint8Array>() {
            if shared.live(id) {
                shared.early(id, &early.to_vec());
            }
        } else if let Some(answer) = get(data, "answer").as_string() {
            // A job no longer wanted has its builds dropped unread, not offered.
            let hand_over = if shared.live(id) {
                get(data, "hand_over").dyn_into::<js_sys::Uint8Array>().map(|bytes| bytes.to_vec()).unwrap_or_default()
            } else {
                trace("page dropped builds", id, String::new());
                Vec::new()
            };
            shared.answer(id, decode(&answer), &hand_over);
        } else if let Some(fatal) = get(data, "fatal").as_string() {
            let reason = shared.panic.take().unwrap_or(fatal);
            shared.answer(id, Err(format!("The kernel worker stopped: {reason}. Try again.")), &[]);
            shared.stop(&reason);
        }
    }

    fn decode(answer: &str) -> Result<serde_json::Value, String> {
        let mut answer: serde_json::Value = serde_json::from_str(answer)
            .map_err(|error| format!("The kernel worker's answer is unreadable: {error}"))?;
        match answer.get_mut("err").map(serde_json::Value::take) {
            Some(serde_json::Value::String(error)) => Err(error),
            _ => Ok(answer.get_mut("ok").map(serde_json::Value::take).unwrap_or_default()),
        }
    }

    /// Sends `job` and the part's saved bytes to the kernel worker. `answered` runs on the
    /// interface thread with the answer, and `ctx` repaints then. The builds the worker made
    /// for the job are offered to `kernel`, the part's own, just before.
    pub fn submit(
        part: Vec<u8>,
        job: &Job,
        kernel: Arc<dyn soliddraft_kernel_api::Kernel>,
        ctx: &egui::Context,
        live: impl Fn() -> bool + 'static,
        answered: impl FnOnce(Result<serde_json::Value, String>) + 'static,
    ) -> Result<u64, String> {
        submit_with_early(part, job, kernel, ctx, live, answered, || {})
    }

    /// [`submit`], with `early` run when a checked build hands its rebuilt part over,
    /// after the part has been offered to `kernel` and before the job's answer.
    pub fn submit_with_early(
        part: Vec<u8>,
        job: &Job,
        kernel: Arc<dyn soliddraft_kernel_api::Kernel>,
        ctx: &egui::Context,
        live: impl Fn() -> bool + 'static,
        answered: impl FnOnce(Result<serde_json::Value, String>) + 'static,
        early: impl FnMut() + 'static,
    ) -> Result<u64, String> {
        CLIENT.with_borrow_mut(|client| {
            if client.as_ref().is_none_or(|client| client.shared.borrow().stopped) {
                // Dropping the stopped client here, outside its own handlers, ends it.
                *client = Some(Client::start()?);
            }
            let client = client.as_ref().expect("the kernel worker started");
            let id = NEXT.with(|next| next.replace(next.get() + 1));
            let header = serde_json::to_string(job).map_err(|error| error.to_string())?;
            let sent = Sent { slot: job.slot(), header, part };
            post(&client.worker, id, &sent)?;
            let mut shared = client.shared.borrow_mut();
            shared.waiting.insert(id, (Box::new(answered), ctx.clone(), kernel, Box::new(live), sent));
            shared.early.insert(id, Box::new(early));
            Ok(id)
        })
    }

    /// Posts job `id` to `worker`.
    fn post(worker: &web_sys::Worker, id: u64, sent: &Sent) -> Result<(), String> {
        let bytes = js_sys::Uint8Array::from(sent.part.as_slice());
        let message = js_sys::Object::new();
        set(&message, "id", &(id as f64).into());
        set(&message, "slot", &sent.slot.into());
        set(&message, "header", &sent.header.as_str().into());
        set(&message, "part", &bytes);
        if tracing() {
            set(&message, "trace", &true.into());
        }
        worker
            .post_message_with_transfer(&message, &js_sys::Array::of1(&bytes.buffer()))
            .map_err(|error| format!("The kernel worker did not take the job: {error:?}"))
    }

    /// Stops the worker's jobs that nobody wants any more, such as a cancelled edit's
    /// build, instead of letting them run to completion. The worker cannot be interrupted
    /// inside a job, so it is replaced: the jobs still wanted are sent again to a new
    /// worker under their ids, and the unwanted ones are dropped unanswered, as a
    /// superseded job is. Returns whether a job was dropped.
    pub fn abandon_unwanted() -> bool {
        CLIENT.with_borrow_mut(|client| {
            let Some(old) = client.as_ref() else { return false };
            let mut shared = old.shared.borrow_mut();
            let unwanted: Vec<u64> =
                shared.waiting.iter().filter(|(_, (_, _, _, live, _))| !live()).map(|(id, _)| *id).collect();
            if unwanted.is_empty() || shared.stopped {
                return false;
            }
            for id in &unwanted {
                shared.waiting.remove(id);
                shared.early.remove(id);
                trace("page abandoned job", *id, String::new());
            }
            let wanted: Vec<(u64, Waiting)> = shared.waiting.drain().collect();
            let mut early: HashMap<u64, Box<dyn FnMut()>> = shared.early.drain().collect();
            drop(shared);
            // Dropping the old client terminates its worker, and the job it was running.
            *client = None;
            let fresh = match Client::start() {
                Ok(fresh) => fresh,
                Err(error) => {
                    for (_, (answered, ctx, _, _, _)) in wanted {
                        answered(Err(error.clone()));
                        ctx.request_repaint();
                    }
                    return true;
                }
            };
            for (id, waiting) in wanted {
                match post(&fresh.worker, id, &waiting.4) {
                    Ok(()) => {
                        let mut shared = fresh.shared.borrow_mut();
                        shared.waiting.insert(id, waiting);
                        if let Some(early) = early.remove(&id) {
                            shared.early.insert(id, early);
                        }
                    }
                    Err(error) => {
                        let (answered, ctx, _, _, _) = waiting;
                        answered(Err(error));
                        ctx.request_repaint();
                    }
                }
            }
            *client = Some(fresh);
            true
        })
    }

    /// The array a browser check sets as `sdKernelTrace` to follow each job, if any.
    fn trace_list() -> Option<js_sys::Array> {
        get(&js_sys::global(), "sdKernelTrace").dyn_into::<js_sys::Array>().ok()
    }

    fn tracing() -> bool {
        trace_list().is_some()
    }

    /// Notes one step of job `id` in the page's `sdKernelTrace`, when a check set one.
    pub fn trace(stage: &str, id: u64, detail: String) {
        if let Some(list) = trace_list() {
            let entry = js_sys::Object::new();
            set(&entry, "stage", &stage.into());
            set(&entry, "id", &(id as f64).into());
            set(&entry, "detail", &detail.into());
            set(&entry, "at", &js_sys::Date::now().into());
            list.push(&entry);
        }
    }

    /// The builds the worker's kernel made for the job just answered, as bytes the page's
    /// kernel can be offered. Empty when there is no host yet or nothing was built.
    #[wasm_bindgen]
    pub fn kernel_worker_hand_over() -> Vec<u8> {
        HOST.with_borrow_mut(|host| host.as_mut().and_then(|host| host.take_hand_over().ok())).unwrap_or_default()
    }

    /// The worker's entry: runs job `id` on this worker's kernel and answers as JSON. A
    /// checked build posts its rebuilt part to the page as `{ id, early }` before that.
    #[wasm_bindgen]
    pub fn kernel_worker_run(header: &str, part: &[u8], id: f64) -> String {
        let early = |bytes: Vec<u8>| {
            let bytes = js_sys::Uint8Array::from(bytes.as_slice());
            let message = js_sys::Object::new();
            set(&message, "id", &id.into());
            set(&message, "early", &bytes);
            if let Ok(scope) = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>() {
                let _ = scope.post_message_with_transfer(&message, &js_sys::Array::of1(&bytes.buffer()));
            }
        };
        let result = serde_json::from_str::<Job>(header).map_err(|error| error.to_string()).and_then(|job| {
            HOST.with_borrow_mut(|host| {
                host.get_or_insert_with(|| {
                    report_panics();
                    Host::new(crate::startup::kernel())
                })
                .run_with(part, &job, &early)
            })
        });
        match result {
            Ok(value) => serde_json::json!({ "ok": value }),
            Err(error) => serde_json::json!({ "err": error }),
        }
        .to_string()
    }

    /// A panic in the worker traps the module. The page learns why before the trap does.
    fn report_panics() {
        std::panic::set_hook(Box::new(|info| {
            let message = js_sys::Object::new();
            set(&message, "panic", &info.to_string().into());
            if let Ok(scope) = js_sys::global().dyn_into::<web_sys::DedicatedWorkerGlobalScope>() {
                let _ = scope.post_message(&message);
            }
        }));
    }

    /// The address of the page's wasm-bindgen glue, which trunk preloads by name.
    fn glue_url() -> Option<String> {
        let document = web_sys::window()?.document()?;
        let links = document.query_selector_all("link[rel=modulepreload]").ok()?;
        (0..links.length())
            .filter_map(|i| links.item(i)?.dyn_into::<web_sys::HtmlLinkElement>().ok())
            .map(|link| link.href())
            .find(|href| href.ends_with(".js") && !href.contains("/snippets/"))
    }

    fn set(object: &js_sys::Object, key: &str, value: &JsValue) {
        let _ = js_sys::Reflect::set(object, &key.into(), value);
    }

    fn get(object: &JsValue, key: &str) -> JsValue {
        js_sys::Reflect::get(object, &key.into()).unwrap_or(JsValue::UNDEFINED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> crate::document::Document {
        crate::startup::demo_document()
    }

    #[test]
    fn the_worker_measures_the_part_it_was_sent() {
        let doc = demo();
        let feature = *doc.model().features().last().map(|f| &f.id).unwrap();
        let body = doc.model().body_after(feature).unwrap();
        let expected = doc.kernel().mass_properties(body).unwrap();
        let (_, part) = doc.model().snapshot().encode().unwrap();
        let mut host = Host::new(crate::startup::kernel());
        let answer = host.run(&part, &Job::MassProperties { feature }).unwrap();
        let measured: soliddraft_kernel_api::MassProperties = serde_json::from_value(answer).unwrap();
        assert_eq!(measured, expected);
    }

    #[test]
    fn the_same_part_is_not_rebuilt_and_an_edited_one_reuses_its_builds() {
        let mut doc = demo();
        let first = doc.model().features()[0].id;
        let (_, part) = doc.model().snapshot().encode().unwrap();
        let mut host = Host::new(crate::startup::kernel());
        host.run(&part, &Job::MassProperties { feature: first }).unwrap();
        let built = host.part.as_ref().unwrap().1.body_after(first);
        host.run(&part, &Job::MassProperties { feature: first }).unwrap();
        assert_eq!(host.part.as_ref().unwrap().1.body_after(first), built);

        let last = doc.features.last().unwrap().id;
        doc.apply(crate::document::Command::Rename { id: last, name: "Renamed".into() }).unwrap();
        let (_, renamed) = doc.model().snapshot().encode().unwrap();
        assert_ne!(renamed, part);
        host.run(&renamed, &Job::MassProperties { feature: first }).unwrap();
        assert_eq!(host.part.as_ref().unwrap().1.body_after(first), built, "unchanged inputs reuse the build");
    }

    #[test]
    fn a_feature_the_worker_cannot_find_is_refused() {
        let (_, part) = demo().model().snapshot().encode().unwrap();
        let mut host = Host::new(crate::startup::kernel());
        let error = host.run(&part, &Job::MassProperties { feature: model::FeatureId(9999) }).unwrap_err();
        assert!(error.contains("no such feature"), "{error}");
        assert!(host.run(b"not a part", &Job::MassProperties { feature: model::FeatureId(1) }).is_err());
    }

    #[test]
    fn the_worker_builds_proves_and_meshes_an_edited_part() {
        let doc = demo();
        let base = doc.features[0].id;
        let (_, part) = doc.model().snapshot().encode().unwrap();
        let edit = crate::drag::ScalarDraft {
            feature: model::FeatureId(u32::try_from(base).unwrap()),
            dimension: crate::drag::Dimension::Depth,
            value: 12.0,
        };
        let mut host = Host::new(crate::startup::kernel());
        for job in [
            Job::Build { edit: Some(edit), level: Default::default(), curved_surfaces: None, checked: false },
            Job::Build { edit: None, level: Default::default(), curved_surfaces: None, checked: false },
        ] {
            let answer = host.run(&part, &job).unwrap();
            assert_eq!(answer["failure"], serde_json::Value::Null, "{answer}");
            assert!(answer["triangles"].as_u64().unwrap() > 0, "{answer}");
            let statuses = answer["statuses"].as_array().unwrap();
            assert_eq!(statuses.len(), doc.features.len());
            assert!(statuses.iter().all(|status| status["status"].is_null()), "{answer}");
        }
        let too_deep = crate::drag::ScalarDraft { value: -5.0, ..edit };
        let refused = host.run(
            &part,
            &Job::Build { edit: Some(too_deep), level: Default::default(), curved_surfaces: None, checked: false },
        );
        assert!(refused.is_err() || !refused.as_ref().unwrap()["failure"].is_null(), "{refused:?}");
    }

    #[test]
    fn the_page_rebuilds_an_edited_part_from_the_workers_hand_over_without_evaluating_it() {
        let doc = demo();
        let base = doc.features[0].id;
        let (_, part) = doc.model().snapshot().encode().unwrap();
        let edit = crate::drag::ScalarDraft {
            feature: model::FeatureId(u32::try_from(base).unwrap()),
            dimension: crate::drag::Dimension::Depth,
            value: 12.0,
        };
        let worker = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        let mut host = Host::new(worker.clone());
        let answer = host
            .run(
                &part,
                &Job::Build { edit: Some(edit), level: Default::default(), curved_surfaces: None, checked: false },
            )
            .unwrap();
        assert_eq!(answer["failure"], serde_json::Value::Null, "{answer}");
        let bytes = host.take_hand_over().unwrap();

        // The page applies the same edit to its own definitions and rebuilds from the offer.
        let page = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        assert!(page.offer_builds(1, &bytes).unwrap() > 0);
        let mut edited = model::Document::from_bytes(&part, page.clone()).unwrap();
        let command = crate::document::scalar_draft_command(&edited, edit).unwrap();
        edited.execute_deferred(command).unwrap();
        edited.rebuild();
        assert!(edited.features().iter().all(|feature| edited.status(feature.id).is_some_and(|s| s.is_ok())));
        assert_eq!(page.fresh_builds(), 0, "the part and its edit were both taken from the worker");
        assert!(page.mass_properties(edited.body().unwrap()).unwrap().volume > 0.0);
    }

    /// The browser's sequence on the bracket: open through the worker, adopt on the page,
    /// then a depth edit through the worker. The page must name every face as its own
    /// build would and take every build from the worker.
    #[test]
    fn an_edit_after_an_adopted_open_names_faces_as_a_fresh_build_does() {
        let part = include_bytes!("../../../site/parts/bracket.dwprt").to_vec();
        let worker = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        let page = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        let mut host = Host::new(worker.clone());
        host.run(&part, &Job::Build { edit: None, level: Default::default(), curved_surfaces: None, checked: false })
            .unwrap();
        page.offer_builds(1, &host.take_hand_over().unwrap()).unwrap();
        let opened = model::Document::from_bytes(&part, page.clone()).unwrap();
        assert_eq!(page.fresh_builds(), 0);
        let base = opened.features().iter().find(|f| matches!(f.kind, model::FeatureKind::Extrude(_))).unwrap().id;
        let (_, saved) = opened.snapshot().encode().unwrap();
        let edit = crate::drag::ScalarDraft { feature: base, dimension: crate::drag::Dimension::Depth, value: 12.0 };
        host.run(
            &saved,
            &Job::Build { edit: Some(edit), level: Default::default(), curved_surfaces: None, checked: false },
        )
        .unwrap();
        page.offer_builds(1, &host.take_hand_over().unwrap()).unwrap();
        let mut edited = opened.fork();
        edited.execute_deferred(crate::document::scalar_draft_command(&edited, edit).unwrap()).unwrap();
        edited.reuse_builds_from(&opened);
        edited.rebuild();

        let mut fresh = model::Document::from_bytes(&part, crate::startup::kernel()).unwrap();
        fresh.execute_deferred(crate::document::scalar_draft_command(&fresh, edit).unwrap()).unwrap();
        fresh.rebuild();
        for feature in fresh.features() {
            assert_eq!(edited.status(feature.id), fresh.status(feature.id), "{}", feature.name);
            assert_eq!(edited.generated_faces(feature.id), fresh.generated_faces(feature.id), "{}", feature.name);
        }
        assert_eq!(page.fresh_builds(), 0, "every build was taken from the worker");
        // The page shows the edited part without reading any handed-over body.
        crate::document::ready_display(&edited, soliddraft_tessellation::Level::Standard, None).unwrap();
        assert_eq!(page.bodies_read(), 0, "the edit and its display read no exact body");
    }

    /// A restarted worker numbers its entities from the start again. The builds a second
    /// worker hands over take new page ids, never the ids of entities a first worker's
    /// builds took on the same page kernel.
    #[test]
    fn a_second_workers_builds_take_none_of_the_firsts_ids() {
        let page = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        let build = Job::Build { edit: None, level: Default::default(), curved_surfaces: None, checked: false };
        let bracket = include_bytes!("../../../site/parts/bracket.dwprt").to_vec();
        let mut first = Host::new(Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new()));
        first.run(&bracket, &build).unwrap();
        page.offer_builds(1, &first.take_hand_over().unwrap()).unwrap();
        let opened = model::Document::from_bytes(&bracket, page.clone()).unwrap();
        let tray = include_bytes!("../../../site/parts/tray.dwprt").to_vec();
        let mut second = Host::new(Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new()));
        second.run(&tray, &build).unwrap();
        page.offer_builds(2, &second.take_hand_over().unwrap()).unwrap();
        let other = model::Document::from_bytes(&tray, page.clone()).unwrap();
        assert_eq!(page.fresh_builds(), 0, "both parts were taken from their workers");
        let ids = |doc: &model::Document| {
            let (_, body) = doc.live_bodies().last().copied().unwrap();
            let topology = page.topology(body).unwrap();
            let faces = topology.faces.iter().map(|face| face.id.raw());
            faces.chain(topology.edges.iter().map(|edge| edge.id.raw())).collect::<std::collections::BTreeSet<_>>()
        };
        let (bracket, tray) = (ids(&opened), ids(&other));
        let shared: Vec<_> = bracket.intersection(&tray).take(5).collect();
        assert!(shared.is_empty(), "the tray took ids of the bracket: {shared:?}");
    }

    /// The tree names a body every frame, through `Kernel::body_kind`. On a body adopted
    /// unread with no topology handed over, that answers from what came with the build
    /// and reads no exact geometry, where the topology the name asked for before does.
    #[test]
    fn a_body_kind_reads_no_exact_geometry() {
        let part = include_bytes!("../../../site/parts/bracket.dwprt").to_vec();
        let page = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        let worker = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        worker.keep_builds().unwrap();
        // Built and handed over before anything meshed it, so no topology travels.
        model::Document::from_bytes(&part, worker.clone()).unwrap();
        page.offer_builds(1, &worker.hand_over().unwrap()).unwrap();
        let opened = model::Document::from_bytes(&part, page.clone()).unwrap();
        let (_, body) = opened.live_bodies().last().copied().unwrap();
        assert_eq!(page.bodies_read(), 0);
        assert_eq!(page.body_kind(body).unwrap(), soliddraft_kernel_api::BodyKind::Solid);
        assert_eq!(page.bodies_read(), 0, "the body's kind read its exact geometry");
        page.topology(body).unwrap();
        assert_eq!(page.bodies_read(), 1, "its topology is computed from the exact geometry");
    }

    /// The browser with a GPU preview. The worker hands the rebuilt part over as soon as
    /// it is built, then makes the checked meshes. The page describes the body for its GPU
    /// from the early hand-over, and takes every checked mesh from the answer's without
    /// reading the exact body.
    #[test]
    fn a_checked_build_hands_its_part_over_early_and_its_checked_meshes_after() {
        use soliddraft_tessellation::Level;

        let part = include_bytes!("../../../site/parts/bracket.dwprt").to_vec();
        let worker = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        let page = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        let mut host = Host::new(worker.clone());
        let early = std::cell::RefCell::new(Vec::new());
        let job = Job::Build { edit: None, level: Default::default(), curved_surfaces: None, checked: true };
        host.run_with(&part, &job, &|bytes| early.borrow_mut().push(bytes)).unwrap();
        let early = early.into_inner();
        assert_eq!(early.len(), 1, "one early hand-over");
        assert!(page.offer_builds(1, &early[0]).unwrap() > 0);
        let opened = model::Document::from_bytes(&part, page.clone()).unwrap();
        assert_eq!(page.fresh_builds(), 0, "the early hand-over carried the part");
        let (_, body) = opened.live_bodies().last().copied().unwrap();
        let topology = page.topology(body).unwrap();
        let levels = crate::graphics::body_mesh_levels(None, &opened.settings().viewport, &topology);
        assert!(page.gpu_preview(body, levels.params(Level::Standard)).unwrap().is_some());

        page.offer_builds(1, &host.take_hand_over().unwrap()).unwrap();
        let read = page.bodies_read();
        for level in [Level::Coarse, Level::Standard, Level::Fine] {
            let params = levels.params(level);
            assert!(!page.checked_rendering(body, params).unwrap().indices.is_empty());
            page.checked_rendering_edges(body, params).unwrap();
        }
        assert_eq!(page.bodies_read(), read, "the page made a checked mesh the worker had made");
        assert_eq!(page.fresh_previews(), 0, "the page sampled a CPU preview");
    }

    #[test]
    fn jobs_round_trip_as_the_worker_reads_them() {
        let job = Job::MassProperties { feature: model::FeatureId(3) };
        let header = serde_json::to_string(&job).unwrap();
        assert_eq!(header, r#"{"job":"mass_properties","feature":3}"#);
        assert_eq!(serde_json::from_str::<Job>(&header).unwrap(), job);
    }

    /// A curve change in the browser: the worker samples the bodies the page already
    /// holds at the new setting, and the page shows them without sampling them again.
    #[test]
    fn a_remesh_hands_its_meshes_to_bodies_the_page_already_holds() {
        use crate::graphics::{CurvedSurfaces, body_mesh_levels};

        let part = include_bytes!("../../../site/parts/bracket.dwprt").to_vec();
        let worker = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        let page = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        let mut host = Host::new(worker.clone());
        host.run(&part, &Job::Build { edit: None, level: Default::default(), curved_surfaces: None, checked: false })
            .unwrap();
        page.offer_builds(1, &host.take_hand_over().unwrap()).unwrap();
        let opened = model::Document::from_bytes(&part, page.clone()).unwrap();

        let choice = Some(CurvedSurfaces::Ultra);
        host.run(&part, &Job::Remesh { curved_surfaces: choice, checked: false }).unwrap();
        page.offer_builds(1, &host.take_hand_over().unwrap()).unwrap();
        let sampled = page.fresh_previews();
        let mut levels = 0;
        for (_, body) in opened.live_bodies() {
            let topology = page.topology(body).unwrap();
            for params in body_mesh_levels(choice, &opened.settings().viewport, &topology).all() {
                page.preview(body, params).unwrap();
                page.preview_edges(body, params).unwrap();
                levels += 1;
            }
        }
        assert!(levels > 0);
        assert_eq!(page.fresh_previews(), sampled, "the page sampled no level the worker had made");
    }
}
