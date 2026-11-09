//! Spline interpolation expressed through ezpz's coordinate-sharing API.

use super::*;

#[derive(Clone)]
pub(super) struct SplineDatum {
    mode: crate::spline::SplineMode,
    points: Vec<P2>,
    spans: Vec<Vec<(DatumPoint, P2)>>,
    interpolation: Interpolation,
}
impl SplineDatum {
    pub fn tangents(&self) -> [DatumLineSegment; 2] {
        let first = &self.spans[0];
        let last = self.spans.last().unwrap();
        [
            DatumLineSegment::new(first[0].0, first[1].0),
            DatumLineSegment::new(last[last.len() - 2].0, last.last().unwrap().0),
        ]
    }
    pub fn point_on(&self, vars: &mut Vars, out: DatumPoint, at: P2, requests: &mut Vec<Constraint>) -> Attachment {
        let nearest = crate::spline::nearest(self.mode, &self.points, at);
        let parameter = scalar(vars, nearest.parameter);
        let mut attachment =
            self.interpolation.point_on(vars, &self.spans[nearest.span], out, parameter, nearest.parameter, requests);
        attachment.spans = self.spans.iter().map(|span| span.iter().map(|p| p.0).collect()).collect();
        attachment.selected = nearest.span;
        attachment
    }
}

pub(super) fn datum(vars: &mut Vars, entity: &Entity) -> Option<(SplineDatum, Vec<Constraint>)> {
    use crate::spline::SplineMode;
    let EntityKind::Spline { mode, ref points } = entity.kind else {
        return None;
    };
    let geometry = crate::spline::spans(mode, points)?;
    let authored: Vec<_> =
        points.iter().enumerate().map(|(index, p)| (vars.point(PointRef::new(entity.id, index), *p), *p)).collect();
    let mut requests = Vec::new();
    let algebra = Interpolation::new(vars, &mut requests);
    let spans = if mode == SplineMode::Fit && points.len() > 2 {
        let spans: Vec<Vec<_>> = geometry
            .iter()
            .enumerate()
            .map(|(i, span)| {
                vec![
                    authored[i],
                    (auxiliary(vars, span.controls[1]), span.controls[1]),
                    (auxiliary(vars, span.controls[2]), span.controls[2]),
                    authored[i + 1],
                ]
            })
            .collect();
        requests.push(Constraint::Midpoint(DatumLineSegment::new(spans[0][0].0, spans[0][2].0), spans[0][1].0));
        let last = spans.last().unwrap();
        requests.push(Constraint::Midpoint(DatumLineSegment::new(last[1].0, last[3].0), last[2].0));
        let third = scalar(vars, 1.0 / 3.0);
        requests.push(Constraint::Fixed(third, 1.0 / 3.0));
        for pair in spans.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            requests.push(Constraint::Midpoint(DatumLineSegment::new(a[2].0, b[1].0), a[3].0));
            let q = auxiliary(vars, b[1].1.lerp(a[1].1, 1.0 / 3.0));
            requests.extend(algebra.lerp(b[1].0, a[1].0, q, third));
            requests.extend(algebra.lerp(a[2].0, b[2].0, q, third));
        }
        spans
    } else {
        let degree = 3.min(points.len() - 1);
        let end = points.len() - degree;
        let mut knots: Vec<f64> = std::iter::repeat_n(0.0, degree + 1)
            .chain((1..end).map(|i| i as f64))
            .chain(std::iter::repeat_n(end as f64, degree + 1))
            .collect();
        let mut net = authored;
        for knot in 1..end {
            let t = knot as f64;
            while knots.iter().filter(|v| **v == t).count() < degree {
                let k = (degree..net.len()).find(|&i| t < knots[i + 1]).unwrap();
                let multiplicity = knots.iter().filter(|v| **v == t).count();
                let mut next = vec![net[0]; net.len() + 1];
                next[..=k - degree].copy_from_slice(&net[..=k - degree]);
                next[k - multiplicity + 1..].copy_from_slice(&net[k - multiplicity..]);
                for i in k - degree + 1..=k - multiplicity {
                    let alpha = (t - knots[i]) / (knots[i + degree] - knots[i]);
                    let parameter = scalar(vars, alpha);
                    requests.push(Constraint::Fixed(parameter, alpha));
                    let at = net[i - 1].1.lerp(net[i].1, alpha);
                    let point = auxiliary(vars, at);
                    requests.extend(algebra.lerp(net[i - 1].0, net[i].0, point, parameter));
                    next[i] = (point, at);
                }
                net = next;
                knots.insert(k + 1, t);
            }
        }
        (0..end).map(|i| net[i * degree..=i * degree + degree].to_vec()).collect()
    };
    Some((SplineDatum { mode, points: points.clone(), spans, interpolation: algebra }, requests))
}

#[derive(Clone, Copy)]
struct Interpolation {
    zero: ezpz::Id,
    one: ezpz::Id,
}
impl Interpolation {
    fn new(vars: &mut Vars, requests: &mut Vec<Constraint>) -> Self {
        let zero = scalar(vars, 0.0);
        let one = scalar(vars, 1.0);
        requests.extend([Constraint::Fixed(zero, 0.0), Constraint::Fixed(one, 1.0)]);
        Self { zero, one }
    }

    fn lerp(&self, a: DatumPoint, b: DatumPoint, out: DatumPoint, t: ezpz::Id) -> [Constraint; 2] {
        [a.id_x(), a.id_y()]
            .into_iter()
            .zip([b.id_x(), b.id_y()])
            .zip([out.id_x(), out.id_y()])
            .map(|((a, b), out)| {
                // With virtual y coordinates 0, 1 and t, the horizontal
                // distance is exactly out-a-(b-a)*t. The denominator stays 1.
                Constraint::HorizontalPointLineDistance(
                    DatumPoint::new_xy(out, t),
                    DatumLineSegment::new(DatumPoint::new_xy(a, self.zero), DatumPoint::new_xy(b, self.one)),
                    0.0,
                )
            })
            .collect::<Vec<_>>()
            .try_into()
            .unwrap()
    }

    fn point_on(
        &self,
        vars: &mut Vars,
        controls: &[(DatumPoint, P2)],
        out: DatumPoint,
        t: ezpz::Id,
        initial: f64,
        requests: &mut Vec<Constraint>,
    ) -> Attachment {
        let request_start = requests.len();
        let mut graph = Vec::new();
        let mut row = controls.to_vec();
        for count in (1..row.len()).rev() {
            for i in 0..count {
                let at = row[i].1.lerp(row[i + 1].1, initial);
                let next = if count == 1 { out } else { auxiliary(vars, at) };
                graph.push([row[i].0, row[i + 1].0, next]);
                requests.extend(self.lerp(row[i].0, row[i + 1].0, next, t));
                row[i] = (next, at);
            }
        }
        Attachment {
            parameter: t,
            graph,
            algebra: *self,
            spans: vec![controls.iter().map(|p| p.0).collect()],
            selected: 0,
            request_start,
        }
    }
}

pub(super) struct Attachment {
    parameter: ezpz::Id,
    graph: Vec<[DatumPoint; 3]>,
    algebra: Interpolation,
    spans: Vec<Vec<DatumPoint>>,
    selected: usize,
    request_start: usize,
}
impl Attachment {
    pub fn offset_requests(&mut self, offset: usize) {
        self.request_start += offset;
    }
    fn nearest(&self, values: &[f64]) -> crate::spline::Nearest {
        let at = |point: DatumPoint| P2::new(values[point.id_x() as usize], values[point.id_y() as usize]);
        let spans: Vec<_> = self
            .spans
            .iter()
            .map(|points| crate::spline::BezierSpan { controls: points.iter().map(|p| at(*p)).collect() })
            .collect();
        crate::spline::nearest_spans(&spans, at(self.graph.last().unwrap()[2]))
    }
    fn select(&mut self, index: usize, requests: &mut [ConstraintRequest]) {
        let mut row = self.spans[index].clone();
        let mut slot = 0;
        for count in (1..row.len()).rev() {
            for i in 0..count {
                let out = self.graph[slot][2];
                self.graph[slot] = [row[i], row[i + 1], out];
                let [first, second] = self.algebra.lerp(row[i], row[i + 1], out, self.parameter);
                requests[self.request_start + slot * 2] = ConstraintRequest::highest_priority(first);
                requests[self.request_start + slot * 2 + 1] = ConstraintRequest::highest_priority(second);
                row[i] = out;
                slot += 1;
            }
        }
        self.selected = index;
    }
    fn contains(&self, values: &[f64]) -> bool {
        let t = values[self.parameter as usize];
        t.is_finite() && (-1e-10..=1.0 + 1e-10).contains(&t)
    }
    fn seed(&self, values: &mut [f64], t: f64) {
        values[self.parameter as usize] = t;
        for [a, b, out] in &self.graph {
            for (a, b, out) in [(a.id_x(), b.id_x(), out.id_x()), (a.id_y(), b.id_y(), out.id_y())] {
                values[out as usize] = values[a as usize] + (values[b as usize] - values[a as usize]) * t;
            }
        }
    }
}

pub(super) fn finite_solve(
    requests: &mut Vec<ConstraintRequest>,
    mut guesses: Vec<(ezpz::Id, f64)>,
    attachments: &mut [Attachment],
) -> Result<(ezpz::SolveOutcome, Vec<usize>), String> {
    let mut clamped = Vec::new();
    for attempt in 0..=4 {
        let result =
            ezpz::solve(requests, guesses, Config::default().with_max_iterations(60).with_convergence_tolerance(1e-11))
                .map_err(|error| error.error.to_string())?;
        let mut values = result.final_values().to_vec();
        if values.iter().any(|v| !v.is_finite()) {
            return Err("The solver returned non-finite spline coordinates".into());
        }
        let outside: Vec<_> =
            attachments.iter().enumerate().filter(|(_, a)| !a.contains(&values)).map(|(i, _)| i).collect();
        if outside.is_empty() {
            return Ok((result, clamped));
        }
        if attempt == 4 {
            return Err("The point-on-spline relation has no solved finite parameter".into());
        }
        for i in outside {
            let attachment = &mut attachments[i];
            let nearest = attachment.nearest(&values);
            let seed = if attempt < 3 && nearest.span != attachment.selected {
                attachment.select(nearest.span, requests);
                nearest.parameter
            } else if attempt < 3 {
                [0.5, 0.25, 0.75][attempt]
            } else {
                let endpoint = if values[attachment.parameter as usize] < 0.0 { 0.0 } else { 1.0 };
                requests.push(ConstraintRequest::highest_priority(Constraint::Fixed(attachment.parameter, endpoint)));
                clamped.push(i);
                endpoint
            };
            attachment.seed(&mut values, seed);
        }
        guesses = values.into_iter().enumerate().map(|(id, value)| (id as ezpz::Id, value)).collect();
    }
    unreachable!("bounded attempt count")
}

fn scalar(vars: &mut Vars, at: f64) -> ezpz::Id {
    let id = vars.ids.next_id();
    vars.guesses.push((id, at));
    id
}
fn auxiliary(vars: &mut Vars, at: P2) -> DatumPoint {
    let p = DatumPoint::new(&mut vars.ids);
    vars.guesses.extend([(p.id_x(), at.u), (p.id_y(), at.v)]);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vars() -> Vars {
        Vars { ids: ezpz::IdGenerator::default(), points: HashMap::new(), radii: HashMap::new(), guesses: Vec::new() }
    }
    fn run(vars: Vars, constraints: Vec<Constraint>, attachments: &mut [Attachment]) -> Vec<f64> {
        let mut requests: Vec<_> = constraints.into_iter().map(ConstraintRequest::highest_priority).collect();
        let (result, _) = finite_solve(&mut requests, vars.guesses, attachments).unwrap();
        assert!(result.converged(), "{:?}", result.unsatisfied());
        result.final_values().to_vec()
    }
    fn fix(requests: &mut Vec<Constraint>, point: DatumPoint, at: P2) {
        requests.extend([Constraint::Fixed(point.id_x(), at.u), Constraint::Fixed(point.id_y(), at.v)]);
    }
    #[test]
    fn interpolation_tracks_control_edits_and_stays_regular_at_endpoints() {
        for at in [0.0, 0.25, 0.5, 1.0] {
            for height in [4.0, 8.0] {
                let mut vars = vars();
                let mut requests = Vec::new();
                let algebra = Interpolation::new(&mut vars, &mut requests);
                let controls: Vec<_> = [P2::ZERO, P2::new(0.0, 4.0), P2::new(4.0, 4.0), P2::new(4.0, 0.0)]
                    .into_iter()
                    .map(|p| (auxiliary(&mut vars, p), p))
                    .collect();
                for (i, (point, p)) in controls.iter().enumerate() {
                    fix(&mut requests, *point, if i == 1 || i == 2 { P2::new(p.u, height) } else { *p });
                }
                let t = scalar(&mut vars, at);
                requests.push(Constraint::Fixed(t, at));
                let output = auxiliary(&mut vars, P2::ZERO);
                algebra.point_on(&mut vars, &controls, output, t, at, &mut requests);
                let values = run(vars, requests, &mut []);
                let expected = P2::new(12.0 * at * at - 8.0 * at * at * at, 3.0 * height * at * (1.0 - at));
                assert!(P2::new(values[output.id_x() as usize], values[output.id_y() as usize]).dist(expected) < 1e-9);
            }
        }
    }
    #[test]
    fn free_parameter_follows_a_point_drag_along_the_actual_curve() {
        for initial in [0.0, 0.2, 0.9, 1.0] {
            let mut vars = vars();
            let mut requests = Vec::new();
            let algebra = Interpolation::new(&mut vars, &mut requests);
            let controls: Vec<_> = [P2::ZERO, P2::new(0.0, 4.0), P2::new(4.0, 4.0), P2::new(4.0, 0.0)]
                .into_iter()
                .map(|p| {
                    let datum = auxiliary(&mut vars, p);
                    fix(&mut requests, datum, p);
                    (datum, p)
                })
                .collect();
            let t = scalar(&mut vars, initial);
            let output = auxiliary(&mut vars, P2::ZERO);
            let attachment = algebra.point_on(&mut vars, &controls, output, t, initial, &mut requests);
            requests.push(Constraint::Fixed(output.id_x(), 2.0));
            let values = run(vars, requests, &mut [attachment]);
            assert!((values[t as usize] - 0.5).abs() < 1e-9);
            assert!((values[output.id_y() as usize] - 3.0).abs() < 1e-9);
        }
    }
    #[test]
    fn spline_entities_follow_dimensions_and_reopen() {
        for mode in [crate::SplineMode::Fit, crate::SplineMode::Control] {
            let mut sketch = Sketch::new();
            let points = vec![
                P2::ZERO,
                P2::new(1.0, 3.0),
                P2::new(3.0, -1.0),
                P2::new(5.0, 4.0),
                P2::new(7.0, 2.0),
                P2::new(8.0, 0.0),
            ];
            let entity = sketch.add(EntityKind::Spline { mode, points: points.clone() }, false);
            for (i, p) in points.iter().enumerate().take(points.len() - 1) {
                sketch.constrain(ConstraintKind::Fix { point: PointRef::new(entity, i), at: *p });
            }
            let end = PointRef::new(entity, points.len() - 1);
            sketch.constrain(ConstraintKind::Horizontal { target: Align::Points(PointRef::ORIGIN, end) });
            let dimension = sketch.constrain(ConstraintKind::Distance {
                a: PointRef::ORIGIN,
                b: end,
                kind: DistanceKind::Horizontal,
                value: 12.0,
                label: P2::new(4.0, -2.0),
            });
            super::super::solve(&mut sketch, None, true);
            assert_eq!(sketch.status, SolveStatus::FullyDefined);
            assert!(sketch.point(end).unwrap().dist(P2::new(12.0, 0.0)) < 1e-9);
            let mut sketch: Sketch = serde_json::from_str(&serde_json::to_string(&sketch).unwrap()).unwrap();
            sketch.constraint_mut(dimension).unwrap().set_value(10.0);
            super::super::solve(&mut sketch, None, true);
            assert_eq!(sketch.status, SolveStatus::FullyDefined);
            assert!(sketch.point(end).unwrap().dist(P2::new(10.0, 0.0)) < 1e-9);
            assert_eq!(sketch.entity(entity).unwrap().point_count(), points.len());
        }
    }

    #[test]
    fn fixed_spline_keeps_point_on_the_authored_curve_and_refuses_extension() {
        let mut sketch = Sketch::new();
        let kind = EntityKind::Spline {
            mode: crate::SplineMode::Control,
            points: vec![P2::ZERO, P2::new(1.0, 4.0), P2::new(3.0, 4.0), P2::new(4.0, 0.0)],
        };
        let curve = sketch.add(kind.clone(), false);
        sketch.constrain(ConstraintKind::FixEntity { entity: curve, at: kind });
        let point = sketch.add(EntityKind::Point { p: P2::new(1.0, 2.0) }, true);
        let point = PointRef::new(point, 0);
        sketch.constrain(ConstraintKind::PointOn { point, entity: curve });
        let dimension = sketch.constrain(ConstraintKind::Distance {
            a: PointRef::ORIGIN,
            b: point,
            kind: DistanceKind::Horizontal,
            value: 2.0,
            label: P2::ZERO,
        });
        super::super::solve(&mut sketch, None, true);
        assert_eq!(sketch.status, SolveStatus::FullyDefined);
        assert!(sketch.point(point).unwrap().dist(P2::new(2.0, 3.0)) < 1e-8);
        sketch.constraint_mut(dimension).unwrap().set_value(5.0);
        super::super::solve(&mut sketch, None, true);
        assert!(matches!(sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }));
    }
    #[test]
    fn point_on_switches_spans_when_a_dimension_moves_along_the_curve() {
        for mode in [crate::SplineMode::Fit, crate::SplineMode::Control] {
            let mut sketch = Sketch::new();
            let kind = EntityKind::Spline { mode, points: (0..6).map(|i| P2::new(i as f64, 0.0)).collect() };
            let curve = sketch.add(kind.clone(), false);
            sketch.constrain(ConstraintKind::FixEntity { entity: curve, at: kind });
            let point = PointRef::new(sketch.add(EntityKind::Point { p: P2::new(0.5, 0.0) }, true), 0);
            sketch.constrain(ConstraintKind::PointOn { point, entity: curve });
            let dimension = sketch.constrain(ConstraintKind::Distance {
                a: PointRef::ORIGIN,
                b: point,
                kind: DistanceKind::Horizontal,
                value: 0.5,
                label: P2::ZERO,
            });
            super::super::solve(&mut sketch, None, true);
            assert_eq!(sketch.status, SolveStatus::FullyDefined);
            for value in [4.5, 0.25, 3.5] {
                sketch.constraint_mut(dimension).unwrap().set_value(value);
                super::super::solve(&mut sketch, None, true);
                assert_eq!(sketch.status, SolveStatus::FullyDefined, "{mode:?}, {value}");
                assert!(sketch.point(point).unwrap().dist(P2::new(value, 0.0)) < 1e-9);
            }
        }
    }

    #[test]
    fn endpoint_tangency_uses_the_solved_spline_derivative() {
        for mode in [crate::SplineMode::Fit, crate::SplineMode::Control] {
            let mut sketch = Sketch::new();
            let points = vec![P2::ZERO, P2::new(1.0, 3.0), P2::new(3.0, 1.0), P2::new(5.0, 2.0)];
            let end = *points.last().unwrap();
            let geometry = crate::spline::spans(mode, &points).unwrap();
            let derivative = geometry.last().unwrap().derivative(1.0).normalized();
            let kind = EntityKind::Spline { mode, points };
            let curve = sketch.add(kind.clone(), false);
            sketch.constrain(ConstraintKind::FixEntity { entity: curve, at: kind });
            let line =
                sketch.add(EntityKind::Line { a: end, b: end + derivative.scale(3.0) + P2::new(0.1, 0.1) }, false);
            sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(curve, 3), b: PointRef::new(line, 0) });
            sketch.constrain(ConstraintKind::Tangent { a: curve, b: line });
            sketch.constrain(ConstraintKind::Distance {
                a: PointRef::new(line, 0),
                b: PointRef::new(line, 1),
                kind: DistanceKind::Aligned,
                value: 3.0,
                label: P2::ZERO,
            });
            super::super::solve(&mut sketch, None, true);
            assert_eq!(sketch.status, SolveStatus::FullyDefined, "{mode:?}");
            let delta = sketch.point(PointRef::new(line, 1)).unwrap() - sketch.point(PointRef::new(line, 0)).unwrap();
            assert!(delta.cross(derivative).abs() < 1e-9);
            assert!((delta.len() - 3.0).abs() < 1e-9);
        }
    }
}
