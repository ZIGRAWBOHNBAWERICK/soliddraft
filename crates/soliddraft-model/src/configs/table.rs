//! The configuration table as one authored candidate: named-value expressions and text
//! properties, by row. A blank cell is absent from its row and inherits.

use super::{Configurations, properties};
use crate::{Document, Feature, values::Values};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use soliddraft_document::ConfigurationId;
use std::collections::{BTreeMap, BTreeSet};

/// The Default row's reserved name.
pub const DEFAULT_ROW: &str = "Default";

/// One authored table: the file's Default row, then the named rows in creation order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    /// Named values in creation order.
    pub values: Vec<GridValue>,
    /// Text properties by name.
    pub properties: Vec<String>,
    /// Every row, Default first.
    pub rows: Vec<GridRow>,
}

/// A named-value column.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridValue {
    /// The existing value's name.
    pub name: String,
    /// Its resolved unit in the file's Default row.
    pub unit: String,
}

/// One row's authored cells.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GridRow {
    /// Stable identity; None for Default.
    pub id: Option<ConfigurationId>,
    /// Row name.
    pub name: String,
    /// Expressions by value name.
    pub values: BTreeMap<String, String>,
    /// Text by property name.
    pub properties: properties::Properties,
}

/// Why a candidate table was refused, with the cell when one is at fault.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{}{}{reason}", .row.as_deref().map(|r| format!("{r}: ")).unwrap_or_default(),
    .column.as_deref().map(|c| format!("{c}: ")).unwrap_or_default())]
pub struct GridRefusal {
    /// The row's name.
    pub row: Option<String>,
    /// The value or property's name.
    pub column: Option<String>,
    /// The reason in words.
    pub reason: String,
}

fn refusal(row: &str, column: Option<&str>, reason: impl Into<String>) -> GridRefusal {
    GridRefusal { row: Some(row.into()), column: column.map(str::to_owned), reason: reason.into() }
}

fn units(values: &Values) -> BTreeMap<String, String> {
    let summary = values.summary();
    let rows = summary.as_array().map(Vec::as_slice).unwrap_or_default();
    rows.iter().filter_map(|v| Some((v["name"].as_str()?.to_owned(), v["unit"].as_str()?.to_owned()))).collect()
}

/// A resolved unit in words, for a refusal.
fn unit_words(unit: &str) -> String {
    match unit {
        "mm" => "length (mm)".into(),
        "deg" => "angle (deg)".into(),
        "" => "number".into(),
        other => other.into(),
    }
}

/// Overlays one row's cells on `base`. When the row fails as a whole, a cell that fails
/// alone takes the blame; a fault across cells stays with the row.
fn overlay(
    base: &Values,
    row: &str,
    cells: &BTreeMap<String, String>,
    features: &[Feature],
) -> Result<Values, GridRefusal> {
    let mut base = base.clone();
    for (name, text) in cells {
        base.pin_cell(name, text, features);
    }
    base.overlay(cells, features).map_err(|whole| {
        cells
            .iter()
            .find_map(|(name, text)| {
                let one = BTreeMap::from([(name.clone(), text.clone())]);
                base.overlay(&one, features).err().map(|reason| refusal(row, Some(name), reason))
            })
            .unwrap_or_else(|| refusal(row, None, whole))
    })
}

fn check_units(layer: &Values, row: &GridRow, expected: &BTreeMap<String, String>) -> Result<(), GridRefusal> {
    let resolved = units(layer);
    for name in row.values.keys() {
        let (found, wanted) = (resolved.get(name), expected.get(name));
        if found != wanted {
            let found = found.map_or("nothing".into(), |u| unit_words(u));
            let wanted = wanted.map_or("nothing".into(), |u| unit_words(u));
            return Err(refusal(&row.name, Some(name), format!("expected {wanted}, found {found}")));
        }
    }
    Ok(())
}

fn canonical(layer: &Values, cells: &mut BTreeMap<String, String>) {
    for (name, text) in cells.iter_mut() {
        if let Some(stored) = layer.expression(name) {
            *text = stored.to_owned();
        }
    }
}

impl Configurations {
    /// The rows after Default, with their globals' overrides as authored. The table
    /// lists globals only; a parameter's override stays out of it.
    fn grid_rows<'a>(&'a self, values: &'a Values) -> impl Iterator<Item = GridRow> + 'a {
        self.rows.iter().map(|r| GridRow {
            id: Some(r.id.clone()),
            name: r.name.clone(),
            values: r
                .overrides
                .iter()
                .filter(|(k, _)| values.has_global(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            properties: r.properties.clone(),
        })
    }

    /// Replaces every row's cells, keeping its identity and place, and its parameters'
    /// overrides, which the table does not list.
    pub(super) fn set_grid(&mut self, grid: &Grid) {
        for row in &mut self.rows {
            if let Some(cells) = grid.rows.iter().find(|r| r.name == row.name) {
                row.overrides.retain(|name, _| !grid.values.iter().any(|v| v.name == *name));
                row.overrides.extend(cells.values.clone());
                row.properties = cells.properties.clone();
            }
        }
    }
}

impl Document {
    /// The accepted table.
    pub fn configuration_grid(&self) -> Grid {
        let mut names = BTreeSet::new();
        let unit = units(&self.values);
        let summary = self.values.summary();
        let order = summary.as_array().map(Vec::as_slice).unwrap_or_default();
        let values: Vec<GridValue> = order
            .iter()
            .filter_map(|v| v["name"].as_str())
            .map(|name| GridValue { name: name.to_owned(), unit: unit.get(name).cloned().unwrap_or_default() })
            .collect();
        let default_cells =
            values.iter().filter_map(|v| Some((v.name.clone(), self.values.expression(&v.name)?.to_owned()))).collect();
        let default_row =
            GridRow { id: None, name: DEFAULT_ROW.into(), values: default_cells, properties: self.properties.clone() };
        let rows: Vec<GridRow> =
            std::iter::once(default_row).chain(self.configurations.grid_rows(&self.values)).collect();
        for row in &rows {
            names.extend(row.properties.keys().cloned());
        }
        Grid { values, properties: names.into_iter().collect(), rows }
    }

    /// Checks a whole candidate against the accepted document, without changing it, and
    /// returns it in canonical form: unit literals converted, identities filled in.
    pub fn check_configuration_grid(&self, candidate: &Grid) -> Result<Grid, GridRefusal> {
        let live = self.configuration_grid();
        let general = |reason: String| GridRefusal { row: None, column: None, reason };
        let names = |grid: &Grid| grid.values.iter().map(|v| v.name.clone()).collect::<BTreeSet<_>>();
        if let Some(name) = names(candidate).difference(&names(&live)).next() {
            return Err(general(format!("unknown named value {name}")));
        }
        if let Some(name) = names(&live).difference(&names(candidate)).next() {
            return Err(general(format!("missing named value {name}")));
        }
        for value in &candidate.values {
            let wanted = live.values.iter().find(|v| v.name == value.name).map(|v| v.unit.as_str());
            if wanted != Some(value.unit.as_str()) {
                let wanted = unit_words(wanted.unwrap_or_default());
                let reason = format!("expected {wanted}, found unit {}", value.unit);
                return Err(GridRefusal { row: None, column: Some(value.name.clone()), reason });
            }
        }
        let mut seen = BTreeSet::new();
        for row in &candidate.rows {
            if !seen.insert(&row.name) {
                return Err(general(format!("duplicate configuration {}", row.name)));
            }
            if !live.rows.iter().any(|r| r.name == row.name) {
                return Err(general(format!("unknown configuration {}", row.name)));
            }
        }
        if let Some(row) = live.rows.iter().find(|r| !seen.contains(&r.name)) {
            return Err(general(format!("missing configuration {}", row.name)));
        }
        let mut grid = candidate.clone();
        grid.values = live.values.clone();
        grid.rows.sort_by_key(|row| live.rows.iter().position(|r| r.name == row.name));
        for (row, stable) in grid.rows.iter_mut().zip(&live.rows) {
            row.id = stable.id.clone();
            for (name, text) in &row.properties {
                properties::check_name(name).map_err(|reason| refusal(&row.name, Some(name), reason))?;
                properties::check_text(name, text).map_err(|reason| refusal(&row.name, Some(name), reason))?;
            }
        }
        let default = &mut grid.rows[0];
        if let Some(value) = live.values.iter().find(|v| !default.values.contains_key(&v.name)) {
            return Err(refusal(DEFAULT_ROW, Some(&value.name), "missing required cell"));
        }
        let base = self.configurations.base_or(&self.features);
        let defaults = overlay(&self.values, DEFAULT_ROW, &default.values, base)?;
        let expected: BTreeMap<_, _> = live.values.iter().map(|v| (v.name.clone(), v.unit.clone())).collect();
        check_units(&defaults, default, &expected)?;
        canonical(&defaults, &mut default.values);
        for row in grid.rows.iter_mut().skip(1) {
            let mut layer = overlay(&defaults, &row.name, &row.values, base)?;
            check_units(&layer, row, &expected)?;
            canonical(&layer, &mut row.values);
            // The row keeps its parameters' overrides, which the table does not list.
            if let Some(kept) = self.configurations.rows.iter().find(|r| r.name == row.name) {
                let hidden: BTreeMap<_, _> = kept
                    .overrides
                    .iter()
                    .filter(|(k, _)| !self.values.has_global(k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                if !hidden.is_empty() {
                    layer = layer.overlay(&hidden, base).map_err(|reason| refusal(&row.name, None, reason))?;
                }
            }
            layer.validate(base).map_err(|reason| refusal(&row.name, None, reason))?;
        }
        let used: BTreeSet<_> = grid.rows.iter().flat_map(|row| row.properties.keys().cloned()).collect();
        grid.properties = used.into_iter().collect();
        Ok(grid)
    }

    /// Each row's resolved named values and text properties, for `grid`. The table must
    /// have passed [`check_configuration_grid`](Self::check_configuration_grid).
    pub fn resolve_configuration_grid(&self, grid: &Grid) -> Value {
        let Some(default) = grid.rows.first() else { return json!([]) };
        let base = self.configurations.base_or(&self.features);
        let Ok(defaults) = self.values.overlay(&default.values, base) else { return json!([]) };
        grid.rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let layer = if i == 0 { Ok(defaults.clone()) } else { defaults.overlay(&row.values, base) };
                let mut properties = default.properties.clone();
                properties.extend(row.properties.clone());
                let inherited: Vec<_> = properties.keys().filter(|p| !row.properties.contains_key(*p)).collect();
                json!({
                    "name": row.name,
                    "values": layer.map(|l| l.summary()).unwrap_or(Value::Null),
                    "properties": properties,
                    "inherited_properties": inherited,
                })
            })
            .collect()
    }
}
