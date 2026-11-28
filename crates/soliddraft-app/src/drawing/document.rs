//! The open drawing: the `soliddraft-drawing` document, its undo stack, its file, and what
//! the interface keeps beside it (the views' transforms and circles for picking, the
//! last update for the banner). Every change goes through [`DrawingDoc::apply`] with a
//! `soliddraft-drawing` [`Command`].

use super::part::PartContext;
use soliddraft_drawing::file::{self, FileMeta};
use soliddraft_drawing::library::{self, StyleFile};
use soliddraft_drawing::measure::display_text;
use soliddraft_drawing::project::{self, CircleInfo, Projected, ViewTransform};
use soliddraft_drawing::{
    AnnotationId, AnnotationState, Command, ContentHash, Dimension, DocumentId, Drawing, UndoStack, View, ViewDisplay,
    ViewId, ViewKind, ViewScale,
};
use soliddraft_kernel_api::Point2;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use web_time::Instant;

/// What the last update from the model did, for the banner.
#[derive(Clone, Debug)]
pub struct UpdateReport {
    pub from_revision: ContentHash,
    pub to_revision: ContentHash,
    /// What changed in the part, in the part's words.
    pub change: String,
    pub views: usize,
    pub changed: Vec<AnnotationId>,
    pub dangling: Vec<AnnotationId>,
    pub dangling_views: Vec<soliddraft_drawing::DanglingView>,
    pub at: Instant,
    pub dismissed: bool,
}

struct PickingCache {
    last_update: Option<UpdateReport>,
    transforms: HashMap<ViewId, ViewTransform>,
    circles: HashMap<ViewId, Vec<CircleInfo>>,
}

/// The open drawing document.
pub struct DrawingDoc {
    /// The drawing's id, kept in its file's manifest.
    pub id: DocumentId,
    pub drawing: Drawing,
    pub path: Option<PathBuf>,
    pub(super) saved_revision: Option<ContentHash>,
    pub dirty: bool,
    /// Counts every change, so autosave can tell edits apart from redraws.
    pub edits: u64,
    stack: UndoStack,
    undo_picking: Vec<PickingCache>,
    redo_picking: Vec<PickingCache>,
    /// Model-to-view transforms of the views, from the last projection. Not saved.
    pub transforms: HashMap<ViewId, ViewTransform>,
    /// Circles seen face-on per view, from the last projection. Not saved.
    pub circles: HashMap<ViewId, Vec<CircleInfo>>,
    pub last_update: Option<UpdateReport>,
    /// The part revision the drawing was last compared with.
    pub seen_digest: ContentHash,
    /// How many commands the part's log held at the last update, so the banner can say
    /// what changed since. Not saved.
    pub log_mark: usize,
    journal: soliddraft_document::journal::Identity,
}

impl DrawingDoc {
    pub fn journal_key(&self) -> u64 {
        soliddraft_document::journal::register(
            &self.journal,
            "drawing",
            || serde_json::json!({"drawing":self.drawing, "id":self.id}),
        );
        self.journal.key()
    }

    fn journal_command(
        &self,
        command: impl FnOnce() -> serde_json::Value,
    ) -> soliddraft_document::journal::CommandGuard {
        self.journal_key();
        soliddraft_document::journal::begin(&self.journal, command)
    }

    /// The canonical drawing command handler shared with MCP and journal replay.
    pub fn run_journal_command(
        &mut self,
        input: serde_json::Value,
        part: Option<&PartContext<'_>>,
    ) -> Result<(), String> {
        match input["op"].as_str().unwrap_or_default() {
            "execute" => self.apply(serde_json::from_value(input["command"].clone()).map_err(|e| e.to_string())?, part),
            "undo" => self.undo().map(|_| ()).ok_or_else(|| "nothing to undo".into()),
            "redo" => self.redo().map(|_| ()).ok_or_else(|| "nothing to redo".into()),
            _ => Err("unknown drawing journal command".into()),
        }
    }

    pub fn new(drawing: Drawing) -> DrawingDoc {
        let seen = drawing.model.pinned.clone();
        DrawingDoc {
            id: DocumentId::new(),
            drawing,
            path: None,
            saved_revision: None,
            dirty: false,
            edits: 0,
            stack: UndoStack::new(),
            undo_picking: Vec::new(),
            redo_picking: Vec::new(),
            transforms: HashMap::new(),
            circles: HashMap::new(),
            last_update: None,
            seen_digest: seen,
            log_mark: 0,
            journal: Default::default(),
        }
    }

    pub fn style(sheet: &soliddraft_drawing::Sheet) -> &'static StyleFile {
        library::template(sheet.template.as_str()).style_file()
    }

    pub fn can_undo(&self) -> bool {
        self.stack.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.stack.can_redo()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.stack.undo_name()
    }

    /// How many steps the undo stack holds.
    pub fn undo_depth(&self) -> usize {
        self.stack.depth()
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.stack.redo_name()
    }

    pub fn undo(&mut self) -> Option<String> {
        let journal = self.journal_command(|| serde_json::json!({"op":"undo"}));
        let result = (|| {
            let name = self.stack.undo(&mut self.drawing)?;
            let before = self.undo_picking.pop().expect("picking history follows drawing history");
            let after = self.swap_picking(before);
            self.redo_picking.push(after);
            self.touch();
            Some(name.to_owned())
        })();
        let _ = journal.finish(result.as_ref().ok_or("nothing to undo"));
        result
    }

    pub fn redo(&mut self) -> Option<String> {
        let journal = self.journal_command(|| serde_json::json!({"op":"redo"}));
        let result = (|| {
            let name = self.stack.redo(&mut self.drawing)?;
            let after = self.redo_picking.pop().expect("picking history follows drawing history");
            let before = self.swap_picking(after);
            self.undo_picking.push(before);
            self.touch();
            Some(name.to_owned())
        })();
        let _ = journal.finish(result.as_ref().ok_or("nothing to redo"));
        result
    }

    /// True when the part changed since the drawing's views were made: the drawing
    /// still shows the revision it is pinned at.
    pub fn is_stale(&self) -> bool {
        self.seen_digest != self.drawing.model.pinned
    }

    pub(super) fn view_name(&self, id: ViewId) -> Option<String> {
        let sheet = self.drawing.sheet_of(id)?;
        Some(project::view_label_in(&self.drawing, sheet, sheet.view(id)?))
    }

    pub(super) fn rebuild_guidance(&self) -> Option<&'static str> {
        self.drawing
            .sheets
            .iter()
            .flat_map(|sheet| &sheet.views)
            .any(View::needs_inclusion_for_redraw)
            .then_some(soliddraft_drawing::export::INCLUDE_FOR_REDRAW)
    }

    pub fn view_is_up_to_date(&self, view: &View) -> bool {
        self.view_update_reason(view).is_none()
    }

    pub(super) fn view_update_reason(&self, view: &View) -> Option<&str> {
        if let Some(failure) = self.active_refusals().find(|f| f.view == view.id) {
            Some(&failure.cause)
        } else if view.needs_inclusion_for_redraw() {
            Some(soliddraft_drawing::export::INCLUDE_FOR_REDRAW)
        } else if !view.geometry.display_only && !view.has_saved_catallax_linework() {
            Some("Rebuild to replace the saved line work with Catallax line work")
        } else if !view.accepts_revision(self.seen_digest.as_str())
            || !view.accepts_revision(self.drawing.model.pinned.as_str())
        {
            Some("Rebuild to match the part revision")
        } else {
            None
        }
    }

    pub(super) fn view_revision(view: &View) -> String {
        let digest = view.geometry.source_digest.as_str();
        let digits = digest.split_once(':').map_or(digest, |(_, digits)| digits);
        if digits.is_empty() { "unknown".into() } else { digits.chars().take(8).collect() }
    }

    /// Views that need a redraw.
    pub fn stale_views(&self) -> Vec<ViewId> {
        self.drawing
            .sheets
            .iter()
            .flat_map(|s| &s.views)
            .filter(|v| !self.view_is_up_to_date(v))
            .map(|v| v.id)
            .collect()
    }

    fn active_refusals(&self) -> impl Iterator<Item = &soliddraft_drawing::DanglingView> {
        self.drawing.refused_views.iter()
    }

    pub fn refused_views(&self) -> impl Iterator<Item = (usize, ViewId, String, &str)> {
        self.active_refusals().filter_map(|failure| {
            let i = project::sheet_of(&self.drawing, failure.view)?;
            let sheet = &self.drawing.sheets[i];
            let view = sheet.view(failure.view)?;
            Some((i, view.id, project::view_label_in(&self.drawing, sheet, view), failure.cause.as_str()))
        })
    }

    pub fn view_refusal_messages(&self) -> impl Iterator<Item = String> {
        self.refused_views().map(|(_, _, name, cause)| format!("{name}: {cause}"))
    }

    pub fn attention_count(&self) -> usize {
        self.drawing.annotations_needing_attention().count()
    }

    pub fn dangling_count(&self) -> usize {
        self.drawing
            .annotations_needing_attention()
            .filter(|a| matches!(a.state, AnnotationState::Dangling { .. }))
            .count()
    }

    /// Applies a command, with the part when the command reads it.
    pub fn apply(&mut self, cmd: Command, part: Option<&PartContext<'_>>) -> Result<(), String> {
        let part_key = part.and_then(|p| p.doc.model().journal_key());
        let journal = self.journal_command(|| serde_json::json!({"op":"execute", "command":cmd, "part":part_key}));
        journal.finish((|| {
            if let Command::SetViewExcluded { view, excluded: true } = &cmd
                && let Some((_, _, name, cause)) = self.refused_views().find(|(_, id, _, _)| id == view)
            {
                return Err(format!("Cannot exclude {name} from automatic update: {cause}"));
            }
            let from = self.drawing.model.pinned.clone();
            let p = part.map(|c| c.part());
            // A remade view also changes its descendants' linework and picking data.
            let remade = match &cmd {
                Command::EditView { view, .. }
                | Command::SetViewCrop { view, .. }
                | Command::AddBrokenOut { view, .. }
                | Command::RemoveBrokenOut { view, .. }
                | Command::SetViewBreaks { view, .. }
                | Command::SetHalfSection { view, .. }
                | Command::ReattachView { view, .. } => Some(*view),
                Command::SetViewDisplay { view, display } => {
                    let before = self.drawing.view(*view).map(|v| v.display);
                    before.filter(|b| b.tangent != display.tangent || b.seams != display.seams).map(|_| *view)
                }
                _ => None,
            };
            let report = self.stack.run(&mut self.drawing, &cmd, p.as_ref()).map_err(|e| e.to_string())?;
            self.undo_picking.push(PickingCache {
                last_update: self.last_update.clone(),
                transforms: self.transforms.clone(),
                circles: self.circles.clone(),
            });
            self.redo_picking.clear();
            self.rescale_picking();
            self.touch();
            if matches!(cmd, Command::DeleteView { .. } | Command::DeleteSheet { .. })
                && let Some(report) = &mut self.last_update
            {
                report.dangling_views.retain(|failure| self.drawing.view(failure.view).is_some());
            }
            if let (Some(id), Some(p)) = (remade, p.as_ref()) {
                let redrawn: Vec<_> = project::dependent_views(&self.drawing, id)
                    .into_iter()
                    .filter(|&child| child == id || self.drawing.view(child).is_some_and(|v| !v.excluded_from_update))
                    .collect();
                if let Some(report) = &mut self.last_update {
                    report.dangling_views.retain(|failure| !redrawn.contains(&failure.view));
                }
                if let Ok(index) = soliddraft_drawing::PartIndex::build(p) {
                    let mut contexts = project::Contexts::new();
                    for child in redrawn {
                        if project::compute_in(&self.drawing, p, &index, child, &mut contexts).is_ok() {
                            self.remember(child, &contexts[&child].projected);
                        }
                    }
                }
            }
            if let (Some(r), Some(ctx)) = (report, part) {
                for (id, projected) in &r.projections {
                    self.remember(*id, projected);
                }
                self.seen_digest = ctx.digest.clone();
                let changed = r.changed.iter().map(|c| c.annotation).collect();
                let dangling = r.dangling.iter().map(|d| d.annotation).collect();
                let log = ctx.doc.command_log();
                let change = log.get(self.log_mark..).map(|l| l.join(", ")).unwrap_or_default();
                self.log_mark = log.len();
                self.last_update = Some(UpdateReport {
                    from_revision: from,
                    to_revision: ctx.digest.clone(),
                    change,
                    views: r.projections.len(),
                    changed,
                    dangling,
                    dangling_views: r.dangling_views,
                    at: Instant::now(),
                    dismissed: false,
                });
            }
            Ok(())
        })())
    }

    fn swap_picking(&mut self, mut cache: PickingCache) -> PickingCache {
        std::mem::swap(&mut self.last_update, &mut cache.last_update);
        std::mem::swap(&mut self.transforms, &mut cache.transforms);
        std::mem::swap(&mut self.circles, &mut cache.circles);
        cache
    }

    fn rescale_picking(&mut self) {
        for (id, xf) in &mut self.transforms {
            let Ok(scale) = project::view_scale(&self.drawing, *id) else { continue };
            let ratio = scale / xf.scale;
            if (ratio - 1.0).abs() < 1e-12 {
                continue;
            }
            *xf = xf.rescaled(scale);
            if let Some(circles) = self.circles.get_mut(id) {
                for c in circles {
                    c.centre = xf.point(c.model_centre);
                    c.radius = c.model_radius * scale;
                }
            }
        }
    }

    /// The last projection of a view, rebuilt from the cached geometry.
    pub fn projected_of(&self, view: ViewId) -> Option<Projected> {
        let v = self.drawing.view(view)?;
        let transform = *self.transforms.get(&view)?;
        Some(Projected {
            geometry: v.geometry.clone(),
            transform,
            circles: self.circles.get(&view).cloned().unwrap_or_default(),
        })
    }

    /// The next free annotation id.
    pub fn next_annotation_id(&self) -> AnnotationId {
        self.drawing.next_annotation_id()
    }

    /// Projects a new view of the part for placing. The view is not added.
    pub fn project_new(
        &self,
        ctx: &PartContext<'_>,
        sheet: usize,
        kind: &ViewKind,
        display: ViewDisplay,
        scale: ViewScale,
    ) -> Result<(View, Projected), String> {
        let s = self.drawing.sheets.get(sheet).ok_or("no such sheet")?;
        let mut view = View {
            id: self.drawing.next_view_id(),
            kind: kind.clone(),
            position: Point2::new(0.0, 0.0),
            alignment: None,
            scale,
            display,
            excluded_from_update: false,
            geometry: Default::default(),
            label: None,
            crop: None,
            broken_out: Vec::new(),
            exploded: false,
            breaks: Vec::new(),
            half: None,
        };
        let mut with_view = s.clone();
        with_view.views.push(view.clone());
        let spec = project::spec_of(&with_view, &view).ok_or("the view has no orientation")?;
        let k = project::effective_scale(&with_view, &view);
        let part = ctx.part();
        let index = soliddraft_drawing::PartIndex::build(&part).map_err(|e| e.to_string())?;
        let p = project::project_part(&part, &index, &spec, k)
            .map_err(|e| format!("{}: {e}", project::view_label_in(&self.drawing, &with_view, &view)))?;
        view.geometry = p.geometry.clone();
        Ok((view, p))
    }

    /// Remembers a projection's transform and circles after its view was placed.
    pub fn remember(&mut self, view: ViewId, p: &Projected) {
        self.transforms.insert(view, p.transform);
        self.circles.insert(view, p.circles.clone());
    }

    /// A dimension's display text in the drawing's style.
    pub fn dimension_text(sheet: &soliddraft_drawing::Sheet, d: &Dimension) -> String {
        display_text(d, &Self::style(sheet).symbols)
    }

    /// Marks a change: unsaved, and one more edit for autosave.
    pub fn touch(&mut self) {
        self.dirty = true;
        self.edits += 1;
    }

    /// The drawing file's bytes, with line work from `kernel`, and the thumbnail unless
    /// `thumbnail` is false.
    pub fn to_bytes(&self, kernel: &str, thumbnail: bool) -> Result<Vec<u8>, String> {
        let meta = FileMeta { id: self.id, ..FileMeta::new(kernel, self.drawing.meta.units) };
        file::to_bytes_with(&self.drawing, &meta, thumbnail).map_err(|e| e.to_string())
    }

    /// Writes the drawing file, keeping backups as the settings say, and remembers
    /// where. The id is kept: saving under another name is a rename, not a copy.
    /// Returns a warning when the backup could not be made.
    pub fn save(&mut self, path: &Path, kernel: &str) -> Result<Option<String>, String> {
        let bytes = self.to_bytes(kernel, true)?;
        let identity = soliddraft_document::identify(std::io::Cursor::new(&bytes)).map_err(|e| e.to_string())?;
        let warning = crate::file::write_document(path, &bytes)?;
        self.path = Some(path.to_path_buf());
        self.saved_revision = Some(identity.revision);
        self.dirty = false;
        Ok(warning)
    }

    /// Keeps historical paths relative to the file just saved, preserving each pin.
    pub(crate) fn rebase_history_paths(
        &mut self,
        previous_path: Option<&Path>,
        previous_model: &soliddraft_drawing::DocRef,
    ) {
        let Some(path) = self.path.as_deref() else { return };
        let current_model = &self.drawing.model;
        self.stack.map_model_paths(|model| {
            if model.id == previous_model.id && model.kind == previous_model.kind && model.path == previous_model.path {
                return current_model.path.clone();
            }
            let target = previous_path.map_or_else(|| PathBuf::from(&model.path), |old| model.stored_path(old));
            let mut model = model.clone();
            model.set_path(path, &target);
            model.path
        });
    }

    /// A drawing from the bytes of a drawing file, such as a recovery copy.
    pub fn from_bytes(bytes: &[u8]) -> Result<DrawingDoc, String> {
        let (drawing, meta) = file::from_bytes(bytes).map_err(|e| e.to_string())?;
        let mut doc = DrawingDoc::new(drawing);
        doc.id = meta.id;
        doc.saved_revision = soliddraft_document::identify(std::io::Cursor::new(bytes)).ok().map(|id| id.revision);
        Ok(doc)
    }

    /// Opens a drawing file. It shows the part revision it is pinned at; whether the
    /// part has moved on is the resolver's answer (see `super::resolve_model`).
    pub fn open(path: &Path) -> Result<DrawingDoc, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut doc = Self::from_bytes(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        doc.path = Some(path.to_path_buf());
        Ok(doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_drawing::fixture::bracket_a3;
    use soliddraft_drawing::{AnnotationKind, Scale};

    #[test]
    fn aligned_views_slide_along_their_guide_and_follow_their_parent() {
        let mut doc = DrawingDoc::new(bracket_a3());
        let (front, top, left) = (ViewId(1), ViewId(2), ViewId(3));
        let p0 = doc.drawing.view(front).unwrap().position;
        // Top is aligned vertically under Front: it may move up and down only.
        doc.apply(Command::MoveView { view: top, to: Point2::new(150.0, 80.0) }, None).unwrap();
        assert_eq!(doc.drawing.view(top).unwrap().position, Point2::new(p0.u, 80.0));
        // Moving Front drags both children.
        doc.apply(Command::MoveView { view: front, to: Point2::new(110.0, 200.0) }, None).unwrap();
        assert_eq!(doc.drawing.view(top).unwrap().position, Point2::new(110.0, 90.0));
        assert_eq!(doc.drawing.view(left).unwrap().position.v, 200.0);
        // Breaking the alignment frees the view.
        doc.apply(Command::BreakAlignment { view: left }, None).unwrap();
        doc.apply(Command::MoveView { view: left, to: Point2::new(300.0, 150.0) }, None).unwrap();
        assert_eq!(doc.drawing.view(left).unwrap().position, Point2::new(300.0, 150.0));
        assert_eq!(doc.undo_label(), Some("Move view"));
        doc.undo();
        assert_eq!(doc.drawing.view(left).unwrap().position.v, 200.0);
        doc.redo();
        assert_eq!(doc.drawing.view(left).unwrap().position.u, 300.0);
    }

    #[test]
    fn freeze_as_note_keeps_the_text_and_drops_the_references() {
        let mut doc = DrawingDoc::new(bracket_a3());
        let dangling = doc
            .drawing
            .annotations_needing_attention()
            .find(|a| matches!(a.state, AnnotationState::Dangling { .. }))
            .unwrap()
            .id;
        doc.apply(Command::FreezeAsNote { annotation: dangling }, None).unwrap();
        let a = doc.drawing.annotation(dangling).unwrap();
        assert!(matches!(&a.body, AnnotationKind::Note { text, .. } if text == "12 (ref)"));
        assert!(a.anchors.is_empty());
        assert_eq!(doc.attention_count(), 1);
    }

    #[test]
    fn sheet_scale_rescales_views_that_follow_it() {
        let mut doc = DrawingDoc::new(bracket_a3());
        let width = |doc: &DrawingDoc, id: u32| {
            let b = doc.drawing.view(ViewId(id)).unwrap().geometry.bbox;
            b[2] - b[0]
        };
        let (w0, i0) = (width(&doc, 1), width(&doc, 4));
        doc.apply(Command::SetSheetScale { sheet: 0, scale: Scale::new(1, 2) }, None).unwrap();
        assert!((width(&doc, 1) - w0 / 2.0).abs() < 1e-3);
        assert!((width(&doc, 4) - i0).abs() < 1e-9, "a custom-scale view keeps its size");
    }
}
