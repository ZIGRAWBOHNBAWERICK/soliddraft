//! A part's manufacturing information: datums, dimensions with limits, geometric
//! tolerances and surface textures on faces, and the notes of the whole part.
//!
//! Items name their faces persistently (see [`reference`](crate::reference)), so an item
//! follows its faces through edits: a bore edited from 8 to 10 mm keeps its fit and its
//! position tolerance. A dimension stores no size. Its value is measured on the body
//! each time it is asked for, so it is always the size the part has.
//!
//! A feature under a position tolerance is located from the datums of that tolerance:
//! a basic distance from each datum plane its axis runs along. Nobody states those
//! distances. They are measured ([`Located`]), written into a STEP file with the
//! tolerance, and left out again when such a file is read, since the part measures
//! its own.
//!
//! An item whose face is gone, or whose faces no longer fit it (a diameter on a face
//! that became a plane), is not moved to another face and not dropped. It stays in the
//! part with its problem, and applies again when the face comes back.
//!
//! A threaded hole states its own thread: each hole feature with a thread adds an item,
//! the diameter of its walls with the thread's designation ([`PmiRow::feature`]). The
//! part does not store that item. It follows the hole: edit the thread and the item
//! changes, take it out and the item goes. It states what is left of the thread on the
//! body ([`ThreadedWall`](crate::thread::ThreadedWall)): the walls that still have
//! thread, and how deep it goes from where the hole starts now. It reaches a STEP file
//! and a drawing like any other.

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::drawing::Curve3;
use soliddraft_kernel_api::pmi::{
    Anchor, At, Feature, GdtCharacteristic, HoleHeadKind, HoleHeadNominals, HoleNominals, PartNotes, Pmi, PmiItem,
    PmiKind, StepPmi, Tolerance, length_text,
};
use soliddraft_kernel_api::{
    BodyId, EdgeId, FaceAxis, FaceId, FaceRole, HolePart, Kernel, KernelError, SurfaceKind, Topology,
};

use crate::document::Document;
use crate::feature::{FeatureId, FeatureKind};
use crate::reference::{self, FaceRef, View};

/// A part's manufacturing information, on persistent face names.
pub type PartPmi = Pmi<FaceRef>;

/// One item as it stands on the part's body.
#[derive(Clone, Debug, PartialEq)]
pub struct PmiRow {
    /// The item as the part stores it.
    pub item: PmiItem<FaceRef>,
    /// The item on the body's faces, when every face it names is there.
    pub on_body: Option<PmiItem<FaceId>>,
    /// The measured values in millimetres: one for a size or a distance; the length,
    /// the width and the end radius for a slot.
    pub measured: Vec<f64>,
    /// Nominals for a typed hole, including values whose units are not millimetres.
    pub hole: Option<HoleNominals>,
    /// For a position tolerance, the basic distances that locate its features from
    /// its datums.
    pub located: Vec<Located>,
    /// Why the item does not apply as it stands, for people.
    pub problem: Option<String>,
    /// The threaded hole that states the item, which the part does not store: change
    /// it through that hole.
    pub feature: Option<FeatureId>,
    /// For an item a threaded hole states, which of the hole's holes it covers, from
    /// zero. Empty for an item the part stores.
    pub placements: Vec<usize>,
    /// How deep a thread goes from the face its hole starts on, for a thread that does
    /// not go through.
    pub depth: Option<f64>,
    /// The body the item's faces are on, when they are on one.
    pub body: Option<BodyId>,
}

impl PmiRow {
    /// The item as a drawing writes it, such as `Ø8 H7` or `M6x1-6H DEEP 12`.
    pub fn callout(&self) -> String {
        if let PmiKind::Hole { .. } = &self.item.kind {
            let diameter = self.measured.first().map_or_else(|| "?".to_owned(), |value| length_text(*value));
            return self.item.hole_callout(&diameter, self.hole.as_ref());
        }
        let callout = self.item.callout(&self.measured);
        match self.depth {
            Some(depth) => format!("{callout} DEEP {}", soliddraft_kernel_api::pmi::length_text(depth)),
            None => callout,
        }
    }

    /// True when the item applies: its faces are there and, for a dimension, measure.
    pub fn applies(&self) -> bool {
        self.problem.is_none()
    }
}

/// An item a threaded hole states.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadItem {
    /// The item: the diameter of the threaded walls, with the thread's designation.
    pub item: PmiItem<FaceRef>,
    /// The hole that states it.
    pub feature: FeatureId,
    /// Which of the hole's holes it covers, from zero, in order.
    pub placements: Vec<usize>,
    /// How deep the thread goes from the face the hole starts on now, for a thread that
    /// does not run out through the hole.
    pub depth: Option<f64>,
    /// The body the walls are on.
    pub body: Option<BodyId>,
    /// Why the item does not apply, when a later feature took the hole's thread away.
    pub problem: Option<String>,
}

/// Holes of one feature whose threads read the same.
struct ThreadGroup {
    through: bool,
    depth: Option<f64>,
    body: Option<BodyId>,
    placements: Vec<usize>,
}

/// A basic distance that locates one feature of a position tolerance from one of its
/// datums.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Located {
    /// The datum it is measured from.
    pub datum: String,
    /// Which of the tolerance's features it locates, from zero.
    pub feature: usize,
    /// The distance in millimetres.
    pub value: f64,
}

impl Located {
    /// The name a file gives the distance, after its tolerance's: `Hole position/2
    /// from B`.
    pub fn name(&self, tolerance: &str) -> String {
        locating_name(tolerance, self.feature, &self.datum)
    }
}

fn locating_name(tolerance: &str, feature: usize, datum: &str) -> String {
    format!("{tolerance}/{} from {datum}", feature + 1)
}

fn locating<F: Clone>(datum: &Feature<F>, feature: &Feature<F>) -> PmiKind<F> {
    PmiKind::Location {
        from: Anchor { feature: datum.clone(), at: At::Feature },
        to: Anchor { feature: feature.clone(), at: At::Feature },
        tolerance: Tolerance::Basic,
        nominal: None,
    }
}

fn same_faces<F: PartialEq>(a: &[F], b: &[F]) -> bool {
    a.len() == b.len() && a.iter().all(|face| b.contains(face))
}

/// The part's manufacturing information as a STEP file takes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PmiOnBody {
    /// The items that apply, on the body's faces, with the part's notes, followed by
    /// the locating distances of the position tolerances among them.
    pub pmi: Pmi<FaceId>,
    /// How many of the part's items `pmi` holds, without the locating distances.
    pub items: usize,
    /// The items left out, by name, with the reason.
    pub left_out: Vec<(String, String)>,
}

/// What reading a STEP file's manufacturing information into the part did.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PmiImport {
    /// Items added to the part, by name.
    pub added: Vec<String>,
    /// Items of the file that were not added, with the reason.
    pub left_out: Vec<(String, String)>,
}

/// A kernel refusal about one item, without the item's name in front.
fn reason(error: KernelError, id: &str) -> String {
    let text = match error {
        KernelError::InvalidInput(detail) => detail,
        KernelError::Unsupported(_) => "Coming soon".to_owned(),
        other => other.to_string(),
    };
    text.strip_prefix(&format!("{id}: ")).map(str::to_owned).unwrap_or(text)
}

// Allow for conversion of exact carriers to doubles, not for an approximation of the part.
const HOLE_EPS: f64 = 1e-7;

struct HoleRim {
    z: f64,
    radius: f64,
    edges: Vec<EdgeId>,
    neighbours: Vec<FaceId>,
}

fn hole_rims(
    kernel: &dyn Kernel,
    body: BodyId,
    topology: &Topology,
    faces: &[FaceId],
    axis: FaceAxis,
) -> Result<[HoleRim; 2], String> {
    let mut rims: Vec<HoleRim> = Vec::with_capacity(2);
    for &id in faces {
        let face = topology.face(id).ok_or("A hole wall is missing.")?;
        for edge in face.edges.iter().filter_map(|edge| topology.edge(*edge)) {
            if edge.faces.iter().all(|face| faces.contains(face)) {
                continue; // Seam or boundary between pieces of the same wall.
            }
            let curve = kernel.edge_drawing(body, edge.id).map_err(|error| error.to_string())?;
            let (center, normal, radius) = match curve.curve {
                Curve3::Circle { center, normal, radius } | Curve3::Arc { center, normal, radius, .. }
                    if !curve.approximate =>
                {
                    (center, normal, radius)
                }
                _ => continue,
            };
            let offset = center - axis.point;
            if offset.cross(axis.direction).length() > HOLE_EPS || normal.cross(axis.direction).length() > HOLE_EPS {
                continue;
            }
            let z = offset.dot(axis.direction);
            let rim = match rims.iter_mut().find(|rim| (rim.z - z).abs() <= HOLE_EPS) {
                Some(rim) if (rim.radius - radius).abs() <= HOLE_EPS => rim,
                Some(_) => return Err("The hole has different radii at the same rim.".into()),
                None => {
                    if rims.len() == 2 {
                        return Err("The hole wall has more than two circular rims.".into());
                    }
                    rims.push(HoleRim { z, radius, edges: Vec::new(), neighbours: Vec::new() });
                    rims.last_mut().unwrap()
                }
            };
            if !rim.edges.contains(&edge.id) {
                rim.edges.push(edge.id);
                for neighbour in edge.faces.iter().filter(|face| !faces.contains(face)) {
                    if !rim.neighbours.contains(neighbour) {
                        rim.neighbours.push(*neighbour);
                    }
                }
            }
        }
    }
    rims.try_into().map_err(|_| "The hole wall needs two checked circular rims.".to_owned())
}

fn measure_hole(
    kernel: &dyn Kernel,
    body: BodyId,
    topology: &Topology,
    item: &PmiItem<FaceId>,
) -> Result<HoleNominals, String> {
    let PmiKind::Hole { pilot, head, through, thread, .. } = &item.kind else {
        unreachable!();
    };
    let carriers = kernel.face_carriers(body).map_err(|error| error.to_string())?;
    let carrier = |face| carriers.iter().find(|candidate| candidate.id == face);
    let axis = |face| carrier(face).and_then(|face| face.axis);
    let pilot_axis = pilot.first().copied().and_then(axis).ok_or("Pick a cylindrical pilot wall.")?;
    for &face in pilot {
        let other = axis(face).ok_or("Every pilot face must be cylindrical.")?;
        if (other.radius - pilot_axis.radius).abs() > HOLE_EPS
            || (other.point - pilot_axis.point).cross(pilot_axis.direction).length() > HOLE_EPS
            || other.direction.cross(pilot_axis.direction).length() > HOLE_EPS
        {
            return Err("The pilot faces are not on the same cylinder.".into());
        }
    }
    let pilot_rims = hole_rims(kernel, body, topology, pilot, pilot_axis)?;
    let planar = |face| {
        carrier(face)
            .and_then(|face| face.normal)
            .is_some_and(|normal| normal.cross(pilot_axis.direction).length() <= HOLE_EPS)
    };
    let closed = |rim: &HoleRim| {
        rim.neighbours.iter().filter_map(|id| topology.face(*id)).any(|face| {
            (planar(face.id) && face.edges.iter().all(|edge| rim.edges.contains(edge)))
                || face.origins.iter().any(|origin| matches!(origin.role, FaceRole::Hole { part: HolePart::Tip, .. }))
        })
    };
    let opening = |rim: &HoleRim| {
        !closed(rim)
            && rim.neighbours.iter().copied().any(|id| {
                planar(id)
                    && carrier(id).is_some_and(|face| {
                        !face.origins.iter().any(|origin| {
                            matches!(origin.role, FaceRole::Hole { part: HolePart::CounterboreFloor, .. })
                        })
                    })
            })
    };
    let (entry_z, end, head_nominals) = if let Some(head) = head {
        let surface = match head.kind {
            HoleHeadKind::Counterbore => SurfaceKind::Cylinder,
            HoleHeadKind::Countersink => SurfaceKind::Cone,
        };
        if !head.faces.iter().all(|face| carrier(*face).is_some_and(|face| face.surface == surface)) {
            return Err("The head faces do not match the chosen head kind.".into());
        }
        let head_rims = hole_rims(kernel, body, topology, &head.faces, pilot_axis)?;
        let mut matched = None;
        for (i, inner) in head_rims.iter().enumerate() {
            let outer = &head_rims[1 - i];
            if !opening(outer) || outer.radius <= pilot_axis.radius + HOLE_EPS {
                continue;
            }
            for (j, start) in pilot_rims.iter().enumerate() {
                let end = &pilot_rims[1 - j];
                if (start.z - inner.z).abs() > HOLE_EPS || (outer.z - inner.z) * (end.z - start.z) >= 0.0 {
                    continue;
                }
                let connected = match head.kind {
                    HoleHeadKind::Counterbore => {
                        (inner.radius - outer.radius).abs() <= HOLE_EPS
                            && inner.neighbours.iter().any(|id| start.neighbours.contains(id) && planar(*id))
                    }
                    HoleHeadKind::Countersink => {
                        (inner.radius - pilot_axis.radius).abs() <= HOLE_EPS
                            && inner.edges.iter().any(|id| start.edges.contains(id))
                    }
                };
                if connected && matched.replace((inner, outer, end)).is_some() {
                    return Err("The head and pilot do not have a unique junction.".into());
                }
            }
        }
        let (inner, outer, end) = matched.ok_or("The head and pilot must meet on the same axis.")?;
        let depth_mm = (outer.z - inner.z).abs();
        let diameter_mm = outer.radius * 2.0;
        let values = match head.kind {
            HoleHeadKind::Counterbore => HoleHeadNominals::Counterbore { diameter_mm, depth_mm },
            HoleHeadKind::Countersink => HoleHeadNominals::Countersink {
                diameter_mm,
                included_angle_deg: 2.0 * (outer.radius - inner.radius).atan2(depth_mm).to_degrees(),
            },
        };
        (outer.z, end, Some(values))
    } else if *through && pilot_rims.iter().all(opening) {
        (pilot_rims[0].z, &pilot_rims[1], None)
    } else if !*through && opening(&pilot_rims[0]) && closed(&pilot_rims[1]) {
        (pilot_rims[0].z, &pilot_rims[1], None)
    } else if !*through && opening(&pilot_rims[1]) && closed(&pilot_rims[0]) {
        (pilot_rims[1].z, &pilot_rims[0], None)
    } else {
        return Err("The pilot does not have the selected through or blind extent.".into());
    };
    if (*through && !opening(end)) || (!*through && !closed(end)) {
        return Err("The pilot does not have the selected through or blind extent.".into());
    }
    let depth_mm = (end.z - entry_z).abs();
    if thread.as_ref().and_then(|thread| thread.depth.as_ref()).is_some_and(|depth| depth.value() > depth_mm + HOLE_EPS)
    {
        return Err("Usable thread depth exceeds the available drill depth.".into());
    }
    Ok(HoleNominals {
        pilot_diameter_mm: pilot_axis.radius * 2.0,
        head: head_nominals,
        drill_depth_mm: (!through).then_some(depth_mm),
    })
}

impl Document {
    /// The part's manufacturing information.
    pub fn pmi(&self) -> &PartPmi {
        &self.pmi
    }

    /// The persistent names of body faces, as one feature of an item.
    pub fn pmi_feature(&self, faces: &[FaceId]) -> Result<Feature<FaceRef>, crate::NamingError> {
        faces.iter().map(|face| self.face_ref(*face)).collect()
    }

    /// The items the part's threaded holes state: the diameter of each hole feature's
    /// threaded walls with its thread's designation. An item covers the holes whose
    /// callout reads the same, so a later cut that leaves one hole of a pattern less
    /// thread than the others gives that hole an item of its own. A hole whose thread a
    /// later feature took away entirely states one item with the reason. A suppressed or
    /// failed hole states none.
    pub fn thread_items(&self) -> Vec<ThreadItem> {
        let walls = self.threaded_walls();
        let mut taken = self.pmi.clone();
        let mut out = Vec::new();
        for feature in self.features() {
            let FeatureKind::Hole(hole) = &feature.kind else { continue };
            let Some(thread) = &hole.thread else { continue };
            if self.status(feature.id) != Some(&crate::FeatureStatus::Ok) {
                continue;
            }
            let mut groups: Vec<ThreadGroup> = Vec::new();
            for wall in walls.iter().filter(|wall| wall.feature == feature.id) {
                let (through, depth, body) = (wall.through, wall.depth(), Some(wall.body));
                match groups.iter_mut().find(|g| g.through == through && g.depth == depth && g.body == body) {
                    Some(group) => group.placements.push(wall.placement),
                    None => groups.push(ThreadGroup { through, depth, body, placements: vec![wall.placement] }),
                }
            }
            let problem = groups.is_empty().then(|| {
                format!("No wall of {} is threaded any more: a later feature took the thread away.", feature.name)
            });
            if groups.is_empty() {
                // Nothing is left to measure: the item says what the hole states. A Hole
                // Wizard hole states its depth through its end condition, and its points
                // through its position sketch.
                let original = self.original_hole_extents(feature.id);
                let count = self.hole_points(hole).map(|pts| pts.len()).unwrap_or(hole.points.len());
                for placement in 0..count {
                    let effective_depth = original
                        .get(placement)
                        .copied()
                        .flatten()
                        .map(|extent| extent.depth)
                        .or_else(|| hole.specified_depth());
                    let through = matches!(effective_depth, Some(soliddraft_kernel_api::HoleDepth::Through))
                        && thread.length.is_none();
                    let depth = thread.length.or(match effective_depth {
                        Some(soliddraft_kernel_api::HoleDepth::Blind { depth }) => Some(depth),
                        _ => None,
                    });
                    match groups.iter_mut().find(|group| group.through == through && group.depth == depth) {
                        Some(group) => group.placements.push(placement),
                        None => groups.push(ThreadGroup { through, depth, body: None, placements: vec![placement] }),
                    }
                }
            }
            let name = format!("{} thread", feature.name.replace('/', " "));
            for group in groups {
                let item = PmiItem {
                    id: taken.free_name(&name),
                    kind: PmiKind::Size {
                        size: soliddraft_kernel_api::pmi::SizeKind::Diameter,
                        features: group.placements.iter().map(|i| vec![self.hole_wall(feature, *i)]).collect(),
                        tolerance: Tolerance::None,
                        through: group.through,
                        thread: Some(thread.designation()),
                        nominal: None,
                    },
                };
                taken.items.push(item.clone());
                out.push(ThreadItem {
                    item,
                    feature: feature.id,
                    placements: group.placements,
                    depth: group.depth,
                    body: group.body,
                    problem: problem.clone(),
                });
            }
        }
        out
    }

    /// Every item with the body faces it stands on, a dimension's measured value, and
    /// what is wrong with it if it does not apply. The items the part stores come first,
    /// then those its threaded holes state.
    pub fn pmi_rows(&self) -> Vec<PmiRow> {
        // The bodies an item can be on: the part's one body, or each of several.
        let bodies: Vec<(BodyId, Result<Topology, String>)> = match self.body() {
            Some(body) => vec![(body, self.view_of(body).map_err(|e| e.to_string()))],
            None => self
                .live_bodies()
                .into_iter()
                .map(|(_, body)| (body, self.view_of(body).map_err(|e| e.to_string())))
                .collect(),
        };
        let names = self.name_map();
        let stated = self.thread_items();
        let mut rows: Vec<PmiRow> = Vec::with_capacity(self.pmi.items.len() + stated.len());
        let items = self
            .pmi
            .items
            .iter()
            .map(|item| (item, None))
            .chain(stated.iter().map(|stated| (&stated.item, Some(stated))));
        for (item, stated) in items {
            let mut row = PmiRow {
                item: item.clone(),
                on_body: None,
                measured: Vec::new(),
                hole: None,
                located: Vec::new(),
                problem: None,
                feature: stated.map(|s| s.feature),
                placements: stated.map_or_else(Vec::new, |s| s.placements.clone()),
                depth: stated.and_then(|s| s.depth),
                body: None,
            };
            if let Some(problem) = stated.and_then(|s| s.problem.clone()) {
                row.problem = Some(problem);
                rows.push(row);
                continue;
            }
            // A thread's walls name their body. Any other item is on the part's one body,
            // or on the first of several bodies that has every face it names.
            let wanted = stated.and_then(|s| s.body);
            let mut problem = None;
            for (body, topology) in bodies.iter().filter(|(body, _)| wanted.is_none_or(|wanted| wanted == *body)) {
                let topology = match topology {
                    Ok(topology) => topology,
                    Err(error) => {
                        problem.get_or_insert_with(|| format!("The part's faces could not be read: {error}"));
                        continue;
                    }
                };
                match self.item_on(item, topology, &names) {
                    Err(message) => {
                        problem.get_or_insert(message);
                    }
                    Ok(on_body) => {
                        problem = None;
                        if matches!(on_body.kind, PmiKind::Hole { .. }) {
                            match measure_hole(self.kernel.as_ref(), *body, topology, &on_body) {
                                Ok(values) => {
                                    row.measured.push(values.pilot_diameter_mm);
                                    row.hole = Some(values);
                                }
                                Err(error) => problem = Some(error),
                            }
                        } else if on_body.is_dimension() {
                            let alone = Pmi { items: vec![on_body.clone()], part: PartNotes::default() };
                            match self.kernel.pmi_nominals(*body, &alone) {
                                Ok(values) => row.measured = values.into_iter().next().unwrap_or_default(),
                                Err(error) => problem = Some(reason(error, &item.id)),
                            }
                        }
                        row.on_body = Some(on_body);
                        row.body = Some(*body);
                        break;
                    }
                }
            }
            row.problem = match (&row.on_body, problem) {
                (_, Some(problem)) => Some(problem),
                (None, None) => Some("The part has no body.".to_owned()),
                (Some(_), None) => None,
            };
            rows.push(row);
        }
        // A tolerance leans on its datums: one whose datum does not apply does not either.
        let standing: std::collections::BTreeMap<String, Feature<FaceId>> = rows
            .iter()
            .filter(|row| row.applies())
            .filter_map(|row| match row.on_body.as_ref().map(|item| &item.kind) {
                Some(PmiKind::Datum { letter, feature }) => Some((letter.clone(), feature.clone())),
                _ => None,
            })
            .collect();
        for row in &mut rows {
            if let PmiKind::Gdt { datums, .. } = &row.item.kind
                && row.problem.is_none()
                && let Some(missing) =
                    datums.iter().flat_map(|datum| datum.letters()).find(|letter| !standing.contains_key(*letter))
            {
                row.problem = Some(format!("Datum {missing} does not apply."));
            }
        }
        // A feature under a position tolerance is located from each datum plane of the
        // tolerance that its axis runs along. The kernel decides which: a distance it
        // cannot measure, such as one to a plane the axis crosses, is not one.
        for row in &mut rows {
            let Some(body) = row.body else { continue };
            let Some(PmiItem {
                kind: PmiKind::Gdt { characteristic: GdtCharacteristic::Position, datums, features, .. },
                ..
            }) = row.on_body.as_ref().filter(|_| row.problem.is_none())
            else {
                continue;
            };
            for (index, feature) in features.iter().enumerate() {
                for datum in datums {
                    let Some(from) = standing.get(datum.letter.as_str()) else { continue };
                    let alone = Pmi {
                        items: vec![PmiItem { id: row.item.id.clone(), kind: locating(from, feature) }],
                        part: PartNotes::default(),
                    };
                    let value = self
                        .kernel
                        .pmi_nominals(body, &alone)
                        .ok()
                        .and_then(|values| values.first().and_then(|measured| measured.first().copied()));
                    if let Some(value) = value.filter(|value| value.abs() > 1e-9) {
                        row.located.push(Located { datum: datum.letter.clone(), feature: index, value });
                    }
                }
            }
        }
        rows
    }

    /// An item on the faces of one body, or why it is not on that body.
    fn item_on(
        &self,
        item: &PmiItem<FaceRef>,
        topology: &Topology,
        names: &reference::NameMap,
    ) -> Result<PmiItem<FaceId>, String> {
        let view = View { topology };
        item.try_map_features(|feature| {
            let mut faces: Feature<FaceId> = Vec::new();
            for face in feature {
                let found = reference::resolve_faces(face, &view, names).map_err(|unresolved| {
                    if face.label.is_empty() {
                        unresolved.message
                    } else {
                        format!("{}: {}", face.label, unresolved.message)
                    }
                })?;
                for id in found {
                    if !faces.contains(&id) {
                        faces.push(id);
                    }
                }
            }
            Ok::<_, String>(faces)
        })
    }

    /// The part's manufacturing information for a STEP file: the items that apply, on
    /// the body's faces, and the names of the ones left out. The material note is the
    /// part's own, or else the name of the material assigned to the part.
    pub fn pmi_on_body(&self) -> PmiOnBody {
        self.pmi_on(self.body())
    }

    /// The part's manufacturing information for a STEP file of `body`: an item on another
    /// body of the part is left out.
    pub(crate) fn pmi_on(&self, body: Option<BodyId>) -> PmiOnBody {
        let mut out = PmiOnBody::default();
        let mut locating_items = Vec::new();
        for row in self.pmi_rows() {
            let elsewhere = (row.body.is_some() && row.body != body).then(|| "It is on another body.".to_owned());
            let problem = row.problem.clone().or(elsewhere).or_else(|| row.item.file_problem());
            match (row.on_body, problem) {
                (Some(item), None) => {
                    if let PmiKind::Gdt { datums, features, .. } = &item.kind {
                        for located in &row.located {
                            let from = datums
                                .iter()
                                .find(|datum| datum.letter == located.datum)
                                .and_then(|datum| self.pmi.datum_feature(&datum.letter))
                                .and_then(|feature| self.pmi_feature_on_body(feature));
                            if let (Some(from), Some(to)) = (from, features.get(located.feature)) {
                                locating_items.push(PmiItem { id: located.name(&item.id), kind: locating(&from, to) });
                            }
                        }
                    }
                    out.pmi.items.push(item);
                }
                (_, problem) => {
                    out.left_out.push((row.item.id, problem.unwrap_or_else(|| "It does not apply.".to_owned())));
                }
            }
        }
        out.items = out.pmi.items.len();
        out.pmi.items.extend(locating_items);
        out.pmi.part = self.part_notes();
        out
    }

    /// The faces of the body a feature of the part's information names now.
    fn pmi_feature_on_body(&self, feature: &Feature<FaceRef>) -> Option<Feature<FaceId>> {
        let body = self.body()?;
        let topology = self.view_of(body).ok()?;
        let view = View { topology: &topology };
        let names = self.name_map();
        let mut faces: Feature<FaceId> = Vec::new();
        for face in feature {
            for id in reference::resolve_faces(face, &view, &names).ok()? {
                if !faces.contains(&id) {
                    faces.push(id);
                }
            }
        }
        Some(faces)
    }

    /// The part's notes, with the assigned material's name where the part gives no
    /// material note of its own.
    pub fn part_notes(&self) -> PartNotes {
        let mut notes = self.pmi.part.clone();
        if notes.material.is_none() {
            notes.material = self.material(None).map(|material| material.name.clone());
        }
        notes
    }

    /// The items a STEP file's manufacturing information becomes on the faces of the
    /// import features of that file, one per solid, and what cannot be placed. Face keys of the file
    /// are matched to the feature's faces by the import's own face roles; an item on a
    /// face the import did not build is left out by name. Nothing is added to the part:
    /// run the returned items through
    /// [`Command::PutPmiItem`](crate::Command::PutPmiItem).
    pub fn pmi_from_step(&self, features: &[FeatureId], read: &StepPmi) -> (Vec<PmiItem<FaceRef>>, PmiImport) {
        let mut report = PmiImport::default();
        for skipped in &read.skipped {
            report.left_out.push((format!("{} {}", skipped.kind, skipped.entity), skipped.reason.clone()));
        }
        let mut items = Vec::new();
        let is_import = !features.is_empty()
            && features
                .iter()
                .all(|id| self.feature(*id).is_some_and(|f| matches!(f.kind, FeatureKind::ImportStep(_))));
        let body = self.body();
        let topology = body.and_then(|body| self.view_of(body).ok());
        let (true, Some(topology)) = (is_import, topology) else {
            for item in &read.pmi.items {
                report.left_out.push((item.id.clone(), "The imported body is not in the part.".to_owned()));
            }
            return (items, report);
        };
        // The face of the body that carries each key of the file.
        let face_of = |key: &str| -> Option<FaceId> {
            let mut found = topology.faces.iter().filter(|face| {
                face.origins.iter().any(|origin| {
                    crate::feature_of_tag(origin.tag.as_str()).is_some_and(|id| features.contains(&id))
                        && matches!(&origin.role, FaceRole::Imported { key: k } if k == key)
                })
            });
            let first = found.next()?;
            found.next().is_none().then_some(first.id)
        };
        // The locating distances a file of ours holds beside a position tolerance are
        // not items: the part measures its own.
        let derived = |item: &PmiItem<String>| -> bool {
            let PmiKind::Location { from, to, tolerance: Tolerance::Basic, .. } = &item.kind else { return false };
            from.at == At::Feature
                && to.at == At::Feature
                && read.pmi.items.iter().any(|other| {
                    let PmiKind::Gdt { characteristic: GdtCharacteristic::Position, datums, features, .. } =
                        &other.kind
                    else {
                        return false;
                    };
                    features.iter().enumerate().any(|(index, feature)| {
                        same_faces(feature, &to.feature)
                            && datums.iter().any(|datum| {
                                item.id == locating_name(&other.id, index, &datum.letter)
                                    && read
                                        .pmi
                                        .datum_feature(&datum.letter)
                                        .is_some_and(|on| same_faces(on, &from.feature))
                            })
                    })
                })
        };
        let mut taken = self.pmi.clone();
        for item in read.pmi.items.iter().filter(|item| !derived(item)) {
            let placed = item.try_map(|key| {
                let face = face_of(key).ok_or_else(|| "A face it is attached to was not imported.".to_owned())?;
                self.face_ref(face).map_err(|error| error.to_string())
            });
            let mut item = match placed {
                Ok(item) => item,
                Err(reason) => {
                    report.left_out.push((item.id.clone(), reason));
                    continue;
                }
            };
            // The part measures its own dimensions; the file's value is not kept.
            match &mut item.kind {
                PmiKind::Size { nominal, .. } | PmiKind::Location { nominal, .. } => *nominal = None,
                _ => {}
            }
            item.id = taken.free_name(&item.id.replace('/', " "));
            if let PmiKind::Datum { letter, .. } = &item.kind
                && taken.datum_letters().contains(&letter.as_str())
            {
                report.left_out.push((item.id.clone(), format!("Datum {letter} is already defined in the part.")));
                continue;
            }
            match taken.problem_with(&item, None) {
                Some(problem) => report.left_out.push((item.id.clone(), problem)),
                None => {
                    report.added.push(item.id.clone());
                    taken.items.push(item.clone());
                    items.push(item);
                }
            }
        }
        (items, report)
    }
}
