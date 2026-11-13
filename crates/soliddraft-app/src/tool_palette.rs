//! Named command tiles for the Edit Suite's left palette.

use crate::commands::{self, CommandSpec, Tab};
use crate::icons;
use crate::state::{AppAction, AppState, EditorParams, RibbonTab};
use crate::theme::{self, edit_suite as tokens};
use egui::{CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

const COMPACT_WIDTH: f32 = 48.0;

pub fn panel(root: &mut egui::Ui, state: &mut AppState) {
    let compact = root.available_width() < theme::COMPACT_WIDTH;
    egui::Panel::left("tool_palette")
        .exact_size(if compact { COMPACT_WIDTH } else { tokens::PALETTE_WIDTH })
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::panel()).inner_margin(4))
        .show(root, |ui| show_mode(ui, state, compact));
}

pub fn plane_picker_panel(root: &mut egui::Ui, state: &mut AppState) {
    if !state.editor.as_ref().is_some_and(|editor| matches!(editor.params, EditorParams::PickPlane)) {
        return;
    }
    egui::Panel::top("sketch_plane_picker")
        .exact_size(38.0)
        .frame(egui::Frame::new().fill(theme::panel()).inner_margin(6))
        .show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label("Sketch plane");
                for plane in crate::document::PlaneKind::ALL {
                    let name = state.doc.model().datum_name(plane.to_model());
                    if ui.button(name).clicked() {
                        state.push(AppAction::StartSketchOn(crate::document::SketchPlane::Reference {
                            plane,
                            offset: 0.0,
                        }));
                    }
                }
                ui.label(egui::RichText::new("or pick a flat face").color(theme::muted()));
                if ui.button("Cancel").clicked() {
                    state.push(AppAction::CancelEditor);
                }
            });
        });
}

fn label(command: &CommandSpec, state: &AppState) -> String {
    match command.id {
        "sketch" if !state.in_sketch() => "Start sketch".to_owned(),
        "smart_dimension" => "Smart Dimension".to_owned(),
        _ => command.label_in(state),
    }
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    show_mode(ui, state, false);
}

fn show_mode(ui: &mut egui::Ui, state: &mut AppState, compact: bool) {
    let tab = state.workspace();
    let mut pending = None;
    let mut relations_anchor = ui.max_rect();
    egui::ScrollArea::vertical()
        .id_salt(("palette", state.drawing.is_active(), state.assembly.is_active(), tab.id()))
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(tokens::COLUMN_GAP, tokens::COLUMN_GAP);
            let start = matches!(tab, Tab::Part(RibbonTab::Solid | RibbonTab::Sketch)) && !state.in_sketch();
            if start {
                let command = commands::get("sketch").expect("the registry has Start sketch");
                let width = ui.available_width();
                let response = tile(ui, state, command, width, 30.0, compact);
                if response.clicked() {
                    pending = Some(command);
                }
            }
            if tab == Tab::Part(RibbonTab::Sketch) {
                let command = commands::get("smart_dimension").expect("the registry has Smart Dimension");
                let response = tile(ui, state, command, ui.available_width(), 30.0, compact);
                if response.clicked() {
                    pending = Some(command);
                }
            }
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
                            && !(state.assembly.is_active()
                                && tab == Tab::Assembly
                                && command.id == "assembly_interference")
                    })
                    .collect();
                if commands.is_empty() {
                    continue;
                }
                ui.add_space(6.0);
                if compact {
                    ui.separator();
                } else {
                    ui.label(
                        egui::RichText::new(commands::palette_group(group, tab))
                            .font(theme::font_medium(tokens::SMALL_SIZE))
                            .color(theme::muted()),
                    );
                }
                let width =
                    if compact { ui.available_width() } else { (ui.available_width() - tokens::COLUMN_GAP) / 2.0 };
                for row in commands.chunks(if compact { 1 } else { 2 }) {
                    let height = row
                        .iter()
                        .map(|command| {
                            if compact {
                                28.0
                            } else {
                                text(ui, state, command, width).size().y + 2.0 * tokens::TOOL_PADDING
                            }
                        })
                        .fold(24.0, f32::max);
                    ui.horizontal(|ui| {
                        for command in row {
                            let response = tile(ui, state, command, width, height, compact);
                            if command.id == "relations" {
                                relations_anchor = response.rect;
                            }
                            if response.clicked() {
                                pending = Some(*command);
                            }
                        }
                    });
                }
            }
        });
    if let Some(command) = pending {
        state.run_command(command);
    }
    crate::ribbon::relations(ui, state, tab, relations_anchor);
}

fn text(ui: &egui::Ui, state: &AppState, command: &CommandSpec, width: f32) -> std::sync::Arc<egui::Galley> {
    let wide = command.id == "sketch" && !state.in_sketch();
    let color = if !command.is_enabled(state) {
        theme::muted()
    } else if wide {
        theme::on_accent()
    } else if (command.active)(state) {
        theme::selection_ink()
    } else {
        theme::ink()
    };
    ui.painter().layout(
        label(command, state),
        if wide { theme::font_semibold(tokens::VALUE_SIZE) } else { theme::font(tokens::TOOL_LABEL_SIZE) },
        color,
        width - tokens::ICON_SIZE - 3.0 * tokens::TOOL_PADDING,
    )
}

fn tile(
    ui: &mut egui::Ui,
    state: &AppState,
    command: &'static CommandSpec,
    width: f32,
    height: f32,
    compact: bool,
) -> egui::Response {
    let enabled = command.is_enabled(state);
    let active = (command.active)(state);
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let response =
        ui.interact(rect, ui.id().with(command.id), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    response.dnd_set_drag_payload(crate::command_line::ToolDrag { id: command.id });
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, active, label(command, state)));
    let primary = !compact && command.id == "sketch" && !state.in_sketch();
    let wide = primary || (!compact && command.id == "smart_dimension");
    let fill = if primary && enabled {
        if response.hovered() { tokens::PRIMARY_HOVER } else { theme::accent() }
    } else if active {
        theme::selection_bg()
    } else if enabled && response.hovered() {
        theme::hover()
    } else if wide {
        theme::accent_soft()
    } else {
        theme::panel()
    };
    let focus = if response.has_focus() {
        Stroke::new(if primary { 2.0 } else { 1.0 }, if primary { theme::on_accent() } else { theme::accent() })
    } else {
        Stroke::NONE
    };
    ui.painter().rect(rect, CornerRadius::same(tokens::TOOL_RADIUS), fill, focus, StrokeKind::Inside);
    let galley = text(ui, state, command, if compact { tokens::PALETTE_WIDTH } else { width });
    let x = if compact {
        rect.center().x - tokens::ICON_SIZE / 2.0
    } else if wide {
        rect.center().x - (tokens::ICON_SIZE + tokens::TOOL_PADDING + galley.size().x) / 2.0
    } else {
        rect.left() + tokens::TOOL_PADDING
    };
    let icon = Rect::from_center_size(
        pos2(x + tokens::ICON_SIZE / 2.0, rect.center().y),
        vec2(tokens::ICON_SIZE, tokens::ICON_SIZE),
    );
    let treatment = if enabled { icons::Treatment::Normal } else { icons::Treatment::Disabled };
    let ink = if !enabled {
        theme::muted()
    } else if primary {
        theme::on_accent()
    } else if active {
        theme::selection_ink()
    } else {
        theme::ink()
    };
    icons::paint_as(ui.painter(), command.icon, icon, ink, treatment);
    let at = pos2(x + tokens::ICON_SIZE + tokens::TOOL_PADDING, rect.center().y - galley.size().y / 2.0);
    if !compact {
        ui.painter().galley(at, galley, theme::ink());
    }
    response.on_hover_text(format!("{}\n{}", label(command, state), command.tip(state)))
}
