//! What a relation glyph means, in words. Hovering a glyph shows the relation's name and
//! what it holds; a right-click on one lists every relation and dimension on the glyph's
//! geometry. Hovering a row lights that constraint and its geometry up on the sketch, a
//! click on the row selects it, and Delete removes it as one undo step.

use super::{GlyphLook, paint_glyph};
use crate::document::Command;
use crate::ring_menu::{AMBER, DANGER, DANGER_INK, GROUND, INK, LINE, MUTED};
use crate::sketch::{
    Align, Constraint, ConstraintId, ConstraintKind, DistanceKind, EntityId, EntityKind, ORIGIN, PointRef, Sketch,
    SolveStatus, fmt_value, solve,
};
use crate::state::{AppAction, SketchSel, SketchSession};
use crate::theme;
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, pos2, vec2};

const WIDTH: f32 = 340.0;
const ROW: f32 = 28.0;
const HEADER: f32 = 26.0;
const DELETE_WIDTH: f32 = 56.0;
/// The list scrolls past this height.
const MAX_LIST: f32 = 360.0;

/// The relations menu a right-click on a relation glyph opened.
#[derive(Clone, Debug, PartialEq)]
pub struct RelationsMenu {
    /// Where the right-click was.
    pub at: Pos2,
    /// The relation whose glyph was clicked.
    pub glyph: ConstraintId,
    /// The geometry that relation holds; the menu lists every constraint on it.
    pub entities: Vec<EntityId>,
    /// The constraint whose row is under the pointer.
    pub hovered: Option<ConstraintId>,
    /// Each opening is its own popup: egui closes an open popup on a click outside it,
    /// and the right-click that moves the menu to another glyph is one.
    popup: egui::Id,
}

/// The name of an entity, as the relation list in Properties shows it.
pub fn entity_name(sketch: &Sketch, id: EntityId) -> String {
    if id == ORIGIN {
        return "Origin".into();
    }
    match sketch.entity(id) {
        Some(e) => format!("{} {}", e.kind_name(), e.id),
        None => format!("missing entity {id}"),
    }
}

/// The name of a point: its entity, and which point of it.
pub fn point_name(sketch: &Sketch, p: PointRef) -> String {
    let owner = entity_name(sketch, p.entity);
    let Some(e) = sketch.entity(p.entity).filter(|_| p.entity != ORIGIN) else { return owner };
    let which = match (&e.kind, p.index) {
        (EntityKind::Point { .. }, _) => return owner,
        (EntityKind::Line { .. }, 0) | (EntityKind::Arc { .. }, 1) | (EntityKind::EllipticArc { .. }, 3) => "start",
        (EntityKind::Line { .. }, 1) | (EntityKind::Arc { .. }, 2) | (EntityKind::EllipticArc { .. }, 4) => "end",
        (EntityKind::Arc { .. } | EntityKind::Circle { .. } | EntityKind::Ellipse { .. }, 0)
        | (EntityKind::EllipticArc { .. }, 0) => "centre",
        (EntityKind::Ellipse { .. } | EntityKind::EllipticArc { .. }, 1) => "major axis end",
        (EntityKind::Ellipse { .. } | EntityKind::EllipticArc { .. }, 2) => "minor axis end",
        (EntityKind::Text { .. }, _) => "origin",
        (EntityKind::Spline { .. }, i) => return format!("{owner} point {}", i + 1),
        _ => return owner,
    };
    format!("{owner} {which}")
}

/// The geometry a constraint holds, by name.
pub fn owners(sketch: &Sketch, c: &Constraint) -> String {
    c.entities().iter().map(|id| entity_name(sketch, *id)).collect::<Vec<_>>().join(", ")
}

/// The constraint's name with its value, as a row of a relation list shows it.
pub fn title(sketch: &Sketch, c: &Constraint) -> String {
    if c.is_reference() {
        let value = c.display_value(sketch).map(fmt_value).unwrap_or_else(|| "?".into());
        format!("{} ({value})", c.name())
    } else {
        match c.value() {
            Some(v) => format!("{} {}", c.name(), fmt_value(v)),
            None => c.name().to_owned(),
        }
    }
}

/// What a constraint does, in one plain sentence that names its geometry.
pub fn sentence(sketch: &Sketch, c: &Constraint) -> String {
    let e = |id: EntityId| entity_name(sketch, id);
    let p = |r: PointRef| point_name(sketch, r);
    if c.is_reference() {
        return format!("Measures {} without driving it.", owners(sketch, c));
    }
    match &c.kind {
        ConstraintKind::Coincident { a, b } => format!("{} and {} meet at one point.", p(*a), p(*b)),
        ConstraintKind::PointOn { point, entity } => format!("{} lies on {}.", p(*point), e(*entity)),
        ConstraintKind::Horizontal { target: Align::Line(l) } => format!("{} stays horizontal.", e(*l)),
        ConstraintKind::Horizontal { target: Align::Points(a, b) } => {
            format!("{} and {} stay level with each other.", p(*a), p(*b))
        }
        ConstraintKind::Vertical { target: Align::Line(l) } => format!("{} stays vertical.", e(*l)),
        ConstraintKind::Vertical { target: Align::Points(a, b) } => {
            format!("{} and {} stay one above the other.", p(*a), p(*b))
        }
        ConstraintKind::Parallel { a, b } => format!("{} and {} stay parallel.", e(*a), e(*b)),
        ConstraintKind::Perpendicular { a, b } => format!("{} and {} stay at right angles.", e(*a), e(*b)),
        ConstraintKind::Tangent { a, b } => format!("{} and {} touch without a corner.", e(*a), e(*b)),
        ConstraintKind::Equal { a, b } => {
            let lines = sketch.entity(*a).is_some_and(|x| matches!(x.kind, EntityKind::Line { .. }));
            let what = if lines { "length" } else { "radius" };
            format!("{} and {} keep the same {what}.", e(*a), e(*b))
        }
        ConstraintKind::Midpoint { point, line } => format!("{} stays at the middle of {}.", p(*point), e(*line)),
        ConstraintKind::Concentric { a, b } => format!("{} and {} share one centre.", e(*a), e(*b)),
        ConstraintKind::Symmetric { a, b, axis } => {
            format!("{} and {} mirror each other across {}.", p(*a), p(*b), e(*axis))
        }
        ConstraintKind::Fix { point, .. } => format!("{} cannot move.", p(*point)),
        ConstraintKind::FixEntity { entity, .. } => format!("{} cannot move or change size.", e(*entity)),
        ConstraintKind::Offset { a, b, .. } => format!("{} follows {} at a set distance.", e(*b), e(*a)),
        ConstraintKind::OffsetCap { cap, .. } => format!("{} closes the end of an offset.", e(*cap)),
        ConstraintKind::PatternTransform { a, b, .. } => format!("{} is a pattern copy that follows {}.", e(*b), e(*a)),
        ConstraintKind::Distance { a, b, kind, .. } => {
            let how = match kind {
                DistanceKind::Aligned => "",
                DistanceKind::Horizontal => " across",
                DistanceKind::Vertical => " up and down",
            };
            format!("{} and {} stay this far apart{how}.", p(*a), p(*b))
        }
        ConstraintKind::PointLineDistance { point, line, .. } => {
            format!("{} stays this far from {}.", p(*point), e(*line))
        }
        ConstraintKind::Angle { a, b, .. } => format!("{} and {} keep this angle.", e(*a), e(*b)),
        ConstraintKind::Radius { entity, diameter, .. } => {
            format!("{} keeps this {}.", e(*entity), if *diameter { "diameter" } else { "radius" })
        }
        ConstraintKind::Reference { .. } => format!("Measures {}.", owners(sketch, c)),
    }
}

/// The tooltip over a relation glyph.
pub fn tooltip(ui: &mut egui::Ui, sketch: &Sketch, c: &Constraint) {
    ui.label(egui::RichText::new(c.name()).family(theme::semibold()).color(theme::ink()));
    ui.label(sentence(sketch, c));
    ui.label(
        egui::RichText::new("Right-click to list every relation on this geometry").size(11.0).color(theme::muted()),
    );
}

/// Every constraint on any of `entities`, in the order the sketch holds them.
pub fn related<'a>(sketch: &'a Sketch, entities: &[EntityId]) -> Vec<&'a Constraint> {
    sketch.constraints.iter().filter(|c| c.entities().iter().any(|id| entities.contains(id))).collect()
}

/// Opens the menu for the relation `glyph`, right-clicked at `at`.
pub fn open(ctx: &egui::Context, s: &mut SketchSession, at: Pos2, glyph: ConstraintId) {
    let Some(c) = s.sketch.constraint(glyph) else { return };
    let popup = egui::Id::new("sketch_relations_menu").with(ctx.cumulative_frame_nr());
    s.relations_menu = Some(RelationsMenu { at, glyph, entities: c.entities(), hovered: None, popup });
    egui::Popup::open_id(ctx, popup);
}

/// Closes the menu, if it is open.
pub fn close(ctx: &egui::Context, s: &mut SketchSession) {
    if let Some(menu) = s.relations_menu.take() {
        egui::Popup::close_id(ctx, menu.popup);
        ctx.request_repaint();
    }
}

#[derive(Default)]
struct Outcome {
    hovered: Option<ConstraintId>,
    select: Option<ConstraintId>,
    delete: Option<ConstraintId>,
}

/// Shows the open menu over the viewport, on the layer `layer`. Returns the sketch edit a
/// Delete makes.
pub fn show(ui: &egui::Ui, s: &mut SketchSession, layer: egui::LayerId) -> Option<AppAction> {
    let ctx = ui.ctx().clone();
    let menu = s.relations_menu.clone()?;
    if !egui::Popup::is_id_open(&ctx, menu.popup) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        close(&ctx, s);
        return None;
    }
    let rows: Vec<Constraint> = related(&s.sketch, &menu.entities).into_iter().cloned().collect();
    if rows.is_empty() {
        close(&ctx, s);
        return None;
    }
    let frame = egui::Frame::new()
        .fill(GROUND)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(4)
        .shadow(egui::Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(102) });
    let session: &SketchSession = s;
    let outcome = egui::Popup::new(menu.popup, ctx.clone(), Rect::from_min_size(menu.at, egui::Vec2::ZERO), layer)
        .kind(egui::PopupKind::Menu)
        .open_memory(None)
        .frame(frame)
        .gap(4.0)
        .width(WIDTH)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| body(ui, session, &menu, &rows))
        .map(|shown| shown.inner)
        .unwrap_or_default();
    if let Some(m) = s.relations_menu.as_mut()
        && m.hovered != outcome.hovered
    {
        m.hovered = outcome.hovered;
        ctx.request_repaint();
    }
    if let Some(id) = outcome.select {
        s.selection = vec![SketchSel::Constraint(id)];
        close(&ctx, s);
    }
    let id = outcome.delete?;
    s.sketch.remove_constraint(id);
    s.selection.retain(|x| *x != SketchSel::Constraint(id));
    if let Some(m) = s.relations_menu.as_mut() {
        m.hovered = None;
    }
    solve::solve(&mut s.sketch, None, true);
    ctx.request_repaint();
    Some(AppAction::Command(Command::EditSketch { id: s.feature, sketch: s.sketch.clone() }))
}

fn body(ui: &mut egui::Ui, s: &SketchSession, menu: &RelationsMenu, rows: &[Constraint]) -> Outcome {
    let mut outcome = Outcome::default();
    let width = ui.available_width();
    let (head, heading) = ui.allocate_exact_size(vec2(width, HEADER), Sense::hover());
    let subtitle = menu.entities.iter().map(|id| entity_name(&s.sketch, *id)).collect::<Vec<_>>().join(", ");
    heading
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("Relations on {subtitle}")));
    let painter = ui.painter();
    let name = painter.layout_no_wrap("RELATIONS".into(), theme::font_semibold(10.5), MUTED);
    let subtitle_x = head.left() + 8.0 + name.size().x + 6.0;
    painter.galley(pos2(head.left() + 8.0, head.top() + 6.0), name, MUTED);
    painter.with_clip_rect(Rect::from_min_max(pos2(subtitle_x, head.top()), head.right_bottom())).text(
        pos2(subtitle_x, head.top() + 11.5),
        Align2::LEFT_CENTER,
        &subtitle,
        theme::font(10.5),
        MUTED,
    );
    let conflicting = match &s.sketch.status {
        SolveStatus::OverDefined { conflicting } => conflicting.clone(),
        _ => Vec::new(),
    };
    egui::ScrollArea::vertical().id_salt("sketch_relations_rows").max_height(MAX_LIST).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for c in rows {
            // Rows are keyed by their constraint, not their place: an undo while a button is
            // held moves the rows, and a positional id would hand the release to another row.
            let (rect, _) = ui.allocate_exact_size(vec2(width, ROW), Sense::hover());
            let row = ui.interact(rect, ui.id().with(("relation_row", c.id)), Sense::click());
            let label = format!("{} on {}", title(&s.sketch, c), owners(&s.sketch, c));
            let explanation = sentence(&s.sketch, c);
            row.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
            let delete_rect = Rect::from_min_max(
                pos2(rect.right() - DELETE_WIDTH - 4.0, rect.top() + 3.0),
                rect.right_bottom() - vec2(4.0, 3.0),
            );
            let delete = ui.interact(delete_rect, ui.id().with(("relation_delete", c.id)), Sense::click());
            delete.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Delete {label}")));
            let over = row.contains_pointer() || delete.contains_pointer();
            if over {
                outcome.hovered = Some(c.id);
            }
            let selected = s.selection.contains(&SketchSel::Constraint(c.id));
            let broken = conflicting.contains(&c.id) || c.is_dangling(&s.sketch);
            let painter = ui.painter();
            if over {
                painter.rect_filled(rect, CornerRadius::same(3), Color32::from_white_alpha(14));
            }
            if c.id == menu.glyph {
                // The relation whose glyph was clicked.
                painter.rect_filled(
                    Rect::from_min_size(rect.left_top() + vec2(0.0, 5.0), vec2(2.0, ROW - 10.0)),
                    CornerRadius::same(1),
                    AMBER,
                );
            }
            let look = if selected {
                GlyphLook::Selected
            } else if broken {
                GlyphLook::Broken
            } else if over {
                GlyphLook::Hovered
            } else {
                GlyphLook::Plain
            };
            paint_glyph(
                painter,
                &c.kind,
                Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(15.0, 15.0)),
                look,
            );
            let ink = if broken { DANGER_INK } else { INK };
            let name = painter.layout_no_wrap(title(&s.sketch, c), theme::font(12.0), ink);
            let text_left = rect.left() + 32.0;
            let text_right = delete_rect.left() - 6.0;
            let name_width = name.size().x;
            painter
                .with_clip_rect(Rect::from_min_max(pos2(text_left, rect.top()), pos2(text_right, rect.bottom())))
                .galley(pos2(text_left, rect.center().y - name.size().y / 2.0), name, ink);
            let owners_left = text_left + name_width + 8.0;
            if owners_left < text_right {
                painter
                    .with_clip_rect(Rect::from_min_max(pos2(owners_left, rect.top()), pos2(text_right, rect.bottom())))
                    .text(
                        pos2(owners_left, rect.center().y),
                        Align2::LEFT_CENTER,
                        owners(&s.sketch, c),
                        theme::font(10.5),
                        MUTED,
                    );
            }
            let (fill, text) =
                if delete.hovered() { (DANGER, Color32::WHITE) } else { (Color32::TRANSPARENT, DANGER_INK) };
            painter.rect_filled(delete_rect, CornerRadius::same(3), fill);
            painter.text(delete_rect.center(), Align2::CENTER_CENTER, "Delete", theme::font(11.5), text);
            if delete.clicked() {
                outcome.delete = Some(c.id);
            } else if row.clicked() {
                outcome.select = Some(c.id);
            }
            row.on_hover_text(explanation);
        }
    });
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::P2;

    #[test]
    fn every_relation_has_a_sentence_naming_its_geometry() {
        let mut s = Sketch::new();
        let a = s.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
        let b = s.add(EntityKind::Line { a: P2::new(0.0, 5.0), b: P2::new(10.0, 5.0) }, false);
        let circle = s.add(EntityKind::Circle { center: P2::new(20.0, 0.0), radius: 3.0 }, false);
        let parallel = s.constrain(ConstraintKind::Parallel { a, b });
        let meet = s.constrain(ConstraintKind::Coincident { a: PointRef::new(a, 0), b: PointRef::ORIGIN });
        let equal = s.constrain(ConstraintKind::Equal { a, b });
        let on = s.constrain(ConstraintKind::PointOn { point: PointRef::new(circle, 0), entity: a });
        let words = |id| sentence(&s, s.constraint(id).unwrap());
        assert_eq!(words(parallel), "Line 1 and Line 2 stay parallel.");
        assert_eq!(words(meet), "Line 1 start and Origin meet at one point.");
        assert_eq!(words(equal), "Line 1 and Line 2 keep the same length.");
        assert_eq!(words(on), "Circle 3 centre lies on Line 1.");
        let around: Vec<_> = related(&s, &[b]).into_iter().map(|c| c.id).collect();
        assert_eq!(around, vec![parallel, equal]);
    }
}
