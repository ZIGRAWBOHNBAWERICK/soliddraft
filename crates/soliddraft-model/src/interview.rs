//! The interview: what a part's manufacturing information still lacks, asked one
//! decision at a time, each with a proposal and its reason.
//!
//! An imported part has geometry and no requirements. The kernel reads its features off
//! its faces ([`Document::recognised`]); the interview then lists what a drawing of the
//! part still lacks: material and general tolerance, datum and dimension checks, and
//! requirements such as fits and threads. A decision the part has already taken is
//! not asked again.
//!
//! Every proposal comes from a rule, never from a guess about intent:
//!
//! - datums: the largest flat face, then the largest flat face square to it, then the
//!   largest square to both;
//! - measured dimensions describe geometry, not the intended fit, thread,
//!   position tolerance or finish;
//! - a boss and a slot get their sizes and a distance from the datum they are parallel
//!   to, under the general tolerance;
//! - the general tolerance is ISO 2768-mK.
//!
//! A question about a feature names the feature by its faces, so two features of one
//! size are two questions. Each hole of a pattern is asked about on its own account:
//! an item on some of its holes answers for those holes only, and the question that
//! remains asks about the rest.
//!
//! What recognition found and what the interview proposes are kept apart: recognition
//! is geometry and is the same whatever the part states, while a proposal is a
//! suggestion a person accepts or replaces.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::pmi::{
    Anchor, At, Feature, GdtCharacteristic, HoleHead, HoleHeadKind, HoleHeadNominals, HoleNominals, PmiItem, PmiKind,
    SizeKind, Tolerance, length_text,
};
use soliddraft_kernel_api::recognition::{CompoundHead, EdgeTreatmentKind, Recognised};
use soliddraft_kernel_api::{FaceId, FaceInfo, SurfaceKind, Topology, Vec3};

use crate::document::Document;
use crate::reference::FaceRef;

/// A feature the kernel read off the body, on the body's faces.
#[derive(Clone, Debug, PartialEq)]
pub struct RecognisedFeature {
    /// The feature kind, such as `hole`, `counterbore`, `pocket` or `chamfer`.
    pub kind: String,
    /// What the interview and the panel call it, such as `Hole Ø8` or `4 holes Ø4`.
    pub name: String,
    /// The faces that are the feature: a hole's or a boss's wall, each hole's wall of a
    /// pattern, a slot's flats and ends.
    pub faces: Vec<FaceId>,
    /// For a hole or a slot: true when nothing closes it.
    pub through: Option<bool>,
    /// The faces a hole opens onto, or the face a boss stands on.
    pub opens_onto: Vec<FaceId>,
    /// The faces that close a hole, or the face on top of a boss.
    pub closed_by: Vec<FaceId>,
    /// Its linear dimensions in millimetres, by name.
    pub sizes: Vec<(String, f64)>,
    /// The included countersink angle or the chamfer angle, in degrees.
    pub angle_deg: Option<f64>,
}

/// What a question proposes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Proposal {
    /// An item to add to the part.
    Item {
        /// The item.
        item: Box<PmiItem<FaceRef>>,
    },
    /// The part's general tolerance.
    GeneralTolerance {
        /// Such as `ISO 2768-mK`.
        text: String,
    },
}

/// One decision the part has not taken yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Question {
    /// A name that stays the same while the question is open, such as `datum A`. A
    /// question about a feature ends with the persistent names of the faces it asks
    /// about, one for each hole of a pattern, such as `hole Ø8 size: Import1 · Face 7`.
    pub id: String,
    /// The question, for people.
    pub asks: String,
    /// The proposed answer as a drawing writes it, such as `Ø8 H7`. Empty when the
    /// rules have nothing to propose and a person must say.
    pub proposes: String,
    /// Why that proposal.
    pub reason: String,
    /// The proposal itself, to accept as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<Proposal>,
}

/// A flat face of the body as a datum sees it.
#[derive(Clone, Copy)]
struct Flat {
    face: FaceId,
    normal: Vec3,
    centroid: Vec3,
    area: f64,
}

fn square(a: Vec3, b: Vec3) -> bool {
    a.dot(b).abs() < 1e-9
}

fn parallel(a: Vec3, b: Vec3) -> bool {
    a.cross(b).length() < 1e-9
}

/// The id of a question about a feature: what it asks, then the persistent name of one
/// face of each member it asks about. Members own disjoint faces, so features of one
/// size, and different holes of one pattern, never share an id.
fn question_id(key: &str, aspect: &str, members: &[Feature<FaceRef>]) -> String {
    let faces: Vec<String> = members
        .iter()
        .filter_map(|member| member.first())
        .map(|face| {
            if face.label.is_empty() {
                format!("{} {} {}", face.feature, face.generator, face.ordinal + 1)
            } else {
                face.label.clone()
            }
        })
        .collect();
    format!("{key} {aspect}: {}", faces.join(", "))
}

/// The body's state the questions are worked out from.
struct Asking<'a> {
    doc: &'a Document,
    topology: &'a Topology,
    /// Faces under a size, a position tolerance, a finish, a slot item and a distance.
    sized: BTreeSet<FaceId>,
    hole_intent: BTreeSet<FaceId>,
    radii: BTreeSet<FaceId>,
    edge_limits: BTreeSet<FaceId>,
    plane_distances: BTreeSet<(FaceId, FaceId)>,
    native_holes: Vec<&'a PmiKind<FaceId>>,
    positioned: BTreeSet<FaceId>,
    finished: BTreeSet<FaceId>,
    slotted: BTreeSet<FaceId>,
    located: BTreeSet<FaceId>,
    /// Names taken by the part's items and by proposals so far.
    names: BTreeSet<String>,
    /// The datum planes the part has or is proposed to have, primary first.
    datums: Vec<(String, Flat)>,
    questions: Vec<Question>,
}

impl Asking<'_> {
    fn face(&self, face: FaceId) -> Option<&FaceInfo> {
        self.topology.faces.iter().find(|f| f.id == face)
    }

    fn refs(&self, faces: &[FaceId]) -> Option<Vec<FaceRef>> {
        faces.iter().map(|face| self.doc.face_ref(*face).ok()).collect()
    }

    /// A name no item and no proposal has yet.
    fn name(&mut self, base: &str) -> String {
        let mut name = base.to_owned();
        let mut n = 2;
        while !self.names.insert(name.clone()) {
            name = format!("{base} {n}");
            n += 1;
        }
        name
    }

    fn ask(&mut self, id: String, asks: String, reason: &str, item: PmiItem<FaceRef>, measured: &[f64]) {
        self.questions.push(Question {
            id,
            asks,
            proposes: item.callout(measured),
            reason: reason.to_owned(),
            proposal: Some(Proposal::Item { item: Box::new(item) }),
        });
    }

    fn unanswered(&mut self, id: String, asks: String, reason: &str) {
        self.questions.push(Question { id, asks, proposes: String::new(), reason: reason.into(), proposal: None });
    }

    /// The datum plane parallel to a face with this normal: the one a height or a depth
    /// along the normal is held from.
    fn datum_along(&self, normal: Vec3) -> Option<(String, Flat)> {
        self.datums.iter().find(|(_, flat)| parallel(flat.normal, normal)).cloned()
    }

    /// The distance from a datum plane to a face parallel to it.
    fn distance(&self, datum: &Flat, to: FaceId) -> Option<f64> {
        let face = self.face(to)?;
        Some((face.centroid - datum.centroid).dot(datum.normal).abs())
    }
}

impl Document {
    /// The features the kernel reads off the part's body.
    /// This is geometry: it does not depend on what manufacturing information the part
    /// states, and nothing in it is a proposal.
    pub fn recognised(&self) -> Result<Vec<RecognisedFeature>, String> {
        let body = self.body().ok_or("The part has no body yet.")?;
        let found = self.kernel().recognise(body).map_err(|error| error.to_string())?;
        let labels = |faces: &[FaceId]| faces.to_vec();
        let mut out = Vec::new();
        for (index, hole) in found.holes.iter().enumerate() {
            if found.hole_patterns.iter().any(|pattern| pattern.contains(&index)) {
                continue;
            }
            let mut sizes = vec![("diameter".to_owned(), hole.diameter)];
            sizes.extend(hole.depth.map(|depth| ("depth".to_owned(), depth)));
            out.push(RecognisedFeature {
                kind: "hole".into(),
                name: format!("Hole Ø{}", length_text(hole.diameter)),
                faces: labels(&hole.faces),
                through: Some(hole.through),
                opens_onto: labels(&hole.opens_onto),
                closed_by: labels(&hole.bottoms),
                sizes,
                angle_deg: None,
            });
        }
        for pattern in &found.hole_patterns {
            let Some(first) = pattern.first().map(|hole| &found.holes[*hole]) else { continue };
            let mut sizes = vec![("diameter".to_owned(), first.diameter)];
            sizes.extend(first.depth.map(|depth| ("depth".to_owned(), depth)));
            out.push(RecognisedFeature {
                kind: "hole pattern".into(),
                name: format!("{} holes Ø{}", pattern.len(), length_text(first.diameter)),
                faces: pattern.iter().flat_map(|hole| labels(&found.holes[*hole].faces)).collect(),
                through: Some(first.through),
                opens_onto: labels(&first.opens_onto),
                closed_by: labels(&first.bottoms),
                sizes,
                angle_deg: None,
            });
        }
        for boss in &found.bosses {
            let mut sizes = vec![("diameter".to_owned(), boss.diameter)];
            sizes.extend(boss.height.map(|height| ("height".to_owned(), height)));
            out.push(RecognisedFeature {
                kind: "boss".into(),
                name: format!("Boss Ø{}", length_text(boss.diameter)),
                faces: labels(&boss.faces),
                through: None,
                opens_onto: labels(&[boss.base]),
                closed_by: labels(&[boss.top]),
                sizes,
                angle_deg: None,
            });
        }
        for slot in &found.slots {
            out.push(RecognisedFeature {
                kind: "slot".into(),
                name: format!("Slot {}×{}", length_text(slot.length), length_text(slot.width)),
                faces: slot.flats.iter().chain(slot.ends.iter().flatten()).copied().collect(),
                through: Some(slot.through),
                opens_onto: Vec::new(),
                closed_by: Vec::new(),
                sizes: vec![("length".to_owned(), slot.length), ("width".to_owned(), slot.width)],
                angle_deg: None,
            });
        }
        for hole in found.compound_holes {
            let (kind, angle_deg) = match hole.head {
                CompoundHead::Counterbore { .. } => ("counterbore", None),
                CompoundHead::Countersink { angle_deg } => ("countersink", Some(angle_deg)),
            };
            out.push(RecognisedFeature {
                kind: kind.into(),
                name: format!("{} Ø{}", capital(kind), length_text(hole.pilot_diameter)),
                faces: hole.pilot_faces.into_iter().chain(hole.head_faces).collect(),
                through: Some(hole.through),
                opens_onto: if hole.through { vec![hole.entry, hole.end] } else { vec![hole.entry] },
                closed_by: if hole.through { Vec::new() } else { vec![hole.end] },
                sizes: vec![
                    ("diameter".into(), hole.pilot_diameter),
                    ("depth".into(), hole.depth),
                    ("head_diameter".into(), hole.head_diameter),
                    ("head_depth".into(), hole.head_depth),
                ],
                angle_deg,
            });
        }
        for recess in found.recesses {
            let kind = if recess.open_sided { "step" } else { "pocket" };
            out.push(RecognisedFeature {
                kind: kind.into(),
                name: capital(kind),
                faces: std::iter::once(recess.floor).chain(recess.walls).collect(),
                through: None,
                opens_onto: recess.opens_onto,
                closed_by: vec![recess.floor],
                sizes: vec![("depth".into(), recess.depth)],
                angle_deg: None,
            });
        }
        for edge in found.edge_treatments {
            let (kind, dimension, value, angle_deg) = match edge.kind {
                EdgeTreatmentKind::Fillet { radius } => ("fillet", "radius", radius, None),
                EdgeTreatmentKind::Chamfer { setback } => ("chamfer", "setback", setback, Some(45.0)),
            };
            out.push(RecognisedFeature {
                kind: kind.into(),
                name: capital(kind),
                faces: vec![edge.face],
                through: None,
                opens_onto: Vec::new(),
                closed_by: Vec::new(),
                sizes: vec![(dimension.into(), value)],
                angle_deg,
            });
        }
        Ok(out)
    }

    /// The decisions the part's manufacturing information still lacks, in the order to
    /// take them, each with a proposal and its reason. Empty when nothing is missing.
    pub fn interview(&self) -> Result<Vec<Question>, String> {
        let body = self.body().ok_or("The part has no body yet.")?;
        let found: Recognised = self.kernel().recognise(body).map_err(|error| error.to_string())?;
        let topology = self.view_of(body).map_err(|error| error.to_string())?;
        let rows = self.pmi_rows();
        let mut asking = Asking {
            doc: self,
            topology: &topology,
            sized: BTreeSet::new(),
            hole_intent: BTreeSet::new(),
            radii: BTreeSet::new(),
            edge_limits: BTreeSet::new(),
            plane_distances: BTreeSet::new(),
            native_holes: Vec::new(),
            positioned: BTreeSet::new(),
            finished: BTreeSet::new(),
            slotted: BTreeSet::new(),
            located: BTreeSet::new(),
            names: self.pmi().items.iter().map(|item| item.id.clone()).collect(),
            datums: Vec::new(),
            questions: Vec::new(),
        };
        for row in &rows {
            if !row.applies() {
                continue;
            }
            let Some(item) = &row.on_body else { continue };
            let all = || item.features().into_iter().flatten().copied();
            match &item.kind {
                PmiKind::Size { size: SizeKind::Diameter, tolerance, thread, .. } => {
                    asking.sized.extend(all());
                    if !tolerance.is_none() || thread.is_some() {
                        asking.hole_intent.extend(all());
                    }
                }
                PmiKind::Size { size: SizeKind::Radius, tolerance, .. } => {
                    asking.radii.extend(all());
                    if !tolerance.is_none() {
                        asking.edge_limits.extend(all());
                    }
                }
                PmiKind::Hole { pilot, tolerance, thread, .. } => {
                    asking.native_holes.push(&item.kind);
                    asking.sized.extend(pilot.iter().copied());
                    if !tolerance.is_none() || thread.is_some() {
                        asking.hole_intent.extend(pilot.iter().copied());
                    }
                }
                PmiKind::Gdt { characteristic: GdtCharacteristic::Position, .. } => asking.positioned.extend(all()),
                PmiKind::SurfaceFinish { .. } => asking.finished.extend(all()),
                PmiKind::Slot { .. } => asking.slotted.extend(all()),
                PmiKind::Location { from, to, .. } => {
                    asking.located.extend(all());
                    if let (At::Feature, At::Feature, [a], [b]) =
                        (&from.at, &to.at, from.feature.as_slice(), to.feature.as_slice())
                    {
                        asking.plane_distances.insert(ordered_pair(*a, *b));
                    }
                }
                PmiKind::Chamfer { bevel, .. } | PmiKind::EdgeBreak { bevel, .. } => {
                    asking.edge_limits.extend(bevel);
                }
                PmiKind::PositionControl(_) => asking.positioned.extend(all()),
                PmiKind::Width { .. }
                | PmiKind::BasicAngle { .. }
                | PmiKind::DatumTarget(_)
                | PmiKind::Knurl { .. }
                | PmiKind::FaceNote { .. } => {}
                PmiKind::Datum { letter, feature } => {
                    let flat = feature.first().and_then(|face| asking.face(*face)).and_then(|info| {
                        Some(Flat { face: info.id, normal: info.normal?, centroid: info.centroid, area: info.area })
                    });
                    if let Some(flat) = flat {
                        asking.datums.push((letter.clone(), flat));
                    }
                }
                PmiKind::Gdt { .. } => {}
            }
        }

        let notes = self.part_notes();
        if notes.material.is_none() {
            asking.questions.push(Question {
                id: "material".into(),
                asks: "What is the part made of?".into(),
                proposes: String::new(),
                reason: "The part names no material, and no rule can tell one from a shape.".into(),
                proposal: None,
            });
        }
        if notes.general_tolerance.is_none() {
            let text = "ISO 2768-mK".to_owned();
            asking.questions.push(Question {
                id: "general tolerance".into(),
                asks: "Which general tolerance covers what has none of its own?".into(),
                proposes: text.clone(),
                reason: "The medium class for lengths and class K for form: the usual pair for machined parts.".into(),
                proposal: Some(Proposal::GeneralTolerance { text }),
            });
        }
        datums(&mut asking);
        holes(&mut asking, &found);
        bosses(&mut asking, &found);
        slots(&mut asking, &found);
        recesses(&mut asking, &found);
        edges(&mut asking, &found);
        Ok(asking.questions)
    }
}

/// Three datums: the largest flat face, the largest square to it, the largest square to
/// both. Datums the part has already count, in the order it lists them.
fn datums(asking: &mut Asking) {
    let mut flats: Vec<Flat> = asking
        .topology
        .faces
        .iter()
        .filter(|face| face.surface == SurfaceKind::Plane)
        .filter_map(|face| Some(Flat { face: face.id, normal: face.normal?, centroid: face.centroid, area: face.area }))
        .collect();
    // Largest first; of equal faces the one nearest the origin, so the choice is the same
    // every time.
    flats.sort_by(|a, b| {
        let near = |flat: &Flat| flat.centroid.x + flat.centroid.y + flat.centroid.z;
        if (a.area - b.area).abs() > 1e-6 * a.area.max(b.area).max(1.0) {
            b.area.total_cmp(&a.area)
        } else {
            near(a).total_cmp(&near(b))
        }
    });
    let reasons = [
        "The largest flat face: the part rests on it.",
        "The largest flat face square to the first datum.",
        "The largest flat face square to the first two datums.",
    ];
    let mut letters: BTreeSet<String> = asking.doc.pmi().datum_letters().into_iter().map(str::to_owned).collect();
    while asking.datums.len() < 3 {
        let taken: Vec<Flat> = asking.datums.iter().map(|(_, flat)| *flat).collect();
        let Some(flat) = flats
            .iter()
            .find(|flat| taken.iter().all(|datum| datum.face != flat.face && square(datum.normal, flat.normal)))
            .copied()
        else {
            return;
        };
        let letter = ('A'..='Z')
            .filter(|c| !matches!(c, 'I' | 'O' | 'Q'))
            .map(String::from)
            .find(|letter| !letters.contains(letter))
            .unwrap_or_else(|| "A".to_owned());
        letters.insert(letter.clone());
        let place = asking.datums.len();
        asking.datums.push((letter.clone(), flat));
        let Some(feature) = asking.refs(&[flat.face]) else { continue };
        let name = asking.name(&format!("Datum {letter}"));
        let item = PmiItem { id: name, kind: PmiKind::Datum { letter: letter.clone(), feature } };
        asking.ask(
            format!("datum {letter}"),
            format!("Which face is datum {letter}?"),
            reasons[place.min(2)],
            item,
            &[],
        );
    }
}

/// Record checked dimensions separately from unknown requirements.
fn holes(asking: &mut Asking, found: &Recognised) {
    let mut groups: Vec<Vec<usize>> = found.hole_patterns.clone();
    for index in 0..found.holes.len() {
        if !groups.iter().any(|group| group.contains(&index)) {
            groups.push(vec![index]);
        }
    }
    for group in groups {
        let Some(&index) = group.first() else { continue };
        let first = &found.holes[index];
        let walls: Vec<&[FaceId]> = group.iter().map(|hole| found.holes[*hole].faces.as_slice()).collect();
        let diameter = length_text(first.diameter);
        let pattern = group.len() > 1;
        let key = if pattern { format!("holes Ø{diameter}") } else { format!("hole Ø{diameter}") };
        let Some(features) = walls.iter().map(|wall| asking.refs(wall)).collect::<Option<Vec<_>>>() else { continue };
        // Each hole is answered for on its own: the holes no item of a kind covers yet,
        // as their walls. An item on one wall of a pattern leaves the others open.
        let open = |covered: &BTreeSet<FaceId>| -> Vec<Feature<FaceRef>> {
            walls
                .iter()
                .zip(&features)
                .filter(|(wall, _)| !wall.iter().any(|face| covered.contains(face)))
                .map(|(_, feature)| feature.clone())
                .collect()
        };
        if first.through {
            let members = open(&asking.sized);
            if !members.is_empty() {
                let id = question_id(&key, "size", &members);
                let item = PmiItem {
                    id: asking.name(&format!("{} diameter", capital(&key))),
                    kind: PmiKind::Size {
                        size: SizeKind::Diameter,
                        features: members,
                        tolerance: Tolerance::None,
                        through: true,
                        thread: None,
                        nominal: None,
                    },
                };
                asking.ask(
                    id,
                    format!("Record the measured diameter for {key}?"),
                    "The cylindrical walls establish the diameter, not the fit.",
                    item,
                    &[first.diameter],
                );
            }
            hole_intent(asking, &key, open(&asking.hole_intent));
        } else {
            for &index in &group {
                let hole = &found.holes[index];
                axial_hole(asking, &format!("hole Ø{diameter}"), &hole.faces, hole.diameter, false, hole.depth, None);
            }
        }
        hole_position(asking, &key, open(&asking.positioned));

        if !pattern && first.through && found.bosses.iter().any(|boss| first.opens_onto.contains(&boss.top)) {
            let members = open(&asking.finished);
            if !members.is_empty() {
                asking.unanswered(
                    question_id(&key, "finish", &members),
                    format!("What finish does {key} require?"),
                    "The geometry does not establish a required roughness or manufacturing process.",
                );
            }
        }
    }
    for hole in &found.compound_holes {
        let (label, kind, nominal) = match hole.head {
            CompoundHead::Counterbore { .. } => (
                "counterbore",
                HoleHeadKind::Counterbore,
                HoleHeadNominals::Counterbore { diameter_mm: hole.head_diameter, depth_mm: hole.head_depth },
            ),
            CompoundHead::Countersink { angle_deg } => (
                "countersink",
                HoleHeadKind::Countersink,
                HoleHeadNominals::Countersink { diameter_mm: hole.head_diameter, included_angle_deg: angle_deg },
            ),
        };
        let key = format!("{label} Ø{}", length_text(hole.pilot_diameter));
        axial_hole(
            asking,
            &key,
            &hole.pilot_faces,
            hole.pilot_diameter,
            hole.through,
            (!hole.through).then_some(hole.depth),
            Some((kind, &hole.head_faces, nominal)),
        );
        if !hole.pilot_faces.iter().any(|face| asking.positioned.contains(face))
            && let Some(pilot) = asking.refs(&hole.pilot_faces)
        {
            hole_position(asking, &key, vec![pilot]);
        }
    }
}

fn hole_intent(asking: &mut Asking, key: &str, members: Vec<Feature<FaceRef>>) {
    if !members.is_empty() {
        asking.unanswered(
            question_id(key, "fit or thread", &members),
            format!("What fit or thread, if any, is required for {key}?"),
            "A measured bore cannot identify its mating part, fit class or thread designation.",
        );
    }
}

fn hole_position(asking: &mut Asking, key: &str, members: Vec<Feature<FaceRef>>) {
    if asking.datums.len() == 3 && !members.is_empty() {
        asking.unanswered(
            question_id(key, "position", &members),
            format!("What position limits apply to {key}?"),
            "The geometry establishes the nominal location, not how far it may vary.",
        );
    }
}

fn axial_hole(
    asking: &mut Asking,
    key: &str,
    walls: &[FaceId],
    diameter: f64,
    through: bool,
    depth: Option<f64>,
    head: Option<(HoleHeadKind, &[FaceId], HoleHeadNominals)>,
) {
    let Some(pilot) = asking.refs(walls) else { return };
    let id = |aspect| question_id(key, aspect, std::slice::from_ref(&pilot));
    let same_faces = |a: &[FaceId], b: &[FaceId]| a.len() == b.len() && a.iter().all(|face| b.contains(face));
    let native = asking.native_holes.iter().find_map(|item| {
        let PmiKind::Hole { pilot, head: actual, through: actual_through, thread, .. } = item else { return None };
        let same_head = match (head, actual) {
            (None, None) => true,
            (Some((kind, faces, _)), Some(actual)) => kind == actual.kind && same_faces(faces, &actual.faces),
            _ => false,
        };
        (*actual_through == through && same_faces(pilot, walls) && same_head)
            .then(|| thread.as_ref().is_some_and(|thread| thread.depth.is_none()))
    });
    if native.is_none() {
        let asks = format!("Record the checked dimensions of {key}?");
        if !through && depth.is_none() {
            asking.unanswered(
                id("geometry"),
                format!("What drill depth is required for {key}?"),
                "The recogniser has not established the full axial drill depth.",
            );
        } else if walls.iter().any(|face| asking.sized.contains(face)) {
            asking.unanswered(
                id("geometry"),
                asks,
                "Update the existing pilot callout to include the axial geometry instead of adding a second size.",
            );
        } else {
            let measured = HoleNominals {
                pilot_diameter_mm: diameter,
                head: head.map(|(_, _, nominal)| nominal),
                drill_depth_mm: depth,
            };
            let head = match head {
                Some((kind, faces, _)) => {
                    let Some(faces) = asking.refs(faces) else { return };
                    Some(HoleHead { kind, faces })
                }
                None => None,
            };
            let item = PmiItem {
                id: asking.name(&format!("{} dimensions", capital(key))),
                kind: PmiKind::Hole {
                    pilot: pilot.clone(),
                    head,
                    tolerance: Tolerance::None,
                    through,
                    drill_depth: None,
                    thread: None,
                },
            };
            asking.questions.push(Question {
                id: id("geometry"),
                asks,
                proposes: item.hole_callout(&length_text(diameter), Some(&measured)),
                reason: "The checked walls and axial ends establish these nominal dimensions, not the tolerances."
                    .into(),
                proposal: Some(Proposal::Item { item: Box::new(item) }),
            });
        }
    }
    if !through && native == Some(true) {
        asking.unanswered(
            id("thread depth"),
            format!("What usable thread depth does {key} require?"),
            "The drill depth is not the usable thread depth.",
        );
    }
    if !walls.iter().any(|face| asking.hole_intent.contains(face)) {
        hole_intent(asking, key, vec![pilot]);
    }
}

fn ordered_pair(a: FaceId, b: FaceId) -> (FaceId, FaceId) {
    if a < b { (a, b) } else { (b, a) }
}

fn recesses(asking: &mut Asking, found: &Recognised) {
    for recess in &found.recesses {
        if recess.opens_onto.iter().any(|&top| asking.plane_distances.contains(&ordered_pair(top, recess.floor))) {
            continue;
        }
        let Some(floor) = asking.refs(&[recess.floor]) else { continue };
        let key = if recess.open_sided { "step" } else { "pocket" };
        let id = question_id(key, "depth", std::slice::from_ref(&floor));
        let Some(&top) = recess.opens_onto.first() else { continue };
        let Some(top) = asking.refs(&[top]) else { continue };
        let item = PmiItem {
            id: asking.name(&format!("{} depth", capital(key))),
            kind: PmiKind::Location {
                from: Anchor { feature: top, at: At::Feature },
                to: Anchor { feature: floor, at: At::Feature },
                tolerance: Tolerance::None,
                nominal: None,
            },
        };
        asking.ask(
            id,
            format!("Record the measured {key} depth?"),
            "The opening and floor planes establish the perpendicular depth.",
            item,
            &[recess.depth],
        );
    }
}

fn edges(asking: &mut Asking, found: &Recognised) {
    for edge in &found.edge_treatments {
        let Some(feature) = asking.refs(&[edge.face]) else { continue };
        let (key, nominal) = match edge.kind {
            EdgeTreatmentKind::Fillet { radius } => ("fillet", format!("R{}", length_text(radius))),
            EdgeTreatmentKind::Chamfer { setback } => {
                if !asking.edge_limits.contains(&edge.face)
                    && let (Some(a), Some(b)) = (asking.refs(&[edge.supports[0]]), asking.refs(&[edge.supports[1]]))
                {
                    let id = question_id("chamfer", "treatment", std::slice::from_ref(&feature));
                    let item = PmiItem {
                        id: asking.name("Chamfer"),
                        kind: PmiKind::Chamfer { bevel: feature, sides: [a, b] },
                    };
                    asking.ask(
                        id,
                        "Record the measured chamfer, or specify edge-break limits?".into(),
                        "The setback and angle come from equal setbacks on perpendicular supports; any limits must be stated.",
                        item,
                        &[setback, 45.0],
                    );
                }
                continue;
            }
        };
        let id = |aspect| question_id(key, aspect, std::slice::from_ref(&feature));
        if let EdgeTreatmentKind::Fillet { radius } = edge.kind
            && !asking.radii.contains(&edge.face)
        {
            let item = PmiItem {
                id: asking.name("Fillet radius"),
                kind: PmiKind::Size {
                    size: SizeKind::Radius,
                    features: vec![feature.clone()],
                    tolerance: Tolerance::None,
                    through: false,
                    thread: None,
                    nominal: None,
                },
            };
            asking.ask(
                id("radius"),
                "Record the measured fillet radius?".into(),
                "The tangent cylindrical band establishes the nominal radius.",
                item,
                &[radius],
            );
        }
        if !asking.edge_limits.contains(&edge.face) {
            asking.unanswered(
                id("treatment"),
                format!("What treatment or limits are required for the {key} ({nominal})?"),
                "The measured profile does not establish the manufacturing requirement.",
            );
        }
    }
}

fn capital(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// A boss: its diameter, and the height of its top from the datum it is parallel to.
fn bosses(asking: &mut Asking, found: &Recognised) {
    for boss in &found.bosses {
        let diameter = length_text(boss.diameter);
        let key = format!("boss Ø{diameter}");
        let Some(wall) = asking.refs(&boss.faces) else { continue };
        let id = |aspect: &str| question_id(&key, aspect, std::slice::from_ref(&wall));
        if !boss.faces.iter().any(|face| asking.sized.contains(face)) {
            let name = asking.name(&format!("{} diameter", capital(&key)));
            let item = PmiItem {
                id: name,
                kind: PmiKind::Size {
                    size: SizeKind::Diameter,
                    features: vec![wall.clone()],
                    tolerance: Tolerance::None,
                    through: false,
                    thread: None,
                    nominal: None,
                },
            };
            asking.ask(
                id("size"),
                format!("How large is the {key}?"),
                "A drawing needs its size. With no tolerance of its own the general tolerance covers it.",
                item,
                &[boss.diameter],
            );
        }
        let top = asking.face(boss.top).and_then(|face| face.normal);
        let datum = top.and_then(|normal| asking.datum_along(normal));
        if let Some((letter, datum)) = datum
            && !asking.located.contains(&boss.top)
            && let Some(height) = asking.distance(&datum, boss.top).filter(|height| *height > 1e-9)
            && let (Some(from), Some(to)) = (asking.refs(&[datum.face]), asking.refs(&[boss.top]))
        {
            let name = asking.name(&format!("{} height", capital(&key)));
            let item = PmiItem {
                id: name,
                kind: PmiKind::Location {
                    from: Anchor { feature: from, at: At::Feature },
                    to: Anchor { feature: to, at: At::Feature },
                    tolerance: Tolerance::None,
                    nominal: None,
                },
            };
            asking.ask(
                id("height"),
                format!("How high does the {key} stand?"),
                &format!("Its top is parallel to datum {letter}, so its height is held from there."),
                item,
                &[height],
            );
        }
    }
}

/// A slot: its size, and the distance of its middle from the datum its flats are
/// parallel to.
fn slots(asking: &mut Asking, found: &Recognised) {
    for slot in &found.slots {
        let key = format!("slot {}×{}", length_text(slot.length), length_text(slot.width));
        let (Some(low), Some(high)) = (asking.refs(&[slot.flats[0]]), asking.refs(&[slot.flats[1]])) else { continue };
        let (Some(near), Some(far)) = (asking.refs(&slot.ends[0]), asking.refs(&slot.ends[1])) else { continue };
        let id = |aspect: &str| question_id(&key, aspect, std::slice::from_ref(&low));
        if !slot.flats.iter().any(|face| asking.slotted.contains(face)) {
            let name = asking.name(&capital(&key));
            let item = PmiItem {
                id: name,
                kind: PmiKind::Slot {
                    flats: [low.clone(), high.clone()],
                    ends: [near, far],
                    tolerance: Tolerance::None,
                },
            };
            asking.ask(
                id("size"),
                format!("How large is the {key}?"),
                "A drawing needs its size. With no tolerance of its own the general tolerance covers it.",
                item,
                &[slot.length, slot.width, slot.width / 2.0],
            );
        }
        let normal = asking.face(slot.flats[0]).and_then(|face| face.normal);
        let datum = normal.and_then(|normal| asking.datum_along(normal));
        if let Some((letter, datum)) = datum
            && !slot.flats.iter().any(|face| asking.located.contains(face))
            && let (Some(a), Some(b)) = (asking.distance(&datum, slot.flats[0]), asking.distance(&datum, slot.flats[1]))
            && let Some(from) = asking.refs(&[datum.face])
        {
            let name = asking.name(&format!("{} height", capital(&key)));
            let item = PmiItem {
                id: name,
                kind: PmiKind::Location {
                    from: Anchor { feature: from, at: At::Feature },
                    to: Anchor { feature: [low.clone(), high].concat(), at: At::Middle },
                    tolerance: Tolerance::None,
                    nominal: None,
                },
            };
            asking.ask(
                id("height"),
                format!("Where is the {key} held from?"),
                &format!("Its sides are parallel to datum {letter}, so its middle is held from there."),
                item,
                &[(a + b) / 2.0],
            );
        }
    }
}
