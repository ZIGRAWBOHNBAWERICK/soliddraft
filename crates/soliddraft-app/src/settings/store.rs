//! Where the settings are kept.
//!
//! The desktop app keeps them in `settings.json` in its config folder. The browser app
//! keeps the same JSON text in the browser's own storage for the page's site
//! (`localStorage`), under [`BROWSER_KEY`]. That storage belongs to one browser profile
//! on one computer: the server never receives it, and another visitor, another browser
//! or a private window starts from its own empty copy.

use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The browser storage key that holds the settings text.
pub const BROWSER_KEY: &str = "soliddraft-settings";

/// Why a settings change was not kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotKept {
    /// There is nowhere to keep settings: no home folder on the desktop, or a browser that
    /// blocks site storage.
    Nowhere,
    /// The browser's storage for this site is full.
    Full,
    /// The settings file or storage refused the write, with the reason.
    Write(String),
}

impl std::fmt::Display for NotKept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NotKept::Nowhere if cfg!(target_arch = "wasm32") => {
                write!(f, "this browser does not let the page keep settings")
            }
            NotKept::Nowhere => write!(f, "SolidDraft cannot find a folder to keep its settings in"),
            NotKept::Full => write!(f, "the browser's storage for this site is full"),
            NotKept::Write(why) => write!(f, "{why}"),
        }
    }
}

/// What a change that was applied but not kept lasts for, as the window says it.
pub fn lasts() -> &'static str {
    if cfg!(target_arch = "wasm32") {
        "The change lasts until you reload or close this page."
    } else {
        "The change lasts until you close SolidDraft."
    }
}

/// A place that holds the whole settings text under one name: the browser's storage, or a
/// test's stand-in for it.
pub trait Shelf: Send + Sync {
    /// The stored text; `None` when nothing is stored yet.
    fn get(&self) -> Result<Option<String>, NotKept>;
    fn put(&self, text: &str) -> Result<(), NotKept>;
}

/// Where the settings in use live.
#[derive(Clone)]
pub enum Store {
    /// A settings file.
    File(PathBuf),
    /// A shelf: the browser's storage, or a test's.
    Shelf(Arc<dyn Shelf>),
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Store::File(path) => write!(f, "File({})", path.display()),
            Store::Shelf(shelf) => write!(f, "Shelf({:p})", Arc::as_ptr(shelf)),
        }
    }
}

impl PartialEq for Store {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Store::File(a), Store::File(b)) => a == b,
            (Store::Shelf(a), Store::Shelf(b)) => std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b)),
            _ => false,
        }
    }
}

impl Store {
    /// The settings as stored: an empty object when there are none or they cannot be read.
    pub fn load(&self) -> Value {
        let text = match self {
            Store::File(path) => std::fs::read_to_string(super::readable(path)).ok(),
            Store::Shelf(shelf) => shelf.get().ok().flatten(),
        };
        text.and_then(|t| serde_json::from_str(&t).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Default::default()))
    }

    /// Replaces the stored settings with `all`. A file is written atomically.
    pub fn save(&self, all: &Value) -> Result<(), NotKept> {
        let text = serde_json::to_string_pretty(all).map_err(|e| NotKept::Write(e.to_string()))?;
        match self {
            Store::File(path) => soliddraft_document::save_atomic(path, text.as_bytes())
                .map_err(|e| NotKept::Write(format!("cannot write {}: {e}", path.display()))),
            Store::Shelf(shelf) => shelf.put(&text),
        }
    }

    /// Whether settings were stored here before.
    pub fn exists(&self) -> bool {
        match self {
            Store::File(path) => super::readable(path).exists(),
            Store::Shelf(shelf) => shelf.get().is_ok_and(|t| t.is_some()),
        }
    }
}

static OVERRIDE: Mutex<Option<Store>> = Mutex::new(None);

/// Keeps the settings in `store` instead of the platform's place, for tests and automation.
/// `None` goes back to the platform's place.
pub fn use_store(store: Option<Store>) {
    *OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = store;
}

/// The store in use: the one set with [`use_store`], else the browser's storage in the
/// browser, else `settings.json` in the config folder.
pub fn current() -> Result<Store, NotKept> {
    if let Some(store) = OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return Ok(store);
    }
    #[cfg(target_arch = "wasm32")]
    {
        Ok(Store::Shelf(browser::BROWSER.clone()))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        super::settings_file().map(Store::File).ok_or(NotKept::Nowhere)
    }
}

/// A shelf in memory, standing in for the browser's storage in tests: one per browser.
#[derive(Default)]
pub struct Memory {
    text: Mutex<Option<String>>,
    refuse: Option<NotKept>,
}

impl Memory {
    /// An empty shelf.
    pub fn new() -> Arc<Memory> {
        Arc::new(Memory::default())
    }

    /// A shelf that reads as empty and refuses every write with `why`, as a browser with
    /// blocked or full storage does.
    pub fn refusing(why: NotKept) -> Arc<Memory> {
        Arc::new(Memory { text: Mutex::new(None), refuse: Some(why) })
    }

    /// The stored text.
    pub fn text(&self) -> Option<String> {
        self.text.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Shelf for Memory {
    fn get(&self) -> Result<Option<String>, NotKept> {
        Ok(self.text())
    }

    fn put(&self, text: &str) -> Result<(), NotKept> {
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        *self.text.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_owned());
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::{BROWSER_KEY, NotKept, Shelf};
    use std::sync::{Arc, LazyLock};

    /// The browser's `localStorage` for the page's site.
    struct Browser;

    /// The one browser store, so the store in use compares equal from call to call.
    pub(super) static BROWSER: LazyLock<Arc<dyn Shelf>> = LazyLock::new(|| Arc::new(Browser));

    fn storage() -> Result<web_sys::Storage, NotKept> {
        web_sys::window().and_then(|w| w.local_storage().ok().flatten()).ok_or(NotKept::Nowhere)
    }

    fn refused(error: wasm_bindgen::JsValue) -> NotKept {
        let field = |name: &str| js_sys::Reflect::get(&error, &name.into()).ok().and_then(|v| v.as_string());
        match field("name").as_deref() {
            Some("QuotaExceededError") => NotKept::Full,
            Some("SecurityError") => NotKept::Nowhere,
            _ => NotKept::Write(format!(
                "the browser refused to store the settings: {}",
                field("message").unwrap_or_else(|| "no reason given".into())
            )),
        }
    }

    impl Shelf for Browser {
        fn get(&self) -> Result<Option<String>, NotKept> {
            storage()?.get_item(BROWSER_KEY).map_err(refused)
        }

        fn put(&self, text: &str) -> Result<(), NotKept> {
            storage()?.set_item(BROWSER_KEY, text).map_err(refused)
        }
    }
}
