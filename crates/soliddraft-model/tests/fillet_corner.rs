//! A fillet's corner choice on the Catallax backend: cliff builds what the default
//! builds, smooth is stored and refused until Catallax builds it, and choices a fillet
//! cannot have are refused before any kernel call.

use std::sync::Arc;

use soliddraft_kernel_api::{OpTag, RadiusLaw};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, SolveStatus, solve::solve};
use soliddraft_model::{
    BlendFeature, Command, CommandError, DatumPlane, Document, EdgeRef, EndCondition, ExtrudeFeature, ExtrudeOp,
    FeatureId, FeatureKind, FeatureStatus, FilletCorner, FilletLaw, ProfileSource, SketchFeature, SketchPlane,
};

/// A 40 x 30 x 10 block on Top.
fn block() -> Document {
    let mut d = Document::new("Block", Arc::new(CatallaxKernel::new()));
    let mut s = Sketch::new();
    s.rectangle(P2::new(0.0, 0.0), 40.0, 30.0).unwrap();
    solve(&mut s, None, true);
    assert_eq!(s.status, SolveStatus::FullyDefined);
    let sketch = add(
        &mut d,
        FeatureKind::Sketch(SketchFeature {
            plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
            sketch: s,
        }),
    )
    .unwrap();
    add(
        &mut d,
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: sketch },
            operation: ExtrudeOp::Boss,
            end: EndCondition::Blind { depth: 10.0 },
            reverse: false,
        }),
    )
    .unwrap();
    d
}

fn add(d: &mut Document, kind: FeatureKind) -> Result<FeatureId, CommandError> {
    Ok(d.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })?.created.unwrap())
}

fn volume(d: &Document) -> f64 {
    d.kernel().mass_properties(d.body().unwrap()).unwrap().volume
}

/// The back top edge and the high-x top edge, which meet at a corner whose third edge,
/// the vertical one, is left sharp: a partly filleted corner.
fn corner_edges(d: &Document) -> Vec<EdgeRef> {
    let t = d.kernel().topology(d.body().unwrap()).unwrap();
    let top = || t.edges.iter().filter(|e| e.midpoint.z > 10.0 - 1e-6);
    let back = top().max_by(|a, b| a.midpoint.y.total_cmp(&b.midpoint.y)).expect("the back top edge");
    let side = top().max_by(|a, b| a.midpoint.x.total_cmp(&b.midpoint.x)).expect("the high-x top edge");
    vec![d.edge_ref(back.id).unwrap(), d.edge_ref(side.id).unwrap()]
}

fn fillet(edges: &[EdgeRef], laws: Vec<FilletLaw>, corner: Option<FilletCorner>) -> FeatureKind {
    FeatureKind::Fillet(BlendFeature { edges: edges.to_vec(), size: 2.0, overflow: None, laws, corner })
}

#[test]
fn cliff_builds_what_the_default_builds_and_is_saved() {
    let mut d = block();
    let edges = corner_edges(&d);
    let plain = add(&mut d, fillet(&edges, Vec::new(), None)).unwrap();
    assert_eq!(d.status(plain), Some(&FeatureStatus::Ok));
    let rounded = volume(&d);
    assert!(rounded < 12000.0, "the fillet removes material: {rounded}");
    d.execute(Command::DeleteFeature { id: plain }).unwrap();

    let f = add(&mut d, fillet(&edges, Vec::new(), Some(FilletCorner::Cliff))).unwrap();
    assert_eq!(d.status(f), Some(&FeatureStatus::Ok));
    assert!((volume(&d) - rounded).abs() <= 1e-9 * rounded, "cliff is the default: {} vs {rounded}", volume(&d));

    let back = Document::from_bytes(&d.to_bytes().unwrap(), d.kernel().clone()).unwrap();
    match &back.feature(f).unwrap().kind {
        FeatureKind::Fillet(b) => assert_eq!(b.corner, Some(FilletCorner::Cliff), "the choice is saved"),
        other => panic!("still a fillet: {other:?}"),
    }
}

#[test]
fn smooth_is_kept_and_refused_until_catallax_builds_it() {
    let mut d = block();
    let edges = corner_edges(&d);
    let before = volume(&d);
    let body = d.body();
    let f = add(&mut d, fillet(&edges, Vec::new(), Some(FilletCorner::Smooth))).unwrap();
    match d.status(f) {
        Some(FeatureStatus::Failed(why)) => {
            assert!(why.to_lowercase().contains("coming soon"), "the refusal says it waits: {why}")
        }
        other => panic!("the fillet is refused, not {other:?}"),
    }
    assert_eq!(d.body(), body, "the body passes through unchanged");
    assert!((volume(&d) - before).abs() < 1e-9, "no cliff is built in its place");

    // Saving keeps the choice, so the part builds once Catallax does.
    let back = Document::from_bytes(&d.to_bytes().unwrap(), d.kernel().clone()).unwrap();
    match &back.feature(f).unwrap().kind {
        FeatureKind::Fillet(b) => assert_eq!(b.corner, Some(FilletCorner::Smooth)),
        other => panic!("still a fillet: {other:?}"),
    }

    // Back to cliff, the same fillet builds.
    d.execute(Command::EditFeature { id: f, kind: fillet(&edges, Vec::new(), Some(FilletCorner::Cliff)) }).unwrap();
    assert_eq!(d.status(f), Some(&FeatureStatus::Ok));
    assert!(volume(&d) < before);
}

#[test]
fn corner_choices_a_fillet_cannot_have_are_refused() {
    let mut d = block();
    let edges = corner_edges(&d);
    let ends = {
        let t = d.kernel().topology(d.body().unwrap()).unwrap();
        let high_x = t
            .faces
            .iter()
            .find(|f| f.normal.is_some_and(|n| (n - soliddraft_kernel_api::Vec3::X).length() < 1e-9))
            .expect("the high-x face");
        d.face_ref(high_x.id).unwrap()
    };
    let law = || vec![FilletLaw { edge: 0, from: ends.clone(), radius: RadiusLaw::Linear { start: 1.0, end: 3.0 } }];
    let invalid = |r: Result<FeatureId, CommandError>| match r {
        Err(CommandError::Invalid(why)) => why,
        other => panic!("refused as invalid, not {other:?}"),
    };

    let chamfer = FeatureKind::Chamfer(BlendFeature {
        edges: edges.clone(),
        size: 1.0,
        overflow: None,
        laws: Vec::new(),
        corner: Some(FilletCorner::Cliff),
    });
    assert_eq!(invalid(add(&mut d, chamfer)), "a chamfer has no corner choice");
    let why = invalid(add(&mut d, fillet(&edges, law(), Some(FilletCorner::Smooth))));
    assert!(why.contains("closes its corners as cliffs"), "a variable fillet has no smooth corners: {why}");
    assert_eq!(d.features().len(), 2, "nothing was added");

    // Cliff is what a variable fillet builds, so naming it is valid; the laws still wait.
    let f = add(&mut d, fillet(&edges, law(), Some(FilletCorner::Cliff))).unwrap();
    assert!(matches!(d.status(f), Some(FeatureStatus::Failed(why)) if why.contains("coming soon")), "{:?}", d.status(f));

    // A form previews through the same call, so it refuses what a saved file could hold.
    let body = d.body().unwrap();
    let ids: Vec<_> = edges.iter().map(|e| d.resolve_edge(e, body).unwrap()).collect();
    let FeatureKind::Fillet(smooth) = fillet(&edges, law(), Some(FilletCorner::Smooth)) else { unreachable!() };
    let why = d.round(&OpTag::new("preview"), &smooth, false, &ids, body).map(|r| r.body).unwrap_err();
    assert!(why.contains("closes its corners as cliffs"), "{why}");
}
