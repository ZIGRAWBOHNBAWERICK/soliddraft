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
use soliddraft_app::document::{
    AnswerBound, Command, Document, ExtrudeParams, FeatureStatus, PlaneKind, ProfileKind, Reference, Selection,
};
use soliddraft_app::settings::FrameLayout;
use soliddraft_app::state::{AppAction, Tool};
use soliddraft_assistant::tools::call_checked;
use soliddraft_assistant::{Caller, MockProvider, MockTurn};
use std::sync::Arc;

fn bounded_fixture() -> serde_json::Value {
    json!({
        "rule": "Imported profile maximum deviation",
        "kept_definition": "Kept profile A",
        "measured_bound": 0.008,
        "allowed_bound": 0.01,
        "unit": "mm"
    })
}

#[test]
fn identical_mcp_edits_and_their_history_keep_the_current_proof() {
    let kernel = Arc::new(gated::Gated::default());
    let mut doc = Document::new("Identical edit", kernel);
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { depth: 25.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let geometry = doc.model().geometry_hash();
    let mut state = soliddraft_app::state::AppState::new(doc);
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_ok());
    let caller = Caller::Mcp { client: "identical-edit-regression".into() };
    for (tool, args) in [
        ("edit_extrude", json!({"feature":"Boss-Extrude1", "depth":25, "flip":false})),
        ("undo", json!({})),
        ("redo", json!({})),
    ] {
        let result = call_checked(&mut AppTools { state: &mut state }, tool, args, &caller);
        assert!(!result.is_error, "{tool}: {}", result.content);
        assert_eq!(state.doc.model().geometry_hash(), geometry);
        assert!(!state.doc.proof_pending(), "{tool} stranded the unchanged geometry pending");
        assert!(state.doc.proof_state().current_proved_model(), "{tool}: {:?}", state.doc.proof_state());
        assert!(state.export_part(soliddraft_exchange::Format::Step).is_ok(), "{tool} blocked export");
    }
    state.doc.undo_interactive().unwrap();
    assert!(state.doc.proof_state().current_proved_model());
    state.doc.redo_interactive().unwrap();
    assert!(state.doc.proof_state().current_proved_model());
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_ok());
}

#[test]
fn rename_cannot_upgrade_a_bounded_answer_or_unlock_export() {
    let kernel = Arc::new(gated::Gated::default());
    let mut doc = Document::new("Bounded rename", kernel.clone());
    let id = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams { depth: 25.0, ..Default::default() },
            via_assistant: false,
        })
        .unwrap()
        .unwrap();
    let bound: AnswerBound = serde_json::from_value(bounded_fixture()).unwrap();
    doc.record_answer_within_bound(id, bound.clone()).unwrap();
    let mut state = soliddraft_app::state::AppState::new(doc);
    assert!(state.doc.proof_state().answered_within_bound);
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_err());
    kernel.rules.lock().unwrap().refuse = vec![25.0];
    let caller = Caller::Mcp { client: "bounded-rename-regression".into() };
    let renamed = call_checked(
        &mut AppTools { state: &mut state },
        "rename",
        json!({"feature": "Boss-Extrude1", "name": "Renamed plate"}),
        &caller,
    );
    assert!(!renamed.is_error, "{}", renamed.content);
    let exported = state.export_part(soliddraft_exchange::Format::Step);
    assert!(
        exported.is_err(),
        "rename unlocked STEP export; status={:?}, proof={:?}",
        state.doc.feature(id).unwrap().status,
        state.doc.proof_state()
    );
    assert_eq!(state.doc.feature(id).unwrap().status, FeatureStatus::AnsweredWithinBound(bound));
    state.doc.undo_interactive().unwrap();
    assert!(state.doc.proof_state().answered_within_bound);
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_err());
    state.doc.redo_interactive().unwrap();
    assert!(state.doc.proof_state().answered_within_bound);
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_err());
    state.doc.features[0].status = FeatureStatus::Ok;
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_err(), "the row is not the proof authority");
    state.doc.rebuild().unwrap();
    assert!(matches!(state.doc.feature(id).unwrap().status, FeatureStatus::AnsweredWithinBound(_)));
}

fn settle(doc: &mut Document) {
    let start = std::time::Instant::now();
    while doc.proof_pending() {
        assert!(start.elapsed() < std::time::Duration::from_secs(600), "proof did not settle");
        doc.poll_proof();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn rename_keeps_a_refused_result_and_cannot_enable_export() {
    let kernel = Arc::new(gated::Gated::default());
    let mut doc = Document::new("Refused rename", kernel.clone());
    let id = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams { depth: 25.0, ..Default::default() },
            via_assistant: false,
        })
        .unwrap()
        .unwrap();
    kernel.rules.lock().unwrap().refuse = vec![30.0];
    doc.apply_interactive(Command::EditExtrude { id, params: ExtrudeParams { depth: 30.0, ..Default::default() } })
        .unwrap();
    settle(&mut doc);
    let refused = doc.feature(id).unwrap().status.clone();
    assert!(matches!(refused, FeatureStatus::Error(_)));
    let mut state = soliddraft_app::state::AppState::new(doc);
    kernel.rules.lock().unwrap().refuse.clear();
    let caller = Caller::Mcp { client: "refused-rename-regression".into() };
    let renamed = call_checked(
        &mut AppTools { state: &mut state },
        "rename",
        json!({"feature": "Boss-Extrude1", "name": "Rejected plate"}),
        &caller,
    );
    assert!(!renamed.is_error, "{}", renamed.content);
    assert_eq!(state.doc.feature(id).unwrap().status, refused);
    assert!(state.doc.proof_state().failed);
    assert!(!state.doc.proof_state().current_proved_model());
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_err());
    state.doc.undo_interactive().unwrap();
    state.doc.redo_interactive().unwrap();
    assert_eq!(state.doc.feature(id).unwrap().status, refused);
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_err());
}

#[test]
fn geometry_changes_need_their_own_proof_and_undo_recovers_the_kept_bound() {
    let kernel = Arc::new(gated::Gated::default());
    let mut doc = Document::new("Scoped bound", kernel.clone());
    let id = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams { depth: 25.0, ..Default::default() },
            via_assistant: false,
        })
        .unwrap()
        .unwrap();
    let bound: AnswerBound = serde_json::from_value(bounded_fixture()).unwrap();
    doc.record_answer_within_bound(id, bound.clone()).unwrap();
    let kept = doc.model().geometry_hash();
    let mut state = soliddraft_app::state::AppState::new(doc);
    let release = Release(kernel.clone());
    kernel.rules.lock().unwrap().hold = Some(30.0);
    state
        .doc
        .apply_interactive(Command::EditExtrude { id, params: ExtrudeParams { depth: 30.0, ..Default::default() } })
        .unwrap();
    assert_ne!(state.doc.model().geometry_hash(), kept);
    assert!(state.doc.proof_state().pending);
    assert!(!state.doc.proof_state().answered_within_bound);
    let caller = Caller::Mcp { client: "pending-rename-regression".into() };
    let renamed = call_checked(
        &mut AppTools { state: &mut state },
        "rename",
        json!({"feature": "Boss-Extrude1", "name": "New plate"}),
        &caller,
    );
    assert!(!renamed.is_error, "{}", renamed.content);
    assert!(state.doc.proof_state().pending);
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_err());
    drop(release);
    settle(&mut state.doc);
    assert!(state.doc.proof_state().current_proved_model());
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_ok());
    state.doc.undo_interactive().unwrap();
    state.doc.undo_interactive().unwrap();
    settle(&mut state.doc);
    assert_eq!(state.doc.model().geometry_hash(), kept);
    assert_eq!(state.doc.feature(id).unwrap().status, FeatureStatus::AnsweredWithinBound(bound));
    assert!(state.export_part(soliddraft_exchange::Format::Step).is_err());
}

#[test]
fn a_bounded_answer_is_distinct_from_proof_and_keeps_its_definition_and_limit() {
    let case = ribbon::case("edit-suite-bounded");
    let bound: AnswerBound = serde_json::from_value(bounded_fixture()).unwrap();
    assert_eq!(serde_json::to_value(&bound).unwrap(), bounded_fixture());
    for mode in [soliddraft_app::theme::Mode::Light, soliddraft_app::theme::Mode::Dark] {
        let mut app = suite(ribbon::plate());
        soliddraft_app::theme::set_mode(&app.ctx, mode);
        let id = app.state().state.doc.features[0].id;
        app.state_mut().state.doc.record_answer_within_bound(id, bound.clone()).unwrap();
        app.run_steps(8);
        let state = &app.state().state;
        let proof = state.doc.proof_state();
        assert!(proof.answered_within_bound);
        assert!(!proof.failed && !proof.pending);
        assert!(!proof.current_proved_model());
        assert_eq!(state.edit_status().unwrap().0, soliddraft_app::document::WITHIN_BOUND_LABEL);
        let badge = app.get_by_label("Boss-Extrude1 proof: Answered within a stated bound").rect();
        let chip = app.get_by_label("Boss-Extrude1 in history").rect();
        assert!(chip.contains_rect(badge));
        assert!(ribbon::painted_in(&app, chip).iter().any(|text| text.text.contains("0.008 mm")));
        let palette = soliddraft_app::theme::palette();
        assert_ne!(palette.bounded, palette.ok);
        assert_ne!(palette.bounded, palette.warn);
        assert_ne!(palette.bounded, palette.error);
        let caller = Caller::Mcp { client: "bounded-fixture".into() };
        for (tool, extension) in [("export_step", "step"), ("export_iges", "iges"), ("export_stl", "stl")] {
            let destination = case.path(&format!("bounded.{extension}"));
            std::fs::write(&destination, b"existing export").unwrap();
            let output = call_checked(
                &mut AppTools { state: &mut app.state_mut().state },
                tool,
                json!({"path": destination}),
                &caller,
            );
            assert!(output.is_error && output.content.contains("stated bound"), "{tool}: {}", output.content);
            assert_eq!(std::fs::read(destination).unwrap(), b"existing export");
        }
        app.hover_at(badge.center());
        app.run_steps(90);
        let text = ribbon::painted(&app).into_iter().map(|p| p.text).collect::<Vec<_>>().join("\n");
        for detail in ["0.008 mm", "0.01 mm", "Kept profile A", "Imported profile maximum deviation"] {
            assert!(text.contains(detail), "{detail} is shown in {mode:?}");
        }
        case.frame(&mut app, &format!("{mode:?}-within-bound"));
    }
}

#[test]
fn bounded_fixtures_reject_answers_outside_the_declared_limit_or_without_a_definition() {
    for (key, value) in [
        ("measured_bound", json!(0.011)),
        ("measured_bound", json!(-0.001)),
        ("allowed_bound", json!(-0.01)),
        ("kept_definition", json!("")),
        ("rule", json!("")),
        ("unit", json!("")),
    ] {
        let mut fixture = bounded_fixture();
        fixture[key] = value;
        assert!(serde_json::from_value::<AnswerBound>(fixture).is_err(), "invalid {key} is refused");
    }
    assert!(AnswerBound::new("rule".into(), "definition".into(), f64::NAN, 0.01, "mm".into()).is_err());
    assert!(AnswerBound::new("rule".into(), "definition".into(), 0.001, f64::INFINITY, "mm".into()).is_err());
}

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
        app.set_size(vec2(640.0, 640.0));
        app.run_steps(8);
        assert!(ribbon::window(app).contains_rect(app.get_by_label("AI session").rect()));
        case.frame(app, "640-session");
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
        assert_eq!(app.state().state.edit_status().unwrap().0, "Proof pending");
        let caller = Caller::Mcp { client: "proof-light-test".into() };
        let path = case.path("part.step");
        std::fs::write(&path, b"existing export").unwrap();
        for (tool, extension) in [("export_step", "step"), ("export_iges", "iges"), ("export_stl", "stl")] {
            let destination = case.path(&format!("part.{extension}"));
            std::fs::write(&destination, b"existing export").unwrap();
            let output = call_checked(
                &mut AppTools { state: &mut app.state_mut().state },
                tool,
                json!({"path": destination}),
                &caller,
            );
            assert!(output.is_error && output.content.contains("proof"), "{tool}: {}", output.content);
            assert_eq!(std::fs::read(destination).unwrap(), b"existing export");
        }
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
        for (tool, extension) in [("export_step", "step"), ("export_iges", "iges"), ("export_stl", "stl")] {
            let destination = case.path(&format!("part.{extension}"));
            std::fs::write(&destination, b"existing export").unwrap();
            let output = call_checked(
                &mut AppTools { state: &mut app.state_mut().state },
                tool,
                json!({"path": destination}),
                &caller,
            );
            assert!(
                output.is_error && output.content.contains("Boss-Extrude2") && output.content.contains("did not build"),
                "{tool}: {}",
                output.content
            );
            assert_eq!(std::fs::read(destination).unwrap(), b"existing export");
        }
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
