//! Nominal CAD comparisons, not inspection results. Measurement providers supply checked
//! geometry and independent design targets; the current shape cannot define its own target.

use soliddraft_kernel_api::pmi::{
    Anchor, DatumRef, Feature, GdtCharacteristic, Pmi, PmiItem, PmiKind, SizeKind, Tolerance,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Indeterminate {
    UnresolvedFace,
    UnresolvedDatum,
    MissingNominal,
    MissingMeasurement,
    InvalidNominal,
    InvalidMeasurement,
    InvalidLimits,
    DatumFrameMismatch,
    Unsupported(&'static str),
}

impl Indeterminate {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::UnresolvedFace => "A controlled face could not be resolved.",
            Self::UnresolvedDatum => "A datum could not be resolved.",
            Self::MissingNominal => "No independent design nominal is available.",
            Self::MissingMeasurement => "No checked CAD measurement is available.",
            Self::InvalidNominal => "The design nominal is not a valid number for this check.",
            Self::InvalidMeasurement => "The checked CAD measurement is not a valid number for this check.",
            Self::InvalidLimits => "A valid numerical tolerance interval could not be established.",
            Self::DatumFrameMismatch => "The design target and measured axis must use the specified datum order.",
            Self::Unsupported(reason) => reason,
        }
    }
}

/// Inclusive limits in millimetres. Position reports diametral error, not radial error.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Comparison {
    pub(crate) measured_mm: f64,
    pub(crate) lower_mm: f64,
    pub(crate) upper_mm: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Status {
    InTolerance(Comparison),
    OutOfTolerance(Comparison),
    Indeterminate(Indeterminate),
}

/// A known failure takes precedence; any unchecked contributor prevents a passing face.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Verdict {
    InTolerance,
    Indeterminate,
    OutOfTolerance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckKind {
    Cylinder(SizeKind),
    Location,
    SlotLength,
    SlotWidth,
    Geometric(GdtCharacteristic),
    HoleDepth,
    HoleHead,
    HoleThread,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Contributor<'a> {
    pub(crate) item_id: &'a str,
    pub(crate) kind: CheckKind,
    /// Zero-based feature index for a size or geometric control, including a pattern.
    pub(crate) member: Option<usize>,
    pub(crate) status: Status,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct FaceStatus<'a> {
    pub(crate) contributors: Vec<Contributor<'a>>,
}

impl FaceStatus<'_> {
    /// No applicable tolerance is not a passing result.
    pub(crate) fn verdict(&self) -> Option<Verdict> {
        self.contributors
            .iter()
            .map(|control| match control.status {
                Status::InTolerance(_) => Verdict::InTolerance,
                Status::OutOfTolerance(_) => Verdict::OutOfTolerance,
                Status::Indeterminate(_) => Verdict::Indeterminate,
            })
            .max()
    }
}

pub(crate) enum DimensionCheck<'a, F> {
    Cylinder { size: SizeKind, member: usize, faces: &'a [F] },
    Location { from: &'a Anchor<F>, to: &'a Anchor<F> },
    SlotLength(&'a [Feature<F>; 2]),
    SlotWidth(&'a [Feature<F>; 2]),
}

pub(crate) struct DimensionMeasurement {
    pub(crate) measured_mm: f64,
    /// Independently specified target, if one is known outside the PMI item.
    pub(crate) nominal_mm: Option<f64>,
}

/// Ordered primary, secondary and tertiary planar datums. The provider must resolve
/// them and establish the same frame convention for the basic target and measured axis.
pub(crate) type DatumFrame<'a> = [&'a str; 3];

pub(crate) struct PositionCheck<'a, F> {
    pub(crate) member: usize,
    pub(crate) faces: &'a [F],
    pub(crate) frame: DatumFrame<'a>,
}

pub(crate) struct PositionMeasurement<'a> {
    pub(crate) nominal_frame: DatumFrame<'a>,
    pub(crate) nominal_xy_mm: [f64; 2],
    pub(crate) measured_frame: DatumFrame<'a>,
    /// Endpoints of the checked cylindrical axis over the controlled feature's full
    /// extent, in the transverse plane of a nominal axis normal to the primary datum.
    pub(crate) axis_xy_mm: [[f64; 2]; 2],
}

/// Compare every applicable control on a durable face reference, in PMI order. Providers
/// must refuse unresolved references and unsupported geometry (including noncylindrical
/// position features), not approximate measurements from a tessellation.
pub(crate) fn face_status<'a, F: PartialEq>(
    pmi: &'a Pmi<F>,
    face: &F,
    mut dimension: impl FnMut(&'a PmiItem<F>, DimensionCheck<'a, F>) -> Result<DimensionMeasurement, Indeterminate>,
    mut position: impl FnMut(&'a PmiItem<F>, PositionCheck<'a, F>) -> Result<PositionMeasurement<'a>, Indeterminate>,
) -> FaceStatus<'a> {
    let mut result = FaceStatus::default();
    for item in &pmi.items {
        let mut add = |kind, member, status: Result<Status, Indeterminate>| {
            result.contributors.push(Contributor {
                item_id: &item.id,
                kind,
                member,
                status: status.unwrap_or_else(Status::Indeterminate),
            });
        };
        let mut dimension =
            |tolerance, nominal, request| compare_dimension(tolerance, nominal, || dimension(item, request));
        match &item.kind {
            PmiKind::Size { size, features, tolerance, thread, nominal, .. } if *tolerance != Tolerance::Basic => {
                for (member, faces) in features.iter().enumerate().filter(|(_, faces)| faces.contains(face)) {
                    let status = if thread.is_some() {
                        Err(Indeterminate::Unsupported("Threaded feature size is not checked."))
                    } else {
                        dimension(*tolerance, *nominal, DimensionCheck::Cylinder { size: *size, member, faces })
                    };
                    add(CheckKind::Cylinder(*size), Some(member), status);
                }
            }
            PmiKind::Hole { pilot, head, tolerance, through, thread, .. } => {
                if pilot.contains(face) {
                    if *tolerance != Tolerance::Basic {
                        let size = SizeKind::Diameter;
                        add(
                            CheckKind::Cylinder(size),
                            Some(0),
                            dimension(*tolerance, None, DimensionCheck::Cylinder { size, member: 0, faces: pilot }),
                        );
                    }
                    if !through {
                        add(CheckKind::HoleDepth, None, Err(Indeterminate::Unsupported("Drill depth is not checked.")));
                    }
                    if thread.is_some() {
                        add(
                            CheckKind::HoleThread,
                            None,
                            Err(Indeterminate::Unsupported("Thread requirements are not checked.")),
                        );
                    }
                }
                if head.as_ref().is_some_and(|head| head.faces.contains(face)) {
                    add(
                        CheckKind::HoleHead,
                        None,
                        Err(Indeterminate::Unsupported("Hole head dimensions are not checked.")),
                    );
                }
            }
            PmiKind::Location { from, to, tolerance, nominal }
                if *tolerance != Tolerance::Basic && (from.feature.contains(face) || to.feature.contains(face)) =>
            {
                add(CheckKind::Location, None, dimension(*tolerance, *nominal, DimensionCheck::Location { from, to }));
            }
            PmiKind::Slot { flats, ends, tolerance } if *tolerance != Tolerance::Basic => {
                if flats.iter().any(|faces| faces.contains(face)) {
                    add(CheckKind::SlotWidth, None, dimension(*tolerance, None, DimensionCheck::SlotWidth(flats)));
                }
                if ends.iter().any(|faces| faces.contains(face)) {
                    add(CheckKind::SlotLength, None, dimension(*tolerance, None, DimensionCheck::SlotLength(ends)));
                }
            }
            PmiKind::Gdt { characteristic, tolerance, diameter, material, datums, features } => {
                for (member, faces) in features.iter().enumerate().filter(|(_, faces)| faces.contains(face)) {
                    let status = if *characteristic != GdtCharacteristic::Position {
                        Err(Indeterminate::Unsupported("This geometric characteristic is not checked."))
                    } else if !diameter {
                        Err(Indeterminate::Unsupported("Only cylindrical position zones are checked."))
                    } else if !material.is_regardless() {
                        Err(Indeterminate::Unsupported("Position with a material modifier is not checked."))
                    } else {
                        datum_frame(datums).and_then(|frame| {
                            compare_position(*tolerance, frame, || {
                                position(item, PositionCheck { member, faces, frame })
                            })
                        })
                    };
                    add(CheckKind::Geometric(*characteristic), Some(member), status);
                }
            }
            // Datum definitions, finish requirements and basic dimensions do not
            // themselves give a numerical tolerance to check.
            PmiKind::Datum { .. }
            | PmiKind::SurfaceFinish { .. }
            | PmiKind::Size { .. }
            | PmiKind::Location { .. }
            | PmiKind::Slot { .. } => {}
        }
    }
    result
}

fn compare_dimension(
    tolerance: Tolerance,
    nominal: Option<f64>,
    measure: impl FnOnce() -> Result<DimensionMeasurement, Indeterminate>,
) -> Result<Status, Indeterminate> {
    if tolerance.problem().is_some() {
        return Err(Indeterminate::InvalidLimits);
    }
    let (upper, lower) = tolerance.deviations().ok_or(Indeterminate::Unsupported(match tolerance {
        Tolerance::Fit { .. } => "The fit class has no checked numerical limits.",
        _ => "No numerical tolerance limits are specified for this dimension.",
    }))?;
    let measured = measure()?;
    let nominal = nominal.or(measured.nominal_mm).ok_or(Indeterminate::MissingNominal)?;
    if !nominal.is_finite() || nominal < 0.0 {
        return Err(Indeterminate::InvalidNominal);
    }
    compare(measured.measured_mm, nominal + lower, nominal + upper)
}

fn datum_frame(datums: &[DatumRef]) -> Result<DatumFrame<'_>, Indeterminate> {
    let [a, b, c] = datums else {
        return Err(Indeterminate::Unsupported("Position requires three ordered datums for this check."));
    };
    let frame = [a.letter.as_str(), b.letter.as_str(), c.letter.as_str()];
    if datums.iter().any(|datum| !datum.material.is_regardless()) {
        return Err(Indeterminate::Unsupported("Datum material modifiers are not checked."));
    }
    if frame.iter().any(|letter| letter.is_empty() || letter.contains('-'))
        || frame[0] == frame[1]
        || frame[0] == frame[2]
        || frame[1] == frame[2]
    {
        return Err(Indeterminate::Unsupported("Position requires distinct, single datum references."));
    }
    Ok(frame)
}

fn compare_position<'a>(
    tolerance: f64,
    frame: DatumFrame<'a>,
    measure: impl FnOnce() -> Result<PositionMeasurement<'a>, Indeterminate>,
) -> Result<Status, Indeterminate> {
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(Indeterminate::InvalidLimits);
    }
    let measured = measure()?;
    if measured.nominal_frame != frame || measured.measured_frame != frame {
        return Err(Indeterminate::DatumFrameMismatch);
    }
    let [x, y] = measured.nominal_xy_mm;
    if !x.is_finite() || !y.is_finite() {
        return Err(Indeterminate::InvalidNominal);
    }
    if !measured.axis_xy_mm.iter().flatten().all(|v| v.is_finite()) {
        return Err(Indeterminate::InvalidMeasurement);
    }
    // The maximum distance of a straight axis from a parallel line occurs at an endpoint.
    let [start, end] = measured.axis_xy_mm.map(|[mx, my]| (mx - x).hypot(my - y));
    compare(2.0 * start.max(end), 0.0, tolerance)
}

fn compare(measured: f64, lower: f64, upper: f64) -> Result<Status, Indeterminate> {
    if !measured.is_finite() || measured < 0.0 {
        return Err(Indeterminate::InvalidMeasurement);
    }
    if !lower.is_finite() || !upper.is_finite() || lower >= upper {
        return Err(Indeterminate::InvalidLimits);
    }
    let values = Comparison { measured_mm: measured, lower_mm: lower, upper_mm: upper };
    Ok(if (lower..=upper).contains(&measured) { Status::InTolerance(values) } else { Status::OutOfTolerance(values) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::pmi::{At, MaterialCondition};

    #[test]
    fn native_hole_does_not_turn_unchecked_head_or_depth_into_a_pass() {
        use soliddraft_kernel_api::pmi::{HoleHead, HoleHeadKind};

        let pmi = Pmi {
            items: vec![PmiItem {
                id: "Compound hole".into(),
                kind: PmiKind::Hole {
                    pilot: vec![1],
                    head: Some(HoleHead { kind: HoleHeadKind::Counterbore, faces: vec![2] }),
                    tolerance: Tolerance::Deviation { lower: -0.1, upper: 0.1 },
                    through: false,
                    drill_depth: None,
                    thread: None,
                },
            }],
            part: Default::default(),
        };
        let status = |face| {
            face_status(
                &pmi,
                &face,
                |_, _| Ok(DimensionMeasurement { measured_mm: 4.0, nominal_mm: Some(4.0) }),
                |_, _| panic!("no position constraint"),
            )
        };
        let pilot = status(1);
        assert!(matches!(pilot.contributors[0].status, Status::InTolerance(_)));
        assert_eq!(pilot.contributors[1].kind, CheckKind::HoleDepth);
        assert_eq!(pilot.verdict(), Some(Verdict::Indeterminate));
        let head = status(2);
        assert_eq!(head.contributors[0].kind, CheckKind::HoleHead);
        assert_eq!(head.verdict(), Some(Verdict::Indeterminate));
        assert_eq!(status(3).verdict(), None);
    }

    fn diameter(tolerance: Tolerance, nominal: Option<f64>) -> PmiItem<u8> {
        PmiItem {
            id: "Hole diameter".into(),
            kind: PmiKind::Size {
                size: SizeKind::Diameter,
                features: vec![vec![1]],
                tolerance,
                nominal,
                through: true,
                thread: None,
            },
        }
    }

    fn position() -> PmiItem<u8> {
        PmiItem {
            id: "Bolt position".into(),
            kind: PmiKind::Gdt {
                characteristic: GdtCharacteristic::Position,
                tolerance: 1.25,
                diameter: true,
                material: MaterialCondition::Regardless,
                datums: ["A", "B", "C"].map(DatumRef::new).into(),
                features: vec![vec![3], vec![1, 2]],
            },
        }
    }

    fn axis<'a>(frame: DatumFrame<'a>, end: [f64; 2]) -> PositionMeasurement<'a> {
        PositionMeasurement {
            nominal_frame: ["A", "B", "C"],
            nominal_xy_mm: [10.0, 20.0],
            measured_frame: frame,
            axis_xy_mm: [[10.0, 20.0], end],
        }
    }

    #[test]
    fn asymmetric_size_limits_include_both_boundaries_without_slack() {
        let pmi = Pmi {
            items: vec![diameter(Tolerance::Deviation { lower: -0.05, upper: 0.1 }, Some(10.0))],
            ..Pmi::default()
        };
        for (measured, inside) in
            [(9.95_f64.next_down(), false), (9.95, true), (10.0, true), (10.1, true), (10.1_f64.next_up(), false)]
        {
            let found = face_status(
                &pmi,
                &1,
                |_, check| {
                    assert!(matches!(
                        check,
                        DimensionCheck::Cylinder { size: SizeKind::Diameter, member: 0, faces: [1] }
                    ));
                    Ok(DimensionMeasurement { measured_mm: measured, nominal_mm: None })
                },
                |_, _| unreachable!(),
            );
            let values = Comparison { measured_mm: measured, lower_mm: 9.95, upper_mm: 10.1 };
            let expected = if inside { Status::InTolerance(values) } else { Status::OutOfTolerance(values) };
            assert_eq!(found.contributors[0].status, expected);
        }
    }

    #[test]
    fn position_checks_both_axis_ends_for_the_selected_member_in_datum_order() {
        let pmi = Pmi { items: vec![position()], ..Pmi::default() };
        for (end, expected) in [
            ([10.125, 20.0], Status::InTolerance(Comparison { measured_mm: 0.25, lower_mm: 0.0, upper_mm: 1.25 })),
            ([10.375, 20.5], Status::InTolerance(Comparison { measured_mm: 1.25, lower_mm: 0.0, upper_mm: 1.25 })),
            ([10.75, 20.0], Status::OutOfTolerance(Comparison { measured_mm: 1.5, lower_mm: 0.0, upper_mm: 1.25 })),
        ] {
            let found = face_status(
                &pmi,
                &2,
                |_, _| unreachable!(),
                |_, check| {
                    assert_eq!((check.member, check.faces, check.frame), (1, &[1, 2][..], ["A", "B", "C"]));
                    let mut measurement = axis(check.frame, end);
                    if end[0] > 10.5 {
                        measurement.axis_xy_mm.swap(0, 1);
                    }
                    Ok(measurement)
                },
            );
            assert_eq!(
                found.contributors,
                [Contributor {
                    item_id: "Bolt position",
                    kind: CheckKind::Geometric(GdtCharacteristic::Position),
                    member: Some(1),
                    status: expected,
                }]
            );
        }
        let found = face_status(&pmi, &2, |_, _| unreachable!(), |_, _| Ok(axis(["A", "C", "B"], [10.0, 20.0])));
        assert_eq!(found.contributors[0].status, Status::Indeterminate(Indeterminate::DatumFrameMismatch));
    }

    #[test]
    fn width_uses_both_flats_without_treating_the_ends_as_width_faces() {
        let pmi = Pmi {
            items: vec![PmiItem {
                id: "Slot".into(),
                kind: PmiKind::Slot {
                    flats: [vec![1, 2], vec![3]],
                    ends: [vec![4], vec![5]],
                    tolerance: Tolerance::Limits { lower: 0.0, upper: 0.125 },
                },
            }],
            ..Pmi::default()
        };
        let width = face_status(
            &pmi,
            &2,
            |_, check| {
                let DimensionCheck::SlotWidth(flats) = check else { panic!("slot flat must request width") };
                assert_eq!(flats, &[vec![1, 2], vec![3]]);
                Ok(DimensionMeasurement { measured_mm: 6.125, nominal_mm: Some(6.0) })
            },
            |_, _| unreachable!(),
        );
        assert_eq!(width.contributors[0].kind, CheckKind::SlotWidth);
        assert_eq!(width.verdict(), Some(Verdict::InTolerance));
        let length = face_status(
            &pmi,
            &4,
            |_, check| {
                let DimensionCheck::SlotLength(ends) = check else { panic!("slot end must request length") };
                assert_eq!(ends, &[vec![4], vec![5]]);
                Err(Indeterminate::MissingMeasurement)
            },
            |_, _| unreachable!(),
        );
        assert_eq!(length.contributors[0].status, Status::Indeterminate(Indeterminate::MissingMeasurement));
    }

    #[test]
    fn unresolved_and_unsupported_controls_remain_visible_on_a_measured_face() {
        let mut flatness = position();
        flatness.id = "Flatness".into();
        let PmiKind::Gdt { characteristic, .. } = &mut flatness.kind else { unreachable!() };
        *characteristic = GdtCharacteristic::Flatness;
        let pmi = Pmi {
            items: vec![diameter(Tolerance::Symmetric { value: 0.25 }, Some(4.0)), position(), flatness],
            ..Pmi::default()
        };
        let evaluate = |measured_mm, refusal| {
            face_status(
                &pmi,
                &1,
                |_, _| Ok(DimensionMeasurement { measured_mm, nominal_mm: None }),
                |_, _| Err(refusal),
            )
        };
        let found = evaluate(4.0, Indeterminate::UnresolvedFace);
        assert_eq!(found.verdict(), Some(Verdict::Indeterminate));
        let [size, position, form] = found.contributors.as_slice() else { panic!("one result per control") };
        assert_eq!(size.item_id, "Hole diameter");
        assert!(matches!(size.status, Status::InTolerance(_)));
        assert_eq!(
            (position.item_id, position.status),
            ("Bolt position", Status::Indeterminate(Indeterminate::UnresolvedFace))
        );
        assert_eq!(
            (form.item_id, form.status),
            (
                "Flatness",
                Status::Indeterminate(Indeterminate::Unsupported("This geometric characteristic is not checked."))
            )
        );
        let outside = evaluate(5.0, Indeterminate::UnresolvedDatum);
        assert_eq!(outside.verdict(), Some(Verdict::OutOfTolerance));
        assert_eq!(outside.contributors[1].status, Status::Indeterminate(Indeterminate::UnresolvedDatum));
        assert_eq!(Indeterminate::UnresolvedFace.message(), "A controlled face could not be resolved.");
    }

    #[test]
    fn basic_location_is_not_a_control_and_an_actual_size_is_not_its_own_nominal() {
        let anchor = |face| Anchor { feature: vec![face], at: At::Feature };
        let basic = PmiItem {
            id: "Basic".into(),
            kind: PmiKind::Location { from: anchor(1), to: anchor(2), tolerance: Tolerance::Basic, nominal: Some(3.0) },
        };
        let pmi = Pmi { items: vec![basic, diameter(Tolerance::Symmetric { value: 0.25 }, None)], ..Pmi::default() };
        let found = face_status(
            &pmi,
            &1,
            |_, _| Ok(DimensionMeasurement { measured_mm: 4.0, nominal_mm: None }),
            |_, _| unreachable!(),
        );
        assert_eq!(found.contributors.len(), 1);
        assert_eq!(found.contributors[0].status, Status::Indeterminate(Indeterminate::MissingNominal));
        assert_eq!(face_status(&pmi, &2, |_, _| unreachable!(), |_, _| unreachable!()).verdict(), None);
    }
}
