//! A fillet's variable-radius laws on the Catallax backend: each law starts at the end its
//! face names, edges without one keep the constant radius, the fillet is stored and refused
//! until Catallax builds it, and bad laws are refused before any kernel call.

use std::sync::Arc;

use soliddraft_kernel_api::{FaceId, RadiusLaw, VariableFilletEdge, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, SolveStatus, solve::solve};
use soliddraft_model::{
    BlendFeature, Candidate, Command, CommandError, DatumPlane, Document, EdgeRef, EndCondition, ExtrudeFeature,
    ExtrudeOp, FaceRef, FeatureId, FeatureKind, FeatureStatus, FilletLaw, FilletOverflow, FilletSetback, ProfileSource,
    ReferenceSlot, RepairReason, SketchFeature, SketchPlane,
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

/// The two top edges that run along x, at the back and the front of the block, and the
/// side faces at their high-x end and along the back edge.
struct Picks {
    back: EdgeRef,
    front: EdgeRef,
    /// The face at x = 40, which both edges run into at their high-x end.
    high_x: FaceRef,
    /// The back face, which runs along the whole back edge and meets neither end.
    along: FaceRef,
}

fn picks(d: &Document) -> Picks {
    let t = d.kernel().topology(d.body().unwrap()).unwrap();
    let along_x = || {
        t.edges.iter().filter(|e| {
            e.midpoint.z > 10.0 - 1e-6 && (e.bbox.max.x - e.bbox.min.x) > 39.0 && (e.bbox.max.y - e.bbox.min.y) < 1e-6
        })
    };
    let back = along_x().max_by(|a, b| a.midpoint.y.total_cmp(&b.midpoint.y)).expect("the back top edge");
    let front = along_x().min_by(|a, b| a.midpoint.y.total_cmp(&b.midpoint.y)).expect("the front top edge");
    let face_facing = |n: Vec3| -> FaceId {
        t.faces.iter().find(|f| f.normal.is_some_and(|m| (m - n).length() < 1e-9)).expect("a side face").id
    };
    Picks {
        back: d.edge_ref(back.id).unwrap(),
        front: d.edge_ref(front.id).unwrap(),
        high_x: d.face_ref(face_facing(Vec3::X)).unwrap(),
        along: d.face_ref(face_facing(Vec3::Y)).unwrap(),
    }
}

fn law(edge: u32, from: &FaceRef, start: f64, end: f64) -> FilletLaw {
    FilletLaw { edge, from: from.clone(), radius: RadiusLaw::Linear { start, end } }
}

fn fillet(edges: &[&EdgeRef], laws: Vec<FilletLaw>) -> FeatureKind {
    FeatureKind::Fillet(BlendFeature {
        edges: edges.iter().map(|e| (*e).clone()).collect(),
        size: 2.0,
        overflow: None,
        laws,
    })
}

#[test]
fn each_law_starts_at_its_named_end_and_unlawed_edges_keep_the_radius() {
    let d = block();
    let p = picks(&d);
    let body = d.body().unwrap();
    let edges = [d.resolve_edge(&p.back, body).unwrap(), d.resolve_edge(&p.front, body).unwrap()];
    let t = d.kernel().topology(body).unwrap();
    let placed = d.resolve_variable(&[law(0, &p.high_x, 1.0, 3.0)], &edges, 2.0, body).unwrap();
    let [lawed, constant]: [VariableFilletEdge; 2] = placed.try_into().unwrap();

    // The law runs on the back edge from its x = 40 end, whatever the edge's own direction.
    assert_eq!(lawed.edge, edges[0]);
    let from = t.vertex(lawed.from).unwrap().position;
    assert!((from.x - 40.0).abs() < 1e-9, "the law starts at the high-x end: {from:?}");
    assert_eq!(lawed.radius, RadiusLaw::Linear { start: 1.0, end: 3.0 });

    // The front edge has no law: it keeps R2 from one of its own ends.
    assert_eq!(constant.edge, edges[1]);
    assert!(t.edge(edges[1]).unwrap().vertices.contains(&constant.from));
    assert_eq!(constant.radius, RadiusLaw::Linear { start: 2.0, end: 2.0 });
}

#[test]
fn laws_are_kept_and_refused_until_catallax_builds_them() {
    let mut d = block();
    let p = picks(&d);
    let before = volume(&d);
    let body = d.body();
    let f = add(&mut d, fillet(&[&p.back], vec![law(0, &p.high_x, 1.0, 3.0)])).unwrap();
    match d.status(f) {
        Some(FeatureStatus::Failed(why)) => {
            assert!(why.to_lowercase().contains("coming soon"), "the refusal says it waits: {why}")
        }
        other => panic!("the fillet is refused, not {other:?}"),
    }
    assert_eq!(d.body(), body, "the body passes through unchanged");
    assert!((volume(&d) - before).abs() < 1e-9, "no constant radius is built in its place");

    // Saving keeps the laws, so the part builds once Catallax does.
    let back = Document::from_bytes(&d.to_bytes().unwrap(), d.kernel().clone()).unwrap();
    match &back.feature(f).unwrap().kind {
        FeatureKind::Fillet(b) => assert_eq!(b.laws, vec![law(0, &p.high_x, 1.0, 3.0)]),
        other => panic!("still a fillet: {other:?}"),
    }

    // Without its laws, the same fillet builds at its constant radius.
    d.execute(Command::EditFeature { id: f, kind: fillet(&[&p.back], Vec::new()) }).unwrap();
    assert_eq!(d.status(f), Some(&FeatureStatus::Ok));
    assert!(volume(&d) < before);
}

#[test]
fn a_law_from_a_face_along_its_edge_asks_for_repair() {
    let mut d = block();
    let p = picks(&d);
    let f = add(&mut d, fillet(&[&p.back], vec![law(0, &p.along, 1.0, 3.0)])).unwrap();
    let Some(FeatureStatus::RepairNeeded(items)) = d.status(f) else {
        panic!("a face along the edge names no end: {:?}", d.status(f));
    };
    let [item] = items.as_slice() else { panic!("one law to repair: {items:?}") };
    assert_eq!(item.slot, ReferenceSlot::Law { index: 0 });
    assert!(matches!(item.reason, RepairReason::Mismatch { .. }), "{item:?}");
    assert!(item.message.starts_with("radius law 1: "), "{}", item.message);
    let body = d.body().unwrap();
    let high_x = d.resolve_face(&p.high_x, body).unwrap();
    assert_eq!(item.candidates.len(), 2, "one face at each end: {:?}", item.candidates);
    assert!(item.candidates.contains(&Candidate::Face(high_x)));
}

#[test]
fn bad_laws_and_policies_are_refused() {
    let mut d = block();
    let p = picks(&d);
    let invalid = |r: Result<FeatureId, CommandError>| matches!(r, Err(CommandError::Invalid(_)));
    let one = |l: FilletLaw| fillet(&[&p.back], vec![l]);
    assert!(invalid(add(&mut d, one(law(0, &p.high_x, -1.0, 3.0)))), "radii are not negative");
    assert!(invalid(add(&mut d, one(law(0, &p.high_x, 0.0, 0.0)))), "and not both zero");
    assert!(invalid(add(&mut d, one(law(0, &p.high_x, f64::NAN, 3.0)))), "and finite");
    assert!(invalid(add(&mut d, one(law(1, &p.high_x, 1.0, 3.0)))), "a law names one of the fillet's edges");
    let twice = fillet(&[&p.back], vec![law(0, &p.high_x, 1.0, 3.0), law(0, &p.high_x, 2.0, 3.0)]);
    assert!(invalid(add(&mut d, twice)), "one law per edge");
    let keep = FeatureKind::Fillet(BlendFeature {
        edges: vec![p.back.clone()],
        size: 2.0,
        overflow: Some(FilletOverflow::KeepNeighbours(Vec::new())),
        laws: vec![law(0, &p.high_x, 1.0, 3.0)],
    });
    assert!(invalid(add(&mut d, keep)), "a variable fillet rolls over");
    let chamfer = FeatureKind::Chamfer(BlendFeature {
        edges: vec![p.back.clone()],
        size: 1.0,
        overflow: None,
        laws: vec![law(0, &p.high_x, 1.0, 3.0)],
    });
    assert!(invalid(add(&mut d, chamfer)), "a chamfer has no radius laws");
    assert_eq!(d.features().len(), 2, "nothing was added");

    // A zero end is an exact collapsed end, so it is a valid law.
    assert!(add(&mut d, one(law(0, &p.high_x, 0.0, 3.0))).is_ok());
}

#[test]
fn removing_an_edge_takes_its_law_and_stations() {
    let p = picks(&block());
    let station = |edge: u32| FilletSetback { edge, at: p.high_x.clone(), setback: 1.0 };
    let mut b = BlendFeature {
        edges: vec![p.back.clone(), p.front.clone(), p.back.clone()],
        size: 2.0,
        overflow: Some(FilletOverflow::KeepNeighbours(vec![station(0), station(1), station(2)])),
        laws: vec![law(2, &p.high_x, 1.0, 3.0), law(1, &p.high_x, 2.0, 4.0)],
    };
    b.remove_edge(1);
    assert_eq!(b.edges.len(), 2);
    assert_eq!(b.stations().iter().map(|s| s.edge).collect::<Vec<_>>(), vec![0, 1]);
    assert_eq!(b.laws, vec![law(1, &p.high_x, 1.0, 3.0)]);
    b.remove_edge(5);
    assert_eq!(b.edges.len(), 2, "an index past the list changes nothing");
}
