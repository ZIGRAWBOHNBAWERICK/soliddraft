//! Workspace lifetime, recovery and the frame loop.

use crate::document::{Document, Selection};
use crate::file;
use crate::panels::{Layout, Side};
use crate::state::{AppAction, AppState, NoteKind};
use crate::theme;
use crate::viewport::look::{Preset, Tier};
use crate::viewport::render::{DisplayMode, ViewportRenderer};
use egui_dock::DockState;
use std::path::PathBuf;
use std::time::Duration;
use web_time::Instant;

use super::dispatch::selection_of;
use super::presentation::docks::{Panels, Tab, center_dock, show_dock};

/// Options for automated runs (benchmarks and screenshots).
#[derive(Clone, Debug, Default)]
pub struct Automation {
    pub screenshot: Option<std::path::PathBuf>,
    pub screenshot_after_frames: u64,
    pub exit_after_screenshot: bool,
    pub bench: bool,
    pub bench_seconds: f32,
    /// Connect the assistant with the key from the environment or the keychain.
    pub connect_assistant: bool,
    /// Serve MCP on this loopback port (0 picks one).
    pub mcp_port: Option<u16>,
    /// Start in this render look instead of the default from the settings.
    pub preset: Option<Preset>,
    /// Start light or dark instead of the appearance from the settings.
    pub theme: Option<theme::Mode>,
}

struct Bench {
    reported: bool,
    started: Instant,
    warmup: Duration,
    frames: Vec<f32>,
    last: Instant,
}

pub struct SolidDraftApp {
    pub state: AppState,
    /// The middle dock: the part and the drawing.
    pub(super) dock: DockState<Tab>,
    /// Additional open part tabs parked beside the active part.
    pub open_parts: crate::top_bar::OpenParts,
    /// The panel layout as last written to disk.
    saved_layout: Layout,
    /// The command line's chips and session height as last written to disk.
    saved_bar: crate::command_line::BarSettings,
    saved_commands: crate::commands::History,
    saved_create_pins: crate::create_menu::Pins,
    automation: Automation,
    frame: u64,
    bench: Option<Bench>,
    screenshot_requested: bool,
    title: String,
    /// The GPU the window draws with, for part thumbnails. Desktop only.
    render_state: Option<egui_wgpu::RenderState>,
    /// The renderer part thumbnails are drawn with, made at the first save.
    offscreen: Option<crate::viewport::offscreen::Offscreen>,
    /// The open Preferences window.
    pub preferences: Option<crate::preferences::Preferences>,
    /// The recent-list key of a part the browser just saved, waiting for the screenshot
    /// its picture is cut from.
    #[cfg(target_arch = "wasm32")]
    pub(super) web_thumbnail: Option<String>,
    /// A part the browser's kernel worker is building before the page opens it.
    #[cfg(target_arch = "wasm32")]
    opening: Option<Opening>,
    /// Actions waiting while the page reads the exact geometry they need.
    #[cfg(target_arch = "wasm32")]
    pub(super) reading: Option<Reading>,
}

/// Actions held until the part's bodies adopted from the kernel worker are read. An edit
/// adopts them without their exact geometry; a command that needs it waits here, behind
/// a busy state drawn before the first body is read.
#[cfg(target_arch = "wasm32")]
pub(super) struct Reading {
    pub held: Vec<AppAction>,
    /// A frame has drawn the busy state.
    pub drawn: bool,
}

/// A part file waiting for the kernel worker's builds. When they have been offered to
/// the page's kernel, the page opens the file without evaluating it again.
#[cfg(target_arch = "wasm32")]
struct Opening {
    name: String,
    bytes: Vec<u8>,
    answer: std::sync::mpsc::Receiver<Result<(), String>>,
    /// The worker has answered; its builds are being read.
    answered: bool,
    /// Cleared when the open is dropped, so a cancelled open's builds are not offered.
    live: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(target_arch = "wasm32")]
impl Drop for Opening {
    fn drop(&mut self) {
        self.live.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Marks the screenshot a browser save asks for.
#[cfg(target_arch = "wasm32")]
struct WebThumbnail;

impl SolidDraftApp {
    fn focus_tool_document(&mut self) {
        match self.state.tool_document.take() {
            Some(Tab::Drawing) => crate::drawing::show_drawing_tab(&mut self.dock),
            Some(Tab::Assembly) => crate::assembly::show_tab(&mut self.dock),
            Some(Tab::Viewport) => crate::drawing::show_part_tab(&mut self.dock),
            _ => {}
        }
    }

    pub fn new(cc: &eframe::CreationContext<'_>, doc: Document, automation: Automation) -> Self {
        crate::settings::capture_install_kind();
        theme::install(&cc.egui_ctx, automation.theme.unwrap_or_else(|| crate::settings::appearance().theme));
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            let tier = Tier::of(rs.adapter.get_info().backend);
            let renderer = ViewportRenderer::new(&rs.device, &rs.queue, rs.target_format, tier);
            rs.renderer.write().callback_resources.insert(renderer);
        }
        log::debug!("renderer installed");
        #[cfg(target_arch = "wasm32")]
        crate::web::set_context(&cc.egui_ctx);
        let mut state = AppState::new(doc);
        state.viewport.camera.fit_framed(state.doc.bounds());
        let view = crate::settings::view();
        let preset = automation.preset.unwrap_or(view.preset);
        state.viewport.preset = preset;
        state.viewport.references = view.references;
        state.viewport.grid_style = view.grid;
        state.viewport.scene = Some(view.scene);
        if !preset.shows_edges() {
            state.viewport.display = DisplayMode::Shaded;
        }
        let ctx = cc.egui_ctx.clone();
        let waker: std::sync::Arc<dyn Fn() + Send + Sync> = std::sync::Arc::new(move || ctx.request_repaint());
        if automation.connect_assistant {
            state.chat = crate::chat::ChatState::from_environment();
        }
        state.chat.set_waker(waker.clone());
        if let Some(port) = automation.mcp_port {
            match state.start_mcp(port, Some(waker)) {
                Ok(url) => {
                    log::info!("MCP server at {url}");
                    eprintln!("SolidDraft MCP server: {url}");
                    state.chat.status.push_str(&format!(" · MCP {url}"));
                }
                Err(e) => state.note(e, NoteKind::Error),
            }
        }
        let bench = automation.bench.then(|| Bench {
            reported: false,
            started: Instant::now(),
            warmup: Duration::from_secs(1),
            frames: Vec::new(),
            last: Instant::now(),
        });
        Self {
            saved_layout: state.layout,
            saved_bar: state.command_line.settings.clone(),
            saved_commands: state.commands.clone(),
            saved_create_pins: state.create_pins.clone(),
            state,
            dock: center_dock(),
            open_parts: crate::top_bar::OpenParts::default(),
            automation,
            frame: 0,
            bench,
            screenshot_requested: false,
            title: String::new(),
            #[cfg(not(target_arch = "wasm32"))]
            render_state: cc.wgpu_render_state.clone(),
            #[cfg(target_arch = "wasm32")]
            render_state: None,
            offscreen: None,
            preferences: None,
            #[cfg(target_arch = "wasm32")]
            web_thumbnail: None,
            #[cfg(target_arch = "wasm32")]
            opening: None,
            #[cfg(target_arch = "wasm32")]
            reading: None,
        }
    }

    /// Starts autosave with its session folder under `root`, and opens the Recover
    /// documents window when a session that ended without quitting left copies. The
    /// desktop binary calls it; tests and automated runs leave autosave off.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_recovery(&mut self, root: &std::path::Path, policy: soliddraft_document::AutosavePolicy) {
        match crate::recovery::Recovery::start(root, policy) {
            Ok(recovery) => self.state.recovery = Some(recovery),
            Err(e) => self.state.note(format!("Autosave is off: {e}"), NoteKind::Error),
        }
    }

    /// Starts autosave into the browser's storage for this site, and opens the Recover
    /// documents window once the browser says an earlier visit left unsaved work.
    #[cfg(target_arch = "wasm32")]
    pub fn start_recovery_in_browser(&mut self, policy: soliddraft_document::AutosavePolicy) {
        self.state.recovery = Some(crate::recovery::Recovery::start_in_browser(policy));
    }

    /// Shows the start page instead of the empty part, as the app does at launch when no
    /// document was asked for.
    pub fn show_start_page(&mut self, ctx: &egui::Context) {
        let ctx = ctx.clone();
        self.state.start = Some(crate::start_page::StartPage::new(&self.state, move || ctx.request_repaint()));
    }

    /// Opens a document file from its bytes: an upload in the browser, or a document the
    /// browser keeps for the recent list.
    /// Opens the import review on a STEP file the browser uploaded for Import STEP: the
    /// same review a file picked on the desktop gets, from the same bytes.
    fn review_step_upload(&mut self, name: &str, bytes: Vec<u8>) {
        match crate::import::Preview::from_bytes(&self.state.doc, name, bytes) {
            Ok(preview) => self.state.import_dialog = Some(preview),
            Err(message) => crate::import::report::refused(
                &mut self.state,
                std::path::Path::new(name),
                soliddraft_kernel_api::ImportFormat::Step,
                message,
            ),
        }
    }

    /// Opens uploaded files: a STEP file Import STEP asked for goes to its review. An
    /// assembly opens with the other files as the documents it places, since the page has
    /// no folder to find them in, and the visitor chooses which when there are several;
    /// otherwise the first file opens.
    fn open_uploads(&mut self, mut files: Vec<(String, Vec<u8>)>) {
        use soliddraft_document::DocKind;
        let step_asked = std::mem::take(&mut self.state.step_upload);
        if files.is_empty() {
            return;
        }
        // A new upload replaces an Open assembly choice still waiting for an answer, so
        // the old choice cannot open over what this one opens.
        self.state.assembly.choice = None;
        let is_step = |name: &str| {
            std::path::Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("step") || e.eq_ignore_ascii_case("stp"))
        };
        if step_asked && is_step(&files[0].0) {
            let (name, bytes) = files.swap_remove(0);
            return self.review_step_upload(&name, bytes);
        }
        if files.iter().any(|(name, _)| DocKind::of_path(std::path::Path::new(name)) == Some(DocKind::Assembly)) {
            return crate::assembly::open_uploaded(&mut self.state, files);
        }
        let (name, bytes) = files.remove(0);
        if !files.is_empty() {
            self.state.note(format!("Opened {name}; open one part or drawing at a time"), NoteKind::Info);
        }
        self.open_bytes(&name, &bytes);
    }

    pub(super) fn open_bytes(&mut self, name: &str, bytes: &[u8]) {
        use soliddraft_document::DocKind;
        self.state.assembly.choice = None;
        match DocKind::of_path(std::path::Path::new(name)) {
            Some(DocKind::Drawing) => match crate::drawing::document::DrawingDoc::from_bytes(bytes) {
                Ok(mut doc) => {
                    doc.path = Some(PathBuf::from(name));
                    crate::drawing::restore(&mut self.state, &mut self.dock, doc);
                    self.state.note(format!("Opened {name}"), NoteKind::Ok);
                    #[cfg(target_arch = "wasm32")]
                    crate::documents_browser::remember(name, DocKind::Drawing, bytes);
                }
                Err(e) => self.state.note(format!("{name}: {e}"), NoteKind::Error),
            },
            Some(DocKind::Assembly) => crate::assembly::open_bytes(&mut self.state, name, bytes.to_vec()),
            #[cfg(target_arch = "wasm32")]
            _ if self.opening.is_none() => self.open_in_worker(name, bytes),
            _ => {
                let opened =
                    file::open_bytes(name, bytes, self.state.doc.kernel().clone(), Some(self.state.doc.model()));
                #[cfg(target_arch = "wasm32")]
                if opened.is_ok() {
                    crate::documents_browser::remember(name, DocKind::Part, bytes);
                }
                self.opened(opened);
            }
        }
    }

    /// Sends a part file to the browser's kernel worker to build. The page opens it once
    /// the worker's builds have been offered to the page's kernel; see [`Self::poll_opening`].
    #[cfg(target_arch = "wasm32")]
    fn open_in_worker(&mut self, name: &str, bytes: &[u8]) {
        let (sender, answer) = std::sync::mpsc::channel();
        let job = crate::kernel_worker::Job::Build { edit: None, level: Default::default() };
        let kernel = self.state.doc.kernel().clone();
        let ctx = crate::web::context().unwrap_or_default();
        let live = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let wanted = live.clone();
        let still = move || wanted.load(std::sync::atomic::Ordering::Acquire);
        match crate::kernel_worker::submit(bytes.to_vec(), &job, kernel, &ctx, still, move |result| {
            let _ = sender.send(result.map(|_| ()));
        }) {
            Ok(_) => {
                self.opening =
                    Some(Opening { name: name.to_owned(), bytes: bytes.to_vec(), answer, answered: false, live })
            }
            // Without a worker the page opens the file itself.
            Err(_) => self.open_here(name, bytes),
        }
    }

    /// Opens the part the kernel worker built, on the builds it handed over. A worker that
    /// failed leaves the page to open the file itself.
    #[cfg(target_arch = "wasm32")]
    fn poll_opening(&mut self) {
        let Some(opening) = &mut self.opening else { return };
        // An answer, or a worker that stopped without one: either way the page opens the
        // file next, taking whatever builds were offered.
        if !opening.answered {
            if let Err(std::sync::mpsc::TryRecvError::Empty) = opening.answer.try_recv() {
                return;
            }
            opening.answered = true;
        }
        // The offered builds are read one a frame, so no frame holds the page for all of them.
        if self.state.doc.kernel().read_offers(1).unwrap_or(0) > 0 {
            if let Some(ctx) = crate::web::context() {
                ctx.request_repaint();
            }
            return;
        }
        let opening = self.opening.take().expect("a part is opening");
        self.open_here(&opening.name, &opening.bytes);
    }

    /// Opens a part file on the page's kernel, taking any builds offered to it.
    #[cfg(target_arch = "wasm32")]
    fn open_here(&mut self, name: &str, bytes: &[u8]) {
        let opened = file::open_bytes(name, bytes, self.state.doc.kernel().clone(), Some(self.state.doc.model()));
        if opened.is_ok() {
            crate::documents_browser::remember(name, soliddraft_document::DocKind::Part, bytes);
        }
        self.opened_from(opened, Some(bytes));
    }

    /// A PNG of the part as the viewport would first show it, for its file. `None` when
    /// there is no body or no GPU, or the picture failed; the save goes on without it.
    pub(super) fn thumbnail(&mut self) -> Option<Vec<u8>> {
        let rs = self.render_state.as_ref()?;
        let doc = &self.state.doc;
        if doc.display_body().is_none() || doc.meshes.is_empty() {
            return None;
        }
        let offscreen = self.offscreen.get_or_insert_with(|| {
            let tier = Tier::of(rs.adapter.get_info().backend);
            crate::viewport::offscreen::Offscreen::new(&rs.device, &rs.queue, tier)
        });
        let picture = offscreen.capture(&rs.device, &rs.queue, doc, crate::viewport::offscreen::THUMBNAIL);
        match picture.and_then(|p| p.png()) {
            Ok(png) => Some(png),
            Err(e) => {
                log::warn!("no thumbnail for the part: {e}");
                None
            }
        }
    }

    /// Closes Recover documents once nothing is left in it to decide.
    pub(super) fn close_recovery_when_done(&mut self) {
        if let Some(recovery) = self.state.recovery.as_mut().filter(|r| !r.has_offers() && r.problems.is_empty()) {
            recovery.show = false;
        }
    }

    /// Opens the document in a recovery copy in place of the open one of its kind.
    pub(super) fn restore(&mut self, file: &std::path::Path) {
        use soliddraft_document::DocKind;
        enum Opened {
            Part(Box<Document>),
            Drawing(Box<crate::drawing::document::DrawingDoc>),
        }
        let Some(recovery) = self.state.recovery.as_mut() else { return };
        let (record, opened) = match recovery.restore(file, |record, bytes| match record.kind {
            DocKind::Drawing => {
                crate::drawing::document::DrawingDoc::from_bytes(bytes).map(|doc| Opened::Drawing(Box::new(doc)))
            }
            _ => file::open_bytes(&record.name, bytes, self.state.doc.kernel().clone(), Some(self.state.doc.model()))
                .map(|doc| Opened::Part(Box::new(doc))),
        }) {
            Ok(opened) => opened,
            Err(e) => return self.state.note(e, NoteKind::Error),
        };
        let (kind, edits) = match opened {
            Opened::Drawing(mut doc) => {
                doc.path = record.path.clone();
                doc.touch();
                let edits = doc.edits;
                crate::drawing::restore(&mut self.state, &mut self.dock, *doc);
                (DocKind::Drawing, edits)
            }
            Opened::Part(mut doc) => {
                doc.name = record.name.clone();
                doc.path = record.path.clone();
                doc.touch();
                let edits = doc.edits;
                self.opened(Ok(*doc));
                self.state.note(format!("Restored {} · not saved yet", record.name), NoteKind::Ok);
                (DocKind::Part, edits)
            }
        };
        if let Some(r) = self.state.recovery.as_mut() {
            r.restored(record.id, kind, edits);
        }
    }

    /// Shows a part that was just opened, or why it could not be.
    pub(super) fn opened(&mut self, opened: Result<Document, String>) {
        self.opened_from(opened, None);
    }

    /// Shows a part that was just opened from `saved`, when the caller still has the
    /// bytes, so its revision is remembered without saving the part again.
    pub(super) fn opened_from(&mut self, opened: Result<Document, String>, saved: Option<&[u8]>) {
        match opened {
            Ok(doc) => {
                self.close_editor();
                self.state.sketch = None;
                self.state.chat.preview = None;
                let n = doc.features.len();
                self.state.scan = Default::default();
                if std::mem::take(&mut self.state.open_in_new_tab) {
                    self.open_parts.park_active(&mut self.state, doc);
                } else {
                    self.state.doc = doc;
                }
                match saved {
                    Some(saved) => self.state.remember_opened_revision(saved),
                    None => self.state.remember_revision(),
                }
                self.state.viewport.camera.fit_framed(crate::scan::document_bounds(&self.state.doc));
                self.state.recent = file::recent();
                let problems = self.state.doc.error_count() + self.state.doc.repair_count();
                if std::mem::take(&mut self.state.drawing_after_open) {
                    self.state.push(AppAction::Drawing(crate::drawing::DrawingAction::MakeDrawing));
                }
                if problems == 0 {
                    self.state.note(format!("Opened {} · {n} features rebuilt", self.state.doc.name), NoteKind::Ok);
                } else {
                    self.state.note(
                        format!(
                            "Opened {} · {problems} feature{} need attention",
                            self.state.doc.name,
                            if problems == 1 { "" } else { "s" }
                        ),
                        NoteKind::Error,
                    );
                }
            }
            Err(e) => {
                self.state.drawing_after_open = false;
                self.state.open_in_new_tab = false;
                self.state.note(e, NoteKind::Error);
            }
        }
    }

    pub(super) fn automation_step(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.frame += 1;
        if let Some(bench) = self.bench.as_mut() {
            let now = Instant::now();
            if now.duration_since(bench.started) > bench.warmup {
                bench.frames.push(now.duration_since(bench.last).as_secs_f32() * 1000.0);
            }
            bench.last = now;
            // Orbit continuously so every frame renders a new view.
            self.state.viewport.camera.orbit(3.0, 0.6);
            ctx.request_repaint();
            if !bench.reported
                && now.duration_since(bench.started).as_secs_f32()
                    > bench.warmup.as_secs_f32() + self.automation.bench_seconds
            {
                bench.reported = true;
                let mut f = bench.frames.clone();
                f.sort_by(|a, b| a.total_cmp(b));
                let n = f.len().max(1);
                let mean: f32 = f.iter().sum::<f32>() / n as f32;
                let pct = |p: f32| f[((n as f32 - 1.0) * p) as usize];
                let info = frame.wgpu_render_state().map(|rs| rs.adapter.get_info());
                println!("soliddraft bench");
                println!(
                    "  adapter: {}",
                    info.as_ref()
                        .map(|i| format!("{} ({:?}, {:?})", i.name, i.backend, i.device_type))
                        .unwrap_or_default()
                );
                println!("  triangles: {}", self.state.doc.triangle_count());
                println!("  frames measured: {n} over {:.1} s", bench.frames.iter().sum::<f32>() / 1000.0);
                println!(
                    "  frame time ms: mean {mean:.2}  median {:.2}  p95 {:.2}  p99 {:.2}  max {:.2}",
                    pct(0.5),
                    pct(0.95),
                    pct(0.99),
                    f[n - 1]
                );
                println!("  fps: mean {:.0}  at p95 {:.0}", 1000.0 / mean, 1000.0 / pct(0.95));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if let Some(path) = self.automation.screenshot.clone() {
            if self.frame == self.automation.screenshot_after_frames && !self.screenshot_requested {
                self.screenshot_requested = true;
                log::info!("screenshot requested at frame {}", self.frame);
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            if self.frame < self.automation.screenshot_after_frames + 2 {
                ctx.request_repaint();
            }
            let image = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = image {
                let (w, h) = (image.width() as u32, image.height() as u32);
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                match image::save_buffer(&path, &bytes, w, h, image::ColorType::Rgba8) {
                    Ok(()) => println!("screenshot saved to {} ({w}x{h})", path.display()),
                    Err(e) => eprintln!("screenshot failed: {e}"),
                }
                if self.automation.exit_after_screenshot {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }
}

/// A file to open at startup, for `--open`.
pub fn open_at_start(app: &mut SolidDraftApp, path: PathBuf) {
    app.state.push(AppAction::Open(Some(path)));
}

impl SolidDraftApp {
    /// Hands the open assembly the coarse and fine levels the GPU asked for, which its
    /// loader brings in the background.
    fn request_assembly_levels(&mut self) {
        let (Some(rs), Some(s)) = (self.render_state.as_ref(), self.state.assembly.session.as_mut()) else { return };
        if !self.frame.is_multiple_of(15) {
            return;
        }
        let wanted = match rs.renderer.write().callback_resources.get_mut::<ViewportRenderer>() {
            Some(r) => r.take_assembly_wanted(),
            None => return,
        };
        if !wanted.is_empty() {
            s.doc.request_levels(&wanted);
        }
    }

    /// Asks for a screenshot to cut the picture of the part saved under `key` from. The
    /// browser cannot read a texture back while a frame waits, so it uses the window's
    /// own pixels, a frame later.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn picture_for_recent(&mut self, ctx: &egui::Context, key: String) {
        self.web_thumbnail = Some(key);
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(WebThumbnail)));
    }

    /// Cuts the viewport out of the screenshot [`Self::picture_for_recent`] asked for and
    /// keeps it with the recent document.
    #[cfg(target_arch = "wasm32")]
    fn keep_recent_picture(&mut self, ctx: &egui::Context) {
        if self.web_thumbnail.is_none() {
            return;
        }
        let shot = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, user_data, .. }
                    if user_data.data.as_ref().is_some_and(|d| d.is::<WebThumbnail>()) =>
                {
                    Some(image.clone())
                }
                _ => None,
            })
        });
        let (Some(image), Some(key)) = (shot, self.web_thumbnail.clone()) else { return };
        self.web_thumbnail = None;
        let ppp = ctx.pixels_per_point();
        // The part alone: the heads-up pills and the navigation bar along the bottom stay
        // out of the picture.
        let rect = self.state.viewport.rect;
        let rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - crate::viewport::HUD_BAND));
        let pixels: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
        let Some(whole) = image::RgbaImage::from_raw(image.width() as u32, image.height() as u32, pixels) else {
            return;
        };
        let (x, y) = ((rect.left() * ppp).max(0.0) as u32, (rect.top() * ppp).max(0.0) as u32);
        let (w, h) = ((rect.width() * ppp) as u32, (rect.height() * ppp) as u32);
        if w == 0 || h == 0 {
            return;
        }
        let cut = image::imageops::crop_imm(&whole, x, y, w, h).to_image();
        let small = image::imageops::thumbnail(&cut, 512, (512 * h / w).max(1));
        let mut png = Vec::new();
        match small.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png) {
            Ok(()) => crate::documents_browser::set_thumbnail(&key, &png),
            Err(e) => log::warn!("no picture for the recent list: {e}"),
        }
    }

    /// The first body of the part adopted without its exact geometry, if any.
    #[cfg(target_arch = "wasm32")]
    pub(super) fn unread_body(&self) -> Option<soliddraft_kernel_api::BodyId> {
        let model = self.state.doc.model();
        let kernel = model.kernel();
        model.live_bodies().into_iter().map(|(_, body)| body).find(|body| kernel.exact_unread(*body).unwrap_or(false))
    }

    /// Reads one unread body a frame once the busy state has been drawn, then lets the
    /// held actions run.
    #[cfg(target_arch = "wasm32")]
    fn poll_reading(&mut self, ctx: &egui::Context) {
        let Some(reading) = &self.reading else { return };
        if !reading.drawn {
            return;
        }
        if let Some(body) = self.unread_body() {
            if let Err(error) = self.state.doc.model().kernel().read_exact(body) {
                log::warn!("the page could not read a body's exact geometry: {error}");
            }
            ctx.request_repaint();
            return;
        }
        for action in self.reading.take().map(|reading| reading.held).unwrap_or_default() {
            self.state.push(action);
        }
        ctx.request_repaint();
    }

    /// The browser builds the part away from the page's thread. While a build is owed the
    /// page says so above the status bar and offers Cancel, which returns the part to its
    /// last accepted revision. While the page reads exact geometry a command needs, it says
    /// that instead, and Cancel drops the command.
    #[cfg(target_arch = "wasm32")]
    fn building(&mut self, ctx: &egui::Context) {
        let opening = self.opening.as_ref().map(|opening| opening.name.clone());
        let reading = self.reading.is_some();
        let pending = self.state.doc.proof_pending() || opening.is_some() || reading;
        let mut cancel = None;
        if !pending {
            self.state.cancelling = false;
        }
        if pending {
            egui::Area::new(egui::Id::new("browser-build"))
                .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -64.0))
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(match &opening {
                                Some(name) => format!("Opening {name}"),
                                None if reading => "Reading the part".to_owned(),
                                None if self.state.cancelling => "Cancelling…".to_owned(),
                                None => "Building the part".to_owned(),
                            });
                            let button = ui.add_enabled(!self.state.cancelling, egui::Button::new("Cancel"));
                            if button.clicked() {
                                // Cancelling an open keeps the part on screen; cancelling a
                                // read drops the actions waiting for it.
                                if self.opening.take().is_none() && self.reading.take().is_none() {
                                    self.state.cancelling = true;
                                    self.state.push(AppAction::CancelBuild);
                                }
                            }
                            cancel = Some(button.rect.center());
                        });
                    });
                });
        }
        if let Some(reading) = &mut self.reading {
            reading.drawn = true;
            ctx.request_repaint();
        }
        let cancel = cancel.map(|centre| serde_json::json!({ "x": centre.x, "y": centre.y }));
        let picking = self.state.viewport.picking;
        crate::web::show_build(serde_json::json!({
            "pending": pending,
            "cancel": cancel,
            "reading": reading,
            "picking": picking,
            "cancelling": self.state.cancelling,
        }));
    }

    fn edit_suite_frame(&mut self, root: &mut egui::Ui) {
        let chrome_frame = egui::Frame::new().fill(theme::chrome()).inner_margin(egui::Margin::ZERO);
        egui::Panel::top("menu")
            .exact_size(theme::MENU_HEIGHT)
            .frame(chrome_frame)
            .show_separator_line(false)
            .show(root, |ui| crate::top_bar::show(ui, &mut self.state, &mut self.dock, &mut self.open_parts));
        self.leave_start_page();
        let start = self.state.start.is_some();
        let status = self.status();
        if start {
            crate::command_line::panel(root, &mut self.state, &status);
        } else {
            // D1 Clear Bench top tool strip (workspace tabs + icon-over-label tools).
            crate::tool_palette::panel(root, &mut self.state);
            // Bottom bar (Zoo model: chips, ai, command line, physical analysis, filter, units, status).
            crate::command_line::panel(root, &mut self.state, &status);
            // Slim left toggle rail (Tree, History, Values, Props + Clear).
            crate::left_bar::panel(root, &mut self.state);
            let window_width = root.available_width();
            if self.state.tree_shown() || crate::feature_panel::active(&self.state) {
                self.side_panel(root, Side::Tree, window_width);
            }
            // Contextual inline form strip sits over the viewport without shifting the side column.
            crate::options_bar::panel(root, &mut self.state);
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::chrome())).show(root, |ui| {
            if start {
                crate::start_page::show(ui, &mut self.state);
            } else {
                let mut panels = Panels { state: &mut self.state };
                show_dock(ui, &mut self.dock, egui::Id::new("center_dock"), &mut panels);
            }
        });
        if !start {
            crate::options_bar::float(root, &mut self.state);
        }
    }

    /// Closes the start page once a document was opened, made or edited.
    fn leave_start_page(&mut self) {
        if self.state.start.as_ref().is_some_and(|page| !page.still_wanted(&self.state)) {
            self.state.start = None;
            // A New drawing whose part never arrived does not start one later.
            self.state.drawing_after_open = false;
        }
    }

    /// Hands autosave the documents as this frame left them, notes any copy that failed,
    /// and wakes the app when the next copy is due.
    fn autosave(&mut self, ctx: &egui::Context) {
        let Some(recovery) = self.state.recovery.as_mut() else { return };
        let backend = self.state.doc.kernel().backend();
        let kernel = format!("{} {}", backend.name, backend.version);
        let drawing = self.state.drawing.session.as_ref().map(|s| &s.doc);
        let wait = recovery.tick(&self.state.doc, drawing, &kernel, Instant::now());
        let failures = std::mem::take(&mut recovery.failures);
        if let Some(wait) = wait {
            ctx.request_repaint_after(wait);
        }
        if let Some(failure) = failures.into_iter().last() {
            self.state.note(failure, NoteKind::Error);
        }
    }
}

impl eframe::App for SolidDraftApp {
    /// In the browser a frame with a click takes the time the browser saw the click.
    #[cfg(target_arch = "wasm32")]
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        crate::web::stamp_pointer_time(raw_input);
    }

    /// A clean exit: autosave finishes and deletes this session's recovery copies.
    fn on_exit(&mut self) {
        if let Some(recovery) = self.state.recovery.take() {
            recovery.close();
        }
    }

    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        crate::multibody::show(root.ctx(), &mut self.state);
        crate::material_dialog::show(root.ctx(), &mut self.state);
        crate::properties::show_appearance_editor(root.ctx(), &mut self.state);
        let ctx = &root.ctx().clone();
        log::debug!("ui frame {}", self.frame);
        self.state.doc.set_repaint(ctx);
        self.focus_tool_document();
        let dt = ctx.input(|i| i.unstable_dt) * 1000.0;
        self.state.frame_ms = if self.state.frame_ms == 0.0 { dt } else { self.state.frame_ms * 0.9 + dt * 0.1 };
        self.state.viewport.overlay_input(ctx);
        self.shortcuts(ctx);
        crate::drawing::frame(&mut self.state, &mut self.dock);
        crate::assembly::frame(&mut self.state, &mut self.dock, ctx);
        // The browser hands over a dropped file's name and bytes, not a path to read.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dropped: Vec<PathBuf> =
                ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
            if !dropped.is_empty() {
                crate::assembly::dropped(&mut self.state, dropped);
            }
        }
        // They are read as an upload is: placed in the active assembly, or else opened.
        #[cfg(target_arch = "wasm32")]
        {
            let dropped: Vec<web_sys::File> =
                ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.web_file().cloned()).collect());
            let purpose = if self.state.assembly.is_active() {
                crate::platform::Purpose::Insert
            } else {
                crate::platform::Purpose::Open
            };
            crate::web::read(dropped, purpose);
        }
        self.request_assembly_levels();
        self.state.poll_assistant();
        if self.state.service_mcp() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        self.focus_tool_document();
        // In the browser an opened file arrives as an upload, frames after the request.
        if let Some(uploaded) = crate::platform::take_upload() {
            match uploaded.purpose {
                crate::platform::Purpose::Insert => crate::assembly::insert_bytes(&mut self.state, uploaded.files),
                crate::platform::Purpose::Open => self.open_uploads(uploaded.files),
                crate::platform::Purpose::Parameters => {
                    crate::values_tab::apply_upload(&mut self.state, uploaded.files)
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.poll_opening();
        #[cfg(target_arch = "wasm32")]
        self.poll_reading(ctx);

        if let Some(e) = self.state.editor.as_mut() {
            e.refresh(&self.state.doc);
        }
        // Native workers wake the context when they answer. In the browser the work
        // itself runs in these frames, so keep drawing while an answer is owed.
        if self.state.poll_drag() && cfg!(target_arch = "wasm32") {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        if let Some(error) = self.state.mass.poll(&self.state.doc) {
            self.state.note(error, NoteKind::Error);
        }
        // Hover info is derived from the viewport's last pick so every panel sees the same name.
        let viewport = &self.state.viewport;
        self.state.hover_info = viewport
            .hover
            .and_then(|p| selection_of(&self.state.doc, p))
            .or(viewport.hover_reference.map(Selection::Reference))
            .and_then(|sel| self.state.selection_filter().selection(Some(sel)))
            .and_then(|sel| self.state.doc.describe(sel));
        self.state.hover_point = self.state.viewport.hover_point;

        #[cfg(target_arch = "wasm32")]
        crate::web::begin_frame();
        self.edit_suite_frame(root);
        self.dialogs(ctx);
        // After the frame's controls are cleared, so the browser check finds its buttons.
        crate::assembly::show_choice(ctx, &mut self.state);
        crate::import::report::notice(ctx, &self.state);
        #[cfg(target_arch = "wasm32")]
        self.building(ctx);
        crate::preferences::window(ctx, &mut self.preferences, &mut self.state);
        self.apply_actions(ctx);
        self.leave_start_page();
        #[cfg(target_arch = "wasm32")]
        crate::web::feedback_document(&self.state);
        // Panel choices are written once they settle, never in the middle of a drag.
        if self.state.commands != self.saved_commands {
            if let Err(e) = crate::settings::save_command_history(&self.state.commands) {
                log::warn!("cannot save the command history: {e}");
            }
            self.saved_commands = self.state.commands.clone();
        }
        if self.state.create_pins != self.saved_create_pins {
            if let Err(e) = crate::settings::save_create_pins(&self.state.create_pins) {
                log::warn!("cannot save the pinned create commands: {e}");
            }
            self.saved_create_pins = self.state.create_pins.clone();
        }
        if self.state.layout != self.saved_layout && !ctx.input(|i| i.pointer.any_down()) {
            self.state.layout.save();
            self.saved_layout = self.state.layout;
        }
        if self.state.command_line.settings != self.saved_bar && !ctx.input(|i| i.pointer.any_down()) {
            if let Err(e) = crate::settings::save_command_line(&self.state.command_line.settings) {
                log::warn!("cannot save the command line: {e}");
            }
            self.saved_bar = self.state.command_line.settings.clone();
        }
        self.autosave(ctx);
        #[cfg(target_arch = "wasm32")]
        self.keep_recent_picture(ctx);
        self.automation_step(ctx, frame);
        if self.state.note.as_ref().is_some_and(|n| n.at.elapsed() < crate::command_line::NOTE_ON_BAR) {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        let title = if self.state.start.is_some() {
            "SolidDraft".to_owned()
        } else {
            format!("{}{} - SolidDraft", self.state.doc.name, if self.state.doc.dirty { " •" } else { "" })
        };
        if title != self.title {
            self.title = title.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }
    }
}
