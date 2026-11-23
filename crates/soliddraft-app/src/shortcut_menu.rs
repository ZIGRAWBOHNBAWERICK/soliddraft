//! The commands pinned to S in each workspace.

use crate::commands::{self, CommandSpec, Context, History};
use crate::state::{AppState, NoteKind};
use egui::pos2;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Workspace {
    Part,
    Assembly,
    Sketch,
    Drawing,
}

impl Workspace {
    const ALL: [Self; 4] = [Self::Part, Self::Assembly, Self::Sketch, Self::Drawing];

    pub(crate) fn of(state: &AppState) -> Self {
        if state.assembly.is_active() {
            Self::Assembly
        } else if state.drawing.is_active() {
            Self::Drawing
        } else if state.in_sketch() {
            Self::Sketch
        } else {
            Self::Part
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Part => "Part",
            Self::Assembly => "Assembly",
            Self::Sketch => "Sketch",
            Self::Drawing => "Drawing",
        }
    }

    fn context(self) -> Context {
        Context {
            assembly: self == Self::Assembly,
            sketch: self == Self::Sketch,
            drawing: self == Self::Drawing,
            ..Context::default()
        }
    }

    fn defaults(self) -> &'static [&'static str] {
        match self {
            Self::Part => &["sketch", "extrude", "cut", "fillet", "measure", "zoom_fit"],
            Self::Assembly => &["insert_component", "assembly_mate", "assembly_update", "zoom_fit"],
            Self::Sketch => &["sketch_line", "sketch_rectangle", "sketch_circle", "sketch_slot", "zoom_fit"],
            Self::Drawing => {
                &["drawing_model_view", "drawing_projected", "drawing_section", "drawing_note", "zoom_fit"]
            }
        }
    }

    fn offers(self, command: &CommandSpec) -> bool {
        command.id != "shortcut_menu" && command.available(self.context())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Preferences {
    pinned: BTreeMap<Workspace, Vec<String>>,
    hint_seen: bool,
}

impl Preferences {
    pub(crate) fn commands(&self, workspace: Workspace) -> Vec<&'static CommandSpec> {
        let ids: Vec<&str> = match self.pinned.get(&workspace) {
            Some(ids) => ids.iter().map(String::as_str).collect(),
            None => workspace.defaults().to_vec(),
        };
        let mut seen = std::collections::HashSet::new();
        ids.into_iter()
            .filter(|id| seen.insert(*id))
            .filter_map(commands::get)
            .filter(|c| workspace.offers(c))
            .collect()
    }

    fn replace(&mut self, workspace: Workspace, commands: &[&CommandSpec]) {
        self.pinned.insert(workspace, commands.iter().map(|c| c.id.to_owned()).collect());
    }
}

pub(crate) struct Customize {
    workspace: Workspace,
    draft: Preferences,
    query: String,
}

pub(crate) fn toggle(ctx: &egui::Context, state: &mut AppState) {
    if state.start.is_some() || state.shortcut_menu_editor.is_some() {
        return;
    }
    let origin = ctx.pointer_hover_pos().unwrap_or_else(|| ctx.content_rect().center());
    let workspace = Workspace::of(state);
    state.palette = if state.palette.as_ref().is_some_and(|p| p.shortcut.is_some()) {
        None
    } else {
        Some(crate::palette::Palette::for_shortcut(workspace, origin))
    };
}

pub(crate) fn teach(state: &mut AppState) {
    if state.start.is_none() && !state.shortcut_menu.hint_seen {
        state.shortcut_menu.hint_seen = true;
        let _ = crate::settings::save_shortcut_menu(&state.shortcut_menu);
        state.note("Press S to open your shortcut menu.", NoteKind::Info);
    }
}

pub(crate) fn edit(state: &mut AppState, workspace: Workspace) {
    state.shortcut_menu_editor =
        Some(Customize { workspace, draft: state.shortcut_menu.clone(), query: String::new() });
}

pub(crate) fn customize(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut editor) = state.shortcut_menu_editor.take() else { return };
    let mut open = true;
    let mut save = false;
    let mut export = false;
    egui::Window::new("Customize shortcut menu")
        .default_pos(pos2(ctx.content_rect().center().x - 220.0, ctx.content_rect().top() + 130.0))
        .default_width(430.0)
        .collapsible(false)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for workspace in Workspace::ALL {
                    if ui.selectable_value(&mut editor.workspace, workspace, workspace.label()).clicked() {
                        editor.query.clear();
                    }
                }
            });
            ui.separator();
            let workspace = editor.workspace;
            let mut pinned = editor.draft.commands(workspace);
            let mut change = None;
            egui::ScrollArea::vertical().id_salt("shortcut_pins").max_height(172.0).show(ui, |ui| {
                for (i, command) in pinned.iter().enumerate() {
                    ui.push_id(command.id, |ui| {
                        ui.horizontal(|ui| {
                            if pin_button(ui, i > 0, "↑", format!("Move {} up", command.label)) {
                                change = Some((i, Some(i - 1)));
                            }
                            if pin_button(ui, i + 1 < pinned.len(), "↓", format!("Move {} down", command.label)) {
                                change = Some((i, Some(i + 1)));
                            }
                            if pin_button(ui, true, "×", format!("Remove {}", command.label)) {
                                change = Some((i, None));
                            }
                            ui.label(command.shortcut_row_label());
                        });
                    });
                }
                if pinned.is_empty() {
                    ui.label("No pinned commands");
                }
            });
            if let Some((index, destination)) = change {
                if let Some(destination) = destination {
                    pinned.swap(index, destination);
                } else {
                    pinned.remove(index);
                }
                editor.draft.replace(workspace, &pinned);
            }
            ui.separator();
            let field = ui.add(egui::TextEdit::singleline(&mut editor.query).hint_text("Find a command to add"));
            field.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Find a command to add"));
            #[cfg(target_arch = "wasm32")]
            crate::web::show_control("Find a command to add", field.rect);
            let candidates =
                commands::all().iter().filter(|c| workspace.offers(c) && !pinned.iter().any(|p| p.id == c.id));
            let available = if editor.query.trim().is_empty() {
                let mut commands: Vec<_> = candidates.collect();
                commands.sort_by_key(|c| c.label);
                commands
            } else {
                commands::search_in(candidates, &editor.query, &History::default())
                    .into_iter()
                    .map(|result| result.command)
                    .collect()
            };
            egui::ScrollArea::vertical().id_salt("shortcut_catalog").max_height(152.0).show(ui, |ui| {
                if available.is_empty() {
                    ui.label("No matching commands");
                }
                for command in available {
                    let label = format!("Add {}", command.shortcut_row_label());
                    let response = ui.button(&label).on_hover_text(command.doc);
                    #[cfg(target_arch = "wasm32")]
                    if ui.is_rect_visible(response.rect) {
                        crate::web::show_control(label, response.rect);
                    }
                    if response.clicked() {
                        pinned.push(command);
                        editor.draft.replace(workspace, &pinned);
                    }
                }
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                let response = ui.button("Save shortcut menu");
                #[cfg(target_arch = "wasm32")]
                crate::web::show_control("Save shortcut menu", response.rect);
                save = response.clicked();
                if ui.button("Reset this workspace").clicked() {
                    editor.draft.pinned.remove(&workspace);
                }
                export = ui.button("Export preferences").clicked();
            });
        });
    if export && let Err(error) = crate::settings::export_preferences(&editor.draft) {
        state.note(format!("Could not export preferences: {error}"), NoteKind::Error);
    }
    if save {
        state.shortcut_menu = editor.draft;
        if let Err(error) = crate::settings::save_shortcut_menu(&state.shortcut_menu) {
            state.note(format!("Could not save shortcut menu preferences: {error}"), NoteKind::Error);
        }
    } else if open {
        state.shortcut_menu_editor = Some(editor);
    }
}

fn pin_button(ui: &mut egui::Ui, enabled: bool, icon: &str, label: String) -> bool {
    let response = ui.add_enabled(enabled, egui::Button::new(icon).small());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
    #[cfg(target_arch = "wasm32")]
    crate::web::show_control(&label, response.rect);
    response.on_hover_text(label).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_resolve_and_edits_round_trip_per_workspace() {
        let mut prefs = Preferences::default();
        for workspace in Workspace::ALL {
            assert_eq!(prefs.commands(workspace).iter().map(|c| c.id).collect::<Vec<_>>(), workspace.defaults());
        }
        prefs.replace(Workspace::Sketch, &[commands::get("zoom_fit").unwrap(), commands::get("sketch_line").unwrap()]);
        prefs.replace(Workspace::Drawing, &[]);
        let saved = serde_json::to_string(&prefs).unwrap();
        let reopened: Preferences = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            reopened.commands(Workspace::Sketch).iter().map(|c| c.id).collect::<Vec<_>>(),
            ["zoom_fit", "sketch_line"]
        );
        assert!(reopened.commands(Workspace::Drawing).is_empty());
        assert_eq!(reopened.commands(Workspace::Assembly).len(), Workspace::Assembly.defaults().len());
    }
}
