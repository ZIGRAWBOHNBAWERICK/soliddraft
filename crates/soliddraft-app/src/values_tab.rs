//! Parameters and configurations, shared by the dock and the expanded view.

use std::collections::BTreeMap;

use crate::document::Command;
use crate::grid::exchange::Format;
use crate::state::{AppAction, AppState};
use crate::theme;
use egui::RichText;
use soliddraft_model::configs::{DEFAULT_ROW, Edit, ValueRow, ValuesTable};

/// Which text field an edit belongs to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    /// A global's or a parameter's cell in a column.
    Cell(String, usize),
    /// A global's or a parameter's name.
    Name(String),
    /// A text property's cell in a column.
    Property(String, usize),
    /// A parameter's comment.
    Comment(String),
    /// The new global's name.
    NewGlobalName,
    /// The new global's formula.
    NewGlobalFormula,
    /// The new configuration's name.
    NewConfiguration,
    /// The new property's name.
    NewPropertyName,
    /// The new property's text.
    NewPropertyText,
}

/// What the tab asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum ValuesAction {
    /// Brings the tab forward, showing the side panel if it is hidden.
    Show,
    /// Applies a typed cell, name or new row.
    Commit(Field, String),
    /// Deletes a global.
    RemoveGlobal(String),
    /// Adds the typed global.
    AddGlobal,
    /// Adds the typed configuration.
    AddConfiguration,
    /// Makes a column the selected configuration.
    Activate(String),
    /// Edits the selected configuration or parameter.
    Edit(Edit),
    /// Adds the typed text property.
    AddProperty,
    /// Asks for a table file and applies it.
    Import,
    /// Asks where to write the table of globals and properties.
    Export(Format),
}

/// The tab's session state. Document truth stays in the part.
#[derive(Clone, Debug, Default)]
pub struct ValuesTab {
    table: Option<((u64, u64), ValuesTable)>,
    document: Option<soliddraft_document::DocumentId>,
    import_request: Option<(soliddraft_document::DocumentId, u64)>,
    expanded: bool,
    filter: String,
    config_name: Option<TabName>,
    completions: Vec<String>,
    /// Typed text not yet applied, or refused.
    drafts: BTreeMap<Field, String>,
    /// Why a field's text was refused, in words.
    errors: BTreeMap<Field, String>,
    formula_errors: BTreeMap<Field, &'static str>,
    /// A refusal just made, to scroll into view once.
    reveal: Option<Field>,
    /// The tab's last message.
    pub message: Option<String>,
}

#[derive(Clone, Debug)]
enum TabName {
    Add,
    Rename(String),
    Duplicate(String),
}

impl ValuesTab {
    /// The refusal shown on a field, if any.
    pub fn error(&self, field: &Field) -> Option<&str> {
        self.errors.get(field).map(String::as_str)
    }
}

/// The part's table, made again only when the part changes.
fn table(state: &mut AppState) -> ValuesTable {
    let document = state.doc.model().id();
    if state.values.document != Some(document) {
        state.values.table = None;
        state.values.drafts.clear();
        state.values.errors.clear();
        state.values.formula_errors.clear();
        state.values.config_name = None;
        state.values.document = Some(document);
    }
    let revision = (state.doc.revision(), state.doc.displayed_revision());
    match &state.values.table {
        Some((r, t)) if *r == revision => t.clone(),
        _ => {
            let t = state.doc.model().values_table();
            state.values.completions = completions(&t);
            state.values.table = Some((revision, t.clone()));
            t
        }
    }
}

/// True for text that is a number, with or without a unit.
fn plain_number(text: &str) -> bool {
    let mut words = text.split_whitespace();
    let number = words.next().is_some_and(|w| w.parse::<f64>().is_ok());
    let unit = words.next().is_none_or(|u| matches!(u, "mm" | "cm" | "m" | "in" | "deg"));
    number && unit && words.next().is_none()
}

/// A number as a cell shows it.
fn number(v: f64) -> String {
    let text = format!("{v:.4}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Stored text as a cell shows it: a formula starts with `=`.
fn shown(text: &str) -> String {
    if plain_number(text) || text.starts_with('=') { text.to_owned() } else { format!("={text}") }
}

/// A value with its unit, for a hover.
fn with_unit(v: f64, unit: &str) -> String {
    if unit.is_empty() { number(v) } else { format!("{} {unit}", number(v)) }
}

/// A refusal in plain words.
pub fn plain(message: &str) -> String {
    let after = |key: &str| message.split(key).nth(1).map(|s| s.trim().trim_end_matches('.').to_owned());
    if let Some(name) = after("unknown value ") {
        format!("Nothing is named {name}")
    } else if message.contains("value cycle") {
        "This formula depends on itself".into()
    } else if message.contains("too deep") {
        "This formula goes through too many other values".into()
    } else if message.contains("division by zero") {
        "This divides by zero".into()
    } else if message.contains("incompatible units") {
        "These units cannot be added together".into()
    } else if message.contains("needs a length") || message.contains("expected length") {
        "This needs a length, such as 10 or 10 mm".into()
    } else if message.contains("needs an angle") || message.contains("expected angle") {
        "This needs an angle, such as 30 or 30 deg".into()
    } else if message.contains("needs a count") {
        "This needs a whole number".into()
    } else if message.contains("needs a plain number") {
        "This needs a plain number, such as 0.3".into()
    } else if message.contains("finite number") {
        "The result is not a usable number".into()
    } else if message.contains("unsupported character") {
        "Check the characters in the formula".into()
    } else if message.contains("expected a value")
        || message.contains("missing )")
        || message.contains("unexpected token")
    {
        "This formula is not complete".into()
    } else if let Some(unit) = after("unknown unit ") {
        format!("{unit} is not a unit; use mm, cm, m, in or deg")
    } else if let Some(name) = after("the name ") {
        format!("{} is already used", name.trim_end_matches(" is already taken"))
    } else if message.contains("must be an identifier") || message.contains("letters, digits and underscores") {
        "A name uses letters, digits and underscores, starts with a letter and is not a unit".into()
    } else {
        let mut chars = message.chars();
        chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
    }
}

/// One editable field. Returns the text to apply when the person presses Enter or leaves it.
/// What a field shows and how assistive tools name it.
struct Text<'a> {
    current: &'a str,
    hint: &'a str,
    label: &'a str,
}

fn field(
    ui: &mut egui::Ui,
    tab: &mut ValuesTab,
    key: Field,
    text: Text<'_>,
    width: f32,
    frame: bool,
) -> Option<String> {
    let Text { current, hint, label } = text;
    let mut text = tab.drafts.get(&key).cloned().unwrap_or_else(|| current.to_owned());
    let error = tab.errors.get(&key).cloned();
    let mut edit =
        egui::TextEdit::singleline(&mut text).hint_text(hint).desired_width(width).min_size(egui::vec2(width, 0.0));
    if !frame {
        edit = edit.frame(egui::Frame::NONE);
    }
    if error.is_some() {
        edit = edit.text_color(theme::error());
    }
    let response = ui.add(edit);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    if response.changed() {
        tab.drafts.insert(key.clone(), text.clone());
    }
    if matches!(key, Field::Cell(..) | Field::NewGlobalFormula)
        && (response.has_focus() || response.lost_focus())
        && text.starts_with('=')
        && let Some(completed) = complete(ui, &response, &text, &tab.completions)
    {
        tab.drafts.insert(key, completed);
        response.request_focus();
        return None;
    }
    let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if response.lost_focus() && escaped {
        tab.drafts.remove(&key);
        tab.errors.remove(&key);
        tab.formula_errors.remove(&key);
        return None;
    }
    if response.lost_focus() && tab.drafts.get(&key).is_some_and(|d| d == current) {
        tab.drafts.remove(&key);
        tab.errors.remove(&key);
        tab.formula_errors.remove(&key);
    }
    let commit = response.lost_focus() && tab.drafts.contains_key(&key);
    commit.then_some(text)
}

fn complete(ui: &mut egui::Ui, edit: &egui::Response, text: &str, options: &[String]) -> Option<String> {
    let (at, separator) =
        text.char_indices().rev().find(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.')))?;
    let start = at + separator.len_utf8();
    let token = &text[start..];
    if token.is_empty() || token.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let lower = token.to_ascii_lowercase();
    let matches: Vec<_> = options
        .iter()
        .filter(|name| name.to_ascii_lowercase().starts_with(&lower) && !name.eq_ignore_ascii_case(token))
        .take(5)
        .collect();
    if matches.is_empty() {
        return None;
    }
    let mut chosen = None;
    if edit.has_focus() && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)) {
        chosen = matches.first().map(|name| name.as_str());
    } else {
        egui::Popup::from_response(edit).open(true).show(|ui| {
            for name in matches {
                if ui.selectable_label(false, name).clicked() {
                    chosen = Some(name.as_str());
                }
            }
        });
    }
    chosen.map(|name| format!("{}{name}", &text[..start]))
}

fn parameter_row(
    ui: &mut egui::Ui,
    tab: &mut ValuesTab,
    row: &ValueRow,
    table: &ValuesTable,
    width: f32,
    compact: bool,
    actions: &mut Vec<ValuesAction>,
) {
    let column = table.active;
    let configuration = &table.columns[column];
    let authored = row.cells[column].as_deref();
    let inherited = column != 0 && authored.is_none();
    let result = row.values[column];
    let current = if let Some(text) = authored {
        shown(text)
    } else if inherited {
        String::new()
    } else {
        result.map(number).unwrap_or_default()
    };
    let hint = if inherited {
        row.cells[0].as_deref().map(shown).or_else(|| row.values[0].map(number)).unwrap_or_default()
    } else {
        String::new()
    };
    let differs = column != 0
        && (authored.is_some() && authored != row.cells[0].as_deref()
            || result.zip(row.values[0]).is_some_and(|(current, default)| current != default)
            || row.logical[column] != row.logical[0]);
    ui.horizontal(|ui| {
        name_cell(ui, tab, row, (width * 0.25).clamp(136.0, 300.0), actions);
        if row.input.is_none() {
            let delete = ui.small_button("×").on_hover_text(format!("Delete {}", row.name));
            delete.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Delete {}", row.name))
            });
            if delete.clicked() {
                actions.push(ValuesAction::RemoveGlobal(row.name.clone()));
            }
        }
    });
    if !compact {
        ui.label(RichText::new(&row.unit).color(theme::muted()));
    }
    let key = Field::Cell(row.name.clone(), column);
    ui.vertical(|ui| {
        let label = format!("{configuration} {}", row.name);
        let edit = |ui: &mut egui::Ui, tab: &mut ValuesTab| {
            let width = if compact { 94.0 } else { width * 0.24 };
            field(ui, tab, key.clone(), Text { current: &current, hint: &hint, label: &label }, width, true)
        };
        let commit = if differs {
            let marked = egui::Frame::new()
                .fill(theme::selection_bg())
                .corner_radius(3.0)
                .inner_margin(egui::Margin::same(2))
                .show(ui, |ui| edit(ui, tab));
            marked.response.on_hover_text("Differs from Default");
            marked.inner
        } else {
            edit(ui, tab)
        };
        if let Some(text) = commit {
            actions.push(ValuesAction::Commit(key.clone(), text));
        }
        error_line(ui, tab, &key);
    });
    if let Some(code) = tab.formula_errors.get(&key) {
        ui.label(RichText::new(*code).color(theme::error())).on_hover_text(tab.error(&key).unwrap_or_default());
    } else if let Some(reason) = &row.errors[column] {
        ui.label(RichText::new(soliddraft_model::formula_error_code(reason)).color(theme::error()))
            .on_hover_text(plain(reason));
    } else if let Some(value) = result {
        let text =
            if row.logical[column] { if value == 0.0 { "FALSE".into() } else { "TRUE".into() } } else { number(value) };
        ui.label(RichText::new(text).family(theme::medium())).on_hover_text(with_unit(value, &row.unit));
    } else {
        ui.label("-");
    }
    if compact {
        ui.label(RichText::new(&row.unit).color(theme::muted()));
    }
    let label = format!("Comment {}", row.name);
    let comment = Field::Comment(row.name.clone());
    if let Some(text) =
        field(ui, tab, comment.clone(), Text { current: &row.comment, hint: "", label: &label }, width * 0.24, true)
    {
        actions.push(ValuesAction::Commit(comment, text));
    }
    ui.end_row();
}

/// A row's name, editable in place.
fn name_cell(ui: &mut egui::Ui, tab: &mut ValuesTab, row: &ValueRow, width: f32, actions: &mut Vec<ValuesAction>) {
    let key = Field::Name(row.name.clone());
    ui.vertical(|ui| {
        let label = format!("Name {}", row.name);
        if let Some(text) =
            field(ui, tab, key.clone(), Text { current: &row.name, hint: "", label: &label }, width, false)
        {
            actions.push(ValuesAction::Commit(key.clone(), text));
        }
        error_line(ui, tab, &key);
    })
    .response
    .on_hover_text(if row.input.is_some() {
        format!("{}: use this name in formulas. Click to rename.", row.label)
    } else {
        "Use this name in formulas. Click to rename.".to_owned()
    });
}

/// A field's refusal under it, scrolled into view the first time it shows.
fn error_line(ui: &mut egui::Ui, tab: &mut ValuesTab, key: &Field) {
    if let Some(error) = tab.errors.get(key) {
        let response = ui.label(RichText::new(error).color(theme::error()).size(11.0));
        if tab.reveal.as_ref() == Some(key) {
            response.scroll_to_me(None);
            tab.reveal = None;
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).family(theme::semibold()).size(12.0));
    ui.end_row();
}

/// Mounts the panel in the part dock and opens its full-window view on request.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    ui.set_min_width(ui.available_width());
    if state.assembly.is_active() || state.drawing.is_active() {
        ui.label("Switch to the part to edit parameters and configurations.");
        return;
    }
    if state.values.expanded {
        let ctx = ui.ctx().clone();
        let size = (ctx.content_rect().size() - egui::vec2(48.0, 48.0)).max(egui::Vec2::ZERO);
        let modal = egui::Modal::new(egui::Id::new("expanded_parameters")).show(&ctx, |ui| {
            ui.set_width(size.x);
            ui.set_height(size.y);
            contents(ui, state);
        });
        if modal.should_close() {
            state.values.expanded = false;
        }
    } else {
        contents(ui, state);
    }
}

fn tab_menu(ui: &mut egui::Ui, tab: &mut ValuesTab, table: &ValuesTable, c: usize, actions: &mut Vec<ValuesAction>) {
    let configuration = table.columns[c].clone();
    if ui.button("Duplicate tab").clicked() {
        tab.config_name = Some(TabName::Duplicate(configuration.clone()));
        tab.drafts.insert(Field::NewConfiguration, format!("{configuration} copy"));
        tab.errors.remove(&Field::NewConfiguration);
        ui.close();
    }
    if c == 0 {
        return;
    }
    if ui.button("Rename tab").clicked() {
        tab.config_name = Some(TabName::Rename(configuration.clone()));
        tab.drafts.insert(Field::NewConfiguration, configuration.clone());
        tab.errors.remove(&Field::NewConfiguration);
        ui.close();
    }
    for (label, offset, enabled) in [("Move left", -1, c > 1), ("Move right", 1, c + 1 < table.columns.len())] {
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            actions.push(ValuesAction::Edit(Edit::Move { configuration: configuration.clone(), offset }));
            ui.close();
        }
    }
    if ui.button("Delete tab").clicked() {
        actions.push(ValuesAction::Edit(Edit::Delete { configuration }));
        ui.close();
    }
}

fn tabs(ui: &mut egui::Ui, tab: &mut ValuesTab, table: &ValuesTable, actions: &mut Vec<ValuesAction>) {
    egui::ScrollArea::horizontal().id_salt("configuration_tabs").show(ui, |ui| {
        ui.horizontal(|ui| {
            for (c, column) in table.columns.iter().enumerate() {
                let active = c == table.active;
                let title = RichText::new(column).family(if active { theme::semibold() } else { theme::medium() });
                let response = ui.selectable_label(active, title);
                response.widget_info(|| {
                    egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, format!("Column {column}"))
                });
                response.context_menu(|ui| tab_menu(ui, tab, table, c, actions));
                if response.clicked() && !active {
                    actions.push(ValuesAction::Activate(column.clone()));
                }
            }
            let add = ui.small_button("+").on_hover_text("Add configuration");
            add.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New configuration"));
            if add.clicked() {
                tab.config_name = Some(TabName::Add);
                tab.drafts.remove(&Field::NewConfiguration);
                tab.errors.remove(&Field::NewConfiguration);
            }
            ui.menu_button("...", |ui| tab_menu(ui, tab, table, table.active, actions))
                .response
                .on_hover_text("Configuration actions");
        });
    });
    if let Some(mode) = &tab.config_name {
        let label = match mode {
            TabName::Add => "Add configuration",
            TabName::Rename(_) => "Rename configuration",
            TabName::Duplicate(_) => "Duplicate configuration",
        };
        ui.horizontal_wrapped(|ui| {
            let entered = field(
                ui,
                tab,
                Field::NewConfiguration,
                Text { current: "", hint: "Configuration name", label: "New configuration name" },
                124.0,
                true,
            );
            if ui.button(label).clicked() || entered.is_some() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                actions.push(ValuesAction::AddConfiguration);
            }
            if ui.button("Cancel").clicked() {
                tab.config_name = None;
            }
        });
        error_line(ui, tab, &Field::NewConfiguration);
    }
}

fn add_global(ui: &mut egui::Ui, tab: &mut ValuesTab, actions: &mut Vec<ValuesAction>) {
    ui.horizontal_wrapped(|ui| {
        field(
            ui,
            tab,
            Field::NewGlobalName,
            Text { current: "", hint: "New global", label: "New global name" },
            105.0,
            true,
        );
        let formula = field(
            ui,
            tab,
            Field::NewGlobalFormula,
            Text { current: "", hint: "Value or =formula", label: "New global formula" },
            105.0,
            true,
        );
        let add = ui.small_button("Add");
        add.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add global"));
        if add.clicked() || formula.is_some() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            actions.push(ValuesAction::AddGlobal);
        }
    });
    error_line(ui, tab, &Field::NewGlobalName);
    error_line(ui, tab, &Field::NewGlobalFormula);
}

fn contents(ui: &mut egui::Ui, state: &mut AppState) {
    let table = table(state);
    let mut actions = Vec::new();
    let mut history = None;
    let tab = &mut state.values;
    if tab.completions.is_empty() {
        tab.completions = completions(&table);
    }
    ui.horizontal_wrapped(|ui| {
        let title = if tab.expanded { 15.0 } else { 12.5 };
        ui.label(RichText::new("Parameters & configurations").family(theme::semibold()).size(title));
        let expand = if tab.expanded { "Back to dock" } else { "Expand" };
        if ui.small_button(expand).clicked() {
            tab.expanded = !tab.expanded;
        }
    });
    tabs(ui, tab, &table, &mut actions);
    crate::display_states::show_part(ui, state);
    let model = state.doc.model();
    let grid = model.configuration_grid();
    let configuration = &table.columns[table.active];
    let tab = &mut state.values;
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        let filter =
            ui.add(egui::TextEdit::singleline(&mut tab.filter).hint_text("Filter parameters").desired_width(122.0));
        filter.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Filter parameters"));
        if ui.add_enabled(state.doc.undo_label().is_some(), egui::Button::new("Undo")).clicked() {
            history = Some(AppAction::Undo);
        }
        if ui.add_enabled(state.doc.redo_label().is_some(), egui::Button::new("Redo")).clicked() {
            history = Some(AppAction::Redo);
        }
        ui.menu_button("Table file", |ui| {
            if ui.button("Import table").clicked() {
                actions.push(ValuesAction::Import);
                ui.close();
            }
            for (name, format) in
                [("Export CSV", Format::Csv), ("Export XLSX", Format::Xlsx), ("Export JSON", Format::Json)]
            {
                if ui.button(name).clicked() {
                    actions.push(ValuesAction::Export(format));
                    ui.close();
                }
            }
        });
        let mut degrees = model.formula_degrees();
        if ui
            .checkbox(&mut degrees, "Trig in degrees")
            .on_hover_text("Interpret plain angles as degrees in formulas")
            .changed()
        {
            actions.push(ValuesAction::Edit(Edit::Degrees(degrees)));
        }
    });
    if let Some(message) = &tab.message {
        ui.label(RichText::new(message).color(theme::muted()));
    }
    add_global(ui, tab, &mut actions);
    ui.add_space(4.0);
    let filter = tab.filter.trim().to_ascii_lowercase();
    let matches = |text: &str| filter.is_empty() || text.to_ascii_lowercase().contains(&filter);
    let show_row = |row: &ValueRow| {
        matches(&row.name)
            || matches(&row.label)
            || matches(&row.comment)
            || row.cells[table.active].as_deref().is_some_and(matches)
    };
    let compact = ui.available_width() < 480.0;
    let width = (ui.available_width() - 60.0).clamp(430.0, 1440.0);
    egui::ScrollArea::both().id_salt("parameters_scroll").auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("parameters_table").num_columns(5).striped(true).spacing([8.0, 5.0]).show(ui, |ui| {
            let columns = if compact {
                ["Name", "Expression", "Evaluates to", "Unit", "Comment"]
            } else {
                ["Name", "Unit", "Expression", "Evaluates to", "Comment"]
            };
            for title in columns {
                ui.label(RichText::new(title).family(theme::semibold()).color(theme::muted()));
            }
            ui.end_row();
            section(ui, "Global parameters");
            for row in table.globals.iter().filter(|row| show_row(row)) {
                parameter_row(ui, tab, row, &table, width, compact, &mut actions);
            }
            if !table.groups.is_empty() {
                section(ui, "Model parameters");
            }
            for group in &table.groups {
                if !matches(&group.name) && !group.rows.iter().any(show_row) {
                    continue;
                }
                section(ui, &group.name);
                for row in group.rows.iter().filter(|row| matches(&group.name) || show_row(row)) {
                    parameter_row(ui, tab, row, &table, width, compact, &mut actions);
                }
            }
        });
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Feature suppression").id_salt("parameter_feature_suppression").show(ui, |ui| {
            for feature in model.features() {
                let Some((default, override_value)) = model.configuration_suppression(configuration, feature.id) else {
                    continue;
                };
                ui.horizontal(|ui| {
                    let mut suppressed = override_value.unwrap_or(default);
                    let response = ui.checkbox(&mut suppressed, &feature.name);
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Checkbox,
                            true,
                            suppressed,
                            format!("Suppress {}", feature.name),
                        )
                    });
                    if response.changed() {
                        actions.push(ValuesAction::Edit(Edit::Suppress {
                            configuration: configuration.clone(),
                            feature: feature.id,
                            suppressed: Some(suppressed),
                        }));
                    }
                    if override_value.is_some() && ui.small_button("Use Default").clicked() {
                        actions.push(ValuesAction::Edit(Edit::Suppress {
                            configuration: configuration.clone(),
                            feature: feature.id,
                            suppressed: None,
                        }));
                    }
                });
            }
        });
        egui::CollapsingHeader::new("Custom properties").id_salt("parameter_properties").show(ui, |ui| {
            properties(ui, tab, &grid, &table, &mut actions);
        });
    });
    for action in actions {
        state.push(AppAction::Values(action));
    }
    if let Some(action) = history {
        state.push(action);
    }
}

fn completions(table: &ValuesTable) -> Vec<String> {
    let mut names: Vec<_> = table
        .globals
        .iter()
        .chain(table.groups.iter().flat_map(|group| &group.rows))
        .map(|row| row.name.clone())
        .collect();
    names.extend(soliddraft_model::FORMULA_FUNCTIONS.iter().map(|name| format!("{name}(")));
    names.extend(["TRUE".into(), "FALSE".into(), "Mass".into(), "Volume".into()]);
    names.sort_unstable_by_key(|name| name.to_ascii_lowercase());
    names
}

fn properties(
    ui: &mut egui::Ui,
    tab: &mut ValuesTab,
    grid: &soliddraft_model::configs::Grid,
    table: &ValuesTable,
    actions: &mut Vec<ValuesAction>,
) {
    let c = table.active;
    for property in &grid.properties {
        ui.horizontal(|ui| {
            ui.label(property);
            let authored = grid.rows[c].properties.get(property).map_or("", String::as_str);
            let default = grid.rows[0].properties.get(property).map_or("", String::as_str);
            let hint = if c == 0 { "" } else { default };
            let key = Field::Property(property.clone(), c);
            let label = format!("{} property {property}", table.columns[c]);
            if let Some(text) =
                field(ui, tab, key.clone(), Text { current: authored, hint, label: &label }, 128.0, true)
            {
                actions.push(ValuesAction::Commit(key.clone(), text));
            }
            error_line(ui, tab, &key);
        });
    }
    ui.horizontal_wrapped(|ui| {
        field(
            ui,
            tab,
            Field::NewPropertyName,
            Text { current: "", hint: "New property", label: "New property name" },
            105.0,
            true,
        );
        field(
            ui,
            tab,
            Field::NewPropertyText,
            Text { current: "", hint: "Text", label: "New property text" },
            105.0,
            true,
        );
        if ui.button("Add property").clicked() {
            actions.push(ValuesAction::AddProperty);
        }
    });
    error_line(ui, tab, &Field::NewPropertyName);
    error_line(ui, tab, &Field::NewPropertyText);
}

/// Why the part cannot take a value now.
fn busy(state: &AppState) -> Option<&'static str> {
    (state.drag_preview.is_some() || state.editor.is_some() || state.sketch.is_some())
        .then_some("Finish the open edit first")
}

/// Runs a part command for `key`: a refusal stays on that field, success clears it.
fn run(state: &mut AppState, key: Field, command: Command) -> bool {
    if let Some(reason) = busy(state) {
        state.values.errors.insert(key, reason.to_owned());
        return false;
    }
    match state.doc.apply(command) {
        Ok(_) => {
            state.values.drafts.remove(&key);
            state.values.errors.remove(&key);
            state.values.formula_errors.remove(&key);
            state.values.table = None;
            true
        }
        Err(e) => {
            if matches!(&key, Field::Cell(..) | Field::NewGlobalFormula) {
                state.values.formula_errors.insert(key.clone(), soliddraft_model::formula_error_code(&e));
            }
            state.values.errors.insert(key.clone(), plain(&e));
            state.values.reveal = Some(key);
            false
        }
    }
}

/// A typed expression, keeping the leading `=` on spreadsheet formulas.
fn formula(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn draft(state: &AppState, key: &Field) -> String {
    state.values.drafts.get(key).map(|t| t.trim().to_owned()).unwrap_or_default()
}

/// Runs what the tab asked for.
pub fn apply(state: &mut AppState, action: ValuesAction) {
    match action {
        ValuesAction::Show => state.show_pane(crate::panels::Pane::Values),
        ValuesAction::Commit(key, text) => commit(state, key, text),
        ValuesAction::RemoveGlobal(name) => {
            run(state, Field::Name(name.clone()), Command::RemoveValue { name });
        }
        ValuesAction::AddGlobal => {
            let name = draft(state, &Field::NewGlobalName);
            let typed = draft(state, &Field::NewGlobalFormula);
            if name.is_empty() && typed.is_empty() {
                return;
            }
            let Some(expression) = formula(&typed) else {
                state.values.errors.insert(Field::NewGlobalFormula, "Type a value for the new global".into());
                return;
            };
            if name.is_empty() {
                state.values.errors.insert(Field::NewGlobalName, "Type a name for the new global".into());
                return;
            }
            if state.doc.model().values().has_global(&name) {
                state.values.errors.insert(Field::NewGlobalName, format!("{name} is already used"));
                return;
            }
            if run(state, Field::NewGlobalFormula, Command::SetValue { name, expression }) {
                state.values.drafts.remove(&Field::NewGlobalName);
                state.values.errors.remove(&Field::NewGlobalName);
            }
        }
        ValuesAction::AddConfiguration => {
            let name = draft(state, &Field::NewConfiguration);
            if name.is_empty() {
                state.values.errors.insert(Field::NewConfiguration, "Type a name for the configuration".into());
                return;
            }
            let command = match &state.values.config_name {
                Some(TabName::Rename(configuration)) => {
                    Command::EditParameters(Edit::Rename { configuration: configuration.clone(), name })
                }
                Some(TabName::Duplicate(configuration)) => {
                    Command::EditParameters(Edit::Duplicate { configuration: configuration.clone(), name })
                }
                _ => Command::AddConfiguration { name },
            };
            if run(state, Field::NewConfiguration, command) {
                state.values.config_name = None;
            }
        }
        ValuesAction::Edit(edit) => {
            let changed_order = matches!(edit, Edit::Move { .. } | Edit::Delete { .. });
            state.values.message = if let Some(reason) = busy(state) {
                Some(reason.into())
            } else {
                match state.doc.apply(Command::EditParameters(edit)) {
                    Ok(_) => {
                        state.values.table = None;
                        if changed_order {
                            let is_current = |field: &Field| !matches!(field, Field::Cell(..) | Field::Property(..));
                            state.values.drafts.retain(|key, _| is_current(key));
                            state.values.errors.retain(|key, _| is_current(key));
                            state.values.formula_errors.retain(|key, _| is_current(key));
                        }
                        None
                    }
                    Err(error) => Some(plain(&error)),
                }
            };
        }
        ValuesAction::Activate(name) => {
            if let Some(reason) = busy(state) {
                state.values.message = Some(reason.to_owned());
            } else if let Err(e) = state.doc.apply(Command::ActivateConfiguration { name }) {
                state.values.message = Some(plain(&e));
            } else {
                state.values.message = None;
                state.values.table = None;
            }
        }
        ValuesAction::AddProperty => {
            let name = draft(state, &Field::NewPropertyName);
            let text = draft(state, &Field::NewPropertyText);
            if name.is_empty() {
                state.values.errors.insert(Field::NewPropertyName, "Type a name for the property".into());
                return;
            }
            let active = table(state).active;
            if set_property(state, Field::NewPropertyText, &name, active, &text) {
                state.values.drafts.remove(&Field::NewPropertyName);
                state.values.errors.remove(&Field::NewPropertyName);
            }
        }
        ValuesAction::Import => state.values.message = import(state),
        ValuesAction::Export(format) => state.values.message = export(state, format),
    }
}

fn commit(state: &mut AppState, key: Field, text: String) {
    let columns = table(state).columns;
    match key.clone() {
        Field::Cell(name, c) => {
            let configuration = columns.get(c).cloned().unwrap_or_else(|| DEFAULT_ROW.to_owned());
            run(state, key, Command::SetValueCell { configuration, name, expression: formula(&text) });
        }
        Field::Name(name) => {
            let to = text.trim().to_owned();
            run(state, key, Command::RenameValue { name, to });
        }
        Field::Property(name, c) => {
            set_property(state, key, &name, c, text.trim());
        }
        Field::Comment(name) => {
            run(state, key, Command::EditParameters(Edit::Comment { name, text: text.trim().to_owned() }));
        }
        // The new rows apply through their own buttons.
        _ => {}
    }
}

/// Sets one text property cell as one undo step.
fn set_property(state: &mut AppState, key: Field, name: &str, column: usize, text: &str) -> bool {
    let Some(configuration) = table(state).columns.get(column).cloned() else { return false };
    run(state, key, Command::EditParameters(Edit::Property { configuration, name: name.into(), text: text.into() }))
}

fn import(state: &mut AppState) -> Option<String> {
    if let Some(reason) = busy(state) {
        return Some(reason.into());
    }
    state.values.import_request = Some((state.doc.model().id(), state.doc.revision()));
    let path = crate::platform::open_parameters(&["csv", "xlsx", "json"])?;
    state.values.import_request = None;
    let applied = crate::grid::tools::preview_file(state, &path).and_then(|_| apply_import(state));
    state.values.table = None;
    Some(import_message(applied))
}

fn apply_import(state: &mut AppState) -> Result<(), String> {
    let id = state.grid.shown().map(|p| p.id.clone()).ok_or("the file could not be read")?;
    crate::grid::preview::apply(state, &id)?;
    Ok(())
}

fn import_message(result: Result<(), String>) -> String {
    match result {
        Ok(()) => "Imported table".into(),
        Err(error) => plain(&error),
    }
}

pub(crate) fn apply_upload(state: &mut AppState, files: Vec<(String, Vec<u8>)>) {
    let pending = state.values.import_request.take();
    let result = (|| {
        if pending != Some((state.doc.model().id(), state.doc.revision())) {
            return Err("the part changed while the table picker was open".into());
        }
        if let Some(reason) = busy(state) {
            return Err(reason.into());
        }
        let [(name, bytes)] = files.as_slice() else { return Err("choose one configuration table".into()) };
        let extension = name.rsplit_once('.').map_or("", |(_, extension)| extension).to_ascii_lowercase();
        let format = Format::parse(&extension)?;
        let table = crate::grid::exchange::decode(format, bytes)?;
        let report = crate::grid::preview::preview(state, &table)?;
        state.grid.bind(&report);
        apply_import(state)
    })();
    state.values.table = None;
    state.values.message = Some(import_message(result));
}

fn export(state: &mut AppState, format: Format) -> Option<String> {
    let extension = format.extension();
    let path = crate::platform::save_path(
        "Export configuration table",
        "Configuration table",
        extension,
        &format!("configurations.{extension}"),
    )?;
    Some(match crate::grid::tools::export_file(state, &path, format) {
        Ok(()) => format!("Exported {}", path.display()),
        Err(e) => e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formulas_show_with_an_equals_sign_and_numbers_without() {
        assert_eq!(shown("140"), "140");
        assert_eq!(shown("152.4 mm"), "152.4 mm");
        assert_eq!(shown("plate_w / 2"), "=plate_w / 2");
        assert_eq!(formula(" =Width*2 "), Some("=Width*2".to_owned()));
        assert_eq!(formula("  "), None);
        assert_eq!(number(12.5), "12.5");
        assert_eq!(number(10.0), "10");
    }

    #[test]
    fn refusals_read_as_plain_words() {
        assert_eq!(plain("unknown value Wdth"), "Nothing is named Wdth");
        assert_eq!(plain("Narrow: value cycle"), "This formula depends on itself");
        assert_eq!(plain("division by zero"), "This divides by zero");
        assert_eq!(plain("the input needs a length"), "This needs a length, such as 10 or 10 mm");
    }
}
