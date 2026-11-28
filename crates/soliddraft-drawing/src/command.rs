//! The drawing's undoable commands, as data. The ribbon, the canvas tools, the assistant
//! and the MCP server all change a drawing through these.

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::Point2;

use crate::annotation::{Annotation, AnnotationKind, AnnotationState, Dimension};
use crate::entity::EntityRef;
use crate::ids::SHEET;
use crate::ids::{AnnotationId, TemplateId, ViewId};
use crate::part::{Part, PartIndex};
use crate::sheet::{Projection, RevisionRow, Scale, Table, TableKind, TitleCell};
use crate::update::{self, UpdateReport};
use crate::view::{
    AlignAxis, Alignment, Boundary, Break, BrokenOut, FoldDirection, HalfSection, View, ViewDisplay, ViewKind,
    ViewScale,
};
use crate::{DocRef, Drawing, DrawingError, DrawingMeta, library, measure, project};

/// One undoable change to a drawing.
// Placing a view carries the whole view; commands are run one at a time, so the size of
// the largest variant costs nothing worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Place a new view on a sheet.
    PlaceView {
        /// Index of the sheet.
        sheet: usize,
        /// The view, with its id.
        view: View,
    },
    /// Move a view. Aligned children follow and an aligned view slides only along its
    /// alignment.
    MoveView {
        /// The view.
        view: ViewId,
        /// New position in sheet millimetres.
        to: Point2,
    },
    /// Drop a view's alignment with its parent.
    BreakAlignment {
        /// The view.
        view: ViewId,
    },
    /// Change a view's scale.
    SetViewScale {
        /// The view.
        view: ViewId,
        /// The new scale.
        scale: ViewScale,
    },
    /// Add a dimension. The annotation's body is a dimension.
    AddDimension {
        /// The annotation, with its id.
        annotation: Annotation,
    },
    /// Replace a dimension's settings: kind, value, tolerance, precision and text.
    EditDimension {
        /// The annotation.
        annotation: AnnotationId,
        /// The new settings.
        dimension: Dimension,
    },
    /// Add a note. The annotation's body is a note.
    AddNote {
        /// The annotation, with its id.
        annotation: Annotation,
    },
    /// Delete an annotation.
    DeleteAnnotation {
        /// The annotation.
        annotation: AnnotationId,
    },
    /// Attach one end of an annotation to another entity, the repair for a lost reference.
    Reattach {
        /// The annotation.
        annotation: AnnotationId,
        /// Index into its anchors.
        end: usize,
        /// The new entity.
        entity: EntityRef,
    },
    /// Replace an annotation with a note that keeps its text, the repair that gives up on
    /// its references.
    FreezeAsNote {
        /// The annotation.
        annotation: AnnotationId,
    },
    /// Accept a changed value: its highlight goes and the value becomes the one later
    /// changes compare with.
    AcceptChange {
        /// The annotation.
        annotation: AnnotationId,
    },
    /// Add any annotation: a centre mark, a callout, a symbol. An annotation of view
    /// [`SHEET`](crate::SHEET) belongs to the first sheet.
    AddAnnotation {
        /// The annotation, with its id.
        annotation: Annotation,
    },
    /// Move an annotation's text.
    MoveAnnotation {
        /// The annotation.
        annotation: AnnotationId,
        /// New placement in its view's millimetres.
        to: Point2,
    },
    /// Change the text of a note or hole callout.
    EditNote {
        /// The annotation.
        annotation: AnnotationId,
        /// The new text.
        text: String,
    },
    /// Change how a view draws its lines.
    SetViewDisplay {
        /// The view.
        view: ViewId,
        /// The new display.
        display: ViewDisplay,
    },
    /// Leave a view out of automatic updates, or put it back.
    SetViewExcluded {
        /// The view.
        view: ViewId,
        /// True to leave it out.
        excluded: bool,
    },
    /// Change a sheet's scale. Views that follow it rescale.
    SetSheetScale {
        /// Index of the sheet.
        sheet: usize,
        /// The new scale.
        scale: Scale,
    },
    /// Change a sheet's projection angle.
    SetSheetProjection {
        /// Index of the sheet.
        sheet: usize,
        /// The new angle.
        projection: Projection,
    },
    /// Turn automatic updates on or off.
    SetAutomaticUpdate {
        /// True for on.
        on: bool,
    },
    /// Replace the document properties.
    SetMeta {
        /// The new properties.
        meta: DrawingMeta,
    },
    /// Override one title block field of a sheet; an empty value goes back to the
    /// field's link.
    SetTitleField {
        /// Index of the sheet.
        sheet: usize,
        /// The field's key.
        key: String,
        /// The value.
        value: String,
    },
    /// Change a sheet's format: its paper, zones and title block come from another
    /// template. Its views, annotations, scale and projection angle stay as they are.
    SetSheetFormat {
        /// Index of the sheet.
        sheet: usize,
        /// The template.
        template: TemplateId,
    },
    /// Place a title block field of a sheet elsewhere in the block, or back where its
    /// template puts it.
    SetTitleLayout {
        /// Index of the sheet.
        sheet: usize,
        /// The field's key.
        key: String,
        /// The new place, or `None` for the template's.
        layout: Option<TitleCell>,
    },
    /// Bind a title block field to a part property; an empty property removes the
    /// binding. The value comes from the source the drawing accepted. Binding a field
    /// clears the sheet's own value for it.
    BindTitleProperty {
        /// Index of the sheet.
        sheet: usize,
        /// The field's key.
        key: String,
        /// The part property's name.
        property: String,
    },
    /// Delete a view and its annotations. Views derived from it must go first.
    DeleteView {
        /// The view.
        view: ViewId,
    },
    /// Give a view its own name; an empty name goes back to the default.
    RenameView {
        /// The view.
        view: ViewId,
        /// The name.
        name: String,
    },
    /// Restore a projected view's default alignment with its parent.
    Align {
        /// The view.
        view: ViewId,
    },
    /// Regenerate the views from the current model and re-resolve every annotation.
    UpdateFromModel,
    /// Take an assembly drawing as [`crate::assembly::update_assembly_drawing`] brought it
    /// to its assembly: views, BOM, balloons and pin together.
    UpdateFromAssembly {
        /// The drawing brought up to date.
        drawing: Box<Drawing>,
    },
    /// Add a sheet at the end, made from a template.
    AddSheet {
        /// Its name, such as "Sheet2".
        name: String,
        /// The template's id.
        template: crate::TemplateId,
    },
    /// Delete a sheet with its views, annotations and tables. Views on other sheets
    /// must not be made from its views, and the last sheet stays.
    DeleteSheet {
        /// Index of the sheet.
        sheet: usize,
    },
    /// Rename a sheet.
    RenameSheet {
        /// Index of the sheet.
        sheet: usize,
        /// The new name.
        name: String,
    },
    /// Move a sheet to another place in the order. Views keep their ids, so nothing
    /// else changes; location labels name the new sheet numbers.
    MoveSheet {
        /// Index of the sheet.
        sheet: usize,
        /// Its new index.
        to: usize,
    },
    /// Move a view, with its annotations, to another sheet. Alignments with views left
    /// behind are dropped; views made from it keep pointing at it, and their location
    /// labels say where it went.
    MoveViewToSheet {
        /// The view.
        view: ViewId,
        /// Index of the sheet it goes to.
        sheet: usize,
        /// Its position there, in sheet millimetres.
        to: Point2,
    },
    /// Add an annotation that belongs to a sheet rather than to a view, such as a
    /// revision symbol or a general note. Its view is [`SHEET`](crate::SHEET) and its
    /// points are sheet millimetres.
    AddSheetAnnotation {
        /// Index of the sheet.
        sheet: usize,
        /// The annotation, with its id.
        annotation: Annotation,
    },
    /// Replace what an annotation says, keeping its anchors and placement: a GD&T
    /// frame's tolerance, a surface finish's requirement, a dimension set's precision.
    /// The new body must be of the same kind.
    EditAnnotation {
        /// The annotation.
        annotation: AnnotationId,
        /// The new body.
        body: AnnotationKind,
    },
    /// Add a table to a sheet.
    AddTable {
        /// Index of the sheet.
        sheet: usize,
        /// The table.
        table: Table,
    },
    /// Delete a table.
    DeleteTable {
        /// Index of the sheet.
        sheet: usize,
        /// Index of the table on the sheet.
        table: usize,
    },
    /// Move a table.
    MoveTable {
        /// Index of the sheet.
        sheet: usize,
        /// Index of the table on the sheet.
        table: usize,
        /// The new top-left corner, in sheet millimetres.
        to: Point2,
    },
    /// Add a revision: a row at the end of the sheet's revision table, which is made at
    /// the top right of the drawing area when the sheet has none. The drawing's
    /// revision becomes the row's.
    AddRevision {
        /// Index of the sheet.
        sheet: usize,
        /// The row.
        row: RevisionRow,
    },
    /// Change what a derived view is made from: a section's cutting line or direction,
    /// a detail's boundary or factor, an auxiliary view's side. Remakes its line work.
    EditView {
        /// The view.
        view: ViewId,
        /// The new kind; it must be the same kind of view with the same parent.
        kind: ViewKind,
    },
    /// Crop a view to a boundary in its own millimetres, or remove the crop. Remakes
    /// its line work.
    SetViewCrop {
        /// The view.
        view: ViewId,
        /// The boundary, or `None` for no crop.
        crop: Option<Boundary>,
    },
    /// Add a broken-out section to a view. Remakes its line work.
    AddBrokenOut {
        /// The view.
        view: ViewId,
        /// The region and depth.
        broken_out: BrokenOut,
    },
    /// Remove a broken-out section from a view. Remakes its line work.
    RemoveBrokenOut {
        /// The view.
        view: ViewId,
        /// Index into the view's broken-out sections.
        index: usize,
    },
    /// Give a view its breaks, or none. Remakes its line work.
    SetViewBreaks {
        /// The view.
        view: ViewId,
        /// The breaks, all running the same way.
        breaks: Vec<Break>,
    },
    /// Make a view a half section, or a whole view again. Remakes its line work.
    SetHalfSection {
        /// The view.
        view: ViewId,
        /// The half section, or `None` for none.
        half: Option<HalfSection>,
    },
    /// Point an auxiliary view at another edge, the repair for a lost reference edge.
    ReattachView {
        /// The view.
        view: ViewId,
        /// The new reference edge.
        edge: EntityRef,
    },
}

impl Command {
    /// A short name for undo menus, such as "Move view".
    pub fn name(&self) -> &'static str {
        match self {
            Command::PlaceView { .. } => "Place view",
            Command::MoveView { .. } => "Move view",
            Command::BreakAlignment { .. } => "Break alignment",
            Command::SetViewScale { .. } => "Set view scale",
            Command::AddDimension { .. } => "Add dimension",
            Command::EditDimension { .. } => "Edit dimension",
            Command::AddNote { .. } => "Add note",
            Command::DeleteAnnotation { .. } => "Delete annotation",
            Command::Reattach { .. } => "Reattach",
            Command::FreezeAsNote { .. } => "Keep as note",
            Command::AcceptChange { .. } => "Accept change",
            Command::AddAnnotation { annotation } => match annotation.body {
                AnnotationKind::CentreMark => "Add centre mark",
                AnnotationKind::Centreline => "Add centreline",
                AnnotationKind::Weld { .. } => "Add weld symbol",
                AnnotationKind::HoleCallout { .. } => "Add hole callout",
                AnnotationKind::Note { .. } => "Add note",
                AnnotationKind::Dimension(_) => "Add dimension",
                AnnotationKind::DimensionSet(_) => "Add dimensions",
                AnnotationKind::Gdt { .. } => "Add geometric tolerance",
                AnnotationKind::PositionFrame { .. } => "Add position frame",
                AnnotationKind::DatumTarget { .. } => "Add datum target",
                AnnotationKind::Datum { .. } => "Add datum",
                AnnotationKind::SurfaceFinish { .. } => "Add surface finish",
                AnnotationKind::RevisionSymbol { .. } => "Add revision symbol",
                AnnotationKind::Balloon { .. } => "Add balloon",
            },
            Command::MoveAnnotation { .. } => "Move annotation",
            Command::EditNote { .. } => "Edit note",
            Command::SetViewDisplay { .. } => "Display style",
            Command::SetViewExcluded { .. } => "Exclude from update",
            Command::SetSheetScale { .. } => "Sheet scale",
            Command::SetSheetProjection { .. } => "Projection angle",
            Command::SetAutomaticUpdate { .. } => "Automatic update",
            Command::SetMeta { .. } => "Document properties",
            Command::SetTitleField { .. } => "Title block",
            Command::SetSheetFormat { .. } => "Sheet format",
            Command::SetTitleLayout { .. } => "Title block layout",
            Command::BindTitleProperty { .. } => "Bind title field",
            Command::DeleteView { .. } => "Delete view",
            Command::RenameView { .. } => "Rename view",
            Command::Align { .. } => "Align",
            Command::UpdateFromModel => "Update from model",
            Command::UpdateFromAssembly { .. } => "Update from assembly",
            Command::AddSheet { .. } => "Add sheet",
            Command::DeleteSheet { .. } => "Delete sheet",
            Command::RenameSheet { .. } => "Rename sheet",
            Command::MoveSheet { .. } => "Move sheet",
            Command::MoveViewToSheet { .. } => "Move view to sheet",
            Command::AddSheetAnnotation { .. } => "Add annotation",
            Command::EditAnnotation { .. } => "Edit annotation",
            Command::AddTable { .. } => "Add table",
            Command::DeleteTable { .. } => "Delete table",
            Command::MoveTable { .. } => "Move table",
            Command::AddRevision { .. } => "Add revision",
            Command::EditView { .. } => "Edit view",
            Command::SetViewCrop { .. } => "Crop view",
            Command::AddBrokenOut { .. } => "Broken-out section",
            Command::RemoveBrokenOut { .. } => "Remove broken-out section",
            Command::SetViewBreaks { .. } => "Break view",
            Command::SetHalfSection { .. } => "Half section",
            Command::ReattachView { .. } => "Reattach view",
        }
    }
}

/// Applies a command that needs no model to `drawing`.
///
/// [`Command::UpdateFromModel`] and [`Command::Reattach`] read the part and return
/// [`DrawingError::NeedsModel`] here; run them with [`apply_with`].
pub fn apply(drawing: &mut Drawing, command: &Command) -> Result<(), DrawingError> {
    if command.dimension_needs_model(drawing) {
        return Err(DrawingError::NeedsModel(command.name()));
    }
    match command {
        Command::PlaceView { sheet, view } => {
            if drawing.view(view.id).is_some() {
                return Err(DrawingError::BadRequest(format!("{} is already on the drawing", view.id)));
            }
            if let Some(parent) = view.parent()
                && drawing.view(parent).is_none()
            {
                return Err(DrawingError::UnknownView(parent));
            }
            if let ViewKind::Section { parent, .. } = view.kind
                && drawing
                    .view(parent)
                    .is_some_and(|parent| parent.geometry.source_digest != view.geometry.source_digest)
            {
                return Err(DrawingError::BadRequest(
                    "The section and its parent must come from the same part revision".into(),
                ));
            }
            project::validate_break(&view.geometry, view.display, 1.0, !view.breaks.is_empty())?;
            if view.letter().is_some_and(|l| l == drawing.next_view_label) {
                drawing.next_view_label = next_letter(&drawing.next_view_label);
            }
            let sheet = drawing.sheets.get_mut(*sheet).ok_or(DrawingError::BadRequest(format!("no sheet {sheet}")))?;
            let mut view = view.clone();
            if let ViewKind::Projected { projection, .. } = &mut view.kind {
                projection.get_or_insert(sheet.projection);
            }
            sheet.views.push(view);
            Ok(())
        }
        Command::MoveView { view, to } => move_view(drawing, *view, *to),
        Command::BreakAlignment { view } => {
            drawing.view_mut(*view).ok_or(DrawingError::UnknownView(*view))?.alignment = None;
            Ok(())
        }
        Command::SetViewScale { view, scale } => set_view_scale(drawing, *view, *scale),
        Command::AddDimension { annotation } => {
            if !matches!(annotation.body, AnnotationKind::Dimension(_)) {
                return Err(DrawingError::BadRequest("add dimension needs a dimension".into()));
            }
            add_annotation(drawing, annotation)
        }
        Command::AddNote { annotation } => {
            if !matches!(annotation.body, AnnotationKind::Note { .. }) {
                return Err(DrawingError::BadRequest("add note needs a note".into()));
            }
            add_annotation(drawing, annotation)
        }
        Command::EditDimension { annotation, dimension } => {
            let a = drawing.annotation_mut(*annotation).ok_or(DrawingError::UnknownAnnotation(*annotation))?;
            if !matches!(a.body, AnnotationKind::Dimension(_)) {
                return Err(DrawingError::BadRequest(format!("{annotation} is not a dimension")));
            }
            edit_annotation_body(a, &AnnotationKind::Dimension(dimension.clone()));
            Ok(())
        }
        Command::DeleteAnnotation { annotation } => {
            for sheet in &mut drawing.sheets {
                if let Some(i) = sheet.annotations.iter().position(|a| a.id == *annotation) {
                    sheet.annotations.remove(i);
                    return Ok(());
                }
            }
            Err(DrawingError::UnknownAnnotation(*annotation))
        }
        Command::FreezeAsNote { annotation } => {
            let sheet = drawing.sheet_of_annotation(*annotation).ok_or(DrawingError::UnknownAnnotation(*annotation))?;
            let style = library::template(sheet.template.as_str()).style_file();
            let symbols = style.symbols.clone();
            let a = drawing.annotation_mut(*annotation).ok_or(DrawingError::UnknownAnnotation(*annotation))?;
            let text = match &a.body {
                AnnotationKind::Dimension(d) => measure::display_text(d, &symbols),
                AnnotationKind::Note { text, .. }
                | AnnotationKind::HoleCallout { text }
                | AnnotationKind::SurfaceFinish { text, .. } => text.clone(),
                AnnotationKind::Datum { letter } => letter.clone(),
                AnnotationKind::RevisionSymbol { revision } => revision.clone(),
                AnnotationKind::Balloon { item, .. } => item.to_string(),
                AnnotationKind::Gdt { .. }
                | AnnotationKind::PositionFrame { .. }
                | AnnotationKind::DatumTarget { .. } => crate::display::annotation_name(a, style),
                AnnotationKind::DimensionSet(set) => set.values_text(),
                AnnotationKind::Weld { .. } => "Fillet weld".into(),
                AnnotationKind::CentreMark | AnnotationKind::Centreline => String::new(),
            };
            // The reference mark says the text no longer follows the part.
            a.body = AnnotationKind::Note { text: format!("{text} (ref)"), leader: Vec::new() };
            a.anchors.clear();
            a.source = None;
            a.state = AnnotationState::Ok;
            Ok(())
        }
        Command::AcceptChange { annotation } => {
            let a = drawing.annotation_mut(*annotation).ok_or(DrawingError::UnknownAnnotation(*annotation))?;
            if matches!(a.state, AnnotationState::Changed { .. }) {
                a.state = AnnotationState::Ok;
            }
            Ok(())
        }
        Command::AddAnnotation { annotation } => {
            add_annotation(drawing, annotation)?;
            advance_datum(drawing, annotation);
            Ok(())
        }
        Command::MoveAnnotation { annotation, to } => {
            drawing.annotation_mut(*annotation).ok_or(DrawingError::UnknownAnnotation(*annotation))?.placement = *to;
            Ok(())
        }
        Command::EditNote { annotation, text } => {
            let a = drawing.annotation_mut(*annotation).ok_or(DrawingError::UnknownAnnotation(*annotation))?;
            match &mut a.body {
                AnnotationKind::Note { text: t, .. } | AnnotationKind::HoleCallout { text: t } => {
                    if *t != *text {
                        a.source = None;
                    }
                    *t = text.clone();
                    Ok(())
                }
                _ => Err(DrawingError::BadRequest(format!("{annotation} is not a note"))),
            }
        }
        Command::SetViewDisplay { view, display } => {
            let v = drawing.view_mut(*view).ok_or(DrawingError::UnknownView(*view))?;
            project::validate_break(&v.geometry, *display, 1.0, !v.breaks.is_empty())?;
            v.display = *display;
            Ok(())
        }
        Command::SetViewExcluded { view, excluded } => {
            drawing.view_mut(*view).ok_or(DrawingError::UnknownView(*view))?.excluded_from_update = *excluded;
            Ok(())
        }
        Command::SetSheetScale { sheet, scale } => set_sheet_scale(drawing, *sheet, *scale),
        Command::SetSheetProjection { sheet, projection } => {
            let s = sheet_mut(drawing, *sheet)?;
            for v in &mut s.views {
                if let ViewKind::Projected { projection, .. } = &mut v.kind {
                    projection.get_or_insert(s.projection);
                }
            }
            s.projection = *projection;
            Ok(())
        }
        Command::SetAutomaticUpdate { on } => {
            drawing.settings.automatic_update = *on;
            Ok(())
        }
        Command::SetMeta { meta } => {
            drawing.meta = meta.clone();
            Ok(())
        }
        Command::SetTitleField { sheet, key, value } => {
            let s = sheet_mut(drawing, *sheet)?;
            // Only a text field of the sheet's block, by its exact key, takes a value; a
            // stale value can still be cleared.
            let template = library::template(s.template.as_str());
            let editable = template.title_block.fields.iter().any(|field| field.symbol.is_none() && field.key == *key);
            if !editable && !(value.is_empty() && s.format_overrides.contains_key(key)) {
                return Err(DrawingError::BadRequest(format!("The title block has no text field {key}")));
            }
            if value.is_empty() {
                s.format_overrides.remove(key);
            } else {
                s.format_overrides.insert(key.clone(), value.clone());
            }
            Ok(())
        }
        Command::SetSheetFormat { sheet, template } => {
            let format = library::library()
                .templates
                .iter()
                .find(|t| t.id == template.as_str())
                .ok_or_else(|| DrawingError::BadRequest(format!("there is no template {template}")))?;
            let model = format.template();
            let s = drawing.sheets.get(*sheet).ok_or_else(|| DrawingError::BadRequest(format!("no sheet {sheet}")))?;
            let outside: Vec<_> = s
                .views
                .iter()
                .filter(|view| {
                    let [x0, y0, x1, y1] = project::view_bounds(view);
                    [x0, y0, x1, y1].iter().any(|v| !v.is_finite())
                        || x0 < 0.0
                        || y0 < 0.0
                        || x1 > model.size.width_mm
                        || y1 > model.size.height_mm
                })
                .map(|view| project::view_label_in(drawing, s, view))
                .collect();
            if !outside.is_empty() {
                return Err(DrawingError::BadRequest(format!(
                    "Move or rescale these views before changing the paper size: {}",
                    outside.join(", ")
                )));
            }
            let smaller = model.size.width_mm < s.size.width_mm || model.size.height_mm < s.size.height_mm;
            let mut staged = drawing.clone();
            let s = sheet_mut(&mut staged, *sheet)?;
            s.template = template.clone();
            s.size = model.size;
            s.zones = model.zones;
            // Cells placed for another block's fields go; the new block's own cells stand.
            s.title_layout.retain(|key, cell| format.title_block.fields.iter().any(|f| &f.key == key && cell.fits(f)));
            if smaller {
                let list = crate::display::layout_sheet(&staged, *sheet);
                let mut outside = Vec::new();
                for item in list.sheet_items() {
                    use crate::display::Owner;
                    let bounds = item.bounds();
                    if bounds.x0 >= -1e-6
                        && bounds.y0 >= -1e-6
                        && bounds.x1 <= list.paper.0 + 1e-6
                        && bounds.y1 <= list.paper.1 + 1e-6
                    {
                        continue;
                    }
                    let label = match item.owner() {
                        Owner::Sheet => continue,
                        Owner::View(id) | Owner::Curve(id, _) => {
                            let s = staged.sheet_of(id).expect("drawn view's sheet");
                            project::view_label_in(&staged, s, s.view(id).expect("drawn view"))
                        }
                        Owner::Annotation(id) | Owner::AnnotationEnd(id, _) => format!("annotation {id}"),
                        Owner::Table(i) => match staged.sheets[*sheet].tables[i].kind {
                            crate::TableKind::Revision => "revision table".into(),
                            crate::TableKind::Hole => "hole table".into(),
                            crate::TableKind::Bom => "bill of materials".into(),
                        },
                    };
                    if !outside.contains(&label) {
                        outside.push(label);
                    }
                }
                if !outside.is_empty() {
                    return Err(DrawingError::BadRequest(format!(
                        "Move or rescale these items before changing the paper size: {}",
                        outside.join(", ")
                    )));
                }
            }
            drawing.sheets[*sheet] = staged.sheets.swap_remove(*sheet);
            Ok(())
        }
        Command::SetTitleLayout { sheet, key, layout } => {
            let s = sheet_mut(drawing, *sheet)?;
            let format = library::template(s.template.as_str());
            let field = format
                .title_block
                .fields
                .iter()
                .find(|f| &f.key == key)
                .ok_or_else(|| DrawingError::BadRequest(format!("the title block has no field {key}")))?;
            match layout {
                Some(cell) if !cell.fits(field) => {
                    return Err(DrawingError::BadRequest(format!(
                        "{} must stay inside its template cell with room for its caption and value",
                        field.label
                    )));
                }
                Some(cell) => {
                    s.title_layout.insert(key.clone(), *cell);
                }
                None => {
                    s.title_layout.remove(key);
                }
            }
            Ok(())
        }
        Command::BindTitleProperty { sheet, key, property } => {
            let s = sheet_mut(drawing, *sheet)?;
            if property.is_empty() {
                s.title_bindings.remove(key);
            } else {
                s.title_bindings.insert(key.clone(), property.clone());
            }
            Ok(())
        }
        Command::DeleteView { view } => delete_view(drawing, *view),
        Command::RenameView { view, name } => {
            let v = drawing.view_mut(*view).ok_or(DrawingError::UnknownView(*view))?;
            v.label = if name.trim().is_empty() { None } else { Some(name.clone()) };
            Ok(())
        }
        Command::Align { view } => align(drawing, *view),
        Command::AddSheet { name, template } => {
            let t = crate::Template::by_id(template.as_str())
                .ok_or(DrawingError::BadRequest(format!("no template {template}")))?;
            drawing.sheets.push(crate::Sheet::from_template(name, &t));
            Ok(())
        }
        Command::DeleteSheet { sheet } => delete_sheet(drawing, *sheet),
        Command::RenameSheet { sheet, name } => {
            sheet_mut(drawing, *sheet)?.name = name.clone();
            Ok(())
        }
        Command::MoveSheet { sheet, to } => {
            let n = drawing.sheets.len();
            if *sheet >= n || *to >= n {
                return Err(DrawingError::BadRequest(format!("no sheet {}", sheet.max(to))));
            }
            let s = drawing.sheets.remove(*sheet);
            drawing.sheets.insert(*to, s);
            Ok(())
        }
        Command::MoveViewToSheet { view, sheet, to } => move_view_to_sheet(drawing, *view, *sheet, *to),
        Command::AddSheetAnnotation { sheet, annotation } => {
            if annotation.view != SHEET {
                return Err(DrawingError::BadRequest("a sheet annotation belongs to no view".into()));
            }
            if drawing.annotation(annotation.id).is_some() {
                return Err(DrawingError::BadRequest(format!("{} is already on the drawing", annotation.id)));
            }
            advance_datum(drawing, annotation);
            sheet_mut(drawing, *sheet)?.annotations.push(annotation.clone());
            Ok(())
        }
        Command::EditAnnotation { annotation, body } => {
            let a = drawing.annotation_mut(*annotation).ok_or(DrawingError::UnknownAnnotation(*annotation))?;
            if std::mem::discriminant(&a.body) != std::mem::discriminant(body) {
                return Err(DrawingError::BadRequest(format!("{annotation} is another kind of annotation")));
            }
            edit_annotation_body(a, body);
            Ok(())
        }
        Command::AddTable { sheet, table } => {
            let target = sheet_mut(drawing, *sheet)?;
            if table.kind == TableKind::Hole {
                let view = table
                    .hole_view
                    .ok_or_else(|| DrawingError::BadRequest("A hole table needs a source view".into()))?;
                if target.view(view).is_none() {
                    return Err(DrawingError::UnknownView(view));
                }
                if table.holes.is_empty() || target.tables.iter().any(|existing| existing.hole_view == Some(view)) {
                    return Err(DrawingError::BadRequest(
                        "A view must have visible holes and can have only one hole table".into(),
                    ));
                }
            }
            target.tables.push(table.clone());
            Ok(())
        }
        Command::DeleteTable { sheet, table } => {
            let s = sheet_mut(drawing, *sheet)?;
            if *table >= s.tables.len() {
                return Err(DrawingError::BadRequest(format!("sheet {sheet} has no table {table}")));
            }
            s.tables.remove(*table);
            Ok(())
        }
        Command::MoveTable { sheet, table, to } => {
            let s = sheet_mut(drawing, *sheet)?;
            s.tables
                .get_mut(*table)
                .ok_or(DrawingError::BadRequest(format!("sheet {sheet} has no table {table}")))?
                .position = *to;
            Ok(())
        }
        Command::AddRevision { sheet, row } => {
            let s = sheet_mut(drawing, *sheet)?;
            if s.revision_table().is_none() {
                let position = crate::display::revision_table_home(s);
                s.tables.push(Table::revision(position));
            }
            let t = s.tables.iter_mut().find(|t| t.kind == TableKind::Revision).expect("just made");
            t.revisions.push(row.clone());
            crate::display::keep_above_title_block(s);
            drawing.meta.revision = row.revision.clone();
            Ok(())
        }
        Command::UpdateFromAssembly { drawing: next } => {
            if !crate::assembly::is_assembly_drawing(drawing) || next.model.id != drawing.model.id {
                return Err(DrawingError::BadRequest("only the drawing's own assembly updates it".into()));
            }
            *drawing = (**next).clone();
            Ok(())
        }
        Command::Reattach { .. }
        | Command::UpdateFromModel
        | Command::EditView { .. }
        | Command::SetViewCrop { .. }
        | Command::AddBrokenOut { .. }
        | Command::RemoveBrokenOut { .. }
        | Command::SetViewBreaks { .. }
        | Command::SetHalfSection { .. }
        | Command::ReattachView { .. } => Err(DrawingError::NeedsModel(command.name())),
    }
}

/// The letter after `label`: "A" gives "B", "Z" gives "AA". I, O and Q are skipped, as
/// ASME Y14.3 asks, so they are not read as 1 and 0.
pub fn next_letter(label: &str) -> String {
    let mut chars: Vec<u8> = label.bytes().filter(u8::is_ascii_uppercase).collect();
    if chars.is_empty() {
        return "A".into();
    }
    loop {
        let mut i = chars.len();
        loop {
            if i == 0 {
                chars.insert(0, b'A');
                break;
            }
            i -= 1;
            if chars[i] < b'Z' {
                chars[i] += 1;
                break;
            }
            chars[i] = b'A';
        }
        if !chars.iter().any(|c| matches!(c, b'I' | b'O' | b'Q')) {
            return String::from_utf8(chars).expect("ASCII");
        }
    }
}

/// Moves the next datum letter on when an annotation uses it.
fn advance_datum(drawing: &mut Drawing, annotation: &Annotation) {
    if let AnnotationKind::Datum { letter } = &annotation.body
        && *letter == drawing.next_datum_label
    {
        drawing.next_datum_label = next_letter(letter);
    }
}

/// Deletes a sheet, once no view elsewhere is made from its views.
fn delete_sheet(drawing: &mut Drawing, sheet: usize) -> Result<(), DrawingError> {
    if drawing.sheets.len() <= 1 {
        return Err(DrawingError::BadRequest("a drawing keeps at least one sheet".into()));
    }
    let s = drawing.sheets.get(sheet).ok_or(DrawingError::BadRequest(format!("no sheet {sheet}")))?;
    let ids: Vec<ViewId> = s.views.iter().map(|v| v.id).collect();
    let others = drawing.sheets.iter().enumerate().filter(|(i, _)| *i != sheet).flat_map(|(_, s)| &s.views);
    if let Some(v) = others.into_iter().find(|v| v.parent().is_some_and(|p| ids.contains(&p))) {
        return Err(DrawingError::BadRequest(format!("{} on another sheet is made from a view on this one", v.id)));
    }
    drawing.sheets.remove(sheet);
    drawing.refused_views.retain(|failure| !ids.contains(&failure.view));
    Ok(())
}

/// Moves a view and its annotations to another sheet.
fn move_view_to_sheet(drawing: &mut Drawing, view: ViewId, to_sheet: usize, to: Point2) -> Result<(), DrawingError> {
    let from = project::sheet_of(drawing, view).ok_or(DrawingError::UnknownView(view))?;
    if to_sheet >= drawing.sheets.len() {
        return Err(DrawingError::BadRequest(format!("no sheet {to_sheet}")));
    }
    if from == to_sheet {
        return move_view(drawing, view, to);
    }
    let before = scale_followers(drawing, view)?;
    let moved = drawing.view(view).expect("view on source sheet");
    let ratio = project::scale_in(drawing, to_sheet, moved) / project::scale_in(drawing, from, moved);
    for (id, _) in &before {
        check_break_scale(drawing, *id, ratio)?;
    }
    let s = &mut drawing.sheets[from];
    let i = s.views.iter().position(|v| v.id == view).expect("found");
    let mut v = s.views.remove(i);
    if let ViewKind::Projected { projection, .. } = &mut v.kind {
        projection.get_or_insert(s.projection);
    }
    let annotations: Vec<Annotation> = s.annotations.iter().filter(|a| a.view == view).cloned().collect();
    s.annotations.retain(|a| a.view != view);
    let mut tables = Vec::new();
    s.tables.retain(|table| {
        if table.hole_view == Some(view) {
            tables.push(table.clone());
            false
        } else {
            true
        }
    });
    for other in &mut s.views {
        if other.alignment.is_some_and(|a| a.parent == view) {
            other.alignment = None;
        }
    }
    v.alignment = None;
    v.position = to;
    let target = &mut drawing.sheets[to_sheet];
    target.views.push(v);
    target.annotations.extend(annotations);
    target.tables.extend(tables);
    rescale_changed(drawing, before)
}

impl Command {
    /// Whether the command can read geometry from the supplied part.
    pub fn uses_part(&self, drawing: &Drawing) -> bool {
        matches!(
            self,
            Self::UpdateFromModel
                | Self::Reattach { .. }
                | Self::EditView { .. }
                | Self::SetViewCrop { .. }
                | Self::AddBrokenOut { .. }
                | Self::RemoveBrokenOut { .. }
                | Self::SetViewBreaks { .. }
                | Self::SetHalfSection { .. }
                | Self::ReattachView { .. }
                | Self::SetViewDisplay { .. }
        ) || self.dimension_needs_model(drawing)
    }

    /// Whether the command must measure anchors against the part before applying.
    pub fn dimension_needs_model(&self, drawing: &Drawing) -> bool {
        let annotation = match self {
            Self::AddDimension { annotation } | Self::AddAnnotation { annotation } => {
                let measured = match &annotation.body {
                    AnnotationKind::Dimension(d) => matches!(d.value, crate::DimValue::Measured { .. }),
                    AnnotationKind::DimensionSet(_) => matches!(self, Self::AddAnnotation { .. }),
                    _ => false,
                };
                if !measured {
                    return false;
                }
                annotation
            }
            Self::EditDimension { annotation, dimension }
            | Self::EditAnnotation { annotation, body: AnnotationKind::Dimension(dimension) } => {
                let Some(a) = drawing.annotation(*annotation) else { return false };
                let AnnotationKind::Dimension(before) = &a.body else { return false };
                if !matches!(dimension.value, crate::DimValue::Measured { .. })
                    || (dimension.kind == before.kind
                        && dimension.dim_type == before.dim_type
                        && dimension.value == before.value
                        && dimension.chamfer_angle == before.chamfer_angle)
                {
                    return false;
                }
                a
            }
            Self::EditAnnotation { annotation, body: AnnotationKind::DimensionSet(after) } => {
                let Some(a) = drawing.annotation(*annotation) else { return false };
                let AnnotationKind::DimensionSet(before) = &a.body else { return false };
                if before.kind == after.kind
                    && before.orientation == after.orientation
                    && before.dim_type == after.dim_type
                    && before.values == after.values
                {
                    return false;
                }
                a
            }
            _ => return false,
        };
        !annotation.anchors.is_empty()
            && (annotation.anchors.iter().any(|a| a.entity.section_leg != 0)
                || drawing.view(annotation.view).is_some_and(|view| !view.geometry.breaks.is_empty()))
    }
}

fn edit_annotation_body(annotation: &mut Annotation, body: &AnnotationKind) {
    fn text(dimension: &Dimension) -> &str {
        if dimension.text.is_empty() { Dimension::VALUE_TEXT } else { &dimension.text }
    }
    let changed = match (&annotation.body, body) {
        (AnnotationKind::Dimension(before), AnnotationKind::Dimension(after)) => {
            before.kind != after.kind
                || before.dim_type != after.dim_type
                || before.value != after.value
                || before.tolerance != after.tolerance
                || before.chamfer_angle != after.chamfer_angle
                || before.requirement != after.requirement
                || text(before) != text(after)
        }
        (AnnotationKind::Note { text: before, .. }, AnnotationKind::Note { text: after, .. }) => before != after,
        _ => annotation.body != *body,
    };
    if changed {
        annotation.source = None;
    }
    annotation.body = body.clone();
}

fn measure_dimension(drawing: &Drawing, part: &Part, annotation: &mut Annotation) -> Result<(), DrawingError> {
    let sheet = drawing.sheet_of(annotation.view).ok_or(DrawingError::UnknownView(annotation.view))?;
    let view = sheet.view(annotation.view).expect("the view is on the sheet");
    if view.geometry.source_digest != part.digest.as_str() {
        return Err(DrawingError::BadRequest("Rebuild the view from the open part before dimensioning it".into()));
    }
    let index = PartIndex::build(part)?;
    let context = crate::derived::context(drawing, part, &index, annotation.view)?;
    let symbols = &library::template(sheet.template.as_str()).style_file().symbols;
    update::re_resolve(annotation, &index, &context, symbols, part);
    if let AnnotationState::Dangling { cause, .. } = &annotation.state {
        return Err(DrawingError::BadRequest(cause.clone()));
    }
    annotation.state = AnnotationState::Ok;
    Ok(())
}

/// Applies any command, reading the part for those that need it. Returns the update's
/// report for [`Command::UpdateFromModel`].
pub fn apply_with(drawing: &mut Drawing, command: &Command, part: &Part) -> Result<Option<UpdateReport>, DrawingError> {
    match command {
        Command::AddDimension { annotation } | Command::AddAnnotation { annotation }
            if command.dimension_needs_model(drawing) =>
        {
            let mut measured = annotation.clone();
            measure_dimension(drawing, part, &mut measured)?;
            if let AnnotationKind::Dimension(d) = &mut measured.body
                && let crate::DimValue::Measured { value } = d.value
            {
                d.precision = d.precision.max(crate::measure::precision_for(value));
            } else if let AnnotationKind::DimensionSet(set) = &mut measured.body {
                set.precision =
                    set.values.iter().copied().map(crate::measure::precision_for).fold(set.precision, u8::max);
            }
            add_annotation(drawing, &measured).map(|_| None)
        }
        Command::EditDimension { annotation, .. } | Command::EditAnnotation { annotation, .. }
            if command.dimension_needs_model(drawing) =>
        {
            let mut measured = drawing.annotation(*annotation).expect("dimension to edit").clone();
            match command {
                Command::EditDimension { dimension, .. } => {
                    edit_annotation_body(&mut measured, &AnnotationKind::Dimension(dimension.clone()))
                }
                Command::EditAnnotation { body, .. } => edit_annotation_body(&mut measured, body),
                _ => unreachable!(),
            }
            measure_dimension(drawing, part, &mut measured)?;
            *drawing.annotation_mut(*annotation).expect("dimension to edit") = measured;
            Ok(None)
        }
        Command::UpdateFromModel => update::update_from_model(drawing, part).map(Some),
        Command::Reattach { annotation, end, entity } => {
            update::reattach(drawing, part, *annotation, *end, entity.clone()).map(|_| None)
        }
        Command::ReattachView { view, edge } => update::reattach_view(drawing, part, *view, edge.clone()).map(|_| None),
        Command::EditView { view, kind } => edit_and_regenerate(drawing, part, *view, |v| {
            if std::mem::discriminant(&v.kind) != std::mem::discriminant(kind) || v.parent() != kind_parent(kind) {
                return Err(DrawingError::BadRequest(format!("{view} cannot become another kind of view")));
            }
            v.kind = kind.clone();
            Ok(())
        }),
        Command::SetViewCrop { view, crop } => edit_and_regenerate(drawing, part, *view, |v| {
            v.crop.clone_from(crop);
            Ok(())
        }),
        Command::AddBrokenOut { view, broken_out } => edit_and_regenerate(drawing, part, *view, |v| {
            v.broken_out.push(broken_out.clone());
            Ok(())
        }),
        Command::RemoveBrokenOut { view, index } => edit_and_regenerate(drawing, part, *view, |v| {
            if *index >= v.broken_out.len() {
                return Err(DrawingError::BadRequest(format!("{view} has no broken-out section {index}")));
            }
            v.broken_out.remove(*index);
            Ok(())
        }),
        Command::SetViewBreaks { view, breaks } => {
            update::edit_breaks(drawing, part, *view, breaks.clone()).map(|_| None)
        }
        Command::SetHalfSection { view, half } => edit_and_regenerate(drawing, part, *view, |v| {
            if half.is_some() && !matches!(v.kind, ViewKind::Base { .. } | ViewKind::Projected { .. }) {
                return Err(DrawingError::BadRequest("A half section requires a model or projected view".into()));
            }
            v.half.clone_from(half);
            Ok(())
        }),
        Command::SetViewDisplay { view, display } => {
            let before = drawing.view(*view).ok_or(DrawingError::UnknownView(*view))?.display;
            if before.tangent != display.tangent || before.seams != display.seams {
                edit_and_regenerate(drawing, part, *view, |v| {
                    v.display = *display;
                    Ok(())
                })
            } else {
                apply(drawing, command).map(|_| None)
            }
        }
        other => apply(drawing, other).map(|_| None),
    }
}

fn edit_and_regenerate(
    drawing: &mut Drawing,
    part: &Part,
    id: ViewId,
    edit: impl FnOnce(&mut View) -> Result<(), DrawingError>,
) -> Result<Option<UpdateReport>, DrawingError> {
    let v = drawing.view_mut(id).ok_or(DrawingError::UnknownView(id))?;
    let before = v.clone();
    edit(v)?;
    let result = update::regenerate_view(drawing, part, id);
    if result.is_err() {
        *drawing.view_mut(id).expect("existing view") = before;
    }
    result.map(|_| None)
}

fn kind_parent(kind: &ViewKind) -> Option<ViewId> {
    match kind {
        ViewKind::Base { .. } => None,
        ViewKind::Projected { parent, .. }
        | ViewKind::Section { parent, .. }
        | ViewKind::Detail { parent, .. }
        | ViewKind::Auxiliary { parent, .. } => Some(*parent),
    }
}

fn add_annotation(drawing: &mut Drawing, annotation: &Annotation) -> Result<(), DrawingError> {
    if drawing.annotation(annotation.id).is_some() {
        return Err(DrawingError::BadRequest(format!("{} is already on the drawing", annotation.id)));
    }
    let view = annotation.view;
    let sheet = if view == SHEET {
        drawing.sheets.first_mut().ok_or(DrawingError::BadRequest("the drawing has no sheet".into()))?
    } else {
        drawing.sheets.iter_mut().find(|s| s.view(view).is_some()).ok_or(DrawingError::UnknownView(view))?
    };
    sheet.annotations.push(annotation.clone());
    Ok(())
}

fn sheet_mut(drawing: &mut Drawing, sheet: usize) -> Result<&mut crate::Sheet, DrawingError> {
    drawing.sheets.get_mut(sheet).ok_or(DrawingError::BadRequest(format!("no sheet {sheet}")))
}

/// Changes a sheet's scale, rescaling every view that inherits it across all sheets.
fn set_sheet_scale(drawing: &mut Drawing, sheet: usize, scale: Scale) -> Result<(), DrawingError> {
    sheet_mut(drawing, sheet)?;
    let before = drawing
        .sheets
        .iter()
        .enumerate()
        .flat_map(|(si, s)| s.views.iter().map(move |v| (si, v)))
        .map(|(si, v)| (v.id, project::scale_in(drawing, si, v)))
        .collect();
    let old = std::mem::replace(&mut drawing.sheets[sheet].scale, scale);
    let result = rescale_changed(drawing, before);
    if result.is_err() {
        drawing.sheets[sheet].scale = old;
    }
    result
}

/// Deletes a view with its annotations, once nothing is derived from it.
fn delete_view(drawing: &mut Drawing, view: ViewId) -> Result<(), DrawingError> {
    let sheet = project::sheet_of(drawing, view).ok_or(DrawingError::UnknownView(view))?;
    if drawing.sheets.iter().flat_map(|s| &s.views).any(|v| v.parent() == Some(view)) {
        return Err(DrawingError::BadRequest("delete the views derived from this one first".into()));
    }
    let s = &mut drawing.sheets[sheet];
    for v in &mut s.views {
        if v.alignment.is_some_and(|a| a.parent == view) {
            v.alignment = None;
        }
    }
    s.views.retain(|v| v.id != view);
    s.tables.retain(|table| table.hole_view != Some(view));
    s.annotations.retain(|a| a.view != view);
    drawing.refused_views.retain(|failure| failure.view != view);
    Ok(())
}

/// Restores a projected view's alignment with its parent: side by side for left and
/// right folds, one above the other for up and down.
fn align(drawing: &mut Drawing, view: ViewId) -> Result<(), DrawingError> {
    let v = drawing.view(view).ok_or(DrawingError::UnknownView(view))?;
    let parent = v.parent().ok_or(DrawingError::BadRequest("a base view has nothing to align with".into()))?;
    let axis = match &v.kind {
        ViewKind::Projected { direction, .. } => match direction {
            FoldDirection::Left | FoldDirection::Right => AlignAxis::Horizontal,
            FoldDirection::Up | FoldDirection::Down => AlignAxis::Vertical,
            _ => return Err(DrawingError::BadRequest("pictorial views are not aligned".into())),
        },
        _ => AlignAxis::Horizontal,
    };
    let p = drawing.view(parent).ok_or(DrawingError::UnknownView(parent))?.position;
    let v = drawing.view_mut(view).expect("checked");
    v.alignment = Some(Alignment { parent, axis });
    match axis {
        AlignAxis::Horizontal => v.position.v = p.v,
        AlignAxis::Vertical => v.position.u = p.u,
    }
    Ok(())
}

/// Moves a view. An aligned view keeps the coordinate it shares with its parent, so it
/// slides only along its alignment; views aligned to the moved view follow it.
fn move_view(drawing: &mut Drawing, id: ViewId, to: Point2) -> Result<(), DrawingError> {
    let view = drawing.view(id).ok_or(DrawingError::UnknownView(id))?;
    let mut to = to;
    if let Some(al) = view.alignment
        && let Some(parent) = drawing.view(al.parent)
    {
        match al.axis {
            AlignAxis::Horizontal => to.v = parent.position.v,
            AlignAxis::Vertical => to.u = parent.position.u,
        }
    }
    let from = view.position;
    let delta = Point2::new(to.u - from.u, to.v - from.v);
    let mut moving = vec![id];
    let mut i = 0;
    while i < moving.len() {
        let current = moving[i];
        let children: Vec<ViewId> = drawing
            .sheets
            .iter()
            .flat_map(|s| &s.views)
            .filter(|v| v.alignment.is_some_and(|a| a.parent == current) && !moving.contains(&v.id))
            .map(|v| v.id)
            .collect();
        moving.extend(children);
        i += 1;
    }
    for m in moving {
        let v = drawing.view_mut(m).expect("collected from the drawing");
        v.position = Point2::new(v.position.u + delta.u, v.position.v + delta.v);
    }
    Ok(())
}

/// Changes a view's scale. Its line work and annotations, and those of children that use
/// the parent's scale, scale about each view's origin.
fn set_view_scale(drawing: &mut Drawing, id: ViewId, scale: ViewScale) -> Result<(), DrawingError> {
    let before = scale_followers(drawing, id)?;
    let view = drawing.view_mut(id).ok_or(DrawingError::UnknownView(id))?;
    let old = std::mem::replace(&mut view.scale, scale);
    let result = rescale_changed(drawing, before);
    if result.is_err() {
        drawing.view_mut(id).expect("existing view").scale = old;
    }
    result
}

/// Captures the effective scales of a view and descendants that inherit its scale.
fn scale_followers(drawing: &Drawing, id: ViewId) -> Result<Vec<(ViewId, f64)>, DrawingError> {
    let mut followers = vec![id];
    let mut i = 0;
    while i < followers.len() {
        let current = followers[i];
        let children: Vec<ViewId> = drawing
            .sheets
            .iter()
            .flat_map(|s| &s.views)
            .filter(|v| v.parent() == Some(current) && v.scale == ViewScale::Parent)
            .map(|v| v.id)
            .collect();
        followers.extend(children);
        i += 1;
    }
    followers.into_iter().map(|v| project::view_scale(drawing, v).map(|scale| (v, scale))).collect()
}

fn rescale_changed(drawing: &mut Drawing, before: Vec<(ViewId, f64)>) -> Result<(), DrawingError> {
    let mut changes = Vec::new();
    for (v, old) in before {
        let new = project::view_scale(drawing, v)?;
        let ratio = new / old;
        if (ratio - 1.0).abs() >= 1e-12 {
            check_break_scale(drawing, v, ratio)?;
            changes.push((v, old, new));
        }
    }
    for (v, old, new) in changes {
        rescale(drawing, v, old, new);
    }
    Ok(())
}

fn check_break_scale(drawing: &Drawing, id: ViewId, ratio: f64) -> Result<(), DrawingError> {
    let view = drawing.view(id).ok_or(DrawingError::UnknownView(id))?;
    project::validate_break(&view.geometry, view.display, ratio, !view.breaks.is_empty())
}

/// Scales a view's line work and its annotations' points about the view's origin, and
/// the marks its derived views keep on it.
fn rescale(drawing: &mut Drawing, v: ViewId, old_scale: f64, new_scale: f64) {
    let Some(view) = drawing.view_mut(v) else { return };
    let gap = view.geometry.breaks.first().copied();
    let ratio = new_scale / old_scale;
    project::rescale_view(view, ratio);
    let s = |p: Point2| project::scale_view_point(p, ratio, gap);
    let rounded = |p: Point2| {
        let q = s(p);
        Point2::new(crate::part::round(q.u), crate::part::round(q.v))
    };
    for child in drawing.sheets.iter_mut().flat_map(|sh| sh.views.iter_mut()).filter(|c| c.parent() == Some(v)) {
        match &mut child.kind {
            ViewKind::Section { line, .. } => line.iter_mut().for_each(|p| *p = rounded(*p)),
            ViewKind::Detail { center, radius, profile, .. } => {
                let boundary = if profile.len() >= 3 {
                    crate::Boundary::Polygon { points: std::mem::take(profile) }
                } else {
                    crate::Boundary::Circle { center: *center, radius: *radius }
                };
                *center = s(*center);
                match project::scaled_boundary(&boundary, ratio, gap) {
                    crate::Boundary::Circle { radius: scaled, .. } => *radius = scaled,
                    crate::Boundary::Polygon { points } => *profile = points,
                }
            }
            _ => {}
        }
    }
    if gap.is_some() {
        for table in drawing.sheets.iter_mut().flat_map(|s| &mut s.tables).filter(|t| t.hole_view == Some(v)) {
            for hole in &mut table.holes {
                let p = s(Point2::new(hole.centre.u * old_scale, hole.centre.v * old_scale));
                hole.centre = Point2::new(p.u / new_scale, p.v / new_scale);
            }
        }
    }
    for a in drawing.sheets.iter_mut().flat_map(|s| s.annotations.iter_mut()).filter(|a| a.view == v) {
        a.placement = rounded(a.placement);
        for anchor in &mut a.anchors {
            anchor.cached = rounded(anchor.cached);
        }
        if let AnnotationKind::Note { leader, .. } = &mut a.body {
            for p in leader {
                *p = rounded(*p);
            }
        }
    }
}

/// Undo and redo for a drawing: each command keeps the drawing as it was before it, so
/// an update undoes as one step with its geometry and references.
#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    undo: Vec<(&'static str, Drawing)>,
    redo: Vec<(&'static str, Drawing)>,
}

impl UndoStack {
    /// An empty stack.
    pub fn new() -> UndoStack {
        UndoStack::default()
    }

    /// Rewrites only the model paths in history when the drawing's file moves.
    pub fn map_model_paths(&mut self, mut path: impl FnMut(&DocRef) -> String) {
        for (_, drawing) in self.undo.iter_mut().chain(&mut self.redo) {
            drawing.model.path = path(&drawing.model);
        }
    }

    /// Runs `command`, with the part when given, and records it. A failed command leaves
    /// the drawing and the stack as they were.
    pub fn run(
        &mut self,
        drawing: &mut Drawing,
        command: &Command,
        part: Option<&Part>,
    ) -> Result<Option<UpdateReport>, DrawingError> {
        let before = drawing.clone();
        let result = match part {
            Some(p) => apply_with(drawing, command, p),
            None => apply(drawing, command).map(|_| None),
        };
        match result {
            Ok(r) => {
                self.undo.push((command.name(), before));
                self.redo.clear();
                Ok(r)
            }
            Err(e) => {
                *drawing = before;
                Err(e)
            }
        }
    }

    /// Undoes the last command; returns its name, or `None` when there is nothing to undo.
    pub fn undo(&mut self, drawing: &mut Drawing) -> Option<&'static str> {
        let (name, before) = self.undo.pop()?;
        self.redo.push((name, std::mem::replace(drawing, before)));
        Some(name)
    }

    /// Redoes the last undone command.
    pub fn redo(&mut self, drawing: &mut Drawing) -> Option<&'static str> {
        let (name, after) = self.redo.pop()?;
        self.undo.push((name, std::mem::replace(drawing, after)));
        Some(name)
    }

    /// How many commands [`undo`](Self::undo) can undo.
    pub fn depth(&self) -> usize {
        self.undo.len()
    }

    /// The name of the command [`undo`](Self::undo) would undo.
    pub fn undo_name(&self) -> Option<&'static str> {
        self.undo.last().map(|(n, _)| *n)
    }

    /// The name of the command [`redo`](Self::redo) would redo.
    pub fn redo_name(&self) -> Option<&'static str> {
        self.redo.last().map(|(n, _)| *n)
    }

    /// True when there is something to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// True when there is something to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
