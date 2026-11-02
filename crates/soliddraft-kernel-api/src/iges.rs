//! Exact-surface IGES export and its whole-boundary certificate.

use serde::{Deserialize, Serialize};

/// Requested IGES accuracy, independent of modelling and mesh tolerances.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IgesOptions {
    /// Inclusive maximum boundary displacement in millimetres. Must be finite and nonnegative.
    pub tolerance: f64,
}

impl Default for IgesOptions {
    fn default() -> Self {
        Self { tolerance: 1e-7 }
    }
}

/// What IGES wrote, preserving Catallax's rational similarity certificate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IgesReport {
    /// Proven whole-boundary displacement in millimetres, rounded upward as a decimal.
    pub max_deviation: String,
    /// The same bound as the nearest `f64` not below it.
    pub max_deviation_mm: f64,
    /// Exact rational displacement bound, including every face and trim.
    pub boundary_bound_mm: String,
    /// Exact rational trim rounding bound in millimetres.
    pub trim_deviation_mm: String,
    /// Exact positive similarity scale applied to the written geometry.
    pub scale: String,
    /// Exact rational bound on the boundary's absolute coordinate sum in millimetres.
    pub coordinate_bound_mm: String,
    /// Exact rational requested tolerance in millimetres.
    pub tolerance_mm: String,
    /// Output length units, currently `mm`.
    pub units: String,
    /// Always true for a returned file. Larger bounds are refused.
    pub within_tolerance: bool,
    /// Manifold solids written.
    pub solids: usize,
    /// Faces written, preserving the exact body's count.
    pub faces: usize,
    /// Shared edges written, including export-only seams.
    pub edges: usize,
    /// Vertices written, including ring vertices required by IGES.
    pub vertices: usize,
    /// Export-only periodic seams.
    pub added_edges: usize,
    /// Export-only ring vertices.
    pub added_vertices: usize,
}

/// An IGES 5.3 file and its report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IgesExport {
    /// The file's text.
    pub text: String,
    /// What was written and its proven deviation.
    pub report: IgesReport,
}
