//! A solid's faces at Catallax's three nested display levels: the point counts a
//! display budget reads before any mesh is made, and the mesh at the levels it picks.
//!
//! One plan answers both. Catallax samples the shared edges and refines every face
//! once, keeps the result, and counts each face's points at each level without building
//! a vertex or index buffer. The mesh is then assembled from the kept plan, so counting
//! first costs no second pass.

use std::collections::BTreeMap;

use catallax_app::Error;
use catallax_mesh::{FaceLevelPlan, MeshOptions};
use catallax_numbers::{Decider, OperationSpan, Refusal};
use soliddraft_kernel_api::{BodyId, FaceLevel, FaceLevelPoints, KernelError, KernelResult, Mesh, TessellationParams};

use crate::request::RequestPolicy;
use crate::{CatallaxKernel, Solid, convert, options, query_error, survives_f32};

const QUERY: &str = "face_levels";

/// Picks a level for each counted face, in the order of the counts, or declines the mesh.
pub(crate) type Choose<'a> = &'a mut dyn FnMut(&[FaceLevelPoints]) -> Option<Vec<FaceLevel>>;

fn native(level: FaceLevel) -> catallax_mesh::FaceLevel {
    match level {
        FaceLevel::Coarse => catallax_mesh::FaceLevel::Preview,
        FaceLevel::Standard => catallax_mesh::FaceLevel::Bounded,
        FaceLevel::Fine => catallax_mesh::FaceLevel::Proven,
    }
}

fn failed(message: &str) -> KernelError {
    KernelError::QueryFailed { query: QUERY.into(), code: -1, message: message.into() }
}

/// The counts of `body` at `params` and, when `choose` is given and picks levels from
/// them, the mesh at those levels. `None` for a body with no nested levels: a field, a
/// numerical body, or a request with an edge-length control, which the adapter checks on
/// a finished mesh.
pub(crate) fn planned(
    kernel: &CatallaxKernel,
    body: BodyId,
    params: &TessellationParams,
    choose: Option<Choose<'_>>,
) -> KernelResult<Option<(Vec<FaceLevelPoints>, Option<Mesh>)>> {
    if kernel.field(body)?.is_some() || kernel.approximate(body)?.is_some() {
        return Ok(None);
    }
    let solid = kernel.solid(body)?;
    let options = options(params)?;
    if options.max_edge_length.is_some() {
        return Ok(None);
    }
    let (result, work) = RequestPolicy::Display.run(|| {
        let mut decider = Decider::default();
        let span = OperationSpan::begin("brep", &QUERY, QUERY, &decider);
        let result = run(&solid, &options, params, choose, &mut decider);
        span.finish(&decider, result.as_ref().err(), None);
        result
    });
    if work.exhausted_here() {
        let limit = Error::ResourceLimit;
        return Err(query_error(QUERY, Refusal::new(limit.code(), limit.to_string())));
    }
    result.map_err(|refusal| query_error(QUERY, refusal))?.map(Some)
}

/// A kernel refusal outside, an interface error inside.
type Planned = Result<KernelResult<(Vec<FaceLevelPoints>, Option<Mesh>)>, Refusal>;

fn run(
    solid: &Solid,
    options: &MeshOptions,
    params: &TessellationParams,
    choose: Option<Choose<'_>>,
    decider: &mut Decider,
) -> Planned {
    let built = solid.app.checkpoint.built();
    let faces = built.model.faces_of_body(built.result);
    let plan = catallax_mesh::plan_face_levels(&built.model, built.result, options, decider)?;
    let counts = match counts(&plan, &faces, solid) {
        Ok(counts) => counts,
        Err(error) => return Ok(Err(error)),
    };
    let Some(chosen) = choose.and_then(|choose| choose(&counts)) else { return Ok(Ok((counts, None))) };
    if chosen.len() != counts.len() {
        return Ok(Err(KernelError::InvalidInput("a face level mesh takes one level per counted face".into())));
    }
    if chosen.iter().all(|level| *level == FaceLevel::Fine) {
        let mesh = plan.emit()?.value;
        return Ok(survives_f32(&mesh, options).map(|()| (counts, Some(convert::mesh(&mesh, &solid.ids, params)))));
    }
    let levels: BTreeMap<_, _> = faces.iter().copied().zip(chosen.into_iter().map(native)).collect();
    let mesh = plan.emit_selected(|face| levels.get(&face).copied().unwrap_or(catallax_mesh::FaceLevel::Proven))?;
    if mesh.positions.iter().flatten().any(|v| !(*v as f32).is_finite()) {
        return Ok(Err(KernelError::Unsupported(
            "preview coordinates beyond f32 display range is not available in the Catallax application entry".into(),
        )));
    }
    Ok(Ok((counts, Some(convert::preview(&mesh, &solid.ids)))))
}

/// Each face's points per level, in topology order.
fn counts(
    plan: &FaceLevelPlan<'_>,
    faces: &[catallax_topology::FaceId],
    solid: &Solid,
) -> KernelResult<Vec<FaceLevelPoints>> {
    faces
        .iter()
        .map(|face| {
            let id = *solid.ids.faces.get(face).ok_or_else(|| failed("A planned face has no interface identity"))?;
            let at = |level: FaceLevel| {
                plan.point_count(*face, native(level))
                    .and_then(|points| u32::try_from(points).ok())
                    .ok_or_else(|| failed("A face of the body has no planned point count"))
            };
            Ok(FaceLevelPoints {
                face: id,
                points: [at(FaceLevel::Coarse)?, at(FaceLevel::Standard)?, at(FaceLevel::Fine)?],
            })
        })
        .collect()
}
