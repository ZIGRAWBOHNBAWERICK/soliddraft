//! Every value-table path uses the sketch popup's positive-length rule.

use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{ConstraintKind, DistanceKind, EntityKind, P2, PointRef, Sketch, solve};
use soliddraft_model::{Command, DatumPlane, Document, FeatureKind, SketchFeature, SketchPlane, values::Input};
use std::sync::Arc;

fn fixture(family: usize) -> (Document, soliddraft_model::FeatureId, u32, String) {
    let mut sketch = Sketch::new();
    let circle = sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 5.0 }, false);
    let arc = sketch.add(
        EntityKind::Arc { center: P2::new(-10.0, 0.0), start: P2::new(-5.0, 0.0), end: P2::new(-10.0, 5.0) },
        false,
    );
    let line = sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
    let other = sketch.add(EntityKind::Line { a: P2::new(20.0, 6.0), b: P2::new(30.0, 16.0) }, false);
    let point = sketch.add(EntityKind::Point { p: P2::new(20.0, 6.0) }, false);
    let a = PointRef::ORIGIN;
    let b = PointRef::new(point, 0);
    let label = P2::new(5.0, 10.0);
    let kind = match family {
        0 => ConstraintKind::Radius { entity: circle, value: 5.0, diameter: false, label },
        1 => ConstraintKind::Radius { entity: circle, value: 10.0, diameter: true, label },
        2 => ConstraintKind::Radius { entity: arc, value: 5.0, diameter: false, label },
        3 => ConstraintKind::Distance { a, b, kind: DistanceKind::Aligned, value: 436.0_f64.sqrt(), label },
        4 => ConstraintKind::Distance { a, b, kind: DistanceKind::Horizontal, value: 20.0, label },
        5 => ConstraintKind::Distance { a, b, kind: DistanceKind::Vertical, value: 6.0, label },
        6 => ConstraintKind::PointLineDistance { point: b, line, value: 6.0, label },
        _ => ConstraintKind::Angle { a: line, b: other, value: 45.0, label },
    };
    let constraint = sketch.constrain(kind);
    solve::solve(&mut sketch, None, true);
    let mut doc = Document::new("Sketch values", Arc::new(CatallaxKernel::new()));
    let feature = doc
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
    let name = doc
        .values()
        .parameter_names(doc.features())
        .into_iter()
        .find(|p| p.input == Input::Dimension { feature, constraint })
        .unwrap()
        .name;
    (doc, feature, constraint, name)
}

#[test]
fn every_sketch_length_family_refuses_zero_and_negative_cells_atomically() {
    for family in 0..7 {
        let (mut doc, feature, constraint, name) = fixture(family);
        let original = (doc.part_json(), doc.undo_depth());
        for text in ["0", "-8"] {
            let error = doc
                .execute(Command::SetValueCell {
                    configuration: "Default".into(),
                    name: name.clone(),
                    expression: Some(text.into()),
                })
                .unwrap_err()
                .to_string();
            assert!(error.contains("greater than 0"), "family {family}: {error}");
            assert_eq!((doc.part_json(), doc.undo_depth()), original);
        }
        doc.execute(Command::SetValueCell { configuration: "Default".into(), name, expression: Some("12".into()) })
            .unwrap();
        let FeatureKind::Sketch(s) = &doc.feature(feature).unwrap().kind else { unreachable!() };
        assert_eq!(s.sketch.constraint(constraint).unwrap().value(), Some(12.0));
        doc.undo().unwrap();
        assert_eq!(doc.part_json(), original.0);
        doc.redo().unwrap();
        let reopened = Document::from_part_json(&doc.part_json(), Arc::new(CatallaxKernel::new())).unwrap();
        let FeatureKind::Sketch(s) = &reopened.feature(feature).unwrap().kind else { unreachable!() };
        assert_eq!(s.sketch.constraint(constraint).unwrap().value(), Some(12.0));
    }
}

#[test]
fn signed_angles_and_zero_angles_remain_valid_values() {
    let (mut doc, feature, constraint, name) = fixture(7);
    for value in [-30.0, 0.0, 60.0] {
        doc.execute(Command::SetValueCell {
            configuration: "Default".into(),
            name: name.clone(),
            expression: Some(value.to_string()),
        })
        .unwrap();
        let FeatureKind::Sketch(s) = &doc.feature(feature).unwrap().kind else { unreachable!() };
        assert_eq!(s.sketch.constraint(constraint).unwrap().value(), Some(value));
    }
}
