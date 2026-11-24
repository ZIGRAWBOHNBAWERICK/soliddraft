//! The app's settings: the assistant's provider and model, whether the side area is
//! shown and how wide, the command line's chips and session height, the pinned and recent
//! commands, File locations: the folders and
//! files where reference files are found (`soliddraft_document::locations`), the
//! viewport's default look, [`ReferenceDisplay`] and the grid's [`GridStyle`], the
//! interface's light or dark [`Appearance`], and autosave,
//! recovery and backups ([`DocumentSettings`]), and navigation: the mouse preset, the
//! wheel, the view keys and the camera ([`NavigationSettings`]). The user's own
//! material library sits beside them ([`user_library_path`]).
//!
//! The desktop keeps them in `settings.json` in the config directory next to the
//! recent-files list; the browser keeps the same text in its own storage for the site
//! ([`store`]).
//!
//! Saving rewrites only the sections the app knows and keeps every other key, so
//! settings written by a newer build survive an older one.

pub mod store;

use crate::command_line::BarSettings;
use crate::commands::{History, Shortcuts};
use crate::navigation::SaveError;
use crate::panels::Layout;
pub use crate::viewport::grid::GridStyle;
use crate::viewport::look::Preset;
use crate::viewport::scenes::Scene;
use serde_json::Value;
use soliddraft_assistant::AssistantSettings;
use soliddraft_document::{AutosavePolicy, Backups, FileLocations};
use soliddraft_materials::{Catalog, Library, Origin};
use std::path::{Path, PathBuf};
pub use store::{NotKept, Store};

pub(crate) fn settings_file() -> Option<PathBuf> {
    crate::file::config_dir().map(|d| d.join("settings.json"))
}

/// The settings as stored now: an empty object when there are none, they cannot be read,
/// or there is nowhere to keep them.
fn stored() -> Value {
    store::current().map(|s| s.load()).unwrap_or_else(|_| Value::Object(Default::default()))
}

/// The assistant settings, or the defaults when there are none or they are unreadable.
pub fn assistant() -> AssistantSettings {
    assistant_from(&stored())
}

/// Saves the assistant settings.
pub fn save_assistant(settings: &AssistantSettings) -> Result<(), NotKept> {
    save_section("assistant", settings)
}

fn assistant_from(all: &Value) -> AssistantSettings {
    serde_json::from_value(all["assistant"].clone()).unwrap_or_default()
}

/// The panel layout, or the default when there is none or it is unreadable.
pub fn layout() -> Layout {
    layout_from(&stored())
}

/// Saves the panel layout.
pub fn save_layout(layout: &Layout) -> Result<(), NotKept> {
    save_section("layout", layout)
}

fn layout_from(all: &Value) -> Layout {
    let mut layout = serde_json::from_value::<Layout>(all["layout"].clone()).unwrap_or_default().clamped();
    // Classic serialized inactive Suite flags too; keep its active tree choice until a layout save.
    if (all["frame_layout"] == "classic" || all["layout"]["suite"].is_null())
        && let Some(shown) = all["layout"]["tree"]["shown"].as_bool()
    {
        layout.suite.part = shown;
    }
    layout
}

/// The panel layout in the settings file at `file`.
#[cfg(test)]
pub(crate) fn layout_in(file: &Path) -> Layout {
    layout_from(&Store::File(file.to_owned()).load())
}

/// Saves the panel layout into the settings file at `file`, keeping its other keys.
#[cfg(test)]
pub(crate) fn save_layout_in(file: &Path, layout: &Layout) -> Result<(), NotKept> {
    save_section_to(&Store::File(file.to_owned()), "layout", layout)
}

/// The command line's chips and session height, or the defaults when there are none or
/// they are unreadable. Ids no command has any more are dropped.
pub fn command_line() -> BarSettings {
    command_line_from(&stored())
}

/// Saves the command line's chips and session height.
pub fn save_command_line(bar: &BarSettings) -> Result<(), NotKept> {
    save_section("command_line", bar)
}

fn command_line_from(all: &Value) -> BarSettings {
    serde_json::from_value::<BarSettings>(all["command_line"].clone()).unwrap_or_default().cleaned()
}

/// The pinned and recent commands, or none when there are none or they are unreadable.
/// Ids no command has any more are dropped.
pub fn command_history() -> History {
    command_history_from(&stored())
}

/// Saves the pinned and recent commands.
pub fn save_command_history(history: &History) -> Result<(), NotKept> {
    save_section("commands", history)
}

/// Saved Hole Wizard favorite presets, or empty when none are stored.
pub fn hole_favorites() -> Vec<soliddraft_model::hole_wizard::HoleFavorite> {
    serde_json::from_value(stored()["hole_favorites"].clone()).unwrap_or_default()
}

/// Persists the Hole Wizard favorite presets to desktop settings or browser storage.
pub fn save_hole_favorites(favorites: &[soliddraft_model::hole_wizard::HoleFavorite]) -> Result<(), NotKept> {
    save_section("hole_favorites", &favorites.to_vec())
}

fn command_history_from(all: &Value) -> History {
    let mut history: History = serde_json::from_value(all["commands"].clone()).unwrap_or_default();
    history.retain_known();
    history
}

/// The create commands pinned from the Create menu, or every one when there are none
/// or they are unreadable. Ids the menu no longer lists are dropped.
pub fn create_pins() -> crate::create_menu::Pins {
    create_pins_from(&stored())
}

/// Saves the create commands pinned from the Create menu.
pub fn save_create_pins(pins: &crate::create_menu::Pins) -> Result<(), NotKept> {
    save_section("create_menu", pins)
}

fn create_pins_from(all: &Value) -> crate::create_menu::Pins {
    if all["create_menu"].is_null() {
        return Default::default();
    }
    serde_json::from_value::<crate::create_menu::Pins>(all["create_menu"].clone()).unwrap_or_default().retain_known()
}

/// The user's shortcut preset and overrides, or the defaults when there are none or
/// they are unreadable.
pub fn shortcuts() -> Shortcuts {
    shortcuts_from(&stored())
}

/// Saves the user's shortcut preset and overrides.
pub fn save_shortcuts(shortcuts: &Shortcuts) -> Result<(), NotKept> {
    save_section("shortcuts", shortcuts)
}

fn shortcuts_from(all: &Value) -> Shortcuts {
    Shortcuts::from_json(&all["shortcuts"])
}

pub(crate) fn shortcut_menu() -> crate::shortcut_menu::Preferences {
    serde_json::from_value(stored()["shortcut_menu"].clone()).unwrap_or_default()
}

pub(crate) fn save_shortcut_menu(menu: &crate::shortcut_menu::Preferences) -> Result<(), NotKept> {
    save_section("shortcut_menu", menu)
}

pub(crate) fn export_preferences(menu: &crate::shortcut_menu::Preferences) -> Result<(), String> {
    let Some(path) = crate::platform::save_path("Export preferences", "JSON", "json", "soliddraft-preferences.json")
    else {
        return Ok(());
    };
    crate::platform::write_file(&path, &shortcut_menu_export(menu)?)
}

/// The file Export preferences writes: the shortcut menu's settings and nothing else.
/// Other settings hold private paths, history and provider choices the user did not ask
/// to export.
pub(crate) fn shortcut_menu_export(menu: &crate::shortcut_menu::Preferences) -> Result<Vec<u8>, String> {
    let menu = serde_json::to_value(menu).map_err(|error| error.to_string())?;
    serde_json::to_vec_pretty(&serde_json::json!({ "shortcut_menu": menu })).map_err(|error| error.to_string())
}

/// File locations, or none when there are none or they are unreadable.
pub fn file_locations() -> FileLocations {
    file_locations_from(&stored())
}

/// Saves File locations.
pub fn save_file_locations(locations: &FileLocations) -> Result<(), NotKept> {
    save_section("file_locations", locations)
}

fn file_locations_from(all: &Value) -> FileLocations {
    serde_json::from_value(all["file_locations"].clone()).unwrap_or_default()
}

/// File locations from the settings file at `file`.
pub fn file_locations_in(file: &Path) -> FileLocations {
    file_locations_from(&Store::File(file.to_owned()).load())
}

/// Saves File locations into the settings file at `file`, keeping its other keys.
pub fn save_file_locations_in(file: &Path, locations: &FileLocations) -> Result<(), NotKept> {
    save_section_to(&Store::File(file.to_owned()), "file_locations", locations)
}

/// The user's own material and appearance library, "My library":
/// `libraries/my-library.toml` in the config directory. It may not exist yet; saving
/// it creates it.
pub fn user_library_path() -> Option<PathBuf> {
    crate::file::config_dir().map(|d| d.join("libraries").join("my-library.toml"))
}

/// Every material and appearance library: the built-in ones, the user's own library
/// and those File locations lists.
#[cfg(not(target_arch = "wasm32"))]
pub fn material_catalog() -> Catalog {
    Catalog::load(crate::file::config_read_file("libraries/my-library.toml").as_deref(), &file_locations())
}

/// The user's own library, loaded, or a new empty one at its path when it does not
/// exist yet.
#[cfg(not(target_arch = "wasm32"))]
pub fn user_library() -> Result<Library, String> {
    let path = user_library_path().ok_or("SolidDraft cannot find a folder to keep your library in")?;
    if let Some(source) = crate::file::config_read_file("libraries/my-library.toml")
        && source.exists()
    {
        let mut lib = Library::load(&source, Origin::User).map_err(|e| e.to_string())?;
        lib.path = Some(path);
        return Ok(lib);
    }
    let mut lib = Library::new("My library", Origin::User);
    lib.path = Some(path);
    Ok(lib)
}

/// The built-in catalog and the browser profile's custom library.
#[cfg(target_arch = "wasm32")]
pub fn material_catalog() -> Catalog {
    let mut libraries = Catalog::built_in_libraries();
    libraries.extend(user_library().ok());
    Catalog::from_libraries(libraries, Vec::new())
}

/// Loads the custom library saved in this browser profile.
#[cfg(target_arch = "wasm32")]
pub fn user_library() -> Result<Library, String> {
    match stored()["material_library"].as_str() {
        Some(text) => Library::parse(text, Origin::User, None).map_err(|e| e.to_string()),
        None => Ok(Library::new("My library", Origin::User)),
    }
}

/// Saves the user library in the platform's settings store.
pub fn save_material_library(library: &Library) -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    return save_section("material_library", &library.to_toml()).map_err(|e| e.to_string());
    #[cfg(not(target_arch = "wasm32"))]
    library.save().map_err(|e| e.to_string())
}

/// What the viewport shows of the part's reference geometry without being asked. Display
/// choices of the user, not of the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ReferenceDisplay {
    /// The modelling grid, and the screen grid of a scene that has one.
    pub show_grid: bool,
    /// The floor under the part: the shadow the part casts on the ground.
    pub show_floor: bool,
    pub show_origin_planes: bool,
    /// Keeps the origin axes on with nothing selected. A selected reference shows them
    /// either way.
    pub always_show_origin_axes: bool,
}

impl Default for ReferenceDisplay {
    fn default() -> Self {
        Self { show_grid: true, show_floor: true, show_origin_planes: true, always_show_origin_axes: true }
    }
}

/// Viewport settings: the look the viewport starts in, the reference geometry it shows
/// and how the grid looks. Each key of the `view` section falls back on its own, so a
/// value this build cannot read costs only that value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewSettings {
    pub preset: Preset,
    pub scene: Scene,
    pub references: ReferenceDisplay,
    pub grid: GridStyle,
}

/// The viewport settings, or the defaults when there are none or they are unreadable.
pub fn view() -> ViewSettings {
    view_from(&stored())
}

/// The look the viewport starts in, Studio when the settings do not say.
pub fn default_preset() -> Preset {
    view().preset
}

/// Saves the look the viewport starts in.
pub fn save_default_preset(preset: Preset) -> Result<(), NotKept> {
    save_view_keys(serde_json::json!({ "preset": preset }))
}

/// Saves what the viewport shows of the reference geometry.
pub fn save_reference_display(references: &ReferenceDisplay) -> Result<(), NotKept> {
    save_view_keys(references)
}

/// Saves the named background without changing the other view settings.
pub fn save_scene(scene: Scene) -> Result<(), NotKept> {
    save_view_keys(serde_json::json!({ "scene": scene }))
}

/// The scene gallery favourites, with the approved first five on a fresh installation.
pub fn scene_favourites() -> Vec<Scene> {
    let saved = serde_json::from_value::<Vec<Scene>>(stored()["view"]["scene_favourites"].clone());
    let chosen = saved.unwrap_or_else(|_| Scene::ALL[..5].to_vec());
    Scene::ALL.into_iter().filter(|scene| chosen.contains(scene)).collect()
}

/// Saves favourites without replacing other viewport preferences.
pub fn save_scene_favourites(scenes: &[Scene]) -> Result<(), NotKept> {
    save_view_keys(serde_json::json!({ "scene_favourites": scenes }))
}

/// Saves how the grid looks.
pub fn save_grid_style(style: &GridStyle) -> Result<(), NotKept> {
    save_view_keys(serde_json::json!({ "grid": style.to_json() }))
}

fn view_from(all: &Value) -> ViewSettings {
    let view = &all["view"];
    let shown = ReferenceDisplay::default();
    let flag = |key: &str, default| view[key].as_bool().unwrap_or(default);
    ViewSettings {
        preset: serde_json::from_value(view["preset"].clone()).unwrap_or_default(),
        scene: serde_json::from_value(view["scene"].clone()).unwrap_or_default(),
        references: ReferenceDisplay {
            show_grid: flag("show_grid", shown.show_grid),
            show_floor: flag("show_floor", shown.show_floor),
            show_origin_planes: flag("show_origin_planes", shown.show_origin_planes),
            always_show_origin_axes: flag("always_show_origin_axes", shown.always_show_origin_axes),
        },
        grid: GridStyle::from_json(&view["grid"]),
    }
}

/// Rewrites `keys` of the `view` section and keeps the section's other keys, so saving the
/// look cannot drop a display choice, or a key a newer build wrote.
fn save_view_keys(keys: impl serde::Serialize) -> Result<(), NotKept> {
    save_view_keys_to(&store::current()?, keys)
}

fn save_view_keys_to(store: &Store, keys: impl serde::Serialize) -> Result<(), NotKept> {
    let mut view = match store.load().get("view") {
        Some(Value::Object(section)) => section.clone(),
        _ => Default::default(),
    };
    if let Value::Object(keys) = serde_json::to_value(keys).map_err(|e| NotKept::Write(e.to_string()))? {
        view.extend(keys);
    }
    save_section_to(store, "view", &view)
}

/// The format the `graphics` record is saved in.
const GRAPHICS_FORMAT: &str = "soliddraft-graphics/1";

/// The graphics settings, or the defaults when there are none or this build cannot read
/// them.
pub fn graphics() -> crate::graphics::Graphics {
    graphics_from(&stored())
}

fn graphics_from(all: &Value) -> crate::graphics::Graphics {
    let record = &all["graphics"];
    let readable = match record.get("format") {
        Some(format) => format == GRAPHICS_FORMAT,
        // A record saved before the format was named holds the same fields.
        None => record.is_object(),
    };
    if readable { serde_json::from_value(record.clone()).unwrap_or_default() } else { Default::default() }
}

/// Saves the graphics settings, with their format.
pub fn save_graphics(graphics: &crate::graphics::Graphics) -> Result<(), NotKept> {
    let mut record = serde_json::to_value(graphics).map_err(|e| NotKept::Write(e.to_string()))?;
    record["format"] = Value::String(GRAPHICS_FORMAT.into());
    save_section("graphics", &record)
}

pub fn theme_preferences() -> crate::theme::Appearance {
    theme_preferences_from(&stored())
}

fn theme_preferences_from(all: &Value) -> crate::theme::Appearance {
    use crate::theme::{Appearance, Background, BuiltIn, Selection};
    if let Ok(mut appearance) = serde_json::from_value::<Appearance>(all["theme_preferences"].clone()) {
        appearance.restore_resources();
        return appearance;
    }
    let mut appearance = Appearance::default();
    appearance.selected = Selection::BuiltIn(BuiltIn::for_mode(appearance_from(all).theme));
    if all["view"]["scene"].is_string() {
        appearance.background = Background::Scene;
    }
    appearance
}

pub fn save_theme_preferences(appearance: &crate::theme::Appearance) -> Result<(), NotKept> {
    save_appearance_and_view(appearance, None)
}

pub(crate) fn save_appearance_and_view(
    appearance: &crate::theme::Appearance,
    scene: Option<Scene>,
) -> Result<(), NotKept> {
    let store = store::current()?;
    let mut all = store.load();
    let value = |e: serde_json::Error| NotKept::Write(e.to_string());
    all["theme_preferences"] = serde_json::to_value(appearance).map_err(value)?;
    let theme = if appearance.tokens(None).is_dark { crate::theme::Mode::Dark } else { crate::theme::Mode::Light };
    if !all["appearance"].is_object() {
        all["appearance"] = serde_json::json!({});
    }
    all["appearance"]["theme"] = serde_json::to_value(theme).map_err(value)?;
    if let Some(scene) = scene {
        if !all["view"].is_object() {
            all["view"] = serde_json::json!({});
        }
        all["view"]["scene"] = serde_json::to_value(scene).map_err(value)?;
    }
    record_install_preset(&mut all);
    store.save(&all)
}

/// Appearance: light or dark interface. The `appearance` key of the settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: crate::theme::Mode,
}

/// The appearance, light when the settings do not say.
pub fn appearance() -> Appearance {
    appearance_from(&stored())
}

/// Saves the appearance.
pub fn save_appearance(appearance: &Appearance) -> Result<(), NotKept> {
    save_section("appearance", appearance)
}

fn appearance_from(all: &Value) -> Appearance {
    serde_json::from_value(all["appearance"].clone()).unwrap_or_default()
}

/// How documents are protected: autosave's interval and edit count, where recovery
/// copies go, and how many backups a save keeps and where. The `documents` key of the
/// settings.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct DocumentSettings {
    pub autosave: AutosavePolicy,
    /// Where recovery copies go. `None` is `recovery` in the config directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_folder: Option<PathBuf>,
    pub backups: Backups,
}

impl DocumentSettings {
    /// The recovery folder: the one set, or `recovery` in the config directory.
    pub fn recovery_folder(&self) -> Option<PathBuf> {
        self.recovery_folder.clone().or_else(|| crate::file::config_dir().map(|d| d.join("recovery")))
    }
}

/// The document settings, or the defaults when there are none or they are unreadable.
pub fn documents() -> DocumentSettings {
    documents_from(&stored())
}

/// Saves the document settings.
pub fn save_documents(documents: &DocumentSettings) -> Result<(), NotKept> {
    save_section("documents", documents)
}

fn documents_from(all: &Value) -> DocumentSettings {
    serde_json::from_value(all["documents"].clone()).unwrap_or_default()
}

pub use crate::navigation::NavigationSettings;

/// Whether this installation had been used before this start. A new one starts on the
/// Fusion navigation preset; an existing one keeps SolidDraft's original controls until
/// the user picks another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallKind {
    New,
    Existing,
}

impl InstallKind {
    /// Existing when settings were stored before, or, for a settings file, when the
    /// config directory holds a recent-files list, this version's or the old name's.
    pub fn detect() -> Self {
        let used = match store::current() {
            Ok(store @ Store::File(_)) => {
                store.exists() || crate::file::config_read_file("recent.json").is_some_and(|f| f.exists())
            }
            Ok(store) => store.exists(),
            Err(_) => false,
        };
        if used { InstallKind::Existing } else { InstallKind::New }
    }
}

/// The kind captured at startup and the store it was captured for.
static INSTALL: std::sync::Mutex<Option<(Option<Store>, InstallKind)>> = std::sync::Mutex::new(None);

/// The kind captured for the store in use; none when nothing captured it or the store has
/// changed since.
fn captured_install_kind() -> Option<InstallKind> {
    let current = store::current().ok();
    INSTALL.lock().unwrap().as_ref().filter(|(s, _)| *s == current).map(|(_, kind)| *kind)
}

/// Decides at startup, before anything writes the settings, whether this is a new
/// installation. From then on every save of the settings also records the preset that
/// decision gave, so writing some other setting first cannot make a new installation look
/// like an old one at the next start.
pub fn capture_install_kind() -> InstallKind {
    let kind = InstallKind::detect();
    *INSTALL.lock().unwrap() = Some((store::current().ok(), kind));
    kind
}

/// The kind captured at startup for the store in use, or found now when nothing captured
/// it there.
pub fn install_kind() -> InstallKind {
    captured_install_kind().unwrap_or_else(InstallKind::detect)
}

/// The navigation settings, or the defaults for `install` when there are none.
/// Reading never writes the settings.
pub fn navigation(install: InstallKind) -> NavigationSettings {
    NavigationSettings::from_json(&stored()["navigation"], install)
}

/// Saves the navigation settings. Refused when they, or the stored ones, come from a
/// newer SolidDraft.
pub fn save_navigation(settings: &NavigationSettings) -> Result<(), SaveError> {
    save_navigation_to(&store::current().map_err(SaveError::NotKept)?, settings)
}

fn save_navigation_to(store: &Store, settings: &NavigationSettings) -> Result<(), SaveError> {
    if settings.read_only {
        return Err(SaveError::NewerVersion { found: settings.version });
    }
    let stored = store.load()["navigation"]["version"].as_u64().unwrap_or(0);
    if stored > crate::navigation::VERSION {
        return Err(SaveError::NewerVersion { found: stored });
    }
    save_section_to(store, "navigation", &settings.to_json()).map_err(SaveError::NotKept)
}

/// Records the captured installation's preset in settings that have none yet.
fn record_install_preset(all: &mut Value) {
    let Some(kind) = captured_install_kind() else { return };
    let preset = crate::navigation::default_preset(kind);
    match all["navigation"].as_object_mut() {
        None => all["navigation"] = NavigationSettings::new(kind).to_json(),
        Some(nav) => {
            let newer = nav.get("version").and_then(Value::as_u64).unwrap_or(0) > crate::navigation::VERSION;
            if !newer && !nav.get("preset_id").is_some_and(Value::is_string) {
                nav.insert("preset_id".into(), preset.into());
            }
        }
    }
}

/// The file to read `file`'s current contents from: the old name's when this is the
/// settings file and only the old one exists.
pub(crate) fn readable(file: &Path) -> PathBuf {
    if settings_file().as_deref() == Some(file) {
        crate::file::config_read_file("settings.json").unwrap_or_else(|| file.to_owned())
    } else {
        file.to_owned()
    }
}

/// Rewrites one section of the settings in use and keeps the others as they are.
fn save_section(key: &str, value: &impl serde::Serialize) -> Result<(), NotKept> {
    save_section_to(&store::current()?, key, value)
}

/// Rewrites one section of `store` and keeps the others as they are. A file is written
/// atomically.
fn save_section_to(store: &Store, key: &str, value: &impl serde::Serialize) -> Result<(), NotKept> {
    let mut all = store.load();
    all[key] = serde_json::to_value(value).map_err(|e| NotKept::Write(e.to_string()))?;
    if key == "layout" && all["frame_layout"] == "classic" {
        all["frame_layout"] = Value::String("edit_suite".into());
    }
    if key != "navigation" {
        record_install_preset(&mut all);
    }
    store.save(&all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_assistant::ProviderKind;

    fn file_store(file: &Path) -> Store {
        Store::File(file.to_owned())
    }

    /// Export preferences in the shortcut menu writes the shortcut menu's settings only:
    /// never the folders, history, favorites, provider or library kept beside them.
    #[test]
    fn shortcut_export_holds_only_the_shortcut_menu() {
        let shelf = store::Memory::new();
        store::use_store(Some(Store::Shelf(shelf)));
        let mut locations = FileLocations::default();
        locations.add(soliddraft_document::LocationKind::ReferencedDocuments, PathBuf::from("/home/private/projects"));
        save_file_locations(&locations).unwrap();
        let mut history = History::default();
        history.record("extrude");
        save_command_history(&history).unwrap();
        save_assistant(&AssistantSettings::default()).unwrap();
        let exported = shortcut_menu_export(&crate::shortcut_menu::Preferences::default());
        store::use_store(None);
        let exported: Value = serde_json::from_slice(&exported.unwrap()).unwrap();
        let keys: Vec<&String> = exported.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["shortcut_menu"], "the export holds other settings: {exported}");
        assert!(!exported.to_string().contains("/home/private/projects"));
    }

    #[test]
    fn scenes_fall_back_without_costing_other_view_settings() {
        for scene in [serde_json::json!(null), serde_json::json!("future"), serde_json::json!(7)] {
            let view = view_from(&serde_json::json!({"view": {"scene": scene, "preset": "fast", "show_grid": false}}));
            assert_eq!(view.scene, Scene::White);
            assert_eq!(view.preset, Preset::Fast);
            assert!(!view.references.show_grid);
        }
        for scene in Scene::ALL {
            let view = view_from(&serde_json::json!({"view": {"scene": scene}}));
            assert_eq!(view.scene, scene);
        }
    }

    #[test]
    fn legacy_frame_settings_keep_panel_and_view_choices() {
        for frame in [
            serde_json::json!("classic"),
            serde_json::json!("edit_suite"),
            serde_json::json!("future"),
            serde_json::json!(null),
        ] {
            let all = serde_json::json!({
                "frame_layout": frame,
                "layout": {"tree": {"shown": true, "width": 300.0}},
                "view": {"scene": "graphite", "show_grid": false, "always_show_origin_axes": false},
                "commands": {"pinned": ["extrude", "sweep"]},
                "future": {"keep": 7}
            });
            let layout = layout_from(&all);
            assert!(layout.suite.part);
            assert_eq!(layout.tree.width, 300.0);
            let view = view_from(&all);
            assert_eq!(view.scene, Scene::Graphite);
            assert!(!view.references.show_grid);
            assert!(!view.references.always_show_origin_axes);
            assert_eq!(command_history_from(&all).pinned, ["extrude", "sweep"]);
        }
        assert!(layout_from(&serde_json::json!({})).suite.part, "a new install shows the part's tree");
        assert!(
            !layout_from(&serde_json::json!({"frame_layout": "classic", "layout": {"tree": {"shown": false}}}))
                .suite
                .part,
            "a hidden Classic tree stays hidden"
        );
        assert!(view_from(&serde_json::json!({"frame_layout": "classic"})).references.always_show_origin_axes);
        assert!(
            !layout_from(&serde_json::json!({
                "layout": {"tree": {"shown": true, "width": 300.0}, "suite": {"part": false}}
            }))
            .suite
            .part,
            "an explicit pane choice wins over the legacy tree setting"
        );
    }

    #[test]
    fn create_pins_persist_beside_other_keys() {
        let dir = std::env::temp_dir().join(format!("soliddraft-create-pins-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        std::fs::write(&file, r#"{"theme":"dark"}"#).unwrap();
        let store = file_store(&file);
        let mut pins = create_pins_from(&store.load());
        assert_eq!(pins, crate::create_menu::Pins::default(), "no pins saved: every command is pinned");
        pins.toggle("sweep");
        pins.toggle("loft");
        save_section_to(&store, "create_menu", &pins).unwrap();
        let back = create_pins_from(&store.load());
        assert_eq!(back, pins);
        assert!(!back.is_pinned("sweep") && !back.is_pinned("loft") && back.is_pinned("extrude"));
        let all: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(all["theme"], "dark", "the other keys survive");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn command_history_persists_beside_other_keys() {
        let dir = std::env::temp_dir().join(format!("soliddraft-commands-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        std::fs::write(&file, r#"{"theme":"dark","commands":{"pinned":["fillet","not_a_command"]}}"#).unwrap();
        let store = file_store(&file);
        let mut h = command_history_from(&store.load());
        assert_eq!(h.pinned, ["fillet"]);
        assert!(h.recent.is_empty());
        h.record("extrude");
        save_section_to(&store, "commands", &h).unwrap();
        assert_eq!(command_history_from(&store.load()), h);
        let all: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(all["theme"], "dark");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_command_line_persists_beside_other_keys() {
        let dir = std::env::temp_dir().join(format!("soliddraft-command-line-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        std::fs::write(&file, r#"{"theme":"dark"}"#).unwrap();
        let store = file_store(&file);
        assert_eq!(command_line_from(&store.load()), BarSettings::default());
        let mut bar = BarSettings::default();
        bar.place("fillet", Some(0));
        bar.session_height = 410.0;
        save_section_to(&store, "command_line", &bar).unwrap();
        assert_eq!(command_line_from(&store.load()), bar);
        std::fs::write(&file, r#"{"command_line":{"tools":["gone","fillet"],"session_height":5}}"#).unwrap();
        let back = command_line_from(&store.load());
        assert_eq!(back.tools, ["fillet"]);
        assert_eq!(back.session_height, crate::command_line::SESSION_MIN_HEIGHT);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn assistant_settings_persist_beside_other_keys() {
        let dir = std::env::temp_dir().join(format!("soliddraft-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        let store = file_store(&file);
        assert_eq!(assistant_from(&store.load()), AssistantSettings::default());
        std::fs::write(&file, r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(assistant_from(&store.load()), AssistantSettings::default());
        let mut s = AssistantSettings { provider: Some(ProviderKind::OpenRouter), ..Default::default() };
        s.set_model(ProviderKind::OpenRouter, "anthropic/claude-fable-5.1");
        save_section_to(&store, "assistant", &s).unwrap();
        assert_eq!(assistant_from(&store.load()), s);
        let all: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(all["theme"], "dark");
        assert_eq!(all["assistant"]["provider"], "openrouter");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_default_look_is_studio_until_saved() {
        let dir = std::env::temp_dir().join(format!("soliddraft-view-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        let store = file_store(&file);
        assert_eq!(view_from(&store.load()).preset, Preset::Studio);
        std::fs::write(&file, r#"{"view":{"preset":"cinema"},"theme":"dark"}"#).unwrap();
        assert_eq!(view_from(&store.load()).preset, Preset::Studio, "an unknown look falls back to the default");
        save_view_keys_to(&store, serde_json::json!({ "preset": Preset::Realistic })).unwrap();
        assert_eq!(view_from(&store.load()).preset, Preset::Realistic);
        let all: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(all["view"]["preset"], "realistic");
        assert_eq!(all["theme"], "dark");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn appearance_reads_the_old_mode_and_restores_a_theme_and_picture() {
        use crate::theme::{Background, BuiltIn, Rgb, Selection, ThemeFile};
        let old = serde_json::json!({"appearance": {"theme": "dark"}, "view": {"scene": "night"}});
        let mut appearance = theme_preferences_from(&old);
        assert_eq!(appearance.selected, Selection::BuiltIn(BuiltIn::ClearBenchDark));
        assert_eq!(appearance.background, Background::Scene);
        assert_eq!(theme_preferences_from(&serde_json::json!({})).background, Background::Gradient);

        let imported =
            br#"{"format":"soliddraft-theme/1","name":"Night work","base":"clear_bench_dark","accent":[255,198,48]}"#;
        let file = ThemeFile::read(imported).unwrap();
        appearance.imported = Some(file);
        appearance.selected = Selection::Imported;
        appearance.colors.logo = Some(Rgb([220, 30, 96]));
        let mut png = std::io::Cursor::new(Vec::new());
        let source = image::RgbaImage::from_pixel(12, 10, image::Rgba([32, 80, 92, 255]));
        source.write_to(&mut png, image::ImageFormat::Png).unwrap();
        appearance.set_picture("teal.png".into(), png.get_ref()).unwrap();

        let json = serde_json::to_value(&appearance).unwrap();
        assert_eq!(json["format"], "soliddraft-appearance/1");
        let restored = theme_preferences_from(&serde_json::json!({"theme_preferences": json}));
        assert_eq!(restored, appearance);
        assert_eq!(restored.picture().as_deref(), Some(&source));
        assert_eq!(restored.picture_name(), Some("teal.png"));
        assert_eq!(restored.tokens(None).palette.accent, egui::Color32::from_rgb(255, 198, 48));

        appearance.selected = Selection::System;
        assert!(appearance.tokens(Some(egui::Theme::Dark)).is_dark);
        assert!(!appearance.tokens(Some(egui::Theme::Light)).is_dark);
        assert!(!appearance.tokens(None).is_dark);
    }

    #[test]
    fn imported_theme_refuses_wrong_version_and_unreadable_colours() {
        use crate::theme::ThemeFile;
        for invalid in [
            br#"{"format":"soliddraft-theme/2","name":"Wrong version"}"#.as_slice(),
            br#"{"format":"soliddraft-theme/1","name":" ","ink":[255,255,255]}"#,
            br#"{"format":"soliddraft-theme/1","name":"White ink","ink":[255,255,255]}"#,
            br#"{"format":"soliddraft-theme/1","name":"Invalid number","accent":[500,0,0]}"#,
        ] {
            assert!(ThemeFile::read(invalid).is_err());
        }
    }

    #[test]
    fn the_appearance_is_light_until_saved() {
        use crate::theme::Mode;
        let dir = std::env::temp_dir().join(format!("soliddraft-appearance-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        let store = file_store(&file);
        assert_eq!(appearance_from(&store.load()).theme, Mode::Light);
        std::fs::write(&file, r#"{"appearance":{"theme":"sepia"},"theme":"dark"}"#).unwrap();
        assert_eq!(appearance_from(&store.load()).theme, Mode::Light, "an unknown theme falls back to light");
        save_section_to(&store, "appearance", &Appearance { theme: Mode::Dark }).unwrap();
        assert_eq!(appearance_from(&store.load()).theme, Mode::Dark);
        let all: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(all["appearance"]["theme"], "dark");
        assert_eq!(all["theme"], "dark", "a foreign key is kept");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn document_settings_default_and_persist() {
        let dir = std::env::temp_dir().join(format!("soliddraft-documents-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        std::fs::write(&file, r#"{"documents":{"autosave":{"interval_seconds":30}},"theme":"dark"}"#).unwrap();
        let store = file_store(&file);
        let mut d = documents_from(&store.load());
        assert_eq!(d.autosave, AutosavePolicy { interval_seconds: 30, ..Default::default() }, "missing keys default");
        assert_eq!(d.backups, Backups::default());
        d.backups = Backups { count: 3, folder: Some(dir.join("backups")) };
        d.recovery_folder = Some(dir.join("recovery"));
        save_section_to(&store, "documents", &d).unwrap();
        assert_eq!(documents_from(&store.load()), d);
        assert_eq!(d.recovery_folder(), Some(dir.join("recovery")));
        let all: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(all["theme"], "dark");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn file_locations_persist_beside_other_keys() {
        use soliddraft_document::LocationKind;
        let dir = std::env::temp_dir().join(format!("soliddraft-locations-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        std::fs::write(&file, r#"{"assistant":{"provider":"openrouter"}}"#).unwrap();
        assert_eq!(file_locations_in(&file), FileLocations::new());
        let mut f = FileLocations::new();
        f.add(LocationKind::DrawingTemplates, "/shared/templates");
        f.add(LocationKind::ReferencedDocuments, "/shared/parts");
        save_file_locations_in(&file, &f).unwrap();
        assert_eq!(file_locations_in(&file), f);
        assert_eq!(
            assistant_from(&file_store(&file).load()).provider,
            Some(ProviderKind::OpenRouter),
            "the assistant section is kept"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn shortcuts_persist_beside_other_keys() {
        use crate::commands::{KeyChord, ShortcutPreset};
        let dir = std::env::temp_dir().join(format!("soliddraft-shortcuts-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        std::fs::write(&file, r#"{"theme":"dark"}"#).unwrap();
        let store = file_store(&file);
        assert_eq!(shortcuts_from(&store.load()), Shortcuts::default());
        let mut s = Shortcuts::for_preset(ShortcutPreset::Solidworks);
        s.assign("revolve", KeyChord::plain(egui::Key::V)).unwrap();
        s.clear("chamfer");
        save_section_to(&store, "shortcuts", &s).unwrap();
        assert_eq!(shortcuts_from(&store.load()), s);
        let all: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(all["theme"], "dark");
        let _ = std::fs::remove_dir_all(dir);
    }
}
