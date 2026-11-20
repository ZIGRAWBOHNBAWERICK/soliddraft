//! The P1 bracket test's third and fourth checks through the application's tools.
//!
//! C3: one action writes the STEP file and a drawing of it as PDF and DXF. The drawing
//! has the front, top and side views and a section through the bore, shows every item
//! of the part's manufacturing information once, locates the bore and the holes from
//! the datums, lays nothing over anything, and names the part, its material, its
//! general tolerance and its revision in the title block. Nobody moves anything.
//! C4: nothing on the drawing is missing from the file, and an item the drawing cannot
//! show is listed by name.
//!
//! Set `SOLIDDRAFT_PMI_PROOF_DIR` to keep the three files.

#[path = "support/bracket_pmi.rs"]
mod bracket;

use bracket::{annotate, build, call, state};
use serde_json::json;
use soliddraft_drawing::{AnnotationKind, ViewKind, display, pmi_items};

const ITEMS: [&str; 14] = [
    "Datum A",
    "Datum B",
    "Datum C",
    "Bore diameter",
    "Bore finish",
    "Bore position",
    "Boss diameter",
    "Boss height",
    "Hole diameter",
    "Hole position",
    "Slot",
    "Slot height",
    "Flatness of A",
    "Perpendicularity of B",
];

/// The basic distances that locate the bore and the four holes from datums B and C: the
/// holes share two rows and two columns, so each distance is drawn once.
const LOCATED: [&str; 6] = [
    "Bore position: 25 from B",
    "Bore position: 30 from C",
    "Hole position: 12 from B",
    "Hole position: 32 from B",
    "Hole position: 10 from C",
    "Hole position: 50 from C",
];

#[test]
fn one_action_writes_the_step_file_and_a_drawing_of_everything_in_it() {
    let scratch = std::env::temp_dir().join(format!("soliddraft-pmi-drawing-{}", std::process::id()));
    let dir = std::env::var_os("SOLIDDRAFT_PMI_PROOF_DIR").map_or(scratch.clone(), std::path::PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = state("P1 bracket");
    build(&mut s);
    annotate(&mut s);

    let step = dir.join("p1-bracket-package.step");
    let report = call(&mut s, "manufacturing_package", json!({"path": step}));
    for extension in ["step", "pdf", "dxf"] {
        let file = step.with_extension(extension);
        assert!(std::fs::metadata(&file).is_ok_and(|m| m.len() > 1000), "{} is written", file.display());
    }
    assert_eq!(report["manufacturing_information"], json!({"items": 14}), "{report}");

    // C3: every item once, the features located from the datums, nothing overlapping.
    let drawing = &report["drawing"];
    assert_eq!(drawing["not_shown"], json!([]), "every item has a place: {drawing:#}");
    let shown = drawing["shown"].as_object().unwrap();
    for name in ITEMS.iter().chain(&LOCATED) {
        assert_eq!(shown.get(*name), Some(&json!(1)), "{name} is on the drawing once: {shown:?}");
    }
    assert_eq!(shown.len(), ITEMS.len() + LOCATED.len(), "and nothing else is: {shown:?}");
    assert_eq!(drawing["overlaps"], json!([]), "nothing lies over anything");
    // C4: nothing on the drawing that the file does not state.
    assert_eq!(drawing["not_in_file"], json!([]));

    let session = s.drawing.session.as_ref().expect("the drawing is open");
    let made = &session.doc.drawing;
    let sheet = &made.sheets[0];
    let sections = sheet.views.iter().filter(|v| matches!(v.kind, ViewKind::Section { .. })).count();
    assert_eq!((sheet.views.len(), sections), (4, 1), "front, top and side views and a section through the bore");
    assert!(pmi_items::overlaps(made, 0).is_empty());

    // The title block names the part, its material, its general tolerance and its revision.
    let field = |key: &str| display::property(made, sheet, 0, 1, key);
    assert_eq!(field("title"), "P1 bracket");
    assert_eq!(field("material"), "6061-T6");
    assert_eq!(field("general_tolerance"), "ISO 2768-m");
    assert!(!field("revision").is_empty(), "a revision");
    // The part's other note is on the sheet, and the template's stock notes are not.
    let notes: Vec<&str> = sheet
        .annotations
        .iter()
        .filter_map(|a| match &a.body {
            AnnotationKind::Note { text, .. } if a.anchors.is_empty() => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(notes, ["NOTES\n1. Break sharp edges"]);

    // The DXF says the same in text a reader can search.
    let dxf = std::fs::read_to_string(step.with_extension("dxf")).unwrap();
    for text in ["6061-T6", "ISO 2768-m", "H7", "THRU"] {
        assert!(dxf.contains(text), "the DXF holds {text}");
    }

    // C4, the other way: what a drawing cannot show is listed by name with the reason,
    // not drawn somewhere it does not belong. A drawing with the top view alone sees no
    // height, so the heights and the faces it sees flat are listed.
    let mut top_only = soliddraft_app::drawing::NewDrawing::new();
    for (view, on) in &mut top_only.views {
        *on = *view == soliddraft_kernel_api::drawing::StandardView::Top;
    }
    let session = soliddraft_app::drawing::create_drawing(&s, &top_only).unwrap();
    let listed: Vec<&str> = session.requirements.unplaced.iter().map(|u| u.name.as_str()).collect();
    for name in ["Datum A", "Boss height", "Slot", "Slot height", "Flatness of A"] {
        assert!(listed.contains(&name), "{name} is listed: {:?}", session.requirements.unplaced);
    }
    assert!(session.requirements.unplaced.iter().all(|u| !u.reason.is_empty()));
    let on_top = pmi_items::shown(&session.doc.drawing);
    assert!(listed.iter().all(|name| !on_top.contains_key(*name)), "and none of them is drawn");
    assert_eq!(on_top.get("Bore diameter"), Some(&1));
    let _ = std::fs::remove_dir_all(&scratch);
}
