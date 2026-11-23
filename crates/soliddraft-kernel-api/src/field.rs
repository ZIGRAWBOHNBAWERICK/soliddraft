//! Terminal field inputs that vary over the part, and the explicit outputs a field has
//! besides its labelled mesh: certified moments of the field and a mass stand-in.

use serde::{Deserialize, Serialize};

/// A part axis a linear grade runs along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradeAxis {
    /// The part's x axis.
    X,
    /// The part's y axis.
    Y,
    /// The part's z axis.
    Z,
}

impl GradeAxis {
    /// Every axis, in the order the editor offers them.
    pub const ALL: [GradeAxis; 3] = [GradeAxis::X, GradeAxis::Y, GradeAxis::Z];

    /// The axis's index in a coordinate triple.
    pub fn index(self) -> usize {
        match self {
            GradeAxis::X => 0,
            GradeAxis::Y => 1,
            GradeAxis::Z => 2,
        }
    }

    /// The lowercase name files and tools use.
    pub fn name(self) -> &'static str {
        match self {
            GradeAxis::X => "x",
            GradeAxis::Y => "y",
            GradeAxis::Z => "z",
        }
    }
}

/// A value graded linearly along one part axis: `start` at part coordinate `from` on
/// `axis`, `end` at `to`, linear in between and clamped beyond. The anchors are part
/// coordinates, so an upstream size edit does not move them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearGrade {
    /// The part axis the value changes along.
    pub axis: GradeAxis,
    /// Coordinate on `axis` where the value is `start`.
    pub from: f64,
    /// Coordinate on `axis` where the value is `end`.
    pub to: f64,
    /// The value at `from` and before it.
    pub start: f64,
    /// The value at `to` and beyond it.
    pub end: f64,
}

impl LinearGrade {
    /// The value at a part position, as the kernel defines it.
    pub fn at(&self, position: [f64; 3]) -> f64 {
        let t = ((position[self.axis.index()] - self.from) / (self.to - self.from)).clamp(0.0, 1.0);
        self.start + (self.end - self.start) * t
    }

    /// The anchor points in part coordinates: `from` and `to` on the axis, zero elsewhere.
    pub fn anchors(&self) -> [[f64; 3]; 2] {
        let mut from = [0.0; 3];
        let mut to = [0.0; 3];
        from[self.axis.index()] = self.from;
        to[self.axis.index()] = self.to;
        [from, to]
    }

    /// The change of value per millimetre along the axis.
    pub fn slope(&self) -> f64 {
        ((self.end - self.start) / (self.to - self.from)).abs()
    }
}

/// The faces a distance grade measures from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradeFaces<F> {
    /// Every face of the body the field fills: denser or thicker near its skin.
    Skin,
    /// Faces the user picked, such as a load face.
    Picked(Vec<F>),
}

impl<F> GradeFaces<F> {
    /// The picked faces; none for the skin.
    pub fn picked(&self) -> &[F] {
        match self {
            GradeFaces::Skin => &[],
            GradeFaces::Picked(faces) => faces,
        }
    }
}

/// A value graded by distance from faces of the body: `near` on the faces, `far` at
/// `span` from them and beyond, linear in between.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistanceGrade<F> {
    /// The faces the distance is measured from.
    pub faces: GradeFaces<F>,
    /// The value on the faces.
    pub near: f64,
    /// The value at `span` from the faces and beyond.
    pub far: f64,
    /// The distance in millimetres over which the value changes.
    pub span: f64,
}

/// A value graded by distance from a line parallel to a part axis: `start` at radius
/// `from`, `end` at radius `to`, linear in between and clamped beyond.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RadialGrade {
    /// The part axis the line runs along.
    pub axis: GradeAxis,
    /// A part point the line passes through.
    pub through: [f64; 3],
    /// Radius in millimetres where the value is `start`.
    pub from: f64,
    /// Radius in millimetres where the value is `end`.
    pub to: f64,
    /// The value at `from`.
    pub start: f64,
    /// The value at `to`.
    pub end: f64,
}

impl RadialGrade {
    /// The value at a part position, clamped beyond the two radii.
    pub fn at(&self, position: [f64; 3]) -> f64 {
        let along = self.axis.index();
        let r = (0..3).filter(|&i| i != along).map(|i| (position[i] - self.through[i]).powi(2)).sum::<f64>().sqrt();
        ramp(r, self.from, self.to, self.start, self.end)
    }
}

/// A value graded by distance from a part point: `start` at distance `from`, `end` at
/// distance `to`, linear in between and clamped beyond.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointGrade {
    /// The part point the distance is measured from.
    pub centre: [f64; 3],
    /// Distance in millimetres where the value is `start`.
    pub from: f64,
    /// Distance in millimetres where the value is `end`.
    pub to: f64,
    /// The value at `from`.
    pub start: f64,
    /// The value at `to`.
    pub end: f64,
}

impl PointGrade {
    /// The value at a part position, clamped beyond the two distances.
    pub fn at(&self, position: [f64; 3]) -> f64 {
        let r = (0..3).map(|i| (position[i] - self.centre[i]).powi(2)).sum::<f64>().sqrt();
        ramp(r, self.from, self.to, self.start, self.end)
    }
}

fn ramp(r: f64, from: f64, to: f64, start: f64, end: f64) -> f64 {
    let t = ((r - from) / (to - from)).clamp(0.0, 1.0);
    start + (end - start) * t
}

/// The kinds of grade, for asking a kernel which it builds and for the editor's choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GradeKind {
    /// One value everywhere.
    Constant,
    /// Linear along a part axis.
    Linear,
    /// By distance from faces of the body.
    Distance,
    /// By distance from a line parallel to a part axis.
    Radial,
    /// By distance from a part point.
    Point,
}

impl GradeKind {
    /// Every kind, in the order the editor offers them.
    pub const ALL: [GradeKind; 5] =
        [GradeKind::Constant, GradeKind::Linear, GradeKind::Distance, GradeKind::Radial, GradeKind::Point];

    /// The name the editor shows.
    pub fn label(self) -> &'static str {
        match self {
            GradeKind::Constant => "Constant",
            GradeKind::Linear => "Linear along an axis",
            GradeKind::Distance => "Distance from faces",
            GradeKind::Radial => "Radial from an axis",
            GradeKind::Point => "From a point",
        }
    }

    /// The lowercase name files and tools use.
    pub fn name(self) -> &'static str {
        match self {
            GradeKind::Constant => "constant",
            GradeKind::Linear => "linear",
            GradeKind::Distance => "distance",
            GradeKind::Radial => "radial",
            GradeKind::Point => "point",
        }
    }
}

/// The terminal field a grade drives: a lattice's beam radius or a gyroid's level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GradedField {
    /// A lattice's beam radius, in millimetres.
    LatticeRadius,
    /// A gyroid's dimensionless level.
    GyroidLevel,
}

/// How a field parameter varies over the part. A constant is written as a bare
/// number, as files always had it; a function is an object that names it, so a reader
/// that does not know a function refuses the file instead of reading it as a constant.
/// `F` is how a distance grade names its faces: persistent references in a saved part,
/// kernel face ids in a kernel request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    from = "GradeFile<F>",
    into = "GradeFile<F>",
    bound(serialize = "F: Serialize + Clone", deserialize = "F: Deserialize<'de>")
)]
pub enum Grade<F = crate::FaceId> {
    /// The same value everywhere.
    Constant(f64),
    /// Linear along one part axis, clamped beyond its anchors.
    Linear(LinearGrade),
    /// By distance from faces of the body.
    Distance(DistanceGrade<F>),
    /// By distance from a line parallel to a part axis.
    Radial(RadialGrade),
    /// By distance from a part point.
    Point(PointGrade),
}

impl<F> Grade<F> {
    /// Every value the grade is built from, for finiteness checks.
    pub fn values(&self) -> Vec<f64> {
        match self {
            Grade::Constant(v) => vec![*v],
            Grade::Linear(l) => vec![l.from, l.to, l.start, l.end],
            Grade::Distance(d) => vec![d.near, d.far, d.span],
            Grade::Radial(r) => [&r.through[..], &[r.from, r.to, r.start, r.end]].concat(),
            Grade::Point(p) => [&p.centre[..], &[p.from, p.to, p.start, p.end]].concat(),
        }
    }

    /// The values the grade takes: the constant, or both endpoints.
    pub fn endpoints(&self) -> Vec<f64> {
        match self {
            Grade::Constant(v) => vec![*v],
            Grade::Linear(l) => vec![l.start, l.end],
            Grade::Distance(d) => vec![d.near, d.far],
            Grade::Radial(r) => vec![r.start, r.end],
            Grade::Point(p) => vec![p.start, p.end],
        }
    }

    /// Validates a wall thickness in millimetres over the whole part: a constant or a
    /// linear grade, the only ones a wall takes.
    pub fn check_thickness(&self) -> Result<(), &'static str> {
        if !matches!(self, Grade::Constant(_) | Grade::Linear(_)) {
            return Err("Wall thickness is a constant or a linear grade.");
        }
        if self.values().iter().any(|v| !v.is_finite()) || self.endpoints().iter().any(|v| *v <= 0.0) {
            return Err("Wall thickness must be finite and positive, with finite grade anchors.");
        }
        if matches!(self, Grade::Linear(l) if l.from == l.to) {
            return Err("Thickness needs two distinct grade anchors.");
        }
        Ok(())
    }

    /// The constant, when the grade is one.
    pub fn constant(&self) -> Option<f64> {
        match self {
            Grade::Constant(v) => Some(*v),
            _ => None,
        }
    }

    /// Which kind of grade this is.
    pub fn kind(&self) -> GradeKind {
        match self {
            Grade::Constant(_) => GradeKind::Constant,
            Grade::Linear(_) => GradeKind::Linear,
            Grade::Distance(_) => GradeKind::Distance,
            Grade::Radial(_) => GradeKind::Radial,
            Grade::Point(_) => GradeKind::Point,
        }
    }

    /// Why the grade's own definition is impossible: a ramp that starts and ends at the
    /// same place, a distance that changes over no span, or a radius below zero. `None`
    /// when it is a valid definition, which a kernel may still not support.
    pub fn impossible(&self) -> Option<&'static str> {
        match self {
            Grade::Constant(_) => None,
            Grade::Linear(l) if l.from == l.to => Some("a linear grade needs two different anchors on its axis"),
            Grade::Linear(_) => None,
            Grade::Distance(d) if d.span.is_nan() || d.span <= 0.0 => Some("a distance grade needs a span above zero"),
            Grade::Distance(DistanceGrade { faces: GradeFaces::Picked(faces), .. }) if faces.is_empty() => {
                Some("a distance grade needs at least one face, or the skin")
            }
            Grade::Distance(_) => None,
            Grade::Radial(RadialGrade { from, to, .. }) | Grade::Point(PointGrade { from, to, .. })
                if *from < 0.0 || *to < 0.0 =>
            {
                Some("a radial or point grade measures distances, which cannot be below zero")
            }
            Grade::Radial(RadialGrade { from, to, .. }) | Grade::Point(PointGrade { from, to, .. }) if from == to => {
                Some("a radial or point grade needs two different distances")
            }
            Grade::Radial(_) | Grade::Point(_) => None,
        }
    }

    /// The faces a distance grade picked; none for every other grade and for the skin.
    pub fn picked_faces(&self) -> &[F] {
        match self {
            Grade::Distance(d) => d.faces.picked(),
            _ => &[],
        }
    }

    /// Whether the grade measures from picked faces, so a clicked face belongs to it.
    pub fn picks_faces(&self) -> bool {
        matches!(self, Grade::Distance(DistanceGrade { faces: GradeFaces::Picked(_), .. }))
    }

    /// The picked faces, to edit or repair; `None` when the grade picks none.
    pub fn picked_faces_mut(&mut self) -> Option<&mut Vec<F>> {
        match self {
            Grade::Distance(DistanceGrade { faces: GradeFaces::Picked(faces), .. }) => Some(faces),
            _ => None,
        }
    }

    /// The same grade with each picked face named another way, such as a persistent
    /// reference resolved to a kernel face id.
    pub fn try_map_faces<G, E>(&self, mut map: impl FnMut(usize, &F) -> Result<G, E>) -> Result<Grade<G>, E> {
        Ok(match self {
            Grade::Constant(v) => Grade::Constant(*v),
            Grade::Linear(l) => Grade::Linear(*l),
            Grade::Distance(d) => Grade::Distance(DistanceGrade {
                faces: match &d.faces {
                    GradeFaces::Skin => GradeFaces::Skin,
                    GradeFaces::Picked(faces) => GradeFaces::Picked(
                        faces.iter().enumerate().map(|(i, face)| map(i, face)).collect::<Result<_, _>>()?,
                    ),
                },
                near: d.near,
                far: d.far,
                span: d.span,
            }),
            Grade::Radial(r) => Grade::Radial(*r),
            Grade::Point(p) => Grade::Point(*p),
        })
    }
}

impl<F> From<f64> for Grade<F> {
    fn from(value: f64) -> Self {
        Grade::Constant(value)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum GradeFile<F> {
    Constant(f64),
    Function(GradeFunction<F>),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum GradeFunction<F> {
    Linear(LinearGrade),
    Distance(DistanceGrade<F>),
    Radial(RadialGrade),
    Point(PointGrade),
}

impl<F> From<GradeFile<F>> for Grade<F> {
    fn from(file: GradeFile<F>) -> Self {
        match file {
            GradeFile::Constant(v) => Grade::Constant(v),
            GradeFile::Function(GradeFunction::Linear(l)) => Grade::Linear(l),
            GradeFile::Function(GradeFunction::Distance(d)) => Grade::Distance(d),
            GradeFile::Function(GradeFunction::Radial(r)) => Grade::Radial(r),
            GradeFile::Function(GradeFunction::Point(p)) => Grade::Point(p),
        }
    }
}

impl<F> From<Grade<F>> for GradeFile<F> {
    fn from(grade: Grade<F>) -> Self {
        match grade {
            Grade::Constant(v) => GradeFile::Constant(v),
            Grade::Linear(l) => GradeFile::Function(GradeFunction::Linear(l)),
            Grade::Distance(d) => GradeFile::Function(GradeFunction::Distance(d)),
            Grade::Radial(r) => GradeFile::Function(GradeFunction::Radial(r)),
            Grade::Point(p) => GradeFile::Function(GradeFunction::Point(p)),
        }
    }
}

/// The largest widths a field's moment enclosures may have. Each is absolute, with no
/// relative allowance. The defaults are the widths the graded-field design requires.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldMassAccuracy {
    /// Volume width in cubic millimetres.
    pub volume_mm3: f64,
    /// Width of each centroid coordinate in millimetres.
    pub centroid_mm: f64,
    /// Width of each centroidal inertia component in millimetres to the fifth power.
    pub inertia_mm5: f64,
}

impl Default for FieldMassAccuracy {
    /// 0.01 mm³, 0.001 mm and 0.1 mm⁵.
    fn default() -> Self {
        FieldMassAccuracy { volume_mm3: 0.01, centroid_mm: 0.001, inertia_mm5: 0.1 }
    }
}

/// A proven enclosure of an exact quantity, as exact rationals such as `"-7/25"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExactRange {
    /// Lower end.
    pub lo: String,
    /// Upper end.
    pub hi: String,
}

/// Unit-density moments of a field's own definition, proven by interval integration of
/// the field. No mesh, preview or display region contributes to them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldMass {
    /// Always `original-field-definition`.
    pub scope: String,
    /// The requested widths, as exact rationals: volume, centroid and inertia.
    pub requested_widths: [String; 3],
    /// Volume in cubic millimetres.
    pub volume_mm3: ExactRange,
    /// Centre of gravity in millimetres; absent exactly for a zero volume.
    pub centroid_mm: Option<[ExactRange; 3]>,
    /// Inertia about the centre of gravity along the part axes, at unit density, in
    /// millimetres to the fifth power; absent exactly for a zero volume.
    pub inertia_mm5: Option<[[ExactRange; 3]; 3]>,
    /// Classified boxes and analytic intervals the integration used.
    pub cells: usize,
    /// Deterministic work units the request used.
    pub work: u64,
}

/// The explicit mass stand-in of a field: a STEP file with the field's source envelope
/// as a massless surface model and unit-density proxy solids whose moments lie inside
/// the field's enclosures.
#[derive(Clone, Debug)]
pub struct StandInExport {
    /// The STEP AP242 file.
    pub bytes: Vec<u8>,
    /// What it holds and the proof that its mass bodies match the field.
    pub report: StandInReport,
}

/// Moments of the proxy solids, as exact rationals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProxyMoments {
    /// Volume in cubic millimetres.
    pub volume_mm3: String,
    /// Centre of gravity in millimetres.
    pub centroid_mm: [String; 3],
    /// Centroidal inertia in millimetres to the fifth power.
    pub inertia_mm5: [[String; 3]; 3],
}

/// Bounds of the proxy's moments after the file's coordinate rounding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExchangedMoments {
    /// Volume in cubic millimetres.
    pub volume_mm3: ExactRange,
    /// Centre of gravity in millimetres.
    pub centroid_mm: [ExactRange; 3],
    /// Centroidal inertia in millimetres to the fifth power.
    pub inertia_mm5: [[ExactRange; 3]; 3],
}

/// What a mass stand-in wrote and why its mass bodies stand for the field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StandInReport {
    /// Always `mass stand-in — not field boundary geometry`.
    pub label: String,
    /// Always `massless-surface-envelope-plus-unit-density-solids`.
    pub representation: String,
    /// The field's own moments the proxy was matched to.
    pub field: FieldMass,
    /// The proxy solids' exact moments.
    pub proxy: ProxyMoments,
    /// The proxy's moments once written, including the file's rounding; each lies inside
    /// the field's enclosure.
    pub exchanged: ExchangedMoments,
    /// The proxy's exact inertia residual against its target, in millimetres to the fifth power.
    pub inertia_residual: String,
    /// Unit-density proxy solids written.
    pub mass_bodies: usize,
    /// Massless surface envelopes written: the field's source region.
    pub surface_envelopes: usize,
    /// Proven bound, in millimetres, of the file's coordinate rounding.
    pub max_deviation_mm: String,
    /// The file states no density or material.
    pub density_metadata: bool,
    /// Deterministic work units the request used, the field moments included.
    pub work: u64,
    /// SHA-256 of the file.
    pub sha256: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_stay_bare_numbers_and_functions_name_themselves() {
        assert_eq!(serde_json::to_string(&Grade::<crate::FaceId>::Constant(1.5)).unwrap(), "1.5");
        let linear: Grade =
            Grade::Linear(LinearGrade { axis: GradeAxis::Z, from: 20.0, to: 30.0, start: 1.0, end: 2.0 });
        let text = serde_json::to_string(&linear).unwrap();
        assert_eq!(text, r#"{"linear":{"axis":"z","from":20.0,"to":30.0,"start":1.0,"end":2.0}}"#);
        assert_eq!(serde_json::from_str::<Grade>(&text).unwrap(), linear);
        assert_eq!(serde_json::from_str::<Grade>("-0.1").unwrap(), Grade::Constant(-0.1));
    }

    #[test]
    fn unknown_grade_functions_and_keys_are_refused() {
        for text in [
            r#"{"spline":{"axis":"z","from":0,"to":1,"start":1,"end":2}}"#,
            r#"{"radial":{"axis":"z","from":0,"to":1,"start":1,"end":2}}"#,
            r#"{"point":{"centre":[0,0,0],"from":0,"to":1,"start":1,"end":2,"axis":"z"}}"#,
            r#"{"distance":{"faces":"skin","near":1,"far":2}}"#,
            r#"{"distance":{"faces":"everything","near":1,"far":2,"span":3}}"#,
            r#"{"linear":{"axis":"z","from":0,"to":1,"start":1,"end":2,"span":3}}"#,
            r#"{"linear":{"axis":"w","from":0,"to":1,"start":1,"end":2}}"#,
            r#"{"linear":{"axis":"z","from":0,"to":1,"start":1}}"#,
            r#""sin(z)""#,
            "null",
        ] {
            assert!(serde_json::from_str::<Grade>(text).is_err(), "{text}");
        }
    }

    #[test]
    fn a_linear_grade_clamps_beyond_its_anchors() {
        let grade = LinearGrade { axis: GradeAxis::Z, from: 20.0, to: 30.0, start: 1.0, end: 2.0 };
        assert_eq!(grade.at([0.0, 0.0, 22.0]), 1.2);
        assert_eq!(grade.at([5.0, -5.0, 10.0]), 1.0);
        assert_eq!(grade.at([0.0, 0.0, 31.0]), 2.0);
        assert_eq!(grade.anchors(), [[0.0, 0.0, 20.0], [0.0, 0.0, 30.0]]);
        assert_eq!(grade.slope(), 0.1);
        let x = LinearGrade { axis: GradeAxis::X, ..grade };
        assert_eq!(x.anchors(), [[20.0, 0.0, 0.0], [30.0, 0.0, 0.0]]);
    }

    #[test]
    fn new_grades_name_themselves_and_read_back() {
        let skin: Grade<u32> =
            Grade::Distance(DistanceGrade { faces: GradeFaces::Skin, near: 2.0, far: 1.0, span: 5.0 });
        let text = serde_json::to_string(&skin).unwrap();
        assert_eq!(text, r#"{"distance":{"faces":"skin","near":2.0,"far":1.0,"span":5.0}}"#);
        assert_eq!(serde_json::from_str::<Grade<u32>>(&text).unwrap(), skin);
        let picked: Grade<u32> =
            Grade::Distance(DistanceGrade { faces: GradeFaces::Picked(vec![7]), near: 2.0, far: 1.0, span: 5.0 });
        let text = serde_json::to_string(&picked).unwrap();
        assert_eq!(text, r#"{"distance":{"faces":{"picked":[7]},"near":2.0,"far":1.0,"span":5.0}}"#);
        assert_eq!(serde_json::from_str::<Grade<u32>>(&text).unwrap(), picked);
        let radial: Grade = Grade::Radial(RadialGrade {
            axis: GradeAxis::Z,
            through: [1.0, 2.0, 0.0],
            from: 0.0,
            to: 4.0,
            start: 1.0,
            end: 2.0,
        });
        let text = serde_json::to_string(&radial).unwrap();
        assert_eq!(
            text,
            r#"{"radial":{"axis":"z","through":[1.0,2.0,0.0],"from":0.0,"to":4.0,"start":1.0,"end":2.0}}"#
        );
        assert_eq!(serde_json::from_str::<Grade>(&text).unwrap(), radial);
        let point: Grade =
            Grade::Point(PointGrade { centre: [0.0, 0.0, 5.0], from: 1.0, to: 3.0, start: 0.5, end: 1.5 });
        let text = serde_json::to_string(&point).unwrap();
        assert_eq!(text, r#"{"point":{"centre":[0.0,0.0,5.0],"from":1.0,"to":3.0,"start":0.5,"end":1.5}}"#);
        assert_eq!(serde_json::from_str::<Grade>(&text).unwrap(), point);
    }

    #[test]
    fn radial_and_point_grades_clamp_beyond_their_distances() {
        let radial =
            RadialGrade { axis: GradeAxis::Z, through: [1.0, 0.0, 0.0], from: 0.0, to: 4.0, start: 1.0, end: 2.0 };
        assert_eq!(radial.at([1.0, 0.0, 50.0]), 1.0);
        assert_eq!(radial.at([1.0, 2.0, -3.0]), 1.5);
        assert_eq!(radial.at([9.0, 0.0, 0.0]), 2.0);
        let point = PointGrade { centre: [0.0, 0.0, 5.0], from: 1.0, to: 3.0, start: 0.5, end: 1.5 };
        assert_eq!(point.at([0.0, 0.0, 5.0]), 0.5);
        assert_eq!(point.at([0.0, 2.0, 5.0]), 1.0);
        assert_eq!(point.at([0.0, 0.0, 15.0]), 1.5);
    }

    #[test]
    fn impossible_definitions_are_named_and_valid_ones_pass() {
        let distance =
            |faces: GradeFaces<u32>, span| Grade::Distance(DistanceGrade { faces, near: 1.0, far: 2.0, span });
        assert!(distance(GradeFaces::Skin, 0.0).impossible().is_some());
        assert!(distance(GradeFaces::Picked(vec![]), 1.0).impossible().is_some());
        assert!(distance(GradeFaces::Picked(vec![3]), 1.0).impossible().is_none());
        assert!(distance(GradeFaces::Skin, 1.0).impossible().is_none());
        let point = |from, to| Grade::<u32>::Point(PointGrade { centre: [0.0; 3], from, to, start: 1.0, end: 2.0 });
        assert!(point(-1.0, 2.0).impossible().is_some());
        assert!(point(2.0, 2.0).impossible().is_some());
        assert!(point(0.0, 2.0).impossible().is_none());
        let linear = Grade::<u32>::Linear(LinearGrade { axis: GradeAxis::X, from: 1.0, to: 1.0, start: 1.0, end: 2.0 });
        assert!(linear.impossible().is_some());
    }

    #[test]
    fn mapping_faces_keeps_every_number_and_the_skin() {
        let picked: Grade<u32> =
            Grade::Distance(DistanceGrade { faces: GradeFaces::Picked(vec![4, 5]), near: 2.0, far: 1.0, span: 5.0 });
        let mapped: Grade<String> = picked.try_map_faces(|i, f| Ok::<_, ()>(format!("{i}:{f}"))).unwrap();
        assert_eq!(mapped.picked_faces(), ["0:4".to_string(), "1:5".to_string()]);
        assert_eq!(mapped.values(), picked.values());
        let skin: Grade<u32> =
            Grade::Distance(DistanceGrade { faces: GradeFaces::Skin, near: 2.0, far: 1.0, span: 5.0 });
        let mapped: Grade<String> = skin.try_map_faces(|_, _| Err::<String, _>("never called")).unwrap();
        assert_eq!(mapped.kind(), GradeKind::Distance);
        assert!(mapped.picked_faces().is_empty());
    }
}
