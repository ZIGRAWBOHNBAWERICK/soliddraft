//! Proven topology areas. A retained face's picking identity only finds a candidate;
//! it does not establish that the same proof applies to the current trim.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};

use catallax_measure::{Accuracy, Enclosure};
use catallax_numbers::{Answer, Real, Refusal};
use catallax_shapes::{BSplineCurve, Curve, PointDef, SplineBoundary, SplineSupport, Surface};
use catallax_topology::{EdgeId, FaceId, Loop, Model, Reads, VertexId};

use super::{Solid, convert::midpoint, numerical};

type Area = Result<f64, Refusal>;

struct Proven {
    area: f64,
    // A hit from a predecessor can answer the other topology view of this
    // body, but only the original proof can be reused by a later checkpoint.
    reads: Option<Reads>,
}

#[derive(Default)]
pub(super) struct Areas(Mutex<BTreeMap<FaceId, Proven>>);

/// The most recent bodies are candidates for independently rebuilt planar faces.
/// Do not keep a released body's model or meshes alive to serve another edit.
#[derive(Default)]
pub(super) struct Recent(Mutex<VecDeque<Weak<Solid>>>);

pub(super) struct Query<'a> {
    solid: &'a Solid,
    ancestors: Vec<Arc<Solid>>,
    recent: Vec<Arc<Solid>>,
}

impl Recent {
    pub(super) fn query<'a>(&self, solid: &'a Arc<Solid>) -> Query<'a> {
        const RECENT: usize = 8;
        let mut ancestors = Vec::new();
        let mut prefix = solid.area_prefix.upgrade();
        while let Some(parent) = prefix {
            prefix = parent.area_prefix.upgrade();
            ancestors.push(parent);
        }
        let mut history = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let recent =
            history.iter().rev().filter_map(Weak::upgrade).filter(|other| !Arc::ptr_eq(solid, other)).collect();
        let current = Arc::downgrade(solid);
        history.retain(|body| body.strong_count() != 0 && !Weak::ptr_eq(body, &current));
        if history.len() == RECENT {
            history.pop_front();
        }
        history.push_back(current);
        Query { solid, ancestors, recent }
    }
}

fn model(body: &Solid) -> &Model {
    &body.app.checkpoint.built().model
}

fn prove(model: &Model, face: FaceId, accuracy: &Accuracy) -> Result<Answer<Enclosure>, Refusal> {
    numerical("face_area", |dec| catallax_measure::face_area(model, face, accuracy, dec))
}

pub(super) fn measure(model: &Model, face: FaceId, accuracy: &Accuracy, cache: Option<&Query<'_>>) -> Area {
    match cache {
        Some(cache) => cache.area(model, face, accuracy),
        None => prove(model, face, accuracy).map(|answer| midpoint(&answer.value)),
    }
}

impl Query<'_> {
    fn area(&self, current: &Model, face: FaceId, accuracy: &Accuracy) -> Area {
        let mut areas = self.solid.areas.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(area) = areas.get(&face) {
            return Ok(area.area);
        }
        let own_id = self.solid.ids.faces[&face];
        let origins = &self.solid.ids.face_origins[&own_id];
        for other in &self.ancestors {
            let old = model(other);
            let previous = other.areas.0.lock().unwrap_or_else(|e| e.into_inner());
            for (&old_face, old_area) in previous.iter() {
                let old_id = other.ids.faces[&old_face];
                if &other.ids.face_origins[&old_id] == origins
                    && same_face(old, old_face, current, face)
                    && (old_id == own_id
                        && old_area.reads.as_ref().is_some_and(|reads| current.unchanged_where_read(old, reads))
                        || local_area(old, old_face) && local_area(current, face))
                {
                    areas.insert(face, Proven { area: old_area.area, reads: None });
                    return Ok(old_area.area);
                }
            }
        }
        // Only the planar measure paths below are compared across independent
        // builds, where equal numeric handles provide no evidence.
        if matches!(current.geom().surface(current.face(face).surface), Surface::Plane(_)) {
            for other in &self.recent {
                let old = model(other);
                let Ok(previous) = other.areas.0.try_lock() else { continue };
                for (&old_face, old_area) in previous.iter() {
                    if &other.ids.face_origins[&other.ids.faces[&old_face]] == origins
                        && same_rational_polygon(old, old_face, current, face)
                    {
                        areas.insert(face, Proven { area: old_area.area, reads: None });
                        return Ok(old_area.area);
                    }
                }
            }
        }
        let mut watched = current.clone();
        watched.watch();
        let answer = prove(&watched, face, accuracy)?;
        let area = midpoint(&answer.value);
        // A best-effort enclosure at the proof cap is not reusable.
        if !answer.marks.iter().any(|mark| mark.code == "measure.accuracy-not-reached") {
            areas.insert(face, Proven { area, reads: Some(watched.watched()) });
        }
        Ok(area)
    }
}

/// Shared-prefix geometry handles name immutable definitions. Topology handles
/// can change even when sewing restores exactly the same boundary.
fn compare_cover(
    a: &Model,
    af: FaceId,
    b: &Model,
    bf: FaceId,
    mut same_trim: impl FnMut(FaceId, FaceId) -> bool,
) -> bool {
    // The integrator groups edge uses across the whole cover, not just one chart.
    same_trim(af, bf)
        && match (a.face_chart_cover(af), b.face_chart_cover(bf)) {
            (None, None) => true,
            (Some(a_cover), Some(b_cover)) => {
                let mut faces = Vec::new();
                a_cover.len() == b_cover.len()
                    && a_cover.iter().zip(b_cover).all(|(&x, &y)| pair(&mut faces, x, y) && same_trim(x, y))
            }
            _ => false,
        }
}

fn same_face(a: &Model, af: FaceId, b: &Model, bf: FaceId) -> bool {
    let mut edges = Vec::new();
    let mut vertices = Vec::new();
    compare_cover(a, af, b, bf, |x, y| trims(a, x, b, y, &mut edges, &mut vertices))
}

// For a plane with explicit vertices and native lines or splines, and for a
// checked rational patch, Catallax's area engine only needs the face and
// ordered physical trims (including roles and the fitted-rail predicate).
// New coedge uses on untouched edges need not invalidate those proofs.
fn local_area(model: &Model, face: FaceId) -> bool {
    let matches = |face| {
        let value = model.face(face);
        value.loops.iter().all(|id| match model.loop_(*id) {
            Loop::Vertex(_) => false,
            Loop::Coedges(coedges) => coedges.iter().all(|id| {
                let edge = model.edge(model.coedge(*id).edge);
                matches!(model.geom().curve(edge.curve), Curve::Line(_) | Curve::BSpline(_))
                    && [edge.start, edge.end]
                        .into_iter()
                        .flatten()
                        .all(|v| matches!(model.geom().point(model.vertex(v).point), PointDef::Explicit(_)))
            }),
        })
    };
    let surface = |face| model.geom().surface(model.face(face).surface);
    matches(face)
        && match model.face_chart_cover(face) {
            Some(cover) => cover.iter().all(|&cell| {
                matches!(surface(cell), Surface::Plane(_) | Surface::BSpline(SplineSupport::RationalPatch(_)))
                    && matches(cell)
            }),
            None => matches!(surface(face), Surface::Plane(_) | Surface::BSpline(SplineSupport::RationalPatch(_))),
        }
}

fn trims(
    a: &Model,
    af: FaceId,
    b: &Model,
    bf: FaceId,
    edges: &mut Vec<(EdgeId, EdgeId)>,
    vertices: &mut Vec<(VertexId, VertexId)>,
) -> bool {
    let (left, right) = (a.face(af), b.face(bf));
    if left.surface != right.surface || a.face_origins(af) != b.face_origins(bf) {
        return false;
    }
    let mut vertex = |x: VertexId, y: VertexId| a.vertex(x) == b.vertex(y) && pair(vertices, x, y);
    walks(
        a,
        af,
        b,
        bf,
        |x, y| {
            let (e, f) = (a.edge(x), b.edge(y));
            e.curve == f.curve
                && e.sense == f.sense
                && e.tolerance == f.tolerance
                && pair(edges, x, y)
                && (!matches!(a.geom().curve(e.curve), Curve::BSpline(_)) || fitted_rail(a, x) == fitted_rail(b, y))
        },
        &mut vertex,
    )
}

fn pair<T: Copy + Eq>(seen: &mut Vec<(T, T)>, left: T, right: T) -> bool {
    if let Some(&(a, b)) = seen.iter().find(|&&(a, b)| a == left || b == right) {
        return a == left && b == right;
    }
    seen.push((left, right));
    true
}

// A remade B-spline edge can have new owners even when its curve did not
// change. Area refuses fitted rails before attempting any chart integration.
fn fitted_rail(model: &Model, edge: EdgeId) -> bool {
    model.coedges_of_edge(edge).into_iter().any(|coedge| {
        model.owner_of_coedge(coedge).is_some_and(|(_, face)| {
            matches!(
                model.geom().surface(model.face(face).surface),
                Surface::BSpline(
                    SplineSupport::RollingEnvelope(_)
                        | SplineSupport::RollingSaddle(_)
                        | SplineSupport::RollingSphere(_)
                )
            )
        })
    })
}

fn rational_equal<const N: usize>(a: [&Real; N], b: [&Real; N]) -> bool {
    a.into_iter().zip(b).all(|(x, y)| x.rational_ref().is_some_and(|v| Some(v) == y.rational_ref()))
}

fn same_rational_polygon(a: &Model, af: FaceId, b: &Model, bf: FaceId) -> bool {
    let mut edges = Vec::new();
    let mut vertices = Vec::new();
    let mut points = Vec::new();
    compare_cover(a, af, b, bf, |af, bf| {
        let (Surface::Plane(p), Surface::Plane(q)) =
            (a.geom().surface(a.face(af).surface), b.geom().surface(b.face(bf).surface))
        else {
            return false;
        };
        if !a.face_origins(af).iter().map(|o| &o.role).eq(b.face_origins(bf).iter().map(|o| &o.role)) {
            return false;
        }
        rational_equal(
            [
                &p.origin.x,
                &p.origin.y,
                &p.origin.z,
                &p.normal.x,
                &p.normal.y,
                &p.normal.z,
                &p.ref_dir.x,
                &p.ref_dir.y,
                &p.ref_dir.z,
            ],
            [
                &q.origin.x,
                &q.origin.y,
                &q.origin.z,
                &q.normal.x,
                &q.normal.y,
                &q.normal.z,
                &q.ref_dir.x,
                &q.ref_dir.y,
                &q.ref_dir.z,
            ],
        ) && a.face(af).loops.iter().all(|l| matches!(a.loop_(*l), Loop::Coedges(_)))
            && walks(
                a,
                af,
                b,
                bf,
                |xi, yi| {
                    let (x, y) = (a.edge(xi), b.edge(yi));
                    x.start.is_some()
                        && x.end.is_some()
                        && x.sense == y.sense
                        && x.tolerance == y.tolerance
                        && pair(&mut edges, xi, yi)
                        && match (a.geom().curve(x.curve), b.geom().curve(y.curve)) {
                            // Catallax's rational planar path only uses the endpoints.
                            (Curve::Line(_), Curve::Line(_)) => true,
                            (Curve::BSpline(left), Curve::BSpline(right)) => {
                                straight_patch_row(a, xi, left)
                                    && straight_patch_row(b, yi, right)
                                    && left.same_definition(right)
                                    && !fitted_rail(a, xi)
                                    && !fitted_rail(b, yi)
                            }
                            _ => false,
                        }
                },
                |x, y| {
                    if !pair(&mut vertices, x, y) {
                        return false;
                    }
                    let (x, y) = (a.vertex(x), b.vertex(y));
                    if x.tolerance != y.tolerance || !pair(&mut points, x.point, y.point) {
                        return false;
                    }
                    let (PointDef::Explicit(x), PointDef::Explicit(y)) =
                        (a.geom().point(x.point), b.geom().point(y.point))
                    else {
                        return false;
                    };
                    rational_equal([&x.x, &x.y, &x.z], [&y.x, &y.y, &y.z])
                },
            )
    })
}

/// Independent sweeps and lofts can rebuild the same planar cap on degree-one
/// rational patch rows. The owning patch identifies the native curve view
/// (not a transported/reduced curve). Constant rational weights throughout
/// that carrier keep the row's trace representation independent of other rows.
fn straight_patch_row(model: &Model, edge: EdgeId, curve: &BSplineCurve) -> bool {
    curve.degree() == 1
        && curve.control_count() == 2
        && curve.has_rational_definition()
        && model.coedges_of_edge(edge).into_iter().any(|coedge| {
            let Some((_, owner)) = model.owner_of_coedge(coedge) else { return false };
            let Surface::BSpline(SplineSupport::RationalPatch(patch)) = model.geom().surface(model.face(owner).surface)
            else {
                return false;
            };
            let carrier = patch.carrier();
            [SplineBoundary::VMin, SplineBoundary::VMax, SplineBoundary::UMin, SplineBoundary::UMax]
                .into_iter()
                .any(|side| curve.is_boundary_of(carrier, side))
                && carrier
                    .weights()
                    .iter()
                    .all(|w| w.rational_ref().is_some_and(|weight| Some(weight) == curve.weight(0).rational_ref()))
        })
}

fn walks(
    a: &Model,
    af: FaceId,
    b: &Model,
    bf: FaceId,
    mut edge: impl FnMut(EdgeId, EdgeId) -> bool,
    mut vertex: impl FnMut(VertexId, VertexId) -> bool,
) -> bool {
    let (left, right) = (a.face(af), b.face(bf));
    left.sense == right.sense
        && left.tolerance == right.tolerance
        && left.loops.len() == right.loops.len()
        && left.loops.iter().zip(&right.loops).all(|(&x, &y)| match (a.loop_(x), b.loop_(y)) {
            (Loop::Vertex(x), Loop::Vertex(y)) => vertex(*x, *y),
            (Loop::Coedges(x), Loop::Coedges(y)) => {
                x.len() == y.len()
                    && x.iter().zip(y).all(|(&u, &v)| {
                        let (u, v) = (a.coedge(u), b.coedge(v));
                        if u.sense != v.sense || !edge(u.edge, v.edge) {
                            return false;
                        }
                        let (x, y) = (a.edge(u.edge), b.edge(v.edge));
                        [x.start, x.end].into_iter().zip([y.start, y.end]).all(|(x, y)| match (x, y) {
                            (Some(x), Some(y)) => vertex(x, y),
                            (None, None) => true,
                            _ => false,
                        })
                    })
            }
            _ => false,
        })
}
