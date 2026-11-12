//! Line and arc sweeps through the exact application entry, properties, display and STEP.

use soliddraft_kernel_api::*;
use soliddraft_kernel_catallax::CatallaxKernel;

fn path() -> Sweep {
    Sweep::line([100.0, 200.0, 300.0], [102.0, 201.0, 303.0], "travel")
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
    assert_eq!(continued_recipe["format"], "catallax-recipe/5");
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
        let p = Sweep::line(path().start, end, "travel");
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

fn line(id: &str, to: [f64; 3]) -> PathSegment {
    PathSegment::Line { id: id.into(), to, corner_axis: None }
}

fn arc(id: &str, centre: [f64; 3], axis: [f64; 3], turn: ArcTurn) -> PathSegment {
    PathSegment::Arc { id: id.into(), centre, axis, turn, corner_axis: None }
}

fn following(segments: Vec<PathSegment>) -> Sweep {
    Sweep { start: [0.0; 3], segments, orientation: SweepOrientation::FollowPath }
}

/// The public example: a 2 by 1 profile at radii 1 to 2 from the path.
fn off_path(k: &CatallaxKernel) -> BodyId {
    let profile = Profile::rectangle(Plane::XY, [1.0, -1.5].into(), 2.0, 1.0);
    k.create_body_from_profile(&OpTag::new("profile"), &profile).unwrap().body
}

/// Editor display controls: 0.04 mm chord, 28.6 degree facet normals.
fn editor_mesh(k: &CatallaxKernel, body: BodyId) -> Mesh {
    let controls = TessellationParams { chord_height: 0.04, surface_angle_deg: 28.6, ..TessellationParams::default() };
    k.tessellate(body, &controls).unwrap()
}

fn volume_is(k: &CatallaxKernel, body: BodyId, expected: f64) {
    let volume = k.mass_properties(body).unwrap().volume;
    assert!((volume - expected).abs() < 1e-6, "{volume} versus {expected}");
}

#[test]
fn line_arc_line_chain_keeps_every_segment_through_display_and_step() {
    use std::f64::consts::PI;
    let k = CatallaxKernel::new();
    let sheet = off_path(&k);
    let path = following(vec![
        line("rise", [0.0, 0.0, 4.0]),
        arc("bend", [0.0, 5.0, 4.0], [-1.0, 0.0, 0.0], ArcTurn::Quarter),
        line("run", [0.0, 9.0, 9.0]),
    ]);
    let swept = k.sweep(&OpTag::new("chain"), sheet, &path).unwrap();
    // Two 2 by 1 by 4 prisms and a quarter annulus of radii 6 and 7, width 2.
    volume_is(&k, swept.body, 16.0 + 6.5 * PI);
    let recipe = k.body_recipe(swept.body).unwrap();
    let steps = recipe["steps"].as_array().unwrap();
    let rail = steps.iter().find(|s| s["op"] == "path").unwrap();
    assert_eq!(rail["segments"][1]["kind"], "arc");
    assert_eq!(rail["segments"][1]["to"][2], "9");
    assert_eq!(steps.iter().find(|s| s["op"] == "sweep").unwrap()["orientation"], "rotation_minimising");
    let topology = k.measured_topology(swept.body).unwrap();
    let on = |face: &FaceInfo, id: &str| {
        face.origins.iter().any(|o| matches!(&o.role, FaceRole::Side { path: Some(p), .. } if p == id))
    };
    for id in ["rise", "bend", "run"] {
        assert!(topology.faces.iter().filter(|f| on(f, id)).count() >= 2, "{id}");
    }
    let caps = |role: FaceRole| topology.faces.iter().filter(|f| f.origins.iter().any(|o| o.role == role)).count();
    assert_eq!((caps(FaceRole::StartCap), caps(FaceRole::EndCap)), (1, 1));
    let mesh = editor_mesh(&k, swept.body);
    assert!(!mesh.indices.is_empty() && !mesh.unproved);
    for face in &topology.faces {
        assert!(mesh.triangle_faces.contains(&face.id), "{} is not displayed", face.id);
    }
    let step = k.export_step(swept.body, &StepOptions::default()).unwrap();
    assert!(step.text.contains("MANIFOLD_SOLID_BREP"));
    assert!(step.text.contains("TOROIDAL_SURFACE") || step.text.contains("CYLINDRICAL_SURFACE"));
}

#[test]
fn stationary_turn_merges_coplanar_sides_and_keeps_both_origins() {
    use std::f64::consts::PI;
    let k = CatallaxKernel::new();
    let sheet = off_path(&k);
    let path = following(vec![line("rise", [0.0, 0.0, 4.0]), line("run", [0.0, 4.0, 4.0])]);
    let swept = k.sweep(&OpTag::new("turn"), sheet, &path).unwrap();
    volume_is(&k, swept.body, 16.0 + 1.5 * PI);
    let topology = k.measured_topology(swept.body).unwrap();
    let on = |face: &FaceInfo, id: &str| {
        face.origins.iter().any(|o| matches!(&o.role, FaceRole::Side { path: Some(p), .. } if p == id))
    };
    let merged = topology.faces.iter().filter(|f| on(f, "rise") && on(f, "run")).count();
    assert_eq!(merged, 2, "the width sides stay one face with both origins");
    let roles = |face: FaceId| swept.record.face_roles.iter().filter(move |o| o.face == face).count();
    assert!(topology.faces.iter().filter(|f| on(f, "rise") && on(f, "run")).all(|f| roles(f.id) >= 2));
}

#[test]
fn reversal_uses_the_saved_axis_and_asks_for_it_when_missing() {
    use std::f64::consts::PI;
    let k = CatallaxKernel::new();
    let sheet = off_path(&k);
    let mut path = following(vec![line("rise", [0.0, 0.0, 4.0]), line("back", [0.0, 0.0, 0.0])]);
    let refused = k.sweep(&OpTag::new("reverse"), sheet, &path).unwrap_err().to_string();
    assert!(refused.contains("Set a turn axis where the path doubles back"), "{refused}");
    path.segments[1].set_corner_axis(Some([-1.0, 0.0, 0.0]));
    let swept = k.sweep(&OpTag::new("reverse"), sheet, &path).unwrap();
    // The half turn joins two prisms with a half annulus.
    volume_is(&k, swept.body, 16.0 + 3.0 * PI);
    let recipe = k.body_recipe(swept.body).unwrap();
    let steps = recipe["steps"].as_array().unwrap();
    let rail = steps.iter().find(|s| s["op"] == "path").unwrap();
    assert_eq!(rail["segments"][1]["corner_axis"][0], "-1");
    path.segments[0].set_corner_axis(Some([-1.0, 0.0, 0.0]));
    let refused = k.sweep(&OpTag::new("reverse"), sheet, &path).unwrap_err().to_string();
    assert!(refused.contains("only where the path doubles back"), "{refused}");
}

#[test]
fn fixed_line_chain_is_a_regularised_union_with_a_join_face_name() {
    let k = CatallaxKernel::new();
    let sheet = off_path(&k);
    let path = Sweep {
        start: [0.0; 3],
        segments: vec![line("rise", [0.0, 0.0, 4.0]), line("back", [0.0, 0.0, 2.0])],
        orientation: SweepOrientation::Fixed,
    };
    let swept = k.sweep(&OpTag::new("retrace"), sheet, &path).unwrap();
    volume_is(&k, swept.body, 8.0);
    let topology = k.measured_topology(swept.body).unwrap();
    assert_eq!(topology.faces.len(), 6);
    let joins = topology.faces.iter().flat_map(|f| &f.origins);
    assert!(joins.filter(|o| matches!(&o.role, FaceRole::Envelope { path } if path == "back")).count() == 1);
}

#[test]
fn every_cardinal_turn_builds_from_a_saved_turn() {
    use std::f64::consts::PI;
    let k = CatallaxKernel::new();
    for (turn, quarters) in [(ArcTurn::Quarter, 1.0), (ArcTurn::Half, 2.0), (ArcTurn::ThreeQuarter, 3.0)] {
        let sheet = off_path(&k);
        let path = following(vec![arc("bend", [0.0, 5.0, 0.0], [-2.0, 0.0, 0.0], turn)]);
        let swept = k.sweep(&OpTag::new("bend"), sheet, &path).unwrap();
        volume_is(&k, swept.body, 6.5 * PI * quarters);
    }
}

#[test]
fn unfinished_motions_say_only_that_they_are_coming_soon() {
    let k = CatallaxKernel::new();
    let sheet = off_path(&k);
    let axis = [-1.0, 0.0, 0.0];
    for path in [
        // Translating along a circle.
        Sweep {
            orientation: SweepOrientation::Fixed,
            ..following(vec![arc("a", [0.0, 5.0, 0.0], axis, ArcTurn::Half)])
        },
        // A 45 degree join.
        following(vec![line("a", [0.0, 0.0, 4.0]), line("b", [0.0, 3.0, 7.0])]),
        // The axis passes through the profile.
        following(vec![arc("a", [0.0, -1.5, 0.0], axis, ArcTurn::Half)]),
        // A quarter turn about an axis of irrational length.
        following(vec![arc("a", [0.0, 5.0, 0.0], [-1.0, 0.0, 1.0], ArcTurn::Quarter)]),
    ] {
        let refused = k.sweep(&OpTag::new("later"), sheet, &path).unwrap_err().to_string();
        assert!(refused.contains("This sweep path is coming soon."), "{path:?}: {refused}");
    }
}

fn spline() -> Sweep {
    Sweep {
        start: [0.0; 3],
        orientation: SweepOrientation::Fixed,
        segments: vec![PathSegment::Spline {
            id: "rail".into(),
            degree: 3,
            control_points: vec![[0.0; 3], [1.0, 0.0, 1.0], [1.0, 1.0, 2.0], [0.0, 1.0, 3.0]],
            weights: vec![1.0; 4],
            knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
        }],
    }
}

#[test]
fn spline_path_keeps_exact_controls_and_plain_typed_refusals() {
    let k = CatallaxKernel::new();
    let sheet = k
        .create_body_from_profile(&OpTag::new("profile"), &Profile::rectangle(Plane::XY, [0.0, 0.0].into(), 4.0, 2.0))
        .unwrap()
        .body;
    let path = spline();
    let swept = k.sweep(&OpTag::new("spline"), sheet, &path).unwrap();
    assert!((k.mass_properties(swept.body).unwrap().volume - 24.0).abs() < 1e-8);
    let recipe = k.body_recipe(swept.body).unwrap();
    let rail = recipe["steps"].as_array().unwrap().iter().find(|s| s["op"] == "path").unwrap();
    assert_eq!(rail["segments"][0]["control_points"][2], serde_json::json!(["1", "1", "2"]));
    assert_eq!(rail["segments"][0]["id"], "rail");
    assert!(k.export_step(swept.body, &StepOptions::default()).unwrap().text.contains("B_SPLINE_SURFACE"));
    let mut returning = path.clone();
    let PathSegment::Spline { control_points, .. } = &mut returning.segments[0] else { unreachable!() };
    *control_points.last_mut().unwrap() = [0.0; 3];
    let refused = k.sweep(&OpTag::new("return"), sheet, &returning).unwrap_err();
    assert!(
        matches!(refused, KernelError::InvalidInput(ref m) if m.contains("path may turn back or cross itself")),
        "{refused}"
    );
    let mut turning = path.clone();
    turning.orientation = SweepOrientation::FollowPath;
    assert!(k.sweep(&OpTag::new("turn"), sheet, &turning).unwrap_err().to_string().contains("one spline segment"));
    let mut invalid = path;
    let PathSegment::Spline { weights, .. } = &mut invalid.segments[0] else { unreachable!() };
    weights[1] = 0.0;
    assert!(
        k.sweep(&OpTag::new("weight"), sheet, &invalid).unwrap_err().to_string().contains("positive finite weight")
    );
    assert!(k.sweep(&OpTag::new("after"), sheet, &spline()).is_ok());
}
