//! Converts a shell request into an origin-named Catallax operation.
//!
//! The caller resolves each selected handle against the input solid. This module
//! neither measures geometry nor changes the body arena; construction and proof
//! remain with the existing native-operation application path.

use catallax_recipe::{FaceRef, Number, Operation, StepId};
use soliddraft_kernel_api::{FaceId, KernelError, KernelResult};

/// Preserve selected face origins and the application's round-trip decimal.
/// An empty selection requests a closed shell, as in the kernel API.
pub(crate) fn operation(
    body: StepId,
    open_faces: &[FaceId],
    thickness: f64,
    mut resolve: impl FnMut(FaceId) -> KernelResult<FaceRef>,
) -> KernelResult<Operation> {
    let thickness = wall(thickness)?;
    let open = open_faces.iter().map(|face| resolve(*face)).collect::<KernelResult<Vec<_>>>()?;
    Ok(Operation::Shell { body, open, thickness })
}

/// Opens every face with the one reference `{}`, never a list of the faces' origins, so
/// the selection means the same after any earlier feature changes which faces exist.
pub(crate) fn all_faces(body: StepId, thickness: f64) -> KernelResult<Operation> {
    Ok(Operation::Shell { body, open: vec![FaceRef::all()], thickness: wall(thickness)? })
}

fn wall(thickness: f64) -> KernelResult<Number> {
    if !thickness.is_finite() || thickness <= 0.0 {
        return Err(KernelError::InvalidInput("shell thickness must be finite and positive".into()));
    }
    Number::parse(&thickness.to_string()).map_err(|e| KernelError::InvalidInput(format!("shell thickness: {e}")))
}
