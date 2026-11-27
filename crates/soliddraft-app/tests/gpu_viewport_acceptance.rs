//! Manual hardware capture for the Mac GPU acceptance gate.

#[cfg(target_os = "macos")]
#[path = "support/viewport_acceptance.rs"]
mod viewport_acceptance;

#[cfg(target_os = "macos")]
#[test]
#[ignore = "requires an Apple Metal adapter; use ci/gpu-acceptance/run.py"]
fn capture_metal_p1() {
    let out =
        std::path::PathBuf::from(std::env::var_os("SOLIDDRAFT_GPU_ACCEPTANCE_OUT").expect("gate output directory"));
    viewport_acceptance::capture_metal_p1(&out);
}
