//! Persistent references: naming faces and edges by where they came from.
//!
//! Kernel ids change on every rebuild, so a feature never stores one. It stores a name
//! made of the feature that generated the face, the part of that feature that made it
//! (its [`Generator`]: a cap, the side swept from a given sketch entity, the blend of a
//! given edge) and an ordinal among faces with the same generator. An edge is named by
//! the two faces it separates.
//!
//! The name stands for an origin. On each rebuild the generating feature says which
//! kernel origin (operation tag and role) each of its names stands for, and the kernel
//! reports every face's origins: the pieces of a split face all keep the origin, and a
//! face merged from several carries all of theirs. A name resolves to the faces that
//! carry its origin, so it survives renames, splits, merges and dimension edits without
//! following history or comparing geometry.
//!
//! The result is checked against a geometric fingerprint taken when the reference was
//! made: the surface or curve type must not change, and a planar face must still face
//! the same way. Shared generator edges also require the edge and both support faces
//! to retain their recorded fingerprints. Other references use position and size only
//! to rank the candidates offered when a reference breaks.
//!
//! A reference that resolves to no entity, to several, or to one that fails the check
//! does not guess. The feature stops with a [`RepairNeeded`] that says which reference
//! broke, why, and which entities could replace it.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::{
    CurveKind, EdgeId, EdgeInfo, FaceCarrier, FaceId, FaceInfo, HolePart, Origin, SurfaceKind, Topology, Vec3,
};
use soliddraft_sketch::EntityId;

use crate::feature::FeatureId;

/// The part of a feature that made a face.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Generator {
    /// An extrusion's cap in the sketch plane.
    StartCap,
    /// An extrusion's cap at its far end.
    EndCap,
    /// A side swept from a sketch entity.
    Side {
        /// The sketch line, arc or circle.
        entity: EntityId,
        /// The saved path piece that carried it, when a sweep's path has several.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<Box<str>>,
    },
    /// A fillet or chamfer face.
    Blend {
        /// Index of the edge in the feature's edge list.
        edge: u32,
    },
    /// A face of a hole.
    Hole {
        /// Index of the hole in the feature's point list.
        placement: u32,
        /// Which face of the hole.
        part: HolePart,
    },
    /// A face of a pattern's or mirror's copy. Faces of one copy are ranked by the
    /// names of the faces they copy.
    Copy {
        /// Which copy: 1 and up for a pattern, 1 for a mirror image.
        instance: u32,
    },
    /// Any other face a feature made.
    Other,
    /// A face from one immutable imported source. A replacement needs new references.
    Imported {
        /// Hash of the source that generated this face.
        sha256: Box<str>,
    },
    /// An inset wall of a shell, ranked by its source face name.
    ShellWall,
    /// A rounded face where a shell's wall turns round a concave edge, ranked by the
    /// names of the edge's faces. A shell makes it; it is not a fillet.
    ShellRound,
    /// A spherical face where a shell's wall turns round a concave vertex, ranked by
    /// the names of the vertex's faces.
    ShellCorner,
    /// An offset image of a face, ranked by its source face name.
    OffsetFace,
    /// A round where offset images meet at an edge.
    OffsetRound,
    /// A round where offset images meet at a vertex.
    OffsetCorner,
    /// A closing wall along a thickened face's boundary.
    ThickenRim,
    /// A profile end left exposed at a join inside a sweep path.
    PathJoin {
        /// The saved path piece at that join.
        path: Box<str>,
    },
}

impl fmt::Display for Generator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Generator::StartCap => f.write_str("Start cap"),
            Generator::EndCap => f.write_str("End cap"),
            Generator::Side { entity, path: None } => write!(f, "Side of {entity}"),
            Generator::Side { entity, path: Some(path) } => write!(f, "Side of {entity} along {path}"),
            Generator::Blend { edge } => write!(f, "Blend of edge {}", edge + 1),
            Generator::Hole { placement, part } => write!(f, "Hole {} {}", placement + 1, part.label().to_lowercase()),
            Generator::Copy { instance } => write!(f, "Copy {instance} face"),
            Generator::Other | Generator::Imported { .. } => f.write_str("Face"),
            Generator::OffsetFace => f.write_str("Offset face"),
            Generator::OffsetRound => f.write_str("Offset round"),
            Generator::OffsetCorner => f.write_str("Offset corner"),
            Generator::ThickenRim => f.write_str("Thicken rim"),
            Generator::ShellWall => f.write_str("Inner wall"),
            Generator::ShellRound => f.write_str("Inner round"),
            Generator::ShellCorner => f.write_str("Inner corner"),
            Generator::PathJoin { path } => write!(f, "Path join at {path}"),
        }
    }
}

/// A name a feature defined during a rebuild, with the kernel origin it stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedFace {
    /// The operation and role the kernel reports on the faces this name finds. Valid in
    /// this rebuild only: operation tags are the document's, but a rebuild may rename them.
    pub origin: Origin,
    /// What made it.
    pub generator: Generator,
    /// Its rank among the feature's faces with the same generator.
    pub ordinal: u32,
}

/// What must stay true of a referenced face.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceFingerprint {
    /// Surface type; must not change.
    pub surface: SurfaceKind,
    /// Outward normal of a planar face; must not change direction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<Vec3>,
    /// Area centroid when the reference was made; ranks candidates.
    pub centroid: Vec3,
    /// Area when the reference was made; ranks candidates.
    pub area: f64,
}

impl FaceFingerprint {
    /// The fingerprint of a face as it is now.
    pub fn of(face: &FaceInfo) -> Self {
        FaceFingerprint { surface: face.surface, normal: face.normal, centroid: face.centroid, area: face.area }
    }

    /// Why `face` is not the face this fingerprint describes, if it is not.
    fn mismatch(&self, face: &FaceInfo) -> Option<String> {
        self.changed(face.surface, face.normal)
    }

    fn changed(&self, surface: SurfaceKind, normal: Option<Vec3>) -> Option<String> {
        if surface != self.surface {
            return Some(format!("the face became {:?} instead of {:?}", surface, self.surface));
        }
        match (self.normal, normal) {
            (Some(a), Some(b)) if a.dot(b) < 1.0 - 1e-6 => {
                Some(format!("the face now faces {:?} instead of {:?}", round3(b), round3(a)))
            }
            _ => None,
        }
    }

    fn score(&self, face: &FaceInfo) -> f64 {
        let kind = if face.surface == self.surface { 0.0 } else { 1e6 };
        kind + face.centroid.distance(self.centroid) + (face.area - self.area).abs().sqrt()
    }
}

/// What must stay true of a referenced edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeFingerprint {
    /// Curve type; must not change.
    pub curve: CurveKind,
    /// Midpoint when the reference was made; ranks candidates.
    pub midpoint: Vec3,
    /// Length when the reference was made; ranks candidates.
    pub length: f64,
}

impl EdgeFingerprint {
    /// The fingerprint of an edge as it is now.
    pub fn of(edge: &EdgeInfo) -> Self {
        EdgeFingerprint { curve: edge.curve, midpoint: edge.midpoint, length: edge.length }
    }

    fn score(&self, edge: &EdgeInfo) -> f64 {
        let kind = if edge.curve == self.curve { 0.0 } else { 1e6 };
        kind + edge.midpoint.distance(self.midpoint) + (edge.length - self.length).abs()
    }
}

fn round3(v: Vec3) -> [f64; 3] {
    let r = |x: f64| (x * 1000.0).round() / 1000.0 + 0.0;
    [r(v.x), r(v.y), r(v.z)]
}

/// A face, named by origin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceRef {
    /// The feature that generated it.
    pub feature: FeatureId,
    /// The part of the feature that made it.
    pub generator: Generator,
    /// Rank among the feature's faces with the same generator.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ordinal: u32,
    /// The check.
    pub fingerprint: FaceFingerprint,
    /// What the face was called when the reference was made, such as
    /// "Boss-Extrude1 · End cap", for repair prompts. Resolution never reads it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}

/// An edge, named by the two faces it separates (the same face twice for a seam).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeRef {
    /// The faces on either side.
    pub faces: [FaceRef; 2],
    /// The check.
    pub fingerprint: EdgeFingerprint,
    /// Made among several generator edges sharing these faces. Such references require
    /// unchanged fingerprints even when only one candidate remains.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disambiguate: bool,
    /// What the edge was called when the reference was made, for repair prompts.
    /// Resolution never reads it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

/// A vertex, named by the faces that meet at it. A vertex is where its faces meet, so it
/// follows them through edits; the position is kept for repair messages only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VertexRef {
    /// Every face around the vertex, in the order the kernel listed them when it was named.
    pub faces: Vec<FaceRef>,
    /// Where the vertex was when the reference was made. Resolution never reads it.
    pub position: Vec3,
    /// What the vertex was called when the reference was made, for repair prompts.
    /// Resolution never reads it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

/// Which reference of a feature broke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "slot", rename_all = "snake_case")]
pub enum ReferenceSlot {
    /// A sketch's plane face.
    SketchPlane,
    /// One of a fillet's or chamfer's edges.
    Edge {
        /// Index in the feature's edge list.
        index: u32,
    },
    /// One of a draft's faces.
    Face {
        /// Index in the feature's face list.
        index: u32,
    },
    /// The face a hole starts on, or the face of a mirror's or draft's plane.
    Plane,
    /// The face an extrusion goes up to.
    Limit,
}

impl fmt::Display for ReferenceSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReferenceSlot::SketchPlane => f.write_str("sketch plane"),
            ReferenceSlot::Edge { index } => write!(f, "edge {}", index + 1),
            ReferenceSlot::Face { index } => write!(f, "face {}", index + 1),
            ReferenceSlot::Plane => f.write_str("plane"),
            ReferenceSlot::Limit => f.write_str("end face"),
        }
    }
}

/// Why a reference broke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum RepairReason {
    /// Nothing in the body matches the name any more.
    Missing,
    /// Several entities match; the reference does not choose among them.
    Ambiguous,
    /// One entity matches but its geometry fails the fingerprint check.
    Mismatch {
        /// What changed.
        detail: String,
    },
}

/// An entity a repair could point a reference at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Candidate {
    /// A face of the feature's input body.
    Face(FaceId),
    /// An edge of the feature's input body.
    Edge(EdgeId),
}

/// A reference that did not resolve to exactly one matching entity. The feature that
/// holds it is not built until the reference is repaired.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepairNeeded {
    /// Which reference.
    pub slot: ReferenceSlot,
    /// Why.
    pub reason: RepairReason,
    /// The problem, for people.
    pub message: String,
    /// Entities that could replace it, best match first: the several matches when the
    /// name is ambiguous, the nearest by fingerprint otherwise.
    pub candidates: Vec<Candidate>,
}

/// Why an entity could not be given a persistent name.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum NamingError {
    /// The entity is not in any body of the document's current rebuild.
    #[error("{0} is not part of the model")]
    NotInModel(String),
    /// No feature defines a name for any origin the kernel reports on the entity.
    #[error("{0} has no origin in the feature tree")]
    NoOrigin(String),
    /// The name would match other entities too, for example a piece of a face that a
    /// later feature split.
    #[error("{0} cannot be named uniquely: {1}")]
    NotUnique(String, String),
}

/// The names each feature defined in the current rebuild.
pub(crate) type NameMap<'a> = HashMap<FeatureId, &'a [NamedFace]>;

/// A body as references see it.
pub(crate) struct View<'a> {
    pub topology: &'a Topology,
}

/// A reference that did not resolve.
#[derive(Debug)]
pub(crate) struct Unresolved {
    pub reason: RepairReason,
    pub message: String,
    pub candidates: Vec<Candidate>,
}

impl Unresolved {
    pub fn into_repair(self, slot: ReferenceSlot) -> RepairNeeded {
        RepairNeeded { slot, reason: self.reason, message: self.message, candidates: self.candidates }
    }
}

const CANDIDATES: usize = 5;

/// How far apart, in millimetres, two planar pieces of one origin may lie and still
/// count as one sketch plane. The pieces of a split face share their surface exactly;
/// this only absorbs the display mesh's round-off in their centroids.
const SAME_PLANE: f64 = 1e-6;

/// The faces of the body that carry an origin, in topology order.
fn carriers<'a>(origin: &'a Origin, view: &'a View) -> impl Iterator<Item = FaceId> + 'a {
    view.topology.faces.iter().filter(move |f| f.origins.contains(origin)).map(|f| f.id)
}

fn ranked_faces(fp: &FaceFingerprint, faces: impl Iterator<Item = FaceId>, view: &View) -> Vec<Candidate> {
    let mut scored: Vec<(f64, FaceId)> =
        faces.filter_map(|id| view.topology.face(id).map(|f| (fp.score(f), id))).collect();
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    scored.into_iter().take(CANDIDATES).map(|(_, id)| Candidate::Face(id)).collect()
}

/// The current faces a face reference names, in topology order. Never empty.
fn face_set(r: &FaceRef, view: &View, names: &NameMap) -> Result<Vec<FaceId>, Unresolved> {
    let missing = |message: String| Unresolved {
        reason: RepairReason::Missing,
        message,
        candidates: ranked_faces(&r.fingerprint, view.topology.faces.iter().map(|f| f.id), view),
    };
    let Some(made) = names.get(&r.feature) else {
        return Err(missing(format!("{} is not built before this feature", r.feature)));
    };
    let Some(seed) = made.iter().find(|n| n.generator == r.generator && n.ordinal == r.ordinal) else {
        return Err(missing(format!("{} no longer makes its {} #{}", r.feature, r.generator, r.ordinal + 1)));
    };
    let present: Vec<FaceId> = carriers(&seed.origin, view).collect();
    if present.is_empty() {
        return Err(missing(format!("the {} of {} is gone", r.generator, r.feature)));
    }
    Ok(present)
}

/// The one face a reference names.
pub(crate) fn resolve_face(r: &FaceRef, view: &View, names: &NameMap) -> Result<FaceId, Unresolved> {
    let set = face_set(r, view, names)?;
    if set.len() > 1 {
        return Err(Unresolved {
            reason: RepairReason::Ambiguous,
            message: format!("the {} of {} was split into {} faces", r.generator, r.feature, set.len()),
            candidates: ranked_faces(&r.fingerprint, set.into_iter(), view),
        });
    }
    let face = view.topology.face(set[0]).expect("face_set returns faces of the topology");
    if let Some(detail) = r.fingerprint.mismatch(face) {
        return Err(Unresolved {
            reason: RepairReason::Mismatch { detail: detail.clone() },
            message: format!("the {} of {}: {detail}", r.generator, r.feature),
            candidates: vec![Candidate::Face(face.id)],
        });
    }
    Ok(face.id)
}

/// The plane a sketch-plane reference names: the one face it names, or the pieces of a
/// split planar face when they all lie in one plane, since any of them gives the same
/// sketch plane. Returns the first such face in topology order.
pub(crate) fn resolve_plane(r: &FaceRef, view: &View, names: &NameMap) -> Result<FaceId, Unresolved> {
    let set = face_set(r, view, names)?;
    let face = |id: FaceId| view.topology.face(id).expect("face_set returns faces of the topology");
    let first = face(set[0]);
    let mismatch = set.iter().find_map(|id| r.fingerprint.mismatch(face(*id)));
    let coplanar = first.normal.is_some_and(|n| {
        set.iter().all(|id| {
            let f = face(*id);
            f.normal == Some(n) && (f.centroid - first.centroid).dot(n).abs() <= SAME_PLANE
        })
    });
    match mismatch {
        Some(detail) => Err(Unresolved {
            reason: RepairReason::Mismatch { detail: detail.clone() },
            message: format!("the {} of {}: {detail}", r.generator, r.feature),
            candidates: ranked_faces(&r.fingerprint, set.into_iter(), view),
        }),
        None if set.len() > 1 && !coplanar => Err(Unresolved {
            reason: RepairReason::Ambiguous,
            message: format!(
                "the {} of {} was split into {} faces that do not share a plane",
                r.generator,
                r.feature,
                set.len()
            ),
            candidates: ranked_faces(&r.fingerprint, set.into_iter(), view),
        }),
        None => Ok(first.id),
    }
}

/// The normal and a point of the plane [`resolve_plane`] would find, from exact carriers
/// alone. `None` leaves the answer to `resolve_plane` and the display topology: a broken
/// reference ranks its repair candidates by area and centroid, and only an axis-aligned
/// plane clear of the origin has an offset that cannot depend on the display centroid.
/// There the topology's snap of that centroid onto the plane cancels exactly, and a
/// tilted plane keeps its round-off, or a plane through the origin its sign of zero.
pub(crate) fn carrier_plane(r: &FaceRef, carriers: &[FaceCarrier], names: &NameMap) -> Option<(Vec3, Vec3)> {
    let seed = names.get(&r.feature)?.iter().find(|n| n.generator == r.generator && n.ordinal == r.ordinal)?;
    let mut pieces = carriers.iter().filter(|f| f.origins.contains(&seed.origin));
    let first = pieces.next()?;
    let (normal, on_plane) = (first.normal?, first.on_plane?);
    let offset = on_plane.dot(normal);
    let axis_aligned = [normal.x, normal.y, normal.z].iter().filter(|v| **v == 0.0).count() == 2;
    let unchanged = |f: &FaceCarrier| r.fingerprint.changed(f.surface, f.normal).is_none();
    let same_plane = |f: &FaceCarrier| f.normal == Some(normal) && f.on_plane.is_some_and(|p| p.dot(normal) == offset);
    let clean = axis_aligned && offset.abs() > SAME_PLANE && unchanged(first);
    (clean && pieces.all(|f| unchanged(f) && same_plane(f))).then_some((normal, on_plane))
}

/// Every current face a face reference names, for an assignment that follows a face
/// through later features, such as an appearance: every piece of a split face matches.
/// Only a missing face, or one whose surface type or facing changed, breaks it.
pub(crate) fn resolve_faces(r: &FaceRef, view: &View, names: &NameMap) -> Result<Vec<FaceId>, Unresolved> {
    let set = face_set(r, view, names)?;
    let face = |id: &FaceId| view.topology.face(*id).expect("face_set returns faces of the topology");
    let fits: Vec<FaceId> = set.iter().copied().filter(|id| r.fingerprint.mismatch(face(id)).is_none()).collect();
    if fits.is_empty() {
        let detail = r.fingerprint.mismatch(face(&set[0])).unwrap_or_default();
        return Err(Unresolved {
            reason: RepairReason::Mismatch { detail: detail.clone() },
            message: format!("the {} of {}: {detail}", r.generator, r.feature),
            candidates: ranked_faces(&r.fingerprint, set.into_iter(), view),
        });
    }
    Ok(fits)
}

/// Every current face that carries an origin of a feature's names, in topology order.
pub(crate) fn faces_of_feature(feature: FeatureId, view: &View, names: &NameMap) -> Vec<FaceId> {
    let Some(made) = names.get(&feature) else { return Vec::new() };
    let origins: Vec<&Origin> = made.iter().map(|n| &n.origin).collect();
    view.topology.faces.iter().filter(|f| f.origins.iter().any(|o| origins.contains(&o))).map(|f| f.id).collect()
}

fn ranked_edges(fp: &EdgeFingerprint, edges: impl Iterator<Item = EdgeId>, view: &View) -> Vec<Candidate> {
    let mut scored: Vec<(f64, EdgeId)> =
        edges.filter_map(|id| view.topology.edge(id).map(|e| (fp.score(e), id))).collect();
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    scored.into_iter().take(CANDIDATES).map(|(_, id)| Candidate::Edge(id)).collect()
}

/// The one edge a reference names: the edge between a face carrying the first name's
/// origin and a face carrying the second's. Faces may have been split or merged, as long
/// as one edge remains, or one edge and both support faces retain their fingerprints.
pub(crate) fn resolve_edge(r: &EdgeRef, view: &View, names: &NameMap) -> Result<EdgeId, Unresolved> {
    let all_edges = || view.topology.edges.iter().map(|e| e.id);
    let sides = [face_set(&r.faces[0], view, names), face_set(&r.faces[1], view, names)];
    let mut sets = Vec::new();
    for (side, fr) in sides.into_iter().zip(&r.faces) {
        let set = side.map_err(|u| Unresolved {
            candidates: ranked_edges(&r.fingerprint, all_edges(), view),
            message: format!("its face: {}", u.message),
            ..u
        })?;
        let fits: Vec<FaceId> = set
            .iter()
            .copied()
            .filter(|id| fr.fingerprint.mismatch(view.topology.face(*id).unwrap()).is_none())
            .collect();
        if fits.is_empty() {
            let detail = fr.fingerprint.mismatch(view.topology.face(set[0]).unwrap()).unwrap_or_default();
            return Err(Unresolved {
                reason: RepairReason::Mismatch { detail: detail.clone() },
                message: format!("its face, the {} of {}: {detail}", fr.generator, fr.feature),
                candidates: ranked_edges(&r.fingerprint, all_edges(), view),
            });
        }
        sets.push(fits);
    }
    let (a, b) = (&sets[0], &sets[1]);
    let between = |e: &&EdgeInfo| match e.faces.as_slice() {
        [p, q] => (a.contains(p) && b.contains(q)) || (a.contains(q) && b.contains(p)),
        _ => false,
    };
    let found: Vec<&EdgeInfo> = view.topology.edges.iter().filter(between).collect();
    let label = format!(
        "{} of {} and {} of {}",
        r.faces[0].generator, r.faces[0].feature, r.faces[1].generator, r.faces[1].feature
    );
    if r.disambiguate && !found.is_empty() {
        let same_face = |id: FaceId, side: usize| {
            sets[side].contains(&id)
                && FaceFingerprint::of(view.topology.face(id).unwrap()) == r.faces[side].fingerprint
        };
        let unchanged: Vec<_> = found
            .iter()
            .filter(|edge| {
                EdgeFingerprint::of(edge) == r.fingerprint
                    && match edge.faces.as_slice() {
                        [p, q] => (same_face(*p, 0) && same_face(*q, 1)) || (same_face(*q, 0) && same_face(*p, 1)),
                        _ => false,
                    }
            })
            .collect();
        if let [edge] = unchanged.as_slice() {
            return Ok(edge.id);
        }
        return Err(Unresolved {
            reason: RepairReason::Ambiguous,
            message: "The selected edge or its adjacent faces changed. Pick the edge again.".into(),
            candidates: ranked_edges(&r.fingerprint, found.iter().map(|edge| edge.id), view),
        });
    }
    match found.as_slice() {
        [] => Err(Unresolved {
            reason: RepairReason::Missing,
            message: format!("no edge is left between the {label}"),
            candidates: ranked_edges(&r.fingerprint, all_edges(), view),
        }),
        [e] if e.curve != r.fingerprint.curve => {
            let detail = format!("the edge became {:?} instead of {:?}", e.curve, r.fingerprint.curve);
            Err(Unresolved {
                reason: RepairReason::Mismatch { detail: detail.clone() },
                message: format!("the edge between the {label}: {detail}"),
                candidates: vec![Candidate::Edge(e.id)],
            })
        }
        [e] => Ok(e.id),
        many => Err(Unresolved {
            reason: RepairReason::Ambiguous,
            message: format!("{} edges lie between the {label}", many.len()),
            candidates: ranked_edges(&r.fingerprint, many.iter().map(|edge| edge.id), view),
        }),
    }
}

/// The origin-based name of a face, without checking that it is unique. A face merged
/// from several carries several names; the earliest feature's comes first.
fn origin_name(face: FaceId, view: &View, names: &NameMap) -> Result<FaceRef, NamingError> {
    let info = view.topology.face(face).ok_or_else(|| NamingError::NotInModel(face.to_string()))?;
    let mut ids: Vec<&FeatureId> = names.keys().collect();
    ids.sort();
    for id in ids {
        if let Some(n) = names[id].iter().find(|n| info.origins.contains(&n.origin)) {
            return Ok(FaceRef {
                feature: *id,
                generator: n.generator.clone(),
                ordinal: n.ordinal,
                fingerprint: FaceFingerprint::of(info),
                label: String::new(),
            });
        }
    }
    Err(NamingError::NoOrigin(face.to_string()))
}

/// The names of an edge's two faces from the construction pair the kernel reports. A
/// merged face carries several origins, and only the pair says which one this edge
/// bounds; [`origin_name`] would take the earliest feature's. Each name keeps the
/// fingerprint of the incident face that carries its origin, whatever the pair's order.
fn construction_names(info: &EdgeInfo, view: &View, names: &NameMap) -> Option<[FaceRef; 2]> {
    let pair = info.construction_origins.as_ref()?;
    let [a, b] = info.faces.as_slice() else { return None };
    let (a, b) = (view.topology.face(*a)?, view.topology.face(*b)?);
    let incident = if a.origins.contains(&pair[0]) && b.origins.contains(&pair[1]) {
        [a, b]
    } else if b.origins.contains(&pair[0]) && a.origins.contains(&pair[1]) {
        [b, a]
    } else {
        return None;
    };
    let named = |side: usize| {
        names.iter().find_map(|(feature, made)| {
            made.iter().find(|n| n.origin == pair[side]).map(|n| FaceRef {
                feature: *feature,
                generator: n.generator.clone(),
                ordinal: n.ordinal,
                fingerprint: FaceFingerprint::of(incident[side]),
                label: String::new(),
            })
        })
    };
    Some([named(0)?, named(1)?])
}

/// A persistent name for a face of the body, checked to resolve back to that face.
pub(crate) fn name_face(face: FaceId, view: &View, names: &NameMap) -> Result<FaceRef, NamingError> {
    let r = origin_name(face, view, names)?;
    match resolve_face(&r, view, names) {
        Ok(back) if back == face => Ok(r),
        Ok(other) => Err(NamingError::NotUnique(face.to_string(), format!("the name finds {other}"))),
        Err(u) => Err(NamingError::NotUnique(face.to_string(), u.message)),
    }
}

/// A persistent name for a planar face to sketch on, checked to resolve back to that
/// face's plane. A piece of a split face can carry a sketch, since all the pieces of
/// its origin share one plane; [`name_face`] refuses it.
pub(crate) fn name_plane(face: FaceId, view: &View, names: &NameMap) -> Result<FaceRef, NamingError> {
    let r = origin_name(face, view, names)?;
    let plane_of = |id: FaceId| view.topology.face(id).and_then(|f| f.normal.map(|n| (n, f.centroid.dot(n))));
    let (Some((n, d)), Ok(back)) = (plane_of(face), resolve_plane(&r, view, names)) else {
        let why =
            if plane_of(face).is_none() { "it is not planar".to_owned() } else { "its name finds no plane".into() };
        return Err(NamingError::NotUnique(face.to_string(), why));
    };
    match plane_of(back) {
        Some((m, e)) if m == n && (e - d).abs() <= SAME_PLANE => Ok(r),
        _ => Err(NamingError::NotUnique(face.to_string(), format!("the name finds {back}"))),
    }
}

/// A persistent name for an edge of the body, checked to resolve back to that edge.
pub(crate) fn name_edge(edge: EdgeId, view: &View, names: &NameMap) -> Result<EdgeRef, NamingError> {
    let info = view.topology.edge(edge).ok_or_else(|| NamingError::NotInModel(edge.to_string()))?;
    let [a, b] = info.faces.as_slice() else {
        return Err(NamingError::NotUnique(edge.to_string(), format!("it bounds {} faces", info.faces.len())));
    };
    let faces = match construction_names(info, view, names) {
        Some(faces) => faces,
        None => [origin_name(*a, view, names)?, origin_name(*b, view, names)?],
    };
    let mut r = EdgeRef { disambiguate: false, faces, fingerprint: EdgeFingerprint::of(info), label: String::new() };
    let mut resolved = resolve_edge(&r, view, names);
    let supports = [r.faces[0].fingerprint.surface, r.faces[1].fingerprint.surface];
    if info.curve == CurveKind::Line
        && matches!(
            supports,
            [SurfaceKind::Cylinder, SurfaceKind::Cylinder]
                | [SurfaceKind::Cylinder, SurfaceKind::Plane]
                | [SurfaceKind::Plane, SurfaceKind::Cylinder]
        )
        && matches!(&resolved, Err(Unresolved { reason: RepairReason::Ambiguous, .. }))
    {
        r.disambiguate = true;
        resolved = resolve_edge(&r, view, names);
    }
    match resolved {
        Ok(back) if back == edge => Ok(r),
        Ok(other) => Err(NamingError::NotUnique(edge.to_string(), format!("the name finds {other}"))),
        Err(u) => Err(NamingError::NotUnique(edge.to_string(), u.message)),
    }
}

/// The generator and ordinal of a face a feature made, for labels.
pub(crate) fn origin_label(face: FaceId, view: &View, names: &NameMap) -> Option<(FeatureId, Generator, u32)> {
    origin_name(face, view, names).ok().map(|r| (r.feature, r.generator, r.ordinal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::{Aabb, FaceRole, OpTag};

    fn origin(tag: &str, role: FaceRole) -> Origin {
        Origin { tag: OpTag::new(tag), role }
    }

    fn face(id: u64, z: f64, origins: Vec<Origin>) -> FaceInfo {
        FaceInfo {
            id: FaceId::from_raw(id),
            surface: SurfaceKind::Plane,
            area: 1.0,
            centroid: Vec3::new(id as f64, 0.0, z),
            bbox: Aabb { min: Vec3::ZERO, max: Vec3::ZERO },
            edges: Vec::new(),
            normal: Some(Vec3::Z),
            origins,
        }
    }

    fn topology(faces: Vec<FaceInfo>) -> Topology {
        Topology {
            kind: soliddraft_kernel_api::BodyKind::Solid,
            faces,
            edges: Vec::new(),
            vertex_count: 0,
            vertices: Vec::new(),
            bbox: Aabb { min: Vec3::ZERO, max: Vec3::new(1.0, 1.0, 1.0) },
            display_chord_height: None,
        }
    }

    fn carrier(id: u64, z: f64, origins: Vec<Origin>) -> FaceCarrier {
        FaceCarrier {
            id: FaceId::from_raw(id),
            surface: SurfaceKind::Plane,
            normal: Some(Vec3::Z),
            on_plane: Some(Vec3::new(3.0, 4.0, z)),
            origins,
        }
    }

    #[test]
    fn carriers_answer_only_clean_planes_and_leave_the_rest_to_the_topology() {
        let cap = origin("F2.r0.sweep", FaceRole::EndCap);
        let named = [NamedFace { origin: cap.clone(), generator: Generator::EndCap, ordinal: 0 }];
        let names: NameMap = [(FeatureId(2), &named[..])].into();
        let r = FaceRef {
            feature: FeatureId(2),
            generator: Generator::EndCap,
            ordinal: 0,
            fingerprint: FaceFingerprint::of(&face(1, 10.0, vec![])),
            label: String::new(),
        };
        let plane = |faces: &[FaceCarrier]| carrier_plane(&r, faces, &names);
        let whole = carrier(1, 10.0, vec![cap.clone()]);
        assert_eq!(plane(std::slice::from_ref(&whole)), whole.normal.zip(whole.on_plane));
        // The pieces of a split face give one plane, whichever comes first.
        let piece = carrier(2, 10.0, vec![cap.clone()]);
        assert_eq!(plane(&[whole.clone(), piece.clone()]), whole.normal.zip(whole.on_plane));
        assert_eq!(plane(&[piece.clone(), whole.clone()]), piece.normal.zip(piece.on_plane));
        // Each of these needs a display area or centroid, or may take its bits from one.
        let refused = [
            ("the origin is gone", vec![carrier(1, 10.0, vec![])]),
            ("pieces lie in two planes", vec![whole.clone(), carrier(2, 12.0, vec![cap.clone()])]),
            ("a plane through the origin", vec![carrier(1, 0.0, vec![cap.clone()])]),
            ("the face turned over", vec![FaceCarrier { normal: Some(-Vec3::Z), ..whole.clone() }]),
            ("the face is curved", vec![FaceCarrier { normal: None, on_plane: None, ..whole.clone() }]),
            ("the surface changed", vec![FaceCarrier { surface: SurfaceKind::Cylinder, ..whole.clone() }]),
            ("a tilted plane", vec![FaceCarrier { normal: Some(Vec3::new(0.0, 0.6, 0.8)), ..whole.clone() }]),
        ];
        for (why, faces) in refused {
            assert_eq!(plane(&faces), None, "{why}");
        }
    }

    #[test]
    fn names_resolve_by_origin_through_splits_and_merges() {
        let plate = origin("F2.r0.sweep", FaceRole::EndCap);
        let tab = origin("F4.r0.sweep", FaceRole::EndCap);
        let named = [NamedFace { origin: plate.clone(), generator: Generator::EndCap, ordinal: 0 }];
        let tab_named = [NamedFace { origin: tab.clone(), generator: Generator::EndCap, ordinal: 0 }];
        let names: NameMap = [(FeatureId(2), &named[..]), (FeatureId(4), &tab_named[..])].into();
        // The plate's top merged with the tab's: one face with both origins.
        let merged = topology(vec![face(1, 10.0, vec![plate.clone(), tab.clone()]), face(2, 0.0, vec![])]);
        let view = View { topology: &merged };
        let r = name_face(FaceId::from_raw(1), &view, &names).unwrap();
        assert_eq!((r.feature, &r.generator), (FeatureId(2), &Generator::EndCap), "the earliest feature names it");
        assert_eq!(resolve_face(&r, &view, &names).unwrap(), FaceId::from_raw(1));
        // A later cut split it into two coplanar pieces: one sketch plane, two faces.
        let split = topology(vec![face(3, 10.0, vec![plate.clone()]), face(4, 10.0, vec![plate.clone(), tab])]);
        let view = View { topology: &split };
        assert_eq!(resolve_plane(&r, &view, &names).unwrap(), FaceId::from_raw(3));
        assert!(matches!(resolve_face(&r, &view, &names).unwrap_err().reason, RepairReason::Ambiguous));
        assert_eq!(resolve_faces(&r, &view, &names).unwrap().len(), 2);
        // Pieces in different planes are not one sketch plane.
        let apart = topology(vec![face(3, 10.0, vec![plate.clone()]), face(4, 12.0, vec![plate])]);
        let view = View { topology: &apart };
        assert!(matches!(resolve_plane(&r, &view, &names).unwrap_err().reason, RepairReason::Ambiguous));
        // Nothing carries the origin any more.
        let gone = topology(vec![face(5, 10.0, vec![])]);
        let view = View { topology: &gone };
        assert!(matches!(resolve_plane(&r, &view, &names).unwrap_err().reason, RepairReason::Missing));
    }

    #[test]
    fn shared_face_pair_names_an_unchanged_generator_but_never_guesses_after_a_move() {
        let a = origin("F2", FaceRole::Other);
        let b = origin("F4", FaceRole::Other);
        let a_names = [NamedFace { origin: a.clone(), generator: Generator::Other, ordinal: 0 }];
        let b_names = [NamedFace { origin: b.clone(), generator: Generator::Other, ordinal: 0 }];
        let names: NameMap = [(FeatureId(2), &a_names[..]), (FeatureId(4), &b_names[..])].into();
        let mut sides = vec![face(1, 6.0, vec![a]), face(2, 6.0, vec![b])];
        sides[0].surface = SurfaceKind::Cylinder;
        sides[0].normal = None;
        sides[1].normal = Some(Vec3::Y);
        let mut body = topology(sides);
        body.edges = [-4.0, 4.0]
            .into_iter()
            .enumerate()
            .map(|(index, x)| EdgeInfo {
                id: EdgeId::from_raw(index as u64 + 1),
                curve: CurveKind::Line,
                length: 12.0,
                midpoint: Vec3::new(x, 3.0, 6.0),
                bbox: Aabb { min: Vec3::new(x, 3.0, 0.0), max: Vec3::new(x, 3.0, 12.0) },
                faces: vec![FaceId::from_raw(1), FaceId::from_raw(2)],
                construction_origins: None,
                vertices: vec![],
            })
            .collect();
        let selected = EdgeId::from_raw(2);
        let reference = name_edge(selected, &View { topology: &body }, &names).unwrap();
        assert!(reference.disambiguate);
        let saved = serde_json::to_string(&reference).unwrap();
        let reference: EdgeRef = serde_json::from_str(&saved).unwrap();
        assert_eq!(resolve_edge(&reference, &View { topology: &body }, &names).unwrap(), selected);
        let mut unique = body.clone();
        unique.edges.retain(|edge| edge.id == selected);
        let legacy = name_edge(selected, &View { topology: &unique }, &names).unwrap();
        assert!(!legacy.disambiguate);
        for edge in &mut unique.edges {
            edge.midpoint.x += 8.0;
        }
        for face in &mut unique.faces {
            face.centroid.x += 8.0;
        }
        assert_eq!(
            resolve_edge(&legacy, &View { topology: &unique }, &names).unwrap(),
            selected,
            "single-edge references retain their existing behavior"
        );
        let mut moved = body.clone();
        for edge in &mut moved.edges {
            edge.midpoint.x += 8.0;
        }
        for face in &mut moved.faces {
            face.centroid.x += 8.0;
        }
        assert!(
            matches!(
                resolve_edge(&reference, &View { topology: &moved }, &names).unwrap_err().reason,
                RepairReason::Ambiguous
            ),
            "a different branch at the old point must not steal the reference"
        );
        let mut lost = moved.clone();
        lost.edges.retain(|edge| edge.id != selected);
        assert!(
            resolve_edge(&reference, &View { topology: &lost }, &names).is_err(),
            "the surviving opposite branch must not take a missing generator's reference"
        );
        body.edges[0].midpoint = body.edges[1].midpoint;
        assert!(
            matches!(
                resolve_edge(&reference, &View { topology: &body }, &names).unwrap_err().reason,
                RepairReason::Ambiguous
            ),
            "equal fingerprints stay ambiguous"
        );
    }
}
