//! Review a drawing across two saved revisions of the P1 bracket in the application: the
//! drawing of the candidate compares its part with the earlier revision, a comment goes on
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
use soliddraft_app::review::{ReviewerMarkup, RevisionPin};
use soliddraft_app::state::AppAction;
use soliddraft_document::identify_file;
use soliddraft_drawing::{Command, PmiSource};

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

    let kernel = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(20_000_000));
    let doc = Document::new("P1 bracket", kernel);
    let mut app = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(move |cc| SolidDraftApp::new(cc, doc, Automation::default()));
    {
        // Revision A as annotated; revision B with the bore at 10 mm instead of 8.
        let state = &mut app.state_mut().state;
        bracket::build(state);
        bracket::annotate(state);
        state.doc.model().save(&baseline).unwrap();
        bracket::call(state, "drive", json!({"sketch": "Sketch4", "dimension": 1, "expression": "10"}));
    }
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

    // Compare, pick the changed bore and comment on its annotation.
    drawing(&mut app, DrawingAction::ReviewRevisions(Some(baseline.clone())));
    assert_eq!(session(&app).selection, Some(DrawingSel::Review));
    app.get_by_label("Changed · Bore diameter").click();
    app.run_steps(2);
    app.get_by_label("Ø8 H7");
    // The drawing tree names the annotation too.
    assert!(app.query_all_by_label("Ø10 H7").next().is_some());
    app.get_by_label("Candidate − baseline: +2 mm");
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
    assert_eq!(on.place, place);
    assert_ne!(pin(&baseline).revision, pin(&candidate).revision);

    // Another revision of the drawing does not take the comment.
    drawing(
        &mut app,
        DrawingAction::Command(Command::SetTitleField { sheet: 0, key: "owner".into(), value: "Rig Shop".into() }),
    );
    drawing(&mut app, DrawingAction::Save(Some(edited_drawing.clone())));
    assert_ne!(pin(&edited_drawing).revision, pin(&drawing_path).revision);
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
    app.get_by_label("Candidate − baseline: +2 mm");

    // A second save of the same comment is refused, and no source changed.
    drawing(&mut app, DrawingAction::SaveMarkup(Some(drawing_path.clone())));
    for (path, before) in [&baseline, &candidate, &drawing_path].into_iter().zip(&sources) {
        assert_eq!(&std::fs::read(path).unwrap(), before, "{} is as it was", path.display());
    }
    drop(app);
    std::fs::remove_dir_all(&root).unwrap();
}
