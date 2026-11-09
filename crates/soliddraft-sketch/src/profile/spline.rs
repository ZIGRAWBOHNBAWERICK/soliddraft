use super::*;
use crate::spline::{BezierSpan, spans};
use soliddraft_kernel_api::BezierControls;

pub(super) fn span(from: Point2, controls: BezierControls, to: Point2) -> BezierSpan {
    let mut points = vec![p2(from)];
    points.extend(controls.as_slice().iter().copied().map(p2));
    points.push(p2(to));
    BezierSpan { controls: points }
}
pub(super) fn reverse(from: Point2, controls: BezierControls, to: Point2) -> Segment {
    let controls = match controls {
        BezierControls::Quadratic(p) => BezierControls::Quadratic(p),
        BezierControls::Cubic([a, b]) => BezierControls::Cubic([b, a]),
    };
    Segment::Bezier { from: to, controls, to: from }
}

pub(super) fn segments(entity: &Entity) -> Option<Vec<Segment>> {
    let EntityKind::Spline { mode, ref points } = entity.kind else {
        return None;
    };
    let mut pending = spans(mode, points)?.into_iter().rev().map(|span| (span, 0)).collect::<Vec<_>>();
    let mut result = Vec::new();
    while let Some((span, depth)) = pending.pop() {
        let points = &span.controls;
        let chord = *points.last().unwrap() - points[0];
        let regular = chord.len() > JOIN_TOLERANCE
            && (points[1] - points[0]).len() > 1e-12
            && (*points.last().unwrap() - points[points.len() - 2]).len() > 1e-12
            && points.windows(2).all(|p| (p[1] - p[0]).dot(chord) >= 0.0);
        if !regular {
            if depth >= 20 || pending.len() + result.len() >= 4096 {
                return None;
            }
            let [left, right] = span.split();
            pending.push((right, depth + 1));
            pending.push((left, depth + 1));
            continue;
        }
        result.push(match points.as_slice() {
            [from, to] => Segment::Line { from: pt(*from), to: pt(*to) },
            [from, control, to] => {
                Segment::Bezier { from: pt(*from), controls: BezierControls::Quadratic([pt(*control)]), to: pt(*to) }
            }
            [from, first, second, to] => Segment::Bezier {
                from: pt(*from),
                controls: BezierControls::Cubic([pt(*first), pt(*second)]),
                to: pt(*to),
            },
            _ => unreachable!("polynomial span degree"),
        });
    }
    Some(result)
}

pub(super) fn append(result: &mut RegionLoop, edge: &Edge, forward: bool, from: P2, to: P2) {
    let source = edge.spline.as_ref().unwrap();
    let mut segments = source.clone();
    if !forward {
        segments.reverse();
        for segment in &mut segments {
            *segment = match *segment {
                Segment::Bezier { from, controls, to } => reverse(from, controls, to),
                Segment::Line { from, to } => Segment::Line { from: to, to: from },
                _ => unreachable!("spline segments"),
            };
        }
    }
    match segments.first_mut().unwrap() {
        Segment::Bezier { from: first, .. } | Segment::Line { from: first, .. } => *first = pt(from),
        _ => unreachable!("spline segments"),
    }
    match segments.last_mut().unwrap() {
        Segment::Bezier { to: last, .. } | Segment::Line { to: last, .. } => *last = pt(to),
        _ => unreachable!("spline segments"),
    }
    result.entities.extend(std::iter::repeat_n(edge.entity, segments.len()));
    result.segments.extend(segments);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SplineMode;
    fn arch(sketch: &mut Sketch, controls: Vec<P2>) -> EntityId {
        let from = controls[0];
        let to = *controls.last().unwrap();
        let curve = sketch.add(EntityKind::Spline { mode: SplineMode::Control, points: controls }, false);
        sketch.add(EntityKind::Line { a: to, b: from }, false);
        curve
    }
    #[test]
    fn polynomial_profiles_keep_controls_area_and_span_provenance() {
        let mut sketch = Sketch::new();
        let curve = arch(&mut sketch, vec![P2::ZERO, P2::new(0.0, 4.0), P2::new(4.0, 4.0), P2::new(4.0, 0.0)]);
        let regions = sketch.regions().unwrap();
        assert_eq!(regions.len(), 1);
        assert!((regions[0].area() - 48.0 / 5.0).abs() < 1e-10);
        let index = regions[0].outer.entities.iter().position(|id| *id == curve).unwrap();
        let Segment::Bezier { controls: BezierControls::Cubic(controls), .. } = regions[0].outer.segments[index] else {
            panic!("polynomial controls expected");
        };
        assert_eq!(controls, [Point2::new(4.0, 4.0), Point2::new(0.0, 4.0)]);
        regions[0].to_profile(Plane::XY).check().unwrap();
        let mut sketch = Sketch::new();
        let curve = arch(
            &mut sketch,
            vec![P2::ZERO, P2::new(0.0, 3.0), P2::new(2.0, 4.0), P2::new(4.0, 3.0), P2::new(4.0, 0.0)],
        );
        let regions = sketch.regions().unwrap();
        let source = &regions[0].outer;
        assert!(source.entities.iter().filter(|id| **id == curve).count() >= 2);
        assert_eq!(source.entities.len(), source.segments.len());
        regions[0].to_profile(Plane::XY).check().unwrap();
    }
    #[test]
    fn closed_spline_entities_form_a_loop_in_both_modes() {
        for mode in [SplineMode::Fit, SplineMode::Control] {
            let mut sketch = Sketch::new();
            let curve = sketch.add(
                EntityKind::Spline {
                    mode,
                    points: vec![P2::ZERO, P2::new(4.0, 0.0), P2::new(4.0, 3.0), P2::new(0.0, 3.0), P2::ZERO],
                },
                false,
            );
            let regions = sketch.regions().unwrap();
            assert_eq!(regions.len(), 1, "{mode:?}");
            assert!(regions[0].area() > 1.0);
            assert!(regions[0].outer.entities.iter().all(|id| *id == curve));
            regions[0].to_profile(Plane::XY).check().unwrap();
        }
    }
    #[test]
    fn nesting_uses_the_polynomial_curve_close_to_its_apex() {
        let mut sketch = Sketch::new();
        arch(&mut sketch, vec![P2::ZERO, P2::new(0.0, 4.0), P2::new(4.0, 4.0), P2::new(4.0, 0.0)]);
        sketch.add(EntityKind::Circle { center: P2::new(2.0, 2.998), radius: 0.0002 }, false);
        let regions = sketch.regions().unwrap();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].holes.len(), 1);
        assert!((regions[0].area() - (48.0 / 5.0 - PI * 0.0002_f64.powi(2))).abs() < 1e-10);
    }
}
