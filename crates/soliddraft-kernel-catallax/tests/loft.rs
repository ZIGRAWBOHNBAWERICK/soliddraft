//! Loft uses retained definitions for geometry, face boundaries and refusals.

use soliddraft_kernel_api::*;
use soliddraft_kernel_catallax::CatallaxKernel;

#[test]
fn face_profiles_keep_exact_holes_and_a_rotated_polygon_frame() {
    let k = CatallaxKernel::new();
    let rotated =
        Plane { origin: Vec3::new(1.0, 2.0, 3.0), normal: Vec3::new(0.6, 0.0, 0.8), x_dir: Vec3::new(0.8, 0.0, -0.6) };
    let mut ring = Profile::circle(Plane::XY, Point2::new(0.0, 0.0), 2.0);
    ring.holes.push(Profile::circle(Plane::XY, Point2::new(0.0, 0.0), 1.0).outer);
    for (profile, area) in
        [(Profile::rectangle(rotated, Point2::new(0.0, 0.0), 2.0, 2.0), 4.0), (ring, 3.0 * std::f64::consts::PI)]
    {
        let p = k.create_body_from_profile(&OpTag::new("profile"), &profile).unwrap();
        let original = k.extrude(&OpTag::new("original"), p.body, &Extrude { distance: 1.0, direction: None }).unwrap();
        let face = original.record.face_roles.iter().find(|f| f.role == FaceRole::EndCap).unwrap().face;
        let extracted = k.create_profile_from_face(&OpTag::new("face"), original.body, face).unwrap();
        let body =
            k.extrude(&OpTag::new("from_face"), extracted.body, &Extrude { distance: 2.0, direction: None }).unwrap();
        assert!((k.mass_properties(body.body).unwrap().volume - area * 2.0).abs() < 1e-7);
        for body in [p.body, original.body, extracted.body, body.body] {
            k.release(body).unwrap();
        }
    }
}

#[test]
fn coplanar_loft_is_refused_plainly_and_retained_lofts_export_as_native_recipes() {
    let k = CatallaxKernel::new();
    let a = k
        .create_body_from_profile(&OpTag::new("a"), &Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 2.0, 2.0))
        .unwrap();
    let error = k.loft(&OpTag::new("flat"), &[a.body, a.body], LoftShape::Ruled).unwrap_err();
    assert!(
        matches!(error,KernelError::InvalidInput(ref m) if m == "Loft profiles must enclose a solid. Move them onto different planes"),
        "{error}"
    );
    let plane = Plane { origin: Vec3::new(0.0, 0.0, 3.0), ..Plane::XY };
    let b = k
        .create_body_from_profile(&OpTag::new("b"), &Profile::rectangle(plane, Point2::new(0.0, 0.0), 4.0, 4.0))
        .unwrap();
    let body = k.loft(&OpTag::new("loft"), &[a.body, b.body], LoftShape::Ruled).unwrap();
    assert!((k.mass_properties(body.body).unwrap().volume - 28.0).abs() < 1e-7);
    let retained = k.body_recipe(body.body).unwrap();
    assert_eq!(retained["format"], "catallax-recipe/5");
    assert!(retained["steps"].as_array().unwrap().iter().any(|s| s["op"] == "loft"));
    catallax_recipe::Recipe::parse(&retained.to_string()).unwrap();
    for body in [a.body, b.body, body.body] {
        k.release(body).unwrap();
    }
}
