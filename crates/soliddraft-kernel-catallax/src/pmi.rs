//! Manufacturing information through Catallax's STEP PMI writer and reader.
//!
//! The app states an item as people do; a STEP file holds dimensions, tolerances and
//! textures one by one. This module converts between the two: decimals to exact
//! rationals and back, the app's face ids to the model's, and a slot, which is one item
//! in the app, to the three dimensions a file holds for it.

use catallax_numbers::{Rational, Refusal};
use catallax_step::pmi as cx;
use soliddraft_kernel_api::pmi::{
    Anchor, At, DatumRef, Decimal, FitClass, GdtCharacteristic, MaterialCondition, PartNotes, Pmi, PmiItem, PmiKind,
    SizeKind, SkippedPmi, StepPmi, SurfaceProcess, Tolerance, roughness,
};
use soliddraft_kernel_api::{BodyId, FaceId, KernelError, KernelResult};

use crate::{Solid, numerical};

/// The names a file gives the three dimensions of a slot, after the slot's own.
const SLOT_LENGTH: &str = "length";
const SLOT_WIDTH: &str = "width";
const SLOT_RADIUS: &str = "end radius";

fn part_name(id: &str, part: &str) -> String {
    format!("{id}/{part}")
}

fn invalid(id: &str, what: &str) -> KernelError {
    KernelError::InvalidInput(format!("{id}: {what}"))
}

fn rational(id: &str, value: f64) -> KernelResult<Rational> {
    let decimal = Decimal::from_f64(value).ok_or_else(|| invalid(id, "a value is not a number"))?;
    Rational::parse(decimal.as_str()).map_err(|e| KernelError::InvalidInput(e.to_string()))
}

/// A rational as a number: exactly when it terminates within fifteen digits, else the
/// nearest double.
fn number(value: &Rational) -> f64 {
    value
        .to_decimal_exact()
        .and_then(|text| Decimal::parse(&text))
        .map_or_else(|| value.to_f64_nearest(), |decimal| decimal.value())
}

fn characteristic(value: GdtCharacteristic) -> cx::Characteristic {
    match value {
        GdtCharacteristic::Angularity => cx::Characteristic::Angularity,
        GdtCharacteristic::CircularRunout => cx::Characteristic::CircularRunout,
        // On the faces of a part the feature is an axis, so the tolerance is coaxiality.
        GdtCharacteristic::Concentricity => cx::Characteristic::Coaxiality,
        GdtCharacteristic::Cylindricity => cx::Characteristic::Cylindricity,
        GdtCharacteristic::Flatness => cx::Characteristic::Flatness,
        GdtCharacteristic::ProfileOfLine => cx::Characteristic::LineProfile,
        GdtCharacteristic::Parallelism => cx::Characteristic::Parallelism,
        GdtCharacteristic::Perpendicularity => cx::Characteristic::Perpendicularity,
        GdtCharacteristic::Position => cx::Characteristic::Position,
        GdtCharacteristic::Circularity => cx::Characteristic::Roundness,
        GdtCharacteristic::Straightness => cx::Characteristic::Straightness,
        GdtCharacteristic::ProfileOfSurface => cx::Characteristic::SurfaceProfile,
        GdtCharacteristic::Symmetry => cx::Characteristic::Symmetry,
        GdtCharacteristic::TotalRunout => cx::Characteristic::TotalRunout,
    }
}

fn characteristic_back(value: cx::Characteristic) -> GdtCharacteristic {
    match value {
        cx::Characteristic::Angularity => GdtCharacteristic::Angularity,
        cx::Characteristic::CircularRunout => GdtCharacteristic::CircularRunout,
        cx::Characteristic::Coaxiality | cx::Characteristic::Concentricity => GdtCharacteristic::Concentricity,
        cx::Characteristic::Cylindricity => GdtCharacteristic::Cylindricity,
        cx::Characteristic::Flatness => GdtCharacteristic::Flatness,
        cx::Characteristic::LineProfile => GdtCharacteristic::ProfileOfLine,
        cx::Characteristic::Parallelism => GdtCharacteristic::Parallelism,
        cx::Characteristic::Perpendicularity => GdtCharacteristic::Perpendicularity,
        cx::Characteristic::Position => GdtCharacteristic::Position,
        cx::Characteristic::Roundness => GdtCharacteristic::Circularity,
        cx::Characteristic::Straightness => GdtCharacteristic::Straightness,
        cx::Characteristic::SurfaceProfile => GdtCharacteristic::ProfileOfSurface,
        cx::Characteristic::Symmetry => GdtCharacteristic::Symmetry,
        cx::Characteristic::TotalRunout => GdtCharacteristic::TotalRunout,
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

fn limits(id: &str, value: Tolerance) -> KernelResult<Option<cx::Limits>> {
    Ok(match value {
        Tolerance::None => None,
        Tolerance::Basic => Some(cx::Limits::Basic),
        Tolerance::Fit { class } => {
            Some(cx::Limits::Fit { deviation: class.deviation().to_owned(), grade: class.grade().to_owned() })
        }
        Tolerance::Symmetric { .. } | Tolerance::Deviation { .. } | Tolerance::Limits { .. } => {
            let (upper, lower) = value.deviations().unwrap_or_default();
            Some(cx::Limits::Deviations { upper: rational(id, upper)?, lower: rational(id, lower)? })
        }
    })
}

/// A file's limits as a tolerance, or `None` for a class the app does not write.
fn limits_back(value: &Option<cx::Limits>) -> Option<Tolerance> {
    Some(match value {
        None => Tolerance::None,
        Some(cx::Limits::Basic) => Tolerance::Basic,
        Some(cx::Limits::Fit { deviation, grade }) => {
            Tolerance::Fit { class: FitClass::parse(&format!("{deviation}{grade}"))? }
        }
        Some(cx::Limits::Deviations { upper, lower }) => {
            let (upper, lower) = (number(upper), number(lower));
            if upper > 0.0 && lower == -upper {
                Tolerance::Symmetric { value: upper }
            } else {
                Tolerance::Deviation { upper, lower }
            }
        }
    })
}

fn location<F: Clone>(
    id: &str,
    from: &Anchor<F>,
    to: &Anchor<F>,
    tolerance: Tolerance,
) -> KernelResult<cx::ItemKind<F>> {
    Ok(cx::ItemKind::Location(cx::Location {
        from: cx::Anchor { feature: from.feature.clone(), at: at(from.at) },
        to: cx::Anchor { feature: to.feature.clone(), at: at(to.at) },
        limits: limits(id, tolerance)?,
        nominal: None,
    }))
}

/// The items a file holds for one of the app's: one, or three for a slot. Nominal
/// values are left out: Catallax measures them.
fn file_items<F: Clone>(item: &PmiItem<F>) -> KernelResult<Vec<cx::Item<F>>> {
    let id = item.id.as_str();
    let one = |kind| Ok(vec![cx::Item { id: id.to_owned(), kind }]);
    match &item.kind {
        PmiKind::Datum { letter, feature } => {
            one(cx::ItemKind::Datum(cx::Datum { letter: letter.clone(), feature: feature.clone() }))
        }
        PmiKind::Size { size, features, tolerance, through, thread, .. } => one(cx::ItemKind::Size(cx::Size {
            kind: match size {
                SizeKind::Diameter => cx::SizeKind::Diameter,
                SizeKind::Radius => cx::SizeKind::Radius,
            },
            features: features.clone(),
            limits: limits(id, *tolerance)?,
            through: *through,
            thread: thread.clone(),
            nominal: None,
        })),
        // The scalar nominals API gives the pilot diameter, just as for any size.
        // on_model rejects native hole requirements before converting for STEP.
        PmiKind::Hole { pilot, .. } => one(cx::ItemKind::Size(cx::Size {
            kind: cx::SizeKind::Diameter,
            features: vec![pilot.clone()],
            limits: None,
            through: false,
            thread: None,
            nominal: None,
        })),
        PmiKind::Location { from, to, tolerance, .. } => one(location(id, from, to, *tolerance)?),
        PmiKind::Slot { flats, ends, tolerance } => {
            let end = |feature: &Vec<F>, at| Anchor { feature: feature.clone(), at };
            Ok(vec![
                cx::Item {
                    id: part_name(id, SLOT_LENGTH),
                    kind: location(id, &end(&ends[0], At::Outer), &end(&ends[1], At::Outer), *tolerance)?,
                },
                cx::Item {
                    id: part_name(id, SLOT_WIDTH),
                    kind: location(id, &end(&flats[0], At::Feature), &end(&flats[1], At::Feature), *tolerance)?,
                },
                cx::Item {
                    id: part_name(id, SLOT_RADIUS),
                    kind: cx::ItemKind::Size(cx::Size {
                        kind: cx::SizeKind::Radius,
                        features: ends.to_vec(),
                        limits: None,
                        through: false,
                        thread: None,
                        nominal: None,
                    }),
                },
            ])
        }
        PmiKind::Width { .. } | PmiKind::Chamfer { .. } | PmiKind::EdgeBreak { .. } | PmiKind::BasicAngle { .. } => {
            Err(invalid(id, &format!("{} PMI is not supported by this STEP kernel", item.kind_label())))
        }
        PmiKind::DatumTarget(_) | PmiKind::PositionControl(_) | PmiKind::Knurl { .. } | PmiKind::FaceNote { .. } => {
            Err(invalid(id, &format!("{} PMI is not supported by this STEP kernel", item.kind_label())))
        }
        PmiKind::Gdt { characteristic: which, tolerance, diameter, material, datums, features } => {
            if datums.iter().any(|datum| !datum.material.is_regardless() || datum.letter.contains('-')) {
                return Err(KernelError::Unsupported("a datum reference at a material condition".into()));
            }
            one(cx::ItemKind::Tolerance(cx::GeometricTolerance {
                characteristic: characteristic(*which),
                value: rational(id, *tolerance)?,
                diameter_zone: *diameter,
                material: match material {
                    MaterialCondition::Regardless => None,
                    MaterialCondition::Maximum => Some(cx::MaterialCondition::Maximum),
                    MaterialCondition::Least => Some(cx::MaterialCondition::Least),
                },
                datums: datums.iter().map(|datum| datum.letter.clone()).collect(),
                features: features.clone(),
            }))
        }
        PmiKind::SurfaceFinish { text, process, method, lay, allowance, all_around, feature } => {
            if !method.is_empty() || lay.is_some() || !allowance.is_empty() || *all_around {
                return Err(KernelError::Unsupported("a surface finish with a method, lay or allowance".into()));
            }
            let (parameter, value) = roughness(text)
                .ok_or_else(|| invalid(id, "the finish must read as a parameter and a value, such as Ra 1.6"))?;
            one(cx::ItemKind::Texture(cx::SurfaceTexture {
                parameter: parameter.to_owned(),
                value: Rational::parse(value.as_str()).map_err(|e| KernelError::InvalidInput(e.to_string()))?,
                process: match process {
                    SurfaceProcess::Any => cx::Process::Any,
                    SurfaceProcess::Removal => cx::Process::Removal,
                    SurfaceProcess::NoRemoval => cx::Process::NoRemoval,
                },
                feature: feature.clone(),
            }))
        }
    }
}

/// The app's information as a file holds it, and how many file items each of the
/// app's became.
fn to_catallax<F: Clone>(pmi: &Pmi<F>) -> KernelResult<(cx::Pmi<F>, Vec<usize>)> {
    let mut items = Vec::with_capacity(pmi.items.len());
    let mut counts = Vec::with_capacity(pmi.items.len());
    for item in &pmi.items {
        let held = file_items(item)?;
        counts.push(held.len());
        items.extend(held);
    }
    let part = cx::PartNotes {
        general_tolerance: pmi.part.general_tolerance.clone(),
        material: pmi.part.material.clone(),
        notes: pmi.part.notes.clone(),
    };
    Ok((cx::Pmi { items, part }, counts))
}

/// One of a file's items in the app's types, or `None` when a value of it has no form
/// the app writes.
fn app_item<F: Clone>(item: &cx::Item<F>) -> Option<PmiItem<F>> {
    let kind = match &item.kind {
        cx::ItemKind::Datum(datum) => PmiKind::Datum { letter: datum.letter.clone(), feature: datum.feature.clone() },
        cx::ItemKind::Size(size) => PmiKind::Size {
            size: match &size.kind {
                cx::SizeKind::Diameter => SizeKind::Diameter,
                cx::SizeKind::Radius => SizeKind::Radius,
                // The app does not yet import a compound STEP hole callout. Report it
                // as unread rather than turning it into a diameter and losing its requirements.
                cx::SizeKind::Hole(_) => return None,
            },
            features: size.features.clone(),
            tolerance: limits_back(&size.limits)?,
            through: size.through,
            thread: size.thread.clone(),
            nominal: size.nominal.as_ref().map(number),
        },
        cx::ItemKind::Location(location) => PmiKind::Location {
            from: Anchor { feature: location.from.feature.clone(), at: at_back(location.from.at) },
            to: Anchor { feature: location.to.feature.clone(), at: at_back(location.to.at) },
            tolerance: limits_back(&location.limits)?,
            nominal: location.nominal.as_ref().map(number),
        },
        cx::ItemKind::Tolerance(tolerance) => PmiKind::Gdt {
            characteristic: characteristic_back(tolerance.characteristic),
            tolerance: number(&tolerance.value),
            diameter: tolerance.diameter_zone,
            material: match tolerance.material {
                None => MaterialCondition::Regardless,
                Some(cx::MaterialCondition::Maximum) => MaterialCondition::Maximum,
                Some(cx::MaterialCondition::Least) => MaterialCondition::Least,
            },
            datums: tolerance.datums.iter().map(|letter| DatumRef::new(letter)).collect(),
            features: tolerance.features.clone(),
        },
        cx::ItemKind::Texture(texture) => PmiKind::SurfaceFinish {
            text: format!("{} {}", texture.parameter, Decimal::from_f64(number(&texture.value))?),
            process: match texture.process {
                cx::Process::Any => SurfaceProcess::Any,
                cx::Process::Removal => SurfaceProcess::Removal,
                cx::Process::NoRemoval => SurfaceProcess::NoRemoval,
            },
            method: String::new(),
            lay: None,
            allowance: String::new(),
            all_around: false,
            feature: texture.feature.clone(),
        },
    };
    Some(PmiItem { id: item.id.clone(), kind })
}

fn same_faces<F: PartialEq>(a: &[F], b: &[F]) -> bool {
    a.len() == b.len() && a.iter().all(|face| b.contains(face))
}

/// The slot the three dimensions named after `id` make, when they are a slot's: a
/// length over two round ends, a width between two flats under the same tolerance, and
/// the radius of those ends.
fn slot<F: Clone + PartialEq>(id: &str, items: &[PmiItem<F>]) -> Option<PmiItem<F>> {
    let part = |name: &str| items.iter().find(|item| item.id == part_name(id, name)).map(|item| &item.kind);
    let PmiKind::Location { from: first, to: second, tolerance, .. } = part(SLOT_LENGTH)? else { return None };
    let PmiKind::Location { from: low, to: high, tolerance: across, .. } = part(SLOT_WIDTH)? else { return None };
    let PmiKind::Size { size: SizeKind::Radius, features, tolerance: Tolerance::None, through: false, .. } =
        part(SLOT_RADIUS)?
    else {
        return None;
    };
    let round = first.at == At::Outer && second.at == At::Outer;
    let flat = low.at == At::Feature && high.at == At::Feature;
    let ends = features.len() == 2
        && features.iter().any(|feature| same_faces(feature, &first.feature))
        && features.iter().any(|feature| same_faces(feature, &second.feature));
    (round && flat && ends && tolerance == across).then(|| PmiItem {
        id: id.to_owned(),
        kind: PmiKind::Slot {
            flats: [low.feature.clone(), high.feature.clone()],
            ends: [first.feature.clone(), second.feature.clone()],
            tolerance: *tolerance,
        },
    })
}

/// A file's information in the app's types: slots put back together, and the names of
/// the items whose values the app has no form for.
fn from_catallax<F: Clone + PartialEq>(pmi: &cx::Pmi<F>) -> (Pmi<F>, Vec<String>) {
    let mut lost = Vec::new();
    let mut read = Vec::with_capacity(pmi.items.len());
    for item in &pmi.items {
        match app_item(item) {
            Some(item) => read.push(item),
            None => lost.push(item.id.clone()),
        }
    }
    let mut items: Vec<PmiItem<F>> = Vec::with_capacity(read.len());
    for item in &read {
        let Some((id, _)) = item.id.rsplit_once('/') else {
            items.push(item.clone());
            continue;
        };
        match slot(id, &read) {
            Some(slot) if !items.iter().any(|placed| placed.id == slot.id) => items.push(slot),
            Some(_) => {}
            None => items.push(item.clone()),
        }
    }
    let part = PartNotes {
        general_tolerance: pmi.part.general_tolerance.clone(),
        material: pmi.part.material.clone(),
        notes: pmi.part.notes.clone(),
    };
    (Pmi { items, part }, lost)
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

/// The app's information as a file holds it, on the faces of this solid's model, and
/// how many file items each of the app's became.
fn held(
    solid: &Solid,
    body: BodyId,
    pmi: &Pmi<FaceId>,
) -> KernelResult<(cx::Pmi<catallax_topology::FaceId>, Vec<usize>)> {
    let named = pmi.try_map(|_, face| solid.cat_face(*face, body))?;
    to_catallax(&named)
}

/// The app's information on the faces of this solid's model.
pub(super) fn on_model(
    solid: &Solid,
    body: BodyId,
    pmi: &Pmi<FaceId>,
) -> KernelResult<cx::Pmi<catallax_topology::FaceId>> {
    if let Some(item) = pmi.items.iter().find(|item| matches!(item.kind, PmiKind::Hole { .. })) {
        return Err(invalid(&item.id, "compound hole and depth PMI is not supported in STEP export"));
    }
    held(solid, body, pmi).map(|(pmi, _)| pmi)
}

/// The values of every item, measured on the solid: one for a size or a distance; the
/// length, the width and the end radius for a slot; none for anything else.
pub(super) fn nominals(solid: &Solid, body: BodyId, pmi: &Pmi<FaceId>) -> KernelResult<Vec<Vec<f64>>> {
    let (pmi, counts) = held(solid, body, pmi)?;
    pmi.validate().map_err(|e| error("pmi_nominals", e))?;
    let built = solid.app.checkpoint.built();
    let answer =
        numerical("pmi_nominals", |dec| cx::nominals(&built.model, &pmi, dec).map(catallax_numbers::Answer::exact))
            .map_err(|e| error("pmi_nominals", e))?;
    let mut measured = answer.value.into_iter();
    Ok(counts
        .into_iter()
        .map(|count| measured.by_ref().take(count).flatten().map(|real| real.to_f64()).collect())
        .collect())
}

/// Reads a STEP file's manufacturing information. Face keys are the entity names the
/// file's import gives its faces.
pub(super) fn read(source: &[u8]) -> KernelResult<StepPmi> {
    let read = catallax_import::read_step_pmi(source)
        .map_err(|e| KernelError::InvalidInput(format!("This STEP file could not be read: {e:?}")))?;
    let mut skipped: Vec<SkippedPmi> =
        read.skipped.into_iter().map(|s| SkippedPmi { entity: s.entity, kind: s.kind, reason: s.reason }).collect();
    // An item whose values the app has no form for is left out by name, like any other.
    let (pmi, lost) = from_catallax(&read.pmi);
    skipped.extend(lost.into_iter().map(|id| SkippedPmi {
        entity: id,
        kind: "value".into(),
        reason: "a value has no form that can be shown".into(),
    }));
    Ok(StepPmi { pmi, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot_item() -> PmiItem<u32> {
        PmiItem {
            id: "Slot".to_owned(),
            kind: PmiKind::Slot {
                flats: [vec![1], vec![2]],
                ends: [vec![3], vec![4]],
                tolerance: Tolerance::Deviation { upper: 0.1, lower: 0.0 },
            },
        }
    }

    #[test]
    fn a_slot_is_three_dimensions_in_a_file_and_one_item_again_when_read() {
        let pmi = Pmi { items: vec![slot_item()], part: PartNotes::default() };
        let (held, counts) = to_catallax(&pmi).unwrap();
        assert_eq!(counts, [3]);
        let names: Vec<&str> = held.items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(names, ["Slot/length", "Slot/width", "Slot/end radius"]);
        let (read, lost) = from_catallax(&held);
        assert!(lost.is_empty());
        assert_eq!(read, pmi);
    }

    #[test]
    fn dimensions_that_only_share_a_name_stay_apart() {
        let pmi = Pmi { items: vec![slot_item()], part: PartNotes::default() };
        let (mut held, _) = to_catallax(&pmi).unwrap();
        // A width under another tolerance is not this slot's.
        if let cx::ItemKind::Location(width) = &mut held.items[1].kind {
            width.limits = None;
        }
        let (read, _) = from_catallax(&held);
        assert_eq!(read.items.len(), 3);
    }
}
