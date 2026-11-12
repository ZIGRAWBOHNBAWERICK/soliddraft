//! Catallax `path` and `sweep` steps for the application's line and arc chains.
//!
//! The chain is checked here so a refusal reads plainly. Catallax still decides the
//! supported domain: its refusal codes, never its wording, reach the application.

use std::collections::BTreeSet;

use catallax_numbers::{Integer, Rational, Sign};
use catallax_recipe::{ElementId, Number, Operation};
use catallax_shapes::{RPoint3, RVector3};
use serde_json::{Value, json};
use soliddraft_kernel_api::*;

use crate::{gap, native};

const ENDPOINTS: &str = "Sweep needs two different finite endpoints";
const REFUSAL: &str = "This sweep could not finish. Try a simpler polygon profile or path";
/// The path motions this build cannot follow yet.
const UNFINISHED: &str = "This sweep path";

fn invalid(message: &str) -> KernelError {
    KernelError::InvalidInput(message.into())
}

fn numbers(value: [f64; 3]) -> KernelResult<[Number; 3]> {
    if !value.iter().all(|v| v.is_finite()) {
        return Err(invalid(ENDPOINTS));
    }
    let [x, y, z] = value;
    Ok([native::number(x, "sweep path")?, native::number(y, "sweep path")?, native::number(z, "sweep path")?])
}

fn point(value: [f64; 3]) -> KernelResult<RPoint3> {
    let [x, y, z] = numbers(value)?.map(|n| n.value().clone());
    Ok(RPoint3::new(x, y, z))
}

fn vector(value: [f64; 3]) -> KernelResult<RVector3> {
    let [x, y, z] = numbers(value)?.map(|n| n.value().clone());
    Ok(RVector3::new(x, y, z))
}

fn text(point: &RPoint3) -> Value {
    json!([Number::new(point.x.clone()), Number::new(point.y.clone()), Number::new(point.z.clone())])
}

fn root(n: &Integer) -> Option<Integer> {
    if n.sign() == Sign::Negative {
        return None;
    }
    if n.is_zero() {
        return Some(Integer::zero());
    }
    let mut x = Integer::one().mul_pow2(n.bit_length().div_ceil(2));
    loop {
        let q = n.div_rem_floor(&x)?.0;
        let y = (&x + &q).div_rem_floor(&Integer::from_i64(2))?.0;
        if y >= x {
            return (&x * &x == *n).then_some(x);
        }
        x = y;
    }
}

/// The exact square root, when it is rational.
fn sqrt(n: &Rational) -> Option<Rational> {
    Rational::new(root(n.numerator())?, root(n.denominator())?)
}

/// Where an arc ends. Only an axis of rational length has a rational quarter turn.
fn arc_end(start: &RPoint3, centre: &RPoint3, axis: &RVector3, turn: ArcTurn) -> KernelResult<RPoint3> {
    let radius = start.minus(centre);
    if axis.is_zero() {
        return Err(invalid("An arc needs a nonzero axis"));
    }
    if radius.is_zero() {
        return Err(invalid("An arc needs a centre away from where it starts"));
    }
    if !radius.dot(axis).is_zero() {
        return Err(invalid("An arc's axis must be square to the line from its centre to where it starts"));
    }
    let across = |sign: i64| -> KernelResult<RVector3> {
        let length = sqrt(&axis.norm_squared()).ok_or_else(|| gap(UNFINISHED))?;
        Ok(axis.cross(&radius).scaled(&(Rational::from_i64(sign) / length)))
    };
    Ok(centre.plus(&match turn {
        ArcTurn::Quarter => across(1)?,
        ArcTurn::Half => radius.scaled(&Rational::from_i64(-1)),
        ArcTurn::ThreeQuarter => across(-1)?,
    }))
}

/// The Catallax `path` step for `path`, with exact arc ends. Refuses an invalid path.
pub(super) fn path_step(tag: &str, path: &Sweep) -> KernelResult<Value> {
    if path.segments.is_empty() {
        return Err(invalid("Sweep needs at least one path segment"));
    }
    let start = point(path.start)?;
    let mut previous = start.clone();
    let mut names = BTreeSet::new();
    let mut segments = Vec::with_capacity(path.segments.len());
    for segment in &path.segments {
        if ElementId::new(segment.id()).is_err() {
            return Err(invalid("Sweep path segment needs a valid name"));
        }
        if !names.insert(segment.id()) {
            return Err(invalid("Sweep path segments need different names"));
        }
        let mut step = match segment {
            PathSegment::Line { id, to, .. } => {
                let to = point(*to)?;
                if to == previous {
                    return Err(invalid(ENDPOINTS));
                }
                previous = to;
                json!({"kind":"line", "id":id, "to":text(&previous)})
            }
            PathSegment::Spline { id, degree, control_points, weights, knots } => {
                let count = control_points.len();
                if *degree == 0 || *degree >= count {
                    return Err(invalid("A spline degree must be positive and less than its number of control points"));
                }
                if knots.len() != count + degree + 1 {
                    return Err(invalid("A spline needs one knot per control point plus its degree plus one"));
                }
                if weights.len() != count || weights.iter().any(|w| !w.is_finite() || *w <= 0.0) {
                    return Err(invalid("A spline needs one positive finite weight per control point"));
                }
                if knots.iter().any(|k| !k.is_finite())
                    || knots.windows(2).any(|k| k[0] > k[1])
                    || knots[..=*degree].iter().any(|k| *k != 0.0)
                    || knots[count..].iter().any(|k| *k != 1.0)
                    || knots[degree + 1..count].iter().any(|k| *k <= 0.0 || *k >= 1.0)
                    || knots[degree + 1..count].windows(degree + 1).any(|k| k[0] == k[*degree])
                {
                    return Err(invalid(
                        "Spline knots must be ordered, start at zero and end at one, with no interior knot repeated more than the degree",
                    ));
                }
                let points = control_points.iter().map(|p| point(*p)).collect::<KernelResult<Vec<_>>>()?;
                if points.first() != Some(&previous) {
                    return Err(invalid("A spline must start where the previous path segment ends"));
                }
                let values = |values: &[f64]| {
                    values.iter().map(|v| native::number(*v, "spline path")).collect::<KernelResult<Vec<_>>>()
                };
                previous = points.last().cloned().unwrap_or(previous);
                json!({"kind":"spline", "id":id, "degree":degree,
                    "control_points":points.iter().map(text).collect::<Vec<_>>(),
                    "weights":values(weights)?, "knots":values(knots)?})
            }
            PathSegment::Arc { id, centre, axis, turn, .. } => {
                let (centre, axis) = (point(*centre)?, vector(*axis)?);
                previous = arc_end(&previous, &centre, &axis, *turn)?;
                let [x, y, z] = [axis.x, axis.y, axis.z].map(Number::new);
                json!({"kind":"arc", "id":id, "centre":text(&centre), "to":text(&previous), "axis":[x, y, z]})
            }
        };
        if let Some(axis) = segment.corner_axis() {
            step["corner_axis"] = json!(numbers(axis)?);
        }
        segments.push(step);
    }
    Ok(json!({"id":format!("{tag}_path"), "op":"path", "start":text(&start), "segments":segments}))
}

/// The Catallax `path` step of a stored sweep's parameters.
pub(super) fn stored_path(tag: &str, stored: &Value) -> KernelResult<Value> {
    let path: Sweep = serde_json::from_value(stored.clone())
        .map_err(|e| KernelError::InvalidInput(format!("stored sweep path: {e}")))?;
    path_step(tag, &path)
}

/// The Catallax `sweep` step reading `{tag}_path`.
pub(super) fn operation(tag: &str, profile: &Value, stored: &Value) -> KernelResult<Operation> {
    let path: Sweep = serde_json::from_value(stored.clone())
        .map_err(|e| KernelError::InvalidInput(format!("stored sweep path: {e}")))?;
    let orientation = match path.orientation {
        SweepOrientation::Fixed => "fixed",
        SweepOrientation::FollowPath => "rotation_minimising",
    };
    serde_json::from_value(json!({
        "op":"sweep", "profile":profile, "path":format!("{tag}_path"), "orientation":orientation
    }))
    .map_err(|e| KernelError::InvalidInput(format!("stored sweep: {e}")))
}

/// What a person reads when a sweep does not build. `code` is Catallax's stable refusal.
pub(super) fn refusal(code: &str) -> KernelError {
    match code {
        "features.unsupported-path" => gap(UNFINISHED),
        "sweep.spline-admissibility-unproven" => invalid(
            "This spline sweep was refused because the path may turn back or cross itself. Use a path that keeps moving away from the profile plane",
        ),
        "sweep.missing-corner-axis" => invalid("Set a turn axis where the path doubles back"),
        "path.corner-axis" => {
            invalid("A turn axis belongs only where the path doubles back, and must be square to the path")
        }
        _ => KernelError::Unsupported(REFUSAL.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: i64, y: i64, z: i64) -> RPoint3 {
        RPoint3::new(Rational::from_i64(x), Rational::from_i64(y), Rational::from_i64(z))
    }

    fn v(x: i64, y: i64, z: i64) -> RVector3 {
        RVector3::new(Rational::from_i64(x), Rational::from_i64(y), Rational::from_i64(z))
    }

    #[test]
    fn exact_ends_follow_the_right_hand_rule_for_any_rational_axis_length() {
        let (start, centre) = (p(0, 0, 4), p(0, 5, 4));
        assert_eq!(arc_end(&start, &centre, &v(-2, 0, 0), ArcTurn::Quarter).unwrap(), p(0, 5, 9));
        assert_eq!(arc_end(&start, &centre, &v(-2, 0, 0), ArcTurn::Half).unwrap(), p(0, 10, 4));
        assert_eq!(arc_end(&start, &centre, &v(-2, 0, 0), ArcTurn::ThreeQuarter).unwrap(), p(0, 5, -1));
    }

    #[test]
    fn an_axis_of_irrational_length_has_only_a_half_turn() {
        let (start, centre) = (p(1, -1, 0), p(0, 0, 0));
        let axis = v(1, 1, 0);
        assert_eq!(arc_end(&start, &centre, &axis, ArcTurn::Half).unwrap(), p(-1, 1, 0));
        let refused = arc_end(&start, &centre, &axis, ArcTurn::Quarter).unwrap_err().to_string();
        assert!(refused.contains("coming soon"), "{refused}");
    }

    #[test]
    fn decimal_inputs_stay_exact_through_a_following_arc() {
        let line = PathSegment::Line { id: "rise".into(), to: [0.1, 0.2, 0.3], corner_axis: None };
        let arc = PathSegment::Arc {
            id: "bend".into(),
            centre: [0.1, 0.7, 0.3],
            axis: [-1.0, 0.0, 0.0],
            turn: ArcTurn::Quarter,
            corner_axis: None,
        };
        let path =
            Sweep { start: [0.1, 0.2, 0.0], segments: vec![line, arc], orientation: SweepOrientation::FollowPath };
        let step = path_step("t", &path).unwrap();
        assert_eq!(step["segments"][1]["to"], json!(["0.1", "0.7", "0.8"]));
    }

    #[test]
    fn invalid_paths_are_refused_in_plain_words() {
        let arc = |centre, axis| Sweep {
            start: [0.0; 3],
            segments: vec![PathSegment::Arc { id: "a".into(), centre, axis, turn: ArcTurn::Half, corner_axis: None }],
            orientation: SweepOrientation::FollowPath,
        };
        for (path, expected) in [
            (arc([0.0, 5.0, 0.0], [0.0; 3]), "nonzero axis"),
            (arc([0.0; 3], [1.0, 0.0, 0.0]), "centre away"),
            (arc([0.0, 5.0, 0.0], [0.0, 1.0, 0.0]), "square to the line"),
            (Sweep { segments: vec![], ..arc([0.0, 5.0, 0.0], [1.0, 0.0, 0.0]) }, "at least one"),
            (Sweep::line([0.0; 3], [f64::NAN; 3], "t"), "finite"),
            (Sweep::line([0.0; 3], [1.0; 3], "Not A Name"), "valid name"),
        ] {
            let refused = path_step("t", &path).unwrap_err().to_string();
            assert!(refused.contains(expected), "{refused}");
        }
        let mut twice = Sweep::line([0.0; 3], [1.0; 3], "t");
        twice.segments.push(PathSegment::Line { id: "t".into(), to: [2.0; 3], corner_axis: None });
        assert!(path_step("t", &twice).unwrap_err().to_string().contains("different names"));
    }
}
