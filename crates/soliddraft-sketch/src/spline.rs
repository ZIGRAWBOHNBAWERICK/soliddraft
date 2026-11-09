//! Polynomial spline evaluation in sketch coordinates.

use crate::P2;

/// How authored points define a spline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplineMode {
    /// Natural cubic interpolation through every authored point.
    Fit,
    /// A clamped uniform B-spline with degree at most three.
    Control,
}

/// One polynomial span in its local unit interval.
#[derive(Clone, Debug, PartialEq)]
pub struct BezierSpan {
    /// Ordered endpoints and interior controls, with degree one to three.
    pub controls: Vec<P2>,
}
impl BezierSpan {
    /// Evaluate with de Casteljau interpolation.
    pub fn at(&self, t: f64) -> P2 {
        evaluate_bezier(&self.controls, t)
    }

    /// Derivative with respect to the local span parameter.
    pub fn derivative(&self, t: f64) -> P2 {
        let degree = (self.controls.len() - 1) as f64;
        let differences: Vec<_> = self.controls.windows(2).map(|p| (p[1] - p[0]).scale(degree)).collect();
        evaluate_bezier(&differences, t)
    }

    /// Second derivative with respect to the local span parameter.
    pub fn second_derivative(&self, t: f64) -> P2 {
        let degree = self.controls.len() - 1;
        if degree < 2 {
            return P2::ZERO;
        }
        let differences: Vec<_> = self
            .controls
            .windows(3)
            .map(|p| (p[2] - p[1].scale(2.0) + p[0]).scale((degree * (degree - 1)) as f64))
            .collect();
        evaluate_bezier(&differences, t)
    }

    /// Half the integral of u dv - v du over this span.
    pub fn signed_area(&self) -> f64 {
        let p = &self.controls;
        let coefficients = match p.len() {
            2 => vec![p[0], p[1] - p[0]],
            3 => vec![p[0], (p[1] - p[0]).scale(2.0), p[0] - p[1].scale(2.0) + p[2]],
            4 => vec![
                p[0],
                (p[1] - p[0]).scale(3.0),
                (p[0] - p[1].scale(2.0) + p[2]).scale(3.0),
                p[3] - p[2].scale(3.0) + p[1].scale(3.0) - p[0],
            ],
            _ => unreachable!("polynomial span degree"),
        };
        let mut area = 0.0;
        for (i, a) in coefficients.iter().enumerate() {
            for (j, b) in coefficients.iter().enumerate().skip(1) {
                area += a.cross(*b) * j as f64 / (i + j) as f64;
            }
        }
        area * 0.5
    }

    /// Horizontal ray crossings with the usual half-open endpoint rule.
    pub fn crossings(&self, point: P2) -> usize {
        let n = self.controls.len() - 1;
        let d: Vec<_> = self.controls.windows(2).map(|p| (p[1].v - p[0].v) * n as f64).collect();
        let mut cuts = vec![0.0, 1.0];
        let mut root = |t: f64| {
            if t > 0.0 && t < 1.0 {
                cuts.push(t);
            }
        };
        if n == 2 {
            if d[1] != d[0] {
                root(-d[0] / (d[1] - d[0]));
            }
        } else if n == 3 {
            let a = d[0] - 2.0 * d[1] + d[2];
            let b = 2.0 * (d[1] - d[0]);
            let c = d[0];
            if a == 0.0 {
                if b != 0.0 {
                    root(-c / b);
                }
            } else {
                let discriminant = b * b - 4.0 * a * c;
                if discriminant >= 0.0 {
                    let q = -0.5 * (b + discriminant.sqrt().copysign(b));
                    if q == 0.0 {
                        root(-b / (2.0 * a));
                    } else {
                        root(q / a);
                        root(c / q);
                    }
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup();
        cuts.windows(2)
            .filter(|pair| {
                let a = self.at(pair[0]);
                let b = self.at(pair[1]);
                if (a.v > point.v) == (b.v > point.v) {
                    return false;
                }
                let mut lo = pair[0];
                let mut hi = pair[1];
                for _ in 0..56 {
                    let mid = (lo + hi) * 0.5;
                    if (self.at(mid).v > point.v) == (a.v > point.v) {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                self.at((lo + hi) * 0.5).u > point.u
            })
            .count()
    }

    /// Subdivide the control polygon at one half.
    pub fn split(&self) -> [Self; 2] {
        let mut row = self.controls.clone();
        let mut left = vec![row[0]];
        let mut right = vec![*row.last().unwrap()];
        for count in (1..row.len()).rev() {
            for i in 0..count {
                row[i] = row[i].mid(row[i + 1]);
            }
            left.push(row[0]);
            right.push(row[count - 1]);
        }
        right.reverse();
        [Self { controls: left }, Self { controls: right }]
    }
}

fn evaluate_bezier(points: &[P2], t: f64) -> P2 {
    let mut row = points.to_vec();
    for count in (1..row.len()).rev() {
        for i in 0..count {
            row[i] = row[i].lerp(row[i + 1], t);
        }
    }
    row[0]
}

/// Builds polynomial spans without fitting or sampling the authored geometry.
/// Invalid coordinates and fewer than two points fail.
pub fn spans(mode: SplineMode, points: &[P2]) -> Option<Vec<BezierSpan>> {
    if points.len() < 2 || points.iter().any(|p| !p.u.is_finite() || !p.v.is_finite()) {
        return None;
    }
    Some(match mode {
        SplineMode::Fit => fit_spans(points),
        SplineMode::Control => control_spans(points),
    })
}

fn fit_spans(points: &[P2]) -> Vec<BezierSpan> {
    if points.len() == 2 {
        return vec![BezierSpan { controls: points.to_vec() }];
    }
    // Natural end conditions M0=Mn=0. The interior second derivatives solve
    // M(i-1)+4Mi+M(i+1)=6(P(i+1)-2Pi+P(i-1)) on uniform intervals.
    let interior = points.len() - 2;
    let mut diagonal = vec![4.0; interior];
    let mut rhs: Vec<_> = points.windows(3).map(|p| (p[2] - p[1].scale(2.0) + p[0]).scale(6.0)).collect();
    for i in 1..interior {
        let factor = 1.0 / diagonal[i - 1];
        diagonal[i] -= factor;
        rhs[i] = rhs[i] - rhs[i - 1].scale(factor);
    }
    let mut second = vec![P2::ZERO; points.len()];
    for i in (0..interior).rev() {
        second[i + 1] = (rhs[i] - second[i + 2]).scale(1.0 / diagonal[i]);
    }
    points
        .windows(2)
        .enumerate()
        .map(|(i, p)| {
            let chord = p[1] - p[0];
            let first = chord - (second[i].scale(2.0) + second[i + 1]).scale(1.0 / 6.0);
            let last = chord + (second[i] + second[i + 1].scale(2.0)).scale(1.0 / 6.0);
            BezierSpan { controls: vec![p[0], p[0] + first.scale(1.0 / 3.0), p[1] - last.scale(1.0 / 3.0), p[1]] }
        })
        .collect()
}

fn knots(count: usize, degree: usize) -> Vec<f64> {
    let end = count - degree;
    std::iter::repeat_n(0.0, degree + 1)
        .chain((1..end).map(|i| i as f64))
        .chain(std::iter::repeat_n(end as f64, degree + 1))
        .collect()
}

fn de_boor(points: &[P2], knots: &[f64], degree: usize, t: f64) -> P2 {
    let span = (degree..points.len()).find(|&i| t < knots[i + 1]).unwrap_or(points.len() - 1);
    let mut row = points[span - degree..=span].to_vec();
    for step in 1..=degree {
        for j in (step..=degree).rev() {
            let i = span - degree + j;
            let alpha = (t - knots[i]) / (knots[i + degree + 1 - step] - knots[i]);
            row[j] = row[j - 1].lerp(row[j], alpha);
        }
    }
    row[degree]
}

fn control_spans(points: &[P2]) -> Vec<BezierSpan> {
    let degree = 3.min(points.len() - 1);
    if degree == 1 {
        return vec![BezierSpan { controls: points.to_vec() }];
    }
    let knots = knots(points.len(), degree);
    let derivatives: Vec<_> = points
        .windows(2)
        .enumerate()
        .map(|(i, p)| (p[1] - p[0]).scale(degree as f64 / (knots[i + degree + 1] - knots[i + 1])))
        .collect();
    let tangent = |t| de_boor(&derivatives, &knots[1..knots.len() - 1], degree - 1, t);
    (0..points.len() - degree)
        .map(|i| {
            let a = de_boor(points, &knots, degree, i as f64);
            let b = de_boor(points, &knots, degree, (i + 1) as f64);
            let mut controls = vec![a, a + tangent(i as f64).scale(1.0 / degree as f64)];
            if degree == 3 {
                controls.push(b - tangent((i + 1) as f64).scale(1.0 / 3.0));
            }
            controls.push(b);
            BezierSpan { controls }
        })
        .collect()
}

/// Closest point on a spline, with the local span parameter used by the solver.
#[derive(Clone, Copy, Debug)]
pub struct Nearest {
    /// Polynomial span index.
    pub span: usize,
    /// Parameter in the span's closed unit interval.
    pub parameter: f64,
    /// Position on the evaluated curve.
    pub point: P2,
}

/// Middle parameter of the complete spline, for relation glyph placement.
pub fn midpoint(mode: SplineMode, points: &[P2]) -> P2 {
    let Some(spans) = spans(mode, points) else {
        return points.first().copied().unwrap_or(P2::ZERO);
    };
    let parameter = spans.len() as f64 / 2.0;
    let index = (parameter.floor() as usize).min(spans.len() - 1);
    spans[index].at(parameter - index as f64)
}

/// Drawing samples. Kernel profiles retain the polynomial controls instead.
pub fn polyline(mode: SplineMode, points: &[P2], segments: usize) -> Vec<P2> {
    let Some(spans) = spans(mode, points) else {
        return points.to_vec();
    };
    let count = segments.div_ceil(spans.len()).max(4);
    let mut result = Vec::new();
    for span in &spans {
        result.extend((0..count).map(|i| span.at(i as f64 / count as f64)));
    }
    result.push(spans.last().unwrap().at(1.0));
    result
}

fn choose(n: usize, k: usize) -> f64 {
    (0..k).fold(1.0, |value, i| value * (n - i) as f64 / (i + 1) as f64)
}
fn split_coefficients(coefficients: &[f64]) -> [Vec<f64>; 2] {
    let mut row = coefficients.to_vec();
    let mut left = vec![row[0]];
    let mut right = vec![*row.last().unwrap()];
    for count in (1..row.len()).rev() {
        for i in 0..count {
            row[i] = (row[i] + row[i + 1]) * 0.5;
        }
        left.push(row[0]);
        right.push(row[count - 1]);
    }
    right.reverse();
    [left, right]
}

/// Finds stationary distances by subdividing their Bernstein polynomial.
/// This numerical query seeds picking and solver parameters; it does not build solids.
pub fn nearest(mode: SplineMode, points: &[P2], target: P2) -> Nearest {
    let Some(spans) = spans(mode, points) else {
        return Nearest { span: 0, parameter: 0.0, point: points.first().copied().unwrap_or(P2::ZERO) };
    };
    nearest_spans(&spans, target)
}

pub(crate) fn nearest_spans(spans: &[BezierSpan], target: P2) -> Nearest {
    let fallback = spans.first().and_then(|span| span.controls.first()).copied().unwrap_or(P2::ZERO);
    let mut best = Nearest { span: 0, parameter: 0.0, point: fallback };
    if !target.u.is_finite() || !target.v.is_finite() {
        return best;
    }
    let mut distance = f64::INFINITY;
    for (index, span) in spans.iter().enumerate() {
        let mut candidate = |t| {
            let point = span.at(t);
            let d = (point - target).dot(point - target);
            if d < distance {
                distance = d;
                best = Nearest { span: index, parameter: t, point };
            }
        };
        candidate(0.0);
        candidate(1.0);
        let degree = span.controls.len() - 1;
        let mut coefficients = vec![0.0; 2 * degree];
        for i in 0..=degree {
            for j in 0..degree {
                let k = i + j;
                coefficients[k] += (span.controls[i] - target)
                    .dot((span.controls[j + 1] - span.controls[j]).scale(degree as f64))
                    * choose(degree, i)
                    * choose(degree - 1, j)
                    / choose(2 * degree - 1, k);
            }
        }
        if coefficients.iter().any(|value| !value.is_finite()) {
            continue;
        }
        let mut pending = vec![(coefficients, 0.0, 1.0, 0)];
        let mut cells = 0;
        while let Some((coefficients, lo, hi, depth)) = pending.pop() {
            cells += 1;
            if cells > 4096 {
                break;
            }
            if coefficients.iter().all(|v| *v >= 0.0) || coefficients.iter().all(|v| *v <= 0.0) {
                continue;
            }
            let mid = (lo + hi) * 0.5;
            candidate(mid);
            if depth >= 40 {
                continue;
            }
            let [left, right] = split_coefficients(&coefficients);
            pending.push((right, mid, hi, depth + 1));
            pending.push((left, lo, mid, depth + 1));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: P2, b: P2) {
        assert!(a.dist(b) < 1e-9, "{a:?} != {b:?}");
    }
    fn points() -> Vec<P2> {
        vec![
            P2::new(0.0, 0.0),
            P2::new(1.0, 3.0),
            P2::new(3.0, -1.0),
            P2::new(5.0, 4.0),
            P2::new(7.0, 2.0),
            P2::new(8.0, 0.0),
        ]
    }
    #[test]
    fn natural_fit_interpolates_and_has_two_continuous_derivatives() {
        let p = points();
        let spans = spans(SplineMode::Fit, &p).unwrap();
        assert_eq!(spans.len(), p.len() - 1);
        for (span, pair) in spans.iter().zip(p.windows(2)) {
            close(span.at(0.0), pair[0]);
            close(span.at(1.0), pair[1]);
        }
        close(spans[0].second_derivative(0.0), P2::ZERO);
        close(spans.last().unwrap().second_derivative(1.0), P2::ZERO);
        for pair in spans.windows(2) {
            close(pair[0].derivative(1.0), pair[1].derivative(0.0));
            close(pair[0].second_derivative(1.0), pair[1].second_derivative(0.0));
        }
    }
    #[test]
    fn control_spans_agree_with_independent_cox_de_boor_basis() {
        fn basis(i: usize, degree: usize, t: f64, k: &[f64]) -> f64 {
            if degree == 0 {
                return if k[i] <= t && t < k[i + 1] { 1.0 } else { 0.0 };
            }
            let a = k[i + degree] - k[i];
            let b = k[i + degree + 1] - k[i + 1];
            (if a > 0.0 { (t - k[i]) / a * basis(i, degree - 1, t, k) } else { 0.0 })
                + (if b > 0.0 { (k[i + degree + 1] - t) / b * basis(i + 1, degree - 1, t, k) } else { 0.0 })
        }
        for count in 2..=6 {
            let p = points()[..count].to_vec();
            let degree = 3.min(count - 1);
            let k = knots(count, degree);
            let spans = spans(SplineMode::Control, &p).unwrap();
            close(spans[0].at(0.0), p[0]);
            close(spans.last().unwrap().at(1.0), *p.last().unwrap());
            for (i, span) in spans.iter().enumerate() {
                for j in 0..20 {
                    let t = j as f64 / 20.0;
                    let expected = p
                        .iter()
                        .enumerate()
                        .fold(P2::ZERO, |sum, (n, p)| sum + p.scale(basis(n, degree, i as f64 + t, &k)));
                    close(span.at(t), expected);
                }
            }
        }
    }
    #[test]
    fn subdivision_and_reversal_preserve_the_curve() {
        for mode in [SplineMode::Fit, SplineMode::Control] {
            let p = points();
            let mut reverse = p.clone();
            reverse.reverse();
            let original = spans(mode, &p).unwrap();
            let reversed = spans(mode, &reverse).unwrap();
            for (a, b) in original.iter().zip(reversed.iter().rev()) {
                let halves = a.split();
                for i in 0..=20 {
                    let t = i as f64 / 20.0;
                    close(a.at(t), b.at(1.0 - t));
                    let (half, local) = if t <= 0.5 { (&halves[0], t * 2.0) } else { (&halves[1], t * 2.0 - 1.0) };
                    close(a.at(t), half.at(local));
                }
            }
        }
    }
    #[test]
    fn point_indices_preserve_large_authored_control_polygons() {
        use crate::{Entity, EntityKind, PointRef};
        let points: Vec<_> = (0..520).map(|i| P2::new(i as f64, (i % 7) as f64)).collect();
        let mut entity = Entity {
            id: 17,
            kind: EntityKind::Spline { mode: SplineMode::Control, points: points.clone() },
            construction: false,
        };
        assert_eq!(entity.point_count(), 520);
        let reference = PointRef::new(17, 519);
        entity.set_point(reference.index, P2::new(1000.0, 2.0));
        assert_eq!(entity.point(reference.index), Some(P2::new(1000.0, 2.0)));
        let copy: Entity = serde_json::from_str(&serde_json::to_string(&entity).unwrap()).unwrap();
        assert_eq!(entity, copy);
        assert_eq!(copy.point(518), Some(points[518]));
    }
    #[test]
    fn stationary_distance_picking_reaches_apices_and_finite_endpoints() {
        let points = [P2::ZERO, P2::new(0.0, 4.0), P2::new(4.0, 4.0), P2::new(4.0, 0.0)];
        let apex = nearest(SplineMode::Control, &points, P2::new(2.0, 4.0));
        assert!(apex.point.dist(P2::new(2.0, 3.0)) < 1e-9);
        assert!((apex.parameter - 0.5).abs() < 1e-9);
        let endpoint = nearest(SplineMode::Control, &points, P2::new(-1.0, -1.0));
        assert_eq!(endpoint.parameter, 0.0);
        assert_eq!(endpoint.point, P2::ZERO);
    }
    #[test]
    fn numerical_picking_bounds_invalid_targets_and_overflowing_distances() {
        let points = [P2::ZERO, P2::new(1.0, 4.0), P2::new(3.0, 4.0), P2::new(4.0, 0.0)];
        for mode in [SplineMode::Fit, SplineMode::Control] {
            for target in [P2::new(f64::NAN, 0.0), P2::new(0.0, f64::INFINITY)] {
                assert_eq!(nearest(mode, &points, target).point, P2::ZERO);
            }
        }
        let huge = [P2::new(-f64::MAX, 0.0), P2::new(f64::MAX, 0.0)];
        let result = nearest(SplineMode::Control, &huge, P2::ZERO);
        assert!(result.point.u.is_finite() && result.point.v.is_finite());
    }
    #[test]
    fn unsupported_spline_edits_refuse_by_name_without_changing_the_sketch() {
        use crate::{
            EntityKind, Sketch,
            tools::{ChamferSize, End, Operation, TrimOptions},
        };
        let mut sketch = Sketch::new();
        let curve = sketch.add(
            EntityKind::Spline {
                mode: SplineMode::Control,
                points: vec![P2::ZERO, P2::new(1.0, 4.0), P2::new(3.0, 4.0), P2::new(4.0, 0.0)],
            },
            false,
        );
        let line = sketch.add(EntityKind::Line { a: P2::new(-1.0, 1.0), b: P2::new(5.0, 1.0) }, false);
        for operation in [
            Operation::Trim { entity: curve, at: P2::new(2.0, 3.0), options: TrimOptions::default() },
            Operation::Trim { entity: line, at: P2::new(2.0, 1.0), options: TrimOptions::default() },
            Operation::TrimCorner {
                a: curve,
                b: line,
                keep_a: P2::new(2.0, 3.0),
                keep_b: P2::new(4.0, 1.0),
                options: TrimOptions::default(),
            },
            Operation::Extend { entity: line, end: End::End, boundary: curve },
            Operation::Offset { entities: vec![curve], distance: 1.0, options: Default::default() },
            Operation::Fillet { a: curve, b: line, keep_a: P2::ZERO, keep_b: P2::new(4.0, 1.0), radius: 1.0 },
            Operation::Chamfer {
                a: curve,
                b: line,
                keep_a: P2::ZERO,
                keep_b: P2::new(4.0, 1.0),
                size: ChamferSize::Distances { first: 1.0, second: 1.0 },
            },
            Operation::Split { entity: curve, at: P2::new(2.0, 3.0), second: None },
        ] {
            let before = sketch.clone();
            let error = sketch.apply_operation(&operation).unwrap_err();
            assert!(error.0.contains("spline"), "{error:?}");
            assert_eq!(sketch, before);
        }
        sketch
            .apply_operation(&Operation::Offset { entities: vec![line], distance: 1.0, options: Default::default() })
            .unwrap();
        let group = sketch
            .constraints
            .iter()
            .find_map(|c| match c.kind {
                crate::ConstraintKind::Offset { group, .. } => Some(group),
                _ => None,
            })
            .unwrap();
        sketch.entity_mut(line).unwrap().kind = sketch.entity(curve).unwrap().kind.clone();
        let before = sketch.clone();
        let error = sketch.apply_operation(&Operation::EditOffset { group, distance: 2.0 }).unwrap_err();
        assert!(error.0.contains("spline"), "{error:?}");
        assert_eq!(sketch, before);
    }
}
