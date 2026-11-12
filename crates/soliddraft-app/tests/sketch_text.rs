//! Native text placement, atomic property edits, history and persistence.
#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use soliddraft_app::{
    commands,
    document::{Command, ExtrudeParams, PlaneKind, SketchPlane},
    sketch::{EntityKind, P2, TextFont},
    sketcher::Frame,
    state::{AppAction, Tool},
};

#[test]
fn text_tool_schema_accepts_bundled_fonts_and_refuses_invalid_text_atomically() {
    use serde_json::json;
    use soliddraft_app::{app_tools::AppTools, document::Document, state::AppState};
    use soliddraft_assistant::{Caller, tools::call_checked};

    let mut state = AppState::new(Document::new("Text tools", soliddraft_app::kernel()));
    let caller = Caller::Mcp { client: "text test".into() };
    for (content, height, valid) in [("oae", 10.0, true), ("", 10.0, false), ("B", -1.0, false)] {
        let before = state.doc.model().part_json();
        let output = call_checked(
            &mut AppTools { state: &mut state },
            "sketch_geometry",
            json!({"plane":"top","operations":[{"tool":"add","construction":false,
                "kind":{"type":"text","text":{"content":content,"font":"inter_regular",
                    "height":height,"position":{"u":0,"v":0}}}}]}),
            &caller,
        );
        assert_eq!(!output.is_error, valid, "{}", output.content);
        if !valid {
            assert_eq!(state.doc.model().part_json(), before);
        }
    }
}

#[test]
fn text_command_places_and_edits_one_retained_entity() {
    let dir = std::env::temp_dir().join(format!("soliddraft-text-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut document = ribbon::blank();
    let id = document
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut app = ribbon::app_at(document, ribbon::EXPANDED_WINDOW, 1.0);
    let command = commands::get("sketch_text").unwrap();
    assert!(!command.is_enabled(&app.state().state));
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(15);
    assert!(command.is_enabled(&app.state().state));
    ribbon::click_tab(&mut app, "Sketch");
    ribbon::click_ribbon(&mut app, command.label);
    assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Text);
    let empty = app.state().state.sketch.as_ref().unwrap().sketch.clone();
    app.key_press(Key::Escape);
    app.run_steps(4);
    assert_eq!(app.state().state.sketch.as_ref().unwrap().sketch, empty);
    assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Select);
    ribbon::click_ribbon(&mut app, command.label);
    {
        let s = app.state_mut().state.sketch.as_mut().unwrap();
        s.text.content = "oae".into();
        s.text.font = TextFont::InterMedium;
        s.auto_relations = false;
    }
    let before = app.state().state.sketch.as_ref().unwrap().sketch.clone();
    let position = P2::new(-15.0, 0.0);
    let state = &app.state().state;
    let at = Frame {
        plane: state.sketch.as_ref().unwrap().plane,
        camera: &state.viewport.camera,
        rect: state.viewport.rect,
    }
    .screen(position)
    .unwrap();
    app.hover_at(at);
    app.run_steps(4);
    assert_eq!(app.state().state.sketch.as_ref().unwrap().sketch, before);
    ribbon::click_at(&mut app, at);
    let placed = app.state().state.sketch.as_ref().unwrap().sketch.clone();
    assert_eq!(placed.drawn().count(), 1);
    assert_eq!(placed.regions().unwrap().len(), 3);
    assert_eq!(placed.regions().unwrap().iter().map(|r| r.holes.len()).sum::<usize>(), 3);
    assert!(matches!(placed.drawn().next().unwrap().kind, EntityKind::Text { .. }));
    if let Some(notice) = placed.text_extrusion_notice(placed.regions().unwrap().len()) {
        app.get_by_label(notice);
        let preview = app.state().state.doc.preview_extrude(&ExtrudeParams { sketch: Some(id), ..Default::default() });
        assert_eq!(preview.err().as_deref(), Some(notice));
    }
    app.state_mut().state.push(AppAction::Undo);
    app.run_steps(5);
    assert_eq!(app.state().state.sketch.as_ref().unwrap().sketch, before);
    app.state_mut().state.push(AppAction::Redo);
    app.run_steps(5);
    assert_eq!(app.state().state.sketch.as_ref().unwrap().sketch, placed);
    ribbon::click_at(&mut app, at);
    {
        let s = app.state_mut().state.sketch.as_mut().unwrap();
        let text = &mut s.text_edit.as_mut().expect("text editor for selected entity").draft;
        text.font = TextFont::InterRegular;
        text.height = 15.0;
        text.position = P2::new(-10.0, 5.0);
    }
    app.get_by_label("Content").focus();
    app.run_steps(2);
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.get_by_label("Content").type_text("B");
    app.run_steps(3);
    assert_eq!(app.state().state.sketch.as_ref().unwrap().text_edit.as_ref().unwrap().draft.content, "B");
    assert_eq!(app.state().state.sketch.as_ref().unwrap().sketch, placed, "buffered until Apply");
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Apply text").scroll_to_me();
    app.run_steps(4);
    app.get_by_role_and_label(egui::accesskit::Role::Button, "Apply text").click();
    app.run_steps(6);
    let edited = app.state().state.sketch.as_ref().unwrap().sketch.clone();
    assert_eq!(
        edited.regions().unwrap().len(),
        1,
        "geometry={:?}; message={:?}",
        edited.entities,
        app.state().state.sketch.as_ref().unwrap().message
    );
    assert_eq!(edited.regions().unwrap()[0].holes.len(), 2);
    app.render().unwrap().save(dir.join("text.png")).unwrap();
    app.state_mut().state.push(AppAction::Undo);
    app.run_steps(5);
    assert_eq!(app.state().state.sketch.as_ref().unwrap().sketch, placed);
    app.state_mut().state.push(AppAction::Redo);
    app.run_steps(5);
    assert_eq!(app.state().state.sketch.as_ref().unwrap().sketch, edited);
    app.key_press(Key::Escape);
    app.run_steps(4);
    for action in [
        AppAction::ExitSketch,
        AppAction::SaveAs(Some(dir.join("text.dwprt"))),
        AppAction::Open(Some(dir.join("text.dwprt"))),
    ] {
        app.state_mut().state.push(action);
        app.run_steps(10);
    }
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, edited);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
