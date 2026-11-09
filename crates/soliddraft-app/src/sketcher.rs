//! The sketcher: draws the open sketch on its plane in the viewport and turns
//! pointer input into entities, relations and dimensions. Everything here is
//! in screen space on top of the 3D view; the plane maps points both ways.
//!
//! Inference happens while the pointer moves: a point within reach snaps to an
//! existing point, the middle of a line or a curve, and a new line that is
//! nearly level or plumb with its start snaps exactly and gets the relation.
//! New entity points keep their snap relations. Radius placements use the snapped
//! coordinate and retain an on-circle relation when snapping to an existing point.

use crate::document::{Command, Document};
use crate::sketch::{
    Align, Constraint, ConstraintId, ConstraintKind, DistanceKind, Entity, EntityId, EntityKind, P2, PointRef, Sketch,
    SolveStatus, fmt_value, solve,
    tools::{End, Operation, polygon_corners},
};
use crate::sketch_tools;
use crate::state::{AppAction, DimEdit, SketchDrag, SketchSel, SketchSession, Snap, Tool, ToolPreview, TrimDrag};
use crate::theme;
use crate::viewport::camera::Camera;
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use glam::Vec3;
use soliddraft_kernel_api::Plane;
use soliddraft_sketch::inference::{self, InferenceOptions, SnapFilter, apply_snaps};
use std::ops::{Add, Sub};

mod ellipse;
mod rectangle;
mod slot;
mod spline;

const POINT_REACH: f32 = 8.0;
const CURVE_REACH: f32 = 6.0;
const ALIGN_REACH: f32 = 6.0;

/// Screen mapping of a sketch plane through the camera.
pub struct Frame<'a> {
    pub plane: Plane,
    pub camera: &'a Camera,
    pub rect: Rect,
}

fn gv(v: soliddraft_kernel_api::Vec3) -> Vec3 {
    Vec3::new(v.x as f32, v.y as f32, v.z as f32)
}

impl Frame<'_> {
    pub fn world(&self, p: P2) -> Vec3 {
        gv(self.plane.to_world(soliddraft_kernel_api::Point2::new(p.u, p.v)))
    }

    pub fn screen(&self, p: P2) -> Option<Pos2> {
        self.camera.project(self.world(p), self.rect)
    }

    /// Plane coordinates under a screen position.
    pub fn plane_at(&self, pos: Pos2) -> Option<P2> {
        let (o, d) = self.camera.ray(pos, self.rect);
        let n = gv(self.plane.normal);
        let denom = d.dot(n);
        if denom.abs() < 1e-6 {
            return None;
        }
        let origin = gv(self.plane.origin);
        let t = (origin - o).dot(n) / denom;
        let hit = o + d * t;
        let rel = hit - origin;
        let x = gv(self.plane.x_dir);
        let y = gv(self.plane.y_dir());
        Some(P2::new(rel.dot(x) as f64, rel.dot(y) as f64))
    }

    /// Screen pixels per millimetre at the plane origin.
    pub fn scale(&self) -> f32 {
        let a = self.screen(P2::ZERO);
        let b = self.screen(P2::new(1.0, 0.0));
        match (a, b) {
            (Some(a), Some(b)) => (b - a).length().max(1e-3),
            _ => 1.0,
        }
    }
}

fn dist_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = if ab.length_sq() < 1e-6 { 0.0 } else { ((p - a).dot(ab) / ab.length_sq()).clamp(0.0, 1.0) };
    (a + ab * t - p).length()
}

/// Distance in pixels from `pos` to an entity's curve.
fn entity_distance(frame: &Frame<'_>, e: &Entity, pos: Pos2) -> f32 {
    let poly: Vec<Pos2> = e.polyline(64).into_iter().filter_map(|p| frame.screen(p)).collect();
    if poly.len() == 1 {
        return (poly[0] - pos).length();
    }
    poly.windows(2).map(|w| dist_to_segment(pos, w[0], w[1])).fold(f32::INFINITY, f32::min)
}

/// What the pointer is over, in the sketch.
fn hit_test(frame: &Frame<'_>, sketch: &Sketch, pos: Pos2, glyphs: &[(ConstraintId, Rect)]) -> Option<SketchSel> {
    let mut best_point: Option<(f32, PointRef)> = None;
    for e in &sketch.entities {
        for i in 0..e.point_count() {
            let Some(sp) = e.point(i).and_then(|p| frame.screen(p)) else { continue };
            let d = (sp - pos).length();
            if d <= POINT_REACH && best_point.is_none_or(|(bd, _)| d < bd) {
                best_point = Some((d, PointRef::new(e.id, i)));
            }
        }
    }
    if let Some((_, r)) = best_point {
        return Some(SketchSel::Point(r));
    }
    for (id, r) in glyphs {
        if r.contains(pos) {
            return Some(SketchSel::Constraint(*id));
        }
    }
    let mut best: Option<(f32, EntityId)> = None;
    for e in sketch.drawn() {
        let d = entity_distance(frame, e, pos);
        if d <= CURVE_REACH && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, e.id));
        }
    }
    best.map(|(_, id)| SketchSel::Entity(id))
}

/// The curve under the pointer, ignoring points and relation glyphs.
fn ignore_guide_targets(s: &SketchSession) -> bool {
    s.trim_options.ignore_construction && (s.tool == Tool::Trim || (s.tool.bounded_trim() && s.trim_bounds.len() == 2))
}

fn hit_curve(frame: &Frame<'_>, sketch: &Sketch, pos: Pos2, ignore_construction: bool) -> Option<EntityId> {
    let mut best: Option<(f32, EntityId)> = None;
    for e in sketch.drawn().filter(|e| !e.is_point() && !(ignore_construction && e.construction)) {
        let d = entity_distance(frame, e, pos);
        if d <= POINT_REACH && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, e.id));
        }
    }
    best.map(|(_, id)| id)
}

fn commit(s: &SketchSession, actions: &mut Vec<AppAction>) {
    actions.push(AppAction::Command(Command::EditSketch { id: s.feature, sketch: s.sketch.clone() }));
}

pub fn finish_spline(s: &mut SketchSession, actions: &mut Vec<AppAction>) {
    mirrored_edit(s, actions, true, spline::finish);
}

/// Runs the sketcher for one frame: input first, then drawing.
pub fn run(
    ui: &mut egui::Ui,
    s: &mut SketchSession,
    doc: &Document,
    camera: &Camera,
    rect: Rect,
    resp: &egui::Response,
) -> Vec<AppAction> {
    let mut actions = Vec::new();
    let frame = Frame { plane: s.plane, camera, rect };
    let painter = ui.painter().with_clip_rect(rect);
    let glyphs = if s.show_relations { glyph_rects(&frame, &s.sketch) } else { Vec::new() };
    let hover_pos = resp.hover_pos().filter(|_| !resp.dragged());
    s.cursor = hover_pos.and_then(|p| frame.plane_at(p));

    // Hover and inference. A tool that places points infers at the pointer; one that
    // changes a curve finds the curve under it and previews the change; the rest pick.
    s.hover = None;
    s.snaps.clear();
    s.preview = None;
    let places = s.tool.places_points();
    let picks = s.tool.picks_curve();
    let copy_places = (matches!(s.tool, Tool::Mirror | Tool::LinearPattern | Tool::CircularPattern)
        || s.tool.transforms())
        && (s.tool != Tool::Mirror || s.modify.mirror_axis.is_none())
        && !sketch_tools::selected_in_order(s).is_empty()
        && !ui.input(|i| i.modifiers.ctrl || i.modifiers.shift);
    let mut snapped: Option<P2> = None;
    if let Some(pos) = hover_pos {
        if places || copy_places {
            let prev = s.pending.last().map(|(p, _)| *p);
            if let Some(raw) = frame.plane_at(pos) {
                let filter = s.quick_snap.unwrap_or(if s.auto_relations { SnapFilter::All } else { SnapFilter::None });
                let options = InferenceOptions {
                    filter,
                    directions: s.tool.draws_line(),
                    point_reach: f64::from(POINT_REACH),
                    curve_reach: f64::from(CURVE_REACH),
                    align_reach: f64::from(ALIGN_REACH),
                    min_length: 14.0,
                };
                let raw = if s.tool.rectangle() { rectangle::project_pick(s, raw) } else { raw };
                let (p, mut snaps) =
                    inference::infer(&s.sketch, raw, prev, options, |a, b| match (frame.screen(a), frame.screen(b)) {
                        (Some(a), Some(b)) => f64::from((a - b).length()),
                        _ => f64::INFINITY,
                    });
                let projected = if s.tool.rectangle() { rectangle::project_pick(s, p) } else { p };
                if projected.dist(p) >= 1e-6 {
                    snaps.clear();
                }
                let p = projected;
                let close = s.tool.spline_mode().is_some()
                    && s.pending.len() >= 3
                    && frame.screen(s.pending[0].0).is_some_and(|first| first.distance(pos) <= POINT_REACH);
                snapped = Some(if close { s.pending[0].0 } else { p });
                s.snaps = if close { Vec::new() } else { snaps };
            }
        } else if picks {
            s.hover = hit_curve(&frame, &s.sketch, pos, ignore_guide_targets(s)).map(SketchSel::Entity);
            s.preview = curve_preview(s, &frame, pos);
        } else {
            s.hover = hit_test(&frame, &s.sketch, pos, &glyphs);
            if s.hover.is_none()
                && let Some((id, _)) = label_rects(&frame, &s.sketch, ui).into_iter().find(|(_, r)| r.contains(pos))
            {
                s.hover = Some(SketchSel::Constraint(id));
            }
        }
    }
    if s.tool.selects() && s.tool != Tool::Select {
        if copy_places && let Some(p) = snapped {
            copy_value(s, p);
        }
        s.preview = sketch_tools::selection_preview(s);
    }
    if hover_pos.is_some() {
        let cursor =
            if places || picks || copy_places { egui::CursorIcon::Crosshair } else { egui::CursorIcon::Default };
        ui.ctx().set_cursor_icon(cursor);
    }

    let modifiers = ui.input(|i| i.modifiers);
    let clicked = resp.clicked_by(egui::PointerButton::Primary);
    let double = resp.double_clicked_by(egui::PointerButton::Primary);
    let secondary = resp.secondary_clicked();

    // Tool input.
    match s.tool {
        Tool::Select => select_input(ui, s, &frame, resp, clicked, double, modifiers, &mut actions),
        tool if tool.selects() => {
            copy_input(ui, s, &frame, resp, clicked, snapped);
            s.preview = sketch_tools::selection_preview(s);
        }
        Tool::Dimension => {
            if clicked && let Some(pos) = hover_pos {
                dimension_click(s, &frame, pos, &mut actions);
            }
        }
        Tool::TrimCorner | Tool::Fillet | Tool::Chamfer => {
            if clicked && let Some(pos) = hover_pos {
                corner_click(s, &frame, pos, &mut actions);
            }
        }
        Tool::Split => {
            if clicked && let Some(pos) = hover_pos {
                split_click(s, &frame, pos, &mut actions);
            }
        }
        Tool::TrimInside | Tool::TrimOutside => {
            if clicked && let Some(pos) = hover_pos {
                bounded_trim_click(s, &frame, pos, &mut actions);
            }
        }
        Tool::Trim | Tool::Extend => {
            let dragging = s.tool == Tool::Trim && trim_input(ui, s, &frame, resp, &mut actions);
            if clicked && !dragging {
                match s.preview.as_ref().and_then(|p| p.operation.clone()) {
                    Some(operation) => {
                        sketch_tools::apply(s, &operation, &mut actions);
                    }
                    None => {
                        let why = s.preview.as_ref().and_then(|p| p.refused.clone());
                        s.say(why.unwrap_or_else(|| "Point at a curve".to_owned()));
                    }
                }
            }
        }
        _ => {
            if (clicked || double)
                && let Some(p) = snapped
            {
                let snaps = s.snaps.clone();
                let finish = if s.tool.spline_mode().is_some() {
                    (double || resp.triple_clicked_by(egui::PointerButton::Primary))
                        && s.pending
                            .last()
                            .and_then(|(point, _)| frame.screen(*point))
                            .zip(hover_pos)
                            .is_some_and(|(last, cursor)| last.distance(cursor) <= POINT_REACH)
                } else {
                    double
                };
                tool_click(s, p, snaps, finish, &mut actions);
                s.quick_snap = None;
            }
        }
    }
    if secondary && s.tool.selects() && s.tool != Tool::Select {
        s.set_tool(Tool::Select);
    } else if secondary && s.tool != Tool::Select {
        // Right click ends the current chain, or puts the tool down.
        if s.pending.is_empty() {
            s.set_tool(Tool::Select);
        } else {
            s.pending.clear();
            s.chain_from = None;
            s.quick_snap = None;
        }
    }

    s.dynamic_preview = dynamic_preview(s, snapped);

    // Drawing.
    paint_plane(&painter, &frame, doc, &s.sketch);
    paint_regions(&painter, &frame, s);
    paint_entities(&painter, &frame, s);
    paint_glyphs(&painter, s, &glyphs);
    paint_dimensions(&painter, &frame, s);
    paint_rubber_band(&painter, &frame, s, snapped);
    for (kind, construction) in &s.dynamic_preview {
        let entity = Entity { id: 0, kind: kind.clone(), construction: *construction };
        let points: Vec<_> = entity.polyline(64).into_iter().filter_map(|p| frame.screen(p)).collect();
        if points.len() == 1 {
            painter.circle_stroke(points[0], 4.0, Stroke::new(1.5, theme::accent()));
        } else if *construction {
            painter.extend(egui::Shape::dashed_line(&points, Stroke::new(1.5, theme::accent()), 6.0, 4.0));
        } else if points.len() > 1 {
            painter.add(egui::Shape::line(points, Stroke::new(1.5, theme::accent())));
        }
    }
    if let Some(axis) = s.dynamic_mirror {
        painter.text(
            pos2(rect.left() + 18.0, rect.bottom() - 48.0),
            Align2::LEFT_BOTTOM,
            format!("Dynamic mirror · Line {axis} · Esc stops"),
            theme::font_semibold(11.0),
            theme::accent(),
        );
    }
    paint_tool_preview(&painter, &frame, s);
    paint_diagnostics(&painter, &frame, s);
    if let Some(drag) = &s.trim_drag
        && drag.extend.is_none()
    {
        let trail: Vec<Pos2> = drag.path.iter().filter_map(|p| frame.screen(*p)).collect();
        if trail.len() > 1 {
            painter.add(egui::Shape::line(trail, Stroke::new(1.5, theme::accent())));
        }
        for cut in &drag.checkpoints {
            if let Some(at) = frame.screen(cut.at) {
                painter.rect_filled(Rect::from_center_size(at, vec2(6.0, 6.0)), CornerRadius::ZERO, theme::error());
            }
        }
    }
    paint_snaps(&painter, &frame, s, hover_pos, snapped);
    dim_editor(ui, s, &frame, &mut actions);
    paint_status(ui, s, rect);
    crate::measure::guides::switches(ui, s, rect);
    actions
}

/// Sets the value that follows the pointer until its placement is complete.
fn copy_value(s: &mut SketchSession, p: P2) {
    match s.tool {
        Tool::Mirror if s.pending.len() == 1 => s.modify.axis_end = p,
        Tool::LinearPattern | Tool::Move | Tool::Copy if s.pending.len() == 1 => {
            s.modify.step = p.sub(s.pending[0].0);
        }
        Tool::CircularPattern if s.pending.is_empty() && s.modify.pattern_group.is_none() => s.modify.center = p,
        Tool::Rotate if s.pending.is_empty() => s.modify.center = p,
        Tool::Scale if s.pending.is_empty() => s.modify.base = p,
        _ => {}
    }
}

/// Signed distance from a selected curve, respecting the chain's walking direction.
fn offset_distance(s: &SketchSession, target: EntityId, p: P2) -> Option<f64> {
    if s.modify.offset.chain {
        let seed = *sketch_tools::selected_in_order(s).first()?;
        let chain = s.sketch.offset_chain_from(seed).ok()?;
        let (_, reverse) = chain.iter().find(|(id, _)| *id == target)?;
        let distance = match s.sketch.entity(target)?.kind {
            EntityKind::Line { a, b } => (p - a).dot((b - a).normalized().perp()),
            EntityKind::Arc { center, start, .. } => center.dist(start) - center.dist(p),
            EntityKind::Circle { center, radius } => radius - center.dist(p),
            _ => return None,
        };
        return Some(if *reverse { -distance } else { distance });
    }
    let mut previous: Option<P2> = None;
    for id in sketch_tools::selected_in_order(s) {
        let kind = s.sketch.entity(id)?.kind.clone();
        let (a, b) = match kind {
            EntityKind::Line { a, b } => (a, b),
            EntityKind::Arc { start, end, .. } => (start, end),
            EntityKind::Circle { center, radius } if id == target => return Some(radius - center.dist(p)),
            _ => return None,
        };
        let reverse = previous.is_some_and(|q| a.dist(q) > 1e-6 && b.dist(q) <= 1e-6);
        if id == target {
            let distance = match kind {
                EntityKind::Line { .. } => p.sub(a).dot(b.sub(a).normalized().perp()),
                EntityKind::Arc { center, .. } => center.dist(a) - center.dist(p),
                _ => unreachable!(),
            };
            return Some(if reverse { -distance } else { distance });
        }
        previous = Some(if reverse { a } else { b });
    }
    None
}

/// Copy tools select without moving the source. Ctrl/Shift keeps selecting after a seed
/// is picked; ordinary clicks then place the axis, step or centre.
fn copy_input(
    ui: &egui::Ui,
    s: &mut SketchSession,
    frame: &Frame<'_>,
    resp: &egui::Response,
    clicked: bool,
    snapped: Option<P2>,
) {
    if s.tool == Tool::Offset && s.modify.offset_group.is_some() {
        return;
    }

    let modifiers = ui.input(|i| i.modifiers);
    if s.tool == Tool::Offset {
        if resp.drag_started_by(egui::PointerButton::Primary)
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            let hit = hit_test(frame, &s.sketch, origin, &[]);
            let id = match hit {
                Some(SketchSel::Point(p)) => Some(p.entity),
                Some(SketchSel::Entity(id)) => Some(id),
                _ => None,
            };
            if let Some(entity) = id.filter(|id| sketch_tools::selected_in_order(s).contains(id)) {
                s.drag = Some(SketchDrag::Offset { entity });
            }
        }
        if let Some(SketchDrag::Offset { entity }) = s.drag
            && let Some(p) = resp.interact_pointer_pos().and_then(|pos| frame.plane_at(pos))
            && let Some(distance) = offset_distance(s, entity, p)
        {
            s.modify.distance = distance;
        }
        if resp.drag_stopped_by(egui::PointerButton::Primary) {
            s.drag = None;
        }
    }
    if !clicked {
        return;
    }
    if s.modify.pattern_group.is_some() && (modifiers.ctrl || modifiers.shift) {
        s.say("This pattern keeps its source selection; edit the sources or start a new pattern");
        return;
    }
    if modifiers.ctrl
        || modifiers.shift
        || sketch_tools::selected_in_order(s).is_empty()
        || s.tool == Tool::Offset
        || (s.tool == Tool::Mirror && s.modify.mirror_axis.is_some())
    {
        if let Some(h) = resp.hover_pos().and_then(|pos| hit_test(frame, &s.sketch, pos, &[])) {
            if modifiers.ctrl || modifiers.shift {
                let entity = |pick: &SketchSel| match pick {
                    SketchSel::Entity(id) => Some(*id),
                    SketchSel::Point(p) => Some(p.entity),
                    _ => None,
                };
                if s.selection.iter().any(|pick| entity(pick) == entity(&h)) {
                    s.selection.retain(|pick| entity(pick) != entity(&h));
                } else {
                    s.selection.push(h);
                }
            } else {
                s.selection = vec![h];
            }
            s.pending.clear();
        }
        return;
    }
    let Some(p) = snapped else { return };
    match s.tool {
        Tool::Mirror | Tool::LinearPattern | Tool::Move | Tool::Copy => {
            if s.pending.len() != 1 {
                s.pending.clear();
                s.pending.push((p, Vec::new()));
                if s.tool == Tool::Mirror {
                    s.modify.axis_start = p;
                } else if matches!(s.tool, Tool::Move | Tool::Copy) {
                    s.modify.base = p;
                }
                copy_value(s, p);
            } else {
                copy_value(s, p);
                s.pending.push((p, Vec::new()));
            }
        }
        Tool::CircularPattern | Tool::Rotate => {
            s.modify.center = p;
            s.pending = vec![(p, Vec::new())];
        }
        Tool::Scale => {
            s.modify.base = p;
            s.pending = vec![(p, Vec::new())];
        }
        _ => {}
    }
}

fn split_click(s: &mut SketchSession, frame: &Frame<'_>, pos: Pos2, actions: &mut Vec<AppAction>) {
    if s.corner_first.is_none()
        && let Some(id) = hit_curve(frame, &s.sketch, pos, false)
        && s.sketch.entity(id).is_some_and(|e| matches!(e.kind, EntityKind::Circle { .. }))
        && let Some(at) = frame.plane_at(pos)
    {
        s.corner_first = Some((id, s.sketch.entity(id).expect("picked circle").nearest(at).0));
        s.selection = vec![SketchSel::Entity(id)];
        return;
    }
    if let Some(operation) = s.preview.as_ref().and_then(|p| p.operation.clone()) {
        if sketch_tools::apply(s, &operation, actions) {
            s.corner_first = None;
            s.selection.clear();
        }
    } else {
        s.say(
            s.preview
                .as_ref()
                .and_then(|p| p.refused.clone())
                .unwrap_or_else(|| "Point at a line, arc or circle".into()),
        );
    }
}

fn corner_click(s: &mut SketchSession, frame: &Frame<'_>, pos: Pos2, actions: &mut Vec<AppAction>) {
    if s.corner_first.is_none() {
        let first = hit_curve(frame, &s.sketch, pos, false).filter(|id| {
            s.sketch.entity(*id).is_some_and(|e| matches!(e.kind, EntityKind::Line { .. } | EntityKind::Arc { .. }))
        });
        if let (Some(id), Some(at)) = (first, frame.plane_at(pos)) {
            s.corner_first = Some((id, at));
            s.selection = vec![SketchSel::Entity(id)];
        } else {
            s.say("Pick a line or arc on the part to keep");
        }
        return;
    }
    match s.preview.as_ref().and_then(|p| p.operation.clone()) {
        Some(operation) => {
            if sketch_tools::apply(s, &operation, actions) {
                s.corner_first = None;
                s.selection.clear();
            }
        }
        None => s.say(
            s.preview.as_ref().and_then(|p| p.refused.clone()).unwrap_or_else(|| "Pick the second line or arc".into()),
        ),
    }
}

fn bounded_trim_click(s: &mut SketchSession, frame: &Frame<'_>, pos: Pos2, actions: &mut Vec<AppAction>) {
    if s.trim_bounds.len() < 2 {
        if let Some(id) = hit_curve(frame, &s.sketch, pos, false) {
            if s.trim_bounds.contains(&id) {
                s.say("Choose a different bounding curve");
            } else {
                s.trim_bounds.push(id);
                s.selection = s.trim_bounds.iter().copied().map(SketchSel::Entity).collect();
                s.preview = None;
            }
        } else {
            s.say("Pick a bounding line, arc or circle");
        }
        return;
    }
    match s.preview.as_ref().and_then(|p| p.operation.clone()) {
        Some(operation) => {
            sketch_tools::apply(s, &operation, actions);
        }
        None => s.say(
            s.preview
                .as_ref()
                .and_then(|p| p.refused.clone())
                .unwrap_or_else(|| "Pick an open target line or arc".into()),
        ),
    }
}

fn trim_segment(s: &mut SketchSession, drag: &mut TrimDrag, frame: &Frame<'_>, mut from: P2, to: P2) {
    while from.dist(to) > 1e-6 {
        let undo_at = drag.checkpoints.last().and_then(|cut| {
            let delta = to - from;
            if delta.dot(cut.direction) >= 0.0 {
                return None;
            }
            let t = (cut.at - from).dot(delta) / delta.dot(delta);
            let tolerance = 1e-6 / delta.len();
            // Undo at the marker's projection, not when a sample first enters its hit radius.
            if !(-tolerance..=1.0 + tolerance).contains(&t) {
                return None;
            }
            let p = from.lerp(to, t.clamp(0.0, 1.0));
            match (frame.screen(cut.at), frame.screen(p)) {
                (Some(a), Some(b)) if a.distance(b) <= 6.0 => Some(p),
                _ => None,
            }
        });
        let end = undo_at.unwrap_or(to);
        let result =
            if from.dist(end) > 1e-6 { s.sketch.trim_stroke(from, end, s.trim_options) } else { Ok(Vec::new()) };
        match result {
            Ok(cuts) if cuts.is_empty() && undo_at.is_some() => {
                drag.checkpoints.pop().expect("last checkpoint exists").restore(&mut s.sketch);
            }
            Ok(cuts) => drag.checkpoints.extend(cuts),
            Err(why) => {
                s.say(why.to_string());
                break;
            }
        }
        if undo_at.is_none() {
            break;
        }
        from = end;
    }
}

fn trim_input(
    ui: &egui::Ui,
    s: &mut SketchSession,
    frame: &Frame<'_>,
    resp: &egui::Response,
    actions: &mut Vec<AppAction>,
) -> bool {
    if resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        && let Some(from) = frame.plane_at(origin)
    {
        let extend = hit_curve(frame, &s.sketch, origin, s.trim_options.ignore_construction)
            .filter(|id| s.sketch.entity(*id).is_some_and(|e| e.is_line() || matches!(e.kind, EntityKind::Arc { .. })));
        s.trim_drag = Some(TrimDrag { before: s.sketch.clone(), extend, path: vec![from], checkpoints: Vec::new() });
    }
    let stopped = resp.drag_stopped_by(egui::PointerButton::Primary);
    if !resp.dragged_by(egui::PointerButton::Primary) && !stopped {
        return false;
    }
    let Some(mut drag) = s.trim_drag.take() else { return false };
    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    if let Some(at) = resp.interact_pointer_pos().and_then(|p| frame.plane_at(p)) {
        let last = *drag.path.last().expect("trim trail starts at the press");
        if at.dist(last) > 1e-6 {
            if let Some(id) = drag.extend {
                s.sketch = drag.before.clone();
                if let Err(why) = s.sketch.extend_drag(id, at, s.trim_options) {
                    s.say(why.to_string());
                }
            } else {
                trim_segment(s, &mut drag, frame, last, at);
            }
            drag.path.push(at);
        }
    }
    if stopped {
        if s.sketch != drag.before {
            solve::solve(&mut s.sketch, None, true);
            commit(s, actions);
        }
    } else {
        s.trim_drag = Some(drag);
    }
    true
}

/// Pointer input for the select tool: clicks pick, drags move, double-clicks edit.
#[allow(clippy::too_many_arguments)]
fn select_input(
    ui: &mut egui::Ui,
    s: &mut SketchSession,
    frame: &Frame<'_>,
    resp: &egui::Response,
    clicked: bool,
    double: bool,
    modifiers: egui::Modifiers,
    actions: &mut Vec<AppAction>,
) {
    // Drag start: a point, an entity or a dimension label.
    if resp.drag_started_by(egui::PointerButton::Primary)
        && let Some(pos) = ui.input(|i| i.pointer.press_origin())
    {
        let labels = label_rects(frame, &s.sketch, ui);
        let hit = hit_test(frame, &s.sketch, pos, &[]);
        let label = labels.into_iter().find(|(_, r)| r.contains(pos)).map(|(id, _)| id);
        s.drag = match (hit, label) {
            (Some(SketchSel::Point(p)), _) if p.entity != crate::sketch::ORIGIN => {
                Some(SketchDrag::Point { point: p, moved: false })
            }
            (Some(SketchSel::Entity(e)), _) => {
                frame.plane_at(pos).map(|last| SketchDrag::Entity { entity: e, last, moved: false })
            }
            (_, Some(c)) => {
                let label = s.sketch.constraint(c).and_then(|c| c.label()).unwrap_or(P2::ZERO);
                let at = frame.plane_at(pos).unwrap_or(label);
                Some(SketchDrag::Label { constraint: c, offset: label.sub(at) })
            }
            _ => None,
        };
    }
    if resp.dragged_by(egui::PointerButton::Primary)
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(at) = frame.plane_at(pos)
    {
        match s.drag.clone() {
            Some(SketchDrag::Point { point, .. }) => {
                solve::solve(&mut s.sketch, Some(solve::Drag { point, to: at }), false);
                s.drag = Some(SketchDrag::Point { point, moved: true });
            }
            Some(SketchDrag::Entity { entity, last, .. }) => {
                let d = at.sub(last);
                if let Some(e) = s.sketch.entity_mut(entity) {
                    e.translate(d);
                }
                // Fixed-value relations hold; the rest of the sketch follows the moved entity.
                solve::solve(&mut s.sketch, None, false);
                s.drag = Some(SketchDrag::Entity { entity, last: at, moved: true });
            }
            Some(SketchDrag::Label { constraint, offset }) => {
                if let Some(c) = s.sketch.constraint_mut(constraint) {
                    c.set_label(at.add(offset));
                }
            }
            Some(SketchDrag::Offset { .. }) => {}
            None => {}
        }
    }
    if resp.drag_stopped_by(egui::PointerButton::Primary) {
        match s.drag.take() {
            Some(SketchDrag::Point { moved: true, .. }) | Some(SketchDrag::Entity { moved: true, .. }) => {
                solve::solve(&mut s.sketch, None, true);
                commit(s, actions);
            }
            Some(SketchDrag::Label { .. }) => commit(s, actions),
            _ => {}
        }
    }
    if double {
        if let Some(SketchSel::Constraint(c)) = s.hover
            && let Some(con) = s.sketch.constraint(c)
            && con.is_dimension()
        {
            if con.value().is_some() {
                actions.push(AppAction::EditDimension(crate::viewport::dimensions::DimensionTarget::Sketch {
                    sketch: s.feature,
                    constraint: c,
                }));
            } else if con.is_reference() {
                s.say("Reference dimensions are read-only; change to driving to edit");
            }
        }
        return;
    }
    if clicked {
        match s.hover {
            Some(h) => {
                if modifiers.ctrl || modifiers.shift {
                    if let Some(i) = s.selection.iter().position(|x| *x == h) {
                        s.selection.remove(i);
                    } else {
                        s.selection.push(h);
                    }
                } else {
                    s.selection = vec![h];
                }
            }
            None => s.selection.clear(),
        }
    }
}

/// What Trim or Extend would do to the curve under the pointer. Extend lengthens
/// the end nearer the pointer to the nearest curve beyond it; Trim removes the interval
/// under the pointer. The preview carries the operation a click applies.
fn curve_preview(s: &SketchSession, frame: &Frame<'_>, pos: Pos2) -> Option<ToolPreview> {
    let id = hit_curve(frame, &s.sketch, pos, ignore_guide_targets(s))?;
    let at = frame.plane_at(pos)?;
    let entity = s.sketch.entity(id)?.clone();
    let preview = match s.tool {
        Tool::TrimCorner | Tool::Fillet | Tool::Chamfer => {
            if let Some((a, keep_a)) = s.corner_first {
                let operation = match s.tool {
                    Tool::Fillet => Operation::Fillet { a, b: id, keep_a, keep_b: at, radius: s.modify.fillet_radius },
                    Tool::Chamfer => Operation::Chamfer { a, b: id, keep_a, keep_b: at, size: s.modify.chamfer },
                    _ => Operation::TrimCorner { a, b: id, keep_a, keep_b: at, options: s.trim_options },
                };
                let mut preview = sketch_tools::try_operation(s, operation);
                preview.target = Some(id);
                preview
            } else {
                ToolPreview {
                    target: Some(id),
                    refused: s
                        .sketch
                        .entity(id)
                        .filter(|e| !matches!(e.kind, EntityKind::Line { .. } | EntityKind::Arc { .. }))
                        .map(|_| "A corner needs lines or arcs, not a closed circle".into()),
                    ..Default::default()
                }
            }
        }
        Tool::Split => {
            let operation = if let Some((circle, first)) = s.corner_first {
                if circle != id {
                    return Some(ToolPreview {
                        target: Some(id),
                        refused: Some("Pick the second point on the same circle".into()),
                        ..Default::default()
                    });
                }
                Operation::Split { entity: circle, at: first, second: Some(at) }
            } else if matches!(entity.kind, EntityKind::Circle { .. }) {
                return Some(ToolPreview { target: Some(id), ..Default::default() });
            } else {
                Operation::Split { entity: id, at, second: None }
            };
            let mut preview = sketch_tools::try_operation(s, operation);
            preview.target = Some(id);
            preview
        }
        Tool::TrimInside | Tool::TrimOutside => {
            if let [a, b] = s.trim_bounds.as_slice() {
                let mut preview = sketch_tools::try_operation(
                    s,
                    Operation::TrimBounded {
                        entity: id,
                        a: *a,
                        b: *b,
                        outside: s.tool == Tool::TrimOutside,
                        options: s.trim_options,
                    },
                );
                preview.target = Some(id);
                preview
            } else {
                ToolPreview {
                    target: Some(id),
                    refused: s.trim_bounds.contains(&id).then(|| "Choose a different bounding curve".into()),
                    ..Default::default()
                }
            }
        }
        Tool::Trim => match s.sketch.trim_preview(id, at, s.trim_options) {
            Ok(trim) => ToolPreview {
                added: if s.trim_options.keep_as_construction { trim.kept } else { Vec::new() },
                removed: trim.removed,
                target: Some(id),
                operation: Some(Operation::Trim { entity: id, at, options: s.trim_options }),
                ..Default::default()
            },
            Err(e) => ToolPreview { target: Some(id), refused: Some(e.0), ..Default::default() },
        },
        Tool::Extend => {
            // The end to move is the one nearer the pointer on screen.
            let (a, b) = match entity.kind {
                EntityKind::Line { a, b } => (a, b),
                EntityKind::Arc { start, end, .. } => (start, end),
                _ => {
                    return Some(ToolPreview {
                        target: Some(id),
                        refused: Some("A circle has no endpoint to extend".into()),
                        ..Default::default()
                    });
                }
            };
            let nearer = |p: P2| frame.screen(p).map(|sp| (sp - pos).length()).unwrap_or(f32::INFINITY);
            let end = if nearer(a) <= nearer(b) { End::Start } else { End::End };
            match s.sketch.extend_boundary(id, end) {
                Some(boundary) => {
                    let operation = Operation::Extend { entity: id, end, boundary };
                    let mut preview = sketch_tools::try_operation(s, operation);
                    preview.target = Some(id);
                    preview.boundary = Some(boundary);
                    preview
                }
                None => ToolPreview {
                    target: Some(id),
                    refused: Some("Nothing lies beyond that end to extend to".into()),
                    ..Default::default()
                },
            }
        }
        _ => return None,
    };
    Some(preview)
}

/// A click with a drawing tool.
fn tool_click(s: &mut SketchSession, p: P2, snaps: Vec<Snap>, double: bool, actions: &mut Vec<AppAction>) {
    mirrored_placement(s, p, snaps, double, actions, true);
}

fn mirrored_placement(
    s: &mut SketchSession,
    p: P2,
    snaps: Vec<Snap>,
    double: bool,
    actions: &mut Vec<AppAction>,
    analyse: bool,
) -> Vec<EntityId> {
    if s.dynamic_mirror.is_some() && matches!(s.tool, Tool::Ellipse | Tool::PartialEllipse) {
        s.say("Stop dynamic mirror with Esc before drawing an ellipse");
        return Vec::new();
    }
    mirrored_edit(s, actions, analyse, |s, actions, analyse| draw_click(s, p, snaps, double, actions, analyse))
}

fn mirrored_edit(
    s: &mut SketchSession,
    actions: &mut Vec<AppAction>,
    analyse: bool,
    edit: impl FnOnce(&mut SketchSession, &mut Vec<AppAction>, bool),
) -> Vec<EntityId> {
    let Some(axis) = s.dynamic_mirror.filter(|_| s.tool.places_points()) else {
        edit(s, actions, analyse);
        return Vec::new();
    };
    if !s.sketch.entity(axis).is_some_and(|e| e.is_line()) {
        s.dynamic_mirror = None;
        edit(s, actions, analyse);
        s.say("Dynamic mirror stopped: its axis no longer exists");
        return Vec::new();
    }
    let checkpoint = s.sketch.clone();
    let pending = s.pending.clone();
    let chain = s.chain_from;
    let first_action = actions.len();
    edit(s, actions, false);
    let entities: Vec<_> = s.sketch.drawn().filter(|e| checkpoint.entity(e.id).is_none()).map(|e| e.id).collect();
    if entities.is_empty() {
        return Vec::new();
    }
    let operation =
        Operation::MirrorLinked { entities, axis: crate::sketch::tools::MirrorAxis::Entity { entity: axis } };
    match s.sketch.apply_operation(&operation) {
        Ok(ids) => {
            solve::solve(&mut s.sketch, None, analyse);
            for action in &mut actions[first_action..] {
                if let AppAction::Command(Command::EditSketch { sketch, .. }) = action {
                    *sketch = s.sketch.clone();
                }
            }
            ids
        }
        Err(error) => {
            s.sketch = checkpoint;
            s.pending = pending;
            s.chain_from = chain;
            actions.truncate(first_action);
            s.say(error.to_string());
            Vec::new()
        }
    }
}

fn dynamic_preview(s: &SketchSession, p: Option<P2>) -> Vec<(EntityKind, bool)> {
    let Some(p) = p.filter(|_| s.dynamic_mirror.is_some() && s.tool.places_points()) else {
        return Vec::new();
    };
    if s.tool.spline_mode().is_some() {
        let axis = s.dynamic_mirror.and_then(|id| s.sketch.entity(id));
        let Some(EntityKind::Line { a, b }) = axis.map(|e| &e.kind) else {
            return Vec::new();
        };
        return spline::preview(s, p)
            .and_then(|kind| crate::sketch::tools::reflected(kind, *a, *b).ok())
            .map(|kind| vec![(kind, s.construction)])
            .unwrap_or_default();
    }
    let mut copy = SketchSession::new(s.feature, s.plane, s.sketch.clone(), s.was_orthographic);
    copy.tool = s.tool;
    copy.pending = s.pending.clone();
    copy.chain_from = s.chain_from;
    copy.construction = s.construction;
    copy.modify = s.modify.clone();
    copy.dynamic_mirror = s.dynamic_mirror;
    let ids = mirrored_placement(&mut copy, p, s.snaps.clone(), false, &mut Vec::new(), false);
    ids.into_iter().filter_map(|id| copy.sketch.entity(id).map(|e| (e.kind.clone(), e.construction))).collect()
}

fn draw_click(
    s: &mut SketchSession,
    p: P2,
    snaps: Vec<Snap>,
    double: bool,
    actions: &mut Vec<AppAction>,
    analyse: bool,
) {
    match s.tool {
        Tool::Point => {
            let id = s.sketch.add(EntityKind::Point { p }, false);
            apply_snaps(&mut s.sketch, PointRef::new(id, 0), &snaps, None);
            solve::solve(&mut s.sketch, None, analyse);
            commit(s, actions);
        }
        Tool::Line | Tool::Centreline => {
            if double {
                // A double click ends the chain; the first click of it already landed.
                s.pending.clear();
                s.chain_from = None;
                return;
            }
            match s.pending.pop() {
                None => s.pending.push((p, snaps)),
                Some((a, a_snaps)) => {
                    if a.dist(p) < 1e-6 {
                        s.pending.push((a, a_snaps));
                        return;
                    }
                    let id = s.sketch.add(EntityKind::Line { a, b: p }, s.draws_construction());
                    apply_snaps(&mut s.sketch, PointRef::new(id, 0), &a_snaps, Some(id));
                    apply_snaps(&mut s.sketch, PointRef::new(id, 1), &snaps, Some(id));
                    solve::solve(&mut s.sketch, None, analyse);
                    commit(s, actions);
                    let closed = snaps.iter().any(|x| matches!(x, Snap::Point(_)));
                    if closed {
                        s.chain_from = None;
                    } else {
                        let end = PointRef::new(id, 1);
                        s.chain_from = Some(end);
                        s.pending.push((p, vec![Snap::Point(end)]));
                    }
                }
            }
        }
        Tool::Rectangle
        | Tool::CentreRectangle
        | Tool::ThreePointCornerRectangle
        | Tool::ThreePointCentreRectangle
        | Tool::Parallelogram => rectangle::click(s, p, snaps, actions, analyse),
        Tool::FitSpline | Tool::ControlSpline => spline::click(s, p, snaps, double, actions, analyse),
        Tool::Ellipse | Tool::PartialEllipse => ellipse::click(s, p, snaps, actions),
        Tool::Circle => match s.pending.pop() {
            None => s.pending.push((p, snaps)),
            Some((c, c_snaps)) => {
                let r = c.dist(p);
                if r < 1e-6 {
                    s.pending.push((c, c_snaps));
                    return;
                }
                let id = s.sketch.add(EntityKind::Circle { center: c, radius: r }, s.construction);
                apply_snaps(&mut s.sketch, PointRef::new(id, 0), &c_snaps, None);
                if let Some(Snap::Point(r)) = snaps.iter().find(|x| matches!(x, Snap::Point(_))) {
                    s.sketch.constrain(ConstraintKind::PointOn { point: *r, entity: id });
                }
                solve::solve(&mut s.sketch, None, analyse);
                commit(s, actions);
            }
        },
        Tool::Arc => {
            if s.pending.len() < 2 {
                s.pending.push((p, snaps));
                return;
            }
            let (start, start_snaps) = s.pending.remove(1);
            let (c, c_snaps) = s.pending.remove(0);
            let r = c.dist(start);
            if r < 1e-6 || c.dist(p) < 1e-6 {
                return;
            }
            let end = c.add(p.sub(c).normalized().scale(r));
            let mut sweep = end.sub(c).angle() - start.sub(c).angle();
            while sweep < 0.0 {
                sweep += std::f64::consts::TAU;
            }
            // Arcs are stored counter-clockwise; a clockwise pick swaps the ends.
            let (a, b, a_snaps, b_snaps) = if sweep <= std::f64::consts::PI {
                (start, end, start_snaps, snaps)
            } else {
                (end, start, snaps, start_snaps)
            };
            let id = s.sketch.add(EntityKind::Arc { center: c, start: a, end: b }, s.construction);
            apply_snaps(&mut s.sketch, PointRef::new(id, 0), &c_snaps, None);
            apply_snaps(&mut s.sketch, PointRef::new(id, 1), &a_snaps, None);
            apply_snaps(&mut s.sketch, PointRef::new(id, 2), &b_snaps, None);
            solve::solve(&mut s.sketch, None, analyse);
            commit(s, actions);
        }
        Tool::Slot | Tool::CentrepointSlot | Tool::ThreePointArcSlot | Tool::CentrepointArcSlot => {
            slot::click(s, p, snaps, actions, analyse);
        }
        Tool::Polygon => match s.pending.pop() {
            None => s.pending.push((p, snaps)),
            Some((c, c_snaps)) => {
                let operation = Operation::Polygon { center: c, vertex: p, sides: s.modify.sides };
                let before = s.sketch.entities.len();
                if sketch_tools::apply_with_analysis(s, &operation, actions, analyse) {
                    // The first vertex keeps the relation its snap showed; the centre is
                    // not a point of the sketch.
                    if let Some(first) = s.sketch.entities.get(before).map(|e| e.id) {
                        apply_snaps(&mut s.sketch, PointRef::new(first, 0), &snaps, None);
                        solve::solve(&mut s.sketch, None, analyse);
                        if let Some(AppAction::Command(Command::EditSketch { sketch, .. })) = actions.last_mut() {
                            *sketch = s.sketch.clone();
                        }
                    }
                } else {
                    s.pending.push((c, c_snaps));
                }
            }
        },
        Tool::Select
        | Tool::Dimension
        | Tool::Trim
        | Tool::TrimCorner
        | Tool::Fillet
        | Tool::Chamfer
        | Tool::Split
        | Tool::TrimInside
        | Tool::TrimOutside
        | Tool::Extend
        | Tool::Offset
        | Tool::Mirror
        | Tool::LinearPattern
        | Tool::CircularPattern
        | Tool::Move
        | Tool::Copy
        | Tool::Rotate
        | Tool::Scale => {}
    }
}

/// Smart dimension: pick one or two items, then place the value.
fn dimension_click(s: &mut SketchSession, frame: &Frame<'_>, pos: Pos2, actions: &mut Vec<AppAction>) {
    let hit = hit_test(frame, &s.sketch, pos, &[]).filter(|h| !matches!(h, SketchSel::Constraint(_)));
    let Some(at) = frame.plane_at(pos) else { return };
    match (s.dim_first, hit) {
        (None, Some(h)) => {
            s.dim_first = Some(h);
            s.say(match h {
                SketchSel::Entity(_) => "Click to place the dimension, or pick a second entity",
                _ => "Pick a second point or line, or click to place a distance to the origin",
            });
        }
        (None, None) => s.say("Pick a line, arc, circle, ellipse or point to dimension"),
        (Some(first), Some(second)) if second != first && s.dim_second.is_none() && !is_curve_sel(&s.sketch, first) => {
            s.dim_second = Some(second);
            s.say("Click to place the dimension");
        }
        (Some(first), _) => {
            let kind = dimension_kind(&s.sketch, first, s.dim_second, at)
                .and_then(|kind| if s.reference_dimensions { kind.as_reference() } else { Some(kind) });
            s.dim_first = None;
            s.dim_second = None;
            if let Some(kind) = kind {
                let id = s.sketch.constrain(kind);
                solve::solve(&mut s.sketch, None, true);
                commit(s, actions);
                if let Some(value) = s.sketch.constraint(id).and_then(|c| c.value()) {
                    s.dim_edit = Some(DimEdit::new(id, value));
                }
            } else {
                s.say("Those two cannot be dimensioned together");
            }
        }
    }
}

fn is_curve_sel(sketch: &Sketch, sel: SketchSel) -> bool {
    matches!(sel, SketchSel::Entity(e) if sketch.entity(e).is_some_and(|e| e.is_curve() || crate::sketch::ellipse::Ellipse::of(&e.kind).is_some()))
}

/// The dimension two picks and a placement call for, measured from the sketch.
fn dimension_kind(sketch: &Sketch, first: SketchSel, second: Option<SketchSel>, label: P2) -> Option<ConstraintKind> {
    let ent = |sel: SketchSel| match sel {
        SketchSel::Entity(e) => sketch.entity(e).cloned(),
        _ => None,
    };
    let point_of = |sel: SketchSel| match sel {
        SketchSel::Point(p) => Some(p),
        SketchSel::Entity(e) => sketch.entity(e).filter(|e| e.is_point()).map(|_| PointRef::new(e, 0)),
        _ => None,
    };
    let point_line = |point, line| {
        let mut c = Constraint { id: 0, kind: ConstraintKind::PointLineDistance { point, line, value: 0.0, label } };
        c.set_value(c.measured_value(sketch)?);
        Some(c.kind)
    };
    let between = |a: PointRef, b: PointRef| {
        let (pa, pb) = (sketch.point(a)?, sketch.point(b)?);
        let (lo, hi) = (P2::new(pa.u.min(pb.u), pa.v.min(pb.v)), P2::new(pa.u.max(pb.u), pa.v.max(pb.v)));
        let axis_aligned = (pa.u - pb.u).abs() < 1e-6 || (pa.v - pb.v).abs() < 1e-6;
        let kind = if axis_aligned {
            DistanceKind::Aligned
        } else if label.u < lo.u || label.u > hi.u {
            DistanceKind::Vertical
        } else if label.v < lo.v || label.v > hi.v {
            DistanceKind::Horizontal
        } else {
            DistanceKind::Aligned
        };
        let value = match kind {
            DistanceKind::Aligned => pa.dist(pb),
            DistanceKind::Horizontal => (pa.u - pb.u).abs(),
            DistanceKind::Vertical => (pa.v - pb.v).abs(),
        };
        Some(ConstraintKind::Distance { a, b, kind, value, label })
    };
    match (first, second) {
        (SketchSel::Entity(e), None) => {
            let ent = sketch.entity(e)?;
            match ent.kind {
                EntityKind::Spline { ref points, .. }
                    if points.len() >= 2 && points[0].dist(*points.last().unwrap()) > 1e-7 =>
                {
                    between(PointRef::new(e, 0), PointRef::new(e, points.len() - 1))
                }
                EntityKind::Spline { .. } => None,
                ref kind @ (EntityKind::Ellipse { .. } | EntityKind::EllipticArc { .. }) => {
                    let ellipse = crate::sketch::ellipse::Ellipse::of(kind).unwrap();
                    let direction = (label - ellipse.center).normalized();
                    let index = if direction.dot(ellipse.axis().normalized()).abs()
                        >= direction.dot(ellipse.axis().perp().normalized()).abs()
                    {
                        1
                    } else {
                        2
                    };
                    let b = PointRef::new(e, index);
                    Some(ConstraintKind::Distance {
                        a: PointRef::new(e, 0),
                        b,
                        kind: DistanceKind::Aligned,
                        value: ellipse.center.dist(ent.point(index)?),
                        label,
                    })
                }
                EntityKind::Line { .. } => between(PointRef::new(e, 0), PointRef::new(e, 1)),
                EntityKind::Circle { radius, .. } => {
                    Some(ConstraintKind::Radius { entity: e, value: radius * 2.0, diameter: true, label })
                }
                EntityKind::Arc { .. } => {
                    Some(ConstraintKind::Radius { entity: e, value: ent.radius()?, diameter: false, label })
                }
                EntityKind::Point { .. } => between(PointRef::new(e, 0), PointRef::ORIGIN),
            }
        }
        (SketchSel::Point(p), None) => between(p, PointRef::ORIGIN),
        (a, Some(b)) => {
            if let (Some(pa), Some(pb)) = (point_of(a), point_of(b)) {
                return between(pa, pb);
            }
            match (ent(a), ent(b)) {
                (Some(la), Some(lb)) if la.is_line() && lb.is_line() => {
                    let (EntityKind::Line { a: a0, b: a1 }, EntityKind::Line { a: b0, b: b1 }) = (la.kind, lb.kind)
                    else {
                        return None;
                    };
                    let (da, db) = (a1.sub(a0).normalized(), b1.sub(b0).normalized());
                    if da.cross(db).abs() < 1e-3 {
                        return point_line(PointRef::new(la.id, 0), lb.id);
                    }
                    let mut deg = (db.angle() - da.angle()).to_degrees().rem_euclid(360.0);
                    if deg > 180.0 {
                        deg -= 180.0;
                    }
                    Some(ConstraintKind::Angle { a: la.id, b: lb.id, value: deg, label })
                }
                (Some(l), _) if l.is_line() => point_line(point_of(b)?, l.id),
                (_, Some(l)) if l.is_line() => point_line(point_of(a)?, l.id),
                _ => None,
            }
        }
        (SketchSel::Constraint(_), _) => None,
    }
}

// Painting ---------------------------------------------------------------------

fn entity_color(s: &SketchSession, e: &Entity) -> Color32 {
    let conflicting = match &s.sketch.status {
        SolveStatus::OverDefined { conflicting } => conflicting.clone(),
        _ => Vec::new(),
    };
    let in_conflict = s.sketch.constraints_of(e.id).any(|c| conflicting.contains(&c.id));
    if in_conflict {
        theme::error()
    } else if e.construction {
        theme::muted()
    } else if s.sketch.free.contains(&e.id) {
        theme::accent()
    } else {
        theme::ink()
    }
}

fn paint_plane(painter: &egui::Painter, frame: &Frame<'_>, doc: &Document, sketch: &Sketch) {
    let mut half = 100.0_f64;
    if let Some((lo, hi)) = doc.bounds() {
        half = half.max(((hi - lo).length() as f64) * 0.65);
    }
    if let Some((lo, hi)) = sketch.bounds() {
        half = half.max(lo.u.abs().max(lo.v.abs()).max(hi.u.abs()).max(hi.v.abs()) * 1.3);
    }
    let corners = [P2::new(-half, -half), P2::new(half, -half), P2::new(half, half), P2::new(-half, half)];
    let pts: Vec<Pos2> = corners.iter().filter_map(|p| frame.screen(*p)).collect();
    if pts.len() == 4 {
        painter.add(egui::Shape::convex_polygon(
            pts.clone(),
            theme::accent_alpha(10),
            Stroke::new(1.0, theme::accent().gamma_multiply(0.35)),
        ));
    }
    // Axes of the sketch frame through the origin.
    let axis = |a: P2, b: P2, color: Color32| {
        if let (Some(p), Some(q)) = (frame.screen(a), frame.screen(b)) {
            painter.line_segment([p, q], Stroke::new(1.0, color));
        }
    };
    axis(P2::new(-half, 0.0), P2::new(half, 0.0), theme::line().gamma_multiply(1.3));
    axis(P2::new(0.0, -half), P2::new(0.0, half), theme::line().gamma_multiply(1.3));
}

fn paint_regions(painter: &egui::Painter, frame: &Frame<'_>, s: &SketchSession) {
    let sketch = if let Some(preview) = s.preview.as_ref().filter(|p| !p.replaced.is_empty()) {
        let mut visible = s.sketch.clone();
        visible.entities.retain(|e| !preview.replaced.contains(&e.id));
        std::borrow::Cow::Owned(visible)
    } else {
        std::borrow::Cow::Borrowed(&s.sketch)
    };
    for outline in crate::sketch::profile::region_outlines(&sketch) {
        let pts: Vec<Pos2> = outline.iter().filter_map(|p| frame.screen(*p)).collect();
        if pts.len() >= 3 {
            let mesh = polygon_fill(&pts, theme::accent_alpha(18));
            painter.add(mesh);
        }
    }
}

/// A fan triangulation is enough for the convex and mildly concave shapes a
/// sketch draws; the fill is a hint, not geometry.
fn polygon_fill(pts: &[Pos2], color: Color32) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    let c = pts.iter().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2()) / pts.len() as f32;
    mesh.vertices.push(egui::epaint::Vertex { pos: c.to_pos2(), uv: egui::epaint::WHITE_UV, color });
    for p in pts {
        mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv: egui::epaint::WHITE_UV, color });
    }
    let n = pts.len() as u32;
    for i in 0..n {
        mesh.indices.extend([0, 1 + i, 1 + (i + 1) % n]);
    }
    egui::Shape::mesh(mesh)
}

fn dashed(painter: &egui::Painter, pts: &[Pos2], stroke: Stroke) {
    painter.add(egui::Shape::dashed_line(pts, stroke, 5.0, 4.0));
}

fn paint_entities(painter: &egui::Painter, frame: &Frame<'_>, s: &SketchSession) {
    let selected = |sel: SketchSel| s.selection.contains(&sel);
    let joined_points: std::collections::HashSet<_> =
        s.sketch.constraints.iter().flat_map(Constraint::points).collect();
    let mut relation_entities = std::collections::HashSet::new();
    let mut relation_points = std::collections::HashSet::new();
    for sel in &s.selection {
        if let SketchSel::Constraint(id) = sel
            && let Some(c) = s.sketch.constraint(*id)
        {
            relation_entities.extend(c.entities());
            relation_points.extend(c.points());
        }
    }
    if let Some(issue) = crate::sketch_diagnostics::selected(s) {
        relation_entities.extend(issue.entities.iter().copied());
    }
    for snap in &s.snaps {
        match snap {
            Snap::Intersection(a, b) => relation_entities.extend([*a, *b]),
            Snap::Parallel(entity) | Snap::Perpendicular(entity) | Snap::Tangent(entity) => {
                relation_entities.insert(*entity);
            }
            _ => {}
        }
    }
    let replacing = s.preview.as_ref().and_then(|p| p.operation.as_ref()).and_then(|op| match op {
        Operation::Trim { entity, options, .. } | Operation::TrimBounded { entity, options, .. }
            if options.keep_as_construction =>
        {
            Some([*entity, *entity])
        }
        Operation::TrimCorner { a, b, options, .. } if options.keep_as_construction => Some([*a, *b]),
        Operation::Fillet { a, b, .. } | Operation::Chamfer { a, b, .. } => Some([*a, *b]),
        Operation::Split { entity, .. } => Some([*entity, *entity]),
        _ => None,
    });
    for e in s.sketch.drawn() {
        if replacing.is_some_and(|ids| ids.contains(&e.id))
            || s.preview.as_ref().is_some_and(|p| p.replaced.contains(&e.id))
        {
            continue;
        }
        let color = entity_color(s, e);
        let is_sel = selected(SketchSel::Entity(e.id));
        let is_related = relation_entities.contains(&e.id);
        let is_hover = s.hover == Some(SketchSel::Entity(e.id));
        let pts: Vec<Pos2> = e.polyline(96).into_iter().filter_map(|p| frame.screen(p)).collect();
        if is_sel || is_related || is_hover {
            let halo = Stroke::new(
                if is_sel || is_related { 6.0 } else { 5.0 },
                theme::selection_bg().gamma_multiply(if is_sel || is_related { 1.0 } else { 0.7 }),
            );
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(pts.clone(), halo));
            }
        }
        let stroke = Stroke::new(if e.construction { 1.0 } else { 1.8 }, if is_sel { theme::accent() } else { color });
        if e.is_point() {
            continue;
        }
        let handle_selected =
            s.selection.iter().any(|selected| matches!(selected,SketchSel::Point(point) if point.entity==e.id));
        let handle_hovered = matches!(s.hover,Some(SketchSel::Point(point)) if point.entity==e.id);
        if (is_sel || is_hover || handle_selected || handle_hovered)
            && let EntityKind::Spline { mode: crate::sketch::SplineMode::Control, ref points } = e.kind
        {
            let polygon: Vec<_> = points.iter().filter_map(|point| frame.screen(*point)).collect();
            dashed(painter, &polygon, Stroke::new(0.8, theme::muted()));
        }
        if pts.len() >= 2 {
            if e.construction {
                dashed(painter, &pts, stroke);
            } else {
                painter.add(egui::Shape::line(pts, stroke));
            }
        }
    }
    // Points: filled when joined to something, hollow when free.
    for e in &s.sketch.entities {
        if s.preview.as_ref().is_some_and(|p| p.replaced.contains(&e.id)) {
            continue;
        }
        let color = entity_color(s, e);
        for i in 0..e.point_count() {
            let r = PointRef::new(e.id, i);
            let Some(sp) = e.point(i).and_then(|p| frame.screen(p)) else { continue };
            let is_center = matches!(
                e.kind,
                EntityKind::Arc { .. }
                    | EntityKind::Circle { .. }
                    | EntityKind::Ellipse { .. }
                    | EntityKind::EllipticArc { .. }
            ) && i == 0;
            let is_sel = selected(SketchSel::Point(r));
            let is_related = relation_points.contains(&r) || (e.is_point() && relation_entities.contains(&e.id));
            let is_hover = s.hover == Some(SketchSel::Point(r));
            let joined = joined_points.contains(&r);
            if is_sel || is_related || is_hover {
                painter.circle_filled(sp, 7.0, theme::selection_bg());
            }
            if e.id == crate::sketch::ORIGIN {
                let stroke = Stroke::new(1.2, if is_sel { theme::accent() } else { theme::muted() });
                painter.line_segment([sp + vec2(-6.0, 0.0), sp + vec2(6.0, 0.0)], stroke);
                painter.line_segment([sp + vec2(0.0, -6.0), sp + vec2(0.0, 6.0)], stroke);
                painter.circle_stroke(sp, 3.0, stroke);
                continue;
            }
            let fill = if is_sel { theme::accent() } else { color };
            if is_center {
                painter.line_segment([sp + vec2(-4.0, 0.0), sp + vec2(4.0, 0.0)], Stroke::new(1.2, fill));
                painter.line_segment([sp + vec2(0.0, -4.0), sp + vec2(0.0, 4.0)], Stroke::new(1.2, fill));
            } else if joined || e.is_point() {
                painter.rect_filled(Rect::from_center_size(sp, vec2(5.0, 5.0)), CornerRadius::ZERO, fill);
            } else {
                painter.rect(
                    Rect::from_center_size(sp, vec2(5.0, 5.0)),
                    CornerRadius::ZERO,
                    theme::panel(),
                    Stroke::new(1.2, fill),
                    StrokeKind::Middle,
                );
            }
        }
    }
}

/// Where each relation's glyph sits on screen.
fn glyph_rects(frame: &Frame<'_>, sketch: &Sketch) -> Vec<(ConstraintId, Rect)> {
    let mut out = Vec::new();
    let mut per_anchor: std::collections::HashMap<(i32, i32), usize> = std::collections::HashMap::new();
    for c in &sketch.constraints {
        if c.is_dimension() {
            continue;
        }
        for anchor in glyph_anchors(sketch, c) {
            let Some(sp) = frame.screen(anchor.0) else { continue };
            let base = sp + anchor.1;
            let key = ((base.x / 4.0) as i32, (base.y / 4.0) as i32);
            let n = per_anchor.entry(key).or_insert(0);
            let pos = base + vec2(16.0 * *n as f32, 0.0);
            *n += 1;
            out.push((c.id, Rect::from_center_size(pos, vec2(14.0, 14.0))));
        }
    }
    out
}

/// Sketch point and screen offset where a relation's glyph goes, one per
/// entity it relates.
fn glyph_anchors(sketch: &Sketch, c: &Constraint) -> Vec<(P2, egui::Vec2)> {
    let beside = |id: EntityId| -> Option<(P2, egui::Vec2)> {
        let e = sketch.entity(id)?;
        Some((e.midpoint(), vec2(0.0, -14.0)))
    };
    let at_point = |r: PointRef| sketch.point(r).map(|p| (p, vec2(10.0, -10.0)));
    match c.kind {
        ConstraintKind::Coincident { a, .. } => at_point(a).into_iter().collect(),
        ConstraintKind::Symmetric { a, b, .. } => at_point(a).into_iter().chain(at_point(b)).collect(),
        ConstraintKind::FixEntity { entity, .. } => beside(entity).into_iter().collect(),
        ConstraintKind::PatternTransform { b, .. } => beside(b).into_iter().collect(),
        ConstraintKind::PointOn { point, .. }
        | ConstraintKind::Midpoint { point, .. }
        | ConstraintKind::Fix { point, .. } => at_point(point).into_iter().collect(),
        ConstraintKind::Horizontal { target } | ConstraintKind::Vertical { target } => match target {
            Align::Line(l) => beside(l).into_iter().collect(),
            Align::Points(a, _) => at_point(a).into_iter().collect(),
        },
        ConstraintKind::Parallel { a, b }
        | ConstraintKind::Perpendicular { a, b }
        | ConstraintKind::Tangent { a, b }
        | ConstraintKind::Equal { a, b }
        | ConstraintKind::Concentric { a, b }
        | ConstraintKind::Offset { a, b, .. } => beside(a).into_iter().chain(beside(b)).collect(),
        ConstraintKind::OffsetCap { cap, .. } => beside(cap).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn paint_glyphs(painter: &egui::Painter, s: &SketchSession, glyphs: &[(ConstraintId, Rect)]) {
    let conflicting = match &s.sketch.status {
        SolveStatus::OverDefined { conflicting } => conflicting.clone(),
        _ => Vec::new(),
    };
    for (id, r) in glyphs {
        let Some(c) = s.sketch.constraint(*id) else { continue };
        if s.preview.as_ref().is_some_and(|p| c.entities().iter().any(|id| p.replaced.contains(id))) {
            continue;
        }

        let sel = s.selection.contains(&SketchSel::Constraint(*id));
        let hover = s.hover == Some(SketchSel::Constraint(*id));
        let bad = conflicting.contains(id);
        let (bg, fg) = if sel {
            (theme::accent(), theme::on_accent())
        } else if bad {
            (theme::error(), theme::on_accent())
        } else if hover {
            (theme::selection_bg(), theme::selection_ink())
        } else {
            (theme::hud_bg(), theme::muted())
        };
        painter.rect(
            *r,
            CornerRadius::same(3),
            bg,
            Stroke::new(1.0, if sel || bad { bg } else { theme::line() }),
            StrokeKind::Inside,
        );
        let g = r.shrink(3.0);
        let st = Stroke::new(1.3, fg);
        let (l, t, rr, b) = (g.left(), g.top(), g.right(), g.bottom());
        let (cx, cy) = (g.center().x, g.center().y);
        match c.kind {
            ConstraintKind::Coincident { .. } => {
                painter.circle_stroke(pos2(cx - 2.0, cy), 2.5, st);
                painter.circle_stroke(pos2(cx + 2.0, cy), 2.5, st);
            }
            ConstraintKind::Horizontal { .. } => {
                painter.text(g.center(), Align2::CENTER_CENTER, "H", theme::font_semibold(9.5), fg);
            }
            ConstraintKind::Vertical { .. } => {
                painter.text(g.center(), Align2::CENTER_CENTER, "V", theme::font_semibold(9.5), fg);
            }
            ConstraintKind::Parallel { .. } => {
                painter.line_segment([pos2(l + 1.0, b), pos2(cx - 0.5, t)], st);
                painter.line_segment([pos2(cx + 1.5, b), pos2(rr, t)], st);
            }
            ConstraintKind::Perpendicular { .. } => {
                painter.line_segment([pos2(l, b), pos2(rr, b)], st);
                painter.line_segment([pos2(cx, b), pos2(cx, t)], st);
            }
            ConstraintKind::Equal { .. } => {
                painter.line_segment([pos2(l, cy - 2.0), pos2(rr, cy - 2.0)], st);
                painter.line_segment([pos2(l, cy + 2.0), pos2(rr, cy + 2.0)], st);
            }
            ConstraintKind::Tangent { .. } => {
                painter.circle_stroke(pos2(cx, cy + 1.0), 3.0, st);
                painter.line_segment([pos2(l, cy - 2.0), pos2(rr, cy - 2.0)], st);
            }
            ConstraintKind::Concentric { .. } => {
                painter.circle_stroke(g.center(), 3.5, st);
                painter.circle_stroke(g.center(), 1.2, st);
            }
            ConstraintKind::Midpoint { .. } => {
                painter.line_segment([pos2(l, cy), pos2(rr, cy)], st);
                painter.circle_filled(g.center(), 1.8, fg);
            }
            ConstraintKind::PointOn { .. } => {
                painter.line_segment([pos2(l, b - 1.0), pos2(rr, t + 1.0)], st);
                painter.circle_filled(g.center(), 1.8, fg);
            }
            ConstraintKind::Offset { .. } => {
                painter.line_segment([pos2(l, cy - 3.0), pos2(rr, cy - 3.0)], st);
                painter.line_segment([pos2(l, cy + 3.0), pos2(rr, cy + 3.0)], st);
                painter.line_segment([pos2(cx, cy - 3.0), pos2(cx, cy + 3.0)], st);
            }
            ConstraintKind::OffsetCap { .. } => {
                painter.text(g.center(), Align2::CENTER_CENTER, "C", theme::font_semibold(9.5), fg);
            }
            ConstraintKind::Symmetric { .. } => {
                painter.line_segment([pos2(cx, t), pos2(cx, b)], st);
                painter.line_segment([pos2(l, cy), pos2(rr, cy)], st);
                painter.circle_filled(pos2(l, cy), 1.5, fg);
                painter.circle_filled(pos2(rr, cy), 1.5, fg);
            }
            ConstraintKind::PatternTransform { .. } => {
                for x in [l + 1.0, rr - 1.0] {
                    for y in [t + 1.0, b - 1.0] {
                        painter.circle_filled(pos2(x, y), 1.3, fg);
                    }
                }
            }
            ConstraintKind::Fix { .. } | ConstraintKind::FixEntity { .. } => {
                painter.rect_stroke(
                    Rect::from_min_max(pos2(l + 1.0, cy - 0.5), pos2(rr - 1.0, b)),
                    CornerRadius::same(1),
                    st,
                    StrokeKind::Middle,
                );
                painter.line_segment([pos2(cx, t), pos2(cx, cy)], st);
            }
            _ => {}
        }
    }
}

pub fn arrow(painter: &egui::Painter, tip: Pos2, dir: egui::Vec2, color: Color32) {
    let d = dir.normalized();
    let side = vec2(-d.y, d.x);
    painter.add(egui::Shape::convex_polygon(
        vec![tip, tip - d * 8.0 + side * 2.6, tip - d * 8.0 - side * 2.6],
        color,
        Stroke::NONE,
    ));
}

/// Screen rectangles of the dimension labels.
fn label_rects(frame: &Frame<'_>, sketch: &Sketch, ui: &egui::Ui) -> Vec<(ConstraintId, Rect)> {
    sketch
        .constraints
        .iter()
        .filter(|c| c.is_dimension())
        .filter_map(|c| {
            let sp = frame.screen(c.label()?)?;
            let text = dim_text(c, sketch);
            let galley = ui.painter().layout_no_wrap(text, theme::font_medium(11.5), theme::ink());
            Some((c.id, Rect::from_center_size(sp, galley.size() + vec2(12.0, 6.0))))
        })
        .collect()
}

pub(crate) fn dim_text(c: &Constraint, sketch: &Sketch) -> String {
    let value = c.display_value(sketch).map(fmt_value).unwrap_or_else(|| "?".into());
    let text = match c.geometry_kind() {
        ConstraintKind::Radius { diameter: true, .. } => format!("Ø{value}"),
        ConstraintKind::Radius { diameter: false, .. } => format!("R{value}"),
        ConstraintKind::Angle { .. } => format!("{value}°"),
        _ => value,
    };
    if c.is_reference() { format!("({text})") } else { text }
}

fn paint_dimensions(painter: &egui::Painter, frame: &Frame<'_>, s: &SketchSession) {
    let conflicting = match &s.sketch.status {
        SolveStatus::OverDefined { conflicting } => conflicting.clone(),
        _ => Vec::new(),
    };
    for c in s.sketch.constraints.iter().filter(|c| c.is_dimension()) {
        let sel = s.selection.contains(&SketchSel::Constraint(c.id));
        let hover = s.hover == Some(SketchSel::Constraint(c.id));
        let bad = conflicting.contains(&c.id);
        let color = if bad {
            theme::error()
        } else if sel {
            theme::accent()
        } else {
            theme::muted().gamma_multiply(1.15)
        };
        let Some(label) = c.label() else { continue };
        let Some(lp) = frame.screen(label) else { continue };
        paint_dimension_lines(painter, frame, &s.sketch, c, label, lp, color);
        let bg = if sel {
            theme::selection_bg()
        } else if hover {
            theme::hover()
        } else {
            theme::hud_bg()
        };
        let ink = if bad { theme::error() } else { theme::ink() };
        value_pill(
            painter,
            lp,
            dim_text(c, &s.sketch),
            ink,
            bg,
            Stroke::new(1.0, if sel { theme::accent() } else { theme::line() }),
        );
    }
}

/// Draws a dimension's extension and dimension lines, arrows and leader in `color`.
fn paint_dimension_lines(
    painter: &egui::Painter,
    frame: &Frame<'_>,
    sk: &Sketch,
    c: &Constraint,
    label: P2,
    lp: Pos2,
    color: Color32,
) {
    let st = Stroke::new(1.0, color);
    let label_leader = |a: P2, b: P2| {
        let span = b.sub(a);
        let length2 = span.dot(span);
        let t = if length2 > 0.0 { (label.sub(a).dot(span) / length2).clamp(0.0, 1.0) } else { 0.0 };
        let nearest = a.add(span.scale(t));
        if nearest.dist(label) > 1e-6
            && let Some(at) = frame.screen(nearest)
        {
            painter.line_segment([at, lp], st);
        }
    };
    match c.geometry_kind() {
        ConstraintKind::Distance { a, b, kind, .. } => {
            let (Some(pa), Some(pb)) = (sk.point(a), sk.point(b)) else { return };
            // Dimension line end points in plane coordinates.
            let (da, db) = match kind {
                DistanceKind::Aligned => {
                    let dir = pb.sub(pa).normalized();
                    let n = dir.perp();
                    let off = label.sub(pa).dot(n);
                    (pa.add(n.scale(off)), pb.add(n.scale(off)))
                }
                DistanceKind::Horizontal => (P2::new(pa.u, label.v), P2::new(pb.u, label.v)),
                DistanceKind::Vertical => (P2::new(label.u, pa.v), P2::new(label.u, pb.v)),
            };
            let (Some(sa), Some(sb), Some(spa), Some(spb)) =
                (frame.screen(da), frame.screen(db), frame.screen(pa), frame.screen(pb))
            else {
                return;
            };
            let ext = |from: Pos2, to: Pos2| {
                let d = (to - from).normalized();
                painter.line_segment([from + d * 4.0, to + d * 5.0], Stroke::new(0.8, color.gamma_multiply(0.8)));
            };
            ext(spa, sa);
            ext(spb, sb);
            painter.line_segment([sa, sb], st);
            label_leader(da, db);
            let dir = sb - sa;
            if dir.length() > 20.0 {
                arrow(painter, sa, -dir, color);
                arrow(painter, sb, dir, color);
            }
        }
        ConstraintKind::PointLineDistance { point, line, .. } => {
            let (Some(p), Some(l)) = (sk.point(point), sk.entity(line)) else { return };
            let EntityKind::Line { a, b } = l.kind else { return };
            // The foot on the infinite line, so the dimension works past the ends.
            let dir = b.sub(a).normalized();
            let foot = a.add(dir.scale(p.sub(a).dot(dir)));
            let along = label.sub(foot).dot(dir);
            let (da, db) = (p.add(dir.scale(along)), foot.add(dir.scale(along)));
            let (Some(sa), Some(sb), Some(sp), Some(sf)) =
                (frame.screen(da), frame.screen(db), frame.screen(p), frame.screen(foot))
            else {
                return;
            };
            painter.line_segment([sp, sa], Stroke::new(0.8, color.gamma_multiply(0.8)));
            painter.line_segment([sf, sb], Stroke::new(0.8, color.gamma_multiply(0.8)));
            painter.line_segment([sa, sb], st);
            label_leader(da, db);
            let d = sb - sa;
            if d.length() > 20.0 {
                arrow(painter, sa, -d, color);
                arrow(painter, sb, d, color);
            }
        }
        ConstraintKind::Angle { a, b, .. } => {
            let (Some(la), Some(lb)) = (sk.entity(a), sk.entity(b)) else { return };
            let (EntityKind::Line { a: a0, b: a1 }, EntityKind::Line { a: b0, b: b1 }) = (&la.kind, &lb.kind) else {
                return;
            };
            let (a0, a1, b0, b1) = (*a0, *a1, *b0, *b1);
            // Intersection of the two lines.
            let (d1, d2) = (a1.sub(a0), b1.sub(b0));
            let den = d1.cross(d2);
            if den.abs() < 1e-9 {
                return;
            }
            let t = b0.sub(a0).cross(d2) / den;
            let x = a0.add(d1.scale(t));
            let r = label.dist(x);
            let ang = |d: P2| {
                // Point each direction toward the label's side.
                let d = d.normalized();
                if label.sub(x).dot(d) < 0.0 { d.scale(-1.0).angle() } else { d.angle() }
            };
            let (mut a_from, mut a_to) = (ang(d1), ang(d2));
            if a_to < a_from {
                std::mem::swap(&mut a_from, &mut a_to);
            }
            if a_to - a_from > std::f64::consts::PI {
                std::mem::swap(&mut a_from, &mut a_to);
                a_to += std::f64::consts::TAU;
            }
            let n = 24;
            let pts: Vec<Pos2> = (0..=n)
                .filter_map(|i| {
                    frame.screen(x.add(P2::from_angle(a_from + (a_to - a_from) * i as f64 / n as f64).scale(r)))
                })
                .collect();
            if pts.len() >= 2 {
                painter.add(egui::Shape::line(pts, st));
            }
            for d in [d1, d2] {
                let end = x.add(P2::from_angle(ang(d)).scale(r * 1.1));
                if let (Some(sx), Some(se)) = (frame.screen(x), frame.screen(end)) {
                    painter.line_segment([sx, se], Stroke::new(0.8, color.gamma_multiply(0.8)));
                }
            }
        }
        ConstraintKind::Radius { entity, diameter, .. } => {
            let Some(e) = sk.entity(entity) else { return };
            let (Some(center), Some(r)) = (e.point(0), e.radius()) else { return };
            let dir = label.sub(center).normalized();
            let on = center.add(dir.scale(r));
            let (Some(son), Some(sc)) = (frame.screen(on), frame.screen(center)) else { return };
            if diameter {
                let opposite = center.sub(dir.scale(r));
                if let Some(so) = frame.screen(opposite) {
                    painter.line_segment([so, son], st);
                    arrow(painter, so, so - son, color);
                }
            } else {
                painter.line_segment([sc, son], st);
            }
            arrow(painter, son, son - sc, color);
            painter.line_segment([son, lp], st);
        }
        _ => {}
    }
}

/// Draws a dimension's value pill centred on `at` and returns its rectangle.
pub fn value_pill(painter: &egui::Painter, at: Pos2, text: String, ink: Color32, bg: Color32, stroke: Stroke) -> Rect {
    let galley = painter.layout_no_wrap(text, theme::font_medium(11.5), ink);
    let r = Rect::from_center_size(at, galley.size() + vec2(12.0, 6.0));
    painter.rect(r, CornerRadius::same(4), bg, stroke, StrokeKind::Inside);
    painter.galley(r.center() - galley.size() / 2.0, galley, ink);
    r
}

/// Draws closed-sketch dimension lines and returns label placements and text.
/// The viewport paints labels after the full scene so later lines cannot cover values.
#[allow(clippy::too_many_arguments)]
pub fn paint_closed_dimensions(
    painter: &egui::Painter,
    camera: &Camera,
    rect: Rect,
    plane: Plane,
    sketch: &Sketch,
    position: impl Fn(ConstraintId) -> Option<Vec3>,
    taken: impl IntoIterator<Item = Rect>,
    authored_pass: Option<bool>,
    pointer: Option<Pos2>,
) -> Vec<(ConstraintId, Rect, Vec3, String)> {
    let frame = Frame { plane, camera, rect };
    let gv = |v: soliddraft_kernel_api::Vec3| Vec3::new(v.x as f32, v.y as f32, v.z as f32);
    let mut shown = Vec::new();
    let mut previous = taken.into_iter();
    let mut taken = Vec::new();
    // Reserve authored labels first; only automatic placements may move to clear them.
    for authored in [true, false] {
        if authored_pass.is_some_and(|only| only != authored) {
            continue;
        }
        for c in sketch.constraints.iter().filter(|c| c.is_dimension()) {
            let placed = position(c.id);
            if placed.is_some() != authored {
                continue;
            }
            let Some(label) = c.label() else { continue };
            let mut world =
                placed.unwrap_or_else(|| gv(plane.to_world(soliddraft_kernel_api::Point2::new(label.u, label.v))));
            let Some(projected) = camera.project(world, rect) else { continue };
            let text = dim_text(c, sketch);
            let size =
                painter.layout_no_wrap(text.clone(), theme::font_medium(11.5), theme::ink()).size() + vec2(12.0, 6.0);
            taken.extend(previous.by_ref());
            let at = if authored { projected } else { clear_dimension_spot(projected, size, &taken) };
            if at != projected {
                let (origin, ray) = camera.ray(at, rect);
                let normal = camera.forward();
                world = origin + ray * ((world - origin).dot(normal) / ray.dot(normal));
            }
            let relative = world - gv(plane.origin);
            let label = P2::new(relative.dot(gv(plane.x_dir)) as f64, relative.dot(gv(plane.y_dir())) as f64);
            let Some(lp) = frame.screen(label) else { continue };
            let hover = pointer.is_some_and(|p| Rect::from_center_size(at, size).contains(p));
            let color = if hover { theme::accent() } else { theme::ink().gamma_multiply(0.7) };
            paint_dimension_lines(painter, &frame, sketch, c, label, lp, color);
            if at.distance_sq(lp) > 0.01 {
                painter.line_segment([lp, at], Stroke::new(1.0, color));
            }
            let r = Rect::from_center_size(at, size);
            taken.push(r);
            shown.push((c.id, r, world, text));
        }
    }
    shown
}

/// Clears every occupied rectangle by advancing past each blocker, with no search cap
/// that could leave later values covering earlier ones.
fn clear_dimension_spot(mut at: Pos2, size: egui::Vec2, taken: &[Rect]) -> Pos2 {
    for _ in 0..taken.len() {
        let candidate = Rect::from_center_size(at, size);
        let Some(blocker) = taken.iter().find(|other| other.expand(2.0).intersects(candidate)) else {
            break;
        };
        at.y = blocker.bottom() + 4.0 + size.y / 2.0;
    }
    at
}

fn paint_rubber_band(painter: &egui::Painter, frame: &Frame<'_>, s: &SketchSession, snapped: Option<P2>) {
    let Some(p) = snapped else { return };
    let st = Stroke::new(1.5, theme::accent().gamma_multiply(0.8));
    let seg = |a: P2, b: P2| {
        if let (Some(sa), Some(sb)) = (frame.screen(a), frame.screen(b)) {
            painter.line_segment([sa, sb], st);
        }
    };
    let curve = |pts: Vec<P2>| {
        let sp: Vec<Pos2> = pts.into_iter().filter_map(|q| frame.screen(q)).collect();
        if sp.len() >= 2 {
            painter.add(egui::Shape::line(sp, st));
        }
    };
    match (s.tool, s.pending.as_slice()) {
        (Tool::FitSpline | Tool::ControlSpline, _) => spline::paint(painter, frame, s, p, st),
        (Tool::Ellipse | Tool::PartialEllipse, _) => {
            if let Some(kind) = ellipse::preview(s, p) {
                curve(Entity { id: 0, kind, construction: false }.polyline(96));
            }
            if let Some((c, _)) = s.pending.first() {
                seg(*c, p);
            }
        }
        (Tool::Line | Tool::Centreline, [(a, _)]) => {
            if s.draws_construction() {
                if let (Some(a), Some(b)) = (frame.screen(*a), frame.screen(p)) {
                    dashed(painter, &[a, b], st);
                }
            } else {
                seg(*a, p);
            }
        }
        (tool, _) if tool.rectangle() => {
            let dashed_segment = |a, b| {
                if let (Some(a), Some(b)) = (frame.screen(a), frame.screen(b)) {
                    dashed(painter, &[a, b], st);
                }
            };
            if let Some(corners) = rectangle::corners(s, p) {
                for i in 0..4 {
                    if s.construction {
                        dashed_segment(corners[i], corners[(i + 1) % 4]);
                    } else {
                        seg(corners[i], corners[(i + 1) % 4]);
                    }
                }
                if let Some((a, b)) = rectangle::guide(s.tool, corners) {
                    dashed_segment(a, b);
                }
            } else if let Some((a, _)) = s.pending.first() {
                seg(*a, p);
            }
        }
        (Tool::Circle, [(c, _)]) => {
            let e = Entity { id: 0, kind: EntityKind::Circle { center: *c, radius: c.dist(p) }, construction: false };
            curve(e.polyline(64));
        }
        (Tool::Arc, [(c, _)]) => seg(*c, p),
        (Tool::Arc, [(c, _), (start, _)]) => {
            let r = c.dist(*start);
            if r > 1e-6 && c.dist(p) > 1e-6 {
                let end = c.add(p.sub(*c).normalized().scale(r));
                let mut sweep = end.sub(*c).angle() - start.sub(*c).angle();
                while sweep < 0.0 {
                    sweep += std::f64::consts::TAU;
                }
                let (a, b) = if sweep <= std::f64::consts::PI { (*start, end) } else { (end, *start) };
                let e = Entity { id: 0, kind: EntityKind::Arc { center: *c, start: a, end: b }, construction: false };
                curve(e.polyline(64));
            }
        }
        (Tool::Polygon, [(c, _)]) => {
            if let Ok(corners) = polygon_corners(*c, p, s.modify.sides) {
                let mut closed = corners.clone();
                closed.push(corners[0]);
                curve(closed);
                seg(*c, p);
            }
        }
        (Tool::Slot | Tool::CentrepointSlot | Tool::ThreePointArcSlot | Tool::CentrepointArcSlot, _) => {
            for (kind, construction) in slot::preview(s, p) {
                let points = Entity { id: 0, kind, construction }.polyline(64);
                if construction {
                    let points: Vec<_> = points.into_iter().filter_map(|p| frame.screen(p)).collect();
                    dashed(painter, &points, st);
                } else {
                    curve(points);
                }
            }
        }
        _ => {}
    }
}

/// The active Modify tool's preview: what it adds in the accent, what it removes in the
/// error colour, over the geometry as it is.
fn paint_tool_preview(painter: &egui::Painter, frame: &Frame<'_>, s: &SketchSession) {
    let Some(preview) = &s.preview else { return };
    let guide = match s.tool {
        Tool::Mirror => Some((s.modify.axis_start, s.modify.axis_end)),
        Tool::LinearPattern => s.pending.first().map(|(a, _)| (*a, a.add(s.modify.step))),
        Tool::CircularPattern | Tool::Rotate => Some((s.modify.center, s.modify.center)),
        Tool::Move | Tool::Copy => Some((s.modify.base, s.modify.base + s.modify.step)),
        Tool::Scale => Some((s.modify.base, s.modify.base)),
        _ => None,
    };
    if let Some((a, b)) = guide
        && let (Some(a), Some(b)) = (frame.screen(a), frame.screen(b))
    {
        let stroke = Stroke::new(1.5, theme::accent());
        painter.add(egui::Shape::dashed_line(&[a, b], stroke, 5.0, 4.0));
        for p in [a, b] {
            painter.circle_stroke(p, 4.0, stroke);
        }
    }
    let curve = |kind: EntityKind, stroke: Stroke, construction: bool| {
        let e = Entity { id: 0, kind, construction: false };
        let pts: Vec<Pos2> = e.polyline(96).into_iter().filter_map(|q| frame.screen(q)).collect();
        if e.is_point() {
            if let Some(p) = pts.first() {
                painter.rect_filled(Rect::from_center_size(*p, vec2(5.0, 5.0)), CornerRadius::ZERO, stroke.color);
            }
        } else if pts.len() >= 2 {
            if construction {
                dashed(painter, &pts, stroke);
            } else {
                painter.add(egui::Shape::line(pts, stroke));
            }
        }
    };
    for kind in &preview.removed {
        let keep = s.tool.trims() && s.trim_options.keep_as_construction;
        let stroke =
            if keep { Stroke::new(1.5, theme::accent()) } else { Stroke::new(4.0, theme::error().gamma_multiply(0.9)) };
        curve(kind.clone(), stroke, keep);
    }
    for (index, kind) in preview.added.iter().enumerate() {
        let guide = preview.construction.get(index).copied().unwrap_or_else(|| match &preview.operation {
            Some(
                Operation::TrimCorner { a, b, .. } | Operation::Fillet { a, b, .. } | Operation::Chamfer { a, b, .. },
            ) => {
                let id = if index == 1 { *b } else { *a };
                s.sketch.entity(id).is_some_and(|e| e.construction)
            }
            Some(Operation::Split { entity, .. }) => s.sketch.entity(*entity).is_some_and(|e| e.construction),
            _ => false,
        });
        curve(kind.clone(), Stroke::new(2.0, theme::accent()), guide);
    }
    if s.tool == Tool::Split {
        let points: Vec<_> = match &preview.operation {
            Some(Operation::Split { entity, at, second }) => s
                .sketch
                .entity(*entity)
                .map(|e| std::iter::once(*at).chain(*second).map(|p| e.nearest(p).0).collect())
                .unwrap_or_default(),
            _ => s.corner_first.map(|(_, at)| vec![at]).unwrap_or_default(),
        };
        for point in points {
            if let Some(at) = frame.screen(point) {
                painter.circle_stroke(at, 4.5, Stroke::new(2.0, theme::accent()));
            }
        }
    }
    if let Some(boundary) = preview.boundary
        && let Some(e) = s.sketch.entity(boundary)
    {
        curve(e.kind.clone(), Stroke::new(3.0, theme::selection_bg()), false);
    }
}

fn paint_diagnostics(painter: &egui::Painter, frame: &Frame<'_>, s: &SketchSession) {
    let Some(issue) = crate::sketch_diagnostics::selected(s) else { return };
    for point in &issue.locations {
        if let Some(at) = frame.screen(*point).filter(|p| p.is_finite()) {
            painter.circle_stroke(at, 8.0, Stroke::new(2.0, theme::error()));
            painter.line_segment([at - vec2(3.0, 0.0), at + vec2(3.0, 0.0)], Stroke::new(1.5, theme::error()));
            painter.line_segment([at - vec2(0.0, 3.0), at + vec2(0.0, 3.0)], Stroke::new(1.5, theme::error()));
        }
    }
}

fn paint_snaps(
    painter: &egui::Painter,
    frame: &Frame<'_>,
    s: &SketchSession,
    hover: Option<Pos2>,
    snapped: Option<P2>,
) {
    let Some(p) = snapped else { return };
    let Some(sp) = frame.screen(p) else { return };
    if hover.is_none() {
        return;
    }
    let accent = theme::accent();
    // Construction line previews already show this path. A second dash period
    // would fill their gaps; keep the inference badges below without the overlay.
    if let Some((prev, _)) = s.pending.last()
        && let Some(sprev) = frame.screen(*prev)
        && !(s.tool.draws_line() && s.draws_construction())
    {
        for snap in &s.snaps {
            if matches!(
                snap,
                Snap::Horizontal | Snap::Vertical | Snap::Parallel(_) | Snap::Perpendicular(_) | Snap::Tangent(_)
            ) {
                painter.add(egui::Shape::dashed_line(
                    &[sprev, sp],
                    Stroke::new(1.0, accent.gamma_multiply(0.6)),
                    4.0,
                    4.0,
                ));
            }
        }
    }
    // The snap marker and its glyph.
    let mut x = sp.x + 12.0;
    for snap in &s.snaps {
        let r = Rect::from_min_size(pos2(x, sp.y + 10.0), vec2(16.0, 16.0));
        painter.rect(r, CornerRadius::same(3), theme::hud_bg(), Stroke::new(1.0, accent), StrokeKind::Inside);
        let st = Stroke::new(1.3, accent);
        let g = r.shrink(3.5);
        match snap {
            Snap::Point(_) => {
                painter.rect_filled(Rect::from_center_size(g.center(), vec2(5.0, 5.0)), CornerRadius::ZERO, accent);
            }
            Snap::Midpoint(_) => {
                painter.line_segment([g.left_center(), g.right_center()], st);
                painter.circle_filled(g.center(), 1.8, accent);
            }
            Snap::On(_) => {
                painter.line_segment([g.left_bottom(), g.right_top()], st);
                painter.circle_filled(g.center(), 1.8, accent);
            }
            Snap::Intersection(_, _) => {
                painter.line_segment([g.left_bottom(), g.right_top()], st);
                painter.line_segment([g.left_top(), g.right_bottom()], st);
            }
            Snap::Parallel(_) => {
                painter.line_segment([pos2(g.left() + 1.0, g.bottom()), pos2(g.center().x - 0.5, g.top())], st);
                painter.line_segment([pos2(g.center().x + 1.5, g.bottom()), g.right_top()], st);
            }
            Snap::Perpendicular(_) => {
                painter.line_segment([g.left_bottom(), g.right_bottom()], st);
                painter.line_segment([g.center_bottom(), g.center_top()], st);
            }
            Snap::Tangent(_) => {
                painter.circle_stroke(g.center() + vec2(0.0, 1.0), 3.0, st);
                painter.line_segment([g.left_center() - vec2(0.0, 2.0), g.right_center() - vec2(0.0, 2.0)], st);
            }
            Snap::Horizontal => {
                painter.text(g.center(), Align2::CENTER_CENTER, "H", theme::font_semibold(9.5), accent);
            }
            Snap::Vertical => {
                painter.text(g.center(), Align2::CENTER_CENTER, "V", theme::font_semibold(9.5), accent);
            }
        }
        x += 18.0;
    }
    if s.snaps.iter().any(|x| matches!(x, Snap::Point(_) | Snap::Midpoint(_) | Snap::On(_) | Snap::Intersection(_, _)))
    {
        painter.circle_stroke(sp, 6.0, Stroke::new(1.5, accent));
    } else {
        painter.line_segment([sp + vec2(-5.0, 0.0), sp + vec2(5.0, 0.0)], Stroke::new(1.0, accent));
        painter.line_segment([sp + vec2(0.0, -5.0), sp + vec2(0.0, 5.0)], Stroke::new(1.0, accent));
    }
    // Coordinates by the cursor.
    let text = format!("{}, {}", fmt_value((p.u * 100.0).round() / 100.0), fmt_value((p.v * 100.0).round() / 100.0));
    painter.text(sp + vec2(12.0, -10.0), Align2::LEFT_BOTTOM, text, theme::font(10.5), theme::muted());
}

/// The popup where a dimension value is typed.
fn dim_editor(ui: &mut egui::Ui, s: &mut SketchSession, frame: &Frame<'_>, actions: &mut Vec<AppAction>) {
    let Some(edit) = s.dim_edit.clone() else { return };
    if s.sketch.constraint(edit.constraint).is_some_and(|c| c.is_reference()) {
        s.dim_edit = None;
        return;
    }
    let Some(c) = s.sketch.constraint(edit.constraint).cloned() else {
        s.dim_edit = None;
        return;
    };
    let Some(anchor) = c.label().and_then(|l| frame.screen(l)) else { return };
    let is_angle = matches!(c.kind, ConstraintKind::Angle { .. });
    let mut close = false;
    let mut apply = false;
    egui::Area::new(egui::Id::new(("dim_edit", edit.constraint)))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor + vec2(14.0, 12.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(theme::panel())
                .stroke(Stroke::new(1.0, theme::line()))
                .corner_radius(CornerRadius::same(7))
                .shadow(ui.style().visuals.popup_shadow)
                .inner_margin(egui::Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(c.name()).size(11.0).color(theme::muted()));
                        let mut text = edit.text.clone();
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut text).desired_width(64.0).font(theme::font_medium(12.5)),
                        );
                        resp.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Dimension value")
                        });
                        if edit.focus {
                            resp.request_focus();
                        }
                        ui.label(
                            egui::RichText::new(if is_angle { "°" } else { "mm" })
                                .size(11.0)
                                .italics()
                                .color(theme::muted()),
                        );
                        if let Some(e) = s.dim_edit.as_mut() {
                            e.text = text;
                            e.focus = false;
                        }
                        // Enter makes the field give up focus; Esc closes without applying.
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            close = true;
                        } else if resp.lost_focus() {
                            apply = true;
                        }
                        if ui.add(egui::Button::new("OK").small()).clicked() {
                            apply = true;
                        }
                    });
                });
        });
    if apply && s.dim_edit.as_ref().is_some_and(|e| e.text.trim() == e.opened_with) {
        close = true;
    } else if apply {
        let text = s.dim_edit.as_ref().map(|e| e.text.clone()).unwrap_or_default();
        match text.trim().trim_end_matches("mm").trim().parse::<f64>() {
            Ok(v) if v.is_finite() && (v > 0.0 || is_angle) => {
                let before = s.sketch.clone();
                if let Some(c) = s.sketch.constraint_mut(edit.constraint) {
                    c.set_value(v);
                }
                solve::solve(&mut s.sketch, None, true);
                if matches!(s.sketch.status, SolveStatus::Failed(_)) {
                    s.sketch = before;
                    s.say("The solver could not apply that value");
                } else {
                    commit(s, actions);
                }
                close = true;
            }
            _ => s.say("Enter a number"),
        }
    }
    if close {
        s.dim_edit = None;
    }
}

/// The sketch status pill and the tool hint, top left of the viewport.
fn paint_status(ui: &mut egui::Ui, s: &mut SketchSession, rect: Rect) {
    let painter = ui.painter().with_clip_rect(rect);
    let (status_color, status) = match &s.sketch.status {
        SolveStatus::Empty => (theme::muted(), "Empty sketch".to_owned()),
        SolveStatus::UnderDefined => (theme::accent(), "Under defined".to_owned()),
        SolveStatus::FullyDefined => (theme::ok(), "Fully defined".to_owned()),
        SolveStatus::OverDefined { conflicting } => {
            (theme::error(), format!("Over defined · {} conflicting", conflicting.len()))
        }
        SolveStatus::Failed(_) => (theme::error(), "Solver failed".to_owned()),
    };
    let hint = match (s.tool, s.pending.len()) {
        (Tool::Select, _) => "Click to select, drag to move, double-click a dimension to edit",
        (Tool::Line, 0) => "Line: click the start point",
        (Tool::Line, _) => "Line: click end; right-click or double-click ends chain; Esc: Select",
        (Tool::Centreline, 0) => "Centreline: click the start point",
        (Tool::Centreline, _) => "Centreline: click end; right-click or double-click ends chain; Esc: Select",
        (Tool::Rectangle, 0) => "Rectangle: click the first corner",
        (Tool::Rectangle, _) => "Rectangle: click the opposite corner",
        (Tool::FitSpline | Tool::ControlSpline, _) => {
            "Spline: click points; double-click or Enter to finish, Backspace removes the last point"
        }
        (Tool::CentreRectangle, 0) => "Centre rectangle: click the centre",
        (Tool::CentreRectangle, _) => "Centre rectangle: click a corner",
        (Tool::ThreePointCornerRectangle | Tool::Parallelogram, 0) => "Click the first corner",
        (Tool::ThreePointCornerRectangle | Tool::Parallelogram, 1) => "Click the second corner to set the first edge",
        (Tool::ThreePointCornerRectangle, _) => "3-point corner rectangle: click to set perpendicular height",
        (Tool::Parallelogram, _) => "Parallelogram: click the third corner",
        (Tool::ThreePointCentreRectangle, 0) => "3-point centre rectangle: click the centre",
        (Tool::ThreePointCentreRectangle, 1) => "3-point centre rectangle: click a side midpoint",
        (Tool::ThreePointCentreRectangle, _) => "3-point centre rectangle: click to set half-width",

        (Tool::Ellipse | Tool::PartialEllipse, 0) => "Ellipse: click the centre",
        (Tool::Ellipse | Tool::PartialEllipse, 1) => "Ellipse: click the first semi-axis endpoint",
        (Tool::Ellipse | Tool::PartialEllipse, 2) => "Ellipse: click the second semi-axis endpoint",
        (Tool::PartialEllipse, 3) => "Partial ellipse: click the start point",
        (Tool::PartialEllipse, _) => "Partial ellipse: click the counterclockwise end point",
        (Tool::Ellipse, _) => "Ellipse: click the second semi-axis endpoint",
        (Tool::Circle, 0) => "Circle: click the centre",
        (Tool::Circle, _) => "Circle: click a point on the circle",
        (Tool::Arc, 0) => "Arc: click the centre",
        (Tool::Arc, 1) => "Arc: click the start point",
        (Tool::Arc, _) => "Arc: click the end point",
        (Tool::Slot, 0) => "Slot: click the first centre",
        (Tool::Slot, 1) => "Slot: click the second centre",
        (Tool::Slot, _) => "Slot: click to set the width",
        (Tool::CentrepointSlot, 0) => "Centrepoint slot: click the midpoint",
        (Tool::CentrepointSlot, 1) => "Centrepoint slot: click an end centre",
        (Tool::CentrepointSlot, _) => "Centrepoint slot: click to set the width",
        (Tool::ThreePointArcSlot, 0) => "Three-point arc slot: click the start centre",
        (Tool::ThreePointArcSlot, 1) => "Three-point arc slot: click the end centre",
        (Tool::ThreePointArcSlot, 2) => "Three-point arc slot: click a point along the arc",
        (Tool::ThreePointArcSlot, _) => "Three-point arc slot: click to set the width",
        (Tool::CentrepointArcSlot, 0) => "Centrepoint arc slot: click the arc centre",
        (Tool::CentrepointArcSlot, 1) => "Centrepoint arc slot: click the start centre",
        (Tool::CentrepointArcSlot, 2) => "Centrepoint arc slot: click the counterclockwise end centre",
        (Tool::CentrepointArcSlot, _) => "Centrepoint arc slot: click to set the width",
        (Tool::Point, _) => "Point: click to place",
        (Tool::Dimension, _) => {
            if s.dim_first.is_some() {
                "Click to place the dimension"
            } else {
                "Smart dimension: pick an entity or point"
            }
        }
        (Tool::Polygon, 0) => "Polygon: click the centre",
        (Tool::Polygon, _) => "Polygon: click a vertex; Esc or right-click cancels",
        (Tool::TrimCorner | Tool::Fillet | Tool::Chamfer, _) if s.corner_first.is_none() => {
            "Corner: pick the first retained portion"
        }
        (Tool::TrimCorner | Tool::Fillet | Tool::Chamfer, _) => "Corner: pick the second retained portion",
        (Tool::Split, _) if s.corner_first.is_some() => "Split: pick the second point on the circle",
        (Tool::Split, _) => "Split: click inside a line or arc; a circle needs two points",
        (Tool::TrimInside | Tool::TrimOutside, _) if s.trim_bounds.is_empty() => "Trim: first bound",
        (Tool::TrimInside | Tool::TrimOutside, _) if s.trim_bounds.len() == 1 => "Trim: second bound",
        (Tool::TrimInside, _) => "Trim inside: pick targets",
        (Tool::TrimOutside, _) => "Trim outside: pick targets",
        (Tool::Trim, _) => "Trim: click or drag",
        (Tool::Extend, _) => "Extend: click the end of a line or arc to lengthen; Esc or right-click ends",
        (Tool::Offset, _) => "Offset: drag a selected curve to set distance and side, then Apply",
        (Tool::Mirror, _) if s.modify.mirror_axis.is_some() => "Mirror: select entities, then Apply",
        (Tool::Mirror, 0) => "Mirror: click the axis start; Ctrl-click selects entities",
        (Tool::Mirror, 1) => "Mirror: click the axis end, then Apply",
        (Tool::Mirror, _) => "Mirror: axis placed; Apply adds the copies",
        (Tool::LinearPattern, 0) => "Linear pattern: click the step start; Ctrl-click selects entities",
        (Tool::LinearPattern, 1) => "Linear pattern: click the step end, then Apply",
        (Tool::LinearPattern, _) => "Linear pattern: step placed; Apply adds the copies",
        (Tool::CircularPattern, _) => "Circular pattern: click the centre, then Apply; Ctrl-click selects entities",
        (Tool::Move | Tool::Copy, 0) => "Click a base point; Ctrl-click selects entities",
        (Tool::Move | Tool::Copy, 1) => "Click the destination, then Apply",
        (Tool::Move | Tool::Copy, _) => "Displacement placed; Apply commits the preview",
        (Tool::Rotate, _) => "Rotate: click the centre, set the angle, then Apply",
        (Tool::Scale, _) => "Scale: click the base point, set the factor, then Apply",
    };
    let message = s.message.as_ref().filter(|(_, at)| at.elapsed().as_secs_f32() < 4.0).map(|(m, _)| m.clone());
    let line2 = message.unwrap_or_else(|| hint.to_owned());
    let g1 = painter.layout_no_wrap(status.clone(), theme::font_semibold(11.5), status_color);
    let g2 = painter.layout_no_wrap(line2, theme::font(11.0), theme::muted());
    let w = g1.size().x.max(g2.size().x) + 20.0;
    let r = Rect::from_min_size(pos2(rect.left() + 12.0, rect.top() + 12.0), vec2(w, 40.0));
    painter.rect(r, CornerRadius::same(8), theme::hud_bg(), Stroke::new(1.0, theme::line()), StrokeKind::Inside);
    painter.circle_filled(pos2(r.left() + 10.0, r.top() + 13.0), 3.5, status_color);
    painter.galley(pos2(r.left() + 18.0, r.top() + 6.0), g1, status_color);
    painter.galley(pos2(r.left() + 10.0, r.top() + 22.0), g2, theme::muted());
}

/// Draws a sketch that is not open, in a quiet colour, for selected sketches.
pub fn paint_passive(
    painter: &egui::Painter,
    camera: &Camera,
    rect: Rect,
    plane: Plane,
    sketch: &Sketch,
    color: Color32,
) {
    let frame = Frame { plane, camera, rect };
    for e in sketch.drawn() {
        let pts: Vec<Pos2> = e.polyline(64).into_iter().filter_map(|p| frame.screen(p)).collect();
        let stroke = Stroke::new(if e.construction { 1.0 } else { 1.6 }, color);
        if e.is_point() {
            if let Some(p) = pts.first() {
                painter.rect_filled(Rect::from_center_size(*p, vec2(4.0, 4.0)), CornerRadius::ZERO, color);
            }
        } else if pts.len() >= 2 {
            if e.construction {
                dashed(painter, &pts, stroke);
            } else {
                painter.add(egui::Shape::line(pts, stroke));
            }
        }
    }
}

/// Adds one relation through the same solve and undo path in both sketch controls.
pub fn add_relation(s: &mut SketchSession, kind: ConstraintKind) -> AppAction {
    add_relations(s, &[kind])
}

/// Add a composite relation with one solve and one document undo step.
pub fn add_relations(s: &mut SketchSession, kinds: &[ConstraintKind]) -> AppAction {
    for kind in kinds {
        if !s.sketch.constraints.iter().any(|c| c.kind == *kind) {
            s.sketch.constrain(kind.clone());
        }
    }
    solve::solve(&mut s.sketch, None, true);
    AppAction::Command(Command::EditSketch { id: s.feature, sketch: s.sketch.clone() })
}

/// The supported relation types, including choices unavailable for the current selection.
pub fn relations_menu(ui: &mut egui::Ui, s: &mut SketchSession) -> Option<AppAction> {
    if ui.checkbox(&mut s.show_relations, "Show all relations").changed() && !s.show_relations {
        s.selection.retain(|sel| match sel {
            SketchSel::Constraint(id) => s.sketch.constraint(*id).is_some_and(|c| c.is_dimension()),
            _ => true,
        });
    }
    ui.separator();
    ui.label("Add relation to selection");
    let applicable = applicable_relation_sets(&s.sketch, &s.selection);
    let mut chosen = None;
    for (name, hint) in [
        ("Horizontal", "Select one line or two points"),
        ("Vertical", "Select one line or two points"),
        ("Parallel", "Select two lines"),
        ("Perpendicular", "Select two lines"),
        ("Coincident", "Select two points"),
        ("On line", "Select one point and one line"),
        ("On curve", "Select one point and one arc, circle or ellipse"),
        ("Tangent", "Select a line and an arc or circle, or two arcs or circles"),
        ("Equal", "Select two lines, two circular curves, or two ellipses"),
        ("Concentric", "Select two arcs, circles or ellipses"),
        ("Collinear", "Select two lines"),
        ("Symmetric", "Select two points or matching entities, then the axis line last"),
        ("Coradial", "Select two arcs or circles"),
        ("Intersection", "Select one point and two lines"),
        ("Midpoint", "Select one point and one line"),
        ("Fix", "Select one point or entity other than the origin"),
    ] {
        let kinds = applicable.iter().find(|(label, _)| *label == name).map(|(_, kinds)| kinds);
        let already_applied =
            kinds.is_some_and(|kinds| kinds.iter().all(|kind| s.sketch.constraints.iter().any(|c| c.kind == *kind)));
        if ui
            .add_enabled(kinds.is_some() && !already_applied, egui::Button::new(name))
            .on_hover_text(hint)
            .on_disabled_hover_text(if already_applied { "Already applied" } else { hint })
            .clicked()
        {
            chosen = kinds.cloned();
            ui.close();
        }
    }
    chosen.map(|kinds| add_relations(s, &kinds))
}

/// Relations that can be added to the current selection, for the property panel.
pub fn applicable_relations(sketch: &Sketch, selection: &[SketchSel]) -> Vec<(&'static str, ConstraintKind)> {
    let ents: Vec<&Entity> = selection
        .iter()
        .filter_map(|s| match s {
            SketchSel::Entity(e) => sketch.entity(*e),
            _ => None,
        })
        .collect();
    let points: Vec<PointRef> = selection
        .iter()
        .filter_map(|s| match s {
            SketchSel::Point(p) => Some(*p),
            SketchSel::Entity(e) => sketch.entity(*e).filter(|e| e.is_point()).map(|_| PointRef::new(*e, 0)),
            _ => None,
        })
        .collect();
    let lines: Vec<EntityId> = ents.iter().filter(|e| e.is_line()).map(|e| e.id).collect();
    let curves: Vec<EntityId> = ents.iter().filter(|e| e.is_curve()).map(|e| e.id).collect();
    let mut out: Vec<(&'static str, ConstraintKind)> = Vec::new();
    let ellipses: Vec<_> =
        ents.iter().filter(|e| crate::sketch::ellipse::Ellipse::of(&e.kind).is_some()).map(|e| e.id).collect();
    let splines: Vec<_> = ents.iter().filter(|e| matches!(e.kind, EntityKind::Spline { .. })).map(|e| e.id).collect();
    if !splines.is_empty() {
        if ents.len() == 1 && points.len() == 1 {
            out.push(("On curve", ConstraintKind::PointOn { point: points[0], entity: splines[0] }));
        } else if ents.len() == 2
            && points.is_empty()
            && crate::sketch::relations::tangent_supported(sketch, ents[0].id, ents[1].id)
        {
            out.push(("Tangent", ConstraintKind::Tangent { a: ents[0].id, b: ents[1].id }));
        } else if ents.len() == 1
            && points.is_empty()
            && let Ok(kinds) =
                sketch.relation_constraints(&crate::sketch::relations::Relation::FixEntity { entity: splines[0] })
        {
            out.push(("Fix", kinds[0].clone()));
        }
        return out;
    }
    if !ellipses.is_empty() {
        match (ents.len(), ellipses.len(), points.len()) {
            (2, 2, 0) => out.push(("Equal", ConstraintKind::Equal { a: ellipses[0], b: ellipses[1] })),
            (1, 1, 1) => out.push(("On curve", ConstraintKind::PointOn { point: points[0], entity: ellipses[0] })),
            _ => {}
        }
        let centers: Vec<_> = ents
            .iter()
            .filter(|e| e.is_curve() || crate::sketch::ellipse::Ellipse::of(&e.kind).is_some())
            .map(|e| e.id)
            .collect();
        if centers.len() == 2 && ents.len() == 2 && points.is_empty() {
            out.push(("Concentric", ConstraintKind::Concentric { a: centers[0], b: centers[1] }));
        }
        return out;
    }
    match (lines.len(), curves.len(), points.len()) {
        (1, 0, 0) => {
            out.push(("Horizontal", ConstraintKind::Horizontal { target: Align::Line(lines[0]) }));
            out.push(("Vertical", ConstraintKind::Vertical { target: Align::Line(lines[0]) }));
        }
        (2, 0, 0) => {
            out.push(("Parallel", ConstraintKind::Parallel { a: lines[0], b: lines[1] }));
            out.push(("Perpendicular", ConstraintKind::Perpendicular { a: lines[0], b: lines[1] }));
            out.push(("Equal", ConstraintKind::Equal { a: lines[0], b: lines[1] }));
        }
        (1, 1, 0) => out.push(("Tangent", ConstraintKind::Tangent { a: lines[0], b: curves[0] })),
        (0, 2, 0) => {
            out.push(("Tangent", ConstraintKind::Tangent { a: curves[0], b: curves[1] }));
            out.push(("Equal", ConstraintKind::Equal { a: curves[0], b: curves[1] }));
            out.push(("Concentric", ConstraintKind::Concentric { a: curves[0], b: curves[1] }));
        }
        (0, 0, 1) => {
            if points[0].entity != crate::sketch::ORIGIN {
                out.push((
                    "Fix",
                    ConstraintKind::Fix { point: points[0], at: sketch.point(points[0]).unwrap_or(P2::ZERO) },
                ));
            }
        }
        (0, 0, 2) => {
            out.push(("Coincident", ConstraintKind::Coincident { a: points[0], b: points[1] }));
            out.push(("Horizontal", ConstraintKind::Horizontal { target: Align::Points(points[0], points[1]) }));
            out.push(("Vertical", ConstraintKind::Vertical { target: Align::Points(points[0], points[1]) }));
        }
        (1, 0, 1) => {
            out.push(("On line", ConstraintKind::PointOn { point: points[0], entity: lines[0] }));
            out.push(("Midpoint", ConstraintKind::Midpoint { point: points[0], line: lines[0] }));
        }
        (0, 1, 1) => out.push(("On curve", ConstraintKind::PointOn { point: points[0], entity: curves[0] })),
        _ => {}
    }
    if let [SketchSel::Entity(id)] = selection
        && sketch.entity(*id).is_some_and(|e| !e.is_point())
        && let Ok(kinds) = sketch.relation_constraints(&crate::sketch::relations::Relation::FixEntity { entity: *id })
    {
        out.push(("Fix", kinds[0].clone()));
    }
    out
}

/// Selection choices, including choices that store more than one solver relation.
pub fn applicable_relation_sets(sketch: &Sketch, selection: &[SketchSel]) -> Vec<(&'static str, Vec<ConstraintKind>)> {
    use crate::sketch::relations::Relation;
    let mut out: Vec<_> =
        applicable_relations(sketch, selection).into_iter().map(|(name, kind)| (name, vec![kind])).collect();
    let mut requests = Vec::new();
    if let [SketchSel::Entity(a), SketchSel::Entity(b)] = selection {
        requests.push(("Collinear", Relation::Collinear { a: *a, b: *b }));
        requests.push(("Coradial", Relation::Coradial { a: *a, b: *b }));
    }
    let point = |sel: &SketchSel| match sel {
        SketchSel::Point(p) => Some(*p),
        SketchSel::Entity(id) if sketch.entity(*id).is_some_and(|e| e.is_point()) => Some(PointRef::new(*id, 0)),
        _ => None,
    };
    if let [first, second, SketchSel::Entity(axis)] = selection {
        if let (Some(a), Some(b)) = (point(first), point(second)) {
            requests.push(("Symmetric", Relation::SymmetricPoints { a, b, axis: *axis }));
        } else if let (SketchSel::Entity(a), SketchSel::Entity(b)) = (first, second) {
            requests.push(("Symmetric", Relation::SymmetricEntities { a: *a, b: *b, axis: *axis }));
        }
    }
    if selection.len() == 3 {
        let points: Vec<_> = selection.iter().filter_map(point).collect();
        let lines: Vec<_> = selection
            .iter()
            .filter_map(|sel| match sel {
                SketchSel::Entity(id) if sketch.entity(*id).is_some_and(|e| e.is_line()) => Some(*id),
                _ => None,
            })
            .collect();
        if let ([p], [a, b]) = (points.as_slice(), lines.as_slice()) {
            requests.push(("Intersection", Relation::Intersection { point: *p, a: *a, b: *b }));
        }
    }
    for (name, request) in requests {
        if let Ok(kinds) = sketch.relation_constraints(&request) {
            out.push((name, kinds));
        }
    }
    out
}

/// A rectangle, for drawing from automation: the same entities and relations
/// the rectangle tool makes. Returns the four line ids, bottom first.
pub fn add_rectangle(sketch: &mut Sketch, a: P2, b: P2) -> [EntityId; 4] {
    rectangle::add_loop(sketch, [a, P2::new(b.u, a.v), b, P2::new(a.u, b.v)], Tool::Rectangle, false)
}
