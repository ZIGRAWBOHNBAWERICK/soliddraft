//! Tolerance-based sketch tools. These retain analytic lines and circular arcs;
//! Catallax's existing converter supplies the exact kernel representation.

mod mirror;
pub use mirror::{MirrorAxis, reflected};

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};

use crate::{
    Constraint, ConstraintId, ConstraintKind, Entity, EntityId, EntityKind, ORIGIN, OffsetJoin, P2, PointRef, Sketch,
};

mod patterns;
pub(crate) use patterns::pattern_axes;
pub use patterns::{PatternDefinition, PatternDirection, PatternInstance, PatternSpec};

mod finishing;
pub use finishing::{ChamferSize, JunctionPreview, SplitPreview};
mod offset;
pub use offset::{OffsetCap, OffsetOptions};

const EPS: f64 = 1e-6;

/// Session or per-operation choices for trimming construction geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrimOptions {
    /// Retain removed portions as non-profile construction geometry.
    pub keep_as_construction: bool,
    /// Protect construction targets, while still using them as boundaries.
    pub ignore_construction: bool,
}

impl TrimOptions {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The endpoint to extend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    /// The start of a line or counter-clockwise arc.
    Start,
    /// The end of a line or counter-clockwise arc.
    End,
}

/// One sketch tool invocation. All coordinates are in the sketch's plane, in mm.
/// Mirror and pattern copies are independent geometry. Pattern counts include the seed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// Add seed geometry for a tool, using the same entities as the pointer tools.
    Add {
        /// Analytic geometry to add.
        kind: EntityKind,
        /// Exclude this entity from profiles.
        #[serde(default)]
        construction: bool,
    },
    /// Place a line with the same snap preview and retained relations as the viewport.
    SnappedLine {
        /// Raw first placement, in mm.
        start: P2,
        /// Raw second placement, in mm.
        end: P2,
        /// Positive snap reach in mm.
        reach: f64,
        /// Snap class for the first placement.
        #[serde(default)]
        start_filter: crate::inference::SnapFilter,
        /// Snap class for the second placement.
        #[serde(default)]
        end_filter: crate::inference::SnapFilter,
        /// Exclude the line from profiles.
        #[serde(default)]
        construction: bool,
    },
    /// Add a validated relation or a set of equivalent solver relations atomically.
    Relate {
        /// Relation and its selected geometry.
        relation: crate::relations::Relation,
    },
    /// A regular polygon, counter-clockwise from its first vertex.
    Polygon {
        /// Circumcentre.
        center: P2,
        /// First vertex, which sets radius and rotation.
        vertex: P2,
        /// Number of sides, at least three.
        sides: u32,
    },
    /// Insert a tangent radius between two picked lines or arcs.
    Fillet {
        /// First curve.
        a: EntityId,
        /// Second curve.
        b: EntityId,
        /// Portion of the first curve to retain.
        keep_a: P2,
        /// Portion of the second curve to retain.
        keep_b: P2,
        /// Positive radius, mm.
        radius: f64,
    },
    /// Replace a line corner by a segment sized from the corner.
    Chamfer {
        /// First line.
        a: EntityId,
        /// Second line.
        b: EntityId,
        /// Portion of the first line to retain.
        keep_a: P2,
        /// Portion of the second line to retain.
        keep_b: P2,
        /// Setbacks or setback and angle.
        size: ChamferSize,
    },
    /// Divide a curve without removing any portion.
    Split {
        /// Line, arc or circle.
        entity: EntityId,
        /// Point projected onto the curve.
        at: P2,
        /// A second point, required only for a circle.
        #[serde(default)]
        second: Option<P2>,
    },
    /// Remove the picked interval between intersections with other sketch curves.
    Trim {
        /// Line, arc or circle to trim.
        entity: EntityId,
        /// A point identifying the interval to remove.
        at: P2,
        /// Optional construction handling. Omission preserves ordinary trim behavior.
        #[serde(default, skip_serializing_if = "TrimOptions::is_default")]
        options: TrimOptions,
    },
    /// Trim or extend two lines/arcs to a corner while retaining the picked portions.
    TrimCorner {
        /// First line or arc.
        a: EntityId,
        /// Second line or arc, distinct from the first.
        b: EntityId,
        /// A point on the portion of the first curve to keep.
        keep_a: P2,
        /// A point on the portion of the second curve to keep.
        keep_b: P2,
        /// Optional construction handling.
        #[serde(default, skip_serializing_if = "TrimOptions::is_default")]
        options: TrimOptions,
    },
    /// Remove the inside or outside portions between two selected boundary crossings.
    TrimBounded {
        /// Open line or arc to trim.
        entity: EntityId,
        /// First bounding curve, distinct from the target.
        a: EntityId,
        /// Second bounding curve, distinct from the first and target.
        b: EntityId,
        /// Remove the outside portions when true, otherwise remove the inside interval.
        outside: bool,
        /// Optional construction handling.
        #[serde(default, skip_serializing_if = "TrimOptions::is_default")]
        options: TrimOptions,
    },
    /// Extend one endpoint to the nearest intersection with a bounded entity.
    Extend {
        /// Line or arc to extend.
        entity: EntityId,
        /// Which endpoint moves.
        end: End,
        /// The limiting line, arc or circle.
        boundary: EntityId,
    },
    /// Copy an ordered, connected line/arc chain at a signed distance to its left.
    /// A negative distance offsets to the right. A lone circle uses inward positive.
    Offset {
        /// Chain entities in walking order; the first entity sets the direction.
        entities: Vec<EntityId>,
        /// Signed distance, mm. Adjacent curves meet at miter intersections.
        distance: f64,
        /// Chain selection, opposite side, caps and source construction choices.
        #[serde(default, skip_serializing_if = "OffsetOptions::is_default")]
        options: OffsetOptions,
    },
    /// Change a linked offset group's distance while retaining its entity ids.
    EditOffset {
        /// Existing offset group id.
        group: crate::ConstraintId,
        /// New nonzero signed distance, mm.
        distance: f64,
    },
    /// Translate selected entities, preserving compatible constraints or refusing atomically.
    Move {
        /// Entities to move.
        entities: Vec<EntityId>,
        /// Translation in the sketch plane, mm.
        displacement: P2,
    },
    /// Make an independent translated copy, without copying driving relations.
    Copy {
        /// Entities to copy.
        entities: Vec<EntityId>,
        /// Placement base point.
        base: P2,
        /// Displacement from the base point, mm.
        displacement: P2,
    },
    /// Rotate about a point, optionally making an independent copy.
    Rotate {
        /// Entities to transform.
        entities: Vec<EntityId>,
        /// Rotation centre.
        center: P2,
        /// Signed rotation in degrees.
        angle_degrees: f64,
        /// Keep the source geometry and add independent copies.
        #[serde(default)]
        copy: bool,
    },
    /// Uniformly scale about a point, preserving compatible constraints or refusing.
    Scale {
        /// Entities to resize.
        entities: Vec<EntityId>,
        /// Fixed base point of the scaling.
        base: P2,
        /// Positive finite scale factor.
        factor: f64,
    },
    /// Copy geometry across an infinite line, reversing reflected arc endpoints.
    Mirror {
        /// Entities to copy.
        entities: Vec<EntityId>,
        /// First point on the mirror axis.
        axis_start: P2,
        /// Second point on the mirror axis.
        axis_end: P2,
    },
    /// Reflect geometry while retaining symmetry about a stored axis.
    MirrorLinked {
        /// Entities to reflect, excluding the axis itself.
        entities: Vec<EntityId>,
        /// Existing axis line or two points for a fixed construction axis.
        axis: MirrorAxis,
    },
    /// Create a retained pattern definition and linked instances.
    Pattern {
        /// Source geometry.
        entities: Vec<EntityId>,
        /// Distribution and dimensions.
        spec: PatternSpec,
        /// Omitted zero-based positions, excluding the seed.
        #[serde(default)]
        skipped: Vec<[u32; 2]>,
    },
    /// Edit a retained pattern without changing surviving instance ids.
    EditPattern {
        /// Pattern definition id.
        group: u32,
        /// New distribution and dimensions.
        spec: PatternSpec,
        /// Omitted zero-based positions, excluding the seed.
        #[serde(default)]
        skipped: Vec<[u32; 2]>,
    },
    /// Copy geometry at repeated translations.
    LinearPattern {
        /// Seed entities.
        entities: Vec<EntityId>,
        /// Translation between instances.
        step: P2,
        /// Total instance count, including the seed.
        count: u32,
    },
    /// Copy geometry at repeated rotations about a point.
    CircularPattern {
        /// Seed entities.
        entities: Vec<EntityId>,
        /// Rotation centre.
        center: P2,
        /// Angle between instances, in degrees.
        angle_degrees: f64,
        /// Total instance count, including the seed.
        count: u32,
    },
}

/// A refused tool leaves the sketch unchanged.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct ToolError(pub String);

type Result<T, E = ToolError> = std::result::Result<T, E>;

/// What a trim would remove and keep, in target traversal order.
#[derive(Clone, Debug, PartialEq)]
pub struct TrimPreview {
    /// The portions of the target that disappear.
    pub removed: Vec<EntityKind>,
    /// The pieces of the target that remain, from its start to its end.
    pub kept: Vec<EntityKind>,
}

/// The resulting curves and removed pieces of a two-entity corner.
#[derive(Clone, Debug, PartialEq)]
pub struct CornerPreview {
    /// New geometry in first/second entity order.
    pub kept: [EntityKind; 2],
    /// Only the portions that disappear, not the retained geometry.
    pub removed: Vec<EntityKind>,
    joint: [PointRef; 2],
}

/// One power-trim cut, retained until the gesture ends so retracing can restore it.
#[derive(Clone, Debug, PartialEq)]
pub struct TrimCheckpoint {
    /// Where the stroke crossed the curve.
    pub at: P2,
    /// Stroke direction at the crossing.
    pub direction: P2,
    entity: Entity,
    entity_index: usize,
    next_entity: EntityId,
    next_constraint: ConstraintId,
    next_pattern: u32,
    constraints: Vec<(usize, Constraint)>,
    patterns: Vec<(usize, PatternDefinition)>,
}

impl TrimCheckpoint {
    /// Restores this cut after all later checkpoints have been restored.
    pub fn restore(self, sketch: &mut Sketch) {
        sketch.entities.retain(|e| e.id < self.next_entity && e.id != self.entity.id);
        sketch.entities.insert(self.entity_index, self.entity);
        let changed: std::collections::HashSet<_> = self.constraints.iter().map(|(_, c)| c.id).collect();
        sketch.constraints.retain(|c| c.id < self.next_constraint && !changed.contains(&c.id));
        for (index, constraint) in self.constraints {
            sketch.constraints.insert(index, constraint);
        }
        let changed: std::collections::HashSet<_> = self.patterns.iter().map(|(_, p)| p.id).collect();
        sketch.patterns.retain(|p| p.id < self.next_pattern && !changed.contains(&p.id));
        for (index, pattern) in self.patterns {
            sketch.patterns.insert(index, pattern);
        }
        sketch.next_entity = self.next_entity;
        sketch.next_constraint = self.next_constraint;
        sketch.next_pattern = self.next_pattern;
    }
}

/// The corners of a regular polygon, counter-clockwise from `vertex` about `center`.
/// Right angles come out exact, as [`Operation::Polygon`] makes them.
pub fn polygon_corners(center: P2, vertex: P2, sides: u32) -> Result<Vec<P2>> {
    if !finite(center) || !finite(vertex) || center.dist(vertex) <= EPS || !(3..=1000).contains(&sides) {
        return refuse("a polygon needs a finite centre, a distinct vertex and 3 to 1000 sides");
    }
    let d = vertex - center;
    Ok((0..sides)
        .map(|i| {
            if i == 0 {
                vertex
            } else {
                let (s, c) = rotation_sin_cos(360.0 * f64::from(i) / f64::from(sides));
                center + P2::new(c * d.u - s * d.v, s * d.u + c * d.v)
            }
        })
        .collect())
}

fn refuse<T>(message: &str) -> Result<T> {
    Err(ToolError(message.into()))
}

fn finite(p: P2) -> bool {
    p.u.is_finite() && p.v.is_finite()
}

fn entity(kind: EntityKind) -> Entity {
    Entity { id: 0, kind, construction: false }
}

pub(crate) fn valid(kind: &EntityKind) -> bool {
    match *kind {
        EntityKind::Spline { mode, ref points } => crate::spline::spans(mode, points).is_some(),
        ref kind @ (EntityKind::Ellipse { .. } | EntityKind::EllipticArc { .. }) => {
            let ellipse = crate::ellipse::Ellipse::of(kind).unwrap();
            ellipse.valid()
                && if let EntityKind::EllipticArc { start, end, .. } = *kind {
                    finite(start)
                        && finite(end)
                        && start.dist(end) > EPS
                        && start.dist(ellipse.at(ellipse.parameter(start))) < EPS
                        && end.dist(ellipse.at(ellipse.parameter(end))) < EPS
                } else {
                    true
                }
        }
        EntityKind::Point { p } => finite(p),
        EntityKind::Line { a, b } => finite(a) && finite(b) && a.dist(b).is_finite() && a.dist(b) > EPS,
        EntityKind::Arc { center, start, end } => {
            let r = center.dist(start);
            finite(center)
                && finite(start)
                && finite(end)
                && r.is_finite()
                && r > EPS
                && start.dist(end) > EPS
                && (center.dist(end) - r).abs() <= EPS
        }
        EntityKind::Circle { center, radius } => finite(center) && radius.is_finite() && radius > EPS,
    }
}

// Degree inputs name right angles exactly. Keep their zero components exact
// rather than sending trig residuals into Catallax's rational arc conversion.
fn rotation_sin_cos(degrees: f64) -> (f64, f64) {
    match degrees.rem_euclid(360.0) {
        0.0 => (0.0, 1.0),
        90.0 => (1.0, 0.0),
        180.0 => (0.0, -1.0),
        270.0 => (-1.0, 0.0),
        angle => angle.to_radians().sin_cos(),
    }
}

pub(crate) fn ends(kind: &EntityKind) -> Option<[P2; 2]> {
    match *kind {
        EntityKind::Line { a, b } => Some([a, b]),
        EntityKind::Arc { start, end, .. } => Some([start, end]),
        _ => None,
    }
}

fn copy_endpoints(entity: &Entity) -> Option<[(PointRef, P2); 2]> {
    let indices = match entity.kind {
        EntityKind::Spline { ref points, .. } if points.len() >= 2 => [0, points.len() - 1],
        EntityKind::Line { .. } => [0, 1],
        EntityKind::Arc { .. } => [1, 2],
        EntityKind::EllipticArc { .. } => [3, 4],
        _ => return None,
    };
    Some(indices.map(|index| {
        (PointRef::new(entity.id, index), entity.point(index).expect("endpoint index matches entity kind"))
    }))
}

fn reversed(kind: EntityKind) -> EntityKind {
    match kind {
        EntityKind::Line { a, b } => EntityKind::Line { a: b, b: a },
        // Arcs are always CCW in the sketch model. Offset uses a separate walking direction.
        _ => kind,
    }
}

fn set_end(kind: &mut EntityKind, end: End, p: P2) {
    match (kind, end) {
        (EntityKind::Line { a, .. }, End::Start) => *a = p,
        (EntityKind::Line { b, .. }, End::End) => *b = p,
        (EntityKind::Arc { start, .. }, End::Start) => *start = p,
        (EntityKind::Arc { end, .. }, End::End) => *end = p,
        _ => {}
    }
}

fn parameter(kind: &EntityKind, p: P2) -> Option<f64> {
    match *kind {
        EntityKind::Line { a, b } => {
            let d = b - a;
            Some((p - a).dot(d) / d.dot(d))
        }
        EntityKind::Arc { center, start, .. } => {
            let (_, sweep) = entity(kind.clone()).arc_angles()?;
            let a = ((p - center).angle() - (start - center).angle()).rem_euclid(TAU);
            Some(if a > TAU - EPS / center.dist(start) { 0.0 } else { a / sweep })
        }
        _ => None,
    }
}

pub(crate) fn on_curve(kind: &EntityKind, p: P2) -> bool {
    entity(kind.clone()).nearest(p).1 <= EPS
}

pub(crate) fn circle(kind: &EntityKind) -> Option<(P2, f64)> {
    match *kind {
        EntityKind::Arc { center, start, .. } => Some((center, center.dist(start))),
        EntityKind::Circle { center, radius } => Some((center, radius)),
        _ => None,
    }
}

/// Intersections of the infinite supporting lines/circles, never tessellated curves.
pub(crate) fn intersections(a: &EntityKind, b: &EntityKind) -> Vec<P2> {
    if let (EntityKind::Line { a: p, b: q }, EntityKind::Line { a: r, b: s }) = (a, b) {
        let (p, q, r, s) = (*p, *q, *r, *s);
        let (d, e) = (q - p, s - r);
        let den = d.cross(e);
        if den.abs() <= 1e-12 * d.len() * e.len() {
            return Vec::new();
        }
        return vec![p + d.scale((r - p).cross(e) / den)];
    }
    if let EntityKind::Line { a: p, b: q } = *a {
        let Some((c, r)) = circle(b) else { return Vec::new() };
        let d = (q - p).normalized();
        let t = (c - p).dot(d);
        let foot = p + d.scale(t);
        let h2 = r * r - (foot - c).dot(foot - c);
        if h2 < -EPS * r.max(1.0) {
            return Vec::new();
        }
        let h = h2.max(0.0).sqrt();
        if h <= EPS { vec![foot] } else { vec![foot - d.scale(h), foot + d.scale(h)] }
    } else if matches!(b, EntityKind::Line { .. }) {
        intersections(b, a)
    } else if let (Some((c, r)), Some((d, s))) = (circle(a), circle(b)) {
        let delta = d - c;
        let len = delta.len();
        if len <= EPS || len > r + s + EPS || len < (r - s).abs() - EPS {
            return Vec::new();
        }
        let along = (r * r - s * s + len * len) / (2.0 * len);
        let foot = c + delta.scale(along / len);
        let h2 = r * r - along * along;
        if h2 < -EPS * r.max(1.0) {
            return Vec::new();
        }
        let off = delta.perp().scale(h2.max(0.0).sqrt() / len);
        if off.len() <= EPS { vec![foot] } else { vec![foot - off, foot + off] }
    } else {
        Vec::new()
    }
}

fn point_at(kind: &EntityKind, t: f64) -> P2 {
    let [a, b] = ends(kind).expect("line or arc");
    if t == 0.0 {
        return a;
    }
    if t == 1.0 {
        return b;
    }
    match *kind {
        EntityKind::Line { .. } => a.lerp(b, t),
        EntityKind::Arc { center, start, .. } => {
            let (angle, sweep) = entity(kind.clone()).arc_angles().expect("arc");
            center + P2::from_angle(angle + sweep * t).scale(center.dist(start))
        }
        _ => unreachable!(),
    }
}

/// The nearest point where `target`, lengthened beyond `end`, meets the bounded `limit`,
/// with how far the end travels to reach it. A line or arc target only.
fn extension(target: &Entity, end: End, limit: &Entity) -> Option<(f64, P2)> {
    let [a, b] = ends(&target.kind)?;
    let mut candidates: Vec<_> = intersections(&target.kind, &limit.kind)
        .into_iter()
        .filter_map(|p| {
            if !on_curve(&limit.kind, p) {
                return None;
            }
            let distance = match target.kind {
                EntityKind::Line { .. } => {
                    let t = parameter(&target.kind, p)?;
                    match end {
                        End::Start if t < 0.0 => a.dist(p),
                        End::End if t > 1.0 => b.dist(p),
                        _ => return None,
                    }
                }
                EntityKind::Arc { center, .. } => {
                    let (_, sweep) = target.arc_angles()?;
                    let delta = match end {
                        End::Start => ((a - center).angle() - (p - center).angle()).rem_euclid(TAU),
                        End::End => ((p - center).angle() - (b - center).angle()).rem_euclid(TAU),
                    };
                    if sweep + delta >= TAU - EPS / center.dist(a) {
                        return None;
                    }
                    delta * center.dist(a)
                }
                _ => return None,
            };
            (distance > EPS).then_some((distance, p))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    candidates.first().copied()
}

fn corner_piece(kind: EntityKind, keep: P2, corner: P2) -> Result<(EntityKind, End)> {
    let [start, end] = ends(&kind).ok_or_else(|| ToolError("corner trim needs two lines or arcs".into()))?;
    let keep = entity(kind.clone()).nearest(keep).0;
    let pick = parameter(&kind, keep).expect("line or arc");
    let hit = parameter(&kind, corner).expect("line or arc");
    let side = if corner.dist(start) <= EPS {
        End::Start
    } else if corner.dist(end) <= EPS {
        End::End
    } else if matches!(kind, EntityKind::Line { .. }) || on_curve(&kind, corner) {
        if keep.dist(corner) <= EPS {
            return refuse("pick a portion to keep, away from the corner");
        }
        if pick < hit { End::End } else { End::Start }
    } else {
        let EntityKind::Arc { center, .. } = kind else { unreachable!() };
        let before = ((start - center).angle() - (corner - center).angle()).rem_euclid(TAU);
        let after = ((corner - center).angle() - (end - center).angle()).rem_euclid(TAU);
        if (before - after).abs() * center.dist(start) <= EPS {
            if (pick - 0.5).abs() <= EPS {
                return refuse("pick nearer an arc end to choose its extension direction");
            }
            if pick < 0.5 { End::End } else { End::Start }
        } else if before < after {
            End::Start
        } else {
            End::End
        }
    };
    let mut changed = kind;
    set_end(&mut changed, side, corner);
    if !valid(&changed) {
        return refuse("the corner would collapse a curve or create an invalid arc");
    }
    Ok((changed, side))
}

fn corner_removed(kind: EntityKind, changed: EntityKind, side: End) -> Option<EntityKind> {
    let index = usize::from(side == End::End);
    let at = ends(&changed)?[index];
    if ends(&kind)?[index].dist(at) <= EPS || !on_curve(&kind, at) {
        return None;
    }
    let mut removed = kind;
    set_end(&mut removed, if side == End::Start { End::End } else { End::Start }, at);
    valid(&removed).then_some(removed)
}

impl Sketch {
    fn tool_entity(&self, id: EntityId) -> Result<Entity> {
        self.entity(id)
            .cloned()
            .filter(|e| e.id != ORIGIN && valid(&e.kind))
            .ok_or_else(|| ToolError(format!("entity {id} is missing, degenerate or the origin")))
    }

    fn tool_selection(&self, ids: &[EntityId]) -> Result<Vec<Entity>> {
        if ids.is_empty() {
            return refuse("select at least one entity");
        }
        let mut selected = Vec::with_capacity(ids.len());
        for &id in ids {
            if selected.iter().any(|e: &Entity| e.id == id) {
                return refuse("selection repeats an entity");
            }
            selected.push(self.tool_entity(id)?);
        }
        Ok(selected)
    }

    fn add_tool_entities(&mut self, geometry: Vec<(EntityKind, bool)>) -> Result<Vec<EntityId>> {
        if geometry.iter().any(|(kind, _)| !valid(kind)) {
            return refuse("the tool would produce non-finite or degenerate geometry");
        }
        let ids: Vec<_> = geometry.into_iter().map(|(kind, construction)| self.add(kind, construction)).collect();
        // Copied curves own their points. Join matching endpoints within this new geometry only.
        for (i, &a) in ids.iter().enumerate() {
            let Some(ea) = self.entity(a).cloned() else { continue };
            let Some(pa) = copy_endpoints(&ea) else { continue };
            for &b in &ids[i + 1..] {
                let eb = self.entity(b).expect("new entity");
                let Some(pb) = copy_endpoints(eb) else { continue };
                for &(a, p) in &pa {
                    for &(b, q) in &pb {
                        if p.dist(q) <= EPS {
                            self.constrain(ConstraintKind::Coincident { a, b });
                        }
                    }
                }
            }
        }
        Ok(ids)
    }

    /// Apply one tool atomically. Returns the created or retained entity ids.
    /// Trim/extend keep the target id when possible and drop its old relations,
    /// because dimensions or endpoint relations could otherwise restore deleted geometry.
    pub fn apply_operation(&mut self, operation: &Operation) -> Result<Vec<EntityId>> {
        match operation {
            Operation::Add { kind, construction } => self.add_tool_entities(vec![(kind.clone(), *construction)]),
            Operation::SnappedLine { start, end, reach, start_filter, end_filter, construction } => {
                use crate::inference::{InferenceOptions, apply_snaps, infer};
                if !finite(*start) || !finite(*end) || !reach.is_finite() || *reach <= 0.0 {
                    return refuse("snapped line needs finite points and a positive finite reach");
                }
                let (a, first) = infer(self, *start, None, InferenceOptions::uniform(*reach, *start_filter), P2::dist);
                let (b, last) = infer(self, *end, Some(a), InferenceOptions::uniform(*reach, *end_filter), P2::dist);
                let ids = self.add_tool_entities(vec![(EntityKind::Line { a, b }, *construction)])?;
                apply_snaps(self, PointRef::new(ids[0], 0), &first, Some(ids[0]));
                apply_snaps(self, PointRef::new(ids[0], 1), &last, Some(ids[0]));
                Ok(ids)
            }
            Operation::Relate { relation } => {
                let kinds = self.relation_constraints(relation)?;
                for kind in kinds {
                    if !self.constraints.iter().any(|c| c.kind == kind) {
                        self.constrain(kind);
                    }
                }
                Ok(Vec::new())
            }
            Operation::Polygon { center, vertex, sides } => {
                let corners = polygon_corners(*center, *vertex, *sides)?;
                self.add_tool_entities(
                    (0..corners.len())
                        .map(|i| (EntityKind::Line { a: corners[i], b: corners[(i + 1) % corners.len()] }, false))
                        .collect(),
                )
            }
            Operation::Fillet { a, b, keep_a, keep_b, radius } => self.fillet(*a, *b, *keep_a, *keep_b, *radius),
            Operation::Chamfer { a, b, keep_a, keep_b, size } => self.chamfer(*a, *b, *keep_a, *keep_b, *size),
            Operation::Split { entity, at, second } => self.split(*entity, *at, *second),
            Operation::Trim { entity: id, at, options } => self.trim_interval(*id, *at, *options),
            Operation::TrimCorner { a, b, keep_a, keep_b, options } => {
                self.trim_corner(*a, *b, *keep_a, *keep_b, *options)
            }
            Operation::TrimBounded { entity, a, b, outside, options } => {
                let preview = self.trim_bounded_preview(*entity, *a, *b, *outside, *options)?;
                self.replace_trim_entity(*entity, preview, *options)
            }
            Operation::Extend { entity: id, end, boundary } => self.extend_to(*id, *end, *boundary),
            Operation::Offset { entities, distance, options } => self.offset_chain(entities, *distance, *options),
            Operation::EditOffset { group, distance } => self.edit_offset(*group, *distance),
            Operation::Move { entities, displacement } => {
                if !finite(*displacement) || displacement.len() <= EPS {
                    return refuse("move needs a finite nonzero displacement");
                }
                self.transform_selection(entities, false, 1.0, |p| p + *displacement)
            }
            Operation::Copy { entities, base, displacement } => {
                if !finite(*base) || !finite(*displacement) || displacement.len() <= EPS {
                    return refuse("copy needs a finite base and a finite nonzero displacement");
                }
                self.transform_selection(entities, true, 1.0, |p| p + *displacement)
            }
            Operation::Rotate { entities, center, angle_degrees, copy } => {
                if !finite(*center) || !angle_degrees.is_finite() || angle_degrees.rem_euclid(360.0) < 1e-10 {
                    return refuse("rotate needs a finite centre and a nonzero finite rotation");
                }
                let (sin, cos) = rotation_sin_cos(*angle_degrees);
                self.transform_selection(entities, *copy, 1.0, |p| {
                    let d = p - *center;
                    *center + P2::new(cos * d.u - sin * d.v, sin * d.u + cos * d.v)
                })
            }
            Operation::Scale { entities, base, factor } => {
                if !finite(*base) || !factor.is_finite() || *factor <= 0.0 || (*factor - 1.0).abs() < 1e-10 {
                    return refuse("scale needs a finite base and a positive finite factor other than one");
                }
                self.transform_selection(entities, false, *factor, |p| *base + (p - *base).scale(*factor))
            }
            Operation::MirrorLinked { entities, axis } => self.mirror_linked(entities, *axis),
            Operation::Mirror { entities, axis_start, axis_end } => {
                if !finite(*axis_start) || !finite(*axis_end) || axis_start.dist(*axis_end) <= EPS {
                    return refuse("the mirror axis needs two distinct finite points");
                }
                let dir = (*axis_end - *axis_start).normalized();
                self.transform_copies(entities, 2, true, |_| {
                    move |p| {
                        let v = p - *axis_start;
                        *axis_start + dir.scale(2.0 * v.dot(dir)) - v
                    }
                })
            }
            Operation::Pattern { entities, spec, skipped } => self.create_pattern(entities, *spec, skipped),
            Operation::EditPattern { group, spec, skipped } => self.edit_pattern(*group, *spec, skipped),
            Operation::LinearPattern { entities, step, count } => {
                if !finite(*step) || step.len() <= EPS {
                    return refuse("pattern step must be finite and nonzero");
                }
                self.transform_copies(entities, *count, false, |i| {
                    let delta = step.scale(f64::from(i));
                    move |p| p + delta
                })
            }
            Operation::CircularPattern { entities, center, angle_degrees, count } => {
                if !(2..=1000).contains(count) {
                    return refuse("pattern count including the seed must be from 2 to 1000");
                }
                if !finite(*center) || !angle_degrees.is_finite() || angle_degrees.rem_euclid(360.0).abs() < 1e-10 {
                    return refuse("circular pattern needs a finite centre and a nonzero rotation");
                }
                // Reject repeated seed positions, including a duplicated final full-turn instance.
                for i in 1..*count {
                    if (angle_degrees * f64::from(i))
                        .rem_euclid(360.0)
                        .min(360.0 - (angle_degrees * f64::from(i)).rem_euclid(360.0))
                        < 1e-8
                    {
                        return refuse("circular pattern repeats an instance");
                    }
                }
                self.transform_copies(entities, *count, false, |i| {
                    let (s, c) = rotation_sin_cos(angle_degrees * f64::from(i));
                    move |p: P2| {
                        let d = p - *center;
                        *center + P2::new(c * d.u - s * d.v, s * d.u + c * d.v)
                    }
                })
            }
        }
    }

    fn transform_selection(
        &mut self,
        ids: &[EntityId],
        copy: bool,
        scale: f64,
        map: impl Fn(P2) -> P2,
    ) -> Result<Vec<EntityId>> {
        let mut selected = self.tool_selection(ids)?;
        for e in &mut selected {
            for index in 0..e.point_count() {
                e.set_point(index, map(e.point(index).expect("entity point")));
            }
            if let EntityKind::Circle { radius, .. } = &mut e.kind {
                *radius *= scale;
            }
            if !valid(e.kind) {
                return refuse("the transform would produce non-finite or degenerate geometry");
            }
        }
        if copy {
            return self.add_tool_entities(selected.into_iter().map(|e| (e.kind, e.construction)).collect());
        }
        // Solve on a trial sketch. Existing dimensions, fixed geometry and relations
        // must all hold at the requested result without moving unselected geometry.
        let mut trial = self.clone();
        for e in selected {
            *trial.entity_mut(e.id).expect("validated selection") = e;
        }
        let expected = trial.entities.clone();
        crate::solve::solve(&mut trial, None, true);
        let compatible = trial.entities.iter().zip(&expected).all(|(actual, expected)| {
            valid(actual.kind)
                && actual.points().iter().zip(expected.points()).all(|(a, b)| a.dist(b) <= EPS)
                && match (actual.radius(), expected.radius()) {
                    (Some(a), Some(b)) => (a - b).abs() <= EPS,
                    _ => true,
                }
        });
        if !compatible || matches!(trial.status, crate::SolveStatus::OverDefined { .. } | crate::SolveStatus::Failed(_))
        {
            return refuse(
                "constraints prevent this transform; include connected geometry or remove the blocking relations",
            );
        }
        *self = trial;
        Ok(ids.to_vec())
    }

    fn transform_copies<F: Fn(P2) -> P2>(
        &mut self,
        ids: &[EntityId],
        count: u32,
        reflect: bool,
        transform: impl Fn(u32) -> F,
    ) -> Result<Vec<EntityId>> {
        if !(2..=1000).contains(&count) {
            return refuse("pattern count including the seed must be from 2 to 1000");
        }
        let selected = self.tool_selection(ids)?;
        let mut out = Vec::new();
        for i in 1..count {
            let map = transform(i);
            for e in &selected {
                let kind = match e.kind {
                    EntityKind::Spline { mode, ref points } => {
                        EntityKind::Spline { mode, points: points.iter().copied().map(&map).collect() }
                    }
                    EntityKind::Ellipse { center, major, minor } => {
                        EntityKind::Ellipse { center: map(center), major: map(major), minor: map(minor) }
                    }
                    EntityKind::EllipticArc { center, major, minor, start, end } => {
                        let (start, end) = if reflect { (end, start) } else { (start, end) };
                        EntityKind::EllipticArc {
                            center: map(center),
                            major: map(major),
                            minor: map(minor),
                            start: map(start),
                            end: map(end),
                        }
                    }
                    EntityKind::Point { p } => EntityKind::Point { p: map(p) },
                    EntityKind::Line { a, b } => EntityKind::Line { a: map(a), b: map(b) },
                    EntityKind::Arc { center, start, end } => {
                        let (start, end) = if reflect { (end, start) } else { (start, end) };
                        EntityKind::Arc { center: map(center), start: map(start), end: map(end) }
                    }
                    EntityKind::Circle { center, radius } => EntityKind::Circle { center: map(center), radius },
                };
                out.push((kind, e.construction));
            }
        }
        self.add_tool_entities(out)
    }

    fn replace_tool_entity(&mut self, id: EntityId, pieces: Vec<EntityKind>) -> Result<Vec<EntityId>> {
        if pieces.iter().any(|k| !valid(k)) {
            return refuse("the edit would produce degenerate geometry");
        }
        let old = self.tool_entity(id)?;
        self.constraints.retain(|c| !c.entities().contains(&id));
        let mut ids = Vec::with_capacity(pieces.len());
        for (i, kind) in pieces.into_iter().enumerate() {
            if i == 0 {
                self.entity_mut(id).expect("target exists").kind = kind;
                ids.push(id);
            } else {
                ids.push(self.add(kind, old.construction));
            }
        }
        if ids.is_empty() {
            self.remove_entity(id);
        }
        Ok(ids)
    }

    fn trim_target(&self, id: EntityId, options: TrimOptions) -> Result<Entity> {
        let target = self.tool_entity(id)?;
        if matches!(target.kind, EntityKind::Spline { .. }) {
            return refuse("trim does not yet support spline targets");
        }
        if crate::ellipse::Ellipse::of(&target.kind).is_some() {
            return refuse("trim does not yet support ellipse targets");
        }
        if target.construction && (options.ignore_construction || options.keep_as_construction) {
            return refuse("construction targets are kept by the current trim options");
        }
        Ok(target)
    }

    /// What trimming `id` at `at` removes and keeps, without changing the sketch.
    pub fn trim_preview(&self, id: EntityId, at: P2, options: TrimOptions) -> Result<TrimPreview> {
        let target = self.trim_target(id, options)?;
        if self.drawn().any(|e| matches!(e.kind, EntityKind::Spline { .. })) {
            return refuse("trim does not yet support spline boundaries");
        }
        if self.drawn().any(|e| crate::ellipse::Ellipse::of(&e.kind).is_some()) {
            return refuse("trim does not yet support ellipse boundaries");
        }
        if !finite(at) || target.is_point() {
            return refuse("trim needs a line, arc or circle and a finite pick point");
        }
        let target_kind = &target.kind;
        let crossings: Vec<P2> = self
            .drawn()
            .filter(|e| e.id != id)
            .flat_map(|other| {
                intersections(&target.kind, &other.kind)
                    .into_iter()
                    .filter(move |p| on_curve(target_kind, *p) && on_curve(&other.kind, *p))
            })
            .collect();
        if let EntityKind::Circle { center, .. } = target.kind {
            let angle = |p: P2| (p - center).angle().rem_euclid(TAU);
            let mut cuts = crossings;
            cuts.sort_by(|a, b| angle(*a).total_cmp(&angle(*b)));
            cuts.dedup_by(|a, b| a.dist(*b) <= EPS);
            if cuts.len() > 1 && cuts[0].dist(*cuts.last().unwrap()) <= EPS {
                cuts.pop();
            }
            let pick = target.nearest(at).0;
            if cuts.iter().any(|p| p.dist(pick) <= EPS) {
                return refuse("pick inside a trim interval, not at an intersection");
            }
            if cuts.len() < 2 {
                return Ok(TrimPreview { removed: vec![target.kind], kept: Vec::new() });
            }
            let next = cuts.iter().position(|p| angle(*p) > angle(pick)).unwrap_or(0);
            let previous = (next + cuts.len() - 1) % cuts.len();
            return Ok(TrimPreview {
                removed: vec![EntityKind::Arc { center, start: cuts[previous], end: cuts[next] }],
                kept: vec![EntityKind::Arc { center, start: cuts[next], end: cuts[previous] }],
            });
        }
        let mut cuts = vec![0.0, 1.0];
        cuts.extend(crossings.into_iter().map(|p| parameter(&target.kind, p).expect("line or arc").clamp(0.0, 1.0)));
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| point_at(&target.kind, *a).dist(point_at(&target.kind, *b)) <= EPS);
        let p = target.nearest(at).0;
        let t = parameter(&target.kind, p).expect("line or arc");
        if cuts.iter().any(|c| point_at(&target.kind, *c).dist(p) <= EPS) {
            return refuse("pick inside a trim interval, not at an endpoint or intersection");
        }
        let Some(pair) = cuts.windows(2).find(|w| w[0] < t && t < w[1]) else {
            return refuse("no trim interval at this point");
        };
        let piece = |a: f64, b: f64| {
            let mut kind = target.kind.clone();
            set_end(&mut kind, End::Start, point_at(&target.kind, a));
            set_end(&mut kind, End::End, point_at(&target.kind, b));
            kind
        };
        let kept = [(0.0, pair[0]), (pair[1], 1.0)]
            .into_iter()
            .filter(|(a, b)| point_at(&target.kind, *a).dist(point_at(&target.kind, *b)) > EPS)
            .map(|(a, b)| piece(a, b))
            .collect();
        Ok(TrimPreview { removed: vec![piece(pair[0], pair[1])], kept })
    }

    /// Freely extends a line or arc along its supporting curve toward a pointer.
    /// Picks within the current span do nothing; refusals leave the sketch unchanged.
    pub fn extend_drag(&mut self, id: EntityId, at: P2, options: TrimOptions) -> Result<bool> {
        if options.ignore_construction && self.tool_entity(id)?.construction {
            return refuse("construction targets are protected from trim extension");
        }
        let target = self.tool_entity(id)?;
        if matches!(target.kind, EntityKind::Spline { .. }) {
            return refuse("extend does not yet support spline targets");
        }
        if !finite(at) {
            return refuse("extension needs a finite pointer position");
        }
        let mut kind = target.kind.clone();
        match kind {
            EntityKind::Line { a, b } => {
                let t = parameter(&kind, at).expect("line");
                if (0.0..=1.0).contains(&t) {
                    return Ok(false);
                }
                set_end(&mut kind, if t < 0.0 { End::Start } else { End::End }, a.lerp(b, t));
            }
            EntityKind::Arc { center, start, end } => {
                if center.dist(at) <= EPS {
                    return refuse("move away from the arc centre to choose an extension direction");
                }
                let p = center + (at - center).normalized().scale(center.dist(start));
                if on_curve(&kind, p) {
                    return Ok(false);
                }
                let before = ((start - center).angle() - (p - center).angle()).rem_euclid(TAU);
                let after = ((p - center).angle() - (end - center).angle()).rem_euclid(TAU);
                set_end(&mut kind, if before < after { End::Start } else { End::End }, p);
            }
            _ => return refuse("only lines and arcs have endpoints to extend"),
        }
        self.replace_tool_entity(id, vec![kind])?;
        Ok(true)
    }

    /// Trims every curve crossed between pointer samples, in stroke order.
    /// The open start avoids recutting a curve just restored at a trail marker.
    /// Returns a checkpoint before each cut. A refusal leaves the sketch unchanged.
    pub fn trim_stroke(&mut self, from: P2, to: P2, options: TrimOptions) -> Result<Vec<TrimCheckpoint>> {
        let stroke = EntityKind::Line { a: from, b: to };
        if !valid(&stroke) {
            return refuse("a trim stroke needs a finite nonzero length");
        }
        let direction = to - from;
        let mut hits = Vec::new();
        for target in self.drawn().filter(|e| {
            !e.is_point()
                && valid(&e.kind)
                && !(e.construction && (options.ignore_construction || options.keep_as_construction))
        }) {
            for at in intersections(&stroke, &target.kind) {
                let tangent = circle(&target.kind)
                    .is_some_and(|(center, _)| direction.dot(at - center).abs() <= EPS * direction.len());
                if !tangent && from.dist(at) > EPS && on_curve(&stroke, at) && on_curve(&target.kind, at) {
                    hits.push((from.dist(at), target.id, at));
                }
            }
        }
        if hits.is_empty() {
            return Ok(Vec::new());
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut trial = self.clone();
        let mut families = std::collections::HashMap::<EntityId, Vec<EntityId>>::new();
        let mut checkpoints = Vec::new();
        for (_, original, at) in hits {
            let family = families.entry(original).or_insert_with(|| vec![original]);
            // Earlier cuts can remove boundaries, so check eligibility against the evolving sketch.
            let target = family.iter().copied().find(|id| {
                trial.entity(*id).is_some_and(|e| on_curve(&e.kind, at)) && trial.trim_preview(*id, at, options).is_ok()
            });
            let Some(target) = target else { continue };
            let entity_index = trial.entities.iter().position(|e| e.id == target).expect("trim target exists");
            let mut checkpoint = TrimCheckpoint {
                at,
                direction,
                entity: trial.entities[entity_index].clone(),
                entity_index,
                next_entity: trial.next_entity,
                next_constraint: trial.next_constraint,
                next_pattern: trial.next_pattern,
                constraints: Vec::new(),
                patterns: Vec::new(),
            };
            let before_constraints = trial.constraints.clone();
            let before_patterns = trial.patterns.clone();
            let kept = trial.trim_interval(target, at, options)?;
            // Include every change from pattern dissolution, not only relations on the target.
            let after_constraints: std::collections::HashMap<_, _> =
                trial.constraints.iter().map(|c| (c.id, c)).collect();
            checkpoint.constraints = before_constraints
                .into_iter()
                .enumerate()
                .filter(|(_, old)| after_constraints.get(&old.id).copied() != Some(old))
                .collect();
            let after_patterns: std::collections::HashMap<_, _> = trial.patterns.iter().map(|p| (p.id, p)).collect();
            checkpoint.patterns = before_patterns
                .into_iter()
                .enumerate()
                .filter(|(_, old)| after_patterns.get(&old.id).copied() != Some(old))
                .collect();
            family.retain(|id| *id != target);
            family.extend(kept);
            checkpoints.push(checkpoint);
        }
        if !checkpoints.is_empty() {
            *self = trial;
        }
        Ok(checkpoints)
    }

    /// Previews a corner from two lines/arcs and the portions the user wants to keep.
    /// Multiple intersections use the nearest combined pick distance; ties are refused.
    pub fn corner_preview(
        &self,
        a: EntityId,
        b: EntityId,
        keep_a: P2,
        keep_b: P2,
        options: TrimOptions,
    ) -> Result<CornerPreview> {
        if a == b || !finite(keep_a) || !finite(keep_b) {
            return refuse("corner trim needs two distinct curves and finite keep points");
        }
        let first = self.tool_entity(a)?;
        let second = self.tool_entity(b)?;
        if matches!(first.kind, EntityKind::Spline { .. }) || matches!(second.kind, EntityKind::Spline { .. }) {
            return refuse("corner trim does not yet support spline geometry");
        }
        if ends(&first.kind).is_none() || ends(&second.kind).is_none() {
            return refuse("corner trim needs two lines or arcs; a closed circle has no endpoint");
        }
        let keep_a = first.nearest(keep_a).0;
        let keep_b = second.nearest(keep_b).0;
        let mut hits = intersections(&first.kind, &second.kind);
        hits.dedup_by(|a, b| a.dist(*b) <= EPS);
        let mut choices: Vec<_> = hits
            .into_iter()
            .map(|at| (at.dist(keep_a) + at.dist(keep_b), at))
            .filter(|(score, _)| score.is_finite())
            .collect();
        choices.sort_by(|a, b| a.0.total_cmp(&b.0));
        let Some(&(score, at)) = choices.first() else {
            return refuse("the curves have no unique corner; parallel or coincident curves cannot join this way");
        };
        if choices.get(1).is_some_and(|(other, _)| (other - score).abs() <= EPS) {
            return refuse("two corners match these picks; pick closer to the intended corner");
        }
        // Keep existing endpoints exact rather than moving them by intersection round-off.
        let at = ends(&first.kind)
            .expect("line or arc")
            .into_iter()
            .chain(ends(&second.kind).expect("line or arc"))
            .find(|p| p.dist(at) <= EPS)
            .unwrap_or(at);
        let (ka, end_a) = corner_piece(first.kind.clone(), keep_a, at)?;
        let (kb, end_b) = corner_piece(second.kind.clone(), keep_b, at)?;
        if options.ignore_construction
            && ((first.construction && ka != first.kind) || (second.construction && kb != second.kind))
        {
            return refuse("a corner cannot change a protected construction target");
        }
        let pa = PointRef::new(a, usize::from(first.is_curve()) + usize::from(end_a == End::End));
        let pb = PointRef::new(b, usize::from(second.is_curve()) + usize::from(end_b == End::End));
        let joined = |c: &Constraint| matches!(c.kind, ConstraintKind::Coincident { a, b } if (a == pa && b == pb) || (a == pb && b == pa));
        if ka == first.kind && kb == second.kind && self.constraints.iter().any(joined) {
            return refuse("these curves already share this corner");
        }
        Ok(CornerPreview {
            kept: [ka.clone(), kb.clone()],
            removed: [corner_removed(first.kind, ka, end_a), corner_removed(second.kind, kb, end_b)]
                .into_iter()
                .flatten()
                .collect(),
            joint: [pa, pb],
        })
    }

    fn trim_corner(
        &mut self,
        a: EntityId,
        b: EntityId,
        keep_a: P2,
        keep_b: P2,
        options: TrimOptions,
    ) -> Result<Vec<EntityId>> {
        let preview = self.corner_preview(a, b, keep_a, keep_b, options)?;
        let mut trial = self.clone();
        for (id, kind) in [a, b].into_iter().zip(preview.kept) {
            if trial.entity(id).expect("corner target validated").kind != kind {
                trial.replace_tool_entity(id, vec![kind])?;
            }
        }
        // Changed targets lose their old joint; unchanged joined pairs were refused above.
        trial.constrain(ConstraintKind::Coincident { a: preview.joint[0], b: preview.joint[1] });
        let mut ids = vec![a, b];
        if options.keep_as_construction {
            for kind in preview.removed {
                ids.push(trial.add(kind, true));
            }
        }
        *self = trial;
        Ok(ids)
    }

    /// Trims an open target between two distinct, transverse boundary crossings.
    /// Other curves do not contribute cuts. Zero/multiple crossings are refused.
    pub fn trim_bounded_preview(
        &self,
        id: EntityId,
        a: EntityId,
        b: EntityId,
        outside: bool,
        options: TrimOptions,
    ) -> Result<TrimPreview> {
        if id == a || id == b || a == b {
            return refuse("pick two distinct bounds and a different target");
        }
        let target = self.trim_target(id, options)?;
        let Some([start, end]) = ends(&target.kind) else {
            return refuse("inside/outside trim needs an open line or arc");
        };
        let tangent = |kind: &EntityKind, at: P2| match *kind {
            EntityKind::Spline { mode, ref points } => {
                let nearest = crate::spline::nearest(mode, points, at);
                crate::spline::spans(mode, points)
                    .map(|spans| spans[nearest.span].derivative(nearest.parameter).normalized())
                    .unwrap_or(P2::ZERO)
            }
            EntityKind::Line { a, b } => (b - a).normalized(),
            EntityKind::Arc { center, .. } | EntityKind::Circle { center, .. } => (at - center).perp().normalized(),
            ref kind @ (EntityKind::Ellipse { .. } | EntityKind::EllipticArc { .. }) => {
                let ellipse = crate::ellipse::Ellipse::of(kind).unwrap();
                let t = ellipse.parameter(at);
                (ellipse.axis().scale(-t.sin()) + ellipse.axis().perp().scale(ellipse.ratio() * t.cos())).normalized()
            }
            EntityKind::Point { .. } => P2::ZERO,
        };
        let mut cuts = Vec::with_capacity(2);
        for bound in [a, b] {
            let boundary = self.tool_entity(bound)?;
            if matches!(boundary.kind, EntityKind::Spline { .. }) {
                return refuse("bounded trim does not yet support spline boundaries");
            }
            if crate::ellipse::Ellipse::of(&boundary.kind).is_some() {
                return refuse("bounded trim does not yet support ellipse boundaries");
            }
            let mut hits: Vec<_> = intersections(&target.kind, &boundary.kind)
                .into_iter()
                .filter(|p| on_curve(&target.kind, *p) && on_curve(&boundary.kind, *p))
                .collect();
            hits.dedup_by(|p, q| p.dist(*q) <= EPS);
            if hits.len() != 1 {
                return refuse("each bound must cross the open target exactly once");
            }
            let mut at = hits[0];
            if tangent(&target.kind, at).cross(tangent(&boundary.kind, at)).abs() <= 1e-9 {
                return refuse("a tangent contact does not bound a trim interval");
            }
            if let Some(p) = [start, end].into_iter().find(|p| p.dist(at) <= EPS) {
                at = p;
            }
            cuts.push((parameter(&target.kind, at).expect("open target").clamp(0.0, 1.0), at));
        }
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let [(lo, p), (hi, q)] = [cuts[0], cuts[1]];
        if lo >= hi || p.dist(q) <= EPS {
            return refuse("the bounds must make two distinct crossings");
        }
        let piece = |from: P2, to: P2| {
            let mut kind = target.kind.clone();
            set_end(&mut kind, End::Start, from);
            set_end(&mut kind, End::End, to);
            kind
        };
        let inside = vec![piece(p, q)];
        let mut tails = Vec::new();
        if lo > 0.0 {
            tails.push(piece(start, p));
        }
        if hi < 1.0 {
            tails.push(piece(q, end));
        }
        let (removed, kept) = if outside { (tails, inside) } else { (inside, tails) };
        if removed.is_empty() {
            return refuse("no target geometry lies outside these bounds");
        }
        if removed.iter().chain(&kept).any(|kind| !valid(kind)) {
            return refuse("the bounds would produce degenerate geometry");
        }
        Ok(TrimPreview { removed, kept })
    }

    fn replace_trim_entity(
        &mut self,
        id: EntityId,
        preview: TrimPreview,
        options: TrimOptions,
    ) -> Result<Vec<EntityId>> {
        if options.keep_as_construction && preview.removed.iter().any(|kind| !valid(kind)) {
            return refuse("the trim would create degenerate construction geometry");
        }
        if options.keep_as_construction && preview.kept.is_empty() {
            self.entity_mut(id).expect("trim target validated").construction = true;
            return Ok(vec![id]);
        }
        let mut ids = self.replace_tool_entity(id, preview.kept)?;
        if options.keep_as_construction {
            for kind in preview.removed {
                ids.push(self.add(kind, true));
            }
        }
        Ok(ids)
    }

    fn trim_interval(&mut self, id: EntityId, at: P2, options: TrimOptions) -> Result<Vec<EntityId>> {
        let preview = self.trim_preview(id, at, options)?;
        self.replace_trim_entity(id, preview, options)
    }

    /// The bounded entity a pointer extend of `id` meets first beyond `end`: the nearest
    /// intersection over every other drawn line, arc and circle. `None` when nothing lies
    /// beyond that end, or when `id` is not a line or arc.
    pub fn extend_boundary(&self, id: EntityId, end: End) -> Option<EntityId> {
        let target = self.tool_entity(id).ok()?;
        ends(&target.kind)?;
        self.drawn()
            .filter(|e| e.id != id && valid(&e.kind) && !e.is_point())
            .filter_map(|limit| extension(&target, end, limit).map(|(distance, _)| (distance, limit.id)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }

    fn extend_to(&mut self, id: EntityId, end: End, boundary: EntityId) -> Result<Vec<EntityId>> {
        if id == boundary {
            return refuse("extension boundary must differ from the target");
        }
        let target = self.tool_entity(id)?;
        let limit = self.tool_entity(boundary)?;
        if matches!(target.kind, EntityKind::Spline { .. }) || matches!(limit.kind, EntityKind::Spline { .. }) {
            return refuse("extend does not yet support spline targets or boundaries");
        }
        if crate::ellipse::Ellipse::of(&limit.kind).is_some() {
            return refuse("extend does not yet support ellipse boundaries");
        }
        if ends(&target.kind).is_none() {
            return refuse("extend needs a line or arc");
        }
        let Some((_, p)) = extension(&target, end, &limit) else {
            return refuse("the boundary has no intersection beyond that endpoint");
        };
        let mut kind = target.kind.clone();
        set_end(&mut kind, end, p);
        self.replace_tool_entity(id, vec![kind])
    }
}
