//! Feature recognition through Catallax, on the app's face ids.

mod analytic;

use std::collections::BTreeSet;

use catallax_measure::recognition as cx;
use soliddraft_kernel_api::recognition::{Boss, Hole, Recognised, Slot};
use soliddraft_kernel_api::{FaceId, KernelError, KernelResult};

use crate::{Solid, numerical};

/// The holes, hole patterns, bosses and slots of a solid.
pub(super) fn recognise(solid: &Solid) -> KernelResult<Recognised> {
    let built = solid.app.checkpoint.built();
    let (found, mut out) = numerical("recognise", |dec| {
        let basic = cx::recognise(&built.model, built.result, dec)?;
        let analytic = analytic::recognise(&built.model, &solid.ids, dec);
        Ok(catallax_numbers::Answer::exact((basic, analytic)))
    })
    .map_err(|refusal| crate::query_error("recognise", refusal))?
    .value;
    let face =
        |cat: &catallax_topology::FaceId| -> KernelResult<FaceId> {
            solid.ids.faces.get(cat).copied().ok_or_else(|| {
                KernelError::Unavailable("a recognised feature is on a face the body does not list".into())
            })
        };
    let faces = |cats: &[catallax_topology::FaceId]| cats.iter().map(face).collect::<KernelResult<Vec<FaceId>>>();
    let compound_walls: BTreeSet<_> =
        out.compound_holes.iter().flat_map(|hole| hole.pilot_faces.iter().chain(&hole.head_faces).copied()).collect();
    let mut indices = Vec::with_capacity(found.holes.len());
    for hole in &found.holes {
        let wall = faces(&hole.faces)?;
        if wall.iter().any(|id| compound_walls.contains(id)) {
            indices.push(None);
        } else {
            indices.push(Some(out.holes.len()));
            out.holes.push(Hole {
                faces: wall,
                diameter: hole.diameter,
                through: hole.through,
                depth: hole.depth,
                opens_onto: faces(&hole.opens_onto)?,
                bottoms: faces(&hole.bottoms)?,
            });
        }
    }
    for pattern in &found.hole_patterns {
        let members: Vec<_> = pattern.iter().filter_map(|&index| indices[index]).collect();
        if members.len() > 1 {
            out.hole_patterns.push(members);
        }
    }
    out.bosses = found
        .bosses
        .iter()
        .map(|boss| {
            Ok(Boss {
                faces: faces(&boss.faces)?,
                diameter: boss.diameter,
                height: boss.height,
                base: face(&boss.base)?,
                top: face(&boss.top)?,
            })
        })
        .collect::<KernelResult<_>>()?;
    out.slots = found
        .slots
        .iter()
        .map(|slot| {
            Ok(Slot {
                flats: [face(&slot.flats[0])?, face(&slot.flats[1])?],
                ends: [faces(&slot.ends[0])?, faces(&slot.ends[1])?],
                width: slot.width,
                length: slot.length,
                through: slot.through,
            })
        })
        .collect::<KernelResult<_>>()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use catallax_shapes::Surface;
    use soliddraft_kernel_api::*;

    use crate::CatallaxKernel;

    fn block(k: &CatallaxKernel, tag: &str, from: Vec3, width: f64, height: f64, depth: f64) -> BodyId {
        let plane = Plane::XY.with_origin(from);
        let profile = Profile::rectangle(plane, Point2::new(0.0, 0.0), width, height);
        let sheet = k.create_body_from_profile(&OpTag::new(tag), &profile).unwrap().body;
        k.extrude(&OpTag::new(tag), sheet, &Extrude { distance: depth, direction: None }).unwrap().body
    }

    fn hole(
        k: &CatallaxKernel,
        body: BodyId,
        x: f64,
        y: f64,
        diameter: f64,
        depth: HoleDepth,
        head: Option<HoleHead>,
    ) -> BodyId {
        k.hole(
            &OpTag::new("drilled"),
            body,
            &Hole {
                placements: vec![HolePlacement { point: Vec3::new(x, y, 10.0), direction: -Vec3::Z, on_face: None }],
                diameter,
                depth,
                tip: HoleTip::Flat,
                head,
            },
        )
        .unwrap()
        .body
    }

    fn bare_step(k: &CatallaxKernel, body: BodyId) -> (CatallaxKernel, BodyId) {
        // Only STEP geometry crosses to the new kernel; no recipe or PMI is supplied.
        let step = k.export_step(body, &StepOptions::default()).unwrap();
        let source = step.text.as_bytes();
        let imported = CatallaxKernel::new();
        let result = imported
            .import_step(
                &OpTag::new("bare"),
                &ImportStepRequest {
                    source_solid: None,
                    name: "bare.step",
                    source,
                    sha256: &catallax_import::sha256_hex(source),
                    options: &ImportOptions::default(),
                    decisions: &[],
                    record: None,
                },
            )
            .unwrap();
        let body = result.bodies[0].result.body;
        assert_eq!(imported.body_accuracy(body).unwrap(), BodyAccuracy::Exact);
        (imported, body)
    }

    fn close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-8, "{actual} != {expected}");
    }

    fn planar_z(k: &CatallaxKernel, body: BodyId, face: FaceId, expected: f64) {
        let solid = k.solid(body).unwrap();
        let model = &solid.app.checkpoint.built().model;
        let native = solid.ids.faces.iter().find(|(_, id)| **id == face).unwrap().0;
        let Surface::Plane(plane) = model.geom().surface(model.face(*native).surface) else {
            panic!("expected plane on {face:?}");
        };
        close(plane.normal.x.to_f64(), 0.0);
        close(plane.normal.y.to_f64(), 0.0);
        close(plane.origin.z.to_f64(), expected);
    }

    fn round_faces(k: &CatallaxKernel, body: BodyId, x: f64, radius: f64, cone: bool) -> Vec<FaceId> {
        let solid = k.solid(body).unwrap();
        let model = &solid.app.checkpoint.built().model;
        solid
            .ids
            .faces
            .iter()
            .filter_map(|(&native, &id)| {
                let matches = match model.geom().surface(model.face(native).surface) {
                    Surface::Cylinder(c) => {
                        !cone && c.origin.x.to_f64() == x && c.radius_sq.to_f64() == radius * radius
                    }
                    Surface::Cone(c) => cone && c.apex.x.to_f64() == x,
                    _ => false,
                };
                matches.then_some(id)
            })
            .collect()
    }

    #[test]
    fn bare_step_compound_holes_keep_wall_membership_and_axial_depth() {
        use soliddraft_kernel_api::recognition::CompoundHead;
        let k = CatallaxKernel::new();
        let plate = block(&k, "plate", Vec3::ZERO, 70.0, 40.0, 10.0);
        let body = hole(
            &k,
            plate,
            -22.0,
            0.0,
            4.0,
            HoleDepth::Blind { depth: 7.0 },
            Some(HoleHead::Counterbore { diameter: 10.0, depth: 2.0 }),
        );
        let body = hole(
            &k,
            body,
            22.0,
            0.0,
            4.0,
            HoleDepth::Through,
            Some(HoleHead::Countersink { diameter: 8.0, angle_deg: 90.0 }),
        );
        let body = hole(&k, body, 0.0, -10.0, 4.0, HoleDepth::Through, None);
        let body = hole(&k, body, 0.0, 10.0, 4.0, HoleDepth::Blind { depth: 6.0 }, None);

        let (k, body) = bare_step(&k, body);
        let got = k.recognise(body).unwrap();
        assert_eq!(got.compound_holes.len(), 2, "{got:#?}");
        assert_eq!(got.holes.len(), 2, "{got:#?}");
        assert!(got.holes.iter().any(|h| h.through && h.depth == Some(10.0)));
        assert!(got.holes.iter().any(|h| !h.through && h.depth == Some(6.0)));
        for f in &got.compound_holes {
            let x = f.mouth_center.x;
            assert_eq!(f.mouth_center, Vec3::new(x, 0.0, 10.0));
            assert_eq!(f.direction, -Vec3::Z);
            close(f.pilot_diameter, 4.0);
            assert_eq!(f.pilot_faces, round_faces(&k, body, x, 2.0, false));
            assert!(!f.pilot_faces.is_empty());
            planar_z(&k, body, f.entry, 10.0);
            match f.head {
                CompoundHead::Counterbore { shoulder } => {
                    assert_eq!(x, -22.0);
                    assert!(!f.through);
                    close(f.head_diameter, 10.0);
                    close(f.depth, 7.0);
                    planar_z(&k, body, shoulder, 8.0);
                    planar_z(&k, body, f.end, 3.0);
                    assert_eq!(f.head_faces, round_faces(&k, body, x, 5.0, false));
                }
                CompoundHead::Countersink { angle_deg } => {
                    assert_eq!(x, 22.0);
                    assert!(f.through);
                    close(f.head_diameter, 8.0);
                    close(f.depth, 10.0);
                    close(angle_deg, 90.0);
                    planar_z(&k, body, f.end, 0.0);
                    assert_eq!(f.head_faces, round_faces(&k, body, x, 4.0, true));
                }
            }
            assert!(!f.head_faces.is_empty());
            close(f.head_depth, 2.0);
        }
    }

    #[test]
    fn bare_step_distinguishes_closed_pocket_and_open_step() {
        let k = CatallaxKernel::new();
        let plate = block(&k, "plate", Vec3::ZERO, 60.0, 30.0, 10.0);
        let pocket = block(&k, "pocket", Vec3::new(-14.0, 0.0, 8.0), 12.0, 8.0, 4.0);
        let step = block(&k, "step", Vec3::new(26.0, 0.0, 7.0), 16.0, 12.0, 5.0);
        let body = k.boolean(&OpTag::new("pocket"), BooleanOp::Subtract, plate, pocket).unwrap().body;
        let body = k.boolean(&OpTag::new("step"), BooleanOp::Subtract, body, step).unwrap().body;
        let (k, body) = bare_step(&k, body);
        let got = k.recognise(body).unwrap();
        assert_eq!(got.recesses.len(), 2, "{got:#?}");
        assert_eq!(got.recesses.iter().filter(|f| f.open_sided).count(), 1);
        for f in &got.recesses {
            let (depth, walls): (_, &[(u8, f64)]) = if f.open_sided {
                (3.0, &[(0, 18.0), (1, -6.0), (1, 6.0)])
            } else {
                (2.0, &[(0, -20.0), (0, -8.0), (1, -4.0), (1, 4.0)])
            };
            close(f.depth, depth);
            planar_z(&k, body, f.floor, 10.0 - depth);
            assert_eq!(f.opens_onto.len(), 1);
            planar_z(&k, body, f.opens_onto[0], 10.0);
            let solid = k.solid(body).unwrap();
            let model = &solid.app.checkpoint.built().model;
            let mut actual: Vec<(u8, f64)> = f
                .walls
                .iter()
                .map(|id| {
                    let native = *solid.ids.faces.iter().find(|(_, face)| *face == id).unwrap().0;
                    let Surface::Plane(plane) = model.geom().surface(model.face(native).surface) else {
                        panic!("expected wall plane");
                    };
                    close(plane.normal.z.to_f64(), 0.0);
                    if plane.normal.x.to_f64() != 0.0 {
                        (0, plane.origin.x.to_f64())
                    } else {
                        (1, plane.origin.y.to_f64())
                    }
                })
                .collect();
            actual.sort_by(|(axis, at), (other_axis, other_at)| {
                axis.cmp(other_axis).then_with(|| at.total_cmp(other_at))
            });
            assert_eq!(actual, walls);
        }
    }

    fn vertical_edge(k: &CatallaxKernel, body: BodyId, x: f64, y: f64) -> EdgeId {
        let solid = k.solid(body).unwrap();
        let model = &solid.app.checkpoint.built().model;
        solid
            .ids
            .edges
            .iter()
            .find_map(|(&edge, &id)| {
                let store = model.geom();
                let edge = model.edge(edge);
                let point = |v| store.position(model.vertex(v).point);
                let [a, b] = [point(edge.start?), point(edge.end?)];
                (a.x.to_f64() == x
                    && b.x.to_f64() == x
                    && a.y.to_f64() == y
                    && b.y.to_f64() == y
                    && a.z.to_f64() != b.z.to_f64())
                .then_some(id)
            })
            .unwrap()
    }

    #[test]
    fn bare_step_edge_treatments_require_two_proved_supports() {
        use soliddraft_kernel_api::recognition::EdgeTreatmentKind;
        let k = CatallaxKernel::new();
        let plate = block(&k, "plate", Vec3::ZERO, 40.0, 30.0, 10.0);
        let edge = vertical_edge(&k, plate, 20.0, 15.0);
        let body = k.fillet(&OpTag::new("round"), plate, &[edge], 2.0).unwrap().body;
        let edge = vertical_edge(&k, body, -20.0, -15.0);
        let body = k.chamfer(&OpTag::new("bevel"), body, &[edge], 1.0).unwrap().body;
        let edge = vertical_edge(&k, body, -20.0, 15.0);
        let body = k.chamfer_distances(&OpTag::new("unequal"), body, &[edge], 1.0, Some(2.0)).unwrap().body;
        let (k, body) = bare_step(&k, body);
        let got = k.recognise(body).unwrap();
        assert_eq!(got.edge_treatments.len(), 2, "{got:#?}");
        let solid = k.solid(body).unwrap();
        let model = &solid.app.checkpoint.built().model;
        let surface = |face| {
            let native = *solid.ids.faces.iter().find(|(_, id)| **id == face).unwrap().0;
            model.geom().surface(model.face(native).surface)
        };
        for f in &got.edge_treatments {
            let (x, y) = match f.kind {
                EdgeTreatmentKind::Fillet { radius } => {
                    close(radius, 2.0);
                    let Surface::Cylinder(cyl) = surface(f.face) else { panic!("not a cylinder") };
                    close(cyl.origin.x.to_f64(), 18.0);
                    close(cyl.origin.y.to_f64(), 13.0);
                    (20.0, 15.0)
                }
                EdgeTreatmentKind::Chamfer { setback } => {
                    close(setback, 1.0);
                    assert!(matches!(surface(f.face), Surface::Plane(_)));
                    (-20.0, -15.0)
                }
            };
            let mut supports = f.supports.map(|face| {
                let Surface::Plane(plane) = surface(face) else { panic!("not a support plane") };
                if plane.normal.x.to_f64() != 0.0 { (0, plane.origin.x.to_f64()) } else { (1, plane.origin.y.to_f64()) }
            });
            supports.sort_by_key(|p| p.0);
            assert_eq!(supports, [(0, x), (1, y)]);
        }
    }

    #[test]
    fn nontangent_cylinder_is_not_a_fillet() {
        let k = CatallaxKernel::new();
        let plate = block(&k, "plate", Vec3::ZERO, 40.0, 30.0, 10.0);
        let cutter = contract::cylinder(&k, Vec3::new(20.0, -15.0, -1.0), 3.0, 12.0);
        let body = k.boolean(&OpTag::new("corner cut"), BooleanOp::Subtract, plate, cutter).unwrap().body;
        assert!(!round_faces(&k, body, 20.0, 3.0, false).is_empty());
        let got = k.recognise(body).unwrap();
        assert!(got.edge_treatments.is_empty(), "{got:#?}");
    }

    #[test]
    fn bare_step_offset_head_is_not_a_compound_hole() {
        let k = CatallaxKernel::new();
        let plate = block(&k, "plate", Vec3::ZERO, 40.0, 30.0, 10.0);
        let body = hole(&k, plate, 0.0, 0.0, 4.0, HoleDepth::Through, None);
        let body = hole(&k, body, 2.5, 0.0, 10.0, HoleDepth::Blind { depth: 2.0 }, None);
        let (k, body) = bare_step(&k, body);
        let got = k.recognise(body).unwrap();
        assert!(got.compound_holes.is_empty(), "{got:#?}");
        assert!(!round_faces(&k, body, 0.0, 2.0, false).is_empty());
        assert!(!round_faces(&k, body, 2.5, 5.0, false).is_empty());
    }
}
