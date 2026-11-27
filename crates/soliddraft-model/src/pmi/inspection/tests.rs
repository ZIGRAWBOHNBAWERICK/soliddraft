use std::path::PathBuf;
use std::sync::Arc;

use soliddraft_kernel_api::pmi::{PmiItem, PmiKind, Tolerance};
use soliddraft_kernel_catallax::CatallaxKernel;

use super::{DatumRef, FaceRef, InspectionPlan};
use crate::{Command, Document, FeatureId, FeatureKind, ImportStepFeature};

const PART: &str = "P1 bracket, \"L\"";
const REVISION: &str = "Rev, \"B\"\r\nreleased";
const NOTE: &str = "Deburr, \"do not polish\"\nSee drawing.";
const BRACKET: &[u8] = include_bytes!("../../../../../ci/fixtures/pmi/p1-bracket-pmi.step");

fn bracket() -> (Document, FeatureId) {
    let mut doc = Document::new(PART, Arc::new(CatallaxKernel::new()));
    let import = doc
        .execute(Command::AddFeature {
            kind: FeatureKind::ImportStep(ImportStepFeature::new("p1-bracket-pmi.step", BRACKET.to_vec())),
            name: None,
            position: None,
            via_assistant: false,
        })
        .unwrap()
        .created
        .unwrap();
    let read = doc.kernel().read_step_pmi(BRACKET).unwrap();
    let (items, report) = doc.pmi_from_step(&[import], &read);
    assert_eq!(report.added.len(), 14);
    assert!(report.left_out.is_empty(), "{report:?}");
    for item in items {
        doc.execute(Command::PutPmiItem { item: Box::new(item), replaces: None }).unwrap();
    }
    let mut notes = read.pmi.part;
    notes.notes.push(NOTE.to_owned());
    doc.execute(Command::SetPartNotes { notes }).unwrap();
    (doc, import)
}

#[test]
fn p1_inspection_qif_exports_requirements_without_inventing_results() {
    let (doc, _) = bracket();
    let plan = InspectionPlan::from_document(&doc, Some(REVISION));
    let metadata = soliddraft_exchange::qif::Metadata {
        qpid: "550e8400-e29b-41d4-a716-446655440000",
        organization: "ISO",
        designator: "1101",
        year: Some("2017"),
    };
    let mut qif = Vec::new();
    let omitted = plan.write_qif(&mut qif, metadata).unwrap();
    let notes: Vec<_> = plan
        .groups
        .iter()
        .flat_map(|g| &g.rows)
        .filter(|r| r.kind == super::InspectionKind::Note)
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(omitted.iter().map(|o| o.characteristic_id.as_str()).collect::<Vec<_>>(), notes, "{omitted:?}");
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_QIF_PROOF_DIR") {
        let dir = PathBuf::from(dir);
        let mut csv = Vec::new();
        plan.write_csv(&mut csv).unwrap();
        std::fs::write(dir.join("p1.qif"), qif).unwrap();
        std::fs::write(dir.join("p1.csv"), csv).unwrap();
    }
}

#[test]
fn planar_inspection_qif_exports_width_and_reports_unrepresentable_requirements() {
    use super::{InspectionGroup, PmiRow};
    use soliddraft_kernel_api::pmi::Decimal;

    let (doc, _) = bracket();
    let mut plan = InspectionPlan::from_document(&doc, Some(REVISION));
    let face_group = |letter| doc.pmi().datum_feature(letter).unwrap().clone();
    let (a, b, c) = (face_group("A"), face_group("B"), face_group("C"));
    let group = |id: &str, kind, measured| {
        InspectionGroup::from_pmi(PmiRow {
            item: PmiItem { id: id.into(), kind },
            measured,
            on_body: None,
            hole: None,
            located: Vec::new(),
            problem: None,
            feature: None,
            placements: Vec::new(),
            depth: None,
            body: None,
        })
    };
    plan.groups = vec![
        group(
            "Width",
            PmiKind::Width { flats: [a.clone(), b.clone()], tolerance: Tolerance::Symmetric { value: 0.5 } },
            vec![12.0],
        ),
        group("Chamfer", PmiKind::Chamfer { bevel: c.clone(), sides: [a.clone(), b.clone()] }, vec![2.0, 45.0]),
        group(
            "Edge break",
            PmiKind::EdgeBreak {
                bevel: c,
                sides: [a.clone(), b.clone()],
                min_mm: Decimal::parse("0.1").unwrap(),
                max_mm: Decimal::parse("0.6").unwrap(),
            },
            vec![0.3],
        ),
        group("Basic angle", PmiKind::BasicAngle { planes: [a, b] }, vec![90.0]),
    ];
    let metadata = soliddraft_exchange::qif::Metadata {
        qpid: "6b30ca06-1cee-4e24-a942-e6814d659608",
        organization: "ISO",
        designator: "1101",
        year: Some("2017"),
    };
    let mut qif = Vec::new();
    let omitted = plan.write_qif(&mut qif, metadata).unwrap();
    let skipped: Vec<_> = omitted.iter().map(|o| (o.characteristic_id.as_str(), o.reason)).collect();
    assert_eq!(
        skipped,
        [
            ("pmi/Chamfer/chamfer_setback", "chamfer setback has no QIF mapping in this profile"),
            ("pmi/Chamfer/chamfer_angle", "angle has no QIF mapping in this profile"),
            ("pmi/Edge break/edge_break", "bounded edge break has no QIF mapping in this profile"),
            ("pmi/Basic angle/basic_angle", "angle has no QIF mapping in this profile"),
        ]
    );
    let xml = std::str::from_utf8(&qif).unwrap();
    assert!(xml.contains("<WidthCharacteristicNominal "));
    assert!(xml.contains("value=\"pmi/Width/width\""));
    assert!(xml.contains("<TargetValue>12</TargetValue>"));
    for (id, _) in skipped {
        assert!(!xml.contains(id), "{id} must not become a QIF characteristic");
    }
    if let Some(dir) = std::env::var_os("SOLIDDRAFT_QIF_PROOF_DIR") {
        std::fs::write(PathBuf::from(dir).join("planar.qif"), qif).unwrap();
    }
}

/// Test reader for the CSV wire format, independent of the writer and its column ordering.
fn parse_csv(csv: &[u8]) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut input = std::str::from_utf8(csv).unwrap().chars().peekable();
    while let Some(c) = input.next() {
        match c {
            '"' if quoted && input.next_if_eq(&'"').is_some() => field.push('"'),
            '"' if quoted => {
                quoted = false;
                assert!(matches!(input.peek(), Some(',' | '\r')));
            }
            '"' => {
                assert!(field.is_empty(), "quote within an unquoted field");
                quoted = true;
            }
            ',' if !quoted => record.push(std::mem::take(&mut field)),
            '\r' if !quoted => {
                assert_eq!(input.next(), Some('\n'));
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            '\n' if !quoted => panic!("bare LF outside a quoted field"),
            c => field.push(c),
        }
    }
    assert!(!quoted && field.is_empty() && record.is_empty(), "truncated CSV record");
    assert!(records.iter().all(|record| record.len() == records[0].len()));
    records
}

fn csv(doc: &Document) -> Vec<Vec<String>> {
    let mut output = Vec::new();
    InspectionPlan::from_document(doc, Some(REVISION)).write_csv(&mut output).unwrap();
    parse_csv(&output)
}

struct SavedPart(PathBuf);

impl Drop for SavedPart {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn p1_inspection_csv_preserves_grouping_and_trace_after_native_reopen() {
    let (mut doc, _) = bracket();
    let PmiKind::Location { from, to, .. } = doc.pmi().item("Slot height").unwrap().kind.clone() else {
        panic!("P1 slot height should be a distance");
    };
    doc.execute(Command::PutPmiItem {
        item: Box::new(PmiItem {
            id: "Slot height, \"basic\"".to_owned(),
            kind: PmiKind::Location { from, to, tolerance: Tolerance::Basic, nominal: None },
        }),
        replaces: None,
    })
    .unwrap();

    let records = csv(&doc);
    let rows = &records[1..];
    let col = |name| records[0].iter().position(|column| column == name).unwrap();
    let named = |name| rows.iter().filter(move |row| row[col("pmi_item")] == name);
    assert!(rows.iter().all(|row| {
        row[col("part")] == PART
            && row[col("revision")] == REVISION
            && row[col("material")] == "6061-T6"
            && row[col("general_tolerance")] == "ISO 2768-m"
            && row[col("actual")].is_empty()
            && row[col("disposition")].is_empty()
    }));
    let ids: std::collections::HashSet<_> = rows.iter().map(|row| &row[col("characteristic_id")]).collect();
    assert_eq!(ids.len(), rows.len());

    let PmiKind::Size { features, .. } = &doc.pmi().item("Hole diameter").unwrap().kind else {
        panic!("P1 hole diameters should name a pattern");
    };
    let diameter: Vec<_> = named("Hole diameter").collect();
    let position: Vec<_> = named("Hole position").collect();
    assert_eq!((diameter.len(), position.len()), (4, 12));
    let (frames, []) = position.as_chunks::<3>() else { panic!("incomplete position frame") };
    for (index, (diameter, frame)) in diameter.iter().zip(frames).enumerate() {
        let member = (index + 1).to_string();
        assert_eq!(diameter[col("group_id")], "pmi/Hole diameter");
        assert_eq!(diameter[col("member")], member);
        assert_eq!(diameter[col("kind")], "diameter");
        assert_eq!(diameter[col("nominal")], "4");
        assert_eq!(diameter[col("unit")], "mm");
        assert_eq!(diameter[col("lower_limit")], "4");
        assert_eq!(diameter[col("upper_limit")], "4.1");
        let faces: Vec<Vec<FaceRef>> = serde_json::from_str(&diameter[col("face_refs")]).unwrap();
        assert_eq!(faces, features[index..index + 1]);

        assert!(frame.iter().all(|row| {
            row[col("group_id")] == "pmi/Hole position"
                && row[col("member")] == member
                && row[col("face_refs")] == diameter[col("face_refs")]
        }));
        let check = frame[0];
        assert_eq!(check[col("kind")], "position");
        assert_eq!(check[col("applicability")], "inspect");
        assert_eq!(check[col("unit")], "mm");
        assert_eq!(check[col("upper_limit")], "0.2");
        assert!(check[col("nominal")].is_empty() && check[col("lower_limit")].is_empty());
        let frame_datums: Vec<DatumRef> = serde_json::from_str(&check[col("datum_order")]).unwrap();
        assert_eq!(frame_datums, [DatumRef::new("A"), DatumRef::new("B"), DatumRef::new("C")]);
        assert_eq!(frame[1][col("locating_datum")], "B");
        assert_eq!(frame[2][col("locating_datum")], "C");
        let (x, y) = [("10", "12"), ("10", "32"), ("50", "12"), ("50", "32")][index];
        assert_eq!(frame[1][col("nominal")], y);
        assert_eq!(frame[2][col("nominal")], x);
        assert!(frame[1..].iter().all(|row| {
            row[col("applicability")] == "reference"
                && row[col("tolerance")] == "basic"
                && row[col("lower_limit")].is_empty()
                && row[col("upper_limit")].is_empty()
        }));
    }

    let fit = named("Bore diameter").next().unwrap();
    assert_eq!(fit[col("fit_class")], "H7");
    assert_eq!(fit[col("tolerance")], "iso_286_fit");
    assert!(fit[col("lower_limit")].is_empty() && fit[col("upper_limit")].is_empty());
    let finish = named("Bore finish").next().unwrap();
    assert_eq!(finish[col("unit")], "µm");
    assert_eq!(finish[col("upper_limit")], "1.6");
    let slot: Vec<_> = named("Slot").collect();
    assert_eq!(
        slot.iter().map(|row| row[col("kind")].as_str()).collect::<Vec<_>>(),
        ["slot_length", "slot_width", "slot_radius"]
    );
    assert_eq!(slot[1][col("upper_limit")], "6.1");
    assert!(slot[2][col("upper_limit")].is_empty());
    let basic = named("Slot height, \"basic\"").next().unwrap();
    assert_eq!(basic[col("applicability")], "reference");
    assert_eq!(basic[col("nominal")], "18");
    assert_eq!(named("Datum A").next().unwrap()[col("applicability")], "reference");
    let note = rows.iter().find(|row| row[col("pmi_callout")] == NOTE).unwrap();
    assert_eq!(note[col("kind")], "note");
    assert_eq!(note[col("applicability")], "reference");
    assert!(note[col("nominal")].is_empty() && note[col("upper_limit")].is_empty());

    let saved = SavedPart(std::env::temp_dir().join(format!("inspection-p1-{}.dwprt", std::process::id())));
    doc.save(&saved.0).unwrap();
    let reopened = Document::open(&saved.0, Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(csv(&reopened), records);
}

#[test]
fn suppressed_pmi_keeps_the_characteristic_without_inventing_a_nominal() {
    let (mut doc, import) = bracket();
    let before = InspectionPlan::from_document(&doc, None);
    doc.execute(Command::SetSuppressed { id: import, suppressed: true }).unwrap();
    let after = csv(&doc);
    let col = |name| after[0].iter().position(|column| column == name).unwrap();
    let diameter = before.groups.iter().find(|group| group.item_id.as_deref() == Some("Hole diameter")).unwrap();
    let unresolved: Vec<_> = after[1..].iter().filter(|row| row[col("pmi_item")] == "Hole diameter").collect();
    assert_eq!(unresolved.len(), 4);
    for (row, original) in unresolved.into_iter().zip(&diameter.rows) {
        assert_eq!(row[col("characteristic_id")], original.id);
        assert_eq!(row[col("applicability")], "unresolved");
        assert!(!row[col("problem")].is_empty());
        for name in ["nominal", "lower_limit", "upper_limit", "actual", "disposition"] {
            assert!(row[col(name)].is_empty(), "{name} must not be fabricated");
        }
    }
}
