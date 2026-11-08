//! Mate commands. Every one runs the same way: copy the truth, change the copy, prove and
//! solve it, and only then publish truth, pose and undo as one step. A refusal changes
//! nothing, and Update solves its candidate through the same [`AssemblyDocument::solve_truth`].

use std::collections::BTreeMap;

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::{Signed, Zero};
use serde::Serialize;
use soliddraft_document::{ContentHash, DocRef, DocumentId};
use soliddraft_kernel_api::{Certainty, ExactRadius, ExactRational, ExactSupport};
use soliddraft_model::ValueTable;

use crate::document::{AssemblyDocument, EditError, key_of};
use crate::mates::bodies::{End, Gathered, Problem, Rule, Scope};
use crate::mates::motion::{self, DistanceEntity, Relation};
use crate::mates::reference::{Shape, ShapeKind, check_endpoint};
use crate::mates::solver::{
    self, Along, Body, Constraint, FreeMotion, LocalAxis, LocalPlane, LocalPoint, LocalSlot, Mirrored, Slide,
    SolverState, SolverStatus,
};
use crate::mates::{
    Alignment, EndpointSelector, Entity, FaceEndpoint, Mate, MateEntry, MateId, MateRefusal, PartEndpoint, PlaneProof,
    RefusalCode, SlotConstraint, StoredSupport, StoredTurn, SupportProof, WidthConstraint,
};
use crate::path::{ComponentId, OccurrencePath, Step};
use crate::placement::Placement;
use crate::schema::Assembly;
use crate::store::Freshness;

/// One change to a mate.
#[derive(Clone, Debug, PartialEq)]
pub enum MateEdit {
    /// A new signed angle in degrees.
    Degrees(f64),
    /// New inclusive signed angular limits in degrees.
    AngleLimits([f64; 2]),
    /// A new literal distance, of a Distance or a Width by dimension.
    Distance(String),
    /// A new relation between the normals.
    Alignment(Alignment),
    /// Whether an Origin mate aligns the axes too.
    AlignAxes(bool),
    /// A Width's new percentage.
    Percent(f64),
    /// Whether a Width measures from its second groove face.
    Flipped(bool),
    /// How a Hinge's planes relate.
    PlaneAlignment(Alignment),
    /// Whether a Concentric, a Profile Center between circles or a Slot holds the turn where
    /// it is.
    LockRotation(bool),
    /// A Limit Distance's new least distance.
    Min(String),
    /// A Limit Distance's new greatest distance.
    Max(String),
    /// A Linear Coupler's new ratio; the parts' places become its zero.
    Ratio([f64; 2]),
    /// Whether a Linear Coupler's second part slides the other way; the parts' places become
    /// its zero.
    Reverse(bool),
}

/// Where a new Slot mate puts the pin along the slot.
#[derive(Clone, Debug, PartialEq)]
pub enum SlotInput {
    /// Anywhere between the round ends' centres.
    Free,
    /// Halfway between them.
    Center,
    /// A literal distance from the first end's centre, or the second's when `flipped`.
    Distance {
        /// The distance, as a length such as 3 mm.
        distance: String,
        /// Measured from the second end.
        flipped: bool,
    },
    /// That distance as a percentage of the run between the centres.
    Percent {
        /// From 0 to 100.
        percent: f64,
        /// Measured from the second end.
        flipped: bool,
    },
}

/// Where a new Width mate puts the tab.
#[derive(Clone, Debug, PartialEq)]
pub enum WidthInput {
    /// Anywhere between the groove's faces.
    Free,
    /// Centred in the groove.
    Centered,
    /// A literal gap from the first groove face, or the second when `flipped`, to the
    /// tab face nearest it.
    Dimension {
        /// The gap, as a length such as 3 mm.
        distance: String,
        /// Measured from the second groove face.
        flipped: bool,
    },
    /// That gap as a percentage of the play.
    Percent {
        /// From 0 to 100.
        percent: f64,
        /// Measured from the second groove face.
        flipped: bool,
    },
}

/// What a solve found.
pub(crate) struct Solved {
    /// The parts it moved, in the root's frame.
    pub placements: BTreeMap<Body, Placement>,
    pub status: SolverStatus,
}

struct ProvenProblem {
    bodies: BTreeMap<Body, Placement>,
    constraints: Vec<Constraint>,
    parts: usize,
}

/// One side of a measurement.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProofIdentity {
    /// The occurrence.
    pub path: OccurrencePath,
    /// The part.
    pub document: DocumentId,
    /// The accepted revision the proof is on.
    pub revision: ContentHash,
    /// That revision's geometry hash.
    pub geometry: ContentHash,
    /// The face's label at that revision.
    pub label: String,
    /// True when the source has moved on, so this proof is of the accepted revision only.
    pub retained: bool,
}

/// The signed distance between two parallel placed planes, from `a` along its outward
/// normal, on the placements in the assembly's root frame.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlaneMeasurement {
    /// What is measured.
    pub quantity: &'static str,
    /// The unit.
    pub unit: &'static str,
    /// The frame.
    pub frame: &'static str,
    /// The first face.
    pub a: ProofIdentity,
    /// The second face.
    pub b: ProofIdentity,
    /// The placements it was measured on.
    pub placements: [Placement; 2],
    /// Exact or bounded.
    pub certainty: Certainty,
    /// The nearest double.
    pub value: f64,
    /// Lower bound, rounded outward.
    pub lo: f64,
    /// Upper bound, rounded outward.
    pub hi: f64,
    /// The exact value as a fraction.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub rational: String,
}

pub(crate) fn value_refusal(what: &str, message: String) -> MateRefusal {
    let code = match message.as_str() {
        "value cycle" | "value graph is too deep" => RefusalCode::ValueCycle,
        "incompatible units in sum" | "the input needs a length" => RefusalCode::UnitMismatch,
        _ => RefusalCode::InvalidValue,
    };
    MateRefusal::new(code, format!("{what}: {message}"))
}

fn component_path(c: ComponentId) -> OccurrencePath {
    OccurrencePath::root().child(Step::component(c))
}

fn mate_named<'a>(truth: &'a mut Assembly, name: &str) -> Result<&'a mut Mate, MateRefusal> {
    truth
        .mates
        .iter_mut()
        .find_map(|m| match m {
            MateEntry::Known(m) if m.name() == name => Some(m),
            _ => None,
        })
        .ok_or_else(|| MateRefusal::invalid(format!("no mate named {name}")))
}

fn check_new_name(truth: &Assembly, name: &str) -> Result<(), MateRefusal> {
    if name.trim().is_empty() {
        return Err(MateRefusal::invalid("a mate needs a name"));
    }
    if truth.known_mates().any(|m| m.name() == name) {
        return Err(MateRefusal::invalid(format!("the name {name} is already taken")));
    }
    Ok(())
}

/// `refusal` naming `mate`.
fn named(refusal: MateRefusal, mate: &str) -> MateRefusal {
    MateRefusal { mates: vec![mate.to_owned()], ..refusal }
}

/// A length with no reference to any value.
fn literal(text: &str) -> Result<soliddraft_model::Expression, MateRefusal> {
    let none = ValueTable::default();
    let expression = none.parse(text).map_err(|e| value_refusal("the distance", e))?;
    none.length(&expression).map_err(|e| value_refusal("the distance", e))?;
    Ok(expression)
}

impl AssemblyDocument {
    /// Runs `edit` on a copy of the truth, proves and solves the copy, and publishes it
    /// as one undo step. With `strict` a candidate that leaves any freedom is refused.
    fn transact<T>(
        &mut self,
        label: &str,
        strict: bool,
        edit: impl FnOnce(&mut AssemblyDocument, &mut Assembly) -> Result<T, MateRefusal>,
    ) -> Result<T, MateRefusal> {
        self.finish_loading();
        let mut candidate = self.assembly.clone();
        let out = edit(self, &mut candidate)?;
        let solved = self.solve_truth(&candidate)?;
        if strict && solved.status.remaining_dof > 0 {
            let free: Vec<String> = solved.status.free.iter().map(solver::describe).collect();
            return Err(MateRefusal {
                remaining_dof: Some(solved.status.remaining_dof),
                free: solved.status.free,
                ..MateRefusal::new(
                    RefusalCode::UnderConstrained,
                    format!("{} freedoms would remain: {}", solved.status.remaining_dof, free.join(", ")),
                )
            });
        }
        let kept = crate::mates::bodies::locals(&Scope::new(&candidate, &self.table, &self.store), &solved.placements);
        crate::mates::bodies::keep(&mut candidate, kept);
        if !solved.status.bounded.is_empty() {
            let problem = self.gather(&Scope::new(&candidate, &self.table, &self.store))?;
            self.verify_problem(problem)?;
        }
        if candidate != self.assembly {
            self.publish(label, candidate);
        }
        Ok(out)
    }

    /// Makes `candidate`, which differs in mates, values, placements and the overrides
    /// that place parts inside sub-assemblies only, the truth.
    fn publish(&mut self, label: &str, candidate: Assembly) {
        let moved: Vec<(ComponentId, Placement)> = candidate
            .components
            .iter()
            .filter(|c| self.assembly.component(c.id).is_none_or(|old| old.placement != c.placement))
            .map(|c| (c.id, c.placement))
            .collect();
        let ids: Vec<ComponentId> = moved.iter().map(|(c, _)| *c).collect();
        let mut overrides: Vec<OccurrencePath> = self
            .assembly
            .overrides
            .iter()
            .chain(&candidate.overrides)
            .map(|o| o.path.clone())
            .filter(|p| self.assembly.override_at(p) != candidate.override_at(p))
            .collect();
        overrides.sort();
        overrides.dedup();
        let rows: Vec<_> = ids
            .iter()
            .map(|c| component_path(*c))
            .chain(overrides.iter().cloned())
            .filter_map(|p| self.table.find(&p))
            .collect();
        let rows_before = rows.iter().map(|r| self.row_state(*r)).collect();
        let mut before = self.image(&ids, &overrides);
        before.design = Some(crate::document::Design::of(&self.assembly));
        crate::document::Design::of(&candidate).restore(&mut self.assembly);
        self.assembly.next_id = candidate.next_id;
        for (id, placement) in &moved {
            if let Some(c) = self.assembly.component_mut(*id) {
                c.placement = *placement;
            }
        }
        for p in &overrides {
            self.assembly.overrides.retain(|o| o.path != *p);
            self.assembly.overrides.extend(candidate.override_at(p).cloned());
        }
        self.assembly.overrides.sort_by(|a, b| a.path.cmp(&b.path));
        let mut placed: Vec<(OccurrencePath, Placement)> =
            moved.iter().map(|(id, placement)| (component_path(*id), *placement)).collect();
        let scope = Scope::new(&self.assembly, &self.table, &self.store);
        placed.extend(overrides.iter().filter_map(|p| Some((p.clone(), scope.local(p)?))));
        for (path, placement) in &placed {
            if let Some(row) = self.table.find(path) {
                let shifted = self.table.set_local(row, *placement, &self.store);
                self.bvh_moved.extend(shifted.iter().copied());
                self.changes.moved.extend(shifted);
            }
        }
        self.record(label, before, rows_before);
        // A pattern's members follow their seed, and the table holds them as rows of their own.
        if self.assembly.patterns.iter().any(|p| p.seeds.iter().any(|s| ids.contains(s))) {
            self.reflatten();
        }
    }

    /// Proves every face `truth`'s mates name at the revisions it pins, and solves it.
    /// `truth` has the structure the document shows; its sub-assemblies bring their own
    /// mates.
    pub(crate) fn solve_truth(&mut self, truth: &Assembly) -> Result<Solved, MateRefusal> {
        let problem = self.gather(&Scope::new(truth, &self.table, &self.store))?;
        self.solve_problem(problem)
    }

    /// Stores on each root mate endpoint that reads an axis or a point the one proved at
    /// `candidate`'s revisions. Update proves the new support to solve on it, and the
    /// status reads what is stored, so the stored one must be the new one too.
    pub(crate) fn refresh_supports(&mut self, candidate: &mut AssemblyDocument) -> Result<(), MateRefusal> {
        let mut ends = Vec::new();
        {
            let scope = Scope::new(&candidate.assembly, &candidate.table, &candidate.store);
            for (i, entry) in candidate.assembly.mates.iter().enumerate() {
                let MateEntry::Known(m) = entry else { continue };
                for (j, e) in m.faces().into_iter().enumerate() {
                    if e.support.is_some() {
                        ends.push((i, j, self.end_of(&scope, e.path.clone(), e, m.name())?));
                    }
                }
            }
        }
        for (i, j, end) in ends {
            let source = self.rebased(&end);
            let support = self.stored_end(&source, &end.entity, end.support.as_ref())?;
            if let MateEntry::Known(m) = &mut candidate.assembly.mates[i] {
                m.faces_mut()[j].support = support;
            }
        }
        Ok(())
    }

    /// Proves the faces `problem` reads and solves it.
    fn prove_problem(&mut self, problem: Problem) -> Result<ProvenProblem, MateRefusal> {
        let mut constraints = Vec::new();
        let mut dangling: Vec<(String, String)> = Vec::new();
        for mate in problem.mates {
            match mate {
                Gathered::Origin { mate, body, axes, at } => {
                    constraints.push(Constraint::Origin { mate, body, axes, at })
                }
                Gathered::Lock { mate, a, b, relative } => constraints.push(Constraint::Lock { mate, a, b, relative }),
                Gathered::Between { mate, ends, rule } => {
                    let mut planes = Vec::new();
                    for end in &ends {
                        let source = self.rebased(end);
                        match self.read_end(&mate, &source, end) {
                            Ok(read) => planes.push(read),
                            Err(r) if r.code == RefusalCode::DanglingReference => {
                                let at = format!(
                                    "{}: occurrence {}, {}: {}",
                                    end.source.path,
                                    end.body,
                                    end.entity.describe(),
                                    r.message
                                );
                                dangling.push((mate.clone(), at));
                            }
                            Err(r) => return Err(MateRefusal { mates: vec![mate.clone()], ..r }),
                        }
                    }
                    if planes.len() == ends.len() {
                        constraints.extend(constraint(mate, planes, &rule)?);
                    }
                }
            }
        }
        if !dangling.is_empty() {
            let mut mates: Vec<String> = dangling.iter().map(|(m, _)| m.clone()).collect();
            mates.dedup();
            let list: Vec<&str> = dangling.iter().map(|(_, at)| at.as_str()).collect();
            let message = format!("{}; mates {}", list.join("; "), mates.join(", "));
            return Err(MateRefusal { mates, ..MateRefusal::new(RefusalCode::DanglingReference, message) });
        }
        Ok(ProvenProblem { bodies: problem.bodies, constraints, parts: problem.parts })
    }

    pub(crate) fn verify_problem(&mut self, problem: Problem) -> Result<SolverStatus, MateRefusal> {
        let p = self.prove_problem(problem)?;
        let (dof, free) = solver::freedoms(&p.bodies, &p.constraints)?;
        let mut status = status_of(dof + 6 * p.parts.saturating_sub(p.bodies.len()), free);
        status.bounded = motion::certificates(&p.bodies, &p.constraints)?;
        Ok(status)
    }

    pub(crate) fn solve_problem(&mut self, problem: Problem) -> Result<Solved, MateRefusal> {
        let p = self.prove_problem(problem)?;
        let placements = solver::solve(&p.bodies, &p.constraints)?;
        let (dof, free) = solver::freedoms(&placements, &p.constraints)?;
        let mut status = status_of(dof + 6 * p.parts.saturating_sub(p.bodies.len()), free);
        status.bounded = motion::certificates(&placements, &p.constraints)?;
        let moved = placements.into_iter().filter(|(b, pose)| p.bodies[b] != *pose).collect();
        Ok(Solved { placements: moved, status })
    }

    /// The accepted assembly's status. Exact groups need stored definitions only;
    /// bounded groups re-prove residuals at their accepted pose.
    pub fn solver(&mut self) -> SolverStatus {
        if let Some(s) = &self.solver_status {
            return s.clone();
        }
        let status = match self.read_status() {
            Ok(Some(status)) => status,
            Ok(None) => self
                .gather(&Scope::new(&self.assembly, &self.table, &self.store))
                .and_then(|p| self.verify_problem(p))
                .unwrap_or_else(|r| SolverStatus::error(r.to_string())),
            Err(error) => SolverStatus::error(error),
        };
        self.solver_status = Some(status.clone());
        status
    }

    fn read_status(&self) -> Result<Option<SolverStatus>, String> {
        let problem = self.gather(&Scope::new(&self.assembly, &self.table, &self.store)).map_err(|r| r.to_string())?;
        let stored = |end: &End| stored_read(end).ok_or("a mate's face has no stored normal, axis or point");
        let mut constraints = Vec::new();
        for mate in problem.mates {
            constraints.push(match mate {
                Gathered::Origin { mate, body, axes, at } => Constraint::Origin { mate, body, axes, at },
                Gathered::Lock { mate, a, b, relative } => Constraint::Lock { mate, a, b, relative },
                Gathered::Between { mate, ends, rule } => {
                    let planes = ends.iter().map(stored).collect::<Result<Vec<_>, _>>()?;
                    constraints.extend(constraint(mate, planes, &rule).map_err(|r| r.to_string())?);
                    continue;
                }
            });
        }
        if motion::needs_motion(&problem.bodies, &constraints) {
            return Ok(None);
        }
        let (dof, free) = solver::freedoms(&problem.bodies, &constraints).map_err(|r| r.to_string())?;
        Ok(Some(status_of(dof + 6 * problem.parts.saturating_sub(problem.bodies.len()), free)))
    }

    /// What `end` reads at `source`: its profile or slot when it stored one, else its
    /// plane, axis or point.
    fn read_end(&mut self, mate: &str, source: &DocRef, end: &End) -> Result<Read, MateRefusal> {
        match end.support.as_ref().and_then(StoredSupport::shape) {
            Some(kind) => {
                Ok(read_shape(end.body.clone(), self.prove_shape(source, &end.entity, kind, end.support.as_ref())?))
            }
            None => read_of(mate, end.body.clone(), &self.prove_support(source, &end.entity, end.support.as_ref())?),
        }
    }

    /// What a mate stores for `entity` at `source`, proving it as `stored` was proved.
    fn stored_end(
        &mut self,
        source: &DocRef,
        entity: &Entity,
        stored: Option<&StoredSupport>,
    ) -> Result<Option<StoredSupport>, MateRefusal> {
        match stored.and_then(StoredSupport::shape) {
            Some(kind) => Ok(Some(self.prove_shape(source, entity, kind, stored)?.stored())),
            None => Ok(self.prove_support(source, entity, stored)?.stored()),
        }
    }

    /// How many parts were rebuilt so far to prove planes.
    pub fn parts_rebuilt_for_proofs(&self) -> usize {
        self.proofs.built
    }

    fn endpoint(&mut self, truth: &Assembly, selector: &EndpointSelector) -> Result<FaceEndpoint, MateRefusal> {
        self.endpoint_as(truth, selector, None, "")
    }

    /// An endpoint that reads a profile or a slot when `shape` says so, else its plane, axis
    /// or point. A profile or slot it cannot read is refused naming `mate`.
    fn endpoint_as(
        &mut self,
        truth: &Assembly,
        selector: &EndpointSelector,
        shape: Option<ShapeKind>,
        mate: &str,
    ) -> Result<FaceEndpoint, MateRefusal> {
        let source = Self::part_source(truth, selector.component)?;
        let entity = self.select(&source, &selector.face)?;
        let document = source.id.ok_or_else(|| MateRefusal::invalid("the source has no document id"))?;
        let support = match shape {
            Some(kind) => Some(self.prove_shape(&source, &entity, kind, None).map_err(|r| named(r, mate))?.stored()),
            None => self.stored_support(&source, &entity)?,
        };
        Ok(FaceEndpoint {
            path: component_path(selector.component),
            document,
            configuration: source.configuration,
            entity,
            support,
        })
    }

    fn identity(
        &self,
        path: &OccurrencePath,
        source: &DocRef,
        proof: &PlaneProof,
    ) -> Result<ProofIdentity, MateRefusal> {
        let retained = key_of(source)
            .and_then(|k| self.store.find(&k))
            .is_some_and(|d| !matches!(self.store.get(d).freshness, Freshness::Current | Freshness::Unchecked));
        Ok(ProofIdentity {
            path: path.clone(),
            document: proof.document,
            revision: proof.revision.clone(),
            geometry: proof.geometry.clone(),
            label: proof.label.clone(),
            retained,
        })
    }

    fn measure(&mut self, a: &FaceEndpoint, b: &FaceEndpoint) -> Result<PlaneMeasurement, MateRefusal> {
        check_endpoint(a)?;
        check_endpoint(b)?;
        let (ea, eb, placements) = {
            let scope = Scope::new(&self.assembly, &self.table, &self.store);
            let world = |e: &FaceEndpoint| {
                scope.world(&e.path).ok_or_else(|| MateRefusal::invalid(format!("no occurrence at {}", e.path)))
            };
            let placements = [world(a)?, world(b)?];
            (self.end_of(&scope, a.path.clone(), a, "")?, self.end_of(&scope, b.path.clone(), b, "")?, placements)
        };
        let (ra, rb) = (self.rebased(&ea), self.rebased(&eb));
        let (pa, pb) = (self.prove(&ra, &a.entity)?, self.prove(&rb, &b.entity)?);
        let (certainty, value, lo, hi, rational) = match signed_gap(&pa, &pb, &placements) {
            Ok(gap) => {
                let value = solver::nearest(&gap);
                let (mut lo, mut hi) = (value, value);
                if solver::exact(lo).is_some_and(|r| r > gap) {
                    lo = lo.next_down();
                }
                if solver::exact(hi).is_some_and(|r| r < gap) {
                    hi = hi.next_up();
                }
                (Certainty::Exact, value, lo, hi, solver::text(&gap))
            }
            Err(r) if r.code == RefusalCode::UnsupportedPlacement => {
                let p = |v: &PlaneProof| LocalPlane {
                    normal: v.plane.normal(),
                    offset: solver::rational(&v.plane.offset).expect("proved rational"),
                };
                let (value, lo, hi) = motion::measure_gap(&p(&pa), &p(&pb), &placements)?;
                (Certainty::Bounded, value, lo, hi, String::new())
            }
            Err(r) => return Err(r),
        };
        Ok(PlaneMeasurement {
            quantity: "signed_support_plane_distance",
            unit: "mm",
            frame: "root",
            a: self.identity(&a.path, &ra, &pa)?,
            b: self.identity(&b.path, &rb, &pb)?,
            placements,
            certainty,
            value,
            lo,
            hi,
            rational,
        })
    }

    /// Measures the signed distance between two faces' planes. Changes no truth.
    pub fn measure_planes(
        &mut self,
        a: &EndpointSelector,
        b: &EndpointSelector,
    ) -> Result<PlaneMeasurement, MateRefusal> {
        self.finish_loading();
        let truth = self.assembly.clone();
        let (a, b) = (self.endpoint(&truth, a)?, self.endpoint(&truth, b)?);
        self.measure(&a, &b)
    }

    /// What each mate between faces measures on the accepted placements, by proof: the
    /// signed gap for those between parallel planes, and nothing but the proofs for a
    /// Perpendicular. A mate whose proof cannot be had says why; the accepted pose stands.
    pub fn mate_measurements(&mut self) -> Vec<(MateId, Result<Option<PlaneMeasurement>, MateRefusal>)> {
        let mates: Vec<Mate> = self.assembly.known_mates().cloned().collect();
        mates
            .iter()
            .filter(|m| !m.faces().is_empty())
            .map(|m| {
                let planes = m.faces().iter().all(|e| e.support.is_none() && e.entity.stored_normal().is_some());
                let measured = match m.measured() {
                    Some([a, b]) if planes => self.measure(a, b).map(Some),
                    _ => self.prove_ends(m).map(|()| None),
                };
                (m.id(), measured)
            })
            .collect()
    }

    /// Proves every face a mate names, on the accepted placements' revisions.
    fn prove_ends(&mut self, mate: &Mate) -> Result<(), MateRefusal> {
        let ends = {
            let scope = Scope::new(&self.assembly, &self.table, &self.store);
            let mut ends = Vec::new();
            for e in mate.faces() {
                check_endpoint(e)?;
                ends.push(self.end_of(&scope, e.path.clone(), e, mate.name())?);
            }
            ends
        };
        for end in &ends {
            let source = self.rebased(end);
            self.stored_end(&source, &end.entity, end.support.as_ref())?;
        }
        Ok(())
    }

    /// Adds a signed angle about an explicitly named plane normal or axis direction.
    pub fn mate_angle(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        reference: &EndpointSelector,
        degrees: f64,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        check_degrees(degrees)?;
        self.transact("Angle mate", strict, |doc, truth| {
            check_new_name(truth, name)?;
            if a.component == b.component {
                return Err(MateRefusal::invalid("an Angle relates two different parts"));
            }
            let (a, b, reference) = (doc.endpoint(truth, a)?, doc.endpoint(truth, b)?, doc.endpoint(truth, reference)?);
            let id = MateId(truth.take_id());
            truth.mates.push(MateEntry::Known(Mate::Angle { id, name: name.into(), a, b, reference, degrees }));
            Ok(id)
        })
    }

    /// Adds inclusive signed angle limits, with the same reference direction as Angle.
    pub fn mate_limit_angle(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        reference: &EndpointSelector,
        limits: [f64; 2],
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        check_angle_limits(limits)?;
        self.transact("Limit angle mate", strict, |doc, truth| {
            check_new_name(truth, name)?;
            if a.component == b.component {
                return Err(MateRefusal::invalid("a Limit Angle relates two different parts"));
            }
            let (a, b, reference) = (doc.endpoint(truth, a)?, doc.endpoint(truth, b)?, doc.endpoint(truth, reference)?);
            let id = MateId(truth.take_id());
            truth.mates.push(MateEntry::Known(Mate::LimitAngle {
                id,
                name: name.into(),
                a,
                b,
                reference,
                min_degrees: limits[0],
                max_degrees: limits[1],
            }));
            Ok(id)
        })
    }

    /// Adds an Origin mate on a component. One undo step.
    pub fn mate_origin(
        &mut self,
        name: &str,
        component: ComponentId,
        align_axes: bool,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.transact("Origin mate", strict, |_, truth| {
            check_new_name(truth, name)?;
            if truth.component(component).is_none() {
                return Err(MateRefusal::invalid(format!("the assembly has no component {component}")));
            }
            let id = MateId(truth.take_id());
            truth.mates.push(MateEntry::Known(Mate::Origin { id, name: name.into(), component, align_axes }));
            Ok(id)
        })
    }

    /// Adds a distance mate between two faces. One undo step. Partial mates are accepted
    /// unless `strict` asks for a fully constrained result.
    pub fn mate_distance(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        distance: &str,
        alignment: Alignment,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.mate_faces(name, [a, b], alignment, Some(distance), strict)
    }

    /// Puts two faces on one plane. One undo step. Partial mates are accepted unless
    /// `strict` asks for a fully constrained result.
    pub fn mate_coincident(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        alignment: Alignment,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.mate_faces(name, [a, b], alignment, None, strict)
    }

    /// Keeps two faces' planes parallel, their normals as `alignment` says. One undo step.
    pub fn mate_parallel(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        alignment: Alignment,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.mate_between("Parallel mate", name, [a, b], strict, |id, name, [a, b]| {
            Ok(Mate::Parallel { id, name, a, b, alignment })
        })
    }

    /// Keeps two faces' planes at right angles. One undo step.
    pub fn mate_perpendicular(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.mate_between("Perpendicular mate", name, [a, b], strict, |id, name, [a, b]| {
            Ok(Mate::Perpendicular { id, name, a, b })
        })
    }

    /// Puts a tab between the two faces of a groove. `alignment` relates `tab[0]`'s normal to
    /// `groove[0]`'s. One undo step.
    pub fn mate_width(
        &mut self,
        name: &str,
        groove: [&EndpointSelector; 2],
        tab: [&EndpointSelector; 2],
        alignment: Alignment,
        width: &WidthInput,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.transact("Width mate", strict, |doc, truth| {
            check_new_name(truth, name)?;
            if groove[0].component != groove[1].component || tab[0].component != tab[1].component {
                return Err(MateRefusal::invalid("a groove's two faces are on one component, and a tab's on one"));
            }
            if groove[0].component == tab[0].component {
                return Err(MateRefusal::invalid("a mate ties two different components"));
            }
            let constraint = match width {
                WidthInput::Free => WidthConstraint::Free,
                WidthInput::Centered => WidthConstraint::Centered,
                WidthInput::Dimension { distance, flipped } => {
                    WidthConstraint::Dimension { distance: literal(distance)?, flipped: *flipped }
                }
                WidthInput::Percent { percent, flipped } => {
                    WidthConstraint::Percent { percent: check_percent(*percent)?, flipped: *flipped }
                }
            };
            let groove = [doc.endpoint(truth, groove[0])?, doc.endpoint(truth, groove[1])?];
            let tab = [doc.endpoint(truth, tab[0])?, doc.endpoint(truth, tab[1])?];
            if groove[0].entity == groove[1].entity || tab[0].entity == tab[1].entity {
                return Err(MateRefusal::invalid("a groove and a tab each have two different faces"));
            }
            let id = MateId(truth.take_id());
            truth.mates.push(MateEntry::Known(Mate::Width {
                id,
                name: name.into(),
                groove,
                tab,
                alignment,
                constraint,
            }));
            Ok(id)
        })
    }

    /// Locks `b` where it is relative to `a`, exactly for cube placements or with a
    /// certified residual bound for general placements. One undo step.
    pub fn mate_lock(
        &mut self,
        name: &str,
        a: ComponentId,
        b: ComponentId,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.transact("Lock mate", strict, |_, truth| {
            check_new_name(truth, name)?;
            if a == b {
                return Err(MateRefusal::invalid("a mate ties two different components"));
            }
            let end = |c: ComponentId| -> Result<(PartEndpoint, Placement), MateRefusal> {
                let source = Self::part_source(truth, c)?;
                let document = source.id.ok_or_else(|| MateRefusal::invalid("the source has no document id"))?;
                let placement = truth.component(c).expect("part_source found it").placement;
                Ok((PartEndpoint { path: component_path(c), document, configuration: source.configuration }, placement))
            };
            let ((ea, pa), (eb, pb)) = (end(a)?, end(b)?);
            let relative = relative_lock(name, &pa, &pb)?;
            let id = MateId(truth.take_id());
            truth.mates.push(MateEntry::Known(Mate::Lock { id, name: name.into(), a: ea, b: eb, relative }));
            Ok(id)
        })
    }

    /// Puts `b`'s axis on `a`'s line, a point on an axis, or two points on one. `alignment`
    /// relates two axes' directions and is not stored where a point is involved. With
    /// `lock_rotation` the turn about the axis is held where the solve leaves it, which
    /// must be a quarter-turn rotation from `a`. One undo step.
    pub fn mate_concentric(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        alignment: Alignment,
        lock_rotation: bool,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.transact("Concentric mate", strict, |doc, truth| {
            check_new_name(truth, name)?;
            if a.component == b.component {
                return Err(MateRefusal::invalid("a mate ties two different components"));
            }
            let (a, b) = (doc.endpoint(truth, a)?, doc.endpoint(truth, b)?);
            let axis = |e: &FaceEndpoint| match &e.support {
                Some(StoredSupport::Axis { .. }) => true,
                Some(StoredSupport::Enclosed { support }) => {
                    matches!(support.as_ref(), soliddraft_kernel_api::EnclosedSupport::Axis { .. })
                }
                _ => false,
            };
            let axes = axis(&a) && axis(&b);
            if lock_rotation && !axes {
                return Err(MateRefusal::invalid("Lock rotation holds the turn between two axes"));
            }
            let id = MateId(truth.take_id());
            let alignment = axes.then_some(alignment);
            let paths = [a.path.clone(), b.path.clone()];
            truth.mates.push(MateEntry::Known(Mate::Concentric {
                id,
                name: name.into(),
                a,
                b,
                alignment,
                lock_rotation: None,
            }));
            if lock_rotation {
                let turn = doc.solved_turn(truth, &paths, name)?;
                if let Mate::Concentric { lock_rotation, .. } = mate_named(truth, name)? {
                    *lock_rotation = Some(turn);
                }
            }
            Ok(id)
        })
    }

    /// Lays a cylinder or sphere `b` against the plane `a`, one radius from it. One undo step.
    pub fn mate_tangent(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        alignment: Alignment,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.mate_between("Tangent mate", name, [a, b], strict, |id, name, [a, b]| {
            Ok(Mate::Tangent { id, name, a, b, alignment })
        })
    }

    /// Puts two axes on one line and two planes on one plane, under one name. The first
    /// axis and plane are on one component and the second on another. One undo step.
    pub fn mate_hinge(
        &mut self,
        name: &str,
        axes: [&EndpointSelector; 2],
        planes: [&EndpointSelector; 2],
        alignment: Alignment,
        plane_alignment: Alignment,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.transact("Hinge mate", strict, |doc, truth| {
            check_new_name(truth, name)?;
            if axes[0].component != planes[0].component || axes[1].component != planes[1].component {
                return Err(MateRefusal::invalid(
                    "a hinge's first axis and plane are on one component, and its second ones on another",
                ));
            }
            if axes[0].component == axes[1].component {
                return Err(MateRefusal::invalid("a mate ties two different components"));
            }
            let axes = [doc.endpoint(truth, axes[0])?, doc.endpoint(truth, axes[1])?];
            let planes = [doc.endpoint(truth, planes[0])?, doc.endpoint(truth, planes[1])?];
            let id = MateId(truth.take_id());
            truth.mates.push(MateEntry::Known(Mate::Hinge {
                id,
                name: name.into(),
                axes,
                planes,
                alignment,
                plane_alignment,
            }));
            Ok(id)
        })
    }

    /// Mirrors `b` of `a` about `plane`: two planes, two axes or two points, on two
    /// components, about a plane of any component. `alignment` relates two planes' normals or
    /// two axes' directions and is not stored for points. One undo step.
    pub fn mate_symmetric(
        &mut self,
        name: &str,
        [a, b, plane]: [&EndpointSelector; 3],
        alignment: Alignment,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.transact("Symmetric mate", strict, |doc, truth| {
            check_new_name(truth, name)?;
            if a.component == b.component {
                return Err(MateRefusal::invalid("a Symmetric mirrors one component's entity onto another's"));
            }
            let (a, b, plane) = (doc.endpoint(truth, a)?, doc.endpoint(truth, b)?, doc.endpoint(truth, plane)?);
            let points = matches!(a.support, Some(StoredSupport::Point { .. }))
                || matches!(&a.support, Some(StoredSupport::Enclosed { support }) if matches!(support.as_ref(), soliddraft_kernel_api::EnclosedSupport::Point { .. }));
            let id = MateId(truth.take_id());
            let alignment = (!points).then_some(alignment);
            truth.mates.push(MateEntry::Known(Mate::Symmetric { id, name: name.into(), a, b, plane, alignment }));
            Ok(id)
        })
    }

    /// Couples how far `b` slides along its plane's normal or axis's direction to how far
    /// `a` slides along its own: `a` sliding `ratio[0]` slides `b` `ratio[1]`, the other way
    /// with `reverse`. Each is measured from its reference component's origin, or the
    /// assembly's, and the parts' places now are its zero. One undo step.
    #[allow(clippy::too_many_arguments, reason = "each is one setting of the mate")]
    pub fn mate_linear_coupler(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        references: [Option<ComponentId>; 2],
        ratio: [f64; 2],
        reverse: bool,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        self.transact("Linear Coupler mate", strict, |doc, truth| {
            check_new_name(truth, name)?;
            if a.component == b.component {
                return Err(MateRefusal::invalid("a mate ties two different components"));
            }
            for (side, reference) in [a, b].into_iter().zip(references) {
                if reference == Some(side.component) {
                    return Err(MateRefusal::invalid("a part slides relative to another part, not to itself"));
                }
            }
            let ratio = check_ratio(ratio)?;
            let (ea, eb) = (doc.endpoint(truth, a)?, doc.endpoint(truth, b)?);
            let part = |c: ComponentId| -> Result<PartEndpoint, MateRefusal> {
                let source = Self::part_source(truth, c)?;
                let document = source.id.ok_or_else(|| MateRefusal::invalid("the source has no document id"))?;
                Ok(PartEndpoint { path: component_path(c), document, configuration: source.configuration })
            };
            let reference_a = references[0].map(part).transpose()?;
            let reference_b = references[1].map(part).transpose()?;
            let offset = doc.coupler_offset(truth, name, [&ea, &eb], references, ratio, reverse)?;
            let id = MateId(truth.take_id());
            truth.mates.push(MateEntry::Known(Mate::LinearCoupler {
                id,
                name: name.into(),
                a: ea,
                b: eb,
                reference_a,
                reference_b,
                ratio,
                reverse,
                offset,
            }));
            Ok(id)
        })
    }

    /// Holds the signed distance from `a`'s plane to `b`, as a Distance measures it, anywhere
    /// from `min` to `max`. One undo step.
    #[allow(clippy::too_many_arguments, reason = "each is one setting of the mate")]
    pub fn mate_limit_distance(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        [min, max]: [&str; 2],
        alignment: Alignment,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        let (min, max) = (literal(min)?, literal(max)?);
        check_limits(&min, &max)?;
        self.mate_between("Limit Distance mate", name, [a, b], strict, |id, name, [a, b]| {
            Ok(Mate::LimitDistance { id, name, a, b, alignment, min, max })
        })
    }

    /// Centres two profiles, rectangles, regular polygons or circles, on each other, their
    /// planes `distance` apart. Polygons line their edges up; with `lock_rotation` two circles
    /// hold their turn where the solve leaves it. One undo step.
    #[allow(clippy::too_many_arguments, reason = "each is one setting of the mate")]
    pub fn mate_profile_center(
        &mut self,
        name: &str,
        a: &EndpointSelector,
        b: &EndpointSelector,
        alignment: Alignment,
        distance: &str,
        lock_rotation: bool,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        let distance = literal(distance)?;
        self.transact("Profile Center mate", strict, |doc, truth| {
            check_new_name(truth, name)?;
            if a.component == b.component {
                return Err(MateRefusal::invalid("a mate ties two different components"));
            }
            let profile = Some(ShapeKind::Profile);
            let (a, b) = (doc.endpoint_as(truth, a, profile, name)?, doc.endpoint_as(truth, b, profile, name)?);
            let circles =
                [&a, &b].iter().all(|e| matches!(e.support, Some(StoredSupport::Profile { sides: None, .. })));
            if lock_rotation && !circles {
                return Err(MateRefusal::invalid(
                    "Lock rotation holds two circles' turn; polygons are held by their edges",
                ));
            }
            let id = MateId(truth.take_id());
            let paths = [a.path.clone(), b.path.clone()];
            truth.mates.push(MateEntry::Known(Mate::ProfileCenter {
                id,
                name: name.into(),
                a,
                b,
                alignment,
                distance,
                lock_rotation: None,
            }));
            if lock_rotation {
                let turn = doc.solved_turn(truth, &paths, name)?;
                if let Some(lock) = locked_turn(mate_named(truth, name)?) {
                    *lock = Some(turn);
                }
            }
            Ok(id)
        })
    }

    /// Puts the pin's axis in a straight slot: square to its run, on its mid-plane, and along
    /// it as `position` says. `alignment` relates the pin's direction to the slot's round
    /// ends' axes; with `lock_rotation` the pin's turn is held where the solve leaves it. One
    /// undo step.
    #[allow(clippy::too_many_arguments, reason = "each is one setting of the mate")]
    pub fn mate_slot(
        &mut self,
        name: &str,
        slot: &EndpointSelector,
        pin: &EndpointSelector,
        alignment: Alignment,
        position: &SlotInput,
        lock_rotation: bool,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        let constraint = match position {
            SlotInput::Free => SlotConstraint::Free,
            SlotInput::Center => SlotConstraint::Center,
            SlotInput::Distance { distance, flipped } => {
                SlotConstraint::Distance { distance: literal(distance)?, flipped: *flipped }
            }
            SlotInput::Percent { percent, flipped } => {
                SlotConstraint::Percent { percent: check_percent(*percent)?, flipped: *flipped }
            }
        };
        self.transact("Slot mate", strict, |doc, truth| {
            check_new_name(truth, name)?;
            if slot.component == pin.component {
                return Err(MateRefusal::invalid("a mate ties two different components"));
            }
            let (slot, pin) = (doc.endpoint_as(truth, slot, Some(ShapeKind::Slot), name)?, doc.endpoint(truth, pin)?);
            let id = MateId(truth.take_id());
            let paths = [slot.path.clone(), pin.path.clone()];
            truth.mates.push(MateEntry::Known(Mate::Slot {
                id,
                name: name.into(),
                slot,
                pin,
                alignment,
                constraint,
                lock_rotation: None,
            }));
            if lock_rotation {
                let turn = doc.solved_turn(truth, &paths, name)?;
                if let Some(lock) = locked_turn(mate_named(truth, name)?) {
                    *lock = Some(turn);
                }
            }
            Ok(id)
        })
    }

    /// A Linear Coupler's offset at the placements `truth` holds: `ratio[0]` times how far
    /// `b`'s plane or axis lies along its direction, less (plus, with `reverse`) `ratio[1]`
    /// times how far `a`'s does, each from its reference component's origin or the
    /// assembly's, exactly.
    fn coupler_offset(
        &mut self,
        truth: &Assembly,
        mate: &str,
        [a, b]: [&FaceEndpoint; 2],
        references: [Option<ComponentId>; 2],
        ratio: [f64; 2],
        reverse: bool,
    ) -> Result<String, MateRefusal> {
        let exact =
            |v: f64| solver::exact(v).ok_or_else(|| MateRefusal::invalid(format!("{mate}: a value is not finite")));
        let mut value = BigRational::zero();
        let ca = if reverse { exact(ratio[1])? } else { -exact(ratio[1])? };
        for ((end, reference), c) in [(b, references[1]), (a, references[0])].into_iter().zip([exact(ratio[0])?, ca]) {
            let component = end
                .component()
                .ok_or_else(|| MateRefusal::invalid(format!("{mate}: a coupler reaches the root's own parts")))?;
            let source = Self::part_source(truth, component)?;
            let read =
                read_of(mate, end.path.clone(), &self.prove_support(&source, &end.entity, end.support.as_ref())?)?;
            let at = truth.component(component).map(|c| c.placement).unwrap_or_default();
            if matches!(read.local, Local::Native(_)) {
                let from =
                    reference.and_then(|r| truth.component(r)).map_or(glam::DVec3::ZERO, |c| c.placement.translation);
                let projected = motion::projection(&read, at, from).ok_or_else(|| {
                    named(
                        MateRefusal::new(
                            RefusalCode::UnsupportedReference,
                            "a Linear Coupler reads a plane or an axis",
                        ),
                        mate,
                    )
                })?;
                value += c * exact(projected)?;
                continue;
            }
            let (direction, along) = slide_of(mate, read.local)?;
            if let Some(n) = solver::turned(at.rotation, direction) {
                let k = n.iter().position(|x| *x != 0).expect("signed axis");
                let origin = reference.and_then(|r| truth.component(r)).map_or(0.0, |c| c.placement.translation[k]);
                value += c
                    * (BigRational::from_integer(BigInt::from(n[k])) * (exact(at.translation[k])? - exact(origin)?)
                        + along);
            } else {
                let n = motion::rational_direction(at.rotation, direction);
                let origin =
                    reference.and_then(|r| truth.component(r)).map_or(glam::DVec3::ZERO, |c| c.placement.translation);
                let dot = (0..3)
                    .map(|k| Ok(n[k].clone() * (exact(at.translation[k])? - exact(origin[k])?)))
                    .collect::<Result<Vec<BigRational>, MateRefusal>>()?
                    .into_iter()
                    .sum::<BigRational>();
                value += c * (dot + along);
            }
        }
        Ok(solver::text(&value))
    }

    /// The quarter-turn rotation of the part at `paths[1]` in the frame of the part at
    /// `paths[0]`, once `truth` is solved.
    fn solved_turn(
        &mut self,
        truth: &Assembly,
        paths: &[OccurrencePath; 2],
        _mate: &str,
    ) -> Result<StoredTurn, MateRefusal> {
        let solved = self.solve_truth(truth)?;
        let scope = Scope::new(truth, &self.table, &self.store);
        let at = |p: &OccurrencePath| {
            solved
                .placements
                .get(p)
                .copied()
                .or_else(|| scope.world(p))
                .ok_or_else(|| MateRefusal::invalid(format!("no occurrence at {p}")))
        };
        let (a, b) = (at(&paths[0])?, at(&paths[1])?);
        // The product rounds even when both turns are exact, so it is read as the quarter
        // turn it lies within 1e-12 of; the lock then holds that turn exactly.
        let turn = crate::placement::canonical((a.rotation.inverse() * b.rotation).normalize());
        Ok(solver::cube_of(turn).map_or(StoredTurn::Bounded(turn.to_array()), StoredTurn::Cube))
    }

    /// Adds a mate between two faces: a distance when one is given, else a Coincident.
    fn mate_faces(
        &mut self,
        name: &str,
        [a, b]: [&EndpointSelector; 2],
        alignment: Alignment,
        distance: Option<&str>,
        strict: bool,
    ) -> Result<MateId, MateRefusal> {
        let label = if distance.is_some() { "Distance mate" } else { "Coincident mate" };
        let distance = distance.map(literal).transpose()?;
        self.mate_between(label, name, [a, b], strict, |id, name, [a, b]| {
            Ok(match distance {
                Some(distance) => Mate::Distance { id, name, a, b, alignment, distance },
                None => Mate::Coincident { id, name, a, b, alignment },
            })
        })
    }

    /// Adds the mate `make` builds between two faces of different components.
    fn mate_between(
        &mut self,
        label: &str,
        name: &str,
        [a, b]: [&EndpointSelector; 2],
        strict: bool,
        make: impl FnOnce(MateId, String, [FaceEndpoint; 2]) -> Result<Mate, MateRefusal>,
    ) -> Result<MateId, MateRefusal> {
        self.transact(label, strict, |doc, truth| {
            check_new_name(truth, name)?;
            if a.component == b.component {
                return Err(MateRefusal::invalid("a mate ties two different components"));
            }
            let (a, b) = (doc.endpoint(truth, a)?, doc.endpoint(truth, b)?);
            let id = MateId(truth.take_id());
            truth.mates.push(MateEntry::Known(make(id, name.into(), [a, b])?));
            Ok(id)
        })
    }

    /// Changes mates together, as one candidate and one undo step.
    pub fn edit_mates(&mut self, edits: &[(String, MateEdit)]) -> Result<(), MateRefusal> {
        self.transact("Edit mate", false, |doc, truth| {
            let mut locks = Vec::new();
            for (name, edit) in edits {
                let mate = mate_named(truth, name)?;
                let kind = mate.kind();
                match (mate, edit) {
                    (Mate::Angle { degrees, .. }, MateEdit::Degrees(v)) => {
                        check_degrees(*v)?;
                        *degrees = *v;
                    }
                    (Mate::LimitAngle { min_degrees, max_degrees, .. }, MateEdit::AngleLimits(v)) => {
                        check_angle_limits(*v)?;
                        *min_degrees = v[0];
                        *max_degrees = v[1];
                    }
                    (m, MateEdit::Distance(text)) if m.expression().is_some() => {
                        let distance = m.expression_mut().expect("checked");
                        if !distance.references().is_empty() {
                            let driver = distance.text().to_owned();
                            return Err(MateRefusal {
                                mates: vec![name.clone()],
                                ..MateRefusal::new(
                                    RefusalCode::DrivenInput,
                                    format!("{name} is driven by {driver}; edit {driver} instead"),
                                )
                            });
                        }
                        *distance = literal(text)?;
                    }
                    (
                        Mate::Distance { alignment, .. }
                        | Mate::Coincident { alignment, .. }
                        | Mate::Parallel { alignment, .. }
                        | Mate::Width { alignment, .. }
                        | Mate::Tangent { alignment, .. }
                        | Mate::Hinge { alignment, .. }
                        | Mate::LimitDistance { alignment, .. }
                        | Mate::ProfileCenter { alignment, .. }
                        | Mate::Slot { alignment, .. }
                        | Mate::Symmetric { alignment: Some(alignment), .. }
                        | Mate::Concentric { alignment: Some(alignment), .. },
                        MateEdit::Alignment(a),
                    ) => *alignment = *a,
                    (Mate::Hinge { plane_alignment, .. }, MateEdit::PlaneAlignment(a)) => *plane_alignment = *a,
                    (
                        Mate::Concentric { lock_rotation, .. }
                        | Mate::ProfileCenter { lock_rotation, .. }
                        | Mate::Slot { lock_rotation, .. },
                        MateEdit::LockRotation(false),
                    ) => *lock_rotation = None,
                    (Mate::Concentric { a, b, alignment: Some(_), .. }, MateEdit::LockRotation(true)) => {
                        locks.push((name.clone(), [a.path.clone(), b.path.clone()]));
                    }
                    (Mate::ProfileCenter { a, b, .. }, MateEdit::LockRotation(true)) => {
                        let circles = [&*a, &*b]
                            .iter()
                            .all(|e| matches!(e.support, Some(StoredSupport::Profile { sides: None, .. })));
                        if !circles {
                            return Err(MateRefusal::invalid(format!(
                                "{name}: Lock rotation holds two circles' turn; polygons are held by their edges"
                            )));
                        }
                        locks.push((name.clone(), [a.path.clone(), b.path.clone()]));
                    }
                    (Mate::Slot { slot, pin, .. }, MateEdit::LockRotation(true)) => {
                        locks.push((name.clone(), [slot.path.clone(), pin.path.clone()]));
                    }
                    (Mate::LimitDistance { min, .. }, MateEdit::Min(text)) => *min = literal(text)?,
                    (Mate::LimitDistance { max, .. }, MateEdit::Max(text)) => *max = literal(text)?,
                    (Mate::LinearCoupler { ratio, .. }, MateEdit::Ratio(r)) => *ratio = check_ratio(*r)?,
                    (Mate::LinearCoupler { reverse, .. }, MateEdit::Reverse(on)) => *reverse = *on,
                    (Mate::Slot { constraint: SlotConstraint::Percent { percent, .. }, .. }, MateEdit::Percent(p)) => {
                        *percent = check_percent(*p)?
                    }
                    (
                        Mate::Slot {
                            constraint:
                                SlotConstraint::Distance { flipped, .. } | SlotConstraint::Percent { flipped, .. },
                            ..
                        },
                        MateEdit::Flipped(on),
                    ) => *flipped = *on,
                    (Mate::Origin { align_axes, .. }, MateEdit::AlignAxes(on)) => *align_axes = *on,
                    (
                        Mate::Width { constraint: WidthConstraint::Percent { percent, .. }, .. },
                        MateEdit::Percent(p),
                    ) => *percent = check_percent(*p)?,
                    (
                        Mate::Width {
                            constraint:
                                WidthConstraint::Dimension { flipped, .. } | WidthConstraint::Percent { flipped, .. },
                            ..
                        },
                        MateEdit::Flipped(on),
                    ) => *flipped = *on,
                    (_, MateEdit::Distance(_)) => {
                        return Err(MateRefusal::invalid(format!("{name} is a {kind} mate; it has no distance")));
                    }
                    (_, _) => return Err(MateRefusal::invalid(format!("{name} has no such setting"))),
                }
            }
            for (name, mate) in edits {
                if let (Mate::LimitDistance { min, max, .. }, MateEdit::Min(_) | MateEdit::Max(_)) =
                    (mate_named(truth, name)?, mate)
                {
                    check_limits(min, max)?;
                }
            }
            // A coupler whose ratio or direction changed takes the parts' places as its zero.
            let mut zeroed: Vec<&String> = edits
                .iter()
                .filter(|(_, e)| matches!(e, MateEdit::Ratio(_) | MateEdit::Reverse(_)))
                .map(|(n, _)| n)
                .collect();
            zeroed.dedup();
            for name in zeroed {
                let Mate::LinearCoupler { a, b, reference_a, reference_b, ratio, reverse, .. } =
                    mate_named(truth, name)?.clone()
                else {
                    continue;
                };
                let component = |r: &Option<PartEndpoint>| r.as_ref().and_then(|r| crate::mates::holder(&r.path));
                let references = [component(&reference_a), component(&reference_b)];
                let snapshot = truth.clone();
                let value = doc.coupler_offset(&snapshot, name, [&a, &b], references, ratio, reverse)?;
                if let Mate::LinearCoupler { offset, .. } = mate_named(truth, name)? {
                    *offset = value;
                }
            }
            // A turn is locked where the other edits leave it.
            for (name, paths) in locks {
                if let Some(lock) = locked_turn(mate_named(truth, &name)?) {
                    *lock = None;
                }
                let turn = doc.solved_turn(truth, &paths, &name)?;
                if let Some(lock) = locked_turn(mate_named(truth, &name)?) {
                    *lock = Some(turn);
                }
            }
            Ok(())
        })
    }

    /// Removes a mate. One undo step; the components stay where they are.
    pub fn delete_mate(&mut self, name: &str) -> Result<(), MateRefusal> {
        self.transact("Delete mate", false, |_, truth| {
            let id = mate_named(truth, name)?.id();
            truth.mates.retain(|m| m.known().is_none_or(|m| m.id() != id));
            Ok(())
        })
    }

    /// Adds or changes an assembly value, solving everything it drives in the same step.
    pub fn set_value(&mut self, name: &str, expression: &str) -> Result<(), MateRefusal> {
        self.transact("Set value", false, |_, truth| {
            let id = truth.next_id;
            let mut taken = false;
            truth
                .values
                .set(name, expression, || {
                    taken = true;
                    id
                })
                .map_err(|e| value_refusal(name, e))?;
            if taken {
                truth.next_id += 1;
            }
            Ok(())
        })
    }

    /// Renames a value. What reads it holds its id, so only its label changes.
    pub fn rename_value(&mut self, name: &str, to: &str) -> Result<(), MateRefusal> {
        self.transact("Rename value", false, |_, truth| {
            let Assembly { values, mates, .. } = truth;
            let expressions = mates.iter_mut().filter_map(|m| match m {
                MateEntry::Known(m) => m.expression_mut(),
                MateEntry::Future(_) => None,
            });
            values.rename(name, to, expressions).map_err(|e| value_refusal(name, e))
        })
    }

    /// Binds a mate's distance to an expression over assembly values. A distance has one
    /// driver: `replace` swaps it, and without it a second driver is refused.
    pub fn drive_mate(&mut self, mate: &str, expression: &str, replace: bool) -> Result<(), MateRefusal> {
        self.transact("Drive mate", false, |_, truth| {
            let driver = truth.values.parse(expression).map_err(|e| value_refusal(mate, e))?;
            let Some(distance) = mate_named(truth, mate)?.expression_mut() else {
                return Err(MateRefusal::invalid(format!("{mate} has no distance to drive")));
            };
            if driver.references().is_empty() {
                return Err(MateRefusal::new(
                    RefusalCode::InvalidValue,
                    format!("{mate}: a driver names a value; a literal distance is an edit"),
                ));
            }
            if !distance.references().is_empty() && !replace {
                return Err(MateRefusal {
                    mates: vec![mate.to_owned()],
                    ..MateRefusal::new(
                        RefusalCode::AlreadyDriven,
                        format!("{mate} is driven by {}; a distance has one driver", distance.text()),
                    )
                });
            }
            *distance = driver;
            Ok(())
        })
    }

    /// Moves mated components: each move proposes a pose, and the mates decide what of
    /// it stands. Refused when they leave nothing of it.
    pub(crate) fn move_mated(&mut self, moves: &[(OccurrencePath, Placement)]) -> Result<(), EditError> {
        let before = self.assembly.clone();
        self.transact("Move", false, |_, truth| {
            for (path, placement) in moves {
                match Self::owner(path) {
                    Ok(c) => {
                        if let Some(component) = truth.component_mut(c) {
                            component.placement = *placement;
                        }
                    }
                    Err(p) => truth.override_mut(&p).placement = Some(*placement),
                }
            }
            Ok(())
        })
        .map_err(|r| EditError::Invalid(r.to_string()))?;
        if self.assembly == before {
            return Err(EditError::Invalid("the mates hold this component where it is".into()));
        }
        Ok(())
    }
}

fn status_of(remaining: usize, free: Vec<FreeMotion>) -> SolverStatus {
    let state = if remaining == 0 { SolverState::Solved } else { SolverState::Underconstrained };
    SolverStatus { state, remaining_dof: remaining, free, diagnostics: Vec::new(), bounded: Vec::new() }
}

/// A percentage a Width or a Slot can hold.
fn check_degrees(v: f64) -> Result<(), MateRefusal> {
    if v.is_finite() && (-180.0..=180.0).contains(&v) {
        Ok(())
    } else {
        Err(MateRefusal::new(RefusalCode::InvalidValue, "signed degrees must be finite and between -180 and 180"))
    }
}
fn check_angle_limits(v: [f64; 2]) -> Result<(), MateRefusal> {
    check_degrees(v[0])?;
    check_degrees(v[1])?;
    if v[0] <= v[1] {
        Ok(())
    } else {
        Err(MateRefusal::new(RefusalCode::InvalidValue, "least angle exceeds greatest angle"))
    }
}

fn check_percent(percent: f64) -> Result<f64, MateRefusal> {
    if (0.0..=100.0).contains(&percent) {
        Ok(percent)
    } else {
        Err(MateRefusal::new(RefusalCode::InvalidValue, format!("a percentage is from 0 to 100, not {percent}")))
    }
}

/// A Linear Coupler's ratio: two positive finite numbers.
fn check_ratio(ratio: [f64; 2]) -> Result<[f64; 2], MateRefusal> {
    if ratio.iter().all(|r| r.is_finite() && *r > 0.0) {
        Ok(ratio)
    } else {
        Err(MateRefusal::new(
            RefusalCode::InvalidValue,
            format!("a coupler's ratio is two positive numbers, not {} : {}", ratio[0], ratio[1]),
        ))
    }
}

/// A Limit Distance's least distance must not pass its greatest.
fn check_limits(min: &soliddraft_model::Expression, max: &soliddraft_model::Expression) -> Result<(), MateRefusal> {
    let none = ValueTable::default();
    let (lo, hi) = (none.length(min), none.length(max));
    match (lo, hi) {
        (Ok(lo), Ok(hi)) if lo > hi => Err(MateRefusal::new(
            RefusalCode::InvalidValue,
            format!("a limit's least distance, {}, is more than its greatest, {}", min.text(), max.text()),
        )),
        _ => Ok(()),
    }
}

/// The turn a mate with Lock rotation holds, to change.
fn locked_turn(mate: &mut Mate) -> Option<&mut Option<StoredTurn>> {
    match mate {
        Mate::Concentric { lock_rotation, .. }
        | Mate::ProfileCenter { lock_rotation, .. }
        | Mate::Slot { lock_rotation, .. } => Some(lock_rotation),
        _ => None,
    }
}

/// `b`'s placement in `a`'s frame, when both are exact quarter-turn rotations. The turn
/// between them is the product of their integer matrices, and the offset is computed
/// through `a`'s, so the lock holds what the two placements say exactly.
fn relative_lock(mate: &str, a: &Placement, b: &Placement) -> Result<Placement, MateRefusal> {
    let (Some(ra), Some(rb)) = (solver::cube_of(a.rotation), solver::cube_of(b.rotation)) else {
        return Ok(a.inverse().compose(b));
    };
    // R_a^T R_b, in integers.
    let m = |i: usize, j: usize| (0..3).map(|k| ra[k][i] * rb[k][j]).sum::<i8>();
    let col = |c: usize| glam::DVec3::new(m(0, c) as f64, m(1, c) as f64, m(2, c) as f64);
    let rotation = glam::DQuat::from_mat3(&glam::DMat3::from_cols(col(0), col(1), col(2)));
    let exact =
        |v: f64| solver::exact(v).ok_or_else(|| MateRefusal::invalid(format!("{mate}: a placement is not finite")));
    // r = R_a^T (t_b - t_a), each row of R_a^T picking one signed difference.
    let mut r = [0.0; 3];
    for (j, out) in r.iter_mut().enumerate() {
        let i = (0..3).find(|i| ra[*i][j] != 0).expect("a signed permutation");
        let d = exact(b.translation[i])? - exact(a.translation[i])?;
        *out = solver::nearest(&(BigRational::from_integer(BigInt::from(ra[i][j])) * d));
    }
    Ok(Placement { rotation, translation: glam::DVec3::from_array(r) })
}

/// What one end of a mate reads, in its part's frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Local {
    Native(Box<soliddraft_kernel_api::EnclosedSupport>),
    Plane(LocalPlane),
    Axis(LocalAxis),
    Point(LocalPoint),
    Profile(LocalProfile),
    Slot(LocalSlot),
}

/// A profile in its part's frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LocalProfile {
    pub plane: LocalPlane,
    pub centre: LocalPoint,
    /// A polygon's number of sides; `None` for a circle.
    pub sides: Option<u32>,
    /// A polygon's edge directions, when a proof read them; the status has none.
    pub edges: Option<Vec<glam::DVec3>>,
}

/// What a proven profile or slot reads.
fn read_shape(body: Body, shape: Shape) -> Read {
    let local = match shape {
        Shape::Profile(p) => Local::Profile(LocalProfile {
            plane: LocalPlane { normal: p.normal, offset: p.offset },
            centre: p.centre,
            sides: p.sides,
            edges: Some(p.edges),
        }),
        Shape::Slot(s) => Local::Slot(LocalSlot { depth: s.depth, ends: s.ends }),
    };
    Read { body, local, radius: None }
}

/// The way an end slides and how far it lies that way from its part's origin: a plane's
/// normal and offset, or an axis's direction and its point's place along it.
fn slide_of(mate: &str, local: Local) -> Result<([i8; 3], BigRational), MateRefusal> {
    match local {
        Local::Plane(p) => Ok((p.normal, p.offset)),
        Local::Axis(x) => {
            let along = (0..3).map(|k| BigRational::from_integer(BigInt::from(x.direction[k])) * &x.through[k]).sum();
            Ok((x.direction, along))
        }
        _ => Err(MateRefusal {
            mates: vec![mate.to_owned()],
            ..MateRefusal::new(
                RefusalCode::UnsupportedReference,
                format!(
                    "{mate}: a Linear Coupler reads a plane or an axis, whose normal or direction is the way its part slides"
                ),
            )
        }),
    }
}

/// A radius a curved face or circle has.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Radius {
    /// The radius when it is rational; otherwise its square, for a message.
    pub value: Result<BigRational, BigRational>,
    /// For a face, whether its outward side looks away from the axis or centre.
    pub outward: Option<bool>,
}

/// One end of a mate as a solve reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Read {
    pub body: Body,
    pub local: Local,
    pub radius: Option<Radius>,
}

/// The exact square root of a rational, when it has one.
fn rational_root(squared: &BigRational) -> Option<BigRational> {
    if squared < &BigRational::zero() {
        return None;
    }
    let (n, d) = (squared.numer().sqrt(), squared.denom().sqrt());
    (&n * &n == *squared.numer() && &d * &d == *squared.denom()).then(|| BigRational::new(n, d))
}

fn radius_of(r: &Option<ExactRadius>) -> Option<Radius> {
    let r = r.as_ref()?;
    let squared = solver::rational(&r.squared)?;
    let value = rational_root(&squared).ok_or(squared);
    Some(Radius { value, outward: r.outward })
}

/// What a proof says an end reads.
fn read_of(mate: &str, body: Body, proof: &SupportProof) -> Result<Read, MateRefusal> {
    let not_exact = || {
        MateRefusal::new(RefusalCode::UnsupportedReference, format!("{mate}: {} has no exact coordinates", proof.label))
    };
    let point = |p: &[ExactRational; 3]| -> Result<LocalPoint, MateRefusal> {
        let c = |i: usize| solver::rational(&p[i]).ok_or_else(not_exact);
        Ok([c(0)?, c(1)?, c(2)?])
    };
    Ok(match &proof.support {
        ExactSupport::Enclosed(support) => Read { body, local: Local::Native(support.clone()), radius: None },
        ExactSupport::Plane(plane) => {
            let offset = solver::rational(&plane.offset).ok_or_else(|| {
                MateRefusal::new(RefusalCode::UnsupportedPlane, format!("{mate}: the plane has no exact offset"))
            })?;
            Read { body, local: Local::Plane(LocalPlane { normal: plane.normal(), offset }), radius: None }
        }
        ExactSupport::Axis(a) => Read {
            body,
            local: Local::Axis(LocalAxis { direction: a.direction(), through: point(&a.through)? }),
            radius: radius_of(&a.radius),
        },
        ExactSupport::Point(p) => Read { body, local: Local::Point(point(&p.at)?), radius: radius_of(&p.radius) },
    })
}

/// What an end reads from what its mate stored, for the status: the stored axis or point,
/// or the stored normal of a plane with offset zero.
fn stored_read(end: &End) -> Option<Read> {
    let exact = |v: f64| solver::exact(v).unwrap_or_default();
    let radius = |r: &Option<f64>, outward: &Option<bool>| r.map(|r| Radius { value: Ok(exact(r)), outward: *outward });
    let (local, radius) = match &end.support {
        Some(StoredSupport::Enclosed { support }) => (Local::Native(support.clone()), None),
        Some(StoredSupport::Axis { direction, through, radius: r, outward }) => {
            (Local::Axis(LocalAxis { direction: *direction, through: through.map(exact) }), radius(r, outward))
        }
        Some(StoredSupport::Point { at, radius: r, outward }) => (Local::Point(at.map(exact)), radius(r, outward)),
        Some(StoredSupport::Profile { normal, centre, sides }) => {
            let plane = LocalPlane { normal: *normal, offset: BigRational::zero() };
            (Local::Profile(LocalProfile { plane, centre: centre.map(exact), sides: *sides, edges: None }), None)
        }
        Some(StoredSupport::Slot { depth, ends, .. }) => {
            (Local::Slot(LocalSlot { depth: *depth, ends: ends.map(|e| e.map(exact)) }), None)
        }
        None => {
            let normal = end.entity.stored_normal()?;
            (Local::Plane(LocalPlane { normal, offset: BigRational::zero() }), None)
        }
    };
    Some(Read { body: end.body.clone(), local, radius })
}

/// The solver's constraints for a mate between faces, edges or vertices, from what its
/// ends read. A Hinge gives two.
fn constraint(mate: String, reads: Vec<Read>, rule: &Rule) -> Result<Vec<Constraint>, MateRefusal> {
    let named = |code: RefusalCode, message: &str| MateRefusal {
        mates: vec![mate.clone()],
        ..MateRefusal::new(code, format!("{mate}: {message}"))
    };
    let unsupported = |message: &str| named(RefusalCode::UnsupportedReference, message);
    let exact =
        |d: f64| solver::exact(d).ok_or_else(|| MateRefusal::invalid(format!("{mate}: the distance is not finite")));
    let nonparallel_width = matches!(rule, Rule::Width { .. })
        && reads.as_chunks::<2>().0.iter().any(|pair| match (&pair[0].local, &pair[1].local) {
            (Local::Plane(p), Local::Plane(q)) => p.normal.iter().zip(q.normal).any(|(x, y)| x.abs() != y.abs()),
            _ => false,
        });
    if nonparallel_width || reads.iter().any(|r| matches!(r.local, Local::Native(_))) {
        return motion::native_constraint(mate, reads, rule);
    }
    let mut reads = reads.into_iter();
    let first = reads.next().expect("two ends");
    let second = reads.next().expect("two ends");
    let (a, b) = (first.body, second.body);
    let out = match rule {
        Rule::Angle { min, max, limit } => {
            check_angle_limits([*min, *max])?;
            let third = reads.next().expect("reference direction");
            let dir = |l: Local| match l {
                Local::Plane(p) => Ok(p.normal),
                Local::Axis(x) => Ok(x.direction),
                _ => Err(unsupported(
                    "an Angle reads plane normals or axis directions, including its explicit reference",
                )),
            };
            let directions = [dir(first.local)?, dir(second.local)?, dir(third.local)?];
            vec![Constraint::Motion {
                mate,
                relation: Box::new(Relation::Angle {
                    a,
                    b,
                    reference: third.body,
                    directions,
                    min: *min,
                    max: *max,
                    limit: *limit,
                }),
            }]
        }
        Rule::Parallel { sign, distance, coincident } => {
            let at = |d: f64| if *coincident { "in the plane".to_owned() } else { format!("{d} mm from the plane") };
            match (first.local, second.local, distance) {
                (Local::Plane(plane_a), Local::Plane(plane_b), _) => {
                    let along = distance.map(Along::Distance);
                    vec![Constraint::Parallel { mate, a, b, plane_a, plane_b, sign: *sign, along }]
                }
                (Local::Axis(axis_a), Local::Axis(axis_b), None) => {
                    vec![Constraint::Axes { mate, a, b, axis_a, axis_b, sign: *sign, coaxial: false }]
                }
                (Local::Plane(plane), Local::Axis(axis), Some(d)) => {
                    let (gap, says) = (exact(*d)?, at(*d));
                    let (point, direction) = (axis.through, Some(axis.direction));
                    vec![Constraint::FromPlane { mate, a, plane, b, point, direction, gap, upto: None, says }]
                }
                (Local::Axis(axis), Local::Plane(plane), Some(d)) => {
                    let (gap, says) = (exact(*d)?, at(*d));
                    let (point, direction) = (axis.through, Some(axis.direction));
                    vec![Constraint::FromPlane { mate, a: b, plane, b: a, point, direction, gap, upto: None, says }]
                }
                (Local::Plane(plane), Local::Point(point), Some(d)) => {
                    let (gap, says) = (exact(*d)?, at(*d));
                    vec![Constraint::FromPlane { mate, a, plane, b, point, direction: None, gap, upto: None, says }]
                }
                (Local::Point(point), Local::Plane(plane), Some(d)) => {
                    let (gap, says) = (exact(*d)?, at(*d));
                    vec![Constraint::FromPlane {
                        mate,
                        a: b,
                        plane,
                        b: a,
                        point,
                        direction: None,
                        gap,
                        upto: None,
                        says,
                    }]
                }
                (Local::Axis(axis_a), Local::Axis(axis_b), Some(_)) if *coincident => {
                    vec![Constraint::Axes { mate, a, b, axis_a, axis_b, sign: *sign, coaxial: true }]
                }
                (Local::Axis(axis), Local::Point(point), Some(_)) if *coincident => {
                    vec![Constraint::OnAxis { mate, a, axis, b, point }]
                }
                (Local::Point(point), Local::Axis(axis), Some(_)) if *coincident => {
                    vec![Constraint::OnAxis { mate, a: b, axis, b: a, point }]
                }
                (Local::Point(point_a), Local::Point(point_b), Some(_)) if *coincident => {
                    vec![Constraint::Points { mate, a, point_a, b, point_b }]
                }
                (x, y, Some(d)) => {
                    if *d < 0.0 {
                        return Err(named(
                            RefusalCode::InvalidValue,
                            "a point-point or axis-axis distance is nonnegative",
                        ));
                    }
                    let end = |l| match l {
                        Local::Point(p) => Ok(DistanceEntity::Point(p)),
                        Local::Axis(x) => Ok(DistanceEntity::Axis(x)),
                        _ => Err(unsupported(
                            "Distance takes two planes, a plane and a point or axis, two points, or two axes",
                        )),
                    };
                    let ends = [end(x)?, end(y)?];
                    if !matches!(
                        &ends,
                        [DistanceEntity::Point(_), DistanceEntity::Point(_)]
                            | [DistanceEntity::Axis(_), DistanceEntity::Axis(_)]
                    ) {
                        return Err(unsupported("Distance takes two points or two axes"));
                    }
                    let distance = exact(*d)?;
                    vec![Constraint::Motion {
                        mate,
                        relation: Box::new(Relation::Separation {
                            a,
                            b,
                            ends: Box::new(ends),
                            distance,
                            parallel: Some(*sign),
                        }),
                    }]
                }
                (_, _, None) => return Err(unsupported("Parallel takes two planes or two axes")),
            }
        }
        Rule::Perpendicular => match (first.local, second.local) {
            (Local::Plane(plane_a), Local::Plane(plane_b)) => {
                vec![Constraint::Perpendicular { mate, a, b, plane_a, plane_b }]
            }
            (Local::Axis(axis_a), Local::Axis(axis_b)) => {
                vec![Constraint::Axes { mate, a, b, axis_a, axis_b, sign: 0, coaxial: false }]
            }
            _ => return Err(unsupported("Perpendicular takes two planes or two axes")),
        },
        Rule::Width { sign, mode } => {
            let plane = |l: Local| match l {
                Local::Plane(p) => Ok(p),
                _ => Err(named(RefusalCode::UnsupportedPlane, "a Width reads four planar faces")),
            };
            let (plane_a, groove) = (plane(first.local)?, plane(second.local)?);
            let (t, u) = (reads.next().expect("four planes"), reads.next().expect("four planes"));
            let (tab_body, tab, other_body, other_tab) = (t.body, plane(t.local)?, u.body, plane(u.local)?);
            if b != a || other_body != tab_body || tab_body == a {
                let message = format!("{mate}: a groove's faces are on one part and a tab's on another");
                return Err(MateRefusal { mates: vec![mate], ..MateRefusal::invalid(message) });
            }
            let along = Some(Along::Width { groove, tab: other_tab, mode: mode.clone() });
            vec![Constraint::Parallel { mate, a, b: tab_body, plane_a, plane_b: tab, sign: *sign, along }]
        }
        Rule::Concentric { sign, lock } => {
            let m = mate.clone();
            let mut out = match (first.local, second.local) {
                (Local::Axis(axis_a), Local::Axis(axis_b)) => {
                    let sign =
                        sign.ok_or_else(|| MateRefusal::invalid(format!("{mate}: two axes need an alignment")))?;
                    let (a, b) = (a.clone(), b.clone());
                    vec![Constraint::Axes { mate: m, a, b, axis_a, axis_b, sign, coaxial: true }]
                }
                (Local::Axis(axis), Local::Point(point)) => {
                    vec![Constraint::OnAxis { mate: m, a: a.clone(), axis, b: b.clone(), point }]
                }
                (Local::Point(point), Local::Axis(axis)) => {
                    vec![Constraint::OnAxis { mate: m, a: b.clone(), axis, b: a.clone(), point }]
                }
                (Local::Point(point_a), Local::Point(point_b)) => {
                    vec![Constraint::Points { mate: m, a: a.clone(), point_a, b: b.clone(), point_b }]
                }
                _ => {
                    return Err(unsupported(
                        "Concentric takes cylinders, cones, spheres, circles, lines and points, not planes",
                    ));
                }
            };
            if let Some(turn) = lock {
                if !matches!(out[0], Constraint::Axes { .. }) {
                    return Err(MateRefusal::invalid(format!("{mate}: Lock rotation holds the turn between two axes")));
                }
                out.push(Constraint::Turn { mate, a, b, relative: turn.rotation()? });
            }
            out
        }
        Rule::Tangent { sign } => {
            let Local::Plane(plane) = first.local else {
                let radius = |r: Option<Radius>| -> Result<(BigRational, bool), MateRefusal> {
                    let Some(Radius { value: Ok(r), outward: Some(outward) }) = r else {
                        return Err(unsupported(
                            "curved Tangent requires two cylindrical or spherical faces with rational radii",
                        ));
                    };
                    Ok((r, outward))
                };
                let (ra, oa) = radius(first.radius)?;
                let (rb, ob) = radius(second.radius)?;
                let external = (oa == ob) == (*sign < 0);
                if !external && ra == rb {
                    return Err(unsupported("equal-radius internal Tangent is degenerate; use Concentric"));
                }
                let distance = if external { ra + rb } else { (ra - rb).abs() };
                let end = |l| match l {
                    Local::Axis(x) => Ok(DistanceEntity::Axis(x)),
                    Local::Point(p) => Ok(DistanceEntity::Point(p)),
                    _ => Err(unsupported("curved Tangent requires two cylinders or spheres")),
                };
                let ends = [end(first.local)?, end(second.local)?];
                return Ok(vec![Constraint::Motion {
                    mate,
                    relation: Box::new(Relation::Separation { a, b, ends: Box::new(ends), distance, parallel: None }),
                }]);
            };
            let Some(Radius { value, outward: Some(outward) }) = second.radius else {
                return Err(unsupported("a Tangent's second end is a cylinder or a sphere"));
            };
            let r = value.map_err(|squared| {
                unsupported(&format!("its radius is not rational: the radius squared is {}", solver::text(&squared)))
            })?;
            // The axis or centre sits one radius in front of the plane, or behind it: in
            // front when a shaft's outward side meets the plane's opposed, or a bore's
            // aligned.
            let gap = if outward == (*sign < 0) { r } else { -r };
            let says = "tangent".to_owned();
            match second.local {
                Local::Axis(axis) => {
                    let (point, direction) = (axis.through, Some(axis.direction));
                    vec![Constraint::FromPlane { mate, a, plane, b, point, direction, gap, upto: None, says }]
                }
                Local::Point(point) => {
                    vec![Constraint::FromPlane { mate, a, plane, b, point, direction: None, gap, upto: None, says }]
                }
                Local::Native(_) | Local::Plane(_) | Local::Profile(_) | Local::Slot(_) => {
                    return Err(unsupported("a Tangent's second end is a cylinder or a sphere"));
                }
            }
        }
        Rule::Hinge { sign, plane_sign } => {
            let (Local::Axis(axis_a), Local::Axis(axis_b)) = (first.local, second.local) else {
                return Err(unsupported("a Hinge's first two ends are axes"));
            };
            let (p, q) = (reads.next().expect("four ends"), reads.next().expect("four ends"));
            let (Local::Plane(plane_a), Local::Plane(plane_b)) = (p.local, q.local) else {
                return Err(unsupported("a Hinge's last two ends are planes"));
            };
            vec![
                Constraint::Axes { mate: mate.clone(), a, b, axis_a, axis_b, sign: *sign, coaxial: true },
                Constraint::Parallel {
                    mate,
                    a: p.body,
                    b: q.body,
                    plane_a,
                    plane_b,
                    sign: *plane_sign,
                    along: Some(Along::Distance(0.0)),
                },
            ]
        }
        Rule::Symmetric { sign } => {
            let third = reads.next().expect("three ends");
            let Local::Plane(plane) = third.local else {
                return Err(unsupported(
                    "a Symmetric's third end is the symmetry plane: a planar face or a reference plane",
                ));
            };
            if let (Some(ra), Some(rb)) = (&first.radius, &second.radius)
                && ra.value != rb.value
            {
                return Err(unsupported("the two have different radii; a Symmetric mirrors equal ones"));
            }
            let directed =
                || sign.ok_or_else(|| MateRefusal::invalid(format!("{mate}: two planes or axes need an alignment")));
            let (ends, sign) = match (first.local, second.local) {
                (Local::Plane(p), Local::Plane(q)) => (Mirrored::Planes(p, q), directed()?),
                (Local::Axis(p), Local::Axis(q)) => (Mirrored::Axes(p, q), directed()?),
                (Local::Point(p), Local::Point(q)) => (Mirrored::Points(p, q), 1),
                _ => return Err(unsupported("Symmetric mirrors two planes, two axes or two points, of one kind")),
            };
            vec![Constraint::Symmetric { mate, a, b, s: third.body, plane, ends, sign }]
        }
        Rule::Coupler { ratio, reverse, offset, references, frame } => {
            let slide = |body: Body, local: Local, from: &Option<Body>| -> Result<Slide, MateRefusal> {
                let (direction, offset) = slide_of(&mate, local)?;
                Ok(Slide { body, direction, offset, from: from.clone(), frame: frame.clone() })
            };
            let (sa, sb) = (slide(a, first.local, &references[0])?, slide(b, second.local, &references[1])?);
            vec![Constraint::Coupler {
                mate,
                a: sa,
                b: sb,
                ratio: ratio.clone(),
                reverse: *reverse,
                offset: offset.clone(),
            }]
        }
        Rule::Limit { sign, min, max } => {
            let (gap, upto) = (exact(*min)?, Some(exact(*max)?));
            let says = format!("from {min} mm to {max} mm");
            match (first.local, second.local) {
                (Local::Plane(plane_a), Local::Plane(plane_b)) => {
                    let along = Some(Along::Range(*min, *max));
                    vec![Constraint::Parallel { mate, a, b, plane_a, plane_b, sign: *sign, along }]
                }
                (Local::Plane(plane), Local::Axis(axis)) => {
                    let (point, direction) = (axis.through, Some(axis.direction));
                    vec![Constraint::FromPlane { mate, a, plane, b, point, direction, gap, upto, says }]
                }
                (Local::Axis(axis), Local::Plane(plane)) => {
                    let (point, direction) = (axis.through, Some(axis.direction));
                    vec![Constraint::FromPlane { mate, a: b, plane, b: a, point, direction, gap, upto, says }]
                }
                (Local::Plane(plane), Local::Point(point)) => {
                    vec![Constraint::FromPlane { mate, a, plane, b, point, direction: None, gap, upto, says }]
                }
                (Local::Point(point), Local::Plane(plane)) => {
                    vec![Constraint::FromPlane { mate, a: b, plane, b: a, point, direction: None, gap, upto, says }]
                }
                _ => {
                    return Err(unsupported(
                        "a Limit Distance measures from a plane to a plane, an axis or a point; a length across two axes or points is not solved exactly in this version",
                    ));
                }
            }
        }
        Rule::Profile { sign, distance, lock } => {
            let (Local::Profile(pa), Local::Profile(pb)) = (first.local, second.local) else {
                return Err(unsupported("Profile Center reads two profiles: planar faces or circular edges"));
            };
            let kind = |sides: Option<u32>| match sides {
                None => "a circle".to_owned(),
                Some(4) => "a rectangle".to_owned(),
                Some(n) => format!("a {n}-sided polygon"),
            };
            if pa.sides != pb.sides {
                return Err(unsupported(&format!(
                    "Profile Center centres two profiles of one kind, not {} and {}",
                    kind(pa.sides),
                    kind(pb.sides)
                )));
            }
            let axis = LocalAxis { direction: pa.plane.normal, through: pa.centre };
            let normal = pa.plane.normal;
            let mut out = vec![
                Constraint::Parallel {
                    mate: mate.clone(),
                    a: a.clone(),
                    b: b.clone(),
                    plane_a: pa.plane,
                    plane_b: pb.plane,
                    sign: *sign,
                    along: Some(Along::Distance(*distance)),
                },
                Constraint::OnAxis { mate: mate.clone(), a: a.clone(), axis, b: b.clone(), point: pb.centre },
            ];
            match (pa.sides, lock) {
                (Some(_), Some(_)) => {
                    return Err(MateRefusal::invalid(format!(
                        "{mate}: Lock rotation holds two circles' turn; polygons are held by their edges"
                    )));
                }
                (Some(_), None) => {
                    let edges = pa.edges.zip(pb.edges);
                    out.push(Constraint::Spin { mate, a, b, normal, edges });
                }
                (None, Some(turn)) => {
                    out.push(Constraint::Turn { mate, a, b, relative: turn.rotation()? });
                }
                (None, None) => {}
            }
            out
        }
        Rule::Slot { sign, mode, lock } => {
            let Local::Slot(slot) = first.local else {
                return Err(unsupported("a Slot's first end is the slot"));
            };
            let axis = match second.local {
                Local::Axis(axis) => axis,
                Local::Slot(_) => {
                    return Err(unsupported("a slot in a slot is not supported in this version; mate a pin's axis"));
                }
                _ => {
                    return Err(unsupported(
                        "a Slot's second end is the pin's axis: a cylinder, or a circular or straight edge",
                    ));
                }
            };
            let mut out = vec![Constraint::Slot {
                mate: mate.clone(),
                a: a.clone(),
                slot,
                b: b.clone(),
                axis,
                sign: *sign,
                mode: mode.clone(),
            }];
            if let Some(turn) = lock {
                out.push(Constraint::Turn { mate, a, b, relative: turn.rotation()? });
            }
            out
        }
    };
    Ok(out)
}

fn signed_gap(a: &PlaneProof, b: &PlaneProof, placements: &[Placement; 2]) -> Result<BigRational, MateRefusal> {
    let off = |_| MateRefusal::new(RefusalCode::UnsupportedPlacement, "a face is placed off the world axes");
    // Only a placement that takes the normal exactly onto an axis is proved: one that
    // lands within a tolerance of it is tilted, and far out the tilt is a real gap.
    let normal = |p: &PlaneProof, at: &Placement| {
        solver::turned(at.rotation, p.plane.normal()).map(|n| n.map(f64::from)).ok_or(())
    };
    let (na, nb) = (normal(a, &placements[0]).map_err(off)?, normal(b, &placements[1]).map_err(off)?);
    let axis = na.iter().position(|c| *c != 0.0).expect("a unit axis");
    if nb[axis] == 0.0 {
        return Err(MateRefusal::new(RefusalCode::NotParallel, "the two faces' planes are not parallel"));
    }
    let whole = |v: f64| BigRational::from_integer(BigInt::from(v as i64));
    let side = whole(na[axis]);
    let relation = whole(if na[axis] == nb[axis] { 1.0 } else { -1.0 });
    let exact = |v: f64| solver::exact(v).ok_or_else(|| MateRefusal::invalid("a placement is not finite"));
    let (ta, tb) = (exact(placements[0].translation[axis])?, exact(placements[1].translation[axis])?);
    let offset = |p: &PlaneProof| {
        solver::rational(&p.plane.offset)
            .ok_or_else(|| MateRefusal::new(RefusalCode::UnsupportedPlane, "the plane has no exact offset"))
    };
    Ok(side * (tb - ta) + relation * offset(b)? - offset(a)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Step;

    fn q(n: i64, d: i64) -> BigRational {
        BigRational::new(BigInt::from(n), BigInt::from(d))
    }

    fn body(id: u32) -> Body {
        OccurrencePath::root().child(Step::component(ComponentId(id)))
    }

    fn plane(normal: [i8; 3], offset: i64) -> Read {
        Read { body: body(2), local: Local::Plane(LocalPlane { normal, offset: q(offset, 1) }), radius: None }
    }

    fn shaft(squared: BigRational, outward: Option<bool>) -> Read {
        let value = rational_root(&squared).ok_or(squared);
        Read {
            body: body(4),
            local: Local::Axis(LocalAxis { direction: [0, 0, 1], through: [q(0, 1), q(0, 1), q(0, 1)] }),
            radius: Some(Radius { value, outward }),
        }
    }

    fn gap(c: &[Constraint]) -> BigRational {
        let [Constraint::FromPlane { gap, .. }] = c else { panic!("one constraint from the plane: {c:?}") };
        gap.clone()
    }

    #[test]
    fn rational_roots_are_exact_or_absent() {
        assert_eq!(rational_root(&q(16, 9)), Some(q(4, 3)));
        assert_eq!(rational_root(&q(0, 1)), Some(q(0, 1)));
        assert_eq!(rational_root(&q(2, 1)), None);
        assert_eq!(rational_root(&q(-4, 1)), None);
    }

    /// A shaft lies in front of a plane it meets opposed, a bore's wall behind it, and the
    /// other way round when aligned.
    #[test]
    fn a_tangent_puts_the_axis_one_radius_from_the_plane_on_the_right_side() {
        let tangent = |outward: bool, sign: i8| {
            let reads = vec![plane([0, 0, 1], 10), shaft(q(16, 1), Some(outward))];
            gap(&constraint("T".into(), reads, &Rule::Tangent { sign }).unwrap())
        };
        assert_eq!(tangent(true, -1), q(4, 1));
        assert_eq!(tangent(true, 1), q(-4, 1));
        assert_eq!(tangent(false, -1), q(-4, 1));
        assert_eq!(tangent(false, 1), q(4, 1));
    }

    #[test]
    fn a_tangent_refuses_an_irrational_radius_and_an_edge_by_name() {
        let reads = vec![plane([0, 0, 1], 10), shaft(q(2, 1), Some(true))];
        let refused = constraint("T".into(), reads, &Rule::Tangent { sign: -1 }).unwrap_err();
        assert_eq!((refused.code, refused.mates.clone()), (RefusalCode::UnsupportedReference, vec!["T".to_owned()]));
        assert!(refused.message.contains("radius squared is 2"), "{}", refused.message);
        let reads = vec![plane([0, 0, 1], 10), shaft(q(16, 1), None)];
        let refused = constraint("T".into(), reads, &Rule::Tangent { sign: -1 }).unwrap_err();
        assert_eq!(refused.code, RefusalCode::UnsupportedReference);
    }

    #[test]
    fn a_distance_across_two_axes_is_bounded_and_a_coincident_one_is_coaxial() {
        let axes = || vec![shaft(q(16, 1), Some(true)), Read { body: body(2), ..shaft(q(16, 1), Some(false)) }];
        let distance = Rule::Parallel { sign: 1, distance: Some(5.0), coincident: false };
        let made = constraint("D".into(), axes(), &distance).unwrap();
        assert!(matches!(made[..], [Constraint::Motion { .. }]));
        let coincident = Rule::Parallel { sign: 1, distance: Some(0.0), coincident: true };
        let made = constraint("C".into(), axes(), &coincident).unwrap();
        assert!(matches!(made[..], [Constraint::Axes { coaxial: true, sign: 1, .. }]));
    }

    #[test]
    fn a_plane_and_an_axis_read_from_either_end_measure_from_the_plane() {
        let rule = Rule::Parallel { sign: 1, distance: Some(3.0), coincident: false };
        for reads in [
            vec![plane([0, 0, 1], 10), shaft(q(16, 1), Some(true))],
            vec![shaft(q(16, 1), Some(true)), plane([0, 0, 1], 10)],
        ] {
            let made = constraint("D".into(), reads, &rule).unwrap();
            let [Constraint::FromPlane { a, b, gap, direction, .. }] = &made[..] else { panic!("{made:?}") };
            assert_eq!((a, b, gap, direction), (&body(2), &body(4), &q(3, 1), &Some([0, 0, 1])));
        }
    }

    #[test]
    fn concentric_takes_no_planes_and_validates_saved_turns() {
        let rule = Rule::Concentric { sign: Some(1), lock: None };
        let refused = constraint("C".into(), vec![plane([0, 0, 1], 0), shaft(q(1, 1), Some(true))], &rule).unwrap_err();
        assert_eq!(refused.code, RefusalCode::UnsupportedReference);
        let axes = || vec![Read { body: body(2), ..shaft(q(16, 1), Some(false)) }, shaft(q(16, 1), Some(true))];
        let skew = Rule::Concentric { sign: Some(1), lock: Some(StoredTurn::Cube([[1, 1, 0], [0, 1, 0], [0, 0, 1]])) };
        assert_eq!(constraint("C".into(), axes(), &skew).unwrap_err().code, RefusalCode::Invalid);
        let quarter =
            Rule::Concentric { sign: Some(1), lock: Some(StoredTurn::Cube([[0, -1, 0], [1, 0, 0], [0, 0, 1]])) };
        let made = constraint("C".into(), axes(), &quarter).unwrap();
        assert!(matches!(made[..], [Constraint::Axes { .. }, Constraint::Turn { .. }]));
    }
}
