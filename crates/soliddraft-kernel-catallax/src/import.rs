//! STEP and IGES repair records and the kernel's imported recipe checkpoint.

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

fn step_refusal(error: catallax_import::ImportRefusal) -> KernelError {
    use catallax_import::ImportRefusal::*;
    let message = match error {
        Malformed { .. } => "This file could not be read as a STEP solid.".into(),
        Unsupported { type_name, .. } => return KernelError::Unsupported(format!("STEP import of {type_name}")),
        UnknownUnit { .. } => "This STEP file's length units could not be read.".into(),
        HashMismatch { .. } => "The saved import does not match the original file.".into(),
        NotSolid { .. } => "This STEP file does not contain a supported closed solid.".into(),
        InvalidDecision { detail, .. } if detail.contains("below the measured") => {
            "The allowed movement must be at least the measured gap.".into()
        }
        InvalidDecision { detail, .. } if detail.contains("not supported") => {
            return KernelError::Unsupported(format!("STEP import repair: {detail}"));
        }
        InvalidDecision { .. } => "The saved repair decision does not match this STEP file.".into(),
        ValidationFailed { detail, .. } if detail.contains("tenth") => {
            "This repair would move more than a tenth of the nearest feature, so it cannot be applied. Decline it or correct the file.".into()
        }
        ValidationFailed { detail, .. } if detail.contains("exceeds") => {
            "This repair moves the part further than the allowed movement. Raise the limit or decline the repair.".into()
        }
        ValidationFailed { detail, .. } if detail.contains("convex planar cover") || detail.contains("not be bounded") => {
            return KernelError::Unsupported(format!("STEP import repair: {detail}"));
        }
        ValidationFailed { .. } | IncidenceGap { .. } | PlanarRepairNotVerified { .. } => {
            "This STEP file could not be imported without an unverified repair.".into()
        }
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

fn source_format(format: ImportFormat) -> catallax_import::SourceFormat {
    match format {
        ImportFormat::Step => catallax_import::SourceFormat::Step,
        ImportFormat::Iges => catallax_import::SourceFormat::Iges,
        ImportFormat::Mesh => unreachable!("mesh uses its own reader"),
    }
}

fn refusal(format: ImportFormat, error: catallax_import::ImportRefusal) -> KernelError {
    if format == ImportFormat::Step {
        return step_refusal(error);
    }
    use catallax_import::ImportRefusal::*;
    let message = match error {
        Malformed { .. } => "This IGES file could not be read. The file is incomplete or damaged.".into(),
        Unsupported { type_name, .. } => {
            if type_name.contains("orthonormal") {
                "This IGES file uses a transform that changes lengths or angles. It cannot be imported without changing the shape.".into()
            } else if type_name.contains("144") || type_name.contains("143") {
                "This IGES file contains trimmed surfaces rather than a supported closed solid. Export it as a solid and try again.".into()
            } else {
                "This IGES file contains a shape or transform that cannot be imported yet. Try exporting it as a STEP solid.".into()
            }
        }
        UnknownUnit { .. } => {
            "This IGES file's length units could not be read. Export it with millimetre, inch or metre units.".into()
        }
        NotSolid { .. } => {
            "This IGES file does not form a supported closed solid. Check that its faces and edges meet exactly.".into()
        }
        SelfIntersectionNotVerified { faces, .. } => {
            format!("This IGES solid could not be checked for crossing faces at {} and {}.", faces[0], faces[1])
        }
        other => match step_refusal(other) {
            KernelError::InvalidInput(message) => message.replace("STEP", "IGES"),
            _ => "This IGES repair cannot be verified. Decline the repair or correct the source file.".into(),
        },
    };
    KernelError::InvalidInput(message)
}

pub(super) fn propose(format: ImportFormat, source: &[u8], input: &ImportOptions) -> KernelResult<ProposalSet> {
    if format == ImportFormat::Mesh {
        checked_mesh(source)?;
        return Ok(ProposalSet { source_sha256: catallax_import::sha256_hex(source), proposals: Vec::new() });
    }
    let proposals = catallax_import::propose_source(source_format(format), source, &options(input)?)
        .map_err(|error| refusal(format, error))?;
    if !matches!(proposals.format.as_str(), PROPOSALS_FORMAT | catallax_import::INCIDENCE_PROPOSALS_FORMAT) {
        return Err(match format {
            ImportFormat::Mesh => unreachable!(),
            ImportFormat::Step => KernelError::Unsupported(format!("STEP import proposal format {}", proposals.format)),
            ImportFormat::Iges => KernelError::InvalidInput(
                "This IGES file needs a repair that cannot be reviewed yet. Try exporting it as a STEP solid.".into(),
            ),
        });
    }
    wire(proposals)
}

fn require_repair_format(source: ImportFormat, format: &str) -> KernelResult<()> {
    match format {
        REPAIR_FORMAT | LEGACY_REPAIR_FORMAT | catallax_import::INCIDENCE_REPAIR_FORMAT => Ok(()),
        _ if source == ImportFormat::Iges => Err(KernelError::InvalidInput(
            "This IGES file needs a repair record that cannot be replayed yet. Try exporting it as a STEP solid."
                .into(),
        )),
        _ => Err(KernelError::Unsupported(format!("STEP import repair format {format}"))),
    }
}

impl CatallaxKernel {
    pub(super) fn import_body(
        &self,
        format: ImportFormat,
        tag: &OpTag,
        request: &ImportStepRequest<'_>,
    ) -> KernelResult<ImportStepResult> {
        if catallax_import::sha256_hex(request.source) != request.sha256 {
            return Err(KernelError::InvalidInput("the imported file does not match its sha256".into()));
        }
        if format == ImportFormat::Mesh {
            return self.import_mesh_body(tag, request);
        }
        if let Some(saved) = request.record {
            require_repair_format(format, &saved.format)?;
        }
        let decisions: Vec<catallax_import::Decision> = wire(request.decisions)?;
        let mut model = catallax_topology::Model::new();
        let imported = catallax_import::apply_source(
            &mut model,
            source_format(format),
            request.source,
            request.name,
            &options(request.options)?,
            &decisions,
            None,
        )
        .map_err(|error| refusal(format, error))?;
        require_repair_format(format, &imported.record.format)?;
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
                catallax_import::rebuild_source(
                    &mut catallax_topology::Model::new(),
                    source_format(format),
                    request.source,
                    &options(request.options)?,
                    &saved,
                    None,
                )
                .map_err(|error| refusal(format, error))?;
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
        let kind = match format {
            ImportFormat::Step => OpKind::ImportStep,
            ImportFormat::Iges => OpKind::ImportIges,
            ImportFormat::Mesh => unreachable!(),
        };
        self.build_imported_body(kind, tag, request, record, &imported.bodies[0].0)
    }

    fn import_mesh_body(&self, tag: &OpTag, request: &ImportStepRequest<'_>) -> KernelResult<ImportStepResult> {
        checked_mesh(request.source)?;
        if !request.decisions.is_empty() {
            return Err(KernelError::InvalidInput("Mesh solids do not take repair decisions.".into()));
        }
        let record = RepairRecord {
            format: "soliddraft-mesh-import/1".into(),
            source: RepairSource { name: request.name.into(), sha256: request.sha256.into() },
            derivative: None,
            changes: Vec::new(),
            face_tolerances: Vec::new(),
            source_incidence: Vec::new(),
        };
        if request.record.is_some_and(|saved| saved != &record) {
            return Err(KernelError::InvalidInput("The saved mesh import does not match this file.".into()));
        }
        self.build_imported_body(OpKind::ImportMesh, tag, request, record, request.name)
    }

    fn build_imported_body(
        &self,
        kind: OpKind,
        tag: &OpTag,
        request: &ImportStepRequest<'_>,
        record: RepairRecord,
        name: &str,
    ) -> KernelResult<ImportStepResult> {
        let mut state = self.state()?;
        let seq = state.id();
        let result = format!("op_{seq}");
        let capture = state.capture;
        drop(state);
        let mut saved = BTreeMap::new();
        for decision in request.decisions {
            let (id, answer) = match decision {
                Decision::Decline { id } => (id, "decline".to_owned()),
                Decision::Accept { id, tolerance_mm, approver } => (id, format!("accept:{tolerance_mm}:{approver}")),
            };
            let index = id
                .strip_prefix('g')
                .and_then(|s| s.parse().ok())
                .ok_or("invalid gap id")
                .map_err(|e| KernelError::InvalidInput(e.into()))?;
            saved.insert(question_id(request.sha256, index), answer);
        }
        let mut step = json!({"id":result, "op":kind.name(), "file":{"name":request.name,"sha256":request.sha256},
            "pose":{"rotation":["1","0","0","0"],"translation":["0","0","0"]}, "decisions":saved});
        if kind == OpKind::ImportMesh {
            step.as_object_mut().expect("step object").remove("decisions");
            step["scale"] = json!("1");
        }
        // The format that later features convert to, so their builds resume this checkpoint
        // instead of importing the file again.
        let problem = json!({"format":catallax_recipe::FORMAT_V5, "name":"SolidDraft", "steps":[step],"result":result});
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
            KernelError::InvalidInput(format!(
                "This {} body could not be imported and displayed. Try a file with a single closed solid.",
                if kind == OpKind::ImportMesh { "mesh" } else { "imported" }
            ))
        })?;
        let display = MeshCache::new();
        display.get(display_key(&catallax_mesh::MeshOptions::default()), || Ok(app.display.value))?;
        let app = catallax_app::EvaluatedApplicationBody { checkpoint: app.checkpoint, names: app.names };
        let mut state = self.state()?;
        state.files.entry(request.sha256.to_owned()).or_insert_with(|| request.source.into());
        let tags = BTreeMap::from([(result.clone(), tag.clone())]);
        let ids = convert::Ids::new(&app, &[], &tags, &mut state.next, &[]);
        let history = ids.record(seq, tag, kind, &result, &[]);
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
            bodies: vec![ImportedBody { name: name.to_owned(), result: OpResult { body, record: history } }],
            left_open: Vec::new(),
            record,
        })
    }
}

fn checked_mesh(source: &[u8]) -> KernelResult<()> {
    let mesh = catallax_import::mesh::ClosedMesh::read(source, &Rational::one(), &mut Decider::default())
        .map_err(|error| KernelError::InvalidInput(match error.code {
            "import.mesh-open" => "This mesh has open edges. Close them before importing it as a solid.",
            "import.mesh-orientation" | "import.mesh-winding" => "This mesh has reversed faces. Orient them outwards before importing it as a solid.",
            "import.mesh-degenerate" => "This mesh has triangles with no area. Remove them before importing it as a solid.",
            "import.mesh-crossing" => "This mesh crosses or touches itself. Correct the crossing faces before importing it as a solid.",
            "import.mesh-too-large" => "This mesh has more than 200,000 triangles. Reduce the triangle count before importing it as a solid.",
            "import.mesh-nonmanifold-edge" | "import.mesh-pinched-vertex" => "This mesh has edges or vertices shared by separate surfaces. Separate them before importing it as a solid.",
            "import.mesh-malformed" => "This file could not be read as an STL or OBJ mesh.",
            _ => "This mesh could not be checked within the available work limit.",
        }.into()))?;
    if mesh.lumps().len() != 1 {
        return Err(KernelError::InvalidInput("Import one closed mesh solid at a time.".into()));
    }
    Ok(())
}
