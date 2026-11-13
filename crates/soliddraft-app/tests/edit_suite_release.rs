//! L14, L22 and F14 through the app, rendered frames and the real proof worker.

mod gated;
#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Rect, pos2, vec2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use serde_json::json;
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::chat::ChatState;
use soliddraft_app::document::{Command, Document, ExtrudeParams, PlaneKind, ProfileKind, Reference, Selection};
use soliddraft_app::settings::FrameLayout;
use soliddraft_app::state::{AppAction, Tool};
use soliddraft_assistant::tools::call_checked;
use soliddraft_assistant::{Caller, MockProvider, MockTurn};
use std::sync::Arc;

fn suite(doc: Document) -> App {
    let mut app = ribbon::app(doc);
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.state_mut().state.layout.tree.shown = false;
    app.run_steps(8);
    app
}

#[test]
fn ai_overlays_history_counts_turns_and_collapses_after_thirty_idle_seconds() {
    let case = ribbon::case("edit-suite-ai");
    let mut app = suite(ribbon::plate());
    app.state_mut().state.chat = ChatState::connected(
        Arc::new(MockProvider::new(vec![MockTurn::text("The plate is ready.")])),
        "test assistant",
    );
    case.run(&mut app, |app| {
        let viewport = app.state().state.viewport.rect;
        let chip = app.get_by_label("Boss-Extrude1 in history").rect();
        ribbon::type_command(app, "ai");
        app.run_steps(8);
        let session = app.get_by_label("AI session").rect();
        assert_eq!(app.state().state.viewport.rect, viewport, "the session overlays the workspace");
        assert!(session.contains_rect(chip), "the session opens over history");
        assert!(app.get_by_label("Command line").is_focused());
        app.get_by_label("Command line").type_text("Describe the plate");
        app.key_press(Key::Enter);
        ribbon::until(app, "assistant reply", |app| {
            app.state().state.chat.messages.iter().any(|m| m.text == "The plate is ready.")
        });
        assert!(app.query_all_by_label("ai · 1 turn").next().is_some());
        app.hover_at(viewport.center());
        app.run_steps(60 * 20);
        assert!(app.state().state.command_line.session_open);
        assert!(ribbon::painted(app).iter().any(|p| p.text.starts_with("Collapses when idle,")));
        app.hover_at(session.center());
        app.run_steps(60 * 20);
        assert!(app.state().state.command_line.session_open, "pointer activity restarts the timer");
        case.frame(app, "session-countdown");
        app.run_steps(60 * 11);
        assert!(!app.state().state.command_line.session_open);
        assert!(app.query_all_by_label("AI session").next().is_none());
        let ai = app.get_by_label("ai · 1 turn").rect();
        assert!(ribbon::window(app).contains_rect(ai));
        ribbon::click_at(app, ai.center());
        app.run_steps(8);
        assert!(app.state().state.command_line.session_open);
        assert!(app.state().state.chat.messages.iter().any(|m| m.text == "The plate is ready."));
        app.key_press(Key::Escape);
        app.run_steps(8);
        assert!(!app.state().state.command_line.session_open);
        case.frame(app, "collapsed-turn-chip");
    });
}

#[test]
fn narrow_palette_keeps_tools_named_for_accessibility_and_the_ai_chip_visible() {
    let case = ribbon::case("edit-suite-compact");
    let mut app = suite(ribbon::blank());
    case.run(&mut app, |app| {
        app.set_size(vec2(1280.0, 800.0));
        app.run_steps(8);
        let start = ribbon::control(app, "Start sketch", ribbon::window(app));
        assert!(start.rect.width() > 200.0);
        app.set_size(vec2(960.0, 640.0));
        app.run_steps(8);
        let start = ribbon::control(app, "Start sketch", ribbon::window(app));
        assert!(start.rect.width() <= 48.0);
        assert!(app.state().state.viewport.rect.width() > 800.0);
        app.state_mut().state.doc.selection = Some(Selection::Reference(Reference::Plane(PlaneKind::Top)));
        ribbon::click_at(app, start.rect.center());
        app.run_steps(8);
        assert!(app.state().state.in_sketch());
        let line = ribbon::control(app, "Line", Rect::from_min_max(pos2(0.0, 56.0), pos2(48.0, 600.0)));
        assert!(line.rect.width() <= 48.0);
        ribbon::click_at(app, line.rect.center());
        app.run_steps(8);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Line);
        app.hover_at(line.rect.center());
        app.run_steps(60);
        assert!(ribbon::window(app).contains_rect(app.get_by_label("ai").rect()));
        case.frame(app, "960-compact");
    });
}

#[test]
fn start_page_actions_fit_at_1280_and_in_a_narrow_window() {
    let case = ribbon::case("edit-suite-start");
    let mut app = suite(ribbon::blank());
    let state = &mut app.state_mut().state;
    state.start = Some(soliddraft_app::start_page::StartPage::with_docs(state, Vec::new()));
    for width in [1280.0, 800.0, 640.0] {
        app.set_size(vec2(width, 640.0));
        app.run_steps(8);
        for label in ["New part", "New assembly", "New drawing", "Open…"] {
            assert!(ribbon::window(&app).contains_rect(app.get_by_label(label).rect()), "{label} fits at {width}");
        }
        case.frame(&mut app, &format!("start-{width:.0}"));
    }
}

struct Release(Arc<gated::Gated>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.rules.lock().unwrap().hold = None;
        self.0.changed.notify_all();
    }
}

#[test]
fn proof_lights_follow_the_changed_feature_and_unproved_exports_leave_files_alone() {
    let case = ribbon::case("edit-suite-proof");
    let kernel = Arc::new(gated::Gated::default());
    let mut doc = Document::new("Proof lights", kernel.clone());
    doc.apply(Command::AddExtrude { params: ExtrudeParams::default(), via_assistant: false }).unwrap();
    let mut params = ExtrudeParams {
        profile: ProfileKind::Rectangle { width: 20.0, height: 10.0 },
        depth: 25.0,
        ..Default::default()
    };
    let edited = doc.apply(Command::AddExtrude { params: params.clone(), via_assistant: false }).unwrap().unwrap();
    let mut app = suite(doc);
    let _release = Release(kernel.clone());
    case.run(&mut app, |app| {
        assert!(app.query_all_by_label("Boss-Extrude1 proof: Proved").next().is_some());
        assert!(app.query_all_by_label("Boss-Extrude2 proof: Proved").next().is_some());
        kernel.rules.lock().unwrap().hold = Some(30.0);
        params.depth = 30.0;
        app.state_mut().state.push(AppAction::Command(Command::EditExtrude { id: edited, params: params.clone() }));
        app.run_steps(8);
        assert!(app.query_all_by_label("Boss-Extrude1 proof: Proved").next().is_some());
        assert!(app.query_all_by_label("Boss-Extrude2 proof: Pending").next().is_some());
        let caller = Caller::Mcp { client: "proof-light-test".into() };
        let path = case.path("part.step");
        std::fs::write(&path, b"existing export").unwrap();
        let output = call_checked(
            &mut AppTools { state: &mut app.state_mut().state },
            "export_step",
            json!({"path": path}),
            &caller,
        );
        assert!(output.is_error, "pending geometry cannot export");
        assert_eq!(std::fs::read(&path).unwrap(), b"existing export");
        case.frame(app, "pending");
        kernel.rules.lock().unwrap().hold = None;
        kernel.changed.notify_all();
        ribbon::until(app, "accepted proof", |app| !app.state().state.doc.proof_pending());
        assert!(app.query_all_by_label("Boss-Extrude2 proof: Proved").next().is_some());
        kernel.rules.lock().unwrap().refuse = vec![35.0];
        params.depth = 35.0;
        app.state_mut().state.push(AppAction::Command(Command::EditExtrude { id: edited, params: params.clone() }));
        app.run_steps(8);
        ribbon::until(app, "refused proof", |app| !app.state().state.doc.proof_pending());
        app.run_steps(4);
        assert!(app.query_all_by_label("Boss-Extrude1 proof: Proved").next().is_some());
        assert!(app.query_all_by_label("Boss-Extrude2 proof: Refused").next().is_some());
        let output = call_checked(
            &mut AppTools { state: &mut app.state_mut().state },
            "export_step",
            json!({"path": path}),
            &caller,
        );
        assert!(output.is_error, "refused geometry cannot export");
        assert_eq!(std::fs::read(&path).unwrap(), b"existing export");
        case.frame(app, "refused-on-edited-feature");
        app.state_mut().state.push(AppAction::Undo);
        app.run_steps(8);
        ribbon::until(app, "undo proof", |app| !app.state().state.doc.proof_pending());
        app.run_steps(4);
        assert!(app.query_all_by_label("Boss-Extrude2 proof: Proved").next().is_some());
        let output = call_checked(
            &mut AppTools { state: &mut app.state_mut().state },
            "export_step",
            json!({"path": path}),
            &caller,
        );
        assert!(!output.is_error, "{}", output.content);
        assert_ne!(std::fs::read(&path).unwrap(), b"existing export");
    });
}
