//! Views: projections of the model placed on a sheet.

use kurbo::BezPath;
use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::drawing::{StandardView, ViewSpec};
use soliddraft_kernel_api::{Point2, Vec3};

use crate::entity::EntityRef;
use crate::ids::ViewId;

/// One view on a sheet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// Id, unique within the drawing.
    pub id: ViewId,
    /// How the view is derived.
    pub kind: ViewKind,
    /// Sheet position of the view's origin, in sheet millimetres from the bottom-left
    /// corner with Y up. The origin is the centre of the geometry's bounding box.
    pub position: Point2,
    /// The alignment kept with a parent view, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alignment: Option<Alignment>,
    /// Scale of the view.
    pub scale: ViewScale,
    /// Line display options.
    pub display: ViewDisplay,
    /// True when automatic updates skip this view.
    #[serde(default)]
    pub excluded_from_update: bool,
    /// The cached line work.
    pub geometry: ViewGeometry,
    /// The view's name in the drawing tree and its caption. `None` uses the default for
    /// its kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// A crop: only the line work inside this boundary, in view millimetres, is shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<Boundary>,
    /// Broken-out sections: inside each boundary the material is removed down to a
    /// depth and the cut is hatched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub broken_out: Vec<BrokenOut>,
    /// For a view of an assembly: it shows the assembly's exploded view.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exploded: bool,
    /// The interruption of a long view, stored as zero or one break.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<Break>,
    /// A half section: one side of a symmetry line cut open to the plane through it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half: Option<HalfSection>,
}

impl View {
    /// True when the saved line work was drawn by the kernel's exact drawing queries,
    /// which is what an export writes.
    pub fn has_saved_catallax_linework(&self) -> bool {
        self.geometry.catallax && !self.geometry.unproved && !self.geometry.display_only
    }

    /// True when an export prints the view. A section drawn from the display mesh shows
    /// on screen only, so it never blocks an export and is never counted in one.
    pub fn is_printed(&self) -> bool {
        !self.geometry.display_only
    }

    /// True when the view is excluded from automatic update and its saved line work
    /// cannot be exported, so it must be included and rebuilt first.
    pub fn needs_inclusion_for_redraw(&self) -> bool {
        self.excluded_from_update && !self.has_saved_catallax_linework() && self.is_printed()
    }

    /// True when the view was drawn from `digest` or excluded from automatic update.
    pub fn accepts_revision(&self, digest: &str) -> bool {
        self.excluded_from_update || self.geometry.source_digest == digest
    }

    /// The parent view, for every kind except a base view.
    pub fn parent(&self) -> Option<ViewId> {
        match self.kind {
            ViewKind::Base { .. } => None,
            ViewKind::Projected { parent, .. }
            | ViewKind::Section { parent, .. }
            | ViewKind::Detail { parent, .. }
            | ViewKind::Auxiliary { parent, .. } => Some(parent),
        }
    }

    pub(crate) fn geometry_parent(&self) -> Option<ViewId> {
        match self.kind {
            ViewKind::Base { .. } | ViewKind::Projected { .. } => None,
            _ => self.parent(),
        }
    }

    /// The section, detail or view letter of a derived view, if it has one.
    pub fn letter(&self) -> Option<&str> {
        match &self.kind {
            ViewKind::Section { label, .. } | ViewKind::Detail { label, .. } => Some(label),
            ViewKind::Auxiliary { label, .. } if !label.is_empty() => Some(label),
            _ => None,
        }
    }
}

/// How a view is derived from the model or from another view.
// An auxiliary view carries a full entity reference; a drawing has a handful of views,
// so the larger variant costs nothing worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewKind {
    /// A view of the model from a given orientation.
    Base {
        /// Where the eye looks from.
        orientation: Orientation,
    },
    /// A view folded off a parent view.
    Projected {
        /// The view it is folded from.
        parent: ViewId,
        /// Which side of the parent it sits on.
        direction: FoldDirection,
        /// The projection angle at placement. Older files inherit the sheet's angle
        /// until it changes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        projection: Option<crate::Projection>,
    },
    /// A section through a parent view along a cutting line.
    ///
    /// The cutting plane holds the line and the parent's view direction. Seen on the
    /// parent, the arrows at the line's ends point the way the section looks: to the
    /// left of the line run from its leftmost (or, when vertical, lowest) point, unless
    /// `flip` turns them round. See [`section_arrow`](crate::project::section_arrow).
    Section {
        /// The view the cutting line is drawn on.
        parent: ViewId,
        /// The cutting line in the parent's view millimetres, from end to end. Two
        /// points make a straight cut. Three make an aligned cut: the second leg is
        /// revolved about the middle point into the plane of the first. An even number
        /// from four make an offset cut: parallel cuts joined by jogs square to them,
        /// which cut no material.
        line: Vec<Point2>,
        /// True to look the other way along the cut.
        flip: bool,
        /// The section letter, such as `"A"` for section A-A.
        label: String,
        /// The ends of the cutting line in model space. The cut stays at this place on
        /// the part when the parent's view moves; `line` is redrawn from it at every
        /// update. Empty in drawings made before phase D3: then `line` is the reference.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        model_line: Vec<Vec3>,
        /// The hatch pattern of the cut faces, an id from the library's hatch patterns;
        /// `None` uses the first.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hatch: Option<String>,
    },
    /// A magnified region of a parent view, inside a circle or a closed profile.
    Detail {
        /// The view the boundary is drawn on.
        parent: ViewId,
        /// Centre of the circle in the parent's view millimetres. The magnified view is
        /// centred on it, also for a profile.
        center: Point2,
        /// Radius of the circle in the parent's view millimetres.
        radius: f64,
        /// Magnification relative to the parent, such as 2. With
        /// [`ViewScale::Parent`] the view's scale is the parent's times this.
        factor: f64,
        /// The detail letter, such as `"B"`.
        label: String,
        /// A closed profile in the parent's view millimetres that replaces the circle as
        /// the boundary. Empty for a circle.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        profile: Vec<Point2>,
        /// The centre in model space, so the detail stays on the same place of the part
        /// when the parent's view moves. `None` in drawings made before phase D3.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_center: Option<Vec3>,
    },
    /// A view unfolded normal to a reference edge of the parent: it looks along the
    /// parent's sheet, square to the edge, so a face the parent sees edge on along that
    /// edge shows at its true size.
    Auxiliary {
        /// The view the edge is picked in.
        parent: ViewId,
        /// The reference edge.
        edge: EntityRef,
        /// The view looks from the side of the edge away from the parent's centre;
        /// true looks from the other side.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        flip: bool,
        /// The view letter, such as `"C"` for view C, or empty for none.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        label: String,
    },
}

/// A closed region in view millimetres: a crop, a detail boundary or a broken-out
/// section.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Boundary {
    /// A circle.
    Circle {
        /// Centre.
        center: Point2,
        /// Radius, greater than zero.
        radius: f64,
    },
    /// A closed polygon; the last point joins the first.
    Polygon {
        /// The corners, at least three.
        points: Vec<Point2>,
    },
}

impl Boundary {
    /// A rectangle from two opposite corners.
    pub fn rectangle(a: Point2, b: Point2) -> Boundary {
        let (x0, x1, y0, y1) = (a.u.min(b.u), a.u.max(b.u), a.v.min(b.v), a.v.max(b.v));
        Boundary::Polygon {
            points: vec![Point2::new(x0, y0), Point2::new(x1, y0), Point2::new(x1, y1), Point2::new(x0, y1)],
        }
    }

    /// True when `p` is inside (even-odd for a polygon).
    pub fn contains(&self, p: Point2) -> bool {
        match self {
            Boundary::Circle { center, radius } => (p.u - center.u).hypot(p.v - center.v) <= *radius,
            Boundary::Polygon { points } => {
                let mut inside = false;
                let n = points.len();
                for i in 0..n {
                    let (a, b) = (points[i], points[(i + n - 1) % n]);
                    if (a.v > p.v) != (b.v > p.v) && p.u < (b.u - a.u) * (p.v - a.v) / (b.v - a.v) + a.u {
                        inside = !inside;
                    }
                }
                inside
            }
        }
    }

    /// The boundary as a closed path.
    pub fn path(&self) -> BezPath {
        match self {
            Boundary::Circle { center, radius } => {
                kurbo::Shape::to_path(&kurbo::Circle::new((center.u, center.v), *radius), 1e-3)
            }
            Boundary::Polygon { points } => {
                let mut p = BezPath::new();
                for (i, q) in points.iter().enumerate() {
                    if i == 0 {
                        p.move_to((q.u, q.v));
                    } else {
                        p.line_to((q.u, q.v));
                    }
                }
                p.close_path();
                p
            }
        }
    }

    /// Corners along the boundary, a circle at `segments` points: the polygon a hatch
    /// is clipped with.
    pub fn polygon(&self, segments: usize) -> Vec<Point2> {
        match self {
            Boundary::Circle { center, radius } => (0..segments.max(8))
                .map(|i| {
                    let t = std::f64::consts::TAU * i as f64 / segments.max(8) as f64;
                    Point2::new(center.u + radius * t.cos(), center.v + radius * t.sin())
                })
                .collect(),
            Boundary::Polygon { points } => points.clone(),
        }
    }

    /// `[min x, min y, max x, max y]`.
    pub fn bounds(&self) -> [f64; 4] {
        match self {
            Boundary::Circle { center, radius } => {
                [center.u - radius, center.v - radius, center.u + radius, center.v + radius]
            }
            Boundary::Polygon { points } => {
                points.iter().fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
                    [b[0].min(p.u), b[1].min(p.v), b[2].max(p.u), b[3].max(p.v)]
                })
            }
        }
    }

    /// The boundary moved by `(du, dv)` and scaled by `k` about the origin, in that
    /// order of application: `p -> (p + d) * k`.
    pub fn mapped(&self, du: f64, dv: f64, k: f64) -> Boundary {
        let m = |p: Point2| Point2::new((p.u + du) * k, (p.v + dv) * k);
        match self {
            Boundary::Circle { center, radius } => Boundary::Circle { center: m(*center), radius: radius * k },
            Boundary::Polygon { points } => Boundary::Polygon { points: points.iter().map(|p| m(*p)).collect() },
        }
    }
}

/// A broken-out section: inside the boundary the material in front of a plane is
/// removed and the cut is hatched, the rest of the view stays as it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrokenOut {
    /// The region, in the view's millimetres.
    pub boundary: Boundary,
    /// How deep the cut goes, in model millimetres from the point of the part nearest
    /// the eye.
    pub depth: f64,
}

/// A break in a view: the stretch of the part between two model points along one sheet
/// direction is left out, and the view closes up to a gap between two break lines. The
/// line work keeps its model references and the view its scale; dimensions across the
/// break measure the model, so they keep their full length.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Break {
    /// One end of the stretch left out, a model point.
    pub from: Vec3,
    /// The other end, a model point.
    pub to: Vec3,
    /// The sheet direction the view is shortened along: horizontal leaves out a stretch
    /// of the view's width, vertical of its height.
    pub axis: AlignAxis,
}

impl Break {
    /// The gap between break lines, in sheet millimetres.
    pub const GAP: f64 = 6.0;
}

/// A half section of a symmetric part: on one side of the symmetry line the material in
/// front of the plane through that line, facing the eye, is removed and the cut is
/// hatched; the other side shows the outside. The symmetry line is drawn as a centre
/// line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HalfSection {
    /// Two model points on the symmetry line. The cutting plane holds the line and faces
    /// the eye.
    pub axis: [Vec3; 2],
    /// False cuts the side to the left of the line run from its first point to its
    /// second, as the view shows it; true cuts the right side.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
}

/// The orientation of a base view.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Orientation {
    /// One of the standard orientations.
    Standard {
        /// Which one.
        view: StandardView,
    },
    /// Any other orientation.
    Custom {
        /// Unit direction from the model towards the eye.
        direction: Vec3,
        /// Which way is up on the sheet.
        up: Vec3,
    },
}

impl Orientation {
    /// The kernel request for this orientation with the given display options.
    pub fn spec(&self, display: &ViewDisplay) -> ViewSpec {
        let mut spec = match *self {
            Orientation::Standard { view } => ViewSpec::standard(view),
            Orientation::Custom { direction, up } => ViewSpec::new(direction, up),
        };
        spec.smooth_edges = display.tangent != TangentEdges::Removed;
        spec.seams = display.seams;
        spec
    }
}

/// The side of its parent a projected view sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FoldDirection {
    /// Left of the parent.
    Left,
    /// Right of the parent.
    Right,
    /// Above the parent.
    Up,
    /// Below the parent.
    Down,
    /// Above and left of the parent: a pictorial view.
    UpLeft,
    /// Above and right of the parent: a pictorial view.
    UpRight,
    /// Below and left of the parent: a pictorial view.
    DownLeft,
    /// Below and right of the parent: a pictorial view.
    DownRight,
}

impl FoldDirection {
    /// True for the four diagonal directions, which give pictorial views.
    pub fn is_diagonal(self) -> bool {
        matches!(
            self,
            FoldDirection::UpLeft | FoldDirection::UpRight | FoldDirection::DownLeft | FoldDirection::DownRight
        )
    }
}

/// An alignment a child view keeps with its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alignment {
    /// The view it is aligned with.
    pub parent: ViewId,
    /// Which coordinate the two views share.
    pub axis: AlignAxis,
}

/// The axis of an alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignAxis {
    /// Side by side: the views share their Y and the child slides horizontally.
    Horizontal,
    /// One above the other: the views share their X and the child slides vertically.
    Vertical,
}

/// The scale of a view.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewScale {
    /// The sheet's scale. The default for base views.
    Sheet,
    /// The parent's scale. The default for child views.
    Parent,
    /// A scale of its own.
    Custom {
        /// Sheet millimetres per model millimetre, such as 0.5 for 1:2.
        factor: f64,
    },
}

/// How a view draws its lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewDisplay {
    /// Hidden lines.
    pub hidden: HiddenLines,
    /// Edges between tangent faces.
    pub tangent: TangentEdges,
    /// True to draw seams.
    #[serde(default)]
    pub seams: bool,
}

impl ViewDisplay {
    /// The default for orthographic views: hidden lines and tangent edges removed.
    pub const ORTHOGRAPHIC: ViewDisplay =
        ViewDisplay { hidden: HiddenLines::Removed, tangent: TangentEdges::Removed, seams: false };

    /// The default for pictorial views: hidden lines removed, tangent edges phantom.
    pub const PICTORIAL: ViewDisplay =
        ViewDisplay { hidden: HiddenLines::Removed, tangent: TangentEdges::Phantom, seams: false };
}

impl Default for ViewDisplay {
    fn default() -> ViewDisplay {
        ViewDisplay::ORTHOGRAPHIC
    }
}

/// Whether hidden lines are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiddenLines {
    /// Not drawn.
    Removed,
    /// Drawn dashed.
    Visible,
}

/// How edges between tangent faces are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TangentEdges {
    /// Not drawn.
    Removed,
    /// Drawn in a thin phantom line.
    Phantom,
    /// Drawn like any other edge.
    Visible,
}

/// The cached line work of a view, so a drawing opens without the kernel.
///
/// Coordinates are view millimetres: the view's scale is applied and the origin is the
/// view's [`position`](View::position), with Y up.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewGeometry {
    /// Digest of the model the geometry was made from. A view whose digest differs from
    /// the model's is out of date.
    pub source_digest: String,
    /// The transform that produced this saved line work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transform: Option<crate::project::ViewTransform>,
    /// Visible and hidden curves.
    pub curves: Vec<Curve2>,
    /// Regions to hatch, for section views.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hatch: Vec<Polygon>,
    /// The boundary a detail view is drawn inside, in view millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Boundary>,
    /// Bounds of the curves: `[min x, min y, max x, max y]`.
    pub bbox: [f64; 4],
    /// Drawn from the kernel's unproved preview, for the screen. Drawing exports refuse
    /// it and are made from certified line work instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unproved: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    /// The view uses a display section and has no exportable line work.
    pub display_only: bool,
    /// The saved line work came from Catallax drawing queries.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub catallax: bool,
    /// Screen or unknown quality, independently of whether the mesh was proved.
    /// Older caches have unknown quality and must be regenerated for export.
    #[serde(default = "unknown_quality")]
    pub coarse: bool,
    /// Where the view's breaks close up, in view millimetres, for the break lines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breaks: Vec<BreakGap>,
    /// A half section's symmetry line across the view, in view millimetres.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symmetry_segments: Vec<[Point2; 2]>,
    /// Unclipped projected model bounds of a half section, in model millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetry_bounds: Option<[f64; 4]>,
}

/// A closed-up break as drawn: two break lines `gap` apart across the view.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BreakGap {
    /// The sheet direction the view is shortened along.
    pub axis: AlignAxis,
    /// Where the first break line crosses that direction, in view millimetres.
    pub at: f64,
    /// The gap to the second break line, in sheet millimetres.
    pub gap: f64,
    #[serde(default = "mid_fraction")]
    pub(crate) centre_fraction: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) omitted_mm: Option<f64>,
}

impl BreakGap {
    pub(crate) fn along(self, p: Point2) -> f64 {
        if self.axis == AlignAxis::Vertical { p.v } else { p.u }
    }

    pub(crate) fn unshift(self, p: Point2) -> Point2 {
        let offset = self
            .omitted_mm
            .map_or(0.0, |omitted| (omitted - self.gap) * ((self.along(p) - self.at) / self.gap).clamp(0.0, 1.0));
        if self.axis == AlignAxis::Vertical { Point2::new(p.u, p.v + offset) } else { Point2::new(p.u + offset, p.v) }
    }

    pub(crate) fn trace(self, a: Point2, b: Point2) -> [[Point2; 2]; 2] {
        let (a0, b0) = (self.unshift(a), self.unshift(b));
        let (start, end) = (self.along(a0), self.along(b0));
        let far = self.at + self.omitted_mm.unwrap_or(self.gap);
        let (first, last) = if start <= end { (self.at, far) } else { (far, self.at) };
        let at = |station: f64, shift: f64| {
            let t = ((station - start) / (end - start)).clamp(0.0, 1.0);
            let p = Point2::new(a0.u + (b0.u - a0.u) * t, a0.v + (b0.v - a0.v) * t);
            if t == 0.0 {
                a
            } else if t == 1.0 {
                b
            } else if self.axis == AlignAxis::Vertical {
                Point2::new(p.u, p.v - shift)
            } else {
                Point2::new(p.u - shift, p.v)
            }
        };
        if start == end && self.contains(a) {
            return [[a, a], [b, b]];
        }
        if start == end {
            return [[a, b], [b, b]];
        }
        [
            [a, at(first, first - self.at - if first == far { self.gap } else { 0.0 })],
            [at(last, last - self.at - if last == far { self.gap } else { 0.0 }), b],
        ]
    }

    pub(crate) fn contains(self, p: Point2) -> bool {
        let p = self.along(p);
        p > self.at && p < self.at + self.gap
    }
}

fn mid_fraction() -> f64 {
    0.5
}

fn unknown_quality() -> bool {
    true
}

/// One curve of a view.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve2 {
    /// The curve in view millimetres. Stored as SVG path data with absolute commands.
    #[serde(with = "svg_path")]
    pub path: BezPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Projected endpoints of the source edge when the view clips a break.
    pub ends: Option<[Point2; 2]>,
    /// The model entity it comes from.
    pub source: EntityRef,
    /// False when the model hides it.
    pub visible: bool,
    /// What kind of curve it is.
    pub kind: CurveKind,
    /// True for an edge between tangent faces, drawn as the view's tangent edge option
    /// says.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tangent: bool,
    /// How faithfully the path follows the model. Files written before the label
    /// read as polygon, the weaker claim.
    #[serde(default)]
    pub accuracy: Accuracy,
}

/// How faithfully a curve's path follows the model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accuracy {
    /// The model's own curve, with its exact endpoints and shape.
    Exact,
    /// A polyline from the display mesh, at its chord tolerance: a silhouette, an edge
    /// split by visibility, a section crossing or a curve the kernel has not converted.
    /// A spline sampled on the sheet is labelled this way too.
    #[default]
    Polygon,
    /// A fitted curve, measured placement or cubic approximation of a circular curve.
    Approximate,
}

impl Accuracy {
    pub(crate) fn for_path(self, path: &BezPath) -> Self {
        if path.elements().iter().any(|el| matches!(el, kurbo::PathEl::CurveTo(..))) { Self::Approximate } else { self }
    }
}

/// How many curves of a view carry each accuracy label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccuracyCounts {
    /// Curves whose path is the model's own curve.
    pub exact: usize,
    /// Curves whose path is a mesh polyline.
    pub polygon: usize,
    /// Curves whose geometry or placement is approximate.
    #[serde(default)]
    pub approximate: usize,
}

impl AccuracyCounts {
    /// Adds another view's counts.
    pub fn add(&mut self, other: AccuracyCounts) {
        self.exact += other.exact;
        self.polygon += other.polygon;
        self.approximate += other.approximate;
    }
}

impl ViewGeometry {
    /// How many of the view's curves carry each accuracy label.
    pub fn accuracy(&self) -> AccuracyCounts {
        let count = |accuracy| self.curves.iter().filter(|c| c.accuracy.for_path(&c.path) == accuracy).count();
        AccuracyCounts {
            exact: count(Accuracy::Exact),
            polygon: count(Accuracy::Polygon),
            approximate: count(Accuracy::Approximate),
        }
    }
}

/// What a curve of a view shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CurveKind {
    /// An edge of the model.
    Edge,
    /// The outline of a curved face.
    Silhouette,
    /// A seam of a closed face.
    Seam,
    /// Where a section plane cuts a face.
    Section,
}

/// A closed region in view millimetres.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Polygon {
    /// The outer boundary.
    pub outer: Vec<Point2>,
    /// Boundaries of holes in the region.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<Vec<Point2>>,
    /// Only the part of the region inside this boundary counts, as in a detail of a
    /// section or a broken-out section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<Boundary>,
    /// True when a boundary of the region is approximate: fitted by the kernel or
    /// sampled on the sheet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub approximate: bool,
}

/// Serialises a [`BezPath`] as SVG path data, which is shorter and easier to read than
/// kurbo's element list. Every `f64` is written with its shortest exact form, so the
/// path reads back unchanged.
mod svg_path {
    use kurbo::BezPath;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(path: &BezPath, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&path.to_svg())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BezPath, D::Error> {
        let data = String::deserialize(deserializer)?;
        BezPath::from_svg(&data).map_err(serde::de::Error::custom)
    }
}
