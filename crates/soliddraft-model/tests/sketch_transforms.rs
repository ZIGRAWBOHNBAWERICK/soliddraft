//! Transformed analytic sketch profiles remain valid at the Catallax boundary.

use std::sync::Arc;

use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, P2, Sketch, tools::Operation};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, FeatureStatus, ProfileSource,
    SketchFeature, SketchPlane,
};

#[test]
fn moved_copied_rotated_and_scaled_arcs_extrude_and_reopen() {
    let mut sketch = Sketch::new();
    let arc = sketch
        .add(EntityKind::Arc { center: P2::new(10.0, 0.0), start: P2::new(12.0, 0.0), end: P2::new(8.0, 0.0) }, false);
    let line = sketch.add(EntityKind::Line { a: P2::new(8.0, 0.0), b: P2::new(12.0, 0.0) }, false);
    let guide = sketch.add(EntityKind::Circle { center: P2::new(10.0, 0.0), radius: 1.0 }, true);
    let mut ids = vec![arc, line, guide];
    sketch.apply_operation(&Operation::Move { entities: ids.clone(), displacement: P2::new(2.0, 3.0) }).unwrap();
    let copied = sketch
        .apply_operation(&Operation::Copy {
            entities: ids.clone(),
            base: P2::new(12.0, 3.0),
            displacement: P2::new(10.0, 0.0),
        })
        .unwrap();
    ids.extend(copied);
    sketch
        .apply_operation(&Operation::Rotate {
            entities: ids.clone(),
            center: P2::new(1.0, 1.0),
            angle_degrees: 90.0,
            copy: false,
        })
        .unwrap();
    sketch.apply_operation(&Operation::Scale { entities: ids, base: P2::new(1.0, 1.0), factor: 0.5 }).unwrap();
    assert_eq!(sketch.regions().unwrap().len(), 2);
    let mut doc = Document::new("Transformed profiles", Arc::new(CatallaxKernel::new()));
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
    assert_eq!(doc.status(extrude_id), Some(&FeatureStatus::Ok));
    let actual = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
    assert!((actual - 2.0 * std::f64::consts::PI).abs() < 1e-5, "{actual}");
    let mut reopened = Document::from_bytes(&doc.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    reopened.rebuild();
    assert_eq!(reopened.status(extrude_id), Some(&FeatureStatus::Ok));
    let actual = reopened.kernel().mass_properties(reopened.body().unwrap()).unwrap().volume;
    assert!((actual - 2.0 * std::f64::consts::PI).abs() < 1e-5, "{actual}");
}
