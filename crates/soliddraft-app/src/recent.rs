//! The recent documents the start page lists: name, kind, location, date and thumbnail.
//!
//! On the desktop they are the files in the recent-files list (`file::recent`), read on a
//! worker so a slow disk never holds up a frame. In the browser they are the documents
//! this browser opened or saved, kept in its own storage with their bytes
//! (`documents_browser`): a page cannot open a file again by its path.

use std::path::PathBuf;
#[cfg(target_arch = "wasm32")]
use std::sync::Arc;

use soliddraft_document::DocKind;

/// Where a recent document is opened from.
#[derive(Clone, Debug)]
pub enum Source {
    /// A file on disk.
    File(PathBuf),
    /// A document file's bytes the browser keeps.
    #[cfg(target_arch = "wasm32")]
    Browser(Arc<Vec<u8>>),
}

/// One row of the recent list.
#[derive(Clone)]
pub struct RecentDoc {
    /// The file name, with its extension.
    pub name: String,
    pub kind: Option<DocKind>,
    /// The folder the file is in, or where the browser keeps it.
    pub location: String,
    /// When the file was last written or the browser last kept it, in milliseconds since
    /// the Unix epoch.
    pub when_ms: Option<u64>,
    /// The picture saved in the file, when it has one.
    pub thumbnail: Option<egui::ColorImage>,
    pub source: Source,
}

impl RecentDoc {
    /// The name without its extension, as the title bar and the tree show it.
    pub fn title(&self) -> &str {
        self.name.rsplit_once('.').map_or(self.name.as_str(), |(stem, _)| stem)
    }
}

/// The recent list being read in the background.
pub struct Loader {
    #[cfg(not(target_arch = "wasm32"))]
    answer: std::sync::mpsc::Receiver<Vec<RecentDoc>>,
}

/// Starts reading the recent list. `wake` is called when it is ready.
pub fn load(wake: impl Fn() + Send + 'static) -> Loader {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (tell, answer) = std::sync::mpsc::channel();
        let paths = crate::file::recent();
        let spawned = std::thread::Builder::new().name("recent documents".into()).spawn(move || {
            let _ = tell.send(paths.into_iter().map(|p| from_file(&p)).collect());
            wake();
        });
        if let Err(e) = spawned {
            log::warn!("cannot read the recent documents: {e}");
        }
        Loader { answer }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = wake;
        crate::documents_browser::load_recent();
        Loader {}
    }
}

impl Loader {
    /// The list, once it has been read. An error leaves the list empty and is logged.
    pub fn poll(&mut self) -> Option<Vec<RecentDoc>> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            match self.answer.try_recv() {
                Ok(docs) => Some(docs),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Vec::new()),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            crate::documents_browser::take_recent().map(|found| match found {
                Ok(stored) => stored.into_iter().map(from_browser).collect(),
                Err(e) => {
                    log::warn!("cannot read the recent documents: {e}");
                    Vec::new()
                }
            })
        }
    }
}

/// A row for a file on disk.
#[cfg(not(target_arch = "wasm32"))]
pub fn from_file(path: &std::path::Path) -> RecentDoc {
    let when_ms = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    RecentDoc {
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        kind: DocKind::of_path(path),
        location: path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
        when_ms,
        thumbnail: soliddraft_document::thumbnail(path).ok().flatten().and_then(|png| picture(&png)),
        source: Source::File(path.to_path_buf()),
    }
}

/// A row for a document the browser keeps.
#[cfg(target_arch = "wasm32")]
fn from_browser(stored: crate::documents_browser::Stored) -> RecentDoc {
    RecentDoc {
        name: stored.name,
        kind: Some(stored.kind),
        location: "This browser".to_owned(),
        when_ms: Some(stored.when_ms),
        thumbnail: soliddraft_document::thumbnail_of(&stored.bytes)
            .ok()
            .flatten()
            .or(stored.thumbnail)
            .and_then(|png| picture(&png)),
        source: Source::Browser(stored.bytes),
    }
}

/// A document kind as the start page and Recover documents name it.
pub fn kind_name(kind: Option<DocKind>) -> &'static str {
    match kind {
        Some(DocKind::Part) => "Part",
        Some(DocKind::Assembly) => "Assembly",
        Some(DocKind::Drawing) => "Drawing",
        None => "Document",
    }
}

/// A PNG as an image egui can draw. `None` when it does not decode.
pub fn picture(png: &[u8]) -> Option<egui::ColorImage> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
}

/// When something happened, as a person reads it: "Today 14:05", "Yesterday 09:12" or
/// "3 Nov 2026", in local time.
pub fn when_text(ms: u64) -> String {
    use chrono::{Local, TimeZone};
    let Some(at) = Local.timestamp_millis_opt(ms as i64).single() else { return String::new() };
    let today = Local::now().date_naive();
    let day = at.date_naive();
    if day == today {
        format!("Today {}", at.format("%H:%M"))
    } else if today.pred_opt() == Some(day) {
        format!("Yesterday {}", at.format("%H:%M"))
    } else {
        at.format("%-d %b %Y").to_string()
    }
}
