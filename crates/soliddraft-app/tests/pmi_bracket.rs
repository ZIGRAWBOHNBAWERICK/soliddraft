//! The P1 bracket test's first two checks through the application's tools.
//!
//! C1: the bracket's fourteen items of manufacturing information go into its STEP file
//! with the distances that locate the bore and the holes from the datums,
//! come back onto the same faces in a new part, and stay there through save and reopen.
//! C2: the bore edited from 8 to 10 mm reads Ø10 H7 with its position tolerance still
//! attached, and exports again.
//!
//! Set `SOLIDDRAFT_PMI_PROOF_DIR` to keep the STEP files, for the checks in other
//! readers (`ci/check-step-pmi.py`, the NIST STEP File Analyzer).

#[path = "support/bracket_pmi.rs"]
mod bracket;

use bracket::{annotate, build, call, state};
use serde_json::{Value, json};
use soliddraft_app::state::AppState;
use soliddraft_kernel_api::SurfaceKind;

/// Every item's name and callout, in the part's order, with no item in trouble.
fn callouts(state: &mut AppState) -> Vec<(String, String)> {
    let listed = call(state, "pmi_list", json!({}));
    listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            assert!(row.get("problem").is_none(), "{row}");
            (row["name"].as_str().unwrap().to_owned(), row["callout"].as_str().unwrap().to_owned())
        })
        .collect()
}

fn expected(bore: &str) -> Vec<(String, String)> {
    [
        ("Datum A", "Datum A".to_owned()),
        ("Datum B", "Datum B".to_owned()),
        ("Datum C", "Datum C".to_owned()),
        ("Bore diameter", format!("\u{d8}{bore} H7")),
        ("Bore finish", "Ra 1.6".to_owned()),
        ("Bore position", "Position \u{d8}0.05 A|B|C".to_owned()),
        ("Boss diameter", "\u{d8}16 \u{b1}0.1".to_owned()),
        ("Boss height", "15 \u{b1}0.1".to_owned()),
        ("Hole diameter", "4\u{d7} \u{d8}4 +0.1/0 THRU".to_owned()),
        ("Hole position", "Position \u{d8}0.2 A|B|C".to_owned()),
        ("Slot", "20\u{d7}6 +0.1/0 R3".to_owned()),
        ("Slot height", "18 \u{b1}0.1".to_owned()),
        ("Flatness of A", "Flatness 0.05".to_owned()),
        ("Perpendicularity of B", "Perpendicularity 0.1 A".to_owned()),
    ]
    .into_iter()
    .map(|(name, callout)| (name.to_owned(), callout))
    .collect()
}

/// The basic distances that locate the bore and the four holes from datums B and C, as
/// (item, feature, datum, millimetres), sorted.
fn located(state: &mut AppState) -> Vec<(String, u64, String, i64)> {
    let listed = call(state, "pmi_list", json!({}));
    let mut found = Vec::new();
    for row in listed["items"].as_array().unwrap() {
        for one in row["located"].as_array().into_iter().flatten() {
            found.push((
                row["name"].as_str().unwrap().to_owned(),
                one["feature"].as_u64().unwrap(),
                one["datum"].as_str().unwrap().to_owned(),
                (one["value"].as_f64().unwrap() * 1e6).round() as i64,
            ));
        }
    }
    found.sort();
    found
}

fn expected_located() -> Vec<(String, u64, String, i64)> {
    let mm = |value: i64| value * 1_000_000;
    // Holes in the order they were given: (10, 12), (10, 32), (50, 12), (50, 32).
    let mut all = vec![
        ("Bore position".to_owned(), 0, "B".to_owned(), mm(25)),
        ("Bore position".to_owned(), 0, "C".to_owned(), mm(30)),
    ];
    for (index, (x, y)) in [(10, 12), (10, 32), (50, 12), (50, 32)].into_iter().enumerate() {
        all.push(("Hole position".to_owned(), index as u64, "B".to_owned(), mm(y)));
        all.push(("Hole position".to_owned(), index as u64, "C".to_owned(), mm(x)));
    }
    all.sort();
    all
}

/// Per item, the surface kind and centroid of every face it is on.
type Places = Vec<(String, Vec<(SurfaceKind, [i64; 3])>)>;

/// Where each item sits.
fn places(state: &AppState) -> Places {
    state
        .doc
        .model()
        .pmi_rows()
        .into_iter()
        .map(|row| {
            let mut faces: Vec<_> = row
                .on_body
                .expect("the item is on the body")
                .features()
                .into_iter()
                .flatten()
                .map(|id| {
                    let face = state.doc.named_face(*id).unwrap();
                    let c = face.centroid;
                    (face.surface, [c.x, c.y, c.z].map(|v| (f64::from(v) * 1e3).round() as i64))
                })
                .collect();
            faces.sort_by_key(|face| face.1);
            (row.item.id, faces)
        })
        .collect()
}

fn export(state: &mut AppState, path: &std::path::Path) -> Value {
    let result = call(state, "export_step", json!({"path": path}));
    assert_eq!(result["manufacturing_information"], json!({"items": 14}), "{result}");
    result
}

#[test]
fn the_bracket_carries_its_manufacturing_information_through_step_and_a_bore_edit() {
    let kept = std::env::var_os("SOLIDDRAFT_PMI_PROOF_DIR").map(std::path::PathBuf::from);
    let scratch = std::env::temp_dir().join(format!("soliddraft-pmi-bracket-{}", std::process::id()));
    let dir = kept.as_deref().unwrap_or(&scratch);
    std::fs::create_dir_all(dir).unwrap();

    let mut first = state("P1 bracket");
    build(&mut first);
    annotate(&mut first);
    assert_eq!(callouts(&mut first), expected("8"));
    assert_eq!(located(&mut first), expected_located());
    let step = dir.join("p1-bracket-pmi.step");
    export(&mut first, &step);
    let original = places(&first);

    // C1: a new part opens the file and has every item on the same face.
    let mut second = state("Imported bracket");
    let preview = call(&mut second, "preview_import", json!({"path": step}));
    let imported =
        call(&mut second, "import_step", json!({"path": step, "decline": [], "preview_token": preview["token"]}));
    assert_eq!(
        imported["manufacturing_information"]["summary"], "14 manufacturing items read",
        "{}",
        imported["manufacturing_information"]
    );
    assert_eq!(callouts(&mut second), expected("8"));
    assert_eq!(located(&mut second), expected_located());
    assert_eq!(places(&second), original);
    let notes = json!({"general_tolerance": "ISO 2768-m", "material": "6061-T6", "notes": ["Break sharp edges"]});
    assert_eq!(call(&mut second, "pmi_list", json!({}))["notes"], notes);

    // One undo takes the file's information off again; redo puts it back.
    call(&mut second, "undo", json!({}));
    assert!(callouts(&mut second).is_empty());
    call(&mut second, "redo", json!({}));
    assert_eq!(callouts(&mut second), expected("8"));

    // Saved and reopened, it is still all there, and it exports the same again.
    let part = dir.join("imported-bracket.dwprt");
    call(&mut second, "save", json!({"path": part}));
    let mut third = state("Reopened bracket");
    call(&mut third, "open", json!({"path": part}));
    assert_eq!(callouts(&mut third), expected("8"));
    assert_eq!(places(&third), original);
    let again = dir.join("p1-bracket-pmi-reexported.step");
    export(&mut third, &again);
    let reread = third.doc.kernel().read_step_pmi(&std::fs::read(&again).unwrap()).unwrap();
    // The fourteen items and the ten distances that locate the bore and the holes.
    assert_eq!((reread.pmi.items.len(), reread.skipped.len()), (24, 0));

    // C2: the bore goes from 8 to 10 mm and keeps its fit and its position tolerance.
    call(&mut first, "drive", json!({"sketch": "Sketch4", "dimension": 1, "expression": "10"}));
    assert_eq!(callouts(&mut first), expected("10"));
    let edited = dir.join("p1-bracket-pmi-bore-10.step");
    export(&mut first, &edited);
    let read = first.doc.kernel().read_step_pmi(&std::fs::read(&edited).unwrap()).unwrap();
    let bore = read.pmi.item("Bore diameter").unwrap();
    let soliddraft_kernel_api::pmi::PmiKind::Size { nominal, .. } = &bore.kind else { panic!("a size") };
    assert_eq!(*nominal, Some(10.0));
    assert_eq!(bore.callout(&[10.0]), "\u{d8}10 H7");
    // The file states the fit itself, and the position tolerance is still on the bore:
    // the same face as the diameter, with its zone and its three datums.
    let soliddraft_kernel_api::pmi::PmiKind::Size { tolerance, features: bore_faces, .. } = &bore.kind else {
        panic!("a size")
    };
    assert_eq!(*tolerance, soliddraft_kernel_api::pmi::Tolerance::parse("H7").unwrap());
    let position = read.pmi.item("Bore position").expect("the position tolerance is in the file");
    assert_eq!(position.callout(&[]), "Position \u{d8}0.05 A|B|C");
    let soliddraft_kernel_api::pmi::PmiKind::Gdt { features: toleranced, .. } = &position.kind else {
        panic!("a geometric tolerance")
    };
    assert_eq!(toleranced, bore_faces, "the tolerance is on the bore's face");
    // And in the part: the bore's three items read as before, on a bore of 10 mm.
    let listed = callouts(&mut first);
    for (name, callout) in
        [("Bore diameter", "\u{d8}10 H7"), ("Bore finish", "Ra 1.6"), ("Bore position", "Position \u{d8}0.05 A|B|C")]
    {
        assert!(listed.contains(&(name.to_owned(), callout.to_owned())), "{name} reads {callout}: {listed:?}");
    }

    // A later feature on top: a fifth hole through the plate. Every item stays on its
    // faces, and the file carries all of them.
    call(
        &mut first,
        "sketch",
        json!({"plane": "top", "offset": -1, "shapes": [{"shape": "circle", "center": [30, 36, -1], "diameter": 3}]}),
    );
    call(&mut first, "cut", json!({"sketch": "Sketch18", "depth": 7, "through_all": false, "flip": true}));
    assert_eq!(callouts(&mut first), expected("10"));
    let later = dir.join("p1-bracket-pmi-later-feature.step");
    assert_eq!(export(&mut first, &later)["manufacturing_information"], json!({"items": 14}));
    let read = first.doc.kernel().read_step_pmi(&std::fs::read(&later).unwrap()).unwrap();
    assert_eq!((read.pmi.items.len(), read.skipped.len()), (24, 0));
    let _ = std::fs::remove_dir_all(&scratch);
}
