//! Mechanical relations retain their source joints and continuous angular coordinates.
use super::{FaceEndpoint, Mate, MateId, MateRefusal, RefusalCode};
use crate::schema::Assembly;
use serde::{Deserialize, Serialize};

/// The supported kinematic motion relations. These relations do not model contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mechanism {
    /// Two rotations with a specified ratio, opposite by default.
    Gear,
    /// Translation per revolution of a pinion joint.
    RackAndPinion,
    /// Axial travel per revolution of a concentric joint.
    Screw,
}
impl Mechanism {
    pub(crate) fn kind(self) -> &'static str {
        match self {
            Self::Gear => "gear",
            Self::RackAndPinion => "rack_and_pinion",
            Self::Screw => "screw",
        }
    }
}
pub(super) fn joint(truth: &Assembly, id: MateId) -> Result<[FaceEndpoint; 2], MateRefusal> {
    match truth.known_mates().find(|m| m.id() == id) {
        Some(Mate::Hinge { axes, .. }) => Ok(axes.clone()),
        Some(Mate::Concentric { a, b, lock_rotation: None, .. }) => Ok([a.clone(), b.clone()]),
        _ => Err(MateRefusal::new(
            RefusalCode::UnsupportedMotion,
            "a mechanical rotation needs an existing Hinge or an unlocked axis-axis Concentric mate",
        )),
    }
}
pub(super) fn joint_named(truth: &Assembly, name: &str) -> Result<(MateId, [FaceEndpoint; 2]), MateRefusal> {
    let m = truth
        .known_mates()
        .find(|m| m.name() == name)
        .ok_or_else(|| MateRefusal::invalid(format!("no joint named {name}")))?;
    Ok((m.id(), joint(truth, m.id())?))
}
