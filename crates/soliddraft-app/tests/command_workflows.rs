//! Reference planes through stable command ids and the native options bar.

#[path = "support/ui.rs"]
mod ribbon;

use egui::Key;
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::state::EditorParams;

fn settle(app: &mut App) {
    app.run_steps(30);
}

#[test]
fn reference_plane_editor_creates_a_plane_and_attaches_a_sketch() {
    let case = ribbon::case("reference-plane");
    let mut app = ribbon::app(ribbon::blank());
    case.run(&mut app, |app| {
        settle(app);
        assert!(soliddraft_app::commands::get("reference_plane").unwrap().is_enabled(&app.state().state));
        ribbon::run_command(app, "reference_plane");
        settle(app);
        assert!(matches!(app.state().state.editor.as_ref().map(|e| &e.params), Some(EditorParams::ReferencePlane(_))));
        ribbon::type_field(app, "Offset value", "-6");
        let ok = app.get_by_label("OK").rect();
        ribbon::click_at(app, ok.center());
        ribbon::until(app, "the reference plane", |a| {
            a.state().state.editor.is_none() && a.state().state.doc.features.len() == 1
        });
        settle(app);
        let plane = app.state().state.doc.features[0].clone();
        assert_eq!(plane.name, "Plane1");
        assert_eq!(plane.plane.unwrap().origin.z, -6.0);
        case.frame(app, "reference-plane");
        ribbon::run_command(app, "toggle_tree");
        let eye = app.get_by_label("Hide Plane1").rect();
        ribbon::click_at(app, eye.center());
        settle(app);
        assert_eq!(
            app.state().state.doc.view_choice(soliddraft_app::document::ViewItem::Feature(plane.id)),
            Some(false)
        );
        let eye = app.get_by_label("Show Plane1").rect();
        ribbon::click_at(app, eye.center());
        settle(app);
        let row = app.get_by_label("Plane1").rect();
        ribbon::click_at(app, row.center());
        ribbon::run_command(app, "sketch");
        ribbon::until(app, "the attached sketch", |a| a.state().state.sketch.is_some());
        let session = app.state().state.sketch.as_ref().unwrap();
        assert_eq!(session.plane.origin.z, -6.0);
        case.frame(app, "sketch-on-plane");
    });
}

#[test]
fn reference_plane_on_a_translated_part_is_visible() {
    use soliddraft_app::document::{Command, ExtrudeParams, PlaneKind, SketchPlane};
    let case = ribbon::case("reference-plane-translated");
    let mut doc = ribbon::blank();
    let sketch = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut drawing = soliddraft_sketch::Sketch::new();
    drawing.rectangle(soliddraft_sketch::P2::new(1005.0, 1005.0), 10.0, 10.0).unwrap();
    doc.apply(Command::EditSketch { id: sketch, sketch: drawing }).unwrap();
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(sketch), depth: 5.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        settle(app);
        app.key_press(Key::Z);
        settle(app);
        ribbon::run_command(app, "reference_plane");
        settle(app);
        ribbon::type_field(app, "Offset value", "-6");
        let ok = app.get_by_label("OK").rect();
        ribbon::click_at(app, ok.center());
        ribbon::until(app, "the translated part's plane", |a| {
            a.state().state.editor.is_none() && a.state().state.doc.features.len() == 3
        });
        settle(app);
        case.frame(app, "translated-reference-plane");
        assert!(
            ribbon::painted_in(app, app.state().state.viewport.rect).iter().any(|p| p.text == "Plane1"),
            "the shown plane is labelled in the viewport"
        );
    });
}
