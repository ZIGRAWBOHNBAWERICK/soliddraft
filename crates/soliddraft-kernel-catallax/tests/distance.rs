//! Exact offsets and selected-face thickening through the application kernel API.
use soliddraft_kernel_api::*;
use soliddraft_kernel_catallax::CatallaxKernel;

fn block(k: &CatallaxKernel) -> BodyId {
    let profile = k
        .create_body_from_profile(&OpTag::new("profile"), &Profile::rectangle(Plane::XY, [0.0, 0.0].into(), 10.0, 8.0))
        .unwrap()
        .body;
    k.extrude(&OpTag::new("stock"), profile, &Extrude { distance: 6.0, direction: None }).unwrap().body
}

#[test]
fn inward_offset_uses_the_exact_result_and_keeps_its_source() {
    let k = CatallaxKernel::new();
    let source = block(&k);
    let result = k.offset(&OpTag::new("offset"), source, -1.0).unwrap();
    assert_eq!(result.record.op, OpKind::Offset);
    assert!((k.mass_properties(result.body).unwrap().volume - 192.0).abs() < 1e-8);
    let topology = k.topology(result.body).unwrap();
    assert_eq!((topology.faces.len(), topology.edges.len()), (6, 12));
    assert!(topology.faces.iter().all(|f| !f.origins.is_empty()));
    assert!(k.export_step(result.body, &StepOptions::default()).unwrap().text.contains("MANIFOLD_SOLID_BREP"));
    assert!(k.offset(&OpTag::new("zero"), source, 0.0).unwrap_err().to_string().contains("nonzero distance"));
    let consumed = k.offset(&OpTag::new("consumed"), source, -10.0).unwrap_err();
    assert!(consumed.to_string().contains("remove the whole body"), "{consumed}");
    assert!((k.mass_properties(source).unwrap().volume - 480.0).abs() < 1e-8);
}

#[test]
fn planar_thickening_builds_the_requested_side_and_names_its_rims() {
    let k = CatallaxKernel::new();
    let source = block(&k);
    let face = k.topology(source).unwrap().faces.iter().find(|f| f.normal == Some(Vec3::Z)).unwrap().id;
    for (side, min, max) in [(ThickenSide::Front, 6.0, 8.0), (ThickenSide::Back, 4.0, 6.0)] {
        let result =
            k.thicken(&OpTag::new("layer"), source, &Thicken { faces: vec![face], thickness: 2.0, side }).unwrap();
        assert_eq!(result.record.op, OpKind::Thicken);
        assert!((k.mass_properties(result.body).unwrap().volume - 160.0).abs() < 1e-8);
        let topology = k.topology(result.body).unwrap();
        assert!((topology.bbox.min.z - min).abs() < 1e-8);
        assert!((topology.bbox.max.z - max).abs() < 1e-8);
        assert!(topology.faces.iter().all(|f| !f.origins.is_empty()), "{topology:?}");
        assert!(result.record.face_roles.iter().any(|f| matches!(f.role, FaceRole::Rim { .. })));
        assert!(k.export_step(result.body, &StepOptions::default()).is_ok());
    }
    assert!(
        k.thicken(&OpTag::new("empty"), source, &Thicken { faces: vec![], thickness: 2.0, side: ThickenSide::Front })
            .unwrap_err()
            .to_string()
            .contains("Select at least one face")
    );
    assert!((k.mass_properties(source).unwrap().volume - 480.0).abs() < 1e-8);
}
