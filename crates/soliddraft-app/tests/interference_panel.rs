//! B7: Interference Detection and Clearance Verification in the window. The ribbon opens
//! the Interference panel; its fields and Check button run the same check as the MCP
//! tools, and a result's row chooses its two parts.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};
use serde_json::{Value, json};
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::assembly::{AssemblyAction, AssemblySession};
use soliddraft_app::interference::Mode;
use soliddraft_app::state::{AppAction, AppState};
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

static PANEL_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());

const HANG: Duration = Duration::from_secs(600);

type App<'a> = Harness<'a, SolidDraftApp>;

fn tool(state: &mut AppState, name: &str, arguments: Value) -> Value {
    let caller = Caller::Mcp { client: "test".into() };
    let out = call_checked(&mut AppTools { state }, name, arguments, &caller);
    assert!(!out.is_error, "{name}: {}", out.content);
    serde_json::from_str(&out.content.to_string()).unwrap_or(Value::Null)
}

fn part(state: &mut AppState, file: &Path, width: f64, height: f64, depth: f64) {
    let shape = json!({ "shape": "rectangle", "center": [0, 0, 0], "width": width, "height": height });
    tool(state, "sketch", json!({ "plane": "top", "shapes": [shape] }));
    tool(state, "extrude", json!({ "sketch": "Sketch1", "depth": depth, "flip": false }));
    tool(state, "save", json!({ "path": file }));
}

fn session<'h>(h: &'h App<'_>) -> &'h AssemblySession {
    h.state().state.assembly.session.as_ref().expect("an open assembly")
}

fn settle(h: &mut App<'_>) {
    let until = Instant::now() + HANG;
    loop {
        h.run_steps(4);
        if !session(h).doc.is_loading() {
            break;
        }
        assert!(Instant::now() < until, "the assembly did not settle");
    }
    h.run_steps(12);
}

fn button(h: &mut App<'_>, name: &str) {
    h.get_all_by_role_and_label(egui::accesskit::Role::Button, name).next().unwrap().click();
    h.run_steps(4);
}

/// Every text the window's accessibility tree holds: labels keep theirs in the value.
fn texts(h: &App<'_>) -> Vec<String> {
    h.query_all(by()).flat_map(|n| [n.accesskit_node().label(), n.accesskit_node().value()]).flatten().collect()
}

fn has(h: &App<'_>, text: &str) -> bool {
    texts(h).iter().any(|t| t.contains(text))
}

fn snapshot(h: &mut App<'_>, name: &str) {
    let dir = std::env::var_os("SOLIDDRAFT_INTERFERENCE_PROOF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-interference-proof-{}", std::process::id())));
    std::fs::create_dir_all(&dir).unwrap();
    h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
}

fn gpu_available() -> bool {
    let instance = egui_wgpu::wgpu::Instance::default();
    let adapters = pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all()));
    if adapters.is_empty() {
        assert!(std::env::var_os("SOLIDDRAFT_REQUIRE_GPU").is_none(), "required GPU unavailable");
        eprintln!("no GPU adapter; skipping");
        return false;
    }
    true
}

#[test]
fn check_interference_and_clearance_from_the_panel() {
    let _guard = PANEL_TEST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !gpu_available() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("soliddraft-interference-panel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut h: App<'static> = Harness::builder()
        .with_size(vec2(1600.0, 1000.0))
        .with_pixels_per_point(1.0)
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| SolidDraftApp::new(cc, soliddraft_app::new_document(), Automation::default()));
    h.run_steps(3);

    // A plate, a block resting on it by a Coincident mate, and a second block sunk 1 mm
    // into it by a distance mate.
    let (a, b) = (dir.join("a.dwprt"), dir.join("b.dwprt"));
    {
        let st = &mut h.state_mut().state;
        part(st, &a, 40.0, 30.0, 10.0);
        tool(st, "make_assembly", json!({}));
        tool(st, "assembly_save", json!({ "path": dir.join("rig.dwasm") }));
        tool(st, "new_part", json!({}));
        part(st, &b, 10.0, 10.0, 6.0);
        tool(st, "show_assembly", json!({}));
        for x in [-10, 10] {
            let placement = json!({ "translation": [x, 0, 30], "rotation": [0, 0, 0, 1] });
            tool(st, "insert_component", json!({ "paths": [b], "placement": placement }));
        }
        tool(st, "assembly_mate_origin", json!({ "name": "O", "component": 2, "align_axes": true }));
        let face = |c: u64, g: &str| json!({ "component": c, "feature": "Boss-Extrude1", "generator": g });
        let seat =
            json!({ "name": "Seat", "a": face(2, "end_cap"), "b": face(4, "start_cap"), "alignment": "opposed" });
        tool(st, "assembly_mate_coincident", seat);
        let sink = json!({ "name": "Sink", "a": face(2, "end_cap"), "b": face(5, "start_cap"),
                           "alignment": "opposed", "distance": "-1 mm" });
        tool(st, "assembly_mate_distance", sink);
        st.push(AppAction::Assembly(AssemblyAction::Show));
        st.push(AppAction::Assembly(AssemblyAction::ZoomFit));
    }
    settle(&mut h);

    // The ribbon opens the panel in the property panel's place.
    button(&mut h, "Interference");
    assert!(h.state().state.assembly.interference.open);
    button(&mut h, "Check");
    settle(&mut h);
    let st = &h.state().state.assembly.interference;
    assert!(st.error.is_none(), "{:?}", st.error);
    let (report, _) = st.report.clone().expect("a report");
    assert!(has(&h, "Exact geometry and certified mesh bounds at each part's pinned revision."));
    assert_eq!(
        (report.pairs, report.interferes, report.touching, report.touching_or_undecided, report.clear),
        (3, 1, 1, 0, 1)
    );
    assert!(has(&h, "3 pairs: 1 interfering, 1 touching, 0 undecided, 1 clear"), "{:?}", texts(&h));
    assert!(has(&h, "Interferes: b<5> and a<2>") || has(&h, "Interferes: a<2> and b<5>"), "{:?}", texts(&h));
    assert!(has(&h, "The exact solids touch and have no overlap volume."), "{:?}", texts(&h));
    snapshot(&mut h, "1-interference");

    // The tools give the same report.
    let tools = tool(&mut h.state_mut().state, "assembly_check_interference", json!({}));
    assert_eq!(tools["report"]["interferes"], 1);
    assert_eq!(tools["report"]["touching_or_undecided"], 0);
    assert_eq!(tools["report"]["touching"], 1);

    let (_, work) = soliddraft_kernel_catallax::with_request_allowance(0, || {
        soliddraft_app::interference::apply(
            &mut h.state_mut().state,
            soliddraft_app::interference::InterferenceAction::Check,
        );
    });
    assert!(work.exhausted);
    settle(&mut h);
    assert!(has(&h, "The exact body comparison reached its calculation limit."), "{:?}", texts(&h));
    snapshot(&mut h, "1-exact-refused");
    button(&mut h, "Check");
    settle(&mut h);

    // A result's row chooses its two parts.
    let interfering = texts(&h).into_iter().find(|t| t.starts_with("Interferes: ")).unwrap();
    h.get_by_value(&interfering).click();
    h.run_steps(4);
    let chosen: Vec<String> = session(&h).chosen.iter().map(ToString::to_string).collect();
    assert_eq!(chosen.len(), 2);
    assert!(chosen.contains(&"2".to_owned()) && chosen.contains(&"5".to_owned()), "{chosen:?}");

    // A clearance of 25 mm: the resting and sunk blocks are under it, and the two blocks,
    // 10 mm apart side by side, are too.
    h.state_mut().state.assembly.interference.mode = Mode::Clearance;
    h.run_steps(3);
    {
        let d = &mut h.state_mut().state.assembly.interference;
        d.clearance.clear();
    }
    h.run_steps(2);
    h.get_by_label("Clearance").focus();
    h.run_steps(2);
    h.get_by_label("Clearance").type_text("12 mm");
    h.run_steps(3);
    button(&mut h, "Check");
    settle(&mut h);
    let (report, _) = h.state().state.assembly.interference.report.clone().expect("a report");
    assert_eq!(report.clearance_mm, Some(12.0));
    assert_eq!((report.below, report.met, report.undecided), (Some(3), Some(0), Some(0)), "{:#?}", report.results);
    assert!(has(&h, "3 pairs: 3 at or under the clearance, 0 undecided, 0 clear of it"), "{:?}", texts(&h));
    snapshot(&mut h, "2-clearance");

    // A clearance typed wrong is refused in the panel and keeps the last report.
    {
        let d = &mut h.state_mut().state.assembly.interference;
        d.clearance = "-1 mm".into();
    }
    button(&mut h, "Check");
    settle(&mut h);
    let st = &h.state().state.assembly.interference;
    assert!(st.error.as_deref().is_some_and(|e| e.contains("zero or more")), "{:?}", st.error);
    assert!(st.report.is_some());
    snapshot(&mut h, "3-refused");

    // Moving a part marks the report out of date.
    {
        let st = &mut h.state_mut().state;
        tool(st, "assembly_delete_mate", json!({ "mate": "Sink" }));
    }
    settle(&mut h);
    assert!(has(&h, "The assembly changed since this check"), "{:?}", texts(&h));
}

#[test]
fn check_hole_alignment_from_the_panel() {
    let _guard = PANEL_TEST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !gpu_available() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("soliddraft-hole-panel-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut h: App<'static> = Harness::builder()
        .with_size(vec2(1600.0, 1000.0))
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| SolidDraftApp::new(cc, soliddraft_app::new_document(), Automation::default()));
    h.run_steps(3);
    let a = dir.join("plate.dwprt");
    {
        let st = &mut h.state_mut().state;
        part(st, &a, 40.0, 30.0, 5.0);
        tool(
            st,
            "sketch",
            json!({ "plane": "top", "shapes": [{ "shape": "circle", "center": [0, 0, 0], "diameter": 4 }] }),
        );
        tool(st, "cut", json!({ "sketch": "Sketch2", "through_all": true, "depth": 5, "flip": true }));
        tool(st, "save", json!({ "path": a }));
        tool(st, "make_assembly", json!({}));
        tool(st, "assembly_save", json!({ "path": dir.join("rig.dwasm") }));
        tool(st, "insert_component", json!({ "paths": [a], "placement": { "translation": [0.25, 0, 10] } }));
        st.push(AppAction::Assembly(AssemblyAction::Show));
        st.push(AppAction::Assembly(AssemblyAction::ZoomFit));
    }
    settle(&mut h);
    button(&mut h, "Interference");
    h.get_by_label("Hole alignment").click();
    h.run_steps(3);
    assert_eq!(h.state().state.assembly.interference.mode, Mode::HoleAlignment, "{:?}", texts(&h));
    h.state_mut().state.assembly.interference.max_deviation = "0.25 mm".into();
    button(&mut h, "Check");
    settle(&mut h);
    let draft = &h.state().state.assembly.interference;
    assert!(draft.error.is_none(), "{:?}", draft.error);
    let (report, _) = draft.hole_report.as_ref().unwrap();
    assert_eq!((report.aligned, report.misaligned), (0, 1));
    assert!(has(&h, "0 aligned, 1 misaligned within 0.25 mm"), "{:?}", texts(&h));
    snapshot(&mut h, "4-hole-alignment");
    let row = texts(&h).into_iter().find(|t| t.starts_with("Misaligned holes: ")).unwrap();
    h.get_by_value(&row).click();
    h.run_steps(3);
    assert_eq!(session(&h).chosen.len(), 2);
    h.state_mut().state.assembly.interference.chosen_only = true;
    button(&mut h, "Check");
    assert_eq!(h.state().state.assembly.interference.hole_report.as_ref().unwrap().0.parts, 2);
    h.state_mut().state.assembly.interference.max_deviation = "-1 mm".into();
    button(&mut h, "Check");
    assert!(h.state().state.assembly.interference.error.as_ref().unwrap().contains("zero or more"));
    assert!(h.state().state.assembly.interference.hole_report.is_some());
    {
        let st = &mut h.state_mut().state;
        tool(st, "insert_component", json!({ "paths": [a], "placement": { "translation": [0, 0, 20] } }));
    }
    settle(&mut h);
    assert!(has(&h, "The assembly changed since this check"));
}
