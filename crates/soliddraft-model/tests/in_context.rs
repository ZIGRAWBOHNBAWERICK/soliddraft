//! In-context references in the part: a sketch on a face of another component builds on
//! the plane the part keeps, through save and reopen, follows a moved face as one undo
//! step, and keeps its last plane when the reference breaks.

use std::sync::Arc;

use soliddraft_kernel_api::{Plane, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, ContextFace, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureId, FeatureKind,
    FeatureStatus, ProfileSource, SketchFeature, SketchPlane, context,
};

fn doc() -> Document {
    Document::new("Plate", Arc::new(CatallaxKernel::new()))
}

fn add(doc: &mut Document, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
        .unwrap()
        .created
        .unwrap()
}

/// The top face of the base below, as the plate's frame sees it: `z` millimetres up.
fn base_top(z: f64) -> ContextFace {
    ContextFace::new(
        ("assembly-id", "Rig"),
        "2",
        ("1", "base<1>"),
        ("extrude1-end-cap", "Extrude1 · End cap"),
        Plane { origin: Vec3::new(0.0, 0.0, z), normal: Vec3::Z, x_dir: Vec3::X },
    )
}

fn sketch_on(plane: SketchPlane, shape: EntityKind) -> FeatureKind {
    let mut s = Sketch::new();
    s.add(shape, false);
    solve(&mut s, None, true);
    FeatureKind::Sketch(SketchFeature { plane, sketch: s })
}

fn circle() -> EntityKind {
    EntityKind::Circle { center: P2::ZERO, radius: 5.0 }
}

fn boss(sketch: FeatureId, depth: f64) -> FeatureKind {
    FeatureKind::Extrude(ExtrudeFeature {
        profile: ProfileSource::Sketch { feature: sketch },
        operation: ExtrudeOp::Boss,
        end: EndCondition::Blind { depth },
        reverse: false,
        options: Default::default(),
    })
}

/// The part's body box along z.
fn heights(doc: &Document) -> (f64, f64) {
    let body = doc.body().expect("a body");
    let b = doc.kernel().topology(body).unwrap().bbox;
    (b.min.z, b.max.z)
}

/// The kernel pads the box of a body with curved faces by 0.01 mm; the planes are
/// checked exactly through [`plane_z`].
const BOX_PAD: f64 = 0.011;

fn close(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() <= BOX_PAD && (a.1 - b.1).abs() <= BOX_PAD
}

/// Where the sketch was built, along z.
fn plane_z(doc: &Document, sketch: FeatureId) -> f64 {
    doc.built_sketch(sketch).expect("a built sketch").plane.origin.z
}

/// A peg sketched on the base's top face 20 mm below the plate's origin, 25 mm tall.
fn pegged() -> (Document, FeatureId, FeatureId) {
    let mut d = doc();
    let sketch = add(&mut d, sketch_on(SketchPlane::Context { context: base_top(-20.0), offset: 0.0 }, circle()));
    let peg = add(&mut d, boss(sketch, 25.0));
    (d, sketch, peg)
}

#[test]
fn a_sketch_on_another_components_face_builds_on_the_kept_plane_and_survives_reopening() {
    let (d, sketch, peg) = pegged();
    assert_eq!(d.status(sketch), Some(&FeatureStatus::Ok));
    assert_eq!(d.status(peg), Some(&FeatureStatus::Ok));
    assert!(close(heights(&d), (-20.0, 5.0)), "{:?}", heights(&d));
    assert_eq!(plane_z(&d, sketch), -20.0);
    let bytes = d.to_bytes().unwrap();
    let back = Document::from_bytes(&bytes, Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(context::references(back.features()), context::references(d.features()));
    assert!(close(heights(&back), (-20.0, 5.0)));
    let refs = context::references(back.features());
    assert_eq!(refs.len(), 1);
    assert_eq!((refs[0].0, refs[0].1.label()), (sketch, "base<1> · Extrude1 · End cap".to_owned()));
}

#[test]
fn following_a_moved_face_rebuilds_and_undoes_as_one_step() {
    let (mut d, sketch, _) = pegged();
    let moved = base_top(-20.0).followed(
        "Rig",
        "base<1>",
        Plane { origin: Vec3::new(0.0, 0.0, -15.0), normal: Vec3::Z, x_dir: Vec3::X },
    );
    let kind = context::with_reference(&d.feature(sketch).unwrap().kind, moved).unwrap();
    let depth = d.undo_depth();
    d.execute(Command::EditFeature { id: sketch, kind }).unwrap();
    assert_eq!(d.undo_depth(), depth + 1);
    assert!(close(heights(&d), (-15.0, 10.0)), "{:?}", heights(&d));
    assert_eq!(plane_z(&d, sketch), -15.0);
    d.undo().unwrap();
    assert!(close(heights(&d), (-20.0, 5.0)));
    assert_eq!(plane_z(&d, sketch), -20.0);
    d.redo().unwrap();
    assert!(close(heights(&d), (-15.0, 10.0)));
    assert_eq!(plane_z(&d, sketch), -15.0);
}

#[test]
fn a_broken_reference_keeps_its_plane_and_a_later_feature_builds_on_top() {
    let (mut d, sketch, _) = pegged();
    let broken = base_top(-20.0).broken("base<1> is no longer in the assembly");
    let kind = context::with_reference(&d.feature(sketch).unwrap().kind, broken).unwrap();
    d.execute(Command::EditFeature { id: sketch, kind }).unwrap();
    assert_eq!(d.status(sketch), Some(&FeatureStatus::Ok));
    assert!(close(heights(&d), (-20.0, 5.0)));
    assert_eq!(plane_z(&d, sketch), -20.0);
    let refs = context::references(d.features());
    assert_eq!(refs[0].1.broken.as_deref(), Some("base<1> is no longer in the assembly"));
    // A later feature on the part's own Top plane builds over the kept plane.
    let mut square = Sketch::new();
    square.rectangle(P2::ZERO, 30.0, 30.0).unwrap();
    solve(&mut square, None, true);
    let plate = add(
        &mut d,
        FeatureKind::Sketch(SketchFeature {
            plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
            sketch: square,
        }),
    );
    let top = add(&mut d, boss(plate, 8.0));
    assert_eq!(d.status(top), Some(&FeatureStatus::Ok));
    assert!(close(heights(&d), (-20.0, 8.0)), "{:?}", heights(&d));
}

#[test]
fn a_kept_plane_that_is_not_a_plane_stops_the_sketch_with_its_reason() {
    let mut d = doc();
    let bad = ContextFace { normal: [0.0, 0.0, 2.0], ..base_top(-20.0) };
    let sketch = add(&mut d, sketch_on(SketchPlane::Context { context: bad, offset: 0.0 }, circle()));
    match d.status(sketch) {
        Some(FeatureStatus::Failed(why)) => assert!(why.contains("unusable") && why.contains("base<1>"), "{why}"),
        other => panic!("expected a failed sketch, got {other:?}"),
    }
}

#[test]
fn an_offset_from_the_kept_plane_moves_the_sketch_along_its_normal() {
    let mut d = doc();
    let sketch = add(&mut d, sketch_on(SketchPlane::Context { context: base_top(-20.0), offset: 4.0 }, circle()));
    add(&mut d, boss(sketch, 6.0));
    assert!(close(heights(&d), (-16.0, -10.0)), "{:?}", heights(&d));
    assert_eq!(plane_z(&d, sketch), -16.0);
}

#[test]
fn a_reference_plane_never_takes_a_face_of_another_component() {
    let mut d = doc();
    let taken = FeatureKind::ReferencePlane(SketchPlane::Context { context: base_top(-20.0), offset: 0.0 });
    let adding = Command::AddFeature { kind: taken.clone(), name: None, position: None, via_assistant: false };
    assert!(d.execute(adding).is_err(), "only a sketch takes another component's face");
    assert!(d.features().is_empty());
    let own = add(&mut d, FeatureKind::ReferencePlane(SketchPlane::Datum { datum: DatumPlane::Top, offset: 5.0 }));
    assert!(d.execute(Command::EditFeature { id: own, kind: taken.clone() }).is_err());
    assert!(context::references(d.features()).is_empty());
    assert!(context::of_feature(&taken).is_none());
}
