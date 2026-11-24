//! Drag checks run on owned snapshots. Only certified prefixes reach the live document.
use super::*;
use crate::mates::MateRefusal;
use crate::path::ComponentId;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// A nearest face named by the certified mesh, for viewport feedback.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MotionFace {
    /// Part occurrence.
    pub path: OccurrencePath,
    /// Face label at the pinned revision.
    pub face: String,
    /// Persistent face reference matching the display mesh, or empty when unavailable.
    pub reference: String,
}

/// Feedback for the last collision or dynamic clearance update.
#[derive(Clone, Debug, Serialize)]
pub struct MotionReport {
    /// This check uses certified meshes, not exact two-body contact.
    pub tier: &'static str,
    /// Minimum requested separation, in millimetres.
    pub clearance_mm: f64,
    /// Whether motion stopped before reaching the wish.
    pub stopped: bool,
    /// Bounds on the nearest moving pair at the accepted pose.
    pub distance: Option<DistanceBounds>,
    /// Nearest faces at the stop.
    pub faces: Vec<MotionFace>,
    /// A refusal leaves the previous preview in place.
    pub error: Option<String>,
    /// The parts Physical Dynamics pushed out of the way, by occurrence path.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pushed: Vec<OccurrencePath>,
}

pub(crate) struct Motion {
    clearance: f64,
    push: bool,
    pending: Option<Placement>,
    job: Option<mpsc::Receiver<Result<(AssemblyDocument, MotionReport), String>>>,
    cancelled: Arc<AtomicBool>,
    pub(crate) release: bool,
}
impl Drop for Motion {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl AssemblyDocument {
    /// Starts collision detection, or dynamic clearance when a minimum distance is given.
    /// Includes hidden parts and refuses unchecked geometry. Suppressed parts are omitted.
    pub fn begin_guarded_drag(&mut self, path: OccurrencePath, clearance: Option<f64>) -> Result<(), crate::EditError> {
        self.begin_motion(path, clearance, false)
    }

    /// Starts Physical Dynamics: a guarded drag in which the dragged component pushes the
    /// parts it meets instead of stopping. A pushed part moves by the dragged component's
    /// translation, as its own mates allow; one its mates hold stops the drag as a collision
    /// does. Kinematic only: no forces, masses, friction or bounce. The dragged component is
    /// one of the root's own; what it pushes is a root component.
    pub fn begin_dynamic_drag(&mut self, path: OccurrencePath, clearance: Option<f64>) -> Result<(), crate::EditError> {
        if path.steps().len() != 1 || path.steps()[0].member.is_some() {
            return Err(crate::EditError::Invalid("Physical Dynamics drags one of the root's own components".into()));
        }
        self.begin_motion(path, clearance, true)
    }

    fn begin_motion(
        &mut self,
        path: OccurrencePath,
        clearance: Option<f64>,
        push: bool,
    ) -> Result<(), crate::EditError> {
        if cfg!(target_arch = "wasm32") {
            return Err(crate::EditError::Invalid("guarded dragging needs the desktop collision worker".into()));
        }
        let clearance = clearance.unwrap_or(0.0);
        if !clearance.is_finite() || clearance < 0.0 {
            return Err(crate::EditError::Invalid("a dynamic clearance is a finite length of zero or more".into()));
        }
        self.begin_drag(path)?;
        self.motion_report = None;
        self.motion = Some(Motion {
            clearance,
            push,
            pending: None,
            job: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            release: false,
        });
        Ok(())
    }

    /// True until all requested collision checks and a requested release have completed.
    pub fn drag_pending(&self) -> bool {
        self.motion.as_ref().is_some_and(|m| m.job.is_some() || m.pending.is_some())
    }

    /// Last completed mesh-certified drag result. Preview feedback is never saved.
    pub fn drag_report(&self) -> Option<&MotionReport> {
        self.motion_report.as_ref()
    }

    pub(crate) fn queue_motion(&mut self, target: Placement) -> Result<(), MateRefusal> {
        if !target.translation.is_finite()
            || !target.rotation.is_finite()
            || (target.rotation.length_squared() - 1.0).abs() > 1e-8
        {
            return Err(MateRefusal::invalid("a drag target needs finite coordinates and a unit quaternion"));
        }
        let m = self.motion.as_mut().expect("guarded drag");
        if m.release {
            return Err(MateRefusal::invalid("the drag is being released"));
        }
        m.pending = Some(target);
        self.poll_drag();
        Ok(())
    }

    /// Takes completed work without waiting, then starts the most recent queued wish.
    pub fn poll_drag(&mut self) {
        let Some(mut motion) = self.motion.take() else { return };
        if let Some(rx) = motion.job.as_ref() {
            let result = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("the collision worker ended without a result".into()))
                }
            };
            if let Some(result) = result {
                motion.job = None;
                match result {
                    Ok((worker, report)) => {
                        if let (Some(live), Some(done)) = (&self.drag, &worker.drag)
                            && live.base == self.assembly
                            && worker.assembly == self.assembly
                        {
                            self.show_drag_candidate(done.candidate.clone());
                            self.certified = worker.certified;
                            self.motion_report = Some(report);
                        }
                    }
                    Err(error) => {
                        self.motion_report = Some(MotionReport {
                            tier: "certified_mesh",
                            clearance_mm: motion.clearance,
                            stopped: true,
                            distance: None,
                            faces: Vec::new(),
                            error: Some(error),
                            pushed: Vec::new(),
                        })
                    }
                }
            }
        }
        if motion.job.is_none() {
            if let Some(target) = motion.pending.take() {
                let mut worker = self.drag_snapshot();
                let (clearance, push) = (motion.clearance, motion.push);
                let cancelled = motion.cancelled.clone();
                let (tx, rx) = mpsc::channel();
                motion.job = Some(rx);
                std::thread::spawn(move || {
                    let result =
                        advance(&mut worker, target, clearance, push, &cancelled).map(|report| (worker, report));
                    let _ = tx.send(result);
                });
            } else if motion.release {
                if let Err(e) = self.commit_drag() {
                    self.motion_report = Some(MotionReport {
                        tier: "certified_mesh",
                        clearance_mm: motion.clearance,
                        stopped: true,
                        distance: None,
                        faces: Vec::new(),
                        error: Some(e.to_string()),
                        pushed: Vec::new(),
                    });
                }
                return;
            }
        }
        self.motion = Some(motion);
    }
}

struct Part {
    row: OccurrenceIndex,
    path: OccurrencePath,
    mesh: Arc<LocalMesh>,
    radius: f64,
}
impl Part {
    fn solid(&self, at: Placement) -> Solid {
        let place = |i: u32| at.apply(DVec3::from_array(self.mesh.positions[i as usize].map(f64::from)));
        let tris = self.mesh.indices.as_chunks::<3>().0.iter().map(|t| t.map(place)).collect();
        Solid::new(tris, self.mesh.triangle_faces.clone(), self.mesh.chord)
    }
}

fn parts(doc: &mut AssemblyDocument, cancelled: &AtomicBool) -> Result<Vec<Part>, String> {
    let request = CheckRequest { include_hidden: true, ..CheckRequest::default() };
    let (entries, unread) = doc.scope_parts(&request).map_err(|e| e.to_string())?;
    if !unread.is_empty() {
        return Err(unread.iter().map(|p| p.message.as_str()).collect::<Vec<_>>().join("; "));
    }
    if entries.len() > MAX_PARTS {
        return Err(format!("a guarded drag checks at most {MAX_PARTS} parts"));
    }
    entries
        .into_iter()
        .map(|entry| {
            if cancelled.load(Ordering::Relaxed) {
                return Err("drag cancelled".into());
            }
            let (_, mesh, _) = doc.placed(&entry, request.chord).map_err(|(_, e)| e)?;
            let radius =
                mesh.positions.iter().map(|p| DVec3::from_array(p.map(f64::from)).length()).fold(0.0, f64::max);
            Ok(Part { row: entry.row, path: entry.path, mesh, radius })
        })
        .collect()
}

fn poses(doc: &AssemblyDocument, parts: &[Part]) -> Vec<Placement> {
    parts.iter().map(|p| doc.table.world(p.row)).collect()
}

fn travel(a: Placement, b: Placement, radius: f64) -> f64 {
    if a == b {
        return 0.0;
    }
    let q = if a.rotation.dot(b.rotation) < 0.0 { -b.rotation } else { b.rotation };
    // Four times quaternion chord length bounds the shortest rotation's angle.
    let distance = a.translation.distance(b.translation) + 4.0 * (a.rotation - q).length() * radius;
    distance * (1.0 + 1e-10) + 1e-9
}

fn mixed(a: Placement, b: Placement, t: f64) -> Placement {
    Placement::new(a.rotation.slerp(b.rotation, t), a.translation.lerp(b.translation, t))
}

fn changing(a: &[Placement], b: &[Placement], i: usize, j: usize) -> bool {
    if a[i] == b[i] && a[j] == b[j] {
        return false;
    }
    // A common translation leaves an existing contact unchanged.
    !(a[i].rotation == b[i].rotation
        && a[j].rotation == b[j].rotation
        && (0..3).all(|k| {
            let exact = |x| num_rational::BigRational::from_float(x);
            match (
                exact(b[i].translation[k]),
                exact(a[i].translation[k]),
                exact(b[j].translation[k]),
                exact(a[j].translation[k]),
            ) {
                (Some(bi), Some(ai), Some(bj), Some(aj)) => bi - ai == bj - aj,
                _ => false,
            }
        }))
}

/// Whether two parts keep their relative placement, to 1e-9 mm, from `a` to `b`: a pushed
/// part and what pushes it, whose translations agree up to rounding.
fn together(a: &[Placement], b: &[Placement], i: usize, j: usize) -> bool {
    a[i].rotation == b[i].rotation
        && a[j].rotation == b[j].rotation
        && ((b[i].translation - a[i].translation) - (b[j].translation - a[j].translation)).length() <= 1e-9
}

fn measure(
    parts: &[Part],
    at: &[Placement],
    end: &[Placement],
    clearance: f64,
    swept: bool,
    group: &[bool],
) -> (bool, Option<DistanceBounds>, Vec<MotionFace>) {
    let solids: Vec<_> = parts.iter().zip(at).map(|(p, at)| p.solid(*at)).collect();
    let mut safe = true;
    let mut minimum: Option<DistanceBounds> = None;
    let mut faces = Vec::new();
    for i in 0..parts.len() {
        for j in i + 1..parts.len() {
            if !changing(at, end, i, j) || (group[i] && group[j] && together(at, end, i, j)) {
                continue;
            }
            let movement = if swept {
                travel(at[i], end[i], parts[i].radius) + travel(at[j], end[j], parts[j].radius)
            } else {
                0.0
            };
            let (pair, near) = compare(&solids[i], &solids[j], Some(f64::MAX / 4.0));
            if pair.outcome != Outcome::Clear || pair.distance.lo <= clearance + movement {
                safe = false;
            }
            if minimum.is_none_or(|d| pair.distance.lo < d.lo) {
                minimum = Some(pair.distance);
                faces.clear();
                if let Some((ta, tb)) = near.first() {
                    faces.push(MotionFace {
                        path: parts[i].path.clone(),
                        face: parts[i].mesh.faces[solids[i].face(*ta) as usize].1.clone(),
                        reference: parts[i].mesh.face_names[solids[i].face(*ta) as usize].clone(),
                    });
                    faces.push(MotionFace {
                        path: parts[j].path.clone(),
                        face: parts[j].mesh.faces[solids[j].face(*tb) as usize].1.clone(),
                        reference: parts[j].mesh.face_names[solids[j].face(*tb) as usize].clone(),
                    });
                }
            } else if let Some(d) = minimum.as_mut() {
                d.hi = match (d.hi, pair.distance.hi) {
                    (Some(a), Some(b)) => Some(a.min(b)),
                    (a, b) => a.or(b),
                };
            }
        }
    }
    (safe, minimum, faces)
}

fn joint_phases(doc: &AssemblyDocument) -> Result<Vec<[f64; 2]>, String> {
    use crate::mates::bodies::{Gathered, Rule};
    let truth = &doc.drag.as_ref().ok_or("no drag is active")?.candidate;
    let problem = doc.gather(&Scope::new(truth, &doc.table, &doc.store)).map_err(|e| e.to_string())?;
    Ok(problem
        .mates
        .into_iter()
        .filter_map(|m| match m {
            Gathered::Between { rule: Rule::Mechanical { angles, .. }, .. } => Some(angles),
            _ => None,
        })
        .collect())
}

fn short_joint_steps(before: &[[f64; 2]], after: &[[f64; 2]]) -> bool {
    before.len() == after.len()
        && before
            .iter()
            .flatten()
            .zip(after.iter().flatten())
            .all(|(a, b)| (a - b).abs() <= std::f64::consts::FRAC_PI_4)
}

/// The root component a guarded step met and did not move: what Physical Dynamics pushes.
fn obstacle(parts: &[Part], faces: &[MotionFace], before: &[Placement], after: &[Placement]) -> Option<ComponentId> {
    faces.iter().find_map(|f| {
        let i = parts.iter().position(|p| p.path == f.path)?;
        (before[i] == after[i]).then(|| crate::mates::holder(&f.path)).flatten()
    })
}

fn advance(
    doc: &mut AssemblyDocument,
    target: Placement,
    clearance: f64,
    push: bool,
    cancelled: &AtomicBool,
) -> Result<MotionReport, String> {
    let parts = parts(doc, cancelled)?;
    let path = doc.drag.as_ref().ok_or("no drag is active")?.path.clone();
    let dragged = crate::mates::holder(&path);
    let mut work = vec![(target, 0_u32)];
    // What this wish pushes, once met: it moves with every later step of the wish.
    let mut carried: Vec<ComponentId> = Vec::new();
    let mut report = MotionReport {
        tier: "certified_mesh",
        clearance_mm: clearance,
        stopped: false,
        distance: None,
        faces: Vec::new(),
        error: None,
        pushed: Vec::new(),
    };
    for _ in 0..256 {
        if cancelled.load(Ordering::Relaxed) {
            return Err("drag cancelled".into());
        }
        let Some((wish, depth)) = work.pop() else { return Ok(report) };
        let before = poses(doc, &parts);
        let phases = joint_phases(doc)?;
        let old_table = doc.table.clone();
        let old_drag = doc.drag.clone();
        let start = doc.table.local(doc.table.find(&path).ok_or("the dragged occurrence disappeared")?);
        if let Err(e) = doc.preview_carrying(wish, &carried) {
            doc.table = old_table;
            doc.drag = old_drag;
            return Err(e.to_string());
        }
        let after = poses(doc, &parts);
        // The dragged component and what it pushes move as one group.
        let group: Vec<bool> = parts
            .iter()
            .map(|p| {
                let holder = crate::mates::holder(&p.path);
                holder.is_some() && (holder == dragged || holder.is_some_and(|h| carried.contains(&h)))
            })
            .collect();
        let (safe, distance, faces) = measure(&parts, &before, &after, clearance, true, &group);
        if safe && short_joint_steps(&phases, &joint_phases(doc)?) {
            for &id in &carried {
                let path = OccurrencePath::root().child(crate::path::Step::component(id));
                if !report.pushed.contains(&path)
                    && let Some((previous, current)) = old_table.find(&path).zip(doc.table.find(&path))
                    && old_table.local(previous) != doc.table.local(current)
                {
                    report.pushed.push(path);
                }
            }
            // Use the same set of moving pairs when reporting their accepted separation.
            let (_, distance, faces) = measure(&parts, &after, &before, clearance, false, &group);
            report.distance = distance;
            report.faces = faces;
            continue;
        }
        doc.table = old_table;
        doc.drag = old_drag;
        let met = obstacle(&parts, &faces, &before, &after);
        report.distance = distance;
        report.faces = faces;
        let mid = mixed(start, wish, 0.5);
        let stuck = depth >= 32
            || mid == start
            || before
                .iter()
                .zip(&after)
                .map(|(a, b)| a.translation.distance(b.translation) + (a.rotation - b.rotation).length())
                .fold(0.0, f64::max)
                < 1e-6;
        // Physical Dynamics pushes what it met instead of stopping, and tries the step again.
        if stuck
            && push
            && let Some(id) = met
            && Some(id) != dragged
            && !carried.contains(&id)
            && carried.len() < 16
        {
            carried.push(id);
            work.push((wish, depth));
            continue;
        }
        if stuck {
            report.stopped = true;
            return Ok(report);
        }
        work.push((wish, depth + 1));
        work.push((mid, depth + 1));
    }
    report.stopped = true;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DQuat;

    #[test]
    fn travel_bounds_every_point_of_a_rotating_translating_part() {
        for angle in [0.001, 0.5, 1.5, std::f64::consts::PI] {
            let a = Placement::IDENTITY;
            let b = Placement::new(DQuat::from_rotation_z(angle), DVec3::new(10.0, -7.0, 3.0));
            let bound = travel(a, b, 20.0);
            for step in 0..=100 {
                let pose = mixed(a, b, step as f64 / 100.0);
                for p in [DVec3::X, DVec3::Y, DVec3::Z] {
                    assert!(pose.apply(p * 20.0).distance(a.apply(p * 20.0)) <= bound);
                }
            }
        }
    }

    #[test]
    fn a_full_coupled_turn_requires_subdivision_even_when_its_quaternion_repeats() {
        assert!(!short_joint_steps(&[[0.0, 0.0]], &[[0.0, std::f64::consts::TAU]]));
        assert!(short_joint_steps(&[[0.0, 0.0]], &[[0.2, 0.3]]));
    }

    #[test]
    fn common_translation_requires_equal_exact_displacements() {
        let before = [Placement::translation(DVec3::X), Placement::IDENTITY];
        let after = [Placement::translation(DVec3::X * 1e16); 2];
        assert_eq!(after[0].translation - before[0].translation, after[1].translation - before[1].translation);
        assert!(changing(&before, &after, 0, 1));
        let before = [Placement::IDENTITY, Placement::translation(DVec3::X * 1e16)];
        let shifted = [Placement::translation(DVec3::X * 4.0), Placement::translation(DVec3::X * (1e16 + 4.0))];
        assert!(!changing(&before, &shifted, 0, 1));
    }
}
