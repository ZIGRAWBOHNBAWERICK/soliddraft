//! Proven planes, axes and points for assembly mates.
//!
//! A display centroid or a rounded reference point cannot prove a mate. These queries read
//! the plane a face lies on, the axis of a cylinder, cone, circle or line, and the centre
//! of a sphere or the point of a vertex from the kernel's own definitions, exactly. They
//! cover axis-aligned directions with rational coordinates and refuse everything else.
//! The separate enclosed-support query supplies rational bounds for general analytic
//! definitions, explicitly labelled as bounded by the assembly solver.

use serde::{Deserialize, Serialize};

use crate::{EdgeId, FaceId, VertexId};

/// An exact rational number, in lowest terms with a positive denominator. Written as
/// decimal integers so no precision is lost on the way through this interface.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExactRational {
    /// The numerator.
    pub numerator: String,
    /// The denominator, greater than zero.
    pub denominator: String,
}

/// A coordinate axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// The x axis.
    X,
    /// The y axis.
    Y,
    /// The z axis.
    Z,
}

impl Axis {
    /// The axis's index in `[x, y, z]`.
    pub fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }
}

/// The plane `u · p = offset` a face lies on, where `u` is the unit axis the face's
/// outward side looks along. A negative direction measures the offset along that
/// negative axis, so the bottom of a block at `z = 0` has `u = -z` and offset 0, and a
/// face at `x = -20` looking along `-x` has offset 20.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExactAxisPlane {
    /// The axis the outward normal lies on.
    pub axis: Axis,
    /// True when the outward normal points along the positive axis.
    pub positive: bool,
    /// Where the plane lies along the outward normal, in millimetres.
    pub offset: ExactRational,
}

impl ExactAxisPlane {
    /// The outward unit normal as coordinates, each exactly -1, 0 or 1.
    pub fn normal(&self) -> [i8; 3] {
        let mut n = [0; 3];
        n[self.axis.index()] = if self.positive { 1 } else { -1 };
        n
    }
}

/// The entity of a body a support query reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum SupportEntity {
    /// A face: a plane, cylinder, cone or sphere.
    Face(FaceId),
    /// An edge: a line or a circle.
    Edge(EdgeId),
    /// A vertex.
    Vertex(VertexId),
}

/// What a support was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportSource {
    /// A planar face.
    Plane,
    /// A cylindrical face.
    Cylinder,
    /// A conical face.
    Cone,
    /// A spherical face.
    Sphere,
    /// A straight edge.
    Line,
    /// A circular edge.
    Circle,
    /// A vertex.
    Vertex,
}

/// A radius read from a curved face or a circle.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExactRadius {
    /// The radius squared, which a kernel may hold without extracting a root.
    pub squared: ExactRational,
    /// For a face, true when its outward side looks away from the axis or centre, as on a
    /// shaft or a ball, and false when it looks towards it, as in a bore. `None` for an
    /// edge, which has no material side.
    pub outward: Option<bool>,
}

/// The line `through + t u`, where `u` is the unit axis given by `axis` and `positive`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExactAxis {
    /// The world axis the direction lies on.
    pub axis: Axis,
    /// True when the direction points along the positive axis. A cylinder's or cone's
    /// direction is the kernel's own; a line's follows its natural direction and a
    /// circle's its normal.
    pub positive: bool,
    /// A point on the line, in millimetres: a cylinder's axis origin, a cone's apex, a
    /// circle's centre or a line's origin.
    pub through: [ExactRational; 3],
    /// The radius of a cylinder or circle; `None` for a cone or a line.
    pub radius: Option<ExactRadius>,
    /// What it was read from.
    pub source: SupportSource,
}

impl ExactAxis {
    /// The unit direction as coordinates, each exactly -1, 0 or 1.
    pub fn direction(&self) -> [i8; 3] {
        let mut n = [0; 3];
        n[self.axis.index()] = if self.positive { 1 } else { -1 };
        n
    }
}

/// A point: a sphere's centre or a vertex.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExactPoint {
    /// Its coordinates, in millimetres.
    pub at: [ExactRational; 3],
    /// A sphere's radius; `None` for a vertex.
    pub radius: Option<ExactRadius>,
    /// What it was read from.
    pub source: SupportSource,
}

/// What a face, edge or vertex supports, read exactly from the kernel's definition.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExactSupport {
    /// Algebraic or off-axis native definitions, explicitly enclosed rather than exact.
    Enclosed(Box<EnclosedSupport>),
    /// A planar face's plane.
    Plane(ExactAxisPlane),
    /// The axis of a cylinder, cone or circle, or a line.
    Axis(ExactAxis),
    /// The centre of a sphere, or a vertex.
    Point(ExactPoint),
}

/// Rational endpoints enclosing a native algebraic value. Bounds are inclusive.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SupportBounds {
    /// Lower endpoint.
    pub lo: ExactRational,
    /// Upper endpoint.
    pub hi: ExactRational,
}

/// Native analytic support enclosed before any placement is applied.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum EnclosedSupport {
    /// A plane through `through`, with an outward, possibly unnormalized normal.
    Plane {
        /// Outward normal.
        normal: [SupportBounds; 3],
        /// A point on the supporting plane.
        through: [SupportBounds; 3],
    },
    /// An axis through `through`, with a possibly unnormalized direction.
    Axis {
        /// Native direction.
        direction: [SupportBounds; 3],
        /// A point on the axis.
        through: [SupportBounds; 3],
        /// Squared radius, when the support has one.
        radius_squared: Option<SupportBounds>,
        /// Material side of a curved face, absent for an edge.
        outward: Option<bool>,
    },
    /// A sphere centre or an explicit vertex.
    Point {
        /// Native coordinates.
        at: [SupportBounds; 3],
        /// Squared radius for a sphere.
        radius_squared: Option<SupportBounds>,
        /// Material side for a sphere.
        outward: Option<bool>,
    },
}
