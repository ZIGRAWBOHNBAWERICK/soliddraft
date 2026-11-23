//! Thin extrusions: the wall regions along a sketch's curves.
//!
//! A thin extrusion sweeps a wall of constant thickness along each chain of the sketch's
//! curves instead of the closed regions they bound. Each chain is offset with the
//! sketch's own analytic offset, on one side, centred on the curve or on both sides
//! with two thicknesses. An open chain's wall is closed by a straight end at each end of
//! the chain. The wall regions are then ordinary closed regions, so the kernel sweeps
//! them as it sweeps any profile and the faces keep names that follow the sketch:
//! the curve's own side of a one-sided wall is the curve's side, the offset sides are
//! thin walls of the curve and the ends are thin ends of the chain.

use std::collections::{HashMap, HashSet};

use soliddraft_sketch::tools::{OffsetCap, OffsetOptions, Operation};
use soliddraft_sketch::{EntityId, EntityKind, P2, Region, Sketch};

use crate::feature::{ThinFeature, ThinSide};
use crate::reference::Generator;

/// A wall region and the names of the faces its curves sweep, by the region's entity ids.
pub(crate) type Wall = (Region, HashMap<EntityId, Generator>);

/// The largest wall thickness, as for every other length a feature takes.
const MAX_THICKNESS: f64 = 100_000.0;

/// The wall regions of `sketch` for `thin`, one or more per chain of curves.
pub(crate) fn walls(sketch: &Sketch, thin: &ThinFeature) -> Result<Vec<Wall>, String> {
    let (first, second) = thin.side.split(thin.thickness);
    for t in [first, second] {
        if !t.is_finite() || t < 0.0 || t > MAX_THICKNESS {
            return Err("the thin wall's thickness must be a positive length".into());
        }
    }
    if first <= 0.0 || (matches!(thin.side, ThinSide::Two { .. }) && second <= 0.0) {
        return Err("the thin wall's thickness must be a positive length".into());
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for entity in sketch.drawn().filter(|e| !e.construction) {
        match entity.kind {
            EntityKind::Point { .. } => continue,
            EntityKind::Text { .. } => return Err("a thin wall cannot follow sketch text".into()),
            EntityKind::Spline { .. } | EntityKind::Ellipse { .. } | EntityKind::EllipticArc { .. } => {
                return Err(
                    "a thin wall along a spline or an ellipse is coming soon; it follows lines, arcs and circles"
                        .into(),
                );
            }
            EntityKind::Line { .. } | EntityKind::Arc { .. } | EntityKind::Circle { .. } => {}
        }
        if seen.contains(&entity.id) {
            continue;
        }
        let chain =
            sketch.offset_chain_from(entity.id).map_err(|e| format!("the thin wall cannot follow the sketch: {e}"))?;
        seen.extend(chain.iter().map(|(id, _)| *id));
        out.extend(chain_walls(sketch, &chain, thin)?);
    }
    if out.is_empty() {
        return Err("the sketch has no curves for a thin wall".into());
    }
    Ok(out)
}

/// The start and end of a curve walked forward, or backward when `reverse`.
fn ends(kind: &EntityKind, reverse: bool) -> Option<(P2, P2)> {
    let (a, b) = match *kind {
        EntityKind::Line { a, b } => (a, b),
        EntityKind::Arc { start, end, .. } => (start, end),
        _ => return None,
    };
    Some(if reverse { (b, a) } else { (a, b) })
}

/// The end of curve `id` of `sketch` nearest `to`.
fn nearest_end(sketch: &Sketch, id: EntityId, to: P2) -> Option<P2> {
    let (a, b) = ends(&sketch.entity(id)?.kind, false)?;
    Some(if a.dist(to) <= b.dist(to) { a } else { b })
}

/// The walls of one chain: `chain` in walking order, with each curve's direction.
fn chain_walls(sketch: &Sketch, chain: &[(EntityId, bool)], thin: &ThinFeature) -> Result<Vec<Wall>, String> {
    let first = sketch.entity(chain[0].0).ok_or("a chain curve is missing")?;
    let last = sketch.entity(chain[chain.len() - 1].0).ok_or("a chain curve is missing")?;
    let open = match (ends(&first.kind, chain[0].1), ends(&last.kind, chain[chain.len() - 1].1)) {
        (Some((start, _)), Some((_, end))) => {
            Some((start, end)).filter(|(s, e)| s.dist(*e) > soliddraft_kernel_api::PROFILE_JOIN_TOLERANCE)
        }
        _ => None,
    };
    let (a, b) = thin.side.split(thin.thickness);
    let build = |sign: f64| wall(sketch, chain, open, sign * a, -sign * b);
    match (thin.side, open) {
        // A closed chain's one-sided wall goes outside unless reversed: try one sign and
        // keep it when its wall lies on the wanted side.
        (ThinSide::One { reverse }, None) => {
            let walls = build(1.0)?;
            if outside(&walls) != reverse { Ok(walls) } else { build(-1.0) }
        }
        (ThinSide::One { reverse }, Some(_)) => build(if reverse { -1.0 } else { 1.0 }),
        // A closed chain's second wall goes inside.
        (ThinSide::Two { .. }, None) => {
            let walls = build(1.0)?;
            if outside(&walls) { Ok(walls) } else { build(-1.0) }
        }
        (ThinSide::Two { .. } | ThinSide::MidPlane, _) => build(1.0),
    }
}

/// True when the first wall's outer boundary is the offset one, which then lies outside
/// the curves.
fn outside(walls: &[Wall]) -> bool {
    walls.first().is_some_and(|(region, names)| {
        region.outer.entities.iter().any(|id| matches!(names.get(id), Some(Generator::ThinWall { second: false, .. })))
    })
}

/// The wall of `chain` offset by `first` one way and `second` (of opposite sign, or 0 for
/// none) the other way, with straight ends when `open` gives the chain's two ends.
fn wall(
    sketch: &Sketch,
    chain: &[(EntityId, bool)],
    open: Option<(P2, P2)>,
    first: f64,
    second: f64,
) -> Result<Vec<Wall>, String> {
    let mut s = sketch.clone();
    let members: Vec<EntityId> = chain.iter().map(|(id, _)| *id).collect();
    // Only this chain bounds the wall: every other curve becomes a guide.
    let others: Vec<EntityId> =
        s.drawn().filter(|e| !e.construction && !members.contains(&e.id)).map(|e| e.id).collect();
    for id in others {
        if let Some(e) = s.entity_mut(id) {
            e.construction = true;
        }
    }
    let too_thick = |e: soliddraft_sketch::tools::ToolError| format!("the thin wall does not fit the sketch: {e}");
    // The members are already in walking order, so the offsets come back in that order.
    let options =
        OffsetOptions { chain: false, both_sides: false, cap: OffsetCap::None, make_base_construction: false };
    let mut names = HashMap::new();
    let mut sides = Vec::new();
    for (distance, second_side) in [(first, false), (second, true)] {
        if distance == 0.0 {
            continue;
        }
        let made = s
            .apply_operation(&Operation::Offset { entities: members.clone(), distance, options })
            .map_err(too_thick)?;
        if made.len() != members.len() {
            return Err("the thin wall does not fit the sketch".into());
        }
        for (target, source) in made.iter().zip(&members) {
            names.insert(*target, Generator::ThinWall { entity: *source, second: second_side });
        }
        sides.push(made);
    }
    let one_sided = sides.len() == 1;
    if one_sided {
        for id in &members {
            names.insert(*id, Generator::Side { entity: *id, path: None });
        }
    } else {
        for id in &members {
            if let Some(e) = s.entity_mut(*id) {
                e.construction = true;
            }
        }
    }
    if let Some((start, end)) = open {
        for (at, index, last) in [(start, 0, false), (end, members.len() - 1, true)] {
            let near = |side: &Vec<EntityId>| nearest_end(&s, side[index], at);
            let a = near(&sides[0]).ok_or("the thin wall's end is missing")?;
            let b = if one_sided { at } else { near(&sides[1]).ok_or("the thin wall's end is missing")? };
            let cap = s
                .apply_operation(&Operation::Add { kind: EntityKind::Line { a, b }, construction: false })
                .map_err(too_thick)?;
            for id in cap {
                names.insert(id, Generator::ThinEnd { entity: members[index], last });
            }
        }
    }
    let regions = s.regions().map_err(|e| format!("the thin wall does not close: {e}"))?;
    if regions.is_empty() {
        return Err("the thin wall does not close".into());
    }
    Ok(regions.into_iter().map(|region| (region, names.clone())).collect())
}
