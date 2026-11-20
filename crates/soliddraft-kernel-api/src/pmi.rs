//! Manufacturing information on a part: datums, dimensions with their limits, geometric
//! tolerances and surface textures on faces, and the notes of the whole part.
//!
//! A [`Pmi`] is generic over what names a face. The part document names faces by its
//! persistent references, a kernel call by [`FaceId`](crate::FaceId), and a STEP file by
//! the face keys its import reports.
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
        self.try_map_features(|faces| faces.iter().map(&mut map).collect())
    }

    /// The same item with every feature's faces named another way, where one name may
    /// stand for several faces, such as the pieces of a face a later cut split.
    pub fn try_map_features<G, E>(
        &self,
        mut feature: impl FnMut(&Feature<F>) -> Result<Feature<G>, E>,
    ) -> Result<PmiItem<G>, E> {
        let kind = match &self.kind {
            PmiKind::Datum { letter, feature: faces } => {
                PmiKind::Datum { letter: letter.clone(), feature: feature(faces)? }
            }
            PmiKind::Size { size, features, limits, nominal } => PmiKind::Size {
                size: *size,
                features: features.iter().map(&mut feature).collect::<Result<_, _>>()?,
                limits: limits.clone(),
                nominal: nominal.clone(),
            },
            PmiKind::Location { from, to, limits, nominal } => PmiKind::Location {
                from: Anchor { feature: feature(&from.feature)?, at: from.at },
                to: Anchor { feature: feature(&to.feature)?, at: to.at },
                limits: limits.clone(),
                nominal: nominal.clone(),
            },
            PmiKind::Tolerance { characteristic, value, diameter_zone, material, datums, features } => {
                PmiKind::Tolerance {
                    characteristic: *characteristic,
                    value: value.clone(),
                    diameter_zone: *diameter_zone,
                    material: *material,
                    datums: datums.clone(),
                    features: features.iter().map(&mut feature).collect::<Result<_, _>>()?,
                }
            }
            PmiKind::Texture { parameter, value, process, feature: faces } => PmiKind::Texture {
                parameter: parameter.clone(),
                value: value.clone(),
                process: *process,
                feature: feature(faces)?,
            },
        };
        Ok(PmiItem { id: self.id.clone(), kind })
    }

    /// Every feature the item names, in order.
    pub fn features(&self) -> Vec<&Feature<F>> {
        match &self.kind {
            PmiKind::Datum { feature, .. } | PmiKind::Texture { feature, .. } => vec![feature],
            PmiKind::Size { features, .. } | PmiKind::Tolerance { features, .. } => features.iter().collect(),
            PmiKind::Location { from, to, .. } => vec![&from.feature, &to.feature],
        }
    }

    /// True for a size or a distance: an item with a nominal value.
    pub fn is_dimension(&self) -> bool {
        matches!(self.kind, PmiKind::Size { .. } | PmiKind::Location { .. })
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
        /// The limits, or `None` for a size under the part's general tolerance.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limits: Option<Limits>,
        /// The value a STEP file stated, in millimetres. `None` in a part: the size is
        /// measured on the body.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nominal: Option<Decimal>,
    },
    /// The distance between two features.
    Location {
        /// The feature it is measured from.
        from: Anchor<F>,
        /// The feature it is measured to.
        to: Anchor<F>,
        /// The limits, or `None` for a distance under the general tolerance.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limits: Option<Limits>,
        /// The value a STEP file stated, in millimetres. `None` in a part.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nominal: Option<Decimal>,
    },
    /// A geometric tolerance.
    Tolerance {
        /// The characteristic.
        characteristic: Characteristic,
        /// The width of the tolerance zone in millimetres.
        value: Decimal,
        /// The zone is a cylinder and the value its diameter.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        diameter_zone: bool,
        /// The material condition the tolerance applies at.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        material: Option<MaterialCondition>,
        /// The datum letters it refers to: primary, secondary, tertiary.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        datums: Vec<String>,
        /// The features it applies to. Several are a pattern.
        features: Vec<Feature<F>>,
    },
    /// A surface texture requirement.
    Texture {
        /// The roughness parameter, such as `Ra`.
        parameter: String,
        /// Its limit in micrometres.
        value: Decimal,
        /// What the process may do to the surface.
        #[serde(default)]
        process: Process,
        /// The feature it applies to.
        feature: Feature<F>,
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

/// The limits of a dimension.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "limits", rename_all = "snake_case")]
pub enum Limits {
    /// Deviations from the nominal value in millimetres, such as +0.1 and 0.
    Deviations {
        /// The upper deviation.
        upper: Decimal,
        /// The lower deviation, below the upper one.
        lower: Decimal,
    },
    /// An ISO 286 tolerance class, such as H7.
    Fit {
        /// The fundamental deviation: capital letters for a hole, small for a shaft.
        deviation: String,
        /// The tolerance grade number, such as `7`.
        grade: String,
    },
}

impl Limits {
    /// The limits as a drawing writes them after the nominal value: ` H7`, ` ±0.1`, or
    /// ` +0.1/0`.
    pub fn suffix(&self) -> String {
        match self {
            Limits::Fit { deviation, grade } => format!(" {deviation}{grade}"),
            Limits::Deviations { upper, lower } => {
                let signed = |value: &Decimal| {
                    if value.is_positive() { format!("+{value}") } else { value.to_string() }
                };
                if upper.is_positive() && lower.as_str().strip_prefix('-') == Some(upper.as_str()) {
                    format!(" \u{b1}{upper}")
                } else {
                    format!(" {}/{}", signed(upper), signed(lower))
                }
            }
        }
    }

    /// Reads a tolerance class such as `H7` or `js6`.
    pub fn fit(class: &str) -> Option<Limits> {
        let class = class.trim();
        let split = class.find(|c: char| c.is_ascii_digit())?;
        let (deviation, grade) = class.split_at(split);
        let letters = (1..=2).contains(&deviation.len())
            && (deviation.bytes().all(|b| b.is_ascii_uppercase()) || deviation.bytes().all(|b| b.is_ascii_lowercase()));
        let digits = (1..=2).contains(&grade.len()) && grade.bytes().all(|b| b.is_ascii_digit());
        (letters && digits).then(|| Limits::Fit { deviation: deviation.to_owned(), grade: grade.to_owned() })
    }
}

/// A geometric characteristic, ISO 1101.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Characteristic {
    /// Angularity.
    Angularity,
    /// Circular run-out.
    CircularRunout,
    /// Coaxiality.
    Coaxiality,
    /// Concentricity.
    Concentricity,
    /// Cylindricity.
    Cylindricity,
    /// Flatness.
    Flatness,
    /// Profile of a line.
    LineProfile,
    /// Parallelism.
    Parallelism,
    /// Perpendicularity.
    Perpendicularity,
    /// Position.
    Position,
    /// Roundness.
    Roundness,
    /// Straightness.
    Straightness,
    /// Profile of a surface.
    SurfaceProfile,
    /// Symmetry.
    Symmetry,
    /// Total run-out.
    TotalRunout,
}

/// Whether a characteristic refers to datums.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatumUse {
    /// Never: a form tolerance.
    None,
    /// With or without.
    Optional,
    /// Always, one to three.
    Required,
}

impl Characteristic {
    /// Every characteristic, in the order menus list them.
    pub const ALL: [Characteristic; 15] = [
        Characteristic::Position,
        Characteristic::Flatness,
        Characteristic::Perpendicularity,
        Characteristic::Parallelism,
        Characteristic::Angularity,
        Characteristic::Straightness,
        Characteristic::Roundness,
        Characteristic::Cylindricity,
        Characteristic::Concentricity,
        Characteristic::Coaxiality,
        Characteristic::Symmetry,
        Characteristic::CircularRunout,
        Characteristic::TotalRunout,
        Characteristic::LineProfile,
        Characteristic::SurfaceProfile,
    ];

    /// The characteristic in words, such as `Position`.
    pub fn label(self) -> &'static str {
        match self {
            Characteristic::Angularity => "Angularity",
            Characteristic::CircularRunout => "Circular run-out",
            Characteristic::Coaxiality => "Coaxiality",
            Characteristic::Concentricity => "Concentricity",
            Characteristic::Cylindricity => "Cylindricity",
            Characteristic::Flatness => "Flatness",
            Characteristic::LineProfile => "Profile of a line",
            Characteristic::Parallelism => "Parallelism",
            Characteristic::Perpendicularity => "Perpendicularity",
            Characteristic::Position => "Position",
            Characteristic::Roundness => "Roundness",
            Characteristic::Straightness => "Straightness",
            Characteristic::SurfaceProfile => "Profile of a surface",
            Characteristic::Symmetry => "Symmetry",
            Characteristic::TotalRunout => "Total run-out",
        }
    }

    /// Whether the characteristic refers to datums.
    pub fn datum_use(self) -> DatumUse {
        match self {
            Characteristic::Cylindricity
            | Characteristic::Flatness
            | Characteristic::Roundness
            | Characteristic::Straightness => DatumUse::None,
            Characteristic::LineProfile | Characteristic::Position | Characteristic::SurfaceProfile => {
                DatumUse::Optional
            }
            Characteristic::Angularity
            | Characteristic::CircularRunout
            | Characteristic::Coaxiality
            | Characteristic::Concentricity
            | Characteristic::Parallelism
            | Characteristic::Perpendicularity
            | Characteristic::Symmetry
            | Characteristic::TotalRunout => DatumUse::Required,
        }
    }
}

/// The material condition a geometric tolerance applies at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialCondition {
    /// Maximum material requirement.
    Maximum,
    /// Least material requirement.
    Least,
}

/// What a surface texture requirement allows the process to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Process {
    /// Any process.
    #[default]
    Any,
    /// Material must be removed.
    Removal,
    /// Material must not be removed.
    NoRemoval,
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
        let range = |limits: &Option<Limits>| match limits {
            Some(Limits::Deviations { upper, lower }) if upper.value() <= lower.value() => {
                Some("The upper deviation must be above the lower one.".to_owned())
            }
            Some(Limits::Fit { deviation, grade }) if Limits::fit(&format!("{deviation}{grade}")).is_none() => {
                Some("A tolerance class is one or two letters and a grade, such as H7.".to_owned())
            }
            _ => None,
        };
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
            PmiKind::Size { features, limits, .. } => {
                if features.is_empty() {
                    return Some("Pick the faces it applies to.".into());
                }
                range(limits)
            }
            PmiKind::Location { from, to, limits, .. } => {
                let same = from.at == to.at
                    && from.feature.len() == to.feature.len()
                    && from.feature.iter().all(|face| to.feature.contains(face));
                if same {
                    return Some("The distance must run between two different features.".into());
                }
                if [from, to].iter().any(|end| end.at == At::Middle && end.feature.len() != 2) {
                    return Some("The middle is taken between two faces.".into());
                }
                range(limits)
            }
            PmiKind::Tolerance { characteristic, value, datums, features, .. } => {
                if features.is_empty() {
                    return Some("Pick the faces it applies to.".into());
                }
                if !value.is_positive() {
                    return Some("The tolerance must be more than zero.".into());
                }
                if datums.len() > 3 {
                    return Some("A tolerance refers to at most three datums.".into());
                }
                if datums.iter().enumerate().any(|(i, letter)| datums[..i].contains(letter)) {
                    return Some("A datum is referred to twice.".into());
                }
                let defined: Vec<&str> = others()
                    .filter_map(|other| match &other.kind {
                        PmiKind::Datum { letter, .. } => Some(letter.as_str()),
                        _ => None,
                    })
                    .collect();
                if let Some(missing) = datums.iter().find(|letter| !defined.contains(&letter.as_str())) {
                    return Some(format!("Datum {missing} is not defined."));
                }
                match (characteristic.datum_use(), datums.len()) {
                    (DatumUse::None, 1..) => Some(format!("{} refers to no datum.", characteristic.label())),
                    (DatumUse::Required, 0) => Some(format!("{} needs a datum.", characteristic.label())),
                    _ => None,
                }
            }
            PmiKind::Texture { parameter, value, .. } => {
                if parameter.is_empty() || !parameter.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    return Some("The roughness parameter is letters and digits, such as Ra.".into());
                }
                (!value.is_positive()).then(|| "The roughness value must be more than zero.".to_owned())
            }
        }
    }

    /// The items that refer to datum `letter`, by name.
    pub fn users_of_datum(&self, letter: &str) -> Vec<&str> {
        self.items
            .iter()
            .filter(
                |item| matches!(&item.kind, PmiKind::Tolerance { datums, .. } if datums.iter().any(|d| d == letter)),
            )
            .map(|item| item.id.as_str())
            .collect()
    }
}

/// A measured length as a drawing writes it: `8`, `7.5`, `0.125`.
pub fn length_text(value: f64) -> String {
    Decimal::from_f64(value).map_or_else(|| "?".to_owned(), |d| d.to_string())
}

impl<F> PmiItem<F> {
    /// The item as a drawing writes it, with a dimension's measured value: `Ø8 H7`,
    /// `4× Ø4 +0.1/0`, `15 ±0.1`, `Position Ø0.05 A|B|C`, `Ra 1.6`, `Datum A`. A
    /// dimension without a measured value reads `?`.
    pub fn callout(&self, nominal: Option<f64>) -> String {
        let measured = nominal.map_or_else(|| "?".to_owned(), length_text);
        let suffix = |limits: &Option<Limits>| limits.as_ref().map(Limits::suffix).unwrap_or_default();
        match &self.kind {
            PmiKind::Datum { letter, .. } => format!("Datum {letter}"),
            PmiKind::Size { size, features, limits, .. } => {
                let count = if features.len() > 1 { format!("{}\u{d7} ", features.len()) } else { String::new() };
                let sign = match size {
                    SizeKind::Diameter => "\u{d8}",
                    SizeKind::Radius => "R",
                };
                format!("{count}{sign}{measured}{}", suffix(limits))
            }
            PmiKind::Location { limits, .. } => format!("{measured}{}", suffix(limits)),
            PmiKind::Tolerance { characteristic, value, diameter_zone, material, datums, .. } => {
                let mut text =
                    format!("{} {}{value}", characteristic.label(), if *diameter_zone { "\u{d8}" } else { "" });
                match material {
                    Some(MaterialCondition::Maximum) => text.push_str(" (M)"),
                    Some(MaterialCondition::Least) => text.push_str(" (L)"),
                    None => {}
                }
                if !datums.is_empty() {
                    text.push(' ');
                    text.push_str(&datums.join("|"));
                }
                text
            }
            PmiKind::Texture { parameter, value, .. } => format!("{parameter} {value}"),
        }
    }

    /// What kind of item it is, in words: `Datum`, `Diameter`, `Radius`, `Distance`,
    /// the characteristic's name, or `Surface finish`.
    pub fn kind_label(&self) -> &'static str {
        match &self.kind {
            PmiKind::Datum { .. } => "Datum",
            PmiKind::Size { size: SizeKind::Diameter, .. } => "Diameter",
            PmiKind::Size { size: SizeKind::Radius, .. } => "Radius",
            PmiKind::Location { .. } => "Distance",
            PmiKind::Tolerance { characteristic, .. } => characteristic.label(),
            PmiKind::Texture { .. } => "Surface finish",
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
    fn limits_read_as_a_drawing_writes_them() {
        let d = |text| Decimal::parse(text).unwrap();
        assert_eq!(Limits::fit("H7").unwrap().suffix(), " H7");
        assert_eq!(Limits::fit("js6").unwrap().suffix(), " js6");
        assert!(Limits::fit("7H").is_none());
        assert_eq!(Limits::Deviations { upper: d("0.1"), lower: d("-0.1") }.suffix(), " \u{b1}0.1");
        assert_eq!(Limits::Deviations { upper: d("0.1"), lower: d("0") }.suffix(), " +0.1/0");
    }

    #[test]
    fn an_item_keeps_its_shape_in_a_file() {
        let item = PmiItem {
            id: "Bore diameter".to_owned(),
            kind: PmiKind::Size {
                size: SizeKind::Diameter,
                features: vec![vec!["#17".to_owned()]],
                limits: Limits::fit("H7"),
                nominal: None,
            },
        };
        let text = serde_json::to_string(&item).unwrap();
        assert_eq!(
            text,
            r##"{"id":"Bore diameter","type":"size","size":"diameter","features":[["#17"]],"limits":{"limits":"fit","deviation":"H","grade":"7"}}"##
        );
        assert_eq!(serde_json::from_str::<PmiItem<String>>(&text).unwrap(), item);
    }
}
