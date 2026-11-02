//! Straight sweeps through the exact application entry, properties, display and STEP.

use soliddraft_kernel_api::*;
use soliddraft_kernel_catallax::CatallaxKernel;

fn path() -> Sweep {
    Sweep { start: [100.0, 200.0, 300.0], end: [102.0, 201.0, 303.0], segment_id: "travel".into() }
}

#[test]
fn straight_sweep_retains_path_names_and_exact_displacement_through_export() {
    let k = CatallaxKernel::new();
    let profile = Profile::rectangle(Plane::XY.with_origin([7.0, -3.0, 11.0].into()), [2.0, 1.0].into(), 4.0, 2.0);
    let sheet = k.create_body_from_profile(&OpTag::new("profile"), &profile).unwrap().body;
    let swept = k.sweep(&OpTag::new("travel"), sheet, &path()).unwrap();
    assert_eq!(swept.record.op, OpKind::Sweep);
    assert_eq!(swept.record.face_roles.len(), 6);
    let mass = k.mass_properties(swept.body).unwrap();
    assert!((mass.volume - 24.0).abs() < 1e-8);
    assert!((mass.centroid - Vec3::new(10.0, -1.5, 12.5)).length() < 1e-8);
    let recipe = k.body_recipe(swept.body).unwrap();
    let steps = recipe["steps"].as_array().unwrap();
    let rail = steps.iter().find(|s| s["op"] == "path").unwrap();
    assert_eq!(rail["segments"][0]["id"], "travel");
    assert_eq!(rail["start"][0], "100");
    assert!(steps.iter().any(|s| s["op"] == "sweep"));
    let topology = k.measured_topology(swept.body).unwrap();
    assert_eq!((topology.faces.len(), topology.edges.len(), topology.vertices.len()), (6, 12, 8));
    assert!((topology.bbox.min - Vec3::new(7.0, -3.0, 11.0)).length() < 1e-8);
    assert!((topology.bbox.max - Vec3::new(13.0, 0.0, 14.0)).length() < 1e-8);
    for face in &topology.faces {
        assert!(!face.origins.is_empty());
    }
    let mesh = k.tessellate(swept.body, &TessellationParams::default()).unwrap();
    assert!(!mesh.indices.is_empty());
    let step = k.export_step(swept.body, &StepOptions::default()).unwrap();
    assert!(step.text.contains("MANIFOLD_SOLID_BREP"));
    let continued = k.boolean(&OpTag::new("continued"), BooleanOp::Union, swept.body, swept.body).unwrap().body;
    assert!((k.mass_properties(continued).unwrap().volume - 24.0).abs() < 1e-8);
    let continued_recipe = k.body_recipe(continued).unwrap();
    assert_eq!(continued_recipe["format"], "catallax-recipe/4");
    assert!(continued_recipe["steps"].as_array().unwrap().iter().any(|s| s["op"] == "path"));
    k.release(continued).unwrap();
    k.release(sheet).unwrap();
    k.release(swept.body).unwrap();
}

#[test]
fn outside_sweep_domain_has_plain_messages_and_preserves_profile() {
    let k = CatallaxKernel::new();
    let polygon = k
        .create_body_from_profile(&OpTag::new("polygon"), &Profile::rectangle(Plane::XY, [0.0, 0.0].into(), 4.0, 2.0))
        .unwrap()
        .body;
    for (end, expected) in
        [([100.0, 200.0, 300.0], "different finite endpoints"), ([102.0, 201.0, 300.0], "leave the profile plane")]
    {
        let p = Sweep { end, ..path() };
        assert!(k.sweep(&OpTag::new("bad"), polygon, &p).unwrap_err().to_string().contains(expected));
    }
    let circle = k
        .create_body_from_profile(&OpTag::new("circle"), &Profile::circle(Plane::XY, [0.0, 0.0].into(), 1.0))
        .unwrap()
        .body;
    assert!(
        k.sweep(&OpTag::new("bad"), circle, &path())
            .unwrap_err()
            .to_string()
            .contains("closed polygon profile with straight sides")
    );
    assert!(k.sweep(&OpTag::new("valid"), polygon, &path()).is_ok());
}

#[test]
fn sweep_build_refusal_keeps_replay_data_and_uses_plain_text() {
    let k = CatallaxKernel::with_diagnostics(0);
    let polygon = k
        .create_body_from_profile(&OpTag::new("polygon"), &Profile::rectangle(Plane::XY, [0.0, 0.0].into(), 4.0, 2.0))
        .unwrap()
        .body;
    let message = k.sweep(&OpTag::new("sweep"), polygon, &path()).unwrap_err().to_string();
    assert!(message.contains("This sweep could not finish"), "{message}");
    for internal in ["Catallax", "stage", "numbers.", "kernel"] {
        assert!(!message.contains(internal), "{message}");
    }
    assert!(!k.take_diagnostics().unwrap()["questions"].as_array().unwrap().is_empty());
}
