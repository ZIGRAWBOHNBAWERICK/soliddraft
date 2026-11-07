//! Mate commands. Every one runs the same way: copy the truth, change the copy, prove and
//! solve it, and only then publish truth, pose and undo as one step. A refusal changes
//! nothing, and Update solves its candidate through the same [`AssemblyDocument::solve_truth`].

use std::collections::BTreeMap;

use num_bigint::BigInt;
use num_rational::BigRational;
use serde::Serialize;
use soliddraft_document::{ContentHash, DocRef, DocumentId};
use soliddraft_kernel_api::Certainty;
use soliddraft_model::ValueTable;

use crate::document::{AssemblyDocument, EditError, key_of};
use crate::mates::bodies::{End, Gathered, Problem, Rule, Scope};
use crate::mates::reference::check_endpoint;
use crate::mates::solver::{self, Along, Body, Constraint, FreeMotion, LocalPlane, SolverState, SolverStatus};
use crate::mates::{
    Alignment, EndpointSelector, FaceEndpoint, Mate, MateEntry, MateId, MateRefusal, PartEndpoint, PlaneProof,
    RefusalCode, WidthConstraint,
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
                Gathered::Planes { mate, ends, rule } => {
                    let mut planes = Vec::new();
                    for end in &ends {
                        let source = self.rebased(end);
                        match self.prove(&source, &end.entity) {
                            Ok(proof) => planes.push((end.body.clone(), local_plane(&mate, &proof)?)),
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
                        constraints.push(constraint(mate, planes, &rule)?);
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
        let stored = |end: &End| {
            let normal = end.entity.stored_normal().ok_or("a mate's face has no stored normal")?;
            Ok::<_, String>((end.body.clone(), LocalPlane { normal, offset: num_traits::Zero::zero() }))
        };
        let mut constraints = Vec::new();
        for mate in problem.mates {
            constraints.push(match mate {
                Gathered::Origin { mate, body, axes, at } => Constraint::Origin { mate, body, axes, at },
                Gathered::Lock { mate, a, b, relative } => Constraint::Lock { mate, a, b, relative },
                Gathered::Planes { mate, ends, rule } => {
                    let planes = ends.iter().map(stored).collect::<Result<Vec<_>, _>>()?;
                    constraint(mate, planes, &rule).map_err(|r| r.to_string())?
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
        Ok(FaceEndpoint {
            path: component_path(selector.component),
            document,
            configuration: source.configuration,
            entity,
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
                let measured = match m.measured() {
                    Some([a, b]) => self.measure(a, b).map(Some),
                    None => self.prove_ends(m).map(|()| None),
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
            self.prove(&source, &end.entity)?;
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
        self.transact("Edit mate", false, |_, truth| {
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
                        | Mate::Width { alignment, .. },
                        MateEdit::Alignment(a),
                    ) => *alignment = *a,
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

/// `b`'s placement in `a`'s frame, when the turn between them is a quarter-turn rotation.
/// The turn is stored as the rotation it rounds onto, and the offset is computed through
/// that rotation's integer matrix, so the lock holds what the two placements say exactly.
fn relative_lock(mate: &str, a: &Placement, b: &Placement) -> Result<Placement, MateRefusal> {
    let turn = a.rotation.inverse() * b.rotation;
    let m = solver::cube_of(turn).ok_or_else(|| off_quarter_turns(mate))?;
    let col = |c: usize| glam::DVec3::new(m[0][c] as f64, m[1][c] as f64, m[2][c] as f64);
    let rotation = glam::DQuat::from_mat3(&glam::DMat3::from_cols(col(0), col(1), col(2)));
    let ra = solver::cube_of(a.rotation).ok_or_else(|| solver::unsupported_lock(mate))?;
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

/// The solver's constraint for a mate between planes, from the planes its ends lie on.
fn constraint(mate: String, planes: Vec<(Body, LocalPlane)>, rule: &Rule) -> Result<Constraint, MateRefusal> {
    let not_parallel = |mate: &str| MateRefusal {
        mates: vec![mate.to_owned()],
        ..MateRefusal::new(
            RefusalCode::NotParallel,
            format!("{mate}: a groove's faces, and a tab's, are parallel planes"),
        )
    };
    let mut planes = planes.into_iter();
    let (a, plane_a) = planes.next().expect("two planes");
    let (b, plane_b) = planes.next().expect("two planes");
    Ok(match rule {
        Rule::Parallel { sign, distance } => {
            Constraint::Parallel { mate, a, b, plane_a, plane_b, sign: *sign, along: distance.map(Along::Distance) }
        }
        Rule::Perpendicular => Constraint::Perpendicular { mate, a, b, plane_a, plane_b },
        Rule::Width { sign, mode } => {
            // The ends are groove, groove, tab, tab.
            let (groove_body, groove) = (b, plane_b);
            let (tab_body, tab) = planes.next().expect("four planes");
            let (other_body, other_tab) = planes.next().expect("four planes");
            if groove_body != a || other_body != tab_body || tab_body == a {
                let message = format!("{mate}: a groove's faces are on one part and a tab's on another");
                return Err(MateRefusal { mates: vec![mate], ..MateRefusal::invalid(message) });
            }
            let parallel =
                |p: &LocalPlane, q: &LocalPlane| p.normal.iter().zip(q.normal).all(|(x, y)| x.abs() == y.abs());
            if !parallel(&plane_a, &groove) || !parallel(&tab, &other_tab) {
                return Err(not_parallel(&mate));
            }
            let along = Some(Along::Width { groove, tab: other_tab, mode: mode.clone() });
            Constraint::Parallel { mate, a, b: tab_body, plane_a, plane_b: tab, sign: *sign, along }
        }
    })
}

fn local_plane(mate: &str, proof: &PlaneProof) -> Result<LocalPlane, MateRefusal> {
    let offset = solver::rational(&proof.plane.offset).ok_or_else(|| {
        MateRefusal::new(RefusalCode::UnsupportedPlane, format!("{mate}: the plane has no exact offset"))
    })?;
    Ok(LocalPlane { normal: proof.plane.normal(), offset })
}

fn signed_gap(a: &PlaneProof, b: &PlaneProof, placements: &[Placement; 2]) -> Result<BigRational, MateRefusal> {
    let off = |_| MateRefusal::new(RefusalCode::UnsupportedPlacement, "a face is placed off the world axes");
    let normal = |p: &PlaneProof, at: &Placement| {
        let n = p.plane.normal().map(f64::from);
        let v = at.rotate(glam::DVec3::from_array(n));
        let rounded = v.to_array().map(f64::round);
        (v.distance(glam::DVec3::from_array(rounded)) <= 1e-12 && rounded.iter().map(|c| c.abs()).sum::<f64>() == 1.0)
            .then_some(rounded)
            .ok_or(())
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
