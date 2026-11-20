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

use std::sync::Arc;

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, document::Document, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_api::SurfaceKind;

fn state(name: &str) -> AppState {
    let kernel = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_diagnostics(20_000_000));
    AppState::new(Document::new(name, kernel))
}

fn call(state: &mut AppState, tool: &str, arguments: Value) -> Value {
    let out = call_checked(&mut AppTools { state }, tool, arguments, &Caller::Mcp { client: "pmi-bracket".into() });
    assert!(!out.is_error, "{tool}: {}", out.content);
    serde_json::from_str(&out.content).unwrap_or(Value::Null)
}

/// The bracket's build steps of the canonical scenario: everything before its edits.
fn build(state: &mut AppState) {
    let scenario: Value = serde_json::from_str(include_str!("../../../ci/scenarios/canonical-p1.json")).unwrap();
    for step in scenario["steps"].as_array().unwrap() {
        call(state, step["tool"].as_str().unwrap(), step["arguments"].clone());
        if step["label"] == "original" {
            return;
        }
    }
    panic!("the scenario has no step labelled original");
}

/// The name of the one face of `surface` whose centroid is at `at`; `None` is any value.
fn face(state: &AppState, surface: SurfaceKind, at: [Option<f64>; 3]) -> String {
    let found: Vec<_> = state
        .doc
        .faces
        .iter()
        .filter(|f| f.surface == surface)
        .filter(|f| {
            // The app keeps display centroids in single precision.
            let c = [f.centroid.x, f.centroid.y, f.centroid.z].map(f64::from);
            at.iter().zip(c).all(|(want, have)| want.is_none_or(|want| (want - have).abs() < 1e-3))
        })
        .collect();
    assert_eq!(found.len(), 1, "one {surface:?} face at {at:?}, found {}", found.len());
    let feature = state.doc.feature(found[0].feature).unwrap();
    format!("{} \u{b7} {}", feature.name, found[0].label)
}

/// The faces the bracket's items go on, by name.
struct Faces {
    a: String,
    b: String,
    c: String,
    bore: String,
    boss: String,
    boss_top: String,
    holes: Vec<String>,
    slot_low: String,
    slot_high: String,
    slot_ends: [String; 2],
}

fn faces(state: &AppState, bore_x: f64) -> Faces {
    let plane = |at| face(state, SurfaceKind::Plane, at);
    let cylinder = |at| face(state, SurfaceKind::Cylinder, at);
    // The slot's rounded ends are half cylinders about x = 23 and x = 37, so their
    // centroids lie outside those axes.
    let ends = |low: bool| {
        let found: Vec<_> = state
            .doc
            .faces
            .iter()
            .filter(|f| f.surface == SurfaceKind::Cylinder && (f.centroid.z - 18.0).abs() < 1e-3)
            .filter(|f| (f.centroid.x < 30.0) == low)
            .collect();
        assert_eq!(found.len(), 1, "one slot end");
        format!("{} \u{b7} {}", state.doc.feature(found[0].feature).unwrap().name, found[0].label)
    };
    Faces {
        a: plane([None, None, Some(0.0)]),
        b: plane([None, Some(0.0), None]),
        c: plane([Some(0.0), None, None]),
        bore: cylinder([Some(bore_x), Some(25.0), Some(7.5)]),
        boss: cylinder([Some(30.0), Some(25.0), Some(10.0)]),
        boss_top: plane([Some(30.0), Some(25.0), Some(15.0)]),
        holes: [[10.0, 12.0], [10.0, 32.0], [50.0, 12.0], [50.0, 32.0]]
            .into_iter()
            .map(|[x, y]| cylinder([Some(x), Some(y), Some(2.5)]))
            .collect(),
        slot_low: plane([Some(30.0), Some(2.5), Some(15.0)]),
        slot_high: plane([Some(30.0), Some(2.5), Some(21.0)]),
        slot_ends: [ends(true), ends(false)],
    }
}

/// Adds the fourteen items and the part's notes.
fn annotate(state: &mut AppState) {
    let f = faces(state, 30.0);
    let pattern: Vec<Value> = f.holes.iter().map(|hole| json!([hole])).collect();
    let abc = json!(["A", "B", "C"]);
    let items = [
        json!({"kind": "datum", "letter": "A", "faces": [f.a]}),
        json!({"kind": "datum", "letter": "B", "faces": [f.b]}),
        json!({"kind": "datum", "letter": "C", "faces": [f.c]}),
        json!({"kind": "diameter", "name": "Bore diameter", "faces": [f.bore], "tolerance": "H7"}),
        json!({"kind": "finish", "name": "Bore finish", "faces": [f.bore], "roughness": "Ra 1.6"}),
        json!({"kind": "tolerance", "name": "Bore position", "characteristic": "position", "value": "0.05",
               "diameter_zone": true, "datums": abc, "faces": [f.bore]}),
        json!({"kind": "diameter", "name": "Boss diameter", "faces": [f.boss], "tolerance": "\u{b1}0.1"}),
        json!({"kind": "distance", "name": "Boss height", "from_faces": [f.a], "to_faces": [f.boss_top],
               "tolerance": "\u{b1}0.1"}),
        json!({"kind": "diameter", "name": "Hole diameter", "features": pattern, "tolerance": "+0.1/0",
               "through": true}),
        json!({"kind": "tolerance", "name": "Hole position", "characteristic": "position", "value": "0.2",
               "diameter_zone": true, "datums": abc, "features": pattern}),
        json!({"kind": "slot", "name": "Slot", "flats": [[f.slot_low], [f.slot_high]],
               "ends": [[f.slot_ends[0]], [f.slot_ends[1]]], "tolerance": "+0.1/0"}),
        json!({"kind": "distance", "name": "Slot height", "from_faces": [f.a],
               "to_faces": [f.slot_low, f.slot_high], "to_at": "middle", "tolerance": "\u{b1}0.1"}),
        json!({"kind": "tolerance", "name": "Flatness of A", "characteristic": "flatness", "value": "0.05",
               "faces": [f.a]}),
        json!({"kind": "tolerance", "name": "Perpendicularity of B", "characteristic": "perpendicularity",
               "value": "0.1", "datums": ["A"], "faces": [f.b]}),
    ];
    for item in items {
        call(state, "pmi_add", item);
    }
    call(
        state,
        "pmi_notes",
        json!({"general_tolerance": "ISO 2768-m", "material": "6061-T6", "notes": ["Break sharp edges"]}),
    );
}

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

/// Where each item sits: per item, the surface kind and centroid of every face.
fn places(state: &AppState) -> Vec<(String, Vec<(SurfaceKind, [i64; 3])>)> {
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
    assert!(read.pmi.item("Bore position").is_some());
    let _ = std::fs::remove_dir_all(&scratch);
}
