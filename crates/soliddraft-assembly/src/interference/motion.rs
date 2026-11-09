//! Drag checks run on owned snapshots. Only certified prefixes reach the live document.
use super::*;
use crate::mates::MateRefusal;
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
}

pub(crate) struct Motion {
    clearance: f64,
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
        let clearance = clearance.unwrap_or(0.0);
        if !clearance.is_finite() || clearance < 0.0 {
            return Err(crate::EditError::Invalid("a dynamic clearance is a finite length of zero or more".into()));
        }
        self.begin_drag(path)?;
        self.motion_report = None;
        self.motion = Some(Motion {
            clearance,
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
                        if let (Some(live), Some(done)) = (&self.drag, &worker.drag) {
                            if live.base == self.assembly && worker.assembly == self.assembly {
                                self.show_drag_candidate(done.candidate.clone());
                                self.certified = worker.certified;
                                self.motion_report = Some(report);
                            }
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
                        })
                    }
                }
            }
        }
        if motion.job.is_none() {
            if let Some(target) = motion.pending.take() {
                let mut worker = self.drag_snapshot();
                let clearance = motion.clearance;
                let cancelled = motion.cancelled.clone();
                let (tx, rx) = mpsc::channel();
                motion.job = Some(rx);
                std::thread::spawn(move || {
                    let result = advance(&mut worker, target, clearance, &cancelled).map(|report| (worker, report));
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

fn parts(doc: &mut AssemblyDocument) -> Result<Vec<Part>, String> {
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
        && b[i].translation - a[i].translation == b[j].translation - a[j].translation)
}

fn measure(
    parts: &[Part],
    at: &[Placement],
    end: &[Placement],
    clearance: f64,
    swept: bool,
) -> (bool, Option<DistanceBounds>, Vec<MotionFace>) {
    let solids: Vec<_> = parts.iter().zip(at).map(|(p, at)| p.solid(*at)).collect();
    let mut safe = true;
    let mut minimum: Option<DistanceBounds> = None;
    let mut faces = Vec::new();
    for i in 0..parts.len() {
        for j in i + 1..parts.len() {
            if !changing(at, end, i, j) {
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
                    });
                    faces.push(MotionFace {
                        path: parts[j].path.clone(),
                        face: parts[j].mesh.faces[solids[j].face(*tb) as usize].1.clone(),
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

fn advance(
    doc: &mut AssemblyDocument,
    target: Placement,
    clearance: f64,
    cancelled: &AtomicBool,
) -> Result<MotionReport, String> {
    let parts = parts(doc)?;
    let path = doc.drag.as_ref().ok_or("no drag is active")?.path.clone();
    let mut work = vec![(target, 0_u32)];
    let mut report = MotionReport {
        tier: "certified_mesh",
        clearance_mm: clearance,
        stopped: false,
        distance: None,
        faces: Vec::new(),
        error: None,
    };
    for _ in 0..256 {
        if cancelled.load(Ordering::Relaxed) {
            return Err("drag cancelled".into());
        }
        let Some((wish, depth)) = work.pop() else { return Ok(report) };
        let before = poses(doc, &parts);
        let old_table = doc.table.clone();
        let old_drag = doc.drag.clone();
        let start = doc.table.local(doc.table.find(&path).ok_or("the dragged occurrence disappeared")?);
        if let Err(e) = doc.preview_drag(wish) {
            doc.table = old_table;
            doc.drag = old_drag;
            return Err(e.to_string());
        }
        let after = poses(doc, &parts);
        let (safe, distance, faces) = measure(&parts, &before, &after, clearance, true);
        if safe {
            // Use the same set of moving pairs when reporting their accepted separation.
            let (_, distance, faces) = measure(&parts, &after, &before, clearance, false);
            report.distance = distance;
            report.faces = faces;
            continue;
        }
        doc.table = old_table;
        doc.drag = old_drag;
        report.distance = distance;
        report.faces = faces;
        let mid = mixed(start, wish, 0.5);
        if depth >= 32
            || mid == start
            || before
                .iter()
                .zip(&after)
                .map(|(a, b)| a.translation.distance(b.translation) + (a.rotation - b.rotation).length())
                .fold(0.0, f64::max)
                < 1e-6
        {
            report.stopped = true;
            return Ok(report);
        }
        work.push((wish, depth + 1));
        work.push((mid, depth + 1));
    }
    report.stopped = true;
    Ok(report)
}
