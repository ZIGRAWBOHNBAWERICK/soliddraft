//! Shared-value commands preserve history, formulas, units and configuration boundaries.

use soliddraft_app::{
    document::{Command, Document, ExtrudeParams, FeatureId, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, Sketch, solve},
};
use soliddraft_kernel_catallax::CatallaxKernel;
use std::{f64::consts::PI, sync::Arc};

fn fixture() -> (Document, FeatureId, [u32; 2]) {
    let mut doc = Document::new("Shared values", Arc::new(CatallaxKernel::new()));
    let feature = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut sketch = Sketch::new();
    let mut dimensions = Vec::new();
    for (x, radius) in [(-12.0, 5.0), (12.0, 7.0)] {
        let entity = sketch.add(EntityKind::Circle { center: P2::new(x, 0.0), radius }, false);
        dimensions.push(sketch.constrain(ConstraintKind::Radius {
            entity,
            value: radius * 2.0,
            diameter: true,
            label: P2::new(x, -10.0),
        }));
    }
    solve::solve(&mut sketch, None, true);
    doc.apply(Command::EditSketch { id: feature, sketch }).unwrap();
    (doc, feature, dimensions.try_into().unwrap())
}

fn values(doc: &Document, id: FeatureId, dimensions: [u32; 2]) -> [f64; 2] {
    let sketch = &doc.feature(id).unwrap().sketch().unwrap().sketch;
    dimensions.map(|id| sketch.constraint(id).unwrap().value().unwrap())
}

fn link(id: FeatureId, constraints: Vec<u32>, name: &str, value: Option<f64>) -> Command {
    Command::LinkSketchValues { id, constraints, name: name.into(), value }
}

#[test]
fn linked_values_are_one_undo_step_and_feed_real_catallax_profiles() {
    let (mut doc, id, ids) = fixture();
    let original = doc.model().part_json();
    let depth = doc.model().undo_depth();
    doc.apply(link(id, ids.to_vec(), "Span", None)).unwrap();
    assert_eq!(doc.model().undo_depth(), depth + 1);
    assert_eq!(values(&doc, id, ids), [10.0, 10.0]);
    doc.undo().unwrap();
    assert_eq!(doc.model().part_json(), original);
    doc.redo().unwrap();
    assert_eq!(values(&doc, id, ids), [10.0, 10.0]);
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(id), depth: 2.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let volume = |doc: &Document| doc.model().kernel().mass_properties(doc.model().body().unwrap()).unwrap().volume;
    assert!((volume(&doc) - 100.0 * PI).abs() < 1e-5);
    doc.apply(link(id, vec![ids[1]], "Span", Some(20.0))).unwrap();
    assert_eq!(values(&doc, id, ids), [20.0, 20.0]);
    assert!((volume(&doc) - 400.0 * PI).abs() < 1e-5);
    doc.apply(Command::UnlinkSketchValues { id, constraints: vec![ids[1]] }).unwrap();
    assert_eq!(values(&doc, id, ids), [20.0, 20.0]);
    doc.apply(Command::SetValue { name: "Span".into(), expression: "16 mm".into() }).unwrap();
    assert_eq!(values(&doc, id, ids), [16.0, 20.0]);
}

#[test]
fn shared_angles_keep_signed_and_zero_targets() {
    let (mut doc, id, _) = fixture();
    let mut sketch = doc.feature(id).unwrap().sketch().unwrap().sketch.clone();
    let a = sketch.add(EntityKind::Line { a: P2::new(-5.0, -20.0), b: P2::new(5.0, -20.0) }, false);
    let b = sketch.add(EntityKind::Line { a: P2::new(-5.0, -20.0), b: P2::new(5.0, -10.0) }, false);
    let angle = sketch.constrain(ConstraintKind::Angle { a, b, value: 45.0, label: P2::new(0.0, -15.0) });
    solve::solve(&mut sketch, None, true);
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    doc.apply(Command::SetValue { name: "Bend".into(), expression: "45deg".into() }).unwrap();
    doc.apply(link(id, vec![angle], "Bend", None)).unwrap();
    for value in [-30.0, 0.0, 60.0] {
        doc.apply(link(id, vec![angle], "Bend", Some(value))).unwrap();
        assert_eq!(doc.feature(id).unwrap().sketch().unwrap().sketch.constraint(angle).unwrap().value(), Some(value));
    }
}

#[test]
fn joining_an_existing_formula_does_not_replace_its_definition() {
    let (mut doc, id, ids) = fixture();
    doc.apply(Command::SetValue { name: "Span".into(), expression: "2 * 6 mm".into() }).unwrap();
    let before = doc.model().part_json();
    doc.apply(link(id, ids.to_vec(), "Span", None)).unwrap();
    assert_eq!(values(&doc, id, ids), [12.0, 12.0]);
    let expression =
        doc.model().values_table().globals.into_iter().find(|r| r.name == "Span").unwrap().cells[0].clone();
    assert_eq!(expression.as_deref(), Some("2 * 6 mm"));
    doc.undo().unwrap();
    assert_eq!(doc.model().part_json(), before);
}

#[test]
fn values_cells_cannot_make_a_shared_group_negative_or_zero() {
    let (mut doc, id, ids) = fixture();
    doc.apply(link(id, ids.to_vec(), "Span", None)).unwrap();
    let before = (doc.model().part_json(), doc.model().undo_depth());
    for text in ["-10 mm", "0 mm"] {
        assert!(
            doc.apply(Command::SetValueCell {
                configuration: "Default".into(),
                name: "Span".into(),
                expression: Some(text.into())
            })
            .unwrap_err()
            .contains("greater than 0")
        );
        assert_eq!((doc.model().part_json(), doc.model().undo_depth()), before);
        assert_eq!(values(&doc, id, ids), [10.0, 10.0]);
    }
    doc.apply(Command::SetValueCell {
        configuration: "Default".into(),
        name: "Span".into(),
        expression: Some("18 mm".into()),
    })
    .unwrap();
    assert_eq!(values(&doc, id, ids), [18.0, 18.0]);
    doc.undo().unwrap();
    assert_eq!(values(&doc, id, ids), [10.0, 10.0]);
    doc.redo().unwrap();
    assert_eq!(values(&doc, id, ids), [18.0, 18.0]);
}

#[test]
fn invalid_names_values_targets_and_units_leave_everything_unchanged() {
    let (mut doc, id, ids) = fixture();
    doc.apply(Command::SetValue { name: "Turn".into(), expression: "30 deg".into() }).unwrap();
    for command in [
        link(id, ids.to_vec(), "bad name", None),
        link(id, ids.to_vec(), "Span", Some(0.0)),
        link(id, ids.to_vec(), "Span", Some(f64::NAN)),
        link(id, vec![ids[0], 999], "Span", None),
        link(id, vec![ids[0], ids[0]], "Span", None),
        link(id, ids.to_vec(), "Turn", None),
    ] {
        let before = (doc.model().part_json(), doc.model().undo_depth());
        assert!(doc.apply(command).is_err());
        assert_eq!((doc.model().part_json(), doc.model().undo_depth()), before);
    }
    let mut sketch = doc.feature(id).unwrap().sketch().unwrap().sketch.clone();
    let a = sketch.add(EntityKind::Line { a: P2::new(-5.0, -20.0), b: P2::new(5.0, -20.0) }, false);
    let b = sketch.add(EntityKind::Line { a: P2::new(-5.0, -20.0), b: P2::new(5.0, -10.0) }, false);
    let angle = sketch.constrain(ConstraintKind::Angle { a, b, value: 45.0, label: P2::new(0.0, -15.0) });
    doc.apply(Command::EditSketch { id, sketch: sketch.clone() }).unwrap();
    let before = (doc.model().part_json(), doc.model().undo_depth());
    assert!(doc.apply(link(id, vec![ids[0], angle], "Span", None)).is_err());
    assert_eq!((doc.model().part_json(), doc.model().undo_depth()), before);
    let c = sketch.constraint_mut(ids[1]).unwrap();
    c.kind = c.kind.as_reference().unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let before = (doc.model().part_json(), doc.model().undo_depth());
    assert!(doc.apply(link(id, ids.to_vec(), "Span", None)).is_err());
    assert_eq!((doc.model().part_json(), doc.model().undo_depth()), before);
}

#[test]
fn a_late_binding_cycle_rolls_back_earlier_members_and_preserves_formulas() {
    let (mut doc, id, ids) = fixture();
    let table = doc.model().values_table();
    let second=table.groups.iter().flat_map(|g|&g.rows).find(|r|matches!(r.input,Some(soliddraft_model::values::Input::Dimension {constraint,..}) if constraint==ids[1])).unwrap().name.clone();
    doc.apply(Command::SetValue { name: "Span".into(), expression: second }).unwrap();
    let before = (doc.model().part_json(), doc.model().undo_depth());
    assert!(doc.apply(link(id, ids.to_vec(), "Span", None)).is_err());
    assert_eq!((doc.model().part_json(), doc.model().undo_depth()), before);
    assert!(doc.apply(link(id, vec![ids[0]], "Span", Some(12.0))).is_err());
    assert_eq!((doc.model().part_json(), doc.model().undo_depth()), before);
}

#[test]
fn linking_and_unlinking_are_isolated_to_the_active_configuration() {
    let (mut doc, id, ids) = fixture();
    doc.apply(Command::AddConfiguration { name: "Thin".into() }).unwrap();
    doc.apply(Command::ActivateConfiguration { name: "Thin".into() }).unwrap();
    doc.apply(link(id, ids.to_vec(), "Span", None)).unwrap();
    doc.apply(link(id, vec![ids[0]], "Span", Some(20.0))).unwrap();
    assert_eq!(values(&doc, id, ids), [20.0, 20.0]);
    doc.apply(Command::ActivateConfiguration { name: "Default".into() }).unwrap();
    assert_eq!(values(&doc, id, ids), [10.0, 14.0]);
    doc.apply(Command::ActivateConfiguration { name: "Thin".into() }).unwrap();
    doc.apply(Command::UnlinkSketchValues { id, constraints: vec![ids[1]] }).unwrap();
    doc.apply(Command::SetValueCell {
        configuration: "Thin".into(),
        name: "Span".into(),
        expression: Some("16 mm".into()),
    })
    .unwrap();
    assert_eq!(values(&doc, id, ids), [16.0, 20.0]);
    doc.apply(Command::ActivateConfiguration { name: "Default".into() }).unwrap();
    assert_eq!(values(&doc, id, ids), [10.0, 14.0]);
}
