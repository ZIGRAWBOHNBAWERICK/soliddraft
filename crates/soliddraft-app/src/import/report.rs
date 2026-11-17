//! The import report: every imported file of the part, what it opened as and why.
//!
//! It shows only when asked for, from File > Import report or from the status bar's note
//! after an import that did not open as an exact solid. It never shows while the tree is
//! browsed.

use soliddraft_kernel_api::{Approval, BodyKind, ImportFormat};

use crate::document::{Document, FeatureId, FeatureKind, FeatureStatus, Selection, model_id};
use crate::state::{AppAction, AppState, NoteKind};

/// What an imported file opened as, from best to worst.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Its faces closed as they are. Nothing changed.
    Exact,
    /// It closed after stated repairs, each with its bound.
    Repaired,
    /// Open faces with no volume or mass.
    Surface,
    /// It did not open.
    Refused,
    /// One of several bodies of a file whose record lists repairs it does not attribute
    /// to one body, and no face repair is attributed to this body.
    FileRepaired,
    /// It is suppressed, rolled back or not built yet, so it has no reliable tier.
    NotEvaluated,
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::Exact => "Exact solid",
            Tier::Repaired => "Solid, repaired",
            Tier::Surface => "Surface body",
            Tier::Refused => "Refused",
            Tier::FileRepaired => "Solid, file repaired",
            Tier::NotEvaluated => "Not evaluated",
        }
    }
}

/// A file the importer refused this session. It has no feature, so the report keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub file: String,
    pub format: ImportFormat,
    pub reason: String,
}

/// Records a file the importer refused, keeps it for the report and says why.
pub fn refused(state: &mut AppState, path: &std::path::Path, format: ImportFormat, reason: String) {
    let file = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
    keep_refusal(state, file, format, reason.clone());
    state.note(reason, NoteKind::Error);
}

/// Keeps `file`'s latest refusal for the report.
pub fn keep_refusal(state: &mut AppState, file: String, format: ImportFormat, reason: String) {
    state.import_refusals.retain(|r| r.file != file || r.format != format);
    state.import_refusals.push(Refusal { file, format, reason });
}

/// One row of the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub file: String,
    pub format: ImportFormat,
    pub tier: Tier,
    /// Why it opened as it did, in words.
    pub why: String,
    /// A check the import did not run, in words.
    pub not_checked: Option<String>,
    /// The import feature, absent for a refused file.
    pub feature: Option<FeatureId>,
}

/// The report's rows: each import of `doc` in tree order, then each refused file.
pub fn rows(doc: &Document, refused: &[Refusal]) -> Vec<Row> {
    let mut rows: Vec<Row> = doc
        .features
        .iter()
        .filter_map(|f| match &f.kind {
            FeatureKind::ImportStep(import) => Some(row(doc, f.id, import.source_name.clone(), import.format)),
            _ => None,
        })
        .collect();
    rows.extend(refused.iter().map(|r| Row {
        file: r.file.clone(),
        format: r.format,
        tier: Tier::Refused,
        why: r.reason.clone(),
        not_checked: None,
        feature: None,
    }));
    rows
}

/// The row of import feature `id`.
pub fn row(doc: &Document, id: FeatureId, file: String, format: ImportFormat) -> Row {
    let (tier, why) = opened_as(doc, id);
    let not_checked = doc.model().import_check_note(model_id(id));
    Row { file, format, tier, why, not_checked, feature: Some(id) }
}

/// What import feature `id` opened as, and why.
fn opened_as(doc: &Document, id: FeatureId) -> (Tier, String) {
    let unbuilt = |why: &str| (Tier::NotEvaluated, why.to_owned());
    let Some(feature) = doc.feature(id) else { return unbuilt("The import is no longer in the part.") };
    match &feature.status {
        FeatureStatus::Error(error) => return (Tier::Refused, error.clone()),
        FeatureStatus::Inactive if feature.suppressed => return unbuilt("Suppressed, so it is not built."),
        FeatureStatus::Inactive => return unbuilt("Past the rollback bar, so it is not built."),
        FeatureStatus::Pending => return unbuilt("Its proof is still pending."),
        FeatureStatus::Ok | FeatureStatus::NeedsRepair(_) | FeatureStatus::AnsweredWithinBound(_) => {}
    }
    let Some(body) = feature.body else { return unbuilt("It has no body.") };
    match doc.model().kernel().body_kind(body) {
        Err(_) => return unbuilt("Its body's kind could not be read."),
        Ok(BodyKind::Sheet) => {
            return (Tier::Surface, "The file holds open faces, not a closed solid. No volume or mass.".into());
        }
        Ok(_) => {}
    }
    let Some(record) = doc.model().import_record(model_id(id)) else {
        return (Tier::Exact, "Closed as it is.".into());
    };
    // A file of several solids shares one record. A repaired face names its solid, so it
    // belongs to that body; a gap change names only source entities, so it stays the
    // file's and is never spread over every body the file gave.
    let solid = match &feature.kind {
        FeatureKind::ImportStep(import) => import.source_solid.as_deref(),
        _ => None,
    };
    let own = |of: &str| solid.is_none_or(|solid| solid == of);
    let faces = record.face_tolerances.iter().filter(|f| own(&f.solid)).count()
        + record.source_incidence.iter().filter(|f| own(&f.solid)).count();
    let changes: Vec<String> = record.changes.iter().map(change_text).collect();
    let file_repaired = record.changes.iter().any(|c| c.approval != Approval::Declined);
    let mut why: Vec<String> = Vec::new();
    if solid.is_none() {
        why.extend(changes);
    }
    if faces > 0 {
        why.push(format!(
            "{faces} {} repaired within a stated deviation from the source",
            if faces == 1 { "face" } else { "faces" }
        ));
    }
    let mut text = capitalised(&why.join("; "));
    if !text.is_empty() {
        text.push('.');
    }
    if solid.is_some() && !record.changes.is_empty() {
        let file = record.changes.iter().map(change_text).collect::<Vec<_>>().join("; ");
        if faces == 0 {
            text.push_str("No face repairs are attributed to this body.");
        }
        text.push_str(&format!(" The file's record also lists repairs it does not attribute to one body: {file}."));
    }
    let own_repaired = faces > 0 || (solid.is_none() && file_repaired);
    let tier = if own_repaired {
        Tier::Repaired
    } else if solid.is_some() && file_repaired {
        Tier::FileRepaired
    } else {
        Tier::Exact
    };
    if text.is_empty() { (tier, "Closed as it is.".into()) } else { (tier, text.trim_start().to_owned()) }
}

/// A gap change in words.
fn change_text(change: &soliddraft_kernel_api::RepairChange) -> String {
    let kind = super::kind_name(&change.kind);
    match change.approval {
        Approval::Declined => format!("declined {kind}; the solid stays open"),
        Approval::Automatic => format!("{kind} closed automatically within {} mm", change.tolerance_mm),
        Approval::User => format!(
            "{kind} closed within {} mm, approved by {}",
            change.tolerance_mm,
            change.approver.as_deref().unwrap_or("the reviewer")
        ),
    }
}

fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// The status bar's link asks for the report through this flag, read by [`window`].
fn requested_id() -> egui::Id {
    egui::Id::new("import report requested")
}

/// Asks for the report to open on the next frame.
pub fn request(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(requested_id(), true));
}

/// A fresh note about an import, shown whole above the status bar with its link to the
/// report: the status bar has room for only the start of it.
pub fn notice(ctx: &egui::Context, state: &AppState) {
    let Some(note) = state.note.as_ref().filter(|note| note.report) else { return };
    if note.at.elapsed() >= crate::command_line::NOTE_ON_BAR || state.import_report {
        return;
    }
    egui::Area::new(egui::Id::new("import notice"))
        .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -36.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(420.0);
                let color = if note.kind == crate::state::NoteKind::Error {
                    crate::theme::error()
                } else {
                    ui.visuals().text_color()
                };
                ui.add(egui::Label::new(egui::RichText::new(&note.text).color(color)).wrap());
                if ui.link("Open the import report").clicked() {
                    request(ui.ctx());
                }
            });
        });
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
}

/// The Import report window, while it is open.
pub fn window(ctx: &egui::Context, state: &mut AppState) {
    if ctx.data_mut(|d| d.remove_temp::<bool>(requested_id())).unwrap_or(false) {
        state.import_report = true;
    }
    if !state.import_report {
        return;
    }
    let rows = rows(&state.doc, &state.import_refusals);
    let mut open = true;
    let mut action = None;
    egui::Window::new("Import report").collapsible(false).open(&mut open).show(ctx, |ui| {
        if rows.is_empty() {
            ui.label("No file has been imported into this part.");
            return;
        }
        egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
            egui::Grid::new("import report").num_columns(4).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                ui.strong("File");
                ui.strong("Opened as");
                ui.strong("Why");
                ui.label("");
                ui.end_row();
                for row in &rows {
                    ui.label(&row.file).on_hover_text(row.format.label());
                    ui.label(row.tier.label());
                    ui.vertical(|ui| {
                        ui.add(egui::Label::new(&row.why).wrap());
                        if let Some(note) = &row.not_checked {
                            ui.add(egui::Label::new(egui::RichText::new(format!("Not checked: {note}")).weak()).wrap());
                        }
                    });
                    ui.horizontal(|ui| {
                        if let Some(feature) = row.feature {
                            if ui.button("Show in tree").clicked() {
                                action = Some(Err(feature));
                            }
                            if !cfg!(target_arch = "wasm32") && ui.button("Replace file…").clicked() {
                                action = Some(Ok(feature));
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        });
    });
    match action {
        Some(Err(feature)) => state.push(AppAction::Select(Some(Selection::Feature(feature)))),
        Some(Ok(feature)) => state.push(AppAction::ReplaceImport { feature, path: None }),
        None => {}
    }
    state.import_report = open;
}
