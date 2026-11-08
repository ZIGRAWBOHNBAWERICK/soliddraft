//! Static alignment of bore axes read from pinned native cylinder definitions.

use super::{CheckCode, CheckRefusal, CheckRequest, CheckScope, SkippedPart, from_mate, invalid, seed};
use crate::AssemblyDocument;
use crate::mates::{bodies::Scope, solver};
use crate::{OccurrencePath, Placement};
use num_rational::BigRational;
use num_traits::Zero;
use serde::Serialize;
use soliddraft_document::{ContentHash, DocKind, DocRef};
use soliddraft_kernel_api::{ExactAxis, ExactSupport, SupportEntity, SurfaceKind};

/// Maximum distinct bore axes in one check, bounding the pairwise exact comparisons.
pub const MAX_HOLE_AXES: usize = 512;

/// Which parts to inspect and how far apart parallel hole axes may be to be listed.
#[derive(Clone, Debug, PartialEq)]
pub struct HoleAlignmentRequest {
    /// Whole assembly or selected occurrences, including their descendants.
    pub scope: CheckScope,
    /// Include hidden parts. Suppressed parts never take part.
    pub include_hidden: bool,
    /// Largest lateral axis deviation to list, in millimetres. Zero lists coaxial axes only.
    pub max_deviation_mm: f64,
}

impl Default for HoleAlignmentRequest {
    fn default() -> Self {
        Self { scope: CheckScope::Assembly, include_hidden: false, max_deviation_mm: 1.0 }
    }
}

/// One distinct internal cylindrical axis, at the revision the assembly pins.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HoleAxis {
    /// Part occurrence.
    pub path: OccurrencePath,
    /// Part label in the tree.
    pub label: String,
    /// Representative cylindrical face label. Coaxial faces share one entry.
    pub face: String,
    /// Revision read for the axis.
    pub revision: ContentHash,
    /// World axis index, zero for X, one for Y, two for Z.
    pub axis: usize,
    /// Exact rational coordinates of a point on the axis in the root frame.
    pub through: [String; 3],
}

/// Whether two parallel bore axes lie on one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoleOutcome {
    /// The axes lie on exactly the same line.
    Aligned,
    /// The axes are parallel but offset by no more than the requested deviation.
    Misaligned,
}

/// A pair of bore axes from different parts, within the requested deviation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HolePair {
    /// Index of the first axis in the report's `holes` list.
    pub a: usize,
    /// Index of the second axis.
    pub b: usize,
    /// Exact classification.
    pub outcome: HoleOutcome,
    /// Exact squared lateral distance in square millimetres.
    pub deviation_squared: String,
    /// Rounded distance for display only. Classification uses `deviation_squared`.
    pub deviation_mm: f64,
}

/// Read-only Hole Alignment results. This tests infinite axes, not hole diameter or fit.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HoleAlignmentReport {
    /// `hole_alignment`.
    pub kind: &'static str,
    /// `exact_axes`, independent of the certified-mesh contact tier.
    pub tier: &'static str,
    /// Requested maximum lateral deviation in millimetres.
    pub max_deviation_mm: f64,
    /// Number of parts whose cylinders were inspected, including parts with no holes.
    pub parts: usize,
    /// Every distinct proven bore axis.
    pub holes: Vec<HoleAxis>,
    /// Parts whose cylinders could not all be read exactly.
    pub skipped: Vec<SkippedPart>,
    /// True only when every part in scope was inspected.
    pub complete: bool,
    /// Parallel axis pairs within the requested deviation, misaligned pairs first.
    pub results: Vec<HolePair>,
    /// Exactly coaxial pairs.
    pub aligned: usize,
    /// Parallel pairs with a nonzero lateral offset within the requested deviation.
    pub misaligned: usize,
    /// Pairs with different directions, outside this check's parallel-axis test.
    pub nonparallel: usize,
    /// Parallel pairs beyond the requested deviation.
    pub beyond: usize,
}

struct Axis {
    info: HoleAxis,
    through: [BigRational; 3],
}

/// Apply the stored quaternion in rationals, including its norm, as mate proofs do.
fn placed(axis: &ExactAxis, at: Placement) -> Option<(usize, [BigRational; 3])> {
    let direction = solver::turned(at.rotation, axis.direction())?;
    let k = direction.iter().position(|v| *v != 0)?;
    let point: Vec<_> = axis.through.iter().map(solver::rational).collect::<Option<_>>()?;
    let q = at.rotation.to_array().map(solver::exact);
    let [Some(x), Some(y), Some(z), Some(w)] = q else { return None };
    let b = [x, y, z];
    let bb: BigRational = b.iter().map(|v| v * v).sum();
    let ww = &w * &w;
    let bv: BigRational = b.iter().zip(&point).map(|(a, b)| a * b).sum();
    let cross = [
        &b[1] * &point[2] - &b[2] * &point[1],
        &b[2] * &point[0] - &b[0] * &point[2],
        &b[0] * &point[1] - &b[1] * &point[0],
    ];
    let norm = &ww + &bb;
    if norm.is_zero() {
        return None;
    }
    let two = BigRational::from_integer(2.into());
    let t: Vec<_> = at.translation.to_array().into_iter().map(solver::exact).collect::<Option<_>>()?;
    let through =
        std::array::from_fn(|i| ((&ww - &bb) * &point[i] + &two * &bv * &b[i] + &two * &w * &cross[i]) / &norm + &t[i]);
    Some((k, through))
}

fn distance_squared(a: &Axis, b: &Axis) -> BigRational {
    (0..3)
        .filter(|k| *k != a.info.axis)
        .map(|k| {
            let d = &a.through[k] - &b.through[k];
            &d * &d
        })
        .sum()
}

impl AssemblyDocument {
    /// Compare internal cylinder axes from the pinned parts without changing the assembly.
    pub fn check_hole_alignment(
        &mut self,
        request: &HoleAlignmentRequest,
    ) -> Result<HoleAlignmentReport, CheckRefusal> {
        let limit = solver::exact(request.max_deviation_mm)
            .filter(|v| *v >= BigRational::zero())
            .ok_or_else(|| invalid("a maximum hole deviation is a finite length of zero or more"))?;
        self.finish_loading();
        let scope = CheckRequest {
            scope: request.scope.clone(),
            include_hidden: request.include_hidden,
            ..CheckRequest::default()
        };
        let (entries, mut skipped) = self.scope_parts(&scope)?;
        if entries.len() < 2 {
            return Err(CheckRefusal::new(CheckCode::TooFewParts, "Hole Alignment compares two parts or more"));
        }
        if entries.len() > super::MAX_PARTS {
            return Err(CheckRefusal::new(CheckCode::TooLarge, "Hole Alignment takes at most 128 parts"));
        }
        let mut axes = Vec::new();
        let mut parts = 0;
        for entry in entries {
            let source = (|| {
                if self.store.get(self.table.definition(entry.row)).kind == DocKind::Mesh {
                    return Err((
                        CheckCode::MeshReference,
                        "a mesh reference has no native cylinder definitions".into(),
                    ));
                }
                let scope = Scope::new(&self.assembly, &self.table, &self.store);
                let path = seed(&entry.path);
                let r = scope.part_source(&path).map_err(from_mate)?;
                Ok(self.rebased_source(&r, &scope.chain(&path)))
            })();
            let found = source
                .and_then(|source| self.hole_axes(&source, self.table.world(entry.row), &entry.path, &entry.label));
            match found {
                Ok(found) => {
                    parts += 1;
                    axes.extend(found);
                }
                Err((code, message)) => {
                    skipped.push(SkippedPart { path: entry.path, label: entry.label, code, message })
                }
            }
            if axes.len() > MAX_HOLE_AXES {
                return Err(CheckRefusal::new(
                    CheckCode::TooLarge,
                    format!("Hole Alignment takes at most {MAX_HOLE_AXES} distinct bore axes"),
                ));
            }
        }
        let mut report = HoleAlignmentReport {
            kind: "hole_alignment",
            tier: "exact_axes",
            max_deviation_mm: request.max_deviation_mm,
            parts,
            holes: axes.iter().map(|a| a.info.clone()).collect(),
            complete: skipped.is_empty(),
            skipped,
            results: Vec::new(),
            aligned: 0,
            misaligned: 0,
            nonparallel: 0,
            beyond: 0,
        };
        let limit2 = &limit * &limit;
        for i in 0..axes.len() {
            for j in i + 1..axes.len() {
                let (a, b) = (&axes[i], &axes[j]);
                if a.info.path == b.info.path {
                    continue;
                }
                if a.info.axis != b.info.axis {
                    report.nonparallel += 1;
                    continue;
                }
                let d2 = distance_squared(a, b);
                if d2 > limit2 {
                    report.beyond += 1;
                    continue;
                }
                let outcome = if d2.is_zero() {
                    report.aligned += 1;
                    HoleOutcome::Aligned
                } else {
                    report.misaligned += 1;
                    HoleOutcome::Misaligned
                };
                report.results.push(HolePair {
                    a: i,
                    b: j,
                    outcome,
                    deviation_mm: (0..3)
                        .filter(|k| *k != a.info.axis)
                        .map(|k| solver::nearest(&(&a.through[k] - &b.through[k])))
                        .fold(0.0_f64, f64::hypot),
                    deviation_squared: solver::text(&d2),
                });
            }
        }
        report.results.sort_by_key(|p| (p.outcome == HoleOutcome::Aligned, p.a, p.b));
        Ok(report)
    }

    fn hole_axes(
        &mut self,
        source: &DocRef,
        at: Placement,
        path: &OccurrencePath,
        label: &str,
    ) -> Result<Vec<Axis>, (CheckCode, String)> {
        let part = self.proof_part(source).map_err(from_mate)?;
        let part = part.lock().expect("the part lock");
        let body = part.body().ok_or((CheckCode::ProofUnavailable, "the pinned part has no solid body".into()))?;
        let topology = part.kernel().topology(body).map_err(|e| (CheckCode::UnsupportedBody, e.to_string()))?;
        let mut found: Vec<Axis> = Vec::new();
        for face in topology.faces.iter().filter(|f| f.surface == SurfaceKind::Cylinder) {
            let name = part.face_label(face.id).map(|l| l.text).unwrap_or_else(|| format!("Face {}", face.id.raw()));
            let refuse = |why| (CheckCode::UnsupportedBody, format!("{name}: {why}"));
            let support =
                part.kernel().entity_support(body, SupportEntity::Face(face.id)).map_err(|e| refuse(e.to_string()))?;
            let ExactSupport::Axis(axis) = support else {
                return Err(refuse("the cylinder has no exact axis".into()));
            };
            match axis.radius.as_ref().and_then(|r| r.outward) {
                Some(true) => continue,
                Some(false) => {}
                None => return Err(refuse("the cylinder's material side is unknown".into())),
            }
            let (k, through) =
                placed(&axis, at).ok_or_else(|| refuse("the placed axis is not exactly on a world axis".into()))?;
            if found.iter().any(|a| a.info.axis == k && (0..3).all(|i| i == k || a.through[i] == through[i])) {
                continue;
            }
            let info = HoleAxis {
                path: path.clone(),
                label: label.into(),
                face: name,
                revision: source.pinned.clone(),
                axis: k,
                through: through.each_ref().map(solver::text),
            };
            found.push(Axis { info, through });
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{DQuat, DVec3};
    use soliddraft_kernel_api::{Axis as WorldAxis, ExactRational, SupportSource};

    fn axis() -> ExactAxis {
        ExactAxis {
            axis: WorldAxis::Z,
            positive: true,
            through: [2, 3, 4].map(|v| ExactRational { numerator: v.to_string(), denominator: "1".into() }),
            radius: None,
            source: SupportSource::Cylinder,
        }
    }

    #[test]
    fn places_axes_exactly_and_refuses_a_tiny_tilt() {
        let at = Placement::new(DQuat::from_rotation_x(std::f64::consts::FRAC_PI_2), DVec3::new(10.0, 20.0, 30.0));
        let (k, p) = placed(&axis(), at).unwrap();
        assert_eq!(k, 1);
        assert_eq!(p.each_ref().map(solver::text), ["12", "16", "33"]);
        let tilted = Placement { rotation: DQuat::from_rotation_x(2.0_f64.powi(-42)), translation: DVec3::ZERO };
        assert!(placed(&axis(), tilted).is_none());
        let spun = Placement::new(DQuat::from_rotation_z(0.37), DVec3::ZERO);
        assert!(placed(&axis(), spun).is_some());
    }
}
