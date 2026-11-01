//! G1: a distance mate made in the window, and a Coincident one. Real picks in the id
//! buffer, the Mate panel's fields and buttons, the ribbon's Update, and what the scene
//! then draws.

use egui::{Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use glam::Vec3;
use serde_json::json;
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

fn tool(state: &mut AppState, name: &str, arguments: serde_json::Value) {
    let caller = Caller::Mcp { client: "test".into() };
    let out = call_checked(&mut AppTools { state }, name, arguments, &caller);
    assert!(!out.is_error, "{name}: {}", out.content);
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
        let s = session(h);
        if !s.doc.is_loading() {
            break;
        }
        assert!(Instant::now() < until, "the assembly did not settle");
    }
    h.run_steps(12);
}

fn click_at(h: &mut App<'_>, pos: Pos2) {
    h.hover_at(pos);
    h.run_steps(6);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.run_steps(2);
    }
    h.run_steps(3);
}

fn button(h: &mut App<'_>, name: &str) {
    h.get_all_by_role_and_label(egui::accesskit::Role::Button, name).next().unwrap().click();
    h.run_steps(4);
}

/// Replaces what a panel field holds, by typing into it.
fn type_into(h: &mut App<'_>, label: &str, text: &str) {
    let d = &mut h.state_mut().state.assembly.mate;
    match label {
        "Mate name" => d.name.clear(),
        "Mate distance" => d.distance.clear(),
        "Value name" => d.value_name.clear(),
        _ => d.value_expression.clear(),
    }
    h.run_steps(2);
    h.get_by_label(label).focus();
    h.run_steps(2);
    h.get_by_label(label).type_text(text);
    h.run_steps(3);
}

fn view(h: &mut App<'_>, view: StandardView) {
    let s = h.state_mut().state.assembly.session.as_mut().unwrap();
    s.viewport.camera.set_view(view);
    h.state_mut().state.push(AppAction::Assembly(AssemblyAction::ZoomFit));
    h.run_steps(60);
}

/// Where a point of the assembly lands on screen.
fn screen(h: &App<'_>, world: Vec3) -> Pos2 {
    let s = session(h);
    s.viewport.camera.project(world, s.viewport.rect).unwrap()
}

/// The world z of the scene instance drawn for B, as the renderer gets it.
fn drawn_z(h: &App<'_>) -> f32 {
    drawn(h, 1)[2]
}

/// The world translation of the scene instance drawn for the `i`th component.
fn drawn(h: &App<'_>, i: usize) -> [f32; 3] {
    let s = session(h);
    let row = s.components()[i];
    let scene = s.scene.lock().unwrap();
    let slot = scene.slot(row).expect("the component is drawn") as usize;
    let (m, o) = (scene.instances()[slot].rows, scene.origin());
    [m[0][3] + o.x as f32, m[1][3] + o.y as f32, m[2][3] + o.z as f32]
}

/// Both components upright: the instance the renderer gets, and the occurrence table's
/// pose, keep the world basis.
fn upright(h: &App<'_>) {
    let s = session(h);
    let scene = s.scene.lock().unwrap();
    for row in s.components() {
        let m = scene.instances()[scene.slot(row).expect("drawn") as usize].rows;
        for (i, r) in m.iter().enumerate() {
            for (j, v) in r[..3].iter().enumerate() {
                assert!((v - f32::from(u8::from(i == j))).abs() <= 1e-6, "instance basis {m:?}");
            }
        }
        let q = s.doc.table().world(row).rotation;
        for axis in [glam::DVec3::X, glam::DVec3::Y, glam::DVec3::Z] {
            assert!((q * axis).distance(axis) <= 1e-12, "{q} on {axis}");
        }
    }
}

/// The revision the scene draws for the `i`th component, and the top of its box.
fn shown(h: &App<'_>, i: usize) -> (String, f64) {
    let s = session(h);
    let row = s.components()[i];
    let definition = s.doc.table().definition(row);
    let scene = s.scene.lock().unwrap();
    let slot = scene.slot(row).expect("drawn");
    let draw =
        scene.draws(|_| true).into_iter().find(|d| (d.first_instance..d.first_instance + d.count).contains(&slot));
    assert_eq!(draw.map(|d| d.definition), Some(definition), "the scene draws the table's definition");
    (s.doc.store().get(definition).key.revision.to_string(), s.doc.table().bounds(row).max.z)
}

fn snapshot(h: &mut App<'_>, name: &str) {
    let dir = std::env::var_os("SOLIDDRAFT_MATE_PROOF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/mate-test"));
    std::fs::create_dir_all(&dir).unwrap();
    h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
fn pick_drive_update_and_refuse_in_the_window() {
    let instance = egui_wgpu::wgpu::Instance::default();
    let adapters = pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all()));
    if adapters.is_empty() {
        assert!(std::env::var_os("SOLIDDRAFT_REQUIRE_GPU").is_none(), "required GPU unavailable");
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("soliddraft-mate-panel-{}", std::process::id()));
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

    // The two parts, placed apart so each cap can be clicked: B floats at x = 60.
    let (a, b) = (dir.join("a.dwprt"), dir.join("b.dwprt"));
    {
        let st = &mut h.state_mut().state;
        part(st, &a, 40.0, 30.0, 10.0);
        tool(st, "make_assembly", json!({}));
        tool(st, "assembly_save", json!({ "path": dir.join("rig.dwasm") }));
        tool(st, "new_part", json!({}));
        part(st, &b, 20.0, 10.0, 6.0);
        tool(st, "show_assembly", json!({}));
        let placement = json!({ "translation": [60, 0, 0], "rotation": [0, 0, 0, 1] });
        tool(st, "insert_component", json!({ "paths": [b], "placement": placement }));
        st.push(AppAction::Assembly(AssemblyAction::Show));
    }
    settle(&mut h);

    // Open the Mate panel from the ribbon and pick A's top and B's underside.
    button(&mut h, "Mate");
    assert!(h.state().state.assembly.mate.open);
    view(&mut h, StandardView::Top);
    settle(&mut h);
    let at = screen(&h, Vec3::new(-10.0, 0.0, 10.0));
    click_at(&mut h, at);
    settle(&mut h);
    let first = h.state().state.assembly.mate.a.clone().expect("the first pick names a face");
    assert_eq!(first.component.0, 2);
    assert_eq!(first.face.label, "Boss-Extrude1 · End cap");

    view(&mut h, StandardView::Bottom);
    settle(&mut h);
    let at = screen(&h, Vec3::new(60.0, 0.0, 0.0));
    click_at(&mut h, at);
    settle(&mut h);
    let second = h.state().state.assembly.mate.b.clone().expect("the second pick names a face");
    assert_eq!(second.component.0, 4);
    assert_eq!(second.face.label, "Boss-Extrude1 · Start cap");

    // The distance mate through the panel, with the supporting mates through commands.
    view(&mut h, StandardView::Isometric);
    type_into(&mut h, "Mate name", "Z");
    type_into(&mut h, "Mate distance", "5 mm");
    button(&mut h, "Add mate");
    settle(&mut h);
    assert!(h.state().state.assembly.mate.error.is_none(), "{:?}", h.state().state.assembly.mate.error);
    {
        let st = &mut h.state_mut().state;
        tool(st, "assembly_mate_origin", json!({ "name": "O", "component": 2, "align_axes": true }));
        for (name, n) in [("X", [1, 0, 0]), ("Y", [0, 1, 0])] {
            let face = |c: u64| json!({ "component": c, "feature": "Boss-Extrude1", "generator": "side", "normal": n });
            let arguments =
                json!({ "name": name, "a": face(2), "b": face(4), "distance": "0 mm", "alignment": "aligned" });
            tool(st, "assembly_mate_distance", arguments);
        }
    }
    settle(&mut h);
    h.get_by_value("Z: 5 mm");
    h.get_by_value("Fully constrained");
    assert_eq!(drawn_z(&h), 15.0, "B's underside sits 5 mm above A's top");
    snapshot(&mut h, "1-mated");

    // A conflicting mate is refused and the scene stays.
    type_into(&mut h, "Mate name", "Conflict");
    type_into(&mut h, "Mate distance", "6 mm");
    let revision = session(&h).doc.revision();
    button(&mut h, "Add mate");
    settle(&mut h);
    assert!(h.state().state.assembly.mate.error.as_ref().unwrap().contains("over_constrained"));
    assert_eq!(session(&h).doc.revision(), revision);
    assert_eq!(drawn_z(&h), 15.0);
    snapshot(&mut h, "2-refused");

    // A typed value drives it.
    type_into(&mut h, "Value name", "gap");
    type_into(&mut h, "Value expression", "5 mm");
    button(&mut h, "Set value");
    settle(&mut h);
    button(&mut h, "Drive Z");
    settle(&mut h);
    type_into(&mut h, "Value expression", "1.2 cm");
    button(&mut h, "Set value");
    settle(&mut h);
    h.get_by_value("Z: 12 mm");
    assert_eq!(drawn_z(&h), 22.0);
    snapshot(&mut h, "3-driven");

    // A taller A keeps the accepted pose and says so, until Update.
    {
        let st = &mut h.state_mut().state;
        tool(st, "open", json!({ "path": a }));
        tool(st, "edit_extrude", json!({ "feature": "Boss-Extrude1", "depth": 14, "flip": false }));
        tool(st, "save", json!({ "path": a }));
        tool(st, "show_assembly", json!({}));
        st.push(AppAction::Assembly(AssemblyAction::Show));
    }
    settle(&mut h);
    h.get_by_value("The part has changed; Update to use it");
    assert_eq!(drawn_z(&h), 22.0);
    snapshot(&mut h, "4-stale");
    button(&mut h, "Update");
    settle(&mut h);
    assert_eq!(drawn_z(&h), 26.0);
    h.get_by_value("Z: 12 mm");
    snapshot(&mut h, "5-updated");
    drop(h);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pick_coincident_update_and_refuse_in_the_window() {
    let instance = egui_wgpu::wgpu::Instance::default();
    let adapters = pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all()));
    if adapters.is_empty() {
        assert!(std::env::var_os("SOLIDDRAFT_REQUIRE_GPU").is_none(), "required GPU unavailable");
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("soliddraft-mate-coincident-{}", std::process::id()));
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

    // A, and B over it at [3, 7, 30]; O, Cx and Cy through the commands bring B to
    // [10, 10, 30].
    let (a, b, rig) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("rig.dwasm"));
    {
        let st = &mut h.state_mut().state;
        part(st, &a, 40.0, 30.0, 10.0);
        tool(st, "make_assembly", json!({}));
        tool(st, "assembly_save", json!({ "path": rig }));
        tool(st, "new_part", json!({}));
        part(st, &b, 20.0, 10.0, 6.0);
        tool(st, "show_assembly", json!({}));
        let placement = json!({ "translation": [3, 7, 30], "rotation": [0, 0, 0, 1] });
        tool(st, "insert_component", json!({ "paths": [b], "placement": placement }));
        tool(st, "assembly_mate_origin", json!({ "name": "O", "component": 2, "align_axes": true }));
        for (name, n) in [("Cx", [1, 0, 0]), ("Cy", [0, 1, 0])] {
            let face = |c: u64| json!({ "component": c, "feature": "Boss-Extrude1", "generator": "side", "normal": n });
            tool(
                st,
                "assembly_mate_coincident",
                json!({ "name": name, "a": face(2), "b": face(4), "alignment": "aligned" }),
            );
        }
        st.push(AppAction::Assembly(AssemblyAction::Show));
    }
    settle(&mut h);
    assert_eq!(drawn(&h, 1), [10.0, 10.0, 30.0]);

    // Pick A's top from above and B's underside from below and to the side, past A.
    button(&mut h, "Mate");
    view(&mut h, StandardView::Top);
    settle(&mut h);
    let at = screen(&h, Vec3::new(-10.0, 0.0, 10.0));
    click_at(&mut h, at);
    settle(&mut h);
    let first = h.state().state.assembly.mate.a.clone().expect("the first pick names a face");
    assert_eq!((first.component.0, first.face.label.as_str()), (2, "Boss-Extrude1 · End cap"));
    h.state_mut().state.assembly.session.as_mut().unwrap().viewport.camera.turn_to("Below", 0.0, -45.0);
    h.state_mut().state.push(AppAction::Assembly(AssemblyAction::ZoomFit));
    h.run_steps(60);
    settle(&mut h);
    let at = screen(&h, Vec3::new(18.0, 10.0, 30.0));
    click_at(&mut h, at);
    settle(&mut h);
    let second = h.state().state.assembly.mate.b.clone().expect("the second pick names a face");
    assert_eq!((second.component.0, second.face.label.as_str()), (4, "Boss-Extrude1 · Start cap"));

    // Coincident, opposed, named Cz: it has no distance to type.
    view(&mut h, StandardView::Isometric);
    settle(&mut h);
    button(&mut h, "Coincident");
    button(&mut h, "Opposed");
    assert!(h.query_by_label("Mate distance").is_none(), "a Coincident has no distance");
    type_into(&mut h, "Mate name", "Cz");
    button(&mut h, "Add mate");
    settle(&mut h);
    assert!(h.state().state.assembly.mate.error.is_none(), "{:?}", h.state().state.assembly.mate.error);
    h.get_by_value("Cz: coincident, opposed, 0 mm apart");
    h.get_by_value("Fully constrained");
    assert_eq!(drawn(&h, 1), [10.0, 10.0, 10.0], "B's underside on A's top");
    upright(&h);
    let summary = AppTools { state: &mut h.state_mut().state }.summary()["assembly"].clone();
    let cz = &summary["mates"][3];
    assert_eq!((cz["kind"].as_str(), cz["measurement"]["rational"].as_str()), (Some("coincident"), Some("0")));
    assert!(cz.get("distance").is_none(), "{cz}");
    type_into(&mut h, "Value name", "gap");
    assert!(h.query_by_label("Drive Cz").is_none(), "a Coincident has nothing to drive");
    snapshot(&mut h, "coincident-1-mated");

    // Saved and opened again; A rises to 14 mm in its own window. The assembly keeps the
    // accepted A and pose until Update.
    {
        let st = &mut h.state_mut().state;
        tool(st, "assembly_save", json!({ "path": rig }));
        tool(st, "open", json!({ "path": rig }));
        tool(st, "open", json!({ "path": a }));
        tool(st, "edit_extrude", json!({ "feature": "Boss-Extrude1", "depth": 14, "flip": false }));
        tool(st, "save", json!({ "path": a }));
        tool(st, "show_assembly", json!({}));
        st.push(AppAction::Assembly(AssemblyAction::Show));
    }
    settle(&mut h);
    h.get_by_value("The part has changed; Update to use it");
    let a10 = shown(&h, 0);
    assert_eq!(a10.1, 10.0);
    assert_eq!(drawn(&h, 1), [10.0, 10.0, 10.0]);
    snapshot(&mut h, "coincident-2-stale");
    button(&mut h, "Update");
    settle(&mut h);
    let a14 = shown(&h, 0);
    assert!(a14.0 != a10.0 && a14.1 == 14.0, "{a14:?}");
    assert_eq!(drawn(&h, 1), [10.0, 10.0, 14.0]);
    h.get_by_value("Cz: coincident, opposed, 0 mm apart");
    upright(&h);
    snapshot(&mut h, "coincident-3-updated");

    // The caps aligned while Cx and Cy hold B upright: refused, and nothing moves.
    let accepted =
        |h: &mut App<'_>| AppTools { state: &mut h.state_mut().state }.summary()["assembly"]["accepted_state"].clone();
    let before = accepted(&mut h);
    button(&mut h, "Aligned");
    type_into(&mut h, "Mate name", "Flip");
    button(&mut h, "Add mate");
    settle(&mut h);
    let error = h.state().state.assembly.mate.error.clone().unwrap_or_default();
    assert!(error.contains("over_constrained"), "{error}");
    assert_eq!(accepted(&mut h), before);
    assert_eq!(drawn(&h, 1), [10.0, 10.0, 14.0]);
    assert_eq!(shown(&h, 0), a14);
    snapshot(&mut h, "coincident-4-refused");

    // Undo and Redo of Update take the accepted A and B's pose together.
    tool(&mut h.state_mut().state, "undo", json!({}));
    settle(&mut h);
    assert_eq!((shown(&h, 0), drawn(&h, 1)), (a10.clone(), [10.0, 10.0, 10.0]));
    tool(&mut h.state_mut().state, "redo", json!({}));
    settle(&mut h);
    assert_eq!((shown(&h, 0), drawn(&h, 1)), (a14, [10.0, 10.0, 14.0]));
    snapshot(&mut h, "coincident-5-redone");
    drop(h);
    let _ = std::fs::remove_dir_all(dir);
}
