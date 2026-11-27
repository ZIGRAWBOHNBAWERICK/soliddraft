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

pub mod inspection;

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::drawing::Curve3;
use soliddraft_kernel_api::pmi::{
    Anchor, At, Feature, GdtCharacteristic, HoleHeadKind, HoleHeadNominals, HoleNominals, PartNotes, Pmi, PmiItem,
    PmiKind, PositionFrame, StepPmi, TargetArea, TargetKind, Tolerance, length_text,
};
use soliddraft_kernel_api::{
    BodyId, EdgeId, FaceAxis, FaceId, FaceRole, HolePart, Kernel, KernelError, SurfaceKind, Topology,
};
use soliddraft_kernel_api::{FaceCarrier, Vec3};

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
    /// Nominals in millimetres unless angular: basic angle is in degrees; chamfer is
    /// first-side setback in millimetres followed by bevel angle in degrees.
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
    sweep_rad: f64,
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
            let (center, normal, radius, sweep) = match curve.curve {
                Curve3::Circle { center, normal, radius } if !curve.approximate => {
                    (center, normal, radius, std::f64::consts::TAU)
                }
                Curve3::Arc { center, normal, radius, start_deg, end_deg, .. } if !curve.approximate => {
                    (center, normal, radius, (end_deg - start_deg).to_radians())
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
                    rims.push(HoleRim { z, radius, sweep_rad: 0.0, edges: Vec::new(), neighbours: Vec::new() });
                    rims.last_mut().unwrap()
                }
            };
            if !rim.edges.contains(&edge.id) {
                rim.sweep_rad += sweep;
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
    let end_matches = |rim: &HoleRim| if *through { opening(rim) } else { closed(rim) };
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
    } else if opening(&pilot_rims[0]) && end_matches(&pilot_rims[1]) {
        (pilot_rims[0].z, &pilot_rims[1], None)
    } else if !*through && opening(&pilot_rims[1]) && closed(&pilot_rims[0]) {
        (pilot_rims[1].z, &pilot_rims[0], None)
    } else {
        return Err("The pilot does not have the selected through or blind extent.".into());
    };
    if head.is_some() && !end_matches(end) {
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

fn direction(vector: [f64; 3]) -> Result<Vec3, String> {
    let [x, y, z] = vector;
    let scale = x.abs().max(y.abs()).max(z.abs());
    if !vector.into_iter().all(f64::is_finite) || scale == 0.0 {
        return Err("The direction must be finite and nonzero.".into());
    }
    let scaled = Vec3::new(x / scale, y / scale, z / scale);
    Ok(scaled * (1.0 / scaled.length()))
}

fn cylinder(carriers: &[FaceCarrier], group: &[FaceId]) -> Result<FaceAxis, String> {
    let axis_of = |id| carriers.iter().find(|face| face.id == id).and_then(|face| face.axis);
    let axis = group.first().copied().and_then(axis_of).ok_or("Pick a checked cylindrical feature.")?;
    for &id in group {
        let other = axis_of(id).ok_or("Every controlled face must be cylindrical.")?;
        if (axis.radius - other.radius).abs() > HOLE_EPS
            || (axis.point - other.point).cross(axis.direction).length() > HOLE_EPS
            || axis.direction.cross(other.direction).length() > HOLE_EPS
        {
            return Err("A feature group must describe one checked cylinder.".into());
        }
    }
    Ok(axis)
}

fn full_cylinder(kernel: &dyn Kernel, body: BodyId, topology: &Topology, faces: &[FaceId], axis: FaceAxis) -> bool {
    hole_rims(kernel, body, topology, faces, axis).is_ok_and(|rims| {
        rims.iter().all(|rim| {
            (rim.sweep_rad - std::f64::consts::TAU).abs() <= HOLE_EPS && (rim.radius - axis.radius).abs() <= HOLE_EPS
        })
    })
}

fn feature_of_size(
    kernel: &dyn Kernel,
    body: BodyId,
    topology: &Topology,
    carriers: &[FaceCarrier],
    faces: &[FaceId],
) -> bool {
    if let Ok(axis) = cylinder(carriers, faces) {
        return full_cylinder(kernel, body, topology, faces, axis);
    }
    matches!(faces, [a, b] if a != b)
        && planar_group(carriers, &faces[..1]).ok().zip(planar_group(carriers, &faces[1..]).ok()).is_some_and(
            |((a, na), (b, nb))| {
                na.dot(nb) < 0.0 && na.cross(nb).length() <= HOLE_EPS && (b - a).dot(na).abs() > HOLE_EPS
            },
        )
}

enum TargetFootprint {
    Round(f64),
    Box([f64; 2]),
}

impl TargetFootprint {
    // The chord is inflated by its proved sagitta. A trim entering this envelope
    // prevents a containment proof, including a hole wholly inside the footprint.
    fn touches(&self, a: [f64; 2], b: [f64; 2], error: f64) -> bool {
        let d = [b[0] - a[0], b[1] - a[1]];
        match self {
            Self::Round(radius) => {
                let square = d[0] * d[0] + d[1] * d[1];
                let t = if square == 0.0 { 0.0 } else { (-(a[0] * d[0] + a[1] * d[1]) / square).clamp(0.0, 1.0) };
                (a[0] + t * d[0]).hypot(a[1] + t * d[1]) <= radius + error
            }
            Self::Box(half) => {
                let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
                for i in 0..2 {
                    let bound = half[i] + error;
                    if d[i] == 0.0 {
                        if a[i].abs() > bound {
                            return false;
                        }
                    } else {
                        let t0 = (-bound - a[i]) / d[i];
                        let t1 = (bound - a[i]) / d[i];
                        lo = lo.max(t0.min(t1));
                        hi = hi.min(t0.max(t1));
                    }
                }
                lo <= hi
            }
        }
    }
}

fn target_on_face(
    kernel: &dyn Kernel,
    body: BodyId,
    carriers: &[FaceCarrier],
    target: &soliddraft_kernel_api::pmi::DatumTarget<FaceId>,
) -> Result<(), String> {
    let face = carriers.iter().find(|face| face.id == target.face).ok_or("The target face is missing.")?;
    let (Some(origin), Some(normal)) = (face.on_plane, face.normal) else {
        return Err("The target needs a checked planar supporting face.".into());
    };
    let [x, y, z] = target.point_mm;
    let point = Vec3::new(x, y, z);
    if !target.point_mm.into_iter().all(f64::is_finite) || (point - origin).dot(normal).abs() > HOLE_EPS {
        return Err("The target point is off its supporting face.".into());
    }
    let (footprint, along) = match &target.kind {
        TargetKind::Point => (TargetFootprint::Round(0.0), None),
        TargetKind::Area { area: TargetArea::Circular { diameter_mm } } => {
            (TargetFootprint::Round(diameter_mm.value() / 2.0), None)
        }
        TargetKind::Line { length_mm, direction: along } => {
            (TargetFootprint::Box([length_mm.value() / 2.0, 0.0]), Some(direction(*along)?))
        }
        TargetKind::Area { area: TargetArea::Rectangular { length_mm, width_mm, direction: along } } => {
            (TargetFootprint::Box([length_mm.value() / 2.0, width_mm.value() / 2.0]), Some(direction(*along)?))
        }
    };
    let x_axis = along.unwrap_or_else(|| {
        let axis = normal.cross(if normal.x.abs() < 0.8 { Vec3::new(1.0, 0.0, 0.0) } else { Vec3::Z });
        axis * (1.0 / axis.length())
    });
    if x_axis.dot(normal).abs() > HOLE_EPS {
        return Err("The datum target direction must be tangent to its supporting plane.".into());
    }
    let y_axis = normal.cross(x_axis);
    let local = |p: Vec3| {
        let d = p - point;
        [d.dot(x_axis), d.dot(y_axis)]
    };
    let loops = kernel.face_drawing_loops(body, target.face).map_err(|error| error.to_string())?;
    let mut inside = false;
    let mut on_line = false;
    for edge in loops.iter().flatten() {
        let (radius, angle) = match &edge.curve {
            Curve3::Line { .. } if !edge.approximate => (0.0, 0.0),
            Curve3::Circle { radius, .. } if !edge.approximate => (*radius, std::f64::consts::TAU),
            Curve3::Arc { radius, start_deg, end_deg, .. } if !edge.approximate => {
                (*radius, (end_deg - start_deg).to_radians())
            }
            _ => return Err("The target trim cannot be checked from its boundary curves.".into()),
        };
        let steps = (angle * (radius / (8.0 * HOLE_EPS)).sqrt()).ceil().clamp(1.0, 4096.0) as u32;
        let half_angle = angle / (4.0 * f64::from(steps));
        let error = 2.0 * radius * half_angle.sin().powi(2);
        let mut a = local(edge.curve.start());
        for step in 1..=steps {
            let b = local(edge.curve.point_at(f64::from(step) / f64::from(steps)));
            if footprint.touches(a, b, error + HOLE_EPS) {
                if matches!(target.kind, TargetKind::Point) && error == 0.0 && footprint.touches(a, b, HOLE_EPS) {
                    on_line = true;
                } else {
                    return Err("The target footprint meets or crosses the supporting face trim.".into());
                }
            }
            if (a[1] > 0.0) != (b[1] > 0.0) && a[0] + (b[0] - a[0]) * (-a[1] / (b[1] - a[1])) > 0.0 {
                inside = !inside;
            }
            a = b;
        }
    }
    if inside || on_line { Ok(()) } else { Err("The target point is off its supporting face.".into()) }
}

struct ProjectionEntry {
    face: FaceId,
    outward: Vec3,
}

fn projection_start(
    kernel: &dyn Kernel,
    body: BodyId,
    topology: &Topology,
    carriers: &[FaceCarrier],
    group: &[FaceId],
    axis: FaceAxis,
    entry: ProjectionEntry,
) -> Result<(), String> {
    let ProjectionEntry { face: entry, outward } = entry;
    let face = carriers.iter().find(|face| face.id == entry).ok_or("The projection entry face is missing.")?;
    let (Some(point), Some(normal)) = (face.on_plane, face.normal) else {
        return Err("The projection entry needs a checked planar face.".into());
    };
    if normal.dot(outward) <= 0.0
        || normal.cross(outward).length() > HOLE_EPS
        || axis.direction.cross(outward).length() > HOLE_EPS
    {
        return Err("The projected direction must run outward along the controlled axis.".into());
    }
    let z = (point - axis.point).dot(normal) / axis.direction.dot(normal);
    let rims = hole_rims(kernel, body, topology, group, axis)?;
    let mut openings = rims.iter().filter(|rim| (rim.z - z).abs() <= HOLE_EPS && rim.neighbours.contains(&entry));
    let rim = openings
        .next()
        .filter(|_| openings.next().is_none())
        .ok_or("The axis does not have a unique opening at the selected entry.")?;
    if (rim.radius - axis.radius).abs() > HOLE_EPS
        || rims.iter().any(|other| (other.z - rim.z) * axis.direction.dot(outward) > HOLE_EPS)
        || topology.face(entry).is_none_or(|face| face.edges.iter().all(|edge| rim.edges.contains(edge)))
    {
        return Err("The selected entry must open outward into free space.".into());
    }
    Ok(())
}

fn checked_gdt_kind(kind: &PmiKind<FaceRef>) -> bool {
    match kind {
        PmiKind::DatumTarget(_) | PmiKind::PositionControl(_) => true,
        PmiKind::Gdt { material, datums, .. } => {
            !material.is_regardless() || datums.iter().any(|datum| !datum.material.is_regardless())
        }
        _ => false,
    }
}

fn planar_group(carriers: &[FaceCarrier], ids: &[FaceId]) -> Result<(Vec3, Vec3), String> {
    let plane = |id| {
        let face = carriers.iter().find(|face| face.id == id && face.surface == SurfaceKind::Plane)?;
        Some((face.on_plane?, face.normal?.normalized()?))
    };
    let (point, normal) = ids.first().and_then(|id| plane(*id)).ok_or("Pick planar faces for this dimension.")?;
    if ids[1..].iter().any(|id| {
        plane(*id).is_none_or(|(other, direction)| {
            direction.dot(normal) <= 0.0
                || direction.cross(normal).length() > HOLE_EPS
                || (other - point).dot(normal).abs() > HOLE_EPS
        })
    }) {
        return Err("Faces in one group must lie on the same oriented plane.".into());
    }
    Ok((point, normal))
}

fn measure_planar_pmi(
    topology: &Topology,
    carriers: &[FaceCarrier],
    kind: &PmiKind<FaceId>,
) -> Result<Vec<f64>, String> {
    let plane = |group: &Feature<FaceId>| planar_group(carriers, group);
    match kind {
        PmiKind::Width { flats: [a, b], .. } => {
            let ((p, n), (q, m)) = (plane(a)?, plane(b)?);
            let width = (q - p).dot(n).abs();
            if n.dot(m) >= 0.0 || n.cross(m).length() > HOLE_EPS || !(width > HOLE_EPS && width.is_finite()) {
                return Err("Width needs two distinct opposed parallel planes.".into());
            }
            Ok(vec![width])
        }
        PmiKind::BasicAngle { planes: [a, b] } => {
            let ((_, n), (_, m)) = (plane(a)?, plane(b)?);
            let sine = n.cross(m).length();
            if sine <= HOLE_EPS {
                return Err("Basic angle needs two nonparallel planes.".into());
            }
            Ok(vec![sine.atan2(-n.dot(m)).to_degrees()])
        }
        PmiKind::Chamfer { bevel, sides: [a, b] } | PmiKind::EdgeBreak { bevel, sides: [a, b], .. } => {
            let ((p, n), (q, m), (r, h)) = (plane(a)?, plane(b)?, plane(bevel)?);
            let adjacent = |side: &[FaceId]| {
                bevel.iter().any(|id| {
                    topology.face(*id).is_some_and(|face| {
                        face.edges
                            .iter()
                            .filter_map(|id| topology.edge(*id))
                            .any(|edge| edge.faces.iter().any(|face| side.contains(face)))
                    })
                })
            };
            let along = n.cross(m);
            if along.length() <= HOLE_EPS || along.dot(h).abs() > HOLE_EPS || !adjacent(a) || !adjacent(b) {
                return Err("The bevel must meet both sides along their corner direction.".into());
            }
            let setback = |p: Vec3, n: Vec3, q: Vec3, m: Vec3| -> Option<f64> {
                let across = (m - n * m.dot(n)).normalized()?;
                let corner = p + across * ((q - p).dot(m) / across.dot(m));
                let slope = across.dot(h);
                if slope <= HOLE_EPS {
                    return None;
                }
                let value = (corner - r).dot(h) / slope;
                (value > HOLE_EPS && value.is_finite()).then_some(value)
            };
            let first = setback(p, n, q, m).ok_or("The bevel must cut back the theoretical corner.")?;
            let second = setback(q, m, p, n).ok_or("The bevel must cut back the theoretical corner.")?;
            if matches!(kind, PmiKind::EdgeBreak { .. }) {
                Ok(vec![first.max(second)])
            } else {
                let angle = n.cross(h).length().atan2(n.dot(h).abs()).to_degrees();
                Ok(vec![first, angle])
            }
        }
        _ => unreachable!("only native planar PMI goes through this measurement"),
    }
}

impl Document {
    fn checked_gdt_on(
        &self,
        state: &PartPmi,
        item: &PmiItem<FaceId>,
        body: BodyId,
        topology: &Topology,
        names: &reference::NameMap,
        carriers: &[FaceCarrier],
    ) -> Result<(), String> {
        let check_datum = |datum: &soliddraft_kernel_api::pmi::DatumRef, require_face: bool| -> Result<(), String> {
            for letter in datum.letters() {
                if !require_face && datum.material.is_regardless() {
                    continue;
                }
                let declared = state
                    .items
                    .iter()
                    .find(|item| {
                        matches!(&item.kind, PmiKind::Datum { letter: name, .. }
                    if name == letter)
                    })
                    .ok_or_else(|| format!("Datum {letter} is not defined."))?;
                let placed =
                    self.item_on(declared, topology, names).map_err(|error| format!("Datum {letter}: {error}"))?;
                let PmiKind::Datum { feature, .. } = placed.kind else { unreachable!() };
                if !datum.material.is_regardless()
                    && (!feature_of_size(self.kernel.as_ref(), body, topology, carriers, &feature)
                        || state
                            .items
                            .iter()
                            .any(|item| matches!(&item.kind, PmiKind::DatumTarget(t) if t.datum == letter)))
                {
                    return Err(format!("Datum {letter} does not support a material boundary modifier."));
                }
            }
            Ok(())
        };
        match &item.kind {
            PmiKind::DatumTarget(target) => target_on_face(self.kernel.as_ref(), body, carriers, target),
            PmiKind::PositionControl(control) => {
                let projected = match &control.frame {
                    PositionFrame::Single { projected_zone, .. } => projected_zone.as_ref(),
                    PositionFrame::Composite { .. } => None,
                };
                let outward = projected.map(|zone| direction(zone.direction)).transpose()?;
                let material = control.frame.segments().position(|segment| !segment.material.is_regardless());
                for (i, group) in control.features.iter().enumerate() {
                    if control.features[..i].iter().any(|other| group.iter().any(|face| other.contains(face))) {
                        return Err("Controlled feature groups must not reuse faces.".into());
                    }
                    let axis =
                        cylinder(carriers, group).map_err(|error| format!("Controlled feature {}: {error}", i + 1))?;
                    if let Some(segment) = material
                        && !full_cylinder(self.kernel.as_ref(), body, topology, group, axis)
                    {
                        return Err(format!(
                            "Position segment {}: feature {} does not support a material modifier.",
                            segment + 1,
                            i + 1
                        ));
                    }
                    if let (Some(zone), Some(outward)) = (projected, outward) {
                        let entry =
                            *zone.entry_faces.get(i).ok_or("A projected zone needs an entry for every feature.")?;
                        projection_start(
                            self.kernel.as_ref(),
                            body,
                            topology,
                            carriers,
                            group,
                            axis,
                            ProjectionEntry { face: entry, outward },
                        )
                        .map_err(|error| format!("Projected entry {}: {error}", i + 1))?;
                    }
                }
                for (i, segment) in control.frame.segments().enumerate() {
                    for datum in &segment.datums {
                        check_datum(datum, true).map_err(|error| format!("Position segment {}: {error}", i + 1))?;
                    }
                }
                Ok(())
            }
            PmiKind::Gdt { characteristic, material, datums, features, .. } => {
                if !material.is_regardless()
                    && (!matches!(
                        characteristic,
                        GdtCharacteristic::Position
                            | GdtCharacteristic::Straightness
                            | GdtCharacteristic::Flatness
                            | GdtCharacteristic::Parallelism
                            | GdtCharacteristic::Perpendicularity
                            | GdtCharacteristic::Angularity
                    ) || features
                        .iter()
                        .any(|group| !feature_of_size(self.kernel.as_ref(), body, topology, carriers, group)))
                {
                    return Err("This control does not support a material modifier on its features.".into());
                }
                datums.iter().try_for_each(|datum| check_datum(datum, false))
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn check_pmi_edit(&self, state: &PartPmi, item: &PmiItem<FaceRef>) -> Result<(), String> {
        if let Some(problem) = state.problem_with(item, Some(&item.id)) {
            return Err(problem);
        }
        let needs_targets = |other: &&PmiItem<FaceRef>| matches!(&other.kind, PmiKind::DatumTarget(t) if item.references_datum(&t.datum));
        if !checked_gdt_kind(&item.kind) && !state.items.iter().any(|other| needs_targets(&other)) {
            return Ok(());
        }
        let names = self.name_map();
        let mut problem = "The part has no body for this PMI item.".to_owned();
        for (_, body) in self.live_bodies() {
            let checked = (|| {
                let topology = self.view_of(body).map_err(|error| error.to_string())?;
                let on = self.item_on(item, &topology, &names)?;
                let carriers = self.kernel.face_carriers(body).map_err(|error| error.to_string())?;
                self.checked_gdt_on(state, &on, body, &topology, &names, &carriers)?;
                for target in state.items.iter().filter(needs_targets).filter(|target| target.id != item.id) {
                    let on = self.item_on(target, &topology, &names)?;
                    self.checked_gdt_on(state, &on, body, &topology, &names, &carriers)
                        .map_err(|problem| format!("Datum target {}: {problem}", target.id))?;
                }
                Ok::<_, String>(())
            })();
            match checked {
                Ok(()) => return Ok(()),
                Err(error) => problem = error,
            }
        }
        Err(problem)
    }

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
        let carriers: Vec<std::cell::OnceCell<_>> = (0..bodies.len()).map(|_| std::cell::OnceCell::new()).collect();
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
            for (index, (body, topology)) in
                bodies.iter().enumerate().filter(|(_, (body, _))| wanted.is_none_or(|wanted| wanted == *body))
            {
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
                        if checked_gdt_kind(&item.kind) {
                            problem = self.pmi.problem_with(item, Some(&item.id)).or_else(|| {
                                carriers[index]
                                    .get_or_init(|| self.kernel.face_carriers(*body))
                                    .as_ref()
                                    .map_err(|error| error.to_string())
                                    .and_then(|carriers| {
                                        self.checked_gdt_on(&self.pmi, &on_body, *body, topology, &names, carriers)
                                    })
                                    .err()
                            });
                        } else if matches!(on_body.kind, PmiKind::Hole { .. }) {
                            match measure_hole(self.kernel.as_ref(), *body, topology, &on_body) {
                                Ok(values) => {
                                    row.measured.push(values.pilot_diameter_mm);
                                    row.hole = Some(values);
                                }
                                Err(error) => problem = Some(error),
                            }
                        } else if matches!(
                            on_body.kind,
                            PmiKind::Width { .. }
                                | PmiKind::Chamfer { .. }
                                | PmiKind::EdgeBreak { .. }
                                | PmiKind::BasicAngle { .. }
                        ) {
                            match carriers[index]
                                .get_or_init(|| self.kernel.face_carriers(*body))
                                .as_ref()
                                .map_err(|error| error.to_string())
                                .and_then(|carriers| measure_planar_pmi(topology, carriers, &on_body.kind))
                            {
                                Ok(values) => row.measured = values,
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
        let standing: std::collections::BTreeMap<_, _> = rows
            .iter()
            .filter(|row| row.applies())
            .filter_map(|row| match row.on_body.as_ref().map(|item| &item.kind) {
                Some(PmiKind::Datum { letter, feature }) => Some((letter.clone(), (row.body, feature.clone()))),
                _ => None,
            })
            .collect();
        let mut targeted = std::collections::BTreeMap::new();
        for row in &rows {
            if let PmiKind::DatumTarget(target) = &row.item.kind {
                targeted.entry(target.datum.clone()).and_modify(|all| *all &= row.applies()).or_insert(row.applies());
            }
        }
        for row in &mut rows {
            if row.problem.is_none()
                && let Some(missing) = row.item.referenced_datums().find(|letter| {
                    standing.get(*letter).is_none_or(|(body, _)| *body != row.body)
                        || (targeted.get(*letter) == Some(&false) && !matches!(row.item.kind, PmiKind::DatumTarget(_)))
                })
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
                    if targeted.contains_key(datum.letter.as_str()) {
                        continue;
                    }
                    let Some((_, from)) = standing.get(datum.letter.as_str()) else { continue };
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
        item.map_attachments(
            &mut (),
            |_, feature| {
                // A bevel can tilt when its setback changes, and the angle measures
                // precisely such a change. Keep the origin and surface checks; validate
                // the resolved planes and their relationships in measure_planar_pmi.
                let may_turn = match &item.kind {
                    PmiKind::Chamfer { bevel, .. } | PmiKind::EdgeBreak { bevel, .. } => {
                        std::ptr::eq(feature, bevel.as_slice())
                    }
                    PmiKind::BasicAngle { .. } => true,
                    _ => false,
                };
                let mut faces: Feature<FaceId> = Vec::new();
                for face in feature {
                    let mut turning;
                    let reference = if may_turn && face.fingerprint.normal.is_some() {
                        turning = face.clone();
                        turning.fingerprint.normal = None;
                        &turning
                    } else {
                        face
                    };
                    let found = reference::resolve_faces(reference, &view, names).map_err(|unresolved| {
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
            },
            |_, face| {
                let resolved = reference::resolve_faces(face, &view, names).map_err(|error| error.message)?;
                match resolved.as_slice() {
                    [face] => Ok(*face),
                    _ => Err("A single-face attachment is ambiguous after the geometry edit.".into()),
                }
            },
        )
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
        let mut invalid_datums: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        // Resolve declarations and targets before frames, independent of file order.
        let stage = |item: &PmiItem<String>| match item.kind {
            PmiKind::Datum { .. } => 0,
            PmiKind::DatumTarget(_) => 1,
            _ => 2,
        };
        let mut ordered: Vec<_> = read.pmi.items.iter().filter(|item| !derived(item)).collect();
        ordered.sort_by_key(|item| stage(item));
        for item in ordered {
            let datum = match &item.kind {
                PmiKind::Datum { letter, .. } => Some(letter.as_str()),
                PmiKind::DatumTarget(target) => Some(target.datum.as_str()),
                _ => None,
            };
            if let Some(datum) = invalid_datums.iter().find(|letter| item.references_datum(letter)) {
                report.left_out.push((item.id.clone(), format!("Datum {datum} could not be imported.")));
                continue;
            }
            let placed = item.try_map(|key| {
                let face = face_of(key).ok_or_else(|| "A face it is attached to was not imported.".to_owned())?;
                self.face_ref(face).map_err(|error| error.to_string())
            });
            let mut item = match placed {
                Ok(item) => item,
                Err(reason) => {
                    invalid_datums.extend(datum);
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
                invalid_datums.extend(datum);
                report.left_out.push((item.id.clone(), format!("Datum {letter} is already defined in the part.")));
                continue;
            }
            match self.check_pmi_edit(&taken, &item) {
                Err(problem) => {
                    invalid_datums.extend(datum);
                    report.left_out.push((item.id.clone(), problem));
                }
                Ok(()) => {
                    taken.items.push(item.clone());
                    report.added.push(item.id.clone());
                    items.push(item);
                }
            }
        }
        (items, report)
    }
}
