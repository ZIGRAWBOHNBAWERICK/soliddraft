//! Shared sketch values use the part's existing globals and dimension bindings.

use crate::{
    document::{Command, FeatureId},
    sketch::{ConstraintId, ConstraintKind},
    state::{AppAction, AppState, SketchSel, SketchSession},
    theme,
};
use soliddraft_model::{
    self as model,
    configs::{ValueRow, ValuesTable},
    values::Input,
};
use std::collections::{BTreeMap, BTreeSet};

/// The pending form is session state, not a second set of saved bindings.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkValues {
    pub constraints: Vec<ConstraintId>,
    pub name: String,
    pub value: String,
    pub value_changed: bool,
    pub initial: f64,
    pub revision: u64,
    pub existing_open: bool,
    pub existing_index: usize,
    pub focus_existing: bool,
    pub focus: bool,
    pub error: Option<String>,
}

/// Actions from the property form or command palette.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Open,
    Cancel,
    Apply { unlink: bool },
}

fn cell(row: &ValueRow, column: usize) -> Option<&str> {
    row.cells.get(column).and_then(Option::as_deref).or_else(|| row.cells.first().and_then(Option::as_deref))
}

fn parameter(table: &ValuesTable, feature: model::FeatureId, constraint: ConstraintId) -> Option<&ValueRow> {
    table
        .groups
        .iter()
        .find(|g| g.feature == feature)?
        .rows
        .iter()
        .find(|r| matches!(r.input,Some(Input::Dimension {constraint:c,..}) if c==constraint))
}

fn literal(text: &str) -> bool {
    model::ValueTable::default().parse(text).is_ok_and(|expression| expression.is_literal())
}

/// Direct global aliases are shared values; arbitrary equations remain equations.
pub fn links(doc: &model::Document, feature: FeatureId) -> BTreeMap<ConstraintId, String> {
    let table = doc.values_table();
    table
        .groups
        .iter()
        .filter(|g| u64::from(g.feature.0) == feature)
        .flat_map(|g| &g.rows)
        .filter_map(|row| {
            let Some(Input::Dimension { constraint, .. }) = row.input else { return None };
            let name = cell(row, table.active)?.trim();
            doc.values().has_global(name).then(|| (constraint, name.to_owned()))
        })
        .collect()
}

/// The label selection the command can link, in selection order.
pub fn selected(s: &SketchSession) -> Result<Vec<ConstraintId>, String> {
    if s.drag.is_some()
        || s.trim_drag.is_some()
        || !s.pending.is_empty()
        || s.dim_first.is_some()
        || s.corner_first.is_some()
    {
        return Err("Finish the current sketch gesture first".into());
    }
    if s.dim_edit.is_some() {
        return Err("Finish the current value edit first".into());
    }
    let mut ids = Vec::new();
    let mut angle = None;
    for selection in &s.selection {
        let SketchSel::Constraint(id) = selection else { return Err("Select dimension labels, not geometry".into()) };
        let c = s.sketch.constraint(*id).ok_or("The dimension no longer exists")?;
        if c.is_reference() {
            return Err("Reference dimensions are read-only".into());
        }
        if c.value().is_none() || c.measured_value(&s.sketch).is_none() {
            return Err("Select valid driving dimensions".into());
        }
        let is_angle = matches!(c.kind, ConstraintKind::Angle { .. });
        if angle.is_some_and(|a| a != is_angle) {
            return Err("Link lengths separately from angles".into());
        }
        angle = Some(is_angle);
        ids.push(*id);
    }
    if ids.is_empty() {
        return Err("Select one or more driving dimension labels".into());
    }
    Ok(ids)
}

/// Validate at the document boundary and reuse its atomic command batch.
pub(crate) fn commands(
    doc: &model::Document,
    feature: model::FeatureId,
    ids: &[ConstraintId],
    name: &str,
    value: Option<f64>,
    unlink: bool,
) -> Result<Vec<model::Command>, String> {
    let model::FeatureKind::Sketch(s) = &doc.feature(feature).ok_or("No such sketch")?.kind else {
        return Err("Select a sketch".into());
    };
    let mut seen = BTreeSet::new();
    let mut angle = None;
    let mut first = None;
    for id in ids {
        if !seen.insert(*id) {
            return Err("Select each dimension once".into());
        }
        let c = s.sketch.constraint(*id).ok_or("The dimension no longer exists")?;
        let v = c.value().filter(|v| v.is_finite()).ok_or("Only driving dimensions can be linked")?;
        if c.measured_value(&s.sketch).is_none() {
            return Err("The dimension targets are not valid".into());
        }
        let a = matches!(c.kind, ConstraintKind::Angle { .. });
        if angle.is_some_and(|old| old != a) {
            return Err("Link lengths separately from angles".into());
        }
        angle = Some(a);
        first.get_or_insert(v);
    }
    let first = first.ok_or("Select one or more driving dimensions")?;
    let angle = angle.unwrap_or(false);
    let unit = if angle { "deg" } else { "mm" };
    let table = doc.values_table();
    let configuration = table.columns[table.active].clone();
    let mut commands = Vec::new();
    let name = name.trim();
    if !unlink {
        if name.is_empty() {
            return Err("Type a shared value name".into());
        }
        if let Some(global) = table.globals.iter().find(|g| g.name == name) {
            if !doc.values().has_global(name) {
                return Err("Use a named global, not a measured value".into());
            }
            if !global.unit.is_empty() && global.unit != unit {
                return Err(format!("This selection needs {unit}, not {}", global.unit));
            }
            let current = global
                .values
                .get(table.active)
                .copied()
                .flatten()
                .filter(|v| v.is_finite())
                .ok_or("Resolve the shared value first")?;
            if !angle && current <= 0.0 {
                return Err("A shared length must be greater than zero".into());
            }
            if let Some(value) = value {
                if !cell(global, table.active).is_some_and(literal) {
                    return Err("Edit the shared formula in Values instead".into());
                }
                commands.push(model::Command::SetValueCell {
                    configuration: configuration.clone(),
                    name: name.into(),
                    expression: Some(format!("{value} {unit}")),
                });
            }
        } else {
            let initial = value.unwrap_or(first);
            if !initial.is_finite() || (!angle && initial <= 0.0) {
                return Err("A shared length must be greater than zero".into());
            }
            commands.push(model::Command::SetValue { name: name.into(), expression: format!("{initial} {unit}") });
        }
        if value.is_some_and(|v| !v.is_finite() || (!angle && v <= 0.0)) {
            return Err("Enter a finite value; lengths must be greater than zero".into());
        }
    }
    for id in ids {
        let row = parameter(&table, feature, *id).ok_or("The dimension is not an editable input")?;
        let expression = if unlink {
            let linked = cell(row, table.active).is_some_and(|name| doc.values().has_global(name.trim()));
            if !linked {
                return Err("The selected dimension is not linked to a shared value".into());
            }
            if table.active == 0 {
                None
            } else {
                Some(format!(
                    "{} {unit}",
                    s.sketch.constraint(*id).and_then(|c| c.value()).ok_or("Missing dimension value")?
                ))
            }
        } else {
            Some(name.into())
        };
        commands.push(model::Command::SetValueCell {
            configuration: configuration.clone(),
            name: row.name.clone(),
            expression,
        });
    }
    Ok(commands)
}

/// Open a form without changing the document or sketch selection.
pub fn open(state: &mut AppState) {
    let Some(s) = state.sketch.as_ref() else { return };
    let ids = match selected(s) {
        Ok(ids) => ids,
        Err(error) => {
            if let Some(s) = state.sketch.as_mut() {
                s.say(error);
            }
            return;
        }
    };
    let initial = s.sketch.constraint(ids[0]).and_then(|c| c.value()).unwrap_or(0.0);
    let linked = links(state.doc.model(), s.feature);
    let name = linked
        .get(&ids[0])
        .filter(|name| ids.iter().all(|id| linked.get(id) == Some(name)))
        .cloned()
        .unwrap_or_default();
    let revision = state.doc.revision();
    let s = state.sketch.as_mut().unwrap();
    s.dim_edit = None;
    s.relations_open = false;
    s.link_values = Some(LinkValues {
        constraints: ids,
        name,
        value: initial.to_string(),
        value_changed: false,
        initial,
        revision,
        existing_open: false,
        existing_index: 0,
        focus_existing: false,
        focus: true,
        error: None,
    });
    state.layout.tree.shown = true;
}

/// Escape dismisses the name list before it dismisses the form.
pub fn cancel(state: &mut AppState) {
    let Some(s) = state.sketch.as_mut() else { return };
    if let Some(form) = s.link_values.as_mut().filter(|form| form.existing_open) {
        form.existing_open = false;
        form.focus_existing = true;
    } else {
        s.link_values = None;
    }
}

/// The anchored form keeps the current property-panel vocabulary.
pub fn popup(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(s) = state.sketch.as_mut() else { return };
    let Some(mut form) = s.link_values.take() else { return };
    let table = state.doc.model().values_table();
    let angle = form
        .constraints
        .first()
        .and_then(|id| s.sketch.constraint(*id))
        .is_some_and(|c| matches!(c.kind, ConstraintKind::Angle { .. }));
    let unit = if angle { "deg" } else { "mm" };
    let list_was_open = form.existing_open;
    if form.existing_open && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        form.existing_open = false;
        form.focus_existing = true;
    }
    let mut open = true;
    let mut apply = None;
    let anchor = egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(ui.max_rect().width(), 32.0));
    egui::Popup::new(ui.id().with("sketch_link_values"), ui.ctx().clone(), anchor, ui.layer_id())
        .kind(egui::PopupKind::Popup)
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .width(280.0)
        .show(|ui| {
            ui.strong("Link values");
            ui.label(format!("Active configuration: {}", table.columns[table.active]));
            ui.label(if table.active == 0 {
                "Default bindings and values are inherited by other configurations unless overridden."
            } else {
                "Bindings and value edits override only this configuration."
            });
            let name_label = ui.label("Shared value name");
            let response = ui
                .add(egui::TextEdit::singleline(&mut form.name).desired_width(ui.available_width()))
                .labelled_by(name_label.id);
            if form.focus {
                response.request_focus();
                form.focus = false;
            }
            if response.gained_focus() {
                form.existing_open = false;
            }
            let mut changed = response.changed();
            let existing: Vec<_> =
                table.globals.iter().filter(|r| state.doc.model().values().has_global(&r.name)).collect();
            let chooser = ui.button("Choose existing value");
            let opened_list = chooser.clicked() && !form.existing_open;
            if chooser.clicked() {
                form.existing_open = !form.existing_open;
                form.existing_index = existing.iter().position(|r| r.name == form.name.trim()).unwrap_or(0);
                form.focus_existing = true;
            }
            if form.focus_existing && !form.existing_open {
                chooser.request_focus();
                form.focus_existing = false;
            }
            if form.existing_open {
                form.existing_index = form.existing_index.min(existing.len().saturating_sub(1));
                let tab = ui.input(|i| i.key_pressed(egui::Key::Tab));
                if tab {
                    form.existing_open = false;
                    chooser.request_focus();
                }
                let mut picked = None;
                if !opened_list && !tab {
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
                        form.existing_index = (form.existing_index + 1).min(existing.len().saturating_sub(1));
                        form.focus_existing = true;
                    }
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
                        form.existing_index = form.existing_index.saturating_sub(1);
                        form.focus_existing = true;
                    }
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                        picked = existing.get(form.existing_index).copied();
                    }
                }
                if existing.is_empty() {
                    ui.label("No shared values yet. Type a new name above.");
                }
                egui::ScrollArea::vertical().id_salt("shared_names").max_height(100.0).show(ui, |ui| {
                    for (index, row) in existing.iter().enumerate() {
                        let response = ui.selectable_label(index == form.existing_index, &row.name);
                        if response.clicked() {
                            picked = Some(*row);
                        }
                        if form.focus_existing && index == form.existing_index {
                            response.request_focus();
                            response.scroll_to_me(None);
                        }
                    }
                });
                form.focus_existing = false;
                if let Some(row) = picked {
                    form.name = row.name.clone();
                    changed = true;
                    form.existing_open = false;
                    chooser.request_focus();
                }
            }
            let global = table.globals.iter().find(|g| g.name == form.name.trim());
            if changed {
                form.value = global
                    .and_then(|r| r.values.get(table.active).copied().flatten())
                    .unwrap_or(form.initial)
                    .to_string();
                form.value_changed = false;
                form.error = None;
            }
            let editable = global.is_none_or(|r| cell(r, table.active).is_some_and(literal));
            let value_label = ui.label(format!("Shared numeric value ({unit})"));
            let response = ui
                .add_enabled(editable, egui::TextEdit::singleline(&mut form.value).desired_width(ui.available_width()))
                .labelled_by(value_label.id);
            if response.gained_focus() {
                form.existing_open = false;
            }
            form.value_changed |= response.changed();
            if !editable {
                ui.label("Edit the shared formula in Values instead.");
            }
            ui.label("Changing this value updates every linked member.");
            ui.label("Linking replaces expressions on the selected dimensions.");
            if let Some(error) = &form.error {
                ui.colored_label(theme::error(), error);
            }
            ui.horizontal(|ui| {
                if ui.button("Link selected").clicked() {
                    apply = Some(false);
                }
                if ui.button("Unlink selected").clicked() {
                    apply = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    ui.close();
                }
            });
            if !list_was_open
                && !opened_list
                && !form.existing_open
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                apply = Some(false);
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                ui.close();
            }
        });
    if open {
        state.sketch.as_mut().unwrap().link_values = Some(form);
    }
    if let Some(unlink) = apply {
        state.push(AppAction::SketchValues(Action::Apply { unlink }));
    }
}

/// Apply through the same undoable document facade as other property edits.
pub fn apply(state: &mut AppState, unlink: bool) -> bool {
    let Some(s) = state.sketch.as_ref() else { return false };
    let Some(form) = s.link_values.as_ref() else { return false };
    if form.revision != state.doc.revision() {
        state.sketch.as_mut().unwrap().link_values.as_mut().unwrap().error =
            Some("The part changed. Close and reopen Link values.".into());
        return false;
    }
    if !unlink && !form.value_changed {
        let current = links(state.doc.model(), s.feature);
        if form.constraints.iter().all(|id| current.get(id).is_some_and(|name| name == form.name.trim())) {
            state.sketch.as_mut().unwrap().link_values = None;
            return true;
        }
    }
    let value = if form.value_changed && !unlink {
        match form.value.trim().parse::<f64>() {
            Ok(v) => Some(v),
            Err(_) => {
                state.sketch.as_mut().unwrap().link_values.as_mut().unwrap().error = Some("Enter a number".into());
                return false;
            }
        }
    } else {
        None
    };
    let command = if unlink {
        Command::UnlinkSketchValues { id: s.feature, constraints: form.constraints.clone() }
    } else {
        Command::LinkSketchValues {
            id: s.feature,
            constraints: form.constraints.clone(),
            name: form.name.clone(),
            value,
        }
    };
    match state.doc.apply(command) {
        Ok(_) => {
            state.sketch.as_mut().unwrap().link_values = None;
            true
        }
        Err(error) => {
            state.sketch.as_mut().unwrap().link_values.as_mut().unwrap().error = Some(crate::values_tab::plain(&error));
            false
        }
    }
}
