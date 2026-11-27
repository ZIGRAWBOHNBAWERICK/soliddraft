//! Desktop STEP admission uses the part's kernel, but only the UI can change the part.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use soliddraft_document::DocumentId;
use soliddraft_kernel_api::Interrupt;

use super::{
    AppState, CommitError, Document, ImportFormat, ImportStepFeature, NoteKind, Prepared, Preview, finish_in, prepare,
    read_file, report, show_result,
};

pub(crate) struct Opening {
    interrupt: Interrupt,
    origin: (DocumentId, u64),
    name: String,
    answer: Receiver<Finished>,
    #[cfg(test)]
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Opening {
    fn drop(&mut self) {
        self.interrupt.stop();
    }
}

enum Finished {
    Review { path: PathBuf, preview: Result<Preview, String> },
    Admission { preview: Preview, prepared: Result<Prepared, String>, decline: bool },
}

fn origin(doc: &Document) -> (DocumentId, u64) {
    (doc.instance(), doc.revision())
}

fn start(ctx: &egui::Context, state: &mut AppState, name: String, work: impl FnOnce() -> Finished + Send + 'static) {
    state.step_open = None;
    let interrupt = Interrupt::new();
    let stop = interrupt.clone();
    let (sender, answer) = mpsc::channel();
    let repaint = ctx.clone();
    let spawned = std::thread::Builder::new().name("step-open".into()).spawn(move || {
        let result = stop.run(work);
        let _ = sender.send(result);
        repaint.request_repaint();
    });
    match spawned {
        Ok(_worker) => {
            state.step_open = Some(Opening {
                interrupt,
                origin: origin(&state.doc),
                name,
                answer,
                #[cfg(test)]
                worker: Some(_worker),
            });
            ctx.request_repaint();
        }
        Err(error) => state.note(format!("Could not start STEP import: {error}"), NoteKind::Error),
    }
}

pub(crate) fn start_step_open(ctx: &egui::Context, state: &mut AppState, path: PathBuf) {
    state.import_dialog = None;
    let kernel = state.doc.kernel().clone();
    let document = state.doc.model().id();
    let revision = state.doc.revision();
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    start(ctx, state, name, move || {
        let preview = read_file(&path).and_then(|(name, source)| {
            Preview::propose(kernel.as_ref(), document, revision, &name, source, ImportFormat::Step)
        });
        Finished::Review { path, preview }
    });
}

pub(super) fn start_admission(
    ctx: &egui::Context,
    state: &mut AppState,
    preview: Preview,
    feature: ImportStepFeature,
    decline: bool,
) {
    let doc = state.doc.scratch();
    start(ctx, state, preview.feature.source_name.clone(), move || {
        let prepared = prepare(&doc, &preview, feature);
        Finished::Admission { preview, prepared, decline }
    });
}

/// Paint Cancel before taking the answer: a click wins even if the worker just finished.
pub(super) fn window(ctx: &egui::Context, state: &mut AppState) -> bool {
    let Some(pending) = state.step_open.take() else { return false };
    if pending.origin != origin(&state.doc) || state.import_dialog.is_some() {
        return false;
    }
    let mut open = true;
    let mut cancel = false;
    egui::Window::new("Opening STEP").collapsible(false).open(&mut open).show(ctx, |ui| {
        ui.label(&pending.name);
        ui.label("Checking STEP file…");
        cancel = ui.button("Cancel").clicked();
    });
    if cancel || !open {
        ctx.request_repaint();
        return true;
    }
    match pending.answer.try_recv() {
        Err(TryRecvError::Empty) => {
            state.step_open = Some(pending);
            return true;
        }
        Err(TryRecvError::Disconnected) => state.note("STEP import stopped unexpectedly.", NoteKind::Error),
        Ok(Finished::Review { path, preview }) => match preview {
            Ok(preview) => state.import_dialog = Some(preview),
            Err(reason) => report::refused(state, &path, ImportFormat::Step, reason),
        },
        Ok(Finished::Admission { preview, prepared, decline }) => {
            if state.doc.proof_pending() {
                return true;
            }
            let result = prepared.and_then(|prepared| prepared.commit(&mut state.doc)).map_err(CommitError::Refused);
            let committed = finish_in(state, pending.name.clone(), ImportFormat::Step, result);
            show_result(state, preview, decline, committed);
        }
    }
    ctx.request_repaint();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};
    use soliddraft_kernel_api::BodyKind;
    use std::time::{Duration, Instant};

    fn until(mut ready: impl FnMut() -> bool, timeout: Duration, reason: &str) {
        let started = Instant::now();
        while !ready() {
            assert!(started.elapsed() < timeout, "{reason}");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    #[ignore = "requires the ABC census fixture, see docs/exchange.md"]
    fn census_step_opens_as_solid_and_cancel_stops_its_running_admission() {
        let path = PathBuf::from(std::env::var_os("SOLIDDRAFT_CENSUS_STEP").expect("path to the ABC STEP fixture"));
        let source = std::fs::read(&path).unwrap();
        let cold = crate::startup::kernel();
        let (old, work) = soliddraft_kernel_catallax::with_request_allowance(100_000_000, || {
            cold.source_solids(ImportFormat::Step, &source)
        });
        assert!(old.is_err() && work.exhausted, "fixture must exceed the former allowance: {work:?}");

        let state = AppState::new(Document::new("Census import", crate::startup::kernel()));
        let before = state.doc.model().semantic_hash();
        let revision = state.doc.revision();
        let mut app = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_ui_state(|ui, state| super::super::window(ui.ctx(), state), state);
        let ctx = app.ctx.clone();
        start_step_open(&ctx, app.state_mut(), path.clone());
        let timeout = Duration::from_secs(60);
        let second = Duration::from_secs(1);
        app.run_steps(3);
        let pending = app.state_mut().step_open.as_mut().expect("STEP worker pending");
        let worker = pending.worker.take().unwrap();
        until(|| pending.interrupt.request_running(), timeout, "STEP admission did not start");
        let cancel = app.get_by_role_and_label(Role::Button, "Cancel");
        let started = Instant::now();
        cancel.click();
        app.run_steps(3);
        assert!(app.state().step_open.is_none(), "Cancel did not return control");
        assert!(started.elapsed() < second, "Cancel took {:?}", started.elapsed());
        until(|| worker.is_finished(), second.saturating_sub(started.elapsed()), "admission kept running after Cancel");
        worker.join().unwrap();
        eprintln!("Cancelled running STEP admission and worker exited in {:?}", started.elapsed());
        assert!(app.state().import_dialog.is_none());
        assert!(app.state().import_refusals.is_empty());
        assert_eq!(app.state().doc.revision(), revision);
        assert_eq!(app.state().doc.model().semantic_hash(), before);

        let ctx = app.ctx.clone();
        start_step_open(&ctx, app.state_mut(), path);
        until(
            || {
                app.run_steps(1);
                app.state().step_open.is_none()
            },
            timeout,
            "STEP review did not finish",
        );
        app.run_steps(3);
        assert_eq!(app.state().doc.revision(), revision);
        app.get_by_role_and_label(Role::Button, "Accept").click();
        app.run_steps(3);
        until(
            || {
                app.run_steps(1);
                app.state().step_open.is_none()
            },
            timeout,
            "STEP admission did not finish",
        );
        let state = app.state();
        assert!(state.import_refusals.is_empty(), "{:?}", state.import_refusals);
        let body = state.doc.display_body().expect("the census STEP must open as a solid");
        let accuracy = state.doc.kernel().body_accuracy(body).unwrap();
        assert_eq!(state.doc.kernel().body_kind(body).unwrap(), BodyKind::Solid);
        eprintln!(
            "Opened census STEP as a solid ({accuracy:?}) after exceeding {} work units on the old cap",
            work.limit
        );
    }
}
