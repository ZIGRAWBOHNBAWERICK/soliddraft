//! Sparse named-value and text-property overrides, resolved from one persisted feature base.

mod properties;
mod table;

pub use properties::Properties;
pub(crate) use properties::check as check_properties;
pub use table::{DEFAULT_ROW, Grid, GridRefusal, GridRow, GridValue};

use crate::command::Applied;
use crate::{Command, CommandError, Document, Feature, FeatureId, values::Values};
use serde::{Deserialize, Serialize};
use soliddraft_document::{ConfigurationId, DocumentId};
use std::collections::{BTreeMap, BTreeSet};

/// Configuration rows and their common sketch starting state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configurations {
    rows: Vec<Row>,
    active: Option<ConfigurationId>,
    base: Vec<Feature>,
    #[serde(skip)]
    resolved: Option<Values>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    id: ConfigurationId,
    name: String,
    overrides: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    properties: Properties,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    suppression: BTreeMap<FeatureId, bool>,
}

/// An edit from the parameters and configurations view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Edit {
    /// Rename a named configuration without changing its identity.
    Rename {
        /// Current tab name.
        configuration: String,
        /// New tab name.
        name: String,
    },
    /// Copy a configuration and its overrides under a new identity.
    Duplicate {
        /// Source tab, including Default.
        configuration: String,
        /// Name of the copy.
        name: String,
    },
    /// Delete a named configuration. Default is always present.
    Delete {
        /// Tab to delete.
        configuration: String,
    },
    /// Move a tab among the named configurations.
    Move {
        /// Tab to move.
        configuration: String,
        /// Number of positions to move. Negative means left.
        offset: i8,
    },
    /// Set an explicit suppression state, or let the feature inherit again.
    Suppress {
        /// Tab to edit, including Default.
        configuration: String,
        /// Feature in the tree.
        feature: FeatureId,
        /// Explicit flag, or None to use Default.
        suppressed: Option<bool>,
    },
    /// Set or clear a parameter's comment.
    Comment {
        /// Parameter name.
        name: String,
        /// Comment text. An empty string clears the comment.
        text: String,
    },
    /// Set or clear a configuration's custom property.
    Property {
        /// Tab to edit, including Default.
        configuration: String,
        /// Property name.
        name: String,
        /// Property text. An empty string clears the authored value.
        text: String,
    },
    /// Use degrees for trigonometric functions.
    Degrees(bool),
}

impl Configurations {
    /// Whether the document has only its implicit Default row.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn uses_formula_settings(&self) -> bool {
        self.rows
            .iter()
            .any(|row| !row.suppression.is_empty() || row.overrides.values().any(|text| text.starts_with('=')))
    }

    pub(crate) fn lookup<'a>(&'a self, defaults: &'a Values) -> &'a Values {
        self.resolved.as_ref().unwrap_or(defaults)
    }

    /// The definitions rows resolve from: the common base, or the tree without rows.
    fn base_or<'a>(&'a self, features: &'a [Feature]) -> &'a [Feature] {
        if self.is_empty() { features } else { &self.base }
    }

    fn row(&self, name: &str) -> Result<usize, String> {
        self.rows.iter().position(|r| r.name == name).ok_or_else(|| format!("unknown configuration {name}"))
    }

    pub(crate) fn validate(&mut self, defaults: &mut Values, features: &[Feature]) -> Result<Vec<Feature>, String> {
        if self.is_empty() {
            if self.active.is_some() || !self.base.is_empty() {
                return Err("configuration state has no rows".into());
            }
            self.resolved = None;
            return defaults.validate(features);
        }
        if self.rows.len() > 256 {
            return Err("at most 256 configurations are supported".into());
        }
        if self.base.len() != features.len()
            || self.base.iter().zip(features).any(|(a, b)| {
                a.id != b.id || a.body != b.body || a.name != b.name || a.via_assistant != b.via_assistant
            })
        {
            return Err("configuration base disagrees with the feature tree".into());
        }
        for feature in &self.base {
            Document::check_kind(&feature.kind).map_err(|e| e.to_string())?;
        }
        Document::check_order(&self.base).map_err(|e| e.to_string())?;
        Document::check_field_last(&self.base).map_err(|e| e.to_string())?;
        defaults.validate(&self.base)?;
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut active_values = None;
        for row in &self.rows {
            check_name(&row.name)?;
            properties::check(&row.properties).map_err(|e| format!("{}: {e}", row.name))?;
            if row.id.0.is_empty() || !ids.insert(&row.id) || !names.insert(&row.name) {
                return Err("duplicate configuration id or name".into());
            }
            if row.suppression.keys().any(|id| !self.base.iter().any(|feature| feature.id == *id)) {
                return Err(format!("{}: suppression names a missing feature", row.name));
            }
            let mut values = defaults.overlay(&row.overrides, &self.base).map_err(|e| format!("{}: {e}", row.name))?;
            values.validate(&self.base).map_err(|e| format!("{}: {e}", row.name))?;
            if self.active.as_ref() == Some(&row.id) {
                active_values = Some(values);
            }
        }
        if self.active.is_some() && active_values.is_none() {
            return Err("unknown active configuration".into());
        }
        let mut features = match active_values.as_mut() {
            Some(values) => values.validate(&self.base)?,
            None => defaults.validate(&self.base)?,
        };
        if let Some(row) = self.rows.iter().find(|r| Some(&r.id) == self.active.as_ref()) {
            for feature in &mut features {
                if let Some(suppressed) = row.suppression.get(&feature.id) {
                    feature.suppressed = *suppressed;
                }
            }
        }
        self.resolved = active_values;
        Ok(features)
    }

    /// Drops every row's override of a parameter that no longer exists. Refuses while a
    /// row's override text still reads one.
    pub(crate) fn forget_names(&mut self, names: &[String]) -> Result<(), String> {
        for row in &mut self.rows {
            for (key, text) in &row.overrides {
                if let Some(name) = crate::value_expr::names(text).into_iter().find(|n| names.contains(n)) {
                    return Err(format!("{key} in {} reads {name}; change it before removing the dimension", row.name));
                }
            }
            row.overrides.retain(|key, _| !names.contains(key));
        }
        Ok(())
    }

    pub(crate) fn capture_edits(&mut self, before: &[Feature], after: &[Feature]) -> bool {
        if self.is_empty() {
            return false;
        }
        let next: Vec<_> = after
            .iter()
            .map(|f| {
                let mut base = f.clone();
                if let Some(previous) = before.iter().find(|b| b.id == f.id)
                    && let Some(old) = self.base.iter().find(|b| b.id == f.id)
                {
                    if previous.kind == f.kind {
                        base.kind = old.kind.clone();
                    }
                    if previous.suppressed == f.suppressed {
                        base.suppressed = old.suppressed;
                    }
                }
                base
            })
            .collect();
        let changed = next != self.base;
        self.base = next;
        for row in &mut self.rows {
            row.suppression.retain(|id, _| self.base.iter().any(|feature| feature.id == *id));
        }
        changed
    }
}

impl Configurations {
    /// Sets one override in row `i`, in canonical text.
    fn set_override(
        &mut self,
        i: usize,
        defaults: &mut Values,
        name: &str,
        text: &str,
        features: &[Feature],
    ) -> Result<(), String> {
        defaults.pin_cell(name, text, features);
        self.rows[i].overrides.insert(name.to_owned(), text.to_owned());
        let layer = defaults.overlay(&self.rows[i].overrides, &self.base)?;
        self.rows[i].overrides.insert(name.to_owned(), layer.expression(name).ok_or("unknown value")?.into());
        Ok(())
    }
}

fn check_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name.len() > 128 || name == "Default" {
        return Err("a configuration needs a nonempty name other than reserved Default".into());
    }
    Ok(())
}

impl Configurations {
    fn check_new_name(&self, name: &str, replacing: Option<usize>) -> Result<(), String> {
        check_name(name)?;
        if self.rows.iter().enumerate().any(|(index, row)| Some(index) != replacing && row.name == name) {
            return Err(format!("configuration {name} already exists"));
        }
        Ok(())
    }

    fn add(&mut self, name: &str, features: &[Feature], source: Option<Row>) -> Result<(), String> {
        self.check_new_name(name, None)?;
        if self.is_empty() {
            self.base = features.to_vec();
        }
        self.rows.push(Row {
            id: ConfigurationId(DocumentId::new().to_string()),
            name: name.into(),
            overrides: source.as_ref().map_or_else(BTreeMap::new, |row| row.overrides.clone()),
            properties: source.as_ref().map_or_else(Properties::new, |row| row.properties.clone()),
            suppression: source.map_or_else(BTreeMap::new, |row| row.suppression),
        });
        Ok(())
    }

    fn suppress(
        &mut self,
        configuration: &str,
        feature: FeatureId,
        suppressed: Option<bool>,
        features: &mut [Feature],
    ) -> Result<String, String> {
        let base = if self.is_empty() { features } else { &mut self.base };
        let default = base.iter_mut().find(|f| f.id == feature).ok_or("the feature is gone")?;
        let label = format!("edit {} suppression", default.name);
        if configuration == DEFAULT_ROW {
            default.suppressed = suppressed.unwrap_or(false);
        } else {
            let i = self.row(configuration)?;
            if let Some(suppressed) = suppressed {
                self.rows[i].suppression.insert(feature, suppressed);
            } else {
                self.rows[i].suppression.remove(&feature);
            }
        }
        Ok(label)
    }

    fn edit(
        &mut self,
        edit: &Edit,
        values: &mut Values,
        features: &mut Vec<Feature>,
        properties: &mut Properties,
    ) -> Result<String, String> {
        Ok(match edit {
            Edit::Rename { configuration, name } => {
                let i = self.row(configuration)?;
                self.check_new_name(name, Some(i))?;
                self.rows[i].name.clone_from(name);
                format!("rename configuration {configuration}")
            }
            Edit::Duplicate { configuration, name } => {
                let source =
                    if configuration == DEFAULT_ROW { None } else { Some(self.rows[self.row(configuration)?].clone()) };
                self.add(name, features, source)?;
                self.active = self.rows.last().map(|row| row.id.clone());
                format!("duplicate configuration {configuration}")
            }
            Edit::Delete { configuration } => {
                let i = self.row(configuration)?;
                let row = self.rows.remove(i);
                if self.active.as_ref() == Some(&row.id) {
                    self.active = None;
                }
                if self.is_empty() {
                    *features = std::mem::take(&mut self.base);
                }
                format!("delete configuration {configuration}")
            }
            Edit::Move { configuration, offset } => {
                let from = self.row(configuration)?;
                let to = from
                    .checked_add_signed(isize::from(*offset))
                    .filter(|i| *i < self.rows.len())
                    .ok_or("the configuration cannot move farther")?;
                let row = self.rows.remove(from);
                self.rows.insert(to, row);
                format!("move configuration {configuration}")
            }
            Edit::Suppress { configuration, feature, suppressed } => {
                self.suppress(configuration, *feature, *suppressed, features)?
            }
            Edit::Comment { name, text } => {
                values.set_comment(name, text, self.base_or(features))?;
                format!("edit {name} comment")
            }
            Edit::Property { configuration, name, text } => {
                properties::check_name(name)?;
                let cells = if configuration == DEFAULT_ROW {
                    properties
                } else {
                    let i = self.row(configuration)?;
                    &mut self.rows[i].properties
                };
                if text.is_empty() {
                    cells.remove(name);
                } else {
                    properties::check_text(name, text)?;
                    cells.insert(name.clone(), text.clone());
                }
                properties::check(cells)?;
                format!("edit {name} property")
            }
            Edit::Degrees(degrees) => {
                values.degrees = *degrees;
                "change formula angle units".into()
            }
        })
    }
}

pub(crate) fn handles(command: &Command, has_rows: bool) -> bool {
    matches!(
        command,
        Command::AddConfiguration { .. }
            | Command::SetConfigurationValue { .. }
            | Command::ActivateConfiguration { .. }
            | Command::EditParameters { .. }
            | Command::SetConfigurationGrid { .. }
            | Command::RestoreConfigurations { .. }
    ) || has_rows
        && matches!(
            command,
            Command::SetValue { .. }
                | Command::Drive { .. }
                | Command::RenameValue { .. }
                | Command::SetValueCell { .. }
                | Command::RemoveValue { .. }
                | Command::SetSuppressed { .. }
        )
}

impl Document {
    /// The selected row's stable identity; None denotes Default.
    pub fn active_configuration(&self) -> Option<&ConfigurationId> {
        self.configurations.active.as_ref()
    }

    /// Whether the parameter functions interpret plain angle arguments as degrees.
    pub fn formula_degrees(&self) -> bool {
        self.values.degrees
    }

    /// Default suppression and the named configuration's override for a feature.
    pub fn configuration_suppression(&self, configuration: &str, id: FeatureId) -> Option<(bool, Option<bool>)> {
        let state = &self.configurations;
        let default = state.base_or(&self.features).iter().find(|feature| feature.id == id)?.suppressed;
        let override_value =
            state.rows.iter().find(|row| row.name == configuration).and_then(|row| row.suppression.get(&id));
        Some((default, override_value.copied()))
    }

    /// Counts accepted changes to the table, undo and redo included. A change that
    /// leaves the geometry alone still gives dependants a new source revision.
    pub fn table_edits(&self) -> u64 {
        self.table_edits
    }

    /// Configuration names and authored overrides in creation order.
    pub fn configuration_summary(&self) -> serde_json::Value {
        let state = &self.configurations;
        let active =
            state.rows.iter().find(|r| Some(&r.id) == state.active.as_ref()).map_or("Default", |r| r.name.as_str());
        let mut rows = vec![serde_json::json!({"name":"Default", "overrides":{}})];
        rows.extend(state.rows.iter().map(|r| serde_json::json!({"name":r.name,"overrides":r.overrides})));
        serde_json::json!({"active":active,"rows":rows})
    }

    pub(crate) fn apply_configuration(&mut self, command: Command) -> Result<Applied, CommandError> {
        let old = Command::RestoreConfigurations {
            state: Box::new(self.configurations.clone()),
            values: self.values.clone(),
            features: self.features.clone(),
            properties: self.properties.clone(),
        };
        let mut state = self.configurations.clone();
        let mut defaults = self.values.clone();
        let mut features = self.features.clone();
        let mut file_properties = self.properties.clone();
        let result = (|| -> Result<String, String> {
            // Parameter names follow the common definitions, or the tree without rows.
            let base = state.base_or(&features).to_vec();
            let label = match &command {
                Command::AddConfiguration { name } => {
                    state.add(name, &features, None)?;
                    format!("add configuration {name}")
                }
                Command::EditParameters { edit } => {
                    state.edit(edit, &mut defaults, &mut features, &mut file_properties)?
                }
                Command::SetSuppressed { id, suppressed } => {
                    let configuration = state
                        .rows
                        .iter()
                        .find(|row| Some(&row.id) == state.active.as_ref())
                        .map_or(DEFAULT_ROW, |row| row.name.as_str())
                        .to_owned();
                    state.suppress(&configuration, *id, Some(*suppressed), &mut features)?
                }
                Command::SetConfigurationValue { configuration, name, expression } => {
                    let i = state.row(configuration)?;
                    state.set_override(i, &mut defaults, name, expression, &features)?;
                    format!("set {configuration} value {name}")
                }
                Command::SetValueCell { configuration, name, expression } if configuration != DEFAULT_ROW => {
                    let i = state.row(configuration)?;
                    match expression {
                        Some(text) => state.set_override(i, &mut defaults, name, text, &features)?,
                        None => {
                            state.rows[i].overrides.remove(name);
                        }
                    }
                    format!("set {configuration} value {name}")
                }
                Command::SetValueCell { name, expression, .. } => {
                    let mut base = state.base.clone();
                    defaults.set_default_cell(name, expression.as_deref(), &mut base)?;
                    state.base = base;
                    format!("set value {name}")
                }
                Command::RemoveValue { name } => {
                    if let Some(row) = state.rows.iter().find(|r| {
                        r.overrides
                            .iter()
                            .any(|(key, text)| key != name && crate::value_expr::names(text).contains(name))
                    }) {
                        return Err(format!("{name} is used in configuration {}", row.name));
                    }
                    defaults.remove(name)?;
                    for row in &mut state.rows {
                        row.overrides.remove(name);
                    }
                    format!("delete value {name}")
                }
                Command::ActivateConfiguration { name } => {
                    state.active = if name == "Default" { None } else { Some(state.rows[state.row(name)?].id.clone()) };
                    format!("activate configuration {name}")
                }
                Command::SetValue { name, expression } => {
                    if let Some(row) = state
                        .rows
                        .iter_mut()
                        .find(|r| Some(&r.id) == state.active.as_ref() && r.overrides.contains_key(name))
                    {
                        defaults.pin_cell(name, expression, &base);
                        row.overrides.insert(name.clone(), expression.clone());
                        let layer = defaults.overlay(&row.overrides, &base)?;
                        row.overrides.insert(name.clone(), layer.expression(name).ok_or("unknown value")?.into());
                    } else {
                        defaults.set(name, expression, &base)?;
                    }
                    format!("set value {name}")
                }
                Command::Drive { input, expression } => {
                    defaults.drive(input.clone(), expression, &base)?;
                    "drive input".into()
                }
                Command::RenameValue { name, to } => {
                    defaults.rename(name, to, &base)?;
                    for row in &mut state.rows {
                        if let Some(text) = row.overrides.remove(name) {
                            row.overrides.insert(to.clone(), text);
                        }
                        for text in row.overrides.values_mut() {
                            *text = crate::value_expr::rename_tokens(text, name, to);
                        }
                    }
                    format!("rename value {name}")
                }
                Command::SetConfigurationGrid { grid } => {
                    let grid = self.check_configuration_grid(grid).map_err(|e| e.to_string())?;
                    let default = grid.rows.first().ok_or("the table has no Default row")?;
                    for row in &grid.rows {
                        for (name, text) in &row.values {
                            defaults.pin_cell(name, text, &base);
                        }
                    }
                    defaults = defaults.overlay(&default.values, &base)?;
                    file_properties = default.properties.clone();
                    state.set_grid(&grid);
                    "edit configuration grid".into()
                }
                Command::RestoreConfigurations { state: saved, values, features: saved_features, properties } => {
                    state = *saved.clone();
                    defaults = values.clone();
                    features = saved_features.clone();
                    file_properties = properties.clone();
                    "restore configurations".into()
                }
                _ => unreachable!(),
            };
            features = state.validate(&mut defaults, &features)?;
            Ok(label)
        })();
        let label = result.map_err(CommandError::Invalid)?;
        if features.len() != self.features.len()
            || features.iter().zip(&self.features).any(|(a, b)| {
                a.id != b.id
                    || a.name != b.name
                    || a.body != b.body
                    || a.via_assistant != b.via_assistant
                    || std::mem::discriminant(&a.kind) != std::mem::discriminant(&b.kind)
            })
        {
            return Err(CommandError::Invalid("a configuration command cannot replace the feature tree".into()));
        }
        let first =
            self.features.iter().zip(&features).position(|(a, b)| a.kind != b.kind || a.suppressed != b.suppressed);
        self.configurations = state;
        self.values = defaults;
        self.features = features;
        self.properties = file_properties;
        self.table_edits += 1;
        if let Some(i) = first {
            self.touch(i);
        }
        let redo = Command::RestoreConfigurations {
            state: Box::new(self.configurations.clone()),
            values: self.values.clone(),
            features: self.features.clone(),
            properties: self.properties.clone(),
        };
        Ok(Applied { inverse: old, redo, created: None, label })
    }
}

/// One row of the values table.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueRow {
    /// The name formulas use.
    pub name: String,
    /// The label people read: a global's name, or a parameter's such as `Depth`.
    pub label: String,
    /// The unit symbol, empty for a count or a plain number.
    pub unit: String,
    /// The note shown beside the parameter in every configuration.
    pub comment: String,
    /// Whether each result is a spreadsheet boolean.
    pub logical: Vec<bool>,
    /// Why the expression could not be evaluated, if any, by configuration.
    pub errors: Vec<Option<String>>,
    /// The parameter's identity; none for a global.
    pub input: Option<crate::values::Input>,
    /// Authored text by column, Default first. A blank configuration cell inherits; a
    /// parameter's blank Default cell holds the number typed into its feature.
    pub cells: Vec<Option<String>>,
    /// What the row comes to in each column, when that column resolves.
    pub values: Vec<Option<f64>>,
}

/// A feature's parameters, in tree order.
#[derive(Clone, Debug, PartialEq)]
pub struct ValueGroup {
    /// The feature.
    pub feature: crate::FeatureId,
    /// Its tree name.
    pub name: String,
    /// Its parameters.
    pub rows: Vec<ValueRow>,
}

/// Globals and every feature's parameters, with a column per configuration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValuesTable {
    /// Column names: Default, then each configuration in creation order.
    pub columns: Vec<String>,
    /// The selected column.
    pub active: usize,
    /// User-defined globals, in creation order.
    pub globals: Vec<ValueRow>,
    /// Parameters by feature.
    pub groups: Vec<ValueGroup>,
}

impl Document {
    /// The values table: what the Values tab shows.
    pub fn values_table(&self) -> ValuesTable {
        let state = &self.configurations;
        let base = state.base_or(&self.features);
        let defaults = &self.values;
        let mut columns = vec![DEFAULT_ROW.to_owned()];
        columns.extend(state.rows.iter().map(|r| r.name.clone()));
        let active =
            state.active.as_ref().and_then(|id| state.rows.iter().position(|r| r.id == *id)).map_or(0, |i| i + 1);
        let physical = self.values().reads_physical_properties().then(|| self.parameter_physical_values());
        let measure = |column| if column == active { physical.as_ref() } else { None };
        let resolved: Vec<_> =
            std::iter::once(Some(defaults.column(base, measure(0))))
                .chain(state.rows.iter().enumerate().map(|(i, row)| {
                    defaults.overlay(&row.overrides, base).ok().map(|v| v.column(base, measure(i + 1)))
                }))
                .collect();
        let value = |column: usize, name: &str| {
            resolved[column].as_ref().and_then(|c| c.iter().find(|(n, ..)| n == name)).map(|(_, q, _)| q)
        };
        let unit = |name: &str| {
            resolved
                .iter()
                .flatten()
                .flat_map(|c| c.iter())
                .find(|(n, q, _)| n == name && q.is_ok())
                .map(|(.., u)| u.clone())
        };
        let row = |name: &str, label: String, input: Option<crate::values::Input>, default: Option<String>| {
            let mut cells = vec![default];
            cells.extend(state.rows.iter().map(|r| r.overrides.get(name).cloned()));
            ValueRow {
                name: name.to_owned(),
                label,
                unit: unit(name).unwrap_or_default(),
                comment: defaults.comment(name).to_owned(),
                input,
                cells,
                values: (0..columns.len())
                    .map(|c| value(c, name).and_then(|q| q.as_ref().ok()).map(|q| q.value))
                    .collect(),
                logical: (0..columns.len())
                    .map(|c| value(c, name).and_then(|q| q.as_ref().ok()).is_some_and(|q| q.logical))
                    .collect(),
                errors: (0..columns.len()).map(|c| value(c, name).and_then(|q| q.as_ref().err()).cloned()).collect(),
            }
        };
        let globals = defaults
            .summary()
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v["name"].as_str())
            .map(|name| row(name, name.to_owned(), None, defaults.expression(name).map(str::to_owned)))
            .collect();
        let parameters = defaults.parameter_names(base);
        let groups = base
            .iter()
            .filter_map(|f| {
                let rows: Vec<ValueRow> = parameters
                    .iter()
                    .filter(|p| p.input.feature() == f.id)
                    .map(|p| {
                        let mut r = row(
                            &p.name,
                            p.label.clone(),
                            Some(p.input.clone()),
                            defaults.driver(&p.input).map(str::to_owned),
                        );
                        r.unit = p.unit.symbol().to_owned();
                        r
                    })
                    .collect();
                (!rows.is_empty()).then(|| ValueGroup { feature: f.id, name: f.name.clone(), rows })
            })
            .collect();
        ValuesTable { columns, active, globals, groups }
    }
}
