//! The assistant drives the manufacturing information tools end to end on the sample
//! bracket. It recognises the features and runs the interview, which change nothing;
//! takes the interview's proposals and the material the user named, as a card the user
//! applies; adds a finish the user stated, at once under auto-apply; lists the items;
//! and writes the manufacturing package from a card it cannot apply itself. Every value
//! on the part is the rules' own: the assistant's answer leaves the part as the same
//! answer over MCP does.

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::chat::ChatState;
use soliddraft_app::command_tools::{ProposalStatus, board_bracket};
use soliddraft_app::state::AppState;
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
