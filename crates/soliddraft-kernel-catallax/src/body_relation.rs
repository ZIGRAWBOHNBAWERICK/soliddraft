//! Exact body-pair queries on private copies at the accepted placements.

use catallax_app::queries;
use catallax_measure::{Accuracy, Enclosure, distance::PreparedFace};
use catallax_numbers::{Answer, Outcome, Rational, Real, Refusal, StrictSign};
use catallax_shapes::{Isometry, Pose, RVector3, Rotation, distance::Options};
use catallax_topology::{BodyId as NativeBodyId, Model, transformed_body};
use serde_json::json;
use soliddraft_kernel_api::*;

use crate::{CatallaxKernel, Solid, build, numerical};

fn calculation_limit(code: &str) -> bool {
    code.contains("budget") || code.contains("work-limit") || code.contains("deadline")
}

fn refused(query: &str, code: &str) -> BodyQueryRefusal {
    let reason = if calculation_limit(code) {
        "The check reached its calculation limit."
    } else if code == "app.relation-envelope" {
        "A lattice or gyroid part's material lies on its beams or sheet, not in the whole region its solid outlines."
    } else if code == "body-relation.marked" {
        "The check returned an approximation or an unresolved accuracy bound."
    } else if code.contains("distance") || code.contains("surface") || code.contains("curve") {
        "The check does not support this combination of faces and edges."
    } else {
        "The check could not settle the geometric comparisons these parts need."
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
    let (app, names, ..) = build(
        &input,
        OpKind::Intersect,
        &[],
        prefix.as_ref().map(|s| &s.app),
        None,
        &files,
        None,
        &Default::default(),
        &[],
    )?;
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

fn witness_step<T>(result: Result<T, Refusal>, first_refusal: &mut Option<Refusal>) -> Result<Option<T>, Refusal> {
    if catallax_numbers::current_operation_work().exhausted {
        return Err(Refusal::new("numbers.operation-budget", "operation work allowance exhausted"));
    }
    match result {
        Ok(value) => Ok(Some(value)),
        Err(r) if calculation_limit(r.code) => Err(r),
        Err(r) => {
            first_refusal.get_or_insert(r);
            Ok(None)
        }
    }
}

fn exceeds(
    model: &mut Model,
    bodies: [NativeBodyId; 2],
    gap: &Enclosure,
    clearance: &Rational,
) -> Result<Option<bool>, BodyQueryRefusal> {
    if &gap.lo > clearance {
        return Ok(Some(true));
    }
    if &gap.hi <= clearance {
        return Ok(Some(false));
    }
    let within = numerical("clearance_witness", |dec| {
        let accuracy = Accuracy::default();
        let mut first_refusal = None;
        let mut bounds = |body| {
            let mut faces = Vec::new();
            for face in model.faces_of_body(body) {
                if let Some(bounds) =
                    witness_step(catallax_measure::face_bounding_box(model, face, &accuracy, dec), &mut first_refusal)?
                {
                    faces.push((face, bounds.value, None));
                }
            }
            Ok::<_, Refusal>(faces)
        };
        let [mut first, mut second] = [bounds(bodies[0])?, bounds(bodies[1])?];
        let limit = clearance * clearance;
        let options = Options { width: Rational::parse("1e-9").unwrap(), ..Options::default() };
        for (a, ab, prepared_a) in &mut first {
            for (b, bb, prepared_b) in &mut second {
                dec.check_deadline()?;
                let separation_sq = (0..3).fold(Rational::zero(), |sum, i| {
                    let d = (&ab.min[i] - &bb.max[i]).max(&bb.min[i] - &ab.max[i]).max(Rational::zero());
                    sum + &d * &d
                });
                if separation_sq > limit {
                    continue;
                }
                if prepared_a.is_none() {
                    *prepared_a = Some(witness_step(PreparedFace::new(model, *a, dec), &mut first_refusal)?);
                }
                let Some(Some(a)) = prepared_a.as_ref() else { break };
                if prepared_b.is_none() {
                    *prepared_b = Some(witness_step(PreparedFace::new(model, *b, dec), &mut first_refusal)?);
                }
                let Some(Some(b)) = prepared_b.as_ref() else { continue };
                let Some(bounds) = witness_step(a.face_to_face(b, model, &options, dec), &mut first_refusal)? else {
                    continue;
                };
                let upper = bounds.upper;
                #[cfg(test)]
                let upper = tests::face_upper(upper);
                if upper.as_ref().is_some_and(|hi| hi <= clearance) {
                    return Ok(Answer::exact(true));
                }
                if let Some([p, q]) = bounds.witnesses {
                    let delta = &p.minus(&q).norm_squared() - &Real::from(limit.clone());
                    let sign = match dec.sign(&delta) {
                        Outcome::Proven(StrictSign::Negative) | Outcome::ProvenCoincident(_) => Ok(true),
                        Outcome::Proven(StrictSign::Positive) => Ok(false),
                        Outcome::NotExact(n) => Err(Refusal::from(n)),
                        Outcome::Refused(r) => Err(r),
                    };
                    if witness_step(sign, &mut first_refusal)? == Some(true) {
                        return Ok(Answer::exact(true));
                    }
                }
            }
        }
        first_refusal.map_or_else(|| Ok(Answer::exact(false)), Err)
    });
    answer("Clearance", within).map(|within| within.then_some(false))
}

fn zero_gap(clearance: Option<f64>) -> BodyDistance {
    BodyDistance {
        distance: measurement(&Enclosure { lo: Rational::zero(), hi: Rational::zero() }, false),
        zero: true,
        exceeds_clearance: clearance.map(|_| false),
        clearance_refusal: None,
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
    #[cfg(test)]
    let related = related.map(tests::replace_native_enclosure);
    let related = if related.is_err() && catallax_numbers::current_operation_work().exhausted {
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
                queries::BodyRelation::Clear { gap } => {
                    let (exceeds_clearance, clearance_refusal) =
                        match threshold.as_ref().map_or(Ok(None), |c| exceeds(&mut model, [a, b], &gap, c)) {
                            Ok(decision) => (decision, None),
                            Err(refusal) => (None, Some(refusal)),
                        };
                    (
                        Ok(no_overlap()),
                        Some(Ok(BodyDistance {
                            distance: measurement(&gap, marked),
                            zero: false,
                            exceeds_clearance,
                            clearance_refusal,
                        })),
                    )
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::contract;

    thread_local! {
        static FACE_UPPER_FLOOR: std::cell::Cell<Option<Rational>> = const { std::cell::Cell::new(None) };
        static NATIVE_ENCLOSURE: std::cell::Cell<Option<Enclosure>> = const { std::cell::Cell::new(None) };
    }

    pub(super) fn face_upper(upper: Option<Rational>) -> Option<Rational> {
        FACE_UPPER_FLOOR.with(|slot| {
            let floor = slot.take();
            let upper = match (upper, &floor) {
                (Some(hi), Some(floor)) if &hi < floor => Some(floor.clone()),
                (upper, _) => upper,
            };
            slot.set(floor);
            upper
        })
    }

    pub(super) fn replace_native_enclosure(mut answer: Answer<queries::BodyRelation>) -> Answer<queries::BodyRelation> {
        if let Some(enclosure) = NATIVE_ENCLOSURE.take() {
            let queries::BodyRelation::Clear { gap } = &mut answer.value else {
                panic!("the two blocks are 1 mm apart");
            };
            assert!(enclosure.lo < gap.lo && gap.hi < enclosure.hi);
            *gap = enclosure;
        }
        answer
    }

    #[test]
    fn public_relation_settles_clearance_when_the_native_enclosure_straddles() {
        let k = CatallaxKernel::new();
        let body = contract::block(&k);
        let a = PlacedBody { body, translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0] };
        let b = PlacedBody { translation: [0.0, 0.0, 11.0], ..a };
        let enclosure = Enclosure { lo: Rational::parse("0.9").unwrap(), hi: Rational::parse("1.1").unwrap() };
        for floor in [None, Some(enclosure.hi.clone())] {
            for (first, second) in [(a, b), (b, a)] {
                for (threshold, expected) in [(1.0, Some(false)), (0.99, None)] {
                    NATIVE_ENCLOSURE.set(Some(enclosure.clone()));
                    FACE_UPPER_FLOOR.set(floor.clone());
                    let result = k.body_relation(first, second, Some(threshold));
                    FACE_UPPER_FLOOR.set(None);
                    let result = result.unwrap();
                    assert_eq!(result.overlap.unwrap().positive, Some(false));
                    let gap = result.gap.unwrap().unwrap();
                    assert!(gap.distance.lo < threshold && gap.distance.hi > threshold);
                    assert_eq!(gap.exceeds_clearance, expected);
                    assert!(gap.clearance_refusal.is_none());
                }
            }
        }
    }

    #[test]
    fn a_straddling_gap_uses_an_admitted_witness_at_exact_clearance() {
        let k = CatallaxKernel::new();
        let body = contract::block(&k);
        let Operands { mut model, bodies: [a, b], .. } = operands(&k, body, body).unwrap();
        let placed = PlacedBody { body, translation: [0.0, 0.0, 11.0], rotation: [0.0, 0.0, 0.0, 1.0] };
        let b = transformed_body(&mut model, b, &motion(placed).unwrap(), Clone::clone);
        let clearance = Rational::from_i64(1);
        let enclosure = Enclosure { lo: Rational::parse("0.9").unwrap(), hi: Rational::parse("1.1").unwrap() };
        for bodies in [[a, b], [b, a]] {
            assert_eq!(exceeds(&mut model, bodies, &enclosure, &clearance), Ok(Some(false)));
        }
        let (answer, work) = crate::with_request_allowance(0, || exceeds(&mut model, [a, b], &enclosure, &clearance));
        assert!(work.exhausted);
        assert_eq!(answer.unwrap_err().code, "numbers.operation-budget");
    }
}
