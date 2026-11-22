//! Drawing queries. Exported views, silhouettes, sections, edge curves and face loops
//! come from Catallax's hidden-line views and drawing curves. Screen views may keep the
//! display mesh, marked unproved.

use std::collections::{BTreeMap, BTreeSet};

use catallax_app::queries::SectionPlane;
use catallax_app::views::{self, CONTOUR_FIT_MARK, DRAWING_FIT_MARK, DrawingCurve, StrokeSource};
use catallax_numbers::{Answer, Decider, MarkLog, Rational, Refusal};
use catallax_shapes::{RPoint3, RVector3, Surface};
use catallax_topology::{Loop, Model};
use soliddraft_kernel_api::drawing::{Curve3, CurveSource, ViewCurve, ViewCurves, ViewSpec, mesh_hlr};
use soliddraft_kernel_api::*;

use crate::{CatallaxKernel, Solid};

/// Chord of a screen view of a fitted body: Catallax's default display chord, doubled because
/// the adapter halves a requested chord. A fitted body's preview shares the certified display
/// route, whose cost grows as the chord shrinks, and a view's automatic chord is far below a pixel.
const FITTED_VIEW_CHORD: f64 = 0.02;

/// Automatic screen views use a chord of 0.02 percent of the body diagonal.
const PREVIEW_VIEW_CHORD_RATIO: f64 = 2e-4;

/// The view a request means on this body. An exported view and an explicit tolerance stay as
/// they are. The chord also sets the depth margin, so the two stay matched.
pub(super) fn resolved_view(k: &CatallaxKernel, body: BodyId, spec: &ViewSpec) -> ViewSpec {
    let fitted = k.solid(body).is_ok_and(|solid| {
        let built = solid.app.checkpoint.built();
        built.model.blend_base(built.result).is_some()
    });
    if spec.preview && spec.tolerance <= 0.0 {
        let tolerance = if fitted {
            FITTED_VIEW_CHORD
        } else if let Ok(topology) = k.topology(body) {
            PREVIEW_VIEW_CHORD_RATIO * topology.bbox.diagonal().max(1.0)
        } else {
            return *spec;
        };
        ViewSpec { tolerance, ..*spec }
    } else {
        *spec
    }
}

pub(super) fn check_exact_tolerance(spec: &ViewSpec) -> KernelResult<()> {
    if spec.tolerance != 0.0 {
        return Err(KernelError::InvalidInput("Drawing tolerance applies to mesh preview only".into()));
    }
    Ok(())
}

/// A view of `body`. A screen view comes from the display mesh and is marked unproved;
/// an exported one is Catallax's hidden-line view.
pub(super) fn view_curves(k: &CatallaxKernel, body: BodyId, spec: &ViewSpec) -> KernelResult<ViewCurves> {
    if spec.frame().is_none() {
        return Err(KernelError::InvalidInput("view direction is zero or parallel to up".into()));
    }
    if !spec.preview {
        check_exact_tolerance(spec)?;
        require_drawing_solid(k, body, "view_curves")?;
        return exact_view(k.solid(body)?.as_ref(), spec);
    }
    if k.topology(body)?.kind != BodyKind::Solid {
        return Err(KernelError::Unsupported("view_curves of a sheet body".into()));
    }
    if spec.tolerance <= 0.0 && k.solid(body).is_ok_and(|solid| solid.has_splines()) {
        let spec =
            ViewSpec { tolerance: catallax_mesh::MeshOptions::default().chord_tolerance.to_f64_nearest(), ..*spec };
        // Automatic screen line work shares the named mesh.
        let mut view = mesh_hlr::rendering_view_curves(k, body, &spec)?;
        view.unproved = true;
        return Ok(view);
    }
    let spec = &resolved_view(k, body, spec);
    let mut view = mesh_hlr::view_curves(k, body, spec)?;
    view.unproved = true;
    let Ok(solid) = k.solid(body) else { return Ok(view) };
    let mut counts = BTreeMap::new();
    for curve in &view.curves {
        *counts.entry(curve.source).or_insert(0) += 1;
    }
    // Keep whole lines and circles for dimensions after polygon visibility. Split edges
    // and silhouettes keep the mesh result.
    for curve in &mut view.curves {
        if let CurveSource::Edge(edge) = curve.source
            && counts[&curve.source] == 1
            && let Ok(drawn) = edge_drawing(&solid, body, edge)
            && matches!(drawn.curve, Curve3::Line { .. } | Curve3::Circle { .. })
        {
            curve.curve = drawn.curve;
            curve.approximate |= drawn.approximate;
        }
    }
    Ok(ViewCurves {
        unproved: view.unproved,
        ..ViewCurves::new(view.curves, &spec.frame().expect("mesh_hlr checked the frame"))
    })
}

pub(super) fn require_drawing_solid(k: &CatallaxKernel, body: BodyId, query: &str) -> KernelResult<()> {
    if k.field(body)?.is_some() || k.approximate(body)?.is_some() || k.body_kind(body)? != BodyKind::Solid {
        return Err(crate::query_error(query, Refusal::new("app.view-solid", "the body has no exact drawing solid")));
    }
    Ok(())
}

/// Catallax's hidden-line view, each stroke drawn as an exact or fitted curve. Tangent
/// edges are left out unless `spec.smooth_edges`, seams unless `spec.seams`. A refusal
/// reaches the caller as a refusal.
fn exact_view(solid: &Solid, spec: &ViewSpec) -> KernelResult<ViewCurves> {
    let frame = spec.frame().ok_or_else(|| KernelError::InvalidInput("view direction is zero".into()))?;
    let built = solid.app.checkpoint.built();
    let (model, body) = (&built.model, built.result);
    let direction = direction(spec.direction)?;
    let drawn = catallax("view_curves", |dec| {
        let view = views::hidden_line_view(model, body, &direction, dec).map_err(|q| q.refusal)?;
        let mut marks = view.marks;
        let mut drawn = Vec::new();
        for stroke in &view.value.strokes {
            if (stroke.smooth && !spec.smooth_edges) || (stroke.seam && !spec.seams) {
                continue;
            }
            let curve = views::stroke_drawing(&view.value, stroke, dec)?;
            marks.extend(&curve.marks);
            drawn.push((stroke.source, stroke.seam, stroke.visible, stroke.smooth, curve.value));
        }
        Ok(Answer::with_marks(drawn, marks))
    })?;
    let mut curves = Vec::with_capacity(drawn.value.len());
    for (source, seam, visible, smooth, curve) in drawn.value {
        let source = match source {
            StrokeSource::Edge(edge) if seam => CurveSource::Seam(face_id(solid, seam_face(model, edge)?)?),
            StrokeSource::Edge(edge) => CurveSource::Edge(edge_id(solid, edge)?),
            StrokeSource::Contour(face) => CurveSource::Silhouette(face_id(solid, face)?),
        };
        curves.push(ViewCurve {
            curve: curve3(&curve.curve),
            source,
            visible,
            smooth,
            approximate: curve.fit.is_some(),
        });
    }
    Ok(ViewCurves { catallax: true, approximate: view_marks(&drawn.marks), ..ViewCurves::new(curves, &frame) })
}

/// The outline of `body` seen from `direction`: Catallax's contour generators, each drawn
/// as an exact or fitted curve and tagged with its curved face.
pub(super) fn silhouette_curves(k: &CatallaxKernel, body: BodyId, direction: Vec3) -> KernelResult<Vec<ViewCurve>> {
    let solid = k.solid(body)?;
    let built = solid.app.checkpoint.built();
    let (model, result) = (&built.model, built.result);
    let toward = self::direction(direction)?;
    let drawn = catallax("silhouette_curves", |dec| {
        let contours = views::contour_generators(model, result, &toward, dec).map_err(|q| q.refusal)?;
        let mut marks = contours.marks;
        let mut drawn = Vec::new();
        for arc in &contours.value.arcs {
            let curve = views::drawing_curve(&contours.value.model, arc.curve, arc.start, arc.end, dec)?;
            marks.extend(&curve.marks);
            drawn.push((arc.face, arc.fit.is_some() || curve.value.fit.is_some(), curve.value.curve));
        }
        Ok(Answer::with_marks(drawn, marks))
    })?;
    let whole = !view_marks(&drawn.marks).is_empty();
    drawn
        .value
        .into_iter()
        .map(|(face, fitted, curve)| {
            Ok(ViewCurve {
                approximate: fitted || whole,
                ..ViewCurve::new(curve3(&curve), CurveSource::Silhouette(face_id(&solid, face)?), true)
            })
        })
        .collect()
}

/// Where `plane` cuts `body`: Catallax's face-by-face section, each piece drawn as an
/// exact or fitted curve and tagged with the face it cuts. An edge of the body in the
/// plane is tagged with its face that leaves the plane, not a face lying in it.
pub(super) fn section_curves(k: &CatallaxKernel, body: BodyId, plane: &Plane) -> KernelResult<Vec<ViewCurve>> {
    let solid = k.solid(body)?;
    let built = solid.app.checkpoint.built();
    let (model, result) = (&built.model, built.result);
    let [x, y, z] = [plane.origin.x, plane.origin.y, plane.origin.z].map(|v| exact(v, "section origin"));
    let plane = SectionPlane { origin: RPoint3::new(x?, y?, z?), normal: vector(plane.normal, "section normal")? };
    let drawn = catallax("section_curves", |dec| {
        let section = catallax_app::queries::section_curves(model, result, &plane, dec).map_err(|q| q.refusal)?;
        let mut marks = section.marks;
        let in_plane: BTreeSet<_> = section.value.curves.iter().filter_map(|arc| arc.edge).collect();
        let mut drawn = Vec::new();
        for arc in &section.value.curves {
            let curve = views::drawing_curve(&section.value.model, arc.curve, arc.start, arc.end, dec)?;
            marks.extend(&curve.marks);
            // A planar face whose every edge lies in the plane lies in it: a cap.
            let face = arc
                .faces
                .iter()
                .copied()
                .find(|face| arc.edge.is_none() || !in_plane_face(model, *face, &in_plane))
                .or(arc.faces.first().copied());
            drawn.push((face, curve.value.fit.is_some(), curve.value.curve));
        }
        Ok(Answer::with_marks(drawn, marks))
    })?;
    let whole = !view_marks(&drawn.marks).is_empty();
    drawn
        .value
        .into_iter()
        .map(|(face, fitted, curve)| {
            let source = match face {
                Some(face) => CurveSource::Section(face_id(&solid, face)?),
                None => CurveSource::Unknown,
            };
            Ok(ViewCurve { approximate: fitted || whole, ..ViewCurve::new(curve3(&curve), source, true) })
        })
        .collect()
}

fn in_plane_face(
    model: &Model,
    face: catallax_topology::FaceId,
    in_plane: &BTreeSet<catallax_topology::EdgeId>,
) -> bool {
    let stored = model.face(face);
    matches!(model.geom().surface(stored.surface), Surface::Plane(_))
        && stored.loops.iter().all(|lp| match model.loop_(*lp) {
            Loop::Coedges(coedges) => coedges.iter().all(|c| in_plane.contains(&model.coedge(*c).edge)),
            _ => false,
        })
}

/// The exact or fitted curve of an edge, from its start to its end.
pub(super) fn edge_drawing(solid: &Solid, body: BodyId, edge: EdgeId) -> KernelResult<ViewCurve> {
    let id = solid
        .ids
        .edges
        .iter()
        .find_map(|(id, e)| (*e == edge).then_some(*id))
        .ok_or(KernelError::UnknownEdge { body, edge })?;
    let built = solid.app.checkpoint.built();
    let drawn =
        catallax("edge_curve", |dec| views::edge_drawing(&built.model, built.result, id, dec).map_err(|q| q.refusal))?;
    let curve = ViewCurve {
        approximate: drawn.value.fit.is_some() || !drawn.marks.is_empty(),
        ..ViewCurve::new(curve3(&drawn.value.curve), CurveSource::Edge(edge), true)
    };
    Ok(curve)
}

/// A face's boundary loops as exact or fitted edge curves in travel order, outer loop
/// first.
pub(super) fn face_drawing_loops(solid: &Solid, body: BodyId, face: FaceId) -> KernelResult<Vec<Vec<ViewCurve>>> {
    let id = solid
        .ids
        .faces
        .iter()
        .find_map(|(id, f)| (*f == face).then_some(*id))
        .ok_or(KernelError::UnknownFace { body, face })?;
    let built = solid.app.checkpoint.built();
    let drawn = catallax("face_loops", |dec| {
        views::face_drawing_loops(&built.model, built.result, id, dec).map_err(|q| q.refusal)
    })?;
    let marked = !view_marks(&drawn.marks).is_empty();
    let mut loops: Vec<Vec<ViewCurve>> = drawn
        .value
        .iter()
        .map(|chain| {
            chain
                .iter()
                .map(|edge| {
                    Ok(ViewCurve {
                        approximate: marked || edge.drawn.fit.is_some(),
                        ..ViewCurve::new(curve3(&edge.drawn.curve), CurveSource::Edge(edge_id(solid, edge.edge)?), true)
                    })
                })
                .collect::<KernelResult<Vec<_>>>()
        })
        .collect::<KernelResult<_>>()?;
    loops.retain(|chain| !chain.is_empty());
    // Catallax does not order loops outer-first. The largest planar loop encloses the holes.
    let area = |curves: &[ViewCurve]| soliddraft_kernel_api::drawing::section::loop_area_vector(curves).length();
    loops.sort_by(|a, b| area(b).total_cmp(&area(a)));
    Ok(loops)
}

/// Runs a Catallax drawing query on the request's allowance, or the application's when
/// there is none. A refusal, an exhausted allowance included, comes back as an error,
/// never as a mesh view.
fn catallax<T>(query: &str, run: impl FnOnce(&mut Decider) -> Result<Answer<T>, Refusal>) -> KernelResult<Answer<T>> {
    let within = || {
        let answer = crate::numerical(query, run);
        if catallax_numbers::current_operation_work().exhausted {
            return Err(Refusal::new("numbers.operation-budget", "operation work allowance exhausted"));
        }
        answer
    };
    let answer = if catallax_numbers::operation_budget_active() {
        within()
    } else {
        catallax_numbers::with_operation_budget(crate::APP_REQUEST_WORK, within).0
    };
    answer.map_err(|refusal| crate::query_error(query, refusal))
}

/// The marks that make a whole answer approximate. A fitted curve's own marks are
/// shown on that curve instead.
fn view_marks(marks: &MarkLog) -> Vec<String> {
    let codes: BTreeSet<&str> = marks
        .iter()
        .map(|mark| mark.code)
        .filter(|code| ![DRAWING_FIT_MARK, CONTOUR_FIT_MARK].contains(code))
        .collect();
    codes.into_iter().map(str::to_owned).collect()
}

fn curve3(curve: &DrawingCurve) -> Curve3 {
    let v = |p: &[f64; 3]| Vec3::new(p[0], p[1], p[2]);
    match curve {
        DrawingCurve::Line { a, b } => Curve3::Line { a: v(a), b: v(b) },
        DrawingCurve::Arc { center, normal, x_axis, radius, start_deg, end_deg } => Curve3::Arc {
            center: v(center),
            normal: v(normal),
            x_axis: v(x_axis),
            radius: *radius,
            start_deg: *start_deg,
            end_deg: *end_deg,
        },
        DrawingCurve::Circle { center, normal, radius } => {
            Curve3::Circle { center: v(center), normal: v(normal), radius: *radius }
        }
        DrawingCurve::Nurbs { degree, knots, points, weights } => Curve3::Nurbs {
            degree: *degree as u32,
            knots: knots.iter().map(|knot| knot - knots[0]).collect(),
            points: points.iter().map(v).collect(),
            weights: weights.clone(),
        },
    }
}

fn exact(value: f64, what: &str) -> KernelResult<Rational> {
    Rational::from_f64_exact(value).ok_or_else(|| KernelError::InvalidInput(format!("the {what} is not finite")))
}

fn vector(v: Vec3, what: &str) -> KernelResult<RVector3> {
    let [x, y, z] = [v.x, v.y, v.z].map(|c| exact(c, what));
    let v = RVector3::new(x?, y?, z?);
    if v.is_zero() {
        return Err(KernelError::InvalidInput(format!("the {what} is zero")));
    }
    Ok(v)
}

/// A view direction as an exact vector, scaled so its largest component is one: the
/// isometric view's direction is then exactly (1, -1, 1).
fn direction(v: Vec3) -> KernelResult<RVector3> {
    let v = vector(v, "view direction")?;
    let largest = [&v.x, &v.y, &v.z].into_iter().map(Rational::abs).max().expect("three components");
    let unit = |c: &Rational| c.checked_div(&largest).expect("the largest component is not zero");
    Ok(RVector3::new(unit(&v.x), unit(&v.y), unit(&v.z)))
}

fn edge_id(solid: &Solid, edge: catallax_topology::EdgeId) -> KernelResult<EdgeId> {
    solid.ids.edges.get(&edge).copied().ok_or_else(|| missing("edge"))
}

fn face_id(solid: &Solid, face: catallax_topology::FaceId) -> KernelResult<FaceId> {
    solid.ids.faces.get(&face).copied().ok_or_else(|| missing("face"))
}

/// The face on both sides of a seam.
fn seam_face(model: &Model, edge: catallax_topology::EdgeId) -> KernelResult<catallax_topology::FaceId> {
    model.coedges_of_edge(edge).into_iter().find_map(|c| model.owner_of_coedge(c)).map(|(_, f)| f).ok_or_else(|| {
        KernelError::QueryFailed {
            query: "view_curves".into(),
            code: 0,
            message: "a seam of the view lies on no face".into(),
        }
    })
}

fn missing(what: &str) -> KernelError {
    KernelError::QueryFailed {
        query: "drawing".into(),
        code: 0,
        message: format!("a Catallax {what} of the view has no SolidDraft id"),
    }
}
