//! A STEP file that holds open faces rather than a closed solid opens as a surface body,
//! named and described as one, and refuses what needs volume in plain words.

use soliddraft_app::document::{Document, FeatureStatus, Selection};
use soliddraft_app::import::{Preview, Review, commit};
use soliddraft_kernel_api::BodyKind;

/// A 2 x 3 mm planar face in a SHELL_BASED_SURFACE_MODEL, from Catallax's import tests.
const SURFACE: &[u8] = include_bytes!("data/surface-square.step");

#[test]
fn a_step_surface_model_opens_as_a_marked_surface_body() {
    let mut doc = Document::new("Surface", soliddraft_app::kernel());
    let preview = Preview::from_bytes(&doc, "surface-square.step", SURFACE.to_vec()).unwrap();
    commit(&mut doc, &preview, &Review::default()).unwrap();
    assert!(doc.features.iter().all(|f| f.status == FeatureStatus::Ok));

    let body = doc.model().body().unwrap();
    let kernel = doc.model().kernel().clone();
    assert_eq!(kernel.topology(body).unwrap().kind, BodyKind::Sheet);
    assert_eq!(kernel.validate(body).unwrap().kind, BodyKind::Sheet);

    assert_eq!(doc.body_name(0), "Surface body");
    doc.selection = Some(Selection::Body);
    let info = doc.describe(Selection::Body).unwrap();
    assert_eq!(info.title, "Surface body");
    assert!(info.details.iter().any(|(name, value)| name == "Kind" && value.starts_with("Surface")));

    let refusal = kernel.mass_properties(body).unwrap_err().to_string();
    assert!(refusal.contains("surface body encloses no volume"), "{refusal}");
}

/// `SURFACE`'s entities renumbered past its own, so both copies fit in one file.
fn second_copy() -> String {
    let text = std::str::from_utf8(SURFACE).unwrap();
    text.split("DATA;\n").nth(1).unwrap().split("ENDSEC;").next().unwrap().replace('#', "#900")
}

/// A file of two surface models imports both, each a surface body; one with a solid and
/// a surface model keeps both, the solid as a solid.
#[test]
fn every_surface_model_of_a_step_file_opens_as_its_own_surface_body() {
    let surface = std::str::from_utf8(SURFACE).unwrap();
    let block = std::str::from_utf8(include_bytes!("../../../ci/fixtures/import/block-seam-4um.step")).unwrap();
    for (name, base, kinds) in [
        ("two-surfaces.step", surface, vec![BodyKind::Sheet, BodyKind::Sheet]),
        ("block-and-surface.step", block, vec![BodyKind::Solid, BodyKind::Sheet]),
    ] {
        let source =
            base.replace("ENDSEC;\nEND-ISO-10303-21;", &format!("{}ENDSEC;\nEND-ISO-10303-21;", second_copy()));
        assert_ne!(source, base, "{name}: the second model was not added");
        let mut doc = Document::new("Surfaces", soliddraft_app::kernel());
        let preview = Preview::from_bytes(&doc, name, source.into_bytes()).unwrap();
        commit(&mut doc, &preview, &Review::default()).unwrap();
        assert!(doc.features.iter().all(|f| f.status == FeatureStatus::Ok), "{name}");
        let kernel = doc.model().kernel().clone();
        let bodies: Vec<_> =
            doc.model().live_bodies().into_iter().map(|(_, body)| kernel.topology(body).unwrap().kind).collect();
        assert_eq!(bodies, kinds, "{name}");
        assert_eq!(doc.body_name(1), "Surface body 2", "{name}");
    }
}
