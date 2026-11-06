//! Corner edits must still form profiles accepted by the existing kernel converter.

use std::sync::Arc;

use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, P2, Sketch, solve, tools::Operation};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, FeatureStatus, ProfileSource,
    SketchFeature, SketchPlane,
};

#[test]
fn trimmed_line_and_arc_corners_extrude_through_catallax() {
    let mut rectangle = Sketch::new();
    let a = rectangle.add(EntityKind::Line { a: P2::new(-20.0, 0.0), b: P2::ZERO }, false);
    let b = rectangle.add(EntityKind::Line { a: P2::new(10.0, -10.0), b: P2::new(10.0, 20.0) }, false);
    rectangle.add(EntityKind::Line { a: P2::new(10.0, 20.0), b: P2::new(-20.0, 20.0) }, false);
    rectangle.add(EntityKind::Line { a: P2::new(-20.0, 20.0), b: P2::new(-20.0, 0.0) }, false);
    rectangle
        .apply_operation(&Operation::TrimCorner {
            options: Default::default(),
            a,
            b,
            keep_a: P2::new(-10.0, 0.0),
            keep_b: P2::new(10.0, 10.0),
        })
        .unwrap();
    let mut sector = Sketch::new();
    let a = sector.add(EntityKind::Arc { center: P2::ZERO, start: P2::new(10.0, 0.0), end: P2::new(0.0, 10.0) }, false);
    let b = sector.add(EntityKind::Line { a: P2::new(0.0, 15.0), b: P2::ZERO }, false);
    sector.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
    sector
        .apply_operation(&Operation::TrimCorner {
            options: Default::default(),
            a,
            b,
            keep_a: P2::new(7.0, 7.0),
            keep_b: P2::new(0.0, 5.0),
        })
        .unwrap();
    for (name, mut sketch, expected) in
        [("Line corner", rectangle, 1200.0), ("Arc corner", sector, 50.0 * std::f64::consts::PI)]
    {
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
