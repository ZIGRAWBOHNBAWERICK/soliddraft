//! Exact loft recipes and plain messages for Catallax's typed refusals.

use catallax_recipe::{Number, Operation};
use catallax_shapes::{Curve, PointDef, Surface};
use catallax_topology::{Loop, Sense};
use serde_json::{Value, json};
use soliddraft_kernel_api::*;

use crate::Solid;

pub(super) fn refusal(code: &str) -> KernelError {
    let message = match code {
        "recipe.loft-sections" | "loft.sections" => "Loft needs at least two profiles in order",
        "recipe.loft-starts" | "recipe.loft-loops" | "loft.incompatible-sections" | "loft.incompatible-nesting" => {
            "Loft profiles need matching closed regions and holes in the same order"
        }
        "loft.admissibility-unproven" => {
            "Loft could not verify that these profiles form a solid without crossing or folding. Try simpler profiles or change their order"
        }
        "loft.legacy-parameterization" => "This loft uses an older definition. Create the loft again",
        "app.resource-limit" | "numbers.resource-limit" => {
            "Loft could not finish checking these profiles. Try fewer or simpler profiles"
        }
        _ => "Loft could not build a checked solid from these profiles. Try simpler profiles or change their order",
    };
    KernelError::InvalidInput(message.into())
}

/// Chooses correspondence starts from the converted sketches, never from display edges.
pub(super) fn operation(
    profiles: &[String],
    shape: LoftShape,
    recipe: &catallax_recipe::Recipe,
) -> KernelResult<Operation> {
    let mut sections = Vec::new();
    for profile in profiles {
        let sketch = recipe
            .steps
            .iter()
            .find(|s| s.id.as_str() == profile)
            .ok_or_else(|| refusal("loft.incompatible-sections"))?;
        let Operation::Sketch { loops, .. } = &sketch.op else {
            return Err(refusal("loft.incompatible-sections"));
        };
        let start: Vec<_> = loops.iter().filter_map(|lp| lp.0.first().map(|s| s.id())).collect();
        sections.push(json!({"profile":profile,"start":start}));
    }
    serde_json::from_value(json!({"op":"loft","sections":sections,"shape":shape}))
        .map_err(|_| refusal("loft.incompatible-sections"))
}

/// Retains rational polygon and full-circle boundaries without using a display mesh.
pub(super) fn face_profile(solid: &Solid, body: BodyId, face: FaceId) -> KernelResult<Value> {
    let unsupported = || {
        KernelError::InvalidInput(
        "Loft needs a planar face with an exact polygon or circle boundary parallel to a reference plane. Use a closed sketch for this profile".into())
    };
    let id = solid
        .ids
        .faces
        .iter()
        .find_map(|(native, app)| (*app == face).then_some(*native))
        .ok_or(KernelError::UnknownFace { body, face })?;
    let model = &solid.app.checkpoint.built().model;
    let f = model.face(id);
    let Surface::Plane(plane) = model.geom().surface(f.surface) else { return Err(unsupported()) };
    let normal = [&plane.normal.x, &plane.normal.y, &plane.normal.z];
    let axis = (0..3)
        .find(|i| {
            normal[*i].rational_ref().is_some_and(|v| !v.is_zero())
                && (0..3).all(|j| j == *i || normal[j].rational_ref().is_some_and(|v| v.is_zero()))
        })
        .ok_or_else(unsupported)?;
    let origins = [&plane.origin.x, &plane.origin.y, &plane.origin.z];
    let offset = Number::new(origins[axis].rational_ref().ok_or_else(unsupported)?.clone());
    let (rotation, translation) = match axis {
        0 => (json!(["1", "0", "1", "0"]), json!([offset, "0", "0"])),
        1 => (json!(["1", "-1", "0", "0"]), json!(["0", offset, "0"])),
        _ => (json!(["1", "0", "0", "0"]), json!(["0", "0", offset])),
    };
    let project = |p: &catallax_shapes::Point3| -> KernelResult<Value> {
        let q = [&p.x, &p.y, &p.z].map(|v| v.rational_ref().cloned());
        let [Some(x), Some(y), Some(z)] = q else { return Err(unsupported()) };
        let xy = match axis {
            0 => [-z, y],
            1 => [x, -z],
            _ => [x, y],
        };
        Ok(json!(xy.map(Number::new)))
    };
    let mut loops = Vec::new();
    for (li, boundary) in f.loops.iter().enumerate() {
        let Loop::Coedges(coedges) = model.loop_(*boundary) else { return Err(unsupported()) };
        let mut segments = Vec::new();
        for (si, coedge) in coedges.iter().enumerate() {
            let c = model.coedge(*coedge);
            let e = model.edge(c.edge);
            let name = format!("l{li}s{si}");
            match model.geom().curve(e.curve) {
                Curve::Line(_) => {
                    let end = if c.sense == Sense::Forward { e.end } else { e.start }.ok_or_else(unsupported)?;
                    let PointDef::Explicit(point) = model.geom().point(model.vertex(end).point) else {
                        return Err(unsupported());
                    };
                    segments.push(json!({"kind":"line","id":name,"to":project(point)?}));
                }
                Curve::Circle(circle) if e.start.is_none() && e.end.is_none() => {
                    let radius = circle.radius_sq.sqrt();
                    let radius = Number::new(radius.rational_ref().ok_or_else(unsupported)?.clone());
                    segments.push(json!({"kind":"circle","id":name,"centre":project(&circle.center)?,"radius":radius}));
                }
                _ => return Err(unsupported()),
            }
        }
        loops.push(segments);
    }
    if loops.is_empty() {
        return Err(unsupported());
    }
    Ok(json!({"op":"sketch","plane":{"rotation":rotation,"translation":translation},"loops":loops}))
}
