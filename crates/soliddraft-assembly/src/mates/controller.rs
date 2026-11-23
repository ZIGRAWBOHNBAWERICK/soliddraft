//! The Mate Controller: named positions of chosen mates' values, applied together as one
//! undo step, and the frames between two positions for an animation.
//!
//! It drives Distance and Angle mates, a Width by dimension or percentage, a Profile Center's
//! distance, and a Slot's or a Path's distance or percentage along it. A position stores one
//! value per driven mate, in millimetres, degrees or percent. Applying one edits those mates
//! as Edit mate does, so a value that cannot be met is refused and changes nothing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::AssemblyDocument;
use crate::mates::bodies::{self, Scope};
use crate::mates::{Mate, MateEdit, MateEntry, MateId, MateRefusal, SlotConstraint, WidthConstraint};
use crate::path::{ComponentId, OccurrencePath, Step};
use crate::placement::Placement;
use crate::schema::Assembly;

/// The mates a Mate Controller drives and its saved positions.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MateController {
    /// The driven mates, in order.
    pub mates: Vec<MateId>,
    /// The saved positions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub positions: Vec<ControllerPosition>,
}

/// One saved position: a value for each driven mate, in the controller's order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControllerPosition {
    /// Its name, unique in the controller.
    pub name: String,
    /// Millimetres, degrees or percent, one per driven mate.
    pub values: Vec<f64>,
}

/// What a mate's driven value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Driven {
    Length,
    Degrees,
    Percent,
}

fn driven(mate: &Mate) -> Option<Driven> {
    match mate {
        Mate::Angle { .. } => Some(Driven::Degrees),
        Mate::Width { constraint: WidthConstraint::Percent { .. }, .. }
        | Mate::Slot { constraint: SlotConstraint::Percent { .. }, .. }
        | Mate::Path { constraint: SlotConstraint::Percent { .. }, .. } => Some(Driven::Percent),
        m if m.expression().is_some() => Some(Driven::Length),
        _ => None,
    }
}

fn known(truth: &Assembly, id: MateId) -> Option<&Mate> {
    truth.mates.iter().filter_map(MateEntry::known).find(|m| m.id() == id)
}

/// The value a driven mate holds now.
fn value_of(truth: &Assembly, mate: &Mate) -> Result<f64, MateRefusal> {
    let name = mate.name();
    match (driven(mate), mate) {
        (Some(Driven::Degrees), Mate::Angle { degrees, .. }) => Ok(*degrees),
        (
            Some(Driven::Percent),
            Mate::Width { constraint: WidthConstraint::Percent { percent, .. }, .. }
            | Mate::Slot { constraint: SlotConstraint::Percent { percent, .. }, .. }
            | Mate::Path { constraint: SlotConstraint::Percent { percent, .. }, .. },
        ) => Ok(*percent),
        (Some(Driven::Length), m) => {
            let e = m.expression().expect("a driven length");
            if !e.references().is_empty() {
                return Err(MateRefusal::invalid(format!(
                    "{name} is driven by {}; the Mate Controller drives literal values",
                    e.text()
                )));
            }
            truth.values.length(e).map_err(|e| super::command::value_refusal(name, e))
        }
        _ => Err(MateRefusal::invalid(format!(
            "{name} is a {} mate; the Mate Controller drives distances, angles and positions along a slot or path",
            mate.kind()
        ))),
    }
}

/// The edit that sets a driven mate's value.
fn edit_for(mate: &Mate, value: f64) -> MateEdit {
    match driven(mate) {
        Some(Driven::Degrees) => MateEdit::Degrees(value),
        Some(Driven::Percent) => MateEdit::Percent(value),
        _ => MateEdit::Distance(format!("{value} mm")),
    }
}

/// Takes mates the assembly no longer holds out of the controller.
pub(crate) fn prune(truth: &mut Assembly) {
    let gone: Vec<MateId> =
        truth.mate_controller.iter().flat_map(|c| &c.mates).copied().filter(|id| known(truth, *id).is_none()).collect();
    for id in gone {
        forget(truth, id);
    }
}

/// Takes a deleted mate out of the controller and its positions.
pub(crate) fn forget(truth: &mut Assembly, id: MateId) {
    let Some(c) = truth.mate_controller.as_mut() else { return };
    if let Some(i) = c.mates.iter().position(|m| *m == id) {
        c.mates.remove(i);
        for p in &mut c.positions {
            if i < p.values.len() {
                p.values.remove(i);
            }
        }
    }
    if c.mates.is_empty() {
        truth.mate_controller = None;
    }
}

fn controller(truth: &Assembly) -> Result<&MateController, MateRefusal> {
    truth.mate_controller.as_ref().ok_or_else(|| MateRefusal::invalid("the assembly has no Mate Controller"))
}

fn position<'a>(truth: &'a Assembly, name: &str) -> Result<&'a ControllerPosition, MateRefusal> {
    controller(truth)?
        .positions
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| MateRefusal::invalid(format!("the Mate Controller has no position named {name}")))
}

/// The edits that put the driven mates at `values`.
fn edits(truth: &Assembly, values: &[f64]) -> Result<Vec<(String, MateEdit)>, MateRefusal> {
    let c = controller(truth)?;
    if values.len() != c.mates.len() || values.iter().any(|v| !v.is_finite()) {
        return Err(MateRefusal::invalid(format!(
            "the Mate Controller drives {} mates; give one finite value for each",
            c.mates.len()
        )));
    }
    c.mates
        .iter()
        .zip(values)
        .map(|(id, v)| {
            let mate = known(truth, *id).ok_or_else(|| MateRefusal::invalid(format!("no mate with id {}", id.0)))?;
            Ok((mate.name().to_owned(), edit_for(mate, *v)))
        })
        .collect()
}

impl AssemblyDocument {
    /// The Mate Controller, when the assembly has one.
    pub fn mate_controller(&self) -> Option<&MateController> {
        self.assembly().mate_controller.as_ref()
    }

    /// The names of the mates the controller drives, in order.
    pub fn controller_mates(&self) -> Vec<String> {
        let truth = self.assembly();
        truth
            .mate_controller
            .iter()
            .flat_map(|c| &c.mates)
            .filter_map(|id| known(truth, *id).map(|m| m.name().to_owned()))
            .collect()
    }

    /// Chooses the mates the controller drives, as one undo step. Saved positions keep the
    /// values of mates still driven and take the current value of a newly driven one. No
    /// mates removes the controller.
    pub fn set_controller_mates(&mut self, names: &[String]) -> Result<(), MateRefusal> {
        self.transact("Mate Controller", false, |_, truth| {
            let mut ids = Vec::new();
            let mut now = Vec::new();
            for name in names {
                let mate = truth
                    .mates
                    .iter()
                    .filter_map(MateEntry::known)
                    .find(|m| m.name() == name)
                    .ok_or_else(|| MateRefusal::invalid(format!("no mate named {name}")))?;
                if ids.contains(&mate.id()) {
                    return Err(MateRefusal::invalid(format!("{name} is named twice")));
                }
                now.push(value_of(truth, mate)?);
                ids.push(mate.id());
            }
            if ids.is_empty() {
                truth.mate_controller = None;
                return Ok(());
            }
            let old = truth.mate_controller.take().unwrap_or_default();
            let positions = old
                .positions
                .into_iter()
                .map(|p| {
                    let values = ids
                        .iter()
                        .zip(&now)
                        .map(|(id, v)| {
                            old.mates.iter().position(|m| m == id).and_then(|i| p.values.get(i)).unwrap_or(v)
                        })
                        .copied()
                        .collect();
                    ControllerPosition { name: p.name, values }
                })
                .collect();
            truth.mate_controller = Some(MateController { mates: ids, positions });
            Ok(())
        })
    }

    /// Saves the driven mates' current values as a position, replacing one of that name.
    /// One undo step.
    pub fn save_controller_position(&mut self, name: &str) -> Result<(), MateRefusal> {
        let name = name.trim();
        if name.is_empty() {
            return Err(MateRefusal::invalid("a position needs a name"));
        }
        self.transact("Save position", false, |_, truth| {
            let ids = controller(truth)?.mates.clone();
            let values = ids
                .iter()
                .map(|id| {
                    let mate =
                        known(truth, *id).ok_or_else(|| MateRefusal::invalid(format!("no mate with id {}", id.0)))?;
                    value_of(truth, mate)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let c = truth.mate_controller.as_mut().expect("checked");
            match c.positions.iter_mut().find(|p| p.name == name) {
                Some(p) => p.values = values,
                None => c.positions.push(ControllerPosition { name: name.to_owned(), values }),
            }
            Ok(())
        })
    }

    /// Removes a saved position. One undo step.
    pub fn delete_controller_position(&mut self, name: &str) -> Result<(), MateRefusal> {
        self.transact("Delete position", false, |_, truth| {
            position(truth, name)?;
            truth.mate_controller.as_mut().expect("checked").positions.retain(|p| p.name != name);
            Ok(())
        })
    }

    /// Puts the driven mates at a saved position, as one undo step.
    pub fn apply_controller_position(&mut self, name: &str) -> Result<(), MateRefusal> {
        let values = position(self.assembly(), name)?.values.clone();
        self.apply_controller_values(&values)
    }

    /// Puts the driven mates at `values`, one per driven mate, as one undo step.
    pub fn apply_controller_values(&mut self, values: &[f64]) -> Result<(), MateRefusal> {
        let edits = edits(self.assembly(), values)?;
        self.edit_mates(&edits)
    }

    /// The driven values `t` of the way from one saved position to another, `t` from 0 to 1.
    pub fn controller_values_between(&self, from: &str, to: &str, t: f64) -> Result<Vec<f64>, MateRefusal> {
        let truth = self.assembly();
        let (a, b) = (&position(truth, from)?.values, &position(truth, to)?.values);
        Ok(a.iter().zip(b).map(|(a, b)| a + (b - a) * t.clamp(0.0, 1.0)).collect())
    }

    /// The root components' placements at `steps + 1` evenly spaced frames from one saved
    /// position to another, both included. Solves each frame and changes nothing.
    pub fn controller_frames(
        &mut self,
        from: &str,
        to: &str,
        steps: usize,
    ) -> Result<Vec<Vec<(ComponentId, Placement)>>, MateRefusal> {
        if steps == 0 || steps > 600 {
            return Err(MateRefusal::invalid("an animation takes from 1 to 600 steps"));
        }
        self.cancel_drag();
        self.finish_loading();
        let mut frames = Vec::with_capacity(steps + 1);
        for k in 0..=steps {
            let values = self.controller_values_between(from, to, k as f64 / steps as f64)?;
            let candidate = self.controller_candidate(&values)?;
            frames.push(candidate.components.iter().map(|c| (c.id, c.placement)).collect());
        }
        Ok(frames)
    }

    /// Shows the driven mates at `values` without an undo step, as a drag preview does: the
    /// next edit, `cancel_drag` or `apply_controller_values` ends it. For an animation.
    pub fn preview_controller_values(&mut self, values: &[f64]) -> Result<(), MateRefusal> {
        if !self.dragging() {
            let first = self.assembly().components.first().map(|c| c.id);
            let first = first.ok_or_else(|| MateRefusal::invalid("the assembly has no components"))?;
            self.begin_drag(OccurrencePath::root().child(Step::component(first)))
                .map_err(|e| MateRefusal::invalid(e.to_string()))?;
        }
        let base = self.drag.as_ref().expect("a preview").base.clone();
        let candidate = self.controller_candidate_from(base, values)?;
        self.show_drag_candidate(candidate);
        Ok(())
    }

    fn controller_candidate(&mut self, values: &[f64]) -> Result<Assembly, MateRefusal> {
        let base = self.assembly().clone();
        self.controller_candidate_from(base, values)
    }

    /// `base` with the driven mates at `values`, solved and proved.
    fn controller_candidate_from(&mut self, mut candidate: Assembly, values: &[f64]) -> Result<Assembly, MateRefusal> {
        let edits = edits(&candidate, values)?;
        self.edit_in(&mut candidate, &edits)?;
        let solved = self.solve_truth(&candidate)?;
        let kept = bodies::locals(&Scope::new(&candidate, &self.table, &self.store), &solved.placements);
        bodies::keep(&mut candidate, kept);
        self.refresh_mechanical(&mut candidate, None)?;
        if !solved.status.bounded.is_empty() {
            self.verify_problem(self.gather(&Scope::new(&candidate, &self.table, &self.store))?)?;
        }
        Ok(candidate)
    }
}

/// The driven mates' values by name, for reports.
pub fn controller_values(doc: &AssemblyDocument) -> BTreeMap<String, f64> {
    let truth = doc.assembly();
    truth
        .mate_controller
        .iter()
        .flat_map(|c| &c.mates)
        .filter_map(|id| known(truth, *id))
        .filter_map(|m| Some((m.name().to_owned(), value_of(truth, m).ok()?)))
        .collect()
}
