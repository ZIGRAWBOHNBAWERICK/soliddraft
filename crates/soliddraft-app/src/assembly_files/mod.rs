//! The files an open assembly writes, in the window and over MCP: Pack and Go (the
//! assembly with every document it references, in a new folder), and URDF and OpenUSD
//! with joints. What each writes is `soliddraft-assembly`'s and `soliddraft-scene`'s (see
//! `docs/assemblies.md`); this module holds the commands and their dialogs.

pub mod drawing;
pub(crate) mod registry;
pub(crate) mod tools;

use crate::state::{AppState, NoteKind};
use soliddraft_assembly::{PackReport, PackRequest};
use soliddraft_document::DocRef;
use std::path::{Path, PathBuf};

/// What the ribbon asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum FilesAction {
    /// Asks for a folder and packs into it.
    PackAndGo,
    /// Asks for a file and exports to it: `urdf` or `usda`.
    Export(&'static str),
    /// Makes a drawing of the assembly with a BOM and balloons.
    MakeDrawing,
}

/// Why Pack and Go is not offered in the browser.
pub const BROWSER_PACK: &str = "Pack and Go writes a folder of files; the browser has no folders. Use the desktop app";

/// The drawing crate's repoint, as Pack and Go takes it.
fn repoint(bytes: &[u8], model: &DocRef) -> Result<Vec<u8>, String> {
    soliddraft_drawing::file::repoint(bytes, model).map_err(|e| e.to_string())
}

/// Packs the open assembly as `request` says, with the drawings of what it packs. The
/// assembly must be saved: Pack and Go copies files.
pub(crate) fn pack(state: &mut AppState, mut request: PackRequest, drawings: bool) -> Result<PackReport, String> {
    let s = state.assembly.session.as_mut().ok_or("no assembly is open")?;
    let root = s.doc.path().ok_or("save the assembly first: Pack and Go copies its file")?.to_path_buf();
    if s.doc.is_dirty() {
        return Err("save the assembly first: Pack and Go copies the files as they are saved".into());
    }
    request.root = root;
    request.search.extend(crate::settings::file_locations().search_folders().iter().cloned());
    let drawings: Option<soliddraft_assembly::pack::Repoint> = if drawings { Some(&repoint) } else { None };
    soliddraft_assembly::pack_and_go(&request, drawings).map_err(|e| e.to_string())
}

/// Exports the open assembly to `path`: on the desktop every file is written; in the
/// browser a URDF and its meshes download as one zip.
pub(crate) fn export(
    state: &mut AppState,
    path: &Path,
) -> Result<(soliddraft_scene::export::Articulation, Vec<PathBuf>), String> {
    let s = state.assembly.session.as_mut().ok_or("no assembly is open")?;
    let (articulation, files) = soliddraft_scene::export::export_files(&mut s.doc, path)?;
    let paths: Vec<PathBuf> = files.iter().map(|(p, _)| p.clone()).collect();
    if cfg!(target_arch = "wasm32") && files.len() > 1 {
        let dir = path.parent().unwrap_or(Path::new(""));
        let entries: Vec<(String, Vec<u8>)> = files
            .into_iter()
            .map(|(p, b)| (p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().replace('\\', "/"), b))
            .collect();
        crate::platform::write_file(&path.with_extension("zip"), &crate::zip::stored(&entries))?;
    } else {
        for (p, bytes) in &files {
            crate::platform::write_file(p, bytes)?;
        }
    }
    Ok((articulation, paths))
}

pub fn apply(state: &mut AppState, action: FilesAction) {
    match action {
        FilesAction::MakeDrawing => match drawing::make_drawing(state, &drawing::AssemblyDrawingRequest::default()) {
            Ok(said) => state.note(said, NoteKind::Ok),
            Err(e) => state.note(e, NoteKind::Error),
        },
        FilesAction::PackAndGo => {
            if cfg!(target_arch = "wasm32") {
                return state.note(BROWSER_PACK, NoteKind::Info);
            }
            let Some(into) = crate::platform::pick_folder("Pack and Go into an empty folder") else { return };
            let request = PackRequest { into, ..PackRequest::default() };
            match pack(state, request, true) {
                Ok(report) => state.note(
                    format!(
                        "Packed {} files into {}",
                        report.files.len(),
                        report.root.parent().unwrap_or(&report.root).display()
                    ),
                    NoteKind::Ok,
                ),
                Err(e) => state.note(e, NoteKind::Error),
            }
        }
        FilesAction::Export(ext) => {
            let Some(s) = state.assembly.session.as_ref() else { return };
            let (label, filter) =
                if ext == "urdf" { ("Export URDF", "URDF robot") } else { ("Export OpenUSD", "OpenUSD text") };
            let name = format!("{}.{ext}", s.doc.assembly().name);
            let Some(path) = crate::platform::save_path(label, filter, ext, &name) else { return };
            match export(state, &path) {
                Ok((a, files)) => {
                    let moving = a.joints.iter().filter(|j| j.kind != soliddraft_scene::JointKind::Fixed).count();
                    let notes = if a.notes.is_empty() { String::new() } else { format!(" · {}", a.notes.join("; ")) };
                    state.note(
                        format!(
                            "Exported {} links and {moving} moving joints in {} files{notes}",
                            a.links.len(),
                            files.len()
                        ),
                        if a.notes.is_empty() { NoteKind::Ok } else { NoteKind::Info },
                    );
                }
                Err(e) => state.note(e, NoteKind::Error),
            }
        }
    }
}
