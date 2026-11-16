//! The relations strip that opens when Ctrl (Cmd on macOS) is let go after picks were
//! added to the sketch selection. It holds a button for each relation the selection can
//! take, from the same offer list as the ribbon's Relations button, then Smart Dimension
//! when the two picks make a dimension. Each button's tooltip names it. Esc, a click
//! elsewhere or a button closes the strip; a relation is added as one undo step.

use super::{GlyphLook, applicable_relation_sets, dimension_pair, dimension_word, paint_glyph};
use crate::icons;
use crate::ring_menu::{AMBER, AMBER_INK, GROUND, INK, LINE};
use crate::sketch::{ConstraintKind, Sketch};
use crate::state::{AppAction, SketchSel, SketchSession, Tool};
use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, pos2, vec2};

/// A button's side.
const BUTTON: f32 = 28.0;
/// The strip opens this far up and right of the last pick, clear of the pointer.
const OFFSET: egui::Vec2 = vec2(10.0, -10.0);

/// The strip, while it is open.
#[derive(Clone, Debug, PartialEq)]
pub struct PairMenu {
    /// The last pick, where the strip opens.
    pub at: Pos2,
    /// The selection it was opened for; any change to it closes the strip.
    selection: Vec<SketchSel>,
    /// Each opening is its own popup, so a strip closed by a click is never revived.
    popup: egui::Id,
}

/// One button of the strip.
#[derive(Clone, Debug, PartialEq)]
pub enum Offer {
    /// A relation the selection can take; `applied` when it has it already.
    Relation { name: &'static str, kinds: Vec<ConstraintKind>, applied: bool },
    /// Smart Dimension with the two picks taken: an "angle" or a "distance".
    Dimension { word: &'static str },
}

impl Offer {
    /// What the button's tooltip and accessibility label call it.
    pub fn name(&self) -> &'static str {
        match self {
            Offer::Relation { name, .. } => name,
            Offer::Dimension { .. } => "Smart Dimension",
        }
    }

    fn tip(&self) -> String {
        match self {
            Offer::Relation { name, applied: true, .. } => format!("{name}: already applied"),
            Offer::Relation { name, .. } => (*name).to_owned(),
            Offer::Dimension { word } => format!("Smart Dimension: place the {word}"),
        }
    }
}

/// The buttons for `selection`: every relation the existing offer list gives it, then
/// Smart Dimension when the picks make one.
pub fn offers(sketch: &Sketch, selection: &[SketchSel]) -> Vec<Offer> {
    let mut out: Vec<Offer> = applicable_relation_sets(sketch, selection)
        .into_iter()
        .map(|(name, kinds)| {
            let applied = kinds.iter().all(|kind| sketch.constraints.iter().any(|c| c.kind == *kind));
            Offer::Relation { name, kinds, applied }
        })
        .collect();
    if let Some(pair) = dimension_pair(sketch, selection) {
        out.push(Offer::Dimension { word: dimension_word(sketch, pair) });
    }
    out
}

/// Opens the strip at `at` for the current selection, when it holds two or more picks
/// of geometry and something applies to them.
pub fn open(ctx: &egui::Context, s: &mut SketchSession, at: Pos2) {
    close(ctx, s);
    let picks = s.selection.iter().filter(|pick| !matches!(pick, SketchSel::Constraint(_))).count();
    if picks < 2 || offers(&s.sketch, &s.selection).is_empty() {
        return;
    }
    let popup = egui::Id::new("sketch_pair_menu").with(ctx.cumulative_frame_nr());
    s.pair_menu = Some(PairMenu { at, selection: s.selection.clone(), popup });
    egui::Popup::open_id(ctx, popup);
    ctx.request_repaint();
}

/// Closes the strip, if it is open.
pub fn close(ctx: &egui::Context, s: &mut SketchSession) {
    if let Some(menu) = s.pair_menu.take() {
        egui::Popup::close_id(ctx, menu.popup);
        ctx.request_repaint();
    }
}

/// Shows the open strip on the layer `layer`. Returns what a button asks for.
pub fn show(ui: &egui::Ui, s: &mut SketchSession, layer: egui::LayerId) -> Option<AppAction> {
    let ctx = ui.ctx().clone();
    let menu = s.pair_menu.clone()?;
    let offers = offers(&s.sketch, &s.selection);
    if !egui::Popup::is_id_open(&ctx, menu.popup)
        || s.selection != menu.selection
        || s.tool != Tool::Select
        || offers.is_empty()
        || ctx.input(|i| i.key_pressed(egui::Key::Escape))
    {
        close(&ctx, s);
        return None;
    }
    let frame = egui::Frame::new()
        .fill(GROUND)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(4)
        .shadow(egui::Shadow { offset: [0, 12], blur: 32, spread: 0, color: Color32::from_black_alpha(102) });
    let chosen =
        egui::Popup::new(menu.popup, ctx.clone(), Rect::from_min_size(menu.at + OFFSET, egui::Vec2::ZERO), layer)
            .kind(egui::PopupKind::Menu)
            .open_memory(None)
            .frame(frame)
            .gap(0.0)
            .align(egui::RectAlign::TOP_START)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| strip(ui, &offers))
            .and_then(|shown| shown.inner)?;
    close(&ctx, s);
    Some(match chosen {
        Offer::Relation { kinds, .. } => AppAction::SketchRelationSet(kinds),
        Offer::Dimension { .. } => AppAction::SketchTool(Tool::Dimension),
    })
}

fn strip(ui: &mut egui::Ui, offers: &[Offer]) -> Option<Offer> {
    let mut chosen = None;
    let strip = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (index, offer) in offers.iter().enumerate() {
            if index > 0 && matches!(offer, Offer::Dimension { .. }) {
                let (rect, _) = ui.allocate_exact_size(vec2(7.0, BUTTON), Sense::hover());
                ui.painter().vline(rect.center().x, rect.y_range().shrink(4.0), Stroke::new(1.0, LINE));
            }
            if button(ui, offer) {
                chosen = Some(offer.clone());
            }
        }
    });
    strip
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Relations for the selection"));
    chosen
}

fn button(ui: &mut egui::Ui, offer: &Offer) -> bool {
    let enabled = !matches!(offer, Offer::Relation { applied: true, .. });
    let (rect, response) =
        ui.allocate_exact_size(vec2(BUTTON, BUTTON), if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, offer.name()));
    let hot = enabled && (response.hovered() || response.has_focus());
    let painter = ui.painter();
    if hot {
        painter.rect_filled(rect, CornerRadius::same(4), AMBER);
    }
    let glyph = Rect::from_center_size(pos2(rect.center().x, rect.center().y), vec2(15.0, 15.0));
    match offer {
        Offer::Relation { kinds, .. } => {
            paint_glyph(painter, &kinds[0], glyph, if hot { GlyphLook::Selected } else { GlyphLook::Plain });
            if !enabled {
                painter.rect_filled(glyph.expand(1.0), CornerRadius::same(3), GROUND.gamma_multiply(0.6));
            }
        }
        Offer::Dimension { .. } => {
            icons::paint(painter, icons::Icon::SmartDim, glyph, if hot { AMBER_INK } else { INK })
        }
    }
    let clicked = response.clicked();
    response.on_hover_text(offer.tip());
    enabled && clicked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{EntityKind, P2};

    fn names(offers: &[Offer]) -> Vec<&'static str> {
        offers.iter().map(Offer::name).collect()
    }

    #[test]
    fn two_lines_offer_their_relations_and_an_angle_and_a_line_and_an_arc_offer_tangent() {
        let mut s = Sketch::new();
        let a = s.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
        let b = s.add(EntityKind::Line { a: P2::new(0.0, 5.0), b: P2::new(8.0, 12.0) }, false);
        let arc = s.add(
            EntityKind::Arc { center: P2::new(20.0, 5.0), start: P2::new(25.0, 5.0), end: P2::new(20.0, 10.0) },
            false,
        );
        let lines = offers(&s, &[SketchSel::Entity(a), SketchSel::Entity(b)]);
        assert_eq!(names(&lines), ["Parallel", "Perpendicular", "Equal", "Collinear", "Smart Dimension"]);
        assert_eq!(lines.last(), Some(&Offer::Dimension { word: "angle" }));
        let tangent = offers(&s, &[SketchSel::Entity(a), SketchSel::Entity(arc)]);
        assert_eq!(names(&tangent), ["Tangent"]);
        s.constrain(ConstraintKind::Parallel { a, b });
        let applied = offers(&s, &[SketchSel::Entity(a), SketchSel::Entity(b)]);
        assert!(matches!(applied[0], Offer::Relation { name: "Parallel", applied: true, .. }));
        assert!(matches!(applied[1], Offer::Relation { name: "Perpendicular", applied: false, .. }));
    }
}
