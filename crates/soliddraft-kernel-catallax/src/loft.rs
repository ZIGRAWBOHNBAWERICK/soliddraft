//! Exact loft recipes and plain messages for Catallax's typed refusals.

use catallax_numbers::Rational;
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
        "application.work-limit" | "numbers.resource-limit" => {
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
            "Loft cannot read an exact polygon or circle profile from this face. Use a closed sketch for this profile"
                .into(),
        )
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
    let normal = &plane.normal;
    let length = normal.dot(normal).sqrt();
    let length = length.rational_ref().ok_or_else(unsupported)?;
    let [nx, ny, nz] = normal.rational_components().ok_or_else(unsupported)?;
    let quaternion = if nx.is_zero() && ny.is_zero() && nz.sign() == catallax_numbers::Sign::Negative {
        [Rational::zero(), Rational::one(), Rational::zero(), Rational::zero()]
    } else {
        [length + nz, -ny.clone(), nx.clone(), Rational::zero()]
    };
    let [w, x, y, z] = quaternion.clone();
    let rotation = catallax_shapes::Rotation::from_quaternion(w, x, y, z).map_err(|_| unsupported())?;
    let inverse = rotation.inverse();
    let translation = [&plane.origin.x, &plane.origin.y, &plane.origin.z].map(|v| v.rational_ref().cloned());
    let [Some(tx), Some(ty), Some(tz)] = translation else { return Err(unsupported()) };
    let translation = [tx, ty, tz].map(Number::new);
    let rotation = quaternion.map(Number::new);
    let project = |p: &catallax_shapes::Point3| -> KernelResult<Value> {
        let local = inverse.apply(&p.minus(&plane.origin));
        let [x, y, z] = local.rational_components().ok_or_else(unsupported)?;
        if !z.is_zero() {
            return Err(unsupported());
        }
        Ok(json!([Number::new(x.clone()), Number::new(y.clone())]))
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
                    let end = if c.sense == Sense::Same { e.end } else { e.start }.ok_or_else(unsupported)?;
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
