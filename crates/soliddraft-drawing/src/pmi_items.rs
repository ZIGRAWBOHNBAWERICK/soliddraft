//! Manufacturing information on the drawing: the part's items, each shown once, in the
//! view that shows it best, placed by the drawing with nothing laid over anything else.
//!
//! The part states the requirements ([`Requirements`](crate::Requirements)): datums,
//! toleranced sizes and distances, slots, geometric tolerances and surface finishes, each
//! on faces of the body. The drawing shows them and adds nothing of its own. Every
//! annotation made here names the part item it shows ([`PmiSource`]), so a check can
//! tell that nothing on the drawing is missing from the part ([`not_from_part`]) and
//! that every item is there once ([`shown`]).
//!
//! Where each kind goes:
//!
//! - a diameter or radius, on the round edge of its feature in a view that looks along
//!   its axis, with the count of a pattern before it and THRU after a through hole;
//! - a threaded hole, the same way, as a callout of its designation and how deep the
//!   thread goes: `4× M6x1-6H THRU`, `M8x1-6H DEEP 12`;
//! - a distance, between the edges of its two ends in a view that sees both end on,
//!   stacked outside the view;
//! - a slot, as one callout on a round end where the slot is seen in true shape;
//! - a datum, a tolerance or a finish of a flat face, on the line the face shows as
//!   where a view sees it edge on;
//! - a tolerance of a feature that has a size on the drawing, under that size;
//! - the basic distances that locate the features of a position tolerance, as framed
//!   dimensions from the datum's line to each feature's centre, one for each distinct
//!   distance.
//!
//! An item no view can carry is not drawn and is listed by name with the reason.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use kurbo::Shape as _;
use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::drawing::Curve3;
use soliddraft_kernel_api::pmi::{At, PmiKind, SizeKind, Tolerance};
use soliddraft_kernel_api::{EdgeId, FaceId, FaceInfo, Point2, SurfaceKind, Vec3};
use soliddraft_model::pmi::PmiRow;

use crate::annotation::{
    Anchor, Annotation, AnnotationKind, AnnotationState, Attach, Dimension, DimensionKind, LinearOrientation,
    PmiSource, StyleOverrides,
};
use crate::display::{self, Item, Owner};
use crate::ids::{AnnotationId, ViewId};
use crate::measure::{AnchorGeometry, anchor_point, precision_for};
use crate::model_items::{plane_edge, visible_in};
use crate::part::{Part, PartIndex, round};
use crate::project::ViewContext;
use crate::{Drawing, DrawingError};

/// What bringing the part's items onto a sheet did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PmiReport {
    /// The names of the items drawn, in the part's order.
    pub placed: Vec<String>,
    /// The items no view could carry.
    pub unplaced: Vec<Unplaced>,
}

/// An item the drawing does not show.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unplaced {
    /// The item's name in the part.
    pub name: String,
    /// Why it is not drawn, for people.
    pub reason: String,
}

/// First dimension line from the view's outline, then the step between stacked ones, in
/// sheet millimetres (ISO 129 asks for at least 10 and 7).
const FIRST_OFFSET: f64 = 10.0;
const STACK_STEP: f64 = 8.0;
/// How far a callout's elbow sits beyond the round edge it points at.
const CALLOUT_REACH: f64 = 8.0;
/// How far a symbol on a face's line sits from that line.
const SYMBOL_OFFSET: f64 = 12.0;
/// Room kept clear around every text.
const CLEAR: f64 = 1.0;
/// Moves tried before an annotation stays where it is.
const MOVES: usize = 60;

/// What a feature is for a drawing.
#[derive(Clone, Copy, Debug)]
enum Shape {
    Plane { point: Vec3, normal: Vec3 },
    Round { centre: Vec3, axis: Vec3, radius: f64 },
}

/// The body's faces, with the round edges that give a cylinder its axis.
struct Body<'a> {
    faces: HashMap<FaceId, FaceInfo>,
    /// The cylinder each round face lies on, from the exact model.
    axes: HashMap<FaceId, soliddraft_kernel_api::FaceAxis>,
    index: &'a PartIndex<'a>,
    tol: f64,
}

impl Body<'_> {
    /// The shape of a face: a plane, or a cylinder read off a round edge of the face.
    fn face(&self, face: FaceId) -> Option<Shape> {
        let info = self.faces.get(&face)?;
        match info.surface {
            SurfaceKind::Plane => Some(Shape::Plane { point: info.centroid, normal: info.normal?.normalized()? }),
            SurfaceKind::Cylinder => {
                let axis = self.axes.get(&face)?;
                // The point of the axis level with the face's middle.
                let centre = axis.point + axis.direction * (info.centroid - axis.point).dot(axis.direction);
                Some(Shape::Round { centre, axis: axis.direction, radius: axis.radius })
            }
            _ => None,
        }
    }

    /// The shape of a feature: that of its first face.
    fn feature(&self, feature: &[FaceId]) -> Result<Shape, String> {
        let face = feature.first().ok_or("it is on no face")?;
        self.face(*face).ok_or_else(|| "only flat and round faces are drawn".to_owned())
    }

    /// The round edge of a feature that `view` shows best: the edge, where on it an
    /// annotation attaches to stand at its centre, that centre, and whether the view
    /// shows the edge as a visible line. Visible first, then nearest the eye. A round
    /// edge is any edge of the feature's faces but the straight ones along the axis.
    fn round_edge(&self, feature: &[FaceId], view: &ViewContext, axis: Vec3) -> Option<(EdgeId, Attach, Vec3, bool)> {
        let mut best: Option<((bool, f64), EdgeId, Attach, Vec3)> = None;
        for face in feature {
            for edge in &self.faces.get(face)?.edges {
                let curve = self.index.edge_curve(*edge)?;
                let (attach, centre) = match curve {
                    Curve3::Line { .. } => continue,
                    Curve3::Circle { center, .. } | Curve3::Arc { center, .. } => (Attach::Centre, *center),
                    other => {
                        let on_axis = self.axes.get(face)?.point;
                        let centre = on_axis + axis * (other.point_at(0.5) - on_axis).dot(axis);
                        (Attach::Point { at: centre }, centre)
                    }
                };
                let key = (visible_in(view, *edge), view.xf.frame.depth(centre));
                if best.as_ref().is_none_or(|(k, ..)| (key.0, key.1) > (k.0, k.1 + self.tol)) {
                    best = Some((key, *edge, attach, centre));
                }
            }
        }
        best.map(|(key, edge, attach, centre)| (edge, attach, centre, key.0))
    }
}

/// How an annotation may be moved to clear what it lies over.
#[derive(Clone, Copy, Debug)]
struct Slide {
    /// Unit direction in view millimetres.
    along: Point2,
    /// The step of one move.
    step: f64,
}

/// One annotation before it is on the sheet.
struct Plan {
    source: PmiSource,
    view: ViewId,
    body: AnnotationKind,
    anchors: Vec<(EdgeId, Attach, Point2)>,
    placement: Point2,
    slide: Slide,
    /// The plan this one moves with: the size a tolerance frame sits under.
    host: Option<usize>,
    /// For a stacked linear dimension: the stack it is in and its span.
    stack: Option<(u8, i8, f64)>,
    /// Its item turned out not to be drawable: nothing of it goes on the sheet.
    dropped: bool,
}

/// The passes over the part's items. Distances are planned and stacked first, so the
/// callouts planned after them point away from the stacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pass {
    Distances,
    Sizes,
    Rest,
}

/// One end of a distance on the body.
struct End {
    shape: Shape,
    faces: Vec<FaceId>,
    at: At,
    /// The second face of a pair, for the middle of two faces.
    pair: Option<Shape>,
}

struct Planner<'a> {
    body: Body<'a>,
    views: Vec<&'a ViewContext>,
    bbox: BTreeMap<ViewId, [f64; 4]>,
    plans: Vec<Plan>,
    /// Diagonals taken around each round centre, by view.
    diagonals_taken: BTreeMap<(ViewId, i64, i64), Vec<(i8, i8)>>,
    /// Symbols already on each edge, by view.
    on_edge: BTreeMap<(ViewId, EdgeId), usize>,
    /// The plan of the size on each feature, by its first face.
    sizes: BTreeMap<FaceId, usize>,
    /// The sides of each view that carry stacked dimensions: view, vertical, side.
    stacked: BTreeSet<(ViewId, u8, i8)>,
    /// For each view, the way that leads away from the other views: across, then up.
    away: BTreeMap<ViewId, (i8, i8)>,
    /// The side the basic distances from each datum take in each view.
    datum_sides: BTreeMap<(ViewId, u8, String), i8>,
    pass: Pass,
    /// Text height and leader shoulder of the sheet's style.
    text_height: f64,
    shoulder: f64,
    times: String,
    through: String,
    depth: String,
}

fn unit(p: Point2) -> Point2 {
    let length = p.u.hypot(p.v);
    if length > 1e-12 { Point2::new(p.u / length, p.v / length) } else { Point2::new(0.0, 1.0) }
}

impl Planner<'_> {
    /// The diagonals of a view in the order callouts take them: those that point at
    /// no stack of dimensions first, and of those the right-hand ones, so a frame can
    /// sit under the text.
    fn diagonals(&self, view: ViewId) -> [(i8, i8); 4] {
        let mut order = [(1, 1), (1, -1), (-1, 1), (-1, -1)];
        order.sort_by_key(|(across, up)| {
            i32::from(self.stacked.contains(&(view, 1, *across))) + i32::from(self.stacked.contains(&(view, 0, *up)))
        });
        order
    }

    /// The next free diagonal around a round centre.
    fn diagonal(&mut self, view: ViewId, centre: Point2) -> Point2 {
        let order = self.diagonals(view);
        let key = (view, (centre.u * 1e3).round() as i64, (centre.v * 1e3).round() as i64);
        let taken = self.diagonals_taken.entry(key).or_default();
        let dir = order.into_iter().find(|d| !taken.contains(d)).unwrap_or(order[0]);
        taken.push(dir);
        let s = std::f64::consts::FRAC_1_SQRT_2;
        Point2::new(f64::from(dir.0) * s, f64::from(dir.1) * s)
    }

    /// How many annotations are planned in a view.
    fn planned_in(&self, view: ViewId) -> i32 {
        self.plans.iter().filter(|plan| !plan.dropped && plan.view == view).count() as i32
    }

    /// A callout on a round edge: a diameter, a radius, a slot's callout. Of the
    /// features of a pattern it points at the one furthest out the way callouts go in
    /// its view, so the leader is short and crosses nothing.
    fn callout(
        &mut self,
        features: &[&[FaceId]],
        axis: Vec3,
        radius: f64,
    ) -> Result<(ViewId, EdgeId, Attach, Point2, Point2, Point2), String> {
        let first = features.first().copied().ok_or("it is on no feature")?;
        let mut best: Option<((bool, i32), &ViewContext)> = None;
        for (order, view) in self.views.iter().enumerate() {
            if axis.dot(view.xf.frame.direction).abs() < 1.0 - 1e-6 {
                continue;
            }
            let Some((_, _, _, visible)) = self.body.round_edge(first, view, axis) else { continue };
            let key = (visible, -(order as i32));
            if best.as_ref().is_none_or(|(k, _)| key > *k) {
                best = Some((key, view));
            }
        }
        let (_, view) = best.ok_or("no view looks along its axis")?;
        let lead = self.diagonals(view.id)[0];
        let mut chosen: Option<(f64, EdgeId, Attach, Point2)> = None;
        for feature in features {
            let Some((edge, attach, centre, _)) = self.body.round_edge(feature, view, axis) else { continue };
            let at = view.xf.point(centre);
            let out = at.u * f64::from(lead.0) + at.v * f64::from(lead.1);
            if chosen.as_ref().is_none_or(|(o, ..)| out > *o + 1e-9) {
                chosen = Some((out, edge, attach, at));
            }
        }
        let (_, edge, attach, centre) = chosen.ok_or("no view looks along its axis")?;
        let (id, scale) = (view.id, view.xf.scale);
        let dir = self.diagonal(id, centre);
        let reach = radius * scale + CALLOUT_REACH;
        let placement = Point2::new(round(centre.u + dir.u * reach), round(centre.v + dir.v * reach));
        Ok((id, edge, attach, centre, placement, dir))
    }

    /// The outward side of a view at a point: the unit direction to the nearest side of
    /// the view's outline along `across`.
    fn outward(&self, view: ViewId, at: Point2, across: Point2) -> Point2 {
        let bbox = self.bbox.get(&view).copied().unwrap_or([0.0; 4]);
        let (lo, hi, here) =
            if across.u.abs() > across.v.abs() { (bbox[0], bbox[2], at.u) } else { (bbox[1], bbox[3], at.v) };
        let sign = if here - lo < hi - here { -1.0 } else { 1.0 };
        Point2::new(across.u.abs() * sign, across.v.abs() * sign)
    }

    /// A symbol on the line a flat face shows as: the view, the edge, the point on it and
    /// the outward direction.
    fn on_line(&mut self, point: Vec3, normal: Vec3) -> Result<(ViewId, EdgeId, Attach, Point2, Point2), String> {
        let mut best: Option<((bool, i32, i32), &ViewContext, EdgeId)> = None;
        for (order, view) in self.views.iter().enumerate() {
            let frame = &view.xf.frame;
            if normal.dot(frame.direction).abs() > 1e-6 {
                continue;
            }
            if normal.dot(frame.right).abs() < 1.0 - 1e-6 && normal.dot(frame.up).abs() < 1.0 - 1e-6 {
                continue;
            }
            let Some((edge, visible)) = plane_edge(self.body.index, view, point, normal, self.body.tol) else {
                continue;
            };
            // A visible line first, then the view with the fewest annotations so far.
            let key = (visible, -self.planned_in(view.id), -(order as i32));
            if best.as_ref().is_none_or(|(k, ..)| key > *k) {
                best = Some((key, view, edge));
            }
        }
        let (_, view, edge) = best.ok_or("no view sees the face edge on")?;
        let curve = self.body.index.edge_curve(edge).ok_or("the edge is gone")?;
        let taken = self.on_edge.entry((view.id, edge)).or_default();
        let fraction = [0.25, 0.7, 0.5, 0.1, 0.9][*taken % 5];
        *taken += 1;
        let at = curve.point_at(fraction);
        let on = view.xf.point(at);
        let across = unit(view.xf.vector(normal));
        let out = self.outward(view.id, on, across);
        Ok((view.id, edge, Attach::Point { at }, on, out))
    }

    /// The anchor of one end of a distance in a view: the edge, where on it, the point
    /// in the view, whether the view shows the edge as a visible line and whether it
    /// sees a round end face on. `lean` is the direction across the measuring one that
    /// the dimension line lies in; with it a flat end is taken at its near end, so
    /// extension lines start where the dimension is. Without it, at its middle.
    fn end_anchor(
        &self,
        end: &End,
        view: &ViewContext,
        direction: Vec3,
        towards: f64,
        lean: Option<Point2>,
    ) -> Option<(EdgeId, Attach, Point2, bool, bool)> {
        match end.shape {
            Shape::Plane { point, normal } => {
                let (edge, visible) = plane_edge(self.body.index, view, point, normal, self.body.tol)?;
                let curve = self.body.index.edge_curve(edge)?;
                let near = match lean {
                    None => Attach::Mid,
                    Some(lean) => match curve {
                        Curve3::Line { .. } => {
                            let along = |p: Vec3| {
                                let q = view.xf.point(p);
                                q.u * lean.u + q.v * lean.v
                            };
                            if along(curve.end()) > along(curve.start()) { Attach::End } else { Attach::Start }
                        }
                        _ => Attach::Max,
                    },
                };
                let across = lean.unwrap_or(Point2::new(0.0, 1.0));
                let at_near = anchor_point(
                    &AnchorGeometry { curve: Some(curve), point: Vec3::ZERO, attach: near },
                    &view.xf,
                    across,
                );
                let (attach, model) = match (end.at, end.pair) {
                    (At::Middle, Some(Shape::Plane { point: other, .. })) => {
                        let at = at_near + normal * ((other - point).dot(normal) / 2.0);
                        (Attach::Point { at }, at)
                    }
                    _ => (near, at_near),
                };
                Some((edge, attach, view.xf.point(model), visible, false))
            }
            Shape::Round { axis, radius, .. } => {
                let (edge, centred, on_axis, visible) = self.body.round_edge(&end.faces, view, axis)?;
                let attach = match end.at {
                    At::Inner => Attach::Point { at: on_axis + direction * (radius * towards) },
                    At::Outer => Attach::Point { at: on_axis - direction * (radius * towards) },
                    _ => centred,
                };
                let model = match attach {
                    Attach::Point { at } => at,
                    _ => on_axis,
                };
                let face_on = axis.dot(view.xf.frame.direction).abs() > 1.0 - 1e-6;
                Some((edge, attach, view.xf.point(model), visible, face_on))
            }
        }
    }

    /// A linear dimension between two ends.
    /// `datum` is the letter of the datum a basic distance is measured from: all the
    /// basic distances from one datum in a view go on one side, the one that leads away
    /// from the other views.
    fn linear(
        &mut self,
        datum: Option<&str>,
        source: PmiSource,
        from: &End,
        to: &End,
        value: f64,
        tolerance: Tolerance,
    ) -> Result<(), String> {
        let position = |end: &End| match end.shape {
            Shape::Plane { point, .. } => point,
            Shape::Round { centre, .. } => centre,
        };
        let direction = match (from.shape, to.shape) {
            (Shape::Plane { normal, .. }, _) | (_, Shape::Plane { normal, .. }) => normal,
            (Shape::Round { axis, .. }, Shape::Round { .. }) => {
                let between = position(to) - position(from);
                (between - axis * between.dot(axis)).normalized().ok_or("its two ends share an axis")?
            }
        };
        // From the first end towards the second.
        let towards = if (position(to) - position(from)).dot(direction) >= 0.0 { 1.0 } else { -1.0 };
        let mut best: Option<(i32, Plan)> = None;
        for (order, view) in self.views.iter().enumerate() {
            let frame = &view.xf.frame;
            if direction.dot(frame.direction).abs() > 1e-6 {
                continue;
            }
            let orientation = if direction.dot(frame.right).abs() > 1.0 - 1e-6 {
                LinearOrientation::Horizontal
            } else if direction.dot(frame.up).abs() > 1.0 - 1e-6 {
                LinearOrientation::Vertical
            } else {
                continue;
            };
            let (Some(a), Some(b)) = (
                self.end_anchor(from, view, direction, towards, None),
                self.end_anchor(to, view, direction, -towards, None),
            ) else {
                continue;
            };
            let vertical = orientation == LinearOrientation::Vertical;
            // Across the measuring direction, the side the two ends lean to.
            let lean = if vertical { a.2.u + b.2.u } else { a.2.v + b.2.v };
            let leaning: i8 = if lean >= 0.0 { 1 } else { -1 };
            let side = match datum {
                Some(letter) => {
                    let away = self.away.get(&view.id).map_or(0, |a| if vertical { a.0 } else { a.1 });
                    *self.datum_sides.entry((view.id, u8::from(vertical), letter.to_owned())).or_insert(if away == 0 {
                        leaning
                    } else {
                        away
                    })
                }
                None => leaning,
            };
            let across = if vertical { Point2::new(f64::from(side), 0.0) } else { Point2::new(0.0, f64::from(side)) };
            let (Some(a), Some(b)) = (
                self.end_anchor(from, view, direction, towards, Some(across)),
                self.end_anchor(to, view, direction, -towards, Some(across)),
            ) else {
                continue;
            };
            if a.0 == b.0 && a.1 == b.1 {
                continue;
            }
            let score = (4 * (i32::from(a.4) + i32::from(b.4)) + i32::from(a.3) + i32::from(b.3)) * 100 - order as i32;
            if best.as_ref().is_some_and(|(s, _)| score <= *s) {
                continue;
            }
            let mut dimension = Dimension::measured(DimensionKind::Linear { orientation }, value, precision_for(value));
            dimension.tolerance = tolerance;
            dimension.requirement = true;
            let plan = Plan {
                source: source.clone(),
                view: view.id,
                body: AnnotationKind::Dimension(dimension),
                anchors: vec![(a.0, a.1, a.2), (b.0, b.1, b.2)],
                placement: Point2::default(),
                slide: Slide { along: across, step: STACK_STEP },
                host: None,
                stack: Some((u8::from(vertical), side, value * view.xf.scale)),
                dropped: false,
            };
            best = Some((score, plan));
        }
        let (_, plan) = best.ok_or("no view sees both of its ends on edge")?;
        self.plans.push(plan);
        Ok(())
    }

    fn row(&mut self, row: &PmiRow, rows: &[PmiRow]) -> Result<(), String> {
        if let Some(problem) = &row.problem {
            return Err(problem.clone());
        }
        let item = row.on_body.as_ref().ok_or("its faces are not on the body")?;
        let wanted = match &item.kind {
            PmiKind::Location { .. } => self.pass == Pass::Distances,
            PmiKind::Size { .. } | PmiKind::Hole { .. } => self.pass == Pass::Sizes,
            PmiKind::Gdt { .. } => self.pass != Pass::Sizes,
            _ => self.pass == Pass::Rest,
        };
        if !wanted {
            return Ok(());
        }
        let source = source_of(row);
        let no_slide = Slide { along: Point2::new(0.0, 1.0), step: 3.0 };
        match &item.kind {
            PmiKind::Datum { letter, feature } => match self.body.feature(feature)? {
                Shape::Plane { point, normal } => {
                    let (view, edge, attach, on, out) = self.on_line(point, normal)?;
                    self.plans.push(Plan {
                        source,
                        view,
                        body: AnnotationKind::Datum { letter: letter.clone() },
                        anchors: vec![(edge, attach, on)],
                        placement: Point2::new(
                            round(on.u + out.u * SYMBOL_OFFSET),
                            round(on.v + out.v * SYMBOL_OFFSET),
                        ),
                        slide: Slide { along: out, step: 3.0 },
                        host: None,
                        stack: None,
                        dropped: false,
                    });
                }
                Shape::Round { axis, radius, .. } => {
                    let (view, edge, _, centre, placement, dir) = self.callout(&[feature.as_slice()], axis, radius)?;
                    let scale = self.views.iter().find(|v| v.id == view).map_or(1.0, |v| v.xf.scale);
                    let on = Point2::new(centre.u + dir.u * radius * scale, centre.v + dir.v * radius * scale);
                    let at = self.views.iter().find(|v| v.id == view).map(|v| v.xf.unproject(on));
                    self.plans.push(Plan {
                        source,
                        view,
                        body: AnnotationKind::Datum { letter: letter.clone() },
                        anchors: vec![(edge, at.map_or(Attach::Mid, |at| Attach::Point { at }), on)],
                        placement,
                        slide: Slide { along: dir, step: 3.0 },
                        host: None,
                        stack: None,
                        dropped: false,
                    });
                }
            },
            PmiKind::Size { features, through, thread: Some(thread), .. } => {
                let first = features.first().ok_or("it is on no feature")?;
                let Shape::Round { axis, radius, .. } = self.body.feature(first)? else {
                    return Err("a thread needs a round face".into());
                };
                let all: Vec<&[FaceId]> = features.iter().map(Vec::as_slice).collect();
                // The leader reaches the wall on the side callouts go, as a slot's does.
                let (view, edge, _, centre, placement, dir) = self.callout(&all, axis, radius)?;
                let scale = self.views.iter().find(|v| v.id == view).map_or(1.0, |v| v.xf.scale);
                let on = Point2::new(centre.u + dir.u * radius * scale, centre.v + dir.v * radius * scale);
                let at = self.views.iter().find(|v| v.id == view).map(|v| v.xf.unproject(on));
                let text =
                    thread_text(features.len(), thread, *through, row.depth, &self.times, &self.through, &self.depth);
                if let Some(face) = first.first() {
                    self.sizes.insert(*face, self.plans.len());
                }
                let placement_on_wall = row.feature.and_then(|feature| {
                    thread_placement(&self.body.index.edge_ref(edge)?, feature)
                        .filter(|placement| row.placements.contains(placement))
                });
                let source = source_on(row, placement_on_wall);
                self.plans.push(Plan {
                    source,
                    view,
                    body: AnnotationKind::Note { text, leader: Vec::new() },
                    anchors: vec![(edge, at.map_or(Attach::Mid, |at| Attach::Point { at }), on)],
                    placement,
                    slide: Slide { along: dir, step: 3.0 },
                    host: None,
                    stack: None,
                    dropped: false,
                });
            }
            PmiKind::Size { size, features, tolerance, through, .. } => {
                let first = features.first().ok_or("it is on no feature")?;
                let Shape::Round { axis, radius, .. } = self.body.feature(first)? else {
                    return Err("a size needs a round face".into());
                };
                let all: Vec<&[FaceId]> = features.iter().map(Vec::as_slice).collect();
                let (view, edge, centred, centre, placement, dir) = self.callout(&all, axis, radius)?;
                let (kind, value) = match size {
                    SizeKind::Diameter => (DimensionKind::Diameter, 2.0 * radius),
                    SizeKind::Radius => (DimensionKind::Radius, radius),
                };
                let value = row.measured.first().copied().unwrap_or(value);
                let mut dimension = Dimension::measured(kind, value, precision_for(value));
                dimension.tolerance = *tolerance;
                dimension.requirement = true;
                dimension.text = size_text(features.len(), *through, &self.times, &self.through);
                if let Some(face) = first.first() {
                    self.sizes.insert(*face, self.plans.len());
                }
                self.plans.push(Plan {
                    source,
                    view,
                    body: AnnotationKind::Dimension(dimension),
                    anchors: vec![(edge, centred, centre)],
                    placement,
                    slide: Slide { along: dir, step: 3.0 },
                    host: None,
                    stack: None,
                    dropped: false,
                });
            }
            PmiKind::Location { from, to, tolerance, .. } => {
                let end = |anchor: &soliddraft_kernel_api::pmi::Anchor<FaceId>| -> Result<End, String> {
                    let shape = self.body.feature(&anchor.feature)?;
                    let pair = anchor.feature.get(1).and_then(|face| self.body.face(*face));
                    Ok(End { shape, faces: anchor.feature.clone(), at: anchor.at, pair })
                };
                let (from, to) = (end(from)?, end(to)?);
                let value = row.measured.first().copied().ok_or("the part could not measure it")?;
                self.linear(None, source, &from, &to, value, *tolerance)?;
            }
            PmiKind::Slot { ends, .. } => {
                let Shape::Round { axis, radius, .. } = self.body.feature(&ends[0])? else {
                    return Err("a slot needs round ends".into());
                };
                // The leader reaches the end on the side callouts go, so it lands on the round.
                let (view, edge, _, centre, placement, dir) =
                    self.callout(&[ends[0].as_slice(), ends[1].as_slice()], axis, radius)?;
                let scale = self.views.iter().find(|v| v.id == view).map_or(1.0, |v| v.xf.scale);
                let on = Point2::new(centre.u + dir.u * radius * scale, centre.v + dir.v * radius * scale);
                let at = self.views.iter().find(|v| v.id == view).map(|v| v.xf.unproject(on));
                self.plans.push(Plan {
                    source,
                    view,
                    body: AnnotationKind::Note { text: row.callout(), leader: Vec::new() },
                    anchors: vec![(edge, at.map_or(Attach::Mid, |at| Attach::Point { at }), on)],
                    placement,
                    slide: Slide { along: dir, step: 3.0 },
                    host: None,
                    stack: None,
                    dropped: false,
                });
            }
            PmiKind::Gdt { characteristic, tolerance, diameter, material, datums, features } => {
                if self.pass == Pass::Distances {
                    return self.located(row, rows, features);
                }
                let first = features.first().ok_or("it is on no feature")?;
                let body = AnnotationKind::Gdt {
                    characteristic: *characteristic,
                    tolerance: *tolerance,
                    diameter: *diameter,
                    material: *material,
                    datums: datums.clone(),
                };
                let th = self.text_height;
                if let Some(host) = first.first().and_then(|face| self.sizes.get(face)).copied() {
                    // Under the size of the same feature, starting where its text starts.
                    let (at, right, view) =
                        (self.plans[host].placement, self.plans[host].slide.along.u >= 0.0, self.plans[host].view);
                    let below = at.v - th / 2.0 - CLEAR - th;
                    let u = if right {
                        at.u + self.shoulder + 1.0
                    } else {
                        at.u - self.shoulder - 1.0 - gdt_width(&body, th)
                    };
                    self.plans.push(Plan {
                        source: source.clone(),
                        view,
                        body,
                        anchors: Vec::new(),
                        placement: Point2::new(round(u), round(below)),
                        slide: no_slide,
                        host: Some(host),
                        stack: None,
                        dropped: false,
                    });
                } else {
                    let Shape::Plane { point, normal } = self.body.feature(first)? else {
                        return Err("its feature has no size on the drawing to sit under".into());
                    };
                    let (view, edge, attach, on, out) = self.on_line(point, normal)?;
                    let reach = SYMBOL_OFFSET + 2.0 * th;
                    self.plans.push(Plan {
                        source: source.clone(),
                        view,
                        body,
                        anchors: vec![(edge, attach, on)],
                        placement: Point2::new(round(on.u + out.u * reach + 3.0), round(on.v + out.v * reach)),
                        slide: Slide { along: out, step: 3.0 },
                        host: None,
                        stack: None,
                        dropped: false,
                    });
                }
            }
            PmiKind::SurfaceFinish { text, process, method, lay, allowance, all_around, feature } => {
                let body = AnnotationKind::SurfaceFinish {
                    text: text.clone(),
                    process: *process,
                    method: method.clone(),
                    lay: *lay,
                    allowance: allowance.clone(),
                    all_around: *all_around,
                };
                match self.body.feature(feature)? {
                    Shape::Plane { point, normal } => {
                        let (view, edge, attach, on, out) = self.on_line(point, normal)?;
                        self.plans.push(Plan {
                            source,
                            view,
                            body,
                            anchors: vec![(edge, attach, on)],
                            placement: Point2::new(
                                round(on.u + out.u * SYMBOL_OFFSET),
                                round(on.v + out.v * SYMBOL_OFFSET),
                            ),
                            slide: Slide { along: out, step: 3.0 },
                            host: None,
                            stack: None,
                            dropped: false,
                        });
                    }
                    Shape::Round { axis, radius, .. } => {
                        let (view, edge, _, centre, placement, dir) =
                            self.callout(&[feature.as_slice()], axis, radius)?;
                        let scale = self.views.iter().find(|v| v.id == view).map_or(1.0, |v| v.xf.scale);
                        let on = Point2::new(centre.u + dir.u * radius * scale, centre.v + dir.v * radius * scale);
                        let at = self.views.iter().find(|v| v.id == view).map(|v| v.xf.unproject(on));
                        self.plans.push(Plan {
                            source,
                            view,
                            body,
                            anchors: vec![(edge, at.map_or(Attach::Mid, |at| Attach::Point { at }), on)],
                            placement,
                            slide: Slide { along: dir, step: 3.0 },
                            host: None,
                            stack: None,
                            dropped: false,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// The basic distances that locate a position tolerance's features from its datums,
    /// one dimension for each distinct distance from each datum.
    fn located(&mut self, row: &PmiRow, rows: &[PmiRow], features: &[Vec<FaceId>]) -> Result<(), String> {
        let mut seen: BTreeSet<(String, i64)> = BTreeSet::new();
        for located in &row.located {
            let micrometres = (located.value * 1000.0).round() as i64;
            if !seen.insert((located.datum.clone(), micrometres)) {
                continue;
            }
            let datum = rows.iter().find_map(|other| match &other.on_body.as_ref()?.kind {
                PmiKind::Datum { letter, feature } if *letter == located.datum => Some(feature.clone()),
                _ => None,
            });
            let (Some(datum), Some(feature)) = (datum, features.get(located.feature)) else { continue };
            let from = End { shape: self.body.feature(&datum)?, faces: datum, at: At::Feature, pair: None };
            let to = End { shape: self.body.feature(feature)?, faces: feature.clone(), at: At::Feature, pair: None };
            let source = PmiSource::Located { item: row.item.id.clone(), datum: located.datum.clone(), micrometres };
            self.linear(Some(&located.datum), source, &from, &to, located.value, Tolerance::Basic)?;
        }
        Ok(())
    }

    /// Stacks the linear dimensions outside their views, shorter spans nearer the outline.
    fn stack(&mut self) {
        let mut stacks: BTreeMap<(ViewId, u8, i8), Vec<usize>> = BTreeMap::new();
        for (i, plan) in self.plans.iter().enumerate() {
            if let Some((vertical, side, _)) = plan.stack
                && !plan.dropped
            {
                stacks.entry((plan.view, vertical, side)).or_default().push(i);
                self.stacked.insert((plan.view, vertical, side));
            }
        }
        for ((view, vertical, side), mut members) in stacks {
            let bbox = self.bbox.get(&view).copied().unwrap_or([0.0; 4]);
            members.sort_by(|a, b| {
                let span = |i: &usize| self.plans[*i].stack.map_or(0.0, |s| s.2);
                span(a).total_cmp(&span(b))
            });
            for (k, i) in members.into_iter().enumerate() {
                let offset = FIRST_OFFSET + STACK_STEP * k as f64;
                let side = f64::from(side);
                let (a, b) = (self.plans[i].anchors[0].2, self.plans[i].anchors[1].2);
                self.plans[i].placement = if vertical == 1 {
                    let edge = if side > 0.0 { bbox[2] } else { bbox[0] };
                    Point2::new(round(edge + side * offset), round((a.v + b.v) / 2.0))
                } else {
                    let edge = if side > 0.0 { bbox[3] } else { bbox[1] };
                    Point2::new(round((a.u + b.u) / 2.0), round(edge + side * offset))
                };
            }
        }
    }
}

/// The width of a tolerance frame at text height `th`, as the display draws it.
fn gdt_width(body: &AnnotationKind, th: f64) -> f64 {
    let AnnotationKind::Gdt { tolerance, diameter, material, datums, .. } = body else { return 0.0 };
    let (h, pad) = (2.0 * th, th * 0.45);
    let modifier = |regardless: bool| if regardless { 0.0 } else { th * 1.3 };
    let value = format!("{}{}", if *diameter { "Ø" } else { "" }, display::tolerance_text(*tolerance));
    let mut width = h + display::text_width(&value, th) + 2.0 * pad + modifier(material.is_regardless());
    for datum in datums {
        width += (display::text_width(&datum.letter, th) + 2.0 * pad + modifier(datum.material.is_regardless())).max(h);
    }
    width
}

/// What a size's dimension shows around its value: the count of a pattern before it and
/// the through mark after it, as in `4× <DIM> THRU`.
fn size_text(count: usize, through: bool, times: &str, through_mark: &str) -> String {
    let count = if count > 1 { format!("{count}{times} ") } else { String::new() };
    let through = if through { format!(" {through_mark}") } else { String::new() };
    format!("{count}{}{through}", Dimension::VALUE_TEXT)
}

/// What a threaded hole's callout says: the count of a pattern, the designation, then
/// the through mark or how deep the thread goes, as in `4× M6x1-6H THRU` or
/// `M8x1-6H DEEP 12`.
fn thread_text(
    count: usize,
    designation: &str,
    through: bool,
    depth: Option<f64>,
    times: &str,
    through_mark: &str,
    depth_mark: &str,
) -> String {
    let count = if count > 1 { format!("{count}{times} ") } else { String::new() };
    let end = match (through, depth) {
        (true, _) => format!(" {through_mark}"),
        (false, Some(depth)) => format!(" {depth_mark} {}", soliddraft_kernel_api::pmi::length_text(depth)),
        (false, None) => String::new(),
    };
    format!("{count}{designation}{end}")
}

/// The part's notes as the sheet writes them: the template's heading, then each note
/// with its number.
fn notes_text(heading: &str, notes: &[String]) -> String {
    std::iter::once(heading.to_owned())
        .chain(notes.iter().enumerate().map(|(i, note)| format!("{}. {note}", i + 1)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn notes_heading(template: &str) -> String {
    crate::library::template(template).notes.as_ref().map_or_else(|| "NOTES".to_owned(), |n| n.heading.clone())
}

/// Puts the part's notes on sheet `sheet`, in the template's notes block at the bottom
/// left of the drawing area. The general tolerance and the material are not repeated
/// here: the title block shows them. Does nothing when the part has no other note or the
/// sheet shows them already.
pub fn place_notes(drawing: &mut Drawing, sheet: usize, part: &Part) {
    let notes = &part.requirements.notes.notes;
    let Some(page) = drawing.sheets.get(sheet) else { return };
    if notes.is_empty() || page.annotations.iter().any(|a| a.source == Some(PmiSource::Notes)) {
        return;
    }
    let template = crate::library::template(page.template.as_str());
    let (offset, height) = template.notes.as_ref().map_or(([8.0, 6.0], 2.5), |n| (n.offset, n.height));
    let [left, bottom, _, _] = template.drawing_area();
    let lines = notes.len() as f64 + 1.0;
    let annotation = Annotation {
        id: drawing.next_annotation_id(),
        view: crate::ids::SHEET,
        body: AnnotationKind::Note {
            text: notes_text(&notes_heading(page.template.as_str()), notes),
            leader: Vec::new(),
        },
        anchors: Vec::new(),
        placement: Point2::new(left + offset[0], bottom + lines * height * 1.7),
        style: StyleOverrides { text_height: Some(height), ..Default::default() },
        state: AnnotationState::Ok,
        source: Some(PmiSource::Notes),
    };
    let id = annotation.id;
    drawing.sheets[sheet].annotations.push(annotation);
    // Along the bottom of the drawing area until it is clear of what is there.
    let others: Vec<AnnotationId> =
        drawing.sheets[sheet].annotations.iter().map(|a| a.id).filter(|a| *a != id).collect();
    let planned = places(drawing, &[id]);
    for _ in 0..MOVES {
        let laid = lay(drawing, sheet);
        if laid.clash(&[id], &others).is_none() {
            return;
        }
        if laid.off_frame(&[id]) {
            break;
        }
        if let Some(a) = drawing.annotation_mut(id) {
            a.placement = Point2::new(round(a.placement.u + 5.0), a.placement.v);
        }
    }
    put(drawing, &[id], &planned);
}

/// What an annotation made from `row` names: an item the part stores by its name, the
/// thread a hole states by the hole, so renaming the hole or giving another item the
/// thread's name does not part the annotation from its thread.
fn source_of(row: &PmiRow) -> PmiSource {
    source_on(row, None)
}

fn source_on(row: &PmiRow, placement: Option<usize>) -> PmiSource {
    match row.feature {
        Some(feature) => PmiSource::Thread {
            feature: feature.0,
            placement: placement.or_else(|| row.placements.first().copied()).unwrap_or(0) as u32,
            item: row.item.id.clone(),
        },
        None => PmiSource::Item { item: row.item.id.clone() },
    }
}

fn thread_placement(entity: &crate::EntityRef, feature: soliddraft_model::FeatureId) -> Option<usize> {
    let Some(crate::ModelEntity::Edge(edge)) = &entity.model else { return None };
    edge.faces.iter().find_map(|face| match face.generator {
        soliddraft_model::Generator::Hole { placement, .. } if face.feature == feature => Some(placement as usize),
        _ => None,
    })
}

fn annotation_row<'a>(
    annotation: &Annotation,
    source: &PmiSource,
    rows: &'a [PmiRow],
) -> Option<(&'a PmiRow, Option<usize>)> {
    let PmiSource::Thread { feature, placement, .. } = source else {
        return row_of(source, rows).map(|row| (row, None));
    };
    let feature = soliddraft_model::FeatureId(*feature);
    let anchored = annotation.anchors.iter().find_map(|anchor| thread_placement(&anchor.entity, feature));
    for placement in anchored.into_iter().chain([*placement as usize]) {
        if let Some(row) = rows.iter().find(|row| row.feature == Some(feature) && row.placements.contains(&placement)) {
            return Some((row, Some(placement)));
        }
    }
    None
}

/// The row an annotation made from the part shows: an item the part stores by its name,
/// a hole's thread by the hole and one of its holes. An annotation made before threads
/// were followed by their holes names a thread by its name, when no stored item has it.
fn row_of<'r>(source: &PmiSource, rows: &'r [PmiRow]) -> Option<&'r PmiRow> {
    match source {
        PmiSource::Item { item } => rows
            .iter()
            .find(|row| row.feature.is_none() && row.item.id == *item)
            .or_else(|| rows.iter().find(|row| row.feature.is_some() && row.item.id == *item)),
        PmiSource::Thread { feature, placement, .. } => rows.iter().find(|row| {
            row.feature.is_some_and(|f| f.0 == *feature) && row.placements.contains(&(*placement as usize))
        }),
        PmiSource::Located { .. } | PmiSource::Notes => None,
    }
}

/// True when an annotation of kind `body` is how the drawing shows an item of kind
/// `kind`: a thread as a note, a plain size as a dimension, and so on.
fn shows<F>(body: &AnnotationKind, kind: &PmiKind<F>) -> bool {
    matches!(
        (body, kind),
        (AnnotationKind::Note { .. }, PmiKind::Size { thread: Some(_), .. } | PmiKind::Slot { .. })
            | (AnnotationKind::Dimension(_), PmiKind::Size { thread: None, .. } | PmiKind::Location { .. })
            | (AnnotationKind::Gdt { .. }, PmiKind::Gdt { .. })
            | (AnnotationKind::Datum { .. }, PmiKind::Datum { .. })
            | (AnnotationKind::SurfaceFinish { .. }, PmiKind::SurfaceFinish { .. })
    )
}

/// Brings every annotation made from the part's items up to the part as it is now: its
/// tolerance, its count, its letter, its text. An annotation whose item the part no
/// longer has, or that no longer applies, is taken off the drawing. One whose item is now
/// shown another way, as a size that became a thread or a thread that became a plain
/// size, is taken off too when `redraw` says [`import`] runs next to draw the item again
/// as it is; otherwise it stays as it was until then. Returns the names of the items
/// taken off for good.
pub(crate) fn refresh(
    drawing: &mut Drawing,
    part: &Part,
    refresh_view: impl Fn(ViewId) -> bool,
    redraw: bool,
) -> Vec<String> {
    let all = &part.requirements.rows;
    let rows: BTreeMap<&str, &PmiRow> =
        all.iter().filter(|row| row.feature.is_none()).map(|row| (row.item.id.as_str(), row)).collect();
    let notes = &part.requirements.notes.notes;
    let mut removed = Vec::new();
    let mut thread_notes = BTreeMap::new();
    if redraw {
        for annotation in drawing.sheets.iter().flat_map(|sheet| &sheet.annotations) {
            let Some((row, _)) = annotation.source.as_ref().and_then(|source| annotation_row(annotation, source, all))
            else {
                continue;
            };
            if row.feature.is_none()
                || !row.applies()
                || !shows(&annotation.body, &row.item.kind)
                || !refresh_view(annotation.view)
            {
                continue;
            }
            let resolved = !matches!(annotation.state, AnnotationState::Dangling { .. });
            // A merged group keeps a note whose leader still reaches a hole, when it has one.
            let kept = thread_notes.entry(row.item.id.as_str()).or_insert((annotation.id, resolved));
            if resolved && !kept.1 {
                *kept = (annotation.id, resolved);
            }
        }
    }
    for sheet in &mut drawing.sheets {
        let style = crate::library::template(sheet.template.as_str()).style_file();
        let heading = notes_heading(sheet.template.as_str());
        sheet.annotations.retain_mut(|annotation| {
            if !refresh_view(annotation.view) {
                return true;
            }
            let Some(source) = annotation.source.clone() else { return true };
            let mut redrawn = false;
            let keep = match &source {
                PmiSource::Notes => {
                    if let AnnotationKind::Note { text, .. } = &mut annotation.body {
                        *text = notes_text(&heading, notes);
                    }
                    !notes.is_empty()
                }
                PmiSource::Located { item, datum, micrometres } => rows.get(item.as_str()).is_some_and(|row| {
                    row.applies()
                        && row
                            .located
                            .iter()
                            .any(|l| l.datum == *datum && (l.value * 1000.0).round() as i64 == *micrometres)
                }),
                PmiSource::Item { .. } | PmiSource::Thread { .. } => {
                    match annotation_row(annotation, &source, all).filter(|(row, _)| row.applies()) {
                        Some((row, placement)) if shows(&annotation.body, &row.item.kind) => {
                            if thread_notes.get(row.item.id.as_str()).is_some_and(|(kept, _)| *kept != annotation.id) {
                                return false;
                            }
                            update(annotation, row, &style.symbols);
                            annotation.source = Some(source_on(row, placement));
                            true
                        }
                        Some(_) => {
                            redrawn = true;
                            !redraw
                        }
                        None => false,
                    }
                }
            };
            if !keep
                && !redrawn
                && let Some(item) = source.item()
            {
                removed.push(item.to_owned());
            }
            keep
        });
    }
    removed.sort();
    removed.dedup();
    removed
}

/// Copies what the part states about an item into the annotation that shows it.
fn update(annotation: &mut Annotation, row: &PmiRow, symbols: &crate::library::Symbols) {
    match (&mut annotation.body, &row.item.kind) {
        (AnnotationKind::Note { text, .. }, PmiKind::Size { features, through, thread: Some(thread), .. }) => {
            *text = thread_text(
                features.len(),
                thread,
                *through,
                row.depth,
                &symbols.times,
                &symbols.through,
                &symbols.depth,
            );
        }
        (AnnotationKind::Dimension(d), PmiKind::Size { features, tolerance, through, .. }) => {
            d.tolerance = *tolerance;
            d.requirement = true;
            d.text = size_text(features.len(), *through, &symbols.times, &symbols.through);
            if let Some(value) = row.measured.first() {
                d.value = crate::DimValue::Measured { value: *value };
            }
        }
        (AnnotationKind::Dimension(d), PmiKind::Location { tolerance, .. }) => {
            d.tolerance = *tolerance;
            d.requirement = true;
            if let Some(value) = row.measured.first() {
                d.value = crate::DimValue::Measured { value: *value };
            }
        }
        (AnnotationKind::Note { text, .. }, PmiKind::Slot { .. } | PmiKind::Hole { .. }) => *text = row.callout(),
        (
            AnnotationKind::Gdt { characteristic, tolerance, diameter, material, datums },
            PmiKind::Gdt { characteristic: c, tolerance: t, diameter: d, material: m, datums: refs, .. },
        ) => {
            (*characteristic, *tolerance, *diameter, *material) = (*c, *t, *d, *m);
            datums.clone_from(refs);
        }
        (AnnotationKind::Datum { letter }, PmiKind::Datum { letter: stated, .. }) => letter.clone_from(stated),
        (
            AnnotationKind::SurfaceFinish { text, process, method, lay, allowance, all_around },
            PmiKind::SurfaceFinish { text: t, process: p, method: m, lay: l, allowance: a, all_around: all, .. },
        ) => {
            (*process, *lay, *all_around) = (*p, *l, *all);
            text.clone_from(t);
            method.clone_from(m);
            allowance.clone_from(a);
        }
        _ => {}
    }
}

/// Brings the part's manufacturing information into the views of sheet `sheet`, each item
/// once, and moves what it made until nothing lies over anything else.
pub fn import(
    drawing: &mut Drawing,
    sheet: usize,
    part: &Part,
    index: &PartIndex,
    views: &[ViewContext],
) -> Result<PmiReport, DrawingError> {
    let mut report = PmiReport::default();
    let rows = &part.requirements.rows;
    if rows.is_empty() {
        return Ok(report);
    }
    let topology = part.geometry.topology()?;
    let size = topology.faces.iter().fold(1.0_f64, |size, face| size.max(face.bbox.max.distance(face.bbox.min)));
    let template = drawing.sheets.get(sheet).map(|s| s.template.as_str()).unwrap_or("");
    let style = crate::library::template(template).style_file();
    // Items already on the drawing stay where they are and are not drawn twice.
    let present: BTreeSet<&str> = drawing
        .sheets
        .iter()
        .flat_map(|sheet| &sheet.annotations)
        .filter_map(|annotation| row_of(annotation.source.as_ref()?, rows))
        .map(|row| row.item.id.as_str())
        .collect();
    let candidates: Vec<&ViewContext> = views
        .iter()
        .filter(|v| !v.pictorial)
        .filter(|v| drawing.view(v.id).is_some_and(|view| !matches!(view.kind, crate::ViewKind::Section { .. })))
        .collect();
    let axes = part.geometry.face_carriers()?.into_iter().filter_map(|c| Some((c.id, c.axis?))).collect();
    // The way out of the group of views, for each view: away from the middle of the
    // others.
    let places: Vec<(ViewId, Point2)> =
        candidates.iter().filter_map(|v| Some((v.id, drawing.view(v.id)?.position))).collect();
    let away: BTreeMap<ViewId, (i8, i8)> = places
        .iter()
        .map(|(id, at)| {
            let others: Vec<&Point2> = places.iter().filter(|(other, _)| other != id).map(|(_, p)| p).collect();
            let n = others.len().max(1) as f64;
            let (mu, mv) = (others.iter().map(|p| p.u).sum::<f64>() / n, others.iter().map(|p| p.v).sum::<f64>() / n);
            let sign = |d: f64| {
                if others.is_empty() || d.abs() < 1.0 {
                    0
                } else if d > 0.0 {
                    1
                } else {
                    -1
                }
            };
            (*id, (sign(at.u - mu), sign(at.v - mv)))
        })
        .collect();
    let mut planner = Planner {
        body: Body { faces: topology.faces.into_iter().map(|f| (f.id, f)).collect(), axes, index, tol: 1e-6 * size },
        bbox: candidates.iter().filter_map(|v| Some((v.id, drawing.view(v.id)?.geometry.bbox))).collect(),
        views: candidates,
        plans: Vec::new(),
        diagonals_taken: BTreeMap::new(),
        on_edge: BTreeMap::new(),
        sizes: BTreeMap::new(),
        stacked: BTreeSet::new(),
        away,
        datum_sides: BTreeMap::new(),
        pass: Pass::Distances,
        text_height: style.text.height,
        shoulder: style.lines.leader_shoulder,
        times: style.symbols.times.clone(),
        through: style.symbols.through.clone(),
        depth: style.symbols.depth.clone(),
    };
    let mut failed: BTreeMap<String, String> = BTreeMap::new();
    for pass in [Pass::Distances, Pass::Sizes, Pass::Rest] {
        planner.pass = pass;
        for row in rows {
            if present.contains(row.item.id.as_str()) || failed.contains_key(&row.item.id) {
                continue;
            }
            if let Err(reason) = planner.row(row, rows) {
                // Nothing of an item that fails is drawn.
                for plan in &mut planner.plans {
                    if plan.source.item() == Some(row.item.id.as_str()) {
                        plan.dropped = true;
                    }
                }
                failed.insert(row.item.id.clone(), reason);
            }
        }
        if pass == Pass::Distances {
            planner.stack();
        }
    }
    for row in rows {
        match failed.remove(&row.item.id) {
            Some(reason) => report.unplaced.push(Unplaced { name: row.item.id.clone(), reason }),
            None if !present.contains(row.item.id.as_str()) => report.placed.push(row.item.id.clone()),
            None => {}
        }
    }

    let mut next = drawing.next_annotation_id();
    let mut ids: Vec<Option<AnnotationId>> = Vec::new();
    let mut made: Vec<Made> = Vec::new();
    for plan in planner.plans {
        if plan.dropped {
            ids.push(None);
            continue;
        }
        let anchors = plan
            .anchors
            .iter()
            .filter_map(|(id, attach, cached)| {
                Some(Anchor { entity: index.edge_ref(*id)?, attach: *attach, cached: *cached })
            })
            .collect();
        drawing.sheets[sheet].annotations.push(Annotation {
            id: next,
            view: plan.view,
            body: plan.body,
            anchors,
            placement: plan.placement,
            style: StyleOverrides::default(),
            state: AnnotationState::Ok,
            source: Some(plan.source),
        });
        let host = plan.host.and_then(|host| ids.get(host).copied().flatten());
        made.push(Made {
            id: next,
            slide: plan.slide,
            host,
            hosted: plan.host.is_some(),
            stacked: plan.stack.is_some(),
        });
        ids.push(Some(next));
        next = AnnotationId(next.0 + 1);
    }
    settle(drawing, sheet, &made);
    Ok(report)
}

/// An annotation just put on the sheet, for settling.
struct Made {
    id: AnnotationId,
    slide: Slide,
    /// The annotation it sits under and moves with.
    host: Option<AnnotationId>,
    hosted: bool,
    stacked: bool,
}

/// Brings the part's manufacturing information into the views of sheet `sheet`,
/// projecting each view through the kernel first.
pub fn import_part(drawing: &mut Drawing, sheet: usize, part: &Part) -> Result<PmiReport, DrawingError> {
    let index = PartIndex::build(part)?;
    let ids: Vec<_> = drawing.sheets.get(sheet).map(|s| s.views.iter().map(|v| v.id).collect()).unwrap_or_default();
    let contexts =
        ids.into_iter().map(|id| ViewContext::compute(drawing, part, &index, id)).collect::<Result<Vec<_>, _>>()?;
    import(drawing, sheet, part, &index, &contexts)
}

/// A rectangle in sheet millimetres.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Rect {
    fn overlaps(&self, other: &Rect) -> bool {
        self.x0 < other.x1 && other.x0 < self.x1 && self.y0 < other.y1 && other.y0 < self.y1
    }

    fn holds(&self, p: (f64, f64)) -> bool {
        p.0 > self.x0 && p.0 < self.x1 && p.1 > self.y0 && p.1 < self.y1
    }
}

/// What a laid-out sheet has that texts must stay clear of.
struct Laid {
    /// The text boxes of each annotation.
    texts: BTreeMap<AnnotationId, Vec<Rect>>,
    /// Points along the lines of each annotation.
    lines: BTreeMap<AnnotationId, Vec<(f64, f64)>>,
    /// Points along everything else on the sheet: views, the sheet format, tables.
    fixed: Vec<(f64, f64)>,
    /// The drawing frame inside its zone band: left, bottom, right, top.
    frame: [f64; 4],
}

fn points_of(path: &kurbo::BezPath, out: &mut Vec<(f64, f64)>) {
    for line in display::flatten(path, 0.1) {
        for pair in line.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let steps = ((b - a).hypot() / 0.75).ceil().max(1.0) as usize;
            for k in 0..=steps {
                let p = a.lerp(b, k as f64 / steps as f64);
                out.push((p.x, p.y));
            }
        }
    }
}

fn text_box(text: &display::TextItem) -> Rect {
    let width = display::text_width(&text.text, text.height);
    let (x0, x1) = match text.halign {
        crate::library::Align::Left => (0.0, width),
        crate::library::Align::Centre => (-width / 2.0, width / 2.0),
        crate::library::Align::Right => (-width, 0.0),
    };
    let (y0, y1) = match text.valign {
        display::VAlign::Bottom => (0.0, text.height),
        display::VAlign::Centre => (-text.height / 2.0, text.height / 2.0),
        display::VAlign::Top => (-text.height, 0.0),
    };
    let mut path = kurbo::Rect::new(x0 - CLEAR, y0 - CLEAR, x1 + CLEAR, y1 + CLEAR).to_path(0.1);
    path.apply_affine(
        kurbo::Affine::translate((text.pos.u, text.pos.v)) * kurbo::Affine::rotate(text.angle.to_radians()),
    );
    let b = path.bounding_box();
    Rect { x0: b.x0, y0: b.y0, x1: b.x1, y1: b.y1 }
}

fn lay(drawing: &Drawing, sheet: usize) -> Laid {
    let list = display::layout_sheet(drawing, sheet);
    let frame = drawing.sheets.get(sheet).map_or([0.0, 0.0, list.paper.0, list.paper.1], |page| {
        let template = crate::library::template(page.template.as_str());
        let ([l, b, r, t], band) = (template.frame(), template.zones.band);
        [l + band, b + band, r - band, t - band]
    });
    let mut laid = Laid { texts: BTreeMap::new(), lines: BTreeMap::new(), fixed: Vec::new(), frame };
    for item in list.printable() {
        match (item.owner(), item) {
            (Owner::Annotation(id) | Owner::AnnotationEnd(id, _), Item::Text(text)) => {
                laid.texts.entry(id).or_default().push(text_box(text));
            }
            (Owner::Annotation(id) | Owner::AnnotationEnd(id, _), Item::Path(path)) => {
                points_of(&path.path, laid.lines.entry(id).or_default());
            }
            (_, Item::Path(path)) => points_of(&path.path, &mut laid.fixed),
            (_, Item::Text(text)) => {
                // A text of the sheet or a view stands for its corners and its middle.
                let b = text_box(text);
                let (mx, my) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
                laid.fixed.extend([
                    (b.x0, b.y0),
                    (b.x1, b.y0),
                    (b.x0, b.y1),
                    (b.x1, b.y1),
                    (mx, my),
                    (mx, b.y0),
                    (mx, b.y1),
                ]);
            }
        }
    }
    // A tolerance frame and a datum's box are as much in the way as a text.
    if let Some(page) = drawing.sheets.get(sheet) {
        let th = crate::library::template(page.template.as_str()).style_file().text.height;
        for a in &page.annotations {
            let origin = page.view(a.view).map_or(Point2::default(), |v| v.position);
            let (u, v) = (a.placement.u + origin.u, a.placement.v + origin.v);
            let frame = match &a.body {
                AnnotationKind::Gdt { .. } => Rect { x0: u, y0: v - th, x1: u + gdt_width(&a.body, th), y1: v + th },
                AnnotationKind::Datum { letter } => {
                    let half = (display::text_width(letter, th) + th).max(2.0 * th) / 2.0;
                    Rect { x0: u - half, y0: v - th, x1: u + half, y1: v + th }
                }
                _ => continue,
            };
            laid.texts.entry(a.id).or_default().push(frame);
        }
    }
    laid
}

impl Laid {
    /// True when a text of `group` reaches beyond the drawing frame.
    fn off_frame(&self, group: &[AnnotationId]) -> bool {
        let [l, b, r, t] = self.frame;
        group.iter().filter_map(|id| self.texts.get(id)).flatten().any(|x| x.x0 < l || x.y0 < b || x.x1 > r || x.y1 > t)
    }

    /// What the texts of `group` lie over, or that lies over them: another annotation
    /// among `others`, or something fixed on the sheet.
    fn clash(&self, group: &[AnnotationId], others: &[AnnotationId]) -> Option<Option<AnnotationId>> {
        let boxes: Vec<&Rect> = group.iter().filter_map(|id| self.texts.get(id)).flatten().collect();
        if self.off_frame(group) {
            return Some(None);
        }
        for b in &boxes {
            if self.fixed.iter().any(|p| b.holds(*p)) {
                return Some(None);
            }
        }
        for other in others {
            if group.contains(other) {
                continue;
            }
            let theirs = self.texts.get(other).map_or(&[][..], Vec::as_slice);
            if boxes.iter().any(|b| theirs.iter().any(|t| b.overlaps(t))) {
                return Some(Some(*other));
            }
            // Their lines through these texts, and these lines through their texts.
            let their_lines = self.lines.get(other).map_or(&[][..], Vec::as_slice);
            if boxes.iter().any(|b| their_lines.iter().any(|p| b.holds(*p))) {
                return Some(Some(*other));
            }
            let own_lines = group.iter().filter_map(|id| self.lines.get(id)).flatten();
            if own_lines.into_iter().any(|p| theirs.iter().any(|t| t.holds(*p))) {
                return Some(Some(*other));
            }
        }
        None
    }
}

/// Moves each new annotation, with what sits under it, along its own direction until
/// its texts are clear of the views, the sheet and the annotations settled before it.
/// The stacked dimensions settle first, nearest their views; the callouts and symbols
/// then clear them. A datum with no clear place along its direction tries across it.
/// An annotation that finds no clear place on the paper stays where it was
/// planned, and [`overlaps`] names it.
fn settle(drawing: &mut Drawing, sheet: usize, made: &[Made]) {
    let new: BTreeSet<AnnotationId> = made.iter().map(|m| m.id).collect();
    let mut settled: Vec<AnnotationId> =
        drawing.sheets[sheet].annotations.iter().map(|a| a.id).filter(|id| !new.contains(id)).collect();
    let order = made.iter().filter(|m| m.stacked).chain(made.iter().filter(|m| !m.stacked));
    for lead in order.filter(|m| !m.hosted) {
        let group: Vec<AnnotationId> =
            std::iter::once(lead.id).chain(made.iter().filter(|m| m.host == Some(lead.id)).map(|m| m.id)).collect();
        let planned = places(drawing, &group);
        // A stacked dimension moves out along its stack. A callout or a symbol moves
        // along its leader or, when that leads nowhere, straight across or straight up;
        // the way that needs the fewest steps wins.
        let along = lead.slide.along;
        let mut ways = vec![along];
        if !lead.stacked && along.u.abs() > 1e-9 && along.v.abs() > 1e-9 {
            ways.push(Point2::new(along.u.signum(), 0.0));
            ways.push(Point2::new(0.0, along.v.signum()));
        }
        let mut best: Option<(usize, Vec<Option<Point2>>)> = None;
        let slide = |ways: &[Point2], best: &mut Option<(usize, Vec<Option<Point2>>)>, drawing: &mut Drawing| {
            for way in ways {
                put(drawing, &group, &planned);
                for moves in 0..MOVES {
                    if best.as_ref().is_some_and(|(fewest, _)| moves >= *fewest) {
                        break;
                    }
                    let laid = lay(drawing, sheet);
                    if laid.clash(&group, &settled).is_none() {
                        *best = Some((moves, places(drawing, &group)));
                        break;
                    }
                    if laid.off_frame(&group) && moves > 0 {
                        break;
                    }
                    for member in &group {
                        if let Some(a) = drawing.annotation_mut(*member) {
                            a.placement = Point2::new(
                                round(a.placement.u + way.u * lead.slide.step),
                                round(a.placement.v + way.v * lead.slide.step),
                            );
                        }
                    }
                }
            }
        };
        slide(&ways, &mut best, drawing);
        if best.is_none() && drawing.annotation(lead.id).is_some_and(|a| matches!(a.body, AnnotationKind::Datum { .. }))
        {
            slide(&[Point2::new(-along.v, along.u), Point2::new(along.v, -along.u)], &mut best, drawing);
        }
        put(drawing, &group, best.as_ref().map_or(&planned, |(_, at)| at));
        settled.extend(group);
    }
}

fn places(drawing: &Drawing, group: &[AnnotationId]) -> Vec<Option<Point2>> {
    group.iter().map(|id| drawing.annotation(*id).map(|a| a.placement)).collect()
}

fn put(drawing: &mut Drawing, group: &[AnnotationId], at: &[Option<Point2>]) {
    for (member, at) in group.iter().zip(at) {
        if let (Some(a), Some(at)) = (drawing.annotation_mut(*member), at) {
            a.placement = *at;
        }
    }
}

/// The names of the annotations on sheet `sheet` whose texts lie over another
/// annotation, a view or the sheet's own lines and texts, with what they lie over.
pub fn overlaps(drawing: &Drawing, sheet: usize) -> Vec<(String, String)> {
    let Some(page) = drawing.sheets.get(sheet) else { return Vec::new() };
    let style = crate::library::template(page.template.as_str()).style_file();
    let laid = lay(drawing, sheet);
    let ids: Vec<AnnotationId> = page.annotations.iter().map(|a| a.id).collect();
    let name = |id: AnnotationId| {
        drawing.annotation(id).map_or_else(|| format!("Annotation {}", id.0), |a| display::annotation_name(a, style))
    };
    let mut out = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        // Each pair once; the sheet and the views for every annotation.
        match laid.clash(&[*id], &ids[..i]) {
            Some(Some(other)) => out.push((name(*id), name(other))),
            Some(None) => out.push((name(*id), "the sheet or a view".to_owned())),
            None => {}
        }
    }
    out
}

/// How many annotations show each item of the part, by the item's name. The basic
/// distances of a position tolerance count under their own names, such as
/// `Hole position: 12 from B`.
pub fn shown(drawing: &Drawing) -> BTreeMap<String, usize> {
    let mut count = BTreeMap::new();
    for annotation in drawing.sheets.iter().flat_map(|s| &s.annotations) {
        let key = match &annotation.source {
            Some(PmiSource::Item { item } | PmiSource::Thread { item, .. }) => item.clone(),
            Some(PmiSource::Located { item, datum, micrometres }) => {
                format!(
                    "{item}: {} from {datum}",
                    soliddraft_kernel_api::pmi::length_text(*micrometres as f64 / 1000.0)
                )
            }
            Some(PmiSource::Notes) | None => continue,
        };
        *count.entry(key).or_insert(0) += 1;
    }
    count
}

/// The annotations that state a requirement the part does not: a tolerance, a datum, a
/// tolerance frame or a surface finish with no item of the part behind it, or one whose
/// item is not among `items`, by name as the drawing lists them.
pub fn not_from_part(drawing: &Drawing, items: &BTreeSet<String>) -> Vec<String> {
    let mut out = Vec::new();
    for sheet in &drawing.sheets {
        let style = crate::library::template(sheet.template.as_str()).style_file();
        for annotation in &sheet.annotations {
            let states = match &annotation.body {
                AnnotationKind::Dimension(d) => !d.tolerance.is_none() || d.requirement,
                AnnotationKind::Gdt { .. } | AnnotationKind::Datum { .. } | AnnotationKind::SurfaceFinish { .. } => {
                    true
                }
                _ => annotation.source.is_some(),
            };
            let backed = match &annotation.source {
                Some(PmiSource::Notes) => true,
                Some(source) => source.item().is_some_and(|item| items.contains(item)),
                None => false,
            };
            if states && !backed {
                out.push(display::annotation_name(annotation, style));
            }
        }
    }
    out
}
