//! Review a drawing across two saved revisions of the P1 bracket in the application: the
//! drawing of the candidate compares its part with the earlier revision (a changed bore
//! fit, a changed boss diameter, a reworded and a removed face note), a comment goes on
//! the drawing annotation that shows the changed item, and the comment reopens on that
//! revision of the drawing only. Neither part file nor the drawing file changes.

#[path = "support/bracket_pmi.rs"]
mod bracket;

use std::path::Path;
use std::sync::Arc;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::document::Document;
use soliddraft_app::drawing::{DrawingAction, DrawingSel, DrawingSession, LinkState};
use soliddraft_app::review::{FaceOrigin, ReviewerMarkup, RevisionPin, SavedDrawing, SavedRevision};
use soliddraft_app::state::AppAction;
use soliddraft_document::identify_file;
use soliddraft_drawing::{AnnotationKind, AnnotationState, Command, DimValue, PmiSource};
use soliddraft_kernel_api::SurfaceKind;
use soliddraft_kernel_api::pmi::{PmiKind, Tolerance};

type App = Harness<'static, SolidDraftApp>;

fn action(app: &mut App, action: AppAction) {
    app.state_mut().state.push(action);
    app.run_steps(2);
}

fn drawing(app: &mut App, command: DrawingAction) {
    action(app, AppAction::Drawing(command));
}

fn session(app: &App) -> &DrawingSession {
    app.state().state.drawing.session.as_ref().expect("a drawing is open")
}

fn pin(path: &Path) -> RevisionPin {
    let identity = identify_file(path).unwrap();
    RevisionPin { document: identity.id, revision: identity.revision }
}

fn cylindrical_face(state: &soliddraft_app::state::AppState, z: f32) -> FaceOrigin {
    let matches: Vec<_> = state
        .doc
        .faces
        .iter()
        .filter(|face| {
            face.surface == SurfaceKind::Cylinder
                && (face.centroid.x - 30.0).abs() < 1e-3
                && (face.centroid.y - 25.0).abs() < 1e-3
                && (face.centroid.z - z).abs() < 1e-3
        })
        .collect();
    let [face] = matches.as_slice() else { panic!("one cylindrical face at (30, 25, {z})") };
    FaceOrigin::from(&state.doc.model().face_ref(face.id).unwrap())
}

fn review_value(app: &App, column: &str, value: &str) {
    let heading = app.get_by_label(column).rect();
    assert!(
        app.query_all_by_label(value).any(|node| {
            let rect = node.rect();
            (rect.left() - heading.left()).abs() < 1.0
                && rect.top() >= heading.bottom()
                && rect.top() < heading.bottom() + 12.0
        }),
        "{column} does not show {value}"
    );
}

#[test]
fn a_drawing_review_compares_bracket_revisions_and_reopens_its_markup_on_that_drawing_revision() {
    let root = std::env::temp_dir().join(format!("soliddraft-drawing-review-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for folder in ["parts", "config"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    soliddraft_app::file::set_config_dir(Some(root.join("config")));
    let baseline = root.join("parts/bracket-a.dwprt");
    let candidate = root.join("parts/bracket-b.dwprt");
    let drawing_path = root.join("parts/bracket-b.dwdrw");
    let edited_drawing = root.join("parts/bracket-b-owner.dwdrw");
    let markup_path = root.join("bracket-b-review.json");
    let stale_markup_path = root.join("stale-review.json");
    let part_markup_path = root.join("part-review.json");

    let kernel = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(20_000_000));
    let doc = Document::new("P1 bracket", kernel);
    let mut app = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| SolidDraftApp::new(cc, doc, Automation::default()));
    let (baseline_bore, baseline_boss, candidate_bore, candidate_boss) = {
        let state = &mut app.state_mut().state;
        bracket::build(state);
        bracket::annotate(state);
        let faces = bracket::faces(state, 30.0);
        for (name, face, text) in
            [("Bore note", &faces.bore, "Hone after boring"), ("Boss note", &faces.boss, "Deburr the boss rim")]
        {
            bracket::call(state, "pmi_add", json!({"kind": "face_note", "name": name, "faces": [face], "text": text}));
        }
        let baseline_bore = cylindrical_face(state, 7.5);
        let baseline_boss = cylindrical_face(state, 10.0);
        state.doc.model().save(&baseline).unwrap();
        // Revision B: a looser bore fit, a larger boss, one face note reworded, one removed.
        bracket::call(state, "drive", json!({"sketch": "Sketch3", "dimension": 1, "expression": "18"}));
        for id in ["Bore diameter", "Bore note"] {
            let mut item = state.doc.model().pmi().item(id).unwrap().clone();
            match &mut item.kind {
                PmiKind::Size { tolerance, .. } => *tolerance = Tolerance::parse("H8").unwrap(),
                PmiKind::FaceNote { text, .. } => *text = "Hone to size after boring".into(),
                _ => unreachable!(),
            }
            state.doc.put_pmi(item, Some(id.into())).unwrap();
        }
        state.doc.remove_pmi("Boss note").unwrap();
        (baseline_bore, baseline_boss, cylindrical_face(state, 7.5), cylindrical_face(state, 10.0))
    };
    app.run_steps(2);
    action(&mut app, AppAction::SaveAs(Some(candidate.clone())));
    drawing(&mut app, DrawingAction::MakeDrawing);
    drawing(&mut app, DrawingAction::CreateDrawing);
    drawing(&mut app, DrawingAction::Save(Some(drawing_path.clone())));
    assert_eq!(session(&app).link_state(), LinkState::Current, "the drawing shows revision B");
    let bore = session(&app).doc.drawing.sheets[0]
        .annotations
        .iter()
        .find(|a| a.source == Some(PmiSource::Item { item: "Bore diameter".into() }))
        .expect("the drawing shows the bore diameter")
        .id;
    let sources = [&baseline, &candidate, &drawing_path].map(|p| std::fs::read(p).unwrap());

    // Compare: each change shows its value before and after in the review panel.
    drawing(&mut app, DrawingAction::ReviewRevisions(Some(baseline.clone())));
    assert_eq!(session(&app).selection, Some(DrawingSel::Review));
    let changes = [
        ("Changed · Bore diameter", "Ø8 H7", "Ø8 H8"),
        ("Changed · Bore note", "Hone after boring", "Hone to size after boring"),
        ("Changed · Boss diameter", "Ø16 ±0.1", "Ø18 ±0.1"),
        ("Removed · Boss note", "Deburr the boss rim", "Item not present"),
    ];
    for (row, before, after) in changes {
        app.get_by_label(row).click();
        app.run_steps(2);
        review_value(&app, "Baseline", before);
        review_value(&app, "Candidate", after);
        if row == "Changed · Boss diameter" {
            app.get_by_label("Candidate − baseline: +2 mm");
        }
    }

    // The saved revisions hold those values on the bracket's bore and boss faces.
    let kernel = app.state().state.doc.kernel().clone();
    let baseline_revision = SavedRevision::open(&baseline, kernel.clone()).unwrap();
    let candidate_revision = SavedRevision::open(&candidate, kernel).unwrap();
    let compared: Vec<_> = baseline_revision.compare(&candidate_revision).changes.iter().map(|c| c.id).collect();
    assert_eq!(compared, ["Bore diameter", "Bore note", "Boss diameter", "Boss note"]);
    let callout = |revision: &SavedRevision, item: &str| {
        let state = revision.item(item).unwrap();
        state.item.callout(state.nominal_mm.as_deref().unwrap_or_default())
    };
    for (revision, item, face, value) in [
        (&baseline_revision, "Bore diameter", &baseline_bore, "Ø8 H7"),
        (&baseline_revision, "Bore note", &baseline_bore, "Hone after boring"),
        (&baseline_revision, "Boss diameter", &baseline_boss, "Ø16 ±0.1"),
        (&baseline_revision, "Boss note", &baseline_boss, "Deburr the boss rim"),
        (&candidate_revision, "Bore diameter", &candidate_bore, "Ø8 H8"),
        (&candidate_revision, "Bore note", &candidate_bore, "Hone to size after boring"),
        (&candidate_revision, "Boss diameter", &candidate_boss, "Ø18 ±0.1"),
    ] {
        assert_eq!(revision.item(item).unwrap().item.features(), [std::slice::from_ref(face)], "{item}");
        assert_eq!(callout(revision, item), value);
    }
    assert!(candidate_revision.item("Boss note").is_none());
    for (revision, diameter) in [(&baseline_revision, 16.0), (&candidate_revision, 18.0)] {
        let measured = revision.item("Boss diameter").unwrap().nominal_mm.clone().unwrap();
        assert!(matches!(measured[..], [d] if (d - diameter).abs() < 1e-6), "{measured:?}");
    }
    candidate_revision
        .markup(Some("Bore diameter"), "Part comment".into())
        .unwrap()
        .save_new(&part_markup_path)
        .unwrap();
    drawing(&mut app, DrawingAction::OpenMarkup(Some(part_markup_path)));
    assert!(session(&app).review.as_ref().unwrap().draft().is_none());
    app.get_by_label("Changed · Bore diameter").click();
    app.run_steps(2);
    review_value(&app, "Baseline", "Ø8 H7");
    review_value(&app, "Candidate", "Ø8 H8");
    app.get_by_label("Attach markup to selected item").click();
    app.run_steps(2);
    app.get_by_label("On drawing sheet 1");
    let place = *session(&app).review.as_ref().unwrap().place().expect("the comment is on the drawing");
    assert_eq!((place.sheet, place.annotation), (0, bore));
    app.get_by_label("Comment").focus();
    app.run_steps(2);
    app.get_by_label("Comment").type_text("Check the bore fit on the drawing.");
    app.run_steps(2);
    drawing(&mut app, DrawingAction::SaveMarkup(Some(markup_path.clone())));
    let saved = ReviewerMarkup::open(&markup_path).expect("the markup is saved");
    assert_eq!(saved.candidate, pin(&candidate));
    assert_eq!(saved.item_id.as_deref(), Some("Bore diameter"));
    assert_eq!(saved.text, "Check the bore fit on the drawing.");
    let on = saved.drawing.as_ref().expect("the markup names the drawing");
    assert_eq!(on.drawing, pin(&drawing_path));
    assert_eq!((on.place.sheet, on.place.annotation), (place.sheet, place.annotation));
    let sidecar: serde_json::Value = serde_json::from_slice(&std::fs::read(&markup_path).unwrap()).unwrap();
    assert_eq!(sidecar["drawing"]["place"].as_object().unwrap().len(), 2);
    let mut no_item = saved;
    no_item.item_id = None;
    assert!(no_item.place_on(&SavedDrawing::open(&drawing_path).unwrap()).is_none());
    assert_ne!(pin(&baseline).revision, pin(&candidate).revision);

    // Another revision of the drawing does not take the comment.
    drawing(
        &mut app,
        DrawingAction::Command(Command::SetTitleField { sheet: 0, key: "owner".into(), value: "Rig Shop".into() }),
    );
    drawing(&mut app, DrawingAction::ReviewRevisions(Some(baseline.clone())));
    drawing(&mut app, DrawingAction::OpenMarkup(Some(markup_path.clone())));
    drawing(&mut app, DrawingAction::SaveMarkup(Some(stale_markup_path.clone())));
    assert!(!stale_markup_path.exists(), "a dirty drawing cannot save review markup");
    drawing(&mut app, DrawingAction::Save(Some(edited_drawing.clone())));
    assert_ne!(pin(&edited_drawing).revision, pin(&drawing_path).revision);
    drawing(&mut app, DrawingAction::OpenMarkup(Some(markup_path.clone())));
    drawing(&mut app, DrawingAction::SaveMarkup(Some(stale_markup_path.clone())));
    assert!(!stale_markup_path.exists(), "the still-open review cannot save markup on another revision");
    drawing(&mut app, DrawingAction::ReviewRevisions(Some(baseline.clone())));
    drawing(&mut app, DrawingAction::OpenMarkup(Some(markup_path.clone())));
    let review = session(&app).review.as_ref().expect("the edited drawing is under review");
    assert!(review.draft().is_none() && review.place().is_none(), "the comment stays off another drawing revision");

    // The drawing it was written on reopens with it, on its annotation.
    drawing(&mut app, DrawingAction::CloseDrawing);
    drawing(&mut app, DrawingAction::OpenFile(drawing_path.clone()));
    drawing(&mut app, DrawingAction::ReviewRevisions(Some(baseline.clone())));
    drawing(&mut app, DrawingAction::OpenMarkup(Some(markup_path.clone())));
    let review = session(&app).review.as_ref().expect("the drawing is under review");
    assert_eq!(review.draft().map(|d| d.text.as_str()), Some("Check the bore fit on the drawing."));
    assert_eq!(review.place().map(|p| (p.sheet, p.annotation)), Some((0, bore)));
    assert_eq!(session(&app).sheet, 0);
    assert_eq!(session(&app).selection, Some(DrawingSel::Review));
    app.get_by_label("Attached to: Bore diameter");
    app.get_by_label("On drawing sheet 1");
    review_value(&app, "Baseline", "Ø8 H7");
    review_value(&app, "Candidate", "Ø8 H8");

    // Reattached to the boss rim, the bore annotation no longer shows the bore diameter,
    // so a comment on that item does not go on it.
    let rim = session(&app).doc.drawing.sheets[0]
        .annotations
        .iter()
        .find(|a| a.source == Some(PmiSource::Item { item: "Boss diameter".into() }))
        .expect("the drawing shows the boss diameter")
        .anchors[0]
        .entity
        .clone();
    drawing(&mut app, DrawingAction::Command(Command::Reattach { annotation: bore, end: 0, entity: rim }));
    let moved = session(&app).doc.drawing.annotation(bore).expect("the annotation stays on the sheet");
    assert_eq!(moved.state, AnnotationState::Ok);
    let AnnotationKind::Dimension(dimension) = &moved.body else { panic!("the bore annotation is a dimension") };
    assert!(
        matches!(dimension.value, DimValue::Measured { value } if (value - 18.0).abs() < 1e-6),
        "it measures the boss rim: {:?}",
        dimension.value
    );
    assert_eq!(moved.source, None, "the annotation no longer names the bore diameter");
    let moved_path = root.join("parts/bracket-b-moved.dwdrw");
    drawing(&mut app, DrawingAction::Save(Some(moved_path.clone())));
    let moved_drawing = SavedDrawing::open(&moved_path).unwrap();
    assert!(moved_drawing.shows(candidate_revision.pin()));
    let bore_comment = candidate_revision.markup(Some("Bore diameter"), "On the bore".into()).unwrap();
    assert!(bore_comment.on_drawing(&moved_drawing).is_none());
    drawing(&mut app, DrawingAction::ReviewRevisions(Some(baseline.clone())));
    app.get_by_label("Changed · Bore diameter").click();
    app.run_steps(2);
    app.get_by_label("Attach markup to selected item").click();
    app.run_steps(2);
    let review = session(&app).review.as_ref().expect("the moved drawing is under review");
    assert!(review.draft().is_none() && review.place().is_none(), "no annotation shows the bore diameter");

    // A later revision saved as another file and left open does not stand in for the saved
    // revision the drawing shows: the review of that drawing still opens with its comment.
    drawing(&mut app, DrawingAction::CloseDrawing);
    let later = root.join("parts/bracket-c.dwprt");
    bracket::call(&mut app.state_mut().state, "drive", json!({"sketch": "Sketch4", "dimension": 1, "expression": "9"}));
    app.run_steps(2);
    action(&mut app, AppAction::SaveAs(Some(later.clone())));
    assert_eq!(pin(&later).document, pin(&candidate).document, "B and C are revisions of one part");
    assert_ne!(pin(&later).revision, pin(&candidate).revision);
    drawing(&mut app, DrawingAction::OpenFile(drawing_path.clone()));
    assert_eq!(session(&app).model_file(), Some(later.clone()), "the open part C has the drawing's part ID");
    drawing(&mut app, DrawingAction::ReviewRevisions(Some(baseline.clone())));
    drawing(&mut app, DrawingAction::OpenMarkup(Some(markup_path.clone())));
    let review = session(&app).review.as_ref().expect("the saved revision B is under review");
    assert_eq!(review.draft().map(|d| d.text.as_str()), Some("Check the bore fit on the drawing."));
    assert_eq!(review.place().map(|p| (p.sheet, p.annotation)), Some((0, bore)));
    review_value(&app, "Baseline", "Ø8 H7");
    review_value(&app, "Candidate", "Ø8 H8");

    // A second save of the same comment is refused, and no source changed.
    drawing(&mut app, DrawingAction::SaveMarkup(Some(drawing_path.clone())));
    for (path, before) in [&baseline, &candidate, &drawing_path].into_iter().zip(&sources) {
        assert_eq!(&std::fs::read(path).unwrap(), before, "{} is as it was", path.display());
    }
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}
