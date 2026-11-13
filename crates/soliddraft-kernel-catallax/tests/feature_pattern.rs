//! Feature instances replay the selected operation without copying unrelated stock.
use soliddraft_kernel_api::*;
use soliddraft_kernel_catallax::CatallaxKernel;

fn block(k: &CatallaxKernel, tag: &str, center: Vec3, w: f64, h: f64, depth: f64) -> BodyId {
    let sheet = k
        .create_body_from_profile(
            &OpTag::new(format!("{tag}.profile")),
            &Profile::rectangle(Plane::XY.with_origin(center), Point2::new(0.0, 0.0), w, h),
        )
        .unwrap()
        .body;
    k.extrude(&OpTag::new(tag), sheet, &Extrude { distance: depth, direction: None }).unwrap().body
}
fn volume(k: &CatallaxKernel, b: BodyId, expected: f64) {
    assert!((k.mass_properties(b).unwrap().volume - expected).abs() < 1e-6);
}
#[test]
fn linear_boss_copies_preserve_stock_and_later_cut() {
    let k = CatallaxKernel::new();
    let stock = block(&k, "stock", Vec3::ZERO, 30.0, 10.0, 2.0);
    let tool = block(&k, "boss.tool", Vec3::new(-10.0, 0.0, 2.0), 2.0, 2.0, 3.0);
    let boss = k.boolean(&OpTag::new("boss"), BooleanOp::Union, stock, tool).unwrap().body;
    let cut = block(&k, "cut.tool", Vec3::new(12.0, 0.0, 0.0), 2.0, 2.0, 2.0);
    let later = k.boolean(&OpTag::new("cut"), BooleanOp::Subtract, boss, cut).unwrap().body;
    let layout = PatternLayout::Linear { step: Vec3::new(6.0, 0.0, 0.0), count: 3 };
    let result = k.pattern_feature(&OpTag::new("row"), later, boss, Some(stock), &layout).unwrap();
    volume(&k, result.body, 628.0);
    let bounds = k.topology(result.body).unwrap().bbox;
    assert_eq!((bounds.min.x, bounds.max.x), (-15.0, 15.0));
    assert!(result.record.face_roles.iter().any(|f| matches!(f.role, FaceRole::FeatureCopy { instance: 1, .. })));
    assert!(result.record.face_roles.iter().any(|f| matches!(f.role, FaceRole::FeatureCopy { instance: 2, .. })));
    assert!(k.export_step(result.body, &StepOptions::default()).is_ok());
    let recipe: catallax_recipe::Recipe = serde_json::from_value(k.body_recipe(result.body).unwrap()).unwrap();
    recipe.check().unwrap();

    volume(&k, later, 604.0);
}
#[test]
fn circular_cut_instances_remove_material_and_refuse_unsavable_rotation() {
    let k = CatallaxKernel::new();
    let stock = block(&k, "stock", Vec3::ZERO, 20.0, 20.0, 2.0);
    let tool = block(&k, "notch.tool", Vec3::new(6.0, 0.0, 0.0), 2.0, 2.0, 2.0);
    let source = k.boolean(&OpTag::new("notch"), BooleanOp::Subtract, stock, tool).unwrap().body;
    let layout =
        PatternLayout::Circular { axis_origin: Vec3::ZERO, axis_direction: Vec3::Z, count: 4, angle_deg: 90.0 };
    let result = k.pattern_feature(&OpTag::new("ring"), source, source, Some(stock), &layout).unwrap();
    volume(&k, result.body, 768.0);
    let refused = k
        .pattern_feature(
            &OpTag::new("refused"),
            source,
            source,
            Some(stock),
            &PatternLayout::Circular { axis_origin: Vec3::ZERO, axis_direction: Vec3::Z, count: 4, angle_deg: 45.0 },
        )
        .unwrap_err();
    assert!(refused.to_string().contains("cannot be saved exactly"), "{refused}");
    volume(&k, source, 792.0);
}
