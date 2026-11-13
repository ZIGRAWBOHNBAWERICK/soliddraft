//! Context for the active tool and selection in the Edit Suite frame.

use crate::commands;
use crate::dimension_edit::FeatureField;
use crate::document::{FeatureId, FeatureStatus, Selection, fmt_mm};
use crate::icons::{self, Icon};
use crate::state::{AppAction, AppState};
use crate::theme;
use egui::{Sense, vec2};

pub const HEIGHT: f32 = 46.0;

pub fn panel(root: &mut egui::Ui, state: &mut AppState) {
    egui::Panel::top("options_bar")
        .exact_size(HEIGHT)
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::chrome()).inner_margin(egui::Margin::symmetric(8, 6)))
        .show(root, |ui| show(ui, state));
}

fn active_tool(state: &AppState) -> (Icon, String, String) {
    if let Some(sketch) = &state.sketch {
        let command = commands::all()
            .iter()
            .filter(|c| (c.active)(state))
            .find(|c| matches!(c.action_now(state), Some(AppAction::SketchTool(tool)) if tool == sketch.tool));
        if let Some(command) = command {
            let label =
                if command.id == "smart_dimension" { "Smart Dimension".into() } else { command.label_in(state) };
            return (command.icon, label, command.doc.into());
        }
        return (Icon::Sketch, sketch.tool.label().into(), "Pick entities in the sketch".into());
    }
    if let Some(editor) = &state.editor {
        let icon =
            commands::all().iter().find(|c| (c.active)(state) && c.outside_sketch).map_or(Icon::Config, |c| c.icon);
        return (icon, editor.title(), "".into());
    }
    if state.measure.open {
        return (Icon::Measure, "Measure".into(), "Pick edges or faces to measure".into());
    }
    (Icon::Glyph("cursor"), "Select".into(), "".into())
}

fn command(ui: &mut egui::Ui, state: &mut AppState, id: &str) {
    let command = commands::get(id).expect("options bar commands are registered");
    let enabled = command.is_enabled(state);
    let selected = (command.active)(state);
    let response = ui
        .add_enabled(enabled, egui::Button::new(command.label_in(state)).min_size(vec2(0.0, 28.0)).selected(selected));
    if response.clicked() {
        state.run_command(command);
    }
    response.on_hover_text(command.tip(state));
}

fn show(ui: &mut egui::Ui, state: &mut AppState) {
    if state.editor.is_some() && !state.in_sketch() {
        crate::feature_forms::show(ui, state);
        return;
    }
    state.form_bar.reset();
    let context = (state.in_sketch(), state.assembly.is_active(), state.drawing.is_active());
    ui.data_mut(|data| {
        let id = egui::Id::new("options_details_context");
        if data.get_temp::<(bool, bool, bool)>(id) != Some(context) {
            state.options_details = false;
            data.insert_temp(id, context);
        }
    });
    let mut sheet_anchor = ui.available_rect_before_wrap();
    if state.in_sketch() {
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), 30.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if ui
                    .add_sized(
                        vec2(116.0, 30.0),
                        egui::Button::new(egui::RichText::new("Close sketch").color(theme::edit_suite::ON_DANGER))
                            .fill(theme::edit_suite::DANGER),
                    )
                    .on_hover_text("Close sketch (Enter or Esc)")
                    .clicked()
                {
                    state.push(AppAction::ExitSketch);
                }
                if let Some(sketch) = &state.sketch {
                    let (text, color) = match &sketch.sketch.status {
                        crate::sketch::SolveStatus::Empty => ("Empty", theme::muted()),
                        crate::sketch::SolveStatus::UnderDefined => {
                            ("Under defined", theme::edit_suite::UNDER_DEFINED_LABEL)
                        }
                        crate::sketch::SolveStatus::FullyDefined => ("Fully defined", theme::ink()),
                        crate::sketch::SolveStatus::OverDefined { .. } => ("Over defined", theme::error()),
                        crate::sketch::SolveStatus::Failed(_) => ("Solver failed", theme::error()),
                    };
                    ui.colored_label(color, text);
                }
                let more = ui.button("Sketch options");
                sheet_anchor = more.rect;
                if more.clicked() {
                    state.options_details = !state.options_details;
                }
                let (icon, label, _) = active_tool(state);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let (rect, response) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    icons::paint(ui.painter(), icon, rect, theme::accent());
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("Active tool: {label}"))
                    });
                    ui.add(egui::Label::new(label).truncate());
                });
            },
        );
        details_sheet(ui, state, sheet_anchor);
        crate::sketch_values::popup(ui, state);
        return;
    }
    if state.assembly.is_active() || state.drawing.is_active() || state.drawing.new.is_some() {
        let response = ui.button(if state.assembly.is_active() { "Assembly options" } else { "Drawing options" });
        if response.clicked() {
            state.options_details = !state.options_details;
        }
        details_sheet(ui, state, response.rect);
        return;
    }
    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (icon, label, prompt) = active_tool(state);
        let width = (ui.available_width() * 0.25).clamp(120.0, 234.0);
        let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
        ui.painter().rect_filled(rect, theme::RADIUS, theme::panel());
        icons::paint(
            ui.painter(),
            icon,
            egui::Rect::from_min_size(rect.min + vec2(8.0, 6.0), vec2(18.0, 18.0)),
            theme::accent(),
        );
        let mut job =
            egui::text::LayoutJob::simple(label.clone(), theme::font_medium(12.0), theme::ink(), width - 42.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let text = ui.painter().layout_job(job);
        ui.painter().galley(rect.min + vec2(34.0, (rect.height() - text.size().y) / 2.0), text, theme::ink());
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("Active tool: {label}")));
        response.on_hover_text(&label);

        if state.doc.selection.is_some() || state.measure.open {
            let response = ui.button("Details");
            sheet_anchor = response.rect;
            if response.clicked() {
                state.options_details = !state.options_details;
            }
        }
        egui::ScrollArea::horizontal()
            .id_salt(("options_contents", label, format!("{:?}", state.doc.selection)))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if state.in_sketch() || state.editor.is_some() || state.measure.open {
                        ui.label(egui::RichText::new(prompt).color(theme::muted()));
                    } else if let Some(selection) = state.doc.selection {
                        selection_options(ui, state, selection);
                    } else {
                        ui.label(egui::RichText::new("Pick").color(theme::muted()));
                        ui.checkbox(&mut state.pick_filter.faces, "Faces").on_hover_text("Pick faces");
                        ui.checkbox(&mut state.pick_filter.edges, "Edges").on_hover_text("Pick edges and vertices");
                        ui.checkbox(&mut state.pick_filter.bodies, "Bodies")
                            .on_hover_text("Pick the whole body when face or edge picking is off");
                        ui.separator();
                        ui.label(egui::RichText::new("View").color(theme::muted()));
                        for id in ["view_isometric", "zoom_fit", "view_front", "view_top", "view_right", "normal_to"] {
                            command(ui, state, id);
                        }
                    }
                });
            });
    });
    details_sheet(ui, state, sheet_anchor);
}

fn selection_options(ui: &mut egui::Ui, state: &mut AppState, selection: Selection) {
    let Some(info) = state.doc.describe(selection) else {
        ui.label("Selection unavailable");
        return;
    };
    if matches!(selection, Selection::Edge(_) | Selection::Face(_) | Selection::Feature(_) | Selection::Body) {
        for id in ["fillet", "chamfer"] {
            command(ui, state, id);
        }
        ui.separator();
    }
    ui.label(egui::RichText::new(info.title).color(theme::muted()));
    if matches!(selection, Selection::Edge(_) | Selection::Face(_)) {
        for (label, value) in info.details.iter().filter(|(label, _)| matches!(label.as_str(), "Length" | "Area")) {
            value_label(ui, label, value);
        }
    }
    let features: Vec<_> = if matches!(selection, Selection::Body | Selection::Face(_) | Selection::Edge(_)) {
        let end = state.doc.rollback.unwrap_or(state.doc.features.len()).min(state.doc.features.len());
        state.doc.features[..end]
            .iter()
            .filter(|f| !f.suppressed && f.status != FeatureStatus::Inactive)
            .map(|f| f.id)
            .collect()
    } else {
        info.feature.into_iter().collect()
    };
    for id in features {
        feature_dimensions(
            ui,
            state,
            id,
            matches!(selection, Selection::Body | Selection::Face(_) | Selection::Edge(_)),
        );
    }
}

fn value_label(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::new()
        .fill(theme::panel())
        .inner_margin(egui::Margin::symmetric(8, 4))
        .corner_radius(theme::RADIUS)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(label).color(theme::muted()));
                ui.label(egui::RichText::new(value).font(theme::font_medium(12.0)));
            });
        });
}

fn feature_dimensions(ui: &mut egui::Ui, state: &AppState, id: FeatureId, named: bool) {
    let Some(feature) = state.doc.feature(id) else { return };
    let Some(info) = state.doc.describe(Selection::Feature(id)) else { return };
    let mut values = Vec::new();
    for field in [FeatureField::Width, FeatureField::Height, FeatureField::Diameter] {
        if let Some(value) = field.get(&feature.kind) {
            values.push((field.label().to_owned(), format!("{} mm", fmt_mm(value))));
        }
    }
    values.extend(info.details.into_iter().filter(|(label, _)| {
        matches!(
            label.as_str(),
            "Depth"
                | "Angle"
                | "End"
                | "Radius"
                | "Distance"
                | "Diameter"
                | "Wall thickness"
                | "Offset"
                | "Layout"
                | "Head"
        )
    }));
    if let Some(sketch) = feature.sketch() {
        for constraint in &sketch.sketch.constraints {
            if let Some(value) = constraint.display_value(&sketch.sketch) {
                let unit = if constraint.name() == "Angle" { "°" } else { " mm" };
                values.push((constraint.name().into(), format!("{}{unit}", fmt_mm(value))));
            }
        }
    }
    for (label, value) in values {
        let label = if named { format!("{} · {label}", feature.name) } else { label };
        value_label(ui, &label, &value);
    }
}

fn details_sheet(ui: &mut egui::Ui, state: &mut AppState, anchor: egui::Rect) {
    if !state.options_details {
        return;
    }
    let screen = ui.ctx().input(|i| i.content_rect());
    let width = 380.0_f32.min(screen.width() - 16.0);
    let left = anchor.left().clamp(screen.left() + 8.0, (screen.right() - width - 8.0).max(screen.left() + 8.0));
    egui::Area::new(egui::Id::new("options_details"))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(left, anchor.bottom() + 8.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(theme::panel())
                .stroke(egui::Stroke::new(1.0, theme::line()))
                .corner_radius(theme::RADIUS)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_width(width - 16.0);
                    ui.set_height((screen.bottom() - anchor.bottom() - 32.0).clamp(120.0, 650.0));
                    if ui.button("Close options").clicked() {
                        state.options_details = false;
                    }
                    if state.assembly.is_active() {
                        crate::assembly::properties(ui, state);
                    } else if state.drawing.new.is_some() && !state.drawing.is_active() {
                        crate::drawing::properties_drawing::new_drawing(ui, state);
                    } else if state.drawing.is_active() {
                        crate::drawing::properties_drawing::show(ui, state);
                    } else {
                        crate::properties::show(ui, state);
                    }
                });
        });
}
