//! The Edit Suite ring and command columns, measured from the approved L00 prototype.

use crate::commands::CommandSpec;
use crate::document::Selection;
use crate::{icons, theme};
use egui::{Align2, Color32, CornerRadius, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

const GROUND: Color32 = Color32::from_rgb(0x35, 0x38, 0x3e);
const INK: Color32 = Color32::from_rgb(0xe6, 0xe8, 0xec);
const MUTED: Color32 = Color32::from_rgb(0x9d, 0xa2, 0xab);
const LINE: Color32 = Color32::from_rgb(0x4a, 0x4e, 0x55);
const AMBER: Color32 = Color32::from_rgb(0xf5, 0xa5, 0x24);
const AMBER_INK: Color32 = Color32::from_rgb(0x23, 0x16, 0x00);
const DANGER: Color32 = Color32::from_rgb(0xe0, 0x3a, 0x2e);
const DANGER_INK: Color32 = Color32::from_rgb(0xff, 0x9d, 0x94);
const ROW: f32 = 26.0;
const HEADER: f32 = 26.0;
const PAD: f32 = 4.0;
const GAP: f32 = 4.0;
const PILL_HEIGHT: f32 = 28.0;
const POSITIONS: [(f32, f32); 8] =
    [(0.0, -1.0), (0.72, -0.7), (1.0, 0.0), (0.72, 0.7), (0.0, 1.0), (-0.72, 0.7), (-1.0, 0.0), (-0.72, -0.7)];

pub(crate) struct Item {
    pub command: &'static CommandSpec,
    pub label: String,
    pub enabled: bool,
    pub soon: bool,
    pub selected: bool,
    pub selection: Option<Selection>,
    pub shortcut: String,
    pub tip: String,
}

pub(crate) struct Column {
    pub label: &'static str,
    pub subtitle: String,
    pub items: Vec<Item>,
}

pub(crate) struct Menu {
    pub title: String,
    pub quick: [Item; 8],
    pub columns: Vec<Column>,
}

#[derive(Clone, Copy)]
pub(crate) struct Choice {
    pub command: &'static CommandSpec,
    pub selection: Option<Selection>,
}

pub(crate) struct Layout {
    pub bounds: Rect,
    pub ring: Rect,
    pub list: Rect,
    pub pills: [Rect; 8],
    pub columns: Vec<Rect>,
}

fn text_width(ctx: &egui::Context, text: &str, size: f32) -> f32 {
    ctx.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_owned(), theme::font(size), INK).size().x)
}

impl Layout {
    pub fn measure(ctx: &egui::Context, menu: &Menu, at: Pos2) -> Self {
        let pill_widths = std::array::from_fn(|i| text_width(ctx, &menu.quick[i].label, 12.0) + 40.0);
        let widths = menu
            .columns
            .iter()
            .map(|column| {
                column
                    .items
                    .iter()
                    .map(|item| {
                        let suffix = if item.soon {
                            text_width(ctx, "Coming soon", 10.5)
                        } else {
                            text_width(ctx, &item.shortcut, 11.0)
                        };
                        text_width(ctx, &item.label, 12.0) + suffix + 52.0
                    })
                    .fold(172.0, f32::max)
            })
            .collect::<Vec<_>>();
        let rows = menu.columns.iter().map(|column| column.items.len()).max().unwrap_or(0);
        Self::arrange(at, ctx.content_rect().shrink(6.0), pill_widths, &widths, HEADER + ROW * rows as f32 + 2.0 * PAD)
    }

    fn arrange(at: Pos2, available: Rect, pill_widths: [f32; 8], widths: &[f32], list_height: f32) -> Self {
        let ring_size = vec2(440.0_f32.min(available.width()), 236.0_f32.min((available.height() - 58.0).max(28.0)));
        let mut ring = inside(Rect::from_center_size(at, ring_size), available);
        let gaps = GAP * widths.len().saturating_sub(1) as f32;
        let total_width = widths.iter().sum::<f32>();
        let list_size = vec2(
            (total_width + gaps + 2.0 * PAD).min(available.width()),
            list_height.min((available.height() - ring.height() - GAP).max(28.0)),
        );
        let mut below = ring.bottom() + GAP + list_size.y <= available.bottom();
        if !below && ring.top() - GAP - list_size.y < available.top() {
            let shift_up = ring.bottom() + GAP + list_size.y - available.bottom();
            let shift_down = available.top() + GAP + list_size.y - ring.top();
            if shift_up <= shift_down {
                ring = ring.translate(vec2(0.0, -shift_up));
                below = true;
            } else {
                ring = ring.translate(vec2(0.0, shift_down));
            }
        }
        let y = if below { ring.bottom() + GAP } else { ring.top() - GAP - list_size.y };
        let list = inside(Rect::from_min_size(pos2(ring.center().x - list_size.x / 2.0, y), list_size), available);
        let horizontal_radius =
            150.0_f32.min((ring.width() - pill_widths.iter().copied().fold(0.0, f32::max)).max(0.0) / 2.0);
        let vertical_radius = 92.0_f32.min((ring.height() - PILL_HEIGHT).max(0.0) / 2.0);
        let pills = std::array::from_fn(|i| {
            Rect::from_center_size(
                ring.center() + vec2(POSITIONS[i].0 * horizontal_radius, POSITIONS[i].1 * vertical_radius),
                vec2(pill_widths[i].min(ring.width()), PILL_HEIGHT),
            )
        });
        let scale = ((list.width() - gaps - 2.0 * PAD) / total_width.max(1.0)).min(1.0);
        let mut x = list.left() + PAD;
        let columns = widths
            .iter()
            .map(|width| {
                let column =
                    Rect::from_min_size(pos2(x, list.top() + PAD), vec2(width * scale, list.height() - 2.0 * PAD));
                x += column.width() + GAP;
                column
            })
            .collect();
        Self { bounds: ring.union(list), ring, list, pills, columns }
    }
}

fn inside(rect: Rect, bounds: Rect) -> Rect {
    Rect::from_min_size(
        pos2(
            rect.left().clamp(bounds.left(), (bounds.right() - rect.width()).max(bounds.left())),
            rect.top().clamp(bounds.top(), (bounds.bottom() - rect.height()).max(bounds.top())),
        ),
        rect.size(),
    )
}

fn panel(painter: &egui::Painter, rect: Rect, radius: u8, blur: u8, offset: i8) {
    let frame = egui::Frame::new()
        .fill(GROUND)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(radius))
        .shadow(egui::Shadow { offset: [0, offset], blur, spread: 0, color: Color32::from_black_alpha(102) });
    painter.add(frame.paint(rect));
}

fn danger(item: &Item) -> bool {
    matches!(item.command.id, "delete" | "delete_feature" | "exit_sketch")
}

fn icon(item: &Item) -> icons::Icon {
    use icons::Icon;
    match item.command.id {
        "sketch" | "edit_sketch" => Icon::Sketch,
        "smart_dimension" | "show_dimensions" => Icon::SmartDim,
        "normal_to" => Icon::OnPlane,
        "measure" => Icon::Measure,
        "edit_feature" | "rename" => Icon::Props,
        "hide_body" => Icon::Eye,
        "isolate" => Icon::Fit,
        "select_other" | "select_tangent_chain" | "select_loop" => Icon::Views,
        "sketch_horizontal" | "sketch_vertical" | "sketch_fix" => Icon::Relation,
        "construction" => Icon::Construction,
        "sketch_trim" => Icon::Trim,
        "sketch_offset" => Icon::Offset,
        "sketch_select" => Icon::Sketch,
        "sketch_line" => Icon::Line,
        "sketch_rectangle" => Icon::Rectangle,
        "sketch_circle" => Icon::Circle,
        "sketch_arc" => Icon::Arc,
        "exit_sketch" | "delete" | "delete_feature" => Icon::Close,
        "origin_axes" => Icon::Origin,
        "origin_planes" | "reference_plane" => Icon::Plane,
        "fillet" => Icon::Fillet,
        "chamfer" => Icon::Chamfer,
        "export_step" => Icon::Save,
        "mass_properties" => Icon::Mass,
        "material" => Icon::Material,
        "appearance" => Icon::Display,
        _ => item.command.icon,
    }
}

fn button(ui: &mut egui::Ui, item: &Item, rect: Rect, id: Id, pill: bool) -> bool {
    let response = ui.interact(rect, id, if item.enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, item.enabled, &item.label));
    let highlighted = item.enabled && (response.hovered() || response.has_focus());
    let mut ink = if danger(item) { DANGER_INK } else { INK };
    let painter = ui.painter();
    if pill {
        panel(painter, rect, 14, 14, 4);
    }
    if highlighted {
        painter.rect_filled(
            rect,
            CornerRadius::same(if pill { 14 } else { 3 }),
            if danger(item) { DANGER } else { AMBER },
        );
        ink = if danger(item) { Color32::WHITE } else { AMBER_INK };
    } else if pill && danger(item) {
        painter.rect_stroke(
            rect,
            CornerRadius::same(14),
            Stroke::new(1.0, DANGER.gamma_multiply(0.6)),
            StrokeKind::Inside,
        );
    } else if item.selected {
        painter.rect_filled(rect, CornerRadius::same(3), Color32::from_white_alpha(12));
    }
    if !item.enabled {
        ink = ink.gamma_multiply(0.55);
    }
    let left = rect.left() + 8.0;
    icons::paint(painter, icon(item), Rect::from_center_size(pos2(left + 7.5, rect.center().y), vec2(15.0, 15.0)), ink);
    let suffix = if !pill && item.soon {
        "Coming soon"
    } else if !pill {
        &item.shortcut
    } else {
        ""
    };
    let suffix_width = text_width(ui.ctx(), suffix, 10.5);
    if !suffix.is_empty() {
        painter.text(
            pos2(rect.right() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            suffix,
            theme::font(10.5),
            if highlighted { ink } else { MUTED },
        );
    }
    let text_rect = Rect::from_min_max(
        pos2(left + 21.0, rect.top()),
        pos2(rect.right() - 8.0 - if suffix.is_empty() { 0.0 } else { suffix_width + 9.0 }, rect.bottom()),
    );
    painter.with_clip_rect(text_rect).text(
        pos2(text_rect.left(), rect.center().y),
        Align2::LEFT_CENTER,
        &item.label,
        theme::font(12.0),
        ink,
    );
    let key = response.has_focus()
        && ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                || input.consume_key(egui::Modifiers::NONE, egui::Key::Space)
        });
    let clicked = item.enabled && (response.clicked() || key);
    response.on_hover_text(&item.tip);
    clicked
}

pub(crate) fn show(ui: &mut egui::Ui, menu: &Menu, layout: &Layout, opening: bool) -> Option<Choice> {
    let offset = ui.min_rect().min - layout.bounds.min;
    ui.set_min_size(layout.bounds.size());
    let ring = layout.ring.translate(offset);
    let list = layout.list.translate(offset);
    let painter = ui.painter().clone();
    let radius = 75.0_f32.min(ring.height() / 2.0 - 18.0);
    painter.circle_filled(ring.center(), radius, Color32::from_rgba_unmultiplied(34, 36, 39, 46));
    let points: Vec<_> =
        (0..=96).map(|i| ring.center() + Vec2::angled(i as f32 * std::f32::consts::TAU / 96.0) * radius).collect();
    painter.add(egui::Shape::dashed_line(&points, Stroke::new(1.5, AMBER.gamma_multiply(0.55)), 4.5, 4.5));
    painter.circle_filled(ring.center(), 4.0, AMBER);
    let caption =
        ui.ctx().fonts_mut(|fonts| fonts.layout_no_wrap(menu.title.clone(), theme::font(10.5), Color32::WHITE));
    let caption_rect =
        Rect::from_center_size(ring.center() + vec2(0.0, 23.0), vec2((caption.size().x + 14.0).min(180.0), 18.0));
    painter.rect_filled(caption_rect, CornerRadius::same(8), Color32::from_rgba_unmultiplied(23, 24, 27, 217));
    painter.with_clip_rect(caption_rect.shrink2(vec2(7.0, 0.0))).galley(
        pos2(caption_rect.left() + 7.0, caption_rect.center().y - caption.size().y / 2.0),
        caption,
        Color32::WHITE,
    );
    let mut choice = None;
    let mut focus = opening;
    let quick_group = ui.scope_builder(egui::UiBuilder::new().max_rect(ring), |ui| {
        ui.set_min_size(ring.size());
        for (index, item) in menu.quick.iter().enumerate() {
            let rect = layout.pills[index].translate(offset);
            let id = ui.id().with(("quick", index, item.command.id));
            if button(ui, item, rect, id, true) {
                choice = Some(Choice { command: item.command, selection: item.selection });
            }
            if focus && item.enabled {
                ui.ctx().memory_mut(|memory| memory.request_focus(id));
                focus = false;
            }
        }
    });
    quick_group.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Quick commands"));
    panel(&painter, list, 8, 32, 12);
    for (index, column) in menu.columns.iter().enumerate() {
        let rect = layout.columns[index].translate(offset);
        if index > 0 {
            painter.vline(rect.left() - 2.0, rect.y_range(), Stroke::new(1.0, LINE));
        }
        let heading_rect = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
        let heading = ui.allocate_rect(heading_rect, Sense::hover());
        heading.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, column.label));
        let name = column.label.to_uppercase();
        let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_no_wrap(name, theme::font_semibold(10.5), MUTED));
        let subtitle_x = rect.left() + 8.0 + galley.size().x + 6.0;
        painter.galley(pos2(rect.left() + 8.0, rect.top() + 6.0), galley, MUTED);
        painter.with_clip_rect(Rect::from_min_max(pos2(subtitle_x, rect.top()), rect.right_bottom())).text(
            pos2(subtitle_x, rect.top() + 11.5),
            Align2::LEFT_CENTER,
            &column.subtitle,
            theme::font(10.5),
            MUTED,
        );
        let body = Rect::from_min_max(rect.min + vec2(0.0, HEADER), rect.max);
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                egui::ScrollArea::vertical().id_salt(("context_column", index)).max_height(body.height()).show(
                    ui,
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (row, item) in column.items.iter().enumerate() {
                            let (row_rect, _) = ui.allocate_exact_size(vec2(body.width(), ROW), Sense::hover());
                            if button(ui, item, row_rect, ui.id().with(("command", row, item.command.id)), false) {
                                choice = Some(Choice { command: item.command, selection: item.selection });
                            }
                        }
                    },
                );
            },
        );
    }
    choice
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_and_columns_fit_at_window_edges_without_overlapping() {
        for size in [vec2(1440.0, 900.0), vec2(1280.0, 720.0), vec2(800.0, 600.0)] {
            let bounds = Rect::from_min_size(Pos2::ZERO, size).shrink(6.0);
            for point in
                [bounds.center(), bounds.left_top(), bounds.right_top(), bounds.left_bottom(), bounds.right_bottom()]
            {
                let layout = Layout::arrange(point, bounds, [126.0; 8], &[186.0, 172.0, 192.0], 216.0);
                assert!(bounds.contains_rect(layout.bounds), "{size:?} {point:?}");
                assert!(!layout.ring.intersects(layout.list));
                for pill in layout.pills {
                    assert!(bounds.contains_rect(pill));
                }
                for column in layout.columns {
                    assert!(layout.list.contains_rect(column));
                }
            }
        }
        let bounds = Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0));
        let at = pos2(720.0, 450.0);
        let layout = Layout::arrange(at, bounds, [126.0; 8], &[186.0, 172.0, 192.0], 216.0);
        assert_eq!(layout.ring.center(), at);
        assert_eq!(layout.pills.len(), 8);
    }
}
