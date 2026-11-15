//! A part built on one Catallax kernel and handed over to another, as the browser's
//! kernel worker hands its builds to the page: the second kernel rebuilds the same part
//! from the offered bodies without evaluating any feature or computing the topology
//! again, with the same names, mass, topology and export.

use soliddraft_kernel_api::{Kernel, StepOptions};
use soliddraft_kernel_catallax::CatallaxKernel;
use std::sync::Arc;

const BRACKET: &[u8] = include_bytes!("../../../site/parts/bracket.dwprt");

/// A topology with each face, edge and vertex id replaced by its rank in an order of the
/// entity's own geometry, so two kernels' numberings of the same body compare equal.
fn canonical(topology: &soliddraft_kernel_api::Topology) -> serde_json::Value {
    use std::collections::BTreeMap;
    let rank = |keys: Vec<(u64, String)>| -> BTreeMap<u64, usize> {
        let mut keys = keys;
        keys.sort_by(|a, b| a.1.cmp(&b.1));
        keys.into_iter().enumerate().map(|(rank, (id, _))| (id, rank)).collect()
    };
    let faces = rank(
        topology.faces.iter().map(|f| (f.id.raw(), format!("{:?}{:?}{:?}", f.centroid, f.area, f.origins))).collect(),
    );
    let edges = rank(topology.edges.iter().map(|e| (e.id.raw(), format!("{:?}{:?}", e.midpoint, e.length))).collect());
    let vertices = rank(topology.vertices.iter().map(|v| (v.id.raw(), format!("{:?}", v.position))).collect());
    let mut t = topology.clone();
    let face = |id: soliddraft_kernel_api::FaceId| soliddraft_kernel_api::FaceId::from_raw(faces[&id.raw()] as u64);
    let edge = |id: soliddraft_kernel_api::EdgeId| soliddraft_kernel_api::EdgeId::from_raw(edges[&id.raw()] as u64);
    let vertex =
        |id: soliddraft_kernel_api::VertexId| soliddraft_kernel_api::VertexId::from_raw(vertices[&id.raw()] as u64);
    for f in &mut t.faces {
        f.id = face(f.id);
        f.edges = f.edges.iter().map(|id| edge(*id)).collect();
        f.edges.sort();
    }
    for e in &mut t.edges {
        e.id = edge(e.id);
        e.faces = e.faces.iter().map(|id| face(*id)).collect();
        e.faces.sort();
        e.vertices = e.vertices.iter().map(|id| vertex(*id)).collect();
    }
    for v in &mut t.vertices {
        v.id = vertex(v.id);
        v.edges = v.edges.iter().map(|id| edge(*id)).collect();
        v.edges.sort();
    }
    t.faces.sort_by_key(|f| f.id);
    t.edges.sort_by_key(|e| e.id);
    t.vertices.sort_by_key(|v| v.id);
    serde_json::to_value(t).unwrap()
}

#[test]
fn a_part_handed_over_rebuilds_on_another_kernel_without_evaluating_any_feature() {
    let worker = Arc::new(CatallaxKernel::new());
    worker.keep_builds().unwrap();
    let built = soliddraft_model::Document::from_bytes(BRACKET, worker.clone()).unwrap();
    assert!(worker.fresh_builds() > 0);
    // The worker's proof meshes the part for display, which computes its topology.
    let worker_topology = worker.topology(built.body().unwrap()).unwrap();
    let bytes = worker.hand_over().unwrap();

    let page = Arc::new(CatallaxKernel::new());
    let offered = page.offer_builds(&bytes).unwrap();
    assert_eq!(offered as u64, worker.fresh_builds(), "every build made on the worker is offered");
    let adopted = soliddraft_model::Document::from_bytes(BRACKET, page.clone()).unwrap();
    assert_eq!(page.fresh_builds(), 0, "the page evaluated no feature");

    for feature in built.features() {
        assert_eq!(adopted.status(feature.id), built.status(feature.id), "{}", feature.name);
    }
    let (a, b) = (built.body().unwrap(), adopted.body().unwrap());
    let mass = |kernel: &CatallaxKernel, body| kernel.mass_properties(body).unwrap();
    assert_eq!(mass(&page, b), mass(&worker, a));
    let topology = page.topology(b).unwrap();
    assert_eq!(page.fresh_topologies(), 0, "the page took the worker's topology");
    assert_eq!(topology.faces.len(), worker_topology.faces.len());
    // Renamed into the page's ids, it is the topology the page would have computed itself,
    // up to the numbering of kernel-local ids; names carry identity, and they match.
    let own = Arc::new(CatallaxKernel::new());
    let reference = soliddraft_model::Document::from_bytes(BRACKET, own.clone()).unwrap();
    assert_eq!(canonical(&topology), canonical(&own.topology(reference.body().unwrap()).unwrap()));
    let labels = |doc: &soliddraft_model::Document, body| {
        let mut labels: Vec<_> =
            doc.face_labels(body).into_iter().map(|(_, label)| label.map(|l| format!("{l:?}"))).collect();
        labels.sort();
        labels
    };
    assert_eq!(labels(&adopted, b), labels(&built, a), "faces keep their names");
    let step = |kernel: &CatallaxKernel, body| kernel.export_step(body, &StepOptions::default()).unwrap().report;
    assert_eq!(format!("{:?}", step(&page, b)), format!("{:?}", step(&worker, a)));
}

/// A worker can hand a build over before it computes the build's topology, as when a
/// later job reuses that build and only then meshes it for display. The topology then
/// follows on its own, and the page still computes neither.
#[test]
fn a_topology_computed_after_its_build_was_handed_over_follows_it() {
    let worker = Arc::new(CatallaxKernel::new());
    worker.keep_builds().unwrap();
    let built = soliddraft_model::Document::from_bytes(BRACKET, worker.clone()).unwrap();
    let builds = worker.hand_over().unwrap();
    let topology = worker.topology(built.body().unwrap()).unwrap();
    let late = worker.hand_over().unwrap();

    let page = Arc::new(CatallaxKernel::new());
    page.offer_builds(&builds).unwrap();
    assert_eq!(page.offer_builds(&late).unwrap(), 1, "only the late topology travels");
    let adopted = soliddraft_model::Document::from_bytes(BRACKET, page.clone()).unwrap();
    assert_eq!(page.topology(adopted.body().unwrap()).unwrap().faces.len(), topology.faces.len());
    assert_eq!((page.fresh_builds(), page.fresh_topologies()), (0, 0));
    assert_eq!(
        worker.hand_over().unwrap().len(),
        CatallaxKernel::new().hand_over().unwrap().len(),
        "nothing is sent twice"
    );
}
