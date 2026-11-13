//! Evidence shipped alongside a STEP file.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The answer for one feature. Inactive features contributed no geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerKind {
    /// Exact result accepted.
    Proved,
    /// The requested result has no answer yet.
    Pending,
    /// Measured deviation only, never proof.
    WithinStatedBound,
    /// The requested result was refused.
    Refused,
    /// Suppressed or below the rollback bar.
    Inactive,
}

impl AnswerKind {
    /// Plain label for the report.
    pub fn label(self) -> &'static str {
        match self {
            Self::Proved => "Proved",
            Self::Pending => "Pending",
            Self::WithinStatedBound => "Within a stated bound",
            Self::Refused => "Refused",
            Self::Inactive => "Inactive",
        }
    }
}

/// An individual feature's model answer and independent rendering evidence.
#[derive(Clone, Debug, Serialize)]
pub struct FeatureProof {
    /// Stable feature id or operation tag.
    pub id: String,
    /// User's feature name.
    pub name: String,
    /// Answer about the exact model, independent of its rendering.
    pub answer_kind: AnswerKind,
    /// Bound on the model answer, if any.
    pub model_bound: Option<serde_json::Value>,
    /// Measured rendering evidence, never an export certificate.
    pub display_bound: Option<serde_json::Value>,
    /// Deterministic work for the retained feature build, excluding display and export.
    /// `None` means this backend or input did not record it, never zero work.
    pub work_used: Option<u64>,
    /// Plain reason for a pending or refused answer, or inactivity.
    pub message: Option<String>,
}

/// The model evidence kept with the exact bytes being exported.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ProofReport {
    /// Features in document order, including suppressed and rolled-back features.
    pub features: Vec<FeatureProof>,
}

/// Appends to the whole filename, preserving `.step` or `.stp`.
pub fn sidecar_path(path: &Path, extension: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".proof.{extension}"));
    PathBuf::from(name)
}

impl super::Export {
    /// Writes proof reports first and publishes the STEP file last. All report
    /// encoding and export validation finish before the writer touches any file.
    /// Each report names the SHA-256 of the STEP bytes it describes.
    pub fn write_with(
        &self,
        path: &Path,
        mut write: impl FnMut(&Path, &[u8]) -> Result<(), String>,
    ) -> Result<(), String> {
        if matches!(self.report, super::Report::Step(_) | super::Report::StandIn(_)) {
            let proof = self.proof.as_ref().ok_or("This STEP file has no proof report; export was not written")?;
            if proof
                .features
                .iter()
                .any(|feature| !matches!(feature.answer_kind, AnswerKind::Proved | AnswerKind::Inactive))
            {
                return Err("This part has an unproved answer. STEP export requires a proved result.".into());
            }
            let hash = format!("{:x}", Sha256::digest(&self.bytes));
            let json = serde_json::json!({
                "format": "soliddraft-proof/1",
                "file": path.file_name().map(|name| name.to_string_lossy()),
                "sha256": hash,
                "work_scope": "Original retained feature builds; excludes display and export. Null means unavailable.",
                "features": proof.features,
                "export": self.report,
            });
            let mut machine = serde_json::to_vec_pretty(&json).map_err(|e| e.to_string())?;
            machine.push(b'\n');
            let mut human = format!(
                "SolidDraft STEP proof report\nFile: {}\nSHA-256: {hash}\n{}\n\nWork counts cover original retained feature builds, excluding display and export.\nUnavailable means no count was recorded. Inactive features contributed no geometry.\nDisplay bounds are measured deviations, never model proof.\n",
                path.display(),
                self.report.summary()
            );
            for feature in &proof.features {
                let work = feature.work_used.map_or_else(|| "unavailable".into(), |work| work.to_string());
                human.push_str(&format!(
                    "\n{} [{}]: {}\nWork used: {work}\n",
                    feature.name,
                    feature.id,
                    feature.answer_kind.label()
                ));
                for (label, bound) in [("Model bound", &feature.model_bound), ("Display bound", &feature.display_bound)]
                {
                    if let Some(bound) = bound {
                        human.push_str(&format!(
                            "{label}: {}\n",
                            serde_json::to_string_pretty(bound).map_err(|e| e.to_string())?
                        ));
                    }
                }
                if let Some(message) = &feature.message {
                    human.push_str(&format!("{message}\n"));
                }
            }
            write(&sidecar_path(path, "json"), &machine)?;
            write(&sidecar_path(path, "txt"), human.as_bytes())?;
        }
        write(path, &self.bytes)
    }
}
