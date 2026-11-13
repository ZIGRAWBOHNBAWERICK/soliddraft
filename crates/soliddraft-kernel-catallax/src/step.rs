//! STEP AP242 export through Catallax's exporter, with its proven deviation report.

use catallax_app::{EvaluatedApplicationBody, certify_built};
use catallax_features::Built;
use catallax_numbers::{Decider, Rational, Refusal};
use catallax_shapes::{SplineSupport, Surface};
use soliddraft_kernel_api::{KernelError, KernelResult, StepExport, StepOptions, StepReport};

use crate::{numerical, query_error};

/// Writes the exact body as STEP. The deviation is Catallax's proven bound on the file
/// against the represented body, rounded up to a decimal and to the next `f64`, never down.
/// A fitted carrier's own fit to its exact definition is reported beside it. A body
/// with a ring band is written only under its certificate, so a ring without one is refused.
pub(super) fn export(app: &EvaluatedApplicationBody, options: &StepOptions) -> KernelResult<StepExport> {
    if !options.tolerance.is_finite() || options.tolerance <= 0.0 {
        return Err(KernelError::InvalidInput(format!(
            "the STEP tolerance is {} mm; it must be more than zero",
            options.tolerance
        )));
    }
    let tolerance =
        Rational::parse(&options.tolerance.to_string()).map_err(|e| KernelError::InvalidInput(e.to_string()))?;
    let catallax_options = catallax_step::StepOptions {
        product_name: options.product_name.clone(),
        curve_tolerance: tolerance.clone(),
        ..catallax_step::StepOptions::default()
    };
    let mut bytes = Vec::new();
    let built = app.checkpoint.built();
    let mut fit = carrier_fit(built);
    let answer = numerical("export_step", |dec| {
        if has_ring_band(built) {
            let (answer, ring_fit) = write_ring_body(built, &catallax_options, dec, &mut bytes)?;
            fit = fit.clone().max(ring_fit);
            return Ok(answer);
        }
        catallax_step::write_step(&built.model, &[built.result], &catallax_options, dec, &mut bytes)
    })
    .map_err(|e| query_error("export_step", e))?;
    let text = String::from_utf8(bytes).map_err(|e| KernelError::QueryFailed {
        query: "export_step".into(),
        code: 0,
        message: e.to_string(),
    })?;
    let report = answer.value;
    let max_deviation = report.max_deviation.to_decimal_bounds(40).1;
    Ok(StepExport {
        text,
        report: StepReport {
            max_deviation_mm: f64_up(&report.max_deviation),
            max_deviation,
            fit_mm: f64_up(&fit),
            within_tolerance: report.max_deviation <= tolerance,
            solids: report.solids,
            faces: report.faces,
            edges: report.edges,
            vertices: report.vertices,
            fitted_curves: report.approximated_curves,
            added_vertices: report.added_vertices,
            added_edges: report.added_edges,
            removed_faces: report.regularizations.iter().map(|r| r.removed_faces).sum(),
            removed_edges: report.regularizations.iter().map(|r| r.removed_edges).sum(),
            warnings: answer.marks.iter().map(|m| format!("{}: {}", m.code, m.detail)).collect(),
        },
    })
}

fn surfaces(built: &Built) -> impl Iterator<Item = &Surface> {
    built
        .model
        .faces_of_body(built.result)
        .into_iter()
        .map(|face| built.model.geom().surface(built.model.face(face).surface))
}

fn has_ring_band(built: &Built) -> bool {
    surfaces(built).any(|surface| matches!(surface, Surface::BSpline(SplineSupport::RollingSaddle(_))))
}

/// The worst carrier fit. A ring band's fit comes from its certificate.
fn carrier_fit(built: &Built) -> Rational {
    surfaces(built).fold(Rational::zero(), |worst, surface| match surface {
        Surface::BSpline(SplineSupport::RollingEnvelope(envelope)) => worst.max(envelope.bound().upper.clone()),
        Surface::BSpline(SplineSupport::SplineFillet(fillet)) => worst.max(fillet.bound().upper.clone()),
        _ => worst,
    })
}

/// Certifies a body with a ring band and writes it. The answer's second part is the worst fit
/// the certificate records: the bands, and the walls they trim. A file over the requested
/// tolerance is refused, because the certificate covers only a file that reaches it.
fn write_ring_body(
    built: &Built,
    options: &catallax_step::StepOptions,
    dec: &mut Decider,
    out: &mut Vec<u8>,
) -> Result<(catallax_numbers::Answer<catallax_step::StepReport>, Rational), Refusal> {
    let certified = certify_built(built.clone(), None, dec).map_err(|error| match error {
        catallax_app::Error::Build(catallax_features::BuildError::Geometry { refusal, .. }) => refusal,
        other => Refusal::new(other.code(), other.to_string()),
    })?;
    let junction = certified
        .built()
        .junction()
        .ok_or_else(|| Refusal::new("numbers.geometric-gap-unproven", "the ring has no certificate"))?;
    let bands = junction.saddles().iter().map(|ring| ring.bound().upper.clone());
    let trims = junction.faces().iter().map(|&face| junction.face_bound(face));
    let fit = bands.chain(trims).fold(Rational::zero(), Rational::max);
    let written = certified.built().built();
    let answer = catallax_step::write_rolling_junction_step(
        &written.model,
        written.result,
        &catallax_step::CertifiedSplineStepOptions {
            product_name: options.product_name.clone(),
            file_tolerance: options.curve_tolerance.clone(),
            ..Default::default()
        },
        dec,
        out,
    )?;
    Ok((answer, fit))
}

/// The smallest `f64` not below `value`.
pub(super) fn f64_up(value: &Rational) -> f64 {
    let nearest = value.to_f64_nearest();
    match Rational::from_f64_exact(nearest) {
        Some(exact) if &exact < value => nearest.next_up(),
        _ => nearest,
    }
}
