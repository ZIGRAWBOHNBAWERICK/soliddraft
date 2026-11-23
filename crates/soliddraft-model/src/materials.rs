//! A part's material and appearances.
//!
//! Face colours take precedence over body, feature, part and material colours.
//! Persistent face names and body keys keep assignments attached through rebuilds.
//! The part stores copies of the materials and appearances it uses.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use soliddraft_document::{ConfigScope, ConfigurationId};
use soliddraft_kernel_api::FaceId;
use soliddraft_materials::{Appearance, AppearanceId, Mass, Material, MaterialId, Snapshots, Unknown, scope};

use crate::document::Document;
use crate::feature::FeatureId;
use crate::reference::{self, Candidate, FaceRef, RepairReason, View};

/// Where an appearance is applied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case")]
pub enum AppearanceTarget {
    /// The whole part.
    Part,
    /// A solid body, by its stable model key.
    Body {
        /// The model body key.
        body: u32,
    },
    /// Every face a feature made, through later splits.
    Feature {
        /// The feature.
        feature: FeatureId,
    },
    /// One face, by its persistent name.
    Face {
        /// The face.
        face: FaceRef,
    },
}

impl AppearanceTarget {
    /// True when both name the same place, however their fingerprints were taken: two
    /// references to a face are the same place when they name the same feature,
    /// generator and ordinal.
    pub fn same_place(&self, other: &AppearanceTarget) -> bool {
        match (self, other) {
            (AppearanceTarget::Part, AppearanceTarget::Part) => true,
            (AppearanceTarget::Body { body: a }, AppearanceTarget::Body { body: b }) => a == b,
            (AppearanceTarget::Feature { feature: a }, AppearanceTarget::Feature { feature: b }) => a == b,
            (AppearanceTarget::Face { face: a }, AppearanceTarget::Face { face: b }) => {
                a.feature == b.feature && a.generator == b.generator && a.ordinal == b.ordinal
            }
            _ => false,
        }
    }

    /// How specific the target is.
    fn level(&self) -> u8 {
        match self {
            AppearanceTarget::Part => 0,
            AppearanceTarget::Feature { .. } => 1,
            AppearanceTarget::Body { .. } => 2,
            AppearanceTarget::Face { .. } => 3,
        }
    }
}

/// An appearance applied somewhere in the part, in some configurations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppearanceAssignment {
    /// Where.
    #[serde(flatten)]
    pub target: AppearanceTarget,
    /// Which appearance; the part holds its copy.
    pub appearance: AppearanceId,
    /// In which configurations.
    #[serde(flatten)]
    pub scope: ConfigScope,
}

/// A part or body material, in some configurations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaterialAssignment {
    /// The body key, or the whole part when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<u32>,
    /// Which material; the part holds its copy.
    pub material: MaterialId,
    /// In which configurations.
    #[serde(flatten)]
    pub scope: ConfigScope,
}

/// Everything about a part's materials and appearances that its file holds, as the
/// `material`, `appearances` and `snapshots` keys of `part.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PartMaterials {
    /// The material assignments; at most one per scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub material: Vec<MaterialAssignment>,
    /// The appearance assignments, at most one per place and scope, in the order they
    /// were made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub appearances: Vec<AppearanceAssignment>,
    /// The copies of every material and appearance the assignments name.
    #[serde(default, skip_serializing_if = "Snapshots::is_empty")]
    pub snapshots: Snapshots,
}

impl PartMaterials {
    /// True when the part has no material and no appearance.
    pub fn is_empty(&self) -> bool {
        self.material.is_empty() && self.appearances.is_empty() && self.snapshots.is_empty()
    }

    /// Applies an appearance at a place in these configurations, replacing the one
    /// there, or removes it with `None`, and drops the copies nothing uses any more.
    pub(crate) fn assign(&mut self, target: &AppearanceTarget, appearance: Option<&Appearance>, scope: &ConfigScope) {
        self.appearances.retain(|a| !(a.target.same_place(target) && a.scope == *scope));
        if let Some(a) = appearance {
            self.appearances.push(AppearanceAssignment {
                target: target.clone(),
                appearance: a.id.clone(),
                scope: scope.clone(),
            });
            self.snapshots.put_appearance(a.clone());
        }
        self.prune();
    }

    /// Drops the copies no assignment uses.
    pub(crate) fn prune(&mut self) {
        let materials: Vec<MaterialId> = self.material.iter().map(|m| m.material.clone()).collect();
        let appearances: Vec<AppearanceId> = self.appearances.iter().map(|a| a.appearance.clone()).collect();
        self.snapshots.retain_used(&materials, &appearances);
    }

    /// Why the file's materials section is inconsistent, if it is: an assignment
    /// naming a material or appearance the file holds no copy of.
    pub(crate) fn check(&self) -> Result<(), String> {
        for m in &self.material {
            if self.snapshots.material(&m.material).is_none() {
                return Err(format!("the material {} has no copy in the file", m.material));
            }
        }
        for a in &self.appearances {
            if self.snapshots.appearance(&a.appearance).is_none() {
                return Err(format!("the appearance {} has no copy in the file", a.appearance));
            }
        }
        Ok(())
    }
}

/// An appearance assignment whose place is not in the part any more, or not as it was.
/// It stays in the part, and applies again if its face comes back (a feature is
/// unsuppressed, an edit is undone); the properties panel offers to re-point it or
/// remove it.
#[derive(Clone, Debug, PartialEq)]
pub struct LostAppearance {
    /// Its index in [`PartMaterials::appearances`].
    pub index: usize,
    /// Why.
    pub reason: RepairReason,
    /// The problem, for people.
    pub message: String,
    /// Faces it could be moved to, best match first.
    pub candidates: Vec<Candidate>,
}

/// The appearance of every face of the part's body, as a renderer reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct AppearanceTable {
    /// The appearances in use. Index 0 is the part's own: what a face with no
    /// appearance of its own or its feature's shows.
    pub appearances: Vec<Appearance>,
    /// The faces that show another appearance than index 0, and its index.
    pub faces: HashMap<FaceId, u32>,
    /// The appearance a body's faces inherit before feature or face overrides.
    pub bodies: HashMap<u32, u32>,
    /// Assignments that did not apply.
    pub lost: Vec<LostAppearance>,
}

impl AppearanceTable {
    /// The inherited appearance of a body.
    pub fn of_body(&self, body: u32) -> &Appearance {
        &self.appearances[self.bodies.get(&body).copied().unwrap_or(0) as usize]
    }

    /// The appearance a face shows.
    pub fn of(&self, face: FaceId) -> &Appearance {
        &self.appearances[self.faces.get(&face).copied().unwrap_or(0) as usize]
    }
}

fn material_in<'a>(
    m: &'a PartMaterials,
    body: Option<u32>,
    configuration: Option<&ConfigurationId>,
) -> Option<&'a Material> {
    scope::pick(m.material.iter().filter(|a| a.body == body), |a| &a.scope, configuration)
        .and_then(|a| m.snapshots.material(&a.material))
}

fn part_override<'a>(m: &'a PartMaterials, configuration: Option<&ConfigurationId>) -> Option<&'a AppearanceId> {
    scope::pick(m.appearances.iter().filter(|a| a.target == AppearanceTarget::Part), |a| &a.scope, configuration)
        .map(|a| &a.appearance)
}

fn part_appearance_in(m: &PartMaterials, configuration: Option<&ConfigurationId>) -> Appearance {
    part_override(m, configuration)
        .or_else(|| material_in(m, None, configuration).and_then(|mat| mat.default_appearance.as_ref()))
        .and_then(|id| m.snapshots.appearance(id))
        .cloned()
        .unwrap_or_else(Appearance::default_part)
}

fn appearance_slot(table: &mut AppearanceTable, appearance: &Appearance) -> u32 {
    if let Some(slot) = table.appearances.iter().position(|a| a == appearance) {
        return slot as u32;
    }
    let slot = table.appearances.len() as u32;
    table.appearances.push(appearance.clone());
    slot
}

/// Why a part has no mass to show.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum MassError {
    /// The part has no material, or its material no density.
    #[error("{0}")]
    Unknown(Unknown),
    /// The part has no body yet.
    #[error("the part has no body")]
    NoBody,
    /// The kernel could not measure the body.
    #[error("the kernel could not measure the body: {0}")]
    Kernel(String),
}

impl Document {
    /// The part's material and appearance assignments and the copies they use.
    pub fn materials(&self) -> &PartMaterials {
        &self.materials
    }

    /// The part's material in a configuration (`None` for the default one), from the
    /// copy the part holds.
    pub fn material(&self, configuration: Option<&ConfigurationId>) -> Option<&Material> {
        material_in(&self.materials, None, configuration)
    }

    /// The effective material of one solid body, inheriting the part material.
    pub fn body_material(&self, body: u32, configuration: Option<&ConfigurationId>) -> Option<&Material> {
        material_in(&self.materials, Some(body), configuration).or_else(|| self.material(configuration))
    }

    /// What a face with no appearance of its own or its feature's shows: the part's
    /// appearance, else the material's default appearance, else SolidDraft's default.
    pub fn part_appearance(&self, configuration: Option<&ConfigurationId>) -> Appearance {
        part_appearance_in(&self.materials, configuration)
    }

    /// The appearance of every face of the part's body in a configuration, with the
    /// assignments that did not apply.
    pub fn appearance_table(&self, configuration: Option<&ConfigurationId>) -> AppearanceTable {
        self.appearance_table_in(&self.materials, configuration)
    }

    /// The appearance table as it would be with `appearance` applied at `target` in
    /// every configuration (or removed there with `None`), without changing the part.
    /// The properties panel shows a colour or a transparency this way while it is
    /// still being dragged, and applies it once as one undo step.
    pub fn appearance_table_previewing(
        &self,
        configuration: Option<&ConfigurationId>,
        target: &AppearanceTarget,
        appearance: Option<&Appearance>,
    ) -> AppearanceTable {
        let mut m = self.materials.clone();
        m.assign(target, appearance, &ConfigScope::All);
        self.appearance_table_in(&m, configuration)
    }

    fn appearance_table_in(&self, m: &PartMaterials, configuration: Option<&ConfigurationId>) -> AppearanceTable {
        let mut table = AppearanceTable {
            appearances: vec![part_appearance_in(m, configuration)],
            faces: HashMap::new(),
            bodies: HashMap::new(),
            lost: Vec::new(),
        };
        // The assignment that holds at each place, most specific place last so it wins.
        let mut chosen: Vec<(usize, &AppearanceAssignment)> = Vec::new();
        for (i, a) in m.appearances.iter().enumerate() {
            if a.target == AppearanceTarget::Part || !scope::applies(&a.scope, configuration) {
                continue;
            }
            match chosen.iter_mut().find(|(_, c)| c.target.same_place(&a.target)) {
                Some(slot) if scope::rank(&a.scope) >= scope::rank(&slot.1.scope) => *slot = (i, a),
                Some(_) => {}
                None => chosen.push((i, a)),
            }
        }
        chosen.sort_by_key(|(i, a)| (a.target.level(), *i));
        let has_body_materials =
            part_override(m, configuration).is_none() && m.material.iter().any(|a| a.body.is_some());
        if chosen.is_empty() && !has_body_materials {
            return table;
        }
        let topologies: Vec<_> = self
            .live_bodies()
            .into_iter()
            .filter_map(|(key, id)| self.view_of(id).ok().map(|topology| (key, topology)))
            .collect();
        if has_body_materials {
            for (key, topology) in &topologies {
                let Some(material) = material_in(m, Some(*key), configuration) else { continue };
                let appearance = material.default_appearance.as_ref().and_then(|id| m.snapshots.appearance(id));
                let slot = appearance_slot(&mut table, &appearance.cloned().unwrap_or_else(Appearance::default_part));
                table.bodies.insert(*key, slot);
                table.faces.extend(topology.faces.iter().map(|face| (face.id, slot)));
            }
        }
        let names = self.name_map();
        for (i, a) in chosen {
            let Some(appearance) = m.snapshots.appearance(&a.appearance) else { continue };
            let faces = match &a.target {
                AppearanceTarget::Part => continue,
                AppearanceTarget::Body { body } => {
                    let Some((_, topology)) = topologies.iter().find(|(key, _)| key == body) else {
                        table.lost.push(LostAppearance {
                            index: i,
                            reason: RepairReason::Missing,
                            message: format!("Body {} is absent from this configuration", body + 1),
                            candidates: Vec::new(),
                        });
                        continue;
                    };
                    let slot = appearance_slot(&mut table, appearance);
                    table.bodies.insert(*body, slot);
                    table.faces.extend(topology.faces.iter().map(|face| (face.id, slot)));
                    continue;
                }
                AppearanceTarget::Feature { feature } => {
                    if self.feature(*feature).is_none() {
                        table.lost.push(LostAppearance {
                            index: i,
                            reason: RepairReason::Missing,
                            message: format!("{feature} is not in the part any more"),
                            candidates: Vec::new(),
                        });
                        continue;
                    }
                    topologies
                        .iter()
                        .flat_map(|(_, topology)| reference::faces_of_feature(*feature, &View { topology }, &names))
                        .collect()
                }
                AppearanceTarget::Face { face } => {
                    let mut faces = Vec::new();
                    let mut failure = None;
                    for (_, topology) in &topologies {
                        match reference::resolve_faces(face, &View { topology }, &names) {
                            Ok(found) => faces.extend(found),
                            Err(error)
                                if failure.as_ref().is_none_or(|old: &reference::Unresolved| {
                                    old.candidates.len() < error.candidates.len()
                                }) =>
                            {
                                failure = Some(error)
                            }
                            Err(_) => {}
                        }
                    }
                    if faces.is_empty() {
                        if let Some(u) = failure {
                            let message = if face.label.is_empty() {
                                u.message
                            } else {
                                format!("{}: {}", face.label, u.message)
                            };
                            table.lost.push(LostAppearance {
                                index: i,
                                reason: u.reason,
                                message,
                                candidates: u.candidates,
                            });
                        }
                        continue;
                    }
                    faces
                }
            };
            let slot = appearance_slot(&mut table, appearance);
            table.faces.extend(faces.into_iter().map(|face| (face, slot)));
        }
        table
    }

    /// The part's mass properties in a configuration, using each live body's density.
    pub fn mass(&self, configuration: Option<&ConfigurationId>) -> Result<Mass, MassError> {
        let bodies = self.live_bodies();
        if bodies.is_empty() {
            return Err(MassError::NoBody);
        }
        let mut masses = Vec::with_capacity(bodies.len());
        for (key, body) in bodies {
            let props = self.kernel.mass_properties(body).map_err(|e| MassError::Kernel(e.to_string()))?;
            let mass = soliddraft_materials::mass::of_body(&props, self.body_material(key, configuration))
                .map_err(MassError::Unknown)?;
            masses.push(mass);
        }
        Ok(soliddraft_materials::mass::combine_in_place(masses))
    }
}
