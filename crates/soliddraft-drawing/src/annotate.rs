//! Annotations made from model points rather than picks: what the command line and the
//! assistant use to place dimension sets and symbols without a window.
//!
//! An anchor goes on the edge nearest the point, at the edge's end, middle or centre
//! when the point is one of those, otherwise at the point itself, so it follows the
//! part through updates like a picked anchor.

use soliddraft_kernel_api::{Point2, Vec3};

use crate::annotation::{
    Anchor, Annotation, AnnotationKind, AnnotationState, Attach, DimType, DimensionSet, LinearOrientation, SetKind,
    StyleOverrides,
};
use crate::ids::ViewId;
use crate::measure::{measure_set, precision_for};
use crate::part::{PartIndex, round};
use crate::project::{ViewContext, ViewTransform};
use crate::{Drawing, DrawingError};

/// The anchor on the edge nearest `at`, with its point in the view of `xf`.
pub fn anchor_near(index: &PartIndex, xf: &ViewTransform, at: Vec3) -> Option<Anchor> {
    let (id, curve) = nearest_edge(index, at)?;
    anchor_on(index, xf, id, curve, 0, at)
}

pub(crate) fn nearest_edge<'a>(
    index: &'a PartIndex<'_>,
    at: Vec3,
) -> Option<(soliddraft_kernel_api::EdgeId, &'a soliddraft_kernel_api::drawing::Curve3)> {
    index.edges().min_by(|(_, a), (_, b)| curve_distance(a, at).total_cmp(&curve_distance(b, at)))
}

fn anchor_on(
    index: &PartIndex,
    xf: &ViewTransform,
    id: soliddraft_kernel_api::EdgeId,
    curve: &soliddraft_kernel_api::drawing::Curve3,
    leg: u8,
    at: Vec3,
) -> Option<Anchor> {
    let mut entity = index.edge_ref(id)?;
    entity.section_leg = leg;
    let mut candidates =
        vec![(Attach::Start, curve.start()), (Attach::End, curve.end()), (Attach::Mid, curve.point_at(0.5))];
    if let soliddraft_kernel_api::drawing::Curve3::Arc { center, .. }
    | soliddraft_kernel_api::drawing::Curve3::Circle { center, .. } = curve
    {
        candidates.push((Attach::Centre, *center));
    }
    let (attach, point) = candidates
        .into_iter()
        .min_by(|a, b| a.1.distance(at).total_cmp(&b.1.distance(at)))
        .filter(|(_, p)| p.distance(at) <= 1e-3)
        .unwrap_or((Attach::Point { at }, at));
    let point = xf.leg_turn(leg).map_or(point, |turn| turn.point(point));
    Some(Anchor { entity, attach, cached: xf.point(point) })
}

/// The nearest model edge occurrence in the given view.
pub fn view_anchor_near(index: &PartIndex, ctx: &ViewContext, at: Vec3) -> Option<Anchor> {
    if ctx.xf.leg_turn(1).is_none() && ctx.xf.breaks.is_empty() {
        return anchor_near(index, &ctx.xf, at);
    }
    let nearest = if ctx.xf.breaks.is_empty() { None } else { Some(curve_distance(nearest_edge(index, at)?.1, at)) };
    use kurbo::{ParamCurveNearest, Point, Shape};
    let (_, _, _, id, curve, leg) = ctx
        .projected
        .geometry
        .curves
        .iter()
        .filter_map(|projected| {
            if projected.kind != crate::CurveKind::Edge {
                return None;
            }
            let bounds = projected.path.bounding_box();
            if bounds.width().max(bounds.height()) <= 1e-6 {
                return None;
            }
            let id = soliddraft_kernel_api::EdgeId::from_raw(projected.source.kernel_hint?);
            let curve = index.edge_curve(id)?;
            let distance = curve_distance(curve, at);
            if nearest.is_some_and(|nearest| distance > nearest + 1e-6) {
                return None;
            }
            let leg = projected.source.section_leg;
            let at_on_leg = ctx.xf.point_on_leg(leg, at);
            if ctx.xf.breaks.hides(at_on_leg.dot(ctx.xf.breaks.axis(&ctx.xf.frame))) {
                return None;
            }
            let q = ctx.xf.precise_point(at_on_leg);
            let nearest = projected
                .path
                .segments()
                .map(|segment| segment.nearest(Point::new(q.u, q.v), 1e-6).distance_sq)
                .fold(f64::INFINITY, f64::min);
            Some((distance, nearest, !projected.visible, id, curve, leg))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)).then(a.5.cmp(&b.5)))?;
    let at = ctx.retained_edge_point(id, leg, curve, at).unwrap_or(at);
    anchor_on(index, &ctx.xf, id, curve, leg, at)
}

fn curve_distance(curve: &soliddraft_kernel_api::drawing::Curve3, at: Vec3) -> f64 {
    curve.sample(0.0).windows(2).map(|w| segment_distance(at, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f64 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

fn dimension_anchor_near(index: &PartIndex, ctx: &ViewContext, at: Vec3) -> Option<Anchor> {
    if ctx.xf.breaks.is_empty() {
        return view_anchor_near(index, ctx, at);
    }
    if let Some(circle) = ctx
        .projected
        .circles
        .iter()
        .find(|circle| ctx.xf.point_on_leg(circle.entity.section_leg, at).distance(circle.model_centre) <= 1e-3)
    {
        return Some(Anchor { entity: circle.entity.clone(), attach: Attach::Centre, cached: circle.centre });
    }
    let mut anchor = view_anchor_near(index, ctx, at)?;
    if let Attach::Point { at } = anchor.attach {
        let crate::Resolution::Found(crate::Entity::Edge(id)) = index.resolve(&anchor.entity) else { return None };
        let curve = index.edge_curve(id)?;
        let t = crate::project::parameter_near(curve, at, None)?;
        anchor.attach = Attach::EdgeParameter { t };
        anchor.cached = ctx.xf.point(ctx.xf.point_on_leg(anchor.entity.section_leg, curve.point_at(t)));
    }
    Some(anchor)
}

fn new_annotation(
    drawing: &Drawing,
    view: ViewId,
    body: AnnotationKind,
    anchors: Vec<Anchor>,
    placement: Point2,
) -> Annotation {
    Annotation {
        id: drawing.next_annotation_id(),
        view,
        body,
        anchors,
        placement,
        style: StyleOverrides::default(),
        state: AnnotationState::Ok,
        source: None,
    }
}

/// Ordinate, baseline or chain dimensions of view `ctx` through the model points
/// `points`, the first being the origin, measured along `orientation` (horizontal or
/// vertical). The values or their dimension lines sit `offset` sheet millimetres beyond
/// the outermost point across the measuring direction: above or right for a positive
/// offset, below or left for a negative one.
pub fn dimension_set(
    drawing: &Drawing,
    index: &PartIndex,
    ctx: &ViewContext,
    kind: SetKind,
    orientation: LinearOrientation,
    points: &[Vec3],
    offset: f64,
) -> Result<Annotation, DrawingError> {
    if points.len() < 2 {
        return Err(DrawingError::BadRequest("a dimension set needs an origin and at least one point".into()));
    }
    if orientation == LinearOrientation::Aligned {
        return Err(DrawingError::BadRequest("dimension sets are horizontal or vertical".into()));
    }
    let anchors: Vec<Anchor> = points
        .iter()
        .map(|p| dimension_anchor_near(index, ctx, *p))
        .collect::<Option<_>>()
        .ok_or(DrawingError::BadRequest("the part has no edges".into()))?;
    let originals: Vec<_> = anchors
        .iter()
        .map(|a| match index.resolve(&a.entity) {
            crate::Resolution::Found(crate::Entity::Edge(id)) => index.edge_curve(id),
            _ => None,
        })
        .collect();
    let curves: Vec<_> = anchors
        .iter()
        .zip(&originals)
        .map(|(anchor, original)| crate::update::anchor_curve(ctx, anchor, *original))
        .collect();
    let geometry: Vec<_> = anchors
        .iter()
        .zip(originals.iter().zip(&curves))
        .map(|(anchor, (original, curve))| crate::update::anchor_geometry(&ctx.xf, anchor, *original, curve.as_deref()))
        .collect();
    let mut set = DimensionSet { kind, orientation, values: Vec::new(), precision: 0, dim_type: DimType::Projected };
    let values =
        measure_set(&set, &geometry, &ctx.xf).ok_or(DrawingError::BadRequest("the points do not measure".into()))?;
    set.precision = values.iter().map(|v| precision_for(*v)).max().unwrap_or(0);
    set.values = values.into_iter().map(crate::model_items::round_value).collect();
    let across: Vec<f64> = anchors
        .iter()
        .map(|a| if orientation == LinearOrientation::Horizontal { a.cached.v } else { a.cached.u })
        .collect();
    let level = if offset >= 0.0 {
        across.iter().copied().fold(f64::NEG_INFINITY, f64::max) + offset
    } else {
        across.iter().copied().fold(f64::INFINITY, f64::min) + offset
    };
    let first = anchors[0].cached;
    let placement = if orientation == LinearOrientation::Horizontal {
        Point2::new(first.u, round(level))
    } else {
        Point2::new(round(level), first.v)
    };
    Ok(new_annotation(drawing, ctx.id, AnnotationKind::DimensionSet(set), anchors, placement))
}

/// An annotation of view `ctx` attached to the edge nearest the model point `at`, with
/// its text or symbol `offset` sheet millimetres from the anchor: a geometric tolerance,
/// a datum, a surface finish or a revision symbol.
pub fn attached(
    drawing: &Drawing,
    index: &PartIndex,
    ctx: &ViewContext,
    at: Vec3,
    body: AnnotationKind,
    offset: Point2,
) -> Result<Annotation, DrawingError> {
    let anchor = view_anchor_near(index, ctx, at).ok_or(DrawingError::BadRequest("the part has no edges".into()))?;
    let placement = Point2::new(round(anchor.cached.u + offset.u), round(anchor.cached.v + offset.v));
    Ok(new_annotation(drawing, ctx.id, body, vec![anchor], placement))
}
