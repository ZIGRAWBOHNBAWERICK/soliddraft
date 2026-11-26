//! Signed linear worst-case stacks of explicitly selected dimensions, in millimetres.
//!
//! For independent limits, the minimum (maximum) is the sum of each link's signed
//! minimum (maximum). Subtracting a dimension swaps its endpoints. The caller
//! supplies the path and directions; this calculation does not resolve geometry.

use soliddraft_kernel_api::pmi::{PmiItem, PmiKind};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackDirection {
    Forward,
    Reverse,
}

pub struct StackLink<'a, F> {
    pub item: &'a PmiItem<F>,
    pub direction: StackDirection,
    /// Checked, unsigned nominal for the selected control. `None` means it could
    /// not be resolved; the stack does not substitute an item value or assume zero.
    pub nominal_mm: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StackRange {
    pub nominal_mm: f64,
    pub min_mm: f64,
    pub max_mm: f64,
}

impl StackRange {
    fn from_deviations(nominal_mm: f64, lower_mm: f64, upper_mm: f64) -> Option<Self> {
        let min_mm = nominal_mm + lower_mm;
        let max_mm = nominal_mm + upper_mm;
        (nominal_mm.is_finite() && min_mm.is_finite() && max_mm.is_finite()).then_some(Self {
            nominal_mm,
            min_mm,
            max_mm,
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct StackContribution<'a> {
    pub item_id: &'a str,
    pub range: StackRange,
}

#[derive(Debug, PartialEq)]
pub struct StackResult<'a> {
    pub range: StackRange,
    /// Signed contributions in input order.
    pub contributions: Vec<StackContribution<'a>>,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum StackError {
    #[error("no dimensions selected")]
    Empty,
    #[error("stack link {index}: {reason}")]
    Link {
        /// Zero-based index into the selected links.
        index: usize,
        reason: StackLinkError,
    },
    #[error("stack totals are not finite")]
    NonFiniteTotal,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum StackLinkError {
    #[error("control is not a single linear dimension")]
    Unsupported,
    #[error("nominal is missing")]
    MissingNominal,
    #[error("nominal must be finite and non-negative")]
    InvalidNominal,
    #[error("numeric tolerance limits are missing")]
    MissingLimits,
    #[error("tolerance deviations must be finite and ordered")]
    InvalidLimits,
    #[error("signed bounds are not finite")]
    NonFiniteRange,
}

/// Calculate the nominal and worst-case bounds. Any unknown or unsupported link
/// prevents a numerical result. Only explicit numeric tolerances are used.
pub fn signed_worst_case<'a, F>(links: &[StackLink<'a, F>]) -> Result<StackResult<'a>, StackError> {
    if links.is_empty() {
        return Err(StackError::Empty);
    }

    // Sum deviations separately so small tolerances survive cancellation of nominal lengths.
    let (mut nominal_mm, mut lower_mm, mut upper_mm) = (0.0, 0.0, 0.0);
    let mut contributions = Vec::with_capacity(links.len());
    for (index, link) in links.iter().enumerate() {
        let error = |reason| StackError::Link { index, reason };
        let tolerance = match &link.item.kind {
            PmiKind::Size { features, tolerance, .. } if features.len() == 1 => tolerance,
            PmiKind::Location { tolerance, .. } => tolerance,
            _ => return Err(error(StackLinkError::Unsupported)),
        };
        let nominal = link.nominal_mm.ok_or_else(|| error(StackLinkError::MissingNominal))?;
        if !nominal.is_finite() || nominal < 0.0 {
            return Err(error(StackLinkError::InvalidNominal));
        }
        let (upper, lower) = tolerance.deviations().ok_or_else(|| error(StackLinkError::MissingLimits))?;
        if !lower.is_finite() || !upper.is_finite() || lower > upper {
            return Err(error(StackLinkError::InvalidLimits));
        }
        let (nominal, lower, upper) = match link.direction {
            StackDirection::Forward => (nominal, lower, upper),
            StackDirection::Reverse => (-nominal, -upper, -lower),
        };
        let range =
            StackRange::from_deviations(nominal, lower, upper).ok_or_else(|| error(StackLinkError::NonFiniteRange))?;
        contributions.push(StackContribution { item_id: &link.item.id, range });
        nominal_mm += nominal;
        lower_mm += lower;
        upper_mm += upper;
    }
    let range = StackRange::from_deviations(nominal_mm, lower_mm, upper_mm).ok_or(StackError::NonFiniteTotal)?;
    Ok(StackResult { range, contributions })
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::pmi::{Anchor, At, Tolerance};

    fn distance(id: &str, nominal: f64, tolerance: Tolerance) -> PmiItem<u8> {
        PmiItem {
            id: id.into(),
            kind: PmiKind::Location {
                from: Anchor { feature: vec![1], at: At::Feature },
                to: Anchor { feature: vec![2], at: At::Feature },
                tolerance,
                nominal: Some(nominal),
            },
        }
    }

    fn range(nominal_mm: f64, min_mm: f64, max_mm: f64) -> StackRange {
        StackRange { nominal_mm, min_mm, max_mm }
    }

    #[test]
    fn flange_and_plate_clearance_matches_hand_calculation() {
        let opening = distance("opening", 40.0, Tolerance::Limits { lower: -0.125, upper: 0.25 });
        let flange = distance("flange", 12.0, Tolerance::Deviation { lower: -0.25, upper: 0.125 });
        let plate = distance("plate", 20.0, Tolerance::Deviation { lower: -0.5, upper: 0.25 });
        let forward = StackDirection::Forward;
        let reverse = StackDirection::Reverse;
        let mut links = [
            StackLink { item: &opening, direction: forward, nominal_mm: Some(40.0) },
            StackLink { item: &flange, direction: reverse, nominal_mm: Some(12.0) },
            StackLink { item: &plate, direction: reverse, nominal_mm: Some(20.0) },
        ];
        let clearance = signed_worst_case(&links).unwrap();
        // 40 - 12 - 20; min = 39.875 - 12.125 - 20.25; max = 40.25 - 11.75 - 19.5.
        assert_eq!(clearance.range, range(8.0, 7.5, 9.0));
        assert_eq!(
            clearance.contributions,
            [
                StackContribution { item_id: "opening", range: range(40.0, 39.875, 40.25) },
                StackContribution { item_id: "flange", range: range(-12.0, -12.125, -11.75) },
                StackContribution { item_id: "plate", range: range(-20.0, -20.25, -19.5) },
            ]
        );

        links[0].direction = reverse;
        links[1].direction = forward;
        links[2].direction = forward;
        assert_eq!(signed_worst_case(&links).unwrap().range, range(-8.0, -9.0, -7.5));
    }

    #[test]
    fn a_missing_limit_refuses_the_whole_stack() {
        let opening = distance("opening", 40.0, Tolerance::Symmetric { value: 0.25 });
        let plate = distance("plate", 20.0, Tolerance::None);
        let links = [
            StackLink { item: &opening, direction: StackDirection::Forward, nominal_mm: Some(40.0) },
            StackLink { item: &plate, direction: StackDirection::Reverse, nominal_mm: Some(20.0) },
        ];
        assert_eq!(
            signed_worst_case(&links),
            Err(StackError::Link { index: 1, reason: StackLinkError::MissingLimits })
        );
    }

    #[test]
    fn unresolved_nominal_is_not_replaced_by_stored_value() {
        let plate = distance("plate", 20.0, Tolerance::Symmetric { value: 0.25 });
        let link = StackLink { item: &plate, direction: StackDirection::Reverse, nominal_mm: None };
        assert_eq!(
            signed_worst_case(&[link]),
            Err(StackError::Link { index: 0, reason: StackLinkError::MissingNominal })
        );
    }

    #[test]
    fn a_multi_dimension_control_is_not_a_single_stack_link() {
        let slot = PmiItem {
            id: "slot".into(),
            kind: PmiKind::Slot {
                flats: [vec![1], vec![2]],
                ends: [vec![3], vec![4]],
                tolerance: Tolerance::Symmetric { value: 0.25 },
            },
        };
        let link = StackLink { item: &slot, direction: StackDirection::Forward, nominal_mm: Some(20.0) };
        assert_eq!(signed_worst_case(&[link]), Err(StackError::Link { index: 0, reason: StackLinkError::Unsupported }));
    }

    #[test]
    fn unilateral_size_limits_can_exclude_nominal() {
        let bore = PmiItem {
            id: "bore".into(),
            kind: PmiKind::Size {
                size: soliddraft_kernel_api::pmi::SizeKind::Diameter,
                features: vec![vec![1]],
                tolerance: Tolerance::Deviation { lower: 0.125, upper: 0.25 },
                through: true,
                thread: None,
                nominal: None,
            },
        };
        let link = StackLink { item: &bore, direction: StackDirection::Reverse, nominal_mm: Some(10.0) };
        assert_eq!(signed_worst_case(&[link]).unwrap().range, range(-10.0, -10.25, -10.125));
    }

    #[test]
    fn invalid_limits_refuse_a_range() {
        for tolerance in
            [Tolerance::Limits { lower: 0.25, upper: -0.25 }, Tolerance::Deviation { lower: f64::NAN, upper: 0.25 }]
        {
            let plate = distance("plate", 20.0, tolerance);
            let link = StackLink { item: &plate, direction: StackDirection::Reverse, nominal_mm: Some(20.0) };
            assert_eq!(
                signed_worst_case(&[link]),
                Err(StackError::Link { index: 0, reason: StackLinkError::InvalidLimits })
            );
        }
    }

    #[test]
    fn cancelling_nominals_do_not_cancel_small_tolerances() {
        let opening = distance("opening", 1.0, Tolerance::Symmetric { value: 2.0_f64.powi(-60) });
        let plate = distance("plate", 1.0, Tolerance::Symmetric { value: 2.0_f64.powi(-61) });
        let links = [
            StackLink { item: &opening, direction: StackDirection::Forward, nominal_mm: Some(1.0) },
            StackLink { item: &plate, direction: StackDirection::Reverse, nominal_mm: Some(1.0) },
        ];
        let limit = 3.0 * 2.0_f64.powi(-61);
        assert_eq!(signed_worst_case(&links).unwrap().range, range(0.0, -limit, limit));
    }
}
