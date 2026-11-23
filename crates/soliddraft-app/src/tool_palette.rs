//! D1 Clear Bench top tool strip: workspace tab row (`Solid`, `Sketch`, `Assembly`,
//! `Inspect`, `View` or the active document's workspaces) above a 58px icon-over-label
//! tool strip with a primary cobalt `Start sketch` CTA and a red `Close sketch` button
//! in sketch mode. Keeps the panel id `"tool_palette"` and `ToolDrag` chip drag support.

use crate::commands::{self, CommandSpec, Tab};
use crate::icons;
use crate::state::{AppAction, AppState, RibbonTab};
use crate::theme;
use egui::{Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

/// Total height of the workspace tab strip + icon-over-label tool strip.
pub const HEIGHT: f32 =
    theme::clear_bench::SPACING.workspace_strip_height + theme::clear_bench::SPACING.tool_strip_height;

pub fn panel(root: &mut egui::Ui, state: &mut AppState) {
    egui::Panel::top("tool_palette")
        .exact_size(HEIGHT)
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::ribbon()).inner_margin(egui::Margin::ZERO))
        .show_separator_line(false)
        .show(root, |ui| show(ui, state));
}

fn label(command: &CommandSpec, state: &AppState) -> String {
    match command.id {
        "sketch" if !state.in_sketch() => "Start sketch".to_owned(),
        "smart_dimension" => "Smart Dimension".to_owned(),
        _ => command.label_in(state),
    }
}

/// Short label painted beneath the 20px icon so cards stay crisp and single-line.
fn card_caption(command: &CommandSpec, state: &AppState) -> String {
    match command.id {
        "sketch" if !state.in_sketch() => "New Sketch".to_owned(),
        "smart_dimension" => "Smart Dim".to_owned(),
        _ => command.label_in(state),
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let full_rect = ui.max_rect();
    let ws_h = theme::spacing().workspace_strip_height;
    let strip_h = theme::spacing().tool_strip_height;

    // An open sketch keeps the part on its Sketch workspace until closed.
    let locked = state.in_sketch() && !state.drawing.is_active() && !state.assembly.is_active();
    if locked {
        state.ribbon_tab = RibbonTab::Sketch;
    }

    // 1. Workspace tab strip (28px)
    ui.allocate_ui_with_layout(vec2(full_rect.width(), ws_h), egui::Layout::left_to_right(egui::Align::BOTTOM), |ui| {
        ui.add_space(10.0);
        ui.spacing_mut().item_spacing.x = 2.0;
        let current = state.workspace();
        for tab in state.workspaces() {
            let selected = current == tab;
            let enabled = !locked || selected;
            let hint = if enabled { tab.label().to_owned() } else { "Close sketch to change workspace".to_owned() };
            let resp = workspace_tab(ui, tab.label(), &format!("{} tab", tab.label()), &hint, enabled, selected);
            if enabled && resp.clicked() {
                state.select_workspace(tab);
            }
        }
    });
    ui.painter().hline(full_rect.x_range(), full_rect.top() + ws_h - 0.5, Stroke::new(1.0, theme::line_faint()));

    // 2. Icon-over-label tool strip (58px)
    let tab = state.workspace();
    let mut pending: Option<&'static CommandSpec> = None;
    let mut relations_anchor = Rect::NOTHING;

    ui.allocate_ui_with_layout(
        vec2(full_rect.width(), strip_h),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.add_space(10.0);
            // In sketch mode, red Close sketch + solve status pill sit at the far right (step_139).
            if locked {
                if close_sketch_button(ui).clicked() {
                    state.push(AppAction::ExitSketch);
                }
                ui.add_space(6.0);
                if let Some(sketch) = &state.sketch {
                    sketch_status_pill(ui, &sketch.sketch.status);
                }
                ui.add_space(8.0);
            }

            // Icon-over-label tool cards, left to right. A tab with more tools than the
            // strip is wide keeps the first tools of each group as cards and puts the rest
            // of the group in a More menu at its end, so no tool is scrolled out of sight.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add_space(10.0);
                let start = matches!(tab, Tab::Part(RibbonTab::Solid | RibbonTab::Sketch)) && !state.in_sketch();
                let mut lead: Vec<(&'static CommandSpec, bool)> = Vec::new();
                if start {
                    lead.push((commands::get("sketch").expect("the registry has Start sketch"), true));
                }
                if tab == Tab::Part(RibbonTab::Sketch) {
                    lead.push((commands::get("smart_dimension").expect("the registry has Smart Dimension"), false));
                }
                let mut groups: Vec<(&'static str, Vec<&'static CommandSpec>)> = Vec::new();
                for (group, commands) in commands::ribbon(tab) {
                    if state.drawing.is_active() && group == "Assistant" {
                        continue;
                    }
                    if state.assembly.is_active() && tab == Tab::Assembly && group == "View" {
                        continue;
                    }
                    let commands: Vec<_> = commands
                        .into_iter()
                        .map(|(command, _)| command)
                        .filter(|command| {
                            (command.shown)(state)
                                && !(start && command.id == "sketch")
                                && command.id != "smart_dimension"
                                && !(state.in_sketch() && matches!(command.id, "sketch" | "exit_sketch"))
                                && !(state.assembly.is_active()
                                    && tab == Tab::Assembly
                                    && command.id == "assembly_interference")
                        })
                        .collect();
                    if !commands.is_empty() {
                        groups.push((group, commands));
                    }
                }
                let lead_width: f32 = lead.iter().map(|(c, primary)| card_width(ui, state, c, *primary) + GAP).sum();
                let widths: Vec<Vec<f32>> =
                    groups.iter().map(|(_, cs)| cs.iter().map(|c| card_width(ui, state, c, false)).collect()).collect();
                let shown = fit(&widths, lead_width, !lead.is_empty(), ui.available_width() - 10.0);

                egui::ScrollArea::horizontal()
                    .id_salt(("d1_tool_strip", state.drawing.is_active(), state.assembly.is_active(), tab.id()))
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            ui.spacing_mut().item_spacing.x = GAP;
                            let mut drew_any = false;
                            for (command, primary) in &lead {
                                if tool_card(ui, state, *command, *primary).clicked() {
                                    pending = Some(*command);
                                }
                                drew_any = true;
                            }
                            for ((group, commands), shown) in groups.iter().zip(&shown) {
                                if drew_any {
                                    group_divider(ui);
                                }
                                for command in commands[..*shown].iter().copied() {
                                    let resp = tool_card(ui, state, command, false);
                                    if command.id == "relations" {
                                        relations_anchor = resp.rect;
                                    }
                                    if resp.clicked() {
                                        pending = Some(command);
                                    }
                                }
                                let rest = &commands[*shown..];
                                if !rest.is_empty() {
                                    let more = more_card(ui, state, group, rest, &mut pending);
                                    if rest.iter().any(|c| c.id == "relations") {
                                        relations_anchor = more;
                                    }
                                }
                                drew_any = true;
                            }
                        });
                    });
            });
        },
    );

    ui.painter().hline(full_rect.x_range(), full_rect.bottom() - 0.5, Stroke::new(1.0, theme::line()));

    if let Some(command) = pending {
        state.run_command(command);
    }
    if relations_anchor != Rect::NOTHING {
        relations(ui, state, tab, relations_anchor);
    }
}

fn workspace_tab(
    ui: &mut egui::Ui,
    text: &str,
    accessible: &str,
    hint: &str,
    enabled: bool,
    selected: bool,
) -> egui::Response {
    let font = theme::font_semibold(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, Color32::PLACEHOLDER);
    let size = vec2((galley.size().x + 24.0).max(52.0), 24.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, accessible));

    let hovered = enabled && (resp.hovered() || resp.has_focus());
    let radius = CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 };
    if selected {
        ui.painter().rect_filled(rect, radius, theme::panel());
        ui.painter().hline(rect.x_range(), rect.bottom() - 1.0, Stroke::new(2.0, theme::accent()));
    } else if hovered {
        ui.painter().rect_filled(rect, radius, theme::hover());
    }

    let ink = if selected {
        theme::accent()
    } else if !enabled {
        theme::muted().gamma_multiply(0.45)
    } else if hovered {
        theme::ink()
    } else {
        theme::muted()
    };
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    resp.on_hover_text(hint)
}

/// The space between two cards of the strip.
const GAP: f32 = 4.0;
/// The width a group divider takes, with the gap after it.
const DIVIDER: f32 = 9.0 + GAP;
/// The width of a group's More card, with the gap before it.
const MORE: f32 = 40.0 + GAP;

/// How many tools of each group stay on the strip as cards when the strip is `room` wide.
/// `widths` holds each group's card widths in order. Every group keeps at least one card;
/// the group showing the most cards gives one up first, the later group on a tie.
fn fit(widths: &[Vec<f32>], lead: f32, has_lead: bool, room: f32) -> Vec<usize> {
    let mut shown: Vec<usize> = widths.iter().map(Vec::len).collect();
    let total = |shown: &[usize]| {
        let dividers = (widths.len() + usize::from(has_lead)).saturating_sub(1) as f32 * DIVIDER;
        let cards: f32 = widths
            .iter()
            .zip(shown)
            .map(|(w, n)| w[..*n].iter().map(|w| w + GAP).sum::<f32>() + if *n < w.len() { MORE } else { 0.0 })
            .sum();
        lead + dividers + cards
    };
    while total(&shown) > room {
        let Some(most) = (0..shown.len()).filter(|i| shown[*i] > 1).max_by_key(|i| (shown[*i], *i)) else {
            break;
        };
        shown[most] -= 1;
    }
    shown
}

/// The width `tool_card` gives a command's card.
fn card_width(ui: &egui::Ui, state: &AppState, command: &CommandSpec, primary: bool) -> f32 {
    let font = if primary { theme::font_semibold(11.0) } else { theme::font_medium(11.0) };
    let galley = ui.painter().layout_no_wrap(card_caption(command, state), font, Color32::PLACEHOLDER);
    (galley.size().x + 16.0).max(if primary { 76.0 } else { 54.0 })
}

/// The card at a group's end that opens a menu of the group's tools the strip has no room
/// for. Returns where the card is.
fn more_card(
    ui: &mut egui::Ui,
    state: &AppState,
    group: &str,
    rest: &[&'static CommandSpec],
    pending: &mut Option<&'static CommandSpec>,
) -> Rect {
    let (rect, response) = ui.allocate_exact_size(vec2(MORE - GAP, 48.0), Sense::click());
    let name = format!("More {group} tools");
    let active = rest.iter().any(|c| (c.active)(state));
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, name.clone()));
    let open = egui::Popup::is_id_open(ui.ctx(), response.id.with("popup"));
    let (fill, stroke) = if active {
        (theme::selection_bg(), Stroke::new(1.0, theme::accent()))
    } else if open || response.hovered() {
        (theme::hover(), Stroke::NONE)
    } else {
        (Color32::TRANSPARENT, Stroke::NONE)
    };
    let stroke = if response.has_focus() { Stroke::new(1.5, theme::accent()) } else { stroke };
    ui.painter().rect(rect, CornerRadius::same(theme::radii().tool), fill, stroke, StrokeKind::Inside);
    let ink = if active { theme::selection_ink() } else { theme::muted() };
    icons::paint(
        ui.painter(),
        icons::Icon::ChevronDown,
        Rect::from_center_size(pos2(rect.center().x, rect.top() + 17.0), vec2(14.0, 14.0)),
        ink,
    );
    let galley = ui.painter().layout_no_wrap("More".to_owned(), theme::font_medium(11.0), ink);
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - galley.size().y - 5.0),
        galley,
        ink,
    );
    let response = response.on_hover_text(format!("{name} ({})", rest.len()));
    egui::Popup::menu(&response)
        .align(egui::RectAlign::BOTTOM_START)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(240.0);
            ui.label(
                egui::RichText::new(group.to_uppercase()).family(theme::semibold()).size(10.0).color(theme::muted()),
            );
            for command in rest.iter().copied() {
                if menu_row(ui, state, command).clicked() {
                    *pending = Some(command);
                    ui.close();
                }
            }
        });
    rect
}

/// One tool in a group's More menu: its icon, name and shortcut.
fn menu_row(ui: &mut egui::Ui, state: &AppState, command: &'static CommandSpec) -> egui::Response {
    let enabled = command.is_enabled(state);
    let active = (command.active)(state);
    let accessible = label(command, state);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let response = ui.interact(
        rect,
        ui.id().with(("ribbon_tool", command.id)),
        if enabled { Sense::click_and_drag() } else { Sense::hover() },
    );
    response.dnd_set_drag_payload(crate::command_line::ToolDrag { id: command.id });
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, active, accessible.clone()));
    if active {
        ui.painter().rect_filled(rect, CornerRadius::same(4), theme::selection_bg());
    } else if enabled && (response.hovered() || response.has_focus()) {
        ui.painter().rect_filled(rect, CornerRadius::same(4), theme::hover());
    }
    let ink = if !enabled {
        theme::muted().gamma_multiply(0.55)
    } else if active {
        theme::selection_ink()
    } else {
        theme::ink()
    };
    let treatment = if enabled { icons::Treatment::Normal } else { icons::Treatment::Disabled };
    let icon = Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(16.0, 16.0));
    icons::paint_as(ui.painter(), command.icon, icon, ink, treatment);
    ui.painter().text(
        pos2(rect.left() + 30.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &accessible,
        theme::font(12.0),
        ink,
    );
    if let Some(shortcut) = state.shortcuts.shortcut_text(command.id) {
        ui.painter().text(
            pos2(rect.right() - 8.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            shortcut,
            theme::font(11.0),
            theme::muted(),
        );
    }
    response.on_hover_text(command.tip(state))
}

fn group_divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 36.0), Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range().shrink(2.0), Stroke::new(1.0, theme::line()));
}

fn tool_card(ui: &mut egui::Ui, state: &AppState, command: &'static CommandSpec, primary: bool) -> egui::Response {
    let enabled = command.is_enabled(state);
    let active = (command.active)(state);
    let caption = card_caption(command, state);
    let accessible = label(command, state);

    let font = if primary { theme::font_semibold(11.0) } else { theme::font_medium(11.0) };
    let galley = ui.painter().layout_no_wrap(caption, font, Color32::PLACEHOLDER);
    let width = card_width(ui, state, command, primary);
    let height = 48.0;

    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let response = ui.interact(
        rect,
        ui.id().with(("ribbon_tool", command.id)),
        if enabled { Sense::click_and_drag() } else { Sense::hover() },
    );
    response.dnd_set_drag_payload(crate::command_line::ToolDrag { id: command.id });
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, active, accessible.clone()));

    let (fill, stroke) = if primary && enabled {
        let bg = if response.hovered() { theme::accent_hover() } else { theme::accent() };
        (bg, Stroke::NONE)
    } else if active {
        (theme::selection_bg(), Stroke::new(1.0, theme::accent()))
    } else if enabled && response.hovered() {
        (theme::hover(), Stroke::NONE)
    } else {
        (Color32::TRANSPARENT, Stroke::NONE)
    };
    let focus_stroke = if response.has_focus() {
        Stroke::new(if primary { 2.0 } else { 1.5 }, if primary { theme::on_accent() } else { theme::accent() })
    } else {
        stroke
    };
    ui.painter().rect(rect, CornerRadius::same(theme::radii().tool), fill, focus_stroke, StrokeKind::Inside);

    let ink = if !enabled {
        theme::muted().gamma_multiply(0.55)
    } else if primary {
        theme::on_accent()
    } else if active {
        theme::selection_ink()
    } else {
        theme::ink()
    };
    let icon_size = theme::spacing().ribbon_icon_size;
    let icon_rect = Rect::from_center_size(pos2(rect.center().x, rect.top() + 17.0), vec2(icon_size, icon_size));
    let treatment = if enabled { icons::Treatment::Normal } else { icons::Treatment::Disabled };
    icons::paint_as(ui.painter(), command.icon, icon_rect, ink, treatment);
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - galley.size().y - 5.0),
        galley,
        ink,
    );

    response.on_hover_text(format!("{accessible}\n{}", command.tip(state)))
}

fn close_sketch_button(ui: &mut egui::Ui) -> egui::Response {
    let text = "Close sketch";
    let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::font_semibold(12.5), theme::on_danger());
    let size = vec2(galley.size().x + 28.0, 36.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    let fill = if resp.hovered() { theme::danger().gamma_multiply(0.9) } else { theme::danger() };
    ui.painter().rect_filled(rect, CornerRadius::same(7), fill);
    ui.painter().galley(
        pos2(rect.center().x - galley.size().x / 2.0, rect.center().y - galley.size().y / 2.0),
        galley,
        theme::on_danger(),
    );
    resp.on_hover_text("Close sketch (Enter)")
}

fn sketch_status_pill(ui: &mut egui::Ui, status: &crate::sketch::SolveStatus) {
    let (text, fg, bg) = match status {
        crate::sketch::SolveStatus::Empty => ("Empty", theme::muted(), theme::chip()),
        crate::sketch::SolveStatus::UnderDefined => ("Under defined", theme::accent(), theme::accent_soft()),
        crate::sketch::SolveStatus::FullyDefined => ("Fully defined", theme::ok(), theme::ok_soft()),
        crate::sketch::SolveStatus::OverDefined { .. } => ("Over defined", theme::error(), theme::error_soft()),
        crate::sketch::SolveStatus::Failed(_) => ("Solver failed", theme::error(), theme::error_soft()),
    };
    egui::Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(theme::radii().pill))
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).family(theme::semibold()).size(11.5).color(fg));
        });
}

/// Opens relations beneath the Relations card on the Sketch strip.
fn relations(ui: &egui::Ui, state: &mut AppState, tab: Tab, anchor: Rect) {
    if let Some(s) = state.sketch.as_mut() {
        if tab != Tab::Part(RibbonTab::Sketch) {
            s.relations_open = false;
            return;
        }
        let mut open = s.relations_open;
        let action = egui::Popup::new(ui.id().with("sketch_relations"), ui.ctx().clone(), anchor, ui.layer_id())
            .align(egui::RectAlign::BOTTOM_START)
            .kind(egui::PopupKind::Menu)
            .open_bool(&mut open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .width(220.0)
            .gap(4.0)
            .show(|ui| crate::sketcher::relations_menu(ui, s))
            .and_then(|shown| shown.inner);
        s.relations_open = open;
        if let Some(action) = action {
            state.push(action);
        }
    }
}
