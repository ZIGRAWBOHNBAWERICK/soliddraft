//! The Values tab, beside the feature tree in the same panel.
//!
//! Globals come first, then every number typed into a feature, grouped by feature in
//! tree order, with a column per configuration. A cell takes a number, a name or a
//! formula; pressing Enter or leaving the cell applies it as one undo step, and a refusal
//! stays on the cell in words while the part keeps its last good state.

use std::collections::BTreeMap;

use crate::document::Command;
use crate::grid::exchange::Format;
use crate::state::{AppAction, AppState};
use crate::theme;
use egui::RichText;
use soliddraft_model::configs::{DEFAULT_ROW, ValueRow, ValuesTable};

/// Which text field an edit belongs to.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    /// A global's or a parameter's cell in a column.
    Cell(String, usize),
    /// A global's or a parameter's name.
    Name(String),
    /// A text property's cell in a column.
    Property(String, usize),
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
    /// Set to bring the tab forward on the next frame.
    pub focus: bool,
    table: Option<(u64, ValuesTable)>,
    /// Typed text not yet applied, or refused.
    drafts: BTreeMap<Field, String>,
    /// Why a field's text was refused, in words.
    errors: BTreeMap<Field, String>,
    /// A refusal just made, to scroll into view once.
    reveal: Option<Field>,
    /// The tab's last message.
    pub message: Option<String>,
}

impl ValuesTab {
    /// The refusal shown on a field, if any.
    pub fn error(&self, field: &Field) -> Option<&str> {
        self.errors.get(field).map(String::as_str)
    }
}

/// The part's table, made again only when the part changes.
fn table(state: &mut AppState) -> ValuesTable {
    let revision = state.doc.revision();
    match &state.values.table {
        Some((r, t)) if *r == revision => t.clone(),
        _ => {
            let t = state.doc.model().values_table();
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
    if plain_number(text) { text.to_owned() } else { format!("={text}") }
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
        "Use numbers, names, units and + - * / ( ) only".into()
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
    let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if response.lost_focus() && escaped {
        tab.drafts.remove(&key);
        tab.errors.remove(&key);
        return None;
    }
    let commit = response.lost_focus() && tab.drafts.get(&key).is_some_and(|d| d != current);
    commit.then_some(text)
}

/// One row's cells, a column at a time.
fn row_cells(
    ui: &mut egui::Ui,
    tab: &mut ValuesTab,
    row: &ValueRow,
    columns: &[String],
    actions: &mut Vec<ValuesAction>,
) {
    for (c, column) in columns.iter().enumerate() {
        let authored = row.cells[c].as_deref();
        let value = row.values[c];
        let (current, hint) = match (authored, c) {
            (Some(text), _) => (shown(text), String::new()),
            (None, 0) => (value.map(number).unwrap_or_default(), String::new()),
            (None, _) => (String::new(), value.map(number).unwrap_or_default()),
        };
        let key = Field::Cell(row.name.clone(), c);
        ui.vertical(|ui| {
            let label = format!("{column} {}", row.name);
            if let Some(text) =
                field(ui, tab, key.clone(), Text { current: &current, hint: &hint, label: &label }, 76.0, true)
            {
                actions.push(ValuesAction::Commit(key.clone(), text));
            }
            error_line(ui, tab, &key);
        })
        .response
        .on_hover_text(match (value, authored) {
            (Some(v), Some(text)) if !plain_number(text) => format!("={text} comes to {}", with_unit(v, &row.unit)),
            (Some(v), None) if c > 0 => format!("Same as Default: {}", with_unit(v, &row.unit)),
            (Some(v), _) => with_unit(v, &row.unit),
            (None, _) => "Does not resolve in this configuration".into(),
        });
    }
}

/// A row's name, editable in place.
fn name_cell(ui: &mut egui::Ui, tab: &mut ValuesTab, row: &ValueRow, actions: &mut Vec<ValuesAction>) {
    let key = Field::Name(row.name.clone());
    ui.vertical(|ui| {
        let label = format!("Name {}", row.name);
        if let Some(text) =
            field(ui, tab, key.clone(), Text { current: &row.name, hint: "", label: &label }, 150.0, false)
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

/// The tab's content.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    if state.assembly.is_active() || state.drawing.is_active() {
        ui.add_space(8.0);
        ui.label(RichText::new("Values belong to the part. Switch to the part to see them.").color(theme::muted()));
        return;
    }
    let table = table(state);
    let grid = state.doc.model().configuration_grid();
    let mut actions = Vec::new();
    let tab = &mut state.values;
    egui::ScrollArea::both().id_salt("values_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(4.0);
        egui::Grid::new("values_table").num_columns(table.columns.len() + 2).spacing([6.0, 4.0]).show(ui, |ui| {
            ui.label(RichText::new("Name").color(theme::muted()));
            for (c, column) in table.columns.iter().enumerate() {
                let active = c == table.active;
                let text = RichText::new(column).family(if active { theme::semibold() } else { theme::medium() });
                let response = ui.selectable_label(active, text);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Column {column}"))
                });
                let response = response.on_hover_text(if active {
                    format!("{column} is the configuration the part shows")
                } else {
                    format!("Show the part as {column}")
                });
                if response.clicked() && !active {
                    actions.push(ValuesAction::Activate(column.clone()));
                }
            }
            ui.end_row();

            section(ui, "Globals");
            for row in &table.globals {
                name_cell(ui, tab, row, &mut actions);
                row_cells(ui, tab, row, &table.columns, &mut actions);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&row.unit).color(theme::muted()));
                    let delete = ui.small_button("×").on_hover_text(format!("Delete {}", row.name));
                    delete.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Delete {}", row.name))
                    });
                    if delete.clicked() {
                        actions.push(ValuesAction::RemoveGlobal(row.name.clone()));
                    }
                });
                ui.end_row();
            }
            field(
                ui,
                tab,
                Field::NewGlobalName,
                Text { current: "", hint: "New global", label: "New global name" },
                150.0,
                true,
            );
            let formula = field(
                ui,
                tab,
                Field::NewGlobalFormula,
                Text { current: "", hint: "Value or =formula", label: "New global formula" },
                76.0,
                true,
            );
            for _ in 1..table.columns.len() {
                ui.label("");
            }
            let add = ui.small_button("Add").on_hover_text("Add the global");
            add.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add global"));
            let entered = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if add.clicked() || formula.is_some() && entered {
                actions.push(ValuesAction::AddGlobal);
            }
            ui.end_row();
            for key in [Field::NewGlobalName, Field::NewGlobalFormula] {
                if let Some(error) = tab.errors.get(&key) {
                    ui.label(RichText::new(error).color(theme::error()).size(11.0));
                    ui.end_row();
                }
            }

            for group in &table.groups {
                section(ui, &group.name);
                for row in &group.rows {
                    name_cell(ui, tab, row, &mut actions);
                    row_cells(ui, tab, row, &table.columns, &mut actions);
                    ui.label(RichText::new(&row.unit).color(theme::muted()));
                    ui.end_row();
                }
            }

            if !grid.properties.is_empty() {
                section(ui, "Properties");
            }
            for property in &grid.properties {
                ui.label(property);
                for (c, column) in table.columns.iter().enumerate() {
                    let text = grid.rows.get(c).and_then(|r| r.properties.get(property)).cloned().unwrap_or_default();
                    let key = Field::Property(property.clone(), c);
                    let hint = if c == 0 { "" } else { "same as Default" };
                    ui.vertical(|ui| {
                        let label = format!("{column} property {property}");
                        if let Some(text) =
                            field(ui, tab, key.clone(), Text { current: &text, hint, label: &label }, 76.0, true)
                        {
                            actions.push(ValuesAction::Commit(key.clone(), text));
                        }
                        if let Some(error) = tab.errors.get(&key) {
                            ui.label(RichText::new(error).color(theme::error()).size(11.0));
                        }
                    });
                }
                ui.end_row();
            }
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let entered = field(
                ui,
                tab,
                Field::NewConfiguration,
                Text { current: "", hint: "New configuration", label: "New configuration name" },
                150.0,
                true,
            );
            let add = ui.button("Add configuration");
            if add.clicked() || entered.is_some() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                actions.push(ValuesAction::AddConfiguration);
            }
        });
        if let Some(error) = tab.errors.get(&Field::NewConfiguration) {
            ui.label(RichText::new(error).color(theme::error()).size(11.0));
        }
        ui.horizontal(|ui| {
            field(
                ui,
                tab,
                Field::NewPropertyName,
                Text { current: "", hint: "New property", label: "New property name" },
                150.0,
                true,
            );
            field(
                ui,
                tab,
                Field::NewPropertyText,
                Text { current: "", hint: "Text", label: "New property text" },
                76.0,
                true,
            );
            if ui.button("Add property").clicked() {
                actions.push(ValuesAction::AddProperty);
            }
        });
        for key in [Field::NewPropertyName, Field::NewPropertyText] {
            if let Some(error) = tab.errors.get(&key) {
                ui.label(RichText::new(error).color(theme::error()).size(11.0));
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("Import table").on_hover_text("Read globals and properties from a CSV or XLSX file").clicked()
            {
                actions.push(ValuesAction::Import);
            }
            if ui.button("Export CSV").clicked() {
                actions.push(ValuesAction::Export(Format::Csv));
            }
            if ui.button("Export XLSX").clicked() {
                actions.push(ValuesAction::Export(Format::Xlsx));
            }
        });
        if let Some(message) = &tab.message {
            ui.label(RichText::new(message).color(theme::muted()));
        }
        ui.add_space(4.0);
        ui.label(
            RichText::new("Type a number, a name, or a formula such as =Width*2. Enter applies it.")
                .color(theme::muted())
                .size(11.0),
        );
    });
    for action in actions {
        state.push(AppAction::Values(action));
    }
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
            state.values.table = None;
            true
        }
        Err(e) => {
            state.values.errors.insert(key.clone(), plain(&e));
            state.values.reveal = Some(key);
            false
        }
    }
}

/// A typed formula without its leading `=`; none when blank.
fn formula(text: &str) -> Option<String> {
    let text = text.trim();
    let text = text.strip_prefix('=').unwrap_or(text).trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn draft(state: &AppState, key: &Field) -> String {
    state.values.drafts.get(key).map(|t| t.trim().to_owned()).unwrap_or_default()
}

/// Runs what the tab asked for.
pub fn apply(state: &mut AppState, action: ValuesAction) {
    match action {
        ValuesAction::Show => {
            state.values.focus = true;
            state.layout.tree.shown = true;
        }
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
            run(state, Field::NewConfiguration, Command::AddConfiguration { name });
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
            if set_property(state, Field::NewPropertyText, &name, 0, &text) {
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
        // The new rows apply through their own buttons.
        _ => {}
    }
}

/// Sets one text property cell through the configuration table, as one undo step.
fn set_property(state: &mut AppState, key: Field, name: &str, column: usize, text: &str) -> bool {
    let mut grid = state.doc.model().configuration_grid();
    let Some(row) = grid.rows.get_mut(column) else { return false };
    if text.is_empty() {
        row.properties.remove(name);
    } else {
        row.properties.insert(name.to_owned(), text.to_owned());
    }
    match state.doc.model().check_configuration_grid(&grid) {
        Ok(grid) => run(state, key, Command::SetConfigurationGrid { grid: Box::new(grid) }),
        Err(e) => {
            state.values.errors.insert(key, plain(&e.reason));
            false
        }
    }
}

fn import(state: &mut AppState) -> Option<String> {
    // The browser's upload route opens documents; a table needs its own.
    if cfg!(target_arch = "wasm32") {
        return Some("Coming soon".to_owned());
    }
    let path = crate::platform::open_path("Import configuration table", "Configuration table", &["csv", "xlsx"])?;
    let applied = crate::grid::tools::preview_file(state, &path).and_then(|_| {
        let id = state.grid.shown().map(|p| p.id.clone()).ok_or("The file could not be read")?;
        crate::grid::preview::apply(state, &id)
    });
    state.values.table = None;
    Some(match applied {
        Ok(_) => "Imported as one undo step.".to_owned(),
        Err(e) => plain(&e),
    })
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
        assert_eq!(formula(" =Width*2 "), Some("Width*2".to_owned()));
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
