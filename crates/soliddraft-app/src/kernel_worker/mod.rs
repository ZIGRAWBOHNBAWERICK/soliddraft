//! Kernel work the browser runs off its interface thread.
//!
//! The page has one thread, and a long Catallax query on it freezes the camera and every
//! control until it ends. The browser build therefore starts a dedicated Web Worker
//! (`kernel_worker.js`) on the same compiled WebAssembly module. The worker has its own
//! memory and owns the only Catallax kernel in the browser.
//!
//! The page's parts live on a [`calls::ReplayKernel`]. To build a definition, the page
//! sends its saved bytes to the worker, which rebuilds them with a
//! [`calls::RecordingKernel`] and answers with every kernel call it made. The page then
//! runs its ordinary rebuild, which makes the same calls and gets the worker's answers
//! back with the worker's body ids, so nothing is evaluated on the interface thread. A
//! query the worker has not answered is refused as unavailable for now and sent to the
//! worker as a [`Job::Calls`] job.
//!
//! The interface stays the owner of its part. A job's answer is accepted only for the
//! document and revision that asked for it; the caller keeps that check. A newer job of
//! the same kind replaces one that has not started. A worker that traps answers the job
//! it was running with the reason, and the next job starts a new worker, so a failure
//! never takes the part on screen with it.
//!
//! [`Host`] is the worker's half. It is plain Rust, so native tests run it directly.

pub mod calls;

use calls::{Call, RecordingKernel};
use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::Kernel;
use soliddraft_model as model;
use std::collections::HashMap;
use std::sync::Arc;

/// Answered when a newer job of the same kind replaced one before it started.
pub const SUPERSEDED: &str = "A newer request replaced this one";

/// One request to the kernel worker. The part travels beside it as saved bytes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "job", rename_all = "snake_case")]
pub enum Job {
    /// Volume, area, centroid and inertia of the body after `feature`.
    MassProperties { feature: model::FeatureId },
    /// Rebuilds the part and its display at `level`, as the page's proof will, and
    /// answers with the calls made. `cold` builds without any earlier build.
    Prove {
        document: soliddraft_document::DocumentId,
        level: soliddraft_tessellation::Level,
        budget: u64,
        cold: bool,
    },
    /// Runs calls the page could not replay and answers with them.
    Calls { calls: Vec<Call> },
}

/// The worker's kernel and the parts it built last.
pub struct Host {
    kernel: Arc<RecordingKernel>,
    part: Option<(Vec<u8>, model::Document)>,
    /// The last part each document's proof built without a failure, to build the next on.
    proved: HashMap<soliddraft_document::DocumentId, model::Document>,
}

impl Host {
    pub fn new(kernel: Arc<dyn Kernel>) -> Self {
        Self { kernel: Arc::new(RecordingKernel::new(kernel)), part: None, proved: HashMap::new() }
    }

    /// Runs `job` against the part saved as `part` and answers with plain data.
    pub fn run(&mut self, part: &[u8], job: &Job) -> Result<serde_json::Value, String> {
        let answer = self.answer(part, job);
        let calls = self.kernel.take();
        match job {
            Job::MassProperties { .. } => answer,
            Job::Prove { .. } | Job::Calls { .. } => {
                answer?;
                serde_json::to_value(calls).map_err(|error| error.to_string())
            }
        }
    }

    fn answer(&mut self, part: &[u8], job: &Job) -> Result<serde_json::Value, String> {
        match job {
            Job::MassProperties { feature } => {
                let doc = self.open(part)?;
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
            Job::Prove { document, level, budget, cold } => {
                let previous = if *cold { None } else { self.proved.get(document) };
                let mut doc =
                    model::Document::from_bytes_with_input(part, self.kernel.clone(), previous, |_| {})
                        .map_err(|error| format!("The kernel worker could not read the part: {error}"))?;
                doc.set_rebuild_budget(*budget);
                if crate::drag::model_failure(&doc).is_none() {
                    // The page asks for the same display once its own rebuild succeeds,
                    // and adopting it reads these for every body it shows.
                    let _ = crate::document::ready_display(&doc, *level);
                    for (_, body) in doc.live_bodies() {
                        let _ = self.kernel.display_bound(body);
                        let _ = self.kernel.body_notice(body);
                        let _ = self.kernel.topology(body);
                    }
                    if !doc.is_dirty() {
                        self.proved.insert(*document, doc);
                    }
                }
                Ok(serde_json::Value::Null)
            }
            Job::Calls { calls } => {
                for call in calls {
                    calls::dispatch(self.kernel.as_ref(), call)?;
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
pub use browser::{fetch_missed, generation, replay, submit};

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::calls::{Answered as Recorded, Call, ReplayKernel};
    use super::{Host, Job, SUPERSEDED};
    use soliddraft_kernel_api::Kernel;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;

    /// What a job's answer is handed to on the interface thread.
    type Answered = Box<dyn FnOnce(Result<serde_json::Value, String>)>;

    thread_local! {
        static CLIENT: RefCell<Option<Client>> = const { RefCell::new(None) };
        static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
        static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
        static REPLAY: std::sync::Arc<ReplayKernel> =
            std::sync::Arc::new(ReplayKernel::new(soliddraft_kernel_catallax::CatallaxKernel::new().backend()));
        /// Workers started so far. Body ids from one worker mean nothing to the next.
        static GENERATION: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        /// Keys of missed calls sent to the worker and not answered yet.
        static FETCHING: RefCell<std::collections::HashSet<String>> = RefCell::new(Default::default());
    }

    /// The page's kernel: every part in the browser is built on it.
    pub fn replay() -> std::sync::Arc<ReplayKernel> {
        REPLAY.with(|replay| replay.clone())
    }

    /// Changes when a new worker starts. A rebuild replayed from an older worker's
    /// answers must not be adopted.
    pub fn generation() -> u64 {
        GENERATION.get()
    }

    /// Sends the calls the page could not replay to the worker, each once.
    pub fn fetch_missed(ctx: &egui::Context) {
        let missed = replay().take_missed();
        let calls: Vec<Call> = FETCHING.with_borrow_mut(|fetching| {
            missed
                .into_iter()
                .filter(|call| fetching.insert(super::calls::key(&call.method, &call.args)))
                .collect()
        });
        if calls.is_empty() {
            return;
        }
        let keys: Vec<String> = calls.iter().map(|call| super::calls::key(&call.method, &call.args)).collect();
        let slot = format!("fetch:{}", keys[0]);
        let started = generation();
        let submitted = submit(Vec::new(), &Job::Calls { calls }, &slot, ctx, move |answer| {
            FETCHING.with_borrow_mut(|fetching| keys.iter().for_each(|key| _ = fetching.remove(key)));
            if generation() != started {
                return;
            }
            match answer.and_then(|answer| serde_json::from_value::<Vec<Recorded>>(answer).map_err(|e| e.to_string())) {
                Ok(recorded) => replay().learn(recorded),
                Err(error) => log::warn!("The kernel worker could not answer a query: {error}"),
            }
        });
        if let Err(error) = submitted {
            log::warn!("{error}");
        }
    }

    #[derive(Default)]
    struct Shared {
        waiting: HashMap<u64, (Answered, egui::Context)>,
        /// The last panic the worker reported, the reason for the trap that follows it.
        panic: Option<String>,
        stopped: bool,
    }

    impl Shared {
        fn answer(&mut self, id: u64, result: Result<serde_json::Value, String>) {
            if let Some((answered, ctx)) = self.waiting.remove(&id) {
                answered(result);
                ctx.request_repaint();
            }
        }

        fn stop(&mut self, reason: &str) {
            self.stopped = true;
            for (_, (answered, ctx)) in self.waiting.drain() {
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
            shared.answer(id, Err(SUPERSEDED.into()));
        } else if let Some(answer) = get(data, "answer").as_string() {
            shared.answer(id, decode(&answer));
        } else if let Some(fatal) = get(data, "fatal").as_string() {
            let reason = shared.panic.take().unwrap_or(fatal);
            shared.answer(id, Err(format!("The kernel worker stopped: {reason}. Try again.")));
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
    /// interface thread with the answer, and `ctx` repaints then.
    pub fn submit(
        part: Vec<u8>,
        job: &Job,
        slot: &str,
        ctx: &egui::Context,
        answered: impl FnOnce(Result<serde_json::Value, String>) + 'static,
    ) -> Result<(), String> {
        CLIENT.with_borrow_mut(|client| {
            if client.as_ref().is_none_or(|client| client.shared.borrow().stopped) {
                // Dropping the stopped client here, outside its own handlers, ends it.
                let restarted = client.is_some();
                *client = Some(Client::start()?);
                GENERATION.set(GENERATION.get() + 1);
                if restarted {
                    // The new worker numbers its bodies from the start again.
                    replay().forget();
                    FETCHING.with_borrow_mut(|fetching| fetching.clear());
                }
            }
            let client = client.as_ref().expect("the kernel worker started");
            let id = NEXT.with(|next| next.replace(next.get() + 1));
            let header = serde_json::to_string(job).map_err(|error| error.to_string())?;
            let bytes = js_sys::Uint8Array::from(part.as_slice());
            let message = js_sys::Object::new();
            set(&message, "id", &(id as f64).into());
            set(&message, "slot", &slot.into());
            set(&message, "header", &header.into());
            set(&message, "part", &bytes);
            client
                .worker
                .post_message_with_transfer(&message, &js_sys::Array::of1(&bytes.buffer()))
                .map_err(|error| format!("The kernel worker did not take the job: {error:?}"))?;
            client.shared.borrow_mut().waiting.insert(id, (Box::new(answered), ctx.clone()));
            Ok(())
        })
    }

    /// The worker's entry: runs one job on this worker's kernel and answers as JSON.
    #[wasm_bindgen]
    pub fn kernel_worker_run(header: &str, part: &[u8]) -> String {
        let result = serde_json::from_str::<Job>(header).map_err(|error| error.to_string()).and_then(|job| {
            HOST.with_borrow_mut(|host| {
                host.get_or_insert_with(|| {
                    report_panics();
                    Host::new(std::sync::Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new()))
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
    fn jobs_round_trip_as_the_worker_reads_them() {
        let job = Job::MassProperties { feature: model::FeatureId(3) };
        let header = serde_json::to_string(&job).unwrap();
        assert_eq!(header, r#"{"job":"mass_properties","feature":3}"#);
        assert_eq!(serde_json::from_str::<Job>(&header).unwrap(), job);
    }
}
