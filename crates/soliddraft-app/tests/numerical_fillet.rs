//! A fillet Catallax builds within a stated tolerance, as its rolling-ball fillets are:
//! it builds, previews, names its round, takes radius edits and refusals, saves and
//! reopens, and says approximate wherever the part's accuracy is shown.

use std::sync::Arc;

use soliddraft_app::document::{
    APPROXIMATE_BODY_NAME, Command, Document, ExtrudeParams, FeatureId, FeatureStatus, FilletParams, PREVIEW_MADE,
    ProfileKind, Selection,
};
use soliddraft_kernel_api::OpKind;
use soliddraft_kernel_catallax::CatallaxKernel;

const WORDS: &str = "Approximate, built within 0.01 mm. Its display, measurements and mass are estimates";

/// A backend whose fillets come back as numerical bodies.
fn kernel() -> Arc<CatallaxKernel> {
    Arc::new(CatallaxKernel::with_numerical_stand_in(&[OpKind::Fillet]))
}

/// The removed volume of a fillet of radius `r` along a straight right-angled edge `length` long.
fn removed(r: f64, length: f64) -> f64 {
    (1.0 - std::f64::consts::FRAC_PI_4) * r * r * length
}

/// A 10 x 8 x 6 mm block, and a fillet of radius `r` along one of its 10 mm top edges.
fn filleted(r: f64) -> (Document, FeatureId, FeatureId) {
    let mut doc = Document::new("Numerical fillet", kernel());
    let block = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams {
                profile: ProfileKind::Rectangle { width: 10.0, height: 8.0 },
                depth: 6.0,
                ..Default::default()
            },
            via_assistant: false,
        })
        .unwrap()
        .unwrap();
    let edge = top_edge(&doc);
    let fillet =
        doc.apply(Command::AddFillet { params: FilletParams { edges: vec![edge], radius: r } }).unwrap().unwrap();
    (doc, block, fillet)
}

/// A 10 mm top edge of the block, by its persistent name.
fn top_edge(doc: &Document) -> soliddraft_model::EdgeRef {
    let edge = doc.edges.iter().find(|e| (e.length - 10.0).abs() < 1e-6 && (e.midpoint.z - 6.0).abs() < 1e-6).unwrap();
    doc.edge_ref(edge.id).unwrap()
}

fn row<'a>(rows: &'a [(String, String)], name: &str) -> Option<&'a str> {
    rows.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

fn volume(doc: &Document, feature: FeatureId) -> f64 {
    doc.mass_properties(feature).unwrap().volume
}

/// The display sample is faceted, so its volume is an estimate; it stays within 1%.
fn assert_volume(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < expected * 0.01, "{actual} != {expected}");
}

/// The round's faces: those the fillet feature generated.
fn rounds(doc: &Document, fillet: FeatureId) -> Vec<String> {
    doc.faces.iter().filter(|face| face.feature == fillet).map(|face| face.label.clone()).collect()
}

#[test]
fn a_numerical_fillet_builds_names_its_round_and_says_approximate() {
    let (doc, _, fillet) = filleted(2.0);
    assert_eq!(doc.feature(fillet).unwrap().status, FeatureStatus::Ok);
    assert_eq!(doc.body_name(0), APPROXIMATE_BODY_NAME);
    let info = doc.describe(Selection::Feature(fillet)).unwrap();
    assert_eq!(row(&info.details, "Accuracy"), Some(WORDS));
    assert_eq!(rounds(&doc, fillet).len(), 1, "one round face, named by the fillet: {:?}", doc.faces);
    assert_eq!(doc.faces.len(), 7);
    assert_volume(volume(&doc, fillet), 480.0 - removed(2.0, 10.0));
}

#[test]
fn a_numerical_fillet_preview_marks_only_its_round() {
    let (doc, block, _) = filleted(2.0);
    let before = doc.feature(block).unwrap().body;
    let params = FilletParams { edges: vec![top_edge_before(&doc, block)], radius: 3.0 };
    let preview = doc.preview_rounding(&params.blend(), false, before).unwrap();
    let mesh = preview.result.unwrap();
    let marked = mesh.face_features.iter().filter(|f| **f == PREVIEW_MADE).count();
    assert!(marked > 0 && marked < mesh.face_features.len(), "{marked} of {}", mesh.face_features.len());
}

/// The edge the fillet rounds, on the block before the fillet.
fn top_edge_before(doc: &Document, block: FeatureId) -> soliddraft_model::EdgeRef {
    let body = doc.feature(block).unwrap().body.unwrap();
    let model = doc.model();
    let t = model.kernel().topology(body).unwrap();
    let edge = t.edges.iter().find(|e| (e.length - 10.0).abs() < 1e-6 && (e.midpoint.z - 6.0).abs() < 1e-6).unwrap();
    model.edge_ref(edge.id).unwrap()
}

#[test]
fn a_numerical_fillet_takes_a_radius_edit_and_keeps_its_part_when_an_edit_is_refused() {
    let (mut doc, block, fillet) = filleted(2.0);
    let round = rounds(&doc, fillet);
    let edges = vec![top_edge_before(&doc, block)];
    doc.apply(Command::EditFillet { id: fillet, params: FilletParams { edges: edges.clone(), radius: 3.0 } }).unwrap();
    assert_eq!(doc.feature(fillet).unwrap().status, FeatureStatus::Ok);
    assert_eq!(doc.body_name(0), APPROXIMATE_BODY_NAME);
    assert_eq!(rounds(&doc, fillet), round, "the round keeps its name through the edit");
    assert_volume(volume(&doc, fillet), 480.0 - removed(3.0, 10.0));

    // The block is 6 mm tall: no fillet of 7 mm fits along its top edge.
    let refused = doc.apply(Command::EditFillet { id: fillet, params: FilletParams { edges, radius: 7.0 } });
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(doc.feature(fillet).unwrap().status, FeatureStatus::Ok);
    assert_volume(volume(&doc, fillet), 480.0 - removed(3.0, 10.0));
    assert_eq!(doc.body_name(0), APPROXIMATE_BODY_NAME);
}

#[test]
fn a_numerical_fillet_saves_reopens_and_edits_after_reopening() {
    let (mut doc, _, fillet) = filleted(2.0);
    let name = doc.feature(fillet).unwrap().name.clone();
    let round = rounds(&doc, fillet);
    let dir = std::env::temp_dir().join(format!("soliddraft-numerical-fillet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("filleted.dwprt");
    soliddraft_app::file::save(&mut doc, &path).unwrap();
    let mut reopened = soliddraft_app::file::open(&path, kernel(), None).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let id = reopened.features.iter().find(|f| f.name == name).unwrap().id;
    assert_eq!(reopened.feature(id).unwrap().status, FeatureStatus::Ok);
    assert_eq!(reopened.body_name(0), APPROXIMATE_BODY_NAME);
    assert_eq!(rounds(&reopened, id), round);
    assert_volume(volume(&reopened, id), 480.0 - removed(2.0, 10.0));

    let block = reopened.features[0].id;
    let edges = vec![top_edge_before(&reopened, block)];
    reopened.apply(Command::EditFillet { id, params: FilletParams { edges, radius: 1.0 } }).unwrap();
    assert_volume(volume(&reopened, id), 480.0 - removed(1.0, 10.0));
}
