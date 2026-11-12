//! Read-only queries between two placed solids owned by one kernel.

use serde::Serialize;

use crate::{BodyId, FaceId, Measurement};

/// A body in the assembly frame. The quaternion is [x, y, z, w].
#[derive(Clone, Copy, Debug)]
pub struct PlacedBody {
    /// Kernel-owned solid.
    pub body: BodyId,
    /// Translation in millimetres.
    pub translation: [f64; 3],
    /// Nonzero rotation quaternion, normalised exactly by the backend.
    pub rotation: [f64; 4],
}

/// One refused quantity, with a stable diagnostic code and a user-facing reason.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BodyQueryRefusal {
    /// The refused quantity.
    pub query: String,
    /// The kernel's refusal identity.
    pub code: String,
    /// Plain explanation, safe to show in the results panel.
    pub message: String,
}

/// A volume enclosure and the exact sign decision before conversion to doubles.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OverlapVolume {
    /// Cubic millimetres.
    pub volume: Measurement,
    /// True for a proved positive volume, false for a proved zero, absent otherwise.
    pub positive: Option<bool>,
}

/// Source faces carrying shared area patches. Lists need not pair one to one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BodyContactFaces {
    /// Faces on the first operand.
    pub first: Vec<FaceId>,
    /// Faces on the second operand.
    pub second: Vec<FaceId>,
}

/// Minimum distance and decisions made against the rational bounds.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BodyDistance {
    /// Millimetres, with outward-rounded endpoints.
    pub distance: Measurement,
    /// True only when the upper bound is exactly zero.
    pub zero: bool,
    /// Whether the requested clearance is exceeded. Absent when not requested or undecided.
    pub exceeds_clearance: Option<bool>,
}

/// Independent answers: a refused volume does not discard proved contact faces.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BodyRelation {
    /// Regularised intersection volume. An empty result is a successful zero.
    pub overlap: Result<OverlapVolume, BodyQueryRefusal>,
    /// Shared area patches, excluding edge and point contacts.
    pub contact: Result<BodyContactFaces, BodyQueryRefusal>,
    /// Minimum distance, requested when needed to settle contact or clearance.
    pub gap: Option<Result<BodyDistance, BodyQueryRefusal>>,
}
