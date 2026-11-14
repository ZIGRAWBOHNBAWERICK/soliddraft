//! Rebuild the browser-drawn part through the document, kernel and display mesh.
use std::sync::Arc;

use soliddraft_kernel_api::{FaceId, Mesh, Plane, TessellationParams, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::{Document, FeatureId, FeatureKind, Generator, SketchPlane};

const PART: &str = include_str!("fixtures/angled-face-cut.json");

fn covers(mesh: &Mesh, face: FaceId, plane: Plane, u: f64, v: f64) -> bool {
    let point = plane.origin + plane.x_dir * u + plane.y_dir() * v;
    mesh.indices.chunks_exact(3).zip(&mesh.triangle_faces).any(|(indices, id)| {
        if *id != face {
            return false;
        }
        let points: Vec<_> = indices
            .iter()
            .map(|index| {
                let [x, y, z] = mesh.positions[*index as usize];
                let delta = Vec3::new(f64::from(x), f64::from(y), f64::from(z)) - point;
                (delta.dot(plane.x_dir), delta.dot(plane.y_dir()))
            })
            .collect();
        let cross = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;
        let signs = [cross(points[0], points[1]), cross(points[1], points[2]), cross(points[2], points[0])];
        signs.iter().all(|s| *s >= 0.0) || signs.iter().all(|s| *s <= 0.0)
    })
}

#[test]
fn saved_browser_cut_opens_the_face_where_the_sketch_was_drawn() {
    let doc = Document::from_part_json(PART, Arc::new(CatallaxKernel::new())).unwrap();
    for (id, status) in doc.statuses() {
        assert!(status.is_ok(), "{id}: {status:?}");
    }
    let sketch = FeatureId(4);
    let cut = FeatureId(5);
    let FeatureKind::Sketch(source) = &doc.feature(sketch).unwrap().kind else { panic!() };
    let SketchPlane::Face { face: reference } = &source.plane else { panic!() };
    let origin = &doc
        .generated_faces(reference.feature)
        .iter()
        .find(|f| f.generator == reference.generator && f.ordinal == reference.ordinal)
        .unwrap()
        .origin;
    let before = doc.kernel().topology(doc.body_before(cut).unwrap()).unwrap();
    let original = before.faces.iter().find(|f| f.origins.contains(origin)).unwrap();
    let result = doc.body().unwrap();
    let topology = doc.kernel().topology(result).unwrap();
    let mounted = topology.faces.iter().find(|f| f.origins.contains(origin)).unwrap();
    let removed_area = original.area - mounted.area;
    assert!((1000.0..1100.0).contains(&removed_area), "the 1063 mm² opening removed {removed_area} mm²");
    let cap = &doc.generated_faces(cut).iter().find(|f| f.generator == Generator::StartCap).unwrap().origin;
    assert!(!topology.faces.iter().any(|f| f.origins.contains(cap)), "the cut kept an internal start cap");
    let frame = doc.built_sketch(sketch).unwrap().plane;
    let mesh = doc.kernel().tessellate(result, &TessellationParams::default()).unwrap();
    assert!(
        !covers(&mesh, mounted.id, frame, -52.195125579833984, -18.50117540359497),
        "the displayed face covers the hole centre"
    );
    assert!(covers(&mesh, mounted.id, frame, -90.0, -18.5), "the test must also see material beside the opening");
}
