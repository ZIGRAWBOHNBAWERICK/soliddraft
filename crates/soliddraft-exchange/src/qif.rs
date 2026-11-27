//! QIF 3.0 inspection requirements, without CAD geometry, measurement results or a machine program.
//!
//! QIF IDs are document-local. The `SolidDraft.CharacteristicId` and `SolidDraft.FaceRefs`
//! attributes preserve the CSV characteristic ID and the persistent native face references.

use std::collections::{HashMap, HashSet};
use std::fmt::{self, Display, Write};
use std::io;

use soliddraft_kernel_api::pmi::{DatumRef, GdtCharacteristic, MaterialCondition, Tolerance};

/// QIF requires a document UUID and a formal inspection standard. Neither is inferred from PMI.
pub struct Metadata<'a> {
    /// UUID identifying this QIF document.
    pub qpid: &'a str,
    /// Organization that publishes the inspection standard.
    pub organization: &'a str,
    /// Standard designation without the organization or publication year.
    pub designator: &'a str,
    /// Publication year, when known.
    pub year: Option<&'a str>,
}

/// Identification and document-wide requirements for the inspected part.
pub struct Part<'a> {
    /// Part name or number.
    pub name: &'a str,
    /// Manufacturing revision, when known.
    pub revision: Option<&'a str>,
    /// Material as specified by the designer.
    pub material: Option<&'a str>,
    /// General tolerance as specified by the designer.
    pub general_tolerance: Option<&'a str>,
}

/// Inspection types supported by the requirements-only export profile.
pub enum Requirement {
    /// A datum definition.
    Datum,
    /// A linear dimension in millimetres.
    Dimension {
        /// Measurement type.
        kind: Dimension,
        /// Nominal value.
        target: f64,
        /// Source tolerance classification.
        tolerance: Tolerance,
        /// Absolute lower and upper bounds, when stated numerically.
        limits: Option<(f64, f64)>,
    },
    /// A geometric tolerance.
    Geometric {
        /// Geometric characteristic from the source frame.
        kind: GdtCharacteristic,
        /// Stated tolerance value in millimetres.
        limit_mm: f64,
        /// Whether the tolerance zone is specified by diameter.
        diametrical: bool,
        /// The controlled feature's material condition.
        material: MaterialCondition,
    },
    /// An Ra requirement.
    RoughnessAverage {
        /// Maximum Ra in micrometres.
        limit_um: f64,
    },
    /// Retain this ID in the omission report with the reason.
    Omit(&'static str),
}

/// Linear dimensional characteristics supported by this profile.
#[derive(Clone, Copy)]
pub enum Dimension {
    /// Diameter.
    Diameter,
    /// Radius.
    Radius,
    /// Length or separation.
    Length,
    /// Width.
    Width,
}

/// One CSV row. Each feature key is a JSON array of the grouped persistent face references.
pub struct Row<'a> {
    /// Stable characteristic ID from the inspection plan.
    pub id: &'a str,
    /// Original PMI callout, for display.
    pub callout: &'a str,
    /// Controlled features in source order.
    pub features: Vec<String>,
    /// Label for a datum definition.
    pub datum_label: Option<&'a str>,
    /// Datum references in precedence order.
    pub frame: &'a [DatumRef],
    /// Datum from which a basic distance locates its feature.
    pub locating_datum: Option<&'a str>,
    /// Values and mapping for this characteristic.
    pub requirement: Requirement,
}

/// A source row excluded from the exported profile.
#[derive(Debug)]
pub struct Omission {
    /// Stable characteristic ID from the inspection plan.
    pub characteristic_id: String,
    /// Why the row could not be written.
    pub reason: &'static str,
}

struct Feature<'a> {
    definition: usize,
    nominal: usize,
    item: usize,
    faces: &'a str,
}

struct Datum<'a> {
    id: usize,
    row: &'a Row<'a>,
    features: Vec<usize>,
}

struct Characteristic<'a> {
    definition: usize,
    nominal: usize,
    item: Option<usize>,
    tag: &'static str,
    row: &'a Row<'a>,
    features: Vec<usize>,
    frame: Option<usize>,
}

/// Write a QIF document and return rows the profile could not represent. Reference dimensions
/// have nominals but no inspection items. Features carry face identity, not inferred axes or geometry.
pub fn write<'a>(
    mut out: impl io::Write,
    part: Part<'_>,
    meta: Metadata<'_>,
    rows: &'a [Row<'a>],
) -> io::Result<Vec<Omission>> {
    let invalid = |msg| io::Error::new(io::ErrorKind::InvalidInput, msg);
    if meta.qpid.len() != 36
        || !meta
            .qpid
            .bytes()
            .enumerate()
            .all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == b'-' } else { c.is_ascii_hexdigit() })
    {
        return Err(invalid("QIF QPId must be a UUID"));
    }
    if part.name.trim().is_empty() || meta.organization.trim().is_empty() || meta.designator.trim().is_empty() {
        return Err(invalid("QIF requires a part name and a formal standard"));
    }
    let mut seen = HashSet::new();
    if rows.iter().any(|row| row.id.is_empty() || !seen.insert(row.id)) {
        return Err(invalid("QIF source characteristic IDs must be nonempty and unique"));
    }

    // QIF IDs are unique across record types. Reserve part and formal standard IDs first.
    let mut last_id = 2;
    let mut id = || {
        last_id += 1;
        last_id
    };
    let mut features = Vec::new();
    let mut feature_indices = HashMap::new();
    let mut feature = |faces: &'a str, id: &mut dyn FnMut() -> usize| {
        *feature_indices.entry(faces).or_insert_with(|| {
            let index = features.len();
            features.push(Feature { definition: id(), nominal: id(), item: id(), faces });
            index
        })
    };
    let mut datums = Vec::new();
    let mut datum_indices = HashMap::new();
    let mut omissions = Vec::new();
    for row in rows.iter().filter(|row| matches!(row.requirement, Requirement::Datum)) {
        match row.datum_label {
            Some(label)
                if !row.features.is_empty() && label.bytes().all(|c| c.is_ascii_uppercase()) && !label.is_empty() =>
            {
                if datum_indices.insert(label, datums.len()).is_some() {
                    return Err(invalid("duplicate QIF datum label"));
                }
                datums.push(Datum {
                    id: id(),
                    row,
                    features: row.features.iter().map(|f| feature(f, &mut id)).collect(),
                });
            }
            _ => omissions
                .push(Omission { characteristic_id: row.id.to_owned(), reason: "unresolved or unsupported datum" }),
        }
    }
    let mut characteristics = Vec::new();
    let mut frames = Vec::new();
    let mut frame_ids = HashMap::new();
    for row in rows.iter().filter(|row| !matches!(row.requirement, Requirement::Datum)) {
        let tag = match characteristic_tag(row) {
            Ok(tag) => tag,
            Err(reason) => {
                omissions.push(Omission { characteristic_id: row.id.to_owned(), reason });
                continue;
            }
        };
        if row.frame.len() > 3
            || row.frame.iter().any(|d| !datum_indices.contains_key(d.letter.as_str()))
            || row.locating_datum.is_some_and(|label| !datum_indices.contains_key(label))
        {
            omissions.push(Omission {
                characteristic_id: row.id.to_owned(),
                reason: "unresolved or unsupported datum reference",
            });
            continue;
        }
        let mut refs: Vec<_> = row.features.iter().map(|f| feature(f, &mut id)).collect();
        if let Some(label) = row.locating_datum {
            for &index in &datums[datum_indices[label]].features {
                if !refs.contains(&index) {
                    refs.push(index);
                }
            }
        }
        let frame = (!row.frame.is_empty() && matches!(row.requirement, Requirement::Geometric { .. })).then(|| {
            // Many rows in a pattern use the same datum frame.
            let key: Vec<_> =
                row.frame.iter().map(|d| (datum_indices[d.letter.as_str()], modifier(d.material))).collect();
            *frame_ids.entry(key.clone()).or_insert_with(|| {
                let frame_id = id();
                frames.push((frame_id, key));
                frame_id
            })
        });
        let reference = matches!(row.requirement, Requirement::Dimension { tolerance: Tolerance::Basic, .. });
        characteristics.push(Characteristic {
            definition: id(),
            nominal: id(),
            item: (!reference).then(&mut id),
            tag,
            row,
            features: refs,
            frame,
        });
    }

    let mut xml = String::new();
    writeln!(xml, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<QIFDocument xmlns=\"http://qifstandards.org/xsd/qif3\" versionQIF=\"3.0.0\" idMax=\"{last_id}\">").unwrap();
    text(&mut xml, "QPId", meta.qpid)?;
    attributes(
        &mut xml,
        &[("SolidDraft.Material", part.material), ("SolidDraft.GeneralTolerance", part.general_tolerance)],
    )?;
    if !characteristics.is_empty() {
        xml.push_str("<StandardsDefinitions n=\"1\"><Standard id=\"2\"><Organization>");
        text(
            &mut xml,
            match meta.organization {
                "ISO" | "ASME" | "ANSI" => "StandardsOrganizationEnum",
                _ => "OtherStandardsOrganization",
            },
            meta.organization,
        )?;
        xml.push_str("</Organization>");
        text(&mut xml, "Designator", meta.designator)?;
        if let Some(year) = meta.year {
            text(&mut xml, "Year", year)?;
        }
        xml.push_str("</Standard></StandardsDefinitions>\n");
    }
    xml.push_str(concat!(
        "<FileUnits><PrimaryUnits><LinearUnit><SIUnitName>meter</SIUnitName><UnitName>mm</UnitName>",
        "<UnitConversion><Factor>0.001</Factor></UnitConversion></LinearUnit></PrimaryUnits>",
        "<OtherUnits n=\"1\"><LinearUnit><SIUnitName>meter</SIUnitName><UnitName>µm</UnitName>",
        "<UnitConversion><Factor>0.000001</Factor></UnitConversion></LinearUnit></OtherUnits></FileUnits>\n",
    ));
    if !datums.is_empty() {
        list(&mut xml, "DatumDefinitions", datums.len());
        for datum in &datums {
            write!(xml, "<DatumDefinition id=\"{}\">", datum.id).unwrap();
            attributes(&mut xml, &[("SolidDraft.CharacteristicId", Some(datum.row.id))])?;
            text(&mut xml, "DatumLabel", datum.row.datum_label.unwrap())?;
            references(&mut xml, "FeatureNominalIds", datum.features.iter().map(|&index| features[index].nominal));
            xml.push_str("</DatumDefinition>\n");
        }
        xml.push_str("</DatumDefinitions>\n");
    }
    if !frames.is_empty() {
        list(&mut xml, "DatumReferenceFrames", frames.len());
        for (frame_id, frame) in &frames {
            write!(xml, "<DatumReferenceFrame id=\"{frame_id}\">").unwrap();
            list(&mut xml, "Datums", frame.len());
            for ((index, modifier), precedence) in frame.iter().zip(["PRIMARY", "SECONDARY", "TERTIARY"]) {
                write!(xml, concat!("<Datum><SimpleDatum><DatumDefinitionId>{}</DatumDefinitionId>",
                    "<MaterialModifier>{}</MaterialModifier><ReferencedComponent>ACTUAL</ReferencedComponent></SimpleDatum>",
                    "<Precedence><PrecedenceEnum>{}</PrecedenceEnum></Precedence></Datum>"), datums[*index].id, modifier, precedence).unwrap();
            }
            xml.push_str("</Datums></DatumReferenceFrame>\n");
        }
        xml.push_str("</DatumReferenceFrames>\n");
    }
    xml.push_str("<Product><PartSet n=\"1\"><Part id=\"1\">");
    text(&mut xml, "ModelNumber", part.name)?;
    if let Some(revision) = part.revision {
        text(&mut xml, "Version", revision)?;
    }
    references(&mut xml, "FeatureNominalIds", features.iter().map(|f| f.nominal));
    references(&mut xml, "CharacteristicNominalIds", characteristics.iter().map(|c| c.nominal));
    references(&mut xml, "DatumDefinitionIds", datums.iter().map(|d| d.id));
    references(&mut xml, "DatumReferenceFrameIds", frames.iter().map(|(id, _)| *id));
    xml.push_str("</Part></PartSet><RootPart><Id>1</Id></RootPart></Product>\n");
    if !features.is_empty() {
        write_features(&mut xml, &features)?;
    }
    if !characteristics.is_empty() {
        write_characteristics(&mut xml, &characteristics, &features)?;
    }
    xml.push_str("</QIFDocument>\n");
    out.write_all(xml.as_bytes())?;
    Ok(omissions)
}

fn characteristic_tag(row: &Row<'_>) -> Result<&'static str, &'static str> {
    if let Requirement::Omit(reason) = row.requirement {
        return Err(reason);
    }
    if row.features.is_empty() {
        return Err("no associated feature");
    }
    match row.requirement {
        Requirement::Dimension { kind, target, tolerance, limits }
            if target.is_finite()
                && limits.is_none_or(|(lo, hi)| lo.is_finite() && hi.is_finite() && lo <= hi)
                && (matches!(tolerance, Tolerance::Basic | Tolerance::None | Tolerance::Fit { .. })
                    || limits.is_some()) =>
        {
            match kind {
                Dimension::Diameter => Ok("Diameter"),
                _ if matches!(tolerance, Tolerance::Fit { .. }) => {
                    Err("a non-diameter fit has no mapping in this profile")
                }
                Dimension::Radius => Ok("Radius"),
                Dimension::Length => Ok("Length"),
                Dimension::Width => Ok("Width"),
            }
        }
        Requirement::Geometric { kind, limit_mm, diametrical, .. } if limit_mm.is_finite() && limit_mm >= 0.0 => {
            match kind {
                GdtCharacteristic::Position => Ok("Position"),
                GdtCharacteristic::Flatness if !diametrical && row.frame.is_empty() => Ok("Flatness"),
                GdtCharacteristic::Perpendicularity if !row.frame.is_empty() => Ok("Perpendicularity"),
                _ => Err("unsupported geometric tolerance"),
            }
        }
        Requirement::RoughnessAverage { limit_um } if limit_um.is_finite() && limit_um > 0.0 => Ok("SurfaceTexture"),
        _ => Err("missing or invalid requirement value"),
    }
}

fn write_features(xml: &mut String, features: &[Feature<'_>]) -> io::Result<()> {
    xml.push_str("<Features>");
    list(xml, "FeatureDefinitions", features.len());
    for f in features {
        writeln!(xml, "<OtherShapeFeatureDefinition id=\"{}\"><Description>SolidDraft PMI face group</Description></OtherShapeFeatureDefinition>", f.definition).unwrap();
    }
    xml.push_str("</FeatureDefinitions>");
    list(xml, "FeatureNominals", features.len());
    for f in features {
        write!(xml, "<OtherShapeFeatureNominal id=\"{}\">", f.nominal).unwrap();
        attributes(xml, &[("SolidDraft.FaceRefs", Some(f.faces))])?;
        writeln!(xml, "<FeatureDefinitionId>{}</FeatureDefinitionId></OtherShapeFeatureNominal>", f.definition)
            .unwrap();
    }
    xml.push_str("</FeatureNominals>");
    list(xml, "FeatureItems", features.len());
    for f in features {
        writeln!(xml, concat!("<OtherShapeFeatureItem id=\"{}\"><FeatureNominalId>{}</FeatureNominalId>",
            "<FeatureName>Face group {}</FeatureName><DeterminationMode><Checked/></DeterminationMode></OtherShapeFeatureItem>"), f.item, f.nominal, f.nominal).unwrap();
    }
    xml.push_str("</FeatureItems></Features>\n");
    Ok(())
}

fn write_characteristics(xml: &mut String, rows: &[Characteristic<'_>], features: &[Feature<'_>]) -> io::Result<()> {
    xml.push_str("<Characteristics><FormalStandardId>2</FormalStandardId>");
    list(xml, "CharacteristicDefinitions", rows.len());
    for c in rows {
        let tag = c.tag;
        write!(xml, "<{tag}CharacteristicDefinition id=\"{}\">", c.definition).unwrap();
        text(xml, "Description", c.row.callout)?;
        match c.row.requirement {
            Requirement::Dimension { tolerance, limits, .. } => {
                if tolerance == Tolerance::Basic {
                    xml.push_str("<DimensionType>BASIC_OR_TED</DimensionType>");
                }
                if let Some((lo, hi)) = limits {
                    write!(xml, "<Tolerance><MaxValue>{hi}</MaxValue><MinValue>{lo}</MinValue><DefinedAsLimit>true</DefinedAsLimit></Tolerance>").unwrap();
                } else if let Tolerance::Fit { class } = tolerance {
                    let zone = if class.deviation().as_bytes()[0].is_ascii_uppercase() { "HOLE" } else { "SHAFT" };
                    write!(xml, "<LimitsAndFitsSpecification zoneVariance=\"{zone}\"><FormVariance>{}</FormVariance><Grade>{}</Grade></LimitsAndFitsSpecification>", class.deviation(), class.grade()).unwrap();
                } else {
                    let mode = if tolerance == Tolerance::Basic { "SET" } else { "MEASURED" };
                    write!(xml, "<NonTolerance>{mode}</NonTolerance>").unwrap();
                }
            }
            Requirement::Geometric { limit_mm, diametrical, material, .. } => {
                write!(xml, "<ToleranceValue>{limit_mm}</ToleranceValue>").unwrap();
                if let Some(frame) = c.frame {
                    write!(xml, "<DatumReferenceFrameId>{frame}</DatumReferenceFrameId>").unwrap();
                }
                write!(xml, "<MaterialCondition>{}</MaterialCondition>", modifier(material)).unwrap();
                let zone = if diametrical {
                    "DiametricalZone"
                } else if tag == "Position" {
                    "NonDiametricalZone"
                } else {
                    "PlanarZone"
                };
                if tag != "Flatness" {
                    write!(xml, "<ZoneShape><{zone}/></ZoneShape>").unwrap();
                }
            }
            Requirement::RoughnessAverage { limit_um } => {
                write!(xml, "<RoughnessAverage linearUnit=\"µm\">{limit_um}</RoughnessAverage>").unwrap();
            }
            Requirement::Datum | Requirement::Omit(_) => unreachable!(),
        }
        writeln!(xml, "</{tag}CharacteristicDefinition>").unwrap();
    }
    xml.push_str("</CharacteristicDefinitions>");
    list(xml, "CharacteristicNominals", rows.len());
    for c in rows {
        let tag = c.tag;
        write!(xml, "<{tag}CharacteristicNominal id=\"{}\">", c.nominal).unwrap();
        attributes(
            xml,
            &[("SolidDraft.CharacteristicId", Some(c.row.id)), ("SolidDraft.LocatingDatum", c.row.locating_datum)],
        )?;
        write!(xml, "<CharacteristicDefinitionId>{}</CharacteristicDefinitionId>", c.definition).unwrap();
        references(xml, "FeatureNominalIds", c.features.iter().map(|&index| features[index].nominal));
        if let Requirement::Dimension { target, .. } = c.row.requirement {
            write!(xml, "<TargetValue>{target}</TargetValue>").unwrap();
        }
        writeln!(xml, "</{tag}CharacteristicNominal>").unwrap();
    }
    xml.push_str("</CharacteristicNominals>");
    let count = rows.iter().filter(|c| c.item.is_some()).count();
    if count != 0 {
        list(xml, "CharacteristicItems", count);
        for (c, id) in rows.iter().filter_map(|c| c.item.map(|id| (c, id))) {
            let tag = c.tag;
            write!(xml, "<{tag}CharacteristicItem id=\"{id}\"><CharacteristicDesignator>").unwrap();
            text(xml, "Designator", c.row.id)?;
            xml.push_str("</CharacteristicDesignator>");
            references(xml, "FeatureItemIds", c.features.iter().map(|&index| features[index].item));
            writeln!(xml, "<CharacteristicNominalId>{}</CharacteristicNominalId></{tag}CharacteristicItem>", c.nominal)
                .unwrap();
        }
        xml.push_str("</CharacteristicItems>");
    }
    xml.push_str("</Characteristics>\n");
    Ok(())
}

fn list(xml: &mut String, tag: &str, count: usize) {
    write!(xml, "<{tag} n=\"{count}\">").unwrap();
}

fn references(xml: &mut String, tag: &str, ids: impl ExactSizeIterator<Item = usize>) {
    if ids.len() == 0 {
        return;
    }
    list(xml, tag, ids.len());
    for id in ids {
        write!(xml, "<Id>{id}</Id>").unwrap();
    }
    write!(xml, "</{tag}>").unwrap();
}

fn modifier(material: MaterialCondition) -> &'static str {
    match material {
        MaterialCondition::Regardless => "NONE",
        MaterialCondition::Maximum => "MAXIMUM",
        MaterialCondition::Least => "LEAST",
    }
}

fn text(xml: &mut String, tag: &str, value: &str) -> io::Result<()> {
    write!(xml, "<{tag}>{}</{tag}>", escaped(value)?).unwrap();
    Ok(())
}

fn attributes(xml: &mut String, pairs: &[(&str, Option<&str>)]) -> io::Result<()> {
    let count = pairs.iter().filter(|(_, value)| value.is_some()).count();
    if count == 0 {
        return Ok(());
    }
    list(xml, "Attributes", count);
    for (name, value) in pairs.iter().filter_map(|(name, value)| value.map(|value| (*name, value))) {
        write!(xml, "<AttributeStr name=\"{name}\" value=\"{}\"/>", escaped(value)?).unwrap();
    }
    xml.push_str("</Attributes>");
    Ok(())
}

fn escaped(value: &str) -> io::Result<impl Display + '_> {
    if value.chars().any(|c| matches!(c, '\0'..='\x08' | '\x0b' | '\x0c' | '\x0e'..='\x1f' | '\u{fffe}' | '\u{ffff}')) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "text is not valid in XML 1.0"));
    }
    Ok(fmt::from_fn(move |f| {
        let mut offset = 0;
        for (i, c) in value.char_indices() {
            let escape = match c {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '"' => "&quot;",
                '\t' => "&#9;",
                '\n' => "&#10;",
                '\r' => "&#13;",
                _ => continue,
            };
            f.write_str(&value[offset..i])?;
            f.write_str(escape)?;
            offset = i + c.len_utf8();
        }
        f.write_str(&value[offset..])
    }))
}
