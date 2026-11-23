//! The Create menu: every command that adds or removes material, with its shortcut,
//! in one drop-down. Each row carries a pin; a pinned command also shows as a tile in
//! the tool strip, so the strip holds the commands a person uses and the menu holds
//! all of them. The pins are kept in the settings file (see `settings::create_pins`).
//!
//! The menu is one self-contained widget: [`button`] draws the Create control and its
//! drop-down wherever the strip mounts it, and [`shows`] tells the strip which tiles
//! to draw.

use serde::{Deserialize, Serialize};

use crate::commands::{self, CommandSpec, Tab};
use crate::icons;
use crate::state::{AppState, RibbonTab};
use crate::theme;
use egui::{Align2, Rect, Sense, pos2, vec2};

/// The strip panels whose commands the menu lists.
const PANELS: [&str; 2] = ["Create", "Remove"];

/// The commands the menu lists, in strip order.
pub fn commands() -> Vec<&'static CommandSpec> {
    commands::ribbon(Tab::Part(RibbonTab::Solid))
        .into_iter()
        .filter(|(panel, _)| PANELS.contains(panel))
        .flat_map(|(_, items)| items.into_iter().map(|(command, _)| command))
        .collect()
}

/// The pinned commands, by id, in the order the menu lists them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pins {
    pub pinned: Vec<String>,
}

/// Every command starts pinned, so the strip is complete until a person trims it.
impl Default for Pins {
    fn default() -> Self {
        Pins { pinned: commands().iter().map(|c| c.id.to_owned()).collect() }
    }
}

impl Pins {
    pub fn is_pinned(&self, id: &str) -> bool {
        self.pinned.iter().any(|p| p == id)
    }

    /// Pins an unpinned command or unpins a pinned one.
    pub fn toggle(&mut self, id: &str) {
        if self.is_pinned(id) {
            self.pinned.retain(|p| p != id);
        } else {
            self.pinned.push(id.to_owned());
        }
    }

    /// Drops ids the menu does not list, such as a command a newer build added.
    pub fn retain_known(mut self) -> Self {
        let known = commands();
        self.pinned.retain(|id| known.iter().any(|c| c.id == id));
        self
    }
}

/// Whether the strip shows `command`: any command outside the menu, or a pinned one.
pub fn shows(state: &AppState, command: &CommandSpec) -> bool {
    !commands().iter().any(|c| c.id == command.id) || state.create_pins.is_pinned(command.id)
}

/// The Create control, `width` wide, and its drop-down.
pub fn button(ui: &mut egui::Ui, state: &mut AppState, width: f32) -> egui::Response {
    let id = ui.id().with("create_menu");
    let (rect, response) = ui.allocate_exact_size(vec2(width, 28.0), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let painter = ui.painter();
    let fill = if open {
        theme::selection_bg()
    } else if response.hovered() {
        theme::hover()
    } else {
        theme::accent_soft()
    };
    painter.rect_filled(rect, theme::RADIUS, fill);
    painter.text(
        pos2(rect.left() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Create",
        theme::font_semibold(12.0),
        theme::ink(),
    );
    icons::paint(
        painter,
        icons::Icon::ChevronDown,
        Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(12.0, 12.0)),
        theme::muted(),
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, open, "Create menu"));
    #[cfg(target_arch = "wasm32")]
    crate::web::show_control("Create menu", rect);
    let mut run = None;
    // A pin click keeps the menu open; a click outside it or on a command closes it.
    egui::Popup::menu(&response)
        .id(id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(260.0)
        .gap(4.0)
        .show(|ui| {
            let shown: Vec<_> = commands().into_iter().filter(|c| (c.shown)(state)).collect();
            for command in shown {
                if row(ui, state, command) {
                    run = Some(command);
                }
            }
            ui.separator();
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Pinned commands show in the tool palette. Use a row's pin to pin or unpin it.",
                    )
                    .font(theme::font(11.0))
                    .color(theme::muted()),
                )
                .wrap(),
            );
        });
    if let Some(command) = run {
        egui::Popup::close_id(ui.ctx(), id);
        state.run_command(command);
    }
    response
}

/// One menu row: the command's icon, name and shortcut, and its pin at the right. The
/// pin of an unpinned command shows while the row is hovered. Returns true when the
/// row was clicked to run the command.
fn row(ui: &mut egui::Ui, state: &mut AppState, command: &'static CommandSpec) -> bool {
    let enabled = command.is_enabled(state);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let label = command.label_in(state);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.clone()));
    let pin_rect = Rect::from_center_size(pos2(rect.right() - 14.0, rect.center().y), vec2(22.0, 22.0));
    let pin = ui.interact(pin_rect, ui.id().with(("pin", command.id)), Sense::click());
    let pinned = state.create_pins.is_pinned(command.id);
    let pin_label = format!("{} {label}", if pinned { "Unpin" } else { "Pin" });
    pin.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, pinned, pin_label.clone()));
    #[cfg(target_arch = "wasm32")]
    crate::web::show_control(pin_label.clone(), pin_rect);
    let hovered = response.hovered() || pin.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, theme::RADIUS, theme::hover());
    }
    let ink = if enabled { theme::ink() } else { theme::muted() };
    let treatment = if enabled { icons::Treatment::Normal } else { icons::Treatment::Disabled };
    icons::paint_as(
        painter,
        command.icon,
        Rect::from_min_size(rect.min + vec2(6.0, 5.0), vec2(18.0, 18.0)),
        ink,
        treatment,
    );
    painter.text(pos2(rect.left() + 32.0, rect.center().y), Align2::LEFT_CENTER, &label, theme::font(12.0), ink);
    if let Some(key) = state.shortcuts.shortcut_text(command.id) {
        painter.text(
            pos2(pin_rect.left() - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            key,
            theme::font(11.0),
            theme::muted(),
        );
    }
    if pinned || hovered {
        pin_glyph(painter, pin_rect, pinned, pin.hovered());
    }
    let pin = pin.on_hover_text(if pinned { "Unpin from the tool palette" } else { "Pin to the tool palette" });
    if pin.clicked() {
        state.create_pins.toggle(command.id);
        return false;
    }
    response.clicked() && enabled
}

/// A drawing pin: a head and a needle, solid when pinned.
fn pin_glyph(painter: &egui::Painter, rect: Rect, pinned: bool, hot: bool) {
    let color = if pinned || hot { theme::accent() } else { theme::muted() };
    let head = pos2(rect.center().x, rect.top() + 8.0);
    let stroke = egui::Stroke::new(1.5, color);
    if pinned {
        painter.circle_filled(head, 4.0, color);
    } else {
        painter.circle_stroke(head, 4.0, stroke);
    }
    painter.line_segment([pos2(head.x, head.y + 4.0), pos2(head.x, rect.bottom() - 3.0)], stroke);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_create_command_starts_pinned_and_toggles() {
        let all = commands();
        assert!(all.iter().any(|c| c.id == "extrude"), "Extrude is a create command");
        assert!(all.iter().any(|c| c.id == "cut"), "Cut is listed with the create commands");
        assert!(!all.iter().any(|c| c.id == "fillet"), "a modify command is not");
        let mut pins = Pins::default();
        assert!(all.iter().all(|c| pins.is_pinned(c.id)));
        pins.toggle("sweep");
        assert!(!pins.is_pinned("sweep"));
        pins.toggle("sweep");
        assert!(pins.is_pinned("sweep"));
        let read: Pins = serde_json::from_value(serde_json::json!({"pinned": ["loft", "gone"]})).unwrap();
        assert_eq!(read.retain_known().pinned, ["loft"], "an unknown command is dropped");
    }
}
