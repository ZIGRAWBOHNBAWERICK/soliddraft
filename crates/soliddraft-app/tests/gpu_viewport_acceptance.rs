//! Manual hardware captures for the Mac GPU acceptance gates.

#[cfg(target_os = "macos")]
#[path = "support/viewport_acceptance.rs"]
mod viewport_acceptance;

#[cfg(target_os = "macos")]
#[path = "support/gpu_readback.rs"]
mod gpu_readback;

#[path = "support/trimmed_carrier.rs"]
mod trimmed_carrier;

#[cfg(target_os = "macos")]
fn out() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("SOLIDDRAFT_GPU_ACCEPTANCE_OUT").expect("gate output directory"))
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires an Apple Metal adapter; use ci/gpu-acceptance/run.py"]
fn capture_metal_p1() {
    viewport_acceptance::capture_metal_p1(&out());
}

/// Each part of the manifest in `SOLIDDRAFT_GPU_ACCEPTANCE_PARTS` opens in the app on
/// Metal, and its GPU preview is read back and checked against the checked mesh.
#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires an Apple Metal adapter; use ci/gpu-acceptance/preview.py"]
fn read_back_metal_previews() {
    let out = out();
    let manifest =
        std::path::PathBuf::from(std::env::var_os("SOLIDDRAFT_GPU_ACCEPTANCE_PARTS").expect("parts manifest"));
    let (app, state) = viewport_acceptance::metal_app(&out);
    let info = state.adapter.get_info();
    let device = gpu_readback::Device {
        device: state.device.clone(),
        queue: state.queue.clone(),
        adapter: serde_json::json!({
            "backend": format!("{:?}", info.backend),
            "device_type": format!("{:?}", info.device_type),
            "name": info.name,
            "installed": soliddraft_app::viewport::gpu_preview::active().map(|gpu| gpu.adapter.clone()),
        }),
        tier: app.state().state.graphics_device.tier,
    };
    let kernel = app.state().state.doc.kernel().clone();
    gpu_readback::read_back_parts(&device, kernel, &manifest, &out);
}

/// The trimmed carrier keeps a preview drawn over the wrong part of the right carrier off
/// its face. The rounded boss's base fillet is a quarter of a torus tube: centred on
/// (20, 15), 10 mm to the tube centre at z = 12, tube radius 2, between the boss at
/// r = 8 and the top face at z = 10. Points on that quarter measure on the face. Points
/// on the same torus over the other three quarters, as a GPU preview that took the long
/// way round the tube once drew them, measure far from it.
#[test]
fn a_preview_on_the_wrong_arc_of_its_carrier_is_far_from_its_face() {
    use soliddraft_kernel_api::SurfaceKind;
    let config = std::env::temp_dir().join(format!("soliddraft-trimmed-carrier-{}", std::process::id()));
    soliddraft_app::file::set_config_dir(Some(config));
    let kernel = soliddraft_app::startup::kernel();
    let part = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../site/parts/rounded-boss.dwprt");
    let doc = soliddraft_app::file::open(&part, kernel.clone(), None).expect("the rounded boss opens");
    let body = doc.display_body().expect("one body is shown");
    let topology = kernel.topology(body).unwrap();
    let torus = topology.faces.iter().find(|face| face.surface == SurfaceKind::Torus).expect("the base fillet");
    let (lo, hi) = (torus.bbox.min, torus.bbox.max);
    assert!((lo.x - 10.0).abs() < 0.05 && (hi.x - 30.0).abs() < 0.05, "the fillet's box: {:?}", torus.bbox);
    assert!((lo.z - 10.0).abs() < 0.05 && (hi.z - 12.0).abs() < 0.05, "the fillet's box: {:?}", torus.bbox);

    let params = doc.tess_params;
    let checked = kernel.checked_rendering(body, &params).unwrap();
    let own: Vec<[[f64; 3]; 3]> = checked
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .zip(&checked.triangle_faces)
        .filter(|(_, face)| **face == torus.id)
        .map(|(t, _)| t.map(|i| checked.positions[i as usize].map(f64::from)))
        .collect();
    let edges = kernel.checked_rendering_edges(body, &params).unwrap();
    let boundary: Vec<Vec<[f64; 3]>> = edges
        .iter()
        .filter(|edge| torus.edges.contains(&edge.edge))
        .map(|edge| edge.points.iter().map(|p| [p.x, p.y, p.z]).collect())
        .collect();
    let tolerance = 0.01;
    let carrier = trimmed_carrier::TrimmedCarrier::new(kernel.as_ref(), body, torus.id, own, boundary, tolerance);

    // A point on the torus at `around` degrees about its axis and `tube` degrees about
    // the tube centre, with the tube's outward normal.
    let at = |around: f64, tube: f64| {
        let (sa, ca) = around.to_radians().sin_cos();
        let (st, ct) = tube.to_radians().sin_cos();
        let r = 10.0 + 2.0 * ct;
        ([20.0 + r * ca, 15.0 + r * sa, 12.0 + 2.0 * st], [ct * ca, ct * sa, st])
    };
    for around in (0..360).step_by(30).map(f64::from) {
        for tube in [190.0, 225.0, 260.0] {
            let (point, normal) = at(around, tube);
            let measured = carrier.measure(point, normal, tolerance);
            assert!(
                measured.on_face && measured.distance <= tolerance,
                "on the fillet at {around}°, tube {tube}°: {measured:?}"
            );
        }
        for tube in [300.0, 330.0, 0.0, 45.0, 90.0, 135.0] {
            let (point, normal) = at(around, tube);
            let measured = carrier.measure(point, normal, tolerance);
            assert!(measured.distance > 1.0, "on the wrong arc at {around}°, tube {tube}°: {measured:?}");
        }
    }
}
