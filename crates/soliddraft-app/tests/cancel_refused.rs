//! A Cancel the part cannot carry out leaves the page building, not claiming to cancel.

mod gated;
#[path = "support/ui.rs"]
mod ribbon;

use std::sync::Arc;

use soliddraft_app::document::Document;
use soliddraft_app::state::{AppAction, NoteKind};

struct Release(Arc<gated::Gated>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.rules.lock().unwrap().hold = None;
        self.0.changed.notify_all();
    }
}

/// A part whose first proof is still pending has no accepted revision, so Cancel is
/// refused. The browser had already said Cancelling and disabled its button; the refusal
/// must clear that, since the build goes on. The first proof is held, so the window is
/// deterministic.
#[test]
fn a_refused_cancel_of_a_first_proof_stops_saying_cancelling() {
    let _case = ribbon::case("cancel-refused");
    let kernel = Arc::new(gated::Gated::default());
    let release = Release(kernel.clone());
    kernel.rules.lock().unwrap().hold = Some(10.0);
    let mut model = soliddraft_model::Document::new("First proof", kernel);
    model
        .execute_deferred(soliddraft_model::Command::AddFeature {
            kind: soliddraft_model::FeatureKind::Extrude(soliddraft_model::ExtrudeFeature {
                profile: soliddraft_model::ProfileSource::Shape {
                    datum: soliddraft_model::DatumPlane::Top,
                    shape: soliddraft_model::BuiltinShape::Rectangle { width: 6.0, height: 6.0 },
                },
                operation: soliddraft_model::ExtrudeOp::Boss,
                end: soliddraft_model::EndCondition::Blind { depth: 10.0 },
                reverse: false,
                options: Default::default(),
            }),
            name: None,
            position: None,
            via_assistant: false,
        })
        .unwrap();
    let mut app = ribbon::app(Document::from_model(model));
    assert!(app.state().state.doc.proof_pending(), "the first proof is held");

    // The browser's Cancel button sets this and asks for the cancel in the same click.
    app.state_mut().state.cancelling = true;
    app.state_mut().state.push(AppAction::CancelBuild);
    app.run_steps(3);
    let state = &app.state().state;
    assert!(state.doc.proof_pending(), "nothing was cancelled, so the build goes on");
    assert!(!state.cancelling, "a refused Cancel still claims to cancel");
    let note = state.note.as_ref().expect("the refusal is said");
    assert_eq!(note.kind, NoteKind::Error);
    assert!(note.text.contains("no earlier part"), "{}", note.text);
    drop(release);
}
