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
    },
}

impl Job {
    /// Jobs of one slot replace each other while they wait.
    pub fn slot(&self) -> &'static str {
        match self {
            Job::MassProperties { .. } => "mass_properties",
            Job::Build { .. } => "build",
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
            Job::Build { edit, level } => {
                let proved = crate::drag::prove_now(std::sync::Arc::new(doc.fork()), *edit, *level)?;
                let model = &proved.model;
                let statuses: Vec<_> = model
                    .features()
                    .iter()
                    .map(|feature| serde_json::json!({ "feature": feature.id, "status": built(model, feature.id).err() }))
                    .collect();
                let triangles = proved.display.as_ref().map(|display| display.mesh.indices.len() / 3);
                sample_display_levels(model);
                Ok(serde_json::json!({
                    "failure": proved.failure,
                    "display_error": proved.display_error,
                    "statuses": statuses,
                    "triangles": triangles,
                }))
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
/// samples none.
fn sample_display_levels(model: &model::Document) {
    use soliddraft_tessellation::{Level, Levels};
    let Some((_, body)) = model.live_bodies().last().copied() else { return };
    let kernel = model.kernel();
    let Ok(topology) = kernel.topology(body) else { return };
    let levels = Levels::new(&model.settings().viewport, topology.bbox.diagonal());
    for level in [Level::Coarse, Level::Standard, Level::Fine] {
        let params = *levels.params(level);
        let _ = kernel.preview(body, &params);
        let _ = kernel.preview_edges(body, &params);
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
pub use browser::{submit, trace};

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
        static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
    }

    /// Whether the page still wants a job's builds: a cancelled or replaced job's answer
    /// still arrives, and its builds must not reach the kernel a later job reads.
    type Live = Box<dyn Fn() -> bool>;

    /// Who waits for a job: its answer's handler, the context to repaint, the kernel of
    /// the part that asked, which is offered the builds the worker made for the job, and
    /// whether that job is still wanted.
    type Waiting = (Answered, egui::Context, Arc<dyn soliddraft_kernel_api::Kernel>, Live);

    #[derive(Default)]
    struct Shared {
        waiting: HashMap<u64, Waiting>,
        /// The last panic the worker reported, the reason for the trap that follows it.
        panic: Option<String>,
        stopped: bool,
    }

    impl Shared {
        /// Whether job `id`'s builds are still wanted, before they are copied off the message.
        fn live(&self, id: u64) -> bool {
            self.waiting.get(&id).is_some_and(|(_, _, _, live)| live())
        }

        fn answer(&mut self, id: u64, result: Result<serde_json::Value, String>, hand_over: &[u8]) {
            if let Some((answered, ctx, kernel, _)) = self.waiting.remove(&id) {
                // The builds come first, so a rebuild the answer starts takes them.
                if !hand_over.is_empty()
                    && let Err(error) = kernel.offer_builds(hand_over)
                {
                    log::warn!("the page could not take the kernel worker's builds: {error}");
                }
                answered(result);
                ctx.request_repaint();
            }
        }

        fn stop(&mut self, reason: &str) {
            self.stopped = true;
            for (_, (answered, ctx, _, _)) in self.waiting.drain() {
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
            let shared = Rc::new(RefCell::new(Shared::default()));
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
            let kind = ["answer", "fatal", "superseded"].into_iter().find(|key| !get(data, key).is_undefined());
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
        CLIENT.with_borrow_mut(|client| {
            if client.as_ref().is_none_or(|client| client.shared.borrow().stopped) {
                // Dropping the stopped client here, outside its own handlers, ends it.
                *client = Some(Client::start()?);
            }
            let client = client.as_ref().expect("the kernel worker started");
            let id = NEXT.with(|next| next.replace(next.get() + 1));
            let header = serde_json::to_string(job).map_err(|error| error.to_string())?;
            let bytes = js_sys::Uint8Array::from(part.as_slice());
            let message = js_sys::Object::new();
            set(&message, "id", &(id as f64).into());
            set(&message, "slot", &job.slot().into());
            set(&message, "header", &header.into());
            set(&message, "part", &bytes);
            if tracing() {
                set(&message, "trace", &true.into());
            }
            client
                .worker
                .post_message_with_transfer(&message, &js_sys::Array::of1(&bytes.buffer()))
                .map_err(|error| format!("The kernel worker did not take the job: {error:?}"))?;
            client.shared.borrow_mut().waiting.insert(id, (Box::new(answered), ctx.clone(), kernel, Box::new(live)));
            Ok(id)
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

    /// The worker's entry: runs one job on this worker's kernel and answers as JSON.
    #[wasm_bindgen]
    pub fn kernel_worker_run(header: &str, part: &[u8]) -> String {
        let result = serde_json::from_str::<Job>(header).map_err(|error| error.to_string()).and_then(|job| {
            HOST.with_borrow_mut(|host| {
                host.get_or_insert_with(|| {
                    report_panics();
                    Host::new(crate::kernel())
                })
                .run(part, &job)
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
        crate::demo_document()
    }

    #[test]
    fn the_worker_measures_the_part_it_was_sent() {
        let doc = demo();
        let feature = *doc.model().features().last().map(|f| &f.id).unwrap();
        let body = doc.model().body_after(feature).unwrap();
        let expected = doc.kernel().mass_properties(body).unwrap();
        let (_, part) = doc.model().snapshot().encode().unwrap();
        let mut host = Host::new(crate::kernel());
        let answer = host.run(&part, &Job::MassProperties { feature }).unwrap();
        let measured: soliddraft_kernel_api::MassProperties = serde_json::from_value(answer).unwrap();
        assert_eq!(measured, expected);
    }

    #[test]
    fn the_same_part_is_not_rebuilt_and_an_edited_one_reuses_its_builds() {
        let mut doc = demo();
        let first = doc.model().features()[0].id;
        let (_, part) = doc.model().snapshot().encode().unwrap();
        let mut host = Host::new(crate::kernel());
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
        let mut host = Host::new(crate::kernel());
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
        let mut host = Host::new(crate::kernel());
        for job in [
            Job::Build { edit: Some(edit), level: Default::default() },
            Job::Build { edit: None, level: Default::default() },
        ] {
            let answer = host.run(&part, &job).unwrap();
            assert_eq!(answer["failure"], serde_json::Value::Null, "{answer}");
            assert!(answer["triangles"].as_u64().unwrap() > 0, "{answer}");
            let statuses = answer["statuses"].as_array().unwrap();
            assert_eq!(statuses.len(), doc.features.len());
            assert!(statuses.iter().all(|status| status["status"].is_null()), "{answer}");
        }
        let too_deep = crate::drag::ScalarDraft { value: -5.0, ..edit };
        let refused = host.run(&part, &Job::Build { edit: Some(too_deep), level: Default::default() });
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
        let answer = host.run(&part, &Job::Build { edit: Some(edit), level: Default::default() }).unwrap();
        assert_eq!(answer["failure"], serde_json::Value::Null, "{answer}");
        let bytes = host.take_hand_over().unwrap();

        // The page applies the same edit to its own definitions and rebuilds from the offer.
        let page = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
        assert!(page.offer_builds(&bytes).unwrap() > 0);
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
        host.run(&part, &Job::Build { edit: None, level: Default::default() }).unwrap();
        page.offer_builds(&host.take_hand_over().unwrap()).unwrap();
        let opened = model::Document::from_bytes(&part, page.clone()).unwrap();
        assert_eq!(page.fresh_builds(), 0);
        let base = opened.features().iter().find(|f| matches!(f.kind, model::FeatureKind::Extrude(_))).unwrap().id;
        let (_, saved) = opened.snapshot().encode().unwrap();
        let edit = crate::drag::ScalarDraft { feature: base, dimension: crate::drag::Dimension::Depth, value: 12.0 };
        host.run(&saved, &Job::Build { edit: Some(edit), level: Default::default() }).unwrap();
        page.offer_builds(&host.take_hand_over().unwrap()).unwrap();
        let mut edited = opened.fork();
        edited.execute_deferred(crate::document::scalar_draft_command(&edited, edit).unwrap()).unwrap();
        edited.reuse_builds_from(&opened);
        edited.rebuild();

        let mut fresh = model::Document::from_bytes(&part, crate::kernel()).unwrap();
        fresh.execute_deferred(crate::document::scalar_draft_command(&fresh, edit).unwrap()).unwrap();
        fresh.rebuild();
        for feature in fresh.features() {
            assert_eq!(edited.status(feature.id), fresh.status(feature.id), "{}", feature.name);
            assert_eq!(edited.generated_faces(feature.id), fresh.generated_faces(feature.id), "{}", feature.name);
        }
        assert_eq!(page.fresh_builds(), 0, "every build was taken from the worker");
        // The page shows the edited part without reading any handed-over body.
        crate::document::ready_display(&edited, soliddraft_tessellation::Level::Standard).unwrap();
        assert_eq!(page.bodies_read(), 0, "the edit and its display read no exact body");
    }

    #[test]
    fn jobs_round_trip_as_the_worker_reads_them() {
        let job = Job::MassProperties { feature: model::FeatureId(3) };
        let header = serde_json::to_string(&job).unwrap();
        assert_eq!(header, r#"{"job":"mass_properties","feature":3}"#);
        assert_eq!(serde_json::from_str::<Job>(&header).unwrap(), job);
    }
}
