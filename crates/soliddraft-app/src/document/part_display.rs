//! One revision's part display: every live body's display mesh, the names of its faces
//! and edges, and the pick table that numbers them.
//!
//! The proof worker and the document's refreshes make a display the same way. Each body
//! is prepared once: its topology, rendering, edge samples and names. The bodies are then
//! combined in a fixed order, the display body first and the others in body-key order,
//! and pick numbers are given out in that order. The document puts the result on screen
//! whole through `Document::publish`, so a pick number always means a face, edge or
//! corner of the mesh on screen.

use super::{
    DisplayMesh, FeatureId, NamedEdge, NamedFace, PickEntry, app_id, display_mesh, edge_polylines, gpu_display_mesh,
    named_entities, screen_mesh,
};
use crate::viewport::gpu_preview::{self, Meshing};
use soliddraft_kernel_api::{BodyId, EdgePolyline, FaceId, Mesh, TessellationParams, Topology};
use soliddraft_model as model;
use std::collections::HashMap;
use std::sync::Arc;

/// What the document shows for one revision: each body's display mesh, the names of
/// the bodies' faces and edges, and the pick table the meshes' pick numbers index.
#[derive(Clone, Debug)]
pub(super) struct PartDisplay {
    /// The display body: the last live body, whose levels the display quality follows.
    pub(super) body: BodyId,
    /// The display body's tessellation parameters.
    pub(super) params: TessellationParams,
    pub(super) meshes: HashMap<BodyId, Arc<DisplayMesh>>,
    /// Pick id (index + 1) to entity, over every mesh in `meshes`.
    pub(super) picks: Vec<PickEntry>,
    pub(super) faces: Vec<NamedFace>,
    pub(super) edges: Vec<NamedEdge>,
}

/// A part display made from a model, ready to publish, with what an early display and
/// the display quality also need.
#[derive(Clone, Debug)]
pub(crate) struct ReadyDisplay {
    pub(super) part: PartDisplay,
    /// Every body's display mesh in one, for a display shown before the document adopts
    /// it. With one body it is that body's mesh.
    pub mesh: Arc<DisplayMesh>,
    /// Every body's faces, edges and vertices, the display body's first. Its box is the
    /// display body's.
    pub(super) topology: Topology,
    /// The display body's kernel mesh, which the display quality keeps for its level.
    pub(super) kernel_mesh: Mesh,
}

/// One body's display geometry and names, before its picks are numbered.
struct BodyDisplay {
    body: BodyId,
    /// The feature the body's picks belong to.
    feature: FeatureId,
    params: TessellationParams,
    topology: Topology,
    kernel_mesh: Mesh,
    polylines: Vec<EdgePolyline>,
    /// The body's GPU preview, drawn instead of `kernel_mesh`.
    gpu: Option<Meshing>,
    faces: Vec<NamedFace>,
    edges: Vec<NamedEdge>,
}

impl BodyDisplay {
    /// Queries, renders and names `body` at `level` of its display levels. A fallback
    /// takes the kernel's CPU preview instead of the screen's usual mesh.
    fn prepare(
        model: &model::Document,
        body: BodyId,
        feature: FeatureId,
        level: soliddraft_tessellation::Level,
        curved_surfaces: Option<crate::graphics::CurvedSurfaces>,
        fallback: bool,
    ) -> Result<Self, String> {
        let kernel = model.kernel();
        let topology = kernel.topology(body).map_err(|error| error.to_string())?;
        let params =
            *crate::graphics::body_mesh_levels(curved_surfaces, &model.settings().viewport, &topology).params(level);
        let (kernel_mesh, polylines) = if fallback {
            let mesh = kernel.preview(body, &params).map_err(|error| error.to_string())?;
            (mesh, kernel.preview_edges(body, &params).unwrap_or_default())
        } else {
            let mesh = screen_mesh(kernel.as_ref(), body, &params).map_err(|error| error.to_string())?;
            (mesh, edge_polylines(kernel.as_ref(), body, &params))
        };
        let (faces, edges) = named_entities(model, body, &topology);
        Ok(Self { body, feature, params, topology, kernel_mesh, polylines, gpu: None, faces, edges })
    }

    /// The body's display mesh, its faces, edges and corners numbered after `picks`.
    fn display_mesh(&mut self, picks: &mut Vec<PickEntry>) -> DisplayMesh {
        let makers: HashMap<FaceId, FeatureId> = self.faces.iter().map(|face| (face.id, face.feature)).collect();
        match self.gpu.take() {
            Some(meshing) => gpu_display_mesh(self.feature, self.body, meshing, &self.topology, &makers, picks),
            None => display_mesh(self.feature, &self.kernel_mesh, &self.polylines, &self.topology, &makers, picks),
        }
    }
}

/// The display of every live body of `model` at `level`, or `None` when it has no body.
pub(crate) fn ready_display(
    model: &model::Document,
    level: soliddraft_tessellation::Level,
    curved_surfaces: Option<crate::graphics::CurvedSurfaces>,
) -> Result<Option<ReadyDisplay>, String> {
    display_of(model, level, curved_surfaces, false)
}

/// Every live body's CPU preview, for a part whose checked display failed: a built
/// body is always drawn.
pub(crate) fn ready_fallback(
    model: &model::Document,
    level: soliddraft_tessellation::Level,
    curved_surfaces: Option<crate::graphics::CurvedSurfaces>,
) -> Result<Option<ReadyDisplay>, String> {
    display_of(model, level, curved_surfaces, true)
}

fn display_of(
    model: &model::Document,
    level: soliddraft_tessellation::Level,
    curved_surfaces: Option<crate::graphics::CurvedSurfaces>,
    fallback: bool,
) -> Result<Option<ReadyDisplay>, String> {
    let bodies = model.live_bodies();
    let Some((_, body)) = bodies.last().copied() else { return Ok(None) };
    let feature = display_feature(model, body);
    let mut prepared = vec![BodyDisplay::prepare(model, body, feature, level, curved_surfaces, fallback)?];
    for (key, other) in bodies.into_iter().filter(|(_, other)| *other != body) {
        let feature = body_feature(model, key, other);
        prepared.push(BodyDisplay::prepare(model, other, feature, level, curved_surfaces, fallback)?);
    }
    Ok(Some(combine(prepared)))
}

/// The GPU preview of `model`'s one live body at `level`, shown while its checked
/// display is made. `None` without a GPU preview, for a part of several bodies, or for
/// a body the kernel cannot describe.
pub(crate) fn ready_preview(
    model: &model::Document,
    level: soliddraft_tessellation::Level,
    curved_surfaces: Option<crate::graphics::CurvedSurfaces>,
) -> Result<Option<ReadyDisplay>, String> {
    if gpu_preview::active().is_none() {
        return Ok(None);
    }
    let Some(target) = preview_target(model, level, curved_surfaces)? else { return Ok(None) };
    let Some(meshing) = gpu_preview::mesh_body(model.kernel().as_ref(), target.body, &target.params) else {
        return Ok(None);
    };
    Ok(Some(preview_display(model, target, meshing)))
}

/// What the browser's page needs to mesh `model`'s one live body on its GPU: its body,
/// feature, topology and display parameters at `level`. `None` for a part of several
/// bodies or none.
pub(crate) struct PreviewTarget {
    pub body: BodyId,
    feature: FeatureId,
    topology: Topology,
    pub params: TessellationParams,
}

pub(crate) fn preview_target(
    model: &model::Document,
    level: soliddraft_tessellation::Level,
    curved_surfaces: Option<crate::graphics::CurvedSurfaces>,
) -> Result<Option<PreviewTarget>, String> {
    let bodies = model.live_bodies();
    let [(_, body)] = bodies.as_slice() else { return Ok(None) };
    let topology = model.kernel().topology(*body).map_err(|error| error.to_string())?;
    let params =
        *crate::graphics::body_mesh_levels(curved_surfaces, &model.settings().viewport, &topology).params(level);
    Ok(Some(PreviewTarget { body: *body, feature: display_feature(model, *body), topology, params }))
}

/// The display of a GPU preview the page meshed for `target`.
pub(crate) fn preview_display(
    model: &model::Document,
    target: PreviewTarget,
    meshing: gpu_preview::Meshing,
) -> ReadyDisplay {
    let (faces, edges) = named_entities(model, target.body, &target.topology);
    combine(vec![BodyDisplay {
        body: target.body,
        feature: target.feature,
        params: target.params,
        topology: target.topology,
        kernel_mesh: Mesh::default(),
        polylines: Vec::new(),
        gpu: Some(meshing),
        faces,
        edges,
    }])
}

/// The display body's picks belong to the last feature that built it and is neither
/// suppressed nor rolled back.
fn display_feature(model: &model::Document, body: BodyId) -> FeatureId {
    model
        .features()
        .iter()
        .rev()
        .find(|feature| {
            !matches!(
                model.status(feature.id),
                Some(model::FeatureStatus::Suppressed | model::FeatureStatus::RolledBack)
            ) && model.body_after(feature.id) == Some(body)
        })
        .map(|feature| app_id(feature.id))
        .unwrap_or(0)
}

/// Another body's picks belong to the last feature of its own key that built it.
fn body_feature(model: &model::Document, key: u32, body: BodyId) -> FeatureId {
    model
        .features()
        .iter()
        .rev()
        .find(|feature| feature.body == key && model.body_after(feature.id) == Some(body))
        .map_or(0, |feature| app_id(feature.id))
}

/// Numbers the prepared bodies' picks in order, the first body's first, and joins their
/// meshes, names and topology into one display.
fn combine(prepared: Vec<BodyDisplay>) -> ReadyDisplay {
    let mut prepared = prepared.into_iter();
    let mut first = prepared.next().expect("a display has a body");
    let mut picks = Vec::new();
    let mesh = Arc::new(first.display_mesh(&mut picks));
    let BodyDisplay { body, params, topology, kernel_mesh, faces, edges, .. } = first;
    let part = PartDisplay { body, params, meshes: HashMap::from([(body, mesh.clone())]), picks, faces, edges };
    let mut display = ReadyDisplay { part, mesh, topology, kernel_mesh };
    let mut aggregate: Option<DisplayMesh> = None;
    for mut other in prepared {
        let aggregate = aggregate.get_or_insert_with(|| (*display.mesh).clone());
        let mesh = other.display_mesh(&mut display.part.picks);
        let offset = aggregate.positions.len() as u32;
        aggregate.positions.extend_from_slice(&mesh.positions);
        aggregate.normals.extend_from_slice(&mesh.normals);
        aggregate.face_picks.extend_from_slice(&mesh.face_picks);
        aggregate.face_features.extend_from_slice(&mesh.face_features);
        aggregate.face_appearances.extend_from_slice(&mesh.face_appearances);
        aggregate.indices.extend(mesh.indices.iter().map(|i| i + offset));
        aggregate.edges.extend_from_slice(&mesh.edges);
        aggregate.corners.extend_from_slice(&mesh.corners);
        display.part.meshes.insert(other.body, Arc::new(mesh));
        display.part.faces.extend(other.faces);
        display.part.edges.extend(other.edges);
        display.topology.faces.extend(other.topology.faces);
        display.topology.edges.extend(other.topology.edges);
        display.topology.vertices.extend(other.topology.vertices);
    }
    if let Some(aggregate) = aggregate {
        display.mesh = Arc::new(aggregate);
    }
    display
}
