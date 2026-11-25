//! Read-only geometry interface bound to the body or placed assembly being drawn.
//!
//! Drawings never build or mutate geometry. [`DrawingGeometry`] narrows the contract to
//! the topology, history, edge curves, view projection, section cutting, face carriers
//! and occlusion meshes a drawing actually queries. Two adapters implement it:
//!
//! - [`KernelBody`]: a single body in a [`Kernel`] session, with mesh hidden-line
//!   fallback for preview views (`spec.preview == true`) only.
//! - [`crate::PlacedBodies`]: an assembly's placed parts, transforming queries into each
//!   part's frame, prefixing entity origins with the component path, naming refusals by
//!   component, and measuring inter-part occlusion against meshes.

use std::sync::Arc;

use soliddraft_kernel_api::drawing::{SectionSpec, SectionView, ViewCurve, ViewCurves, ViewSpec, mesh_hlr};
use soliddraft_kernel_api::{
    BodyId, EdgeId, EdgePolyline, FaceCarrier, HistoryRecord, Kernel, KernelError, KernelResult, Mesh,
    TessellationParams, Topology,
};

/// Read-only geometry queries bound to the body or placed assembly a drawing shows.
pub trait DrawingGeometry: Send + Sync {
    /// Topology (faces, edges, vertices and bounds) of the drawn geometry.
    fn topology(&self) -> KernelResult<Topology>;

    /// Operation history records used to fingerprint and name entities.
    fn history(&self) -> KernelResult<Vec<HistoryRecord>>;

    /// Analytic face carriers (planes, axes) used when placing PMI annotations.
    fn face_carriers(&self) -> KernelResult<Vec<FaceCarrier>> {
        Ok(Vec::new())
    }

    /// Exact or approximate curve of an edge for drawing projection and indexing.
    fn edge_drawing(&self, edge: EdgeId) -> KernelResult<ViewCurve>;

    /// Sampled edge polylines used as a fallback when an edge has no exact drawing curve.
    fn rendering_edges(&self, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>>;

    /// Projected visible and hidden curves for a view.
    ///
    /// When `spec.preview` is `true`, an adapter may fall back to an unproved mesh
    /// hidden-line view (`unproved = true`, `catallax = false`) if exact view curves are
    /// unsupported. When `spec.preview` is `false` (proved / exported line work), an
    /// unsupported view must return `Err(KernelError::Unsupported(_))` and never promote
    /// an unproved preview.
    fn view_curves(&self, spec: &ViewSpec) -> KernelResult<ViewCurves>;

    /// Cut line work and hatch regions for a section view.
    fn section_view(&self, spec: &SectionSpec) -> KernelResult<SectionView>;

    /// A triangle mesh of this geometry for inter-part occlusion when placed in an
    /// assembly: the preview mesh when `preview` is `true`, or a certified tessellation
    /// when `preview` is `false`.
    fn occlusion_mesh(&self, params: &TessellationParams, preview: bool) -> KernelResult<Mesh>;
}

impl<T: DrawingGeometry + ?Sized> DrawingGeometry for &T {
    fn topology(&self) -> KernelResult<Topology> {
        (**self).topology()
    }

    fn history(&self) -> KernelResult<Vec<HistoryRecord>> {
        (**self).history()
    }

    fn face_carriers(&self) -> KernelResult<Vec<FaceCarrier>> {
        (**self).face_carriers()
    }

    fn edge_drawing(&self, edge: EdgeId) -> KernelResult<ViewCurve> {
        (**self).edge_drawing(edge)
    }

    fn rendering_edges(&self, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        (**self).rendering_edges(params)
    }

    fn view_curves(&self, spec: &ViewSpec) -> KernelResult<ViewCurves> {
        (**self).view_curves(spec)
    }

    fn section_view(&self, spec: &SectionSpec) -> KernelResult<SectionView> {
        (**self).section_view(spec)
    }

    fn occlusion_mesh(&self, params: &TessellationParams, preview: bool) -> KernelResult<Mesh> {
        (**self).occlusion_mesh(params, preview)
    }
}

impl<T: DrawingGeometry + ?Sized> DrawingGeometry for Arc<T> {
    fn topology(&self) -> KernelResult<Topology> {
        (**self).topology()
    }

    fn history(&self) -> KernelResult<Vec<HistoryRecord>> {
        (**self).history()
    }

    fn face_carriers(&self) -> KernelResult<Vec<FaceCarrier>> {
        (**self).face_carriers()
    }

    fn edge_drawing(&self, edge: EdgeId) -> KernelResult<ViewCurve> {
        (**self).edge_drawing(edge)
    }

    fn rendering_edges(&self, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        (**self).rendering_edges(params)
    }

    fn view_curves(&self, spec: &ViewSpec) -> KernelResult<ViewCurves> {
        (**self).view_curves(spec)
    }

    fn section_view(&self, spec: &SectionSpec) -> KernelResult<SectionView> {
        (**self).section_view(spec)
    }

    fn occlusion_mesh(&self, params: &TessellationParams, preview: bool) -> KernelResult<Mesh> {
        (**self).occlusion_mesh(params, preview)
    }
}

impl<T: DrawingGeometry + ?Sized> DrawingGeometry for Box<T> {
    fn topology(&self) -> KernelResult<Topology> {
        (**self).topology()
    }

    fn history(&self) -> KernelResult<Vec<HistoryRecord>> {
        (**self).history()
    }

    fn face_carriers(&self) -> KernelResult<Vec<FaceCarrier>> {
        (**self).face_carriers()
    }

    fn edge_drawing(&self, edge: EdgeId) -> KernelResult<ViewCurve> {
        (**self).edge_drawing(edge)
    }

    fn rendering_edges(&self, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        (**self).rendering_edges(params)
    }

    fn view_curves(&self, spec: &ViewSpec) -> KernelResult<ViewCurves> {
        (**self).view_curves(spec)
    }

    fn section_view(&self, spec: &SectionSpec) -> KernelResult<SectionView> {
        (**self).section_view(spec)
    }

    fn occlusion_mesh(&self, params: &TessellationParams, preview: bool) -> KernelResult<Mesh> {
        (**self).occlusion_mesh(params, preview)
    }
}

/// A single body in a kernel session, bound as a read-only [`DrawingGeometry`].
#[derive(Clone, Copy)]
pub struct KernelBody<'k> {
    /// The kernel session that owns the body.
    pub kernel: &'k dyn Kernel,
    /// The body being drawn.
    pub body: BodyId,
}

impl<'k> KernelBody<'k> {
    /// Binds `body` in `kernel` for read-only drawing queries.
    pub fn new(kernel: &'k dyn Kernel, body: BodyId) -> KernelBody<'k> {
        KernelBody { kernel, body }
    }
}

impl DrawingGeometry for KernelBody<'_> {
    fn topology(&self) -> KernelResult<Topology> {
        self.kernel.topology(self.body)
    }

    fn history(&self) -> KernelResult<Vec<HistoryRecord>> {
        self.kernel.history(self.body)
    }

    fn face_carriers(&self) -> KernelResult<Vec<FaceCarrier>> {
        self.kernel.face_carriers(self.body)
    }

    fn edge_drawing(&self, edge: EdgeId) -> KernelResult<ViewCurve> {
        self.kernel.edge_drawing(self.body, edge)
    }

    fn rendering_edges(&self, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        self.kernel.rendering_edges(self.body, params)
    }

    fn view_curves(&self, spec: &ViewSpec) -> KernelResult<ViewCurves> {
        match self.kernel.view_curves(self.body, spec) {
            Err(KernelError::Unsupported(_)) if spec.preview => {
                let mut curves = mesh_hlr::view_curves(self.kernel, self.body, spec)?;
                curves.unproved = true;
                Ok(curves)
            }
            other => other,
        }
    }

    fn section_view(&self, spec: &SectionSpec) -> KernelResult<SectionView> {
        self.kernel.section_view(self.body, spec)
    }

    fn occlusion_mesh(&self, params: &TessellationParams, preview: bool) -> KernelResult<Mesh> {
        if preview { self.kernel.preview(self.body, params) } else { self.kernel.tessellate(self.body, params) }
    }
}
