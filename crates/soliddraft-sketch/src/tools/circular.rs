//! Analytic placement shared by pointer previews and headless tools.

use super::*;

fn checked(kind: EntityKind) -> Result<EntityKind> {
    if valid(&kind) { Ok(kind) } else { refuse("circle or arc needs finite, distinct points") }
}

/// Fit a circle through three distinct, noncollinear points.
pub fn perimeter_circle(a: P2, b: P2, c: P2) -> Result<EntityKind> {
    let u = b - a;
    let v = c - a;
    let cross = u.cross(v);
    if !finite(a)
        || !finite(b)
        || !finite(c)
        || a.dist(b) <= EPS
        || a.dist(c) <= EPS
        || b.dist(c) <= EPS
        || cross.abs() <= EPS * u.len().max(v.len())
    {
        return refuse("perimeter circle needs three distinct, noncollinear points");
    }
    let center = a + P2::new(v.v * u.dot(u) - u.v * v.dot(v), u.u * v.dot(v) - v.u * u.dot(u)).scale(0.5 / cross);
    checked(EntityKind::Circle { center, radius: center.dist(a) })
}

/// Project the last pick onto the start radius and retain the requested sweep.
pub fn centrepoint_arc(center: P2, start: P2, end: P2, clockwise: bool) -> Result<EntityKind> {
    if !finite(end) || center.dist(end) <= EPS {
        return refuse("arc end must be distinct from its centre");
    }
    let end = center + (end - center).normalized().scale(center.dist(start));
    let (start, end) = if clockwise { (end, start) } else { (start, end) };
    checked(EntityKind::Arc { center, start, end })
}

/// Fit an arc from start to end through the third pick, including major sweeps.
pub fn three_point_arc(start: P2, end: P2, through: P2) -> Result<EntityKind> {
    let EntityKind::Circle { center, .. } = perimeter_circle(start, end, through)? else { unreachable!() };
    let angle = |p: P2| ((p - center).angle() - (start - center).angle()).rem_euclid(TAU);
    let (start, end) = if angle(through) < angle(end) { (start, end) } else { (end, start) };
    checked(EntityKind::Arc { center, start, end })
}

/// Continue outward from a line or circular arc endpoint with a tangent arc.
/// Returns the curve and the new curve's point index at the join.
pub fn tangent_arc(sketch: &Sketch, from: PointRef, end: P2) -> Result<(EntityKind, usize)> {
    let source = sketch.entity(from.entity).ok_or_else(|| ToolError("tangent source is missing".into()))?;
    let (start, tangent) = match (&source.kind, from.index) {
        (EntityKind::Line { a, b }, 0) => (*a, *a - *b),
        (EntityKind::Line { a, b }, 1) => (*b, *b - *a),
        (EntityKind::Arc { center, start, .. }, 1) => (*start, (*center - *start).perp()),
        (EntityKind::Arc { center, end, .. }, 2) => (*end, (*end - *center).perp()),
        _ => return refuse("pick a line or circular arc endpoint for the tangent arc"),
    };
    if !valid(&source.kind) || !finite(end) || start.dist(end) <= EPS {
        return refuse("tangent arc needs a distinct finite endpoint");
    }
    let normal = tangent.normalized().perp();
    let chord = end - start;
    let projection = chord.dot(normal);
    if projection.abs() <= EPS {
        return refuse("tangent arc endpoint cannot lie on the starting tangent");
    }
    let distance = chord.dot(chord) / (2.0 * projection);
    let center = start + normal.scale(distance);
    let (start, end, join) = if distance > 0.0 { (start, end, 1) } else { (end, start, 2) };
    Ok((checked(EntityKind::Arc { center, start, end })?, join))
}
