//! Requested body properties, computed away from the desktop event loop.

use crate::document::{Document, FeatureId};
use soliddraft_kernel_api::BodyId;
use std::sync::mpsc;

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

/// One outstanding request per window. Repeated clicks share its work.
#[derive(Default)]
pub(crate) struct State {
    pending: Option<Pending>,
    failure: Option<(Request, String)>,
}

impl State {
    /// Starts a measurement without waiting for it on the interface thread.
    pub(crate) fn request(&mut self, doc: &Document, feature: FeatureId, ctx: &egui::Context) -> Result<(), String> {
        let request = Request::of(doc, feature).ok_or("Select a solid feature with measurable properties")?;
        if let Some(pending) = &self.pending {
            return if pending.request == request { Ok(()) } else { Err("A measurement is already running".into()) };
        }
        self.failure = None;
        if doc.kernel().cached_mass_properties(request.body).is_some() {
            return Ok(());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // The fork retains the immutable body even if the user opens another part.
            let model = doc.model().fork();
            let ctx = ctx.clone();
            let (sender, answer) = mpsc::channel();
            std::thread::Builder::new()
                .name("mass-properties".into())
                .spawn(move || {
                    let result = model.kernel().mass_properties(request.body).map(|_| ()).map_err(|e| e.to_string());
                    let _ = sender.send(result);
                    ctx.request_repaint();
                })
                .map_err(|_| "Could not start the measurement. Try again.")?;
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
    pub(crate) fn poll(&mut self, doc: &Document) {
        let Some(pending) = &self.pending else { return };
        let result = match pending.answer.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("The measurement stopped. Try again.".into()),
        };
        let request = pending.request;
        self.pending = None;
        if request.current(doc) {
            self.failure = result.err().map(|error| (request, error));
        }
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
    fn pending_and_refused_properties_cannot_follow_another_revision_or_part() {
        let mut doc = Document::new("Body", crate::kernel());
        let feature =
            doc.apply(Command::AddExtrude { params: ExtrudeParams::default(), via_assistant: false }).unwrap().unwrap();
        let request = Request::of(&doc, feature).unwrap();
        let (sender, answer) = mpsc::channel();
        let mut state = State { pending: Some(Pending { request, answer }), failure: None };
        assert_eq!(state.message(&doc, feature), Some("Calculating mass properties..."));
        state.poll(&doc);
        assert!(state.pending.is_some());
        sender.send(Err("measure.unsupported-surface".into())).unwrap();
        state.poll(&doc);
        assert_eq!(state.message(&doc, feature), Some("measure.unsupported-surface"));

        let (sender, answer) = mpsc::channel();
        state.pending = Some(Pending { request, answer });
        state.failure = None;
        doc.apply(Command::Rename { id: feature, name: "Changed".into() }).unwrap();
        sender.send(Err("obsolete refusal".into())).unwrap();
        state.poll(&doc);
        assert!(state.pending.is_none());
        assert!(state.message(&doc, feature).is_none());
        assert!(state.failure.is_none());
        assert!(!request.current(&Document::new("Other part", crate::kernel())));
    }
}
