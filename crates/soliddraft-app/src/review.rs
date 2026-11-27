//! Compare the authored PMI of two saved parts and attach comments in a separate file.
//!
//! The caller chooses which files are revisions of the same part. Items match by
//! their saved IDs, not by position, text or proximity. A rename is a removal and
//! an addition. Generated PMI and whole-part notes have no saved item IDs and are
//! not compared. Native PMI resolution supplies the measurements and diagnostics.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::io::{self, Cursor};
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use soliddraft_document::{ContentHash, DocumentId, identify, save_new};
use soliddraft_kernel_api::Kernel;
use soliddraft_kernel_api::pmi::{PmiItem, PmiKind};
use soliddraft_model::pmi::PmiRow;
use soliddraft_model::{Document, FaceRef, FeatureId, FileError, Generator};

/// The identity and semantic revision of a saved part, independent of its path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionPin {
    pub document: DocumentId,
    pub revision: ContentHash,
}

/// The origin of a face. A saved reference's fingerprint still checks resolution
/// in its own revision; repair labels and checks are not cross-revision IDs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FaceOrigin {
    pub feature: FeatureId,
    pub generator: Generator,
    pub ordinal: u32,
}

impl From<&FaceRef> for FaceOrigin {
    fn from(face: &FaceRef) -> Self {
        Self { feature: face.feature, generator: face.generator.clone(), ordinal: face.ordinal }
    }
}

/// A saved PMI item and the measurements available after reopening its part.
#[derive(Debug)]
pub struct ItemState {
    pub item: PmiItem<FaceOrigin>,
    /// Diameter, radius or distance; for a slot, length, width and end radius.
    /// `None` means this is not a dimension or its measurement is unavailable.
    pub nominal_mm: Option<Vec<f64>>,
    /// Why the native PMI could not be resolved or measured in this revision.
    pub problem: Option<String>,
}

impl ItemState {
    fn from_row(row: PmiRow) -> Self {
        let mut problem = row.problem;
        let count = match row.item.kind {
            PmiKind::Size { .. } | PmiKind::Location { .. } => 1,
            PmiKind::Slot { .. } => 3,
            _ => 0,
        };
        let nominal_mm = if count == 0 || problem.is_some() {
            None
        } else if row.measured.len() == count && row.measured.iter().all(|value| value.is_finite()) {
            Some(row.measured)
        } else {
            problem = Some("The dimension did not return all of its measurements.".into());
            None
        };
        let Ok(mut item) = row.item.try_map_features(|faces| {
            let mut origins: Vec<_> = faces.iter().map(FaceOrigin::from).collect();
            origins.sort_unstable();
            Ok::<_, Infallible>(origins)
        });
        match &mut item.kind {
            PmiKind::Size { features, .. } | PmiKind::Gdt { features, .. } => features.sort_unstable(),
            PmiKind::Slot { flats, ends, .. } => {
                flats.sort_unstable();
                ends.sort_unstable();
            }
            // Location endpoints and datum precedence are ordered by meaning.
            _ => {}
        }
        Self { item, nominal_mm, problem }
    }

    fn same_as(&self, other: &Self) -> bool {
        self.item == other.item
            && self.nominal_mm == other.nominal_mm
            && self.problem.is_some() == other.problem.is_some()
    }
}

/// A read-only review snapshot. It does not retain a live document or kernel body.
#[derive(Debug)]
pub struct SavedRevision {
    pin: RevisionPin,
    items: Vec<ItemState>,
}

impl SavedRevision {
    /// Reopens a native part package. The pin comes from the saved revision, not
    /// from re-encoding or normalizing the review data.
    pub fn from_bytes(bytes: &[u8], kernel: Arc<dyn Kernel>) -> Result<Self, FileError> {
        let identity = identify(Cursor::new(bytes))?;
        let part = Document::from_bytes(bytes, kernel)?;
        let mut items: Vec<_> =
            part.pmi_rows().into_iter().filter(|row| row.feature.is_none()).map(ItemState::from_row).collect();
        items.sort_unstable_by(|a, b| a.item.id.cmp(&b.item.id));
        Ok(Self { pin: RevisionPin { document: identity.id, revision: identity.revision }, items })
    }

    pub fn open(path: &Path, kernel: Arc<dyn Kernel>) -> Result<Self, FileError> {
        Self::from_bytes(&std::fs::read(path)?, kernel)
    }

    pub fn pin(&self) -> &RevisionPin {
        &self.pin
    }

    pub fn item(&self, id: &str) -> Option<&ItemState> {
        self.items.binary_search_by(|state| state.item.id.as_str().cmp(id)).ok().map(|index| &self.items[index])
    }

    /// All unresolved or unmeasurable items on this side, even if they did not change.
    pub fn issues(&self) -> impl Iterator<Item = (&str, &str)> {
        self.items.iter().filter_map(|state| Some((state.item.id.as_str(), state.problem.as_deref()?)))
    }

    /// Returns changes in item-ID order. The original semantic pins are unaffected
    /// by this ordering and may differ even when there are no PMI changes.
    pub fn compare<'a>(&'a self, candidate: &'a Self) -> RevisionComparison<'a> {
        let mut baseline_items = self.items.iter().peekable();
        let mut candidate_items = candidate.items.iter().peekable();
        let mut changes = Vec::new();
        while let Some(id) = [baseline_items.peek().copied(), candidate_items.peek().copied()]
            .into_iter()
            .flatten()
            .map(|state| state.item.id.as_str())
            .min()
        {
            let before = baseline_items.next_if(|state| state.item.id == id);
            let after = candidate_items.next_if(|state| state.item.id == id);
            if !matches!((before, after), (Some(before), Some(after)) if before.same_as(after)) {
                changes.push(ItemChange { id, before, after });
            }
        }
        RevisionComparison { baseline: self, candidate, changes }
    }

    /// Attaches a review comment to this candidate, or to one of its saved PMI
    /// items. Returns `None` for an item ID not present in the candidate.
    pub fn markup(&self, item_id: Option<&str>, text: String) -> Option<ReviewerMarkup> {
        if let Some(id) = item_id {
            self.item(id)?;
        }
        Some(ReviewerMarkup { candidate: self.pin.clone(), item_id: item_id.map(str::to_owned), text })
    }
}

#[derive(Debug)]
pub struct RevisionComparison<'a> {
    pub baseline: &'a SavedRevision,
    pub candidate: &'a SavedRevision,
    pub changes: Vec<ItemChange<'a>>,
}

/// The same ID on both sides is a change. A missing side is an unmatched item ID.
#[derive(Debug)]
pub struct ItemChange<'a> {
    pub id: &'a str,
    pub before: Option<&'a ItemState>,
    pub after: Option<&'a ItemState>,
}

impl ItemChange<'_> {
    /// Candidate minus baseline, in millimetres, for like dimensions measured on both sides.
    pub fn dimension_delta_mm(&self) -> Option<Vec<f64>> {
        let (before, after) = (self.before?, self.after?);
        match (&before.item.kind, &after.item.kind) {
            (PmiKind::Size { size: before, .. }, PmiKind::Size { size: after, .. }) if before == after => {}
            (PmiKind::Location { .. }, PmiKind::Location { .. }) | (PmiKind::Slot { .. }, PmiKind::Slot { .. }) => {}
            _ => return None,
        }
        Some(
            before
                .nominal_mm
                .as_ref()?
                .iter()
                .zip(after.nominal_mm.as_ref()?)
                .map(|(before, after)| after - before)
                .collect(),
        )
    }

    /// Face origins named on only one side of this item. This is separate from
    /// native resolution failures, which are reported by each revision's `issues`.
    pub fn unmatched_faces(&self) -> UnmatchedFaces<'_> {
        fn origins(state: Option<&ItemState>) -> BTreeSet<&FaceOrigin> {
            state.into_iter().flat_map(|state| state.item.features()).flatten().collect()
        }
        let before = origins(self.before);
        let after = origins(self.after);
        UnmatchedFaces {
            baseline_only: before.difference(&after).copied().collect(),
            candidate_only: after.difference(&before).copied().collect(),
        }
    }
}

#[derive(Debug)]
pub struct UnmatchedFaces<'a> {
    pub baseline_only: Vec<&'a FaceOrigin>,
    pub candidate_only: Vec<&'a FaceOrigin>,
}

/// JSON sidecar data. No paths, kernel IDs or modifications to the part are needed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerMarkup {
    pub candidate: RevisionPin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    pub text: String,
}

impl ReviewerMarkup {
    pub fn applies_to(&self, candidate: &SavedRevision) -> bool {
        self.candidate == *candidate.pin() && self.item_id.as_deref().is_none_or(|id| candidate.item(id).is_some())
    }

    /// Creates a separate file; refuses any existing path, including either source.
    pub fn save_new(&self, path: &Path) -> io::Result<()> {
        save_new(path, &serde_json::to_vec_pretty(self)?)
    }

    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
}
