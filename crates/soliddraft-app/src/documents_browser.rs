//! The browser build's documents: autosave's recovery copies and the recent-documents
//! list, kept in this site's IndexedDB (`documents_browser.js`).
//!
//! That storage belongs to one browser profile on one computer. The server never receives
//! it, so two people using the site, two browsers or a private window never see each
//! other's files. The browser answers in the background: each call here starts the work
//! and returns, and what comes back waits in this module until a frame takes it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use soliddraft_document::DocKind;
use soliddraft_document::recovery::{self, RecoveryCopy};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/src/documents_browser.js")]
extern "C" {
    #[wasm_bindgen(js_name = start)]
    fn js_start(id: &str) -> js_sys::Promise;
    // Bytes go over as copies: the calls keep them past an await, and a view into the
    // module's memory would not survive that.
    #[wasm_bindgen(js_name = put)]
    fn js_put(file: &str, bytes: &js_sys::Uint8Array, written: f64) -> js_sys::Promise;
    #[wasm_bindgen(js_name = remove)]
    fn js_remove(file: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = adopt)]
    fn js_adopt(key: &str, file: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = discard)]
    fn js_discard(key: &str) -> js_sys::Promise;
    #[wasm_bindgen(js_name = recent)]
    fn js_recent() -> js_sys::Promise;
    #[wasm_bindgen(js_name = remember)]
    fn js_remember(key: &str, name: &str, kind: &str, when: f64, bytes: &js_sys::Uint8Array) -> js_sys::Promise;
    #[wasm_bindgen(js_name = thumbnail)]
    fn js_thumbnail(key: &str, png: &js_sys::Uint8Array) -> js_sys::Promise;
    #[wasm_bindgen(js_name = forget)]
    fn js_forget() -> js_sys::Promise;
}

/// What the browser held from earlier visits when autosave started.
pub struct Found {
    /// The copies to offer, newest first; each `file` is the copy's key.
    pub offers: Vec<RecoveryCopy>,
    /// Damaged copies, as sentences. They are deleted.
    pub problems: Vec<String>,
}

/// A document the browser keeps for the recent list.
#[derive(Clone)]
pub struct Stored {
    pub name: String,
    pub kind: DocKind,
    /// When it was last opened or saved, in milliseconds since the Unix epoch.
    pub when_ms: u64,
    pub bytes: std::sync::Arc<Vec<u8>>,
    /// A picture the browser made when it saved the document, for a file without one.
    pub thumbnail: Option<Vec<u8>>,
}

#[derive(Default)]
struct Shared {
    found: Option<Result<Found, String>>,
    /// The offered copies' bytes, by key, so a restore needs no wait.
    offered: HashMap<String, Vec<u8>>,
    failures: Vec<String>,
    recent: Option<Result<Vec<Stored>, String>>,
}

thread_local! {
    static SHARED: RefCell<Shared> = RefCell::new(Shared::default());
}

fn error(value: JsValue) -> String {
    js_sys::Reflect::get(&value, &"message".into())
        .ok()
        .and_then(|s| s.as_string())
        .or_else(|| value.as_string())
        .unwrap_or_else(|| "the browser's storage failed".into())
}

fn field(value: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(value, &name.into()).unwrap_or(JsValue::UNDEFINED)
}

fn bytes(value: &JsValue) -> Vec<u8> {
    js_sys::Uint8Array::new(value).to_vec()
}

fn now_ms() -> u64 {
    js_sys::Date::now() as u64
}

/// Runs a storage call in the background; a failure becomes a note.
fn spawn(what: &'static str, promise: js_sys::Promise) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = JsFuture::from(promise).await {
            fail(format!("{what}: {}", error(e)));
        }
        crate::web::wake();
    });
}

/// Records a failure for the app to show.
pub fn fail(sentence: String) {
    SHARED.with(|s| s.borrow_mut().failures.push(sentence));
}

/// The failures since the last call.
pub fn take_failures() -> Vec<String> {
    SHARED.with(|s| std::mem::take(&mut s.borrow_mut().failures))
}

/// Claims a new session and looks for the copies of sessions that ended.
pub fn start_recovery() {
    let id = soliddraft_document::DocumentId::new().to_string();
    // Called now, not in the task: the script takes the session id before it first
    // waits, so a copy autosave writes at once already has its place.
    let started = js_start(&id);
    wasm_bindgen_futures::spawn_local(async move {
        let found = match JsFuture::from(started).await {
            Ok(list) => Ok(read_offers(&list)),
            Err(e) => Err(error(e)),
        };
        SHARED.with(|s| s.borrow_mut().found = Some(found));
        crate::web::wake();
    });
}

fn read_offers(list: &JsValue) -> Found {
    let mut found = Found { offers: Vec::new(), problems: Vec::new() };
    for item in js_sys::Array::from(list).iter() {
        let key = field(&item, "key").as_string().unwrap_or_default();
        let copy = bytes(&field(&item, "bytes"));
        match recovery::decode(std::io::Cursor::new(&copy)) {
            Ok((record, _)) => {
                found.offers.push(RecoveryCopy { record, file: PathBuf::from(&key) });
                SHARED.with(|s| s.borrow_mut().offered.insert(key, copy));
            }
            Err(e) => {
                found.problems.push(format!("A recovery copy is damaged and was skipped ({e})"));
                spawn("Recovery", js_discard(&key));
            }
        }
    }
    found.offers.sort_by_key(|c| std::cmp::Reverse(c.record.written_ms));
    found
}

/// What the browser held when autosave started, once it has answered.
pub fn take_found() -> Option<Result<Found, String>> {
    SHARED.with(|s| s.borrow_mut().found.take())
}

/// The bytes of the offered copy `key`.
pub fn offered(key: &str) -> Option<Vec<u8>> {
    SHARED.with(|s| s.borrow().offered.get(key).cloned())
}

/// Where autosave's jobs put copies in the browser.
pub struct RecoveryShelf;

/// Keeps this session's copy `file`.
pub fn put(file: &str, copy: Vec<u8>, written_ms: u64) {
    let copy = js_sys::Uint8Array::from(copy.as_slice());
    spawn("Autosave could not keep a recovery copy", js_put(file, &copy, written_ms as f64));
}

/// Deletes this session's copy `file`.
pub fn remove(file: &str) {
    spawn("Autosave could not delete a recovery copy", js_remove(file));
}

/// Makes the offered copy `key` this session's copy `file`.
pub fn adopt(key: &str, file: &str) {
    SHARED.with(|s| s.borrow_mut().offered.remove(key));
    spawn("Recovery", js_adopt(key, file));
}

/// Deletes the offered copy `key`.
pub fn discard(key: &str) {
    SHARED.with(|s| s.borrow_mut().offered.remove(key));
    spawn("Recovery", js_discard(key));
}

/// Asks the browser for the recent documents; they arrive through [`take_recent`].
pub fn load_recent() {
    wasm_bindgen_futures::spawn_local(async move {
        let recent = match JsFuture::from(js_recent()).await {
            Ok(list) => Ok(js_sys::Array::from(&list)
                .iter()
                .filter_map(|item| {
                    let kind = DocKind::from_extension(&field(&item, "kind").as_string()?)?;
                    Some(Stored {
                        name: field(&item, "name").as_string()?,
                        kind,
                        when_ms: field(&item, "when").as_f64().unwrap_or(0.0) as u64,
                        bytes: std::sync::Arc::new(bytes(&field(&item, "bytes"))),
                        thumbnail: Some(field(&item, "thumbnail")).filter(|t| !t.is_undefined()).map(|t| bytes(&t)),
                    })
                })
                .collect()),
            Err(e) => Err(error(e)),
        };
        SHARED.with(|s| s.borrow_mut().recent = Some(recent));
        crate::web::wake();
    });
}

/// The recent documents, once the browser has answered [`load_recent`].
pub fn take_recent() -> Option<Result<Vec<Stored>, String>> {
    SHARED.with(|s| s.borrow_mut().recent.take())
}

/// The recent list's key for a document: one entry per document, whatever its name.
pub fn key(id: soliddraft_document::DocumentId, kind: DocKind) -> String {
    format!("{id}.{}", kind.extension())
}

/// Puts the document file `bytes`, called `name`, at the top of the recent list. The same
/// document saved again replaces its entry.
pub fn remember(name: &str, kind: DocKind, bytes: &[u8]) {
    let key = match soliddraft_document::identify(std::io::Cursor::new(bytes)) {
        Ok(identity) => key(identity.id, kind),
        Err(_) => name.to_owned(),
    };
    let bytes = js_sys::Uint8Array::from(bytes);
    spawn("The recent list was not kept", js_remember(&key, name, kind.extension(), now_ms() as f64, &bytes));
}

/// Gives the recent document `key` a picture: `png`, a PNG.
pub fn set_thumbnail(key: &str, png: &[u8]) {
    let png = js_sys::Uint8Array::from(png);
    spawn("The picture for the recent list was not kept", js_thumbnail(key, &png));
}

/// Empties the recent list.
pub fn forget_recent() {
    spawn("The recent list was not cleared", js_forget());
}
