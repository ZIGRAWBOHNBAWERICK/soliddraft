//! The device budget: how much display data the app lets itself hold.
//!
//! The app owns the budget, not the document: a file opens the same on every machine,
//! and a smaller device shows it at coarser levels. The display cache counts what it
//! holds against the budget and sets the ceiling the level choice obeys.
//!
//! A single part is budgeted before it is meshed. The kernel plans the part once and
//! counts the points of every face at every level; [`DeviceBudget::grant`] steps faces
//! down from the levels the camera asks for until the counted total fits, and only then
//! is a mesh made ([`DeviceBudget::mesh_faces`]). When even the coarsest face levels of
//! a plan are over, no mesh is made and the part is planned at its next coarser level
//! ([`DeviceBudget::mesh_part`]). A larger budget grants finer levels and a smaller one
//! coarser. No level's mesh depends on the budget.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use soliddraft_kernel_api::{BodyId, FaceId, FaceLevel, FaceLevelPoints, Kernel, KernelResult, Mesh};

use crate::level::Level;
use crate::policy::{Deviations, FaceLevels, Levels};

/// Display data held, counted once per distinct mesh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// Triangles.
    pub triangles: u64,
    /// Bytes of vertex and index data.
    pub bytes: u64,
}

/// The most display bytes one counted point can take: a position and a normal, and at
/// most two triangles of three indices and a face label each.
pub const POINT_BYTES: u64 = 24 + 2 * (12 + 8);

impl Usage {
    /// The most display data a mesh of `points` counted points takes. A face has at
    /// most two triangles per point, whatever its shape, so the bound holds before any
    /// triangle exists.
    pub fn of_points(points: u64) -> Usage {
        Usage { triangles: points.saturating_mul(2), bytes: points.saturating_mul(POINT_BYTES) }
    }
}

/// One face as the budget sees it before the part is meshed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceDemand {
    /// The points the kernel counts for the face at each level, coarsest first.
    pub points: [u32; 3],
    /// The level the camera asks for.
    pub wanted: Level,
    /// How many pixels a millimetre covers at the face. Zero for a face out of view,
    /// which then gives up its detail first.
    pub pixels_per_mm: f64,
}

/// The levels the budget grants the faces of a part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Granted {
    /// Each face's level, in the order of the demands: the wanted one or a coarser one.
    pub levels: Vec<Level>,
    /// The counted points of those levels.
    pub points: u64,
    /// False when the budget is too small for the coarsest level of every face. The
    /// grant is then as coarse as it can be.
    pub fits: bool,
}

/// A single part's mesh with a level per face.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceMesh {
    /// The mesh. Faces at different levels meet on the same edge points.
    pub mesh: Mesh,
    /// Each face and the level it was granted, in topology order.
    pub levels: Vec<(FaceId, Level)>,
    /// The points counted for those levels before meshing. The mesh has exactly this
    /// many vertices.
    pub points: u64,
    /// As [`Granted::fits`]. False only for a part that had to be drawn over budget.
    pub fits: bool,
}

/// What became of one plan of a part's faces.
#[derive(Clone, Debug, PartialEq)]
pub enum Planned {
    /// The kernel plans no face levels for the body. It keeps its whole-part levels.
    Whole,
    /// The counted points are over the budget with every face at its coarsest. No mesh
    /// was made.
    Over(Granted),
    /// The mesh, within the budget.
    Mesh(Box<FaceMesh>),
}

/// One face moved to its next coarser level that has fewer points.
#[derive(Clone, Copy, Debug)]
struct Step {
    /// Pixels of error added on screen per point saved.
    loss: f64,
    saved: u32,
    face: usize,
    to: Level,
}

impl Step {
    /// The next step down for a face at `from`, if a coarser level saves any points.
    fn down(face: usize, demand: &FaceDemand, from: Level, levels: &impl Deviations) -> Option<Step> {
        let at = |level: Level| demand.points[level.index()];
        let mut to = from.coarser()?;
        while at(to) >= at(from) {
            to = to.coarser()?;
        }
        let saved = at(from) - at(to);
        let added = match (levels.deviation(to), levels.deviation(from)) {
            (Some(coarse), Some(fine)) => (coarse - fine).max(0.0) * demand.pixels_per_mm.max(0.0),
            _ => 0.0,
        };
        Some(Step { loss: added / f64::from(saved), saved, face, to })
    }
}

impl Ord for Step {
    /// The greatest step is the one to take first: the least loss per point saved, then
    /// the most points saved, then the first face.
    fn cmp(&self, other: &Step) -> Ordering {
        other
            .loss
            .total_cmp(&self.loss)
            .then(self.saved.cmp(&other.saved))
            .then(other.face.cmp(&self.face))
            .then(other.to.cmp(&self.to))
    }
}

impl PartialOrd for Step {
    fn partial_cmp(&self, other: &Step) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Step {
    fn eq(&self, other: &Step) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Step {}

/// The most display data the app holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceBudget {
    /// Triangles.
    pub triangles: u64,
    /// Bytes of vertex and index data.
    pub bytes: u64,
}

/// Above this share of the budget, nothing is promoted to the fine level.
const FINE_SHARE: f64 = 0.8;

impl DeviceBudget {
    /// No limit.
    pub const UNLIMITED: DeviceBudget = DeviceBudget { triangles: u64::MAX, bytes: u64::MAX };

    /// Derives a display allowance from the device's enabled maximum buffer size.
    /// Leave room for the assembly arena to grow fourfold and cap the allowance at
    /// 2 GiB: a legal allocation size is not a measure of available memory.
    pub fn from_buffer_limit(max_buffer_size: u64) -> DeviceBudget {
        let bytes = (max_buffer_size / 4).min(2 << 30);
        // Use a conservative 64 bytes per triangle for the count limit. The cache
        // also checks actual bytes, since vertex sharing varies by mesh.
        DeviceBudget { triangles: bytes / 64, bytes }
    }

    /// The finest level the level choice may promote to with `usage` held: fine below
    /// 80 percent of the budget, standard below all of it, coarse beyond.
    pub fn ceiling(&self, usage: Usage) -> Level {
        let share = self.share(usage);
        if share < FINE_SHARE {
            Level::Fine
        } else if share < 1.0 {
            Level::Standard
        } else {
            Level::Coarse
        }
    }

    /// True when `usage` is within the budget.
    pub fn fits(&self, usage: Usage) -> bool {
        usage.triangles <= self.triangles && usage.bytes <= self.bytes
    }

    /// How many counted points still fit with `held` already held.
    pub fn points_left(&self, held: Usage) -> u64 {
        let triangles = self.triangles.saturating_sub(held.triangles) / 2;
        let bytes = self.bytes.saturating_sub(held.bytes) / POINT_BYTES;
        triangles.min(bytes)
    }

    /// Chooses the level of each face of a part from its counted points, before the part
    /// is meshed. Every face gets the level it wants when the counted total fits in what
    /// `held` leaves of the budget. Otherwise faces step down until it does, the step
    /// that costs the least error on screen per point saved first, so faces out of view
    /// go before the ones the camera is on. A face never steps to a level that saves it
    /// no points.
    pub fn grant(&self, held: Usage, levels: &impl Deviations, faces: &[FaceDemand]) -> Granted {
        let room = self.points_left(held);
        let mut granted: Vec<Level> = faces.iter().map(|face| face.wanted).collect();
        let mut points: u64 = faces.iter().map(|face| u64::from(face.points[face.wanted.index()])).sum();
        let mut steps: BinaryHeap<Step> =
            faces.iter().enumerate().filter_map(|(i, face)| Step::down(i, face, face.wanted, levels)).collect();
        while points > room {
            let Some(step) = steps.pop() else { break };
            points -= u64::from(step.saved);
            granted[step.face] = step.to;
            steps.extend(Step::down(step.face, &faces[step.face], step.to, levels));
        }
        Granted { levels: granted, points, fits: points <= room }
    }

    /// Meshes a single part with a level per face, within what `held` leaves of the
    /// budget. The kernel plans `body` once at the parameters of `levels` and counts
    /// every face; `want` says which level the camera asks of a face and how many pixels
    /// a millimetre covers there; [`grant`](Self::grant) settles the levels from the
    /// counts; and the kernel then makes the one mesh, with no second plan. When the
    /// grant does not fit, the kernel makes no mesh at all.
    pub fn mesh_faces(
        &self,
        held: Usage,
        kernel: &dyn Kernel,
        body: BodyId,
        levels: &FaceLevels,
        want: impl FnMut(FaceId) -> (Level, f64),
    ) -> KernelResult<Planned> {
        self.plan_faces(held, kernel, body, levels, want, false)
    }

    /// Meshes a single part at the finest of its levels the budget has room for: planned
    /// at `from` and, while [`mesh_faces`](Self::mesh_faces) finds a plan over the
    /// budget, at the next coarser level, each time before any mesh is made. Returns the
    /// level the part was planned at with its mesh. A part over the budget even at its
    /// coarse level is meshed there all the same, so it can be drawn, and its
    /// [`FaceMesh::fits`] says so. `None` when the kernel plans no face levels for the
    /// body.
    pub fn mesh_part(
        &self,
        held: Usage,
        kernel: &dyn Kernel,
        body: BodyId,
        levels: &Levels,
        from: Level,
        mut want: impl FnMut(FaceId) -> (Level, f64),
    ) -> KernelResult<Option<(Level, FaceMesh)>> {
        let mut level = from;
        loop {
            let last = level.coarser().is_none();
            match self.plan_faces(held, kernel, body, &levels.faces_at(level), &mut want, last)? {
                Planned::Whole => return Ok(None),
                Planned::Mesh(mesh) => return Ok(Some((level, *mesh))),
                Planned::Over(_) => match level.coarser() {
                    Some(coarser) => level = coarser,
                    // Not reached: the coarse plan is meshed even when it is over.
                    None => return Ok(None),
                },
            }
        }
    }

    /// [`mesh_faces`](Self::mesh_faces); with `over` set, a grant that does not fit is
    /// meshed all the same.
    fn plan_faces(
        &self,
        held: Usage,
        kernel: &dyn Kernel,
        body: BodyId,
        levels: &FaceLevels,
        mut want: impl FnMut(FaceId) -> (Level, f64),
        over: bool,
    ) -> KernelResult<Planned> {
        let mut settled = None;
        let mesh = kernel.face_level_mesh(body, levels.params(), &mut |counts: &[FaceLevelPoints]| {
            let demands: Vec<FaceDemand> = counts
                .iter()
                .map(|count| {
                    let (wanted, pixels_per_mm) = want(count.face);
                    FaceDemand { points: count.points, wanted, pixels_per_mm }
                })
                .collect();
            let granted = self.grant(held, levels, &demands);
            let chosen =
                (granted.fits || over).then(|| granted.levels.iter().map(|level| FaceLevel::from(*level)).collect());
            settled = Some((counts.iter().map(|count| count.face).collect::<Vec<_>>(), granted));
            chosen
        })?;
        Ok(match (settled, mesh) {
            (None, _) => Planned::Whole,
            (Some((_, granted)), None) => Planned::Over(granted),
            (Some((faces, granted)), Some(mesh)) => Planned::Mesh(Box::new(FaceMesh {
                mesh,
                levels: faces.into_iter().zip(granted.levels).collect(),
                points: granted.points,
                fits: granted.fits,
            })),
        })
    }

    /// The larger of the triangle and byte shares `usage` takes of the budget.
    pub fn share(&self, usage: Usage) -> f64 {
        let part = |used: u64, limit: u64| if limit == 0 { f64::INFINITY } else { used as f64 / limit as f64 };
        part(usage.triangles, self.triangles).max(part(usage.bytes, self.bytes))
    }
}

impl Default for DeviceBudget {
    fn default() -> DeviceBudget {
        // Headless callers have no adapter. Use WebGPU's baseline buffer limit.
        DeviceBudget::from_buffer_limit(256 << 20)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceiling_drops_as_the_budget_fills() {
        let b = DeviceBudget { triangles: 1000, bytes: 1 << 20 };
        assert_eq!(b.ceiling(Usage::default()), Level::Fine);
        assert_eq!(b.ceiling(Usage { triangles: 799, bytes: 0 }), Level::Fine);
        assert_eq!(b.ceiling(Usage { triangles: 800, bytes: 0 }), Level::Standard);
        assert_eq!(b.ceiling(Usage { triangles: 0, bytes: 1 << 20 }), Level::Coarse, "bytes count too");
        assert!(b.fits(Usage { triangles: 1000, bytes: 1 << 20 }));
        assert!(!b.fits(Usage { triangles: 1001, bytes: 0 }));
        assert_eq!(DeviceBudget::UNLIMITED.ceiling(Usage { triangles: u64::MAX / 2, bytes: 0 }), Level::Fine);
        assert_eq!(DeviceBudget { triangles: 0, bytes: 0 }.ceiling(Usage::default()), Level::Coarse);
    }

    /// A part 100 mm across: a face is sampled to 0.5, 0.1 or 0.05 mm.
    fn levels() -> FaceLevels {
        FaceLevels::new(&crate::quality::Quality::Auto, 100.0)
    }

    /// Room for exactly `points` counted points.
    fn room(points: u64) -> DeviceBudget {
        DeviceBudget { triangles: 2 * points, bytes: POINT_BYTES * points }
    }

    fn face(points: [u32; 3], wanted: Level, pixels_per_mm: f64) -> FaceDemand {
        FaceDemand { points, wanted, pixels_per_mm }
    }

    #[test]
    fn counted_points_bound_the_display_data() {
        assert_eq!(Usage::of_points(1000), Usage { triangles: 2000, bytes: 64_000 });
        assert_eq!(Usage::of_points(u64::MAX).triangles, u64::MAX);
        let b = DeviceBudget { triangles: 1000, bytes: 64_000 };
        assert_eq!(b.points_left(Usage::default()), 500, "the triangle limit is the tighter one");
        assert_eq!(b.points_left(Usage { triangles: 0, bytes: 64_000 - 640 }), 10);
        assert_eq!(b.points_left(Usage { triangles: 5000, bytes: 0 }), 0);
        assert!(b.fits(Usage::of_points(b.points_left(Usage::default()))));
    }

    #[test]
    fn every_face_gets_what_it_wants_when_the_count_fits() {
        let faces = [face([10, 40, 160], Level::Fine, 10.0), face([4, 4, 4], Level::Standard, 10.0)];
        let g = room(164).grant(Usage::default(), &levels(), &faces);
        assert_eq!(g, Granted { levels: vec![Level::Fine, Level::Standard], points: 164, fits: true });
        assert_eq!(DeviceBudget::UNLIMITED.grant(Usage::default(), &levels(), &faces), g);
    }

    #[test]
    fn a_smaller_budget_steps_faces_down_until_the_count_fits() {
        let l = levels();
        // Four equal faces close up, one far away and one out of view, all wanting fine.
        let mut faces = vec![face([10, 40, 160], Level::Fine, 10.0); 4];
        faces.push(face([10, 40, 160], Level::Fine, 1.0));
        faces.push(face([10, 40, 160], Level::Fine, 0.0));
        let wanted: u64 = 6 * 160;
        let mut last = wanted;
        for points in [wanted, 800, 700, 500, 300, 100, 60] {
            let g = room(points).grant(Usage::default(), &l, &faces);
            assert!(g.fits && g.points <= points, "{points}: {g:?}");
            assert_eq!(g.points, g.levels.iter().map(|level| [10, 40, 160][level.index()]).sum::<u64>());
            assert!(g.points <= last, "a smaller budget never grants more points");
            assert!(g.levels[5] <= g.levels[4] && g.levels[4] <= g.levels[0], "{points}: {g:?}");
            last = g.points;
        }
        // 800 points: the face out of view gives up fine and then standard, and the far
        // face gives up fine. Nothing close up is touched.
        let g = room(800).grant(Usage::default(), &l, &faces);
        assert_eq!(g.levels, [Level::Fine, Level::Fine, Level::Fine, Level::Fine, Level::Standard, Level::Coarse]);
        assert_eq!(g.points, 4 * 160 + 40 + 10);
        // Display data held elsewhere counts against the same budget.
        let held = Usage::of_points(160);
        assert_eq!(room(wanted).grant(held, &l, &faces), room(800).grant(Usage::default(), &l, &faces));
        // Too small for even the coarsest levels: all coarse, and it says so.
        let g = room(59).grant(Usage::default(), &l, &faces);
        assert_eq!((g.levels, g.points, g.fits), (vec![Level::Coarse; 6], 60, false));
    }

    #[test]
    fn a_face_never_steps_to_a_level_that_saves_nothing() {
        let l = levels();
        // A planar face has the same points at every level; a wall has the same at its
        // two coarser ones.
        let faces = [face([4, 4, 4], Level::Fine, 10.0), face([50, 50, 400], Level::Fine, 10.0)];
        let g = room(100).grant(Usage::default(), &l, &faces);
        assert_eq!(g, Granted { levels: vec![Level::Fine, Level::Standard], points: 54, fits: true });
        let g = room(10).grant(Usage::default(), &l, &faces);
        assert_eq!(g, Granted { levels: vec![Level::Fine, Level::Standard], points: 54, fits: false });
        // A face that skips a level steps straight to the one that saves points.
        let skip = [face([10, 90, 90], Level::Fine, 10.0)];
        assert_eq!(room(20).grant(Usage::default(), &l, &skip).levels, [Level::Coarse]);
    }
}
