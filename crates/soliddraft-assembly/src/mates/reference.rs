//! Finding faces, edges, vertices and reference planes, and proving what they support at a
//! pinned revision: a plane, an axis or a point.
//!
//! A proof belongs to a document, a revision and a face. It comes from the part rebuilt on
//! the kernel from that revision's bytes, never from a display copy, so a cached picture
//! can be shown but never proves a mate. Selectors are for commands: what is stored is the
//! part's persistent [`FaceRef`], or the datum or feature id of a reference plane.
//!
//! A reference plane is proved from its definition: a datum plane is exact, and a plane
//! feature is its datum or source plane moved by its stored offset, added in rationals. A
//! plane feature on a face is refused; the face itself can be mated. A face's axis or a
//! point comes from [`Kernel::entity_support`](soliddraft_kernel_api::Kernel::entity_support),
//! which reads the native definition exactly or refuses.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use num_rational::BigRational;
use num_traits::{Signed, Zero};
use serde::Serialize;
use soliddraft_document::{ConfigurationId, ContentHash, DocKind, DocRef, DocumentId};
use soliddraft_kernel_api::{
    Axis, CurveKind, EdgeId, ExactAxisPlane, ExactRadius, ExactRational, ExactSupport, FaceId, KernelError,
    SupportEntity, SupportSource, SurfaceKind, Topology, VertexId,
};
use soliddraft_model::{
    DatumPlane, Document as PartDocument, EdgeRef, FaceRef, FeatureId, FeatureKind, Generator, RepairNeeded,
    RepairReason, SketchPlane, VertexRef,
};

use crate::document::AssemblyDocument;
use crate::mates::{EndView, Entity, FaceEndpoint, MateRefusal, PartPlane, RefusalCode, StoredSupport, datum_normal};
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
    /// The persistent name a picked edge carries.
    PickedEdge(EdgeRef),
    /// The one edge between two faces, each named by its current label.
    Edge([String; 2]),
    /// The persistent name a picked vertex carries.
    PickedVertex(VertexRef),
    /// The one vertex where faces meet, each named by its current label.
    Vertex(Vec<String>),
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

/// What a face, edge, vertex or reference plane supports, proven on one revision of a part.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SupportProof {
    /// The part.
    pub document: DocumentId,
    /// The revision it was proven on.
    pub revision: ContentHash,
    /// That revision's geometry hash.
    pub geometry: ContentHash,
    /// The plane, axis or point in the part's frame.
    pub support: ExactSupport,
    /// The entity's label at that revision.
    pub label: String,
}

impl SupportProof {
    /// What a mate stores of it for the solver's status: the axis or point, as doubles.
    pub(crate) fn stored(&self) -> Option<StoredSupport> {
        use crate::mates::solver::{nearest, rational};
        let f = |c: &ExactRational| rational(c).map_or(f64::NAN, |r| nearest(&r));
        let r = |r: &Option<ExactRadius>| r.as_ref().map(|r| (f(&r.squared).sqrt(), r.outward));
        match &self.support {
            ExactSupport::Enclosed(support) => Some(StoredSupport::Enclosed { support: support.clone() }),
            ExactSupport::Plane(_) => None,
            ExactSupport::Axis(a) => {
                let (radius, outward) = r(&a.radius).map_or((None, None), |(v, o)| (Some(v), o));
                Some(StoredSupport::Axis {
                    direction: a.direction(),
                    through: a.through.each_ref().map(f),
                    radius,
                    outward,
                })
            }
            ExactSupport::Point(p) => {
                let (radius, outward) = r(&p.radius).map_or((None, None), |(v, o)| (Some(v), o));
                Some(StoredSupport::Point { at: p.at.each_ref().map(f), radius, outward })
            }
        }
    }
}

/// A profile proven on one revision of a part: the centre of a planar face's outline, a
/// rectangle, a regular polygon or a circle, or of a circular edge, with its plane.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProfileProof {
    /// The plane's outward normal, or a circular edge's direction.
    pub normal: [i8; 3],
    /// Where the plane lies along `normal`.
    pub offset: BigRational,
    /// The centre: a circle's, or the average of a polygon's corners.
    pub centre: [BigRational; 3],
    /// A polygon's number of sides; `None` for a circle.
    pub sides: Option<u32>,
    /// A polygon's edge directions, for lining two up: from corners the sketch placed, so
    /// read as doubles.
    pub edges: Vec<glam::DVec3>,
    /// The entity's label at that revision.
    pub label: String,
}

/// A straight slot proven on one revision of a part: two parallel walls closed by two round
/// ends.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SlotProof {
    /// The direction the round ends' axes run: the first end's cylinder's own.
    pub depth: [i8; 3],
    /// A point on each round end's axis, the one nearer the negative side of the run first.
    pub ends: [[BigRational; 3]; 2],
    /// The round ends' radius squared.
    pub radius_squared: BigRational,
    /// The face's label at that revision.
    pub label: String,
}

/// What a Profile Center or a Slot reads of a face or edge.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Shape {
    Profile(ProfileProof),
    Slot(SlotProof),
}

/// Which shape to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ShapeKind {
    Profile,
    Slot,
}

impl StoredSupport {
    /// The shape this stored support asks a proof to read, when it is one.
    pub(crate) fn shape(&self) -> Option<ShapeKind> {
        match self {
            StoredSupport::Profile { .. } => Some(ShapeKind::Profile),
            StoredSupport::Slot { .. } => Some(ShapeKind::Slot),
            StoredSupport::Enclosed { .. } | StoredSupport::Axis { .. } | StoredSupport::Point { .. } => None,
        }
    }
}

impl Shape {
    /// What a mate stores of it for the solver's status, as doubles.
    pub(crate) fn stored(&self) -> StoredSupport {
        use crate::mates::solver::nearest;
        match self {
            Shape::Profile(p) => {
                StoredSupport::Profile { normal: p.normal, centre: p.centre.each_ref().map(nearest), sides: p.sides }
            }
            Shape::Slot(s) => StoredSupport::Slot {
                depth: s.depth,
                ends: s.ends.each_ref().map(|e| e.each_ref().map(nearest)),
                radius: nearest(&s.radius_squared).sqrt(),
            },
        }
    }
}

/// What a proof is of on its part.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Proved {
    Face(FeatureId, Generator, u32),
    Datum(DatumPlane),
    Plane(FeatureId),
    /// An edge or a vertex, by its reference as JSON.
    Named(String),
}

type PlaneKey = (DocumentId, ContentHash, Proved);

/// Proofs and the rebuilt parts behind them. All of it is derived: dropping it changes
/// nothing but how soon the next proof comes.
#[derive(Default)]
pub(crate) struct Proofs {
    parts: HashMap<(DocumentId, ContentHash), Arc<Mutex<PartDocument>>>,
    supports: HashMap<PlaneKey, SupportProof>,
    shapes: HashMap<(DocumentId, ContentHash, ShapeKind, String), Shape>,
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

    /// The proof of the plane `entity` lies on, at the revision `r` pins. An axis or a point
    /// is refused as `unsupported_plane`.
    pub(crate) fn prove(&mut self, r: &DocRef, entity: &Entity) -> Result<PlaneProof, MateRefusal> {
        let proof = self.prove_support(r, entity, None)?;
        match proof.support {
            ExactSupport::Plane(plane) => Ok(PlaneProof {
                document: proof.document,
                revision: proof.revision,
                geometry: proof.geometry,
                plane,
                label: proof.label,
            }),
            _ => Err(MateRefusal::new(
                RefusalCode::UnsupportedPlane,
                format!("{}: {} is not planar", r.path, entity.describe()),
            )),
        }
    }

    /// The proof of what `entity` supports at the revision `r` pins: a plane, an axis or a
    /// point. `stored` is what the mate recorded when it was made; a proof whose kind or
    /// direction differs refuses as `dangling_reference`.
    pub(crate) fn prove_support(
        &mut self,
        r: &DocRef,
        entity: &Entity,
        stored: Option<&StoredSupport>,
    ) -> Result<SupportProof, MateRefusal> {
        let id = r.id.ok_or_else(|| unavailable("the reference has no document id".into()))?;
        let what = match entity {
            Entity::Face(face) => Proved::Face(face.feature, face.generator.clone(), face.ordinal),
            Entity::Plane(PartPlane::Datum { datum }) => Proved::Datum(*datum),
            Entity::Plane(PartPlane::Feature { feature, .. }) => Proved::Plane(*feature),
            Entity::Edge(edge) => Proved::Named(serde_json::to_string(edge).unwrap_or_default()),
            Entity::Vertex(vertex) => Proved::Named(serde_json::to_string(vertex).unwrap_or_default()),
        };
        let key = (id, r.pinned.clone(), what);
        let proof = match self.proofs.supports.get(&key) {
            Some(proof) => proof.clone(),
            None => {
                let part = self.proof_part(r)?;
                let part = part.lock().expect("the part lock");
                let proof = match entity {
                    Entity::Plane(plane) => {
                        let p = reference_plane_proof(&part, r, id, plane)?;
                        SupportProof {
                            document: p.document,
                            revision: p.revision,
                            geometry: p.geometry,
                            support: ExactSupport::Plane(p.plane),
                            label: p.label,
                        }
                    }
                    _ => {
                        let body = part.body().ok_or_else(|| unavailable(format!("{} has no body", r.path)))?;
                        let found = match entity {
                            Entity::Face(face) => part.resolve_face(face, body).map(SupportEntity::Face),
                            Entity::Edge(edge) => part.resolve_edge(edge, body).map(SupportEntity::Edge),
                            Entity::Vertex(vertex) => part.resolve_vertex(vertex, body).map(SupportEntity::Vertex),
                            Entity::Plane(_) => unreachable!("proved above"),
                        }
                        .map_err(repair_refusal)?;
                        support_of(&part, r, id, found)?
                    }
                };
                drop(part);
                self.proofs.supports.insert(key, proof.clone());
                proof
            }
        };
        if let Entity::Plane(PartPlane::Feature { normal, .. }) = entity
            && !matches!(&proof.support, ExactSupport::Plane(p) if p.normal() == *normal)
        {
            let message = format!("{} now faces another way than when it was mated", proof.label);
            return Err(MateRefusal::new(RefusalCode::DanglingReference, message));
        }
        let changed = match (stored, &proof.support) {
            (None, _) => false,
            (Some(StoredSupport::Axis { direction, .. }), ExactSupport::Axis(a)) => a.direction() != *direction,
            (Some(StoredSupport::Point { .. }), ExactSupport::Point(_)) => false,
            (Some(StoredSupport::Enclosed { support: old }), ExactSupport::Enclosed(new)) => {
                std::mem::discriminant(old.as_ref()) != std::mem::discriminant(new.as_ref())
            }
            (Some(_), _) => true,
        };
        if changed {
            let message = format!("{} now lies another way than when it was mated", entity.describe());
            return Err(MateRefusal::new(RefusalCode::DanglingReference, message));
        }
        Ok(proof)
    }

    /// The axis or point a mate stores for `entity`, proven at the revision `r` pins.
    pub(crate) fn stored_support(&mut self, r: &DocRef, entity: &Entity) -> Result<Option<StoredSupport>, MateRefusal> {
        Ok(self.prove_support(r, entity, None)?.stored())
    }

    /// The profile or slot `entity` bounds at the revision `r` pins. `stored` is what the
    /// mate recorded when it was made; one whose plane or slot now faces another way refuses
    /// as `dangling_reference`.
    pub(crate) fn prove_shape(
        &mut self,
        r: &DocRef,
        entity: &Entity,
        kind: ShapeKind,
        stored: Option<&StoredSupport>,
    ) -> Result<Shape, MateRefusal> {
        let id = r.id.ok_or_else(|| unavailable("the reference has no document id".into()))?;
        let key = (id, r.pinned.clone(), kind, format!("{entity:?}"));
        let shape = match self.proofs.shapes.get(&key) {
            Some(shape) => shape.clone(),
            None => {
                let part = self.proof_part(r)?;
                let part = part.lock().expect("the part lock");
                let body = part.body().ok_or_else(|| unavailable(format!("{} has no body", r.path)))?;
                let found = match entity {
                    Entity::Face(face) => part.resolve_face(face, body).map(SupportEntity::Face),
                    Entity::Edge(edge) => part.resolve_edge(edge, body).map(SupportEntity::Edge),
                    Entity::Vertex(vertex) => part.resolve_vertex(vertex, body).map(SupportEntity::Vertex),
                    Entity::Plane(_) => {
                        let what = match kind {
                            ShapeKind::Profile => "Profile Center reads a planar face or a circular edge",
                            ShapeKind::Slot => "Slot reads a wall or a round end of a straight slot",
                        };
                        return Err(MateRefusal::new(
                            RefusalCode::UnsupportedReference,
                            format!("{}: {what}, not a reference plane", r.path),
                        ));
                    }
                }
                .map_err(repair_refusal)?;
                let shape = match kind {
                    ShapeKind::Profile => Shape::Profile(profile_of(&part, r, found)?),
                    ShapeKind::Slot => Shape::Slot(slot_of(&part, r, found)?),
                };
                drop(part);
                self.proofs.shapes.insert(key, shape.clone());
                shape
            }
        };
        let changed = match (stored, &shape) {
            (Some(StoredSupport::Profile { normal, .. }), Shape::Profile(p)) => p.normal != *normal,
            (Some(StoredSupport::Slot { depth, .. }), Shape::Slot(s)) => s.depth != *depth,
            (None, _) => false,
            (Some(_), _) => true,
        };
        if changed {
            let message = format!("{} now lies another way than when it was mated", entity.describe());
            return Err(MateRefusal::new(RefusalCode::DanglingReference, message));
        }
        Ok(shape)
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
        let labelled = |text: &str| -> Result<FaceId, MateRefusal> {
            let found: Vec<FaceId> = labels
                .iter()
                .filter(|(_, l)| l.as_ref().is_some_and(|l| l.text.eq_ignore_ascii_case(text.trim())))
                .map(|(f, _)| *f)
                .collect();
            match found[..] {
                [face] => Ok(face),
                [] => Err(MateRefusal::new(RefusalCode::NoSuchFace, format!("{} has no face {text}", r.path))),
                _ => Err(MateRefusal::new(
                    RefusalCode::AmbiguousReference,
                    format!("{} has several faces labelled {text}", r.path),
                )),
            }
        };
        match selector {
            FaceSelector::PickedEdge(edge) => {
                part.resolve_edge(edge, body).map_err(repair_refusal)?;
                return Ok(Entity::Edge(edge.clone()));
            }
            FaceSelector::PickedVertex(vertex) => {
                part.resolve_vertex(vertex, body).map_err(repair_refusal)?;
                return Ok(Entity::Vertex(vertex.clone()));
            }
            FaceSelector::Edge(between) => {
                let faces = [labelled(&between[0])?, labelled(&between[1])?];
                let t = part.kernel().topology(body).map_err(|e| unavailable(e.to_string()))?;
                let found: Vec<EdgeId> = t
                    .edges
                    .iter()
                    .filter(|e| faces.iter().all(|f| e.faces.contains(f)) && faces[0] != faces[1])
                    .map(|e| e.id)
                    .collect();
                let [edge] = found[..] else {
                    let (code, what) = if found.is_empty() {
                        (RefusalCode::NoSuchFace, "no edge")
                    } else {
                        (RefusalCode::AmbiguousReference, "several edges")
                    };
                    let message = format!("{} has {what} between {} and {}", r.path, between[0], between[1]);
                    return Err(MateRefusal::new(code, message));
                };
                return part
                    .edge_ref(edge)
                    .map(Entity::Edge)
                    .map_err(|e| MateRefusal::new(RefusalCode::AmbiguousReference, e.to_string()));
            }
            FaceSelector::Vertex(around) => {
                let faces = around.iter().map(|l| labelled(l)).collect::<Result<Vec<_>, _>>()?;
                let t = part.kernel().topology(body).map_err(|e| unavailable(e.to_string()))?;
                let found: Vec<_> = t
                    .vertices
                    .iter()
                    .filter(|v| {
                        let touching: Vec<FaceId> =
                            v.edges.iter().filter_map(|e| t.edge(*e)).flat_map(|e| e.faces.iter().copied()).collect();
                        faces.iter().all(|f| touching.contains(f))
                    })
                    .map(|v| v.id)
                    .collect();
                let [vertex] = found[..] else {
                    let (code, what) = if found.is_empty() {
                        (RefusalCode::NoSuchFace, "no vertex")
                    } else {
                        (RefusalCode::AmbiguousReference, "several vertices")
                    };
                    let message = format!("{} has {what} where {} meet", r.path, around.join(", "));
                    return Err(MateRefusal::new(code, message));
                };
                return part
                    .vertex_ref(vertex)
                    .map(Entity::Vertex)
                    .map_err(|e| MateRefusal::new(RefusalCode::AmbiguousReference, e.to_string()));
            }
            _ => {}
        }
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
            FaceSelector::Plane(_)
            | FaceSelector::PickedEdge(_)
            | FaceSelector::Edge(_)
            | FaceSelector::PickedVertex(_)
            | FaceSelector::Vertex(_) => unreachable!("found above"),
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

/// The refusal for a reference that does not resolve to one entity.
fn repair_refusal(repair: RepairNeeded) -> MateRefusal {
    let code = match repair.reason {
        RepairReason::Ambiguous => RefusalCode::AmbiguousReference,
        RepairReason::Missing | RepairReason::Mismatch { .. } => RefusalCode::DanglingReference,
    };
    MateRefusal::new(code, repair.message)
}

/// What a face, edge or vertex supports, from the kernel's definition. A refused planar
/// face is `unsupported_plane`, as before; anything else the kernel refuses is
/// `unsupported_reference`.
fn support_of(
    part: &PartDocument,
    r: &DocRef,
    document: DocumentId,
    entity: SupportEntity,
) -> Result<SupportProof, MateRefusal> {
    let body = part.body().ok_or_else(|| unavailable(format!("{} has no body", r.path)))?;
    let label = match entity {
        SupportEntity::Face(face) => part.face_label(face).map(|l| l.text).unwrap_or_default(),
        SupportEntity::Edge(edge) => part.edge_ref(edge).map(|e| e.label).unwrap_or_default(),
        SupportEntity::Vertex(vertex) => part.vertex_ref(vertex).map(|v| v.label).unwrap_or_default(),
    };
    let support = part
        .kernel()
        .entity_support(body, entity)
        .or_else(|error| {
            if matches!(error, KernelError::Unsupported(_)) {
                part.kernel()
                    .entity_support_enclosed(body, entity)
                    .map(|support| ExactSupport::Enclosed(Box::new(support)))
            } else {
                Err(error)
            }
        })
        .map_err(|e| {
            let planar = match entity {
                SupportEntity::Face(face) => part
                    .kernel()
                    .topology(body)
                    .ok()
                    .and_then(|t| t.face(face).map(|f| f.surface == SurfaceKind::Plane))
                    .unwrap_or(false),
                _ => false,
            };
            let code = match e {
                KernelError::Unsupported(_) if planar => RefusalCode::UnsupportedPlane,
                KernelError::Unsupported(_) => RefusalCode::UnsupportedReference,
                _ => RefusalCode::ProofUnavailable,
            };
            let named = if label.is_empty() { String::new() } else { format!(", {label}") };
            MateRefusal::new(code, format!("{}{named}: {e}", r.path))
        })?;
    Ok(SupportProof { document, revision: r.pinned.clone(), geometry: part.geometry_hash(), support, label })
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

/// A rational read from the kernel, or the refusal `refuse` makes of what it was.
fn exact_of(value: &ExactRational, refuse: &impl Fn(String) -> MateRefusal) -> Result<BigRational, MateRefusal> {
    crate::mates::solver::rational(value).ok_or_else(|| refuse("a value is not rational".into()))
}

fn exact_point(
    p: &[ExactRational; 3],
    refuse: &impl Fn(String) -> MateRefusal,
) -> Result<[BigRational; 3], MateRefusal> {
    Ok([exact_of(&p[0], refuse)?, exact_of(&p[1], refuse)?, exact_of(&p[2], refuse)?])
}

fn dot3(a: &[BigRational; 3], b: &[BigRational; 3]) -> BigRational {
    &a[0] * &b[0] + &a[1] * &b[1] + &a[2] * &b[2]
}

fn signed_axis(n: [i8; 3]) -> [BigRational; 3] {
    n.map(|c| BigRational::from_integer(c.into()))
}

/// How far apart a polygon's side lengths or corner radii may be, relative to them, and the
/// polygon still be read as regular: its corners come from a sketch, placed by turns that
/// round.
const REGULAR_TOLERANCE: f64 = 1e-9;

/// A face's boundary split into its loops, each as its edges.
fn loops(t: &Topology, edges: &[EdgeId]) -> Vec<Vec<EdgeId>> {
    let mut out: Vec<(Vec<EdgeId>, Vec<VertexId>)> = Vec::new();
    for &e in edges {
        let vertices = t.edge(e).map(|e| e.vertices.clone()).unwrap_or_default();
        let joined: Vec<usize> = (0..out.len()).filter(|i| out[*i].1.iter().any(|v| vertices.contains(v))).collect();
        let mut merged = (vec![e], vertices);
        for i in joined.into_iter().rev() {
            let (es, vs) = out.remove(i);
            merged.0.extend(es);
            merged.1.extend(vs);
        }
        out.push(merged);
    }
    out.into_iter().map(|(es, _)| es).collect()
}

/// The loop that encloses the others: the one whose bounds reach furthest.
fn outer(t: &Topology, loops: Vec<Vec<EdgeId>>) -> Option<Vec<EdgeId>> {
    let reach = |l: &Vec<EdgeId>| {
        let boxes: Vec<_> = l.iter().filter_map(|e| t.edge(*e)).map(|e| e.bbox).collect();
        let lo = boxes.iter().fold(glam::DVec3::INFINITY, |m, b| m.min(glam::DVec3::new(b.min.x, b.min.y, b.min.z)));
        let hi =
            boxes.iter().fold(glam::DVec3::NEG_INFINITY, |m, b| m.max(glam::DVec3::new(b.max.x, b.max.y, b.max.z)));
        (hi - lo).length()
    };
    loops.into_iter().max_by(|a, b| reach(a).total_cmp(&reach(b)))
}

/// A closed chain of edges' corners in order round it, or `None` when it is not one chain.
fn corners(t: &Topology, chain: &[EdgeId]) -> Option<Vec<VertexId>> {
    let ends = |e: EdgeId| -> Option<[VertexId; 2]> {
        match t.edge(e)?.vertices[..] {
            [a, b] if a != b => Some([a, b]),
            _ => None,
        }
    };
    let first = ends(chain[0])?;
    let mut order = vec![first[0]];
    let (mut at, mut via) = (first[1], chain[0]);
    while at != first[0] {
        if order.len() > chain.len() {
            return None;
        }
        order.push(at);
        let next = chain.iter().copied().filter(|e| *e != via).find(|e| ends(*e).is_some_and(|v| v.contains(&at)))?;
        let [p, q] = ends(next)?;
        at = if p == at { q } else { p };
        via = next;
    }
    (order.len() == chain.len()).then_some(order)
}

/// The profile a planar face's outline or a circular edge makes, read exactly.
fn profile_of(part: &PartDocument, r: &DocRef, entity: SupportEntity) -> Result<ProfileProof, MateRefusal> {
    let body = part.body().ok_or_else(|| unavailable(format!("{} has no body", r.path)))?;
    let kernel = part.kernel();
    let label = match entity {
        SupportEntity::Face(face) => part.face_label(face).map(|l| l.text).unwrap_or_default(),
        SupportEntity::Edge(edge) => part.edge_ref(edge).map(|e| e.label).unwrap_or_default(),
        SupportEntity::Vertex(vertex) => part.vertex_ref(vertex).map(|v| v.label).unwrap_or_default(),
    };
    let named = if label.is_empty() { String::new() } else { format!(", {label}") };
    let refuse = |m: String| MateRefusal::new(RefusalCode::UnsupportedReference, format!("{}{named}: {m}", r.path));
    let support = |e: SupportEntity| {
        kernel.entity_support(body, e).map_err(|err| match err {
            KernelError::Unsupported(_) => refuse(err.to_string()),
            _ => unavailable(format!("{}{named}: {err}", r.path)),
        })
    };
    let circle = |edge: EdgeId| -> Result<([i8; 3], [BigRational; 3]), MateRefusal> {
        match support(SupportEntity::Edge(edge))? {
            ExactSupport::Axis(a) if a.source == SupportSource::Circle => {
                Ok((a.direction(), exact_point(&a.through, &refuse)?))
            }
            _ => Err(refuse("a straight edge does not say which face's outline it bounds; mate the face".into())),
        }
    };
    match entity {
        SupportEntity::Vertex(_) => {
            Err(refuse("Profile Center reads a planar face or a circular edge, not a vertex".into()))
        }
        SupportEntity::Edge(edge) => {
            let (normal, centre) = circle(edge)?;
            let offset = dot3(&signed_axis(normal), &centre);
            Ok(ProfileProof { normal, offset, centre, sides: None, edges: Vec::new(), label })
        }
        SupportEntity::Face(face) => {
            let ExactSupport::Plane(plane) = support(entity)? else {
                return Err(refuse("Profile Center reads a planar face or a circular edge".into()));
            };
            let (normal, offset) = (plane.normal(), exact_of(&plane.offset, &refuse)?);
            let t = kernel.topology(body).map_err(|e| unavailable(e.to_string()))?;
            let edges = t.face(face).map(|f| f.edges.clone()).unwrap_or_default();
            let outline = outer(&t, loops(&t, &edges)).ok_or_else(|| refuse("the face has no outline".into()))?;
            let kinds: Vec<CurveKind> = outline.iter().filter_map(|e| t.edge(*e)).map(|e| e.curve).collect();
            if kinds.iter().all(|k| *k == CurveKind::Circle) {
                // One circle, whole or in arcs: each arc has its centre and its radius.
                let mut found: Option<([BigRational; 3], ExactRational)> = None;
                for edge in &outline {
                    let ExactSupport::Axis(a) = support(SupportEntity::Edge(*edge))? else {
                        return Err(refuse("an edge of its outline is not a circle".into()));
                    };
                    if a.direction().map(i8::abs) != normal.map(i8::abs) {
                        return Err(refuse("its circle does not lie in its plane".into()));
                    }
                    let centre = exact_point(&a.through, &refuse)?;
                    let squared = a.radius.map(|r| r.squared).ok_or_else(|| refuse("a circle has no radius".into()))?;
                    match &found {
                        Some((c, r)) if *c != centre || *r != squared => {
                            return Err(refuse("its outline's arcs are not one circle".into()));
                        }
                        Some(_) => {}
                        None => found = Some((centre, squared)),
                    }
                }
                let (centre, _) = found.expect("an outline has an edge");
                return Ok(ProfileProof { normal, offset, centre, sides: None, edges: Vec::new(), label });
            }
            if kinds.iter().any(|k| *k != CurveKind::Line) {
                return Err(refuse(
                    "its outline mixes lines with arcs or curves: Profile Center reads a rectangle, a regular polygon or a circle, and a rounded or chamfered outline is none of them".into(),
                ));
            }
            let order = corners(&t, &outline).ok_or_else(|| refuse("its outline is not one closed chain".into()))?;
            let mut points = Vec::with_capacity(order.len());
            for v in order {
                match support(SupportEntity::Vertex(v))? {
                    ExactSupport::Point(p) => points.push(exact_point(&p.at, &refuse)?),
                    _ => return Err(refuse("a corner is not a point".into())),
                }
            }
            let n = points.len();
            let count = BigRational::from_integer((n as i64).into());
            let centre: [BigRational; 3] =
                std::array::from_fn(|k| points.iter().map(|p| p[k].clone()).sum::<BigRational>() / &count);
            let sides: Vec<[BigRational; 3]> =
                (0..n).map(|i| std::array::from_fn(|k| &points[(i + 1) % n][k] - &points[i][k])).collect();
            let rectangle = n == 4 && (0..n).all(|i| dot3(&sides[i], &sides[(i + 1) % n]).is_zero());
            let even = |values: Vec<BigRational>| {
                let values: Vec<f64> = values.iter().map(crate::mates::solver::nearest).collect();
                let (lo, hi) = values.iter().fold((f64::INFINITY, 0.0_f64), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
                lo > 0.0 && (hi - lo) <= REGULAR_TOLERANCE * hi
            };
            let regular = even(sides.iter().map(|e| dot3(e, e)).collect())
                && even(
                    points
                        .iter()
                        .map(|p| {
                            let d: [BigRational; 3] = std::array::from_fn(|k| &p[k] - &centre[k]);
                            dot3(&d, &d)
                        })
                        .collect(),
                );
            if !rectangle && !regular {
                return Err(refuse(format!(
                    "its outline has {n} straight sides but is neither a rectangle nor a regular polygon"
                )));
            }
            let edges = sides
                .iter()
                .map(|e| glam::DVec3::from_array(e.each_ref().map(crate::mates::solver::nearest)).normalize())
                .collect();
            Ok(ProfileProof { normal, offset, centre, sides: Some(n as u32), edges, label })
        }
    }
}

/// The straight slot a wall or round end belongs to: walking from it across edges that run
/// along the round ends' axes, two parallel walls and two round ends, read exactly.
fn slot_of(part: &PartDocument, r: &DocRef, entity: SupportEntity) -> Result<SlotProof, MateRefusal> {
    let body = part.body().ok_or_else(|| unavailable(format!("{} has no body", r.path)))?;
    let kernel = part.kernel();
    let SupportEntity::Face(face) = entity else {
        return Err(MateRefusal::new(
            RefusalCode::UnsupportedReference,
            format!("{}: Slot reads a wall or a round end of a straight slot, a face", r.path),
        ));
    };
    let label = part.face_label(face).map(|l| l.text).unwrap_or_default();
    let named = if label.is_empty() { String::new() } else { format!(", {label}") };
    let refuse = |m: String| MateRefusal::new(RefusalCode::UnsupportedReference, format!("{}{named}: {m}", r.path));
    let support = |e: SupportEntity| {
        kernel.entity_support(body, e).map_err(|err| match err {
            KernelError::Unsupported(_) => refuse(format!("the slot is not read exactly: {err}")),
            _ => unavailable(format!("{}{named}: {err}", r.path)),
        })
    };
    let t = kernel.topology(body).map_err(|e| unavailable(e.to_string()))?;
    let across = |f: FaceId| -> Vec<(EdgeId, FaceId)> {
        t.face(f)
            .into_iter()
            .flat_map(|info| info.edges.iter())
            .filter_map(|e| Some((*e, t.edge(*e)?.faces.iter().copied().find(|g| *g != f)?)))
            .collect()
    };
    let not_a_slot = || {
        refuse(
            "Slot reads a wall or a round end of a straight slot: two parallel walls closed by two round ends".into(),
        )
    };
    let round = |s: &ExactSupport| match s {
        ExactSupport::Axis(a) if a.source == SupportSource::Cylinder => Some(a.clone()),
        _ => None,
    };
    // The round ends' axis direction: the face's own when it is one, else a neighbour's.
    let first = support(SupportEntity::Face(face))?;
    let depth = match (&first, round(&first)) {
        (_, Some(a)) => a.direction(),
        (ExactSupport::Plane(p), None) => {
            let n = p.normal();
            let mut found = None;
            for (_, g) in across(face) {
                if t.face(g).is_some_and(|i| i.surface == SurfaceKind::Cylinder)
                    && let Some(a) = round(&support(SupportEntity::Face(g))?)
                    && a.direction().iter().zip(n).all(|(x, y)| x * y == 0)
                {
                    found = Some(a.direction());
                    break;
                }
            }
            found.ok_or_else(not_a_slot)?
        }
        _ => return Err(not_a_slot()),
    };
    let k_depth = depth.iter().position(|c| *c != 0).expect("a signed axis");
    // The faces joined to it by straight edges along the depth.
    let mut ring = vec![face];
    let mut i = 0;
    while i < ring.len() {
        for (e, g) in across(ring[i]) {
            if ring.contains(&g) || t.edge(e).is_none_or(|info| info.curve != CurveKind::Line) {
                continue;
            }
            if let ExactSupport::Axis(a) = support(SupportEntity::Edge(e))?
                && a.direction()[k_depth] != 0
            {
                ring.push(g);
            }
        }
        i += 1;
        if ring.len() > 8 {
            return Err(not_a_slot());
        }
    }
    let mut walls = Vec::new();
    let mut ends = Vec::new();
    for f in &ring {
        match support(SupportEntity::Face(*f))? {
            ExactSupport::Plane(p) => walls.push((p.normal(), exact_of(&p.offset, &refuse)?)),
            s => match round(&s) {
                Some(a) => ends.push(a),
                None => return Err(not_a_slot()),
            },
        }
    }
    if walls.is_empty() && ends.len() == 4 {
        return Err(refuse("an arc slot, whose walls are round, is not supported in this version".into()));
    }
    let ([(n0, o0), (n1, o1)], [e0, e1]) = (&walls[..], &ends[..]) else { return Err(not_a_slot()) };
    if n1.map(|c| -c) != *n0 || n0[k_depth] != 0 {
        return Err(refuse("the slot's walls are not two parallel faces across its round ends".into()));
    }
    let k_across = n0.iter().position(|c| *c != 0).expect("a signed axis");
    let k_along = 3 - k_depth - k_across;
    let squared = |a: &soliddraft_kernel_api::ExactAxis| {
        a.radius.as_ref().map(|r| exact_of(&r.squared, &refuse)).transpose()?.ok_or_else(not_a_slot)
    };
    let (r0, r1) = (squared(e0)?, squared(e1)?);
    let same_side = e0.radius.as_ref().map(|r| r.outward) == e1.radius.as_ref().map(|r| r.outward);
    if e0.direction()[k_depth] == 0 || e1.direction()[k_depth] == 0 || r0 != r1 || !same_side {
        return Err(refuse("the slot's round ends are not two equal half-cylinders along one direction".into()));
    }
    // Each wall's place across the run, from the part's origin.
    let place = |n: &[i8; 3], o: &BigRational| BigRational::from_integer(n[k_across].into()) * o;
    let (w0, w1) = (place(n0, o0), place(n1, o1));
    let four = BigRational::from_integer(4.into());
    let mid = (&w0 + &w1) / BigRational::from_integer(2.into());
    let (p0, p1) = (exact_point(&e0.through, &refuse)?, exact_point(&e1.through, &refuse)?);
    if (&w0 - &w1).abs() * (&w0 - &w1).abs() != four * &r0 || p0[k_across] != mid || p1[k_across] != mid {
        return Err(refuse("the slot's walls are not tangent to its round ends".into()));
    }
    if p0[k_along] == p1[k_along] {
        return Err(not_a_slot());
    }
    let (first, second, own) = if p0[k_along] < p1[k_along] { (p0, p1, e0) } else { (p1, p0, e1) };
    Ok(SlotProof { depth: own.direction(), ends: [first, second], radius_squared: r0, label })
}
