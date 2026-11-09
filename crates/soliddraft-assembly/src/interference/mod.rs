//! Interference Detection and Clearance Verification, static, on certified meshes.
//!
//! Each part is rebuilt at the revision its assembly pins, as mate proofs are, and the
//! kernel tessellates it with a proven chord bound. The display copy never takes part.
//! [`geometry`] turns two placed meshes into one of three outcomes: clear, interferes, or
//! touching or undecided. An exact face mate adds what the meshes cannot: two opposed
//! faces it holds on one plane, sharing a point, touch exactly.
//!
//! Volumes of overlap, exact touching without a mate, and distances closer than the
//! meshes' bounds need an exact two-body query from the kernel; see
//! `docs/needs-catallax.md`. They are refused by name, never estimated.
//!
//! The rules are in `docs/interference.md`.

mod certified;
mod holes;
pub(crate) mod motion;

pub use holes::{HoleAlignmentReport, HoleAlignmentRequest, HoleAxis, HoleOutcome, HolePair};
pub(crate) mod geometry;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use glam::DVec3;
use serde::Serialize;
use soliddraft_document::{ContentHash, DocKind, DocRef, DocumentId};
use soliddraft_kernel_api::{Certainty, FaceId, KernelError, TessellationParams};

use crate::document::AssemblyDocument;
use crate::mates::bodies::Scope;
use crate::mates::{Alignment, Entity, FaceEndpoint, Mate, MateRefusal, RefusalCode, solver};
use crate::occurrence::{Flags, OccurrenceIndex};
use crate::path::{OccurrencePath, Step};
use crate::placement::Placement;
use crate::store::LoadState;
use geometry::Solid;

/// The chord bound asked of the kernel unless a request names another, in millimetres.
pub const DEFAULT_CHORD_MM: f64 = 0.01;

/// The finest and coarsest chord bounds a request may ask for, in millimetres.
pub const CHORD_RANGE_MM: (f64, f64) = (0.001, 1.0);

/// The most part occurrences one check takes.
pub const MAX_PARTS: usize = 128;

/// Face pairs a result names.
const FACE_PAIRS: usize = 8;

/// Which part occurrences a check compares, pair by pair.
#[derive(Clone, Debug, PartialEq)]
pub enum CheckScope {
    /// Every part in the assembly.
    Assembly,
    /// The parts at or under these occurrences: parts, sub-assemblies or pattern members.
    Occurrences(Vec<OccurrencePath>),
}

/// What to check.
#[derive(Clone, Debug, PartialEq)]
pub struct CheckRequest {
    /// Which parts.
    pub scope: CheckScope,
    /// `None` checks interference; a distance in millimetres verifies that clearance.
    pub clearance: Option<f64>,
    /// Whether hidden parts take part. Suppressed parts never do.
    pub include_hidden: bool,
    /// The chord bound asked of the kernel, in millimetres.
    pub chord: f64,
}

impl Default for CheckRequest {
    fn default() -> CheckRequest {
        CheckRequest { scope: CheckScope::Assembly, clearance: None, include_hidden: false, chord: DEFAULT_CHORD_MM }
    }
}

/// Why a check, or one part of it, was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckCode {
    /// The request itself cannot be used.
    Invalid,
    /// Fewer than two parts to compare.
    TooFewParts,
    /// More than [`MAX_PARTS`] parts.
    TooLarge,
    /// The pinned revision's bytes, a kernel or a body are not available.
    ProofUnavailable,
    /// A mesh reference body: it has no exact solid to prove anything on.
    MeshReference,
    /// The kernel will not give this body a certified mesh.
    UnsupportedBody,
    /// A part in a configuration other than the default.
    UnsupportedConfiguration,
}

impl CheckCode {
    /// The code as JSON writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            CheckCode::Invalid => "invalid",
            CheckCode::TooFewParts => "too_few_parts",
            CheckCode::TooLarge => "too_large",
            CheckCode::ProofUnavailable => "proof_unavailable",
            CheckCode::MeshReference => "mesh_reference",
            CheckCode::UnsupportedBody => "unsupported_body",
            CheckCode::UnsupportedConfiguration => "unsupported_configuration",
        }
    }
}

/// A refused check. Nothing was compared.
#[derive(Clone, Debug, PartialEq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct CheckRefusal {
    /// Why, as a code.
    #[serde(rename = "refused")]
    pub code: CheckCode,
    /// Why, in words.
    pub message: String,
}

impl CheckRefusal {
    fn new(code: CheckCode, message: impl Into<String>) -> CheckRefusal {
        CheckRefusal { code, message: message.into() }
    }
}

/// What a pair of parts was proven to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Proven apart.
    Clear,
    /// Proven to share a point inside both.
    Interferes,
    /// Neither proven: the meshes come within their bounds of each other.
    TouchingOrUndecided,
}

impl Outcome {
    /// The outcome in words, as the results panel shows it.
    pub fn words(self) -> &'static str {
        match self {
            Outcome::Clear => "Clear",
            Outcome::Interferes => "Interferes",
            Outcome::TouchingOrUndecided => "Touching or undecided",
        }
    }
}

/// What a clearance check proved of a pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClearanceOutcome {
    /// Proven farther apart than the clearance.
    Met,
    /// Proven at or under the clearance, overlaps included.
    Below,
    /// Neither proven.
    Undecided,
}

impl ClearanceOutcome {
    /// The outcome in words.
    pub fn words(self) -> &'static str {
        match self {
            ClearanceOutcome::Met => "Clearance met",
            ClearanceOutcome::Below => "At or under the clearance",
            ClearanceOutcome::Undecided => "Undecided",
        }
    }
}

/// Proven bounds on the least distance between two parts, in millimetres. Both are zero
/// for parts that interfere or are proven to touch.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct DistanceBounds {
    /// The parts are at least this far apart.
    pub lo: f64,
    /// The parts are at most this far apart; absent for parts whose boxes alone prove
    /// them clear, which are not measured further.
    pub hi: Option<f64>,
}

/// Two faces whose meshes come nearest, by label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FacePair {
    /// A face of the first part.
    pub a: String,
    /// A face of the second part.
    pub b: String,
}

/// Exact touching, from a mate: two opposed faces on one proven plane, sharing a point.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Contact {
    /// The mate that holds the faces on one plane.
    pub mate: String,
    /// The first part's face.
    pub a: String,
    /// The second part's face.
    pub b: String,
    /// A point the two faces share, in the root's frame.
    pub point: [f64; 3],
}

/// One pair of parts.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PairResult {
    /// The first part's occurrence.
    pub a: OccurrencePath,
    /// The second part's occurrence.
    pub b: OccurrencePath,
    /// Their labels in the tree.
    pub a_label: String,
    /// See `a_label`.
    pub b_label: String,
    /// What was proven.
    pub outcome: Outcome,
    /// What a clearance check proved; absent for an interference check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clearance: Option<ClearanceOutcome>,
    /// Proven bounds on the least distance between them.
    pub distance: DistanceBounds,
    /// A point proven inside both parts, for parts that interfere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub witness: Option<[f64; 3]>,
    /// The faces whose meshes come nearest, nearest first.
    pub faces: Vec<FacePair>,
    /// Faces that exactly touch, from exact mates.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contact: Vec<Contact>,
    /// What was proven, in a sentence.
    pub why: String,
}

/// A part a check used, and the revision it was proven on.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CheckedPart {
    /// The occurrence.
    pub path: OccurrencePath,
    /// Its label in the tree.
    pub label: String,
    /// The part.
    pub document: DocumentId,
    /// The pinned revision it was rebuilt from.
    pub revision: ContentHash,
    /// Triangles in its certified mesh.
    pub triangles: usize,
}

/// A part a check could not use, and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SkippedPart {
    /// The occurrence.
    pub path: OccurrencePath,
    /// Its label in the tree.
    pub label: String,
    /// Why, as a code.
    pub code: CheckCode,
    /// Why, in words.
    pub message: String,
}

/// What a check proved. Changes nothing in the assembly.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CheckReport {
    /// `interference` or `clearance`.
    pub kind: &'static str,
    /// How it was proven: `certified_mesh`.
    pub tier: &'static str,
    /// The clearance verified, for a clearance check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clearance_mm: Option<f64>,
    /// The chord bound each mesh was proven to.
    pub chord_mm: f64,
    /// The largest binary64 allowance added to the margins.
    pub rounding_mm: f64,
    /// The parts compared.
    pub parts: Vec<CheckedPart>,
    /// The parts in scope that could not be compared, each refused by name, and the
    /// sub-assemblies in scope whose structure could not be read, whose parts are unknown.
    pub skipped: Vec<SkippedPart>,
    /// True when every part in scope is known and was compared.
    pub complete: bool,
    /// Pairs compared.
    pub pairs: usize,
    /// Pairs proven clear.
    pub clear: usize,
    /// Pairs proven to interfere.
    pub interferes: usize,
    /// Pairs neither proven clear nor proven to interfere.
    pub touching_or_undecided: usize,
    /// For a clearance check: pairs at or under the clearance, met, and undecided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub below: Option<usize>,
    /// See `below`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub met: Option<usize>,
    /// See `below`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undecided: Option<usize>,
    /// Every pair not proven clear (or, for a clearance check, not proven to meet it),
    /// interfering pairs first.
    pub results: Vec<PairResult>,
    /// What this tier does not report, and why.
    pub refused: Vec<&'static str>,
}

/// What this tier refuses to report, by name.
const REFUSED: [&str; 2] = [
    "volume: an overlap's volume needs Catallax's exact two-body intersection",
    "exact_touching: touching without an exact mate needs Catallax's exact two-body distance",
];

/// A part's certified mesh in its own frame, with its faces.
pub(crate) struct LocalMesh {
    positions: Vec<[f32; 3]>,
    /// Three indices per triangle.
    indices: Vec<u32>,
    /// The face index of each triangle, into `faces`.
    triangle_faces: Vec<u32>,
    /// Each face's kernel id at this revision and its label.
    faces: Vec<(FaceId, String)>,
    chord: f64,
}

/// Certified meshes by part, revision and chord bound. Derived: dropping it changes
/// nothing but how soon the next check comes.
#[derive(Clone, Default)]
pub(crate) struct Certified {
    meshes: HashMap<(DocumentId, ContentHash, u64), Arc<LocalMesh>>,
}

/// Meshes kept between checks.
const MESHES_KEPT: usize = 256;

/// A part occurrence in scope.
struct Entry {
    row: OccurrenceIndex,
    path: OccurrencePath,
    label: String,
}

fn invalid(message: impl Into<String>) -> CheckRefusal {
    CheckRefusal::new(CheckCode::Invalid, message)
}

fn from_mate(r: MateRefusal) -> (CheckCode, String) {
    let code = match r.code {
        RefusalCode::UnsupportedConfiguration => CheckCode::UnsupportedConfiguration,
        RefusalCode::ProofUnavailable => CheckCode::ProofUnavailable,
        _ => CheckCode::Invalid,
    };
    (code, r.message)
}

/// The path of the component a pattern member was generated from, at every step: it
/// places the same definition.
fn seed(path: &OccurrencePath) -> OccurrencePath {
    OccurrencePath(path.steps().iter().map(|s| Step::component(s.component)).collect())
}

impl AssemblyDocument {
    /// Checks interference, or with `request.clearance` verifies a clearance, between
    /// every pair of parts in scope. Proves on certified meshes of the parts rebuilt at
    /// their pinned revisions and changes nothing.
    pub fn check_interference(&mut self, request: &CheckRequest) -> Result<CheckReport, CheckRefusal> {
        let (lo, hi) = CHORD_RANGE_MM;
        if !(request.chord.is_finite() && (lo..=hi).contains(&request.chord)) {
            return Err(invalid(format!("the chord bound is {lo} mm to {hi} mm")));
        }
        if let Some(c) = request.clearance
            && !(c.is_finite() && c >= 0.0)
        {
            return Err(invalid("a clearance is a length of zero or more"));
        }
        self.finish_loading();
        let (entries, unread) = self.scope_parts(request)?;
        if entries.len() < 2 {
            let mut message = "a check compares two parts or more".to_owned();
            for branch in &unread {
                message += &format!("; {}: {}", branch.label, branch.message);
            }
            return Err(CheckRefusal::new(CheckCode::TooFewParts, message));
        }
        if entries.len() > MAX_PARTS {
            return Err(CheckRefusal::new(
                CheckCode::TooLarge,
                format!("{} parts are in scope; one check takes at most {MAX_PARTS}", entries.len()),
            ));
        }
        let mut parts = Vec::new();
        // A sub-assembly whose structure could not be read hides parts nobody knows of, so
        // the check is not complete; it is listed as it stands, by path and cause.
        let mut skipped = unread;
        let mut solids: Vec<(Entry, Solid, Arc<LocalMesh>, DocRef)> = Vec::new();
        for entry in entries {
            match self.placed(&entry, request.chord) {
                Ok((solid, mesh, source)) => {
                    parts.push(CheckedPart {
                        path: entry.path.clone(),
                        label: entry.label.clone(),
                        document: source.id.expect("a proven part has an id"),
                        revision: source.pinned.clone(),
                        triangles: solid.count(),
                    });
                    solids.push((entry, solid, mesh, source));
                }
                Err((code, message)) => {
                    skipped.push(SkippedPart { path: entry.path, label: entry.label, code, message })
                }
            }
        }
        let contacts = self.contacts();
        let mut results = Vec::new();
        let (mut clear, mut interferes, mut open) = (0, 0, 0);
        let (mut below, mut met, mut undecided) = (0, 0, 0);
        let mut pairs = 0;
        for i in 0..solids.len() {
            for j in i + 1..solids.len() {
                pairs += 1;
                let (a, b) = (&solids[i], &solids[j]);
                let (mut result, near) = compare(&a.1, &b.1, request.clearance);
                result.a = a.0.path.clone();
                result.b = b.0.path.clone();
                result.a_label = a.0.label.clone();
                result.b_label = b.0.label.clone();
                let label = |m: &LocalMesh, s: &Solid, t: u32| m.faces[s.face(t) as usize].1.clone();
                result.faces = near
                    .iter()
                    .map(|(ta, tb)| FacePair { a: label(&a.2, &a.1, *ta), b: label(&b.2, &b.1, *tb) })
                    .fold(Vec::new(), |mut v, p| {
                        if !v.contains(&p) && v.len() < FACE_PAIRS {
                            v.push(p);
                        }
                        v
                    });
                if result.outcome != Outcome::Clear {
                    self.add_contacts(&mut result, &contacts, (&a.1, &a.2, &a.3), (&b.1, &b.2, &b.3));
                }
                match result.outcome {
                    Outcome::Clear => clear += 1,
                    Outcome::Interferes => interferes += 1,
                    Outcome::TouchingOrUndecided => open += 1,
                }
                match result.clearance {
                    Some(ClearanceOutcome::Below) => below += 1,
                    Some(ClearanceOutcome::Met) => met += 1,
                    Some(ClearanceOutcome::Undecided) => undecided += 1,
                    None => {}
                }
                let listed = match result.clearance {
                    Some(c) => c != ClearanceOutcome::Met,
                    None => result.outcome != Outcome::Clear,
                };
                if listed {
                    results.push(result);
                }
            }
        }
        let rank = |r: &PairResult| match (r.clearance, r.outcome) {
            (_, Outcome::Interferes) => 0,
            (Some(ClearanceOutcome::Below), _) => 1,
            _ => 2,
        };
        results.sort_by(|p, q| rank(p).cmp(&rank(q)).then(p.a.cmp(&q.a)).then(p.b.cmp(&q.b)));
        let clearance = request.clearance.is_some();
        Ok(CheckReport {
            kind: if clearance { "clearance" } else { "interference" },
            tier: "certified_mesh",
            clearance_mm: request.clearance,
            chord_mm: request.chord,
            rounding_mm: solids.iter().map(|s| s.1.rounding).fold(0.0, f64::max),
            complete: skipped.is_empty(),
            parts,
            skipped,
            pairs,
            clear,
            interferes,
            touching_or_undecided: open,
            below: clearance.then_some(below),
            met: clearance.then_some(met),
            undecided: clearance.then_some(undecided),
            results,
            refused: REFUSED.to_vec(),
        })
    }

    /// The part occurrences a request names, in table order, each once, and the
    /// sub-assemblies in scope or above it whose pinned structure could not be read, so
    /// whose parts are not known. Suppressed branches are left out, and hidden ones unless asked for, as their
    /// parts are.
    fn scope_parts(&self, request: &CheckRequest) -> Result<(Vec<Entry>, Vec<SkippedPart>), CheckRefusal> {
        let roots: Vec<OccurrenceIndex> = match &request.scope {
            CheckScope::Assembly => vec![OccurrenceIndex::ROOT],
            CheckScope::Occurrences(paths) => paths
                .iter()
                .map(|p| self.table.find(p).ok_or_else(|| invalid(format!("the assembly has no occurrence {p}"))))
                .collect::<Result<_, _>>()?,
        };
        let mut rows: Vec<OccurrenceIndex> = roots.into_iter().flat_map(|r| self.table.subtree(r)).collect();
        rows.sort();
        rows.dedup();
        // A sub-assembly is known only at its pinned revision. One whose structure could
        // not be read hides its parts; one shown from another revision places them where
        // that revision does, not where the pin does. Either way its parts are left out
        // and it is listed, whether it is in the scope or above it.
        let unknown = |row: OccurrenceIndex| -> Option<String> {
            if !self.table.flags(row).has(Flags::ASSEMBLY) {
                return None;
            }
            let def = self.store.get(self.table.definition(row));
            let cause = def.problem.as_ref().map(|p| format!(": {p}")).unwrap_or_default();
            match (&def.assembly, def.state) {
                (None, _) => Some(format!("its structure could not be read, so its parts are not known{cause}")),
                (Some(_), LoadState::Stale) => {
                    Some(format!("its pinned revision could not be read, so where its parts sit is not known{cause}"))
                }
                _ => None,
            }
        };
        // Down from the root, an occurrence's own hidden or suppressed flag is trusted only
        // while every assembly above it is known at its pin, since an unknown one supplied
        // the flag. The first unknown assembly on the way is listed instead.
        let mut unread: BTreeMap<OccurrenceIndex, String> = BTreeMap::new();
        let mut parts = Vec::new();
        for row in rows {
            let mut chain = vec![row];
            while let Some(up) = self.table.parent(*chain.last().expect("one row")) {
                chain.push(up);
            }
            chain.pop(); // the root: always known, never hidden
            let mut in_scope = true;
            for &r in chain.iter().rev() {
                let f = self.table.flags(r);
                let out = f.has(Flags::REMOVED)
                    || f.has(Flags::SUPPRESSED_SELF)
                    || (!request.include_hidden && f.has(Flags::HIDDEN_SELF));
                if out {
                    in_scope = false;
                    break;
                }
                if let Some(why) = unknown(r) {
                    unread.entry(r).or_insert(why);
                    in_scope = false;
                    break;
                }
            }
            if in_scope && !self.table.flags(row).has(Flags::ASSEMBLY) {
                parts.push(Entry { row, path: self.table.path(row), label: self.label(row) });
            }
        }
        let unread = unread
            .into_iter()
            .map(|(row, message)| SkippedPart {
                path: self.table.path(row),
                label: self.label(row),
                code: CheckCode::ProofUnavailable,
                message,
            })
            .collect();
        Ok((parts, unread))
    }

    /// A part occurrence's certified mesh, placed in the root's frame.
    fn placed(&mut self, entry: &Entry, chord: f64) -> Result<(Solid, Arc<LocalMesh>, DocRef), (CheckCode, String)> {
        let def = self.store.get(self.table.definition(entry.row));
        if def.kind == DocKind::Mesh {
            return Err((
                CheckCode::MeshReference,
                format!("{} is a mesh reference body; it has no exact solid", entry.label),
            ));
        }
        let source = {
            let scope = Scope::new(&self.assembly, &self.table, &self.store);
            let seed = seed(&entry.path);
            let source = scope.part_source(&seed).map_err(from_mate)?;
            let chain = scope.chain(&seed);
            self.rebased_source(&source, &chain)
        };
        let mesh = self.certified_mesh(&source, chord)?;
        let at = self.table.world(entry.row);
        let place = |i: u32| {
            let p = mesh.positions[i as usize];
            at.apply(DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
        };
        let tris: Vec<[DVec3; 3]> = mesh.indices.as_chunks::<3>().0.iter().map(|t| t.map(place)).collect();
        if !tris.iter().flatten().all(|p| p.is_finite()) {
            return Err((
                CheckCode::UnsupportedBody,
                format!("{}: a placed mesh point is not a finite number", entry.label),
            ));
        }
        let solid = Solid::new(tris, mesh.triangle_faces.clone(), mesh.chord);
        Ok((solid, mesh, source))
    }

    /// The certified mesh of the part `r` pins, rebuilt from that revision's bytes.
    fn certified_mesh(&mut self, r: &DocRef, chord: f64) -> Result<Arc<LocalMesh>, (CheckCode, String)> {
        let id = r.id.ok_or((CheckCode::ProofUnavailable, "the reference has no document id".to_owned()))?;
        let key = (id, r.pinned.clone(), chord.to_bits());
        if let Some(mesh) = self.certified.meshes.get(&key) {
            return Ok(mesh.clone());
        }
        let part = self.proof_part(r).map_err(from_mate)?;
        let part = part.lock().expect("the part lock");
        let name = crate::occurrence::label_of(r);
        let body = part.body().ok_or((CheckCode::ProofUnavailable, format!("{name} has no body")))?;
        let params = TessellationParams {
            chord_height: chord,
            curve_angle_deg: 0.0,
            surface_angle_deg: 0.0,
            max_edge_length: 0.0,
            max_aspect_ratio: 0.0,
        };
        let mesh = part.kernel().tessellate(body, &params).map_err(|e| {
            let code = match e {
                KernelError::Unsupported(_) => CheckCode::UnsupportedBody,
                _ => CheckCode::ProofUnavailable,
            };
            (code, format!("{name}: {e}"))
        })?;
        if mesh.unproved {
            return Err((CheckCode::UnsupportedBody, format!("{name}: the kernel gave an unproved mesh")));
        }
        if let Some(c) = mesh.unavailable_controls.iter().find(|c| c.contains("chord")) {
            return Err((CheckCode::UnsupportedBody, format!("{name}: the chord bound is not certified ({c})")));
        }
        let mut faces: Vec<(FaceId, String)> = Vec::new();
        let mut index: BTreeMap<FaceId, u32> = BTreeMap::new();
        let triangle_faces = mesh
            .triangle_faces
            .iter()
            .map(|f| {
                *index.entry(*f).or_insert_with(|| {
                    let label = part.face_label(*f).map(|l| l.text).unwrap_or_else(|| format!("Face {}", f.raw()));
                    faces.push((*f, label));
                    faces.len() as u32 - 1
                })
            })
            .collect();
        let local =
            Arc::new(LocalMesh { positions: mesh.positions, indices: mesh.indices, triangle_faces, faces, chord });
        drop(part);
        if self.certified.meshes.len() >= MESHES_KEPT {
            self.certified.meshes.clear();
        }
        self.certified.meshes.insert(key, local.clone());
        Ok(local)
    }

    /// The root's face mates that hold two opposed faces exactly on one plane, by proof
    /// on the accepted placements: the mate, both ends and their faces.
    fn contacts(&mut self) -> Vec<MateContact> {
        let measured = self.mate_measurements();
        let mates: Vec<_> = self.assembly.known_mates().cloned().collect();
        mates
            .iter()
            .filter_map(|m| {
                // Only the families that hold the two planes a fixed distance apart; a
                // reference plane has no face on the mesh.
                let (Mate::Distance { a, b, alignment, .. } | Mate::Coincident { a, b, alignment, .. }) = m else {
                    return None;
                };
                let (Entity::Face(face_a), Entity::Face(face_b)) = (&a.entity, &b.entity) else { return None };
                let (_, Ok(Some(measurement))) = measured.iter().find(|(id, _)| *id == m.id())? else { return None };
                let exact = measurement.certainty == Certainty::Exact && measurement.rational == "0";
                // The world axis both planes lie across, from turns that take each normal
                // exactly onto it.
                let axis = |e: &FaceEndpoint, at: &Placement| {
                    solver::turned(at.rotation, e.entity.stored_normal()?)?.iter().position(|c| *c != 0)
                };
                let [pa, pb] = &measurement.placements;
                let axis = axis(a, pa).filter(|x| axis(b, pb) == Some(*x))?;
                (exact && *alignment == Alignment::Opposed).then(|| MateContact {
                    mate: m.name().to_owned(),
                    a: (a.path.clone(), face_a.clone()),
                    b: (b.path.clone(), face_b.clone()),
                    axis,
                })
            })
            .collect()
    }

    /// Adds exact touching from `contacts` to an undecided or interfering pair.
    fn add_contacts(
        &mut self,
        result: &mut PairResult,
        contacts: &[MateContact],
        a: (&Solid, &Arc<LocalMesh>, &DocRef),
        b: (&Solid, &Arc<LocalMesh>, &DocRef),
    ) {
        for c in contacts {
            let (first, second) = if c.a.0 == result.a && c.b.0 == result.b {
                ((a, &c.a.1), (b, &c.b.1))
            } else if c.a.0 == result.b && c.b.0 == result.a {
                ((a, &c.b.1), (b, &c.a.1))
            } else {
                continue;
            };
            let ((sa, ma, ra), face_a) = first;
            let ((sb, mb, rb), face_b) = second;
            let (Some(fa), Some(fb)) = (self.face_index(ra, ma, face_a), self.face_index(rb, mb, face_b)) else {
                continue;
            };
            // Both placed faces must lie across the axis the mate proved they share.
            if plane_axis(sa, fa) != Some(c.axis) || plane_axis(sb, fb) != Some(c.axis) {
                continue;
            }
            if let Some(p) = geometry::shared_point(sa, fa, sb, fb, c.axis) {
                result.contact.push(Contact {
                    mate: c.mate.clone(),
                    a: ma.faces[fa as usize].1.clone(),
                    b: mb.faces[fb as usize].1.clone(),
                    point: p.to_array(),
                });
            }
        }
        if !result.contact.is_empty() && result.outcome == Outcome::TouchingOrUndecided {
            result.distance = DistanceBounds { lo: 0.0, hi: Some(0.0) };
            if let Some(c) = result.clearance.as_mut() {
                *c = ClearanceOutcome::Below;
            }
            let c = &result.contact[0];
            result.why = format!(
                "Exactly touching: mate {} holds {} and {} on one plane, opposed, and the faces share the point {}. \
                 No overlap was proven; proving there is none needs the exact tier.",
                c.mate,
                c.a,
                c.b,
                point_text(c.point)
            );
        }
    }

    /// The index in `mesh` of the face a mate names, resolved on the part at the pinned
    /// revision.
    fn face_index(&mut self, r: &DocRef, mesh: &LocalMesh, face: &soliddraft_model::FaceRef) -> Option<u32> {
        let part = self.proof_part(r).ok()?;
        let part = part.lock().ok()?;
        let found = part.resolve_face(face, part.body()?).ok()?;
        mesh.faces.iter().position(|(f, _)| *f == found).map(|i| i as u32)
    }
}

/// A root mate holding two opposed faces exactly on one plane.
struct MateContact {
    mate: String,
    a: (OccurrencePath, soliddraft_model::FaceRef),
    b: (OccurrencePath, soliddraft_model::FaceRef),
    /// The world axis normal to the plane they share.
    axis: usize,
}

/// The world axis normal to a planar face, read from its placed triangles.
fn plane_axis(solid: &Solid, face: u32) -> Option<usize> {
    let t = (0..solid.count() as u32).find(|t| solid.face(*t) == face)?;
    let [p, q, r] = solid.triangle(t);
    let n = (q - p).cross(r - p).abs();
    let axis = if n.x >= n.y && n.x >= n.z {
        0
    } else if n.y >= n.z {
        1
    } else {
        2
    };
    (n[axis] > 0.0).then_some(axis)
}

fn point_text(p: [f64; 3]) -> String {
    format!("({}, {}, {}) mm", round(p[0]), round(p[1]), round(p[2]))
}

fn round(v: f64) -> String {
    let r = (v * 1e4).round() / 1e4;
    if r == 0.0 { "0".into() } else { format!("{r}") }
}

fn mm(v: f64) -> String {
    format!("{} mm", round(v))
}

/// A distance known to lie from `lo` to `hi`, as one length when the two round alike.
fn span(lo: f64, hi: f64) -> String {
    if round(lo) == round(hi) { mm(hi) } else { format!("{} to {}", round(lo), mm(hi)) }
}

/// What two placed solids are proven to be, and the triangle pairs within the clearance
/// and margin, nearest first, for naming faces. The paths and labels are the caller's.
fn compare(a: &Solid, b: &Solid, clearance: Option<f64>) -> (PairResult, Vec<(u32, u32)>) {
    let margin = certified::add_up(a.slack(), b.slack());
    let cutoff = margin + clearance.unwrap_or(0.0);
    let gap = a.bounds.distance(&b.bounds);
    let mut result = PairResult {
        a: OccurrencePath::root(),
        b: OccurrencePath::root(),
        a_label: String::new(),
        b_label: String::new(),
        outcome: Outcome::TouchingOrUndecided,
        clearance: None,
        distance: DistanceBounds { lo: 0.0, hi: None },
        witness: None,
        faces: Vec::new(),
        contact: Vec::new(),
        why: String::new(),
    };
    if gap > cutoff {
        // Every point of each mesh lies in its box, so the meshes are at least as far
        // apart as the boxes.
        let lo = certified::sub_down(gap, margin);
        result.outcome = Outcome::Clear;
        result.distance = DistanceBounds { lo, hi: None };
        result.why = format!("The parts are at least {} apart: their certified meshes' boxes are.", mm(lo));
        result.clearance = clearance.map(|_| ClearanceOutcome::Met);
        return (result, Vec::new());
    }
    let near = geometry::near(a, b, cutoff);
    let faces = near.pairs.iter().map(|(ta, tb, _)| (*ta, *tb)).collect();
    let Some(d) = near.upper else {
        result.why = "A part's certified mesh is empty.".into();
        result.clearance = clearance.map(|_| ClearanceOutcome::Undecided);
        return (result, faces);
    };
    // The meshes are proven at least `lower` apart and at most `d`.
    let lower = near.lower;
    // The parts are at most this far apart.
    let hi = certified::add_up(d, margin);
    let within: Vec<_> = near.pairs.iter().filter(|p| p.2.lo <= margin).copied().collect();
    if let Some(y) = geometry::witness(a, b, &within) {
        result.outcome = Outcome::Interferes;
        result.distance = DistanceBounds { lo: 0.0, hi: Some(0.0) };
        result.witness = Some(y.to_array());
        result.why = format!(
            "The point {} is inside both parts: inside both certified meshes and farther from each than its bound.",
            point_text(y.to_array())
        );
    } else if lower > margin {
        // The two meshes' surfaces do not meet, so each connected piece of either surface
        // lies wholly inside the other mesh or wholly outside it, and the meshes overlap
        // only where a piece of one lies inside the other. One point per piece settles it,
        // every piece of both: a patterned part's mesh has one piece per copy.
        let inside = |s: &Solid, other: &Solid| -> Vec<Option<bool>> {
            s.pieces().iter().map(|t| other.contains(s.triangle(*t)[0])).collect()
        };
        let found = [inside(a, b), inside(b, a)].concat();
        if found.iter().all(|c| *c == Some(false)) {
            let lo = certified::sub_down(lower, margin);
            result.outcome = Outcome::Clear;
            result.distance = DistanceBounds { lo, hi: Some(hi) };
            result.why = format!(
                "The parts are {} to {} apart: their certified meshes are {} apart, and each mesh is within {} of its part.",
                mm(lo),
                mm(hi),
                span(lower, d),
                mm(a.slack().max(b.slack()))
            );
        } else {
            result.distance = DistanceBounds { lo: 0.0, hi: Some(hi) };
            result.why = if found.contains(&Some(true)) {
                "A piece of one part's certified mesh lies inside the other's, and no point was proven inside both."
            } else {
                "Whether a piece of one part's certified mesh lies inside the other's is unsettled: every containment \
                 ray grazed a triangle."
            }
            .into();
        }
    } else {
        result.distance = DistanceBounds { lo: 0.0, hi: Some(hi) };
        result.why = format!(
            "The certified meshes come within {} of each other, inside the {} the bounds leave open: the parts may \
             touch, overlap by less than that, or stand apart by less than that.",
            mm(d),
            mm(margin)
        );
    }
    result.clearance = clearance.map(|c| match result.outcome {
        Outcome::Interferes => ClearanceOutcome::Below,
        _ if result.distance.hi.is_some_and(|hi| hi <= c) => ClearanceOutcome::Below,
        _ if result.distance.lo > c => ClearanceOutcome::Met,
        _ => ClearanceOutcome::Undecided,
    });
    if let (Some(c), Some(outcome)) = (clearance, result.clearance)
        && outcome != ClearanceOutcome::Undecided
        && result.outcome != Outcome::Interferes
    {
        let side = if outcome == ClearanceOutcome::Met { "more than" } else { "at most" };
        result.why = format!("{} So they are {side} {} apart.", result.why, mm(c));
    }
    (result, faces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use geometry::tests_support::block;

    fn compare(a: &Solid, b: &Solid, clearance: Option<f64>) -> PairResult {
        super::compare(a, b, clearance).0
    }

    #[test]
    fn three_outcomes_on_blocks() {
        let a = block([0.0; 3], [40.0, 30.0, 10.0], 0.01);
        let apart = block([10.0, 5.0, 12.0], [30.0, 25.0, 18.0], 0.01);
        let r = compare(&a, &apart, None);
        assert_eq!(r.outcome, Outcome::Clear);
        assert!(r.distance.lo > 1.97 && r.distance.lo < 2.0, "{:?}", r.distance);
        let on = block([10.0, 5.0, 10.0], [30.0, 25.0, 16.0], 0.01);
        let r = compare(&a, &on, None);
        assert_eq!(r.outcome, Outcome::TouchingOrUndecided);
        assert_eq!(r.distance.lo, 0.0);
        let into = block([10.0, 5.0, 9.0], [30.0, 25.0, 15.0], 0.01);
        let r = compare(&a, &into, None);
        assert_eq!(r.outcome, Outcome::Interferes);
        assert!(r.witness.is_some());
    }

    #[test]
    fn clearance_is_proven_both_ways_or_undecided() {
        let a = block([0.0; 3], [40.0, 30.0, 10.0], 0.01);
        let b = block([10.0, 5.0, 12.0], [30.0, 25.0, 18.0], 0.01);
        assert_eq!(compare(&a, &b, Some(1.0)).clearance, Some(ClearanceOutcome::Met));
        assert_eq!(compare(&a, &b, Some(3.0)).clearance, Some(ClearanceOutcome::Below));
        // Two millimetres apart, asked for two: the bounds cannot tell.
        assert_eq!(compare(&a, &b, Some(2.0)).clearance, Some(ClearanceOutcome::Undecided));
        let into = block([10.0, 5.0, 9.0], [30.0, 25.0, 15.0], 0.01);
        assert_eq!(compare(&a, &into, Some(0.5)).clearance, Some(ClearanceOutcome::Below));
    }

    #[test]
    fn near_parallel_ridges_never_prove_a_clearance_they_miss() {
        // The parts are 0.01 mm apart where the ridges cross; the margin is about 0.002 mm.
        // Asked for 0.011 mm, the bounds cannot tell, at the review's 9e-7 rad as at its
        // controls, 0 and 1.1e-6 rad.
        for angle in [0.0, 9e-7, 1.1e-6] {
            let (a, b) = geometry::tests_support::ridges(angle, 0.001);
            let r = compare(&a, &b, Some(0.011));
            assert_eq!(r.clearance, Some(ClearanceOutcome::Undecided), "angle {angle}: {r:#?}");
            assert!(r.distance.lo <= 0.01, "angle {angle}: {:?}", r.distance);
            assert!(r.distance.hi.is_some_and(|hi| hi >= 0.01), "angle {angle}: {:?}", r.distance);
            // Asked for 0.007 mm, below the distance less the margin, the clearance is met.
            assert_eq!(compare(&a, &b, Some(0.007)).clearance, Some(ClearanceOutcome::Met), "angle {angle}");
        }
    }

    /// The review's patterned part: 100 unit cubes 20 mm apart along x, in `order`.
    fn cubes(order: impl IntoIterator<Item = usize>) -> Solid {
        let at = |i: usize| ([20.0 * i as f64, 0.0, 0.0], [20.0 * i as f64 + 1.0, 1.0, 1.0]);
        geometry::tests_support::boxes(&order.into_iter().map(at).collect::<Vec<_>>(), 0.001)
    }

    #[test]
    fn a_piece_of_a_part_inside_another_part_is_never_clear() {
        // B holds the whole of cube 2, [40, 41] x [0, 1] x [0, 1], and its surface is 4 mm
        // from every cube's. Neither mesh's first vertex is inside the other, in any order.
        let b = block([35.0, -4.0, -4.0], [46.0, 7.0, 7.0], 0.001);
        let orders: [Vec<usize>; 3] = [
            (0..100).collect(),
            [2].into_iter().chain((0..100).filter(|i| *i != 2)).collect(),
            (0..100).rev().collect(),
        ];
        for order in orders {
            let first = order[0];
            let a = cubes(order);
            for (x, y) in [(&a, &b), (&b, &a)] {
                let r = compare(x, y, Some(0.5));
                assert_eq!((r.outcome, r.clearance), (Outcome::Interferes, Some(ClearanceOutcome::Below)), "{r:#?}");
                let w = r.witness.expect("a witness");
                assert!(w[0] > 40.0 && w[0] < 41.0, "cube {first} first: {w:?}");
            }
        }
    }

    #[test]
    fn a_piece_too_thin_to_prove_is_undecided_not_clear() {
        // A plate 0.0015 mm thick, thinner than twice its mesh's bound, inside B: no point
        // is proven inside both, and B holds it all the same.
        let a = geometry::tests_support::boxes(&[([0.0; 3], [1.0; 3]), ([40.0, 0.0, 0.0], [41.0, 1.0, 0.0015])], 0.001);
        let b = block([35.0, -4.0, -4.0], [46.0, 7.0, 7.0], 0.001);
        let r = compare(&a, &b, Some(0.5));
        assert_eq!(
            (r.outcome, r.clearance),
            (Outcome::TouchingOrUndecided, Some(ClearanceOutcome::Undecided)),
            "{r:#?}"
        );
        assert_eq!(r.distance.lo, 0.0);
        assert!(r.why.starts_with("A piece of one part's"), "{}", r.why);
    }

    #[test]
    fn pieces_of_a_part_all_outside_another_are_clear() {
        // B between cubes 0 and 1, 4 mm from each.
        let b = block([5.0, -4.0, -4.0], [16.0, 7.0, 7.0], 0.001);
        for a in [cubes(0..100), cubes((0..100).rev())] {
            let r = compare(&a, &b, Some(0.5));
            assert_eq!((r.outcome, r.clearance), (Outcome::Clear, Some(ClearanceOutcome::Met)), "{r:#?}");
            assert!(r.distance.lo > 3.99 && r.distance.lo < 4.0, "{:?}", r.distance);
        }
    }

    #[test]
    fn a_long_skinny_face_never_proves_a_clearance_it_misses() {
        // The review's blocks: A's top face is a 30 m by 0.0625 mm strip of two slivers and
        // B's underside is exactly 0.015625 mm over it. Binary64 projection onto a sliver
        // put B 0.022 mm away, past 0.018 mm with the margin; the gap fails 0.018 mm.
        let a = block([0.0, 0.0, -1.0], [30_000.0, 0.0625, 0.0], 0.001);
        let b = block([21_000.0, 0.015625, 0.015625], [21_001.0, 0.0234375, 1.0], 0.001);
        // Shorter by ten along x, or ten times wider along y: the same gap.
        let short = (
            block([0.0, 0.0, -1.0], [3_000.0, 0.0625, 0.0], 0.001),
            block([2_100.0, 0.015625, 0.015625], [2_101.0, 0.0234375, 1.0], 0.001),
        );
        let wide = (
            block([0.0, 0.0, -1.0], [30_000.0, 0.625, 0.0], 0.001),
            block([21_000.0, 0.15625, 0.015625], [21_001.0, 0.234375, 1.0], 0.001),
        );
        // Moved 100 m off the origin in x and y: every coordinate is still exact in f32.
        let moved = (
            block([100_000.0, 100_000.0, -1.0], [130_000.0, 100_000.062_5, 0.0], 0.001),
            block([121_000.0, 100_000.015_625, 0.015625], [121_001.0, 100_000.023_437_5, 1.0], 0.001),
        );
        let cases = [("long", (&a, &b)), ("short", (&short.0, &short.1)), ("wide", (&wide.0, &wide.1))];
        for (name, (a, b)) in cases.into_iter().chain([("moved", (&moved.0, &moved.1))]) {
            for (x, y) in [(a, b), (b, a)] {
                let r = compare(x, y, Some(0.018));
                assert_eq!((r.outcome, r.clearance), (Outcome::Clear, Some(ClearanceOutcome::Below)), "{name}: {r:#?}");
                let hi = r.distance.hi.expect("an upper bound");
                assert!(r.distance.lo <= 0.015625 && hi >= 0.015625, "{name}: {:?}", r.distance);
                // Under the gap less the margin, the clearance is met.
                let r = compare(x, y, Some(0.012));
                assert_eq!(r.clearance, Some(ClearanceOutcome::Met), "{name}: {r:#?}");
            }
        }
    }

    #[test]
    fn a_part_inside_another_is_not_clear() {
        let a = block([0.0; 3], [40.0, 30.0, 10.0], 0.01);
        let b = block([10.0, 10.0, 2.0], [20.0, 20.0, 8.0], 0.01);
        assert_eq!(compare(&a, &b, None).outcome, Outcome::Interferes);
    }
}
