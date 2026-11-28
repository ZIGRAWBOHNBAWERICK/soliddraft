//! Manual hardware captures for the Mac GPU acceptance gates.

#[cfg(target_os = "macos")]
#[path = "support/viewport_acceptance.rs"]
mod viewport_acceptance;

#[cfg(target_os = "macos")]
#[path = "support/gpu_readback.rs"]
mod gpu_readback;

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
