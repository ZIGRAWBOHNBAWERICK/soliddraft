//! Configurations and exploded views of an assembly.
//!
//! A **configuration** is a named state of the assembly: which components are hidden or
//! suppressed, where each sits, the overrides below them, the values that drive its mates
//! and its exploded view. The truth always holds the active configuration's state, so
//! everything that reads the truth (loading, mates, interference, saving) sees the active
//! one and needs to know nothing else. Every other configuration keeps its own state in
//! its row ([`Configuration::state`]). Activating one swaps the two: the truth's state goes
//! into the row that was active, and the chosen row's state into the truth.
//!
//! A component inserted while one configuration is active is absent from the other rows'
//! states, so it keeps whatever state it has when one of them is activated. Values a
//! configuration does not know keep theirs the same way. Deleting a component takes it out
//! of every configuration.
//!
//! An **exploded view** is a list of steps, each moving some occurrences along a direction
//! by a distance, applied in order. It is for display: exploding the assembly moves what
//! the viewport, picking and drawings show and never changes a placement. The steps belong
//! to the configuration, so each configuration has its own.

use std::collections::HashSet;

use glam::DVec3;
use serde::{Deserialize, Serialize};
use soliddraft_model::ValueTable;

use crate::path::{ComponentId, OccurrencePath};
use crate::placement::Placement;
use crate::schema::{Assembly, Invalid, Override};

/// The most root components an assembly with configurations may have: each configuration
/// keeps every component's state.
pub const MAX_CONFIGURED_COMPONENTS: usize = 1024;

/// The most configurations one assembly may have.
pub const MAX_CONFIGURATIONS: usize = 64;

/// The name the first configuration gets: the state the assembly had before it had any.
pub const DEFAULT_NAME: &str = "Default";

/// One named configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    /// Stable id, from the assembly's counter.
    pub id: u32,
    /// The name; unique within the assembly.
    pub name: String,
    /// The configuration's state while another is active. The active configuration has
    /// none: its state is the truth itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Box<ConfigState>>,
}

impl Configuration {
    /// True for the active configuration.
    pub fn is_active(&self) -> bool {
        self.state.is_none()
    }
}

/// What a configuration holds of its own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigState {
    /// Every root component's placement and flags, by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ComponentState>,
    /// The overrides below the root's components.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub overrides: Vec<Override>,
    /// The values; a value is matched by id.
    #[serde(default, skip_serializing_if = "ValueTable::is_empty")]
    pub values: ValueTable,
    /// The exploded view's steps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub explode: Vec<ExplodeStep>,
}

/// A root component's state in a configuration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentState {
    /// The component.
    pub id: ComponentId,
    /// Its placement.
    #[serde(default, skip_serializing_if = "Placement::is_identity")]
    pub placement: Placement,
    /// Hidden in this configuration.
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    /// Suppressed in this configuration.
    #[serde(default, skip_serializing_if = "is_false")]
    pub suppressed: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// One step of an exploded view: every occurrence in `occurrences`, with everything under
/// it, moves `distance` millimetres along `direction`, in the root's frame, after the
/// steps before it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplodeStep {
    /// Stable id, from the assembly's counter.
    pub id: u32,
    /// What moves.
    pub occurrences: Vec<OccurrencePath>,
    /// Which way, in the root's frame; need not be a unit vector.
    pub direction: [f64; 3],
    /// How far, in millimetres.
    pub distance: f64,
}

impl ExplodeStep {
    /// The translation the step applies.
    pub fn offset(&self) -> DVec3 {
        DVec3::from_array(self.direction).normalize_or_zero() * self.distance
    }

    fn check(&self, assembly: &Assembly) -> Result<(), String> {
        let d = DVec3::from_array(self.direction);
        if !d.is_finite() || d.length() < 1e-12 {
            return Err(format!("explode step {} needs a direction", self.id));
        }
        if !self.distance.is_finite() || self.distance.abs() > 1e7 {
            return Err(format!("explode step {} needs a finite distance", self.id));
        }
        if self.occurrences.is_empty() {
            return Err(format!("explode step {} moves nothing", self.id));
        }
        for p in &self.occurrences {
            if !names_occurrence(assembly, p) {
                return Err(format!("explode step {} moves {p}, which is not in the assembly", self.id));
            }
        }
        Ok(())
    }
}

/// True when the first step of `path` names a component or a pattern member of
/// `assembly`. Deeper steps are checked when the structure is read.
pub(crate) fn names_occurrence(assembly: &Assembly, path: &OccurrencePath) -> bool {
    let Some(first) = path.steps().first() else { return false };
    if assembly.component(first.component).is_none() {
        return false;
    }
    match first.member {
        None => true,
        Some((pid, index)) => {
            assembly.pattern(pid).is_some_and(|p| p.seeds.contains(&first.component) && index < p.rule.len())
        }
    }
}

impl ConfigState {
    /// The state the truth holds now.
    pub fn capture(assembly: &Assembly) -> ConfigState {
        ConfigState {
            components: assembly
                .components
                .iter()
                .map(|c| ComponentState {
                    id: c.id,
                    placement: c.placement,
                    hidden: c.hidden,
                    suppressed: c.suppressed,
                })
                .collect(),
            overrides: assembly.overrides.clone(),
            values: assembly.values.clone(),
            explode: assembly.explode.clone(),
        }
    }

    /// Puts this state into the truth. Components it does not know keep theirs; values are
    /// matched by id and others keep theirs; overrides and explode steps that name an
    /// occurrence the assembly no longer has are dropped.
    fn apply(&self, assembly: &mut Assembly) -> Result<(), Invalid> {
        for s in &self.components {
            if let Some(c) = assembly.component_mut(s.id) {
                c.placement = s.placement;
                c.hidden = s.hidden;
                c.suppressed = s.suppressed;
            }
        }
        let overrides: Vec<Override> =
            self.overrides.iter().filter(|o| names_occurrence(assembly, &o.path)).cloned().collect();
        assembly.overrides = overrides;
        let ids: Vec<u32> = assembly.values.ids().collect();
        for id in ids {
            let Some(text) = self.values.expression(id).map(|e| e.text().to_owned()) else { continue };
            let name = assembly.values.name_of(id).expect("listed above").to_owned();
            let mut next = assembly.values.clone();
            next.set(&name, &text, || id).map_err(|e| Invalid(format!("value {name}: {e}")))?;
            assembly.values = next;
        }
        let explode: Vec<ExplodeStep> = self
            .explode
            .iter()
            .filter_map(|s| {
                let occurrences: Vec<OccurrencePath> =
                    s.occurrences.iter().filter(|p| names_occurrence(assembly, p)).cloned().collect();
                (!occurrences.is_empty()).then(|| ExplodeStep { occurrences, ..s.clone() })
            })
            .collect();
        assembly.explode = explode;
        Ok(())
    }

    /// Forgets the components in `gone`, and the overrides and explode steps under them.
    pub(crate) fn forget(&mut self, gone: &HashSet<ComponentId>) {
        let under = |p: &OccurrencePath| p.steps().first().is_some_and(|s| gone.contains(&s.component));
        self.components.retain(|c| !gone.contains(&c.id));
        self.overrides.retain(|o| !under(&o.path));
        for s in &mut self.explode {
            s.occurrences.retain(|p| !under(p));
        }
        self.explode.retain(|s| !s.occurrences.is_empty());
    }
}

impl Assembly {
    /// The active configuration, when the assembly has configurations.
    pub fn active_configuration(&self) -> Option<&Configuration> {
        self.configurations.iter().find(|c| c.is_active())
    }

    /// The configuration called `name`.
    pub fn configuration_named(&self, name: &str) -> Option<&Configuration> {
        self.configurations.iter().find(|c| c.name == name)
    }

    /// Adds an inactive configuration called `name` holding the state the truth holds now,
    /// and returns its id. The first one added also adds [`DEFAULT_NAME`], active, for the
    /// state the assembly had before.
    pub fn add_configuration(&mut self, name: &str) -> Result<u32, Invalid> {
        let name = name.trim();
        check_name(self, name, None)?;
        if self.components.len() > MAX_CONFIGURED_COMPONENTS {
            return Err(Invalid(format!(
                "an assembly with more than {MAX_CONFIGURED_COMPONENTS} components cannot have configurations"
            )));
        }
        if self.configurations.len() >= MAX_CONFIGURATIONS {
            return Err(Invalid(format!("an assembly may have at most {MAX_CONFIGURATIONS} configurations")));
        }
        if self.configurations.is_empty() {
            if name == DEFAULT_NAME {
                return Err(Invalid(format!("{DEFAULT_NAME} is the configuration the assembly has now")));
            }
            let id = self.take_id();
            self.configurations.push(Configuration { id, name: DEFAULT_NAME.into(), state: None });
        }
        let id = self.take_id();
        let state = Some(Box::new(ConfigState::capture(self)));
        self.configurations.push(Configuration { id, name: name.into(), state });
        Ok(id)
    }

    /// Makes the configuration `id` the active one: the truth's state goes into the row
    /// that was active and the chosen row's state into the truth.
    pub fn activate_configuration(&mut self, id: u32) -> Result<(), Invalid> {
        let Some(target) = self.configurations.iter().position(|c| c.id == id) else {
            return Err(Invalid(format!("there is no configuration {id}")));
        };
        if self.configurations[target].is_active() {
            return Ok(());
        }
        let Some(active) = self.configurations.iter().position(Configuration::is_active) else {
            return Err(Invalid("no configuration is active".into()));
        };
        let mut next = self.clone();
        let state = next.configurations[target].state.take().expect("an inactive row has a state");
        next.configurations[active].state = Some(Box::new(ConfigState::capture(self)));
        state.apply(&mut next)?;
        next.normalize();
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Renames configuration `id`.
    pub fn rename_configuration(&mut self, id: u32, name: &str) -> Result<(), Invalid> {
        let name = name.trim();
        check_name(self, name, Some(id))?;
        let row = self
            .configurations
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| Invalid(format!("there is no configuration {id}")))?;
        row.name = name.into();
        Ok(())
    }

    /// Deletes an inactive configuration. When only the active one is left, the assembly
    /// goes back to having none.
    pub fn delete_configuration(&mut self, id: u32) -> Result<(), Invalid> {
        let Some(row) = self.configurations.iter().find(|c| c.id == id) else {
            return Err(Invalid(format!("there is no configuration {id}")));
        };
        if row.is_active() {
            return Err(Invalid(format!("{} is active; activate another configuration first", row.name)));
        }
        self.configurations.retain(|c| c.id != id);
        if self.configurations.len() == 1 {
            self.configurations.clear();
        }
        Ok(())
    }

    /// Forgets the components in `gone` in every configuration and in the exploded view.
    pub(crate) fn forget_configured(&mut self, gone: &HashSet<ComponentId>) {
        for c in &mut self.configurations {
            if let Some(s) = &mut c.state {
                s.forget(gone);
            }
        }
        let mut now = ConfigState { explode: std::mem::take(&mut self.explode), ..Default::default() };
        now.forget(gone);
        self.explode = now.explode;
    }

    /// Checks the configurations and the exploded view.
    pub(crate) fn validate_configurations(&self) -> Result<(), Invalid> {
        let bad = |m: String| Err(Invalid(m));
        let mut names = HashSet::new();
        for c in &self.configurations {
            if c.name.trim().is_empty() || c.name.trim() != c.name || !names.insert(c.name.as_str()) {
                return bad(format!("configuration {} needs a name of its own", c.id));
            }
            if let Some(s) = &c.state {
                if !s.components.is_sorted_by(|a, b| a.id < b.id) {
                    return bad(format!("configuration {} lists its components out of order", c.name));
                }
                if let Some(x) = s.components.iter().find(|x| self.component(x.id).is_none()) {
                    return bad(format!("configuration {} holds component {}, which does not exist", c.name, x.id));
                }
                if !s.overrides.is_sorted_by(|a, b| a.path < b.path) {
                    return bad(format!("configuration {} lists its overrides out of order", c.name));
                }
                if let Some(o) = s.overrides.iter().find(|o| !names_occurrence(self, &o.path)) {
                    return bad(format!("configuration {} overrides {}, which is not in the assembly", c.name, o.path));
                }
                let mut steps = HashSet::new();
                for step in &s.explode {
                    step.check(self).map_err(|m| Invalid(format!("configuration {}: {m}", c.name)))?;
                    if !steps.insert(step.id) || step.id >= self.next_id {
                        return bad(format!(
                            "configuration {} has explode step id {} twice or too high",
                            c.name, step.id
                        ));
                    }
                }
            }
        }
        let active = self.configurations.iter().filter(|c| c.is_active()).count();
        if !self.configurations.is_empty() && active != 1 {
            return bad(format!("{active} configurations are active; exactly one must be"));
        }
        if self.configurations.len() == 1 {
            return bad("an assembly with configurations has at least two".into());
        }
        for step in &self.explode {
            step.check(self).map_err(Invalid)?;
        }
        Ok(())
    }

    /// The ids the configurations and the active exploded view use. A configuration made
    /// from another starts with copies of its explode steps, so a step's id names it within
    /// its configuration; [`Assembly::validate_configurations`] checks those apart.
    pub(crate) fn configured_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.configurations.iter().map(|c| c.id).collect();
        ids.extend(self.explode.iter().map(|s| s.id));
        ids
    }

    /// Puts the configurations' lists in canonical order.
    pub(crate) fn normalize_configurations(&mut self) {
        for c in &mut self.configurations {
            if let Some(s) = &mut c.state {
                s.components.sort_by_key(|x| x.id);
                s.overrides.sort_by(|a, b| a.path.cmp(&b.path));
            }
        }
    }
}

fn check_name(assembly: &Assembly, name: &str, except: Option<u32>) -> Result<(), Invalid> {
    if name.is_empty() || name.chars().count() > 64 {
        return Err(Invalid("a configuration needs a name of 1 to 64 characters".into()));
    }
    if assembly.configurations.iter().any(|c| c.name == name && Some(c.id) != except) {
        return Err(Invalid(format!("there is already a configuration called {name}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_document::{ContentHash, DocKind, DocRef, DocumentId};

    fn sample() -> Assembly {
        let mut a = Assembly::new("Frame");
        let r = DocRef::new(DocumentId::new(), DocKind::Part, "bolt.dwprt", ContentHash::of_bytes(b"bolt"));
        let s = a.add_source(r);
        a.add_component(s, Placement::translation(DVec3::new(10.0, 0.0, 0.0)));
        a.add_component(s, Placement::translation(DVec3::new(0.0, 20.0, 0.0)));
        a
    }

    #[test]
    fn a_configuration_keeps_its_own_suppression_placements_and_values() {
        let mut a = sample();
        a.values.set("gap", "5 mm", || 90).unwrap();
        a.next_id = 91;
        let long = a.add_configuration("Long").unwrap();
        assert_eq!(a.active_configuration().unwrap().name, DEFAULT_NAME);
        a.validate().unwrap();
        // Change the Default: suppress one and move the other.
        a.components[0].suppressed = true;
        a.components[1].placement = Placement::translation(DVec3::new(0.0, 99.0, 0.0));
        a.values.set("gap", "7 mm", || unreachable!()).unwrap();
        a.activate_configuration(long).unwrap();
        a.validate().unwrap();
        assert!(!a.components[0].suppressed, "Long holds the state from before");
        assert_eq!(a.components[1].placement.translation.y, 20.0);
        assert_eq!(a.values.expression(90).unwrap().text(), "5 mm");
        let default = a.configuration_named(DEFAULT_NAME).unwrap().id;
        a.activate_configuration(default).unwrap();
        assert!(a.components[0].suppressed);
        assert_eq!(a.components[1].placement.translation.y, 99.0);
        assert_eq!(a.values.expression(90).unwrap().text(), "7 mm");
        let text = serde_json::to_string(&a).unwrap();
        let back: Assembly = serde_json::from_str(&text).unwrap();
        assert_eq!(back, a);
        back.validate().unwrap();
    }

    #[test]
    fn a_component_added_later_keeps_its_state_and_a_deleted_one_goes() {
        let mut a = sample();
        let other = a.add_configuration("Other").unwrap();
        let s = a.sources[0].id;
        let c = a.add_component(s, Placement::translation(DVec3::new(5.0, 5.0, 5.0)));
        a.activate_configuration(other).unwrap();
        assert_eq!(a.component(c).unwrap().placement.translation.x, 5.0);
        let gone: HashSet<ComponentId> = [a.components[0].id].into();
        a.components.retain(|x| !gone.contains(&x.id));
        a.forget_configured(&gone);
        a.validate().unwrap();
    }

    #[test]
    fn names_are_unique_and_the_active_one_cannot_go() {
        let mut a = sample();
        assert!(a.add_configuration(DEFAULT_NAME).is_err());
        let b = a.add_configuration("B").unwrap();
        assert!(a.add_configuration("B").is_err());
        let d = a.active_configuration().unwrap().id;
        assert!(a.delete_configuration(d).is_err());
        a.rename_configuration(b, "Open").unwrap();
        a.delete_configuration(b).unwrap();
        assert!(a.configurations.is_empty(), "one left goes back to none");
        a.validate().unwrap();
    }

    #[test]
    fn explode_steps_are_checked() {
        let mut a = sample();
        let first = a.components[0].id;
        let id = a.take_id();
        a.explode.push(ExplodeStep {
            id,
            occurrences: vec![OccurrencePath::root().child(crate::path::Step::component(first))],
            direction: [0.0, 0.0, 1.0],
            distance: 30.0,
        });
        a.validate().unwrap();
        assert_eq!(a.explode[0].offset(), DVec3::new(0.0, 0.0, 30.0));
        a.explode[0].direction = [0.0; 3];
        assert!(a.validate().is_err());
        a.explode[0].direction = [1.0, 0.0, 0.0];
        a.explode[0].occurrences = vec!["77".parse().unwrap()];
        assert!(a.validate().is_err());
    }
}
