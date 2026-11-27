//! The viewport's GPU preview. A rebuilt part shows first from the GPU mesher, and its
//! checked CPU mesh replaces it when ready. A checked result that arrives late, for a
//! part an edit has already replaced, never replaces the newer preview. With the GPU
//! preview switched off, the same edits show the CPU preview as before.
//!
//! Any adapter runs this, a software one included: it checks the display logic and the
//! mesher's output on this device, and is never hardware acceptance.

mod gated;

use std::sync::Arc;
use std::time::{Duration, Instant};

use gated::Gated;
use soliddraft_app::document::{Command, DisplayMesh, Document, ExtrudeParams};
use soliddraft_app::viewport::gpu_preview;
use soliddraft_kernel_api::BodyId;
use soliddraft_tessellation::Projection;

const HANG: Duration = Duration::from_secs(600);

fn device() -> Option<(wgpu::Device, wgpu::Queue, String)> {
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        assert!(std::env::var_os("SOLIDDRAFT_REQUIRE_GPU").is_none(), "no adapter");
        return None;
    };
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
    Some((device, queue, gpu_preview::describe(&info)))
}

fn shown(doc: &Document) -> (BodyId, Arc<DisplayMesh>) {
    assert_eq!(doc.meshes.len(), 1, "one body is shown");
    doc.meshes.iter().map(|(body, mesh)| (*body, mesh.clone())).next().unwrap()
}

fn top(mesh: &DisplayMesh) -> f32 {
    mesh.bounds().expect("the mesh has bounds").1.z
}

fn wait(kernel: &Gated, what: &str, done: impl Fn(&gated::Rules) -> bool) {
    let rules = kernel.rules.lock().unwrap();
    let (_rules, timeout) = kernel.changed.wait_timeout_while(rules, HANG, |r| !done(r)).unwrap();
    assert!(!timeout.timed_out(), "{what}");
}

fn release(kernel: &Gated, body: BodyId) {
    kernel.rules.lock().unwrap().checked_released.push(body);
    kernel.changed.notify_all();
}

fn frames(doc: &mut Document, until: impl Fn(&Document) -> bool, what: &str) {
    let deadline = Instant::now() + HANG;
    let projection = Projection::Orthographic { pixels_per_mm: 10.0 };
    while !until(doc) {
        assert!(Instant::now() < deadline, "{what}");
        doc.poll_proof();
        doc.update_display(&projection, 0.016);
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The renderer draws the GPU preview, and a pick on its top face takes its point from
/// the kernel, since the preview has no CPU triangles.
fn drawn_and_picked(device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document, mesh: &DisplayMesh) {
    use soliddraft_app::viewport::offscreen::Offscreen;
    use soliddraft_app::viewport::render::Picked;
    let mut offscreen = Offscreen::new(device, queue, soliddraft_app::viewport::look::Tier::Basic);
    let picture = offscreen.capture(device, queue, doc, [160, 120]).unwrap();
    let corner = &picture.pixels[..4];
    let painted = picture.pixels.chunks_exact(4).filter(|pixel| *pixel != corner).count();
    assert!(painted > 500, "the GPU preview drew {painted} pixels");

    let (lo, hi) = mesh.bounds().unwrap();
    let centre = (lo + hi) / 2.0;
    let origin = glam::Vec3::new(centre.x, centre.y, hi.z + 100.0);
    let gpu = mesh.gpu.as_ref().unwrap();
    let hits: Vec<glam::Vec3> = gpu
        .faces
        .iter()
        .filter(|face| face.drawn)
        .filter_map(|face| {
            soliddraft_app::viewport::pick::hit_point(doc, Picked::Face(face.pick), origin, glam::Vec3::NEG_Z)
        })
        .collect();
    assert!(hits.iter().any(|hit| (hit.z - hi.z).abs() < 1e-3), "no pick met the top face: {hits:?}");
}

fn depth(depth: f64) -> ExtrudeParams {
    ExtrudeParams { depth, ..Default::default() }
}

#[test]
fn the_gpu_preview_shows_first_and_a_late_checked_result_never_replaces_a_newer_preview() {
    let Some((device, queue, adapter)) = device() else { return };
    gpu_preview::install_unchecked(&device, &queue, &adapter).unwrap();
    let kernel = Arc::new(Gated::default());
    kernel.rules.lock().unwrap().hold_checked = true;
    let mut doc = Document::new("GPU preview", kernel.clone());
    let id = doc.apply(Command::AddExtrude { params: depth(25.0), via_assistant: false }).unwrap().unwrap();

    // The new body shows from the GPU mesher while the display worker's checked mesh
    // of it waits.
    let (old_body, old) = shown(&doc);
    assert!(old.gpu.is_some(), "the new body shows from the GPU mesher first");
    assert!(old.positions.is_empty(), "no face of a box falls back to the CPU preview");
    drawn_and_picked(&device, &queue, &doc, &old);
    wait(&kernel, "the old body's checked mesh never started", |r| r.checked_holding.contains(&old_body));

    // An edit while that checked mesh is still being made: the edited part shows its
    // own GPU preview while its checked mesh waits too.
    doc.apply_interactive(Command::EditExtrude { id, params: depth(30.0) }).unwrap();
    frames(&mut doc, |doc| doc.provisional_mesh().is_some_and(|mesh| mesh.gpu.is_some()), "no GPU preview of the edit");
    let preview = doc.provisional_mesh().unwrap();
    let new_body = preview.gpu.as_ref().unwrap().body;
    assert_ne!(new_body, old_body);
    assert!(top(&preview) > top(&old) + 1.0, "the preview shows the edited part");
    wait(&kernel, "the edit's checked mesh never started", |r| r.checked_holding.contains(&new_body));

    // The old part's checked mesh arrives late. The edit's preview stays on screen.
    release(&kernel, old_body);
    wait(&kernel, "the old body's checked mesh was not let go", |r| !r.checked_holding.contains(&old_body));
    let projection = Projection::Orthographic { pixels_per_mm: 10.0 };
    for _ in 0..40 {
        doc.poll_proof();
        doc.update_display(&projection, 0.016);
        doc.finish_display();
        std::thread::sleep(Duration::from_millis(5));
    }
    let still = doc.provisional_mesh().expect("the edit's preview is still shown");
    assert!(Arc::ptr_eq(&still, &preview), "the late checked mesh of the old part replaced the newer preview");

    // The edit's checked mesh replaces its preview, with the CPU's triangles.
    release(&kernel, new_body);
    frames(&mut doc, |doc| !doc.proof_pending() && doc.provisional_mesh().is_none(), "the edit's proof never landed");
    for _ in 0..20 {
        doc.update_display(&projection, 0.016);
    }
    let (body, checked) = shown(&doc);
    assert_eq!(body, new_body);
    assert!(checked.gpu.is_none() && !checked.positions.is_empty(), "the checked mesh replaced the preview");
    assert!((top(&checked) - top(&preview)).abs() < 0.05, "the checked mesh is of the edited part");
    assert_eq!(kernel.rules.lock().unwrap().cpu_previews, 0, "a CPU preview ran beside the GPU preview");

    // With the GPU preview off, the next edit shows the CPU preview, as without a GPU.
    gpu_preview::uninstall();
    kernel.rules.lock().unwrap().hold_checked = false;
    doc.apply_interactive(Command::EditExtrude { id, params: depth(35.0) }).unwrap();
    frames(&mut doc, |doc| !doc.proof_pending() && doc.provisional_mesh().is_none(), "the CPU edit never landed");
    let (_, cpu) = shown(&doc);
    assert!(cpu.gpu.is_none() && !cpu.positions.is_empty());
    assert!(top(&cpu) > top(&checked) + 1.0);
    assert!(kernel.rules.lock().unwrap().cpu_previews > 0, "the CPU preview is the display without a GPU");
}
