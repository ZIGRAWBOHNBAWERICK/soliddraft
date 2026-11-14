//! The narrow icon strip at the window's left edge, left of the tool palette. It holds
//! the document's workspaces: one icon each, named on hover, with the active one marked.
//! Selecting an icon does what the workspace tab in the title bar did.

use crate::commands::Tab;
use crate::drawing::DrawingTab;
use crate::icons::{self, Icon};
use crate::state::{AppState, RibbonTab};
use crate::theme;
use egui::{CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

/// The strip's width, its edge line included.
pub const WIDTH: f32 = 40.0;
const BUTTON: f32 = 32.0;
const ICON: f32 = 18.0;
/// The bar that marks the active icon, at the strip's outer edge.
const MARK: f32 = 2.0;

pub fn panel(root: &mut egui::Ui, state: &mut AppState) {
    egui::Panel::left("left_bar")
        .exact_size(WIDTH)
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::panel_header()).inner_margin(egui::Margin::symmetric(4, 6)))
        .show(root, |ui| show(ui, state));
}

/// The icon a workspace shows in the strip.
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
    // An open sketch keeps the part on its Sketch workspace until the sketch closes.
    let locked = state.in_sketch() && !state.drawing.is_active() && !state.assembly.is_active();
    if locked {
        state.ribbon_tab = RibbonTab::Sketch;
    }
    let edge = ui.max_rect().left() - 4.0;
    ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
    for tab in state.workspaces() {
        let selected = state.workspace() == tab;
        let enabled = !locked || selected;
        let (rect, response) =
            ui.allocate_exact_size(vec2(BUTTON, BUTTON), if enabled { Sense::click() } else { Sense::hover() });
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, format!("{} tab", tab.label()))
        });
        let active = enabled && (response.hovered() || response.has_focus());
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, CornerRadius::same(4), theme::selection_bg());
            let mark = Rect::from_min_max(pos2(edge, rect.top() + 4.0), pos2(edge + MARK, rect.bottom() - 4.0));
            painter.rect_filled(mark, CornerRadius::same(1), theme::accent());
        } else if active {
            painter.rect_filled(rect, CornerRadius::same(4), theme::hover());
        }
        let color = if selected {
            theme::selection_ink()
        } else if active {
            theme::ink()
        } else if enabled {
            theme::muted()
        } else {
            theme::muted().gamma_multiply(0.45)
        };
        icons::paint(painter, icon(tab), Rect::from_center_size(rect.center(), vec2(ICON, ICON)), color);
        if enabled && response.has_focus() {
            painter.rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, theme::accent()), StrokeKind::Inside);
        }
        let hint = if enabled { tab.label().to_owned() } else { "Close sketch to change workspace".to_owned() };
        let mut tip = egui::Tooltip::for_enabled(&response);
        tip.popup = tip.popup.align(egui::RectAlign::RIGHT);
        tip.show(|ui| ui.label(hint));
        if enabled && response.clicked() {
            state.select_workspace(tab);
        }
    }
}
