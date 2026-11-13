//! Several-body modelling through application commands and the Combine dialog.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui_kittest::kittest::Queryable;
use soliddraft_app::document::{Command, Document, ExtrudeParams, FeatureStatus, ProfileKind, Selection};
use soliddraft_app::settings::FrameLayout;
use soliddraft_kernel_api::BooleanOp;

fn block(doc: &mut Document, width: f64, depth: f64) -> u64 {
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { profile: ProfileKind::Rectangle { width, height: 10.0 }, depth, ..Default::default() },
        via_assistant: false,
    })
    .unwrap()
    .unwrap()
}

fn pair() -> Document {
    let mut doc = Document::new("Several bodies", soliddraft_app::kernel());
    block(&mut doc, 10.0, 10.0);
    doc.new_body();
    block(&mut doc, 20.0, 5.0);
    doc
}

fn volume(doc: &Document, key: u32) -> f64 {
    let body = doc.model().live_bodies().into_iter().find(|(body, _)| *body == key).unwrap().1;
    doc.kernel().mass_properties(body).unwrap().volume
}

fn assert_picks(doc: &Document) {
    for face in &doc.faces {
        let pick = doc.pick_of_face(face.id).expect("each visible face can be picked");
        assert!(matches!(doc.pick(pick), Some(soliddraft_app::document::PickEntry::Face(_, id)) if id == face.id));
    }
}

#[test]
fn selecting_a_body_or_its_face_targets_only_that_body_and_reopens() {
    let mut doc = pair();
    assert_eq!(doc.meshes.len(), 2);
    assert_picks(&doc);
    let first = doc.model().live_bodies()[0].1;
    let first_face = doc.kernel().topology(first).unwrap().faces[0].id;
    doc.target_selection(Some(Selection::Face(first_face)));
    assert_eq!(doc.active_body, 0);
    doc.select_body(1);
    let id = doc.features[1].id;
    let mut params = match &doc.features[1].kind {
        soliddraft_app::document::FeatureKind::Extrude(params) => params.clone(),
        _ => panic!("extrusion"),
    };
    params.depth = 6.0;
    doc.apply(Command::EditExtrude { id, params }).unwrap();
    assert_eq!(doc.model().live_bodies()[0].1, first);
    assert!((volume(&doc, 1) - 1200.0).abs() < 1e-6);
    assert_eq!(doc.features[1].body_key, 1);
    let saved = doc.model().to_bytes().unwrap();
    let back = Document::from_model(soliddraft_model::Document::from_bytes(&saved, soliddraft_app::kernel()).unwrap());
    assert_eq!(back.meshes.len(), 2);
    assert!((volume(&back, 0) - 1000.0).abs() < 1e-6);
    assert!((volume(&back, 1) - 1200.0).abs() < 1e-6);
    assert_picks(&back);
}

#[test]
fn combine_checks_refusals_before_changing_geometry_or_history() {
    let mut doc = pair();
    let before = doc.model().truth();
    let depth = doc.model().undo_depth();
    assert_eq!(doc.combine(0, 0, BooleanOp::Union).unwrap_err(), "Choose two different bodies to combine.");
    assert_eq!(doc.combine(0, 7, BooleanOp::Union).unwrap_err(), "Combine needs two available solid bodies.");
    assert!(doc.combine(1, 0, BooleanOp::Intersect).unwrap().is_some());
    doc.undo().unwrap();
    let mut empty = Document::new("Empty result", soliddraft_app::kernel());
    block(&mut empty, 10.0, 5.0);
    empty.new_body();
    block(&mut empty, 20.0, 10.0);
    let empty_truth = empty.model().truth();
    let empty_depth = empty.model().undo_depth();
    // The target is entirely inside the tool, so subtraction leaves no solid.
    let error = empty.combine(0, 1, BooleanOp::Subtract).unwrap_err();
    assert_eq!(error, "This combine would leave no solid body.");
    assert_eq!(empty.model().truth(), empty_truth);
    assert_eq!(empty.model().undo_depth(), empty_depth);
    assert_eq!(doc.model().truth(), before);
    assert_eq!(doc.model().undo_depth(), depth);
    assert_eq!(empty.meshes.len(), 2);
}

#[test]
fn every_boolean_is_one_undo_step_and_its_operation_can_be_edited() {
    for (operation, expected) in
        [(BooleanOp::Union, 1500.0), (BooleanOp::Subtract, 500.0), (BooleanOp::Intersect, 500.0)]
    {
        let mut doc = pair();
        let depth = doc.model().undo_depth();
        let id = doc.combine(0, 1, operation).unwrap().unwrap();
        assert_eq!(doc.model().undo_depth(), depth + 1);
        assert_eq!(doc.features.last().unwrap().status, FeatureStatus::Ok);
        assert_eq!(doc.meshes.len(), 1);
        assert!((volume(&doc, 0) - expected).abs() < 1e-6);
        doc.undo().unwrap();
        assert_eq!(doc.meshes.len(), 2);
        assert_picks(&doc);
        doc.redo().unwrap();
        doc.edit_combine(id, 1, BooleanOp::Union).unwrap();
        assert!((volume(&doc, 0) - 1500.0).abs() < 1e-6);
    }
}

#[test]
fn new_body_and_combine_are_available_in_the_application() {
    let case = ribbon::case("multibody-combine");
    let mut doc = Document::new("Several bodies", soliddraft_app::kernel());
    block(&mut doc, 10.0, 10.0);
    let mut app = ribbon::app(doc);
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    case.run(&mut app, |app| {
        ribbon::run_command(app, "new_body");
        assert_eq!(app.state().state.doc.active_body, 1);
        block(&mut app.state_mut().state.doc, 20.0, 5.0);
        app.run_steps(8);
        assert_eq!(app.state().state.doc.meshes.len(), 2);
        assert_picks(&app.state().state.doc);
        case.frame(app, "two-bodies");
        ribbon::run_command(app, "combine");
        app.run_steps(4);
        app.get_by_label("Combine bodies").get_by_label("Combine").click();
        app.run_steps(8);
        assert_eq!(app.state().state.doc.model().live_bodies().len(), 1);
        assert!((volume(&app.state().state.doc, 1) - 1500.0).abs() < 1e-6);
        case.frame(app, "combined");
        ribbon::run_command(app, "undo");
        assert_eq!(app.state().state.doc.meshes.len(), 2);
    });
}
