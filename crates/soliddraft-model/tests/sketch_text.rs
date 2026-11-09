//! Font profiles retain counters and regenerate real Catallax solids.
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, Sketch, SketchText, TextFont};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureId, FeatureKind, FeatureStatus,
    ProfileSource, SketchFeature, SketchPlane,
};
use std::sync::Arc;

/// A Top sketch and a 2 mm blind Boss from it.
fn boss_document(name: &str, sketch: Sketch) -> (Document, FeatureId, FeatureId) {
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
    (doc, sketch_id, extrude_id)
}

fn edit_sketch(doc: &mut Document, id: FeatureId, sketch: Sketch) {
    doc.execute(Command::EditFeature {
        id,
        kind: FeatureKind::Sketch(SketchFeature {
            plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
            sketch,
        }),
    })
    .unwrap();
}

fn assert_volume(doc: &Document, extrude_id: FeatureId, expected: f64) {
    assert_eq!(doc.status(extrude_id), Some(&FeatureStatus::Ok));
    let actual = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

fn total_area(sketch: &Sketch) -> f64 {
    sketch.regions().unwrap().iter().map(|region| region.area()).sum()
}

#[test]
fn text_counters_extrude_and_edits_rebuild_after_reopen() {
    for font in [TextFont::InterRegular, TextFont::InterMedium] {
        let mut sketch = Sketch::new();
        let text_id = sketch
            .add(EntityKind::Text { text: SketchText { content: "o".into(), font, ..Default::default() } }, false);
        let regions = sketch.regions().unwrap();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions.iter().map(|region| region.holes.len()).sum::<usize>(), 1);
        let (mut doc, sketch_id, extrude_id) = boss_document("Sketch text", sketch.clone());
        assert_volume(&doc, extrude_id, total_area(&sketch) * 2.0);
        let EntityKind::Text { text } = &mut sketch.entity_mut(text_id).unwrap().kind else { unreachable!() };
        text.content = "B".into();
        text.height = 15.0;
        text.position.u = 30.0;
        let area = sketch.regions().unwrap()[0].area();
        edit_sketch(&mut doc, sketch_id, sketch);
        assert_volume(&doc, extrude_id, area * 2.0);
        let reopened = Document::from_part_json(&doc.part_json(), Arc::new(CatallaxKernel::new())).unwrap();
        assert_volume(&reopened, extrude_id, area * 2.0);
    }
}

/// The exact "oae" recipe the pinned kernel once refused at its first disjoint union.
#[test]
fn disconnected_word_extrudes_as_one_boss_and_rebuilds_after_edit_and_reopen() {
    let mut sketch = Sketch::new();
    let text_id =
        sketch.add(EntityKind::Text { text: SketchText { content: "oae".into(), ..Default::default() } }, false);
    let regions = sketch.regions().unwrap();
    assert_eq!(regions.len(), 3);
    assert_eq!(regions.iter().map(|region| region.holes.len()).sum::<usize>(), 3);
    assert!(sketch.text_extrusion_notice(regions.len()).is_none());
    let (mut doc, sketch_id, extrude_id) = boss_document("Disconnected text", sketch.clone());
    assert_volume(&doc, extrude_id, total_area(&sketch) * 2.0);
    let reopened = Document::from_part_json(&doc.part_json(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_volume(&reopened, extrude_id, total_area(&sketch) * 2.0);
    let EntityKind::Text { text } = &mut sketch.entity_mut(text_id).unwrap().kind else { unreachable!() };
    text.content = "Bob".into();
    text.font = TextFont::InterMedium;
    text.height = 15.0;
    let regions = sketch.regions().unwrap();
    assert_eq!(regions.len(), 3);
    assert_eq!(regions.iter().map(|region| region.holes.len()).sum::<usize>(), 4);
    let area = total_area(&sketch);
    edit_sketch(&mut doc, sketch_id, sketch);
    assert_volume(&doc, extrude_id, area * 2.0);
    let reopened = Document::from_part_json(&doc.part_json(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_volume(&reopened, extrude_id, area * 2.0);
}
