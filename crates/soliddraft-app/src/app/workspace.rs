//! Workspace lifetime, recovery and the frame loop.

use crate::document::{Document, Selection};
use crate::panels::{Layout, Side};
use crate::state::{AppAction, AppState, NoteKind};
use crate::theme;
use crate::viewport::look::{Preset, Tier};
use crate::viewport::render::{DisplayMode, ViewportRenderer};
use crate::{file, ribbon};
use egui_dock::DockState;
use std::path::PathBuf;
use std::time::Duration;
use web_time::Instant;

use super::dispatch::selection_of;
use super::presentation::docks::{Panels, Tab, center_dock, show_dock, tree_dock};

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
    pub(super) tree_dock: DockState<Tab>,
    /// The panel layout as last written to disk.
    saved_layout: Layout,
    /// The command line's chips and session height as last written to disk.
    saved_bar: crate::command_line::BarSettings,
    saved_commands: crate::commands::History,
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
        theme::install_for_layout(
            &cc.egui_ctx,
            automation.theme.unwrap_or_else(|| crate::settings::appearance().theme),
            crate::settings::frame_layout(),
        );
        if let Some(rs) = cc.wgpu_render_state.as_ref() {
            let tier = Tier::of(rs.adapter.get_info().backend);
            let renderer = ViewportRenderer::new(&rs.device, &rs.queue, rs.target_format, tier);
            rs.renderer.write().callback_resources.insert(renderer);
        }
        log::debug!("renderer installed");
        #[cfg(target_arch = "wasm32")]
        crate::web::set_context(&cc.egui_ctx);
        let mut state = AppState::new(doc);
        state.viewport.camera.fit(state.doc.bounds());
        let view = crate::settings::view();
        let preset = automation.preset.unwrap_or(view.preset);
        state.viewport.preset = preset;
        state.viewport.references = view.references;
        state.viewport.grid_style = view.grid;
        state.viewport.scene = (state.frame_layout == crate::settings::FrameLayout::EditSuite).then_some(view.scene);
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
            state,
            dock: center_dock(),
            tree_dock: tree_dock(),
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
    pub(super) fn open_bytes(&mut self, name: &str, bytes: &[u8]) {
        use soliddraft_document::DocKind;
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
            Some(DocKind::Assembly) => self.state.note(
                format!("{name}: assemblies open from their folder, with their parts, in the desktop app"),
                NoteKind::Error,
            ),
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
        match opened {
            Ok(doc) => {
                self.close_editor();
                self.state.sketch = None;
                self.state.chat.preview = None;
                let n = doc.features.len();
                self.state.doc = doc;
                self.state.remember_revision();
                self.state.viewport.camera.fit(self.state.doc.bounds());
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
        let rect = self.state.viewport.rect;
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

    fn classic_frame(&mut self, root: &mut egui::Ui) {
        self.frame_with_tools(root, false);
    }

    fn frame_with_tools(&mut self, root: &mut egui::Ui, palette: bool) {
        let edit_suite = palette;
        let palette = palette && !self.state.drawing.is_active() && !self.state.assembly.is_active();
        // The menu bar is chrome; the ribbon paints its own strip and body.
        let chrome_frame = egui::Frame::new().fill(theme::chrome()).inner_margin(egui::Margin::ZERO);
        let panel_frame = egui::Frame::new().fill(theme::panel()).inner_margin(egui::Margin::ZERO);
        egui::Panel::top("menu")
            .exact_size(theme::MENU_HEIGHT)
            .frame(chrome_frame)
            .show_separator_line(true)
            .show(root, |ui| self.menu_bar(ui));
        self.leave_start_page();
        let start = self.state.start.is_some();
        if !start {
            if palette {
                crate::options_bar::panel(root, &mut self.state);
            } else {
                egui::Panel::top("ribbon")
                    .exact_size(theme::RIBBON_TABS_HEIGHT + theme::RIBBON_BODY_HEIGHT)
                    .frame(panel_frame)
                    .show_separator_line(false)
                    .show(root, |ui| ribbon::show(ui, &mut self.state));
            }
        }
        let status = self.status();
        if !edit_suite {
            crate::command_line::panel(root, &mut self.state, &status);
        } else if start || !palette {
            crate::command_line::edit_suite_panel(root, &mut self.state, &status);
        }
        let window_width = root.available_width();
        if !start {
            if palette {
                crate::tool_palette::plane_picker_panel(root, &mut self.state);
                crate::tool_palette::panel(root, &mut self.state);
                crate::command_line::edit_suite_panel(root, &mut self.state, &status);
                crate::timeline::panel(root, &mut self.state);
            }
            if !palette || self.state.layout.tree.shown {
                self.side_panel(root, Side::Tree, window_width);
            }
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::chrome())).show(root, |ui| {
            if start {
                crate::start_page::show(ui, &mut self.state);
            } else {
                let mut panels = Panels { state: &mut self.state };
                show_dock(ui, &mut self.dock, egui::Id::new("center_dock"), &mut panels);
            }
        });
    }

    fn edit_suite_frame(&mut self, root: &mut egui::Ui) {
        self.frame_with_tools(root, true);
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
    /// A clean exit: autosave finishes and deletes this session's recovery copies.
    fn on_exit(&mut self) {
        if let Some(recovery) = self.state.recovery.take() {
            recovery.close();
        }
    }

    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        theme::set_layout(ctx, self.state.frame_layout);
        log::debug!("ui frame {}", self.frame);
        self.state.doc.set_repaint(ctx);
        self.focus_tool_document();
        let dt = ctx.input(|i| i.unstable_dt) * 1000.0;
        self.state.frame_ms = if self.state.frame_ms == 0.0 { dt } else { self.state.frame_ms * 0.9 + dt * 0.1 };
        self.state.viewport.overlay_input(ctx);
        self.shortcuts(ctx);
        crate::drawing::frame(&mut self.state, &mut self.dock);
        crate::assembly::frame(&mut self.state, &mut self.dock, ctx);
        // The browser hands over a dropped file's name only, not a path to read.
        #[cfg(not(target_arch = "wasm32"))]
        {
            let dropped: Vec<PathBuf> =
                ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
            if !dropped.is_empty() {
                crate::assembly::dropped(&mut self.state, dropped);
            }
        }
        self.request_assembly_levels();
        self.state.poll_assistant();
        if self.state.service_mcp() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        self.focus_tool_document();
        // In the browser an opened file arrives as an upload, frames after the request.
        if let Some((name, bytes)) = crate::platform::take_upload() {
            self.open_bytes(&name, &bytes);
        }

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

        match self.state.frame_layout {
            crate::settings::FrameLayout::Classic => self.classic_frame(root),
            crate::settings::FrameLayout::EditSuite => self.edit_suite_frame(root),
        }
        self.dialogs(ctx);
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
