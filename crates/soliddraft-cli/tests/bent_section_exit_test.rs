//! Offset and aligned sections through `soliddraft draw`, as separate processes, on the
//! certified Catallax section path:
//!
//! - an offset section of the bracket through two staggered holes (hole 1 at 25, 23 and
//!   hole 4 at 115, 67): each cut shows the hole on its own leg, the cut faces add up to
//!   the true section areas and the jog draws no line;
//! - an aligned section of the five-hole flange through hole 1, the bore and hole 3:
//!   the second leg is revolved into the first leg's plane, so hole 3 shows at its true
//!   distance from the axis, and nothing at the pivot is drawn twice. Its inclined leg
//!   draws from the display mesh until Catallax certifies inclined sections;
//! - both survive a model edit and `draw --update`, reopen with their cutting lines, and
//!   export with one DXF HATCH per cut region, its loops those of the region;
//! - offset cuts through the flange's slot hatch the slot as a notch where a jog opens
//!   it and as two separate regions where a cut crosses it.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use soliddraft_drawing::{CurveKind, View, ViewKind};
use soliddraft_kernel_api::{Point2, Vec3};

#[path = "support/section_output.rs"]
mod section_output;
use section_output::{hatch_loops, svg_segments};

fn segment_covers(segments: &[[Point2; 2]], point: Point2) -> bool {
    segments.iter().any(|[a, b]| {
        let (du, dv) = (b.u - a.u, b.v - a.v);
        let length = du * du + dv * dv;
        if length <= 1e-12 {
            return false;
        }
        let t = (((point.u - a.u) * du + (point.v - a.v) * dv) / length).clamp(0.0, 1.0);
        (point.u - (a.u + t * du)).hypot(point.v - (a.v + t * dv)) <= 0.002
    })
}

fn pdf_segments(pdf: &lopdf::Document) -> Vec<[Point2; 2]> {
    let page = pdf.get_pages()[&1];
    let contents = lopdf::content::Content::decode(&pdf.get_page_content(page)).unwrap();
    stroked_segments(contents)
}

fn stroked_segments(contents: lopdf::content::Content) -> Vec<[Point2; 2]> {
    let mut segments = Vec::new();
    let mut pending = Vec::new();
    let mut last = None;
    let mut start = None;
    let mut transform = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let mut saved = Vec::new();
    let map = |[a, b, c, d, e, f]: [f64; 6], x, y| Point2::new(a * x + c * y + e, b * x + d * y + f);
    for op in contents.operations {
        let number = |i: usize| f64::from(op.operands[i].as_float().unwrap());
        let point = |i| map(transform, number(i), number(i + 1));
        if matches!(op.operator.as_str(), "h" | "s" | "b" | "b*")
            && let (Some(from), Some(to)) = (last, start)
        {
            pending.push([from, to]);
            last = start;
        }
        match op.operator.as_str() {
            "q" => saved.push(transform),
            "Q" => transform = saved.pop().unwrap(),
            "cm" => {
                let [a, b, c, d, _, _] = transform;
                let [g, h, i, j, k, l] = std::array::from_fn(number);
                let origin = map(transform, k, l);
                transform = [a * g + c * h, b * g + d * h, a * i + c * j, b * i + d * j, origin.u, origin.v];
            }
            "m" => {
                last = Some(point(0));
                start = last;
            }
            "l" => {
                let end = point(0);
                if let Some(from) = last.replace(end) {
                    pending.push([from, end]);
                }
            }
            "c" => last = Some(point(4)),
            "v" | "y" => last = Some(point(2)),
            "S" | "s" | "B" | "B*" | "b" | "b*" => {
                segments.append(&mut pending);
                last = None;
                start = None;
            }
            "n" | "f" | "F" | "f*" => {
                pending.clear();
                last = None;
                start = None;
            }
            _ => {}
        }
    }
    segments
}

#[test]
fn readback_uses_painted_pdf_paths_in_page_points() {
    let contents = lopdf::content::Content::decode(
        b"2 0 0 2 10 20 cm 1 4 m 2 4 l n q 1 0 0 1 3 1 cm 1 4 m 2 4 l S Q 1 7 m 2 7 l S",
    )
    .unwrap();
    assert_eq!(
        stroked_segments(contents),
        [[Point2::new(18.0, 30.0), Point2::new(20.0, 30.0)], [Point2::new(12.0, 34.0), Point2::new(14.0, 34.0)],]
    );
}

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

fn scratch(name: &str, recipe: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(recipes_dir().join(recipe), dir.join(recipe)).unwrap();
    dir
}

/// The bent section of the drawing's first sheet, and its scale.
fn bent_section(path: &Path, points: usize) -> (View, f64) {
    let (drawing, _) = soliddraft_drawing::file::read(path).expect("the drawing opens");
    let sheet = &drawing.sheets[0];
    let section = sheet
        .views
        .iter()
        .find(|v| matches!(&v.kind, ViewKind::Section { line, .. } if line.len() == points))
        .unwrap_or_else(|| panic!("no section with a {points}-point cutting line"))
        .clone();
    let ViewKind::Section { line, model_line, .. } = &section.kind else { unreachable!() };
    assert_eq!(model_line.len(), line.len(), "the cut keeps its model points");
    let scale = soliddraft_drawing::project::effective_scale(sheet, &section);
    (section, scale)
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

/// The hatched area in model square millimetres.
fn cut_area(view: &View, scale: f64) -> f64 {
    let area: f64 = view
        .geometry
        .hatch
        .iter()
        .map(|h| ring_area(&h.outer) - h.holes.iter().map(|r| ring_area(r)).sum::<f64>())
        .sum();
    area / (scale * scale)
}

/// The centre, along the sheet's X, of the section curves cut from faces of the hole of
/// radius `r` on the axis at `(x, y)`: faces whose fingerprint lies within the hole.
/// `None` when the cut misses the hole.
fn hole_cut_at(view: &View, x: f64, y: f64, r: f64) -> Option<f64> {
    let us: Vec<f64> = view
        .geometry
        .curves
        .iter()
        .filter(|c| c.kind == CurveKind::Section)
        .filter(|c| {
            let p = c.source.fingerprint.point;
            (p.x - x).hypot(p.y - y) <= r + 1e-6
        })
        .flat_map(|c| c.path.elements().iter().filter_map(|e| e.end_point()).map(|p| p.x).collect::<Vec<_>>())
        .collect();
    let (lo, hi) = us.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), u| (lo.min(*u), hi.max(*u)));
    (!us.is_empty()).then_some((lo + hi) / 2.0)
}

fn near(actual: f64, expected: f64, tol: f64, what: &str) {
    assert!((actual - expected).abs() <= tol, "{what}: {actual} is not {expected}");
}

/// Straight visible pieces of the view, as segments in view millimetres.
fn visible_segments(view: &View) -> Vec<[(f64, f64); 2]> {
    let mut out = Vec::new();
    for c in view.geometry.curves.iter().filter(|c| c.visible && !c.path.to_svg().contains(['C', 'Q'])) {
        let points: Vec<(f64, f64)> =
            c.path.elements().iter().filter_map(|e| e.end_point()).map(|p| (p.x, p.y)).collect();
        out.extend(points.windows(2).map(|w| [w[0], w[1]]));
    }
    out
}

/// Each cut keeps to its own stretch: no visible line runs across the joint at sheet X
/// `joint`, where a jog or the pivot meets the next cut, and no line along the joint is
/// drawn twice. Curved pieces are checked by their ends' sides.
fn assert_joint_drawn_once(view: &View, joint: f64) {
    for c in view.geometry.curves.iter().filter(|c| c.visible) {
        let xs: Vec<f64> = c.path.elements().iter().filter_map(|e| e.end_point()).map(|p| p.x).collect();
        let (lo, hi) = xs.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(*x), hi.max(*x)));
        assert!(
            !(lo < joint - 1e-6 && hi > joint + 1e-6),
            "a visible line runs across the joint at {joint}: {}",
            c.path.to_svg()
        );
    }
    let on_joint: Vec<(f64, f64)> = visible_segments(view)
        .into_iter()
        .filter(|[a, b]| (a.0 - joint).abs() < 1e-6 && (b.0 - joint).abs() < 1e-6)
        .map(|[a, b]| (a.1.min(b.1), a.1.max(b.1)))
        .collect();
    for (i, a) in on_joint.iter().enumerate() {
        for b in &on_joint[i + 1..] {
            assert!(a.1.min(b.1) - a.0.max(b.0) < 1e-6, "a line on the joint is drawn twice: {a:?} and {b:?}");
        }
    }
}

/// Sets `field` of recipe step `id` and writes the recipe back.
fn edit_recipe(path: &Path, id: &str, field: &str, value: f64) {
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let step = v["steps"].as_array_mut().unwrap().iter_mut().find(|s| s["id"] == id).unwrap();
    step[field] = value.into();
    std::fs::write(path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

#[test]
fn offset_section_through_staggered_bracket_holes() {
    let dir = scratch("offset-section", "bracket.json");
    let args = [
        "draw",
        "bracket.json",
        "--template",
        "iso-a3",
        "--offset-section",
        "top:25,23,10;70,23,10;70,67,10;115,67,10",
        "--out",
        "bracket.dwdrw",
        "--out",
        "bracket.svg",
        "--out",
        "bracket.pdf",
        "--out",
        "bracket.dxf",
        "--json",
    ];
    let report: Value = serde_json::from_str(&soliddraft(&dir, &args)).unwrap();
    assert_eq!(report["views"], 5, "four standard views and the section: {report:#}");
    let (section, scale) = bent_section(&dir.join("bracket.dwdrw"), 4);
    assert!(section.has_saved_catallax_linework(), "certified line work, which an export writes");
    let ViewKind::Section { model_line, label, .. } = &section.kind else { unreachable!() };
    // The jog runs at x = 70 from y = 23 to y = 67, as asked.
    for (p, (x, y)) in model_line[1..3].iter().zip([(70.0, 23.0), (70.0, 67.0)]) {
        near(p.x, x, 1e-9, "jog x");
        near(p.y, y, 1e-9, "jog y");
    }
    // Each leg cuts its own hole: hole 1 on the first leg, hole 4 on the second. A
    // straight cut along either leg would cut hole 3 or hole 2 instead.
    let first = hole_cut_at(&section, 25.0, 23.0, 4.5).expect("the first leg cuts hole 1");
    let second = hole_cut_at(&section, 115.0, 67.0, 4.5).expect("the second leg cuts hole 4");
    assert!(hole_cut_at(&section, 115.0, 23.0, 4.5).is_none(), "hole 3 lies off the cut");
    assert!(hole_cut_at(&section, 25.0, 67.0, 4.5).is_none(), "hole 2 lies off the cut");
    near((second - first).abs(), 90.0 * scale, 1e-6, "hole 1 to hole 4 along the cut");
    // True section areas: on y = 23 for x < 70, the flange (10 x 60) and the plate
    // (60 x 10) less hole 1 (9 x 10); on y = 67 for x >= 70, the plate (50 x 10) less
    // hole 4.
    near(cut_area(&section, scale), 1110.0 + 410.0, 1e-6, "cut area");
    // The jog at x = 70 lies 45 mm along the cut from hole 1.
    assert_joint_drawn_once(&section, first + (second - first) * 0.5);
    // Exported with one HATCH per cut region, loop for loop, with the letter on the sheet.
    assert_dxf_hatches_match(&dir.join("bracket.dxf"), &section);
    let svg = std::fs::read_to_string(dir.join("bracket.svg")).unwrap();
    assert!(svg.contains(&format!(">SECTION {label}-{label}</text>")), "the section's caption");
    let (saved, _) = soliddraft_drawing::file::read(&dir.join("bracket.dwdrw")).unwrap();
    let ViewKind::Section { parent, .. } = section.kind else { unreachable!() };
    let parent = saved.view(parent).unwrap();
    let height = saved.sheets[0].size.height_mm;
    let svg_visible = svg_segments(&svg, "VISIBLE", height);
    let svg_cut = svg_segments(&svg, "CUTTING", height);
    let pdf = lopdf::Document::load(dir.join("bracket.pdf")).expect("the PDF loads");
    assert_eq!(pdf.get_pages().len(), 1);
    let pdf_lines = pdf_segments(&pdf);
    let page_point = |p: Point2| Point2::new(p.u * 72.0 / 25.4, p.v * 72.0 / 25.4);
    let sheet_point = |view: &View, at| {
        let p = view.geometry.transform.unwrap().point(at);
        Point2::new(view.position.u + p.u, view.position.v + p.v)
    };
    for (x, y) in [(25.0_f64, 23.0), (115.0, 67.0)] {
        for wall in [x - 4.5, x + 4.5] {
            let on = sheet_point(&section, Vec3::new(wall, y, 5.0));
            assert!(segment_covers(&svg_visible, on), "SVG hole wall at {on:?}");
            assert!(segment_covers(&pdf_lines, page_point(on)), "PDF hole wall at {on:?}");
        }
    }
    let jog = sheet_point(parent, Vec3::new(70.0, 45.0, 10.0));
    assert!(segment_covers(&svg_cut, jog), "the SVG trace crosses the fixed jog");
    assert!(segment_covers(&pdf_lines, page_point(jog)), "the PDF trace crosses the fixed jog");

    // The flange gets taller: the drawing rebuilds the same cut against the new part.
    edit_recipe(&dir.join("bracket.json"), "flange", "distance", 70.0);
    let report: Value = serde_json::from_str(&soliddraft(
        &dir,
        &["draw", "--update", "bracket.dwdrw", "--out", "after.svg", "--out", "after.dxf", "--json"],
    ))
    .unwrap();
    assert!(report.get("dangling_views").is_none(), "{report:#}");
    let (after, scale) = bent_section(&dir.join("bracket.dwdrw"), 4);
    assert!(after.has_saved_catallax_linework());
    let ViewKind::Section { model_line: kept, .. } = &after.kind else { unreachable!() };
    assert_eq!(kept, model_line, "the cut stays where it was on the part");
    near(cut_area(&after, scale), 1520.0 + 100.0, 1e-6, "cut area after the flange grew 10 mm");
    assert!(hole_cut_at(&after, 25.0, 23.0, 4.5).is_some() && hole_cut_at(&after, 115.0, 67.0, 4.5).is_some());
    assert_dxf_hatches_match(&dir.join("after.dxf"), &after);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A region's loops as corners in model millimetres: counterclockwise from the lowest,
/// then leftmost, corner, with points along a straight side left out.
fn corners(ring: &[(f64, f64)]) -> Vec<[f64; 2]> {
    let n = ring.len();
    let turns = |i: usize| {
        let (a, b, c) = (ring[(i + n - 1) % n], ring[i], ring[(i + 1) % n]);
        ((b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0)).abs() > 1e-9
    };
    let mut out: Vec<(f64, f64)> = (0..n).filter(|&i| turns(i)).map(|i| ring[i]).collect();
    let twice: f64 = (0..out.len())
        .map(|i| {
            let (a, b) = (out[i], out[(i + 1) % out.len()]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    if twice < 0.0 {
        out.reverse();
    }
    let first = (0..out.len()).min_by(|&i, &j| out[i].1.total_cmp(&out[j].1).then(out[i].0.total_cmp(&out[j].0)));
    out.rotate_left(first.unwrap_or(0));
    out.into_iter().map(|(u, v)| [(u * 1e3).round() / 1e3, (v * 1e3).round() / 1e3]).collect()
}

/// The hatch regions of a section, outer loop first, as corners in model millimetres
/// along the cut and up its depth, from the regions' lowest corner. The view may show
/// either axis either way round, so each reading is tried; the one that matches
/// `expected` is returned, or the first when none does.
fn hatch_regions(regions: &[Vec<Vec<Point2>>], scale: f64, expected: &[Vec<Vec<[f64; 2]>>]) -> Vec<Vec<Vec<[f64; 2]>>> {
    let all = regions.iter().flatten().flatten();
    let (lo, hi) = all.fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(lo, hi), p| {
        ([lo[0].min(p.u), lo[1].min(p.v)], [hi[0].max(p.u), hi[1].max(p.v)])
    });
    let extent = [(hi[0] - lo[0]) / scale, (hi[1] - lo[1]) / scale];
    let mut first = None;
    for reading in 0..8 {
        let (swap, flip_a, flip_b) = (reading & 1 != 0, reading & 2 != 0, reading & 4 != 0);
        let mut out: Vec<Vec<Vec<[f64; 2]>>> = regions
            .iter()
            .map(|loops| {
                let mut loops: Vec<Vec<[f64; 2]>> = loops
                    .iter()
                    .map(|ring| {
                        let points: Vec<(f64, f64)> = ring
                            .iter()
                            .map(|p| {
                                let (u, v) = ((p.u - lo[0]) / scale, (p.v - lo[1]) / scale);
                                let (eu, ev) = (extent[0], extent[1]);
                                let (mut a, mut b, ea, eb) = if swap { (v, u, ev, eu) } else { (u, v, eu, ev) };
                                if flip_a {
                                    a = ea - a;
                                }
                                if flip_b {
                                    b = eb - b;
                                }
                                (a, b)
                            })
                            .collect();
                        corners(&points)
                    })
                    .collect();
                if loops.len() > 1 {
                    loops[1..].sort_by(|a, b| a.partial_cmp(b).unwrap());
                }
                loops
            })
            .collect();
        out.sort_by(|a, b| a.partial_cmp(b).unwrap());
        if out == expected {
            return out;
        }
        first.get_or_insert(out);
    }
    first.unwrap_or_default()
}

fn polygon_loops(view: &View) -> Vec<Vec<Vec<Point2>>> {
    view.geometry.hatch.iter().map(|h| std::iter::once(&h.outer).chain(&h.holes).cloned().collect()).collect()
}

/// The DXF's HATCH boundaries are the view's hatch regions on the sheet, loop for loop:
/// the outer boundary first and marked external, its holes after it.
fn assert_dxf_hatches_match(dxf: &Path, view: &View) {
    let read = hatch_loops(&std::fs::read(dxf).unwrap());
    let o = view.position;
    let mut left: Vec<Vec<Vec<Point2>>> = polygon_loops(view)
        .into_iter()
        .map(|loops| loops.into_iter().map(|r| r.iter().map(|p| Point2::new(p.u + o.u, p.v + o.v)).collect()).collect())
        .collect();
    assert_eq!(read.len(), left.len(), "one HATCH per cut region");
    for hatch in read {
        let flags: Vec<bool> = hatch.iter().map(|(external, _)| *external).collect();
        assert!(flags.first() == Some(&true) && flags[1..].iter().all(|f| !f), "external loop first: {flags:?}");
        let loops: Vec<Vec<Point2>> = hatch.into_iter().map(|(_, points)| points).collect();
        let same = |a: &Vec<Vec<Point2>>| {
            a.len() == loops.len()
                && a.iter().zip(&loops).all(|(x, y)| {
                    x.len() == y.len()
                        && x.iter().zip(y).all(|(p, q)| (p.u - q.u).abs() < 1e-5 && (p.v - q.v).abs() < 1e-5)
                })
        };
        let at = left.iter().position(same).unwrap_or_else(|| panic!("a HATCH matches no cut region: {loops:?}"));
        left.remove(at);
    }
}

/// Offset sections through the flange's 40 x 12 slot, whose cut at x = 5 is the flange
/// (90 x 60 along the cut and up) with the slot (y 25 to 65, z 25 to 37) as a hole. A
/// jog at y = 45 opens the slot into a notch in the first cut's outline; a middle cut
/// from y = 30 to 50 leaves the material above and below the slot as two separate
/// regions. The plate (10 deep) is cut at x = 50.
#[test]
fn offset_cuts_through_the_flange_slot_hatch_notches_and_islands() {
    let dir = scratch("offset-slot", "bracket.json");
    let rect = |s0: f64, s1: f64, z0: f64, z1: f64| vec![vec![[s0, z0], [s1, z0], [s1, z1], [s0, z1]]];
    let cases = [
        (
            "notch",
            "top:5,0,10;5,45,10;50,45,10;50,90,10",
            4,
            vec![
                vec![vec![
                    [0.0, 0.0],
                    [45.0, 0.0],
                    [45.0, 25.0],
                    [25.0, 25.0],
                    [25.0, 37.0],
                    [45.0, 37.0],
                    [45.0, 60.0],
                    [0.0, 60.0],
                ]],
                rect(45.0, 90.0, 0.0, 10.0),
            ],
        ),
        (
            "islands",
            "top:50,0,10;50,30,10;5,30,10;5,50,10;50,50,10;50,90,10",
            6,
            vec![
                rect(0.0, 30.0, 0.0, 10.0),
                rect(30.0, 50.0, 0.0, 25.0),
                rect(30.0, 50.0, 37.0, 60.0),
                rect(50.0, 90.0, 0.0, 10.0),
            ],
        ),
    ];
    for (name, through, points, mut expected) in cases {
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let drawing = format!("{name}.dwdrw");
        let dxf = format!("{name}.dxf");
        let args =
            ["draw", "bracket.json", "--offset-section", through, "--out", drawing.as_str(), "--out", dxf.as_str()];
        soliddraft(&dir, &args);
        let (section, scale) = bent_section(&dir.join(&drawing), points);
        assert!(section.has_saved_catallax_linework(), "{name}");
        let regions = hatch_regions(&polygon_loops(&section), scale, &expected);
        assert_eq!(regions, expected, "{name}: the hatch regions along the cut and up");
        assert_dxf_hatches_match(&dir.join(&dxf), &section);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The flange's second leg runs at 144 degrees. Catallax's certified section does not yet
/// cut an inclined plane through its cylinders within its work allowance (the kernel
/// fix is its own Catallax task); until it does, the view shows the section cut from the
/// display mesh. The geometry checks hold at the mesh's chord tolerance.
#[test]
fn aligned_section_through_a_five_hole_flange() {
    let dir = scratch("aligned-section", "flange.json");
    let hole3 = (-28.315594803123158, 20.572483830236564);
    let through = format!("top:35,0,10;0,0,10;{},{},10", hole3.0, hole3.1);
    let args = [
        "draw",
        "flange.json",
        "--template",
        "iso-a3",
        "--aligned-section",
        through.as_str(),
        "--out",
        "flange.dwdrw",
        "--json",
    ];
    let out = Command::new(env!("CARGO_BIN_EXE_soliddraft"))
        .args(args)
        .current_dir(&dir)
        .env("RUST_LOG", "soliddraft_drawing=warn")
        .output()
        .unwrap();
    let log = String::from_utf8(out.stderr).unwrap();
    assert!(out.status.success(), "{log}");
    assert!(log.contains("showing the display mesh"), "{log}");
    assert_neutral(&String::from_utf8(out.stdout.clone()).unwrap());
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["views"], 4, "{report:#}");
    let (section, scale) = bent_section(&dir.join("flange.dwdrw"), 3);
    assert!(section.geometry.display_only && !section.geometry.unproved);
    assert!(!section.has_saved_catallax_linework());
    // Hole 3 is revolved onto the first leg's line: it shows 35 mm from the axis, as
    // hole 1 does, so the two cut holes are 70 mm apart. Unrevolved, its projection
    // would sit 28.3 mm from the axis.
    let tol = 0.05 * scale;
    let first = hole_cut_at(&section, 35.0, 0.0, 5.0).expect("the first leg cuts hole 1");
    let revolved = hole_cut_at(&section, hole3.0, hole3.1, 5.0).expect("the second leg cuts hole 3");
    near((revolved - first).abs(), 70.0 * scale, tol, "hole 1 to revolved hole 3");
    for (x, y) in [(10.815594803123162, 33.28697807033037), (-28.315594803123165, -20.572483830236557)] {
        assert!(hole_cut_at(&section, x, y, 5.0).is_none(), "the cut misses the hole at {x}, {y}");
    }
    // True areas on each leg: the hub wall (10 x 25) and the disk beside it (10 x 10),
    // then the disk beyond the hole (10 x 10). A straight cut through hole 1 would
    // find no hole on the far side and hatch 100 mm^2 more.
    near(cut_area(&section, scale), 2.0 * (250.0 + 100.0 + 100.0), 9.0, "cut area");
    // The pivot lies halfway between the two holes, each 35 mm from the axis. The far
    // half of the hub's top rim shows on both legs; each leg draws only its own half.
    assert_joint_drawn_once(&section, (first + revolved) / 2.0);

    // The reason goes to the log on stderr only when RUST_LOG asks for it; the summary on
    // stdout reads as for any drawing either way.
    let print = ["draw", "flange.json", "--aligned-section", through.as_str(), "--out", "flange.svg"];
    for rust_log in [Some("warn"), None] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_soliddraft"));
        command.args(print).current_dir(&dir);
        match rust_log {
            Some(filter) => command.env("RUST_LOG", filter),
            None => command.env_remove("RUST_LOG"),
        };
        let out = command.output().unwrap();
        let (stdout, log) = (String::from_utf8(out.stdout).unwrap(), String::from_utf8(out.stderr).unwrap());
        assert!(out.status.success(), "{log}");
        assert_eq!(log.contains("showing the display mesh"), rust_log.is_some(), "{log}");
        assert!(stdout.contains(": 4 views at "), "{stdout}");
        assert_neutral(&stdout);
    }
    let svg = std::fs::read_to_string(dir.join("flange.svg")).unwrap();
    assert!(svg.contains(r#"<g id="VISIBLE""#), "the other views print");
    assert!(!svg.contains(r#"<g id="HATCH""#), "the display section's hatch stays off the print");
    assert!(!svg.contains(r#"<g id="CUTTING""#), "the display section's cutting line stays off the print");
    let (drawing, _) = soliddraft_drawing::file::read(&dir.join("flange.dwdrw")).unwrap();
    let list = soliddraft_drawing::display::layout_sheet(&drawing, 0);
    assert!(
        list.items
            .iter()
            .any(|i| matches!(i.owner(), soliddraft_drawing::display::Owner::Curve(id, _) if id == section.id))
    );
    assert!(
        list.printable()
            .all(|i| !matches!(i.owner(), soliddraft_drawing::display::Owner::Curve(id, _) if id == section.id))
    );

    // A taller hub: both legs rebuild with it.
    edit_recipe(&dir.join("flange.json"), "hub", "distance", 30.0);
    let report: Value =
        serde_json::from_str(&soliddraft(&dir, &["draw", "--update", "flange.dwdrw", "--json"])).unwrap();
    assert!(report.get("dangling_views").is_none(), "{report:#}");
    let (after, scale) = bent_section(&dir.join("flange.dwdrw"), 3);
    // The bore tool still ends at z = 26, so the taller hub closes over the bore: each leg
    // gains the 10 x 4 mm cap between the bore's edge and the axis.
    near(cut_area(&after, scale), 2.0 * (300.0 + 40.0 + 100.0 + 100.0), 9.0, "cut area after the hub grew 5 mm");
    let first = hole_cut_at(&after, 35.0, 0.0, 5.0).unwrap();
    let revolved = hole_cut_at(&after, hole3.0, hole3.1, 5.0).unwrap();
    assert_joint_drawn_once(&after, (first + revolved) / 2.0);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Standard output never says how a view's line work was made.
fn assert_neutral(stdout: &str) {
    let lower = stdout.to_lowercase();
    for word in ["mesh", "certif", "proof", "proved", "refus", "fallback"] {
        assert!(!lower.contains(word), "stdout mentions {word}: {stdout}");
    }
}

#[test]
fn bent_sections_refuse_malformed_cutting_lines() {
    let dir = scratch("bent-refusals", "bracket.json");
    for (flag, value, says) in [
        ("--offset-section", "top:25,23,10;70,23,10;70,67,10", "even number of points"),
        ("--aligned-section", "top:25,23,10;70,23,10", "three points"),
        ("--offset-section", "top:25,23,10;70,23,10;80,67,10;115,67,10", "jogs square"),
        ("--aligned-section", "top:25,23,10;70,23,10;115,23,10", "must not be in line"),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_soliddraft"))
            .args(["draw", "bracket.json", flag, value, "--out", "refused.dwdrw"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(!out.status.success(), "{flag} {value} was accepted");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(says), "{flag} {value}: {stderr}");
        assert!(!dir.join("refused.dwdrw").exists(), "a refused drawing was written");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_oblique_offset_keeps_its_square_jog_after_extension_and_reopen() {
    let dir = scratch("oblique-offset", "bracket.json");
    soliddraft(
        &dir,
        &[
            "draw",
            "bracket.json",
            "--break",
            "top:20,0,0:40,0,0:h",
            "--offset-section",
            "top:10,10,0;40,20,0;38,26,0;68,36,0",
            "--out",
            "oblique.dwdrw",
            "--out",
            "oblique.svg",
        ],
    );
    let (section, _) = bent_section(&dir.join("oblique.dwdrw"), 4);
    let ViewKind::Section { model_line, .. } = &section.kind else { unreachable!() };
    let leg = model_line[1] - model_line[0];
    let jog = model_line[2] - model_line[1];
    near(leg.dot(jog), 0.0, 1e-8, "the extended leg is square to its jog");
    let svg = std::fs::read_to_string(dir.join("oblique.svg")).unwrap();
    let (saved, _) = soliddraft_drawing::file::read(&dir.join("oblique.dwdrw")).unwrap();
    let ViewKind::Section { parent, .. } = section.kind else { unreachable!() };
    let parent = saved.view(parent).unwrap();
    let trace = svg_segments(&svg, "CUTTING", saved.sheets[0].size.height_mm);
    let xf = parent.geometry.transform.unwrap();
    for point in [Vec3::new(16.0, 12.0, 0.0), Vec3::new(56.0, 32.0, 0.0)] {
        let p = xf.point(point);
        let on = Point2::new(parent.position.u + p.u, parent.position.v + p.v);
        assert!(segment_covers(&trace, on), "the SVG trace follows the retained oblique leg at {point:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
