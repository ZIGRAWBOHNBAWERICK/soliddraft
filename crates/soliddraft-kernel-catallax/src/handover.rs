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
use catallax_recipe::{EdgeName, Origin, VertexName};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use soliddraft_kernel_api::{
    EdgeId, EdgePolyline, FaceId, HistoryRecord, KernelError, KernelResult, Mesh, Topology, VertexId,
};

type CatFace = catallax_topology::FaceId;
type CatEdge = catallax_topology::EdgeId;
type CatVertex = catallax_topology::VertexId;

/// The first bytes of a hand-over, then its version.
const MAGIC: &[u8; 16] = b"soliddraft-hand2";

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
    /// The early display meshes the handing kernel sampled, by display key, with its ids.
    #[serde(default)]
    pub previews: Vec<HandedPreview>,
    /// The checked displays the handing kernel was refused, by display key, with the
    /// reason, so the receiving kernel keeps its preview instead of trying again.
    #[serde(default)]
    pub refused: Vec<(String, String)>,
}

/// One early display mesh and its edges, as [`soliddraft_kernel_api::Kernel::preview`] and
/// `preview_edges` answer for one display key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HandedPreview {
    pub key: String,
    pub mesh: Mesh,
    pub edges: Vec<EdgePolyline>,
}

/// Early display meshes renamed into a kernel's own ids, by display key.
pub(crate) type Previews = BTreeMap<String, (Mesh, Vec<EdgePolyline>)>;

/// Checked displays made here, by display key: the mesh and its edges, or the reason
/// it was refused.
pub(crate) type Checked = BTreeMap<String, Result<(Mesh, Vec<EdgePolyline>), String>>;

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

    /// The handed-over early display meshes, renamed like [`HandedTopology::renamed`];
    /// `None` if any face or edge they name has no id here.
    pub(crate) fn renamed_previews(
        &self,
        faces: &BTreeMap<u32, FaceId>,
        edges: &BTreeMap<u32, EdgeId>,
    ) -> Option<Previews> {
        let face_ids: BTreeMap<u64, FaceId> =
            self.faces.iter().map(|(theirs, index)| Some((*theirs, *faces.get(index)?))).collect::<Option<_>>()?;
        let edge_ids: BTreeMap<u64, EdgeId> =
            self.edges.iter().map(|(theirs, index)| Some((*theirs, *edges.get(index)?))).collect::<Option<_>>()?;
        self.previews
            .iter()
            .map(|preview| {
                let mut mesh = preview.mesh.clone();
                mesh.triangle_faces =
                    mesh.triangle_faces.iter().map(|id| face_ids.get(&id.raw()).copied()).collect::<Option<_>>()?;
                let edges = preview
                    .edges
                    .iter()
                    .map(|polyline| {
                        Some(EdgePolyline {
                            edge: *edge_ids.get(&polyline.edge.raw())?,
                            points: polyline.points.clone(),
                        })
                    })
                    .collect::<Option<_>>()?;
                Some((preview.key.clone(), (mesh, edges)))
            })
            .collect()
    }
}

/// What a kernel needs to adopt a build without reading its bytes, so that an edit
/// reads no exact geometry the user did not ask for.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HandedIdentity {
    /// The handed recipe's step ids, in order.
    pub steps: Vec<String>,
    /// Each face's Catallax handle index, the handing kernel's interface id for it and
    /// its Catallax origins, in the handing kernel's step ids.
    pub faces: Vec<(u32, u64, Vec<Origin>)>,
    pub edges: Vec<(u32, u64, Option<EdgeName>)>,
    pub vertices: Vec<(u32, u64, Option<VertexName>)>,
    /// The handing kernel's history record of the operation, in its interface ids.
    pub record: HistoryRecord,
    /// Whether any face is a B-spline surface.
    pub splines: bool,
    /// Whether the body is open faces rather than a closed solid.
    #[serde(default)]
    pub surface: bool,
    /// The planar faces whose plane has a rational normal and origin.
    pub planes: Vec<HandedPlane>,
}

/// A planar face's exact plane, as rational numbers in text.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HandedPlane {
    pub face: u32,
    pub normal: [String; 3],
    pub origin: [String; 3],
    /// Whether the face's sense agrees with the plane's normal.
    pub same_sense: bool,
}

impl HandedIdentity {
    /// The build's Catallax names, renamed from the handing kernel's step ids as
    /// [`renamed_names`] does. `None` if a handle or a renamed name cannot be made.
    pub(crate) fn names(&self, renames: &BTreeMap<String, String>) -> Option<Names> {
        fn index_of<T>(list: &[(u32, u64, T)]) -> Vec<u32> {
            list.iter().map(|(index, _, _)| *index).collect()
        }
        let (faces, edges, vertices) =
            mint(&index_of(&self.faces), &index_of(&self.edges), &index_of(&self.vertices)).ok()?;
        let names = Names {
            faces: faces
                .into_iter()
                .zip(&self.faces)
                .map(|(handle, (_, _, origins))| (handle, origins.clone()))
                .collect(),
            edges: edges.into_iter().zip(&self.edges).map(|(handle, (_, _, name))| (handle, name.clone())).collect(),
            vertices: vertices
                .into_iter()
                .zip(&self.vertices)
                .map(|(handle, (_, _, name))| (handle, name.clone()))
                .collect(),
        };
        renamed_names(names, renames)
    }

    /// The handing kernel's interface id for each handle index, faces, edges and vertices.
    pub(crate) fn worker_ids(&self) -> (BTreeMap<u32, u64>, BTreeMap<u32, u64>, BTreeMap<u32, u64>) {
        fn ids<T>(list: &[(u32, u64, T)]) -> BTreeMap<u32, u64> {
            list.iter().map(|(index, id, _)| (*index, *id)).collect()
        }
        (ids(&self.faces), ids(&self.edges), ids(&self.vertices))
    }
}

/// Catallax face, edge and vertex handles.
type Handles = (Vec<CatFace>, Vec<CatEdge>, Vec<CatVertex>);

/// Catallax handles for entity handle indices. Catallax makes a handle only while it
/// reads its transfer format, so the indices pass through that format as a small
/// record of their own, with each arena declared unbounded.
fn mint(faces: &[u32], edges: &[u32], vertices: &[u32]) -> Result<Handles, catallax_numbers::Refusal> {
    use catallax_numbers::transfer::{Codec, Reader, Writer};
    struct Indices {
        lists: [Vec<u32>; 3],
        handles: Option<Handles>,
    }
    fn handles<T: 'static>(
        reader: &mut Reader<'_>,
    ) -> Result<Vec<catallax_shapes::Handle<T>>, catallax_numbers::Refusal> {
        reader.arena::<T>(usize::MAX)?;
        (0..reader.count()?).map(|_| catallax_shapes::Handle::<T>::read(reader)).collect()
    }
    impl Codec for Indices {
        fn write(&self, writer: &mut Writer) -> Result<(), catallax_numbers::Refusal> {
            for list in &self.lists {
                list.len().write(writer)?;
                for index in list {
                    index.write(writer)?;
                }
            }
            Ok(())
        }
        fn read(reader: &mut Reader<'_>) -> Result<Self, catallax_numbers::Refusal> {
            let faces = handles::<catallax_topology::Face>(reader)?;
            let edges = handles::<catallax_topology::Edge>(reader)?;
            let vertices = handles::<catallax_topology::Vertex>(reader)?;
            Ok(Indices { lists: Default::default(), handles: Some((faces, edges, vertices)) })
        }
    }
    let indices = Indices { lists: [faces.to_vec(), edges.to_vec(), vertices.to_vec()], handles: None };
    let bytes = catallax_numbers::transfer::encode(&indices, &mut Decider::default())?;
    let read: Indices = catallax_numbers::transfer::decode(&bytes, &mut Decider::default())?;
    Ok(read.handles.expect("a read record holds handles"))
}

/// One build handed over: its recipe key, its Catallax bytes, what adopting it unread
/// needs and, when the handing kernel computed it, its topology. A build handed over
/// earlier sends its topology alone once the handing kernel computes it, with no bytes.
pub(crate) struct Handed {
    pub key: String,
    pub body: Option<Vec<u8>>,
    pub topology: Option<HandedTopology>,
    pub identity: Option<HandedIdentity>,
}

/// What another kernel handed over for one recipe key.
pub(crate) enum Received {
    /// A build, with its topology if the other kernel had computed it.
    Build(Offer),
    /// The topology of a build handed over earlier.
    Topology(Arc<HandedTopology>),
}

/// A build offered by another kernel. Its bytes are read only when a build takes it:
/// most offers repeat bodies the receiving kernel already holds. Its identity is read
/// only then too: reading every offer's identity as the hand-over arrived was most of
/// the browser page's work on a measurement's answer, which offers the whole part
/// again, and on a busy machine it held the page for over a quarter of a second.
#[derive(Clone)]
pub(crate) struct Offer {
    bytes: Arc<[u8]>,
    built: Arc<std::sync::OnceLock<Result<Arc<Built>, String>>>,
    pub topology: Option<Arc<HandedTopology>>,
    /// What adopting the build without reading its bytes needs, as handed, if it was.
    identity_bytes: Option<Arc<[u8]>>,
    /// The identity read from `identity_bytes` the first time it is asked for, or none
    /// if they do not read.
    identity: Arc<std::sync::OnceLock<Option<Arc<HandedIdentity>>>>,
    /// Counts the reads of the receiving kernel's offers.
    pub reads: Arc<std::sync::atomic::AtomicU64>,
    /// The kernel instance that handed the build over, whose ids its identity carries.
    pub source: u64,
}

impl std::fmt::Debug for Offer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Offer").field("bytes", &self.bytes.len()).field("read", &self.is_read()).finish()
    }
}

impl Offer {
    /// Whether the offered build has been read from its bytes yet.
    pub(crate) fn is_read(&self) -> bool {
        self.built.get().is_some()
    }

    /// The offered build, read from its bytes the first time it is asked for.
    pub(crate) fn built(&self) -> KernelResult<Arc<Built>> {
        self.read().clone().map_err(KernelError::InvalidInput)
    }

    /// The offered build or why its bytes did not read, read the first time.
    pub(crate) fn read(&self) -> &Result<Arc<Built>, String> {
        self.built.get_or_init(|| {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            catallax_app::handover::read_body(&self.bytes, &mut Decider::default())
                .map(Arc::new)
                .map_err(|refusal| format!("{}: {}", refusal.code, refusal.detail))
        })
    }

    /// The size of the offered bytes.
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the build came with what adopting it unread needs, without reading it.
    pub(crate) fn has_identity(&self) -> bool {
        self.identity_bytes.is_some()
    }

    /// What adopting the build without reading its bytes needs, read the first time it
    /// is asked for. `None` if it was not handed or does not read: the build is then
    /// adopted from its bytes, as one handed without it is.
    pub(crate) fn identity(&self) -> Option<&Arc<HandedIdentity>> {
        let bytes = self.identity_bytes.as_ref()?;
        self.identity.get_or_init(|| serde_json::from_slice(bytes).ok().map(Arc::new)).as_ref()
    }
}

/// Builds by key, as bytes another kernel can be offered.
pub(crate) fn encode(builds: &[Handed]) -> KernelResult<Vec<u8>> {
    let mut out = MAGIC.to_vec();
    out.extend((builds.len() as u32).to_le_bytes());
    for handed in builds {
        let topology = serde_json::to_vec(&handed.topology).map_err(|e| KernelError::InvalidInput(e.to_string()))?;
        let identity = serde_json::to_vec(&handed.identity).map_err(|e| KernelError::InvalidInput(e.to_string()))?;
        let body = handed.body.as_deref().unwrap_or_default();
        for part in [handed.key.as_bytes(), body, &topology, &identity] {
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
        // An identity is read when a build asks for it; `null` means none was handed.
        let identity = part()?;
        let received = if body.is_empty() {
            Received::Topology(Arc::new(topology.ok_or_else(broken)?))
        } else {
            Received::Build(Offer {
                bytes: body.into(),
                built: Arc::default(),
                topology: topology.map(Arc::new),
                identity_bytes: (identity != b"null").then(|| identity.into()),
                identity: Arc::default(),
                reads: Arc::default(),
                source: 0,
            })
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

/// `names` with every step id in `renames` replaced, including the ids Catallax derives
/// from a step's id, such as a pattern copy's `<step>_copy1`. `None` if a renamed name
/// does not read back.
pub(crate) fn renamed_names(names: Names, renames: &BTreeMap<String, String>) -> Option<Names> {
    if renames.is_empty() {
        return Some(names);
    }
    fn renamed<T: Serialize + serde::de::DeserializeOwned>(value: &T, renames: &BTreeMap<String, String>) -> Option<T> {
        let mut json = serde_json::to_value(value).ok()?;
        rename_steps(&mut json, renames);
        serde_json::from_value(json).ok()
    }
    fn optional<T: Serialize + serde::de::DeserializeOwned>(
        name: &Option<T>,
        renames: &BTreeMap<String, String>,
    ) -> Option<Option<T>> {
        match name {
            Some(name) => renamed(name, renames).map(Some),
            None => Some(None),
        }
    }
    Some(Names {
        faces: names
            .faces
            .iter()
            .map(|(face, origins)| Some((*face, renamed(origins, renames)?)))
            .collect::<Option<_>>()?,
        edges: names.edges.iter().map(|(edge, name)| Some((*edge, optional(name, renames)?))).collect::<Option<_>>()?,
        vertices: names
            .vertices
            .iter()
            .map(|(vertex, name)| Some((*vertex, optional(name, renames)?)))
            .collect::<Option<_>>()?,
    })
}

fn rename_steps(value: &mut Value, renames: &BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(to) = renamed_step(text, renames) {
                *text = to;
            }
        }
        Value::Array(values) => values.iter_mut().for_each(|value| rename_steps(value, renames)),
        Value::Object(fields) => {
            let renamed: serde_json::Map<String, Value> = std::mem::take(fields)
                .into_iter()
                .map(|(key, mut value)| {
                    rename_steps(&mut value, renames);
                    (renamed_step(&key, renames).unwrap_or(key), value)
                })
                .collect();
            *fields = renamed;
        }
        _ => {}
    }
}

/// `text` as a step id in `renames`, or such an id followed by `_` and a suffix.
fn renamed_step(text: &str, renames: &BTreeMap<String, String>) -> Option<String> {
    if let Some(to) = renames.get(text) {
        return Some(to.clone());
    }
    text.match_indices('_').find_map(|(at, _)| Some(format!("{}{}", renames.get(&text[..at])?, &text[at..])))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_id_and_the_ids_derived_from_it_are_renamed() {
        let renames = BTreeMap::from([("op_183".to_owned(), "op_198".to_owned())]);
        assert_eq!(renamed_step("op_183", &renames).as_deref(), Some("op_198"));
        assert_eq!(renamed_step("op_183_copy1", &renames).as_deref(), Some("op_198_copy1"));
        assert_eq!(renamed_step("op_18", &renames), None);
        assert_eq!(renamed_step("op_1834", &renames), None);
        assert_eq!(renamed_step("l0s1", &renames), None);
        let mut value = serde_json::json!({"op_183": ["op_183_scan_a", {"step": "op_183"}], "role": "End"});
        rename_steps(&mut value, &renames);
        assert_eq!(value, serde_json::json!({"op_198": ["op_198_scan_a", {"step": "op_198"}], "role": "End"}));
    }

    /// A hand-over of one build whose identity is `identity`, as [`encode`] lays it out.
    fn one_build(identity: &[u8]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend(1u32.to_le_bytes());
        for part in [b"key".as_slice(), b"body", b"null", identity] {
            out.extend((part.len() as u64).to_le_bytes());
            out.extend(part);
        }
        out
    }

    fn offer(bytes: &[u8]) -> Offer {
        match decode(bytes).unwrap().pop() {
            Some((_, Received::Build(offer))) => offer,
            _ => panic!("one build"),
        }
    }

    #[test]
    fn an_offers_identity_is_read_only_when_a_build_asks_for_it() {
        // Taking the offer reads no identity: these bytes would not have read.
        let damaged = offer(&one_build(b"{\"steps\":"));
        assert!(damaged.has_identity() && damaged.identity.get().is_none());
        // Asked for, it does not read, so the build is adopted from its bytes instead.
        assert!(damaged.identity().is_none());
        let none = offer(&one_build(b"null"));
        assert!(!none.has_identity() && none.identity().is_none());
    }
}
