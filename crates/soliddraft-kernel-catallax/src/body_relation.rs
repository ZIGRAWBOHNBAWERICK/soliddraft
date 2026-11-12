//! Exact body-pair queries on private copies at the accepted placements.

use catallax_app::queries;
use catallax_measure::{Accuracy, Enclosure, distance::PreparedFace};
use catallax_numbers::{Answer, Rational, Refusal};
use catallax_shapes::{Isometry, Pose, RVector3, Rotation, distance::Options};
use catallax_topology::{Model, transformed_body};
use serde_json::json;
use soliddraft_kernel_api::*;

use crate::{CatallaxKernel, Solid, build, numerical};

fn refused(query: &str, code: &str) -> BodyQueryRefusal {
    let reason = if code.contains("budget") || code.contains("work-limit") || code.contains("deadline") {
        "The check reached its calculation limit."
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

/// Even a zero-width rational interval can lie between two doubles.
fn measurement(e: &Enclosure) -> Measurement {
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
        certainty: if e.is_exact() { Certainty::Exact } else { Certainty::Bounded },
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

/// Both source chains already use unique operation ids in this kernel. Evaluate their
/// union once so both handles belong to one model, retaining the longest common prefix.
fn operands(k: &CatallaxKernel, a: BodyId, b: BodyId) -> KernelResult<(Model, [catallax_topology::BodyId; 2])> {
    let state = k.state()?;
    let (first, second) = (state.get(a)?, state.get(b)?);
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
    let (app, names) = build(&input, OpKind::Intersect, prefix.as_ref().map(|s| &s.app), &files, None)?;
    let built = app.checkpoint.built();
    let bodies = result_names.map(|name| built.bodies[&names[&name]]);
    Ok((built.model.clone(), bodies))
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

fn minimum_gap(
    model: &mut Model,
    [a, b]: [catallax_topology::BodyId; 2],
    clearance: Option<f64>,
) -> Result<BodyDistance, BodyQueryRefusal> {
    if [a, b].iter().any(|body| !model.body(*body).marks.is_empty()) {
        return Err(refused("Minimum distance", "body-relation.marked"));
    }
    let enclosure = answer(
        "Minimum distance",
        numerical("body_minimum_distance", |dec| {
            let prepare = |body, dec: &mut catallax_numbers::Decider| {
                model
                    .faces_of_body(body)
                    .into_iter()
                    .map(|face| PreparedFace::new(model, face, dec))
                    .collect::<Result<Vec<_>, _>>()
            };
            let first = prepare(a, dec)?;
            let second = prepare(b, dec)?;
            let options = Options { width: Rational::parse("1e-9").unwrap(), ..Options::default() };
            let mut lower: Option<Rational> = None;
            let mut upper: Option<Rational> = None;
            for a in &first {
                for b in &second {
                    let bounds = a.face_to_face(b, model, &options, dec)?;
                    if std::env::var_os("SOLIDDRAFT_F13_TRACE").is_some()
                        && clearance.is_some_and(|c| (0.29..0.31).contains(&c))
                    {
                        let c = crate::native::number(clearance.unwrap(), "clearance").unwrap().value().clone();
                        if bounds.lower <= c
                            && bounds.upper.as_ref().is_some_and(|hi| hi > &c)
                            && let Some([p, q]) = &bounds.witnesses
                        {
                            use catallax_numbers::{Outcome, Real, StrictSign};
                            let squared = p.minus(q).norm_squared();
                            let exact = squared.as_rational().map(|r| format!("{}/{}", r.numerator(), r.denominator()));
                            let sign = match dec.sign(&(&squared - &Real::from(&c * &c))) {
                                Outcome::Proven(StrictSign::Negative) => "proven_negative",
                                Outcome::Proven(StrictSign::Positive) => "proven_positive",
                                Outcome::ProvenCoincident(_) => "proven_zero",
                                Outcome::NotExact(_) => "not_exact",
                                Outcome::Refused(_) => "refused",
                            };
                            eprintln!(
                                "[DEBUG-sd-f13] admitted_witness_squared={exact:?} squared_minus_threshold_squared={sign}"
                            );
                        }
                    }
                    lower = Some(lower.map_or(bounds.lower.clone(), |lo| lo.min(bounds.lower)));
                    if let Some(hi) = bounds.upper {
                        upper = Some(upper.map_or(hi.clone(), |old| old.min(hi)));
                    }
                }
            }
            let (Some(lo), Some(hi)) = (lower, upper) else {
                return Err(Refusal::new("body-relation.empty-boundary", "a solid has no boundary"));
            };
            Ok(Answer::exact(Enclosure { lo, hi }))
        }),
    )?;
    let exceeds_clearance = clearance.and_then(|c| {
        // Match the decimal conversion used to build authored dimensions.
        let c = crate::native::number(c, "clearance").ok()?.value().clone();
        if std::env::var_os("SOLIDDRAFT_F13_TRACE").is_some() {
            eprintln!(
                "[DEBUG-sd-f13] gap_lo={}/{} gap_hi={}/{} threshold={}/{} lower_gt={} upper_le={}",
                enclosure.lo.numerator(),
                enclosure.lo.denominator(),
                enclosure.hi.numerator(),
                enclosure.hi.denominator(),
                c.numerator(),
                c.denominator(),
                enclosure.lo > c,
                enclosure.hi <= c,
            );
        }
        if enclosure.lo > c {
            Some(true)
        } else if enclosure.hi <= c {
            Some(false)
        } else {
            None
        }
    });
    Ok(BodyDistance { distance: measurement(&enclosure), zero: enclosure.hi.is_zero(), exceeds_clearance })
}

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
    if std::env::var_os("SOLIDDRAFT_F13_TRACE").is_some() && clearance.is_some_and(|c| (0.29..0.31).contains(&c)) {
        let c = clearance.unwrap();
        let binary = Rational::from_f64_exact(c).unwrap();
        let exact = |v: f64| {
            let r = Rational::from_f64_exact(v).unwrap();
            format!("{}/{}", r.numerator(), r.denominator())
        };
        eprintln!(
            "[DEBUG-sd-f13] input_clearance_short={c} binary_threshold={}/{}",
            binary.numerator(),
            binary.denominator()
        );
        eprintln!(
            "[DEBUG-sd-f13] placement_a_t={:?} placement_b_t={:?}",
            a.translation.map(exact),
            b.translation.map(exact)
        );
        eprintln!("[DEBUG-sd-f13] recipe_a={}", k.body_recipe(a.body)?);
        eprintln!("[DEBUG-sd-f13] recipe_b={}", k.body_recipe(b.body)?);
    }
    let (first, second) = (k.solid(a.body)?, k.solid(b.body)?);
    let (mut model, bodies) = operands(k, a.body, b.body)?;
    let motions = [motion(a)?, motion(b)?];
    let bodies = std::array::from_fn(|i| transformed_body(&mut model, bodies[i], &motions[i], Clone::clone));
    let [a, b] = bodies;
    let overlap = answer(
        "Overlap volume",
        numerical("overlap_volume", |dec| {
            queries::overlap_volume(&model, a, b, &Accuracy::default(), dec).map_err(|r| r.refusal)
        }),
    )
    .map(|e| OverlapVolume {
        positive: if e.lo > Rational::zero() {
            Some(true)
        } else if e.hi.is_zero() {
            Some(false)
        } else {
            None
        },
        volume: measurement(&e),
    });
    let contact = answer(
        "Contact faces",
        numerical("contact_faces", |dec| queries::contact_faces(&model, a, b, dec).map_err(|r| r.refusal)),
    )
    .map(|contact| BodyContactFaces {
        first: source_faces(&first, &contact, &contact.first),
        second: source_faces(&second, &contact, &contact.second),
    });
    let zero_distance = overlap.as_ref().is_ok_and(|v| v.positive == Some(true))
        || contact.as_ref().is_ok_and(|c| !c.first.is_empty() || !c.second.is_empty());
    let gap = if zero_distance {
        Some(Ok(BodyDistance {
            distance: measurement(&Enclosure { lo: Rational::zero(), hi: Rational::zero() }),
            zero: true,
            exceeds_clearance: clearance.map(|_| false),
        }))
    } else if clearance.is_some() || overlap.as_ref().is_ok_and(|v| v.positive == Some(false)) {
        Some(minimum_gap(&mut model, bodies, clearance))
    } else {
        None
    };
    Ok(BodyRelation { overlap, contact, gap })
}
