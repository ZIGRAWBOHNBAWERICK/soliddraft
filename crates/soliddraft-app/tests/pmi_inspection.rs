//! The inspection files of the manufacturing package. A reader built on quick-xml, which
//! follows the package relationships and shares no code with the writer, checks the AS9102
//! workbook against the CSV for the P1 bracket and for a part with the newer PMI kinds. The
//! ribbon's action writes the same bytes as the tool.
//!
//! Set `SOLIDDRAFT_PMI_PROOF_DIR` to keep the files for `crates/soliddraft-model/tests/check-as9102.py`.

#[path = "support/bracket_pmi.rs"]
mod bracket;

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use bracket::{annotate, build, call, state};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::state::AppState;
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_api::SurfaceKind::{Cylinder, Plane};

/// Worksheets by name, in workbook order: each a map from a reference such as "B3" to
/// the cell's text, numbers in their shortest form.
type Workbook = Vec<(String, BTreeMap<String, String>)>;

fn attribute(tag: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    tag.attributes()
        .map(Result::unwrap)
        .find(|a| a.key.as_ref() == name)
        .map(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).unwrap().into_owned())
}

/// Each start tag of `xml` with its attributes, by qualified name.
fn elements(xml: &str) -> Vec<(String, BTreeMap<String, String>)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut out = Vec::new();
    loop {
        match reader.read_event().unwrap() {
            Event::Start(tag) => {
                let attributes = tag
                    .attributes()
                    .map(Result::unwrap)
                    .map(|a| {
                        let value = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).unwrap().into_owned();
                        (String::from_utf8(a.key.as_ref().to_vec()).unwrap(), value)
                    })
                    .collect();
                out.push((String::from_utf8(tag.name().as_ref().to_vec()).unwrap(), attributes));
            }
            Event::Eof => return out,
            _ => {}
        }
    }
}

/// The cells of one worksheet, with inline strings decoded as a spreadsheet reads them.
fn cells(xml: &str) -> BTreeMap<String, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut out = BTreeMap::new();
    let mut cell: Option<(String, Option<String>, String)> = None;
    let mut reading = false;
    loop {
        match reader.read_event().unwrap() {
            Event::Start(tag) => match tag.local_name().as_ref() {
                b"c" => cell = Some((attribute(&tag, b"r").unwrap(), attribute(&tag, b"t"), String::new())),
                b"t" | b"v" => reading = cell.is_some(),
                _ => {}
            },
            Event::End(tag) => match tag.local_name().as_ref() {
                b"c" => {
                    let (reference, kind, text) = cell.take().unwrap();
                    let value = match kind.as_deref() {
                        Some("inlineStr") => decode_escapes(&text),
                        None => text.parse::<f64>().unwrap().to_string(),
                        Some(other) => panic!("{reference}: unexpected cell type {other}"),
                    };
                    assert!(out.insert(reference.clone(), value).is_none(), "{reference} twice");
                }
                b"t" | b"v" => reading = false,
                _ => {}
            },
            Event::Text(text) if reading => cell.as_mut().unwrap().2.push_str(&text.xml10_content().unwrap()),
            Event::GeneralRef(entity) if reading => {
                let c = entity.resolve_char_ref().unwrap().unwrap_or_else(|| match entity.decode().unwrap().as_ref() {
                    "amp" => '&',
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    other => panic!("unknown entity {other}"),
                });
                cell.as_mut().unwrap().2.push(c);
            }
            Event::Eof => return out,
            _ => {}
        }
    }
}

/// OOXML `_xHHHH_` character escapes.
fn decode_escapes(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("_x") {
        let hex = rest.get(start + 2..start + 6).filter(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()));
        match hex.filter(|_| rest.as_bytes().get(start + 6) == Some(&b'_')) {
            Some(hex) => {
                out.push_str(&rest[..start]);
                out.push(char::from_u32(u32::from_str_radix(hex, 16).unwrap()).unwrap());
                rest = &rest[start + 7..];
            }
            None => {
                out.push_str(&rest[..start + 2]);
                rest = &rest[start + 2..];
            }
        }
    }
    out + rest
}

fn read_workbook(path: &Path) -> Workbook {
    let mut zip = zip::ZipArchive::new(Cursor::new(std::fs::read(path).unwrap())).unwrap();
    let mut part = |name: &str| {
        let mut text = String::new();
        zip.by_name(name).unwrap_or_else(|_| panic!("no part {name}")).read_to_string(&mut text).unwrap();
        text
    };
    let types = elements(&part("[Content_Types].xml"));
    let declared = |name: &str, kind: &str| {
        types.iter().any(|(tag, a)| {
            tag == "Override" && a["PartName"] == format!("/{name}") && a["ContentType"].ends_with(kind)
        })
    };
    let root = elements(&part("_rels/.rels"))
        .into_iter()
        .find(|(tag, a)| tag == "Relationship" && a["Type"].ends_with("/officeDocument"))
        .unwrap()
        .1["Target"]
        .clone();
    assert!(declared(&root, ".sheet.main+xml"));
    let (folder, file) = root.rsplit_once('/').unwrap();
    let relations: BTreeMap<String, (String, String)> = elements(&part(&format!("{folder}/_rels/{file}.rels")))
        .into_iter()
        .filter(|(tag, _)| tag == "Relationship")
        .map(|(_, a)| (a["Id"].clone(), (a["Type"].clone(), format!("{folder}/{}", a["Target"]))))
        .collect();
    assert!(relations.values().any(|(kind, target)| kind.ends_with("/styles") && declared(target, ".styles+xml")));
    elements(&part(&root))
        .into_iter()
        .filter(|(tag, _)| tag == "sheet")
        .map(|(_, a)| {
            let (kind, target) = &relations[&a["r:id"]];
            assert!(kind.ends_with("/worksheet") && declared(target, ".worksheet+xml"), "{target}");
            (a["name"].clone(), cells(&part(target)))
        })
        .collect()
}

/// RFC 4180 records.
fn read_csv(path: &Path) -> Vec<Vec<String>> {
    let text = std::fs::read_to_string(path).unwrap();
    let (mut records, mut record, mut field, mut quoted) = (Vec::new(), Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.next_if_eq(&'"').is_some() => field.push('"'),
            '"' => quoted = !quoted,
            ',' if !quoted => record.push(std::mem::take(&mut field)),
            '\r' if !quoted && chars.next_if_eq(&'\n').is_some() => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            c => field.push(c),
        }
    }
    assert!(field.is_empty() && record.is_empty() && !quoted, "a complete last record");
    records
}

/// Form 3 rows from the header row down, by column header; numbers in shortest form.
fn form3_rows(form: &BTreeMap<String, String>) -> Vec<BTreeMap<String, String>> {
    let columns: Vec<(String, String)> = (b'A'..=b'Z')
        .map(|c| char::from(c).to_string())
        .filter_map(|column| form.get(&format!("{column}7")).map(|name| (column, name.clone())))
        .collect();
    assert_eq!(columns.first().map(|(_, name)| name.as_str()), Some("5. Char. No."));
    (8..)
        .map_while(|row| {
            form.contains_key(&format!("A{row}")).then(|| {
                columns
                    .iter()
                    .map(|(column, name)| {
                        (name.clone(), form.get(&format!("{column}{row}")).cloned().unwrap_or_default())
                    })
                    .collect()
            })
        })
        .collect()
}

/// Checks that Form 3 holds the CSV's characteristics in order with the same values, that
/// every result and disposition is blank, and that all forms name the part. Returns the rows
/// by characteristic ID.
fn check_against_csv(
    workbook: &Path,
    part: &str,
    part_number: &str,
    revision: &str,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let sheets = read_workbook(workbook);
    assert_eq!(sheets.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), ["Form 1", "Form 2", "Form 3"]);
    for (name, sheet) in &sheets {
        let at = |cell: &str| sheet.get(cell).map_or("", String::as_str);
        assert_eq!((at("A2"), at("B2")), ("1. Part Number", "2. Part Name"), "{name}");
        assert_eq!((at("A3"), at("B3"), at("C3"), at("D3")), (part_number, part, "", ""), "{name}");
    }
    assert_eq!(sheets[0].1.get("B4").map_or("", String::as_str), revision);
    let records = read_csv(&workbook.with_extension("csv"));
    let header = &records[0];
    let csv = |record: &[String], name: &str| record[header.iter().position(|c| c == name).unwrap()].clone();
    let rows = form3_rows(&sheets[2].1);
    assert_eq!(rows.len(), records.len() - 1, "one Form 3 row per CSV characteristic");
    let number = |text: &str| (!text.is_empty()).then(|| text.parse::<f64>().unwrap());
    let json = |text: &str| -> Value { serde_json::from_str(if text.is_empty() { "[]" } else { text }).unwrap() };
    let mut by_id = BTreeMap::new();
    for (row, record) in rows.into_iter().zip(&records[1..]) {
        let id = csv(record, "characteristic_id");
        assert_eq!(row["5. Char. No."], id);
        assert_eq!((csv(record, "part"), csv(record, "revision")), (part.to_owned(), revision.to_owned()));
        assert!(csv(record, "actual").is_empty() && csv(record, "disposition").is_empty(), "{id}");
        for blank in ["9. Results", "11. Nonconformance Number", "Disposition"] {
            assert!(row[blank].is_empty(), "{id}: {blank} is an inspection result, not a requirement");
        }
        for (column, field) in [
            ("8. Requirement", "pmi_callout"),
            ("12. Additional Data / Comments", "problem"),
            ("Group ID", "group_id"),
            ("PMI Item", "pmi_item"),
            ("Kind", "kind"),
            ("Applicability", "applicability"),
            ("Unit", "unit"),
            ("Tolerance", "tolerance"),
            ("Fit Class", "fit_class"),
            ("Locating Datum", "locating_datum"),
        ] {
            assert_eq!(row[column], csv(record, field), "{id}: {column}");
        }
        for (column, field) in [
            ("Member", "member"),
            ("Nominal", "nominal"),
            ("Lower Limit", "lower_limit"),
            ("Upper Limit", "upper_limit"),
        ] {
            assert_eq!(number(&row[column]), number(&csv(record, field)), "{id}: {column}");
        }
        for (column, field) in [("Datum Order (JSON)", "datum_order"), ("Face References (JSON)", "face_refs")] {
            assert_eq!(json(&row[column]), json(&csv(record, field)), "{id}: {column}");
        }
        assert!(by_id.insert(id.clone(), row).is_none(), "{id} is unique");
    }
    by_id
}

fn letters(row: &BTreeMap<String, String>) -> Vec<String> {
    serde_json::from_str::<Vec<Value>>(&row["Datum Order (JSON)"])
        .unwrap()
        .iter()
        .map(|datum| datum["letter"].as_str().unwrap().to_owned())
        .collect()
}

fn proof_dir(name: &str) -> (PathBuf, Option<PathBuf>) {
    let scratch = std::env::temp_dir().join(format!("soliddraft-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let proof = std::env::var_os("SOLIDDRAFT_PMI_PROOF_DIR").map(PathBuf::from);
    if let Some(proof) = &proof {
        std::fs::create_dir_all(proof).unwrap();
    }
    (scratch, proof)
}

/// The ribbon's action and the tool with the same input write the same inspection bytes.
fn ribbon_agrees(s: &mut AppState, dir: &Path, stem: &str) {
    let ribbon = dir.join(format!("{stem}-ribbon.step"));
    let tool = dir.join(format!("{stem}-tool.step"));
    let summary = soliddraft_app::pmi::ribbon_package(s, &ribbon).unwrap();
    assert!(summary.starts_with("STEP, PDF, DXF, CSV and AS9102 workbook written"), "{summary}");
    assert!(summary.contains("characteristics to inspect"), "{summary}");
    call(s, "manufacturing_package", json!({ "path": tool }));
    for extension in ["csv", "xlsx"] {
        let (a, b) = (
            std::fs::read(ribbon.with_extension(extension)).unwrap(),
            std::fs::read(tool.with_extension(extension)).unwrap(),
        );
        assert!(a == b, "the ribbon and the tool write the same {extension}");
    }
    check_against_csv(&ribbon.with_extension("xlsx"), &s.doc.name, "", "");
}

#[test]
fn p1_package_writes_an_as9102_workbook_that_matches_its_csv() {
    let (dir, proof) = proof_dir("pmi-inspection-p1");
    let mut s = state("P1 bracket");
    build(&mut s);
    annotate(&mut s);

    let step = dir.join("p1-package.step");
    let report =
        call(&mut s, "manufacturing_package", json!({ "path": step, "part_number": "P1-100", "revision": "B" }));
    let inspection = &report["inspection"];
    assert_eq!(
        (inspection["part_number"].as_str(), inspection["revision"].as_str()),
        (Some("P1-100"), Some("B")),
        "{report}"
    );
    assert_eq!(inspection["unresolved"], json!([]), "every P1 item applies: {report}");
    let rows = check_against_csv(&step.with_extension("xlsx"), "P1 bracket", "P1-100", "B");
    assert_eq!(inspection["characteristics"], json!(rows.len()));
    let count = |applies: &str| rows.values().filter(|row| row["Applicability"] == applies).count();
    assert_eq!(
        (inspection["inspect"].as_u64(), inspection["reference"].as_u64()),
        (Some(count("inspect") as u64), Some(count("reference") as u64))
    );

    // A patterned hole: one diameter and one position per member, located by basic distances
    // from B and C in the frame's datum order.
    for member in 1..=4 {
        let diameter = &rows[&format!("pmi/Hole diameter/{member}/diameter")];
        assert_eq!(
            (diameter["Nominal"].as_str(), diameter["Lower Limit"].as_str(), diameter["Upper Limit"].as_str()),
            ("4", "4", "4.1")
        );
        let position = &rows[&format!("pmi/Hole position/{member}/position")];
        assert_eq!(
            (position["Applicability"].as_str(), position["Upper Limit"].as_str(), position["Unit"].as_str()),
            ("inspect", "0.2", "mm")
        );
        assert_eq!(letters(position), ["A", "B", "C"]);
        assert_eq!(position["Face References (JSON)"], diameter["Face References (JSON)"]);
        for datum in ["B", "C"] {
            let basic = &rows[&format!("pmi/Hole position/{member}/basic/{datum}")];
            assert_eq!(
                (basic["Applicability"].as_str(), basic["Tolerance"].as_str(), basic["Locating Datum"].as_str()),
                ("reference", "basic", datum)
            );
        }
    }
    let bore = &rows["pmi/Bore diameter/1/diameter"];
    assert_eq!(
        (bore["Fit Class"].as_str(), bore["Lower Limit"].as_str(), bore["8. Requirement"].as_str()),
        ("H7", "", "Ø8 H7")
    );
    assert_eq!(rows["pmi/Datum A/datum"]["Applicability"], "reference");

    ribbon_agrees(&mut s, &dir, "p1");
    if let Some(proof) = proof {
        for extension in ["csv", "xlsx"] {
            std::fs::copy(step.with_extension(extension), proof.join(format!("p1-package.{extension}"))).unwrap();
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn newer_kinds_reach_the_workbook_and_typed_frames_stop_the_package() {
    let (dir, proof) = proof_dir("pmi-inspection-new-kinds");
    let mut s = state("Plate, new kinds");
    call(
        &mut s,
        "sketch",
        json!({"plane": "top", "shapes": [{"shape": "rectangle", "center": [40, 20, 0], "width": 80, "height": 40}]}),
    );
    call(&mut s, "extrude", json!({"sketch": "Sketch1", "depth": 10, "flip": false}));
    let cap = "Boss-Extrude1 · End cap";
    call(
        &mut s,
        "hole",
        json!({"face": cap, "points": [[12, 20, 10]], "diameter": 4, "counterbore": {"diameter": 10, "depth": 3}}),
    );
    for x in [40, 62] {
        call(&mut s, "hole", json!({"face": cap, "points": [[x, 20, 10]], "diameter": 6}));
    }
    let top = bracket::face(&s, Plane, [None, None, Some(10.0)]);
    let bottom = bracket::face(&s, Plane, [None, None, Some(0.0)]);
    let left = bracket::face(&s, Plane, [Some(0.0), None, None]);
    let side = bracket::face(&s, Plane, [None, Some(0.0), None]);
    let pilot = bracket::face(&s, Cylinder, [Some(12.0), Some(20.0), Some(3.5)]);
    let head = bracket::face(&s, Cylinder, [Some(12.0), Some(20.0), Some(8.5)]);
    let first = bracket::face(&s, Cylinder, [Some(40.0), None, None]);
    let second = bracket::face(&s, Cylinder, [Some(62.0), None, None]);
    for (letter, face) in [("A", &top), ("B", &left), ("C", &side)] {
        call(&mut s, "pmi_add", json!({"kind": "datum", "letter": letter, "faces": [face]}));
    }
    for input in [
        json!({"kind": "hole", "name": "Counterbored", "pilot_faces": [pilot], "head_kind": "counterbore",
               "head_faces": [head], "through": true, "tolerance": "+0.1/0"}),
        json!({"kind": "width", "name": "Plate thickness", "flats": [[top], [bottom]], "tolerance": "\u{b1}0.1"}),
        json!({"kind": "basic_angle", "name": "Corner", "planes": [[top], [left]]}),
        json!({"kind": "datum_target", "name": "Top target", "target": {"datum": "A", "number": 1, "face": top,
               "point_mm": [40, 30, 10], "kind": {"shape": "point"}}}),
        json!({"kind": "position_control", "name": "Bore pattern", "features": [[first], [second]],
               "frame": {"frame": "composite",
                   "upper": {"value_mm": "0.10", "diameter_zone": true, "material": "maximum",
                             "datums": [{"letter": "A"}, {"letter": "B"}, {"letter": "C"}]},
                   "lower": {"value_mm": "0.04", "diameter_zone": true, "material": "least",
                             "datums": [{"letter": "A"}, {"letter": "B"}]}}}),
        json!({"kind": "position_control", "name": "Projection", "faces": [second],
               "frame": {"frame": "single", "segment": {"value_mm": "0.15", "diameter_zone": true, "datums": [{"letter": "A"}]},
                   "projected_zone": {"height_mm": "8", "direction": [0, 0, 1], "entry_faces": [top]}}}),
    ] {
        let added = call(&mut s, "pmi_add", input);
        assert!(added["added"].get("problem").is_none(), "{added}");
    }

    // The typed frames reach the plan with each segment's own datum order.
    let plan = soliddraft_model::pmi::inspection::InspectionPlan::from_document(s.doc.model(), Some("A"));
    let workbook = dir.join("new-kinds-plan.xlsx");
    std::fs::write(&workbook, plan.as9102_xlsx(Some("NK-1")).unwrap()).unwrap();
    let mut csv = Vec::new();
    plan.write_csv(&mut csv).unwrap();
    std::fs::write(workbook.with_extension("csv"), csv).unwrap();
    let rows = check_against_csv(&workbook, "Plate, new kinds", "NK-1", "A");
    for member in 1..=2 {
        let upper = &rows[&format!("pmi/Bore pattern/{member}/composite_position_upper")];
        let lower = &rows[&format!("pmi/Bore pattern/{member}/composite_position_lower")];
        assert_eq!(
            (upper["Upper Limit"].as_str(), letters(upper)),
            ("0.1", vec!["A".to_owned(), "B".into(), "C".into()])
        );
        assert_eq!((lower["Upper Limit"].as_str(), letters(lower)), ("0.04", vec!["A".to_owned(), "B".into()]));
        assert!(upper["Nominal"].is_empty() && upper["Applicability"] == "inspect");
    }
    let projected = &rows["pmi/Projection/1/projected_position"];
    assert_eq!((projected["Upper Limit"].as_str(), letters(projected)), ("0.15", vec!["A".to_owned()]));
    assert_eq!(rows["pmi/Top target/datum_target"]["Applicability"], "reference");
    if let Some(proof) = &proof {
        for extension in ["csv", "xlsx"] {
            std::fs::copy(workbook.with_extension(extension), proof.join(format!("new-kinds-plan.{extension}")))
                .unwrap();
        }
    }

    // STEP cannot carry them yet, so the package refuses before it writes anything.
    let step = dir.join("new-kinds.step");
    let refused = call_checked(
        &mut AppTools { state: &mut s },
        "manufacturing_package",
        json!({ "path": step }),
        &Caller::Mcp { client: "pmi-inspection".into() },
    );
    assert!(refused.is_error && refused.content.contains("Top target"), "{}", refused.content);
    for extension in ["step", "pdf", "dxf", "csv", "xlsx"] {
        assert!(!step.with_extension(extension).exists(), "no {extension} after a refusal");
    }

    // Without them, the hole, width and angle are left out of STEP and named, and stay in
    // the inspection plan.
    for name in ["Top target", "Bore pattern", "Projection"] {
        call(&mut s, "pmi_remove", json!({ "name": name }));
    }
    let report = call(&mut s, "manufacturing_package", json!({ "path": step, "part_number": "NK-1", "revision": "A" }));
    let left_out = report["manufacturing_information"]["left_out"].to_string();
    for name in ["Counterbored", "Plate thickness", "Corner"] {
        assert!(left_out.contains(name), "{name} is named as left out of STEP: {report}");
    }
    let rows = check_against_csv(&step.with_extension("xlsx"), "Plate, new kinds", "NK-1", "A");
    let row = |id: &str| rows.get(id).unwrap_or_else(|| panic!("{id} in {:?}", rows.keys()));
    for (id, nominal, lower, upper, unit) in [
        ("pmi/Counterbored/1/diameter", "4", "4", "4.1", "mm"),
        ("pmi/Counterbored/1/counterbore_diameter", "10", "", "", "mm"),
        ("pmi/Counterbored/1/counterbore_depth", "3", "", "", "mm"),
        ("pmi/Plate thickness/width", "10", "9.9", "10.1", "mm"),
        ("pmi/Corner/basic_angle", "90", "", "", "°"),
    ] {
        let found = row(id);
        let close = |text: &str, expected: &str| match (text.parse::<f64>(), expected.parse::<f64>()) {
            (Ok(text), Ok(expected)) => (text - expected).abs() < 1e-9,
            _ => text == expected,
        };
        assert!(
            close(&found["Nominal"], nominal)
                && close(&found["Lower Limit"], lower)
                && close(&found["Upper Limit"], upper)
                && found["Unit"] == unit,
            "{id}: {found:?}"
        );
    }
    assert_eq!(row("pmi/Corner/basic_angle")["Applicability"], "reference");
    assert_eq!(row("pmi/Counterbored/1/counterbore_depth")["Applicability"], "inspect");
    ribbon_agrees(&mut s, &dir, "new-kinds");
    if let Some(proof) = proof {
        for extension in ["csv", "xlsx"] {
            std::fs::copy(step.with_extension(extension), proof.join(format!("new-kinds-package.{extension}")))
                .unwrap();
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
