//! Larger and legacy sheets through `soliddraft draw`, as separate processes: the bracket
//! on ISO A3 and ANSI B, the built-in sizes, and on ISO A0 and ANSI E, the larger ones,
//! with an edited title field. SVG, PDF and DXF retain the paper size and the field position.

use std::path::{Path, PathBuf};
use std::process::Command;

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

fn pdf_text(pdf: &lopdf::Document, page_id: lopdf::ObjectId, text: &str) -> ([f64; 6], [f64; 2]) {
    let page = pdf.get_dictionary(page_id).unwrap();
    let fonts = page.get(b"Resources").unwrap().as_dict().unwrap().get(b"Font").unwrap().as_dict().unwrap();
    let font = pdf.get_dictionary(fonts.iter().next().unwrap().1.as_reference().unwrap()).unwrap();
    let cmap = pdf.get_object(font.get(b"ToUnicode").unwrap().as_reference().unwrap()).unwrap().as_stream().unwrap();
    let glyphs: std::collections::BTreeMap<_, _> = String::from_utf8(cmap.get_plain_content().unwrap())
        .unwrap()
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_ascii_whitespace();
            let hex = |s: &str| u16::from_str_radix(s.strip_prefix('<')?.strip_suffix('>')?, 16).ok();
            let (source, unicode) = (hex(parts.next()?)?, hex(parts.next()?)?);
            parts.next().is_none().then_some((source, unicode))
        })
        .collect();
    let content = lopdf::content::Content::decode(&pdf.get_page_content(page_id)).unwrap();
    let mut matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut text_origin = [0.0; 2];
    for op in content.operations {
        let number = |i: usize| f64::from(op.operands[i].as_float().unwrap());
        match op.operator.as_str() {
            "cm" => matrix = std::array::from_fn(number),
            "Tm" => text_origin = [number(4), number(5)],
            "Tj" => {
                let encoded = op.operands[0].as_str().unwrap();
                let unicode: Option<Vec<u16>> = encoded
                    .chunks_exact(2)
                    .map(|pair| glyphs.get(&u16::from_be_bytes([pair[0], pair[1]])).copied())
                    .collect();
                if unicode.and_then(|c| String::from_utf16(&c).ok()).as_deref() == Some(text) {
                    return (matrix, text_origin);
                }
            }
            _ => {}
        }
    }
    panic!("no {text} in the PDF");
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
            "owner=4,9,54,9",
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
        assert_eq!(sheet.title_layout["owner"].cell, [4.0, 9.0, 54.0, 9.0]);
        let expected = (width - 10.0 - 180.0 + 4.0 + 1.5, 10.0 + 9.0 + 1.5);

        let svg = std::fs::read_to_string(dir.join(out("svg"))).unwrap();
        assert!(
            svg.contains(&format!(r#"width="{width}mm" height="{height}mm" viewBox="0 0 {width} {height}""#)),
            "{template}: the SVG is not {width} x {height} mm"
        );
        let (x, y) = svg_text(&svg, "Rig Shop");
        near(x, expected.0, "SVG text x");
        near(y, height - expected.1, "SVG text y");

        let pdf = lopdf::Document::load(dir.join(out("pdf"))).expect("the PDF loads");
        let pages = pdf.get_pages();
        assert_eq!(pages.len(), 1);
        let page_id = *pages.values().next().unwrap();
        let page = pdf.get_dictionary(page_id).unwrap();
        let media: Vec<f64> =
            page.get(b"MediaBox").unwrap().as_array().unwrap().iter().map(|v| v.as_float().unwrap() as f64).collect();
        let pt = 72.0 / 25.4;
        assert!((media[2] - width * pt).abs() < 0.01 && (media[3] - height * pt).abs() < 0.01, "{template}: {media:?}");

        let (matrix, text) = pdf_text(&pdf, page_id, "Rig Shop");
        near(matrix[0], pt, "PDF page scale x");
        near(matrix[3], pt, "PDF page scale y");
        near(matrix[4] + matrix[0] * text[0] + matrix[2] * text[1], expected.0 * pt, "PDF text x in points");
        near(matrix[5] + matrix[1] * text[0] + matrix[3] * text[1], expected.1 * pt, "PDF text y in points");

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
        near(text.x, expected.0, "DXF text x");
        near(text.y, expected.1, "DXF text y");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
