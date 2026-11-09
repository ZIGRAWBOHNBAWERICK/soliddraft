//! Rigid instance relationships expressed with the existing solver primitives.
use super::*;

type AxisKey = (u32, u64, u64, u64);
pub(super) type Axes = HashMap<AxisKey, (DatumLineSegment, DatumLineSegment)>;

fn hidden(vars: &mut Vars, at: P2) -> DatumPoint {
    let p = DatumPoint::new(&mut vars.ids);
    vars.guesses.push((p.id_x(), at.u));
    vars.guesses.push((p.id_y(), at.v));
    p
}

fn fixed(vars: &mut Vars, reqs: &mut Vec<Constraint>, at: P2) -> DatumPoint {
    let p = hidden(vars, at);
    reqs.push(Constraint::Fixed(p.id_x(), at.u));
    reqs.push(Constraint::Fixed(p.id_y(), at.v));
    p
}

pub(super) fn requirements(
    vars: &mut Vars,
    axes: &mut Axes,
    source: Entity,
    target: Entity,
    transform: ConstraintKind,
) -> Vec<Constraint> {
    let mut reqs = Vec::new();
    let ConstraintKind::PatternTransform { group, translation, center, angle_degrees: angle, .. } = transform else {
        return reqs;
    };
    if source.point_count() != target.point_count() {
        return reqs;
    }
    let angle = angle.rem_euclid(360.0);
    let rotation = if angle.abs() > 1e-10 {
        let key = (group, center.u.to_bits(), center.v.to_bits(), angle.to_bits());
        if let Some(lines) = axes.get(&key) {
            Some(*lines)
        } else if let Some((c, a, b)) = crate::tools::pattern_axes(center, angle) {
            let c = fixed(vars, &mut reqs, c);
            let a = fixed(vars, &mut reqs, a);
            let b = fixed(vars, &mut reqs, b);
            let lines = (DatumLineSegment::new(c, a), DatumLineSegment::new(c, b));
            axes.insert(key, lines);
            Some(lines)
        } else {
            return reqs;
        }
    } else {
        None
    };
    for index in 0..source.point_count() {
        let Some(p) = vars.points.get(&PointRef::new(source.id, index)).copied() else { continue };
        let Some(q) = vars.points.get(&PointRef::new(target.id, index)).copied() else { continue };
        if let Some((first, second)) = rotation {
            let at = source.point(index).expect("entity point");
            let d = at - center;
            let middle = hidden(vars, center + P2::new(d.u, -d.v));
            let translated = translation.len() > 1e-10;
            let rotated =
                if translated { hidden(vars, target.point(index).expect("entity point") - translation) } else { q };
            reqs.push(Constraint::Symmetric(first, p, middle));
            reqs.push(Constraint::Symmetric(second, middle, rotated));
            if translated {
                reqs.push(Constraint::HorizontalDistance(rotated, q, -translation.u));
                reqs.push(Constraint::VerticalDistance(rotated, q, -translation.v));
            }
        } else {
            reqs.push(Constraint::HorizontalDistance(p, q, -translation.u));
            reqs.push(Constraint::VerticalDistance(p, q, -translation.v));
        }
    }
    if matches!(source.kind, EntityKind::Circle { .. })
        && matches!(target.kind, EntityKind::Circle { .. })
        && let (Some(a), Some(b)) = (vars.radii.get(&source.id), vars.radii.get(&target.id))
    {
        reqs.push(Constraint::ScalarEqual(a.id, b.id));
    }
    reqs
}
