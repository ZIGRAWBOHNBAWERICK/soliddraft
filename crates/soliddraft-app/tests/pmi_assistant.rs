//! The assistant drives the manufacturing information tools end to end on the sample
//! bracket. It recognises the features and runs the interview, which change nothing;
//! takes the interview's proposals and the material the user named, as a card the user
//! applies; adds a finish the user stated, at once under auto-apply; lists the items;
//! and writes the manufacturing package from a card it cannot apply itself. Every value
//! on the part is the rules' own: the assistant's answer leaves the part as the same
//! answer over MCP does.

use egui_dock::DockState;
use serde_json::{Value, json};
use soliddraft_app::app::Tab;
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::chat::ChatState;
use soliddraft_app::command_tools::{ProposalStatus, board_bracket};
use soliddraft_app::state::AppState;
use soliddraft_app::top_bar::OpenParts;
use soliddraft_assistant::{Caller, MockProvider, MockTurn, ToolHost};
use std::sync::Arc;

const SIX: [&str; 6] = ["pmi_recognise", "pmi_interview", "pmi_answer", "pmi_add", "pmi_list", "manufacturing_package"];

/// Sends `prompt`, answers it with `turns`, and returns the tool results the model got,
/// as (tool, is_error, result) in call order.
fn exchange(
    st: &mut AppState,
    provider: &MockProvider,
    prompt: &str,
    turns: Vec<MockTurn>,
) -> Vec<(String, bool, Value)> {
    let asked = provider.requests().len();
    provider.push(turns);
    st.chat.draft = prompt.into();
    st.send_chat();
    st.wait_assistant();
    assert_eq!(provider.remaining(), 0, "every scripted turn was used");
    let requests = provider.requests();
    let mut names = std::collections::HashMap::new();
    let mut results = Vec::new();
    for request in &requests[asked + 1..] {
        let message = request.messages.last().expect("a message");
        for block in &request.messages[request.messages.len() - 2].content {
            if block["type"] == "tool_use" {
                names.insert(block["id"].as_str().unwrap().to_owned(), block["name"].as_str().unwrap().to_owned());
            }
        }
        for block in message.content.iter().filter(|b| b["type"] == "tool_result") {
            let content = block["content"].as_str().unwrap();
            results.push((
                names[block["tool_use_id"].as_str().unwrap()].clone(),
                block["is_error"].as_bool().unwrap_or(false),
                serde_json::from_str(content).unwrap_or_else(|_| json!(content)),
            ));
        }
    }
    results
}

fn mcp(state: &mut AppState, tool: &str, input: Value) -> Value {
    let out = AppTools { state }.call(tool, input, &Caller::Mcp { client: "test".into() });
    assert!(!out.is_error, "{tool}: {}", out.content);
    serde_json::from_str(&out.content).unwrap()
}

#[test]
fn headless_and_assistant_reject_an_invalid_position_edit() {
    let provider = Arc::new(MockProvider::new(Vec::new()));
    let mut st = AppState::new(board_bracket());
    st.chat = ChatState::connected(provider.clone(), "mock provider");
    let recognised = mcp(&mut st, "pmi_recognise", json!({}));
    let hole = recognised["features"].as_array().unwrap().iter().find(|f| f["kind"] == "hole").unwrap();
    mcp(&mut st, "pmi_answer", json!({"accept": "all"}));

    let input = |letters: &[&str]| {
        json!({
            "kind": "tolerance", "name": "Bore frame", "characteristic": "position",
            "faces": hole["faces"], "value": 0.05, "diameter_zone": true, "datums": letters
        })
    };
    mcp(&mut st, "pmi_add", input(&["A", "B", "C"]));
    let before = mcp(&mut st, "pmi_list", json!({}));
    let depth = st.doc.undo_depth();

    for (letters, diagnostic) in
        [(["A", "A", "C"], "A datum is referred to twice"), (["A", "B", "ZZ"], "Datum ZZ is not defined")]
    {
        let mut invalid = input(&letters);
        invalid["replaces"] = json!("Bore frame");
        let out = AppTools { state: &mut st }.call("pmi_add", invalid.clone(), &Caller::Mcp { client: "test".into() });
        assert!(out.is_error && out.content.contains(diagnostic), "{}", out.content);
        let answer = exchange(
            &mut st,
            &provider,
            "Edit the bore frame with these datum references.",
            vec![MockTurn::tools("", vec![("pmi_add", invalid)]), MockTurn::text("That edit is invalid.")],
        );
        let [(_, true, error)] = &answer[..] else { panic!("expected refusal, not a card: {answer:?}") };
        assert!(error.to_string().contains(diagnostic), "{error}");
        assert_eq!(mcp(&mut st, "pmi_list", json!({})), before);
        assert_eq!(st.doc.undo_depth(), depth);
    }
}

#[test]
fn position_tool_refuses_incomplete_and_conflicting_intent() {
    let provider = Arc::new(MockProvider::new(Vec::new()));
    let mut st = AppState::new(board_bracket());
    st.chat = ChatState::connected(provider.clone(), "mock provider");
    let recognised = mcp(&mut st, "pmi_recognise", json!({}));
    let feature = recognised["features"].as_array().unwrap().iter().find(|f| f["kind"] == "hole").unwrap();
    mcp(&mut st, "pmi_answer", json!({"accept":"all"}));
    let baseline = mcp(&mut st, "pmi_list", json!({}));
    let depth = st.doc.undo_depth();
    let segment = json!({"value_mm":"0.1","diameter_zone":true,"datums":[{"letter":"A"}]});
    for (frame, diagnostic) in [
        (
            json!({"frame":"single","segment":segment,
            "projected_zone":{"direction":[0,0,1],"entry_faces":[]}}),
            "height_mm",
        ),
        (
            json!({"frame":"single","segment":segment,
            "projected_zone":{"height_mm":"0","direction":[0,0,1],"entry_faces":[]}}),
            "height must be positive",
        ),
        (
            json!({"frame":"single","segment":segment,
            "projected_zone":{"height_mm":"8","entry_faces":[]}}),
            "direction",
        ),
        (json!({"frame":"composite","upper":segment,"lower":segment,"projected_zone":null}), "projected_zone"),
        (
            json!({"frame":"composite","upper":segment,
            "lower":{"value_mm":"0.02","diameter_zone":true,"datums":[{"letter":"A","material":"maximum"}]}}),
            "material boundary modifier",
        ),
        (
            json!({"frame":"composite","upper":segment,
            "lower":{"value_mm":"0.02","diameter_zone":true,"datums":[{"letter":"B"}]}}),
            "ordered prefix",
        ),
    ] {
        let input = json!({"kind":"position_control","name":"New bore frame","faces":feature["faces"],"frame":frame});
        let out = AppTools { state: &mut st }.call("pmi_add", input.clone(), &Caller::Mcp { client: "test".into() });
        assert!(out.is_error && out.content.contains(diagnostic), "{diagnostic}: {}", out.content);
        let answered = exchange(
            &mut st,
            &provider,
            "Add a position frame only if its required values are present.",
            vec![MockTurn::tools("", vec![("pmi_add", input)]), MockTurn::text("I need a valid frame.")],
        );
        let [(_, true, _)] = &answered[..] else { panic!("the assistant received a proposal: {answered:?}") };
        assert_eq!(mcp(&mut st, "pmi_list", json!({})), baseline);
        assert_eq!(st.doc.undo_depth(), depth);
    }
}

#[test]
fn typed_gdt_requires_human_apply_even_with_auto_apply() {
    let provider = Arc::new(MockProvider::new(Vec::new()));
    let mut st = AppState::new(board_bracket());
    st.chat = ChatState::connected(provider.clone(), "mock provider");
    let recognised = mcp(&mut st, "pmi_recognise", json!({}));
    let feature = recognised["features"].as_array().unwrap().iter().find(|f| f["kind"] == "hole").unwrap();
    let mut direct = AppState::new(board_bracket());
    for state in [&mut st, &mut direct] {
        mcp(state, "pmi_answer", json!({"accept":"all"}));
    }
    let input = json!({
        "kind":"position_control", "name":"Checked frame", "faces":feature["faces"],
        "frame":{"frame":"composite",
            "upper":{"value_mm":"0.1","diameter_zone":true,"datums":[{"letter":"A"},{"letter":"B"},{"letter":"C"}]},
            "lower":{"value_mm":"0.02","diameter_zone":true,"datums":[{"letter":"A"},{"letter":"B"}]}}
    });
    let depth = st.doc.undo_depth();
    st.toggle_auto_apply();
    let result = exchange(
        &mut st,
        &provider,
        "Propose the two position segments I specified: Ø0.1 A B C and Ø0.02 A B.",
        vec![
            MockTurn::tools("", vec![("pmi_add", input.clone())]),
            MockTurn::tools("", vec![("apply", json!({"proposal_id":"P1"}))]),
            MockTurn::text("The frame awaits your approval."),
        ],
    );
    let [(_, false, card), (_, true, error)] = &result[..] else {
        panic!("{result:?}");
    };
    assert_eq!(card["status"], "pending");
    assert!(error.to_string().contains("only the user"));
    assert_eq!(st.doc.undo_depth(), depth);
    assert!(st.doc.model().pmi().item("Checked frame").is_none());

    st.apply_proposal(card["proposal_id"].as_str().unwrap());
    mcp(&mut direct, "pmi_add", input);
    assert_eq!(mcp(&mut st, "pmi_list", json!({})), mcp(&mut direct, "pmi_list", json!({})));
    assert_eq!(st.doc.undo_depth(), depth + 1);
}

#[test]
fn the_assistant_drives_the_manufacturing_information_tools() {
    let dir = std::env::temp_dir().join(format!("soliddraft-pmi-assistant-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let provider = Arc::new(MockProvider::new(Vec::new()));
    let mut st = AppState::new(board_bracket());
    st.chat = ChatState::connected(provider.clone(), "mock provider");

    // Reading: recognition and the interview change nothing.
    let read = exchange(
        &mut st,
        &provider,
        "Add the manufacturing information this part needs.",
        vec![
            MockTurn::tools("", vec![("pmi_recognise", json!({})), ("pmi_interview", json!({}))]),
            MockTurn::text("The interview asks for the material; every other question has a proposal."),
        ],
    );
    let offered: Vec<String> = provider.requests()[0].tools.iter().map(|t| t.name.clone()).collect();
    for tool in SIX {
        assert!(offered.iter().any(|name| name == tool), "the assistant is offered {tool}: {offered:?}");
    }
    let [(recognise, false, found), (interview, false, asked)] = &read[..] else { panic!("{read:?}") };
    assert_eq!((recognise.as_str(), interview.as_str()), ("pmi_recognise", "pmi_interview"));
    let hole = found["features"].as_array().unwrap().iter().find(|f| f["kind"] == "hole").expect("the bore is a hole");
    let questions = asked["questions"].as_array().unwrap();
    assert!(questions.iter().any(|q| q["id"] == "material" && q["proposal"].is_null()), "{asked}");
    assert_eq!(mcp(&mut st, "pmi_list", json!({}))["items"], json!([]), "reading changes nothing");
    let mut rules = AppState::new(board_bracket());
    assert_eq!(asked, &mcp(&mut rules, "pmi_interview", json!({})), "the same interview as over MCP");

    // Answering: the proposals and the user's material, as a card.
    let answer = json!({ "accept": "all", "material": "6061-T6" });
    let answered = exchange(
        &mut st,
        &provider,
        "The material is 6061-T6. Take every proposal.",
        vec![MockTurn::tools("", vec![("pmi_answer", answer.clone())]), MockTurn::text("The card adds them.")],
    );
    let [(_, false, card)] = &answered[..] else { panic!("{answered:?}") };
    assert_eq!(card["status"], "pending", "{card}");
    let id = card["proposal_id"].as_str().unwrap().to_owned();
    assert_eq!(mcp(&mut st, "pmi_list", json!({}))["items"], json!([]), "nothing changes before Apply");
    let proposed = questions.iter().filter(|q| q["proposal"]["item"].is_object()).count();
    let rows = &st.chat.book.get(&id).unwrap().rows;
    eprintln!("{id}: {}\n{rows:#?}", st.chat.book.get(&id).unwrap().summary);
    assert!(proposed >= 3, "datums, the bore and the boss are proposed: {asked}");
    assert_eq!(rows.iter().filter(|(what, _)| what == "Adds").count(), proposed, "{rows:?}");
    assert!(rows.contains(&("Material".to_owned(), "6061-T6".to_owned())), "{rows:?}");
    let depth = st.doc.undo_depth();
    st.apply_proposal(&id);
    assert_eq!(st.doc.undo_depth(), depth + 1, "the answers are one undo step");
    assert!(matches!(st.proposal_status(&id), Some(ProposalStatus::Applied { .. })));
    assert!(st.can_undo(&id), "the card undoes its step");
    let listed = mcp(&mut st, "pmi_list", json!({}));
    assert_eq!(listed["items"].as_array().unwrap().len(), proposed, "{listed}");
    mcp(&mut rules, "pmi_answer", answer);
    assert_eq!(listed, mcp(&mut rules, "pmi_list", json!({})), "the rules decide every value");

    // A value the user states, at once under auto-apply, then the list.
    st.toggle_auto_apply();
    let finish = json!({ "kind": "finish", "name": "Bore finish", "faces": hole["faces"], "roughness": "Ra 1.6" });
    let added = exchange(
        &mut st,
        &provider,
        "Give the bore Ra 1.6.",
        vec![
            MockTurn::tools("", vec![("pmi_add", finish)]),
            MockTurn::tools("", vec![("pmi_list", json!({}))]),
            MockTurn::text("The bore has Ra 1.6."),
        ],
    );
    st.toggle_auto_apply();
    let [(_, false, applied), (_, false, listed)] = &added[..] else { panic!("{added:?}") };
    assert_eq!(applied["status"], "applied", "{applied}");
    let items = listed["items"].as_array().unwrap();
    assert!(
        items.iter().any(|i| i["name"] == "Bore finish" && i["callout"].as_str().unwrap().contains("Ra 1.6")),
        "{listed}"
    );

    // The package: a card the assistant cannot apply, written when the user applies it.
    let step = dir.join("bracket.step");
    let written = exchange(
        &mut st,
        &provider,
        &format!("Write the drawing and files to {}.", step.display()),
        vec![
            MockTurn::tools("", vec![("manufacturing_package", json!({ "path": step }))]),
            MockTurn::tools("", vec![("apply", json!({ "proposal_id": "P3" }))]),
            MockTurn::text("The card writes the files."),
        ],
    );
    let [(_, false, card), (_, true, refused)] = &written[..] else { panic!("{written:?}") };
    assert_eq!((card["proposal_id"].as_str(), card["status"].as_str()), (Some("P3"), Some("pending")), "{card}");
    assert!(refused.as_str().unwrap().contains("only the user can apply"), "{refused}");
    assert!(!step.exists(), "nothing is written before Apply");
    st.apply_proposal("P3");
    for file in ["bracket.step", "bracket.pdf", "bracket.dxf"] {
        assert!(dir.join(file).is_file(), "{file} is written");
    }
    assert!(st.drawing.is_active(), "the drawing is open");
    assert!(matches!(st.proposal_status("P3"), Some(ProposalStatus::Applied { .. })));
    assert!(!st.can_undo("P3"), "writing files is no undo step");
    std::fs::remove_dir_all(dir).unwrap();
}

/// A card is checked against the part it was made for. When the part changes under a
/// pending `pmi_answer` card, so that the same answer would now take proposals the
/// card never showed, Apply refuses the card and the part keeps what it had.
#[test]
fn apply_refuses_a_card_the_part_has_outgrown() {
    let provider = Arc::new(MockProvider::new(Vec::new()));
    let mut st = AppState::new(board_bracket());
    st.chat = ChatState::connected(provider.clone(), "mock provider");
    let answered = exchange(
        &mut st,
        &provider,
        "Take every proposal.",
        vec![
            MockTurn::tools("", vec![("pmi_answer", json!({ "accept": "all" }))]),
            MockTurn::text("The card adds them."),
        ],
    );
    let [(_, false, card)] = &answered[..] else { panic!("{answered:?}") };
    let id = card["proposal_id"].as_str().unwrap().to_owned();
    let rows = st.chat.book.get(&id).unwrap().rows.clone();

    // A Ø5 hole that no card question covers, cut before Apply.
    let hole = json!({
        "summary": "A Ø5 hole",
        "commands": [
            { "op": "sketch", "plane": "Boss-Extrude1 · End cap", "shapes": [
                { "shape": "circle", "center": [20, 70, 10], "diameter": 5 }
            ]},
            { "op": "cut", "sketch": "previous", "through_all": true, "depth": 10, "flip": false }
        ]
    });
    let cut = mcp(&mut st, "propose", hole);
    mcp(&mut st, "apply", json!({ "proposal_id": cut["proposal_id"] }));
    let asked = mcp(&mut st, "pmi_interview", json!({}));
    let new_hole = |q: &Value| q["id"].as_str().unwrap_or_default().starts_with("hole Ø5 ");
    assert!(
        asked["questions"].as_array().unwrap().iter().any(|q| new_hole(q) && q["proposal"].is_object()),
        "the new hole has proposals: {asked}"
    );

    let depth = st.doc.undo_depth();
    st.apply_proposal(&id);
    let listed = mcp(&mut st, "pmi_list", json!({}));
    let items = listed["items"].as_array().unwrap();
    let shown =
        |item: &Value| rows.iter().any(|(_, row)| row.starts_with(&format!("{}:", item["name"].as_str().unwrap())));
    assert!(items.iter().all(shown), "only what the card showed can land: {listed}\ncard: {rows:#?}");
    assert_eq!(items.len(), 0, "the stale card changes nothing: {listed}");
    assert_eq!(st.doc.undo_depth(), depth, "no undo step");
    assert!(matches!(st.proposal_status(&id), Some(ProposalStatus::Pending)), "the card stays pending");
    let note = st.note.as_ref().map(|n| n.text.clone()).unwrap_or_default();
    assert!(note.contains("out of date"), "the user is told why: {note}");
}

/// The package card shows the path the files are written to, and Apply writes there,
/// even when the path the assistant gave has spaces around it.
#[test]
fn the_package_card_writes_where_it_says() {
    let dir = std::env::temp_dir().join(format!("soliddraft-pmi-package-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let provider = Arc::new(MockProvider::new(Vec::new()));
    let mut st = AppState::new(board_bracket());
    st.chat = ChatState::connected(provider.clone(), "mock provider");
    mcp(&mut st, "pmi_answer", json!({ "accept": "all", "material": "6061-T6" }));

    let step = dir.join("bracket.step");
    let spaced = format!("  {}  ", step.display());
    let made = exchange(
        &mut st,
        &provider,
        "Write the package.",
        vec![
            MockTurn::tools("", vec![("manufacturing_package", json!({ "path": spaced }))]),
            MockTurn::text("The card writes the files."),
        ],
    );
    let [(_, false, card)] = &made[..] else { panic!("{made:?}") };
    let id = card["proposal_id"].as_str().unwrap().to_owned();
    let rows = st.chat.book.get(&id).unwrap().rows.clone();
    assert!(
        rows.contains(&("Writes".to_owned(), step.display().to_string())),
        "the card names the trimmed path: {rows:?}"
    );

    st.apply_proposal(&id);
    let note = st.note.as_ref().map(|n| n.text.clone()).unwrap_or_default();
    for file in ["bracket.step", "bracket.pdf", "bracket.dxf"] {
        assert!(dir.join(file).is_file(), "{file} is written where the card says: {note}");
    }
    assert!(matches!(st.proposal_status(&id), Some(ProposalStatus::Applied { .. })), "{note}");
    std::fs::remove_dir_all(dir).unwrap();
}

/// A card belongs to the part it was made for. In another part tab, even one holding
/// the same bracket, where the same item would show the same rows, Apply refuses an add
/// card and a package card and changes and writes nothing. Back in its own tab, each
/// card applies.
#[test]
fn a_card_applies_only_in_the_part_it_was_made_for() {
    let dir = std::env::temp_dir().join(format!("soliddraft-pmi-card-part-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let provider = Arc::new(MockProvider::new(Vec::new()));
    let mut st = AppState::new(board_bracket());
    st.chat = ChatState::connected(provider.clone(), "mock provider");
    let found = mcp(&mut st, "pmi_recognise", json!({}));
    let hole = found["features"].as_array().unwrap().iter().find(|f| f["kind"] == "hole").expect("the bore is a hole");
    let finish = json!({ "kind": "finish", "name": "Bore finish", "faces": hole["faces"], "roughness": "Ra 1.6" });
    let step = dir.join("bracket.step");
    let made = exchange(
        &mut st,
        &provider,
        "Give the bore Ra 1.6 and write the package.",
        vec![
            MockTurn::tools("", vec![("pmi_add", finish), ("manufacturing_package", json!({ "path": step }))]),
            MockTurn::text("Two cards."),
        ],
    );
    let [(_, false, add), (_, false, package)] = &made[..] else { panic!("{made:?}") };
    let ids = [add["proposal_id"].as_str().unwrap().to_owned(), package["proposal_id"].as_str().unwrap().to_owned()];

    // A second tab with the same bracket: the add card would show the same rows there.
    let mut tabs = OpenParts::default();
    let mut dock = DockState::new(vec![Tab::Viewport]);
    tabs.park_active(&mut st, board_bracket());
    let depth = st.doc.undo_depth();
    for id in &ids {
        st.apply_proposal(id);
        let note = st.note.as_ref().map(|n| n.text.clone()).unwrap_or_default();
        assert!(note.contains("another part"), "{id} is refused in the other tab: {note}");
        assert!(matches!(st.proposal_status(id), Some(ProposalStatus::Pending)), "{id} stays pending");
    }
    assert_eq!(mcp(&mut st, "pmi_list", json!({}))["items"], json!([]), "nothing lands on the other part");
    assert_eq!(st.doc.undo_depth(), depth, "no undo step on the other part");
    assert!(!step.exists(), "nothing is written for the other part");
    assert!(!st.drawing.is_active(), "no drawing opens");

    // Back in the tab the cards were made in.
    tabs.activate(0, &mut st, &mut dock);
    for id in &ids {
        st.apply_proposal(id);
        let note = st.note.as_ref().map(|n| n.text.clone()).unwrap_or_default();
        assert!(matches!(st.proposal_status(id), Some(ProposalStatus::Applied { .. })), "{id} applies: {note}");
    }
    let listed = mcp(&mut st, "pmi_list", json!({}));
    assert!(listed["items"].as_array().unwrap().iter().any(|i| i["name"] == "Bore finish"), "{listed}");
    for file in ["bracket.step", "bracket.pdf", "bracket.dxf"] {
        assert!(dir.join(file).is_file(), "{file} is written");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// The direct command writes the path exactly as its caller gave it: a relative
/// ` bracket.step` is that file, and a `bracket.step` beside it is left alone. Only the
/// assistant's input is trimmed, once, when its card is made.
#[test]
fn the_direct_package_command_keeps_its_path_exactly() {
    let dir = std::env::temp_dir().join(format!("soliddraft-pmi-exact-path-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bracket.step"), "unrelated").unwrap();
    let mut st = AppState::new(board_bracket());
    mcp(&mut st, "pmi_answer", json!({ "accept": "all", "material": "6061-T6" }));

    // A relative path is read from the working directory. No other test here uses one.
    let here = std::env::current_dir().unwrap();
    std::env::set_current_dir(&dir).unwrap();
    let out = AppTools { state: &mut st }.call(
        "manufacturing_package",
        json!({ "path": " bracket.step" }),
        &Caller::Mcp { client: "test".into() },
    );
    std::env::set_current_dir(here).unwrap();
    assert!(!out.is_error, "{}", out.content);
    assert_eq!(std::fs::read_to_string(dir.join("bracket.step")).unwrap(), "unrelated", "{}", out.content);
    for file in [" bracket.step", " bracket.pdf", " bracket.dxf"] {
        assert!(dir.join(file).is_file(), "{file:?} is written: {}", out.content);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
