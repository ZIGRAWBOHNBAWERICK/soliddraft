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
}

impl Tier {
    pub fn label(self) -> &'static str {
        match self {
            Tier::Exact => "Exact solid",
            Tier::Repaired => "Solid, repaired",
            Tier::Surface => "Surface body",
            Tier::Refused => "Refused",
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
    state.import_refusals.retain(|r| r.file != file || r.format != format);
    state.import_refusals.push(Refusal { file, format, reason: reason.clone() });
    state.note(reason, NoteKind::Error);
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
    let Some(feature) = doc.feature(id) else { return (Tier::Refused, "The import is no longer in the part.".into()) };
    if let FeatureStatus::Error(error) = &feature.status {
        return (Tier::Refused, error.clone());
    }
    let surface =
        feature.body.is_some_and(|body| doc.model().kernel().body_kind(body).is_ok_and(|kind| kind == BodyKind::Sheet));
    if surface {
        return (Tier::Surface, "The file holds open faces, not a closed solid. No volume or mass.".into());
    }
    let Some(record) = doc.model().import_record(model_id(id)) else {
        return (Tier::Exact, "Closed as it is.".into());
    };
    let mut why: Vec<String> = Vec::new();
    for change in &record.changes {
        let kind = super::kind_name(&change.kind);
        why.push(match change.approval {
            Approval::Declined => format!("Declined {kind}; the solid stays open"),
            Approval::Automatic => format!("{kind} closed automatically within {} mm", change.tolerance_mm),
            Approval::User => format!(
                "{kind} closed within {} mm, approved by {}",
                change.tolerance_mm,
                change.approver.as_deref().unwrap_or("the reviewer")
            ),
        });
    }
    let refitted = record.face_tolerances.len() + record.source_incidence.len();
    if refitted > 0 {
        why.push(format!(
            "{refitted} {} repaired within a stated deviation from the source",
            if refitted == 1 { "face" } else { "faces" }
        ));
    }
    let repaired = record.changes.iter().any(|c| c.approval != Approval::Declined) || refitted > 0;
    if why.is_empty() {
        (Tier::Exact, "Closed as it is.".into())
    } else {
        (if repaired { Tier::Repaired } else { Tier::Exact }, capitalised(&why.join("; ")) + ".")
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
