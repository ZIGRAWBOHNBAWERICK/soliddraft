//! The unproved GPU preview on the renderer's own device.
//!
//! When the renderer has a usable hardware adapter, the viewport shows each new body
//! first from the Catallax GPU mesher: the kernel describes the body in floats
//! ([`Kernel::gpu_preview`]), the mesher writes vertices into buffers on the renderer's
//! device, and the renderer draws them without a CPU copy. The checked mesh still comes
//! from the CPU ([`Kernel::checked_rendering`]) and replaces the preview when it is ready.
//! A preview is never shown as checked.
//!
//! WebGL2, a software or fallback adapter, and `SOLIDDRAFT_GPU_PREVIEW=off` keep today's
//! CPU preview, [`Kernel::rendering`], as the display.
//!
//! A face the GPU mesher refuses is drawn from the CPU preview, with its reason logged.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use egui_wgpu::wgpu;
use glam::Vec3;
use soliddraft_kernel_api::{BodyId, FaceId, GpuPreview, Kernel, Mesh, TessellationParams};
use soliddraft_kernel_catallax::gpu_preview::{Meshed, Mesher};

/// The switch that keeps the CPU preview on a machine with a usable GPU.
pub const SWITCH: &str = "SOLIDDRAFT_GPU_PREVIEW";

/// The GPU preview's mesher, compiled on the renderer's device.
pub struct Gpu {
    mesher: Mesher,
    /// Backend and adapter, as each acceptance run records them.
    pub adapter: String,
}

static GPU: Mutex<Option<Arc<Gpu>>> = Mutex::new(None);
static STATUS: Mutex<String> = Mutex::new(String::new());

/// The GPU preview, when one is installed. Display code asks per body, so a preview
/// installed after a document opened applies from its next display.
pub fn active() -> Option<Arc<Gpu>> {
    GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

thread_local! {
    /// Set while the browser's kernel worker builds for a page that shows GPU previews:
    /// the worker makes the checked meshes the page will show, though it has no GPU.
    static CHECKED_ROUTE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the viewport takes its preview from the GPU and its final mesh from
/// [`Kernel::checked_rendering`].
pub fn route_active() -> bool {
    active().is_some() || CHECKED_ROUTE.get()
}

/// Runs `work` on this thread as if the GPU preview were installed, so its displays
/// are the checked meshes. The kernel worker builds this way for a page with a GPU.
pub(crate) fn checked_route<T>(work: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            CHECKED_ROUTE.set(self.0);
        }
    }
    let _reset = Reset(CHECKED_ROUTE.replace(true));
    work()
}

/// What the last install chose, for the logs and the acceptance records: the backend
/// and adapter, or why the CPU preview stays.
pub fn status() -> String {
    STATUS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

fn set_status(status: String) {
    log::info!(target: "soliddraft_app::gpu_preview", "{status}");
    *STATUS.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = status;
}

/// Whether `value`, the switch's setting, turns the GPU preview off.
pub fn switch_off(value: Option<&str>) -> bool {
    value.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "cpu" | "no"))
}

fn switched_off() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    let value = std::env::var(SWITCH).ok();
    #[cfg(target_arch = "wasm32")]
    let value = web_sys::window().and_then(|window| window.location().search().ok()).and_then(|query| {
        query.split(['?', '&']).find_map(|part| part.strip_prefix("gpu-preview=")).map(str::to_owned)
    });
    switch_off(value.as_deref())
}

/// Why `info` is no adapter for the GPU preview: a software rasterizer, a fallback
/// adapter, or a backend without compute.
pub fn refusal(info: &wgpu::AdapterInfo) -> Option<String> {
    refusal_of(&info.name, info.device_type, info.backend)
}

fn refusal_of(name: &str, device_type: wgpu::DeviceType, backend: wgpu::Backend) -> Option<String> {
    let software = ["llvmpipe", "lavapipe", "swiftshader", "softpipe", "software", "microsoft basic render"];
    let lower = name.to_ascii_lowercase();
    if matches!(backend, wgpu::Backend::Gl | wgpu::Backend::Noop) {
        Some(format!("preview.no-device: {backend:?} has no compute for the GPU mesher"))
    } else if device_type == wgpu::DeviceType::Cpu || software.iter().any(|s| lower.contains(s)) {
        Some(format!("preview.no-adapter: {name} is a software adapter"))
    } else {
        None
    }
}

/// The adapter as an acceptance record names it.
pub fn describe(info: &wgpu::AdapterInfo) -> String {
    format!("backend: {:?}, device_type: {:?}, name: {:?}", info.backend, info.device_type, info.name)
}

/// Installs the GPU preview on the renderer's device unless the switch is off or the
/// adapter is not a usable GPU. Returns the reason when it keeps the CPU preview.
pub fn install(info: &wgpu::AdapterInfo, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<(), String> {
    let refused = if switched_off() { Some(format!("preview.gpu-disabled: {SWITCH} is off")) } else { refusal(info) };
    if let Some(reason) = refused {
        set_status(format!("GPU preview off, CPU preview kept ({reason}); adapter {}", describe(info)));
        return Err(reason);
    }
    install_unchecked(device, queue, &describe(info))
}

/// Installs the GPU preview on `device` whatever its adapter. Tests use it with a
/// software adapter, which checks correctness and is never acceptance.
pub fn install_unchecked(device: &wgpu::Device, queue: &wgpu::Queue, adapter: &str) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mesher = pollster::block_on(Mesher::new(device, queue)).inspect_err(|reason| {
            set_status(format!("GPU preview off, CPU preview kept ({reason}); adapter {adapter}"));
        })?;
        let gpu = Gpu { mesher, adapter: adapter.to_owned() };
        *GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(gpu));
        set_status(format!("GPU preview on {adapter}"));
        Ok(())
    }
    #[cfg(target_arch = "wasm32")]
    {
        let (device, queue, adapter) = (device.clone(), queue.clone(), adapter.to_owned());
        wasm_bindgen_futures::spawn_local(async move {
            match Mesher::new(&device, &queue).await {
                Ok(mesher) => {
                    let gpu = Gpu { mesher, adapter: adapter.clone() };
                    *GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(gpu));
                    set_status(format!("GPU preview on {adapter}"));
                }
                Err(reason) => set_status(format!("GPU preview off, CPU preview kept ({reason}); adapter {adapter}")),
            }
        });
        Ok(())
    }
}

/// Removes the GPU preview, so displays from now on take the CPU route.
pub fn uninstall() {
    *GPU.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

/// One face of a GPU preview, as the renderer colours and picks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuFace {
    pub face: FaceId,
    /// Pick id, 0 for a face the id buffer must not name.
    pub pick: u32,
    pub feature: u32,
    /// Index into the body's appearance table.
    pub appearance: u32,
    /// False when the GPU refused the face and the CPU preview draws it instead.
    pub drawn: bool,
}

/// The GPU half of a display mesh: buffers on the preview's device and one entry per
/// face of the description.
#[derive(Clone)]
pub struct GpuFaces {
    /// The body the description came from, for the kernel's point queries.
    pub body: BodyId,
    pub meshed: Arc<Meshed>,
    pub faces: Vec<GpuFace>,
    /// The body's box, since no CPU positions exist for the GPU faces.
    pub bounds: (Vec3, Vec3),
}

impl GpuFaces {
    /// Whether the GPU draws the face with pick id `pick`.
    pub fn draws(&self, pick: u32) -> bool {
        pick != 0 && self.faces.iter().any(|face| face.pick == pick && face.drawn)
    }

    /// The face the GPU draws under pick id `pick`.
    pub fn face(&self, pick: u32) -> Option<FaceId> {
        self.faces.iter().find(|face| face.pick == pick && face.drawn).map(|face| face.face)
    }
}

impl PartialEq for GpuFaces {
    fn eq(&self, other: &Self) -> bool {
        self.body == other.body
            && Arc::ptr_eq(&self.meshed, &other.meshed)
            && self.faces == other.faces
            && self.bounds == other.bounds
    }
}

impl std::fmt::Debug for GpuFaces {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuFaces").field("faces", &self.faces).field("bounds", &self.bounds).finish_non_exhaustive()
    }
}

/// A body's GPU preview, meshed on the preview's device, with the CPU preview triangles
/// of the faces the mesher refused.
pub(crate) struct Meshing {
    pub preview: GpuPreview,
    pub meshed: Meshed,
    /// The CPU preview restricted to the refused faces; empty when none refused.
    pub refused: Mesh,
}

/// The kernel's description of `body` for the installed GPU preview, or `None` when
/// there is no GPU preview or the kernel has no description of the body.
fn describe_body(kernel: &dyn Kernel, body: BodyId, params: &TessellationParams) -> Option<(Arc<Gpu>, GpuPreview)> {
    let gpu = active()?;
    match kernel.gpu_preview(body, params) {
        Ok(Some(preview)) => Some((gpu, preview)),
        Ok(None) => None,
        Err(error) => {
            log::warn!(target: "soliddraft_app::gpu_preview", "no GPU preview of {body:?}: {error}");
            None
        }
    }
}

/// The mesher's answer as a [`Meshing`]: the faces it refused come from the CPU
/// preview, each with its reason logged. `None` when the device failed.
fn finish(
    kernel: &dyn Kernel,
    body: BodyId,
    params: &TessellationParams,
    preview: GpuPreview,
    meshed: Result<Meshed, String>,
) -> Option<Meshing> {
    let meshed = match meshed {
        Ok(meshed) => meshed,
        Err(reason) => {
            log::warn!(target: "soliddraft_app::gpu_preview", "GPU preview of {body:?} fell back to the CPU: {reason}");
            return None;
        }
    };
    let refused: HashSet<FaceId> =
        meshed.faces.iter().filter(|face| face.draw.is_err()).map(|face| face.face).collect();
    for face in meshed.faces.iter().filter(|face| face.draw.is_err()) {
        log::info!(
            target: "soliddraft_app::gpu_preview",
            "face {:?} of {body:?} drawn from the CPU preview: {}",
            face.face,
            face.draw.as_ref().err().copied().unwrap_or_default()
        );
    }
    let refused = if refused.is_empty() {
        Mesh::default()
    } else {
        match kernel.preview(body, params) {
            Ok(full) => only_faces(full, &refused),
            Err(error) => {
                log::warn!(target: "soliddraft_app::gpu_preview", "CPU preview of refused faces of {body:?}: {error}");
                return None;
            }
        }
    };
    Some(Meshing { preview, meshed, refused })
}

/// Meshes `body` on the GPU preview now. `None` when there is no GPU preview, the
/// kernel has no description of the body, or the device failed; the caller keeps the
/// CPU route. The browser's device answers later: see [`start`].
pub(crate) fn mesh_body(kernel: &dyn Kernel, body: BodyId, params: &TessellationParams) -> Option<Meshing> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (gpu, preview) = describe_body(kernel, body, params)?;
        let meshed = pollster::block_on(gpu.mesher.mesh(&preview, params));
        finish(kernel, body, params, preview, meshed)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (kernel, body, params);
        None
    }
}

/// A GPU preview being meshed. The desktop's is ready at once; the browser's device
/// answers after the frame.
#[derive(Clone)]
pub(crate) struct Pending(Arc<Mutex<Option<Option<Meshing>>>>);

impl Pending {
    /// `None` while the device has not answered; then the preview, or `None` when it
    /// failed and the caller keeps the CPU route. Taken once.
    pub(crate) fn take(&self) -> Option<Option<Meshing>> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }
}

/// Starts meshing `body` on the GPU preview. `None` when there is no GPU preview or the
/// kernel has no description of the body.
pub(crate) fn start(kernel: Arc<dyn Kernel>, body: BodyId, params: TessellationParams) -> Option<Pending> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let meshing = mesh_body(kernel.as_ref(), body, &params)?;
        Some(Pending(Arc::new(Mutex::new(Some(Some(meshing))))))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let (gpu, preview) = describe_body(kernel.as_ref(), body, &params)?;
        let slot = Arc::new(Mutex::new(None));
        let answer = slot.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let meshed = gpu.mesher.mesh(&preview, &params).await;
            let meshing = finish(kernel.as_ref(), body, &params, preview, meshed);
            *answer.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(meshing);
            if let Some(ctx) = crate::web::context() {
                ctx.request_repaint();
            }
        });
        Some(Pending(slot))
    }
}

/// `mesh` with only the triangles of `faces`.
fn only_faces(mut mesh: Mesh, faces: &HashSet<FaceId>) -> Mesh {
    let mut indices = Vec::new();
    let mut triangle_faces = Vec::new();
    for (triangle, face) in mesh.indices.as_chunks::<3>().0.iter().zip(&mesh.triangle_faces) {
        if faces.contains(face) {
            indices.extend_from_slice(triangle);
            triangle_faces.push(*face);
        }
    }
    mesh.indices = indices;
    mesh.triangle_faces = triangle_faces;
    mesh
}

/// The GPU half of a body's display mesh, with its faces numbered as `picks` numbers
/// them and marked with the features that made them.
pub(crate) fn faces(
    body: BodyId,
    meshed: Meshed,
    picks: &HashMap<FaceId, u32>,
    feature: impl Fn(FaceId) -> u32,
    bounds: (Vec3, Vec3),
) -> GpuFaces {
    GpuFaces {
        body,
        faces: meshed
            .faces
            .iter()
            .map(|face| GpuFace {
                face: face.face,
                pick: picks.get(&face.face).copied().unwrap_or(0),
                feature: feature(face.face),
                appearance: 0,
                drawn: face.draw.is_ok(),
            })
            .collect(),
        meshed: Arc::new(meshed),
        bounds,
    }
}

/// Mirrors `Params` in gpu_preview.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ConvertParams {
    origin: [f32; 4],
    triangles: u32,
    row: u32,
    _pad: [u32; 2],
}

/// The compute pass that turns the mesher's vertices into the renderer's face vertices
/// and silhouette inputs, with each face's pick, feature and appearance.
pub(crate) struct Convert {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    per_axis: u32,
}

/// The faces the CPU draws in a GPU preview: those the mesher refused, already in the
/// renderer's vertex and silhouette layouts.
pub(crate) struct CpuPart<'a> {
    pub vertices: &'a [u8],
    pub indices: &'a [u32],
    pub silhouettes: &'a [u8],
}

/// What [`Convert::run`] wrote: buffers the renderer draws.
pub(crate) struct Converted {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub index_count: u32,
    pub silhouettes: wgpu::Buffer,
    pub silhouette_count: u32,
}

const GROUP: u32 = 64;
/// Bytes of the renderer's `FaceVertex` and `SilhouetteTriangle`.
pub(crate) const FACE_VERTEX_BYTES: u64 = 36;
pub(crate) const SILHOUETTE_BYTES: u64 = 72;

impl Convert {
    pub(crate) fn new(device: &wgpu::Device) -> Convert {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GPU preview to face vertices"),
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_preview.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("GPU preview to face vertices"),
            layout: None,
            module: &shader,
            entry_point: Some("convert"),
            compilation_options: Default::default(),
            cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0);
        Convert { pipeline, layout, per_axis: device.limits().max_compute_workgroups_per_dimension.max(1) }
    }

    /// Writes face vertices for the GPU faces of `gpu`, followed by the CPU part's, and
    /// indices for both, the CPU ones shifted past the GPU vertices. The silhouette
    /// inputs of the GPU triangles come first, then the CPU part's. `table` is the
    /// length of the body's appearance table.
    pub(crate) fn run(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &GpuFaces,
        table: usize,
        cpu: CpuPart<'_>,
    ) -> Converted {
        let CpuPart { vertices: cpu_vertices, indices: cpu_indices, silhouettes: cpu_silhouettes } = cpu;
        use wgpu::util::DeviceExt;
        let meshed = &gpu.meshed;
        let (gpu_vertices, gpu_indices) = match &meshed.buffers {
            Some(buffers) => {
                let end = meshed.faces.iter().filter_map(|face| face.draw.as_ref().ok()).map(|range| range.end).max();
                (buffers.vertex_count, end.unwrap_or(0))
            }
            None => (0, 0),
        };
        let triangles = gpu_indices / 3;
        let mut slots = vec![0u32; triangles as usize];
        for (slot, face) in meshed.faces.iter().enumerate() {
            if let Ok(range) = &face.draw {
                slots[(range.start / 3) as usize..(range.end / 3) as usize].fill(slot as u32);
            }
        }
        let table_entries: Vec<[u32; 4]> = gpu
            .faces
            .iter()
            .map(|face| {
                let appearance = if (face.appearance as usize) < table { face.appearance } else { 0 };
                [face.pick, face.feature, appearance, 0]
            })
            .collect();
        let shifted: Vec<u32> = cpu_indices.iter().map(|i| i + gpu_vertices).collect();
        let vertex_bytes = u64::from(gpu_vertices) * FACE_VERTEX_BYTES + cpu_vertices.len() as u64;
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU preview faces"),
            size: vertex_bytes.max(FACE_VERTEX_BYTES),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_count = gpu_indices + shifted.len() as u32;
        let indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU preview indices"),
            size: (u64::from(index_count) * 4).max(4),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let silhouette_bytes = u64::from(triangles) * SILHOUETTE_BYTES + cpu_silhouettes.len() as u64;
        let silhouettes = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU preview silhouette triangles"),
            size: silhouette_bytes.max(SILHOUETTE_BYTES),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        if !cpu_vertices.is_empty() {
            queue.write_buffer(&vertices, u64::from(gpu_vertices) * FACE_VERTEX_BYTES, cpu_vertices);
        }
        if !shifted.is_empty() {
            queue.write_buffer(&indices, u64::from(gpu_indices) * 4, bytemuck::cast_slice(&shifted));
        }
        if !cpu_silhouettes.is_empty() {
            queue.write_buffer(&silhouettes, u64::from(triangles) * SILHOUETTE_BYTES, cpu_silhouettes);
        }
        if let Some(buffers) = meshed.buffers.as_ref().filter(|_| triangles > 0) {
            let index_copy = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GPU preview indices for conversion"),
                size: u64::from(gpu_indices) * 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let groups = triangles.div_ceil(GROUP);
            let x = groups.min(self.per_axis);
            let origin = meshed.origin.map(|v| v as f32);
            let params = ConvertParams {
                origin: [origin[0], origin[1], origin[2], 0.0],
                triangles,
                row: x * GROUP,
                _pad: [0; 2],
            };
            let make = |label: &str, contents: &[u8]| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents,
                    usage: wgpu::BufferUsages::STORAGE,
                })
            };
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("GPU preview conversion"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let slots = make("GPU preview triangle faces", bytemuck::cast_slice(&slots));
            let table = make("GPU preview face table", bytemuck::cast_slice(&table_entries));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("GPU preview conversion"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: buffers.vertex.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: index_copy.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: slots.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: table.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 5, resource: vertices.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 6, resource: silhouettes.as_entire_binding() },
                ],
            });
            let mut encoder = device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("GPU preview conversion") });
            encoder.copy_buffer_to_buffer(&buffers.index, 0, &index_copy, 0, u64::from(gpu_indices) * 4);
            encoder.copy_buffer_to_buffer(&buffers.index, 0, &indices, 0, u64::from(gpu_indices) * 4);
            {
                let mut pass = encoder.begin_compute_pass(&Default::default());
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
            }
            queue.submit([encoder.finish()]);
        }
        let silhouette_count = triangles + (cpu_silhouettes.len() as u64 / SILHOUETTE_BYTES) as u32;
        Converted { vertices, indices, index_count, silhouettes, silhouette_count }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conversion_shader_validates() {
        let source = include_str!("gpu_preview.wgsl");
        let module =
            naga::front::wgsl::parse_str(source).unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .unwrap();
    }

    #[test]
    fn the_switch_and_software_adapters_keep_the_cpu_preview() {
        assert!(switch_off(Some("off")) && switch_off(Some(" CPU ")) && switch_off(Some("0")));
        assert!(!switch_off(None) && !switch_off(Some("on")));
        use wgpu::{Backend, DeviceType};
        assert_eq!(refusal_of("Apple M2", DeviceType::IntegratedGpu, Backend::Metal), None);
        let software = refusal_of("llvmpipe (LLVM 17.0.6, 256 bits)", DeviceType::Cpu, Backend::Vulkan);
        assert!(software.unwrap().starts_with("preview.no-adapter"));
        let fallback = refusal_of("Apple M2", DeviceType::Cpu, Backend::Metal);
        assert!(fallback.unwrap().starts_with("preview.no-adapter"));
        let webgl = refusal_of("ANGLE", DeviceType::IntegratedGpu, Backend::Gl);
        assert!(webgl.unwrap().starts_with("preview.no-device"));
    }
}
