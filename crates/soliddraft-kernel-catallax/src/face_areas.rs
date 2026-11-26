//! Proven topology areas. A retained face's picking identity only finds a candidate;
//! it does not establish that the same proof applies to the current trim.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};

use catallax_measure::{Accuracy, Enclosure};
use catallax_numbers::{Answer, Real, Refusal};
use catallax_shapes::{Curve, PointDef, SplineSupport, Surface};
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
        for other in &self.ancestors {
            let old = model(other);
            let previous = other.areas.0.lock().unwrap_or_else(|e| e.into_inner());
            for (&old_face, old_area) in previous.iter() {
                if other.ids.faces[&old_face] == own_id
                    && let Some(reads) = &old_area.reads
                    && current.unchanged_where_read(old, reads)
                    && same_face(old, old_face, current, face)
                {
                    areas.insert(face, Proven { area: old_area.area, reads: None });
                    return Ok(old_area.area);
                }
            }
        }
        // No arena-handle equality across independent builds. The rational
        // straight-boundary path depends only on the plane and directed walks.
        if current.face_chart_cover(face).is_none()
            && matches!(current.geom().surface(current.face(face).surface), Surface::Plane(_))
        {
            let origins = &self.solid.ids.face_origins[&own_id];
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
fn same_face(a: &Model, af: FaceId, b: &Model, bf: FaceId) -> bool {
    trims(a, af, b, bf)
        && match (a.face_chart_cover(af), b.face_chart_cover(bf)) {
            (None, None) => true,
            (Some(a_cover), Some(b_cover)) => {
                a_cover.len() == b_cover.len() && a_cover.iter().zip(b_cover).all(|(&x, &y)| trims(a, x, b, y))
            }
            _ => false,
        }
}

fn trims(a: &Model, af: FaceId, b: &Model, bf: FaceId) -> bool {
    let (left, right) = (a.face(af), b.face(bf));
    if left.surface != right.surface || a.face_origins(af) != b.face_origins(bf) {
        return false;
    }
    let mut edges = Vec::new();
    let mut vertices = Vec::new();
    let mut vertex = |x: VertexId, y: VertexId| a.vertex(x) == b.vertex(y) && pair(&mut vertices, x, y);
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
                && pair(&mut edges, x, y)
                && (x == y
                    || !matches!(a.geom().curve(e.curve), Curve::BSpline(_))
                    || fitted_rail(a, x) == fitted_rail(b, y))
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
    if a.face_chart_cover(af).is_some() || b.face_chart_cover(bf).is_some() {
        return false;
    }
    let (Surface::Plane(p), Surface::Plane(q)) =
        (a.geom().surface(a.face(af).surface), b.geom().surface(b.face(bf).surface))
    else {
        return false;
    };
    rational_equal(
        [&p.origin.x, &p.origin.y, &p.origin.z, &p.normal.x, &p.normal.y, &p.normal.z],
        [&q.origin.x, &q.origin.y, &q.origin.z, &q.normal.x, &q.normal.y, &q.normal.z],
    ) && a.face(af).loops.iter().all(|l| matches!(a.loop_(*l), Loop::Coedges(_)))
        && walks(
            a,
            af,
            b,
            bf,
            |x, y| {
                let (x, y) = (a.edge(x), b.edge(y));
                x.start.is_some()
                    && x.end.is_some()
                    && matches!(a.geom().curve(x.curve), Curve::Line(_))
                    && matches!(b.geom().curve(y.curve), Curve::Line(_))
            },
            |x, y| {
                let (PointDef::Explicit(x), PointDef::Explicit(y)) =
                    (a.geom().point(a.vertex(x).point), b.geom().point(b.vertex(y).point))
                else {
                    return false;
                };
                rational_equal([&x.x, &x.y, &x.z], [&y.x, &y.y, &y.z])
            },
        )
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
