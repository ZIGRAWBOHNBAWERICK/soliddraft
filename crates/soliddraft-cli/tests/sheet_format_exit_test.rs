//! Larger and legacy sheets through `soliddraft draw`, as separate processes: the bracket
//! on ISO A3 and ANSI B, the built-in sizes, and on ISO A0 and ANSI E, the larger ones,
//! with a title block field's text set and two fields moved to other cells. Each SVG,
//! PDF page and DXF has the paper's declared size, the moved field's text sits in its new
//! cell in the SVG and the DXF, and the drawing reopens with the moved cells.

use std::path::{Path, PathBuf};
use std::process::Command;

use soliddraft_drawing::display::{self, Item};

fn soliddraft(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_soliddraft"))
        .args(args)
        .current_dir(dir)
        .env("SOURCE_DATE_EPOCH", "1791936000")
        .output()
        .expect("running soliddraft");
    assert!(out.status.success(), "soliddraft {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn near(actual: f64, expected: f64, what: &str) {
    assert!((actual - expected).abs() < 1e-3, "{what}: {actual} is not {expected}");
}

/// The x and y of the SVG text element that writes `text`.
fn svg_text(svg: &str, text: &str) -> (f64, f64) {
    let line = svg.lines().find(|l| l.ends_with(&format!(">{text}</text>"))).unwrap_or_else(|| panic!("no {text}"));
    let attr = |name: &str| -> f64 {
        let start = line.find(&format!(" {name}=\"")).unwrap() + name.len() + 3;
        line[start..].split('"').next().unwrap().parse().unwrap()
    };
    (attr("x"), attr("y"))
}

#[test]
fn larger_and_legacy_sheets_print_at_their_size_with_their_title_layout() {
    let dir = std::env::temp_dir().join(format!("soliddraft-sheet-format-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let recipes = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../recipes");
    std::fs::copy(recipes.join("bracket.json"), dir.join("bracket.json")).unwrap();
    for (template, width, height) in
        [("iso-a3", 420.0, 297.0), ("ansi-b", 431.8, 279.4), ("iso-a0", 1189.0, 841.0), ("ansi-e", 1117.6, 863.6)]
    {
        let out = |ext: &str| format!("{template}.{ext}");
        let args = [
            "draw",
            "bracket.json",
            "--template",
            template,
            "--title-field",
            "owner=Rig Shop",
            "--title-cell",
            "owner=120,27,60,9,5",
            "--title-cell",
            "material=0,9,60,9",
            "--out",
            &out("dwdrw"),
            "--out",
            &out("svg"),
            "--out",
            &out("pdf"),
            "--out",
            &out("dxf"),
        ];
        soliddraft(&dir, &args);

        // The drawing reopens with the field's text and both moved cells.
        let (drawing, _) = soliddraft_drawing::file::read(&dir.join(out("dwdrw"))).expect("the drawing opens");
        let sheet = &drawing.sheets[0];
        assert_eq!(sheet.template.as_str(), template);
        assert_eq!(sheet.format_overrides.get("owner").map(String::as_str), Some("Rig Shop"));
        assert_eq!(sheet.title_layout["owner"].cell, [120.0, 27.0, 60.0, 9.0]);
        assert_eq!(sheet.title_layout["owner"].size, Some(5.0));
        assert_eq!(sheet.title_layout["material"].cell, [0.0, 9.0, 60.0, 9.0]);

        // The laid-out sheet has the paper's size and the text in its new cell.
        let list = display::layout_sheet(&drawing, 0);
        assert_eq!(list.paper, (width, height), "{template}");
        let origin = soliddraft_drawing::library::template(template).title_block_origin();
        let item = list
            .items
            .iter()
            .find_map(|i| match i {
                Item::Text(t) if t.text == "Rig Shop" => Some(t.clone()),
                _ => None,
            })
            .expect("the owner's text is on the sheet");
        near(item.pos.u, origin.u + 121.5, "the owner's text x");
        near(item.pos.v, origin.v + 28.5, "the owner's text y");
        // The 5 mm text asked for is held to the 9 mm cell less its 5 mm caption band.
        near(item.height, 4.0, "the owner's text height");
        let (at, _) = soliddraft_drawing::export::baseline(&item);

        let svg = std::fs::read_to_string(dir.join(out("svg"))).unwrap();
        assert!(
            svg.contains(&format!(r#"width="{width}mm" height="{height}mm" viewBox="0 0 {width} {height}""#)),
            "{template}: the SVG is not {width} x {height} mm"
        );
        let (x, y) = svg_text(&svg, "Rig Shop");
        near(x, at.u, "SVG text x");
        near(y, height - at.v, "SVG text y");

        let pdf = lopdf::Document::load(dir.join(out("pdf"))).expect("the PDF loads");
        let pages = pdf.get_pages();
        assert_eq!(pages.len(), 1);
        let page = pdf.get_dictionary(*pages.values().next().unwrap()).unwrap();
        let media: Vec<f64> =
            page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|v| v.as_float().unwrap() as f64).collect();
        let pt = 72.0 / 25.4;
        assert!((media[2] - width * pt).abs() < 0.01 && (media[3] - height * pt).abs() < 0.01, "{template}: {media:?}");

        let dxf = dxf::Drawing::load_file(dir.join(out("dxf"))).unwrap();
        let extents = &dxf.header.maximum_drawing_extents;
        near(extents.x, width, "DXF extents x");
        near(extents.y, height, "DXF extents y");
        let text = dxf
            .entities()
            .find_map(|e| match &e.specific {
                dxf::entities::EntityType::Text(t) if t.value == "Rig Shop" => Some(t.location.clone()),
                dxf::entities::EntityType::MText(t) if t.text == "Rig Shop" => Some(t.insertion_point.clone()),
                _ => None,
            })
            .expect("the owner's text is in the DXF");
        near(text.x, at.u, "DXF text x");
        near(text.y, at.v, "DXF text y");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
