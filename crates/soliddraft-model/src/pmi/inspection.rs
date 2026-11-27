//! Design requirements for inspection. CAD measurements supply nominals, never as-built results.
//!
//! IDs are part-local. A stored item's ID, its one-based feature order, and the check's role
//! identify a characteristic. A located basic dimension also names its datum. A generated thread
//! uses its native hole feature ID and one-based placement, independent of callout regrouping.
//! Renaming a stored PMI item or reordering its features changes that identity; native reopen does not.

use std::fmt::Display;
use std::io::{self, Write};
use std::ops::Range;

use serde::Serialize;
use soliddraft_kernel_api::pmi::{
    DatumRef, Feature, GdtCharacteristic, HoleHeadKind, HoleHeadNominals, MaterialCondition, PmiKind, SizeKind,
    Tolerance, roughness,
};

use crate::pmi::PmiRow;
use crate::{Document, FaceRef, FeatureId};

/// Inspection requirements for one part. The manufacturing revision is supplied by the caller,
/// not inferred from a file name or the CAD document's content hash.
#[derive(Debug, PartialEq)]
pub struct InspectionPlan {
    pub part: String,
    pub revision: Option<String>,
    pub material: Option<String>,
    pub general_tolerance: Option<String>,
    pub groups: Vec<InspectionGroup>,
}

/// One native PMI item or part note, with its child characteristics in drawing order.
#[derive(Debug, PartialEq)]
pub struct InspectionGroup {
    pub id: String,
    pub item_id: Option<String>,
    pub callout: String,
    pub datums: Vec<DatumRef>,
    pub problem: Option<String>,
    pub rows: Vec<InspectionRow>,
    face_groups: Vec<Feature<FaceRef>>,
    thread_feature: Option<FeatureId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectionKind {
    Diameter,
    Radius,
    DrillDepth,
    CounterboreDiameter,
    CounterboreDepth,
    CountersinkDiameter,
    CountersinkAngle,
    ThreadDepth,
    Distance,
    SlotLength,
    SlotWidth,
    SlotRadius,
    Geometric { characteristic: GdtCharacteristic, diameter: bool, material: MaterialCondition },
    SurfaceFinish,
    Thread,
    Datum,
    Note,
}

impl InspectionKind {
    fn key(self) -> &'static str {
        match self {
            Self::Diameter => "diameter",
            Self::DrillDepth => "drill_depth",
            Self::CounterboreDiameter => "counterbore_diameter",
            Self::CounterboreDepth => "counterbore_depth",
            Self::CountersinkDiameter => "countersink_diameter",
            Self::CountersinkAngle => "countersink_angle",
            Self::ThreadDepth => "thread_depth",
            Self::Radius => "radius",
            Self::Distance => "distance",
            Self::SlotLength => "slot_length",
            Self::SlotWidth => "slot_width",
            Self::SlotRadius => "slot_radius",
            Self::Geometric { characteristic, .. } => characteristic.key(),
            Self::SurfaceFinish => "surface_finish",
            Self::Thread => "thread",
            Self::Datum => "datum",
            Self::Note => "note",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectionUnit {
    Millimetre,
    Micrometre,
    Degree,
}

impl InspectionUnit {
    fn symbol(self) -> &'static str {
        match self {
            Self::Millimetre => "mm",
            Self::Micrometre => "µm",
            Self::Degree => "°",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applicability {
    Inspect,
    Reference,
    Unresolved,
}

impl Applicability {
    fn key(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Reference => "reference",
            Self::Unresolved => "unresolved",
        }
    }
}

/// One design requirement. Limits are absolute bounds, not deviations or CAD comparisons.
/// A fit class or general tolerance is retained without guessing a numerical limit. GD&T and
/// roughness carry their stated upper limit only; a material modifier may affect the GD&T limit
/// when the actual feature size is known.
#[derive(Debug, PartialEq)]
pub struct InspectionRow {
    pub id: String,
    /// One-based position in the PMI pattern (or placement of the native thread feature).
    pub member: Option<usize>,
    pub kind: InspectionKind,
    pub unit: Option<InspectionUnit>,
    pub nominal: Option<f64>,
    pub lower_limit: Option<f64>,
    pub upper_limit: Option<f64>,
    pub dimension_tolerance: Option<Tolerance>,
    /// Index into the group's ordered datum frame, only for a derived basic distance.
    pub locating_datum: Option<usize>,
    face_groups: Range<usize>,
}

impl InspectionRow {
    pub fn applicability(&self, group: &InspectionGroup) -> Applicability {
        if matches!(self.kind, InspectionKind::Datum | InspectionKind::Note)
            || self.dimension_tolerance == Some(Tolerance::Basic)
        {
            Applicability::Reference
        } else if group.problem.is_some() || (self.dimension_tolerance.is_some() && self.nominal.is_none()) {
            Applicability::Unresolved
        } else {
            Applicability::Inspect
        }
    }

    /// The controlled or dimensioned faces of this row, grouped as in the source PMI.
    /// A locating distance refers to its datum's face separately, by ordered datum reference.
    pub fn faces<'a>(&self, group: &'a InspectionGroup) -> &'a [Feature<FaceRef>] {
        &group.face_groups[self.face_groups.clone()]
    }

    fn dimension(&mut self, nominal: Option<f64>, tolerance: Tolerance) {
        self.unit = Some(InspectionUnit::Millimetre);
        self.nominal = nominal.filter(|value| value.is_finite());
        self.dimension_tolerance = Some(tolerance);
        if let Some((nominal, (upper, lower))) = self.nominal.zip(tolerance.deviations())
            && tolerance.problem().is_none()
        {
            self.lower_limit = Some(nominal + lower).filter(|value| value.is_finite());
            self.upper_limit = Some(nominal + upper).filter(|value| value.is_finite());
        }
    }
}

impl InspectionGroup {
    fn new(id: String, item_id: Option<String>, callout: String) -> Self {
        Self {
            id,
            item_id,
            callout,
            datums: Vec::new(),
            problem: None,
            rows: Vec::new(),
            face_groups: Vec::new(),
            thread_feature: None,
        }
    }

    fn add(
        &mut self,
        kind: InspectionKind,
        member: Option<usize>,
        face_groups: Range<usize>,
        locating_datum: Option<usize>,
    ) -> &mut InspectionRow {
        let role = kind.key();
        let group = &self.id;
        let id = match (self.thread_feature, member, locating_datum) {
            (Some(feature), Some(member), _) => format!("thread/{}/{member}", feature.0),
            (_, Some(member), Some(datum)) => format!("{group}/{member}/basic/{}", self.datums[datum].letter),
            (_, Some(member), None) => format!("{group}/{member}/{role}"),
            _ => format!("{group}/{role}"),
        };
        self.rows.push(InspectionRow {
            id,
            member,
            kind,
            unit: None,
            nominal: None,
            lower_limit: None,
            upper_limit: None,
            dimension_tolerance: None,
            locating_datum,
            face_groups,
        });
        self.rows.last_mut().unwrap()
    }

    fn from_pmi(source: PmiRow) -> Self {
        let callout = source.callout();
        let PmiRow { item, measured, hole, located, problem, feature, placements, .. } = source;
        let id = match (feature, placements.first()) {
            (Some(feature), Some(first)) => format!("thread/{}/group/{}", feature.0, first + 1),
            _ => format!("pmi/{}", item.id),
        };
        let mut group = Self::new(id, Some(item.id), callout);
        group.problem = problem;
        group.thread_feature = feature;
        match item.kind {
            PmiKind::Datum { feature, .. } => {
                group.face_groups.push(feature);
                group.add(InspectionKind::Datum, None, 0..1, None);
            }
            PmiKind::Size { size, features, tolerance, thread, .. } => {
                group.face_groups = features;
                let kind = match (size, thread.is_some()) {
                    (_, true) => InspectionKind::Thread,
                    (SizeKind::Diameter, false) => InspectionKind::Diameter,
                    (SizeKind::Radius, false) => InspectionKind::Radius,
                };
                for index in 0..group.face_groups.len() {
                    let member = placements.get(index).copied().unwrap_or(index) + 1;
                    let row = group.add(kind, Some(member), index..index + 1, None);
                    // The CAD cylinder's diameter is not the thread's inspection requirement.
                    if kind != InspectionKind::Thread {
                        row.dimension(measured.first().copied(), tolerance);
                    }
                }
            }
            PmiKind::Hole { pilot, head, tolerance, through, drill_depth, thread } => {
                group.face_groups.push(pilot);
                group
                    .add(InspectionKind::Diameter, Some(1), 0..1, None)
                    .dimension(hole.map(|nominal| nominal.pilot_diameter_mm), tolerance);
                if !through {
                    group.add(InspectionKind::DrillDepth, Some(1), 0..1, None).dimension(
                        drill_depth
                            .map(|depth| depth.value())
                            .or_else(|| hole.and_then(|nominal| nominal.drill_depth_mm)),
                        Tolerance::None,
                    );
                }
                if let Some(head) = head {
                    group.face_groups.push(head.faces);
                    let nominal = hole.and_then(|nominal| nominal.head);
                    let (diameter_kind, extent_kind, diameter, extent, unit) = match (head.kind, nominal) {
                        (HoleHeadKind::Counterbore, nominal) => {
                            let (diameter, depth) = match nominal {
                                Some(HoleHeadNominals::Counterbore { diameter_mm, depth_mm }) => {
                                    (Some(diameter_mm), Some(depth_mm))
                                }
                                _ => (None, None),
                            };
                            (
                                InspectionKind::CounterboreDiameter,
                                InspectionKind::CounterboreDepth,
                                diameter,
                                depth,
                                InspectionUnit::Millimetre,
                            )
                        }
                        (HoleHeadKind::Countersink, nominal) => {
                            let (diameter, angle) = match nominal {
                                Some(HoleHeadNominals::Countersink { diameter_mm, included_angle_deg }) => {
                                    (Some(diameter_mm), Some(included_angle_deg))
                                }
                                _ => (None, None),
                            };
                            (
                                InspectionKind::CountersinkDiameter,
                                InspectionKind::CountersinkAngle,
                                diameter,
                                angle,
                                InspectionUnit::Degree,
                            )
                        }
                    };
                    group.add(diameter_kind, Some(1), 1..2, None).dimension(diameter, Tolerance::None);
                    let row = group.add(extent_kind, Some(1), 1..2, None);
                    row.dimension(extent, Tolerance::None);
                    row.unit = Some(unit);
                }
                if let Some(thread) = thread {
                    group.add(InspectionKind::Thread, Some(1), 0..1, None);
                    if let Some(depth) = thread.depth {
                        group
                            .add(InspectionKind::ThreadDepth, Some(1), 0..1, None)
                            .dimension(Some(depth.value()), Tolerance::None);
                    }
                }
            }
            PmiKind::Location { from, to, tolerance, .. } => {
                group.face_groups = vec![from.feature, to.feature];
                group.add(InspectionKind::Distance, None, 0..2, None).dimension(measured.first().copied(), tolerance);
            }
            PmiKind::Slot { flats: [a, b], ends: [c, d], tolerance } => {
                group.face_groups = vec![a, b, c, d];
                for (index, (kind, faces, tolerance)) in [
                    (InspectionKind::SlotLength, 2..4, tolerance),
                    (InspectionKind::SlotWidth, 0..2, tolerance),
                    (InspectionKind::SlotRadius, 2..4, Tolerance::None),
                ]
                .into_iter()
                .enumerate()
                {
                    group.add(kind, None, faces, None).dimension(measured.get(index).copied(), tolerance);
                }
            }
            PmiKind::Gdt { characteristic, tolerance, diameter, material, datums, features } => {
                group.face_groups = features;
                group.datums = datums;
                let kind = InspectionKind::Geometric { characteristic, diameter, material };
                let mut located = located.into_iter().peekable();
                for index in 0..group.face_groups.len() {
                    let row = group.add(kind, Some(index + 1), index..index + 1, None);
                    row.unit = Some(InspectionUnit::Millimetre);
                    row.upper_limit = Some(tolerance);
                    // pmi_rows emits these in feature order, then in datum precedence order.
                    while located.peek().is_some_and(|distance| distance.feature == index) {
                        let distance = located.next().unwrap();
                        if let Some(datum) = group.datums.iter().position(|datum| datum.letter == distance.datum) {
                            group
                                .add(InspectionKind::Distance, Some(index + 1), index..index + 1, Some(datum))
                                .dimension(Some(distance.value), Tolerance::Basic);
                        }
                    }
                }
            }
            PmiKind::SurfaceFinish { feature, text, .. } => {
                group.face_groups.push(feature);
                let row = group.add(InspectionKind::SurfaceFinish, None, 0..1, None);
                if let Some((_, limit)) = roughness(&text) {
                    row.unit = Some(InspectionUnit::Micrometre);
                    row.upper_limit = Some(limit.value());
                }
            }
        }
        group
    }
}

impl InspectionPlan {
    /// Use the document's checked PMI mapping, retaining unresolved items with their reasons.
    pub fn from_document(doc: &Document, revision: Option<&str>) -> Self {
        let rows = doc.pmi_rows();
        let notes = doc.part_notes();
        let mut groups = Vec::with_capacity(rows.len() + notes.notes.len());
        groups.extend(rows.into_iter().map(InspectionGroup::from_pmi));
        for (index, note) in notes.notes.into_iter().enumerate() {
            let mut group = InspectionGroup::new(format!("note/{}", index + 1), None, note);
            group.add(InspectionKind::Note, None, 0..0, None);
            groups.push(group);
        }
        Self {
            part: doc.name().to_owned(),
            revision: revision.map(str::to_owned),
            material: notes.material,
            general_tolerance: notes.general_tolerance,
            groups,
        }
    }

    /// Write UTF-8 RFC 4180 CSV. Datum order and persistent face groups are JSON arrays in
    /// their cells. Text is preserved verbatim; actual and disposition are always empty.
    pub fn write_csv(&self, mut out: impl Write) -> io::Result<()> {
        out.write_all(concat!(
            "part,revision,material,general_tolerance,group_id,characteristic_id,pmi_item,member,kind,applicability,",
            "pmi_callout,nominal,unit,tolerance,fit_class,lower_limit,upper_limit,datum_order,locating_datum,",
            "face_refs,problem,actual,disposition\r\n",
        ).as_bytes())?;
        for group in &self.groups {
            let datums = json(&group.datums)?;
            for row in &group.rows {
                for text in [
                    &self.part,
                    self.revision.as_deref().unwrap_or_default(),
                    self.material.as_deref().unwrap_or_default(),
                    self.general_tolerance.as_deref().unwrap_or_default(),
                    &group.id,
                    &row.id,
                    group.item_id.as_deref().unwrap_or_default(),
                ] {
                    csv_field(&mut out, text)?;
                }
                csv_number(&mut out, row.member)?;
                csv_field(&mut out, row.kind.key())?;
                csv_field(&mut out, row.applicability(group).key())?;
                csv_field(&mut out, &group.callout)?;
                csv_number(&mut out, row.nominal)?;
                csv_field(&mut out, row.unit.map_or("", InspectionUnit::symbol))?;
                let (tolerance, fit) = match row.dimension_tolerance {
                    None => ("", None),
                    Some(Tolerance::None) => ("general", None),
                    Some(Tolerance::Symmetric { .. }) => ("symmetric", None),
                    Some(Tolerance::Deviation { .. }) => ("deviation", None),
                    Some(Tolerance::Limits { .. }) => ("limits", None),
                    Some(Tolerance::Fit { class }) => ("iso_286_fit", Some(class)),
                    Some(Tolerance::Basic) => ("basic", None),
                };
                csv_field(&mut out, tolerance)?;
                csv_number(&mut out, fit)?;
                csv_number(&mut out, row.lower_limit)?;
                csv_number(&mut out, row.upper_limit)?;
                csv_field(&mut out, &datums)?;
                csv_field(&mut out, row.locating_datum.map_or("", |index| &group.datums[index].letter))?;
                csv_field(&mut out, &json(row.faces(group))?)?;
                csv_field(&mut out, group.problem.as_deref().unwrap_or_default())?;
                out.write_all(b",\r\n")?;
            }
        }
        Ok(())
    }
}

fn json(value: &(impl Serialize + ?Sized)) -> io::Result<String> {
    serde_json::to_string(value).map_err(io::Error::other)
}

fn csv_number(out: &mut impl Write, value: Option<impl Display>) -> io::Result<()> {
    if let Some(value) = value {
        write!(out, "{value}")?;
    }
    out.write_all(b",")
}

fn csv_field(out: &mut impl Write, value: &str) -> io::Result<()> {
    if value.bytes().any(|byte| matches!(byte, b',' | b'"' | b'\r' | b'\n')) {
        out.write_all(b"\"")?;
        let mut pieces = value.split('"');
        out.write_all(pieces.next().unwrap().as_bytes())?;
        for piece in pieces {
            out.write_all(b"\"\"")?;
            out.write_all(piece.as_bytes())?;
        }
        out.write_all(b"\"")?;
    } else {
        out.write_all(value.as_bytes())?;
    }
    out.write_all(b",")
}

#[cfg(test)]
mod tests;
