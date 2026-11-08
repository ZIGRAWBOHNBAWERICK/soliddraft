//! Picked generator edges round independently when their face origins are shared.

use catallax_boolean::{Containment, classify_point};
use catallax_features::evaluate;
use catallax_numbers::{Decider, Outcome, Rational, with_operation_budget};
use catallax_recipe::Recipe;
use catallax_shapes::RPoint3;
use soliddraft_kernel_api::*;
use soliddraft_kernel_catallax::CatallaxKernel;

fn assert_material(k: &CatallaxKernel, outside: [&str; 3], inside: [&str; 3]) {
    let evidence = k.take_diagnostics().unwrap();
    let problem = &evidence["questions"].as_array().unwrap().last().unwrap()["problem"];
    let recipe = Recipe::parse(&problem.to_string()).unwrap();
    let (built, work) = with_operation_budget(20_000_000, || evaluate(&recipe, &mut Decider::default()));
    assert!(!work.exhausted);
    let mut built = built.unwrap();
    for (point, expected) in [(outside, Containment::Outside), (inside, Containment::Inside)] {
        let [x, y, z] = point.map(|n| Rational::parse(n).unwrap());
        let (actual, work) = with_operation_budget(20_000_000, || {
            classify_point(&mut built.model, built.result, &RPoint3::new(x, y, z), &mut Decider::default())
        });
        assert!(!work.exhausted);
        assert!(matches!(actual, Outcome::Proven(found) if found == expected), "{point:?}: {actual:?}");
    }
}

fn assert_one_strip(k: &CatallaxKernel, body: BodyId, at: Vec3, radius: f64, outside: [&str; 3], inside: [&str; 3]) {
    let topology = k.topology(body).unwrap();
    let edge = topology.edges.iter().find(|e| e.midpoint.distance(at) < 1e-6).expect("picked generator").id;
    let original_cylinders = topology.faces.iter().filter(|f| f.surface == SurfaceKind::Cylinder).count();
    let selected = topology.edge(edge).unwrap();
    let opposite = topology
        .edges
        .iter()
        .find(|other| {
            other.id != edge
                && other.faces.len() == selected.faces.len()
                && other.faces.iter().all(|face| selected.faces.contains(face))
        })
        .expect("the shared support pair has two generators");
    let rounded = k.fillet(&OpTag::new("opposite"), body, &[opposite.id], radius).unwrap();
    assert!(rounded.record.face_roles.iter().any(|face| face.role == FaceRole::Blend { edge: opposite.id }));
    k.release(rounded.body).unwrap();
    let before = k.mass_properties(body).unwrap();
    for (edit, radius) in [radius, radius / 2.0].into_iter().enumerate() {
        let rounded = k.fillet(&OpTag::new("round"), body, &[edge], radius).unwrap();
        let strips: Vec<_> =
            rounded.record.face_roles.iter().filter(|face| matches!(face.role, FaceRole::Blend { .. })).collect();
        assert_eq!(strips.len(), 1);
        assert_eq!(strips[0].role, FaceRole::Blend { edge }, "blend belongs to the picked generator");
        let saved = k.body_recipe(rounded.body).unwrap();
        let operation = &saved["steps"].as_array().unwrap().last().unwrap()["native"];
        assert!(operation["edges"][0]["near"].is_array(), "a shared origin pair retains the picked point");
        assert!(k.validate(rounded.body).unwrap().valid);
        let topology = k.topology(rounded.body).unwrap();
        let cylinders = topology.faces.iter().filter(|f| f.surface == SurfaceKind::Cylinder).count();
        assert_eq!(cylinders, original_cylinders + 1, "exactly one rounded strip");
        if edit == 0 {
            assert_material(k, outside, inside);
        }
        assert!(k.export_step(rounded.body, &StepOptions::default()).unwrap().report.within_tolerance);
        k.release(rounded.body).unwrap();
    }
    assert_eq!(k.mass_properties(body).unwrap(), before);
}

#[test]
fn picking_one_parallel_cylinder_generator_rounds_only_that_edge() {
    let k = CatallaxKernel::with_diagnostics(20_000_000);
    let a = contract::cylinder(&k, Vec3::new(-3.0, 0.0, 0.0), 5.0, 12.0);
    let b = contract::cylinder(&k, Vec3::new(3.0, 0.0, 0.0), 5.0, 12.0);
    let lens = k.boolean(&OpTag::new("lens"), BooleanOp::Intersect, a, b).unwrap().body;
    assert_one_strip(&k, lens, Vec3::new(0.0, 4.0, 6.0), 1.25, ["0", "18/5", "6"], ["0", "-39/10", "6"]);
}

#[test]
fn picking_one_cylinder_flat_generator_rounds_only_that_edge() {
    let k = CatallaxKernel::with_diagnostics(20_000_000);
    let left = Point2::new(-4.0, 3.0);
    let right = Point2::new(4.0, 3.0);
    let profile = Profile {
        plane: Plane::XY,
        holes: vec![],
        outer: Loop(vec![
            Segment::Line { from: left, to: right },
            Segment::Arc { center: Point2::new(0.0, 0.0), from: right, to: left, clockwise: true },
        ]),
    };
    let section = k.create_body_from_profile(&OpTag::new("section"), &profile).unwrap().body;
    let body = k.extrude(&OpTag::new("shaft"), section, &Extrude { distance: 12.0, direction: None }).unwrap().body;
    assert_one_strip(&k, body, Vec3::new(4.0, 3.0, 6.0), 1.75, ["39/10", "29/10", "6"], ["-39/10", "29/10", "6"]);
}

#[test]
fn oblique_rim_measures_and_exports() {
    let k = CatallaxKernel::with_diagnostics(200_000_000);
    let cylinder = contract::cylinder(&k, Vec3::ZERO, 5.0, 16.0);
    let points = [(-30.0, 30.5), (30.0, -14.5), (30.0, 40.0), (-30.0, 40.0)].map(|(u, v)| Point2::new(u, v));
    let outer = Loop((0..4).map(|i| Segment::Line { from: points[i], to: points[(i + 1) % 4] }).collect());
    let plane = Plane { origin: Vec3::new(0.0, -30.0, 0.0), normal: -Vec3::Y, x_dir: Vec3::X };
    let section =
        k.create_body_from_profile(&OpTag::new("section"), &Profile { plane, outer, holes: vec![] }).unwrap().body;
    let tool =
        k.extrude(&OpTag::new("tool"), section, &Extrude { distance: 60.0, direction: Some(Vec3::Y) }).unwrap().body;
    let cut = k.boolean(&OpTag::new("cut"), BooleanOp::Subtract, cylinder, tool).unwrap().body;
    let topology = k.topology(cut).unwrap();
    let edge = topology.edges.iter().find(|e| e.curve == CurveKind::Ellipse).expect("closed oblique rim").id;
    let before = k.mass_properties(cut).unwrap();
    assert!(k.body_notice(cut).is_none());
    for radius in [1.0, 0.5] {
        let rounded = k.fillet(&OpTag::new("round"), cut, &[edge], radius).unwrap();
        assert!(k.validate(rounded.body).unwrap().valid);
        assert!(k.body_notice(rounded.body).is_none());
        assert!(!k.tessellate(rounded.body, &TessellationParams::default()).unwrap().indices.is_empty());
        let result = k.mass_properties(rounded.body);
        let diagnostics = k.take_diagnostics().unwrap();
        let call = diagnostics["calls"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|call| call["operation"] == "mass_properties")
            .unwrap();
        println!(
            "oblique mass radius={radius}: wall_ms={} work={} status={} error={}",
            call["wall_ms"], call["work"], call["status"], call["error"]
        );
        if let Some(folder) = std::env::var_os("SOLIDDRAFT_MEASURE_OUT") {
            let folder = std::path::PathBuf::from(folder);
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(
                folder.join(format!("oblique-mass-r{radius}.json")),
                serde_json::to_vec_pretty(&diagnostics).unwrap(),
            )
            .unwrap();
        }
        let measured = result.unwrap();
        assert!(measured.volume > 600.0 && measured.volume < before.volume);
        assert!(measured.area > 0.0 && measured.inertia.is_some());
        assert!(k.measured_topology(rounded.body).unwrap().bbox.diagonal().is_finite());
        assert!(k.export_step(rounded.body, &StepOptions::default()).unwrap().report.within_tolerance);
        k.release(rounded.body).unwrap();
    }
    assert_eq!(k.mass_properties(cut).unwrap(), before);
}

#[test]
fn a_radius_past_the_face_limit_is_refused_not_built_smaller() {
    let k = CatallaxKernel::with_diagnostics(20_000_000);
    let block = contract::block(&k);
    let topology = k.topology(block).unwrap();
    let edge = topology.edges.iter().find(|e| e.midpoint.distance(Vec3::new(0.0, 15.0, 10.0)) < 1e-6).unwrap().id;
    let before = k.mass_properties(block).unwrap();
    // Catallax builds R50 here as the R10 strip that consumes the 10 mm side face.
    for radius in [10.5, 50.0] {
        let refused = k.fillet(&OpTag::new("round"), block, &[edge], radius).unwrap_err().to_string();
        assert!(refused.contains("fillet.radius-too-large"), "R{radius}: {refused}");
        assert!(refused.contains("; radius limit 10 mm"), "R{radius}: {refused}");
    }
    assert_eq!(k.mass_properties(block).unwrap(), before);
    // R10 consumes the face exactly and builds: 12000 - 4000 (1 - pi/4) mm3.
    let rounded = k.fillet(&OpTag::new("round"), block, &[edge], 10.0).unwrap();
    let volume = k.mass_properties(rounded.body).unwrap().volume;
    let exact = 12000.0 - 4000.0 * (1.0 - std::f64::consts::FRAC_PI_4);
    assert!((volume - exact).abs() < 1e-6, "{volume} != {exact}");
}
