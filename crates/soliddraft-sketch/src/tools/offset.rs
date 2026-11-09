//! Linked analytic offsets and ordered chain selection.

use super::*;
use crate::ConstraintId;

/// End treatment for an open offset chain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffsetCap {
    /// Leave the chain open.
    #[default]
    None,
    /// Join endpoints with straight segments.
    Lines,
    /// Join endpoints with semicircular arcs.
    Round,
}

/// Choices shared by the offset property panel and headless operation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OffsetOptions {
    /// Follow connected curves from the first selected seed, preserving its direction.
    pub chain: bool,
    /// Offset both sides at the same distance.
    pub both_sides: bool,
    /// Caps apply to open chains only.
    pub cap: OffsetCap,
    /// Keep the originals as guides after creating the offset geometry.
    pub make_base_construction: bool,
}

impl OffsetOptions {
    pub(super) fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

struct Plan {
    sources: Vec<EntityId>,
    geometry: Vec<(EntityKind, bool)>,
    relations: Vec<ConstraintKind>,
}

fn endpoint(kind: EntityKind, reverse: bool, end: bool) -> u8 {
    u8::from(matches!(kind, EntityKind::Arc { .. })) + u8::from(end != reverse)
}

fn point(kind: EntityKind, index: u8) -> P2 {
    entity(kind).point(index).expect("validated chain endpoint")
}

fn outward(kind: EntityKind, reverse: bool, end: bool) -> P2 {
    let direction = match kind {
        EntityKind::Line { a, b } => (b - a).normalized(),
        EntityKind::Arc { center, .. } => (point(kind, endpoint(kind, reverse, end)) - center).normalized().perp(),
        _ => unreachable!(),
    };
    direction.scale(if reverse { -1.0 } else { 1.0 }).scale(if end { 1.0 } else { -1.0 })
}

impl Sketch {
    /// Find a nonbranching connected chain, walking through the seed in its stored direction.
    pub fn offset_chain_from(&self, seed: EntityId) -> Result<Vec<(EntityId, bool)>> {
        let seed = self.tool_entity(seed)?;
        if matches!(seed.kind, EntityKind::Circle { .. }) {
            return Ok(vec![(seed.id, false)]);
        }
        let Some([start, end]) = ends(seed.kind) else {
            return refuse("offset needs lines, arcs or a lone circle");
        };
        let curves: Vec<_> =
            self.drawn().filter(|e| e.construction == seed.construction && ends(e.kind).is_some()).collect();
        let mut ordered = vec![(seed.id, false)];
        let mut used = vec![seed.id];
        let mut at = end;
        let next = |at: P2, used: &[EntityId]| -> Result<Option<(EntityId, bool)>> {
            let mut degree = 0;
            let mut choices = Vec::new();
            for e in &curves {
                for (index, p) in ends(e.kind).expect("open curve").into_iter().enumerate() {
                    if p.dist(at) <= EPS {
                        degree += 1;
                        if !used.contains(&e.id) {
                            choices.push((e.id, index == 1));
                        }
                    }
                }
            }
            if degree > 2 || choices.len() > 1 {
                return refuse("the chain branches here; select an unbranched chain manually");
            }
            Ok(choices.first().copied())
        };
        while let Some((id, reverse)) = next(at, &used)? {
            let e = self.tool_entity(id)?;
            ordered.push((id, reverse));
            used.push(id);
            at = point(e.kind, endpoint(e.kind, reverse, true));
        }
        if at.dist(start) > EPS {
            at = start;
            while let Some((id, entering_at_end)) = next(at, &used)? {
                let reverse = !entering_at_end;
                let e = self.tool_entity(id)?;
                ordered.insert(0, (id, reverse));
                used.push(id);
                at = point(e.kind, endpoint(e.kind, reverse, false));
            }
        }
        Ok(ordered)
    }

    fn offset_sources(&self, ids: &[EntityId], chain: bool) -> Result<Vec<(Entity, bool)>> {
        let selected = self.tool_selection(ids)?;
        if chain {
            let ordered = self.offset_chain_from(selected[0].id)?;
            if selected.iter().any(|e| !ordered.iter().any(|(id, _)| e.id == *id)) {
                return refuse("the selected curves do not belong to one chain");
            }
            return ordered.into_iter().map(|(id, reverse)| Ok((self.tool_entity(id)?, reverse))).collect();
        }
        let mut previous = None;
        let mut out = Vec::new();
        for e in selected {
            if matches!(e.kind, EntityKind::Circle { .. }) && ids.len() == 1 {
                return Ok(vec![(e, false)]);
            }
            let Some([a, b]) = ends(e.kind) else {
                return refuse("offset needs a connected line/arc chain or a lone circle");
            };
            let reverse = if let Some(p) = previous {
                if a.dist(p) <= EPS {
                    false
                } else if b.dist(p) <= EPS {
                    true
                } else {
                    return refuse("select curves in walking order, or enable chain selection");
                }
            } else {
                false
            };
            previous = Some(if reverse { a } else { b });
            out.push((e, reverse));
        }
        Ok(out)
    }
}

fn shifted(source: &[(Entity, bool)], distance: f64, closed: bool) -> Result<Vec<EntityKind>> {
    if source.len() == 1
        && let EntityKind::Circle { center, radius } = source[0].0.kind
    {
        if radius - distance <= EPS {
            return refuse("offset collapses a circle radius");
        }
        return Ok(vec![EntityKind::Circle { center, radius: radius - distance }]);
    }
    let mut out = Vec::new();
    for &(e, reverse) in source {
        let d = if reverse { -distance } else { distance };
        out.push(match e.kind {
            EntityKind::Line { a, b } => {
                let n = (b - a).normalized().perp().scale(d);
                let kind = EntityKind::Line { a: a + n, b: b + n };
                if reverse { reversed(kind) } else { kind }
            }
            EntityKind::Arc { center, start, end } => {
                let r = center.dist(start);
                if r - d <= EPS {
                    return refuse("offset collapses an arc radius");
                }
                EntityKind::Arc {
                    center,
                    start: center + (start - center).scale((r - d) / r),
                    end: center + (end - center).scale((r - d) / r),
                }
            }
            _ => return refuse("offset needs lines, arcs or a lone circle"),
        });
    }
    let joins = if closed { out.len() } else { out.len() - 1 };
    for i in 0..joins {
        let j = (i + 1) % out.len();
        let ri = source[i].1 && matches!(out[i], EntityKind::Arc { .. });
        let rj = source[j].1 && matches!(out[j], EntityKind::Arc { .. });
        let pi = point(out[i], endpoint(out[i], ri, true));
        let pj = point(out[j], endpoint(out[j], rj, false));
        let at = if pi.dist(pj) <= EPS {
            pi
        } else {
            intersections(out[i], out[j])
                .into_iter()
                .min_by(|a, b| (a.dist(pi) + a.dist(pj)).total_cmp(&(b.dist(pi) + b.dist(pj))))
                .ok_or_else(|| ToolError("offset neighbours do not intersect".into()))?
        };
        set_end(&mut out[i], if ri { End::Start } else { End::End }, at);
        set_end(&mut out[j], if rj { End::End } else { End::Start }, at);
    }
    for ((e, reverse), kind) in source.iter().zip(&out) {
        if let (EntityKind::Line { a, b }, EntityKind::Line { a: c, b: d }) = (e.kind, *kind)
            && (b - a).dot(d - c) * if *reverse { -1.0 } else { 1.0 } <= EPS * EPS
        {
            return refuse("offset consumes a chain edge");
        }
    }
    Ok(out)
}

fn validate_crossings(curves: &[EntityKind]) -> Result<()> {
    if curves.iter().any(|kind| !valid(*kind)) {
        return refuse("offset would create invalid geometry");
    }
    for (i, &a) in curves.iter().enumerate() {
        for &b in &curves[i + 1..] {
            for p in intersections(a, b) {
                let shared_end = ends(a).is_some_and(|ends| ends.iter().any(|q| q.dist(p) <= EPS))
                    && ends(b).is_some_and(|ends| ends.iter().any(|q| q.dist(p) <= EPS));
                if on_curve(a, p) && on_curve(b, p) && !shared_end {
                    return refuse("offset would make the chain intersect itself");
                }
            }
        }
    }
    Ok(())
}

impl Sketch {
    fn offset_plan(&self, source: &[(Entity, bool)], distance: f64, options: OffsetOptions) -> Result<Plan> {
        if source.is_empty() || !distance.is_finite() || distance.abs() <= EPS {
            return refuse("offset distance must be finite and nonzero");
        }
        let circle = matches!(source[0].0.kind, EntityKind::Circle { .. });
        let closed = circle
            || point(source[0].0.kind, endpoint(source[0].0.kind, source[0].1, false)).dist(point(
                source.last().expect("source").0.kind,
                endpoint(source.last().expect("source").0.kind, source.last().expect("source").1, true),
            )) <= EPS;
        let caps = options.cap != OffsetCap::None && !closed;
        if caps && source.iter().any(|(e, _)| e.construction != source[0].0.construction) {
            return refuse("a capped chain must use the same construction state throughout");
        }
        if caps && !options.both_sides && options.make_base_construction && !source[0].0.construction {
            return refuse(
                "one-sided caps need the originals as their other profile boundary; use both sides to make the originals construction",
            );
        }
        let count = source.len();
        let sides = if options.both_sides { 2 } else { 1 };
        let mut geometry = Vec::new();
        for side in 0..sides {
            geometry.extend(
                shifted(source, distance * if side == 0 { 1.0 } else { -1.0 }, closed)?
                    .into_iter()
                    .zip(source)
                    .map(|(kind, (e, _))| (kind, e.construction)),
            );
        }
        let total = geometry.len() + if caps { 2 } else { 0 };
        self.next_entity
            .checked_add(u32::try_from(total).map_err(|_| ToolError("too many offset curves".into()))?)
            .ok_or_else(|| ToolError("no entity ids remain".into()))?;
        let id = |i: usize| self.next_entity + i as u32;
        let group = self.next_constraint;
        let mut relations = Vec::new();
        let mut joins = Vec::new();
        for side in 0..sides {
            for (i, (e, reverse)) in source.iter().enumerate() {
                let target = side * count + i;
                let kind = geometry[target].0;
                let target_reverse = *reverse && matches!(kind, EntityKind::Arc { .. });
                let mut locks = [None; 2];
                if !closed {
                    for (end, active) in [(false, i == 0), (true, i + 1 == count)] {
                        if active {
                            locks[usize::from(end)] = Some((
                                PointRef::new(e.id, endpoint(e.kind, *reverse, end)),
                                PointRef::new(id(target), endpoint(kind, target_reverse, end)),
                            ));
                        }
                    }
                }
                relations.push(ConstraintKind::Offset {
                    a: e.id,
                    b: id(target),
                    group,
                    distance,
                    reverse: *reverse,
                    opposite: side == 1,
                    ends: locks,
                });
                if !circle && (i + 1 < count || closed) {
                    let next = side * count + (i + 1) % count;
                    let next_reverse = source[(i + 1) % count].1 && matches!(geometry[next].0, EntityKind::Arc { .. });
                    joins.push(ConstraintKind::Coincident {
                        a: PointRef::new(id(target), endpoint(kind, target_reverse, true)),
                        b: PointRef::new(id(next), endpoint(geometry[next].0, next_reverse, false)),
                    });
                }
            }
        }
        if caps {
            for end in [false, true] {
                let i = if end { count - 1 } else { 0 };
                let (e, reverse) = source[i];
                let target_reverse = reverse && matches!(geometry[i].0, EntityKind::Arc { .. });
                let mut a = PointRef::new(id(i), endpoint(geometry[i].0, target_reverse, end));
                let mut pa = point(geometry[i].0, a.index);
                let (mut b, mut pb) = if options.both_sides {
                    let j = count + i;
                    let rev = reverse && matches!(geometry[j].0, EntityKind::Arc { .. });
                    let index = endpoint(geometry[j].0, rev, end);
                    (PointRef::new(id(j), index), point(geometry[j].0, index))
                } else {
                    let index = endpoint(e.kind, reverse, end);
                    (PointRef::new(e.id, index), point(e.kind, index))
                };
                let kind = match options.cap {
                    OffsetCap::Lines => EntityKind::Line { a: pa, b: pb },
                    OffsetCap::Round => {
                        let center = pa.mid(pb);
                        let mut arc = EntityKind::Arc { center, start: pa, end: pb };
                        if (entity(arc).midpoint() - center).dot(outward(e.kind, reverse, end)) < 0.0 {
                            std::mem::swap(&mut a, &mut b);
                            std::mem::swap(&mut pa, &mut pb);
                            arc = EntityKind::Arc { center, start: pa, end: pb };
                        }
                        arc
                    }
                    OffsetCap::None => unreachable!(),
                };
                relations.push(ConstraintKind::OffsetCap { cap: id(geometry.len()), a, b, group });
                geometry.push((kind, e.construction));
            }
        }
        let mut bounds: Vec<_> = geometry.iter().map(|(kind, _)| *kind).collect();
        if caps && !options.both_sides {
            bounds.extend(source.iter().map(|(e, _)| e.kind));
        }
        validate_crossings(&bounds)?;
        relations.extend(joins);
        Ok(Plan { sources: source.iter().map(|(e, _)| e.id).collect(), geometry, relations })
    }

    pub(super) fn offset_chain(
        &mut self,
        ids: &[EntityId],
        distance: f64,
        options: OffsetOptions,
    ) -> Result<Vec<EntityId>> {
        let source = self.offset_sources(ids, options.chain)?;
        let plan = self.offset_plan(&source, distance, options)?;
        let ids = plan.geometry.into_iter().map(|(kind, construction)| self.add(kind, construction)).collect();
        for kind in plan.relations {
            self.constrain(kind);
        }
        if options.make_base_construction {
            for id in plan.sources {
                self.entity_mut(id).expect("validated source").construction = true;
            }
        }
        Ok(ids)
    }

    /// Current signed distance for an existing offset group.
    pub fn offset_group_distance(&self, group: ConstraintId) -> Option<f64> {
        self.constraints.iter().find_map(|c| match c.kind {
            ConstraintKind::Offset { group: id, distance, .. } if id == group => Some(distance),
            _ => None,
        })
    }

    /// The copied curves and caps in a group, in creation order.
    pub fn offset_group_targets(&self, group: ConstraintId) -> Vec<EntityId> {
        self.constraints
            .iter()
            .filter_map(|c| match c.kind {
                ConstraintKind::Offset { group: id, b, .. } if id == group => Some(b),
                ConstraintKind::OffsetCap { group: id, cap, .. } if id == group => Some(cap),
                _ => None,
            })
            .collect()
    }

    pub(super) fn edit_offset(&mut self, group: ConstraintId, distance: f64) -> Result<Vec<EntityId>> {
        let rows: Vec<_> = self
            .constraints
            .iter()
            .filter_map(|c| match c.kind {
                ConstraintKind::Offset { group: id, a, reverse, opposite: false, .. } if id == group => {
                    Some((a, reverse))
                }
                _ => None,
            })
            .collect();
        if rows.is_empty() {
            return refuse("that offset group no longer exists");
        }
        let source =
            rows.into_iter().map(|(id, reverse)| Ok((self.tool_entity(id)?, reverse))).collect::<Result<Vec<_>>>()?;
        let targets = self.offset_group_targets(group);
        let both_sides = self
            .constraints
            .iter()
            .any(|c| matches!(c.kind, ConstraintKind::Offset { group: id, opposite: true, .. } if id == group));
        let cap = self
            .constraints
            .iter()
            .find_map(|c| match c.kind {
                ConstraintKind::OffsetCap { group: id, cap, .. } if id == group => {
                    self.entity(cap).map(|e| if e.is_line() { OffsetCap::Lines } else { OffsetCap::Round })
                }
                _ => None,
            })
            .unwrap_or_default();
        let plan = self.offset_plan(&source, distance, OffsetOptions { both_sides, cap, ..Default::default() })?;
        if plan.geometry.len() != targets.len() {
            return refuse("this offset group was changed; recreate the offset to change its distance");
        }
        for (&id, (kind, _)) in targets.iter().zip(&plan.geometry) {
            let e = self.tool_entity(id)?;
            if std::mem::discriminant(&e.kind) != std::mem::discriminant(kind) {
                return refuse("an offset member changed type; recreate the offset");
            }
        }
        let map = |id: EntityId| {
            id.checked_sub(self.next_entity).and_then(|index| targets.get(index as usize)).copied().unwrap_or(id)
        };
        let reference = |p: PointRef| PointRef::new(map(p.entity), p.index);
        let mut trial = self.clone();
        for (&id, (kind, _)) in targets.iter().zip(plan.geometry) {
            trial.entity_mut(id).expect("validated target").kind = kind;
        }
        for kind in plan.relations {
            let (kind, owner) = match kind {
                ConstraintKind::Offset { a, b, distance, reverse, opposite, ends, .. } => (
                    ConstraintKind::Offset {
                        a,
                        b: map(b),
                        group,
                        distance,
                        reverse,
                        opposite,
                        ends: ends.map(|pair| pair.map(|(a, b)| (reference(a), reference(b)))),
                    },
                    Some(map(b)),
                ),
                ConstraintKind::OffsetCap { cap, a, b, .. } => (
                    ConstraintKind::OffsetCap { cap: map(cap), a: reference(a), b: reference(b), group },
                    Some(map(cap)),
                ),
                ConstraintKind::Coincident { a, b } => {
                    (ConstraintKind::Coincident { a: reference(a), b: reference(b) }, None)
                }
                _ => unreachable!(),
            };
            if let Some(owner) = owner {
                let existing = trial
                    .constraints
                    .iter_mut()
                    .find(|c| match c.kind {
                        ConstraintKind::Offset { group: id, b, .. } => id == group && b == owner,
                        ConstraintKind::OffsetCap { group: id, cap, .. } => id == group && cap == owner,
                        _ => false,
                    })
                    .expect("group member validated");
                existing.kind = kind;
            } else if !trial.constraints.iter().any(|c| c.kind == kind) {
                trial.constrain(kind);
            }
        }
        crate::solve::solve(&mut trial, None, false);
        if matches!(trial.status, crate::SolveStatus::OverDefined { .. } | crate::SolveStatus::Failed(_))
            || trial.drawn().any(|e| !valid(e.kind))
        {
            return refuse("that offset distance conflicts with existing relations");
        }
        *self = trial;
        Ok(targets)
    }
}
