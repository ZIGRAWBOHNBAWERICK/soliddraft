//! Features proved from a body's boundary, without relying on feature history.
//!
//! A part made in the app knows its features from its history. An imported body has
//! none, so the kernel reads them from the faces: which lie on one cylinder, which side
//! holds material, which faces close a hole. Nothing is fitted or guessed; a face no
//! rule reads is in no feature.

use serde::{Deserialize, Serialize};

use crate::{FaceId, Vec3};

/// The features of one body.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Recognised {
    /// Round holes, each once.
    pub holes: Vec<Hole>,
    /// Groups of equal holes, as indices into `holes`. A hole is in at most one.
    pub hole_patterns: Vec<Vec<usize>>,
    /// Round bosses.
    pub bosses: Vec<Boss>,
    /// Slots with round ends.
    pub slots: Vec<Slot>,
    /// Stepped or conical mouths with a single coaxial pilot. Not repeated in `holes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compound_holes: Vec<CompoundHole>,
    /// Shallow recesses with a flat floor and straight walls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recesses: Vec<Recess>,
    /// Edge treatments with proved analytic support on both sides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_treatments: Vec<EdgeTreatment>,
}

/// A round hole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hole {
    /// Its wall: the faces of one cylinder.
    pub faces: Vec<FaceId>,
    /// Its diameter in millimetres.
    pub diameter: f64,
    /// True when no face closes it.
    pub through: bool,
    /// The distance between its two ends, when both are circles about its axis.
    pub depth: Option<f64>,
    /// The faces it opens onto.
    pub opens_onto: Vec<FaceId>,
    /// The faces that close it.
    pub bottoms: Vec<FaceId>,
}

/// A round boss.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Boss {
    /// Its wall: the faces of one cylinder.
    pub faces: Vec<FaceId>,
    /// Its diameter in millimetres.
    pub diameter: f64,
    /// Its height, when both ends are circles about its axis.
    pub height: Option<f64>,
    /// The face it stands on.
    pub base: FaceId,
    /// The face that closes it.
    pub top: FaceId,
}

/// A slot with round ends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    /// Its two flat sides.
    pub flats: [FaceId; 2],
    /// Its two round ends, each the faces of one cylinder.
    pub ends: [Vec<FaceId>; 2],
    /// Its width, the distance between the flats.
    pub width: f64,
    /// Its length over the ends.
    pub length: f64,
    /// True when no face floors it.
    pub through: bool,
}

/// A pilot and a larger head joined at a common axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompoundHole {
    /// Pilot wall faces.
    pub pilot_faces: Vec<FaceId>,
    /// Head wall faces; excludes a flat counterbore shoulder.
    pub head_faces: Vec<FaceId>,
    /// The entry face containing the head opening.
    pub entry: FaceId,
    /// The exit face or flat blind bottom.
    pub end: FaceId,
    /// Centre of the entry circle in millimetres.
    pub mouth_center: Vec3,
    /// Unit direction along the axis from the mouth into the body.
    pub direction: Vec3,
    /// Pilot diameter in millimetres.
    pub pilot_diameter: f64,
    /// Head diameter at the mouth in millimetres.
    pub head_diameter: f64,
    /// Axial distance from the entry to the exit or flat blind bottom, in millimetres.
    pub depth: f64,
    /// Axial distance from the entry to the pilot, in millimetres.
    pub head_depth: f64,
    /// True for a pilot that opens through the body.
    pub through: bool,
    /// The geometry joining the two diameters.
    pub head: CompoundHead,
}

/// Geometry of a compound hole's head.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CompoundHead {
    /// A cylindrical wall and a planar annular shoulder.
    Counterbore {
        /// The annular face.
        shoulder: FaceId,
    },
    /// A conical wall joining the pilot directly.
    Countersink {
        /// Included angle of the cone in degrees.
        angle_deg: f64,
    },
}

/// A flat-bottomed pocket or open step, shallower than its narrowest floor span.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recess {
    /// The recessed plane.
    pub floor: FaceId,
    /// Rising faces bounding the floor. Excludes exterior faces below an open side.
    pub walls: Vec<FaceId>,
    /// The coplanar faces at the top of the rising walls.
    pub opens_onto: Vec<FaceId>,
    /// Perpendicular depth in millimetres.
    pub depth: f64,
    /// True when at least one side of the floor is open.
    pub open_sided: bool,
}

/// An analytic band between two perpendicular flat supports.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeTreatment {
    /// The face between the supports.
    pub face: FaceId,
    /// The two faces joined by the band.
    pub supports: [FaceId; 2],
    /// The recognized profile and its geometric size.
    pub kind: EdgeTreatmentKind,
}

/// Profile of a recognized edge treatment; no manufacturing intent is implied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum EdgeTreatmentKind {
    /// A tangent quarter cylinder.
    Fillet {
        /// Radius in millimetres.
        radius: f64,
    },
    /// An equal-setback planar bevel.
    Chamfer {
        /// Setback along either support in millimetres.
        setback: f64,
    },
}
