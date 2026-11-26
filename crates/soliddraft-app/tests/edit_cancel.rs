//! Cancel stops an edit still building. The part returns to its last accepted state, and
//! the build is stopped part way rather than run to its end. The kernel's own stop, on a
//! cut that runs for minutes, is checked in `soliddraft-kernel-catallax`.

mod gated;
#[path = "support/ui.rs"]
mod ribbon;

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_kittest::kittest::Queryable;
use soliddraft_app::document::{Command, Document, ExtrudeParams, FeatureId, FeatureStatus};
use soliddraft_app::state::AppAction;

struct Release(Arc<gated::Gated>);

impl Drop for Release {
    fn drop(&mut self) {
        let mut rules = self.0.rules.lock().unwrap();
        rules.hold = None;
        rules.spin = None;
        self.0.changed.notify_all();
    }
}

/// A 25 mm extrusion, built and accepted, on the gated kernel, and the window on it.
fn extrusion(name: &str) -> (Arc<gated::Gated>, FeatureId, ribbon::App) {
    let kernel = Arc::new(gated::Gated::default());
    let mut doc = Document::new(name, kernel.clone());
    let id = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams { depth: 25.0, ..Default::default() },
            via_assistant: false,
        })
        .unwrap()
        .unwrap();
    (kernel, id, ribbon::app(doc))
}

/// What the window said last and the state of its document, for a failed assertion.
fn said(app: &ribbon::App) -> String {
    let state = &app.state().state;
    let note = state.note.as_ref().map(|note| note.text.as_str());
    format!(
        "note {note:?}, cancelling {}, pending {}, undo depth {}, worker busy {}",
        state.cancelling,
        state.doc.proof_pending(),
        state.doc.undo_depth(),
        state.doc.worker_busy()
    )
}

/// Clicks Cancel as the pointer does, and lets the window answer. The bar that offers it
/// is placed a few frames after it first appears.
fn cancel(app: &mut ribbon::App) {
    app.run_steps(5);
    let button = app.get_by_label("Cancel build").rect();
    ribbon::click_at(app, button.center());
}

/// The depth edit the dimension box sends, which builds in the background.
fn depth(app: &mut ribbon::App, id: FeatureId, depth: f64) {
    let edit = Command::EditExtrude { id, params: ExtrudeParams { depth, ..Default::default() } };
    app.state_mut().state.push(AppAction::Command(edit));
    app.run_steps(3);
}

/// The desktop says nothing for a build under a second. Past that it says the part is
/// building and offers Cancel, which returns the part to its last accepted state. The
/// held build keeps the window open as long as the test needs.
#[test]
fn the_desktop_offers_cancel_once_a_build_has_run_a_second() {
    let case = ribbon::case("edit-cancel-desktop");
    let (kernel, id, mut app) = extrusion("Cancel on the desktop");
    let accepted = app.state().state.doc.model().geometry_hash();
    let release = Release(kernel.clone());
    kernel.rules.lock().unwrap().hold = Some(30.0);
    let asked = Instant::now();
    depth(&mut app, id, 30.0);
    assert!(app.state().state.doc.proof_pending(), "the held edit builds in the background");
    if asked.elapsed() < Duration::from_millis(500) {
        assert!(app.query_by_label("Cancel build").is_none(), "a build under a second offers no Cancel");
    }
    ribbon::until(&mut app, "Cancel for a build that has run a second", |app| {
        app.query_by_label("Cancel build").is_some()
    });
    assert!(asked.elapsed() >= Duration::from_secs(1));
    app.run_steps(5);
    case.frame(&mut app, "building");

    cancel(&mut app);
    app.run_steps(3);
    assert!(!app.state().state.doc.proof_pending(), "the part waits for a build after Cancel: {}", said(&app));
    let state = &app.state().state;
    assert_eq!(state.doc.model().geometry_hash(), accepted, "Cancel kept the edit");
    let note = state.note.as_ref().map(|note| note.text.as_str());
    assert!(note.is_some_and(|text| text.contains("Cancelled")), "{note:?}");
    assert!(app.query_by_label("Cancel build").is_none(), "Cancel is still offered");
    drop(release);
}

/// A build that would run for minutes is cancelled from the window. The part is as it
/// was, the build is stopped part way through the interrupt its worker runs it under, and
/// the next edit builds as usual instead of waiting behind it.
#[test]
fn cancel_stops_a_long_build_part_way_and_the_next_edit_does_not_wait_for_it() {
    let _case = ribbon::case("edit-cancel-stop");
    let (kernel, id, mut app) = extrusion("Stop a build");
    let accepted = app.state().state.doc.model().geometry_hash();
    let undo_depth = app.state().state.doc.undo_depth();
    let release = Release(kernel.clone());
    kernel.rules.lock().unwrap().spin = Some(30.0);
    depth(&mut app, id, 30.0);
    assert!(app.state().state.doc.proof_pending(), "the long edit builds in the background");
    ribbon::until(&mut app, "Cancel for the long build", |app| app.query_by_label("Cancel build").is_some());
    assert!(app.state().state.doc.worker_busy(), "the long build is not running");
    assert!(kernel.rules.lock().unwrap().spun.is_empty(), "the long build ended before Cancel");

    cancel(&mut app);
    let cancelled = Instant::now();
    app.run_steps(3);
    assert!(!app.state().state.doc.proof_pending(), "the part waits for a build after Cancel: {}", said(&app));
    let doc = &app.state().state.doc;
    assert_eq!(doc.model().geometry_hash(), accepted, "Cancel kept the edit");
    assert_eq!(doc.undo_depth(), undo_depth, "the cancelled edit is still on the undo stack");
    assert_eq!(doc.displayed_revision(), doc.revision(), "the part on screen is not the accepted one");

    // The worker stops the build at once, not after its minute.
    while app.state().state.doc.worker_busy() {
        assert!(cancelled.elapsed() < Duration::from_secs(20), "the cancelled build is still running");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(kernel.rules.lock().unwrap().spun, [true], "the cancelled build was not stopped part way");
    drop(release);

    depth(&mut app, id, 20.0);
    ribbon::until(&mut app, "the 20 mm edit", |app| !app.state().state.doc.proof_pending());
    let doc = &app.state().state.doc;
    assert!(doc.features.iter().all(|feature| feature.status == FeatureStatus::Ok));
    assert_ne!(doc.model().geometry_hash(), accepted, "the 20 mm edit changed nothing");
    assert_eq!(doc.undo_depth(), undo_depth + 1);
}
