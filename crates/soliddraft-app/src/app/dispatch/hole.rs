//! Hole Wizard start and inline position-sketch actions.

use super::SolidDraftApp;
use crate::document::{HoleParams, Selection};
use crate::state::{EditorParams, NoteKind, SketchSession};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HoleAction {
    Start,
    /// Opens the live inline position sketch inside an active Hole Wizard editor.
    OpenPositionSketch,
    /// Exits the live inline position sketch and syncs its points back to the Hole Wizard.
    ClosePositionSketch,
    /// Pushes panel-side point/pattern edits into an open inline position sketch session.
    SyncPositionSketch,
}

impl SolidDraftApp {
    pub(super) fn dispatch_hole(&mut self, action: HoleAction) {
        match action {
            HoleAction::Start => {
                let face = match self.state.doc.selection {
                    Some(Selection::Face(f)) => self.state.doc.face_ref(f),
                    _ => None,
                };
                self.open_editor(None, EditorParams::Hole(Box::new(HoleParams { face, ..Default::default() })));
                self.state.doc.selection = None;
            }
            HoleAction::OpenPositionSketch => {
                let Some(editor) = self.state.editor.as_mut() else { return };
                let EditorParams::Hole(h) = &mut editor.params else { return };
                let Some(face_ref) = h.face.clone() else {
                    self.state.note("Pick a planar face first", NoteKind::Info);
                    return;
                };
                let Some(body) = self.state.doc.model().body() else { return };
                let Ok(plane) =
                    self.state.doc.model().plane_of(&soliddraft_model::PlaneRef::Face { face: face_ref }, body)
                else {
                    self.state.note("The hole's start face is not planar", NoteKind::Error);
                    return;
                };
                let sketch = h.ensure_position_sketch().clone();
                let was_orthographic = self.state.viewport.camera.orthographic;
                let mut session = SketchSession::new(
                    crate::hole_wizard_panel::INLINE_POSITION_SKETCH_ID,
                    plane,
                    sketch,
                    was_orthographic,
                );
                session.set_tool(crate::state::Tool::Point);
                self.state.sketch = Some(session);
                self.state.ribbon_tab = crate::state::RibbonTab::Sketch;
            }
            HoleAction::ClosePositionSketch => {
                let Some(session) = self.state.sketch.take() else { return };
                self.state.viewport.camera.orthographic = session.was_orthographic;
                self.state.ribbon_tab = crate::state::RibbonTab::Solid;
                if let Some(editor) = self.state.editor.as_mut()
                    && let EditorParams::Hole(h) = &mut editor.params
                {
                    h.position_sketch = Some(session.sketch);
                    h.sync_points_from_sketch();
                    editor.refresh(&self.state.doc);
                }
            }
            HoleAction::SyncPositionSketch => {
                if let Some(session) = self.state.sketch.as_mut()
                    && session.feature == crate::hole_wizard_panel::INLINE_POSITION_SKETCH_ID
                    && let Some(editor) = self.state.editor.as_ref()
                    && let EditorParams::Hole(h) = &editor.params
                    && let Some(sk) = &h.position_sketch
                {
                    session.sketch = sk.clone();
                }
            }
        }
    }
}
