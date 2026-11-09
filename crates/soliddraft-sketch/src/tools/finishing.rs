//! Analytic corner treatments and splits, shared with the viewport preview.

use super::*;
use crate::Align;

/// How a line chamfer is sized from the selected corner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChamferSize {
    /// Distances along the first and second lines.
    Distances {
        /// First setback, mm.
        first: f64,
        /// Second setback, mm.
        second: f64,
    },
    /// Setback on the first line and the acute angle to that line.
    DistanceAngle {
        /// First setback, mm.
        distance: f64,
        /// Angle to the first retained line, degrees.
        angle_degrees: f64,
    },
}

/// Two adjusted curves and the tangent arc or chamfer that joins them.
#[derive(Clone, Debug, PartialEq)]
pub struct JunctionPreview {
    /// Retained first and second curves.
    pub kept: [EntityKind; 2],
    /// Joining arc or line.
    pub bridge: EntityKind,
    /// Portions removed from the original curves.
    pub removed: Vec<EntityKind>,
    /// Original point references whose positions change.
    pub joint: [PointRef; 2],
    /// Bridge point indices corresponding to the first and second curves.
    pub bridge_points: [u8; 2],
}

/// Two pieces made by a split, with their shared endpoints.
#[derive(Clone, Debug, PartialEq)]
pub struct SplitPreview {
    /// Pieces in the original curve's direction.
    pub pieces: [EntityKind; 2],
    /// Shared endpoint indices on the first and second pieces.
    pub joints: Vec<(u8, u8)>,
}

fn positive(value: f64) -> bool {
    value.is_finite() && value > EPS
}

fn loci(kind: EntityKind, radius: f64) -> Vec<EntityKind> {
    match kind {
        EntityKind::Line { a, b } => {
            let d = (b - a).normalized();
            let n = P2::new(-d.v, d.u).scale(radius);
            vec![EntityKind::Line { a: a + n, b: b + n }, EntityKind::Line { a: a - n, b: b - n }]
        }
        EntityKind::Arc { center, start, .. } => {
            let r = center.dist(start);
            [r + radius, (r - radius).abs()]
                .into_iter()
                .filter(|r| positive(*r))
                .map(|radius| EntityKind::Circle { center, radius })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn contacts(kind: EntityKind, center: P2, radius: f64) -> Vec<P2> {
    match kind {
        EntityKind::Line { a, b } => {
            let d = (b - a).normalized();
            vec![a + d.scale((center - a).dot(d))]
        }
        EntityKind::Arc { center: c, start, .. } if c.dist(center) > EPS => {
            let d = (center - c).normalized().scale(c.dist(start));
            [c + d, c - d].into_iter().filter(|p| (p.dist(center) - radius).abs() <= EPS).collect()
        }
        _ => Vec::new(),
    }
}

fn away(kind: EntityKind, index: u8) -> P2 {
    match kind {
        EntityKind::Line { a, b } => if index == 0 { b - a } else { a - b }.normalized(),
        EntityKind::Arc { center, start, end } => {
            let r = if index == 1 { start - center } else { end - center };
            P2::new(-r.v, r.u).normalized().scale(if index == 1 { 1.0 } else { -1.0 })
        }
        _ => P2::ZERO,
    }
}

impl Sketch {
    fn junction_preview(&self, a: EntityId, b: EntityId, keep_a: P2, keep_b: P2) -> Result<CornerPreview> {
        let first = self.tool_entity(a)?;
        let second = self.tool_entity(b)?;
        if first.construction != second.construction {
            return refuse("use two profile curves or two construction curves for a corner treatment");
        }
        // Finishing replaces an existing corner coincidence rather than refusing it.
        let mut geometry = self.clone();
        geometry.constraints.clear();
        geometry.corner_preview(a, b, keep_a, keep_b, TrimOptions::default())
    }

    /// Insert a minor tangent arc at the picked corner between lines or circular arcs.
    pub fn fillet_preview(
        &self,
        a: EntityId,
        b: EntityId,
        keep_a: P2,
        keep_b: P2,
        radius: f64,
    ) -> Result<JunctionPreview> {
        if !positive(radius) {
            return refuse("fillet radius must be finite and positive");
        }
        let corner = self.junction_preview(a, b, keep_a, keep_b)?;
        let first = self.tool_entity(a)?;
        let second = self.tool_entity(b)?;
        let at = Entity { id: a, kind: corner.kept[0], construction: false }
            .point(corner.joint[0].index)
            .expect("corner endpoint");
        let mut choices = Vec::new();
        for left in loci(first.kind, radius) {
            for right in loci(second.kind, radius) {
                for center in intersections(left, right) {
                    for pa in contacts(first.kind, center, radius) {
                        for pb in contacts(second.kind, center, radius) {
                            if pa.dist(pb) <= EPS {
                                continue;
                            }
                            let (Ok((ka, ea)), Ok((kb, eb))) =
                                (corner_piece(first.kind, keep_a, pa), corner_piece(second.kind, keep_b, pb))
                            else {
                                continue;
                            };
                            let indices = [
                                u8::from(first.is_curve()) + u8::from(ea == End::End),
                                u8::from(second.is_curve()) + u8::from(eb == End::End),
                            ];
                            if indices != [corner.joint[0].index, corner.joint[1].index]
                                || !on_curve(ka, first.nearest(keep_a).0)
                                || !on_curve(kb, second.nearest(keep_b).0)
                            {
                                continue;
                            }
                            let forward = (pa - center).cross(pb - center) >= 0.0;
                            let bridge = if forward {
                                EntityKind::Arc { center, start: pa, end: pb }
                            } else {
                                EntityKind::Arc { center, start: pb, end: pa }
                            };
                            if !valid(bridge) {
                                continue;
                            }
                            let bridge_points = if forward { [1, 2] } else { [2, 1] };
                            if away(ka, indices[0]).dot(away(bridge, bridge_points[0])) > -1.0 + 1e-6
                                || away(kb, indices[1]).dot(away(bridge, bridge_points[1])) > -1.0 + 1e-6
                            {
                                continue;
                            }
                            let preview = JunctionPreview {
                                kept: [ka, kb],
                                bridge,
                                removed: [corner_removed(first.kind, ka, ea), corner_removed(second.kind, kb, eb)]
                                    .into_iter()
                                    .flatten()
                                    .collect(),
                                joint: corner.joint,
                                bridge_points,
                            };
                            if !choices.iter().any(|(_, p): &(f64, JunctionPreview)| match (p.bridge, preview.bridge) {
                                (
                                    EntityKind::Arc { center: c, start: s, end: e },
                                    EntityKind::Arc { center: d, start: t, end: f },
                                ) => c.dist(d) <= EPS && s.dist(t) <= EPS && e.dist(f) <= EPS,
                                _ => false,
                            }) {
                                choices.push((center.dist(at), preview));
                            }
                        }
                    }
                }
            }
        }
        choices.sort_by(|a, b| a.0.total_cmp(&b.0));
        let Some((score, best)) = choices.first() else {
            return refuse(
                "that radius does not fit the picked portions; use a smaller radius or pick farther from the corner",
            );
        };
        if choices.get(1).is_some_and(|(next, _)| (next - score).abs() <= EPS) {
            return refuse("two fillets match these picks; pick closer to the intended corner");
        }
        Ok(best.clone())
    }

    /// Replace a corner between two lines by a segment sized with setbacks or an angle.
    pub fn chamfer_preview(
        &self,
        a: EntityId,
        b: EntityId,
        keep_a: P2,
        keep_b: P2,
        size: ChamferSize,
    ) -> Result<JunctionPreview> {
        let first = self.tool_entity(a)?;
        let second = self.tool_entity(b)?;
        if !first.is_line() || !second.is_line() {
            return refuse("chamfer needs two nonparallel lines");
        }
        let corner = self.junction_preview(a, b, keep_a, keep_b)?;
        let at = Entity { id: a, kind: corner.kept[0], construction: false }
            .point(corner.joint[0].index)
            .expect("corner endpoint");
        let far = |i: usize| {
            Entity { id: 0, kind: corner.kept[i], construction: false }
                .point(1 - corner.joint[i].index)
                .expect("other line endpoint")
        };
        let da = (far(0) - at).normalized();
        let db = (far(1) - at).normalized();
        let (d1, d2) = match size {
            ChamferSize::Distances { first, second } => (first, second),
            ChamferSize::DistanceAngle { distance, angle_degrees } => {
                if !angle_degrees.is_finite() || !(0.0..90.0).contains(&angle_degrees) || angle_degrees <= 0.0 {
                    return refuse("chamfer angle must be between 0 and 90 degrees");
                }
                let theta = da.dot(db).clamp(-1.0, 1.0).acos();
                let alpha = angle_degrees.to_radians();
                if theta + alpha >= std::f64::consts::PI - 1e-10 {
                    return refuse("that angle does not meet the second retained line");
                }
                (distance, distance * alpha.sin() / (theta + alpha).sin())
            }
        };
        if !positive(d1) || !positive(d2) || d1 >= at.dist(far(0)) - EPS || d2 >= at.dist(far(1)) - EPS {
            return refuse("chamfer setbacks must be positive and fit inside both retained lines");
        }
        let pa = at + da.scale(d1);
        let pb = at + db.scale(d2);
        let mut kept = corner.kept;
        for (i, p) in [pa, pb].into_iter().enumerate() {
            set_end(&mut kept[i], if corner.joint[i].index == 0 { End::Start } else { End::End }, p);
        }
        let ea = if corner.joint[0].index == 0 { End::Start } else { End::End };
        let eb = if corner.joint[1].index == 0 { End::Start } else { End::End };
        Ok(JunctionPreview {
            kept,
            bridge: EntityKind::Line { a: pa, b: pb },
            removed: [corner_removed(first.kind, kept[0], ea), corner_removed(second.kind, kept[1], eb)]
                .into_iter()
                .flatten()
                .collect(),
            joint: corner.joint,
            bridge_points: [0, 1],
        })
    }

    fn apply_junction(
        &mut self,
        a: EntityId,
        b: EntityId,
        preview: JunctionPreview,
        radius: Option<f64>,
    ) -> Result<Vec<EntityId>> {
        let construction = self.tool_entity(a)?.construction;
        let line_changes =
            [a, b].into_iter().filter(|id| self.entity(*id).is_some_and(|e| e.is_line())).collect::<Vec<_>>();
        // A removed corner point cannot keep its old anchor or coincidence. Far endpoints retain theirs.
        self.constraints.retain(|c| c.is_reference() || (!c.points().iter().any(|p| preview.joint.contains(p))
            && !matches!(c.kind, ConstraintKind::Equal { a, b } if line_changes.contains(&a) || line_changes.contains(&b))
            && !matches!(c.kind, ConstraintKind::Midpoint { line, .. } if line_changes.contains(&line))));
        self.entity_mut(a).expect("validated target").kind = preview.kept[0];
        self.entity_mut(b).expect("validated target").kind = preview.kept[1];
        let bridge = self.add(preview.bridge, construction);
        for i in 0..2 {
            self.constrain(ConstraintKind::Coincident {
                a: preview.joint[i],
                b: PointRef::new(bridge, preview.bridge_points[i]),
            });
        }
        if let Some(value) = radius {
            for id in [a, b] {
                self.constrain(ConstraintKind::Tangent { a: id, b: bridge });
            }
            self.constrain(ConstraintKind::Radius {
                entity: bridge,
                value,
                diameter: false,
                label: entity(preview.bridge).midpoint() + P2::new(value, value),
            });
        }
        Ok(vec![a, b, bridge])
    }

    pub(super) fn fillet(
        &mut self,
        a: EntityId,
        b: EntityId,
        keep_a: P2,
        keep_b: P2,
        radius: f64,
    ) -> Result<Vec<EntityId>> {
        let preview = self.fillet_preview(a, b, keep_a, keep_b, radius)?;
        self.apply_junction(a, b, preview, Some(radius))
    }

    pub(super) fn chamfer(
        &mut self,
        a: EntityId,
        b: EntityId,
        keep_a: P2,
        keep_b: P2,
        size: ChamferSize,
    ) -> Result<Vec<EntityId>> {
        let preview = self.chamfer_preview(a, b, keep_a, keep_b, size)?;
        self.apply_junction(a, b, preview, None)
    }

    /// Split a line or arc at one point, or a circle at two distinct points.
    pub fn split_preview(&self, id: EntityId, at: P2, second: Option<P2>) -> Result<SplitPreview> {
        let target = self.tool_entity(id)?;
        if !finite(at) || second.is_some_and(|p| !finite(p)) || target.is_point() {
            return refuse("split needs a curve and finite points");
        }
        let at = target.nearest(at).0;
        let pieces = match target.kind {
            EntityKind::Circle { center, .. } => {
                let Some(other) = second else {
                    return refuse("pick two distinct points to split a circle");
                };
                let other = target.nearest(other).0;
                if at.dist(other) <= EPS {
                    return refuse("pick two distinct points to split a circle");
                }
                [EntityKind::Arc { center, start: at, end: other }, EntityKind::Arc { center, start: other, end: at }]
            }
            kind => {
                if second.is_some() {
                    return refuse("a line or arc needs only one split point");
                }
                let [start, end] = ends(kind).ok_or_else(|| ToolError("split needs a line, arc or circle".into()))?;
                if at.dist(start) <= EPS || at.dist(end) <= EPS {
                    return refuse("pick inside the curve, away from its endpoints");
                }
                let mut pieces = [kind, kind];
                set_end(&mut pieces[0], End::End, at);
                set_end(&mut pieces[1], End::Start, at);
                pieces
            }
        };
        if pieces.iter().any(|k| !valid(*k)) {
            return refuse("the split would create a degenerate curve");
        }
        let joints = match target.kind {
            EntityKind::Line { .. } => vec![(1, 0)],
            EntityKind::Arc { .. } => vec![(2, 1)],
            EntityKind::Circle { .. } => vec![(2, 1), (1, 2)],
            _ => unreachable!(),
        };
        Ok(SplitPreview { pieces, joints })
    }

    pub(super) fn split(&mut self, id: EntityId, at: P2, second: Option<P2>) -> Result<Vec<EntityId>> {
        let preview = self.split_preview(id, at, second)?;
        let old = self.tool_entity(id)?;
        self.entity_mut(id).expect("validated target").kind = preview.pieces[0];
        let new = self.add(preview.pieces[1], old.construction);
        if !matches!(old.kind, EntityKind::Circle { .. }) {
            let endpoint = PointRef::new(id, old.point_count() - 1);
            let replacement = PointRef::new(new, endpoint.index);
            self.constraints = self
                .constraints
                .iter()
                .filter_map(|c| {
                    if old.is_line()
                        && matches!(c.geometry_kind(), ConstraintKind::Equal { a, b } if a == id || b == id)
                        || matches!(c.kind, ConstraintKind::Midpoint { line, .. } if line == id)
                    {
                        return None;
                    }
                    remap_split(*c, endpoint, replacement)
                })
                .collect();
        }
        for (a, b) in preview.joints {
            self.constrain(ConstraintKind::Coincident { a: PointRef::new(id, a), b: PointRef::new(new, b) });
        }
        if old.is_line() {
            self.constrain(ConstraintKind::Parallel { a: id, b: new });
        } else {
            self.constrain(ConstraintKind::Coincident { a: PointRef::new(id, 0), b: PointRef::new(new, 0) });
        }
        Ok(vec![id, new])
    }
}

fn remap_split(mut c: Constraint, endpoint: PointRef, replacement: PointRef) -> Option<Constraint> {
    let map = |p| if p == endpoint { replacement } else { p };
    let align = |target| match target {
        Align::Points(a, b) => Align::Points(map(a), map(b)),
        target => target,
    };
    let reference = c.is_reference();
    let kind = match c.geometry_kind() {
        ConstraintKind::Coincident { a, b } => ConstraintKind::Coincident { a: map(a), b: map(b) },
        ConstraintKind::PointOn { point, entity } => ConstraintKind::PointOn { point: map(point), entity },
        ConstraintKind::Horizontal { target } => ConstraintKind::Horizontal { target: align(target) },
        ConstraintKind::Vertical { target } => ConstraintKind::Vertical { target: align(target) },
        ConstraintKind::Midpoint { point, line } => ConstraintKind::Midpoint { point: map(point), line },
        ConstraintKind::Fix { point, at } => ConstraintKind::Fix { point: map(point), at },
        ConstraintKind::Distance { a, b, kind, value, label } => {
            ConstraintKind::Distance { a: map(a), b: map(b), kind, value, label }
        }
        ConstraintKind::PointLineDistance { point, line, value, label } => {
            ConstraintKind::PointLineDistance { point: map(point), line, value, label }
        }
        kind if !c.points().contains(&endpoint) => kind,
        _ => return None,
    };
    c.kind = if reference { kind.as_reference()? } else { kind };
    Some(c)
}
