//! What differs between the desktop and the browser: file dialogs and writing files.
//!
//! On the desktop a dialog asks for a path and files go to disk. The browser has no file
//! system: saving downloads the file under the name it would have had, and opening
//! uploads files, which arrive a frame or more later through [`take_upload`].

use std::path::{Path, PathBuf};

/// What uploaded files are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// To open: a document, or an assembly with the documents it places.
    Open,
    /// To place in the open assembly.
    Insert,
}

/// Files the browser handed over, each with its name, and what they are for.
#[derive(Debug)]
pub struct Uploaded {
    pub purpose: Purpose,
    pub files: Vec<(String, Vec<u8>)>,
}

/// Asks where to save a file. In the browser the file downloads to wherever the browser
/// puts downloads, so this returns `default_name` without asking.
pub fn save_path(title: &str, filter: &str, extension: &str, default_name: &str) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rfd::FileDialog::new().add_filter(filter, &[extension]).set_file_name(default_name).set_title(title).save_file()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (title, filter, extension);
        Some(PathBuf::from(default_name))
    }
}

/// Asks for a file with one of `extensions` to open. In the browser this starts an
/// upload and returns `None`; the file arrives later through [`take_upload`].
pub fn open_path(title: &str, filter: &str, extensions: &[&str]) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rfd::FileDialog::new().add_filter(filter, extensions).set_title(title).pick_file()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (title, filter);
        crate::web::upload(extensions, false, Purpose::Open);
        None
    }
}

/// Asks for a document to open, as [`open_path`] does. The browser takes several files
/// at once, so an assembly can be opened with the parts it places, which it has no
/// folder to find.
pub fn open_document(title: &str, filter: &str, extensions: &[&str]) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        open_path(title, filter, extensions)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (title, filter);
        crate::web::upload(extensions, true, Purpose::Open);
        None
    }
}

/// Asks for one or more files with one of `extensions`, such as the parts to insert in an
/// assembly. In the browser this starts an upload and returns `None`; the files arrive
/// later through [`take_upload`], for [`Purpose::Insert`].
pub fn open_paths(title: &str, filter: &str, extensions: &[&str]) -> Option<Vec<PathBuf>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rfd::FileDialog::new().add_filter(filter, extensions).set_title(title).pick_files()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (title, filter);
        crate::web::upload(extensions, true, Purpose::Insert);
        None
    }
}

/// Asks for a folder, for File locations. The browser has no folders to pick.
pub fn pick_folder(title: &str) -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        rfd::FileDialog::new().set_title(title).pick_folder()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = title;
        None
    }
}

/// Writes a file atomically, making its directory first, so a failed write never leaves
/// a half-written file. In the browser the file downloads instead, under the path's file
/// name.
pub fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        soliddraft_document::save_atomic(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "download".into());
        crate::web::download(&name, bytes)
    }
}

/// Writes an export: the file at `path` and the files beside it, such as a STEP file's
/// proof report. On the desktop each is a file. In the browser, where each would be its
/// own download, they arrive as one zip named after the export.
pub fn write_export(path: &Path, files: Vec<(std::path::PathBuf, Vec<u8>)>) -> Result<(), String> {
    if cfg!(target_arch = "wasm32") && files.len() > 1 {
        let entries: Vec<(String, Vec<u8>)> = files
            .into_iter()
            .map(|(file, bytes)| {
                (file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), bytes)
            })
            .collect();
        return write_file(&path.with_extension("zip"), &crate::zip::stored(&entries));
    }
    for (file, bytes) in files {
        write_file(&file, &bytes)?;
    }
    Ok(())
}

/// The files the user last uploaded, once they have arrived. Always `None` on the
/// desktop, where opening a file is immediate.
pub fn take_upload() -> Option<Uploaded> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
    #[cfg(target_arch = "wasm32")]
    {
        crate::web::take_upload()
    }
}
