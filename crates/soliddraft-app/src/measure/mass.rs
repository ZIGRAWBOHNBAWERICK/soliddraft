//! Requested body properties, computed away from the desktop event loop.

use crate::document::{Document, FeatureId};
use soliddraft_kernel_api::BodyId;
use std::sync::mpsc;

#[cfg(not(target_arch = "wasm32"))]
use super::MASS_WORK_LIMIT;

#[derive(Clone, Copy, PartialEq)]
struct Request {
    document: soliddraft_document::DocumentId,
    revision: u64,
    feature: FeatureId,
    body: BodyId,
}

impl Request {
    fn of(doc: &Document, feature: FeatureId) -> Option<Self> {
        doc.proof_state().current_proved_display().then_some(())?;
        Some(Self { document: doc.model().id(), revision: doc.revision(), feature, body: doc.feature(feature)?.body? })
    }

    fn current(self, doc: &Document) -> bool {
        Self::of(doc, self.feature) == Some(self)
    }
}

struct Pending {
    request: Request,
    answer: mpsc::Receiver<Result<(), String>>,
}

#[cfg(not(target_arch = "wasm32"))]
type Job = Box<dyn FnOnce() + Send>;

#[cfg(not(target_arch = "wasm32"))]
struct Worker {
    next: std::sync::Arc<std::sync::Mutex<Option<Job>>>,
    wake: mpsc::SyncSender<()>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Worker {
    fn new() -> Result<Self, String> {
        let next = std::sync::Arc::new(std::sync::Mutex::new(None::<Job>));
        let queued = next.clone();
        let (wake, ready) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("mass-properties".into())
            .spawn(move || {
                while ready.recv().is_ok() {
                    let job = queued.lock().unwrap().take();
                    if let Some(job) = job {
                        job();
                    }
                }
            })
            .map_err(|_| "Could not start the measurement. Try again.")?;
        Ok(Self { next, wake })
    }

    fn submit(&self, job: Job) -> Result<(), String> {
        *self.next.lock().unwrap() = Some(job);
        match self.wake.try_send(()) {
            Ok(()) | Err(mpsc::TrySendError::Full(())) => Ok(()),
            Err(mpsc::TrySendError::Disconnected(())) => Err("The measurement stopped. Try again.".into()),
        }
    }

    fn clear_queue(&self) {
        self.next.lock().unwrap().take();
    }
}

/// One worker and one replaceable queued request per window.
#[derive(Default)]
pub(crate) struct State {
    pending: Option<Pending>,
    failure: Option<(Request, String)>,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<Worker>,
}

impl State {
    /// Starts a measurement without waiting for it on the interface thread.
    pub(crate) fn request(&mut self, doc: &Document, feature: FeatureId, ctx: &egui::Context) -> Result<(), String> {
        let request = Request::of(doc, feature).ok_or("Select a solid feature with measurable properties")?;
        self.failure = None;
        if doc.kernel().cached_mass_properties(request.body).is_some() {
            self.pending = None;
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(worker) = &self.worker {
                worker.clear_queue();
            }
            return Ok(());
        }
        if self.pending.as_ref().is_some_and(|pending| pending.request == request) {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // The fork retains the immutable body even if the user opens another part.
            let model = doc.model().fork();
            let ctx = ctx.clone();
            let (sender, answer) = mpsc::channel();
            if self.worker.is_none() {
                self.worker = Some(Worker::new()?);
            }
            let submitted = self.worker.as_ref().unwrap().submit(Box::new(move || {
                let started = std::time::Instant::now();
                let (result, work) = soliddraft_kernel_catallax::with_request_allowance(MASS_WORK_LIMIT, || {
                    model.kernel().mass_properties(request.body)
                });
                log::debug!("mass properties: {} work units in {} ms", work.used, started.elapsed().as_millis());
                let result = if work.exhausted {
                    Err(format!("mass_properties: numbers.operation-budget: {} work units exhausted", work.limit))
                } else {
                    result.map(|_| ()).map_err(|e| e.to_string())
                };
                let _ = sender.send(result);
                ctx.request_repaint();
            }));
            if let Err(error) = submitted {
                self.worker = None;
                return Err(error);
            }
            self.pending = Some(Pending { request, answer });
            Ok(())
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            Err("Coming soon".into())
        }
    }

    /// Accepts an answer only for the document revision that requested it.
    pub(crate) fn poll(&mut self, doc: &Document) -> Option<String> {
        let pending = self.pending.as_ref()?;
        if !pending.request.current(doc) {
            self.pending = None;
            self.failure = None;
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(worker) = &self.worker {
                worker.clear_queue();
            }
            return None;
        }
        let result = match pending.answer.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return None,
            Err(mpsc::TryRecvError::Disconnected) => Err("The measurement stopped. Try again.".into()),
        };
        let request = pending.request;
        self.pending = None;
        if request.current(doc) {
            self.failure = result.err().map(|error| (request, error));
            return self.failure.as_ref().map(|(_, error)| error.clone());
        }
        None
    }

    /// Pending or refused properties for this feature, without querying geometry.
    pub(crate) fn message<'a>(&'a self, doc: &Document, feature: FeatureId) -> Option<&'a str> {
        let request = Request::of(doc, feature)?;
        if self.pending.as_ref().is_some_and(|p| p.request == request) {
            return Some("Calculating mass properties...");
        }
        self.failure.as_ref().filter(|(failed, _)| *failed == request).map(|(_, error)| error.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Command, ExtrudeParams};

    #[test]
    fn an_unfinished_old_request_does_not_block_the_edited_part() {
        let mut doc = Document::new("Body", crate::kernel());
        let feature =
            doc.apply(Command::AddExtrude { params: ExtrudeParams::default(), via_assistant: false }).unwrap().unwrap();
        let request = Request::of(&doc, feature).unwrap();
        let (_held_sender, answer) = mpsc::channel();
        let mut state = State { pending: Some(Pending { request, answer }), ..Default::default() };
        doc.apply(Command::Rename { id: feature, name: "Changed".into() }).unwrap();
        state.poll(&doc);
        assert!(state.pending.is_none());
        state.request(&doc, feature, &egui::Context::default()).unwrap();
        assert_eq!(state.message(&doc, feature), Some("Calculating mass properties..."));
    }

    #[test]
    fn a_cached_answer_is_available_while_an_old_request_is_unfinished() {
        let mut doc = Document::new("Body", crate::kernel());
        let feature =
            doc.apply(Command::AddExtrude { params: ExtrudeParams::default(), via_assistant: false }).unwrap().unwrap();
        let request = Request::of(&doc, feature).unwrap();
        let (_held_sender, answer) = mpsc::channel();
        let mut state = State { pending: Some(Pending { request, answer }), ..Default::default() };
        doc.apply(Command::Rename { id: feature, name: "Changed".into() }).unwrap();
        doc.mass_properties(feature).unwrap();
        state.request(&doc, feature, &egui::Context::default()).unwrap();
        assert!(state.pending.is_none());
        assert!(state.message(&doc, feature).is_none());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn a_busy_worker_runs_only_the_latest_queued_request() {
        use std::time::Duration;
        let worker = Worker::new().unwrap();
        let (started, running) = mpsc::channel();
        let (release, held) = mpsc::channel();
        let (finished, answers) = mpsc::channel();
        worker
            .submit(Box::new(move || {
                started.send(()).unwrap();
                held.recv().unwrap();
            }))
            .unwrap();
        running.recv_timeout(Duration::from_secs(5)).unwrap();
        for revision in 0..32 {
            let finished = finished.clone();
            worker
                .submit(Box::new(move || {
                    finished.send(revision).unwrap();
                }))
                .unwrap();
        }
        assert_eq!(answers.try_recv(), Err(mpsc::TryRecvError::Empty));
        release.send(()).unwrap();
        assert_eq!(answers.recv_timeout(Duration::from_secs(5)).unwrap(), 31);
        drop(finished);
        drop(worker);
        assert!(answers.recv_timeout(Duration::from_secs(5)).is_err());
    }

    #[test]
    fn pending_and_refused_properties_cannot_follow_another_revision_or_part() {
        let mut doc = Document::new("Body", crate::kernel());
        let feature =
            doc.apply(Command::AddExtrude { params: ExtrudeParams::default(), via_assistant: false }).unwrap().unwrap();
        let request = Request::of(&doc, feature).unwrap();
        let (sender, answer) = mpsc::channel();
        let mut state = State { pending: Some(Pending { request, answer }), ..Default::default() };
        assert_eq!(state.message(&doc, feature), Some("Calculating mass properties..."));
        assert!(state.poll(&doc).is_none());
        assert!(state.pending.is_some());
        sender.send(Err("measure.unsupported-surface".into())).unwrap();
        assert_eq!(state.poll(&doc).as_deref(), Some("measure.unsupported-surface"));
        assert_eq!(state.message(&doc, feature), Some("measure.unsupported-surface"));

        let (sender, answer) = mpsc::channel();
        state.pending = Some(Pending { request, answer });
        state.failure = None;
        doc.apply(Command::Rename { id: feature, name: "Changed".into() }).unwrap();
        sender.send(Err("obsolete refusal".into())).unwrap();
        assert!(state.poll(&doc).is_none());
        assert!(state.pending.is_none());
        assert!(state.message(&doc, feature).is_none());
        assert!(state.failure.is_none());
        assert!(!request.current(&Document::new("Other part", crate::kernel())));
    }
}
