//! Refine mesh results through the pinned exact solids, never their display copies.

use soliddraft_kernel_api::{BodyQueryRefusal, BodyRelation, KernelError, PlacedBody};

use super::{
    AssemblyDocument, ClearanceOutcome, DistanceBounds, DocRef, Entry, ExactContactFaces, Outcome, PairResult,
};

fn unavailable(error: KernelError) -> BodyQueryRefusal {
    let budget = error.to_string().contains("budget") || error.to_string().contains("work-limit");
    BodyQueryRefusal {
        query: "Exact body comparison".into(),
        code: if budget { "numbers.operation-budget" } else { "body-relation.unavailable" }.into(),
        message: if budget {
            "The exact body comparison reached its calculation limit."
        } else {
            "The exact body comparison is unavailable for these parts. Their saved solids could not be compared."
        }
        .into(),
    }
}

impl AssemblyDocument {
    pub(super) fn exact_pair(
        &mut self,
        result: &mut PairResult,
        a: (&Entry, &DocRef),
        b: (&Entry, &DocRef),
        clearance: Option<f64>,
    ) {
        let queried = || -> Result<_, BodyQueryRefusal> {
            let mut part = |(entry, source): (&Entry, &DocRef)| {
                let document =
                    self.proof_part(source).map_err(|_| unavailable(KernelError::Unavailable(String::new())))?;
                let part = document.lock().expect("the part lock");
                let body = part.body().ok_or_else(|| unavailable(KernelError::Unavailable(String::new())))?;
                let kernel = part.kernel().clone();
                drop(part);
                let at = self.table.world(entry.row);
                Ok::<_, BodyQueryRefusal>((
                    kernel,
                    PlacedBody { body, translation: at.translation.to_array(), rotation: at.rotation.to_array() },
                    document,
                ))
            };
            let (ka, pa, ra) = part(a)?;
            let (kb, pb, rb) = part(b)?;
            if !std::sync::Arc::ptr_eq(&ka, &kb) {
                return Err(unavailable(KernelError::Unavailable(String::new())));
            }
            let relation = ka.body_relation(pa, pb, clearance).map_err(unavailable)?;
            Ok((relation, ra, rb))
        }();
        let (relation, a, b) = match queried {
            Ok(answer) => answer,
            Err(r) => {
                result.exact_refused.push(r);
                return;
            }
        };
        let BodyRelation { overlap, contact, gap } = relation;
        let positive = match overlap {
            Ok(v) => {
                result.overlap_mm3 = Some(v.volume);
                if v.positive.is_none() {
                    result.exact_refused.push(BodyQueryRefusal {
                        query: "Overlap volume".into(), code: "body-relation.overlap-undecided".into(),
                        message: "Overlap was not determined. The proven volume range includes both zero and positive values.".into(),
                    });
                }
                v.positive
            }
            Err(r) => {
                result.exact_refused.push(r);
                None
            }
        };
        let has_contact = match contact {
            Ok(faces) => {
                let labels = |doc: &soliddraft_model::Document, faces: Vec<_>| {
                    faces
                        .into_iter()
                        .map(|f| doc.face_label(f).map(|l| l.text).unwrap_or_else(|| "Face".into()))
                        .collect()
                };
                let a = labels(&a.lock().expect("the part lock"), faces.first);
                let b = labels(&b.lock().expect("the part lock"), faces.second);
                let contact = ExactContactFaces { a, b };
                let found = !contact.a.is_empty() || !contact.b.is_empty();
                result.contact_faces = Some(contact);
                found
            }
            Err(r) => {
                result.exact_refused.push(r);
                false
            }
        };
        if positive == Some(true) {
            result.outcome = Outcome::Interferes;
            result.distance = DistanceBounds { lo: 0.0, hi: Some(0.0) };
            result.clearance = clearance.map(|_| ClearanceOutcome::Below);
            result.why = "The exact solids share a positive volume of material.".into();
        } else if positive == Some(false) {
            result.witness = None;
            if has_contact {
                result.outcome = Outcome::Touching;
                result.why = "The exact solids touch at shared faces and have no overlap volume.".into();
            } else {
                if result.outcome != Outcome::Clear {
                    result.outcome = Outcome::TouchingOrUndecided;
                }
                result.why = "The exact solids have no overlap volume.".into();
            }
        }
        match gap {
            Some(Ok(gap)) => {
                // A boundary distance alone cannot distinguish containment from separation.
                if positive == Some(false) || result.outcome == Outcome::Clear {
                    result.distance = DistanceBounds { lo: gap.distance.lo, hi: Some(gap.distance.hi) };
                    if gap.zero {
                        result.outcome = Outcome::Touching;
                        result.why = "The exact solids touch and have no overlap volume.".into();
                    } else if gap.distance.lo > 0.0 {
                        result.outcome = Outcome::Clear;
                        result.why = "The exact solids are separated by a proven gap.".into();
                    }
                    result.clearance = clearance.map(|_| match gap.exceeds_clearance {
                        Some(true) => ClearanceOutcome::Met,
                        Some(false) => ClearanceOutcome::Below,
                        None => ClearanceOutcome::Undecided,
                    });
                } else if gap.zero {
                    result.distance = DistanceBounds { lo: 0.0, hi: Some(0.0) };
                    result.clearance = clearance.map(|_| ClearanceOutcome::Below);
                }
                if clearance.is_some() && result.clearance == Some(ClearanceOutcome::Undecided) {
                    result.exact_refused.push(BodyQueryRefusal {
                        query: "Clearance".into(), code: "body-relation.clearance-undecided".into(),
                        message: "The clearance was not determined. The proven distance range crosses the requested clearance.".into(),
                    });
                }
            }
            Some(Err(r)) => result.exact_refused.push(r),
            None => {}
        }
    }
}
