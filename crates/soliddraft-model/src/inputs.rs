//! What each feature reads, declared in one place.
//!
//! A feature continues one body and reads other things besides: the sketch it sweeps, a
//! reference plane, the feature that made a face it names, and for some features another
//! body, such as a Combine's tool or the body an extrusion's start face lies on. [`reads`]
//! describes these inputs for every kind of feature, and two rules use that description:
//!
//! - **Order.** A feature reads only features above it ([`misordered`]). A missing input is
//!   allowed: deleting a feature leaves its readers to report what broke and to be
//!   repaired, so an old file whose parent was deleted still opens. Bodies have no place
//!   in the order; a body input is read as it stands just before the feature.
//! - **Rebuild.** [`Document::resolve_reads`] finds the build each input stands for. The
//!   versions of the builds that can change while the feature's own input body stays the
//!   same complete its cache key (see [`crate::rebuild`]), and the bodies it finds are the
//!   ones the evaluator builds from, so lookup and evaluation read the same inputs.

use soliddraft_kernel_api::BodyId;

use crate::document::{Document, FeatureStatus};
use crate::evaluate::Built;
use crate::feature::{
    DraftFeature, EndCondition, ExtrudeFeature, ExtrudeStart, Feature, FeatureId, FeatureKind, MirrorFeature, PlaneRef,
    ProfileSource, PullRef, SketchFeature, SketchPlane,
};
use crate::reference::ReferenceSlot;

/// One input of a feature besides the body it continues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Read {
    /// A face, edge or vertex named on the body the feature builds on, made by this
    /// feature. The input body stands for that build, so this only orders the features.
    Name(FeatureId),
    /// The sketch the feature sweeps. `open` also takes open chains, as a thin extrusion
    /// does; otherwise the sketch needs a closed region.
    Profile {
        /// The sketch.
        sketch: FeatureId,
        /// Open chains are enough.
        open: bool,
    },
    /// A reference plane, which can move while the feature's input body stays the same.
    /// `slot` is where a missing plane stops the feature before anything is built; with
    /// none the evaluator reports it.
    Plane {
        /// The reference plane feature.
        feature: FeatureId,
        /// Where a missing plane is reported.
        slot: Option<ReferenceSlot>,
    },
    /// The feature a pattern repeats.
    Source(FeatureId),
    /// One profile of a loft: a sketch, or the feature that made a planar face.
    LoftProfile(FeatureId),
    /// A hole's position sketch.
    PositionSketch(FeatureId),
    /// The face a sketch or a reference plane lies on, made by this feature, on whichever
    /// body holds it. That body can change while the reader's own body stays the same.
    OnFace(FeatureId),
    /// The face or vertex an extrusion starts on, made by this feature. It may lie on
    /// another body, which the extrusion then reads.
    Start(FeatureId),
    /// Another body, by its stable key: a Combine's tool.
    Body(u32),
}

impl Read {
    /// The feature this input orders the reader after, if any.
    pub(crate) fn feature(self) -> Option<FeatureId> {
        match self {
            Read::Name(id)
            | Read::Profile { sketch: id, .. }
            | Read::Plane { feature: id, .. }
            | Read::Source(id)
            | Read::LoftProfile(id)
            | Read::PositionSketch(id)
            | Read::OnFace(id)
            | Read::Start(id) => Some(id),
            Read::Body(_) => None,
        }
    }
}

/// Everything `kind` reads besides the body it continues, in the order a rebuild
/// resolves them.
pub(crate) fn reads(kind: &FeatureKind) -> Vec<Read> {
    let mut out = Vec::new();
    match kind {
        FeatureKind::Combine { tool, .. } => out.push(Read::Body(*tool)),
        FeatureKind::Sketch(SketchFeature { plane, .. }) => match plane {
            SketchPlane::Feature { feature, .. } => {
                out.push(Read::Plane { feature: *feature, slot: Some(ReferenceSlot::SketchPlane) });
            }
            SketchPlane::Face { face } | SketchPlane::FaceOffset { face, .. } => out.push(Read::OnFace(face.feature)),
            SketchPlane::Datum { .. } | SketchPlane::Context { .. } => {}
        },
        FeatureKind::ReferencePlane(plane) => match plane {
            SketchPlane::Feature { feature, .. } => {
                out.push(Read::Plane { feature: *feature, slot: Some(ReferenceSlot::Plane) });
            }
            SketchPlane::Face { face } | SketchPlane::FaceOffset { face, .. } => out.push(Read::OnFace(face.feature)),
            SketchPlane::Datum { .. } | SketchPlane::Context { .. } => {}
        },
        FeatureKind::Revolve(r) => out.push(Read::Profile { sketch: r.sketch, open: false }),
        FeatureKind::Sweep(s) => out.push(Read::Profile { sketch: s.sketch, open: false }),
        FeatureKind::Loft(l) => out.extend(l.profiles.iter().map(|p| Read::LoftProfile(p.feature()))),
        FeatureKind::Extrude(ExtrudeFeature { profile, end, options, .. }) => {
            if let ProfileSource::Sketch { feature } = profile {
                out.push(Read::Profile { sketch: *feature, open: options.thin.is_some() });
            }
            if let EndCondition::UpToFace { face } = end {
                out.push(Read::Name(face.feature));
            }
            match &options.from {
                Some(ExtrudeStart::Surface { plane: PlaneRef::Face { face } }) => out.push(Read::Start(face.feature)),
                Some(ExtrudeStart::Vertex { vertex }) => {
                    let mut faces = vertex.faces.iter().map(|f| f.feature);
                    out.extend(faces.next().map(Read::Start));
                    out.extend(faces.map(Read::Name));
                }
                Some(ExtrudeStart::Surface { plane: PlaneRef::Feature { feature } }) => {
                    out.push(Read::Plane { feature: *feature, slot: None });
                }
                Some(ExtrudeStart::Surface { plane: PlaneRef::Datum { .. } } | ExtrudeStart::Offset { .. }) | None => {}
            }
            if let Some(EndCondition::UpToFace { face }) = &options.direction2 {
                out.push(Read::Name(face.feature));
            }
        }
        FeatureKind::Fillet(b) | FeatureKind::Chamfer(b) => {
            out.extend(b.edges.iter().flat_map(|e| e.faces.iter().map(|f| Read::Name(f.feature))));
        }
        FeatureKind::Hole(h) => {
            out.push(Read::Name(h.face.feature));
            out.extend(h.sketch.map(Read::PositionSketch));
            if let Some(wizard) = &h.wizard {
                use crate::hole_wizard::HoleEndCondition;
                match &wizard.end_condition {
                    HoleEndCondition::UpToSurface { face } | HoleEndCondition::OffsetFromSurface { face, .. } => {
                        out.push(Read::Name(face.feature));
                    }
                    HoleEndCondition::UpToVertex { vertex } => {
                        out.extend(vertex.faces.iter().map(|f| Read::Name(f.feature)))
                    }
                    HoleEndCondition::Blind { .. } | HoleEndCondition::ThroughAll | HoleEndCondition::UpToNext => {}
                }
            }
        }
        FeatureKind::Pattern(p) => out.extend(p.source.map(Read::Source)),
        FeatureKind::Thicken(t) => out.extend(t.faces.iter().map(|f| Read::Name(f.feature))),
        FeatureKind::Scan(_) | FeatureKind::Offset(_) | FeatureKind::ImportStep(_) => {}
        FeatureKind::Lattice(l) => {
            out.extend(l.radius_mm.iter().flat_map(|g| g.picked_faces().iter().map(|f| Read::Name(f.feature))));
        }
        FeatureKind::Gyroid(g) => out.extend(g.level.picked_faces().iter().map(|f| Read::Name(f.feature))),
        FeatureKind::Mirror(MirrorFeature { plane, .. }) => out.extend(plane_read(plane)),
        FeatureKind::Shell(s) => out.extend(s.open_faces.picked().iter().map(|f| Read::Name(f.feature))),
        FeatureKind::Draft(DraftFeature { faces, neutral, pull, .. }) => {
            out.extend(faces.iter().map(|f| Read::Name(f.feature)));
            out.extend(plane_read(neutral));
            if let Some(p) = pull {
                match p {
                    PullRef::Axis { .. } => {}
                    PullRef::Edge { edge } => out.extend(edge.faces.iter().map(|f| Read::Name(f.feature))),
                    PullRef::Face { face } => out.push(Read::Name(face.feature)),
                }
            }
        }
    }
    out
}

/// What a mirror plane or a draft's neutral plane reads.
fn plane_read(plane: &PlaneRef) -> Option<Read> {
    match plane {
        PlaneRef::Feature { feature } => Some(Read::Plane { feature: *feature, slot: Some(ReferenceSlot::Plane) }),
        PlaneRef::Face { face } => Some(Read::Name(face.feature)),
        PlaneRef::Datum { .. } => None,
    }
}

/// The features `kind` reads, sorted, each once.
pub(crate) fn features_read(kind: &FeatureKind) -> Vec<FeatureId> {
    let mut out: Vec<FeatureId> = reads(kind).into_iter().filter_map(Read::feature).collect();
    out.sort();
    out.dedup();
    out
}

/// The first feature, in tree order, that reads a feature at or below its own place,
/// with the feature it reads. A feature that reads one missing from the tree is in order.
pub(crate) fn misordered(features: &[Feature]) -> Option<(FeatureId, FeatureId)> {
    features.iter().enumerate().find_map(|(i, f)| {
        features_read(&f.kind).into_iter().find(|dep| features[i..].iter().any(|g| g.id == *dep)).map(|dep| (f.id, dep))
    })
}

/// A feature's inputs as one rebuild found them.
#[derive(Debug, Default)]
pub(crate) struct Resolved {
    /// The versions of the builds the feature reads that can change while its input body
    /// stays the same, in the order of [`reads`]: the rest of its cache key.
    pub versions: Vec<u64>,
    /// The other body an extrusion's start face or vertex lies on. None when it starts on
    /// its own body or names no face.
    pub start: Option<BodyId>,
    /// A Combine's tool body as it stands before the Combine.
    pub tool: Option<BodyId>,
}

impl Document {
    /// Finds the build every input of `f` stands for, as the tree stands before it, or the
    /// feature's result when an input stops it: a missing or unbuilt sketch, a missing
    /// reference plane or loft profile.
    pub(crate) fn resolve_reads(&self, f: &Feature, current: Option<BodyId>) -> Result<Resolved, Box<Built>> {
        let mut out = Resolved::default();
        let version = |id: FeatureId| self.built_index(id).map(|i| self.built[i].version);
        let owner = |body: BodyId| self.built.iter().find(|b| b.owned == Some(body)).map(|b| b.version);
        for read in reads(&f.kind) {
            match read {
                Read::Name(_) => {}
                Read::Profile { sketch, open } => out.versions.push(self.sketch_input(sketch, current, open)?.version),
                Read::Plane { feature, slot } => {
                    if let Some(slot) = slot
                        && self.reference_plane(feature).is_none()
                    {
                        let missing = self.missing_plane(feature, slot);
                        return Err(Box::new(Built::pass(FeatureStatus::RepairNeeded(vec![missing]), current)));
                    }
                    out.versions.extend(version(feature));
                }
                Read::Source(id) => out.versions.extend(version(id)),
                Read::LoftProfile(id) => {
                    let Some(index) = self.built_index(id) else {
                        return Err(Box::new(Built::failed(current, "A loft profile is missing. Select it again")));
                    };
                    if !self.built[index].status.is_ok() {
                        return Err(Box::new(Built::pass(FeatureStatus::Blocked(id), current)));
                    }
                    out.versions.push(self.built[index].version);
                }
                Read::PositionSketch(id) => {
                    let Some(index) = self.built_index(id) else {
                        return Err(Box::new(Built::failed(current, format!("its position sketch {id} is missing"))));
                    };
                    if !self.built[index].status.is_ok() {
                        return Err(Box::new(Built::pass(FeatureStatus::Blocked(id), current)));
                    }
                    out.versions.push(self.built[index].version);
                }
                Read::OnFace(maker) => {
                    let body = self.feature(maker).and_then(|g| self.body_at(g.body, self.built.len())).or(current);
                    out.versions.extend(body.and_then(owner));
                }
                Read::Start(maker) => {
                    out.start = self.other_body(f, maker);
                    out.versions.extend(out.start.and_then(owner));
                }
                Read::Body(key) => {
                    let end = self.index_of(f.id).unwrap_or(self.built.len());
                    out.tool = self.body_at(key, end);
                    out.versions.extend(out.tool.and_then(owner));
                }
            }
        }
        Ok(out)
    }

    /// The body that holds a face `maker` made, as it stands just before `f`, when that
    /// is not `f`'s own body: the body of the feature that made the face, or the body a
    /// Combine joined it to.
    fn other_body(&self, f: &Feature, maker: FeatureId) -> Option<BodyId> {
        let key = self.feature(maker)?.body;
        let end = self.index_of(f.id).unwrap_or(self.built.len());
        let key = self.surviving_key(key, end);
        if key == f.body {
            return None;
        }
        self.body_at(key, end)
    }
}
