//! `soliddraft asm pack`, `asm export` and `asm drawing`: the files an assembly writes.

use std::path::{Path, PathBuf};

use serde_json::json;
use soliddraft_assembly::{PackRequest, pack_and_go};
use soliddraft_document::DocRef;

use super::asm::Args;

/// The drawing crate's repoint, as Pack and Go takes it.
pub(crate) fn repoint(bytes: &[u8], model: &DocRef) -> Result<Vec<u8>, String> {
    soliddraft_drawing::file::repoint(bytes, model).map_err(|e| e.to_string())
}

/// `asm pack <file.dwasm> <folder> [--prefix P] [--suffix S] [--flatten] [--drawings]`.
pub(crate) fn pack(file: &Path, rest: &[String], a: &Args, search: Vec<PathBuf>) -> Result<bool, String> {
    let [into] = rest else { return Err("pack takes the assembly and a destination folder".into()) };
    let request = PackRequest {
        root: file.to_path_buf(),
        into: PathBuf::from(into),
        prefix: a.value("--prefix").unwrap_or_default().to_owned(),
        suffix: a.value("--suffix").unwrap_or_default().to_owned(),
        flatten: a.value("--flatten").is_some(),
        search,
    };
    let drawings: Option<soliddraft_assembly::pack::Repoint> =
        if a.value("--drawings").is_some() { Some(&repoint) } else { None };
    let report = pack_and_go(&request, drawings).map_err(|e| e.to_string())?;
    if a.json {
        println!("{}", json!(report));
    } else {
        for f in &report.files {
            let how = if f.rewritten { "rewrote" } else { "copied" };
            println!("{how} {} -> {}", f.from.display(), f.to.display());
        }
        println!("packed {} files; open {}", report.files.len(), report.root.display());
    }
    Ok(true)
}

/// `asm export <file.dwasm> <out.urdf | out.usda>`: links and joints for robotics tools.
pub(crate) fn export(
    doc: &mut soliddraft_assembly::AssemblyDocument,
    rest: &[String],
    a: &Args,
) -> Result<bool, String> {
    let [out] = rest else { return Err("export takes the assembly and a .urdf or .usda file".into()) };
    let (articulation, files) = soliddraft_scene::export::export(doc, Path::new(out))?;
    if a.json {
        println!("{}", json!({ "articulation": articulation, "files": files }));
    } else {
        for j in &articulation.joints {
            let kind = format!("{:?}", j.kind).to_lowercase();
            let from = if j.mates.is_empty() { String::new() } else { format!(" from {}", j.mates.join(", ")) };
            println!("{} {kind}{from}", j.name);
        }
        for n in &articulation.notes {
            println!("note: {n}");
        }
        println!("wrote {} files; {}", files.len(), out);
    }
    Ok(false)
}
