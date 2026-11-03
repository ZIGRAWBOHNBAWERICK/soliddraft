//! Named values and dimension bindings stored with the part's truth.
//!
//! Globals are named expressions. Every number typed into a feature is a parameter: it
//! has a default name, such as `BossExtrude1.Depth`, and a formula may read it or drive
//! it. One graph holds both, so a dimension can feed a global and a global a dimension.
//! A parameter's name is stored only once something uses it or the user renames it;
//! until then it follows its feature's name.

use serde::{Deserialize, Serialize};
use soliddraft_sketch::ConstraintId;

use crate::params::{self, Parameter, Unit};
use crate::value_expr::{
    self, Expr, Expression, NamedValue, Quantity, check_name, check_parameter_name, evaluate_tree, rename_tokens,
};
use crate::{Document, Feature, FeatureId, FeatureKind};

const MAX_DEPENDENCY_DEPTH: usize = 32;

/// A driven feature input, identified independently of its display name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Input {
    /// A sketch constraint with a numeric dimension.
    Dimension {
        /// Stable sketch identity.
        feature: FeatureId,
        /// Stable dimension identity.
        constraint: ConstraintId,
    },
    /// An extrusion's depth in millimetres.
    Depth {
        /// Stable extrusion identity.
        feature: FeatureId,
    },
    /// Another number a feature stores, such as a fillet's radius or a pattern's count.
    Parameter {
        /// Stable feature identity.
        feature: FeatureId,
        /// Which number.
        parameter: Parameter,
    },
}

impl Input {
    /// The feature that owns this input.
    pub fn feature(&self) -> FeatureId {
        match *self {
            Self::Dimension { feature, .. } | Self::Depth { feature } | Self::Parameter { feature, .. } => feature,
        }
    }
}

/// Authored values and bindings. Empty state is omitted from old-format truth.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Values {
    entries: Vec<NamedValue>,
    bindings: Vec<Binding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tracked: Vec<crate::tracked::Row>,
    /// Parameter names that no longer follow their feature's name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    names: Vec<Pin>,
    /// The stored numbers the expressions read, from the last check against features.
    #[serde(skip)]
    known: Known,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    input: Input,
    expression: Expression,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    input: Input,
    name: String,
}

/// Stored numbers by input. A cache: it never makes two states differ.
#[derive(Clone, Default)]
struct Known(Vec<(Input, Quantity, Unit)>);

impl PartialEq for Known {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for Known {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Known({})", self.0.len())
    }
}

/// One node of the value graph.
#[derive(Clone, PartialEq)]
enum Node {
    Value(u32),
    Input(Input),
}

/// One parameter as the values table lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterName {
    /// Stable identity.
    pub input: Input,
    /// The name formulas use.
    pub name: String,
    /// The label people read, such as `Depth` or `D2`.
    pub label: String,
    /// What it measures.
    pub unit: Unit,
    /// The stored number, before any formula drives it.
    pub stored: f64,
}

/// Resolves the graph, once per node.
struct Resolver<'a> {
    values: &'a Values,
    features: Option<&'a [Feature]>,
    done: Vec<(Node, Quantity, usize)>,
    visiting: Vec<Node>,
}

impl Resolver<'_> {
    fn stored(&self, input: &Input) -> Result<(Quantity, Unit), String> {
        let found = match self.features {
            Some(features) => params::read(features, input).map(|(value, unit)| {
                let (length, angle) = unit.dimensions();
                (Quantity { value, length, angle }, unit)
            }),
            None => self.values.known.0.iter().find(|(i, ..)| i == input).map(|(_, q, u)| (*q, *u)),
        };
        found.ok_or_else(|| match self.values.pinned(input) {
            Some(name) => format!("{name} belongs to a feature that is gone"),
            None => "a value belongs to a feature that is gone".into(),
        })
    }

    fn get(&mut self, node: &Node) -> Result<(Quantity, usize), String> {
        if let Some((_, q, depth)) = self.done.iter().find(|(n, ..)| n == node) {
            return Ok((*q, *depth));
        }
        if self.visiting.contains(node) {
            return Err("value cycle".into());
        }
        if self.visiting.len() >= MAX_DEPENDENCY_DEPTH {
            return Err("value graph is too deep".into());
        }
        let (expression, unit) = match node {
            Node::Value(id) => {
                let v = self.values.entries.iter().find(|v| v.id == *id).ok_or("missing value reference")?;
                (Some(&v.expression), None)
            }
            Node::Input(input) => {
                let (stored, unit) = self.stored(input)?;
                match self.values.bindings.iter().find(|b| b.input == *input) {
                    Some(b) => (Some(&b.expression), Some(unit)),
                    None => {
                        self.done.push((node.clone(), stored, 1));
                        return Ok((stored, 1));
                    }
                }
            }
        };
        let expression = expression.expect("set above");
        self.visiting.push(node.clone());
        let mut depth = 1;
        let mut q = evaluate_tree(&expression.tree, &mut |atom| {
            let next = match atom {
                Expr::Ref(id) => Node::Value(*id),
                Expr::Param(input) => Node::Input(input.clone()),
                _ => return Err("missing value reference".into()),
            };
            let (quantity, dependency_depth) = self.get(&next)?;
            depth = depth.max(dependency_depth + 1);
            Ok(quantity)
        })?;
        self.visiting.pop();
        if depth > MAX_DEPENDENCY_DEPTH {
            return Err("value graph is too deep".into());
        }
        if let Some(unit) = unit {
            q = coerce(q, unit, expression.is_bare_literal())?;
        }
        self.done.push((node.clone(), q, depth));
        Ok((q, depth))
    }

    /// The stored numbers this resolution read, for later reads without features.
    fn known(&self) -> Known {
        let mut out = Vec::new();
        for (node, ..) in &self.done {
            if let Node::Input(input) = node
                && let Ok((q, unit)) = self.stored(input)
            {
                out.push((input.clone(), q, unit));
            }
        }
        Known(out)
    }
}

/// A driven input's quantity in its own unit. A bare number takes the input's unit; a
/// count or a plain number also takes a number in document units.
fn coerce(mut q: Quantity, unit: Unit, bare: bool) -> Result<Quantity, String> {
    let wanted = unit.dimensions();
    if bare || matches!(unit, Unit::Count | Unit::Number) && (q.length, q.angle) == (1, 0) {
        (q.length, q.angle) = wanted;
    }
    if (q.length, q.angle) != wanted {
        return Err(match unit {
            Unit::Length => "the input needs a length".into(),
            Unit::Angle => "the input needs an angle".into(),
            Unit::Count => "the input needs a count".into(),
            Unit::Number => "the input needs a plain number".into(),
        });
    }
    Ok(q)
}

impl Values {
    /// Whether there is no value data to persist.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.bindings.is_empty() && self.tracked.is_empty() && self.names.is_empty()
    }

    fn pinned(&self, input: &Input) -> Option<&str> {
        self.names.iter().find(|p| p.input == *input).map(|p| p.name.as_str())
    }

    fn is_global(&self, name: &str) -> bool {
        self.entries.iter().any(|v| v.name == name)
    }

    /// Whether `name` is a global, as opposed to a parameter or nothing.
    pub fn has_global(&self, name: &str) -> bool {
        self.is_global(name)
    }

    /// Every parameter of `features`, in tree order, with the name formulas use.
    pub fn parameter_names(&self, features: &[Feature]) -> Vec<ParameterName> {
        let mut taken: Vec<String> = self
            .entries
            .iter()
            .map(|v| v.name.clone())
            .chain(self.tracked.iter().map(|r| r.name.clone()))
            .chain(self.names.iter().map(|p| p.name.clone()))
            .collect();
        let mut out = Vec::new();
        for feature in features {
            let part = params::name_part(feature);
            for p in params::parameters(feature) {
                let name = match self.pinned(&p.input) {
                    Some(name) => name.to_owned(),
                    None => {
                        let base = format!("{part}.{}", p.label);
                        let name = std::iter::once(base.clone())
                            .chain((2..).map(|n| format!("{base}_{n}")))
                            .find(|n| !taken.contains(n))
                            .expect("an unused name");
                        taken.push(name.clone());
                        name
                    }
                };
                out.push(ParameterName { input: p.input, name, label: p.label, unit: p.unit, stored: p.value });
            }
        }
        out
    }

    /// The parameter called `name`, pinning its name so later renames of its feature keep
    /// what refers to it.
    fn parameter(&mut self, name: &str, features: &[Feature]) -> Option<Input> {
        if let Some(pin) = self.names.iter().find(|p| p.name == name) {
            return Some(pin.input.clone());
        }
        let found = self.parameter_names(features).into_iter().find(|p| p.name == name)?;
        self.names.push(Pin { input: found.input.clone(), name: name.to_owned() });
        Some(found.input)
    }

    /// Pins every parameter `text` names.
    fn pin_references(&mut self, text: &str, features: &[Feature]) {
        for name in value_expr::names(text) {
            if !self.is_global(&name) && !self.tracked.iter().any(|r| r.name == name) {
                self.parameter(&name, features);
            }
        }
    }

    /// Pins the parameters a cell's own name and text name, before an overlay reads them.
    pub(crate) fn pin_cell(&mut self, name: &str, text: &str, features: &[Feature]) {
        if !self.is_global(name) {
            self.parameter(name, features);
        }
        self.pin_references(text, features);
    }

    pub(crate) fn set(&mut self, name: &str, text: &str, features: &[Feature]) -> Result<(), String> {
        check_name(name)?;
        if self.tracked.iter().any(|r| r.name == name) {
            return Err(format!("{name} is tracked and cannot be set"));
        }
        if self.names.iter().any(|p| p.name == name) {
            return Err(format!("the name {name} is already taken"));
        }
        if !self.is_global(name) {
            if self.entries.len() + self.tracked.len() >= 256 {
                return Err("at most 256 named values are supported".into());
            }
            let id = self
                .entries
                .iter()
                .map(|v| v.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or("value identifiers exhausted")?;
            self.entries.push(NamedValue { id, name: name.into(), expression: Expression::zero() });
        }
        self.pin_references(text, features);
        let expression = self.parse(text)?;
        self.entries.iter_mut().find(|v| v.name == name).expect("inserted above").expression = expression;
        self.resolve_entries(Some(features))?;
        Ok(())
    }

    /// Removes a global that nothing reads.
    pub(crate) fn remove(&mut self, name: &str) -> Result<(), String> {
        let index =
            self.entries.iter().position(|v| v.name == name).ok_or_else(|| format!("no global named {name}"))?;
        let id = self.entries[index].id;
        let readers = self
            .entries
            .iter()
            .filter(|v| v.expression.references().contains(&id))
            .map(|v| v.name.clone())
            .chain(
                self.bindings
                    .iter()
                    .filter(|b| b.expression.references().contains(&id))
                    .map(|b| self.pinned(&b.input).map_or_else(|| "a feature's value".to_owned(), str::to_owned)),
            )
            .collect::<Vec<_>>();
        if let Some(reader) = readers.first() {
            return Err(format!("{name} is used by {reader}"));
        }
        self.entries.remove(index);
        Ok(())
    }

    pub(crate) fn expression(&self, name: &str) -> Option<&str> {
        if let Some(v) = self.entries.iter().find(|v| v.name == name) {
            return Some(v.expression.text.as_str());
        }
        let pin = self.names.iter().find(|p| p.name == name)?;
        self.bindings.iter().find(|b| b.input == pin.input).map(|b| b.expression.text.as_str())
    }

    /// This state with each override in place: a global's expression, or a parameter's
    /// driver. The overrides' parameters must be pinned already.
    pub(crate) fn overlay(
        &self,
        overrides: &std::collections::BTreeMap<String, String>,
        features: &[Feature],
    ) -> Result<Self, String> {
        let mut next = self.clone();
        for (name, text) in overrides {
            let expression = self.parse(text)?;
            if let Some(entry) = next.entries.iter_mut().find(|v| v.name == *name) {
                entry.expression = expression;
            } else if let Some(pin) = self.names.iter().find(|p| p.name == *name) {
                next.bind(pin.input.clone(), expression);
            } else {
                return Err(format!("unknown value {name}"));
            }
        }
        next.refresh(features)?;
        Ok(next)
    }

    /// Resolves every global against `features` and keeps the numbers it read.
    fn refresh(&mut self, features: &[Feature]) -> Result<(), String> {
        let mut resolver = Resolver { values: self, features: Some(features), done: Vec::new(), visiting: Vec::new() };
        for v in &self.entries {
            resolver.get(&Node::Value(v.id))?;
        }
        let known = resolver.known();
        self.known = known;
        Ok(())
    }

    fn bind(&mut self, input: Input, expression: Expression) {
        if let Some(binding) = self.bindings.iter_mut().find(|b| b.input == input) {
            binding.expression = expression;
        } else {
            self.bindings.push(Binding { input, expression });
        }
    }

    pub(crate) fn drive(&mut self, input: Input, text: &str, features: &[Feature]) -> Result<(), String> {
        self.pin_references(text, features);
        let expression = self.parse(text)?;
        self.bind(input, expression);
        Ok(())
    }

    /// Sets a cell of the file's Default column. A global takes the text as its
    /// expression. A parameter takes a plain number as its stored value, as editing the
    /// feature does, and anything else as its driver; with no text it keeps the number it
    /// has now and is no longer driven.
    pub(crate) fn set_default_cell(
        &mut self,
        name: &str,
        text: Option<&str>,
        features: &mut [Feature],
    ) -> Result<(), String> {
        if self.is_global(name) {
            let text = text.ok_or_else(|| format!("{name} needs a value; delete it to remove it"))?;
            return self.set(name, text, features);
        }
        let input = self.parameter(name, features).ok_or_else(|| format!("unknown value {name}"))?;
        let value = match text {
            Some(text) => {
                self.pin_references(text, features);
                let expression = self.parse(text)?;
                if !expression.is_literal() {
                    self.bind(input, expression);
                    return Ok(());
                }
                self.literal(&input, &expression, features)?
            }
            None => {
                let mut resolver =
                    Resolver { values: self, features: Some(features), done: Vec::new(), visiting: Vec::new() };
                resolver.get(&Node::Input(input.clone()))?.0.value
            }
        };
        self.bindings.retain(|b| b.input != input);
        let feature = features.iter_mut().find(|f| f.id == input.feature()).ok_or("the feature is gone")?;
        params::write(feature, &input, value)
    }

    /// What a literal comes to as `input`'s number.
    fn literal(&self, input: &Input, expression: &Expression, features: &[Feature]) -> Result<f64, String> {
        let (_, unit) = params::read(features, input).ok_or("the feature is gone")?;
        let q = evaluate_tree(&expression.tree, &mut |_| Err("missing value".into()))?;
        Ok(coerce(q, unit, expression.is_bare_literal())?.value)
    }

    pub(crate) fn rename(&mut self, name: &str, to: &str, features: &[Feature]) -> Result<(), String> {
        let taken = |values: &Values, input: Option<&Input>| {
            values.is_global(to)
                || values.tracked.iter().any(|r| r.name == to)
                || values.parameter_names(features).iter().any(|p| p.name == to && Some(&p.input) != input)
        };
        if let Some(row) = self.tracked.iter().position(|r| r.name == name) {
            check_name(to)?;
            if to != name && taken(self, None) {
                return Err(format!("the name {to} is already taken"));
            }
            self.tracked[row].name = to.into();
            return Ok(());
        }
        if let Some(index) = self.entries.iter().position(|v| v.name == name) {
            check_name(to)?;
            if to != name && taken(self, None) {
                return Err(format!("the name {to} is already taken"));
            }
            self.entries[index].name = to.into();
        } else {
            check_parameter_name(to)?;
            let input = self.parameter(name, features).ok_or_else(|| format!("no value named {name}"))?;
            if to != name && taken(self, Some(&input)) {
                return Err(format!("the name {to} is already taken"));
            }
            self.names.iter_mut().find(|p| p.input == input).expect("pinned above").name = to.into();
        }
        for expression in self
            .entries
            .iter_mut()
            .map(|v| &mut v.expression)
            .chain(self.bindings.iter_mut().map(|b| &mut b.expression))
        {
            expression.text = rename_tokens(&expression.text, name, to);
        }
        Ok(())
    }

    fn lookup(&self, token: &str) -> Result<Expr, String> {
        if self.tracked.iter().any(|r| r.name == token) {
            return Err(format!("{token} is tracked and cannot drive an expression"));
        }
        if let Some(v) = self.entries.iter().find(|v| v.name == token) {
            return Ok(Expr::Ref(v.id));
        }
        if let Some(pin) = self.names.iter().find(|p| p.name == token) {
            return Ok(Expr::Param(pin.input.clone()));
        }
        Err(format!("unknown value {token}"))
    }

    fn parse(&self, text: &str) -> Result<Expression, String> {
        value_expr::parse(text, &|token| self.lookup(token)).map(Expression::with_converted_literal)
    }

    /// Every global's quantity, reading stored numbers from `features` or, without
    /// them, from the last check.
    fn resolve_entries(&self, features: Option<&[Feature]>) -> Result<Vec<Quantity>, String> {
        let mut resolver = Resolver { values: self, features, done: Vec::new(), visiting: Vec::new() };
        self.entries.iter().map(|v| resolver.get(&Node::Value(v.id)).map(|(q, _)| q)).collect()
    }

    /// Globals in creation order, with resolved document units.
    pub fn summary(&self) -> serde_json::Value {
        let resolved = self.resolve_entries(None).ok();
        self.entries
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let q = resolved.as_ref().map(|r| r[i]);
                serde_json::json!({"name":v.name,"expression":v.expression.text,"value":q.map(|q| q.value),
                "unit": q.map_or("", value_expr::unit_name)})
            })
            .collect()
    }

    /// Each global's and each parameter's value and unit symbol, in `features`, by name.
    pub(crate) fn column(&self, features: &[Feature]) -> Result<Vec<(String, f64, String)>, String> {
        let mut resolver = Resolver { values: self, features: Some(features), done: Vec::new(), visiting: Vec::new() };
        let mut out = Vec::new();
        for v in &self.entries {
            let (q, _) = resolver.get(&Node::Value(v.id))?;
            out.push((v.name.clone(), q.value, value_expr::unit_name(q).to_owned()));
        }
        for p in self.parameter_names(features) {
            let (q, _) = resolver.get(&Node::Input(p.input.clone()))?;
            out.push((p.name, q.value, p.unit.symbol().to_owned()));
        }
        Ok(out)
    }

    /// The driver's text for `input`, if a formula drives it.
    pub fn driver(&self, input: &Input) -> Option<&str> {
        self.bindings.iter().find(|b| b.input == *input).map(|b| b.expression.text.as_str())
    }

    /// Expression text by the feature's input labels.
    pub fn expressions(&self, feature: &Feature) -> std::collections::BTreeMap<String, String> {
        self.bindings
            .iter()
            .filter(|b| b.input.feature() == feature.id)
            .filter_map(|b| {
                let key = match b.input {
                    Input::Depth { .. } => "depth".into(),
                    Input::Parameter { parameter, .. } => parameter.key(),
                    Input::Dimension { constraint, .. } => {
                        let FeatureKind::Sketch(sketch) = &feature.kind else {
                            return None;
                        };
                        (sketch
                            .sketch
                            .constraints
                            .iter()
                            .filter(|c| c.value().is_some())
                            .position(|c| c.id == constraint)?
                            + 1)
                        .to_string()
                    }
                };
                Some((key, b.expression.text.clone()))
            })
            .collect()
    }

    /// Checks persisted expressions and bindings, including their stable references, and
    /// returns `features` with every driven input set.
    pub(crate) fn validate(&mut self, features: &[Feature]) -> Result<Vec<Feature>, String> {
        let mut ids = Vec::new();
        let mut names = Vec::new();
        for v in &self.entries {
            check_name(&v.name)?;
            if ids.contains(&v.id) || names.contains(&v.name.as_str()) {
                return Err("duplicate value id or name".into());
            }
            ids.push(v.id);
            names.push(v.name.as_str());
        }
        for row in &self.tracked {
            check_name(&row.name)?;
            if names.contains(&row.name.as_str()) {
                return Err(format!("duplicate value name {}", row.name));
            }
            names.push(&row.name);
            row.source.validate()?;
            if let Some(check) = &row.check
                && crate::tracked::Check::parse(&check.text, row.source.unit())? != *check
            {
                return Err("check text and resolved limit disagree".into());
            }
        }
        if names.len() > 256 {
            return Err("too many named values".into());
        }
        let mut pinned = Vec::new();
        for pin in &self.names {
            check_parameter_name(&pin.name)?;
            if names.contains(&pin.name.as_str()) || pinned.contains(&&pin.input) {
                return Err(format!("duplicate value name {}", pin.name));
            }
            names.push(&pin.name);
            pinned.push(&pin.input);
        }
        let mut inputs = Vec::new();
        for b in &self.bindings {
            if inputs.contains(&b.input) {
                return Err("an input has two drivers".into());
            }
            inputs.push(b.input.clone());
        }
        for e in self.entries.iter().map(|v| &v.expression).chain(self.bindings.iter().map(|b| &b.expression)) {
            if self.parse(&e.text)?.tree != e.tree {
                return Err("expression text and references disagree".into());
            }
        }
        let mut resolver = Resolver { values: self, features: Some(features), done: Vec::new(), visiting: Vec::new() };
        for v in &self.entries {
            resolver.get(&Node::Value(v.id))?;
        }
        let mut next = features.to_vec();
        for b in &self.bindings {
            let (q, _) = resolver.get(&Node::Input(b.input.clone()))?;
            let f = next.iter_mut().find(|f| f.id == b.input.feature()).ok_or("driven feature is missing")?;
            params::write(f, &b.input, q.value)?;
        }
        for (before, after) in features.iter().zip(&next) {
            if before.kind != after.kind {
                Document::check_kind(&after.kind).map_err(|e| e.to_string())?;
            }
        }
        let known = resolver.known();
        self.known = known;
        Ok(next)
    }
}

impl Document {
    /// The document's named values and bindings.
    pub fn values(&self) -> &Values {
        self.configurations.lookup(&self.values)
    }
}

/// Parses a single check limit using the value parser's units and decimal conversion.
pub(crate) fn parse_limit(text: &str, unit: &str) -> Result<f64, String> {
    let expression = Values::default().parse(text)?;
    if !expression.is_literal() {
        return Err("a check needs one literal limit".into());
    }
    let q = value_expr::evaluate(&expression, &std::collections::BTreeMap::new())?;
    let (dimension, name) = match unit {
        "mm" => ((1, 0), "a length"),
        "deg" => ((0, 1), "an angle"),
        _ => ((2, 0), "a area"),
    };
    if !expression.is_bare_literal() && (q.length, q.angle) != dimension {
        return Err(format!("the check needs {name} limit"));
    }
    Ok(q.value)
}
