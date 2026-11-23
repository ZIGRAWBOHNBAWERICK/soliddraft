//! Drawings of the open assembly: the assembly as `soliddraft-drawing` reads it, making a
//! drawing of it with a BOM and balloons, and bringing one up to date.

use crate::drawing::{DrawingDoc, DrawingSession};
use crate::state::AppState;
use soliddraft_assembly::{AssemblyDocument, Flags, OccurrenceIndex, OccurrencePath, Placement};
use soliddraft_document::{DocKind, DocRef, DocumentId};
use soliddraft_drawing::assembly::{AssemblyModel, BomRow, NewAssemblyDrawing, PlacedPart};
use soliddraft_drawing::{Command, DrawingMeta, library};
use soliddraft_kernel_api::Vec3;
use soliddraft_kernel_api::drawing::StandardView;
use std::collections::HashMap;

/// The open assembly as a drawing reads it: every shown part occurrence with its display
/// mesh placed where the assembly puts it, and again where its exploded view puts it when
/// it has one, and the parts-only bill of materials.
pub fn assembly_model(doc: &mut AssemblyDocument) -> Result<AssemblyModel, String> {
    doc.finish_loading();
    let exploded: HashMap<OccurrencePath, Placement> =
        if doc.assembly().explode.is_empty() { HashMap::new() } else { doc.exploded_worlds().into_iter().collect() };
    let path = doc.path().ok_or("save the assembly first: its drawing refers to its file")?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let reference = DocRef::new(doc.id(), DocKind::Assembly, name, doc.revision());
    let table = doc.table();
    let mut items: Vec<(DocumentId, BomRow)> = Vec::new();
    let mut parts = Vec::new();
    let mut moved = Vec::new();
    for row in (1..table.len() as u32).map(OccurrenceIndex) {
        let f = table.flags(row);
        if f.has(Flags::ASSEMBLY) || f.has(Flags::SUPPRESSED) || f.has(Flags::REMOVED) {
            continue;
        }
        let def_index = table.definition(row);
        let def = doc.store().get(def_index);
        if def.kind != DocKind::Part {
            continue;
        }
        let item = match items.iter_mut().position(|(d, _)| *d == def.key.document) {
            Some(i) => {
                items[i].1.quantity += 1;
                items[i].1.item
            }
            None => {
                let item = items.len() as u32 + 1;
                let file = doc.reference(def_index).map(|r| r.path.clone()).unwrap_or_default();
                let label = def.label.trim_end_matches(".dwprt").to_owned();
                items.push((def.key.document, BomRow { item, name: label, file, quantity: 1 }));
                item
            }
        };
        // Hidden parts count in the BOM; only shown ones are drawn.
        if f.has(Flags::HIDDEN) {
            continue;
        }
        let Some(mesh) = def.mesh.as_ref() else { continue };
        let path = table.path(row);
        let place = |world: Placement| {
            move |p: &[f32; 3]| {
                let w = world.apply(glam::DVec3::new(p[0] as f64, p[1] as f64, p[2] as f64));
                Vec3::new(w.x, w.y, w.z)
            }
        };
        let mut faces = vec![0u32; mesh.triangle_count()];
        for (k, face) in mesh.faces.iter().enumerate() {
            for t in face.first..face.first + face.count {
                if let Some(slot) = faces.get_mut(t as usize) {
                    *slot = k as u32;
                }
            }
        }
        let placed = |world: Placement| {
            let at = place(world);
            PlacedPart {
                path: path.to_string(),
                item,
                positions: mesh.positions.iter().map(&at).collect(),
                triangles: mesh.indices.as_chunks::<3>().0.to_vec(),
                faces: faces.clone(),
                edges: mesh.edges.iter().map(|e| e.iter().map(&at).collect()).collect(),
            }
        };
        parts.push(placed(table.world(row)));
        if !exploded.is_empty() {
            moved.push(placed(exploded.get(&path).copied().unwrap_or_else(|| table.world(row))));
        }
    }
    Ok(AssemblyModel {
        reference: Some(reference),
        parts,
        exploded: moved,
        bom: items.into_iter().map(|(_, r)| r).collect(),
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
    let model = assembly_model(&mut s.doc)?;
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
    let model = assembly_model(&mut a.doc)?;
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
