//! Both spline definitions build and measure through Catallax after reopening.

use std::sync::Arc;

use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, P2, Sketch, SplineMode, solve};
use soliddraft_model::{
    BuiltinShape, Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, FeatureStatus,
    ProfileSource, SketchFeature, SketchPlane,
};

#[test]
fn fit_and_control_spline_profiles_extrude_cut_and_reopen_with_polynomial_volumes() {
    for (mode, points, expected) in [
        (SplineMode::Fit, vec![P2::ZERO, P2::new(2.0, 3.0), P2::new(4.0, 0.0)], 15.0),
        (SplineMode::Control, vec![P2::ZERO, P2::new(0.0, 4.0), P2::new(4.0, 4.0), P2::new(4.0, 0.0)], 96.0 / 5.0),
    ] {
        let name = format!("{mode:?} spline");
        let mut sketch = Sketch::new();
        sketch.add(EntityKind::Spline { mode, points }, false);
        sketch.add(EntityKind::Line { a: P2::new(4.0, 0.0), b: P2::ZERO }, false);
        solve::solve(&mut sketch, None, true);
        for cut in [false, true] {
            let mut doc = Document::new(&name, Arc::new(CatallaxKernel::new()));
            if cut {
                doc.execute(Command::AddFeature {
                    kind: FeatureKind::Extrude(ExtrudeFeature {
                        profile: ProfileSource::Shape {
                            datum: DatumPlane::Top,
                            shape: BuiltinShape::Rectangle { width: 12.0, height: 8.0 },
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
            let expected = if cut { 12.0 * 8.0 * 2.0 - expected } else { expected };
            let actual = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
            assert!((actual - expected).abs() < 1e-5, "{name} cut={cut}: {actual} != {expected}");
            let reopened = Document::from_part_json(&doc.part_json(), Arc::new(CatallaxKernel::new())).unwrap();
            assert_eq!(reopened.status(extrude_id), Some(&FeatureStatus::Ok));
            let actual = reopened.kernel().mass_properties(reopened.body().unwrap()).unwrap().volume;
            assert!((actual - expected).abs() < 1e-5, "reopened {name} cut={cut}: {actual} != {expected}");
        }
    }
}
