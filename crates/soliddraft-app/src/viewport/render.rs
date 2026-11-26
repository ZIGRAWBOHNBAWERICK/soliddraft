//! wgpu side of the viewport. Runs inside egui-wgpu's paint callback: the
//! scene is drawn to offscreen targets in `prepare` (MSAA colour, depth, and
//! a two-channel 32-bit id buffer), a small window of the id buffer around the
//! cursor is copied back for picking, and `paint` blits the colour result into
//! egui's own render pass.
//!
//! The id buffer is `Rg32Uint`. The first channel is 0 for the open part and,
//! for an assembly, the occurrence's row plus one; the second is the entity: a
//! two-bit kind and a 30-bit id. Both are good for this session only.
//!
//! Passes, in order: the shadow map from the light fixed above the model (only
//! when the geometry changed), the ambient occlusion prepass, sampling and blur
//! (full tier only), the colour pass (faces, the ground shadow, the origin planes, the
//! grid, edges as wide lines, ghosts) and the id pass. Depth is reversed (near 1, far 0) on every
//! camera pass. `look.rs` holds the presets and the tiers.

use super::assembly::{AssemblyFrame, AssemblyRenderer};
use super::grid::{self, GridFrame};
use super::planes::{self, PlaneFrame};
use crate::document::DisplayMesh;
use crate::viewport::environment;
use crate::viewport::look::{Preset, Tier};
use egui_wgpu::wgpu::{self, util::DeviceExt};
use glam::{Mat4, Vec3};
use soliddraft_kernel_api::BodyId;
use soliddraft_materials::{Appearance, Shading};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const KIND_SHIFT: u32 = 30;
/// Body flag of the line-only body that holds the thread marks (see `scene.wgsl`).
const THREAD_MARKS: u32 = 16;
/// The rest of an assembly around a part edited in context: drawn as a ghost, with its
/// own opacity, in a dimmed grey (scene.wgsl `context`).
const CONTEXT: u32 = 2 | 256;
pub const KIND_MASK: u32 = 0b11 << KIND_SHIFT;
pub const ID_MASK: u32 = !KIND_MASK;

/// What the id buffer reported under the cursor: a pick id (see
/// `Document::pick`) with the kind it was drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picked {
    Face(u32),
    Edge(u32),
    Vertex(u32),
}

impl Picked {
    pub fn decode(v: u32) -> Option<Self> {
        let id = v & ID_MASK;
        if id == 0 {
            return None;
        }
        match v >> KIND_SHIFT {
            1 => Some(Picked::Face(id)),
            2 => Some(Picked::Edge(id)),
            3 => Some(Picked::Vertex(id)),
            _ => None,
        }
    }

    pub fn pick(self) -> u32 {
        match self {
            Picked::Face(p) | Picked::Edge(p) | Picked::Vertex(p) => p,
        }
    }
}

pub use soliddraft_document::display_state::DisplayMode;

/// Existing document identity, display version and retained body; never sent to the GPU.
pub type PickRevision = (soliddraft_document::DocumentId, u64, Option<BodyId>);

/// What a frame was drawn for. An id-buffer pixel means the same thing only under all of
/// these: the view, the projection with its aspect, and the size of the render target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PickView {
    pub view: Mat4,
    pub proj: Mat4,
    pub size: [u32; 2],
}

/// Window of the id buffer around the cursor, read back from the GPU.
#[derive(Clone, Debug, Default)]
pub struct PickSample {
    pub origin: [u32; 2],
    pub width: u32,
    pub height: u32,
    /// Texels: occurrence (0 for the open part) and entity.
    pub data: Vec<[u32; 2]>,
    pub serial: u64,
    pub revision: Option<PickRevision>,
    /// The frame the sample was read from.
    pub seen: PickView,
}

impl PickSample {
    /// The entity under a pixel: a kind and an id (see [`Picked::decode`]).
    pub fn at(&self, x: u32, y: u32) -> Option<u32> {
        self.texel(x, y).map(|t| t[1])
    }

    /// Both channels under a pixel.
    pub fn texel(&self, x: u32, y: u32) -> Option<[u32; 2]> {
        if x < self.origin[0] || y < self.origin[1] {
            return None;
        }
        let (lx, ly) = (x - self.origin[0], y - self.origin[1]);
        if lx >= self.width || ly >= self.height {
            return None;
        }
        self.data.get((ly * PICK_ROW_TEXELS + lx) as usize).copied()
    }
}

/// Everything the UI thread hands the GPU thread for one frame.
pub struct FrameInput {
    pub view: Mat4,
    /// Reversed-depth projection (`Camera::projection`).
    pub proj: Mat4,
    pub eye: Vec3,
    /// Camera right, up and toward-the-viewer directions; the studio rig follows them.
    pub basis: [Vec3; 3],
    pub orthographic: bool,
    /// Distance from the eye to the orbit target.
    pub distance: f32,
    pub size_px: [u32; 2],
    pub pixels_per_point: f32,
    pub display: DisplayMode,
    pub preset: Preset,
    pub controls: Controls,
    /// The gradient and optional screen grid composited behind the model.
    pub backdrop: super::scenes::Backdrop,
    pub backdrop_picture: Option<Arc<image::RgbaImage>>,
    /// Whether the ground under the model shows the model's shadow.
    pub floor: bool,
    pub hover_id: u32,
    pub selected_id: u32,
    pub selected_feature: u32,
    pub selected_body: Option<BodyId>,
    pub cursor_px: Option<[u32; 2]>,
    /// Document meshes with their version; re-uploaded when the version changes.
    pub meshes: (u64, Vec<(BodyId, Arc<DisplayMesh>)>),
    /// Added or removed material, with whether it removes.
    pub ghost: Option<(Arc<DisplayMesh>, bool)>,
    /// The rest of an assembly, dimmed, while the part is edited in its context. Never
    /// picked: the part's tab picks it on the CPU (see `in_context`).
    pub context: Option<Arc<DisplayMesh>>,
    /// Drawn instead of the document bodies when set.
    pub preview: Option<Arc<DisplayMesh>>,
    /// The marks of the part's cosmetic threads: lines only, drawn with the bodies and
    /// never picked (see `threads`).
    pub threads: Option<Arc<DisplayMesh>>,
    /// Only revision-bound application previews participate in picking.
    pub preview_pickable: bool,
    pub pick_revision: Option<PickRevision>,
    pub pick_out: Arc<Mutex<PickSample>>,
    /// An assembly to draw with the part, if one is open.
    pub assembly: Option<AssemblyFrame>,
    /// The modelling grid, when one shows.
    pub grid: Option<GridFrame>,
    /// The origin planes, when any shows.
    pub planes: Option<PlaneFrame>,
    #[cfg(target_arch = "wasm32")]
    pub(super) part_probe: Option<PartProbe>,
}

#[cfg(target_arch = "wasm32")]
pub(super) struct PartProbe {
    pub camera: super::camera::Camera,
    pub unproved: bool,
    pub pending: bool,
    /// The part's active display state, by name.
    pub display_state: Option<String>,
    /// The display mode the frame is drawn in.
    pub display_mode: String,
    /// How many body meshes the frame was given to draw.
    pub visible_meshes: usize,
    pub chord_mm: f64,
    pub angle_deg: f64,
    pub standard_chord_mm: f64,
    pub curves_settled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    pub lighting: crate::graphics::Lighting,
    pub supersample: bool,
    pub vertex_markers: bool,
    pub xray_references: bool,
    pub hidden_selected_edges: bool,
    pub tint_preview: bool,
    pub red_removal: bool,
    /// The hatch of a section view's cap, while a section view is shown.
    pub section: Option<crate::graphics::SectionHatch>,
    pub edge_color: Option<crate::theme::Rgb>,
    pub selection_color: Option<crate::theme::Rgb>,
}

impl Default for Controls {
    fn default() -> Self {
        Self {
            lighting: Default::default(),
            supersample: false,
            vertex_markers: false,
            xray_references: false,
            hidden_selected_edges: false,
            tint_preview: true,
            red_removal: true,
            section: None,
            edge_color: None,
            selection_color: None,
        }
    }
}

pub(crate) const DEFAULT_EDGE: egui::Color32 = egui::Color32::from_rgb(41, 48, 64);
pub(crate) const DEFAULT_SELECTION: egui::Color32 = egui::Color32::from_rgb(31, 94, 209);

impl Controls {
    pub(super) fn color_uniform(color: Option<crate::theme::Rgb>) -> [f32; 4] {
        color.map_or([0.0; 4], |c| egui::Rgba::from(egui::Color32::from(c)).to_array())
    }
}

impl FrameInput {
    fn pick_view(&self) -> PickView {
        PickView { view: self.view, proj: self.proj, size: self.size_px }
    }
}

pub struct ViewportCallback(pub FrameInput);

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(r) = resources.get_mut::<ViewportRenderer>() {
            r.prepare(device, queue, &self.0);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(r) = resources.get::<ViewportRenderer>() {
            r.paint(pass);
            #[cfg(target_arch = "wasm32")]
            if let Some(probe) = &self.0.part_probe
                && r.targets.is_some()
            {
                let triangles = if r.draws_faces() {
                    ViewportRenderer::scene_bodies(&r.preview, &r.bodies)
                        .map(|body| (body.index_count / 3) as usize)
                        .sum()
                } else {
                    0_usize
                };
                let threads = r.scene_threads().map_or(0, |threads| threads.line_count as usize);
                crate::web::show_part_view(&probe.camera, _info.viewport, threads);
                crate::web::show_viewport(serde_json::json!({
                    "label": probe.unproved.then_some("unproved preview"),
                    "triangles": triangles,
                    "pending": probe.pending,
                    "preview": r.preview.is_some(),
                    "display_state": probe.display_state,
                    "display_mode": probe.display_mode,
                    "visible_meshes": probe.visible_meshes,
                    "chord_mm": probe.chord_mm,
                    "angle_deg": probe.angle_deg,
                    "standard_chord_mm": probe.standard_chord_mm,
                    "curves_settled": probe.curves_settled,
                }));
            }
        }
    }
}

/// Mirrors `Globals` in scene.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    view_proj: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    proj: [[f32; 4]; 4],
    inv_proj: [[f32; 4]; 4],
    sun_view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    viewport: [f32; 4],
    pull: [f32; 4],
    key: [f32; 4],
    fill: [f32; 4],
    rim: [f32; 4],
    sun: [f32; 4],
    ground: [f32; 4],
    shadow: [f32; 4],
    look: [f32; 4],
    grid_anchor: [f32; 4],
    grid_u: [f32; 4],
    grid_v: [f32; 4],
    grid_ink: [f32; 4],
    grid_lines: [f32; 4],
    plane_size: [f32; 4],
    planes: [[f32; 4]; 3],
    edge_color: [f32; 4],
    selection_color: [f32; 4],
    sh: [[f32; 4]; 9],
    hover_id: u32,
    selected_id: u32,
    /// Faces made by this feature are highlighted (0 for none).
    selected_feature: u32,
    _pad: u32,
}

/// Mirrors `Backdrop` in scene.wgsl. Stop RGB is linear; w is the stop position.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Backdrop {
    stops: [[f32; 4]; 5],
    radial: [f32; 4],
    grid: [f32; 4],
    picture_uv: [f32; 4],
}

impl Backdrop {
    fn from_srgb(bg: super::scenes::Backdrop, size: [u32; 2], ppp: f32, picture: Option<&image::RgbaImage>) -> Self {
        let picture_uv = picture.map_or([1.0, 1.0, 0.0, 0.0], |image| {
            let viewport_aspect = size[0].max(1) as f32 / size[1].max(1) as f32;
            let image_aspect = image.width() as f32 / image.height() as f32;
            let x = (viewport_aspect / image_aspect).min(1.0);
            let y = (image_aspect / viewport_aspect).min(1.0);
            [x, y, (1.0 - x) * 0.5, (1.0 - y) * 0.5]
        });
        Self {
            stops: bg.stops.map(|(color, at)| {
                let [r, g, b, _] = egui::Rgba::from(color).to_array();
                [r, g, b, at]
            }),
            radial: bg.radial,
            grid: [bg.grid[0], bg.grid[1], size[0] as f32 / ppp, size[1] as f32 / ppp],
            picture_uv,
        }
    }
}

/// Appearances one body can show; a face indexing past them shows the first.
pub const MAX_APPEARANCES: usize = 64;
/// The opacity from which a face counts as opaque and draws in the opaque pass;
/// `OPAQUE` in scene.wgsl is the same.
const OPAQUE: f32 = 0.999;

/// One appearance as `Shade` in scene.wgsl reads it.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuShade {
    /// Linear base colour and opacity.
    base: [f32; 4],
    /// Linear emitted light; w unused.
    emissive: [f32; 4],
    /// Metallic, roughness; zw unused.
    params: [f32; 4],
}

impl From<Shading> for GpuShade {
    fn from(s: Shading) -> Self {
        let e = s.emissive;
        GpuShade { base: s.base_color, emissive: [e[0], e[1], e[2], 0.0], params: [s.metallic, s.roughness, 0.0, 0.0] }
    }
}

/// Mirrors `Body` in scene.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BodyUniform {
    flags: u32,
    _pad: [u32; 3],
    tint: [f32; 4],
    shades: [GpuShade; MAX_APPEARANCES],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FaceVertex {
    position: [f32; 3],
    normal: [f32; 3],
    face_id: u32,
    feature_id: u32,
    appearance: u32,
}

/// One edge segment, drawn as a quad.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LineInstance {
    a: [f32; 3],
    b: [f32; 3],
    id: u32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TaggedVertex {
    position: [f32; 3],
    id: u32,
}

struct GpuBody {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    lines: wgpu::Buffer,
    line_count: u32,
    points: wgpu::Buffer,
    point_count: u32,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    bounds: Option<(Vec3, Vec3)>,
    /// Some face shows an appearance that is not fully opaque, so the body draws in
    /// the see-through pass as well as the opaque one.
    see_through: bool,
}

/// The ambient occlusion targets, full tier only.
struct AoTargets {
    depth: wgpu::TextureView,
    normals: wgpu::TextureView,
    raw: wgpu::TextureView,
    sample_bind_group: wgpu::BindGroup,
    blur_bind_group: wgpu::BindGroup,
    blurred: wgpu::TextureView,
}

struct Targets {
    size: [u32; 2],
    color_msaa: Option<wgpu::TextureView>,
    scale: u32,
    color: wgpu::TextureView,
    depth_msaa: wgpu::TextureView,
    id: wgpu::Texture,
    id_view: wgpu::TextureView,
    id_depth: wgpu::TextureView,
    blit_bind_group: wgpu::BindGroup,
    ao: Option<AoTargets>,
    /// Group 2 of the scene passes: the occlusion result, or a white texel.
    frame_bind_group: wgpu::BindGroup,
}

/// The shadow light's frame for one set of bounds: where the map looks from and
/// where the ground sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunFrame {
    pub view_proj: Mat4,
    /// Centre of the ground disc, on the plane under the model's lowest point.
    pub ground: Vec3,
    pub ground_radius: f32,
    /// Radius of the model's bounding sphere.
    pub radius: f32,
    /// World size of one shadow-map texel.
    pub texel: f32,
}

/// The light fixed above the model, tilted like the environment's softbox so the
/// shadow falls beside the part and not only under it. It is fixed to the model, not
/// the camera, so the map is redrawn only when the geometry changes.
pub fn sun_frame(lo: Vec3, hi: Vec3) -> SunFrame {
    sun_frame_with_ground(lo, hi, true)
}

fn sun_frame_with_ground(lo: Vec3, hi: Vec3, follows_model: bool) -> SunFrame {
    let center = (lo + hi) / 2.0;
    let radius = ((hi - lo).length() / 2.0).max(1.0);
    let toward = environment::softbox_direction();
    let ground_radius = radius * 2.6;
    let eye = center + toward * radius * 3.0;
    let up = if toward.z.abs() > 0.99 { Vec3::Y } else { Vec3::Z };
    let view = glam::camera::rh::view::look_at_mat4(eye, center, up);
    let half = radius * 1.05;
    // Deep enough to reach the far rim of the ground disc behind the model.
    let (ground, reach) = if follows_model {
        let height = lo.z - radius * 1e-3;
        (Vec3::new(center.x, center.y, height), (ground_radius * ground_radius + (hi.z - lo.z).powi(2)).sqrt())
    } else {
        let drift = center.z / toward.z;
        let mut projected = center - toward * drift;
        projected.z = 0.0;
        (projected, ground_radius + drift.abs())
    };
    let far = radius * 3.0 + reach + radius;
    let proj = glam::camera::rh::proj::directx::orthographic(-half, half, -half, half, radius, far);
    SunFrame { view_proj: proj * view, ground, ground_radius, radius, texel: 2.0 * half / SHADOW_SIZE as f32 }
}

const MSAA: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Uint;
/// Bytes per id texel.
const ID_TEXEL: u32 = 8;
/// Depth24Plus filters with a comparison sampler on WebGL2 as well.
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
const SHADOW_SIZE: u32 = 2048;
const NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
/// Edge width in points.
const EDGE_WIDTH: f32 = 1.3;
/// How far edges are pulled toward the viewer, as a fraction of their distance.
const EDGE_PULL: f32 = 0.003;
/// Reversed depth: the near plane clears to 1 and far is 0.
const DEPTH_CLEAR: f32 = 0.0;
const DEPTH_COMPARE: wgpu::CompareFunction = wgpu::CompareFunction::GreaterEqual;
/// Texels per read-back row: 512 bytes, a multiple of the 256-byte copy alignment.
pub const PICK_ROW_TEXELS: u32 = 64;
pub const PICK_ROWS: u32 = 32;

/// The inputs of the last pick read back and of the one in flight. A pick counts as read
/// only once its sample is published, so one whose readback fails is read again.
struct PickCache<T> {
    read: Option<T>,
    in_flight: Option<T>,
}

impl<T> Default for PickCache<T> {
    fn default() -> Self {
        PickCache { read: None, in_flight: None }
    }
}

impl<T: PartialEq> PickCache<T> {
    /// Whether a pick of `inputs` must be read: none with these inputs was published yet.
    fn wants(&self, inputs: &T) -> bool {
        self.read.as_ref() != Some(inputs)
    }

    fn issued(&mut self, inputs: T) {
        self.in_flight = Some(inputs);
    }

    fn published(&mut self) {
        if let Some(inputs) = self.in_flight.take() {
            self.read = Some(inputs);
        }
    }

    fn failed(&mut self) {
        self.in_flight = None;
    }
}

/// What the id buffer under the cursor shows in a part: the cursor, the pick revision,
/// the view, the bodies drawn, the preview drawn in their place and the display choices.
/// The origin planes and the grid are not drawn into it: a plane is hit on the CPU
/// where the sample shows no body. Their frames also carry the hovered plane's
/// highlight, which a pick sets, so keying on them read every pick over a plane twice.
#[derive(PartialEq)]
struct PickInputs {
    cursor: [u32; 2],
    revision: Option<PickRevision>,
    view: PickView,
    meshes: (u64, Vec<BodyId>),
    preview: Option<(usize, bool)>,
    display: DisplayMode,
}

impl PickInputs {
    fn of(input: &FrameInput, cursor: [u32; 2]) -> Self {
        PickInputs {
            cursor,
            revision: input.pick_revision,
            view: input.pick_view(),
            meshes: (input.meshes.0, input.meshes.1.iter().map(|(id, _)| *id).collect()),
            preview: input.preview.as_ref().map(|mesh| (Arc::as_ptr(mesh) as usize, input.preview_pickable)),
            display: input.display,
        }
    }
}

#[derive(Default)]
struct Readback {
    /// A copy has been submitted and its map callback has not fired yet.
    pending: bool,
    /// The map callback fired; the buffer can be read.
    ready: bool,
    origin: [u32; 2],
    serial: u64,
    revision: Option<PickRevision>,
    seen: PickView,
}

struct Layouts {
    /// Group 0 of the passes that need only the uniforms (shadow, occlusion).
    uniforms: wgpu::BindGroupLayout,
    body: wgpu::BindGroupLayout,
    frame: wgpu::BindGroupLayout,
    ao_inputs: wgpu::BindGroupLayout,
    blit: wgpu::BindGroupLayout,
}

pub struct ViewportRenderer {
    tier: Tier,
    samples: u32,
    face_pipeline: wgpu::RenderPipeline,
    /// The faces whose appearance is not opaque: blended over everything opaque,
    /// both sides, writing no depth, so what is behind or inside them shows.
    see_through_pipeline: wgpu::RenderPipeline,
    ground_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    plane_pipeline: wgpu::RenderPipeline,
    xray_planes_pipeline: wgpu::RenderPipeline,
    hidden_edges_pipeline: wgpu::RenderPipeline,
    markers_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    shadow_pipeline: wgpu::RenderPipeline,
    id_face_pipeline: wgpu::RenderPipeline,
    id_edge_pipeline: wgpu::RenderPipeline,
    id_vertex_pipeline: wgpu::RenderPipeline,
    /// Normal prepass, occlusion and blur; full tier only.
    ao_pipelines: Option<[wgpu::RenderPipeline; 3]>,
    blit_pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    pick_globals: wgpu::Buffer,
    pick_globals_bind_group: wgpu::BindGroup,
    backdrop: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    uniforms_bind_group: wgpu::BindGroup,
    layouts: Layouts,
    sampler: wgpu::Sampler,
    shadow_view: wgpu::TextureView,
    sun: Option<SunFrame>,
    /// What the shadow map was last drawn from: mesh version and preview.
    shadow_key: Option<(u64, usize, bool)>,
    white: wgpu::TextureView,
    transparent: wgpu::TextureView,
    backdrop_picture: Option<(Arc<image::RgbaImage>, wgpu::TextureView)>,
    sh: [[f32; 4]; 9],
    targets: Option<Targets>,
    bodies: HashMap<BodyId, GpuBody>,
    /// `u64::MAX` means nothing uploaded yet, so version 0 (an empty document) still uploads.
    mesh_version: u64,
    /// Keyed by the `Arc` address of the ghost mesh; never dereferenced.
    ghost: Option<(usize, u32, GpuBody)>,
    /// Keyed like `ghost`.
    context: Option<(usize, GpuBody)>,
    preview: Option<(usize, u32, GpuBody)>,
    threads: Option<(usize, GpuBody)>,
    pick_buffer: wgpu::Buffer,
    readback: Arc<Mutex<Readback>>,
    /// What the last pick read back showed. On WebGL reading a pick waits for the frame on
    /// the GPU, which software rendering takes most of a second to draw, so the pick under
    /// a resting pointer is read again only when something it shows can have changed.
    picks: PickCache<PickInputs>,
    display: DisplayMode,
    frame: u64,
    assembly: AssemblyRenderer,
}

fn picture_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    [width, height]: [u32; 2],
    rgba: &[u8],
) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("viewport background picture"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            rgba,
        )
        .create_view(&Default::default())
}

impl ViewportRenderer {
    pub fn graphics_device(
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        requested: u32,
    ) -> crate::graphics::GraphicsDevice {
        let info = adapter.get_info();
        let limits = device.limits();
        let flags = |format: wgpu::TextureFormat| {
            if device.features().contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
                adapter.get_texture_format_features(format).flags
            } else {
                format.guaranteed_format_features(device.features()).flags
            }
        };
        let color = flags(COLOR_FORMAT);
        let depth = flags(DEPTH_FORMAT);
        let mut samples = vec![1];
        if color.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE) {
            samples.extend(
                [2, 4, 8, 16]
                    .into_iter()
                    .filter(|&n| color.sample_count_supported(n) && depth.sample_count_supported(n)),
            );
        }
        let active_samples = if samples.contains(&requested) {
            requested
        } else if samples.contains(&MSAA) {
            MSAA
        } else {
            1
        };
        crate::graphics::GraphicsDevice {
            gpu: if info.name.is_empty() { "Name unavailable".into() } else { info.name },
            renderer: format!("{:?}", info.backend),
            samples,
            active_samples,
            tier: Tier::of(info.backend, &limits),
            display_budget: soliddraft_tessellation::DeviceBudget::from_buffer_limit(limits.max_buffer_size),
        }
    }

    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, target_format: wgpu::TextureFormat, tier: Tier) -> Self {
        Self::with_samples(device, queue, target_format, tier, MSAA)
    }

    pub fn with_samples(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        tier: Tier,
        samples: u32,
    ) -> Self {
        log::info!("viewport renderer: {} tier", tier.label());
        let shader = device.create_shader_module(wgpu::include_wgsl!("scene.wgsl"));

        let stages = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let uniform_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: stages,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture_entry = |binding: u32, sample_type, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture { sample_type, view_dimension, multisampled: false },
            count: None,
        };
        let sampler_entry = |binding: u32, kind| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(kind),
            count: None,
        };
        let d2 = wgpu::TextureViewDimension::D2;
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                uniform_entry(0),
                texture_entry(1, wgpu::TextureSampleType::Depth, d2),
                sampler_entry(2, wgpu::SamplerBindingType::Comparison),
                texture_entry(3, wgpu::TextureSampleType::Float { filterable: true }, wgpu::TextureViewDimension::Cube),
                sampler_entry(4, wgpu::SamplerBindingType::Filtering),
            ],
        });
        let layouts = Layouts {
            uniforms: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("uniforms"),
                entries: &[uniform_entry(0)],
            }),
            body: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("body"),
                entries: &[uniform_entry(0)],
            }),
            frame: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("frame"),
                entries: &[texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }, d2)],
            }),
            ao_inputs: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("occlusion inputs"),
                entries: &[
                    texture_entry(0, wgpu::TextureSampleType::Depth, d2),
                    texture_entry(1, wgpu::TextureSampleType::Float { filterable: false }, d2),
                ],
            }),
            blit: device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blit"),
                entries: &[
                    texture_entry(0, wgpu::TextureSampleType::Float { filterable: true }, d2),
                    sampler_entry(1, wgpu::SamplerBindingType::Filtering),
                    uniform_entry(2),
                    texture_entry(3, wgpu::TextureSampleType::Float { filterable: true }, d2),
                ],
            }),
        };

        let pipeline_layout = |label: &str, groups: &[&wgpu::BindGroupLayout]| {
            let groups: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().map(|g| Some(*g)).collect();
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &groups,
                immediate_size: 0,
            })
        };
        let scene_layout = pipeline_layout("scene", &[&globals_layout, &layouts.body, &layouts.frame]);
        // The grid and the planes read only the globals: nothing else is bound with an
        // empty document.
        let grid_layout = pipeline_layout("grid", &[&globals_layout]);
        let uniforms_layout = pipeline_layout("uniforms", &[&layouts.uniforms]);
        let ao_layout = pipeline_layout("occlusion", &[&layouts.uniforms, &layouts.ao_inputs]);
        let blit_pipeline_layout = pipeline_layout("blit", &[&layouts.blit]);

        let face_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<FaceVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32, 3 => Uint32, 4 => Uint32],
        };
        let line_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32],
        };
        let point_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TaggedVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32],
        };

        struct Desc<'a> {
            label: &'a str,
            layout: &'a wgpu::PipelineLayout,
            vs: &'a str,
            fs: Option<&'a str>,
            buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
            format: Option<wgpu::TextureFormat>,
            blend: Option<wgpu::BlendState>,
            samples: u32,
            cull: Option<wgpu::Face>,
            depth: Option<wgpu::DepthStencilState>,
        }
        let pipeline = |d: Desc<'_>| {
            let targets = [d.format.map(|format| wgpu::ColorTargetState {
                format,
                blend: d.blend,
                write_mask: wgpu::ColorWrites::ALL,
            })];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(d.label),
                layout: Some(d.layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(d.vs),
                    compilation_options: Default::default(),
                    buffers: d.buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: d.cull,
                    ..Default::default()
                },
                depth_stencil: d.depth,
                multisample: wgpu::MultisampleState { count: d.samples, ..Default::default() },
                fragment: d.fs.map(|fs| wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: if d.format.is_some() { &targets } else { &[] },
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let depth_compare = |write: bool, compare| {
            Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(write),
                depth_compare: Some(compare),
                stencil: Default::default(),
                bias: Default::default(),
            })
        };
        let depth = |write| depth_compare(write, DEPTH_COMPARE);
        let premul = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let back = Some(wgpu::Face::Back);
        let faces = [Some(face_layout.clone())];
        let lines = [Some(line_layout)];
        let points = [Some(point_layout)];
        let face_pipeline = pipeline(Desc {
            label: "faces",
            layout: &scene_layout,
            vs: "vs_face",
            fs: Some("fs_face"),
            buffers: &faces,
            format: Some(COLOR_FORMAT),
            blend: premul,
            samples,
            cull: back,
            depth: depth(true),
        });
        let see_through_pipeline = pipeline(Desc {
            label: "see-through faces",
            layout: &scene_layout,
            vs: "vs_face",
            fs: Some("fs_face_see_through"),
            buffers: &faces,
            format: Some(COLOR_FORMAT),
            blend: premul,
            samples,
            cull: None,
            depth: depth(false),
        });
        let ground_pipeline = pipeline(Desc {
            label: "ground",
            layout: &scene_layout,
            vs: "vs_ground",
            fs: Some("fs_ground"),
            buffers: &[],
            format: Some(COLOR_FORMAT),
            blend: premul,
            samples,
            cull: None,
            depth: depth(false),
        });
        let grid_pipeline = pipeline(Desc {
            label: "grid",
            layout: &grid_layout,
            vs: "vs_grid",
            fs: Some("fs_grid"),
            buffers: &[],
            format: Some(COLOR_FORMAT),
            blend: premul,
            samples,
            cull: None,
            depth: depth(false),
        });
        let plane_pipeline = pipeline(Desc {
            label: "planes",
            layout: &grid_layout,
            vs: "vs_plane",
            fs: Some("fs_plane"),
            buffers: &[],
            format: Some(COLOR_FORMAT),
            blend: premul,
            samples,
            cull: None,
            depth: depth(false),
        });
        let xray_planes_pipeline = pipeline(Desc {
            label: "planes through solid",
            layout: &grid_layout,
            vs: "vs_plane",
            fs: Some("fs_plane"),
            buffers: &[],
            format: Some(COLOR_FORMAT),
            blend: premul,
            samples,
            cull: None,
            depth: depth_compare(false, wgpu::CompareFunction::Always),
        });
        let hidden_edges_pipeline = pipeline(Desc {
            label: "selected hidden edges",
            layout: &scene_layout,
            vs: "vs_edge",
            fs: Some("fs_hidden_edge"),
            buffers: &lines,
            format: Some(COLOR_FORMAT),
            blend: premul,
            samples,
            cull: None,
            depth: depth_compare(false, wgpu::CompareFunction::Less),
        });
        let markers_pipeline = pipeline(Desc {
            label: "vertex markers",
            layout: &scene_layout,
            vs: "vs_marker",
            fs: Some("fs_marker"),
            buffers: &points,
            format: Some(COLOR_FORMAT),
            blend: premul,
            samples,
            cull: None,
            depth: depth(false),
        });
        let edge_pipeline = pipeline(Desc {
            label: "edges",
            layout: &scene_layout,
            vs: "vs_edge",
            fs: Some("fs_edge"),
            buffers: &lines,
            format: Some(COLOR_FORMAT),
            blend: premul,
            samples,
            cull: None,
            depth: depth(true),
        });
        let shadow_pipeline = pipeline(Desc {
            label: "shadow",
            layout: &uniforms_layout,
            vs: "vs_shadow",
            fs: None,
            buffers: &faces,
            format: None,
            blend: None,
            samples: 1,
            cull: None,
            depth: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 },
            }),
        });
        let id_face_pipeline = pipeline(Desc {
            label: "id faces",
            layout: &scene_layout,
            vs: "vs_id_face",
            fs: Some("fs_id"),
            buffers: &faces,
            format: Some(ID_FORMAT),
            blend: None,
            samples: 1,
            cull: back,
            depth: depth(true),
        });
        let id_edge_pipeline = pipeline(Desc {
            label: "id edges",
            layout: &scene_layout,
            vs: "vs_id_edge",
            fs: Some("fs_id"),
            buffers: &lines,
            format: Some(ID_FORMAT),
            blend: None,
            samples: 1,
            cull: None,
            depth: depth(true),
        });
        let id_vertex_pipeline = pipeline(Desc {
            label: "id vertices",
            layout: &scene_layout,
            vs: "vs_id_vertex",
            fs: Some("fs_id"),
            buffers: &points,
            format: Some(ID_FORMAT),
            blend: None,
            samples: 1,
            cull: None,
            depth: depth(true),
        });
        let ao_pipelines = tier.has_ambient_occlusion().then(|| {
            let fullscreen = |label, fs, format| {
                pipeline(Desc {
                    label,
                    layout: &ao_layout,
                    vs: "vs_blit",
                    fs: Some(fs),
                    buffers: &[],
                    format: Some(format),
                    blend: None,
                    samples: 1,
                    cull: None,
                    depth: None,
                })
            };
            [
                pipeline(Desc {
                    label: "occlusion normals",
                    layout: &uniforms_layout,
                    vs: "vs_normals",
                    fs: Some("fs_normals"),
                    buffers: &faces,
                    format: Some(NORMAL_FORMAT),
                    blend: None,
                    samples: 1,
                    cull: back,
                    depth: depth(true),
                }),
                fullscreen("occlusion", "fs_ssao", AO_FORMAT),
                fullscreen("occlusion blur", "fs_ao_blur", AO_FORMAT),
            ]
        });

        let blit_fs = if target_format.is_srgb() { "fs_blit_linear" } else { "fs_blit_gamma" };
        let blit_pipeline = pipeline(Desc {
            label: "blit",
            layout: &blit_pipeline_layout,
            vs: "vs_blit",
            fs: Some(blit_fs),
            buffers: &[],
            format: Some(target_format),
            blend: None,
            samples: 1,
            cull: None,
            depth: None,
        });

        let global_buffer = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<Globals>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let globals = global_buffer("scene globals");
        let pick_globals = global_buffer("pick globals");
        let backdrop = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("backdrop"),
            size: std::mem::size_of::<Backdrop>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scene"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shadow_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("shadow map"),
                size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SHADOW_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let env = environment::build();
        let env_view = upload_environment(device, queue, &env);
        let env_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("environment"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let global_bind_group = |buffer: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("globals"),
                layout: &globals_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_view) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&env_view) },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&env_sampler) },
                ],
            })
        };
        let globals_bind_group = global_bind_group(&globals);
        let pick_globals_bind_group = global_bind_group(&pick_globals);
        let uniforms_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uniforms"),
            layout: &layouts.uniforms,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });
        let white = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("white"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: AO_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[255],
            )
            .create_view(&Default::default());
        let pick_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pick readback"),
            size: (PICK_ROW_TEXELS * ID_TEXEL * PICK_ROWS) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            tier,
            samples,
            face_pipeline,
            see_through_pipeline,
            ground_pipeline,
            grid_pipeline,
            plane_pipeline,
            xray_planes_pipeline,
            hidden_edges_pipeline,
            markers_pipeline,
            edge_pipeline,
            shadow_pipeline,
            id_face_pipeline,
            id_edge_pipeline,
            id_vertex_pipeline,
            ao_pipelines,
            blit_pipeline,
            globals,
            pick_globals,
            pick_globals_bind_group,
            backdrop,
            globals_bind_group,
            uniforms_bind_group,
            layouts,
            sampler,
            shadow_view,
            sun: None,
            shadow_key: None,
            white,
            transparent: picture_view(device, queue, [1, 1], &[0; 4]),
            backdrop_picture: None,
            sh: env.irradiance,
            targets: None,
            bodies: HashMap::new(),
            mesh_version: u64::MAX,
            ghost: None,
            context: None,
            preview: None,
            threads: None,
            pick_buffer,
            readback: Arc::new(Mutex::new(Readback::default())),
            picks: PickCache::default(),
            display: DisplayMode::default(),
            frame: 0,
            assembly: AssemblyRenderer::new(device, COLOR_FORMAT, DEPTH_FORMAT, samples, ID_FORMAT),
        }
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// True when assemblies are culled on the GPU on this device.
    pub fn assembly_culls(&self) -> bool {
        self.assembly.culls()
    }

    /// What the last frame drew of the assembly.
    pub fn assembly_stats(&self) -> super::assembly::AssemblyStats {
        self.assembly.stats
    }

    /// The assembly definitions and levels the GPU wanted to draw and did not have
    /// uploaded: pass them to `AssemblyDocument::request_levels`.
    pub fn take_assembly_wanted(
        &mut self,
    ) -> Vec<(soliddraft_assembly::DefinitionIndex, soliddraft_tessellation::Level)> {
        self.assembly.take_wanted()
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: [u32; 2], scale: u32) {
        let size = [size[0].max(1), size[1].max(1)];
        if self.targets.as_ref().map(|t| (t.size, t.scale)) == Some((size, scale)) {
            return;
        }
        let tex = |label: &str, format, samples, usage, scale| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size[0] * scale, height: size[1] * scale, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let sampled = ra | wgpu::TextureUsages::TEXTURE_BINDING;
        let color_msaa = (self.samples > 1)
            .then(|| tex("color msaa", COLOR_FORMAT, self.samples, ra, scale).create_view(&Default::default()));
        let color = tex("color", COLOR_FORMAT, 1, sampled, scale).create_view(&Default::default());
        let depth_msaa = tex("scene depth", DEPTH_FORMAT, self.samples, ra, scale).create_view(&Default::default());
        let id = tex("id", ID_FORMAT, 1, ra | wgpu::TextureUsages::COPY_SRC, 1);
        let id_view = id.create_view(&Default::default());
        let id_depth = tex("id depth", DEPTH_FORMAT, 1, ra, 1).create_view(&Default::default());
        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blit"),
            layout: &self.layouts.blit,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&color) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.backdrop.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(
                        self.backdrop_picture.as_ref().map_or(&self.transparent, |(_, view)| view),
                    ),
                },
            ],
        });
        let ao = self.ao_pipelines.is_some().then(|| {
            let depth = tex("occlusion depth", DEPTH_FORMAT, 1, sampled, scale).create_view(&Default::default());
            let normals = tex("occlusion normals", NORMAL_FORMAT, 1, sampled, scale).create_view(&Default::default());
            let raw = tex("occlusion raw", AO_FORMAT, 1, sampled, scale).create_view(&Default::default());
            let blurred = tex("occlusion", AO_FORMAT, 1, sampled, scale).create_view(&Default::default());
            let inputs = |label, second: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &self.layouts.ao_inputs,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&depth) },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(second) },
                    ],
                })
            };
            let sample_bind_group = inputs("occlusion inputs", &normals);
            let blur_bind_group = inputs("occlusion blur inputs", &raw);
            AoTargets { depth, normals, raw, sample_bind_group, blur_bind_group, blurred }
        });
        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &self.layouts.frame,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(ao.as_ref().map_or(&self.white, |a| &a.blurred)),
            }],
        });
        self.targets = Some(Targets {
            size,
            color_msaa,
            scale,
            color,
            depth_msaa,
            id,
            id_view,
            id_depth,
            blit_bind_group,
            ao,
            frame_bind_group,
        });
    }

    fn upload_body(&self, device: &wgpu::Device, mesh: &DisplayMesh, flags: u32) -> GpuBody {
        let table = mesh.appearances.len().min(MAX_APPEARANCES);
        if mesh.appearances.len() > MAX_APPEARANCES {
            log::warn!("a body shows {} appearances; the first {MAX_APPEARANCES} are drawn", mesh.appearances.len());
        }
        let verts: Vec<FaceVertex> = (0..mesh.positions.len())
            .map(|i| FaceVertex {
                position: mesh.positions[i].to_array(),
                normal: mesh.normals[i].to_array(),
                face_id: mesh.face_picks[i],
                feature_id: mesh.face_features.get(i).copied().unwrap_or(0),
                appearance: mesh.face_appearances.get(i).copied().filter(|a| (*a as usize) < table).unwrap_or(0),
            })
            .collect();
        let mut shades = [GpuShade::from(Appearance::default_part().shading()); MAX_APPEARANCES];
        for (slot, s) in shades.iter_mut().zip(&mesh.appearances) {
            *slot = (*s).into();
        }
        let see_through = verts.iter().any(|v| shades[v.appearance as usize].base[3] < OPAQUE);
        let mut lines: Vec<LineInstance> = Vec::new();
        for e in &mesh.edges {
            lines.extend(e.segments.as_chunks::<2>().0.iter().map(|s| LineInstance {
                a: s[0].to_array(),
                b: s[1].to_array(),
                id: e.pick,
            }));
        }
        let points: Vec<TaggedVertex> =
            mesh.corners.iter().map(|v| TaggedVertex { position: v.position.to_array(), id: v.pick }).collect();
        let non_empty = |v: &[u8]| if v.is_empty() { vec![0u8; 32] } else { v.to_vec() };
        let make = |label: &str, bytes: &[u8], usage| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: &non_empty(bytes),
                usage,
            })
        };
        let uniform = make(
            "body",
            bytemuck::bytes_of(&BodyUniform { flags, _pad: [0; 3], tint: [0.0; 4], shades }),
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("body"),
            layout: &self.layouts.body,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() }],
        });
        GpuBody {
            vertices: make("faces", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX),
            indices: make("indices", bytemuck::cast_slice(&mesh.indices), wgpu::BufferUsages::INDEX),
            index_count: mesh.indices.len() as u32,
            lines: make("edges", bytemuck::cast_slice(&lines), wgpu::BufferUsages::VERTEX),
            line_count: lines.len() as u32,
            points: make("points", bytemuck::cast_slice(&points), wgpu::BufferUsages::VERTEX),
            point_count: points.len() as u32,
            uniform,
            bind_group,
            bounds: mesh.bounds(),
            see_through,
        }
    }

    fn draws_faces(&self) -> bool {
        self.display != DisplayMode::Wireframe
    }

    fn scene_bodies<'a>(
        preview: &'a Option<(usize, u32, GpuBody)>,
        bodies: &'a HashMap<BodyId, GpuBody>,
    ) -> impl Iterator<Item = &'a GpuBody> {
        let preview = preview.as_ref().map(|(_, _, body)| body);
        preview.into_iter().chain(bodies.values().filter(move |_| preview.is_none()))
    }

    // A construction preview stands in for the built part and its thread marks.
    fn scene_threads(&self) -> Option<&GpuBody> {
        self.threads.as_ref().filter(|_| self.preview.is_none()).map(|(_, body)| body)
    }

    /// Draws a frame into the offscreen targets and reads back the pick window. The
    /// paint callback calls it; headless tests and benchmarks call it directly.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, input: &FrameInput) {
        self.frame += 1;
        log::debug!("prepare frame {} size {:?}", self.frame, input.size_px);
        self.display = input.display;
        let limit = device.limits().max_texture_dimension_2d / 2;
        let pixels = u64::from(input.size_px[0]) * u64::from(input.size_px[1]);
        let scale = if input.controls.supersample
            && input.size_px.iter().all(|&side| side <= limit)
            && pixels * u64::from(self.samples) <= 12_000_000
        {
            2
        } else {
            1
        };
        let same_picture = match (&self.backdrop_picture, &input.backdrop_picture) {
            (None, None) => true,
            (Some((cached, _)), Some(current)) => Arc::ptr_eq(cached, current),
            _ => false,
        };
        if !same_picture {
            self.backdrop_picture = input.backdrop_picture.as_ref().map(|picture| {
                let view = picture_view(device, queue, [picture.width(), picture.height()], picture.as_raw());
                (picture.clone(), view)
            });
            self.targets = None;
        }
        self.ensure_targets(device, input.size_px, scale);

        let (version, meshes) = &input.meshes;
        if *version != self.mesh_version
            || self.bodies.len() != meshes.len()
            || meshes.iter().any(|(id, _)| !self.bodies.contains_key(id))
        {
            self.bodies = meshes.iter().map(|(id, m)| (*id, self.upload_body(device, m, 0))).collect();
            self.mesh_version = *version;
        }
        match &input.ghost {
            Some((g, removes)) => {
                let ptr = Arc::as_ptr(g) as usize;
                let tint = if *removes { input.controls.red_removal } else { input.controls.tint_preview };
                let flags = 2 | if *removes { 8 } else { 0 } | if tint { 0 } else { 32 };
                if self.ghost.as_ref().map(|(p, flags, _)| (*p, *flags)) != Some((ptr, flags)) {
                    self.ghost = Some((ptr, flags, self.upload_body(device, g, flags)));
                }
            }
            None => self.ghost = None,
        }
        match &input.context {
            Some(c) => {
                let ptr = Arc::as_ptr(c) as usize;
                if self.context.as_ref().map(|(q, _)| *q) != Some(ptr) {
                    self.context = Some((ptr, self.upload_body(device, c, CONTEXT)));
                }
            }
            None => self.context = None,
        }
        match &input.preview {
            Some(p) => {
                let ptr = Arc::as_ptr(p) as usize;
                let flags = 4 | match input.controls.section {
                    // A section's cap keeps its fill: the preview tint is a feature preview's.
                    Some(hatch) => match hatch {
                        crate::graphics::SectionHatch::Solid => 0,
                        crate::graphics::SectionHatch::Diagonal => 64,
                        crate::graphics::SectionHatch::Crosshatch => 64 | 128,
                    },
                    None if input.controls.tint_preview => 0,
                    None => 32,
                };
                if self.preview.as_ref().map(|(q, flags, _)| (*q, *flags)) != Some((ptr, flags)) {
                    self.preview = Some((ptr, flags, self.upload_body(device, p, flags)));
                }
            }
            None => self.preview = None,
        }
        match &input.threads {
            Some(t) => {
                let ptr = Arc::as_ptr(t) as usize;
                if self.threads.as_ref().map(|(q, _)| *q) != Some(ptr) {
                    self.threads = Some((ptr, self.upload_body(device, t, THREAD_MARKS)));
                }
            }
            None => self.threads = None,
        }
        for (id, body) in &self.bodies {
            // Only the flags change per frame; the appearance table stays as uploaded.
            let flags: u32 = if input.selected_body == Some(*id) { 1 } else { 0 };
            queue.write_buffer(&body.uniform, 0, bytemuck::bytes_of(&flags));
        }

        let (look, ground_shadow) = input.controls.lighting.look(input.preset, self.tier, input.floor);
        let show_faces = self.draws_faces();
        let show_edges = self.display != DisplayMode::Shaded;
        let preview_key = self.preview.as_ref().map_or(0, |(p, _, _)| *p);
        // Construction previews replace the colour and revision-bound pick geometry.
        let shown: Vec<_> = Self::scene_bodies(&self.preview, &self.bodies).collect();
        let bounds = shown.iter().filter_map(|b| b.bounds).reduce(|(lo, hi), (l, h)| (lo.min(l), hi.max(h)));
        let shadows = look.shadows && show_faces && bounds.is_some();
        let follows_model = input.controls.lighting.shadow_follows_model;
        let shadow_key = (self.mesh_version, preview_key, follows_model);
        let draw_shadow_map = shadows && self.shadow_key != Some(shadow_key);
        if draw_shadow_map && let Some((lo, hi)) = bounds {
            self.sun = Some(sun_frame_with_ground(lo, hi, follows_model));
        }
        let ao = look.ambient_occlusion > 0.0 && show_faces && self.ao_pipelines.is_some();
        let ground = shadows && ground_shadow && self.sun.is_some_and(|s| input.eye.z > s.ground.z);

        let size = self.targets.as_ref().map(|t| t.size).unwrap_or([1, 1]);
        let view_proj = input.proj * input.view;
        let [right, up, back] = input.basis;
        let rig = |r: f32, u: f32, b: f32, intensity: f32| {
            let d = (right * r + up * u + back * b).normalize();
            [d.x, d.y, d.z, intensity * look.rig]
        };
        let sun = self.sun.unwrap_or(SunFrame {
            view_proj: Mat4::IDENTITY,
            ground: Vec3::ZERO,
            ground_radius: 1.0,
            radius: 1.0,
            texel: 1.0,
        });
        let toward_sun = environment::softbox_direction();
        queue.write_buffer(
            &self.backdrop,
            0,
            bytemuck::bytes_of(&Backdrop::from_srgb(
                input.backdrop,
                input.size_px,
                input.pixels_per_point,
                input.backdrop_picture.as_deref(),
            )),
        );
        let model_radius = bounds.map_or(100.0, |(lo, hi)| ((hi - lo).length() / 2.0).max(1.0));
        let grid = input.grid.unwrap_or(GridFrame {
            anchor: Vec3::ZERO,
            u: Vec3::X,
            v: Vec3::Y,
            minor: 1.0,
            major: 1.0,
            pull: 0.0,
            ink: egui::Color32::TRANSPARENT,
            opacity: (0.0, 0.0),
        });
        let plane_ink = |plane: usize| {
            input.planes.map_or([0.0; 4], |p| {
                let [r, g, b, _] = egui::Rgba::from(p.planes[plane].0).to_array();
                [r, g, b, p.planes[plane].1]
            })
        };
        let mut globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            view: input.view.to_cols_array_2d(),
            proj: input.proj.to_cols_array_2d(),
            inv_proj: input.proj.inverse().to_cols_array_2d(),
            sun_view_proj: sun.view_proj.to_cols_array_2d(),
            eye: [input.eye.x, input.eye.y, input.eye.z, if input.orthographic { 1.0 } else { 0.0 }],
            viewport: [size[0] as f32, size[1] as f32, EDGE_WIDTH * input.pixels_per_point, input.pixels_per_point],
            pull: [EDGE_PULL, EDGE_PULL * input.distance, model_radius * 0.06, model_radius * 0.002],
            key: rig(-0.45, 0.3, 0.85, 0.8),
            fill: rig(0.85, 0.1, 0.5, 0.3),
            rim: rig(0.2, 0.6, -0.75, 0.15),
            sun: [toward_sun.x, toward_sun.y, toward_sun.z, if shadows { look.sun } else { 0.0 }],
            ground: [sun.ground.x, sun.ground.y, sun.ground.z, sun.ground_radius],
            shadow: [look.shadow_strength, 0.007 * look.shadow_softness, if shadows { 1.0 } else { 0.0 }, sun.texel],
            look: [
                look.environment,
                look.reflections,
                if ao { look.ambient_occlusion } else { 0.0 },
                (environment::LEVELS - 1) as f32,
            ],
            grid_anchor: [grid.anchor.x, grid.anchor.y, grid.anchor.z, grid.pull],
            grid_u: [grid.u.x, grid.u.y, grid.u.z, grid.minor],
            grid_v: [grid.v.x, grid.v.y, grid.v.z, grid.major],
            grid_ink: egui::Rgba::from(grid.ink).to_array(),
            grid_lines: [
                grid.opacity.0,
                grid.opacity.1,
                grid::LINE_WIDTH.0 * input.pixels_per_point,
                grid::LINE_WIDTH.1 * input.pixels_per_point,
            ],
            plane_size: [
                input.planes.map_or(0.0, |p| p.half),
                planes::FILL,
                planes::OUTLINE,
                planes::OUTLINE_POINTS * input.pixels_per_point,
            ],
            planes: [plane_ink(0), plane_ink(1), plane_ink(2)],
            edge_color: Controls::color_uniform(input.controls.edge_color),
            selection_color: Controls::color_uniform(input.controls.selection_color),
            sh: self.sh,
            hover_id: input.hover_id,
            selected_id: input.selected_id,
            selected_feature: input.selected_feature,
            _pad: 0,
        };
        if scale > 1 {
            queue.write_buffer(&self.pick_globals, 0, bytemuck::bytes_of(&globals));
            for value in &mut globals.viewport {
                *value *= scale as f32;
            }
            globals.grid_lines[2] *= scale as f32;
            globals.grid_lines[3] *= scale as f32;
            globals.plane_size[3] *= scale as f32;
        }
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        // Pump map callbacks and publish a finished readback before issuing a new one.
        let _ = device.poll(wgpu::PollType::Poll);
        let (pending, ready, origin, serial, revision, seen) = {
            let rb = self.readback.lock().unwrap();
            (rb.pending, rb.ready, rb.origin, rb.serial, rb.revision, rb.seen)
        };
        if ready {
            {
                let view = self.pick_buffer.slice(..).get_mapped_range().expect("mapped pick buffer");
                let data: Vec<[u32; 2]> = bytemuck::cast_slice::<u8, [u32; 2]>(&view).to_vec();
                let mut out = input.pick_out.lock().unwrap();
                out.origin = origin;
                out.width = PICK_ROW_TEXELS;
                out.height = PICK_ROWS;
                out.data = data;
                out.serial = serial;
                out.revision = revision;
                out.seen = seen;
            }
            self.pick_buffer.unmap();
            let mut rb = self.readback.lock().unwrap();
            rb.ready = false;
            rb.pending = false;
            self.picks.published();
        } else if !pending {
            // A readback whose map failed published nothing, so its pick is read again.
            self.picks.failed();
        }
        let pick_bodies = || {
            let preview = self.preview.as_ref().filter(|_| input.preview_pickable).map(|(_, _, body)| body);
            preview.into_iter().chain(self.bodies.values().filter(move |_| preview.is_none()))
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("viewport") });
        self.assembly.prepare(device, queue, &mut encoder, input);
        let targets = self.targets.as_ref().expect("targets");
        let draw_faces = |pass: &mut wgpu::RenderPass<'_>, bodies: &mut dyn Iterator<Item = &GpuBody>, groups: bool| {
            for body in bodies {
                if groups {
                    pass.set_bind_group(1, &body.bind_group, &[]);
                }
                pass.set_vertex_buffer(0, body.vertices.slice(..));
                pass.set_index_buffer(body.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..body.index_count, 0, 0..1);
            }
        };
        let draw_lines = |pass: &mut wgpu::RenderPass<'_>, bodies: &mut dyn Iterator<Item = &GpuBody>| {
            for body in bodies {
                if body.line_count > 0 {
                    pass.set_bind_group(1, &body.bind_group, &[]);
                    pass.set_vertex_buffer(0, body.lines.slice(..));
                    pass.draw(0..6, 0..body.line_count);
                }
            }
        };
        let depth_attachment = |view, clear| {
            Some(wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            })
        };
        let color_attachment = |view, store| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store },
            })
        };

        if draw_shadow_map {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow"),
                color_attachments: &[],
                depth_stencil_attachment: depth_attachment(&self.shadow_view, 1.0),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.uniforms_bind_group, &[]);
            draw_faces(&mut pass, &mut shown.iter().copied(), false);
            drop(pass);
            self.shadow_key = Some(shadow_key);
        }

        if ao && let (Some([normals, sample, blur]), Some(t)) = (&self.ao_pipelines, &targets.ao) {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("occlusion normals"),
                    color_attachments: &[color_attachment(&t.normals, wgpu::StoreOp::Store)],
                    depth_stencil_attachment: depth_attachment(&t.depth, DEPTH_CLEAR),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(normals);
                pass.set_bind_group(0, &self.uniforms_bind_group, &[]);
                draw_faces(&mut pass, &mut shown.iter().copied(), false);
            }
            for (label, pipeline, inputs, out) in [
                ("occlusion", sample, &t.sample_bind_group, &t.raw),
                ("occlusion blur", blur, &t.blur_bind_group, &t.blurred),
            ] {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(label),
                    color_attachments: &[color_attachment(out, wgpu::StoreOp::Store)],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.uniforms_bind_group, &[]);
                pass.set_bind_group(1, inputs, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: targets.color_msaa.as_ref().unwrap_or(&targets.color),
                    depth_slice: None,
                    resolve_target: targets.color_msaa.as_ref().map(|_| &targets.color),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: if targets.color_msaa.is_some() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth_msaa,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(DEPTH_CLEAR),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            pass.set_bind_group(2, &targets.frame_bind_group, &[]);
            let ghost = self.ghost.as_ref().map(|(_, _, b)| b);
            let context = self.context.as_ref().map(|(_, b)| b);
            if show_faces {
                pass.set_pipeline(&self.face_pipeline);
                draw_faces(&mut pass, &mut shown.iter().copied(), true);
                self.assembly.draw(&mut pass);
                // The assembly binds its own group 0.
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
            }
            if ground {
                // The ground darkens behind the opaque faces and in front of the background.
                pass.set_pipeline(&self.ground_pipeline);
                pass.draw(0..6, 0..1);
            }
            if input.planes.is_some() {
                // Only where no face drew, and over the ground's shadow.
                pass.set_pipeline(if input.controls.xray_references {
                    &self.xray_planes_pipeline
                } else {
                    &self.plane_pipeline
                });
                pass.draw(0..18, 0..1);
            }
            if input.grid.is_some() {
                // Tested against the faces and written to no buffer, so it can hide nothing.
                pass.set_pipeline(&self.grid_pipeline);
                pass.draw(0..3, 0..1);
            }
            if show_faces && shown.iter().any(|b| b.see_through) {
                // After everything opaque, so the opaque depth hides what is behind.
                pass.set_pipeline(&self.see_through_pipeline);
                draw_faces(&mut pass, &mut shown.iter().copied().filter(|b| b.see_through), true);
            }
            if show_faces && let Some(c) = context {
                // Over the part's opaque faces, so the part shows through it.
                pass.set_pipeline(&self.face_pipeline);
                draw_faces(&mut pass, &mut std::iter::once(c), true);
            }
            if show_faces && let Some(g) = ghost {
                pass.set_pipeline(&self.face_pipeline);
                draw_faces(&mut pass, &mut std::iter::once(g), true);
            }
            if let Some(c) = context {
                pass.set_pipeline(&self.edge_pipeline);
                draw_lines(&mut pass, &mut std::iter::once(c));
            }
            if show_faces
                && input.controls.hidden_selected_edges
                && self.preview.is_none()
                && let Some(body) = input.selected_body.and_then(|id| self.bodies.get(&id))
            {
                pass.set_pipeline(&self.hidden_edges_pipeline);
                draw_lines(&mut pass, &mut std::iter::once(body));
            }
            if show_edges {
                self.assembly.draw_edges(&mut pass);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
            }
            if show_edges || ghost.is_some() {
                pass.set_pipeline(&self.edge_pipeline);
                if show_edges {
                    draw_lines(&mut pass, &mut shown.iter().copied().chain(ghost));
                } else {
                    draw_lines(&mut pass, &mut ghost.into_iter());
                }
            }
            if input.controls.vertex_markers && self.preview.is_none() {
                pass.set_pipeline(&self.markers_pipeline);
                for body in &shown {
                    if body.point_count > 0 {
                        pass.set_bind_group(1, &body.bind_group, &[]);
                        pass.set_vertex_buffer(0, body.points.slice(..));
                        pass.draw(0..6, 0..body.point_count);
                    }
                }
            }
            if let Some(threads) = self.scene_threads() {
                pass.set_pipeline(&self.edge_pipeline);
                draw_lines(&mut pass, &mut std::iter::once(threads));
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ids"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.id_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.id_depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(DEPTH_CLEAR),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(2, &targets.frame_bind_group, &[]);
            pass.set_bind_group(
                0,
                if scale > 1 { &self.pick_globals_bind_group } else { &self.globals_bind_group },
                &[],
            );
            pass.set_pipeline(&self.id_face_pipeline);
            draw_faces(&mut pass, &mut pick_bodies(), true);
            if !input.preview_pickable || self.preview.is_none() {
                pass.set_pipeline(&self.id_edge_pipeline);
                draw_lines(&mut pass, &mut self.bodies.values());
                pass.set_pipeline(&self.id_vertex_pipeline);
                for body in self.bodies.values() {
                    if body.point_count > 0 {
                        pass.set_bind_group(1, &body.bind_group, &[]);
                        pass.set_vertex_buffer(0, body.points.slice(..));
                        pass.draw(0..6, 0..body.point_count);
                    }
                }
            }
            self.assembly.draw_ids(&mut pass);
        }

        let mut issued = None;
        let inputs = input.cursor_px.map(|cursor| PickInputs::of(input, cursor));
        // Only WebGL waits for the readback: there a pick is read again when one of its
        // inputs changes. An assembly's scene has no version here, so its pick is read
        // every frame.
        let changed = !cfg!(target_arch = "wasm32")
            || input.assembly.is_some()
            || inputs.as_ref().is_none_or(|inputs| self.picks.wants(inputs));
        if let (Some(cursor), false, true) = (input.cursor_px, pending || ready, changed) {
            if let Some(inputs) = inputs {
                self.picks.issued(inputs);
            }
            let w = PICK_ROW_TEXELS.min(size[0]);
            let h = PICK_ROWS.min(size[1]);
            let x = cursor[0].saturating_sub(w / 2).min(size[0] - w);
            let y = cursor[1].saturating_sub(h / 2).min(size[1] - h);
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &targets.id,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &self.pick_buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(PICK_ROW_TEXELS * ID_TEXEL),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            issued = Some([x, y]);
        }
        queue.submit([encoder.finish()]);
        self.assembly.after_submit();
        if let Some(origin) = issued {
            {
                let mut rb = self.readback.lock().unwrap();
                rb.pending = true;
                rb.ready = false;
                rb.origin = origin;
                rb.serial = self.frame;
                rb.revision = input.pick_revision;
                rb.seen = input.pick_view();
            }
            let state = self.readback.clone();
            self.pick_buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                let mut rb = state.lock().unwrap();
                if result.is_ok() {
                    rb.ready = true;
                } else {
                    rb.pending = false;
                }
            });
        }
    }

    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>) {
        log::debug!("paint");
        let Some(targets) = &self.targets else { return };
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, &targets.blit_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Uploads the baked environment as a cube map with one mip level per roughness step.
fn upload_environment(device: &wgpu::Device, queue: &wgpu::Queue, env: &environment::Environment) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("environment"),
        size: wgpu::Extent3d { width: environment::SIZE, height: environment::SIZE, depth_or_array_layers: 6 },
        mip_level_count: environment::LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, texels) in env.levels.iter().enumerate() {
        let size = environment::SIZE >> level;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(texels),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size * 8), rows_per_image: Some(size) },
            wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 6 },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("environment"),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    fn project(m: Mat4, p: Vec3) -> Vec3 {
        let c = m * Vec4::new(p.x, p.y, p.z, 1.0);
        c.truncate() / c.w
    }

    /// A pick whose readback fails is read again with unchanged inputs; one published
    /// is not, until an input changes.
    #[test]
    fn a_failed_pick_readback_is_retried_and_a_published_one_is_kept() {
        let mut picks = PickCache::default();
        assert!(picks.wants(&1));
        picks.issued(1);
        picks.failed();
        assert!(picks.wants(&1), "a failed readback is read again");
        picks.issued(1);
        picks.published();
        assert!(!picks.wants(&1), "a published pick with unchanged inputs is kept");
        assert!(picks.wants(&2));
    }

    #[test]
    fn the_shadow_map_covers_the_model_and_the_ground_disc() {
        let (lo, hi) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(120.0, 90.0, 60.0));
        let sun = sun_frame(lo, hi);
        let center = (lo + hi) / 2.0;
        let c = project(sun.view_proj, center);
        assert!(c.x.abs() < 1e-4 && c.y.abs() < 1e-4);
        for corner in [lo, hi, Vec3::new(lo.x, hi.y, lo.z), Vec3::new(hi.x, lo.y, hi.z)] {
            let p = project(sun.view_proj, corner);
            assert!(p.x.abs() <= 1.0 && p.y.abs() <= 1.0 && (0.0..=1.0).contains(&p.z), "{corner} -> {p}");
        }
        // Every point of the ground disc lies within the map's depth range.
        for k in 0..16 {
            let a = k as f32 / 16.0 * std::f32::consts::TAU;
            let g = sun.ground + Vec3::new(a.cos(), a.sin(), 0.0) * sun.ground_radius;
            let z = project(sun.view_proj, g).z;
            assert!((0.0..=1.0).contains(&z), "ground {g} depth {z}");
        }
        assert!(sun.ground.z < lo.z);
        // The light is above the model: the top of the model is nearer to it than the bottom.
        assert!(
            project(sun.view_proj, Vec3::new(60.0, 45.0, 60.0)).z
                < project(sun.view_proj, Vec3::new(60.0, 45.0, 0.0)).z
        );
    }

    /// What wgpu's WebGL2 backend does with each entry point the basic tier creates a
    /// pipeline for: translate it to GLSL ES 3.00. A shader feature WebGL2 lacks fails
    /// here instead of in a browser.
    #[test]
    fn the_basic_tier_translates_to_webgl2() {
        use naga::back::glsl;
        let module = naga::front::wgsl::parse_str(include_str!("scene.wgsl")).expect("scene.wgsl parses");
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .expect("scene.wgsl validates");
        let options =
            glsl::Options { version: glsl::Version::Embedded { version: 300, is_webgl: true }, ..Default::default() };
        let vertex = [
            "vs_face",
            "vs_ground",
            "vs_grid",
            "vs_plane",
            "vs_edge",
            "vs_shadow",
            "vs_id_face",
            "vs_id_edge",
            "vs_id_vertex",
            "vs_blit",
        ];
        let fragment =
            ["fs_face", "fs_ground", "fs_grid", "fs_plane", "fs_edge", "fs_id", "fs_blit_linear", "fs_blit_gamma"];
        let stages = vertex
            .iter()
            .map(|e| (naga::ShaderStage::Vertex, e))
            .chain(fragment.iter().map(|e| (naga::ShaderStage::Fragment, e)));
        for (stage, entry) in stages {
            let pipeline =
                glsl::PipelineOptions { shader_stage: stage, entry_point: entry.to_string(), multiview: None };
            let mut out = String::new();
            glsl::Writer::new(&mut out, &module, &info, &options, &pipeline, Default::default())
                .and_then(|mut w| w.write())
                .unwrap_or_else(|e| panic!("{entry} does not translate to GLSL ES 3.00: {e}"));
            assert!(out.starts_with("#version 300 es"), "{entry}");
        }
    }

    #[test]
    fn globals_match_the_shader_layout() {
        // Five matrices, twenty-one vectors, nine harmonics and four ids.
        assert_eq!(std::mem::size_of::<Globals>(), 5 * 64 + 21 * 16 + 9 * 16 + 16);
        assert_eq!(std::mem::size_of::<Backdrop>(), 8 * 16);
        assert_eq!(std::mem::size_of::<LineInstance>(), 28);
        assert_eq!(std::mem::size_of::<BodyUniform>(), 32 + 48 * MAX_APPEARANCES);
    }
}
