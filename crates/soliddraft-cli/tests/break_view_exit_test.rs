//! Half sections and break views through `soliddraft draw`, as separate processes, on the
//! certified Catallax path, on the long shaft in `recipes/shaft.json` (a 20 mm body from
//! x = 0 to 240, 12 mm journals to x = -20 and 260, a 6 mm bore through):
//!
//! - a half section of the front view cuts open the side above the axis to the plane
//!   through it: only that side is hatched, with the true area of the cut wall, the other
//!   side shows the outside, and the axis is drawn as a centre line;
//! - a break leaves out x = 60 to 180 and closes up to a 6 mm gap: the view is shorter
//!   by what it leaves out, no line work is drawn in the gap, and a baseline dimension
//!   across the break reads the full 280 mm on the sheet and in the native DXF;
//! - both reflow after a model edit and `draw --update`, and reopen as they were.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use soliddraft_drawing::{AnnotationKind, CurveKind, View};

fn recipes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes")
}

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

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(recipes_dir().join("shaft.json"), dir.join("shaft.json")).unwrap();
    dir
}

/// The drawing's only view and its scale.
fn front(path: &Path) -> (View, f64) {
    let (drawing, _) = soliddraft_drawing::file::read(path).expect("the drawing opens");
    let sheet = &drawing.sheets[0];
    assert_eq!(sheet.views.len(), 1);
    let view = sheet.views[0].clone();
    assert!(view.has_saved_catallax_linework(), "certified line work, which an export writes");
    let scale = soliddraft_drawing::project::effective_scale(sheet, &view);
    (view, scale)
}

fn ring_area(ring: &[soliddraft_kernel_api::Point2]) -> f64 {
    (0..ring.len())
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            a.u * b.v - b.u * a.v
        })
        .sum::<f64>()
        .abs()
        / 2.0
}

fn cut_area(view: &View, scale: f64) -> f64 {
    let area: f64 = view
        .geometry
        .hatch
        .iter()
        .map(|h| ring_area(&h.outer) - h.holes.iter().map(|r| ring_area(r)).sum::<f64>())
        .sum();
    area / (scale * scale)
}

fn near(actual: f64, expected: f64, tol: f64, what: &str) {
    assert!((actual - expected).abs() <= tol, "{what}: {actual} is not {expected}");
}

fn ends(view: &View, kind: Option<CurveKind>) -> Vec<(f64, f64)> {
    view.geometry
        .curves
        .iter()
        .filter(|c| kind.is_none_or(|k| c.kind == k))
        .flat_map(|c| c.path.elements().iter().filter_map(|e| e.end_point()).map(|p| (p.x, p.y)).collect::<Vec<_>>())
        .collect()
}

fn set_body_length(path: &Path, id: &str, distance: f64) {
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let step = v["steps"].as_array_mut().unwrap().iter_mut().find(|s| s["id"] == id).unwrap();
    step["distance"] = distance.into();
    std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

#[test]
fn half_section_of_a_symmetric_shaft() {
    let dir = scratch("half-section");
    let invalid = Command::new(env!("CARGO_BIN_EXE_soliddraft"))
        .args([
            "draw",
            "shaft.json",
            "--views",
            "front",
            "--half-section",
            "front:-20,0,0:260,3,0",
            "--out",
            "bad.dwdrw",
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("same cutting plane"));
    assert!(!dir.join("bad.dwdrw").exists());
    let args = [
        "draw",
        "shaft.json",
        "--views",
        "front",
        "--dims",
        "none",
        "--half-section",
        "front:-20,0,0:260,0,0",
        "--out",
        "shaft.dwdrw",
        "--out",
        "shaft.svg",
        "--out",
        "shaft.dxf",
        "--json",
    ];
    soliddraft(&dir, &args);
    let (view, scale) = front(&dir.join("shaft.dwdrw"));
    assert!(view.half.is_some(), "the half section is saved with the view");
    let [a, b] = view.geometry.symmetry.expect("the symmetry line is drawn");
    near(a.v, b.v, 1e-9, "the shaft's axis runs across the sheet");
    let axis = a.v;
    // Only the side above the axis is cut: the wall between the bore and the outside,
    // 7 mm over the 240 mm body and 3 mm over each 20 mm journal.
    near(cut_area(&view, scale), 7.0 * 240.0 + 2.0 * 3.0 * 20.0, 1e-6, "cut area");
    for h in &view.geometry.hatch {
        assert!(h.outer.iter().all(|p| p.v >= axis - 1e-6), "a hatch below the axis: {:?}", h.outer);
    }
    let cut = ends(&view, Some(CurveKind::Section));
    assert!(!cut.is_empty() && cut.iter().all(|(_, v)| *v >= axis - 1e-6), "section lines lie on the cut side only");
    // The other side shows the outside: the body's outline 10 mm below the axis.
    let outside: Vec<(f64, f64)> = view
        .geometry
        .curves
        .iter()
        .filter(|c| c.kind != CurveKind::Section)
        .flat_map(|c| c.path.elements().iter().filter_map(|e| e.end_point()).map(|p| (p.x, p.y)).collect::<Vec<_>>())
        .collect();
    assert!(outside.iter().any(|(_, v)| (*v - (axis - 10.0 * scale)).abs() < 1e-6), "the uncut half is drawn");
    let svg = std::fs::read_to_string(dir.join("shaft.svg")).unwrap();
    assert!(svg.contains(r#"<g id="HATCH""#));
    let counts =
        soliddraft_drawing::export::dxf::count_entities(&std::fs::read(dir.join("shaft.dxf")).unwrap()).unwrap();
    assert_eq!(counts.get("HATCH/HATCH").copied().unwrap_or(0), view.geometry.hatch.len());

    // The body grows 10 mm: the half section reflows with it.
    set_body_length(&dir.join("shaft.json"), "body", 250.0);
    let report: Value =
        serde_json::from_str(&soliddraft(&dir, &["draw", "--update", "shaft.dwdrw", "--json"])).unwrap();
    assert!(report.get("dangling_views").is_none(), "{report:#}");
    let (after, scale) = front(&dir.join("shaft.dwdrw"));
    assert_eq!(after.half, view.half, "the half section reopens as it was");
    // The body now runs to x = 250; the right journal shows only from 250 to 260.
    near(cut_area(&after, scale), 7.0 * 250.0 + 3.0 * 20.0 + 3.0 * 10.0, 1e-6, "cut area after the edit");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The dimensions of a DXF as (drawn length, length its style measures).
fn dxf_dimensions(path: &Path) -> Vec<(f64, f64)> {
    let dxf = dxf::Drawing::load_file(path).unwrap();
    let factor = |name: &str| {
        dxf.dim_styles().find(|s| s.name == name).map_or(1.0, |s| s.dimension_linear_measurement_scale_factor)
    };
    dxf.entities()
        .filter_map(|e| match &e.specific {
            dxf::entities::EntityType::RotatedDimension(d) => {
                let (p, q) = (&d.definition_point_2, &d.definition_point_3);
                let angle = d.rotation_angle.to_radians();
                let drawn = ((q.x - p.x) * angle.cos() + (q.y - p.y) * angle.sin()).abs();
                Some((drawn, drawn * factor(&d.dimension_base.dimension_style_name)))
            }
            _ => None,
        })
        .collect()
}

fn set_values(path: &Path) -> Vec<f64> {
    let (drawing, _) = soliddraft_drawing::file::read(path).unwrap();
    drawing.sheets[0]
        .annotations
        .iter()
        .filter_map(|a| match &a.body {
            AnnotationKind::DimensionSet(set) => Some(set.values.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn break_view_of_a_long_shaft_keeps_its_full_length() {
    let dir = scratch("break-view");
    let custom = Command::new(env!("CARGO_BIN_EXE_soliddraft"))
        .args(["draw", "shaft.json", "--break", "front:60,0,0:180,0,0:h:0", "--out", "bad.dwdrw"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(!custom.status.success());
    assert!(!dir.join("bad.dwdrw").exists());
    let args = [
        "draw",
        "shaft.json",
        "--views",
        "front",
        "--dims",
        "none",
        "--break",
        "front:50,0,0:150,0,0:h",
        "--break",
        "front:60,0,0:180,0,0:h",
        "--baseline",
        "front:h:-20,0,0;260,0,0",
        "--out",
        "shaft.dwdrw",
        "--out",
        "shaft.dxf",
        "--out",
        "shaft.pdf",
        "--json",
    ];
    soliddraft(&dir, &args);
    let (view, scale) = front(&dir.join("shaft.dwdrw"));
    let [gap] = view.geometry.breaks[..] else { panic!("one break: {:?}", view.geometry.breaks) };
    near(gap.gap, 6.0, 1e-12, "gap");
    // 280 mm long, 120 mm left out, closed up to 6 mm.
    let [x0, _, x1, _] = view.geometry.bbox;
    near(x1 - x0, (280.0 - 120.0) * scale + 6.0, 1e-3, "drawn length");
    // Nothing is drawn inside the gap.
    for (u, _) in ends(&view, None) {
        assert!(!(u > gap.at + 1e-6 && u < gap.at + gap.gap - 1e-6), "line work in the gap at {u}");
    }
    // The pieces on either side keep their model references: the body's outline runs on
    // across the gap as one curve with one source.
    let body_sides: Vec<_> = view
        .geometry
        .curves
        .iter()
        .filter(|c| {
            let us: Vec<f64> = c.path.elements().iter().filter_map(|e| e.end_point()).map(|p| p.x).collect();
            us.iter().any(|u| *u < gap.at) && us.iter().any(|u| *u > gap.at + gap.gap)
        })
        .collect();
    assert!(!body_sides.is_empty(), "the body's outline runs on across the gap as one curve");
    // The dimension across the break reads the model's length on the sheet and in the
    // native DXF, though it is drawn shorter.
    assert_eq!(set_values(&dir.join("shaft.dwdrw")), [280.0]);
    let dims = dxf_dimensions(&dir.join("shaft.dxf"));
    let (drawn, measured) = *dims.first().expect("a DIMENSION entity");
    near(measured, 280.0, 1e-6, "the DXF dimension measures");
    near(drawn, (280.0 - 120.0) * scale + 6.0, 1e-3, "the DXF dimension is drawn across the closed-up view");

    // The right journal grows 10 mm, and the bore with it so it still runs through: the
    // view and the dimension reflow, the break stays.
    set_body_length(&dir.join("shaft.json"), "right", 30.0);
    set_body_length(&dir.join("shaft.json"), "bore_tool", 292.0);
    let report: Value =
        serde_json::from_str(&soliddraft(&dir, &["draw", "--update", "shaft.dwdrw", "--out", "after.dxf", "--json"]))
            .unwrap();
    assert!(report.get("dangling_views").is_none(), "{report:#}");
    let (after, scale) = front(&dir.join("shaft.dwdrw"));
    assert_eq!(after.breaks, view.breaks, "the break reopens as it was");
    let [x0, _, x1, _] = after.geometry.bbox;
    near(x1 - x0, (290.0 - 120.0) * scale + 6.0, 1e-3, "drawn length after the edit");
    assert_eq!(set_values(&dir.join("shaft.dwdrw")), [290.0]);
    let (_, measured) = dxf_dimensions(&dir.join("after.dxf"))[0];
    near(measured, 290.0, 1e-6, "the DXF dimension after the edit");
    std::fs::remove_dir_all(&dir).unwrap();
}
