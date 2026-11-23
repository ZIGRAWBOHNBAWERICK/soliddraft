use soliddraft_kernel_api::*;
use soliddraft_kernel_catallax::CatallaxKernel;
use std::sync::{Condvar, Mutex};
/// Catallax, with selected operations held back or refused on request.
#[derive(Default)]
pub struct Gated {
    inner: CatallaxKernel,
    pub rules: Mutex<Rules>,
    pub changed: Condvar,
}

#[derive(Default)]
pub struct Rules {
    /// Extrusions this deep wait until the hold is lifted.
    pub hold: Option<f64>,
    pub hold_next: bool,
    /// True while an extrusion waits on the hold.
    pub holding: Option<f64>,
    /// Extrusions this deep fail.
    pub refuse: Vec<f64>,
    /// Fillets with these radii fail.
    pub refuse_fillet_radii: Vec<f64>,
    /// No body can be meshed for display.
    pub refuse_display: bool,
    /// Measured rendering evidence supplied independently of the model verdict.
    pub display_bound: Option<DisplayBound>,
    /// Bodies with more volume than this cannot be meshed for display.
    pub refuse_display_above: Option<f64>,
    /// Bodies with more volume than this cannot be sampled for their faces and edges, as
    /// when Catallax builds an approximate body its display writer then refuses.
    pub refuse_topology_above: Option<f64>,
    /// Pair proofs fail, as an incomplete bound would.
    pub refuse_pairs: bool,
    /// Refuses the next background proof reservation before any feature builds.
    pub refuse_proof_reservation: bool,
    /// Delays the next drawing view to exercise native request handling.
    pub drawing_delay: std::time::Duration,
}

impl Gated {
    fn check_extrude(&self, distance: f64) -> KernelResult<()> {
        let mut rules = self.rules.lock().unwrap();
        if rules.hold_next {
            rules.hold_next = false;
            rules.hold = Some(distance);
        }
        if rules.hold == Some(distance) {
            rules.holding = Some(distance);
            self.changed.notify_all();
            rules = self.changed.wait_while(rules, |r| r.hold == Some(distance)).unwrap();
            rules.holding = None;
        }
        if rules.refuse.contains(&distance) {
            return Err(KernelError::Failed {
                op: OpKind::Extrude,
                code: 0,
                message: format!("the scenario refuses a depth of {distance}"),
            });
        }
        Ok(())
    }

    fn display(&self, body: BodyId) -> KernelResult<()> {
        let (all, above) = {
            let rules = self.rules.lock().unwrap();
            (rules.refuse_display, rules.refuse_display_above)
        };
        if !all && !above.is_some_and(|limit| self.inner.mass_properties(body).is_ok_and(|m| m.volume > limit)) {
            return Ok(());
        }
        Err(KernelError::QueryFailed {
            query: "tessellate".into(),
            code: 0,
            message: "mesh.precision-or-budget: the scenario refuses to mesh".into(),
        })
    }
}
impl Kernel for Gated {
    fn edge_drawing(&self, body: BodyId, edge: EdgeId) -> KernelResult<drawing::ViewCurve> {
        self.inner.edge_drawing(body, edge)
    }

    fn edge_curve(&self, body: BodyId, edge: EdgeId) -> KernelResult<drawing::Curve3> {
        self.inner.edge_curve(body, edge)
    }
    fn view_curves(&self, body: BodyId, spec: &drawing::ViewSpec) -> KernelResult<drawing::ViewCurves> {
        let delay = std::mem::take(&mut self.rules.lock().unwrap().drawing_delay);
        std::thread::sleep(delay);
        if spec.preview { drawing::mesh_hlr::view_curves(self, body, spec) } else { self.inner.view_curves(body, spec) }
    }
    fn backend(&self) -> BackendInfo {
        self.inner.backend()
    }
    fn create_body_from_profile(&self, tag: &OpTag, profile: &Profile) -> KernelResult<OpResult> {
        self.inner.create_body_from_profile(tag, profile)
    }
    fn extrude(&self, tag: &OpTag, profile: BodyId, params: &Extrude) -> KernelResult<OpResult> {
        self.check_extrude(params.distance)?;
        self.inner.extrude(tag, profile, params)
    }
    fn extrude_on_face(
        &self,
        tag: &OpTag,
        profile: &Profile,
        support: FaceSupport,
        distance: f64,
    ) -> KernelResult<OpResult> {
        self.check_extrude(distance.abs())?;
        self.inner.extrude_on_face(tag, profile, support, distance)
    }
    fn revolve(&self, tag: &OpTag, profile: BodyId, params: &Revolve) -> KernelResult<OpResult> {
        self.inner.revolve(tag, profile, params)
    }
    fn boolean(&self, tag: &OpTag, op: BooleanOp, target: BodyId, tool: BodyId) -> KernelResult<OpResult> {
        self.inner.boolean(tag, op, target, tool)
    }
    fn fillet(&self, tag: &OpTag, body: BodyId, edges: &[EdgeId], radius: f64) -> KernelResult<OpResult> {
        if self.rules.lock().unwrap().refuse_fillet_radii.contains(&radius) {
            return Err(KernelError::Failed {
                op: OpKind::Fillet,
                code: 0,
                message: format!("the scenario refuses a radius of {radius}"),
            });
        }
        self.inner.fillet(tag, body, edges, radius)
    }
    fn chamfer(&self, tag: &OpTag, body: BodyId, edges: &[EdgeId], distance: f64) -> KernelResult<OpResult> {
        self.inner.chamfer(tag, body, edges, distance)
    }
    fn shell(&self, tag: &OpTag, body: BodyId, open: &[FaceId], thickness: f64) -> KernelResult<OpResult> {
        self.inner.shell(tag, body, open, thickness)
    }
    fn lattice(&self, tag: &OpTag, body: BodyId, params: &Lattice) -> KernelResult<OpResult> {
        self.inner.lattice(tag, body, params)
    }
    fn gyroid(&self, tag: &OpTag, body: BodyId, params: &Gyroid) -> KernelResult<OpResult> {
        self.inner.gyroid(tag, body, params)
    }
    fn export_field_stl(&self, body: BodyId) -> KernelResult<FieldExport> {
        self.inner.export_field_stl(body)
    }
    fn export_field_stl_at(&self, body: BodyId, accuracy_mm: f64) -> KernelResult<FieldExport> {
        self.inner.export_field_stl_at(body, accuracy_mm)
    }
    fn print_mesh(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Option<PrintMesh>> {
        self.inner.print_mesh(body, params)
    }
    fn construction_edit(
        &self,
        body: BodyId,
        edits: &[soliddraft_kernel_api::ScalarEdit],
        params: &TessellationParams,
    ) -> KernelResult<soliddraft_kernel_api::ConstructionEdit> {
        self.inner.construction_edit(body, edits, params)
    }
    fn reserve_proof_work(&self) -> KernelResult<Box<dyn soliddraft_kernel_api::ProofWork>> {
        if self.rules.lock().unwrap().refuse_proof_reservation {
            return Err(KernelError::InvalidInput("This edit could not start its proof. Try the edit again".into()));
        }
        self.inner.reserve_proof_work()
    }
    fn rendering(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Mesh> {
        self.display(body)?;
        self.inner.rendering(body, params)
    }

    fn rendering_edges(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        self.inner.rendering_edges(body, params)
    }

    fn display_bound(&self, _body: BodyId) -> KernelResult<Option<DisplayBound>> {
        Ok(self.rules.lock().unwrap().display_bound.clone())
    }

    fn preview(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Mesh> {
        self.display(body)?;
        self.inner.preview(body, params)
    }
    fn tessellate(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Mesh> {
        self.display(body)?;
        self.inner.tessellate(body, params)
    }
    fn tessellate_edges(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        self.inner.tessellate_edges(body, params)
    }
    fn mass_properties(&self, body: BodyId) -> KernelResult<MassProperties> {
        self.inner.mass_properties(body)
    }
    fn validate(&self, body: BodyId) -> KernelResult<Validation> {
        self.inner.validate(body)
    }
    fn topology(&self, body: BodyId) -> KernelResult<Topology> {
        let above = self.rules.lock().unwrap().refuse_topology_above;
        if above.is_some_and(|limit| self.inner.mass_properties(body).is_ok_and(|m| m.volume > limit)) {
            return Err(KernelError::QueryFailed {
                query: "approximate_display".into(),
                code: 0,
                message: "step.numerical-rounding-topology: the scenario refuses to sample this body".into(),
            });
        }
        self.inner.topology(body)
    }
    fn edge_length(&self, body: BodyId, edge: EdgeId) -> KernelResult<Measurement> {
        self.inner.edge_length(body, edge)
    }
    fn face_area(&self, body: BodyId, face: FaceId) -> KernelResult<Measurement> {
        self.inner.face_area(body, face)
    }
    fn edge_midpoint(&self, body: BodyId, edge: EdgeId) -> KernelResult<PointProof> {
        self.inner.edge_midpoint(body, edge)
    }
    fn measure_pair(
        &self,
        body: BodyId,
        a: PairEndpoint,
        b: PairEndpoint,
        quantity: PairQuantity,
    ) -> KernelResult<Measurement> {
        if self.rules.lock().unwrap().refuse_pairs {
            return Err(KernelError::Unsupported("measure.bounds-incomplete: the scenario refuses pair proofs".into()));
        }
        self.inner.measure_pair(body, a, b, quantity)
    }
    fn face_support_plane(&self, body: BodyId, face: FaceId) -> KernelResult<soliddraft_kernel_api::ExactAxisPlane> {
        self.inner.face_support_plane(body, face)
    }
    fn history(&self, body: BodyId) -> KernelResult<Vec<HistoryRecord>> {
        self.inner.history(body)
    }
    fn export_step(
        &self,
        body: BodyId,
        options: &soliddraft_kernel_api::StepOptions,
    ) -> KernelResult<soliddraft_kernel_api::StepExport> {
        self.inner.export_step(body, options)
    }
    fn release(&self, body: BodyId) -> KernelResult<()> {
        self.inner.release(body)
    }
}
