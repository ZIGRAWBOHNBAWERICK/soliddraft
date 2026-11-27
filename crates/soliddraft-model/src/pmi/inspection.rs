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
    /// Document name; not a manufacturing part number.
    pub part: String,
    /// Caller-supplied manufacturing revision, if known.
    pub revision: Option<String>,
    /// Material stated in the part notes.
    pub material: Option<String>,
    /// General tolerance designation stated in the part notes.
    pub general_tolerance: Option<String>,
    /// Inspection groups in document order.
    pub groups: Vec<InspectionGroup>,
}

/// One native PMI item or part note, with its child characteristics in drawing order.
#[derive(Debug, PartialEq)]
pub struct InspectionGroup {
    /// Stable part-local group identifier.
    pub id: String,
    /// Source PMI item identifier; absent for a part note.
    pub item_id: Option<String>,
    /// Human-readable requirement from the source PMI.
    pub callout: String,
    /// Datum references in precedence order.
    pub datums: Vec<DatumRef>,
    /// Why the source PMI could not be resolved.
    pub problem: Option<String>,
    /// Characteristics belonging to this group, in requirement order.
    pub rows: Vec<InspectionRow>,
    face_groups: Vec<Feature<FaceRef>>,
    datum_label: Option<String>,
    roughness_average: bool,
    thread_feature: Option<FeatureId>,
}

/// Design requirement represented by one characteristic row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectionKind {
    /// Diameter of a cylinder, including a hole pilot.
    Diameter,
    /// Radius of a circular feature.
    Radius,
    /// Axial length from entry to the end of the full drill diameter.
    DrillDepth,
    /// Diameter of the cylindrical head enlargement.
    CounterboreDiameter,
    /// Axial distance from entry to the counterbore shoulder.
    CounterboreDepth,
    /// Major diameter at the countersink entry.
    CountersinkDiameter,
    /// Full included angle of the countersink cone.
    CountersinkAngle,
    /// Specified usable thread depth from the entry.
    ThreadDepth,
    /// Distance between two features or from a locating datum.
    Distance,
    /// Length of a slot between its ends.
    SlotLength,
    /// Width between the slot flats.
    SlotWidth,
    /// Radius of the slot ends.
    SlotRadius,
    /// A geometric control.
    Geometric {
        /// Characteristic named by the feature control frame.
        characteristic: GdtCharacteristic,
        /// Whether the tolerance zone is specified by diameter.
        diameter: bool,
        /// Material condition modifier on the tolerance zone.
        material: MaterialCondition,
    },
    /// Surface texture requirement.
    SurfaceFinish,
    /// Thread designation, not a measured cylinder diameter.
    Thread,
    /// Datum definition, not a measurement.
    Datum,
    /// Part note, not a measurement.
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

/// Unit of the nominal and numerical limits on a characteristic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectionUnit {
    /// Linear dimensions.
    Millimetre,
    /// Surface roughness.
    Micrometre,
    /// Angular dimensions in degrees.
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

/// Whether a design requirement can contribute an inspection characteristic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applicability {
    /// Inspection requirement, with no as-built result implied.
    Inspect,
    /// Reference information such as a basic dimension or note.
    Reference,
    /// A missing or unresolved design input prevents inspection.
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
    /// Stable part-local characteristic identifier.
    pub id: String,
    /// One-based position in the PMI pattern (or placement of the native thread feature).
    pub member: Option<usize>,
    /// What the requirement controls.
    pub kind: InspectionKind,
    /// Units for the numeric values, if relevant.
    pub unit: Option<InspectionUnit>,
    /// Design nominal, if specified or available from the checked model.
    pub nominal: Option<f64>,
    /// Absolute lower bound, when stated numerically.
    pub lower_limit: Option<f64>,
    /// Absolute upper bound, when stated numerically.
    pub upper_limit: Option<f64>,
    /// Source dimensional tolerance, if applicable.
    pub dimension_tolerance: Option<Tolerance>,
    /// Index into the group's ordered datum frame, only for a derived basic distance.
    pub locating_datum: Option<usize>,
    face_groups: Range<usize>,
}

impl InspectionRow {
    /// Classify the requirement without treating a CAD nominal as an inspection result.
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
            datum_label: None,
            roughness_average: false,
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
            PmiKind::Datum { letter, feature } => {
                group.datum_label = Some(letter);
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
                if let Some((parameter, limit)) = roughness(&text) {
                    row.unit = Some(InspectionUnit::Micrometre);
                    row.upper_limit = Some(limit.value());
                    group.roughness_average = parameter == "Ra";
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

    /// Publish inspection requirements as QIF 3.0, without generating measurement results.
    /// The returned omissions identify rows that this profile could not carry.
    pub fn write_qif(
        &self,
        out: impl Write,
        metadata: soliddraft_exchange::qif::Metadata<'_>,
    ) -> io::Result<Vec<soliddraft_exchange::qif::Omission>> {
        use soliddraft_exchange::qif::{self, Dimension, Requirement};

        let mut rows = Vec::new();
        for group in &self.groups {
            for row in &group.rows {
                let dimension = |kind| match (row.nominal, row.unit, row.dimension_tolerance) {
                    (Some(target), Some(InspectionUnit::Millimetre), Some(tolerance)) => {
                        Requirement::Dimension { kind, target, tolerance, limits: row.lower_limit.zip(row.upper_limit) }
                    }
                    _ => Requirement::Omit("missing nominal or dimensional tolerance"),
                };
                let requirement = if group.problem.is_some() {
                    Requirement::Omit("unresolved source PMI")
                } else {
                    match row.kind {
                        InspectionKind::Datum => Requirement::Datum,
                        InspectionKind::Diameter
                        | InspectionKind::CounterboreDiameter
                        | InspectionKind::CountersinkDiameter => dimension(Dimension::Diameter),
                        InspectionKind::Radius | InspectionKind::SlotRadius => dimension(Dimension::Radius),
                        InspectionKind::Distance
                        | InspectionKind::SlotLength
                        | InspectionKind::DrillDepth
                        | InspectionKind::CounterboreDepth
                        | InspectionKind::ThreadDepth => dimension(Dimension::Length),
                        InspectionKind::SlotWidth => dimension(Dimension::Width),
                        InspectionKind::Geometric { characteristic, diameter, material } => {
                            match (row.unit, row.upper_limit) {
                                (Some(InspectionUnit::Millimetre), Some(limit_mm)) => Requirement::Geometric {
                                    kind: characteristic,
                                    limit_mm,
                                    diametrical: diameter,
                                    material,
                                },
                                _ => Requirement::Omit("missing geometric tolerance"),
                            }
                        }
                        InspectionKind::SurfaceFinish => match (row.unit, row.upper_limit) {
                            (Some(InspectionUnit::Micrometre), Some(limit_um)) if group.roughness_average => {
                                Requirement::RoughnessAverage { limit_um }
                            }
                            _ => Requirement::Omit("surface finish has no numeric Ra limit"),
                        },
                        InspectionKind::Thread => Requirement::Omit("thread has no QIF mapping in this profile"),
                        InspectionKind::CountersinkAngle => {
                            Requirement::Omit("angle has no QIF mapping in this profile")
                        }
                        InspectionKind::Note => Requirement::Omit("note is not an inspection requirement"),
                    }
                };
                rows.push(qif::Row {
                    id: &row.id,
                    callout: &group.callout,
                    features: row
                        .faces(group)
                        .iter()
                        .filter(|faces| !faces.is_empty())
                        .map(json)
                        .collect::<io::Result<_>>()?,
                    datum_label: group.datum_label.as_deref(),
                    frame: &group.datums,
                    locating_datum: row.locating_datum.map(|index| group.datums[index].letter.as_str()),
                    requirement,
                });
            }
        }
        qif::write(
            out,
            qif::Part {
                name: &self.part,
                revision: self.revision.as_deref(),
                material: self.material.as_deref(),
                general_tolerance: self.general_tolerance.as_deref(),
            },
            metadata,
            &rows,
        )
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
