//! Reference annotations must not renumber inputs or discard expression bindings.

use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{ConstraintKind, EntityKind, P2, Sketch};
use soliddraft_model::{Command, DatumPlane, Document, FeatureKind, SketchFeature, SketchPlane, values::Input};
use std::sync::Arc;

fn fixture() -> (Document, soliddraft_model::FeatureId, u32, u32) {
    let mut sketch = Sketch::new();
    let a = sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 5.0 }, false);
    let b = sketch.add(EntityKind::Circle { center: P2::new(20.0, 0.0), radius: 10.0 }, false);
    let first =
        sketch.constrain(ConstraintKind::Radius { entity: a, value: 5.0, diameter: false, label: P2::new(7.0, 7.0) });
    let second = sketch.constrain(ConstraintKind::Radius {
        entity: b,
        value: 10.0,
        diameter: false,
        label: P2::new(30.0, 10.0),
    });
    let mut doc = Document::new("Reference controls", Arc::new(CatallaxKernel::new()));
    let id = doc
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
    (doc, id, first, second)
}

#[test]
fn reference_dimensions_are_not_editable_inputs_and_keep_other_dimension_names_stable() {
    let (mut doc, id, first, second) = fixture();
    let input = Input::Dimension { feature: id, constraint: second };
    let name = doc.values().parameter_names(doc.features()).into_iter().find(|p| p.input == input).unwrap().name;
    let mut kind = doc.feature(id).unwrap().kind.clone();
    let FeatureKind::Sketch(s) = &mut kind else { unreachable!() };
    s.sketch.set_dimension_reference(first, true).unwrap();
    doc.execute(Command::EditFeature { id, kind }).unwrap();
    let names = doc.values().parameter_names(doc.features());
    assert!(!names.iter().any(|p| p.input == Input::Dimension { feature: id, constraint: first }));
    assert_eq!(names.iter().find(|p| p.input == input).unwrap().name, name);
    let before = doc.part_json();
    let history = doc.undo_depth();
    assert!(doc.execute(Command::SetSketchDimension { id, constraint: first, value: 99.0 }).is_err());
    assert_eq!(doc.part_json(), before);
    assert_eq!(doc.undo_depth(), history);
    doc.undo().unwrap();
    assert!(
        matches!(&doc.feature(id).unwrap().kind,FeatureKind::Sketch(s) if !s.sketch.constraint(first).unwrap().is_reference())
    );
    doc.redo().unwrap();
    assert!(
        matches!(&doc.feature(id).unwrap().kind,FeatureKind::Sketch(s) if s.sketch.constraint(first).unwrap().is_reference())
    );
}

#[test]
fn reference_only_profiles_follow_authored_geometry_through_catallax() {
    use soliddraft_model::{EndCondition, ExtrudeFeature, ExtrudeOp, FeatureStatus, ProfileSource};
    let mut sketch = Sketch::new();
    let circle = sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 5.0 }, false);
    let reference = sketch.constrain(
        ConstraintKind::Radius { entity: circle, value: 999.0, diameter: true, label: P2::new(9.0, 7.0) }
            .as_reference()
            .unwrap(),
    );
    let mut doc = Document::new("Reference profile", Arc::new(CatallaxKernel::new()));
    let id = doc
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
    let boss = doc
        .execute(Command::AddFeature {
            kind: FeatureKind::Extrude(ExtrudeFeature {
                profile: ProfileSource::Sketch { feature: id },
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
    assert_eq!(doc.status(boss), Some(&FeatureStatus::Ok));
    let volume = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
    assert!((volume - 50.0 * std::f64::consts::PI).abs() < 1e-5);
    let mut kind = doc.feature(id).unwrap().kind.clone();
    let FeatureKind::Sketch(s) = &mut kind else { unreachable!() };
    s.sketch.entity_mut(circle).unwrap().kind = EntityKind::Circle { center: P2::ZERO, radius: 8.0 };
    doc.execute(Command::EditFeature { id, kind }).unwrap();
    assert_eq!(doc.status(boss), Some(&FeatureStatus::Ok));
    let FeatureKind::Sketch(s) = &doc.feature(id).unwrap().kind else { unreachable!() };
    assert_eq!(s.sketch.constraint(reference).unwrap().display_value(&s.sketch), Some(16.0));
    let volume = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
    assert!((volume - 128.0 * std::f64::consts::PI).abs() < 1e-5);
}

#[test]
fn an_expression_binding_is_not_silently_removed_by_reference_conversion() {
    let (mut doc, id, first, _) = fixture();
    doc.execute(Command::Drive {
        input: Input::Dimension { feature: id, constraint: first },
        expression: "5 mm".into(),
    })
    .unwrap();
    let before = doc.part_json();
    let history = doc.undo_depth();
    let mut kind = doc.feature(id).unwrap().kind.clone();
    let FeatureKind::Sketch(s) = &mut kind else { unreachable!() };
    s.sketch.set_dimension_reference(first, true).unwrap();
    assert!(doc.execute(Command::EditFeature { id, kind }).is_err());
    assert_eq!(doc.part_json(), before);
    assert_eq!(doc.undo_depth(), history);
    assert_eq!(doc.values().driver(&Input::Dimension { feature: id, constraint: first }), Some("5 mm"));
}
