//! Configuration tabs drawn as browser tabs. The selected tab joins the table under it;
//! a named tab closes from its own button or a middle click, renames on a double click
//! and moves when dragged; the plus after the last tab opens a new one.

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, pos2, vec2};

use crate::icons::{self, Icon};
use crate::theme;

/// The strip's height.
pub const HEIGHT: f32 = 34.0;
const MIN_WIDTH: f32 = 76.0;
const MAX_WIDTH: f32 = 200.0;
const EDIT_WIDTH: f32 = 168.0;
const PLUS: f32 = 34.0;

/// What a person asked of the strip. Indices count tabs, Default first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabEvent {
    /// Show this tab's configuration.
    Select(usize),
    /// Delete this named configuration.
    Close(usize),
    /// Rename this tab in place.
    Rename(usize),
    /// Copy this tab to a new one.
    Duplicate(usize),
    /// Move a named tab to another place among the named tabs.
    Move {
        /// Where it is.
        from: usize,
        /// Where it goes.
        to: usize,
    },
    /// Open a new tab.
    New,
}

/// A name field shown in a tab: in tab `at`, or in a new tab after the last one when
/// `at` is the number of tabs.
pub struct Editing<'a> {
    /// Which tab shows the field.
    pub at: usize,
    /// Draws the field in the space it is given.
    pub draw: &'a mut dyn FnMut(&mut Ui),
}

/// Draws the strip and returns what was asked of it.
pub fn show(ui: &mut Ui, id: Id, names: &[String], active: usize, mut editing: Option<Editing<'_>>) -> Vec<TabEvent> {
    let mut events = Vec::new();
    let editing_at = editing.as_ref().map(|e| e.at);
    let count = names.len() + usize::from(editing_at == Some(names.len()));
    let natural: Vec<f32> = (0..count)
        .map(|i| {
            if editing_at == Some(i) {
                return EDIT_WIDTH;
            }
            let text = ui.painter().layout_no_wrap(names[i].clone(), theme::font_semibold(12.0), Color32::PLACEHOLDER);
            (text.size().x + 66.0).clamp(MIN_WIDTH, MAX_WIDTH)
        })
        .collect();
    // Tabs narrow together, as a browser's do, before the strip scrolls.
    let room = ui.available_width() - PLUS - 12.0;
    let sum: f32 = natural.iter().sum();
    let scale = if sum > room { (room / sum).max(0.0) } else { 1.0 };
    let widths: Vec<f32> = natural
        .iter()
        .enumerate()
        .map(|(i, w)| if editing_at == Some(i) { *w } else { (w * scale).max(MIN_WIDTH) })
        .collect();
    let width = (widths.iter().sum::<f32>() + PLUS + 12.0).max(ui.available_width());
    egui::ScrollArea::horizontal().id_salt(id.with("scroll")).show(ui, |ui| {
        let (strip, _) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::hover());
        let painter = ui.painter().clone();
        painter.rect_filled(strip, CornerRadius::ZERO, theme::chrome());
        let base = strip.bottom() - 0.5;
        painter.line_segment([pos2(strip.left(), base), pos2(strip.right(), base)], Stroke::new(1.0, theme::line()));
        let mut x = strip.left() + 6.0;
        let rects: Vec<Rect> = widths
            .iter()
            .map(|w| {
                let rect = Rect::from_min_size(pos2(x, strip.top() + 5.0), vec2(*w, HEIGHT - 5.0));
                x += w;
                rect
            })
            .collect();
        for (i, rect) in rects.iter().copied().enumerate() {
            if editing_at == Some(i) {
                paint_tab(&painter, rect, true, false);
                paint_icon(&painter, rect, true);
                if let Some(editing) = editing.as_mut() {
                    let field =
                        Rect::from_min_max(pos2(rect.left() + 26.0, rect.top() + 4.0), rect.max - vec2(8.0, 4.0));
                    ui.scope_builder(UiBuilder::new().max_rect(field), |ui| (editing.draw)(ui));
                }
                continue;
            }
            tab(ui, id, &painter, (i, &names[i], i == active), (rect, &rects), &mut events);
        }
        for i in 0..count.saturating_sub(1) {
            let quiet = |j: usize| j != active && editing_at != Some(j) && !ui.rect_contains_pointer(rects[j]);
            if quiet(i) && quiet(i + 1) {
                let x = rects[i].right();
                painter.line_segment(
                    [pos2(x, rects[i].top() + 8.0), pos2(x, rects[i].bottom() - 7.0)],
                    Stroke::new(1.0, theme::line()),
                );
            }
        }
        let plus = Rect::from_center_size(pos2(x + 4.0 + 13.0, strip.center().y + 2.0), vec2(26.0, 26.0));
        let response = ui.interact(plus, id.with("new"), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New configuration"));
        #[cfg(target_arch = "wasm32")]
        crate::web::show_visible_control(ui, "New configuration", plus);
        let hot = response.hovered() || response.has_focus();
        if hot {
            painter.rect_filled(plus, CornerRadius::same(6), theme::hover());
        }
        icons::paint(
            &painter,
            Icon::Plus,
            Rect::from_center_size(plus.center(), vec2(14.0, 14.0)),
            if hot { theme::ink() } else { theme::muted() },
        );
        if response.on_hover_text("New configuration").clicked() {
            events.push(TabEvent::New);
        }
    });
    events
}

fn paint_tab(painter: &egui::Painter, rect: Rect, selected: bool, hovered: bool) {
    let top = CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 };
    if selected {
        painter.rect(rect, top, theme::panel(), Stroke::new(1.0, theme::line()), StrokeKind::Inside);
        // The selected tab opens into the table below, as a browser tab opens into its page.
        painter.line_segment(
            [pos2(rect.left() + 1.0, rect.bottom() - 0.5), pos2(rect.right() - 1.0, rect.bottom() - 0.5)],
            Stroke::new(2.0, theme::panel()),
        );
        painter.line_segment(
            [pos2(rect.left() + 6.0, rect.top() + 0.5), pos2(rect.right() - 6.0, rect.top() + 0.5)],
            Stroke::new(2.0, theme::accent()),
        );
    } else if hovered {
        let inner = Rect::from_min_max(rect.min + vec2(2.0, 2.0), rect.max - vec2(2.0, 4.0));
        painter.rect_filled(inner, CornerRadius::same(6), theme::hover());
    }
}

fn paint_icon(painter: &egui::Painter, rect: Rect, selected: bool) {
    let icon = Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), vec2(13.0, 13.0));
    icons::paint(painter, Icon::Config, icon, if selected { theme::accent() } else { theme::muted() });
}

/// One tab that is not being renamed.
fn tab(
    ui: &mut Ui,
    id: Id,
    painter: &egui::Painter,
    (i, name, selected): (usize, &str, bool),
    (rect, rects): (Rect, &[Rect]),
    events: &mut Vec<TabEvent>,
) {
    let named = i > 0;
    let sense = if named { Sense::click_and_drag() } else { Sense::click() };
    let response = ui.interact(rect, id.with(("tab", i)), sense);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, format!("Configuration {name}"))
    });
    #[cfg(target_arch = "wasm32")]
    crate::web::show_visible_control(ui, format!("Configuration {name}"), rect);
    // A dragged tab follows the pointer and drops between the others.
    let drag_id = id.with(("drag", i));
    let mut offset: f32 = ui.data(|d| d.get_temp(drag_id)).unwrap_or(0.0);
    if response.dragged() {
        offset += response.drag_delta().x;
        ui.data_mut(|d| d.insert_temp(drag_id, offset));
    }
    if response.drag_stopped() {
        let centre = rect.center().x + offset;
        let to = 1 + rects.iter().enumerate().skip(1).filter(|(j, r)| *j != i && r.center().x < centre).count();
        if to != i {
            events.push(TabEvent::Move { from: i, to: to.min(rects.len() - 1) });
        }
        offset = 0.0;
        ui.data_mut(|d| d.remove::<f32>(drag_id));
    }
    let shown = rect.translate(vec2(offset, 0.0));
    let hovered = ui.rect_contains_pointer(rect);
    paint_tab(painter, shown, selected, hovered || offset != 0.0);
    paint_icon(painter, shown, selected);

    let close_rect = Rect::from_center_size(pos2(shown.right() - 14.0, shown.center().y), vec2(18.0, 18.0));
    let text_right = if named { close_rect.left() - 2.0 } else { shown.right() - 10.0 };
    let mut job = LayoutJob::single_section(
        name.to_owned(),
        TextFormat {
            font_id: if selected { theme::font_semibold(12.0) } else { theme::font_medium(12.0) },
            color: if selected || hovered { theme::ink() } else { theme::muted() },
            ..Default::default()
        },
    );
    job.wrap = TextWrapping::truncate_at_width((text_right - shown.left() - 26.0).max(8.0));
    let galley = painter.layout_job(job);
    let text_pos = pos2(shown.left() + 26.0, shown.center().y - galley.size().y / 2.0);
    painter.galley(text_pos, galley, theme::ink());

    if named {
        let close = ui.interact(close_rect, id.with(("close", i)), Sense::click());
        close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Close {name}")));
        #[cfg(target_arch = "wasm32")]
        crate::web::show_visible_control(ui, format!("Close {name}"), close_rect);
        if selected || hovered || close.has_focus() {
            if close.hovered() || close.has_focus() {
                painter.circle_filled(close_rect.center(), 8.0, theme::hover());
            }
            let ink = if close.hovered() { theme::ink() } else { theme::muted() };
            icons::paint(painter, Icon::Close, close_rect.shrink(4.5), ink);
        }
        if close.on_hover_text(format!("Delete {name}; Undo brings it back")).clicked() {
            events.push(TabEvent::Close(i));
            return;
        }
    }
    let tip = if named {
        "Click to show. Double-click to rename, drag to reorder, right-click for more."
    } else {
        "The part's own values. Right-click to copy it to a new configuration."
    };
    let response = response.on_hover_text(tip);
    if named && response.double_clicked() {
        events.push(TabEvent::Rename(i));
    } else if response.clicked() && !selected {
        events.push(TabEvent::Select(i));
    } else if named && response.middle_clicked() {
        events.push(TabEvent::Close(i));
    }
    response.context_menu(|ui| {
        let last = rects.len() - 1;
        for (label, event, enabled) in [
            ("Rename", TabEvent::Rename(i), named),
            ("Duplicate", TabEvent::Duplicate(i), true),
            ("Move left", TabEvent::Move { from: i, to: i.saturating_sub(1) }, i > 1),
            ("Move right", TabEvent::Move { from: i, to: i + 1 }, named && i < last),
            ("Delete", TabEvent::Close(i), named),
        ] {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                events.push(event);
                ui.close();
            }
        }
    });
}
