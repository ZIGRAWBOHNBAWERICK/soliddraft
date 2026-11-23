//! A fillet refused because it would run into other faces names them as the tree does.

use soliddraft_kernel_api::*;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::*;
use std::sync::Arc;

/// Catallax, except that every fillet runs into the block's top face.
struct Reaching(CatallaxKernel);

impl Reaching {
    fn top(&self, body: BodyId) -> FaceId {
        let t = self.0.topology(body).unwrap();
        t.faces.iter().find(|f| (f.centroid.z - 10.0).abs() < 1e-3).unwrap().id
    }
}

impl Kernel for Reaching {
    fn backend(&self) -> BackendInfo {
        self.0.backend()
    }
    fn create_body_from_profile(&self, tag: &OpTag, profile: &Profile) -> KernelResult<OpResult> {
        self.0.create_body_from_profile(tag, profile)
    }
    fn extrude(&self, tag: &OpTag, profile: BodyId, params: &Extrude) -> KernelResult<OpResult> {
        self.0.extrude(tag, profile, params)
    }
    fn revolve(&self, tag: &OpTag, profile: BodyId, params: &Revolve) -> KernelResult<OpResult> {
        self.0.revolve(tag, profile, params)
    }
    fn boolean(&self, tag: &OpTag, op: BooleanOp, target: BodyId, tool: BodyId) -> KernelResult<OpResult> {
        self.0.boolean(tag, op, target, tool)
    }
    fn fillet(&self, _tag: &OpTag, body: BodyId, _edges: &[EdgeId], _radius: f64) -> KernelResult<OpResult> {
        Err(KernelError::FilletReaches {
            faces: vec![self.top(body)],
            between: false,
            radius_limit_mm: Some(4.0),
            message: "fillet.radius-too-large: the ball reaches a face; radius limit 4 mm".into(),
        })
    }
    fn chamfer(&self, tag: &OpTag, body: BodyId, edges: &[EdgeId], distance: f64) -> KernelResult<OpResult> {
        self.0.chamfer(tag, body, edges, distance)
    }
    fn shell(&self, tag: &OpTag, body: BodyId, open_faces: &[FaceId], thickness: f64) -> KernelResult<OpResult> {
        self.0.shell(tag, body, open_faces, thickness)
    }
    fn tessellate(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Mesh> {
        self.0.tessellate(body, params)
    }
    fn tessellate_edges(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        self.0.tessellate_edges(body, params)
    }
    fn mass_properties(&self, body: BodyId) -> KernelResult<MassProperties> {
        self.0.mass_properties(body)
    }
    fn validate(&self, body: BodyId) -> KernelResult<Validation> {
        self.0.validate(body)
    }
    fn topology(&self, body: BodyId) -> KernelResult<Topology> {
        self.0.topology(body)
    }
    fn history(&self, body: BodyId) -> KernelResult<Vec<HistoryRecord>> {
        self.0.history(body)
    }
    fn release(&self, body: BodyId) -> KernelResult<()> {
        self.0.release(body)
    }
}

fn add(d: &mut Document, kind: FeatureKind) -> FeatureId {
    d.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false }).unwrap().created.unwrap()
}

#[test]
fn a_fillet_that_reaches_a_face_names_it_in_its_status_and_its_preview() {
    let kernel = Arc::new(Reaching(CatallaxKernel::new()));
    let mut d = Document::new("Reaching", kernel.clone());
    let mut sketch = sketch::Sketch::new();
    sketch.rectangle(sketch::P2::new(0.0, 0.0), 40.0, 30.0).unwrap();
    sketch::solve::solve(&mut sketch, None, true);
    let s = add(
        &mut d,
        FeatureKind::Sketch(SketchFeature {
            plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
            sketch,
        }),
    );
    add(
        &mut d,
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: s },
            operation: ExtrudeOp::Boss,
            end: EndCondition::Blind { depth: 10.0 },
            reverse: false,
            options: Default::default(),
        }),
    );
    let body = d.body().unwrap();
    let top = d.face_label(kernel.top(body)).unwrap().text;
    assert_eq!(top, "Boss-Extrude1 · End cap");
    let t = d.kernel().topology(body).unwrap();
    let edge = t.edges.iter().find(|e| e.midpoint.z > 10.0 - 1e-6).unwrap();
    let blend = BlendFeature { edges: vec![d.edge_ref(edge.id).unwrap()], size: 5.0, other_distance: None };

    let preview = d.round(&OpTag::new("preview"), &blend, false, &[edge.id], body).unwrap_err();
    assert!(preview.ends_with(&format!("{RUNS_INTO}{top}")), "{preview}");
    assert!(preview.contains("radius limit 4 mm"), "{preview}");

    let fillet = add(&mut d, FeatureKind::Fillet(blend));
    let Some(FeatureStatus::Failed(status)) = d.status(fillet) else { panic!("{:?}", d.status(fillet)) };
    assert_eq!(status, &preview, "the feature and its preview say the same");
}
