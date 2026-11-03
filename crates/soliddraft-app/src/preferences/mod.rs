//! The Preferences window: a searchable list of categories on the left and the chosen
//! page on the right. It fills Navigation > Mouse, Navigation > Orbit and zoom,
//! Navigation > View keys, Viewport > Grid and planes, Interface > Shortcuts and General
//! and documents > Autosave and recovery; the other categories are selectable and
//! searchable, with a Coming soon page until their controls work.
//!
//! [`Preferences`] holds the window's state and a draft of the settings it edits. Every
//! accepted change is handed back as a [`Change`]; the app saves it
//! (`AppState::set_navigation`, `AppState::set_reference_display`,
//! `AppState::set_grid_style`, `AppState::set_shortcuts`,
//! `AppState::set_autosave`) and the tests can do the same
//! without a GPU.

mod ui;

use crate::commands::{self, KeyChord, ShortcutConflict, ShortcutPreset, Shortcuts};
use crate::navigation::view_keys::{self, ViewKeys};
use crate::navigation::{Chord, NavigationSettings, Refusal, Router, SchemaError, presets};
use crate::settings::{GridStyle, NotKept, ReferenceDisplay};
use soliddraft_document::AutosavePolicy;

/// A page of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Page {
    /// Stable: `navigation.mouse`.
    pub id: &'static str,
    pub category: &'static str,
    pub title: &'static str,
}

impl Page {
    /// As the window writes where it is: "Navigation > Mouse".
    pub fn path(&self) -> String {
        let category = categories().iter().find(|c| c.id == self.category).map_or(self.category, |c| c.label);
        format!("{category} > {}", self.title)
    }
}

pub const MOUSE: &str = "navigation.mouse";
pub const ORBIT_ZOOM: &str = "navigation.orbit_zoom";
pub const VIEW_KEYS: &str = "navigation.view_keys";
pub const GRID_AND_PLANES: &str = "viewport.grid_and_planes";
pub const SHORTCUTS: &str = "interface.shortcuts";
pub const AUTOSAVE: &str = "general_documents.autosave";

const PAGES: [Page; 6] = [
    Page { id: MOUSE, category: "navigation", title: "Mouse" },
    Page { id: ORBIT_ZOOM, category: "navigation", title: "Orbit and zoom" },
    Page { id: VIEW_KEYS, category: "navigation", title: "View keys" },
    Page { id: GRID_AND_PLANES, category: "viewport", title: "Grid and planes" },
    Page { id: SHORTCUTS, category: "interface", title: "Shortcuts" },
    Page { id: AUTOSAVE, category: "general_documents", title: "Autosave and recovery" },
];

/// The pages with controls, in the order the list shows them.
pub fn available_pages() -> &'static [Page] {
    &PAGES
}

pub fn page(id: &str) -> Option<&'static Page> {
    PAGES.iter().find(|p| p.id == id)
}

/// The category whose controls are not available yet.
pub fn reserved_category(id: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.id == id && c.status == Status::Reserved)
}

/// Whether a category has pages yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Available,
    /// Its page shows Coming soon.
    Reserved,
}

/// A category of the left list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Category {
    pub id: &'static str,
    pub label: &'static str,
    pub status: Status,
}

const CATEGORIES: [Category; 7] = [
    Category { id: "navigation", label: "Navigation", status: Status::Available },
    Category { id: "viewport", label: "Viewport", status: Status::Available },
    Category { id: "selection_editing", label: "Selection and editing", status: Status::Reserved },
    Category { id: "interface", label: "Interface", status: Status::Available },
    Category { id: "appearance_graphics", label: "Appearance and graphics", status: Status::Reserved },
    Category { id: "general_documents", label: "General and documents", status: Status::Available },
    Category { id: "assistant", label: "Assistant", status: Status::Reserved },
];

pub fn categories() -> &'static [Category] {
    &CATEGORIES
}

/// The categories kept for later pages.
pub fn reserved_categories() -> Vec<&'static Category> {
    CATEGORIES.iter().filter(|c| c.status == Status::Reserved).collect()
}

/// A search result: a page, and the control on it when the match was one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub page: &'static str,
    /// The control the window highlights: `wheel_direction`, `preset.fusion`.
    pub control: Option<String>,
    pub label: String,
}

/// Everything search can find: the pages, their controls, the presets, the view-key
/// profiles and every binding.
fn index() -> Vec<Hit> {
    let hit = |page, control: Option<String>, label: String| Hit { page, control, label };
    let mut all: Vec<Hit> = PAGES.iter().map(|p| hit(p.id, None, p.path())).collect();
    all.extend(
        reserved_categories().into_iter().map(|c| hit(c.id, None, format!("{} settings · Coming soon", c.label))),
    );
    for (control, label) in ui::ORBIT_ZOOM_CONTROLS {
        all.push(hit(ORBIT_ZOOM, Some(control.to_owned()), label.to_owned()));
    }
    for (control, label) in ui::MOUSE_CONTROLS {
        all.push(hit(MOUSE, Some(control.to_owned()), label.to_owned()));
    }
    for p in presets() {
        all.push(hit(MOUSE, Some(format!("preset.{}", p.id)), format!("{} navigation preset", p.label)));
        for b in &p.bindings {
            all.push(hit(
                MOUSE,
                Some(format!("row.{}", b.id)),
                format!("{}: {} {}", p.label, b.action.label(), b.text(false)),
            ));
        }
    }
    for v in view_keys::profiles() {
        all.push(hit(VIEW_KEYS, Some(format!("profile.{}", v.id)), format!("{} view keys", v.label)));
        for b in &v.rows {
            all.push(hit(
                VIEW_KEYS,
                Some(format!("key.{}", b.id)),
                format!("{}: {} {}", v.label, b.action.label(), b.text(false)),
            ));
        }
    }
    for (control, label) in ui::VIEW_KEY_CONTROLS {
        all.push(hit(VIEW_KEYS, Some(control.to_owned()), label.to_owned()));
    }
    for (control, label) in ui::GRID_AND_PLANES_CONTROLS {
        all.push(hit(GRID_AND_PLANES, Some(control.to_owned()), label.to_owned()));
    }
    for (control, label) in ui::SHORTCUTS_CONTROLS {
        all.push(hit(SHORTCUTS, Some(control.to_owned()), label.to_owned()));
    }
    for preset in ShortcutPreset::ALL {
        all.push(hit(
            SHORTCUTS,
            Some(format!("shortcut_preset.{}", preset.id())),
            format!("{} shortcut preset", preset.label()),
        ));
    }
    for c in commands::all() {
        let label = if !c.in_app() {
            format!("{} · Automation only", c.label)
        } else if c.is_working() {
            match c.shortcut_text() {
                Some(k) => format!("{}: {k}", c.shortcut_row_label()),
                None => c.shortcut_row_label(),
            }
        } else {
            format!("{} · Coming soon", c.label)
        };
        all.push(hit(SHORTCUTS, Some(format!("command.{}", c.id)), label));
    }
    for (control, label) in ui::AUTOSAVE_CONTROLS {
        all.push(hit(AUTOSAVE, Some(control.to_owned()), label.to_owned()));
    }
    all
}

/// What matches `query`, ignoring case and the spaces around it. Every word must appear.
pub fn search(query: &str) -> Vec<Hit> {
    let words: Vec<String> = query.trim().to_lowercase().split_whitespace().map(str::to_owned).collect();
    if words.is_empty() {
        return Vec::new();
    }
    index().into_iter().filter(|h| words.iter().all(|w| h.label.to_lowercase().contains(w.as_str()))).collect()
}

/// A change the window asks the app to make.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Save and use these navigation settings.
    Navigation(NavigationSettings),
    /// Save and use these display choices.
    Display(ReferenceDisplay),
    /// Use this grid style, and save it when `keep`. A slider being dragged shows its
    /// value at once and is saved when it is let go.
    Grid { style: GridStyle, keep: bool },
    /// Save and use these command shortcuts.
    Shortcuts(Shortcuts),
    /// Save and use these autosave settings.
    Autosave(AutosavePolicy),
}

/// A line the window shows under its page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub error: bool,
}

/// The window's state.
pub struct Preferences {
    pub page: &'static str,
    pub search: String,
    /// The control a search result pointed at.
    pub highlight: Option<String>,
    /// The navigation settings as the window shows them.
    pub draft: NavigationSettings,
    /// The display choices as the window shows them.
    pub display: ReferenceDisplay,
    /// The grid style as the window shows it.
    pub grid: GridStyle,
    /// The grid style was changed and not saved yet.
    grid_pending: bool,
    /// The command shortcuts as the window shows them.
    pub shortcuts: Shortcuts,
    /// Filters the command rows on the Shortcuts page.
    pub shortcut_search: String,
    /// The command waiting for a new shortcut key press.
    pub shortcut_capture: Option<&'static str>,
    /// A shortcut assignment waiting for the user to confirm a swap.
    pub shortcut_conflict: Option<ShortcutConflict>,
    /// The autosave settings as the window shows them.
    pub autosave: AutosavePolicy,
    /// The autosave settings were changed and not saved yet.
    autosave_pending: bool,
    pub message: Option<Message>,
    /// The row waiting for a new gesture from the practice area.
    pub capture: Option<&'static str>,
    /// Names what the last gesture in the practice area did.
    pub practice_label: String,
    practice: Router,
    mac: bool,
    changes: Vec<Change>,
}

impl Preferences {
    /// The window on Navigation > Mouse with `settings`, `display` and `grid`.
    pub fn new(settings: &NavigationSettings, display: ReferenceDisplay, grid: GridStyle) -> Self {
        let mac = cfg!(target_os = "macos");
        Self {
            page: MOUSE,
            search: String::new(),
            highlight: None,
            draft: settings.clone(),
            display,
            grid,
            grid_pending: false,
            shortcuts: crate::settings::shortcuts(),
            shortcut_search: String::new(),
            shortcut_capture: None,
            shortcut_conflict: None,
            autosave: AutosavePolicy::default(),
            autosave_pending: false,
            message: None,
            capture: None,
            practice_label: "Try a gesture here".to_owned(),
            practice: Router::new(settings, mac),
            mac,
            changes: Vec::new(),
        }
    }

    /// Sets the command shortcuts the window starts with.
    pub fn with_shortcuts(mut self, shortcuts: &Shortcuts) -> Self {
        self.shortcuts = shortcuts.clone();
        self
    }

    /// Whether the Shortcuts page is waiting for the next key press.
    pub fn is_capturing_key(&self) -> bool {
        self.page == SHORTCUTS && self.search.trim().is_empty() && self.shortcut_capture.is_some()
    }

    /// The window showing `policy` as the autosave settings.
    pub fn with_autosave(mut self, policy: AutosavePolicy) -> Self {
        self.autosave = policy;
        self
    }

    /// Shows `policy` as the autosave settings, and hands it to the app to save once
    /// `settle` is true, so a value being dragged is saved when it is let go.
    pub fn edit_autosave(&mut self, policy: AutosavePolicy, settle: bool) {
        if policy != self.autosave {
            self.autosave = policy;
            self.autosave_pending = true;
            self.message = None;
        }
        if self.autosave_pending && settle {
            self.changes.push(Change::Autosave(self.autosave));
            self.autosave_pending = false;
        }
    }

    /// The changes made since the last call, for the app to save.
    pub fn take_changes(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.changes)
    }

    /// Puts the draft back to what was saved, after the app could not save a change.
    pub fn revert(&mut self, saved: &NavigationSettings, why: String) {
        self.draft = saved.clone();
        self.message = Some(Message { text: why, error: true });
    }

    pub fn open_page(&mut self, id: &str) {
        let id = page(id).map(|p| p.id).or_else(|| reserved_category(id).map(|c| c.id));
        if let Some(id) = id {
            self.page = id;
            self.search.clear();
            self.highlight = None;
            self.message = None;
            self.capture = None;
            self.shortcut_capture = None;
            self.shortcut_conflict = None;
        }
    }

    /// Opens a search result's page and highlights its control.
    pub fn open_hit(&mut self, hit: &Hit) {
        self.open_page(hit.page);
        self.highlight = hit.control.clone();
        self.search.clear();
    }

    pub(super) fn commit(&mut self, settings: NavigationSettings, said: Option<String>) {
        self.draft = settings;
        self.message = said.map(|text| Message { text, error: false });
        self.changes.push(Change::Navigation(self.draft.clone()));
    }

    fn refuse(&mut self, refusal: Refusal) -> Result<(), Refusal> {
        let text = match &refusal {
            Refusal::Invalid(errors) if errors.iter().any(|e| matches!(e, SchemaError::DuplicateChord { .. })) => {
                "These actions use the same gesture. Try another gesture.".to_owned()
            }
            _ => refusal.to_string(),
        };
        self.message = Some(Message { text, error: true });
        Err(refusal)
    }

    /// Changes the display choices with `edit` and hands them to the app.
    pub fn edit_display(&mut self, edit: impl FnOnce(&mut ReferenceDisplay)) {
        let before = self.display;
        edit(&mut self.display);
        if self.display != before {
            self.message = None;
            self.changes.push(Change::Display(self.display));
        }
    }

    /// Shows `style` as the grid's, and hands it to the app: to use while `settle` is
    /// false, as during a drag, and to save once it is true.
    pub fn edit_grid(&mut self, style: GridStyle, settle: bool) {
        let changed = style != self.grid;
        if changed {
            self.grid = style;
            self.grid_pending = true;
            self.message = None;
        }
        if self.grid_pending && (changed || settle) {
            self.changes.push(Change::Grid { style, keep: settle });
            self.grid_pending = !settle;
        }
    }

    /// Says a change applies but was not kept.
    pub fn not_kept(&mut self, why: &NotKept) {
        let text = format!("Not saved: {why}. {}", crate::settings::store::lasts());
        self.message = Some(Message { text, error: true });
    }

    /// Changes the draft with `edit` and hands it to the app.
    pub fn edit(&mut self, edit: impl FnOnce(&mut NavigationSettings)) {
        let mut s = self.draft.clone();
        edit(&mut s);
        if s != self.draft {
            self.commit(s, None);
        }
    }

    pub fn choose_preset(&mut self, id: &str) -> Result<(), Refusal> {
        let mut s = self.draft.clone();
        match s.select_preset(id) {
            Ok(()) => {
                let label = s.active_preset().label;
                self.commit(s, Some(format!("Navigation now follows {label}.")));
                Ok(())
            }
            Err(r) => self.refuse(r),
        }
    }

    /// Changes a row of the active preset. Refused, with nothing written, when the gesture
    /// is another row's.
    pub fn customize(&mut self, binding: &str, chord: Chord) -> Result<(), Refusal> {
        let mut s = self.draft.clone();
        let preset = s.active_preset().id;
        match s.set_override(preset, binding, chord) {
            Ok(()) => {
                let label =
                    s.active_preset().bindings.iter().find(|b| b.id == binding).map_or("Mouse", |b| b.action.label());
                self.commit(s, Some(format!("{label} gesture changed.")));
                Ok(())
            }
            Err(r) => self.refuse(r),
        }
    }

    /// Drops the active preset's changed rows; the wheel, camera and view keys stay.
    pub fn reset_preset(&mut self) {
        let mut s = self.draft.clone();
        let id = s.active_preset().id;
        if s.reset_preset(id).is_ok() {
            self.commit(s, Some("The preset's rows are back to its defaults.".into()));
        }
    }

    /// Puts the Orbit and zoom page's controls back; the preset and its rows stay.
    pub fn reset_orbit_and_zoom(&mut self) {
        let mut s = self.draft.clone();
        s.reset_orbit_and_zoom();
        self.commit(s, Some("Orbit and zoom are back to their defaults.".into()));
    }

    pub fn choose_view_keys(&mut self, id: &str) -> Result<ViewKeys, Refusal> {
        let mut s = self.draft.clone();
        match view_keys::apply_to(&mut s, id) {
            Ok(keys) => {
                self.commit(s, Some(format!("View keys now follow {}.", keys.profile.label)));
                Ok(keys)
            }
            Err(r) => {
                self.message = Some(Message { text: r.to_string(), error: true });
                Err(r)
            }
        }
    }

    pub fn reset_view_keys(&mut self) {
        let mut s = self.draft.clone();
        view_keys::reset_in(&mut s);
        self.commit(s, Some("View keys are back to SolidDraft's own.".into()));
    }

    fn commit_shortcuts(&mut self, said: Option<String>) {
        self.message = said.map(|text| Message { text, error: false });
        self.changes.push(Change::Shortcuts(self.shortcuts.clone()));
    }

    /// Starts waiting for a key press to assign to command `id`.
    pub fn start_shortcut_capture(&mut self, id: &'static str) {
        let Some(c) = commands::get(id).filter(|c| c.in_app() && c.is_working()) else { return };
        self.shortcut_capture = Some(c.id);
        self.shortcut_conflict = None;
        self.message = Some(Message {
            text: format!("Press a new key for {}, or Esc to cancel.", c.shortcut_row_label()),
            error: false,
        });
    }

    /// Stops waiting for a key press without changing any shortcut.
    pub fn cancel_shortcut_capture(&mut self) {
        self.shortcut_capture = None;
        self.message = None;
    }

    /// Assigns `chord` to command `id`. When another command in a conflicting scope
    /// already holds `chord`, records the conflict and offers to swap instead of
    /// overwriting it.
    pub fn assign_shortcut(&mut self, id: &str, chord: KeyChord) -> Result<(), ShortcutConflict> {
        let Some(c) = commands::get(id).filter(|c| c.in_app() && c.is_working()) else { return Ok(()) };
        self.shortcut_capture = None;
        match self.shortcuts.assign(c.id, chord) {
            Ok(()) => {
                self.shortcut_conflict = None;
                self.commit_shortcuts(Some(format!("{}: shortcut set to {}.", c.shortcut_row_label(), chord.text())));
                Ok(())
            }
            Err(conflict) => {
                self.shortcut_conflict = Some(conflict.clone());
                self.message = None;
                Err(conflict)
            }
        }
    }

    /// Swaps the pending conflicting shortcut between the target command and the command
    /// that held it.
    pub fn swap_shortcut(&mut self) -> bool {
        let Some(conflict) = self.shortcut_conflict.take() else { return false };
        self.shortcut_capture = None;
        self.shortcuts.swap(&conflict);
        let target = commands::get(conflict.command).map_or(conflict.command.to_owned(), |c| c.shortcut_row_label());
        let others: Vec<String> = conflict
            .with
            .iter()
            .filter_map(|id| commands::get(id))
            .map(|c| {
                let now = self.shortcuts.shortcut_text(c.id).unwrap_or_else(|| "None".to_owned());
                format!("{} ({now})", c.shortcut_row_label())
            })
            .collect();
        self.commit_shortcuts(Some(format!(
            "Swapped shortcuts: {target} is now {}; {}.",
            conflict.chord.text(),
            others.join(", ")
        )));
        true
    }

    /// Dismisses the pending shortcut conflict without changing any shortcut.
    pub fn cancel_shortcut_conflict(&mut self) {
        self.shortcut_conflict = None;
        self.message = None;
    }

    /// Clears command `id`'s shortcut.
    pub fn clear_shortcut(&mut self, id: &str) {
        let Some(c) = commands::get(id).filter(|c| c.in_app() && c.is_working()) else { return };
        self.shortcut_capture = None;
        self.shortcut_conflict = None;
        self.shortcuts.clear(c.id);
        self.commit_shortcuts(Some(format!("{}: shortcut cleared.", c.shortcut_row_label())));
    }

    /// Resets command `id`'s shortcut back to the active preset's default.
    pub fn reset_shortcut(&mut self, id: &str) {
        let Some(c) = commands::get(id).filter(|c| c.in_app() && c.is_working()) else { return };
        self.shortcut_capture = None;
        self.shortcut_conflict = None;
        if let Some(conflict) = self.shortcuts.reset_conflict(c.id) {
            self.shortcut_conflict = Some(conflict);
            self.message = None;
            return;
        }
        self.shortcuts.reset(c.id);
        let text = match self.shortcuts.shortcut_text(c.id) {
            Some(k) => format!("{}: shortcut reset to {k}.", c.shortcut_row_label()),
            None => format!("{}: shortcut reset to None.", c.shortcut_row_label()),
        };
        self.commit_shortcuts(Some(text));
    }

    /// Resets all shortcuts to `preset`.
    pub fn reset_all_shortcuts(&mut self, preset: ShortcutPreset) {
        self.shortcut_capture = None;
        self.shortcut_conflict = None;
        self.shortcuts.reset_all(preset);
        let mut navigation = self.draft.clone();
        navigation.keyboard_profile = preset.id().to_owned();
        self.commit(navigation, None);
        self.commit_shortcuts(Some(format!("Shortcuts reset to {}.", preset.label())));
    }

    /// Draws the window's contents.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui::show(self, ui);
    }
}

/// The Preferences window, while `prefs` holds one: draws it, saves what it changed
/// and drops it when closed.
pub fn window(ctx: &egui::Context, prefs: &mut Option<Preferences>, state: &mut crate::state::AppState) {
    let Some(p) = prefs.as_mut() else { return };
    let mut open = true;
    egui::Window::new("Preferences")
        .open(&mut open)
        .default_size([980.0, 640.0])
        .min_size([700.0, 420.0])
        .collapsible(false)
        .show(ctx, |ui| p.ui(ui));
    for change in p.take_changes() {
        match change {
            Change::Navigation(s) => match state.set_navigation(s) {
                Ok(()) => {}
                Err(crate::navigation::SaveError::NotKept(why)) => p.not_kept(&why),
                Err(e) => p.revert(&state.navigation, format!("Not saved: {e}")),
            },
            Change::Display(d) => {
                if let Err(why) = state.set_reference_display(d) {
                    p.not_kept(&why);
                }
            }
            Change::Grid { style, keep: false } => state.viewport.grid_style = style,
            Change::Grid { style, keep: true } => {
                if let Err(why) = state.set_grid_style(style) {
                    p.not_kept(&why);
                }
            }
            Change::Shortcuts(s) => {
                if let Err(why) = state.set_shortcuts(s) {
                    p.not_kept(&why);
                }
            }
            Change::Autosave(policy) => {
                if let Err(why) = state.set_autosave(policy) {
                    p.not_kept(&why);
                }
            }
        }
    }
    if !open {
        *prefs = None;
    }
}
