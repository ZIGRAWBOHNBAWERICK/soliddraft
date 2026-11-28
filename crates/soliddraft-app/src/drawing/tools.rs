//! The phase D3 canvas tools, each a small state machine over the session's
//! [`ToolDraft`](super::ToolDraft): section, detail, auxiliary, crop and broken-out on
//! views; ordinate, baseline and chain dimension sets; geometric tolerance, datum,
//! surface finish and revision symbols. The views and symbols are made by
//! `soliddraft-drawing` (`derived`, `annotate`'s rules); the tools only collect the
//! clicks, preview the result and push the commands.

use super::canvas::{Hit, Sheet2, card, ghost_annotation, ghost_view, to_view, view_at};
use super::part::PartContext;
use super::{CutMode, DrawingAction, DrawingSel, DrawingSession, DrawingTool, PendingDerived, SHEET};
use crate::document::Document;
use crate::state::AppState;
use crate::theme;
use egui::{Stroke, vec2};
use soliddraft_drawing::display::{self as layout, Owner};
use soliddraft_drawing::measure::{CurveShape, precision_for, shape_of, shape_of_curve};
use soliddraft_drawing::project::{self, view_label_in};
use soliddraft_drawing::{
    AlignAxis, Anchor, Annotation, AnnotationKind, AnnotationState, Attach, Boundary, Break, BrokenOut, Command,
    CurveKind, DimType, DimensionSet, DrawingError, EntityRef, HalfSection, LinearOrientation, Part, PartIndex,
    SetKind, StyleOverrides, ViewId, ViewKind, derived,
};
use soliddraft_kernel_api::{Point2, Vec3};

/// Sheet millimetres a click snaps to a circle's centre from, at the current zoom.
fn snap_mm(s: &DrawingSession) -> f64 {
    f64::from(8.0 / s.camera.ppm.max(0.05))
}

/// Runs `f` with the part and its index, or says why it cannot.
pub(super) fn with_part<R>(
    doc: &Document,
    f: impl FnOnce(&Part<'_>, &PartIndex<'_>) -> Result<R, DrawingError>,
) -> Result<R, String> {
    let ctx = PartContext::new(doc).ok_or("The part has no body; extrude something first")?;
    let part = ctx.part();
    let index = PartIndex::build(&part).map_err(|e| e.to_string())?;
    f(&part, &index).map_err(|e| e.to_string())
}

pub(super) fn cache_picking(s: &mut DrawingSession, doc: &Document, view: ViewId) -> bool {
    let Some(saved) = s.doc.drawing.view(view) else { return false };
    if !s.model_is(doc.model().id()) || saved.geometry.source_digest != super::part::digest(doc).as_str() {
        return false;
    }
    if s.doc.transforms.contains_key(&view) {
        return true;
    }
    if let Ok(projected) = with_part(doc, |part, index| {
        let mut ctx = derived::context(&s.doc.drawing, part, index, view)?;
        project::match_saved_circle_sources(index, &saved.geometry, &mut ctx.projected.circles);
        Ok(ctx.projected)
    }) {
        s.doc.remember(view, &projected);
        true
    } else {
        false
    }
}

/// The view a click lands in: the hit curve's view, or the view whose bounds hold the
/// point.
fn view_of_click(s: &DrawingSession, hit: Option<Hit>, p: Point2) -> Option<ViewId> {
    match hit.map(|h| h.owner) {
        Some(Owner::Curve(v, _)) | Some(Owner::View(v)) => Some(v),
        _ => view_at(s, p),
    }
}

/// A point in a view's millimetres, moved onto the nearest face-on circle's centre
/// when one is within reach.
fn snap_centre(s: &DrawingSession, view: ViewId, p: Point2) -> Point2 {
    let reach = snap_mm(s);
    s.doc
        .circles
        .get(&view)
        .and_then(|cs| {
            cs.iter()
                .map(|c| (c.centre, (c.centre.u - p.u).hypot(c.centre.v - p.v)))
                .filter(|(_, d)| *d <= reach)
                .min_by(|a, b| a.1.total_cmp(&b.1))
        })
        .map(|(c, _)| c)
        .unwrap_or(p)
}

/// The anchor a pick on a curve makes: the snapped end, a round's centre, or the
/// curve's middle.
pub(super) fn anchor_from_hit(
    s: &mut DrawingSession,
    doc: &Document,
    hit: Hit,
    pointer: Point2,
) -> Option<(ViewId, Anchor)> {
    let Owner::Curve(view, index) = hit.owner else { return None };
    let v = s.sheet().view(view)?;
    let broken = !v.geometry.breaks.is_empty();
    let c = v.geometry.curves.get(index)?;
    let pointer = to_view(s, view, pointer);
    let measures = matches!(s.tool, DrawingTool::DimensionSet(_));
    if broken && !measures {
        let xf = v.geometry.transform.as_ref().or_else(|| s.doc.transforms.get(&view))?;
        let anchor = if c.kind == CurveKind::Edge {
            with_part(doc, |part, index| {
                if !s.model_is(doc.model().id()) || part.digest.as_str() != v.geometry.source_digest {
                    return Ok(None);
                }
                Ok(soliddraft_drawing::measure::retained_leader(c, xf, Some(index), pointer))
            })
            .ok()??
        } else {
            soliddraft_drawing::measure::retained_leader(c, xf, None, pointer)?
        };
        return Some((view, anchor));
    }
    let model = measures && (broken || c.source.section_leg != 0);
    let current = measures && cache_picking(s, doc, view);
    if model && !current {
        return None;
    }
    let v = s.sheet().view(view)?;
    let c = v.geometry.curves.get(index)?;
    let circles = s.doc.circles.get(&view).map(Vec::as_slice).unwrap_or_default();
    let shape = shape_of_curve(c, circles);
    // A circle's start is where its path begins, not a corner: the centre wins.
    let (attach, at) = match (&shape, hit.end) {
        (CurveShape::Circle { centre, .. }, _) => (Attach::Centre, *centre),
        (_, Some(end)) => (end, soliddraft_drawing::measure::endpoint(c, shape, end)),
        (_, None) if broken => (Attach::Mid, pointer),
        _ => (Attach::Mid, shape.mid()),
    };
    let mut anchor = Anchor { entity: c.source.clone(), attach, cached: at };
    if model
        && !with_part(doc, |_, index| {
            Ok(soliddraft_drawing::measure::place_model_pick(
                index,
                &s.doc.transforms[&view],
                &mut anchor,
                c,
                Some(pointer),
            ))
        })
        .unwrap_or(false)
    {
        return None;
    }
    Some((view, anchor))
}

/// The cutting line across a view through a point of it, horizontal or vertical,
/// reaching 6 mm past the line work.
fn cut_line(s: &DrawingSession, view: ViewId, through: Point2, horizontal: bool) -> Option<[Point2; 2]> {
    let [x0, y0, x1, y1] = s.doc.drawing.view(view)?.geometry.bbox;
    let m = 6.0;
    Some(if horizontal {
        [Point2::new(x0 - m, through.v), Point2::new(x1 + m, through.v)]
    } else {
        [Point2::new(through.u, y0 - m), Point2::new(through.u, y1 + m)]
    })
}

fn pending_of((view, ctx): (soliddraft_drawing::View, project::ViewContext), sheet: usize) -> PendingDerived {
    PendingDerived { view, projected: ctx.projected, sheet }
}

fn section_source(s: &DrawingSession, doc: &Document, parent: ViewId) -> Result<(), String> {
    s.doc.check_part(doc)?;
    let current = super::part::digest(doc);
    if !s.doc.drawing.view(parent).is_some_and(|view| view.geometry.source_digest == current.as_str()) {
        return Err("Rebuild the parent view before adding a cutting line".into());
    }
    Ok(())
}

pub(super) fn pending_view_source(
    s: &DrawingSession,
    doc: &Document,
    view: &soliddraft_drawing::View,
) -> Result<(), String> {
    if let ViewKind::Section { parent, .. } = view.kind {
        section_source(s, doc, parent)?;
    } else {
        s.doc.check_part(doc)?;
    }
    if view.geometry.source_digest != super::part::digest(doc).as_str() {
        return Err("Draw the view again from the current part".into());
    }
    Ok(())
}

fn compute_section(
    s: &DrawingSession,
    doc: &Document,
    sheet: usize,
    parent: ViewId,
    line: &[Point2],
    flip: bool,
) -> Result<PendingDerived, String> {
    section_source(s, doc, parent)?;
    with_part(doc, |part, index| {
        derived::new_section_along(&s.doc.drawing, part, index, sheet, parent, line, flip, None)
    })
    .map(|made| pending_of(made, sheet))
}

#[allow(clippy::too_many_arguments)]
fn compute_detail(
    s: &DrawingSession,
    doc: &Document,
    sheet: usize,
    parent: ViewId,
    center: Point2,
    radius: f64,
    profile: Vec<Point2>,
    factor: f64,
) -> Result<PendingDerived, String> {
    with_part(doc, |part, index| {
        derived::new_detail(&s.doc.drawing, part, index, sheet, parent, center, radius, profile, factor, None)
    })
    .map(|made| pending_of(made, sheet))
}

fn compute_auxiliary(
    s: &DrawingSession,
    doc: &Document,
    sheet: usize,
    parent: ViewId,
    edge: EntityRef,
    flip: bool,
) -> Result<PendingDerived, String> {
    s.doc.check_part(doc)?;
    with_part(doc, |part, index| derived::new_auxiliary(&s.doc.drawing, part, index, sheet, parent, edge, flip, None))
        .map(|made| pending_of(made, sheet))
}

/// Keeps a computed view for placing, or places it at once on another sheet.
fn take_pending(s: &mut DrawingSession, doc: &Document, pending: PendingDerived, push: &mut impl FnMut(DrawingAction)) {
    if pending.sheet != s.sheet {
        if let Err(error) = pending_view_source(s, doc, &pending.view) {
            s.say(error);
            return;
        }
        let id = pending.view.id;
        s.doc.remember(id, &pending.projected);
        push(DrawingAction::Command(Command::PlaceView { sheet: pending.sheet, view: pending.view.clone() }));
        push(DrawingAction::SetTool(DrawingTool::Select));
        push(DrawingAction::SetSheet(pending.sheet));
        push(DrawingAction::Select(Some(DrawingSel::View(id))));
        s.say("Placed on the other sheet; location labels point both ways");
        return;
    }
    s.draft.pending = Some(pending);
}

/// The preview stroke of a line being drawn.
fn preview_stroke() -> Stroke {
    Stroke::new(1.5, theme::accent())
}

fn redraw_on_parent_sheet(s: &mut DrawingSession, announce: bool) -> bool {
    let DrawingTool::Section { replace: Some(id) } = s.tool else { return true };
    let sheet =
        s.doc.drawing.view(id).and_then(|v| v.parent()).and_then(|parent| project::sheet_of(&s.doc.drawing, parent));
    if sheet == Some(s.sheet) {
        return true;
    }
    if announce {
        s.say("Switch to the parent view's sheet to redraw the cutting line");
    }
    false
}

/// The derived-view tools: section, detail, auxiliary, crop and broken-out.
#[allow(clippy::too_many_arguments)]
pub(super) fn derived_tool(
    s: &mut DrawingSession,
    doc: &Document,
    sheet2: &Sheet2<'_>,
    lctx: &layout::Context<'_>,
    hit: Option<Hit>,
    pointer: Option<Point2>,
    ctrl: bool,
    clicked: bool,
    double_clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    if !redraw_on_parent_sheet(s, clicked) {
        return;
    }
    let Some(p) = pointer else {
        paint_draft(s, sheet2, None);
        return;
    };
    if let Some(pending) = s.draft.pending.clone() {
        place_pending(s, doc, sheet2, lctx, pending, p, ctrl, clicked, push);
        return;
    }
    if clicked {
        let target = match s.tool {
            DrawingTool::Section { replace: Some(id) } => s.doc.drawing.view(id).and_then(|v| v.parent()),
            DrawingTool::Section { .. } | DrawingTool::Detail | DrawingTool::HalfSection | DrawingTool::BrokenOut => {
                s.draft.view.or_else(|| view_of_click(s, hit, p))
            }
            _ => None,
        };
        if let Some(view) = target {
            cache_picking(s, doc, view);
        }
    }
    match s.tool.clone() {
        DrawingTool::Section { replace } => section_tool(s, doc, sheet2, hit, p, clicked, replace, push),
        DrawingTool::Detail => detail_tool(s, doc, sheet2, hit, p, clicked, double_clicked, push),
        DrawingTool::Auxiliary => auxiliary_tool(s, doc, hit, clicked, push),
        DrawingTool::Crop => crop_tool(s, sheet2, hit, p, clicked, push),
        DrawingTool::BrokenOut => broken_out_tool(s, sheet2, hit, p, clicked, push),
        DrawingTool::Break => break_tool(s, doc, sheet2, hit, p, clicked, push),
        DrawingTool::HalfSection => half_section_tool(s, doc, sheet2, hit, p, clicked, push),
        _ => {}
    }
}

/// Paints what the draft has so far: the cutting line, circle, profile or rectangle
/// being drawn, following the pointer.
fn paint_draft(s: &DrawingSession, sheet2: &Sheet2<'_>, pointer: Option<Point2>) {
    let Some(view) = s.draft.view else { return };
    let Some(v) = s.doc.drawing.view(view) else { return };
    let o = v.position;
    let at = |q: Point2| sheet2.s(Point2::new(q.u + o.u, q.v + o.v));
    let painter = sheet2.painter;
    let stroke = preview_stroke();
    let pv = pointer.map(|p| to_view(s, view, p));
    match (&s.tool, s.draft.points.as_slice()) {
        (DrawingTool::Section { .. }, pts) if s.draft.cut != CutMode::Straight && !pts.is_empty() => {
            let mut through = pts.to_vec();
            through.extend(pv);
            if let Some(xf) = s.doc.transforms.get(&view) {
                use soliddraft_kernel_api::drawing::Curve3;
                let flat = xf.unbroken();
                let through: Vec<_> = through.into_iter().map(|q| flat.point(xf.unproject(q))).collect();
                let points = bent_line(s.draft.cut, &through).into_iter().map(|q| flat.unproject(q)).collect();
                let path = project::path_of(&Curve3::Polyline { points }, xf);
                for segment in path.segments() {
                    if let kurbo::PathSeg::Line(line) = segment {
                        let ends = [line.p0, line.p1].map(|p| at(Point2::new(p.x, p.y)));
                        painter.extend(egui::Shape::dashed_line(&ends, stroke, 8.0, 3.0));
                    }
                }
            }
            for q in pts {
                painter.circle_filled(at(*q), 4.0, theme::accent());
            }
        }
        (DrawingTool::Break, [start]) => {
            if let Some(q) = pv {
                let [x0, y0, x1, y1] = v.geometry.bbox;
                let ends = if (q.u - start.u).abs() >= (q.v - start.v).abs() {
                    [[start.u, y0, start.u, y1], [q.u, y0, q.u, y1]]
                } else {
                    [[x0, start.v, x1, start.v], [x0, q.v, x1, q.v]]
                };
                for [a, b, c, d] in ends {
                    painter.extend(egui::Shape::dashed_line(
                        &[at(Point2::new(a, b)), at(Point2::new(c, d))],
                        stroke,
                        6.0,
                        3.0,
                    ));
                }
            }
            painter.circle_filled(at(*start), 4.0, theme::accent());
        }
        (DrawingTool::HalfSection, [start]) => {
            if let Some(q) = pv {
                painter.extend(egui::Shape::dashed_line(&[at(*start), at(q)], stroke, 10.0, 3.0));
            }
            painter.circle_filled(at(*start), 4.0, theme::accent());
        }
        (DrawingTool::Section { .. }, [start]) => {
            if let Some(line) = cut_line(s, view, *start, s.draft.horizontal) {
                painter.extend(egui::Shape::dashed_line(&[at(line[0]), at(line[1])], stroke, 8.0, 3.0));
            }
            painter.circle_filled(at(*start), 4.0, theme::accent());
        }
        (DrawingTool::Detail, [centre]) if !s.draft.profile => {
            if let Some(q) = pv {
                let r = ((q.u - centre.u).hypot(q.v - centre.v) * f64::from(s.camera.ppm)) as f32;
                painter.circle_stroke(at(*centre), r.max(1.0), stroke);
            }
            painter.circle_filled(at(*centre), 4.0, theme::accent());
        }
        (DrawingTool::Detail, pts) if s.draft.profile && !pts.is_empty() => {
            let mut poly: Vec<egui::Pos2> = pts.iter().map(|q| at(*q)).collect();
            if let Some(q) = pv {
                poly.push(at(q));
            }
            painter.add(egui::Shape::line(poly, stroke));
            for q in pts {
                painter.circle_filled(at(*q), 3.5, theme::accent());
            }
            if pts.len() >= 3 {
                painter.circle_stroke(at(pts[0]), 7.0, Stroke::new(1.0, theme::accent()));
            }
        }
        (DrawingTool::Crop, [a]) => {
            if let Some(q) = pv {
                let r = egui::Rect::from_two_pos(at(*a), at(q));
                painter.extend(egui::Shape::dashed_line(
                    &[r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()],
                    stroke,
                    6.0,
                    3.0,
                ));
            }
            painter.circle_filled(at(*a), 4.0, theme::accent());
        }
        (DrawingTool::BrokenOut, [centre]) => {
            if let Some(q) = pv {
                let r = ((q.u - centre.u).hypot(q.v - centre.v) * f64::from(s.camera.ppm)) as f32;
                painter.circle_stroke(at(*centre), r.max(1.0), stroke);
            }
            painter.circle_filled(at(*centre), 4.0, theme::accent());
        }
        _ => {}
    }
}

/// The ghost of a computed view at the pointer, along its alignment unless Ctrl is
/// held; a click places it.
#[allow(clippy::too_many_arguments)]
fn place_pending(
    s: &mut DrawingSession,
    doc: &Document,
    sheet2: &Sheet2<'_>,
    lctx: &layout::Context<'_>,
    mut pending: PendingDerived,
    p: Point2,
    ctrl: bool,
    clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    let parent = pending.view.parent().and_then(|id| s.doc.drawing.view(id).cloned());
    pending.view.position = p;
    match (pending.view.alignment, &parent) {
        (Some(al), Some(pv)) if !ctrl => match al.axis {
            AlignAxis::Horizontal => pending.view.position.v = pv.position.v,
            AlignAxis::Vertical => pending.view.position.u = pv.position.u,
        },
        _ => pending.view.alignment = None,
    }
    ghost_view(s, sheet2, lctx, &pending.view, pending.sheet);
    // Alignment guides, as when a projected view is placed.
    if let (Some(al), Some(pv)) = (pending.view.alignment, &parent) {
        let [x0, y0, x1, y1] = project::view_bounds(pv);
        let child = pending.view.position;
        let stroke = Stroke::new(1.0, theme::accent().gamma_multiply(0.8));
        let lines: Vec<(Point2, Point2)> = match al.axis {
            AlignAxis::Horizontal => {
                let (a, b) = if child.u > pv.position.u { (x1, child.u + 40.0) } else { (child.u - 40.0, x0) };
                vec![(Point2::new(a, y0), Point2::new(b, y0)), (Point2::new(a, y1), Point2::new(b, y1))]
            }
            AlignAxis::Vertical => {
                let (a, b) = if child.v > pv.position.v { (y1, child.v + 40.0) } else { (child.v - 40.0, y0) };
                vec![(Point2::new(x0, a), Point2::new(x0, b)), (Point2::new(x1, a), Point2::new(x1, b))]
            }
        };
        for (a, b) in lines {
            sheet2.painter.extend(egui::Shape::dashed_line(&[sheet2.s(a), sheet2.s(b)], stroke, 6.0, 4.0));
        }
    }
    let name = view_label_in(&s.doc.drawing, s.sheet(), &pending.view);
    let line2 = match (&pending.view.alignment, &parent) {
        (Some(_), Some(pv)) => format!("Aligned with {} · Ctrl frees it", s.doc.view_name(pv.id).unwrap_or_default()),
        (None, Some(pv)) if !ctrl => format!("Free · from {}", s.doc.view_name(pv.id).unwrap_or_default()),
        _ => "Free (Ctrl held)".to_owned(),
    };
    card(sheet2.painter, sheet2.s(pending.view.position) + vec2(30.0, -30.0), &name, &line2);
    if clicked {
        if let Err(error) = pending_view_source(s, doc, &pending.view) {
            s.say(error);
            return;
        }
        let id = pending.view.id;
        s.doc.remember(id, &pending.projected);
        push(DrawingAction::Command(Command::PlaceView { sheet: pending.sheet, view: pending.view.clone() }));
        if let (ViewKind::Section { .. }, Some(hatch)) = (&pending.view.kind, s.draft.hatch.clone()) {
            let mut kind = pending.view.kind.clone();
            if let ViewKind::Section { hatch: h, .. } = &mut kind {
                *h = Some(hatch);
            }
            push(DrawingAction::Command(Command::EditView { view: id, kind }));
        }
        push(DrawingAction::SetTool(DrawingTool::Select));
        push(DrawingAction::Select(Some(DrawingSel::View(id))));
        s.say(format!("{name} placed"));
        s.draft.pending = None;
    } else {
        s.draft.pending = Some(pending);
    }
}

#[allow(clippy::too_many_arguments)]
fn section_tool(
    s: &mut DrawingSession,
    doc: &Document,
    sheet2: &Sheet2<'_>,
    hit: Option<Hit>,
    p: Point2,
    clicked: bool,
    replace: Option<ViewId>,
    push: &mut impl FnMut(DrawingAction),
) {
    if s.draft.cut != CutMode::Straight {
        bent_section_tool(s, doc, sheet2, hit, p, clicked, replace, push);
        return;
    }
    match (s.draft.view, s.draft.points.first().copied()) {
        (Some(view), Some(start)) => {
            // The pointer picks the line's direction until the click finishes it.
            let q = to_view(s, view, p);
            let (du, dv) = ((q.u - start.u).abs(), (q.v - start.v).abs());
            if du.max(dv) > 1.0 {
                s.draft.horizontal = du >= dv;
            }
            paint_draft(s, sheet2, Some(p));
            if !clicked {
                return;
            }
            let Some(line) = cut_line(s, view, start, s.draft.horizontal) else { return };
            match replace {
                Some(section) => redraw_section(s, doc, section, &line, push),
                None => match compute_section(s, doc, s.sheet, view, &line, s.draft.flip) {
                    Ok(pending) => {
                        s.draft.points.clear();
                        take_pending(s, doc, pending, push);
                    }
                    Err(e) => {
                        s.say(format!("Section: {e}"));
                        s.draft.reset();
                    }
                },
            }
        }
        _ => {
            if !clicked {
                return;
            }
            // With a parent chosen beforehand, the line must be drawn on it.
            let Some(view) = section_parent(s, hit, p, replace) else { return };
            let start = snap_centre(s, view, to_view(s, view, p));
            s.draft.view = Some(view);
            s.draft.points = vec![start];
            s.say("Move for a horizontal or vertical line, click to finish it");
        }
    }
}

/// The view a section's cutting line is drawn on: the section's parent when redrawing,
/// otherwise the clicked view, which must not be a detail.
fn section_parent(s: &mut DrawingSession, hit: Option<Hit>, p: Point2, replace: Option<ViewId>) -> Option<ViewId> {
    let Some(view) =
        replace.and_then(|r| s.doc.drawing.view(r).and_then(|v| v.parent())).or_else(|| view_of_click(s, hit, p))
    else {
        s.say("Click on the view to cut");
        return None;
    };
    if matches!(s.doc.drawing.view(view).map(|v| &v.kind), Some(ViewKind::Detail { .. })) {
        s.say("A detail view cannot be sectioned; cut its parent");
        return None;
    }
    Some(view)
}

/// The cutting line through the clicked points in a view's millimetres: legs through
/// the centres joined by square jogs for an offset cut, running the way the first and
/// last centres lie furthest apart; the three points as they are for an aligned cut.
fn bent_line(cut: CutMode, points: &[Point2]) -> Vec<Point2> {
    match (cut, points) {
        (CutMode::Offset, [first, .., last]) => {
            derived::staggered_line(points, (last.u - first.u).abs() >= (last.v - first.v).abs())
        }
        _ => points.to_vec(),
    }
}

/// An offset or aligned cutting line: each click adds a point, circle centres snap. An
/// aligned line ends at its third point and an offset line ends with Finish.
#[allow(clippy::too_many_arguments)]
fn bent_section_tool(
    s: &mut DrawingSession,
    doc: &Document,
    sheet2: &Sheet2<'_>,
    hit: Option<Hit>,
    p: Point2,
    clicked: bool,
    replace: Option<ViewId>,
    push: &mut impl FnMut(DrawingAction),
) {
    paint_draft(s, sheet2, Some(p));
    if !clicked {
        return;
    }
    let view = match s.draft.view {
        Some(view) => view,
        None => {
            let Some(view) = section_parent(s, hit, p, replace) else { return };
            s.draft.view = Some(view);
            view
        }
    };
    let q = snap_centre(s, view, to_view(s, view, p));
    s.draft.points.push(q);
    match (s.draft.cut, s.draft.points.len()) {
        (CutMode::Aligned, 3) => finish_cut(s, doc, replace, push),
        (CutMode::Aligned, 1) => s.say("Click the pivot, where the two legs meet"),
        (CutMode::Aligned, _) => s.say("Click where the second leg goes; it turns into the plane of the first"),
        _ => s.say("Click the next centre; press Finish to end the line"),
    }
}

/// Makes the section along the offset or aligned line drawn so far.
fn finish_cut(s: &mut DrawingSession, doc: &Document, replace: Option<ViewId>, push: &mut impl FnMut(DrawingAction)) {
    let Some(view) = s.draft.view else { return };
    let points = s.draft.points.clone();
    let enough = match s.draft.cut {
        CutMode::Offset => points.len() >= 2,
        CutMode::Aligned => points.len() == 3,
        CutMode::Straight => false,
    };
    if !enough {
        s.say("Section: click at least two centres for an offset line, three points for an aligned one");
        return;
    }
    if let Err(error) = section_source(s, doc, view) {
        if replace.is_some() {
            s.draft.clear_view_picks();
        }
        s.say(error);
        return;
    }
    cache_picking(s, doc, view);
    let Some(xf) = s.doc.transforms.get(&view) else {
        if replace.is_some() {
            s.draft.clear_view_picks();
        }
        s.say("Rebuild the view before adding a cutting line");
        return;
    };
    let points: Vec<_> = points.into_iter().map(|p| xf.unbroken().point(xf.unproject(p))).collect();
    let line = match derived::extend_cutting_line(&s.doc.drawing, view, xf, bent_line(s.draft.cut, &points)) {
        Ok(line) => line,
        Err(e) => {
            s.say(format!("Section: {e}"));
            if replace.is_some() {
                s.draft.clear_view_picks();
            } else {
                s.draft.reset();
            }
            return;
        }
    };
    match replace {
        Some(section) => redraw_section(s, doc, section, &line, push),
        None => match compute_section(s, doc, s.sheet, view, &line, s.draft.flip) {
            Ok(pending) => {
                s.draft.points.clear();
                take_pending(s, doc, pending, push);
            }
            Err(e) => {
                s.say(format!("Section: {e}"));
                s.draft.reset();
            }
        },
    }
}

/// Finish, from the panel: ends the offset cutting line being drawn.
pub fn finish_cut_action(state: &mut AppState) {
    let doc = &state.doc;
    let Some(s) = state.drawing.session.as_mut() else { return };
    let DrawingTool::Section { replace } = s.tool else { return };
    if !redraw_on_parent_sheet(s, true) {
        return;
    }
    let mut actions = Vec::new();
    finish_cut(s, doc, replace, &mut |a| actions.push(a));
    for a in actions {
        state.push(crate::state::AppAction::Drawing(a));
    }
}

/// Two clicks on a view: the stretch between them along the way they lie furthest apart
/// is left out, and the view closes up to a gap.
#[allow(clippy::too_many_arguments)]
fn break_tool(
    s: &mut DrawingSession,
    doc: &Document,
    sheet2: &Sheet2<'_>,
    hit: Option<Hit>,
    p: Point2,
    clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    paint_draft(s, sheet2, Some(p));
    if !clicked {
        return;
    }
    let (Some(view), Some(start)) = (s.draft.view, s.draft.points.first().copied()) else {
        let Some(view) = view_of_click(s, hit, p) else {
            s.say("Click on the view to break");
            return;
        };
        if matches!(s.doc.drawing.view(view).map(|v| &v.kind), Some(ViewKind::Detail { .. })) {
            s.say("A detail view cannot be broken; break its parent");
            return;
        }
        s.draft.view = Some(view);
        s.draft.points = vec![to_view(s, view, p)];
        s.say("Click where the stretch to leave out ends");
        return;
    };
    let end = to_view(s, view, p);
    let axis =
        if (end.u - start.u).abs() >= (end.v - start.v).abs() { AlignAxis::Horizontal } else { AlignAxis::Vertical };
    // The clicks land on the view as drawn, also when the part has outgrown its break.
    let saved = s.doc.drawing.view(view).and_then(|view| view.geometry.transform);
    let ends = match saved.or_else(|| s.doc.transforms.get(&view).copied()) {
        Some(xf) => Ok((xf.unproject(start), xf.unproject(end))),
        None => with_part(doc, |part, index| {
            let xf = project::ViewContext::break_edit_transform(&s.doc.drawing, part, index, view)?;
            Ok((xf.unproject(start), xf.unproject(end)))
        }),
    };
    match ends {
        Ok((from, to)) => {
            let breaks = vec![Break { from, to, axis }];
            push(DrawingAction::Command(Command::SetViewBreaks { view, breaks }));
            push(DrawingAction::SetTool(DrawingTool::Select));
            push(DrawingAction::Select(Some(DrawingSel::View(view))));
            s.say("Break set");
        }
        Err(e) => s.say(format!("Break: {e}")),
    }
    s.draft.reset();
}

/// Two clicks on a model or projected view: the line through them is the symmetry axis.
/// One side of it is drawn in section; the view's properties flip which.
#[allow(clippy::too_many_arguments)]
fn half_section_tool(
    s: &mut DrawingSession,
    doc: &Document,
    sheet2: &Sheet2<'_>,
    hit: Option<Hit>,
    p: Point2,
    clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    paint_draft(s, sheet2, Some(p));
    if !clicked {
        return;
    }
    let (Some(view), Some(start)) = (s.draft.view, s.draft.points.first().copied()) else {
        let Some(view) = view_of_click(s, hit, p) else {
            s.say("Click on the view to half-section");
            return;
        };
        if !matches!(
            s.doc.drawing.view(view).map(|v| &v.kind),
            Some(ViewKind::Base { .. } | ViewKind::Projected { .. })
        ) {
            s.say("A half section is made on a model or projected view");
            return;
        }
        s.draft.view = Some(view);
        s.draft.points = vec![snap_centre(s, view, to_view(s, view, p))];
        s.say("Click a second point on the axis");
        return;
    };
    let end = snap_centre(s, view, to_view(s, view, p));
    if (end.u - start.u).hypot(end.v - start.v) < 1e-6 {
        s.say("Click a second point away from the first");
        return;
    }
    let axis = with_part(doc, |part, index| {
        let ctx = derived::context(&s.doc.drawing, part, index, view)?;
        let direction = ctx.xf.frame.direction;
        let depth = part.geometry.topology()?.bbox.center().dot(direction);
        Ok([start, end].map(|point| ctx.xf.unproject(point) + direction * depth))
    });
    match axis {
        Ok(axis) => {
            let half = Some(HalfSection { axis, flip: s.draft.flip });
            push(DrawingAction::Command(Command::SetHalfSection { view, half }));
            push(DrawingAction::SetTool(DrawingTool::Select));
            push(DrawingAction::Select(Some(DrawingSel::View(view))));
            s.say("Half section made");
        }
        Err(e) => s.say(format!("Half section: {e}")),
    }
    s.draft.reset();
}

fn redraw_section(
    s: &mut DrawingSession,
    doc: &Document,
    section: ViewId,
    line: &[Point2],
    push: &mut impl FnMut(DrawingAction),
) {
    let Some(ViewKind::Section { parent, label, .. }) = s.doc.drawing.view(section).map(|v| &v.kind) else {
        s.set_tool(DrawingTool::Select);
        return;
    };
    let model_line = section_source(s, doc, *parent).and_then(|()| {
        with_part(doc, |part, index| {
            let ctx = derived::context(&s.doc.drawing, part, index, *parent)?;
            Ok(line.iter().map(|q| ctx.xf.unproject(*q)).collect::<Vec<_>>())
        })
    });
    match model_line {
        Ok(model_line) => {
            let kind = ViewKind::Section {
                parent: *parent,
                line: line.to_vec(),
                flip: s.draft.flip,
                label: label.clone(),
                model_line,
                hatch: s.draft.hatch.clone(),
            };
            push(DrawingAction::Command(Command::EditView { view: section, kind }));
        }
        Err(e) => {
            s.say(format!("Section: {e}"));
            s.draft.clear_view_picks();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn detail_tool(
    s: &mut DrawingSession,
    doc: &Document,
    sheet2: &Sheet2<'_>,
    hit: Option<Hit>,
    p: Point2,
    clicked: bool,
    double_clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    paint_draft(s, sheet2, Some(p));
    let Some(view) = s.draft.view else {
        if !clicked {
            return;
        }
        let Some(view) = view_of_click(s, hit, p) else {
            s.say("Click on the view to magnify");
            return;
        };
        let centre = snap_centre(s, view, to_view(s, view, p));
        s.draft.view = Some(view);
        s.draft.points = vec![centre];
        return;
    };
    let q = to_view(s, view, p);
    if s.draft.profile {
        let pts = s.draft.points.clone();
        let near_first =
            pts.len() >= 3 && pts.first().is_some_and(|f| (f.u - q.u).hypot(f.v - q.v) <= snap_mm(s).max(2.0));
        if double_clicked || (clicked && near_first) {
            if pts.len() >= 3 {
                finish_detail(s, doc, push);
            } else {
                s.say("A profile needs at least three corners");
            }
        } else if clicked {
            s.draft.points.push(q);
        }
        return;
    }
    if clicked {
        let centre = s.draft.points[0];
        let radius = (q.u - centre.u).hypot(q.v - centre.v);
        if radius < 1.0 {
            s.say("Move further out for the circle's radius");
            return;
        }
        s.draft.points.push(q);
        finish_detail(s, doc, push);
    }
}

/// The sheet a detail goes to, adding one when the draft asks for a new sheet.
fn target_sheet(s: &mut DrawingSession) -> usize {
    match s.draft.target_sheet {
        Some(t) if t < s.doc.drawing.sheets.len() => t,
        Some(_) => {
            let template = s.doc.drawing.sheets[0].template.clone();
            let name = super::sheets::fresh_name(&s.doc.drawing);
            match s.doc.apply(Command::AddSheet { name, template }, None) {
                Ok(()) => s.doc.drawing.sheets.len() - 1,
                Err(e) => {
                    s.say(e);
                    s.sheet
                }
            }
        }
        None => s.sheet,
    }
}

/// Computes the detail the draft describes and keeps it for placing.
fn finish_detail(s: &mut DrawingSession, doc: &Document, push: &mut impl FnMut(DrawingAction)) {
    let Some(view) = s.draft.view else { return };
    if let Err(e) = s.doc.check_part(doc) {
        s.say(format!("Detail: {e}"));
        s.draft.reset();
        return;
    }
    let pts = s.draft.points.clone();
    let (centre, radius, profile) = if s.draft.profile {
        let n = pts.len() as f64;
        let c = Point2::new(pts.iter().map(|q| q.u).sum::<f64>() / n, pts.iter().map(|q| q.v).sum::<f64>() / n);
        let r = pts.iter().map(|q| (q.u - c.u).hypot(q.v - c.v)).fold(0.0, f64::max);
        (c, r.max(1.0), pts)
    } else {
        let c = pts[0];
        let q = pts[1];
        (c, (q.u - c.u).hypot(q.v - c.v), Vec::new())
    };
    let sheet = target_sheet(s);
    match compute_detail(s, doc, sheet, view, centre, radius, profile, s.draft.factor) {
        Ok(pending) => {
            s.draft.points.clear();
            take_pending(s, doc, pending, push);
        }
        Err(e) => {
            s.say(format!("Detail: {e}"));
            s.draft.reset();
        }
    }
}

fn auxiliary_tool(
    s: &mut DrawingSession,
    doc: &Document,
    hit: Option<Hit>,
    clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    if !clicked {
        return;
    }
    let Some(Hit { owner: Owner::Curve(view, index), .. }) = hit else {
        s.say("Click an edge of a view; the auxiliary view unfolds square to it");
        return;
    };
    let Some(c) = s.sheet().view(view).and_then(|v| v.geometry.curves.get(index)).cloned() else { return };
    if c.kind != CurveKind::Edge || c.source.kind != soliddraft_drawing::EntityKind::Edge {
        s.say("That is a silhouette, not an edge; click a model edge");
        return;
    }
    s.draft.view = Some(view);
    s.draft.anchors = vec![Anchor { entity: c.source.clone(), attach: Attach::Mid, cached: shape_of(&c.path).mid() }];
    match compute_auxiliary(s, doc, s.sheet, view, c.source.clone(), s.draft.flip) {
        Ok(pending) => take_pending(s, doc, pending, push),
        Err(e) => {
            s.say(format!("Auxiliary view: {e}"));
            s.draft.reset();
        }
    }
}

fn crop_tool(
    s: &mut DrawingSession,
    sheet2: &Sheet2<'_>,
    hit: Option<Hit>,
    p: Point2,
    clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    paint_draft(s, sheet2, Some(p));
    if !clicked {
        return;
    }
    match (s.draft.view, s.draft.points.first().copied()) {
        (Some(view), Some(a)) => {
            let b = to_view(s, view, p);
            if (a.u - b.u).abs() < 1.0 || (a.v - b.v).abs() < 1.0 {
                s.say("Make the rectangle bigger");
                return;
            }
            push(DrawingAction::Command(Command::SetViewCrop { view, crop: Some(Boundary::rectangle(a, b)) }));
            push(DrawingAction::SetTool(DrawingTool::Select));
            push(DrawingAction::Select(Some(DrawingSel::View(view))));
            s.say("View cropped; the view's properties remove the crop");
        }
        _ => {
            let Some(view) = view_of_click(s, hit, p) else {
                s.say("Click on the view to crop");
                return;
            };
            s.draft.view = Some(view);
            s.draft.points = vec![to_view(s, view, p)];
        }
    }
}

fn broken_out_tool(
    s: &mut DrawingSession,
    sheet2: &Sheet2<'_>,
    hit: Option<Hit>,
    p: Point2,
    clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    paint_draft(s, sheet2, Some(p));
    if !clicked {
        return;
    }
    match (s.draft.view, s.draft.points.first().copied()) {
        (Some(view), Some(center)) => {
            let q = to_view(s, view, p);
            let radius = (q.u - center.u).hypot(q.v - center.v);
            if radius < 1.0 {
                s.say("Move further out for the radius");
                return;
            }
            let boundary = Boundary::Circle { center, radius };
            push(DrawingAction::Command(Command::AddBrokenOut {
                view,
                broken_out: BrokenOut { boundary, depth: s.draft.depth },
            }));
            push(DrawingAction::SetTool(DrawingTool::Select));
            push(DrawingAction::Select(Some(DrawingSel::View(view))));
            s.say(format!("Broken out {} mm deep", s.draft.depth));
        }
        _ => {
            let Some(view) = view_of_click(s, hit, p) else {
                s.say("Click on the view to break out");
                return;
            };
            if matches!(s.doc.drawing.view(view).map(|v| &v.kind), Some(ViewKind::Detail { .. })) {
                s.say("Break out the parent; the detail shows it magnified");
                return;
            }
            s.draft.view = Some(view);
            s.draft.points = vec![snap_centre(s, view, to_view(s, view, p))];
        }
    }
}

/// Flips the direction of the section or auxiliary view being placed, or of the
/// section whose line is being redrawn.
pub fn flip_pending(state: &mut AppState) {
    let doc = &state.doc;
    let Some(s) = state.drawing.session.as_mut() else { return };
    s.draft.flip = !s.draft.flip;
    let Some(pending) = s.draft.pending.clone() else {
        s.say(if s.draft.flip { "The view will look the other way" } else { "Direction restored" });
        return;
    };
    let made = match &pending.view.kind {
        ViewKind::Section { parent, line, .. } if line.len() >= 2 => {
            compute_section(s, doc, pending.sheet, *parent, line, s.draft.flip)
        }
        ViewKind::Auxiliary { parent, edge, .. } => {
            compute_auxiliary(s, doc, pending.sheet, *parent, edge.clone(), s.draft.flip)
        }
        _ => return,
    };
    match made {
        Ok(p) => {
            s.draft.pending = Some(p);
            s.say("Flipped: the view looks the other way");
        }
        Err(e) => s.say(e),
    }
}

/// Closes the profile of the detail being drawn.
pub fn close_profile(state: &mut AppState) {
    let doc = &state.doc;
    let Some(s) = state.drawing.session.as_mut() else { return };
    if s.tool != DrawingTool::Detail || !s.draft.profile || s.draft.points.len() < 3 {
        return;
    }
    let mut actions = Vec::new();
    finish_detail(s, doc, &mut |a| actions.push(a));
    for a in actions {
        state.push(crate::state::AppAction::Drawing(a));
    }
}

/// The dimension set the picks make, placed at a point of the view: ordinate and
/// baseline sets measure from the first pick, a chain from pick to pick.
fn build_set(s: &DrawingSession, kind: SetKind, placement: Point2) -> Annotation {
    let anchors = s.draft.anchors.clone();
    let view = s.draft.view.unwrap_or(SHEET);
    let scale = project::view_scale(&s.doc.drawing, view).unwrap_or(1.0);
    let (us, vs): (Vec<f64>, Vec<f64>) = anchors
        .iter()
        .map(|a| {
            let at = s.doc.transforms.get(&view).map_or(a.cached, |xf| xf.unbroken().point(xf.unproject(a.cached)));
            (at.u, at.v)
        })
        .unzip();
    let spread = |c: &[f64]| {
        c.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - c.iter().cloned().fold(f64::INFINITY, f64::min)
    };
    // Points on one vertical line can only be dimensioned vertically, and the other
    // way round; otherwise the panel's choice holds.
    let orientation = if spread(&us) < 1e-6 && spread(&vs) > 1e-6 {
        LinearOrientation::Vertical
    } else if spread(&vs) < 1e-6 && spread(&us) > 1e-6 {
        LinearOrientation::Horizontal
    } else {
        s.draft.orientation
    };
    let along: Vec<f64> = match orientation {
        LinearOrientation::Vertical => vs.clone(),
        _ => us.clone(),
    };
    let values: Vec<f64> = match kind {
        SetKind::Chain => along.windows(2).map(|w| (w[1] - w[0]).abs() / scale).collect(),
        _ => along[1..].iter().map(|c| (c - along[0]).abs() / scale).collect(),
    };
    let values: Vec<f64> = values.into_iter().map(|v| (v * 1000.0).round() / 1000.0).collect();
    let precision = values.iter().map(|v| precision_for(*v)).max().unwrap_or(0);
    let first = anchors[0].cached;
    let placement = match orientation {
        LinearOrientation::Vertical => Point2::new(placement.u, first.v),
        _ => Point2::new(first.u, placement.v),
    };
    Annotation {
        id: s.doc.next_annotation_id(),
        view,
        body: AnnotationKind::DimensionSet(DimensionSet {
            kind,
            orientation,
            values,
            precision,
            dim_type: DimType::Projected,
        }),
        anchors,
        placement,
        style: StyleOverrides::default(),
        state: AnnotationState::Ok,
        source: None,
    }
}

/// Ordinate, baseline and chain dimensions: picks on the curves of one view, then a
/// click away from them places the set.
#[allow(clippy::too_many_arguments)]
pub(super) fn set_tool(
    s: &mut DrawingSession,
    doc: &Document,
    kind: SetKind,
    sheet2: &Sheet2<'_>,
    lctx: &layout::Context<'_>,
    hit: Option<Hit>,
    pointer: Option<Point2>,
    clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    // The picks so far, numbered.
    if let Some(view) = s.draft.view
        && let Some(v) = s.doc.drawing.view(view)
    {
        for (i, a) in s.draft.anchors.iter().enumerate() {
            let at = sheet2.s(Point2::new(a.cached.u + v.position.u, a.cached.v + v.position.v));
            sheet2.painter.circle_filled(at, 4.5, theme::accent());
            sheet2.painter.text(
                at + vec2(6.0, -6.0),
                egui::Align2::LEFT_BOTTOM,
                if i == 0 { "0".to_owned() } else { i.to_string() },
                theme::font_medium(11.0),
                theme::accent(),
            );
        }
    }
    let Some(p) = pointer else { return };
    let on_curve = matches!(hit, Some(Hit { owner: Owner::Curve(..), .. }));
    if s.draft.anchors.len() >= 2
        && !on_curve
        && let Some(view) = s.draft.view
    {
        let a = build_set(s, kind, to_view(s, view, p));
        ghost_annotation(s, sheet2, lctx, &a);
    }
    if !clicked {
        return;
    }
    match (on_curve, hit.and_then(|h| anchor_from_hit(s, doc, h, p))) {
        (true, Some((view, anchor))) => {
            if s.draft.view.is_some_and(|v| v != view) {
                s.say("Every point must be in the same view");
                return;
            }
            if s.draft
                .anchors
                .iter()
                .any(|a| (a.cached.u - anchor.cached.u).abs() < 1e-6 && (a.cached.v - anchor.cached.v).abs() < 1e-6)
            {
                s.say("That point is picked already");
                return;
            }
            s.draft.view = Some(view);
            s.draft.anchors.push(anchor);
            let n = s.draft.anchors.len();
            s.say(if n == 1 {
                "Origin picked · pick the points to dimension".to_owned()
            } else {
                format!(
                    "{} point{} · pick more, or click away from the edges to place",
                    n - 1,
                    if n == 2 { "" } else { "s" }
                )
            });
        }
        (true, None) => s.say("Pick edges with measurable model geometry"),
        _ => {
            if s.draft.anchors.len() < 2 {
                s.say("Pick the origin and at least one point first");
                return;
            }
            let Some(view) = s.draft.view else { return };
            let a = build_set(s, kind, to_view(s, view, p));
            let id = a.id;
            push(DrawingAction::Command(Command::AddAnnotation { annotation: a }));
            push(DrawingAction::Select(Some(DrawingSel::Annotation(id))));
            s.draft.anchors.clear();
            s.draft.view = None;
            s.say("Dimensions placed · pick the next origin, or Esc");
        }
    }
}

/// The body of the symbol a tool places, from the draft and the drawing's counters.
fn symbol_body(s: &DrawingSession) -> AnnotationKind {
    match s.tool {
        DrawingTool::Weld => AnnotationKind::Weld { size: None },
        DrawingTool::Datum => AnnotationKind::Datum {
            letter: if s.draft.datum.trim().is_empty() {
                s.doc.drawing.next_datum_label.clone()
            } else {
                s.draft.datum.trim().to_uppercase()
            },
        },
        DrawingTool::SurfaceFinish => s.draft.finish.body(),
        DrawingTool::RevisionSymbol => AnnotationKind::RevisionSymbol {
            revision: if s.draft.revision.trim().is_empty() {
                s.doc.drawing.meta.revision.clone()
            } else {
                s.draft.revision.trim().to_owned()
            },
        },
        _ => s.draft.gdt.body(),
    }
}

fn symbol_annotation(s: &DrawingSession, view: ViewId, anchors: Vec<Anchor>, placement: Point2) -> Annotation {
    Annotation {
        id: s.doc.next_annotation_id(),
        view,
        body: symbol_body(s),
        anchors,
        placement,
        style: StyleOverrides::default(),
        state: AnnotationState::Ok,
        source: None,
    }
}

/// Geometric tolerance, datum, surface finish and revision symbols: a click on an edge
/// anchors the symbol, the next click places it. A geometric tolerance or a revision
/// symbol clicked away from the edges stands free where it was clicked.
#[allow(clippy::too_many_arguments)]
pub(super) fn symbol_tool(
    s: &mut DrawingSession,
    doc: &Document,
    sheet2: &Sheet2<'_>,
    lctx: &layout::Context<'_>,
    hit: Option<Hit>,
    pointer: Option<Point2>,
    clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    let Some(p) = pointer else { return };
    if let (Some(view), Some(anchor)) = (s.draft.view, s.draft.anchors.first().cloned()) {
        let Some(v) = s.doc.drawing.view(view) else { return };
        let at = sheet2.s(Point2::new(anchor.cached.u + v.position.u, anchor.cached.v + v.position.v));
        sheet2.painter.circle_filled(at, 4.5, theme::accent());
        let a = symbol_annotation(s, view, vec![anchor], to_view(s, view, p));
        ghost_annotation(s, sheet2, lctx, &a);
        if clicked {
            let id = a.id;
            push(DrawingAction::Command(Command::AddAnnotation { annotation: a }));
            push(DrawingAction::SetTool(DrawingTool::Select));
            push(DrawingAction::Select(Some(DrawingSel::Annotation(id))));
            s.say("Placed · its properties edit the symbol");
        }
        return;
    }
    if !clicked {
        return;
    }
    match hit.and_then(|h| anchor_from_hit(s, doc, h, p)) {
        Some((view, anchor)) => {
            s.draft.view = Some(view);
            s.draft.anchors = vec![anchor];
            s.say("Click where the symbol goes");
        }
        None if matches!(hit, Some(Hit { owner: Owner::Curve(..), .. })) => {
            s.say("Pick edges with measurable model geometry")
        }
        None if matches!(s.tool, DrawingTool::Gdt | DrawingTool::RevisionSymbol) => {
            let view = view_at(s, p).unwrap_or(SHEET);
            let a = symbol_annotation(s, view, Vec::new(), to_view(s, view, p));
            let id = a.id;
            let cmd = if view == SHEET {
                Command::AddSheetAnnotation { sheet: s.sheet, annotation: a }
            } else {
                Command::AddAnnotation { annotation: a }
            };
            push(DrawingAction::Command(cmd));
            push(DrawingAction::SetTool(DrawingTool::Select));
            push(DrawingAction::Select(Some(DrawingSel::Annotation(id))));
        }
        None => s.say("Click an edge of the feature first"),
    }
}

// A projected outline can show two model edges at the same place. A chamfer
// may share its vertex with the farther edge, even when the nearer edge won the pick.
fn overlaps_picked_line(picked: &soliddraft_drawing::Curve2, other: &soliddraft_drawing::Curve2, at: Point2) -> bool {
    use kurbo::{PathEl, Point};

    let [PathEl::MoveTo(a), PathEl::LineTo(b)] = picked.path.elements() else { return false };
    let [PathEl::MoveTo(c), PathEl::LineTo(d)] = other.path.elements() else { return false };
    let direction = *b - *a;
    let length = direction.length();
    if length <= 1e-6 {
        return false;
    }
    let axis = direction / length;
    let (from, to) = (*c - *a, *d - *a);
    let along = axis.dot(Point::new(at.u, at.v) - *a).clamp(0.0, length);
    let (start, end) = (axis.dot(from), axis.dot(to));
    axis.cross(from).abs() <= 1e-6 && axis.cross(to).abs() <= 1e-6 && (start.min(end)..=start.max(end)).contains(&along)
}

/// Picks both ends of each straight edge so rebuilds can move the lines with the part.
#[allow(clippy::too_many_arguments)]
pub(super) fn line_annotation_tool(
    s: &mut DrawingSession,
    doc: &Document,
    sheet2: &Sheet2<'_>,
    lctx: &layout::Context<'_>,
    hit: Option<Hit>,
    pointer: Option<Point2>,
    clicked: bool,
    push: &mut impl FnMut(DrawingAction),
) {
    use soliddraft_drawing::measure::{AnchorGeometry, centreline_segments, chamfer};
    use soliddraft_drawing::{Dimension, DimensionKind, Entity, Resolution};
    use soliddraft_kernel_api::drawing::Curve3;

    let Some(p) = pointer else { return };
    let is_chamfer = s.tool == DrawingTool::Chamfer;
    let mut place = |s: &DrawingSession, view, body, placement, anchors: Vec<Anchor>| {
        let id = s.doc.next_annotation_id();
        let annotation = Annotation {
            id,
            view,
            body,
            anchors,
            placement,
            style: StyleOverrides::default(),
            state: AnnotationState::Ok,
            source: None,
        };
        ghost_annotation(s, sheet2, lctx, &annotation);
        if clicked {
            push(DrawingAction::Command(Command::AddAnnotation { annotation }));
            push(DrawingAction::SetTool(DrawingTool::Select));
            push(DrawingAction::Select(Some(DrawingSel::Annotation(id))));
        }
    };
    if let (Some(view), Some(dim)) = (s.draft.view, &s.draft.chamfer) {
        place(s, view, AnnotationKind::Dimension(dim.clone()), to_view(s, view, p), s.draft.anchors.clone());
        return;
    }
    if !clicked {
        return;
    }
    let Some(Hit { owner: Owner::Curve(view, curve), .. }) = hit else {
        s.say("Pick a visible straight model edge");
        return;
    };
    if s.draft.view.is_some_and(|picked| picked != view) {
        s.say("Pick both edges in the same view");
        return;
    }
    if !cache_picking(s, doc, view) {
        s.say("Rebuild the drawing with its part before picking an edge");
        return;
    }
    let Some(v) = s.sheet().view(view) else { return };
    let Some(curve) = v.geometry.curves.get(curve).filter(|c| c.visible && c.kind == CurveKind::Edge) else {
        s.say("Pick a visible straight model edge");
        return;
    };
    let Some(xf) = s.doc.transforms.get(&view) else { return };
    let Some(ctx) =
        PartContext::new(doc).filter(|ctx| s.model_is(ctx.id) && ctx.digest.as_str() == v.geometry.source_digest)
    else {
        s.say("Rebuild the drawing with its part before picking an edge");
        return;
    };
    let part = ctx.part();
    let Ok(index) = PartIndex::build(&part) else { return };
    // Where the view draws the edge: an aligned section turns its second leg's edges.
    let edge = |entity: &EntityRef| match index.resolve(entity) {
        Resolution::Found(Entity::Edge(id)) => {
            index.edge_curve(id).map(|curve| xf.curve_on_leg(entity.section_leg, curve))
        }
        _ => None,
    };
    let straight_ends = |curve: &soliddraft_drawing::Curve2| {
        let Some(Curve3::Line { a, b }) = edge(&curve.source) else { return None };
        Some([a, b])
    };
    let anchors_of = |curve: &soliddraft_drawing::Curve2, [a, b]: [Vec3; 2]| {
        [(Attach::Start, a), (Attach::End, b)].map(|(attach, at)| Anchor {
            entity: curve.source.clone(),
            attach,
            cached: xf.point(at),
        })
    };
    let Some([a, b]) = straight_ends(curve) else {
        s.say("Pick a visible straight model edge");
        return;
    };
    if is_chamfer && (b - a).dot(xf.frame.direction).abs() > 1e-6 {
        s.say("Use a profile view where the chamfer and adjoining edge are seen at full length");
        return;
    }
    let ends = anchors_of(curve, [a, b]);
    let [first, second] = s.draft.anchors.as_slice() else {
        s.draft.view = Some(view);
        s.draft.anchors.extend(ends);
        return;
    };
    let points = [first.cached, second.cached, ends[0].cached, ends[1].cached];
    if is_chamfer {
        let dimension = |ends: [Anchor; 2]| {
            let anchors = [first, second, &ends[0], &ends[1]];
            let curves = anchors.map(|anchor| edge(&anchor.entity));
            let geometry: [AnchorGeometry<'_>; 4] = std::array::from_fn(|i| AnchorGeometry {
                curve: curves[i].as_ref(),
                point: xf.point_on_leg(anchors[i].entity.section_leg, anchors[i].entity.fingerprint.point),
                attach: anchors[i].attach,
            });
            chamfer(&geometry, xf).map(|(size, angle)| (ends, size, angle))
        };
        let at = to_view(s, view, p);
        let selected = dimension(ends).or_else(|| {
            v.geometry
                .curves
                .iter()
                .filter(|other| other.visible && other.kind == CurveKind::Edge && other.source != curve.source)
                .filter(|other| overlaps_picked_line(curve, other, at))
                .find_map(|other| dimension(anchors_of(other, straight_ends(other)?)))
        });
        let Some((ends, size, angle)) = selected else {
            s.say("Pick a straight adjoining edge that meets the bevel at an acute angle");
            return;
        };
        let mut dim = Dimension::measured(DimensionKind::Chamfer, size, precision_for(size));
        dim.chamfer_angle = Some(angle);
        s.draft.chamfer = Some(dim);
        s.draft.anchors.extend(ends);
    } else if let Some([a, b]) = centreline_segments(points, v.geometry.breaks.first().copied()).next() {
        let middle = Point2::new((a.u + b.u) / 2.0, (a.v + b.v) / 2.0);
        let anchors = s.draft.anchors.iter().cloned().chain(ends).collect();
        place(s, view, AnnotationKind::Centreline, middle, anchors);
    } else {
        s.say("Pick a distinct parallel edge with overlapping length");
    }
}

/// Choose a source view, then place the table on the sheet.
pub(super) fn hole_table_tool(
    s: &mut DrawingSession,
    doc: &Document,
    hit: Option<Hit>,
    p: Point2,
    push: &mut impl FnMut(DrawingAction),
) {
    let chosen = s.draft.view;
    let view = chosen.or_else(|| match hit.map(|h| h.owner) {
        Some(Owner::Curve(id, _) | Owner::View(id)) => Some(id),
        _ => view_at(s, p),
    });
    let Some(view) = view else {
        s.say("Click a view showing holes face-on");
        return;
    };
    if s.sheet().tables.iter().any(|t| t.hole_view == Some(view)) {
        s.say("This view already has a hole table");
        return;
    }
    if let Err(error) = s.doc.check_view_part(doc, view) {
        s.say(error);
        return;
    }
    cache_picking(s, doc, view);
    let Some(projected) = s.doc.projected_of(view) else {
        s.say("Rebuild the drawing with its part before making a hole table");
        return;
    };
    let Some(ctx) = PartContext::new(doc) else {
        s.say("Rebuild the drawing with its part before making a hole table");
        return;
    };
    let holes = projected.hole_rows(&ctx);
    if holes.is_empty() {
        s.say("This view has no visible circular holes");
        return;
    }
    if chosen.is_none() {
        s.draft.view = Some(view);
        return;
    }
    let table = soliddraft_drawing::Table::hole(p, view, holes);
    let index = s.sheet().tables.len();
    push(DrawingAction::Command(Command::AddTable { sheet: s.sheet, table }));
    push(DrawingAction::SetTool(DrawingTool::Select));
    push(DrawingAction::Select(Some(DrawingSel::HoleTable(index))));
}
