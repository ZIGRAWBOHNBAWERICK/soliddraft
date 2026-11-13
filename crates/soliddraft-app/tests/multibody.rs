//! Several-body modelling through application commands and the Combine dialog.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui_kittest::kittest::Queryable;
use soliddraft_app::command_tools::{CommandTools, ProposalBook, Surface};
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

#[test]
fn editing_combine_refuses_export_until_the_dialog_is_finished() {
    use soliddraft_app::state::AppAction;
    use soliddraft_exchange::Format;

    let case = ribbon::case("combine-export-gate");
    std::fs::write(case.path("config/settings.json"), r#"{"frame_layout":"edit_suite"}"#).unwrap();
    let mut doc = pair();
    let id = doc.combine(0, 1, BooleanOp::Union).unwrap().unwrap();
    let name = doc.feature(id).unwrap().name.clone();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        assert!(!app.state().state.export_part(Format::Stl).unwrap().bytes.is_empty());
        app.state_mut().state.push(AppAction::EditFeature(id));
        app.run_steps(8);
        assert!(app.state().state.editor.is_none());
        assert!(app.state().state.doc.combine_form.is_some());
        app.render().unwrap();
        let error = app.state().state.export_part(Format::Stl).expect_err("the live Combine dialog gates export");
        assert!(error.contains("Finish or cancel"), "{error}");
        assert!(app.query_all_by_label(&format!("{name} proof: Pending")).next().is_some());
        let path = case.path("part.stl");
        std::fs::write(&path, b"previous export").unwrap();
        app.state_mut().state.push(AppAction::Export { format: Format::Stl, path: Some(path.clone()) });
        app.run_steps(8);
        assert_eq!(std::fs::read(&path).unwrap(), b"previous export");
        assert!(app.state().state.note.as_ref().unwrap().text.contains("Finish or cancel"));
        app.get_by_label("Combine bodies").get_by_label("Combine").click();
        app.run_steps(8);
        assert!(app.state().state.doc.combine_form.is_none());
        assert!(!app.state().state.export_part(Format::Stl).unwrap().bytes.is_empty());
        assert!(app.query_all_by_label(&format!("{name} proof: Proved")).next().is_some());
    });
}

fn assert_picks(doc: &Document) {
    for face in &doc.faces {
        let pick = doc.pick_of_face(face.id).expect("each visible face can be picked");
        assert!(matches!(doc.pick(pick), Some(soliddraft_app::document::PickEntry::Face(_, id)) if id == face.id));
    }
}

#[test]
fn ordinary_parts_keep_the_legacy_summary_until_a_second_body_is_started() {
    let mut doc = Document::new("Legacy summary", soliddraft_app::kernel());
    block(&mut doc, 10.0, 10.0);
    let mut book = ProposalBook::default();
    let summary = CommandTools { doc: &mut doc, book: &mut book, surface: Surface::Mcp, drawing: None }.summary();
    assert!(summary.get("active_body").is_none());
    assert!(summary.get("bodies").is_none());
    assert!(summary["features"].as_array().unwrap().iter().all(|f| f.get("body_id").is_none()));

    doc.new_body();
    let summary = CommandTools { doc: &mut doc, book: &mut book, surface: Surface::Mcp, drawing: None }.summary();
    assert_eq!(summary["active_body"], 1);
    assert_eq!(summary["features"][0]["body_id"], 0);
    block(&mut doc, 20.0, 5.0);
    let summary = CommandTools { doc: &mut doc, book: &mut book, surface: Surface::Mcp, drawing: None }.summary();
    assert_eq!(summary["bodies"].as_array().unwrap().len(), 2);
    assert_eq!(summary["features"][1]["body_id"], 1);
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
        app.state_mut().state.show_tree(true);
        app.run_steps(8);
        app.get_by_label("Body 1").click();
        app.run_steps(4);
        assert_eq!(app.state().state.doc.active_body, 0);
        app.get_by_label("Body 2").click();
        app.run_steps(4);
        assert_eq!(app.state().state.doc.active_body, 1);
        let selected = app.state().state.doc.features[1].id;
        assert!(!app.state().state.viewport.dimensions.is_empty());
        assert!(
            app.state().state.viewport.dimensions.iter().all(|dimension| dimension.target
                == soliddraft_app::viewport::dimensions::DimensionTarget::Feature(selected))
        );
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

fn settle(doc: &mut Document) {
    let start = std::time::Instant::now();
    while doc.proof_pending() {
        assert!(start.elapsed() < std::time::Duration::from_secs(120), "background proof timed out");
        doc.poll_proof();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn background_edits_and_failures_keep_the_whole_scene_and_its_picks() {
    let mut doc = pair();
    let id = doc.features[0].id;
    doc.select_body(0);
    let mut params = match &doc.features[0].kind {
        soliddraft_app::document::FeatureKind::Extrude(params) => params.clone(),
        _ => panic!("extrusion"),
    };
    params.depth = 12.0;
    doc.apply_interactive(Command::EditExtrude { id, params: params.clone() }).unwrap();
    settle(&mut doc);
    assert_eq!(doc.meshes.len(), 2);
    assert!((volume(&doc, 0) - 1200.0).abs() < 1e-6);
    assert_picks(&doc);
    doc.new_body();
    block(&mut doc, 3.0, 2.0);
    doc.combine(0, 1, BooleanOp::Subtract).unwrap();
    assert_eq!(doc.meshes.len(), 2);
    let meshes = doc.meshes.clone();
    // Shrinking the target below its tool makes the existing Combine leave no solid.
    params.depth = 4.0;
    doc.apply_interactive(Command::EditExtrude { id, params }).unwrap();
    settle(&mut doc);
    assert_eq!(doc.meshes, meshes, "a failed edit preserves every proved body");
    assert_picks(&doc);
    doc.undo().unwrap();
    assert_eq!(doc.meshes.len(), 2);
}

#[test]
fn suppressing_the_first_body_removes_it_when_the_last_body_is_unchanged() {
    let mut doc = pair();
    let id = doc.features[0].id;
    for _ in 0..2 {
        doc.apply(Command::Suppress { id, suppressed: true }).unwrap();
        assert_eq!(doc.model().live_bodies().len(), 1);
        assert_eq!(doc.meshes.len(), 1);
        assert_eq!(doc.faces.len(), 6);
        doc.undo().unwrap();
        assert_eq!(doc.meshes.len(), 2);
        assert_picks(&doc);
    }
}

#[test]
fn export_refuses_to_silently_drop_a_body() {
    let doc = pair();
    assert_eq!(
        doc.model().export(soliddraft_exchange::Format::Step).unwrap_err(),
        "Several-body export is coming soon. Combine the bodies before exporting."
    );
}

#[test]
fn a_new_body_can_start_on_an_existing_bodys_face_without_joining_it() {
    let mut doc = Document::new("Face-based body", soliddraft_app::kernel());
    block(&mut doc, 10.0, 10.0);
    let body = doc.display_body().unwrap();
    let face = doc.kernel().topology(body).unwrap().faces.into_iter().find(|f| f.centroid.z > 9.9).unwrap().id;
    let reference = doc.face_ref(face).unwrap();
    doc.new_body();
    doc.target_selection(Some(Selection::Face(face)));
    assert_eq!(doc.active_body, 1, "choosing a plane preserves the pending new body");
    let sketch = doc
        .apply(Command::AddSketch { plane: soliddraft_app::document::SketchPlane::Face { face: reference } })
        .unwrap()
        .unwrap();
    assert_eq!(doc.active_body, 1);
    let mut drawing = soliddraft_app::sketch::Sketch::new();
    drawing.rectangle(soliddraft_app::sketch::P2::ZERO, 5.0, 5.0).unwrap();
    doc.apply(Command::EditSketch { id: sketch, sketch: drawing }).unwrap();
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(sketch), depth: 2.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    assert_eq!(doc.meshes.len(), 2);
    assert!((volume(&doc, 0) - 1000.0).abs() < 1e-6);
    assert!((volume(&doc, 1) - 50.0).abs() < 1e-6);
    assert_eq!(doc.features.last().unwrap().body_key, 1);
}

#[test]
fn single_body_consumers_explain_the_multi_body_limit() {
    use soliddraft_assembly::source::{DisplaySource, KernelSource};
    let doc = pair();
    let bytes = doc.model().to_bytes().unwrap();
    let source = KernelSource::new(soliddraft_app::kernel());
    assert_eq!(
        source.part_mesh(&bytes).err().unwrap(),
        "Several-body parts in assemblies are coming soon. Combine the bodies first."
    );
    let state = soliddraft_app::state::AppState::new(doc);
    let make = soliddraft_app::commands::get("make_drawing").unwrap();
    assert!(!make.is_enabled(&state));
    assert_eq!(make.tip(&state), "Several-body drawings are coming soon. Combine the bodies first.");
    assert_eq!(
        soliddraft_app::drawing::create_drawing(&state, &soliddraft_app::drawing::NewDrawing::new()).err().unwrap(),
        "Several-body drawings are coming soon. Combine the bodies first."
    );
}

#[test]
fn a_refused_first_feature_in_a_new_body_keeps_the_existing_body_visible() {
    let mut doc = Document::new("Refused new body", soliddraft_app::kernel());
    block(&mut doc, 10.0, 10.0);
    let before = doc.meshes.clone();
    doc.new_body();
    let sketch = doc
        .apply(Command::AddSketch {
            plane: soliddraft_app::document::SketchPlane::Reference {
                plane: soliddraft_app::document::PlaneKind::Top,
                offset: 0.0,
            },
        })
        .unwrap()
        .unwrap();
    assert_eq!(doc.meshes, before, "an empty new body keeps existing geometry visible");
    let error = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams { sketch: Some(sketch), depth: 2.0, ..Default::default() },
            via_assistant: false,
        })
        .unwrap_err();
    assert!(error.contains("no closed region"), "{error}");
    assert_eq!(doc.meshes, before);
}

#[test]
fn a_body_context_menu_targets_the_body_that_was_clicked() {
    let case = ribbon::case("multibody-context-target");
    let mut app = ribbon::app(pair());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.state_mut().state.show_tree(true);
    app.state_mut().state.doc.select_body(1);
    app.run_steps(8);
    case.run(&mut app, |app| {
        let pos = app.get_by_label("Body 1").rect().center();
        for pressed in [true, false] {
            app.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            app.step();
        }
        app.run_steps(4);
        assert_eq!(app.state().state.doc.active_body, 0);
    });
}
