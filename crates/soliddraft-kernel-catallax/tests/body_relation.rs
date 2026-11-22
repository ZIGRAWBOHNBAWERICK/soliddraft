//! Body-pair proofs preserve their sources and refuse exhausted requests.

use soliddraft_kernel_api::{
    BodyId, BooleanOp, Certainty, EdgeId, Extrude, FieldFrame, Grade, Kernel, Lattice, LatticeCell, OpTag, PlacedBody,
    Plane, Point2, Profile, Vec3, contract,
};
use soliddraft_kernel_catallax::{CatallaxKernel, with_request_allowance};

fn placed(body: BodyId, z: f64) -> PlacedBody {
    PlacedBody { body, translation: [0.0, 0.0, z], rotation: [0.0, 0.0, 0.0, 1.0] }
}

#[test]
fn empty_touching_and_positive_intersections_keep_the_source_bodies() {
    let k = CatallaxKernel::new();
    let a = contract::block(&k);
    let b = contract::block(&k);
    let before = [k.body_recipe(a).unwrap(), k.body_recipe(b).unwrap()];
    for (z, expected, area_contact) in [(10.0, 0.0, true), (9.0, 1200.0, true), (10.015625, 0.0, false)] {
        let report = k.body_relation(placed(a, 0.0), placed(b, z), Some(0.015625)).unwrap();
        let volume = report.overlap.unwrap();
        assert!(volume.volume.lo <= expected && volume.volume.hi >= expected, "{volume:?}");
        assert_eq!(volume.positive, Some(expected > 0.0));
        let faces = report.contact.unwrap();
        assert_eq!(!faces.first.is_empty(), area_contact);
        assert_eq!(!faces.second.is_empty(), area_contact);
        let gap = report.gap.unwrap().unwrap();
        assert_eq!(gap.zero, z <= 10.0);
        assert_eq!(gap.exceeds_clearance, Some(false), "{gap:?}");
        if z > 10.0 {
            assert_eq!((gap.distance.lo, gap.distance.hi), (0.015625, 0.015625));
        }
    }
    assert_eq!([k.body_recipe(a).unwrap(), k.body_recipe(b).unwrap()], before);
    assert_eq!(k.mass_properties(a).unwrap().volume, 12000.0);
    assert_eq!(k.mass_properties(b).unwrap().volume, 12000.0);
}

#[test]
fn exact_placement_keeps_quarter_turns_and_small_gaps() {
    let k = CatallaxKernel::new();
    let a = contract::block(&k);
    let mut b = placed(a, 10.0001);
    // Any two equal nonzero components encode an exact quarter turn.
    b.rotation = [0.0, 0.0, 0.7, 0.7];
    let report = k.body_relation(placed(a, 0.0), b, Some(0.00005)).unwrap();
    assert_eq!(report.overlap.unwrap().positive, Some(false));
    let gap = report.gap.unwrap().unwrap();
    assert!(gap.distance.lo > 0.00005 && gap.distance.hi < 0.00015);
    assert_eq!(gap.exceeds_clearance, Some(true));
}

#[test]
fn an_exhausted_query_refuses_and_preserves_both_bodies() {
    let k = CatallaxKernel::new();
    let a = contract::block(&k);
    let b = contract::block(&k);
    let before = [k.body_recipe(a).unwrap(), k.body_recipe(b).unwrap()];
    let (answer, work) = with_request_allowance(0, || k.body_relation(placed(a, 0.0), placed(b, 10.0), None));
    assert!(work.exhausted);
    match answer {
        Err(error) => assert!(error.to_string().contains("budget"), "{error}"),
        Ok(report) => {
            assert_eq!(report.overlap.unwrap_err().code, "numbers.operation-budget");
            assert_eq!(report.contact.unwrap_err().code, "numbers.operation-budget");
        }
    }
    assert_eq!([k.body_recipe(a).unwrap(), k.body_recipe(b).unwrap()], before);
}

/// Catallax's P3, built as Catallax's `parts/P3-filleted-boss.json` builds it: a 40 x 30
/// x 10 block and a boss of radius 8 and height 12 at (20, 15) joined, the junction
/// filleted 2 mm, then the block's four top edges rounded 1 mm. Its torus fillets make
/// pairs between copies that the overlap Boolean cannot intersect.
fn p3(k: &CatallaxKernel) -> BodyId {
    let tag = OpTag::new;
    let block = k
        .create_body_from_profile(
            &tag("block_sketch"),
            &Profile::rectangle(Plane::XY, Point2::new(20.0, 15.0), 40.0, 30.0),
        )
        .unwrap();
    let block = k.extrude(&tag("block"), block.body, &Extrude { distance: 10.0, direction: None }).unwrap().body;
    let boss = k
        .create_body_from_profile(
            &tag("boss_sketch"),
            &Profile::circle(Plane::XY.with_origin(Vec3::new(20.0, 15.0, 10.0)), Point2::new(0.0, 0.0), 8.0),
        )
        .unwrap();
    let boss = k.extrude(&tag("boss"), boss.body, &Extrude { distance: 12.0, direction: None }).unwrap().body;
    let joined = k.boolean(&tag("joined"), BooleanOp::Union, block, boss).unwrap();
    let junction = joined.record.edges.created.clone();
    let rounded = k.fillet(&tag("junction"), joined.body, &junction, 2.0).unwrap().body;
    // The block's top edges: lines at z = 10 along its sides.
    let top: Vec<EdgeId> = k
        .topology(rounded)
        .unwrap()
        .edges
        .iter()
        .filter(|e| {
            let m = e.midpoint;
            (m.z - 10.0).abs() < 1e-9
                && (m.x.abs() < 1e-9 || (m.x - 40.0).abs() < 1e-9 || m.y.abs() < 1e-9 || (m.y - 30.0).abs() < 1e-9)
        })
        .map(|e| e.id)
        .collect();
    assert_eq!(top.len(), 4, "the block's top edges");
    k.fillet(&tag("rounded"), rounded, &top, 1.0).unwrap().body
}

fn along_x(body: BodyId, x: f64) -> PlacedBody {
    PlacedBody { body, translation: [x, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0] }
}

#[test]
fn copies_one_millimetre_apart_are_clear_and_a_clearance_of_exactly_that_is_not_exceeded() {
    let k = CatallaxKernel::new();
    let a = p3(&k);
    for (clearance, exceeds) in [(1.0, Some(false)), (0.5, Some(true)), (1.5, Some(false))] {
        let report = k.body_relation(along_x(a, 0.0), along_x(a, 41.0), Some(clearance)).unwrap();
        assert_eq!(report.overlap.as_ref().unwrap().positive, Some(false), "{report:?}");
        let gap = report.gap.as_ref().unwrap().as_ref().unwrap();
        let d = &gap.distance;
        assert!(d.lo <= 1.0 && d.hi == 1.0 && d.hi - d.lo < 1e-12 && !gap.zero, "{gap:?}");
        // A gap at most the clearance does not exceed it, equality included.
        assert_eq!(gap.exceeds_clearance, exceeds, "clearance {clearance}: {gap:?}");
    }
}

#[test]
fn a_proved_overlap_is_a_clash_however_wide_its_volume_and_an_open_pair_is_never_clear() {
    let k = CatallaxKernel::new();
    let a = p3(&k);
    // Copies overlapping by half: the Boolean refuses, a ball inside both proves the
    // clash, and the volume lies between the ball's and the boxes' overlap.
    let report = k.body_relation(along_x(a, 0.0), along_x(a, 20.0), None).unwrap();
    let overlap = report.overlap.as_ref().unwrap();
    assert_eq!(overlap.positive, Some(true), "{report:?}");
    assert!(overlap.volume.lo > 0.0 && overlap.volume.hi > 2.0 * overlap.volume.lo, "{overlap:?}");
    assert_eq!(overlap.volume.certainty, Certainty::Bounded);
    let gap = report.gap.as_ref().unwrap().as_ref().unwrap();
    assert!(gap.zero && gap.distance.hi == 0.0, "{gap:?}");

    // Copies touching along their end faces: whatever Catallax settles, the pair never
    // reads as clear.
    let report = k.body_relation(along_x(a, 0.0), along_x(a, 40.0), Some(0.5)).unwrap();
    eprintln!("touching copies: {report:?}");
    assert_ne!(report.overlap.as_ref().ok().and_then(|o| o.positive), Some(true), "{report:?}");
    if let Some(Ok(gap)) = &report.gap {
        assert!(gap.distance.lo <= 0.0 && gap.exceeds_clearance != Some(true), "{report:?}");
    }
}

#[test]
fn a_lattice_part_is_never_related_through_its_envelope() {
    let k = CatallaxKernel::new();
    let region = contract::block(&k);
    let lattice = k
        .lattice(
            &OpTag::new("lattice"),
            region,
            &Lattice {
                cell: LatticeCell::Cubic,
                cell_size_mm: 100.0,
                radius_mm: Grade::Constant(1.0),
                frame: FieldFrame::Part,
                accuracy_mm: 0.1,
                min_detail_mm: 0.1,
            },
        )
        .unwrap()
        .body;
    let other = contract::block(&k);
    for (a, b) in [(lattice, other), (other, lattice)] {
        let report = k.body_relation(placed(a, 0.0), placed(b, 50.0), Some(1.0)).unwrap();
        assert_eq!(report.overlap.unwrap_err().code, "app.relation-envelope");
        assert_eq!(report.contact.unwrap_err().code, "app.relation-envelope");
        assert_eq!(report.gap.unwrap().unwrap_err().code, "app.relation-envelope");
    }
    // The region itself is an ordinary solid.
    let report = k.body_relation(placed(region, 0.0), placed(other, 50.0), None).unwrap();
    assert_eq!(report.overlap.unwrap().positive, Some(false));
}
