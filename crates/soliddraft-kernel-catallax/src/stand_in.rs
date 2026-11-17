//! Numerical bodies made through Catallax's numerical API from exact builds, for tests.
//!
//! Catallax's application entry returns exact bodies only, for now. A test backend
//! re-admits an exact result as a numerical body, moving one vertex by a tenth of the
//! stated tolerance and repairing it back, as a numerical repair does. The result is a
//! `NumericalApplicationBody` like the ones rolling-ball fillets will return.

use std::sync::Arc;

use catallax_app::{EvaluatedApplicationOutcome, NumericalApplicationBody, NumericalOptions};
use catallax_numbers::{Decider, Rational, Refusal};
use catallax_recipe::Recipe;

/// `outcome` as a numerical body built within Catallax's default tolerance.
pub(crate) fn numerical(
    outcome: EvaluatedApplicationOutcome,
    recipe: &Recipe,
) -> Result<EvaluatedApplicationOutcome, Refusal> {
    let EvaluatedApplicationOutcome::Exact(app) = outcome else { return Ok(outcome) };
    let built = app.checkpoint.built();
    let (model, result) = (&built.model, built.result);
    let options = NumericalOptions::default();
    let mut dec = Decider::default();
    let (mut definition, trims) = catallax_step::numerical::definition_from_exact_model(
        model,
        result,
        recipe.name.clone(),
        &options.tolerance_mm,
        &mut dec,
    )?;
    let vertex = definition.vertices[0].key.clone();
    let position = definition.vertices[0].position.clone();
    let nudge = &options.tolerance_mm / &Rational::from_i64(10);
    definition.vertices[0].position[0] = &position[0] + &nudge;
    let admitted = catallax_topology::numerical::build_with_native_trims(definition, trims, options, &mut dec)?;
    let repaired = admitted.repair_vertex(&vertex, position, &mut dec)?;
    // The definition keys each face by its exact handle, so the origins carry over.
    let origins = model
        .faces_of_body(result)
        .into_iter()
        .map(|face| (format!("{face:?}"), model.face_origins(face).to_vec()))
        .collect();
    Ok(EvaluatedApplicationOutcome::Numerical(Arc::new(NumericalApplicationBody::new(
        repaired,
        Arc::new(recipe.clone()),
        origins,
    ))))
}
