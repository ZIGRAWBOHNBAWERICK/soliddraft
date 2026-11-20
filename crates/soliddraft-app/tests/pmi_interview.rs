//! The P1 bracket test's fifth check through the application's tools.
//!
//! C5: the bracket comes in as a bare STEP file, with no manufacturing information.
//! Recognition finds its bore, its boss, its four holes and its slot. The interview
//! asks only for what is missing, each question with a proposal, and the part that
//! results passes C3 and C4: one action writes the STEP file and a drawing with every
//! item once and nothing overlapping.
//!
//! What recognition found is checked apart from what the interview proposes.
//! Recognition is geometry: which faces each feature owns, what a hole opens onto and
//! whether anything closes it. Those are compared face by face with the bracket as it
//! was built, not by counts and sizes alone.

#[path = "support/bracket_pmi.rs"]
mod bracket;

use bracket::{build, call, face, faces, state};
use std::collections::BTreeSet;

use serde_json::{Value, json};
use soliddraft_app::state::AppState;
use soliddraft_kernel_api::SurfaceKind;

/// The bracket with no manufacturing information, imported from its own STEP file.
fn bare_import(dir: &std::path::Path) -> (AppState, AppState) {
    let mut built = state("P1 bracket");
    build(&mut built);
    let step = dir.join("p1-bracket-bare.step");
    let exported = call(&mut built, "export_step", json!({"path": step}));
    assert!(exported.get("manufacturing_information").is_none(), "the part states nothing: {exported}");
    let text = std::fs::read_to_string(&step).unwrap();
    for entity in ["DATUM", "DIMENSIONAL_", "GEOMETRIC_TOLERANCE", "SURFACE_TEXTURE"] {
        assert!(!text.contains(entity), "the bare file has no {entity}");
    }
    let mut imported = state("Imported bracket");
    let preview = call(&mut imported, "preview_import", json!({"path": step}));
    call(&mut imported, "import_step", json!({"path": step, "decline": [], "preview_token": preview["token"]}));
    assert_eq!(call(&mut imported, "pmi_list", json!({}))["items"], json!([]), "nothing came in with the file");
    (built, imported)
}

/// What recognition says of a part, with every face replaced by where it is: its
/// surface and its centroid to a micrometre. A built and an imported bracket name their
/// faces differently; where the faces are is the same.
fn recognised(state: &mut AppState) -> Vec<Value> {
    let found = call(state, "pmi_recognise", json!({}));
    let place = |name: &Value| -> Value {
        let name = name.as_str().unwrap();
        let face = state
            .doc
            .faces
            .iter()
            .find(|f| {
                state
                    .doc
                    .feature(f.feature)
                    .is_some_and(|feature| format!("{} \u{b7} {}", feature.name, f.label) == name)
            })
            .unwrap_or_else(|| panic!("no face called {name}"));
        let c = [face.centroid.x, face.centroid.y, face.centroid.z].map(|v| (f64::from(v) * 1000.0).round() / 1000.0);
        json!(format!("{:?} at ({}, {}, {})", face.surface, c[0], c[1], c[2]))
    };
    let mut features = found["features"].as_array().unwrap().clone();
    for feature in &mut features {
        for key in ["faces", "opens_onto", "closed_by", "stands_on"] {
            if let Some(list) = feature.get_mut(key).and_then(Value::as_array_mut) {
                let mut placed: Vec<Value> = list.iter().map(place).collect();
                placed.sort_by_key(|v| v.as_str().unwrap().to_owned());
                *list = placed;
            }
        }
    }
    features
}

/// What a question asks, without the faces that end its id: `holes Ø4 size`.
fn asks(question: &Value) -> &str {
    let id = question["id"].as_str().unwrap();
    id.split_once(": ").map_or(id, |(asks, _)| asks)
}

/// The faces a question's proposal is on, one per member, by their names.
fn asked_of(question: &Value) -> Vec<String> {
    let item = &question["proposal"]["item"];
    let features =
        item.get("features").and_then(Value::as_array).cloned().unwrap_or_else(|| vec![item["feature"].clone()]);
    features.iter().map(|feature| feature[0]["label"].as_str().unwrap().to_owned()).collect()
}

/// No two open questions share an id.
fn unique(questions: &[Value]) {
    let ids: BTreeSet<&str> = questions.iter().map(|q| q["id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), questions.len(), "{questions:#?}");
}

/// The open questions.
fn interview(state: &mut AppState) -> Vec<Value> {
    call(state, "pmi_interview", json!({}))["questions"].as_array().unwrap().clone()
}

/// The names of the items of `kind` (as `pmi_list` gives it) on a face, and how many.
fn items_on(state: &mut AppState, kind: &str, face: &str) -> Vec<String> {
    let listed = call(state, "pmi_list", json!({}));
    listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["kind"] == kind)
        .filter(|row| row["faces"].as_array().unwrap().iter().flat_map(|f| f.as_array().unwrap()).any(|f| f == face))
        .map(|row| row["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_bare_import_is_recognised_interviewed_and_drawn() {
    let scratch = std::env::temp_dir().join(format!("soliddraft-pmi-interview-{}", std::process::id()));
    let dir = std::env::var_os("SOLIDDRAFT_PMI_PROOF_DIR").map_or(scratch.clone(), std::path::PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    let (mut built, mut imported) = bare_import(&dir);

    // Recognition, apart from any proposal: the imported body's features own the faces
    // the built body's do, open onto the same faces and are closed by the same ones.
    let found = recognised(&mut imported);
    assert_eq!(found, recognised(&mut built), "the import keeps every feature on its faces");
    let named: Vec<&str> = found.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(named, ["Hole \u{d8}8", "4 holes \u{d8}4", "Boss \u{d8}16", "Slot 20\u{d7}6"]);
    let bore = &found[0];
    assert_eq!(bore["faces"], json!(["Cylinder at (30, 25, 7.5)"]), "the bore owns its wall");
    assert_eq!(bore["through"], true);
    assert_eq!(bore.get("closed_by"), None, "nothing closes the bore");
    assert_eq!(bore["opens_onto"].as_array().unwrap().len(), 2, "it opens onto the underside and the boss top");
    assert_eq!(bore["sizes_mm"], json!({"diameter": 8.0, "depth": 15.0}));
    let holes = &found[1];
    assert_eq!(
        holes["faces"],
        json!([
            "Cylinder at (10, 12, 2.5)",
            "Cylinder at (10, 32, 2.5)",
            "Cylinder at (50, 12, 2.5)",
            "Cylinder at (50, 32, 2.5)"
        ])
    );
    assert_eq!((&holes["through"], &holes["sizes_mm"]), (&json!(true), &json!({"diameter": 4.0, "depth": 5.0})));
    let boss = &found[2];
    assert_eq!(boss["faces"], json!(["Cylinder at (30, 25, 10)"]));
    assert_eq!(boss["closed_by"], json!(["Plane at (30, 25, 15)"]), "the boss top closes it");
    assert_eq!(boss["sizes_mm"], json!({"diameter": 16.0, "height": 10.0}));
    assert!(bore["opens_onto"].as_array().unwrap().contains(&boss["closed_by"][0]), "the bore opens onto the boss top");
    let slot = &found[3];
    assert_eq!(slot["faces"].as_array().unwrap().len(), 4, "two flats and two ends");
    assert_eq!((&slot["through"], &slot["sizes_mm"]), (&json!(true), &json!({"length": 20.0, "width": 6.0})));

    // The interview: everything is missing, so everything is asked, in order, each with
    // a proposal but the material.
    let f = faces(&imported, 30.0);
    let questions = call(&mut imported, "pmi_interview", json!({}))["questions"].as_array().unwrap().clone();
    let asked: Vec<(&str, &str)> = questions.iter().map(|q| (asks(q), q["proposes"].as_str().unwrap())).collect();
    assert_eq!(
        asked,
        [
            ("material", ""),
            ("general tolerance", "ISO 2768-mK"),
            ("datum A", "Datum A"),
            ("datum B", "Datum B"),
            ("datum C", "Datum C"),
            ("holes \u{d8}4 size", "4\u{d7} \u{d8}4 H13 THRU"),
            ("holes \u{d8}4 position", "Position \u{d8}0.5 A|B|C"),
            ("hole \u{d8}8 size", "\u{d8}8 H7 THRU"),
            ("hole \u{d8}8 position", "Position \u{d8}0.05 A|B|C"),
            ("hole \u{d8}8 finish", "Ra 1.6"),
            ("boss \u{d8}16 size", "\u{d8}16"),
            ("boss \u{d8}16 height", "15"),
            ("slot 20\u{d7}6 size", "20\u{d7}6 R3"),
            ("slot 20\u{d7}6 height", "18"),
        ]
    );
    assert!(questions.iter().all(|q| !q["reason"].as_str().unwrap().is_empty()), "every question says why");
    // A feature's question names it by its faces: the bore's by its wall.
    let bore_size = questions.iter().find(|q| asks(q) == "hole \u{d8}8 size").unwrap();
    assert_eq!(asked_of(bore_size), std::slice::from_ref(&f.bore), "{bore_size}");
    assert_eq!(bore_size["id"], format!("hole \u{d8}8 size: {}", f.bore));
    unique(&questions);
    // The datums are the bracket's own: the underside, the flange's back and the left end.
    let datum_face = |letter: &str| {
        let question = questions.iter().find(|q| q["id"] == format!("datum {letter}")).unwrap();
        let reference = &question["proposal"]["item"]["feature"][0];
        reference["label"].as_str().unwrap_or_default().to_owned()
    };
    for (letter, name) in [("A", &f.a), ("B", &f.b), ("C", &f.c)] {
        assert!(name.ends_with(&datum_face(letter)), "datum {letter} is {name}, proposed {}", datum_face(letter));
    }

    // Only what is missing: with datum A and the bore's fit given by hand, those two
    // are not asked again, and the rest still is.
    call(&mut imported, "pmi_add", json!({"kind": "datum", "letter": "A", "faces": [f.a]}));
    call(&mut imported, "pmi_add", json!({"kind": "diameter", "name": "Bore", "faces": [f.bore], "tolerance": "H7"}));
    let open = call(&mut imported, "pmi_interview", json!({}))["questions"].as_array().unwrap().clone();
    let ids: Vec<&str> = open.iter().map(asks).collect();
    assert!(!ids.contains(&"datum A") && !ids.contains(&"hole \u{d8}8 size"), "{ids:?}");
    assert_eq!(ids.len(), questions.len() - 2, "{ids:?}");

    // Every proposal accepted, and the material said: nothing is left to ask.
    let answered = call(&mut imported, "pmi_answer", json!({"accept": "all", "material": "6061-T6"}));
    assert_eq!(answered["questions"], json!([]), "{answered}");
    let listed = call(&mut imported, "pmi_list", json!({}));
    assert_eq!(listed["items"].as_array().unwrap().len(), 12, "{listed}");
    assert!(listed["items"].as_array().unwrap().iter().all(|row| row.get("problem").is_none()), "{listed}");
    assert_eq!(listed["notes"], json!({"general_tolerance": "ISO 2768-mK", "material": "6061-T6"}));
    // One undo takes all the answers back, and one redo returns them.
    call(&mut imported, "undo", json!({}));
    assert_eq!(call(&mut imported, "pmi_list", json!({}))["items"].as_array().unwrap().len(), 2);
    call(&mut imported, "redo", json!({}));

    // The result passes C3 and C4.
    let step = dir.join("p1-bracket-interviewed.step");
    let report = call(&mut imported, "manufacturing_package", json!({"path": step}));
    assert_eq!(report["manufacturing_information"], json!({"items": 12}), "{report}");
    let drawing = &report["drawing"];
    assert_eq!(drawing["not_shown"], json!([]), "{drawing:#}");
    assert_eq!(drawing["overlaps"], json!([]), "{drawing:#}");
    assert_eq!(drawing["not_in_file"], json!([]), "{drawing:#}");
    let shown = drawing["shown"].as_object().unwrap();
    for row in listed["items"].as_array().unwrap() {
        let name = row["name"].as_str().unwrap();
        assert_eq!(shown.get(name), Some(&json!(1)), "{name} is on the drawing once: {shown:?}");
    }
    assert_eq!(drawing["views"], 4);
    let _ = face(&imported, SurfaceKind::Plane, [None, None, Some(0.0)]);
    let _ = std::fs::remove_dir_all(&scratch);
}

/// One hole of the pattern sized and another located by hand: each item answers for
/// its own hole only, so the interview still asks the size of the other three and the
/// position of the other three, and accepting every proposal covers each hole once.
#[test]
fn a_partly_annotated_pattern_is_asked_about_its_other_holes() {
    let dir = std::env::temp_dir().join(format!("soliddraft-pmi-interview-partial-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (_, mut imported) = bare_import(&dir);
    let f = faces(&imported, 30.0);
    for (letter, face) in [("A", &f.a), ("B", &f.b), ("C", &f.c)] {
        call(&mut imported, "pmi_add", json!({"kind": "datum", "letter": letter, "faces": [face]}));
    }
    call(
        &mut imported,
        "pmi_add",
        json!({"kind": "diameter", "name": "First hole", "faces": [f.holes[0]], "tolerance": "H13", "through": true}),
    );
    call(
        &mut imported,
        "pmi_add",
        json!({"kind": "tolerance", "name": "Second hole position", "characteristic": "position", "value": "0.5",
               "diameter_zone": true, "datums": ["A", "B", "C"], "faces": [f.holes[1]]}),
    );

    let questions = interview(&mut imported);
    unique(&questions);
    let find = |asked: &str| {
        let found: Vec<&Value> = questions.iter().filter(|q| asks(q) == asked).collect();
        assert_eq!(found.len(), 1, "one {asked} question: {questions:#?}");
        found[0].clone()
    };
    // The holes but one, in no particular order: recognition orders a pattern's holes.
    let others = |except: usize| -> BTreeSet<String> {
        f.holes.iter().enumerate().filter(|(i, _)| *i != except).map(|(_, hole)| hole.clone()).collect()
    };
    let set = |question: &Value| -> BTreeSet<String> { asked_of(question).into_iter().collect() };
    let size = find("holes \u{d8}4 size");
    assert_eq!(set(&size), others(0), "the sized hole is not asked again, the other three are: {size}");
    assert_eq!(size["proposes"], "3\u{d7} \u{d8}4 H13 THRU");
    assert_eq!(size["asks"], "What are the remaining 3 of the 4 holes \u{d8}4 for?");
    let position = find("holes \u{d8}4 position");
    assert_eq!(set(&position), others(1), "the located hole is not asked again, the other three are: {position}");

    let answered = call(&mut imported, "pmi_answer", json!({"accept": "all", "material": "6061-T6"}));
    assert_eq!(answered["questions"], json!([]), "{answered}");
    for hole in &f.holes {
        assert_eq!(items_on(&mut imported, "Diameter", hole).len(), 1, "{hole} has one size");
        assert_eq!(items_on(&mut imported, "Position", hole).len(), 1, "{hole} has one position");
    }
    assert_eq!(items_on(&mut imported, "Diameter", &f.holes[0]), ["First hole"]);
    assert_eq!(items_on(&mut imported, "Position", &f.holes[1]), ["Second hole position"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A plate with a through hole and a blind hole of one diameter: two features, not a
/// pattern, whose questions differ by their faces. Each answer lands on its own hole,
/// whether accepted by id or with every other proposal.
#[test]
fn equal_holes_on_different_features_are_asked_and_answered_apart() {
    let mut plate = state("Plate");
    for (tool, arguments) in [
        (
            "sketch",
            json!({"plane": "top", "offset": 0, "shapes": [{"shape": "rectangle", "center": [30, 20, 0], "width": 60, "height": 40}]}),
        ),
        ("extrude", json!({"sketch": "Sketch1", "depth": 10, "flip": false})),
        (
            "sketch",
            json!({"plane": "top", "offset": -1, "shapes": [{"shape": "circle", "center": [15, 20, -1], "diameter": 6}]}),
        ),
        ("cut", json!({"sketch": "Sketch2", "depth": 12, "through_all": false, "flip": true})),
        (
            "sketch",
            json!({"plane": "top", "offset": -1, "shapes": [{"shape": "circle", "center": [45, 20, -1], "diameter": 6}]}),
        ),
        ("cut", json!({"sketch": "Sketch3", "depth": 5, "through_all": false, "flip": true})),
    ] {
        call(&mut plate, tool, arguments);
    }
    let through = face(&plate, SurfaceKind::Cylinder, [Some(15.0), Some(20.0), None]);
    let blind = face(&plate, SurfaceKind::Cylinder, [Some(45.0), Some(20.0), None]);
    let found = call(&mut plate, "pmi_recognise", json!({}));
    let holes: Vec<&Value> = found["features"].as_array().unwrap().iter().filter(|f| f["kind"] == "hole").collect();
    assert_eq!(holes.len(), 2, "two holes, no pattern: {found}");
    assert!(holes.iter().all(|hole| hole["name"] == "Hole \u{d8}6"), "{found}");

    let questions = interview(&mut plate);
    unique(&questions);
    let of = |questions: &[Value], asked: &str, wall: &str| -> Option<Value> {
        let found: Vec<&Value> =
            questions.iter().filter(|q| asks(q) == asked && asked_of(q) == [wall.to_owned()]).collect();
        assert!(found.len() <= 1, "{found:#?}");
        found.first().map(|q| (*q).clone())
    };
    for wall in [&through, &blind] {
        for asked in ["hole \u{d8}6 size", "hole \u{d8}6 position"] {
            assert!(of(&questions, asked, wall).is_some(), "{asked} of {wall}: {questions:#?}");
        }
    }

    // Accepting the blind hole's size by its id sizes that hole, and only it.
    let blind_size = of(&questions, "hole \u{d8}6 size", &blind).unwrap();
    assert_eq!(blind_size["proposes"], "\u{d8}6 H13");
    let answered = call(&mut plate, "pmi_answer", json!({"accept": [blind_size["id"]]}));
    assert_eq!(answered["accepted"], json!([blind_size["id"]]));
    assert_eq!(items_on(&mut plate, "Diameter", &blind).len(), 1, "{answered}");
    assert_eq!(items_on(&mut plate, "Diameter", &through).len(), 0, "{answered}");
    let open = interview(&mut plate);
    assert!(of(&open, "hole \u{d8}6 size", &blind).is_none(), "{open:#?}");
    let through_size = of(&open, "hole \u{d8}6 size", &through).expect("the through hole is still asked");
    assert_eq!(through_size["proposes"], "\u{d8}6 H13 THRU");

    // Accept all: each remaining proposal once, on its own hole.
    let answered = call(&mut plate, "pmi_answer", json!({"accept": "all", "material": "6061-T6"}));
    assert_eq!(answered["questions"], json!([]), "{answered}");
    for wall in [&through, &blind] {
        assert_eq!(items_on(&mut plate, "Diameter", wall).len(), 1, "{wall} has one size: {answered}");
        assert_eq!(items_on(&mut plate, "Position", wall).len(), 1, "{wall} has one position: {answered}");
    }
    let listed = call(&mut plate, "pmi_list", json!({}));
    let rows = listed["items"].as_array().unwrap();
    assert!(rows.iter().all(|row| row.get("problem").is_none()), "{listed}");
    let callout_on = |wall: &str| {
        rows.iter()
            .find(|row| row["kind"] == "Diameter" && row["faces"] == json!([[wall]]))
            .map(|row| row["callout"].as_str().unwrap().to_owned())
    };
    assert_eq!(callout_on(&through).as_deref(), Some("\u{d8}6 H13 THRU"));
    assert_eq!(callout_on(&blind).as_deref(), Some("\u{d8}6 H13"));
}
