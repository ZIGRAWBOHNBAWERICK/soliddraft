//! Exact body-pair queries on private copies at the accepted placements.

use catallax_app::queries;
use catallax_measure::{Accuracy, Enclosure};
use catallax_numbers::{Answer, Rational, Refusal};
use catallax_shapes::{Isometry, Pose, RVector3, Rotation};
use catallax_topology::{Model, transformed_body};
use serde_json::json;
use soliddraft_kernel_api::*;

use crate::{CatallaxKernel, Solid, build, numerical};

fn refused(query: &str, code: &str) -> BodyQueryRefusal {
    let reason = if code.contains("budget") || code.contains("work-limit") || code.contains("deadline") {
        "The check reached its calculation limit."
    } else if code == "app.relation-envelope" {
        "A lattice or gyroid part's material lies on its beams or sheet, not in the whole region its solid outlines."
    } else if code == "body-relation.marked" {
        "The check returned an approximation or an unresolved accuracy bound."
    } else if code.contains("distance") || code.contains("surface") || code.contains("curve") {
        "The check does not support this combination of faces and edges."
    } else {
        "The check could not prove the required geometric comparisons for these parts."
    };
    BodyQueryRefusal {
        query: query.into(),
        code: code.into(),
        message: format!("{query} was not determined. {reason}"),
    }
}

fn answer<T>(query: &str, result: Result<Answer<T>, Refusal>) -> Result<T, BodyQueryRefusal> {
    if catallax_numbers::current_operation_work().exhausted {
        return Err(refused(query, "numbers.operation-budget"));
    }
    let answer = result.map_err(|r| refused(query, r.code))?;
    if !answer.marks.is_empty() {
        return Err(refused(query, "body-relation.marked"));
    }
    Ok(answer.value)
}

/// Even a zero-width rational interval can lie between two doubles. A marked enclosure
/// is bounded, never exact.
fn measurement(e: &Enclosure, marked: bool) -> Measurement {
    let mut lo = e.lo.to_f64_nearest();
    let mut hi = e.hi.to_f64_nearest();
    if Rational::from_f64_exact(lo).is_some_and(|r| r > e.lo) {
        lo = lo.next_down();
    }
    if Rational::from_f64_exact(hi).is_some_and(|r| r < e.hi) {
        hi = hi.next_up();
    }
    Measurement {
        value: crate::convert::midpoint(e),
        lo,
        hi,
        certainty: if e.is_exact() && !marked { Certainty::Exact } else { Certainty::Bounded },
    }
}

fn motion(body: PlacedBody) -> KernelResult<Isometry> {
    let exact =
        |n| Rational::from_f64_exact(n).ok_or_else(|| KernelError::InvalidInput("A placement must be finite".into()));
    let [x, y, z, w] = body.rotation;
    let rotation = Rotation::from_quaternion(exact(w)?, exact(x)?, exact(y)?, exact(z)?)
        .map_err(|_| KernelError::InvalidInput("A placement needs a nonzero rotation".into()))?;
    let [x, y, z] = body.translation;
    Ok(Isometry::from(&Pose::from_rational(rotation, RVector3::new(exact(x)?, exact(y)?, exact(z)?))))
}

/// Both bodies in one model, and the refusal for relating them when either is only the
/// envelope of a lattice or gyroid region.
struct Operands {
    model: Model,
    bodies: [catallax_topology::BodyId; 2],
    envelope: Option<Refusal>,
}

/// Both source chains already use unique operation ids in this kernel. Evaluate their
/// union once so both handles belong to one model, retaining the longest common prefix.
fn operands(k: &CatallaxKernel, a: BodyId, b: BodyId) -> KernelResult<Operands> {
    let state = k.state()?;
    let (first, second) = (state.get(a)?, state.get(b)?);
    // Their recipes would replay as exact bodies.
    if first.approximate.is_some() || second.approximate.is_some() {
        return Err(crate::approximate::not_yet());
    }
    let mut steps = first.steps.clone();
    steps.extend(second.steps.clone());
    let prefix = [first, second]
        .into_iter()
        .filter(|s| s.steps.iter().eq(steps.iter().take(s.steps.len())))
        .max_by_key(|s| s.steps.len())
        .and_then(|s| s.solid.clone());
    let result_names = [first.result.clone(), second.result.clone()];
    let input = json!({"format":"soliddraft-recipe/1", "name":"Body check",
        "steps":steps.values().collect::<Vec<_>>(), "result":second.result});
    let sources = state.sources(&steps)?;
    drop(state);
    let files = sources.iter().map(|(sha, bytes)| (sha.clone(), &bytes[..])).collect();
    let (app, names, _, _) =
        build(&input, OpKind::Intersect, &[], prefix.as_ref().map(|s| &s.app), &files, None, &Default::default(), &[])?;
    let crate::Outcome::Exact(app) = app else { return Err(crate::approximate::not_yet()) };
    let built = app.checkpoint.built();
    // A part whose result reads a lattice or gyroid has only the region's envelope here.
    let envelope = result_names.iter().find_map(|name| queries::field_only_envelope(built, &names[name]));
    let bodies = result_names.map(|name| built.bodies[&names[&name]]);
    Ok(Operands { model: built.model.clone(), bodies, envelope })
}

fn source_faces(solid: &Solid, contact: &queries::ContactFaces, patches: &[queries::ContactPatch]) -> Vec<FaceId> {
    solid
        .ids
        .faces
        .iter()
        .filter_map(|(native, source)| {
            let origins = &solid.app.names.faces[native];
            patches
                .iter()
                .any(|patch| contact.model.face_origins(patch.face).iter().any(|o| origins.contains(o)))
                .then_some(*source)
        })
        .collect()
}

/// Whether `gap` exceeds `clearance`, decided by exact comparison: a gap equal to the
/// clearance does not exceed it. `None` when the enclosure straddles the clearance.
fn exceeds(gap: &Enclosure, clearance: &Rational) -> Option<bool> {
    if &gap.lo > clearance {
        Some(true)
    } else if &gap.hi <= clearance {
        Some(false)
    } else {
        None
    }
}

fn zero_gap(clearance: Option<f64>) -> BodyDistance {
    BodyDistance {
        distance: measurement(&Enclosure { lo: Rational::zero(), hi: Rational::zero() }, false),
        zero: true,
        exceeds_clearance: clearance.map(|_| false),
    }
}

fn no_overlap() -> OverlapVolume {
    OverlapVolume {
        volume: measurement(&Enclosure { lo: Rational::zero(), hi: Rational::zero() }, false),
        positive: Some(false),
    }
}

/// Clash, clear or unresolved, as Catallax's relation answers it for the two placed
/// solids. A clash is a proved positive overlap however wide its volume enclosure; a
/// clear pair has a proved positive gap. Anything Catallax leaves unresolved comes back
/// refused, never clear. A lattice or gyroid part is refused: its B-rep is only the
/// envelope of the region its beams or sheet fill.
pub(super) fn relation(
    k: &CatallaxKernel,
    a: PlacedBody,
    b: PlacedBody,
    clearance: Option<f64>,
) -> KernelResult<BodyRelation> {
    if !catallax_numbers::operation_budget_active() {
        return catallax_numbers::with_operation_budget(crate::APP_REQUEST_WORK, || relation(k, a, b, clearance)).0;
    }
    if clearance.is_some_and(|c| !c.is_finite() || c < 0.0) {
        return Err(KernelError::InvalidInput("A clearance must be a finite length of zero or more".into()));
    }
    let field = k.field(a.body)?.is_some() || k.field(b.body)?.is_some();
    let operands = if field { None } else { Some(operands(k, a.body, b.body)?) };
    let envelope = match &operands {
        None => Some(Refusal::new("app.relation-envelope", "a lattice or gyroid part has only its envelope")),
        Some(operands) => operands.envelope.clone(),
    };
    if let Some(refusal) = envelope {
        return Ok(BodyRelation {
            overlap: Err(refused("Overlap volume", refusal.code)),
            contact: Err(refused("Contact faces", refusal.code)),
            gap: clearance.map(|_| Err(refused("Minimum distance", refusal.code))),
        });
    }
    let Operands { mut model, bodies, .. } = operands.expect("a part without a field has operands");
    let (first, second) = (k.solid(a.body)?, k.solid(b.body)?);
    let motions = [motion(a)?, motion(b)?];
    let [a, b] = std::array::from_fn(|i| transformed_body(&mut model, bodies[i], &motions[i], Clone::clone));
    // Match the decimal conversion used to build authored dimensions.
    let threshold = clearance.and_then(|c| crate::native::number(c, "clearance").ok().map(|n| n.value().clone()));
    let related = numerical("body_relation", |dec| {
        queries::body_relation(&model, a, b, &Accuracy::default(), dec).map_err(|r| r.refusal)
    });
    let related = if catallax_numbers::current_operation_work().exhausted {
        Err(Refusal::new("numbers.operation-budget", "operation work allowance exhausted"))
    } else {
        related
    };
    // Solids a proved gap keeps apart share no face, so the imprint is not run.
    let apart = related.as_ref().is_ok_and(|r| matches!(r.value, queries::BodyRelation::Clear { .. }));
    let contact = if apart {
        Ok(BodyContactFaces { first: Vec::new(), second: Vec::new() })
    } else {
        answer(
            "Contact faces",
            numerical("contact_faces", |dec| queries::contact_faces(&model, a, b, dec).map_err(|r| r.refusal)),
        )
        .map(|contact| BodyContactFaces {
            first: source_faces(&first, &contact, &contact.first),
            second: source_faces(&second, &contact, &contact.second),
        })
    };
    let touching = contact.as_ref().is_ok_and(|c| !c.first.is_empty() || !c.second.is_empty());
    let (overlap, gap) = match related {
        Ok(answer) => {
            let marked = !answer.marks.is_empty();
            match answer.value {
                queries::BodyRelation::Clash { overlap } => (
                    Ok(OverlapVolume { volume: measurement(&overlap, marked), positive: Some(true) }),
                    Some(Ok(zero_gap(clearance))),
                ),
                queries::BodyRelation::Clear { gap } => (
                    Ok(no_overlap()),
                    Some(Ok(BodyDistance {
                        distance: measurement(&gap, marked),
                        zero: false,
                        exceeds_clearance: threshold.as_ref().and_then(|c| exceeds(&gap, c)),
                    })),
                ),
            }
        }
        // Catallax answers contact only once it has proved the overlap empty.
        Err(r) if r.code == "assembly.contact" => (Ok(no_overlap()), Some(Ok(zero_gap(clearance)))),
        Err(r) => {
            let refused_overlap = Err(refused("Overlap volume", r.code));
            // Shared faces prove a zero distance even when the overlap stays open.
            let gap = if touching {
                Some(Ok(zero_gap(clearance)))
            } else {
                clearance.map(|_| Err(refused("Minimum distance", r.code)))
            };
            (refused_overlap, gap)
        }
    };
    Ok(BodyRelation { overlap, contact, gap })
}
