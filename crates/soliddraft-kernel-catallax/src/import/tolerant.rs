//! Retain the default reader's geometry and evidence without an exact-model cast.

use catallax_import::tolerant::admission::NativeStage;
use catallax_import::tolerant::opened::{self, BodyView, OpenedStep, Tier};

use super::*;

pub(super) const FORMAT: &str = "soliddraft-tolerant-import/1";

fn admission_error(error: catallax_import::tolerant::Error) -> KernelError {
    match error {
        catallax_import::tolerant::Error::Source(error) => super::step_refusal(error),
        catallax_import::tolerant::Error::Numerical(error) => {
            let message = if error.code.contains("budget") || error.code.contains("deadline") {
                TOO_COMPLEX
            } else if error.code.contains("unit") {
                "This STEP file's units could not be read consistently."
            } else if error.code.contains("resolution") || error.code.contains("precision") {
                "This STEP body's fits could not be checked to the required precision."
            } else {
                "This STEP file could not be opened as a closed solid. Check its faces and edges, or export it again."
            };
            KernelError::InvalidInput(message.into())
        }
    }
}

fn body_refusal(opened: &OpenedStep, index: usize) -> KernelError {
    let body = &opened.bodies()[index];
    if opened
        .tolerant_admission()
        .and_then(|import| import.body_failure(index))
        .is_some_and(|failure| failure.resource_limited())
        && let Some(error) = body.refusal()
    {
        return admission_error(error.clone());
    }
    let message = match body.first_blocker() {
        Some(NativeStage::RequestLimit) => TOO_COMPLEX,
        Some(NativeStage::BodyBuild) => {
            "This STEP body's surfaces or curves could not be read. Export it again as a STEP solid."
        }
        Some(NativeStage::WireConnection) => {
            "This STEP body's boundary edges could not be connected within the allowed gap."
        }
        Some(NativeStage::Sewing) => "This STEP body's faces could not be joined within the allowed gap.",
        Some(NativeStage::ShellIncidence | NativeStage::ManifoldIncidence) => {
            "This STEP body's faces do not form a consistently closed solid."
        }
        Some(NativeStage::CurveFit) => "This STEP body's edges do not fit their curves within the allowed tolerance.",
        Some(NativeStage::CrossingCut) => "This STEP body has a crossing boundary that could not be repaired.",
        Some(NativeStage::SurfaceFit | NativeStage::TraceRebuild) => {
            "This STEP body's edges could not be fitted to its faces within the allowed tolerance."
        }
        Some(NativeStage::PoleBoundary) => "This STEP body's boundary could not be closed at a surface pole.",
        Some(NativeStage::ToleranceGrowth) => "This STEP body needs a repair larger than the allowed tolerance.",
        None => {
            return body.refusal().cloned().map(admission_error).unwrap_or_else(|| {
                KernelError::InvalidInput("This STEP body is open surfaces, not a closed solid.".into())
            });
        }
    };
    KernelError::InvalidInput(message.into())
}

pub(super) fn require_solids(opened: &OpenedStep) -> KernelResult<()> {
    if opened.bodies().is_empty() {
        return Err(KernelError::InvalidInput("This STEP file contains no supported bodies.".into()));
    }
    if !(0..opened.bodies().len())
        .any(|index| opened.bodies()[index].tier() != Tier::SurfaceBody && opened.body(index).is_some())
    {
        return Err(body_refusal(opened, 0));
    }
    Ok(())
}

pub(super) fn has_refused_siblings(opened: &OpenedStep) -> bool {
    let mut admitted = false;
    let mut refused = false;
    for (index, outcome) in opened.bodies().iter().enumerate() {
        match opened.body(index) {
            _ if outcome.tier() == Tier::SurfaceBody && opened.source().bodies()[index].declared_solid() => {
                refused = true;
            }
            Some(BodyView::Certified { .. } | BodyView::Numerical(_) | BodyView::Tolerant { .. })
                if outcome.tier() != Tier::SurfaceBody =>
            {
                admitted = true
            }
            Some(BodyView::Surfaces { .. }) | None => refused = true,
            _ => {}
        }
        if admitted && refused {
            return true;
        }
    }
    false
}

fn resource_limited(error: &catallax_import::tolerant::Error) -> bool {
    match error {
        catallax_import::tolerant::Error::Source(catallax_import::ImportRefusal::ResourceLimit) => true,
        catallax_import::tolerant::Error::Numerical(error) => {
            error.code.contains("budget") || error.code.contains("deadline") || error.code.contains("precision-limit")
        }
        _ => false,
    }
}

fn resource_incomplete(opened: &OpenedStep) -> bool {
    opened.bodies().iter().any(|body| {
        body.refusal().is_some_and(resource_limited) || body.upgrade_refusal().is_some_and(resource_limited)
    })
}

impl CatallaxKernel {
    pub(in crate::import) fn opened_step(&self, source: &[u8]) -> KernelResult<Arc<OpenedStep>> {
        let sha256 = catallax_import::sha256_hex(source);
        if let Some(opened) = self.state()?.tolerant_imports.get(&sha256) {
            return Ok(opened.clone());
        }
        let read = || {
            let step = catallax_recipe::StepId::new("import_source").expect("constant step id");
            let opened = opened::open_step(source, &sha256, &step, Default::default(), &mut Decider::default())
                .map_err(admission_error)?;
            if catallax_numbers::current_operation_work().exhausted {
                return Err(KernelError::InvalidInput(TOO_COMPLEX.into()));
            }
            let complete = !resource_incomplete(&opened);
            Ok((Arc::new(opened), complete))
        };
        let (opened, complete) = crate::RequestPolicy::Open.run(read).0?;
        if complete {
            self.state()?.tolerant_imports.insert(sha256, opened.clone());
        }
        Ok(opened)
    }

    pub(in crate::import) fn import_tolerant_body(
        &self,
        tag: &OpTag,
        request: &ImportStepRequest<'_>,
    ) -> KernelResult<ImportStepResult> {
        if !request.decisions.is_empty() {
            return Err(KernelError::InvalidInput(
                "The saved repair answers do not apply to this tolerant import.".into(),
            ));
        }
        let opened = self.opened_step(request.source)?;
        let index = match request.source_solid {
            Some(key) => opened.source().bodies().iter().position(|body| body.name() == key).ok_or_else(|| {
                KernelError::InvalidInput("The selected body is missing from the original STEP file.".into())
            })?,
            None if opened.bodies().len() == 1 => 0,
            None => return Err(KernelError::InvalidInput("Choose one source solid for each imported body.".into())),
        };
        if opened.bodies()[index].tier() == Tier::SurfaceBody {
            return Err(body_refusal(&opened, index));
        }
        // Checked storage certifies its own geometry, not necessarily the original source.
        // Only an unchanged, non-approximate outcome can become an exact app checkpoint.
        let certified_source = if !opened.bodies()[index].approximate()
            && opened.bodies()[index].tier() == Tier::ValidSolid
            && let Some(BodyView::Certified { model, body }) = opened.body(index)
        {
            Some(
                crate::numerical("import_step", |dec| {
                    let mut bytes = Vec::new();
                    catallax_step::write_step(model, &[body], &Default::default(), dec, &mut bytes)?;
                    Ok(catallax_numbers::Answer::exact(bytes))
                })
                .map_err(|error| crate::query_error("import_step", error))?
                .value,
            )
        } else {
            None
        };
        let derivative_sha256 = certified_source.as_deref().map(catallax_import::sha256_hex);
        let bodies: Vec<_> = opened.bodies().iter().enumerate().map(|(index, body)| {
            let numerical_record = match opened.admission(index) {
                Some(catallax_import::numerical::StepImport::Numerical(import)) => Some(import.record()),
                _ => None,
            };
            json!({"source_solid": opened.source().bodies()[index].name(), "tier": body.tier(), "numerical_record": numerical_record})
        }).collect();
        let record = RepairRecord {
            format: FORMAT.into(),
            source: RepairSource { name: request.name.into(), sha256: request.sha256.into() },
            derivative: derivative_sha256.as_ref().map(|sha256| RepairDerivative { sha256: sha256.clone() }),
            changes: Vec::new(),
            face_tolerances: Vec::new(),
            source_incidence: Vec::new(),
            tolerant: Some(json!({"controls": opened.saved_controls(), "bodies": bodies,
                "native": opened.tolerant_admission().map(|import| import.evidence())})),
        };
        if request.record.is_some_and(|saved| saved != &record) {
            return Err(KernelError::InvalidInput(
                "The saved tolerant import record does not match the original file.".into(),
            ));
        }
        if let (Some(source), Some(sha256)) = (&certified_source, &derivative_sha256) {
            self.state()?.files.entry(request.sha256.to_owned()).or_insert_with(|| request.source.into());
            return self.build_imported_body(
                OpKind::ImportStep,
                tag,
                &ImportStepRequest { source_solid: None, source, sha256, record: None, ..*request },
                record,
                opened.source().bodies()[index].name(),
            );
        }
        let mut state = self.state()?;
        let seq = state.id();
        let result = format!("op_{seq}");
        let approximate = crate::approximate::Approximate::imported(opened.clone(), index, tag, &mut state.next)?;
        drop(state);
        // A readable source is not a usable app body until every face displays.
        approximate.topology().map_err(|error| {
            let reason = if error.to_string().contains("numbers.operation-budget") {
                "This STEP body opened, but could not be displayed within the available work limit."
            } else {
                "This STEP body opened, but its surfaces could not be displayed. Try exporting it again."
            };
            KernelError::InvalidInput(reason.into())
        })?;
        let mut state = self.state()?;
        let history = approximate.record(seq, tag, OpKind::ImportStep, &[], None);
        let body = BodyId::from_raw(state.id());
        let mut step = json!({"id": result, "op": "import_step", "file": {"name": request.name, "sha256": request.sha256},
            "pose": {"rotation": ["1", "0", "0", "0"], "translation": ["0", "0", "0"]}, "decisions": {}});
        if let Some(entity) = request.source_solid {
            step["source_solid"] = json!(entity);
        }
        state.files.entry(request.sha256.to_owned()).or_insert_with(|| request.source.into());
        state.bodies.insert(
            body,
            Stored {
                steps: BTreeMap::from([(seq, step)]),
                result,
                history: vec![history.clone()],
                solid: None,
                approximate: Some(Arc::new(approximate)),
            },
        );
        Ok(ImportStepResult {
            bodies: vec![ImportedBody {
                name: opened.source().bodies()[index].name().to_owned(),
                result: OpResult { body, record: history },
            }],
            left_open: Vec::new(),
            record,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::{BodyAccuracy, Kernel, StepOptions};

    const CLOSED: &str = include_str!("../../../../ci/fixtures/import/block-closed.step");
    const TOLERANT: &str = include_str!("../../../soliddraft-app/tests/data/native-tolerant-vertex.step");

    fn mixed(first: &str, second: &str) -> Vec<u8> {
        let data = second.split_once("DATA;").unwrap().1.split_once("ENDSEC;").unwrap().0;
        let mut renamed = String::new();
        let mut chars = data.chars().peekable();
        while let Some(character) = chars.next() {
            renamed.push(character);
            if character == '#' {
                let number: String = std::iter::from_fn(|| chars.next_if(char::is_ascii_digit)).collect();
                renamed.push_str(&(number.parse::<u64>().unwrap() + 1000).to_string());
            }
        }
        let (prefix, suffix) = first.rsplit_once("ENDSEC;").unwrap();
        format!("{prefix}{renamed}ENDSEC;{suffix}").into_bytes()
    }

    #[test]
    fn healed_checked_storage_beside_a_tolerant_body_stays_tolerant() {
        let source: &[u8] = include_bytes!("../../../../ci/fixtures/import/healed-and-tolerant.step");
        let kernel = CatallaxKernel::new();
        let opened = kernel.opened_step(source).unwrap();
        assert_eq!(opened.bodies().len(), 2);
        let index = opened.source().bodies().iter().position(|body| body.name() == "#178").unwrap();
        assert_eq!(opened.bodies()[index].tier(), Tier::RepairedSolid);
        assert!(opened.bodies()[index].approximate());
        assert!(matches!(opened.body(index), Some(BodyView::Certified { .. })));
        assert!(matches!(opened.body(1 - index), Some(BodyView::Tolerant { .. })));
        let options = Default::default();
        let sha256 = catallax_import::sha256_hex(source);
        let request = ImportStepRequest {
            source_solid: Some(opened.source().bodies()[index].name()),
            name: "mixed.step",
            source,
            sha256: &sha256,
            options: &options,
            decisions: &[],
            record: None,
        };
        let imported = kernel.import_step(&OpTag::new("healed"), &request).unwrap();
        let body = imported.bodies[0].result.body;
        assert_eq!(kernel.body_accuracy(body).unwrap(), BodyAccuracy::Tolerant);
        assert!(imported.record.derivative.is_none());
        assert_eq!(kernel.topology(body).unwrap().faces.len(), 6);
        assert!(!kernel.tessellate(body, &Default::default()).unwrap().indices.is_empty());
        assert!((kernel.mass_properties(body).unwrap().volume - 10004.0).abs() < 0.001);
        let exported = kernel.export_step(body, &StepOptions::default()).unwrap();
        assert!(exported.report.tolerant);
        assert!(!exported.report.within_tolerance);
        let replay = kernel
            .import_step(&OpTag::new("healed_replay"), &ImportStepRequest { record: Some(&imported.record), ..request })
            .unwrap();
        assert_eq!(kernel.body_accuracy(replay.bodies[0].result.body).unwrap(), BodyAccuracy::Tolerant);
    }

    #[test]
    fn a_cold_open_is_charged_and_an_exhausted_open_can_retry_with_more_work() {
        let source = mixed(CLOSED, TOLERANT);
        let fresh = CatallaxKernel::new();
        let measured = fresh.opened_step(&source).unwrap();
        assert!(measured.native_admission_work_used() > 0);
        let limit = measured.read_work_used()
            + measured.checked_upgrade_work_used()
            + measured.native_admission_work_used()
            + measured.bodies().len() as u64;
        let sha256 = catallax_import::sha256_hex(&source);
        let (partial, work) = crate::with_request_allowance(limit, || {
            opened::open_step(
                &source,
                &sha256,
                &catallax_recipe::StepId::new("import_source").unwrap(),
                Default::default(),
                &mut Decider::default(),
            )
        });
        assert!(work.exhausted);
        let partial = partial.unwrap();
        assert!(resource_incomplete(&partial));
        let kernel = CatallaxKernel::new();
        let (limited, work) = crate::with_request_allowance(limit, || kernel.opened_step(&source));
        assert!(work.exhausted);
        assert!(matches!(limited, Err(KernelError::InvalidInput(reason)) if reason == TOO_COMPLEX));
        let retried = kernel.opened_step(&source).unwrap();
        assert!(!resource_incomplete(&retried));
        assert!(matches!(retried.body(0), Some(BodyView::Certified { .. })));
        assert!(matches!(retried.body(1), Some(BodyView::Tolerant { .. })));
        let options = Default::default();
        let sha256 = catallax_import::sha256_hex(&source);
        let request = ImportStepRequest {
            source_solid: Some(retried.source().bodies()[1].name()),
            name: "mixed.step",
            source: &source,
            sha256: &sha256,
            options: &options,
            decisions: &[],
            record: None,
        };
        let import = |kernel: &CatallaxKernel| {
            let imported = kernel.import_step(&OpTag::new("mixed"), &request).unwrap();
            let body = imported.bodies[0].result.body;
            assert_eq!(kernel.body_accuracy(body).unwrap(), BodyAccuracy::Tolerant);
            let topology = kernel.topology(body).unwrap();
            let mesh = kernel.tessellate(body, &Default::default()).unwrap();
            assert!(!mesh.indices.is_empty());
            (imported, topology, mesh)
        };
        assert_eq!(import(&kernel), import(&fresh));
    }
}
