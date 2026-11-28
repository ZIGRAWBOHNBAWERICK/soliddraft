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
    /// Built within a stated tolerance, with no exact proof.
    Approximate,
}

impl AnswerKind {
    /// Plain label for the report.
    pub fn label(self) -> &'static str {
        match self {
            Self::Proved => "Exact",
            Self::Pending => "Pending",
            Self::WithinStatedBound => "Within a stated bound",
            Self::Refused => "Refused",
            Self::Inactive => "Inactive",
            Self::Approximate => "Approximate, within its stated tolerance",
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

/// The machine report's format.
pub const FORMAT: &str = "soliddraft-precision/1";
/// The format older versions wrote, still read.
const LEGACY_FORMAT: &str = "soliddraft-proof/1";
/// The suffix of the reports written beside a STEP file.
const SUFFIX: &str = "precision";
/// The suffix older versions wrote, still read.
const LEGACY_SUFFIX: &str = "proof";

/// Where a report beside `path` is written: the whole filename plus `.precision.<extension>`,
/// preserving `.step` or `.stp`.
pub fn sidecar_path(path: &Path, extension: &str) -> PathBuf {
    suffixed(path, SUFFIX, extension)
}

/// Where older versions wrote the same report: `.proof.<extension>`.
pub fn legacy_sidecar_path(path: &Path, extension: &str) -> PathBuf {
    suffixed(path, LEGACY_SUFFIX, extension)
}

fn suffixed(path: &Path, suffix: &str, extension: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{suffix}.{extension}"));
    PathBuf::from(name)
}

/// Reads the machine report beside the STEP file at `path`: the current name first, then
/// the name older versions wrote, so their exports still open.
pub fn read_report(path: &Path) -> Result<serde_json::Value, String> {
    for candidate in [sidecar_path(path, "json"), legacy_sidecar_path(path, "json")] {
        let bytes = match std::fs::read(&candidate) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("{}: {error}", candidate.display())),
        };
        let report: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", candidate.display()))?;
        return match report["format"].as_str() {
            Some(FORMAT | LEGACY_FORMAT) => Ok(report),
            other => Err(format!("{}: unknown report format {other:?}", candidate.display())),
        };
    }
    Err(format!("{} has no report beside it", path.display()))
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
            let proof = self.proof.as_ref().ok_or("This STEP file has no build report; export was not written")?;
            if proof.features.iter().any(|feature| {
                !matches!(feature.answer_kind, AnswerKind::Proved | AnswerKind::Approximate | AnswerKind::Inactive)
            }) {
                return Err("This part can't be exported as STEP yet: a feature is still building, was refused, or is answered only within a stated bound.".into());
            }
            let hash = format!("{:x}", Sha256::digest(&self.bytes));
            let json = serde_json::json!({
                "format": FORMAT,
                "file": path.file_name().map(|name| name.to_string_lossy()),
                "sha256": hash,
                "work_scope": "Original retained feature builds; excludes display and export. Null means unavailable.",
                "features": proof.features,
                "export": self.report,
            });
            let mut machine = serde_json::to_vec_pretty(&json).map_err(|e| e.to_string())?;
            machine.push(b'\n');
            let mut human = format!(
                "SolidDraft STEP precision report\nFile: {}\nSHA-256: {hash}\n{}\n\nWork counts cover original retained feature builds, excluding display and export.\nUnavailable means no count was recorded. Inactive features contributed no geometry.\nDisplay bounds are measured deviations of the screen mesh, not of the model.\n",
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

#[cfg(test)]
mod tests {
    use super::{FORMAT, legacy_sidecar_path, read_report, sidecar_path};
    use std::path::PathBuf;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("soliddraft-sidecar-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reports_are_written_under_the_precision_name() {
        let step = PathBuf::from("/parts/bracket.step");
        assert_eq!(sidecar_path(&step, "json"), PathBuf::from("/parts/bracket.step.precision.json"));
        assert_eq!(sidecar_path(&step, "txt"), PathBuf::from("/parts/bracket.step.precision.txt"));
        assert_eq!(legacy_sidecar_path(&step, "json"), PathBuf::from("/parts/bracket.step.proof.json"));
    }

    #[test]
    fn a_report_an_older_version_wrote_still_reads() {
        let dir = folder("legacy");
        let step = dir.join("old.step");
        std::fs::write(legacy_sidecar_path(&step, "json"), br#"{"format":"soliddraft-proof/1","sha256":"old"}"#)
            .unwrap();
        assert_eq!(read_report(&step).unwrap()["sha256"], "old");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_current_report_wins_over_an_older_one() {
        let dir = folder("current");
        let step = dir.join("part.step");
        std::fs::write(legacy_sidecar_path(&step, "json"), br#"{"format":"soliddraft-proof/1","sha256":"old"}"#)
            .unwrap();
        std::fs::write(sidecar_path(&step, "json"), format!(r#"{{"format":"{FORMAT}","sha256":"new"}}"#)).unwrap();
        assert_eq!(read_report(&step).unwrap()["sha256"], "new");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_or_foreign_report_is_refused() {
        let dir = folder("missing");
        let step = dir.join("none.step");
        assert!(read_report(&step).is_err());
        std::fs::write(sidecar_path(&step, "json"), br#"{"format":"other/1"}"#).unwrap();
        assert!(read_report(&step).unwrap_err().contains("unknown report format"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
