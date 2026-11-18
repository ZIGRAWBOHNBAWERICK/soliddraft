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
                "This STEP file could not be checked within the available work limit."
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
        Some(NativeStage::RequestLimit) => "This STEP body could not be checked within the available work limit.",
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

pub(super) fn needs_default(opened: &OpenedStep) -> bool {
    let (admitted, needs_default) = opened.bodies().iter().enumerate().map(|(index, _)| opened.body(index)).fold(
        (false, false),
        |(admitted, needs_default), body| match body {
            Some(BodyView::Certified { .. }) => (true, needs_default),
            Some(BodyView::Tolerant { .. } | BodyView::Numerical(_)) => (true, true),
            _ => (admitted, true),
        },
    );
    admitted && needs_default
}

impl CatallaxKernel {
    pub(in crate::import) fn opened_step(&self, source: &[u8]) -> KernelResult<Arc<OpenedStep>> {
        let sha256 = catallax_import::sha256_hex(source);
        if let Some(opened) = self.state()?.tolerant_imports.get(&sha256) {
            return Ok(opened.clone());
        }
        let step = catallax_recipe::StepId::new("import_source").expect("constant step id");
        let opened = opened::open_step(source, &sha256, &step, Default::default(), &mut Decider::default())
            .map_err(admission_error)?;
        let opened = Arc::new(opened);
        self.state()?.tolerant_imports.insert(sha256, opened.clone());
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
        // A checked upgrade owns an actual exact model. Normalize only that model,
        // through Catallax's certified writer, into a reusable feature checkpoint.
        let certified_source = if let Some(BodyView::Certified { model, body }) = opened.body(index) {
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
        let history = approximate.record(seq, tag, OpKind::ImportStep, &[]);
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
