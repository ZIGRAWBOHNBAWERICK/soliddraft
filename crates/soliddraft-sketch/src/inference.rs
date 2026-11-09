//! Analytic snap previews and the relations retained by a placement.

use serde::{Deserialize, Serialize};

use crate::{Align, ConstraintKind, EntityId, EntityKind, P2, PointRef, Sketch, tools};

/// A snap class, optionally used for a single placement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapFilter {
    /// All supported snaps in priority order.
    #[default]
    All,
    /// Entity points, centres and the origin.
    Point,
    /// Line midpoints.
    Midpoint,
    /// Bounded curve crossings.
    Intersection,
    /// Nearest point on a curve.
    On,
    /// Horizontal or vertical from the previous point.
    Axes,
    /// Parallel to an existing line.
    Parallel,
    /// Perpendicular to an existing line.
    Perpendicular,
    /// Tangent to a circular curve.
    Tangent,
    /// Place without snapping.
    None,
}

impl SnapFilter {
    /// Choices exposed by the quick snap control.
    pub const ALL: [Self; 10] = [
        Self::All,
        Self::Point,
        Self::Midpoint,
        Self::Intersection,
        Self::On,
        Self::Axes,
        Self::Parallel,
        Self::Perpendicular,
        Self::Tangent,
        Self::None,
    ];

    /// The control's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All snaps",
            Self::Point => "Point / centre",
            Self::Midpoint => "Midpoint",
            Self::Intersection => "Intersection",
            Self::On => "On curve",
            Self::Axes => "Horizontal / vertical",
            Self::Parallel => "Parallel",
            Self::Perpendicular => "Perpendicular",
            Self::Tangent => "Tangent",
            Self::None => "No snap",
        }
    }
}

/// An inference shown at the pointer and retained on placement.
#[derive(Clone, Debug, PartialEq)]
pub enum Snap {
    /// An existing entity point.
    Point(PointRef),
    /// A line's midpoint.
    Midpoint(EntityId),
    /// A point on a bounded curve.
    On(EntityId),
    /// A crossing retained as two point-on relations.
    Intersection(EntityId, EntityId),
    /// Horizontal from the previous point.
    Horizontal,
    /// Vertical from the previous point.
    Vertical,
    /// A new line parallel to this line.
    Parallel(EntityId),
    /// A new line perpendicular to this line.
    Perpendicular(EntityId),
    /// A new line tangent to this arc or circle.
    Tangent(EntityId),
}

/// Distances use the caller's metric, pixels for the viewport or mm headlessly.
#[derive(Clone, Copy, Debug)]
pub struct InferenceOptions {
    /// Restrict the offered class.
    pub filter: SnapFilter,
    /// Offer line relations only for an actual new line endpoint.
    pub directions: bool,
    /// Point, intersection and midpoint reach.
    pub point_reach: f64,
    /// Curve projection reach.
    pub curve_reach: f64,
    /// Direction projection reach.
    pub align_reach: f64,
    /// Suppress direction inference for very short segments.
    pub min_length: f64,
}

impl InferenceOptions {
    /// A common reach in mm for a headless placement.
    pub fn uniform(reach: f64, filter: SnapFilter) -> Self {
        Self {
            filter,
            directions: true,
            point_reach: reach,
            curve_reach: reach,
            align_reach: reach,
            min_length: reach * 2.0,
        }
    }
}

/// Pick the nearest candidate within each priority class, without mutating the sketch.
/// Curves stay bounded for crossings, curve snaps and arc tangency contacts.
pub fn infer(
    sketch: &Sketch,
    raw: P2,
    previous: Option<P2>,
    options: InferenceOptions,
    distance: impl Fn(P2, P2) -> f64,
) -> (P2, Vec<Snap>) {
    if options.filter == SnapFilter::None || !raw.u.is_finite() || !raw.v.is_finite() {
        return (raw, Vec::new());
    }
    if matches!(options.filter, SnapFilter::All | SnapFilter::Point) {
        let mut point: Option<(f64, PointRef, P2)> = None;
        for e in &sketch.entities {
            for i in 0..e.point_count() {
                let Some(p) = e.point(i) else { continue };
                let d = distance(raw, p);
                if d.is_finite() && d <= options.point_reach && point.is_none_or(|(old, _, _)| d < old) {
                    point = Some((d, PointRef::new(e.id, i), p));
                }
            }
        }
        if let Some((_, reference, p)) = point {
            return (p, vec![Snap::Point(reference)]);
        }
        if options.filter == SnapFilter::Point {
            return (raw, Vec::new());
        }
    }
    let mut best: Option<(u8, f64, P2, Snap)> = None;
    let mut offer = |at: P2, snap: Snap, class: SnapFilter, priority: u8, reach: f64| {
        if options.filter != SnapFilter::All && options.filter != class {
            return;
        }
        let d = distance(raw, at);
        if d.is_finite() && d <= reach && best.as_ref().is_none_or(|(rank, old, _, _)| (priority, d) < (*rank, *old)) {
            best = Some((priority, d, at, snap));
        }
    };
    let curves: Vec<_> = sketch
        .drawn()
        .filter(|e| !e.is_point() && !matches!(e.kind, EntityKind::Text { .. }) && tools::valid(&e.kind))
        .collect();
    if matches!(options.filter, SnapFilter::All | SnapFilter::Intersection) {
        // Reject distant pairs before calculating analytic intersections.
        let nearby: Vec<_> = curves.iter().filter(|e| distance(raw, e.nearest(raw).0) <= options.point_reach).collect();
        for (i, a) in nearby.iter().enumerate() {
            for b in &nearby[i + 1..] {
                for p in tools::intersections(&a.kind, &b.kind) {
                    if tools::on_curve(&a.kind, p) && tools::on_curve(&b.kind, p) {
                        offer(p, Snap::Intersection(a.id, b.id), SnapFilter::Intersection, 1, options.point_reach);
                    }
                }
            }
        }
    }
    for e in &curves {
        if let EntityKind::Line { a, b } = e.kind {
            offer(a.mid(b), Snap::Midpoint(e.id), SnapFilter::Midpoint, 2, options.point_reach);
        }
    }
    if let Some(start) = previous.filter(|p| distance(*p, raw) > options.min_length) {
        offer(P2::new(raw.u, start.v), Snap::Horizontal, SnapFilter::Axes, 3, options.align_reach);
        offer(P2::new(start.u, raw.v), Snap::Vertical, SnapFilter::Axes, 3, options.align_reach);
        if options.directions {
            for e in &curves {
                if let EntityKind::Line { a, b } = e.kind {
                    let dir = (b - a).normalized();
                    offer(
                        start + dir.scale((raw - start).dot(dir)),
                        Snap::Parallel(e.id),
                        SnapFilter::Parallel,
                        3,
                        options.align_reach,
                    );
                    let normal = dir.perp();
                    offer(
                        start + normal.scale((raw - start).dot(normal)),
                        Snap::Perpendicular(e.id),
                        SnapFilter::Perpendicular,
                        3,
                        options.align_reach,
                    );
                } else if let Some((center, radius)) = tools::circle(&e.kind) {
                    let delta = start - center;
                    let len = delta.len();
                    if len < radius - 1e-6 || len <= 1e-6 {
                        continue;
                    }
                    if (len - radius).abs() <= 1e-6 {
                        if tools::on_curve(&e.kind, start) {
                            let dir = delta.perp().normalized();
                            offer(
                                start + dir.scale((raw - start).dot(dir)),
                                Snap::Tangent(e.id),
                                SnapFilter::Tangent,
                                3,
                                options.align_reach,
                            );
                        }
                    } else {
                        let foot = center + delta.scale((radius / len).powi(2));
                        let off = delta.perp().scale((radius / len) * (1.0 - (radius / len).powi(2)).sqrt());
                        for contact in [foot - off, foot + off] {
                            if tools::on_curve(&e.kind, contact) {
                                let dir = (contact - start).normalized();
                                offer(
                                    start + dir.scale((raw - start).dot(dir)),
                                    Snap::Tangent(e.id),
                                    SnapFilter::Tangent,
                                    3,
                                    options.align_reach,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    for e in &curves {
        offer(e.nearest(raw).0, Snap::On(e.id), SnapFilter::On, 4, options.curve_reach);
    }
    best.map_or((raw, Vec::new()), |(_, _, p, snap)| (p, vec![snap]))
}

/// Store the relations shown by inference for a newly placed point.
pub fn apply_snaps(sketch: &mut Sketch, point: PointRef, snaps: &[Snap], new_line: Option<EntityId>) {
    for snap in snaps {
        let relation = match *snap {
            Snap::Point(other) if other.entity != point.entity => {
                Some(ConstraintKind::Coincident { a: point, b: other })
            }
            Snap::Midpoint(line) => Some(ConstraintKind::Midpoint { point, line }),
            Snap::On(entity) => Some(ConstraintKind::PointOn { point, entity }),
            Snap::Intersection(a, b) => {
                sketch.constrain(ConstraintKind::PointOn { point, entity: a });
                Some(ConstraintKind::PointOn { point, entity: b })
            }
            Snap::Horizontal => new_line.map(|line| ConstraintKind::Horizontal { target: Align::Line(line) }),
            Snap::Vertical => new_line.map(|line| ConstraintKind::Vertical { target: Align::Line(line) }),
            Snap::Parallel(other) => new_line.map(|line| ConstraintKind::Parallel { a: other, b: line }),
            Snap::Perpendicular(other) => new_line.map(|line| ConstraintKind::Perpendicular { a: other, b: line }),
            Snap::Tangent(other) => new_line.map(|line| ConstraintKind::Tangent { a: other, b: line }),
            _ => None,
        };
        if let Some(relation) = relation {
            sketch.constrain(relation);
        }
    }
}
