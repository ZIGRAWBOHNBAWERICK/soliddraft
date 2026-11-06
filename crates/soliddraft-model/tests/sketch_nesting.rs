//! Near-boundary holes must reach real Boss/Cut solids and survive document history.

use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, P2, Sketch};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureId, FeatureKind, FeatureStatus,
    ProfileSource, SketchFeature, SketchPlane,
};
use std::{f64::consts::PI, sync::Arc};

fn add_sketch(doc: &mut Document, sketch: Sketch) -> FeatureId {
    doc.execute(Command::AddFeature {
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
    .unwrap()
}

fn extrude(doc: &mut Document, sketch: FeatureId, operation: ExtrudeOp) -> FeatureId {
    doc.execute(Command::AddFeature {
        kind: FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: sketch },
            operation,
            end: EndCondition::Blind { depth: 2.0 },
            reverse: false,
        }),
        name: None,
        position: None,
        via_assistant: false,
    })
    .unwrap()
    .created
    .unwrap()
}

fn assert_volume(doc: &Document, feature: FeatureId, expected: f64) {
    assert_eq!(doc.status(feature), Some(&FeatureStatus::Ok));
    let volume = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
    assert!((volume - expected).abs() < 1e-5, "{volume} != {expected}");
}

#[test]
fn circular_holes_keep_their_material_in_boss_and_cut_after_history_and_reopen() {
    for center in [P2::new(99.6, 6.5), P2::new(99.81187304123692, 0.0)] {
        for cut in [false, true] {
            let mut doc = Document::new("Near-boundary hole", Arc::new(CatallaxKernel::new()));
            let plate = if cut {
                let mut sketch = Sketch::new();
                let corners =
                    [P2::new(-120.0, -120.0), P2::new(120.0, -120.0), P2::new(120.0, 120.0), P2::new(-120.0, 120.0)];
                for i in 0..4 {
                    sketch.add(EntityKind::Line { a: corners[i], b: corners[(i + 1) % 4] }, false);
                }
                let sketch = add_sketch(&mut doc, sketch);
                Some(extrude(&mut doc, sketch, ExtrudeOp::Boss))
            } else {
                None
            };
            let mut sketch = Sketch::new();
            sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 100.0 }, false);
            sketch.add(EntityKind::Circle { center, radius: 0.1 }, false);
            let authored = sketch.entities.clone();
            let sketch = add_sketch(&mut doc, sketch);
            let feature = extrude(&mut doc, sketch, if cut { ExtrudeOp::Cut } else { ExtrudeOp::Boss });
            let annulus = PI * (10000.0 - 0.01) * 2.0;
            let expected = if cut { 240.0 * 240.0 * 2.0 - annulus } else { annulus };
            assert_volume(&doc, feature, expected);
            if !cut {
                let topology = doc.kernel().topology(doc.body().unwrap()).unwrap();
                assert!(
                    topology.faces.iter().flat_map(|f| &f.origins).any(|origin| matches!(
                        origin.role,
                        soliddraft_kernel_api::FaceRole::Side { loop_index: 1, .. }
                    )),
                    "the hole owns a real inner side face"
                );
            }
            doc.undo().unwrap();
            if let Some(plate) = plate {
                assert_volume(&doc, plate, 240.0 * 240.0 * 2.0);
            } else {
                assert!(doc.body().is_none());
            }
            doc.redo().unwrap();
            assert_volume(&doc, feature, expected);
            let reopened = Document::from_part_json(&doc.part_json(), Arc::new(CatallaxKernel::new())).unwrap();
            assert_volume(&reopened, feature, expected);
            let FeatureKind::Sketch(s) = &reopened.feature(sketch).unwrap().kind else { unreachable!() };
            assert_eq!(s.sketch.entities, authored);
            assert_eq!(s.sketch.regions().unwrap()[0].holes.len(), 1);
        }
    }
}
