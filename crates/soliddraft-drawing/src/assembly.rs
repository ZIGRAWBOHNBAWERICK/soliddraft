//! Drawings of assemblies: views of every placed part, a bill of materials and balloons
//! that number the parts.
//!
//! The caller hands over the assembly as an [`AssemblyModel`]: each shown part occurrence
//! as a [`PlacedPart`], its body in its own kernel and where the assembly places it (and
//! where its exploded view puts it), each with the BOM item it belongs to, and the BOM
//! rows. Views go through the same code as a part drawing's, asked of
//! [`PlacedBodies`]: each part's line work is its kernel's own drawing query in the part's
//! frame, and a part the kernel refuses refuses the view by name. Base, projected,
//! section and detail views all work; a section cuts each part through its kernel, so its
//! hatch regions are the kernel's. Where one part hides another, the visibility is
//! measured against the parts' meshes and the view says so (see [`crate::placed`]).
//!
//! Each curve's source names its occurrence: its feature is the part's own tag behind the
//! occurrence path, such as `4/f2` ([`component_of`]). A balloon attaches to it.
//!
//! The BOM lists parts only: one row per part document, in the order the tree first
//! places it, with how many times it is placed. Hidden parts count; suppressed parts do
//! not. A balloon carries its row's item number, and points at a visible edge of one
//! occurrence of that part.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::drawing::{StandardView, ViewSpec};
use soliddraft_kernel_api::{Point2, Vec3};

use crate::annotation::{Anchor, Annotation, AnnotationKind, AnnotationState, Attach, StyleOverrides};
use crate::entity::{EntityKind, EntityRef};
use crate::ids::{AnnotationId, ViewId};
use crate::part::{Part, PartIndex, PartSource, Requirements, round};
use crate::placed::{PlacedBodies, PlacedPart, Rigid, component_of};
use crate::project::{self, Contexts, ViewContext, ViewTransform};
use crate::sheet::{Scale, Table, TableKind};
use crate::template::Template;
use crate::view::{FoldDirection, Orientation, View, ViewDisplay, ViewGeometry, ViewKind, ViewScale};
use crate::{ContentHash, DocRef, Drawing, DrawingError, DrawingMeta};

/// One row of a bill of materials.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BomRow {
    /// The item number, from 1.
    pub item: u32,
    /// The part's name.
    pub name: String,
    /// The part's filename in the shared bill of materials.
    #[serde(serialize_with = "serialize_bom_filename", deserialize_with = "deserialize_bom_filename")]
    pub file: String,
    /// How many times the assembly places it.
    pub quantity: u32,
}

pub(crate) fn bom_filename(path: &str) -> &str {
    let name = path.rsplit(['/', '\\']).next().unwrap_or_default();
    if matches!(name.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic()) { &name[2..] } else { name }
}

fn serialize_bom_filename<S: serde::Serializer>(path: &str, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(bom_filename(path))
}

fn deserialize_bom_filename<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    String::deserialize(deserializer).map(|path| bom_filename(&path).to_owned())
}

fn drawing_bom(rows: &[BomRow]) -> Vec<BomRow> {
    rows.iter().map(|row| BomRow { file: bom_filename(&row.file).to_owned(), ..row.clone() }).collect()
}

/// An assembly as a drawing reads it.
#[derive(Clone, Default)]
pub struct AssemblyModel<'k> {
    /// The assembly, pinned at the revision the parts come from.
    pub reference: Option<DocRef>,
    /// Every part occurrence that is shown, placed. Each part's component is its
    /// occurrence path, such as "4/2".
    pub parts: Vec<PlacedPart<'k>>,
    /// The BOM item each part is counted under, in the order of `parts`.
    pub items: Vec<u32>,
    /// Where the assembly's exploded view puts each part, in the order of `parts`; empty
    /// when it has none.
    pub exploded: Vec<Rigid>,
    /// The bill of materials.
    pub bom: Vec<BomRow>,
}

/// What to put on a new drawing of an assembly.
#[derive(Clone, Debug)]
pub struct NewAssemblyDrawing {
    /// The template.
    pub template: Template,
    /// Document properties.
    pub meta: DrawingMeta,
    /// The views, side by side, the first one ballooned.
    pub views: Vec<StandardView>,
    /// The sheet scale, or `None` for the largest of the template's that fits.
    pub scale: Option<Scale>,
    /// Number the parts in the first view.
    pub balloons: bool,
    /// Show the exploded view in every view.
    pub exploded: bool,
}

/// The bill of materials' columns: heading and width in millimetres.
pub const BOM_COLUMNS: [(&str, f64); 4] = [("ITEM", 14.0), ("PART", 56.0), ("FILE", 56.0), ("QTY", 12.0)];
/// Height of a BOM row, in millimetres.
const ROW: f64 = 7.0;
/// Room kept around views and tables.
const MARGIN: f64 = 12.0;
/// A balloon's radius in sheet millimetres.
pub const BALLOON_RADIUS: f64 = 4.5;
/// How far outside a view's box balloons start, in sheet millimetres.
const BALLOON_OFFSET: f64 = 12.0;

/// Width and height of a BOM with `rows` rows: a title row, a heading row and the rows.
pub fn bom_table_size(rows: usize) -> (f64, f64) {
    (BOM_COLUMNS.iter().map(|(_, w)| w).sum(), ROW * (rows as f64 + 2.0))
}

/// A template's drawing frame, inside its margins and zone band: `[x0, y0, x1, y1]`.
fn drawing_frame(t: &Template) -> [f64; 4] {
    let band = crate::library::library().templates.iter().find(|f| f.id == t.id.as_str()).map_or(0.0, |f| f.zones.band);
    [
        t.margins.left + band,
        t.margins.bottom + band,
        t.size.width_mm - t.margins.right - band,
        t.size.height_mm - t.margins.top - band,
    ]
}

/// Refuses a BOM of `rows` rows hanging from `position`, its top-left corner, that would
/// leave the drawing frame of template `t`.
fn bom_fits(t: &Template, position: Point2, rows: usize) -> Result<(), DrawingError> {
    let [x0, y0, x1, y1] = drawing_frame(t);
    let (w, h) = bom_table_size(rows);
    let e = 1e-9;
    if position.u < x0 - e || position.u + w > x1 + e || position.v > y1 + e || position.v - h < y0 - e {
        return Err(DrawingError::BadRequest(format!(
            "a BOM of {rows} rows is {h:.0} mm tall and does not fit on the {} sheet; use a larger sheet",
            t.size.name
        )));
    }
    Ok(())
}

impl<'k> AssemblyModel<'k> {
    /// The parts as one body, placed as the assembly has them or, with `exploded` and an
    /// exploded view, where that view puts them.
    pub fn bodies(&self, exploded: bool) -> Result<PlacedBodies<'k>, DrawingError> {
        let mut parts = self.parts.clone();
        if exploded && self.exploded.len() == parts.len() {
            for (p, at) in parts.iter_mut().zip(&self.exploded) {
                p.placement = *at;
            }
        }
        Ok(PlacedBodies::new(parts)?)
    }

    /// The revision the views are drawn from.
    pub fn digest(&self) -> ContentHash {
        self.reference.as_ref().map(|r| r.pinned.clone()).unwrap_or_else(|| ContentHash::of_bytes(b""))
    }

    /// The occurrence paths of the parts counted under `item`.
    fn paths_of(&self, item: u32) -> impl Iterator<Item = &str> {
        self.parts.iter().zip(&self.items).filter(move |(_, i)| **i == item).map(|(p, _)| p.component.as_str())
    }
}

/// The assembly as the part the drawing code reads: the placed parts bound as a
/// read-only drawing geometry.
pub fn as_part<'a>(model: &AssemblyModel<'_>, bodies: &'a PlacedBodies<'_>) -> Part<'a> {
    Part {
        geometry: Box::new(bodies),
        digest: model.digest(),
        dimensions: Vec::new(),
        namer: None,
        preview: false,
        source: PartSource::default(),
        requirements: Requirements::default(),
    }
}

/// Runs `f` with the assembly as a part, placed as `exploded` asks, and its index.
fn with_part<R>(
    model: &AssemblyModel<'_>,
    exploded: bool,
    f: impl FnOnce(&Part<'_>, &PartIndex<'_>) -> Result<R, DrawingError>,
) -> Result<R, DrawingError> {
    let bodies = model.bodies(exploded)?;
    let part = as_part(model, &bodies);
    let index = PartIndex::build(&part)?;
    f(&part, &index)
}

/// The line work of a view of the assembly at `scale`, and its transform.
pub fn assembly_view(
    model: &AssemblyModel<'_>,
    spec: &ViewSpec,
    scale: f64,
    exploded: bool,
) -> Result<(ViewGeometry, ViewTransform), DrawingError> {
    with_part(model, exploded, |part, index| {
        let curves = project::view_curves(part, spec)?;
        let xf = project::transform(&curves, spec, scale)?;
        Ok((project::project(index, &curves, &xf, part.digest.as_str(), false).geometry, xf))
    })
}

/// Whether a view shows the exploded view: its own flag, which a derived view takes from
/// its parent when it is made.
fn exploded_at(drawing: &Drawing, view: ViewId) -> bool {
    drawing.view(view).is_some_and(|v| v.exploded)
}

/// What view `id` shows of the assembly now, computed fresh with its parents.
pub fn context(drawing: &Drawing, model: &AssemblyModel<'_>, id: ViewId) -> Result<ViewContext, DrawingError> {
    with_part(model, exploded_at(drawing, id), |part, index| ViewContext::compute(drawing, part, index, id))
}

/// Refuses `model` at another revision than the one the drawing's views show: a view
/// derived from it would sit beside views of the old one, and the drawing would keep its
/// old pin.
fn at_pinned_revision(drawing: &Drawing, model: &AssemblyModel<'_>) -> Result<(), DrawingError> {
    if drawing.model.pinned != model.digest() {
        return Err(DrawingError::BadRequest(
            "the assembly has changed since the drawing's views were made; Rebuild the drawing first".into(),
        ));
    }
    Ok(())
}

/// A view folded off `parent` on `direction`'s side, at the parent's scale, placed in
/// line with it. The model must be at the revision the drawing shows.
pub fn new_assembly_projected(
    drawing: &Drawing,
    model: &AssemblyModel<'_>,
    sheet: usize,
    parent: ViewId,
    direction: FoldDirection,
) -> Result<View, DrawingError> {
    at_pinned_revision(drawing, model)?;
    let exploded = exploded_at(drawing, parent);
    let (mut view, _) = with_part(model, exploded, |part, index| {
        crate::derived::new_projected(drawing, part, index, sheet, parent, direction)
    })?;
    view.exploded = exploded;
    Ok(view)
}

/// A section of `parent` along a horizontal or vertical cutting line through the model
/// point `through`, every part cut by its own kernel. A cut that meets no part is refused,
/// and so is a model at another revision than the one the drawing shows.
pub fn new_assembly_section(
    drawing: &Drawing,
    model: &AssemblyModel<'_>,
    sheet: usize,
    parent: ViewId,
    through: Vec3,
    horizontal: bool,
    flip: bool,
) -> Result<View, DrawingError> {
    at_pinned_revision(drawing, model)?;
    let exploded = exploded_at(drawing, parent);
    let (mut view, _) = with_part(model, exploded, |part, index| {
        let line = crate::derived::cutting_line(drawing, part, index, parent, through, horizontal)?;
        crate::derived::new_section(drawing, part, index, sheet, parent, line, flip, None)
    })?;
    if view.geometry.hatch.is_empty() {
        return Err(DrawingError::BadRequest("the cutting line crosses no part of the assembly".into()));
    }
    view.exploded = exploded;
    Ok(view)
}

/// A detail of `parent` round the model point `center`, inside a circle of `radius`
/// (the parent's view millimetres), magnified `factor` times. The model must be at the
/// revision the drawing shows.
pub fn new_assembly_detail(
    drawing: &Drawing,
    model: &AssemblyModel<'_>,
    sheet: usize,
    parent: ViewId,
    center: Vec3,
    radius: f64,
    factor: f64,
) -> Result<View, DrawingError> {
    at_pinned_revision(drawing, model)?;
    let exploded = exploded_at(drawing, parent);
    let (mut view, _) = with_part(model, exploded, |part, index| {
        let at = crate::derived::context(drawing, part, index, parent)?.xf.point(center);
        crate::derived::new_detail(drawing, part, index, sheet, parent, at, radius, Vec::new(), factor, None)
    })?;
    view.exploded = exploded;
    Ok(view)
}

fn orientation(view: &View) -> Option<StandardView> {
    match &view.kind {
        ViewKind::Base { orientation: Orientation::Standard { view } } => Some(*view),
        _ => None,
    }
}

/// Makes a drawing of an assembly: the views side by side at the largest of the
/// template's scales that fits beside the BOM, the BOM in the top-right corner, and a
/// balloon for every BOM row on the first view.
pub fn make_assembly_drawing(model: &AssemblyModel<'_>, new: &NewAssemblyDrawing) -> Result<Drawing, DrawingError> {
    let reference =
        model.reference.clone().ok_or_else(|| DrawingError::BadRequest("the assembly has no reference".into()))?;
    if new.views.is_empty() {
        return Err(DrawingError::BadRequest("no views asked for".into()));
    }
    let mut drawing = Drawing::new(reference, &new.template);
    drawing.meta = new.meta.clone();
    let t = &new.template;
    let [x0, y0, x1, y1] = drawing_frame(t);
    let (bom_w, bom_h) = bom_table_size(model.bom.len());
    bom_fits(t, Point2::new(x1 - bom_w, y1), model.bom.len())?;
    // The BOM in the top-right corner; the views left of it, above the title block.
    drawing.sheets[0].tables.push(Table {
        kind: TableKind::Bom,
        position: Point2::new(x1 - bom_w, y1),
        revisions: Vec::new(),
        bom: drawing_bom(&model.bom),
        hole_view: None,
        holes: Vec::new(),
    });
    // The views use the drawing area below the BOM and above the title block.
    let title_top = t.title_block.position.v + t.title_block.height + MARGIN;
    let region = (x0 + MARGIN, title_top.max(y0 + MARGIN), x1 - MARGIN, y1 - bom_h - MARGIN);
    let slot_w = (region.2 - region.0) / new.views.len() as f64;
    let slot_h = region.3 - region.1;
    if slot_w <= 0.0 || slot_h <= 0.0 {
        return Err(DrawingError::BadRequest(format!(
            "a BOM of {} rows leaves no room for the views on the {} sheet; use a larger sheet",
            model.bom.len(),
            t.size.name
        )));
    }
    // Extents at full scale, with room for balloons around each view.
    let mut extents = Vec::new();
    for v in &new.views {
        let (g, _) = assembly_view(model, &ViewSpec::standard(*v), 1.0, new.exploded)?;
        extents.push((g.bbox[2] - g.bbox[0], g.bbox[3] - g.bbox[1]));
    }
    let room = 2.0 * (BALLOON_OFFSET + 2.0 * BALLOON_RADIUS);
    let fits = |s: f64| extents.iter().all(|(w, h)| s * w + room <= slot_w && s * h + room <= slot_h);
    let mut scales = t.scales.clone();
    scales.sort_by(|a, b| b.factor().total_cmp(&a.factor()));
    let scale = match new.scale {
        Some(s) => s,
        None => scales.iter().copied().find(|s| fits(s.factor())).ok_or_else(|| {
            DrawingError::BadRequest(format!(
                "the views do not fit beside the BOM on the {} sheet at any of its scales; use a larger sheet",
                t.size.name
            ))
        })?,
    };
    drawing.sheets[0].scale = scale;
    for (i, v) in new.views.iter().enumerate() {
        let (geometry, _) = assembly_view(model, &ViewSpec::standard(*v), scale.factor(), new.exploded)?;
        let centre = Point2::new(round(region.0 + slot_w * (i as f64 + 0.5)), round(region.1 + slot_h / 2.0));
        drawing.sheets[0].views.push(View {
            id: ViewId(i as u32 + 1),
            kind: ViewKind::Base { orientation: Orientation::Standard { view: *v } },
            position: centre,
            alignment: None,
            scale: ViewScale::Sheet,
            display: ViewDisplay::PICTORIAL,
            excluded_from_update: false,
            geometry,
            label: new.exploded.then(|| "Exploded view".to_owned()),
            crop: None,
            broken_out: Vec::new(),
            exploded: new.exploded && !model.exploded.is_empty(),
            breaks: Vec::new(),
            half: None,
        });
    }
    if new.balloons {
        add_balloons(&mut drawing, model, ViewId(1))?;
    }
    Ok(drawing)
}

/// Where a balloon for `item` attaches in a view: the middle of the longest visible
/// stretch of an edge of the first occurrence of the item that shows one (or of
/// `occurrence`), in model space and view millimetres, with that occurrence's path. A
/// stretch is what is left of an edge where other parts hide the rest, so the leader
/// lands on line work the view shows.
fn attach_point(
    model: &AssemblyModel<'_>,
    ctx: &ViewContext,
    item: u32,
    occurrence: Option<&str>,
) -> Option<(String, Vec3, Point2)> {
    let wanted: Vec<&str> = match occurrence {
        Some(p) => vec![p],
        None => model.paths_of(item).collect(),
    };
    // The model curves the view was projected from, beside their projections: the
    // projection keeps, in order, every curve that draws as a path.
    let kept: Vec<_> =
        ctx.curves.curves.iter().filter(|c| project::path_of(&c.curve, &ctx.xf).elements().len() >= 2).collect();
    if kept.len() != ctx.projected.geometry.curves.len() {
        return None;
    }
    let drawn: Vec<_> = kept.into_iter().zip(&ctx.projected.geometry.curves).collect();
    for path in wanted {
        let Some((best, _)) = drawn
            .iter()
            .filter(|(_, c)| {
                c.visible && c.source.kind == EntityKind::Edge && component_of(&c.source.feature) == Some(path)
            })
            .max_by(|(a, _), (b, _)| a.curve.length().total_cmp(&b.curve.length()))
        else {
            continue;
        };
        let at = best.curve.point_at(0.5);
        return Some((path.to_owned(), at, ctx.xf.point(at)));
    }
    None
}

/// Puts a balloon for every BOM row that no balloon on `view` points at, outside the
/// view's box, on the line from the view's middle through its anchor.
fn add_balloons(drawing: &mut Drawing, model: &AssemblyModel<'_>, view: ViewId) -> Result<(), DrawingError> {
    let sheet = crate::project::sheet_of(drawing, view).ok_or(DrawingError::UnknownView(view))?;
    let ctx = context(drawing, model, view)?;
    let [bx0, by0, bx1, by1] = ctx.projected.geometry.bbox;
    let half = Point2::new((bx1 - bx0) / 2.0, (by1 - by0) / 2.0);
    // Only a balloon that points at its part numbers a row: a dangling one claims none.
    let taken: Vec<u32> = drawing.sheets[sheet]
        .annotations
        .iter()
        .filter(|a| a.view == view && !matches!(a.state, AnnotationState::Dangling { .. }))
        .filter_map(|a| match &a.body {
            AnnotationKind::Balloon { item, .. } => Some(*item),
            _ => None,
        })
        .collect();
    let mut placed: Vec<Point2> = drawing.sheets[sheet]
        .annotations
        .iter()
        .filter(|a| a.view == view && matches!(a.body, AnnotationKind::Balloon { .. }))
        .map(|a| a.placement)
        .collect();
    // View millimetres are sheet millimetres about the view's origin.
    let r = BALLOON_RADIUS;
    for row in model.bom.iter().filter(|r| !taken.contains(&r.item)) {
        let Some((path, at, cached)) = attach_point(model, &ctx, row.item, None) else { continue };
        // Out from the middle, past the box, then along until clear of the others.
        let d = Point2::new(cached.u, cached.v);
        let len = (d.u * d.u + d.v * d.v).sqrt().max(1e-9);
        let (ux, uy) = if len > 1e-6 { (d.u / len, d.v / len) } else { (0.0, 1.0) };
        let reach = (half.u * ux.abs() + half.v * uy.abs()) + BALLOON_OFFSET;
        let mut p = Point2::new(round(ux * reach), round(uy * reach));
        for _ in 0..24 {
            if placed.iter().all(|q| ((q.u - p.u).powi(2) + (q.v - p.v).powi(2)).sqrt() > 2.6 * r) {
                break;
            }
            // Step round the view, keeping the distance.
            let angle = p.v.atan2(p.u) + 0.25;
            let dist = (p.u * p.u + p.v * p.v).sqrt();
            p = Point2::new(round(dist * angle.cos()), round(dist * angle.sin()));
        }
        placed.push(p);
        let id = AnnotationId(drawing.next_annotation_id().0);
        let entity = EntityRef::new(EntityKind::Edge, &path, "assembly_edge", 0, at, 0.0);
        drawing.sheets[sheet].annotations.push(Annotation {
            id,
            view,
            body: AnnotationKind::Balloon { item: row.item, occurrence: path },
            anchors: vec![Anchor { entity, attach: Attach::Point { at }, cached }],
            placement: p,
            style: StyleOverrides::default(),
            state: AnnotationState::Ok,
            source: None,
        });
    }
    Ok(())
}

/// The drawing brought to `model`: every view's line work, whatever its kind, the BOM
/// rows, each balloon moved onto its occurrence (dangling when the occurrence is gone or
/// shows no edge in the view, its number from the BOM row its occurrence is now counted
/// under), and a balloon on the first ballooned view for every new row. Pins the new
/// revision. A view excluded from update keeps its line work and its balloons.
pub fn update_assembly_drawing(drawing: &Drawing, model: &AssemblyModel<'_>) -> Result<Drawing, DrawingError> {
    let mut out = drawing.clone();
    if let Some(r) = &model.reference {
        out.model.pinned = r.pinned.clone();
    }
    let item_of: BTreeMap<&str, u32> =
        model.parts.iter().zip(&model.items).map(|(p, i)| (p.component.as_str(), *i)).collect();
    // Only these views are written. A view excluded from updates keeps its line work and
    // its balloons, though a view derived from it is computed from it as it is now.
    let included: HashSet<ViewId> =
        drawing.sheets.iter().flat_map(|s| &s.views).filter(|v| !v.excluded_from_update).map(|v| v.id).collect();
    // Every view, parents first, once for each placement the views show.
    let mut contexts: HashMap<ViewId, ViewContext> = HashMap::new();
    for exploded in [false, true] {
        let ids: Vec<ViewId> = drawing
            .sheets
            .iter()
            .flat_map(|s| &s.views)
            .filter(|v| v.exploded == exploded && !v.excluded_from_update)
            .map(|v| v.id)
            .collect();
        if ids.is_empty() {
            continue;
        }
        with_part(model, exploded, |part, index| {
            let mut memo = Contexts::new();
            for id in ids {
                project::compute_in(drawing, part, index, id, &mut memo)?;
            }
            contexts.extend(memo);
            Ok(())
        })?;
    }
    let bom = drawing_bom(&model.bom);
    let mut ballooned: Option<ViewId> = None;
    for si in 0..out.sheets.len() {
        let template = crate::library::template(out.sheets[si].template.as_str()).template();
        for t in out.sheets[si].tables.iter_mut().filter(|t| t.kind == TableKind::Bom) {
            bom_fits(&template, t.position, model.bom.len())?;
            t.bom.clone_from(&bom);
        }
        let views: Vec<ViewId> = out.sheets[si].views.iter().map(|v| v.id).collect();
        for id in views.into_iter().filter(|id| included.contains(id)) {
            let Some(ctx) = contexts.get(&id) else { continue };
            for a in out.sheets[si].annotations.iter_mut().filter(|a| a.view == id) {
                let AnnotationKind::Balloon { item, occurrence } = &mut a.body else { continue };
                ballooned.get_or_insert(id);
                let Some(now) = item_of.get(occurrence.as_str()) else {
                    a.state = AnnotationState::Dangling {
                        lost: vec![0],
                        cause: format!("{occurrence} is no longer in the assembly"),
                    };
                    continue;
                };
                *item = *now;
                match attach_point(model, ctx, *now, Some(occurrence)) {
                    Some((_, at, cached)) => {
                        if let Some(anchor) = a.anchors.first_mut() {
                            anchor.attach = Attach::Point { at };
                            anchor.entity.fingerprint.point = at;
                            anchor.cached = cached;
                        }
                        a.state = AnnotationState::Ok;
                    }
                    None => {
                        a.state = AnnotationState::Dangling {
                            lost: vec![0],
                            cause: format!("{occurrence} shows no edge in this view"),
                        };
                    }
                }
            }
            let parent_xf = out.view(id).and_then(|v| v.geometry_parent()).and_then(|p| contexts.get(&p)).map(|c| c.xf);
            if let Some(view) = out.view_mut(id) {
                view.geometry = ctx.projected.geometry.clone();
                if let Some(xf) = parent_xf {
                    project::reproject_parent_marks(&mut view.kind, &xf);
                }
            }
        }
    }
    // A BOM taken off the sheet comes back; balloons go on the first view when none is left.
    if !out.sheets.iter().any(|s| s.tables.iter().any(|t| t.kind == TableKind::Bom))
        && let Some(sheet) = out.sheets.first_mut()
    {
        let t = crate::library::template(sheet.template.as_str()).template();
        let [_, _, x1, y1] = drawing_frame(&t);
        let (w, _) = bom_table_size(model.bom.len());
        bom_fits(&t, Point2::new(x1 - w, y1), model.bom.len())?;
        sheet.tables.push(Table {
            kind: TableKind::Bom,
            position: Point2::new(x1 - w, y1),
            revisions: Vec::new(),
            bom,
            hole_view: None,
            holes: Vec::new(),
        });
    }
    let first = out
        .sheets
        .first()
        .and_then(|s| s.views.iter().find(|v| orientation(v).is_some() && included.contains(&v.id)))
        .map(|v| v.id);
    if let Some(view) = ballooned.or(first) {
        add_balloons(&mut out, model, view)?;
    }
    Ok(out)
}

/// A drawing whose model is an assembly.
pub fn is_assembly_drawing(drawing: &Drawing) -> bool {
    drawing.model.kind == crate::DocKind::Assembly
}
