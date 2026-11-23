//! Display state controls for the tree and configuration view.

use soliddraft_document::{
    ConfigurationId, DisplayStateId,
    display_state::{DisplayMode, DisplayScope, DisplayState, DisplayStates},
};

use crate::state::{AppState, NoteKind};

pub(crate) fn picker<T>(
    ui: &mut egui::Ui,
    states: &DisplayStates<T>,
    config: Option<&ConfigurationId>,
) -> Option<Option<DisplayStateId>> {
    let current = states.selected(config);
    let mut change = None;
    egui::ComboBox::from_label("Display state").selected_text(current.map_or("Default", |s| s.name.as_str())).show_ui(
        ui,
        |ui| {
            if ui.selectable_label(current.is_none(), "Default").clicked() {
                change = Some(None);
            }
            for display in states.states().iter().filter(|s| s.scope.includes(config)) {
                if ui.selectable_label(current.is_some_and(|s| s.id == display.id), &display.name).clicked() {
                    change = Some(Some(display.id.clone()));
                }
            }
        },
    );
    change
}

pub(crate) fn scope_editor(
    ui: &mut egui::Ui,
    scope: &mut DisplayScope,
    current: Option<&ConfigurationId>,
    configs: &[(Option<ConfigurationId>, String)],
) {
    let mut all = matches!(scope, DisplayScope::All);
    if ui.checkbox(&mut all, "All configurations").changed() {
        *scope = if all { DisplayScope::All } else { DisplayScope::Only(vec![current.cloned()]) };
    }
    if let DisplayScope::Only(selected) = scope {
        if ui.button("This configuration").clicked() {
            *selected = vec![current.cloned()];
        }
        for (id, name) in configs {
            let mut checked = selected.contains(id);
            if ui.checkbox(&mut checked, name).changed() {
                if checked {
                    selected.push(id.clone());
                } else {
                    selected.retain(|c| c != id);
                }
            }
        }
    }
}

pub(crate) fn mode_picker(ui: &mut egui::Ui, mode: &mut DisplayMode) {
    egui::ComboBox::from_label("Display mode").selected_text(mode.label()).show_ui(ui, |ui| {
        for item in DisplayMode::ALL {
            ui.selectable_value(mode, item, item.label());
        }
    });
}

pub fn show_part(ui: &mut egui::Ui, state: &mut AppState) {
    let current = state.doc.model().active_configuration().cloned();
    let mut next = state.doc.display_states().clone();
    let mut change = Ok(());
    if let Some(id) = picker(ui, &next, current.as_ref()) {
        change = next.select(current.as_ref(), id);
    }
    let config_list = std::iter::once((None, "Default".to_owned()))
        .chain(state.doc.model().named_configurations().map(|(id, name)| (Some(id.clone()), name.to_owned())))
        .collect::<Vec<_>>();
    ui.collapsing("Manage display states", |ui| {
        if let Some(mut active) = next.selected(current.as_ref()).cloned() {
            let before = active.clone();
            ui.label("Changes to appearances apply to this display state.");
            mode_picker(ui, &mut active.mode);
            for (key, _) in state.doc.model().live_bodies() {
                let mut shown = !active.view.hidden.contains(&key);
                if ui.checkbox(&mut shown, state.doc.body_name(key)).changed() {
                    if shown {
                        active.view.hidden.remove(&key);
                    } else {
                        active.view.hidden.insert(key);
                    }
                }
            }
            ui.collapsing("Assign to configurations", |ui| {
                scope_editor(ui, &mut active.scope, current.as_ref(), &config_list);
                if matches!(&active.scope, DisplayScope::Only(ids) if ids.is_empty()) {
                    active.scope = before.scope.clone();
                }
            });
            if ui.button("Delete display state").clicked() {
                next.remove(&active.id);
            } else if active != before {
                change = change.and_then(|_| next.put(active));
            }
        }
        let id = ui.id().with("new_display_state");
        let mut name = ui.data_mut(|d| d.get_temp::<String>(id).unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut name).hint_text("New display state name"));
        if ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Add display state")).clicked() {
            let scope = DisplayScope::Only(vec![current.clone()]);
            let mode = next.selected(current.as_ref()).map_or(state.viewport.display, |d| d.mode);
            let display = DisplayState::new(name.clone(), scope, mode);
            let created = display.id.clone();
            change = change.and_then(|_| next.put(display)).and_then(|_| next.select(current.as_ref(), Some(created)));
            if change.is_ok() {
                name.clear();
            }
        }
        ui.data_mut(|d| d.insert_temp(id, name));
    });
    if let Err(error) = change
        .and_then(|_| if &next == state.doc.display_states() { Ok(()) } else { state.doc.change_display_states(next) })
    {
        state.note(error, NoteKind::Error);
    }
}

pub(crate) fn show_assembly(ui: &mut egui::Ui, session: &mut crate::assembly::AssemblySession) -> Result<(), String> {
    let mut next = session.doc.assembly().viewport_states.clone();
    if let Some(id) = picker(ui, &next, None) {
        next.select(None, id)?;
    }
    let mut change = Ok(());
    ui.collapsing("Manage display states", |ui| {
        if let Some(mut active) = next.selected(None).cloned() {
            let before = active.clone();
            mode_picker(ui, &mut active.mode);
            if ui.button("Delete display state").clicked() {
                next.remove(&active.id);
            } else if active != before {
                change = next.put(active);
            }
        }
        let id = ui.id().with("new_assembly_display_state");
        let mut name = ui.data_mut(|d| d.get_temp::<String>(id).unwrap_or_default());
        ui.add(egui::TextEdit::singleline(&mut name).hint_text("New display state name"));
        if ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Add display state")).clicked() {
            let mode = next.selected(None).map_or(session.viewport.display, |d| d.mode);
            let display = DisplayState::new(name.clone(), DisplayScope::All, mode);
            let created = display.id.clone();
            change = change.and_then(|_| next.put(display)).and_then(|_| next.select(None, Some(created)));
            if change.is_ok() {
                name.clear();
            }
        }
        ui.data_mut(|d| d.insert_temp(id, name));
    });
    change?;
    session.doc.change_viewport_states(next).map_err(|e| e.to_string())
}
