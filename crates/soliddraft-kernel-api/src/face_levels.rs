//! A body's faces at three nested display levels, counted before any mesh is made.

use serde::{Deserialize, Serialize};

use crate::FaceId;

/// One of the three nested display levels a backend plans for the faces of a body at
/// one set of tessellation parameters. The levels share every edge point, so two
/// neighbouring faces at different levels meet without a gap, and a finer level of a
/// face keeps the points of its coarser ones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaceLevel {
    /// Sampled to about ten times the requested chord height.
    Coarse,
    /// Sampled to about twice the requested chord height.
    Standard,
    /// The requested parameters: the face as [`Kernel::tessellate`](crate::Kernel::tessellate)
    /// meshes it.
    #[default]
    Fine,
}

impl FaceLevel {
    /// Every level, coarsest first.
    pub const ALL: [FaceLevel; 3] = [FaceLevel::Coarse, FaceLevel::Standard, FaceLevel::Fine];

    /// The level's place in [`FaceLevel::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }
}

/// How many mesh points one face has at each nested level, known before the mesh is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaceLevelPoints {
    /// The face.
    pub face: FaceId,
    /// Its points at each level, coarsest first. A point on an edge counts once in each
    /// face it borders, as a display mesh keeps the vertices of two faces apart.
    pub points: [u32; 3],
}

impl FaceLevelPoints {
    /// The face's points at `level`.
    pub fn at(&self, level: FaceLevel) -> u32 {
        self.points[level.index()]
    }
}
