//! Import scene in the window: sand3d's washer from GLB and NVIDIA's SimReady box from
//! binary USD come in through `import_scene`, open in the assembly tab, show their trees
//! by name, and draw at the right size and the right way up. Frames are kept in
//! `SOLIDDRAFT_SCENE_IMPORT_PROOF_DIR` (default `target/scene-import-test`).

use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::assembly::{AssemblyAction, AssemblySession};
use soliddraft_app::state::{AppAction, AppState};
use soliddraft_app::viewport::camera::StandardView;
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
    serde_json::from_str(&out.content).unwrap()
}

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ci/fixtures/scene-import").join(rel)
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

fn show(h: &mut App<'_>, view: StandardView) {
    h.state_mut().state.push(AppAction::Assembly(AssemblyAction::Show));
    settle(h);
    let s = h.state_mut().state.assembly.session.as_mut().unwrap();
    s.viewport.camera.set_view(view);
    h.state_mut().state.push(AppAction::Assembly(AssemblyAction::ZoomFit));
    h.run_steps(60);
    settle(h);
}

fn snapshot(h: &mut App<'_>, name: &str) {
    let dir = std::env::var_os("SOLIDDRAFT_SCENE_IMPORT_PROOF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/scene-import-test"));
    std::fs::create_dir_all(&dir).unwrap();
    h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
}

/// True when the assembly tab's tree shows a row called `name`.
fn has_row(h: &App<'_>, name: &str) -> bool {
    h.query_all_by_role_and_label(egui::accesskit::Role::Button, name).next().is_some()
}

/// The tree label of the occurrence at `path`.
fn label(h: &App<'_>, path: &str) -> String {
    let s = session(h);
    let row = s.doc.table().find(&path.parse().unwrap()).unwrap_or_else(|| panic!("no occurrence {path}"));
    s.doc.label(row)
}

#[test]
fn imported_scenes_open_named_and_upright_in_the_window() {
    let instance = egui_wgpu::wgpu::Instance::default();
    let adapters = pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all()));
    if adapters.is_empty() {
        assert!(std::env::var_os("SOLIDDRAFT_REQUIRE_GPU").is_none(), "required GPU unavailable");
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("soliddraft-scene-import-app-{}", std::process::id()));
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

    // The washer from GLB: Y up metres, scaled to millimetres at its root.
    let out = tool(
        &mut h.state_mut().state,
        "import_scene",
        json!({ "path": fixture("sand3d-washer/washer.glb"), "out": dir.join("washer.dwasm") }),
    );
    assert!(
        out["done"].as_str().unwrap().starts_with("Imported washer.glb: 7 mesh bodies, 4 joints, metres, Y up"),
        "{out}"
    );
    show(&mut h, StandardView::Isometric);
    // The tree opens on the root's components; the rest is named the same way below.
    assert!(has_row(&h, "washing_machine"), "the tree shows the scene's root node");
    for (path, name) in [("2/2", "cabinet"), ("2/2/6", "door_hinge"), ("2/2/6/2", "door"), ("2/2/6/2/2", "glass")] {
        assert_eq!(label(&h, path), name);
    }
    // The cabinet stands on Z = 0, 850 mm tall: Y up in the file is Z up here.
    let s = session(&h);
    let table = s.doc.table();
    let cabinet = table.find(&"2/2/2".parse().unwrap()).expect("the cabinet");
    let b = table.bounds(cabinet);
    assert!((b.min.z).abs() < 0.01 && (b.max.z - 850.0).abs() < 0.01, "{b:?}");
    assert_eq!(s.doc.label(cabinet), "cabinet");
    snapshot(&mut h, "1-washer-glb");

    // The SimReady box from binary USD.
    let out = tool(
        &mut h.state_mut().state,
        "import_scene",
        json!({ "path": fixture("omniverse-cubebox/cubebox_a02.usd"), "out": dir.join("cubebox.dwasm") }),
    );
    assert!(
        out["done"].as_str().unwrap().starts_with("Imported cubebox_a02.usd: 4 mesh bodies, 0 joints, metres, Z up"),
        "{out}"
    );
    show(&mut h, StandardView::Isometric);
    assert!(has_row(&h, "cubebox_a02_inst"), "the tree shows the default prim's child");
    assert_eq!(label(&h, "2/6/2"), "M_CubeBox_A01_Body");
    snapshot(&mut h, "2-cubebox-usd");
    let _ = std::fs::remove_dir_all(&dir);
}
