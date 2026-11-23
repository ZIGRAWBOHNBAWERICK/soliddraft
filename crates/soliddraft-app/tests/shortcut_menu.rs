#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, accesskit::Role};
use egui_kittest::kittest::Queryable;
use soliddraft_app::assembly::AssemblyAction;
use soliddraft_app::document::{PlaneKind, SketchPlane};
use soliddraft_app::drawing::DrawingAction;
use soliddraft_app::state::{AppAction, Tool};

fn press_s(app: &mut ribbon::App) {
    app.key_press(Key::S);
    app.run_steps(6);
    assert!(app.state().state.palette.is_some());
}

fn dismiss(app: &mut ribbon::App) {
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert!(app.state().state.palette.is_none());
}

fn action(app: &mut ribbon::App, action: AppAction) {
    app.state_mut().state.push(action);
    app.run_steps(8);
}

#[test]
fn s_opens_at_the_cursor_and_runs_the_active_workspaces_commands() {
    let case = ribbon::case("shortcut-menu-workspaces");
    let mut app = ribbon::app(ribbon::plate());
    let hint = &app.state().state.note.as_ref().expect("first use teaches S").text;
    assert_eq!(hint, "Press S to open your shortcut menu.");
    case.run(&mut app, |app| {
        let cursor = app.state().state.viewport.rect.center();
        app.hover_at(cursor);
        press_s(app);
        let search = app.get_by_role_and_label(Role::TextInput, "Search commands").rect();
        assert!((search.left() - cursor.x).abs() < 30.0);
        assert!((search.top() - cursor.y).abs() < 30.0);
        assert!(app.query_by_label("Run Extrude").is_some());
        assert!(app.query_by_label("Run Insert component").is_none());
        case.frame(app, "part");
        dismiss(app);

        action(app, AppAction::StartSketchOn(SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 }));
        assert!(app.state().state.in_sketch());
        press_s(app);
        assert!(app.query_by_label("Run Line").is_some());
        app.get_by_label("Run Rectangle").click();
        app.run_steps(6);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Rectangle);
        action(app, AppAction::ExitSketch);

        action(app, AppAction::Assembly(AssemblyAction::New));
        press_s(app);
        assert!(app.query_by_label("Run Insert component").is_some());
        assert!(app.query_by_label("Run Extrude").is_none());
        dismiss(app);
        action(app, AppAction::Assembly(AssemblyAction::ShowPart));

        action(app, AppAction::Drawing(DrawingAction::MakeDrawing));
        action(app, AppAction::Drawing(DrawingAction::CreateDrawing));
        assert!(app.state().state.drawing.is_active());
        press_s(app);
        assert!(app.query_by_label("Run Model view").is_some());
        assert!(app.query_by_label("Run Line").is_none());
        dismiss(app);
    });
}

#[test]
fn changes_persist_through_restart_and_s_stays_in_a_text_field() {
    let case = ribbon::case("shortcut-menu-preferences");
    let mut app = ribbon::app(ribbon::plate());
    case.run(&mut app, |app| {
        app.get_by_label("Solid body").click();
        app.run_steps(4);
        app.key_press(Key::F2);
        app.run_steps(6);
        app.get_by_role_and_label(Role::TextInput, "Body name").type_text("s");
        app.key_press(Key::S);
        app.run_steps(4);
        assert!(app.state().state.palette.is_none(), "S belongs to the editing field");
        app.key_press(Key::Escape);
        app.run_steps(6);

        press_s(app);
        app.get_by_label("Customize shortcut menu").click();
        app.run_steps(6);
        app.get_by_label("Move Extrude up").click();
        app.run_steps(4);
        app.get_by_label("Remove Measure").click();
        app.run_steps(4);
        let query = app.get_by_role_and_label(Role::TextInput, "Find a command to add");
        query.click();
        query.type_text("Chamfer");
        app.run_steps(6);
        app.get_by_label("Add Chamfer").click();
        app.run_steps(4);
        case.frame(app, "customize");
        app.get_by_label("Save shortcut menu").click();
        app.run_steps(6);
    });
    drop(app);

    let mut reopened = ribbon::app(ribbon::plate());
    case.run(&mut reopened, |app| {
        assert!(app.state().state.note.as_ref().is_none_or(|note| !note.text.contains("Press S")));
        press_s(app);
        let state = &app.state().state;
        let ids: Vec<_> = state.palette.as_ref().unwrap().results(state).into_iter().map(|m| m.command.id).collect();
        assert_eq!(ids, ["extrude", "sketch", "cut", "fillet", "zoom_fit", "chamfer"]);
        let field = app.get_by_role_and_label(Role::TextInput, "Search commands");
        field.type_text("Revolve");
        app.run_steps(6);
        assert!(app.query_by_label("Run Revolve").is_some(), "search finds unpinned commands");
    });
}
