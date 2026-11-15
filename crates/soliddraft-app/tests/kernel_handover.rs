//! A part built on one Catallax kernel and handed over to another, as the browser's
//! kernel worker hands its builds to the page: the second kernel rebuilds the same part
//! from the offered bodies without evaluating any feature, with the same names, mass,
//! topology and export.

use soliddraft_kernel_api::{Kernel, StepOptions};
use soliddraft_kernel_catallax::CatallaxKernel;
use std::sync::Arc;

const BRACKET: &[u8] = include_bytes!("../../../site/parts/bracket.dwprt");

#[test]
fn a_part_handed_over_rebuilds_on_another_kernel_without_evaluating_any_feature() {
    let worker = Arc::new(CatallaxKernel::new());
    worker.keep_builds().unwrap();
    let built = soliddraft_model::Document::from_bytes(BRACKET, worker.clone()).unwrap();
    assert!(worker.fresh_builds() > 0);
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
    let (ta, tb) = (worker.topology(a).unwrap(), page.topology(b).unwrap());
    assert_eq!((tb.faces.len(), tb.edges.len()), (ta.faces.len(), ta.edges.len()));
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
