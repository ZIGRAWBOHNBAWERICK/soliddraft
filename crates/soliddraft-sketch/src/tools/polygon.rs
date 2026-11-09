//! Retained regular polygons expressed with ordinary solver relations.

use super::*;
use crate::ConstraintId;

/// Relationship between a polygon and its construction circle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolygonMode {
    /// Vertices lie on the circle. The placement point is the first vertex.
    #[default]
    Inscribed,
    /// Sides touch the circle. The placement point is the first side's midpoint.
    Circumscribed,
}

/// Membership and defining relations of a retained polygon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Polygon {
    /// Construction circle; also the group's stable identity.
    pub circle: EntityId,
    /// Counterclockwise sides, beginning at the placement direction.
    pub edges: Vec<EntityId>,
    /// Defining joins, equal lengths and circle relations.
    pub relations: Vec<ConstraintId>,
    /// Meaning of the construction circle radius.
    pub mode: PolygonMode,
}

/// Shared placement geometry. Circumscribed placement names a side midpoint.
pub fn regular_polygon_corners(center: P2, point: P2, sides: u32, mode: PolygonMode) -> Result<Vec<P2>> {
    let mut corners = polygon_corners(center, point, sides)?;
    if mode == PolygonMode::Circumscribed {
        let d = point - center;
        let tangent = (std::f64::consts::PI / f64::from(sides)).tan();
        // Start clockwise of the midpoint so the first side passes through it.
        corners = polygon_corners(center, point - d.perp().scale(tangent), sides)?;
    }
    if !center.dist(point).is_finite()
        || corners.iter().enumerate().any(|(i, &a)| !valid(EntityKind::Line { a, b: corners[(i + 1) % corners.len()] }))
    {
        return refuse("the polygon would produce non-finite or degenerate sides");
    }
    Ok(corners)
}

impl Sketch {
    /// Find a retained polygon from any side or its circle. Edited/deleted defining
    /// relations release the group; surviving geometry remains ordinary sketch geometry.
    pub fn polygon_of(&self, member: EntityId) -> Option<&Polygon> {
        self.polygons.iter().find(|p| {
            (p.circle == member || p.edges.contains(&member))
                && self.entity(p.circle).is_some_and(|e| e.construction && matches!(e.kind, EntityKind::Circle { .. }))
                && p.edges.len() >= 3
                && p.edges.iter().all(|id| self.entity(*id).is_some_and(|e| matches!(e.kind, EntityKind::Line { .. })))
                && p.relations.iter().all(|id| self.constraint(*id).is_some())
        })
    }

    fn polygon_relations(&mut self, circle: EntityId, edges: &[EntityId], mode: PolygonMode) -> Vec<ConstraintId> {
        let mut relations = Vec::with_capacity(edges.len() * 3);
        for (i, &edge) in edges.iter().enumerate() {
            relations.push(self.constrain(ConstraintKind::Coincident {
                a: PointRef::new(edge, 1),
                b: PointRef::new(edges[(i + 1) % edges.len()], 0),
            }));
            if i > 0 {
                relations.push(self.constrain(ConstraintKind::Equal { a: edges[0], b: edge }));
            }
            relations.push(self.constrain(match mode {
                PolygonMode::Inscribed => ConstraintKind::PointOn { point: PointRef::new(edge, 0), entity: circle },
                PolygonMode::Circumscribed => ConstraintKind::Tangent { a: edge, b: circle },
            }));
        }
        relations
    }

    pub(super) fn regular_polygon(
        &mut self,
        center: P2,
        point: P2,
        sides: u32,
        mode: PolygonMode,
    ) -> Result<Vec<EntityId>> {
        let corners = regular_polygon_corners(center, point, sides, mode)?;
        let edges: Vec<_> = (0..corners.len())
            .map(|i| self.add(EntityKind::Line { a: corners[i], b: corners[(i + 1) % corners.len()] }, false))
            .collect();
        let circle = self.add(EntityKind::Circle { center, radius: center.dist(point) }, true);
        let relations = self.polygon_relations(circle, &edges, mode);
        self.polygons.push(Polygon { circle, edges: edges.clone(), relations, mode });
        Ok(edges.into_iter().chain([circle]).collect())
    }

    pub(super) fn edit_polygon(
        &mut self,
        circle: EntityId,
        sides: u32,
        radius: f64,
        mode: PolygonMode,
    ) -> Result<Vec<EntityId>> {
        let Some(group) = self.polygon_of(circle).filter(|p| p.circle == circle).cloned() else {
            return refuse("select an intact retained polygon");
        };
        if !radius.is_finite() || radius <= EPS {
            return refuse("polygon radius must be finite and greater than 0.000001 mm");
        }
        // External dimensions/snaps must never be silently discarded or overridden.
        if self.constraints.iter().any(|c| {
            if group.relations.contains(&c.id) {
                return false;
            }
            let centre_only = match c.kind {
                ConstraintKind::Coincident { a, b } => a.entity == circle || b.entity == circle,
                ConstraintKind::Fix { point, .. } => point.entity == circle,
                ConstraintKind::Radius { entity, value, diameter, .. } => {
                    entity == circle && (value / if diameter { 2.0 } else { 1.0 } - radius).abs() <= EPS
                }
                _ => false,
            };
            c.entities().iter().any(|id| group.edges.contains(id)) || (!centre_only && c.entities().contains(&circle))
        }) {
            return refuse("release external polygon relations or dimensions before editing its parameters");
        }
        let EntityKind::Circle { center, .. } = self.entity(circle).expect("validated circle").kind else {
            unreachable!()
        };
        let first = self.entity(group.edges[0]).expect("validated edge");
        let direction = match (group.mode, first.kind) {
            (PolygonMode::Circumscribed, EntityKind::Line { a, b }) => (a + b).scale(0.5) - center,
            (_, EntityKind::Line { a, .. }) => a - center,
            _ => unreachable!(),
        };
        let corners = regular_polygon_corners(center, center + direction.normalized().scale(radius), sides, mode)?;
        let mut trial = self.clone();
        trial.constraints.retain(|c| !group.relations.contains(&c.id));
        let mut edges = group.edges.clone();
        for id in edges.drain(corners.len().min(edges.len())..) {
            trial.remove_entity(id);
        }
        for i in 0..corners.len() {
            let kind = EntityKind::Line { a: corners[i], b: corners[(i + 1) % corners.len()] };
            if let Some(&id) = edges.get(i) {
                trial.entity_mut(id).expect("validated side").kind = kind;
            } else {
                edges.push(trial.add(kind, first.construction));
            }
        }
        trial.entity_mut(circle).expect("validated circle").kind = EntityKind::Circle { center, radius };
        let relations = trial.polygon_relations(circle, &edges, mode);
        *trial.polygons.iter_mut().find(|p| p.circle == circle).expect("retained group") =
            Polygon { circle, edges: edges.clone(), relations, mode };
        *self = trial;
        Ok(edges.into_iter().chain([circle]).collect())
    }
}
