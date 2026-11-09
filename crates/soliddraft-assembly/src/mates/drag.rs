//! Drag previews leave authored truth and history untouched until release.
use crate::mates::bodies::{self, Scope};
use crate::mates::{MateRefusal, RefusalCode};
use crate::schema::Assembly;
use crate::{AssemblyDocument, EditError, OccurrencePath, Placement};

#[derive(Clone)]
pub(crate) struct Drag {
    pub(crate) path: OccurrencePath,
    pub(crate) base: Assembly,
    pub(crate) candidate: Assembly,
    shown: Vec<(OccurrencePath, Placement)>,
}
impl AssemblyDocument {
    /// Whether a component preview is active.
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Starts one component drag. Targets passed to `preview_drag` use its parent's frame.
    pub fn begin_drag(&mut self, path: OccurrencePath) -> Result<(), EditError> {
        self.finish_loading();
        if self.drag.is_some() {
            return Err(EditError::Invalid("a component drag is already active".into()));
        }
        if path.steps().is_empty() {
            return Err(EditError::Root);
        }
        if self.table.find(&path).is_none_or(|r| self.table.flags(r).has(crate::occurrence::Flags::REMOVED)) {
            return Err(EditError::NoOccurrence(path));
        }
        self.motion_report = None;
        self.drag =
            Some(Drag { path, base: self.assembly.clone(), candidate: self.assembly.clone(), shown: Vec::new() });
        Ok(())
    }

    /// Solves a placement wish under mates and shows the accepted preview without an undo step.
    /// A refused update retains the previous preview.
    pub fn preview_drag(&mut self, target: Placement) -> Result<(), MateRefusal> {
        if self.motion.is_some() {
            return self.queue_motion(target);
        }
        if !target.translation.is_finite()
            || !target.rotation.is_finite()
            || (target.rotation.length_squared() - 1.0).abs() > 1e-8
        {
            return Err(MateRefusal::new(
                RefusalCode::InvalidValue,
                "a drag target needs finite coordinates and a unit quaternion",
            ));
        }
        let drag = self.drag.as_ref().ok_or_else(|| MateRefusal::invalid("no component drag is active"))?;
        if drag.base != self.assembly {
            return Err(MateRefusal::invalid("the assembly changed during the drag; cancel and start again"));
        }
        let path = drag.path.clone();
        let mut candidate = drag.candidate.clone();
        let previous = Scope::new(&candidate, &self.table, &self.store).world(&path).unwrap_or(target);
        match Self::owner(&path) {
            Ok(id) => {
                candidate
                    .component_mut(id)
                    .ok_or_else(|| MateRefusal::invalid("the dragged component was removed"))?
                    .placement = target
            }
            Err(p) => candidate.override_mut(&p).placement = Some(target),
        }
        let solved = self.solve_drag_truth(&candidate, &path, previous)?;
        let kept = bodies::locals(&Scope::new(&candidate, &self.table, &self.store), &solved.placements);
        bodies::keep(&mut candidate, kept);
        self.refresh_mechanical(&mut candidate, None)?;
        self.verify_problem(self.gather(&Scope::new(&candidate, &self.table, &self.store))?)?;
        self.show_drag_candidate(candidate);
        Ok(())
    }

    pub(crate) fn show_drag_candidate(&mut self, candidate: Assembly) {
        let scope = Scope::new(&candidate, &self.table, &self.store);
        let placed: Vec<_> = (1..self.table.len() as u32)
            .filter_map(|i| {
                let row = crate::occurrence::OccurrenceIndex(i);
                let path = self.table.path(row);
                let local = scope.local(&path)?;
                (local != self.table.local(row)).then_some((row, path, local))
            })
            .collect();
        let drag = self.drag.as_mut().expect("active drag");
        for (row, path, local) in placed {
            if !drag.shown.iter().any(|(p, _)| *p == path) {
                let original =
                    Scope::new(&drag.base, &self.table, &self.store).local(&path).unwrap_or(self.table.local(row));
                drag.shown.push((path, original));
            }
            let moved = self.table.set_local(row, local, &self.store);
            self.bvh_moved.extend(moved.iter().copied());
            self.changes.moved.extend(moved);
        }
        drag.candidate = candidate;
    }

    /// Restores the accepted pose and discards the preview, preserving undo and redo.
    pub fn cancel_drag(&mut self) {
        self.motion = None;
        self.motion_report = None;
        let Some(drag) = self.drag.take() else { return };
        self.restore_drag_rows(&drag.shown);
    }

    fn restore_drag_rows(&mut self, rows: &[(OccurrencePath, Placement)]) {
        for (path, pose) in rows {
            if let Some(row) = self.table.find(path) {
                let pose = Scope::new(&self.assembly, &self.table, &self.store).local(path).unwrap_or(*pose);
                let moved = self.table.set_local(row, pose, &self.store);
                self.bvh_moved.extend(moved.iter().copied());
                self.changes.moved.extend(moved);
            }
        }
    }

    /// Commits the last accepted preview as one undo step. A stationary release is a no-op.
    pub fn commit_drag(&mut self) -> Result<(), MateRefusal> {
        if let Some(motion) = self.motion.as_mut() {
            motion.release = true;
            self.poll_drag();
            return Ok(());
        }
        let drag = self.drag.take().ok_or_else(|| MateRefusal::invalid("no component drag is active"))?;
        self.restore_drag_rows(&drag.shown);
        if drag.base != self.assembly {
            return Err(MateRefusal::invalid("the assembly changed during the drag"));
        }
        self.verify_problem(self.gather(&Scope::new(&drag.candidate, &self.table, &self.store))?)?;
        if drag.candidate != self.assembly {
            self.publish("Drag component", drag.candidate);
        }
        Ok(())
    }
}
