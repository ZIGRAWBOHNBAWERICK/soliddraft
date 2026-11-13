//! The start page: what the window shows at launch, before a document is open. It offers
//! New part, New assembly, New drawing and Open, lists the recent documents with their
//! pictures, names, locations and dates, and points at work a crash left behind.
//!
//! The page stands in for the ribbon, the side panels and the viewport. It closes as soon
//! as there is a document to show: one is opened, made or restored, or the empty part
//! gets its first edit. File > Start page shows it again.

use egui::{Align, Layout, Rect, Sense, Stroke, StrokeKind, vec2};
use soliddraft_document::DocumentId;

use crate::recent::{self, RecentDoc, Source};
use crate::state::{AppAction, AppState};
use crate::theme;

const CARD_WIDTH: f32 = 216.0;
const PICTURE_HEIGHT: f32 = 140.0;
const TILE: egui::Vec2 = vec2(168.0, 76.0);

/// What the open documents were when the page appeared. The page closes once they change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Mark {
    part: DocumentId,
    edits: u64,
    drawing: bool,
    assembly: bool,
    busy: bool,
}

impl Mark {
    fn of(state: &AppState) -> Mark {
        Mark {
            part: state.doc.model().id(),
            edits: state.doc.edits,
            drawing: state.drawing.session.is_some() || state.drawing.new.is_some(),
            assembly: state.assembly.session.is_some(),
            busy: state.sketch.is_some() || state.editor.is_some(),
        }
    }
}

/// The start page's state while it shows.
pub struct StartPage {
    mark: Mark,
    loader: Option<recent::Loader>,
    /// The recent documents, once read.
    pub docs: Option<Vec<RecentDoc>>,
    pictures: Vec<Option<egui::TextureHandle>>,
}

impl StartPage {
    /// The page over the documents open now. `wake` draws a frame once the recent list
    /// has been read.
    pub fn new(state: &AppState, wake: impl Fn() + Send + 'static) -> StartPage {
        StartPage { mark: Mark::of(state), loader: Some(recent::load(wake)), docs: None, pictures: Vec::new() }
    }

    /// The page with the recent list given, for tests.
    pub fn with_docs(state: &AppState, docs: Vec<RecentDoc>) -> StartPage {
        StartPage { mark: Mark::of(state), loader: None, docs: Some(docs), pictures: Vec::new() }
    }

    /// Whether the page still stands for the open documents: none was opened, made or
    /// edited since it appeared.
    pub fn still_wanted(&self, state: &AppState) -> bool {
        Mark::of(state) == self.mark
    }

    fn poll(&mut self) {
        if let Some(docs) = self.loader.as_mut().and_then(recent::Loader::poll) {
            self.docs = Some(docs);
            self.loader = None;
            self.pictures.clear();
        }
    }
}

/// Shows the start page in `ui`, while `state.start` holds one.
pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let Some(mut page) = state.start.take() else { return };
    page.poll();
    egui::ScrollArea::vertical().id_salt("start_page").auto_shrink([false, false]).show(ui, |ui| {
        let margin = if ui.available_width() < 900.0 { 16 } else { 40 };
        egui::Frame::new().inner_margin(egui::Margin::symmetric(margin, 32)).show(ui, |ui| {
            ui.label(egui::RichText::new("Start").font(theme::font_semibold(24.0)).color(theme::ink()));
            ui.add_space(14.0);
            let tile_width = TILE.x.min(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
                if tile(ui, "New part", "A solid body from sketches and features", tile_width).clicked() {
                    state.push(AppAction::NewPart);
                }
                if tile(ui, "New assembly", "Parts placed and mated together", tile_width).clicked() {
                    run(state, "new_assembly");
                }
                if tile(ui, "New drawing", "Sheets with views of a part", tile_width).clicked() {
                    state.push(AppAction::NewDrawing);
                }
                if tile(ui, "Open…", "A part, assembly or drawing file", tile_width).clicked() {
                    run(state, "open");
                }
            });
            if state.recovery.as_ref().is_some_and(|r| !r.offers.is_empty()) {
                ui.add_space(18.0);
                recovery_banner(ui, state);
            }
            ui.add_space(28.0);
            ui.label(egui::RichText::new("Recent documents").font(theme::font_semibold(16.0)).color(theme::ink()));
            ui.add_space(10.0);
            match page.docs.as_ref() {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("Reading the recent documents").color(theme::muted()));
                    });
                }
                Some(docs) if docs.is_empty() => {
                    ui.label(egui::RichText::new("Documents you open or save appear here.").color(theme::muted()));
                }
                Some(docs) => {
                    let docs = docs.clone();
                    page.pictures.resize(docs.len(), None);
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(16.0, 16.0);
                        for (i, doc) in docs.iter().enumerate() {
                            let picture = picture(ui, &mut page.pictures[i], doc, i);
                            if card(ui, doc, picture.as_ref()).clicked() {
                                open(state, doc);
                            }
                        }
                    });
                }
            }
        });
    });
    state.start = Some(page);
}

fn run(state: &mut AppState, id: &str) {
    if let Some(command) = crate::commands::get(id) {
        state.run_command(command);
    }
}

fn open(state: &mut AppState, doc: &RecentDoc) {
    match &doc.source {
        Source::File(path) => state.push(AppAction::Open(Some(path.clone()))),
        #[cfg(target_arch = "wasm32")]
        Source::Browser(bytes) => {
            state.push(AppAction::OpenBytes { name: doc.name.clone(), bytes: bytes.as_ref().clone() })
        }
    }
}

/// One of the four large buttons at the top.
fn tile(ui: &mut egui::Ui, title: &str, caption: &str, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, TILE.y), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, title));
    let painter = ui.painter_at(rect);
    let fill = if response.hovered() { theme::hover() } else { theme::panel() };
    let stroke = if response.hovered() { theme::accent() } else { theme::line() };
    painter.rect(rect, 8, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(14.0, 12.0));
    painter.text(inner.left_top(), egui::Align2::LEFT_TOP, title, theme::font_semibold(15.0), theme::ink());
    let galley = painter.layout(caption.to_owned(), theme::font(11.5), theme::muted(), inner.width());
    painter.galley(inner.left_bottom() - vec2(0.0, galley.size().y), galley, theme::muted());
    response
}

fn recovery_banner(ui: &mut egui::Ui, state: &mut AppState) {
    egui::Frame::new()
        .fill(theme::warn_soft())
        .stroke(Stroke::new(1.0, theme::warn()))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let text = if cfg!(target_arch = "wasm32") {
                    "Changes from an earlier visit were not saved. They can be recovered."
                } else {
                    "SolidDraft did not close properly. Unsaved work can be recovered."
                };
                ui.label(egui::RichText::new(text).color(theme::ink()));
                if ui.button("Recover documents…").clicked() {
                    state.push(AppAction::ShowRecovery);
                }
            });
        });
}

/// The texture of a document's picture, made the first time it is drawn.
fn picture(
    ui: &egui::Ui,
    slot: &mut Option<egui::TextureHandle>,
    doc: &RecentDoc,
    index: usize,
) -> Option<egui::TextureHandle> {
    if slot.is_none()
        && let Some(image) = doc.thumbnail.clone()
    {
        *slot = Some(ui.ctx().load_texture(format!("start_page_picture_{index}"), image, Default::default()));
    }
    slot.clone()
}

/// A recent document: its picture, name, location and date. The whole card opens it.
fn card(ui: &mut egui::Ui, doc: &RecentDoc, picture: Option<&egui::TextureHandle>) -> egui::Response {
    let height = PICTURE_HEIGHT + 78.0;
    let (rect, response) = ui.allocate_exact_size(vec2(CARD_WIDTH, height), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, doc.name.as_str()));
    let hovered = response.hovered();
    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        8,
        theme::panel(),
        Stroke::new(1.0, if hovered { theme::accent() } else { theme::line() }),
        StrokeKind::Inside,
    );
    let frame = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(CARD_WIDTH - 16.0, PICTURE_HEIGHT));
    painter.rect_filled(frame, 5, theme::canvas_bg());
    match picture {
        Some(texture) => {
            let size = texture.size_vec2();
            let scale = (frame.width() / size.x).min(frame.height() / size.y);
            let shown = Rect::from_center_size(frame.center(), size * scale);
            painter.image(
                texture.id(),
                shown,
                Rect::from_min_max([0.0, 0.0].into(), [1.0, 1.0].into()),
                egui::Color32::WHITE,
            );
        }
        None => {
            let kind = recent::kind_name(doc.kind);
            painter.text(frame.center(), egui::Align2::CENTER_CENTER, kind, theme::font_medium(13.0), theme::muted());
        }
    }
    let text = Rect::from_min_max(frame.left_bottom() + vec2(2.0, 8.0), rect.right_bottom() - vec2(8.0, 6.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(text).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(text);
    // The cards sit in a row spaced for cards; their lines sit close together.
    child.spacing_mut().item_spacing.y = 3.0;
    let line = |child: &mut egui::Ui, s: &str, font: egui::FontId, color: egui::Color32| {
        child.add(egui::Label::new(egui::RichText::new(s).font(font).color(color)).truncate().selectable(false));
    };
    line(&mut child, doc.title(), theme::font_semibold(13.0), theme::ink());
    let kind = recent::kind_name(doc.kind);
    let when = doc.when_ms.map(recent::when_text).unwrap_or_default();
    let meta = if when.is_empty() { kind.to_owned() } else { format!("{kind} · {when}") };
    line(&mut child, &meta, theme::font(11.5), theme::muted());
    line(&mut child, &doc.location, theme::font(11.5), theme::muted());
    response.on_hover_text(match &doc.source {
        Source::File(path) => path.display().to_string(),
        #[cfg(target_arch = "wasm32")]
        Source::Browser(_) => format!("{} · kept in this browser", doc.name),
    })
}
