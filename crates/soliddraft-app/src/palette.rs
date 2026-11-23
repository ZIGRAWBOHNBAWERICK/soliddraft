//! The command palette: a text field over the command registry, opened with a shortcut.
//! Typing filters the commands the current workspace offers, Up and Down move the
//! choice, and Enter or a click runs it as a ribbon button does.
//!
//! The ranking is [`commands::search_here`], so the palette, the menu bar's search and
//! `soliddraft commands` order a query the same way. With nothing typed it lists the
//! pinned and recent commands.

use crate::commands::{self, CommandSpec, Context, History, Match};
use crate::shortcut_menu::Workspace;
use crate::state::AppState;
use crate::theme;
use egui::{Align2, Key, Modifiers, Pos2, vec2};

/// How many results the palette lists.
pub const SHOWN: usize = 10;

/// The commands a query finds in a context, best first, as many as the palette lists.
pub fn find(query: &str, history: &History, context: Context) -> Vec<Match> {
    let mut found = commands::search_here(query, history, context);
    found.truncate(SHOWN);
    found
}

/// An open palette.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Palette {
    /// What has been typed.
    pub query: String,
    /// The chosen result, counted from the top.
    pub selected: usize,
    pub(crate) shortcut: Option<(Workspace, Pos2)>,
}

impl Palette {
    pub(crate) fn for_shortcut(workspace: Workspace, position: Pos2) -> Self {
        Self { shortcut: Some((workspace, position)), ..Self::default() }
    }

    /// The commands the query finds in the context the app is in, best first.
    pub fn results(&self, state: &AppState) -> Vec<Match> {
        if let Some((workspace, _)) = self.shortcut {
            if self.query.trim().is_empty() {
                return state
                    .shortcut_menu
                    .commands(workspace)
                    .into_iter()
                    .map(|command| Match { command, pinned: true, recent: None, score: 0 })
                    .collect();
            }
            let mut found = commands::search_here(&self.query, &state.commands, Context::of(state));
            found.retain(|m| m.command.id != "shortcut_menu");
            found.truncate(SHOWN);
            return found;
        }
        find(&self.query, &state.commands, Context::of(state))
    }

    /// Moves the choice up or down, stopping at the ends.
    pub fn step(&mut self, down: bool, len: usize) {
        self.selected =
            if down { (self.selected + 1).min(len.saturating_sub(1)) } else { self.selected.saturating_sub(1) };
    }

    /// The chosen command.
    pub fn chosen(&self, state: &AppState) -> Option<&'static CommandSpec> {
        self.results(state).get(self.selected).map(|m| m.command)
    }
}

/// Opens the palette, or closes it when it is open.
pub fn toggle(state: &mut AppState) {
    state.palette = match state.palette {
        Some(_) => None,
        None => Some(Palette::default()),
    };
}

/// Draws the palette when it is open, and runs the command chosen with Enter or a click.
pub fn show(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut palette) = state.palette.take() else { return };
    let shortcut = palette.shortcut.map(|(workspace, _)| workspace);
    if shortcut.is_some_and(|workspace| state.start.is_some() || Workspace::of(state) != workspace) {
        return;
    }
    let (escape, down, up, enter) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::Escape),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::Enter),
        )
    });
    if escape {
        return;
    }
    let query = palette.query.clone();
    let len = palette.results(state).len();
    if down || up {
        palette.step(down, len);
    }
    let mut run = enter.then(|| palette.chosen(state)).flatten();
    let mut customize = false;
    let area = egui::Area::new(egui::Id::new(if shortcut.is_some() { "shortcut_menu" } else { "command_palette" }))
        .order(egui::Order::Foreground);
    let area = match palette.shortcut {
        Some((_, position)) => area.fixed_pos(position + vec2(8.0, 8.0)),
        None => area.anchor(Align2::CENTER_TOP, vec2(0.0, 80.0)),
    };
    let popup = area.show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.set_width(if shortcut.is_some() { 284.0 } else { 460.0 });
            let field = ui.add(
                egui::TextEdit::singleline(&mut palette.query)
                    .hint_text(if shortcut.is_some() { "Search commands" } else { "Type a command" })
                    .desired_width(f32::INFINITY),
            );
            field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Search commands"));
            #[cfg(target_arch = "wasm32")]
            if shortcut.is_some() {
                crate::web::show_control("Shortcut search", field.rect);
            }
            field.request_focus();
            if palette.query != query {
                palette.selected = 0;
            }
            let results = palette.results(state);
            if results.is_empty() && !palette.query.trim().is_empty() {
                ui.label(egui::RichText::new("No matching command").color(theme::muted()));
            }
            if shortcut.is_some() && results.is_empty() && palette.query.trim().is_empty() {
                ui.label("No pinned commands");
            }
            egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| {
                for (i, m) in results.iter().enumerate() {
                    let c = m.command;
                    let label = if c.is_working() { c.label_in(state) } else { format!("{} (coming soon)", c.label) };
                    let button = egui::Button::selectable(i == palette.selected, &label)
                        .shortcut_text(state.shortcuts.shortcut_text(c.id).unwrap_or_default());
                    let enabled = c.is_enabled(state);
                    let response = ui.add_enabled(enabled, button);
                    if shortcut.is_some() {
                        let name = format!("Run {label}");
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, i == palette.selected, &name)
                        });
                        #[cfg(target_arch = "wasm32")]
                        crate::web::show_control(name, response.rect);
                    }
                    if i == palette.selected && (down || up) {
                        response.scroll_to_me(None);
                    }
                    let response = if enabled {
                        response.on_hover_text(c.tip(state))
                    } else {
                        response.on_disabled_hover_text(c.tip(state))
                    };
                    if response.clicked() {
                        run = Some(c);
                    }
                }
            });
            if shortcut.is_some() {
                ui.separator();
                let response = ui.small_button("Customize shortcut menu");
                #[cfg(target_arch = "wasm32")]
                crate::web::show_control("Customize shortcut menu", response.rect);
                customize = response.clicked();
            }
        });
    });
    if let Some(workspace) = shortcut.filter(|_| customize) {
        crate::shortcut_menu::edit(state, workspace);
        return;
    }
    if popup.response.clicked_elsewhere() && shortcut.is_some() {
        return;
    }
    match run {
        Some(command) => state.run_command(command),
        None => state.palette = Some(palette),
    }
}
