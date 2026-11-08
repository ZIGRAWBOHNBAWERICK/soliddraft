//! Ellipse sketch profiles must build and measure through Catallax.

use std::sync::Arc;

use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, P2, Sketch, solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, FeatureStatus, ProfileSource,
    SketchFeature, SketchPlane,
};

#[test]
fn ellipse_and_partial_ellipse_sketches_build_with_their_analytic_volumes() {
    let mut ellipse = Sketch::new();
    ellipse.add(EntityKind::Ellipse { center: P2::ZERO, major: P2::new(4.0, 0.0), minor: P2::new(0.0, 2.0) }, false);
    let mut half = Sketch::new();
    half.add(
        EntityKind::EllipticArc {
            center: P2::ZERO,
            major: P2::new(4.0, 0.0),
            minor: P2::new(0.0, 2.0),
            start: P2::new(4.0, 0.0),
            end: P2::new(-4.0, 0.0),
        },
        false,
    );
    half.add(EntityKind::Line { a: P2::new(-4.0, 0.0), b: P2::new(4.0, 0.0) }, false);
    for (name, mut sketch, expected) in
        [("Ellipse", ellipse, 16.0 * std::f64::consts::PI), ("Partial ellipse", half, 8.0 * std::f64::consts::PI)]
    {
        solve::solve(&mut sketch, None, true);
        let mut doc = Document::new(name, Arc::new(CatallaxKernel::new()));
        let sketch_id = doc
            .execute(Command::AddFeature {
                kind: FeatureKind::Sketch(SketchFeature {
                    plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
                    sketch,
                }),
                name: None,
                position: None,
                via_assistant: false,
            })
            .unwrap()
            .created
            .unwrap();
        let extrude_id = doc
            .execute(Command::AddFeature {
                kind: FeatureKind::Extrude(ExtrudeFeature {
                    profile: ProfileSource::Sketch { feature: sketch_id },
                    operation: ExtrudeOp::Boss,
                    end: EndCondition::Blind { depth: 2.0 },
                    reverse: false,
                }),
                name: None,
                position: None,
                via_assistant: false,
            })
            .unwrap()
            .created
            .unwrap();
        assert_eq!(doc.status(extrude_id), Some(&FeatureStatus::Ok), "{name}");
        let actual = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
        assert!((actual - expected).abs() < 1e-5, "{name}: {actual} != {expected}");
    }
}
