//! A part's manufacturing information: datums, dimensions with limits, geometric
//! tolerances and surface textures on faces, and the notes of the whole part.
//!
//! Items name their faces persistently (see [`reference`](crate::reference)), so an item
//! follows its faces through edits: a bore edited from 8 to 10 mm keeps its fit and its
//! position tolerance. A dimension stores no size. Its value is measured on the body
//! each time it is asked for, so it is always the size the part has.
//!
//! An item whose face is gone, or whose faces no longer fit it (a diameter on a face
//! that became a plane), is not moved to another face and not dropped. It stays in the
//! part with its problem, and applies again when the face comes back.

use serde::{Deserialize, Serialize};
use soliddraft_kernel_api::pmi::{Feature, PartNotes, Pmi, PmiItem, PmiKind, StepPmi};
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
    /// A dimension's measured value in millimetres.
    pub nominal: Option<f64>,
    /// Why the item does not apply as it stands, for people.
    pub problem: Option<String>,
}

impl PmiRow {
    /// The item as a drawing writes it, such as `Ø8 H7`.
    pub fn callout(&self) -> String {
        self.item.callout(self.nominal)
    }

    /// True when the item applies: its faces are there and, for a dimension, measure.
    pub fn applies(&self) -> bool {
        self.problem.is_none()
    }
}

/// The part's manufacturing information as a STEP file takes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PmiOnBody {
    /// The items that apply, on the body's faces, with the part's notes.
    pub pmi: Pmi<FaceId>,
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

    /// Every item with the body faces it stands on, a dimension's measured value, and
    /// what is wrong with it if it does not apply.
    pub fn pmi_rows(&self) -> Vec<PmiRow> {
        let body = self.body();
        let topology = body.and_then(|body| self.view_of(body).ok());
        let names = self.name_map();
        let mut rows: Vec<PmiRow> = Vec::with_capacity(self.pmi.items.len());
        for item in &self.pmi.items {
            let mut row = PmiRow { item: item.clone(), on_body: None, nominal: None, problem: None };
            let (Some(body), Some(topology)) = (body, topology.as_ref()) else {
                row.problem = Some("The part has no body.".to_owned());
                rows.push(row);
                continue;
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
                            Ok(values) => row.nominal = values.into_iter().next().flatten(),
                            Err(error) => row.problem = Some(reason(error, &item.id)),
                        }
                    }
                    row.on_body = Some(on_body);
                }
            }
            rows.push(row);
        }
        // A tolerance leans on its datums: one whose datum does not apply does not either.
        let standing: Vec<String> = rows
            .iter()
            .filter(|row| row.applies())
            .filter_map(|row| match &row.item.kind {
                PmiKind::Datum { letter, .. } => Some(letter.clone()),
                _ => None,
            })
            .collect();
        for row in &mut rows {
            if let PmiKind::Tolerance { datums, .. } = &row.item.kind
                && row.problem.is_none()
                && let Some(missing) = datums.iter().find(|letter| !standing.contains(letter))
            {
                row.problem = Some(format!("Datum {missing} does not apply."));
            }
        }
        rows
    }

    /// The part's manufacturing information for a STEP file: the items that apply, on
    /// the body's faces, and the names of the ones left out. The material note is the
    /// part's own, or else the name of the material assigned to the part.
    pub fn pmi_on_body(&self) -> PmiOnBody {
        let mut out = PmiOnBody::default();
        for row in self.pmi_rows() {
            match (row.on_body, row.problem) {
                (Some(item), None) => out.pmi.items.push(item),
                (_, problem) => {
                    out.left_out.push((row.item.id, problem.unwrap_or_else(|| "It does not apply.".to_owned())));
                }
            }
        }
        out.pmi.part = self.part_notes();
        out
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

    /// The items a STEP file's manufacturing information becomes on the faces of an
    /// import feature of that file, and what cannot be placed. Face keys of the file
    /// are matched to the feature's faces by the import's own face roles; an item on a
    /// face the import did not build is left out by name. Nothing is added to the part:
    /// run the returned items through
    /// [`Command::PutPmiItem`](crate::Command::PutPmiItem).
    pub fn pmi_from_step(&self, feature: FeatureId, read: &StepPmi) -> (Vec<PmiItem<FaceRef>>, PmiImport) {
        let mut report = PmiImport::default();
        for skipped in &read.skipped {
            report.left_out.push((format!("{} {}", skipped.kind, skipped.entity), skipped.reason.clone()));
        }
        let mut items = Vec::new();
        let is_import = self.feature(feature).is_some_and(|f| matches!(f.kind, FeatureKind::ImportStep(_)));
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
                    crate::feature_of_tag(origin.tag.as_str()) == Some(feature)
                        && matches!(&origin.role, FaceRole::Imported { key: k } if k == key)
                })
            });
            let first = found.next()?;
            found.next().is_none().then_some(first.id)
        };
        let mut taken = self.pmi.clone();
        for item in &read.pmi.items {
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
            item.id = taken.free_name(&item.id);
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
