//! A fillet's overflow policy on the Catallax backend: roll-over builds what the default
//! builds, keep-neighbours with setback stations is stored and refused until Catallax
//! builds it, and bad stations are refused before any kernel call.

use std::sync::Arc;

use soliddraft_kernel_api::{FaceId, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, SolveStatus, solve::solve};
use soliddraft_model::{
    BlendFeature, Candidate, Command, CommandError, DatumPlane, Document, EdgeRef, EndCondition, ExtrudeFeature,
    ExtrudeOp, FeatureId, FeatureKind, FeatureStatus, FilletOverflow, FilletSetback, ProfileSource, ReferenceSlot,
    RepairReason, SketchFeature, SketchPlane,
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

/// The top edge at the back of the block, running along x, and the side faces at its
/// two ends and alongside it.
struct Picks {
    edge: EdgeRef,
    /// The face at the edge's high-x end.
    end: soliddraft_model::FaceRef,
    /// The back face, which runs along the whole edge and meets neither end.
    along: soliddraft_model::FaceRef,
}

fn picks(d: &Document) -> Picks {
    let t = d.kernel().topology(d.body().unwrap()).unwrap();
    let edge = t
        .edges
        .iter()
        .filter(|e| e.midpoint.z > 10.0 - 1e-6)
        .max_by(|a, b| a.midpoint.y.total_cmp(&b.midpoint.y))
        .expect("a top edge");
    let face_facing = |n: Vec3| -> FaceId {
        t.faces.iter().find(|f| f.normal.is_some_and(|m| (m - n).length() < 1e-9)).expect("a side face").id
    };
    Picks {
        edge: d.edge_ref(edge.id).unwrap(),
        end: d.face_ref(face_facing(Vec3::X)).unwrap(),
        along: d.face_ref(face_facing(Vec3::Y)).unwrap(),
    }
}

fn fillet(edge: &EdgeRef, overflow: Option<FilletOverflow>) -> FeatureKind {
    FeatureKind::Fillet(BlendFeature { edges: vec![edge.clone()], size: 2.0, overflow })
}

fn keep(at: &soliddraft_model::FaceRef, setback: f64) -> FilletOverflow {
    FilletOverflow::KeepNeighbours(vec![FilletSetback { edge: 0, at: at.clone(), setback }])
}

#[test]
fn roll_over_builds_what_the_default_builds_and_is_saved() {
    let mut d = block();
    let p = picks(&d);
    let plain = add(&mut d, fillet(&p.edge, None)).unwrap();
    let rounded = volume(&d);
    assert!(rounded < 12000.0, "the fillet removes material: {rounded}");
    d.execute(Command::DeleteFeature { id: plain }).unwrap();

    let f = add(&mut d, fillet(&p.edge, Some(FilletOverflow::RollOver))).unwrap();
    assert_eq!(d.status(f), Some(&FeatureStatus::Ok));
    assert!((volume(&d) - rounded).abs() <= 1e-9 * rounded, "roll-over is the default: {} vs {rounded}", volume(&d));

    let back = Document::from_bytes(&d.to_bytes().unwrap(), d.kernel().clone()).unwrap();
    match &back.feature(f).unwrap().kind {
        FeatureKind::Fillet(b) => assert_eq!(b.overflow, Some(FilletOverflow::RollOver), "the policy is saved"),
        other => panic!("still a fillet: {other:?}"),
    }
}

#[test]
fn keep_neighbours_is_kept_and_refused_until_catallax_builds_it() {
    let mut d = block();
    let p = picks(&d);
    let before = volume(&d);
    let body = d.body();
    let f = add(&mut d, fillet(&p.edge, Some(keep(&p.end, 5.0)))).unwrap();
    match d.status(f) {
        Some(FeatureStatus::Failed(why)) => {
            assert!(why.to_lowercase().contains("coming soon"), "the refusal says it waits: {why}")
        }
        other => panic!("the fillet is refused, not {other:?}"),
    }
    assert_eq!(d.body(), body, "the body passes through unchanged");
    assert!((volume(&d) - before).abs() < 1e-9, "nothing is rolled over in its place");

    // Saving keeps the stations, so the part builds once Catallax does.
    let back = Document::from_bytes(&d.to_bytes().unwrap(), d.kernel().clone()).unwrap();
    match &back.feature(f).unwrap().kind {
        FeatureKind::Fillet(b) => assert_eq!(b.overflow, Some(keep(&p.end, 5.0))),
        other => panic!("still a fillet: {other:?}"),
    }

    // Back to roll-over, the same fillet builds.
    d.execute(Command::EditFeature { id: f, kind: fillet(&p.edge, Some(FilletOverflow::RollOver)) }).unwrap();
    assert_eq!(d.status(f), Some(&FeatureStatus::Ok));
    assert!(volume(&d) < before);
}

#[test]
fn a_station_that_names_no_end_asks_for_repair_and_offers_the_end_faces() {
    let mut d = block();
    let p = picks(&d);
    let f = add(&mut d, fillet(&p.edge, Some(keep(&p.along, 5.0)))).unwrap();
    let Some(FeatureStatus::RepairNeeded(items)) = d.status(f) else {
        panic!("a face along the edge names no end: {:?}", d.status(f));
    };
    let [item] = items.as_slice() else { panic!("one station to repair: {items:?}") };
    assert_eq!(item.slot, ReferenceSlot::Setback { index: 0 });
    assert!(matches!(item.reason, RepairReason::Mismatch { .. }), "{item:?}");
    assert!(item.message.contains("does not meet its edge at exactly one end"), "{}", item.message);
    // The offers are the two faces the edge runs into, one at each end.
    let body = d.body().unwrap();
    let offered: Vec<FaceId> = item
        .candidates
        .iter()
        .map(|c| match c {
            Candidate::Face(face) => *face,
            other => panic!("a station end is a face: {other:?}"),
        })
        .collect();
    let end = d.resolve_face(&p.end, body).unwrap();
    assert_eq!(offered.len(), 2, "{offered:?}");
    assert!(offered.contains(&end), "the end face is offered: {offered:?}");

    // Pointing the station at an offered end face clears the repair.
    d.execute(Command::EditFeature { id: f, kind: fillet(&p.edge, Some(keep(&p.end, 5.0))) }).unwrap();
    assert!(matches!(d.status(f), Some(FeatureStatus::Failed(_))), "it reaches Catallax: {:?}", d.status(f));
}

#[test]
fn a_station_whose_end_face_is_gone_asks_for_repair() {
    let mut d = block();
    let p = picks(&d);
    let mut gone = p.end.clone();
    gone.ordinal += 7;
    gone.label = "Gone face".into();
    let f = add(&mut d, fillet(&p.edge, Some(keep(&gone, 5.0)))).unwrap();
    let Some(FeatureStatus::RepairNeeded(items)) = d.status(f) else {
        panic!("a lost end face asks for repair: {:?}", d.status(f));
    };
    assert_eq!(items.iter().map(|r| r.slot).collect::<Vec<_>>(), vec![ReferenceSlot::Setback { index: 0 }]);
    assert!(items[0].message.starts_with("setback 1: "), "{}", items[0].message);
}

#[test]
fn removing_an_edge_takes_its_stations_and_renumbers_the_rest() {
    let station = |edge: u32| FilletSetback { edge, at: picks(&block()).end, setback: 1.0 };
    let mut o = FilletOverflow::KeepNeighbours(vec![station(0), station(1), station(2), station(1)]);
    o.edge_removed(1);
    let FilletOverflow::KeepNeighbours(left) = &o else { unreachable!() };
    assert_eq!(left.iter().map(|s| s.edge).collect::<Vec<_>>(), vec![0, 1]);
    let mut r = FilletOverflow::RollOver;
    r.edge_removed(0);
    assert_eq!(r, FilletOverflow::RollOver);
}

#[test]
fn bad_stations_and_chamfer_policies_are_refused() {
    let mut d = block();
    let p = picks(&d);
    let invalid = |r: Result<FeatureId, CommandError>| matches!(r, Err(CommandError::Invalid(_)));
    assert!(invalid(add(&mut d, fillet(&p.edge, Some(keep(&p.end, 0.0))))), "a setback is positive");
    assert!(invalid(add(&mut d, fillet(&p.edge, Some(keep(&p.end, f64::NAN))))), "and finite");
    let other_edge = FilletOverflow::KeepNeighbours(vec![FilletSetback { edge: 1, at: p.end.clone(), setback: 5.0 }]);
    assert!(invalid(add(&mut d, fillet(&p.edge, Some(other_edge)))), "a station names one of the fillet's edges");
    let chamfer = FeatureKind::Chamfer(BlendFeature {
        edges: vec![p.edge.clone()],
        size: 1.0,
        overflow: Some(FilletOverflow::RollOver),
    });
    assert!(invalid(add(&mut d, chamfer)), "a chamfer has no overflow policy");
    // An empty station list is a valid policy: a closed run has no ends.
    assert!(add(&mut d, fillet(&p.edge, Some(FilletOverflow::KeepNeighbours(vec![])))).is_ok());
    assert_eq!(d.features().len(), 3);
}
