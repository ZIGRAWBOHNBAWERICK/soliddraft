//! The P1 bracket test's fifth check through the application's tools.
//!
//! C5: the bracket comes in as a bare STEP file, with no manufacturing information.
//! Recognition finds its bore, its boss, its four holes and its slot. The interview
//! separates geometric checks from unstated requirements. With explicit requirements
//! supplied, one action writes the STEP file and a drawing with every item once.
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
use soliddraft_app::{app_tools::AppTools, chat::ChatState, state::AppState};
use soliddraft_assistant::{Caller, MockProvider, MockTurn, ToolHost};
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

    // The bare geometry supports nominal checks, not a fit, position tolerance or finish.
    let f = faces(&imported, 30.0);
    let questions = interview(&mut imported);
    assert_eq!(questions, interview(&mut imported));
    let asked: Vec<(&str, &str)> = questions.iter().map(|q| (asks(q), q["proposes"].as_str().unwrap())).collect();
    assert_eq!(
        asked,
        [
            ("material", ""),
            ("general tolerance", "ISO 2768-mK"),
            ("datum A", "Datum A"),
            ("datum B", "Datum B"),
            ("datum C", "Datum C"),
            ("holes Ø4 size", "4× Ø4 THRU"),
            ("holes Ø4 fit or thread", ""),
            ("holes Ø4 position", ""),
            ("hole Ø8 size", "Ø8 THRU"),
            ("hole Ø8 fit or thread", ""),
            ("hole Ø8 position", ""),
            ("hole Ø8 finish", ""),
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
    for known in ["datum A", "hole Ø8 size", "hole Ø8 fit or thread"] {
        assert!(!ids.contains(&known), "{ids:?}");
    }
    assert_eq!(ids.len(), questions.len() - 3, "{ids:?}");

    // Requirements the operator supplies, not values drawn from the bare bracket.
    let pattern: Vec<_> = f.holes.iter().map(|hole| [hole]).collect();
    call(&mut imported, "pmi_answer", json!({"accept": ["datum B", "datum C"]}));
    for input in [
        json!({"kind": "diameter", "name": "Hole diameter", "features": pattern, "tolerance": "H13", "through": true}),
        json!({"kind": "tolerance", "name": "Hole position", "characteristic": "position", "value": "0.5",
               "diameter_zone": true, "datums": ["A", "B", "C"], "features": pattern}),
        json!({"kind": "tolerance", "name": "Bore position", "characteristic": "position", "value": "0.05",
               "diameter_zone": true, "datums": ["A", "B", "C"], "faces": [f.bore]}),
        json!({"kind": "finish", "name": "Bore finish", "faces": [f.bore], "roughness": "Ra 1.6"}),
    ] {
        call(&mut imported, "pmi_add", input);
    }
    let before = call(&mut imported, "pmi_list", json!({}));
    let answered = call(&mut imported, "pmi_answer", json!({"accept": "all", "material": "6061-T6"}));
    assert_eq!(answered["questions"], json!([]), "{answered}");
    let listed = call(&mut imported, "pmi_list", json!({}));
    assert_eq!(listed["items"].as_array().unwrap().len(), 12, "{listed}");
    assert!(listed["items"].as_array().unwrap().iter().all(|row| row.get("problem").is_none()), "{listed}");
    assert_eq!(listed["notes"], json!({"general_tolerance": "ISO 2768-mK", "material": "6061-T6"}));
    // One undo takes all the answers back, and one redo returns them.
    call(&mut imported, "undo", json!({}));
    assert_eq!(call(&mut imported, "pmi_list", json!({})), before);
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
/// its own hole only, so the interview still asks about the other three.
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
    assert_eq!(size["proposes"], "3× Ø4 THRU");
    for (aspect, except) in [("fit or thread", 0), ("position", 1)] {
        let question = find(&format!("holes Ø4 {aspect}"));
        let id = question["id"].as_str().unwrap();
        let asked_faces = id.split_once(": ").unwrap().1.split(", ").map(str::to_owned).collect::<BTreeSet<_>>();
        assert_eq!(asked_faces, others(except), "{question}");
        assert!(question.get("proposal").is_none(), "{question}");
    }

    call(&mut imported, "pmi_answer", json!({"accept": [size["id"]]}));
    for hole in &f.holes {
        assert_eq!(items_on(&mut imported, "Diameter", hole).len(), 1, "{hole} has one size");
    }
    assert_eq!(items_on(&mut imported, "Diameter", &f.holes[0]), ["First hole"]);
    assert_eq!(items_on(&mut imported, "Position", &f.holes[1]), ["Second hole position"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Blind drill depth is geometric. Fit, thread designation and usable thread depth
/// are not, even when the pilot diameter is known.
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
    assert_eq!(questions, interview(&mut plate));
    let find = |questions: &[Value], label: &str, wall: &str| {
        let id = format!("{label}: {wall}");
        questions.iter().find(|q| q["id"] == id).cloned()
    };
    let drill = find(&questions, "hole Ø6 geometry", &blind).unwrap();
    assert_eq!(drill["proposes"], "Ø6 DRILL DEEP 4");
    for wall in [&through, &blind] {
        for aspect in ["fit or thread", "position"] {
            let question = find(&questions, &format!("hole Ø6 {aspect}"), wall).unwrap();
            assert!(question.get("proposal").is_none(), "{question}");
        }
    }
    let answered = call(&mut plate, "pmi_answer", json!({"accept": [drill["id"]]}));
    assert_eq!(answered["accepted"], json!([drill["id"]]));
    let open = interview(&mut plate);
    assert!(find(&open, "hole Ø6 geometry", &blind).is_none(), "{open:#?}");
    assert_eq!(find(&open, "hole Ø6 size", &through).unwrap()["proposes"], "Ø6 THRU");
    assert!(find(&open, "hole Ø6 fit or thread", &blind).is_some(), "{open:#?}");

    let id = drill["proposal"]["item"]["id"].as_str().unwrap();
    let thread = json!({"kind": "hole", "replaces": id, "pilot_faces": [blind], "thread": "M7x1-6H"});
    call(&mut plate, "pmi_add", thread.clone());
    let open = interview(&mut plate);
    assert!(find(&open, "hole Ø6 fit or thread", &blind).is_none(), "{open:#?}");
    let missing_depth = find(&open, "hole Ø6 thread depth", &blind).unwrap();
    assert!(missing_depth.get("proposal").is_none(), "{missing_depth}");

    let mut thread = thread;
    thread["thread_depth"] = json!("2.5");
    thread["drill_depth"] = json!("4");
    let edited = call(&mut plate, "pmi_add", thread);
    assert_eq!(edited["added"]["callout"], "Ø6 DRILL DEEP 4\nM7x1-6H THREAD DEEP 2.5", "{edited}");
    let open = interview(&mut plate);
    assert!(find(&open, "hole Ø6 thread depth", &blind).is_none(), "{open:#?}");
}

#[test]
fn two_compound_hole_checks_match_direct_values_and_survive_reopen() {
    let dir = std::env::temp_dir().join(format!("soliddraft-pmi-interview-compound-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut built = state("Plate");
    call(
        &mut built,
        "sketch",
        json!({
            "plane": "top", "shapes": [{"shape": "rectangle", "center": [40, 20, 0], "width": 80, "height": 40}]
        }),
    );
    call(&mut built, "extrude", json!({"sketch": "Sketch1", "depth": 10, "flip": false}));
    let top = "Boss-Extrude1 · End cap";
    call(
        &mut built,
        "hole",
        json!({
            "face": top, "points": [[12, 20, 10]], "diameter": 4, "depth": 7,
            "counterbore": {"diameter": 10, "depth": 2}
        }),
    );
    call(
        &mut built,
        "hole",
        json!({
            "face": top, "points": [[30, 20, 10]], "diameter": 4, "countersink": {"diameter": 8, "angle": 90}
        }),
    );
    let step = dir.join("compound.step");
    call(&mut built, "export_step", json!({"path": step}));
    let import = || {
        let mut imported = state("Bare plate");
        let preview = call(&mut imported, "preview_import", json!({"path": step}));
        call(&mut imported, "import_step", json!({"path": step, "decline": [], "preview_token": preview["token"]}));
        imported
    };
    let mut accepted = import();
    let mut direct = import();
    let questions = interview(&mut accepted);
    assert_eq!(questions, interview(&mut accepted));
    assert_eq!(questions, interview(&mut direct));
    unique(&questions);

    let mut accepted_ids = Vec::new();
    let mut expectations = Vec::new();
    let recognised = call(&mut accepted, "pmi_recognise", json!({}));
    for (kind, x, pilot_z, surface, head_z, through, callout) in [
        ("counterbore", 12.0, 5.5, SurfaceKind::Cylinder, Some(9.0), false, "Ø4 DRILL DEEP 7\nC'BORE Ø10 DEEP 2"),
        ("countersink", 30.0, 4.0, SurfaceKind::Cone, None, true, "Ø4 THRU\nC'SINK Ø8 × 90°"),
    ] {
        let pilot = face(&accepted, SurfaceKind::Cylinder, [Some(x), Some(20.0), Some(pilot_z)]);
        let head = face(&accepted, surface, [Some(x), Some(20.0), head_z]);
        let entry = recognised["features"].as_array().unwrap().iter().find(|f| f["kind"] == kind).unwrap();
        assert_eq!(entry["faces"], json!([pilot, head]));
        let (depth, head_diameter, angle) = if through { (10.0, 8.0, json!(90.0)) } else { (7.0, 10.0, Value::Null) };
        assert_eq!(entry["through"], through);
        assert_eq!(
            entry["sizes_mm"],
            json!({"diameter": 4.0, "depth": depth, "head_diameter": head_diameter, "head_depth": 2.0})
        );
        assert_eq!(entry["angle_deg"], angle);
        let id = format!("{kind} Ø4 geometry: {pilot}");
        let question = questions.iter().find(|q| q["id"] == id).unwrap_or_else(|| panic!("{id}: {questions:#?}"));
        assert_eq!(question["proposes"], callout);
        let name = question["proposal"]["item"]["id"].as_str().unwrap();
        call(
            &mut direct,
            "pmi_add",
            json!({
                "kind": "hole", "name": name, "pilot_faces": [pilot], "head_kind": kind, "head_faces": [head],
                "through": through
            }),
        );
        accepted_ids.push(id);
        expectations.push((name, pilot, head, callout));
    }
    let provider = std::sync::Arc::new(MockProvider::new(Vec::new()));
    accepted.chat = ChatState::connected(provider.clone(), "mock provider");
    let intent = questions.iter().find(|q| asks(q) == "counterbore Ø4 fit or thread").unwrap();
    let refuse = json!({"accept": [intent["id"]]});
    let error =
        AppTools { state: &mut accepted }.call("pmi_answer", refuse.clone(), &Caller::Mcp { client: "test".into() });
    assert!(error.is_error && error.content.contains("has no proposal"), "{}", error.content);
    provider.push(vec![
        MockTurn::tools("", vec![("pmi_answer", refuse)]),
        MockTurn::tools("", vec![("pmi_answer", json!({"accept": accepted_ids}))]),
        MockTurn::text("The measured dimensions are ready for approval; I still need the fit or thread."),
    ]);
    accepted.chat.draft = "Record the measured compound-hole dimensions; leave the fit unspecified.".into();
    accepted.send_chat();
    accepted.wait_assistant();
    assert_eq!(provider.remaining(), 0);
    let requests = provider.requests();
    let result = |index: usize| {
        requests[index].messages.last().unwrap().content.iter().find(|b| b["type"] == "tool_result").unwrap()
    };
    assert_eq!(result(1)["is_error"], true);
    assert!(result(1)["content"].as_str().unwrap().contains("has no proposal"));
    assert!(!result(2)["is_error"].as_bool().unwrap_or(false));
    let card: Value = serde_json::from_str(result(2)["content"].as_str().unwrap()).unwrap();
    assert_eq!(card["status"], "pending");
    assert_eq!(call(&mut accepted, "pmi_list", json!({}))["items"], json!([]));
    accepted.apply_proposal(card["proposal_id"].as_str().unwrap());
    let open = interview(&mut accepted);
    for (kind, (_, pilot, _, _)) in ["counterbore", "countersink"].into_iter().zip(&expectations) {
        let id = format!("{kind} Ø4 fit or thread: {pilot}");
        let prior = questions.iter().find(|q| q["id"] == id).unwrap();
        assert!(prior.get("proposal").is_none(), "{prior}");
        assert!(open.contains(prior), "accepting a measured size did not answer the fit or thread");
    }
    assert!(open.iter().all(|q| !accepted_ids.iter().any(|id| q["id"] == *id)));
    assert_eq!(accepted.doc.model().pmi(), direct.doc.model().pmi());
    let listed = call(&mut accepted, "pmi_list", json!({}));
    assert_eq!(listed, call(&mut direct, "pmi_list", json!({})));
    for (name, pilot, head, callout) in expectations {
        let row = listed["items"].as_array().unwrap().iter().find(|item| item["name"] == name).unwrap();
        assert_eq!(row["callout"], callout);
        assert_eq!(row["faces"], json!([[pilot], [head]]));
    }
    let native = dir.join("compound.dwprt");
    call(&mut accepted, "save_as", json!({"path": native}));
    let mut reopened = state("Reopened");
    call(&mut reopened, "open", json!({"path": native}));
    assert_eq!(call(&mut reopened, "pmi_list", json!({})), listed);
    assert_eq!(interview(&mut reopened), open);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_recess_depth_and_fillet_radius_do_not_answer_edge_intent() {
    let dir = std::env::temp_dir().join(format!("soliddraft-pmi-interview-recess-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut plate = state("Recess and edge plate");
    call(
        &mut plate,
        "sketch",
        json!({
            "plane": "top", "shapes": [{"shape": "rectangle", "center": [30, 20, 0], "width": 60, "height": 40}]
        }),
    );
    call(&mut plate, "extrude", json!({"sketch": "Sketch1", "depth": 10, "flip": false}));
    call(
        &mut plate,
        "sketch",
        json!({
            "plane": "top", "offset": 11,
            "shapes": [{"shape": "rectangle", "center": [30, 20, 11], "width": 12, "height": 8}]
        }),
    );
    call(&mut plate, "cut", json!({"sketch": "Sketch2", "depth": 3, "through_all": false, "flip": false}));
    let edge_at = |plate: &AppState, x, y| {
        let model = plate.doc.model();
        let topology = model.kernel().topology(model.body().unwrap()).unwrap();
        let edge = topology
            .edges
            .iter()
            .find(|edge| {
                let bounds = edge.bbox;
                (bounds.min.x, bounds.max.x, bounds.min.y, bounds.max.y) == (x, x, y, y) && bounds.min.z < bounds.max.z
            })
            .unwrap();
        let named = plate.doc.named_edge(edge.id).unwrap();
        format!("{} · {}", plate.doc.feature(named.feature).unwrap().name, named.label)
    };
    let fillet = edge_at(&plate, 60.0, 40.0);
    call(&mut plate, "fillet", json!({"edges": [fillet], "radius": 2}));

    let step = dir.join("recess.step");
    call(&mut plate, "export_step", json!({"path": step}));
    let mut imported = state("Bare recess");
    let preview = call(&mut imported, "preview_import", json!({"path": step}));
    call(&mut imported, "import_step", json!({"path": step, "decline": [], "preview_token": preview["token"]}));
    let questions = interview(&mut imported);
    let get = |name: &str| {
        let matching: Vec<_> = questions.iter().filter(|q| asks(q) == name).collect();
        assert_eq!(matching.len(), 1, "{name}: {questions:#?}");
        matching[0]
    };
    let depth = get("pocket depth");
    let radius = get("fillet radius");
    assert_eq!(depth["proposes"], "2");
    assert_eq!(radius["proposes"], "R2");
    let fillet_intent = get("fillet treatment");
    assert!(fillet_intent.get("proposal").is_none(), "{fillet_intent}");
    call(&mut imported, "pmi_answer", json!({"accept": [depth["id"], radius["id"]]}));
    let open = interview(&mut imported);
    assert!(!open.contains(depth) && !open.contains(radius), "{open:#?}");
    assert!(open.contains(fillet_intent), "{open:#?}");

    // Exercise the bevel on an unmodified block, without the recess or fillet's supports.
    for _ in 0..3 {
        call(&mut plate, "undo", json!({}));
    }
    let chamfer = edge_at(&plate, 0.0, 0.0);
    call(&mut plate, "chamfer", json!({"edges": [chamfer], "distance": 1}));
    let questions = interview(&mut plate);
    let treatment = questions.iter().find(|q| asks(q) == "chamfer treatment").unwrap();
    assert_eq!(treatment["proposes"], "1 × 45° CHAMFER");
    let answered = call(&mut plate, "pmi_answer", json!({"accept": [treatment["id"]]}));
    let open = interview(&mut plate);
    assert!(open.iter().all(|q| q["id"] != treatment["id"]), "{open:#?}");
    let items = answered["items"].as_array().unwrap();
    let row = items.iter().find(|item| item["name"] == treatment["proposal"]["item"]["id"]).unwrap();
    assert_eq!(row["callout"], treatment["proposes"]);
    assert!(row.get("problem").is_none(), "{row}");
    std::fs::remove_dir_all(dir).unwrap();
}
