//! Finding faces and reference planes and proving their planes at a pinned revision.
//!
//! A proof belongs to a document, a revision and a face. It comes from the part rebuilt on
//! the kernel from that revision's bytes, never from a display copy, so a cached picture
//! can be shown but never proves a mate. Selectors are for commands: what is stored is the
//! part's persistent [`FaceRef`], or the datum or feature id of a reference plane.
//!
//! A reference plane is proved from its definition: a datum plane is exact, and a plane
//! feature is its datum or source plane moved by its stored offset, added in rationals. A
//! plane feature on a face is refused; the face itself can be mated.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use num_rational::BigRational;
use serde::Serialize;
use soliddraft_document::{ConfigurationId, ContentHash, DocKind, DocRef, DocumentId};
use soliddraft_kernel_api::{Axis, ExactAxisPlane, ExactRational, FaceId, KernelError};
use soliddraft_model::{
    DatumPlane, Document as PartDocument, FaceRef, FeatureId, FeatureKind, Generator, RepairReason, SketchPlane,
};

use crate::document::AssemblyDocument;
use crate::mates::{EndView, Entity, FaceEndpoint, MateRefusal, PartPlane, RefusalCode, datum_normal};
use crate::path::ComponentId;
use crate::schema::Assembly;

/// How many rebuilt parts a document keeps for proofs.
const PARTS_KEPT: usize = 16;

/// Which kind of face a selector asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeneratorChoice {
    /// An extrusion's cap in the sketch plane.
    StartCap,
    /// An extrusion's cap at its far end.
    EndCap,
    /// A side swept from a sketch entity.
    Side,
}

impl GeneratorChoice {
    fn matches(self, generator: &Generator) -> bool {
        match self {
            GeneratorChoice::StartCap => *generator == Generator::StartCap,
            GeneratorChoice::EndCap => *generator == Generator::EndCap,
            GeneratorChoice::Side => matches!(generator, Generator::Side { .. }),
        }
    }
}

/// How a command names a face. Each finds one face at command time or refuses.
#[derive(Clone, Debug, PartialEq)]
pub enum FaceSelector {
    /// The persistent name a pick carries.
    Picked(FaceRef),
    /// The face's current label, such as "Boss-Extrude1 · End cap".
    Label(String),
    /// A reference plane: a datum plane by its name, such as "Top plane", or a plane
    /// feature by its name.
    Plane(String),
    /// The face a feature's generator made; a side also says which way it looks.
    Generator {
        /// The feature's current name.
        feature: String,
        /// What made the face.
        generator: GeneratorChoice,
        /// The proven outward normal, to tell the sides apart.
        normal: Option<[f64; 3]>,
    },
}

/// A face of one of the root's components.
#[derive(Clone, Debug, PartialEq)]
pub struct EndpointSelector {
    /// The component.
    pub component: ComponentId,
    /// The face.
    pub face: FaceSelector,
}

/// A plane proven on one revision of a part.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlaneProof {
    /// The part.
    pub document: DocumentId,
    /// The revision it was proven on.
    pub revision: ContentHash,
    /// That revision's geometry hash.
    pub geometry: ContentHash,
    /// The plane in the part's frame.
    pub plane: ExactAxisPlane,
    /// The face's label at that revision.
    pub label: String,
}

/// What a proof is of on its part.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Proved {
    Face(FeatureId, Generator, u32),
    Datum(DatumPlane),
    Plane(FeatureId),
}

type PlaneKey = (DocumentId, ContentHash, Proved);

/// Proofs and the rebuilt parts behind them. All of it is derived: dropping it changes
/// nothing but how soon the next proof comes.
#[derive(Default)]
pub(crate) struct Proofs {
    parts: HashMap<(DocumentId, ContentHash), Arc<Mutex<PartDocument>>>,
    planes: HashMap<PlaneKey, PlaneProof>,
    /// How many parts were rebuilt for proofs.
    pub built: usize,
}

fn unavailable(message: String) -> MateRefusal {
    MateRefusal::new(RefusalCode::ProofUnavailable, message)
}

fn configuration_refusal(what: fmt::Arguments, configuration: &ConfigurationId) -> MateRefusal {
    MateRefusal::new(
        RefusalCode::UnsupportedConfiguration,
        format!("{what} names configuration {}; this version mates default configurations only", configuration.0),
    )
}

/// Refuses a mate that would have to ignore a configuration. A file keeps the field as
/// read, so this runs before anything is proved or changed.
pub(crate) fn check_configurations(truth: &Assembly) -> Result<(), MateRefusal> {
    for mate in truth.known_mates() {
        for end in mate.ends() {
            check_end(&end).map_err(|r| MateRefusal { mates: vec![mate.name().to_owned()], ..r })?;
        }
        for c in mate.components() {
            let source = truth.component(c).and_then(|c| truth.source(c.source));
            if let Some(configuration) = source.and_then(|s| s.document.configuration.as_ref()) {
                let refused = configuration_refusal(format_args!("component {c}"), configuration);
                return Err(MateRefusal { mates: vec![mate.name().to_owned()], ..refused });
            }
        }
    }
    Ok(())
}

/// Refuses a stored face of a configuration other than the default.
pub(crate) fn check_endpoint(end: &FaceEndpoint) -> Result<(), MateRefusal> {
    check_end(&EndView { path: &end.path, document: end.document, configuration: end.configuration.as_ref() })
}

/// Refuses a stored part occurrence of a configuration other than the default.
pub(crate) fn check_end(end: &EndView) -> Result<(), MateRefusal> {
    match end.configuration {
        Some(configuration) => Err(configuration_refusal(format_args!("occurrence {}", end.path), configuration)),
        None => Ok(()),
    }
}

impl AssemblyDocument {
    pub(crate) fn proof_part(&mut self, r: &DocRef) -> Result<Arc<Mutex<PartDocument>>, MateRefusal> {
        let id = r.id.ok_or_else(|| unavailable("the reference has no document id".into()))?;
        let key = (id, r.pinned.clone());
        if let Some(part) = self.proofs.parts.get(&key) {
            return Ok(part.clone());
        }
        let bytes = self.revision_bytes(r).map_err(unavailable)?;
        let display = self.options.display.clone();
        let source = display
            .as_deref()
            .and_then(|d| d.kernel_source())
            .ok_or_else(|| unavailable("this session has no kernel to prove planes on".into()))?;
        let part = Arc::new(Mutex::new(source.exact(&bytes).map_err(unavailable)?));
        if self.proofs.parts.len() == PARTS_KEPT {
            self.proofs.parts.clear();
        }
        self.proofs.parts.insert(key, part.clone());
        self.proofs.built += 1;
        Ok(part)
    }

    /// The proof of the plane `entity` lies on, at the revision `r` pins.
    pub(crate) fn prove(&mut self, r: &DocRef, entity: &Entity) -> Result<PlaneProof, MateRefusal> {
        let id = r.id.ok_or_else(|| unavailable("the reference has no document id".into()))?;
        let what = match entity {
            Entity::Face(face) => Proved::Face(face.feature, face.generator.clone(), face.ordinal),
            Entity::Plane(PartPlane::Datum { datum }) => Proved::Datum(*datum),
            Entity::Plane(PartPlane::Feature { feature, .. }) => Proved::Plane(*feature),
        };
        let key = (id, r.pinned.clone(), what);
        let proof = match self.proofs.planes.get(&key) {
            Some(proof) => proof.clone(),
            None => {
                let part = self.proof_part(r)?;
                let part = part.lock().expect("the part lock");
                let proof = match entity {
                    Entity::Face(face) => {
                        let body = part.body().ok_or_else(|| unavailable(format!("{} has no body", r.path)))?;
                        let found = part.resolve_face(face, body).map_err(|repair| {
                            let code = match repair.reason {
                                RepairReason::Ambiguous => RefusalCode::AmbiguousReference,
                                RepairReason::Missing | RepairReason::Mismatch { .. } => RefusalCode::DanglingReference,
                            };
                            MateRefusal::new(code, repair.message)
                        })?;
                        plane_of(&part, r, id, found)?
                    }
                    Entity::Plane(plane) => reference_plane_proof(&part, r, id, plane)?,
                };
                drop(part);
                self.proofs.planes.insert(key, proof.clone());
                proof
            }
        };
        if let Entity::Plane(PartPlane::Feature { normal, .. }) = entity
            && proof.plane.normal() != *normal
        {
            let message = format!("{} now faces another way than when it was mated", proof.label);
            return Err(MateRefusal::new(RefusalCode::DanglingReference, message));
        }
        Ok(proof)
    }

    /// The persistent name of the face or plane `selector` finds on the part `r` pins.
    pub(crate) fn select(&mut self, r: &DocRef, selector: &FaceSelector) -> Result<Entity, MateRefusal> {
        let id = r.id.ok_or_else(|| unavailable("the reference has no document id".into()))?;
        let part = self.proof_part(r)?;
        let part = part.lock().expect("the part lock");
        if let FaceSelector::Plane(name) = selector {
            let name = name.trim();
            let datum = [DatumPlane::Top, DatumPlane::Front, DatumPlane::Right]
                .into_iter()
                .filter(|d| part.datum_name(*d).eq_ignore_ascii_case(name) || d.label().eq_ignore_ascii_case(name));
            let features = part
                .features()
                .iter()
                .filter(|f| matches!(f.kind, FeatureKind::ReferencePlane(_)) && f.name.eq_ignore_ascii_case(name));
            let mut found: Vec<PartPlane> = datum.map(|datum| PartPlane::Datum { datum }).collect();
            for f in features {
                let plane = PartPlane::Feature { feature: f.id, normal: [0; 3] };
                let proof = reference_plane_proof(&part, r, id, &plane)?;
                found.push(PartPlane::Feature { feature: f.id, normal: proof.plane.normal() });
            }
            return match found.len() {
                1 => Ok(Entity::Plane(found.remove(0))),
                0 => {
                    Err(MateRefusal::new(RefusalCode::NoSuchFace, format!("{} has no reference plane {name}", r.path)))
                }
                _ => Err(MateRefusal::new(
                    RefusalCode::AmbiguousReference,
                    format!("{} has several reference planes named {name}", r.path),
                )),
            };
        }
        let body = part.body().ok_or_else(|| unavailable(format!("{} has no body", r.path)))?;
        let labels = part.face_labels(body);
        let found: Vec<FaceId> = match selector {
            FaceSelector::Picked(face) => {
                let repaired = part
                    .resolve_face(face, body)
                    .map_err(|repair| MateRefusal::new(RefusalCode::DanglingReference, repair.message))?;
                vec![repaired]
            }
            FaceSelector::Label(text) => labels
                .iter()
                .filter(|(_, l)| l.as_ref().is_some_and(|l| l.text.eq_ignore_ascii_case(text.trim())))
                .map(|(f, _)| *f)
                .collect(),
            FaceSelector::Plane(_) => unreachable!("a plane is found above"),
            FaceSelector::Generator { feature, generator, normal } => {
                let wanted = part.features().iter().find(|f| f.name == *feature).map(|f| f.id);
                labels
                    .iter()
                    .filter(|(_, l)| {
                        l.as_ref().is_some_and(|l| Some(l.feature) == wanted && generator.matches(&l.generator))
                    })
                    .map(|(f, _)| *f)
                    .filter(|f| {
                        normal.is_none_or(|n| {
                            plane_of(&part, r, id, *f).is_ok_and(|p| p.plane.normal().map(f64::from) == n)
                        })
                    })
                    .collect()
            }
        };
        let [face] = found[..] else {
            let (code, what) = if found.is_empty() {
                (RefusalCode::NoSuchFace, "finds no face")
            } else {
                (RefusalCode::AmbiguousReference, "finds several faces")
            };
            return Err(MateRefusal::new(code, format!("the selector {selector:?} {what} on {}", r.path)));
        };
        part.face_ref(face)
            .map(Entity::Face)
            .map_err(|e| MateRefusal::new(RefusalCode::AmbiguousReference, e.to_string()))
    }

    /// The reference of a root component that places a part.
    pub(crate) fn part_source(truth: &Assembly, component: ComponentId) -> Result<DocRef, MateRefusal> {
        let c = truth
            .component(component)
            .ok_or_else(|| MateRefusal::invalid(format!("the assembly has no component {component}")))?;
        let source = truth
            .source(c.source)
            .ok_or_else(|| MateRefusal::invalid(format!("component {component} has no source")))?;
        if let Some(configuration) = &source.document.configuration {
            return Err(configuration_refusal(format_args!("component {component}"), configuration));
        }
        if source.document.kind != DocKind::Part {
            return Err(MateRefusal::invalid(format!(
                "component {component} is a sub-assembly; mates reach the parts placed directly for now"
            )));
        }
        Ok(source.document.clone())
    }
}

fn plane_of(part: &PartDocument, r: &DocRef, document: DocumentId, face: FaceId) -> Result<PlaneProof, MateRefusal> {
    let body = part.body().ok_or_else(|| unavailable(format!("{} has no body", r.path)))?;
    let plane = part.kernel().face_support_plane(body, face).map_err(|e| {
        let code = match e {
            KernelError::Unsupported(_) => RefusalCode::UnsupportedPlane,
            _ => RefusalCode::ProofUnavailable,
        };
        MateRefusal::new(code, format!("{}: {e}", r.path))
    })?;
    Ok(PlaneProof {
        document,
        revision: r.pinned.clone(),
        geometry: part.geometry_hash(),
        plane,
        label: part.face_label(face).map(|l| l.text).unwrap_or_default(),
    })
}

fn rational_text(value: &BigRational) -> ExactRational {
    ExactRational { numerator: value.numer().to_string(), denominator: value.denom().to_string() }
}

/// The plane `u · p = offset` with `u` the signed axis `normal`.
fn axis_plane(normal: [i8; 3], offset: &BigRational) -> ExactAxisPlane {
    let at = normal.iter().position(|c| *c != 0).expect("a unit axis");
    let axis = [Axis::X, Axis::Y, Axis::Z][at];
    ExactAxisPlane { axis, positive: normal[at] > 0, offset: rational_text(offset) }
}

/// A reference plane's exact equation from its definition: a datum plane through the
/// origin, or a plane feature's source moved by its stored offset along the source's
/// normal. A plane on a face is refused by name.
fn reference_plane_proof(
    part: &PartDocument,
    r: &DocRef,
    document: DocumentId,
    plane: &PartPlane,
) -> Result<PlaneProof, MateRefusal> {
    let (normal, offset, label) = match plane {
        PartPlane::Datum { datum } => {
            (datum_normal(*datum), BigRational::from_integer(0.into()), part.datum_name(*datum).to_owned())
        }
        PartPlane::Feature { feature, .. } => {
            let (normal, offset) = feature_plane(part, *feature, 0)?;
            let label = part.feature(*feature).map(|f| f.name.clone()).unwrap_or_default();
            (normal, offset, label)
        }
    };
    Ok(PlaneProof {
        document,
        revision: r.pinned.clone(),
        geometry: part.geometry_hash(),
        plane: axis_plane(normal, &offset),
        label,
    })
}

/// A plane feature's normal and offset along it, followed through the planes it sits on.
fn feature_plane(part: &PartDocument, id: FeatureId, depth: usize) -> Result<([i8; 3], BigRational), MateRefusal> {
    let gone = |message: String| MateRefusal::new(RefusalCode::DanglingReference, message);
    let feature = part.feature(id).ok_or_else(|| gone(format!("reference plane feature {id} is gone")))?;
    let FeatureKind::ReferencePlane(source) = &feature.kind else {
        return Err(gone(format!("feature {id}, {}, is not a reference plane", feature.name)));
    };
    if feature.suppressed || part.reference_plane(id).is_none() {
        return Err(gone(format!("reference plane {} is not built", feature.name)));
    }
    if depth > 64 {
        return Err(MateRefusal::new(
            RefusalCode::UnsupportedPlane,
            format!("{} sits on too many planes", feature.name),
        ));
    }
    let exact = |offset: f64| {
        crate::mates::solver::exact(offset).ok_or_else(|| {
            MateRefusal::new(RefusalCode::UnsupportedPlane, format!("{}: the offset is not finite", feature.name))
        })
    };
    match source {
        SketchPlane::Datum { datum, offset } => Ok((datum_normal(*datum), exact(*offset)?)),
        SketchPlane::Feature { feature: under, offset } => {
            let (normal, base) = feature_plane(part, *under, depth + 1)?;
            Ok((normal, base + exact(*offset)?))
        }
        SketchPlane::Face { .. } | SketchPlane::FaceOffset { .. } => Err(MateRefusal::new(
            RefusalCode::UnsupportedPlane,
            format!(
                "{} is defined on a face; this version proves datum-based planes only, so mate the face itself",
                feature.name
            ),
        )),
    }
}
