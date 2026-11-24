//! The functions a parameter formula can use, grouped as Excel's Insert Function groups
//! them, each with its form and one line on what it gives.

/// One function's help.
#[derive(Clone, Copy, Debug)]
pub struct Help {
    /// The name as typed.
    pub name: &'static str,
    /// The form, as Excel writes it.
    pub syntax: &'static str,
    /// What it gives.
    pub about: &'static str,
}

const fn f(name: &'static str, syntax: &'static str, about: &'static str) -> Help {
    Help { name, syntax, about }
}

/// Every formula function, by group.
pub const GROUPS: &[(&str, &[Help])] = &[
    (
        "Logical",
        &[
            f(
                "IF",
                "IF(test, value_if_true, [value_if_false])",
                "One value when the test is TRUE, another when it is FALSE",
            ),
            f("IFS", "IFS(test1, value1, [test2, value2], ...)", "The value of the first test that is TRUE"),
            f("SWITCH", "SWITCH(value, match1, result1, ..., [default])", "The result whose match equals the value"),
            f("CHOOSE", "CHOOSE(index, value1, value2, ...)", "The value at a position: 1 for the first"),
            f("AND", "AND(test1, [test2], ...)", "TRUE when every test is TRUE"),
            f("OR", "OR(test1, [test2], ...)", "TRUE when any test is TRUE"),
            f("XOR", "XOR(test1, [test2], ...)", "TRUE when an odd number of tests are TRUE"),
            f("NOT", "NOT(test)", "TRUE for FALSE and FALSE for TRUE"),
            f("TRUE", "TRUE()", "The logical value TRUE"),
            f("FALSE", "FALSE()", "The logical value FALSE"),
        ],
    ),
    (
        "Math",
        &[
            f("SUM", "SUM(number1, [number2], ...)", "Adds the numbers"),
            f("PRODUCT", "PRODUCT(number1, [number2], ...)", "Multiplies the numbers"),
            f("SUMSQ", "SUMSQ(number1, [number2], ...)", "Adds the squares of the numbers"),
            f("MIN", "MIN(number1, [number2], ...)", "The smallest number"),
            f("MAX", "MAX(number1, [number2], ...)", "The largest number"),
            f("AVERAGE", "AVERAGE(number1, [number2], ...)", "The mean of the numbers"),
            f("MEDIAN", "MEDIAN(number1, [number2], ...)", "The middle number"),
            f("ABS", "ABS(number)", "The number without its sign"),
            f("SIGN", "SIGN(number)", "1, 0 or -1, by the sign of the number"),
            f("SQRT", "SQRT(number)", "The square root"),
            f("SQRTPI", "SQRTPI(number)", "The square root of the number times pi"),
            f("POWER", "POWER(number, power)", "The number raised to a power, as number^power"),
            f("EXP", "EXP(number)", "e raised to the number"),
            f("LN", "LN(number)", "The natural logarithm"),
            f("LOG", "LOG(number, [base])", "The logarithm to a base, 10 when left out"),
            f("LOG10", "LOG10(number)", "The base-10 logarithm"),
            f("MOD", "MOD(number, divisor)", "The remainder, with the divisor's sign"),
            f("QUOTIENT", "QUOTIENT(numerator, denominator)", "The whole part of a division"),
            f("GCD", "GCD(number1, [number2], ...)", "The greatest common divisor"),
            f("LCM", "LCM(number1, [number2], ...)", "The least common multiple"),
            f("FACT", "FACT(number)", "The factorial"),
            f("PI", "PI()", "3.14159265358979"),
        ],
    ),
    (
        "Rounding",
        &[
            f("ROUND", "ROUND(number, digits)", "Rounds half away from zero to a number of digits"),
            f("ROUNDUP", "ROUNDUP(number, digits)", "Rounds away from zero"),
            f("ROUNDDOWN", "ROUNDDOWN(number, digits)", "Rounds toward zero"),
            f("MROUND", "MROUND(number, multiple)", "Rounds to the nearest multiple"),
            f("CEILING", "CEILING(number, significance)", "Rounds up to a multiple of the significance"),
            f("CEILING.MATH", "CEILING.MATH(number, [significance], [mode])", "Rounds up to a multiple, 1 by default"),
            f("FLOOR", "FLOOR(number, significance)", "Rounds down to a multiple of the significance"),
            f("FLOOR.MATH", "FLOOR.MATH(number, [significance], [mode])", "Rounds down to a multiple, 1 by default"),
            f("INT", "INT(number)", "Rounds down to a whole number"),
            f("TRUNC", "TRUNC(number, [digits])", "Cuts off the decimals"),
            f("EVEN", "EVEN(number)", "Rounds away from zero to an even whole number"),
            f("ODD", "ODD(number)", "Rounds away from zero to an odd whole number"),
        ],
    ),
    (
        "Trigonometry",
        &[
            f("SIN", "SIN(angle)", "The sine; a plain number is in radians, 30 deg is in degrees"),
            f("COS", "COS(angle)", "The cosine"),
            f("TAN", "TAN(angle)", "The tangent"),
            f("ASIN", "ASIN(number)", "The arcsine, in radians"),
            f("ACOS", "ACOS(number)", "The arccosine, in radians"),
            f("ATAN", "ATAN(number)", "The arctangent, in radians"),
            f("ATAN2", "ATAN2(x, y)", "The angle of the point x, y, in radians"),
            f("DEGREES", "DEGREES(radians)", "Radians in degrees"),
            f("RADIANS", "RADIANS(degrees)", "Degrees in radians"),
            f("SINH", "SINH(number)", "The hyperbolic sine"),
            f("COSH", "COSH(number)", "The hyperbolic cosine"),
            f("TANH", "TANH(number)", "The hyperbolic tangent"),
        ],
    ),
    (
        "Errors and tests",
        &[
            f("IFERROR", "IFERROR(value, value_if_error)", "The value, or another when it is an error"),
            f("IFNA", "IFNA(value, value_if_na)", "The value, or another when it is #N/A"),
            f("ISERROR", "ISERROR(value)", "TRUE for any error value"),
            f("ISERR", "ISERR(value)", "TRUE for any error value but #N/A"),
            f("ISNA", "ISNA(value)", "TRUE for #N/A"),
            f("ISNUMBER", "ISNUMBER(value)", "TRUE for a number"),
            f("ISLOGICAL", "ISLOGICAL(value)", "TRUE for TRUE or FALSE"),
            f("ISEVEN", "ISEVEN(number)", "TRUE for an even number"),
            f("ISODD", "ISODD(number)", "TRUE for an odd number"),
            f("NA", "NA()", "The error value #N/A"),
        ],
    ),
];

/// The help for the function called `name`, in any letter case.
pub fn help(name: &str) -> Option<&'static Help> {
    GROUPS.iter().flat_map(|(_, functions)| functions.iter()).find(|h| h.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_formula_function_has_help_and_no_help_names_a_missing_function() {
        for name in soliddraft_model::FORMULA_FUNCTIONS {
            let help = help(name).unwrap_or_else(|| panic!("{name} has no help"));
            assert!(help.syntax.starts_with(&format!("{name}(")), "{name}: {}", help.syntax);
        }
        let listed = GROUPS.iter().map(|(_, functions)| functions.len()).sum::<usize>();
        assert_eq!(listed, soliddraft_model::FORMULA_FUNCTIONS.len());
    }
}
