//! Menus, status, dialogs and shortcuts.

pub(super) mod docks;

use crate::command_line::{Status, StatusNote};
use crate::commands;
use crate::panels::Side;
use crate::state::{AppState, EditorParams, NoteKind};
use crate::theme;
use egui::Key;

use super::workspace::SolidDraftApp;
use docks::pane;

impl SolidDraftApp {
    /// Runs the commands whose shortcuts were pressed, as the command registry binds
    /// them for the current context. A key whose command is disabled and has nothing to
    /// say is left for the widgets.
    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.preferences.as_ref().is_some_and(crate::preferences::Preferences::is_capturing_key) {
            return;
        }
        // The open palette takes the plain keys, as a focused text field does.
        let typing = ctx.egui_wants_keyboard_input() || self.state.palette.is_some();
        let state = &self.state;
        let context = commands::Context::of(state);
        // The view-key profile's rows run first. A row that took over a command's key
        // (Onshape's N) leaves that command off the key.
        let view_keys = (context.part() && !typing)
            .then(|| crate::navigation::view_keys::in_use(&state.navigation).with_shortcuts(&state.shortcuts));
        let superseded: Vec<(&str, Key)> = view_keys.iter().flat_map(|v| v.superseded()).collect();
        // Escape closes an open menu or ribbon row, not what is underneath it.
        let popup = egui::Popup::is_any_open(ctx);
        let bindings: Vec<_> = state
            .shortcuts
            .bindings(context)
            .into_iter()
            .filter(|(_, k)| !(popup && k.key == Key::Escape))
            .filter(|(_, k)| !(state.viewport.moving_dimension() && k.key == Key::Escape))
            .filter(|(c, k)| (!typing || k.fires_while_typing()) && (c.is_enabled(state) || (c.hint)(state).is_some()))
            .filter(|(c, k)| !superseded.contains(&(c.id, k.key)))
            .collect();
        // A key the navigation preset holds to navigate, as Inventor's F2, is left to the
        // viewport; its own command runs when it comes up without a drag having used it.
        let mac = ctx.os() == egui::os::OperatingSystem::Mac;
        let router = &mut self.state.viewport.router;
        router.configure(&self.state.navigation, mac);
        let hold = |k: Key| router.is_hold_key(crate::navigation::NavKey::main(k));
        let mut fired = Vec::new();
        let mut views = Vec::new();
        ctx.input_mut(|i| {
            if let Some(v) = &view_keys {
                i.events.retain(|e| {
                    let egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers } = e else {
                        return true;
                    };
                    let key = physical_key.unwrap_or(*key);
                    let nav = crate::navigation::NavKey { key, location: crate::navigation::key_location(key) };
                    match v.row_for(nav, *modifiers, context.sketch, mac) {
                        Some(row) => {
                            views.push(row.action);
                            false
                        }
                        None => true,
                    }
                });
            }
            let mut taps: Vec<(Key, egui::Modifiers)> = i
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, physical_key, pressed: false, modifiers, .. } => {
                        Some((physical_key.unwrap_or(*key), *modifiers))
                    }
                    _ => None,
                })
                .filter(|(k, _)| !typing && hold(*k) && !router.held_key_used(crate::navigation::NavKey::main(*k)))
                .collect();
            for (command, key) in &bindings {
                if !typing && hold(key.key) {
                    if let Some(t) = taps.iter().position(|(k, m)| *k == key.key && m.matches_logically(key.modifiers))
                    {
                        taps.remove(t);
                        fired.push(*command);
                    }
                } else if i.consume_key(key.modifiers, key.key) {
                    fired.push(*command);
                }
            }
        });
        for action in views {
            crate::navigation::view_keys::run(&mut self.state, action);
        }
        for command in fired {
            self.state.run_command(command);
        }
    }

    /// What the bottom bar says about the active document.
    pub(super) fn status(&self) -> Status {
        let st = &self.state;
        if st.start.is_some() {
            return Status {
                headline: ("No document open".to_owned(), theme::muted()),
                note: st.note.as_ref().map(StatusNote::of),
                details: Vec::new(),
                position: String::new(),
            };
        }
        let other = if st.assembly.is_active() {
            crate::assembly::status(st)
        } else if st.drawing.is_active() {
            crate::drawing::status(st)
        } else {
            None
        };
        other.unwrap_or_else(|| part_status(st))
    }

    /// The D1 Clear Bench single side column: resizable horizontally while any pane is
    /// toggled on, and completely absent (0px) when all panes are off.
    pub(super) fn side_panel(&mut self, root: &mut egui::Ui, side: Side, window_width: f32) {
        if !self.state.tree_shown() {
            return;
        }
        let frame = egui::Frame::new().fill(theme::panel()).inner_margin(egui::Margin::ZERO);
        let state = &mut self.state;
        egui::Panel::left(side.panel_id())
            .resizable(true)
            .default_size(state.layout.panel(side).width)
            .size_range(side.min_width()..=state.layout.max_width(side, window_width))
            .frame(frame)
            .show_separator_line(false)
            .show(root, |ui| pane(ui, state));
        let panel = self.state.layout.panel_mut(side);
        if let Some(s) = egui::PanelState::load(root.ctx(), side.panel_id()) {
            panel.width = s.outer_rect.width();
        }
    }

    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        crate::part_check::show(ctx, &mut self.state);
        crate::print_check::show(ctx, &mut self.state);
        crate::palette::show(ctx, &mut self.state);
        crate::top_bar::dialogs(ctx, &mut self.state);
        let st = &mut self.state;
        if st.show_shortcuts {
            let shortcuts = st.shortcuts.clone();
            let mut open = true;
            egui::Window::new("Keyboard shortcuts").collapsible(false).resizable(false).open(&mut open).show(
                ctx,
                |ui| {
                    egui::ScrollArea::vertical()
                        .min_scrolled_height(600.0)
                        .max_height(640.0)
                        .show(ui, |ui| shortcuts_grid(ui, &shortcuts, st));
                },
            );
            st.show_shortcuts = open;
        }
        if st.show_about {
            egui::Window::new("About SolidDraft").collapsible(false).resizable(false).open(&mut st.show_about).show(
                ctx,
                |ui| {
                    ui.label(egui::RichText::new("SolidDraft").family(theme::semibold()).size(16.0));
                    ui.label("A parametric CAD application written in Rust.");
                    ui.label(
                        egui::RichText::new(format!(
                            "Version {} · kernel: {}",
                            env!("CARGO_PKG_VERSION"),
                            st.doc.backend_name()
                        ))
                        .color(theme::muted()),
                    );
                },
            );
        }
        self.assistant_settings(ctx);
        crate::drawing::reference::window(ctx, &mut self.state);
        crate::locations::window(ctx, &mut self.state);
        crate::import::window(ctx, &mut self.state);
        crate::import::report::window(ctx, &mut self.state);
        crate::scan::window(ctx, &mut self.state);
        crate::recovery::window(ctx, &mut self.state);
        #[cfg(target_arch = "wasm32")]
        crate::journal_browser::window(ctx, &mut self.state);
    }

    /// The assistant settings dialog: the provider, an API key per provider and the models.
    pub(super) fn assistant_settings(&mut self, ctx: &egui::Context) {
        use soliddraft_assistant::key::KeySource;
        use soliddraft_assistant::{ProviderKind, choice};
        enum Act {
            StoreKey(ProviderKind),
            DeleteKey(ProviderKind),
            Save,
        }
        let st = &mut self.state;
        let busy = st.chat.busy();
        let mut open = st.provider_dialog.is_some();
        let mut acts = Vec::new();
        if let Some(dialog) = st.provider_dialog.as_mut() {
            egui::Window::new("Assistant settings").collapsible(false).resizable(false).open(&mut open).show(
                ctx,
                |ui| {
                    ui.label(egui::RichText::new("Provider").family(theme::semibold()));
                    ui.radio_value(&mut dialog.provider, None, "Automatic: the provider with a key, Anthropic if both");
                    for kind in ProviderKind::ALL {
                        ui.radio_value(&mut dialog.provider, Some(kind), kind.label());
                    }
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("API keys").family(theme::semibold()));
                    egui::Grid::new("assistant_keys").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                        for row in &mut dialog.keys {
                            let kind = row.kind;
                            ui.label(kind.label());
                            let text = match &row.found {
                                Some((KeySource::Environment(_), masked)) => {
                                    format!("{masked} from {}, which overrides the keychain", kind.key_var())
                                }
                                Some((KeySource::Keychain(_), masked)) => format!("{masked} in the OS keychain"),
                                None => "No key".into(),
                            };
                            ui.label(egui::RichText::new(text).color(theme::muted()));
                            if ui
                                .add_enabled(row.in_keychain, egui::Button::new("Delete"))
                                .on_hover_text("Remove the key from the OS keychain")
                                .clicked()
                            {
                                acts.push(Act::DeleteKey(kind));
                            }
                            ui.end_row();
                            ui.label("");
                            ui.add(
                                egui::TextEdit::singleline(&mut row.entry)
                                    .password(true)
                                    .hint_text(format!("Paste a new {kind} key"))
                                    .desired_width(260.0),
                            );
                            let label = if row.found.is_some() { "Replace" } else { "Save" };
                            if ui.add_enabled(!row.entry.trim().is_empty(), egui::Button::new(label)).clicked() {
                                acts.push(Act::StoreKey(kind));
                            }
                            ui.end_row();
                        }
                    });
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Models").family(theme::semibold()));
                    egui::Grid::new("assistant_models").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        for (kind, model) in &mut dialog.models {
                            ui.label(kind.label());
                            ui.add(
                                egui::TextEdit::singleline(model).hint_text(kind.default_model()).desired_width(260.0),
                            );
                            ui.end_row();
                        }
                    });
                    for var in [choice::PROVIDER_VAR, choice::MODEL_VAR] {
                        if soliddraft_assistant::environment::var(var).is_some_and(|v| !v.trim().is_empty()) {
                            ui.label(
                                egui::RichText::new(format!("{var} or its legacy alias overrides this setting."))
                                    .color(theme::muted()),
                            );
                        }
                    }
                    ui.add_space(8.0);
                    if ui
                        .add_enabled(!busy, egui::Button::new("Save and reconnect"))
                        .on_disabled_hover_text("Wait for the assistant to finish its answer")
                        .clicked()
                    {
                        acts.push(Act::Save);
                    }
                },
            );
        }
        if !open {
            st.provider_dialog = None;
        }
        for act in acts {
            match act {
                Act::StoreKey(kind) => st.store_key(kind),
                Act::DeleteKey(kind) => st.delete_key(kind),
                Act::Save => st.save_provider_settings(),
            }
        }
    }
}

/// The part's status: how the last rebuild went, what is being edited, how big the model
/// is and where the pointer is.
fn part_status(st: &AppState) -> Status {
    let errors = st.doc.error_count();
    let repairs = st.doc.repair_count();
    let headline = if let Some((message, kind)) = st.edit_status() {
        (message.to_owned(), if matches!(kind, NoteKind::Error) { theme::error() } else { theme::muted() })
    } else if errors > 0 {
        (format!("{errors} rebuild error{}", if errors == 1 { "" } else { "s" }), theme::error())
    } else if repairs > 0 {
        (format!("{repairs} feature{} to repair", if repairs == 1 { "" } else { "s" }), theme::warn())
    } else if st.doc.display_failure().is_some() {
        ("Display failed".to_owned(), theme::error())
    } else {
        ("Rebuild OK".to_owned(), theme::ok())
    };
    let mode = match (&st.sketch, &st.editor) {
        (Some(s), _) => {
            let name = st.doc.feature(s.feature).map(|f| f.name.clone()).unwrap_or("sketch".into());
            format!("Editing {name} · {}", s.sketch.status.label())
        }
        (None, Some(e)) => match e.params {
            EditorParams::Extrude(_) => "Editing Extrude".to_owned(),
            EditorParams::Cut(_) => "Editing Cut".to_owned(),
            _ => format!("Editing {}", e.title().trim_start_matches("Edit ")),
        },
        _ => "Editing part".to_owned(),
    };
    let n = st.doc.features.len();
    let unit = st.display_unit;
    let position = match (st.sketch.as_ref().and_then(|s| s.cursor), st.hover_point) {
        (Some(c), _) => format!("u {:.1}   v {:.1} {}", unit.from_mm(c.u), unit.from_mm(c.v), unit.short()),
        (None, Some(p)) => {
            format!(
                "x {:.1}   y {:.1}   z {:.1} {}",
                unit.from_mm(f64::from(p.x)),
                unit.from_mm(f64::from(p.y)),
                unit.from_mm(f64::from(p.z)),
                unit.short()
            )
        }
        _ => unit.short().to_owned(),
    };
    Status {
        headline,
        note: st
            .doc
            .display_failure()
            .map(|text| StatusNote {
                text: crate::geometry_message::for_user(text),
                color: theme::error(),
                fresh: true,
                report: false,
            })
            .or_else(|| st.note.as_ref().filter(|note| note.kind == NoteKind::Error).map(StatusNote::of))
            .or_else(|| {
                let body = st.doc.display_body()?;
                st.doc.kernel().body_notice(body).map(|text| StatusNote {
                    text: text.to_owned(),
                    color: theme::warn(),
                    fresh: true,
                    report: false,
                })
            })
            .or_else(|| st.note.as_ref().map(StatusNote::of)),
        details: vec![
            mode,
            format!("{n} feature{}", if n == 1 { "" } else { "s" }),
            format!("{} triangles", st.doc.triangle_count()),
            format!("{:.1} ms", st.frame_ms),
        ],
        position,
    }
}

/// Every shortcut in the registry, by where it works, then the mouse.
/// The shortcuts of the commands this build offers: the browser's list has no Quit and
/// no assistant.
fn shortcuts_grid(ui: &mut egui::Ui, shortcuts: &commands::Shortcuts, st: &AppState) {
    egui::Grid::new("shortcuts").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
        for scope in commands::Scope::ALL {
            let rows: Vec<(String, &str)> = commands::all()
                .iter()
                .filter(|c| c.in_app() && (c.shown)(st))
                .filter_map(|c| {
                    let keys: Vec<String> =
                        shortcuts.keys_of(c.id).into_iter().filter(|k| k.scope == scope).map(|k| k.text()).collect();
                    (!keys.is_empty()).then(|| (keys.join(" / "), c.doc))
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            ui.label(egui::RichText::new(scope.label()).family(theme::semibold()).color(theme::muted()));
            ui.end_row();
            for (keys, doc) in rows {
                ui.label(egui::RichText::new(keys).family(theme::semibold()));
                ui.label(doc);
                ui.end_row();
            }
        }
        ui.label(egui::RichText::new("Mouse").family(theme::semibold()).color(theme::muted()));
        ui.end_row();
        for (k, v) in [
            ("Left drag", "Orbit (outside a sketch)"),
            ("Right or middle drag", "Pan (also Shift + left drag)"),
            ("Wheel", "Zoom toward the cursor"),
            ("Click", "Select vertex, edge or face"),
        ] {
            ui.label(egui::RichText::new(k).family(theme::semibold()));
            ui.label(v);
            ui.end_row();
        }
    });
}
