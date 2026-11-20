//! Manufacturing information through Catallax's STEP PMI writer and reader.
//!
//! The app's [`Pmi`] and Catallax's are the same shape. This module converts between
//! them: decimals to exact rationals and back, the app's face ids to the model's.

use catallax_numbers::{Rational, Refusal};
use catallax_step::pmi as cx;
use soliddraft_kernel_api::pmi::{
    Anchor, At, Characteristic, Decimal, Limits, MaterialCondition, PartNotes, Pmi, PmiItem, PmiKind, Process,
    SizeKind, SkippedPmi, StepPmi,
};
use soliddraft_kernel_api::{BodyId, FaceId, KernelError, KernelResult};

use crate::{Solid, numerical};

fn rational(value: &Decimal) -> KernelResult<Rational> {
    Rational::parse(value.as_str()).map_err(|e| KernelError::InvalidInput(e.to_string()))
}

/// A rational as a decimal: exactly when it terminates within fifteen digits, else
/// the nearest nine-place decimal.
fn decimal(value: &Rational) -> Option<Decimal> {
    value
        .to_decimal_exact()
        .and_then(|text| Decimal::parse(&text))
        .or_else(|| Decimal::from_f64(value.to_f64_nearest()))
}

fn characteristic(value: Characteristic) -> cx::Characteristic {
    match value {
        Characteristic::Angularity => cx::Characteristic::Angularity,
        Characteristic::CircularRunout => cx::Characteristic::CircularRunout,
        Characteristic::Coaxiality => cx::Characteristic::Coaxiality,
        Characteristic::Concentricity => cx::Characteristic::Concentricity,
        Characteristic::Cylindricity => cx::Characteristic::Cylindricity,
        Characteristic::Flatness => cx::Characteristic::Flatness,
        Characteristic::LineProfile => cx::Characteristic::LineProfile,
        Characteristic::Parallelism => cx::Characteristic::Parallelism,
        Characteristic::Perpendicularity => cx::Characteristic::Perpendicularity,
        Characteristic::Position => cx::Characteristic::Position,
        Characteristic::Roundness => cx::Characteristic::Roundness,
        Characteristic::Straightness => cx::Characteristic::Straightness,
        Characteristic::SurfaceProfile => cx::Characteristic::SurfaceProfile,
        Characteristic::Symmetry => cx::Characteristic::Symmetry,
        Characteristic::TotalRunout => cx::Characteristic::TotalRunout,
    }
}

fn characteristic_back(value: cx::Characteristic) -> Characteristic {
    match value {
        cx::Characteristic::Angularity => Characteristic::Angularity,
        cx::Characteristic::CircularRunout => Characteristic::CircularRunout,
        cx::Characteristic::Coaxiality => Characteristic::Coaxiality,
        cx::Characteristic::Concentricity => Characteristic::Concentricity,
        cx::Characteristic::Cylindricity => Characteristic::Cylindricity,
        cx::Characteristic::Flatness => Characteristic::Flatness,
        cx::Characteristic::LineProfile => Characteristic::LineProfile,
        cx::Characteristic::Parallelism => Characteristic::Parallelism,
        cx::Characteristic::Perpendicularity => Characteristic::Perpendicularity,
        cx::Characteristic::Position => Characteristic::Position,
        cx::Characteristic::Roundness => Characteristic::Roundness,
        cx::Characteristic::Straightness => Characteristic::Straightness,
        cx::Characteristic::SurfaceProfile => Characteristic::SurfaceProfile,
        cx::Characteristic::Symmetry => Characteristic::Symmetry,
        cx::Characteristic::TotalRunout => Characteristic::TotalRunout,
    }
}

fn at(value: At) -> cx::At {
    match value {
        At::Feature => cx::At::Feature,
        At::Middle => cx::At::Middle,
        At::Inner => cx::At::Inner,
        At::Outer => cx::At::Outer,
    }
}

fn at_back(value: cx::At) -> At {
    match value {
        cx::At::Feature => At::Feature,
        cx::At::Middle => At::Middle,
        cx::At::Inner => At::Inner,
        cx::At::Outer => At::Outer,
    }
}

fn limits(value: &Option<Limits>) -> KernelResult<Option<cx::Limits>> {
    Ok(match value {
        None => None,
        Some(Limits::Deviations { upper, lower }) => {
            Some(cx::Limits::Deviations { upper: rational(upper)?, lower: rational(lower)? })
        }
        Some(Limits::Fit { deviation, grade }) => {
            Some(cx::Limits::Fit { deviation: deviation.clone(), grade: grade.clone() })
        }
    })
}

fn limits_back(value: &Option<cx::Limits>) -> Option<Limits> {
    match value {
        None => None,
        Some(cx::Limits::Deviations { upper, lower }) => {
            Some(Limits::Deviations { upper: decimal(upper)?, lower: decimal(lower)? })
        }
        Some(cx::Limits::Fit { deviation, grade }) => {
            Some(Limits::Fit { deviation: deviation.clone(), grade: grade.clone() })
        }
    }
}

/// The app's information in Catallax's types. Nominal values are left out: Catallax
/// measures them.
fn to_catallax<F: Clone>(pmi: &Pmi<F>) -> KernelResult<cx::Pmi<F>> {
    let mut items = Vec::with_capacity(pmi.items.len());
    for item in &pmi.items {
        let kind = match &item.kind {
            PmiKind::Datum { letter, feature } => {
                cx::ItemKind::Datum(cx::Datum { letter: letter.clone(), feature: feature.clone() })
            }
            PmiKind::Size { size, features, limits: range, .. } => cx::ItemKind::Size(cx::Size {
                kind: match size {
                    SizeKind::Diameter => cx::SizeKind::Diameter,
                    SizeKind::Radius => cx::SizeKind::Radius,
                },
                features: features.clone(),
                limits: limits(range)?,
                nominal: None,
            }),
            PmiKind::Location { from, to, limits: range, .. } => cx::ItemKind::Location(cx::Location {
                from: cx::Anchor { feature: from.feature.clone(), at: at(from.at) },
                to: cx::Anchor { feature: to.feature.clone(), at: at(to.at) },
                limits: limits(range)?,
                nominal: None,
            }),
            PmiKind::Tolerance { characteristic: which, value, diameter_zone, material, datums, features } => {
                cx::ItemKind::Tolerance(cx::GeometricTolerance {
                    characteristic: characteristic(*which),
                    value: rational(value)?,
                    diameter_zone: *diameter_zone,
                    material: material.map(|m| match m {
                        MaterialCondition::Maximum => cx::MaterialCondition::Maximum,
                        MaterialCondition::Least => cx::MaterialCondition::Least,
                    }),
                    datums: datums.clone(),
                    features: features.clone(),
                })
            }
            PmiKind::Texture { parameter, value, process, feature } => cx::ItemKind::Texture(cx::SurfaceTexture {
                parameter: parameter.clone(),
                value: rational(value)?,
                process: match process {
                    Process::Any => cx::Process::Any,
                    Process::Removal => cx::Process::Removal,
                    Process::NoRemoval => cx::Process::NoRemoval,
                },
                feature: feature.clone(),
            }),
        };
        items.push(cx::Item { id: item.id.clone(), kind });
    }
    Ok(cx::Pmi {
        items,
        part: cx::PartNotes {
            general_tolerance: pmi.part.general_tolerance.clone(),
            material: pmi.part.material.clone(),
            notes: pmi.part.notes.clone(),
        },
    })
}

/// Catallax's information in the app's types, or the name of the first item whose
/// values have no decimal form.
fn from_catallax<F: Clone>(pmi: &cx::Pmi<F>) -> Result<Pmi<F>, String> {
    let mut items = Vec::with_capacity(pmi.items.len());
    for item in &pmi.items {
        let lost = || item.id.clone();
        let kind = match &item.kind {
            cx::ItemKind::Datum(datum) => {
                PmiKind::Datum { letter: datum.letter.clone(), feature: datum.feature.clone() }
            }
            cx::ItemKind::Size(size) => PmiKind::Size {
                size: match size.kind {
                    cx::SizeKind::Diameter => SizeKind::Diameter,
                    cx::SizeKind::Radius => SizeKind::Radius,
                },
                features: size.features.clone(),
                limits: match &size.limits {
                    None => None,
                    some => Some(limits_back(some).ok_or_else(lost)?),
                },
                nominal: size.nominal.as_ref().and_then(decimal),
            },
            cx::ItemKind::Location(location) => PmiKind::Location {
                from: Anchor { feature: location.from.feature.clone(), at: at_back(location.from.at) },
                to: Anchor { feature: location.to.feature.clone(), at: at_back(location.to.at) },
                limits: match &location.limits {
                    None => None,
                    some => Some(limits_back(some).ok_or_else(lost)?),
                },
                nominal: location.nominal.as_ref().and_then(decimal),
            },
            cx::ItemKind::Tolerance(tolerance) => PmiKind::Tolerance {
                characteristic: characteristic_back(tolerance.characteristic),
                value: decimal(&tolerance.value).ok_or_else(lost)?,
                diameter_zone: tolerance.diameter_zone,
                material: tolerance.material.map(|m| match m {
                    cx::MaterialCondition::Maximum => MaterialCondition::Maximum,
                    cx::MaterialCondition::Least => MaterialCondition::Least,
                }),
                datums: tolerance.datums.clone(),
                features: tolerance.features.clone(),
            },
            cx::ItemKind::Texture(texture) => PmiKind::Texture {
                parameter: texture.parameter.clone(),
                value: decimal(&texture.value).ok_or_else(lost)?,
                process: match texture.process {
                    cx::Process::Any => Process::Any,
                    cx::Process::Removal => Process::Removal,
                    cx::Process::NoRemoval => Process::NoRemoval,
                },
                feature: texture.feature.clone(),
            },
        };
        items.push(PmiItem { id: item.id.clone(), kind });
    }
    Ok(Pmi {
        items,
        part: PartNotes {
            general_tolerance: pmi.part.general_tolerance.clone(),
            material: pmi.part.material.clone(),
            notes: pmi.part.notes.clone(),
        },
    })
}

/// What a refusal of the PMI writer means to the app: information that does not fit
/// the part is the caller's to correct, and a body the writer cannot name faces on is
/// not supported yet.
pub(super) fn error(query: &str, refusal: Refusal) -> KernelError {
    match refusal.code {
        "step.pmi-invalid" | "step.pmi-not-measurable" | "step.pmi-face-not-written" => {
            KernelError::InvalidInput(refusal.detail)
        }
        "step.pmi-unsupported-body" => KernelError::Unsupported("export_step_with_pmi".into()),
        _ => crate::query_error(query, refusal),
    }
}

/// The app's information on the faces of this solid's model.
pub(super) fn on_model(
    solid: &Solid,
    body: BodyId,
    pmi: &Pmi<FaceId>,
) -> KernelResult<cx::Pmi<catallax_topology::FaceId>> {
    let named = pmi.try_map(|_, face| solid.cat_face(*face, body))?;
    to_catallax(&named)
}

/// The nominal value of every dimension, measured on the solid.
pub(super) fn nominals(solid: &Solid, body: BodyId, pmi: &Pmi<FaceId>) -> KernelResult<Vec<Option<f64>>> {
    let pmi = on_model(solid, body, pmi)?;
    pmi.validate().map_err(|e| error("pmi_nominals", e))?;
    let built = solid.app.checkpoint.built();
    let answer =
        numerical("pmi_nominals", |dec| cx::nominals(&built.model, &pmi, dec).map(catallax_numbers::Answer::exact))
            .map_err(|e| error("pmi_nominals", e))?;
    Ok(answer.value.into_iter().map(|value| value.map(|real| real.to_f64())).collect())
}

/// Reads a STEP file's manufacturing information. Face keys are the entity names the
/// file's import gives its faces.
pub(super) fn read(source: &[u8]) -> KernelResult<StepPmi> {
    let read = catallax_import::read_step_pmi(source)
        .map_err(|e| KernelError::InvalidInput(format!("This STEP file could not be read: {e:?}")))?;
    let mut skipped: Vec<SkippedPmi> =
        read.skipped.into_iter().map(|s| SkippedPmi { entity: s.entity, kind: s.kind, reason: s.reason }).collect();
    // An item whose values have no decimal form is left out by name, like any other.
    let mut pmi = read.pmi;
    loop {
        match from_catallax(&pmi) {
            Ok(pmi) => return Ok(StepPmi { pmi, skipped }),
            Err(id) => {
                pmi.items.retain(|item| item.id != id);
                skipped.push(SkippedPmi {
                    entity: id,
                    kind: "value".into(),
                    reason: "a value is not a decimal number".into(),
                });
            }
        }
    }
}
