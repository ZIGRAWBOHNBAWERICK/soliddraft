//! Cosmetic threads on holes: ISO metric sizes and pitches, the designation a drawing and
//! a STEP file write, and the tap drill a threaded hole is drilled with.
//!
//! A thread here is a mark, not geometry. The hole keeps the plain cylinder of its
//! drill; the part says that the cylinder is tapped, how far, and to which size. The
//! viewport draws the mark on the wall, a drawing calls the hole out by its designation
//! (`M6x1-6H`), and a STEP file carries the designation on the hole's diameter.

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::{FaceId, HoleDepth, HoleHead, HolePart, SurfaceKind, Vec3};

use crate::document::{Document, FeatureStatus};
use crate::feature::{Feature, FeatureId, FeatureKind, PlaneRef};
use crate::reference::{self, FaceFingerprint, FaceRef, Generator, View};

/// One ISO metric size: its nominal diameter, coarse pitch, the fine pitches ISO 262
/// selects for it, and the drill ISO 2306 gives for the coarse thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricSize {
    /// Nominal (major) diameter in millimetres.
    pub nominal: f64,
    /// The coarse pitch.
    pub coarse: f64,
    /// The fine pitches, coarsest first.
    pub fine: &'static [f64],
    /// The tap drill for the coarse pitch.
    pub coarse_drill: f64,
}

impl MetricSize {
    /// Every pitch of the size, coarse first.
    pub fn pitches(&self) -> impl Iterator<Item = f64> + '_ {
        std::iter::once(self.coarse).chain(self.fine.iter().copied())
    }

    /// True when `pitch` is one of the size's pitches.
    pub fn has_pitch(&self, pitch: f64) -> bool {
        self.pitches().any(|p| same(p, pitch))
    }
}

const fn size(nominal: f64, coarse: f64, fine: &'static [f64], coarse_drill: f64) -> MetricSize {
    MetricSize { nominal, coarse, fine, coarse_drill }
}

/// ISO 262 selected sizes for screws, bolts and nuts, M1.6 to M64.
pub const METRIC: &[MetricSize] = &[
    size(1.6, 0.35, &[], 1.25),
    size(2.0, 0.4, &[], 1.6),
    size(2.5, 0.45, &[], 2.05),
    size(3.0, 0.5, &[], 2.5),
    size(3.5, 0.6, &[], 2.9),
    size(4.0, 0.7, &[], 3.3),
    size(5.0, 0.8, &[], 4.2),
    size(6.0, 1.0, &[], 5.0),
    size(8.0, 1.25, &[1.0], 6.8),
    size(10.0, 1.5, &[1.25, 1.0], 8.5),
    size(12.0, 1.75, &[1.5, 1.25], 10.2),
    size(14.0, 2.0, &[1.5], 12.0),
    size(16.0, 2.0, &[1.5], 14.0),
    size(18.0, 2.5, &[2.0, 1.5], 15.5),
    size(20.0, 2.5, &[2.0, 1.5], 17.5),
    size(22.0, 2.5, &[2.0, 1.5], 19.5),
    size(24.0, 3.0, &[2.0], 21.0),
    size(27.0, 3.0, &[2.0], 24.0),
    size(30.0, 3.5, &[2.0], 26.5),
    size(33.0, 3.5, &[2.0], 29.5),
    size(36.0, 4.0, &[3.0], 32.0),
    size(39.0, 4.0, &[3.0], 35.0),
    size(42.0, 4.5, &[3.0], 37.5),
    size(45.0, 4.5, &[3.0], 40.5),
    size(48.0, 5.0, &[3.0], 43.0),
    size(52.0, 5.0, &[4.0], 47.0),
    size(56.0, 5.5, &[4.0], 50.5),
    size(60.0, 5.5, &[4.0], 54.5),
    size(64.0, 6.0, &[4.0], 58.0),
];

/// Tolerance classes of an internal thread (ISO 965-1), the medium class first.
pub const INTERNAL_CLASSES: &[&str] = &["6H", "4H", "5H", "7H", "6G"];

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// The ISO metric size with this nominal diameter.
pub fn metric(nominal: f64) -> Option<&'static MetricSize> {
    METRIC.iter().find(|s| same(s.nominal, nominal))
}

/// A number as a designation writes it: `6`, `1.25`, `0.35`.
fn number(value: f64) -> String {
    let text = format!("{:.3}", value);
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// A hole's cosmetic thread.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoleThread {
    /// Nominal (major) diameter in millimetres: 6 for M6.
    pub size: f64,
    /// Pitch in millimetres.
    pub pitch: f64,
    /// Tolerance class, such as `6H` or `2B`.
    pub class: String,
    /// How deep the thread goes from the face the hole starts on, in millimetres. `None`
    /// threads the whole wall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<f64>,
    /// Explicit callout designation for non-ISO threads (e.g. `1/4-20 UNC-2B`, `1/4-18 NPT`).
    /// `None` formats as ISO metric `M{size}x{pitch}-{class}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub designation_override: Option<String>,
}

impl HoleThread {
    /// The coarse thread of a size, in the medium class, along the whole wall.
    pub fn coarse(size: &MetricSize) -> HoleThread {
        HoleThread {
            size: size.nominal,
            pitch: size.coarse,
            class: INTERNAL_CLASSES[0].to_owned(),
            length: None,
            designation_override: None,
        }
    }

    /// The designation as a drawing and a STEP file write it, such as `M6x1-6H`,
    /// `M8x1-6H`, or `1/4-20 UNC-2B`. For ISO metric threads the pitch is always
    /// written, coarse or fine, so nobody has to know which pitch is the coarse one.
    pub fn designation(&self) -> String {
        if let Some(custom) = self.designation_override.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            return custom.to_owned();
        }
        let class = self.class.trim();
        let mut text = format!("M{}x{}", number(self.size), number(self.pitch));
        if !class.is_empty() {
            text.push('-');
            text.push_str(class);
        }
        text
    }

    /// The table entry of the size, when it is an ISO metric size.
    pub fn metric(&self) -> Option<&'static MetricSize> {
        metric(self.size)
    }

    /// True for the coarse pitch of the size.
    pub fn is_coarse(&self) -> bool {
        self.metric().is_some_and(|s| same(s.coarse, self.pitch))
    }

    /// The drill the thread is tapped in: ISO 2306's drill for a coarse thread, else the
    /// nominal diameter less the pitch, to a tenth of a millimetre.
    pub fn tap_drill(&self) -> f64 {
        match self.metric() {
            Some(size) if self.is_coarse() => size.coarse_drill,
            _ => ((self.size - self.pitch) * 10.0).round() / 10.0,
        }
    }

    /// The basic minor diameter of the internal thread (ISO 724): the smallest hole
    /// the thread can be cut in.
    pub fn minor(&self) -> f64 {
        self.size - 1.082_532 * self.pitch
    }

    /// Why the thread cannot be cut in this hole, for people, or `None` when it can.
    pub fn problem(&self, diameter: f64, depth: &HoleDepth, head: Option<&HoleHead>) -> Option<String> {
        if self.designation_override.is_some() {
            if !(self.size.is_finite() && self.size > 0.0 && self.pitch.is_finite() && self.pitch > 0.0) {
                return Some("Thread major diameter and pitch must be positive.".to_owned());
            }
        } else {
            let Some(size) = self.metric() else {
                return Some(format!("M{} is not an ISO metric thread size.", number(self.size)));
            };
            if !size.has_pitch(self.pitch) {
                let pitches: Vec<String> = size.pitches().map(number).collect();
                return Some(format!(
                    "M{} has no {} mm pitch; its pitches are {}.",
                    number(self.size),
                    number(self.pitch),
                    pitches.join(", ")
                ));
            }
            let class = self.class.trim();
            let plain = (2..=6).contains(&class.len())
                && class.bytes().next().is_some_and(|b| b.is_ascii_digit())
                && class.bytes().all(|b| b.is_ascii_alphanumeric());
            if !plain {
                return Some("The tolerance class is a grade and a letter, such as 6H.".to_owned());
            }
        }
        let tapered_pipe = matches!(self.class.trim(), "NPT" | "Rc" | "PT");
        let min_drill = if tapered_pipe { 0.0 } else { self.minor() - 1e-6 };
        if diameter >= self.size - 1e-9 || diameter <= 0.0 || diameter < min_drill {
            return Some(format!(
                "A {} thread is cut in a hole from Ø{} to under Ø{}; this hole is Ø{}.",
                self.designation(),
                number(if tapered_pipe { 0.0 } else { self.minor() }),
                number(self.size),
                number(diameter)
            ));
        }
        if let Some(length) = self.length {
            if !(length.is_finite() && length > 0.0) {
                return Some("The thread length must be more than zero.".to_owned());
            }
            if let HoleDepth::Blind { depth } = depth
                && length > depth + 1e-9
            {
                return Some(format!(
                    "The thread is {} mm long but the hole is {} mm deep.",
                    number(length),
                    number(*depth)
                ));
            }
            let start = match head {
                Some(HoleHead::Counterbore { depth, .. }) => *depth,
                _ => 0.0,
            };
            if length <= start + 1e-9 {
                return Some(format!("The thread ends inside the {} mm counterbore.", number(start)));
            }
        }
        None
    }
}

/// One wall of a threaded hole as the part's body has it now: where the thread runs,
/// for a viewport to draw its mark.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadedWall {
    /// The hole feature.
    pub feature: FeatureId,
    /// Which of its holes, from zero.
    pub placement: usize,
    /// The thread.
    pub thread: HoleThread,
    /// The point of the hole's axis in the face the hole starts on.
    pub entry: Vec3,
    /// The drilling direction, a unit vector.
    pub direction: Vec3,
    /// The radius of the wall.
    pub radius: f64,
    /// Where the thread starts and ends, in millimetres along `direction` from `entry`:
    /// the part of the wall the thread covers, as later features left the wall.
    pub span: (f64, f64),
}

impl Document {
    /// The persistent name of the main wall of hole `placement` of feature `feature`.
    pub(crate) fn hole_wall(&self, feature: &Feature, placement: usize) -> FaceRef {
        let generator = Generator::Hole { placement: placement as u32, part: HolePart::Wall };
        FaceRef {
            feature: feature.id,
            label: format!("{} \u{b7} {generator}", feature.name),
            generator,
            ordinal: 0,
            fingerprint: FaceFingerprint {
                surface: SurfaceKind::Cylinder,
                normal: None,
                centroid: Vec3::ZERO,
                area: 0.0,
            },
        }
    }

    /// Every wall of the part's threaded holes that the body still has, with the span the
    /// thread covers on it. A wall a later feature took away has none; one it shortened
    /// has the span that is left.
    pub fn threaded_walls(&self) -> Vec<ThreadedWall> {
        let threaded_holes: Vec<_> = self
            .features()
            .iter()
            .filter_map(|feature| {
                let FeatureKind::Hole(hole) = &feature.kind else { return None };
                let thread = hole.thread.as_ref()?;
                (self.status(feature.id) == Some(&FeatureStatus::Ok)).then_some((feature, hole, thread))
            })
            .collect();
        if threaded_holes.is_empty() {
            return Vec::new();
        }
        let Some(body) = self.body() else { return Vec::new() };
        let Ok(topology) = self.view_of(body) else { return Vec::new() };
        let view = View { topology: &topology };
        let names = self.name_map();
        let axes: std::collections::HashMap<FaceId, f64> = self
            .kernel
            .face_carriers(body)
            .map(|carriers| carriers.into_iter().filter_map(|c| Some((c.id, c.axis?.radius))).collect())
            .unwrap_or_default();
        let mut out = Vec::new();
        for (feature, hole, thread) in threaded_holes {
            let Some((_, drilled)) = self.bodies_before(feature.id).into_iter().find(|(key, _)| *key == feature.body)
            else {
                continue;
            };
            let Ok(plane) = self.plane_of(&PlaneRef::Face { face: hole.face.clone() }, drilled) else { continue };
            let direction = -plane.normal;
            for (placement, point) in hole.points.iter().enumerate() {
                let entry = plane.origin + plane.x_dir * point.u + plane.y_dir() * point.v;
                let Ok(faces) = reference::resolve_faces(&self.hole_wall(feature, placement), &view, &names) else {
                    continue;
                };
                let mut low = f64::INFINITY;
                let mut high = f64::NEG_INFINITY;
                let mut radius = None;
                for face in &faces {
                    radius = radius.or_else(|| axes.get(face).copied());
                    let Some(info) = topology.face(*face) else { continue };
                    for edge in info.edges.iter().filter_map(|e| topology.edge(*e)) {
                        let points = edge
                            .vertices
                            .iter()
                            .filter_map(|v| topology.vertices.iter().find(|x| x.id == *v))
                            .map(|v| v.position)
                            .chain([edge.midpoint]);
                        for p in points {
                            let along = (p - entry).dot(direction);
                            low = low.min(along);
                            high = high.max(along);
                        }
                    }
                }
                let start = low.max(0.0);
                let end = thread.length.map_or(high, |length| high.min(length));
                if end.is_nan() || end <= start + 1e-6 {
                    continue;
                }
                out.push(ThreadedWall {
                    feature: feature.id,
                    placement,
                    thread: thread.clone(),
                    entry,
                    direction,
                    radius: radius.unwrap_or(hole.diameter / 2.0),
                    span: (start, end),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn designations_write_the_pitch_and_class() {
        let m6 = HoleThread::coarse(metric(6.0).unwrap());
        assert_eq!(m6.designation(), "M6x1-6H");
        let fine = HoleThread { pitch: 1.0, ..HoleThread::coarse(metric(8.0).unwrap()) };
        assert_eq!(fine.designation(), "M8x1-6H");
        let small = HoleThread::coarse(metric(1.6).unwrap());
        assert_eq!(small.designation(), "M1.6x0.35-6H");
    }

    #[test]
    fn tap_drills_follow_iso_2306_and_d_minus_p() {
        assert_eq!(HoleThread::coarse(metric(6.0).unwrap()).tap_drill(), 5.0);
        assert_eq!(HoleThread::coarse(metric(8.0).unwrap()).tap_drill(), 6.8);
        assert_eq!(HoleThread::coarse(metric(12.0).unwrap()).tap_drill(), 10.2);
        let fine = |nominal, pitch| HoleThread { pitch, ..HoleThread::coarse(metric(nominal).unwrap()) };
        assert_eq!(fine(8.0, 1.0).tap_drill(), 7.0);
        assert_eq!(fine(10.0, 1.25).tap_drill(), 8.8);
        assert_eq!(fine(12.0, 1.25).tap_drill(), 10.8);
        // Every tap drill lies where the thread can be cut.
        for size in METRIC {
            for pitch in size.pitches() {
                let thread = HoleThread { pitch, ..HoleThread::coarse(size) };
                assert_eq!(
                    thread.problem(thread.tap_drill(), &HoleDepth::Through, None),
                    None,
                    "{}",
                    thread.designation()
                );
            }
        }
    }

    #[test]
    fn problems_are_named() {
        let m6 = HoleThread::coarse(metric(6.0).unwrap());
        let through = HoleDepth::Through;
        assert!(m6.problem(6.0, &through, None).unwrap().contains("this hole is Ø6"));
        assert!(m6.problem(4.5, &through, None).is_some());
        let odd = HoleThread { pitch: 0.75, ..m6.clone() };
        assert_eq!(odd.problem(5.0, &through, None).unwrap(), "M6 has no 0.75 mm pitch; its pitches are 1.");
        let long = HoleThread { length: Some(14.0), ..m6.clone() };
        assert_eq!(
            long.problem(5.0, &HoleDepth::Blind { depth: 12.0 }, None).unwrap(),
            "The thread is 14 mm long but the hole is 12 mm deep."
        );
        let short = HoleThread { length: Some(3.0), ..m6.clone() };
        let counterbore = HoleHead::Counterbore { diameter: 11.0, depth: 4.0 };
        assert!(short.problem(5.0, &through, Some(&counterbore)).unwrap().contains("counterbore"));
        let class = HoleThread { class: "H".into(), ..m6 };
        assert!(class.problem(5.0, &through, None).is_some());
        assert_eq!(
            HoleThread { size: 7.0, ..class }.problem(5.0, &through, None).unwrap(),
            "M7 is not an ISO metric thread size."
        );
    }
}
