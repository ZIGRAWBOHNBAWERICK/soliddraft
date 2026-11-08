//! Exact support planes, axes and points read from Catallax's native definitions.
//!
//! A cylinder keeps its axis origin, direction and squared radius, a cone its apex and
//! direction, a sphere its centre and radius, a circle its centre, normal and squared
//! radius, a line its origin and direction, and an explicit vertex its point. Each is read
//! as it is stored. The exact reader refuses non-rational values and off-axis directions.
//! The separate enclosed reader bounds those definitions through Catallax's Decider.

use catallax_numbers::{Rational, Real, Sign};
use catallax_shapes::{Curve, Point3, PointDef, Surface, Vector3};
use catallax_topology::Sense;
use soliddraft_kernel_api::*;

use crate::CatallaxKernel;

fn refused(what: String) -> KernelError {
    KernelError::Unsupported(format!("proven support plane: {what}"))
}

/// A refusal of an exact axis or point.
fn unsupported(what: String) -> KernelError {
    KernelError::Unsupported(format!("exact support: {what}"))
}

fn exact(value: &Rational) -> ExactRational {
    ExactRational { numerator: value.numerator().to_string(), denominator: value.denominator().to_string() }
}

/// The one nonzero component of a native normal, proven with exact arithmetic.
fn axis_of(normal: [&Rational; 3]) -> Option<(Axis, &Rational)> {
    let mut nonzero = [Axis::X, Axis::Y, Axis::Z].into_iter().zip(normal).filter(|(_, c)| !c.is_zero());
    let found = nonzero.next()?;
    nonzero.next().is_none().then_some(found)
}

pub(super) fn face_support_plane(k: &CatallaxKernel, body: BodyId, face: FaceId) -> KernelResult<ExactAxisPlane> {
    let solid = k.solid(body)?;
    let id = solid
        .ids
        .faces
        .iter()
        .find_map(|(id, f)| (*f == face).then_some(*id))
        .ok_or(KernelError::UnknownFace { body, face })?;
    let model = &solid.app.checkpoint.built().model;
    let native = model.face(id);
    let Surface::Plane(plane) = model.geom().surface(native.surface) else {
        return Err(refused(format!("{face} is not planar")));
    };
    let (Some(nx), Some(ny), Some(nz), Some(ox), Some(oy), Some(oz)) = (
        rational(&plane.normal.x),
        rational(&plane.normal.y),
        rational(&plane.normal.z),
        rational(&plane.origin.x),
        rational(&plane.origin.y),
        rational(&plane.origin.z),
    ) else {
        return Err(refused(format!("the plane of {face} is not structurally rational")));
    };
    let (axis, raw) = axis_of([&nx, &ny, &nz]).ok_or_else(|| refused(format!("{face} is not axis-aligned")))?;
    // For a raw plane r · x = c and face sense s, the outward normal is s r / |r_j|.
    let positive = (raw.sign() == Sign::Positive) == (native.sense == Sense::Same);
    let origin = [&ox, &oy, &oz][axis.index()];
    let offset = if positive { origin.clone() } else { &Rational::zero() - origin };
    Ok(ExactAxisPlane { axis, positive, offset: exact(&offset) })
}

fn rational(r: &Real) -> Option<Rational> {
    r.as_rational()
}

fn point_of(p: &Point3) -> Option<[ExactRational; 3]> {
    Some([exact(&rational(&p.x)?), exact(&rational(&p.y)?), exact(&rational(&p.z)?)])
}

/// The world axis a native direction lies on, and whether it points along it.
fn direction_of(v: &Vector3) -> Option<Option<(Axis, bool)>> {
    let (x, y, z) = (rational(&v.x)?, rational(&v.y)?, rational(&v.z)?);
    Some(axis_of([&x, &y, &z]).map(|(axis, raw)| (axis, raw.sign() == Sign::Positive)))
}

fn axis_support(
    what: &str,
    direction: &Vector3,
    through: &Point3,
    radius: Option<ExactRadius>,
    source: SupportSource,
) -> KernelResult<ExactSupport> {
    let not_rational = || unsupported(format!("the axis of {what} is not structurally rational"));
    let (axis, positive) = direction_of(direction)
        .ok_or_else(not_rational)?
        .ok_or_else(|| unsupported(format!("{what} is not axis-aligned")))?;
    let through = point_of(through).ok_or_else(not_rational)?;
    Ok(ExactSupport::Axis(ExactAxis { axis, positive, through, radius, source }))
}

fn radius(what: &str, squared: &Real, outward: Option<bool>) -> KernelResult<ExactRadius> {
    let squared =
        rational(squared).ok_or_else(|| unsupported(format!("the radius of {what} is not structurally rational")))?;
    Ok(ExactRadius { squared: exact(&squared), outward })
}

pub(super) fn entity_support(k: &CatallaxKernel, body: BodyId, entity: SupportEntity) -> KernelResult<ExactSupport> {
    let solid = k.solid(body)?;
    let model = &solid.app.checkpoint.built().model;
    match entity {
        SupportEntity::Face(face) => {
            let id = solid
                .ids
                .faces
                .iter()
                .find_map(|(id, f)| (*f == face).then_some(*id))
                .ok_or(KernelError::UnknownFace { body, face })?;
            let native = model.face(id);
            // Each surface is oriented away from its axis or centre; the face's sense says
            // which way its material lies.
            let outward = Some(native.sense == Sense::Same);
            let what = face.to_string();
            match model.geom().surface(native.surface) {
                Surface::Plane(_) => face_support_plane(k, body, face).map(ExactSupport::Plane),
                Surface::Cylinder(c) => {
                    let r = radius(&what, &c.radius_sq, outward)?;
                    axis_support(&what, &c.axis, &c.origin, Some(r), SupportSource::Cylinder)
                }
                Surface::Cone(c) => axis_support(&what, &c.axis, &c.apex, None, SupportSource::Cone),
                Surface::Sphere(s) => {
                    let r = rational(&s.radius)
                        .ok_or_else(|| unsupported(format!("the radius of {what} is not structurally rational")))?;
                    let squared = exact(&(&r * &r));
                    let at = point_of(&s.center)
                        .ok_or_else(|| unsupported(format!("the centre of {what} is not structurally rational")))?;
                    let radius = Some(ExactRadius { squared, outward });
                    Ok(ExactSupport::Point(ExactPoint { at, radius, source: SupportSource::Sphere }))
                }
                _ => Err(unsupported(format!("{what} is not a plane, cylinder, cone or sphere"))),
            }
        }
        SupportEntity::Edge(edge) => {
            let id = solid
                .ids
                .edges
                .iter()
                .find_map(|(id, e)| (*e == edge).then_some(*id))
                .ok_or(KernelError::UnknownEdge { body, edge })?;
            let what = edge.to_string();
            match model.geom().curve(model.edge(id).curve) {
                Curve::Line(l) => axis_support(&what, &l.dir, &l.origin, None, SupportSource::Line),
                Curve::Circle(c) => {
                    let r = radius(&what, &c.radius_sq, None)?;
                    axis_support(&what, &c.normal, &c.center, Some(r), SupportSource::Circle)
                }
                _ => Err(unsupported(format!("{what} is not a line or a circle"))),
            }
        }
        SupportEntity::Vertex(vertex) => {
            let id = solid
                .ids
                .vertices
                .iter()
                .find_map(|(id, v)| (*v == vertex).then_some(*id))
                .ok_or_else(|| KernelError::InvalidInput(format!("{body} has no vertex {vertex}")))?;
            let what = vertex.to_string();
            match model.geom().point(model.vertex(id).point) {
                PointDef::Explicit(p) => {
                    let at = point_of(p).ok_or_else(|| unsupported(format!("{what} is not structurally rational")))?;
                    Ok(ExactSupport::Point(ExactPoint { at, radius: None, source: SupportSource::Vertex }))
                }
                PointDef::Meet(_) => Err(unsupported(format!("{what} is defined where surfaces meet, not as a point"))),
            }
        }
    }
}

fn enclosed_number(value: &Real) -> KernelResult<SupportBounds> {
    use catallax_numbers::{Decider, OperationSpan};
    let mut decider = Decider::default();
    let width = Rational::from_i64(1) / Rational::from_i64(1 << 60);
    let span = OperationSpan::begin("brep", &"entity_support_enclosed", "entity_support_enclosed", &decider);
    let result = decider.enclose(value, &width);
    span.finish(&decider, result.as_ref().err(), None);
    let ball = result.map_err(|e| crate::query_error("entity_support_enclosed", e))?;
    Ok(SupportBounds { lo: exact(&ball.lo()), hi: exact(&ball.hi()) })
}

pub(super) fn enclosed_support(
    k: &CatallaxKernel,
    body: BodyId,
    entity: SupportEntity,
) -> KernelResult<EnclosedSupport> {
    let solid = k.solid(body)?;
    let model = &solid.app.checkpoint.built().model;
    let coords = |v: [&Real; 3]| -> KernelResult<[SupportBounds; 3]> {
        Ok([enclosed_number(v[0])?, enclosed_number(v[1])?, enclosed_number(v[2])?])
    };
    let axis = |direction: &Vector3,
                through: &Point3,
                radius: Option<&Real>,
                outward: Option<bool>|
     -> KernelResult<EnclosedSupport> {
        let direction = coords([&direction.x, &direction.y, &direction.z])?;
        let through = coords([&through.x, &through.y, &through.z])?;
        let radius_squared = radius.map(enclosed_number).transpose()?;
        Ok(EnclosedSupport::Axis { direction, through, radius_squared, outward })
    };
    match entity {
        SupportEntity::Face(face) => {
            let id = solid
                .ids
                .faces
                .iter()
                .find_map(|(id, f)| (*f == face).then_some(*id))
                .ok_or(KernelError::UnknownFace { body, face })?;
            let native = model.face(id);
            let outward = Some(native.sense == Sense::Same);
            match model.geom().surface(native.surface) {
                Surface::Plane(p) => {
                    let sign = if native.sense == Sense::Same { Real::from_i64(1) } else { Real::from_i64(-1) };
                    let normal = coords([&(&p.normal.x * &sign), &(&p.normal.y * &sign), &(&p.normal.z * &sign)])?;
                    let through = coords([&p.origin.x, &p.origin.y, &p.origin.z])?;
                    Ok(EnclosedSupport::Plane { normal, through })
                }
                Surface::Cylinder(c) => axis(&c.axis, &c.origin, Some(&c.radius_sq), outward),
                Surface::Cone(c) => axis(&c.axis, &c.apex, None, outward),
                Surface::Sphere(s) => {
                    let at = coords([&s.center.x, &s.center.y, &s.center.z])?;
                    let squared = &s.radius * &s.radius;
                    let radius_squared = Some(enclosed_number(&squared)?);
                    Ok(EnclosedSupport::Point { at, radius_squared, outward })
                }
                _ => Err(unsupported("Torus and spline faces have no supported analytic mate definition".into())),
            }
        }
        SupportEntity::Edge(edge) => {
            let id = solid
                .ids
                .edges
                .iter()
                .find_map(|(id, e)| (*e == edge).then_some(*id))
                .ok_or(KernelError::UnknownEdge { body, edge })?;
            match model.geom().curve(model.edge(id).curve) {
                Curve::Line(l) => axis(&l.dir, &l.origin, None, None),
                Curve::Circle(c) => axis(&c.normal, &c.center, Some(&c.radius_sq), None),
                _ => Err(unsupported("Only line and circle edges have enclosed mate supports".into())),
            }
        }
        SupportEntity::Vertex(vertex) => {
            let id = solid
                .ids
                .vertices
                .iter()
                .find_map(|(id, v)| (*v == vertex).then_some(*id))
                .ok_or_else(|| KernelError::InvalidInput(format!("unknown vertex {vertex}")))?;
            match model.geom().point(model.vertex(id).point) {
                PointDef::Explicit(p) => {
                    Ok(EnclosedSupport::Point { at: coords([&p.x, &p.y, &p.z])?, radius_squared: None, outward: None })
                }
                _ => Err(unsupported("Intersection-only vertices have no explicit mate point".into())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Diagnostics;

    /// Under a budget of nothing the query still answers, so it cannot be spending any.
    #[test]
    fn a_zero_work_budget_is_enough() {
        let mut k = CatallaxKernel::new();
        let profile = Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 40.0, 30.0);
        let base = k.create_body_from_profile(&OpTag::new("p"), &profile).unwrap();
        let body = k.extrude(&OpTag::new("b"), base.body, &Extrude { distance: 10.0, direction: None }).unwrap().body;
        let face = k.topology(body).unwrap().faces.iter().find(|f| f.normal == Some(Vec3::Z)).unwrap().id;
        k.diagnostics = Some(Diagnostics::new(Some(0)));
        let plane = k.face_support_plane(body, face).unwrap();
        assert_eq!(plane.normal(), [0, 0, 1]);
    }

    fn extruded(k: &CatallaxKernel, tag: &str, profile: &Profile, depth: f64) -> BodyId {
        let base = k.create_body_from_profile(&OpTag::new(format!("{tag}-p")), profile).unwrap();
        k.extrude(&OpTag::new(tag), base.body, &Extrude { distance: depth, direction: None }).unwrap().body
    }

    fn supports(k: &CatallaxKernel, body: BodyId, kind: SurfaceKind) -> Vec<KernelResult<ExactSupport>> {
        let t = k.topology(body).unwrap();
        t.faces
            .iter()
            .filter(|f| f.surface == kind)
            .map(|f| k.entity_support(body, SupportEntity::Face(f.id)))
            .collect()
    }

    fn whole(n: i64) -> ExactRational {
        ExactRational { numerator: n.to_string(), denominator: "1".into() }
    }

    fn axis(support: &ExactSupport) -> &ExactAxis {
        let ExactSupport::Axis(a) = support else { panic!("an axis, not {support:?}") };
        a
    }

    /// A shaft's side, a bore's wall and a revolved face read as exact axes along +Z or
    /// -Z, through the circle's centre, with the sketch's squared radius. The shaft's
    /// side looks away from its axis and the bore's towards it.
    #[test]
    fn a_shaft_a_bore_and_a_revolved_face_read_as_rational_axes() {
        let k = CatallaxKernel::new();
        let shaft = extruded(&k, "shaft", &Profile::circle(Plane::XY, Point2::new(5.0, -3.0), 4.0), 30.0);
        let [side] = &supports(&k, shaft, SurfaceKind::Cylinder)[..] else { panic!("one side") };
        let side = axis(side.as_ref().unwrap());
        assert_eq!((side.axis, side.through[0].clone(), side.through[1].clone()), (Axis::Z, whole(5), whole(-3)));
        assert_eq!(side.radius, Some(ExactRadius { squared: whole(16), outward: Some(true) }));
        assert_eq!(side.source, SupportSource::Cylinder);

        let block = extruded(&k, "block", &Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 40.0, 30.0), 10.0);
        let tool = extruded(&k, "tool", &Profile::circle(Plane::XY, Point2::new(10.0, 5.0), 3.0), 10.0);
        let holed = k.boolean(&OpTag::new("cut"), BooleanOp::Subtract, block, tool).unwrap().body;
        let [wall] = &supports(&k, holed, SurfaceKind::Cylinder)[..] else { panic!("one wall") };
        let wall = axis(wall.as_ref().unwrap());
        assert_eq!((wall.axis, wall.through[0].clone(), wall.through[1].clone()), (Axis::Z, whole(10), whole(5)));
        assert_eq!(wall.radius, Some(ExactRadius { squared: whole(9), outward: Some(false) }));

        let ring = k
            .create_body_from_profile(
                &OpTag::new("ring-p"),
                &Profile::rectangle(xz(), Point2::new(15.0, 5.0), 10.0, 10.0),
            )
            .unwrap()
            .body;
        let revolve = Revolve { axis_origin: Vec3::ZERO, axis_direction: Vec3::Z, angle_deg: 360.0 };
        let ring = k.revolve(&OpTag::new("ring"), ring, &revolve).unwrap().body;
        let walls = supports(&k, ring, SurfaceKind::Cylinder);
        assert_eq!(walls.len(), 2, "an inner and an outer wall");
        let mut radii: Vec<(ExactRational, Option<bool>)> = walls
            .iter()
            .map(|w| {
                let a = axis(w.as_ref().unwrap());
                assert_eq!((a.axis, a.through[0].clone(), a.through[1].clone()), (Axis::Z, whole(0), whole(0)));
                let r = a.radius.clone().unwrap();
                (r.squared, r.outward)
            })
            .collect();
        radii.sort_by_key(|(r, _)| r.numerator.parse::<i64>().unwrap());
        assert_eq!(radii, [(whole(100), Some(false)), (whole(400), Some(true))]);
    }

    /// A copy turned by 60 degrees has an algebraic centre, so its axis is refused by
    /// name; a copy turned by a quarter turn stays rational.
    #[test]
    fn a_sixty_degree_copy_is_refused_and_a_quarter_turn_is_not() {
        let k = CatallaxKernel::new();
        for (angle, refused) in [(60.0, true), (90.0, false)] {
            let pin = extruded(&k, "pin", &Profile::circle(Plane::XY, Point2::new(20.0, 0.0), 2.0), 5.0);
            let layout = PatternLayout::Circular {
                axis_origin: Vec3::ZERO,
                axis_direction: Vec3::Z,
                count: 2,
                angle_deg: angle,
            };
            let pins = k.pattern(&OpTag::new("pattern"), pin, &layout).unwrap().body;
            let found = supports(&k, pins, SurfaceKind::Cylinder);
            assert_eq!(found.len(), 2);
            let bad: Vec<String> = found
                .iter()
                .filter_map(|s| match s {
                    Err(KernelError::Unsupported(m)) => Some(m.clone()),
                    Err(e) => panic!("{e}"),
                    Ok(_) => None,
                })
                .collect();
            if refused {
                assert_eq!(bad.len(), 1, "{angle} degrees: {bad:?}");
                assert!(bad[0].contains("not structurally rational"), "{}", bad[0]);
                let topology = k.topology(pins).unwrap();
                let face = topology
                    .faces
                    .iter()
                    .find(|f| {
                        f.surface == SurfaceKind::Cylinder && k.entity_support(pins, SupportEntity::Face(f.id)).is_err()
                    })
                    .unwrap();
                let bounded = k.entity_support_enclosed(pins, SupportEntity::Face(face.id)).unwrap();
                let EnclosedSupport::Axis { through, radius_squared: Some(radius), .. } = bounded else {
                    panic!("an enclosed cylinder")
                };
                assert_eq!(radius.lo, whole(4));
                assert_eq!(radius.hi, whole(4));
                assert_ne!(through[1].lo, through[1].hi, "the algebraic centre keeps a nonzero enclosure width");
            } else {
                assert!(bad.is_empty(), "{angle} degrees: {bad:?}");
            }
        }
    }

    /// A sphere reads as its centre and radius; a circular edge as an axis along its normal;
    /// a straight edge as a line.
    #[test]
    fn spheres_circles_and_lines_read_exactly() {
        let k = CatallaxKernel::new();
        let shaft = extruded(&k, "shaft", &Profile::circle(Plane::XY, Point2::new(0.0, 0.0), 5.0), 10.0);
        let t = k.topology(shaft).unwrap();
        let circle = t.edges.iter().find(|e| e.curve == CurveKind::Circle).unwrap();
        let ExactSupport::Axis(a) = k.entity_support(shaft, SupportEntity::Edge(circle.id)).unwrap() else {
            panic!("an axis")
        };
        assert_eq!((a.axis, a.source), (Axis::Z, SupportSource::Circle));
        assert_eq!(a.radius, Some(ExactRadius { squared: whole(25), outward: None }));

        let block = extruded(&k, "block", &Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 40.0, 30.0), 10.0);
        let t = k.topology(block).unwrap();
        for e in t.edges.iter().filter(|e| e.curve == CurveKind::Line) {
            let ExactSupport::Axis(a) = k.entity_support(block, SupportEntity::Edge(e.id)).unwrap() else {
                panic!("an axis")
            };
            assert_eq!(a.source, SupportSource::Line);
            assert!(a.radius.is_none());
        }
        for v in &t.vertices {
            match k.entity_support(block, SupportEntity::Vertex(v.id)) {
                Ok(ExactSupport::Point(p)) => {
                    let x: f64 =
                        p.at[0].numerator.parse::<f64>().unwrap() / p.at[0].denominator.parse::<f64>().unwrap();
                    assert!((x - v.position.x).abs() < 1e-9, "{p:?} at {:?}", v.position);
                }
                other => panic!("vertex {}: {other:?}", v.id),
            }
        }

        let ball = k.create_body_from_profile(&OpTag::new("ball-p"), &half_disc()).unwrap().body;
        let revolve = Revolve { axis_origin: Vec3::ZERO, axis_direction: Vec3::Z, angle_deg: 360.0 };
        let ball = k.revolve(&OpTag::new("ball"), ball, &revolve).unwrap().body;
        let found = supports(&k, ball, SurfaceKind::Sphere);
        let [Ok(ExactSupport::Point(p))] = &found[..] else { panic!("{found:?}") };
        assert_eq!(p.at, [whole(0), whole(0), whole(0)]);
        assert_eq!(p.radius, Some(ExactRadius { squared: whole(36), outward: Some(true) }));
    }

    /// A half disc of radius 6 in the XZ plane, its straight side on the Z axis.
    fn half_disc() -> Profile {
        let (a, b) = (Point2::new(0.0, -6.0), Point2::new(0.0, 6.0));
        let arc = Segment::Arc { center: Point2::new(0.0, 0.0), from: a, to: b, clockwise: false };
        Profile { plane: xz(), outer: Loop(vec![Segment::Line { from: b, to: a }, arc]), holes: Vec::new() }
    }

    /// The plane whose (u, v) are world (x, z).
    fn xz() -> Plane {
        Plane { origin: Vec3::ZERO, normal: -Vec3::Y, x_dir: Vec3::X }
    }
}
