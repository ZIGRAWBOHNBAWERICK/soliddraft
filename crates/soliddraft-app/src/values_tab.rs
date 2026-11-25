//! Parameters and configurations, shared by the dock and the expanded view.

use std::collections::{BTreeMap, BTreeSet};

use crate::config_tabs::{self, TabEvent};
use crate::document::Command;
use crate::grid::exchange::Format;
use crate::icons::{self, Icon};
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
    /// Copies a configuration at once, as a browser duplicates a tab, then opens the
    /// copy's name for editing.
    Duplicate(String),
    /// Makes a column the selected configuration.
    Activate(String),
    /// Edits the selected configuration or parameter.
    Edit(Edit),
    /// Puts a function into the formula last edited.
    InsertFunction(String),
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
    /// The open copy of the part the rows and drafts belong to.
    document: Option<soliddraft_document::DocumentId>,
    /// The open copy and revision a table file was asked for.
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
    /// A field to give the keyboard to when it next shows.
    focus: Option<Field>,
    /// The formula field edited last, for the Functions menu.
    last_formula: Option<LastFormula>,
    /// The field that had the keyboard in the last frame.
    editing: Option<Field>,
    /// A field whose edit Escape ended. egui reports the lost focus a frame after the
    /// Escape, so the Escape is kept until then.
    escaped: Option<Field>,
    /// Sections folded or unfolded from how they start.
    toggled: BTreeSet<String>,
    /// The tab's last message.
    pub message: Option<String>,
}

/// The formula field edited last and its text. A cell keeps its configuration by name:
/// its column moves when a tab moves.
#[derive(Clone, Debug)]
struct LastFormula {
    field: Field,
    configuration: Option<String>,
    text: String,
}

#[derive(Clone, Debug)]
enum TabName {
    Add,
    Rename(String),
}

impl ValuesTab {
    /// Whether a field of the tab had the keyboard in the last frame. egui drops the focus
    /// on Escape before the shortcuts run, so they ask this to leave that Escape to the field.
    pub fn has_keyboard(&self) -> bool {
        self.editing.is_some()
    }

    /// The refusal shown on a field, if any.
    pub fn error(&self, field: &Field) -> Option<&str> {
        self.errors.get(field).map(String::as_str)
    }
}

/// The part's table, made again only when the part changes. Rows, drafts and refusals
/// belong to one open copy of the part: a second tab of the same file starts clean.
fn table(state: &mut AppState) -> ValuesTable {
    let document = state.doc.instance();
    if state.values.document != Some(document) {
        state.values.table = None;
        state.values.drafts.clear();
        state.values.errors.clear();
        state.values.formula_errors.clear();
        state.values.config_name = None;
        // A formula typed in another part is not this part's to finish.
        state.values.last_formula = None;
        state.values.focus = None;
        state.values.reveal = None;
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

/// A number as an editable cell shows it: the 15 significant digits Excel keeps.
fn number(v: f64) -> String {
    general_with(v, 15, 15)
}

/// A number in Excel's General format with up to `digits` significant digits: plain
/// decimals between 1E-9 and 1E11, scientific notation with six digits outside them.
fn general(v: f64, digits: i32) -> String {
    general_with(v, digits, 6)
}

/// General format with `scientific` significant digits in scientific notation.
fn general_with(v: f64, digits: i32, scientific: usize) -> String {
    if v == 0.0 || !v.is_finite() {
        return if v == 0.0 { "0".into() } else { v.to_string() };
    }
    let exponent = v.abs().log10().floor() as i32;
    if !(-9..11).contains(&exponent) {
        let decimals = scientific.clamp(1, 17) - 1;
        let text = format!("{v:.decimals$E}");
        let (mantissa, power) = text.split_once('E').unwrap_or((&text, "0"));
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        let power: i32 = power.parse().unwrap_or(0);
        return format!("{mantissa}E{}{:02}", if power < 0 { '-' } else { '+' }, power.abs());
    }
    let decimals = (digits - 1 - exponent).clamp(0, 20) as usize;
    let text = format!("{v:.decimals$}");
    if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_owned() } else { text }
}

/// Stored text as a cell shows it: a formula starts with `=`.
fn shown(text: &str) -> String {
    if plain_number(text) || text.starts_with('=') { text.to_owned() } else { format!("={text}") }
}

/// A value with its unit, for a hover: every digit Excel keeps.
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
    } else if message.contains("#N/A") {
        "No value is available (#N/A)".into()
    } else if message.ends_with("error value") {
        format!("The formula holds the error value {}", soliddraft_model::formula_error_code(message))
    } else if message.contains("#NUM!") {
        "This has no numeric answer, such as the square root of a negative number".into()
    } else if message.contains("not text") {
        "A parameter holds a number, not text in quotes".into()
    } else if message.contains("no cell ranges") || message.contains("no cell or sheet references") {
        "Name values in a formula, as =Width*2; there are no cells such as A1".into()
    } else if message.contains("not an array") {
        "A parameter holds one number, not an array in braces".into()
    } else if let Some(name) = message.strip_suffix(" has the wrong number of arguments") {
        match crate::formula_help::help(name) {
            Some(help) => format!("{name} takes {}", help.syntax),
            None => format!("{name} has the wrong number of arguments"),
        }
    } else if message.contains("unsupported character") {
        "There's a problem with this formula: check its characters".into()
    } else if message.contains("expected a value")
        || message.contains("missing )")
        || message.contains("unexpected token")
    {
        "There's a problem with this formula: it is not complete".into()
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
    field_response(ui, tab, key, text, width, frame).0
}

/// [`field`], with the field's response.
fn field_response(
    ui: &mut egui::Ui,
    tab: &mut ValuesTab,
    key: Field,
    text: Text<'_>,
    width: f32,
    frame: bool,
) -> (Option<String>, egui::Response) {
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
    let response = if frame { ui.add(edit) } else { theme::add_text_edit_with_hint(ui, edit, theme::muted()) };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, label));
    #[cfg(target_arch = "wasm32")]
    crate::web::show_visible_control(ui, label, response.rect);
    if tab.focus.as_ref() == Some(&key) {
        response.request_focus();
        tab.focus = None;
    }
    if response.gained_focus() {
        response.scroll_to_me(None);
    }
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let had_keyboard = tab.editing.as_ref() == Some(&key);
    if escape && (had_keyboard || response.has_focus() || response.lost_focus()) {
        tab.escaped = Some(key.clone());
    } else if response.has_focus() && tab.escaped.as_ref() == Some(&key) {
        // Escape closed a popup and the field kept the keyboard.
        tab.escaped = None;
    }
    if response.has_focus() {
        tab.editing = Some(key.clone());
    } else if had_keyboard {
        tab.editing = None;
    }
    let escaped = tab.escaped.as_ref() == Some(&key);
    let formula_field = matches!(key, Field::Cell(..) | Field::NewGlobalFormula);
    if formula_field && !escaped && response.has_focus() {
        remember_formula(tab, &key, &text);
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
        return (None, response);
    }
    if response.lost_focus() && escaped {
        tab.escaped = None;
        tab.drafts.remove(&key);
        tab.errors.remove(&key);
        tab.formula_errors.remove(&key);
        // The Functions menu must not finish a formula Escape threw away.
        if tab.last_formula.as_ref().is_some_and(|last| last.field == key) {
            tab.last_formula = None;
        }
        return (None, response);
    }
    if response.lost_focus() && tab.drafts.get(&key).is_some_and(|d| d == current) {
        tab.drafts.remove(&key);
        tab.errors.remove(&key);
        tab.formula_errors.remove(&key);
    }
    let commit = response.lost_focus() && tab.drafts.contains_key(&key);
    (commit.then_some(text), response)
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
                let help = name.strip_suffix('(').and_then(crate::formula_help::help);
                let option = ui.selectable_label(false, name);
                let option = match help {
                    Some(help) => option.on_hover_text(format!("{}: {}", help.syntax, help.about)),
                    None => option,
                };
                if option.clicked() {
                    chosen = Some(name.as_str());
                }
            }
        });
    }
    chosen.map(|name| format!("{}{name}", &text[..start]))
}

/// The table's column widths.
#[derive(Clone, Copy)]
struct Widths {
    name: f32,
    value: f32,
    comment: f32,
}

impl Widths {
    fn new(available: f32) -> Self {
        if available < 560.0 {
            Widths { name: 128.0, value: 104.0, comment: 96.0 }
        } else {
            let w = available.min(1600.0);
            Widths {
                name: (w * 0.24).clamp(150.0, 340.0),
                value: (w * 0.27).clamp(160.0, 400.0),
                comment: (w * 0.2).clamp(120.0, 320.0),
            }
        }
    }
}

/// The indent of a row under a section, and under a feature in a section.
const INDENT: f32 = 18.0;

fn parameter_row(
    ui: &mut egui::Ui,
    tab: &mut ValuesTab,
    row: &ValueRow,
    table: &ValuesTable,
    (widths, indent): (Widths, f32),
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
        ui.add_space(indent);
        name_cell(ui, tab, row, widths.name - indent, actions);
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
    let key = Field::Cell(row.name.clone(), column);
    ui.vertical(|ui| {
        let label = format!("{configuration} {}", row.name);
        let edit = |ui: &mut egui::Ui, tab: &mut ValuesTab| {
            field(ui, tab, key.clone(), Text { current: &current, hint: &hint, label: &label }, widths.value, true)
        };
        let commit = if differs {
            let marked = egui::Frame::new()
                .fill(theme::selection_bg())
                .corner_radius(3.0)
                .inner_margin(egui::Margin::same(2))
                .show(ui, |ui| edit(ui, tab));
            marked.response.on_hover_text(format!("Differs from {DEFAULT_ROW}"));
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
        let text = if row.logical[column] {
            if value == 0.0 { "FALSE".into() } else { "TRUE".into() }
        } else {
            general(value, 10)
        };
        ui.label(RichText::new(text).family(theme::medium())).on_hover_text(with_unit(value, &row.unit));
    } else {
        ui.label("-");
    }
    ui.label(RichText::new(&row.unit).color(theme::muted()));
    let label = format!("Comment {}", row.name);
    let comment = Field::Comment(row.name.clone());
    if let Some(text) =
        field(ui, tab, comment.clone(), Text { current: &row.comment, hint: "", label: &label }, widths.comment, true)
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

/// A section's header row in the table, which folds its rows away when clicked.
struct Section<'a> {
    key: &'a str,
    title: &'a str,
    label: &'a str,
    count: usize,
    indent: f32,
    open_by_default: bool,
}

/// Draws a section's header row and returns whether its rows show.
fn section(ui: &mut egui::Ui, toggled: &mut BTreeSet<String>, s: Section<'_>) -> bool {
    let open = s.open_by_default != toggled.contains(s.key);
    let size = if s.indent > 0.0 { 12.0 } else { 12.5 };
    let title = ui.painter().layout_no_wrap(s.title.to_owned(), theme::font_semibold(size), theme::ink());
    let count = ui.painter().layout_no_wrap(s.count.to_string(), theme::font_medium(11.0), theme::muted());
    let width = s.indent + 22.0 + title.size().x + 8.0 + count.size().x + 6.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 22.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, open, s.label));
    #[cfg(target_arch = "wasm32")]
    crate::web::show_visible_control(ui, s.label, rect);
    let painter = ui.painter();
    let inner = egui::Rect::from_min_max(egui::pos2(rect.left() + s.indent, rect.top()), rect.max);
    if response.hovered() || response.has_focus() {
        painter.rect_filled(inner, egui::CornerRadius::same(4), theme::hover());
    }
    let chevron = if open { Icon::ChevronDown } else { Icon::ChevronRight };
    let at = egui::pos2(inner.left() + 10.0, inner.center().y);
    icons::paint(painter, chevron, egui::Rect::from_center_size(at, egui::vec2(12.0, 12.0)), theme::muted());
    let title_at = egui::pos2(inner.left() + 22.0, inner.center().y - title.size().y / 2.0);
    let count_at = egui::pos2(title_at.x + title.size().x + 8.0, inner.center().y - count.size().y / 2.0);
    painter.galley(title_at, title, theme::ink());
    painter.galley(count_at, count, theme::muted());
    let response = response.on_hover_text(if open { "Click to fold" } else { "Click to unfold" });
    ui.end_row();
    if response.clicked() {
        if !toggled.remove(s.key) {
            toggled.insert(s.key.to_owned());
        }
        return !open;
    }
    open
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

/// Starts naming a tab: a new one, a copy or a rename, with the field focused.
fn start_naming(tab: &mut ValuesTab, mode: TabName, text: String) {
    tab.config_name = Some(mode);
    if text.is_empty() {
        tab.drafts.remove(&Field::NewConfiguration);
    } else {
        tab.drafts.insert(Field::NewConfiguration, text);
    }
    tab.errors.remove(&Field::NewConfiguration);
    tab.focus = Some(Field::NewConfiguration);
}

/// The configurations as browser tabs.
fn tabs(ui: &mut egui::Ui, tab: &mut ValuesTab, table: &ValuesTable, actions: &mut Vec<ValuesAction>) {
    let editing_at = match &tab.config_name {
        Some(TabName::Rename(name)) => table.columns.iter().position(|c| c == name),
        Some(_) => Some(table.columns.len()),
        None => None,
    };
    let (label, current) = match &tab.config_name {
        Some(TabName::Rename(name)) => ("Configuration name", name.clone()),
        _ => ("New configuration name", String::new()),
    };
    let mut draw = |ui: &mut egui::Ui| {
        let text = Text { current: &current, hint: "Name", label };
        let (entered, response) = field_response(ui, tab, Field::NewConfiguration, text, ui.available_width(), false);
        if entered.is_some() {
            actions.push(ValuesAction::AddConfiguration);
        } else if response.lost_focus() && !tab.errors.contains_key(&Field::NewConfiguration) {
            tab.config_name = None;
            tab.drafts.remove(&Field::NewConfiguration);
        }
    };
    let editing = editing_at.map(|at| config_tabs::Editing { at, draw: &mut draw });
    let events = config_tabs::show(ui, egui::Id::new("configuration_tabs"), &table.columns, table.active, editing);
    for event in events {
        let name = |i: usize| table.columns[i].clone();
        match event {
            TabEvent::Select(i) => actions.push(ValuesAction::Activate(name(i))),
            TabEvent::Close(i) => actions.push(ValuesAction::Edit(Edit::Delete { configuration: name(i) })),
            TabEvent::Rename(i) => start_naming(tab, TabName::Rename(name(i)), name(i)),
            TabEvent::Duplicate(i) => actions.push(ValuesAction::Duplicate(name(i))),
            TabEvent::New => start_naming(tab, TabName::Add, String::new()),
            TabEvent::Move { from, to } => {
                if let Some(edit) = move_tab(&table.columns, from, to) {
                    actions.push(ValuesAction::Edit(edit));
                }
            }
        }
    }
    error_line(ui, tab, &Field::NewConfiguration);
}

/// The edit that moves named tab `from` to place `to` among the named tabs.
fn move_tab(columns: &[String], from: usize, to: usize) -> Option<Edit> {
    let to = to.clamp(1, columns.len().saturating_sub(1).max(1));
    let offset = i32::try_from(to).ok()? - i32::try_from(from).ok()?;
    let configuration = columns.get(from).filter(|_| from > 0 && offset != 0)?.clone();
    Some(Edit::Move { configuration, offset })
}

/// The add row at the foot of the globals, as a spreadsheet's empty last row.
fn add_global(ui: &mut egui::Ui, tab: &mut ValuesTab, widths: Widths, actions: &mut Vec<ValuesAction>) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.add_space(INDENT);
            let text = Text { current: "", hint: "Add global variable", label: "New global name" };
            field(ui, tab, Field::NewGlobalName, text, widths.name - INDENT, true);
        });
        error_line(ui, tab, &Field::NewGlobalName);
    });
    let mut entered = false;
    ui.vertical(|ui| {
        let text = Text { current: "", hint: "Value or =formula", label: "New global formula" };
        entered = field(ui, tab, Field::NewGlobalFormula, text, widths.value, true).is_some()
            && ui.input(|i| i.key_pressed(egui::Key::Enter));
        error_line(ui, tab, &Field::NewGlobalFormula);
    });
    let add = tool(ui, Icon::Plus, "Add", "Add the global variable", true);
    add.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add global"));
    if add.clicked() || entered {
        actions.push(ValuesAction::AddGlobal);
    }
    ui.end_row();
}

/// A toolbar button: an icon and a word.
fn tool(ui: &mut egui::Ui, icon: Icon, text: &str, tip: &str, enabled: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::font_medium(12.0), egui::Color32::PLACEHOLDER);
    let size = egui::vec2(galley.size().x + 32.0, 26.0);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, text));
    #[cfg(target_arch = "wasm32")]
    crate::web::show_visible_control(ui, text, rect);
    let hot = enabled && (response.hovered() || response.has_focus());
    if hot {
        ui.painter().rect(
            rect,
            egui::CornerRadius::same(5),
            theme::hover(),
            egui::Stroke::new(1.0, theme::line()),
            egui::StrokeKind::Inside,
        );
    }
    let ink = if !enabled {
        theme::muted().gamma_multiply(0.45)
    } else if hot {
        theme::ink()
    } else {
        theme::muted()
    };
    let icon_rect =
        egui::Rect::from_center_size(egui::pos2(rect.left() + 14.0, rect.center().y), egui::vec2(14.0, 14.0));
    icons::paint(ui.painter(), icon, icon_rect, ink);
    let text_pos = egui::pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_pos, galley, if enabled { theme::ink() } else { ink });
    response.on_hover_text(tip)
}

/// Notes the formula field being edited and the configuration a cell belongs to.
fn remember_formula(tab: &mut ValuesTab, key: &Field, text: &str) {
    let configuration = match key {
        Field::Cell(_, column) => tab.table.as_ref().and_then(|(_, t)| t.columns.get(*column).cloned()),
        _ => None,
    };
    tab.last_formula = Some(LastFormula { field: key.clone(), configuration, text: text.to_owned() });
}

/// Puts a function into the formula last edited, as Excel's Insert Function does. A cell
/// is found again by its configuration; one whose configuration is gone gives way to the
/// new global's formula.
fn insert_function(state: &mut AppState, name: &str) {
    let columns = table(state).columns;
    let tab = &mut state.values;
    let last = tab.last_formula.take().and_then(|last| {
        let field = match last.field {
            Field::Cell(row, _) => {
                Field::Cell(row, columns.iter().position(|c| Some(c) == last.configuration.as_ref())?)
            }
            field => field,
        };
        Some((field, last.text))
    });
    let (key, text) = last.unwrap_or((Field::NewGlobalFormula, String::new()));
    let text = text.trim_end();
    let text = if text.is_empty() || !text.starts_with('=') { format!("={name}(") } else { format!("{text}{name}(") };
    tab.drafts.insert(key.clone(), text.clone());
    remember_formula(tab, &key, &text);
    tab.focus = Some(key);
}

fn toolbar(ui: &mut egui::Ui, state: &mut AppState, actions: &mut Vec<ValuesAction>, history: &mut Option<AppAction>) {
    let (undo, redo) = (state.doc.undo_label().is_some(), state.doc.redo_label().is_some());
    let degrees = state.doc.model().formula_degrees();
    let tab = &mut state.values;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if tool(ui, Icon::Undo, "Undo", "Undo the last change", undo).clicked() {
            *history = Some(AppAction::Undo);
        }
        if tool(ui, Icon::Redo, "Redo", "Redo the change undone", redo).clicked() {
            *history = Some(AppAction::Redo);
        }
        ui.separator();
        let import = "Read the globals and configurations from a CSV, XLSX or JSON table";
        if tool(ui, Icon::Import, "Import table", import, true).clicked() {
            actions.push(ValuesAction::Import);
        }
        ui.menu_button("Export table", |ui| {
            for (name, format) in
                [("Export CSV", Format::Csv), ("Export XLSX", Format::Xlsx), ("Export JSON", Format::Json)]
            {
                if ui.button(name).clicked() {
                    actions.push(ValuesAction::Export(format));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Write the globals and configurations to a table file");
        ui.separator();
        ui.menu_button("fx Functions", |ui| {
            ui.label(RichText::new("Insert into the formula last edited").color(theme::muted()).size(11.0));
            for (group, functions) in crate::formula_help::GROUPS {
                ui.menu_button(*group, |ui| {
                    for help in *functions {
                        if ui.button(help.syntax).on_hover_text(help.about).clicked() {
                            actions.push(ValuesAction::InsertFunction(help.name.to_owned()));
                            ui.close();
                        }
                    }
                });
            }
        })
        .response
        .on_hover_text("Every function a formula can use, as in Excel");
        ui.separator();
        ui.label(RichText::new("Angles").color(theme::muted()));
        let tip = "How SIN, COS and TAN read a plain number, and what ASIN, ACOS and ATAN give. \
                   Excel uses radians. A number with deg, such as 30 deg, is always in degrees.";
        for (label, choice) in [("Radians", false), ("Degrees", true)] {
            if ui.selectable_label(degrees == choice, label).on_hover_text(tip).clicked() && degrees != choice {
                actions.push(ValuesAction::Edit(Edit::Degrees(choice)));
            }
        }
        ui.separator();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
        icons::paint(ui.painter(), Icon::Search, rect, theme::muted());
        let filter =
            ui.add(egui::TextEdit::singleline(&mut tab.filter).hint_text("Filter parameters").desired_width(140.0));
        filter.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Filter parameters"));
    });
}

fn contents(ui: &mut egui::Ui, state: &mut AppState) {
    let table = table(state);
    let mut actions = Vec::new();
    let mut history = None;
    let tab = &mut state.values;
    if tab.completions.is_empty() {
        tab.completions = completions(&table);
    }
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
        icons::paint(ui.painter(), Icon::Formula, rect, theme::accent());
        let title = if tab.expanded { 15.0 } else { 12.5 };
        ui.label(RichText::new("Parameters & configurations").family(theme::semibold()).size(title));
        let expand = if tab.expanded { "Back to dock" } else { "Expand" };
        let tip =
            if tab.expanded { "Put the table back in the side panel" } else { "Open the table in a large window" };
        let button = ui.small_button(expand).on_hover_text(tip);
        #[cfg(target_arch = "wasm32")]
        crate::web::show_control(expand, button.rect);
        if button.clicked() {
            tab.expanded = !tab.expanded;
        }
    });
    ui.add_space(2.0);
    tabs(ui, tab, &table, &mut actions);
    ui.add_space(4.0);
    toolbar(ui, state, &mut actions, &mut history);
    crate::display_states::show_part(ui, state);
    let model = state.doc.model();
    let grid = model.configuration_grid();
    let configuration = &table.columns[table.active];
    let tab = &mut state.values;
    if let Some(message) = &tab.message {
        ui.label(RichText::new(message).color(theme::muted()));
    }
    ui.add_space(4.0);
    let filter = tab.filter.trim().to_ascii_lowercase();
    let matches = |text: &str| filter.is_empty() || text.to_ascii_lowercase().contains(&filter);
    let show_row = |row: &ValueRow| {
        matches(&row.name)
            || matches(&row.label)
            || matches(&row.comment)
            || row.cells[table.active].as_deref().is_some_and(matches)
    };
    let widths = Widths::new(ui.available_width());
    let suppressible: Vec<_> = model
        .features()
        .iter()
        .filter_map(|f| model.configuration_suppression(configuration, f.id).map(|s| (f, s)))
        .collect();
    egui::ScrollArea::both().id_salt("parameters_scroll").auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("parameters_table").num_columns(5).striped(true).spacing([10.0, 5.0]).show(ui, |ui| {
            for title in ["Name", "Value / Formula", "Evaluates to", "Unit", "Comment"] {
                ui.label(RichText::new(title).family(theme::semibold()).color(theme::muted()));
            }
            ui.end_row();
            let globals = Section {
                key: "globals",
                title: "Global variables",
                label: "Global variables",
                count: table.globals.len(),
                indent: 0.0,
                open_by_default: true,
            };
            if section(ui, &mut tab.toggled, globals) {
                for row in table.globals.iter().filter(|row| show_row(row)) {
                    parameter_row(ui, tab, row, &table, (widths, INDENT), &mut actions);
                }
                add_global(ui, tab, widths, &mut actions);
            }
            let count = table.groups.iter().map(|group| group.rows.len()).sum();
            let parameters = Section {
                key: "parameters",
                title: "Feature parameters",
                label: "Feature parameters",
                count,
                indent: 0.0,
                open_by_default: true,
            };
            if count > 0 && section(ui, &mut tab.toggled, parameters) {
                for group in &table.groups {
                    if !matches(&group.name) && !group.rows.iter().any(show_row) {
                        continue;
                    }
                    let key = format!("group {}", group.name);
                    let label = format!("{} parameters", group.name);
                    let heading = Section {
                        key: &key,
                        title: &group.name,
                        label: &label,
                        count: group.rows.len(),
                        indent: INDENT,
                        open_by_default: true,
                    };
                    if section(ui, &mut tab.toggled, heading) {
                        for row in group.rows.iter().filter(|row| matches(&group.name) || show_row(row)) {
                            parameter_row(ui, tab, row, &table, (widths, 2.0 * INDENT), &mut actions);
                        }
                    }
                }
            }
            let suppression = Section {
                key: "suppression",
                title: "Feature suppression",
                label: "Feature suppression",
                count: suppressible.len(),
                indent: 0.0,
                open_by_default: false,
            };
            if section(ui, &mut tab.toggled, suppression) {
                for (feature, (default, override_value)) in &suppressible {
                    ui.horizontal(|ui| {
                        ui.add_space(INDENT);
                        let mut suppressed = override_value.unwrap_or(*default);
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
                    });
                    if override_value.is_some() {
                        if ui.small_button(format!("Use {DEFAULT_ROW}")).clicked() {
                            actions.push(ValuesAction::Edit(Edit::Suppress {
                                configuration: configuration.clone(),
                                feature: feature.id,
                                suppressed: None,
                            }));
                        }
                    } else {
                        ui.label(RichText::new(format!("As {DEFAULT_ROW}")).color(theme::muted()));
                    }
                    ui.end_row();
                }
            }
            let custom = Section {
                key: "properties",
                title: "Custom properties",
                label: "Custom properties",
                count: grid.properties.len(),
                indent: 0.0,
                open_by_default: false,
            };
            if section(ui, &mut tab.toggled, custom) {
                properties(ui, tab, &grid, &table, widths, &mut actions);
            }
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
    widths: Widths,
    actions: &mut Vec<ValuesAction>,
) {
    let c = table.active;
    for property in &grid.properties {
        ui.horizontal(|ui| {
            ui.add_space(INDENT);
            ui.label(property);
        });
        let authored = grid.rows[c].properties.get(property).map_or("", String::as_str);
        let default = grid.rows[0].properties.get(property).map_or("", String::as_str);
        let hint = if c == 0 { "" } else { default };
        let key = Field::Property(property.clone(), c);
        let label = format!("{} property {property}", table.columns[c]);
        ui.vertical(|ui| {
            if let Some(text) =
                field(ui, tab, key.clone(), Text { current: authored, hint, label: &label }, widths.value, true)
            {
                actions.push(ValuesAction::Commit(key.clone(), text));
            }
            error_line(ui, tab, &key);
        });
        ui.end_row();
    }
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.add_space(INDENT);
            let text = Text { current: "", hint: "Add property", label: "New property name" };
            field(ui, tab, Field::NewPropertyName, text, widths.name - INDENT, true);
        });
        error_line(ui, tab, &Field::NewPropertyName);
    });
    ui.vertical(|ui| {
        let text = Text { current: "", hint: "Text", label: "New property text" };
        field(ui, tab, Field::NewPropertyText, text, widths.value, true);
        error_line(ui, tab, &Field::NewPropertyText);
    });
    let add = tool(ui, Icon::Plus, "Add", "Add the custom property", true);
    add.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add property"));
    if add.clicked() {
        actions.push(ValuesAction::AddProperty);
    }
    ui.end_row();
}

/// A refusal without the configuration it names first. The part reports a configuration
/// that fails as `name: cause`, and a name is free text: `#N/A` or `division by zero` would
/// otherwise read as the formula's error.
fn cause<'a>(message: &'a str, columns: &[String]) -> &'a str {
    columns
        .iter()
        .skip(1)
        .filter_map(|name| message.strip_prefix(name.as_str())?.strip_prefix(": "))
        .min_by_key(|rest| rest.len())
        .unwrap_or(message)
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
    let columns = table(state).columns;
    match state.doc.apply(command) {
        Ok(_) => {
            state.values.drafts.remove(&key);
            state.values.errors.remove(&key);
            state.values.formula_errors.remove(&key);
            state.values.table = None;
            true
        }
        Err(e) => {
            let e = cause(&e, &columns);
            if matches!(&key, Field::Cell(..) | Field::NewGlobalFormula) {
                state.values.formula_errors.insert(key.clone(), soliddraft_model::formula_error_code(e));
            }
            state.values.errors.insert(key.clone(), plain(e));
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
            let renamed = match &state.values.config_name {
                Some(TabName::Rename(configuration)) => Some((configuration.clone(), name.clone())),
                _ => None,
            };
            let command = match &state.values.config_name {
                Some(TabName::Rename(configuration)) => {
                    Command::EditParameters(Edit::Rename { configuration: configuration.clone(), name })
                }
                _ => Command::AddConfiguration { name },
            };
            if run(state, Field::NewConfiguration, command) {
                state.values.config_name = None;
                if let (Some((from, to)), Some(last)) = (renamed, state.values.last_formula.as_mut())
                    && last.configuration.as_ref() == Some(&from)
                {
                    last.configuration = Some(to);
                }
            }
        }
        ValuesAction::Duplicate(configuration) => {
            let columns = state.doc.model().values_table().columns;
            let name = (1..)
                .map(|n| if n == 1 { format!("{configuration} copy") } else { format!("{configuration} copy {n}") })
                .find(|name| !columns.contains(name))
                .expect("a free name");
            let command = Command::EditParameters(Edit::Duplicate { configuration, name: name.clone() });
            if run(state, Field::NewConfiguration, command) {
                start_naming(&mut state.values, TabName::Rename(name.clone()), name);
            }
        }
        ValuesAction::Edit(edit) => {
            let changed_order = matches!(edit, Edit::Move { .. } | Edit::Delete { .. });
            let deleted = match &edit {
                Edit::Delete { configuration } => Some(configuration.clone()),
                _ => None,
            };
            let columns = table(state).columns;
            state.values.message = if let Some(reason) = busy(state) {
                Some(reason.into())
            } else {
                match state.doc.apply(Command::EditParameters(edit)) {
                    Ok(_) => {
                        state.values.table = None;
                        if deleted.is_some()
                            && state.values.last_formula.as_ref().is_some_and(|last| last.configuration == deleted)
                        {
                            state.values.last_formula = None;
                        }
                        if changed_order {
                            let is_current = |field: &Field| !matches!(field, Field::Cell(..) | Field::Property(..));
                            state.values.drafts.retain(|key, _| is_current(key));
                            state.values.errors.retain(|key, _| is_current(key));
                            state.values.formula_errors.retain(|key, _| is_current(key));
                        }
                        None
                    }
                    Err(error) => Some(plain(cause(&error, &columns))),
                }
            };
        }
        ValuesAction::InsertFunction(name) => insert_function(state, &name),
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
    mark_import(state);
    let path = crate::platform::open_parameters(&["csv", "xlsx", "json"])?;
    state.values.import_request = None;
    let applied = crate::grid::tools::preview_file(state, &path).and_then(|_| apply_import(state));
    state.values.table = None;
    Some(import_message(applied))
}

/// Notes which open copy of the part, at which revision, asked for a table file: a file
/// that arrives later applies only to it.
fn mark_import(state: &mut AppState) {
    state.values.import_request = Some((state.doc.instance(), state.doc.revision()));
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
        if pending != Some((state.doc.instance(), state.doc.revision())) {
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
        assert_eq!(number(0.1 + 0.2), "0.3");
    }

    #[test]
    fn results_show_as_excel_general_format_shows_them() {
        for (value, shown) in [
            (12.0, "12"),
            (25.4, "25.4"),
            (1.0 / 3.0, "0.3333333333"),
            (-2.7755575615628914e-17, "-2.77556E-17"),
            (123_456_789_012.0, "1.23457E+11"),
            (12_345_678_901.0, "12345678901"),
            (0.000_012_5, "0.0000125"),
            (2_401_077.222_069_578, "2401077.222"),
        ] {
            assert_eq!(general(value, 10), shown, "{value}");
        }
        assert_eq!(with_unit(1.0 / 3.0, "mm"), "0.333333333333333 mm");
    }

    /// A hover and an editable cell keep every digit Excel keeps, in scientific notation
    /// too; only the cell's General display shortens it.
    #[test]
    fn hovers_and_cells_keep_fifteen_digits_in_scientific_notation() {
        assert_eq!(with_unit(1.23456789e-10, "mm"), "1.23456789E-10 mm");
        assert_eq!(with_unit(-2.7755575615628914e-17, ""), "-2.77555756156289E-17");
        assert_eq!(number(123_456_789_012_345.0), "1.23456789012345E+14");
        assert_eq!(general(1.23456789e-10, 10), "1.23457E-10");
    }

    /// A formula cell given text and then Escape applies nothing and forgets the text, so
    /// the Functions menu, which writes into the last formula, cannot finish it.
    #[test]
    fn escape_forgets_the_formula_it_threw_away() {
        let ctx = egui::Context::default();
        let mut tab = ValuesTab::default();
        let key = Field::Cell("BossExtrude1.Depth".into(), 0);
        let frame = |tab: &mut ValuesTab, events: Vec<egui::Event>| {
            let mut entered = None;
            let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                let text = Text { current: "10", hint: "", label: "Default BossExtrude1.Depth" };
                entered = field_response(ui, tab, key.clone(), text, 100.0, true).0;
            });
            output.textures_delta.clear();
            entered
        };
        tab.focus = Some(key.clone());
        frame(&mut tab, Vec::new());
        frame(&mut tab, vec![egui::Event::Text("=1234+".into())]);
        assert!(tab.drafts.get(&key).is_some_and(|text| text.ends_with("=1234+")), "{:?}", tab.drafts);
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(frame(&mut tab, vec![escape]), None, "Escape applies nothing");
        frame(&mut tab, Vec::new());
        assert_eq!(tab.drafts.get(&key), None, "Escape throws the text away");
        assert!(tab.last_formula.is_none(), "the Functions menu would finish the thrown-away formula");
    }

    #[test]
    fn the_functions_menu_writes_into_the_formula_last_edited() {
        let mut state = configured(&[]);
        insert_function(&mut state, "ROUND");
        assert_eq!(state.values.drafts.get(&Field::NewGlobalFormula).map(String::as_str), Some("=ROUND("));
        let cell = Field::Cell("Gap".into(), 0);
        remember_formula(&mut state.values, &cell, "=Width*2+");
        insert_function(&mut state, "PI");
        assert_eq!(state.values.drafts.get(&cell).map(String::as_str), Some("=Width*2+PI("));
        assert_eq!(state.values.focus, Some(cell));
    }

    /// A part with globals Gap and Bore and the named configurations.
    fn configured(names: &[&str]) -> AppState {
        let mut doc = crate::document::Document::new("Plate", crate::startup::kernel());
        doc.apply(Command::AddExtrude { params: Default::default(), via_assistant: false }).unwrap();
        for (name, expression) in [("Gap", "5"), ("Bore", "1")] {
            doc.apply(Command::SetValue { name: name.into(), expression: expression.into() }).unwrap();
        }
        for name in names {
            doc.apply(Command::AddConfiguration { name: (*name).into() }).unwrap();
        }
        let mut state = AppState::new(doc);
        table(&mut state);
        state
    }

    /// A Default formula that fails in one configuration is refused with that
    /// configuration's name in front; the cell shows the formula's own error.
    #[test]
    fn a_configuration_name_does_not_change_a_cell_s_error_code() {
        let mut state = configured(&["#N/A", "division by zero"]);
        commit(&mut state, Field::Cell("Gap".into(), 1), "0".into());
        commit(&mut state, Field::Cell("Gap".into(), 2), "-1".into());
        let bore = Field::Cell("Bore".into(), 0);
        for (formula, configuration, code, words) in [
            ("=1/Gap", "#N/A", "#DIV/0!", "This divides by zero"),
            (
                "=SQRTPI(Gap/1 mm)",
                "division by zero",
                "#NUM!",
                "This has no numeric answer, such as the square root of a negative number",
            ),
        ] {
            let command = Command::SetValueCell {
                configuration: DEFAULT_ROW.into(),
                name: "Bore".into(),
                expression: Some(formula.into()),
            };
            let refusal = state.doc.apply(command).unwrap_err();
            assert!(refusal.starts_with(&format!("{configuration}: ")), "{formula}: {refusal}");
            commit(&mut state, bore.clone(), formula.into());
            assert_eq!(state.values.formula_errors.get(&bore), Some(&code), "{formula}");
            assert_eq!(state.values.error(&bore), Some(words), "{formula}");
        }
    }

    #[test]
    fn the_functions_menu_keeps_to_its_cell_s_configuration_and_part() {
        let mut state = configured(&["A", "B"]);
        remember_formula(&mut state.values, &Field::Cell("Gap".into(), 1), "=Gap*2+");
        apply(&mut state, ValuesAction::Edit(Edit::Move { configuration: "B".into(), offset: -1 }));
        assert_eq!(table(&mut state).columns, ["Default", "B", "A"]);
        apply(&mut state, ValuesAction::InsertFunction("PI".into()));
        assert_eq!(state.values.drafts.get(&Field::Cell("Gap".into(), 2)).map(String::as_str), Some("=Gap*2+PI("));
        assert!(!state.values.drafts.contains_key(&Field::Cell("Gap".into(), 1)), "B's cell took A's formula");

        // A renamed tab keeps its cell; a deleted one gives way to the new global.
        state.values.config_name = Some(TabName::Rename("A".into()));
        state.values.drafts.insert(Field::NewConfiguration, "Wide".into());
        apply(&mut state, ValuesAction::AddConfiguration);
        assert_eq!(table(&mut state).columns, ["Default", "B", "Wide"]);
        state.values.drafts.clear();
        apply(&mut state, ValuesAction::InsertFunction("ROUND".into()));
        assert_eq!(
            state.values.drafts.get(&Field::Cell("Gap".into(), 2)).map(String::as_str),
            Some("=Gap*2+PI(ROUND(")
        );
        apply(&mut state, ValuesAction::Edit(Edit::Delete { configuration: "Wide".into() }));
        state.values.drafts.clear();
        apply(&mut state, ValuesAction::InsertFunction("PI".into()));
        assert_eq!(state.values.drafts.get(&Field::NewGlobalFormula).map(String::as_str), Some("=PI("));
        assert_eq!(state.values.drafts.len(), 1);

        // Another part's Values tab has no formula last edited.
        remember_formula(&mut state.values, &Field::Cell("Gap".into(), 0), "=Gap+");
        state.values.drafts.clear();
        state.doc = crate::document::Document::new("Other", crate::startup::kernel());
        apply(&mut state, ValuesAction::InsertFunction("ROUND".into()));
        assert_eq!(state.values.drafts.get(&Field::NewGlobalFormula).map(String::as_str), Some("=ROUND("));
        assert_eq!(state.values.drafts.len(), 1, "the other part's cell took the formula");
    }

    /// A tab dragged past 127 others lands where it was dropped.
    #[test]
    fn a_long_drag_moves_a_tab_all_the_way() {
        let columns: Vec<String> =
            std::iter::once(DEFAULT_ROW.to_owned()).chain((0..130).map(|i| format!("C{i}"))).collect();
        assert_eq!(move_tab(&columns, 1, 130), Some(Edit::Move { configuration: "C0".into(), offset: 129 }));
        assert_eq!(move_tab(&columns, 130, 1), Some(Edit::Move { configuration: "C129".into(), offset: -129 }));
        assert_eq!(move_tab(&columns, 3, 3), None);
        assert_eq!(move_tab(&columns, 0, 3), None, "Default stays first");
    }

    #[test]
    fn refusals_read_as_plain_words() {
        assert_eq!(plain("unknown value Wdth"), "Nothing is named Wdth");
        assert_eq!(plain("Narrow: value cycle"), "This formula depends on itself");
        assert_eq!(plain("division by zero"), "This divides by zero");
        assert_eq!(plain("the input needs a length"), "This needs a length, such as 10 or 10 mm");
        assert_eq!(plain("ROUND has the wrong number of arguments"), "ROUND takes ROUND(number, digits)");
        assert_eq!(plain("#N/A no value is available"), "No value is available (#N/A)");
    }
}
