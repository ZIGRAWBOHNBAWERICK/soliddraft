//! Application actions and editor sessions.

mod body;
mod editor;
mod hole;
mod profile;
mod selection;

pub use body::BodyAction;
pub use editor::EditorAction;
pub use hole::HoleAction;
pub use profile::ProfileAction;

use crate::document::{Command, Document, FeatureKind, Reference, Selection, SketchPlane};
use crate::file;
use crate::panels::{Layout, Side};
use crate::state::{AppAction, EditorParams, FeatureEditor, NoteKind, ProfileFeature, SketchSession};
use crate::theme;
use crate::viewport::dimensions::DimensionTarget;
use crate::viewport::render::{DisplayMode, Picked};

use super::Tab;
use super::presentation::docks::center_dock;
use super::workspace::SolidDraftApp;

impl SolidDraftApp {
    pub(super) fn apply_actions(&mut self, ctx: &egui::Context) {
        for action in self.state.take_actions() {
            // In the browser a command that needs exact geometry the page has not read yet
            // waits, with the actions after it, until the page has read it.
            #[cfg(target_arch = "wasm32")]
            {
                if let Some(reading) = &mut self.reading {
                    reading.held.push(action);
                    continue;
                }
                if self.needs_exact(&action) && self.unread_body().is_some() {
                    self.reading = Some(super::workspace::Reading { held: vec![action], drawn: false });
                    ctx.request_repaint();
                    continue;
                }
            }
            self.apply(ctx, action);
        }
    }

    /// Whether an action reads the part's exact geometry: checks, exports, sections,
    /// measurements, a sketch on a face, and any command built on the page rather than
    /// in the kernel worker.
    #[cfg(target_arch = "wasm32")]
    fn needs_exact(&self, action: &AppAction) -> bool {
        match action {
            AppAction::CheckPart
            | AppAction::Export { .. }
            | AppAction::ManufacturingPackage
            | AppAction::ToggleSection
            | AppAction::MeasureEntity
            | AppAction::Editor(EditorAction::ApplySuggestedFilletRadius(_))
            | AppAction::ToggleManufacturingInformation => true,
            AppAction::StartSketchOn(SketchPlane::Face { .. }) => true,
            AppAction::Command(command) => !self.state.doc.edits_scalar(command),
            _ => false,
        }
    }

    /// Closes the open editor, restoring the display the editor rolled back.
    pub(super) fn close_editor(&mut self) -> Option<FeatureEditor> {
        if let Some(session) =
            self.state.sketch.take_if(|s| s.feature == crate::hole_wizard_panel::INLINE_POSITION_SKETCH_ID)
        {
            self.state.viewport.camera.orthographic = session.was_orthographic;
            self.state.ribbon_tab = crate::state::RibbonTab::Solid;
        }
        let e = self.state.editor.take()?;
        if e.target.is_some() {
            if self.state.sketch.as_ref().is_none_or(|s| s.restore_rollback.is_none()) {
                // The bar goes back where the user left it, a drag in the editor included.
                self.state.doc.end_rollback_view();
            } else if self.state.doc.rollback != e.saved_rollback {
                // Inside an early sketch's view the bar goes back to that view.
                self.state.doc.rollback = e.saved_rollback;
                let _ = self.state.doc.rebuild();
            }
        }
        Some(e)
    }

    /// Opens an editor; an existing feature is shown as the body before it.
    pub(super) fn open_editor(&mut self, target: Option<crate::document::FeatureId>, params: EditorParams) {
        self.close_editor();
        self.state.chat.preview = None;
        let saved = self.state.doc.rollback;
        if let Some(id) = target
            && let Some(i) = self.state.doc.index_of(id)
            && self.state.doc.rollback.is_none_or(|r| r > i)
        {
            // Saving while the editor is open keeps the bar where the user left it.
            self.state.doc.hold_rollback();
            self.state.doc.rollback = Some(i);
            let _ = self.state.doc.rebuild();
        }
        self.state.editor = Some(FeatureEditor::new(target, params, saved));
        self.state.ribbon_tab = crate::state::RibbonTab::Solid;
    }

    /// Puts back the rollback an early sketch's session set when it opened.
    fn restore_rollback(&mut self, session: &SketchSession) {
        if let Some(rollback) = session.restore_rollback {
            if self.state.editor.as_ref().is_none_or(|e| e.target.is_none()) {
                self.state.doc.end_rollback_view();
            } else {
                self.state.doc.rollback = rollback.filter(|r| *r < self.state.doc.features.len());
                let _ = self.state.doc.rebuild();
            }
        }
    }

    /// Opens the form OK closed again, with the values it had and why they were refused.
    /// The reason shows in the form's message area, not the status bar, and is no
    /// diagnostic: the shape is unsupported, nothing went wrong.
    fn refuse(
        &mut self,
        (target, params, then): (Option<crate::document::FeatureId>, EditorParams, Option<ProfileFeature>),
        why: String,
    ) {
        self.open_editor(target, params);
        if let Some(editor) = self.state.editor.as_mut() {
            editor.then_feature = then;
            editor.refuse(why);
        }
    }

    /// Opens the plane picker for a new sketch. `then` starts on the sketch when it closes.
    fn ask_plane(&mut self, then: Option<ProfileFeature>) {
        self.open_editor(None, EditorParams::PickPlane);
        if let Some(editor) = self.state.editor.as_mut() {
            editor.then_feature = then;
        }
        self.state.origin_open = true;
        self.state.ribbon_tab = crate::state::RibbonTab::Sketch;
        self.state.note("Choose an origin plane or click a flat face", NoteKind::Info);
    }

    /// The selected sketch `feature` can be built from. With none, or one another feature
    /// already uses, the feature's panel opens asking for its sketch instead.
    fn sketch_for(&mut self, feature: ProfileFeature) -> Option<crate::document::FeatureId> {
        let selected = self.state.selected_sketch();
        if let Some(id) = selected
            && self.state.doc.consumer_of(id).is_none()
        {
            return Some(id);
        }
        let used = selected.and_then(|id| sketch_in_use(&self.state.doc, id));
        self.open_editor(None, EditorParams::PickSketch(feature));
        if let Some(editor) = self.state.editor.as_mut() {
            editor.error = used;
        }
        self.state.doc.selection = None;
        None
    }

    /// A new extrusion or cut grows along its sketch's normal. Seen straight down that
    /// normal, as the view is left when the sketch closes, its preview is a flat tint
    /// with no depth to judge. The view turns to isometric, where the depth shows, or,
    /// for a sketch whose normal lies near the isometric line of sight, to a view tilted
    /// off its own normal.
    fn show_depth_of(&mut self, sketch: crate::document::FeatureId) {
        use crate::viewport::camera::{Camera, DepthView, StandardView};
        let Some(plane) = self.state.doc.feature(sketch).and_then(|f| f.plane) else { return };
        let v = |p: soliddraft_kernel_api::Vec3| glam::Vec3::new(p.x as f32, p.y as f32, p.z as f32);
        let camera = &mut self.state.viewport.camera;
        match Camera::depth_view(camera.heading(), v(plane.normal), v(plane.x_dir)) {
            Some(DepthView::Isometric) => camera.set_view(StandardView::Isometric),
            Some(DepthView::Oblique(from)) => camera.look_from("Oblique", from),
            None => {}
        }
    }

    pub(super) fn open_sketch(&mut self, id: crate::document::FeatureId) {
        self.close_editor();
        let Some(f) = self.state.doc.feature(id) else { return };
        let (Some(plane), Some(sk)) = (f.plane, f.sketch()) else {
            self.state.note("The sketch plane could not be resolved; repair it first", NoteKind::Error);
            return;
        };
        let was_orthographic = self.state.viewport.camera.orthographic;
        let mut session = SketchSession::new(id, plane, sk.sketch.clone(), was_orthographic);
        // An early sketch opens on the part as it was then, as a feature's editor does:
        // later features are rolled back until the sketch closes.
        if let Some(index) = self.state.doc.index_of(id)
            && index + 1 < self.state.doc.features.len()
            && self.state.doc.rollback.is_none_or(|r| r > index + 1)
        {
            session.restore_rollback = Some(self.state.doc.rollback);
            // Saving while the sketch is open keeps the bar where the user left it.
            self.state.doc.hold_rollback();
            self.state.doc.rollback = Some(index + 1);
            let _ = self.state.doc.rebuild();
        }
        self.state.sketch = Some(session);
        self.state.doc.selection = Some(Selection::Feature(id));
        self.state.ribbon_tab = crate::state::RibbonTab::Sketch;
        self.state.viewport.camera.orthographic = true;
        self.state.viewport.camera.look_along(glam::Vec3::new(
            plane.normal.x as f32,
            plane.normal.y as f32,
            plane.normal.z as f32,
        ));
        self.state.viewport.camera.fit_framed(self.fit_bounds());
    }

    /// What "zoom to fit" frames: the body, the open sketch, or with nothing
    /// built yet a 200 mm workspace around the sketch origin.
    pub(super) fn fit_bounds(&self) -> Option<(glam::Vec3, glam::Vec3)> {
        let mut acc = crate::scan::document_bounds(&self.state.doc);
        if let Some(s) = &self.state.sketch {
            let f = crate::sketcher::Frame {
                plane: s.plane,
                camera: &self.state.viewport.camera,
                rect: self.state.viewport.rect,
            };
            let (lo, hi) = s
                .sketch
                .bounds()
                .unwrap_or((crate::sketch::P2::new(-100.0, -100.0), crate::sketch::P2::new(100.0, 100.0)));
            let corners = [
                f.world(lo),
                f.world(hi),
                f.world(crate::sketch::P2::new(lo.u, hi.v)),
                f.world(crate::sketch::P2::new(hi.u, lo.v)),
            ];
            for c in corners {
                acc = Some(match acc {
                    Some((a, b)) => (a.min(c), b.max(c)),
                    None => (c, c),
                });
            }
        }
        acc
    }

    pub(super) fn apply(&mut self, ctx: &egui::Context, action: AppAction) {
        match action {
            AppAction::Command(cmd) => {
                if let Command::EditSketch { id, sketch } = &cmd
                    && *id == crate::hole_wizard_panel::INLINE_POSITION_SKETCH_ID
                {
                    if let Some(editor) = self.state.editor.as_mut()
                        && let EditorParams::Hole(h) = &mut editor.params
                    {
                        h.position_sketch = Some(sketch.clone());
                        h.sync_points_from_sketch();
                        editor.refresh(&self.state.doc);
                    }
                    return;
                }
                if let Some(drag) = self.state.drag_preview.take() {
                    drag.cancel(&mut self.state.doc);
                }
                // A sketch edit from the sketcher must not close the editor state.
                let is_sketch_edit = matches!(cmd, Command::EditSketch { .. });
                match self.state.doc.apply_interactive(cmd) {
                    Ok(Some(id)) => {
                        self.state.doc.selection = Some(Selection::Feature(id));
                        self.state.note("Feature added", NoteKind::Ok);
                    }
                    Ok(None) => {
                        if !is_sketch_edit {
                            if self.state.doc.proof_pending() {
                                self.state.note("Edit requested", NoteKind::Info);
                            } else {
                                self.state.note("Rebuilt", NoteKind::Ok);
                            }
                        }
                    }
                    Err(e) => self.state.note(e, NoteKind::Error),
                }
                self.sync_sketch_session();
            }
            AppAction::Undo if self.state.drawing.is_active() => {
                self.apply(ctx, AppAction::Drawing(crate::drawing::DrawingAction::Undo));
            }
            AppAction::Redo if self.state.drawing.is_active() => {
                self.apply(ctx, AppAction::Drawing(crate::drawing::DrawingAction::Redo));
            }
            AppAction::Undo if self.state.assembly.is_active() => {
                self.apply(ctx, AppAction::Assembly(crate::assembly::AssemblyAction::Undo));
            }
            AppAction::Redo if self.state.assembly.is_active() => {
                self.apply(ctx, AppAction::Assembly(crate::assembly::AssemblyAction::Redo));
            }
            AppAction::Drawing(a) => crate::drawing::apply(&mut self.state, &mut self.dock, ctx, a),
            AppAction::Assembly(crate::assembly::AssemblyAction::FromPart) => {
                // The assembly references the part's file, so the part is saved first.
                if self.state.doc.dirty || self.state.doc.path.is_none() {
                    self.apply(ctx, AppAction::Save);
                }
                crate::assembly::apply(&mut self.state, &mut self.dock, crate::assembly::AssemblyAction::FromPart);
            }
            AppAction::Assembly(crate::assembly::AssemblyAction::EditPartInContext(path)) => {
                use crate::assembly::{AssemblyAction, ComponentPart};
                let path = path.or_else(|| self.state.assembly.session.as_ref().and_then(crate::assembly::edit_target));
                let Some(path) = path else {
                    return self.state.note("Select a component to edit its part in context", NoteKind::Info);
                };
                self.apply(ctx, AppAction::OpenComponentPart(path.clone()));
                if matches!(crate::assembly::component_part(&self.state, &path), Ok(ComponentPart::Current)) {
                    if let Err(e) = crate::in_context::enter(&mut self.state, &path) {
                        self.state.note(e, NoteKind::Error);
                        self.apply(ctx, AppAction::Assembly(AssemblyAction::Show));
                    }
                } else if cfg!(target_arch = "wasm32") {
                    // The page opens the part's bytes a frame or more later.
                    self.state.assembly.entering = Some(path);
                }
            }
            AppAction::Assembly(a) => crate::assembly::apply(&mut self.state, &mut self.dock, a),
            AppAction::OpenComponentPart(path) => {
                use crate::assembly::{AssemblyAction, ComponentPart};
                match crate::assembly::component_part(&self.state, &path) {
                    Ok(ComponentPart::Current) => self.apply(ctx, AppAction::Assembly(AssemblyAction::ShowPart)),
                    #[cfg(not(target_arch = "wasm32"))]
                    Ok(ComponentPart::File(path)) => {
                        let opened = file::open(&path, self.state.doc.kernel().clone(), Some(self.state.doc.model()));
                        if opened.is_ok() {
                            self.apply(ctx, AppAction::Assembly(AssemblyAction::ShowPart));
                        }
                        self.opened(opened);
                    }
                    #[cfg(target_arch = "wasm32")]
                    Ok(ComponentPart::Upload(name, bytes)) => {
                        self.apply(ctx, AppAction::Assembly(AssemblyAction::ShowPart));
                        self.open_bytes(&name, &bytes);
                    }
                    Err(why) => self.state.note(why, NoteKind::Info),
                }
            }
            AppAction::CancelBuild => {
                if let Some(drag) = self.state.drag_preview.take() {
                    drag.cancel(&mut self.state.doc);
                }
                match self.state.doc.cancel_build() {
                    Ok(()) => self.state.note("Cancelled the change still building", NoteKind::Info),
                    Err(e) => {
                        // Nothing was cancelled, so the build goes on and Cancel stays
                        // offered; the page must not keep saying Cancelling.
                        self.state.cancelling = false;
                        self.state.note(e, NoteKind::Error)
                    }
                }
                self.sync_sketch_session();
            }
            AppAction::Undo => {
                if let Some(s) = self.state.sketch.as_mut()
                    && let Some(drag) = s.trim_drag.as_mut()
                {
                    if drag.extend.is_some() {
                        s.sketch = drag.before.clone();
                    } else if let Some(cut) = drag.checkpoints.pop() {
                        cut.restore(&mut s.sketch);
                    }
                    return;
                }
                if let Some(drag) = self.state.drag_preview.take() {
                    drag.cancel(&mut self.state.doc);
                }
                self.close_editor();
                match self.state.doc.undo_interactive() {
                    Ok(l) => self.state.note(format!("Undid {l}"), NoteKind::Info),
                    Err(e) => self.state.note(e, NoteKind::Error),
                }
                self.sync_sketch_session();
            }
            AppAction::Redo => {
                if let Some(s) = self.state.sketch.as_mut()
                    && s.trim_drag.is_some()
                {
                    s.say("Finish or cancel the trim drag before redoing");
                    return;
                }
                if let Some(drag) = self.state.drag_preview.take() {
                    drag.cancel(&mut self.state.doc);
                }
                self.close_editor();
                match self.state.doc.redo_interactive() {
                    Ok(l) => self.state.note(format!("Redid {l}"), NoteKind::Info),
                    Err(e) => self.state.note(e, NoteKind::Error),
                }
                self.sync_sketch_session();
            }
            AppAction::StartSketch => {
                if self.state.in_sketch() {
                    return;
                }
                let plane = match self.state.doc.selection {
                    Some(Selection::Feature(id))
                        if matches!(
                            self.state.doc.feature(id).map(|f| &f.kind),
                            Some(FeatureKind::ReferencePlane(_))
                        ) =>
                    {
                        Some(SketchPlane::Feature { feature: id, offset: 0.0 })
                    }
                    Some(Selection::Face(f)) => self.state.doc.face_ref(f).map(|face| SketchPlane::Face { face }),
                    Some(Selection::Reference(Reference::Plane(plane))) => {
                        Some(SketchPlane::Reference { plane, offset: 0.0 })
                    }
                    // Editing in context, a face of another component picked in the view.
                    None => match crate::in_context::picked_plane(&mut self.state) {
                        Some(Ok(plane)) => Some(plane),
                        Some(Err(why)) => return self.state.note(why, NoteKind::Error),
                        None => None,
                    },
                    _ => None,
                };
                match plane {
                    Some(plane) => self.apply(ctx, AppAction::StartSketchOn(plane)),
                    // A feature waiting for its sketch keeps waiting through the new picker,
                    // as when a sketch tool is chosen before the plane.
                    None => {
                        let then = self.state.editor.as_ref().and_then(FeatureEditor::feature_after_sketch);
                        self.ask_plane(then);
                    }
                }
            }
            AppAction::NewSketchFor(feature) => {
                if !self.state.in_sketch() {
                    self.ask_plane(Some(feature));
                }
            }
            AppAction::StartSketchOn(plane) => {
                let closed = self.close_editor();
                let tool = closed.as_ref().and_then(|editor| editor.pending_sketch_tool);
                let then = closed.as_ref().and_then(FeatureEditor::feature_after_sketch);
                match self.state.doc.apply(Command::AddSketch { plane }) {
                    Ok(Some(id)) => {
                        self.open_sketch(id);
                        if let Some(session) = &mut self.state.sketch {
                            if let Some(tool) = tool {
                                session.set_tool(tool);
                            }
                            if let Some(feature) = then {
                                session.then_feature = Some(feature);
                                session.say(format!(
                                    "Draw the profile, then exit the sketch to continue the {}",
                                    feature.title()
                                ));
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => self.state.note(e, NoteKind::Error),
                }
            }
            AppAction::EditSketch(id) => {
                if self.state.sketch.as_ref().is_some_and(|s| s.feature == id) {
                    return;
                }
                self.apply(ctx, AppAction::ExitSketch);
                self.open_sketch(id);
            }
            AppAction::ExitSketch => {
                if self
                    .state
                    .sketch
                    .as_ref()
                    .is_some_and(|s| s.feature == crate::hole_wizard_panel::INLINE_POSITION_SKETCH_ID)
                {
                    self.dispatch_hole(HoleAction::ClosePositionSketch);
                    return;
                }
                let Some(session) = self.state.sketch.take() else { return };
                self.restore_rollback(&session);
                self.state.viewport.camera.orthographic = session.was_orthographic;
                self.state.ribbon_tab = crate::state::RibbonTab::Solid;
                let id = session.feature;
                let empty = self.state.doc.feature(id).and_then(|f| f.sketch()).is_some_and(|s| s.sketch.is_empty());
                if empty && self.state.doc.consumer_of(id).is_none() {
                    // An empty new sketch leaves nothing behind.
                    let _ = self.state.doc.apply(Command::DeleteFeature(id));
                    self.state.doc.selection = None;
                    // The feature it was drawn for asks for a sketch again.
                    if let Some(feature) = session.then_feature {
                        self.open_editor(None, EditorParams::PickSketch(feature));
                    }
                    return;
                }
                self.state.doc.selection = Some(Selection::Feature(id));
                if let Some(feature) = session.then_feature {
                    self.apply(ctx, feature.start());
                }
            }
            AppAction::Profile(action) => self.dispatch_profile(action),
            AppAction::Body(action) => self.dispatch_body(action),
            AppAction::Hole(action) => self.dispatch_hole(action),
            AppAction::Editor(action) => self.dispatch_editor(ctx, action),
            AppAction::GiveFeedback => {
                #[cfg(target_arch = "wasm32")]
                crate::web::open_feedback();
            }
            AppAction::DragDimension(step) => self.state.drag_dimension(step),
            AppAction::EditDimension(target) => match target {
                DimensionTarget::Feature(id) => self.dispatch_editor(ctx, EditorAction::EditFeature(id)),
                DimensionTarget::Sketch { sketch, constraint } => {
                    self.apply(ctx, AppAction::EditSketch(sketch));
                    if crate::sketch_values::links(self.state.doc.model(), sketch).contains_key(&constraint) {
                        if let Some(s) = self.state.sketch.as_mut() {
                            s.selection = vec![crate::state::SketchSel::Constraint(constraint)];
                        }
                        crate::sketch_values::open(&mut self.state);
                        return;
                    }
                    // The sketch opens with the value editor on the dimension, as a
                    // double-click on it inside the sketch does.
                    if let Some(s) = self.state.sketch.as_mut().filter(|s| s.feature == sketch)
                        && let Some(value) = s.sketch.constraint(constraint).and_then(|c| c.value())
                    {
                        s.dim_edit = Some(crate::state::DimEdit::new(constraint, value));
                    } else if let Some(s) = self.state.sketch.as_mut().filter(|s| s.feature == sketch)
                        && s.sketch.constraint(constraint).is_some_and(|c| c.is_reference())
                    {
                        s.selection = vec![crate::state::SketchSel::Constraint(constraint)];
                        s.say("Reference dimensions are read-only; change to driving to edit");
                    }
                }
            },
            AppAction::Dimension(action) => self.state.dimension(action),
            AppAction::Select(sel) => self.select(ctx, sel),
            AppAction::MeasureEntity => self.state.measure.open = !self.state.measure.open,
            AppAction::ToggleManufacturingInformation => {
                self.state.pmi.open = !self.state.pmi.open;
                // The panel lives in the details sheet, so opening it shows the sheet.
                self.state.options_details = self.state.pmi.open;
            }
            AppAction::ManufacturingPackage => {
                let name = format!("{}.step", self.state.doc.name);
                let Some(path) = crate::platform::save_path("Drawing and files", "STEP", "step", &name) else {
                    return;
                };
                let written = crate::app_tools::AppTools { state: &mut self.state }
                    .manufacturing_package(&serde_json::json!({ "path": path }));
                match written {
                    Ok(report) => {
                        let text = crate::pmi::package_summary(&report);
                        self.state.note(text, NoteKind::Info);
                    }
                    Err(error) => self.state.note(error, NoteKind::Error),
                }
            }
            AppAction::ToggleSection => self.state.section.toggle(&self.state.doc),
            AppAction::ShowMaterial => {
                self.state.doc.selection = Some(Selection::Body);
                self.state.options_details = true;
            }
            AppAction::ShowMaterialLibrary => {
                self.state.material_dialog = Some(crate::material_dialog::Dialog::new(&self.state.doc, None));
            }
            AppAction::ShowBodyMaterial => {
                let body = self.state.doc.active_body;
                self.state.material_dialog = Some(crate::material_dialog::Dialog::new(&self.state.doc, Some(body)));
            }
            AppAction::ShowPartAppearance => {
                self.state.appearance_editor =
                    Some((self.state.doc.instance(), soliddraft_model::AppearanceTarget::Part));
            }
            AppAction::ShowBodyAppearance => {
                let target = soliddraft_model::AppearanceTarget::Body { body: self.state.doc.active_body };
                self.state.appearance_editor = Some((self.state.doc.instance(), target));
            }
            AppAction::ShowAppearance => {
                if !matches!(self.state.doc.selection, Some(Selection::Face(_))) {
                    self.state.doc.selection = Some(Selection::Body);
                }
                self.state.options_details = true;
            }
            AppAction::MeasureMassProperties => {
                let feature = self.state.doc.selection.and_then(|sel| self.state.doc.selection_feature(sel));
                if let Some(id) = feature {
                    match self.state.mass.request(&self.state.doc, id, ctx) {
                        Ok(()) => self.state.doc.selection = Some(Selection::Feature(id)),
                        Err(message) => self.state.note(message, NoteKind::Error),
                    }
                } else {
                    self.state.note("Select a solid feature with measurable properties", NoteKind::Error);
                }
            }
            AppAction::FinishSpline => {
                let mut actions = Vec::new();
                if let Some(s) = self.state.sketch.as_mut() {
                    crate::sketcher::finish_spline(s, &mut actions);
                }
                for action in actions {
                    self.apply(ctx, action);
                }
            }
            AppAction::SketchTool(tool) => {
                if !self.state.in_sketch() && crate::commands::Context::of(&self.state).part() {
                    self.apply(ctx, AppAction::StartSketch);
                    if let Some(editor) = &mut self.state.editor
                        && matches!(editor.params, EditorParams::PickPlane)
                    {
                        editor.pending_sketch_tool = Some(tool);
                    }
                }
                if let Some(s) = self.state.sketch.as_mut() {
                    s.set_tool(tool);
                }
            }
            AppAction::SketchEscape => {
                if let Some(s) = self.state.sketch.as_mut() {
                    if s.tool != crate::state::Tool::Select {
                        s.set_tool(crate::state::Tool::Select);
                    } else {
                        s.selection.clear();
                    }
                }
            }
            AppAction::SketchRelations(open) => {
                if let Some(s) = self.state.sketch.as_mut() {
                    s.relations_open = open;
                    if open {
                        self.state.ribbon_tab = crate::state::RibbonTab::Sketch;
                    }
                }
            }
            AppAction::SketchShading(show) => {
                if let Some(s) = self.state.sketch.as_mut() {
                    s.show_regions = show;
                }
            }
            AppAction::SketchDimensions(show) => {
                if let Some(s) = self.state.sketch.as_mut() {
                    s.show_dimensions = show;
                    if !show {
                        s.selection.retain(|sel| {
                            !matches!(sel, crate::state::SketchSel::Constraint(id)
                            if s.sketch.constraint(*id).is_some_and(|c| c.is_dimension()))
                        });
                        if matches!(s.drag, Some(crate::state::SketchDrag::Label { .. })) {
                            s.drag = None;
                        }
                        s.dim_edit = None;
                    }
                }
            }
            AppAction::SketchGridSnap(on) => {
                if let Some(s) = self.state.sketch.as_mut() {
                    s.grid_snap = on;
                }
            }
            AppAction::SketchDiagnostics(open) => {
                if let Some(s) = self.state.sketch.as_mut() {
                    if open {
                        s.set_tool(crate::state::Tool::Select);
                        s.selection.clear();
                        s.diagnostics = Some(crate::sketch_diagnostics::Inspection::new(&s.sketch));
                        self.state.ribbon_tab = crate::state::RibbonTab::Sketch;
                    } else {
                        s.diagnostics = None;
                    }
                }
            }
            AppAction::CheckSketchFeature => {
                self.apply(ctx, AppAction::SketchDiagnostics(true));
                if let Some(report) = self.state.sketch.as_mut().and_then(|s| s.diagnostics.as_mut()) {
                    report.usage = crate::sketch_diagnostics::Usage::Boss;
                }
            }
            AppAction::LocateSketchDiagnostic => {
                let bounds = self.state.sketch.as_ref().and_then(|s| {
                    let issue = crate::sketch_diagnostics::selected(s)?;
                    let frame = crate::sketcher::Frame {
                        plane: s.plane,
                        camera: &self.state.viewport.camera,
                        rect: self.state.viewport.rect,
                    };
                    let owners = issue
                        .relation
                        .into_iter()
                        .flat_map(|_| issue.entities.iter())
                        .filter_map(|id| s.sketch.entity(*id))
                        .flat_map(|e| e.polyline(8));
                    let mut points =
                        issue.locations.iter().copied().chain(owners).map(|p| frame.world(p)).filter(|p| p.is_finite());
                    let first = points.next()?;
                    Some(points.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
                });
                if let Some((lo, hi)) =
                    bounds.filter(|(lo, hi)| (*lo + *hi).is_finite() && (*hi - *lo).length().is_finite())
                {
                    self.state.viewport.camera.fit_framed(Some((lo, hi)));
                } else if let Some(s) = self.state.sketch.as_mut() {
                    s.say("Fault coordinates cannot be displayed at this scale");
                }
            }
            AppAction::ToggleConstruction => {
                if let Some(s) = self.state.sketch.as_mut() {
                    let ids = s.selected_entities();
                    if ids.is_empty() {
                        s.say("Select entities to toggle construction");
                        return;
                    }
                    let all = ids.iter().all(|e| s.sketch.entity(*e).is_some_and(|e| e.construction));
                    for id in ids {
                        if let Some(e) = s.sketch.entity_mut(id) {
                            e.construction = !all;
                        }
                    }
                    crate::sketch::solve::solve(&mut s.sketch, None, true);
                    let cmd = Command::EditSketch { id: s.feature, sketch: s.sketch.clone() };
                    self.apply(ctx, AppAction::Command(cmd));
                }
            }
            AppAction::DeleteSelection => {
                if let Some(s) = self.state.sketch.as_mut() {
                    if s.tool.spline_mode().is_some() && !s.pending.is_empty() {
                        s.pending.pop();
                        s.snaps.clear();
                        return;
                    }
                    if s.selection.is_empty() {
                        return;
                    }
                    for sel in s.selection.clone() {
                        match sel {
                            crate::state::SketchSel::Entity(e) => s.sketch.remove_entity(e),
                            crate::state::SketchSel::Constraint(c) => s.sketch.remove_constraint(c),
                            crate::state::SketchSel::Point(_) => {}
                        }
                    }
                    s.selection.clear();
                    crate::sketch::solve::solve(&mut s.sketch, None, true);
                    let cmd = Command::EditSketch { id: s.feature, sketch: s.sketch.clone() };
                    self.apply(ctx, AppAction::Command(cmd));
                } else if self.state.editor.is_none()
                    && let Some(id) = self.state.selected_feature()
                {
                    self.apply(ctx, AppAction::Command(Command::DeleteFeature(id)));
                }
            }
            AppAction::RenameSelected => {
                if !self.state.in_sketch()
                    && self.state.doc.selection == Some(Selection::Body)
                    && self.state.doc.display_body().is_some()
                {
                    crate::tree::rename_body(&mut self.state);
                } else if let Some(id) = self.state.selected_feature()
                    && let Some(f) = self.state.doc.feature(id)
                {
                    self.state.rename = Some(crate::state::Rename { feature: id, text: f.name.clone(), focus: true });
                    // The name is typed in its tree row, so the tree comes forward.
                    if !self.state.pane_shown(crate::panels::Pane::Tree) {
                        self.state.show_pane(crate::panels::Pane::Tree);
                    }
                }
            }
            AppAction::SetView(v) => self.state.active_viewport_mut().camera.set_view(v),
            AppAction::NormalTo => {
                let normal = match (&self.state.sketch, self.state.doc.selection) {
                    (Some(s), _) => Some(s.plane.normal),
                    (None, Some(Selection::Face(f))) => self.state.doc.face_plane(f).map(|p| p.normal),
                    (None, _) => {
                        self.state.selected_sketch().and_then(|id| self.state.doc.feature(id)?.plane).map(|p| p.normal)
                    }
                };
                match normal {
                    Some(n) => {
                        self.state.viewport.camera.look_along(glam::Vec3::new(n.x as f32, n.y as f32, n.z as f32))
                    }
                    None => self.state.note("Select a planar face or a sketch first", NoteKind::Info),
                }
            }
            AppAction::ZoomFit => {
                let bounds = self.fit_bounds();
                self.state.viewport.camera.fit_framed(bounds);
            }
            AppAction::SetDisplay(d) => {
                if let Err(error) = self.state.set_display_mode(d) {
                    self.state.note(error, NoteKind::Error);
                }
            }
            AppAction::SetCurrentView(view) => self.state.set_current_view(view),
            AppAction::SetPreset(p) => {
                let viewport = self.state.active_viewport_mut();
                // Realistic leaves edges off; switching to or from it moves the shaded
                // modes along, and a wireframe stays a wireframe.
                if p.shows_edges() != viewport.preset.shows_edges() && viewport.display != DisplayMode::Wireframe {
                    viewport.display = if p.shows_edges() { DisplayMode::ShadedWithEdges } else { DisplayMode::Shaded };
                }
                viewport.preset = p;
            }
            AppAction::SetBodyVisible(show) => {
                // A display state keeps its own hidden bodies; without one the body's own view
                // choice decides. Show clears both: a body the base view hides stays hidden
                // under every display state until its own choice shows it again.
                if let Some(mut display) = self.state.doc.display_state().cloned() {
                    let key = self.state.doc.active_body;
                    let changed = if show { display.view.hidden.remove(&key) } else { display.view.hidden.insert(key) };
                    if changed {
                        let mut states = self.state.doc.display_states().clone();
                        let result = states.put(display).and_then(|_| self.state.doc.change_display_states(states));
                        if let Err(error) = result {
                            self.state.note(error, NoteKind::Error);
                        }
                    }
                    let doc = &mut self.state.doc;
                    if show && !crate::tree_visibility::body_shown(doc) {
                        doc.set_view_choice(crate::document::ViewItem::Body(doc.active_body), Some(true));
                    }
                } else {
                    let doc = &mut self.state.doc;
                    if crate::tree_visibility::body_shown(doc) != show {
                        doc.set_view_choice(crate::document::ViewItem::Body(doc.active_body), Some(show));
                    }
                }
            }
            AppAction::ShowAllBodies => {
                // The active display state's hidden bodies show again too.
                if let Some(mut display) = self.state.doc.display_state().cloned()
                    && !display.view.hidden.is_empty()
                {
                    display.view.hidden.clear();
                    let mut states = self.state.doc.display_states().clone();
                    let result = states.put(display).and_then(|_| self.state.doc.change_display_states(states));
                    if let Err(error) = result {
                        self.state.note(error, NoteKind::Error);
                    }
                }
                let doc = &mut self.state.doc;
                for (key, _) in doc.display_bodies() {
                    if !crate::tree_visibility::body_shown_at(doc, key) {
                        doc.set_view_choice(crate::document::ViewItem::Body(key), Some(true));
                    }
                }
            }
            AppAction::SetReferenceDisplay(display) => {
                let _ = self.state.set_reference_display(display);
            }
            AppAction::SketchRelationSet(kinds) => {
                if let Some(sketch) = self.state.sketch.as_mut() {
                    let action = crate::sketcher::add_relations(sketch, &kinds);
                    self.apply(ctx, action);
                }
            }
            AppAction::SetScene(scene) => {
                self.state.viewport.scene = Some(scene);
                if let Some(session) = self.state.assembly.session.as_mut() {
                    session.viewport.scene = Some(scene);
                }
                let saved = self.state.change_saved_appearance(ctx, Some(scene), |appearance| {
                    appearance.background = theme::Background::Scene;
                });
                if let Err(e) = saved {
                    self.state.note(format!("Scene changed, but could not be saved: {e}"), NoteKind::Error);
                }
            }
            AppAction::SetTheme(mode) => {
                let saved = self.state.change_saved_appearance(ctx, None, |appearance| {
                    appearance.selected = theme::Selection::BuiltIn(theme::BuiltIn::for_mode(mode));
                });
                if let Err(e) = saved {
                    self.state.note(format!("Appearance not saved: {e}"), NoteKind::Error);
                }
            }
            AppAction::ToggleProjection => {
                let camera = &mut self.state.active_viewport_mut().camera;
                camera.orthographic = !camera.orthographic;
            }
            AppAction::PreviewProposal(id) => {
                self.close_editor();
                self.state.preview_proposal(&id);
            }
            AppAction::ApplyProposal(id) => {
                self.close_editor();
                self.state.apply_proposal(&id);
            }
            AppAction::UndoProposal(id) => self.state.undo_proposal(&id),
            AppAction::DiscardProposal(id) => self.state.discard_proposal(&id),
            AppAction::FocusChat => self.state.command_line.open_session(),
            AppAction::ToggleSession => self.state.command_line.toggle_session(),
            AppAction::SendChat => self.state.send_chat(),
            AppAction::StopChat => self.state.stop_chat(),
            AppAction::Ask(text) => {
                self.state.command_line.open_session();
                self.state.chat.draft = text;
                self.state.send_chat();
            }
            AppAction::ToggleAutoApply => self.state.toggle_auto_apply(),
            AppAction::Values(action) => crate::values_tab::apply(&mut self.state, action),
            AppAction::SketchValues(crate::sketch_values::Action::Open) => crate::sketch_values::open(&mut self.state),
            AppAction::SketchValues(crate::sketch_values::Action::Cancel) => {
                crate::sketch_values::cancel(&mut self.state)
            }
            AppAction::SketchValues(crate::sketch_values::Action::Apply { unlink }) => {
                if crate::sketch_values::apply(&mut self.state, unlink) {
                    self.sync_sketch_session();
                }
            }
            // The web build's provider, model and key are set on its server.
            #[cfg(target_arch = "wasm32")]
            AppAction::ShowProviderSettings => {
                self.state.note("The assistant's provider and key are set on the SolidDraft server", NoteKind::Info)
            }
            #[cfg(not(target_arch = "wasm32"))]
            AppAction::ShowProviderSettings => {
                self.state.provider_dialog = Some(crate::chat::ProviderDialog::new(&crate::settings::assistant()))
            }
            AppAction::NewPart => {
                self.state.start = None;
                self.state.drawing_after_open = false;
                self.close_editor();
                self.state.sketch = None;
                self.state.chat.preview = None;
                self.state.import_dialog = None;
                self.state.scan = Default::default();
                self.state.doc = crate::startup::new_document();
                self.state.viewport.camera.fit_framed(None);
                self.state.note("New part", NoteKind::Info);
            }
            AppAction::ImportStep(path) => {
                // In the browser the file arrives as an upload, frames after the request.
                self.state.step_upload = path.is_none() && cfg!(target_arch = "wasm32");
                let path = path.or_else(|| crate::platform::open_path("Import STEP", "STEP", &["step", "stp"]));
                if let Some(path) = path {
                    match crate::import::Preview::read(&self.state.doc, &path) {
                        Ok(preview) => self.state.import_dialog = Some(preview),
                        Err(message) => crate::import::report::refused(
                            &mut self.state,
                            &path,
                            soliddraft_kernel_api::ImportFormat::Step,
                            message,
                        ),
                    }
                }
            }
            AppAction::ImportIges(path) => {
                if self.state.in_sketch() || self.state.doc.proof_pending() {
                    return;
                }
                let path = path.or_else(|| crate::platform::open_path("Import IGES", "IGES", &["iges", "igs"]));
                if let Some(path) = path {
                    match crate::import::Preview::read_source(
                        &self.state.doc,
                        &path,
                        soliddraft_kernel_api::ImportFormat::Iges,
                    ) {
                        Ok(preview) => self.state.import_dialog = Some(preview),
                        Err(message) => crate::import::report::refused(
                            &mut self.state,
                            &path,
                            soliddraft_kernel_api::ImportFormat::Iges,
                            message,
                        ),
                    }
                }
            }
            AppAction::ShowImportReport => self.state.import_report = true,
            AppAction::ShowScan => {
                if crate::scan::part_active(&self.state) {
                    self.state.scan.open = true;
                }
            }
            AppAction::FitScan => crate::scan::start_fit(&mut self.state),
            AppAction::ImportScan(path) => {
                if !crate::scan::can_import(&self.state) {
                    return;
                }
                let path =
                    path.or_else(|| crate::platform::open_path("Import scan reference", "STL or OBJ", &["stl", "obj"]));
                if let Some(path) = path {
                    let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| {
                        crate::scan::review(
                            &mut self.state,
                            path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                            bytes,
                        )
                    });
                    if let Err(e) = result {
                        self.state.note(e, NoteKind::Error);
                    }
                }
            }
            AppAction::ImportMesh(path) => {
                if self.state.in_sketch() || self.state.doc.proof_pending() {
                    return;
                }
                let path =
                    path.or_else(|| crate::platform::open_path("Import mesh solid", "STL or OBJ", &["stl", "obj"]));
                if let Some(path) = path {
                    match crate::import::Preview::read_source(
                        &self.state.doc,
                        &path,
                        soliddraft_kernel_api::ImportFormat::Mesh,
                    ) {
                        Ok(preview) => self.state.import_dialog = Some(preview),
                        Err(message) => crate::import::report::refused(
                            &mut self.state,
                            &path,
                            soliddraft_kernel_api::ImportFormat::Mesh,
                            message,
                        ),
                    }
                }
            }
            AppAction::ReplaceImport { feature, path } => {
                if self.state.in_sketch() || self.state.doc.proof_pending() {
                    return;
                }
                let Some(FeatureKind::ImportStep(import)) = self.state.doc.feature(feature).map(|f| &f.kind) else {
                    return;
                };
                let format = import.format;
                let extensions: &[&str] = match format {
                    soliddraft_kernel_api::ImportFormat::Step => &["step", "stp"],
                    soliddraft_kernel_api::ImportFormat::Iges => &["iges", "igs"],
                    soliddraft_kernel_api::ImportFormat::Mesh => &["stl", "obj"],
                };
                let path =
                    path.or_else(|| crate::platform::open_path("Replace imported file", format.label(), extensions));
                if let Some(path) = path {
                    match crate::import::Preview::read_source(&self.state.doc, &path, format)
                        .and_then(|preview| preview.replacing(&self.state.doc, feature))
                    {
                        Ok(preview) => self.state.import_dialog = Some(preview),
                        Err(message) => self.state.note(message, NoteKind::Error),
                    }
                }
            }
            AppAction::ImportScene(path) => {
                let path = path.or_else(|| {
                    crate::platform::open_path(
                        "Import scene",
                        "OpenUSD, glTF, URDF, OBJ and STL",
                        crate::scene_import::EXTENSIONS,
                    )
                });
                let Some(path) = path else { return };
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
                let default = format!("{stem}.{}", soliddraft_document::kind::ASSEMBLY_EXTENSION);
                let Some(out) = crate::platform::save_path(
                    "Save the imported assembly",
                    "SolidDraft assembly",
                    soliddraft_document::kind::ASSEMBLY_EXTENSION,
                    &default,
                ) else {
                    return;
                };
                match crate::scene_import::import(&path, &out, crate::scene_import::Choices::default()) {
                    Ok(record) => {
                        self.state.note(crate::scene_import::said(&record), NoteKind::Ok);
                        self.apply(ctx, AppAction::Assembly(crate::assembly::AssemblyAction::Open(out)));
                    }
                    Err(message) => self.state.note(message, NoteKind::Error),
                }
            }
            AppAction::Open(path) => {
                let path = path
                    .or_else(|| crate::platform::open_document("Open", "SolidDraft documents", &file::OPEN_EXTENSIONS));
                let Some(path) = path else {
                    // The browser's upload arrives later; on the desktop nothing was chosen.
                    self.state.open_in_new_tab &= cfg!(target_arch = "wasm32");
                    return;
                };
                match soliddraft_document::DocKind::of_path(&path) {
                    Some(soliddraft_document::DocKind::Drawing) => {
                        self.state.open_in_new_tab = false;
                        self.apply(ctx, AppAction::Drawing(crate::drawing::DrawingAction::OpenFile(path)));
                    }
                    Some(soliddraft_document::DocKind::Assembly) => {
                        self.state.open_in_new_tab = false;
                        self.apply(ctx, AppAction::Assembly(crate::assembly::AssemblyAction::Open(path)));
                    }
                    _ => {
                        let opened = file::open(&path, self.state.doc.kernel().clone(), Some(self.state.doc.model()));
                        if opened.is_ok() {
                            self.state.import_dialog = None;
                        }
                        self.opened(opened);
                    }
                }
            }
            AppAction::NewDrawing => {
                if self.state.doc.display_body().is_some() {
                    self.apply(ctx, AppAction::Drawing(crate::drawing::DrawingAction::MakeDrawing));
                    return;
                }
                // A drawing shows a part: ask for one, and start the drawing once it opens.
                self.state.drawing_after_open = true;
                let parts = [file::EXTENSION, file::LEGACY_EXTENSION];
                match crate::platform::open_path("Open the part to draw", "SolidDraft part", &parts) {
                    Some(path) => self.apply(ctx, AppAction::Open(Some(path))),
                    // The browser's upload arrives later; on the desktop nothing was chosen.
                    None => self.state.drawing_after_open = cfg!(target_arch = "wasm32"),
                }
            }
            AppAction::OpenBytes { name, bytes } => self.open_bytes(&name, &bytes),
            AppAction::ShowStartPage => {
                let ctx = ctx.clone();
                self.state.start = Some(crate::start_page::StartPage::new(&self.state, move || ctx.request_repaint()));
            }
            AppAction::Save => match self.state.doc.path.clone() {
                Some(p) => self.apply(ctx, AppAction::SaveAs(Some(p))),
                None => self.apply(ctx, AppAction::SaveAs(None)),
            },
            AppAction::SaveAs(path) => {
                let path = path.or_else(|| {
                    let name = format!("{}.{}", self.state.doc.name, file::EXTENSION);
                    crate::platform::save_path("Save part", "SolidDraft part", file::EXTENSION, &name)
                });
                let Some(path) = path else { return };
                let path = file::with_extension(&path);
                let thumbnail = self.thumbnail();
                let saved = if self.state.assembly.in_context.is_some() {
                    // Editing in context: the part, then the assemblies that show it.
                    crate::in_context::save(&mut self.state, &path, thumbnail.as_deref())
                } else {
                    let extras = file::Extras { thumbnail: thumbnail.as_deref(), ..Default::default() };
                    file::save_with(&mut self.state.doc, &path, &extras).map(|warning| {
                        self.state.remember_revision();
                        self.state.recent = file::recent();
                        match warning {
                            Some(w) => (format!("Saved {} · {w}", path.display()), NoteKind::Error),
                            None => (format!("Saved {}", path.display()), NoteKind::Ok),
                        }
                    })
                };
                match saved {
                    Ok((text, kind)) => {
                        #[cfg(target_arch = "wasm32")]
                        {
                            let id = self.state.doc.model().id();
                            let key = crate::documents_browser::key(id, soliddraft_document::DocKind::Part);
                            if crate::in_context::screen_shows_part_alone(&self.state) {
                                self.picture_for_recent(ctx, key);
                            } else if let Some(png) = &thumbnail {
                                // The screen holds the dimmed assembly too: the part's own
                                // picture goes to the recent list instead.
                                crate::documents_browser::set_thumbnail(&key, png);
                            }
                        }
                        self.state.note(text, kind);
                    }
                    Err(e) => self.state.note(e, NoteKind::Error),
                }
            }
            AppAction::CheckPart => crate::part_check::start(&mut self.state, ctx),
            AppAction::Export { format, path } => {
                let path = path.or_else(|| {
                    let name = format!("{}.{}", self.state.doc.name, format.extension());
                    let title = format!("Export {}", format.name());
                    crate::platform::save_path(&title, format.name(), format.extension(), &name)
                });
                let Some(path) = path else { return };
                if crate::print_check::is_mesh(format) {
                    crate::print_check::start(&mut self.state, format, path, ctx);
                    return;
                }
                let exported = self.state.export_part(format);
                let written = exported.and_then(|out| {
                    let mut files = Vec::new();
                    out.write_with(&path, |file, bytes| {
                        files.push((file.to_owned(), bytes.to_vec()));
                        Ok(())
                    })?;
                    crate::platform::write_export(&path, files).map(|()| out.report)
                });
                match written {
                    Ok(report) => {
                        let kind = match &report {
                            // An approximate part's file says so in its summary; it is not
                            // over a tolerance it never claimed.
                            soliddraft_exchange::Report::Step(r) if !r.within_tolerance && r.approximate.is_none() => {
                                NoteKind::Error
                            }
                            soliddraft_exchange::Report::Mesh(r) if !r.closed => NoteKind::Error,
                            _ => NoteKind::Ok,
                        };
                        self.state.note(format!("Exported {} · {}", path.display(), report.summary()), kind);
                    }
                    Err(e) => self.state.note(format!("Export: {e}"), NoteKind::Error),
                }
            }
            AppAction::ClearRecent => {
                file::forget_all();
                #[cfg(target_arch = "wasm32")]
                crate::documents_browser::forget_recent();
                self.state.recent.clear();
                if let Some(page) = self.state.start.as_mut() {
                    page.docs = Some(Vec::new());
                }
            }
            AppAction::ResetLayout => {
                let drawing_open = self.dock.find_tab(&Tab::Drawing).is_some();
                let assembly_open = self.dock.find_tab(&Tab::Assembly).is_some();
                self.dock = center_dock();
                if drawing_open {
                    self.dock.push_to_focused_leaf(Tab::Drawing);
                }
                if assembly_open {
                    self.dock.push_to_focused_leaf(Tab::Assembly);
                }
                self.state.layout = Layout::default();
                self.state.assembly.tree_hidden = false;
                self.state.drawing.tree_hidden = false;
                self.state.command_line.settings.session_height = crate::command_line::SESSION_DEFAULT_HEIGHT;
                // Forget the sizes egui kept, so the defaults apply again.
                ctx.data_mut(|d| {
                    for side in Side::ALL {
                        d.remove::<egui::PanelState>(side.panel_id());
                    }
                    d.remove::<egui::PanelState>(crate::command_line::session_id());
                });
            }
            AppAction::TogglePanel(_) => {
                let shown = self.state.tree_shown();
                self.state.show_tree(!shown);
            }
            AppAction::ShowFileLocations(kind) => {
                self.state.locations_dialog = Some(crate::locations::LocationsDialog::new(kind));
            }
            AppAction::ShowPreferences => {
                self.preferences.get_or_insert_with(|| {
                    crate::preferences::Preferences::new(
                        &self.state.navigation,
                        self.state.viewport.references,
                        self.state.viewport.grid_style,
                    )
                    .with_appearance(self.state.saved_appearance.clone())
                    .with_shortcuts(&self.state.shortcuts)
                    .with_autosave(self.state.autosave_policy())
                });
            }
            AppAction::ShowShortcuts => self.state.show_shortcuts = true,
            AppAction::TogglePalette => crate::palette::toggle(&mut self.state),
            AppAction::ToggleShortcutMenu => crate::shortcut_menu::toggle(ctx, &mut self.state),
            AppAction::ShowAbout => self.state.show_about = true,
            AppAction::ShowRecovery => {
                if let Some(r) = self.state.recovery.as_mut() {
                    r.show = true;
                    r.asked = true;
                }
                #[cfg(target_arch = "wasm32")]
                crate::journal_browser::show();
            }
            AppAction::RestoreRecovery(file) => {
                self.restore(&file);
                self.close_recovery_when_done();
            }
            AppAction::DiscardRecovery(file) => {
                if let Some(Err(e)) = self.state.recovery.as_mut().map(|r| r.discard(&file)) {
                    self.state.note(e, NoteKind::Error);
                }
                self.close_recovery_when_done();
            }
            AppAction::ExportCrashJournal(source) => {
                if let Some(path) = crate::platform::save_path(
                    "Save crash report journal",
                    "Command journal",
                    "jsonl",
                    "crash-report.jsonl",
                ) {
                    match soliddraft_document::journal::export(&source, &path) {
                        Ok(()) => self.state.note(
                            "Crash report journal saved with personal paths replaced. Review it before sharing.",
                            NoteKind::Ok,
                        ),
                        Err(e) => self.state.note(format!("Cannot save crash report journal: {e}"), NoteKind::Error),
                    }
                }
            }
            AppAction::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// After the document changed under an open sketch (undo, redo, a command),
    /// the session takes the document's copy so both agree.
    pub(super) fn sync_sketch_session(&mut self) {
        let Some(s) = self.state.sketch.as_mut() else { return };
        if s.feature == crate::hole_wizard_panel::INLINE_POSITION_SKETCH_ID {
            return;
        }
        s.cancel_trim();
        if s.corner_first.take().is_some() {
            s.selection.clear();
        }
        match self.state.doc.feature(s.feature).and_then(|f| f.sketch()) {
            Some(sk) => {
                if sk.sketch != s.sketch {
                    s.link_values = None;
                    s.text_edit = None;
                    if !s.trim_bounds.is_empty() {
                        s.trim_bounds.clear();
                        s.selection.clear();
                        s.preview = None;
                    }
                    s.sketch = sk.sketch.clone();
                    s.selection.retain(|sel| match sel {
                        crate::state::SketchSel::Entity(e) => s.sketch.entity(*e).is_some(),
                        crate::state::SketchSel::Point(p) => s.sketch.point(*p).is_some(),
                        crate::state::SketchSel::Constraint(c) => s.sketch.constraint(*c).is_some(),
                    });
                }
                if let Some(plane) = self.state.doc.feature(s.feature).and_then(|f| f.plane) {
                    s.plane = plane;
                }
            }
            None => {
                // The sketch is gone (undone past its creation).
                if let Some(session) = self.state.sketch.take() {
                    self.restore_rollback(&session);
                }
                self.state.ribbon_tab = crate::state::RibbonTab::Solid;
            }
        }
    }
}

/// Maps an id-buffer pick to the document entity it stands for.
pub(super) fn selection_of(doc: &Document, picked: Picked) -> Option<Selection> {
    crate::viewport::pick::selection(doc, picked)
}

/// Why sketch `id` cannot start another feature: the feature that already uses it.
fn sketch_in_use(doc: &Document, id: crate::document::FeatureId) -> Option<String> {
    let user = doc.feature(doc.consumer_of(id)?)?;
    let sketch = doc.feature(id).map_or("The sketch", |f| f.name.as_str());
    Some(format!("{sketch} is already used by {}; pick another sketch or draw a new one", user.name))
}
