//! Editable sketch arrays with stable instance identities.

use super::*;
use crate::SolveStatus;

/// One direction of a rectangular array; the count includes its seed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternDirection {
    /// Translation between neighboring instances, mm.
    pub step: P2,
    /// Positions in this direction, including the seed.
    pub count: u32,
}

/// The dimensions and distribution of an editable pattern.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PatternSpec {
    /// One direction or a two-direction grid.
    Linear {
        /// Translation in the first direction, mm.
        step: P2,
        /// Positions in the first direction, including the seed.
        count: u32,
        /// Optional second direction.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        second: Option<PatternDirection>,
    },
    /// Rotations about a fixed point in the sketch plane.
    Circular {
        /// Centre of rotation.
        center: P2,
        /// Angle per step, or the full span when `total_angle` is true.
        angle_degrees: f64,
        /// Positions including the seed.
        count: u32,
        /// A full turn has no repeated final position; partial spans include both ends.
        #[serde(default)]
        total_angle: bool,
    },
}

/// Generated entity ids at one zero-based [column, row] position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternInstance {
    /// Circular patterns use [index, 0]. The seed is [0, 0].
    pub position: [u32; 2],
    /// One generated entity for each seed, in seed order.
    pub entities: Vec<EntityId>,
}

/// A retained definition, including positions whose geometry is skipped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatternDefinition {
    /// Stable sketch-local pattern id.
    pub id: u32,
    /// Source geometry, in selection order.
    pub seeds: Vec<EntityId>,
    /// Editable dimensions and distribution.
    pub spec: PatternSpec,
    /// Omitted zero-based positions. The seed cannot be skipped.
    pub skipped: Vec<[u32; 2]>,
    /// Surviving generated positions and their stable entity ids.
    pub instances: Vec<PatternInstance>,
}

#[derive(Clone, Copy)]
struct Placement {
    position: [u32; 2],
    translation: P2,
    center: P2,
    angle: f64,
}

impl PatternSpec {
    /// All zero-based positions, including the seed, after validating the dimensions.
    pub fn positions(self) -> Result<Vec<[u32; 2]>> {
        Ok(self.placements()?.into_iter().map(|p| p.position).collect())
    }

    fn placements(self) -> Result<Vec<Placement>> {
        let mut out = Vec::new();
        match self {
            Self::Linear { step, count, second } => {
                let second = second.unwrap_or(PatternDirection { step: P2::ZERO, count: 1 });
                let total = count.saturating_mul(second.count);
                if count == 0 || second.count == 0 || !(2..=1000).contains(&total) {
                    return refuse("a pattern needs 2 to 1000 positions including the seed");
                }
                if !finite(step)
                    || !finite(second.step)
                    || (count > 1 && step.len() <= EPS)
                    || (second.count > 1 && second.step.len() <= EPS)
                {
                    return refuse("each active pattern direction needs a finite nonzero step");
                }
                for row in 0..second.count {
                    for column in 0..count {
                        let translation = step.scale(f64::from(column)) + second.step.scale(f64::from(row));
                        if !finite(translation) {
                            return refuse("pattern translations exceed the finite coordinate range");
                        }
                        if out.iter().any(|p: &Placement| p.translation.dist(translation) <= EPS) {
                            return refuse("the two pattern directions repeat a position");
                        }
                        out.push(Placement { position: [column, row], translation, center: P2::ZERO, angle: 0.0 });
                    }
                }
            }
            Self::Circular { center, angle_degrees, count, total_angle } => {
                if !(2..=1000).contains(&count) || !finite(center) || !angle_degrees.is_finite() {
                    return refuse("a circular pattern needs a finite centre, angle and 2 to 1000 positions");
                }
                if total_angle && angle_degrees.abs() > 360.0 + 1e-8 {
                    return refuse("the total pattern angle must be at most one full turn");
                }
                let full = total_angle && (angle_degrees.abs() - 360.0).abs() <= 1e-8;
                let step = if total_angle {
                    angle_degrees / f64::from(if full { count } else { count - 1 })
                } else {
                    angle_degrees
                };
                if step.rem_euclid(360.0).min(360.0 - step.rem_euclid(360.0)) < 1e-8 {
                    return refuse("the circular pattern step must not be zero or a full turn");
                }
                for index in 0..count {
                    let angle = step * f64::from(index);
                    if !angle.is_finite() || (index > 0 && pattern_axes(center, angle).is_none()) {
                        return refuse("pattern rotations exceed the finite coordinate range");
                    }
                    let normalized = angle.rem_euclid(360.0);
                    if index > 0 && normalized.min(360.0 - normalized) < 1e-8 {
                        return refuse("the circular pattern repeats a position");
                    }
                    out.push(Placement { position: [index, 0], translation: P2::ZERO, center, angle });
                }
            }
        }
        Ok(out)
    }
}

pub(crate) fn pattern_axes(center: P2, angle: f64) -> Option<(P2, P2, P2)> {
    let length = center.u.abs().max(center.v.abs()).mul_add(1e-6, 0.0).max(1.0);
    let (sin, cos) = rotation_sin_cos(angle / 2.0);
    let endpoint = |direction: P2| {
        [center + direction.scale(length), center - direction.scale(length)]
            .into_iter()
            .find(|p| finite(*p) && p.dist(center) > EPS)
    };
    Some((center, endpoint(P2::new(1.0, 0.0))?, endpoint(P2::new(cos, sin))?))
}

fn transformed(kind: EntityKind, p: Placement) -> EntityKind {
    let (sin, cos) = rotation_sin_cos(p.angle);
    let map = |at: P2| {
        let d = at - p.center;
        p.center + P2::new(cos * d.u - sin * d.v, sin * d.u + cos * d.v) + p.translation
    };
    match kind {
        EntityKind::Text { text } => EntityKind::Text { text: text.mapped(map) },
        EntityKind::Spline { mode, points } => {
            EntityKind::Spline { mode, points: points.into_iter().map(map).collect() }
        }
        EntityKind::Point { p } => EntityKind::Point { p: map(p) },
        EntityKind::Line { a, b } => EntityKind::Line { a: map(a), b: map(b) },
        EntityKind::Circle { center, radius } => EntityKind::Circle { center: map(center), radius },
        EntityKind::Arc { center, start, end } => {
            EntityKind::Arc { center: map(center), start: map(start), end: map(end) }
        }
        EntityKind::Ellipse { center, major, minor } => {
            EntityKind::Ellipse { center: map(center), major: map(major), minor: map(minor) }
        }
        EntityKind::EllipticArc { center, major, minor, start, end } => EntityKind::EllipticArc {
            center: map(center),
            major: map(major),
            minor: map(minor),
            start: map(start),
            end: map(end),
        },
    }
}

impl Sketch {
    pub(super) fn create_pattern(
        &mut self,
        seeds: &[EntityId],
        spec: PatternSpec,
        skipped: &[[u32; 2]],
    ) -> Result<Vec<EntityId>> {
        self.tool_selection(seeds)?;
        let mut trial = self.clone();
        let group = trial.next_pattern;
        trial.next_pattern = group.checked_add(1).ok_or_else(|| ToolError("no pattern ids remain".into()))?;
        let definition =
            PatternDefinition { id: group, seeds: seeds.to_vec(), spec, skipped: Vec::new(), instances: Vec::new() };
        let ids = trial.rebuild_pattern(definition, spec, skipped)?;
        *self = trial;
        Ok(ids)
    }

    pub(super) fn edit_pattern(
        &mut self,
        group: u32,
        spec: PatternSpec,
        skipped: &[[u32; 2]],
    ) -> Result<Vec<EntityId>> {
        let definition = self
            .patterns
            .iter()
            .find(|p| p.id == group)
            .cloned()
            .ok_or_else(|| ToolError("the pattern no longer exists".into()))?;
        let mut trial = self.clone();
        let ids = trial.rebuild_pattern(definition, spec, skipped)?;
        crate::solve::solve(&mut trial, None, true);
        if matches!(trial.status, SolveStatus::Failed(_) | SolveStatus::OverDefined { .. }) {
            return refuse("the pattern edit conflicts with existing sketch relations");
        }
        *self = trial;
        Ok(ids)
    }

    fn rebuild_pattern(
        &mut self,
        old: PatternDefinition,
        spec: PatternSpec,
        skipped: &[[u32; 2]],
    ) -> Result<Vec<EntityId>> {
        let sources = self.tool_selection(&old.seeds)?;
        if sources.iter().any(|e| matches!(e.kind, EntityKind::Text { .. })) {
            return refuse("Retained patterns do not yet support sketch text");
        }
        let placements = spec.placements()?;
        let positions: Vec<_> = placements.iter().map(|p| p.position).collect();
        let mut skipped = skipped.to_vec();
        skipped.sort_unstable();
        if skipped.windows(2).any(|p| p[0] == p[1])
            || skipped.contains(&[0, 0])
            || skipped.iter().any(|p| !positions.contains(p))
        {
            return refuse("skip distinct existing positions; the seed cannot be skipped");
        }
        let mut geometry = Vec::new();
        let mut instances = Vec::new();
        for p in placements.into_iter().skip(1).filter(|p| !skipped.contains(&p.position)) {
            let previous = old.instances.iter().find(|i| i.position == p.position);
            if previous.is_some_and(|i| i.entities.len() != sources.len()) {
                return refuse("a pattern instance is incomplete; recreate the pattern");
            }
            let mut ids = Vec::new();
            for (index, source) in sources.iter().enumerate() {
                let kind = transformed(source.kind.clone(), p);
                if !valid(&kind) {
                    return refuse("the pattern would produce non-finite or degenerate geometry");
                }
                let existing = previous.map(|i| i.entities[index]);
                if let Some(id) = existing {
                    let e = self.tool_entity(id)?;
                    if std::mem::discriminant(&e.kind) != std::mem::discriminant(&kind) {
                        return refuse("a pattern member changed type; recreate the pattern");
                    }
                }
                geometry.push((p, source.clone(), existing, kind));
                ids.push(existing);
            }
            instances.push((p.position, ids));
        }
        let kept: Vec<_> = geometry.iter().filter_map(|(_, _, id, _)| *id).collect();
        let removed: Vec<_> =
            old.instances.iter().flat_map(|i| i.entities.iter().copied()).filter(|id| !kept.contains(id)).collect();
        let owned =
            |c: &Constraint| matches!(c.kind, ConstraintKind::PatternTransform { group, .. } if group == old.id);
        if self.constraints.iter().any(|c| !owned(c) && c.entities().iter().any(|id| removed.contains(id)))
            || self.patterns.iter().any(|p| p.id != old.id && p.seeds.iter().any(|id| removed.contains(id)))
        {
            return refuse("remove relations or dependent patterns before skipping a constrained instance");
        }
        let added = geometry.iter().filter(|(_, _, id, _)| id.is_none()).count();
        self.next_entity
            .checked_add(u32::try_from(added).map_err(|_| ToolError("too many pattern entities".into()))?)
            .ok_or_else(|| ToolError("no entity ids remain".into()))?;
        self.constraints
            .retain(|c| !owned(c) || matches!(c.kind, ConstraintKind::PatternTransform { b, .. } if kept.contains(&b)));
        self.entities.retain(|e| !removed.contains(&e.id));
        let mut out = Vec::new();
        for (p, source, existing, kind) in geometry {
            let id = if let Some(id) = existing {
                self.entity_mut(id).expect("validated member").kind = kind;
                id
            } else {
                self.add(kind, source.construction)
            };
            let kind = ConstraintKind::PatternTransform {
                a: source.id,
                b: id,
                group: old.id,
                translation: p.translation,
                center: p.center,
                angle_degrees: p.angle,
            };
            if let Some(link) = self.constraints.iter_mut().find(|c| {
                matches!(c.kind, ConstraintKind::PatternTransform { a, b, group, .. } if a == source.id && b == id && group == old.id)
            }) {
                link.kind = kind;
            } else {
                self.constrain(kind);
            }
            out.push(id);
        }
        let mut at = 0;
        let instances = instances
            .into_iter()
            .map(|(position, ids)| {
                let entities = out[at..at + ids.len()].to_vec();
                at += ids.len();
                PatternInstance { position, entities }
            })
            .collect();
        self.patterns.retain(|p| p.id != old.id);
        self.patterns.push(PatternDefinition { id: old.id, seeds: old.seeds, spec, skipped, instances });
        self.patterns.sort_by_key(|p| p.id);
        Ok(out)
    }

    /// Dissolve a definition when direct geometry/relation deletion breaks its links.
    /// Remaining geometry stays in place and becomes independent.
    pub(crate) fn reconcile_patterns(&mut self) {
        let broken: Vec<_> = self.patterns.iter().filter(|p| {
            p.seeds.iter().any(|id| self.entity(*id).is_none()) || p.instances.iter().any(|i| {
                i.entities.len() != p.seeds.len() || i.entities.iter().zip(&p.seeds).any(|(&target, &source)| {
                    self.entity(target).is_none() || !self.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::PatternTransform { a, b, group, .. } if a == source && b == target && group == p.id))
                })
            })
        }).map(|p| p.id).collect();
        self.patterns.retain(|p| !broken.contains(&p.id));
        self.constraints
            .retain(|c| !matches!(c.kind, ConstraintKind::PatternTransform { group, .. } if broken.contains(&group)));
    }
}
