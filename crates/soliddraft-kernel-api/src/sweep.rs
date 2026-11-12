//! Sweep paths in model millimetres: lines, circular arcs and rational splines.
//!
//! An arc is saved as its centre, axis and turn, so its end is never a rounded number
//! and a turn outside the three supported ones cannot be saved. The backend derives
//! exact ends. The float methods here are for display and editing only.

use serde::{Deserialize, Serialize};

/// How the profile turns along a path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepOrientation {
    /// The profile only translates, so it stays parallel to its sketch.
    #[default]
    Fixed,
    /// The profile turns with the path, including at sharp joins.
    FollowPath,
}

/// How far an arc turns, counterclockwise about its axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArcTurn {
    /// 90 degrees.
    #[default]
    Quarter,
    /// 180 degrees.
    Half,
    /// 270 degrees.
    ThreeQuarter,
}

impl ArcTurn {
    /// The turn in degrees.
    pub fn degrees(self) -> f64 {
        match self {
            ArcTurn::Quarter => 90.0,
            ArcTurn::Half => 180.0,
            ArcTurn::ThreeQuarter => 270.0,
        }
    }
}

/// One piece of a sweep path. It starts where the previous piece ends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PathSegment {
    /// A straight line to `to`.
    Line {
        /// Stable saved identity. The faces this piece makes keep it across edits.
        id: String,
        /// End point.
        to: [f64; 3],
        /// The turn axis where this piece doubles back along the previous one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        corner_axis: Option<[f64; 3]>,
    },
    /// A circular arc about `centre`, counterclockwise about `axis` by `turn`.
    Arc {
        /// Stable saved identity. The faces this piece makes keep it across edits.
        id: String,
        /// Centre of the circle.
        centre: [f64; 3],
        /// Normal of the arc's plane.
        axis: [f64; 3],
        /// How far the arc turns.
        turn: ArcTurn,
        /// The turn axis where this piece doubles back along the previous one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        corner_axis: Option<[f64; 3]>,
    },
    /// A clamped rational B-spline. Its first control point is the path's previous end.
    Spline {
        /// Stable saved identity.
        id: String,
        /// Polynomial degree.
        degree: usize,
        /// Control points in model millimetres, including both ends.
        control_points: Vec<[f64; 3]>,
        /// Positive weight for each control point.
        weights: Vec<f64>,
        /// Nondecreasing knot vector.
        knots: Vec<f64>,
    },
}

impl PathSegment {
    /// The stable saved identity.
    pub fn id(&self) -> &str {
        match self {
            PathSegment::Line { id, .. } | PathSegment::Arc { id, .. } | PathSegment::Spline { id, .. } => id,
        }
    }

    /// The saved axis of the turn at the join before this piece.
    pub fn corner_axis(&self) -> Option<[f64; 3]> {
        match self {
            PathSegment::Line { corner_axis, .. } | PathSegment::Arc { corner_axis, .. } => *corner_axis,
            PathSegment::Spline { .. } => None,
        }
    }

    /// Sets the axis of the turn at the join before this piece.
    pub fn set_corner_axis(&mut self, axis: Option<[f64; 3]>) {
        match self {
            PathSegment::Line { corner_axis, .. } | PathSegment::Arc { corner_axis, .. } => *corner_axis = axis,
            PathSegment::Spline { .. } => {}
        }
    }
}

/// A saved path. The profile stays at its authored placement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "SavedSweep")]
pub struct Sweep {
    /// Start of the path.
    pub start: [f64; 3],
    /// The pieces of the path, in order.
    pub segments: Vec<PathSegment>,
    /// How the profile turns along the path.
    pub orientation: SweepOrientation,
}

/// A saved path. Parts from before chains hold one straight segment.
#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
enum SavedSweep {
    Chain {
        start: [f64; 3],
        segments: Vec<PathSegment>,
        #[serde(default)]
        orientation: SweepOrientation,
    },
    Straight {
        start: [f64; 3],
        end: [f64; 3],
        segment_id: String,
    },
}

impl From<SavedSweep> for Sweep {
    fn from(saved: SavedSweep) -> Self {
        match saved {
            SavedSweep::Chain { start, segments, orientation } => Sweep { start, segments, orientation },
            SavedSweep::Straight { start, end, segment_id } => Sweep::line(start, end, segment_id),
        }
    }
}

type P = [f64; 3];

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: P, b: P) -> P {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: P) -> f64 {
    dot(a, a).sqrt()
}

fn scale(a: P, k: f64) -> P {
    [a[0] * k, a[1] * k, a[2] * k]
}

/// The ends and tangents of one piece, for display.
struct Span {
    end: P,
    incoming: P,
    outgoing: P,
    length: f64,
}

impl Sweep {
    /// One straight segment. Orientation has no effect on a single line.
    pub fn line(start: [f64; 3], end: [f64; 3], id: impl Into<String>) -> Self {
        let segments = vec![PathSegment::Line { id: id.into(), to: end, corner_axis: None }];
        Sweep { start, segments, orientation: SweepOrientation::Fixed }
    }

    fn spans(&self) -> Vec<Span> {
        let mut previous = self.start;
        self.segments
            .iter()
            .map(|segment| {
                let span = match segment {
                    PathSegment::Line { to, .. } => {
                        let delta = sub(*to, previous);
                        Span { end: *to, incoming: delta, outgoing: delta, length: length(delta) }
                    }
                    PathSegment::Spline { degree, control_points, weights, knots, .. } => {
                        let end = control_points.last().copied().unwrap_or(previous);
                        let incoming = control_points.get(1).map(|p| sub(*p, previous)).unwrap_or([0.0; 3]);
                        let outgoing = control_points.iter().rev().nth(1).map(|p| sub(end, *p)).unwrap_or([0.0; 3]);
                        let mut length_mm = 0.0;
                        let mut last = previous;
                        if *degree > 0
                            && *degree < control_points.len()
                            && weights.len() == control_points.len()
                            && knots.len() == control_points.len() + degree + 1
                        {
                            let lo = knots[*degree];
                            let hi = knots[control_points.len()];
                            for i in 1..=64 {
                                let t = lo + (hi - lo) * i as f64 / 64.0;
                                let span =
                                    (*degree..control_points.len()).rev().find(|&j| knots[j] <= t).unwrap_or(*degree);
                                let mut row: Vec<[f64; 4]> = (span - degree..=span)
                                    .map(|j| {
                                        let p = control_points[j];
                                        [p[0] * weights[j], p[1] * weights[j], p[2] * weights[j], weights[j]]
                                    })
                                    .collect();
                                for r in 1..=*degree {
                                    for j in (r..=*degree).rev() {
                                        let k = span - degree + j;
                                        let width = knots[k + degree + 1 - r] - knots[k];
                                        let a = if width > 0.0 { (t - knots[k]) / width } else { 0.0 };
                                        row[j] = std::array::from_fn(|c| (1.0 - a) * row[j - 1][c] + a * row[j][c]);
                                    }
                                }
                                let h = row[*degree];
                                if h[3] > 0.0 {
                                    let point = [h[0] / h[3], h[1] / h[3], h[2] / h[3]];
                                    length_mm += length(sub(point, last));
                                    last = point;
                                }
                            }
                        }
                        Span { end, incoming, outgoing, length: length_mm }
                    }
                    PathSegment::Arc { centre, axis, turn, .. } => {
                        let a = sub(previous, *centre);
                        let unit = scale(*axis, 1.0 / length(*axis));
                        let across = cross(unit, a);
                        let b = match turn {
                            ArcTurn::Quarter => across,
                            ArcTurn::Half => scale(a, -1.0),
                            ArcTurn::ThreeQuarter => scale(across, -1.0),
                        };
                        Span {
                            end: add(*centre, b),
                            incoming: across,
                            outgoing: cross(unit, b),
                            length: length(a) * turn.degrees().to_radians(),
                        }
                    }
                };
                previous = span.end;
                span
            })
            .collect()
    }

    /// Where each piece ends, for display.
    pub fn ends(&self) -> Vec<[f64; 3]> {
        self.spans().into_iter().map(|s| s.end).collect()
    }

    /// Where the last piece ends, for display.
    pub fn end(&self) -> [f64; 3] {
        self.ends().last().copied().unwrap_or(self.start)
    }

    /// The path's length, for display.
    pub fn length(&self) -> f64 {
        self.spans().iter().map(|s| s.length).sum()
    }

    /// The direction the path leaves its end in, for display. `None` for an empty path.
    pub fn end_tangent(&self) -> Option<[f64; 3]> {
        self.spans().last().map(|s| s.outgoing)
    }

    /// The direction piece `index` starts in, for display.
    pub fn start_tangent(&self, index: usize) -> Option<[f64; 3]> {
        self.spans().get(index).map(|s| s.incoming)
    }

    /// Whether piece `index` doubles back along the piece before it, so that a profile
    /// turning with the path needs a saved turn axis there.
    pub fn reverses_at(&self, index: usize) -> bool {
        let spans = self.spans();
        let (Some(before), Some(after)) = (index.checked_sub(1).and_then(|i| spans.get(i)), spans.get(index)) else {
            return false;
        };
        let (a, b) = (before.outgoing, after.incoming);
        let scale = length(a) * length(b);
        scale > 0.0 && dot(a, b) < 0.0 && length(cross(a, b)) <= scale * 1e-12
    }

    /// An unused segment identity.
    pub fn next_segment_id(&self) -> String {
        (self.segments.len() + 1..)
            .map(|n| format!("segment{n}"))
            .find(|id| self.segments.iter().all(|s| s.id() != id))
            .expect("a finite path leaves a name unused")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: P, b: P) -> bool {
        length(sub(a, b)) < 1e-9
    }

    #[test]
    fn rational_spline_display_and_saved_parameters_keep_the_path() {
        let path = Sweep {
            start: [0.0; 3],
            orientation: SweepOrientation::Fixed,
            segments: vec![PathSegment::Spline {
                id: "rail".into(),
                degree: 3,
                control_points: vec![[0.0; 3], [0.0, 0.0, 1.0], [0.0, 0.0, 2.0], [0.0, 0.0, 3.0]],
                weights: vec![1.0, 2.0, 2.0, 1.0],
                knots: vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            }],
        };
        assert_eq!(path.end(), [0.0, 0.0, 3.0]);
        assert!((path.length() - 3.0).abs() < 1e-9);
        assert_eq!(path.start_tangent(0), Some([0.0, 0.0, 1.0]));
        assert_eq!(path.end_tangent(), Some([0.0, 0.0, 1.0]));
        assert_eq!(path.segments[0].id(), "rail");
        assert_eq!(serde_json::from_value::<Sweep>(serde_json::to_value(&path).unwrap()).unwrap(), path);
    }

    #[test]
    fn saved_straight_paths_read_as_one_line_with_their_identity() {
        let saved = r#"{"start":[1,2,3],"end":[1,2,9],"segment_id":"travel"}"#;
        let sweep: Sweep = serde_json::from_str(saved).unwrap();
        assert_eq!(sweep, Sweep::line([1.0, 2.0, 3.0], [1.0, 2.0, 9.0], "travel"));
        let again: Sweep = serde_json::from_str(&serde_json::to_string(&sweep).unwrap()).unwrap();
        assert_eq!(sweep, again);
    }

    #[test]
    fn unknown_saved_fields_are_refused_rather_than_dropped() {
        let saved = r#"{"start":[0,0,0],"segments":[],"orientation":"fixed","twist":1}"#;
        assert!(serde_json::from_str::<Sweep>(saved).is_err());
    }

    #[test]
    fn arc_ends_and_tangents_follow_the_axis_and_turn() {
        let arc = |turn| Sweep {
            start: [0.0, 0.0, 4.0],
            segments: vec![PathSegment::Arc {
                id: "bend".into(),
                centre: [0.0, 5.0, 4.0],
                axis: [-2.0, 0.0, 0.0],
                turn,
                corner_axis: None,
            }],
            orientation: SweepOrientation::FollowPath,
        };
        assert!(close(arc(ArcTurn::Quarter).end(), [0.0, 5.0, 9.0]));
        assert!(close(arc(ArcTurn::Half).end(), [0.0, 10.0, 4.0]));
        assert!(close(arc(ArcTurn::ThreeQuarter).end(), [0.0, 5.0, -1.0]));
        let tangent = arc(ArcTurn::Quarter).end_tangent().unwrap();
        assert!(close(scale(tangent, 1.0 / length(tangent)), [0.0, 1.0, 0.0]));
        assert!((arc(ArcTurn::Half).length() - 5.0 * std::f64::consts::PI).abs() < 1e-9);
    }

    #[test]
    fn reversals_are_found_at_joins_only() {
        let line = |id: &str, to| PathSegment::Line { id: id.into(), to, corner_axis: None };
        let chain = Sweep {
            start: [0.0; 3],
            segments: vec![line("a", [0.0, 0.0, 4.0]), line("b", [0.0, 0.0, 1.0]), line("c", [0.0, 3.0, 1.0])],
            orientation: SweepOrientation::FollowPath,
        };
        assert_eq!((0..4).map(|i| chain.reverses_at(i)).collect::<Vec<_>>(), [false, true, false, false]);
    }

    #[test]
    fn new_identities_never_reuse_a_saved_one() {
        let mut chain = Sweep::line([0.0; 3], [0.0, 0.0, 1.0], "segment2");
        assert_eq!(chain.next_segment_id(), "segment3");
        chain.segments.clear();
        assert_eq!(chain.next_segment_id(), "segment1");
    }
}
