//! Manufacturing information on a part: datums, dimensions with their limits, geometric
//! tolerances and surface textures on faces, and the notes of the whole part.
//!
//! The tolerance types here ([`Tolerance`], [`GdtCharacteristic`], [`DatumRef`],
//! [`MaterialCondition`], [`SurfaceProcess`], [`Lay`]) are the part's and the drawing's
//! alike: a drawing shows what the part states.
//!
//! A [`Pmi`] is generic over what names a face. The part document names faces by its
//! persistent references, a kernel call by [`FaceId`](crate::FaceId), and a STEP file by
//! the face keys its import reports.
//!
//! A slot with round ends is one item, as on a drawing (`20×6 +0.1/0 R3`); a STEP file
//! holds its length, its width and its end radius as three dimensions.
//!
//! A dimension holds no size of its own. Its nominal value is measured on the body
//! ([`Kernel::pmi_nominals`](crate::Kernel::pmi_nominals)), so an edit that moves a face
//! moves the dimension with it. A file that was read states the value it carries in
//! `nominal`.

use serde::{Deserialize, Serialize};

/// A decimal number as people write it, such as `0.05`. Equal numbers are equal values:
/// `0.10`, `+0.1` and `.1` all read as `0.1`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Decimal(String);

impl Decimal {
    /// Reads a decimal number: an optional sign, digits and an optional fraction, at most
    /// fifteen significant digits. Returns `None` for anything else, exponents included.
    pub fn parse(text: &str) -> Option<Decimal> {
        let text = text.trim();
        let (negative, digits) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        if whole.is_empty() && fraction.is_empty() {
            return None;
        }
        if !whole.bytes().chain(fraction.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let whole = whole.trim_start_matches('0');
        let fraction = fraction.trim_end_matches('0');
        if whole.len() + fraction.len() > 15 {
            return None;
        }
        let mut out = String::new();
        if negative && !(whole.is_empty() && fraction.is_empty()) {
            out.push('-');
        }
        out.push_str(if whole.is_empty() { "0" } else { whole });
        if !fraction.is_empty() {
            out.push('.');
            out.push_str(fraction);
        }
        Some(Decimal(out))
    }

    /// The decimal nearest a measured value, to nine places.
    pub fn from_f64(value: f64) -> Option<Decimal> {
        if !value.is_finite() {
            return None;
        }
        Decimal::parse(&format!("{value:.9}"))
    }

    /// The number as text, such as `0.05`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The number as a float.
    pub fn value(&self) -> f64 {
        self.0.parse().unwrap_or(0.0)
    }

    /// True when the number is above zero.
    pub fn is_positive(&self) -> bool {
        self.0 != "0" && !self.0.starts_with('-')
    }
}

impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Decimal {
    type Error = String;

    fn try_from(text: String) -> Result<Decimal, String> {
        Decimal::parse(&text).ok_or_else(|| format!("`{text}` is not a decimal number"))
    }
}

impl From<Decimal> for String {
    fn from(value: Decimal) -> String {
        value.0
    }
}

/// The faces of one feature: a bore's cylinder, or the two flats of a slot.
pub type Feature<F> = Vec<F>;

/// An ISO 286 tolerance class, such as `H7` or `js6`: one or two letters, capitals for
/// a hole and small letters for a shaft, and a grade of one or two digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FitClass {
    text: [u8; 4],
    letters: u8,
    len: u8,
}

impl FitClass {
    /// Reads a class as people write it.
    pub fn parse(class: &str) -> Option<FitClass> {
        let class = class.trim();
        let split = class.find(|c: char| c.is_ascii_digit())?;
        let (deviation, grade) = class.split_at(split);
        let letters = (1..=2).contains(&deviation.len())
            && (deviation.bytes().all(|b| b.is_ascii_uppercase()) || deviation.bytes().all(|b| b.is_ascii_lowercase()));
        let digits = (1..=2).contains(&grade.len()) && grade.bytes().all(|b| b.is_ascii_digit());
        if !(letters && digits) {
            return None;
        }
        let mut text = [0; 4];
        text[..class.len()].copy_from_slice(class.as_bytes());
        Some(FitClass { text, letters: deviation.len() as u8, len: class.len() as u8 })
    }

    /// The whole class, such as `H7`.
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.text[..usize::from(self.len)]).unwrap_or_default()
    }

    /// The fundamental deviation, such as `H`.
    pub fn deviation(&self) -> &str {
        &self.as_str()[..usize::from(self.letters)]
    }

    /// The tolerance grade, such as `7`.
    pub fn grade(&self) -> &str {
        &self.as_str()[usize::from(self.letters)..]
    }
}

impl std::fmt::Display for FitClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for FitClass {
    type Error = String;

    fn try_from(text: String) -> Result<FitClass, String> {
        FitClass::parse(&text).ok_or_else(|| format!("`{text}` is not a tolerance class such as H7"))
    }
}

impl From<FitClass> for String {
    fn from(class: FitClass) -> String {
        class.as_str().to_owned()
    }
}

/// The tolerance of a dimension, in millimetres or degrees.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Tolerance {
    /// No tolerance shown: the part's general tolerance applies.
    #[default]
    None,
    /// Plus or minus one value.
    Symmetric {
        /// The deviation, greater than zero.
        value: f64,
    },
    /// Separate upper and lower deviations.
    Deviation {
        /// Upper deviation.
        upper: f64,
        /// Lower deviation, negative below the nominal value.
        lower: f64,
    },
    /// The deviations shown as upper and lower limits.
    Limits {
        /// Upper deviation.
        upper: f64,
        /// Lower deviation, negative below the nominal value.
        lower: f64,
    },
    /// An ISO 286 tolerance class.
    Fit {
        /// The class, such as H7.
        class: FitClass,
    },
    /// Theoretically exact (basic): the value places a tolerance zone and takes no
    /// limits of its own. A geometric tolerance governs the feature it locates.
    Basic,
}

impl Tolerance {
    /// True when no tolerance is given.
    pub fn is_none(&self) -> bool {
        *self == Tolerance::None
    }

    /// The upper and lower deviations in millimetres, for a tolerance given as numbers.
    pub fn deviations(&self) -> Option<(f64, f64)> {
        match *self {
            Tolerance::Symmetric { value } => Some((value, -value)),
            Tolerance::Deviation { upper, lower } | Tolerance::Limits { upper, lower } => Some((upper, lower)),
            Tolerance::None | Tolerance::Fit { .. } | Tolerance::Basic => None,
        }
    }

    /// The tolerance as a callout writes it after the value: ` H7`, ` ±0.1` or
    /// ` +0.1/0`; nothing for none and for basic.
    pub fn suffix(&self) -> String {
        let signed = |value: f64| {
            let text = length_text(value);
            if value > 0.0 { format!("+{text}") } else { text }
        };
        match *self {
            Tolerance::None | Tolerance::Basic => String::new(),
            Tolerance::Fit { class } => format!(" {class}"),
            Tolerance::Symmetric { value } => format!(" \u{b1}{}", length_text(value)),
            Tolerance::Deviation { upper, lower } | Tolerance::Limits { upper, lower } => {
                format!(" {}/{}", signed(upper), signed(lower))
            }
        }
    }

    /// Reads a tolerance as people type it: a class (`H7`), a symmetric deviation
    /// (`±0.1`, `+-0.1`), an upper and a lower deviation (`+0.1/0`, `+0.2/-0.1`), or
    /// `basic`.
    pub fn parse(text: &str) -> Option<Tolerance> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("basic") {
            return Some(Tolerance::Basic);
        }
        if let Some(class) = FitClass::parse(text) {
            return Some(Tolerance::Fit { class });
        }
        let symmetric =
            text.strip_prefix('\u{b1}').or_else(|| text.strip_prefix("+-")).or_else(|| text.strip_prefix("+/-"));
        if let Some(value) = symmetric {
            let value = Decimal::parse(value)?.value();
            return (value > 0.0).then_some(Tolerance::Symmetric { value });
        }
        let (upper, lower) = text.split_once('/')?;
        let (upper, lower) = (Decimal::parse(upper)?.value(), Decimal::parse(lower)?.value());
        (upper > lower).then_some(Tolerance::Deviation { upper, lower })
    }

    /// Why the tolerance cannot stand, or `None` when it can.
    pub fn problem(&self) -> Option<&'static str> {
        match *self {
            Tolerance::Symmetric { value } if !(value > 0.0 && value.is_finite()) => {
                Some("The deviation must be more than zero.")
            }
            Tolerance::Deviation { upper, lower } | Tolerance::Limits { upper, lower }
                if !(upper > lower && upper.is_finite() && lower.is_finite()) =>
            {
                Some("The upper deviation must be above the lower one.")
            }
            _ => None,
        }
    }
}

/// What a surface finish allows the process to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceProcess {
    /// Any process: the basic symbol.
    #[default]
    Any,
    /// Material must be removed: the symbol with a bar.
    Removal,
    /// Material must not be removed: the symbol with a circle.
    NoRemoval,
}

/// The direction of the lay on a surface finish symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lay {
    /// Parallel to the projection plane of the view: "=".
    Parallel,
    /// Perpendicular to it: "⊥".
    Perpendicular,
    /// Crossed in two directions: "X".
    Crossed,
    /// Many directions: "M".
    Multidirectional,
    /// Roughly circular about the surface's centre: "C".
    Circular,
    /// Roughly radial: "R".
    Radial,
    /// Particulate, non-directional: "P".
    Particulate,
}

impl Lay {
    /// The letter or sign written on the symbol.
    pub fn symbol(self) -> &'static str {
        match self {
            Lay::Parallel => "=",
            Lay::Perpendicular => "⊥",
            Lay::Crossed => "X",
            Lay::Multidirectional => "M",
            Lay::Circular => "C",
            Lay::Radial => "R",
            Lay::Particulate => "P",
        }
    }
}

/// A geometric characteristic of ISO 1101 and ASME Y14.5.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GdtCharacteristic {
    /// Form: straightness.
    Straightness,
    /// Form: flatness.
    Flatness,
    /// Form: circularity (roundness).
    Circularity,
    /// Form: cylindricity.
    Cylindricity,
    /// Profile of a line.
    ProfileOfLine,
    /// Profile of a surface.
    ProfileOfSurface,
    /// Orientation: angularity.
    Angularity,
    /// Orientation: perpendicularity.
    Perpendicularity,
    /// Orientation: parallelism.
    Parallelism,
    /// Location: position.
    Position,
    /// Location: concentricity (coaxiality).
    Concentricity,
    /// Location: symmetry.
    Symmetry,
    /// Runout: circular runout.
    CircularRunout,
    /// Runout: total runout.
    TotalRunout,
}

impl GdtCharacteristic {
    /// Every characteristic, in the standards' order.
    pub const ALL: [GdtCharacteristic; 14] = [
        GdtCharacteristic::Straightness,
        GdtCharacteristic::Flatness,
        GdtCharacteristic::Circularity,
        GdtCharacteristic::Cylindricity,
        GdtCharacteristic::ProfileOfLine,
        GdtCharacteristic::ProfileOfSurface,
        GdtCharacteristic::Angularity,
        GdtCharacteristic::Perpendicularity,
        GdtCharacteristic::Parallelism,
        GdtCharacteristic::Position,
        GdtCharacteristic::Concentricity,
        GdtCharacteristic::Symmetry,
        GdtCharacteristic::CircularRunout,
        GdtCharacteristic::TotalRunout,
    ];

    /// The name in words, as in pickers.
    pub fn name(self) -> &'static str {
        match self {
            GdtCharacteristic::Straightness => "Straightness",
            GdtCharacteristic::Flatness => "Flatness",
            GdtCharacteristic::Circularity => "Circularity",
            GdtCharacteristic::Cylindricity => "Cylindricity",
            GdtCharacteristic::ProfileOfLine => "Profile of a line",
            GdtCharacteristic::ProfileOfSurface => "Profile of a surface",
            GdtCharacteristic::Angularity => "Angularity",
            GdtCharacteristic::Perpendicularity => "Perpendicularity",
            GdtCharacteristic::Parallelism => "Parallelism",
            GdtCharacteristic::Position => "Position",
            GdtCharacteristic::Concentricity => "Concentricity",
            GdtCharacteristic::Symmetry => "Symmetry",
            GdtCharacteristic::CircularRunout => "Circular runout",
            GdtCharacteristic::TotalRunout => "Total runout",
        }
    }

    /// The snake-case key, as in files and on the command line.
    pub fn key(self) -> &'static str {
        match self {
            GdtCharacteristic::Straightness => "straightness",
            GdtCharacteristic::Flatness => "flatness",
            GdtCharacteristic::Circularity => "circularity",
            GdtCharacteristic::Cylindricity => "cylindricity",
            GdtCharacteristic::ProfileOfLine => "profile_of_line",
            GdtCharacteristic::ProfileOfSurface => "profile_of_surface",
            GdtCharacteristic::Angularity => "angularity",
            GdtCharacteristic::Perpendicularity => "perpendicularity",
            GdtCharacteristic::Parallelism => "parallelism",
            GdtCharacteristic::Position => "position",
            GdtCharacteristic::Concentricity => "concentricity",
            GdtCharacteristic::Symmetry => "symmetry",
            GdtCharacteristic::CircularRunout => "circular_runout",
            GdtCharacteristic::TotalRunout => "total_runout",
        }
    }

    /// The characteristic with this key.
    pub fn from_key(key: &str) -> Option<GdtCharacteristic> {
        GdtCharacteristic::ALL.into_iter().find(|c| c.key() == key)
    }

    /// Form tolerances never take datums; orientation and runout tolerances always do.
    pub fn takes_datums(self) -> Option<bool> {
        match self {
            GdtCharacteristic::Straightness
            | GdtCharacteristic::Flatness
            | GdtCharacteristic::Circularity
            | GdtCharacteristic::Cylindricity => Some(false),
            GdtCharacteristic::Angularity
            | GdtCharacteristic::Perpendicularity
            | GdtCharacteristic::Parallelism
            | GdtCharacteristic::CircularRunout
            | GdtCharacteristic::TotalRunout => Some(true),
            _ => None,
        }
    }
}

/// The material condition a tolerance or datum reference applies at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialCondition {
    /// Regardless of feature size: no modifier.
    #[default]
    Regardless,
    /// Maximum material condition: Ⓜ.
    Maximum,
    /// Least material condition: Ⓛ.
    Least,
}

impl MaterialCondition {
    /// True for regardless of feature size, which writes nothing.
    pub fn is_regardless(&self) -> bool {
        *self == MaterialCondition::Regardless
    }

    /// The modifier letter drawn in a circle: "M", "L" or "".
    pub fn letter(self) -> &'static str {
        match self {
            MaterialCondition::Regardless => "",
            MaterialCondition::Maximum => "M",
            MaterialCondition::Least => "L",
        }
    }
}

/// One datum reference of a feature control frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatumRef {
    /// The datum letter, or a common datum such as "A-B".
    pub letter: String,
    /// The material condition it applies at.
    #[serde(default, skip_serializing_if = "MaterialCondition::is_regardless")]
    pub material: MaterialCondition,
}

impl DatumRef {
    /// A datum reference regardless of feature size.
    pub fn new(letter: &str) -> DatumRef {
        DatumRef { letter: letter.to_owned(), material: MaterialCondition::Regardless }
    }

    /// The single letters the reference names: `A` and `B` for the common datum `A-B`.
    pub fn letters(&self) -> impl Iterator<Item = &str> {
        self.letter.split('-').map(str::trim)
    }
}

/// The manufacturing information of one part.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pmi<F> {
    /// The items attached to faces, in the order the part lists them.
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<PmiItem<F>>,
    /// What applies to the whole part.
    #[serde(default, skip_serializing_if = "PartNotes::is_empty")]
    pub part: PartNotes,
}

impl<F> Default for Pmi<F> {
    fn default() -> Self {
        Pmi { items: Vec::new(), part: PartNotes::default() }
    }
}

impl<F> Pmi<F> {
    /// True when the part carries no manufacturing information.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.part.is_empty()
    }

    /// The item with this name.
    pub fn item(&self, id: &str) -> Option<&PmiItem<F>> {
        self.items.iter().find(|item| item.id == id)
    }

    /// The same information with every face named another way. Stops at the first face
    /// `map` cannot name and returns its error.
    pub fn try_map<G, E>(&self, mut map: impl FnMut(&PmiItem<F>, &F) -> Result<G, E>) -> Result<Pmi<G>, E> {
        let mut items = Vec::with_capacity(self.items.len());
        for item in &self.items {
            items.push(item.try_map(|face| map(item, face))?);
        }
        Ok(Pmi { items, part: self.part.clone() })
    }
}

/// One piece of manufacturing information on some faces.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PmiItem<F> {
    /// The item's name within the part, such as `Bore diameter`. Unique; a STEP file
    /// keeps it.
    pub id: String,
    /// What the item says.
    #[serde(flatten)]
    pub kind: PmiKind<F>,
}

impl<F> PmiItem<F> {
    /// The same item with every face named another way.
    pub fn try_map<G, E>(&self, mut map: impl FnMut(&F) -> Result<G, E>) -> Result<PmiItem<G>, E> {
        self.map_attachments(
            &mut map,
            |map, faces| faces.iter().map(map).collect(),
            |map, face| map(face),
        )
    }

    /// Maps feature groups, which may expand when split. The mapper must preserve
    /// single-face attachments when given a singleton; use `map_attachments` to
    /// resolve ambiguity separately when mapping to live geometry.
    pub fn try_map_features<G, E>(
        &self,
        mut feature: impl FnMut(&[F]) -> Result<Feature<G>, E>,
    ) -> Result<PmiItem<G>, E> {
        self.map_attachments(&mut feature, |map, faces| map(faces), |map, face| {
            let [face]: [G; 1] = map(std::slice::from_ref(face))?.try_into().ok().expect("one mapped attachment");
            Ok(face)
        })
    }

    /// Maps groups and exact single-face attachments separately.
    pub fn map_attachments<G, E, M>(
        &self,
        state: &mut M,
        mut group: impl FnMut(&mut M, &[F]) -> Result<Feature<G>, E>,
        mut face: impl FnMut(&mut M, &F) -> Result<G, E>,
    ) -> Result<PmiItem<G>, E> {
        let mut feature = |faces: &Feature<F>| group(state, faces);
        let kind = match &self.kind {
            PmiKind::Datum { letter, feature: faces } => {
                PmiKind::Datum { letter: letter.clone(), feature: feature(faces)? }
            }
            PmiKind::Size { size, features, tolerance, through, thread, nominal } => PmiKind::Size {
                size: *size,
                features: features.iter().map(&mut feature).collect::<Result<_, _>>()?,
                tolerance: *tolerance,
                through: *through,
                thread: thread.clone(),
                nominal: *nominal,
            },
            PmiKind::Hole { pilot, head, tolerance, through, drill_depth, thread } => PmiKind::Hole {
                pilot: feature(pilot)?,
                head: match head {
                    Some(head) => Some(HoleHead { kind: head.kind, faces: feature(&head.faces)? }),
                    None => None,
                },
                tolerance: *tolerance,
                through: *through,
                drill_depth: drill_depth.clone(),
                thread: thread.clone(),
            },
            PmiKind::Location { from, to, tolerance, nominal } => PmiKind::Location {
                from: Anchor { feature: feature(&from.feature)?, at: from.at },
                to: Anchor { feature: feature(&to.feature)?, at: to.at },
                tolerance: *tolerance,
                nominal: *nominal,
            },
            PmiKind::Slot { flats, ends, tolerance } => PmiKind::Slot {
                flats: [feature(&flats[0])?, feature(&flats[1])?],
                ends: [feature(&ends[0])?, feature(&ends[1])?],
                tolerance: *tolerance,
            },
            PmiKind::Width { flats: [a, b], tolerance } => {
                PmiKind::Width { flats: [feature(a)?, feature(b)?], tolerance: *tolerance }
            }
            PmiKind::Chamfer { bevel, sides: [a, b] } => {
                PmiKind::Chamfer { bevel: feature(bevel)?, sides: [feature(a)?, feature(b)?] }
            }
            PmiKind::EdgeBreak { bevel, sides: [a, b], min_mm, max_mm } => PmiKind::EdgeBreak {
                bevel: feature(bevel)?,
                sides: [feature(a)?, feature(b)?],
                min_mm: min_mm.clone(),
                max_mm: max_mm.clone(),
            },
            PmiKind::BasicAngle { planes: [a, b] } => PmiKind::BasicAngle { planes: [feature(a)?, feature(b)?] },
            PmiKind::DatumTarget(target) => PmiKind::DatumTarget(DatumTarget {
                datum: target.datum.clone(),
                number: target.number,
                face: face(state, &target.face)?,
                point_mm: target.point_mm,
                kind: target.kind.clone(),
            }),
            PmiKind::PositionControl(control) => {
                let features = control.features.iter().map(&mut feature).collect::<Result<_, _>>()?;
                let frame = match &control.frame {
                    PositionFrame::Single { segment, projected_zone } => PositionFrame::Single {
                        segment: segment.clone(),
                        projected_zone: projected_zone.as_ref().map(|zone| {
                            Ok::<_, E>(ProjectedZone {
                                height_mm: zone.height_mm.clone(),
                                direction: zone.direction,
                                entry_faces: zone.entry_faces.iter().map(|entry| face(state, entry)).collect::<Result<_, E>>()?,
                            })
                        }).transpose()?,
                    },
                    PositionFrame::Composite { upper, lower } => {
                        PositionFrame::Composite { upper: upper.clone(), lower: lower.clone() }
                    }
                };
                PmiKind::PositionControl(PositionControl { features, frame })
            }
            PmiKind::Gdt { characteristic, tolerance, diameter, material, datums, features } => PmiKind::Gdt {
                characteristic: *characteristic,
                tolerance: *tolerance,
                diameter: *diameter,
                material: *material,
                datums: datums.clone(),
                features: features.iter().map(&mut feature).collect::<Result<_, _>>()?,
            },
            PmiKind::SurfaceFinish { text, process, method, lay, allowance, all_around, feature: faces } => {
                PmiKind::SurfaceFinish {
                    text: text.clone(),
                    process: *process,
                    method: method.clone(),
                    lay: *lay,
                    allowance: allowance.clone(),
                    all_around: *all_around,
                    feature: feature(faces)?,
                }
            }
        };
        Ok(PmiItem { id: self.id.clone(), kind })
    }

    /// Every feature the item names, in order.
    pub fn features(&self) -> Vec<&[F]> {
        match &self.kind {
            PmiKind::Datum { feature, .. } | PmiKind::SurfaceFinish { feature, .. } => vec![feature],
            PmiKind::Size { features, .. } | PmiKind::Gdt { features, .. } => {
                features.iter().map(Vec::as_slice).collect()
            }
            PmiKind::DatumTarget(target) => vec![std::slice::from_ref(&target.face)],
            PmiKind::PositionControl(control) => {
                let mut faces: Vec<_> = control.features.iter().map(Vec::as_slice).collect();
                if let PositionFrame::Single { projected_zone: Some(zone), .. } = &control.frame {
                    faces.extend(zone.entry_faces.iter().map(std::slice::from_ref));
                }
                faces
            }
            PmiKind::Hole { pilot, head, .. } => {
                if let Some(head) = head {
                    vec![pilot, &head.faces]
                } else {
                    vec![pilot]
                }
            }
            PmiKind::Location { from, to, .. } => vec![&from.feature, &to.feature],
            PmiKind::Slot { flats, ends, .. } => flats.iter().chain(ends).map(Vec::as_slice).collect(),
            PmiKind::Width { flats, .. } | PmiKind::BasicAngle { planes: flats } => {
                flats.iter().map(Vec::as_slice).collect()
            }
            PmiKind::Chamfer { bevel, sides } | PmiKind::EdgeBreak { bevel, sides, .. } => {
                vec![bevel, &sides[0], &sides[1]]
            }
        }
    }

    /// True for a size, a distance or a slot: an item with measured values.
    pub fn is_dimension(&self) -> bool {
        matches!(
            self.kind,
            PmiKind::Size { .. }
                | PmiKind::Hole { .. }
                | PmiKind::Location { .. }
                | PmiKind::Slot { .. }
                | PmiKind::Width { .. }
                | PmiKind::Chamfer { .. }
                | PmiKind::EdgeBreak { .. }
                | PmiKind::BasicAngle { .. }
        )
    }
}

/// What an item says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PmiKind<F> {
    /// A datum feature and its letter.
    Datum {
        /// The datum letter, such as `A`.
        letter: String,
        /// The feature that establishes it.
        feature: Feature<F>,
    },
    /// The size of one feature, or of every feature of a pattern.
    Size {
        /// Which size.
        size: SizeKind,
        /// The features it applies to. Several are a pattern, such as four equal holes.
        features: Vec<Feature<F>>,
        /// The tolerance. None leaves the size under the part's general tolerance.
        #[serde(default, skip_serializing_if = "Tolerance::is_none")]
        tolerance: Tolerance,
        /// The size is a hole's diameter and the hole goes through the part.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        through: bool,
        /// The feature is threaded: its designation, such as `M6x1-6H`. The diameter is
        /// then that of the hole the thread is cut in, and the designation states the
        /// thread's own size and fit, so the item takes no tolerance.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
        /// The value a STEP file stated, in millimetres. `None` in a part: the size is
        /// measured on the body.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nominal: Option<f64>,
    },
    /// One hole with optional compound head and independently specified drill and thread depths.
    Hole {
        /// The cylindrical pilot wall, including its pieces if split.
        pilot: Feature<F>,
        /// The counterbore wall or countersink cone, if present.
        #[serde(default = "Option::default", skip_serializing_if = "Option::is_none")]
        head: Option<HoleHead<F>>,
        /// Limits on the pilot diameter. None leaves them unstated.
        #[serde(default, skip_serializing_if = "Tolerance::is_none")]
        tolerance: Tolerance,
        /// Whether the drill, not necessarily the thread, breaks through.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        through: bool,
        /// Explicit drill depth in millimetres. If absent on a blind hole, measure it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        drill_depth: Option<Decimal>,
        /// The thread's designation and optional usable depth, separate from the drill.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<HoleThread>,
    },
    /// The distance between two features.
    Location {
        /// The feature it is measured from.
        from: Anchor<F>,
        /// The feature it is measured to.
        to: Anchor<F>,
        /// The tolerance. None leaves the distance under the general tolerance.
        #[serde(default, skip_serializing_if = "Tolerance::is_none")]
        tolerance: Tolerance,
        /// The value a STEP file stated, in millimetres. `None` in a part.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nominal: Option<f64>,
    },
    /// A slot with round ends, as one item: its length over the ends, the width between
    /// its flats and the radius of its ends, such as `20×6 +0.1/0 R3`.
    Slot {
        /// The two flat sides.
        flats: [Feature<F>; 2],
        /// The two round ends.
        ends: [Feature<F>; 2],
        /// The tolerance of the length and the width.
        #[serde(default, skip_serializing_if = "Tolerance::is_none")]
        tolerance: Tolerance,
    },
    /// The width between two opposed flat sides, in millimetres.
    Width {
        /// The two opposed planar face groups.
        flats: [Feature<F>; 2],
        /// The size tolerance, if stated.
        #[serde(default, skip_serializing_if = "Tolerance::is_none")]
        tolerance: Tolerance,
    },
    /// Setback from the theoretical corner along the first side, in millimetres,
    /// and the angle from that side to the bevel, in degrees.
    Chamfer {
        /// The planar bevel.
        bevel: Feature<F>,
        /// The two adjacent planar faces, reference side first.
        sides: [Feature<F>; 2],
    },
    /// Explicit bounds on the largest setback of a planar broken edge.
    EdgeBreak {
        /// The planar break face.
        bevel: Feature<F>,
        /// The adjacent planar faces.
        sides: [Feature<F>; 2],
        /// Minimum setback in millimetres.
        min_mm: Decimal,
        /// Maximum setback in millimetres.
        max_mm: Decimal,
    },
    /// The basic included angle between two planes, in degrees.
    BasicAngle {
        /// The two planar face groups.
        planes: [Feature<F>; 2],
    },
    /// A bounded target on a face belonging to a declared datum.
    DatumTarget(DatumTarget<F>),
    /// Position control with a single or composite frame.
    PositionControl(PositionControl<F>),
    /// A geometric tolerance: a feature control frame (ISO 1101, ASME Y14.5).
    Gdt {
        /// The geometric characteristic.
        characteristic: GdtCharacteristic,
        /// The tolerance value in millimetres.
        tolerance: f64,
        /// The tolerance zone is a cylinder: the value carries a diameter sign.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        diameter: bool,
        /// The material condition the tolerance applies at.
        #[serde(default, skip_serializing_if = "MaterialCondition::is_regardless")]
        material: MaterialCondition,
        /// Datum references, primary first.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        datums: Vec<DatumRef>,
        /// The features it applies to. Several are a pattern.
        features: Vec<Feature<F>>,
    },
    /// A surface finish requirement (ISO 1302, ASME Y14.36).
    SurfaceFinish {
        /// The roughness requirement, such as "Ra 1.6".
        text: String,
        /// What the process may do to the surface.
        #[serde(default)]
        process: SurfaceProcess,
        /// The production method, such as "Ground".
        #[serde(default, skip_serializing_if = "String::is_empty")]
        method: String,
        /// The direction of the lay.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lay: Option<Lay>,
        /// The machining allowance, such as "0.5".
        #[serde(default, skip_serializing_if = "String::is_empty")]
        allowance: String,
        /// The requirement holds all around the outline.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        all_around: bool,
        /// The feature it applies to.
        feature: Feature<F>,
    },
}

/// The target point or center is in the part coordinate frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatumTarget<F> {
    pub datum: String,
    pub number: u32,
    pub face: F,
    pub point_mm: [f64; 3],
    pub kind: TargetKind,
}

/// Target extent; sizes are full dimensions in millimetres.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetKind {
    Point,
    Line { length_mm: Decimal, direction: [f64; 3] },
    Area { area: TargetArea },
}

/// Area target footprint on the supporting face.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetArea {
    Circular { diameter_mm: Decimal },
    Rectangular { length_mm: Decimal, width_mm: Decimal, direction: [f64; 3] },
}

/// One position control, with all its segments governing the same ordered groups.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PositionControl<F> {
    pub features: Vec<Feature<F>>,
    pub frame: PositionFrame<F>,
}

/// A composite is ordered and cannot have a projected zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "frame", rename_all = "snake_case", deny_unknown_fields)]
pub enum PositionFrame<F> {
    Single { segment: PositionSegment, projected_zone: Option<ProjectedZone<F>> },
    Composite { upper: PositionSegment, lower: PositionSegment },
}

impl<F> PositionFrame<F> {
    pub fn segments(&self) -> impl Iterator<Item = &PositionSegment> {
        let (first, lower) = match self {
            Self::Single { segment, .. } => (segment, None),
            Self::Composite { upper, lower } => (upper, Some(lower)),
        };
        std::iter::once(first).chain(lower)
    }

    pub fn segments_mut(&mut self) -> impl Iterator<Item = &mut PositionSegment> {
        let (first, lower) = match self {
            Self::Single { segment, .. } => (segment, None),
            Self::Composite { upper, lower } => (upper, Some(lower)),
        };
        std::iter::once(first).chain(lower)
    }
}

/// Total zone size and the independent feature and datum material conditions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionSegment {
    pub value_mm: Decimal,
    pub diameter_zone: bool,
    #[serde(default, skip_serializing_if = "MaterialCondition::is_regardless")]
    pub material: MaterialCondition,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub datums: Vec<DatumRef>,
}

/// Height follows an outward part-space axis; entry faces follow feature order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedZone<F> {
    pub height_mm: Decimal,
    pub direction: [f64; 3],
    pub entry_faces: Vec<F>,
}

/// The separate wall faces of the enlargement at a hole's entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoleHead<F> {
    /// The shape of the enlargement.
    pub kind: HoleHeadKind,
    /// The wall of the head, including its pieces if split.
    pub faces: Feature<F>,
}

/// The shape of the enlargement at a hole's entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoleHeadKind {
    /// Cylindrical enlargement bounded by a shoulder.
    Counterbore,
    /// Conical enlargement, specified by its full included angle.
    Countersink,
}

/// A hole's explicitly stated thread requirements.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoleThread {
    /// The thread designation, such as `M6x1-6H`.
    pub designation: String,
    /// The usable axial thread depth, in millimetres from the entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<Decimal>,
}

/// Checked nominal values of one hole, independent of its stated requirements.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct HoleNominals {
    /// Diameter of the cylindrical pilot wall, in millimetres.
    pub pilot_diameter_mm: f64,
    /// The enlargement's dimensions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<HoleHeadNominals>,
    /// Axial depth from entry to the end of the full drill diameter, in millimetres.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drill_depth_mm: Option<f64>,
}

/// Checked dimensions of a hole head.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HoleHeadNominals {
    /// A counterbore's cylindrical diameter and axial depth from entry to shoulder.
    Counterbore {
        /// Diameter in millimetres.
        diameter_mm: f64,
        /// Depth in millimetres.
        depth_mm: f64,
    },
    /// A countersink's major diameter at entry and full included angle.
    Countersink {
        /// Diameter in millimetres.
        diameter_mm: f64,
        /// Full included angle in degrees.
        included_angle_deg: f64,
    },
}

/// Which size of a feature a dimension gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeKind {
    /// The diameter of a cylindrical feature.
    Diameter,
    /// The radius of a cylindrical feature.
    Radius,
}

/// One end of a distance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anchor<F> {
    /// The feature.
    pub feature: Feature<F>,
    /// Where on the feature the distance ends.
    #[serde(default, skip_serializing_if = "At::is_feature")]
    pub at: At,
}

/// Where on a feature a distance ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum At {
    /// The feature itself: a plane's surface, a cylinder's axis.
    #[default]
    Feature,
    /// Midway between the feature's two faces: the centre plane of a slot's flats.
    Middle,
    /// The side of a round feature nearest the other end.
    Inner,
    /// The side of a round feature farthest from the other end.
    Outer,
}

impl At {
    fn is_feature(&self) -> bool {
        *self == At::Feature
    }
}

/// A surface finish's roughness as a parameter and its limit in micrometres: `Ra` and
/// `1.6` for "Ra 1.6". `None` for a requirement written any other way.
pub fn roughness(text: &str) -> Option<(&str, Decimal)> {
    let mut words = text.split_whitespace();
    let (parameter, value) = (words.next()?, words.next()?);
    let named = parameter.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && parameter.bytes().all(|b| b.is_ascii_alphanumeric());
    let value = Decimal::parse(value).filter(Decimal::is_positive)?;
    (named && words.next().is_none()).then_some((parameter, value))
}

/// What applies to the whole part.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartNotes {
    /// The general tolerance, such as `ISO 2768-m`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub general_tolerance: Option<String>,
    /// The material as the drawing names it, such as `6061-T6`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Other notes, such as `Break sharp edges`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl PartNotes {
    /// True when the part has no note at all.
    pub fn is_empty(&self) -> bool {
        self.general_tolerance.is_none() && self.material.is_none() && self.notes.is_empty()
    }
}

/// A datum letter is one or two capital letters.
pub fn is_datum_letter(letter: &str) -> bool {
    (1..=2).contains(&letter.len()) && letter.bytes().all(|b| b.is_ascii_uppercase())
}

impl<F: PartialEq> Pmi<F> {
    /// The letters of the part's datums, in order.
    pub fn datum_letters(&self) -> Vec<&str> {
        self.items
            .iter()
            .filter_map(|item| match &item.kind {
                PmiKind::Datum { letter, .. } => Some(letter.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The feature of datum `letter`.
    pub fn datum_feature(&self, letter: &str) -> Option<&Feature<F>> {
        self.items.iter().find_map(|item| match &item.kind {
            PmiKind::Datum { letter: named, feature } if named == letter => Some(feature),
            _ => None,
        })
    }

    /// The first unused datum letter: `A`, `B`, ... skipping `I`, `O` and `Q`, which
    /// drawings leave out.
    pub fn next_datum_letter(&self) -> String {
        let used = self.datum_letters();
        ('A'..='Z')
            .filter(|c| !matches!(c, 'I' | 'O' | 'Q'))
            .map(String::from)
            .find(|letter| !used.contains(&letter.as_str()))
            .unwrap_or_else(|| "AA".to_owned())
    }

    /// A name no item has yet, made from `base`: `Diameter`, then `Diameter 2`.
    pub fn free_name(&self, base: &str) -> String {
        if self.item(base).is_none() {
            return base.to_owned();
        }
        (2..).map(|n| format!("{base} {n}")).find(|name| self.item(name).is_none()).unwrap_or_default()
    }

    /// Why `item` cannot be put in the part, replacing the item named `replacing` if
    /// there is one: a name or datum letter already taken, an empty feature, a datum
    /// nobody defined, values out of range. `None` when it fits.
    pub fn problem_with(&self, item: &PmiItem<F>, replacing: Option<&str>) -> Option<String> {
        let others = || self.items.iter().filter(|other| Some(other.id.as_str()) != replacing);
        if item.id.trim().is_empty() {
            return Some("The item needs a name.".into());
        }
        if item.id.contains('/') {
            return Some("A name cannot hold a slash.".into());
        }
        if others().any(|other| other.id == item.id) {
            return Some(format!("Another item is already called {}.", item.id));
        }
        for feature in item.features() {
            if feature.is_empty() {
                return Some("Pick the faces it applies to.".into());
            }
            if feature.iter().enumerate().any(|(i, face)| feature[..i].contains(face)) {
                return Some("A face is picked twice.".into());
            }
        }
        if matches!(
            item.kind,
            PmiKind::Width { .. } | PmiKind::Chamfer { .. } | PmiKind::EdgeBreak { .. } | PmiKind::BasicAngle { .. }
        ) {
            let groups = item.features();
            if groups
                .iter()
                .enumerate()
                .any(|(i, group)| groups[..i].iter().any(|other| group.iter().any(|face| other.contains(face))))
            {
                return Some("Pick different faces for each group.".into());
            }
        }
        let range = |tolerance: &Tolerance| tolerance.problem().map(str::to_owned);
        match &item.kind {
            PmiKind::Datum { letter, feature } => {
                if !is_datum_letter(letter) {
                    return Some("A datum letter is one or two capital letters.".into());
                }
                for other in others() {
                    if let PmiKind::Datum { letter: taken, feature: on } = &other.kind {
                        if taken == letter {
                            return Some(format!("Datum {letter} is already defined."));
                        }
                        if on.len() == feature.len() && on.iter().all(|face| feature.contains(face)) {
                            return Some(format!("These faces already carry datum {taken}."));
                        }
                    }
                }
                None
            }
            PmiKind::Size { size, features, tolerance, through, thread, .. } => {
                if features.is_empty() {
                    return Some("Pick the faces it applies to.".into());
                }
                if *through && *size != SizeKind::Diameter {
                    return Some("Only a hole's diameter goes through.".into());
                }
                if let Some(thread) = thread {
                    if *size != SizeKind::Diameter {
                        return Some("Only a diameter is threaded.".into());
                    }
                    if let Some(problem) = thread_problem(thread) {
                        return Some(problem.into());
                    }
                    if !tolerance.is_none() {
                        return Some("A thread's designation states its fit; it takes no tolerance.".into());
                    }
                }
                range(tolerance)
            }
            PmiKind::Hole { pilot, head, tolerance, through, drill_depth, thread } => {
                if head.as_ref().is_some_and(|head| head.faces.iter().any(|face| pilot.contains(face))) {
                    return Some("The pilot and head must use different faces.".into());
                }
                if *through && drill_depth.is_some() {
                    return Some("A through drill cannot have a blind drill depth.".into());
                }
                if head.is_none() && *through && thread.as_ref().is_none_or(|thread| thread.depth.is_none()) {
                    return Some("Use Diameter for a through hole with no head or usable thread depth.".into());
                }
                if drill_depth.as_ref().is_some_and(|depth| !depth.is_positive()) {
                    return Some("Drill depth must be positive.".into());
                }
                if let Some(thread) = thread {
                    if let Some(problem) = thread_problem(&thread.designation) {
                        return Some(problem.into());
                    }
                    if !tolerance.is_none() {
                        return Some("A thread's designation states its fit; it takes no tolerance.".into());
                    }
                    if let Some(depth) = &thread.depth {
                        if !depth.is_positive() {
                            return Some("Usable thread depth must be positive.".into());
                        }
                        if drill_depth.as_ref().is_some_and(|drill| depth.value() > drill.value()) {
                            return Some("Usable thread depth cannot exceed drill depth.".into());
                        }
                    }
                }
                range(tolerance)
            }
            PmiKind::Location { from, to, tolerance, .. } => {
                let same = from.at == to.at
                    && from.feature.len() == to.feature.len()
                    && from.feature.iter().all(|face| to.feature.contains(face));
                if same {
                    return Some("The distance must run between two different features.".into());
                }
                if [from, to].iter().any(|end| end.at == At::Middle && end.feature.len() != 2) {
                    return Some("The middle is taken between two faces.".into());
                }
                if matches!(tolerance, Tolerance::Fit { .. }) {
                    return Some("A tolerance class belongs to a size, not a distance.".into());
                }
                range(tolerance)
            }
            PmiKind::Slot { tolerance, .. } => {
                if matches!(tolerance, Tolerance::Fit { .. } | Tolerance::Basic) {
                    return Some("A slot takes deviations, such as +0.1/0.".into());
                }
                range(tolerance)
            }
            PmiKind::Width { tolerance, .. } => range(tolerance),
            PmiKind::EdgeBreak { min_mm, max_mm, .. } => {
                let (min, max) = (min_mm.value(), max_mm.value());
                (!(min >= 0.0 && max > min && max.is_finite()))
                    .then(|| "Edge break bounds must be nonnegative and the maximum must exceed the minimum.".into())
            }
            PmiKind::Chamfer { .. } | PmiKind::BasicAngle { .. } => None,
            PmiKind::DatumTarget(target) => {
                if target.number == 0 {
                    return Some("The datum target number must be positive.".into());
                }
                let Some(on) = others().find_map(|other| match &other.kind {
                    PmiKind::Datum { letter, feature } if letter == &target.datum => Some(feature),
                    _ => None,
                }) else {
                    return Some(format!("Datum {} is not defined.", target.datum));
                };
                if !on.contains(&target.face) {
                    return Some("The target face must belong to its datum declaration.".into());
                }
                if others().any(|other| matches!(&other.kind, PmiKind::DatumTarget(t)
                    if t.datum == target.datum && t.number == target.number))
                {
                    return Some(format!("Datum target {}{} is already defined.", target.datum, target.number));
                }
                if !target.point_mm.into_iter().all(f64::is_finite) {
                    return Some("The target point must be finite.".into());
                }
                let positive = match &target.kind {
                    TargetKind::Point => true,
                    TargetKind::Line { length_mm, .. } => length_mm.is_positive(),
                    TargetKind::Area { area: TargetArea::Circular { diameter_mm } } => diameter_mm.is_positive(),
                    TargetKind::Area { area: TargetArea::Rectangular { length_mm, width_mm, .. } } => {
                        length_mm.is_positive() && width_mm.is_positive()
                    }
                };
                (!positive).then(|| "Datum target sizes must be positive.".into())
            }
            PmiKind::PositionControl(control) => {
                if control.features.is_empty() {
                    return Some("Position control needs at least one feature.".into());
                }
                for (i, segment) in control.frame.segments().enumerate() {
                    let problem = if !segment.value_mm.is_positive() {
                        Some("The zone size must be positive.".into())
                    } else if !segment.diameter_zone {
                        Some("Position control requires a cylindrical zone.".into())
                    } else if segment.datums.len() > 3 {
                        Some("At most three datums are allowed.".into())
                    } else if segment.datums.iter().any(|datum| !is_datum_letter(&datum.letter)) {
                        Some("Position segments require single declared datum letters.".into())
                    } else if segment.datums.iter().enumerate().any(|(i, datum)| {
                        segment.datums[..i].iter().any(|other| other.letter == datum.letter)
                    }) {
                        Some("A datum is referred to twice.".into())
                    } else {
                        segment.datums.iter().find_map(|datum| {
                            (!others().any(|other| matches!(&other.kind, PmiKind::Datum { letter, .. }
                                if letter == &datum.letter)))
                            .then(|| format!("Datum {} is not defined.", datum.letter))
                        })
                    };
                    if let Some(problem) = problem {
                        return Some(format!("Position segment {}: {problem}", i + 1));
                    }
                }
                match &control.frame {
                    PositionFrame::Composite { upper, lower } => {
                        if upper.datums.is_empty() {
                            return Some("The upper composite segment needs a datum frame.".into());
                        }
                        if lower.datums.len() > upper.datums.len()
                            || lower.datums.iter().zip(&upper.datums).any(|(low, up)| low.letter != up.letter)
                        {
                            return Some("Lower segment datums must repeat an ordered prefix of the upper frame.".into());
                        }
                        if lower.value_mm.value() > upper.value_mm.value() {
                            return Some("The lower zone cannot exceed the upper zone.".into());
                        }
                    }
                    PositionFrame::Single { projected_zone: Some(zone), .. } => {
                        if !zone.height_mm.is_positive() {
                            return Some("Projected zone height must be positive.".into());
                        }
                        if zone.entry_faces.len() != control.features.len() {
                            return Some("A projected zone needs one entry face per controlled feature.".into());
                        }
                    }
                    PositionFrame::Single { .. } => {}
                }
                None
            }
            PmiKind::Gdt { characteristic, tolerance, datums, features, .. } => {
                if features.is_empty() {
                    return Some("Pick the faces it applies to.".into());
                }
                if !(*tolerance > 0.0 && tolerance.is_finite()) {
                    return Some("The tolerance must be more than zero.".into());
                }
                if datums.len() > 3 {
                    return Some("A tolerance refers to at most three datums.".into());
                }
                if datums.iter().enumerate().any(|(i, datum)| datums[..i].iter().any(|d| d.letter == datum.letter)) {
                    return Some("A datum is referred to twice.".into());
                }
                let defined: Vec<&str> = others()
                    .filter_map(|other| match &other.kind {
                        PmiKind::Datum { letter, .. } => Some(letter.as_str()),
                        _ => None,
                    })
                    .collect();
                if let Some(missing) =
                    datums.iter().flat_map(DatumRef::letters).find(|letter| !defined.contains(letter))
                {
                    return Some(format!("Datum {missing} is not defined."));
                }
                match (characteristic.takes_datums(), datums.len()) {
                    (Some(false), 1..) => Some(format!("{} refers to no datum.", characteristic.name())),
                    (Some(true), 0) => Some(format!("{} needs a datum.", characteristic.name())),
                    _ => None,
                }
            }
            PmiKind::SurfaceFinish { text, .. } => {
                text.trim().is_empty().then(|| "The finish needs a requirement, such as Ra 1.6.".to_owned())
            }
        }
    }

    /// The items that refer to datum `letter`, by name.
    pub fn users_of_datum(&self, letter: &str) -> Vec<&str> {
        self.items
            .iter()
            .filter(|item| item.references_datum(letter))
            .map(|item| item.id.as_str())
            .collect()
    }
}

/// Why `designation` cannot name a thread in a part and a STEP file, or `None` when it
/// can: it is plain text a STEP string holds as it stands, such as `M6x1-6H`.
pub fn thread_problem(designation: &str) -> Option<&'static str> {
    if designation.trim().is_empty() {
        return Some("The thread needs a designation, such as M6x1-6H.");
    }
    let plain = designation.len() <= 40
        && designation.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        && !designation.contains(['\'', '\\']);
    (!plain).then_some("A thread's designation is plain text of at most 40 characters, such as M6x1-6H.")
}

/// A measured length as a drawing writes it: `8`, `7.5`, `0.125`.
pub fn length_text(value: f64) -> String {
    Decimal::from_f64(value).map_or_else(|| "?".to_owned(), |d| d.to_string())
}

impl<F> PmiItem<F> {
    /// Datum declarations this item references, in frame order.
    pub fn referenced_datums(&self) -> impl Iterator<Item = &str> {
        let (target, first, lower): (_, &[DatumRef], &[DatumRef]) = match &self.kind {
            PmiKind::DatumTarget(target) => (Some(target.datum.as_str()), &[], &[]),
            PmiKind::PositionControl(control) => match &control.frame {
                PositionFrame::Single { segment, .. } => (None, &segment.datums, &[]),
                PositionFrame::Composite { upper, lower } => (None, &upper.datums, &lower.datums),
            },
            PmiKind::Gdt { datums, .. } => (None, datums, &[]),
            _ => (None, &[], &[]),
        };
        target.into_iter().chain(first.iter().chain(lower).flat_map(DatumRef::letters))
    }

    /// Whether this item depends on a datum declaration.
    pub fn references_datum(&self, letter: &str) -> bool {
        self.referenced_datums().any(|datum| datum == letter)
    }

    /// The item as a drawing writes it, with its measured values: `Ø8 H7`,
    /// `4× Ø4 +0.1/0 THRU`, `15 ±0.1`, `20×6 +0.1/0 R3`, `Position Ø0.05 A|B|C`,
    /// `Ra 1.6`, `Datum A`. A size or a distance has one value; a slot its length, its
    /// width and its end radius. A value that is missing reads `?`.
    pub fn callout(&self, measured: &[f64]) -> String {
        let value = |index: usize| measured.get(index).map_or_else(|| "?".to_owned(), |v| length_text(*v));
        match &self.kind {
            PmiKind::Datum { letter, .. } => format!("Datum {letter}"),
            PmiKind::Size { features, through, thread: Some(thread), .. } => {
                let count = if features.len() > 1 { format!("{}\u{d7} ", features.len()) } else { String::new() };
                let thru = if *through { " THRU" } else { "" };
                format!("{count}{thread}{thru}")
            }
            PmiKind::Size { size, features, tolerance, through, .. } => {
                let count = if features.len() > 1 { format!("{}\u{d7} ", features.len()) } else { String::new() };
                let sign = match size {
                    SizeKind::Diameter => "\u{d8}",
                    SizeKind::Radius => "R",
                };
                let thru = if *through { " THRU" } else { "" };
                format!("{count}{sign}{}{}{thru}", value(0), tolerance.suffix())
            }
            PmiKind::Hole { .. } => self.hole_callout(&value(0), None),
            PmiKind::Location { tolerance, .. } => format!("{}{}", value(0), tolerance.suffix()),
            PmiKind::Slot { tolerance, .. } => {
                format!("{}\u{d7}{}{} R{}", value(0), value(1), tolerance.suffix(), value(2))
            }
            PmiKind::Width { tolerance, .. } => format!("{}{}", value(0), tolerance.suffix()),
            PmiKind::Chamfer { .. } => format!("{} × {}° CHAMFER", value(0), value(1)),
            PmiKind::EdgeBreak { min_mm, max_mm, .. } => format!("EDGE BREAK {min_mm}–{max_mm}"),
            PmiKind::BasicAngle { .. } => format!("{}° BASIC", value(0)),
            PmiKind::DatumTarget(target) => {
                let extent = match &target.kind {
                    TargetKind::Point => String::new(),
                    TargetKind::Line { length_mm, .. } => format!(" LINE {length_mm}"),
                    TargetKind::Area { area: TargetArea::Circular { diameter_mm } } => format!(" Ø{diameter_mm}"),
                    TargetKind::Area { area: TargetArea::Rectangular { length_mm, width_mm, .. } } => {
                        format!(" {length_mm}×{width_mm}")
                    }
                };
                format!("TARGET {}{}{extent}", target.datum, target.number)
            }
            PmiKind::PositionControl(control) => {
                let segment = |seg: &PositionSegment| {
                    let mut text = format!("Ø{}", seg.value_mm);
                    if !seg.material.is_regardless() {
                        text.push_str(&format!(" ({})", seg.material.letter()));
                    }
                    for datum in &seg.datums {
                        text.push('|');
                        text.push_str(&datum.letter);
                        if !datum.material.is_regardless() {
                            text.push_str(&format!(" ({})", datum.material.letter()));
                        }
                    }
                    text
                };
                match &control.frame {
                    PositionFrame::Single { segment: single, projected_zone } => {
                        let mut text = format!("Position {}", segment(single));
                        if let Some(zone) = projected_zone {
                            text.push_str(&format!(" Ⓟ{}", zone.height_mm));
                        }
                        text
                    }
                    PositionFrame::Composite { upper, lower } => {
                        format!("Position {}\n{}", segment(upper), segment(lower))
                    }
                }
            }
            PmiKind::Gdt { characteristic, tolerance, diameter, material, datums, .. } => {
                let mut text = format!(
                    "{} {}{}",
                    characteristic.name(),
                    if *diameter { "\u{d8}" } else { "" },
                    length_text(*tolerance)
                );
                if !material.is_regardless() {
                    text.push_str(&format!(" ({})", material.letter()));
                }
                if !datums.is_empty() {
                    let letters: Vec<String> = datums
                        .iter()
                        .map(|datum| {
                            if datum.material.is_regardless() {
                                datum.letter.clone()
                            } else {
                                format!("{} ({})", datum.letter, datum.material.letter())
                            }
                        })
                        .collect();
                    text.push(' ');
                    text.push_str(&letters.join("|"));
                }
                text
            }
            PmiKind::SurfaceFinish { text, .. } => text.clone(),
        }
    }

    /// The complete callout for a hole, combining its checked geometry and stated requirements.
    pub fn hole_callout(&self, pilot_diameter: &str, measured: Option<&HoleNominals>) -> String {
        use std::fmt::Write;

        let PmiKind::Hole { head, tolerance, through, drill_depth, thread, .. } = &self.kind else {
            return String::new();
        };
        let mut text = format!("Ø{pilot_diameter}{}", tolerance.suffix());
        if *through {
            text.push_str(" THRU");
        } else if let Some(depth) = drill_depth {
            let _ = write!(text, " DRILL DEEP {depth}");
        } else if let Some(depth) = measured.and_then(|values| values.drill_depth_mm) {
            let _ = write!(text, " DRILL DEEP {}", length_text(depth));
        } else {
            text.push_str(" DRILL DEEP ?");
        }
        match (head.as_ref().map(|head| head.kind), measured.and_then(|values| values.head)) {
            (Some(HoleHeadKind::Counterbore), Some(HoleHeadNominals::Counterbore { diameter_mm, depth_mm })) => {
                let _ = write!(text, "\nC'BORE Ø{} DEEP {}", length_text(diameter_mm), length_text(depth_mm));
            }
            (
                Some(HoleHeadKind::Countersink),
                Some(HoleHeadNominals::Countersink { diameter_mm, included_angle_deg }),
            ) => {
                let _ = write!(text, "\nC'SINK Ø{} × {}°", length_text(diameter_mm), length_text(included_angle_deg));
            }
            (Some(HoleHeadKind::Counterbore), _) => text.push_str("\nC'BORE Ø? DEEP ?"),
            (Some(HoleHeadKind::Countersink), _) => text.push_str("\nC'SINK Ø? × ?°"),
            (None, _) => {}
        }
        if let Some(thread) = thread {
            let _ = write!(text, "\n{}", thread.designation);
            if let Some(depth) = &thread.depth {
                let _ = write!(text, " THREAD DEEP {depth}");
            }
        }
        text
    }

    /// Why a STEP file cannot hold the item as it stands, or `None` when it can. The
    /// item still stands on the part and on its drawing.
    pub fn file_problem(&self) -> Option<String> {
        match &self.kind {
            PmiKind::Hole { .. } => Some("Compound hole and depth PMI is not supported in STEP export.".to_owned()),
            PmiKind::Width { .. }
            | PmiKind::Chamfer { .. }
            | PmiKind::EdgeBreak { .. }
            | PmiKind::BasicAngle { .. }
            | PmiKind::DatumTarget(_)
            | PmiKind::PositionControl(_) => Some(format!("{} PMI is not supported in STEP export.", self.kind_label())),
            PmiKind::Gdt { datums, .. }
                if datums.iter().any(|datum| !datum.material.is_regardless() || datum.letter.contains('-')) =>
            {
                Some("Coming soon".to_owned())
            }
            PmiKind::SurfaceFinish { method, lay, allowance, all_around, .. }
                if !method.is_empty() || lay.is_some() || !allowance.is_empty() || *all_around =>
            {
                Some("Coming soon".to_owned())
            }
            PmiKind::SurfaceFinish { text, .. } if roughness(text).is_none() => {
                Some("The finish must read as a parameter and a value, such as Ra 1.6.".to_owned())
            }
            _ => None,
        }
    }

    /// What kind of item it is, in words: `Datum`, `Diameter`, `Radius`, `Distance`,
    /// `Slot`, the characteristic's name, or `Surface finish`.
    pub fn kind_label(&self) -> &'static str {
        match &self.kind {
            PmiKind::Datum { .. } => "Datum",
            PmiKind::DatumTarget(_) => "Datum target",
            PmiKind::PositionControl(_) => "Position control",
            PmiKind::Size { size: SizeKind::Diameter, .. } => "Diameter",
            PmiKind::Size { size: SizeKind::Radius, .. } => "Radius",
            PmiKind::Hole { .. } => "Hole",
            PmiKind::Location { .. } => "Distance",
            PmiKind::Slot { .. } => "Slot",
            PmiKind::Width { .. } => "Width",
            PmiKind::Chamfer { .. } => "Chamfer",
            PmiKind::EdgeBreak { .. } => "Edge break",
            PmiKind::BasicAngle { .. } => "Basic angle",
            PmiKind::Gdt { characteristic, .. } => characteristic.name(),
            PmiKind::SurfaceFinish { .. } => "Surface finish",
        }
    }
}

/// The manufacturing information read from a STEP file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StepPmi {
    /// What was read. Faces are named by the keys the file's import gives them.
    pub pmi: Pmi<String>,
    /// Items in the file that were not read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedPmi>,
}

/// An item of a STEP file's manufacturing information that was not read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedPmi {
    /// The item's place in the file, such as `#412`.
    pub entity: String,
    /// What it is, in the file's words.
    pub kind: String,
    /// Why it was left out.
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_read_as_values() {
        for (text, canonical) in
            [("0.10", "0.1"), ("+0.1", "0.1"), (".1", "0.1"), ("-0.0", "0"), ("8", "8"), ("008.50", "8.5")]
        {
            assert_eq!(Decimal::parse(text).unwrap().as_str(), canonical, "{text}");
        }
        for text in ["", ".", "1e3", "abc", "1.2.3", "--1"] {
            assert!(Decimal::parse(text).is_none(), "{text}");
        }
        assert_eq!(Decimal::from_f64(7.999_999_999_9).unwrap().as_str(), "8");
        assert_eq!(Decimal::from_f64(0.05).unwrap().as_str(), "0.05");
    }

    #[test]
    fn tolerances_read_as_a_drawing_writes_them() {
        let class = FitClass::parse("js6").unwrap();
        assert_eq!((class.as_str(), class.deviation(), class.grade()), ("js6", "js", "6"));
        assert!(FitClass::parse("7H").is_none() && FitClass::parse("H").is_none());
        assert_eq!(Tolerance::parse("H7").unwrap().suffix(), " H7");
        assert_eq!(Tolerance::Symmetric { value: 0.1 }.suffix(), " \u{b1}0.1");
        assert_eq!(Tolerance::Deviation { upper: 0.1, lower: 0.0 }.suffix(), " +0.1/0");
        for text in ["H7", "\u{b1}0.1", "+-0.1", "+0.1/0", "+0.2/-0.1", "0/-0.1"] {
            let tolerance = Tolerance::parse(text).unwrap_or_else(|| panic!("{text} reads as a tolerance"));
            assert_eq!(Tolerance::parse(tolerance.suffix().trim()), Some(tolerance), "{text}");
        }
        assert_eq!(Tolerance::parse("basic"), Some(Tolerance::Basic));
        for text in ["", "0.1", "-0.1/+0.1", "\u{b1}0", "H"] {
            assert!(Tolerance::parse(text).is_none(), "{text}");
        }
        assert_eq!(roughness("Ra 1.6").map(|(name, value)| (name, value.to_string())), Some(("Ra", "1.6".to_owned())));
        assert!(roughness("smooth").is_none() && roughness("Ra 1.6 max").is_none() && roughness("1.6 Ra").is_none());
    }

    #[test]
    fn an_item_keeps_its_shape_in_a_file() {
        let item = PmiItem {
            id: "Bore diameter".to_owned(),
            kind: PmiKind::Size {
                size: SizeKind::Diameter,
                features: vec![vec!["#17".to_owned()]],
                tolerance: Tolerance::parse("H7").unwrap(),
                through: false,
                thread: None,
                nominal: None,
            },
        };
        let text = serde_json::to_string(&item).unwrap();
        assert_eq!(
            text,
            r##"{"id":"Bore diameter","type":"size","size":"diameter","features":[["#17"]],"tolerance":{"type":"fit","class":"H7"}}"##
        );
        assert_eq!(serde_json::from_str::<PmiItem<String>>(&text).unwrap(), item);
    }

    #[test]
    fn items_read_as_a_drawing_writes_them() {
        let face = |name: &str| vec![name.to_owned()];
        let item = |kind| PmiItem { id: "x".to_owned(), kind };
        let holes = item(PmiKind::Size {
            size: SizeKind::Diameter,
            features: vec![face("a"), face("b"), face("c"), face("d")],
            tolerance: Tolerance::Deviation { upper: 0.1, lower: 0.0 },
            through: true,
            thread: None,
            nominal: None,
        });
        assert_eq!(holes.callout(&[4.0]), "4\u{d7} \u{d8}4 +0.1/0 THRU");
        let tapped = item(PmiKind::Size {
            size: SizeKind::Diameter,
            features: vec![face("a"), face("b")],
            tolerance: Tolerance::None,
            through: true,
            thread: Some("M6x1-6H".to_owned()),
            nominal: None,
        });
        assert_eq!(tapped.callout(&[5.0]), "2\u{d7} M6x1-6H THRU");
        let part = Pmi { items: Vec::new(), part: PartNotes::default() };
        assert_eq!(part.problem_with(&tapped, None), None);
        let fitted = |kind: &PmiKind<String>| match kind {
            PmiKind::Size { size, features, through, thread, nominal, .. } => PmiKind::Size {
                size: *size,
                features: features.clone(),
                tolerance: Tolerance::parse("H7").unwrap(),
                through: *through,
                thread: thread.clone(),
                nominal: *nominal,
            },
            other => other.clone(),
        };
        assert_eq!(
            part.problem_with(&item(fitted(&tapped.kind)), None).as_deref(),
            Some("A thread's designation states its fit; it takes no tolerance.")
        );
        assert!(thread_problem("M6x1 \u{2013} 6H").is_some() && thread_problem(" ").is_some());
        let slot = item(PmiKind::Slot {
            flats: [face("a"), face("b")],
            ends: [face("c"), face("d")],
            tolerance: Tolerance::Deviation { upper: 0.1, lower: 0.0 },
        });
        assert_eq!(slot.callout(&[20.0, 6.0, 3.0]), "20\u{d7}6 +0.1/0 R3");
        assert_eq!(slot.callout(&[]), "?\u{d7}? +0.1/0 R?");
        let position = item(PmiKind::Gdt {
            characteristic: GdtCharacteristic::Position,
            tolerance: 0.05,
            diameter: true,
            material: MaterialCondition::Regardless,
            datums: ["A", "B", "C"].map(DatumRef::new).to_vec(),
            features: vec![face("a")],
        });
        assert_eq!(position.callout(&[]), "Position \u{d8}0.05 A|B|C");
    }

    #[test]
    fn an_item_that_does_not_fit_says_why() {
        let datum = |letter: &str, face: &str| PmiItem {
            id: format!("Datum {letter}"),
            kind: PmiKind::Datum { letter: letter.to_owned(), feature: vec![face.to_owned()] },
        };
        let part = Pmi { items: vec![datum("A", "a")], part: PartNotes::default() };
        assert_eq!(
            part.problem_with(&datum("A", "b"), None).as_deref(),
            Some("Another item is already called Datum A.")
        );
        let flatness = |datums: &[&str]| PmiItem {
            id: "Flatness".to_owned(),
            kind: PmiKind::Gdt {
                characteristic: GdtCharacteristic::Flatness,
                tolerance: 0.05,
                diameter: false,
                material: MaterialCondition::Regardless,
                datums: datums.iter().map(|letter| DatumRef::new(letter)).collect(),
                features: vec![vec!["a".to_owned()]],
            },
        };
        assert_eq!(part.problem_with(&flatness(&[]), None), None);
        assert_eq!(part.problem_with(&flatness(&["A"]), None).as_deref(), Some("Flatness refers to no datum."));
        assert_eq!(part.problem_with(&flatness(&["B"]), None).as_deref(), Some("Datum B is not defined."));
        assert_eq!(part.users_of_datum("A"), Vec::<&str>::new());
    }
}
