//! Mate commands. Every one runs the same way: copy the truth, change the copy, prove and
//! solve it, and only then publish truth, pose and undo as one step. A refusal changes
//! nothing, and Update solves its candidate through the same [`AssemblyDocument::solve_truth`].

use std::collections::BTreeMap;

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::Zero;
use serde::Serialize;
use soliddraft_document::{ContentHash, DocRef, DocumentId};
use soliddraft_kernel_api::{Certainty, ExactRadius, ExactRational, ExactSupport};
use soliddraft_model::ValueTable;

use crate::document::{AssemblyDocument, EditError, key_of};
use crate::mates::bodies::{End, Gathered, Problem, Rule, Scope};
use crate::mates::reference::check_endpoint;
use crate::mates::solver::{
    self, Along, Body, Constraint, FreeMotion, LocalAxis, LocalPlane, LocalPoint, SolverState, SolverStatus,
};
use crate::mates::{
    Alignment, EndpointSelector, FaceEndpoint, Mate, MateEntry, MateId, MateRefusal, PartEndpoint, PlaneProof,
    RefusalCode, StoredSupport, SupportProof, WidthConstraint,
};
use crate::path::{ComponentId, OccurrencePath, Step};
use crate::placement::Placement;
use crate::schema::Assembly;
use crate::store::Freshness;

/// One change to a mate.
#[derive(Clone, Debug, PartialEq)]
pub enum MateEdit {
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
    /// Whether a Concentric holds the turn about its axis where it is.
    LockRotation(bool),
}

/// Where a new Width mate puts the tab.
#[derive(Clone, Debug, PartialEq)]
pub enum WidthInput {
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
            let support = self.prove_support(&source, &end.entity, end.support.as_ref())?.stored();
            if let MateEntry::Known(m) = &mut candidate.assembly.mates[i] {
                m.faces_mut()[j].support = support;
            }
        }
        Ok(())
    }

    /// Proves the faces `problem` reads and solves it.
    pub(crate) fn solve_problem(&mut self, problem: Problem) -> Result<Solved, MateRefusal> {
        let mut constraints = Vec::new();
        let mut dangling: Vec<(String, String)> = Vec::new();
        for mate in problem.mates {
            match mate {
                Gathered::Origin { mate, body, axes, at } => {
                    constraints.push(Constraint::Origin { mate, body, axes, at })
                }
                Gathered::Lock { mate, a, b, relative } => {
                    if solver::cube_of(relative.rotation).is_none() {
                        return Err(off_quarter_turns(&mate));
                    }
                    constraints.push(Constraint::Lock { mate, a, b, relative })
                }
                Gathered::Between { mate, ends, rule } => {
                    let mut planes = Vec::new();
                    for end in &ends {
                        let source = self.rebased(end);
                        match self.prove_support(&source, &end.entity, end.support.as_ref()) {
                            Ok(proof) => planes.push(read_of(&mate, end.body.clone(), &proof)?),
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
        let placements = solver::solve(&problem.bodies, &constraints)?;
        let (dof, free) = solver::freedoms(&placements, &constraints)?;
        let unmated = problem.parts.saturating_sub(problem.bodies.len());
        let moved = placements.into_iter().filter(|(b, p)| problem.bodies[b] != *p).collect();
        Ok(Solved { placements: moved, status: status_of(dof + 6 * unmated, free) })
    }

    /// The accepted assembly's status, from its definitions: reads no part and proves nothing.
    pub fn solver(&mut self) -> SolverStatus {
        if let Some(s) = &self.solver_status {
            return s.clone();
        }
        let status = self.read_status().unwrap_or_else(SolverStatus::error);
        self.solver_status = Some(status.clone());
        status
    }

    fn read_status(&self) -> Result<SolverStatus, String> {
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
        let (dof, free) = solver::freedoms(&problem.bodies, &constraints).map_err(|r| r.to_string())?;
        Ok(status_of(dof + 6 * problem.parts.saturating_sub(problem.bodies.len()), free))
    }

    /// How many parts were rebuilt so far to prove planes.
    pub fn parts_rebuilt_for_proofs(&self) -> usize {
        self.proofs.built
    }

    fn endpoint(&mut self, truth: &Assembly, selector: &EndpointSelector) -> Result<FaceEndpoint, MateRefusal> {
        let source = Self::part_source(truth, selector.component)?;
        let entity = self.select(&source, &selector.face)?;
        let document = source.id.ok_or_else(|| MateRefusal::invalid("the source has no document id"))?;
        let support = self.stored_support(&source, &entity)?;
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
        let gap = signed_gap(&pa, &pb, &placements)?;
        let value = solver::nearest(&gap);
        let (mut lo, mut hi) = (value, value);
        if solver::exact(lo).is_some_and(|r| r > gap) {
            lo = lo.next_down();
        }
        if solver::exact(hi).is_some_and(|r| r < gap) {
            hi = hi.next_up();
        }
        Ok(PlaneMeasurement {
            quantity: "signed_support_plane_distance",
            unit: "mm",
            frame: "root",
            a: self.identity(&a.path, &ra, &pa)?,
            b: self.identity(&b.path, &rb, &pb)?,
            placements,
            certainty: Certainty::Exact,
            value,
            lo,
            hi,
            rational: solver::text(&gap),
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
            self.prove_support(&source, &end.entity, end.support.as_ref())?;
        }
        Ok(())
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

    /// Locks `b` where it is relative to `a`. One undo step. The relative turn must be a
    /// quarter-turn rotation; anything else is refused, never rounded.
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
            let axes = matches!(
                (&a.support, &b.support),
                (Some(StoredSupport::Axis { .. }), Some(StoredSupport::Axis { .. }))
            );
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

    /// The quarter-turn rotation of the part at `paths[1]` in the frame of the part at
    /// `paths[0]`, once `truth` is solved.
    fn solved_turn(
        &mut self,
        truth: &Assembly,
        paths: &[OccurrencePath; 2],
        mate: &str,
    ) -> Result<solver::Cube, MateRefusal> {
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
        solver::cube_of(turn).ok_or_else(|| off_quarter_turns(mate))
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
                        | Mate::Concentric { alignment: Some(alignment), .. },
                        MateEdit::Alignment(a),
                    ) => *alignment = *a,
                    (Mate::Hinge { plane_alignment, .. }, MateEdit::PlaneAlignment(a)) => *plane_alignment = *a,
                    (Mate::Concentric { lock_rotation, .. }, MateEdit::LockRotation(false)) => *lock_rotation = None,
                    (Mate::Concentric { a, b, alignment: Some(_), .. }, MateEdit::LockRotation(true)) => {
                        locks.push((name.clone(), [a.path.clone(), b.path.clone()]));
                    }
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
            // A turn is locked where the other edits leave it.
            for (name, paths) in locks {
                if let Mate::Concentric { lock_rotation, .. } = mate_named(truth, &name)? {
                    *lock_rotation = None;
                }
                let turn = doc.solved_turn(truth, &paths, &name)?;
                if let Mate::Concentric { lock_rotation, .. } = mate_named(truth, &name)? {
                    *lock_rotation = Some(turn);
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
    SolverStatus { state, remaining_dof: remaining, free, diagnostics: Vec::new() }
}

/// A percentage a Width can hold.
fn check_percent(percent: f64) -> Result<f64, MateRefusal> {
    if (0.0..=100.0).contains(&percent) {
        Ok(percent)
    } else {
        Err(MateRefusal::new(
            RefusalCode::InvalidValue,
            format!("a width's percentage is from 0 to 100, not {percent}"),
        ))
    }
}

fn off_quarter_turns(mate: &str) -> MateRefusal {
    let message = format!(
        "{mate}: the parts are turned off the quarter turns from each other; this version locks quarter-turn relative rotations only"
    );
    MateRefusal { mates: vec![mate.to_owned()], ..MateRefusal::new(RefusalCode::UnsupportedPlacement, message) }
}

/// `b`'s placement in `a`'s frame, when both are exact quarter-turn rotations. The turn
/// between them is the product of their integer matrices, and the offset is computed
/// through `a`'s, so the lock holds what the two placements say exactly.
fn relative_lock(mate: &str, a: &Placement, b: &Placement) -> Result<Placement, MateRefusal> {
    let ra = solver::cube_of(a.rotation).ok_or_else(|| solver::unsupported_lock(mate))?;
    let rb = solver::cube_of(b.rotation).ok_or_else(|| off_quarter_turns(mate))?;
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
    Plane(LocalPlane),
    Axis(LocalAxis),
    Point(LocalPoint),
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
        Some(StoredSupport::Axis { direction, through, radius: r, outward }) => {
            (Local::Axis(LocalAxis { direction: *direction, through: through.map(exact) }), radius(r, outward))
        }
        Some(StoredSupport::Point { at, radius: r, outward }) => (Local::Point(at.map(exact)), radius(r, outward)),
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
    let mut reads = reads.into_iter();
    let first = reads.next().expect("two ends");
    let second = reads.next().expect("two ends");
    let (a, b) = (first.body, second.body);
    let out = match rule {
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
                    vec![Constraint::FromPlane { mate, a, plane, b, point, direction, gap, says }]
                }
                (Local::Axis(axis), Local::Plane(plane), Some(d)) => {
                    let (gap, says) = (exact(*d)?, at(*d));
                    let (point, direction) = (axis.through, Some(axis.direction));
                    vec![Constraint::FromPlane { mate, a: b, plane, b: a, point, direction, gap, says }]
                }
                (Local::Plane(plane), Local::Point(point), Some(d)) => {
                    let (gap, says) = (exact(*d)?, at(*d));
                    vec![Constraint::FromPlane { mate, a, plane, b, point, direction: None, gap, says }]
                }
                (Local::Point(point), Local::Plane(plane), Some(d)) => {
                    let (gap, says) = (exact(*d)?, at(*d));
                    vec![Constraint::FromPlane { mate, a: b, plane, b: a, point, direction: None, gap, says }]
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
                (_, _, Some(_)) => {
                    return Err(unsupported(
                        "a distance between two axes or points is a length across them, which this version does not solve exactly",
                    ));
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
            let parallel =
                |p: &LocalPlane, q: &LocalPlane| p.normal.iter().zip(q.normal).all(|(x, y)| x.abs() == y.abs());
            if !parallel(&plane_a, &groove) || !parallel(&tab, &other_tab) {
                return Err(named(RefusalCode::NotParallel, "a groove's faces, and a tab's, are parallel planes"));
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
                if !solver::cube_rotations().contains(turn) {
                    return Err(off_quarter_turns(&mate));
                }
                out.push(Constraint::Turn { mate, a, b, relative: solver::quaternion(turn) });
            }
            out
        }
        Rule::Tangent { sign } => {
            let Local::Plane(plane) = first.local else {
                return Err(unsupported("a Tangent's first end is the plane"));
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
                    vec![Constraint::FromPlane { mate, a, plane, b, point, direction, gap, says }]
                }
                Local::Point(point) => {
                    vec![Constraint::FromPlane { mate, a, plane, b, point, direction: None, gap, says }]
                }
                Local::Plane(_) => return Err(unsupported("a Tangent's second end is a cylinder or a sphere")),
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
    fn a_distance_across_two_axes_is_refused_and_a_coincident_one_is_coaxial() {
        let axes = || vec![shaft(q(16, 1), Some(true)), Read { body: body(2), ..shaft(q(16, 1), Some(false)) }];
        let distance = Rule::Parallel { sign: 1, distance: Some(5.0), coincident: false };
        let refused = constraint("D".into(), axes(), &distance).unwrap_err();
        assert_eq!(refused.code, RefusalCode::UnsupportedReference);
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
    fn concentric_takes_no_planes_and_locks_quarter_turns_only() {
        let rule = Rule::Concentric { sign: Some(1), lock: None };
        let refused = constraint("C".into(), vec![plane([0, 0, 1], 0), shaft(q(1, 1), Some(true))], &rule).unwrap_err();
        assert_eq!(refused.code, RefusalCode::UnsupportedReference);
        let axes = || vec![Read { body: body(2), ..shaft(q(16, 1), Some(false)) }, shaft(q(16, 1), Some(true))];
        let skew = Rule::Concentric { sign: Some(1), lock: Some([[1, 1, 0], [0, 1, 0], [0, 0, 1]]) };
        assert_eq!(constraint("C".into(), axes(), &skew).unwrap_err().code, RefusalCode::UnsupportedPlacement);
        let quarter = Rule::Concentric { sign: Some(1), lock: Some([[0, -1, 0], [1, 0, 0], [0, 0, 1]]) };
        let made = constraint("C".into(), axes(), &quarter).unwrap();
        assert!(matches!(made[..], [Constraint::Axes { .. }, Constraint::Turn { .. }]));
    }
}
