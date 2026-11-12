use soliddraft_kernel_api::{BodyId, Kernel, PlacedBody, contract};
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
