//! Drawings of the open assembly: the assembly as `soliddraft-drawing` reads it, making a
//! drawing of it with a BOM and balloons, and bringing one up to date.

use crate::drawing::{DrawingDoc, DrawingSession};
use crate::state::AppState;
use soliddraft_assembly::{AssemblyDocument, Flags, KernelSource, OccurrenceIndex, OccurrencePath, Placement};
use soliddraft_document::{ConfigurationId, DocKind, DocRef, DocumentId};
use soliddraft_drawing::assembly::{self, AssemblyModel, BomRow, NewAssemblyDrawing};
use soliddraft_drawing::placed::{PlacedPart, Rigid};
use soliddraft_drawing::{Command, Drawing, DrawingError, DrawingMeta, FoldDirection, View, ViewId, library, project};
use soliddraft_kernel_api::drawing::StandardView;
use soliddraft_kernel_api::{BodyId, Kernel, Vec3};
use soliddraft_model::Document as PartDocument;
use soliddraft_model::configs::DEFAULT_ROW;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// One shown part occurrence with its exact body.
struct ExactPart {
    kernel: Arc<dyn Kernel>,
    body: BodyId,
    placement: Rigid,
    exploded: Rigid,
    path: String,
    label: String,
    item: u32,
}

/// The open assembly's shown parts rebuilt on the kernel, as a drawing reads them: kept
/// alive while [`ExactParts::model`] lends them out.
pub struct ExactParts {
    reference: DocRef,
    parts: Vec<ExactPart>,
    exploded: bool,
    bom: Vec<BomRow>,
    /// The part documents that own the bodies.
    _documents: Vec<Arc<Mutex<PartDocument>>>,
}

impl ExactParts {
    /// The assembly as `soliddraft-drawing` reads it.
    pub fn model(&self) -> AssemblyModel<'_> {
        AssemblyModel {
            reference: Some(self.reference.clone()),
            parts: self
                .parts
                .iter()
                .map(|p| PlacedPart {
                    kernel: p.kernel.as_ref(),
                    body: p.body,
                    placement: p.placement,
                    component: p.path.clone(),
                    label: p.label.clone(),
                })
                .collect(),
            items: self.parts.iter().map(|p| p.item).collect(),
            exploded: if self.exploded { self.parts.iter().map(|p| p.exploded).collect() } else { Vec::new() },
            bom: self.bom.clone(),
        }
    }

    /// True when the assembly has an exploded view.
    pub fn has_exploded(&self) -> bool {
        self.exploded
    }
}

/// The placements counted on one BOM row: a part document in the configuration it shows,
/// `None` for the one its file was saved in when no placement of it names one.
struct BomGroup {
    key: (DocumentId, Option<ConfigurationId>),
    row: BomRow,
    /// The configuration's name, when a placement in the row names it.
    named: Option<String>,
}

fn rigid(p: Placement) -> Rigid {
    Rigid { rotation: p.rotation.to_array(), translation: Vec3::new(p.translation.x, p.translation.y, p.translation.z) }
}

/// The open assembly's shown parts with their exact bodies, each rebuilt on the kernel at
/// the revision the assembly pins, placed where the assembly puts it and where its
/// exploded view does, and the parts-only bill of materials. A part with no exact body,
/// such as a mesh reference, refuses the drawing by name.
pub fn exact_parts(doc: &mut AssemblyDocument) -> Result<ExactParts, String> {
    doc.finish_loading();
    let exploded: HashMap<OccurrencePath, Placement> =
        if doc.assembly().explode.is_empty() { HashMap::new() } else { doc.exploded_worlds().into_iter().collect() };
    let path = doc.path().ok_or("save the assembly first: its drawing refers to its file")?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let reference = DocRef::new(doc.id(), DocKind::Assembly, name, doc.revision());
    let source = KernelSource::new(crate::kernel());
    // One BOM row per part document in the configuration it shows, by the order the tree
    // first places it. A placement that names no configuration shows the one its file was
    // saved in, so it shares a row with a placement that names that one. The row is named
    // for its configuration when a placement in it names one.
    let mut items: Vec<BomGroup> = Vec::new();
    let mut parts = Vec::new();
    let mut documents = Vec::new();
    let rows = doc.table().len() as u32;
    let counted = |f: Flags| !(f.has(Flags::ASSEMBLY) || f.has(Flags::SUPPRESSED) || f.has(Flags::REMOVED));
    let named_documents: HashSet<DocumentId> = (1..rows)
        .map(OccurrenceIndex)
        .filter(|row| counted(doc.table().flags(*row)))
        .map(|row| doc.store().get(doc.table().definition(row)))
        .filter(|def| def.kind == DocKind::Part && def.key.configuration.is_some())
        .map(|def| def.key.document)
        .collect();
    for row in (1..rows).map(OccurrenceIndex) {
        let f = doc.table().flags(row);
        if !counted(f) {
            continue;
        }
        let def_index = doc.table().definition(row);
        let def = doc.store().get(def_index).clone();
        if def.kind != DocKind::Part {
            // Only a part has an exact body. A shown mesh reference refuses the drawing by
            // name rather than leaving a hole where it stands; a hidden one is not drawn.
            if f.has(Flags::HIDDEN) {
                continue;
            }
            let what = if def.kind == DocKind::Mesh { "a mesh reference" } else { "not a part" };
            return Err(format!(
                "{} is {what} with no exact body to draw; hide it or take it out of the assembly to draw the rest",
                doc.label(row)
            ));
        }
        let name = def.label.trim_end_matches(".dwprt").to_owned();
        // A placement that names a configuration, or that may share a row with one, reads
        // its part for which configuration it shows; the others of a document share a row.
        let (shows, named) = if def.key.configuration.is_some() || named_documents.contains(&def.key.document) {
            let part = doc.exact(def_index, &source).map_err(|e| format!("{name}: {e}"))?;
            let part = part.lock().map_err(|_| format!("{name}: the part is unavailable"))?;
            let shows = match &def.key.configuration {
                Some(c) => c.clone(),
                None => part.active_configuration().cloned().unwrap_or_else(|| ConfigurationId(DEFAULT_ROW.into())),
            };
            let named = def.key.configuration.as_ref().map(|c| {
                let names = part.configuration_names();
                names.iter().find(|(id, _)| id == c).map_or_else(|| c.0.clone(), |(_, n)| n.clone())
            });
            (Some(shows), named)
        } else {
            (None, None)
        };
        let key = (def.key.document, shows);
        let item = match items.iter_mut().find(|g| g.key == key) {
            Some(g) => {
                g.row.quantity += 1;
                if g.named.is_none() {
                    g.named = named;
                }
                g.row.item
            }
            None => {
                let item = items.len() as u32 + 1;
                let file = doc.reference(def_index).map(|r| r.path.clone()).unwrap_or_default();
                items.push(BomGroup { key, row: BomRow { item, name, file, quantity: 1 }, named });
                item
            }
        };
        // Hidden parts count in the BOM; only shown ones are drawn.
        if f.has(Flags::HIDDEN) {
            continue;
        }
        let label = doc.label(row);
        let exact = doc.exact(def_index, &source).map_err(|e| format!("{label}: {e}"))?;
        let (kernel, body) = {
            let part = exact.lock().map_err(|_| format!("{label}: the part is unavailable"))?;
            let body = part.body().ok_or_else(|| format!("{label}: the part has no body"))?;
            (part.kernel().clone(), body)
        };
        let occurrence = doc.table().path(row);
        let world = doc.table().world(row);
        parts.push(ExactPart {
            kernel,
            body,
            placement: rigid(world),
            exploded: rigid(exploded.get(&occurrence).copied().unwrap_or(world)),
            path: occurrence.to_string(),
            label,
            item,
        });
        documents.push(exact);
    }
    Ok(ExactParts {
        reference,
        parts,
        exploded: !exploded.is_empty(),
        bom: items
            .into_iter()
            .map(|g| match g.named {
                Some(shown) => BomRow { name: format!("{} ({shown})", g.row.name), ..g.row },
                None => g.row,
            })
            .collect(),
        _documents: documents,
    })
}

/// What a new drawing of the assembly shows.
#[derive(Clone, Debug)]
pub struct AssemblyDrawingRequest {
    /// A template id, such as `iso-a3`.
    pub template: String,
    /// The views; `None` for the isometric alone when the assembly has an exploded view,
    /// else the isometric and the front.
    pub views: Option<Vec<StandardView>>,
    /// Show the exploded view; `None` for whenever the assembly has one.
    pub exploded: Option<bool>,
}

impl Default for AssemblyDrawingRequest {
    fn default() -> AssemblyDrawingRequest {
        AssemblyDrawingRequest { template: "iso-a3".into(), views: None, exploded: None }
    }
}

/// Makes a drawing of the open assembly, its first view ballooned, and makes it the open
/// drawing. The assembly must be saved: the drawing refers to its file.
pub fn make_drawing(state: &mut AppState, request: &AssemblyDrawingRequest) -> Result<String, String> {
    let s = state.assembly.session.as_mut().ok_or("no assembly is open")?;
    if s.doc.is_dirty() || s.doc.path().is_none() {
        return Err("save the assembly first: its drawing refers to the saved file".into());
    }
    let templates = &library::library().templates;
    if !templates.iter().any(|t| t.id == request.template) {
        return Err(format!("no template `{}`", request.template));
    }
    let has_explode = !s.doc.assembly().explode.is_empty();
    let exploded = request.exploded.unwrap_or(has_explode);
    if exploded && !has_explode {
        return Err("the assembly has no exploded view; add explode steps or leave exploded off".into());
    }
    let views = request.views.clone().unwrap_or_else(|| {
        if exploded { vec![StandardView::Isometric] } else { vec![StandardView::Isometric, StandardView::Front] }
    });
    let exact = exact_parts(&mut s.doc)?;
    let model = exact.model();
    if model.parts.is_empty() {
        return Err("the assembly shows no parts to draw".into());
    }
    let name = s.doc.assembly().name.clone();
    let new = NewAssemblyDrawing {
        template: library::template(&request.template).template(),
        meta: DrawingMeta {
            title: name.clone(),
            number: "SD-0001".into(),
            revision: "A".into(),
            designer: std::env::var("USER").map(|u| u.chars().take(24).collect()).unwrap_or_default(),
            date: crate::drawing::today(),
            units: Default::default(),
        },
        views,
        scale: None,
        balloons: true,
        exploded,
    };
    let drawing = soliddraft_drawing::assembly::make_assembly_drawing(&model, &new).map_err(|e| e.to_string())?;
    let rows = model.bom.len();
    let mut doc = DrawingDoc::new(drawing);
    doc.touch();
    let session = DrawingSession::new(doc);
    state.assembly.active = false;
    state.drawing.tree_hidden = false;
    state.drawing.session = Some(session);
    state.drawing.new = None;
    state.drawing.active = true;
    state.tool_document = Some(crate::app::Tab::Drawing);
    Ok(format!(
        "Drawing of {name} · {} · {rows} BOM row{}",
        if exploded { "exploded" } else { "assembled" },
        if rows == 1 { "" } else { "s" }
    ))
}

/// True when the open drawing shows an assembly.
pub fn drawing_is_of_assembly(state: &AppState) -> bool {
    state.drawing.session.as_ref().is_some_and(|s| s.doc.drawing.model.kind == DocKind::Assembly)
}

/// Brings the open drawing of an assembly to the open assembly: views, BOM, balloons and
/// pin as one undo step. The drawing's assembly must be the open one.
pub fn update(state: &mut AppState) -> Result<String, String> {
    let d = state.drawing.session.as_ref().ok_or("no drawing is open")?;
    let wanted = d.doc.drawing.model.id;
    let a = state.assembly.session.as_mut().ok_or("open the drawing's assembly first")?;
    if Some(a.doc.id()) != wanted {
        return Err("the open assembly is not the one this drawing shows; open it first".into());
    }
    if a.doc.is_dirty() {
        return Err("save the assembly first: the drawing pins its saved revision".into());
    }
    let exact = exact_parts(&mut a.doc)?;
    let model = exact.model();
    let d = state.drawing.session.as_mut().expect("checked above");
    let next =
        soliddraft_drawing::assembly::update_assembly_drawing(&d.doc.drawing, &model).map_err(|e| e.to_string())?;
    let dangling = next.annotations_needing_attention().count();
    d.doc.apply(Command::UpdateFromAssembly { drawing: Box::new(next) }, None)?;
    d.doc.seen_digest = d.doc.drawing.model.pinned.clone();
    Ok(format!(
        "Updated from the assembly · {} BOM rows · {dangling} balloon{} to check",
        model.bom.len(),
        if dangling == 1 { "" } else { "s" }
    ))
}

/// Places a view `make` derives from the assembly drawing's view `parent`, every part
/// drawn by its kernel, as one undo step. The drawing's assembly must be the open one, at
/// the revision the drawing shows.
/// Returns the new view's label and id.
fn place_derived(
    state: &mut AppState,
    parent: ViewId,
    what: &str,
    make: impl FnOnce(&Drawing, &AssemblyModel<'_>, usize) -> Result<View, DrawingError>,
) -> Result<(String, u32), String> {
    let d = state.drawing.session.as_ref().ok_or("no drawing is open")?;
    let wanted = d.doc.drawing.model.id;
    let pinned = d.doc.drawing.model.pinned.clone();
    let a = state.assembly.session.as_mut().ok_or("open the drawing's assembly first")?;
    if Some(a.doc.id()) != wanted {
        return Err("the open assembly is not the one this drawing shows; open it first".into());
    }
    // A view of another revision would sit beside the drawing's views of this one.
    if a.doc.revision() != pinned {
        return Err(format!(
            "{what}: the assembly has changed since the drawing's views were made; Rebuild the drawing first"
        ));
    }
    let exact = exact_parts(&mut a.doc)?;
    let model = exact.model();
    let d = state.drawing.session.as_mut().expect("checked above");
    let drawing = &d.doc.drawing;
    let sheet = project::sheet_of(drawing, parent).ok_or("no such view")?;
    let view = make(drawing, &model, sheet).map_err(|e| format!("{what}: {e}"))?;
    let id = view.id;
    d.doc.apply(Command::PlaceView { sheet, view }, None)?;
    let drawing = &d.doc.drawing;
    let label = drawing
        .sheet_of(id)
        .and_then(|s| s.view(id).map(|v| project::view_label_in(drawing, s, v)))
        .unwrap_or_default();
    Ok((label, id.0))
}

/// Section of an assembly drawing's view `parent` along a horizontal or vertical line
/// through the model point `through`, every part cut by its kernel, placed in line with
/// the view as one undo step. Returns the section's label and id.
pub fn section(
    state: &mut AppState,
    parent: ViewId,
    through: Vec3,
    horizontal: bool,
    flip: bool,
) -> Result<(String, u32), String> {
    place_derived(state, parent, "Section", |drawing, model, sheet| {
        assembly::new_assembly_section(drawing, model, sheet, parent, through, horizontal, flip)
    })
}

/// A view folded off an assembly drawing's view `parent` on `direction`'s side, placed
/// as one undo step. Returns its label and id.
pub fn projected(state: &mut AppState, parent: ViewId, direction: FoldDirection) -> Result<(String, u32), String> {
    place_derived(state, parent, "Projected", |drawing, model, sheet| {
        assembly::new_assembly_projected(drawing, model, sheet, parent, direction)
    })
}

/// A detail of an assembly drawing's view `parent` round the model point `center`,
/// inside a circle of `radius` view millimetres, magnified `factor` times, placed as one
/// undo step. Returns its label and id.
pub fn detail(
    state: &mut AppState,
    parent: ViewId,
    center: Vec3,
    radius: f64,
    factor: f64,
) -> Result<(String, u32), String> {
    place_derived(state, parent, "Detail", |drawing, model, sheet| {
        assembly::new_assembly_detail(drawing, model, sheet, parent, center, radius, factor)
    })
}
