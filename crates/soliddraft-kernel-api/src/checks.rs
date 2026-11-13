//! Independent construction coverage and qualified print evidence.

use crate::Mesh;
use serde::Serialize;

/// Construction coverage, without a claim of geometric agreement.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IndependentCheck {
    /// Every authored recipe step, including reference definitions.
    pub steps: Vec<IndependentStep>,
}

/// One authored prefix built separately by both representations.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IndependentStep {
    /// Recipe step name.
    pub step: String,
    /// Operation name.
    pub operation: String,
    /// Application feature responsible for this step, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_tag: Option<crate::OpTag>,
    /// A sketch, path or other reference rather than a solid.
    pub reference: bool,
    /// Whether the boundary representation built this prefix.
    pub boundary_built: bool,
    /// Whether the distance-field representation built this prefix.
    pub field_built: bool,
    /// Refusals, including which representation refused.
    pub unconfirmed: Vec<String>,
}

/// Evidence from the native print route for the actual encoded geometry.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrintReport {
    /// Selected representation. Refusals never switch it.
    pub route: String,
    /// Requested accuracy in millimetres.
    pub requested_accuracy_mm: f64,
    /// Complete certified bound including encoding, rounded upward.
    pub certified_accuracy_mm: f64,
    /// Model-to-definition contribution, rounded upward.
    pub model_bound_mm: f64,
    /// Finite face and trim contribution, rounded upward.
    pub tessellation_bound_mm: f64,
    /// Encoding contribution, rounded upward.
    pub rounding_bound_mm: f64,
    /// Native qualification results in plain words.
    pub stages: Vec<String>,
}

/// Qualified geometry. All formats must preserve these positions and triangles.
#[derive(Clone, Debug)]
pub struct PrintMesh {
    /// Geometry with face labels and normals for OBJ.
    pub mesh: Mesh,
    /// Native metadata-bearing binary STL.
    pub stl: Vec<u8>,
    /// Evidence for the encoded mesh.
    pub report: PrintReport,
}
