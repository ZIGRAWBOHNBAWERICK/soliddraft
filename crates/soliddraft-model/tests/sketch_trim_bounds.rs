//! Bounded trim must produce closed analytic profiles for the existing converter.

use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{
    EntityKind, P2, Sketch, solve,
    tools::{Operation, TrimOptions},
};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, FeatureStatus, ProfileSource,
    SketchFeature, SketchPlane,
};
use std::sync::Arc;

#[test]
fn bounded_line_and_wrapping_arc_profiles_extrude_through_catallax() {
    let options = TrimOptions { keep_as_construction: true, ignore_construction: true };
    let mut rectangle = Sketch::new();
    let top = rectangle.add(EntityKind::Line { a: P2::new(-10.0, 4.0), b: P2::new(10.0, 4.0) }, false);
    let bottom = rectangle.add(EntityKind::Line { a: P2::new(10.0, -4.0), b: P2::new(-10.0, -4.0) }, false);
    let a = rectangle.add(EntityKind::Line { a: P2::new(-3.0, -4.0), b: P2::new(-3.0, 4.0) }, false);
    let b = rectangle.add(EntityKind::Line { a: P2::new(3.0, -4.0), b: P2::new(3.0, 4.0) }, false);
    for entity in [top, bottom] {
        rectangle.apply_operation(&Operation::TrimBounded { options, entity, a, b, outside: true }).unwrap();
    }
    let mut sector = Sketch::new();
    let d = 75.0_f64.sqrt();
    let entity =
        sector.add(EntityKind::Arc { center: P2::ZERO, start: P2::new(-5.0, -d), end: P2::new(-5.0, d) }, false);
    let a = sector.add(EntityKind::Line { a: P2::ZERO, b: P2::new(5.0, -d) }, false);
    let b = sector.add(EntityKind::Line { a: P2::ZERO, b: P2::new(5.0, d) }, false);
    sector.apply_operation(&Operation::TrimBounded { options, entity, a, b, outside: true }).unwrap();
    for (name, mut sketch, expected) in
        [("Bounded rectangle", rectangle, 96.0), ("Bounded wrapping arc", sector, 200.0 * std::f64::consts::PI / 3.0)]
    {
        assert!(sketch.drawn().filter(|e| e.construction).count() >= 2, "removed pieces stay out of profiles");
        solve::solve(&mut sketch, None, true);
        let mut doc = Document::new(name, Arc::new(CatallaxKernel::new()));
        let sketch_id = doc
            .execute(Command::AddFeature {
                kind: FeatureKind::Sketch(SketchFeature {
                    plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
                    sketch,
                }),
                name: None,
                position: None,
                via_assistant: false,
            })
            .unwrap()
            .created
            .unwrap();
        let extrude_id = doc
            .execute(Command::AddFeature {
                kind: FeatureKind::Extrude(ExtrudeFeature {
                    profile: ProfileSource::Sketch { feature: sketch_id },
                    operation: ExtrudeOp::Boss,
                    end: EndCondition::Blind { depth: 2.0 },
                    reverse: false,
                }),
                name: None,
                position: None,
                via_assistant: false,
            })
            .unwrap()
            .created
            .unwrap();
        assert_eq!(doc.status(extrude_id), Some(&FeatureStatus::Ok), "{name}");
        let actual = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
        assert!((actual - expected).abs() < 1e-5, "{name}: {actual} != {expected}");
    }
}
