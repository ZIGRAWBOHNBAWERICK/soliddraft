//! Feature evaluation and rebuild scheduling.

use std::collections::HashMap;
use std::sync::Arc;

use soliddraft_kernel_api::{
    Aabb, BodyId, BooleanOp, CurveKind, Draft, EdgeId, Extrude, ExtrudeLimit, FaceId, FacePoint, FaceRole, FaceSupport,
    Gyroid, HistoryRecord, Hole, HoleDepth, HoleHead, HolePlacement, HoleTip, ImportOptions, ImportStepRequest,
    ImportStepResult, KernelError, KernelResult, Lattice, Mirror, OpResult, OpTag, Origin, PatternLayout, Plane,
    Point2, Profile, SurfaceKind, Vec3, drawing::Curve3,
};

use crate::feature::{
    BlendFeature, Contour, DraftFeature, EndCondition, ExtrudeFeature, ExtrudeOp, ExtrudeStart, Feature, FeatureId,
    FeatureKind, GyroidFeature, HoleFeature, ImportStepFeature, LatticeFeature, MirrorFeature, PatternFeature,
    PatternSpec, PlaneRef, ProfileSource, PullRef, RevolveFeature, SketchFeature, SketchPlane,
};
use crate::rebuild::BuildCache;
use crate::reference::{self, Generator, NamedFace, ReferenceSlot, RepairNeeded, RepairReason, View};
use soliddraft_sketch::{EntityId, Region};

use crate::document::{BuiltSketch, Document, FeatureStatus};

/// How far one direction of an extrusion goes.
#[derive(Clone, Debug)]
enum Extent {
    /// A length along the direction.
    Length(f64),
    /// To a face the kernel finds on the body.
    Limit(ExtrudeLimit),
}

/// Which caps of one sweep of an extrusion keep their names.
#[derive(Clone, Copy, Debug)]
enum Caps {
    /// The only sweep: both caps.
    Both,
    /// The first of two sweeps from the start: its far cap only.
    End,
    /// The second of two sweeps from the start: its far cap, named the start cap.
    Second,
}

/// How far from parallel a start plane may be, as the sine of the angle between normals.
const PARALLEL: f64 = 1e-9;

/// Why an extrusion cannot start on a curved face. The pinned Catallax caps a straight
/// sweep only on planes; a start on a curved face connects here once the kernel can
/// sweep a profile from a curved face to the face moved along the direction. The words
/// name no kernel, so the form shows them as they are rather than a generic refusal.
const CURVED_START: &str = "an extrusion cannot start on a curved face yet: it starts only on a planar face \
     or a plane. Pick a planar face or a plane";

/// Why an extrusion from a tilted start cannot end at a face that crosses the start.
const LIMIT_CROSSES_START: &str = "the face the extrusion ends at crosses the start, so part of the profile \
     would begin past that face and not be extruded. Choose a face that lies beyond the whole start, or a \
     start parallel to the sketch";

/// Why an extrusion cannot start on a plane that holds its direction.
const START_HOLDS_DIRECTION: &str =
    "the start plane contains the extrusion direction, so the extrusion would never leave it";

/// One feature's rebuild result.
#[derive(Clone, Debug)]
pub(crate) struct Built {
    /// Body membership of this retained build, including while definitions are dirty.
    pub body_key: u32,
    pub consumed_tool: Option<u32>,
    pub status: FeatureStatus,
    /// The part body after this feature.
    pub output: Option<BodyId>,
    /// The body this feature made. The rebuild cache owns it and releases it.
    pub owned: Option<BodyId>,
    /// The names of the faces this feature generated, with the origins they stand for.
    pub names: Vec<NamedFace>,
    pub sketch: Option<BuiltSketch>,
    pub plane: Option<Plane>,
    /// Which kept build this is, so a feature that reads it can key on it. Zero for a
    /// result the cache does not keep.
    pub version: u64,
    /// Work used for the retained build, also retained across cache reuse.
    pub work_used: Option<u64>,
    pub import_record: Option<soliddraft_kernel_api::RepairRecord>,
}

impl Built {
    fn in_body(mut self, feature: &Feature) -> Self {
        self.body_key = feature.body;
        self.consumed_tool = match feature.kind {
            FeatureKind::Combine { tool, .. } if self.status.is_ok() => Some(tool),
            _ => None,
        };
        self
    }

    fn pass(status: FeatureStatus, current: Option<BodyId>) -> Built {
        Built {
            body_key: 0,
            consumed_tool: None,
            status,
            output: current,
            owned: None,
            names: Vec::new(),
            sketch: None,
            plane: None,
            version: 0,
            work_used: None,
            import_record: None,
        }
    }

    fn failed(current: Option<BodyId>, message: impl Into<String>) -> Built {
        Built::pass(FeatureStatus::Failed(message.into()), current)
    }
}

fn feature_error(error: KernelError) -> String {
    match error {
        KernelError::InvalidInput(message) => message,
        KernelError::Unsupported(message) if message.ends_with(" is coming soon.") => message,
        other => other.to_string(),
    }
}

impl Document {
    /// A copy of the part that shares this one's feature builds and bodies, so the copy
    /// builds only what changes in it. It has the same id, features and settings, and
    /// an empty undo history, as a part reopened from its file has. For dry runs, such
    /// as an assistant proposal tried before it is applied.
    pub fn fork(&self) -> Document {
        Document {
            id: self.id,
            name: self.name.clone(),
            features: self.features.clone(),
            values: self.values.clone(),
            observations: self.observations.clone(),
            configurations: self.configurations.clone(),
            properties: self.properties.clone(),
            table_edits: self.table_edits,
            datum_names: self.datum_names.clone(),
            rollback: self.rollback,
            held_rollback: self.held_rollback,
            next_id: self.next_id,
            next_body: self.next_body,
            materials: self.materials.clone(),
            pmi: self.pmi.clone(),
            settings: self.settings.clone(),
            view: self.view.clone(),
            kernel: self.kernel.clone(),
            built: self.built.clone(),
            dirty: self.dirty,
            cache: self.cache.clone(),
            undo: Vec::new(),
            redo: Vec::new(),
            cleared_redo: None,
            log: Vec::new(),
            last_solve: None,
            journal: None,
        }
    }

    /// Reuses a fork's successful builds on this kernel without changing the document.
    /// Exact input keys decide reuse even if the document changed after the dry run.
    pub fn reuse_builds_from(&mut self, source: &Document) {
        if self.id == source.id && Arc::ptr_eq(&self.kernel, &source.kernel) {
            self.cache.import(&source.cache);
            self.cache.trim(&self.built);
        }
    }

    /// Adopts a completed fork of the current definitions without evaluating them again.
    /// The caller must also reject completions for an obsolete application revision.
    /// History, identity and authored settings stay on this document. Imported bodies
    /// become live before trimming, so a small budget cannot evict them during adoption.
    pub fn rebuild_reusing(&mut self, source: &Document) -> Result<(), String> {
        if self.id != source.id || !Arc::ptr_eq(&self.kernel, &source.kernel) {
            return Err("the completed build belongs to another document or kernel".into());
        }
        if source.is_dirty() {
            return Err("the completed build still has unbuilt definitions".into());
        }
        let definition = crate::exact::bytes(&(&self.features, self.rollback));
        if definition.is_none() || definition != crate::exact::bytes(&(&source.features, source.rollback)) {
            return Err("the completed build does not match the current feature definitions".into());
        }
        let versions = self.cache.import(&source.cache);
        if source.built.iter().any(|built| built.version != 0 && !versions.contains_key(&built.version)) {
            self.cache.trim(&self.built);
            return Err("the completed build does not share this document's retained inputs".into());
        }
        let journal = self.journal_command(|| serde_json::json!({"op":"rebuild"}));
        journal.finish({
            self.built = source
                .built
                .iter()
                .map(|built| {
                    let mut built = built.clone();
                    if built.version != 0 {
                        built.version = versions[&built.version];
                    }
                    built
                })
                .collect();
            self.dirty = self.built.len();
            self.cache.trim(&self.built);
            self.refresh_tracked();
            Ok(())
        })
    }

    /// True while the document keeps `body`: a feature build it holds made it. A body
    /// no document keeps is released and does not come back.
    pub fn keeps(&self, body: BodyId) -> bool {
        self.cache.holds(body)
    }

    /// The bytes the kept feature builds retain, and the budget they are trimmed to.
    pub fn rebuild_cache_usage(&self) -> (u64, u64) {
        (self.cache.retained(), self.cache.budget())
    }

    /// Sets how many bytes of feature builds the document keeps for reuse. The builds of
    /// the current tree are always kept. A smaller budget applies at once.
    pub fn set_rebuild_budget(&mut self, bytes: u64) {
        self.cache.set_budget(bytes);
        self.cache.trim(&self.built);
    }

    /// True when the current definitions have not been rebuilt.
    pub fn is_dirty(&self) -> bool {
        self.dirty < self.features.len() || self.built.len() != self.features.len()
    }

    /// Rebuilds from the first stale feature to the end of the tree, reusing every
    /// feature build whose exact inputs the cache holds.
    pub fn rebuild(&mut self) {
        let journal = self.journal_command(|| serde_json::json!({"op":"rebuild"}));
        let _ = journal.finish({
            self.rebuild_inner();
            self.refresh_tracked();
            Ok::<_, String>(())
        });
    }

    fn rebuild_inner(&mut self) {
        let start = self.dirty.min(self.built.len());
        self.built.truncate(start);
        let active = self.active_count();
        let mut cache = std::mem::take(&mut self.cache);

        for i in start..self.features.len() {
            let f = self.features[i].clone();
            let current = self.body_at(f.body, i);
            let built = if i >= active {
                Built::pass(FeatureStatus::RolledBack, current)
            } else if f.suppressed {
                Built::pass(FeatureStatus::Suppressed, current)
            } else {
                self.build(&f, current, &mut cache)
            }
            .in_body(&f);
            self.built.push(built);
        }
        cache.trim(&self.built);
        self.cache = cache;
        self.dirty = self.built.len();
    }

    /// Marks every feature from `index` down as stale.
    pub(crate) fn touch(&mut self, index: usize) {
        self.dirty = self.dirty.min(index);
        self.stale_tracked("proof pending");
    }

    /// Refreshes name-dependent checks without retrying refused geometry or dirtying proof.
    pub(crate) fn refresh_names(&mut self) {
        let mut cache = std::mem::take(&mut self.cache);
        for i in 0..self.dirty.min(self.built.len()) {
            if i < self.active_count() && !self.features[i].suppressed {
                let current = self.body_at(self.features[i].body, i);
                self.built[i] = self.build_or_keep(&self.features[i], current, &mut cache, Some(&self.built[i]));
            }
        }
        self.cache = cache;
        self.refresh_tracked();
    }

    /// A feature's build: the kept one when the cache has its exact inputs, else a new
    /// one, kept. Checks whose messages name another feature run every time and are not
    /// kept, so a rename shows in them without building anything.
    fn build(&self, f: &Feature, current: Option<BodyId>, cache: &mut BuildCache) -> Built {
        self.build_or_keep(f, current, cache, None)
    }

    fn build_or_keep(
        &self,
        f: &Feature,
        current: Option<BodyId>,
        cache: &mut BuildCache,
        kept: Option<&Built>,
    ) -> Built {
        let named_plane = match &f.kind {
            FeatureKind::Sketch(SketchFeature { plane: SketchPlane::Feature { feature, .. }, .. }) => {
                Some((*feature, ReferenceSlot::SketchPlane))
            }
            FeatureKind::ReferencePlane(SketchPlane::Feature { feature, .. })
            | FeatureKind::Mirror(MirrorFeature { plane: PlaneRef::Feature { feature }, .. })
            | FeatureKind::Draft(DraftFeature { neutral: PlaneRef::Feature { feature }, .. }) => {
                Some((*feature, ReferenceSlot::Plane))
            }
            _ => None,
        };
        if let Some((id, slot)) = named_plane
            && self.reference_plane(id).is_none()
        {
            return Built::pass(FeatureStatus::RepairNeeded(vec![self.missing_plane(id, slot)]), current);
        }
        let profile = match &f.kind {
            FeatureKind::Extrude(ExtrudeFeature { profile: ProfileSource::Sketch { feature }, options, .. }) => {
                match self.sketch_input(*feature, current, options.thin.is_some()) {
                    Ok(input) => Some(input.version),
                    Err(refused) => return *refused,
                }
            }
            FeatureKind::Sweep(crate::SweepFeature { sketch: feature, .. })
            | FeatureKind::Revolve(RevolveFeature { sketch: feature, .. }) => {
                match self.profile_input(*feature, current) {
                    Ok(input) => Some(input.version),
                    Err(refused) => return *refused,
                }
            }
            FeatureKind::Pattern(p) if p.source.is_some() => {
                p.source.and_then(|id| self.built_index(id)).map(|i| self.built[i].version)
            }
            _ => named_plane.and_then(|(id, _)| self.built_index(id).map(|i| self.built[i].version)),
        };
        let mut profiles: Vec<_> = profile.into_iter().collect();
        if let FeatureKind::Loft(l) = &f.kind {
            for source in &l.profiles {
                let Some(index) = self.built_index(source.feature()) else {
                    return Built::failed(current, "A loft profile is missing. Select it again");
                };
                if !self.built[index].status.is_ok() {
                    return Built::pass(FeatureStatus::Blocked(source.feature()), current);
                }
                profiles.push(self.built[index].version);
            }
        }
        if let FeatureKind::Sketch(SketchFeature {
            plane: SketchPlane::Face { face } | SketchPlane::FaceOffset { face, .. },
            ..
        }) = &f.kind
        {
            let body =
                self.feature(face.feature).and_then(|feature| self.body_at(feature.body, self.built.len())).or(current);
            if let Some(owner) = body.and_then(|body| self.built.iter().find(|b| b.owned == Some(body))) {
                profiles.push(owner.version);
            }
        }
        // An extrusion that starts on another body's face or vertex reads that body.
        if let FeatureKind::Extrude(e) = &f.kind
            && let Some(body) = self.start_body(f, e.options.from.as_ref(), None)
            && let Some(owner) = self.built.iter().find(|b| b.owned == Some(body))
        {
            profiles.push(owner.version);
        }
        // One that starts on a named plane reads the plane, which can move while the
        // sketch, the extrusion and its input body stay as they were.
        if let FeatureKind::Extrude(e) = &f.kind
            && let Some(ExtrudeStart::Surface { plane: PlaneRef::Feature { feature } }) = &e.options.from
            && let Some(index) = self.built_index(*feature)
        {
            profiles.push(self.built[index].version);
        }
        if let FeatureKind::Hole(h) = &f.kind
            && let Some(sketch_id) = h.sketch
        {
            let Some(index) = self.built_index(sketch_id) else {
                return Built::failed(current, format!("its position sketch {sketch_id} is missing"));
            };
            if !self.built[index].status.is_ok() {
                return Built::pass(FeatureStatus::Blocked(sketch_id), current);
            }
            profiles.push(self.built[index].version);
        }
        if let FeatureKind::Combine { tool, .. } = f.kind {
            let end = self.index_of(f.id).unwrap_or(self.built.len());
            if let Some(body) = self.body_at(tool, end)
                && let Some(owner) = self.built.iter().find(|b| b.owned == Some(body))
            {
                profiles.push(owner.version);
            }
        }
        if let Some(kept) = kept {
            return kept.clone();
        }
        let key = cache.key(f, current, profiles);
        if let Some(built) = cache.get(&key) {
            return built;
        }
        let mut result = None;
        let work_used = self.kernel.measure_work(&mut || {
            result = Some(match &f.kind {
                FeatureKind::Combine { operation, tool } => {
                    let end = self.index_of(f.id).unwrap_or(self.built.len());
                    match (current, self.body_at(*tool, end)) {
                        _ if f.body == *tool => Built::failed(current, "Choose two different bodies to combine."),
                        (Some(target), Some(tool)) => {
                            let result = self.kernel.boolean(&OpTag::new(format!("{}.combine", f.id)), *operation, target, tool).map_err(|error| {
                                let message = match error {
                                    KernelError::InvalidInput(message) => message,
                                    KernelError::Unsupported(message) if message == "this would leave nothing of the part" => "This combine would leave no solid body.".into(),
                                    KernelError::Unsupported(_) => "Combine for these bodies is coming soon.".into(),
                                    _ => "These bodies cannot be combined with this operation. Check that their boundaries are closed and overlap.".into(),
                                };
                                KernelError::InvalidInput(message)
                            });
                            self.finish(result, current, |_| Vec::new())
                        }
                        _ => Built::failed(current, "Combine needs two available solid bodies."),
                    }
                }
                FeatureKind::ReferencePlane(p) => match self.sketch_plane_of(p, current) {
                    Ok((plane, _)) => {
                        let mut built = Built::pass(FeatureStatus::Ok, current);
                        built.plane = Some(plane);
                        built
                    }
                    Err(mut built) => {
                        if let FeatureStatus::RepairNeeded(items) = &mut built.status {
                            for item in items {
                                item.slot = ReferenceSlot::Plane;
                            }
                        }
                        *built
                    }
                },
                FeatureKind::Sketch(s) => self.build_sketch(s, current),
                FeatureKind::Extrude(e) => self.build_extrude(f, e, current),
                FeatureKind::Revolve(r) => self.build_revolve(f, r, current),
                FeatureKind::Sweep(s) => self
                    .build_profile_tool(f, s.sketch, current, |tag, profile| self.kernel.sweep(tag, profile, &s.path)),
                FeatureKind::Loft(l) => self.build_loft(f, l, current),
                FeatureKind::Fillet(b) => self.build_blend(f, b, false, current),
                FeatureKind::Chamfer(b) => self.build_blend(f, b, true, current),
                FeatureKind::Hole(h) => self.build_hole(f, h, current),
                FeatureKind::Pattern(p) => self.build_pattern(f, p, current),
                FeatureKind::Mirror(m) => self.build_mirror(f, m, current),
                FeatureKind::Draft(d) => self.build_draft(f, d, current),
                FeatureKind::Offset(o) => self.build_offset(f, o, current),
                FeatureKind::Thicken(t) => self.build_thicken(f, t, current),
                FeatureKind::Shell(s) => self.build_shell(f, s, current),
                FeatureKind::ImportStep(i) => self.build_import(f, i, current),
                FeatureKind::Scan(scan) => {
                    if crate::sha256_hex(&scan.source.bytes) != scan.source.sha256 {
                        Built::failed(current, "The saved scan does not match its original file.")
                    } else if let Some(fit) = &scan.fit {
                        if current.is_some() {
                            Built::failed(current, "A fitted scan must come before the part's solid features.")
                        } else {
                            let fingerprint =
                                crate::sha256_hex(&serde_json::to_vec(&fit.recipe).expect("recipe serializes"))
                                    .into_boxed_str();
                            self.finish(
                                self.kernel.build_scan_fit(&OpTag::new(format!("{}.scan", f.id)), &scan.source, fit),
                                current,
                                |r| {
                                    r.record
                                        .face_roles
                                        .iter()
                                        .enumerate()
                                        .map(|(i, o)| NamedFace {
                                            origin: Origin { tag: r.record.tag.clone(), role: o.role.clone() },
                                            generator: Generator::Imported { sha256: fingerprint.clone() },
                                            ordinal: i as u32,
                                        })
                                        .collect()
                                },
                            )
                        }
                    } else {
                        match self.kernel.scan_reference(&scan.source) {
                            Ok(_) => Built::pass(FeatureStatus::Ok, current),
                            Err(e) => Built::failed(current, feature_error(e)),
                        }
                    }
                }
                FeatureKind::Lattice(l) => self.build_lattice(f, l, current),
                FeatureKind::Gyroid(g) => self.build_gyroid(f, g, current),
            });
        });
        let mut built = result.expect("work observation runs the build");
        match work_used {
            Ok(work) => built.work_used = work,
            Err(error) if built.status.is_ok() => {
                if let Some(body) = built.owned {
                    let _ = self.kernel.release(body);
                }
                built = Built::failed(current, feature_error(error));
            }
            Err(_) => {}
        }
        cache.insert(key, built.in_body(f), &self.kernel)
    }

    /// The built sketch an extrusion sweeps, or the extrusion's result when that sketch
    /// is missing, not built or has no closed region.
    fn profile_input(&self, sketch: FeatureId, current: Option<BodyId>) -> Result<&Built, Box<Built>> {
        self.sketch_input(sketch, current, false)
    }

    /// The built sketch a feature reads, with or without closed regions: a thin
    /// extrusion follows open chains as well (`open`).
    fn sketch_input(&self, sketch: FeatureId, current: Option<BodyId>, open: bool) -> Result<&Built, Box<Built>> {
        let sketch_name = || self.feature(sketch).map(|s| s.name.clone()).unwrap_or_else(|| sketch.to_string());
        let failed = |message: String| Err(Box::new(Built::failed(current, message)));
        let Some(input) = self.built_index(sketch).map(|i| &self.built[i]) else {
            return failed(format!("its sketch {} is missing", sketch_name()));
        };
        let Some(sk) = input.sketch.as_ref() else {
            if input.status.is_ok() {
                return failed(format!("{} is not a sketch", sketch_name()));
            }
            return Err(Box::new(Built::pass(FeatureStatus::Blocked(sketch), current)));
        };
        match &sk.regions {
            _ if open => Ok(input),
            Ok(r) if !r.is_empty() => Ok(input),
            Ok(_) => failed(format!("{} has no closed region", sketch_name())),
            Err(err) => failed(format!("{} has no closed region: {err}", sketch_name())),
        }
    }

    fn sketch_plane_of(
        &self,
        source: &SketchPlane,
        current: Option<BodyId>,
    ) -> Result<(Plane, Option<FaceSupport>), Box<Built>> {
        if !source.offset().is_finite() {
            return Err(Box::new(Built::failed(current, "plane offset must be finite")));
        }
        let plane = match source {
            SketchPlane::Feature { feature, offset } => {
                let plane = self.reference_plane(*feature).ok_or_else(|| {
                    Box::new(Built::pass(
                        FeatureStatus::RepairNeeded(vec![self.missing_plane(*feature, ReferenceSlot::SketchPlane)]),
                        current,
                    ))
                })?;
                (plane.with_origin(plane.origin + plane.normal * *offset), None)
            }
            SketchPlane::Datum { datum, offset } => {
                let p = datum.plane();
                (p.with_origin(p.origin + p.normal * *offset), None)
            }
            SketchPlane::Face { face } | SketchPlane::FaceOffset { face, .. } => {
                let offset = source.offset();
                let repair = |reason, message: String, candidates| {
                    let r = RepairNeeded { slot: ReferenceSlot::SketchPlane, reason, message, candidates };
                    Err(Box::new(Built::pass(FeatureStatus::RepairNeeded(vec![r]), current)))
                };
                let source_body = self
                    .feature(face.feature)
                    .and_then(|feature| self.body_at(feature.body, self.built.len()))
                    .or(current);
                let Some(body) = source_body else {
                    return repair(RepairReason::Missing, "there is no part body to sketch on".into(), vec![]);
                };
                // A sketch plane needs no mesh of the body before it unless this cannot answer.
                if let Ok(carriers) = self.kernel.face_carriers(body)
                    && let Some((face, normal, point)) = reference::carrier_plane(face, &carriers, &self.name_map())
                {
                    let p = plane_on_face(normal, point);
                    return Ok((p.with_origin(p.origin + p.normal * offset), Some(FaceSupport { body, face, offset })));
                }
                let t = match self.view_of(body) {
                    Ok(v) => v,
                    Err(e) => return Err(Box::new(Built::failed(current, e.to_string()))),
                };
                let id = match reference::resolve_plane(face, &View { topology: &t }, &self.name_map()) {
                    Ok(id) => id,
                    Err(u) => {
                        return Err(Box::new(Built::pass(
                            FeatureStatus::RepairNeeded(vec![u.into_repair(ReferenceSlot::SketchPlane)]),
                            current,
                        )));
                    }
                };
                let info = t.face(id).expect("resolved faces are in the topology");
                let Some(normal) = info.normal else {
                    let detail = "the face is not planar".to_string();
                    return repair(RepairReason::Mismatch { detail: detail.clone() }, detail, vec![]);
                };
                let p = plane_on_face(normal, info.centroid);
                (p.with_origin(p.origin + p.normal * offset), Some(FaceSupport { body, face: id, offset }))
            }
        };
        Ok(plane)
    }

    fn missing_plane(&self, feature: FeatureId, slot: ReferenceSlot) -> RepairNeeded {
        let name = self.feature(feature).map(|f| f.name.as_str()).unwrap_or("the selected plane");
        RepairNeeded {
            slot,
            reason: RepairReason::Missing,
            message: format!("{name} is missing or unavailable; choose a reference plane"),
            candidates: vec![],
        }
    }

    fn build_sketch(&self, s: &SketchFeature, current: Option<BodyId>) -> Built {
        let (plane, support) = match self.sketch_plane_of(&s.plane, current) {
            Ok(plane) => plane,
            Err(built) => return *built,
        };
        let regions = s.sketch.regions().map_err(|e| e.to_string());
        Built {
            body_key: 0,
            consumed_tool: None,
            status: FeatureStatus::Ok,
            output: current,
            owned: None,
            names: Vec::new(),
            sketch: Some(BuiltSketch { plane, on_face: support.is_some_and(|s| s.offset == 0.0), support, regions }),
            plane: None,
            version: 0,
            work_used: None,
            import_record: None,
        }
    }

    fn build_revolve(&self, f: &Feature, r: &RevolveFeature, current: Option<BodyId>) -> Built {
        if !r.angle_deg.is_finite()
            || r.angle_deg <= 0.0
            || r.angle_deg > 360.0
            || !r.axis_origin.iter().chain(&r.axis_direction).all(|v| v.is_finite())
            || r.axis_direction.iter().all(|v| *v == 0.0)
        {
            return Built::failed(current, "revolve needs a finite axis, nonzero direction and angle in (0, 360]");
        }
        self.build_profile_tool(f, r.sketch, current, |tag, profile| {
            self.kernel.revolve(
                tag,
                profile,
                &soliddraft_kernel_api::Revolve {
                    axis_origin: r.axis_origin.into(),
                    axis_direction: r.axis_direction.into(),
                    angle_deg: r.angle_deg,
                },
            )
        })
    }

    fn build_profile_tool(
        &self,
        f: &Feature,
        sketch: FeatureId,
        current: Option<BodyId>,
        tool: impl Fn(&OpTag, BodyId) -> KernelResult<soliddraft_kernel_api::OpResult>,
    ) -> Built {
        let input = match self.profile_input(sketch, current) {
            Ok(input) => input,
            Err(refused) => return *refused,
        };
        let sk = input.sketch.as_ref().expect("a profile input is a built sketch");
        let regions = match &sk.regions {
            Ok(regions) if !regions.is_empty() => regions,
            _ => return Built::failed(current, "Select a closed sketch region"),
        };
        let chain = matches!(&f.kind, FeatureKind::Sweep(s) if s.path.segments.len() > 1);
        let mut mine = None;
        let mut faces = Vec::new();
        let mut run = || -> KernelResult<Option<BodyId>> {
            let k = &self.kernel;
            let mut body = current;
            for (i, region) in regions.iter().enumerate() {
                let tag = |step: &str| OpTag::new(format!("{}.r{i}.{step}", f.id));
                let sheet = k.create_body_from_profile(&tag("profile"), &region.to_profile(sk.plane))?;
                let tool = tool(&tag("sweep"), sheet.body);
                let _ = k.release(sheet.body);
                let tool = tool?;
                for origin in &tool.record.face_roles {
                    let generator = match &origin.role {
                        FaceRole::StartCap => Some(Generator::StartCap),
                        FaceRole::EndCap => Some(Generator::EndCap),
                        FaceRole::Side { loop_index, segment, path } => {
                            // A one-piece path's faces keep the names saved before chains.
                            let path = path.as_deref().filter(|_| chain).map(Box::from);
                            region
                                .entity(*loop_index as usize, *segment as usize)
                                .map(|entity| Generator::Side { entity, path })
                        }
                        FaceRole::Envelope { path } => Some(Generator::PathJoin { path: path.as_str().into() }),
                        _ => None,
                    };
                    if let Some(generator) = generator {
                        faces.push((Origin { tag: tool.record.tag.clone(), role: origin.role.clone() }, generator, i));
                    }
                }
                let next = if let Some(previous) = body {
                    let joined = k.boolean(&tag("union"), BooleanOp::Union, previous, tool.body);
                    let _ = k.release(tool.body);
                    joined?.body
                } else {
                    tool.body
                };
                if let Some(previous) = mine.replace(next) {
                    let _ = k.release(previous);
                }
                body = Some(next);
            }
            Ok(body)
        };
        match run() {
            Ok(output) => Built {
                body_key: 0,
                consumed_tool: None,
                status: FeatureStatus::Ok,
                output,
                owned: mine,
                names: number(faces),
                sketch: None,
                plane: None,
                version: 0,
                work_used: None,
                import_record: None,
            },
            Err(error) => {
                if let Some(body) = mine {
                    let _ = self.kernel.release(body);
                }
                let message = if matches!(f.kind, FeatureKind::Sweep(_)) {
                    match error {
                        soliddraft_kernel_api::KernelError::InvalidInput(message) => message,
                        soliddraft_kernel_api::KernelError::Profile(error) => error.to_string(),
                        soliddraft_kernel_api::KernelError::Unsupported(message)
                            if message.ends_with(" is coming soon.") =>
                        {
                            message
                        }
                        _ => "This sweep could not be built from this profile and path".into(),
                    }
                } else {
                    feature_error(error)
                };
                Built::failed(current, message)
            }
        }
    }

    fn build_loft(&self, f: &Feature, loft: &crate::LoftFeature, current: Option<BodyId>) -> Built {
        if loft.profiles.len() < 2 {
            return Built::failed(current, "Loft needs at least two profiles in order");
        }
        let mut made = Vec::new();
        let mut names = Vec::new();
        let run = (|| -> KernelResult<OpResult> {
            for (index, source) in loft.profiles.iter().enumerate() {
                let tag = OpTag::new(format!("{}.profile{index}", f.id));
                let profile = match source {
                    crate::LoftProfile::Sketch { feature } => {
                        let input = self.profile_input(*feature, current).map_err(|_| {
                            KernelError::InvalidInput("Each loft sketch needs one closed region".into())
                        })?;
                        let sk = input.sketch.as_ref().expect("profile input");
                        let regions = sk.regions.as_ref().expect("checked profile");
                        if regions.len() != 1 {
                            return Err(KernelError::InvalidInput("Each loft sketch needs one closed region".into()));
                        }
                        self.kernel.create_body_from_profile(&tag, &regions[0].to_profile(sk.plane))?
                    }
                    crate::LoftProfile::Face { face } => {
                        let body = current
                            .ok_or_else(|| KernelError::InvalidInput("Select a planar face of the part".into()))?;
                        let selected = self.resolve_face(face, body).map_err(|_| {
                            KernelError::InvalidInput("A loft face is missing or split. Select it again".into())
                        })?;
                        self.kernel.create_profile_from_face(&tag, body, selected)?
                    }
                };
                made.push(profile.body);
            }
            let tool = self.kernel.loft(&OpTag::new(format!("{}.loft", f.id)), &made, loft.shape)?;
            names = number(
                tool.record
                    .face_roles
                    .iter()
                    .enumerate()
                    .map(|(i, face)| {
                        let generator = match face.role {
                            FaceRole::StartCap => Generator::StartCap,
                            FaceRole::EndCap => Generator::EndCap,
                            _ => Generator::Other,
                        };
                        (Origin { tag: tool.record.tag.clone(), role: face.role.clone() }, generator, i)
                    })
                    .collect(),
            );
            if let Some(body) = current {
                made.push(tool.body);
                self.kernel.boolean(&OpTag::new(format!("{}.union", f.id)), BooleanOp::Union, body, tool.body)
            } else {
                Ok(tool)
            }
        })();
        for body in made {
            let _ = self.kernel.release(body);
        }
        match run {
            Ok(result) => self.finish(Ok(result), current, |_| names),
            Err(error) => Built::failed(
                current,
                match error {
                    KernelError::InvalidInput(message) | KernelError::Unsupported(message) => message,
                    _ => {
                        "Loft could not join these profiles to the part. Try simpler profiles that meet the part".into()
                    }
                },
            ),
        }
    }

    fn build_extrude(&self, f: &Feature, e: &ExtrudeFeature, current: Option<BodyId>) -> Built {
        let shape_sketch;
        let sk = match &e.profile {
            ProfileSource::Sketch { feature } => match self.sketch_input(*feature, current, e.options.thin.is_some()) {
                Ok(input) => input.sketch.as_ref().expect("a profile input is a built sketch"),
                Err(refused) => return *refused,
            },
            ProfileSource::Shape { datum, shape } => {
                let regions = shape.sketch().and_then(|s| s.regions().map_err(|e| e.to_string()));
                shape_sketch = BuiltSketch { plane: datum.plane(), on_face: false, support: None, regions };
                &shape_sketch
            }
        };
        // A thin wall follows the sketch's curves, so it needs no closed region.
        let no_regions = Vec::new();
        let regions = match &sk.regions {
            _ if e.options.thin.is_some() => sk.regions.as_ref().unwrap_or(&no_regions),
            Ok(r) if !r.is_empty() => r,
            Ok(_) => return Built::failed(current, "the built-in profile has no closed region"),
            Err(err) => return Built::failed(current, format!("the built-in profile has no closed region: {err}")),
        };
        if e.options.thin.is_none()
            && let ProfileSource::Sketch { feature } = &e.profile
            && let Some(source) = self.feature(*feature)
            && let FeatureKind::Sketch(source) = &source.kind
            && let Some(message) = source.sketch.text_extrusion_notice(regions.len())
        {
            return Built::failed(current, message);
        }
        let sketch_name = || match &e.profile {
            ProfileSource::Sketch { feature } => {
                self.feature(*feature).map_or_else(|| feature.to_string(), |f| f.name.clone())
            }
            ProfileSource::Shape { .. } => "the built-in profile".to_string(),
        };
        // The regions to sweep, each with the names its curves give the faces it makes.
        let tools: Vec<(Region, HashMap<EntityId, Generator>)> = match (&e.options.thin, e.options.contours.is_empty())
        {
            (Some(_), false) => {
                return Built::failed(current, "choose either a thin feature or selected contours, not both");
            }
            (Some(thin), true) => {
                let source = match &e.profile {
                    ProfileSource::Sketch { feature } => match self.feature(*feature).map(|f| &f.kind) {
                        Some(FeatureKind::Sketch(s)) => Ok(s.sketch.clone()),
                        _ => Err(format!("its sketch {} is missing", sketch_name())),
                    },
                    ProfileSource::Shape { shape, .. } => shape.sketch(),
                };
                match source.and_then(|s| crate::thin::walls(&s, thin)) {
                    Ok(walls) => walls,
                    Err(message) => return Built::failed(current, message),
                }
            }
            (None, true) => regions.iter().map(|r| (r.clone(), HashMap::new())).collect(),
            (None, false) => {
                let mut chosen = Vec::new();
                for contour in &e.options.contours {
                    match regions.iter().find(|r| Contour::of(r) == *contour) {
                        Some(r) => chosen.push((r.clone(), HashMap::new())),
                        None => {
                            return Built::failed(
                                current,
                                format!(
                                    "a selected contour is no longer in {}; choose the contours again",
                                    sketch_name()
                                ),
                            );
                        }
                    }
                }
                chosen
            }
        };
        let normal = sk.plane.normal;
        let sense = if e.reverse { -1.0 } else { 1.0 };
        let dir = normal * sense;
        // A start plane that is not parallel to the sketch. The extrusion starts on it
        // over the whole profile, and `shift` stays at the sketch plane.
        let mut tilted: Option<Plane> = None;
        // The body the start face or vertex lies on, which may be another body when
        // Merge result is off.
        let start_body = self.start_body(f, e.options.from.as_ref(), current);
        // Where the extrusion starts, along the sketch normal from the sketch plane.
        let shift = match &e.options.from {
            None => 0.0,
            Some(ExtrudeStart::Offset { distance, reverse }) => {
                if !(distance.is_finite() && *distance > 0.0 && *distance <= 100_000.0) {
                    return Built::failed(current, "the start offset must be a positive length");
                }
                if *reverse { -distance * sense } else { distance * sense }
            }
            Some(ExtrudeStart::Surface { plane }) => {
                if let (PlaneRef::Face { face }, Some(body)) = (plane, start_body)
                    && let Ok(id) = self.resolve_plane_face(face, body)
                    && self.kernel.topology(body).ok().is_some_and(|t| t.face(id).is_some_and(|f| f.normal.is_none()))
                {
                    return Built::failed(current, CURVED_START);
                }
                let resolved = match (plane, start_body) {
                    (_, Some(body)) => self.plane_of(plane, body),
                    (PlaneRef::Datum { datum, offset }, None) => {
                        let p = datum.plane();
                        Ok(p.with_origin(p.origin + p.normal * *offset))
                    }
                    (PlaneRef::Feature { feature }, None) => self
                        .reference_plane(*feature)
                        .ok_or_else(|| Box::new(self.missing_plane(*feature, ReferenceSlot::Start))),
                    (PlaneRef::Face { .. }, None) => {
                        return Built::failed(current, "starting on a face needs a body");
                    }
                };
                let p = match resolved {
                    Ok(p) => p,
                    Err(r) => {
                        let r = RepairNeeded { slot: ReferenceSlot::Start, ..*r };
                        return Built::pass(FeatureStatus::RepairNeeded(vec![r]), current);
                    }
                };
                if p.normal.cross(normal).length() > PARALLEL {
                    if p.normal.dot(normal).abs() <= PARALLEL {
                        return Built::failed(current, START_HOLDS_DIRECTION);
                    }
                    tilted = Some(p);
                    0.0
                } else {
                    (p.origin - sk.plane.origin).dot(normal)
                }
            }
            Some(ExtrudeStart::Vertex { vertex }) => {
                let Some(body) = start_body else {
                    return Built::failed(current, "starting at a vertex needs a body");
                };
                let position = self
                    .resolve_vertex(vertex, body)
                    .map_err(|r| RepairNeeded { slot: ReferenceSlot::Start, ..r })
                    .and_then(|v| {
                        self.kernel.topology(body).ok().and_then(|t| t.vertex(v).map(|v| v.position)).ok_or_else(|| {
                            kernel_repair(
                                ReferenceSlot::Start,
                                KernelError::InvalidInput("the vertex is missing".into()),
                            )
                        })
                    });
                match position {
                    Ok(p) => (p - sk.plane.origin).dot(normal),
                    Err(r) => return Built::pass(FeatureStatus::RepairNeeded(vec![r]), current),
                }
            }
        };
        let start = sk.plane.with_origin(sk.plane.origin + normal * shift);
        // How far a point lies ahead of the start along a direction.
        let ahead = |point: Vec3, along: Vec3| match tilted {
            Some(p) => (point - p.origin).dot(p.normal) / along.dot(p.normal),
            None => (point - start.origin).dot(along),
        };
        // How far each direction goes: a length, or a limit the kernel finds on the body.
        let extent = |end: &EndCondition, along: Vec3, slot: ReferenceSlot| -> Result<Extent, Box<Built>> {
            let failed = |message: String| Err(Box::new(Built::failed(current, message)));
            match end {
                EndCondition::Blind { depth } if depth.is_finite() && *depth > 0.0 => Ok(Extent::Length(*depth)),
                EndCondition::Blind { depth } => failed(format!("depth must be positive, got {depth}")),
                EndCondition::ThroughAll => {
                    let Some(body) = current else {
                        return failed("through all needs a body to go through".into());
                    };
                    let bbox = match self.kernel.topology(body) {
                        Ok(t) => t.bbox,
                        Err(err) => return failed(feature_error(err)),
                    };
                    let (lo, hi) = (bbox.min, bbox.max);
                    let reach = (0..8)
                        .map(|c| {
                            let corner = Vec3::new(
                                if c & 1 == 0 { lo.x } else { hi.x },
                                if c & 2 == 0 { lo.y } else { hi.y },
                                if c & 4 == 0 { lo.z } else { hi.z },
                            );
                            ahead(corner, along)
                        })
                        .fold(f64::NEG_INFINITY, f64::max);
                    if reach <= 1e-9 {
                        // Nothing lies this way; the other direction may still reach the body.
                        return Ok(Extent::Length(0.0));
                    }
                    Ok(Extent::Length(reach + 1.0 + 0.01 * bbox.diagonal()))
                }
                EndCondition::UpToNext if tilted.is_some() => failed(
                    "Up to next cannot start on a plane or face that is not parallel to the sketch: \
                     the kernel finds the next face only from a parallel start. Choose Up to face, Blind or \
                     Through all"
                        .into(),
                ),
                EndCondition::UpToNext if current.is_some() => Ok(Extent::Limit(ExtrudeLimit::UpToNext)),
                EndCondition::UpToNext => failed("an extrusion up to the next face needs a body".into()),
                EndCondition::UpToFace { face } => {
                    let Some(body) = current else {
                        return failed("an extrusion up to a face needs a body".into());
                    };
                    match self.resolve_face(face, body) {
                        Ok(face) => Ok(Extent::Limit(ExtrudeLimit::UpToFace { face })),
                        Err(r) => {
                            let r = RepairNeeded { slot, ..r };
                            Err(Box::new(Built::pass(FeatureStatus::RepairNeeded(vec![r]), current)))
                        }
                    }
                }
            }
        };
        let first = match extent(&e.end, dir, ReferenceSlot::Limit) {
            Ok(x) => x,
            Err(refused) => return *refused,
        };
        let second = match e.options.direction2.as_ref().map(|end| extent(end, -dir, ReferenceSlot::SecondLimit)) {
            None => None,
            Some(Ok(x)) => Some(x),
            Some(Err(refused)) => return *refused,
        };
        // Through all that finds no body on its side is refused alone and skipped beside
        // Direction 2.
        let empty = |x: &Extent| matches!(x, Extent::Length(d) if *d <= 0.0);
        if empty(&first) && second.as_ref().is_none_or(empty) {
            return Built::failed(current, "the body lies entirely behind the sketch plane");
        }
        // The sweeps for each region: one from the start, or one through the start when
        // both directions have a length, or one each way when either stops at a limit.
        // Each is (start offset along the normal, direction sense, extent, which names).
        let sweeps: Vec<(f64, f64, Extent, Caps)> = match (&first, &second) {
            (_, None) => vec![(shift, sense, first.clone(), Caps::Both)],
            (Extent::Length(d1), Some(Extent::Length(d2))) => {
                vec![(shift - d2 * sense, sense, Extent::Length(d1 + d2), Caps::Both)]
            }
            (_, Some(other)) => {
                [(shift, sense, first.clone(), Caps::End), (shift, -sense, other.clone(), Caps::Second)]
                    .into_iter()
                    .filter(|(_, _, x, _)| !empty(x))
                    .collect()
            }
        };
        let mut mine: Option<BodyId> = None;
        let mut faces: Vec<(Origin, Generator, usize)> = Vec::new();
        let mut run = |mine: &mut Option<BodyId>| -> KernelResult<Option<BodyId>> {
            let k = &self.kernel;
            let mut body = current;
            for (i, (region, names)) in tools.iter().enumerate() {
                for (j, (offset, way, reach, caps)) in sweeps.iter().enumerate() {
                    let tag = |step: &str| {
                        if sweeps.len() == 1 {
                            OpTag::new(format!("{}.r{i}.{step}", f.id))
                        } else {
                            OpTag::new(format!("{}.r{i}.d{}.{step}", f.id, j + 1))
                        }
                    };
                    let along = normal * *way;
                    if let Some(p) = tilted {
                        let start = p.with_origin(p.origin + normal * *offset);
                        let (tool, named) =
                            self.tilted_sweep(&tag, region, names, sk.plane, start, along, reach, *caps, current)?;
                        faces.extend(named.into_iter().map(|(origin, generator)| (origin, generator, i)));
                        let next = self.join_tool(e.operation, body, tool, &tag)?;
                        if let Some(old) = mine.replace(next) {
                            let _ = k.release(old);
                        }
                        body = Some(next);
                        continue;
                    }
                    let plane = sk.plane.with_origin(sk.plane.origin + normal * *offset);
                    let profile = region.to_profile(plane);
                    let tool = match (reach, sk.support) {
                        (Extent::Length(distance), Some(support)) => {
                            let support = FaceSupport { offset: support.offset + offset, ..support };
                            k.extrude_on_face(&tag("sweep"), &profile, support, distance * way)?
                        }
                        (Extent::Length(distance), None) => {
                            let sheet = k.create_body_from_profile(&tag("profile"), &profile)?;
                            let tool = k.extrude(
                                &tag("sweep"),
                                sheet.body,
                                &Extrude { distance: *distance, direction: Some(along) },
                            );
                            let _ = k.release(sheet.body);
                            tool?
                        }
                        (Extent::Limit(limit), _) => {
                            let Some(target) = current else {
                                return Err(KernelError::InvalidInput("an extrusion up to a face needs a body".into()));
                            };
                            let sheet = k.create_body_from_profile(&tag("profile"), &profile)?;
                            let tool = k.extrude_to(&tag("sweep"), sheet.body, target, Some(along), limit);
                            let _ = k.release(sheet.body);
                            tool?
                        }
                    };
                    for o in &tool.record.face_roles {
                        let generator = match (&o.role, caps) {
                            (FaceRole::StartCap, Caps::Both) => Some(Generator::StartCap),
                            (FaceRole::EndCap, Caps::Both | Caps::End) => Some(Generator::EndCap),
                            // The far cap of the second direction is the extrusion's start cap,
                            // as it is when one sweep goes through the start.
                            (FaceRole::EndCap, Caps::Second) => Some(Generator::StartCap),
                            (FaceRole::StartCap, _) => None,
                            (FaceRole::Side { loop_index, segment, .. }, _) => {
                                region.entity(*loop_index as usize, *segment as usize).map(|entity| {
                                    names.get(&entity).cloned().unwrap_or(Generator::Side { entity, path: None })
                                })
                            }
                            _ => None,
                        };
                        if let Some(generator) = generator {
                            faces.push((Origin { tag: tool.record.tag.clone(), role: o.role.clone() }, generator, i));
                        }
                    }
                    let next = match (e.operation, body) {
                        (ExtrudeOp::Boss, None) => tool.body,
                        (ExtrudeOp::Cut, None) => {
                            let _ = k.release(tool.body);
                            return Err(KernelError::InvalidInput("there is no body to cut".into()));
                        }
                        (op, Some(b)) => {
                            let (kind, step) = if op == ExtrudeOp::Boss {
                                (BooleanOp::Union, "union")
                            } else {
                                (BooleanOp::Subtract, "cut")
                            };
                            let joined = k.boolean(&tag(step), kind, b, tool.body);
                            let _ = k.release(tool.body);
                            joined?.body
                        }
                    };
                    if let Some(old) = mine.replace(next) {
                        let _ = k.release(old);
                    }
                    body = Some(next);
                }
            }
            Ok(body)
        };
        match run(&mut mine) {
            Ok(output) => Built {
                body_key: 0,
                consumed_tool: None,
                status: FeatureStatus::Ok,
                output,
                owned: mine,
                names: number(faces),
                sketch: None,
                plane: None,
                version: 0,
                work_used: None,
                import_record: None,
            },
            Err(err) => {
                if let Some(body) = mine {
                    let _ = self.kernel.release(body);
                }
                Built::failed(current, feature_error(err))
            }
        }
    }

    fn build_blend(&self, f: &Feature, b: &BlendFeature, chamfer: bool, current: Option<BodyId>) -> Built {
        let what = if chamfer { "chamfer" } else { "fillet" };
        let Some(body) = current else {
            return Built::failed(current, format!("there is no body to {what}"));
        };
        if b.edges.is_empty() {
            return Built::failed(current, format!("the {what} has no edges"));
        }
        let t = match self.view_of(body) {
            Ok(v) => v,
            Err(err) => return Built::failed(current, feature_error(err)),
        };
        let names = self.name_map();
        let view = View { topology: &t };
        let mut ids: Vec<EdgeId> = Vec::new();
        let mut repairs = Vec::new();
        for (i, r) in b.edges.iter().enumerate() {
            match reference::resolve_edge(r, &view, &names) {
                Ok(id) => ids.push(id),
                Err(u) => repairs.push(u.into_repair(ReferenceSlot::Edge { index: i as u32 })),
            }
        }
        if !repairs.is_empty() {
            return Built::pass(FeatureStatus::RepairNeeded(repairs), current);
        }
        for (i, id) in ids.iter().enumerate() {
            if let Some(j) = ids[..i].iter().position(|x| x == id) {
                return Built::failed(current, format!("edges {} and {} name the same edge", j + 1, i + 1));
            }
        }
        let tag = OpTag::new(format!("{}.blend", f.id));
        let result = if chamfer {
            self.kernel.chamfer_distances(&tag, body, &ids, b.size, b.other_distance)
        } else {
            self.kernel.fillet(&tag, body, &ids, b.size)
        };
        let result = match result {
            Ok(r) => r,
            Err(err) => {
                let message = self.fillet_reaches(&err, body).unwrap_or_else(|| feature_error(err));
                return Built::failed(current, message);
            }
        };
        let faces = result
            .record
            .face_roles
            .iter()
            .filter_map(|o| match o.role {
                FaceRole::Blend { edge } => ids.iter().position(|x| *x == edge).map(|i| {
                    (
                        Origin { tag: result.record.tag.clone(), role: o.role.clone() },
                        Generator::Blend { edge: i as u32 },
                        0,
                    )
                }),
                // Ranked by where its edges sit in the feature's list, which survives a
                // rebuild where a vertex id may not.
                FaceRole::Corner { vertex } => {
                    let meets = |edge: &EdgeId| t.edge(*edge).is_some_and(|e| e.vertices.contains(&vertex));
                    let rank = ids.iter().enumerate().filter(|(_, edge)| meets(edge));
                    let rank = rank.fold(0, |rank, (i, _)| rank * (ids.len() + 1) + i + 1);
                    Some((Origin { tag: result.record.tag.clone(), role: o.role.clone() }, Generator::Other, rank))
                }
                _ => None,
            })
            .collect();
        Built {
            body_key: 0,
            consumed_tool: None,
            status: FeatureStatus::Ok,
            output: Some(result.body),
            owned: Some(result.body),
            names: number(faces),
            sketch: None,
            plane: None,
            version: 0,
            work_used: None,
            import_record: None,
        }
    }

    /// The body an extrusion's start face or vertex lies on, as it stands before the
    /// extrusion: the body of the feature that made the face, or `current` when that is
    /// the extrusion's own body or the start names no face.
    fn start_body(&self, f: &Feature, from: Option<&ExtrudeStart>, current: Option<BodyId>) -> Option<BodyId> {
        let owner = match from {
            Some(ExtrudeStart::Surface { plane: PlaneRef::Face { face } }) => Some(face.feature),
            Some(ExtrudeStart::Vertex { vertex }) => vertex.faces.first().map(|face| face.feature),
            _ => None,
        };
        let Some(key) = owner.and_then(|id| self.feature(id)).map(|g| g.body) else {
            return current;
        };
        let end = self.index_of(f.id).unwrap_or(self.built.len());
        // A Combine may have joined that body to another, which holds the face now.
        let key = self.surviving_key(key, end);
        if key == f.body {
            return current;
        }
        self.body_at(key, end).or(current)
    }

    /// Joins an extrusion's tool to `body` or cuts it away, or makes it the body when
    /// there is none yet. Releases the tool.
    fn join_tool(
        &self,
        operation: ExtrudeOp,
        body: Option<BodyId>,
        tool: BodyId,
        tag: &dyn Fn(&str) -> OpTag,
    ) -> KernelResult<BodyId> {
        let k = &self.kernel;
        match (operation, body) {
            (ExtrudeOp::Boss, None) => Ok(tool),
            (ExtrudeOp::Cut, None) => {
                let _ = k.release(tool);
                Err(KernelError::InvalidInput("there is no body to cut".into()))
            }
            (op, Some(b)) => {
                let (kind, step) =
                    if op == ExtrudeOp::Boss { (BooleanOp::Union, "union") } else { (BooleanOp::Subtract, "cut") };
                let joined = k.boolean(&tag(step), kind, b, tool);
                let _ = k.release(tool);
                Ok(joined?.body)
            }
        }
    }

    /// One sweep of an extrusion that starts on `start`, a plane not parallel to its
    /// sketch: a prism of the region along `along`, clipped by a slab, a rectangle on
    /// the start plane swept the same way as far as `reach` says. Returns the tool
    /// and the names of its faces.
    #[allow(clippy::too_many_arguments)]
    fn tilted_sweep(
        &self,
        tag: &dyn Fn(&str) -> OpTag,
        region: &Region,
        names: &HashMap<EntityId, Generator>,
        sketch: Plane,
        start: Plane,
        along: Vec3,
        reach: &Extent,
        caps: Caps,
        target: Option<BodyId>,
    ) -> KernelResult<(BodyId, Vec<(Origin, Generator)>)> {
        let k = &self.kernel;
        let Some(span) = crate::tilted_start::span(&region.to_profile(sketch), &start, along) else {
            return Err(KernelError::InvalidInput(START_HOLDS_DIRECTION.into()));
        };
        if let (Extent::Limit(ExtrudeLimit::UpToFace { face }), Some(body)) = (reach, target)
            && !self.limit_clears_start(body, *face, &region.to_profile(sketch), &start, along, &span)?
        {
            return Err(KernelError::InvalidInput(LIMIT_CROSSES_START.into()));
        }
        let margin = crate::tilted_start::margin(&span);
        // How far the slab goes from the start: the length, or past the body for a limit.
        let length = match (reach, target) {
            (Extent::Length(d), _) => *d,
            (Extent::Limit(_), Some(body)) => {
                let bbox = k.topology(body)?.bbox;
                let (lo, hi) = (bbox.min, bbox.max);
                let rate = along.dot(start.normal);
                let past = (0..8)
                    .map(|c| {
                        let corner = Vec3::new(
                            if c & 1 == 0 { lo.x } else { hi.x },
                            if c & 2 == 0 { lo.y } else { hi.y },
                            if c & 4 == 0 { lo.z } else { hi.z },
                        );
                        (corner - start.origin).dot(start.normal) / rate
                    })
                    .fold(f64::NEG_INFINITY, f64::max);
                past.max(span.far - span.near) + margin + 0.01 * bbox.diagonal()
            }
            (Extent::Limit(_), None) => {
                return Err(KernelError::InvalidInput("an extrusion up to a face needs a body".into()));
            }
        };
        if !(length.is_finite() && length > 0.0) {
            return Err(KernelError::InvalidInput("the body lies entirely behind the start plane".into()));
        }
        // The prism starts behind the nearest point of the start plane over the profile.
        let base = sketch.with_origin(sketch.origin + along * (span.near - margin));
        let sheet = k.create_body_from_profile(&tag("profile"), &region.to_profile(base))?;
        let prism = match (reach, target) {
            (Extent::Length(d), _) => k.extrude(
                &tag("sweep"),
                sheet.body,
                &Extrude { distance: span.far - span.near + d + 2.0 * margin, direction: Some(along) },
            ),
            (Extent::Limit(limit), Some(body)) => k.extrude_to(&tag("sweep"), sheet.body, body, Some(along), limit),
            (Extent::Limit(_), None) => unreachable!("a limit without a body is refused above"),
        };
        let _ = k.release(sheet.body);
        let prism = prism?;
        let rectangle = Profile::rectangle(span.slab, Point2::new(0.0, 0.0), span.width, span.height);
        let slab = k.create_body_from_profile(&tag("start"), &rectangle).and_then(|sheet| {
            let slab = k.extrude(&tag("slab"), sheet.body, &Extrude { distance: length, direction: Some(along) });
            let _ = k.release(sheet.body);
            slab
        });
        let slab = match slab {
            Ok(slab) => slab,
            Err(err) => {
                let _ = k.release(prism.body);
                return Err(err);
            }
        };
        let clipped = k.boolean(&tag("clip"), BooleanOp::Intersect, prism.body, slab.body);
        let _ = k.release(prism.body);
        let _ = k.release(slab.body);
        let clipped = clipped?;
        // The sides come from the prism. The start cap is the slab's near face; the end
        // cap is the slab's far face, or the prism's for a limit.
        let limited = matches!(reach, Extent::Limit(_));
        let mut named = Vec::new();
        for o in &prism.record.face_roles {
            let generator = match (&o.role, caps) {
                (FaceRole::Side { loop_index, segment, .. }, _) => region
                    .entity(*loop_index as usize, *segment as usize)
                    .map(|entity| names.get(&entity).cloned().unwrap_or(Generator::Side { entity, path: None })),
                (FaceRole::EndCap, Caps::Both | Caps::End) if limited => Some(Generator::EndCap),
                (FaceRole::EndCap, Caps::Second) if limited => Some(Generator::StartCap),
                _ => None,
            };
            if let Some(generator) = generator {
                named.push((Origin { tag: prism.record.tag.clone(), role: o.role.clone() }, generator));
            }
        }
        for o in &slab.record.face_roles {
            let generator = match (&o.role, caps) {
                (FaceRole::StartCap, Caps::Both) => Some(Generator::StartCap),
                (FaceRole::EndCap, Caps::Both | Caps::End) if !limited => Some(Generator::EndCap),
                (FaceRole::EndCap, Caps::Second) if !limited => Some(Generator::StartCap),
                _ => None,
            };
            if let Some(generator) = generator {
                named.push((Origin { tag: slab.record.tag.clone(), role: o.role.clone() }, generator));
            }
        }
        Ok((clipped.body, named))
    }

    /// Whether the face an extrusion ends at lies ahead of a tilted start over the whole
    /// profile, so the sweep from every point of the start reaches it. A planar face is
    /// checked at the corners of the profile's bounding box, where the gap between the
    /// two planes is least. A curved face must lie wholly beyond the start's farthest
    /// point. A face parallel to the direction is left to the kernel to refuse.
    fn limit_clears_start(
        &self,
        body: BodyId,
        face: FaceId,
        profile: &Profile,
        start: &Plane,
        along: Vec3,
        span: &crate::tilted_start::Span,
    ) -> KernelResult<bool> {
        let t = self.kernel.topology(body)?;
        let Some(limit) = t.face(face) else {
            return Err(KernelError::InvalidInput("the face the extrusion ends at is missing".into()));
        };
        let Some((lo, hi)) = crate::tilted_start::bounds(profile) else {
            return Ok(false);
        };
        let corners = [(lo.u, lo.v), (hi.u, lo.v), (lo.u, hi.v), (hi.u, hi.v)]
            .map(|(u, v)| profile.plane.to_world(Point2::new(u, v)));
        // How far along the direction a point of the sketch plane meets a plane.
        let reach = |on: Vec3, normal: Vec3, from: Vec3| (on - from).dot(normal) / along.dot(normal);
        match limit.normal {
            Some(n) if along.dot(n).abs() > PARALLEL => {
                Ok(corners.iter().all(|c| reach(limit.centroid, n, *c) > reach(start.origin, start.normal, *c)))
            }
            Some(_) => Ok(true),
            None => {
                let (lo, hi) = (limit.bbox.min, limit.bbox.max);
                let nearest = (0..8)
                    .map(|c| {
                        let corner = Vec3::new(
                            if c & 1 == 0 { lo.x } else { hi.x },
                            if c & 2 == 0 { lo.y } else { hi.y },
                            if c & 4 == 0 { lo.z } else { hi.z },
                        );
                        (corner - profile.plane.origin).dot(along)
                    })
                    .fold(f64::INFINITY, f64::min);
                Ok(nearest > span.far)
            }
        }
    }

    /// Resolves a feature's plane: a reference plane, or a planar face of `body` with
    /// its outward normal.
    pub fn plane_of(&self, r: &PlaneRef, body: BodyId) -> Result<Plane, Box<RepairNeeded>> {
        match r {
            PlaneRef::Feature { feature } => self
                .reference_plane(*feature)
                .ok_or_else(|| Box::new(self.missing_plane(*feature, ReferenceSlot::Plane))),
            PlaneRef::Datum { datum, offset } => {
                let p = datum.plane();
                Ok(p.with_origin(p.origin + p.normal * *offset))
            }
            PlaneRef::Face { face } => {
                let id = self
                    .resolve_plane_face(face, body)
                    .map_err(|r| Box::new(RepairNeeded { slot: ReferenceSlot::Plane, ..r }))?;
                let info = self.kernel.topology(body).ok().and_then(|t| t.face(id).cloned());
                match info.and_then(|f| f.normal.map(|n| (n, f.centroid))) {
                    Some((normal, centroid)) => Ok(plane_on_face(normal, centroid)),
                    None => {
                        let detail = "the face is not planar".to_string();
                        Err(Box::new(RepairNeeded {
                            slot: ReferenceSlot::Plane,
                            reason: RepairReason::Mismatch { detail: detail.clone() },
                            message: detail,
                            candidates: vec![],
                        }))
                    }
                }
            }
        }
    }

    /// Resolves a draft's pull direction: an origin axis, a straight edge's direction or
    /// a circular edge's axis, or a planar face's outward normal.
    pub fn pull_of(&self, r: &PullRef, body: BodyId) -> Result<Vec3, Box<RepairNeeded>> {
        let mismatch = |detail: &str| {
            Box::new(RepairNeeded {
                slot: ReferenceSlot::Pull,
                reason: RepairReason::Mismatch { detail: detail.into() },
                message: detail.into(),
                candidates: vec![],
            })
        };
        match r {
            PullRef::Axis { axis } => Ok(axis.direction()),
            PullRef::Edge { edge } => {
                let id = self
                    .resolve_edge(edge, body)
                    .map_err(|r| Box::new(RepairNeeded { slot: ReferenceSlot::Pull, ..r }))?;
                match self.kernel.edge_curve(body, id) {
                    Ok(Curve3::Line { a, b }) => (b - a).normalized().ok_or_else(|| mismatch("the edge has no length")),
                    Ok(Curve3::Circle { normal, .. }) => Ok(normal),
                    Ok(_) => Err(mismatch("the edge is neither straight nor circular")),
                    Err(e) => Err(Box::new(kernel_repair(ReferenceSlot::Pull, e))),
                }
            }
            PullRef::Face { face } => {
                let id = self
                    .resolve_face(face, body)
                    .map_err(|r| Box::new(RepairNeeded { slot: ReferenceSlot::Pull, ..r }))?;
                let info = self.kernel.topology(body).ok().and_then(|t| t.face(id).cloned());
                info.and_then(|f| f.normal).ok_or_else(|| mismatch("the face is not planar"))
            }
        }
    }

    /// The result of an operation that makes a new body from the part's body, with
    /// the faces it made named.
    fn finish(
        &self,
        result: KernelResult<OpResult>,
        current: Option<BodyId>,
        name: impl FnOnce(&OpResult) -> Vec<NamedFace>,
    ) -> Built {
        match result {
            Ok(r) => Built {
                body_key: 0,
                consumed_tool: None,
                status: FeatureStatus::Ok,
                output: Some(r.body),
                owned: Some(r.body),
                names: name(&r),
                sketch: None,
                plane: None,
                version: 0,
                work_used: None,
                import_record: None,
            },
            Err(err) => Built::failed(current, feature_error(err)),
        }
    }

    /// Resolves the 2D placement points of a hole feature: from its position sketch's
    /// non-construction `Point` entities when `h.sketch` is set, else from `h.points`.
    pub fn hole_points(&self, h: &HoleFeature) -> Result<Vec<soliddraft_sketch::P2>, String> {
        if let Some(sketch_id) = h.sketch {
            let Some(feature) = self.feature(sketch_id) else {
                return Err(format!("the hole position sketch {sketch_id} is missing"));
            };
            let FeatureKind::Sketch(sf) = &feature.kind else {
                return Err(format!("{} is not a sketch", feature.name));
            };
            let pts: Vec<soliddraft_sketch::P2> = sf
                .sketch
                .entities
                .iter()
                .filter(|e| e.id != soliddraft_sketch::ORIGIN && !e.construction)
                .filter_map(|e| match e.kind {
                    soliddraft_sketch::EntityKind::Point { p } => Some(p),
                    _ => None,
                })
                .collect();
            if pts.is_empty() {
                return Err("the hole position sketch has no points".into());
            }
            Ok(pts)
        } else if h.points.is_empty() {
            Err("the hole has no points".into())
        } else {
            Ok(h.points.clone())
        }
    }

    fn build_hole(&self, f: &Feature, h: &HoleFeature, current: Option<BodyId>) -> Built {
        use crate::hole_wizard::HoleEndCondition;
        let Some(body) = current else { return Built::failed(current, "there is no body to drill") };
        if let Some(reason) = h.wizard.as_ref().and_then(|w| w.coming_soon_refusal()) {
            return Built::failed(current, reason);
        }
        let points = match self.hole_points(h) {
            Ok(pts) => pts,
            Err(why) => return Built::failed(current, why),
        };
        let face_id = match self.resolve_face(&h.face, body) {
            Ok(id) => id,
            Err(r) => {
                let r = RepairNeeded { slot: ReferenceSlot::Plane, ..r };
                return Built::pass(FeatureStatus::RepairNeeded(vec![r]), current);
            }
        };
        let topology = match self.view_of(body) {
            Ok(t) => t,
            Err(err) => return Built::failed(current, feature_error(err)),
        };
        let Some(face_info) = topology.face(face_id) else {
            return Built::failed(current, "the selected hole face is no longer in the body");
        };
        let Some(normal) = face_info.normal else {
            let label = if h.face.label.is_empty() { "selected face" } else { h.face.label.as_str() };
            return Built::failed(
                current,
                format!("Holes can only be placed on a planar face; {label} is not planar."),
            );
        };
        let plane = plane_on_face(normal, face_info.centroid);
        let flip = h.wizard.as_ref().is_some_and(|w| w.flip_direction);
        let drill_dir = if flip { plane.normal } else { -plane.normal };

        enum ResolvedEnd {
            Blind(f64),
            Through,
            UpToNext,
        }

        let resolved_end = if let Some(wizard) = &h.wizard {
            match &wizard.end_condition {
                HoleEndCondition::Blind { depth } => {
                    if !(depth.is_finite() && *depth > 0.0) {
                        return Built::failed(current, format!("hole depth must be positive, got {depth}"));
                    }
                    ResolvedEnd::Blind(*depth)
                }
                HoleEndCondition::ThroughAll => ResolvedEnd::Through,
                HoleEndCondition::UpToNext => {
                    let profiles = points.iter().map(|pt| {
                        if wizard.hole_type.is_slot() {
                            let length =
                                wizard.slot_length.unwrap_or_else(|| wizard.size_row().default_slot_length_mm());
                            stadium_profile(plane, pt.u, pt.v, h.diameter, length, wizard.slot_angle_deg)
                        } else {
                            Profile::circle(plane, Point2::new(pt.u, pt.v), h.diameter / 2.0)
                        }
                    });
                    let reach = through_reach(&topology.bbox, plane.origin, drill_dir);
                    self.next_parallel_hole_depth(f.id, body, drill_dir, reach, profiles)
                        .map_or(ResolvedEnd::UpToNext, ResolvedEnd::Blind)
                }
                HoleEndCondition::UpToVertex { vertex } => {
                    let vid = match self.resolve_vertex(vertex, body) {
                        Ok(v) => v,
                        Err(r) => {
                            let r = RepairNeeded { slot: ReferenceSlot::Limit, ..r };
                            return Built::pass(FeatureStatus::RepairNeeded(vec![r]), current);
                        }
                    };
                    let Some(vinfo) = topology.vertex(vid) else {
                        return Built::failed(current, "the target vertex is no longer in the body");
                    };
                    let depth = (vinfo.position - plane.origin).dot(drill_dir);
                    if !(depth.is_finite() && depth > 1e-6) {
                        return Built::failed(
                            current,
                            "The target vertex does not lie ahead of the hole's start face.",
                        );
                    }
                    ResolvedEnd::Blind(depth)
                }
                HoleEndCondition::UpToSurface { face: target } => {
                    let tid = match self.resolve_face(target, body) {
                        Ok(id) => id,
                        Err(r) => {
                            let r = RepairNeeded { slot: ReferenceSlot::Limit, ..r };
                            return Built::pass(FeatureStatus::RepairNeeded(vec![r]), current);
                        }
                    };
                    let Some(tinfo) = topology.face(tid) else {
                        return Built::failed(current, "the target surface is no longer in the body");
                    };
                    let Some(tnormal) = tinfo.normal else {
                        return Built::failed(
                            current,
                            "Up to surface on a non-parallel or curved face is coming soon.",
                        );
                    };
                    if tnormal.dot(plane.normal).abs() < 1.0 - 1e-6 {
                        return Built::failed(
                            current,
                            "Up to surface on a non-parallel or curved face is coming soon.",
                        );
                    }
                    let depth = (tinfo.centroid - plane.origin).dot(drill_dir);
                    if !(depth.is_finite() && depth > 1e-6) {
                        return Built::failed(
                            current,
                            "The target surface does not lie ahead of the hole's start face.",
                        );
                    }
                    ResolvedEnd::Blind(depth)
                }
                HoleEndCondition::OffsetFromSurface { face: target, offset, reverse_offset } => {
                    if !offset.is_finite() {
                        return Built::failed(current, "surface offset must be finite");
                    }
                    let tid = match self.resolve_face(target, body) {
                        Ok(id) => id,
                        Err(r) => {
                            let r = RepairNeeded { slot: ReferenceSlot::Limit, ..r };
                            return Built::pass(FeatureStatus::RepairNeeded(vec![r]), current);
                        }
                    };
                    let Some(tinfo) = topology.face(tid) else {
                        return Built::failed(current, "the target surface is no longer in the body");
                    };
                    let Some(tnormal) = tinfo.normal else {
                        return Built::failed(
                            current,
                            "Offset from surface on a non-parallel or curved face is coming soon.",
                        );
                    };
                    if tnormal.dot(plane.normal).abs() < 1.0 - 1e-6 {
                        return Built::failed(
                            current,
                            "Offset from surface on a non-parallel or curved face is coming soon.",
                        );
                    }
                    let dist = (tinfo.centroid - plane.origin).dot(drill_dir);
                    let signed = if *reverse_offset { *offset } else { -*offset };
                    let depth = dist + signed;
                    if !(depth.is_finite() && depth > 1e-6) {
                        return Built::failed(current, "The offset from surface leaves no positive hole depth.");
                    }
                    ResolvedEnd::Blind(depth)
                }
            }
        } else {
            match h.depth {
                HoleDepth::Blind { depth } => ResolvedEnd::Blind(depth),
                HoleDepth::Through => ResolvedEnd::Through,
            }
        };

        let effective_head = h.effective_head();

        // Slot and CounterboreSlot build via exact stadium profile extrusion + boolean subtraction.
        if let Some(wizard) = &h.wizard
            && wizard.hole_type.is_slot()
        {
            let length = wizard.slot_length.unwrap_or_else(|| wizard.size_row().default_slot_length_mm());
            if !(length.is_finite() && length > 0.0) {
                return Built::failed(current, "slot length must be positive and finite");
            }
            let (through_depth, up_to_next) = match resolved_end {
                ResolvedEnd::Blind(d) => (d, false),
                ResolvedEnd::Through => {
                    let reach = through_reach(&topology.bbox, plane.origin, drill_dir);
                    if reach <= 1e-9 {
                        return Built::failed(current, "the body lies entirely behind the hole's start face");
                    }
                    (reach, false)
                }
                ResolvedEnd::UpToNext => {
                    if effective_head.is_some() {
                        return Built::failed(
                            current,
                            "Up to next on counterbored, countersunk or drill-point holes is coming soon.",
                        );
                    }
                    (0.0, true)
                }
            };
            let cbore = match effective_head {
                Some(HoleHead::Counterbore { diameter, depth }) => Some((diameter, depth)),
                _ => None,
            };
            return self.build_slot_cuts(
                f,
                body,
                face_id,
                plane,
                drill_dir,
                flip,
                &points,
                h.diameter,
                length,
                wizard.slot_angle_deg,
                through_depth,
                up_to_next,
                cbore,
            );
        }

        // Circular UpToNext: supported for plain flat holes via Catallax's exact ExtrudeNext.
        if matches!(resolved_end, ResolvedEnd::UpToNext) {
            if effective_head.is_some() || !matches!(h.tip, HoleTip::Flat) {
                return Built::failed(
                    current,
                    "Up to next on counterbored, countersunk or drill-point holes is coming soon.",
                );
            }
            if let Some(thread) = &h.thread
                && let Some(problem) = thread.problem(h.diameter, &HoleDepth::Through, None)
            {
                return Built::failed(current, problem);
            }
            return self.build_circular_up_to_next(f, body, plane, drill_dir, &points, h.diameter);
        }

        let depth = match resolved_end {
            ResolvedEnd::Blind(depth) => HoleDepth::Blind { depth },
            ResolvedEnd::Through | ResolvedEnd::UpToNext => HoleDepth::Through,
        };
        if let Some(thread) = &h.thread
            && let Some(problem) = thread.problem(h.diameter, &depth, effective_head.as_ref())
        {
            return Built::failed(current, problem);
        }
        let placements = points
            .iter()
            .map(|p| HolePlacement {
                point: plane.origin + plane.x_dir * p.u + plane.y_dir() * p.v,
                direction: drill_dir,
                on_face: (!flip).then_some(FacePoint { face: face_id, u: p.u, v: p.v }),
            })
            .collect();
        let params = Hole { placements, diameter: h.diameter, depth, tip: h.tip, head: effective_head };
        let result = self.kernel.hole(&OpTag::new(format!("{}.hole", f.id)), body, &params);
        self.finish(result, current, |r| {
            let faces = r
                .record
                .face_roles
                .iter()
                .filter_map(|o| match o.role {
                    FaceRole::Hole { placement, part } => Some((
                        Origin { tag: r.record.tag.clone(), role: o.role.clone() },
                        Generator::Hole { placement, part },
                        0,
                    )),
                    _ => None,
                })
                .collect();
            number(faces)
        })
    }

    /// Let the kernel find the first trimmed surface over each cutting profile. A shared
    /// parallel end lets the native hole operation retain the drill and head's face names.
    fn next_parallel_hole_depth(
        &self,
        id: FeatureId,
        target: BodyId,
        direction: Vec3,
        reach: f64,
        profiles: impl Iterator<Item = Profile>,
    ) -> Option<f64> {
        let mut common: Option<f64> = None;
        for (i, profile) in profiles.enumerate() {
            let origin = profile.plane.origin;
            let tag = |step: &str| OpTag::new(format!("{id}.hole_limit{i}.{step}"));
            let sheet = self.kernel.create_body_from_profile(&tag("profile"), &profile).ok()?;
            let limit =
                self.kernel.extrude_to(&tag("sweep"), sheet.body, target, Some(direction), &ExtrudeLimit::UpToNext);
            let clipped = limit.is_err();
            let tool = limit.or_else(|_| {
                self.kernel.extrude(&tag("probe"), sheet.body, &Extrude { distance: reach, direction: Some(direction) })
            });
            let _ = self.kernel.release(sheet.body);
            let tool = tool.ok()?;
            let topology = if clipped {
                // A curved carrier can overlap the sweep's bounding box without reaching
                // the tool. Check that the actual intersection is a single whole prism.
                let overlap = self.kernel.boolean(&tag("clip"), BooleanOp::Intersect, target, tool.body);
                let _ = self.kernel.release(tool.body);
                let overlap = overlap.ok()?;
                let topology = self.view_of(overlap.body);
                let _ = self.kernel.release(overlap.body);
                let topology = topology.ok()?;
                let parallel = |face: &soliddraft_kernel_api::FaceInfo| {
                    face.normal.is_some_and(|normal| normal.dot(direction).abs() >= 1.0 - 1e-6)
                };
                let caps = topology.faces.iter().filter(|face| parallel(face)).count();
                let has_entry = topology.faces.iter().any(|face| {
                    face.normal.is_some_and(|normal| normal.dot(direction) <= -1.0 + 1e-6)
                        && (face.centroid - origin).dot(direction).abs() <= 1e-6
                });
                let only_tool_sides = topology.faces.iter().all(|face| {
                    parallel(face)
                        || face
                            .origins
                            .iter()
                            .any(|origin| origin.tag == tool.record.tag && matches!(origin.role, FaceRole::Side { .. }))
                });
                if caps != 2 || !has_entry || !only_tool_sides {
                    return None;
                }
                topology
            } else {
                let topology = self.view_of(tool.body);
                let _ = self.kernel.release(tool.body);
                topology.ok()?
            };
            let mut ends = topology.faces.iter().filter_map(|face| {
                let outward = face.normal?;
                (outward.dot(direction) >= 1.0 - 1e-6).then(|| (face.centroid - origin).dot(direction))
            });
            let depth = ends.next().filter(|depth| depth.is_finite() && *depth > 1e-6)?;
            if ends.next().is_some() || common.is_some_and(|prior| (depth - prior).abs() > 1e-5) {
                return None;
            }
            common = Some(depth);
        }
        common
    }

    #[allow(clippy::too_many_arguments)]
    fn build_slot_cuts(
        &self,
        f: &Feature,
        target: BodyId,
        face_id: FaceId,
        plane: Plane,
        drill_dir: Vec3,
        flip: bool,
        points: &[soliddraft_sketch::P2],
        width: f64,
        length: f64,
        angle_deg: f64,
        depth: f64,
        up_to_next: bool,
        cbore: Option<(f64, f64)>,
    ) -> Built {
        let mut mine: Option<BodyId> = None;
        let run = (|| -> KernelResult<BodyId> {
            let k = &self.kernel;
            let mut body = target;
            for (i, pt) in points.iter().enumerate() {
                let profile = stadium_profile(plane, pt.u, pt.v, width, length, angle_deg);
                let tag = |step: &str| OpTag::new(format!("{}.slot{i}.{step}", f.id));
                let tool = if up_to_next {
                    let sheet = k.create_body_from_profile(&tag("profile"), &profile)?;
                    let swept = k.extrude_to(&tag("sweep"), sheet.body, body, Some(drill_dir), &ExtrudeLimit::UpToNext);
                    let _ = k.release(sheet.body);
                    swept?
                } else if !flip {
                    let support = FaceSupport { body: target, face: face_id, offset: 0.0 };
                    k.extrude_on_face(&tag("sweep"), &profile, support, -depth)?
                } else {
                    let sheet = k.create_body_from_profile(&tag("profile"), &profile)?;
                    let swept =
                        k.extrude(&tag("sweep"), sheet.body, &Extrude { distance: depth, direction: Some(drill_dir) });
                    let _ = k.release(sheet.body);
                    swept?
                };
                let cut = k.boolean(&tag("cut"), BooleanOp::Subtract, body, tool.body);
                let _ = k.release(tool.body);
                let next = cut?.body;
                if let Some(prev) = mine.replace(next) {
                    let _ = k.release(prev);
                }
                body = next;

                if let Some((cb_width, cb_depth)) = cbore {
                    let cb_profile = stadium_profile(plane, pt.u, pt.v, cb_width, length, angle_deg);
                    let cb_tag = |step: &str| OpTag::new(format!("{}.slot{i}.cb.{step}", f.id));
                    let cb_tool = if !flip {
                        let support = FaceSupport { body: target, face: face_id, offset: 0.0 };
                        k.extrude_on_face(&cb_tag("sweep"), &cb_profile, support, -cb_depth)?
                    } else {
                        let sheet = k.create_body_from_profile(&cb_tag("profile"), &cb_profile)?;
                        let swept = k.extrude(
                            &cb_tag("sweep"),
                            sheet.body,
                            &Extrude { distance: cb_depth, direction: Some(drill_dir) },
                        );
                        let _ = k.release(sheet.body);
                        swept?
                    };
                    let cut = k.boolean(&cb_tag("cut"), BooleanOp::Subtract, body, cb_tool.body);
                    let _ = k.release(cb_tool.body);
                    let next = cut?.body;
                    if let Some(prev) = mine.replace(next) {
                        let _ = k.release(prev);
                    }
                    body = next;
                }
            }
            Ok(body)
        })();
        match run {
            Ok(body) => Built {
                body_key: 0,
                consumed_tool: None,
                status: FeatureStatus::Ok,
                output: Some(body),
                owned: mine,
                names: Vec::new(),
                sketch: None,
                plane: None,
                version: 0,
                work_used: None,
                import_record: None,
            },
            Err(err) => {
                if let Some(b) = mine {
                    let _ = self.kernel.release(b);
                }
                Built::failed(Some(target), feature_error(err))
            }
        }
    }

    fn build_circular_up_to_next(
        &self,
        f: &Feature,
        target: BodyId,
        plane: Plane,
        drill_dir: Vec3,
        points: &[soliddraft_sketch::P2],
        diameter: f64,
    ) -> Built {
        let mut mine: Option<BodyId> = None;
        let run = (|| -> KernelResult<BodyId> {
            let k = &self.kernel;
            let mut body = target;
            for (i, pt) in points.iter().enumerate() {
                let profile = Profile {
                    plane,
                    outer: soliddraft_kernel_api::Loop(vec![soliddraft_kernel_api::Segment::Circle {
                        center: Point2::new(pt.u, pt.v),
                        radius: diameter / 2.0,
                    }]),
                    holes: Vec::new(),
                };
                let tag = |step: &str| OpTag::new(format!("{}.h{i}.{step}", f.id));
                let sheet = k.create_body_from_profile(&tag("profile"), &profile)?;
                let swept = k.extrude_to(&tag("sweep"), sheet.body, body, Some(drill_dir), &ExtrudeLimit::UpToNext);
                let _ = k.release(sheet.body);
                let tool = swept?;
                let cut = k.boolean(&tag("cut"), BooleanOp::Subtract, body, tool.body);
                let _ = k.release(tool.body);
                let next = cut?.body;
                if let Some(prev) = mine.replace(next) {
                    let _ = k.release(prev);
                }
                body = next;
            }
            Ok(body)
        })();
        match run {
            Ok(body) => Built {
                body_key: 0,
                consumed_tool: None,
                status: FeatureStatus::Ok,
                output: Some(body),
                owned: mine,
                names: Vec::new(),
                sketch: None,
                plane: None,
                version: 0,
                work_used: None,
                import_record: None,
            },
            Err(err) => {
                if let Some(b) = mine {
                    let _ = self.kernel.release(b);
                }
                Built::failed(Some(target), feature_error(err))
            }
        }
    }

    fn build_pattern(&self, f: &Feature, p: &PatternFeature, current: Option<BodyId>) -> Built {
        let Some(body) = current else { return Built::failed(current, "there is no body to pattern") };
        let layout = match p.layout {
            PatternSpec::Linear { direction, spacing, count } => {
                let Some(unit) = direction.normalized() else {
                    return Built::failed(current, "the pattern direction is zero");
                };
                PatternLayout::Linear { step: unit * spacing, count }
            }
            PatternSpec::Circular { axis_origin, axis_direction, count, angle } => {
                PatternLayout::Circular { axis_origin, axis_direction, count, angle_deg: angle }
            }
        };
        let tag = OpTag::new(format!("{}.pattern", f.id));
        let result = if let Some(source) = p.source {
            let Some(index) = self.built_index(source) else {
                return Built::failed(current, "The pattern source feature is missing");
            };
            let feature = &self.features[index];
            if feature.suppressed || !feature.kind.can_pattern_feature() {
                return Built::failed(current, "Select an unsuppressed boss, cut, hole or local treatment to pattern");
            }
            let Some(source) = self.built[index].output.filter(|_| self.built[index].status.is_ok()) else {
                return Built::failed(current, "The pattern source feature has not built");
            };
            let before = index.checked_sub(1).and_then(|i| self.built[i].output);
            self.kernel.pattern_feature(&tag, body, source, before, &layout)
        } else {
            self.kernel.pattern(&tag, body, &layout)
        };
        self.finish(result, current, |r| self.copies(r, body))
    }

    fn build_mirror(&self, f: &Feature, m: &MirrorFeature, current: Option<BodyId>) -> Built {
        let Some(body) = current else { return Built::failed(current, "there is no body to mirror") };
        let plane = match self.plane_of(&m.plane, body) {
            Ok(p) => p,
            Err(r) => return Built::pass(FeatureStatus::RepairNeeded(vec![*r]), current),
        };
        let params = Mirror { plane_origin: plane.origin, plane_normal: plane.normal, keep_original: m.keep_original };
        let result = self.kernel.mirror(&OpTag::new(format!("{}.mirror", f.id)), body, &params);
        self.finish(result, current, |r| self.copies(r, body))
    }

    /// Names a pattern's or mirror's copies: each copy's faces are ranked by the names of
    /// the faces of `source` they copy, so the ranks follow origins, not positions.
    fn copies(&self, r: &OpResult, source: BodyId) -> Vec<NamedFace> {
        let view = self.view_of(source).ok();
        let names = self.name_map();
        let mut faces: Vec<(u32, Option<SourceName>, FaceRole)> = Vec::new();
        for o in &r.record.face_roles {
            if let FaceRole::Copy { instance, face } = o.role {
                let key = view.as_ref().and_then(|t| reference::origin_label(face, &View { topology: t }, &names));
                faces.push((instance, key, o.role.clone()));
            }
        }
        faces.sort();
        faces.dedup();
        let mut out = Vec::new();
        let mut ordinal = 0;
        for (i, (instance, _, role)) in faces.iter().enumerate() {
            if i > 0 && faces[i - 1].0 != *instance {
                ordinal = 0;
            }
            out.push(NamedFace {
                origin: Origin { tag: r.record.tag.clone(), role: role.clone() },
                generator: Generator::Copy { instance: *instance },
                ordinal,
            });
            ordinal += 1;
        }
        let mut feature_faces = r
            .record
            .face_roles
            .iter()
            .filter_map(|o| match &o.role {
                FaceRole::FeatureCopy { instance, source } => Some((*instance, source, o.role.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        feature_faces.sort();
        feature_faces.dedup();
        let mut counts = HashMap::<u32, u32>::new();
        for (instance, _, role) in feature_faces {
            let count = counts.entry(instance).or_default();
            out.push(NamedFace {
                origin: Origin { tag: r.record.tag.clone(), role },
                generator: Generator::Copy { instance },
                ordinal: *count,
            });
            *count += 1;
        }
        out
    }

    fn build_shell(&self, f: &Feature, s: &crate::ShellFeature, current: Option<BodyId>) -> Built {
        let Some(body) = current else { return Built::failed(current, "there is no body to shell") };
        if let Err(error) = s.check_thickness() {
            return Built::failed(current, error);
        }
        if s.open_faces.is_all() {
            let thickness = s.thickness.constant().expect("validated all-faces thickness");
            return match self.kernel.shell_all_faces(body, thickness) {
                Ok(()) => Built::pass(FeatureStatus::Ok, None),
                Err(error) => Built::failed(current, feature_error(error)),
            };
        }
        let mut repairs = Vec::new();
        let mut faces = Vec::new();
        for (i, reference) in s.open_faces.picked().iter().enumerate() {
            match self.resolve_face(reference, body) {
                Ok(face) => faces.push(face),
                Err(repair) => repairs.push(RepairNeeded { slot: ReferenceSlot::Face { index: i as u32 }, ..repair }),
            }
        }
        if !repairs.is_empty() {
            return Built::pass(FeatureStatus::RepairNeeded(repairs), current);
        }
        let Some(thickness) = s.thickness.faceless() else {
            return Built::failed(current, "Wall thickness is a constant or a linear grade.");
        };
        let result = self.kernel.shell_with_grade(&OpTag::new(format!("{}.shell", f.id)), body, &faces, thickness);
        if let Ok(r) = &result
            && let Some(refusal) = shell_refusal(r)
        {
            let _ = self.kernel.release(r.body);
            return Built::failed(current, refusal);
        }
        self.finish(result, current, |r| self.distance_names(r, body, true))
    }

    fn build_offset(&self, f: &Feature, o: &crate::OffsetFeature, current: Option<BodyId>) -> Built {
        let Some(body) = current else { return Built::failed(current, "There is no body to offset") };
        let result = self.kernel.offset(&OpTag::new(format!("{}.offset", f.id)), body, o.distance);
        self.finish(result, current, |r| self.distance_names(r, body, false))
    }

    fn build_thicken(&self, f: &Feature, t: &crate::ThickenFeature, current: Option<BodyId>) -> Built {
        let Some(body) = current else { return Built::failed(current, "There is no body with faces to thicken") };
        let mut faces = Vec::new();
        let mut repairs = Vec::new();
        for (i, face) in t.faces.iter().enumerate() {
            match self.resolve_face(face, body) {
                Ok(id) => faces.push(id),
                Err(repair) => repairs.push(RepairNeeded { slot: ReferenceSlot::Face { index: i as u32 }, ..repair }),
            }
        }
        if !repairs.is_empty() {
            return Built::pass(FeatureStatus::RepairNeeded(repairs), current);
        }
        if let Err(error) = t.thickness.check_thickness() {
            return Built::failed(current, error);
        }
        let Some(thickness) = t.thickness.faceless() else {
            return Built::failed(current, "Wall thickness is a constant or a linear grade.");
        };
        let result =
            self.kernel.thicken_with_grade(&OpTag::new(format!("{}.thicken", f.id)), body, faces, thickness, t.side);
        self.finish(result, current, |r| self.distance_names(r, body, false))
    }

    fn distance_names(&self, r: &OpResult, body: BodyId, shell: bool) -> Vec<NamedFace> {
        let view = self.view_of(body).ok();
        let names = self.name_map();
        // Never rank an unnamed face by a transient kernel handle: every source face
        // must have a name, or the shell face stays unnamed.
        let labels = |faces: Vec<FaceId>| -> Option<Vec<_>> {
            let t = view.as_ref()?;
            let mut labels: Vec<_> = faces
                .into_iter()
                .map(|face| reference::origin_label(face, &View { topology: t }, &names))
                .collect::<Option<_>>()?;
            labels.sort();
            labels.dedup();
            Some(labels)
        };
        let edge_faces = |edge: EdgeId| view.as_ref().and_then(|t| t.edge(edge)).map(|e| e.faces.clone());
        let mut faces: Vec<_> = r
            .record
            .face_roles
            .iter()
            .filter_map(|o| {
                let (generator, sources) = match o.role {
                    FaceRole::Offset { face } => {
                        (if shell { Generator::ShellWall } else { Generator::OffsetFace }, vec![face])
                    }
                    FaceRole::ShellRound { edge } => {
                        (if shell { Generator::ShellRound } else { Generator::OffsetRound }, edge_faces(edge)?)
                    }
                    FaceRole::Rim { edge } => (Generator::ThickenRim, edge_faces(edge)?),
                    FaceRole::ShellCorner { vertex } => {
                        let edges = view.as_ref().and_then(|t| t.vertex(vertex))?.edges.clone();
                        let faces = edges.into_iter().map(edge_faces).collect::<Option<Vec<_>>>()?;
                        (if shell { Generator::ShellCorner } else { Generator::OffsetCorner }, faces.concat())
                    }
                    _ => return None,
                };
                Some((generator, labels(sources)?, o.role.clone()))
            })
            .collect();
        faces.sort();
        let mut counts: HashMap<Generator, u32> = HashMap::new();
        faces
            .into_iter()
            .map(|(generator, _, role)| {
                let n = counts.entry(generator.clone()).or_default();
                let ordinal = *n;
                *n += 1;
                NamedFace { origin: Origin { tag: r.record.tag.clone(), role }, generator, ordinal }
            })
            .collect()
    }

    /// What the part says about the self-intersection of imported body `id`. A body with
    /// curved faces or edges is not checked for it, and is never called validated. A
    /// planar, straight-edged body gets no claim either, until the backend's check is
    /// pinned. `None` when the import did not build.
    pub fn import_check_note(&self, id: FeatureId) -> Option<String> {
        let built = &self.built[self.built_index(id)?];
        if !matches!(self.feature(id)?.kind, FeatureKind::ImportStep(_)) || !built.status.is_ok() {
            return None;
        }
        let topology = self.view_of(built.output?).ok()?;
        let curved = topology.faces.iter().any(|f| f.surface != SurfaceKind::Plane)
            || topology.edges.iter().any(|e| e.curve != CurveKind::Line);
        curved.then(|| {
            "self-intersection not verified: this body has curved faces or edges, which are not checked for \
             self-intersection. It is not marked validated."
                .to_string()
        })
    }

    fn build_lattice(&self, f: &Feature, l: &LatticeFeature, current: Option<BodyId>) -> Built {
        let Some(body) = current else { return Built::failed(current, "there is no body to lattice") };
        let (Some(cell), Some(cell_size_mm), Some(radius_mm)) = (l.cell, l.cell_size_mm, &l.radius_mm) else {
            return Built::failed(current, "lattice is missing cell size and density; configure the cell");
        };
        let radius_mm = match self.resolve_grade(radius_mm, body) {
            Ok(grade) => grade,
            Err(repairs) => return Built::pass(FeatureStatus::RepairNeeded(repairs), current),
        };
        let params = Lattice {
            cell,
            cell_size_mm,
            radius_mm,
            frame: l.frame,
            accuracy_mm: l.accuracy_mm,
            min_detail_mm: l.min_detail_mm,
        };
        let result = self.kernel.lattice(&OpTag::new(format!("{}.lattice", f.id)), body, &params);
        self.finish_field(result, current)
    }

    fn build_gyroid(&self, f: &Feature, g: &GyroidFeature, current: Option<BodyId>) -> Built {
        let Some(body) = current else { return Built::failed(current, "there is no body for the gyroid") };
        let level = match self.resolve_grade(&g.level, body) {
            Ok(grade) => grade,
            Err(repairs) => return Built::pass(FeatureStatus::RepairNeeded(repairs), current),
        };
        let params = Gyroid {
            kind: g.kind,
            period_mm: g.period_mm,
            level,
            frame: g.frame,
            accuracy_mm: g.accuracy_mm,
            min_detail_mm: g.min_detail_mm,
        };
        let result = self.kernel.gyroid(&OpTag::new(format!("{}.gyroid", f.id)), body, &params);
        self.finish_field(result, current)
    }

    /// A field grade with its picked faces resolved on `body`, or a repair for each face
    /// that no longer resolves.
    fn resolve_grade(
        &self,
        grade: &crate::FieldGrade,
        body: BodyId,
    ) -> Result<soliddraft_kernel_api::Grade, Vec<RepairNeeded>> {
        let mut ids = Vec::new();
        let mut repairs = Vec::new();
        for (i, face) in grade.picked_faces().iter().enumerate() {
            match self.resolve_face(face, body) {
                Ok(id) => ids.push(id),
                Err(repair) => repairs.push(RepairNeeded { slot: ReferenceSlot::Face { index: i as u32 }, ..repair }),
            }
        }
        if !repairs.is_empty() {
            return Err(repairs);
        }
        Ok(grade.try_map_faces(|i, _| Ok::<_, std::convert::Infallible>(ids[i])).unwrap_or_else(|never| match never {}))
    }

    /// A terminal field names one display region and no analytic face.
    fn finish_field(&self, result: KernelResult<OpResult>, current: Option<BodyId>) -> Built {
        self.finish(result, current, |r| {
            vec![NamedFace {
                origin: Origin { tag: r.record.tag.clone(), role: FaceRole::Other },
                generator: Generator::Other,
                ordinal: 0,
            }]
        })
    }

    /// The last rebuild's repair record, including a declined import with no body.
    pub fn import_record(&self, id: FeatureId) -> Option<&soliddraft_kernel_api::RepairRecord> {
        self.built.get(self.built_index(id)?)?.import_record.as_ref()
    }

    fn build_import(&self, f: &Feature, i: &ImportStepFeature, current: Option<BodyId>) -> Built {
        if current.is_some() {
            return Built::failed(current, "an imported body must come first in the part");
        }
        if !i.hash_matches() {
            return Built::failed(current, "the imported file does not match its sha256");
        }
        let decisions = match i.kernel_decisions() {
            Ok(d) => d,
            Err(message) => return Built::failed(current, message),
        };
        let request = ImportStepRequest {
            source_solid: i.source_solid.as_deref(),
            name: &i.source_name,
            source: &i.source,
            sha256: &i.sha256,
            options: &ImportOptions::default(),
            decisions: &decisions,
            record: i.record.as_ref(),
        };
        match self.kernel.import_source(i.format, &OpTag::new(format!("{}.import", f.id)), &request) {
            Ok(mut r) => {
                let release = |r: &ImportStepResult| {
                    for b in &r.bodies {
                        let _ = self.kernel.release(b.result.body);
                    }
                };
                if !r.left_open.is_empty() {
                    release(&r);
                    // A solid the file leaves unnamed is known by its entity number, such as
                    // #178, which means nothing in the tree.
                    let numbered = |name: &String| {
                        name.strip_prefix('#').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                    };
                    let names = match r.left_open.as_slice() {
                        [one] if numbered(one) => "The solid is".to_owned(),
                        all if all.iter().all(numbered) => "The solids are".to_owned(),
                        all => all.join(", "),
                    };
                    let mut built = Built::failed(
                        current,
                        format!(
                            "{names} not imported until the gap is resolved: a declined or unanswered repair leaves the solid open and unbuilt. The original file was never changed"
                        ),
                    );
                    built.import_record = Some(r.record);
                    return built;
                }
                if r.bodies.len() != 1 {
                    release(&r);
                    return Built::failed(current, "Choose one source solid for each imported body.");
                }
                let result = r.bodies.remove(0).result;
                let body = result.body;
                Built {
                    body_key: 0,
                    consumed_tool: None,
                    status: FeatureStatus::Ok,
                    output: Some(body),
                    owned: Some(body),
                    names: imported_names(&result.record, i),
                    sketch: None,
                    plane: None,
                    version: 0,
                    work_used: None,
                    import_record: Some(r.record),
                }
            }
            Err(KernelError::Unsupported(_)) => Built::failed(current, "Coming soon"),
            Err(KernelError::InvalidInput(message)) => Built::failed(current, message),
            Err(err) => Built::failed(current, feature_error(err)),
        }
    }

    fn build_draft(&self, f: &Feature, d: &DraftFeature, current: Option<BodyId>) -> Built {
        let Some(body) = current else { return Built::failed(current, "there is no body to draft") };
        if d.faces.is_empty() {
            return Built::failed(current, "the draft has no faces");
        }
        let mut repairs = Vec::new();
        let mut faces = Vec::new();
        for (i, r) in d.faces.iter().enumerate() {
            match self.resolve_face(r, body) {
                Ok(id) => faces.push(id),
                Err(r) => repairs.push(RepairNeeded { slot: ReferenceSlot::Face { index: i as u32 }, ..r }),
            }
        }
        let neutral = match self.plane_of(&d.neutral, body) {
            Ok(p) => Some(p),
            Err(r) => {
                repairs.push(*r);
                None
            }
        };
        let chosen = match d.pull.as_ref().map(|r| self.pull_of(r, body)).transpose() {
            Ok(direction) => Some(direction),
            Err(r) => {
                repairs.push(*r);
                None
            }
        };
        let (neutral, chosen) = match (neutral, chosen) {
            (Some(neutral), Some(chosen)) if repairs.is_empty() => (neutral, chosen),
            _ => return Built::pass(FeatureStatus::RepairNeeded(repairs), current),
        };
        let pull = chosen.unwrap_or(neutral.normal);
        let pull = if d.reverse { -pull } else { pull };
        let params = Draft { pull, neutral_origin: neutral.origin, neutral_normal: neutral.normal, angle_deg: d.angle };
        let result = self.kernel.draft(&OpTag::new(format!("{}.draft", f.id)), body, &faces, &params);
        self.finish(result, current, |_| Vec::new())
    }
}

/// The name of a face a copy copies: its feature, generator and ordinal.
type SourceName = (FeatureId, Generator, u32);

pub(crate) fn kernel_repair(slot: ReferenceSlot, e: KernelError) -> RepairNeeded {
    RepairNeeded { slot, reason: RepairReason::Missing, message: e.to_string(), candidates: Vec::new() }
}

/// Ordinals: faces with the same generator are ranked by the region that made them. A
/// region makes each generator once, so the rank never depends on geometry.
fn number(mut faces: Vec<(Origin, Generator, usize)>) -> Vec<NamedFace> {
    faces.sort_by_key(|f| (f.1.clone(), f.2));
    let mut counts: HashMap<Generator, u32> = HashMap::new();
    faces
        .into_iter()
        .map(|(origin, generator, _)| {
            let n = counts.entry(generator.clone()).or_default();
            let ordinal = *n;
            *n += 1;
            NamedFace { origin, generator, ordinal }
        })
        .collect()
}

/// The faces of an imported body, ranked by their entity names in the source file:
/// `#17` before `#40` before `#102`. The saved source never changes, so neither do the
/// ranks, and the faces read "Face 1", "Face 2" and so on. A face the import inherited a
/// name for, from the file it replaced, keeps that name instead, and the others are
/// numbered from the import's first new ordinal.
fn imported_names(record: &HistoryRecord, import: &ImportStepFeature) -> Vec<NamedFace> {
    let mut faces: Vec<(u64, &str, &FaceRole)> = record
        .face_roles
        .iter()
        .filter_map(|o| match &o.role {
            FaceRole::Imported { key } => {
                let number = key.strip_prefix('#').and_then(|n| n.parse().ok()).unwrap_or(u64::MAX);
                Some((number, key.as_str(), &o.role))
            }
            _ => None,
        })
        .collect();
    faces.sort();
    faces.dedup();
    let mut next = import.new_ordinals_from;
    faces
        .into_iter()
        .map(|(_, key, role)| {
            let origin = Origin { tag: record.tag.clone(), role: role.clone() };
            match import.face_names.get(key) {
                Some(inherited) => {
                    NamedFace { origin, generator: inherited.generator.clone(), ordinal: inherited.ordinal }
                }
                None => {
                    let ordinal = next;
                    next += 1;
                    NamedFace {
                        origin,
                        generator: if import.source_bound_names {
                            Generator::Imported { sha256: import.sha256.clone().into_boxed_str() }
                        } else {
                            Generator::Other
                        },
                        ordinal,
                    }
                }
            }
        })
        .collect()
}

/// Why a shell's result is refused. A wall at least as thick as the part leaves no hollow:
/// the solid comes back as it was, with no inner wall, which is not the shell asked for.
/// The editor's preview and the rebuild refuse it alike.
pub fn shell_refusal(r: &OpResult) -> Option<&'static str> {
    let walls = r.record.face_roles.iter().any(|o| matches!(o.role, FaceRole::Offset { .. }));
    (!walls).then_some("The wall is too thick to hollow this part. Use a thinner wall.")
}

/// Distance along `dir` from `origin` to reach cleanly past `bbox`.
fn through_reach(bbox: &Aabb, origin: Vec3, dir: Vec3) -> f64 {
    let (lo, hi) = (bbox.min, bbox.max);
    let reach = (0..8)
        .map(|c| {
            let corner = Vec3::new(
                if c & 1 == 0 { lo.x } else { hi.x },
                if c & 2 == 0 { lo.y } else { hi.y },
                if c & 4 == 0 { lo.z } else { hi.z },
            );
            (corner - origin).dot(dir)
        })
        .fold(f64::NEG_INFINITY, f64::max);
    if reach <= 1e-9 { 0.0 } else { reach + 1.0 + 0.01 * bbox.diagonal() }
}

/// Exact stadium (oblong slot) closed CCW profile centred at `(cu, cv)` on `plane`,
/// with width `width` (diameter of end semicircles), centre-to-centre `length`, rotated
/// counter-clockwise by `angle_deg` degrees.
fn stadium_profile(plane: Plane, cu: f64, cv: f64, width: f64, length: f64, angle_deg: f64) -> Profile {
    let norm = angle_deg.rem_euclid(360.0);
    let (cos, sin) = if (norm - 0.0).abs() < 1e-12 || (norm - 360.0).abs() < 1e-12 {
        (1.0, 0.0)
    } else if (norm - 90.0).abs() < 1e-12 {
        (0.0, 1.0)
    } else if (norm - 180.0).abs() < 1e-12 {
        (-1.0, 0.0)
    } else if (norm - 270.0).abs() < 1e-12 {
        (0.0, -1.0)
    } else {
        let rad = angle_deg.to_radians();
        (rad.cos(), rad.sin())
    };
    let r = width * 0.5;
    let half = length * 0.5;
    let c1 = Point2::new(cu - cos * half, cv - sin * half);
    let c2 = Point2::new(cu + cos * half, cv + sin * half);
    let perp_u = -sin * r;
    let perp_v = cos * r;
    let p1 = Point2::new(c1.u - perp_u, c1.v - perp_v);
    let p2 = Point2::new(c2.u - perp_u, c2.v - perp_v);
    let p3 = Point2::new(c2.u + perp_u, c2.v + perp_v);
    let p4 = Point2::new(c1.u + perp_u, c1.v + perp_v);
    use soliddraft_kernel_api::{Loop, Segment};
    Profile {
        plane,
        outer: Loop(vec![
            Segment::Line { from: p1, to: p2 },
            Segment::Arc { center: c2, from: p2, to: p3, clockwise: false },
            Segment::Line { from: p3, to: p4 },
            Segment::Arc { center: c1, from: p4, to: p1, clockwise: false },
        ]),
        holes: Vec::new(),
    }
}

/// The sketch plane on a planar face: origin at the world origin projected onto the
/// face, X horizontal for a face that is not horizontal, else the world X axis. The
/// Catallax adapter builds this frame again in exact numbers for hole centres.
pub fn plane_on_face(normal: Vec3, point: Vec3) -> Plane {
    let n = normal;
    let x_dir = if n.z.abs() < 1.0 - 1e-9 {
        Vec3::Z.cross(n).normalized().expect("not parallel to Z")
    } else {
        (Vec3::X - n * Vec3::X.dot(n)).normalized().expect("parallel to Z")
    };
    Plane { origin: n * point.dot(n), normal: n, x_dir }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_planes_have_upright_frames() {
        let p = plane_on_face(-Vec3::Y, Vec3::new(3.0, -45.0, 7.0));
        assert!(p.is_well_formed());
        assert!(p.origin.distance(Vec3::new(0.0, -45.0, 0.0)) < 1e-12);
        assert!(p.x_dir.distance(Vec3::X) < 1e-12 && p.y_dir().distance(Vec3::Z) < 1e-12);
        let top = plane_on_face(Vec3::Z, Vec3::new(5.0, 5.0, 10.0));
        assert!(top.origin.distance(Vec3::new(0.0, 0.0, 10.0)) < 1e-12);
        assert!(top.x_dir.distance(Vec3::X) < 1e-12 && top.y_dir().distance(Vec3::Y) < 1e-12);
        let right = plane_on_face(Vec3::X, Vec3::new(60.0, 1.0, 1.0));
        assert!(right.x_dir.distance(Vec3::Y) < 1e-12 && right.y_dir().distance(Vec3::Z) < 1e-12);
    }
}
