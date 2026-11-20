//! Feature recognition: the holes, bosses and slots of a body, read off its faces.
//!
//! A part made in the app knows its features from its history. An imported body has
//! none, so the kernel reads them from the faces: which lie on one cylinder, which side
//! holds material, which faces close a hole. Nothing is fitted or guessed; a face no
//! rule reads is in no feature.

use serde::{Deserialize, Serialize};

use crate::FaceId;

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
