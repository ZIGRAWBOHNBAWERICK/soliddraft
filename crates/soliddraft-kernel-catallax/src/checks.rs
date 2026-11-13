//! Native print qualification and independent recipe-prefix construction.

use crate::*;
use catallax_compare::Construction;
use catallax_field::printing::{Options, brep};

impl CatallaxKernel {
    pub(super) fn check_independently(&self, body: BodyId) -> KernelResult<IndependentCheck> {
        let state = self.state()?;
        let stored = state.get(body)?;
        let sources = state.sources(&stored.steps)?;
        let recipe = if let Some(field) = state.fields.get(&body) {
            catallax_recipe::Recipe::parse(&field.recipe().to_string())
                .map_err(|e| KernelError::InvalidInput(e.to_string()))?
        } else {
            state.solid(body)?.app.checkpoint.built().recipe().clone()
        };
        drop(state);
        let files = sources.iter().map(|(sha, bytes)| (sha.clone(), bytes.as_ref())).collect();
        // Each prefix gets the same bounded allowance on a fresh kernel thread.
        let rows = catallax_compare::independent_steps(&recipe, &files, APP_REQUEST_WORK)
            .map_err(|e| query_error("independent_check", e))?;
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
        lattice::export_allowance("print_mesh", lattice::FIELD_ACTION_WORK, || {
            self.qualified_print_within_allowance(solid, params, tolerance)
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
            let certified = catallax_app::certify_built(solid.app.checkpoint.built().clone(), None, &mut dec)
                .map_err(|e| query_error("print_mesh", catallax_numbers::Refusal::new(e.code(), e.to_string())))?;
            let output = brep::mesh_certified(
                certified.built(),
                &Options { tolerance: tolerance.clone(), ..Options::default() },
                &mut dec,
            )
            .map_err(|e| query_error("print_mesh", e))?;
            let built = certified.built().built();
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
        Ok(PrintMesh { mesh, stl, report })
    }
}
