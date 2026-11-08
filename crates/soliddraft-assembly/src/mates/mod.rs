//! Mates: authored relationships that decide where components sit.
//!
//! A mate names faces, edges and vertices by occurrence path and the part's persistent
//! [`FaceRef`], [`EdgeRef`] or [`VertexRef`], or a part's reference plane by its datum or
//! feature id, never by label or kernel handle. Distances
//! are typed expressions; one that reads an assembly value is driven by it. Definitions are
//! the assembly's truth. Proofs, solver status and diagnostics are derived and stay out of
//! it.
//!
//! - [`solver`] places components from proven planes, axes and points and counts what
//!   stays free.
//! - [`reference`] finds faces, edges, vertices and planes and proves what they support
//!   at a pinned revision.
//! - [`command`] runs every change as one candidate: preview, prove, solve, then commit.
//! - [`bodies`] reads the mates of the root and of every sub-assembly in it, and the
//!   part occurrences they move, by path.
//!
//! The families are Origin, Distance, Coincident, Parallel, Perpendicular, Width, Lock,
//! Concentric, Tangent, Hinge, Symmetric, Linear Coupler, Limit Distance, Profile Center and
//! Slot. They cover axis-aligned planes, axes, points, profiles and straight slots with exact
//! coordinates, and quarter-turn rotations; see `docs/mates.md`.

pub(crate) mod bodies;
pub(crate) mod command;
mod reference;
pub(crate) mod solver;

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use soliddraft_document::{ConfigurationId, DocumentId};
use soliddraft_model::{DatumPlane, EdgeRef, Expression, FaceRef, FeatureId, VertexRef};

use crate::path::{ComponentId, OccurrencePath};
use crate::placement::Placement;

pub use command::{MateEdit, PlaneMeasurement, ProofIdentity, SlotInput, WidthInput};
pub use reference::{EndpointSelector, FaceSelector, GeneratorChoice, PlaneProof, SupportProof};
pub use solver::{FreeMotion, SolverState, SolverStatus};

pub(crate) use reference::Proofs;

/// A mate, stable within its assembly. Mate ids come from the assembly's one counter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MateId(pub u32);

impl fmt::Display for MateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// How two planar faces' outward normals relate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    /// The outward normals agree.
    Aligned,
    /// The outward normals oppose.
    Opposed,
}

impl Alignment {
    pub(crate) fn sign(self) -> i8 {
        match self {
            Alignment::Aligned => 1,
            Alignment::Opposed => -1,
        }
    }
}

/// A reference plane of a part: one of its three datum planes, or a plane feature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case", deny_unknown_fields)]
pub enum PartPlane {
    /// The Top, Front or Right plane through the part's origin.
    Datum {
        /// Which one.
        datum: DatumPlane,
    },
    /// A reference plane feature, followed through edits and renames.
    Feature {
        /// The feature.
        feature: FeatureId,
        /// The proven normal when the reference was made, each component -1, 0 or 1. A proof
        /// refuses a plane that now faces another way; the solver's status reads it so it
        /// needs no part.
        normal: [i8; 3],
    },
}

impl PartPlane {
    /// The normal the reference expects, in the part's frame.
    pub fn normal(&self) -> [i8; 3] {
        match self {
            PartPlane::Datum { datum } => datum_normal(*datum),
            PartPlane::Feature { normal, .. } => *normal,
        }
    }
}

/// A datum plane's normal: Top looks along +Z, Front along -Y, Right along +X.
pub(crate) fn datum_normal(datum: DatumPlane) -> [i8; 3] {
    match datum {
        DatumPlane::Top => [0, 0, 1],
        DatumPlane::Front => [0, -1, 0],
        DatumPlane::Right => [1, 0, 0],
    }
}

/// What a mate endpoint names on its part: a face, a reference plane, an edge or a vertex.
#[derive(Clone, Debug, PartialEq)]
pub enum Entity {
    /// A face, by the part's persistent name: a plane, cylinder, cone or sphere.
    Face(FaceRef),
    /// A reference plane.
    Plane(PartPlane),
    /// An edge, by the faces it separates: a line or a circle.
    Edge(EdgeRef),
    /// A vertex, by the faces that meet at it.
    Vertex(VertexRef),
}

impl Entity {
    /// The normal the stored reference expects, when it records one.
    pub(crate) fn stored_normal(&self) -> Option<[i8; 3]> {
        match self {
            Entity::Face(face) => {
                let n = face.fingerprint.normal?;
                Some([n.x, n.y, n.z].map(|c| c.round() as i8))
            }
            Entity::Plane(plane) => Some(plane.normal()),
            Entity::Edge(_) | Entity::Vertex(_) => None,
        }
    }

    /// The entity for a message: a face's label, or the plane.
    pub(crate) fn describe(&self) -> String {
        match self {
            Entity::Face(face) => format!("{} of feature {}", face.label, face.feature),
            Entity::Plane(PartPlane::Datum { datum }) => datum.label().to_owned(),
            Entity::Plane(PartPlane::Feature { feature, .. }) => format!("reference plane feature {feature}"),
            Entity::Edge(edge) => format!("edge {}", edge.label),
            Entity::Vertex(vertex) => format!("vertex {}", vertex.label),
        }
    }
}

/// The axis or point a proof found on an endpoint when it was mated, in the part's frame.
/// The solver's status reads it, so it needs no part; a proof refuses an endpoint whose
/// direction or kind has changed since. The coordinates are the nearest doubles, for the
/// status only: a solve reads the exact values from the proof.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoredSupport {
    /// The axis of a cylinder, cone or circle, or a line.
    Axis {
        /// The direction, each component -1, 0 or 1.
        direction: [i8; 3],
        /// A point on the line.
        through: [f64; 3],
        /// A cylinder's or circle's radius.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radius: Option<f64>,
        /// For a face with a radius, whether its outward side looks away from the axis.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outward: Option<bool>,
    },
    /// A sphere's centre or a vertex.
    Point {
        /// The point.
        at: [f64; 3],
        /// A sphere's radius.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radius: Option<f64>,
        /// For a sphere, whether its outward side looks away from the centre.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outward: Option<bool>,
    },
    /// The centre of a planar face's outline, a rectangle, a regular polygon or a circle, or
    /// of a circular edge.
    Profile {
        /// The plane's outward normal, or a circular edge's direction: each component -1,
        /// 0 or 1.
        normal: [i8; 3],
        /// The centre.
        centre: [f64; 3],
        /// A polygon's number of sides; absent for a circle.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sides: Option<u32>,
    },
    /// A straight slot: two parallel walls closed by two round ends.
    Slot {
        /// The direction the round ends' axes run, each component -1, 0 or 1.
        depth: [i8; 3],
        /// A point on each round end's axis, the end nearer the negative side of the slot's
        /// run first.
        ends: [[f64; 3]; 2],
        /// The round ends' radius.
        radius: f64,
    },
}

/// One side of a mate between planes: a face or a reference plane of one placed part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawEndpoint", into = "RawEndpoint")]
pub struct FaceEndpoint {
    /// The occurrence, from the assembly that holds the mate.
    pub path: OccurrencePath,
    /// The part document.
    pub document: DocumentId,
    /// The part's configuration; absent for the default.
    pub configuration: Option<ConfigurationId>,
    /// The face, plane, edge or vertex. Saved as `face`, `plane`, `edge` or `vertex`.
    pub entity: Entity,
    /// The axis or point it was mated as; absent for a plane.
    pub support: Option<StoredSupport>,
}

/// A face endpoint as `assembly.json` holds it: exactly one of `face`, `plane`, `edge` and
/// `vertex`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEndpoint {
    path: OccurrencePath,
    document: DocumentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    configuration: Option<ConfigurationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    face: Option<FaceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plane: Option<PartPlane>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    edge: Option<EdgeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    vertex: Option<VertexRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    support: Option<StoredSupport>,
}

impl TryFrom<RawEndpoint> for FaceEndpoint {
    type Error = String;

    fn try_from(raw: RawEndpoint) -> Result<FaceEndpoint, String> {
        let entity = match (raw.face, raw.plane, raw.edge, raw.vertex) {
            (Some(face), None, None, None) => Entity::Face(face),
            (None, Some(plane), None, None) => Entity::Plane(plane),
            (None, None, Some(edge), None) => Entity::Edge(edge),
            (None, None, None, Some(vertex)) => Entity::Vertex(vertex),
            _ => return Err("an endpoint names one face, plane, edge or vertex".into()),
        };
        let (path, document, configuration, support) = (raw.path, raw.document, raw.configuration, raw.support);
        Ok(FaceEndpoint { path, document, configuration, entity, support })
    }
}

impl From<FaceEndpoint> for RawEndpoint {
    fn from(e: FaceEndpoint) -> RawEndpoint {
        let mut raw = RawEndpoint {
            path: e.path,
            document: e.document,
            configuration: e.configuration,
            face: None,
            plane: None,
            edge: None,
            vertex: None,
            support: e.support,
        };
        match e.entity {
            Entity::Face(face) => raw.face = Some(face),
            Entity::Plane(plane) => raw.plane = Some(plane),
            Entity::Edge(edge) => raw.edge = Some(edge),
            Entity::Vertex(vertex) => raw.vertex = Some(vertex),
        }
        raw
    }
}

/// One side of a Lock: a whole placed part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartEndpoint {
    /// The occurrence, from the assembly that holds the mate.
    pub path: OccurrencePath,
    /// The part document.
    pub document: DocumentId,
    /// The part's configuration; absent for the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration: Option<ConfigurationId>,
}

/// Where a Width mate puts the tab between the groove's faces.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum WidthConstraint {
    /// Anywhere between the groove's faces.
    Free,
    /// The tab's mid-plane on the groove's.
    Centered,
    /// The gap from the first groove face (the second with `flipped`) to the tab face
    /// nearest it.
    Dimension {
        /// The gap. An expression that reads values is driven by them.
        distance: Expression,
        /// Measured from the second groove face.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        flipped: bool,
    },
    /// That gap as a percentage of the play, the groove's width less the tab's: 0 puts the
    /// tab against the first groove face, 50 centres it, 100 puts it against the second.
    Percent {
        /// The percentage.
        percent: f64,
        /// Measured from the second groove face.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        flipped: bool,
    },
}

/// Where a Slot mate puts the pin along the slot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum SlotConstraint {
    /// Anywhere between the round ends' centres.
    Free,
    /// Halfway between them.
    Center,
    /// The distance from the first end's centre, or the second's with `flipped`.
    Distance {
        /// The distance. An expression that reads values is driven by them.
        distance: Expression,
        /// Measured from the second end.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        flipped: bool,
    },
    /// That distance as a percentage of the run between the ends' centres.
    Percent {
        /// The percentage, 0 to 100.
        percent: f64,
        /// Measured from the second end.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        flipped: bool,
    },
}

/// A part occurrence a mate reads, whatever it names on the part.
pub struct EndView<'a> {
    /// The occurrence, from the assembly that holds the mate.
    pub path: &'a OccurrencePath,
    /// The part document.
    pub document: DocumentId,
    /// The part's configuration; absent for the default.
    pub configuration: Option<&'a ConfigurationId>,
}

/// A mate this version can solve.
#[allow(clippy::large_enum_variant, reason = "a mate list is short and read one by one")]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Mate {
    /// A component's origin on the assembly's. With `align_axes` its axes match too.
    Origin {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The component.
        component: ComponentId,
        /// Whether the component's axes match the assembly's as well.
        align_axes: bool,
    },
    /// The signed distance from `a`'s plane to `b`'s, along `a`'s outward normal. It fixes
    /// the normals' relation and one translation; the faces need not overlap.
    Distance {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The first face.
        a: FaceEndpoint,
        /// The second face.
        b: FaceEndpoint,
        /// How their outward normals relate.
        alignment: Alignment,
        /// The distance. An expression that reads values is driven by them.
        distance: Expression,
    },
    /// `a`'s plane and `b`'s are one plane. It fixes what a zero distance fixes, the
    /// normals' relation and one translation, and has no distance of its own; the faces
    /// need not overlap.
    Coincident {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The first face.
        a: FaceEndpoint,
        /// The second face.
        b: FaceEndpoint,
        /// How their outward normals relate.
        alignment: Alignment,
    },
    /// `a`'s plane and `b`'s are parallel, their normals as `alignment` says. It fixes two
    /// turns and no translation.
    Parallel {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The first face.
        a: FaceEndpoint,
        /// The second face.
        b: FaceEndpoint,
        /// How their outward normals relate.
        alignment: Alignment,
    },
    /// `a`'s plane and `b`'s are at right angles. It fixes one turn and no translation, and
    /// has no alignment: the normals are at right angles either way.
    Perpendicular {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The first face.
        a: FaceEndpoint,
        /// The second face.
        b: FaceEndpoint,
    },
    /// A tab between the two faces of a groove. The four faces are parallel; the tab's
    /// normals relate to the groove's as `alignment` says, from `groove[0]` to `tab[0]`.
    /// It fixes two turns and the translation across the groove.
    Width {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The groove's two faces, on one part.
        groove: [FaceEndpoint; 2],
        /// The tab's two faces, on another.
        tab: [FaceEndpoint; 2],
        /// How `tab[0]`'s outward normal relates to `groove[0]`'s.
        alignment: Alignment,
        /// Where the tab sits across the groove.
        constraint: WidthConstraint,
    },
    /// `b` keeps its placement relative to `a`: `b`'s placement is `a`'s composed with
    /// `relative`. It fixes all six motions between them. The relative turn is a
    /// quarter-turn rotation; anything else is refused.
    Lock {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The first part.
        a: PartEndpoint,
        /// The second part.
        b: PartEndpoint,
        /// `b`'s placement in `a`'s frame.
        relative: Placement,
    },
    /// `b`'s axis on `a`'s line, or a point on a line or on another point: two cylinders,
    /// cones or circles share an axis, a point lies on an axis, or two spheres share a
    /// centre. Planes are refused.
    Concentric {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The first face, edge or vertex.
        a: FaceEndpoint,
        /// The second.
        b: FaceEndpoint,
        /// How the two axes' directions relate; absent where a point is involved.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alignment: Option<Alignment>,
        /// With Lock rotation, `b`'s turn in `a`'s frame: a quarter-turn rotation, as its
        /// integer matrix by rows. Absent leaves the spin about the axis free.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lock_rotation: Option<[[i8; 3]; 3]>,
    },
    /// A cylinder or sphere touching a plane: the axis lies parallel to the plane, or the
    /// centre in front of it, one radius away. The radius must be rational.
    Tangent {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The plane.
        a: FaceEndpoint,
        /// The cylinder or sphere.
        b: FaceEndpoint,
        /// How the curved face's outward normal at the contact relates to the plane's:
        /// `opposed` for a shaft lying on a table, `aligned` for a shaft pressed against the
        /// plane from behind it.
        alignment: Alignment,
    },
    /// A concentric pair and a coincident pair under one name: a pin turning in a hole,
    /// with its shoulder on the face round the hole. Limits are not supported.
    Hinge {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The two axes, as a Concentric reads them.
        axes: [FaceEndpoint; 2],
        /// The two planes, as a Coincident reads them.
        planes: [FaceEndpoint; 2],
        /// How the axes' directions relate.
        alignment: Alignment,
        /// How the planes' outward normals relate.
        plane_alignment: Alignment,
    },
    /// `b` is `a` mirrored about `plane`: two planes, two lines or two points, on two parts,
    /// about a plane of any part.
    Symmetric {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The first entity.
        a: FaceEndpoint,
        /// Its mirror image.
        b: FaceEndpoint,
        /// The symmetry plane.
        plane: FaceEndpoint,
        /// How `b`'s outward normal or direction relates to `a`'s mirrored; absent for
        /// points.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        alignment: Option<Alignment>,
    },
    /// How far `b` slides along its entity's direction is a ratio of how far `a` does along
    /// its own, each measured from a reference part's origin or the assembly's.
    LinearCoupler {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The first part's plane or axis: its normal or direction is the way it slides.
        a: FaceEndpoint,
        /// The second's.
        b: FaceEndpoint,
        /// The part `a` slides relative to; absent for the assembly.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference_a: Option<PartEndpoint>,
        /// The part `b` slides relative to; absent for the assembly.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference_b: Option<PartEndpoint>,
        /// `a` slides `ratio[0]` while `b` slides `ratio[1]`.
        ratio: [f64; 2],
        /// `b` slides the other way.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reverse: bool,
        /// `ratio[0]` times how far `b` lies along its direction, less `ratio[1]` times how
        /// far `a` does (plus, when `reverse`), taken where the parts were when the mate was
        /// made: an exact rational such as `-381/5`.
        offset: String,
    },
    /// The signed distance from `a`'s plane to `b`, as a Distance measures it, anywhere from
    /// `min` to `max`.
    LimitDistance {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The plane the distance is measured from.
        a: FaceEndpoint,
        /// The plane, axis or point it is measured to.
        b: FaceEndpoint,
        /// How their outward normals relate.
        alignment: Alignment,
        /// The least distance.
        min: Expression,
        /// The greatest distance.
        max: Expression,
    },
    /// Two profiles, rectangles, regular polygons or circles, centred on each other with
    /// their planes `distance` apart. Polygons line their edges up; circles keep their spin
    /// unless `lock_rotation` holds it.
    ProfileCenter {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// The first profile.
        a: FaceEndpoint,
        /// The second.
        b: FaceEndpoint,
        /// How their planes' outward normals relate.
        alignment: Alignment,
        /// The signed distance from `a`'s plane to `b`'s, along `a`'s outward normal.
        distance: Expression,
        /// For circles, `b`'s turn in `a`'s frame as a quarter-turn matrix by rows. Absent
        /// leaves the spin free.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lock_rotation: Option<[[i8; 3]; 3]>,
    },
    /// A pin in a straight slot: square to the slot's run, on its mid-plane, and along it as
    /// `constraint` says.
    Slot {
        /// The mate's id.
        id: MateId,
        /// A label; commands find a mate by it.
        name: String,
        /// A wall or a round end of the slot.
        slot: FaceEndpoint,
        /// The pin's axis: a cylinder, or a circular or straight edge.
        pin: FaceEndpoint,
        /// How the pin's direction relates to the slot's round ends' axes.
        alignment: Alignment,
        /// Where the pin sits along the slot.
        constraint: SlotConstraint,
        /// The pin's turn in the slot's part's frame, as a quarter-turn matrix by rows.
        /// Absent leaves the spin free.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lock_rotation: Option<[[i8; 3]; 3]>,
    },
}

impl Mate {
    /// The mate's id.
    pub fn id(&self) -> MateId {
        match self {
            Mate::Origin { id, .. }
            | Mate::Distance { id, .. }
            | Mate::Coincident { id, .. }
            | Mate::Parallel { id, .. }
            | Mate::Perpendicular { id, .. }
            | Mate::Width { id, .. }
            | Mate::Lock { id, .. }
            | Mate::Concentric { id, .. }
            | Mate::Tangent { id, .. }
            | Mate::Hinge { id, .. }
            | Mate::Symmetric { id, .. }
            | Mate::LinearCoupler { id, .. }
            | Mate::LimitDistance { id, .. }
            | Mate::ProfileCenter { id, .. }
            | Mate::Slot { id, .. } => *id,
        }
    }

    /// The mate's label.
    pub fn name(&self) -> &str {
        match self {
            Mate::Origin { name, .. }
            | Mate::Distance { name, .. }
            | Mate::Coincident { name, .. }
            | Mate::Parallel { name, .. }
            | Mate::Perpendicular { name, .. }
            | Mate::Width { name, .. }
            | Mate::Lock { name, .. }
            | Mate::Concentric { name, .. }
            | Mate::Tangent { name, .. }
            | Mate::Hinge { name, .. }
            | Mate::Symmetric { name, .. }
            | Mate::LinearCoupler { name, .. }
            | Mate::LimitDistance { name, .. }
            | Mate::ProfileCenter { name, .. }
            | Mate::Slot { name, .. } => name,
        }
    }

    /// The family, as `assembly.json` names it.
    pub fn kind(&self) -> &'static str {
        match self {
            Mate::Origin { .. } => "origin",
            Mate::Distance { .. } => "distance",
            Mate::Coincident { .. } => "coincident",
            Mate::Parallel { .. } => "parallel",
            Mate::Perpendicular { .. } => "perpendicular",
            Mate::Width { .. } => "width",
            Mate::Lock { .. } => "lock",
            Mate::Concentric { .. } => "concentric",
            Mate::Tangent { .. } => "tangent",
            Mate::Hinge { .. } => "hinge",
            Mate::Symmetric { .. } => "symmetric",
            Mate::LinearCoupler { .. } => "linear_coupler",
            Mate::LimitDistance { .. } => "limit_distance",
            Mate::ProfileCenter { .. } => "profile_center",
            Mate::Slot { .. } => "slot",
        }
    }

    /// The root's components it ties: those its parts are, or the sub-assemblies that
    /// hold them.
    pub fn components(&self) -> Vec<ComponentId> {
        match self {
            Mate::Origin { component, .. } => vec![*component],
            _ => self.ends().iter().filter_map(|e| holder(e.path)).collect(),
        }
    }

    /// The distance's expression, when it has one.
    pub fn expression(&self) -> Option<&Expression> {
        match self {
            Mate::Distance { distance, .. }
            | Mate::Width { constraint: WidthConstraint::Dimension { distance, .. }, .. }
            | Mate::ProfileCenter { distance, .. }
            | Mate::Slot { constraint: SlotConstraint::Distance { distance, .. }, .. } => Some(distance),
            _ => None,
        }
    }

    /// The distance's expression, to change it.
    pub(crate) fn expression_mut(&mut self) -> Option<&mut Expression> {
        match self {
            Mate::Distance { distance, .. }
            | Mate::Width { constraint: WidthConstraint::Dimension { distance, .. }, .. }
            | Mate::ProfileCenter { distance, .. }
            | Mate::Slot { constraint: SlotConstraint::Distance { distance, .. }, .. } => Some(distance),
            _ => None,
        }
    }

    /// Every face or plane the mate names, in order.
    pub fn faces(&self) -> Vec<&FaceEndpoint> {
        match self {
            Mate::Distance { a, b, .. }
            | Mate::Coincident { a, b, .. }
            | Mate::Parallel { a, b, .. }
            | Mate::Perpendicular { a, b, .. }
            | Mate::Concentric { a, b, .. }
            | Mate::Tangent { a, b, .. }
            | Mate::LinearCoupler { a, b, .. }
            | Mate::LimitDistance { a, b, .. }
            | Mate::ProfileCenter { a, b, .. } => vec![a, b],
            Mate::Symmetric { a, b, plane, .. } => vec![a, b, plane],
            Mate::Slot { slot, pin, .. } => vec![slot, pin],
            Mate::Width { groove, tab, .. } => groove.iter().chain(tab).collect(),
            Mate::Hinge { axes, planes, .. } => axes.iter().chain(planes).collect(),
            Mate::Origin { .. } | Mate::Lock { .. } => Vec::new(),
        }
    }

    /// The same endpoints, to change.
    pub(crate) fn faces_mut(&mut self) -> Vec<&mut FaceEndpoint> {
        match self {
            Mate::Distance { a, b, .. }
            | Mate::Coincident { a, b, .. }
            | Mate::Parallel { a, b, .. }
            | Mate::Perpendicular { a, b, .. }
            | Mate::Concentric { a, b, .. }
            | Mate::Tangent { a, b, .. }
            | Mate::LinearCoupler { a, b, .. }
            | Mate::LimitDistance { a, b, .. }
            | Mate::ProfileCenter { a, b, .. } => vec![a, b],
            Mate::Symmetric { a, b, plane, .. } => vec![a, b, plane],
            Mate::Slot { slot, pin, .. } => vec![slot, pin],
            Mate::Width { groove, tab, .. } => groove.iter_mut().chain(tab).collect(),
            Mate::Hinge { axes, planes, .. } => axes.iter_mut().chain(planes).collect(),
            Mate::Origin { .. } | Mate::Lock { .. } => Vec::new(),
        }
    }

    /// The parts a Linear Coupler measures from, as it names them.
    pub fn references(&self) -> Vec<&PartEndpoint> {
        match self {
            Mate::LinearCoupler { reference_a, reference_b, .. } => reference_a.iter().chain(reference_b).collect(),
            _ => Vec::new(),
        }
    }

    /// The two planes a mate measures between, when it fixes a distance between parallel
    /// planes or keeps them parallel: a Width measures from `groove[0]` to `tab[0]`.
    pub fn measured(&self) -> Option<[&FaceEndpoint; 2]> {
        match self {
            Mate::Distance { a, b, .. }
            | Mate::Coincident { a, b, .. }
            | Mate::Parallel { a, b, .. }
            | Mate::LimitDistance { a, b, .. } => Some([a, b]),
            Mate::Width { groove, tab, .. } => Some([&groove[0], &tab[0]]),
            Mate::Origin { .. }
            | Mate::Perpendicular { .. }
            | Mate::Lock { .. }
            | Mate::Concentric { .. }
            | Mate::Tangent { .. }
            | Mate::Hinge { .. }
            | Mate::Symmetric { .. }
            | Mate::LinearCoupler { .. }
            | Mate::ProfileCenter { .. }
            | Mate::Slot { .. } => None,
        }
    }

    /// How the normals relate, for a family that has an alignment.
    pub fn alignment(&self) -> Option<Alignment> {
        match self {
            Mate::Distance { alignment, .. }
            | Mate::Coincident { alignment, .. }
            | Mate::Parallel { alignment, .. }
            | Mate::Width { alignment, .. }
            | Mate::Tangent { alignment, .. }
            | Mate::Hinge { alignment, .. }
            | Mate::LimitDistance { alignment, .. }
            | Mate::ProfileCenter { alignment, .. }
            | Mate::Slot { alignment, .. } => Some(*alignment),
            Mate::Concentric { alignment, .. } | Mate::Symmetric { alignment, .. } => *alignment,
            Mate::Origin { .. } | Mate::Perpendicular { .. } | Mate::Lock { .. } | Mate::LinearCoupler { .. } => None,
        }
    }

    /// The part occurrences the mate reads, in order. An Origin names a component and has
    /// none.
    pub fn ends(&self) -> Vec<EndView<'_>> {
        fn part(e: &PartEndpoint) -> EndView<'_> {
            EndView { path: &e.path, document: e.document, configuration: e.configuration.as_ref() }
        }
        match self {
            Mate::Lock { a, b, .. } => [a, b].into_iter().map(part).collect(),
            _ => self
                .faces()
                .into_iter()
                .map(|e| EndView { path: &e.path, document: e.document, configuration: e.configuration.as_ref() })
                .chain(self.references().into_iter().map(part))
                .collect(),
        }
    }

    /// The occurrence paths the mate reads, to change them.
    pub(crate) fn paths_mut(&mut self) -> Vec<&mut OccurrencePath> {
        match self {
            Mate::Distance { a, b, .. }
            | Mate::Coincident { a, b, .. }
            | Mate::Parallel { a, b, .. }
            | Mate::Perpendicular { a, b, .. }
            | Mate::Concentric { a, b, .. }
            | Mate::Tangent { a, b, .. }
            | Mate::LimitDistance { a, b, .. }
            | Mate::ProfileCenter { a, b, .. } => vec![&mut a.path, &mut b.path],
            Mate::Symmetric { a, b, plane, .. } => vec![&mut a.path, &mut b.path, &mut plane.path],
            Mate::Slot { slot, pin, .. } => vec![&mut slot.path, &mut pin.path],
            Mate::LinearCoupler { a, b, reference_a, reference_b, .. } => [&mut a.path, &mut b.path]
                .into_iter()
                .chain(reference_a.iter_mut().map(|r| &mut r.path))
                .chain(reference_b.iter_mut().map(|r| &mut r.path))
                .collect(),
            Mate::Width { groove, tab, .. } => groove.iter_mut().chain(tab.iter_mut()).map(|e| &mut e.path).collect(),
            Mate::Hinge { axes, planes, .. } => axes.iter_mut().chain(planes.iter_mut()).map(|e| &mut e.path).collect(),
            Mate::Lock { a, b, .. } => vec![&mut a.path, &mut b.path],
            Mate::Origin { .. } => Vec::new(),
        }
    }
}

/// The root component that holds the part at `path`: the part itself, or the
/// sub-assembly it is placed in. `None` for a path through a pattern member, which mates
/// do not reach.
pub(crate) fn holder(path: &OccurrencePath) -> Option<ComponentId> {
    let steps = path.steps();
    (!steps.is_empty() && steps.iter().all(|s| s.member.is_none())).then(|| steps[0].component)
}

impl FaceEndpoint {
    /// The root component it is on, when it is on one of the root's own parts.
    pub fn component(&self) -> Option<ComponentId> {
        match self.path.steps() {
            [step] if step.member.is_none() => Some(step.component),
            _ => None,
        }
    }

    /// The root component that holds its part: the part itself, or the sub-assembly it is
    /// placed in. `None` for a path through a pattern member, which mates do not reach.
    pub fn holder(&self) -> Option<ComponentId> {
        holder(&self.path)
    }
}

/// An entry of the assembly's mate list.
#[allow(clippy::large_enum_variant, reason = "a mate list is short and read one by one")]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MateEntry {
    /// A mate this version understands.
    Known(Mate),
    /// A mate of a family or shape this version does not. It is kept as read and never
    /// counted as solved: while one is there, nothing that would solve can commit.
    Future(Value),
}

impl MateEntry {
    /// The mate, when this version understands it.
    pub fn known(&self) -> Option<&Mate> {
        match self {
            MateEntry::Known(m) => Some(m),
            MateEntry::Future(_) => None,
        }
    }

    /// Its place in the canonical order: known mates by id, then the rest as read.
    pub(crate) fn order(&self) -> (u8, u32) {
        match self {
            MateEntry::Known(m) => (0, m.id().0),
            MateEntry::Future(_) => (1, 0),
        }
    }
}

/// Why a mate command was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalCode {
    /// The candidate leaves freedom a strict request forbids.
    UnderConstrained,
    /// The candidate's mates cannot all hold.
    OverConstrained,
    /// A stored face no longer exists at the proposed revision.
    DanglingReference,
    /// A face selector or reference finds several faces.
    AmbiguousReference,
    /// No face matches a selector.
    NoSuchFace,
    /// A mate or the source it reads names a configuration other than the default.
    UnsupportedConfiguration,
    /// The kernel will not prove this face's plane.
    UnsupportedPlane,
    /// The kernel will not prove this axis or point, or the family does not take it.
    UnsupportedReference,
    /// A placed plane's normal is not on a world axis.
    UnsupportedPlacement,
    /// The two faces' planes are not parallel.
    NotParallel,
    /// The part bytes or kernel needed to prove a plane are not available.
    ProofUnavailable,
    /// The assembly holds a mate this version cannot solve.
    UnknownMate,
    /// A literal edit of a value-driven input.
    DrivenInput,
    /// A second driver for a driven input, without asking to replace the first.
    AlreadyDriven,
    /// An expression of the wrong dimension.
    UnitMismatch,
    /// A value that depends on itself.
    ValueCycle,
    /// An expression or value that cannot be used.
    InvalidValue,
    /// A request that names nothing, or names something unusable.
    Invalid,
    /// More components tied together than this solver takes.
    TooLarge,
}

impl RefusalCode {
    /// The code as text.
    pub fn as_str(self) -> &'static str {
        match self {
            RefusalCode::UnderConstrained => "under_constrained",
            RefusalCode::OverConstrained => "over_constrained",
            RefusalCode::DanglingReference => "dangling_reference",
            RefusalCode::AmbiguousReference => "ambiguous_reference",
            RefusalCode::NoSuchFace => "no_such_face",
            RefusalCode::UnsupportedConfiguration => "unsupported_configuration",
            RefusalCode::UnsupportedPlane => "unsupported_plane",
            RefusalCode::UnsupportedReference => "unsupported_reference",
            RefusalCode::UnsupportedPlacement => "unsupported_placement",
            RefusalCode::NotParallel => "not_parallel",
            RefusalCode::ProofUnavailable => "proof_unavailable",
            RefusalCode::UnknownMate => "unknown_mate",
            RefusalCode::DrivenInput => "driven_input",
            RefusalCode::AlreadyDriven => "already_driven",
            RefusalCode::UnitMismatch => "unit_mismatch",
            RefusalCode::ValueCycle => "value_cycle",
            RefusalCode::InvalidValue => "invalid_value",
            RefusalCode::Invalid => "invalid",
            RefusalCode::TooLarge => "too_large",
        }
    }
}

/// A refused mate command: what kind of refusal, what to tell the user, and what it
/// involved. The accepted assembly is exactly as it was.
#[derive(Clone, Debug, PartialEq, Serialize, thiserror::Error)]
#[error("{}: {message}", code.as_str())]
pub struct MateRefusal {
    /// The kind of refusal.
    #[serde(rename = "refused")]
    pub code: RefusalCode,
    /// The problem, for people.
    pub message: String,
    /// The mates involved, by name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mates: Vec<String>,
    /// How many freedoms the candidate would leave, when that is the problem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_dof: Option<usize>,
    /// The free motions, when that is the problem.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub free: Vec<FreeMotion>,
}

impl MateRefusal {
    /// A refusal with nothing but a code and a message.
    pub fn new(code: RefusalCode, message: impl Into<String>) -> MateRefusal {
        MateRefusal { code, message: message.into(), mates: Vec::new(), remaining_dof: None, free: Vec::new() }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> MateRefusal {
        MateRefusal::new(RefusalCode::Invalid, message)
    }
}
