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

fn command(ui: &mut egui::Ui, state: &mut AppState, id: &str, compact: bool) {
    let command = commands::get(id).expect("options bar commands are registered");
    let enabled = command.is_enabled(state);
    let selected = (command.active)(state);
    let response = if compact {
        let response = ui.add_enabled(enabled, egui::Button::new("").min_size(vec2(28.0, 28.0)).selected(selected));
        let ink = if enabled { theme::ink() } else { theme::muted() };
        let rect = egui::Rect::from_center_size(response.rect.center(), vec2(18.0, 18.0));
        icons::paint(ui.painter(), command.icon, rect, ink);
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, command.label_in(state))
        });
        response
    } else {
        ui.add_enabled(enabled, egui::Button::new(command.label_in(state)).min_size(vec2(0.0, 28.0)).selected(selected))
    };
    if response.clicked() {
        state.run_command(command);
    }
    response.on_hover_text(command.tip(state));
}

fn show(ui: &mut egui::Ui, state: &mut AppState) {
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
                        ui.label(egui::RichText::new("Display").color(theme::muted()));
                        for id in ["display_shaded_edges", "display_shaded", "display_wireframe"] {
                            command(ui, state, id, true);
                        }
                        ui.separator();
                        ui.label(egui::RichText::new("View").color(theme::muted()));
                        for id in ["view_isometric", "zoom_fit", "view_front", "view_top", "view_right", "normal_to"] {
                            command(ui, state, id, false);
                        }
                    }
                });
            });
    });
}

fn selection_options(ui: &mut egui::Ui, state: &mut AppState, selection: Selection) {
    let Some(info) = state.doc.describe(selection) else {
        ui.label("Selection unavailable");
        return;
    };
    if matches!(selection, Selection::Edge(_) | Selection::Face(_) | Selection::Feature(_) | Selection::Body) {
        for id in ["fillet", "chamfer"] {
            command(ui, state, id, false);
        }
        ui.separator();
    }
    ui.label(egui::RichText::new(info.title).color(theme::muted()));
    if matches!(selection, Selection::Edge(_) | Selection::Face(_)) {
        for (label, value) in info.details.iter().filter(|(label, _)| matches!(label.as_str(), "Length" | "Area")) {
            value_label(ui, label, value);
        }
    }
    let features: Vec<_> = if selection == Selection::Body {
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
        feature_dimensions(ui, state, id, selection == Selection::Body);
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
