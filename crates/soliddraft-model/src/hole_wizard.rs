//! Hole Wizard standards, fastener dimension tables, end conditions, options,
//! callout tolerances, and favorites.
//!
//! Every built-in dimension table is grounded in published engineering standards:
//!
//! - **ISO**: ISO 273 (clearance holes), ISO 4762 & DIN 974-1 (SHCS counterbores),
//!   ISO 4014 (hex bolts), ISO 10642 & DIN 74 Form F (90° countersunk SHCS),
//!   ISO 262 & ISO 2306 (metric threads & tap drills), ISO 7-1 (BSPT pipe threads).
//! - **ANSI Metric**: ASME B18.2.8 (metric clearance holes), ASME B18.3.1M (metric SHCS),
//!   ASME B18.2.3.5M (metric hex bolts), ASME B18.3.5M (metric flat countersunk SHCS),
//!   ASME B1.13M (metric threads), ASME B1.20.1 (NPT pipe threads).
//! - **ANSI Inch**: ASME B18.2.8 (inch clearance holes), ASME B18.3 (inch SHCS & 82°
//!   flat countersunk SHCS), ASME B18.2.1 (inch hex cap screws), ASME B1.1 (UNC & UNF
//!   threads at ~75% engagement), ASME B1.20.1 (NPT tapered pipe threads).
//! - **DIN**: DIN EN 20273 (clearance holes), DIN 912 & DIN 974-1 (SHCS counterbores),
//!   DIN 931 (hex bolts), DIN 7991 & DIN 74 (90° countersunk screws), DIN 13-1 & DIN 336
//!   (metric threads & tap drills), DIN 2999 / ISO 7-1 (Whitworth pipe threads).
//! - **JIS**: JIS B 1001 (clearance holes Class 1/2/3), JIS B 1176 (SHCS counterbores),
//!   JIS B 1180 (hex bolts), JIS B 1194 (90° countersunk SHCS), JIS B 0205 (metric threads),
//!   JIS B 0203 (PT tapered pipe threads).
//! - **AS**: AS 1420 (clearance holes & SHCS counterbores), AS 1110.1 (hex bolts),
//!   AS 1427 (countersunk SHCS), AS 1275 (metric threads), AS 1722.1 (tapered pipe threads).
//! - **BSI**: BS 4186 (clearance holes), BS 4168-1 (SHCS counterbores), BS 3692 (hex bolts),
//!   BS 4168-3 (countersunk SHCS), BS 3643 (metric threads), BS 21 (BSPT pipe threads).
//! - **GB**: GB/T 5277 (clearance holes), GB/T 70.1 & GB/T 152.2 (SHCS counterbores),
//!   GB/T 5782 (hex bolts), GB/T 70.3 & GB/T 152.3 (countersunk SHCS), GB/T 196 (metric
//!   threads), GB/T 7306.1 (55° tapered pipe threads).
//! - **KS**: KS B ISO 273 (clearance holes), KS B 1003 (SHCS counterbores), KS B 1002
//!   (hex bolts), KS B 1046 (countersunk SHCS), KS B 0201 (metric threads), KS B 0222 (pipe).
//! - **IS**: IS 1821 (clearance holes), IS 2269 & IS 3406 (SHCS counterbores), IS 1364-1
//!   (hex bolts), IS 6761 (countersunk SHCS), IS 4218 (metric threads), IS 554 (pipe).
//!
//! All dimensions in memory and serialized files are stored in millimetres and degrees.

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::{HoleDepth, HoleHead, HoleTip};

use crate::feature::FeatureId;
use crate::reference::{FaceRef, VertexRef};
use crate::thread::HoleThread;

/// Millimetres per inch (exact international inch).
pub const MM_PER_INCH: f64 = 25.4;

/// Engineering standard supplying Hole Wizard fastener and thread tables.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoleStandard {
    /// International Organization for Standardization (ISO 273 / 4762 / 10642 / 262 / 7-1).
    #[default]
    Iso,
    /// ANSI / ASME Metric (ASME B18.2.8 / B18.3.1M / B18.2.3.5M / B1.13M).
    AnsiMetric,
    /// ANSI / ASME Inch (ASME B18.2.8 / B18.3 / B18.2.1 / B1.1 / B1.20.1).
    AnsiInch,
    /// Deutsches Institut für Normung (DIN EN 20273 / DIN 974-1 / DIN 74 / DIN 13-1).
    Din,
    /// Japanese Industrial Standards (JIS B 1001 / B 1176 / B 1194 / B 0205 / B 0203).
    Jis,
    /// Australian Standards (AS 1420 / AS 1110.1 / AS 1427 / AS 1275 / AS 1722.1).
    As,
    /// British Standards Institution (BS 4186 / BS 4168 / BS 3692 / BS 3643 / BS 21).
    Bsi,
    /// Chinese National Standards (GB/T 5277 / GB/T 152.2 / GB/T 70.3 / GB/T 196 / GB/T 7306.1).
    Gb,
    /// Korean Industrial Standards (KS B ISO 273 / KS B 1003 / KS B 1002 / KS B 0201 / KS B 0222).
    Ks,
    /// Bureau of Indian Standards (IS 1821 / IS 2269 / IS 3406 / IS 6761 / IS 4218 / IS 554).
    Is,
}

impl HoleStandard {
    /// Every standard in display order.
    pub const ALL: [Self; 10] = [
        Self::Iso,
        Self::AnsiMetric,
        Self::AnsiInch,
        Self::Din,
        Self::Jis,
        Self::As,
        Self::Bsi,
        Self::Gb,
        Self::Ks,
        Self::Is,
    ];

    /// Display name shown in the Standard combo box.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Iso => "ISO",
            Self::AnsiMetric => "ANSI Metric",
            Self::AnsiInch => "ANSI Inch",
            Self::Din => "DIN",
            Self::Jis => "JIS",
            Self::As => "AS",
            Self::Bsi => "BSI",
            Self::Gb => "GB",
            Self::Ks => "KS",
            Self::Is => "IS",
        }
    }

    /// True when the standard's native nominal sizes are expressed in inches.
    pub const fn is_inch(self) -> bool {
        matches!(self, Self::AnsiInch)
    }

    /// Authoritative published standards cited for this standard's tables.
    pub const fn source_summary(self) -> &'static str {
        match self {
            Self::Iso => "ISO 273, ISO 4762, DIN 974-1, ISO 4014, ISO 10642, DIN 74, ISO 262, ISO 2306, ISO 7-1",
            Self::AnsiMetric => "ASME B18.2.8, ASME B18.3.1M, ASME B18.2.3.5M, ASME B18.3.5M, ASME B1.13M, ASME B1.20.1",
            Self::AnsiInch => "ASME B18.2.8, ASME B18.3, ASME B18.2.1, ASME B1.1, ASME B1.20.1",
            Self::Din => "DIN EN 20273, DIN 912, DIN 974-1, DIN 931, DIN 7991, DIN 74, DIN 13-1, DIN 336, DIN 2999",
            Self::Jis => "JIS B 1001, JIS B 1176, JIS B 1180, JIS B 1194, JIS B 0205, JIS B 0203",
            Self::As => "AS 1420, AS 1421, AS 1110.1, AS 1427, AS 1275, AS 1722.1",
            Self::Bsi => "BS 4186, BS 4168-1, BS 3692, BS 4168-3, BS 3643, BS 21",
            Self::Gb => "GB/T 5277, GB/T 70.1, GB/T 152.2, GB/T 5782, GB/T 70.3, GB/T 152.3, GB/T 196, GB/T 7306.1",
            Self::Ks => "KS B ISO 273, KS B 1003, KS B 1002, KS B 1046, KS B 0201, KS B 0222",
            Self::Is => "IS 1821, IS 2269, IS 3406, IS 1364-1, IS 6761, IS 4218, IS 554",
        }
    }
}

/// The 8 Hole Wizard hole types.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoleType {
    /// Counterbored circular hole.
    #[default]
    Counterbore,
    /// Countersunk circular hole.
    Countersink,
    /// Simple (clearance or drill) circular hole.
    Simple,
    /// Straight tapped hole with cosmetic thread.
    StraightTap,
    /// Tapered (pipe) tapped hole.
    TaperedTap,
    /// Counterbored oblong slot.
    CounterboreSlot,
    /// Countersunk oblong slot.
    CountersinkSlot,
    /// Simple oblong slot.
    Slot,
}

impl HoleType {
    /// Every hole type in UI grid order.
    pub const ALL: [Self; 8] = [
        Self::Counterbore,
        Self::Countersink,
        Self::Simple,
        Self::StraightTap,
        Self::TaperedTap,
        Self::CounterboreSlot,
        Self::CountersinkSlot,
        Self::Slot,
    ];

    /// Display name.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Counterbore => "Counterbore",
            Self::Countersink => "Countersink",
            Self::Simple => "Hole",
            Self::StraightTap => "Straight Tap",
            Self::TaperedTap => "Tapered Tap",
            Self::CounterboreSlot => "Counterbore Slot",
            Self::CountersinkSlot => "Countersink Slot",
            Self::Slot => "Slot",
        }
    }

    /// Short caption for compact icon tiles.
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Counterbore => "CBore",
            Self::Countersink => "CSink",
            Self::Simple => "Hole",
            Self::StraightTap => "Tap",
            Self::TaperedTap => "Pipe",
            Self::CounterboreSlot => "CB Slot",
            Self::CountersinkSlot => "CS Slot",
            Self::Slot => "Slot",
        }
    }

    /// True for slot variants.
    pub const fn is_slot(self) -> bool {
        matches!(self, Self::CounterboreSlot | Self::CountersinkSlot | Self::Slot)
    }

    /// True for tapped hole variants.
    pub const fn is_tapped(self) -> bool {
        matches!(self, Self::StraightTap | Self::TaperedTap)
    }

    /// True when the type has a counterbore head.
    pub const fn has_counterbore(self) -> bool {
        matches!(self, Self::Counterbore | Self::CounterboreSlot)
    }

    /// True when the type has a countersink head.
    pub const fn has_countersink(self) -> bool {
        matches!(self, Self::Countersink | Self::CountersinkSlot)
    }

    /// True when the Fit (Close / Normal / Loose) selector applies.
    pub const fn uses_fit(self) -> bool {
        !self.is_tapped()
    }

    /// Non-`None` when solid geometry construction requires a Catallax capability
    /// not yet present in the pinned kernel.
    pub const fn coming_soon_reason(self) -> Option<&'static str> {
        match self {
            Self::TaperedTap => Some("Tapered (pipe) tap geometry is coming soon."),
            Self::CountersinkSlot => Some("Countersink slot geometry is coming soon."),
            _ => None,
        }
    }
}

/// Clearance fit series.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoleFit {
    /// Fine / Close fit (e.g., ISO 273 Fine / H12, ASME B18.2.8 Close).
    Close,
    /// Medium / Normal fit (e.g., ISO 273 Medium / H13, ASME B18.2.8 Normal).
    #[default]
    Normal,
    /// Coarse / Loose fit (e.g., ISO 273 Coarse / H14, ASME B18.2.8 Loose).
    Loose,
}

impl HoleFit {
    /// All three fits in order.
    pub const ALL: [Self; 3] = [Self::Close, Self::Normal, Self::Loose];

    /// Display label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Close => "Close",
            Self::Normal => "Normal",
            Self::Loose => "Loose",
        }
    }
}

/// One row of a fastener / hole dimension table. All linear values are in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoleSizeRow {
    /// Nominal size label, e.g. `"M6"`, `"1/4"`, `"#10"`, `"1/4-20"`, `"1/4 NPT"`.
    pub name: &'static str,
    /// Nominal major diameter in millimetres.
    pub nominal_mm: f64,
    /// Close-fit clearance hole diameter in millimetres (or tap drill for tapped rows).
    pub close_mm: f64,
    /// Normal-fit clearance hole diameter in millimetres (or tap drill for tapped rows).
    pub normal_mm: f64,
    /// Loose-fit clearance hole diameter in millimetres (or tap drill for tapped rows).
    pub loose_mm: f64,
    /// Counterbore diameter in millimetres, when applicable.
    pub cbore_diameter_mm: Option<f64>,
    /// Counterbore depth in millimetres, when applicable.
    pub cbore_depth_mm: Option<f64>,
    /// Countersink diameter at the entry surface in millimetres, when applicable.
    pub csk_diameter_mm: Option<f64>,
    /// Countersink included angle in degrees (90° metric, 82° ANSI inch), when applicable.
    pub csk_angle_deg: Option<f64>,
    /// Tap drill diameter in millimetres, for tapped holes.
    pub tap_drill_mm: Option<f64>,
    /// Thread pitch in millimetres, for tapped holes.
    pub pitch_mm: Option<f64>,
    /// Default internal thread tolerance class (`"6H"`, `"2B"`, etc.).
    pub thread_class: Option<&'static str>,
    /// Explicit thread designation for non-ISO threads (e.g. `"1/4-20 UNC-2B"`, `"1/4-18 NPT"`).
    /// `None` uses the standard ISO `M{size}x{pitch}-{class}` format.
    pub thread_designation: Option<&'static str>,
    /// Standard drill-point tip angle in degrees (118°).
    pub tip_angle_deg: f64,
}

impl HoleSizeRow {
    /// Through/pilot diameter for the requested fit (or tap drill for tapped rows).
    pub fn diameter_for_fit(&self, fit: HoleFit) -> f64 {
        if let Some(drill) = self.tap_drill_mm {
            return drill;
        }
        match fit {
            HoleFit::Close => self.close_mm,
            HoleFit::Normal => self.normal_mm,
            HoleFit::Loose => self.loose_mm,
        }
    }

    /// Sensible default centre-to-centre slot length for this nominal size.
    pub fn default_slot_length_mm(&self) -> f64 {
        ((self.nominal_mm * 2.0) * 10.0).round() / 10.0
    }

    /// Builds the cosmetic [`HoleThread`] for a tapped row, when present.
    pub fn hole_thread(&self, length: Option<f64>) -> Option<HoleThread> {
        let pitch = self.pitch_mm?;
        let class = self.thread_class.unwrap_or("6H").to_owned();
        Some(HoleThread {
            size: self.nominal_mm,
            pitch,
            class,
            length,
            designation_override: self.thread_designation.map(str::to_owned),
        })
    }
}

/// One fastener type catalog entry under a standard and hole type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FastenerTable {
    /// Stable identifier saved in part files and favorites.
    pub id: &'static str,
    /// Display name shown in the Type combo box.
    pub label: &'static str,
    /// Published standard cited for the values in `sizes`.
    pub citation: &'static str,
    /// Available nominal sizes in ascending order.
    pub sizes: &'static [HoleSizeRow],
}

/// End condition for a Hole Wizard feature.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HoleEndCondition {
    /// Fixed depth measured from the entry face.
    Blind {
        /// Main-wall depth in millimetres.
        depth: f64,
    },
    /// Through the entire body along the drilling direction.
    #[default]
    ThroughAll,
    /// Up to the next body boundary encountered along the drilling direction.
    UpToNext,
    /// Stops at the plane through the referenced vertex perpendicular to the drilling axis.
    UpToVertex {
        /// Persistent vertex reference.
        vertex: VertexRef,
    },
    /// Stops at the referenced surface.
    UpToSurface {
        /// Persistent face reference.
        face: FaceRef,
    },
    /// Stops at a signed offset from the referenced surface.
    OffsetFromSurface {
        /// Persistent face reference.
        face: FaceRef,
        /// Offset distance in millimetres (positive stops short of the surface unless `reverse_offset` is true).
        offset: f64,
        /// When true, offsets past the surface instead of stopping short of it.
        #[serde(default, skip_serializing_if = "is_false")]
        reverse_offset: bool,
    },
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl HoleEndCondition {
    /// Display label for the end condition selector.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Blind { .. } => "Blind",
            Self::ThroughAll => "Through All",
            Self::UpToNext => "Up To Next",
            Self::UpToVertex { .. } => "Up To Vertex",
            Self::UpToSurface { .. } => "Up To Surface",
            Self::OffsetFromSurface { .. } => "Offset From Surface",
        }
    }

    /// Feature ids referenced by this end condition.
    pub fn dependencies(&self) -> Vec<FeatureId> {
        match self {
            Self::UpToSurface { face } | Self::OffsetFromSurface { face, .. } => vec![face.feature],
            Self::UpToVertex { vertex } => vertex.faces.iter().map(|f| f.feature).collect(),
            Self::Blind { .. } | Self::ThroughAll | Self::UpToNext => Vec::new(),
        }
    }
}

/// Countersink geometry for near-side, under-head, or far-side options.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CountersinkOption {
    /// Countersink mouth diameter in millimetres.
    pub diameter: f64,
    /// Countersink included angle in degrees, in (0, 180).
    pub angle_deg: f64,
}

/// Optional Hole Wizard modifiers (Options section).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoleOptions {
    /// Extra head clearance depth added above a counterbore/countersink head, in mm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_clearance: Option<f64>,
    /// Optional chamfer/countersink at the hole's entry mouth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near_side_csk: Option<CountersinkOption>,
    /// Optional chamfer/countersink between counterbore floor and main bore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub under_head_csk: Option<CountersinkOption>,
    /// Optional chamfer/countersink at the far exit mouth of a through hole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub far_side_csk: Option<CountersinkOption>,
    /// Show cosmetic thread annotation and viewport marks on tapped holes.
    #[serde(default = "default_true")]
    pub cosmetic_thread: bool,
}

fn default_true() -> bool {
    true
}

impl Default for HoleOptions {
    fn default() -> Self {
        Self {
            head_clearance: None,
            near_side_csk: None,
            under_head_csk: None,
            far_side_csk: None,
            cosmetic_thread: true,
        }
    }
}

/// Which hole callout value a tolerance/precision entry governs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalloutParam {
    /// Through/pilot/tap-drill diameter.
    Diameter,
    /// Blind hole depth.
    Depth,
    /// Counterbore diameter.
    CounterboreDiameter,
    /// Counterbore depth.
    CounterboreDepth,
    /// Countersink diameter.
    CountersinkDiameter,
    /// Countersink angle.
    CountersinkAngle,
    /// Thread depth.
    ThreadDepth,
    /// Slot centre-to-centre length.
    SlotLength,
}

impl CalloutParam {
    /// Display name.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Diameter => "Hole Diameter",
            Self::Depth => "Hole Depth",
            Self::CounterboreDiameter => "Counterbore Diameter",
            Self::CounterboreDepth => "Counterbore Depth",
            Self::CountersinkDiameter => "Countersink Diameter",
            Self::CountersinkAngle => "Countersink Angle",
            Self::ThreadDepth => "Thread Depth",
            Self::SlotLength => "Slot Length",
        }
    }
}

/// Tolerance presentation style for a hole callout value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToleranceMode {
    /// Nominal value only.
    #[default]
    None,
    /// Separate upper and lower deviations, e.g. `+0.10/-0.05`.
    Bilateral {
        /// Upper deviation (mm or deg).
        upper: f64,
        /// Lower deviation (mm or deg).
        lower: f64,
    },
    /// Symmetric ± deviation, e.g. `±0.05`.
    Symmetric {
        /// Plus/minus magnitude (mm or deg).
        plus_minus: f64,
    },
    /// Upper and lower limits, e.g. `6.65/6.55`.
    Limit {
        /// Upper deviation added to nominal.
        upper: f64,
        /// Lower deviation added to nominal.
        lower: f64,
    },
    /// ISO / engineering fit class symbol, e.g. `"H13"`, `"H7"`.
    Fit {
        /// Fit designation.
        class: String,
    },
}

impl ToleranceMode {
    /// Display label.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Bilateral { .. } => "Bilateral",
            Self::Symmetric { .. } => "Symmetric",
            Self::Limit { .. } => "Limit",
            Self::Fit { .. } => "Fit",
        }
    }
}

/// Tolerance and decimal precision attached to one callout value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalloutTolerance {
    /// Which parameter this entry formats.
    pub param: CalloutParam,
    /// Tolerance rule.
    #[serde(default)]
    pub mode: ToleranceMode,
    /// Decimal places (0..=4).
    #[serde(default = "default_precision")]
    pub precision: u8,
}

fn default_precision() -> u8 {
    2
}

impl CalloutTolerance {
    /// Formats `nominal` according to this tolerance and precision rule.
    pub fn format_value(&self, nominal: f64) -> String {
        let p = usize::from(self.precision.min(4));
        let fmt = |v: f64| format!("{v:.p$}");
        match &self.mode {
            ToleranceMode::None => fmt(nominal),
            ToleranceMode::Bilateral { upper, lower } => {
                format!("{} +{}/{}", fmt(nominal), fmt(upper.abs()), fmt(*lower))
            }
            ToleranceMode::Symmetric { plus_minus } => format!("{} ±{}", fmt(nominal), fmt(plus_minus.abs())),
            ToleranceMode::Limit { upper, lower } => format!("{}/{}", fmt(nominal + upper), fmt(nominal + lower)),
            ToleranceMode::Fit { class } if !class.trim().is_empty() => format!("{} {}", fmt(nominal), class.trim()),
            ToleranceMode::Fit { .. } => fmt(nominal),
        }
    }
}

/// Complete Hole Wizard specification saved on a [`HoleFeature`](crate::HoleFeature)
/// and in user Favorites.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoleWizardSpec {
    /// Selected engineering standard.
    pub standard: HoleStandard,
    /// Selected hole type.
    pub hole_type: HoleType,
    /// Fastener table id under `(standard, hole_type)`.
    pub fastener: String,
    /// Nominal size name inside `fastener`, e.g. `"M6"` or `"1/4"`.
    pub size: String,
    /// Clearance fit (Close / Normal / Loose).
    #[serde(default)]
    pub fit: HoleFit,
    /// True when the user edited one or more sizing values away from the table defaults.
    #[serde(default, skip_serializing_if = "is_false")]
    pub custom_sizing: bool,
    /// Centre-to-centre slot length in millimetres (used when `hole_type.is_slot()`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot_length: Option<f64>,
    /// Slot orientation angle in the face sketch frame, in degrees.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub slot_angle_deg: f64,
    /// End condition (Blind, Through All, Up To Next, Up To Vertex, Up To Surface, Offset).
    #[serde(default)]
    pub end_condition: HoleEndCondition,
    /// Reverses the drilling direction from into the face (`-normal`) to `+normal`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_direction: bool,
    /// Optional modifiers (head clearance, near/under/far countersinks, cosmetic thread).
    #[serde(default)]
    pub options: HoleOptions,
    /// Per-parameter tolerance and precision callout overrides.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tolerances: Vec<CalloutTolerance>,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl Default for HoleWizardSpec {
    fn default() -> Self {
        Self::new(HoleStandard::Iso, HoleType::Counterbore)
    }
}

/// Resolved hole geometry values derived from a [`HoleWizardSpec`] table lookup.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedHoleValues {
    /// Main wall diameter (or tap drill diameter) in mm.
    pub diameter: f64,
    /// Bottom tip (flat or drill-point cone).
    pub tip: HoleTip,
    /// Entry head (counterbore or countersink), if any.
    pub head: Option<HoleHead>,
    /// Cosmetic thread, when `hole_type` is tapped and `options.cosmetic_thread` is true.
    pub thread: Option<HoleThread>,
    /// Default slot centre-to-centre length, when `hole_type.is_slot()`.
    pub slot_length: Option<f64>,
}

impl HoleWizardSpec {
    /// Creates a spec initialized to the first fastener table and a sensible default size
    /// (M6 for metric, 1/4 for ANSI Inch, or the first available size).
    pub fn new(standard: HoleStandard, hole_type: HoleType) -> Self {
        let tables = fastener_tables(standard, hole_type);
        let table = tables.first().expect("every (standard, hole_type) has at least one fastener table");
        let row = default_row(table, standard);
        Self {
            standard,
            hole_type,
            fastener: table.id.to_owned(),
            size: row.name.to_owned(),
            fit: HoleFit::Normal,
            custom_sizing: false,
            slot_length: hole_type.is_slot().then(|| row.default_slot_length_mm()),
            slot_angle_deg: 0.0,
            end_condition: HoleEndCondition::ThroughAll,
            flip_direction: false,
            options: HoleOptions::default(),
            tolerances: Vec::new(),
        }
    }

    /// Looks up the active [`FastenerTable`].
    pub fn fastener_table(&self) -> &'static FastenerTable {
        let tables = fastener_tables(self.standard, self.hole_type);
        tables.iter().find(|t| t.id == self.fastener).unwrap_or(&tables[0])
    }

    /// Looks up the active [`HoleSizeRow`].
    pub fn size_row(&self) -> &'static HoleSizeRow {
        let table = self.fastener_table();
        table.sizes.iter().find(|r| r.name == self.size).unwrap_or(&table.sizes[0])
    }

    /// Switches standard, keeping the hole type and picking a valid fastener & size.
    pub fn set_standard(&mut self, standard: HoleStandard) {
        if self.standard == standard {
            return;
        }
        self.standard = standard;
        let tables = fastener_tables(standard, self.hole_type);
        let table = tables[0];
        let row = default_row(table, standard);
        self.fastener = table.id.to_owned();
        self.size = row.name.to_owned();
        self.custom_sizing = false;
        if self.hole_type.is_slot() {
            self.slot_length = Some(row.default_slot_length_mm());
        }
    }

    /// Switches hole type, picking a valid fastener & size under the current standard.
    pub fn set_hole_type(&mut self, hole_type: HoleType) {
        if self.hole_type == hole_type {
            return;
        }
        let prev_size = self.size.clone();
        self.hole_type = hole_type;
        let tables = fastener_tables(self.standard, hole_type);
        let table = tables[0];
        let row = table.sizes.iter().find(|r| r.name == prev_size).unwrap_or_else(|| default_row(table, self.standard));
        self.fastener = table.id.to_owned();
        self.size = row.name.to_owned();
        self.custom_sizing = false;
        self.slot_length = hole_type.is_slot().then(|| row.default_slot_length_mm());
    }

    /// Switches fastener table under the current `(standard, hole_type)`.
    pub fn set_fastener(&mut self, fastener_id: &str) {
        let tables = fastener_tables(self.standard, self.hole_type);
        let Some(table) = tables.iter().find(|t| t.id == fastener_id) else { return };
        let prev_size = self.size.clone();
        self.fastener = table.id.to_owned();
        let row = table.sizes.iter().find(|r| r.name == prev_size).unwrap_or_else(|| default_row(table, self.standard));
        self.size = row.name.to_owned();
        self.custom_sizing = false;
        if self.hole_type.is_slot() {
            self.slot_length = Some(row.default_slot_length_mm());
        }
    }

    /// Resolves standard table defaults into concrete geometry parameters.
    pub fn resolve_defaults(&self) -> ResolvedHoleValues {
        let row = self.size_row();
        let diameter = row.diameter_for_fit(self.fit);
        let head = match self.hole_type {
            HoleType::Counterbore | HoleType::CounterboreSlot => Some(HoleHead::Counterbore {
                diameter: row.cbore_diameter_mm.unwrap_or((diameter * 1.65 * 10.0).round() / 10.0),
                depth: row.cbore_depth_mm.unwrap_or(row.nominal_mm),
            }),
            HoleType::Countersink | HoleType::CountersinkSlot => Some(HoleHead::Countersink {
                diameter: row.csk_diameter_mm.unwrap_or((diameter * 2.0 * 10.0).round() / 10.0),
                angle_deg: row.csk_angle_deg.unwrap_or(if self.standard.is_inch() { 82.0 } else { 90.0 }),
            }),
            HoleType::Simple | HoleType::StraightTap | HoleType::TaperedTap | HoleType::Slot => None,
        };
        let thread = if self.hole_type.is_tapped() && self.options.cosmetic_thread {
            row.hole_thread(None)
        } else {
            None
        };
        let tip = if self.hole_type.is_slot() {
            HoleTip::Flat
        } else {
            HoleTip::Cone { angle_deg: row.tip_angle_deg }
        };
        ResolvedHoleValues {
            diameter,
            tip,
            head,
            thread,
            slot_length: self.hole_type.is_slot().then(|| row.default_slot_length_mm()),
        }
    }

    /// Maps the wizard's [`HoleEndCondition`] to the legacy [`HoleDepth`] when representable.
    pub fn legacy_depth(&self) -> HoleDepth {
        match self.end_condition {
            HoleEndCondition::Blind { depth } => HoleDepth::Blind { depth },
            _ => HoleDepth::Through,
        }
    }

    /// Looks up the tolerance/precision rule for `param`, if any.
    pub fn tolerance_for(&self, param: CalloutParam) -> Option<&CalloutTolerance> {
        self.tolerances.iter().find(|t| t.param == param)
    }

    /// Sets or updates a callout tolerance entry.
    pub fn set_tolerance(&mut self, entry: CalloutTolerance) {
        if let Some(existing) = self.tolerances.iter_mut().find(|t| t.param == entry.param) {
            *existing = entry;
        } else {
            self.tolerances.push(entry);
        }
    }

    /// Removes a callout tolerance entry.
    pub fn clear_tolerance(&mut self, param: CalloutParam) {
        self.tolerances.retain(|t| t.param != param);
    }

    /// Named coming-soon refusal when this spec requests geometry not yet supported by
    /// the pinned Catallax kernel.
    pub fn coming_soon_refusal(&self) -> Option<&'static str> {
        if let Some(why) = self.hole_type.coming_soon_reason() {
            return Some(why);
        }
        if self.options.under_head_csk.is_some() {
            return Some("Under head countersink is coming soon.");
        }
        if self.options.far_side_csk.is_some() {
            return Some("Far side countersink is coming soon.");
        }
        if self.options.head_clearance.is_some_and(|c| c > 0.0) && self.hole_type.has_countersink() {
            return Some("Head clearance on a countersink is coming soon.");
        }
        if self.options.near_side_csk.is_some() && self.hole_type.has_counterbore() {
            return Some("Near side countersink on a counterbore is coming soon.");
        }
        None
    }
}

/// A named Hole Wizard preset stored in user Favorites.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoleFavorite {
    /// User-assigned preset name.
    pub name: String,
    /// Saved Hole Wizard specification.
    pub spec: HoleWizardSpec,
    /// Custom diameter override in mm, when `spec.custom_sizing` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_diameter: Option<f64>,
    /// Custom head override, when `spec.custom_sizing` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_head: Option<HoleHead>,
    /// Custom tip override, when `spec.custom_sizing` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_tip: Option<HoleTip>,
    /// Custom thread override, when `spec.custom_sizing` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_thread: Option<HoleThread>,
}

fn default_row(table: &'static FastenerTable, standard: HoleStandard) -> &'static HoleSizeRow {
    let preferred = if standard.is_inch() { &["1/4", "1/4-20", "1/4 NPT"][..] } else { &["M6", "1/4"][..] };
    for want in preferred {
        if let Some(row) = table.sizes.iter().find(|r| r.name == *want) {
            return row;
        }
    }
    &table.sizes[0]
}

// ---------------------------------------------------------------------------
// Dimension table constructors & published standard data
// ---------------------------------------------------------------------------

const fn clr(name: &'static str, nominal_mm: f64, close_mm: f64, normal_mm: f64, loose_mm: f64) -> HoleSizeRow {
    HoleSizeRow {
        name,
        nominal_mm,
        close_mm,
        normal_mm,
        loose_mm,
        cbore_diameter_mm: None,
        cbore_depth_mm: None,
        csk_diameter_mm: None,
        csk_angle_deg: None,
        tap_drill_mm: None,
        pitch_mm: None,
        thread_class: None,
        thread_designation: None,
        tip_angle_deg: 118.0,
    }
}

const fn cbore(
    name: &'static str,
    nominal_mm: f64,
    close_mm: f64,
    normal_mm: f64,
    loose_mm: f64,
    cbore_diameter_mm: f64,
    cbore_depth_mm: f64,
) -> HoleSizeRow {
    HoleSizeRow {
        name,
        nominal_mm,
        close_mm,
        normal_mm,
        loose_mm,
        cbore_diameter_mm: Some(cbore_diameter_mm),
        cbore_depth_mm: Some(cbore_depth_mm),
        csk_diameter_mm: None,
        csk_angle_deg: None,
        tap_drill_mm: None,
        pitch_mm: None,
        thread_class: None,
        thread_designation: None,
        tip_angle_deg: 118.0,
    }
}

const fn csk(
    name: &'static str,
    nominal_mm: f64,
    close_mm: f64,
    normal_mm: f64,
    loose_mm: f64,
    csk_diameter_mm: f64,
    csk_angle_deg: f64,
) -> HoleSizeRow {
    HoleSizeRow {
        name,
        nominal_mm,
        close_mm,
        normal_mm,
        loose_mm,
        cbore_diameter_mm: None,
        cbore_depth_mm: None,
        csk_diameter_mm: Some(csk_diameter_mm),
        csk_angle_deg: Some(csk_angle_deg),
        tap_drill_mm: None,
        pitch_mm: None,
        thread_class: None,
        thread_designation: None,
        tip_angle_deg: 118.0,
    }
}

const fn tap(
    name: &'static str,
    nominal_mm: f64,
    pitch_mm: f64,
    tap_drill_mm: f64,
    thread_class: &'static str,
    thread_designation: Option<&'static str>,
) -> HoleSizeRow {
    HoleSizeRow {
        name,
        nominal_mm,
        close_mm: tap_drill_mm,
        normal_mm: tap_drill_mm,
        loose_mm: tap_drill_mm,
        cbore_diameter_mm: None,
        cbore_depth_mm: None,
        csk_diameter_mm: None,
        csk_angle_deg: None,
        tap_drill_mm: Some(tap_drill_mm),
        pitch_mm: Some(pitch_mm),
        thread_class: Some(thread_class),
        thread_designation,
        tip_angle_deg: 118.0,
    }
}

const fn inch(v: f64) -> f64 {
    v * MM_PER_INCH
}

// ===========================================================================
// METRIC TABLES (ISO 273, DIN 974-1, ISO 4762, ISO 4014, ISO 10642, DIN 74,
// ISO 262, ISO 2306, ASME B18.3.1M, ASME B18.2.3.5M, JIS B 1001, JIS B 1176,
// GB/T 152.2, ISO 7-1 / BS 21)
// ===========================================================================

/// ISO 273 clearance holes (Fine / Medium / Coarse), M1.6–M36.
/// Shared by ISO 273, DIN EN 20273, ASME B18.2.8 (Metric), JIS B 1001,
/// AS 1420, BS 4186, GB/T 5277, KS B ISO 273, and IS 1821.
const ISO_273_CLEARANCE: &[HoleSizeRow] = &[
    clr("M1.6", 1.6, 1.7, 1.8, 2.0),
    clr("M2", 2.0, 2.2, 2.4, 2.6),
    clr("M2.5", 2.5, 2.7, 2.9, 3.1),
    clr("M3", 3.0, 3.2, 3.4, 3.6),
    clr("M3.5", 3.5, 3.7, 3.9, 4.2),
    clr("M4", 4.0, 4.3, 4.5, 4.8),
    clr("M5", 5.0, 5.3, 5.5, 5.8),
    clr("M6", 6.0, 6.4, 6.6, 7.0),
    clr("M8", 8.0, 8.4, 9.0, 10.0),
    clr("M10", 10.0, 10.5, 11.0, 12.0),
    clr("M12", 12.0, 13.0, 13.5, 14.5),
    clr("M14", 14.0, 15.0, 15.5, 16.5),
    clr("M16", 16.0, 17.0, 17.5, 18.5),
    clr("M18", 18.0, 19.0, 20.0, 21.0),
    clr("M20", 20.0, 21.0, 22.0, 24.0),
    clr("M22", 22.0, 23.0, 24.0, 26.0),
    clr("M24", 24.0, 25.0, 26.0, 28.0),
    clr("M27", 27.0, 28.0, 30.0, 32.0),
    clr("M30", 30.0, 31.0, 33.0, 35.0),
    clr("M36", 36.0, 37.0, 39.0, 42.0),
];

/// ISO 4762 / DIN 912 Socket Head Cap Screw counterbores per DIN 974-1 (depth = k_max + clearance).
const ISO_4762_SHCS_CBORE: &[HoleSizeRow] = &[
    cbore("M1.6", 1.6, 1.7, 1.8, 2.0, 3.8, 2.0),
    cbore("M2", 2.0, 2.2, 2.4, 2.6, 4.4, 2.4),
    cbore("M2.5", 2.5, 2.7, 2.9, 3.1, 5.4, 2.9),
    cbore("M3", 3.0, 3.2, 3.4, 3.6, 6.5, 3.4),
    cbore("M4", 4.0, 4.3, 4.5, 4.8, 8.0, 4.4),
    cbore("M5", 5.0, 5.3, 5.5, 5.8, 10.0, 5.4),
    cbore("M6", 6.0, 6.4, 6.6, 7.0, 11.0, 6.4),
    cbore("M8", 8.0, 8.4, 9.0, 10.0, 15.0, 8.6),
    cbore("M10", 10.0, 10.5, 11.0, 12.0, 18.0, 10.6),
    cbore("M12", 12.0, 13.0, 13.5, 14.5, 20.0, 12.6),
    cbore("M14", 14.0, 15.0, 15.5, 16.5, 24.0, 14.6),
    cbore("M16", 16.0, 17.0, 17.5, 18.5, 26.0, 16.6),
    cbore("M20", 20.0, 21.0, 22.0, 24.0, 33.0, 20.6),
    cbore("M24", 24.0, 25.0, 26.0, 28.0, 40.0, 24.6),
    cbore("M30", 30.0, 31.0, 33.0, 35.0, 48.0, 31.0),
];

/// ASME B18.3.1M Metric Socket Head Cap Screw counterbores.
const ASME_B18_3_1M_SHCS_CBORE: &[HoleSizeRow] = &[
    cbore("M1.6", 1.6, 1.7, 1.8, 2.0, 3.8, 1.8),
    cbore("M2", 2.0, 2.2, 2.4, 2.6, 4.4, 2.2),
    cbore("M2.5", 2.5, 2.7, 2.9, 3.1, 5.4, 2.7),
    cbore("M3", 3.0, 3.2, 3.4, 3.6, 6.5, 3.3),
    cbore("M4", 4.0, 4.3, 4.5, 4.8, 8.0, 4.4),
    cbore("M5", 5.0, 5.3, 5.5, 5.8, 10.0, 5.4),
    cbore("M6", 6.0, 6.4, 6.6, 7.0, 11.0, 6.4),
    cbore("M8", 8.0, 8.4, 9.0, 10.0, 15.0, 8.4),
    cbore("M10", 10.0, 10.5, 11.0, 12.0, 18.0, 10.4),
    cbore("M12", 12.0, 13.0, 13.5, 14.5, 20.0, 12.4),
    cbore("M16", 16.0, 17.0, 17.5, 18.5, 26.0, 16.4),
    cbore("M20", 20.0, 21.0, 22.0, 24.0, 31.5, 20.4),
    cbore("M24", 24.0, 25.0, 26.0, 28.0, 37.5, 24.4),
];

/// JIS B 1176 Metric Socket Head Cap Screw counterbores.
const JIS_B1176_SHCS_CBORE: &[HoleSizeRow] = &[
    cbore("M3", 3.0, 3.2, 3.4, 3.6, 6.5, 3.3),
    cbore("M4", 4.0, 4.3, 4.5, 4.8, 8.0, 4.4),
    cbore("M5", 5.0, 5.3, 5.5, 5.8, 9.5, 5.4),
    cbore("M6", 6.0, 6.4, 6.6, 7.0, 11.0, 6.5),
    cbore("M8", 8.0, 8.4, 9.0, 10.0, 14.0, 8.6),
    cbore("M10", 10.0, 10.5, 11.0, 12.0, 17.5, 10.8),
    cbore("M12", 12.0, 13.0, 13.5, 14.5, 20.0, 13.0),
    cbore("M14", 14.0, 15.0, 15.5, 16.5, 23.0, 15.2),
    cbore("M16", 16.0, 17.0, 17.5, 18.5, 26.0, 17.5),
    cbore("M20", 20.0, 21.0, 22.0, 24.0, 32.0, 21.5),
    cbore("M24", 24.0, 25.0, 26.0, 28.0, 39.0, 25.5),
];

/// GB/T 152.2 Socket Head Cap Screw counterbores (GB/T 70.1).
const GB_152_2_SHCS_CBORE: &[HoleSizeRow] = &[
    cbore("M3", 3.0, 3.2, 3.4, 3.6, 6.5, 3.4),
    cbore("M4", 4.0, 4.3, 4.5, 4.8, 8.0, 4.5),
    cbore("M5", 5.0, 5.3, 5.5, 5.8, 10.0, 5.7),
    cbore("M6", 6.0, 6.4, 6.6, 7.0, 11.0, 6.8),
    cbore("M8", 8.0, 8.4, 9.0, 10.0, 15.0, 9.0),
    cbore("M10", 10.0, 10.5, 11.0, 12.0, 18.0, 11.0),
    cbore("M12", 12.0, 13.0, 13.5, 14.5, 20.0, 13.0),
    cbore("M16", 16.0, 17.0, 17.5, 18.5, 26.0, 17.5),
    cbore("M20", 20.0, 21.0, 22.0, 24.0, 33.0, 21.5),
];

/// ISO 4014 / ASME B18.2.3.5M Metric Hex Bolt counterbores (sized for socket wrench clearance).
const ISO_4014_HEX_CBORE: &[HoleSizeRow] = &[
    cbore("M4", 4.0, 4.3, 4.5, 4.8, 12.0, 3.2),
    cbore("M5", 5.0, 5.3, 5.5, 5.8, 13.5, 3.9),
    cbore("M6", 6.0, 6.4, 6.6, 7.0, 16.0, 4.4),
    cbore("M8", 8.0, 8.4, 9.0, 10.0, 20.0, 5.8),
    cbore("M10", 10.0, 10.5, 11.0, 12.0, 24.0, 7.0),
    cbore("M12", 12.0, 13.0, 13.5, 14.5, 27.0, 8.0),
    cbore("M14", 14.0, 15.0, 15.5, 16.5, 31.0, 9.25),
    cbore("M16", 16.0, 17.0, 17.5, 18.5, 34.0, 10.5),
    cbore("M20", 20.0, 21.0, 22.0, 24.0, 43.0, 13.0),
    cbore("M24", 24.0, 25.0, 26.0, 28.0, 52.0, 15.5),
];

/// DIN 931 Hex Bolt counterbores (DIN 931 wrench flats M10=17, M12=19, M14=22).
const DIN_931_HEX_CBORE: &[HoleSizeRow] = &[
    cbore("M4", 4.0, 4.3, 4.5, 4.8, 12.0, 3.2),
    cbore("M5", 5.0, 5.3, 5.5, 5.8, 13.5, 3.9),
    cbore("M6", 6.0, 6.4, 6.6, 7.0, 16.0, 4.4),
    cbore("M8", 8.0, 8.4, 9.0, 10.0, 20.0, 5.8),
    cbore("M10", 10.0, 10.5, 11.0, 12.0, 25.0, 7.0),
    cbore("M12", 12.0, 13.0, 13.5, 14.5, 28.0, 8.0),
    cbore("M14", 14.0, 15.0, 15.5, 16.5, 32.0, 9.25),
    cbore("M16", 16.0, 17.0, 17.5, 18.5, 34.0, 10.5),
    cbore("M20", 20.0, 21.0, 22.0, 24.0, 43.0, 13.0),
    cbore("M24", 24.0, 25.0, 26.0, 28.0, 52.0, 15.5),
];

/// ISO 10642 / DIN 74 Form F 90° Countersunk Flat Head Socket Cap Screws.
const ISO_10642_CSK_90: &[HoleSizeRow] = &[
    csk("M2", 2.0, 2.2, 2.4, 2.6, 4.6, 90.0),
    csk("M2.5", 2.5, 2.7, 2.9, 3.1, 5.7, 90.0),
    csk("M3", 3.0, 3.2, 3.4, 3.6, 6.94, 90.0),
    csk("M4", 4.0, 4.3, 4.5, 4.8, 9.18, 90.0),
    csk("M5", 5.0, 5.3, 5.5, 5.8, 11.47, 90.0),
    csk("M6", 6.0, 6.4, 6.6, 7.0, 13.71, 90.0),
    csk("M8", 8.0, 8.4, 9.0, 10.0, 18.25, 90.0),
    csk("M10", 10.0, 10.5, 11.0, 12.0, 22.73, 90.0),
    csk("M12", 12.0, 13.0, 13.5, 14.5, 27.21, 90.0),
    csk("M16", 16.0, 17.0, 17.5, 18.5, 33.99, 90.0),
    csk("M20", 20.0, 21.0, 22.0, 24.0, 40.71, 90.0),
];

/// DIN 7991 90° Countersunk Flat Head Socket Cap Screws (classic DIN 7991 head diameters).
const DIN_7991_CSK_90: &[HoleSizeRow] = &[
    csk("M3", 3.0, 3.2, 3.4, 3.6, 6.3, 90.0),
    csk("M4", 4.0, 4.3, 4.5, 4.8, 8.4, 90.0),
    csk("M5", 5.0, 5.3, 5.5, 5.8, 10.4, 90.0),
    csk("M6", 6.0, 6.4, 6.6, 7.0, 12.6, 90.0),
    csk("M8", 8.0, 8.4, 9.0, 10.0, 16.5, 90.0),
    csk("M10", 10.0, 10.5, 11.0, 12.0, 20.6, 90.0),
    csk("M12", 12.0, 13.0, 13.5, 14.5, 24.8, 90.0),
    csk("M16", 16.0, 17.0, 17.5, 18.5, 31.0, 90.0),
    csk("M20", 20.0, 21.0, 22.0, 24.0, 37.0, 90.0),
];

/// ISO 262 / ISO 2306 Metric Coarse Tapped Holes (M1.6–M36).
const ISO_METRIC_COARSE_TAP: &[HoleSizeRow] = &[
    tap("M1.6", 1.6, 0.35, 1.25, "6H", None),
    tap("M2", 2.0, 0.4, 1.6, "6H", None),
    tap("M2.5", 2.5, 0.45, 2.05, "6H", None),
    tap("M3", 3.0, 0.5, 2.5, "6H", None),
    tap("M3.5", 3.5, 0.6, 2.9, "6H", None),
    tap("M4", 4.0, 0.7, 3.3, "6H", None),
    tap("M5", 5.0, 0.8, 4.2, "6H", None),
    tap("M6", 6.0, 1.0, 5.0, "6H", None),
    tap("M8", 8.0, 1.25, 6.8, "6H", None),
    tap("M10", 10.0, 1.5, 8.5, "6H", None),
    tap("M12", 12.0, 1.75, 10.2, "6H", None),
    tap("M14", 14.0, 2.0, 12.0, "6H", None),
    tap("M16", 16.0, 2.0, 14.0, "6H", None),
    tap("M18", 18.0, 2.5, 15.5, "6H", None),
    tap("M20", 20.0, 2.5, 17.5, "6H", None),
    tap("M22", 22.0, 2.5, 19.5, "6H", None),
    tap("M24", 24.0, 3.0, 21.0, "6H", None),
    tap("M27", 27.0, 3.0, 24.0, "6H", None),
    tap("M30", 30.0, 3.5, 26.5, "6H", None),
    tap("M36", 36.0, 4.0, 32.0, "6H", None),
];

/// ISO 262 Metric Fine Tapped Holes (tap drill = nominal - pitch).
const ISO_METRIC_FINE_TAP: &[HoleSizeRow] = &[
    tap("M8x1.0", 8.0, 1.0, 7.0, "6H", None),
    tap("M10x1.25", 10.0, 1.25, 8.8, "6H", None),
    tap("M10x1.0", 10.0, 1.0, 9.0, "6H", None),
    tap("M12x1.5", 12.0, 1.5, 10.5, "6H", None),
    tap("M12x1.25", 12.0, 1.25, 10.8, "6H", None),
    tap("M14x1.5", 14.0, 1.5, 12.5, "6H", None),
    tap("M16x1.5", 16.0, 1.5, 14.5, "6H", None),
    tap("M18x1.5", 18.0, 1.5, 16.5, "6H", None),
    tap("M20x1.5", 20.0, 1.5, 18.5, "6H", None),
    tap("M24x2.0", 24.0, 2.0, 22.0, "6H", None),
    tap("M30x2.0", 30.0, 2.0, 28.0, "6H", None),
];

/// ISO 7-1 / BS 21 / JIS B 0203 / DIN 2999 BSPT Tapered Pipe Taps (1:16 taper, 55° Whitworth).
const ISO_7_1_BSPT_PIPE_TAP: &[HoleSizeRow] = &[
    tap("1/8", 9.728, 0.907, 8.4, "Rc", Some("Rc 1/8")),
    tap("1/4", 13.157, 1.337, 11.2, "Rc", Some("Rc 1/4")),
    tap("3/8", 16.662, 1.337, 14.75, "Rc", Some("Rc 3/8")),
    tap("1/2", 20.955, 1.814, 18.25, "Rc", Some("Rc 1/2")),
    tap("3/4", 26.441, 1.814, 23.75, "Rc", Some("Rc 3/4")),
    tap("1", 33.249, 2.309, 30.0, "Rc", Some("Rc 1")),
    tap("1-1/4", 41.910, 2.309, 38.5, "Rc", Some("Rc 1-1/4")),
    tap("1-1/2", 47.803, 2.309, 44.5, "Rc", Some("Rc 1-1/2")),
    tap("2", 59.614, 2.309, 56.0, "Rc", Some("Rc 2")),
];

// ===========================================================================
// ANSI INCH TABLES (ASME B18.2.8, ASME B18.3, ASME B18.2.1, ASME B1.1,
// ASME B1.20.1 NPT)
// ===========================================================================

/// ASME B18.2.8 Inch Clearance Holes (#4 to 1"), converted to exact mm (`* 25.4`).
const ASME_B18_2_8_INCH_CLEARANCE: &[HoleSizeRow] = &[
    clr("#4", inch(0.1120), inch(0.120), inch(0.1285), inch(0.144)),
    clr("#6", inch(0.1380), inch(0.154), inch(0.1695), inch(0.185)),
    clr("#8", inch(0.1640), inch(0.180), inch(0.196), inch(0.213)),
    clr("#10", inch(0.1900), inch(0.2055), inch(0.221), inch(0.238)),
    clr("1/4", inch(0.2500), inch(0.2656), inch(0.2812), inch(0.2969)),
    clr("5/16", inch(0.3125), inch(0.3281), inch(0.3438), inch(0.3594)),
    clr("3/8", inch(0.3750), inch(0.3906), inch(0.4062), inch(0.4219)),
    clr("7/16", inch(0.4375), inch(0.4531), inch(0.4688), inch(0.4844)),
    clr("1/2", inch(0.5000), inch(0.5312), inch(0.5625), inch(0.6094)),
    clr("5/8", inch(0.6250), inch(0.6562), inch(0.6875), inch(0.7344)),
    clr("3/4", inch(0.7500), inch(0.7812), inch(0.8125), inch(0.9062)),
    clr("1", inch(1.0000), inch(1.0312), inch(1.0938), inch(1.1562)),
];

/// ASME B18.3 Inch Socket Head Cap Screw counterbores (#4 to 1").
const ASME_B18_3_SHCS_CBORE: &[HoleSizeRow] = &[
    cbore("#4", inch(0.1120), inch(0.120), inch(0.136), inch(0.144), inch(0.2188), inch(0.112)),
    cbore("#6", inch(0.1380), inch(0.1495), inch(0.1695), inch(0.185), inch(0.2812), inch(0.138)),
    cbore("#8", inch(0.1640), inch(0.177), inch(0.1935), inch(0.213), inch(0.3125), inch(0.164)),
    cbore("#10", inch(0.1900), inch(0.201), inch(0.221), inch(0.238), inch(0.375), inch(0.190)),
    cbore("1/4", inch(0.2500), inch(0.257), inch(0.2812), inch(0.2969), inch(0.4375), inch(0.250)),
    cbore("5/16", inch(0.3125), inch(0.323), inch(0.3438), inch(0.3594), inch(0.5312), inch(0.3125)),
    cbore("3/8", inch(0.3750), inch(0.386), inch(0.4062), inch(0.4219), inch(0.625), inch(0.375)),
    cbore("7/16", inch(0.4375), inch(0.4531), inch(0.4688), inch(0.4844), inch(0.7188), inch(0.4375)),
    cbore("1/2", inch(0.5000), inch(0.5156), inch(0.5312), inch(0.5625), inch(0.8125), inch(0.500)),
    cbore("5/8", inch(0.6250), inch(0.6406), inch(0.6562), inch(0.7344), inch(1.000), inch(0.625)),
    cbore("3/4", inch(0.7500), inch(0.7656), inch(0.7812), inch(0.9062), inch(1.1875), inch(0.750)),
    cbore("1", inch(1.0000), inch(1.0312), inch(1.0625), inch(1.1562), inch(1.625), inch(1.000)),
];

/// ASME B18.2.1 Inch Hex Cap Screw counterbores (1/4" to 1").
const ASME_B18_2_1_HEX_CBORE: &[HoleSizeRow] = &[
    cbore("1/4", inch(0.2500), inch(0.2656), inch(0.2812), inch(0.2969), inch(0.5938), inch(0.1875)),
    cbore("5/16", inch(0.3125), inch(0.3281), inch(0.3438), inch(0.3594), inch(0.6875), inch(0.2344)),
    cbore("3/8", inch(0.3750), inch(0.3906), inch(0.4062), inch(0.4219), inch(0.7812), inch(0.2656)),
    cbore("7/16", inch(0.4375), inch(0.4531), inch(0.4688), inch(0.4844), inch(0.875), inch(0.3125)),
    cbore("1/2", inch(0.5000), inch(0.5312), inch(0.5625), inch(0.6094), inch(1.0312), inch(0.3438)),
    cbore("5/8", inch(0.6250), inch(0.6562), inch(0.6875), inch(0.7344), inch(1.250), inch(0.4219)),
    cbore("3/4", inch(0.7500), inch(0.7812), inch(0.8125), inch(0.9062), inch(1.4688), inch(0.500)),
    cbore("1", inch(1.0000), inch(1.0312), inch(1.0938), inch(1.1562), inch(1.9375), inch(0.6562)),
];

/// ASME B18.3 Inch 82° Flat Countersunk Head Socket Cap Screws (#4 to 3/4").
const ASME_B18_3_CSK_82: &[HoleSizeRow] = &[
    csk("#4", inch(0.1120), inch(0.120), inch(0.1285), inch(0.144), inch(0.255), 82.0),
    csk("#6", inch(0.1380), inch(0.154), inch(0.1695), inch(0.185), inch(0.307), 82.0),
    csk("#8", inch(0.1640), inch(0.180), inch(0.196), inch(0.213), inch(0.359), 82.0),
    csk("#10", inch(0.1900), inch(0.2055), inch(0.221), inch(0.238), inch(0.411), 82.0),
    csk("1/4", inch(0.2500), inch(0.2656), inch(0.2812), inch(0.2969), inch(0.531), 82.0),
    csk("5/16", inch(0.3125), inch(0.3281), inch(0.3438), inch(0.3594), inch(0.656), 82.0),
    csk("3/8", inch(0.3750), inch(0.3906), inch(0.4062), inch(0.4219), inch(0.781), 82.0),
    csk("1/2", inch(0.5000), inch(0.5312), inch(0.5625), inch(0.6094), inch(0.938), 82.0),
    csk("5/8", inch(0.6250), inch(0.6562), inch(0.6875), inch(0.7344), inch(1.188), 82.0),
    csk("3/4", inch(0.7500), inch(0.7812), inch(0.8125), inch(0.9062), inch(1.438), 82.0),
];

/// ASME B1.1 Unified National Coarse (UNC) Tapped Holes (~75% thread engagement).
const ASME_B1_1_UNC_TAP: &[HoleSizeRow] = &[
    tap("#4-40", inch(0.1120), inch(1.0 / 40.0), inch(0.0890), "2B", Some("#4-40 UNC-2B")),
    tap("#6-32", inch(0.1380), inch(1.0 / 32.0), inch(0.1065), "2B", Some("#6-32 UNC-2B")),
    tap("#8-32", inch(0.1640), inch(1.0 / 32.0), inch(0.1360), "2B", Some("#8-32 UNC-2B")),
    tap("#10-24", inch(0.1900), inch(1.0 / 24.0), inch(0.1495), "2B", Some("#10-24 UNC-2B")),
    tap("1/4-20", inch(0.2500), inch(1.0 / 20.0), inch(0.2010), "2B", Some("1/4-20 UNC-2B")),
    tap("5/16-18", inch(0.3125), inch(1.0 / 18.0), inch(0.2570), "2B", Some("5/16-18 UNC-2B")),
    tap("3/8-16", inch(0.3750), inch(1.0 / 16.0), inch(0.3125), "2B", Some("3/8-16 UNC-2B")),
    tap("7/16-14", inch(0.4375), inch(1.0 / 14.0), inch(0.3680), "2B", Some("7/16-14 UNC-2B")),
    tap("1/2-13", inch(0.5000), inch(1.0 / 13.0), inch(0.4219), "2B", Some("1/2-13 UNC-2B")),
    tap("5/8-11", inch(0.6250), inch(1.0 / 11.0), inch(0.5312), "2B", Some("5/8-11 UNC-2B")),
    tap("3/4-10", inch(0.7500), inch(1.0 / 10.0), inch(0.6562), "2B", Some("3/4-10 UNC-2B")),
    tap("1-8", inch(1.0000), inch(1.0 / 8.0), inch(0.8750), "2B", Some("1-8 UNC-2B")),
];

/// ASME B1.1 Unified National Fine (UNF) Tapped Holes (~75% thread engagement).
const ASME_B1_1_UNF_TAP: &[HoleSizeRow] = &[
    tap("#4-48", inch(0.1120), inch(1.0 / 48.0), inch(0.0935), "2B", Some("#4-48 UNF-2B")),
    tap("#6-40", inch(0.1380), inch(1.0 / 40.0), inch(0.1130), "2B", Some("#6-40 UNF-2B")),
    tap("#8-36", inch(0.1640), inch(1.0 / 36.0), inch(0.1360), "2B", Some("#8-36 UNF-2B")),
    tap("#10-32", inch(0.1900), inch(1.0 / 32.0), inch(0.1590), "2B", Some("#10-32 UNF-2B")),
    tap("1/4-28", inch(0.2500), inch(1.0 / 28.0), inch(0.2130), "2B", Some("1/4-28 UNF-2B")),
    tap("5/16-24", inch(0.3125), inch(1.0 / 24.0), inch(0.2720), "2B", Some("5/16-24 UNF-2B")),
    tap("3/8-24", inch(0.3750), inch(1.0 / 24.0), inch(0.3320), "2B", Some("3/8-24 UNF-2B")),
    tap("1/2-20", inch(0.5000), inch(1.0 / 20.0), inch(0.4531), "2B", Some("1/2-20 UNF-2B")),
    tap("5/8-18", inch(0.6250), inch(1.0 / 18.0), inch(0.5781), "2B", Some("5/8-18 UNF-2B")),
    tap("3/4-16", inch(0.7500), inch(1.0 / 16.0), inch(0.6875), "2B", Some("3/4-16 UNF-2B")),
];

/// ASME B1.20.1 NPT Tapered Pipe Taps (1:16 taper, 60° thread form).
const ASME_B1_20_1_NPT_PIPE_TAP: &[HoleSizeRow] = &[
    tap("1/16 NPT", inch(0.3125), inch(1.0 / 27.0), inch(0.2460), "NPT", Some("1/16-27 NPT")),
    tap("1/8 NPT", inch(0.405), inch(1.0 / 27.0), inch(0.3320), "NPT", Some("1/8-27 NPT")),
    tap("1/4 NPT", inch(0.540), inch(1.0 / 18.0), inch(0.4375), "NPT", Some("1/4-18 NPT")),
    tap("3/8 NPT", inch(0.675), inch(1.0 / 18.0), inch(0.5781), "NPT", Some("3/8-18 NPT")),
    tap("1/2 NPT", inch(0.840), inch(1.0 / 14.0), inch(0.7188), "NPT", Some("1/2-14 NPT")),
    tap("3/4 NPT", inch(1.050), inch(1.0 / 14.0), inch(0.9219), "NPT", Some("3/4-14 NPT")),
    tap("1 NPT", inch(1.315), inch(1.0 / 11.5), inch(1.1562), "NPT", Some("1-11.5 NPT")),
    tap("1-1/4 NPT", inch(1.660), inch(1.0 / 11.5), inch(1.5000), "NPT", Some("1-1/4-11.5 NPT")),
    tap("1-1/2 NPT", inch(1.900), inch(1.0 / 11.5), inch(1.7344), "NPT", Some("1-1/2-11.5 NPT")),
    tap("2 NPT", inch(2.375), inch(1.0 / 11.5), inch(2.2188), "NPT", Some("2-11.5 NPT")),
];

// ===========================================================================
// FASTENER TABLE CATALOG PER (HoleStandard, HoleType)
// ===========================================================================

const ISO_CBORE_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "iso_4762_shcs",
        label: "Hex Socket Head Cap Screw - ISO 4762",
        citation: "ISO 4762 / DIN 974-1 / ISO 273",
        sizes: ISO_4762_SHCS_CBORE,
    },
    FastenerTable {
        id: "iso_4014_hex",
        label: "Hex Bolt - ISO 4014",
        citation: "ISO 4014 / ISO 273",
        sizes: ISO_4014_HEX_CBORE,
    },
];
const ISO_CSK_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "iso_10642_csk",
    label: "Hex Socket Countersunk Head Screw - ISO 10642",
    citation: "ISO 10642 / DIN 74 Form F / ISO 273",
    sizes: ISO_10642_CSK_90,
}];
const ISO_SIMPLE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "iso_273_clearance",
    label: "Screw Clearances - ISO 273",
    citation: "ISO 273 (Fine / Medium / Coarse)",
    sizes: ISO_273_CLEARANCE,
}];
const ISO_TAP_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "iso_262_coarse",
        label: "Tapped Hole (Coarse) - ISO 262",
        citation: "ISO 262 / ISO 2306",
        sizes: ISO_METRIC_COARSE_TAP,
    },
    FastenerTable {
        id: "iso_262_fine",
        label: "Tapped Hole (Fine) - ISO 262",
        citation: "ISO 262",
        sizes: ISO_METRIC_FINE_TAP,
    },
];
const ISO_PIPE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "iso_7_1_bspt",
    label: "Tapered Pipe Tap - ISO 7-1 (Rc)",
    citation: "ISO 7-1 / BS 21",
    sizes: ISO_7_1_BSPT_PIPE_TAP,
}];

const ANSI_METRIC_CBORE_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "asme_b18_3_1m_shcs",
        label: "Socket Head Cap Screw - ASME B18.3.1M",
        citation: "ASME B18.3.1M / ASME B18.2.8",
        sizes: ASME_B18_3_1M_SHCS_CBORE,
    },
    FastenerTable {
        id: "asme_b18_2_3_5m_hex",
        label: "Hex Bolt - ASME B18.2.3.5M",
        citation: "ASME B18.2.3.5M / ASME B18.2.8",
        sizes: ISO_4014_HEX_CBORE,
    },
];
const ANSI_METRIC_CSK_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "asme_b18_3_5m_csk",
    label: "Flat Head Socket Cap Screw - ASME B18.3.5M",
    citation: "ASME B18.3.5M / ASME B18.2.8",
    sizes: ISO_10642_CSK_90,
}];
const ANSI_METRIC_SIMPLE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "asme_b18_2_8_metric",
    label: "Screw Clearances - ASME B18.2.8 (Metric)",
    citation: "ASME B18.2.8 (Metric Series)",
    sizes: ISO_273_CLEARANCE,
}];
const ANSI_METRIC_TAP_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "asme_b1_13m_coarse",
        label: "Tapped Hole (Coarse) - ASME B1.13M",
        citation: "ASME B1.13M / ISO 2306",
        sizes: ISO_METRIC_COARSE_TAP,
    },
    FastenerTable {
        id: "asme_b1_13m_fine",
        label: "Tapped Hole (Fine) - ASME B1.13M",
        citation: "ASME B1.13M",
        sizes: ISO_METRIC_FINE_TAP,
    },
];
const ANSI_METRIC_PIPE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "asme_b1_20_1_npt_m",
    label: "Tapered Pipe Tap - ASME B1.20.1 (NPT)",
    citation: "ASME B1.20.1",
    sizes: ASME_B1_20_1_NPT_PIPE_TAP,
}];

const ANSI_INCH_CBORE_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "asme_b18_3_shcs",
        label: "Socket Head Cap Screw - ASME B18.3",
        citation: "ASME B18.3 / ASME B18.2.8",
        sizes: ASME_B18_3_SHCS_CBORE,
    },
    FastenerTable {
        id: "asme_b18_2_1_hex",
        label: "Hex Cap Screw - ASME B18.2.1",
        citation: "ASME B18.2.1 / ASME B18.2.8",
        sizes: ASME_B18_2_1_HEX_CBORE,
    },
];
const ANSI_INCH_CSK_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "asme_b18_3_csk_82",
    label: "Flat Head Socket Cap Screw (82°) - ASME B18.3",
    citation: "ASME B18.3 / ASME B18.2.8",
    sizes: ASME_B18_3_CSK_82,
}];
const ANSI_INCH_SIMPLE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "asme_b18_2_8_inch",
    label: "Screw Clearances - ASME B18.2.8 (Inch)",
    citation: "ASME B18.2.8 (Inch Series)",
    sizes: ASME_B18_2_8_INCH_CLEARANCE,
}];
const ANSI_INCH_TAP_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "asme_b1_1_unc",
        label: "Tapped Hole (UNC) - ASME B1.1",
        citation: "ASME B1.1 (UNC, 75% thread)",
        sizes: ASME_B1_1_UNC_TAP,
    },
    FastenerTable {
        id: "asme_b1_1_unf",
        label: "Tapped Hole (UNF) - ASME B1.1",
        citation: "ASME B1.1 (UNF, 75% thread)",
        sizes: ASME_B1_1_UNF_TAP,
    },
];
const ANSI_INCH_PIPE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "asme_b1_20_1_npt",
    label: "Tapered Pipe Tap - ASME B1.20.1 (NPT)",
    citation: "ASME B1.20.1",
    sizes: ASME_B1_20_1_NPT_PIPE_TAP,
}];

const DIN_CBORE_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "din_912_shcs",
        label: "Socket Head Cap Screw - DIN 912 / DIN 974-1",
        citation: "DIN 912 / DIN 974-1 / DIN EN 20273",
        sizes: ISO_4762_SHCS_CBORE,
    },
    FastenerTable {
        id: "din_931_hex",
        label: "Hex Bolt - DIN 931",
        citation: "DIN 931 / DIN EN 20273",
        sizes: DIN_931_HEX_CBORE,
    },
];
const DIN_CSK_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "din_7991_csk",
        label: "Countersunk Socket Head Screw - DIN 7991",
        citation: "DIN 7991 / DIN 74",
        sizes: DIN_7991_CSK_90,
    },
    FastenerTable {
        id: "din_74_form_f",
        label: "Countersunk Socket Head Screw - DIN 74 Form F",
        citation: "DIN 74 Form F / ISO 10642",
        sizes: ISO_10642_CSK_90,
    },
];
const DIN_SIMPLE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "din_en_20273",
    label: "Screw Clearances - DIN EN 20273",
    citation: "DIN EN 20273 (ISO 273)",
    sizes: ISO_273_CLEARANCE,
}];
const DIN_TAP_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "din_13_1_coarse",
        label: "Tapped Hole (Coarse) - DIN 13-1",
        citation: "DIN 13-1 / DIN 336",
        sizes: ISO_METRIC_COARSE_TAP,
    },
    FastenerTable {
        id: "din_13_fine",
        label: "Tapped Hole (Fine) - DIN 13",
        citation: "DIN 13 / DIN 336",
        sizes: ISO_METRIC_FINE_TAP,
    },
];
const DIN_PIPE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "din_2999_pipe",
    label: "Whitworth Pipe Tap - DIN 2999",
    citation: "DIN 2999 / ISO 7-1",
    sizes: ISO_7_1_BSPT_PIPE_TAP,
}];

const JIS_CBORE_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "jis_b1176_shcs",
        label: "Socket Head Cap Screw - JIS B 1176",
        citation: "JIS B 1176 / JIS B 1001",
        sizes: JIS_B1176_SHCS_CBORE,
    },
    FastenerTable {
        id: "jis_b1180_hex",
        label: "Hex Bolt - JIS B 1180",
        citation: "JIS B 1180 / JIS B 1001",
        sizes: ISO_4014_HEX_CBORE,
    },
];
const JIS_CSK_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "jis_b1194_csk",
    label: "Countersunk Socket Head Screw - JIS B 1194",
    citation: "JIS B 1194 / JIS B 1001",
    sizes: ISO_10642_CSK_90,
}];
const JIS_SIMPLE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "jis_b1001",
    label: "Screw Clearances - JIS B 1001",
    citation: "JIS B 1001 (Class 1 / Class 2 / Class 3)",
    sizes: ISO_273_CLEARANCE,
}];
const JIS_TAP_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "jis_b0205_coarse",
        label: "Tapped Hole (Coarse) - JIS B 0205",
        citation: "JIS B 0205",
        sizes: ISO_METRIC_COARSE_TAP,
    },
    FastenerTable {
        id: "jis_b0205_fine",
        label: "Tapped Hole (Fine) - JIS B 0205",
        citation: "JIS B 0205",
        sizes: ISO_METRIC_FINE_TAP,
    },
];
const JIS_PIPE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "jis_b0203_pt",
    label: "Tapered Pipe Tap - JIS B 0203 (PT)",
    citation: "JIS B 0203 / ISO 7-1",
    sizes: ISO_7_1_BSPT_PIPE_TAP,
}];

const AS_CBORE_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "as_1420_shcs",
        label: "Socket Head Cap Screw - AS 1420 / AS 1421",
        citation: "AS 1420 / AS 1421",
        sizes: ISO_4762_SHCS_CBORE,
    },
    FastenerTable {
        id: "as_1110_hex",
        label: "Hex Bolt - AS 1110.1",
        citation: "AS 1110.1 / AS 1420",
        sizes: ISO_4014_HEX_CBORE,
    },
];
const AS_CSK_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "as_1427_csk",
    label: "Countersunk Socket Head Screw - AS 1427",
    citation: "AS 1427 / AS 1420",
    sizes: ISO_10642_CSK_90,
}];
const AS_SIMPLE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "as_1420_clr",
    label: "Screw Clearances - AS 1420",
    citation: "AS 1420 (ISO 273)",
    sizes: ISO_273_CLEARANCE,
}];
const AS_TAP_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "as_1275_coarse",
        label: "Tapped Hole (Coarse) - AS 1275",
        citation: "AS 1275",
        sizes: ISO_METRIC_COARSE_TAP,
    },
    FastenerTable {
        id: "as_1275_fine",
        label: "Tapped Hole (Fine) - AS 1275",
        citation: "AS 1275",
        sizes: ISO_METRIC_FINE_TAP,
    },
];
const AS_PIPE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "as_1722_pipe",
    label: "Tapered Pipe Tap - AS 1722.1",
    citation: "AS 1722.1 / ISO 7-1",
    sizes: ISO_7_1_BSPT_PIPE_TAP,
}];

const BSI_CBORE_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "bs_4168_shcs",
        label: "Socket Head Cap Screw - BS 4168-1",
        citation: "BS 4168-1 / BS 4186",
        sizes: ISO_4762_SHCS_CBORE,
    },
    FastenerTable {
        id: "bs_3692_hex",
        label: "Hex Bolt - BS 3692",
        citation: "BS 3692 / BS 4186",
        sizes: ISO_4014_HEX_CBORE,
    },
];
const BSI_CSK_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "bs_4168_3_csk",
    label: "Countersunk Socket Head Screw - BS 4168-3",
    citation: "BS 4168-3 / BS 4186",
    sizes: ISO_10642_CSK_90,
}];
const BSI_SIMPLE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "bs_4186_clr",
    label: "Screw Clearances - BS 4186",
    citation: "BS 4186 (ISO 273)",
    sizes: ISO_273_CLEARANCE,
}];
const BSI_TAP_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "bs_3643_coarse",
        label: "Tapped Hole (Coarse) - BS 3643",
        citation: "BS 3643",
        sizes: ISO_METRIC_COARSE_TAP,
    },
    FastenerTable {
        id: "bs_3643_fine",
        label: "Tapped Hole (Fine) - BS 3643",
        citation: "BS 3643",
        sizes: ISO_METRIC_FINE_TAP,
    },
];
const BSI_PIPE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "bs_21_bspt",
    label: "BSPT Tapered Pipe Tap - BS 21",
    citation: "BS 21 / ISO 7-1",
    sizes: ISO_7_1_BSPT_PIPE_TAP,
}];

const GB_CBORE_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "gb_70_1_shcs",
        label: "Socket Head Cap Screw - GB/T 70.1 / GB/T 152.2",
        citation: "GB/T 70.1 / GB/T 152.2 / GB/T 5277",
        sizes: GB_152_2_SHCS_CBORE,
    },
    FastenerTable {
        id: "gb_5782_hex",
        label: "Hex Bolt - GB/T 5782",
        citation: "GB/T 5782 / GB/T 5277",
        sizes: ISO_4014_HEX_CBORE,
    },
];
const GB_CSK_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "gb_70_3_csk",
    label: "Countersunk Socket Head Screw - GB/T 70.3 / GB/T 152.3",
    citation: "GB/T 70.3 / GB/T 152.3 / GB/T 5277",
    sizes: ISO_10642_CSK_90,
}];
const GB_SIMPLE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "gb_5277_clr",
    label: "Screw Clearances - GB/T 5277",
    citation: "GB/T 5277 (ISO 273)",
    sizes: ISO_273_CLEARANCE,
}];
const GB_TAP_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "gb_196_coarse",
        label: "Tapped Hole (Coarse) - GB/T 196",
        citation: "GB/T 196",
        sizes: ISO_METRIC_COARSE_TAP,
    },
    FastenerTable {
        id: "gb_196_fine",
        label: "Tapped Hole (Fine) - GB/T 196",
        citation: "GB/T 196",
        sizes: ISO_METRIC_FINE_TAP,
    },
];
const GB_PIPE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "gb_7306_1_pipe",
    label: "55° Tapered Pipe Tap - GB/T 7306.1",
    citation: "GB/T 7306.1 / ISO 7-1",
    sizes: ISO_7_1_BSPT_PIPE_TAP,
}];

const KS_CBORE_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "ks_b1003_shcs",
        label: "Socket Head Cap Screw - KS B 1003",
        citation: "KS B 1003 / KS B ISO 273",
        sizes: ISO_4762_SHCS_CBORE,
    },
    FastenerTable {
        id: "ks_b1002_hex",
        label: "Hex Bolt - KS B 1002",
        citation: "KS B 1002 / KS B ISO 273",
        sizes: ISO_4014_HEX_CBORE,
    },
];
const KS_CSK_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "ks_b1046_csk",
    label: "Countersunk Socket Head Screw - KS B 1046",
    citation: "KS B 1046 / KS B ISO 273",
    sizes: ISO_10642_CSK_90,
}];
const KS_SIMPLE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "ks_iso_273_clr",
    label: "Screw Clearances - KS B ISO 273",
    citation: "KS B ISO 273",
    sizes: ISO_273_CLEARANCE,
}];
const KS_TAP_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "ks_b0201_coarse",
        label: "Tapped Hole (Coarse) - KS B 0201",
        citation: "KS B 0201",
        sizes: ISO_METRIC_COARSE_TAP,
    },
    FastenerTable {
        id: "ks_b0204_fine",
        label: "Tapped Hole (Fine) - KS B 0204",
        citation: "KS B 0204",
        sizes: ISO_METRIC_FINE_TAP,
    },
];
const KS_PIPE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "ks_b0222_pipe",
    label: "Tapered Pipe Tap - KS B 0222",
    citation: "KS B 0222 / ISO 7-1",
    sizes: ISO_7_1_BSPT_PIPE_TAP,
}];

const IS_CBORE_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "is_2269_shcs",
        label: "Socket Head Cap Screw - IS 2269 / IS 3406",
        citation: "IS 2269 / IS 3406 / IS 1821",
        sizes: ISO_4762_SHCS_CBORE,
    },
    FastenerTable {
        id: "is_1364_hex",
        label: "Hex Bolt - IS 1364-1",
        citation: "IS 1364-1 / IS 1821",
        sizes: ISO_4014_HEX_CBORE,
    },
];
const IS_CSK_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "is_6761_csk",
    label: "Countersunk Socket Head Screw - IS 6761",
    citation: "IS 6761 / IS 1821",
    sizes: ISO_10642_CSK_90,
}];
const IS_SIMPLE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "is_1821_clr",
    label: "Screw Clearances - IS 1821",
    citation: "IS 1821 (ISO 273)",
    sizes: ISO_273_CLEARANCE,
}];
const IS_TAP_TABLES: &[FastenerTable] = &[
    FastenerTable {
        id: "is_4218_coarse",
        label: "Tapped Hole (Coarse) - IS 4218",
        citation: "IS 4218",
        sizes: ISO_METRIC_COARSE_TAP,
    },
    FastenerTable {
        id: "is_4218_fine",
        label: "Tapped Hole (Fine) - IS 4218",
        citation: "IS 4218",
        sizes: ISO_METRIC_FINE_TAP,
    },
];
const IS_PIPE_TABLES: &[FastenerTable] = &[FastenerTable {
    id: "is_554_pipe",
    label: "Tapered Pipe Tap - IS 554",
    citation: "IS 554 / ISO 7-1",
    sizes: ISO_7_1_BSPT_PIPE_TAP,
}];

/// Returns the fastener dimension tables available for `(standard, hole_type)`.
pub fn fastener_tables(standard: HoleStandard, hole_type: HoleType) -> &'static [FastenerTable] {
    match (standard, hole_type) {
        (HoleStandard::Iso, HoleType::Counterbore | HoleType::CounterboreSlot) => ISO_CBORE_TABLES,
        (HoleStandard::Iso, HoleType::Countersink | HoleType::CountersinkSlot) => ISO_CSK_TABLES,
        (HoleStandard::Iso, HoleType::Simple | HoleType::Slot) => ISO_SIMPLE_TABLES,
        (HoleStandard::Iso, HoleType::StraightTap) => ISO_TAP_TABLES,
        (HoleStandard::Iso, HoleType::TaperedTap) => ISO_PIPE_TABLES,

        (HoleStandard::AnsiMetric, HoleType::Counterbore | HoleType::CounterboreSlot) => ANSI_METRIC_CBORE_TABLES,
        (HoleStandard::AnsiMetric, HoleType::Countersink | HoleType::CountersinkSlot) => ANSI_METRIC_CSK_TABLES,
        (HoleStandard::AnsiMetric, HoleType::Simple | HoleType::Slot) => ANSI_METRIC_SIMPLE_TABLES,
        (HoleStandard::AnsiMetric, HoleType::StraightTap) => ANSI_METRIC_TAP_TABLES,
        (HoleStandard::AnsiMetric, HoleType::TaperedTap) => ANSI_METRIC_PIPE_TABLES,

        (HoleStandard::AnsiInch, HoleType::Counterbore | HoleType::CounterboreSlot) => ANSI_INCH_CBORE_TABLES,
        (HoleStandard::AnsiInch, HoleType::Countersink | HoleType::CountersinkSlot) => ANSI_INCH_CSK_TABLES,
        (HoleStandard::AnsiInch, HoleType::Simple | HoleType::Slot) => ANSI_INCH_SIMPLE_TABLES,
        (HoleStandard::AnsiInch, HoleType::StraightTap) => ANSI_INCH_TAP_TABLES,
        (HoleStandard::AnsiInch, HoleType::TaperedTap) => ANSI_INCH_PIPE_TABLES,

        (HoleStandard::Din, HoleType::Counterbore | HoleType::CounterboreSlot) => DIN_CBORE_TABLES,
        (HoleStandard::Din, HoleType::Countersink | HoleType::CountersinkSlot) => DIN_CSK_TABLES,
        (HoleStandard::Din, HoleType::Simple | HoleType::Slot) => DIN_SIMPLE_TABLES,
        (HoleStandard::Din, HoleType::StraightTap) => DIN_TAP_TABLES,
        (HoleStandard::Din, HoleType::TaperedTap) => DIN_PIPE_TABLES,

        (HoleStandard::Jis, HoleType::Counterbore | HoleType::CounterboreSlot) => JIS_CBORE_TABLES,
        (HoleStandard::Jis, HoleType::Countersink | HoleType::CountersinkSlot) => JIS_CSK_TABLES,
        (HoleStandard::Jis, HoleType::Simple | HoleType::Slot) => JIS_SIMPLE_TABLES,
        (HoleStandard::Jis, HoleType::StraightTap) => JIS_TAP_TABLES,
        (HoleStandard::Jis, HoleType::TaperedTap) => JIS_PIPE_TABLES,

        (HoleStandard::As, HoleType::Counterbore | HoleType::CounterboreSlot) => AS_CBORE_TABLES,
        (HoleStandard::As, HoleType::Countersink | HoleType::CountersinkSlot) => AS_CSK_TABLES,
        (HoleStandard::As, HoleType::Simple | HoleType::Slot) => AS_SIMPLE_TABLES,
        (HoleStandard::As, HoleType::StraightTap) => AS_TAP_TABLES,
        (HoleStandard::As, HoleType::TaperedTap) => AS_PIPE_TABLES,

        (HoleStandard::Bsi, HoleType::Counterbore | HoleType::CounterboreSlot) => BSI_CBORE_TABLES,
        (HoleStandard::Bsi, HoleType::Countersink | HoleType::CountersinkSlot) => BSI_CSK_TABLES,
        (HoleStandard::Bsi, HoleType::Simple | HoleType::Slot) => BSI_SIMPLE_TABLES,
        (HoleStandard::Bsi, HoleType::StraightTap) => BSI_TAP_TABLES,
        (HoleStandard::Bsi, HoleType::TaperedTap) => BSI_PIPE_TABLES,

        (HoleStandard::Gb, HoleType::Counterbore | HoleType::CounterboreSlot) => GB_CBORE_TABLES,
        (HoleStandard::Gb, HoleType::Countersink | HoleType::CountersinkSlot) => GB_CSK_TABLES,
        (HoleStandard::Gb, HoleType::Simple | HoleType::Slot) => GB_SIMPLE_TABLES,
        (HoleStandard::Gb, HoleType::StraightTap) => GB_TAP_TABLES,
        (HoleStandard::Gb, HoleType::TaperedTap) => GB_PIPE_TABLES,

        (HoleStandard::Ks, HoleType::Counterbore | HoleType::CounterboreSlot) => KS_CBORE_TABLES,
        (HoleStandard::Ks, HoleType::Countersink | HoleType::CountersinkSlot) => KS_CSK_TABLES,
        (HoleStandard::Ks, HoleType::Simple | HoleType::Slot) => KS_SIMPLE_TABLES,
        (HoleStandard::Ks, HoleType::StraightTap) => KS_TAP_TABLES,
        (HoleStandard::Ks, HoleType::TaperedTap) => KS_PIPE_TABLES,

        (HoleStandard::Is, HoleType::Counterbore | HoleType::CounterboreSlot) => IS_CBORE_TABLES,
        (HoleStandard::Is, HoleType::Countersink | HoleType::CountersinkSlot) => IS_CSK_TABLES,
        (HoleStandard::Is, HoleType::Simple | HoleType::Slot) => IS_SIMPLE_TABLES,
        (HoleStandard::Is, HoleType::StraightTap) => IS_TAP_TABLES,
        (HoleStandard::Is, HoleType::TaperedTap) => IS_PIPE_TABLES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-4
    }

    fn find(std: HoleStandard, ty: HoleType, fastener: &str, size: &str) -> &'static HoleSizeRow {
        let table = fastener_tables(std, ty)
            .iter()
            .find(|t| t.id == fastener)
            .unwrap_or_else(|| panic!("missing fastener {fastener} in {std:?}/{ty:?}"));
        table
            .sizes
            .iter()
            .find(|r| r.name == size)
            .unwrap_or_else(|| panic!("missing size {size} in {fastener}"))
    }

    #[test]
    fn every_standard_and_type_satisfies_geometric_invariants() {
        for std in HoleStandard::ALL {
            assert!(!std.source_summary().is_empty());
            for ty in HoleType::ALL {
                let tables = fastener_tables(std, ty);
                assert!(!tables.is_empty(), "{std:?}/{ty:?} has no tables");
                for table in tables {
                    assert!(!table.citation.is_empty(), "{}", table.id);
                    assert!(!table.sizes.is_empty(), "{}", table.id);
                    for row in table.sizes {
                        assert!(row.nominal_mm > 0.0, "{}/{}", table.id, row.name);
                        assert!(row.close_mm > 0.0 && row.close_mm <= row.normal_mm + 1e-9);
                        assert!(row.normal_mm <= row.loose_mm + 1e-9);
                        if ty.has_counterbore() {
                            let d = row.cbore_diameter_mm.expect("counterbore diameter");
                            let h = row.cbore_depth_mm.expect("counterbore depth");
                            assert!(d > row.loose_mm, "{}/{} cbore {d} <= loose {}", table.id, row.name, row.loose_mm);
                            assert!(h > 0.0, "{}/{} cbore depth", table.id, row.name);
                        }
                        if ty.has_countersink() {
                            let d = row.csk_diameter_mm.expect("countersink diameter");
                            let a = row.csk_angle_deg.expect("countersink angle");
                            assert!(d > row.loose_mm, "{}/{} csk {d} <= loose {}", table.id, row.name, row.loose_mm);
                            assert!(a > 0.0 && a < 180.0);
                        }
                        if ty.is_tapped() {
                            let drill = row.tap_drill_mm.expect("tap drill");
                            let pitch = row.pitch_mm.expect("pitch");
                            assert!(drill > 0.0 && drill < row.nominal_mm, "{}/{}", table.id, row.name);
                            assert!(pitch > 0.0);
                            let thread = row.hole_thread(None).expect("hole thread");
                            assert_eq!(
                                thread.problem(drill, &HoleDepth::Through, None),
                                None,
                                "{}/{} {}",
                                table.id,
                                row.name,
                                thread.designation()
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sampled_iso_din_jis_as_bsi_gb_ks_is_sizes_match_published_standards() {
        // ISO 273 clearance holes: M6 Fine=6.4, Medium=6.6, Coarse=7.0; M12 Medium=13.5
        let iso_m6 = find(HoleStandard::Iso, HoleType::Simple, "iso_273_clearance", "M6");
        assert_eq!((iso_m6.close_mm, iso_m6.normal_mm, iso_m6.loose_mm), (6.4, 6.6, 7.0));
        let iso_m12 = find(HoleStandard::Iso, HoleType::Simple, "iso_273_clearance", "M12");
        assert_eq!((iso_m12.close_mm, iso_m12.normal_mm, iso_m12.loose_mm), (13.0, 13.5, 14.5));

        // ISO 4762 / DIN 974-1 SHCS counterbore: M6 -> Ø11.0 x 6.4, M10 -> Ø18.0 x 10.6
        let shcs_m6 = find(HoleStandard::Iso, HoleType::Counterbore, "iso_4762_shcs", "M6");
        assert_eq!(shcs_m6.cbore_diameter_mm, Some(11.0));
        assert_eq!(shcs_m6.cbore_depth_mm, Some(6.4));
        let shcs_m10 = find(HoleStandard::Din, HoleType::Counterbore, "din_912_shcs", "M10");
        assert_eq!(shcs_m10.cbore_diameter_mm, Some(18.0));
        assert_eq!(shcs_m10.cbore_depth_mm, Some(10.6));

        // ISO 10642 / DIN 74 Form F 90° countersink: M6 -> Ø13.71 x 90°, M8 -> Ø18.25 x 90°
        let csk_m6 = find(HoleStandard::Iso, HoleType::Countersink, "iso_10642_csk", "M6");
        assert_eq!(csk_m6.csk_diameter_mm, Some(13.71));
        assert_eq!(csk_m6.csk_angle_deg, Some(90.0));
        let csk_m8 = find(HoleStandard::Iso, HoleType::Countersink, "iso_10642_csk", "M8");
        assert_eq!(csk_m8.csk_diameter_mm, Some(18.25));

        // DIN 7991 classic countersink M8 -> Ø16.5 (distinct from ISO 10642 Ø18.25)
        let din_csk_m8 = find(HoleStandard::Din, HoleType::Countersink, "din_7991_csk", "M8");
        assert_eq!(din_csk_m8.csk_diameter_mm, Some(16.5));

        // ISO 262 / ISO 2306 metric tap drills: M6x1 -> Ø5.0, M8x1.25 -> Ø6.8, M12x1.75 -> Ø10.2
        assert_eq!(find(HoleStandard::Iso, HoleType::StraightTap, "iso_262_coarse", "M6").tap_drill_mm, Some(5.0));
        assert_eq!(find(HoleStandard::Iso, HoleType::StraightTap, "iso_262_coarse", "M8").tap_drill_mm, Some(6.8));
        assert_eq!(find(HoleStandard::Iso, HoleType::StraightTap, "iso_262_coarse", "M12").tap_drill_mm, Some(10.2));

        // JIS B 1176 SHCS counterbore: M5 -> Ø9.5, M8 -> Ø14.0, M10 -> Ø17.5
        assert_eq!(find(HoleStandard::Jis, HoleType::Counterbore, "jis_b1176_shcs", "M5").cbore_diameter_mm, Some(9.5));
        assert_eq!(find(HoleStandard::Jis, HoleType::Counterbore, "jis_b1176_shcs", "M8").cbore_diameter_mm, Some(14.0));
        assert_eq!(
            find(HoleStandard::Jis, HoleType::Counterbore, "jis_b1176_shcs", "M10").cbore_diameter_mm,
            Some(17.5)
        );

        // ISO 7-1 / BS 21 BSPT tapered pipe tap drills: 1/4 -> 11.2 mm, 1/2 -> 18.25 mm
        assert_eq!(find(HoleStandard::Bsi, HoleType::TaperedTap, "bs_21_bspt", "1/4").tap_drill_mm, Some(11.2));
        assert_eq!(find(HoleStandard::As, HoleType::TaperedTap, "as_1722_pipe", "1/2").tap_drill_mm, Some(18.25));

        // GB/T 152.2, KS B 1003, IS 2269 M6 SHCS counterbore -> Ø11.0
        assert_eq!(find(HoleStandard::Gb, HoleType::Counterbore, "gb_70_1_shcs", "M6").cbore_diameter_mm, Some(11.0));
        assert_eq!(find(HoleStandard::Ks, HoleType::Counterbore, "ks_b1003_shcs", "M6").cbore_diameter_mm, Some(11.0));
        assert_eq!(find(HoleStandard::Is, HoleType::Counterbore, "is_2269_shcs", "M6").cbore_diameter_mm, Some(11.0));
    }

    #[test]
    fn sampled_ansi_inch_and_metric_sizes_match_asme_standards() {
        // ASME B18.2.8 inch clearance: 1/4" Close=17/64" (0.2656"), Normal=9/32" (0.2812"), Loose=19/64" (0.2969")
        let qtr = find(HoleStandard::AnsiInch, HoleType::Simple, "asme_b18_2_8_inch", "1/4");
        assert!(approx(qtr.close_mm, 0.2656 * 25.4));
        assert!(approx(qtr.normal_mm, 0.2812 * 25.4));
        assert!(approx(qtr.loose_mm, 0.2969 * 25.4));

        // ASME B18.3 inch SHCS counterbore: 1/4" -> 7/16" (0.4375") dia x 0.250" deep
        let shcs_qtr = find(HoleStandard::AnsiInch, HoleType::Counterbore, "asme_b18_3_shcs", "1/4");
        assert!(approx(shcs_qtr.cbore_diameter_mm.unwrap(), 0.4375 * 25.4));
        assert!(approx(shcs_qtr.cbore_depth_mm.unwrap(), 0.250 * 25.4));

        // ASME B18.3 inch 82° flat countersunk: 1/4" -> 0.531" dia x 82°
        let csk_qtr = find(HoleStandard::AnsiInch, HoleType::Countersink, "asme_b18_3_csk_82", "1/4");
        assert!(approx(csk_qtr.csk_diameter_mm.unwrap(), 0.531 * 25.4));
        assert_eq!(csk_qtr.csk_angle_deg, Some(82.0));

        // ASME B1.1 UNC tap drill: 1/4-20 -> #7 drill (0.2010" = 5.1054 mm), 1/2-13 -> 27/64" (0.4219")
        let unc_qtr = find(HoleStandard::AnsiInch, HoleType::StraightTap, "asme_b1_1_unc", "1/4-20");
        assert!(approx(unc_qtr.tap_drill_mm.unwrap(), 0.2010 * 25.4));
        assert_eq!(unc_qtr.hole_thread(None).unwrap().designation(), "1/4-20 UNC-2B");
        let unc_half = find(HoleStandard::AnsiInch, HoleType::StraightTap, "asme_b1_1_unc", "1/2-13");
        assert!(approx(unc_half.tap_drill_mm.unwrap(), 0.4219 * 25.4));

        // ASME B1.20.1 NPT tap drill: 1/4 NPT -> 7/16" (0.4375"), 1/2 NPT -> 23/32" (0.7188")
        let npt_qtr = find(HoleStandard::AnsiInch, HoleType::TaperedTap, "asme_b1_20_1_npt", "1/4 NPT");
        assert!(approx(npt_qtr.tap_drill_mm.unwrap(), 0.4375 * 25.4));
        let npt_half = find(HoleStandard::AnsiInch, HoleType::TaperedTap, "asme_b1_20_1_npt", "1/2 NPT");
        assert!(approx(npt_half.tap_drill_mm.unwrap(), 0.7188 * 25.4));

        // ASME B18.3.1M metric SHCS: M20 -> 31.5 mm counterbore
        let am_m20 = find(HoleStandard::AnsiMetric, HoleType::Counterbore, "asme_b18_3_1m_shcs", "M20");
        assert_eq!(am_m20.cbore_diameter_mm, Some(31.5));
        // ASME B18.2.3.5M hex bolt M14 (as shown in reference): normal 15.5, depth 9.25
        let am_hex_m14 = find(HoleStandard::AnsiMetric, HoleType::Counterbore, "asme_b18_2_3_5m_hex", "M14");
        assert_eq!(am_hex_m14.normal_mm, 15.5);
        assert_eq!(am_hex_m14.cbore_depth_mm, Some(9.25));
    }

    #[test]
    fn custom_sizing_and_restore_defaults_and_callout_tolerances() {
        let mut spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::Counterbore);
        assert_eq!(spec.size, "M6");
        let defaults = spec.resolve_defaults();
        assert_eq!(defaults.diameter, 6.6);
        assert_eq!(defaults.head, Some(HoleHead::Counterbore { diameter: 11.0, depth: 6.4 }));

        spec.fit = HoleFit::Close;
        assert_eq!(spec.resolve_defaults().diameter, 6.4);
        spec.fit = HoleFit::Loose;
        assert_eq!(spec.resolve_defaults().diameter, 7.0);

        spec.set_standard(HoleStandard::AnsiInch);
        assert_eq!(spec.size, "1/4");
        assert!(approx(spec.resolve_defaults().diameter, 0.2969 * 25.4));

        let tol = CalloutTolerance {
            param: CalloutParam::CounterboreDepth,
            mode: ToleranceMode::Bilateral { upper: 0.1, lower: -0.05 },
            precision: 2,
        };
        assert_eq!(tol.format_value(6.4), "6.40 +0.10/-0.05");
        let fit_tol = CalloutTolerance {
            param: CalloutParam::Diameter,
            mode: ToleranceMode::Fit { class: "H13".into() },
            precision: 1,
        };
        assert_eq!(fit_tol.format_value(6.6), "6.6 H13");
    }
}
