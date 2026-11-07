//! Feature evaluation and rebuild scheduling.

use std::collections::HashMap;
use std::sync::Arc;

use soliddraft_kernel_api::{
    BodyId, BooleanOp, CurveKind, Draft, EdgeId, Extrude, ExtrudeLimit, FaceId, FacePoint, FaceRole, FilletPolicy,
    Gyroid, HistoryRecord, Hole, HolePlacement, ImportOptions, ImportStepRequest, ImportStepResult, Kernel,
    KernelError, KernelResult, Lattice, Mirror, OpResult, OpTag, Origin, PatternLayout, Plane, RadiusLaw, SurfaceKind,
    Vec3,
};

use crate::feature::{
    BlendFeature, DraftFeature, EndCondition, ExtrudeFeature, ExtrudeOp, Feature, FeatureId, FeatureKind, FilletCorner,
    FilletLaw, FilletOverflow, FilletSetback, GyroidFeature, HoleFeature, ImportStepFeature, LatticeFeature,
    MirrorFeature, PatternFeature, PatternSpec, PlaneRef, ProfileSource, RevolveFeature, SketchFeature, SketchPlane,
    VARIABLE_CORNERS,
};
use crate::rebuild::BuildCache;
use crate::reference::{self, FaceRef, Generator, NamedFace, ReferenceSlot, RepairNeeded, RepairReason, View};

use crate::document::{BuiltSketch, Document, FeatureStatus};

/// One feature's rebuild result.
#[derive(Clone, Debug)]
pub(crate) struct Built {
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
    pub import_record: Option<soliddraft_kernel_api::RepairRecord>,
}

impl Built {
    fn pass(status: FeatureStatus, current: Option<BodyId>) -> Built {
        Built {
            status,
            output: current,
            owned: None,
            names: Vec::new(),
            sketch: None,
            plane: None,
            version: 0,
            import_record: None,
        }
    }

    fn failed(current: Option<BodyId>, message: impl Into<String>) -> Built {
        Built::pass(FeatureStatus::Failed(message.into()), current)
    }
}

fn feature_error(error: KernelError) -> String {
    match error {
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
            next_id: self.next_id,
            materials: self.materials.clone(),
            settings: self.settings.clone(),
            view: self.view.clone(),
            kernel: self.kernel.clone(),
            built: self.built.clone(),
            dirty: self.dirty,
            cache: self.cache.clone(),
            undo: Vec::new(),
            redo: Vec::new(),
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
        let mut current = start.checked_sub(1).and_then(|i| self.built[i].output);
        for i in start..self.features.len() {
            let f = self.features[i].clone();
            let built = if i >= active {
                Built::pass(FeatureStatus::RolledBack, current)
            } else if f.suppressed {
                Built::pass(FeatureStatus::Suppressed, current)
            } else {
                self.build(&f, current, &mut cache)
            };
            current = built.output;
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

    /// A feature's build: the kept one when the cache has its exact inputs, else a new
    /// one, kept. Checks whose messages name another feature run every time and are not
    /// kept, so a rename shows in them without building anything.
    fn build(&self, f: &Feature, current: Option<BodyId>, cache: &mut BuildCache) -> Built {
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
            FeatureKind::Sweep(crate::SweepFeature { sketch: feature, .. })
            | FeatureKind::Revolve(RevolveFeature { sketch: feature, .. })
            | FeatureKind::Extrude(ExtrudeFeature { profile: ProfileSource::Sketch { feature }, .. }) => {
                match self.profile_input(*feature, current) {
                    Ok(input) => Some(input.version),
                    Err(refused) => return *refused,
                }
            }
            _ => named_plane.and_then(|(id, _)| self.built_index(id).map(|i| self.built[i].version)),
        };
        let key = cache.key(f, current, profile);
        if let Some(built) = cache.get(&key) {
            return built;
        }
        let built = match &f.kind {
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
            FeatureKind::Sweep(s) => {
                self.build_profile_tool(f, s.sketch, current, |tag, profile| self.kernel.sweep(tag, profile, &s.path))
            }
            FeatureKind::Fillet(b) => self.build_blend(f, b, false, current),
            FeatureKind::Chamfer(b) => self.build_blend(f, b, true, current),
            FeatureKind::Hole(h) => self.build_hole(f, h, current),
            FeatureKind::Pattern(p) => self.build_pattern(f, p, current),
            FeatureKind::Mirror(m) => self.build_mirror(f, m, current),
            FeatureKind::Draft(d) => self.build_draft(f, d, current),
            FeatureKind::Shell(s) => self.build_shell(f, s, current),
            FeatureKind::ImportStep(i) => self.build_import(f, i, current),
            FeatureKind::Lattice(l) => self.build_lattice(f, l, current),
            FeatureKind::Gyroid(g) => self.build_gyroid(f, g, current),
        };
        cache.insert(key, built, &self.kernel)
    }

    /// The built sketch an extrusion sweeps, or the extrusion's result when that sketch
    /// is missing, not built or has no closed region.
    fn profile_input(&self, sketch: FeatureId, current: Option<BodyId>) -> Result<&Built, Box<Built>> {
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
            Ok(r) if !r.is_empty() => Ok(input),
            Ok(_) => failed(format!("{} has no closed region", sketch_name())),
            Err(err) => failed(format!("{} has no closed region: {err}", sketch_name())),
        }
    }

    fn sketch_plane_of(&self, source: &SketchPlane, current: Option<BodyId>) -> Result<(Plane, bool), Box<Built>> {
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
                (plane.with_origin(plane.origin + plane.normal * *offset), false)
            }
            SketchPlane::Datum { datum, offset } => {
                let p = datum.plane();
                (p.with_origin(p.origin + p.normal * *offset), false)
            }
            SketchPlane::Face { face } | SketchPlane::FaceOffset { face, .. } => {
                let offset = source.offset();
                let repair = |reason, message: String, candidates| {
                    let r = RepairNeeded { slot: ReferenceSlot::SketchPlane, reason, message, candidates };
                    Err(Box::new(Built::pass(FeatureStatus::RepairNeeded(vec![r]), current)))
                };
                let Some(body) = current else {
                    return repair(RepairReason::Missing, "there is no part body to sketch on".into(), vec![]);
                };
                // A sketch plane needs no mesh of the body before it unless this cannot answer.
                if let Ok(carriers) = self.kernel.face_carriers(body)
                    && let Some((normal, point)) = reference::carrier_plane(face, &carriers, &self.name_map())
                {
                    let p = plane_on_face(normal, point);
                    return Ok((p.with_origin(p.origin + p.normal * offset), offset == 0.0));
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
                (p.with_origin(p.origin + p.normal * offset), offset == 0.0)
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
        let (plane, on_face) = match self.sketch_plane_of(&s.plane, current) {
            Ok(plane) => plane,
            Err(built) => return *built,
        };
        let regions = s.sketch.regions().map_err(|e| e.to_string());
        Built {
            status: FeatureStatus::Ok,
            output: current,
            owned: None,
            names: Vec::new(),
            sketch: Some(BuiltSketch { plane, on_face, regions }),
            plane: None,
            version: 0,
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
                status: FeatureStatus::Ok,
                output,
                owned: mine,
                names: number(faces),
                sketch: None,
                plane: None,
                version: 0,
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

    fn build_extrude(&self, f: &Feature, e: &ExtrudeFeature, current: Option<BodyId>) -> Built {
        let shape_sketch;
        let sk = match &e.profile {
            ProfileSource::Sketch { feature } => match self.profile_input(*feature, current) {
                Ok(input) => input.sketch.as_ref().expect("a profile input is a built sketch"),
                Err(refused) => return *refused,
            },
            ProfileSource::Shape { datum, shape } => {
                let regions = shape.sketch().and_then(|s| s.regions().map_err(|e| e.to_string()));
                shape_sketch = BuiltSketch { plane: datum.plane(), on_face: false, regions };
                &shape_sketch
            }
        };
        let regions = match &sk.regions {
            Ok(r) if !r.is_empty() => r,
            Ok(_) => return Built::failed(current, "the built-in profile has no closed region"),
            Err(err) => return Built::failed(current, format!("the built-in profile has no closed region: {err}")),
        };
        let dir = if e.reverse { -sk.plane.normal } else { sk.plane.normal };
        // The tool starts exactly on its sketch plane, also on a face of the part; the
        // kernel's coplanar Boolean rules decide the shared face.
        let start = sk.plane;
        let limit = match &e.end {
            EndCondition::UpToNext => Some(ExtrudeLimit::UpToNext),
            EndCondition::UpToFace { face } => {
                let Some(body) = current else {
                    return Built::failed(current, "an extrusion up to a face needs a body");
                };
                match self.resolve_face(face, body) {
                    Ok(face) => Some(ExtrudeLimit::UpToFace { face }),
                    Err(r) => {
                        let r = RepairNeeded { slot: ReferenceSlot::Limit, ..r };
                        return Built::pass(FeatureStatus::RepairNeeded(vec![r]), current);
                    }
                }
            }
            _ => None,
        };
        if limit.is_some() && current.is_none() {
            return Built::failed(current, "an extrusion up to the next face needs a body");
        }
        let distance = match e.end {
            EndCondition::Blind { depth } if depth.is_finite() && depth > 0.0 => depth,
            EndCondition::Blind { depth } => {
                return Built::failed(current, format!("depth must be positive, got {depth}"));
            }
            EndCondition::ThroughAll => {
                let Some(body) = current else {
                    return Built::failed(current, "through all needs a body to go through");
                };
                let bbox = match self.kernel.topology(body) {
                    Ok(t) => t.bbox,
                    Err(err) => return Built::failed(current, feature_error(err)),
                };
                let (lo, hi) = (bbox.min, bbox.max);
                let reach = (0..8)
                    .map(|c| {
                        let corner = Vec3::new(
                            if c & 1 == 0 { lo.x } else { hi.x },
                            if c & 2 == 0 { lo.y } else { hi.y },
                            if c & 4 == 0 { lo.z } else { hi.z },
                        );
                        (corner - start.origin).dot(dir)
                    })
                    .fold(f64::NEG_INFINITY, f64::max);
                if reach <= 1e-9 {
                    return Built::failed(current, "the body lies entirely behind the sketch plane");
                }
                reach + 1.0 + 0.01 * bbox.diagonal()
            }
            EndCondition::UpToNext | EndCondition::UpToFace { .. } => 0.0,
        };
        let mut mine: Option<BodyId> = None;
        let mut faces: Vec<(Origin, Generator, usize)> = Vec::new();
        let mut run = |mine: &mut Option<BodyId>| -> KernelResult<Option<BodyId>> {
            let k = &self.kernel;
            let mut body = current;
            for (i, region) in regions.iter().enumerate() {
                let tag = |step: &str| OpTag::new(format!("{}.r{i}.{step}", f.id));
                let sheet = k.create_body_from_profile(&tag("profile"), &region.to_profile(start))?;
                let tool = match (&limit, current) {
                    (Some(limit), Some(target)) => k.extrude_to(&tag("sweep"), sheet.body, target, Some(dir), limit),
                    _ => k.extrude(&tag("sweep"), sheet.body, &Extrude { distance, direction: Some(dir) }),
                };
                let _ = k.release(sheet.body);
                let tool = tool?;
                for o in &tool.record.face_roles {
                    let generator = match o.role {
                        FaceRole::StartCap => Some(Generator::StartCap),
                        FaceRole::EndCap => Some(Generator::EndCap),
                        FaceRole::Side { loop_index, segment, .. } => region
                            .entity(loop_index as usize, segment as usize)
                            .map(|entity| Generator::Side { entity, path: None }),
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
            Ok(body)
        };
        match run(&mut mine) {
            Ok(output) => Built {
                status: FeatureStatus::Ok,
                output,
                owned: mine,
                names: number(faces),
                sketch: None,
                plane: None,
                version: 0,
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
            // Station faces that are gone ask for repair in the same round as the edges.
            let stations = match &b.overflow {
                Some(FilletOverflow::KeepNeighbours(stations)) => stations.as_slice(),
                _ => &[],
            };
            let ends = stations.iter().enumerate().map(|(i, s)| (&s.at, ReferenceSlot::Setback { index: i as u32 }));
            let laws = b.laws.iter().enumerate().map(|(i, l)| (&l.from, ReferenceSlot::Law { index: i as u32 }));
            for (at, slot) in ends.chain(laws) {
                if let Err(u) = reference::resolve_face(at, &view, &names) {
                    repairs.push(RepairNeeded { message: format!("{slot}: {}", u.message), ..u.into_repair(slot) });
                }
            }
            return Built::pass(FeatureStatus::RepairNeeded(repairs), current);
        }
        for (i, id) in ids.iter().enumerate() {
            if let Some(j) = ids[..i].iter().position(|x| x == id) {
                return Built::failed(current, format!("edges {} and {} name the same edge", j + 1, i + 1));
            }
        }
        let tag = OpTag::new(format!("{}.blend", f.id));
        let result = if chamfer {
            self.kernel.chamfer(&tag, body, &ids, b.size)
        } else {
            match placed(current, fillet(self.kernel.as_ref(), &tag, b, &ids, body, &t, &names)) {
                Ok(result) => result,
                Err(built) => return *built,
            }
        };
        let result = match result {
            Ok(r) => r,
            Err(err) => return Built::failed(current, feature_error(err)),
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
            status: FeatureStatus::Ok,
            output: Some(result.body),
            owned: Some(result.body),
            names: number(faces),
            sketch: None,
            plane: None,
            version: 0,
            import_record: None,
        }
    }

    /// Resolves a feature's plane: a reference plane, or a planar face of `body` with
    /// its outward normal.
    fn plane_of(&self, r: &PlaneRef, body: BodyId) -> Result<Plane, Box<RepairNeeded>> {
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
                    .resolve_face(face, body)
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

    /// The result of an operation that makes a new body from the part's body, with
    /// the faces it made named.
    fn finish(
        &self,
        result: KernelResult<OpResult>,
        current: Option<BodyId>,
        name: impl Fn(&OpResult) -> Vec<NamedFace>,
    ) -> Built {
        match result {
            Ok(r) => Built {
                status: FeatureStatus::Ok,
                output: Some(r.body),
                owned: Some(r.body),
                names: name(&r),
                sketch: None,
                plane: None,
                version: 0,
                import_record: None,
            },
            Err(err) => Built::failed(current, feature_error(err)),
        }
    }

    fn build_hole(&self, f: &Feature, h: &HoleFeature, current: Option<BodyId>) -> Built {
        let Some(body) = current else { return Built::failed(current, "there is no body to drill") };
        if h.points.is_empty() {
            return Built::failed(current, "the hole has no points");
        }
        let plane = match self.plane_of(&PlaneRef::Face { face: h.face.clone() }, body) {
            Ok(p) => p,
            Err(r) => return Built::pass(FeatureStatus::RepairNeeded(vec![*r]), current),
        };
        let face = self.resolve_face(&h.face, body).ok();
        let placements = h
            .points
            .iter()
            .map(|p| HolePlacement {
                point: plane.origin + plane.x_dir * p.u + plane.y_dir() * p.v,
                direction: -plane.normal,
                on_face: face.map(|face| FacePoint { face, u: p.u, v: p.v }),
            })
            .collect();
        let params = Hole { placements, diameter: h.diameter, depth: h.depth, tip: h.tip, head: h.head };
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
        let result = self.kernel.pattern(&OpTag::new(format!("{}.pattern", f.id)), body, &layout);
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
        out
    }

    fn build_shell(&self, f: &Feature, s: &crate::ShellFeature, current: Option<BodyId>) -> Built {
        let Some(body) = current else { return Built::failed(current, "there is no body to shell") };
        if s.open_faces.is_all() {
            return match self.kernel.shell_all_faces(body, s.thickness) {
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
        let result = self.kernel.shell(&OpTag::new(format!("{}.shell", f.id)), body, &faces, s.thickness);
        if let Ok(r) = &result
            && let Some(refusal) = shell_refusal(r)
        {
            let _ = self.kernel.release(r.body);
            return Built::failed(current, refusal);
        }
        self.finish(result, current, |r| {
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
                        FaceRole::Offset { face } => (Generator::ShellWall, vec![face]),
                        FaceRole::ShellRound { edge } => (Generator::ShellRound, edge_faces(edge)?),
                        FaceRole::ShellCorner { vertex } => {
                            let edges = view.as_ref().and_then(|t| t.vertex(vertex))?.edges.clone();
                            let faces = edges.into_iter().map(edge_faces).collect::<Option<Vec<_>>>()?;
                            (Generator::ShellCorner, faces.concat())
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
        })
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
        let (Some(cell), Some(cell_size_mm), Some(radius_mm)) = (l.cell, l.cell_size_mm, l.radius_mm) else {
            return Built::failed(current, "lattice is missing cell size and density; configure the cubic cell");
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
        let params = Gyroid {
            kind: g.kind,
            period_mm: g.period_mm,
            level: g.level,
            frame: g.frame,
            accuracy_mm: g.accuracy_mm,
            min_detail_mm: g.min_detail_mm,
        };
        let result = self.kernel.gyroid(&OpTag::new(format!("{}.gyroid", f.id)), body, &params);
        self.finish_field(result, current)
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
            name: &i.source_name,
            source: &i.source,
            sha256: &i.sha256,
            options: &ImportOptions::default(),
            decisions: &decisions,
            record: i.record.as_ref(),
        };
        match self.kernel.import_step(&OpTag::new(format!("{}.import", f.id)), &request) {
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
                    let n = r.bodies.len();
                    return Built::failed(current, format!("the file holds {n} solids; a part holds one body"));
                }
                let result = r.bodies.remove(0).result;
                let body = result.body;
                Built {
                    status: FeatureStatus::Ok,
                    output: Some(body),
                    owned: Some(body),
                    names: imported_names(&result.record),
                    sketch: None,
                    plane: None,
                    version: 0,
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
        let Some(neutral) = neutral.filter(|_| repairs.is_empty()) else {
            return Built::pass(FeatureStatus::RepairNeeded(repairs), current);
        };
        let pull = if d.reverse { -neutral.normal } else { neutral.normal };
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
/// ranks, and the faces read "Face 1", "Face 2" and so on.
fn imported_names(record: &HistoryRecord) -> Vec<NamedFace> {
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
    faces
        .into_iter()
        .enumerate()
        .map(|(ordinal, (_, _, role))| NamedFace {
            origin: Origin { tag: record.tag.clone(), role: role.clone() },
            generator: Generator::Other,
            ordinal: ordinal as u32,
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

/// Why a fillet's setback stations or radius laws do not place on the body being
/// rounded.
#[derive(Debug)]
pub(crate) enum EndError {
    /// The definition is wrong in a way no repair pick fixes.
    Failed(String),
    /// Some end faces are gone, or do not meet their edge at exactly one end.
    Repair(Vec<RepairNeeded>),
}

impl EndError {
    /// The problem as one line, for a preview that cannot open a repair.
    pub(crate) fn message(&self) -> String {
        match self {
            EndError::Failed(why) => why.clone(),
            EndError::Repair(items) => items.iter().map(|r| r.message.as_str()).collect::<Vec<_>>().join("; "),
        }
    }
}

/// The end of `edge` that `at` names: the endpoint whose other edges run into that face.
/// A face that is gone, or meets the edge at both ends or neither, asks for repair in
/// `slot`; the faces that meet one end each are offered in its place. `what` names the
/// reference in messages, such as "setback 1".
fn edge_end(
    edge: EdgeId,
    at: &FaceRef,
    slot: ReferenceSlot,
    what: &str,
    topology: &soliddraft_kernel_api::Topology,
    view: &View,
    names: &reference::NameMap,
) -> Result<soliddraft_kernel_api::VertexId, EndError> {
    let Some(info) = topology.edge(edge) else {
        return Err(EndError::Failed(format!("{what}: its edge is not on the body")));
    };
    // The faces an end runs into: those of its other edges that are not the edge's own.
    let end_faces = |v: &soliddraft_kernel_api::VertexId| -> Vec<FaceId> {
        let mut faces: Vec<FaceId> = topology
            .vertex(*v)
            .into_iter()
            .flat_map(|vertex| vertex.edges.iter())
            .filter(|e| **e != edge)
            .filter_map(|e| topology.edge(*e))
            .flat_map(|other| other.faces.iter().copied())
            .filter(|f| !info.faces.contains(f))
            .collect();
        faces.sort_unstable();
        faces.dedup();
        faces
    };
    let ends: Vec<Vec<FaceId>> = info.vertices.iter().map(end_faces).collect();
    let face = reference::resolve_face(at, view, names).map_err(|u| {
        EndError::Repair(vec![RepairNeeded { message: format!("{what}: {}", u.message), ..u.into_repair(slot) }])
    })?;
    let meets: Vec<usize> = (0..ends.len()).filter(|k| ends[*k].contains(&face)).collect();
    if let [k] = meets.as_slice() {
        return Ok(info.vertices[*k]);
    }
    let detail = format!("{} does not meet its edge at exactly one end", at.label);
    let mut candidates = Vec::new();
    for (k, faces) in ends.iter().enumerate() {
        for f in faces {
            let elsewhere = ends.iter().enumerate().any(|(j, other)| j != k && other.contains(f));
            if !elsewhere && !candidates.contains(&reference::Candidate::Face(*f)) {
                candidates.push(reference::Candidate::Face(*f));
            }
        }
    }
    Err(EndError::Repair(vec![RepairNeeded {
        slot,
        message: format!("{what}: {detail}"),
        reason: RepairReason::Mismatch { detail },
        candidates,
    }]))
}

/// The kernel call that rounds `body`, given the fillet's edges resolved there. A fillet
/// with radius laws is variable; otherwise its overflow and corner choices pick the
/// call, and a fillet with neither is the kernel's plain fillet. The outer error is the
/// definition's; the inner one is the kernel's.
pub(crate) fn fillet(
    kernel: &dyn Kernel,
    tag: &OpTag,
    b: &BlendFeature,
    ids: &[EdgeId],
    body: BodyId,
    topology: &soliddraft_kernel_api::Topology,
    names: &reference::NameMap,
) -> Result<KernelResult<OpResult>, EndError> {
    let view = View { topology };
    if !b.laws.is_empty() {
        if matches!(b.overflow, Some(FilletOverflow::KeepNeighbours(_))) {
            return Err(EndError::Failed(
                "a variable-radius fillet rolls over; it cannot keep its neighbours yet".into(),
            ));
        }
        if b.corner == Some(FilletCorner::Smooth) {
            return Err(EndError::Failed(VARIABLE_CORNERS.into()));
        }
        let laws = variable_edges(&b.laws, ids, b.size, topology, &view, names)?;
        return Ok(kernel.fillet_variable(tag, body, &laws));
    }
    let overflow = match &b.overflow {
        None => None,
        Some(FilletOverflow::RollOver) => Some(soliddraft_kernel_api::FilletOverflow::RollOver),
        Some(FilletOverflow::KeepNeighbours(stations)) => Some(soliddraft_kernel_api::FilletOverflow::KeepNeighbours(
            setbacks(stations, ids, topology, &view, names)?,
        )),
    };
    Ok(match (overflow, b.corner) {
        (None, None) => kernel.fillet(tag, body, ids, b.size),
        (overflow, corner) => kernel.fillet_with(
            tag,
            body,
            ids,
            b.size,
            &FilletPolicy { overflow: overflow.unwrap_or_default(), corner: corner.unwrap_or_default() },
        ),
    })
}

/// A placement, or what the feature reports when it cannot be placed.
fn placed<T>(current: Option<BodyId>, r: Result<T, EndError>) -> Result<T, Box<Built>> {
    match r {
        Ok(v) => Ok(v),
        Err(EndError::Failed(why)) => Err(Box::new(Built::failed(current, why))),
        Err(EndError::Repair(items)) => Err(Box::new(Built::pass(FeatureStatus::RepairNeeded(items), current))),
    }
}

/// Gathers per-item results, keeping every repair so one round asks for all of them. A
/// failure that no repair fixes stops at once.
fn gather<T>(items: impl Iterator<Item = Result<T, EndError>>) -> Result<Vec<T>, EndError> {
    let mut placed = Vec::new();
    let mut repairs = Vec::new();
    for item in items {
        match item {
            Ok(t) => placed.push(t),
            Err(EndError::Repair(r)) => repairs.extend(r),
            Err(failed) => return Err(failed),
        }
    }
    if repairs.is_empty() { Ok(placed) } else { Err(EndError::Repair(repairs)) }
}

/// A fillet's setback stations on the body being rounded: each one's edge among the
/// resolved `edges`, and the end of that edge that touches its `at` face.
pub(crate) fn setbacks(
    stations: &[FilletSetback],
    edges: &[EdgeId],
    topology: &soliddraft_kernel_api::Topology,
    view: &View,
    names: &reference::NameMap,
) -> Result<Vec<soliddraft_kernel_api::FilletSetback>, EndError> {
    gather(stations.iter().enumerate().map(|(i, s)| {
        let what = format!("setback {}", i + 1);
        let &edge = edges
            .get(s.edge as usize)
            .ok_or_else(|| EndError::Failed(format!("{what} names no edge of the fillet")))?;
        let slot = ReferenceSlot::Setback { index: i as u32 };
        let at = edge_end(edge, &s.at, slot, &what, topology, view, names)?;
        Ok(soliddraft_kernel_api::FilletSetback { edge, at, setback: s.setback })
    }))
}

/// A variable fillet's edges on the body being rounded, in the fillet's edge order. An
/// edge with a law starts it at the end its `from` face names; one without keeps the
/// constant `size`, which reads the same from either end. Every edge must be open.
pub(crate) fn variable_edges(
    laws: &[FilletLaw],
    edges: &[EdgeId],
    size: f64,
    topology: &soliddraft_kernel_api::Topology,
    view: &View,
    names: &reference::NameMap,
) -> Result<Vec<soliddraft_kernel_api::VariableFilletEdge>, EndError> {
    if let Some((i, law)) = laws.iter().enumerate().find(|(_, l)| l.edge as usize >= edges.len()) {
        return Err(EndError::Failed(format!(
            "radius law {} names edge {}, which the fillet does not have",
            i + 1,
            law.edge + 1
        )));
    }
    gather(edges.iter().enumerate().map(|(e, &edge)| {
        let vertices = topology.edge(edge).map(|info| info.vertices.clone()).unwrap_or_default();
        let [first, _] = vertices.as_slice() else {
            return Err(EndError::Failed(format!(
                "edge {} is closed; a variable-radius fillet rounds open edges only",
                e + 1
            )));
        };
        match laws.iter().enumerate().find(|(_, l)| l.edge as usize == e) {
            Some((i, law)) => {
                let what = format!("radius law {}", i + 1);
                let slot = ReferenceSlot::Law { index: i as u32 };
                let from = edge_end(edge, &law.from, slot, &what, topology, view, names)?;
                Ok(soliddraft_kernel_api::VariableFilletEdge { edge, from, radius: law.radius })
            }
            None => Ok(soliddraft_kernel_api::VariableFilletEdge {
                edge,
                from: *first,
                radius: RadiusLaw::Linear { start: size, end: size },
            }),
        }
    }))
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
