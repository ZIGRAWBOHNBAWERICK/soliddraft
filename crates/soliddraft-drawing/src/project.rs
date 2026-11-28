//! Projection: from a view's place in the drawing to the kernel's view request, and from
//! the kernel's 3D curves to the view's 2D line work.
//!
//! A base view's orientation is stored; a projected view's comes from its parent's,
//! folded towards the side it sits on, with its placement projection angle deciding which
//! face that side shows. A section view turns its parent's frame a quarter turn about
//! the cutting line, and an auxiliary view about its reference edge; a detail view keeps
//! its parent's frame and magnifies it. Scales chain the same way. Curves are projected
//! with the kernel's [`ViewFrame`], scaled, and moved so the centre of their bounding box
//! is the view's origin (for a detail, the centre of its boundary). Lines stay lines and
//! circular arcs become cubic Bézier approximations before projection.
//!
//! [`compute_view`] makes any view's line work, parents first; crops and broken-out
//! sections are applied last, by clipping the line work with
//! [`clip_path`].

use std::collections::BTreeMap;

use kurbo::{BezPath, ParamCurve, PathSeg, Point};
use soliddraft_kernel_api::drawing::{
    Curve3, CurveSource, CutRegion, SectionSpec, SectionView, ViewCurve, ViewCurves, ViewFrame, ViewSpec,
};
use soliddraft_kernel_api::{KernelError, Point2, Vec3};

use soliddraft_kernel_api::drawing::StandardView;

use crate::annotation::{Anchor, Annotation, AnnotationKind, AnnotationState, Attach, StyleOverrides};
use crate::entity::{EntityKind, EntityRef};
use crate::ids::{AnnotationId, ViewId};
use crate::part::{Entity, Part, PartIndex, Resolution, round};
use crate::section_path::{Band, BentSection, CutPath, Turn};
use crate::sheet::{Projection, Sheet};
use crate::view::{
    Accuracy, AlignAxis, Boundary, Break, BreakGap, BrokenOut, Curve2, CurveKind, FoldDirection, HalfSection, Polygon,
    View, ViewDisplay, ViewGeometry, ViewKind, ViewScale,
};
use crate::{Drawing, DrawingError};

/// Model space to view millimetres: project with `frame`, scale, close up the view's
/// breaks, then move `centre` (in projected, scaled and closed-up millimetres) to the
/// origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewTransform {
    /// The kernel's view frame.
    pub frame: ViewFrame,
    /// Sheet millimetres per model millimetre.
    pub scale: f64,
    /// The bounding box centre in projected, scaled millimetres.
    pub centre: Point2,
    /// The stretches a broken view leaves out; none for most views.
    pub breaks: Breaks,
    aligned_turn: Option<Turn>,
}

/// The breaks of a view as its transform applies them: stretches of the part, in model
/// millimetres along the view's right or up direction, that the view leaves out, each
/// closed up to a gap of a few sheet millimetres. Points beyond a stretch move back by
/// what it leaves out; the scale stays the view's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Breaks {
    /// True when the breaks shorten the view's height, false its width.
    vertical: bool,
    stretch: Option<(f64, f64)>,
    centre_fraction: f64,
}

impl Breaks {
    /// Resolves a saved view interruption into the view frame.
    pub fn new(breaks: &[Break], frame: &ViewFrame) -> Result<Breaks, DrawingError> {
        let b = match breaks {
            [] => return Ok(Breaks::default()),
            [b] => b,
            _ => return Err(DrawingError::BadRequest("A view takes one break".into())),
        };
        let vertical = b.axis == AlignAxis::Vertical;
        let axis = if vertical { frame.up } else { frame.right };
        let (a, c) = (b.from.dot(axis), b.to.dot(axis));
        if !a.is_finite() || !c.is_finite() || (a - c).abs() <= 1e-9 {
            return Err(DrawingError::BadRequest("A break needs two distinct positions along its axis".into()));
        }
        Ok(Breaks { vertical, stretch: Some((a.min(c), a.max(c))), centre_fraction: 0.5 })
    }

    /// Whether the view has no interruption.
    pub fn is_empty(&self) -> bool {
        self.stretch.is_none()
    }

    /// Whether a station along the break axis lies inside the removed stretch.
    pub fn hides(&self, s: f64) -> bool {
        self.stretch.is_some_and(|(lo, hi)| s > lo && s < hi)
    }

    /// The model direction along which the view is shortened.
    pub fn axis(&self, frame: &ViewFrame) -> Vec3 {
        if self.vertical { frame.up } else { frame.right }
    }

    fn along(&self, p: Point2) -> f64 {
        if self.vertical { p.v } else { p.u }
    }

    fn with_along(&self, p: Point2, value: f64) -> Point2 {
        if self.vertical { Point2::new(p.u, value) } else { Point2::new(value, p.v) }
    }

    fn fraction(&self, s: f64) -> f64 {
        self.stretch.map_or(0.0, |(lo, hi)| ((s - lo) / (hi - lo)).clamp(0.0, 1.0))
    }

    fn shift(&self, s: f64, scale: f64) -> f64 {
        self.stretch.map_or(0.0, |(lo, hi)| self.fraction(s) * ((hi - lo) * scale - Break::GAP))
    }

    fn unshift(&self, b: f64, scale: f64) -> f64 {
        let Some((lo, hi)) = self.stretch else { return b };
        let start = lo * scale;
        let fraction = ((b - start) / Break::GAP).clamp(0.0, 1.0);
        b + fraction * ((hi - lo) * scale - Break::GAP)
    }

    fn kept(&self, scale: f64) -> [(f64, f64, f64); 2] {
        let (lo, hi) = self.stretch.expect("a broken view");
        [(f64::NEG_INFINITY, lo, 0.0), (hi, f64::INFINITY, self.shift(hi, scale))]
    }
}

impl ViewTransform {
    pub(crate) fn leg_turn(&self, leg: u8) -> Option<Turn> {
        if leg == 1 { self.aligned_turn } else { None }
    }

    /// A model curve where the view draws it from aligned section leg `leg`: the second
    /// leg's curves turned into the first leg's plane, others as they are.
    pub fn curve_on_leg(&self, leg: u8, curve: &Curve3) -> Curve3 {
        self.leg_turn(leg).map_or_else(|| curve.clone(), |turn| turn.curve(curve))
    }

    /// A model point where the view draws it from aligned section leg `leg`.
    pub fn point_on_leg(&self, leg: u8, p: Vec3) -> Vec3 {
        self.leg_turn(leg).map_or(p, |turn| turn.point(p))
    }

    /// A transform with no breaks.
    pub fn new(frame: ViewFrame, scale: f64, centre: Point2) -> ViewTransform {
        ViewTransform { frame, scale, centre, breaks: Breaks::default(), aligned_turn: None }
    }

    /// The same transform without its breaks: what a broken view's first kept stretch
    /// shows, with the view's centre.
    pub fn unbroken(&self) -> ViewTransform {
        ViewTransform { breaks: Breaks::default(), ..*self }
    }

    /// The transform with `breaks`, centred again on what a view of the curves with
    /// sheet bounds `bbox_2d` (see [`ViewCurves::bbox_2d`]) keeps.
    pub fn with_breaks(self, mut breaks: Breaks, bbox_2d: [f64; 4]) -> Result<ViewTransform, DrawingError> {
        let Some((start, end)) = breaks.stretch else { return Ok(self) };
        let (lo, hi) = if breaks.vertical { (bbox_2d[1], bbox_2d[3]) } else { (bbox_2d[0], bbox_2d[2]) };
        let s = self.scale;
        if start <= lo || end >= hi || (end - start) * s <= Break::GAP {
            return Err(invalid_break());
        }
        let middle = ((lo * s - breaks.shift(lo, s)) + (hi * s - breaks.shift(hi, s))) / 2.0;
        breaks.centre_fraction = (breaks.fraction(lo) + breaks.fraction(hi)) / 2.0;
        Ok(ViewTransform { centre: breaks.with_along(self.centre, middle), breaks, ..self })
    }

    /// A model point projected and scaled, not yet closed up or centred.
    pub fn raw(&self, p: Vec3) -> Point {
        let q = self.frame.project(p);
        Point::new(q.u * self.scale, q.v * self.scale)
    }

    /// View millimetres of a model point.
    pub fn point(&self, p: Vec3) -> Point2 {
        let q = self.precise_point(p);
        Point2::new(round(q.u), round(q.v))
    }

    pub(crate) fn precise_point(&self, p: Vec3) -> Point2 {
        let q = self.frame.project(p);
        let mut q = Point2::new(q.u * self.scale, q.v * self.scale);
        if !self.breaks.is_empty() {
            let s = p.dot(self.breaks.axis(&self.frame));
            q = self.breaks.with_along(q, self.breaks.along(q) - self.breaks.shift(s, self.scale));
        }
        Point2::new(q.u - self.centre.u, q.v - self.centre.v)
    }

    /// View millimetres of a box in the frame's sheet coordinates (model millimetres),
    /// `[min x, min y, max x, max y]`.
    pub fn bounds(&self, bbox_2d: [f64; 4]) -> [f64; 4] {
        let [x0, y0, x1, y1] = bbox_2d;
        let s = self.scale;
        let shift = |v: f64, vertical: bool| {
            if !self.breaks.is_empty() && self.breaks.vertical == vertical { self.breaks.shift(v, s) } else { 0.0 }
        };
        [
            round(x0 * s - shift(x0, false) - self.centre.u),
            round(y0 * s - shift(y0, true) - self.centre.v),
            round(x1 * s - shift(x1, false) - self.centre.u),
            round(y1 * s - shift(y1, true) - self.centre.v),
        ]
    }

    /// Where a broken view's gaps lie, in view millimetres.
    pub fn gaps(&self) -> Vec<BreakGap> {
        let Some((lo, hi)) = self.breaks.stretch else { return Vec::new() };
        let axis = if self.breaks.vertical { AlignAxis::Vertical } else { AlignAxis::Horizontal };
        let at = lo * self.scale - self.breaks.along(self.centre);
        vec![BreakGap {
            axis,
            at,
            gap: Break::GAP,
            centre_fraction: self.breaks.centre_fraction,
            omitted_mm: Some((hi - lo) * self.scale),
        }]
    }

    /// Changes scale while keeping the break gap in paper millimetres.
    pub fn rescaled(self, scale: f64) -> Self {
        let ratio = scale / self.scale;
        let mut centre = Point2::new(self.centre.u * ratio, self.centre.v * ratio);
        if !self.breaks.is_empty() {
            let correction = (1.0 - ratio) * Break::GAP * self.breaks.centre_fraction;
            centre = self.breaks.with_along(centre, self.breaks.along(centre) + correction);
        }
        Self { scale, centre, ..self }
    }

    /// A model direction in the view, scaled.
    pub fn vector(&self, d: Vec3) -> Point2 {
        Point2::new(d.dot(self.frame.right) * self.scale, d.dot(self.frame.up) * self.scale)
    }

    /// The model point in the plane through the model origin facing the eye that a view
    /// point shows. Any point along the view direction from it shows at the same place.
    pub fn unproject(&self, p: Point2) -> Vec3 {
        let mut q = Point2::new(p.u + self.centre.u, p.v + self.centre.v);
        if !self.breaks.is_empty() {
            q = self.breaks.with_along(q, self.breaks.unshift(self.breaks.along(q), self.scale));
        }
        self.frame.right * (q.u / self.scale) + self.frame.up * (q.v / self.scale)
    }

    fn kpoint(&self, p: Vec3) -> Point {
        let q = self.point(p);
        Point::new(q.u, q.v)
    }
}

/// A view with what was computed for it: the kernel's curves and the transform into the
/// view's millimetres. Model items and the update work on these.
#[derive(Clone, Debug)]
pub struct ViewContext {
    /// The view.
    pub id: ViewId,
    /// The kernel's curves, with kernel ids. For a section, the cut body's.
    pub curves: ViewCurves,
    /// Model space to view millimetres.
    pub xf: ViewTransform,
    /// True for a pictorial view: one that looks along no model axis.
    pub pictorial: bool,
    /// The curves projected into the view.
    pub projected: Projected,
}

/// Views computed so far, by id: the parents derived views are made from.
pub type Contexts = BTreeMap<ViewId, ViewContext>;

impl ViewContext {
    /// Computes a view's curves and transform at its current scale, with its parents.
    pub fn compute(drawing: &Drawing, part: &Part, index: &PartIndex, id: ViewId) -> Result<ViewContext, DrawingError> {
        let mut memo = Contexts::new();
        compute_in(drawing, part, index, id, &mut memo)?;
        Ok(memo.remove(&id).expect("just computed"))
    }

    /// The kernel request the view was made with.
    pub fn spec(&self) -> ViewSpec {
        let f = self.xf.frame;
        ViewSpec::new(f.direction, f.up)
    }
}

/// Computes view `id` of the drawing into `memo`, its parents first, unless it is there.
pub fn compute_in(
    drawing: &Drawing,
    part: &Part,
    index: &PartIndex,
    id: ViewId,
    memo: &mut Contexts,
) -> Result<(), DrawingError> {
    compute_with_previous(drawing, part, index, id, memo, None)
}

pub(crate) fn compute_with_previous(
    drawing: &Drawing,
    part: &Part,
    index: &PartIndex,
    id: ViewId,
    memo: &mut Contexts,
    previous: Option<&Contexts>,
) -> Result<(), DrawingError> {
    if memo.contains_key(&id) {
        return Ok(());
    }
    let sheet = sheet_of(drawing, id).ok_or(DrawingError::UnknownView(id))?;
    let view = drawing.view(id).ok_or(DrawingError::UnknownView(id))?;
    let ctx = compute_view_at(drawing, part, index, view, sheet, memo, 0, None, previous)?;
    memo.insert(id, ctx);
    Ok(())
}

/// Computes the line work of `view` as if it were on sheet `sheet`, at its scale. The
/// view need not be on the drawing yet, so a tool can preview it; its parent must be.
/// Parents go into `memo`.
pub fn compute_view(
    drawing: &Drawing,
    part: &Part,
    index: &PartIndex,
    view: &View,
    sheet: usize,
    memo: &mut Contexts,
) -> Result<ViewContext, DrawingError> {
    compute_view_at(drawing, part, index, view, sheet, memo, 0, None, None)
}

pub(crate) fn reproject_parent_marks(kind: &mut ViewKind, xf: &ViewTransform) {
    match kind {
        ViewKind::Section { line, model_line, .. } if model_line.len() == line.len() => {
            for (p, m) in line.iter_mut().zip(model_line.iter()) {
                *p = xf.point(*m);
            }
        }
        ViewKind::Detail { center, profile, model_center: Some(m), .. } => {
            let to = xf.point(*m);
            let (du, dv) = (to.u - center.u, to.v - center.v);
            *center = to;
            for p in profile {
                *p = Point2::new(round(p.u + du), round(p.v + dv));
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_view_at(
    drawing: &Drawing,
    part: &Part,
    index: &PartIndex,
    view: &View,
    sheet: usize,
    memo: &mut Contexts,
    depth: u8,
    projection: Option<(ViewDisplay, ViewTransform)>,
    previous: Option<&Contexts>,
) -> Result<ViewContext, DrawingError> {
    if depth > 16 {
        return Err(DrawingError::BadView(view.id));
    }
    if let Some(parent) = view.geometry_parent()
        && !memo.contains_key(&parent)
    {
        let p = drawing.view(parent).ok_or(DrawingError::UnknownView(parent))?;
        let ps = sheet_of(drawing, parent).unwrap_or(sheet);
        let ctx = compute_view_at(drawing, part, index, p, ps, memo, depth + 1, None, previous)?;
        memo.insert(parent, ctx);
    }
    let display = projection.map_or(view.display, |(display, _)| display);
    let scale = scale_in(drawing, sheet, view);
    let refused = |error| view_refusal(|| view_label_in(drawing, &drawing.sheets[sheet], view), error);
    let mut kind = view.kind.clone();
    if let Some(parent) = view.geometry_parent() {
        let after = &memo[&parent].xf;
        if let Some(before) = previous.and_then(|views| views.get(&parent)) {
            remap_parent_boundary(&mut kind, &before.xf, after);
        }
        reproject_parent_marks(&mut kind, after);
    }
    let mut ctx = match &kind {
        ViewKind::Base { .. } | ViewKind::Projected { .. } => {
            let spec = with_display(spec_in(drawing, sheet, view).ok_or(DrawingError::BadView(view.id))?, &display);
            let curves = view_curves(part, &spec).map_err(refused)?;
            let xf = match projection {
                Some((_, xf)) => xf,
                None => view_transform(&curves, &spec, scale, view)?,
            };
            let projected = project(index, &curves, &xf, part.digest.as_str(), part.preview);
            ViewContext { id: view.id, curves, xf, pictorial: is_pictorial(&spec), projected }
        }
        ViewKind::Section { parent, line, flip, model_line, .. } if line.len() > 2 => {
            let pctx = &memo[parent];
            // The cut stays where the model points put it; the parent's line is drawn from
            // them, rounded.
            let model: Vec<Vec3> = if model_line.len() == line.len() {
                model_line.clone()
            } else {
                line.iter().map(|p| pctx.xf.unproject(*p)).collect()
            };
            let bent = BentSection::new(&pctx.xf.frame, &model, *flip).map_err(|e| {
                DrawingError::BadRequest(format!("{}: {e}", view_label_in(drawing, &drawing.sheets[sheet], view)))
            })?;
            let (direction, up) = bent.axes();
            let spec = with_display(ViewSpec::new(direction, up), &display);
            let mut display_only = false;
            let unfolded = bent
                .unfold(|direction, up, origin| {
                    section_or_mesh(
                        part,
                        &with_display(ViewSpec::new(direction, up), &display),
                        origin,
                        Some(&mut display_only),
                    )
                })
                .map_err(|e| refused(e.into()))?;
            let xf = match projection {
                Some((_, xf)) => xf,
                None => view_transform(&unfolded.curves, &spec, scale, view)?,
            };
            let xf = ViewTransform { aligned_turn: bent.aligned_turn(), ..xf };
            let mut projected = project_curves(
                index,
                &unfolded.curves,
                &xf,
                part.digest.as_str(),
                part.preview,
                Some(unfolded.cuts[0].curves.len()),
            );
            projected.geometry.display_only = display_only;
            // Each cut hatches only its own stretch: the jogs and the pivot draw no line.
            let flat = xf.unbroken();
            projected.geometry.hatch = unfolded
                .cuts
                .iter()
                .flat_map(|cut| {
                    let window = cut.band.window(|p| flat.point(p), |d| flat.vector(d));
                    cut.regions.iter().flat_map(move |r| {
                        let polygon = region_polygon(r, &flat, None);
                        clip_polygon_region(polygon, &window, true)
                            .into_iter()
                            .flat_map(move |polygon| split_polygon(polygon, &xf))
                    })
                })
                .filter(|h| h.outer.len() >= 3)
                .collect();
            ViewContext { id: view.id, curves: unfolded.curves, xf, pictorial: false, projected }
        }
        ViewKind::Section { parent, line, flip, model_line, .. } => {
            let pctx = &memo[parent];
            let (direction, up) =
                section_model_axes(&pctx.xf.frame, line, model_line, *flip).ok_or(DrawingError::BadView(view.id))?;
            let spec = with_display(ViewSpec::new(direction, up), &display);
            let origin = model_line.first().copied().unwrap_or_else(|| pctx.xf.unproject(line[0]));
            let mut display_only = false;
            let section =
                section_or_mesh(part, &spec, origin, Some(&mut display_only)).map_err(|e| refused(e.into()))?;
            let xf = match projection {
                Some((_, xf)) => xf,
                None => view_transform(&section.curves, &spec, scale, view)?,
            };
            let mut projected = project(index, &section.curves, &xf, part.digest.as_str(), part.preview);
            projected.geometry.display_only = display_only;
            projected.geometry.hatch = section.regions.iter().flat_map(|r| region_polygons(r, &xf, None)).collect();
            ViewContext { id: view.id, curves: section.curves, xf, pictorial: false, projected }
        }
        ViewKind::Detail { parent, center, radius, profile, model_center, .. } => {
            let original = drawing.view(*parent).ok_or(DrawingError::UnknownView(*parent))?;
            let includes_tangents = |display: ViewDisplay| display.tangent != crate::TangentEdges::Removed;
            let parent_xf = memo[parent].xf;
            let alternate = if includes_tangents(display) != includes_tangents(original.display)
                || display.seams != original.display.seams
            {
                Some(compute_view_at(
                    drawing,
                    part,
                    index,
                    original,
                    sheet_of(drawing, *parent).unwrap_or(sheet),
                    memo,
                    depth + 1,
                    Some((display, parent_xf)),
                    previous,
                )?)
            } else {
                None
            };
            let pctx = alternate.as_ref().unwrap_or(&memo[parent]);
            let ratio = scale / pctx.xf.scale;
            let model = model_center.unwrap_or_else(|| pctx.xf.unproject(*center));
            let mut xf = pctx.xf.rescaled(scale);
            let shift = xf.precise_point(model);
            xf.centre = Point2::new(xf.centre.u + shift.u, xf.centre.v + shift.v);
            xf.breaks.centre_fraction = xf.breaks.fraction(model.dot(xf.breaks.axis(&xf.frame)));
            let gap = pctx.projected.geometry.breaks.first().copied();
            let map = |p: &Point2| {
                let q = scale_view_point(*p, ratio, gap);
                Point2::new(round(q.u - shift.u), round(q.v - shift.v))
            };
            let callout = if profile.len() >= 3 {
                Boundary::Polygon { points: profile.clone() }
            } else {
                Boundary::Circle { center: *center, radius: *radius }
            };
            let boundary = scaled_boundary(&callout, ratio, gap).mapped(-shift.u, -shift.v, 1.0);
            let mut parent = pctx.projected.clone();
            clip_projected(&mut parent, &callout, true);
            let curves = parent
                .geometry
                .curves
                .into_iter()
                .map(|mut c| {
                    c.path = scaled_path(&c.path, ratio, gap, shift);
                    c.ends = c.ends.map(|ends| ends.map(|p| map(&p)));
                    c
                })
                .collect();
            let hatch = parent
                .geometry
                .hatch
                .into_iter()
                .map(|mut h| {
                    h.outer.iter_mut().chain(h.holes.iter_mut().flatten()).for_each(|p| *p = map(p));
                    h.clip = h.clip.map(|clip| scaled_boundary(&clip, ratio, gap).mapped(-shift.u, -shift.v, 1.0));
                    h
                })
                .collect();
            let circles = parent
                .circles
                .into_iter()
                .map(|c| CircleInfo { centre: map(&c.centre), radius: c.radius * ratio, ..c })
                .collect();
            let geometry = ViewGeometry {
                source_digest: part.digest.to_string(),
                curves,
                hatch,
                outline: None,
                bbox: parent.geometry.bbox,
                unproved: parent.geometry.unproved,
                display_only: parent.geometry.display_only,
                catallax: parent.geometry.catallax,
                coarse: parent.geometry.coarse,
                breaks: Vec::new(),
                symmetry_segments: Vec::new(),
            };
            let mut projected = Projected { geometry, transform: xf, circles };
            projected.geometry.bbox = boundary.bounds().map(round);
            projected.geometry.outline = Some(boundary);
            ViewContext { id: view.id, curves: pctx.curves.clone(), xf, pictorial: pctx.pictorial, projected }
        }
        ViewKind::Auxiliary { parent, edge, flip, .. } => {
            let pctx = &memo[parent];
            let (direction, up) = auxiliary_axes(index, pctx, edge, *flip)?;
            let spec = with_display(ViewSpec::new(direction, up), &display);
            let curves = view_curves(part, &spec).map_err(refused)?;
            let xf = match projection {
                Some((_, xf)) => xf,
                None => view_transform(&curves, &spec, scale, view)?,
            };
            let projected = project(index, &curves, &xf, part.digest.as_str(), part.preview);
            ViewContext { id: view.id, curves, xf, pictorial: false, projected }
        }
    };
    if let Some(half) = &view.half {
        half_section(part, index, &mut ctx, half, &display).map_err(refused)?;
    }
    let before = previous.and_then(|views| views.get(&view.id)).map(|ctx| ctx.xf);
    let remap =
        |boundary: &Boundary| before.map_or_else(|| boundary.clone(), |old| remap_boundary(boundary, &old, &ctx.xf));
    let cuts: Vec<_> =
        view.broken_out.iter().map(|cut| BrokenOut { boundary: remap(&cut.boundary), depth: cut.depth }).collect();
    let crop = view.crop.as_ref().map(remap);
    for cut in &cuts {
        broken_out(part, index, &mut ctx, cut, &display).map_err(refused)?;
    }
    if let Some(crop) = &crop {
        clip_projected(&mut ctx.projected, crop, true);
        let mut bbox = curves_bounds(&ctx.projected.geometry.curves).unwrap_or(ctx.projected.geometry.bbox);
        for h in &ctx.projected.geometry.hatch {
            for p in &h.outer {
                bbox = [bbox[0].min(p.u), bbox[1].min(p.v), bbox[2].max(p.u), bbox[3].max(p.v)];
            }
        }
        let c = crop.bounds();
        ctx.projected.geometry.bbox =
            [bbox[0].max(c[0]), bbox[1].max(c[1]), bbox[2].min(c[2]), bbox[3].min(c[3])].map(round);
    }
    ctx.projected.geometry.breaks = ctx.xf.gaps();
    validate_break(&ctx.projected.geometry, display, 1.0, projection.is_none() && !view.breaks.is_empty())?;
    if let Some(half) = &view.half {
        ctx.projected.geometry.symmetry_segments = symmetry_line(&ctx, half);
    }
    Ok(ctx)
}

/// The transform that centres `curves` at `scale`, with the view's breaks closed up.
fn view_transform(
    curves: &ViewCurves,
    spec: &ViewSpec,
    scale: f64,
    view: &View,
) -> Result<ViewTransform, DrawingError> {
    let xf = transform(curves, spec, scale)?;
    xf.with_breaks(Breaks::new(&view.breaks, &xf.frame)?, curves.bbox_2d)
}

/// Cuts `part` on the plane through `origin` facing `spec`. Only a section view's own
/// cut (`display_only` given) on an inclined plane falls back to the display mesh's cut
/// when the exact cut fails: the view is marked display-only and the reason goes to the
/// log. Axis-aligned cuts, half sections and broken-out sections keep the exact answer,
/// refusal included.
fn section_or_mesh(
    part: &Part,
    spec: &ViewSpec,
    origin: Vec3,
    display_only: Option<&mut bool>,
) -> soliddraft_kernel_api::KernelResult<SectionView> {
    let section = |preview| part.geometry.section_view(&SectionSpec { view: ViewSpec { preview, ..*spec }, origin });
    match (section(part.preview), display_only) {
        (Err(exact), Some(display_only)) if !part.preview && is_pictorial(spec) => match section(true) {
            Ok(mut mesh) => {
                log::warn!("drawing section: the exact cut failed, showing the display mesh's: {exact}");
                *display_only = true;
                mesh.curves.unproved = false;
                mesh.curves.catallax = false;
                Ok(mesh)
            }
            Err(_) => Err(exact),
        },
        (answer, _) => answer,
    }
}

/// A half section: the side of the symmetry line `half` names is cut open to the plane
/// through the line facing the eye, and hatched; the other side stays as it is.
fn half_section(
    part: &Part,
    index: &PartIndex,
    ctx: &mut ViewContext,
    half: &HalfSection,
    display: &ViewDisplay,
) -> Result<(), DrawingError> {
    let [a, b] = half.axis.map(|point| point.dot(ctx.xf.frame.direction));
    if !a.is_finite() || !b.is_finite() || (a - b).abs() > 1e-8 {
        return Err(DrawingError::BadRequest("Both symmetry axis points must lie in the same cutting plane".into()));
    }
    let flat = ctx.xf.unbroken();
    let boundary = half_boundary(&flat, half)?;
    let mut spec = with_display(ctx.spec(), display);
    spec.preview = part.preview;
    let section = section_or_mesh(part, &spec, half.axis[0], None)?;
    let mut cut = project(index, &section.curves, &ctx.xf, part.digest.as_str(), part.preview);
    let xf = ctx.xf;
    let in_cut = |p: Point2| boundary.contains(flat.precise_point(xf.unproject(p)));
    clip_projected_lines(&mut ctx.projected, &|p| !in_cut(p));
    clip_projected_lines(&mut cut, &in_cut);
    ctx.projected.geometry.curves.extend(cut.geometry.curves);
    ctx.projected.circles.extend(cut.circles);
    ctx.projected.geometry.unproved |= cut.geometry.unproved;
    ctx.projected.geometry.catallax &= cut.geometry.catallax;
    // The half-plane is convex: the cut regions are clipped to it outright.
    let Boundary::Polygon { points: window } = &boundary else { unreachable!("a half-plane is a polygon") };
    ctx.projected.geometry.hatch.extend(
        section
            .regions
            .iter()
            .flat_map(|r| clip_polygon_region(region_polygon(r, &flat, None), window, true))
            .flat_map(|p| split_polygon(p, &xf)),
    );
    Ok(())
}

/// The half-plane a half section cuts away, in view millimetres: a polygon reaching far
/// past any view on the cut side of the symmetry line.
fn half_boundary(xf: &ViewTransform, half: &HalfSection) -> Result<Boundary, DrawingError> {
    const FAR: f64 = 1e5;
    let (a, b) = (xf.precise_point(half.axis[0]), xf.precise_point(half.axis[1]));
    let (du, dv) = (b.u - a.u, b.v - a.v);
    let len = du.hypot(dv);
    if !len.is_finite() || len <= 1e-9 {
        return Err(DrawingError::BadRequest(
            "a half section's symmetry line must not run along the view direction".into(),
        ));
    }
    let (du, dv) = (du / len, dv / len);
    let side = if half.flip { -1.0 } else { 1.0 };
    let (nu, nv) = (-dv * side, du * side);
    let at = |along: f64, across: f64| Point2::new(a.u + du * along + nu * across, a.v + dv * along + nv * across);
    Ok(Boundary::Polygon { points: vec![at(-FAR, 0.0), at(FAR, 0.0), at(FAR, FAR), at(-FAR, FAR)] })
}

/// A half section's symmetry line across the view's line work, 3 mm past it.
fn symmetry_line(ctx: &ViewContext, half: &HalfSection) -> Vec<[Point2; 2]> {
    let flat = ctx.xf.unbroken();
    let (a, b) = (flat.precise_point(half.axis[0]), flat.precise_point(half.axis[1]));
    let (du, dv) = (b.u - a.u, b.v - a.v);
    let len = du.hypot(dv);
    if !len.is_finite() || len <= 1e-9 {
        return Vec::new();
    }
    let (du, dv) = (du / len, dv / len);
    let [x0, y0, x1, y1] = flat.bounds(ctx.curves.bbox_2d);
    let along = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(u, v)| (u - a.u) * du + (v - a.v) * dv);
    let lo = along.iter().copied().fold(f64::INFINITY, f64::min) - 3.0;
    let hi = along.iter().copied().fold(f64::NEG_INFINITY, f64::max) + 3.0;
    let at = |t: f64| Point2::new(round(a.u + du * t), round(a.v + dv * t));
    let axis = Curve3::Line { a: flat.unproject(at(lo)), b: flat.unproject(at(hi)) };
    path_of(&axis, &ctx.xf)
        .segments()
        .map(|segment| [segment.start(), segment.end()].map(|p| Point2::new(p.x, p.y)))
        .collect()
}

pub(crate) fn view_refusal(label: impl FnOnce() -> String, error: DrawingError) -> DrawingError {
    match error {
        DrawingError::Kernel(source) => DrawingError::ViewRefused { view: label(), source },
        other => other,
    }
}

pub(crate) fn with_display(mut spec: ViewSpec, display: &ViewDisplay) -> ViewSpec {
    spec.smooth_edges = display.tangent != crate::view::TangentEdges::Removed;
    spec.seams = display.seams;
    spec
}

/// Cuts a broken-out section into a view: the material in front of the plane `depth`
/// behind the part's nearest point is removed inside the boundary.
fn broken_out(
    part: &Part,
    index: &PartIndex,
    ctx: &mut ViewContext,
    b: &BrokenOut,
    display: &ViewDisplay,
) -> Result<(), DrawingError> {
    let d = ctx.xf.frame.direction;
    let bbox = part.geometry.topology()?.bbox;
    let front = (0..8)
        .map(|i| {
            Vec3::new(
                if i & 1 == 0 { bbox.min.x } else { bbox.max.x },
                if i & 2 == 0 { bbox.min.y } else { bbox.max.y },
                if i & 4 == 0 { bbox.min.z } else { bbox.max.z },
            )
            .dot(d)
        })
        .fold(f64::NEG_INFINITY, f64::max);
    let mut spec = with_display(ctx.spec(), display);
    spec.preview = part.preview;
    let section = section_or_mesh(part, &spec, d * (front - b.depth), None)?;
    let mut cut = project(index, &section.curves, &ctx.xf, part.digest.as_str(), part.preview);
    clip_projected(&mut ctx.projected, &b.boundary, false);
    clip_projected(&mut cut, &b.boundary, true);
    ctx.projected.geometry.curves.extend(cut.geometry.curves);
    ctx.projected.geometry.unproved |= cut.geometry.unproved;
    ctx.projected.geometry.catallax &= cut.geometry.catallax;
    ctx.projected
        .geometry
        .hatch
        .extend(section.regions.iter().flat_map(|r| region_polygons(r, &ctx.xf, Some(b.boundary.clone()))));
    Ok(())
}

/// A cut region as polygons in view millimetres: one, or in a broken view one for each
/// stretch the view keeps.
pub fn region_polygons(r: &CutRegion, xf: &ViewTransform, clip: Option<Boundary>) -> Vec<Polygon> {
    if xf.breaks.is_empty() {
        return vec![region_polygon(r, xf, clip)];
    }
    split_polygon(region_polygon(r, &xf.unbroken(), clip), xf)
}

/// A polygon drawn with `xf.unbroken()`, cut to each stretch `xf` keeps and closed up.
fn split_polygon(polygon: Polygon, xf: &ViewTransform) -> Vec<Polygon> {
    const FAR: f64 = 1e5;
    let b = xf.breaks;
    if b.is_empty() {
        return vec![polygon];
    }
    let centre = b.along(xf.centre);
    let sheet = |s: f64, far: f64| if s.is_finite() { s * xf.scale - centre } else { far };
    b.kept(xf.scale)
        .into_iter()
        .flat_map(|(lo, hi, d)| {
            let (a0, a1) = (sheet(lo, -FAR), sheet(hi, FAR));
            let window: Vec<Point2> = [(a0, -FAR), (a1, -FAR), (a1, FAR), (a0, FAR)]
                .into_iter()
                .map(|(along, across)| b.with_along(Point2::new(across, across), along))
                .collect();
            let mut clipped = clip_polygon_region(
                Polygon {
                    outer: polygon.outer.clone(),
                    holes: polygon.holes.clone(),
                    clip: None,
                    approximate: polygon.approximate,
                },
                &window,
                true,
            );
            for region in &mut clipped {
                for p in region.outer.iter_mut().chain(region.holes.iter_mut().flatten()) {
                    *p = b.with_along(*p, round(b.along(*p) - d));
                }
                region.clip.clone_from(&polygon.clip);
            }
            clipped
        })
        .collect()
}

fn clip_polygon_region(polygon: Polygon, window: &[Point2], inside: bool) -> Vec<Polygon> {
    let mut rings = vec![polygon.outer];
    rings.extend(polygon.holes);
    crate::display::clip_regions(&rings, window, inside)
        .into_iter()
        .filter_map(|shape| {
            let mut rings = shape.into_iter();
            Some(Polygon { outer: rings.next()?, holes: rings.collect(), clip: None, approximate: polygon.approximate })
        })
        .collect()
}

/// A cut region as a polygon in view millimetres.
pub fn region_polygon(r: &CutRegion, xf: &ViewTransform, clip: Option<Boundary>) -> Polygon {
    let ring = |curves: &[ViewCurve]| {
        let mut out: Vec<Point2> = Vec::new();
        for c in curves {
            for p in c.curve.sample(SHEET_CHORD_MM / xf.scale.max(1e-9)) {
                let q = xf.point(p);
                if out.last().is_none_or(|l| (l.u - q.u).abs() > 1e-9 || (l.v - q.v).abs() > 1e-9) {
                    out.push(q);
                }
            }
        }
        if out.len() > 1 && out.first() == out.last() {
            out.pop();
        }
        out
    };
    Polygon {
        outer: ring(&r.outer),
        holes: r.holes.iter().map(|h| ring(h)).collect(),
        clip,
        approximate: r
            .outer
            .iter()
            .chain(r.holes.iter().flatten())
            .any(|c| c.approximate || !matches!(c.curve, Curve3::Line { .. } | Curve3::Polyline { .. })),
    }
}

/// Keeps the part of every curve, hatch and face-on circle inside `boundary` (or
/// outside it, when `inside` is false).
pub fn clip_projected(p: &mut Projected, boundary: &Boundary, inside: bool) {
    clip_projected_lines(p, &|point| boundary.contains(point) == inside);
    p.geometry.hatch = std::mem::take(&mut p.geometry.hatch)
        .into_iter()
        .flat_map(|mut h| {
            if inside && h.clip.is_none() {
                h.clip = Some(boundary.clone());
                return vec![h];
            }
            let hatches = if let Some(previous) = h.clip.take() {
                h.approximate |= matches!(previous, Boundary::Circle { .. });
                clip_polygon_region(h, &previous.polygon(96), true)
            } else {
                vec![h]
            };
            hatches
                .into_iter()
                .flat_map(|mut h| {
                    if inside {
                        h.clip = Some(boundary.clone());
                        vec![h]
                    } else {
                        h.approximate |= matches!(boundary, Boundary::Circle { .. });
                        clip_polygon_region(h, &boundary.polygon(96), false)
                    }
                })
                .collect()
        })
        .collect();
}

fn clip_projected_lines(p: &mut Projected, keep: &dyn Fn(Point2) -> bool) {
    p.geometry.curves.retain_mut(|c| {
        c.path = clip_path(&c.path, &|q| keep(Point2::new(q.x, q.y)));
        c.path.elements().len() >= 2
    });
    p.circles.retain(|c| keep(c.centre));
}

/// The bounds of the curves' control points, or `None` when there are none.
fn curves_bounds(curves: &[Curve2]) -> Option<[f64; 4]> {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for c in curves {
        let r = kurbo::Shape::bounding_box(&c.path);
        b = [b[0].min(r.x0), b[1].min(r.y0), b[2].max(r.x1), b[3].max(r.y1)];
    }
    b[0].is_finite().then_some(b)
}

/// The parts of `path` where `keep` holds, exactly: each segment is cut where it crosses
/// the boundary of the kept region, found by sampling and bisection, and its pieces are
/// the segment's own subsegments.
pub fn clip_path(path: &BezPath, keep: &dyn Fn(Point) -> bool) -> BezPath {
    let mut out = BezPath::new();
    let mut pen: Option<Point> = None;
    for seg in path.segments() {
        let length = kurbo::ParamCurveArclen::arclen(&seg, 1e-3);
        let n = ((length / 0.25).ceil() as usize).clamp(32, 4000);
        let mut cuts = vec![0.0];
        let mut was = keep(seg.eval(0.0));
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let now = keep(seg.eval(t));
            if now != was {
                let (mut lo, mut hi) = ((i - 1) as f64 / n as f64, t);
                for _ in 0..48 {
                    let mid = 0.5 * (lo + hi);
                    if keep(seg.eval(mid)) == was {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                cuts.push(0.5 * (lo + hi));
            }
            was = now;
        }
        cuts.push(1.0);
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if b - a < 1e-12 || !keep(seg.eval(0.5 * (a + b))) {
                pen = None;
                continue;
            }
            let piece = seg.subsegment(a..b);
            let start = piece.eval(0.0);
            if pen.is_none_or(|q| q.distance(start) > 1e-9) {
                out.move_to(start);
            }
            match piece {
                PathSeg::Line(l) => out.line_to(l.p1),
                PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
                PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
            }
            pen = Some(piece.eval(1.0));
        }
    }
    out
}

/// True when `spec` looks along no model axis.
pub fn is_pictorial(spec: &ViewSpec) -> bool {
    let d = spec.direction.normalized().unwrap_or(Vec3::Z);
    [d.x, d.y, d.z].iter().filter(|c| c.abs() > 1e-9).count() > 1
}

/// The kernel request for a view: its orientation, followed through its parents.
pub fn view_spec(drawing: &Drawing, id: ViewId) -> Result<ViewSpec, DrawingError> {
    let sheet = sheet_of(drawing, id).ok_or(DrawingError::UnknownView(id))?;
    let view = drawing.view(id).ok_or(DrawingError::UnknownView(id))?;
    spec_in(drawing, sheet, view).ok_or(DrawingError::BadView(id))
}

/// The kernel request for a view of `sheet`, or `None` when its orientation is
/// degenerate or a parent is missing. Parents are looked for on `sheet` only; see
/// [`spec_in`] for views whose parents are on other sheets. An auxiliary view's own
/// direction needs the part, so here it looks the way its parent does.
pub fn spec_of(sheet: &Sheet, view: &View) -> Option<ViewSpec> {
    let find = |id: ViewId| sheet.view(id).map(|v| (v, sheet.projection));
    let (direction, up) = view_axes(&find, view, sheet.projection, 0)?;
    Some(with_display(ViewSpec::new(direction, up), &view.display))
}

/// The kernel request for `view` as if on sheet `sheet` of `drawing`, its parents looked
/// for on every sheet. An auxiliary view looks the way its parent does here; its own
/// direction needs the part ([`compute_view`]).
pub fn spec_in(drawing: &Drawing, sheet: usize, view: &View) -> Option<ViewSpec> {
    let find = |id: ViewId| drawing.sheets.iter().find_map(|s| s.view(id).map(|v| (v, s.projection)));
    let projection = drawing.sheets.get(sheet).map_or(Projection::FirstAngle, |s| s.projection);
    let (direction, up) = view_axes(&find, view, projection, 0)?;
    Some(with_display(ViewSpec::new(direction, up), &view.display))
}

type Find<'a> = dyn Fn(ViewId) -> Option<(&'a View, Projection)> + 'a;

/// A view's direction towards the eye and up vector, following its parents back to a
/// base view. `projection` is the angle of the sheet the view is on.
fn view_axes<'a>(find: &Find<'a>, view: &View, projection: Projection, depth: u8) -> Option<(Vec3, Vec3)> {
    if depth > 16 {
        return None;
    }
    let parent = |id: ViewId| {
        let (p, angle) = find(id)?;
        view_axes(find, p, angle, depth + 1)
    };
    match &view.kind {
        ViewKind::Base { orientation } => {
            let s = orientation.spec(&view.display);
            Some((s.direction, s.up))
        }
        ViewKind::Projected { parent: p, direction, projection: placed } => {
            let (pd, pu) = parent(*p)?;
            let frame = ViewSpec::new(pd, pu).frame()?;
            Some(fold(&frame, *direction, placed.unwrap_or(projection)))
        }
        ViewKind::Section { parent: p, line, model_line, flip, .. } => {
            let (pd, pu) = parent(*p)?;
            section_model_axes(&ViewSpec::new(pd, pu).frame()?, line, model_line, *flip)
        }
        ViewKind::Detail { parent: p, .. } | ViewKind::Auxiliary { parent: p, .. } => parent(*p),
    }
}

fn section_model_axes(frame: &ViewFrame, line: &[Point2], model_line: &[Vec3], flip: bool) -> Option<(Vec3, Vec3)> {
    if model_line.len() == line.len() {
        let trace: Vec<_> = model_line.iter().map(|p| frame.project(*p)).collect();
        section_axes(frame, &trace, flip)
    } else {
        section_axes(frame, line, flip)
    }
}

/// The way the arrows of a cutting line point on the parent, the way the section looks,
/// as a unit vector in the parent's view: to the left of the line run from its leftmost
/// (or, when vertical, lowest) point, the other way when `flip` is set. `None` when the
/// line has no length.
pub fn section_arrow(line: &[Point2], flip: bool) -> Option<Point2> {
    if line.len() > 2 {
        // A bent line looks the way its first cut does.
        return CutPath::of_line(line, flip).ok().map(|path| path.spans()[0].arrow());
    }
    let (a, b) = (line.first()?, line.last()?);
    let (mut dx, mut dy) = (b.u - a.u, b.v - a.v);
    let len = dx.hypot(dy);
    if len < 1e-9 {
        return None;
    }
    (dx, dy) = (dx / len, dy / len);
    if dx < -1e-9 || (dx.abs() <= 1e-9 && dy < 0.0) {
        (dx, dy) = (-dx, -dy);
    }
    let arrow = Point2::new(-dy, dx);
    Some(if flip { Point2::new(-arrow.u, -arrow.v) } else { arrow })
}

/// The frame turned a quarter turn about the in-view direction `axis` so that the eye
/// moves to the side `eye` (both unit vectors in the parent's view): what a section or
/// auxiliary view looks along. Returns (direction towards the eye, up).
pub fn fold_about(parent: &ViewFrame, axis: Point2, eye: Point2) -> (Vec3, Vec3) {
    let e = parent.right * axis.u + parent.up * axis.v;
    let n = parent.right * eye.u + parent.up * eye.v;
    let d = parent.direction;
    let turn = |v: Vec3| e * v.dot(e) - d * v.dot(n) + n * v.dot(d);
    (n, turn(parent.up))
}

/// The direction towards the eye and up vector of a section through a parent with frame
/// `parent` along `line`.
pub fn section_axes(parent: &ViewFrame, line: &[Point2], flip: bool) -> Option<(Vec3, Vec3)> {
    let arrow = section_arrow(line, flip)?;
    let axis = Point2::new(arrow.v, -arrow.u);
    Some(fold_about(parent, axis, Point2::new(-arrow.u, -arrow.v)))
}

/// The direction an auxiliary view looks from and its up vector: square to the
/// reference edge in the parent's view, from the side of the edge away from the
/// parent's centre unless `flip`.
fn auxiliary_axes(
    index: &PartIndex,
    pctx: &ViewContext,
    edge: &EntityRef,
    flip: bool,
) -> Result<(Vec3, Vec3), DrawingError> {
    let id = match resolve_in_view(index, pctx, edge) {
        Resolution::Found(Entity::Edge(id)) => id,
        Resolution::Found(_) => return Err(DrawingError::BadRequest("an auxiliary view needs an edge".into())),
        Resolution::Lost(why) => return Err(DrawingError::LostReference(why)),
    };
    let curve = index.edge_curve(id).ok_or(DrawingError::LostReference("the edge has no curve".into()))?;
    let turned = pctx.xf.leg_turn(edge.section_leg).map(|turn| turn.curve(curve));
    let curve = turned.as_ref().unwrap_or(curve);
    let along = pctx.xf.vector(curve.end() - curve.start());
    let len = along.u.hypot(along.v);
    if len < 1e-6 {
        return Err(DrawingError::BadRequest("the parent sees the reference edge end on".into()));
    }
    let axis = Point2::new(along.u / len, along.v / len);
    let mut eye = Point2::new(-axis.v, axis.u);
    let mid = pctx.xf.point(curve.point_at(0.5));
    if mid.u * eye.u + mid.v * eye.v < 0.0 {
        eye = Point2::new(-eye.u, -eye.v);
    }
    if flip {
        eye = Point2::new(-eye.u, -eye.v);
    }
    Ok(fold_about(&pctx.xf.frame, axis, eye))
}

/// Resolves a reference picked in the view `ctx`. On an aligned section it must still
/// be drawn on the leg it was picked from: turning the cut round can take an edge off
/// that leg while the part keeps it.
pub(crate) fn resolve_in_view(index: &PartIndex, ctx: &ViewContext, entity: &EntityRef) -> Resolution {
    let leg = entity.section_leg;
    if leg != 0 && ctx.xf.leg_turn(leg).is_none() {
        return Resolution::Lost("The aligned cut leg is missing".into());
    }
    let found = index.resolve(entity);
    if let Resolution::Found(e) = &found
        && ctx.xf.aligned_turn.is_some()
        && let Some(r) = index.entity_ref(*e)
        && let Some(hint) = r.kernel_hint
        && !ctx
            .projected
            .geometry
            .curves
            .iter()
            .any(|c| c.source.kind == r.kind && c.source.kernel_hint == Some(hint) && c.source.section_leg == leg)
    {
        return Resolution::Lost("The reference is no longer on its leg of the aligned cut".into());
    }
    found
}

/// The direction towards the eye and the up vector of a view folded off a parent with
/// frame `parent` towards `side`.
///
/// Third angle places a view on the side of the face it shows; first angle on the
/// opposite side. The diagonals give pictorial views from the corner between the two
/// faces the neighbouring orthographic folds show.
pub fn fold(parent: &ViewFrame, side: FoldDirection, projection: Projection) -> (Vec3, Vec3) {
    let (r, u, d) = (parent.right, parent.up, parent.direction);
    let (sx, sy): (f64, f64) = match side {
        FoldDirection::Left => (-1.0, 0.0),
        FoldDirection::Right => (1.0, 0.0),
        FoldDirection::Up => (0.0, 1.0),
        FoldDirection::Down => (0.0, -1.0),
        FoldDirection::UpLeft => (-1.0, 1.0),
        FoldDirection::UpRight => (1.0, 1.0),
        FoldDirection::DownLeft => (-1.0, -1.0),
        FoldDirection::DownRight => (1.0, -1.0),
    };
    let k = match projection {
        Projection::ThirdAngle => 1.0,
        Projection::FirstAngle => -1.0,
    };
    let (sx, sy) = (sx * k, sy * k);
    if side.is_diagonal() {
        let direction = (r * sx + u * sy + d).normalized().unwrap_or(d);
        return (direction, u);
    }
    if sy == 0.0 {
        (r * sx, u)
    } else {
        // Folding up or down: the old direction towards the eye becomes the new down or up.
        (u * sy, d * -sy)
    }
}

/// Sheet millimetres per model millimetre of a view, following parents on any sheet.
pub fn view_scale(drawing: &Drawing, id: ViewId) -> Result<f64, DrawingError> {
    let sheet = sheet_of(drawing, id).ok_or(DrawingError::UnknownView(id))?;
    Ok(scale_in(drawing, sheet, drawing.view(id).ok_or(DrawingError::UnknownView(id))?))
}

/// Sheet millimetres per model millimetre of a view of `sheet`, following parents on
/// that sheet. See [`scale_in`] for views whose parents are on other sheets.
pub fn effective_scale(sheet: &Sheet, view: &View) -> f64 {
    let find = |id: ViewId| sheet.view(id).map(|v| (v, sheet.scale.factor()));
    scale_of(&find, sheet.scale.factor(), view, 0)
}

/// Sheet millimetres per model millimetre of `view` as if on sheet `sheet` of
/// `drawing`, following parents on every sheet.
pub fn scale_in(drawing: &Drawing, sheet: usize, view: &View) -> f64 {
    let find = |id: ViewId| drawing.sheets.iter().find_map(|s| s.view(id).map(|v| (v, s.scale.factor())));
    let own = drawing.sheets.get(sheet).map_or(1.0, |s| s.scale.factor());
    scale_of(&find, own, view, 0)
}

type FindScale<'a> = dyn Fn(ViewId) -> Option<(&'a View, f64)> + 'a;

/// A detail view that follows its parent's scale magnifies it by its factor.
fn scale_of<'a>(find: &FindScale<'a>, sheet_scale: f64, view: &View, depth: u8) -> f64 {
    match view.scale {
        ViewScale::Sheet => sheet_scale,
        ViewScale::Custom { factor } => factor,
        ViewScale::Parent => {
            let factor = match view.kind {
                ViewKind::Detail { factor, .. } if factor > 0.0 => factor,
                _ => 1.0,
            };
            match view.parent().and_then(find) {
                Some((p, own)) if depth < 16 => scale_of(find, own, p, depth + 1) * factor,
                _ => sheet_scale * factor,
            }
        }
    }
}

/// The drawing geometry's curves for a view (with mesh fallback handled by the geometry
/// adapter for preview requests only).
pub fn view_curves(part: &Part, spec: &ViewSpec) -> Result<ViewCurves, DrawingError> {
    let spec = ViewSpec { preview: part.preview, ..*spec };
    Ok(part.geometry.view_curves(&spec)?)
}

/// The transform that centres `curves` at `scale`.
pub fn transform(curves: &ViewCurves, spec: &ViewSpec, scale: f64) -> Result<ViewTransform, DrawingError> {
    let frame = spec.frame().ok_or_else(|| DrawingError::Kernel(KernelError::InvalidInput("view direction".into())))?;
    let [x0, y0, x1, y1] = curves.bbox_2d;
    let centre = Point2::new((x0 + x1) / 2.0 * scale, (y0 + y1) / 2.0 * scale);
    Ok(ViewTransform::new(frame, scale, centre))
}

/// A circular edge seen face-on in a view: what centre marks and hole callouts attach
/// to.
#[derive(Clone, Debug, PartialEq)]
pub struct CircleInfo {
    /// The edge.
    pub entity: EntityRef,
    /// Centre in view millimetres.
    pub centre: Point2,
    /// Radius in view millimetres.
    pub radius: f64,
    /// Centre in model space.
    pub model_centre: Vec3,
    /// Radius in model millimetres.
    pub model_radius: f64,
}

/// One view's projection: its line work, its transform and the circles it sees
/// face-on.
#[derive(Clone, Debug, PartialEq)]
pub struct Projected {
    /// The line work.
    pub geometry: ViewGeometry,
    /// Model space to view millimetres.
    pub transform: ViewTransform,
    /// Circles seen face-on, one per edge.
    pub circles: Vec<CircleInfo>,
}

impl Projected {
    /// Visible circular hole rims, deduplicated where the same diameter shares a centre.
    pub fn hole_rows(&self, namer: &dyn crate::Namer) -> Vec<crate::HoleRow> {
        let visible: std::collections::HashSet<_> = self
            .geometry
            .curves
            .iter()
            .filter(|curve| curve.visible && curve.kind == CurveKind::Edge)
            .filter_map(|curve| curve.source.kernel_hint.map(|id| (id, curve.source.section_leg)))
            .collect();
        let mut holes: Vec<_> = self
            .circles
            .iter()
            .filter(|circle| {
                circle.entity.kernel_hint.is_some_and(|id| visible.contains(&(id, circle.entity.section_leg)))
                    && namer.is_hole(circle)
            })
            .map(|circle| {
                let coordinate = self.transform.frame.project(circle.model_centre);
                let scale = self.transform.scale;
                crate::HoleRow {
                    coordinates: Point2::new(round(coordinate.u), round(coordinate.v)),
                    centre: Point2::new(round(circle.centre.u / scale), round(circle.centre.v / scale)),
                    diameter: round(circle.model_radius * 2.0),
                }
            })
            .collect();
        holes.sort_by(|a, b| {
            a.coordinates
                .u
                .total_cmp(&b.coordinates.u)
                .then(a.coordinates.v.total_cmp(&b.coordinates.v))
                .then(a.diameter.total_cmp(&b.diameter))
        });
        holes.dedup_by(|a, b| a.coordinates == b.coordinates && a.diameter == b.diameter);
        holes
    }

    /// Width and height in view millimetres.
    pub fn size(&self) -> (f64, f64) {
        let [x0, y0, x1, y1] = self.geometry.bbox;
        (x1 - x0, y1 - y0)
    }

    /// Changes the scale without going back to the kernel.
    pub fn rescale(&mut self, scale: f64) {
        let k = scale / self.transform.scale;
        if (k - 1.0).abs() < 1e-12 {
            return;
        }
        rescale_geometry(&mut self.geometry, k);
        self.transform = self.transform.rescaled(scale);
        for c in &mut self.circles {
            c.centre = self.transform.point(c.model_centre);
            c.radius *= k;
        }
    }
}

/// Projects the kernel's curves into a view's line work.
pub fn project(index: &PartIndex, curves: &ViewCurves, xf: &ViewTransform, digest: &str, coarse: bool) -> Projected {
    project_curves(index, curves, xf, digest, coarse, None)
}

fn project_curves(
    index: &PartIndex,
    curves: &ViewCurves,
    xf: &ViewTransform,
    digest: &str,
    coarse: bool,
    first_leg_curves: Option<usize>,
) -> Projected {
    let mut out = Vec::with_capacity(curves.curves.len());
    let mut circles: Vec<CircleInfo> = Vec::new();
    for (i, c) in curves.curves.iter().enumerate() {
        let path = path_of(&c.curve, xf);
        if path.elements().len() < 2 {
            continue;
        }
        let (mut source, kind) = source_ref(index, c);
        source.section_leg = u8::from(xf.aligned_turn.is_some() && first_leg_curves.is_some_and(|n| i >= n));
        // Visibility fragments keep the complete edge's exact circle metadata.
        let exact = match c.source {
            CurveSource::Edge(edge) => index.exact_edge_curve(edge),
            _ => None,
        };
        let turned = exact.zip(xf.leg_turn(source.section_leg)).map(|(curve, turn)| turn.curve(curve));
        let geometry = turned.as_ref().or(exact).unwrap_or(&c.curve);
        if !c.approximate
            && let Curve3::Circle { center, normal, radius } = geometry
            && normal.cross(xf.frame.direction).length() < 1e-6
            && !xf.breaks.hides(center.dot(xf.breaks.axis(&xf.frame)))
            && matches!(c.source, CurveSource::Edge(_))
            && !circles.iter().any(|k| k.entity.same_projected_entity(&source))
        {
            circles.push(CircleInfo {
                entity: source.clone(),
                centre: xf.point(*center),
                radius: radius * xf.scale,
                model_centre: *center,
                model_radius: *radius,
            });
        }
        // The kernel's marks win over the curve's kind: a fitted line is still approximate.
        let accuracy =
            if c.approximate || !curves.approximate.is_empty() { Accuracy::Approximate } else { accuracy_of(&c.curve) };
        let accuracy = accuracy.for_path(&path);
        let ends = (!xf.breaks.is_empty() || first_leg_curves.is_some())
            .then(|| [geometry.start(), geometry.end()].map(|p| xf.point(p)));
        out.push(Curve2 { path, source, visible: c.visible, kind, tangent: c.smooth, accuracy, ends });
    }
    let geometry = ViewGeometry {
        source_digest: digest.to_owned(),
        curves: out,
        hatch: Vec::new(),
        outline: None,
        bbox: xf.bounds(curves.bbox_2d),
        unproved: curves.unproved,
        display_only: false,
        catallax: curves.catallax,
        coarse,
        breaks: Vec::new(),
        symmetry_segments: Vec::new(),
    };
    Projected { geometry, transform: *xf, circles }
}

/// Projects `part` for a view with orientation `spec` at `scale`.
pub fn project_part(part: &Part, index: &PartIndex, spec: &ViewSpec, scale: f64) -> Result<Projected, DrawingError> {
    let curves = view_curves(part, spec)?;
    let xf = transform(&curves, spec, scale)?;
    Ok(project(index, &curves, &xf, part.digest.as_str(), part.preview))
}

/// Centre marks for the circles a view sees face-on, one per centre (the largest circle
/// there), with ids from `first_id`.
pub fn centre_marks(projected: &Projected, view: ViewId, first_id: u32) -> Vec<Annotation> {
    let mut by_centre: Vec<&CircleInfo> = Vec::new();
    for c in &projected.circles {
        match by_centre
            .iter()
            .position(|k| (k.centre.u - c.centre.u).abs() < 1e-3 && (k.centre.v - c.centre.v).abs() < 1e-3)
        {
            Some(i) if c.radius > by_centre[i].radius => by_centre[i] = c,
            Some(_) => {}
            None => by_centre.push(c),
        }
    }
    by_centre
        .iter()
        .enumerate()
        .map(|(i, c)| Annotation {
            id: AnnotationId(first_id + i as u32),
            view,
            body: AnnotationKind::CentreMark,
            anchors: vec![Anchor { entity: c.entity.clone(), attach: Attach::Centre, cached: c.centre }],
            placement: c.centre,
            style: StyleOverrides::default(),
            state: AnnotationState::Ok,
            source: None,
        })
        .collect()
}

/// The standard view with these axes, if any.
pub fn standard_of(direction: Vec3, up: Vec3) -> Option<StandardView> {
    let want = ViewSpec::new(direction, up).frame()?;
    StandardView::ALL.into_iter().find(|v| {
        ViewSpec::standard(*v)
            .frame()
            .is_some_and(|f| f.direction.distance(want.direction) < 1e-6 && f.up.distance(want.up) < 1e-6)
    })
}

/// The caption of a standard view, such as "Front view".
pub fn standard_label(view: StandardView) -> &'static str {
    match view {
        StandardView::Front => "Front view",
        StandardView::Back => "Back view",
        StandardView::Top => "Top view",
        StandardView::Bottom => "Bottom view",
        StandardView::Right => "Right view",
        StandardView::Left => "Left view",
        StandardView::Isometric => "Isometric view",
    }
}

/// A view's name in the tree: its own label, or what it shows.
pub fn view_label_in(drawing: &Drawing, sheet: &Sheet, view: &View) -> String {
    let find = |id: ViewId| drawing.sheets.iter().find_map(|s| s.view(id).map(|v| (v, s.projection)));
    if let Some(l) = &view.label {
        return l.clone();
    }
    match &view.kind {
        ViewKind::Section { label, .. } => format!("Section {label}-{label}"),
        ViewKind::Detail { label, .. } => format!("Detail {label}"),
        ViewKind::Auxiliary { label, .. } if !label.is_empty() => format!("View {label}"),
        ViewKind::Auxiliary { .. } => "Auxiliary view".into(),
        _ => match view_axes(&find, view, sheet.projection, 0).and_then(|(d, u)| standard_of(d, u)) {
            Some(s) => standard_label(s).to_owned(),
            None if matches!(view.kind, ViewKind::Base { .. }) => "Model view".into(),
            None => "Pictorial view".into(),
        },
    }
}

/// The side a projected standard view folds to from the front view under a projection
/// angle, or `None` for views that do not fold from the front.
pub fn standard_fold(projection: Projection, view: StandardView) -> Option<FoldDirection> {
    Some(match (projection, view) {
        (Projection::FirstAngle, StandardView::Top) => FoldDirection::Down,
        (Projection::FirstAngle, StandardView::Bottom) => FoldDirection::Up,
        (Projection::FirstAngle, StandardView::Left) => FoldDirection::Right,
        (Projection::FirstAngle, StandardView::Right) => FoldDirection::Left,
        (Projection::ThirdAngle, StandardView::Top) => FoldDirection::Up,
        (Projection::ThirdAngle, StandardView::Bottom) => FoldDirection::Down,
        (Projection::ThirdAngle, StandardView::Left) => FoldDirection::Left,
        (Projection::ThirdAngle, StandardView::Right) => FoldDirection::Right,
        _ => return None,
    })
}

/// The display a standard view starts with in the app: hidden lines shown on
/// orthographic views, removed on pictorial ones, tangent edges phantom on pictorials.
pub fn default_display(view: StandardView) -> ViewDisplay {
    match view {
        StandardView::Isometric => ViewDisplay::PICTORIAL,
        _ => ViewDisplay { hidden: crate::view::HiddenLines::Visible, ..ViewDisplay::ORTHOGRAPHIC },
    }
}

/// Where a view's geometry sits on the sheet: `[left, bottom, right, top]`.
pub fn view_bounds(view: &View) -> [f64; 4] {
    let [x0, y0, x1, y1] = view.geometry.bbox;
    [x0 + view.position.u, y0 + view.position.v, x1 + view.position.u, y1 + view.position.v]
}

/// A view and its dependent descendants, parent first, across every sheet.
pub fn dependent_views(drawing: &Drawing, parent: ViewId) -> Vec<ViewId> {
    let mut out = vec![parent];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i];
        for view in drawing.sheets.iter().flat_map(|sheet| &sheet.views) {
            if view.parent() == Some(parent) && !out.contains(&view.id) {
                out.push(view.id);
            }
        }
        i += 1;
    }
    out
}

/// The views aligned to `parent`, directly or through other children.
pub fn descendants(sheet: &Sheet, parent: ViewId) -> Vec<ViewId> {
    let mut out = Vec::new();
    let mut stack = vec![parent];
    while let Some(p) = stack.pop() {
        for v in &sheet.views {
            if v.alignment.is_some_and(|a| a.parent == p) && !out.contains(&v.id) {
                out.push(v.id);
                stack.push(v.id);
            }
        }
    }
    out
}

/// Index of the sheet a view is on.
pub fn sheet_of(drawing: &Drawing, view: ViewId) -> Option<usize> {
    drawing.sheets.iter().position(|s| s.view(view).is_some())
}

/// The name of a view curve's source.
fn source_ref(index: &PartIndex, c: &ViewCurve) -> (EntityRef, CurveKind) {
    let unknown = || EntityRef::new(EntityKind::Edge, "", "unknown", 0, Vec3::ZERO, 0.0);
    match c.source {
        CurveSource::Edge(id) => (index.edge_ref(id).unwrap_or_else(unknown), CurveKind::Edge),
        CurveSource::Silhouette(id) => (index.face_ref(id).unwrap_or_else(unknown), CurveKind::Silhouette),
        CurveSource::Seam(id) => (index.face_ref(id).unwrap_or_else(unknown), CurveKind::Seam),
        CurveSource::Section(id) => (index.face_ref(id).unwrap_or_else(unknown), CurveKind::Section),
        CurveSource::Unknown => (unknown(), CurveKind::Edge),
    }
}

/// Chord tolerance on the sheet for curves that are neither lines nor arcs.
const SHEET_CHORD_MM: f64 = 0.01;

fn accuracy_of(curve: &Curve3) -> Accuracy {
    match curve {
        Curve3::Line { .. } | Curve3::Arc { .. } | Curve3::Circle { .. } => Accuracy::Exact,
        Curve3::Polyline { .. } | Curve3::Nurbs { .. } => Accuracy::Polygon,
    }
}

/// A model curve as a path in view millimetres. In a broken view the curve is cut where
/// it enters and leaves each stretch left out, and each kept piece moves back whole.
pub fn path_of(curve: &Curve3, xf: &ViewTransform) -> BezPath {
    let b = xf.breaks;
    if b.is_empty() {
        return flat_path_of(curve, xf);
    }
    let flat = xf.unbroken();
    let axis = b.axis(&xf.frame);
    let mut path = BezPath::new();
    for (lo, hi, d) in b.kept(xf.scale) {
        for piece in Band::along(axis, lo, hi).clip(curve) {
            let moved = |p: Point| {
                let q = b.with_along(Point2::new(p.x, p.y), round(b.along(Point2::new(p.x, p.y)) - d));
                Point::new(q.u, q.v)
            };
            path.extend(flat_path_of(&piece, &flat).elements().iter().map(|el| map_el(*el, moved)));
        }
    }
    path
}

fn flat_path_of(curve: &Curve3, xf: &ViewTransform) -> BezPath {
    let mut path = BezPath::new();
    match curve {
        Curve3::Line { a, b } => {
            path.move_to(xf.kpoint(*a));
            path.line_to(xf.kpoint(*b));
        }
        Curve3::Arc { center, normal, x_axis, radius, start_deg, end_deg } => {
            arc_path(&mut path, xf, *center, *normal, *x_axis, *radius, *start_deg, *end_deg);
        }
        Curve3::Circle { center, normal, radius } => {
            let x_axis = soliddraft_kernel_api::drawing::perpendicular(*normal);
            arc_path(&mut path, xf, *center, *normal, x_axis, *radius, 0.0, 360.0);
            path.close_path();
        }
        Curve3::Nurbs { .. } => {
            let simple = curve.simplified(1e-6);
            if !matches!(simple, Curve3::Nurbs { .. }) {
                return flat_path_of(&simple, xf);
            }
            polyline(&mut path, xf, &curve.sample(SHEET_CHORD_MM / xf.scale.max(1e-9)));
        }
        Curve3::Polyline { points } => polyline(&mut path, xf, points),
    }
    path
}

fn polyline(path: &mut BezPath, xf: &ViewTransform, points: &[Vec3]) {
    for (i, p) in points.iter().enumerate() {
        if i == 0 {
            path.move_to(xf.kpoint(*p));
        } else {
            path.line_to(xf.kpoint(*p));
        }
    }
}

/// Cubic Béziers for a circular arc, at most a quarter turn each, built in model space
/// and projected. Parallel projection is affine, so the projected cubics approximate the
/// projected arc exactly as well as the model ones approximate the arc.
#[allow(clippy::too_many_arguments)]
fn arc_path(
    path: &mut BezPath,
    xf: &ViewTransform,
    center: Vec3,
    normal: Vec3,
    x_axis: Vec3,
    radius: f64,
    start_deg: f64,
    end_deg: f64,
) {
    let y_axis = normal.cross(x_axis);
    let at = |t: f64| center + (x_axis * t.cos() + y_axis * t.sin()) * radius;
    let tangent = |t: f64| (x_axis * -t.sin() + y_axis * t.cos()) * radius;
    let (a0, a1) = (start_deg.to_radians(), end_deg.to_radians());
    let pieces = ((end_deg - start_deg).abs() / 90.0).ceil().max(1.0) as usize;
    let step = (a1 - a0) / pieces as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    path.move_to(xf.kpoint(at(a0)));
    for i in 0..pieces {
        let (s, e) = (a0 + step * i as f64, a0 + step * (i + 1) as f64);
        let (p0, p3) = (at(s), at(e));
        let p1 = p0 + tangent(s) * k;
        let p2 = p3 - tangent(e) * k;
        path.curve_to(xf.kpoint(p1), xf.kpoint(p2), xf.kpoint(p3));
    }
}

pub(crate) fn scale_view_point(p: Point2, ratio: f64, gap: Option<BreakGap>) -> Point2 {
    let scaled = Point2::new(p.u * ratio, p.v * ratio);
    let Some(g) = gap else { return scaled };
    let along = if g.axis == AlignAxis::Vertical { p.v } else { p.u };
    let fraction = ((along - g.at) / g.gap).clamp(0.0, 1.0);
    let at = along * ratio + (1.0 - ratio) * g.gap * (fraction - g.centre_fraction);
    if g.axis == AlignAxis::Vertical { Point2::new(scaled.u, at) } else { Point2::new(at, scaled.v) }
}

fn scaled_path(path: &BezPath, ratio: f64, gap: Option<BreakGap>, shift: Point2) -> BezPath {
    let mut offset = Point2::default();
    path.elements()
        .iter()
        .map(|el| {
            if let kurbo::PathEl::MoveTo(p) = *el {
                let moved = scale_view_point(Point2::new(p.x, p.y), ratio, gap);
                offset = Point2::new(moved.u - p.x * ratio, moved.v - p.y * ratio);
            }
            map_el(*el, |p| {
                Point::new(round(p.x * ratio + offset.u - shift.u), round(p.y * ratio + offset.v - shift.v))
            })
        })
        .collect()
}

pub(crate) fn scaled_boundary(boundary: &Boundary, ratio: f64, gap: Option<BreakGap>) -> Boundary {
    map_boundary(boundary, ratio, gap.map(|g| (g.axis, g.at, g.at + g.gap)), |p| scale_view_point(p, ratio, gap))
}

fn map_boundary(
    boundary: &Boundary,
    ratio: f64,
    cuts: Option<(AlignAxis, f64, f64)>,
    map: impl Fn(Point2) -> Point2,
) -> Boundary {
    let spans_gap = cuts.filter(|(axis, start, end)| {
        let [min_x, min_y, max_x, max_y] = boundary.bounds();
        let (lo, hi) = if *axis == AlignAxis::Vertical { (min_y, max_y) } else { (min_x, max_x) };
        [start, end].into_iter().any(|station| lo < *station && *station < hi)
    });
    if let Boundary::Circle { center, radius } = boundary
        && spans_gap.is_none()
        && !cuts.is_some_and(|(axis, lo, hi)| {
            let at = match axis {
                AlignAxis::Horizontal => center.u,
                AlignAxis::Vertical => center.v,
            };
            lo < at && at < hi
        })
    {
        return Boundary::Circle { center: map(*center), radius: radius * ratio };
    }

    let points = boundary.polygon(96);
    let mut mapped = Vec::with_capacity(points.len());
    for (a, b) in points.iter().copied().zip(points.iter().copied().cycle().skip(1)) {
        mapped.push(map(a));
        if let Some((axis, lo, hi)) = spans_gap {
            let along = |p: Point2| match axis {
                AlignAxis::Horizontal => p.u,
                AlignAxis::Vertical => p.v,
            };
            let (start, end) = (along(a), along(b));
            let stations = if start <= end { [lo, hi] } else { [hi, lo] };
            for station in stations {
                let fraction = (station - start) / (end - start);
                if fraction > 0.0 && fraction < 1.0 {
                    mapped.push(map(Point2::new(a.u + (b.u - a.u) * fraction, a.v + (b.v - a.v) * fraction)));
                }
            }
        }
    }
    Boundary::Polygon { points: mapped }
}

fn remap_boundary(boundary: &Boundary, before: &ViewTransform, after: &ViewTransform) -> Boundary {
    if before == after {
        return boundary.clone();
    }
    let flat = before.unbroken();
    let old_cuts = before.gaps().first().map(|g| (g.axis, g.at, g.at + g.gap));
    let unbroken = map_boundary(boundary, 1.0, old_cuts, |p| flat.precise_point(before.unproject(p)));
    let new_cuts = after.gaps().first().map(|g| {
        let at = |s| {
            let p = match g.axis {
                AlignAxis::Horizontal => Point2::new(s, 0.0),
                AlignAxis::Vertical => Point2::new(0.0, s),
            };
            g.along(flat.precise_point(after.unproject(p)))
        };
        (g.axis, at(g.at), at(g.at + g.gap))
    });
    map_boundary(&unbroken, after.scale / flat.scale, new_cuts, |p| after.precise_point(flat.unproject(p)))
}

pub(crate) fn remap_parent_boundary(kind: &mut ViewKind, before: &ViewTransform, after: &ViewTransform) {
    if before == after {
        return;
    }
    if let ViewKind::Detail { center, radius, profile, model_center, .. } = kind {
        let boundary = if profile.len() >= 3 {
            Boundary::Polygon { points: std::mem::take(profile) }
        } else {
            Boundary::Circle { center: *center, radius: *radius }
        };
        *center = after.point(model_center.unwrap_or_else(|| before.unproject(*center)));
        match remap_boundary(&boundary, before, after) {
            Boundary::Circle { radius: r, .. } => *radius = r,
            Boundary::Polygon { points } => *profile = points,
        }
    }
}

pub(crate) fn remap_view_boundaries(view: &mut View, before: &ViewTransform, after: &ViewTransform) {
    if let Some(crop) = &mut view.crop {
        *crop = remap_boundary(crop, before, after);
    }
    for cut in &mut view.broken_out {
        cut.boundary = remap_boundary(&cut.boundary, before, after);
    }
}

fn invalid_break() -> DrawingError {
    DrawingError::BadRequest("The break must shorten the view and leave material on both sides".into())
}

fn spans_break(geometry: &ViewGeometry, display: ViewDisplay) -> bool {
    let Some(gap) = geometry.breaks.first() else { return false };
    let bounds = geometry
        .curves
        .iter()
        .filter(|c| c.visible || display.hidden != crate::HiddenLines::Removed)
        .map(|c| kurbo::Shape::bounding_box(&c.path));
    let (mut left, mut right) = (false, false);
    for r in bounds {
        let (lo, hi) = match gap.axis {
            AlignAxis::Horizontal => (r.x0, r.x1),
            AlignAxis::Vertical => (r.y0, r.y1),
        };
        left |= lo < gap.at - 1e-6;
        right |= hi > gap.at + gap.gap + 1e-6;
    }
    left && right
}

pub(crate) fn validate_break(
    geometry: &ViewGeometry,
    display: ViewDisplay,
    ratio: f64,
    must_span: bool,
) -> Result<(), DrawingError> {
    let too_short =
        geometry.breaks.first().is_some_and(|gap| gap.omitted_mm.is_some_and(|width| width * ratio <= gap.gap));
    if too_short || must_span {
        let spans = spans_break(geometry, display);
        if (too_short && spans) || (must_span && !spans) {
            return Err(invalid_break());
        }
    }
    Ok(())
}

/// Re-projects a view at a new scale without the kernel.
pub fn rescale_view(view: &mut View, ratio: f64) {
    let gap = view.geometry.breaks.first().copied();
    rescale_geometry(&mut view.geometry, ratio);
    if let Some(c) = &mut view.crop {
        *c = scaled_boundary(c, ratio, gap);
    }
    for b in &mut view.broken_out {
        b.boundary = scaled_boundary(&b.boundary, ratio, gap);
    }
}

fn rescale_geometry(geometry: &mut ViewGeometry, ratio: f64) {
    let gap = geometry.breaks.first().copied();
    let s = |p| {
        let q = scale_view_point(p, ratio, gap);
        Point2::new(round(q.u), round(q.v))
    };
    for c in &mut geometry.curves {
        c.path = scaled_path(&c.path, ratio, gap, Point2::default());
        c.ends = c.ends.map(|ends| ends.map(s));
    }
    for poly in &mut geometry.hatch {
        for p in poly.outer.iter_mut().chain(poly.holes.iter_mut().flatten()) {
            *p = s(*p);
        }
        if let Some(c) = &mut poly.clip {
            *c = scaled_boundary(c, ratio, gap);
        }
    }
    let [x0, y0, x1, y1] = geometry.bbox;
    let (a, b) = (s(Point2::new(x0, y0)), s(Point2::new(x1, y1)));
    geometry.bbox = [a.u, a.v, b.u, b.v];
    if let Some(c) = &mut geometry.outline {
        *c = scaled_boundary(c, ratio, gap);
    }
    for ends in &mut geometry.symmetry_segments {
        *ends = ends.map(s);
    }
    if let Some(g) = geometry.breaks.first_mut() {
        g.omitted_mm = g.omitted_mm.map(|width| width * ratio);
        g.at = g.at * ratio - (1.0 - ratio) * g.gap * g.centre_fraction;
    }
}

pub(crate) fn map_el(el: kurbo::PathEl, f: impl Fn(Point) -> Point) -> kurbo::PathEl {
    use kurbo::PathEl;
    match el {
        PathEl::MoveTo(p) => PathEl::MoveTo(f(p)),
        PathEl::LineTo(p) => PathEl::LineTo(f(p)),
        PathEl::QuadTo(a, b) => PathEl::QuadTo(f(a), f(b)),
        PathEl::CurveTo(a, b, c) => PathEl::CurveTo(f(a), f(b), f(c)),
        PathEl::ClosePath => PathEl::ClosePath,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::drawing::StandardView;

    fn frame(v: StandardView) -> ViewFrame {
        ViewSpec::standard(v).frame().unwrap()
    }

    fn same(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 1e-12
    }

    fn ring(points: &[[f64; 2]]) -> Vec<Point2> {
        points.iter().copied().map(Point2::from).collect()
    }

    /// A ring's corners counterclockwise from its lowest, then leftmost, corner, with
    /// points along a straight side left out.
    fn corners(ring: &[Point2]) -> Vec<[f64; 2]> {
        let n = ring.len();
        let mut out: Vec<Point2> = (0..n)
            .filter(|&i| {
                let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
                ((b.u - a.u) * (c.v - b.v) - (b.v - a.v) * (c.u - b.u)).abs() > 1e-9
            })
            .map(|i| ring[i])
            .collect();
        let twice: f64 = (0..out.len())
            .map(|i| {
                let (a, b) = (out[i], out[(i + 1) % out.len()]);
                a.u * b.v - b.u * a.v
            })
            .sum();
        if twice < 0.0 {
            out.reverse();
        }
        let first = (0..out.len())
            .min_by(|&i, &j| out[i].v.total_cmp(&out[j].v).then(out[i].u.total_cmp(&out[j].u)))
            .unwrap_or(0);
        out.rotate_left(first);
        out.into_iter().map(|p| [(p.u * 1e6).round() / 1e6, (p.v * 1e6).round() / 1e6]).collect()
    }

    fn square_with_hole(hole: [[f64; 2]; 2]) -> Polygon {
        let [[u0, v0], [u1, v1]] = hole;
        Polygon {
            outer: ring(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]),
            holes: vec![ring(&[[u0, v0], [u0, v1], [u1, v1], [u1, v0]])],
            clip: None,
            approximate: false,
        }
    }

    #[test]
    fn a_hole_the_clip_opens_becomes_a_notch_in_the_outer_boundary() {
        let window = ring(&[[-100.0, -100.0], [5.0, -100.0], [5.0, 100.0], [-100.0, 100.0]]);
        let clipped = clip_polygon_region(square_with_hole([[4.0, 4.0], [6.0, 6.0]]), &window, true);
        let [only] = &clipped[..] else { panic!("one region: {clipped:?}") };
        assert!(only.holes.is_empty(), "the opened hole is no longer a hole: {only:?}");
        assert_eq!(
            corners(&only.outer),
            [[0.0, 0.0], [5.0, 0.0], [5.0, 4.0], [4.0, 4.0], [4.0, 6.0], [5.0, 6.0], [5.0, 10.0], [0.0, 10.0]]
        );
    }

    #[test]
    fn a_clip_across_a_hole_leaves_separate_islands() {
        let window = ring(&[[4.0, -100.0], [6.0, -100.0], [6.0, 100.0], [4.0, 100.0]]);
        let clipped = clip_polygon_region(square_with_hole([[2.0, 3.0], [8.0, 7.0]]), &window, true);
        let mut islands: Vec<Vec<[f64; 2]>> = clipped
            .iter()
            .map(|p| {
                assert!(p.holes.is_empty(), "{p:?}");
                corners(&p.outer)
            })
            .collect();
        islands.sort_by(|a, b| a[0][1].total_cmp(&b[0][1]));
        assert_eq!(
            islands,
            [
                vec![[4.0, 0.0], [6.0, 0.0], [6.0, 3.0], [4.0, 3.0]],
                vec![[4.0, 7.0], [6.0, 7.0], [6.0, 10.0], [4.0, 10.0]]
            ]
        );
        // A hole wholly inside the window stays a hole.
        let inside = ring(&[[-100.0, -100.0], [100.0, -100.0], [100.0, 100.0], [-100.0, 100.0]]);
        let clipped = clip_polygon_region(square_with_hole([[2.0, 3.0], [8.0, 7.0]]), &inside, true);
        let [whole] = &clipped[..] else { panic!("one region: {clipped:?}") };
        assert_eq!(corners(&whole.outer), [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
        let [hole] = &whole.holes[..] else { panic!("one hole: {whole:?}") };
        assert_eq!(corners(hole), [[2.0, 3.0], [8.0, 3.0], [8.0, 7.0], [2.0, 7.0]]);
    }

    #[test]
    fn first_angle_folds_put_views_opposite_the_face_they_show() {
        let front = frame(StandardView::Front);
        let (d, u) = fold(&front, FoldDirection::Down, Projection::FirstAngle);
        let top = frame(StandardView::Top);
        assert!(same(d, top.direction) && same(u, top.up), "below front is the top view");
        let (d, u) = fold(&front, FoldDirection::Right, Projection::FirstAngle);
        let left = frame(StandardView::Left);
        assert!(same(d, left.direction) && same(u, left.up), "right of front is the left view");
    }

    #[test]
    fn third_angle_folds_put_views_on_the_side_of_the_face_they_show() {
        let front = frame(StandardView::Front);
        let (d, u) = fold(&front, FoldDirection::Up, Projection::ThirdAngle);
        let top = frame(StandardView::Top);
        assert!(same(d, top.direction) && same(u, top.up));
        let (d, _) = fold(&front, FoldDirection::Right, Projection::ThirdAngle);
        assert!(same(d, frame(StandardView::Right).direction));
        let (d, u) = fold(&front, FoldDirection::Down, Projection::ThirdAngle);
        let bottom = frame(StandardView::Bottom);
        assert!(same(d, bottom.direction) && same(u, bottom.up));
        let (d, _) = fold(&front, FoldDirection::UpRight, Projection::ThirdAngle);
        assert!(same(d, frame(StandardView::Isometric).direction), "up and right of front is the isometric");
    }

    #[test]
    fn circular_paths_are_approximate_and_mesh_polylines_are_polygon() {
        let a = Vec3::ZERO;
        let b = Vec3::new(10.0, 0.0, 0.0);
        let frame = ViewSpec::new(Vec3::Z, Vec3::Y).frame().unwrap();
        let xf = ViewTransform::new(frame, 1.0, Point2::new(0.0, 0.0));
        let accuracy = |curve: &Curve3| accuracy_of(curve).for_path(&path_of(curve, &xf));
        assert_eq!(accuracy(&Curve3::Line { a, b }), Accuracy::Exact);
        assert_eq!(accuracy(&Curve3::Circle { center: a, normal: Vec3::Z, radius: 5.0 }), Accuracy::Approximate);
        let arc =
            Curve3::Arc { center: a, normal: Vec3::Z, x_axis: Vec3::X, radius: 5.0, start_deg: 0.0, end_deg: 90.0 };
        assert_eq!(accuracy(&arc), Accuracy::Approximate);
        assert_eq!(accuracy(&Curve3::Polyline { points: vec![a, b] }), Accuracy::Polygon);
        assert_eq!(Accuracy::default(), Accuracy::Polygon, "a file without the label makes the weaker claim");
    }

    #[test]
    fn circles_project_to_exact_ellipses() {
        let spec = ViewSpec::standard(StandardView::Isometric);
        let xf = ViewTransform::new(spec.frame().unwrap(), 2.0, Point2::new(0.0, 0.0));
        let circle = Curve3::Circle { center: Vec3::ZERO, normal: Vec3::Z, radius: 10.0 };
        let path = path_of(&circle, &xf);
        // Every point of the projected path, taken back to the circle's plane, lies on the
        // circle within the cubic approximation's error, about 2.7e-4 of the radius.
        let f = xf.frame;
        let (a, b, c, d) = (f.right.x * 2.0, f.right.y * 2.0, f.up.x * 2.0, f.up.y * 2.0);
        let det = a * d - b * c;
        let mut worst: f64 = 0.0;
        kurbo::flatten(path.iter(), 1e-4, |el| {
            if let kurbo::PathEl::LineTo(p) | kurbo::PathEl::MoveTo(p) = el {
                let (x, y) = ((d * p.x - b * p.y) / det, (-c * p.x + a * p.y) / det);
                worst = worst.max((x.hypot(y) - 10.0).abs());
            }
        });
        assert!(worst < 10.0 * 3e-4, "{worst}");
    }
}
