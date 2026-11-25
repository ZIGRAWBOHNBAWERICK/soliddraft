//! The current proved part as a `soliddraft-drawing` [`Part`], named through its
//! persistent references, with the interface's entity names, hole callouts and
//! feature dimensions.

use crate::document::{Document, FeatureKind};
use soliddraft_drawing::library::{self, Symbols};
use soliddraft_drawing::measure::format_value;
use soliddraft_drawing::project::{CircleInfo, Projected};
use soliddraft_drawing::{ContentHash, DocumentId};
use soliddraft_drawing::{
    Entity, EntityKind, EntityRef, KernelBody, ModelDimension, ModelEntity, ModelMeasure, Namer, Part, PartSource,
    Resolution, feature_name,
};
use soliddraft_kernel_api::{BodyId, EdgeId, FaceId, Kernel, Point2, Topology, Vec3};
use std::sync::Arc;

/// The current proved body and the definitions that produced it.
pub struct PartContext<'a> {
    pub doc: &'a Document,
    pub kernel: Arc<dyn Kernel>,
    pub body: BodyId,
    pub topology: Topology,
    /// The part's id.
    pub id: DocumentId,
    /// The part's revision now, unsaved edits included.
    pub digest: ContentHash,
    pub name: String,
}

impl<'a> PartContext<'a> {
    /// The current proved part, or `None` while its revision is unproved or without a body.
    pub fn new(doc: &'a Document) -> Option<PartContext<'a>> {
        if !doc.proof_state().current_proved_model() {
            return None;
        }
        let body = doc.model().body()?;
        let kernel = doc.kernel().clone();
        let topology = kernel.topology(body).ok()?;
        Some(PartContext {
            doc,
            kernel,
            body,
            topology,
            id: doc.model().id(),
            digest: digest(doc),
            name: doc.name.clone(),
        })
    }

    /// The part as `soliddraft-drawing` takes it for Catallax line work.
    pub fn part(&self) -> Part<'_> {
        Part {
            geometry: Box::new(KernelBody::new(self.kernel.as_ref(), self.body)),
            digest: self.digest.clone(),
            dimensions: feature_dimensions(self.doc),
            namer: Some(self),
            preview: false,
            source: PartSource {
                configuration: self.doc.model().active_configuration().cloned(),
                properties: self.doc.model().resolved_properties(),
            },
            requirements: soliddraft_drawing::Requirements {
                rows: self.doc.model().pmi_rows(),
                notes: self.doc.model().part_notes(),
            },
        }
    }

    fn feature_name(&self, id: u64) -> String {
        self.doc.feature(id).map(|f| f.name.clone()).unwrap_or_else(|| format!("Feature {id}"))
    }

    /// The feature an entity came from: its id, its name, and whether it is still there
    /// and not suppressed. A feature that is gone is named by what the reference
    /// remembered of it.
    pub fn feature_of(&self, entity: &EntityRef) -> Option<(u64, String, bool)> {
        let id = entity.feature.strip_prefix('f').and_then(|id| id.parse::<u64>().ok()).filter(|id| *id != 0).or_else(
            || match &entity.model {
                Some(ModelEntity::Edge(e)) => Some(u64::from(e.faces[0].feature.0)),
                Some(ModelEntity::Face(f)) => Some(u64::from(f.feature.0)),
                None => soliddraft_model::feature_of_tag(&entity.feature).map(|f| u64::from(f.0)),
            },
        )?;
        match self.doc.feature(id) {
            Some(f) if !f.suppressed => Some((id, f.name.clone(), true)),
            _ => Some((id, feature_name(entity), false)),
        }
    }

    /// The feature a hole callout on a circle reads, and for a hole feature which of its
    /// holes the circle is on. A circle between a face of a hole and a face of another
    /// feature, such as the rim a later cut left on a countersink, belongs to the hole.
    fn callout_feature(&self, circle: &CircleInfo) -> Option<(u64, Option<usize>)> {
        if let Some(ModelEntity::Edge(edge)) = &circle.entity.model {
            let hole = edge.faces.iter().find_map(|face| match face.generator {
                soliddraft_model::Generator::Hole { placement, .. } => {
                    let id = u64::from(face.feature.0);
                    self.doc
                        .feature(id)
                        .is_some_and(|f| !f.suppressed && matches!(f.kind, FeatureKind::Hole(_)))
                        .then_some((id, Some(placement as usize)))
                }
                _ => None,
            });
            if hole.is_some() {
                return hole;
            }
        }
        self.feature_of(&circle.entity).map(|(id, _, _)| (id, None))
    }
}

impl Namer for PartContext<'_> {
    /// The model's persistent name for the edge, with the words the interface uses
    /// ("Plate · Edge 3"), so a reference can still be described after its feature is
    /// gone.
    fn edge_ref(&self, id: EdgeId) -> Option<EntityRef> {
        let e = self.topology.edges.iter().find(|e| e.id == id)?;
        // Without a persistent name, let the drawing index use its fingerprint name.
        let model = self.doc.model().edge_ref(id).ok()?;
        let named = self.doc.named_edge(id);
        let feature = named.map(|n| n.feature).unwrap_or(u64::from(model.faces[0].feature.0));
        let label = named.map(|n| n.label.clone()).unwrap_or_else(|| "Edge".into());
        let role = format!("{} · {label}", self.feature_name(feature));
        let mut r = EntityRef::new(EntityKind::Edge, &format!("f{feature}"), &role, 0, e.midpoint, e.length);
        r.model = Some(ModelEntity::Edge(model));
        Some(r)
    }

    fn face_ref(&self, id: FaceId) -> Option<EntityRef> {
        let f = self.topology.faces.iter().find(|f| f.id == id)?;
        let model = self.doc.model().face_ref(id).ok()?;
        let named = self.doc.named_face(id);
        let feature = named.map(|n| n.feature).unwrap_or(u64::from(model.feature.0));
        let label = named.map(|n| n.label.clone()).unwrap_or_else(|| "Face".into());
        let role = format!("{} · {label}", self.feature_name(feature));
        let ordinal = model.ordinal;
        let mut r = EntityRef::new(EntityKind::Face, &format!("f{feature}"), &role, ordinal, f.centroid, f.area);
        r.model = Some(ModelEntity::Face(model));
        Some(r)
    }

    /// Resolves through the model's persistent naming: the feature and generator of the
    /// faces, checked by fingerprint. A reference whose feature is gone or suppressed is
    /// lost with that reason.
    fn resolve(&self, r: &EntityRef) -> Option<Resolution> {
        let what = match r.kind {
            EntityKind::Edge => "edge",
            EntityKind::Face => "face",
            EntityKind::Vertex => "vertex",
        };
        if let Some((_, name, false)) = self.feature_of(r) {
            return Some(Resolution::Lost(format!("the {what} came from {name}, which no longer exists")));
        }
        let found = match r.model.as_ref()? {
            ModelEntity::Edge(e) => self.doc.model().resolve_edge(e, self.body).map(Entity::Edge),
            ModelEntity::Face(f) => self.doc.model().resolve_face(f, self.body).map(Entity::Face),
        };
        Some(match found {
            Ok(e) => Resolution::Found(e),
            Err(repair) => Resolution::Lost(repair.message),
        })
    }

    fn is_hole(&self, circle: &CircleInfo) -> bool {
        self.feature_of(&circle.entity)
            .and_then(|(id, _, _)| self.doc.feature(id))
            .is_some_and(|f| matches!(f.kind, FeatureKind::Cut(_) | FeatureKind::Hole(_)))
    }

    /// "4× Ø9 THRU", "Ø12 DEEP 20" or "Ø24": holes of the same cut and size are counted
    /// by centre, since the far end of a hole lies under its near end face-on. A
    /// threaded hole is called out by its thread, then by its drill where the drill goes
    /// further: "4× M6x1-6H THRU", "M8x1-6H DEEP 12" over "Ø7 DEEP 16". A hole with a
    /// counterbore or a countersink gets the head on a line of its own, whichever of the
    /// hole's circles is picked: "⌴ Ø10 ↧ 3", "⌵ Ø12 × 90°". Depths count from where the
    /// hole starts now, and a thread is called out as far as the wall still has it.
    fn hole_callout(&self, circle: &CircleInfo, projected: &Projected, symbols: &Symbols) -> Option<String> {
        let diameter = trim(format_value(circle.model_radius * 2.0, 2));
        let found = self.callout_feature(circle);
        let feature = found.and_then(|(id, _)| self.doc.feature(id));
        if let (Some((id, placement)), Some(FeatureKind::Hole(hole))) = (found, feature.map(|f| &f.kind)) {
            let feature = soliddraft_model::FeatureId(id as u32);
            let model = self.doc.model();
            let extents = model.hole_extents(feature);
            let walls = model.threaded_walls();
            let callout = |placement: usize| {
                let extent = extents.get(placement).copied().flatten();
                let wall = walls.iter().find(|wall| wall.feature == feature && wall.placement == placement).cloned();
                let top = extent.map_or_else(|| wall.as_ref().map_or(0.0, |wall| wall.top), |extent| extent.top);
                format_built_hole_callout(
                    hole,
                    "",
                    symbols,
                    Some(&BuiltHole { top, depth: extent.map(|extent| extent.depth), wall }),
                )
            };
            let text = callout(placement.unwrap_or(0));
            let placements: std::collections::BTreeSet<_> = projected
                .circles
                .iter()
                .filter_map(|circle| match self.callout_feature(circle) {
                    Some((owner, Some(placement))) if owner == id => Some(placement),
                    _ => None,
                })
                .collect();
            let count = placements.into_iter().filter(|&placement| callout(placement) == text).count();
            return Some(if count > 1 { format!("{count}{} {text}", symbols.times) } else { text });
        }
        let same = |c: &CircleInfo| match found {
            Some((id, _)) => self.callout_feature(c).is_some_and(|(other, _)| other == id),
            None => c.entity.feature == circle.entity.feature,
        };
        let mut centres: Vec<Point2> = Vec::new();
        for c in projected.circles.iter().filter(|c| same(c) && (c.model_radius - circle.model_radius).abs() < 1e-6) {
            if !centres.iter().any(|k| (k.u - c.centre.u).abs() < 1e-3 && (k.v - c.centre.v).abs() < 1e-3) {
                centres.push(c.centre);
            }
        }
        let count = if centres.len() > 1 { format!("{}{} ", centres.len(), symbols.times) } else { String::new() };
        Some(match feature.map(|f| &f.kind) {
            Some(FeatureKind::Cut(p)) if p.through_all => {
                format!("{count}{}{diameter} {}", symbols.diameter, symbols.through)
            }
            Some(FeatureKind::Cut(p)) => {
                format!("{count}{}{diameter} {} {}", symbols.diameter, symbols.depth, trim(format_value(p.depth, 2)))
            }
            _ => format!("{count}{}{diameter}", symbols.diameter),
        })
    }
}

fn trim(s: String) -> String {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s }
}

/// What a hole is like in the part as built: a callout states the hole as it is now.
pub struct BuiltHole {
    /// Where the hole starts now: zero, or deeper where a later cut took material off the
    /// face it was drilled from.
    pub top: f64,
    /// The surviving bore's termination, measured from the original opening.
    pub depth: Option<soliddraft_kernel_api::HoleDepth>,
    /// The hole's threaded wall as the body has it, or `None` when a later cut took the
    /// thread away (or the hole has none).
    pub wall: Option<soliddraft_model::thread::ThreadedWall>,
}

/// Formats a drawing hole callout from a [`soliddraft_model::HoleFeature`], including
/// Hole Wizard counterbore, countersink, slot, thread, end-condition, and callout
/// tolerance/precision data, as the feature states it.
pub fn format_hole_callout(h: &soliddraft_model::HoleFeature, count: &str, symbols: &Symbols) -> String {
    format_built_hole_callout(h, count, symbols, None)
}

/// The hole callout of [`format_hole_callout`], stated as `built` has the hole when given:
/// a cut off the top makes a blind hole and a counterbore shallower and a countersink
/// narrower, one that went deeper takes the head away, and the thread is the one the
/// wall still has.
pub fn format_built_hole_callout(
    h: &soliddraft_model::HoleFeature,
    count: &str,
    symbols: &Symbols,
    built: Option<&BuiltHole>,
) -> String {
    use soliddraft_kernel_api::{HoleDepth, HoleHead};
    use soliddraft_model::hole_wizard::{CalloutParam, HoleEndCondition};

    let fmt = |param: CalloutParam, value: f64| -> String {
        if let Some(tol) = h.wizard.as_ref().and_then(|w| w.tolerance_for(param)) {
            tol.format_value(value)
        } else {
            trim(format_value(value, 2))
        }
    };
    let top = built.map_or(0.0, |b| b.top);

    let fixed_depth = |depth| match depth {
        HoleDepth::Through => symbols.through.clone(),
        HoleDepth::Blind { depth } => format!("{} {}", symbols.depth, fmt(CalloutParam::Depth, depth - top)),
    };
    let built_depth = built.and_then(|b| b.depth);
    let depth_text = if let Some(depth) = built_depth {
        fixed_depth(depth)
    } else {
        match h.wizard.as_ref().map(|w| &w.end_condition) {
            Some(HoleEndCondition::ThroughAll) => symbols.through.clone(),
            Some(HoleEndCondition::Blind { depth }) => {
                format!("{} {}", symbols.depth, fmt(CalloutParam::Depth, *depth - top))
            }
            Some(HoleEndCondition::UpToNext) => "UP TO NEXT".to_owned(),
            Some(HoleEndCondition::UpToVertex { .. }) => "UP TO VERTEX".to_owned(),
            Some(HoleEndCondition::UpToSurface { .. }) => "UP TO SURFACE".to_owned(),
            Some(HoleEndCondition::OffsetFromSurface { offset, .. }) => {
                format!("OFFSET {}", fmt(CalloutParam::Depth, *offset))
            }
            None => fixed_depth(h.depth),
        }
    };

    let effective_depth = built_depth.or_else(|| h.specified_depth());
    let is_slot = h.wizard.as_ref().is_some_and(|w| w.hole_type.is_slot());
    let drilled = format!("{}{} {depth_text}", symbols.diameter, fmt(CalloutParam::Diameter, h.diameter));

    let mut lines = Vec::new();
    match (&h.thread, built) {
        // As built: the thread the wall still has, and the drill under it when the hole
        // goes deeper than the thread.
        (Some(thread), Some(built)) => match &built.wall {
            Some(wall) => {
                lines.push(match wall.depth() {
                    None => format!("{count}{} {}", thread.designation(), symbols.through),
                    Some(depth) => format!(
                        "{count}{} {} {}",
                        thread.designation(),
                        symbols.depth,
                        fmt(CalloutParam::ThreadDepth, depth)
                    ),
                });
                let deeper = match effective_depth {
                    Some(HoleDepth::Blind { depth }) => depth > wall.end() + 1e-9,
                    _ => !wall.through,
                };
                if deeper {
                    lines.push(drilled);
                }
            }
            // A later cut took the thread away: the hole is a plain one now.
            None => lines.push(format!("{count}{drilled}")),
        },
        (Some(thread), None) => match (thread.length, effective_depth) {
            (None, _) => lines.push(format!("{count}{} {depth_text}", thread.designation())),
            (Some(length), depth) => {
                lines.push(format!(
                    "{count}{} {} {}",
                    thread.designation(),
                    symbols.depth,
                    fmt(CalloutParam::ThreadDepth, length)
                ));
                let deeper = match depth {
                    Some(HoleDepth::Blind { depth }) => depth > length + 1e-9,
                    _ => true,
                };
                if deeper {
                    lines.push(drilled);
                }
            }
        },
        (None, _) if is_slot => {
            let slot_len = h
                .wizard
                .as_ref()
                .and_then(|w| w.slot_length.or_else(|| Some(w.size_row().default_slot_length_mm())))
                .unwrap_or(h.diameter * 2.0);
            lines.push(format!(
                "{count}{}{} {} {} SLOT {depth_text}",
                symbols.diameter,
                fmt(CalloutParam::Diameter, h.diameter),
                symbols.times,
                fmt(CalloutParam::SlotLength, slot_len)
            ));
        }
        (None, _) => lines.push(format!("{count}{drilled}")),
    }

    // The head as it is now: a cut off the top makes a counterbore shallower and a
    // countersink narrower, and one that went deeper took the head away.
    match h.effective_head() {
        Some(HoleHead::Counterbore { diameter, depth }) if depth > top + 1e-9 => {
            lines.push(format!(
                "{} {}{} {} {}",
                symbols.counterbore,
                symbols.diameter,
                fmt(CalloutParam::CounterboreDiameter, diameter),
                symbols.depth,
                fmt(CalloutParam::CounterboreDepth, depth - top)
            ));
        }
        Some(HoleHead::Countersink { diameter, angle_deg }) => {
            let now = diameter - 2.0 * top * (angle_deg.to_radians() / 2.0).tan();
            if now > h.diameter + 1e-9 {
                lines.push(format!(
                    "{} {}{} {} {}{}",
                    symbols.countersink,
                    symbols.diameter,
                    fmt(CalloutParam::CountersinkDiameter, now),
                    symbols.times,
                    fmt(CalloutParam::CountersinkAngle, angle_deg),
                    symbols.degree
                ));
            }
        }
        _ => {}
    }

    lines.join("\n")
}

/// The hole callout of a circle in the drawing's own style.
pub fn hole_callout(ctx: &PartContext<'_>, circle: &CircleInfo, projected: &Projected, template: &str) -> String {
    let symbols = &library::template(template).style_file().symbols;
    ctx.hole_callout(circle, projected, symbols).unwrap_or_default()
}

/// The part's revision: its semantic hash, equal while nothing about the part changed.
pub fn digest(doc: &Document) -> ContentHash {
    doc.model().semantic_hash()
}

/// The dimensions the part's features define, for Model items: each extrusion's depth
/// along its sketch normal, and the diameter of every circle its extrusions and cuts
/// were sketched with.
pub fn feature_dimensions(doc: &Document) -> Vec<ModelDimension> {
    let mut out = Vec::new();
    for f in doc.features.iter().filter(|f| !f.suppressed) {
        let (plane, circles) = match &f.kind {
            FeatureKind::Extrude(p) => {
                let plane = match p.sketch {
                    Some(s) => doc.feature(s).and_then(|sf| sf.plane),
                    None => Some(p.plane.to_kernel()),
                };
                let Some(plane) = plane else { continue };
                let normal = if p.flip { -plane.normal } else { plane.normal };
                out.push(ModelDimension {
                    feature: f.name.clone(),
                    name: "depth".into(),
                    measure: ModelMeasure::Distance {
                        axis: normal,
                        from: plane.origin,
                        to: plane.origin + normal * p.depth,
                        sketch_normal: None,
                    },
                    count: 1,
                });
                let circles = match (p.sketch, &p.profile) {
                    (Some(s), _) => sketch_circles(doc, s),
                    (None, crate::document::ProfileKind::Circle { diameter }) => vec![(plane.origin, diameter / 2.0)],
                    _ => Vec::new(),
                };
                (plane, circles)
            }
            FeatureKind::Cut(p) => {
                let Some(plane) = doc.feature(p.sketch).and_then(|sf| sf.plane) else { continue };
                (plane, sketch_circles(doc, p.sketch))
            }
            _ => continue,
        };
        for (center, radius) in circles {
            out.push(ModelDimension {
                feature: f.name.clone(),
                name: "diameter".into(),
                measure: ModelMeasure::Diameter { center, normal: plane.normal, radius },
                count: 1,
            });
        }
    }
    out
}

/// The circles of a sketch feature, as model centre and radius.
fn sketch_circles(doc: &Document, sketch: u64) -> Vec<(Vec3, f64)> {
    let Some(f) = doc.feature(sketch) else { return Vec::new() };
    let (Some(plane), Some(s)) = (f.plane, f.sketch()) else { return Vec::new() };
    s.sketch
        .drawn()
        .filter_map(|e| match e.kind {
            crate::sketch::EntityKind::Circle { center, radius } => {
                Some((plane.to_world(Point2::new(center.u, center.v)), radius))
            }
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod hole_callout_tests {
    use super::*;
    use soliddraft_kernel_api::{HoleDepth, HoleHead, HoleTip, SurfaceKind, Vec3};
    use soliddraft_model::hole_wizard::{
        CalloutParam, CalloutTolerance, HoleStandard, HoleType, HoleWizardSpec, ToleranceMode,
    };
    use soliddraft_model::{FaceFingerprint, FaceRef, FeatureId, Generator, HoleFeature};

    fn dummy_face() -> FaceRef {
        FaceRef {
            feature: FeatureId(1),
            label: "Top".into(),
            generator: Generator::EndCap,
            ordinal: 0,
            fingerprint: FaceFingerprint {
                surface: SurfaceKind::Plane,
                normal: Some(Vec3::Z),
                centroid: Vec3::ZERO,
                area: 100.0,
            },
        }
    }

    #[test]
    fn counterbore_slot_and_tapped_hole_wizard_callouts_include_head_and_tolerances() {
        let symbols = &library::template("iso-a3").style_file().symbols;
        let mut spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::Counterbore);
        spec.set_tolerance(CalloutTolerance {
            param: CalloutParam::CounterboreDepth,
            mode: ToleranceMode::Bilateral { upper: 0.1, lower: -0.05 },
            precision: 2,
        });
        let cb = HoleFeature {
            face: dummy_face(),
            points: vec![],
            diameter: 6.6,
            depth: HoleDepth::Through,
            tip: HoleTip::Flat,
            head: Some(HoleHead::Counterbore { diameter: 11.0, depth: 6.4 }),
            thread: None,
            sketch: None,
            wizard: Some(spec),
        };
        let text = format_hole_callout(&cb, "4× ", symbols);
        assert!(text.contains("4× Ø6.6 THRU"), "{text}");
        assert!(text.contains("Ø11") && text.contains("6.40 +0.10/-0.05"), "{text}");

        let tap_spec = HoleWizardSpec::new(HoleStandard::AnsiInch, HoleType::StraightTap);
        let resolved = tap_spec.resolve_defaults();
        let tapped = HoleFeature {
            face: dummy_face(),
            points: vec![],
            diameter: resolved.diameter,
            depth: HoleDepth::Through,
            tip: resolved.tip,
            head: None,
            thread: resolved.thread,
            sketch: None,
            wizard: Some(tap_spec),
        };
        let tap_text = format_hole_callout(&tapped, "2× ", symbols);
        assert_eq!(tap_text, "2× 1/4-20 UNC-2B THRU");
    }
}
