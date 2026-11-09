//! Shared coordinate values implied by the sketch's saved point and alignment references.

use crate::{Align, ConstraintKind, P2, PointRef, Sketch, SolveStatus};
use std::collections::HashMap;

fn root(parent: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    let mut at = i;
    while parent[at] != at {
        let next = parent[at];
        parent[at] = r;
        at = next;
    }
    r
}

fn join(parent: &mut [usize], a: usize, b: usize) -> bool {
    let (a, b) = (root(parent, a), root(parent, b));
    parent[a.max(b)] = a.min(b);
    a != b
}

fn ends(target: Align, sketch: &Sketch) -> Option<(PointRef, PointRef)> {
    match target {
        Align::Points(a, b) => Some((a, b)),
        Align::Line(id) if sketch.entity(id)?.is_line() => Some((PointRef::new(id, 0), PointRef::new(id, 1))),
        _ => None,
    }
}

/// Resolve equality references, without a proximity test or a change to free coordinates.
pub(super) fn resolve(sketch: &mut Sketch) {
    if matches!(sketch.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }) {
        return;
    }
    let mut points: Vec<_> =
        sketch.entities.iter().flat_map(|e| (0..e.point_count()).map(move |i| PointRef::new(e.id, i))).collect();
    points.sort_by_key(|p| (p.entity, p.index));
    let index: HashMap<_, _> = points.iter().enumerate().map(|(i, &p)| (p, 2 * i)).collect();
    let values: Vec<_> = points
        .iter()
        .flat_map(|&p| {
            let p = sketch.point(p).expect("entity point");
            [p.u, p.v]
        })
        .collect();
    let mut parent: Vec<_> = (0..values.len()).collect();
    let link = |parent: &mut [usize], a: PointRef, b: PointRef, axis| match (index.get(&a), index.get(&b)) {
        (Some(&a), Some(&b)) => join(parent, a + axis, b + axis),
        _ => false,
    };
    for c in &sketch.constraints {
        match c.kind {
            ConstraintKind::Coincident { a, b } => {
                for axis in 0..2 {
                    link(&mut parent, a, b, axis);
                }
            }
            ConstraintKind::OffsetCap { cap, a, b, .. } => {
                let arc = sketch.entity(cap).is_some_and(|e| matches!(e.kind, crate::EntityKind::Arc { .. }));
                for (endpoint, point) in [a, b].into_iter().enumerate() {
                    let end = PointRef::new(cap, u8::from(arc) + endpoint as u8);
                    for axis in 0..2 {
                        link(&mut parent, end, point, axis);
                    }
                }
            }
            ConstraintKind::Horizontal { target } | ConstraintKind::Vertical { target } => {
                if let Some((a, b)) = ends(target, sketch) {
                    let axis = usize::from(matches!(c.kind, ConstraintKind::Horizontal { .. }));
                    link(&mut parent, a, b, axis);
                }
            }
            ConstraintKind::PointOn { point, entity } if sketch.entity(entity).is_some_and(|e| e.is_point()) => {
                for axis in 0..2 {
                    link(&mut parent, point, PointRef::new(entity, 0), axis);
                }
            }
            _ => {}
        }
    }
    // An on-edge or midpoint reference inherits the edge's constant coordinate.
    loop {
        let mut changed = false;
        for c in &sketch.constraints {
            let (point, line) = match c.kind {
                ConstraintKind::PointOn { point, entity } => (point, entity),
                ConstraintKind::Midpoint { point, line } => (point, line),
                _ => continue,
            };
            let Some((a, b)) = ends(Align::Line(line), sketch) else { continue };
            let (Some(&ia), Some(&ib)) = (index.get(&a), index.get(&b)) else { continue };
            for axis in 0..2 {
                if root(&mut parent, ia + axis) == root(&mut parent, ib + axis)
                    || values[ia + axis] == values[ib + axis]
                {
                    changed |= link(&mut parent, a, b, axis);
                    changed |= link(&mut parent, point, a, axis);
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut fixed = vec![None; values.len()];
    let mut conflict = vec![false; values.len()];
    let anchors = std::iter::once((PointRef::ORIGIN, P2::ZERO)).chain(
        sketch
            .constraints
            .iter()
            .filter_map(|c| if let ConstraintKind::Fix { point, at } = c.kind { Some((point, at)) } else { None }),
    );
    for (point, at) in anchors {
        let Some(&i) = index.get(&point) else { continue };
        for (axis, value) in [at.u, at.v].into_iter().enumerate() {
            let r = root(&mut parent, i + axis);
            conflict[r] |= !value.is_finite() || fixed[r].is_some_and(|old| old != value);
            fixed[r] = Some(value);
        }
    }
    for e in &mut sketch.entities {
        for p in 0..e.point_count() {
            let i = index[&PointRef::new(e.id, p)];
            let coordinates = std::array::from_fn::<_, 2, _>(|axis| {
                let r = root(&mut parent, i + axis);
                if conflict[r] { values[i + axis] } else { fixed[r].unwrap_or(values[r]) }
            });
            e.set_point(p, P2::new(coordinates[0], coordinates[1]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntityKind;
    use soliddraft_kernel_api::{Plane, Segment};

    #[test]
    fn saved_point_and_frame_references_remove_plate_tilt() {
        let mut sketch = Sketch::new();
        let shape = sketch.rectangle(P2::new(60.0, 45.0), 120.0, 90.0).unwrap();
        sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(shape.entities[0], 0), b: PointRef::ORIGIN });
        for &id in &shape.entities[..4] {
            let e = sketch.entity_mut(id).unwrap();
            let EntityKind::Line { a, b } = &mut e.kind else { panic!() };
            if a.v == 0.0 {
                a.v = -2e-17;
            }
            if b.v == 0.0 {
                b.v = -4e-17;
            }
            if a.u == 0.0 {
                a.u = -2e-28;
            }
            if b.u == 0.0 {
                b.u = -4e-28;
            }
        }
        let saved = serde_json::to_string(&sketch).unwrap();
        let mut reopened: Sketch = serde_json::from_str(&saved).unwrap();
        reopened.entities.reverse();
        reopened.constraints.reverse();
        let before = reopened.clone();
        let profile = crate::profile::profiles(&reopened, Plane::XY).unwrap().remove(0);
        let expected = soliddraft_kernel_api::Profile::rectangle(
            Plane::XY,
            soliddraft_kernel_api::Point2::new(60.0, 45.0),
            120.0,
            90.0,
        );
        assert_eq!(profile, expected, "the saved references determine the exact plate boundary");
        assert_eq!(reopened, before, "profile extraction preserves the authored coordinates");
    }

    #[test]
    fn an_unconstrained_tiny_slope_is_preserved() {
        let mut sketch = Sketch::new();
        let corners = [P2::new(0.0, -2e-17), P2::new(120.0, -4e-17), P2::new(120.0, 90.0), P2::new(0.0, 90.0)];
        for i in 0..4 {
            sketch.add(EntityKind::Line { a: corners[i], b: corners[(i + 1) % 4] }, false);
        }
        let profile = crate::profile::profiles(&sketch, Plane::XY).unwrap().remove(0);
        let Segment::Line { from, to } = profile.outer.0[0] else { panic!() };
        assert_eq!((from.v, to.v), (-2e-17, -4e-17));
    }

    #[test]
    fn edge_and_midpoint_references_inherit_fixed_coordinates_transitively() {
        let mut sketch = Sketch::new();
        let line = sketch.add(EntityKind::Line { a: P2::new(0.0, -2e-17), b: P2::new(20.0, 4e-17) }, true);
        let on = sketch.add(EntityKind::Point { p: P2::new(3.0, 1e-17) }, true);
        let mid = sketch.add(EntityKind::Point { p: P2::new(10.0, 2e-17) }, true);
        let tail = sketch.add(EntityKind::Line { a: P2::new(3.0, 1e-17), b: P2::new(3.0, 8.0) }, false);
        let tail_mid = sketch.add(EntityKind::Point { p: P2::new(3.0 + 1e-15, 4.0) }, true);
        sketch.constrain(ConstraintKind::Midpoint { point: PointRef::new(tail_mid, 0), line: tail });
        sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(tail, 0), b: PointRef::new(on, 0) });
        sketch.constrain(ConstraintKind::Vertical { target: Align::Line(tail) });
        sketch.constrain(ConstraintKind::PointOn { point: PointRef::new(on, 0), entity: line });
        sketch.constrain(ConstraintKind::Midpoint { point: PointRef::new(mid, 0), line });
        sketch.constrain(ConstraintKind::Horizontal { target: Align::Line(line) });
        sketch.constrain(ConstraintKind::Fix { point: PointRef::new(line, 1), at: P2::new(20.0, 0.0) });
        resolve(&mut sketch);
        for r in [PointRef::new(line, 0), PointRef::new(line, 1), PointRef::new(on, 0), PointRef::new(mid, 0)] {
            assert_eq!(sketch.point(r).unwrap().v, 0.0);
        }
        assert_eq!(sketch.point(PointRef::new(on, 0)).unwrap().u, 3.0);
        assert_eq!(sketch.point(PointRef::new(tail_mid, 0)).unwrap(), P2::new(3.0, 4.0));
    }

    #[test]
    fn contradictory_anchors_are_not_overwritten() {
        let mut sketch = Sketch::new();
        let line = sketch.add(EntityKind::Line { a: P2::new(1.0, 2e-17), b: P2::new(4.0, 4e-17) }, false);
        sketch.constrain(ConstraintKind::Horizontal { target: Align::Line(line) });
        for (i, p) in [(0, P2::new(1.0, 0.0)), (1, P2::new(4.0, 1e-17))] {
            sketch.constrain(ConstraintKind::Fix { point: PointRef::new(line, i), at: p });
        }
        let before = sketch.clone();
        resolve(&mut sketch);
        assert_eq!(sketch, before);
        sketch.status = SolveStatus::OverDefined { conflicting: vec![1] };
        let before = sketch.clone();
        resolve(&mut sketch);
        assert_eq!(sketch, before);
    }
}
