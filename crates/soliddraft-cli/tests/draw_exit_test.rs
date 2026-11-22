//! The phase D1 exit test, through the `soliddraft draw` binary as separate processes:
//!
//! - `soliddraft draw bracket.json --template iso-a3` writes a drawing file, an SVG, a PDF
//!   and a DXF;
//! - the SVG matches `recipes/golden/catallax-bracket-a3.svg`;
//! - the DXF re-reads with the `dxf` crate with the entity counts per layer in
//!   `recipes/golden/catallax-bracket-a3-dxf.json` and no splines;
//! - the PDF has one A3 page and the embedded text "Bracket";
//! - `soliddraft draw --update` after changing the plate thickness in the recipe reports
//!   one changed value and none dangling; after deleting the slot it reports two
//!   dangling, and the drawing file still opens.
//!
//! Set `SOLIDDRAFT_BLESS=1` to rewrite the golden files after a deliberate change.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use soliddraft_drawing::{AnnotationState, Drawing, apply, file};
use soliddraft_kernel_api::Point2;

/// 2026-10-14, so the title block's date is fixed.
const DATE_EPOCH: &str = "1791936000";

fn recipes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes")
}

fn bless() -> bool {
    std::env::var_os("SOLIDDRAFT_BLESS").is_some()
}

fn soliddraft(dir: &Path, args: &[&str]) -> String {
    let out = run(dir, args);
    assert!(out.status.success(), "soliddraft {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_soliddraft"))
        .args(args)
        .current_dir(dir)
        .env("SOURCE_DATE_EPOCH", DATE_EPOCH)
        .output()
        .expect("running soliddraft")
}

fn shortened_box(case: &str) -> (PathBuf, Drawing) {
    use soliddraft_drawing::Command;

    let dir = std::env::temp_dir().join(format!("soliddraft-draw-{case}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let recipe = dir.join("box.json");
    std::fs::copy(recipes_dir().join("box.json"), &recipe).unwrap();
    soliddraft(
        &dir,
        &[
            "draw",
            "box.json",
            "--views",
            "front,top",
            "--hidden",
            "visible",
            "--broken-out",
            "top:0,0,10:4:7",
            "--out",
            "box.dwdrw",
        ],
    );
    let path = dir.join("box.dwdrw");
    let (mut drawing, meta) = file::read(&path).unwrap();
    let cut = drawing.sheets[0].views.iter().find(|v| !v.broken_out.is_empty()).unwrap().id;
    apply(&mut drawing, &Command::AddSheet { name: "Cut".into(), template: "iso-a3".into() }).unwrap();
    apply(&mut drawing, &Command::MoveViewToSheet { view: cut, sheet: 1, to: Point2::new(140.0, 100.0) }).unwrap();
    let mut annotation = drawing.sheets[0].annotations.iter().find(|a| !a.anchors.is_empty()).unwrap().clone();
    annotation.id = drawing.next_annotation_id();
    annotation.view = cut;
    apply(&mut drawing, &Command::AddAnnotation { annotation }).unwrap();
    file::write(&path, &drawing, &meta).unwrap();
    edit_recipe(&recipe, |v| v["steps"][1]["distance"] = 5.into());
    (dir, drawing)
}

fn assert_partial_box_saved(dir: &Path, before: &Drawing) -> Drawing {
    let (after, _) = file::read(&dir.join("box.dwdrw")).unwrap();
    assert_ne!(after.model.pinned, before.model.pinned);
    assert_eq!(after.sheets[0].views[0].geometry.source_digest, after.model.pinned.as_str());
    assert_eq!(after.sheets[1].views[0].geometry, before.sheets[1].views[0].geometry);
    after
}

#[test]
fn pdf_refusal_allows_healthy_svg_before_a_later_legacy_cache_error() {
    let (dir, mut drawing) = shortened_box("pdf-refusal");
    let path = dir.join("box.dwdrw");
    let (_, meta) = file::read(&path).unwrap();
    let mut legacy = drawing.sheets[0].clone();
    legacy.name = "Legacy".into();
    let view = &mut legacy.views[0];
    view.id = drawing.next_view_id();
    view.label = Some("Legacy view".into());
    view.excluded_from_update = true;
    view.geometry.catallax = false;
    legacy.annotations.clear();
    drawing.sheets.push(legacy);
    file::write(&path, &drawing, &meta).unwrap();

    let out = run(
        &dir,
        &["draw", "--update", "box.dwdrw", "--out", "all.pdf", "--out", "sheet.svg", "--out", "later.dxf", "--json"],
    );
    assert!(!out.status.success());
    let error = String::from_utf8(out.stderr).unwrap();
    assert!(
        error.contains("all.pdf: Top view:") && error.contains("the cutting plane is behind the whole body"),
        "{error}"
    );
    assert!(error.contains("sheet-3.svg: Legacy view has no saved Catallax line work"), "{error}");
    assert!(error.find("all.pdf:").unwrap() < error.find("sheet-3.svg:").unwrap());
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    let exports = report["exports"].as_array().unwrap();
    assert_eq!(exports.len(), 1);
    assert_eq!(exports[0]["path"], "sheet.svg");
    assert!(std::fs::read_to_string(dir.join("sheet.svg")).unwrap().contains("<svg"));
    for name in ["all.pdf", "sheet-2.svg", "sheet-3.svg", "later.dxf"] {
        assert!(!dir.join(name).exists(), "{name}");
    }
    assert_partial_box_saved(&dir, &drawing);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn accept_saves_repairs_to_a_partial_rebuild() {
    let (dir, before) = shortened_box("accept");
    let report: Value =
        serde_json::from_str(&soliddraft(&dir, &["draw", "--update", "box.dwdrw", "--accept", "--json"])).unwrap();
    assert!(!report["repairs"].as_array().unwrap().is_empty(), "{report}");
    assert_eq!(report["dangling_views"][0]["name"], "Top view");
    let after = assert_partial_box_saved(&dir, &before);
    assert!(after.sheets[0].annotations.iter().all(|a| !matches!(a.state, AnnotationState::Changed { .. })));
    std::fs::remove_dir_all(dir).unwrap();
}

fn assert_failed_repair_keeps_rebuild(dir: &Path, before: &Drawing, flag: &str, argument: &str) {
    let out = run(dir, &["draw", "--update", "box.dwdrw", flag, argument, "--json"]);
    assert!(!out.status.success());
    let error = String::from_utf8(out.stderr).unwrap();
    assert!(error.contains("Annotation repair failed") && error.contains("Top view:"), "{error}");
    assert!(error.contains("the cutting plane is behind the whole body"), "{error}");
    assert_partial_box_saved(dir, before);
}

#[test]
fn reattach_refusal_keeps_the_partial_rebuild() {
    let (dir, before) = shortened_box("reattach");
    let annotation = before.sheets[1].annotations.last().unwrap().id.0;
    assert_failed_repair_keeps_rebuild(&dir, &before, "--reattach", &format!("{annotation}:0:20,0,5"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_keep_as_note_keeps_the_partial_rebuild() {
    let (dir, before) = shortened_box("keep-as-note");
    assert_failed_repair_keeps_rebuild(&dir, &before, "--keep-as-note", "999999");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn unknown_delete_keeps_the_partial_rebuild() {
    let (dir, before) = shortened_box("delete");
    assert_failed_repair_keeps_rebuild(&dir, &before, "--delete", "999999");
    std::fs::remove_dir_all(dir).unwrap();
}

fn golden(name: &str, got: &str) {
    let path = recipes_dir().join("golden").join(name);
    if bless() {
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want =
        std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}; run with SOLIDDRAFT_BLESS=1", path.display()));
    if want != got {
        let line = want.lines().zip(got.lines()).position(|(a, b)| a != b).unwrap_or(0);
        panic!(
            "{} differs at line {}:\n  want {}\n  got  {}",
            path.display(),
            line + 1,
            want.lines().nth(line).unwrap_or(""),
            got.lines().nth(line).unwrap_or("")
        );
    }
}

fn edit_recipe(path: &Path, f: impl FnOnce(&mut Value)) {
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    f(&mut v);
    std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

#[test]
fn bracket_drawing_exports_and_follows_the_part() {
    let dir = std::env::temp_dir().join(format!("soliddraft-draw-exit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let recipe = dir.join("bracket.json");
    std::fs::copy(recipes_dir().join("bracket.json"), &recipe).unwrap();

    soliddraft(&dir, &["draw", "bracket.json", "--template", "iso-a3"]);
    for ext in ["dwdrw", "svg", "pdf", "dxf"] {
        assert!(dir.join(format!("bracket.{ext}")).is_file(), "bracket.{ext} not written");
    }

    // SVG: the golden file.
    golden("catallax-bracket-a3.svg", &std::fs::read_to_string(dir.join("bracket.svg")).unwrap());

    // DXF: re-reads, golden counts per layer, no splines.
    let dxf = dxf::Drawing::load_file(dir.join("bracket.dxf")).expect("the DXF re-reads");
    let mut counts: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for e in dxf.entities() {
        let kind = match &e.specific {
            dxf::entities::EntityType::Line(_) => "line",
            dxf::entities::EntityType::Arc(_) => "arc",
            dxf::entities::EntityType::Circle(_) => "circle",
            dxf::entities::EntityType::Solid(_) => "solid",
            dxf::entities::EntityType::Text(_) => "text",
            dxf::entities::EntityType::Spline(_) => panic!("a spline in the DXF"),
            _ => "other",
        };
        *counts.entry(e.common.layer.clone()).or_default().entry(kind.to_owned()).or_default() += 1;
    }
    for layer in ["VISIBLE", "DIMENSION", "TEXT", "FORMAT"] {
        assert!(counts.contains_key(layer), "no {layer} entities: {counts:?}");
    }
    golden("catallax-bracket-a3-dxf.json", &(serde_json::to_string_pretty(&counts).unwrap() + "\n"));

    // PDF: one A3 page with the title in it.
    let pdf = lopdf::Document::load(dir.join("bracket.pdf")).expect("the PDF loads");
    let pages = pdf.get_pages();
    assert_eq!(pages.len(), 1);
    let page = pdf.get_dictionary(pages[&1]).unwrap();
    let mbox: Vec<f32> =
        page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|v| v.as_float().unwrap()).collect();
    assert!((mbox[2] - 1190.55).abs() < 0.1 && (mbox[3] - 841.89).abs() < 0.1, "A3 landscape: {mbox:?}");
    let text = pdf.extract_text(&[1]).unwrap();
    assert!(text.contains("Bracket"), "no Bracket in the PDF text: {text}");

    // The plate gets thicker: one value changes, nothing dangles.
    edit_recipe(&recipe, |v| {
        let plate = v["steps"].as_array_mut().unwrap().iter_mut().find(|s| s["id"] == "plate").unwrap();
        plate["distance"] = 12.into();
    });
    let report: Value =
        serde_json::from_str(&soliddraft(&dir, &["draw", "--update", "bracket.dwdrw", "--json"])).unwrap();
    let changed = report["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 1, "{report:#}");
    assert_eq!((changed[0]["previous"].as_str(), changed[0]["now"].as_str()), (Some("10"), Some("12")));
    assert_eq!(report["dangling"].as_array().unwrap().len(), 0, "{report:#}");

    // The slot goes: its two dimensions dangle, and the file still opens.
    edit_recipe(&recipe, |v| {
        let steps = v["steps"].as_array_mut().unwrap();
        steps.retain(|s| !s["id"].as_str().unwrap().starts_with("slot"));
        v["result"] = "hole4".into();
    });
    let report: Value =
        serde_json::from_str(&soliddraft(&dir, &["draw", "--update", "bracket.dwdrw", "--json"])).unwrap();
    assert_eq!(report["changed"].as_array().unwrap().len(), 0, "{report:#}");
    let dangling = report["dangling"].as_array().unwrap();
    assert_eq!(dangling.len(), 2, "{report:#}");
    assert!(dangling.iter().all(|d| d["cause"].as_str().unwrap().contains("slot")), "{report:#}");
    let (drawing, _) = soliddraft_drawing::file::read(&dir.join("bracket.dwdrw")).expect("the file still opens");
    assert_eq!(drawing.annotations_needing_attention().count(), 3, "two dangling and the changed thickness");

    // Repairs: keep one as a note, delete the other, accept the thickness.
    let ids: Vec<String> = dangling.iter().map(|d| d["annotation"].to_string()).collect();
    let report: Value = serde_json::from_str(&soliddraft(
        &dir,
        &["draw", "--update", "bracket.dwdrw", "--keep-as-note", &ids[0], "--delete", &ids[1], "--json"],
    ))
    .unwrap();
    assert_eq!(report["dangling"].as_array().unwrap().len(), 0, "{report:#}");
    let (after, _) = soliddraft_drawing::file::read(&dir.join("bracket.dwdrw")).unwrap();
    assert_eq!(after.sheets[0].annotations.len(), drawing.sheets[0].annotations.len() - 1);
    std::fs::remove_dir_all(&dir).unwrap();
}
