//! Review print evidence before writing a mesh. Coarser accuracy requires a new choice.

use crate::state::{AppState, NoteKind};
use soliddraft_exchange::{Export, Format, Report};
use soliddraft_model::FeatureKind;
use std::{path::PathBuf, sync::mpsc};

pub struct Dialog {
    document: soliddraft_document::DocumentId,
    revision: u64,
    path: PathBuf,
    format: Format,
    pub requested_mm: f64,
    pub accuracy_mm: f64,
    field_route: bool,
    pending: Option<mpsc::Receiver<Result<Export, String>>>,
    pub result: Option<Result<Export, String>>,
    write_error: Option<String>,
}

pub fn is_mesh(format: Format) -> bool {
    matches!(format, Format::Stl | Format::ThreeMf | Format::Obj)
}

pub fn start(state: &mut AppState, format: Format, path: PathBuf, ctx: &egui::Context) {
    if state.print_check.as_ref().is_some_and(|dialog| dialog.pending.is_some()) {
        return;
    }
    if let Err(error) = state.export_ready() {
        state.note(error, NoteKind::Error);
        return;
    }
    let model = state.doc.model();
    let requested_mm = match model.active_field().map(|feature| &feature.kind) {
        Some(FeatureKind::Lattice(field)) => field.accuracy_mm,
        Some(FeatureKind::Gyroid(field)) => field.accuracy_mm,
        _ => model.settings.export.chord_height,
    };
    let mut dialog = Dialog {
        document: state.doc.instance(),
        revision: state.doc.revision(),
        path,
        format,
        requested_mm,
        accuracy_mm: requested_mm,
        field_route: model.active_field().is_some(),
        pending: None,
        result: None,
        write_error: None,
    };
    dialog.request(state, None, ctx);
    state.print_check = Some(dialog);
}

impl Dialog {
    fn request(&mut self, state: &AppState, accuracy: Option<f64>, ctx: &egui::Context) {
        self.result = None;
        self.write_error = None;
        if let Some(mm) = accuracy {
            self.accuracy_mm = mm;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            self.result = Some(state.export_part_at(self.format, accuracy));
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let model = state.doc.model().fork();
            let format = self.format;
            let (send, pending) = mpsc::channel();
            let ctx = ctx.clone();
            match std::thread::Builder::new().name("print-check".into()).spawn(move || {
                let result = model.export_at(format, accuracy);
                let _ = send.send(result);
                ctx.request_repaint();
            }) {
                Ok(_) => self.pending = Some(pending),
                Err(_) => self.result = Some(Err("Could not start the print check. Try again.".into())),
            }
        }
    }
}

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut dialog) = state.print_check.take() else { return };
    let stale = dialog.document != state.doc.instance() || dialog.revision != state.doc.revision();
    if let Some(pending) = &dialog.pending {
        match pending.try_recv() {
            Ok(result) => {
                dialog.result = Some(result);
                dialog.pending = None;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                dialog.result = Some(Err("The print check stopped. Close it and try again.".into()));
                dialog.pending = None;
            }
            Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
        }
    }
    let mut open = true;
    let mut save = false;
    let mut coarser = None;
    let mut cancel = false;
    egui::Window::new("Print check").open(&mut open).default_width(540.0).show(ctx, |ui| {
        ui.label(format!("{} export to {}", dialog.format.name(), dialog.path.display()));
        if stale {
            ui.label("The part changed. Close this report and export again.");
            return;
        }
        ui.label(format!("Print route: {}", if dialog.field_route { "distance field" } else { "boundary model" }));
        ui.label(format!("Requested accuracy: {} mm", dialog.requested_mm));
        if dialog.accuracy_mm != dialog.requested_mm {
            ui.label(format!("Coarser choice: {} mm. This allows more deviation from the part.", dialog.accuracy_mm));
        }
        match &dialog.result {
            None => {
                ui.spinner();
                ui.label("Checking print accuracy...");
            }
            Some(Err(error)) => {
                ui.label(format!("Export refused: {}", refusal_message(error)));
                ui.label("Print accuracy and final qualification remain unconfirmed. No file has been written.");
                // Only accuracy and refinement refusals may offer a coarser request.
                if can_coarsen(error) {
                    let mm = dialog.accuracy_mm * 2.0;
                    if mm.is_finite()
                        && mm > dialog.accuracy_mm
                        && ui.button(format!("Try coarser accuracy ({mm} mm)")).clicked()
                    {
                        coarser = Some(mm);
                    }
                    ui.label(
                        "A coarser request gets its own check. No file is written until you accept a confirmed result.",
                    );
                }
            }
            Some(Ok(output)) => {
                ui.label(output.report.summary());
                match &output.report {
                    Report::Mesh(report) => {
                        if let Some(check) = &report.print_check {
                            ui.label(format!(
                                "Certified accuracy: {} mm",
                                soliddraft_exchange::bound_mm(check.certified_accuracy_mm)
                            ));
                            ui.label(format!(
                                "Model: {} mm; meshing: {} mm; file rounding: {} mm",
                                soliddraft_exchange::bound_mm(check.model_bound_mm),
                                soliddraft_exchange::bound_mm(check.tessellation_bound_mm),
                                soliddraft_exchange::bound_mm(check.rounding_bound_mm)
                            ));
                            for stage in &check.stages {
                                ui.label(stage);
                            }
                        } else {
                            ui.label("A print-route certificate is unavailable for this model.");
                        }
                    }
                    Report::FieldMesh(report) => {
                        ui.label(format!(
                            "Certified accuracy: {} mm",
                            soliddraft_exchange::bound_mm(report.deviation_bound_mm)
                        ));
                        ui.label(format!("Minimum detail kept: {} mm", report.minimum_detail_mm));
                        ui.label(format!(
                            "Closed surface: {}. Self-intersection excluded: {}. Connected: {}.",
                            report.closed, report.non_self_intersecting, report.connected
                        ));
                        ui.label("Field boundary coverage and file rounding certified");
                    }
                    _ => {}
                }
                if ui.button("Save checked export").clicked() {
                    save = true;
                }
            }
        }
        if let Some(error) = &dialog.write_error {
            ui.label(error);
        }
        if ui.button("Cancel").clicked() {
            cancel = true;
        }
    });
    if let Some(mm) = coarser {
        dialog.request(state, Some(mm), ctx);
    }
    if save {
        let result = state.export_ready().and_then(|()| {
            let output =
                dialog.result.as_ref().and_then(|result| result.as_ref().ok()).ok_or("No checked export to save")?;
            crate::platform::write_file(&dialog.path, &output.bytes).map(|()| output.report.summary())
        });
        match result {
            Ok(summary) => {
                state.note(
                    format!(
                        "Exported {} at {} mm requested accuracy; {summary}",
                        dialog.path.display(),
                        dialog.accuracy_mm
                    ),
                    NoteKind::Ok,
                );
                return;
            }
            Err(error) => dialog.write_error = Some(error),
        }
    }
    if open && !cancel {
        state.print_check = Some(dialog);
    }
}

pub(crate) fn can_coarsen(error: &str) -> bool {
    [
        "print.brep-accuracy",
        "print.brep-encoding-topology",
        "numbers.operation-budget",
        "mesh.precision-or-budget",
        "field.print-unresolved",
        "field.print-budget",
        "field.print-grid",
        "field.print-vertices",
    ]
    .iter()
    .any(|code| error.contains(code))
}

fn refusal_message(error: &str) -> String {
    if can_coarsen(error) {
        "The requested accuracy could not be certified within the calculation limits.".into()
    } else if error.contains("print.brep-topology") || error.contains("field.print-topology") {
        "A closed, sound print surface could not be confirmed.".into()
    } else if error.contains("orientation") {
        "The print surface's outward orientation could not be confirmed.".into()
    } else if error.contains("field.print-disconnected") {
        "The print output has disconnected pieces. Edit the part before exporting.".into()
    } else if error.contains("unsupported") {
        "Coming soon".into()
    } else {
        crate::geometry_message::for_user(error)
    }
}
