//! Spreadsheet syntax and functions over SolidDraft quantities.

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
    "COS",
    "FLOOR",
    "IF",
    "INT",
    "MAX",
    "MIN",
    "MOD",
    "NOT",
    "OR",
    "PI",
    "POWER",
    "ROUND",
    "ROUNDDOWN",
    "ROUNDUP",
    "SIN",
    "SQRT",
    "SUM",
    "TAN",
];

/// Spreadsheet display code for a refused parameter formula.
pub fn formula_error_code(reason: &str) -> &'static str {
    if reason.contains("value cycle") {
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
        use Binary::*;
        let a = eval(lhs)?;
        let b = eval(rhs)?;
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
                match self {
                    Add => result(unit, a.value + b.value),
                    Sub => result(unit, a.value - b.value),
                    Eq => Quantity::logical(a.value == b.value && a.logical == b.logical),
                    Ne => Quantity::logical(a.value != b.value || a.logical != b.logical),
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
                args.push(self.expression()?);
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

fn compare(a: Quantity, b: Quantity) -> std::cmp::Ordering {
    a.logical.cmp(&b.logical).then_with(|| a.value.partial_cmp(&b.value).expect("finite numbers"))
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
    let dimension = |base| {
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

pub(super) fn call(
    name: &str,
    args: &[Expr],
    degrees: bool,
    eval: &mut impl FnMut(&Expr) -> Result<Quantity, String>,
) -> Result<Quantity, String> {
    let arity = match name {
        "PI" => 0..=0,
        "IF" => 2..=3,
        "AND" | "OR" | "MIN" | "MAX" | "SUM" | "AVERAGE" => 1..=usize::MAX,
        "ROUND" | "ROUNDUP" | "ROUNDDOWN" | "POWER" | "MOD" | "CEILING" | "FLOOR" | "ATAN2" => 2..=2,
        _ => 1..=1,
    };
    if !arity.contains(&args.len()) {
        return Err(format!("{name} has the wrong number of arguments"));
    }
    if name == "IF" {
        return if scalar(eval(&args[0])?)? != 0.0 {
            eval(&args[1])
        } else {
            args.get(2).map_or(Ok(Quantity::logical(false)), eval)
        };
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
    let q = match name {
        "PI" => Quantity::number(std::f64::consts::PI),
        "NOT" => Quantity::logical(scalar(a)? == 0.0),
        "AND" | "OR" => {
            let mut truth = name == "AND";
            for q in values {
                let value = scalar(q)? != 0.0;
                truth = if name == "AND" { truth && value } else { truth || value };
            }
            Quantity::logical(truth)
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
        "ROUND" | "ROUNDUP" | "ROUNDDOWN" => result(a, round(name, a.value, scalar(b)?)),
        "ABS" => result(a, a.value.abs()),
        "INT" => result(a, a.value.floor()),
        "POWER" => power(a, b)?,
        "SQRT" => power(a, Quantity::number(0.5))?,
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
                return Ok(result(unit, 0.0));
            }
            if a.value > 0.0 && b.value < 0.0 {
                return Err(numeric_error());
            }
            let ratio = quotient(a.value, b.value);
            let rounded = if name == "CEILING" { ratio.ceil() } else { ratio.floor() };
            result(unit, rounded * b.value)
        }
        "SIN" => Quantity::number(angle(a)?.sin()),
        "COS" => Quantity::number(angle(a)?.cos()),
        "TAN" => Quantity::number(angle(a)?.tan()),
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
}
