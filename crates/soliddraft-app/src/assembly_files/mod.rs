//! The files an open assembly writes, in the window and over MCP: Pack and Go (the
//! assembly with every document it references, in a new folder or one zip), and URDF and
//! OpenUSD with joints. The browser has no folders: there Pack and Go packs the files the
//! page holds into a zip it downloads, and an export downloads its files, several as one
//! zip. What each writes is `soliddraft-assembly`'s and `soliddraft-scene`'s (see
//! `docs/assemblies.md`); this module holds the commands and their dialogs.

pub mod drawing;
pub(crate) mod registry;
pub(crate) mod tools;

use crate::state::{AppState, NoteKind};
use soliddraft_assembly::{PackReport, PackRequest};
use soliddraft_document::DocRef;

/// What the ribbon asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum FilesAction {
    /// Asks for a folder and packs into it; the browser downloads a zip.
    PackAndGo,
    /// Asks for a zip file and packs into it; the browser downloads it.
    PackAndGoZip,
    /// Asks for a file and exports to it: `urdf` or `usda`.
    Export(&'static str),
    /// Makes a drawing of the assembly with a BOM and balloons.
    MakeDrawing,
}

/// Why the browser refuses a folder for Pack and Go.
pub const BROWSER_PACK: &str = "The browser has no folders: Pack and Go downloads one zip";

/// Why there is nothing to pack yet.
pub const SAVE_FIRST: &str = "save the assembly first: Pack and Go copies the files as they are saved";

/// The drawing crate's repoint, as Pack and Go takes it.
fn repoint(bytes: &[u8], model: &DocRef) -> Result<Vec<u8>, String> {
    soliddraft_drawing::file::repoint(bytes, model).map_err(|e| e.to_string())
}

/// Packs the open assembly as `request` says, with the drawings of what it packs: into
/// the folder `request.into`, or into the zip file `request.into` when `zip`. The
/// assembly must be saved: Pack and Go copies files. The browser packs the files the page
/// holds and downloads the zip under `request.into`'s name.
pub(crate) fn pack(
    state: &mut AppState,
    mut request: PackRequest,
    drawings: bool,
    zip: bool,
) -> Result<PackReport, String> {
    let s = state.assembly.session.as_ref().ok_or("no assembly is open")?;
    let root = s.doc.path().ok_or(SAVE_FIRST)?.to_path_buf();
    if s.doc.is_dirty() {
        return Err(SAVE_FIRST.into());
    }
    request.root = root;
    let drawings: Option<soliddraft_assembly::pack::Repoint> = if drawings { Some(&repoint) } else { None };
    if cfg!(target_arch = "wasm32") {
        if !zip {
            return Err(BROWSER_PACK.into());
        }
        let (report, bytes) = pack_held(state, &request, drawings)?;
        crate::platform::write_file(&request.into, &bytes)?;
        return Ok(report);
    }
    request.search.extend(crate::settings::file_locations().search_folders().iter().cloned());
    let packed = if zip {
        soliddraft_assembly::pack_zip(&request, drawings)
    } else {
        soliddraft_assembly::pack_and_go(&request, drawings)
    };
    packed.map_err(|e| e.to_string())
}

/// The open assembly packed from the files the page holds: the assembly as it was opened
/// or last saved, and the files handed over with it, each kept in the revision store.
/// Returns the report, each file's `to` its place in the zip, and the zip's bytes.
pub(crate) fn pack_held(
    state: &AppState,
    request: &PackRequest,
    drawings: Option<soliddraft_assembly::pack::Repoint>,
) -> Result<(PackReport, Vec<u8>), String> {
    let s = state.assembly.session.as_ref().ok_or("no assembly is open")?;
    let root = state.revisions.fetch(s.doc.id(), &s.doc.revision()).map_err(|_| SAVE_FIRST)?;
    let held = state.assembly.held.iter().filter_map(|d| {
        let bytes = state.revisions.fetch(d.id, &d.revision).ok()?;
        Some((d.path.clone()?, bytes))
    });
    let source = soliddraft_assembly::HeldFiles::new(std::iter::once((request.root.clone(), root)).chain(held));
    let pack = soliddraft_assembly::gather(request, &source, drawings).map_err(|e| e.to_string())?;
    let bytes = pack.zip().map_err(|e| e.to_string())?;
    Ok((pack.report(std::path::Path::new("")), bytes))
}

/// The name a zip of the open assembly gets: the assembly's file name with `.zip`.
fn zip_name(state: &AppState) -> String {
    let s = state.assembly.session.as_ref();
    let stem = s.and_then(|s| s.doc.path()).and_then(|p| p.file_stem()).map(|n| n.to_string_lossy().into_owned());
    format!("{}.zip", stem.unwrap_or_else(|| s.map(|s| s.doc.assembly().name.clone()).unwrap_or_default()))
}

/// Says what Pack and Go wrote.
fn packed_note(state: &mut AppState, packed: Result<PackReport, String>, place: &std::path::Path) {
    match packed {
        Ok(report) => state.note(format!("Packed {} files into {}", report.files.len(), place.display()), NoteKind::Ok),
        Err(e) => state.note(e, NoteKind::Error),
    }
}

/// Exports the open assembly to `path`, writing every file. In the browser the files
/// download, several as one zip named after `path`.
pub(crate) fn export(
    state: &mut AppState,
    path: &std::path::Path,
) -> Result<(soliddraft_scene::export::Articulation, Vec<std::path::PathBuf>), String> {
    let s = state.assembly.session.as_mut().ok_or("no assembly is open")?;
    let (articulation, files) = soliddraft_scene::export::export_files(&mut s.doc, path)?;
    let paths: Vec<std::path::PathBuf> = files.iter().map(|(p, _)| p.clone()).collect();
    crate::platform::write_export(path, files)?;
    Ok((articulation, paths))
}

pub fn apply(state: &mut AppState, action: FilesAction) {
    match action {
        FilesAction::MakeDrawing => match drawing::make_drawing(state, &drawing::AssemblyDrawingRequest::default()) {
            Ok(said) => state.note(said, NoteKind::Ok),
            Err(e) => state.note(e, NoteKind::Error),
        },
        // The browser has no folder to pick: it downloads the zip.
        FilesAction::PackAndGo if !cfg!(target_arch = "wasm32") => {
            let Some(into) = crate::platform::pick_folder("Pack and Go into an empty folder") else { return };
            let request = PackRequest { into: into.clone(), ..PackRequest::default() };
            let packed = pack(state, request, true, false);
            packed_note(state, packed, &into);
        }
        FilesAction::PackAndGo | FilesAction::PackAndGoZip => {
            let name = zip_name(state);
            let Some(into) = crate::platform::save_path("Pack and Go into a zip", "Zip archive", "zip", &name) else {
                return;
            };
            let request = PackRequest { into: into.clone(), ..PackRequest::default() };
            let packed = pack(state, request, true, true);
            packed_note(state, packed, &into);
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
