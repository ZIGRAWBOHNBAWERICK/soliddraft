//! The SolidDraft part document.
//!
//! A part is an ordered list of [`Feature`]s: sketches, extrusions that add or cut
//! material, fillets and chamfers. The document stores only their definitions and
//! rebuilds them through the app-owned [`Kernel`](soliddraft_kernel_api::Kernel) trait,
//! so it works on any backend.
//!
//! - **Commands.** Every change is a [`Command`] run by [`Document::execute`], one undo
//!   entry each; [`Document::undo`] and [`Document::redo`] walk the stack. The ribbon,
//!   the assistant and scripts all go through the same commands.
//! - **Rebuild.** A command rebuilds only from the first feature it changed, and every
//!   feature build is kept by its exact inputs (see [`rebuild`]), so a rename builds
//!   nothing and undo, redo or a value put back reuse the earlier bodies. Suppressed
//!   features and features below the rollback bar pass the body through unchanged.
//! - **Persistent references.** Features name faces and edges by origin, never by kernel
//!   id (see [`reference`]). A reference that no longer resolves to exactly one matching
//!   entity stops its feature with [`FeatureStatus::RepairNeeded`] instead of guessing.
//! - **Materials and appearances.** A part's material and the appearances of the part,
//!   its features' faces and single faces, each scoped to configurations, with copies
//!   of what they use (see [`materials`]). Mass comes from the material's density.
//! - **Manufacturing information.** Datums, dimensions with limits, geometric
//!   tolerances and surface textures on persistently named faces, and the part's
//!   notes (see [`pmi`]). A dimension's size is measured on the body, never stored.
//! - **Files.** [`Document::save`] and [`Document::open`] read and write the versioned
//!   native format (see [`file`]); a saved file round-trips exactly. What the user shows
//!   and hides travels with the part, outside its truth (see [`view`]).
//!
//! Units are millimetres and degrees.

mod command;
pub mod configs;
pub mod display;
mod document;
mod evaluate;
mod exact;
mod feature;
pub mod file;
pub mod interview;
pub mod materials;
pub mod params;
pub mod pmi;
pub mod rebuild;
pub mod reference;
mod thin;
pub mod thread;
pub mod tracked;
mod value_expr;
pub mod values;
pub mod view;

pub use command::{Command, CommandError, Executed};
pub use document::{BuiltSketch, Document, FITS_BETWEEN, FaceLabel, FeatureStatus, RUNS_INTO, feature_of_tag};
pub use evaluate::{plane_on_face, shell_refusal};
pub use feature::{
    BlendFeature, BuiltinShape, Contour, DatumAxis, DatumPlane, DraftFeature, EndCondition, ExtrudeFeature, ExtrudeOp,
    ExtrudeOptions, ExtrudeStart, Feature, FeatureId, FeatureKind, GyroidFeature, HoleFeature, ImportStepFeature,
    InheritedName, LatticeFeature, LoftFeature, LoftProfile, MirrorFeature, OffsetFeature, PatternFeature, PatternSpec,
    PlaneRef, ProfileSource, PullRef, RevolveFeature, ScanFeature, ShellFeature, ShellOpenings, SketchFeature,
    SketchPlane, SweepFeature, ThickenFeature, ThinFeature, ThinSide, sha256_hex,
};
pub use file::FileError;
pub use materials::{
    AppearanceAssignment, AppearanceTable, AppearanceTarget, LostAppearance, MassError, MaterialAssignment,
    PartMaterials,
};
pub use reference::{
    Candidate, EdgeFingerprint, EdgePiece, EdgeRef, FaceFingerprint, FaceRef, Generator, NamedFace, NamingError,
    ReferenceSlot, RepairNeeded, RepairReason, VertexRef, face_refs,
};
pub use value_expr::{Expression, ValueTable};

/// The sketch types features are built from.
pub use soliddraft_sketch as sketch;
