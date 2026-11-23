//! Drawings of assemblies: views of every placed part, a bill of materials and balloons
//! that number the parts.
//!
//! The caller hands over the assembly as an [`AssemblyModel`]: each part occurrence's
//! display mesh and edge polylines already placed in the assembly's frame (exploded, when
//! the drawing should show the exploded view), each with the BOM item it belongs to, and
//! the BOM rows. Nothing here reads an assembly file or a kernel, so drawings of
//! assemblies work wherever the assembly's display copies do.
//!
//! A view's line work comes from hidden-line removal over all the parts' triangles
//! together, so one part hides another: each part's edges and the silhouettes of its
//! curved faces, split where any triangle in front covers them. The line work is as
//! faithful as the display meshes, so every curve is labelled a polygon. Each curve
//! remembers the occurrence it belongs to (its source's feature is the occurrence path),
//! which is what a balloon attaches to.
//!
//! The BOM lists parts only: one row per part document, in the order the tree first
//! places it, with how many times it is placed. Hidden parts count; suppressed parts do
//! not. A balloon carries its row's item number, and points at a visible edge of one
//! occurrence of that part.

use std::collections::{BTreeMap, HashMap};

use kurbo::BezPath;
use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::drawing::mesh_hlr::{SourcedPolyline, hidden_lines, silhouettes};
use soliddraft_kernel_api::drawing::{Curve3, CurveSource, StandardView, ViewSpec};
use soliddraft_kernel_api::{EdgeId, FaceId, Mesh, Point2, Vec3};

use crate::annotation::{Anchor, Annotation, AnnotationKind, AnnotationState, Attach, StyleOverrides};
use crate::entity::{EntityKind, EntityRef};
use crate::ids::{AnnotationId, ViewId};
use crate::part::round;
use crate::project::{self, ViewTransform};
use crate::sheet::{Scale, Table, TableKind};
use crate::template::Template;
use crate::view::{Accuracy, Curve2, CurveKind, Orientation, View, ViewDisplay, ViewGeometry, ViewKind, ViewScale};
use crate::{DocRef, Drawing, DrawingError, DrawingMeta};

/// One part occurrence of an assembly, placed.
#[derive(Clone, Debug, Default)]
pub struct PlacedPart {
    /// Its occurrence path in the assembly, such as "4/2".
    pub path: String,
    /// The BOM item it is counted under.
    pub item: u32,
    /// Vertex positions in the assembly's frame, millimetres.
    pub positions: Vec<Vec3>,
    /// Triangles, counter-clockwise seen from outside.
    pub triangles: Vec<[u32; 3]>,
    /// The face of each triangle, numbered within the part.
    pub faces: Vec<u32>,
    /// The part's edges as polylines in the assembly's frame.
    pub edges: Vec<Vec<Vec3>>,
}

/// One row of a bill of materials.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BomRow {
    /// The item number, from 1.
    pub item: u32,
    /// The part's name.
    pub name: String,
    /// The part's file, as the assembly references it.
    pub file: String,
    /// How many times the assembly places it.
    pub quantity: u32,
}

/// An assembly as a drawing reads it.
#[derive(Clone, Debug, Default)]
pub struct AssemblyModel {
    /// The assembly, pinned at the revision the parts come from.
    pub reference: Option<DocRef>,
    /// Every part occurrence that is shown, placed.
    pub parts: Vec<PlacedPart>,
    /// The same parts where the assembly's exploded view shows them; empty when it has
    /// none.
    pub exploded: Vec<PlacedPart>,
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

/// The id of the edge or face `local` of the part numbered `part` in the merged mesh.
fn packed(part: usize, local: usize) -> u64 {
    ((part as u64) << 32) | local as u64
}

fn unpacked(raw: u64) -> (usize, usize) {
    ((raw >> 32) as usize, (raw & 0xffff_ffff) as usize)
}

/// Every part's triangles in one mesh, and every part's edges as lines to split, with the
/// accuracy hidden-line removal works to.
fn merged(parts: &[PlacedPart], spec: &ViewSpec) -> Result<(Mesh, Vec<SourcedPolyline>, f64), DrawingError> {
    let frame =
        spec.frame().ok_or_else(|| DrawingError::BadRequest("the view direction is zero or parallel to up".into()))?;
    let mut mesh = Mesh::default();
    let mut lines = Vec::new();
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for (k, p) in parts.iter().enumerate() {
        let base = mesh.positions.len() as u32;
        for v in &p.positions {
            mesh.positions.push([v.x as f32, v.y as f32, v.z as f32]);
            mesh.normals.push([0.0, 0.0, 1.0]);
            for (k, c) in [v.x, v.y, v.z].into_iter().enumerate() {
                lo[k] = lo[k].min(c);
                hi[k] = hi[k].max(c);
            }
        }
        for (t, tri) in p.triangles.iter().enumerate() {
            mesh.indices.extend(tri.map(|i| i + base));
            let face = p.faces.get(t).copied().unwrap_or(0) as usize;
            mesh.triangle_faces.push(FaceId::from_raw(packed(k, face)));
        }
        for (e, points) in p.edges.iter().enumerate() {
            if points.len() >= 2 {
                lines.push(SourcedPolyline {
                    source: CurveSource::Edge(EdgeId::from_raw(packed(k, e))),
                    points: points.clone(),
                });
            }
        }
    }
    if mesh.indices.is_empty() {
        return Err(DrawingError::BadRequest("the assembly shows no parts to draw".into()));
    }
    lines.extend(silhouettes(&mesh, &frame));
    // Display meshes follow their parts to about a ten-thousandth of their size.
    let tol = (Vec3::from(hi).distance(Vec3::from(lo)) * 1e-4).max(0.005);
    Ok((mesh, lines, tol))
}

impl AssemblyModel {
    /// The parts as a view shows them: exploded when it asks and the assembly has an
    /// exploded view.
    pub fn shown(&self, exploded: bool) -> &[PlacedPart] {
        if exploded && !self.exploded.is_empty() { &self.exploded } else { &self.parts }
    }
}

/// The line work of a view of the assembly at `scale`, and its transform.
pub fn assembly_view(
    model: &AssemblyModel,
    spec: &ViewSpec,
    scale: f64,
    exploded: bool,
) -> Result<(ViewGeometry, ViewTransform), DrawingError> {
    let parts = model.shown(exploded);
    let (mesh, lines, tol) = merged(parts, spec)?;
    let frame = spec.frame().expect("checked in merged");
    let curves = hidden_lines(&mesh, &lines, &frame, tol);
    let xf = project::transform(&curves, spec, scale)?;
    let mut out = Vec::with_capacity(curves.curves.len());
    let mut counts: HashMap<(usize, usize), u32> = HashMap::new();
    for c in &curves.curves {
        let path = project::path_of(&c.curve, &xf);
        if path.elements().len() < 2 {
            continue;
        }
        let (kind, ek, raw) = match c.source {
            CurveSource::Edge(e) => (CurveKind::Edge, EntityKind::Edge, e.raw()),
            CurveSource::Silhouette(f) => (CurveKind::Silhouette, EntityKind::Face, f.raw()),
            _ => continue,
        };
        let (part, local) = unpacked(raw);
        let Some(p) = parts.get(part) else { continue };
        let ordinal = counts.entry((part, local)).or_default();
        *ordinal += 1;
        let (point, size) = midpoint(&c.curve);
        let role = if kind == CurveKind::Edge { "assembly_edge" } else { "assembly_silhouette" };
        let mut source = EntityRef::new(ek, &p.path, role, local as u32, point, size);
        source.kernel_hint = Some(raw);
        out.push(Curve2 { path, source, visible: c.visible, kind, tangent: false, accuracy: Accuracy::Polygon });
    }
    let [x0, y0, x1, y1] = curves.bbox_2d;
    let bbox = [
        round(x0 * xf.scale - xf.centre.u),
        round(y0 * xf.scale - xf.centre.v),
        round(x1 * xf.scale - xf.centre.u),
        round(y1 * xf.scale - xf.centre.v),
    ];
    let digest = model.reference.as_ref().map(|r| r.pinned.as_str().to_owned()).unwrap_or_default();
    let geometry = ViewGeometry {
        source_digest: digest,
        curves: out,
        hatch: Vec::new(),
        outline: None,
        bbox,
        unproved: false,
        // Display meshes, not Catallax's drawing queries: exports refuse it.
        catallax: false,
        coarse: false,
    };
    Ok((geometry, xf))
}

/// The point halfway along a curve, and its length.
fn midpoint(curve: &Curve3) -> (Vec3, f64) {
    let len = curve.length();
    match curve {
        Curve3::Polyline { points } if points.len() >= 2 => {
            let mut left = len / 2.0;
            for w in points.windows(2) {
                let d = w[0].distance(w[1]);
                if d >= left && d > 0.0 {
                    return (w[0] + (w[1] - w[0]) * (left / d), len);
                }
                left -= d;
            }
            (*points.last().expect("two at least"), len)
        }
        c => (c.point_at(0.5), len),
    }
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
pub fn make_assembly_drawing(model: &AssemblyModel, new: &NewAssemblyDrawing) -> Result<Drawing, DrawingError> {
    let reference =
        model.reference.clone().ok_or_else(|| DrawingError::BadRequest("the assembly has no reference".into()))?;
    if new.views.is_empty() {
        return Err(DrawingError::BadRequest("no views asked for".into()));
    }
    let mut drawing = Drawing::new(reference, &new.template);
    drawing.meta = new.meta.clone();
    let t = &new.template;
    let band = crate::library::library().templates.iter().find(|f| f.id == t.id.as_str()).map_or(0.0, |f| f.zones.band);
    let (x0, y0) = (t.margins.left + band, t.margins.bottom + band);
    let (x1, y1) = (t.size.width_mm - t.margins.right - band, t.size.height_mm - t.margins.top - band);
    let (bom_w, bom_h) = bom_table_size(model.bom.len());
    // The BOM in the top-right corner; the views left of it, above the title block.
    drawing.sheets[0].tables.push(Table {
        kind: TableKind::Bom,
        position: Point2::new(x1 - bom_w, y1),
        revisions: Vec::new(),
        bom: model.bom.clone(),
    });
    // The views use the drawing area below the BOM and above the title block.
    let title_top = t.title_block.position.v + t.title_block.height + MARGIN;
    let region = (x0 + MARGIN, title_top.max(y0 + MARGIN), x1 - MARGIN, y1 - bom_h - MARGIN);
    let slot_w = (region.2 - region.0) / new.views.len() as f64;
    let slot_h = region.3 - region.1;
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
    let scale = new.scale.unwrap_or_else(|| {
        scales
            .iter()
            .copied()
            .find(|s| fits(s.factor()))
            .unwrap_or_else(|| scales.last().copied().unwrap_or(Scale::FULL))
    });
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
        });
    }
    if new.balloons {
        add_balloons(&mut drawing, model, ViewId(1))?;
    }
    Ok(drawing)
}

/// Where a balloon for `item` attaches in a view: the middle of the longest visible curve
/// of the first occurrence of the item that shows one, in model space and view
/// millimetres, with that occurrence's path.
fn attach_point(
    model: &AssemblyModel,
    geometry: &ViewGeometry,
    xf: &ViewTransform,
    item: u32,
    occurrence: Option<&str>,
) -> Option<(String, Vec3, Point2)> {
    let wanted: Vec<&str> = match occurrence {
        Some(p) => vec![p],
        None => model.parts.iter().filter(|p| p.item == item).map(|p| p.path.as_str()).collect(),
    };
    for path in wanted {
        let Some(best) = geometry
            .curves
            .iter()
            .filter(|c| c.visible && c.source.feature == path)
            .max_by(|a, b| a.source.fingerprint.size.total_cmp(&b.source.fingerprint.size))
        else {
            continue;
        };
        let at = best.source.fingerprint.point;
        return Some((path.to_owned(), at, xf.point(at)));
    }
    None
}

/// Puts a balloon for every BOM row that has none on `view`, outside the view's box, on
/// the line from the view's middle through its anchor.
fn add_balloons(drawing: &mut Drawing, model: &AssemblyModel, view: ViewId) -> Result<(), DrawingError> {
    let sheet = crate::project::sheet_of(drawing, view).ok_or(DrawingError::UnknownView(view))?;
    let v = drawing.view(view).ok_or(DrawingError::UnknownView(view))?.clone();
    let spec = ViewSpec::standard(orientation(&v).unwrap_or(StandardView::Isometric));
    let scale = project::scale_in(drawing, sheet, &v);
    let (geometry, xf) = assembly_view(model, &spec, scale, v.exploded)?;
    let [bx0, by0, bx1, by1] = geometry.bbox;
    let half = Point2::new((bx1 - bx0) / 2.0, (by1 - by0) / 2.0);
    let taken: Vec<u32> = drawing.sheets[sheet]
        .annotations
        .iter()
        .filter(|a| a.view == view)
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
        let Some((path, at, cached)) = attach_point(model, &geometry, &xf, row.item, None) else { continue };
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
        let mut entity = EntityRef::new(EntityKind::Edge, &path, "assembly_edge", 0, at, 0.0);
        entity.kernel_hint = None;
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

/// The drawing brought to `model`: every base view's line work at its scale, the BOM
/// rows, each balloon moved onto its occurrence (dangling when the occurrence is gone or
/// hidden in the view, its number from the BOM row its occurrence is now counted under),
/// and a balloon on the first ballooned view for every new row. Pins the new revision.
pub fn update_assembly_drawing(drawing: &Drawing, model: &AssemblyModel) -> Result<Drawing, DrawingError> {
    let mut out = drawing.clone();
    if let Some(r) = &model.reference {
        out.model.pinned = r.pinned.clone();
    }
    let item_of: BTreeMap<&str, u32> = model.parts.iter().map(|p| (p.path.as_str(), p.item)).collect();
    let mut ballooned: Option<ViewId> = None;
    for si in 0..out.sheets.len() {
        for t in out.sheets[si].tables.iter_mut().filter(|t| t.kind == TableKind::Bom) {
            t.bom = model.bom.clone();
        }
        let views = out.sheets[si].views.clone();
        for v in views {
            let Some(o) = orientation(&v) else { continue };
            if v.excluded_from_update {
                continue;
            }
            let scale = project::scale_in(&out, si, &v);
            let (geometry, xf) = assembly_view(model, &ViewSpec::standard(o), scale, v.exploded)?;
            for a in out.sheets[si].annotations.iter_mut().filter(|a| a.view == v.id) {
                let AnnotationKind::Balloon { item, occurrence } = &mut a.body else { continue };
                ballooned.get_or_insert(v.id);
                let Some(now) = item_of.get(occurrence.as_str()) else {
                    a.state = AnnotationState::Dangling {
                        lost: vec![0],
                        cause: format!("{occurrence} is no longer in the assembly"),
                    };
                    continue;
                };
                *item = *now;
                match attach_point(model, &geometry, &xf, *now, Some(occurrence)) {
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
            if let Some(view) = out.view_mut(v.id) {
                view.geometry = geometry;
            }
        }
    }
    // A BOM taken off the sheet comes back; balloons go on the first view when none is left.
    if !out.sheets.iter().any(|s| s.tables.iter().any(|t| t.kind == TableKind::Bom))
        && let Some(sheet) = out.sheets.first_mut()
    {
        let t = crate::library::template(sheet.template.as_str()).template();
        let band =
            crate::library::library().templates.iter().find(|f| f.id == t.id.as_str()).map_or(0.0, |f| f.zones.band);
        let (w, _) = bom_table_size(model.bom.len());
        let (x1, y1) = (t.size.width_mm - t.margins.right - band, t.size.height_mm - t.margins.top - band);
        sheet.tables.push(Table {
            kind: TableKind::Bom,
            position: Point2::new(x1 - w, y1),
            revisions: Vec::new(),
            bom: model.bom.clone(),
        });
    }
    let first = out.sheets.first().and_then(|s| s.views.iter().find(|v| orientation(v).is_some())).map(|v| v.id);
    if let Some(view) = ballooned.or(first) {
        add_balloons(&mut out, model, view)?;
    }
    Ok(out)
}

/// Why an export refuses a view of an assembly: exports draw Catallax line work, and
/// Catallax does not yet draw several placed parts in one view.
pub const NOT_EXPORTED: &str = "is drawn from the assembly's display meshes; exports draw Catallax line work, \
                                and Catallax does not draw placed parts together yet";

/// A drawing whose model is an assembly.
pub fn is_assembly_drawing(drawing: &Drawing) -> bool {
    drawing.model.kind == crate::DocKind::Assembly
}

/// The path of a balloon's circle, for the canvas and exports: the circle at `centre`.
pub fn balloon_circle(centre: Point2, radius: f64) -> BezPath {
    let c = kurbo::Circle::new((centre.u, centre.v), radius);
    kurbo::Shape::to_path(&c, 0.01)
}
