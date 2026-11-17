//! A body Catallax builds within a stated tolerance, rather than exactly, as rolling-ball
//! fillets will be: the part shows it, picks and measures it, saves and reopens it, and
//! writes it to STEP, naming it approximate in plain words wherever it appears.

use std::sync::Arc;

use soliddraft_app::document::{
    APPROXIMATE_BODY_NAME, Command, Document, ExtrudeParams, FeatureStatus, ProfileKind, Selection,
};
use soliddraft_kernel_api::{FaceAccuracy, OpKind, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;

const WORDS: &str = "Approximate, built within 0.01 mm. Its display, measurements and mass are estimates";

/// A backend whose extrusions come back as numerical bodies.
fn kernel() -> Arc<CatallaxKernel> {
    Arc::new(CatallaxKernel::with_numerical_stand_in(&[OpKind::Extrude]))
}

/// A 10 x 8 x 6 mm block, built by the app as a user would.
fn block() -> (Document, soliddraft_app::document::FeatureId) {
    let mut doc = Document::new("Numerical", kernel());
    let id = doc
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
    (doc, id)
}

fn details(doc: &Document, selection: Selection) -> Vec<(String, String)> {
    doc.describe(selection).unwrap().details
}

fn row<'a>(rows: &'a [(String, String)], name: &str) -> Option<&'a str> {
    rows.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

#[test]
fn a_numerical_body_is_named_approximate_in_the_tree_and_details() {
    let (mut doc, extrude) = block();
    assert_eq!(doc.feature(extrude).unwrap().status, FeatureStatus::Ok);
    assert!(doc.proof_state().current_proved_display(), "{:?}", doc.proof_message());
    assert_eq!(doc.body_name(0), APPROXIMATE_BODY_NAME);
    assert_eq!(doc.body_tolerance(0).as_deref(), Some("0.01"));
    doc.selection = Some(Selection::Body);
    let info = doc.describe(Selection::Body).unwrap();
    assert_eq!(info.title, "Approximate solid body");
    assert_eq!(row(&info.details, "Accuracy"), Some(WORDS));
    // No kernel codes or internal names reach the user.
    for (_, value) in &info.details {
        assert!(!value.to_lowercase().contains("numerical"), "{value}");
    }
    let found = soliddraft_app::command_tools::find_entity(&doc, "Approximate solid body");
    assert_eq!(found, Some(Selection::Body));
}

#[test]
fn a_numerical_body_displays_and_its_faces_and_edges_pick_and_measure() {
    let (doc, extrude) = block();
    assert_eq!(doc.faces.len(), 6);
    assert_eq!(doc.edges.len(), 12);
    assert!(doc.faces.iter().all(|face| face.feature == extrude), "every face belongs to the extrusion");
    assert!(doc.faces.iter().all(|face| face.accuracy == FaceAccuracy::Approximate { tolerance_mm: 0.01 }));
    let top = doc.faces.iter().find(|face| (face.centroid.z - 6.0).abs() < 1e-6).unwrap();
    let face = details(&doc, Selection::Face(top.id));
    assert_eq!(row(&face, "Carrier accuracy"), Some("Approximate, built within 0.01 mm"));
    assert_eq!(row(&face, "Area"), Some("80.0 mm²"));

    let measured = soliddraft_app::measure::measure(&doc, Selection::Face(top.id)).unwrap();
    assert_eq!(measured["certainty"], "approximate");
    assert!((measured["value"].as_f64().unwrap() - 80.0).abs() < 1e-6, "{measured}");
    let long = doc.edges.iter().find(|edge| (edge.length - 10.0).abs() < 1e-6).unwrap();
    let measured = soliddraft_app::measure::measure(&doc, Selection::Edge(long.id)).unwrap();
    assert_eq!(measured["certainty"], "approximate");
    assert!((measured["value"].as_f64().unwrap() - 10.0).abs() < 1e-6, "{measured}");
}

#[test]
fn a_numerical_body_has_mass_properties_marked_approximate() {
    let (doc, extrude) = block();
    let mass = doc.mass_properties(extrude).unwrap();
    assert!((mass.volume - 480.0).abs() < 1e-6, "{mass:?}");
    assert!(mass.centroid.distance(Vec3::new(0.0, 0.0, 3.0)) < 1e-6);
    let feature = details(&doc, Selection::Feature(extrude));
    assert_eq!(row(&feature, "Accuracy"), Some(WORDS));
    assert_eq!(row(&feature, "Volume"), Some("480 mm³"));
}

#[test]
fn a_numerical_body_saves_reopens_and_writes_an_approximate_step_file() {
    let (doc, extrude) = block();
    let before = doc.mass_properties(extrude).unwrap();
    let dir = std::env::temp_dir().join(format!("soliddraft-numerical-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("block.dwprt");
    let mut saved = doc;
    soliddraft_app::file::save(&mut saved, &path).unwrap();
    let reopened = soliddraft_app::file::open(&path, kernel(), None).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(reopened.body_name(0), APPROXIMATE_BODY_NAME);
    let id = reopened.features.iter().find(|f| f.name == saved.feature(extrude).unwrap().name).unwrap().id;
    assert_eq!(reopened.mass_properties(id).unwrap(), before);

    let export = reopened.model().export_at(soliddraft_exchange::Format::Step, None).unwrap();
    let text = String::from_utf8(export.bytes.clone()).unwrap();
    assert_eq!(text.matches("ADVANCED_FACE").count(), 6);
    let summary = export.report.summary();
    assert!(summary.contains("approximate part built within 0.01 mm"), "{summary}");
    assert!(summary.contains("the file's distance from the part is not established"), "{summary}");
    assert!(!summary.contains("proven"), "{summary}");
    let proof = export.proof.as_ref().unwrap();
    assert!(
        proof.features.iter().any(|f| f.answer_kind == soliddraft_exchange::proof::AnswerKind::Approximate),
        "{proof:?}"
    );
    let mut written = Vec::new();
    export
        .write_with(std::path::Path::new("block.step"), |file, bytes| {
            written.push((file.to_owned(), bytes.to_vec()));
            Ok(())
        })
        .unwrap();
    assert!(written.iter().any(|(file, bytes)| file.ends_with("block.step") && bytes == &export.bytes));
}
