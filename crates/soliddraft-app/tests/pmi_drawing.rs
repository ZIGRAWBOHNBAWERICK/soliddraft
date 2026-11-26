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

#[test]
fn hole_requirements_round_trip_through_panel_tools_and_drawing() {
    use egui_kittest::kittest::Queryable;
    use soliddraft_app::{document::Selection, pmi::panel};
    use soliddraft_kernel_api::SurfaceKind::{Cone, Cylinder};
    use std::cell::RefCell;

    let mut s = state("Hole PMI");
    call(
        &mut s,
        "sketch",
        json!({
            "plane": "top",
            "shapes": [{"shape": "rectangle", "center": [40, 20, 0], "width": 80, "height": 40}]
        }),
    );
    call(&mut s, "extrude", json!({"sketch": "Sketch1", "depth": 10, "flip": false}));
    let face = "Boss-Extrude1 · End cap";
    for (x, extra) in [
        (12, json!({"counterbore": {"diameter": 10, "depth": 3}})),
        (30, json!({"countersink": {"diameter": 8, "angle": 90}})),
        (50, json!({"depth": 7})),
        (68, json!({"depth": 7})),
    ] {
        let mut input = json!({"face": face, "points": [[x, 20, 10]], "diameter": 4});
        input.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        call(&mut s, "hole", input);
    }

    let wall = |x, z| bracket::face(&s, Cylinder, [Some(x), Some(20.0), Some(z)]);
    let cases = [
        ("Counterbored", wall(12.0, 3.5), Some(("Counterbore", "counterbore", wall(12.0, 8.5))), true, None),
        (
            "Countersunk",
            wall(30.0, 4.0),
            Some(("Countersink", "countersink", bracket::face(&s, Cone, [Some(30.0), Some(20.0), None]))),
            true,
            None,
        ),
        ("Blind", wall(50.0, 6.5), None, false, None),
        ("Blind and tapped", wall(68.0, 6.5), None, false, Some("4.5")),
    ];
    let id = |name: &str| {
        s.doc
            .faces
            .iter()
            .find(|face| format!("{} · {}", s.doc.feature(face.feature).unwrap().name, face.label) == name)
            .unwrap()
            .id
    };
    let picked: Vec<_> =
        cases.iter().map(|(_, pilot, head, ..)| (id(pilot), head.as_ref().map(|(_, _, head)| id(head)))).collect();
    s.pmi.open = true;
    let s = RefCell::new(s);
    {
        let mut app = egui_kittest::Harness::builder()
            .with_size(egui::vec2(720.0, 1000.0))
            .build_ui(|ui| panel::show(ui, &mut s.borrow_mut()));
        for (i, ((name, _, head, through, thread), (pilot_id, head_id))) in cases.iter().zip(picked).enumerate() {
            app.get_by_label("Hole").click();
            app.run_steps(2);
            if let Some((caption, ..)) = head {
                app.get_by_label(caption).click();
                app.run_steps(2);
                s.borrow_mut().doc.selection = head_id.map(Selection::Face);
                app.run_steps(2);
                app.get_by_label("Use picks as head").click();
                app.run_steps(2);
            }
            s.borrow_mut().doc.selection = Some(Selection::Face(pilot_id));
            app.run_steps(2);
            if *through {
                app.get_by_label("Through hole").click();
                app.run_steps(2);
            }
            if let Some(depth) = thread {
                for (label, value) in [("Thread", "M5x0.8-6H"), ("Usable thread depth", *depth)] {
                    app.get_by_label(label).type_text(value);
                    app.run_steps(2);
                }
            }
            app.get_by_label("Item name").type_text(name);
            app.run_steps(2);
            app.get_by_label("Add").click();
            app.run_steps(2);
            let rows = s.borrow().doc.model().pmi_rows();
            assert_eq!(rows.len(), i + 1, "{name}");
            assert_eq!(rows[i].item.id, *name);
            assert_eq!(rows[i].problem, None, "{name}");

            app.get_all_by_label("Edit").nth(i).unwrap().click();
            app.run_steps(2);
            let (field, value) = if thread.is_some() { ("Drill depth", "7") } else { ("Tolerance", "H7") };
            app.get_by_label(field).type_text(value);
            app.run_steps(2);
            app.get_by_label("Change").click();
            app.run_steps(2);
            let edited = &s.borrow().doc.model().pmi_rows()[i];
            assert_ne!(edited.item, rows[i].item, "{name}: panel edit");
            assert_eq!(edited.item.features(), rows[i].item.features());
            assert_eq!(edited.problem, None, "{name}");
        }
    }

    let mut s = s.into_inner();
    let edited_items = s.doc.model().pmi().clone();
    for (name, pilot, head, through, thread) in cases {
        let mut input = json!({
            "kind": "hole", "replaces": name, "pilot_faces": [pilot], "through": through
        });
        if let Some((_, kind, faces)) = head {
            input["head_kind"] = json!(kind);
            input["head_faces"] = json!([faces]);
        }
        if let Some(depth) = thread {
            input["thread"] = json!("M5x0.8-6H");
            input["thread_depth"] = json!(depth);
            input["drill_depth"] = json!("7");
        }
        let row = call(&mut s, "pmi_add", input);
        assert!(row["added"].get("problem").is_none(), "{row}");
        assert!(row["added"]["hole"].get("tolerance").is_none(), "{row}");
    }
    let panel_items = s.doc.model().pmi().clone();
    for (edited, from_tool) in edited_items.items.iter().zip(&panel_items.items) {
        assert_eq!(from_tool.id, edited.id);
        assert_eq!(from_tool.features(), edited.features());
    }

    let scratch = std::env::temp_dir().join(format!("soliddraft-hole-pmi-{}", std::process::id()));
    let dir = std::env::var_os("SOLIDDRAFT_PMI_PROOF_DIR").map_or(scratch.clone(), std::path::PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hole-pmi.dwprt");
    call(&mut s, "save_as", json!({"path": path}));
    let mut reopened = state("Empty");
    call(&mut reopened, "open", json!({"path": path}));
    assert_eq!(*reopened.doc.model().pmi(), panel_items);
    assert_eq!(call(&mut reopened, "pmi_list", json!({})), call(&mut s, "pmi_list", json!({})));

    call(&mut reopened, "make_drawing", json!({"template":"iso-a3"}));
    let expected = [
        "Ø4 THRU\nC'BORE Ø10 DEEP 3",
        "Ø4 THRU\nC'SINK Ø8 × 90°",
        "Ø4 DRILL DEEP 7",
        "Ø4 DRILL DEEP 7\nM5x0.8-6H THREAD DEEP 4.5",
    ];
    let session = reopened.drawing.session.as_ref().unwrap();
    assert!(session.requirements.unplaced.is_empty(), "{:?}", session.requirements.unplaced);
    let drawing = &session.doc.drawing;
    assert_eq!(pmi_items::shown(drawing).len(), 4);
    for text in expected {
        assert!(
            drawing.sheets[0].annotations.iter().any(|annotation| {
                !annotation.anchors.is_empty()
                    && matches!(&annotation.body, AnnotationKind::Note { text: drawn, .. } if drawn == text)
            }),
            "missing {text}"
        );
    }
    call(&mut reopened, "drawing_export_pdf", json!({"path": dir.join("hole-pmi.pdf")}));
    let _ = std::fs::remove_dir_all(scratch);
}
