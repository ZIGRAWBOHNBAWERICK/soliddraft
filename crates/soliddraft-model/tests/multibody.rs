//! Body targeting, Combine history and compatibility with existing part files.

use soliddraft_kernel_api::BooleanOp;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::{
    BuiltinShape, Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureId, FeatureKind,
    FeatureStatus, ProfileSource,
};
use std::sync::Arc;

fn block(width: f64, depth: f64) -> FeatureKind {
    FeatureKind::Extrude(ExtrudeFeature {
        profile: ProfileSource::Shape {
            datum: DatumPlane::Top,
            shape: BuiltinShape::Rectangle { width, height: 10.0 },
        },
        operation: ExtrudeOp::Boss,
        end: EndCondition::Blind { depth },
        reverse: false,
    })
}

fn add(doc: &mut Document, body: u32, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeatureInBody { body, kind, via_assistant: false }).unwrap().created.unwrap()
}

fn volume(doc: &Document, key: u32) -> f64 {
    let body = doc.live_bodies().into_iter().find(|(body, _)| *body == key).unwrap().1;
    doc.kernel().mass_properties(body).unwrap().volume
}

fn pair() -> (Document, FeatureId, FeatureId) {
    let mut doc = Document::new("Two bodies", Arc::new(CatallaxKernel::new()));
    let a = add(&mut doc, 0, block(10.0, 10.0));
    let b = add(&mut doc, 1, block(20.0, 5.0));
    (doc, a, b)
}

#[test]
fn body_edits_rollback_and_suppression_leave_other_bodies_unchanged() {
    let (mut doc, a, b) = pair();
    let first = doc.body_after(a).unwrap();
    assert_eq!(doc.live_bodies().len(), 2);
    doc.execute(Command::EditFeature { id: b, kind: block(20.0, 6.0) }).unwrap();
    assert_eq!(doc.body_after(a), Some(first));
    assert!((volume(&doc, 0) - 1000.0).abs() < 1e-6);
    assert!((volume(&doc, 1) - 1200.0).abs() < 1e-6);
    assert!(doc.body_before(b).is_none(), "a new body starts without another body's solid");
    doc.execute(Command::RollTo { before: Some(b) }).unwrap();
    assert_eq!(doc.live_bodies(), vec![(0, first)]);
    doc.undo().unwrap();
    doc.execute(Command::SetSuppressed { id: b, suppressed: true }).unwrap();
    assert_eq!(doc.live_bodies(), vec![(0, first)]);
    doc.undo().unwrap();
    assert_eq!(doc.live_bodies().len(), 2);
}

#[test]
fn every_combine_operation_consumes_only_its_tool_and_survives_history_and_files() {
    for (operation, expected) in
        [(BooleanOp::Union, 1500.0), (BooleanOp::Subtract, 500.0), (BooleanOp::Intersect, 500.0)]
    {
        let (mut doc, _, tool) = pair();
        let other = add(&mut doc, 2, block(3.0, 2.0));
        let untouched = doc.body_after(other).unwrap();
        let before = doc.live_bodies();
        let combine = add(&mut doc, 0, FeatureKind::Combine { operation, tool: 1 });
        assert_eq!(doc.status(combine), Some(&FeatureStatus::Ok));
        assert_eq!(doc.live_bodies().len(), 2);
        assert!(doc.live_bodies().contains(&(2, untouched)));
        assert!((volume(&doc, 0) - expected).abs() < 1e-6);
        doc.undo().unwrap();
        assert_eq!(doc.live_bodies(), before);
        doc.redo().unwrap();
        assert!((volume(&doc, 0) - expected).abs() < 1e-6);
        let hash = doc.geometry_hash();
        let back = Document::from_bytes(&doc.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
        assert_eq!(back.geometry_hash(), hash);
        assert_eq!(back.live_bodies().len(), 2);
        assert!((volume(&back, 0) - expected).abs() < 1e-6);
        // Changing the tool invalidates the Combine build, even though its target is unchanged.
        doc.execute(Command::EditFeature { id: tool, kind: block(20.0, 6.0) }).unwrap();
        let expected = if operation == BooleanOp::Union {
            1600.0
        } else if operation == BooleanOp::Subtract {
            400.0
        } else {
            600.0
        };
        assert!((volume(&doc, 0) - expected).abs() < 1e-6);
    }
}

#[test]
fn old_single_body_part_json_and_archive_keep_their_truth_and_shape() {
    let legacy = serde_json::json!({
        "format":"soliddraft-part/2", "name":"Legacy part", "units":"mm", "next_feature":2,
        "features":[{"id":1,"name":"Boss-Extrude1","kind":{"type":"extrude",
            "profile":{"from":"shape","datum":"top","shape":{"type":"rectangle","width":10.0,"height":10.0}},
            "operation":"boss","end":{"type":"blind","depth":10.0},"reverse":false}}]
    });
    let doc = Document::from_part_json(&legacy.to_string(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(doc.features()[0].body, 0);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&doc.part_json()).unwrap(), legacy);
    assert!((volume(&doc, 0) - 1000.0).abs() < 1e-6);
    let hash = doc.semantic_hash();
    let back = Document::from_bytes(&doc.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(back.semantic_hash(), hash);
    assert_eq!(back.part_json(), doc.part_json());
    assert_eq!(back.features()[0].body, 0);
}
