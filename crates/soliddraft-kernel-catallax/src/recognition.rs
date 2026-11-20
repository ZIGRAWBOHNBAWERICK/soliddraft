//! Feature recognition through Catallax, on the app's face ids.

use catallax_measure::recognition as cx;
use soliddraft_kernel_api::recognition::{Boss, Hole, Recognised, Slot};
use soliddraft_kernel_api::{FaceId, KernelError, KernelResult};

use crate::{Solid, numerical};

/// The holes, hole patterns, bosses and slots of a solid.
pub(super) fn recognise(solid: &Solid) -> KernelResult<Recognised> {
    let built = solid.app.checkpoint.built();
    let found = numerical("recognise", |dec| {
        cx::recognise(&built.model, built.result, dec).map(catallax_numbers::Answer::exact)
    })
    .map_err(|refusal| crate::query_error("recognise", refusal))?
    .value;
    let face =
        |cat: &catallax_topology::FaceId| -> KernelResult<FaceId> {
            solid.ids.faces.get(cat).copied().ok_or_else(|| {
                KernelError::Unavailable("a recognised feature is on a face the body does not list".into())
            })
        };
    let faces = |cats: &[catallax_topology::FaceId]| cats.iter().map(face).collect::<KernelResult<Vec<FaceId>>>();
    Ok(Recognised {
        holes: found
            .holes
            .iter()
            .map(|hole| {
                Ok(Hole {
                    faces: faces(&hole.faces)?,
                    diameter: hole.diameter,
                    through: hole.through,
                    depth: hole.depth,
                    opens_onto: faces(&hole.opens_onto)?,
                    bottoms: faces(&hole.bottoms)?,
                })
            })
            .collect::<KernelResult<_>>()?,
        hole_patterns: found.hole_patterns.clone(),
        bosses: found
            .bosses
            .iter()
            .map(|boss| {
                Ok(Boss {
                    faces: faces(&boss.faces)?,
                    diameter: boss.diameter,
                    height: boss.height,
                    base: face(&boss.base)?,
                    top: face(&boss.top)?,
                })
            })
            .collect::<KernelResult<_>>()?,
        slots: found
            .slots
            .iter()
            .map(|slot| {
                Ok(Slot {
                    flats: [face(&slot.flats[0])?, face(&slot.flats[1])?],
                    ends: [faces(&slot.ends[0])?, faces(&slot.ends[1])?],
                    width: slot.width,
                    length: slot.length,
                    through: slot.through,
                })
            })
            .collect::<KernelResult<_>>()?,
    })
}
