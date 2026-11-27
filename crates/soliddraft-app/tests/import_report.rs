//! The import report: what each imported file opened as and why, refused files included.

use soliddraft_app::document::{Command, Document};
use soliddraft_app::import::report::{Refusal, Tier, rows};
use soliddraft_app::import::{CommitError, Preview, Review, commit, commit_in};
use soliddraft_app::state::AppState;
use soliddraft_kernel_api::ImportFormat;

fn import(doc: &mut Document, name: &str, format: ImportFormat, source: &[u8]) {
    let preview = Preview::from_source(doc, name, source.to_vec(), format).unwrap();
    commit(doc, &preview, &Review::default()).unwrap();
}

fn imported(name: &str, format: ImportFormat, source: &[u8]) -> Document {
    let mut doc = Document::new("Report", soliddraft_app::startup::kernel());
    import(&mut doc, name, format, source);
    doc
}

/// Certified, tolerant and surface imports keep distinct authority.
#[test]
fn each_import_says_what_it_opened_as() {
    let cases: [(&str, ImportFormat, &[u8], Tier); 5] = [
        ("occt-box.igs", ImportFormat::Iges, include_bytes!("../../../ci/fixtures/import/occt-box.igs"), Tier::Exact),
        (
            "tetrahedron.obj",
            ImportFormat::Mesh,
            include_bytes!("../../../ci/fixtures/import/tetrahedron.obj"),
            Tier::Exact,
        ),
        (
            "block-seam-4um.step",
            ImportFormat::Step,
            include_bytes!("../../../ci/fixtures/import/block-seam-4um.step"),
            Tier::Repaired,
        ),
        (
            "native-tolerant.step",
            ImportFormat::Step,
            include_bytes!("data/native-tolerant-vertex.step"),
            Tier::Tolerant,
        ),
        ("surface-square.step", ImportFormat::Step, include_bytes!("data/surface-square.step"), Tier::Surface),
    ];
    for (name, format, source, tier) in cases {
        let doc = imported(name, format, source);
        let report = rows(&doc, &[]);
        assert_eq!(report.len(), 1, "{name}");
        assert_eq!((report[0].file.as_str(), report[0].tier), (name, tier));
        assert_eq!(report[0].feature, Some(doc.features[0].id), "{name}");
    }
}

/// A curved import opens as a plain solid, with no note about checks.
#[test]
fn a_curved_import_opens_as_a_solid() {
    let doc = imported("fillet.step", ImportFormat::Step, include_bytes!("../../../site/parts/fillet.step"));
    let report = rows(&doc, &[]);
    assert_ne!(report[0].tier, Tier::Refused);
    for words in [report[0].tier.label(), report[0].why.as_str()] {
        assert!(!words.to_lowercase().contains("certif") && !words.contains("verified"), "{words}");
    }
    assert_eq!(Tier::Exact.label(), "Solid");
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

/// A file that reads but whose body the part refuses becomes a report row, apart from a
/// review the user can answer again, which does not.
#[test]
fn a_body_refused_at_admission_is_a_report_row_and_a_review_error_is_not() {
    let doc = imported("occt-box.igs", ImportFormat::Iges, include_bytes!("../../../ci/fixtures/import/occt-box.igs"));
    let mut state = AppState::new(doc);
    let block = include_bytes!("../../../ci/fixtures/import/block-seam-4um.step").to_vec();
    let preview = Preview::from_source(&state.doc, "block.step", block, ImportFormat::Step).unwrap();

    let unknown = Review { decline: vec!["g9".into()], accept: Vec::new() };
    assert!(matches!(commit_in(&mut state, &preview, &unknown), Err(CommitError::Review(_))));
    assert_eq!(rows(&state.doc, &state.import_refusals).len(), 1, "a review error is not a refusal");

    let refused = commit_in(&mut state, &preview, &Review::default());
    let reason = "an imported body must come first in the part";
    assert_eq!(refused, Err(CommitError::Refused(reason.into())));
    let report = rows(&state.doc, &state.import_refusals);
    assert_eq!(report.len(), 2);
    assert_eq!(
        (report[1].file.as_str(), report[1].tier, report[1].why.as_str()),
        ("block.step", Tier::Refused, reason)
    );
}

/// A suppressed surface import is not built, so the report says it was not evaluated and
/// never infers that it is exact.
#[test]
fn a_suppressed_import_is_not_evaluated() {
    let mut doc = imported("surface-square.step", ImportFormat::Step, include_bytes!("data/surface-square.step"));
    let id = doc.features[0].id;
    doc.apply(Command::Suppress { id, suppressed: true }).unwrap();
    let report = rows(&doc, &[]);
    assert_eq!((report[0].tier, report[0].why.as_str()), (Tier::NotEvaluated, "Suppressed, so it is not built."));
}

/// A body repair is distinguished from the file-level note on its unchanged siblings.
#[test]
fn a_repair_on_one_body_of_a_file_is_not_spread_to_its_siblings() {
    let seam = std::str::from_utf8(include_bytes!("../../../ci/fixtures/import/block-seam-4um.step")).unwrap();
    let blocks = std::str::from_utf8(include_bytes!("../../../ci/fixtures/import/two-blocks-5um.step")).unwrap();
    // Two exact blocks, renumbered past the seam block's entities, after it in one file.
    let copy = blocks.split("DATA;\n").nth(1).unwrap().split("ENDSEC;").next().unwrap().replace('#', "#900");
    let source = seam.replace("ENDSEC;\nEND-ISO-10303-21;", &format!("{copy}ENDSEC;\nEND-ISO-10303-21;"));
    let doc = imported("mixed.step", ImportFormat::Step, source.as_bytes());
    let report = rows(&doc, &[]);
    let tiers: Vec<Tier> = report.iter().map(|row| row.tier).collect();
    assert_eq!(tiers, [Tier::Repaired, Tier::FileRepaired, Tier::FileRepaired]);
}

#[test]
fn a_refused_step_body_does_not_discard_its_certified_sibling() {
    let original = std::str::from_utf8(include_bytes!("../../../ci/fixtures/import/two-blocks-5um.step")).unwrap();
    for (shell, open_shell, survivor, centre) in [
        ("(#40,#80,#102,#124,#136,#148)", "(#40,#80,#102,#124,#136)", "#300", 30.005),
        ("(#190,#230,#252,#274,#286,#298)", "(#190,#230,#252,#274,#286)", "#150", 10.0),
    ] {
        let source = original.replace(shell, open_shell).into_bytes();
        let mut state = AppState::new(Document::new("Partial import", soliddraft_app::startup::kernel()));
        let preview = Preview::from_source(&state.doc, "mixed.step", source.clone(), ImportFormat::Step).unwrap();
        commit_in(&mut state, &preview, &Review::default()).unwrap();
        assert_eq!(state.doc.features.len(), 1);
        assert_eq!(state.doc.model().live_bodies().len(), 1);
        let soliddraft_app::document::FeatureKind::ImportStep(import) = &state.doc.features[0].kind else {
            panic!("import feature")
        };
        assert_eq!(import.source, source);
        assert_eq!(import.source_solid.as_deref(), Some(survivor));
        let mass = state.doc.kernel().mass_properties(state.doc.display_body().unwrap()).unwrap();
        assert!((mass.volume - 5000.0).abs() < 1e-6);
        assert!((mass.centroid.x - centre).abs() < 1e-6);
        let report = rows(&state.doc, &state.import_refusals);
        assert_eq!((report[0].tier, report[1].tier), (Tier::Exact, Tier::Refused));
        assert!(report[1].feature.is_none());
        assert!(!report[1].why.contains("catallax") && !report[1].why.contains("numbers."));

        let saved = state.doc.model().to_bytes().unwrap();
        let reopened =
            soliddraft_app::file::open_bytes("mixed.dwprt", &saved, soliddraft_app::startup::kernel(), None).unwrap();
        assert_eq!(reopened.model().live_bodies().len(), 1);
        assert_eq!(rows(&reopened, &[])[0].tier, Tier::Exact);
        assert!(
            (reopened.kernel().mass_properties(reopened.display_body().unwrap()).unwrap().volume - 5000.0).abs() < 1e-6
        );
        state.doc.undo().unwrap();
        assert!(state.doc.features.is_empty());
        state.doc.redo().unwrap();
        assert_eq!(state.doc.features.len(), 1);
        assert_eq!(state.doc.model().live_bodies().len(), 1);
    }
}
