//! Read-only revision comparison and an editor for separate reviewer markup.

use std::io;
use std::path::Path;

use soliddraft_kernel_api::pmi::length_text;

use crate::review::{ItemChange, ItemState, ReviewerMarkup, SavedRevision};

/// The drawing host supplies the file picker and reports file errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkupFileAction {
    Open,
    SaveAs,
}

#[derive(Debug)]
struct ChangeRow {
    id: String,
    before: Option<String>,
    after: Option<String>,
    delta: Option<String>,
    baseline_only_faces: usize,
    candidate_only_faces: usize,
}

impl From<ItemChange<'_>> for ChangeRow {
    fn from(change: ItemChange<'_>) -> Self {
        fn callout(state: &ItemState) -> String {
            state.item.callout(state.nominal_mm.as_deref().unwrap_or_default())
        }

        let faces = change.unmatched_faces();
        let delta = change.dimension_delta_mm().map(|values| {
            let mut delta = String::new();
            for (index, value) in values.into_iter().enumerate() {
                if index != 0 {
                    delta.push_str(" / ");
                }
                let length = length_text(value);
                if value > 0.0 && length != "0" && length != "?" {
                    delta.push('+');
                }
                delta.push_str(&length);
            }
            delta
        });
        Self {
            id: change.id.to_owned(),
            before: change.before.map(callout),
            after: change.after.map(callout),
            delta,
            baseline_only_faces: faces.baseline_only.len(),
            candidate_only_faces: faces.candidate_only.len(),
        }
    }
}

impl ChangeRow {
    fn status(&self) -> &'static str {
        match (&self.before, &self.after) {
            (None, _) => "Added",
            (_, None) => "Removed",
            _ => "Changed",
        }
    }
}

/// Panel state for two saved revisions. Neither one is opened for writing.
#[derive(Debug)]
pub struct RevisionReview {
    candidate: SavedRevision,
    changes: Vec<ChangeRow>,
    issues: Vec<String>,
    selected: Option<usize>,
    draft: Option<ReviewerMarkup>,
}

impl RevisionReview {
    pub fn new(baseline: SavedRevision, candidate: SavedRevision) -> Self {
        let changes = baseline.compare(&candidate).changes.into_iter().map(ChangeRow::from).collect();
        let issues = [("Baseline", &baseline), ("Candidate", &candidate)]
            .into_iter()
            .flat_map(|(side, revision)| revision.issues().map(move |(id, reason)| format!("{side} · {id}: {reason}")))
            .collect();
        Self { candidate, changes, issues, selected: None, draft: None }
    }

    pub fn draft(&self) -> Option<&ReviewerMarkup> {
        self.draft.as_ref()
    }

    /// Reopens a sidecar only if it still applies to this exact saved candidate.
    /// A failed open leaves the current selection and draft intact.
    pub fn open_markup(&mut self, path: &Path) -> io::Result<()> {
        let markup = ReviewerMarkup::open(path)?;
        if !markup.applies_to(&self.candidate) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Markup does not match this candidate revision or item.",
            ));
        }
        self.selected =
            markup.item_id.as_deref().and_then(|id| self.changes.binary_search_by(|row| row.id.as_str().cmp(id)).ok());
        self.draft = Some(markup);
        Ok(())
    }

    /// Writes a new sidecar, never replacing a source part or an existing sidecar.
    pub fn save_markup(&self, path: &Path) -> io::Result<()> {
        self.nonempty_draft()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Write a markup comment before saving."))?
            .save_new(path)
    }

    fn nonempty_draft(&self) -> Option<&ReviewerMarkup> {
        self.draft.as_ref().filter(|draft| !draft.text.trim().is_empty())
    }

    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<MarkupFileAction> {
        ui.heading("Revision review");
        if self.changes.is_empty() {
            ui.label("No authored PMI changes.");
        } else {
            egui::ScrollArea::vertical().id_salt("review_changes").max_height(144.0).show(ui, |ui| {
                for (index, row) in self.changes.iter().enumerate() {
                    if ui
                        .selectable_label(self.selected == Some(index), format!("{} · {}", row.status(), row.id))
                        .clicked()
                    {
                        self.selected = Some(index);
                    }
                }
            });
        }
        if !self.issues.is_empty() {
            ui.collapsing(format!("PMI issues ({})", self.issues.len()), |ui| {
                for issue in &self.issues {
                    ui.label(issue);
                }
            });
        }
        if let Some(row) = self.selected.map(|index| &self.changes[index]) {
            ui.separator();
            ui.columns(2, |columns| {
                show_side(&mut columns[0], "Baseline", row.before.as_deref(), row.baseline_only_faces);
                show_side(&mut columns[1], "Candidate", row.after.as_deref(), row.candidate_only_faces);
            });
            if let Some(delta) = &row.delta {
                ui.label(format!("Candidate − baseline: {delta} mm"));
            }
            if row.after.is_some() {
                if ui.button("Attach markup to selected item").clicked() {
                    let text = self.draft.take().map_or_else(String::new, |draft| draft.text);
                    self.draft = self.candidate.markup(Some(&row.id), text);
                }
            } else {
                ui.label("Removed items cannot receive candidate markup.");
            }
        }
        self.show_editor(ui)
    }

    fn show_editor(&mut self, ui: &mut egui::Ui) -> Option<MarkupFileAction> {
        ui.separator();
        ui.heading("Reviewer markup");
        if let Some(draft) = &mut self.draft {
            ui.label(format!("Attached to: {}", draft.item_id.as_deref().unwrap_or("candidate revision")));
            let label = ui.label("Comment");
            ui.add(egui::TextEdit::multiline(&mut draft.text).desired_rows(3).desired_width(f32::INFINITY))
                .labelled_by(label.id);
        } else {
            ui.label("Select a candidate item to attach a comment.");
        }
        ui.horizontal(|ui| {
            if ui.button("Open markup…").clicked() {
                return Some(MarkupFileAction::Open);
            }
            if ui.add_enabled(self.nonempty_draft().is_some(), egui::Button::new("Save markup as…")).clicked() {
                return Some(MarkupFileAction::SaveAs);
            }
            None
        })
        .inner
    }
}

fn show_side(ui: &mut egui::Ui, title: &str, callout: Option<&str>, unmatched_faces: usize) {
    ui.strong(title);
    ui.label(callout.unwrap_or("Item not present"));
    if unmatched_faces != 0 {
        ui.label(format!("Faces only on this side: {unmatched_faces}"));
    }
}
