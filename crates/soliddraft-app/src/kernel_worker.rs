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
}

impl Job {
    /// Jobs of one slot replace each other while they wait.
    pub fn slot(&self) -> &'static str {
        match self {
            Job::MassProperties { .. } => "mass_properties",
        }
    }
}

/// The worker's kernel and the part it built last.
pub struct Host {
    kernel: Arc<dyn Kernel>,
    part: Option<(Vec<u8>, model::Document)>,
}

impl Host {
    pub fn new(kernel: Arc<dyn Kernel>) -> Self {
        Self { kernel, part: None }
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
pub use browser::{submit, trace};

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::{Host, Job};
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
        ctx: &egui::Context,
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
            client.shared.borrow_mut().waiting.insert(id, (Box::new(answered), ctx.clone()));
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
    fn jobs_round_trip_as_the_worker_reads_them() {
        let job = Job::MassProperties { feature: model::FeatureId(3) };
        let header = serde_json::to_string(&job).unwrap();
        assert_eq!(header, r#"{"job":"mass_properties","feature":3}"#);
        assert_eq!(serde_json::from_str::<Job>(&header).unwrap(), job);
    }
}
