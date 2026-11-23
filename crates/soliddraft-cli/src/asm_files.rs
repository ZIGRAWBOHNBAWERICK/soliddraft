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

/// `asm drawing <file.dwasm> <out.dwdrw> [--template ID] [--exploded]`:
/// a drawing of the assembly with a BOM and balloons; an existing drawing of the same
/// assembly is brought up to date instead, keeping its id and whatever was added to it.
pub(crate) fn drawing(
    doc: &mut soliddraft_assembly::AssemblyDocument,
    rest: &[String],
    a: &Args,
) -> Result<bool, String> {
    use soliddraft_drawing::assembly::{NewAssemblyDrawing, make_assembly_drawing, update_assembly_drawing};
    use soliddraft_drawing::file::{self, FileMeta};
    use soliddraft_kernel_api::drawing::StandardView;
    let [out] = rest else { return Err("drawing takes the assembly and a .dwdrw file".into()) };
    let out = Path::new(out);
    let model = soliddraft_app::assembly_files::drawing::assembly_model(doc)?;
    let (drawing, meta, verb) = if out.exists() {
        let (old, meta) = file::read(out).map_err(|e| e.to_string())?;
        if old.model.id != Some(doc.id()) {
            return Err(format!("{} is a drawing of another document", out.display()));
        }
        let next = update_assembly_drawing(&old, &model).map_err(|e| e.to_string())?;
        (next, meta.rewritten("display meshes", Default::default()), "updated")
    } else {
        let exploded = a.value("--exploded").is_some();
        if exploded && model.exploded.is_empty() {
            return Err("the assembly has no exploded view; add explode steps first".into());
        }
        let template = a.value("--template").unwrap_or("iso-a3");
        if !soliddraft_drawing::library::library().templates.iter().any(|t| t.id == template) {
            return Err(format!("no template `{template}`"));
        }
        let new = NewAssemblyDrawing {
            template: soliddraft_drawing::library::template(template).template(),
            meta: soliddraft_drawing::DrawingMeta { title: doc.assembly().name.clone(), ..Default::default() },
            views: if exploded {
                vec![StandardView::Isometric]
            } else {
                vec![StandardView::Isometric, StandardView::Front]
            },
            scale: None,
            balloons: true,
            exploded,
        };
        let made = make_assembly_drawing(&model, &new).map_err(|e| e.to_string())?;
        (made, FileMeta::new("display meshes", Default::default()), "wrote")
    };
    let mut drawing = drawing;
    // The reference is stored relative to the drawing.
    if let Some(at) = doc.path() {
        drawing.model.set_path(out, at);
    }
    file::write(out, &drawing, &meta).map_err(|e| e.to_string())?;
    let balloons = drawing
        .sheets
        .iter()
        .flat_map(|s| &s.annotations)
        .filter(|x| matches!(x.body, soliddraft_drawing::AnnotationKind::Balloon { .. }))
        .count();
    if a.json {
        let bom: Vec<_> = drawing.sheets.iter().flat_map(|s| &s.tables).flat_map(|t| t.bom.clone()).collect();
        println!(
            "{}",
            json!({ "drawing": out, "bom": bom, "balloons": balloons, "views": drawing.sheets[0].views.len() })
        );
    } else {
        println!("{verb} {} · {} BOM rows · {balloons} balloons", out.display(), model.bom.len());
    }
    Ok(false)
}
