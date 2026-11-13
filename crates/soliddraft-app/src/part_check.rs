//! Independent part construction, requested from Inspect > Check.

use crate::state::{AppState, NoteKind};
use soliddraft_kernel_api::IndependentCheck;
use std::sync::mpsc;

pub struct Inspection {
    document: soliddraft_document::DocumentId,
    revision: u64,
    pending: Option<mpsc::Receiver<Result<IndependentCheck, String>>>,
    pub result: Option<Result<IndependentCheck, String>>,
}

pub fn start(state: &mut AppState, ctx: &egui::Context) {
    if state.part_check.as_ref().is_some_and(|inspection| inspection.pending.is_some()) {
        return;
    }
    if let Err(error) = state.export_ready() {
        state.note(error, NoteKind::Error);
        return;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ctx;
        state.part_check = Some(Inspection {
            document: state.doc.instance(),
            revision: state.doc.revision(),
            pending: None,
            result: Some(state.doc.model().independent_check()),
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let model = state.doc.model().fork();
        let (send, pending) = mpsc::channel();
        let ctx = ctx.clone();
        match std::thread::Builder::new().name("part-check".into()).spawn(move || {
            let result = model.independent_check();
            let _ = send.send(result);
            ctx.request_repaint();
        }) {
            Ok(_) => {
                state.part_check = Some(Inspection {
                    document: state.doc.instance(),
                    revision: state.doc.revision(),
                    pending: Some(pending),
                    result: None,
                })
            }
            Err(_) => state.note("Could not start the part check. Try again.", NoteKind::Error),
        }
    }
}

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    let Some(inspection) = &mut state.part_check else { return };
    let stale = inspection.document != state.doc.instance() || inspection.revision != state.doc.revision();
    if let Some(pending) = &inspection.pending {
        match pending.try_recv() {
            Ok(result) => {
                inspection.result = Some(result);
                inspection.pending = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                inspection.result = Some(Err("The part check stopped. Close it and try again.".into()));
                inspection.pending = None;
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
        }
    }
    let mut open = true;
    egui::Window::new("Part check").open(&mut open).default_width(540.0).show(ctx, |ui| {
        if stale {
            ui.label("The part changed. This report belongs to an earlier revision.");
            return;
        }
        ui.label("The boundary model and distance field build each solid step independently.");
        ui.label("This checks construction coverage. Surface agreement and print accuracy are not checked here.");
        match &inspection.result {
            None => {
                ui.spinner();
                ui.label("Checking the part...");
            }
            Some(Err(error)) => {
                ui.label(format!("Part could not be confirmed: {error}"));
            }
            Some(Ok(report)) => {
                let bodies = report.steps.iter().filter(|step| !step.reference).count();
                let confirmed = report.steps.iter().filter(|step| step.boundary_built && step.field_built).count();
                let percent = if bodies == 0 { 0.0 } else { 100.0 * confirmed as f64 / bodies as f64 };
                ui.label(format!(
                    "Coverage: {confirmed} of {bodies} solid steps confirmed independently ({percent:.0}%)."
                ));
                egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                    for step in &report.steps {
                        ui.separator();
                        let outcome = if step.reference {
                            "Reference definition"
                        } else if step.boundary_built && step.field_built {
                            "Both models confirmed construction"
                        } else {
                            "Not confirmed by both models"
                        };
                        ui.label(format!("{} ({}) - {outcome}", step.step, step.operation));
                        for reason in &step.unconfirmed {
                            let (model, detail) = reason.split_once(": ").unwrap_or(("Unconfirmed", reason));
                            let message = if detail.contains("compare.envelope-only") {
                                "This field has no independent boundary construction.".to_owned()
                            } else if detail.contains("unsupported") {
                                "Coming soon".to_owned()
                            } else {
                                crate::geometry_message::for_user(detail)
                            };
                            ui.label(format!("{model}: {message}"));
                        }
                    }
                });
            }
        }
    });
    if !open {
        state.part_check = None;
    }
}
