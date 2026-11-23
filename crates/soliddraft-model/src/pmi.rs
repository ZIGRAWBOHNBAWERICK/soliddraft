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
//! changes, take it out and the item goes. It reaches a STEP file and a drawing like
//! any other.

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::pmi::{
    Anchor, At, Feature, GdtCharacteristic, PartNotes, Pmi, PmiItem, PmiKind, StepPmi, Tolerance,
};
use soliddraft_kernel_api::{FaceId, FaceRole, KernelError};

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
    /// For a position tolerance, the basic distances that locate its features from
    /// its datums.
    pub located: Vec<Located>,
    /// Why the item does not apply as it stands, for people.
    pub problem: Option<String>,
    /// The threaded hole that states the item, which the part does not store: change
    /// it through that hole.
    pub feature: Option<FeatureId>,
    /// How deep a thread goes from the face its hole starts on, for a thread that does
    /// not go through.
    pub depth: Option<f64>,
}

impl PmiRow {
    /// The item as a drawing writes it, such as `Ø8 H7` or `M6x1-6H DEEP 12`.
    pub fn callout(&self) -> String {
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
    /// walls with its thread's designation, the hole that states it, and how deep the
    /// thread goes when it does not go through. A suppressed or failed hole states none.
    pub fn thread_items(&self) -> Vec<(PmiItem<FaceRef>, FeatureId, Option<f64>)> {
        let mut taken = self.pmi.clone();
        let mut out = Vec::new();
        for feature in self.features() {
            let FeatureKind::Hole(hole) = &feature.kind else { continue };
            let Some(thread) = &hole.thread else { continue };
            if self.status(feature.id) != Some(&crate::FeatureStatus::Ok) {
                continue;
            }
            let effective_depth = hole.wizard.as_ref().map_or(hole.depth, |w| w.legacy_depth());
            let through = matches!(effective_depth, soliddraft_kernel_api::HoleDepth::Through) && thread.length.is_none();
            let depth = match (thread.length, effective_depth) {
                (Some(length), _) => Some(length),
                (None, soliddraft_kernel_api::HoleDepth::Blind { depth }) => Some(depth),
                (None, soliddraft_kernel_api::HoleDepth::Through) => None,
            };
            let count = self.hole_points(hole).map(|pts| pts.len()).unwrap_or(hole.points.len());
            let item = PmiItem {
                id: taken.free_name(&format!("{} thread", feature.name.replace('/', " "))),
                kind: PmiKind::Size {
                    size: soliddraft_kernel_api::pmi::SizeKind::Diameter,
                    features: (0..count).map(|i| vec![self.hole_wall(feature, i)]).collect(),
                    tolerance: Tolerance::None,
                    through,
                    thread: Some(thread.designation()),
                    nominal: None,
                },
            };
            taken.items.push(item.clone());
            out.push((item, feature.id, depth));
        }
        out
    }

    /// Every item with the body faces it stands on, a dimension's measured value, and
    /// what is wrong with it if it does not apply. The items the part stores come first,
    /// then those its threaded holes state.
    pub fn pmi_rows(&self) -> Vec<PmiRow> {
        let body = self.body();
        let topology = body.map(|body| self.view_of(body));
        let names = self.name_map();
        let stated = self.thread_items();
        let mut rows: Vec<PmiRow> = Vec::with_capacity(self.pmi.items.len() + stated.len());
        let items = self
            .pmi
            .items
            .iter()
            .map(|item| (item, None, None))
            .chain(stated.iter().map(|(item, feature, depth)| (item, Some(*feature), *depth)));
        for (item, feature, depth) in items {
            let mut row = PmiRow {
                item: item.clone(),
                on_body: None,
                measured: Vec::new(),
                located: Vec::new(),
                problem: None,
                feature,
                depth,
            };
            let (Some(body), Some(topology)) = (body, topology.as_ref()) else {
                row.problem = Some("The part has no body.".to_owned());
                rows.push(row);
                continue;
            };
            let topology = match topology {
                Ok(topology) => topology,
                Err(error) => {
                    row.problem = Some(format!("The part's faces could not be read: {error}"));
                    rows.push(row);
                    continue;
                }
            };
            let view = View { topology };
            let resolved = item.try_map_features(|feature| {
                let mut faces: Feature<FaceId> = Vec::new();
                for face in feature {
                    let found = reference::resolve_faces(face, &view, &names).map_err(|unresolved| {
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
            });
            match resolved {
                Err(message) => row.problem = Some(message),
                Ok(on_body) => {
                    if on_body.is_dimension() {
                        let alone = Pmi { items: vec![on_body.clone()], part: PartNotes::default() };
                        match self.kernel.pmi_nominals(body, &alone) {
                            Ok(values) => row.measured = values.into_iter().next().unwrap_or_default(),
                            Err(error) => row.problem = Some(reason(error, &item.id)),
                        }
                    }
                    row.on_body = Some(on_body);
                }
            }
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
        if let Some(body) = body {
            for row in &mut rows {
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
        }
        rows
    }

    /// The part's manufacturing information for a STEP file: the items that apply, on
    /// the body's faces, and the names of the ones left out. The material note is the
    /// part's own, or else the name of the material assigned to the part.
    pub fn pmi_on_body(&self) -> PmiOnBody {
        let mut out = PmiOnBody::default();
        let mut locating_items = Vec::new();
        for row in self.pmi_rows() {
            let problem = row.problem.clone().or_else(|| row.item.file_problem());
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
