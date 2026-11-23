//! The feature property panel: while a feature is created or edited, the side panel
//! turns into the feature's settings.
//!
//! OK, Cancel and the preview switch sit at the top, under the feature's name. Below
//! them the form's fields come in sections, one per field group, that fold open as they
//! are needed: the common choices open, the advanced ones folded. A section that a tick
//! box turns on (Direction 2, Thin feature) stays folded until it is ticked, and
//! ticking it opens it. A folded section the user opens stays open for that kind of
//! feature until the app closes. Enter applies the feature and Escape cancels it, as
//! the `confirm` and `cancel` commands do everywhere in a part.
//!
//! The panel is one self-contained widget: [`show`] draws it into whatever side area
//! mounts it, and [`active`] says when it has a feature to show.

use std::collections::HashMap;

use crate::feature_forms::{self, Key};
use crate::form_bar::Field;
use crate::icons::{self, Icon};
use crate::state::{AppAction, AppState};
use crate::theme;
use egui::{Align2, Rect, Sense, pos2, vec2};

/// The sections the user folded or unfolded, by form kind and section title.
#[derive(Debug, Default)]
pub struct State {
    open: HashMap<(&'static str, &'static str), bool>,
}

impl State {
    /// Whether a section is open: the user's choice, or else `default`.
    pub fn is_open(&self, kind: &'static str, section: &'static str, default: bool) -> bool {
        self.open.get(&(kind, section)).copied().unwrap_or(default)
    }

    pub fn set_open(&mut self, kind: &'static str, section: &'static str, open: bool) {
        self.open.insert((kind, section), open);
    }
}

/// True while a part feature is being created or edited outside a sketch.
pub fn active(state: &AppState) -> bool {
    state.editor.is_some() && !state.in_sketch() && !state.assembly.is_active() && !state.drawing.is_active()
}

/// One section of the form: its title, its fields in order, whether it opens by
/// default and, for a section a tick box turns on, whether it is on.
struct Section<'a> {
    title: &'static str,
    fields: Vec<&'a Field<Key>>,
    opens: bool,
    toggle: Option<bool>,
}

/// The form's sections in the order their fields come. A section opens by default when
/// one of its fields is a common one; a tick-box section opens when it is on.
fn sections<'a>(fields: &'a [Field<Key>], params: &crate::state::EditorParams) -> Vec<Section<'a>> {
    let mut out: Vec<Section<'a>> = Vec::new();
    for field in fields {
        let index = match out.iter().position(|s| s.title == field.group) {
            Some(i) => i,
            None => {
                let toggle = feature_forms::section_toggle(params, field.group);
                out.push(Section { title: field.group, fields: Vec::new(), opens: false, toggle });
                out.len() - 1
            }
        };
        let section = &mut out[index];
        section.opens |= field.primary;
        if field.id != Key::SectionOff {
            section.fields.push(field);
        }
    }
    for section in &mut out {
        if let Some(on) = section.toggle {
            section.opens = on;
        }
    }
    out
}

/// Draws the panel for the open feature form.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(context) = feature_forms::context(state) else { return };
    let Some(editor) = state.editor.as_ref() else { return };
    let spec = feature_forms::spec(&editor.params, context.planes(), context.free_sketches());
    let title = editor.title();
    let message = feature_forms::message(editor);
    let mut actions = Vec::new();
    let mut open_all = false;
    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
    egui::Frame::new().inner_margin(egui::Margin { left: 10, right: 10, top: 8, bottom: 6 }).show(ui, |ui| {
        header(ui, state, &title, spec.icon, spec.confirm, &mut actions);
        if let Some(message) = &message {
            egui::Frame::new()
                .fill(theme::panel())
                .stroke(egui::Stroke::new(1.0, theme::error()))
                .corner_radius(theme::RADIUS)
                .inner_margin(8)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.colored_label(theme::error(), message);
                });
        }
        ui.add_space(2.0);
        egui::ScrollArea::vertical().id_salt("feature_panel_scroll").auto_shrink([false, false]).show(ui, |ui| {
            let Some(editor) = state.editor.as_mut() else { return };
            let sections = sections(&spec.fields, &editor.params);
            #[cfg(target_arch = "wasm32")]
            let mut drawn = Vec::new();
            for section in sections {
                let mut open = state.feature_panel.is_open(spec.kind, section.title, section.opens);
                let mut on = section.toggle;
                let changed = section_header(ui, section.title, &mut open, on.as_mut());
                if let Some(on) = on.filter(|_| changed) {
                    feature_forms::set_section(&mut editor.params, section.title, on);
                    open = on;
                }
                state.feature_panel.set_open(spec.kind, section.title, open);
                #[cfg(target_arch = "wasm32")]
                drawn.push(serde_json::json!({ "title": section.title, "open": open, "on": on }));
                if !open || section.toggle == Some(false) {
                    continue;
                }
                egui::Frame::new().inner_margin(egui::Margin { left: 6, right: 0, top: 2, bottom: 8 }).show(ui, |ui| {
                    for field in &section.fields {
                        ui.push_id((spec.kind, field.id), |ui| {
                            open_all |= row(ui, field, |ui| {
                                feature_forms::render(ui, field.id, editor, &context, &mut actions)
                            });
                        });
                    }
                });
            }
            #[cfg(target_arch = "wasm32")]
            crate::web::show_feature_panel(serde_json::json!({ "title": title, "sections": drawn }));
        });
    });
    if open_all {
        // A field that asks for its details opens the folded sections, where they are.
        let Some(editor) = state.editor.as_ref() else { return };
        for section in sections(&spec.fields, &editor.params) {
            state.feature_panel.set_open(spec.kind, section.title, true);
        }
    }
    for action in actions {
        state.push(action);
    }
}

/// The feature's name with OK, Cancel and the preview switch.
fn header(
    ui: &mut egui::Ui,
    state: &mut AppState,
    title: &str,
    icon: Icon,
    can_confirm: bool,
    actions: &mut Vec<AppAction>,
) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    icons::paint(ui.painter(), icon, Rect::from_min_size(rect.min + vec2(0.0, 4.0), vec2(18.0, 18.0)), theme::accent());
    ui.painter().text(
        pos2(rect.left() + 26.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        theme::font_semibold(13.0),
        theme::ink(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, format!("Feature panel: {title}")));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if can_confirm {
            let ok = action_button(ui, Icon::Check, "OK", true).on_hover_text("OK (Enter)");
            #[cfg(target_arch = "wasm32")]
            crate::web::show_control("Form OK", ok.rect);
            if ok.clicked() {
                actions.push(AppAction::CommitEditor);
            }
        }
        if action_button(ui, Icon::Close, "Cancel", false).on_hover_text("Cancel (Esc)").clicked() {
            actions.push(AppAction::CancelEditor);
        }
        if let Some(editor) = state.editor.as_mut() {
            let shown = editor.show_preview;
            let preview = ui.add(egui::Button::selectable(shown, "Preview")).on_hover_text(if shown {
                "Hide the preview"
            } else {
                "Show the preview"
            });
            #[cfg(target_arch = "wasm32")]
            crate::web::show_control("Form preview", preview.rect);
            if preview.clicked() {
                editor.show_preview = !shown;
            }
        }
    });
    ui.separator();
}

/// A button with an icon and a word, filled with the accent when `primary`.
fn action_button(ui: &mut egui::Ui, icon: Icon, text: &str, primary: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(78.0, 28.0), Sense::click());
    let (fill, ink) = if primary {
        (if response.hovered() { theme::accent().linear_multiply(0.85) } else { theme::accent() }, theme::on_accent())
    } else {
        (if response.hovered() { theme::hover() } else { theme::panel() }, theme::ink())
    };
    let painter = ui.painter();
    painter.rect(rect, theme::RADIUS, fill, egui::Stroke::new(1.0, theme::line()), egui::StrokeKind::Inside);
    icons::paint(painter, icon, Rect::from_min_size(rect.min + vec2(10.0, 7.0), vec2(14.0, 14.0)), ink);
    painter.text(pos2(rect.left() + 30.0, rect.center().y), Align2::LEFT_CENTER, text, theme::font_medium(12.0), ink);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    response
}

/// A section's header: a chevron that folds and unfolds it, then its title, or for a
/// tick-box section the box with the title. Returns true when the box changed.
fn section_header(ui: &mut egui::Ui, title: &'static str, open: &mut bool, toggle: Option<&mut bool>) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, theme::RADIUS, theme::hover());
    }
    painter.hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, theme::line()));
    let chevron = if *open { Icon::ChevronDown } else { Icon::ChevronRight };
    icons::paint(
        painter,
        chevron,
        Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(12.0, 12.0)),
        theme::muted(),
    );
    let label = format!("{} {title}", if *open { "Fold" } else { "Unfold" });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label.clone()));
    #[cfg(target_arch = "wasm32")]
    crate::web::show_control(format!("Section {title}"), rect);
    let mut changed = false;
    match toggle {
        Some(on) => {
            let check = Rect::from_min_max(
                pos2(rect.left() + 2.0, rect.top() + 2.0),
                pos2(rect.right() - 28.0, rect.bottom() - 2.0),
            );
            let checkbox =
                ui.put(check, egui::Checkbox::new(on, egui::RichText::new(title).font(theme::font_semibold(12.0))));
            #[cfg(target_arch = "wasm32")]
            crate::web::show_control(title, checkbox.rect);
            changed = checkbox.changed();
        }
        None => {
            ui.painter().text(
                pos2(rect.left() + 4.0, rect.center().y),
                Align2::LEFT_CENTER,
                title,
                theme::font_semibold(12.0),
                theme::ink(),
            );
        }
    }
    if response.clicked() {
        *open = !*open;
    }
    changed
}

/// One field: its caption in a fixed column, then its control; a field without a
/// caption, such as a tick box or a list, takes the whole row.
fn row(ui: &mut egui::Ui, field: &Field<Key>, add: impl FnOnce(&mut egui::Ui) -> bool) -> bool {
    if field.label.is_empty() {
        return ui.vertical(|ui| add(ui)).inner;
    }
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(96.0, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(egui::RichText::new(&field.label).color(theme::muted())).truncate())
                .on_hover_text(&field.label);
        });
        ui.vertical(|ui| add(ui)).inner
    })
    .inner
}
