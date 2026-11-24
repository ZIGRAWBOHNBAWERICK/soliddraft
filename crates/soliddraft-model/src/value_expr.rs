//! Typed expressions over named values: the one grammar, unit model and evaluator the
//! part's values and an assembly's share.
//!
//! An expression keeps the text as written beside a tree that refers to other values by
//! stable id, so renaming a value changes its label and never what depends on it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod excel;

pub use excel::{FORMULA_FUNCTIONS, formula_error_code};

const MAX_DEPENDENCY_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NamedValue {
    pub(crate) id: u32,
    pub(crate) name: String,
    pub(crate) expression: Expression,
}

/// An expression: the text as stored, and what it means.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expression {
    pub(crate) text: String,
    pub(crate) tree: Expr,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Expr {
    Number(f64, i32, i32),
    Ref(u32),
    /// A feature parameter, by its stable identity.
    Param(crate::values::Input),
    Op(char, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    Formula(Box<Expr>),
    Binary(excel::Binary, Box<Expr>, Box<Expr>),
    Percent(Box<Expr>),
    Call(String, Vec<Expr>),
    Logical(bool),
    InvalidReference,
    Physical(PhysicalProperty),
    /// A function argument left empty, as in `=ROUND(2.5,)`: zero, as in a spreadsheet.
    Missing,
    /// An error value typed as a literal, such as `#N/A`, by its code.
    Error(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum PhysicalProperty {
    Mass,
    Volume,
}

#[derive(Clone, Copy)]
pub(crate) struct Quantity {
    pub(crate) value: f64,
    pub(crate) length: i32,
    pub(crate) angle: i32,
    pub(crate) mass: i32,
    pub(crate) logical: bool,
}

impl Quantity {
    pub(crate) fn number(value: f64) -> Self {
        Self { value, length: 0, angle: 0, mass: 0, logical: false }
    }

    fn logical(value: bool) -> Self {
        Self { logical: true, ..Self::number(if value { 1.0 } else { 0.0 }) }
    }

    pub(crate) fn is_unitless(&self) -> bool {
        self.dimensions() == (0, 0, 0)
    }

    fn dimensions(self) -> (i32, i32, i32) {
        (self.length, self.angle, self.mass)
    }

    fn document_units(mut self) -> Self {
        if self.is_unitless() && !self.logical {
            self.length = 1;
        }
        self
    }
}

/// Every value's quantity, or why they do not resolve.
pub(crate) fn resolve(entries: &[NamedValue]) -> Result<BTreeMap<u32, Quantity>, String> {
    fn visit(
        entries: &[NamedValue],
        id: u32,
        visiting: &mut Vec<u32>,
        done: &mut BTreeMap<u32, (Quantity, usize)>,
    ) -> Result<(Quantity, usize), String> {
        if let Some(cached) = done.get(&id) {
            return Ok(*cached);
        }
        if visiting.contains(&id) {
            return Err("value cycle".into());
        }
        if visiting.len() >= MAX_DEPENDENCY_DEPTH {
            return Err("value graph is too deep".into());
        }
        let v = entries.iter().find(|v| v.id == id).ok_or("missing value reference")?;
        visiting.push(id);
        let mut depth = 1;
        let q = evaluate_tree(&v.expression.tree, &mut |atom| {
            let Expr::Ref(other) = atom else { return Err("missing value reference".into()) };
            let (quantity, dependency_depth) = visit(entries, *other, visiting, done)?;
            depth = depth.max(dependency_depth + 1);
            Ok(quantity)
        })?;
        visiting.pop();
        if depth > MAX_DEPENDENCY_DEPTH {
            return Err("value graph is too deep".into());
        }
        done.insert(id, (q, depth));
        Ok((q, depth))
    }
    let mut done = BTreeMap::new();
    for v in entries {
        visit(entries, v.id, &mut Vec::new(), &mut done)?;
    }
    Ok(done.into_iter().map(|(id, (quantity, _))| (id, quantity)).collect())
}

fn bare_literal(tree: &Expr) -> bool {
    match tree {
        Expr::Number(_, 0, 0) => true,
        Expr::Neg(inner) => bare_literal(inner),
        _ => false,
    }
}

/// What an expression comes to, given the values it may read.
pub(crate) fn evaluate(expression: &Expression, resolved: &BTreeMap<u32, Quantity>) -> Result<Quantity, String> {
    evaluate_tree(&expression.tree, &mut |atom| match atom {
        Expr::Ref(id) => resolved.get(id).copied().ok_or("missing value".into()),
        _ => Err("missing value".into()),
    })
}

/// What a tree comes to; `reference` answers each [`Expr::Ref`] and [`Expr::Param`].
pub(crate) fn evaluate_tree(
    tree: &Expr,
    reference: &mut impl FnMut(&Expr) -> Result<Quantity, String>,
) -> Result<Quantity, String> {
    evaluate_tree_with_angle(tree, false, reference)
}

pub(crate) fn evaluate_tree_with_angle(
    tree: &Expr,
    degrees: bool,
    reference: &mut impl FnMut(&Expr) -> Result<Quantity, String>,
) -> Result<Quantity, String> {
    let q = eval(tree, degrees, reference)?;
    Ok(if bare_literal(tree) { q.document_units() } else { q })
}

fn eval(
    tree: &Expr,
    degrees: bool,
    reference: &mut impl FnMut(&Expr) -> Result<Quantity, String>,
) -> Result<Quantity, String> {
    let mut eval = |e: &Expr| eval(e, degrees, reference);
    let q = match tree {
        Expr::Number(value, length, angle) => {
            Quantity { value: *value, length: *length, angle: *angle, ..Quantity::number(0.0) }
        }
        Expr::Ref(_) | Expr::Param(_) | Expr::Physical(_) => reference(tree)?,
        Expr::Formula(inner) => match inner.as_ref() {
            Expr::Binary(op, a, b) => op.apply_last(a, b, &mut eval)?,
            other => eval(other)?,
        },
        Expr::Logical(value) => Quantity::logical(*value),
        Expr::InvalidReference => return Err("missing value reference".into()),
        Expr::Missing => Quantity::number(0.0),
        Expr::Error(code) => return Err(excel::error_value(code)),
        Expr::Binary(op, a, b) => op.apply(a, b, &mut eval)?,
        Expr::Call(name, args) => excel::call(name, args, degrees, &mut eval)?,
        Expr::Percent(e) => {
            let q = eval(e)?;
            Quantity { value: q.value / 100.0, logical: false, ..q }
        }
        Expr::Neg(e) => {
            let mut q = eval(e)?;
            q.value = -q.value;
            q.logical = false;
            q
        }
        Expr::Op(op, a, b) => {
            let a_bare = bare_literal(a);
            let b_bare = bare_literal(b);
            let mut a = eval(a)?;
            let mut b = eval(b)?;
            match op {
                '+' | '-' => {
                    // A bare number beside a plain count stays a count; elsewhere it is
                    // in document units.
                    if a_bare && !(b.is_unitless() && !b_bare) {
                        a = a.document_units();
                    }
                    if b_bare && !(a.is_unitless() && !a_bare) {
                        b = b.document_units();
                    }
                    if a.dimensions() != b.dimensions() {
                        return Err("incompatible units in sum".into());
                    }
                    a.value = if *op == '+' { a.value + b.value } else { a.value - b.value };
                    a.logical = false;
                    a
                }
                '*' => Quantity {
                    value: a.value * b.value,
                    length: a.length.checked_add(b.length).ok_or("unit exponent too large")?,
                    angle: a.angle.checked_add(b.angle).ok_or("unit exponent too large")?,
                    mass: a.mass.checked_add(b.mass).ok_or("unit exponent too large")?,
                    logical: false,
                },
                '/' if b.value == 0.0 => return Err("division by zero".into()),
                '/' => Quantity {
                    value: a.value / b.value,
                    length: a.length.checked_sub(b.length).ok_or("unit exponent too large")?,
                    angle: a.angle.checked_sub(b.angle).ok_or("unit exponent too large")?,
                    mass: a.mass.checked_sub(b.mass).ok_or("unit exponent too large")?,
                    logical: false,
                },
                _ => return Err("unknown operator".into()),
            }
        }
    };
    if !q.value.is_finite() {
        return Err("expression must resolve to a finite number".into());
    }
    Ok(q)
}

fn unit(text: &str) -> Option<(i128, i32, i32, i32)> {
    match text {
        "mm" => Some((1, 0, 1, 0)),
        "cm" => Some((10, 0, 1, 0)),
        "m" => Some((1000, 0, 1, 0)),
        "in" => Some((254, 1, 1, 0)),
        "deg" => Some((1, 0, 0, 1)),
        _ => None,
    }
}

pub(crate) fn check_name(name: &str) -> Result<(), String> {
    let mut bytes = name.bytes();
    if !bytes.next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || name.len() > 128
        || unit(name).is_some()
    {
        return Err(format!("{name}: a value name must be an identifier and cannot be a unit"));
    }
    Ok(())
}

fn tokenize(text: &str) -> Result<Vec<&str>, String> {
    let formula = text.starts_with('=');
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at].is_ascii_digit() || bytes[at] == b'.' {
            at += 1;
            while at < bytes.len() && (bytes[at].is_ascii_digit() || bytes[at] == b'.') {
                at += 1;
            }
            if at < bytes.len() && matches!(bytes[at], b'e' | b'E') {
                at += 1;
                if at < bytes.len() && matches!(bytes[at], b'+' | b'-') {
                    at += 1;
                }
                while at < bytes.len() && bytes[at].is_ascii_digit() {
                    at += 1;
                }
            }
        } else if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            // A parameter name joins identifiers with dots, as in Sketch1.D1.
            at += 1;
            while at < bytes.len()
                && (bytes[at].is_ascii_alphanumeric()
                    || bytes[at] == b'_'
                    || bytes[at] == b'.' && bytes.get(at + 1).is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_'))
            {
                at += 1;
            }
        } else if formula && bytes[at] == b'#' {
            at += excel::error_literal(&text[at..]).ok_or("unsupported character in expression")?.len();
        } else if formula && b"\"&:;{}[]'$!".contains(&bytes[at]) {
            return Err(excel::unsupported(bytes[at]).into());
        } else if formula && matches!(bytes[at], b'<' | b'>') {
            let op = bytes[at];
            at += 1;
            if bytes.get(at) == Some(&b'=') || op == b'<' && bytes.get(at) == Some(&b'>') {
                at += 1;
            }
        } else if b"+-*/()".contains(&bytes[at]) || formula && b"=^%,".contains(&bytes[at]) {
            at += 1;
        } else {
            return Err("unsupported character in expression".into());
        }
        out.push(&text[start..at]);
    }
    Ok(out)
}

pub(crate) fn rename_tokens<'a>(text: &str, replace: impl Fn(&str, bool) -> Option<&'a str>) -> String {
    let mut out = String::new();
    let mut at = 0;
    let tokens = tokenize(text).expect("accepted expression");
    let formula = text.starts_with('=');
    for (i, token) in tokens.iter().copied().enumerate() {
        let start = token.as_ptr() as usize - text.as_ptr() as usize;
        out.push_str(&text[at..start]);
        let name = unit(token).is_none() && (!formula || tokens.get(i + 1) != Some(&"("));
        out.push_str(if name { replace(token, formula).unwrap_or(token) } else { token });
        at = start + token.len();
    }
    out.push_str(&text[at..]);
    out
}

struct Parser<'a> {
    tokens: &'a [&'a str],
    at: usize,
    lookup: &'a dyn Fn(&str) -> Result<Expr, String>,
    formula: bool,
}
impl Parser<'_> {
    fn sum(&mut self) -> Result<Expr, String> {
        let mut a = self.product()?;
        while self.tokens.get(self.at).is_some_and(|t| matches!(*t, "+" | "-")) {
            let op = self.tokens[self.at].chars().next().expect("operator");
            self.at += 1;
            a = Expr::Op(op, Box::new(a), Box::new(self.product()?));
        }
        Ok(a)
    }
    fn product(&mut self) -> Result<Expr, String> {
        let mut a = self.atom()?;
        while self.tokens.get(self.at).is_some_and(|t| matches!(*t, "*" | "/")) {
            let op = self.tokens[self.at].chars().next().expect("operator");
            self.at += 1;
            a = Expr::Op(op, Box::new(a), Box::new(self.atom()?));
        }
        Ok(a)
    }
    fn atom(&mut self) -> Result<Expr, String> {
        let token = *self.tokens.get(self.at).ok_or("expected a value")?;
        self.at += 1;
        match token {
            "-" => Ok(Expr::Neg(Box::new(self.atom()?))),
            "+" => self.atom(),
            "(" => {
                let a = self.expression()?;
                if self.tokens.get(self.at) != Some(&")") {
                    return Err("missing )".into());
                }
                self.at += 1;
                Ok(a)
            }
            _ if self.formula && token.starts_with('#') => {
                let code = token.to_ascii_uppercase();
                Ok(if code == "#REF!" { Expr::InvalidReference } else { Expr::Error(code) })
            }
            _ => {
                if token.as_bytes()[0].is_ascii_digit() || token.starts_with('.') {
                    let value: f64 = token.parse().map_err(|_| "invalid numeric literal")?;
                    if !value.is_finite() {
                        return Err("a literal must be finite".into());
                    }
                    if let Some(symbol) = self.tokens.get(self.at).filter(|t| t.as_bytes()[0].is_ascii_alphabetic()) {
                        let (factor, scale, length, angle) =
                            unit(symbol).ok_or_else(|| format!("unknown unit {symbol}"))?;
                        self.at += 1;
                        return Ok(Expr::Number(convert_decimal(token, factor, scale)?, length, angle));
                    }
                    Ok(Expr::Number(value, 0, 0))
                } else {
                    if self.formula {
                        if !token.as_bytes()[0].is_ascii_alphabetic() && !token.starts_with('_') {
                            return Err("expected a value".into());
                        }
                        if self.tokens.get(self.at) == Some(&"(") {
                            return self.call(token);
                        }
                        if token.eq_ignore_ascii_case("TRUE") || token.eq_ignore_ascii_case("FALSE") {
                            return Ok(Expr::Logical(token.eq_ignore_ascii_case("TRUE")));
                        }
                    }
                    (self.lookup)(token)
                }
            }
        }
    }

    fn expression(&mut self) -> Result<Expr, String> {
        if self.formula { self.binary(0) } else { self.sum() }
    }
}

fn convert_decimal(text: &str, factor: i128, scale: i32) -> Result<f64, String> {
    let fail = || "unit literal is outside the supported decimal range".to_owned();
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let exponent: i32 = exponent.parse().map_err(|_| fail())?;
    let fraction = mantissa.split_once('.').map_or(0, |(_, tail)| tail.len() as i32);
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let n = digits.parse::<i128>().ok().and_then(|n| n.checked_mul(factor)).ok_or_else(fail)?;
    let exponent = exponent
        .checked_sub(fraction)
        .and_then(|e| e.checked_sub(scale))
        .filter(|e| (-300..=300).contains(e))
        .ok_or_else(fail)?;
    let value: f64 = format!("{n}e{exponent}").parse().map_err(|_| fail())?;
    if !value.is_finite() {
        return Err(fail());
    }
    Ok(value)
}

/// Parses `text`, reading names through `lookup`. The expression keeps the text as written.
pub(crate) fn parse(text: &str, lookup: &dyn Fn(&str) -> Result<Expr, String>) -> Result<Expression, String> {
    if text.len() > 2048 {
        return Err("expression is too long".into());
    }
    let tokens = tokenize(text)?;
    let formula = tokens.first() == Some(&"=");
    let tokens = if formula { &tokens[1..] } else { tokens.as_slice() };
    if tokens.is_empty() || tokens.len() > 64 {
        return Err("expression needs between 1 and 64 tokens".into());
    }
    let mut parser = Parser { tokens, at: 0, lookup, formula };
    let tree = parser.expression()?;
    if parser.at != tokens.len() {
        return Err(format!("unexpected token {}", tokens[parser.at]));
    }
    let tree = if formula { Expr::Formula(Box::new(tree)) } else { tree };
    Ok(Expression { text: text.to_owned(), tree })
}

impl Expression {
    /// An expression that says 0.
    pub(crate) fn zero() -> Expression {
        Expression { text: "0".into(), tree: Expr::Number(0.0, 0, 0) }
    }

    /// The text as stored.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The expression with a unit literal's text in document units. A unit literal is
    /// converted as decimal arithmetic before its first f64 parse.
    pub(crate) fn with_converted_literal(mut self) -> Expression {
        if let Expr::Number(value, 1, 0) = self.tree
            && let Ok(tokens) = tokenize(&self.text)
            && tokens.len() == 2
            && unit(tokens[1]).is_some()
        {
            self.text = format!("{value} mm");
        }
        self
    }

    /// True for one signed number, with or without a unit.
    pub fn is_literal(&self) -> bool {
        fn literal(tree: &Expr) -> bool {
            match tree {
                Expr::Number(..) => true,
                Expr::Neg(inner) => literal(inner),
                _ => false,
            }
        }
        literal(&self.tree)
    }

    /// True when it is a number with no unit.
    pub(crate) fn is_bare_literal(&self) -> bool {
        bare_literal(&self.tree)
    }

    pub(crate) fn is_formula(&self) -> bool {
        matches!(self.tree, Expr::Formula(_))
    }

    pub(crate) fn reads_physical_properties(&self) -> bool {
        self.dependencies().iter().any(|e| matches!(e, Expr::Physical(_)))
    }

    pub(crate) fn dependencies(&self) -> Vec<&Expr> {
        fn walk<'a>(tree: &'a Expr, out: &mut Vec<&'a Expr>) {
            match tree {
                Expr::Physical(_) | Expr::Param(_) | Expr::Ref(_) => out.push(tree),
                Expr::Neg(e) | Expr::Formula(e) | Expr::Percent(e) => walk(e, out),
                Expr::Binary(_, a, b) | Expr::Op(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Expr::Call(_, args) => args.iter().for_each(|e| walk(e, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        walk(&self.tree, &mut out);
        out
    }

    /// The ids of the values it reads, once each, in the order written.
    pub fn references(&self) -> Vec<u32> {
        fn walk(tree: &Expr, out: &mut Vec<u32>) {
            match tree {
                Expr::Number(..)
                | Expr::Param(_)
                | Expr::Logical(_)
                | Expr::InvalidReference
                | Expr::Physical(_)
                | Expr::Missing
                | Expr::Error(_) => {}
                Expr::Ref(id) if out.contains(id) => {}
                Expr::Ref(id) => out.push(*id),
                Expr::Neg(inner) | Expr::Formula(inner) | Expr::Percent(inner) => walk(inner, out),
                Expr::Op(_, a, b) | Expr::Binary(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Expr::Call(_, args) => args.iter().for_each(|e| walk(e, out)),
            }
        }
        let mut out = Vec::new();
        walk(&self.tree, &mut out);
        out
    }

    /// Whether it reads the feature input `input`.
    pub(crate) fn reads(&self, input: &crate::values::Input) -> bool {
        fn walk(tree: &Expr, input: &crate::values::Input) -> bool {
            match tree {
                Expr::Param(found) => found == input,
                Expr::Number(..)
                | Expr::Ref(_)
                | Expr::Logical(_)
                | Expr::InvalidReference
                | Expr::Physical(_)
                | Expr::Missing
                | Expr::Error(_) => false,
                Expr::Neg(inner) | Expr::Formula(inner) | Expr::Percent(inner) => walk(inner, input),
                Expr::Op(_, a, b) | Expr::Binary(_, a, b) => walk(a, input) || walk(b, input),
                Expr::Call(_, args) => args.iter().any(|e| walk(e, input)),
            }
        }
        walk(&self.tree, input)
    }
}

/// The names an expression's text uses, once each, in the order written.
pub(crate) fn names(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let tokens = tokenize(text).unwrap_or_default();
    let formula = text.starts_with('=');
    for (i, token) in tokens.iter().copied().enumerate() {
        let first = token.as_bytes()[0];
        if formula
            && (tokens.get(i + 1) == Some(&"(")
                || token.eq_ignore_ascii_case("TRUE")
                || token.eq_ignore_ascii_case("FALSE"))
        {
            continue;
        }
        if (first.is_ascii_alphabetic() || first == b'_') && unit(token).is_none() && !out.iter().any(|n| n == token) {
            out.push(token.to_owned());
        }
    }
    out
}

/// Checks a parameter's name: identifiers joined by dots, such as Sketch1.D1.
pub(crate) fn check_parameter_name(name: &str) -> Result<(), String> {
    if name.len() > 128 || name.split('.').any(|part| check_name(part).is_err()) {
        return Err(format!("{name}: a name must be letters, digits and underscores, with dots between parts"));
    }
    Ok(())
}

/// A table of named values for a document that binds them to inputs of its own. Ids come
/// from the owner, which keeps them stable and never reuses one. Text is kept as written.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ValueTable {
    entries: Vec<NamedValue>,
}

impl ValueTable {
    /// True with no values.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The id of the value called `name`.
    pub fn id_of(&self, name: &str) -> Option<u32> {
        self.entries.iter().find(|v| v.name == name).map(|v| v.id)
    }

    /// The name of the value with id `id`.
    pub fn name_of(&self, id: u32) -> Option<&str> {
        self.entries.iter().find(|v| v.id == id).map(|v| v.name.as_str())
    }

    /// Parses `text` against this table.
    pub fn parse(&self, text: &str) -> Result<Expression, String> {
        if text.starts_with('=') {
            return Err("unsupported character in expression".into());
        }
        parse(text, &|token| self.id_of(token).map(Expr::Ref).ok_or_else(|| format!("unknown value {token}")))
    }

    /// Gives the value called `name` a new expression, adding it under `new_id()` if it is
    /// new, and checks that every value still resolves.
    pub fn set(&mut self, name: &str, text: &str, new_id: impl FnOnce() -> u32) -> Result<(), String> {
        check_name(name)?;
        if self.id_of(name).is_none() {
            if self.entries.len() >= 256 {
                return Err("at most 256 named values are supported".into());
            }
            self.entries.push(NamedValue { id: new_id(), name: name.into(), expression: Expression::zero() });
        }
        let expression = self.parse(text)?;
        self.entries.iter_mut().find(|v| v.name == name).expect("found or added above").expression = expression;
        resolve(&self.entries)?;
        Ok(())
    }

    /// Renames a value in the table and in `others`, the expressions that read it from
    /// outside. References are by id, so only their text changes.
    pub fn rename<'a>(
        &mut self,
        name: &str,
        to: &str,
        others: impl Iterator<Item = &'a mut Expression>,
    ) -> Result<(), String> {
        check_name(to)?;
        if !self.entries.iter().any(|v| v.name == name) {
            return Err(format!("no value named {name}"));
        }
        if name != to && self.entries.iter().any(|v| v.name == to) {
            return Err(format!("the name {to} is already taken"));
        }
        for expression in self.entries.iter_mut().map(|v| &mut v.expression) {
            expression.text = rename_tokens(&expression.text, |token, _| (token == name).then_some(to));
        }
        for expression in others {
            expression.text = rename_tokens(&expression.text, |token, _| (token == name).then_some(to));
        }
        self.entries.iter_mut().find(|v| v.name == name).expect("checked above").name = to.into();
        Ok(())
    }

    /// What `expression` comes to as a length, in millimetres. A bare number is in
    /// document units, as everywhere.
    pub fn length(&self, expression: &Expression) -> Result<f64, String> {
        let q = evaluate(expression, &resolve(&self.entries)?)?;
        if (q.length, q.angle) != (1, 0) {
            return Err("the input needs a length".into());
        }
        Ok(q.value)
    }

    /// Checks ids and names are unique, that `expressions` and every value parse back to
    /// what is stored, and that every value resolves.
    pub fn validate<'a>(&self, expressions: impl Iterator<Item = &'a Expression>) -> Result<(), String> {
        for (i, v) in self.entries.iter().enumerate() {
            check_name(&v.name)?;
            if self.entries[..i].iter().any(|o| o.id == v.id || o.name == v.name) {
                return Err("duplicate value id or name".into());
            }
        }
        let agrees = |e: &Expression| Ok::<_, String>(self.parse(&e.text)?.tree == e.tree);
        for v in &self.entries {
            if !agrees(&v.expression)? {
                return Err("expression text and references disagree".into());
            }
        }
        for e in expressions {
            if !agrees(e)? {
                return Err("expression text and references disagree".into());
            }
        }
        resolve(&self.entries).map(|_| ())
    }

    /// This table with the expressions `saved` holds for the values both have, matched by
    /// id, as a configuration puts back the values it saved. What each expression reads
    /// stays bound to the ids it read when saved, and its text names them as this table
    /// does now, so a value renamed since, or a new value given an old name, changes
    /// nothing it reads. Values `saved` does not know keep theirs. The whole table is
    /// checked once, so values may read each other the other way round from before.
    pub fn restored(&self, saved: &ValueTable) -> Result<ValueTable, String> {
        let mut out = self.clone();
        for entry in &mut out.entries {
            let Some(old) = saved.expression(entry.id) else { continue };
            let mut text = String::new();
            let mut at = 0;
            for token in tokenize(&old.text)? {
                let start = token.as_ptr() as usize - old.text.as_ptr() as usize;
                text.push_str(&old.text[at..start]);
                let first = token.as_bytes()[0];
                let id = (first.is_ascii_alphabetic() || first == b'_').then(|| saved.id_of(token)).flatten();
                match id {
                    Some(id) => text.push_str(
                        self.name_of(id).ok_or_else(|| format!("{} reads a value that is gone", entry.name))?,
                    ),
                    None => text.push_str(token),
                }
                at = start + token.len();
            }
            text.push_str(&old.text[at..]);
            if self.parse(&text)?.tree != old.tree {
                return Err(format!("{}: {text} no longer reads what it read", entry.name));
            }
            entry.expression = Expression { text, tree: old.tree.clone() };
        }
        resolve(&out.entries)?;
        Ok(out)
    }

    /// The ids in use.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.iter().map(|v| v.id)
    }

    /// The expression of the value with id `id`, as written.
    pub fn expression(&self, id: u32) -> Option<&Expression> {
        self.entries.iter().find(|v| v.id == id).map(|v| &v.expression)
    }

    /// Every value in creation order, with its resolved document units.
    pub fn summary(&self) -> Value {
        let resolved = resolve(&self.entries).expect("committed values are checked");
        self.entries
            .iter()
            .map(|v| {
                let q = resolved[&v.id];
                json!({"id": v.id, "name": v.name, "expression": v.expression.text, "value": q.value, "unit": unit_name(q)})
            })
            .collect()
    }
}

/// The name of a quantity's unit.
pub(crate) fn unit_name(q: Quantity) -> &'static str {
    match q.dimensions() {
        (1, 0, 0) => "mm",
        (2, 0, 0) => "mm²",
        (3, 0, 0) => "mm³",
        (0, 1, 0) => "deg",
        (0, 0, 1) => "kg",
        (0, 0, 0) => "",
        _ => "compound",
    }
}
