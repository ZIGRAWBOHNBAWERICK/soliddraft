//! The import report: what each imported file opened as and why, refused files included.

use soliddraft_app::document::Document;
use soliddraft_app::import::report::{Refusal, Tier, rows};
use soliddraft_app::import::{Preview, Review, commit};
use soliddraft_kernel_api::ImportFormat;

fn import(doc: &mut Document, name: &str, format: ImportFormat, source: &[u8]) {
    let preview = Preview::from_source(doc, name, source.to_vec(), format).unwrap();
    commit(doc, &preview, &Review::default()).unwrap();
}

fn imported(name: &str, format: ImportFormat, source: &[u8]) -> Document {
    let mut doc = Document::new("Report", soliddraft_app::kernel());
    import(&mut doc, name, format, source);
    doc
}

/// Each tier with its reason: an exact box, a block whose seam closed within its bound,
/// and a surface model.
#[test]
fn each_import_says_what_it_opened_as_and_why() {
    let cases: [(&str, ImportFormat, &[u8], Tier, &str); 4] = [
        (
            "occt-box.igs",
            ImportFormat::Iges,
            include_bytes!("../../../ci/fixtures/import/occt-box.igs"),
            Tier::Exact,
            "Closed as it is.",
        ),
        (
            "tetrahedron.obj",
            ImportFormat::Mesh,
            include_bytes!("../../../ci/fixtures/import/tetrahedron.obj"),
            Tier::Exact,
            "Closed as it is.",
        ),
        (
            "block-seam-4um.step",
            ImportFormat::Step,
            include_bytes!("../../../ci/fixtures/import/block-seam-4um.step"),
            Tier::Repaired,
            "Seam closed automatically within 0.01 mm; 4 faces repaired within a stated deviation from the source.",
        ),
        (
            "surface-square.step",
            ImportFormat::Step,
            include_bytes!("data/surface-square.step"),
            Tier::Surface,
            "The file holds open faces, not a closed solid. No volume or mass.",
        ),
    ];
    for (name, format, source, tier, why) in cases {
        let doc = imported(name, format, source);
        let report = rows(&doc, &[]);
        assert_eq!(report.len(), 1, "{name}");
        assert_eq!((report[0].file.as_str(), report[0].tier, report[0].why.as_str()), (name, tier, why));
        assert_eq!(report[0].feature, Some(doc.features[0].id), "{name}");
    }
}

/// A curved import says which check it did not run.
#[test]
fn a_curved_import_names_the_check_not_run() {
    let doc = imported("fillet.step", ImportFormat::Step, include_bytes!("../../../site/parts/fillet.step"));
    let report = rows(&doc, &[]);
    assert!(report[0].not_checked.as_deref().is_some_and(|note| note.starts_with("self-intersection not verified")));
}

/// A refused file has no feature, so the report lists it after the part's imports.
#[test]
fn a_refused_file_is_listed_after_the_imports() {
    let doc = imported("occt-box.igs", ImportFormat::Iges, include_bytes!("../../../ci/fixtures/import/occt-box.igs"));
    let refused =
        Refusal { file: "frame.igs".into(), format: ImportFormat::Iges, reason: "Incidence gap too wide.".into() };
    let report = rows(&doc, &[refused]);
    assert_eq!(report.len(), 2);
    assert_eq!((report[1].file.as_str(), report[1].tier, report[1].feature), ("frame.igs", Tier::Refused, None));
    assert_eq!(report[1].why, "Incidence gap too wide.");
}
