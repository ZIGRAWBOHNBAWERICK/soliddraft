//! Input for an unproved preview meshed on the renderer's GPU.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use crate::{EdgePolyline, FaceId};

/// One body's float description for a GPU mesher: its faces and the edge samples they
/// share. It holds no triangles and no bound. The screen draws it in place of
/// [`Kernel::preview`](crate::Kernel::preview) until a checked display replaces it, and
/// nothing measures, exports or publishes it.
#[derive(Clone)]
pub struct GpuPreview {
    /// World-space origin in millimetres. The backend's description is relative to it.
    pub origin: [f64; 3],
    /// The faces in description order, the order a GPU mesher reports them in.
    pub faces: Vec<FaceId>,
    /// Each edge sampled once, in world coordinates. Both faces of an edge use these
    /// samples as their boundary, so the screen draws the same points as edges.
    pub edges: Vec<EdgePolyline>,
    /// The backend's own description. Only that backend's GPU mesher reads it.
    pub source: Arc<dyn Any + Send + Sync>,
}

impl fmt::Debug for GpuPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuPreview")
            .field("origin", &self.origin)
            .field("faces", &self.faces.len())
            .field("edges", &self.edges.len())
            .finish_non_exhaustive()
    }
}

impl PartialEq for GpuPreview {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source)
            && self.origin == other.origin
            && self.faces == other.faces
            && self.edges == other.edges
    }
}
