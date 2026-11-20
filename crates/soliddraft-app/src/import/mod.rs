//! Review immutable STEP or IGES source bytes before committing the imported feature.

pub mod report;

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use soliddraft_assistant::tools::schema::{array, object, optional, string};
use soliddraft_kernel_api::{
    Approval, Disposition, FaceRole, FileReplacement, FileReplacementRequest, ImportFormat, ImportOptions, KernelError,
    Proposal, ProposalSet, RepairChange, RepairRecord, ReplacedImport,
};
use soliddraft_model::{ImportStepFeature, InheritedName, sha256_hex};

use crate::document::{Command, Document, FeatureId, FeatureKind};
use crate::state::{AppState, NoteKind};

pub fn params() -> Vec<(&'static str, Value)> {
    let mut params = preview_params();
    params.push((
        "preview_token",
        string("The token returned by preview_import for this source and its repair questions."),
    ));
    params.push(("decline", optional(array(string("Gap id to decline, such as g0."), "Repairs to leave open."))));
    params.push((
        "accept",
        optional(array(
            object(&[
                ("id", string("Gap id to repair, such as g0.")),
                ("tolerance_mm", string("The allowed movement in millimetres, a plain decimal of at least the gap.")),
                ("approver", string("The person who approved this repair.")),
            ]),
            "Repairs that need approval and that the user approved.",
        )),
    ));
    params
}

pub fn preview_params() -> Vec<(&'static str, Value)> {
    vec![
        ("path", string("A STEP, IGES, STL or OBJ source file path.")),
        ("feature", optional(string("The imported feature to replace. Omit for a new import."))),
        (
            "format",
            optional(string(
                "Source format: step (default), iges or mesh. Mesh coordinates are millimetres. Use the same format for preview and import.",
            )),
        ),
    ]
}

pub fn source_format(input: &Value) -> Result<ImportFormat, String> {
    match input.get("format") {
        None => Ok(ImportFormat::Step),
        Some(Value::String(format)) if format == "step" => Ok(ImportFormat::Step),
        Some(Value::String(format)) if format == "iges" => Ok(ImportFormat::Iges),
        Some(Value::String(format)) if format == "mesh" => Ok(ImportFormat::Mesh),
        _ => Err("Choose step, iges or mesh as the import format.".into()),
    }
}

pub fn source_path(input: &Value) -> Result<std::path::PathBuf, String> {
    let path = input["path"].as_str().filter(|p| !p.trim().is_empty()).ok_or("missing import source path")?;
    Ok(path.into())
}

/// One repair the reviewer approved.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approved {
    /// The gap, such as `g0`.
    pub id: String,
    /// The allowed movement in millimetres, as a plain decimal.
    pub tolerance_mm: String,
    /// Who approved it.
    pub approver: String,
}

/// The reviewer's answers to the proposed repairs.
#[derive(Clone, Debug, Default)]
pub struct Review {
    /// Gaps to leave open. A declined gap leaves its solid unbuilt.
    pub decline: Vec<String>,
    /// Asked repairs to apply.
    pub accept: Vec<Approved>,
}

/// A changed STEP file matched to every import of the file it replaces.
#[derive(Clone, Debug)]
struct FileSwap {
    /// The imports that read the old file, in tree order, as they were when matched.
    imports: Vec<(FeatureId, ImportStepFeature)>,
    /// What each of them becomes, in the same order.
    replaced: Vec<ReplacedImport>,
    /// The edit moving each import to the new file, with the names its matched faces keep.
    edits: Vec<(FeatureId, soliddraft_model::FeatureKind)>,
    /// What the replacement leaves needing repair, by what holds it.
    lost: Vec<Lost>,
}

/// Something a file replacement leaves needing repair.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Lost {
    /// What holds it: a feature's name, an appearance or a tracked measurement's name.
    pub owner: String,
    /// What it lost, in words.
    pub what: String,
}

impl std::fmt::Display for Lost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.owner, self.what)
    }
}

#[derive(Clone, Debug)]
pub struct Preview {
    feature: ImportStepFeature,
    replacement: Option<FeatureId>,
    file: Option<FileSwap>,
    /// Why a changed single-solid file could not be matched to the old one, so its
    /// references will need repair.
    unmatched: Option<String>,
    proposals: ProposalSet,
    revision: u64,
    document: soliddraft_document::DocumentId,
    /// The dialog's allowed movement of each asked repair, by gap id.
    allowed: BTreeMap<String, String>,
    /// The dialog's approver.
    approver: String,
}

/// An import failure as the interface says it.
fn plain(error: KernelError) -> String {
    match error {
        KernelError::Unsupported(_) => "Coming soon".into(),
        KernelError::InvalidInput(message) => message,
        other => other.to_string(),
    }
}

/// Repairs the kernel can certify after an explicit tolerance approval.
fn approvable(proposal: &Proposal) -> bool {
    matches!(proposal.kind.as_str(), "seam_close" | "planar_incidence")
}

fn gap_name(proposal: &Proposal) -> String {
    let number = proposal.id.strip_prefix('g').and_then(|n| n.parse::<usize>().ok()).unwrap_or(0) + 1;
    format!("Gap {number}")
}

fn kind_name(kind: &str) -> &'static str {
    match kind {
        "seam_close" => "seam",
        "planar_incidence" => "source mismatch",
        "body_gap" => "gap between bodies",
        _ => "opening",
    }
}

/// True for a plain decimal that covers the measured gap. The kernel checks it exactly.
fn covers_gap(allowed: &str, proposal: &Proposal) -> bool {
    let reaches = |gap: f64| allowed.parse::<f64>().is_ok_and(|value| value >= gap);
    plain_mm(allowed) && proposal.distance_max_mm.parse().is_ok_and(reaches)
}

/// A plain decimal number of millimetres, such as `0.03`, as an approval is saved.
fn plain_mm(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit() || b == b'.') && text.parse::<f64>().is_ok()
}

impl Preview {
    pub fn read(doc: &Document, path: &Path) -> Result<Self, String> {
        Self::read_source(doc, path, ImportFormat::Step)
    }

    pub fn read_source(doc: &Document, path: &Path, format: ImportFormat) -> Result<Self, String> {
        let source = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::from_source(doc, &path.file_name().unwrap_or_default().to_string_lossy(), source, format)
    }

    pub fn from_bytes(doc: &Document, name: &str, source: Vec<u8>) -> Result<Self, String> {
        Self::from_source(doc, name, source, ImportFormat::Step)
    }

    pub fn from_source(doc: &Document, name: &str, source: Vec<u8>, format: ImportFormat) -> Result<Self, String> {
        let mut feature = ImportStepFeature::new(name, source);
        feature.format = format;
        let proposals =
            doc.kernel().propose_import_source(format, &feature.source, &ImportOptions::default()).map_err(plain)?;
        if proposals.source_sha256 != feature.sha256 {
            return Err("the repair preview does not match the source file".into());
        }
        let asked = proposals.proposals.iter().filter(|p| p.disposition == Disposition::Ask);
        let allowed = asked.map(|p| (p.id.clone(), p.tolerance_mm.clone())).collect();
        let approver = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
        Ok(Self {
            feature,
            replacement: None,
            file: None,
            unmatched: None,
            proposals,
            revision: doc.revision(),
            document: doc.model().id(),
            allowed,
            approver,
        })
    }

    /// Review a new source for an existing import, discarding the previous source's decisions.
    pub fn replacing(mut self, doc: &Document, id: FeatureId) -> Result<Self, String> {
        let Some(FeatureKind::ImportStep(old)) = doc.feature(id).map(|f| &f.kind) else {
            return Err("Choose an imported feature to replace.".into());
        };
        let changed = old.sha256 != self.feature.sha256 || old.format != self.feature.format;
        if changed && old.format == ImportFormat::Step && self.feature.format == ImportFormat::Step {
            match match_file(doc, old, &self.feature) {
                Ok(swap) => {
                    self.file = Some(swap);
                    self.replacement = Some(id);
                    return Ok(self);
                }
                Err(error) if old.source_solid.is_some() => return Err(error),
                // A single-solid import still takes the file, its references left for repair.
                Err(error) => self.unmatched = Some(error.trim_end_matches(" Nothing was changed.").to_owned()),
            }
        }
        if old.source_solid.is_some() {
            if changed {
                return Err("This body selects a solid from the original file. Import a changed file as new bodies instead of replacing it.".into());
            }
            self.feature.source_solid = old.source_solid.clone();
        }
        if !changed {
            self.feature.source_bound_names = old.source_bound_names;
            self.feature.face_names = old.face_names.clone();
            self.feature.new_ordinals_from = old.new_ordinals_from;
        }
        self.replacement = Some(id);
        Ok(self)
    }

    pub fn token(&self) -> String {
        let reviewed = serde_json::to_vec(&(
            "soliddraft-import-review/1",
            &self.feature.source_name,
            &self.feature.sha256,
            &self.proposals,
        ))
        .expect("import previews serialize");
        if self.replacement.is_some() || self.feature.format != ImportFormat::Step {
            // A replacement is bound to the part it was reviewed against: its document and
            // revision, and for a whole file the imports it matched and how they matched. A
            // review rebuilt after any change gives another token.
            let target = self.replacement.map(|_| (&self.document, self.revision));
            let file = self.file.as_ref().map(|swap| {
                let imports: Vec<_> = swap
                    .imports
                    .iter()
                    .map(|(id, import)| (id, &import.sha256, &import.source_solid, &import.decisions))
                    .collect();
                (imports, &swap.replaced)
            });
            return sha256_hex(
                &serde_json::to_vec(&(&self.feature.format, self.replacement, &reviewed, target, file))
                    .expect("review serializes"),
            );
        }
        sha256_hex(&reviewed)
    }

    /// What accepting this whole-file replacement leaves needing repair: each stored
    /// reference to a face the new file has no verified partner for, and each feature
    /// that stops building. Empty for anything else.
    pub fn lost(&self) -> &[Lost] {
        self.file.as_ref().map_or(&[], |swap| swap.lost.as_slice())
    }

    pub fn summary(&self) -> Value {
        let mut summary = json!({"source_name": self.feature.source_name, "source_sha256": self.feature.sha256,
               "proposals": self.proposals.proposals, "replacement": self.replacement, "token": self.token()});
        if self.file.is_some() {
            summary["needs_repair"] = json!(self.lost());
        }
        if let Some(reason) = &self.unmatched {
            summary["unmatched"] = json!(reason);
        }
        summary
    }
}

/// Matches a changed STEP file to every import of the file it replaces, by complete face
/// definitions. Each face of the new file with a verified partner keeps that partner's
/// name, so every reference to it holds, in this part and in drawings and assemblies of
/// it. References to a face without a partner are listed by name and left for repair.
/// A selected solid without one verified partner refuses the replacement, so the part
/// never changes.
fn match_file(doc: &Document, old: &ImportStepFeature, new: &ImportStepFeature) -> Result<FileSwap, String> {
    use crate::document::{FeatureStatus, model_id};
    use std::collections::HashSet;
    let imports: Vec<(FeatureId, ImportStepFeature)> = doc
        .features
        .iter()
        .filter_map(|feature| match &feature.kind {
            FeatureKind::ImportStep(import) if import.sha256 == old.sha256 && import.format == old.format => {
                Some((feature.id, import.clone()))
            }
            _ => None,
        })
        .collect();
    // The transaction matches the files without repair answers, so imports whose old file
    // needed them cannot be matched faithfully yet.
    if imports.iter().any(|(_, import)| !import.decisions.is_empty()) {
        return Err("This file's imports carry repair answers, which a whole-file replacement cannot carry yet. Nothing was changed.".into());
    }
    let solids: Vec<Option<String>> = imports.iter().map(|(_, import)| import.source_solid.clone()).collect();
    let request = FileReplacementRequest {
        old_source: &old.source,
        old_sha256: &old.sha256,
        new_source: &new.source,
        new_sha256: &new.sha256,
        new_name: &new.source_name,
        source_solids: &solids,
    };
    let replaced = match doc.kernel().replace_import_file(&request).map_err(plain)? {
        FileReplacement::Verified { imports: replaced, .. } if replaced.len() == imports.len() => replaced,
        FileReplacement::Verified { .. } => {
            return Err("The new file was matched to a different set of imports.".into());
        }
        FileReplacement::Unresolved { reasons } => {
            return Err(format!(
                "The new file cannot replace this one without choosing its solids again. {} Nothing was changed.",
                reasons.join(" ")
            ));
        }
    };
    type Name = (soliddraft_model::FeatureId, soliddraft_model::Generator, u32);
    // Each import's current face names by face key, its exact pairs from Catallax, and the
    // first number free for a new face: past every face this import has had, so none
    // takes the number of a face a reference still names.
    let mut names: Vec<BTreeMap<String, InheritedName>> = Vec::new();
    let mut pairs: Vec<BTreeMap<String, String>> = Vec::new();
    let mut firsts: Vec<u32> = Vec::new();
    for ((id, matched), replaced) in imports.iter().zip(&replaced) {
        let faces = doc.model().generated_faces(model_id(*id));
        if faces.is_empty() {
            let name = doc.feature(*id).map_or("An import of this file", |f| f.name.as_str());
            return Err(format!("{name} is not built, so its faces cannot be matched. Nothing was changed."));
        }
        let mut first = matched.new_ordinals_from;
        let mut current = BTreeMap::new();
        for face in faces {
            first = first.max(face.ordinal + 1);
            if let FaceRole::Imported { key } = &face.origin.role {
                current.insert(key.clone(), InheritedName { generator: face.generator.clone(), ordinal: face.ordinal });
            }
        }
        pairs.push(replaced.faces.iter().filter(|(old, _)| current.contains_key(*old)).map(clone_pair).collect());
        names.push(current);
        firsts.push(first);
    }
    let edits_for = |pairs: &[BTreeMap<String, String>]| -> Vec<(FeatureId, soliddraft_model::FeatureKind)> {
        imports
            .iter()
            .zip(&replaced)
            .enumerate()
            .map(|(index, ((id, _), replaced))| {
                let mut import = new.clone();
                import.source_solid = replaced.source_solid.clone();
                import.source_bound_names = true;
                import.face_names =
                    pairs[index].iter().map(|(old, new)| (new.clone(), names[index][old].clone())).collect();
                import.new_ordinals_from = firsts[index];
                (*id, soliddraft_model::FeatureKind::ImportStep(import))
            })
            .collect()
    };
    // Faces Catallax could not pair exactly, because they moved or changed size, follow by
    // shape: the new file's faces are read from a trial build with the exact names.
    let mut trial = doc.scratch();
    trial.apply(Command::ReplaceImportFile(edits_for(&pairs)))?;
    for (index, (id, _)) in imports.iter().enumerate() {
        let body = |doc: &Document| doc.feature(*id).and_then(|f| f.body);
        let (Some(before), Some(after)) = (body(doc), body(&trial)) else { continue };
        let before = shapes(doc.kernel().as_ref(), before);
        let followed = follow(&before, &shapes(trial.kernel().as_ref(), after), &pairs[index]);
        pairs[index].extend(followed);
    }
    let edits = edits_for(&pairs);
    let mut unmatched: HashSet<Name> = HashSet::new();
    for (index, (id, _)) in imports.iter().enumerate() {
        for (key, name) in &names[index] {
            if !pairs[index].contains_key(key) {
                unmatched.insert((model_id(*id), name.generator.clone(), name.ordinal));
            }
        }
    }
    let mut lost: Vec<Lost> = Vec::new();
    for (owner, face) in doc.model().stored_face_refs() {
        if unmatched.contains(&(face.feature, face.generator.clone(), face.ordinal)) {
            // Named as the part names it now: a reference's own label may be older, and an
            // edge reference keeps none for its faces.
            let label = doc
                .model()
                .name_label(face.feature, face.generator.clone(), face.ordinal)
                .map_or_else(|| face.label.clone(), |label| label.text);
            let item = Lost { owner, what: format!("{label} has no matching face in the new file") };
            if !lost.contains(&item) {
                lost.push(item);
            }
        }
    }
    let mut trial = doc.scratch();
    trial.apply(Command::ReplaceImportFile(edits.clone()))?;
    for (id, _) in &imports {
        let mut seen = HashSet::new();
        if !trial.model().generated_faces(model_id(*id)).iter().all(|face| seen.insert((&face.generator, face.ordinal)))
        {
            return Err("The new file's faces cannot all be given distinct names. Nothing was changed.".into());
        }
    }
    // A feature that built before and does not now is also listed, unless its lost
    // references already are; suppressed, rolled-back or already broken features keep
    // their state and are not.
    let built = |feature: &crate::document::Feature| feature.status == FeatureStatus::Ok;
    for after in trial.features.iter().filter(|after| !built(after) && doc.feature(after.id).is_some_and(built)) {
        if lost.iter().any(|item| item.owner == after.name) {
            continue;
        }
        let what = match &after.status {
            FeatureStatus::Error(message) => format!("does not build with the new file: {message}"),
            FeatureStatus::NeedsRepair(items) => {
                let labels: Vec<&str> = items.iter().map(|item| item.label.as_str()).collect();
                format!("needs repair: {}", labels.join(", "))
            }
            _ => "does not build with the new file".into(),
        };
        lost.push(Lost { owner: after.name.clone(), what });
    }
    Ok(FileSwap { imports, replaced, edits, lost })
}

fn clone_pair((old, new): (&String, &String)) -> (String, String) {
    (old.clone(), new.clone())
}

/// A cylinder's or sphere's radius, and whether its material side looks away from the
/// axis or centre.
type Size = (f64, Option<bool>);

/// What the shape matching reads of one imported face, by its key in the source file.
struct Shape {
    surface: soliddraft_kernel_api::SurfaceKind,
    area: f64,
    centroid: soliddraft_kernel_api::Vec3,
    /// A planar face's outward normal, or a cylinder's axis direction up to sign.
    direction: Option<soliddraft_kernel_api::Vec3>,
    /// A cylinder's or sphere's radius and whether its material side looks away from the
    /// axis or centre, read from the kernel's definition.
    size: Option<Size>,
    neighbours: std::collections::BTreeSet<String>,
    bbox: soliddraft_kernel_api::Aabb,
}

/// The middle of an enclosure, in millimetres.
pub(crate) fn middle(bounds: &soliddraft_kernel_api::SupportBounds) -> Option<f64> {
    let value = |r: &soliddraft_kernel_api::ExactRational| {
        Some(r.numerator.parse::<f64>().ok()? / r.denominator.parse::<f64>().ok()?)
    };
    Some((value(&bounds.lo)? + value(&bounds.hi)?) / 2.0)
}

/// A curved face's axis direction and size, from the kernel's definition.
fn support(
    kernel: &dyn soliddraft_kernel_api::Kernel,
    body: soliddraft_kernel_api::BodyId,
    face: soliddraft_kernel_api::FaceId,
) -> Option<(Option<soliddraft_kernel_api::Vec3>, Size)> {
    use soliddraft_kernel_api::{EnclosedSupport, SupportEntity, Vec3};
    match kernel.entity_support_enclosed(body, SupportEntity::Face(face)).ok()? {
        EnclosedSupport::Axis { direction, radius_squared: Some(squared), outward, .. } => {
            let [x, y, z] = [middle(&direction[0])?, middle(&direction[1])?, middle(&direction[2])?];
            Some((Vec3::new(x, y, z).normalized(), (middle(&squared)?.sqrt(), outward)))
        }
        EnclosedSupport::Point { radius_squared: Some(squared), outward, .. } => {
            Some((None, (middle(&squared)?.sqrt(), outward)))
        }
        _ => None,
    }
}

/// The imported faces of a body by source key that can follow by shape: planes, and
/// cylinders and spheres whose radius the kernel reads. Pieces of one source face are
/// taken together.
fn shapes(kernel: &dyn soliddraft_kernel_api::Kernel, body: soliddraft_kernel_api::BodyId) -> BTreeMap<String, Shape> {
    use soliddraft_kernel_api::SurfaceKind;
    let Ok(topology) = kernel.topology(body) else { return BTreeMap::new() };
    let key = |face: soliddraft_kernel_api::FaceId| {
        topology.face(face)?.origins.iter().find_map(|origin| match &origin.role {
            FaceRole::Imported { key } => Some(key.clone()),
            _ => None,
        })
    };
    let mut out: BTreeMap<String, Shape> = BTreeMap::new();
    for face in &topology.faces {
        let Some(k) = key(face.id) else { continue };
        if !out.contains_key(&k) {
            let (direction, size) = match face.surface {
                SurfaceKind::Plane => (face.normal, None),
                SurfaceKind::Cylinder | SurfaceKind::Sphere => match support(kernel, body, face.id) {
                    Some((direction, size)) => (direction, Some(size)),
                    None => continue,
                },
                _ => continue,
            };
            let shape = Shape {
                surface: face.surface,
                area: 0.0,
                centroid: soliddraft_kernel_api::Vec3::ZERO,
                direction,
                size,
                neighbours: Default::default(),
                bbox: face.bbox,
            };
            out.insert(k.clone(), shape);
        }
        let shape = out.get_mut(&k).expect("inserted");
        let area = shape.area + face.area;
        if area > 0.0 {
            shape.centroid = (shape.centroid * shape.area + face.centroid * face.area) * (1.0 / area);
        }
        shape.area = area;
        shape.bbox = joined(shape.bbox, face.bbox);
    }
    for edge in &topology.edges {
        if let [a, b] = edge.faces[..]
            && let (Some(a), Some(b)) = (key(a), key(b))
            && a != b
        {
            out.get_mut(&a).map(|shape| shape.neighbours.insert(b.clone()));
            out.get_mut(&b).map(|shape| shape.neighbours.insert(a));
        }
    }
    out
}

fn joined(a: soliddraft_kernel_api::Aabb, b: soliddraft_kernel_api::Aabb) -> soliddraft_kernel_api::Aabb {
    use soliddraft_kernel_api::Vec3;
    soliddraft_kernel_api::Aabb {
        min: Vec3::new(a.min.x.min(b.min.x), a.min.y.min(b.min.y), a.min.z.min(b.min.z)),
        max: Vec3::new(a.max.x.max(b.max.x), a.max.y.max(b.max.y), a.max.z.max(b.max.z)),
    }
}

/// How far a face may move and still follow: a quarter of the old body's diagonal.
const FOLLOW_REACH: f64 = 0.25;
/// How far a face may turn and still follow, in degrees.
const FOLLOW_TURN_DEGREES: f64 = 1.0;
/// How much a radius may differ and still follow, as a share of it.
const FOLLOW_RADIUS_SHARE: f64 = 1e-6;

/// True when `new` could be `old` moved: the same surface type; a plane facing the same
/// way, or a cylinder's axis parallel, within [`FOLLOW_TURN_DEGREES`]; the same radius
/// and material side; and the centroid within `reach`.
fn same_shape(old: &Shape, new: &Shape, reach: f64) -> bool {
    let turn = FOLLOW_TURN_DEGREES.to_radians().cos();
    let facing = match (old.direction, new.direction) {
        (Some(a), Some(b)) if old.surface == soliddraft_kernel_api::SurfaceKind::Plane => a.dot(b) >= turn,
        (Some(a), Some(b)) => a.dot(b).abs() >= turn,
        (None, None) => true,
        _ => false,
    };
    let sized = match (old.size, new.size) {
        (Some((a, side_a)), Some((b, side_b))) => side_a == side_b && (a - b).abs() <= a.abs() * FOLLOW_RADIUS_SHARE,
        (None, None) => true,
        _ => false,
    };
    old.surface == new.surface && facing && sized && old.centroid.distance(new.centroid) <= reach
}

/// The one candidate, or the nearest when it is less than half as far as the next.
fn clear_nearest(mut fits: Vec<(f64, &String)>) -> Option<&String> {
    fits.sort_by(|a, b| a.0.total_cmp(&b.0));
    match fits.as_slice() {
        [only] => Some(only.1),
        [best, next, ..] if best.0 * 2.0 < next.0 => Some(best.1),
        _ => None,
    }
}

/// Pairs old faces the exact match left over with new faces of the same shape (see
/// [`same_shape`]) whose already paired neighbours are still neighbours. A pair is made
/// only when each face is the other's clear nearest among every face eligible for it,
/// so two holes that could swap stay unpaired and their references go to repair. Pairs
/// made in one round constrain the next, until none is added.
fn follow(
    old: &BTreeMap<String, Shape>,
    new: &BTreeMap<String, Shape>,
    exact: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let Some(bbox) = old.values().map(|shape| shape.bbox).reduce(joined) else {
        return BTreeMap::new();
    };
    let reach = bbox.diagonal() * FOLLOW_REACH;
    let mut pairs = exact.clone();
    loop {
        let taken: std::collections::BTreeSet<&String> = pairs.values().collect();
        let open_old: Vec<(&String, &Shape)> = old.iter().filter(|(key, _)| !pairs.contains_key(*key)).collect();
        let open_new: Vec<(&String, &Shape)> = new.iter().filter(|(key, _)| !taken.contains(key)).collect();
        let eligible = |(_, a): &(&String, &Shape), (_, b): &(&String, &Shape)| {
            same_shape(a, b, reach)
                && a.neighbours.iter().filter_map(|n| pairs.get(n)).all(|n| b.neighbours.contains(n))
        };
        let mut accepted = Vec::new();
        for o in &open_old {
            let forward =
                open_new.iter().filter(|n| eligible(o, n)).map(|n| (o.1.centroid.distance(n.1.centroid), n.0));
            let Some(target) = clear_nearest(forward.collect()) else { continue };
            let n = open_new.iter().find(|n| n.0 == target).expect("a candidate");
            let reverse =
                open_old.iter().filter(|p| eligible(p, n)).map(|p| (p.1.centroid.distance(n.1.centroid), p.0));
            if clear_nearest(reverse.collect()) == Some(o.0) {
                accepted.push((o.0.clone(), target.clone()));
            }
        }
        if accepted.is_empty() {
            break;
        }
        pairs.extend(accepted);
    }
    pairs.into_iter().filter(|(key, _)| !exact.contains_key(key)).collect()
}

/// Moves every import of the old file to the reviewed new file, its matched faces keeping
/// their names, as one undo step. References listed as lost are left for repair.
fn commit_file(doc: &mut Document, swap: &FileSwap, reviewed: &ImportStepFeature) -> Result<FeatureId, String> {
    let stale = || "the part changed during the preview; review the replacement again".to_owned();
    if !reviewed.decisions.is_empty() {
        return Err(
            "The new file needs repair answers, which a whole-file replacement cannot carry yet. Nothing was changed."
                .into(),
        );
    }
    for (id, matched) in &swap.imports {
        match doc.feature(*id).map(|feature| &feature.kind) {
            Some(FeatureKind::ImportStep(now)) if now == matched => {}
            _ => return Err(stale()),
        }
    }
    doc.apply(Command::ReplaceImportFile(swap.edits.clone()))?;
    Ok(swap.imports[0].0)
}

/// Why an import was not committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitError {
    /// The review is incomplete or out of date. Answering it again can succeed.
    Review(String),
    /// The file, or the body it makes, was refused. The import report keeps it.
    Refused(String),
}

impl CommitError {
    pub fn message(&self) -> &str {
        match self {
            CommitError::Review(message) | CommitError::Refused(message) => message,
        }
    }
}

impl From<CommitError> for String {
    fn from(error: CommitError) -> String {
        match error {
            CommitError::Review(message) | CommitError::Refused(message) => message,
        }
    }
}

pub fn commit(doc: &mut Document, preview: &Preview, review: &Review) -> Result<FeatureId, String> {
    commit_reviewed(doc, preview, review).map_err(String::from)
}

/// Commits a reviewed import, telling a review to answer again from a refused file.
pub fn commit_reviewed(doc: &mut Document, preview: &Preview, review: &Review) -> Result<FeatureId, CommitError> {
    let feature = answered(doc, preview, review).map_err(CommitError::Review)?;
    admit(doc, preview, feature).map(|(id, _)| id).map_err(CommitError::Refused)
}

/// Commits an import from the dialog. A refused file becomes a row of the import report,
/// and a file that imports clears its earlier refusal.
pub fn commit_in(state: &mut AppState, preview: &Preview, review: &Review) -> Result<FeatureId, CommitError> {
    let file = preview.feature.source_name.clone();
    let format = preview.feature.format;
    let result = answered(&state.doc, preview, review)
        .map_err(CommitError::Review)
        .and_then(|feature| admit(&mut state.doc, preview, feature).map_err(CommitError::Refused));
    match &result {
        Ok((_, refusals)) => {
            state.import_refusals.retain(|r| r.file != file || r.format != format);
            if !refusals.is_empty() {
                report::keep_refusal(state, file, format, refusals.join(" "));
            }
        }
        Err(CommitError::Refused(reason)) => report::keep_refusal(state, file, format, reason.clone()),
        Err(CommitError::Review(_)) => {}
    }
    result.map(|(id, _)| id)
}

/// The source feature with the review's answers, or why the review must be answered again.
fn answered(doc: &Document, preview: &Preview, review: &Review) -> Result<ImportStepFeature, String> {
    if doc.revision() != preview.revision || doc.model().id() != preview.document || doc.proof_pending() {
        return Err("the part changed during the preview; review the import again".into());
    }
    let find = |id: &String| {
        let proposals = &preview.proposals.proposals;
        proposals.iter().find(|p| &p.id == id).ok_or_else(|| format!("no repair named {id} in this file"))
    };
    let mut feature = preview.feature.clone();
    for id in &review.decline {
        feature = feature.declined(&find(id)?.question);
    }
    for approved in &review.accept {
        let proposal = find(&approved.id)?;
        let approver = approved.approver.trim();
        if proposal.disposition != Disposition::Ask {
            return Err(format!("{} is repaired automatically and takes no approval", approved.id));
        }
        if review.decline.contains(&approved.id) {
            return Err(format!("{} is both accepted and declined", approved.id));
        }
        if !approvable(proposal) {
            return Err("Coming soon".into());
        }
        if approver.is_empty() {
            return Err("an approved repair needs the name of who approved it".into());
        }
        if !plain_mm(approved.tolerance_mm.trim()) {
            return Err("The allowed movement must be a number of millimetres, such as 0.03.".into());
        }
        feature = feature.accepted(&proposal.question, approved.tolerance_mm.trim(), approver);
    }
    let answered = |p: &Proposal| review.decline.contains(&p.id) || review.accept.iter().any(|a| a.id == p.id);
    if preview.proposals.proposals.iter().any(|p| p.disposition == Disposition::Ask && !answered(p)) {
        return Err("this repair needs an explicit tolerance approval before it can be imported".into());
    }
    Ok(feature)
}

/// Builds the answered import on a scratch copy of the part and commits it, or says why
/// the file or its body was refused.
fn admit(
    doc: &mut Document,
    preview: &Preview,
    feature: ImportStepFeature,
) -> Result<(FeatureId, Vec<String>), String> {
    if let Some(swap) = &preview.file {
        // Answers cannot go through a match yet. A single-solid import takes the answered
        // file as a plain replacement instead, its references left for repair.
        let single = swap.imports.iter().all(|(_, import)| import.source_solid.is_none());
        if feature.decisions.is_empty() || !single {
            return commit_file(doc, swap, &feature).map(|id| (id, Vec::new()));
        }
    }
    let mut command = match preview.replacement {
        Some(id) => Command::EditFeature { id, kind: FeatureKind::ImportStep(feature) },
        None => {
            let solids = if feature.format == ImportFormat::Mesh {
                Vec::new()
            } else {
                doc.kernel().source_solids(feature.format, &feature.source).map_err(plain)?
            };
            if solids.len() > 1 {
                Command::ImportBodies(
                    solids
                        .into_iter()
                        .map(|entity| {
                            let mut selected = feature.clone();
                            selected.source_solid = Some(entity);
                            selected
                        })
                        .collect(),
                )
            } else {
                Command::AddFeature(FeatureKind::ImportStep(feature))
            }
        }
    };
    // Keep independently admitted STEP bodies; replacements and explicit repair reviews
    // still validate their entire transaction before changing the part.
    let mut trial = doc.scratch();
    let mut trial_result = trial.apply(command.clone());
    let mut refusals = Vec::new();
    if let Command::ImportBodies(imports) = &command
        && let Some(failed) = trial
            .features
            .iter()
            .filter(|f| doc.feature(f.id).is_none())
            .find(|f| f.status != crate::document::FeatureStatus::Ok)
    {
        use crate::document::FeatureStatus;
        let added = || trial.features.iter().filter(|f| doc.feature(f.id).is_none());
        let partial = preview.feature.format == ImportFormat::Step
            && preview.feature.decisions.is_empty()
            && added().count() == imports.len()
            && added().all(|f| matches!(f.status, FeatureStatus::Ok | FeatureStatus::Error(_)))
            && added().any(|f| f.status == FeatureStatus::Ok);
        if !partial {
            return Err(match &failed.status {
                FeatureStatus::Error(message) => message.clone(),
                _ => "The imported bodies could not be built. Restore the full feature tree and review the file again."
                    .into(),
            });
        }
        let mut admitted = Vec::new();
        for (index, (import, built)) in imports.iter().zip(added()).enumerate() {
            match &built.status {
                FeatureStatus::Ok => admitted.push(import.clone()),
                FeatureStatus::Error(reason) => refusals.push(format!("Body {}: {reason}", index + 1)),
                _ => unreachable!("partial admission requires completed builds"),
            }
        }
        command = Command::ImportBodies(admitted);
        let mut accepted = doc.scratch();
        accepted.reuse_builds_from(trial.model());
        trial_result = accepted.apply(command.clone());
        trial = accepted;
    }
    let imported = match preview.replacement {
        Some(id) => trial.feature(id),
        None => trial.features.last(),
    }
    .ok_or("the import made no feature")?;
    if preview.replacement.is_some() && imported.status != crate::document::FeatureStatus::Ok {
        return Err(match &imported.status {
            crate::document::FeatureStatus::Error(message) => message.clone(),
            _ => "The replacement could not be built. Restore the full feature tree and review it again.".into(),
        });
    }
    let record = trial.model().import_record(soliddraft_model::FeatureId(imported.id as u32));
    let declined = trial_result.is_err()
        && record.is_some_and(|r| {
            !r.changes.is_empty()
                && r.derivative.is_none()
                && r.changes.iter().all(|c| c.approval == Approval::Declined)
        });
    if !declined {
        trial_result?;
    }
    let declined_record = declined.then(|| record.cloned()).flatten();
    doc.reuse_builds_from(trial.model());
    let before: Vec<FeatureId> = doc.features.iter().map(|f| f.id).collect();
    let id = match doc.apply(command) {
        Ok(Some(id)) => Ok(id),
        Err(error) => {
            let imported = doc.features.last().ok_or("the import made no feature")?;
            let record = doc.model().import_record(soliddraft_model::FeatureId(imported.id as u32));
            if declined && record == declined_record.as_ref() { Ok(imported.id) } else { Err(error) }
        }
        Ok(None) => preview.replacement.ok_or_else(|| "the import made no feature".into()),
    }?;
    // A new import brings the file's manufacturing information with it. A replaced
    // file keeps the part's own: its items follow the faces the replacement matched.
    if preview.replacement.is_none() {
        let added: Vec<FeatureId> = doc.features.iter().map(|f| f.id).filter(|id| !before.contains(id)).collect();
        crate::pmi::after_import(doc, &added);
    }
    Ok((id, refusals))
}

/// The repair record as rows of the feature's properties: how each gap was answered and the
/// limit it was held to, how far its vertices moved, and how far each repaired face lies from
/// the source. Output error belongs to an export, and later features are not covered.
pub fn record_rows(record: &RepairRecord) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if record.tolerant.is_some() {
        rows.push((
            "Admission".into(),
            if record.derivative.is_some() {
                "Certified solid. Source admission evidence is recorded.".into()
            } else {
                crate::document::TOLERANT_WORDS.into()
            },
        ));
    }
    for (index, change) in record.changes.iter().enumerate() {
        let gap = format!("Gap {}", index + 1);
        rows.push((gap.clone(), answer_text(change)));
        if change.approval != Approval::Declined {
            rows.push((format!("{gap} approved limit"), format!("{} mm", change.tolerance_mm)));
            rows.push((format!("{gap} vertex movement"), format!("{} mm", change.max_movement_mm)));
        }
    }
    for face in &record.face_tolerances {
        rows.push((format!("Face {} from source", face.face), format!("up to {} mm", face.deviation_upper_mm)));
    }
    for face in &record.source_incidence {
        rows.push((
            format!("Face {} source discrepancy", face.face),
            format!("up to {} mm", face.source_discrepancy_upper_mm),
        ));
        rows.push((
            format!("Face {} shape deviation", face.face),
            format!("up to {} mm", face.geometric_deviation_upper_mm),
        ));
        rows.push((
            format!("Face {} source to result", face.face),
            format!("up to {} mm", face.source_to_result_upper_mm),
        ));
    }
    if !record.changes.is_empty() || !record.source_incidence.is_empty() || !record.face_tolerances.is_empty() {
        rows.push(("Repair figures".into(), "Cover the import only. Each export reports its own error.".into()));
    }
    rows
}

fn answer_text(change: &RepairChange) -> String {
    let kind = kind_name(&change.kind);
    match change.approval {
        Approval::Automatic => format!("Repaired {kind} automatically"),
        Approval::User => {
            format!("Repaired {kind}, approved by {}", change.approver.as_deref().unwrap_or("the reviewer"))
        }
        Approval::Declined => format!("Declined {kind}; the solid stays open"),
    }
}

fn proposal_group(ui: &mut egui::Ui, proposal: &Proposal, allowed: &mut BTreeMap<String, String>) {
    ui.label(format!("{}: {}", gap_name(proposal), kind_name(&proposal.kind)));
    ui.label(if proposal.distance_min_mm == proposal.distance_max_mm {
        format!("Measured {} mm", proposal.distance_max_mm)
    } else {
        format!("Measured {} to {} mm", proposal.distance_min_mm, proposal.distance_max_mm)
    });
    match (proposal.disposition, allowed.get_mut(&proposal.id)) {
        (Disposition::Ask, Some(_)) if !approvable(proposal) => {
            ui.label("Coming soon");
        }
        (Disposition::Ask, Some(value)) => {
            ui.horizontal(|ui| {
                let label = ui.label("Allowed movement");
                ui.add(egui::TextEdit::singleline(value).desired_width(90.0)).labelled_by(label.id);
                ui.label("mm");
            });
            if !covers_gap(value.trim(), proposal) {
                ui.label(format!("Enter at least {} mm", proposal.distance_max_mm));
            }
        }
        _ => {
            ui.label(format!("Repaired automatically, within {} mm", proposal.tolerance_mm));
        }
    }
    let mut reason = proposal.reason.chars();
    ui.label(reason.next().map_or(String::new(), |first| first.to_uppercase().chain(reason).collect()));
    ui.label(format!("File entities {}", proposal.entities.join(", ")));
}

/// What the status bar says after a new import that did not open as an exact solid.
fn opened_note(doc: &Document, id: FeatureId, format: ImportFormat) -> Option<String> {
    let FeatureKind::ImportStep(import) = &doc.feature(id)?.kind else { return None };
    let row = report::row(doc, id, import.source_name.clone(), format);
    (row.tier != report::Tier::Exact).then(|| format!("{} opened as: {}. {}", row.file, row.tier.label(), row.why))
}

pub fn window(ctx: &egui::Context, state: &mut AppState) {
    let Some(mut preview) = state.import_dialog.take() else { return };
    let mut open = true;
    let mut choice = None;
    let mut cancelled = false;
    let format = preview.feature.format;
    let title = if preview.replacement.is_some() {
        "Replace imported file".to_owned()
    } else {
        format!("Import {}", format.label())
    };
    egui::Window::new(title).collapsible(false).open(&mut open).show(ctx, |ui| {
        ui.label(&preview.feature.source_name);
        ui.label("Review repairs before importing. The original file stays unchanged.");
        if format == ImportFormat::Mesh {
            ui.label("STL and OBJ coordinates are read in millimetres. The mesh must form one closed solid, up to 200,000 triangles.");
        }
        if let Some(swap) = &preview.file {
            ui.label("Accept keeps the names of faces that match the old file, so references to them follow. You can undo the replacement.");
            if !swap.lost.is_empty() {
                ui.label("These will need repair:");
                for item in &swap.lost {
                    ui.label(item.to_string());
                }
            }
        } else if preview.replacement.is_some() {
            if let Some(reason) = &preview.unmatched {
                ui.label(format!("The new file's faces were not matched to the old file's. {reason}"));
            }
            ui.label("References to a changed file will need repair. You can undo the replacement.");
        }
        let Preview { proposals, allowed, approver, .. } = &mut preview;
        let asked = || proposals.proposals.iter().filter(|p| p.disposition == Disposition::Ask);
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for proposal in &proposals.proposals {
                ui.group(|ui| proposal_group(ui, proposal, allowed));
            }
            if proposals.proposals.is_empty() {
                ui.label("No repairs proposed.");
            }
        });
        if asked().any(approvable) {
            ui.horizontal(|ui| {
                let label = ui.label("Approved by");
                ui.text_edit_singleline(approver).labelled_by(label.id);
            });
        }
        let ready =
            asked().all(|p| approvable(p) && !approver.trim().is_empty() && covers_gap(allowed[&p.id].trim(), p));
        ui.horizontal(|ui| {
            if ui.add_enabled(ready, egui::Button::new("Accept")).clicked() {
                choice = Some(false);
            }
            if ui.add_enabled(!proposals.proposals.is_empty(), egui::Button::new("Decline")).clicked() {
                choice = Some(true);
            }
            if ui.button("Cancel").clicked() {
                cancelled = true;
            }
        });
    });
    if let Some(decline) = choice {
        let proposals = &preview.proposals.proposals;
        let review = if decline {
            Review { decline: proposals.iter().map(|p| p.id.clone()).collect(), accept: Vec::new() }
        } else {
            let accept = preview.allowed.iter().map(|(id, tolerance_mm)| Approved {
                id: id.clone(),
                tolerance_mm: tolerance_mm.clone(),
                approver: preview.approver.clone(),
            });
            Review { decline: Vec::new(), accept: accept.collect() }
        };
        let committed = commit_in(state, &preview, &review);
        if committed.is_ok() && preview.replacement.is_none() {
            // A new import is framed, however small it is.
            state.push(crate::state::AppAction::ZoomFit);
        }
        match committed {
            Ok(id)
                if preview.replacement.is_none()
                    && let Some(note) = opened_note(&state.doc, id, format) =>
            {
                state.note_import(note, NoteKind::Info)
            }
            Ok(_) => state.note(
                if decline {
                    "Import saved with declined repairs; the solid stays open".into()
                } else if !preview.lost().is_empty() {
                    let count = preview.lost().len();
                    format!(
                        "Replaced imported file. {count} {} repair.",
                        if count == 1 { "item needs" } else { "items need" }
                    )
                } else if preview.file.is_some() {
                    "Replaced imported file. Every reference follows.".into()
                } else if preview.replacement.is_some() {
                    "Replaced imported file. Check any references marked for repair.".into()
                } else {
                    format!("Imported {} body", format.label())
                },
                NoteKind::Info,
            ),
            Err(CommitError::Review(error)) => {
                // The review can be answered again, so the dialog stays.
                state.note(error, NoteKind::Error);
                state.import_dialog = Some(preview);
            }
            Err(CommitError::Refused(error)) => {
                // Answering again cannot help: the dialog closes, and the refusal stays in
                // the import report, which the notice links to.
                state.note_import(format!("{} was refused: {error}", preview.feature.source_name), NoteKind::Error);
            }
        }
    } else if open && !cancelled {
        state.import_dialog = Some(preview);
    }
}
