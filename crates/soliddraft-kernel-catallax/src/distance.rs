//! Exact body offsets and selected-face thickening through native recipe operations.

use catallax_recipe::{FaceRef, Operation, StepId};
use soliddraft_kernel_api::{FaceId, KernelError, KernelResult, OpKind, Thicken};

pub(super) fn offset(body: StepId, distance: f64) -> KernelResult<Operation> {
    if !distance.is_finite() || distance == 0.0 {
        return Err(KernelError::InvalidInput("Offset needs a finite, nonzero distance".into()));
    }
    Ok(Operation::Offset { body, distance: crate::native::number(distance, "offset distance")? })
}

pub(super) fn thicken(
    body: StepId,
    params: &Thicken,
    mut face: impl FnMut(FaceId) -> KernelResult<FaceRef>,
) -> KernelResult<Operation> {
    if params.faces.is_empty() {
        return Err(KernelError::InvalidInput("Select at least one face to thicken".into()));
    }
    if !params.thickness.is_finite() || params.thickness <= 0.0 {
        return Err(KernelError::InvalidInput("Thicken needs a finite, positive thickness".into()));
    }
    let faces = params.faces.iter().map(|id| face(*id)).collect::<KernelResult<Vec<_>>>()?;
    let side = match params.side {
        soliddraft_kernel_api::ThickenSide::Front => catallax_recipe::ThickenSide::Front,
        soliddraft_kernel_api::ThickenSide::Back => catallax_recipe::ThickenSide::Back,
    };
    Ok(Operation::Thicken { body, faces, thickness: crate::native::number(params.thickness, "thickness")?, side })
}

pub(super) fn refusal(op: OpKind, code: &str) -> KernelError {
    let what = if op == OpKind::Offset { "Offset" } else { "Thicken" };
    let message = if matches!(code, "numbers.operation-budget" | "app.resource-limit") {
        format!("{what} needs too much work to finish. Try a smaller distance or a simpler selection")
    } else if op == OpKind::Offset {
        "Offset could not be verified for this body and distance. Try a smaller distance or a simpler body".into()
    } else {
        "These faces cannot be thickened at this thickness and side. Try one planar face or a smaller thickness".into()
    };
    KernelError::InvalidInput(message)
}
