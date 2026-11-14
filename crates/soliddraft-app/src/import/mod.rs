//! Review immutable STEP or IGES source bytes before committing the imported feature.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};
use soliddraft_assistant::tools::schema::{array, object, optional, string};
use soliddraft_kernel_api::{
    Approval, Disposition, ImportFormat, ImportOptions, KernelError, Proposal, ProposalSet, RepairChange, RepairRecord,
};
use soliddraft_model::{ImportStepFeature, sha256_hex};

use crate::document::{Command, Document, FeatureId, FeatureKind};
use crate::state::{AppState, NoteKind};

pub fn params() -> Vec<(&'static str, Value)> {
    let mut params = preview_params();
    params.push((
        "preview_token",
        string("The token returned by preview_import for this source and its repair questions."),
    ));
    params.push(("decline", optional(array(string("Gap id to decline, such as g0."), "Repairs to leave open."))));
    params.push((
        "accept",
        optional(array(
            object(&[
                ("id", string("Gap id to repair, such as g0.")),
                ("tolerance_mm", string("The allowed movement in millimetres, a plain decimal of at least the gap.")),
                ("approver", string("The person who approved this repair.")),
            ]),
            "Repairs that need approval and that the user approved.",
        )),
    ));
    params
}

pub fn preview_params() -> Vec<(&'static str, Value)> {
    vec![
        ("path", string("A STEP, IGES, STL or OBJ source file path.")),
        ("feature", optional(string("The imported feature to replace. Omit for a new import."))),
        (
            "format",
            optional(string(
                "Source format: step (default), iges or mesh. Mesh coordinates are millimetres. Use the same format for preview and import.",
            )),
        ),
    ]
}

pub fn source_format(input: &Value) -> Result<ImportFormat, String> {
    match input.get("format") {
        None => Ok(ImportFormat::Step),
        Some(Value::String(format)) if format == "step" => Ok(ImportFormat::Step),
        Some(Value::String(format)) if format == "iges" => Ok(ImportFormat::Iges),
        Some(Value::String(format)) if format == "mesh" => Ok(ImportFormat::Mesh),
        _ => Err("Choose step, iges or mesh as the import format.".into()),
    }
}

pub fn source_path(input: &Value) -> Result<std::path::PathBuf, String> {
    let path = input["path"].as_str().filter(|p| !p.trim().is_empty()).ok_or("missing import source path")?;
    Ok(path.into())
}

/// One repair the reviewer approved.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approved {
    /// The gap, such as `g0`.
    pub id: String,
    /// The allowed movement in millimetres, as a plain decimal.
    pub tolerance_mm: String,
    /// Who approved it.
    pub approver: String,
}

/// The reviewer's answers to the proposed repairs.
#[derive(Clone, Debug, Default)]
pub struct Review {
    /// Gaps to leave open. A declined gap leaves its solid unbuilt.
    pub decline: Vec<String>,
    /// Asked repairs to apply.
    pub accept: Vec<Approved>,
}

#[derive(Clone, Debug)]
pub struct Preview {
    feature: ImportStepFeature,
    replacement: Option<FeatureId>,
    proposals: ProposalSet,
    revision: u64,
    document: soliddraft_document::DocumentId,
    /// The dialog's allowed movement of each asked repair, by gap id.
    allowed: BTreeMap<String, String>,
    /// The dialog's approver.
    approver: String,
}

/// An import failure as the interface says it.
fn plain(error: KernelError) -> String {
    match error {
        KernelError::Unsupported(_) => "Coming soon".into(),
        KernelError::InvalidInput(message) => message,
        other => other.to_string(),
    }
}

/// Repairs the kernel can certify after an explicit tolerance approval.
fn approvable(proposal: &Proposal) -> bool {
    matches!(proposal.kind.as_str(), "seam_close" | "planar_incidence")
}

fn gap_name(proposal: &Proposal) -> String {
    let number = proposal.id.strip_prefix('g').and_then(|n| n.parse::<usize>().ok()).unwrap_or(0) + 1;
    format!("Gap {number}")
}

fn kind_name(kind: &str) -> &'static str {
    match kind {
        "seam_close" => "seam",
        "planar_incidence" => "source mismatch",
        "body_gap" => "gap between bodies",
        _ => "opening",
    }
}

/// True for a plain decimal that covers the measured gap. The kernel checks it exactly.
fn covers_gap(allowed: &str, proposal: &Proposal) -> bool {
    let reaches = |gap: f64| allowed.parse::<f64>().is_ok_and(|value| value >= gap);
    plain_mm(allowed) && proposal.distance_max_mm.parse().is_ok_and(reaches)
}

/// A plain decimal number of millimetres, such as `0.03`, as an approval is saved.
fn plain_mm(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit() || b == b'.') && text.parse::<f64>().is_ok()
}

impl Preview {
    pub fn read(doc: &Document, path: &Path) -> Result<Self, String> {
        Self::read_source(doc, path, ImportFormat::Step)
    }

    pub fn read_source(doc: &Document, path: &Path, format: ImportFormat) -> Result<Self, String> {
        let source = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::from_source(doc, &path.file_name().unwrap_or_default().to_string_lossy(), source, format)
    }

    pub fn from_bytes(doc: &Document, name: &str, source: Vec<u8>) -> Result<Self, String> {
        Self::from_source(doc, name, source, ImportFormat::Step)
    }

    pub fn from_source(doc: &Document, name: &str, source: Vec<u8>, format: ImportFormat) -> Result<Self, String> {
        let mut feature = ImportStepFeature::new(name, source);
        feature.format = format;
        let proposals =
            doc.kernel().propose_import_source(format, &feature.source, &ImportOptions::default()).map_err(plain)?;
        if proposals.source_sha256 != feature.sha256 {
            return Err("the repair preview does not match the source file".into());
        }
        let asked = proposals.proposals.iter().filter(|p| p.disposition == Disposition::Ask);
        let allowed = asked.map(|p| (p.id.clone(), p.tolerance_mm.clone())).collect();
        let approver = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
        Ok(Self {
            feature,
            replacement: None,
            proposals,
            revision: doc.revision(),
            document: doc.model().id(),
            allowed,
            approver,
        })
    }

    /// Review a new source for an existing import, discarding the previous source's decisions.
    pub fn replacing(mut self, doc: &Document, id: FeatureId) -> Result<Self, String> {
        let Some(FeatureKind::ImportStep(old)) = doc.feature(id).map(|f| &f.kind) else {
            return Err("Choose an imported feature to replace.".into());
        };
        if old.source_solid.is_some() {
            if old.sha256 != self.feature.sha256 || old.format != self.feature.format {
                return Err("This body selects a solid from the original file. Import a changed file as new bodies instead of replacing it.".into());
            }
            self.feature.source_solid = old.source_solid.clone();
        }
        if old.sha256 == self.feature.sha256 && old.format == self.feature.format {
            self.feature.source_bound_names = old.source_bound_names;
        }
        self.replacement = Some(id);
        Ok(self)
    }

    pub fn token(&self) -> String {
        let reviewed = serde_json::to_vec(&(
            "soliddraft-import-review/1",
            &self.feature.source_name,
            &self.feature.sha256,
            &self.proposals,
        ))
        .expect("import previews serialize");
        if self.replacement.is_some() || self.feature.format != ImportFormat::Step {
            return sha256_hex(
                &serde_json::to_vec(&(&self.feature.format, self.replacement, &reviewed)).expect("review serializes"),
            );
        }
        sha256_hex(&reviewed)
    }

    pub fn summary(&self) -> Value {
        json!({"source_name": self.feature.source_name, "source_sha256": self.feature.sha256,
               "proposals": self.proposals.proposals, "replacement": self.replacement, "token": self.token()})
    }
}

pub fn commit(doc: &mut Document, preview: &Preview, review: &Review) -> Result<FeatureId, String> {
    if doc.revision() != preview.revision || doc.model().id() != preview.document || doc.proof_pending() {
        return Err("the part changed during the preview; review the import again".into());
    }
    let find = |id: &String| {
        let proposals = &preview.proposals.proposals;
        proposals.iter().find(|p| &p.id == id).ok_or_else(|| format!("no repair named {id} in this file"))
    };
    let mut feature = preview.feature.clone();
    for id in &review.decline {
        feature = feature.declined(&find(id)?.question);
    }
    for approved in &review.accept {
        let proposal = find(&approved.id)?;
        let approver = approved.approver.trim();
        if proposal.disposition != Disposition::Ask {
            return Err(format!("{} is repaired automatically and takes no approval", approved.id));
        }
        if review.decline.contains(&approved.id) {
            return Err(format!("{} is both accepted and declined", approved.id));
        }
        if !approvable(proposal) {
            return Err("Coming soon".into());
        }
        if approver.is_empty() {
            return Err("an approved repair needs the name of who approved it".into());
        }
        if !plain_mm(approved.tolerance_mm.trim()) {
            return Err("The allowed movement must be a number of millimetres, such as 0.03.".into());
        }
        feature = feature.accepted(&proposal.question, approved.tolerance_mm.trim(), approver);
    }
    let answered = |p: &Proposal| review.decline.contains(&p.id) || review.accept.iter().any(|a| a.id == p.id);
    if preview.proposals.proposals.iter().any(|p| p.disposition == Disposition::Ask && !answered(p)) {
        return Err("this repair needs an explicit tolerance approval before it can be imported".into());
    }
    let command = match preview.replacement {
        Some(id) => Command::EditFeature { id, kind: FeatureKind::ImportStep(feature) },
        None => {
            let solids = if feature.format == ImportFormat::Mesh {
                Vec::new()
            } else {
                doc.kernel().source_solids(feature.format, &feature.source).map_err(plain)?
            };
            if solids.len() > 1 {
                Command::ImportBodies(
                    solids
                        .into_iter()
                        .map(|entity| {
                            let mut selected = feature.clone();
                            selected.source_solid = Some(entity);
                            selected
                        })
                        .collect(),
                )
            } else {
                Command::AddFeature(FeatureKind::ImportStep(feature))
            }
        }
    };
    // Validate the whole import before changing the part, then reuse its builds.
    let mut trial = doc.scratch();
    let trial_result = trial.apply(command.clone());
    if matches!(command, Command::ImportBodies(_))
        && let Some(failed) = trial
            .features
            .iter()
            .filter(|f| doc.feature(f.id).is_none())
            .find(|f| f.status != crate::document::FeatureStatus::Ok)
    {
        return Err(match &failed.status {
            crate::document::FeatureStatus::Error(message) => message.clone(),
            _ => "The imported bodies could not be built. Restore the full feature tree and review the file again."
                .into(),
        });
    }
    let imported = match preview.replacement {
        Some(id) => trial.feature(id),
        None => trial.features.last(),
    }
    .ok_or("the import made no feature")?;
    if preview.replacement.is_some() && imported.status != crate::document::FeatureStatus::Ok {
        return Err(match &imported.status {
            crate::document::FeatureStatus::Error(message) => message.clone(),
            _ => "The replacement could not be built. Restore the full feature tree and review it again.".into(),
        });
    }
    let record = trial.model().import_record(soliddraft_model::FeatureId(imported.id as u32));
    let declined = trial_result.is_err()
        && record.is_some_and(|r| {
            !r.changes.is_empty()
                && r.derivative.is_none()
                && r.changes.iter().all(|c| c.approval == Approval::Declined)
        });
    if !declined {
        trial_result?;
    }
    let declined_record = declined.then(|| record.cloned()).flatten();
    doc.reuse_builds_from(trial.model());
    match doc.apply(command) {
        Ok(Some(id)) => Ok(id),
        Err(error) => {
            let imported = doc.features.last().ok_or("the import made no feature")?;
            let record = doc.model().import_record(soliddraft_model::FeatureId(imported.id as u32));
            if declined && record == declined_record.as_ref() { Ok(imported.id) } else { Err(error) }
        }
        Ok(None) => preview.replacement.ok_or_else(|| "the import made no feature".into()),
    }
}

/// The repair record as rows of the feature's properties: how each gap was answered and the
/// limit it was held to, how far its vertices moved, and how far each repaired face lies from
/// the source. Output error belongs to an export, and later features are not covered.
pub fn record_rows(record: &RepairRecord) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for (index, change) in record.changes.iter().enumerate() {
        let gap = format!("Gap {}", index + 1);
        rows.push((gap.clone(), answer_text(change)));
        if change.approval != Approval::Declined {
            rows.push((format!("{gap} approved limit"), format!("{} mm", change.tolerance_mm)));
            rows.push((format!("{gap} vertex movement"), format!("{} mm", change.max_movement_mm)));
        }
    }
    for face in &record.face_tolerances {
        rows.push((format!("Face {} from source", face.face), format!("up to {} mm", face.deviation_upper_mm)));
    }
    for face in &record.source_incidence {
        rows.push((
            format!("Face {} source discrepancy", face.face),
            format!("up to {} mm", face.source_discrepancy_upper_mm),
        ));
        rows.push((
            format!("Face {} shape deviation", face.face),
            format!("up to {} mm", face.geometric_deviation_upper_mm),
        ));
        rows.push((
            format!("Face {} source to result", face.face),
            format!("up to {} mm", face.source_to_result_upper_mm),
        ));
    }
    if !record.changes.is_empty() || !record.source_incidence.is_empty() || !record.face_tolerances.is_empty() {
        rows.push(("Repair figures".into(), "Cover the import only. Each export reports its own error.".into()));
    }
    rows
}

fn answer_text(change: &RepairChange) -> String {
    let kind = kind_name(&change.kind);
    match change.approval {
        Approval::Automatic => format!("Repaired {kind} automatically"),
        Approval::User => {
            format!("Repaired {kind}, approved by {}", change.approver.as_deref().unwrap_or("the reviewer"))
        }
        Approval::Declined => format!("Declined {kind}; the solid stays open"),
    }
}

fn proposal_group(ui: &mut egui::Ui, proposal: &Proposal, allowed: &mut BTreeMap<String, String>) {
    ui.label(format!("{}: {}", gap_name(proposal), kind_name(&proposal.kind)));
    ui.label(if proposal.distance_min_mm == proposal.distance_max_mm {
        format!("Measured {} mm", proposal.distance_max_mm)
    } else {
        format!("Measured {} to {} mm", proposal.distance_min_mm, proposal.distance_max_mm)
    });
    match (proposal.disposition, allowed.get_mut(&proposal.id)) {
        (Disposition::Ask, Some(_)) if !approvable(proposal) => {
            ui.label("Coming soon");
        }
        (Disposition::Ask, Some(value)) => {
            ui.horizontal(|ui| {
                let label = ui.label("Allowed movement");
                ui.add(egui::TextEdit::singleline(value).desired_width(90.0)).labelled_by(label.id);
                ui.label("mm");
            });
            if !covers_gap(value.trim(), proposal) {
                ui.label(format!("Enter at least {} mm", proposal.distance_max_mm));
            }
        }
        _ => {
            ui.label(format!("Repaired automatically, within {} mm", proposal.tolerance_mm));
        }
    }
    let mut reason = proposal.reason.chars();
    ui.label(reason.next().map_or(String::new(), |first| first.to_uppercase().chain(reason).collect()));
    ui.label(format!("File entities {}", proposal.entities.join(", ")));
}

pub fn window(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut preview) = state.import_dialog.take() else { return };
    let mut open = true;
    let mut choice = None;
    let mut cancelled = false;
    let format = preview.feature.format;
    let title = if preview.replacement.is_some() {
        "Replace imported file".to_owned()
    } else {
        format!("Import {}", format.label())
    };
    egui::Window::new(title).collapsible(false).open(&mut open).show(ctx, |ui| {
        ui.label(&preview.feature.source_name);
        ui.label("Review repairs before importing. The original file stays unchanged.");
        if format == ImportFormat::Mesh {
            ui.label("STL and OBJ coordinates are read in millimetres. The mesh must form one closed solid, up to 200,000 triangles.");
        }
        if preview.replacement.is_some() {
            ui.label("References to a changed file will need repair. You can undo the replacement.");
        }
        let Preview { proposals, allowed, approver, .. } = &mut preview;
        let asked = || proposals.proposals.iter().filter(|p| p.disposition == Disposition::Ask);
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for proposal in &proposals.proposals {
                ui.group(|ui| proposal_group(ui, proposal, allowed));
            }
            if proposals.proposals.is_empty() {
                ui.label("No repairs proposed.");
            }
        });
        if asked().any(approvable) {
            ui.horizontal(|ui| {
                let label = ui.label("Approved by");
                ui.text_edit_singleline(approver).labelled_by(label.id);
            });
        }
        let ready =
            asked().all(|p| approvable(p) && !approver.trim().is_empty() && covers_gap(allowed[&p.id].trim(), p));
        ui.horizontal(|ui| {
            if ui.add_enabled(ready, egui::Button::new("Accept")).clicked() {
                choice = Some(false);
            }
            if ui.add_enabled(!proposals.proposals.is_empty(), egui::Button::new("Decline")).clicked() {
                choice = Some(true);
            }
            if ui.button("Cancel").clicked() {
                cancelled = true;
            }
        });
    });
    if let Some(decline) = choice {
        let proposals = &preview.proposals.proposals;
        let review = if decline {
            Review { decline: proposals.iter().map(|p| p.id.clone()).collect(), accept: Vec::new() }
        } else {
            let accept = preview.allowed.iter().map(|(id, tolerance_mm)| Approved {
                id: id.clone(),
                tolerance_mm: tolerance_mm.clone(),
                approver: preview.approver.clone(),
            });
            Review { decline: Vec::new(), accept: accept.collect() }
        };
        match commit(&mut state.doc, &preview, &review) {
            Ok(_) => state.note(
                if decline {
                    "Import saved with declined repairs; the solid stays open".into()
                } else if preview.replacement.is_some() {
                    "Replaced imported file. Check any references marked for repair.".into()
                } else {
                    format!("Imported {} body", format.label())
                },
                NoteKind::Info,
            ),
            Err(error) => {
                state.note(error, NoteKind::Error);
                state.import_dialog = Some(preview);
            }
        }
    } else if open && !cancelled {
        state.import_dialog = Some(preview);
    }
}
