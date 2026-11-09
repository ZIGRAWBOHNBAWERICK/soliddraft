//! Read-only warnings for sketch curves intended to form closed profiles.

use super::JOIN_TOLERANCE;
use crate::{Align, Constraint, ConstraintId, ConstraintKind, Entity, EntityId, EntityKind, P2, Sketch, tools};
use std::f64::consts::TAU;

/// A profile warning, not an instruction to repair geometry automatically.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticKind {
    /// An endpoint does not join another profile curve.
    OpenEnd,
    /// Two curves cover a common interval of positive length.
    Overlap,
    /// More than two curve directions meet at a point.
    Branch,
    /// A curve is invalid or a closed chain encloses no area.
    Degenerate,
    /// A relation references missing geometry or an unsupported target kind.
    InvalidRelation,
}

impl DiagnosticKind {
    /// The short name used in the inspector.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenEnd => "Open end",
            Self::Overlap => "Overlapping curves",
            Self::Branch => "Branching junction",
            Self::Degenerate => "Invalid or degenerate curve",
            Self::InvalidRelation => "Invalid relation target",
        }
    }
}

/// The geometry and plane locations involved in one warning.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    /// The fault found by the profile analysis.
    pub kind: DiagnosticKind,
    /// Related entity IDs, in sorted order.
    pub entities: Vec<EntityId>,
    /// Finite locations for markers and camera focus.
    pub locations: Vec<P2>,
    /// The authored relation to inspect, never removed by diagnosis.
    pub relation: Option<ConstraintId>,
    /// Why a relation reference is invalid.
    pub message: Option<String>,
}

impl Diagnostic {
    pub(super) fn new(kind: DiagnosticKind, mut entities: Vec<EntityId>, locations: Vec<P2>) -> Self {
        entities.sort_unstable();
        entities.dedup();
        Self {
            kind,
            entities,
            locations: locations.into_iter().filter(|p| p.u.is_finite() && p.v.is_finite()).collect(),
            relation: None,
            message: None,
        }
    }
}

/// Inspect non-construction profile curves without changing the sketch.
/// Open ends use the same join tolerance and resolved references as profile extraction.
/// Missing and wrong-kind relation references are reported without changing their rows.
/// This does not merge curves, close gaps or replace a kernel validity check.
pub fn inspect(sketch: &Sketch) -> Vec<Diagnostic> {
    let mut issues = super::loops(sketch, true).issues;
    for constraint in &sketch.constraints {
        if let Some(message) = invalid_relation(sketch, constraint) {
            let entities = constraint.entities();
            let locations = entities.iter().filter_map(|id| sketch.entity(*id)).flat_map(|e| e.points()).collect();
            let mut issue = Diagnostic::new(DiagnosticKind::InvalidRelation, entities, locations);
            issue.relation = Some(constraint.id);
            issue.message = Some(message);
            issues.push(issue);
        }
    }
    issues.sort_by(|a, b| {
        a.kind.cmp(&b.kind).then(a.entities.cmp(&b.entities)).then_with(|| {
            a.locations
                .first()
                .zip(b.locations.first())
                .map_or(std::cmp::Ordering::Equal, |(a, b)| a.u.total_cmp(&b.u).then(a.v.total_cmp(&b.v)))
        })
    });
    issues
}

fn invalid_relation(sketch: &Sketch, constraint: &Constraint) -> Option<String> {
    for id in constraint.entities() {
        if sketch.entity(id).is_none() {
            return Some(format!("Missing entity {id}"));
        }
    }
    for point in constraint.points() {
        if sketch.point(point).is_none() {
            return Some(format!("Missing point {}:{}", point.entity, point.index));
        }
    }
    let line = |id| sketch.entity(id).is_some_and(|e| e.is_line());
    let ellipse = |id| sketch.entity(id).is_some_and(|e| crate::ellipse::Ellipse::of(e.kind).is_some());
    let centered =
        |id| sketch.entity(id).is_some_and(|e| e.is_curve() || crate::ellipse::Ellipse::of(e.kind).is_some());
    let circular = |id| sketch.entity(id).is_some_and(|e| tools::circle(e.kind).is_some());
    let reason = match constraint.geometry_kind() {
        ConstraintKind::Horizontal { target: Align::Line(id) }
        | ConstraintKind::Vertical { target: Align::Line(id) }
            if !line(id) =>
        {
            Some("This alignment requires a line target")
        }
        ConstraintKind::Parallel { a, b }
        | ConstraintKind::Perpendicular { a, b }
        | ConstraintKind::Angle { a, b, .. }
            if !line(a) || !line(b) =>
        {
            Some("This relation requires two line targets")
        }
        ConstraintKind::Midpoint { line: id, .. } | ConstraintKind::PointLineDistance { line: id, .. } if !line(id) => {
            Some("This relation requires a line target")
        }
        ConstraintKind::Symmetric { a, b, axis } if !line(axis) || a == b || a.entity == axis || b.entity == axis => {
            Some("Symmetry requires two distinct points outside the axis entity")
        }
        ConstraintKind::FixEntity { entity, at }
            if !tools::valid(at)
                || sketch
                    .entity(entity)
                    .is_some_and(|e| std::mem::discriminant(&e.kind) != std::mem::discriminant(&at)) =>
        {
            Some("Fixed geometry no longer matches its recorded type")
        }
        ConstraintKind::Offset { a, b, distance, .. }
            if !distance.is_finite()
                || distance.abs() <= 1e-6
                || !((line(a) && line(b)) || (circular(a) && circular(b))) =>
        {
            Some("Offset requires two matching curves and a nonzero finite distance")
        }
        ConstraintKind::OffsetCap { cap, .. }
            if !line(cap) && !sketch.entity(cap).is_some_and(|e| matches!(e.kind, EntityKind::Arc { .. })) =>
        {
            Some("An offset cap must be a line or semicircular arc")
        }
        ConstraintKind::Radius { entity, .. } if !circular(entity) => Some("Radius requires a circle or arc target"),
        ConstraintKind::Concentric { a, b } if !centered(a) || !centered(b) => {
            Some("Concentric requires two circle, arc or ellipse targets")
        }
        ConstraintKind::Equal { a, b }
            if !((line(a) && line(b)) || (circular(a) && circular(b)) || (ellipse(a) && ellipse(b))) =>
        {
            Some("Equal requires two lines, two circular targets or two ellipses")
        }
        ConstraintKind::Tangent { a, b }
            if !((line(a) && circular(b)) || (circular(a) && line(b)) || (circular(a) && circular(b))) =>
        {
            Some("Tangent requires a line and circular curve, or two circular curves")
        }
        _ => None,
    };
    reason.map(str::to_owned)
}

fn intervals(kind: EntityKind) -> Vec<(f64, f64)> {
    match kind {
        EntityKind::Circle { .. } => vec![(0.0, TAU)],
        EntityKind::Arc { center, start, .. } => {
            let e = Entity { id: 0, kind, construction: false };
            let (_, sweep) = e.arc_angles().expect("valid arc");
            let from = (start - center).angle().rem_euclid(TAU);
            let to = from + sweep;
            if to > TAU { vec![(from, TAU), (0.0, to - TAU)] } else { vec![(from, to)] }
        }
        _ => Vec::new(),
    }
}

fn overlap(a: EntityKind, b: EntityKind) -> Vec<P2> {
    if let (EntityKind::Line { a: p, b: q }, EntityKind::Line { a: r, b: s }) = (a, b) {
        let direction = (q - p).normalized();
        if direction.cross(r - p).abs() > JOIN_TOLERANCE || direction.cross(s - p).abs() > JOIN_TOLERANCE {
            return Vec::new();
        }
        let (x, y) = ((r - p).dot(direction), (s - p).dot(direction));
        let lo = x.min(y).max(0.0);
        let hi = x.max(y).min(p.dist(q));
        return if hi - lo > JOIN_TOLERANCE {
            vec![p + direction.scale(lo), p + direction.scale(hi)]
        } else {
            Vec::new()
        };
    }
    let (Some((c, r)), Some((d, s))) = (tools::circle(a), tools::circle(b)) else { return Vec::new() };
    if c.dist(d) > JOIN_TOLERANCE || (r - s).abs() > JOIN_TOLERANCE {
        return Vec::new();
    }
    let mut points = Vec::new();
    let limits = intervals(b);
    for (from, to) in intervals(a) {
        for &(other_from, other_to) in &limits {
            let lo = from.max(other_from);
            let hi = to.min(other_to);
            if (hi - lo) * r > JOIN_TOLERANCE {
                for angle in [lo, (lo + hi) / 2.0, hi] {
                    points.push(c + P2::from_angle(angle).scale(r));
                }
            }
        }
    }
    points
}

pub(super) fn append_pairs(drawn: &[&Entity], issues: &mut Vec<Diagnostic>) {
    for (i, a) in drawn.iter().enumerate() {
        if a.is_point() || !tools::valid(a.kind) {
            continue;
        }
        for b in &drawn[i + 1..] {
            if b.is_point() || !tools::valid(b.kind) {
                continue;
            }
            let locations = overlap(a.kind, b.kind);
            if !locations.is_empty() {
                issues.push(Diagnostic::new(DiagnosticKind::Overlap, vec![a.id, b.id], locations));
                continue;
            }
            for at in tools::intersections(a.kind, b.kind)
                .into_iter()
                .filter(|p| tools::on_curve(a.kind, *p) && tools::on_curve(b.kind, *p))
            {
                let degree = |kind| {
                    tools::ends(kind)
                        .map_or(2, |ends| if ends.iter().any(|p| p.dist(at) <= JOIN_TOLERANCE) { 1 } else { 2 })
                };
                if degree(a.kind) + degree(b.kind) <= 2 {
                    continue;
                }
                if let Some(issue) = issues.iter_mut().find(|issue| {
                    issue.kind == DiagnosticKind::Branch && issue.locations.iter().any(|p| p.dist(at) <= JOIN_TOLERANCE)
                }) {
                    issue.entities.extend([a.id, b.id]);
                    issue.entities.sort_unstable();
                    issue.entities.dedup();
                } else {
                    issues.push(Diagnostic::new(DiagnosticKind::Branch, vec![a.id, b.id], vec![at]));
                }
            }
        }
    }
}
