//! Native print qualification and independent recipe-prefix construction.

use crate::*;
use catallax_compare::Construction;
use catallax_field::printing::{Options, brep};

impl CatallaxKernel {
    pub(super) fn check_independently(&self, body: BodyId) -> KernelResult<IndependentCheck> {
        let state = self.state()?;
        let stored = state.get(body)?;
        let sources = state.sources(&stored.steps)?;
        let tags: BTreeMap<_, _> =
            stored.history.iter().map(|record| (format!("op_{}", record.seq), record.tag.clone())).collect();
        let field_tag = state.fields.get(&body).and_then(|_| stored.history.last()).map(|record| record.tag.clone());
        let recipe = if let Some(field) = state.fields.get(&body) {
            catallax_recipe::Recipe::parse(&field.recipe().to_string())
                .map_err(|e| KernelError::InvalidInput(e.to_string()))?
        } else {
            state.solid(body)?.app.checkpoint.built().recipe().clone()
        };
        drop(state);
        let files = sources.iter().map(|(sha, bytes)| (sha.clone(), bytes.as_ref())).collect();
        // Each prefix gets the same bounded allowance on a fresh kernel thread.
        let rows =
            independent_steps(&recipe, &files, APP_REQUEST_WORK).map_err(|e| query_error("independent_check", e))?;
        Ok(IndependentCheck {
            steps: rows
                .into_iter()
                .map(|row| {
                    let reference = matches!(row.brep.construction, Construction::Reference);
                    let mut unconfirmed = Vec::new();
                    for (name, construction) in
                        [("Boundary model", &row.brep.construction), ("Distance field", &row.field.construction)]
                    {
                        if let Construction::Refused { blocked_at, refusal } = construction {
                            let at = blocked_at.as_ref().map(|id| format!(" at {id}")).unwrap_or_default();
                            unconfirmed.push(format!("{name}{at}: {} ({})", refusal.detail, refusal.code));
                        }
                    }
                    IndependentStep {
                        feature_tag: tags.get(row.step.as_str()).cloned().or_else(|| {
                            (row.step == recipe.result && field_tag.is_some()).then(|| field_tag.clone()).flatten()
                        }),
                        step: row.step.to_string(),
                        operation: row.operation,
                        reference,
                        boundary_built: matches!(row.brep.construction, Construction::Built { .. }),
                        field_built: matches!(row.field.construction, Construction::Built { .. }),
                        unconfirmed,
                    }
                })
                .collect(),
        })
    }

    pub(super) fn qualified_print(&self, body: BodyId, params: &TessellationParams) -> KernelResult<PrintMesh> {
        let solid = self.solid(body)?;
        let tolerance = native::number(params.chord_height, "print accuracy")?.value().clone();
        if tolerance <= Rational::zero() {
            return Err(KernelError::InvalidInput("Print accuracy must be greater than zero".into()));
        }
        let mut key = display_key(&options(params)?);
        // Print does not promise either angle control, only whether one was requested.
        key.1 = Some(Rational::from_i64(if params.curve_angle_deg > 0.0 || params.surface_angle_deg > 0.0 {
            1
        } else {
            0
        }));
        lattice::export_allowance("print_mesh", lattice::FIELD_ACTION_WORK, || {
            solid
                .print
                .get(key, || self.qualified_print_within_allowance(solid.clone(), params, tolerance))
                .map(|output| (*output).clone())
        })
        .map(|(output, _)| output)
    }

    fn qualified_print_within_allowance(
        &self,
        solid: Arc<Solid>,
        params: &TessellationParams,
        tolerance: Rational,
    ) -> KernelResult<PrintMesh> {
        let (stl, output, source) = {
            let mut dec = Decider::default();
            let exact = solid.app.checkpoint.built();
            let certified =
                if exact.model.body(exact.result).marks.is_empty() {
                    None
                } else {
                    Some(catallax_app::certify_built(exact.clone(), None, &mut dec).map_err(|e| {
                        query_error("print_mesh", catallax_numbers::Refusal::new(e.code(), e.to_string()))
                    })?)
                };
            let options = Options { tolerance: tolerance.clone(), ..Options::default() };
            let output = match &certified {
                Some(certified) => brep::mesh_certified(certified.built(), &options, &mut dec),
                None => brep::mesh(exact, &options, &mut dec),
            }
            .map_err(|e| query_error("print_mesh", e))?;
            let built = certified.as_ref().map_or(exact, |certified| certified.built().built());
            let source = catallax_mesh::mesh_body(
                &built.model,
                built.result,
                &MeshOptions {
                    chord_tolerance: (&tolerance - output.model_bound()).mul_pow2(-1),
                    ..MeshOptions::default()
                },
                &mut dec,
            )
            .map_err(|e| query_error("print_faces", e))?
            .value;
            let (stl, output) = output.stl().map_err(|e| query_error("print_mesh", e))?;
            (stl, output, source)
        };
        let report = PrintReport {
            route: output.route().into(),
            requested_accuracy_mm: params.chord_height,
            certified_accuracy_mm: step::f64_up(&output.mesh().max_deviation),
            model_bound_mm: step::f64_up(output.model_bound()),
            tessellation_bound_mm: step::f64_up(output.tessellation_bound()),
            rounding_bound_mm: step::f64_up(output.rounding_bound()),
            stages: vec![
                "Model accuracy certified".into(),
                "Finite faces and trims certified".into(),
                "File coordinate rounding certified".into(),
                "Closed surface and vertex connections confirmed".into(),
                "Outward orientation confirmed, including cavities".into(),
                "All components and cavities preserved".into(),
            ],
        };
        // Recover face groups from the same tessellation controls used by the print route.
        let key = |points: [[f32; 3]; 3]| {
            let mut bits = points.map(|p| p.map(|v| if v == 0.0 { 0 } else { v.to_bits() }));
            bits.sort();
            bits
        };
        let faces: BTreeMap<_, _> = source
            .triangles
            .iter()
            .map(|t| (key(t.vertices.map(|i| source.positions[i].map(|v| v as f32))), solid.ids.faces[&t.face]))
            .collect();
        let positions: Vec<_> = output.mesh().positions.iter().map(|p| p.map(f64::from)).collect();
        let triangles = output
            .mesh()
            .triangles
            .iter()
            .map(|t| {
                let face = faces.get(&key(t.map(|i| output.mesh().positions[i]))).copied().ok_or_else(|| {
                    KernelError::Unsupported("Print face correspondence could not be confirmed".into())
                })?;
                Ok((*t, face))
            })
            .collect::<KernelResult<Vec<_>>>()?;
        let mut mesh = convert::triangles(&positions, triangles);
        if params.curve_angle_deg > 0.0 || params.surface_angle_deg > 0.0 {
            mesh.unavailable_controls.push("Angle controls are not certified by the print route".into());
        }
        if params.max_edge_length > 0.0 {
            mesh.unavailable_controls.push("Maximum edge length is not certified by the print route".into());
        }
        if params.max_aspect_ratio > 0.0 {
            mesh.unavailable_controls.push("Aspect ratio is not certified by the print route".into());
        }
        Ok(PrintMesh { mesh, stl, report })
    }
}

#[cfg(not(target_arch = "wasm32"))]
use catallax_compare::independent_steps;

#[cfg(target_arch = "wasm32")]
use browser_independent_steps as independent_steps;

// The browser has no OS threads. Each prefix still gets separate evaluations and allowances.
#[cfg(any(target_arch = "wasm32", test))]
fn browser_independent_steps(
    recipe: &catallax_recipe::Recipe,
    files: &catallax_features::ImportFiles<'_>,
    max_work: u64,
) -> Result<Vec<catallax_compare::IndependentStep>, catallax_numbers::Refusal> {
    use catallax_compare::{IndependentStep, KernelBuild};
    use catallax_features::BuildError;
    use catallax_numbers::{Refusal, with_operation_budget, with_operation_log};
    recipe.check().map_err(|error| Refusal::new("compare.recipe", error.to_string()))?;
    let mut rows = Vec::new();
    for (index, step) in recipe.steps.iter().enumerate() {
        let build = |field: bool| {
            if step.op.kind() != catallax_recipe::ValueKind::Body {
                return KernelBuild { construction: Construction::Reference, work: 0, limit: max_work };
            }
            let mut prefix = recipe.clone();
            prefix.steps.truncate(index + 1);
            prefix.result = step.id.clone();
            prefix.materials.retain(|material| prefix.steps.iter().any(|step| step.id == material.region));
            let ((result, work), log) = with_operation_log(|| {
                with_operation_budget(max_work, || {
                    let mut dec = Decider::default();
                    if field {
                        catallax_field::Plan::compile_with_files_checked(&prefix, files, &mut dec)
                            .map(|plan| Construction::Built {
                                faces: plan
                                    .faces()
                                    .iter()
                                    .enumerate()
                                    .filter(|(i, _)| plan.result_face(*i).is_some())
                                    .count(),
                                empty: plan.is_proven_empty(),
                            })
                            .map_err(|refusal| (None, refusal))
                    } else {
                        catallax_features::evaluate_with_files(&prefix, files, &mut dec)
                            .and_then(|built| {
                                if let Some(stand_in) = built.stand_ins.first() {
                                    return Err(BuildError::Geometry {
                                        step: stand_in.step.clone(),
                                        refusal: Refusal::new(
                                            "compare.envelope-only",
                                            "region stand-in is not an independent construction of the field feature",
                                        ),
                                    });
                                }
                                Ok(Construction::Built {
                                    faces: built.model.faces_of_body(built.result).len(),
                                    empty: built.model.body(built.result).kind == catallax_topology::BodyKind::Empty,
                                })
                            })
                            .map_err(|error| match error {
                                BuildError::Geometry { step, refusal } => (Some(step.to_string()), refusal),
                                BuildError::Recipe(error) => (None, Refusal::new("compare.recipe", error.to_string())),
                            })
                    }
                })
            });
            let construction = match result {
                Ok(construction) if !work.exhausted => construction,
                result => {
                    let (blocked_at, refusal) = result.err().unwrap_or_else(|| {
                        (None, Refusal::new("compare.work-exhausted", "The construction reached its calculation limit"))
                    });
                    Construction::Refused {
                        blocked_at: blocked_at
                            .or_else(|| log.iter().find(|entry| entry.refusal.is_some()).map(|entry| entry.id.clone())),
                        refusal,
                    }
                }
            };
            KernelBuild { construction, work: work.used, limit: max_work }
        };
        rows.push(IndependentStep {
            step: step.id.clone(),
            operation: step.op.name().into(),
            brep: build(false),
            field: build(true),
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_and_desktop_construct_the_same_prefixes_and_refuse_exhausted_work() {
        let kernel = CatallaxKernel::new();
        let profile = kernel
            .create_body_from_profile(
                &OpTag::new("profile"),
                &Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 6.0, 6.0),
            )
            .unwrap();
        let body = kernel
            .extrude(&OpTag::new("block"), profile.body, &Extrude { distance: 10.0, direction: None })
            .unwrap()
            .body;
        let recipe = kernel.solid(body).unwrap().app.checkpoint.built().recipe().clone();
        let files = catallax_features::ImportFiles::new();
        for limit in [0, APP_REQUEST_WORK] {
            let browser = browser_independent_steps(&recipe, &files, limit).unwrap();
            let desktop = catallax_compare::independent_steps(&recipe, &files, limit).unwrap();
            assert_eq!(browser.len(), desktop.len());
            for (browser, desktop) in browser.iter().zip(&desktop) {
                assert_eq!(browser.step, desktop.step);
                assert_eq!(browser.brep.construction, desktop.brep.construction);
                assert_eq!(browser.field.construction, desktop.field.construction);
            }
        }
    }
    #[test]
    fn print_certificates_are_reused_only_for_the_same_body_and_controls() {
        let kernel = CatallaxKernel::new();
        let profile = kernel
            .create_body_from_profile(
                &OpTag::new("profile"),
                &Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 6.0, 6.0),
            )
            .unwrap();
        let body = kernel
            .extrude(&OpTag::new("block"), profile.body, &Extrude { distance: 10.0, direction: None })
            .unwrap()
            .body;
        let params = TessellationParams { chord_height: 0.01, ..Default::default() };
        let before = kernel.retained_bytes(body).unwrap();
        let (first, work) = with_request_allowance(APP_REQUEST_WORK, || kernel.print_mesh(body, &params));
        let first = first.unwrap().unwrap();
        assert!(work.used > 0 && !work.exhausted);
        assert!(kernel.retained_bytes(body).unwrap() > before);
        let (again, work) = with_request_allowance(APP_REQUEST_WORK, || kernel.print_mesh(body, &params));
        assert_eq!(again.unwrap().unwrap().stl, first.stl);
        assert_eq!(work.used, 0);
        let finer = TessellationParams { chord_height: 0.005, ..params };
        let (output, work) = with_request_allowance(APP_REQUEST_WORK, || kernel.print_mesh(body, &finer));
        assert_eq!(output.unwrap().unwrap().report.requested_accuracy_mm, 0.005);
        assert!(work.used > 0 && !work.exhausted);
        let changed = kernel
            .extrude(&OpTag::new("changed"), profile.body, &Extrude { distance: 12.0, direction: None })
            .unwrap()
            .body;
        let (output, work) = with_request_allowance(APP_REQUEST_WORK, || kernel.print_mesh(changed, &params));
        assert!(output.is_ok() && work.used > 0 && !work.exhausted);
    }
}
