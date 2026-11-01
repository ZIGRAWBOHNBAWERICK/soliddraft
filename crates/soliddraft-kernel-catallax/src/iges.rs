//! IGES 5.3 export through Catallax's strict exact-surface writer.

use crate::{numerical, query_error};
use catallax_app::EvaluatedApplicationBody;
use catallax_iges::IgesError;
use catallax_numbers::{Answer, Rational, Refusal};
use soliddraft_kernel_api::{IgesExport, IgesOptions, IgesReport, KernelError, KernelResult};

pub(super) fn export(app: &EvaluatedApplicationBody, options: &IgesOptions) -> KernelResult<IgesExport> {
    if !options.tolerance.is_finite() || options.tolerance < 0.0 {
        return Err(KernelError::InvalidInput("the IGES tolerance must be finite and nonnegative".into()));
    }
    let tolerance =
        Rational::parse(&options.tolerance.to_string()).map_err(|e| KernelError::InvalidInput(e.to_string()))?;
    let settings = catallax_iges::IgesOptions { tolerance: tolerance.clone(), ..Default::default() };
    let built = app.checkpoint.built();
    let mut bytes = Vec::new();
    let report = numerical("export_iges", |dec| {
        catallax_iges::write_iges(&built.model, &[built.result], &settings, dec, &mut bytes)
            .map(Answer::exact)
            .map_err(|e| writer_refusal(&e))
    })
    .map_err(|e| query_error("export_iges", e))?
    .value;
    let text = String::from_utf8(bytes).map_err(|e| KernelError::QueryFailed {
        query: "export_iges".into(),
        code: 0,
        message: e.to_string(),
    })?;
    Ok(IgesExport {
        text,
        report: IgesReport {
            max_deviation: report.max_deviation.to_decimal_bounds(40).1,
            max_deviation_mm: crate::step::f64_up(&report.max_deviation),
            boundary_bound_mm: report.max_deviation.to_string(),
            trim_deviation_mm: report.trim_deviation.to_string(),
            scale: report.scale.to_string(),
            coordinate_bound_mm: report.coordinate_bound.to_string(),
            tolerance_mm: tolerance.to_string(),
            units: "mm".into(),
            within_tolerance: report.max_deviation <= tolerance,
            solids: report.solids,
            faces: report.faces,
            edges: report.edges,
            vertices: report.vertices,
            added_edges: report.added_edges,
            added_vertices: report.added_vertices,
        },
    })
}

fn writer_refusal(error: &IgesError) -> Refusal {
    match error {
        IgesError::InvalidOptions => {
            Refusal::new("iges.invalid-options", "tolerance must not be negative and scale_digits must be 1 to 100")
        }
        IgesError::InvalidTopology(detail) => Refusal::new("iges.invalid-topology", detail.clone()),
        IgesError::UncertifiedModel => Refusal::new(
            "iges.uncertified-model",
            "the model lacks the exact-definition correspondence the writer needs",
        ),
        IgesError::UnsupportedSurface(kind) => {
            Refusal::new("iges.unsupported-surface", format!("surface kind {kind:?} is not written"))
        }
        IgesError::UnsupportedCurve(kind) => {
            Refusal::new("iges.unsupported-curve", format!("curve kind {kind:?} is not written"))
        }
        IgesError::UnrepresentableDefinition(detail) => Refusal::new("iges.unrepresentable-definition", *detail),
        IgesError::ToleranceExceeded { proven, requested } => Refusal::new(
            "iges.tolerance-exceeded",
            format!("proven bound {proven} mm exceeds the requested {requested} mm"),
        ),
        IgesError::Decision(refusal) => refusal.clone(),
        IgesError::Io(error) => Refusal::new("iges.io", error.to_string()),
    }
}
