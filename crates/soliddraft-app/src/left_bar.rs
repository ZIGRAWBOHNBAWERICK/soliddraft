//! D1 Clear Bench slim 46px left toggle rail: `Tree`, `History`, `Values`, `Props`, plus a
//! bottom `Clear` action when any pane is open. Clicking a rail button toggles that pane in
//! the single stacked side column; turning all toggles off leaves the side column completely
//! clear (0px).

use crate::commands::Tab;
use crate::drawing::DrawingTab;
use crate::icons::{self, Icon};
use crate::panels::Pane;
use crate::state::{AppState, RibbonTab};
use crate::theme;
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

/// The slim left rail's width in logical pixels (D1 Clear Bench: 46px).
pub const WIDTH: f32 = theme::clear_bench::SPACING.rail_width;
const BUTTON_W: f32 = 38.0;
const BUTTON_H: f32 = 42.0;
const ICON: f32 = 17.0;
/// Active indicator bar at the rail's outer left edge.
const MARK: f32 = 2.5;

pub fn panel(root: &mut egui::Ui, state: &mut AppState) {
    egui::Panel::left("left_bar")
        .exact_size(WIDTH)
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::chrome()).inner_margin(egui::Margin::symmetric(4, 8)))
        .show_separator_line(false)
        .show(root, |ui| show(ui, state));
}

/// The icon a pane shows in the left rail.
pub fn pane_icon(pane: Pane) -> Icon {
    match pane {
        Pane::Tree => Icon::Tree,
        Pane::History => Icon::History,
        Pane::Values => Icon::Config,
        Pane::Props => Icon::Props,
    }
}

/// Short visible label painted under the icon on the slim rail.
pub fn short_label(pane: Pane) -> &'static str {
    match pane {
        Pane::Tree => "Tree",
        Pane::History => "History",
        Pane::Values => "Values",
        Pane::Props => "Props",
    }
}

/// Accessible name a pane's rail button carries for the active document.
pub fn pane_label(state: &AppState, pane: Pane) -> &'static str {
    match pane {
        Pane::Tree if state.assembly.is_active() => "Assembly tree",
        Pane::Tree if state.drawing.is_active() => "Drawing tree",
        Pane::Tree => "Feature tree",
        Pane::History => "History",
        Pane::Values => "Values",
        Pane::Props => "Props",
    }
}

/// The icon a workspace tab shows (kept for callers & tests).
pub fn icon(tab: Tab) -> Icon {
    match tab {
        Tab::Part(RibbonTab::Solid) | Tab::Drawing(DrawingTab::Views) => Icon::Views,
        Tab::Part(RibbonTab::Sketch) => Icon::Sketch,
        Tab::Part(RibbonTab::Assembly) | Tab::Assembly => Icon::Assembly,
        Tab::Part(RibbonTab::Inspect) | Tab::AssemblyInspect => Icon::Measure,
        Tab::Part(RibbonTab::View) | Tab::AssemblyView | Tab::Drawing(DrawingTab::View) => Icon::Eye,
        Tab::Drawing(DrawingTab::Annotate) => Icon::SmartDim,
        Tab::Drawing(DrawingTab::Sheet) => Icon::Sheet,
        Tab::Drawing(DrawingTab::Assistant) => Icon::Spark,
    }
}

fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let rail_rect = ui.max_rect().expand2(vec2(4.0, 8.0));
    ui.painter().vline(rail_rect.right() - 0.5, rail_rect.y_range(), Stroke::new(1.0, theme::line()));

    let edge = rail_rect.left();
    ui.spacing_mut().item_spacing = vec2(0.0, 4.0);

    for pane in Pane::ALL {
        let accessible = pane_label(state, pane);
        let short = short_label(pane);
        let hint = match (pane, state.shortcuts.shortcut_text("toggle_tree")) {
            (Pane::Tree, Some(key)) => format!("{accessible} ({key})"),
            _ => format!("{accessible} · click to stack or hide"),
        };
        if rail_button(ui, edge, pane_icon(pane), short, accessible, &hint, state.pane_shown(pane)) {
            state.toggle_pane(pane);
        }
    }

    // Bottom "Clear" button whenever at least one pane is open (D1 step_208 / step_209).
    if state.tree_shown() {
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(2.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON_W, 22.0), Sense::click());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Clear panels"));
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(5), theme::hover());
            }
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Clear",
                theme::font_semibold(9.5),
                if resp.hovered() { theme::ink() } else { theme::muted() },
            );
            if resp.on_hover_text("Close all side panels").clicked() {
                state.clear_panes();
            }
        });
    }
}

/// One stacked icon-over-label toggle on the slim left rail.
fn rail_button(
    ui: &mut egui::Ui,
    edge: f32,
    icon: Icon,
    short: &str,
    accessible: &str,
    hint: &str,
    selected: bool,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(BUTTON_W, BUTTON_H), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, accessible));
    let hovered = response.hovered() || response.has_focus();
    let painter = ui.painter();
    if selected {
        painter.rect(rect, CornerRadius::same(7), theme::panel(), Stroke::new(1.0, theme::line()), StrokeKind::Inside);
        let mark = Rect::from_min_max(pos2(edge, rect.top() + 7.0), pos2(edge + MARK, rect.bottom() - 7.0));
        painter.rect_filled(mark, CornerRadius::same(1), theme::accent());
    } else if hovered {
        painter.rect_filled(rect, CornerRadius::same(7), theme::hover());
    }
    let color = if selected {
        theme::accent()
    } else if hovered {
        theme::ink()
    } else {
        theme::muted()
    };
    let icon_rect = Rect::from_center_size(pos2(rect.center().x, rect.top() + 14.5), vec2(ICON, ICON));
    icons::paint(painter, icon, icon_rect, color);
    painter.text(
        pos2(rect.center().x, rect.bottom() - 8.5),
        Align2::CENTER_CENTER,
        short,
        theme::font_semibold(theme::typography().rail_label),
        color,
    );
    if response.has_focus() {
        painter.rect_stroke(rect, CornerRadius::same(7), Stroke::new(1.5, theme::accent()), StrokeKind::Inside);
    }
    let mut tip = egui::Tooltip::for_enabled(&response);
    tip.popup = tip.popup.align(egui::RectAlign::RIGHT);
    tip.show(|ui| {
        ui.label(egui::RichText::new(hint).color(Color32::PLACEHOLDER));
    });
    response.clicked()
}
