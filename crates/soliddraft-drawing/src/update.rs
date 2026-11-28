//! Associativity: bringing a drawing up to date with its part.
//!
//! An update regenerates the line work of every view whose source digest differs from
//! the part's, then re-resolves every annotation's references against the new body.
//! Exactly one match moves the anchor and re-measures the value; a value that moved is
//! marked changed until someone accepts it. No match, or several, marks the annotation
//! dangling: it stays on the sheet with its last geometry and the cause in words, until
//! it is reattached, deleted or kept as a note. Nothing is removed by an update.

use serde::{Deserialize, Serialize};

use soliddraft_kernel_api::{KernelError, Point2};

use crate::annotation::{Annotation, AnnotationKind, AnnotationState, Attach, DimValue};
use crate::entity::{EntityKind, EntityRef};
use crate::ids::{AnnotationId, ViewId};
use crate::library::{self, Symbols};
use crate::measure::{AnchorGeometry, anchor_point, extension_direction, measure, value_text};
use crate::part::{Entity, Part, PartIndex, Resolution, feature_name};
use crate::project::{self, Contexts, Projected, ViewContext};
use crate::view::ViewKind;
use crate::{ContentHash, Drawing, DrawingError};

/// What an update did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateReport {
    /// The part revision the drawing was pinned at before.
    pub digest_before: ContentHash,
    /// The part revision it is pinned at now.
    pub digest_after: ContentHash,
    /// Views whose line work was regenerated.
    pub views_updated: usize,
    /// Dimensions whose displayed value changed in this update.
    pub changed: Vec<ChangedValue>,
    /// Annotations with references that do not resolve, after this update.
    pub dangling: Vec<DanglingAnnotation>,
    /// Annotations whose references all resolved.
    pub resolved: usize,
    /// Views that could not be made from the part any more, such as an auxiliary view
    /// whose edge is gone or a section whose plane no longer meets the part. They keep
    /// their last line work until they are repaired or deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dangling_views: Vec<DanglingView>,
    /// PMI items that could not be drawn after the update.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unplaced_pmi: Vec<crate::pmi_items::Unplaced>,
    /// Each regenerated view's projection: its transform and the circles it sees, for
    /// tools that pick on the views.
    #[serde(skip)]
    pub projections: Vec<(ViewId, Projected)>,
}

/// A view that lost what it is made from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DanglingView {
    /// The view.
    pub view: ViewId,
    /// The view's name on the drawing, for reports.
    #[serde(default)]
    pub name: String,
    /// Why, in words.
    pub cause: String,
}

/// A dimension whose value moved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangedValue {
    /// The annotation.
    pub annotation: AnnotationId,
    /// Its view.
    pub view: ViewId,
    /// The value shown before.
    pub previous: String,
    /// The value shown now.
    pub now: String,
}

/// An annotation that lost references.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DanglingAnnotation {
    /// The annotation.
    pub annotation: AnnotationId,
    /// Its view.
    pub view: ViewId,
    /// Indices of the lost anchors.
    pub lost: Vec<usize>,
    /// Why, in words.
    pub cause: String,
}

/// Brings `drawing` up to date with `part`: regenerates out-of-date views, and preview
/// views when `part` asks for certified line work,
/// re-resolves every annotation of every view that is not excluded from updates, and
/// pins the drawing's model reference at the part's revision. This is the drawing half
/// of a Rebuild; the caller binds and re-paths the reference (see
/// `soliddraft_document::resolve::repin`).
pub fn update_from_model(drawing: &mut Drawing, part: &Part) -> Result<UpdateReport, DrawingError> {
    let index = PartIndex::build(part)?;
    let mut report = UpdateReport {
        digest_before: drawing.model.pinned.clone(),
        digest_after: part.digest.clone(),
        views_updated: 0,
        changed: Vec::new(),
        dangling: Vec::new(),
        resolved: 0,
        dangling_views: Vec::new(),
        unplaced_pmi: Vec::new(),
        projections: Vec::new(),
    };
    let mut contexts = Contexts::new();
    let previous = saved_broken_transforms(drawing);
    let mut ids: Vec<ViewId> = drawing.sheets.iter().flat_map(|s| &s.views).map(|v| v.id).collect();
    ids.sort_by_key(|&id| {
        std::iter::successors(drawing.view(id), |view| view.geometry_parent().and_then(|parent| drawing.view(parent)))
            .take(17)
            .count()
    });
    for &id in &ids {
        let view = drawing.view(id).expect("listed");
        if view.excluded_from_update {
            continue;
        }
        let failure = view
            .geometry_parent()
            .and_then(|parent| report.dangling_views.iter().find(|failure| failure.view == parent))
            .map(|failure| failure.cause.clone());
        let failure = failure.or_else(|| {
            project::compute_with_previous(drawing, part, &index, id, &mut contexts, Some(&previous))
                .err()
                .map(|e| view_cause(&e))
        });
        if let Some(cause) = failure {
            let sheet = drawing.sheet_of(id).expect("listed");
            report.dangling_views.push(DanglingView {
                view: id,
                name: project::view_label_in(drawing, sheet, view),
                cause,
            });
            continue;
        }
        let ctx = &contexts[&id];
        let view = drawing.view_mut(id).expect("listed");
        // Proof and quality can improve independently at the same revision.
        if view.geometry.source_digest != part.digest.as_str()
            || (view.geometry.unproved && !ctx.projected.geometry.unproved)
            || (view.geometry.coarse && !ctx.projected.geometry.coarse)
            // Parent definitions can change linework without changing the part revision.
            || (view.geometry.unproved == ctx.projected.geometry.unproved
                && view.geometry.coarse == ctx.projected.geometry.coarse
                && view.geometry != ctx.projected.geometry)
        {
            if let Some(before) = previous.get(&id) {
                project::remap_view_boundaries(view, before, &ctx.xf);
            }
            view.geometry = ctx.projected.geometry.clone();
            report.views_updated += usize::from(view.is_printed());
        }
        report.projections.push((id, ctx.projected.clone()));
    }
    for id in ids {
        remap_parent(drawing, id, &previous, &contexts);
        follow_parent(drawing, id, &contexts);
    }
    let made_from_part = drawing.sheets.iter().position(|sheet| sheet.annotations.iter().any(|a| a.source.is_some()));
    let failed = |id| report.dangling_views.iter().any(|failure| failure.view == id);
    for sheet in &mut drawing.sheets {
        let symbols = library::template(sheet.template.as_str()).style_file().symbols.clone();
        refresh_hole_tables(sheet, &index, &contexts, |view| !failed(view));
        for a in &mut sheet.annotations {
            let Some(ctx) = contexts.get(&a.view) else { continue };
            if failed(a.view) || sheet.views.iter().any(|v| v.id == a.view && v.excluded_from_update) {
                continue;
            }
            if let Some(change) = re_resolve(a, &index, ctx, &symbols, part) {
                report.changed.push(change);
            }
        }
    }
    // What the drawing shows of the part's manufacturing information follows the part:
    // its values, and which items there are.
    if let Some(sheet) = made_from_part {
        // An item now shown another way is taken off here and drawn again by the import.
        crate::pmi_items::refresh(
            drawing,
            part,
            &index,
            |id| id == crate::SHEET || report.projections.iter().any(|(view, _)| *view == id),
            true,
        );
        report.changed.retain(|change| drawing.annotation(change.annotation).is_some());
        let views: Vec<ViewContext> = drawing.sheets[sheet]
            .views
            .iter()
            .filter(|v| !failed(v.id) && !v.excluded_from_update)
            .filter_map(|v| contexts.get(&v.id).cloned())
            .collect();
        report.unplaced_pmi = crate::pmi_items::import(drawing, sheet, part, &index, &views)?.unplaced;
        crate::pmi_items::place_notes(drawing, sheet, part);
    }
    collect(drawing, &mut report);
    report.dangling_views.retain(|failure| drawing.view(failure.view).is_some_and(crate::View::is_printed));
    drawing.refused_views.clone_from(&report.dangling_views);
    drawing.accept_source(part);
    Ok(report)
}

/// Why a view could not be made, in words for the Needs attention panel.
fn view_cause(e: &DrawingError) -> String {
    match e {
        DrawingError::ViewRefused { source, .. } => source.to_string(),
        DrawingError::LostReference(why) => {
            let mut c = why.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
        DrawingError::Kernel(KernelError::QueryFailed { message, .. }) => {
            format!("The kernel refused to draw it: {message}")
        }
        DrawingError::Kernel(k) => format!("The part cannot be cut there any more: {k}"),
        other => other.to_string(),
    }
}

/// Redraws the marks a derived view keeps on its parent from where they are on the
/// part: a section's cutting line and a detail's centre.
pub(crate) fn follow_parent(drawing: &mut Drawing, id: ViewId, contexts: &Contexts) {
    let Some(view) = drawing.view(id) else { return };
    let Some(parent) = view.geometry_parent().and_then(|p| drawing.view(p)) else { return };
    let Some(pctx) = contexts.get(&parent.id).filter(|ctx| ctx.projected.geometry == parent.geometry) else {
        return;
    };
    let xf = pctx.xf;
    let view = drawing.view_mut(id).expect("found");
    project::reproject_parent_marks(&mut view.kind, &xf);
}

/// Points an auxiliary view at another edge, the repair for a lost reference edge, and
/// remakes its line work.
pub fn reattach_view(drawing: &mut Drawing, part: &Part, id: ViewId, entity: EntityRef) -> Result<(), DrawingError> {
    let view = drawing.view_mut(id).ok_or(DrawingError::UnknownView(id))?;
    let ViewKind::Auxiliary { edge, .. } = &mut view.kind else {
        return Err(DrawingError::BadRequest(format!("{id} is not an auxiliary view")));
    };
    *edge = entity;
    regenerate_view(drawing, part, id)
}

/// Remakes a view and its dependent descendants, keeping placement and accepted
/// dimension values. Descendants excluded from updates keep their linework.
pub fn regenerate_view(drawing: &mut Drawing, part: &Part, id: ViewId) -> Result<(), DrawingError> {
    regenerate(drawing, part, id, None)
}

pub(crate) fn edit_breaks(
    drawing: &mut Drawing,
    part: &Part,
    id: ViewId,
    breaks: Vec<crate::Break>,
) -> Result<(), DrawingError> {
    let view = drawing.view(id).ok_or(DrawingError::UnknownView(id))?;
    if matches!(view.kind, ViewKind::Detail { .. }) {
        return Err(DrawingError::BadRequest("A detail inherits its parent's break".into()));
    }
    regenerate(drawing, part, id, Some(breaks))
}

fn saved_broken_transforms(drawing: &Drawing) -> project::Transforms {
    drawing
        .sheets
        .iter()
        .flat_map(|sheet| &sheet.views)
        .filter(|view| !view.geometry.breaks.is_empty())
        .filter_map(|view| Some((view.id, view.geometry.transform?)))
        .collect()
}

fn remap_parent(drawing: &mut Drawing, id: ViewId, previous: &project::Transforms, contexts: &Contexts) {
    if let Some(parent) = drawing.view(id).and_then(crate::View::geometry_parent)
        && let (Some(before), Some(after)) = (previous.get(&parent), contexts.get(&parent))
        && drawing.view(parent).is_some_and(|view| view.geometry == after.projected.geometry)
    {
        let view = drawing.view_mut(id).expect("listed view");
        project::remap_parent_boundary(&mut view.kind, before, &after.xf);
    }
}

fn regenerate(
    drawing: &mut Drawing,
    part: &Part,
    id: ViewId,
    breaks: Option<Vec<crate::Break>>,
) -> Result<(), DrawingError> {
    let descendants = project::dependent_views(drawing, id);
    let ids: Vec<_> = descendants
        .iter()
        .copied()
        .filter(|child| *child == id || !drawing.view(*child).is_some_and(|v| v.excluded_from_update))
        .collect();
    let index = PartIndex::build(part)?;
    let mut previous = saved_broken_transforms(drawing);
    let mut staged = drawing.clone();
    let break_edit = breaks.is_some();
    if let Some(breaks) = breaks {
        // Where the views were drawn, so crops and callouts stay on the same material.
        previous.extend(descendants.iter().filter_map(|&id| Some((id, drawing.view(id)?.geometry.transform?))));
        let mut before = Contexts::new();
        for &view in &ids {
            if !previous.contains_key(&view) {
                let _ = project::compute_in(drawing, part, &index, view, &mut before);
            }
        }
        let estimated: Vec<_> = before
            .into_iter()
            .filter(|(id, _)| {
                !previous.contains_key(id)
                    && drawing.view(*id).is_some_and(|view| {
                        view.geometry.breaks.is_empty() || view.geometry.source_digest == part.digest.as_str()
                    })
            })
            .map(|(view, ctx)| (view, ctx.xf))
            .collect();
        previous.extend(estimated);
        staged.view_mut(id).expect("existing view").breaks = breaks;
    }
    let outgrown: Vec<ViewId> = ids
        .iter()
        .copied()
        .filter(|view| {
            break_edit
                && !previous.contains_key(view)
                && drawing.view(*view).is_some_and(|v| !v.geometry.breaks.is_empty())
        })
        .collect();
    if !outgrown.is_empty() {
        // The plain view gives the frame, scale and centre the saved gap is drawn with.
        let mut plain = staged.clone();
        for &view in &outgrown {
            let v = plain.view_mut(view).expect("listed view");
            project::clear_clipping(v);
        }
        let mut fresh = Contexts::new();
        for view in outgrown {
            project::compute_in(&plain, part, &index, view, &mut fresh)?;
            let saved = project::saved_transform(drawing, &fresh[&view]);
            if let Some(xf) = saved {
                previous.insert(view, xf);
            }
        }
    }
    let mut contexts = Contexts::new();
    for &id in &ids {
        project::compute_with_previous(
            &staged,
            part,
            &index,
            id,
            &mut contexts,
            (!previous.is_empty()).then_some(&previous),
        )?;
    }
    for &id in &ids {
        let ctx = &contexts[&id];
        let view = staged.view_mut(id).expect("computed view");
        if let Some(before) = previous.get(&id) {
            project::remap_view_boundaries(view, before, &ctx.xf);
        }
        view.geometry = ctx.projected.geometry.clone();
    }
    for child in descendants {
        remap_parent(&mut staged, child, &previous, &contexts);
        follow_parent(&mut staged, child, &contexts);
    }
    for sheet in &mut staged.sheets {
        refresh_hole_tables(sheet, &index, &contexts, |view| ids.contains(&view));
        let symbols = &library::template(sheet.template.as_str()).style_file().symbols;
        for a in sheet.annotations.iter_mut().filter(|a| ids.contains(&a.view)) {
            let context = &contexts[&a.view];
            re_resolve(a, &index, context, symbols, part);
        }
    }
    if part.digest == staged.model.pinned {
        crate::pmi_items::refresh(&mut staged, part, &index, |view| ids.contains(&view), false);
    }
    staged.refused_views.retain(|refusal| !ids.contains(&refusal.view));
    *drawing = staged;
    Ok(())
}

fn refresh_hole_tables(
    sheet: &mut crate::Sheet,
    index: &PartIndex,
    contexts: &Contexts,
    updated: impl Fn(ViewId) -> bool,
) {
    let Some(namer) = index.namer() else { return };
    for table in &mut sheet.tables {
        if let Some(view) = table.hole_view.filter(|&view| updated(view))
            && let Some(ctx) = contexts.get(&view)
            && sheet.views.iter().any(|v| v.id == view && !v.excluded_from_update)
        {
            table.holes = ctx.projected.hole_rows(namer);
        }
    }
}

fn collect(drawing: &Drawing, report: &mut UpdateReport) {
    for a in drawing.sheets.iter().flat_map(|s| &s.annotations) {
        match &a.state {
            AnnotationState::Dangling { lost, cause } => report.dangling.push(DanglingAnnotation {
                annotation: a.id,
                view: a.view,
                lost: lost.clone(),
                cause: cause.clone(),
            }),
            _ if !a.anchors.is_empty() => report.resolved += 1,
            _ => {}
        }
    }
}

fn resolve_anchor(index: &PartIndex, ctx: &ViewContext, anchor: &crate::Anchor) -> Resolution {
    project::resolve_in_view(index, ctx, &anchor.entity)
}

/// Re-resolves one annotation; returns the change when its displayed value moved.
pub(crate) fn re_resolve(
    a: &mut Annotation,
    index: &PartIndex,
    ctx: &ViewContext,
    symbols: &Symbols,
    part: &Part,
) -> Option<ChangedValue> {
    if a.anchors.is_empty() {
        return None;
    }
    let found: Vec<Result<Entity, String>> = a
        .anchors
        .iter()
        .map(|anchor| match resolve_anchor(index, ctx, anchor) {
            Resolution::Found(e) => Ok(e),
            Resolution::Lost(why) => Err(why),
        })
        .collect();
    let lost: Vec<usize> = found.iter().enumerate().filter(|(_, r)| r.is_err()).map(|(i, _)| i).collect();
    if !lost.is_empty() {
        let refs: Vec<&EntityRef> = lost.iter().map(|i| &a.anchors[*i].entity).collect();
        let causes: Vec<String> = lost.iter().filter_map(|i| found[*i].clone().err()).collect();
        a.state = AnnotationState::Dangling { lost: lost.clone(), cause: cause(a.anchors.len(), &refs, &causes) };
        return None;
    }
    let entities: Vec<Entity> = found.into_iter().map(|r| r.expect("no losses")).collect();
    for (anchor, e) in a.anchors.iter_mut().zip(&entities) {
        if let Some(mut r) = index.entity_ref(*e) {
            r.section_leg = anchor.entity.section_leg;
            anchor.entity = r;
        }
    }
    remeasure(a, index, ctx, &entities, symbols, part)
}

pub(crate) fn anchor_curve<'a>(
    ctx: &ViewContext,
    anchor: &crate::Anchor,
    curve: Option<&'a soliddraft_kernel_api::drawing::Curve3>,
) -> Option<std::borrow::Cow<'a, soliddraft_kernel_api::drawing::Curve3>> {
    let curve = curve?;
    Some(match ctx.xf.leg_turn(anchor.entity.section_leg) {
        Some(turn) => std::borrow::Cow::Owned(turn.curve(curve)),
        None => std::borrow::Cow::Borrowed(curve),
    })
}

pub(crate) fn anchor_geometry<'a>(
    xf: &project::ViewTransform,
    anchor: &crate::Anchor,
    curve: Option<&'a soliddraft_kernel_api::drawing::Curve3>,
) -> AnchorGeometry<'a> {
    let turn = xf.leg_turn(anchor.entity.section_leg);
    let placed = |p| turn.map_or(p, |turn| turn.point(p));
    let attach = match anchor.attach {
        Attach::Point { at } => Attach::Point { at: placed(at) },
        other => other,
    };
    AnchorGeometry { curve, point: placed(anchor.entity.fingerprint.point), attach }
}

/// Re-caches the anchors' points and re-measures a dimension whose anchors all resolve to
/// `entities`; sets the state and returns the change when the displayed value moved.
fn remeasure(
    a: &mut Annotation,
    index: &PartIndex,
    ctx: &ViewContext,
    entities: &[Entity],
    symbols: &Symbols,
    part: &Part,
) -> Option<ChangedValue> {
    let authored_angle = crate::pmi_items::authored_angle(a, part);
    let curves: Vec<_> = a
        .anchors
        .iter()
        .zip(entities)
        .map(|(anchor, e)| match e {
            Entity::Edge(id) => anchor_curve(ctx, anchor, index.edge_curve(*id)),
            Entity::Face(_) => None,
        })
        .collect();
    let geometry: Vec<_> = a
        .anchors
        .iter()
        .zip(&curves)
        .map(|(anchor, curve)| anchor_geometry(&ctx.xf, anchor, curve.as_deref()))
        .collect();
    let (before_state, mut change) = (a.state.clone(), None);
    match &mut a.body {
        AnnotationKind::Dimension(d) => {
            let old = value_text(d);
            let measured = if d.kind == crate::DimensionKind::Chamfer {
                crate::measure::chamfer(&geometry, &ctx.xf).map(|(size, angle)| {
                    d.chamfer_angle = Some(crate::model_items::round_value(angle));
                    size
                })
            } else if d.kind == crate::DimensionKind::Angular {
                authored_angle.or_else(|| measure(&d.kind, d.dim_type, &geometry, &ctx.xf))
            } else {
                measure(&d.kind, d.dim_type, &geometry, &ctx.xf)
            };
            if matches!(d.value, DimValue::Measured { .. }) && measured.is_none() {
                a.state = AnnotationState::Dangling {
                    lost: (0..a.anchors.len()).collect(),
                    cause: "The references cannot measure this dimension".into(),
                };
                return None;
            }
            let across = extension_direction(&d.kind, Default::default(), Default::default());
            for (anchor, g) in a.anchors.iter_mut().zip(&geometry) {
                anchor.cached = ctx.xf.point(anchor_point(g, &ctx.xf, across));
            }
            if let (DimValue::Measured { value }, Some(new)) = (&mut d.value, measured) {
                *value = crate::model_items::round_value(new);
            }
            let now = value_text(d);
            let accepted = match &before_state {
                AnnotationState::Changed { previous } => previous.clone(),
                _ => old.clone(),
            };
            a.state =
                if now == accepted { AnnotationState::Ok } else { AnnotationState::Changed { previous: accepted } };
            if now != old {
                change = Some(ChangedValue { annotation: a.id, view: a.view, previous: old, now });
            }
        }
        AnnotationKind::DimensionSet(set) => {
            let across = match set.orientation {
                crate::LinearOrientation::Vertical => Point2::new(1.0, 0.0),
                _ => Point2::new(0.0, 1.0),
            };
            for (anchor, g) in a.anchors.iter_mut().zip(&geometry) {
                anchor.cached = ctx.xf.point(anchor_point(g, &ctx.xf, across));
            }
            let old = set.values_text();
            let Some(values) = crate::measure::measure_set(set, &geometry, &ctx.xf) else {
                a.state = AnnotationState::Dangling {
                    lost: (0..a.anchors.len()).collect(),
                    cause: "The references cannot measure this dimension set".into(),
                };
                return None;
            };
            set.values = values.into_iter().map(crate::model_items::round_value).collect();
            let now = set.values_text();
            let accepted = match &before_state {
                AnnotationState::Changed { previous } => previous.clone(),
                _ => old.clone(),
            };
            a.state =
                if now == accepted { AnnotationState::Ok } else { AnnotationState::Changed { previous: accepted } };
            if now != old {
                change = Some(ChangedValue { annotation: a.id, view: a.view, previous: old, now });
            }
        }
        body => {
            if matches!(body, AnnotationKind::Centreline)
                && a.anchors.len() == 4
                && crate::measure::projected_edge_ends(&geometry, &ctx.xf)
                    .and_then(crate::measure::centreline)
                    .is_none()
            {
                a.state = AnnotationState::Dangling {
                    lost: (0..a.anchors.len()).collect(),
                    cause: "The centreline needs two distinct parallel edges with overlapping lengths".into(),
                };
                return None;
            }
            for (anchor, g) in a.anchors.iter_mut().zip(&geometry) {
                anchor.cached = ctx.xf.point(anchor_point(g, &ctx.xf, Default::default()));
            }
            a.state = AnnotationState::Ok;
            match body {
                AnnotationKind::CentreMark => {
                    if let Some(anchor) = a.anchors.first() {
                        a.placement = anchor.cached;
                    }
                }
                AnnotationKind::HoleCallout { text } => {
                    let circle = a.anchors.first().and_then(|anchor| {
                        ctx.projected.circles.iter().find(|c| c.entity.same_projected_entity(&anchor.entity))
                    });
                    let fresh = circle.and_then(|c| index.namer()?.hole_callout(c, &ctx.projected, symbols));
                    if let Some(fresh) = fresh
                        && fresh != *text
                    {
                        let previous = std::mem::replace(text, fresh.clone());
                        a.state = AnnotationState::Changed { previous: previous.clone() };
                        change = Some(ChangedValue { annotation: a.id, view: a.view, previous, now: fresh });
                    }
                }
                _ => {}
            }
        }
    }
    change
}

/// The Needs attention text for lost references, such as "Both references were edges of
/// slot, which no longer exists".
fn cause(total: usize, lost: &[&EntityRef], causes: &[String]) -> String {
    let gone: Vec<String> = lost.iter().map(|r| feature_name(r)).collect();
    let all_missing = causes.iter().all(|c| c.ends_with("which no longer exists"));
    let kind = |r: &EntityRef| match r.kind {
        EntityKind::Edge => "edge",
        EntityKind::Face => "face",
        EntityKind::Vertex => "vertex",
    };
    if all_missing {
        let mut features: Vec<String> = gone.clone();
        features.sort_unstable();
        features.dedup();
        let names = features.join(" and ");
        let exists = if features.len() == 1 { "exists" } else { "exist" };
        return match (lost.len(), total) {
            (1, 1) => format!("The reference was an {} of {names}, which no longer {exists}", kind(lost[0])),
            (2, 2) => format!("Both references were {}s of {names}, which no longer {exists}", kind(lost[0])),
            (n, t) if n == t => format!("All {n} references were on {names}, which no longer {exists}"),
            (n, t) => format!("{n} of {t} references were on {names}, which no longer {exists}"),
        };
    }
    let mut unique: Vec<&String> = causes.iter().collect();
    unique.dedup();
    let text = unique.into_iter().cloned().collect::<Vec<_>>().join("; ");
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => text,
    }
}

/// Attaches end `end` of annotation `id` to `entity` and, once nothing is lost,
/// re-measures it: the Reattach repair.
pub fn reattach(
    drawing: &mut Drawing,
    part: &Part,
    id: AnnotationId,
    end: usize,
    entity: EntityRef,
) -> Result<(), DrawingError> {
    let index = PartIndex::build(part)?;
    let view = drawing.annotation(id).ok_or(DrawingError::UnknownAnnotation(id))?.view;
    let ctx = ViewContext::compute(drawing, part, &index, view)?;
    let template = drawing.sheet_of(view).map(|s| s.template.as_str()).unwrap_or("");
    let symbols = library::template(template).style_file().symbols.clone();
    let mut repaired = drawing.annotation(id).ok_or(DrawingError::UnknownAnnotation(id))?.clone();
    let a = &mut repaired;
    let anchor = a.anchors.get_mut(end).ok_or(DrawingError::BadRequest(format!("{id} has no end {end}")))?;
    anchor.entity = entity;
    // A round takes its centre, a line its middle, unless the end picks a side.
    if let Resolution::Found(Entity::Edge(edge)) = resolve_anchor(&index, &ctx, anchor)
        && let Some(curve) = index.edge_curve(edge)
    {
        let round = crate::measure::radius_of(curve).is_some();
        if round && !matches!(anchor.attach, Attach::Min | Attach::Max | Attach::EdgeParameter { .. }) {
            anchor.attach = Attach::Centre;
        } else if !round && matches!(anchor.attach, Attach::Centre | Attach::Silhouette) {
            anchor.attach = Attach::Mid;
        } else if matches!(anchor.attach, Attach::Point { .. }) {
            let mid = curve.point_at(0.5);
            let at = ctx.retained_edge_point(edge, anchor.entity.section_leg, curve, mid).unwrap_or(mid);
            anchor.attach = Attach::Point { at };
        }
        let curve = anchor_curve(&ctx, anchor, Some(curve));
        let g = anchor_geometry(&ctx.xf, anchor, curve.as_deref());
        anchor.cached = ctx.xf.point(anchor_point(&g, &ctx.xf, Default::default()));
    }
    let resolved: Vec<Option<Entity>> = a
        .anchors
        .iter()
        .map(|x| match resolve_anchor(&index, &ctx, x) {
            Resolution::Found(e) => Some(e),
            Resolution::Lost(_) => None,
        })
        .collect();
    if resolved[end].is_none() {
        return Err(DrawingError::BadRequest(format!("the new reference for end {end} of {id} does not resolve")));
    }
    let entities: Vec<Entity> = resolved.iter().flatten().copied().collect();
    if !crate::pmi_items::anchors_on_item(a, &resolved, part, &index) {
        a.source = None;
    }
    if let AnnotationState::Dangling { lost, cause } = &a.state {
        let lost: Vec<usize> = lost.iter().copied().filter(|i| *i != end && resolved[*i].is_none()).collect();
        if !lost.is_empty() {
            a.state = AnnotationState::Dangling { lost, cause: cause.clone() };
            *drawing.annotation_mut(id).expect("found") = repaired;
            return Ok(());
        }
    }
    if entities.len() == a.anchors.len() {
        // Keep the dimension's last accepted value as the comparison point.
        if let AnnotationState::Dangling { .. } = a.state {
            a.state = AnnotationState::Ok;
        }
        remeasure(a, &index, &ctx, &entities, &symbols, part);
        if let AnnotationState::Dangling { cause, .. } = &a.state {
            return Err(DrawingError::BadRequest(cause.clone()));
        }
        // Reattaching is a deliberate edit: the value it gives is the accepted one.
        if let AnnotationState::Changed { .. } = a.state {
            a.state = AnnotationState::Ok;
        }
    }
    *drawing.annotation_mut(id).expect("found") = repaired;
    Ok(())
}

/// The name of the edge nearest a model point, for picking a reattach target without a
/// window.
pub fn edge_near(part: &Part, at: soliddraft_kernel_api::Vec3) -> Result<EntityRef, DrawingError> {
    let index = PartIndex::build(part)?;
    let (id, _) =
        crate::annotate::nearest_edge(&index, at).ok_or(DrawingError::BadRequest("the part has no edges".into()))?;
    index.edge_ref(id).ok_or(DrawingError::BadRequest("edge vanished".into()))
}
