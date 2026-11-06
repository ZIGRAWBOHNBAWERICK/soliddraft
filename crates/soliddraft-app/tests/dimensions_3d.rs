//! Model-space annotation placement through native pointer input and the real renderer.
use egui::{Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::kittest::Queryable;
use soliddraft_app::document::{Document, Selection, ViewItem};
use soliddraft_app::state::AppAction;
use soliddraft_app::viewport::camera::StandardView;
use soliddraft_app::viewport::dimensions::{DimensionTarget, ShownDimension};
use soliddraft_model::view::AnnotationKey;

#[path = "support/ribbon.rs"]
mod ribbon;
use ribbon::App;

fn frame(app: &mut App, name: &str) {
    app.hover_at(egui::pos2(10.0, 10.0));
    app.run_steps(3);
    let dir = std::env::var_os("SOLIDDRAFT_DIMENSION_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("soliddraft-dimensions-3d"));
    std::fs::create_dir_all(&dir).unwrap();
    app.render().expect("native renderer").save(dir.join(format!("{name}.png"))).unwrap();
}

fn dimension(app: &App, key: &AnnotationKey) -> ShownDimension {
    app.state().state.viewport.dimensions.iter().find(|d| d.key == *key).expect("shown dimension").clone()
}

fn drag(app: &mut App, from: Pos2, to: Pos2) {
    app.hover_at(from);
    app.run_steps(40);
    app.event(egui::Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::default(),
    });
    app.step();
    for i in 1..=12 {
        app.hover_at(from + (to - from) * (i as f32 / 12.0));
        app.step();
    }
    app.event(egui::Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::default(),
    });
    app.run_steps(5);
}

#[test]
fn feature_drag_is_model_space_undoable_and_keeps_value_editing() {
    let _case = ribbon::case("dimensions-3d-feature");
    let mut app = ribbon::app(ribbon::plate());
    app.key_press(Key::Z);
    app.run_steps(30);
    let name = app.state().state.doc.features[0].name.clone();
    let row = app.get_by_label(&name).rect().center();
    ribbon::double_click_at(&mut app, row);
    app.run_steps(5);
    let original = app.state().state.viewport.dimensions.iter().find(|d| d.tip.ends_with("Depth")).unwrap().clone();
    let before_depth = app.state().state.doc.undo_depth();
    let camera = app.state().state.viewport.camera.view();
    let geometry = app.state().state.doc.model().geometry_hash();
    frame(&mut app, "feature-before-drag");
    let to = original.rect.center() + vec2(95.0, -65.0);
    drag(&mut app, original.rect.center(), to);
    let moved = dimension(&app, &original.key);
    assert!(moved.rect.center().distance(to) < 2.0, "label follows pointer: {moved:?}");
    assert!(moved.position.distance(original.position) > 1.0);
    assert_eq!(app.state().state.viewport.camera.view(), camera, "annotation drag never orbits");
    assert_eq!(app.state().state.doc.undo_depth(), before_depth + 1, "one gesture, one history entry");
    assert_eq!(app.state().state.doc.model().geometry_hash(), geometry, "placement never changes geometry");
    frame(&mut app, "feature-after-drag");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert!(dimension(&app, &original.key).position.distance(original.position) < 0.001);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    assert!(dimension(&app, &original.key).position.distance(moved.position) < 0.001);
    for (i, view) in [StandardView::Isometric, StandardView::Front, StandardView::Right].into_iter().enumerate() {
        app.state_mut().state.push(AppAction::SetView(view));
        app.run_steps(40);
        let d = dimension(&app, &original.key);
        let viewport = &app.state().state.viewport;
        let expected = viewport.camera.project(moved.position, viewport.rect).unwrap();
        assert!(d.rect.center().distance(expected) < 1.0, "model anchor at {view:?}");
        assert!(d.rect.is_finite() && d.rect.height() >= 15.0 && d.rect.width() >= 20.0);
        assert_eq!(d.text, "20");
        assert!((d.rect.size() - moved.rect.size()).length() < 0.01, "viewer-facing text stays upright, unscaled");
        for label in &app.state().state.viewport.dimensions {
            assert!(
                ribbon::painted_in(&app, label.rect).iter().any(|p| p.text == label.text && p.rect == p.full),
                "the complete value is painted at {view:?}: {label:?}"
            );
        }
        frame(&mut app, &format!("feature-angle-{i}"));
    }
    let at = dimension(&app, &original.key).rect.center();
    ribbon::double_click_at(&mut app, at);
    assert!(app.state().state.dimension_edit.is_some(), "moved value still edits in place");
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.get_by_label("Depth dimension value").type_text("25");
    app.key_press(Key::Enter);
    app.run_steps(12);
    ribbon::until(&mut app, "the edited depth's proof", |a| !a.state().state.doc.proof_pending());
    assert_eq!(dimension(&app, &original.key).text, "25");
    assert!(dimension(&app, &original.key).position.distance(moved.position) < 0.001);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(12);
    ribbon::until(&mut app, "undo of the value edit", |a| !a.state().state.doc.proof_pending());
    // Geometry undo retains its existing feature-selection behavior.
    app.get_by_label("Solid body").click();
    app.run_steps(5);
    assert_eq!(dimension(&app, &original.key).text, "20");
    assert!(
        dimension(&app, &original.key).position.distance(moved.position) < 0.001,
        "undoing a subsequent geometry edit leaves the earlier placement intact"
    );
    let bytes = app.state().state.doc.model().to_bytes().unwrap();
    let reopened = soliddraft_model::Document::from_bytes(&bytes, soliddraft_app::kernel()).unwrap();
    let mut reopened = ribbon::app(Document::from_model(reopened));
    reopened.state_mut().state.push(AppAction::Select(Some(Selection::Body)));
    reopened.run_steps(6);
    assert!(
        dimension(&reopened, &original.key).position.distance(original.position) < 0.001,
        "the current part format has no 3D placement storage; reopening uses default placement"
    );
}

#[test]
fn every_visible_sketch_dimension_drags_without_selecting_the_body() {
    let _case = ribbon::case("dimensions-3d-sketch");
    let mut doc = ribbon::sketch();
    let id = doc.features[0].id;
    doc.selection = None;
    doc.set_view_choice(ViewItem::Feature(id), Some(true));
    let mut app = ribbon::app(doc);
    app.key_press(Key::Z);
    app.run_steps(30);
    let dims = app.state().state.viewport.dimensions.clone();
    assert_eq!(dims.len(), 2, "both dimensions on a visible unconsumed sketch");
    frame(&mut app, "sketch-before-drag");
    for d in &dims {
        assert!(matches!(d.target, DimensionTarget::Sketch { sketch, .. } if sketch == id));
        let from = dimension(&app, &d.key).rect.center();
        let to = from + vec2(75.0, -45.0);
        drag(&mut app, from, to);
        assert!(dimension(&app, &d.key).rect.center().distance(to) < 2.0);
        assert_ne!(app.state().state.doc.selection, Some(Selection::Body));
    }
    frame(&mut app, "sketch-after-drag");
    let moved = dimension(&app, &dims[1].key);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert!(dimension(&app, &dims[1].key).position.distance(dims[1].position) < 0.001);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert!(dimension(&app, &dims[1].key).position.distance(moved.position) < 0.001);
    app.state_mut().state.doc.set_view_choice(ViewItem::Feature(id), Some(false));
    app.run_steps(4);
    assert!(app.state().state.viewport.dimensions.is_empty(), "hidden sketch has no annotations");
    app.state_mut().state.doc.set_view_choice(ViewItem::Feature(id), Some(true));
    app.run_steps(4);
    assert!(dimension(&app, &dims[1].key).position.distance(moved.position) < 0.001);
    let at = dimension(&app, &dims[0].key).rect.center();
    ribbon::double_click_at(&mut app, at);
    assert_eq!(app.state().state.sketch.as_ref().expect("sketch value editor").feature, id);
}

#[test]
fn escape_cancels_a_dimension_drag_without_orbiting_or_recording_history() {
    let _case = ribbon::case("dimensions-3d-cancel");
    let mut doc = ribbon::plate();
    doc.selection = Some(Selection::Body);
    let mut app = ribbon::app(doc);
    app.key_press(Key::Z);
    app.run_steps(30);
    let label = app.state().state.viewport.dimensions.iter().find(|d| d.tip.ends_with("Depth")).unwrap().clone();
    let depth = app.state().state.doc.undo_depth();
    let camera = app.state().state.viewport.camera.view();
    let from = label.rect.center();
    app.hover_at(from);
    app.run_steps(40);
    app.event(egui::Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.step();
    app.hover_at(from + vec2(90.0, -60.0));
    app.step();
    assert!(dimension(&app, &label.key).position.distance(label.position) > 1.0);
    app.key_press(Key::Escape);
    app.step();
    app.hover_at(from + vec2(120.0, -80.0));
    app.run_steps(3);
    app.event(egui::Event::PointerButton {
        pos: from + vec2(120.0, -80.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(5);
    assert_eq!(app.state().state.doc.selection, Some(Selection::Body));
    assert!(dimension(&app, &label.key).position.distance(label.position) < 0.001);
    assert_eq!(app.state().state.doc.undo_depth(), depth);
    assert_eq!(app.state().state.viewport.camera.view(), camera);
}
