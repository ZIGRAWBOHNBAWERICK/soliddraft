//! Scan references and sampled fits. A reference has no inside or volume.

use serde::{Deserialize, Serialize};

/// The original scan, embedded with the part. Coordinates are millimetres.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanSource {
    /// Original file name.
    pub name: String,
    /// Unchanged STL or OBJ bytes.
    pub bytes: Vec<u8>,
    /// SHA-256 of those bytes.
    pub sha256: String,
}

/// Display data and counts from an admitted reference. No repair is implied.
#[derive(Clone, Debug, Default)]
pub struct ScanReference {
    /// Placed vertices, including isolated samples.
    pub points: Vec<[f64; 3]>,
    /// Original triangles, including degenerate ones.
    pub triangles: Vec<[u32; 3]>,
    /// File triangles.
    pub triangle_count: usize,
    /// Open edges.
    pub boundary_edges: usize,
    /// Reversed edge pairs.
    pub inconsistent_edges: usize,
    /// Edges shared by more than two faces.
    pub nonmanifold_edges: usize,
    /// Isolated samples.
    pub isolated_points: usize,
}

/// Statistics in one direction. These are sampled distances, not bounds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanDeviationSide {
    /// Number of measured samples.
    pub samples: usize,
    /// Largest sampled distance in millimetres.
    pub max_mm: f64,
    /// Root mean square distance in millimetres.
    pub rms_mm: f64,
    /// 95th percentile distance in millimetres.
    pub p95_mm: f64,
}

/// A distance at a point in a colour map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanMapPoint {
    /// Position in millimetres.
    pub point: [f64; 3],
    /// Sampled distance in millimetres.
    pub distance_mm: f64,
}

/// A fitted part and the complete sampled evidence returned with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScanFit {
    /// Ordinary native recipe. Reopening rebuilds these definitions, not the fit.
    pub recipe: serde_json::Value,
    /// Samples from the scan to the part.
    pub scan_to_part: ScanDeviationSide,
    /// Samples from the part to the scan.
    pub part_to_scan: ScanDeviationSide,
    /// Scan-to-part map at every fitting sample.
    pub scan_map: Vec<ScanMapPoint>,
    /// Part-to-scan map at every part sample.
    pub part_map: Vec<ScanMapPoint>,
    /// Proven chord bound of the display mesh used for the distance query.
    pub chord_mm: f64,
    /// Regions the fit could not explain or place, in plain words.
    pub warnings: Vec<String>,
}
