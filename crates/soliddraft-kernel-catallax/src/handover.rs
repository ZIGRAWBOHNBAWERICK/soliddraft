//! Built bodies handed from one kernel to another through `catallax_app::handover`.
//!
//! The browser builds in its kernel worker and the page rebuilds the same definitions
//! on its own kernel. The worker keeps each Catallax build it makes and hands them over
//! as bytes; the page is offered them, and a build of the same definition takes the
//! offered body instead of evaluating it again. Definitions are matched by their
//! Catallax recipe with step ids renamed by position, because each kernel numbers its
//! steps from its own sequence.

use std::collections::BTreeMap;
use std::sync::Arc;

use catallax_app::Names;
use catallax_features::Built;
use catallax_numbers::Decider;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use soliddraft_kernel_api::{EdgeId, FaceId, KernelError, KernelResult, Topology, VertexId};

/// The first bytes of a hand-over, then its version.
const MAGIC: &[u8; 16] = b"soliddraft-hand1";

/// The key of a Catallax recipe that does not depend on the kernel that wrote it.
pub(crate) fn key(recipe: &catallax_recipe::Recipe) -> String {
    let mut value = serde_json::to_value(recipe).expect("Catallax recipes serialize");
    let ids: BTreeMap<String, String> = value["steps"]
        .as_array()
        .map(|steps| {
            steps
                .iter()
                .enumerate()
                .filter_map(|(index, step)| Some((step["id"].as_str()?.to_owned(), format!("step{index}"))))
                .collect()
        })
        .unwrap_or_default();
    rename(&mut value, &ids);
    let digest = Sha256::digest(value.to_string().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn rename(value: &mut Value, ids: &BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(id) = ids.get(text.as_str()) {
                *text = id.clone();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| rename(item, ids)),
        Value::Object(object) => object.values_mut().for_each(|item| rename(item, ids)),
        _ => {}
    }
}

/// A body's topology as the kernel that built it computed it, with that kernel's
/// interface ids for each Catallax handle index, so another kernel can name the same
/// entities with its own ids instead of computing the topology again.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HandedTopology {
    pub topology: Topology,
    pub faces: Vec<(u64, u32)>,
    pub edges: Vec<(u64, u32)>,
    pub vertices: Vec<(u64, u32)>,
}

impl HandedTopology {
    /// The topology with each id the handing kernel gave replaced by the id this kernel
    /// gave the same Catallax entity; `None` if any entity has no id here.
    pub(crate) fn renamed(
        &self,
        faces: &BTreeMap<u32, FaceId>,
        edges: &BTreeMap<u32, EdgeId>,
        vertices: &BTreeMap<u32, VertexId>,
    ) -> Option<Topology> {
        fn map<I: Copy>(pairs: &[(u64, u32)], here: &BTreeMap<u32, I>) -> Option<BTreeMap<u64, I>> {
            pairs.iter().map(|(theirs, index)| Some((*theirs, *here.get(index)?))).collect()
        }
        let (f, e, v) = (map(&self.faces, faces)?, map(&self.edges, edges)?, map(&self.vertices, vertices)?);
        let face = |id: FaceId| f.get(&id.raw()).copied();
        let edge = |id: EdgeId| e.get(&id.raw()).copied();
        let vertex = |id: VertexId| v.get(&id.raw()).copied();
        let mut topology = self.topology.clone();
        for info in &mut topology.faces {
            info.id = face(info.id)?;
            info.edges = info.edges.iter().map(|id| edge(*id)).collect::<Option<_>>()?;
        }
        for info in &mut topology.edges {
            info.id = edge(info.id)?;
            info.faces = info.faces.iter().map(|id| face(*id)).collect::<Option<_>>()?;
            info.vertices = info.vertices.iter().map(|id| vertex(*id)).collect::<Option<_>>()?;
        }
        for info in &mut topology.vertices {
            info.id = vertex(info.id)?;
            info.edges = info.edges.iter().map(|id| edge(*id)).collect::<Option<_>>()?;
        }
        Some(topology)
    }
}

/// One build handed over: its recipe key, its Catallax bytes and, when the handing
/// kernel computed it, its topology. A build handed over earlier sends its topology
/// alone once the handing kernel computes it, with no bytes.
pub(crate) struct Handed {
    pub key: String,
    pub body: Option<Vec<u8>>,
    pub topology: Option<HandedTopology>,
}

/// What another kernel handed over for one recipe key.
pub(crate) enum Received {
    /// A build, with its topology if the other kernel had computed it.
    Build(Offer),
    /// The topology of a build handed over earlier.
    Topology(Arc<HandedTopology>),
}

/// A build offered by another kernel. Its bytes are read only when a build takes it:
/// most offers repeat bodies the receiving kernel already holds.
#[derive(Clone)]
pub(crate) struct Offer {
    bytes: Arc<[u8]>,
    built: Arc<std::sync::OnceLock<Result<Arc<Built>, String>>>,
    pub topology: Option<Arc<HandedTopology>>,
}

impl Offer {
    /// Whether the offered build has been read from its bytes yet.
    pub(crate) fn is_read(&self) -> bool {
        self.built.get().is_some()
    }

    /// The offered build, read from its bytes the first time it is asked for.
    pub(crate) fn built(&self) -> KernelResult<Arc<Built>> {
        self.built
            .get_or_init(|| {
                catallax_app::handover::read_body(&self.bytes, &mut Decider::default())
                    .map(Arc::new)
                    .map_err(|refusal| format!("{}: {}", refusal.code, refusal.detail))
            })
            .clone()
            .map_err(KernelError::InvalidInput)
    }
}

/// Builds by key, as bytes another kernel can be offered.
pub(crate) fn encode(builds: &[Handed]) -> KernelResult<Vec<u8>> {
    let mut out = MAGIC.to_vec();
    out.extend((builds.len() as u32).to_le_bytes());
    for handed in builds {
        let topology = serde_json::to_vec(&handed.topology).map_err(|e| KernelError::InvalidInput(e.to_string()))?;
        let body = handed.body.as_deref().unwrap_or_default();
        for part in [handed.key.as_bytes(), body, &topology] {
            out.extend((part.len() as u64).to_le_bytes());
            out.extend(part);
        }
    }
    Ok(out)
}

/// Reads a hand-over back into builds, each restored into its own Catallax stores.
pub(crate) fn decode(bytes: &[u8]) -> KernelResult<Vec<(String, Received)>> {
    let broken = || KernelError::InvalidInput("The hand-over is damaged or from another version.".into());
    let mut rest = bytes.strip_prefix(MAGIC.as_slice()).ok_or_else(broken)?;
    let mut take = |len: usize| -> KernelResult<&[u8]> {
        let (head, tail) = rest.split_at_checked(len).ok_or_else(broken)?;
        rest = tail;
        Ok(head)
    };
    let count = u32::from_le_bytes(take(4)?.try_into().expect("four bytes"));
    let mut builds = Vec::new();
    let mut part = || -> KernelResult<&[u8]> {
        let len = u64::from_le_bytes(take(8)?.try_into().expect("eight bytes"));
        take(usize::try_from(len).map_err(|_| broken())?)
    };
    for _ in 0..count {
        let key = String::from_utf8(part()?.to_vec()).map_err(|_| broken())?;
        let body = part()?;
        let topology: Option<HandedTopology> = serde_json::from_slice(part()?).map_err(|_| broken())?;
        let received = if body.is_empty() {
            Received::Topology(Arc::new(topology.ok_or_else(broken)?))
        } else {
            Received::Build(Offer { bytes: body.into(), built: Arc::default(), topology: topology.map(Arc::new) })
        };
        builds.push((key, received));
    }
    if !rest.is_empty() {
        return Err(broken());
    }
    Ok(builds)
}

/// The bytes of one build, for [`encode`].
pub(crate) fn write(built: &Built) -> KernelResult<Vec<u8>> {
    catallax_app::handover::write_body(built, &mut Decider::default())
        .map_err(|refusal| KernelError::InvalidInput(format!("{}: {}", refusal.code, refusal.detail)))
}

/// The picking names of a handed-over build's result, as Catallax names a build's own.
pub(crate) fn names(built: &Built) -> Names {
    let (model, body) = (&built.model, built.result);
    Names {
        faces: model.faces_of_body(body).into_iter().map(|face| (face, model.face_origins(face).to_vec())).collect(),
        edges: model.edges_of_body(body).into_iter().map(|edge| (edge, model.edge_name(body, edge))).collect(),
        vertices: model
            .vertices_of_body(body)
            .into_iter()
            .map(|vertex| (vertex, model.vertex_name(body, vertex)))
            .collect(),
    }
}
