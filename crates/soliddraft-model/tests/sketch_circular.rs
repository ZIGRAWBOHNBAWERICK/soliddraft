//! Circular placement keeps analytic profile area through Catallax and reopen.

use std::sync::Arc;

use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, P2, PointRef, Sketch, solve, tools::Operation};
use soliddraft_model::{
    BuiltinShape, Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, FeatureStatus,
    ProfileSource, SketchFeature, SketchPlane,
};

#[test]
fn circular_tools_build_analytic_boss_and_cut_profiles() {
    let circle = Operation::PerimeterCircle {
        a: P2::new(5.0, 0.0),
        b: P2::new(0.0, 5.0),
        c: P2::new(-5.0, 0.0),
        construction: false,
    };
    let center = Operation::CentrepointArc {
        center: P2::ZERO,
        start: P2::new(5.0, 0.0),
        end: P2::new(0.0, -5.0),
        clockwise: false,
        construction: false,
    };
    let through = Operation::ThreePointArc {
        start: P2::new(5.0, 0.0),
        end: P2::new(0.0, -5.0),
        through: P2::new(-5.0, 0.0),
        construction: false,
    };
    let tangent = Operation::TangentArc { from: PointRef::new(1, 1), end: P2::new(5.0, 5.0), construction: false };
    for (name, operation, expected) in [
        ("Perimeter circle", circle, 50.0 * std::f64::consts::PI),
        ("Centrepoint major arc", center, 25.0 * (1.5 * std::f64::consts::PI + 1.0)),
        ("Three-point major arc", through, 25.0 * (1.5 * std::f64::consts::PI + 1.0)),
        ("Tangent arc", tangent, 50.0 + 12.5 * std::f64::consts::PI),
    ] {
        let mut sketch = Sketch::new();
        if matches!(operation, Operation::TangentArc { .. }) {
            sketch.add(EntityKind::Line { a: P2::new(-5.0, 0.0), b: P2::ZERO }, false);
            sketch.apply_operation(&operation).unwrap();
            sketch.add(EntityKind::Line { a: P2::new(5.0, 5.0), b: P2::new(-5.0, 5.0) }, false);
            sketch.add(EntityKind::Line { a: P2::new(-5.0, 5.0), b: P2::new(-5.0, 0.0) }, false);
        } else {
            sketch.apply_operation(&operation).unwrap();
            if !matches!(operation, Operation::PerimeterCircle { .. }) {
                sketch.add(EntityKind::Line { a: P2::new(0.0, -5.0), b: P2::new(5.0, 0.0) }, false);
            }
        }
        solve::solve(&mut sketch, None, true);
        for cut in [false, true] {
            let mut doc = Document::new(name, Arc::new(CatallaxKernel::new()));
            if cut {
                doc.execute(Command::AddFeature {
                    kind: FeatureKind::Extrude(ExtrudeFeature {
                        profile: ProfileSource::Shape {
                            datum: DatumPlane::Top,
                            shape: BuiltinShape::Rectangle { width: 20.0, height: 20.0 },
                        },
                        operation: ExtrudeOp::Boss,
                        end: EndCondition::Blind { depth: 2.0 },
                        reverse: false,
                    }),
                    name: None,
                    position: None,
                    via_assistant: false,
                })
                .unwrap();
            }
            let sketch_id = doc
                .execute(Command::AddFeature {
                    kind: FeatureKind::Sketch(SketchFeature {
                        plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
                        sketch: sketch.clone(),
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
                        operation: if cut { ExtrudeOp::Cut } else { ExtrudeOp::Boss },
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
            let expected = if cut { 20.0 * 20.0 * 2.0 - expected } else { expected };
            let actual = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
            assert!((actual - expected).abs() < 1e-5, "{name} cut={cut}: {actual} != {expected}");
            let reopened = Document::from_part_json(&doc.part_json(), Arc::new(CatallaxKernel::new())).unwrap();
            assert_eq!(reopened.status(extrude_id), Some(&FeatureStatus::Ok));
            let actual = reopened.kernel().mass_properties(reopened.body().unwrap()).unwrap().volume;
            assert!((actual - expected).abs() < 1e-5, "reopened {name} cut={cut}: {actual} != {expected}");
        }
    }
}
