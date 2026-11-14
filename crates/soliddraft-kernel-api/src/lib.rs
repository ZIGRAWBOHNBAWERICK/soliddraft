//! The geometry-kernel interface SolidDraft owns.
//!
//! The app talks to a kernel only through the [`Kernel`] trait. A backend owns every
//! body and hands out opaque [`BodyId`] handles; no kernel type crosses the trait.
//!
//! The contract every backend keeps:
//!
//! - **Operations do not mutate.** Each operation reads its input bodies and returns a
//!   new body. Inputs stay valid and unchanged, so a failed operation always leaves the
//!   previous body as it was. Callers release bodies they no longer need.
//! - **Every operation returns `Result`.** A kernel's own success status is not trusted
//!   on its own: a backend validates each result before returning it and reports an
//!   invalid one as [`KernelError::InvalidResult`].
//! - **Every operation is recorded.** [`Kernel::history`] returns, for a body, the
//!   records of all operations that produced it, keyed by the caller's [`OpTag`], with
//!   the faces, edges and vertices each one created, modified, deleted or split, and the
//!   role of every face it created (a cap, a side swept from a given profile segment, a
//!   blend of a given edge).
//! - **Faces carry their origins.** [`FaceInfo::origins`] lists the operations and roles
//!   that made each face, kept through splits and merges. Persistent references resolve
//!   by them.
//!
//! Units are millimetres and degrees.

#[cfg(feature = "contract")]
pub mod contract;
#[cfg(feature = "counting")]
pub mod counting;
pub mod drawing;
pub mod measure;
pub use assembly_measure::{
    Axis, EnclosedSupport, ExactAxis, ExactAxisPlane, ExactPoint, ExactRadius, ExactRational, ExactSupport,
    SupportBounds, SupportEntity, SupportSource,
};
pub use measure::{Certainty, Measurement, PairEndpoint, PairQuantity, PointProof};
mod assembly_measure;
mod body_relation;
pub use body_relation::{BodyContactFaces, BodyDistance, BodyQueryRefusal, BodyRelation, OverlapVolume, PlacedBody};
mod checks;
mod edit;
mod field;
mod geometry;
mod history;
mod ids;
mod iges;
mod import;
mod profile;
pub mod scan;
mod sweep;

pub use checks::{IndependentCheck, IndependentStep, PrintMesh, PrintReport};
pub use edit::{ConstructionEdit, ConstructionFace, ConstructionMesh, ProofWork, ScalarEdit, ScalarParameter};
pub use field::{
    ExactRange, ExchangedMoments, FieldMass, FieldMassAccuracy, Grade, GradeAxis, LinearGrade, ProxyMoments,
    StandInExport, StandInReport,
};
pub use geometry::{Aabb, Plane, Point2, Vec3};
pub use history::{EntityChanges, FaceOrigin, FaceRole, HistoryRecord, HolePart, OpKind, Origin, Split};
pub use ids::{BodyId, EdgeId, FaceId, OpTag, VertexId};
pub use iges::{IgesExport, IgesOptions, IgesReport};
pub use import::{
    Approval, DecimalBounds, Decision, Disposition, FaceTolerance, ImportFormat, ImportOptions, ImportStepRequest,
    ImportStepResult, ImportedBody, LEGACY_REPAIR_FORMAT, PROPOSALS_FORMAT, PlanarRepairMethod, Proposal, ProposalSet,
    REPAIR_FORMAT, RepairChange, RepairDerivative, RepairRecord, RepairSource, SourceIncidence, question_id,
    question_index,
};
pub use profile::{BezierControls, Loop, PROFILE_JOIN_TOLERANCE, Profile, ProfileError, Segment};
pub use sweep::{ArcTurn, PathSegment, Sweep, SweepOrientation};

use serde::{Deserialize, Serialize};

/// Which boolean to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BooleanOp {
    /// Material in either body.
    Union,
    /// Material in the target but not in the tool.
    Subtract,
    /// Material in both bodies.
    Intersect,
}

impl BooleanOp {
    /// The history kind of this boolean.
    pub fn kind(self) -> OpKind {
        match self {
            BooleanOp::Union => OpKind::Union,
            BooleanOp::Subtract => OpKind::Subtract,
            BooleanOp::Intersect => OpKind::Intersect,
        }
    }
}

/// Parameters of a linear extrusion of a profile body.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Extrude {
    /// Distance to sweep, greater than zero.
    pub distance: f64,
    /// Unit sweep direction. `None` uses the profile plane's normal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<Vec3>,
}

/// Parameters of a revolution of a profile body about an axis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Revolve {
    /// A point on the axis.
    pub axis_origin: Vec3,
    /// Unit axis direction. Positive angles turn counter-clockwise about it.
    pub axis_direction: Vec3,
    /// Sweep angle in degrees, in (0, 360].
    pub angle_deg: f64,
}

/// Where one hole starts and which way it drills.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HolePlacement {
    /// The point on the hole's axis where depth zero is, usually on the entry face.
    pub point: Vec3,
    /// The drilling direction, into the material.
    pub direction: Vec3,
    /// The same placement as authored on a planar face. A backend with exact numbers
    /// builds from it, because `point` has passed through the face's binary64 frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_face: Option<FacePoint>,
}

/// A point in the sketch frame of a planar face of the body being drilled.
///
/// That frame has its origin at the world origin projected onto the face. Its X axis is the
/// world Z axis crossed with the face's outward normal, or the world X axis projected onto
/// a horizontal face, and its Y axis is the normal crossed with X.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FacePoint {
    /// The face.
    pub face: FaceId,
    /// Along the frame's X axis, mm.
    pub u: f64,
    /// Along the frame's Y axis, mm.
    pub v: f64,
}

/// How deep a hole's main wall goes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HoleDepth {
    /// To this depth below the placement point.
    Blind {
        /// Depth in millimetres, greater than zero.
        depth: f64,
    },
    /// Through everything along the drilling direction.
    Through,
}

/// The bottom of a blind hole.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HoleTip {
    /// A flat floor.
    Flat,
    /// A drill-point cone.
    Cone {
        /// Full angle in degrees, in (0, 180); 118 for a standard drill.
        angle_deg: f64,
    },
}

/// A counterbore or countersink at a hole's entry.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HoleHead {
    /// A wider cylinder at the entry.
    Counterbore {
        /// Diameter, more than the hole's.
        diameter: f64,
        /// Depth below the placement point.
        depth: f64,
    },
    /// A cone at the entry.
    Countersink {
        /// Diameter at depth zero, more than the hole's.
        diameter: f64,
        /// Full angle in degrees, in (0, 180); 82, 90 and 100 are standard.
        angle_deg: f64,
    },
}

/// Parameters of a set of holes drilled into a solid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hole {
    /// One entry per hole.
    pub placements: Vec<HolePlacement>,
    /// Diameter of the main wall.
    pub diameter: f64,
    /// Depth of the main wall.
    pub depth: HoleDepth,
    /// The bottom of a blind hole.
    pub tip: HoleTip,
    /// A counterbore or countersink at the entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<HoleHead>,
}

/// Where a pattern's copies go. Copy `k` runs from 0, the body itself.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PatternLayout {
    /// Copy `k` is translated by `k` times `step`.
    Linear {
        /// The translation between neighbouring copies.
        step: Vec3,
        /// Number of copies, including the body itself.
        count: u32,
    },
    /// Copy `k` is turned by `k` times `angle_deg` about the axis.
    Circular {
        /// A point on the axis.
        axis_origin: Vec3,
        /// The axis direction; positive angles turn counter-clockwise about it.
        axis_direction: Vec3,
        /// Number of copies, including the body itself.
        count: u32,
        /// The angle between neighbouring copies, in degrees.
        angle_deg: f64,
    },
}

/// Which side of selected faces receives the thickened material.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThickenSide {
    /// Along the outward face normals.
    #[default]
    Front,
    /// Against the outward face normals.
    Back,
}

/// A solid made from selected faces at a constant thickness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Thicken {
    /// Faces of the input body to thicken.
    pub faces: Vec<FaceId>,
    /// Positive thickness in millimetres.
    pub thickness: f64,
    /// Where the material lies relative to the selected faces.
    pub side: ThickenSide,
}

/// Parameters of a reflection of a solid in a plane.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mirror {
    /// A point on the mirror plane.
    pub plane_origin: Vec3,
    /// The mirror plane's normal.
    pub plane_normal: Vec3,
    /// Unites the reflection with the original when true.
    pub keep_original: bool,
}

/// Parameters of a draft: faces tilted away from a pull direction about a neutral plane.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    /// The direction the part leaves its mould in.
    pub pull: Vec3,
    /// A point on the neutral plane, where drafted faces keep their place.
    pub neutral_origin: Vec3,
    /// The neutral plane's normal.
    pub neutral_normal: Vec3,
    /// Draft angle in degrees, in (-90, 90).
    pub angle_deg: f64,
}

/// The supported lattice cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LatticeCell {
    /// Axis-aligned cubic beams.
    Cubic,
    /// Beams from the cell centre to its eight corners.
    BodyCentred,
    /// Beams from each face centre to its four corners.
    FaceCentred,
    /// Face-centred beams with links between neighbouring face centres.
    Octet,
}

impl LatticeCell {
    /// Cells available in the lattice editor.
    pub const ALL: [Self; 4] = [Self::Cubic, Self::BodyCentred, Self::FaceCentred, Self::Octet];

    /// The name shown in the editor.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cubic => "Cubic",
            Self::BodyCentred => "Body centred",
            Self::FaceCentred => "Face centred",
            Self::Octet => "Octet",
        }
    }
}

/// A terminal field is fixed to the part origin and axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldFrame {
    /// The part frame, with a lattice node at its origin.
    #[default]
    Part,
}

/// The supported gyroid kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GyroidKind {
    /// The material where the gyroid function is at most the level.
    #[default]
    Network,
}

/// A certified, labelled field file and its proof report.
#[derive(Clone, Debug)]
pub struct FieldExport {
    /// The labelled binary STL.
    pub bytes: Vec<u8>,
    /// The proof attached to those bytes.
    pub report: FieldReport,
}

/// Proof and approximation metadata of a field mesh.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldReport {
    /// The label format in the file's header, `catallax-print-stl/1`.
    pub label: String,
    /// Field meshes approximate the native field.
    pub approximate: bool,
    /// Native export route.
    pub route: String,
    /// Exact rational requested accuracy in millimetres.
    pub requested_accuracy_mm: String,
    /// Exact rational smallest detail the stage keeps, in millimetres.
    pub minimum_detail_mm: String,
    /// Proven two-sided bound, including file rounding.
    pub deviation_bound_mm: f64,
    /// Closed, oppositely paired surface.
    pub closed: bool,
    /// Free of self-intersection by its conforming construction.
    pub non_self_intersecting: bool,
    /// One connected component.
    pub connected: bool,
    /// Number of components.
    pub components: usize,
    /// Written triangles.
    pub triangles: usize,
    /// Shared vertices.
    pub vertices: usize,
    /// Candidate-grid and certificate cells the export may visit, across all attempts.
    pub max_cells: usize,
    /// Vertices the mesh may have.
    pub max_vertices: usize,
    /// Deterministic work units the export used.
    pub work: u64,
    /// SHA-256 of the labelled file.
    pub sha256: String,
}

/// Parameters of the final lattice stage.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lattice {
    /// Cell type.
    pub cell: LatticeCell,
    /// Cubic cell edge length in millimetres.
    pub cell_size_mm: f64,
    /// Beam radius in millimetres, read at each point; it controls density.
    pub radius_mm: Grade,
    /// Fixed part frame.
    pub frame: FieldFrame,
    /// Accuracy the lattice is proven to, in millimetres.
    pub accuracy_mm: f64,
    /// The smallest detail the lattice keeps, in millimetres.
    pub min_detail_mm: f64,
}

/// Parameters of the final gyroid stage.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gyroid {
    /// Gyroid kind.
    pub kind: GyroidKind,
    /// Length of one period in millimetres.
    pub period_mm: f64,
    /// Dimensionless level of the gyroid function, which may be negative. It is not a
    /// wall thickness.
    pub level: Grade,
    /// Fixed part frame.
    pub frame: FieldFrame,
    /// Accuracy the gyroid is proven to, in millimetres.
    pub accuracy_mm: f64,
    /// The smallest detail the gyroid keeps, in millimetres.
    pub min_detail_mm: f64,
}

/// Where an extrusion stops when it is not a fixed distance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExtrudeLimit {
    /// At the first place the sweep crosses the target's boundary.
    UpToNext,
    /// At the first hit on a face of the target.
    UpToFace {
        /// A face of the target body.
        face: FaceId,
    },
}

/// Tessellation quality. Zero disables a control.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TessellationParams {
    /// Maximum distance between the mesh and the surface, in millimetres.
    pub chord_height: f64,
    /// Maximum angle between tangents along a curve, in degrees.
    pub curve_angle_deg: f64,
    /// Maximum angle between normals across a triangle, in degrees.
    pub surface_angle_deg: f64,
    /// Target triangle edge length, in millimetres.
    pub max_edge_length: f64,
    /// Largest ratio of a triangle's longest edge to its shortest height. Files and
    /// requests from before this control was added leave it out, which disables it.
    #[serde(default)]
    pub max_aspect_ratio: f64,
}

impl Default for TessellationParams {
    /// 15 degrees for the surface angle: at the SMLib default of 25, triangles next to
    /// circular fillets can wind against their face normal (SMLib known issue).
    fn default() -> Self {
        TessellationParams {
            chord_height: 0.0,
            curve_angle_deg: 15.0,
            surface_angle_deg: 15.0,
            max_edge_length: 0.0,
            max_aspect_ratio: 0.0,
        }
    }
}

/// A measured display result, distinct from a certified mesh or an unmeasured preview.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DisplayBound {
    /// Named native rendering case.
    pub kind: String,
    /// Versioned rule selected before evaluation.
    pub rule: String,
    /// Preset maximum deviation in millimetres.
    pub level_mm: f64,
    /// Whole-surface deviation including screen-coordinate rounding, in millimetres.
    pub measured_mm: f64,
    /// Native measurement method over the complete surface scope.
    pub method: String,
    /// Immutable retained definition, revision and covered entities.
    pub definition: String,
}

impl DisplayBound {
    /// Rejects missing evidence or a measured deviation outside the preset rule.
    pub fn validate(&self) -> Result<(), String> {
        if self.kind != "spline display mesh"
            || self.rule.trim().is_empty()
            || self.method.trim().is_empty()
            || self.definition.trim().is_empty()
            || self.level_mm != 0.01
            || !self.measured_mm.is_finite()
            || self.measured_mm < 0.0
            || self.measured_mm > self.level_mm
        {
            return Err("The spline display needs a measured deviation within its stated 0.01 mm bound and a retained definition".into());
        }
        Ok(())
    }
}

/// A triangle mesh of a body, with each triangle labelled by the face it came from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    /// Vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Unit vertex normals, one per position.
    pub normals: Vec<[f32; 3]>,
    /// Three indices into `positions` per triangle, counter-clockwise seen from outside.
    pub indices: Vec<u32>,
    /// The source face of each triangle.
    pub triangle_faces: Vec<FaceId>,
    /// Requested controls this backend cannot guarantee. The mesh still meets its supported controls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unavailable_controls: Vec<String>,
    /// True for rendering without proof. A measured bound, when present, stays distinct
    /// from a certificate. This geometry is never exported or published as proven.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unproved: bool,
    /// Native measured display evidence. This geometry remains unproved and is never exported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounded: Option<DisplayBound>,
}

impl Mesh {
    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Whether a body encloses volume.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    /// A closed solid.
    Solid,
    /// Faces that do not enclose volume, such as a profile region.
    Sheet,
}

/// The geometric type of a face's surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    /// A plane.
    Plane,
    /// A circular cylinder.
    Cylinder,
    /// A circular cone.
    Cone,
    /// A sphere.
    Sphere,
    /// A torus.
    Torus,
    /// A NURBS or B-spline surface.
    Spline,
    /// Any other surface, such as an offset or swept surface.
    Other,
}

/// The geometric type of an edge's curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveKind {
    /// A straight line.
    Line,
    /// A circle or circular arc held as an analytic circle.
    Circle,
    /// An ellipse or elliptical arc.
    Ellipse,
    /// A NURBS or B-spline curve. Arcs a backend builds as rational splines land here.
    Spline,
    /// Any other curve, such as an intersection curve.
    Other,
}

/// How a face's carrier relates to its retained geometric definition. This is
/// independent of tessellation quality, measurement accuracy and export rounding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FaceAccuracy {
    /// No approximation of the face's retained surface definition.
    Exact,
    /// A checked fit, with a conservative upper bound in millimetres.
    Fitted {
        /// Maximum distance to the retained surface definition, rounded up.
        max_deviation_mm: f64,
    },
    /// The backend supplies no carrier accuracy, for example a field preview.
    #[default]
    Unknown,
}

/// One face of a body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceInfo {
    /// Id that follows the face through history.
    pub id: FaceId,
    /// Surface type.
    pub surface: SurfaceKind,
    /// Carrier accuracy reported by the backend.
    #[serde(default)]
    pub accuracy: FaceAccuracy,
    /// Area within the trim boundary.
    pub area: f64,
    /// Area centroid.
    pub centroid: Vec3,
    /// Bounding box.
    pub bbox: Aabb,
    /// Edges on the face's boundary.
    pub edges: Vec<EdgeId>,
    /// For a planar face, the unit normal pointing out of the material (for a sheet, the
    /// side its profile plane's normal points to). `None` for curved faces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<Vec3>,
    /// The operations that made the face, in a stable order: its persistent identity.
    /// Empty when the backend reports no origins for it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<Origin>,
}

/// What a reference reads of a face, taken from the exact model with no display mesh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceCarrier {
    /// As [`FaceInfo::id`].
    pub id: FaceId,
    /// As [`FaceInfo::surface`].
    pub surface: SurfaceKind,
    /// As [`FaceInfo::normal`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<Vec3>,
    /// A point of a planar face's supporting plane, not necessarily on the trimmed face.
    /// `None` for curved faces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_plane: Option<Vec3>,
    /// As [`FaceInfo::origins`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<Origin>,
}

/// One edge of a body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeInfo {
    /// Id that follows the edge through history.
    pub id: EdgeId,
    /// Curve type.
    #[serde(default = "default_curve")]
    pub curve: CurveKind,
    /// Arc length.
    pub length: f64,
    /// The point at the middle of the edge's parameter range.
    pub midpoint: Vec3,
    /// Bounding box.
    pub bbox: Aabb,
    /// Faces that use the edge: two on a closed solid. A seam lists its face twice.
    pub faces: Vec<FaceId>,
    /// Canonical construction origins of the two incident faces, when the backend
    /// supplies them. These survive face merging; their order need not match `faces`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub construction_origins: Option<[Origin; 2]>,
    /// Start and end vertex; one vertex for a closed edge such as a full circle.
    #[serde(default)]
    pub vertices: Vec<VertexId>,
}

fn default_curve() -> CurveKind {
    CurveKind::Other
}

/// One vertex of a body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VertexInfo {
    /// Id that follows the vertex through history.
    pub id: VertexId,
    /// Position.
    pub position: Vec3,
    /// Edges that end at the vertex.
    pub edges: Vec<EdgeId>,
}

/// The faces, edges and vertices of a body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Topology {
    /// Solid or sheet.
    pub kind: BodyKind,
    /// Faces in backend order.
    pub faces: Vec<FaceInfo>,
    /// Edges in backend order.
    pub edges: Vec<EdgeInfo>,
    /// Number of vertices, the length of `vertices`.
    pub vertex_count: usize,
    /// Vertices in backend order.
    #[serde(default)]
    pub vertices: Vec<VertexInfo>,
    /// Bounding box of the whole body.
    pub bbox: Aabb,
    /// Chord tolerance when centroids, bounds and edge measures describe the display mesh.
    /// Absent when these are kernel geometric queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_chord_height: Option<f64>,
}

impl Topology {
    /// The face with this id.
    pub fn face(&self, id: FaceId) -> Option<&FaceInfo> {
        self.faces.iter().find(|f| f.id == id)
    }

    /// The edge with this id.
    pub fn edge(&self, id: EdgeId) -> Option<&EdgeInfo> {
        self.edges.iter().find(|e| e.id == id)
    }

    /// The vertex with this id.
    pub fn vertex(&self, id: VertexId) -> Option<&VertexInfo> {
        self.vertices.iter().find(|v| v.id == id)
    }
}

/// An edge sampled along its exact curve, for drawing and picking.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgePolyline {
    /// The edge.
    pub edge: EdgeId,
    /// Points on the curve from the edge's start vertex to its end vertex. A closed edge
    /// repeats its first point at the end.
    pub points: Vec<Vec3>,
}

/// Mass properties at unit density.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MassProperties {
    /// Enclosed volume, in cubic millimetres. Zero for a sheet.
    pub volume: f64,
    /// Total surface area, in square millimetres.
    pub area: f64,
    /// Centre of mass. For a sheet, the area centroid.
    pub centroid: Vec3,
    /// Inertia tensor about the centroid, along world axes, at unit density.
    /// Absent when the backend cannot yet compute inertia.
    #[serde(default)]
    pub inertia: Option<[[f64; 3]; 3]>,
}

/// The result of [`Kernel::validate`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Validation {
    /// True when the body passed every check for its kind.
    pub valid: bool,
    /// Solid or sheet.
    pub kind: BodyKind,
    /// True when the body is a closed, manifold solid.
    pub manifold_solid: bool,
    /// What failed, in words. Empty when `valid`.
    pub issues: Vec<String>,
}

/// What a STEP export writes and how closely it must follow the exact body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepOptions {
    /// Product name written into the file.
    pub product_name: String,
    /// Largest distance, in millimetres, the written geometry may lie from the exact
    /// body, including decimal rounding and fitted curves. Greater than zero.
    pub tolerance: f64,
}

impl Default for StepOptions {
    /// 0.0000001 mm, Catallax's default export tolerance.
    fn default() -> Self {
        StepOptions { product_name: "part".into(), tolerance: 1e-7 }
    }
}

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

/// What a STEP export wrote and how far it is from the exact body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepReport {
    /// Proven upper bound, in millimetres, on the distance between the written geometry
    /// and the body, as a decimal rounded up.
    pub max_deviation: String,
    /// The same bound as the nearest `f64` not below it.
    pub max_deviation_mm: f64,
    /// Proven upper bound, in millimetres, on how far the body's fitted surfaces are from
    /// their exact definitions. It adds to `max_deviation_mm`, and the requested tolerance
    /// does not govern it. Zero for a body with no fitted surface.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub fit_mm: f64,
    /// True when `max_deviation` meets the requested tolerance.
    pub within_tolerance: bool,
    /// Solids written.
    pub solids: usize,
    /// Faces written.
    pub faces: usize,
    /// Edges written, including edges STEP needs that the body lacks.
    pub edges: usize,
    /// Vertices written, including vertices STEP needs that the body lacks.
    pub vertices: usize,
    /// Intersection curves written as fitted splines.
    pub fitted_curves: usize,
    /// Vertices added because STEP needs them, such as on closed edges and at poles.
    pub added_vertices: usize,
    /// Edges added because STEP needs them, such as seams of faces without a boundary.
    pub added_edges: usize,
    /// Faces of the exact body that export-only repairs left out of the file, such as a
    /// planar step too narrow for a STEP reader. The exact body keeps them.
    pub removed_faces: usize,
    /// Edges of the exact body that those repairs left out of the file.
    pub removed_edges: usize,
    /// Warnings the backend attached to the export, in words. Empty for a clean export.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// A STEP AP242 file and its report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StepExport {
    /// The file's text.
    pub text: String,
    /// What was written and its proven deviation.
    pub report: StepReport,
}

/// Which backend is answering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendInfo {
    /// Short backend name, currently "catallax".
    pub name: String,
    /// The backend's kernel version, such as an upstream commit.
    pub version: String,
}

/// The outcome of a successful operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpResult {
    /// The new body.
    pub body: BodyId,
    /// What the operation did; also the last entry of `history(body)`.
    pub record: HistoryRecord,
}

/// Why a kernel call failed. A failed call never changes an existing body.
#[derive(Clone, Debug, PartialEq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "error", content = "detail", rename_all = "snake_case")]
pub enum KernelError {
    /// The handle is not, or no longer, owned by this kernel.
    #[error("unknown body {0}")]
    UnknownBody(BodyId),
    /// A face id is not on the given body.
    #[error("face {face} is not on {body}")]
    UnknownFace {
        /// The body searched.
        body: BodyId,
        /// The missing face.
        face: FaceId,
    },
    /// An edge id is not on the given body.
    #[error("edge {edge} is not on {body}")]
    UnknownEdge {
        /// The body searched.
        body: BodyId,
        /// The missing edge.
        edge: EdgeId,
    },
    /// The profile failed its structural check.
    #[error("bad profile: {0}")]
    Profile(#[from] ProfileError),
    /// An argument is out of range or does not fit the operation.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The kernel reported a failure.
    #[error("{op} failed in the kernel (status {code}): {message}")]
    Failed {
        /// The operation.
        op: OpKind,
        /// The kernel's status code.
        code: i64,
        /// The kernel's or backend's explanation.
        message: String,
    },
    /// The kernel reported success, but the result failed validation.
    #[error("{op} reported success but produced an invalid body: {}", issues.join("; "))]
    InvalidResult {
        /// The operation.
        op: OpKind,
        /// What the validation found.
        issues: Vec<String>,
    },
    /// A query, such as tessellation or mass properties, failed.
    #[error("{query} failed (status {code}): {message}")]
    QueryFailed {
        /// Which query.
        query: String,
        /// The kernel's status code.
        code: i64,
        /// The kernel's or backend's explanation.
        message: String,
    },
    /// The named spline rendering request exhausted its unchanged work allowance.
    #[error(
        "Spline display limit (SD-DISPLAY-01): The 0.01 mm surface check stopped after {used} work units. The application limit is {limit} units. The existing part is retained"
    )]
    DisplayLimit {
        /// Work consumed before the refused reservation.
        used: u64,
        /// The application's request allowance.
        limit: u64,
    },
    /// The backend does not implement this operation or input.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// The backend cannot run, for example because its kernel thread stopped.
    #[error("kernel unavailable: {0}")]
    Unavailable(String),
}

/// Shorthand for kernel results.
pub type KernelResult<T> = Result<T, KernelError>;

/// The operations and queries SolidDraft needs from a geometry kernel.
///
/// Implementations are thread-safe; a backend that needs a single kernel thread
/// serialises calls internally.
pub trait Kernel: Send + Sync {
    /// Reads an open mesh or point cloud without turning it into a solid.
    fn scan_reference(&self, _source: &scan::ScanSource) -> KernelResult<scan::ScanReference> {
        Err(KernelError::Unsupported("Coming soon".into()))
    }

    /// Fits supported analytic surfaces, retaining both sampled deviation maps.
    fn fit_scan(&self, _source: &scan::ScanSource) -> KernelResult<scan::ScanFit> {
        Err(KernelError::Unsupported("Coming soon".into()))
    }

    /// Rebuilds the exact saved recipe without fitting the source again.
    fn build_scan_fit(&self, _tag: &OpTag, _source: &scan::ScanSource, _fit: &scan::ScanFit) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("Coming soon".into()))
    }

    /// Compares two solids at their accepted assembly placements without changing either.
    /// Refused quantities remain independent of successful answers.
    /// Clearance uses its shortest round-trip decimal, like authored lengths.
    fn body_relation(&self, _a: PlacedBody, _b: PlacedBody, _clearance: Option<f64>) -> KernelResult<BodyRelation> {
        Err(KernelError::Unsupported("Exact checks between placed parts are unavailable".into()))
    }

    /// Which backend this is.
    fn backend(&self) -> BackendInfo;

    /// A user-facing limitation of the current body, if any.
    fn body_notice(&self, _body: BodyId) -> Option<&'static str> {
        None
    }

    /// Registers a closed profile for an extrusion or revolution.
    /// Catallax defers profile geometry
    /// until a sweep and reports sheet queries as unsupported.
    fn create_body_from_profile(&self, tag: &OpTag, profile: &Profile) -> KernelResult<OpResult>;

    /// Sweeps a profile body along a straight line into a solid.
    fn extrude(&self, tag: &OpTag, profile: BodyId, params: &Extrude) -> KernelResult<OpResult>;

    /// Turns a profile body about an axis into a solid.
    fn revolve(&self, tag: &OpTag, profile: BodyId, params: &Revolve) -> KernelResult<OpResult>;
    /// Retains the exact boundary of a planar face as a profile for a later operation.
    fn create_profile_from_face(&self, _tag: &OpTag, _body: BodyId, _face: FaceId) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("This face cannot be used as a loft profile yet".into()))
    }

    /// Joins ordered profile bodies with exact ruled or smooth side faces.
    fn loft(&self, _tag: &OpTag, _profiles: &[BodyId], _shape: LoftShape) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("Loft is coming soon.".into()))
    }

    /// Sweeps a polygon profile along lines and arcs, or one fixed rational spline.
    fn sweep(&self, _tag: &OpTag, _profile: BodyId, _params: &Sweep) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("Sweep is coming soon.".into()))
    }

    /// Combines two solids. Both inputs stay as they are.
    fn boolean(&self, tag: &OpTag, op: BooleanOp, target: BodyId, tool: BodyId) -> KernelResult<OpResult>;

    /// Rounds the given edges with a circular cross section of `radius`.
    fn fillet(&self, tag: &OpTag, body: BodyId, edges: &[EdgeId], radius: f64) -> KernelResult<OpResult>;

    /// Bevels the given edges with a flat cross section `distance` wide.
    fn chamfer(&self, tag: &OpTag, body: BodyId, edges: &[EdgeId], distance: f64) -> KernelResult<OpResult>;

    /// Bevels edges with independent setbacks on the two adjacent faces. `None`
    /// uses the first distance on both faces, as in [`Kernel::chamfer`].
    fn chamfer_distances(
        &self,
        tag: &OpTag,
        body: BodyId,
        edges: &[EdgeId],
        distance: f64,
        other_distance: Option<f64>,
    ) -> KernelResult<OpResult> {
        if other_distance.is_some_and(|other| other != distance) {
            return Err(KernelError::InvalidInput("two-distance chamfer is unavailable".into()));
        }
        self.chamfer(tag, body, edges, distance)
    }

    /// Hollows a solid to wall `thickness`, leaving `open_faces` open.
    fn shell(&self, tag: &OpTag, body: BodyId, open_faces: &[FaceId], thickness: f64) -> KernelResult<OpResult>;

    /// Opens every face of `body`, an explicit choice rather than a list of picked faces.
    /// Nothing remains, so a success has no result body, and the part carries on without
    /// one. A refusal leaves `body` as it was.
    fn shell_all_faces(&self, _body: BodyId, _thickness: f64) -> KernelResult<()> {
        Err(KernelError::Unsupported("Opening every face is coming soon.".into()))
    }

    /// Offsets the body's boundary by a signed distance. Positive grows the body.
    fn offset(&self, _tag: &OpTag, _body: BodyId, _distance: f64) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("Offset is coming soon.".into()))
    }

    /// Makes a solid from the selected faces at a constant thickness.
    fn thicken(&self, _tag: &OpTag, _body: BodyId, _params: &Thicken) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("Thicken is coming soon.".into()))
    }

    /// Repeats only the operation graph between a feature's input and result.
    fn pattern_feature(
        &self,
        _tag: &OpTag,
        _body: BodyId,
        _source: BodyId,
        _before: Option<BodyId>,
        _layout: &PatternLayout,
    ) -> KernelResult<OpResult> {
        Err(KernelError::Unsupported("Feature patterns are coming soon.".into()))
    }

    /// Sweeps a profile along a straight line until it meets `target` as `limit` says,
    /// and returns the swept solid alone; the caller joins or cuts it. `direction`
    /// `None` uses the profile plane's normal.
    fn extrude_to(
        &self,
        tag: &OpTag,
        profile: BodyId,
        target: BodyId,
        direction: Option<Vec3>,
        limit: &ExtrudeLimit,
    ) -> KernelResult<OpResult> {
        let _ = (tag, profile, target, direction, limit);
        Err(KernelError::Unsupported("extrude_to".into()))
    }

    /// Drills holes into a solid and returns the drilled solid.
    fn hole(&self, tag: &OpTag, body: BodyId, params: &Hole) -> KernelResult<OpResult> {
        let _ = (tag, body, params);
        Err(KernelError::Unsupported("hole".into()))
    }

    /// Copies a solid along a layout and unites the copies with it.
    fn pattern(&self, tag: &OpTag, body: BodyId, layout: &PatternLayout) -> KernelResult<OpResult> {
        let _ = (tag, body, layout);
        Err(KernelError::Unsupported("pattern".into()))
    }

    /// Reflects a solid in a plane, keeping or dropping the original.
    fn mirror(&self, tag: &OpTag, body: BodyId, params: &Mirror) -> KernelResult<OpResult> {
        let _ = (tag, body, params);
        Err(KernelError::Unsupported("mirror".into()))
    }

    /// Tilts faces of a solid away from a pull direction about a neutral plane.
    fn draft(&self, tag: &OpTag, body: BodyId, faces: &[FaceId], params: &Draft) -> KernelResult<OpResult> {
        let _ = (tag, body, faces, params);
        Err(KernelError::Unsupported("draft".into()))
    }

    /// Compiles a terminal lattice and its unproved display. Labelled export certifies
    /// closedness, connectivity and the requested two-sided distance bound.
    fn lattice(&self, tag: &OpTag, body: BodyId, params: &Lattice) -> KernelResult<OpResult> {
        let _ = (tag, body, params);
        Err(KernelError::Unsupported("lattice".into()))
    }

    /// Compiles a terminal gyroid and its unproved display. Labelled export certifies
    /// closedness, connectivity and the requested two-sided distance bound.
    fn gyroid(&self, tag: &OpTag, body: BodyId, params: &Gyroid) -> KernelResult<OpResult> {
        let _ = (tag, body, params);
        Err(KernelError::Unsupported("gyroid".into()))
    }

    /// Certifies an active field and writes its labelled STL.
    fn export_field_stl(&self, body: BodyId) -> KernelResult<FieldExport> {
        let _ = body;
        Err(KernelError::Unsupported("labelled field export".into()))
    }

    /// Proves unit-density moments of an active field's own definition to `accuracy`.
    /// Only an explicit request computes them; no preview or mesh contributes.
    fn field_mass(&self, body: BodyId, accuracy: &FieldMassAccuracy) -> KernelResult<FieldMass> {
        let _ = (body, accuracy);
        Err(KernelError::Unsupported("field mass".into()))
    }

    /// Writes an active field's explicit mass stand-in STEP: its source envelope as a
    /// massless surface and unit-density proxy solids within the field's enclosures.
    fn export_field_stand_in(&self, body: BodyId, accuracy: &FieldMassAccuracy) -> KernelResult<StandInExport> {
        let _ = (body, accuracy);
        Err(KernelError::Unsupported("field mass stand-in".into()))
    }

    /// Independently constructs every recipe prefix with both representations.
    fn independent_check(&self, body: BodyId) -> KernelResult<IndependentCheck> {
        let _ = body;
        Err(KernelError::Unsupported("independent part check".into()))
    }

    /// Qualified print geometry and its certificate, when the backend supports it.
    fn print_mesh(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Option<PrintMesh>> {
        let _ = (body, params);
        Ok(None)
    }

    /// Exports a field at an explicit accuracy without changing its saved settings.
    fn export_field_stl_at(&self, body: BodyId, accuracy_mm: f64) -> KernelResult<FieldExport> {
        let _ = (body, accuracy_mm);
        Err(KernelError::Unsupported("field export at an explicit accuracy".into()))
    }

    /// Triangulates a body within its supported controls. The screen uses [`preview`](Self::preview).
    fn tessellate(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Mesh>;

    /// Triangulates a body once per entry of `levels`, in order, as one call. Each mesh
    /// is the one [`tessellate`](Self::tessellate) gives for that entry. A backend reads
    /// the body once for all of them; over a transport it is one round trip. The default
    /// calls `tessellate` per entry.
    fn tessellate_levels(&self, body: BodyId, levels: &[TessellationParams]) -> KernelResult<Vec<Mesh>> {
        levels.iter().map(|p| self.tessellate(body, p)).collect()
    }

    /// Samples every edge of a body along its exact curve, one polyline per edge in
    /// topology order. `params.curve_angle_deg` bounds the turn between samples and
    /// `params.chord_height`, when set, the distance from chord to curve.
    fn tessellate_edges(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>>;

    /// The screen result under a backend's named rendering rule. Its stated level may
    /// differ from viewport refinement controls; exports use certified tessellation.
    /// Backends without a named rule retain their existing preview behavior.
    fn rendering(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Mesh> {
        self.preview(body, params)
    }

    /// Physical picking edges of the named rendering result, or the ordinary preview.
    fn rendering_edges(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        self.preview_edges(body, params)
    }

    /// Measured rendering evidence for this immutable body, never an export certificate.
    fn display_bound(&self, _body: BodyId) -> KernelResult<Option<DisplayBound>> {
        Ok(None)
    }

    /// A display mesh for the screen: the viewport and drawing views. A backend may
    /// return unproved rendering, with a measured bound or no claimed deviation.
    /// Exports and proven results use [`tessellate`](Self::tessellate).
    /// The default returns `tessellate`.
    fn preview(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Mesh> {
        self.tessellate(body, params)
    }

    /// Runs one feature build and reports deterministic work units, when supported.
    /// Nested calls keep the caller's allowance. This excludes display and export work.
    fn measure_work(&self, run: &mut dyn FnMut()) -> KernelResult<Option<u64>> {
        run();
        Ok(None)
    }

    /// Prepares exact-proof work when no early representation is requested.
    /// Nested rebuild operations receive the synchronous rebuild's allowances.
    fn reserve_proof_work(&self) -> KernelResult<Box<dyn ProofWork>> {
        Err(KernelError::Unsupported("reserved background proof is unavailable in this backend".into()))
    }

    /// Builds a definition-native display before rebuilding the exact model.
    /// Edits name existing operation tags in `body`. Construction has its own
    /// reserved allowance and exact proof keeps the synchronous rebuild's
    /// allowances. No provisional face id is an exact handle.
    /// The default explicitly refuses this early representation.
    fn construction_edit(
        &self,
        _body: BodyId,
        _edits: &[ScalarEdit],
        _params: &TessellationParams,
    ) -> KernelResult<ConstructionEdit> {
        Err(KernelError::Unsupported("definition-native construction preview is unavailable in this backend".into()))
    }

    /// [`preview`](Self::preview) once per entry of `levels`, in order, as one call, the
    /// way [`tessellate_levels`](Self::tessellate_levels) is for `tessellate`. The
    /// default calls `preview` per entry.
    fn preview_levels(&self, body: BodyId, levels: &[TessellationParams]) -> KernelResult<Vec<Mesh>> {
        levels.iter().map(|p| self.preview(body, p)).collect()
    }

    /// Edge polylines matching [`preview`](Self::preview), with no claimed bounds. The
    /// default returns [`tessellate_edges`](Self::tessellate_edges).
    fn preview_edges(&self, body: BodyId, params: &TessellationParams) -> KernelResult<Vec<EdgePolyline>> {
        self.tessellate_edges(body, params)
    }

    /// Volume, area, centroid and inertia.
    fn mass_properties(&self, body: BodyId) -> KernelResult<MassProperties>;

    /// Already computed properties of this immutable body, without starting a query.
    /// Unmeasured bodies and backends without a retained result return `None`.
    fn cached_mass_properties(&self, _body: BodyId) -> Option<MassProperties> {
        None
    }

    /// Checks a body. Never trusts the status of the operation that made it.
    fn validate(&self, body: BodyId) -> KernelResult<Validation>;

    /// Faces and edges with their ids and basic geometry. Rebuilds and displays read it,
    /// so a backend may estimate centroids, boxes, lengths and midpoints from its display.
    fn topology(&self, body: BodyId) -> KernelResult<Topology>;

    /// The faces of a body in no promised order, without building its display mesh. A
    /// rebuild follows a reference through an intermediate body with this, and reads
    /// [`topology`](Self::topology) for whatever these facts cannot answer. A backend
    /// with no cheaper answer refuses, and callers read `topology`.
    fn face_carriers(&self, _body: BodyId) -> KernelResult<Vec<FaceCarrier>> {
        Err(KernelError::Unsupported("exact face carriers".into()))
    }

    /// [`topology`](Self::topology) with the backend's proven centroids, boxes, edge
    /// lengths and midpoints, where it has them. Only an explicit measurement asks for
    /// it. The default returns `topology`.
    fn measured_topology(&self, body: BodyId) -> KernelResult<Topology> {
        self.topology(body)
    }

    /// Proven length of one trimmed edge. Display estimates cannot answer this query.
    fn edge_length(&self, _body: BodyId, _edge: EdgeId) -> KernelResult<Measurement> {
        Err(KernelError::Unsupported("proven edge length".into()))
    }

    /// Proven area of one trimmed face.
    fn face_area(&self, _body: BodyId, _face: FaceId) -> KernelResult<Measurement> {
        Err(KernelError::Unsupported("proven face area".into()))
    }

    /// Proven arc-length midpoint of one edge. Display polylines and pick rays cannot
    /// answer this query.
    fn edge_midpoint(&self, _body: BodyId, _edge: EdgeId) -> KernelResult<PointProof> {
        Err(KernelError::Unsupported("proven edge midpoint".into()))
    }

    /// Proven scalar between two entities of one body, in its part frame. A domain the
    /// backend cannot prove is refused, never estimated.
    fn measure_pair(
        &self,
        _body: BodyId,
        _a: PairEndpoint,
        _b: PairEndpoint,
        _quantity: PairQuantity,
    ) -> KernelResult<Measurement> {
        Err(KernelError::Unsupported("proven pair measurement".into()))
    }

    /// The exact plane a face lies on, for assembly mates: an axis-aligned plane with a
    /// rational offset, read from the kernel's own definition. Any other face is refused.
    fn face_support_plane(&self, _body: BodyId, _face: FaceId) -> KernelResult<ExactAxisPlane> {
        Err(KernelError::Unsupported("proven support plane".into()))
    }

    /// What a face, edge or vertex supports, for assembly mates: a plane, an axis or a
    /// point, read exactly from the kernel's own definition. Directions must lie on world
    /// axes and coordinates must be rational; anything else is refused. The default reads
    /// planar faces through [`face_support_plane`](Self::face_support_plane) and refuses
    /// the rest.
    fn entity_support(&self, body: BodyId, entity: SupportEntity) -> KernelResult<ExactSupport> {
        match entity {
            SupportEntity::Face(face) => self.face_support_plane(body, face).map(ExactSupport::Plane),
            _ => Err(KernelError::Unsupported("proven axes and points".into())),
        }
    }

    /// Encloses native analytic definitions for bounded assembly mates. The exact query
    /// above keeps its exact-or-refused contract.
    fn entity_support_enclosed(&self, body: BodyId, entity: SupportEntity) -> KernelResult<EnclosedSupport> {
        let _ = (body, entity);
        Err(KernelError::Unsupported("enclosed analytic support".into()))
    }

    /// The records of every operation that produced `body`, oldest first.
    fn history(&self, body: BodyId) -> KernelResult<Vec<HistoryRecord>>;

    /// Frees a body. Its handle becomes unknown.
    fn release(&self, body: BodyId) -> KernelResult<()>;

    /// About how many bytes the backend keeps for `body`, for callers that budget the
    /// bodies they keep. An estimate, not a measurement. The default reports
    /// `Unsupported`.
    fn retained_bytes(&self, body: BodyId) -> KernelResult<u64> {
        let _ = body;
        Err(KernelError::Unsupported("retained_bytes".into()))
    }

    // Drawing queries. Each has a default that reports `Unsupported`, so a backend can
    // add them one at a time; [`drawing::mesh_hlr`] gives any backend approximate views.

    /// The curves a parallel view of `body` shows, each marked visible or hidden and
    /// tagged with the edge or face it comes from.
    fn view_curves(&self, body: BodyId, spec: &drawing::ViewSpec) -> KernelResult<drawing::ViewCurves> {
        let _ = (body, spec);
        Err(KernelError::Unsupported("view_curves".into()))
    }

    /// The outline of `body` seen from `direction` (towards the eye), without
    /// visibility: the silhouettes of curved faces, tagged with their face, and any
    /// edges the backend finds on the outline, tagged with the edge.
    fn silhouette_curves(&self, body: BodyId, direction: Vec3) -> KernelResult<Vec<drawing::ViewCurve>> {
        let _ = (body, direction);
        Err(KernelError::Unsupported("silhouette_curves".into()))
    }

    /// Where `plane` cuts `body`, each curve tagged with the face it lies on. All are
    /// marked visible.
    fn section_curves(&self, body: BodyId, plane: &Plane) -> KernelResult<Vec<drawing::ViewCurve>> {
        let _ = (body, plane);
        Err(KernelError::Unsupported("section_curves".into()))
    }

    /// A section view: `body` cut by the plane through `spec.origin` facing the eye,
    /// seen from the cut side, with the regions where the plane meets material. Every
    /// id in the result is an entity of `body`.
    ///
    /// The default subtracts a half-space box with [`boolean`](Self::boolean) and reads
    /// the cut body with [`view_curves`](Self::view_curves), [`topology`](Self::topology)
    /// and [`face_loops`](Self::face_loops); see [`drawing::section::by_half_space`].
    fn section_view(&self, body: BodyId, spec: &drawing::SectionSpec) -> KernelResult<drawing::SectionView> {
        drawing::section::by_half_space(self, body, spec)
    }

    /// The exact geometry of an edge, from its start to its end.
    fn edge_curve(&self, body: BodyId, edge: EdgeId) -> KernelResult<drawing::Curve3> {
        let _ = (body, edge);
        Err(KernelError::Unsupported("edge_curve".into()))
    }

    /// The boundary loops of a face, outer loop first, each a chain of exact edge curves
    /// in loop order.
    fn face_loops(&self, body: BodyId, face: FaceId) -> KernelResult<Vec<Vec<drawing::Curve3>>> {
        let _ = (body, face);
        Err(KernelError::Unsupported("face_loops".into()))
    }

    /// `curve` as lines and circular arcs within `tolerance`, in order, for exporters
    /// that cannot carry splines.
    fn lines_and_arcs(&self, curve: &drawing::Curve3, tolerance: f64) -> KernelResult<Vec<drawing::Curve3>> {
        let _ = (curve, tolerance);
        Err(KernelError::Unsupported("lines_and_arcs".into()))
    }

    // Exchange.

    /// Measures an imported STEP file and classifies every gap, moving no geometry. The
    /// default reports `Unsupported`.
    fn propose_import(&self, source: &[u8], options: &ImportOptions) -> KernelResult<ProposalSet> {
        let _ = (source, options);
        Err(KernelError::Unsupported("propose_import".into()))
    }

    /// Reads a STEP file as a body: automatic repairs and accepted decisions applied, a
    /// declined proposal leaving the file's own geometry. The default reports
    /// `Unsupported`.
    fn import_step(&self, tag: &OpTag, request: &ImportStepRequest<'_>) -> KernelResult<ImportStepResult> {
        let _ = (tag, request);
        Err(KernelError::Unsupported("import_step".into()))
    }

    /// Measures the explicitly selected source format without changing its geometry.
    fn propose_import_source(
        &self,
        format: ImportFormat,
        source: &[u8],
        options: &ImportOptions,
    ) -> KernelResult<ProposalSet> {
        match format {
            ImportFormat::Step => self.propose_import(source, options),
            ImportFormat::Iges => Err(KernelError::Unsupported("IGES import".into())),
            ImportFormat::Mesh => Err(KernelError::Unsupported("mesh solid import".into())),
        }
    }

    /// Stable entity keys in the original STEP or IGES bytes, before repair.
    fn source_solids(&self, format: ImportFormat, source: &[u8]) -> KernelResult<Vec<String>> {
        let _ = (format, source);
        Err(KernelError::Unsupported("source solid selection".into()))
    }

    /// Builds or replays the explicitly selected source format and its repair decisions.
    fn import_source(
        &self,
        format: ImportFormat,
        tag: &OpTag,
        request: &ImportStepRequest<'_>,
    ) -> KernelResult<ImportStepResult> {
        match format {
            ImportFormat::Step => self.import_step(tag, request),
            ImportFormat::Iges => Err(KernelError::Unsupported("IGES import".into())),
            ImportFormat::Mesh => Err(KernelError::Unsupported("mesh solid import".into())),
        }
    }

    /// Writes exact supported surfaces as IGES 5.3, refusing unsupported definitions
    /// and bounds above the requested tolerance before returning any file.
    fn export_iges(&self, body: BodyId, options: &IgesOptions) -> KernelResult<IgesExport> {
        let _ = (body, options);
        Err(KernelError::Unsupported("export_iges".into()))
    }

    /// Writes a solid as a STEP AP242 file, with a proven bound on the distance between
    /// the written and the exact geometry. A bound over `options.tolerance` is reported,
    /// not hidden: the report says so and carries the backend's warning.
    fn export_step(&self, body: BodyId, options: &StepOptions) -> KernelResult<StepExport> {
        let _ = (body, options);
        Err(KernelError::Unsupported("export_step".into()))
    }
}

/// How the sides of a loft pass through its ordered profiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoftShape {
    /// Straight between each consecutive pair of profiles.
    #[default]
    Ruled,
    /// A smooth curve through every profile.
    Smooth,
}
