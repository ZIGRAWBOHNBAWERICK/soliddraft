//! Bodies Catallax builds within a stated tolerance: retained, displayed, picked,
//! measured and written as approximate, never refused or passed off as exact.
use soliddraft_kernel_api::*;
use soliddraft_kernel_catallax::CatallaxKernel;

/// A 10 x 8 x 6 mm block whose extrusion comes back as a numerical body.
fn numerical_block(k: &CatallaxKernel) -> OpResult {
    let profile = k
        .create_body_from_profile(&OpTag::new("profile"), &Profile::rectangle(Plane::XY, [0.0, 0.0].into(), 10.0, 8.0))
        .unwrap()
        .body;
    k.extrude(&OpTag::new("stock"), profile, &Extrude { distance: 6.0, direction: None }).unwrap()
}

#[test]
fn a_numerical_body_is_kept_and_labelled_approximate() {
    let k = CatallaxKernel::with_numerical_stand_in(&[OpKind::Extrude]);
    let block = numerical_block(&k);
    let body = block.body;
    assert_eq!(k.body_accuracy(body).unwrap(), BodyAccuracy::Approximate { tolerance_mm: "0.01".into() });
    assert_eq!(k.body_kind(body).unwrap(), BodyKind::Solid);
    let validation = k.validate(body).unwrap();
    assert!(validation.valid && validation.manifold_solid, "{validation:?}");
    assert_eq!(block.record.op, OpKind::Extrude);
    assert_eq!(block.record.faces.created.len(), 6);
    let roles: Vec<_> = block.record.face_roles.iter().map(|f| &f.role).collect();
    assert!(roles.contains(&&FaceRole::StartCap) && roles.contains(&&FaceRole::EndCap), "{roles:?}");
    assert_eq!(k.history(body).unwrap().last(), Some(&block.record));

    // An exact build on the same backend stays exact.
    let exact = CatallaxKernel::with_numerical_stand_in(&[OpKind::Fillet]);
    assert_eq!(exact.body_accuracy(numerical_block(&exact).body).unwrap(), BodyAccuracy::Exact);
}

#[test]
fn a_numerical_body_displays_and_picks_by_its_own_faces_and_edges() {
    let k = CatallaxKernel::with_numerical_stand_in(&[OpKind::Extrude]);
    let body = numerical_block(&k).body;
    let topology = k.topology(body).unwrap();
    assert_eq!((topology.faces.len(), topology.edges.len(), topology.vertex_count), (6, 12, 8));
    assert_eq!(topology.kind, BodyKind::Solid);
    for face in &topology.faces {
        assert_eq!(face.accuracy, FaceAccuracy::Approximate { tolerance_mm: 0.01 });
        assert!(!face.origins.is_empty(), "face {:?} lost its feature", face.id);
        assert_eq!(face.edges.len(), 4);
        assert_eq!(face.surface, SurfaceKind::Plane);
    }
    assert!(topology.edges.iter().all(|e| e.faces.len() == 2 && e.vertices.len() == 2 && e.curve == CurveKind::Line));
    let top = topology.faces.iter().find(|f| f.normal == Some(Vec3::Z)).unwrap();
    assert!((top.area - 80.0).abs() < 1e-6, "{top:?}");
    assert!((top.centroid.z - 6.0).abs() < 1e-6);

    let params = TessellationParams::default();
    let mesh = k.rendering(body, &params).unwrap();
    assert!(mesh.unproved, "an approximate body's display claims no bound");
    let faces: std::collections::BTreeSet<_> = mesh.triangle_faces.iter().copied().collect();
    assert_eq!(faces, topology.faces.iter().map(|f| f.id).collect());
    assert_eq!(k.preview(body, &params).unwrap(), mesh);
    let edges = k.rendering_edges(body, &params).unwrap();
    assert_eq!(edges.len(), 12);
    assert!(edges.iter().all(|e| topology.edge(e.edge).is_some()));
    assert_eq!(k.display_bound(body).unwrap(), None);
}

#[test]
fn a_numerical_body_is_measured_with_its_mass_properties_as_estimates() {
    let k = CatallaxKernel::with_numerical_stand_in(&[OpKind::Extrude]);
    let body = numerical_block(&k).body;
    assert_eq!(k.cached_mass_properties(body), None);
    let mass = k.mass_properties(body).unwrap();
    assert!((mass.volume - 480.0).abs() < 1e-6, "{mass:?}");
    assert!((mass.area - 376.0).abs() < 1e-6);
    assert!(mass.centroid.distance(Vec3::new(0.0, 0.0, 3.0)) < 1e-6);
    // A block of sides a, b and c: V (b^2 + c^2) / 12 about its x axis.
    let inertia = mass.inertia.unwrap();
    assert!((inertia[0][0] - 480.0 * (64.0 + 36.0) / 12.0).abs() < 1e-4, "{inertia:?}");
    assert!(inertia[0][1].abs() < 1e-6);
    assert_eq!(k.cached_mass_properties(body), Some(mass));

    let topology = k.topology(body).unwrap();
    let long = topology.edges.iter().find(|e| (e.length - 10.0).abs() < 1e-6).unwrap();
    let length = k.edge_length(body, long.id).unwrap();
    assert_eq!(length.certainty, Certainty::Approximate);
    assert!((length.value - 10.0).abs() < 1e-6 && length.lo == length.value && length.hi == length.value);
    let top = topology.faces.iter().find(|f| f.normal == Some(Vec3::Z)).unwrap();
    let area = k.face_area(body, top.id).unwrap();
    assert_eq!(area.certainty, Certainty::Approximate);
    assert!((area.value - 80.0).abs() < 1e-6);
}

#[test]
fn a_numerical_body_writes_its_own_approximate_step_file() {
    let k = CatallaxKernel::with_numerical_stand_in(&[OpKind::Extrude]);
    let body = numerical_block(&k).body;
    let export = k.export_step(body, &StepOptions { product_name: "block".into(), tolerance: 1e-7 }).unwrap();
    assert!(export.text.starts_with("ISO-10303-21;"), "{}", &export.text[..80]);
    assert!(export.text.contains("'block'"));
    assert_eq!(export.text.matches("ADVANCED_FACE").count(), 6);
    let report = export.report;
    // Catallax bounds the rounding of each written length, not the file's distance from
    // the body, so the report keeps the two apart and claims no deviation.
    let approximate = report.approximate.clone().unwrap();
    assert_eq!(approximate.tolerance_mm, 0.01);
    assert!(approximate.max_length_rounding_mm <= 1e-7, "{report:?}");
    assert_eq!(report.max_deviation, soliddraft_kernel_api::NOT_ESTABLISHED);
    assert_eq!((report.max_deviation_mm, report.fit_mm), (0.0, 0.0));
    // An unestablished distance never meets a requested tolerance.
    assert!(!report.within_tolerance);
    assert_eq!((report.solids, report.faces, report.edges, report.vertices), (1, 6, 12, 8));
}

#[test]
fn work_on_a_numerical_body_that_needs_exact_geometry_says_it_is_coming() {
    let k = CatallaxKernel::with_numerical_stand_in(&[OpKind::Extrude]);
    let body = numerical_block(&k).body;
    let edge = k.topology(body).unwrap().edges[0].id;
    let error = k.fillet(&OpTag::new("round"), body, &[edge], 1.0).unwrap_err().to_string();
    assert!(error.contains("coming soon"), "{error}");
    let error = k.face_carriers(body).unwrap_err().to_string();
    assert!(error.contains("coming soon"), "{error}");
    // The body is unchanged by the refusal and frees like any other.
    assert!((k.mass_properties(body).unwrap().volume - 480.0).abs() < 1e-6);
    assert!(k.retained_bytes(body).unwrap() > 0);
    k.release(body).unwrap();
    assert!(matches!(k.topology(body), Err(KernelError::UnknownBody(_))));
}

/// A feature pattern reads its source and earlier bodies only as context, never through
/// their exact solids. An approximate source must not replay as an exact build.
#[test]
fn an_approximate_pattern_source_is_never_replayed_as_exact() {
    let k = CatallaxKernel::with_numerical_stand_in(&[OpKind::Union]);
    let block = |tag: &str, center: Vec3, w: f64, h: f64, depth: f64| {
        let sheet = k
            .create_body_from_profile(
                &OpTag::new(format!("{tag}.profile")),
                &Profile::rectangle(Plane::XY.with_origin(center), Point2::new(0.0, 0.0), w, h),
            )
            .unwrap()
            .body;
        k.extrude(&OpTag::new(tag), sheet, &Extrude { distance: depth, direction: None }).unwrap().body
    };
    let stock = block("stock", Vec3::ZERO, 30.0, 10.0, 2.0);
    let tool = block("boss.tool", Vec3::new(-10.0, 0.0, 2.0), 2.0, 2.0, 3.0);
    assert_eq!(k.body_accuracy(stock).unwrap(), BodyAccuracy::Exact);
    let boss = k.boolean(&OpTag::new("boss"), BooleanOp::Union, stock, tool).unwrap().body;
    assert!(matches!(k.body_accuracy(boss).unwrap(), BodyAccuracy::Approximate { .. }));
    let layout = PatternLayout::Linear { step: Vec3::new(6.0, 0.0, 0.0), count: 3 };
    let error = k.pattern_feature(&OpTag::new("row"), stock, boss, Some(stock), &layout).unwrap_err().to_string();
    assert!(error.ends_with("is coming soon."), "{error}");
    // The exact base is untouched and still takes exact work.
    assert!((k.mass_properties(stock).unwrap().volume - 600.0).abs() < 1e-6);
    // A body relation reads both operands' recipes, so it refuses an approximate one too.
    let placed = |body| PlacedBody { body, rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0; 3] };
    let error = k.body_relation(placed(stock), placed(boss), None).unwrap_err().to_string();
    assert!(error.ends_with("is coming soon."), "{error}");
}
