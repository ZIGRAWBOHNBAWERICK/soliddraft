//! D1 Clear Bench top bar: SolidDraft logo, Home button, single File menu (17.png model),
//! quick Save/Undo/Redo, open document tabs with close and +, coral Donate pill with real
//! project links, Preferences gear, Report a bug, and crate version label.

use crate::app::Tab;
use crate::commands;
use crate::document::Document;
use crate::file;
use crate::icons::{self, Icon};
use crate::state::{AppAction, AppState, NoteKind};
use crate::theme;
use crate::viewport::ViewportState;
use egui::{Align2, Color32, CornerRadius, Rect, RectAlign, Sense, Stroke, StrokeKind, pos2, vec2};
use egui_dock::DockState;
use soliddraft_document::DocumentId;

/// Real project donation and issue links (from `site/app/src/content.ts` & `design-brief.md`).
pub const KOFI_URL: &str = "https://ko-fi.com/pedropaiva56920";
pub const SPONSORS_URL: &str = "https://github.com/sponsors/BohnBawerick";
pub const ISSUES_URL: &str = "https://github.com/ZIGRAWBOHNBAWERICK/soliddraft/issues/new";

/// Every registry command id exposed directly inside the single `File` menu.
pub const FILE_MENU_COMMANDS: &[&str] = &[
    "new_part",
    "new_assembly",
    "make_assembly",
    "make_drawing",
    "open",
    "import_step",
    "import_iges",
    "import_mesh",
    "import_scan",
    "scan_workspace",
    "import_report",
    "insert_component",
    "save",
    "save_as",
    "assembly_save",
    "assembly_save_as",
    "drawing_save",
    "assembly_pack_and_go",
    "assembly_pack_and_go_zip",
    "export_step",
    "export_stl",
    "export_3mf",
    "export_iges",
    "export_obj",
    "assembly_export_urdf",
    "assembly_export_usd",
    "drawing_export_svg",
    "drawing_export_pdf",
    "drawing_export_dxf",
    "manufacturing_package",
    "file_locations",
    "shortcuts",
    "settings",
    "give_feedback",
    "about",
    "quit",
];

/// Every registry command id exposed on top-bar quick-access / utility buttons.
pub const TOP_BAR_COMMANDS: &[&str] = &["save", "assembly_save", "drawing_save", "undo", "redo", "settings", "about"];

/// A parked part tab when multiple parts are open in the top tab strip.
pub struct ParkedPart {
    pub doc: Document,
    pub viewport: ViewportState,
    /// The units this part's readouts use.
    pub unit: crate::state::DisplayUnit,
}

/// Manages open part tabs alongside the active part in `AppState::doc`.
#[derive(Default)]
pub struct OpenParts {
    /// Parked parts before the active part in tab order.
    pub before: Vec<ParkedPart>,
    /// Parked parts after the active part in tab order.
    pub after: Vec<ParkedPart>,
    next_number: usize,
}

impl OpenParts {
    pub fn total(&self) -> usize {
        self.before.len() + 1 + self.after.len()
    }

    pub fn active_index(&self) -> usize {
        self.before.len()
    }

    /// The tab index of the open part `instance`, the active one included.
    fn index_of(&self, instance: DocumentId, active: &Document) -> Option<usize> {
        if active.instance() == instance {
            return Some(self.before.len());
        }
        let parked = |p: &ParkedPart| p.doc.instance() == instance;
        self.before
            .iter()
            .position(parked)
            .or_else(|| self.after.iter().position(parked).map(|j| self.before.len() + 1 + j))
    }

    /// The part in the tab at `index`, the active one included.
    fn part<'a>(&'a self, index: usize, active: &'a Document) -> Option<&'a Document> {
        let here = self.before.len();
        match index.cmp(&here) {
            std::cmp::Ordering::Less => self.before.get(index).map(|p| &p.doc),
            std::cmp::Ordering::Equal => Some(active),
            std::cmp::Ordering::Greater => self.after.get(index - here - 1).map(|p| &p.doc),
        }
    }

    /// Names of all open part tabs in display order: `(name, dirty, is_active, index)`.
    pub fn list(&self, active_doc: &Document) -> Vec<(String, bool, bool, usize)> {
        let mut out = Vec::with_capacity(self.total());
        for (i, p) in self.before.iter().enumerate() {
            out.push((part_tab_title(&p.doc), p.doc.dirty, false, i));
        }
        let active_i = self.before.len();
        out.push((part_tab_title(active_doc), active_doc.dirty, true, active_i));
        for (j, p) in self.after.iter().enumerate() {
            out.push((part_tab_title(&p.doc), p.doc.dirty, false, active_i + 1 + j));
        }
        out
    }

    /// Parks the currently active part and activates a fresh blank part in a new tab.
    pub fn open_new_part(&mut self, state: &mut AppState, dock: &mut DockState<Tab>) {
        self.next_number = self.next_number.max(self.total()) + 1;
        let name = format!("Part{}", self.next_number);
        let new_doc = Document::new(&name, crate::kernel());
        state.start = None;
        state.chat.preview = None;
        state.import_dialog = None;
        state.scan = Default::default();
        self.park_active(state, new_doc);
        crate::drawing::show_part_tab(dock);
        state.drawing.active = false;
        state.assembly.active = false;
        state.note(format!("Opened new tab {name}"), NoteKind::Info);
    }

    /// Parks the active part and its view, and makes `new_doc` the active part in a new
    /// tab at the end of the strip. An open editor or sketch ends first, and puts back the
    /// rollback it set for its view, so the parked part keeps the rollback the user left.
    pub fn park_active(&mut self, state: &mut AppState, new_doc: Document) {
        state.end_document_views();
        let mut new_viewport = ViewportState::default();
        new_viewport.appearance = state.appearance.clone();
        new_viewport.preset = state.viewport.preset;
        new_viewport.references = state.viewport.references;
        new_viewport.grid_style = state.viewport.grid_style;
        new_viewport.scene = state.viewport.scene;
        new_viewport.display = state.viewport.display;
        new_viewport.camera.fit_framed(None);

        let old_doc = std::mem::replace(&mut state.doc, new_doc);
        let old_vp = std::mem::replace(&mut state.viewport, new_viewport);
        let unit = std::mem::take(&mut state.display_unit);
        self.before.push(ParkedPart { doc: old_doc, viewport: old_vp, unit });
        self.before.append(&mut self.after);
    }

    /// Switches to the part tab at `index`.
    pub fn activate(&mut self, index: usize, state: &mut AppState, dock: &mut DockState<Tab>) {
        state.start = None;
        crate::drawing::show_part_tab(dock);
        state.drawing.active = false;
        state.assembly.active = false;

        let active = self.before.len();
        if index == active || index >= self.total() {
            return;
        }
        // The part going back to its tab keeps the rollback the user left, not the one an
        // editor or sketch set for its view.
        state.end_document_views();
        state.chat.preview = None;

        // Flatten all parts into one ordered list, swap the chosen one into `state`.
        let mut all: Vec<ParkedPart> = Vec::with_capacity(self.total());
        all.append(&mut self.before);
        let current_doc = std::mem::replace(&mut state.doc, Document::new("_swap", crate::kernel()));
        let current_vp = std::mem::take(&mut state.viewport);
        all.push(ParkedPart { doc: current_doc, viewport: current_vp, unit: state.display_unit });
        all.append(&mut self.after);

        let chosen = all.remove(index);
        state.doc = chosen.doc;
        state.viewport = chosen.viewport;
        state.viewport.appearance = state.appearance.clone();
        state.display_unit = chosen.unit;
        let rest = all.split_off(index);
        self.before = all;
        self.after = rest;
    }

    /// Closes the part tab at `index`. Returns true if no other open document remains
    /// (caller should open the Start page).
    pub fn close(&mut self, index: usize, state: &mut AppState, dock: &mut DockState<Tab>) -> bool {
        let active = self.before.len();
        if index < active {
            self.before.remove(index);
            return false;
        }
        if index > active {
            let after_idx = index - active - 1;
            if after_idx < self.after.len() {
                self.after.remove(after_idx);
            }
            return false;
        }
        // Closing the currently active part tab.
        if let Some(next) = (!self.after.is_empty()).then(|| self.after.remove(0)) {
            state.editor = None;
            state.sketch = None;
            state.doc = next.doc;
            state.viewport = next.viewport;
            state.viewport.appearance = state.appearance.clone();
            state.display_unit = next.unit;
            return false;
        }
        if let Some(prev) = self.before.pop() {
            state.editor = None;
            state.sketch = None;
            state.doc = prev.doc;
            state.viewport = prev.viewport;
            state.viewport.appearance = state.appearance.clone();
            state.display_unit = prev.unit;
            return false;
        }
        // Last part tab closed: switch to Assembly or Drawing if open, else Start page.
        if state.assembly.session.is_some() {
            crate::assembly::show_tab(dock);
            state.assembly.active = true;
            return false;
        }
        if state.drawing.session.is_some() {
            crate::drawing::show_drawing_tab(dock);
            state.drawing.active = true;
            return false;
        }
        true
    }
}

fn part_tab_title(doc: &Document) -> String {
    match &doc.path {
        Some(path) => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("{}.{}", doc.name, file::EXTENSION)),
        None => format!("{}.{}", doc.name, file::EXTENSION),
    }
}

/// Renders the D1 Clear Bench 36px top bar.
pub fn show(ui: &mut egui::Ui, state: &mut AppState, dock: &mut DockState<Tab>, open_parts: &mut OpenParts) {
    let rect = ui.max_rect();
    ui.painter().hline(rect.x_range(), rect.bottom() - 0.5, Stroke::new(1.0, theme::line()));

    ui.horizontal_centered(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        ui.add_space(6.0);

        // 1. Logo + "SolidDraft"
        brand_mark(ui);

        // 2. Home button (toggles Start page)
        let on_start = state.start.is_some();
        let home_resp = icon_button(
            ui,
            Icon::Home,
            "Home",
            if on_start { "Return to active document" } else { "Home · Start page" },
            true,
            on_start,
        );
        if home_resp.clicked() {
            if on_start {
                state.start = None;
            } else {
                state.push(AppAction::ShowStartPage);
            }
        }

        // 3. Single File button (17.png model)
        file_menu_button(ui, state);

        // 4. Quick-access Save / Undo / Redo
        quick_access(ui, state);

        // Subtle vertical divider before document tabs
        let div_x = ui.cursor().left() + 2.0;
        ui.painter().vline(div_x, rect.y_range().shrink(8.0), Stroke::new(1.0, theme::line()));
        ui.add_space(6.0);

        // 5. Right utility cluster (right-to-left) + Center document tabs
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            ui.add_space(8.0);

            // Version label at far right (from crate version)
            let version = concat!("v", env!("CARGO_PKG_VERSION"));
            let ver_resp = ui
                .add(
                    egui::Button::new(egui::RichText::new(version).font(theme::mono(11.0)).color(theme::muted()))
                        .fill(Color32::TRANSPARENT)
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(theme::RADIUS)),
                )
                .on_hover_text("About SolidDraft");
            if ver_resp.clicked() {
                state.push(AppAction::ShowAbout);
            }

            // Report a bug (opens project issue page)
            let bug_resp = utility_text_button(ui, Icon::Bug, "Report a bug", "Open the SolidDraft issue tracker");
            if bug_resp.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(ISSUES_URL));
            }

            // Preferences gear icon button
            let prefs_tip = match state.shortcuts.shortcut_text("settings") {
                Some(key) => format!("Preferences ({key})"),
                None => "Preferences".to_owned(),
            };
            if icon_button(ui, Icon::Gear, "Preferences", &prefs_tip, true, false).clicked()
                && let Some(cmd) = commands::get("settings")
            {
                state.run_command(cmd);
            }

            // Coral Donate pill with real donation links
            donate_button(ui);

            // Centre: the document tabs and `+`, hidden on the start page. They keep to the
            // room left of the buttons above: more tabs than fit scroll there instead of
            // running under Donate.
            if !on_start {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    egui::ScrollArea::horizontal()
                        .id_salt("document_tabs")
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.horizontal_centered(|ui| {
                                ui.spacing_mut().item_spacing.x = 3.0;
                                document_tabs(ui, state, dock, open_parts);
                            });
                        });
                });
            }
        });
    });
    close_question(ui.ctx(), state, dock, open_parts);
}

/// A document tab the user asked to close while it holds unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Closing {
    pub target: CloseTarget,
    /// The tab's title, as the question names it.
    pub title: String,
    /// Save was chosen: the tab closes once the save has left the document clean.
    saving: bool,
}

/// Which document tab closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseTarget {
    /// One open copy of a part, by its instance: two tabs of one file are two copies.
    Part(DocumentId),
    Assembly,
    Drawing,
}

/// The title of the assembly's or the drawing's tab when it is open with unsaved changes.
fn unsaved_document(state: &AppState, target: CloseTarget) -> Option<String> {
    match target {
        CloseTarget::Part(_) => None,
        CloseTarget::Assembly => state
            .assembly
            .session
            .as_ref()
            .filter(|s| s.doc.is_dirty())
            .map(|s| format!("{}.dwasm", s.doc.assembly().name)),
        CloseTarget::Drawing => {
            state.drawing.session.as_ref().filter(|s| s.doc.dirty).map(|s| format!("{}.dwdrw", s.title()))
        }
    }
}

/// The title of `target`'s tab when it is open with unsaved changes.
fn unsaved(state: &AppState, open_parts: &OpenParts, target: CloseTarget) -> Option<String> {
    match target {
        CloseTarget::Part(instance) => {
            let doc = open_parts.part(open_parts.index_of(instance, &state.doc)?, &state.doc)?;
            doc.dirty.then(|| part_tab_title(doc))
        }
        _ => unsaved_document(state, target),
    }
}

/// Asks Save, Discard or Cancel before an assembly or drawing with unsaved changes
/// closes. Returns true when it asked, and the tab stays open until the answer.
pub(crate) fn ask_before_closing(state: &mut AppState, target: CloseTarget) -> bool {
    match unsaved_document(state, target) {
        Some(title) => {
            state.closing = Some(Closing { target, title, saving: false });
            true
        }
        None => false,
    }
}

/// Closes a document tab, or asks first when it holds unsaved changes.
fn request_close(target: CloseTarget, state: &mut AppState, dock: &mut DockState<Tab>, open_parts: &mut OpenParts) {
    match unsaved(state, open_parts, target) {
        Some(title) => state.closing = Some(Closing { target, title, saving: false }),
        None => close_now(target, state, dock, open_parts),
    }
}

/// Closes a document tab, dropping what it holds.
fn close_now(target: CloseTarget, state: &mut AppState, dock: &mut DockState<Tab>, open_parts: &mut OpenParts) {
    match target {
        CloseTarget::Part(instance) => {
            if let Some(index) = open_parts.index_of(instance, &state.doc)
                && open_parts.close(index, state, dock)
            {
                state.push(AppAction::ShowStartPage);
            }
        }
        CloseTarget::Assembly => {
            state.assembly.session = None;
            state.assembly.active = false;
            if let Some(loc) = dock.find_tab(&Tab::Assembly) {
                dock.remove_tab(loc);
            }
        }
        CloseTarget::Drawing => {
            state.drawing.session = None;
            state.drawing.active = false;
            if let Some(loc) = dock.find_tab(&Tab::Drawing) {
                dock.remove_tab(loc);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

/// Asks whether to save a document whose tab the user is closing: Save saves it and
/// closes the tab, Discard closes it without saving, Cancel leaves it open.
fn close_question(ctx: &egui::Context, state: &mut AppState, dock: &mut DockState<Tab>, open_parts: &mut OpenParts) {
    let Some(closing) = state.closing.clone() else { return };
    if unsaved(state, open_parts, closing.target).is_none() {
        // Saved, or saved and closed some other way: nothing is lost by closing.
        state.closing = None;
        close_now(closing.target, state, dock, open_parts);
        return;
    }
    if closing.saving {
        // The save ran with the last frame's actions and left changes unsaved: it was
        // cancelled or refused, so the tab stays open. A refusal already says why.
        state.closing = None;
        if state.note.as_ref().is_none_or(|note| note.kind != NoteKind::Error) {
            state.note(format!("{} was not saved, so its tab stays open", closing.title), NoteKind::Error);
        }
        return;
    }
    let mut choice = None;
    let modal = egui::Modal::new(egui::Id::new("close_unsaved_document")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(
            egui::RichText::new(format!("Save changes to {}?", closing.title)).family(theme::semibold()).size(13.5),
        );
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Closing the tab without saving loses the changes made since it was last saved.")
                .color(theme::muted()),
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            for (label, pick) in
                [("Save", CloseChoice::Save), ("Discard", CloseChoice::Discard), ("Cancel", CloseChoice::Cancel)]
            {
                let button = ui.button(label);
                #[cfg(target_arch = "wasm32")]
                crate::web::show_control(format!("Close question: {label}"), button.rect);
                if button.clicked() {
                    choice = Some(pick);
                }
            }
        });
    });
    if choice.is_none() && modal.should_close() {
        choice = Some(CloseChoice::Cancel);
    }
    match choice {
        None => {}
        Some(CloseChoice::Cancel) => state.closing = None,
        Some(CloseChoice::Discard) => {
            state.closing = None;
            close_now(closing.target, state, dock, open_parts);
        }
        Some(CloseChoice::Save) => {
            let save = match closing.target {
                CloseTarget::Part(instance) => {
                    // Save writes the active part: a part waiting in its tab comes forward.
                    if let Some(index) = open_parts.index_of(instance, &state.doc) {
                        open_parts.activate(index, state, dock);
                    }
                    AppAction::Save
                }
                CloseTarget::Assembly => AppAction::Assembly(crate::assembly::AssemblyAction::Save(None)),
                CloseTarget::Drawing => AppAction::Drawing(crate::drawing::DrawingAction::Save(None)),
            };
            state.push(save);
            state.closing = Some(Closing { saving: true, ..closing });
        }
    }
}

fn brand_mark(ui: &mut egui::Ui) {
    let (brand, _) = ui.allocate_exact_size(vec2(92.0, 26.0), Sense::hover());
    let sq = Rect::from_center_size(pos2(brand.left() + 10.0, brand.center().y), vec2(16.0, 16.0));
    let logo = theme::logo();
    let glint = if theme::view_uses_custom_logo() { Color32::WHITE } else { Color32::from_rgb(0x60, 0xa5, 0xfa) };
    ui.painter().rect_filled(sq, CornerRadius::same(4), logo);
    ui.painter().rect_filled(
        Rect::from_min_max(sq.min + vec2(6.0, 6.0), sq.max),
        CornerRadius { nw: 0, ne: 0, sw: 0, se: 4 },
        logo.lerp_to_gamma(glint, 0.72),
    );
    ui.painter().text(
        pos2(brand.left() + 24.0, brand.center().y),
        Align2::LEFT_CENTER,
        "SolidDraft",
        theme::font_semibold(13.0),
        theme::ink(),
    );
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str, tip: &str, enabled: bool, selected: bool) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), sense);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label));
    if selected {
        ui.painter().rect(
            rect,
            CornerRadius::same(theme::RADIUS),
            theme::panel(),
            Stroke::new(1.0, theme::line()),
            StrokeKind::Inside,
        );
    } else if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(theme::RADIUS), theme::hover());
    }
    let color = if !enabled {
        theme::muted().gamma_multiply(0.45)
    } else if selected {
        theme::accent()
    } else if resp.hovered() {
        theme::ink()
    } else {
        theme::muted()
    };
    icons::paint(ui.painter(), icon, Rect::from_center_size(rect.center(), vec2(15.0, 15.0)), color);
    if enabled && resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(theme::RADIUS),
            Stroke::new(1.5, theme::accent()),
            StrokeKind::Inside,
        );
    }
    resp.on_hover_text(tip)
}

fn utility_text_button(ui: &mut egui::Ui, icon: Icon, text: &str, tip: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::font_medium(11.5), Color32::PLACEHOLDER);
    let size = vec2(galley.size().x + 28.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    let hot = resp.hovered() || resp.has_focus();
    if hot {
        ui.painter().rect(
            rect,
            CornerRadius::same(theme::RADIUS),
            theme::hover(),
            Stroke::new(1.0, theme::line()),
            StrokeKind::Inside,
        );
    }
    let ink = if hot { theme::ink() } else { theme::muted() };
    icons::paint(
        ui.painter(),
        icon,
        Rect::from_center_size(pos2(rect.left() + 12.0, rect.center().y), vec2(13.0, 13.0)),
        ink,
    );
    ui.painter().galley(pos2(rect.left() + 22.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    resp.on_hover_text(tip)
}

fn donate_button(ui: &mut egui::Ui) {
    let text = "Donate";
    let galley = ui.painter().layout_no_wrap(text.to_owned(), theme::font_semibold(11.5), theme::donate_ink());
    let size = vec2(galley.size().x + 32.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Donate"));
    let fill = if resp.hovered() { theme::donate_bg().gamma_multiply(0.95) } else { theme::donate_bg() };
    ui.painter().rect(
        rect,
        CornerRadius::same(theme::radii().pill),
        fill,
        Stroke::new(1.0, theme::donate_line()),
        StrokeKind::Inside,
    );
    icons::paint(
        ui.painter(),
        Icon::Heart,
        Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(12.0, 12.0)),
        theme::donate_ink(),
    );
    ui.painter().galley(pos2(rect.left() + 24.0, rect.center().y - galley.size().y / 2.0), galley, theme::donate_ink());
    let resp = resp.on_hover_text("Support SolidDraft development");
    egui::Popup::menu(&resp)
        .align(RectAlign::BOTTOM_END)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(210.0);
            ui.label(egui::RichText::new("Support SolidDraft").family(theme::semibold()).size(12.5));
            ui.label(
                egui::RichText::new("Free, open-source parametric CAD. Donations fund development.")
                    .size(11.0)
                    .color(theme::muted()),
            );
            ui.add_space(6.0);
            if ui.button("Support on Ko-fi").on_hover_text(KOFI_URL).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(KOFI_URL));
                ui.close();
            }
            if ui.button("GitHub Sponsors").on_hover_text(SPONSORS_URL).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(SPONSORS_URL));
                ui.close();
            }
        });
}

fn quick_access(ui: &mut egui::Ui, st: &mut AppState) {
    // Save, Undo and Redo act on the active document, and say which step they take.
    let save_id = if st.drawing.is_active() {
        "drawing_save"
    } else if st.assembly.is_active() {
        "assembly_save"
    } else {
        "save"
    };
    let (undo_label, redo_label) = commands::undo_labels(st);
    for (id, icon) in [(save_id, Icon::Save), ("undo", Icon::Undo), ("redo", Icon::Redo)] {
        let Some(command) = commands::get(id) else {
            debug_assert!(false, "no command {id}");
            continue;
        };
        let enabled = command.is_enabled(st);
        let tip = match id {
            "undo" => match &undo_label {
                Some(l) => format!("Undo {l} ({})", st.shortcuts.shortcut_text(id).unwrap_or_default()),
                None => command.tip(st),
            },
            "redo" => match &redo_label {
                Some(l) => format!("Redo {l} ({})", st.shortcuts.shortcut_text(id).unwrap_or_default()),
                None => command.tip(st),
            },
            _ => command.tip(st),
        };
        let button = icon_button(ui, icon, command.label, &tip, enabled, false);
        #[cfg(target_arch = "wasm32")]
        crate::web::show_control(format!("Quick {}", command.label), button.rect);
        if button.clicked() {
            st.run_command(command);
        }
    }
}

fn file_menu_button(ui: &mut egui::Ui, st: &mut AppState) {
    let galley = ui.painter().layout_no_wrap("File".to_owned(), theme::font_semibold(12.5), theme::ink());
    let size = vec2(galley.size().x + 28.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "File"));
    let open = egui::Popup::is_id_open(ui.ctx(), resp.id.with("popup"));
    let stroke = if open || resp.hovered() { theme::accent() } else { theme::line() };
    let fill = if open {
        theme::accent_soft()
    } else if resp.hovered() {
        theme::hover()
    } else {
        theme::panel()
    };
    ui.painter().rect(rect, CornerRadius::same(theme::RADIUS), fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    ui.painter().galley(pos2(rect.left() + 9.0, rect.center().y - galley.size().y / 2.0), galley, theme::ink());
    icons::paint(
        ui.painter(),
        Icon::ChevronDown,
        Rect::from_center_size(pos2(rect.right() - 10.0, rect.center().y), vec2(11.0, 11.0)),
        theme::muted(),
    );

    egui::Popup::menu(&resp)
        .align(RectAlign::BOTTOM_START)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(248.0);
            file_menu_contents(ui, st);
        });
}

fn menu_command(ui: &mut egui::Ui, st: &mut AppState, id: &str, label: &str) {
    let Some(command) = commands::get(id) else {
        debug_assert!(false, "no command {id}");
        return;
    };
    if !(command.shown)(st) {
        return;
    }
    let mut button = egui::Button::new(label);
    if let Some(shortcut) = st.shortcuts.shortcut_text(command.id) {
        button = button.shortcut_text(shortcut);
    }
    let tip = command.tip(st);
    if ui.add_enabled(command.is_enabled(st), button).on_hover_text(&tip).on_disabled_hover_text(&tip).clicked() {
        st.run_command(command);
        ui.close();
    }
}

fn file_menu_contents(ui: &mut egui::Ui, st: &mut AppState) {
    menu_command(ui, st, "new_part", "New part");
    menu_command(ui, st, "new_assembly", "New assembly");
    // An assembly offers nothing that works on a single part.
    if !st.assembly.is_active() {
        menu_command(ui, st, "make_assembly", "Make assembly from part");
        menu_command(ui, st, "make_drawing", "New drawing from part…");
    }
    menu_command(ui, st, "open", "Open…");

    #[cfg(not(target_arch = "wasm32"))]
    ui.menu_button("Open recent", |ui| {
        if st.recent.is_empty() {
            ui.add_enabled(false, egui::Button::new("No recent files"));
        }
        for path in st.recent.clone() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if ui.button(name).on_hover_text(path.display().to_string()).clicked() {
                st.push(AppAction::Open(Some(path)));
                ui.close();
            }
        }
        ui.separator();
        if ui.add_enabled(!st.recent.is_empty(), egui::Button::new("Clear list")).clicked() {
            st.push(AppAction::ClearRecent);
            ui.close();
        }
    });

    ui.separator();
    ui.label(egui::RichText::new("INSERT").family(theme::semibold()).size(10.0).color(theme::muted()));
    if st.assembly.is_active() {
        menu_command(ui, st, "insert_component", "Insert component…");
    } else {
        menu_command(ui, st, "import_step", "Import STEP…");
        menu_command(ui, st, "import_iges", "Import IGES…");
        menu_command(ui, st, "import_mesh", "Import mesh solid…");
        menu_command(ui, st, "import_scan", "Import scan reference…");
        menu_command(ui, st, "scan_workspace", "Scan workspace");
        menu_command(ui, st, "import_report", "Import report");
    }

    ui.separator();
    if st.drawing.is_active() {
        menu_command(ui, st, "drawing_save", "Save drawing");
    } else if st.assembly.is_active() {
        menu_command(ui, st, "assembly_save", "Save assembly");
        menu_command(ui, st, "assembly_save_as", "Save assembly as…");
        menu_command(ui, st, "assembly_pack_and_go", "Pack and Go…");
        menu_command(ui, st, "assembly_pack_and_go_zip", "Pack and Go to zip…");
    } else {
        menu_command(ui, st, "save", "Save");
        menu_command(ui, st, "save_as", "Save as…");
    }

    ui.separator();
    ui.label(egui::RichText::new("EXPORT").family(theme::semibold()).size(10.0).color(theme::muted()));
    if st.drawing.is_active() {
        menu_command(ui, st, "drawing_export_svg", "Export drawing SVG…");
        menu_command(ui, st, "drawing_export_pdf", "Export drawing PDF…");
        menu_command(ui, st, "drawing_export_dxf", "Export drawing DXF…");
    } else if st.assembly.is_active() {
        menu_command(ui, st, "assembly_export_urdf", "Export URDF…");
        menu_command(ui, st, "assembly_export_usd", "Export USD…");
    } else {
        menu_command(ui, st, "export_step", "Export STEP AP242…");
        menu_command(ui, st, "export_stl", "Export STL…");
        menu_command(ui, st, "export_3mf", "Export 3MF…");
        menu_command(ui, st, "export_iges", "Export IGES 5.3…");
        menu_command(ui, st, "export_obj", "Export OBJ…");
        menu_command(ui, st, "manufacturing_package", "Export manufacturing package…");
    }

    ui.separator();
    if ui.button("Design properties…").clicked() {
        st.show_design_properties = true;
        ui.close();
    }
    menu_command(ui, st, "file_locations", "File locations…");
    if ui.button("Customize app bar…").clicked() {
        st.show_customize_bar = true;
        ui.close();
    }
    menu_command(ui, st, "shortcuts", "Keyboard shortcuts…");

    #[cfg(target_arch = "wasm32")]
    if crate::journal_browser::has_offers() && ui.button("Crash report journals…").clicked() {
        st.push(AppAction::ShowRecovery);
        ui.close();
    }
    if st.recovery.as_ref().is_some_and(|r| !r.offers.is_empty() || !r.journals.is_empty())
        && ui.button("Recover documents…").clicked()
    {
        st.push(AppAction::ShowRecovery);
        ui.close();
    }

    ui.separator();
    // Prominently highlighted Preferences entry (17.png & step_208 model)
    if let Some(prefs_cmd) = commands::get("settings").filter(|c| (c.shown)(st)) {
        let shortcut = st.shortcuts.shortcut_text("settings").unwrap_or_default();
        let frame = egui::Frame::new()
            .fill(theme::accent_soft())
            .stroke(Stroke::new(1.0, theme::accent().gamma_multiply(0.55)))
            .corner_radius(CornerRadius::same(theme::RADIUS))
            .inner_margin(egui::Margin::symmetric(6, 3));
        frame.show(ui, |ui| {
            let btn =
                egui::Button::new(egui::RichText::new("Preferences…").family(theme::semibold()).color(theme::ink()))
                    .fill(Color32::TRANSPARENT)
                    .stroke(Stroke::NONE)
                    .shortcut_text(shortcut);
            if ui.add_sized(vec2(ui.available_width(), 22.0), btn).clicked() {
                st.run_command(prefs_cmd);
                ui.close();
            }
        });
    }

    menu_command(ui, st, "give_feedback", "Give feedback…");
    menu_command(ui, st, "about", "About SolidDraft");
    menu_command(ui, st, "quit", "Quit");
}

enum TabAction {
    SelectPart(usize),
    ClosePart(usize),
    SelectAssembly,
    CloseAssembly,
    SelectDrawing,
    CloseDrawing,
}

fn document_tabs(ui: &mut egui::Ui, state: &mut AppState, dock: &mut DockState<Tab>, open_parts: &mut OpenParts) {
    let mut action: Option<TabAction> = None;
    let on_start = state.start.is_some();
    let part_active = !on_start && !state.assembly.is_active() && !state.drawing.is_active();
    let asm_active = !on_start && state.assembly.is_active();
    let drw_active = !on_start && state.drawing.is_active();

    let parts = open_parts.list(&state.doc);
    for (title, dirty, is_current, idx) in parts {
        let active = part_active && is_current;
        match tab_pill(ui, idx, Icon::Extrude, &title, dirty, active) {
            TabClick::Select => action = Some(TabAction::SelectPart(idx)),
            TabClick::Close => action = Some(TabAction::ClosePart(idx)),
            TabClick::None => {}
        }
    }

    if let Some(session) = state.assembly.session.as_ref() {
        let title = format!("{}.dwasm", session.doc.assembly().name);
        match tab_pill(ui, usize::MAX - 1, Icon::Assembly, &title, session.doc.is_dirty(), asm_active) {
            TabClick::Select => action = Some(TabAction::SelectAssembly),
            TabClick::Close => action = Some(TabAction::CloseAssembly),
            TabClick::None => {}
        }
    }

    if let Some(session) = state.drawing.session.as_ref() {
        let title = format!("{}.dwdrw", session.title());
        match tab_pill(ui, usize::MAX, Icon::Sheet, &title, session.doc.dirty || session.has_issues(), drw_active) {
            TabClick::Select => action = Some(TabAction::SelectDrawing),
            TabClick::Close => action = Some(TabAction::CloseDrawing),
            TabClick::None => {}
        }
    }

    // `+` button to open a new or existing part, assembly, or drawing
    let (plus_rect, plus_resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    plus_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "New tab"));
    #[cfg(target_arch = "wasm32")]
    crate::web::show_control("New tab", plus_rect);
    if plus_resp.hovered() {
        ui.painter().rect(
            plus_rect,
            CornerRadius::same(theme::RADIUS),
            theme::hover(),
            Stroke::new(1.0, theme::line_strong()),
            StrokeKind::Inside,
        );
    }
    icons::paint(
        ui.painter(),
        Icon::Plus,
        Rect::from_center_size(plus_rect.center(), vec2(13.0, 13.0)),
        if plus_resp.hovered() { theme::ink() } else { theme::muted() },
    );
    let plus_resp = plus_resp.on_hover_text("Open a new or existing part, assembly or drawing");
    let mut new_part_tab = false;
    egui::Popup::menu(&plus_resp)
        .align(RectAlign::BOTTOM_START)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(210.0);
            ui.label(egui::RichText::new("OPEN IN NEW TAB").family(theme::semibold()).size(10.0).color(theme::muted()));
            ui.add_space(2.0);
            let new_part = ui.button("New part (.dwprt)");
            #[cfg(target_arch = "wasm32")]
            crate::web::show_control("New part (.dwprt)", new_part.rect);
            if new_part.clicked() {
                new_part_tab = true;
                ui.close();
            }
            if ui.button("New assembly (.dwasm)").clicked() {
                if let Some(cmd) = commands::get("new_assembly") {
                    state.run_command(cmd);
                }
                ui.close();
            }
            let new_drawing = ui.button("New drawing (.dwdrw)");
            #[cfg(target_arch = "wasm32")]
            crate::web::show_control("New drawing (.dwdrw)", new_drawing.rect);
            if new_drawing.clicked() {
                state.push(AppAction::NewDrawing);
                ui.close();
            }
            ui.separator();
            if ui.button("Open existing file…").clicked() {
                // A part opened from here takes a new tab; the active part stays open.
                state.open_in_new_tab = true;
                if let Some(cmd) = commands::get("open") {
                    state.run_command(cmd);
                }
                ui.close();
            }
        });

    if new_part_tab {
        open_parts.open_new_part(state, dock);
    }

    if let Some(act) = action {
        match act {
            TabAction::SelectPart(idx) => open_parts.activate(idx, state, dock),
            TabAction::ClosePart(idx) => {
                if let Some(instance) = open_parts.part(idx, &state.doc).map(Document::instance) {
                    request_close(CloseTarget::Part(instance), state, dock, open_parts);
                }
            }
            TabAction::SelectAssembly => {
                state.start = None;
                crate::assembly::show_tab(dock);
                state.assembly.active = true;
                state.drawing.active = false;
            }
            TabAction::CloseAssembly => request_close(CloseTarget::Assembly, state, dock, open_parts),
            TabAction::SelectDrawing => {
                state.start = None;
                crate::drawing::show_drawing_tab(dock);
                state.drawing.active = true;
                state.assembly.active = false;
            }
            TabAction::CloseDrawing => request_close(CloseTarget::Drawing, state, dock, open_parts),
        }
    }
}

enum TabClick {
    None,
    Select,
    Close,
}

/// One document tab. `slot` keeps the ids of two tabs with the same file name apart.
fn tab_pill(ui: &mut egui::Ui, slot: usize, icon: Icon, title: &str, dirty: bool, active: bool) -> TabClick {
    let font = if active { theme::font_semibold(12.0) } else { theme::font_medium(12.0) };
    let ink = if active { theme::ink() } else { theme::muted() };
    let galley = ui.painter().layout_no_wrap(title.to_owned(), font, ink);
    let dot_space = if dirty { 12.0 } else { 0.0 };
    let width = (galley.size().x + 48.0 + dot_space).clamp(96.0, 210.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 26.0), Sense::hover());

    let close_rect = Rect::from_center_size(pos2(rect.right() - 12.0, rect.center().y), vec2(16.0, 16.0));
    let main_rect = Rect::from_min_max(rect.min, pos2(close_rect.left() - 2.0, rect.bottom()));

    let main_resp = ui.interact(main_rect, ui.id().with(("doc_tab", slot)), Sense::click());
    main_resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, title));

    let close_resp = ui.interact(close_rect, ui.id().with(("doc_tab_close", slot)), Sense::click());
    close_resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Close {title}")));
    #[cfg(target_arch = "wasm32")]
    {
        crate::web::show_control(format!("Tab {title}"), main_rect);
        crate::web::show_control(format!("Close {title}"), close_rect);
    }

    let hovered = main_resp.hovered() || close_resp.hovered();
    let (fill, stroke) = if active {
        (theme::panel(), Stroke::new(1.0, theme::line()))
    } else if hovered {
        (theme::hover(), Stroke::NONE)
    } else {
        (Color32::TRANSPARENT, Stroke::NONE)
    };
    ui.painter().rect(rect, CornerRadius::same(theme::RADIUS), fill, stroke, StrokeKind::Inside);

    let icon_color = if active { theme::accent() } else { theme::muted() };
    icons::paint(
        ui.painter(),
        icon,
        Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(13.0, 13.0)),
        icon_color,
    );

    let text_x = rect.left() + 25.0;
    let text_max = close_rect.left() - 4.0 - dot_space;
    let clip = Rect::from_min_max(pos2(text_x, rect.top()), pos2(text_max.max(text_x), rect.bottom()));
    ui.painter().with_clip_rect(clip).galley(pos2(text_x, rect.center().y - galley.size().y / 2.0), galley, ink);

    if dirty {
        ui.painter().circle_filled(pos2(close_rect.left() - 6.0, rect.center().y), 3.0, theme::accent());
    }

    if close_resp.hovered() {
        ui.painter().rect_filled(close_rect, CornerRadius::same(4), theme::hover());
    }
    icons::paint(
        ui.painter(),
        Icon::Close,
        close_rect.shrink(3.5),
        if close_resp.hovered() { theme::ink() } else { theme::muted().gamma_multiply(0.75) },
    );

    if close_resp.clicked() {
        TabClick::Close
    } else if main_resp.clicked() {
        TabClick::Select
    } else {
        TabClick::None
    }
}

/// Dialogs launched from the single `File` menu (`Design Properties…` and `Customize App Bar…`).
pub fn dialogs(ctx: &egui::Context, state: &mut AppState) {
    if state.show_design_properties {
        let mut open = true;
        egui::Window::new("Design properties")
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("design_props_grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                    ui.label(egui::RichText::new("Document").color(theme::muted()));
                    ui.label(egui::RichText::new(&state.doc.name).family(theme::semibold()));
                    ui.end_row();

                    ui.label(egui::RichText::new("Path").color(theme::muted()));
                    let path = state
                        .doc
                        .path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "Not saved yet".to_owned());
                    ui.label(path);
                    ui.end_row();

                    ui.label(egui::RichText::new("Units").color(theme::muted()));
                    ui.label(format!("{} ({})", state.display_unit.name(), state.display_unit.short()));
                    ui.end_row();

                    ui.label(egui::RichText::new("Material").color(theme::muted()));
                    let mat = state.doc.material().map_or("Unassigned", |m| m.name.as_str());
                    ui.label(mat);
                    ui.end_row();

                    ui.label(egui::RichText::new("Features").color(theme::muted()));
                    ui.label(format!("{}", state.doc.features.len()));
                    ui.end_row();

                    ui.label(egui::RichText::new("Geometry engine").color(theme::muted()));
                    ui.label(state.doc.backend_name());
                    ui.end_row();
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Assign material…").clicked() {
                        state.push(AppAction::ShowMaterial);
                    }
                    if ui.button("Open Values").clicked() {
                        state.show_pane(crate::panels::Pane::Values);
                    }
                });
            });
        state.show_design_properties = open;
    }

    if state.show_customize_bar {
        let mut open = true;
        egui::Window::new("Customize app bar")
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(
                        "Pinned quick tools on the bottom command bar. Drag any ribbon tool onto the bar to pin it.",
                    )
                    .color(theme::muted()),
                );
                ui.add_space(8.0);
                let tools = state.command_line.settings.tools.clone();
                let mut remove = None;
                for id in &tools {
                    let label = commands::get(id).map_or(id.as_str(), |c| c.label);
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Remove").clicked() {
                                remove = Some(id.clone());
                            }
                        });
                    });
                }
                if let Some(id) = remove {
                    state.command_line.settings.remove(&id);
                }
                ui.add_space(8.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Reset bar tools").clicked() {
                        state.command_line.settings.tools = crate::command_line::BarSettings::default().tools;
                    }
                    if ui.button("Reset panel layout").clicked() {
                        state.push(AppAction::ResetLayout);
                    }
                });
            });
        state.show_customize_bar = open;
    }
}
