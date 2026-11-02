//! STEP repair records and the kernel's imported recipe checkpoint.

use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

use catallax_numbers::{Decider, Rational};
use serde_json::json;
use soliddraft_kernel_api::*;

use crate::{CatallaxKernel, MeshCache, Solid, Stored, convert, display_key};

fn options(input: &ImportOptions) -> KernelResult<catallax_import::ImportOptions> {
    Ok(catallax_import::ImportOptions {
        search_radius_mm: Rational::parse(&input.search_radius_mm)
            .map_err(|e| KernelError::InvalidInput(e.to_string()))?,
    })
}

fn wire<T: serde::de::DeserializeOwned>(input: impl serde::Serialize) -> KernelResult<T> {
    serde_json::from_value(serde_json::to_value(input).map_err(|e| KernelError::InvalidInput(e.to_string()))?)
        .map_err(|e| KernelError::InvalidInput(format!("STEP import contract: {e}")))
}

fn refusal(error: catallax_import::ImportRefusal) -> KernelError {
    use catallax_import::ImportRefusal::*;
    let message = match error {
        Malformed { .. } => "This file could not be read as a STEP solid.".into(),
        Unsupported { entity, .. } => format!(
            "This STEP file contains geometry that cannot be imported yet, at {entity}. Try a file with a single closed analytic solid."
        ),
        UnknownUnit { .. } => "This STEP file's length units could not be read.".into(),
        HashMismatch { .. } => "The saved import does not match the original file.".into(),
        NotSolid { .. } => "This STEP file does not contain a supported closed solid.".into(),
        InvalidDecision { .. } => "The saved repair decision does not match this STEP file.".into(),
        ValidationFailed { .. } => "This STEP file could not be imported without an unverified repair.".into(),
        SelfIntersecting { faces, .. } => {
            format!("This STEP solid crosses itself at faces {} and {}.", faces[0], faces[1])
        }
        SelfIntersectionNotVerified { faces, reason, .. } => {
            format!("This STEP solid could not be verified at faces {} and {}: {reason}", faces[0], faces[1])
        }
        ResourceLimit => "This STEP file could not be checked within the available work limit.".into(),
    };
    KernelError::InvalidInput(message)
}

pub(super) fn propose(source: &[u8], input: &ImportOptions) -> KernelResult<ProposalSet> {
    let proposals = catallax_import::propose(source, &options(input)?).map_err(refusal)?;
    wire(proposals)
}

impl CatallaxKernel {
    pub(super) fn import_body(&self, tag: &OpTag, request: &ImportStepRequest<'_>) -> KernelResult<ImportStepResult> {
        if catallax_import::sha256_hex(request.source) != request.sha256 {
            return Err(KernelError::InvalidInput("the imported file does not match its sha256".into()));
        }
        let decisions: Vec<catallax_import::Decision> = wire(request.decisions)?;
        let mut model = catallax_topology::Model::new();
        let imported = catallax_import::apply(
            &mut model,
            request.source,
            request.name,
            &options(request.options)?,
            &decisions,
            None,
        )
        .map_err(refusal)?;
        let mismatch =
            || KernelError::InvalidInput("the saved import record does not match this file and its decisions".into());
        match request.record {
            // `apply` above redid the import and certified it again, which is what a record
            // from before face tolerances needs. It may not be relabelled, only matched.
            Some(saved) if saved.format == LEGACY_REPAIR_FORMAT => {
                let fresh: RepairRecord = wire(&imported.record)?;
                if (&saved.source, &saved.derivative, &saved.changes)
                    != (&fresh.source, &fresh.derivative, &fresh.changes)
                {
                    return Err(mismatch());
                }
            }
            Some(saved) => {
                let saved: catallax_import::RepairRecord = wire(saved)?;
                catallax_import::rebuild(
                    &mut catallax_topology::Model::new(),
                    request.source,
                    &options(request.options)?,
                    &saved,
                    None,
                )
                .map_err(refusal)?;
                if saved != imported.record {
                    return Err(mismatch());
                }
            }
            None => {}
        }
        let count = imported.bodies.len() + imported.left_open.len();
        if count != 1 {
            let names = imported
                .bodies
                .iter()
                .map(|(name, _)| name.as_str())
                .chain(imported.left_open.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(KernelError::InvalidInput(format!(
                "the file holds {count} solids; a part holds one body ({names})"
            )));
        }
        let record = wire(imported.record)?;
        if !imported.left_open.is_empty() {
            return Ok(ImportStepResult { bodies: Vec::new(), left_open: imported.left_open, record });
        }
        let mut state = self.state()?;
        let seq = state.id();
        let result = format!("op_{seq}");
        let capture = state.capture;
        drop(state);
        let mut saved = BTreeMap::new();
        for decision in request.decisions {
            let Decision::Decline { id } = decision else {
                return Err(KernelError::InvalidInput(
                    "explicit STEP repair acceptance is not supported in this batch".into(),
                ));
            };
            let index = id
                .strip_prefix('g')
                .and_then(|s| s.parse().ok())
                .ok_or("invalid gap id")
                .map_err(|e| KernelError::InvalidInput(e.into()))?;
            saved.insert(question_id(request.sha256, index), "decline");
        }
        let step = json!({"id":result, "op":"import_step", "file":{"name":request.name,"sha256":request.sha256},
            "pose":{"rotation":["1","0","0","0"],"translation":["0","0","0"]}, "decisions":saved});
        let problem = json!({"format":"catallax-recipe/3", "name":"SolidDraft", "steps":[step],"result":result});
        let recipe = catallax_recipe::Recipe::parse(&problem.to_string())
            .map_err(|e| KernelError::InvalidInput(e.to_string()))?;
        let files = BTreeMap::from([(request.sha256.to_owned(), request.source)]);
        let app = catallax_app::build_incremental_with_files(
            &recipe,
            None,
            &files,
            &catallax_mesh::MeshOptions::default(),
            &mut Decider::default(),
        );
        if capture {
            self.state()?.questions.push(json!({"format":"catallax-question/1","source":"recipe","problem":problem}));
        }
        let app = app.map_err(|_| {
            KernelError::InvalidInput(
                "This STEP body could not be imported and displayed. Try a file with a single closed analytic solid."
                    .into(),
            )
        })?;
        let display = MeshCache::new();
        display.get(display_key(&catallax_mesh::MeshOptions::default()), || Ok(app.display.value))?;
        let app = catallax_app::EvaluatedApplicationBody { checkpoint: app.checkpoint, names: app.names };
        let mut state = self.state()?;
        let tags = BTreeMap::from([(result.clone(), tag.clone())]);
        let ids = convert::Ids::new(&app, &[], &tags, &mut state.next, &[]);
        let history = ids.record(seq, tag, OpKind::ImportStep, &result, &[]);
        let body = BodyId::from_raw(state.id());
        state.bodies.insert(
            body,
            Stored {
                steps: BTreeMap::from([(seq, step)]),
                result,
                history: vec![history.clone()],
                solid: Some(Arc::new(Solid {
                    app,
                    ids,
                    display,
                    previews: MeshCache::new(),
                    topology: OnceLock::new(),
                    measured: OnceLock::new(),
                    mass: OnceLock::new(),
                })),
            },
        );
        Ok(ImportStepResult {
            bodies: vec![ImportedBody {
                name: imported.bodies[0].0.clone(),
                result: OpResult { body, record: history },
            }],
            left_open: Vec::new(),
            record,
        })
    }
}
