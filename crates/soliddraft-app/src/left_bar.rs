//! The narrow icon strip at the window's left edge, left of the tool palette. Its first
//! icons choose what the side area shows: the feature tree, then the Values table. A
//! line divides them from the document's workspaces below. Every icon is named on
//! hover, and the active ones are marked.
//!
//! A pane icon shows its pane in the side area, and hides the area when that pane
//! already shows there. A workspace icon does what the workspace tab in the title bar
//! did.

use crate::commands::Tab;
use crate::drawing::DrawingTab;
use crate::icons::{self, Icon};
use crate::panels::Pane;
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

/// The icon a pane shows in the strip.
pub fn pane_icon(pane: Pane) -> Icon {
    match pane {
        Pane::Tree => Icon::Tree,
        Pane::Values => Icon::Config,
    }
}

/// The name a pane's icon carries for the active document.
pub fn pane_label(state: &AppState, pane: Pane) -> &'static str {
    match pane {
        Pane::Tree if state.assembly.is_active() => "Assembly tree",
        Pane::Tree if state.drawing.is_active() => "Drawing tree",
        Pane::Tree => "Feature tree",
        Pane::Values => "Values",
    }
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
    for pane in Pane::ALL {
        let label = pane_label(state, pane);
        let hint = match (pane, state.shortcuts.shortcut_text("toggle_tree")) {
            (Pane::Tree, Some(key)) => format!("{label} ({key})"),
            _ => label.to_owned(),
        };
        if button(ui, edge, pane_icon(pane), label, &hint, true, state.pane_shown(pane)) {
            state.toggle_pane(pane);
        }
    }
    ui.add_space(2.0);
    let (line, _) = ui.allocate_exact_size(vec2(BUTTON, 1.0), Sense::hover());
    ui.painter().hline(line.x_range().shrink(4.0), line.center().y, Stroke::new(1.0, theme::line()));
    ui.add_space(2.0);
    for tab in state.workspaces() {
        let selected = state.workspace() == tab;
        let enabled = !locked || selected;
        let hint = if enabled { tab.label().to_owned() } else { "Close sketch to change workspace".to_owned() };
        if button(ui, edge, icon(tab), &format!("{} tab", tab.label()), &hint, enabled, selected) {
            state.select_workspace(tab);
        }
    }
}

/// One icon of the strip, with `label` for assistive technology and `hint` on hover.
/// Returns true when clicked.
fn button(ui: &mut egui::Ui, edge: f32, icon: Icon, label: &str, hint: &str, enabled: bool, selected: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(vec2(BUTTON, BUTTON), if enabled { Sense::click() } else { Sense::hover() });
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label));
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
    icons::paint(painter, icon, Rect::from_center_size(rect.center(), vec2(ICON, ICON)), color);
    if enabled && response.has_focus() {
        painter.rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0, theme::accent()), StrokeKind::Inside);
    }
    let mut tip = egui::Tooltip::for_enabled(&response);
    tip.popup = tip.popup.align(egui::RectAlign::RIGHT);
    tip.show(|ui| ui.label(hint));
    enabled && response.clicked()
}
