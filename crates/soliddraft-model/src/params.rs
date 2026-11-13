//! The numbers typed into features, listed so the values table can show and drive them.
//!
//! Every parameter has a stable identity, an [`Input`], and a short label such as
//! `Depth` or `D2`. The table names it after its feature, as in `BossExtrude1.Depth`.

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::{Grade, HoleDepth, HoleHead};
use soliddraft_sketch::ConstraintKind;

use crate::values::Input;
use crate::{BuiltinShape, EndCondition, Feature, FeatureKind, PatternSpec, ProfileSource, SketchPlane};

/// A numeric field of a feature other than a sketch dimension or an extrusion's depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Parameter {
    /// A built-in rectangle's width.
    Width,
    /// A built-in rectangle's height.
    Height,
    /// A built-in circle's or a hole's diameter.
    Diameter,
    /// A counterbore's or countersink's diameter at the entry.
    HeadDiameter,
    /// A counterbore's depth.
    HeadDepth,
    /// A countersink's full angle.
    HeadAngle,
    /// A revolve's sweep, a draft's tilt, or the angle between circular pattern copies.
    Angle,
    /// A fillet's radius, or a lattice's constant beam radius.
    Radius,
    /// A graded lattice's beam radius where the grade starts.
    RadiusStart,
    /// A graded lattice's beam radius where the grade ends.
    RadiusEnd,
    /// Where a graded lattice's radius starts, along its axis.
    RadiusStartAt,
    /// Where a graded lattice's radius ends, along its axis.
    RadiusEndAt,
    /// A gyroid's constant level.
    Level,
    /// A graded gyroid's level where the grade starts.
    LevelStart,
    /// A graded gyroid's level where the grade ends.
    LevelEnd,
    /// Where a graded gyroid's level starts, along its axis.
    LevelStartAt,
    /// Where a graded gyroid's level ends, along its axis.
    LevelEndAt,
    /// The accuracy a lattice or gyroid is proven to.
    Accuracy,
    /// The smallest detail a lattice or gyroid keeps.
    MinDetail,
    /// A chamfer's distance.
    Distance,
    /// A chamfer's second setback.
    OtherDistance,
    /// A blind hole's depth.
    Depth,
    /// The distance between linear pattern copies.
    Spacing,
    /// How many copies a pattern makes, the original included.
    Count,
    /// A shell's wall.
    Thickness,
    /// A plane's distance from what it is offset from.
    Offset,
    /// A lattice's cell edge.
    CellSize,
    /// A gyroid's period.
    Period,
}

impl Parameter {
    /// The label people read, also the last part of the parameter's default name.
    pub fn label(self) -> &'static str {
        match self {
            Parameter::Width => "Width",
            Parameter::Height => "Height",
            Parameter::Diameter => "Diameter",
            Parameter::HeadDiameter => "HeadDiameter",
            Parameter::HeadDepth => "HeadDepth",
            Parameter::HeadAngle => "HeadAngle",
            Parameter::Angle => "Angle",
            Parameter::Radius => "Radius",
            Parameter::RadiusStart => "RadiusStart",
            Parameter::RadiusEnd => "RadiusEnd",
            Parameter::RadiusStartAt => "RadiusStartAt",
            Parameter::RadiusEndAt => "RadiusEndAt",
            Parameter::Level => "Level",
            Parameter::LevelStart => "LevelStart",
            Parameter::LevelEnd => "LevelEnd",
            Parameter::LevelStartAt => "LevelStartAt",
            Parameter::LevelEndAt => "LevelEndAt",
            Parameter::Accuracy => "Accuracy",
            Parameter::MinDetail => "MinDetail",
            Parameter::Distance => "Distance",
            Parameter::OtherDistance => "OtherDistance",
            Parameter::Depth => "Depth",
            Parameter::Spacing => "Spacing",
            Parameter::Count => "Count",
            Parameter::Thickness => "Thickness",
            Parameter::Offset => "Offset",
            Parameter::CellSize => "CellSize",
            Parameter::Period => "Period",
        }
    }

    /// The key a feature summary lists its expression under: the label in snake case.
    pub fn key(self) -> String {
        let mut out = String::new();
        for (i, c) in self.label().chars().enumerate() {
            if c.is_ascii_uppercase() && i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        }
        out
    }
}

/// What a parameter measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    /// Millimetres.
    Length,
    /// Degrees.
    Angle,
    /// A whole number.
    Count,
    /// A plain number, such as a gyroid's level.
    Number,
}

impl Unit {
    /// The unit's symbol, empty for a count.
    pub fn symbol(self) -> &'static str {
        match self {
            Unit::Length => "mm",
            Unit::Angle => "deg",
            Unit::Count | Unit::Number => "",
        }
    }

    pub(crate) fn dimensions(self) -> (i32, i32) {
        match self {
            Unit::Length => (1, 0),
            Unit::Angle => (0, 1),
            Unit::Count | Unit::Number => (0, 0),
        }
    }
}

/// One number a feature stores.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// Stable identity.
    pub input: Input,
    /// The label people read, such as `Depth` or `D2`.
    pub label: String,
    /// What it measures.
    pub unit: Unit,
    /// The stored number.
    pub value: f64,
}

/// The numbers `feature` stores, in the order its property panel shows them.
pub fn parameters(feature: &Feature) -> Vec<Param> {
    let id = feature.id;
    let param = |parameter: Parameter, unit: Unit, value: f64| Param {
        input: Input::Parameter { feature: id, parameter },
        label: parameter.label().to_owned(),
        unit,
        value,
    };
    let mut out = Vec::new();
    match &feature.kind {
        FeatureKind::ReferencePlane(plane) if !matches!(plane, SketchPlane::Face { .. }) => {
            out.push(param(Parameter::Offset, Unit::Length, plane.offset()))
        }
        FeatureKind::ReferencePlane(_) => {}
        FeatureKind::Sketch(sketch) => {
            if !matches!(sketch.plane, SketchPlane::Face { .. }) {
                out.push(param(Parameter::Offset, Unit::Length, sketch.plane.offset()));
            }
            let dimensions = sketch.sketch.constraints.iter().filter(|c| c.is_dimension()).enumerate();
            for (n, c) in dimensions {
                let Some(value) = c.value() else { continue };
                let unit = if matches!(c.kind, ConstraintKind::Angle { .. }) { Unit::Angle } else { Unit::Length };
                let input = Input::Dimension { feature: id, constraint: c.id };
                out.push(Param { input, label: format!("D{}", n + 1), unit, value });
            }
        }
        FeatureKind::Extrude(e) => {
            match e.profile {
                ProfileSource::Shape { shape: BuiltinShape::Rectangle { width, height }, .. } => {
                    out.push(param(Parameter::Width, Unit::Length, width));
                    out.push(param(Parameter::Height, Unit::Length, height));
                }
                ProfileSource::Shape { shape: BuiltinShape::Circle { diameter }, .. } => {
                    out.push(param(Parameter::Diameter, Unit::Length, diameter));
                }
                ProfileSource::Sketch { .. } => {}
            }
            if let EndCondition::Blind { depth } = e.end {
                out.push(Param {
                    input: Input::Depth { feature: id },
                    label: "Depth".into(),
                    unit: Unit::Length,
                    value: depth,
                });
            }
        }
        FeatureKind::Revolve(r) => out.push(param(Parameter::Angle, Unit::Angle, r.angle_deg)),
        FeatureKind::Fillet(b) => out.push(param(Parameter::Radius, Unit::Length, b.size)),
        FeatureKind::Chamfer(b) => {
            out.push(param(Parameter::Distance, Unit::Length, b.size));
            if let Some(other) = b.other_distance {
                out.push(param(Parameter::OtherDistance, Unit::Length, other));
            }
        }
        FeatureKind::Hole(h) => {
            out.push(param(Parameter::Diameter, Unit::Length, h.diameter));
            if let HoleDepth::Blind { depth } = h.depth {
                out.push(param(Parameter::Depth, Unit::Length, depth));
            }
            match h.head {
                Some(HoleHead::Counterbore { diameter, depth }) => {
                    out.push(param(Parameter::HeadDiameter, Unit::Length, diameter));
                    out.push(param(Parameter::HeadDepth, Unit::Length, depth));
                }
                Some(HoleHead::Countersink { diameter, angle_deg }) => {
                    out.push(param(Parameter::HeadDiameter, Unit::Length, diameter));
                    out.push(param(Parameter::HeadAngle, Unit::Angle, angle_deg));
                }
                None => {}
            }
        }
        FeatureKind::Pattern(p) => match p.layout {
            PatternSpec::Linear { spacing, count, .. } => {
                out.push(param(Parameter::Spacing, Unit::Length, spacing));
                out.push(param(Parameter::Count, Unit::Count, f64::from(count)));
            }
            PatternSpec::Circular { count, angle, .. } => {
                out.push(param(Parameter::Count, Unit::Count, f64::from(count)));
                out.push(param(Parameter::Angle, Unit::Angle, angle));
            }
        },
        FeatureKind::Draft(d) => out.push(param(Parameter::Angle, Unit::Angle, d.angle)),
        FeatureKind::Shell(s) => out.push(param(Parameter::Thickness, Unit::Length, s.thickness)),
        FeatureKind::Lattice(l) => {
            if let Some(size) = l.cell_size_mm {
                out.push(param(Parameter::CellSize, Unit::Length, size));
            }
            if let Some(grade) = &l.radius_mm {
                for (parameter, value) in grade_values(grade, RADIUS) {
                    let unit = Unit::Length;
                    out.push(param(parameter, unit, value));
                }
            }
            out.push(param(Parameter::Accuracy, Unit::Length, l.accuracy_mm));
            out.push(param(Parameter::MinDetail, Unit::Length, l.min_detail_mm));
        }
        FeatureKind::Gyroid(g) => {
            out.push(param(Parameter::Period, Unit::Length, g.period_mm));
            for (parameter, value) in grade_values(&g.level, LEVEL) {
                let at = matches!(parameter, Parameter::LevelStartAt | Parameter::LevelEndAt);
                out.push(param(parameter, if at { Unit::Length } else { Unit::Number }, value));
            }
            out.push(param(Parameter::Accuracy, Unit::Length, g.accuracy_mm));
            out.push(param(Parameter::MinDetail, Unit::Length, g.min_detail_mm));
        }
        FeatureKind::Loft(_) | FeatureKind::Sweep(_) | FeatureKind::Mirror(_) | FeatureKind::ImportStep(_) => {}
    }
    out
}

/// The stored number behind `input`, and its unit, if `features` still has it.
pub(crate) fn read(features: &[Feature], input: &Input) -> Option<(f64, Unit)> {
    let feature = features.iter().find(|f| f.id == input.feature())?;
    parameters(feature).into_iter().find(|p| p.input == *input).map(|p| (p.value, p.unit))
}

/// Stores `value` in `feature` as the number behind `input`. A sketch is solved again.
pub(crate) fn write(feature: &mut Feature, input: &Input, value: f64) -> Result<(), String> {
    let name = feature.name.clone();
    let positive = |what: &str| {
        if value > 0.0 { Ok(value) } else { Err(format!("{name}: the {what} must be greater than 0")) }
    };
    match (input, &mut feature.kind) {
        (Input::Depth { .. }, FeatureKind::Extrude(e)) => {
            if value <= 0.0 {
                return Err("depth must be positive".into());
            }
            e.end = EndCondition::Blind { depth: value };
        }
        (Input::Dimension { constraint, .. }, FeatureKind::Sketch(sketch)) => {
            let c =
                sketch.sketch.constraint_mut(*constraint).filter(|c| c.value().is_some()).ok_or("no such dimension")?;
            if !value.is_finite() {
                return Err(format!("{name}: the dimension must be a finite number"));
            }
            if !matches!(c.kind, soliddraft_sketch::ConstraintKind::Angle { .. }) {
                positive("dimension length")?;
            }
            if c.value() != Some(value) {
                c.set_value(value);
                soliddraft_sketch::solve::solve(&mut sketch.sketch, None, true);
                if let soliddraft_sketch::SolveStatus::Failed(e) = &sketch.sketch.status {
                    return Err(e.clone());
                }
            }
        }
        (Input::Parameter { parameter, .. }, kind) => write_parameter(kind, *parameter, value, &positive)?,
        _ => return Err(format!("{name} has no such value")),
    }
    Ok(())
}

fn write_parameter(
    kind: &mut FeatureKind,
    parameter: Parameter,
    value: f64,
    positive: &dyn Fn(&str) -> Result<f64, String>,
) -> Result<(), String> {
    let missing = || Err::<(), String>(format!("this feature has no {}", parameter.label().to_lowercase()));
    match (kind, parameter) {
        (FeatureKind::ReferencePlane(plane), Parameter::Offset) => match plane {
            SketchPlane::Feature { offset, .. }
            | SketchPlane::Datum { offset, .. }
            | SketchPlane::FaceOffset { offset, .. } => *offset = value,
            SketchPlane::Face { .. } => return missing(),
        },
        (FeatureKind::Sketch(s), Parameter::Offset) => match &mut s.plane {
            SketchPlane::Feature { offset, .. }
            | SketchPlane::Datum { offset, .. }
            | SketchPlane::FaceOffset { offset, .. } => *offset = value,
            SketchPlane::Face { .. } => return missing(),
        },
        (FeatureKind::Extrude(e), Parameter::Width | Parameter::Height | Parameter::Diameter) => {
            let ProfileSource::Shape { shape, .. } = &mut e.profile else { return missing() };
            match (shape, parameter) {
                (BuiltinShape::Rectangle { width, .. }, Parameter::Width) => *width = positive("width")?,
                (BuiltinShape::Rectangle { height, .. }, Parameter::Height) => *height = positive("height")?,
                (BuiltinShape::Circle { diameter }, Parameter::Diameter) => *diameter = positive("diameter")?,
                _ => return missing(),
            }
        }
        (FeatureKind::Revolve(r), Parameter::Angle) => {
            if !(value > 0.0 && value <= 360.0) {
                return Err("a revolve's angle must be more than 0 and at most 360 degrees".into());
            }
            r.angle_deg = value;
        }
        (FeatureKind::Fillet(b), Parameter::Radius) => b.size = positive("radius")?,
        (FeatureKind::Chamfer(b), Parameter::Distance) => b.size = positive("distance")?,
        (FeatureKind::Chamfer(b), Parameter::OtherDistance) if b.other_distance.is_some() => {
            b.other_distance = Some(positive("second distance")?);
        }
        (FeatureKind::Hole(h), Parameter::Diameter) => h.diameter = positive("diameter")?,
        (FeatureKind::Hole(h), Parameter::Depth) => match &mut h.depth {
            HoleDepth::Blind { depth } => *depth = positive("depth")?,
            HoleDepth::Through => return missing(),
        },
        (FeatureKind::Hole(h), Parameter::HeadDiameter) => match &mut h.head {
            Some(HoleHead::Counterbore { diameter, .. } | HoleHead::Countersink { diameter, .. }) => {
                *diameter = positive("head diameter")?
            }
            None => return missing(),
        },
        (FeatureKind::Hole(h), Parameter::HeadDepth) => match &mut h.head {
            Some(HoleHead::Counterbore { depth, .. }) => *depth = positive("head depth")?,
            _ => return missing(),
        },
        (FeatureKind::Hole(h), Parameter::HeadAngle) => match &mut h.head {
            Some(HoleHead::Countersink { angle_deg, .. }) => {
                if !(value > 0.0 && value < 180.0) {
                    return Err("a countersink's angle must be more than 0 and less than 180 degrees".into());
                }
                *angle_deg = value
            }
            _ => return missing(),
        },
        (
            FeatureKind::Lattice(l),
            p @ (Parameter::Radius
            | Parameter::RadiusStart
            | Parameter::RadiusEnd
            | Parameter::RadiusStartAt
            | Parameter::RadiusEndAt),
        ) => {
            let Some(grade) = &mut l.radius_mm else { return missing() };
            let at = matches!(p, Parameter::RadiusStartAt | Parameter::RadiusEndAt);
            let value = if at { value } else { positive("beam radius")? };
            if !write_grade(grade, RADIUS, p, value) {
                return missing();
            }
        }
        (
            FeatureKind::Gyroid(g),
            p @ (Parameter::Level
            | Parameter::LevelStart
            | Parameter::LevelEnd
            | Parameter::LevelStartAt
            | Parameter::LevelEndAt),
        ) => {
            if !write_grade(&mut g.level, LEVEL, p, value) {
                return missing();
            }
        }
        (FeatureKind::Lattice(l), Parameter::Accuracy) => l.accuracy_mm = positive("accuracy")?,
        (FeatureKind::Lattice(l), Parameter::MinDetail) => l.min_detail_mm = positive("smallest detail")?,
        (FeatureKind::Gyroid(g), Parameter::Accuracy) => g.accuracy_mm = positive("accuracy")?,
        (FeatureKind::Gyroid(g), Parameter::MinDetail) => g.min_detail_mm = positive("smallest detail")?,
        (FeatureKind::Pattern(p), Parameter::Count) => {
            if value.fract() != 0.0 || !(2.0..=10_000.0).contains(&value) {
                return Err("a pattern's count must be a whole number of at least 2".into());
            }
            match &mut p.layout {
                PatternSpec::Linear { count, .. } | PatternSpec::Circular { count, .. } => *count = value as u32,
            }
        }
        (FeatureKind::Pattern(p), Parameter::Spacing) => match &mut p.layout {
            PatternSpec::Linear { spacing, .. } => *spacing = positive("spacing")?,
            PatternSpec::Circular { .. } => return missing(),
        },
        (FeatureKind::Pattern(p), Parameter::Angle) => match &mut p.layout {
            PatternSpec::Circular { angle, .. } => *angle = value,
            PatternSpec::Linear { .. } => return missing(),
        },
        (FeatureKind::Draft(d), Parameter::Angle) => d.angle = value,
        (FeatureKind::Shell(s), Parameter::Thickness) => s.thickness = positive("thickness")?,
        (FeatureKind::Lattice(l), Parameter::CellSize) if l.cell_size_mm.is_some() => {
            l.cell_size_mm = Some(positive("cell size")?)
        }
        (FeatureKind::Gyroid(g), Parameter::Period) => g.period_mm = positive("period")?,
        _ => return missing(),
    }
    Ok(())
}

/// A feature name as the first part of a parameter name: letters, digits and underscores.
pub(crate) fn name_part(feature: &Feature) -> String {
    let mut out: String = feature.name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    if crate::value_expr::check_name(&out).is_err() {
        out.insert(0, 'F');
    }
    if out.len() > 100 {
        out.truncate(100);
    }
    out
}

/// A grade's parameters: the constant, or the start and end values and where they apply.
type GradeNames = [Parameter; 5];
const RADIUS: GradeNames =
    [Parameter::Radius, Parameter::RadiusStart, Parameter::RadiusEnd, Parameter::RadiusStartAt, Parameter::RadiusEndAt];
const LEVEL: GradeNames =
    [Parameter::Level, Parameter::LevelStart, Parameter::LevelEnd, Parameter::LevelStartAt, Parameter::LevelEndAt];

fn grade_values(grade: &Grade, names: GradeNames) -> Vec<(Parameter, f64)> {
    match grade {
        Grade::Constant(v) => vec![(names[0], *v)],
        Grade::Linear(l) => vec![(names[1], l.start), (names[2], l.end), (names[3], l.from), (names[4], l.to)],
    }
}

/// Sets one of a grade's numbers; false when the grade has no such number.
fn write_grade(grade: &mut Grade, names: GradeNames, parameter: Parameter, value: f64) -> bool {
    let slot = match grade {
        Grade::Constant(v) if parameter == names[0] => v,
        Grade::Linear(l) if parameter == names[1] => &mut l.start,
        Grade::Linear(l) if parameter == names[2] => &mut l.end,
        Grade::Linear(l) if parameter == names[3] => &mut l.from,
        Grade::Linear(l) if parameter == names[4] => &mut l.to,
        _ => return false,
    };
    *slot = value;
    true
}
