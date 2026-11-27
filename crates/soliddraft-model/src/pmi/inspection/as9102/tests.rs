use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::PathBuf;

use zip::ZipArchive;

use super::super::InspectionPlan;
use super::super::tests::{NOTE, PART, REVISION, bracket, parse_csv};
use super::{CHARACTERISTICS, HEADER_ROW, SHEETS};

/// A cell value as a spreadsheet reader sees it.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::pmi::inspection) enum Cell {
    Text(String),
    Number(f64),
}

/// Worksheets by name, in workbook order, each with its cells by reference such as "B3".
pub(in crate::pmi::inspection) type Workbook = Vec<(String, BTreeMap<String, Cell>)>;

/// Test reader that follows the package relationships rather than the writer's part names.
pub(in crate::pmi::inspection) fn read(bytes: &[u8]) -> Workbook {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut part = |name: &str| {
        let mut text = String::new();
        zip.by_name(name).unwrap_or_else(|_| panic!("no part {name}")).read_to_string(&mut text).unwrap();
        text
    };
    let types = part("[Content_Types].xml");
    let root = tags(&part("_rels/.rels"), "Relationship")
        .into_iter()
        .find(|tag| attr(tag, "Type").unwrap().ends_with("/officeDocument"))
        .and_then(|tag| attr(&tag, "Target"))
        .unwrap();
    assert!(types.contains(&format!(r#"PartName="/{root}""#)), "{types}");
    let (folder, file) = root.rsplit_once('/').unwrap();
    let targets: BTreeMap<_, _> = tags(&part(&format!("{folder}/_rels/{file}.rels")), "Relationship")
        .into_iter()
        .map(|tag| (attr(&tag, "Id").unwrap(), (attr(&tag, "Type").unwrap(), attr(&tag, "Target").unwrap())))
        .collect();
    assert!(
        targets
            .values()
            .any(|(kind, target)| kind.ends_with("/styles") && types.contains(&format!("/{folder}/{target}")))
    );
    tags(&part(&root), "sheet")
        .into_iter()
        .map(|tag| {
            let (kind, target) = &targets[&attr(&tag, "r:id").unwrap()];
            assert!(kind.ends_with("/worksheet"));
            assert!(types.contains(&format!(r#"PartName="/{folder}/{target}""#)));
            (attr(&tag, "name").unwrap(), cells(&part(&format!("{folder}/{target}"))))
        })
        .collect()
}

/// Opening tags named `name`, without their contents.
fn tags(xml: &str, name: &str) -> Vec<String> {
    xml.split('<')
        .filter(|piece| piece.strip_prefix(name).is_some_and(|rest| rest.starts_with([' ', '>', '/'])))
        .map(|piece| piece.split_once('>').unwrap().0.to_owned())
        .collect()
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    Some(unescape(&tag[start..start + tag[start..].find('"')?]))
}

fn cells(xml: &str) -> BTreeMap<String, Cell> {
    let mut cells = BTreeMap::new();
    for piece in xml.split("<c ").skip(1) {
        let (tag, rest) = piece.split_once('>').unwrap();
        let reference = attr(&format!(" {tag}"), "r").unwrap();
        if tag.ends_with('/') {
            continue;
        }
        let body = &rest[..rest.find("</c>").unwrap()];
        let value = match attr(&format!(" {tag}"), "t").as_deref() {
            Some("inlineStr") => {
                let text = body.split_once("<t").unwrap().1.split_once('>').unwrap().1;
                Cell::Text(unescape(&text[..text.find("</t>").unwrap()]))
            }
            None => {
                let value = body.strip_prefix("<v>").unwrap().strip_suffix("</v>").unwrap();
                Cell::Number(value.parse().unwrap())
            }
            Some(other) => panic!("unexpected cell type {other}"),
        };
        assert!(cells.insert(reference.clone(), value).is_none(), "{reference} twice");
    }
    cells
}

/// XML entities, then the spreadsheet `_xHHHH_` escapes, in reading order.
fn unescape(text: &str) -> String {
    let text = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#13;", "\r")
        .replace("&amp;", "&");
    let mut out = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("_x") {
        let code = rest.get(start + 2..start + 6).filter(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()));
        match code.filter(|_| rest.as_bytes().get(start + 6) == Some(&b'_')) {
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
    out.push_str(rest);
    out
}

pub(in crate::pmi::inspection) fn text<'a>(sheet: &'a BTreeMap<String, Cell>, reference: &str) -> Option<&'a str> {
    match sheet.get(reference) {
        Some(Cell::Text(text)) => Some(text),
        None => None,
        Some(other) => panic!("{reference} is {other:?}, not text"),
    }
}

/// Form 3 rows as maps from column name to a CSV-comparable string; an empty JSON array
/// reads as a blank cell.
pub(in crate::pmi::inspection) fn characteristics(
    form: &BTreeMap<String, Cell>,
) -> Vec<BTreeMap<&'static str, String>> {
    let header: Vec<_> =
        (b'A'..).take(CHARACTERISTICS.len()).map(|c| text(form, &format!("{}{HEADER_ROW}", char::from(c)))).collect();
    assert_eq!(header, CHARACTERISTICS.map(|(name, _)| Some(name)));
    let mut rows = Vec::new();
    for row in HEADER_ROW + 1.. {
        if !form.keys().any(|key| key.trim_start_matches(char::is_alphabetic) == row.to_string()) {
            break;
        }
        rows.push(
            (b'A'..)
                .zip(CHARACTERISTICS)
                .map(|(column, (name, _))| {
                    let value = match form.get(&format!("{}{row}", char::from(column))) {
                        None => String::new(),
                        Some(Cell::Text(text)) => text.clone(),
                        Some(Cell::Number(value)) => value.to_string(),
                    };
                    (name, value)
                })
                .collect(),
        );
    }
    rows
}

#[test]
fn p1_workbook_relates_three_forms_and_matches_the_csv() {
    let (doc, _) = bracket();
    let plan = InspectionPlan::from_document(&doc, Some(REVISION));
    let part_number = "PN 1 & <2> _x0041_";
    let workbook = plan.as9102_xlsx(Some(part_number)).unwrap();
    let mut csv = Vec::new();
    plan.write_csv(&mut csv).unwrap();
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_PMI_PROOF_DIR") {
        let dir = PathBuf::from(dir);
        std::fs::write(dir.join("p1-inspection.xlsx"), &workbook).unwrap();
        std::fs::write(dir.join("p1-inspection.csv"), &csv).unwrap();
    }

    let sheets = read(&workbook);
    assert_eq!(sheets.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), SHEETS);
    for (_, sheet) in &sheets {
        assert_eq!(text(sheet, "A3"), Some(part_number));
        assert_eq!(text(sheet, "B3"), Some(PART));
        assert_eq!((text(sheet, "C3"), text(sheet, "D3")), (None, None), "serial number and FAIR identifier");
    }
    let [(_, form1), (_, form2), (_, form3)] = &sheets[..] else { panic!("three forms") };
    assert_eq!(text(form1, "A4"), Some("5. Part Revision Level"));
    assert_eq!(text(form1, "B4"), Some(REVISION));
    assert_eq!(text(form3, "B4"), Some(REVISION));
    assert!((5..=15).all(|row| text(form1, &format!("B{row}")).is_none()), "nothing the part does not state");
    assert!((19..=26).all(|row| text(form1, &format!("B{row}")).is_none()), "no approval or verification");
    assert_eq!(text(form1, "B27"), Some("ISO 2768-m"));
    assert_eq!(text(form2, "A6"), Some("6061-T6"));
    assert!(["B6", "C6", "D6", "E6", "F6"].iter().all(|cell| text(form2, cell).is_none()), "no certificates");

    let records = parse_csv(&csv);
    let column = |name| records[0].iter().position(|column| column == name).unwrap();
    let rows = characteristics(form3);
    assert_eq!(rows.len(), records.len() - 1);
    for (row, record) in rows.iter().zip(&records[1..]) {
        let id = &record[column("characteristic_id")];
        assert_eq!(&row["5. Char. No."], id);
        for blank in [
            "6. Reference Location",
            "7. Characteristic Designator",
            "9. Results",
            "10. Designed / Qualified Tooling",
            "11. Nonconformance Number",
            "Disposition",
        ] {
            assert!(row[blank].is_empty(), "{id}: {blank} must stay blank");
        }
        for (sheet, csv) in [
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
            assert_eq!(row[sheet], record[column(csv)], "{id}: {sheet}");
        }
        for (sheet, csv) in [
            ("Member", "member"),
            ("Nominal", "nominal"),
            ("Lower Limit", "lower_limit"),
            ("Upper Limit", "upper_limit"),
        ] {
            let number = |text: &str| (!text.is_empty()).then(|| text.parse::<f64>().unwrap());
            assert_eq!(number(&row[sheet]), number(&record[column(csv)]), "{id}: {sheet}");
        }
        for (sheet, csv) in [("Datum Order (JSON)", "datum_order"), ("Face References (JSON)", "face_refs")] {
            let json = |text: &str| -> serde_json::Value {
                serde_json::from_str(if text.is_empty() { "[]" } else { text }).unwrap()
            };
            assert_eq!(json(&row[sheet]), json(&record[column(csv)]), "{id}: {sheet}");
        }
    }

    // The patterned hole has one diameter and one position frame per member, in datum order.
    let named = |item| rows.iter().filter(move |row| row["PMI Item"] == item);
    assert_eq!(named("Hole diameter").count(), 4);
    for member in 1..=4 {
        let position =
            rows.iter().find(|row| row["5. Char. No."] == format!("pmi/Hole position/{member}/position")).unwrap();
        assert_eq!((position["Applicability"].as_str(), position["Upper Limit"].as_str()), ("inspect", "0.2"));
        let letters: Vec<String> = serde_json::from_str::<Vec<serde_json::Value>>(&position["Datum Order (JSON)"])
            .unwrap()
            .iter()
            .map(|datum| datum["letter"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(letters, ["A", "B", "C"]);
    }
    let note = rows.iter().find(|row| row["8. Requirement"] == NOTE).unwrap();
    assert_eq!((note["Kind"].as_str(), note["Applicability"].as_str()), ("note", "reference"));
    let bore = named("Bore diameter").next().unwrap();
    assert_eq!((bore["Fit Class"].as_str(), bore["Lower Limit"].as_str()), ("H7", ""));
}

#[test]
fn workbook_text_survives_xml_and_spreadsheet_escapes() {
    let (doc, _) = bracket();
    let mut plan = InspectionPlan::from_document(&doc, None);
    plan.part = "=1+1 & <bracket> _x0041_ \"quoted\"".to_owned();
    let sheets = read(&plan.as9102_xlsx(None).unwrap());
    assert_eq!(text(&sheets[0].1, "B3"), Some(plan.part.as_str()));
    assert_eq!(text(&sheets[0].1, "A3"), None, "no part number without one stated");
    assert_eq!(text(&sheets[0].1, "B4"), None, "no revision without one stated");
    plan.part = "bad\u{1}".to_owned();
    assert!(plan.as9102_xlsx(None).is_err(), "control characters cannot be stored");
}
