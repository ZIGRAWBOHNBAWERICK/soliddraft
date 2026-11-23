//! Configurations and the exploded view in the window: the ribbon opens the panel; its
//! name field and Add make a configuration, Activate switches to it, Auto explode moves
//! the parts apart on screen without moving a placement, and Collapse puts them back.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};
use serde_json::{Value, json};
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::assembly::{AssemblyAction, AssemblySession};
use soliddraft_app::state::{AppAction, AppState};
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
    h.get_all_by_role_and_label(egui::accesskit::Role::Button, name)
        .next()
        .unwrap_or_else(|| panic!("no {name}"))
        .click();
    h.run_steps(6);
}

/// Every text the window's accessibility tree holds: labels keep theirs in the value.
fn texts(h: &App<'_>) -> Vec<String> {
    h.query_all(by()).flat_map(|n| [n.accesskit_node().label(), n.accesskit_node().value()]).flatten().collect()
}

fn snapshot(h: &mut App<'_>, name: &str) {
    let dir = std::env::var_os("SOLIDDRAFT_CONFIGURATIONS_PROOF_DIR").map(PathBuf::from).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("soliddraft-configurations-proof-{}", std::process::id()))
    });
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

/// Where each of the root's components shows and where the truth places it.
fn shown_and_placed(h: &App<'_>) -> Vec<(glam::DVec3, glam::DVec3)> {
    let s = session(h);
    s.components()
        .into_iter()
        .map(|r| {
            let step = s.doc.table().step(r);
            let placed = s.doc.assembly().component(step.component).unwrap().placement.translation;
            (s.doc.table().world(r).translation, placed)
        })
        .collect()
}

#[test]
fn the_panel_adds_activates_explodes_and_collapses() {
    if !gpu_available() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("soliddraft-configurations-panel-{}", std::process::id()));
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

    // A plate and a block standing on it.
    let (a, b) = (dir.join("a.dwprt"), dir.join("b.dwprt"));
    {
        let st = &mut h.state_mut().state;
        part(st, &a, 40.0, 30.0, 10.0);
        tool(st, "make_assembly", json!({}));
        tool(st, "assembly_save", json!({ "path": dir.join("rig.dwasm") }));
        tool(st, "new_part", json!({}));
        part(st, &b, 10.0, 10.0, 6.0);
        tool(st, "show_assembly", json!({}));
        let placement = json!({ "translation": [0, 0, 10], "rotation": [0, 0, 0, 1] });
        tool(st, "insert_component", json!({ "paths": [b], "placement": placement }));
        st.push(AppAction::Assembly(AssemblyAction::Show));
        st.push(AppAction::Assembly(AssemblyAction::ZoomFit));
    }
    settle(&mut h);
    let assembled = shown_and_placed(&h);

    // The ribbon opens the panel; a name and Add make a configuration.
    h.state_mut().state.run_command(soliddraft_app::commands::get("assembly_configurations").unwrap());
    h.run_steps(8);
    assert!(h.state().state.assembly.configure.open);
    assert!(texts(&h).iter().any(|t| t.contains("Only the default state")), "{:?}", texts(&h));
    h.get_by_label("Configuration name").focus();
    h.run_steps(2);
    h.get_by_label("Configuration name").type_text("Lifted");
    h.run_steps(3);
    button(&mut h, "Add");
    let names: Vec<String> = session(&h).doc.assembly().configurations.iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, ["Default", "Lifted"]);

    // Activate switches to it; Auto explode moves the block on screen, not its placement.
    button(&mut h, "Activate");
    assert_eq!(session(&h).doc.assembly().active_configuration().unwrap().name, "Lifted");
    button(&mut h, "Auto explode");
    settle(&mut h);
    assert!(session(&h).doc.is_exploded());
    let exploded = shown_and_placed(&h);
    assert!(exploded.iter().zip(&assembled).any(|(now, then)| (now.0 - then.0).length() > 1.0), "{exploded:?}");
    assert!(exploded.iter().zip(&assembled).all(|(now, then)| now.1 == then.1), "a placement moved: {exploded:?}");
    snapshot(&mut h, "1-exploded");

    // Collapse puts the block back where it is placed.
    button(&mut h, "Collapse");
    settle(&mut h);
    assert!(!session(&h).doc.is_exploded());
    assert_eq!(shown_and_placed(&h), assembled);
    snapshot(&mut h, "2-collapsed");
    let _ = std::fs::remove_dir_all(dir);
}
