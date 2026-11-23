use serde_json::{Value, json};
use soliddraft_app::{drawing::DrawingDoc, file::open_bytes, kernel};
use soliddraft_document::manifest::{self, Entry};
use soliddraft_drawing::{AnnotationId, AnnotationKind, Attach, Dimension, DimensionKind, HoleRow, PmiSource, Table};
use soliddraft_kernel_api::Point2;
use soliddraft_model::Document;

#[test]
fn open_part_rejects_direction_two_under_the_part_options_tag() {
    let mut truth: Value = serde_json::from_str(include_str!("../../../ci/fixtures/graded-wall.json")).unwrap();
    truth["features"].as_array_mut().unwrap().truncate(1);
    truth["features"][0]["kind"]["direction2"] = json!({"type":"blind","depth":2});
    truth["format"] = json!("soliddraft-part/6");
    let doc = Document::from_part_json(&truth.to_string(), kernel()).unwrap();
    assert_eq!(doc.truth()["format"], "soliddraft-part/6");
    let mut manifest = doc.manifest();
    manifest.format = "soliddraft-part/5".into();
    truth["format"] = json!(manifest.format);
    let bytes = manifest::write_package(&manifest, &[Entry::new("part.json", truth.to_string().into_bytes())]).unwrap();
    let error = open_bytes("direction-two.dwprt", &bytes, kernel(), None).err().expect("format 5 must refuse Direction 2");
    assert!(error.contains("soliddraft-part/6"), "{error}");
}

#[test]
fn drawings_keep_v2_files_and_write_new_annotations_as_v3() {
    const OLD: &[u8] = include_bytes!("../../soliddraft-drawing/tests/fixtures/main-configured.dwdrw");
    let (original, old_meta) = soliddraft_drawing::file::from_bytes(OLD).unwrap();
    assert_eq!(old_meta.format, "soliddraft-drawing/2");
    let mut doc = DrawingDoc::from_bytes(OLD).unwrap();
    let migrated = doc.to_bytes(&old_meta.kernel, false).unwrap();
    let (reopened, meta) = soliddraft_drawing::file::from_bytes(&migrated).unwrap();
    assert_eq!(meta.id, old_meta.id);
    assert_eq!(reopened, original);

    let sheet = &mut doc.drawing.sheets[0];
    let mut new_annotation = sheet.annotations[0].clone();
    let view = new_annotation.view;
    sheet.tables.push(Table::hole(
        Point2::new(20., 30.),
        view,
        vec![HoleRow { coordinates: Point2::new(12., 15.), centre: Point2::new(7., 9.), diameter: 6. }],
    ));
    let mut chamfer = Dimension::measured(DimensionKind::Chamfer, 3., 2);
    chamfer.chamfer_angle = Some(30.);
    for (id, body, source) in [
        (3, AnnotationKind::Dimension(chamfer), None),
        (4, AnnotationKind::Centreline, None),
        (5, AnnotationKind::Weld { size: Some(4.) }, None),
        (
            6,
            AnnotationKind::HoleCallout { text: "M8".into() },
            Some(PmiSource::Thread { feature: 12, placement: 1, item: "Thread1".into() }),
        ),
    ] {
        new_annotation.id = AnnotationId(id);
        new_annotation.body = body;
        new_annotation.source = source;
        if id == 4 {
            let anchor = new_annotation.anchors[0].clone();
            new_annotation.anchors = [Attach::Start, Attach::End, Attach::Start, Attach::End]
                .into_iter()
                .enumerate()
                .map(|(i, attach)| {
                    let mut anchor = anchor.clone();
                    anchor.attach = attach;
                    anchor.cached = Point2::new(i as f64, 2.);
                    anchor
                })
                .collect();
        } else {
            new_annotation.anchors.clone_from(&original.sheets[0].annotations[0].anchors);
        }
        sheet.annotations.push(new_annotation.clone());
    }
    let bytes = doc.to_bytes(&old_meta.kernel, false).unwrap();
    let (_, new_meta) = soliddraft_drawing::file::from_bytes(&bytes).unwrap();
    assert_eq!(new_meta.format, "soliddraft-drawing/3");
    assert_eq!(meta.format, new_meta.format);
    assert_eq!(new_meta.id, old_meta.id);
    assert_eq!(DrawingDoc::from_bytes(&bytes).unwrap().drawing, doc.drawing);
}
