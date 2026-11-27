//! Review native saves of the P1 bracket without changing the original P1 fixture.

#[path = "support/bracket_pmi.rs"]
mod bracket;

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use bracket::{annotate, build, call, faces, state};
use serde_json::json;
use soliddraft_app::review::{FaceOrigin, ReviewerMarkup, SavedRevision};
use soliddraft_document::{DocumentId, identify_file};
use soliddraft_kernel_api::pmi::{PmiKind, Tolerance};
use soliddraft_model::{Command, Document};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("soliddraft-review-{}", DocumentId::new()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn reorder(candidate: &mut Document) {
    let mut pmi = candidate.pmi().clone();
    pmi.items.reverse();
    // Native files require the datums to precede the frames that use them.
    pmi.items.sort_by_key(|item| !matches!(item.kind, PmiKind::Datum { .. }));
    for item in &mut pmi.items {
        match &mut item.kind {
            PmiKind::Size { features, .. } | PmiKind::Gdt { features, .. } => features.reverse(),
            PmiKind::Location { to, .. } => to.feature.reverse(),
            _ => {}
        }
    }
    candidate.execute(Command::RestorePmi { state: Box::new(pmi) }).unwrap();
}

#[test]
fn saved_bracket_revisions_report_fit_finish_and_measured_changes_and_keep_markup_separate() {
    let scratch = Scratch::new();
    let baseline_path = scratch.0.join("bracket-a.dwprt");
    let candidate_path = scratch.0.join("bracket-b.dwprt");
    let reordered_path = scratch.0.join("bracket-b-reordered.dwprt");
    let markup_path = scratch.0.join("bracket-b-review.json");
    let mut app = state("P1 bracket");
    build(&mut app);
    annotate(&mut app);
    let boss = faces(&app, 30.0).boss;
    call(&mut app, "pmi_add", json!({"kind": "finish", "name": "Boss finish", "faces": [boss], "roughness": "Ra 3.2"}));
    app.doc.model().save(&baseline_path).unwrap();

    call(&mut app, "drive", json!({"sketch": "Sketch4", "dimension": 1, "expression": "10"}));
    let kernel = app.doc.model().kernel().clone();
    let mut edited = Document::from_bytes(&app.doc.model().to_bytes().unwrap(), kernel.clone()).unwrap();
    for id in ["Bore diameter", "Bore finish"] {
        let mut item = edited.pmi().item(id).unwrap().clone();
        match &mut item.kind {
            PmiKind::Size { tolerance, .. } => *tolerance = Tolerance::parse("H8").unwrap(),
            PmiKind::SurfaceFinish { text, .. } => *text = "Ra 0.8".into(),
            _ => panic!("unexpected PMI kind for {id}"),
        }
        edited.execute(Command::PutPmiItem { item: Box::new(item), replaces: Some(id.into()) }).unwrap();
    }
    let mut boss = edited.pmi().item("Boss finish").unwrap().clone();
    let boss_face = FaceOrigin::from(&boss.features()[0][0]);
    edited.execute(Command::RemovePmiItem { id: boss.id.clone() }).unwrap();
    boss.id = "Boss finish replacement".into();
    edited.execute(Command::PutPmiItem { item: Box::new(boss), replaces: None }).unwrap();
    edited.save(&candidate_path).unwrap();
    reorder(&mut edited);
    edited.save(&reordered_path).unwrap();
    drop(edited);
    drop(app);

    let original_a = fs::read(&baseline_path).unwrap();
    let original_b = fs::read(&candidate_path).unwrap();
    let baseline = SavedRevision::open(&baseline_path, kernel.clone()).unwrap();
    let candidate = SavedRevision::open(&candidate_path, kernel.clone()).unwrap();
    let reordered = SavedRevision::open(&reordered_path, kernel).unwrap();
    let review = baseline.compare(&candidate);
    assert_eq!(baseline.issues().chain(candidate.issues()).collect::<Vec<_>>(), []);
    assert_eq!(baseline.pin().document, candidate.pin().document);
    let stored = identify_file(&candidate_path).unwrap();
    assert_eq!(candidate.pin().revision, stored.revision);
    assert_ne!(baseline.pin().revision, candidate.pin().revision);
    assert_ne!(candidate.pin().revision, reordered.pin().revision);
    assert!(candidate.compare(&reordered).changes.is_empty());
    for report in [&review, &baseline.compare(&reordered)] {
        let ids: Vec<_> = report.changes.iter().map(|change| change.id).collect();
        assert_eq!(ids, ["Bore diameter", "Bore finish", "Boss finish", "Boss finish replacement"]);
        let [diameter, finish, removed, added] = report.changes.as_slice() else { unreachable!() };
        for (item, fit, nominal) in [(diameter.before.unwrap(), "H7", 8.0), (diameter.after.unwrap(), "H8", 10.0)] {
            let PmiKind::Size { tolerance, .. } = &item.item.kind else { panic!("not a size") };
            assert_eq!(*tolerance, Tolerance::parse(fit).unwrap());
            assert_eq!(item.nominal_mm.as_deref(), Some([nominal].as_slice()));
        }
        assert_eq!(diameter.dimension_delta_mm(), Some(vec![2.0]));
        assert!(diameter.unmatched_faces().baseline_only.is_empty());
        assert!(diameter.unmatched_faces().candidate_only.is_empty());
        for (item, note) in [(finish.before.unwrap(), "Ra 1.6"), (finish.after.unwrap(), "Ra 0.8")] {
            let PmiKind::SurfaceFinish { text, .. } = &item.item.kind else { panic!("not a face finish") };
            assert_eq!(text, note);
        }
        assert!(removed.after.is_none());
        assert_eq!(removed.unmatched_faces().baseline_only, [&boss_face]);
        assert!(added.before.is_none());
        assert_eq!(added.unmatched_faces().candidate_only, [&boss_face]);
    }

    assert!(candidate.markup(Some("Boss finish"), "Review finish".into()).is_none());
    let markup = candidate.markup(Some("Bore diameter"), "Confirm the new fit before release.".into()).unwrap();
    markup.save_new(&markup_path).unwrap();
    let reopened = ReviewerMarkup::open(&markup_path).unwrap();
    assert_eq!(reopened.candidate, *candidate.pin());
    assert_eq!(reopened.item_id.as_deref(), Some("Bore diameter"));
    assert_eq!(reopened.text, "Confirm the new fit before release.");
    assert!(reopened.applies_to(&candidate));
    assert!(!reopened.applies_to(&baseline));
    assert!(!reopened.applies_to(&reordered));
    for path in [&baseline_path, &candidate_path] {
        assert_eq!(reopened.save_new(path).unwrap_err().kind(), ErrorKind::AlreadyExists);
    }
    assert_eq!(fs::read(baseline_path).unwrap(), original_a);
    assert_eq!(fs::read(candidate_path).unwrap(), original_b);
}

#[test]
fn a_missing_face_does_not_become_a_dimension_delta_or_disappear_from_review() {
    let scratch = Scratch::new();
    let baseline_path = scratch.0.join("bracket-a.dwprt");
    let broken_path = scratch.0.join("bracket-without-bore.dwprt");
    let mut app = state("P1 bracket");
    build(&mut app);
    annotate(&mut app);
    app.doc.model().save(&baseline_path).unwrap();
    let kernel = app.doc.model().kernel().clone();
    let mut broken = Document::open(&baseline_path, kernel.clone()).unwrap();
    let bore_feature = broken.pmi().item("Bore diameter").unwrap().features()[0][0].feature;
    broken.execute(Command::RollTo { before: Some(bore_feature) }).unwrap();
    broken.save(&broken_path).unwrap();
    drop(broken);
    drop(app);

    let baseline = SavedRevision::open(&baseline_path, kernel.clone()).unwrap();
    let broken = SavedRevision::open(&broken_path, kernel).unwrap();
    let review = baseline.compare(&broken);
    let bore = review.changes.iter().find(|change| change.id == "Bore diameter").unwrap();
    assert_eq!(bore.before.unwrap().nominal_mm, Some(vec![8.0]));
    assert_eq!(bore.after.unwrap().nominal_mm, None);
    assert_eq!(bore.dimension_delta_mm(), None);
    assert!(review.candidate.issues().any(|(id, reason)| id == "Bore diameter" && !reason.is_empty()));
    assert!(broken.compare(&broken).candidate.issues().any(|(id, _)| id == "Bore diameter"));
}
