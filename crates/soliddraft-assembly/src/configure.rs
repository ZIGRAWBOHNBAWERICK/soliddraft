//! Configurations and exploded views on an open assembly: each change is one undo step,
//! and the exploded view is shown by moving rows of the occurrence table, never the truth.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use glam::DVec3;
use soliddraft_document::{ConfigurationId, ContentHash, DocKind, DocRef, DocumentId};
use soliddraft_materials::Appearance;

use crate::configuration::ExplodeStep;
use crate::document::{AssemblyDocument, Configured, EditError};
use crate::mesh::SharedMesh;
use crate::occurrence::{Flags, OccurrenceIndex};
use crate::path::{OccurrencePath, Step};
use crate::placement::{Aabb, Placement};
use crate::schema::{Assembly, Invalid};
use crate::store::{DefinitionIndex, DefinitionKey};

/// A live (non-removed) occurrence in an open assembly with its path, pinned definition
/// and configuration, flags, and accepted world placement.
///
/// The placement is always the accepted design placement from
/// [`AssemblyDocument::accepted_worlds`], never a viewport drag preview or an exploded
/// pose. Readers (drawings, robot export, in-context editing, BOM) apply their own
/// selection rules (such as hidden-part handling, mesh vs exact body, or relative frame)
/// on top of these records without indexing the occurrence table and definition store
/// separately.
#[derive(Clone, Debug)]
pub struct PlacedOccurrence {
    /// Its row in the occurrence table.
    pub row: OccurrenceIndex,
    /// Its persistent path from the root assembly.
    pub path: OccurrencePath,
    /// The display label shown in the tree (e.g. `"Bracket<2>"`).
    pub label: String,
    /// Its interned definition index in the store.
    pub definition: DefinitionIndex,
    /// The pinned definition identity: document id, configuration, pinned revision, and
    /// quality override.
    pub key: DefinitionKey,
    /// Whether the definition is a part, sub-assembly, or mesh reference.
    pub kind: DocKind,
    /// The definition's base label (its referenced file stem).
    pub definition_label: String,
    /// The pinned document reference the definition was reached by, if any.
    pub reference: Option<DocRef>,
    /// Effective and self flags (hidden, suppressed, assembly, etc.).
    pub flags: Flags,
    /// Accepted world placement in the root assembly's frame, ignoring viewport drag or
    /// exploded previews.
    pub placement: Placement,
    /// Assembly-level colour override on this occurrence, if any.
    pub colour: Option<Appearance>,
    /// Loaded display mesh of the definition, if any.
    pub mesh: Option<SharedMesh>,
}

impl PlacedOccurrence {
    /// The document id of the pinned definition.
    pub fn document(&self) -> DocumentId {
        self.key.document
    }

    /// The pinned revision of the definition.
    pub fn revision(&self) -> &ContentHash {
        &self.key.revision
    }

    /// The configuration placed by this occurrence (`None` when the placement uses the
    /// document's saved/default configuration).
    pub fn configuration(&self) -> Option<&ConfigurationId> {
        self.key.configuration.as_ref()
    }

    /// True when this occurrence is a sub-assembly (or the root assembly).
    pub fn is_assembly(&self) -> bool {
        self.flags.has(Flags::ASSEMBLY)
    }

    /// True when this occurrence is hidden (itself or through an ancestor).
    pub fn is_hidden(&self) -> bool {
        self.flags.has(Flags::HIDDEN)
    }

    /// True when this occurrence is suppressed (itself or through an ancestor).
    pub fn is_suppressed(&self) -> bool {
        self.flags.has(Flags::SUPPRESSED)
    }

    /// True when this occurrence is a live leaf component (part or mesh reference) that
    /// is not suppressed. Hidden leaves return `true` (for BOM and robot export).
    pub fn is_active_leaf(&self) -> bool {
        !(self.flags.has(Flags::ASSEMBLY) || self.flags.has(Flags::SUPPRESSED) || self.flags.has(Flags::REMOVED))
    }

    /// True when this occurrence is a live leaf component that is shown (neither hidden
    /// nor suppressed).
    pub fn is_shown_leaf(&self) -> bool {
        self.flags.drawn_part()
    }

    /// This occurrence's accepted placement expressed in `frame`'s coordinate system.
    pub fn placement_in(&self, frame: &Placement) -> Placement {
        frame.inverse().compose(&self.placement)
    }
}

impl AssemblyDocument {
    /// Changes the configurations or the exploded view on a copy of the truth with `edit`,
    /// checks the copy, and takes it as one undo step labelled `label`. A refusal changes
    /// nothing. `structural` rereads the structure, for a change of suppression or
    /// placements.
    fn configure<T>(
        &mut self,
        label: &str,
        structural: bool,
        edit: impl FnOnce(&mut Assembly) -> Result<T, Invalid>,
    ) -> Result<T, EditError> {
        let shown = self.exploded;
        self.cancel_drag();
        self.finish_loading();
        let mut next = self.assembly.clone();
        let out = edit(&mut next).map_err(|e| EditError::Invalid(e.0))?;
        next.normalize();
        next.validate().map_err(|e| EditError::Invalid(e.0))?;
        let mut before = self.image(&[], &[]);
        before.configured = Some(Configured::of(&self.assembly));
        self.assembly = next;
        if structural {
            self.reflatten();
        }
        self.push_record(label, before);
        if shown && !self.assembly.explode.is_empty() {
            self.show_explode();
        }
        Ok(out)
    }

    /// Adds an inactive configuration called `name` holding the assembly's state now, and
    /// returns its id. The first one added also adds `Default`, active. One undo step.
    pub fn add_configuration(&mut self, name: &str) -> Result<u32, EditError> {
        self.configure("Add configuration", false, |a| a.add_configuration(name))
    }

    /// Makes configuration `id` the active one: its suppression, placements, overrides,
    /// values and exploded view replace the assembly's, which the configuration that was
    /// active keeps. One undo step.
    pub fn activate_configuration(&mut self, id: u32) -> Result<(), EditError> {
        let name = self.configuration_name(id)?;
        self.configure(&format!("Activate {name}"), true, |a| a.activate_configuration(id))
    }

    /// Renames configuration `id`. One undo step.
    pub fn rename_configuration(&mut self, id: u32, name: &str) -> Result<(), EditError> {
        self.configuration_name(id)?;
        self.configure("Rename configuration", false, |a| a.rename_configuration(id, name))
    }

    /// Deletes an inactive configuration. One undo step.
    pub fn delete_configuration(&mut self, id: u32) -> Result<(), EditError> {
        self.configuration_name(id)?;
        self.configure("Delete configuration", false, |a| a.delete_configuration(id))
    }

    /// The id of the configuration called `name`.
    pub fn configuration_id(&self, name: &str) -> Option<u32> {
        self.assembly.configuration_named(name).map(|c| c.id)
    }

    fn configuration_name(&self, id: u32) -> Result<String, EditError> {
        self.assembly
            .configurations
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .ok_or_else(|| EditError::Invalid(format!("there is no configuration {id}")))
    }

    /// Adds a step to the active configuration's exploded view: `occurrences` move
    /// `distance` millimetres along `direction` in the root's frame, after the steps before.
    /// Returns its id. One undo step.
    pub fn add_explode_step(
        &mut self,
        occurrences: Vec<OccurrencePath>,
        direction: DVec3,
        distance: f64,
    ) -> Result<u32, EditError> {
        for p in &occurrences {
            self.table.find(p).ok_or_else(|| EditError::NoOccurrence(p.clone()))?;
        }
        self.configure("Explode step", false, |a| {
            let id = a.take_configured_id();
            a.explode.push(ExplodeStep { id, occurrences, direction: direction.to_array(), distance });
            Ok(id)
        })
    }

    /// Changes an explode step's direction or distance. One undo step.
    pub fn edit_explode_step(
        &mut self,
        id: u32,
        direction: Option<DVec3>,
        distance: Option<f64>,
    ) -> Result<(), EditError> {
        self.configure("Edit explode step", false, |a| {
            let step = a
                .explode
                .iter_mut()
                .find(|s| s.id == id)
                .ok_or_else(|| Invalid(format!("there is no explode step {id}")))?;
            if let Some(d) = direction {
                step.direction = d.to_array();
            }
            if let Some(d) = distance {
                step.distance = d;
            }
            Ok(())
        })
    }

    /// Deletes an explode step. One undo step.
    pub fn delete_explode_step(&mut self, id: u32) -> Result<(), EditError> {
        self.configure("Delete explode step", false, |a| {
            let before = a.explode.len();
            a.explode.retain(|s| s.id != id);
            if a.explode.len() == before {
                return Err(Invalid(format!("there is no explode step {id}")));
            }
            Ok(())
        })
    }

    /// Replaces the exploded view with one step per shown component of the root, each
    /// moving `distance` millimetres straight away from the middle of the assembly. The
    /// component nearest the middle stays. Returns the new steps' ids. One undo step.
    pub fn auto_explode(&mut self, distance: f64) -> Result<Vec<u32>, EditError> {
        self.finish_loading();
        self.collapse();
        let rows: Vec<(OccurrencePath, Aabb)> = self
            .table
            .children(OccurrenceIndex::ROOT)
            .filter(|r| {
                let f = self.table.flags(*r);
                !f.has(Flags::SUPPRESSED) && !f.has(Flags::REMOVED) && !f.has(Flags::HIDDEN)
            })
            .map(|r| (self.table.path(r), self.table.bounds(r)))
            .filter(|(_, b)| !b.is_empty())
            .collect();
        if rows.len() < 2 {
            return Err(EditError::Invalid("an exploded view needs at least two shown components".into()));
        }
        let mut all = Aabb::EMPTY;
        for (_, b) in &rows {
            all.add(b);
        }
        let middle = all.center();
        let anchor = rows
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.1.center().distance(middle).total_cmp(&b.1.center().distance(middle)))
            .map(|(i, _)| i)
            .expect("two rows at least");
        self.configure("Auto explode", false, |a| {
            a.explode.clear();
            let mut ids = Vec::new();
            for (i, (path, b)) in rows.iter().enumerate() {
                if i == anchor {
                    continue;
                }
                let away = b.center() - middle;
                let direction = if away.length() > 1e-9 { away.normalize() } else { DVec3::Z };
                let id = a.take_configured_id();
                a.explode.push(ExplodeStep {
                    id,
                    occurrences: vec![path.clone()],
                    direction: direction.to_array(),
                    distance,
                });
                ids.push(id);
            }
            Ok(ids)
        })
    }

    /// Shows the exploded view (`true`) or the assembled positions (`false`). Display
    /// only: no undo step, and the truth and its revision do not change. Any edit shows
    /// the assembled positions first.
    pub fn set_exploded(&mut self, exploded: bool) -> Result<(), EditError> {
        if exploded == self.exploded {
            return Ok(());
        }
        if exploded {
            if self.assembly.explode.is_empty() {
                return Err(EditError::Invalid("the assembly has no exploded view; add an explode step".into()));
            }
            self.cancel_drag();
            self.finish_loading();
            self.show_explode();
        } else {
            self.collapse();
        }
        Ok(())
    }

    /// True while the exploded view is shown.
    pub fn is_exploded(&self) -> bool {
        self.exploded
    }

    /// Each directly moved row's total shift in the root's frame.
    fn explode_offsets(&self) -> BTreeMap<OccurrenceIndex, DVec3> {
        let mut offsets = BTreeMap::new();
        for step in &self.assembly.explode {
            let shift = step.offset();
            let mut paths: Vec<&OccurrencePath> = step.occurrences.iter().collect();
            paths.sort();
            paths.dedup();
            // One under another in the same step moves with it, once.
            let mut kept: Vec<&OccurrencePath> = Vec::new();
            for p in paths {
                if !kept.last().is_some_and(|k| p.starts_with(k)) {
                    kept.push(p);
                }
            }
            for p in kept {
                let Some(row) = self.table.find(p) else { continue };
                if row == OccurrenceIndex::ROOT {
                    continue;
                }
                let f = self.table.flags(row);
                if f.has(Flags::SUPPRESSED) || f.has(Flags::REMOVED) {
                    continue;
                }
                *offsets.entry(row).or_insert(DVec3::ZERO) += shift;
            }
        }
        offsets
    }

    /// Moves the occurrences in the exploded view and keeps their original placements.
    pub(crate) fn show_explode(&mut self) {
        let moves: Vec<_> = self
            .explode_offsets()
            .into_iter()
            .map(|(row, offset)| {
                let local = self.table.local(row);
                self.explode_moved.push((row, local));
                let parent = self.table.parent(row).expect("a moved occurrence has a parent");
                let shift = self.table.world(parent).rotation.inverse() * offset;
                (row, Placement { translation: local.translation + shift, ..local })
            })
            .collect();
        self.place_rows(&moves);
        self.exploded = true;
    }

    /// Puts what the exploded view moved back where the truth places it.
    pub(crate) fn collapse(&mut self) {
        if !self.exploded {
            return;
        }
        self.exploded = false;
        let moved = std::mem::take(&mut self.explode_moved);
        self.place_rows(&moved);
    }

    /// World placements in occurrence-table order, including the root, without viewport previews.
    pub fn accepted_worlds(&self) -> Cow<'_, [Placement]> {
        let drag = self.drag.as_ref().map_or(&[][..], |d| d.shown.as_slice());
        if self.explode_moved.is_empty() && drag.is_empty() {
            return Cow::Borrowed(self.table.worlds());
        }
        let mut locals: HashMap<_, _> = self.explode_moved.iter().copied().collect();
        locals.extend(drag.iter().filter_map(|(path, pose)| Some((self.table.find(path)?, *pose))));
        let mut worlds: Vec<Placement> = Vec::with_capacity(self.table.len());
        for row in (0..self.table.len() as u32).map(OccurrenceIndex) {
            let local = locals.get(&row).copied().unwrap_or_else(|| self.table.local(row));
            let world = self.table.parent(row).map_or(local, |p| worlds[p.0 as usize].compose(&local));
            worlds.push(world);
        }
        Cow::Owned(worlds)
    }

    fn placed_occurrence_with(&self, row: OccurrenceIndex, worlds: &[Placement]) -> PlacedOccurrence {
        let def_index = self.table.definition(row);
        let def = self.store.get(def_index);
        PlacedOccurrence {
            row,
            path: self.table.path(row),
            label: self.label(row),
            definition: def_index,
            key: def.key.clone(),
            kind: def.kind,
            definition_label: def.label.clone(),
            reference: self.reference(def_index).cloned(),
            flags: self.table.flags(row),
            placement: worlds[row.0 as usize],
            colour: self.table.colour(row).cloned(),
            mesh: def.mesh.clone(),
        }
    }

    /// Every live (non-removed) occurrence under the root in occurrence-table order, with
    /// its path, pinned definition and configuration, flags, and accepted world placement.
    pub fn placed_occurrences(&self) -> Vec<PlacedOccurrence> {
        let worlds = self.accepted_worlds();
        let mut out = Vec::with_capacity(self.table.len().saturating_sub(1));
        for row in (1..self.table.len() as u32).map(OccurrenceIndex) {
            if self.table.flags(row).has(Flags::REMOVED) {
                continue;
            }
            out.push(self.placed_occurrence_with(row, &worlds));
        }
        out
    }

    /// The live occurrence at `path`, with its pinned definition, flags, and accepted
    /// world placement, or `None` if removed or absent.
    pub fn placed_occurrence(&self, path: &OccurrencePath) -> Option<PlacedOccurrence> {
        let row = self.table.find(path)?;
        let worlds = self.accepted_worlds();
        Some(self.placed_occurrence_with(row, &worlds))
    }

    /// The live occurrence at `row`, with its pinned definition, flags, and accepted
    /// world placement, or `None` if out of range or removed.
    pub fn placed_occurrence_at(&self, row: OccurrenceIndex) -> Option<PlacedOccurrence> {
        if (row.0 as usize) >= self.table.len() || self.table.flags(row).has(Flags::REMOVED) {
            return None;
        }
        let worlds = self.accepted_worlds();
        Some(self.placed_occurrence_with(row, &worlds))
    }

    /// Where each occurrence of the root's components sits in the exploded view, in the
    /// root's frame, without showing it: the assembled placement for one no step moves.
    pub fn exploded_worlds(&mut self) -> Vec<(OccurrencePath, Placement)> {
        if !self.exploded && !self.assembly.explode.is_empty() {
            self.finish_loading();
        }
        let worlds = if self.exploded { Cow::Borrowed(self.table.worlds()) } else { self.accepted_worlds() };
        let mut offsets = vec![DVec3::ZERO; self.table.len()];
        if !self.exploded {
            for (row, shift) in self.explode_offsets() {
                offsets[row.0 as usize] = shift;
            }
        }
        let mut out = Vec::new();
        for row in (1..self.table.len() as u32).map(OccurrenceIndex) {
            let i = row.0 as usize;
            let parent = self.table.parent(row).expect("an occurrence has a parent");
            let inherited = offsets[parent.0 as usize];
            offsets[i] += inherited;
            if !self.table.flags(row).has(Flags::REMOVED) {
                let world = worlds[i];
                out.push((self.table.path(row), Placement { translation: world.translation + offsets[i], ..world }));
            }
        }
        out
    }
}

/// The path of one of the root's components, for tests and tools.
pub fn component_path(id: crate::path::ComponentId) -> OccurrencePath {
    OccurrencePath::root().child(Step::component(id))
}
