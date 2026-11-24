//! Spreadsheet syntax and functions over SolidDraft quantities.
//!
//! A formula starts with `=` and reads as an Excel formula does: the same operators and
//! precedence, the same function names and argument rules, the same error values, and
//! numbers compared to 15 significant digits. Unit literals such as `10 mm` are the one
//! addition; a plain number in a formula is a plain number.

use super::{Expr, Parser, Quantity};
use serde::{Deserialize, Serialize};

/// Functions available in a parameter formula.
pub const FORMULA_FUNCTIONS: &[&str] = &[
    "ABS",
    "ACOS",
    "AND",
    "ASIN",
    "ATAN",
    "ATAN2",
    "AVERAGE",
    "CEILING",
    "CEILING.MATH",
    "CHOOSE",
    "COS",
    "COSH",
    "DEGREES",
    "EVEN",
    "EXP",
    "FACT",
    "FALSE",
    "FLOOR",
    "FLOOR.MATH",
    "GCD",
    "IF",
    "IFERROR",
    "IFNA",
    "IFS",
    "INT",
    "ISERR",
    "ISERROR",
    "ISEVEN",
    "ISLOGICAL",
    "ISNA",
    "ISNUMBER",
    "ISODD",
    "LCM",
    "LN",
    "LOG",
    "LOG10",
    "MAX",
    "MEDIAN",
    "MIN",
    "MOD",
    "MROUND",
    "NA",
    "NOT",
    "ODD",
    "OR",
    "PI",
    "POWER",
    "PRODUCT",
    "QUOTIENT",
    "RADIANS",
    "ROUND",
    "ROUNDDOWN",
    "ROUNDUP",
    "SIGN",
    "SIN",
    "SINH",
    "SQRT",
    "SQRTPI",
    "SUM",
    "SUMSQ",
    "SWITCH",
    "TAN",
    "TANH",
    "TRUE",
    "TRUNC",
    "XOR",
];

/// The error values a formula can show, as Excel spells them.
const ERROR_CODES: &[&str] = &["#DIV/0!", "#N/A", "#NAME?", "#NULL!", "#NUM!", "#REF!", "#VALUE!"];

/// Spreadsheet display code for a refused parameter formula.
pub fn formula_error_code(reason: &str) -> &'static str {
    if let Some(code) = ERROR_CODES.iter().find(|code| reason.starts_with(**code)) {
        code
    } else if reason.contains("value cycle") {
        "#CIRCULAR"
    } else if reason.contains("unknown value ") || reason.contains("unknown function ") {
        "#NAME?"
    } else if reason.contains("division by zero") {
        "#DIV/0!"
    } else if reason.contains("missing value reference") || reason.contains("feature that is gone") {
        "#REF!"
    } else if reason.contains("#NUM!") {
        "#NUM!"
    } else {
        "#VALUE!"
    }
}

/// The error literal `text` starts with, as typed, in any letter case.
pub(super) fn error_literal(text: &str) -> Option<&str> {
    ERROR_CODES.iter().find_map(|code| text.get(..code.len()).filter(|typed| typed.eq_ignore_ascii_case(code)))
}

/// The failure an error value stands for.
pub(super) fn error_value(code: &str) -> String {
    match code {
        "#DIV/0!" => "division by zero".into(),
        "#N/A" => "#N/A no value is available".into(),
        "#NAME?" => "#NAME? error value".into(),
        "#NULL!" => "#NULL! error value".into(),
        "#NUM!" => "#NUM! error value".into(),
        _ => "#VALUE! error value".into(),
    }
}

/// Why a spreadsheet character has no meaning in a parameter formula.
pub(super) fn unsupported(byte: u8) -> &'static str {
    match byte {
        b'"' | b'&' => "a parameter holds a number, not text",
        b':' => "a parameter formula reads values by name; there are no cell ranges",
        b'$' | b'!' | b'[' | b']' | b'\'' => {
            "a parameter formula reads values by name; there are no cell or sheet references"
        }
        b'{' | b'}' => "a parameter holds one number, not an array",
        _ => "unsupported character in expression",
    }
}

/// Whether a failure is an error value a formula can test, as IFERROR does. A cycle, a
/// graph too deep and a part not yet built are not values.
fn is_error_value(reason: &str) -> bool {
    !(reason.contains("value cycle") || reason.contains("too deep") || reason.contains("require the built part"))
}

/// A number as Excel keeps it for comparison: 15 significant digits.
pub(super) fn significant(value: f64) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    format!("{value:.14e}").parse().unwrap_or(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Binary {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

impl Binary {
    fn parse(token: &str) -> Option<(u8, Self)> {
        Some(match token {
            "=" => (0, Self::Eq),
            "<>" => (0, Self::Ne),
            "<" => (0, Self::Lt),
            "<=" => (0, Self::Le),
            ">" => (0, Self::Gt),
            ">=" => (0, Self::Ge),
            "+" => (1, Self::Add),
            "-" => (1, Self::Sub),
            "*" => (2, Self::Mul),
            "/" => (2, Self::Div),
            "^" => (3, Self::Pow),
            _ => return None,
        })
    }

    pub(super) fn apply(
        self,
        lhs: &Expr,
        rhs: &Expr,
        eval: &mut impl FnMut(&Expr) -> Result<Quantity, String>,
    ) -> Result<Quantity, String> {
        let a = eval(lhs)?;
        let b = eval(rhs)?;
        self.combine(a, b, false)
    }

    /// The operation a formula ends with. As in Excel, a sum or difference whose two
    /// sides agree to 15 significant digits comes to exactly zero, so `=0.5-0.4-0.1`
    /// is 0 while `=1*(0.5-0.4-0.1)` keeps the binary rounding error.
    pub(super) fn apply_last(
        self,
        lhs: &Expr,
        rhs: &Expr,
        eval: &mut impl FnMut(&Expr) -> Result<Quantity, String>,
    ) -> Result<Quantity, String> {
        let a = eval(lhs)?;
        let b = eval(rhs)?;
        self.combine(a, b, true)
    }

    fn combine(self, a: Quantity, b: Quantity, last: bool) -> Result<Quantity, String> {
        use Binary::*;
        Ok(match self {
            Mul | Div => {
                if matches!(self, Div) && b.value == 0.0 {
                    return Err("division by zero".into());
                }
                let sign = if matches!(self, Mul) { 1 } else { -1 };
                Quantity {
                    value: if sign == 1 { a.value * b.value } else { a.value / b.value },
                    length: combine(a.length, b.length, sign)?,
                    angle: combine(a.angle, b.angle, sign)?,
                    mass: combine(a.mass, b.mass, sign)?,
                    logical: false,
                }
            }
            Pow => power(a, b)?,
            Add | Sub | Eq | Ne | Lt | Le | Gt | Ge => {
                let unit = common_unit(a, b)?;
                let other = if matches!(self, Sub) { b.value } else { -b.value };
                let cancels = last && significant(a.value) == significant(other);
                match self {
                    Add | Sub if cancels => result(unit, 0.0),
                    Add => result(unit, a.value + b.value),
                    Sub => result(unit, a.value - b.value),
                    Eq => Quantity::logical(compare(a, b).is_eq()),
                    Ne => Quantity::logical(compare(a, b).is_ne()),
                    Lt => Quantity::logical(compare(a, b).is_lt()),
                    Le => Quantity::logical(compare(a, b).is_le()),
                    Gt => Quantity::logical(compare(a, b).is_gt()),
                    Ge => Quantity::logical(compare(a, b).is_ge()),
                    _ => unreachable!(),
                }
            }
        })
    }
}

impl Parser<'_> {
    pub(super) fn binary(&mut self, precedence: u8) -> Result<Expr, String> {
        let mut a = self.atom()?;
        while self.tokens.get(self.at) == Some(&"%") {
            self.at += 1;
            a = Expr::Percent(Box::new(a));
        }
        while let Some((level, op)) = self.tokens.get(self.at).and_then(|token| Binary::parse(token)) {
            if level < precedence {
                break;
            }
            self.at += 1;
            a = Expr::Binary(op, Box::new(a), Box::new(self.binary(level + 1)?));
        }
        Ok(a)
    }

    pub(super) fn call(&mut self, name: &str) -> Result<Expr, String> {
        let name = name.to_ascii_uppercase();
        if !FORMULA_FUNCTIONS.contains(&name.as_str()) {
            return Err(format!("unknown function {name}"));
        }
        self.at += 1;
        let mut args = Vec::new();
        if self.tokens.get(self.at) != Some(&")") {
            loop {
                // An argument left empty is zero, as in `=IF(A>1,,2)`.
                if matches!(self.tokens.get(self.at), Some(&",") | Some(&")")) {
                    args.push(Expr::Missing);
                } else {
                    args.push(self.expression()?);
                }
                if self.tokens.get(self.at) != Some(&",") {
                    break;
                }
                self.at += 1;
            }
        }
        if self.tokens.get(self.at) != Some(&")") {
            return Err("missing )".into());
        }
        self.at += 1;
        Ok(Expr::Call(name, args))
    }
}

/// Excel's order: numbers before logical values, numbers by their 15 significant digits.
fn compare(a: Quantity, b: Quantity) -> std::cmp::Ordering {
    a.logical
        .cmp(&b.logical)
        .then_with(|| significant(a.value).partial_cmp(&significant(b.value)).expect("finite numbers"))
}

fn combine(a: i32, b: i32, sign: i32) -> Result<i32, String> {
    let exponent = if sign == 1 { a.checked_add(b) } else { a.checked_sub(b) };
    exponent.ok_or_else(|| "unit exponent too large".into())
}

fn common_unit(a: Quantity, b: Quantity) -> Result<Quantity, String> {
    if a.is_unitless() {
        Ok(b)
    } else if b.is_unitless() || a.dimensions() == b.dimensions() {
        Ok(a)
    } else {
        Err("incompatible units".into())
    }
}

fn scalar(q: Quantity) -> Result<f64, String> {
    if q.is_unitless() { Ok(q.value) } else { Err("the function needs a plain number".into()) }
}

fn truth(q: Quantity) -> Result<bool, String> {
    Ok(scalar(q)? != 0.0)
}

fn result(q: Quantity, value: f64) -> Quantity {
    Quantity { value, logical: false, ..q }
}

fn numeric_error() -> String {
    "#NUM! result is outside the supported range".into()
}

fn power(a: Quantity, b: Quantity) -> Result<Quantity, String> {
    let exponent = scalar(b)?;
    if a.value == 0.0 && exponent < 0.0 {
        return Err("division by zero".into());
    }
    if a.value == 0.0 && exponent == 0.0 {
        return Err(numeric_error());
    }
    let value = a.value.powf(exponent);
    if !value.is_finite() {
        return Err(numeric_error());
    }
    let dimension = |base| -> Result<i32, String> {
        let raised = f64::from(base) * exponent;
        if raised.fract() != 0.0 || raised.abs() > f64::from(i32::MAX) {
            return Err("unit exponent must be a whole number".into());
        }
        Ok(raised as i32)
    };
    Ok(Quantity {
        value,
        length: dimension(a.length)?,
        angle: dimension(a.angle)?,
        mass: dimension(a.mass)?,
        logical: false,
    })
}

fn quotient(value: f64, step: f64) -> f64 {
    let ratio = value / step;
    let integer = ratio.round();
    if (ratio - integer).abs() <= f64::EPSILON * ratio.abs() { integer } else { ratio }
}

fn round(name: &str, value: f64, digits: f64) -> f64 {
    let digits = digits.trunc();
    if !(-308.0..=308.0).contains(&digits) {
        return if digits > 0.0 {
            value
        } else if value != 0.0 && name == "ROUNDUP" {
            f64::INFINITY.copysign(value)
        } else {
            0.0
        };
    }
    let power = 10_f64.powi(digits as i32);
    let scaled = value.abs() * power;
    if !scaled.is_finite() {
        return value;
    }
    let integer = match name {
        "ROUNDUP" => quotient(scaled, 1.0).ceil(),
        "ROUNDDOWN" => quotient(scaled, 1.0).floor(),
        _ if scaled < f64::MAX / 2.0 => (quotient(scaled, 0.5) * 0.5).round(),
        _ => scaled,
    };
    (integer / power).copysign(value)
}

/// A whole number argument of GCD, LCM or FACT: truncated, at least zero, exact in f64.
fn whole(q: Quantity) -> Result<f64, String> {
    let n = scalar(q)?.trunc();
    if !(0.0..=9_007_199_254_740_992.0).contains(&n) {
        return Err(numeric_error());
    }
    Ok(n)
}

fn gcd(mut a: f64, mut b: f64) -> f64 {
    while b != 0.0 {
        (a, b) = (b, a % b);
    }
    a
}

/// The argument count each function takes.
fn arity(name: &str) -> std::ops::RangeInclusive<usize> {
    match name {
        "PI" | "NA" | "TRUE" | "FALSE" => 0..=0,
        "IF" => 2..=3,
        "LOG" | "TRUNC" => 1..=2,
        "CEILING.MATH" | "FLOOR.MATH" => 1..=3,
        "AND" | "OR" | "XOR" | "MIN" | "MAX" | "SUM" | "AVERAGE" | "PRODUCT" | "MEDIAN" | "SUMSQ" | "GCD" | "LCM" => {
            1..=255
        }
        "IFS" | "CHOOSE" => 2..=255,
        "SWITCH" => 3..=255,
        "ROUND" | "ROUNDUP" | "ROUNDDOWN" | "POWER" | "MOD" | "CEILING" | "FLOOR" | "ATAN2" | "MROUND" | "QUOTIENT"
        | "IFERROR" | "IFNA" => 2..=2,
        _ => 1..=1,
    }
}

/// The functions that read only some of their arguments, or read an error as a value.
fn call_lazy(
    name: &str,
    args: &[Expr],
    eval: &mut impl FnMut(&Expr) -> Result<Quantity, String>,
) -> Option<Result<Quantity, String>> {
    let not_available = || -> Result<Quantity, String> { Err(error_value("#N/A")) };
    Some(match name {
        "IF" => (|| {
            if truth(eval(&args[0])?)? {
                eval(&args[1])
            } else {
                args.get(2).map_or(Ok(Quantity::logical(false)), &mut *eval)
            }
        })(),
        "IFS" => (|| {
            for [condition, value] in args.as_chunks::<2>().0 {
                if truth(eval(condition)?)? {
                    return eval(value);
                }
            }
            not_available()
        })(),
        "CHOOSE" => (|| {
            let index = scalar(eval(&args[0])?)?.trunc();
            if !(1.0..args.len() as f64).contains(&index) {
                return Err(error_value("#VALUE!"));
            }
            eval(&args[index as usize])
        })(),
        "SWITCH" => (|| {
            let target = eval(&args[0])?;
            let (cases, default) = args[1..].as_chunks::<2>();
            for [case, value] in cases {
                let case = eval(case)?;
                if common_unit(target, case).is_ok() && compare(target, case).is_eq() {
                    return eval(value);
                }
            }
            default.first().map_or_else(not_available, &mut *eval)
        })(),
        "IFERROR" | "IFNA" => match eval(&args[0]) {
            Err(e) if is_error_value(&e) && (name == "IFERROR" || formula_error_code(&e) == "#N/A") => eval(&args[1]),
            other => other,
        },
        "ISERROR" | "ISERR" | "ISNA" | "ISNUMBER" | "ISLOGICAL" => match eval(&args[0]) {
            Err(e) if !is_error_value(&e) => Err(e),
            Err(e) => {
                let code = formula_error_code(&e);
                Ok(Quantity::logical(match name {
                    "ISERROR" => true,
                    "ISERR" => code != "#N/A",
                    "ISNA" => code == "#N/A",
                    _ => false,
                }))
            }
            Ok(q) => Ok(Quantity::logical(match name {
                "ISNUMBER" => !q.logical,
                "ISLOGICAL" => q.logical,
                _ => false,
            })),
        },
        "NA" => not_available(),
        "TRUE" | "FALSE" => Ok(Quantity::logical(name == "TRUE")),
        _ => return None,
    })
}

pub(super) fn call(
    name: &str,
    args: &[Expr],
    degrees: bool,
    eval: &mut impl FnMut(&Expr) -> Result<Quantity, String>,
) -> Result<Quantity, String> {
    if !arity(name).contains(&args.len()) || name == "IFS" && args.len() % 2 == 1 {
        return Err(format!("{name} has the wrong number of arguments"));
    }
    if let Some(answer) = call_lazy(name, args, eval) {
        return answer;
    }
    let values = args.iter().map(eval).collect::<Result<Vec<_>, _>>()?;
    let a = values.first().copied().unwrap_or_else(|| Quantity::number(0.0));
    let b = values.get(1).copied().unwrap_or_else(|| Quantity::number(0.0));
    let angle = |q: Quantity| {
        if q.dimensions() == (0, 1, 0) || q.is_unitless() && degrees { Ok(q.value.to_radians()) } else { scalar(q) }
    };
    let inverse = |radians: f64| {
        if degrees {
            Quantity { angle: 1, ..Quantity::number(radians.to_degrees()) }
        } else {
            Quantity::number(radians)
        }
    };
    let same_unit = |values: &[Quantity]| -> Result<Quantity, String> {
        values.iter().try_fold(values[0], |unit, q| common_unit(unit, *q))
    };
    let positive = |q: Quantity| -> Result<f64, String> {
        let x = scalar(q)?;
        if x <= 0.0 { Err(numeric_error()) } else { Ok(x) }
    };
    let q = match name {
        "PI" => Quantity::number(std::f64::consts::PI),
        "NOT" => Quantity::logical(!truth(a)?),
        "AND" | "OR" | "XOR" => {
            let mut count = 0;
            for q in &values {
                count += usize::from(truth(*q)?);
            }
            Quantity::logical(match name {
                "AND" => count == values.len(),
                "OR" => count > 0,
                _ => count % 2 == 1,
            })
        }
        "MIN" | "MAX" | "SUM" | "AVERAGE" => {
            let mut current = a;
            for q in values.iter().copied().skip(1) {
                let unit = common_unit(current, q)?;
                let value = match name {
                    "MIN" => current.value.min(q.value),
                    "MAX" => current.value.max(q.value),
                    _ => current.value + q.value,
                };
                current = result(unit, value);
            }
            if name == "AVERAGE" {
                current.value /= values.len() as f64;
            }
            result(current, current.value)
        }
        "MEDIAN" => {
            let unit = same_unit(&values)?;
            let mut sorted: Vec<f64> = values.iter().map(|q| q.value).collect();
            sorted.sort_by(f64::total_cmp);
            let middle = sorted.len() / 2;
            let median =
                if sorted.len() % 2 == 1 { sorted[middle] } else { (sorted[middle - 1] + sorted[middle]) / 2.0 };
            result(unit, median)
        }
        "PRODUCT" => {
            let mut product = Quantity::number(1.0);
            for q in &values {
                product = Binary::Mul.combine(product, *q, false)?;
            }
            product
        }
        "SUMSQ" => {
            let unit = same_unit(&values)?;
            let squared = Binary::Mul.combine(unit, unit, false)?;
            result(squared, values.iter().map(|q| q.value * q.value).sum())
        }
        "ROUND" | "ROUNDUP" | "ROUNDDOWN" => result(a, round(name, a.value, scalar(b)?)),
        "TRUNC" => result(a, round("ROUNDDOWN", a.value, scalar(b)?)),
        "MROUND" => {
            let unit = common_unit(a, b)?;
            if b.value == 0.0 {
                return Ok(result(unit, 0.0));
            }
            if a.value * b.value < 0.0 {
                return Err(numeric_error());
            }
            result(unit, significant(a.value / b.value).round() * b.value)
        }
        "ABS" => result(a, a.value.abs()),
        "SIGN" => Quantity::number(if a.value > 0.0 {
            1.0
        } else if a.value < 0.0 {
            -1.0
        } else {
            0.0
        }),
        "INT" => result(a, a.value.floor()),
        "EVEN" | "ODD" => {
            let magnitude = a.value.abs();
            let n = if name == "EVEN" {
                (magnitude / 2.0).ceil() * 2.0
            } else {
                ((magnitude + 1.0) / 2.0).ceil() * 2.0 - 1.0
            };
            result(a, if a.value < 0.0 { -n } else { n })
        }
        "ISEVEN" | "ISODD" => {
            let even = scalar(a)?.trunc() % 2.0 == 0.0;
            Quantity::logical(even == (name == "ISEVEN"))
        }
        "POWER" => power(a, b)?,
        "SQRT" => power(a, Quantity::number(0.5))?,
        "SQRTPI" => {
            let x = scalar(a)?;
            if x < 0.0 {
                return Err(numeric_error());
            }
            Quantity::number((x * std::f64::consts::PI).sqrt())
        }
        "EXP" => Quantity::number(scalar(a)?.exp()),
        "LN" => Quantity::number(positive(a)?.ln()),
        "LOG10" => Quantity::number(positive(a)?.log10()),
        "LOG" => {
            let x = positive(a)?;
            let base = if values.len() == 2 { positive(b)? } else { 10.0 };
            if base == 1.0 {
                return Err("division by zero".into());
            }
            Quantity::number(if base == 10.0 {
                x.log10()
            } else if base == 2.0 {
                x.log2()
            } else {
                x.ln() / base.ln()
            })
        }
        "FACT" => {
            let n = whole(a)?;
            if n > 170.0 {
                return Err(numeric_error());
            }
            Quantity::number((1..=n as u32).map(f64::from).product())
        }
        "GCD" | "LCM" => {
            let mut answer = if name == "GCD" { 0.0 } else { 1.0 };
            for q in &values {
                let n = whole(*q)?;
                answer = if name == "GCD" {
                    gcd(answer, n)
                } else if n == 0.0 || answer == 0.0 {
                    0.0
                } else {
                    answer / gcd(answer, n) * n
                };
            }
            Quantity::number(answer)
        }
        "QUOTIENT" => {
            if b.value == 0.0 {
                return Err("division by zero".into());
            }
            let ratio = Binary::Div.combine(a, b, false)?;
            result(ratio, ratio.value.trunc())
        }
        "MOD" => {
            let unit = common_unit(a, b)?;
            if b.value == 0.0 {
                return Err("division by zero".into());
            }
            let remainder = a.value % b.value;
            let remainder = if remainder != 0.0 && remainder.is_sign_negative() != b.value.is_sign_negative() {
                remainder + b.value
            } else {
                remainder
            };
            result(unit, remainder)
        }
        "CEILING" | "FLOOR" => {
            let unit = common_unit(a, b)?;
            if b.value == 0.0 {
                if name == "FLOOR" && a.value != 0.0 {
                    return Err("division by zero".into());
                }
                return Ok(result(unit, 0.0));
            }
            if a.value > 0.0 && b.value < 0.0 {
                return Err(numeric_error());
            }
            let ratio = quotient(a.value, b.value);
            let rounded = if name == "CEILING" { ratio.ceil() } else { ratio.floor() };
            result(unit, rounded * b.value)
        }
        "CEILING.MATH" | "FLOOR.MATH" => {
            let significance = if values.len() >= 2 { b } else { Quantity::number(1.0) };
            let unit = common_unit(a, significance)?;
            let away = values.len() == 3 && scalar(values[2])? != 0.0 && a.value < 0.0;
            let step = significance.value.abs();
            if step == 0.0 {
                return Ok(result(unit, 0.0));
            }
            let ratio = quotient(a.value, step);
            let up = (name == "CEILING.MATH") != away;
            result(unit, if up { ratio.ceil() } else { ratio.floor() } * step)
        }
        "DEGREES" => Quantity::number(if a.dimensions() == (0, 1, 0) { a.value } else { scalar(a)?.to_degrees() }),
        "RADIANS" => {
            let degrees = if a.dimensions() == (0, 1, 0) { a.value } else { scalar(a)? };
            Quantity::number(degrees.to_radians())
        }
        "SIN" => Quantity::number(angle(a)?.sin()),
        "COS" => Quantity::number(angle(a)?.cos()),
        "TAN" => Quantity::number(angle(a)?.tan()),
        "SINH" => Quantity::number(scalar(a)?.sinh()),
        "COSH" => Quantity::number(scalar(a)?.cosh()),
        "TANH" => Quantity::number(scalar(a)?.tanh()),
        "ASIN" => inverse(scalar(a)?.asin()),
        "ACOS" => inverse(scalar(a)?.acos()),
        "ATAN" => inverse(scalar(a)?.atan()),
        "ATAN2" => {
            common_unit(a, b)?;
            if a.value == 0.0 && b.value == 0.0 {
                return Err("division by zero".into());
            }
            inverse(b.value.atan2(a.value))
        }
        _ => return Err(format!("unknown function {name}")),
    };
    if !q.value.is_finite() {
        return Err(numeric_error());
    }
    Ok(q)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value_expr::{evaluate_tree_with_angle, parse};

    fn formula(text: &str, degrees: bool) -> Result<Quantity, String> {
        let expression = parse(text, &|token| Err(format!("unknown value {token}")))?;
        evaluate_tree_with_angle(&expression.tree, degrees, &mut |_| Err("missing value reference".into()))
    }

    #[test]
    // Excel's documented examples round 3.14159; the expected results are not π.
    #[allow(clippy::approx_constant)]
    fn microsoft_excel_examples() {
        use std::f64::consts::PI;
        let cases = [
            ("=ROUND(2.15,1)", 2.2),
            ("=ROUND(-1.475,2)", -1.48),
            ("=ROUND(21.5,-1)", 20.0),
            ("=ROUNDUP(-3.14159,1)", -3.2),
            ("=ROUNDDOWN(3.14159,3)", 3.141),
            ("=MIN(10,7,9,27,0)", 0.0),
            ("=MAX(10,7,9,27,0)", 27.0),
            ("=SUM(12,5,-3,8,-4)", 18.0),
            ("=AVERAGE(10,7,9,27,0)", 10.6),
            ("=ABS(-2)", 2.0),
            ("=SQRT(64)", 8.0),
            ("=POWER(98.6,3.2)", 2401077.222069578),
            ("=MOD(-3,2)", 1.0),
            ("=MOD(3,-2)", -1.0),
            ("=INT(-8.9)", -9.0),
            ("=CEILING(-2.5,2)", -2.0),
            ("=CEILING(-2.5,-2)", -4.0),
            ("=FLOOR(-2.5,-2)", -2.0),
            ("=FLOOR(1.58,0.1)", 1.5),
            ("=PI()", PI),
            ("=SIN(PI()/2)", 1.0),
            ("=COS(PI()/3)", 0.5),
            ("=TAN(PI()/4)", 1.0),
            ("=ASIN(-0.5)", -PI / 6.0),
            ("=ACOS(-0.5)", 2.0 * PI / 3.0),
            ("=ATAN(1)", PI / 4.0),
            ("=ATAN2(-1,1)", 3.0 * PI / 4.0),
        ];
        for (text, expected) in cases {
            let actual = formula(text, false).unwrap().value;
            assert!((actual - expected).abs() <= expected.abs().max(1.0) * 1e-12, "{text} returned {actual}");
        }
    }

    #[test]
    fn precedence_and_logical_values() {
        for (text, expected) in [
            ("=5+2*3", 11.0),
            ("=(5+2)*3", 21.0),
            ("=-2^2", 4.0),
            ("=2^3^2", 64.0),
            ("=4^50%", 2.0),
            ("=IF(2<=3,5,1/0)", 5.0),
        ] {
            assert_eq!(formula(text, false).unwrap().value, expected, "{text}");
        }
        for (text, expected) in
            [("=NOT(2>1)", false), ("=AND(TRUE,2<>3)", true), ("=OR(FaLsE,0)", false), ("=IF(FALSE,1)", false)]
        {
            let actual = formula(text, false).unwrap();
            assert!(actual.logical);
            assert_eq!(actual.value != 0.0, expected, "{text}");
        }
    }

    #[test]
    fn units_and_degrees() {
        let area = formula("=POWER(2 cm,2)", false).unwrap();
        assert_eq!((area.value, area.length, area.angle), (400.0, 2, 0));
        let length = formula("=SQRT(2 cm*8 cm)", false).unwrap();
        assert_eq!((length.value, length.length, length.angle), (40.0, 1, 0));
        assert!((formula("=SIN(30 deg)", false).unwrap().value - 0.5).abs() < 1e-12);
        assert!((formula("=SIN(30)", true).unwrap().value - 0.5).abs() < 1e-12);
        let angle = formula("=ASIN(-0.5)", true).unwrap();
        assert!((angle.value + 30.0).abs() < 1e-12);
        assert_eq!((angle.length, angle.angle), (0, 1));
    }

    #[test]
    fn formula_errors() {
        for (text, expected) in [
            ("=NotAName", "#NAME?"),
            ("=NOPE(1)", "#NAME?"),
            ("=1/0", "#DIV/0!"),
            ("=MOD(1,0)", "#DIV/0!"),
            ("=FLOOR(2,0)", "#DIV/0!"),
            ("=ATAN2(0,0)", "#DIV/0!"),
            ("=#REF!", "#REF!"),
            ("=1 mm+1 deg", "#VALUE!"),
            ("=SIN(10 mm)", "#VALUE!"),
            ("=ROUND(2)", "#VALUE!"),
            ("=SQRT(-1)", "#NUM!"),
            ("=FLOOR(2.5,-2)", "#NUM!"),
        ] {
            let error = formula(text, false).err().unwrap();
            assert_eq!(formula_error_code(&error), expected, "{text}");
        }
    }

    /// What Excel shows for a formula.
    #[derive(Debug)]
    enum Excel {
        Number(f64),
        Logical(bool),
        Error(&'static str),
    }

    /// Formulas as a person types them into the Parameters pane, with what Microsoft Excel
    /// shows for each. Numbers agree to 12 significant digits; zero means exactly zero.
    #[rustfmt::skip]
    #[allow(clippy::approx_constant)]
    const EXCEL: &[(&str, Excel)] = &[
        ("=1+2*3", Excel::Number(7.0)),
        ("=(1+2)*3", Excel::Number(9.0)),
        ("=2^3^2", Excel::Number(64.0)),
        ("=-2^2", Excel::Number(4.0)),
        ("=0-2^2", Excel::Number(-4.0)),
        ("=2^-1", Excel::Number(0.5)),
        ("=10/4", Excel::Number(2.5)),
        ("=7-2-1", Excel::Number(4.0)),
        ("=8/2/2", Excel::Number(2.0)),
        ("=50%", Excel::Number(0.5)),
        ("=200*15%", Excel::Number(30.0)),
        ("=--3", Excel::Number(3.0)),
        ("=1+-+-2", Excel::Number(3.0)),
        ("=2*3^2", Excel::Number(18.0)),
        ("=(2+3)%", Excel::Number(0.05)),
        ("=4^50%", Excel::Number(2.0)),
        ("=1=1", Excel::Logical(true)),
        ("=1<>2", Excel::Logical(true)),
        ("=2>=2", Excel::Logical(true)),
        ("=3<2", Excel::Logical(false)),
        ("=1+1=2", Excel::Logical(true)),
        ("=0.1+0.2=0.3", Excel::Logical(true)),
        ("=TRUE+TRUE", Excel::Number(2.0)),
        ("=TRUE=1", Excel::Logical(false)),
        ("=TRUE>5", Excel::Logical(true)),
        ("=1<2<3", Excel::Logical(false)),
        ("=0.5-0.4-0.1", Excel::Number(0.0)),
        ("=1*(0.5-0.4-0.1)", Excel::Number(-2.7755575615628914e-17)),
        ("=IF(1>2,10,20)", Excel::Number(20.0)),
        ("=IF(TRUE,5)", Excel::Number(5.0)),
        ("=IF(FALSE,5)", Excel::Logical(false)),
        ("=IF(0,1,2)", Excel::Number(2.0)),
        ("=IF(TRUE,,1)", Excel::Number(0.0)),
        ("=IF(2<=3,5,1/0)", Excel::Number(5.0)),
        ("=IFS(1>2,1,2>1,2)", Excel::Number(2.0)),
        ("=IFS(FALSE,1)", Excel::Error("#N/A")),
        ("=IFERROR(1/0,7)", Excel::Number(7.0)),
        ("=IFERROR(4,7)", Excel::Number(4.0)),
        ("=IFERROR(#REF!,1)", Excel::Number(1.0)),
        ("=IFNA(NA(),3)", Excel::Number(3.0)),
        ("=IFNA(1/0,3)", Excel::Error("#DIV/0!")),
        ("=SWITCH(2,1,10,2,20)", Excel::Number(20.0)),
        ("=SWITCH(3,1,10,2,20,99)", Excel::Number(99.0)),
        ("=SWITCH(3,1,10)", Excel::Error("#N/A")),
        ("=CHOOSE(2,10,20,30)", Excel::Number(20.0)),
        ("=CHOOSE(4,1,2,3)", Excel::Error("#VALUE!")),
        ("=CHOOSE(2.9,10,20,30)", Excel::Number(20.0)),
        ("=AND(1,TRUE,2>1)", Excel::Logical(true)),
        ("=OR(FALSE,0)", Excel::Logical(false)),
        ("=XOR(TRUE,TRUE)", Excel::Logical(false)),
        ("=XOR(TRUE,FALSE,TRUE)", Excel::Logical(false)),
        ("=XOR(1,0)", Excel::Logical(true)),
        ("=NOT(0)", Excel::Logical(true)),
        ("=TRUE()", Excel::Logical(true)),
        ("=FALSE()", Excel::Logical(false)),
        ("=MIN(3,1,2)", Excel::Number(1.0)),
        ("=MAX(3,1,2)", Excel::Number(3.0)),
        ("=SUM(1,2,3)", Excel::Number(6.0)),
        ("=AVERAGE(1,2,3,4)", Excel::Number(2.5)),
        ("=PRODUCT(2,3,4)", Excel::Number(24.0)),
        ("=MEDIAN(3,1,4,1,5)", Excel::Number(3.0)),
        ("=MEDIAN(1,2,3,4)", Excel::Number(2.5)),
        ("=SUMSQ(3,4)", Excel::Number(25.0)),
        ("=SUM(1,,2)", Excel::Number(3.0)),
        ("=MAX(-1,,-3)", Excel::Number(0.0)),
        ("=ROUND(2.5,0)", Excel::Number(3.0)),
        ("=ROUND(-2.5,0)", Excel::Number(-3.0)),
        ("=ROUND(1234.567,-2)", Excel::Number(1200.0)),
        ("=ROUND(2.15,1)", Excel::Number(2.2)),
        ("=ROUND(-1.475,2)", Excel::Number(-1.48)),
        ("=ROUND(2.5,)", Excel::Number(3.0)),
        ("=ROUNDUP(3.2,0)", Excel::Number(4.0)),
        ("=ROUNDDOWN(-3.9,0)", Excel::Number(-3.0)),
        ("=MROUND(10,3)", Excel::Number(9.0)),
        ("=MROUND(-10,-3)", Excel::Number(-9.0)),
        ("=MROUND(1.3,0.2)", Excel::Number(1.4)),
        ("=MROUND(5,-2)", Excel::Error("#NUM!")),
        ("=TRUNC(-8.9)", Excel::Number(-8.0)),
        ("=TRUNC(3.14159,2)", Excel::Number(3.14)),
        ("=INT(-8.9)", Excel::Number(-9.0)),
        ("=CEILING(4.3,1)", Excel::Number(5.0)),
        ("=FLOOR(4.7,1)", Excel::Number(4.0)),
        ("=CEILING(-2.5,2)", Excel::Number(-2.0)),
        ("=FLOOR(1.58,0.1)", Excel::Number(1.5)),
        ("=CEILING.MATH(-4.3)", Excel::Number(-4.0)),
        ("=FLOOR.MATH(-4.3)", Excel::Number(-5.0)),
        ("=CEILING.MATH(6.7,2)", Excel::Number(8.0)),
        ("=FLOOR.MATH(-8.1,2)", Excel::Number(-10.0)),
        ("=CEILING.MATH(-5.5,2,-1)", Excel::Number(-6.0)),
        ("=EVEN(1.5)", Excel::Number(2.0)),
        ("=EVEN(3)", Excel::Number(4.0)),
        ("=EVEN(-1)", Excel::Number(-2.0)),
        ("=ODD(1.5)", Excel::Number(3.0)),
        ("=ODD(2)", Excel::Number(3.0)),
        ("=ODD(-2)", Excel::Number(-3.0)),
        ("=ABS(-4)", Excel::Number(4.0)),
        ("=SIGN(-3)", Excel::Number(-1.0)),
        ("=SIGN(0)", Excel::Number(0.0)),
        ("=SQRT(16)", Excel::Number(4.0)),
        ("=SQRT(-1)", Excel::Error("#NUM!")),
        ("=SQRTPI(1)", Excel::Number(1.772453850905516)),
        ("=PI()", Excel::Number(std::f64::consts::PI)),
        ("=POWER(2,10)", Excel::Number(1024.0)),
        ("=EXP(1)", Excel::Number(std::f64::consts::E)),
        ("=LN(EXP(2))", Excel::Number(2.0)),
        ("=LOG(100)", Excel::Number(2.0)),
        ("=LOG(8,2)", Excel::Number(3.0)),
        ("=LOG10(1000)", Excel::Number(3.0)),
        ("=LN(0)", Excel::Error("#NUM!")),
        ("=LOG(10,1)", Excel::Error("#DIV/0!")),
        ("=MOD(10,3)", Excel::Number(1.0)),
        ("=MOD(-10,3)", Excel::Number(2.0)),
        ("=MOD(3,-2)", Excel::Number(-1.0)),
        ("=QUOTIENT(-10,3)", Excel::Number(-3.0)),
        ("=QUOTIENT(5,0)", Excel::Error("#DIV/0!")),
        ("=FACT(5)", Excel::Number(120.0)),
        ("=FACT(1.9)", Excel::Number(1.0)),
        ("=FACT(-1)", Excel::Error("#NUM!")),
        ("=GCD(24,36)", Excel::Number(12.0)),
        ("=LCM(4,6)", Excel::Number(12.0)),
        ("=0^0", Excel::Error("#NUM!")),
        ("=(-8)^(1/3)", Excel::Error("#NUM!")),
        ("=0^-1", Excel::Error("#DIV/0!")),
        ("=DEGREES(PI())", Excel::Number(180.0)),
        ("=RADIANS(180)", Excel::Number(std::f64::consts::PI)),
        ("=SIN(PI()/6)", Excel::Number(0.5)),
        ("=COS(0)", Excel::Number(1.0)),
        ("=ATAN2(1,1)", Excel::Number(std::f64::consts::PI / 4.0)),
        ("=ATAN2(-1,1)", Excel::Number(3.0 * std::f64::consts::PI / 4.0)),
        ("=SINH(0)", Excel::Number(0.0)),
        ("=COSH(0)", Excel::Number(1.0)),
        ("=TANH(0)", Excel::Number(0.0)),
        ("=SIN(30)", Excel::Number(-0.9880316240928618)),
        ("=DEGREES(ASIN(0.5))", Excel::Number(30.0)),
        ("=ISERROR(1/0)", Excel::Logical(true)),
        ("=ISERR(NA())", Excel::Logical(false)),
        ("=ISNA(NA())", Excel::Logical(true)),
        ("=ISNUMBER(5)", Excel::Logical(true)),
        ("=ISNUMBER(TRUE)", Excel::Logical(false)),
        ("=ISNUMBER(1/0)", Excel::Logical(false)),
        ("=ISLOGICAL(TRUE)", Excel::Logical(true)),
        ("=ISEVEN(4)", Excel::Logical(true)),
        ("=ISODD(3)", Excel::Logical(true)),
        ("=1/0", Excel::Error("#DIV/0!")),
        ("=NA()", Excel::Error("#N/A")),
        ("=#N/A", Excel::Error("#N/A")),
        ("=#n/a", Excel::Error("#N/A")),
        ("=#DIV/0!+1", Excel::Error("#DIV/0!")),
        ("=#REF!", Excel::Error("#REF!")),
        ("=#VALUE!", Excel::Error("#VALUE!")),
        ("=#NUM!", Excel::Error("#NUM!")),
        ("=#NULL!", Excel::Error("#NULL!")),
        ("=#NAME?", Excel::Error("#NAME?")),
        ("=Nope", Excel::Error("#NAME?")),
        ("=NOPE(1)", Excel::Error("#NAME?")),
        ("=1/0+NA()", Excel::Error("#DIV/0!")),
        ("=NA()+1/0", Excel::Error("#N/A")),
        ("=pi()", Excel::Number(std::f64::consts::PI)),
        ("=Round(2.5,0)", Excel::Number(3.0)),
        ("=true", Excel::Logical(true)),
    ];

    #[test]
    fn formulas_evaluate_as_excel_shows_them() {
        let mut wrong = Vec::new();
        for (text, expected) in EXCEL {
            let actual = formula(text, false);
            let agrees = match (expected, &actual) {
                (Excel::Number(n), Ok(q)) => !q.logical && (q.value - n).abs() <= n.abs() * 1e-12,
                (Excel::Logical(b), Ok(q)) => q.logical && (q.value != 0.0) == *b,
                (Excel::Error(code), Err(e)) => formula_error_code(e) == *code,
                _ => false,
            };
            if !agrees {
                let shown = actual.map(|q| format!("{}{}", q.value, if q.logical { " (logical)" } else { "" }));
                wrong.push(format!("{text}: Excel shows {expected:?}, got {shown:?}"));
            }
        }
        assert!(
            wrong.is_empty(),
            "{} of {} formulas differ from Excel:\n{}",
            wrong.len(),
            EXCEL.len(),
            wrong.join("\n")
        );
    }

    #[test]
    fn unit_literals_carry_through_excel_functions() {
        for (text, value, length, angle) in [
            ("=10 mm*2", 20.0, 1, 0),
            ("=1 in+1 mm", 26.4, 1, 0),
            ("=MROUND(23 mm,5)", 25.0, 1, 0),
            ("=SUMSQ(3 mm,4 mm)", 25.0, 2, 0),
            ("=PRODUCT(2 mm,3 mm,4)", 24.0, 2, 0),
            ("=MEDIAN(1 cm,5 mm,30 mm)", 10.0, 1, 0),
            ("=QUOTIENT(25 mm,10 mm)", 2.0, 0, 0),
            ("=TRUNC(12.7 mm)", 12.0, 1, 0),
            ("=SIN(30 deg)", 0.5, 0, 0),
            ("=DEGREES(90 deg)", 90.0, 0, 0),
            ("=IFERROR(1/0,5 mm)", 5.0, 1, 0),
        ] {
            let q = formula(text, false).unwrap_or_else(|e| panic!("{text}: {e}"));
            assert!((q.value - value).abs() <= 1e-12 * value.abs(), "{text} returned {}", q.value);
            assert_eq!((q.length, q.angle), (length, angle), "{text}");
        }
        assert!((formula("=RADIANS(180 deg)", false).unwrap().value - std::f64::consts::PI).abs() < 1e-15);
        assert_eq!(formula_error_code(&formula("=1 mm+1 deg", false).err().unwrap()), "#VALUE!");
        assert_eq!(formula_error_code(&formula("=LN(10 mm)", false).err().unwrap()), "#VALUE!");
    }

    #[test]
    fn spreadsheet_text_ranges_and_sheets_are_refused_in_words() {
        for (text, words) in [
            ("=\"thick\"", "not text"),
            ("=1&2", "not text"),
            ("=SUM(A1:A3)", "no cell ranges"),
            ("=Sheet1!A1", "no cell or sheet references"),
            ("=$A$1", "no cell or sheet references"),
            ("={1,2}", "not an array"),
        ] {
            let error = formula(text, false).err().unwrap_or_else(|| panic!("{text} was accepted"));
            assert!(error.contains(words), "{text}: {error}");
        }
    }

    #[test]
    fn a_cycle_is_not_an_error_value_iferror_can_hide() {
        let mut eval = |_: &Expr| -> Result<Quantity, String> { Err("value cycle".into()) };
        let tree = parse("=IFERROR(x,1)", &|_| Ok(Expr::Ref(1))).unwrap().tree;
        assert_eq!(evaluate_tree_with_angle(&tree, false, &mut eval).err().as_deref(), Some("value cycle"));
    }
}
