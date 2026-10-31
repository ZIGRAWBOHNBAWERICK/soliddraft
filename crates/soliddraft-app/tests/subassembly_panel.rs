//! G1: Form subassembly in the window. The parts are chosen in the tree, the context menu
//! opens the panel, the new file is named and applied; then a nested face is picked, the
//! two-file Update runs, the root's value drives the nested parts in the scene, and the
//! free nested part moves through the placement fields. Every pose is the V6 batch 1
//! design's hand-derived one.

use egui::{Modifiers, Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use glam::Vec3;
use serde_json::json;
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::assembly::{AssemblyAction, AssemblySession};
use soliddraft_app::state::{AppAction, AppState};
use soliddraft_app::viewport::camera::StandardView;
use soliddraft_assembly::{Mate, OccurrencePath};
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use soliddraft_model::FaceRef;
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
        if !session(h).doc.is_loading() {
            break;
        }
        assert!(Instant::now() < until, "the assembly did not settle");
    }
    h.run_steps(12);
}

fn show(h: &mut App<'_>) {
    h.state_mut().state.push(AppAction::Assembly(AssemblyAction::Show));
    settle(h);
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

/// The tree row of the occurrence called `name`.
fn tree_row<'n>(h: &'n App<'_>, name: &'n str) -> egui_kittest::Node<'n> {
    h.get_all_by_role_and_label(egui::accesskit::Role::Button, name).next().unwrap_or_else(|| panic!("no row {name}"))
}

/// Replaces what a field holds, by typing into it.
fn type_into(h: &mut App<'_>, label: &str, text: &str, clear: impl FnOnce(&mut AppState)) {
    clear(&mut h.state_mut().state);
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
    settle(h);
}

fn screen(h: &App<'_>, world: Vec3) -> Pos2 {
    let s = session(h);
    s.viewport.camera.project(world, s.viewport.rect).unwrap()
}

fn path(text: &str) -> OccurrencePath {
    text.parse().unwrap()
}

/// The world translation of the scene instance drawn for the occurrence at `at`.
fn drawn(h: &App<'_>, at: &str) -> [f32; 3] {
    let s = session(h);
    let row = s.doc.table().find(&path(at)).unwrap_or_else(|| panic!("no occurrence {at}"));
    let scene = s.scene.lock().unwrap();
    let slot = scene.slot(row).unwrap_or_else(|| panic!("{at} is not drawn")) as usize;
    let o = scene.origin();
    let m = scene.instances()[slot].rows;
    [m[0][3] + o.x as f32, m[1][3] + o.y as f32, m[2][3] + o.z as f32]
}

fn world(h: &App<'_>, at: &str) -> [f64; 3] {
    let s = session(h);
    let row = s.doc.table().find(&path(at)).unwrap();
    s.doc.table().world(row).translation.to_array()
}

fn close(a: [f32; 3], b: [f32; 3]) {
    assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-5), "{a:?} against {b:?}");
}

fn sha(path: &Path) -> String {
    soliddraft_document::ContentHash::of_bytes(&std::fs::read(path).unwrap()).to_string()
}

fn snapshot(h: &mut App<'_>, name: &str) {
    let dir = std::env::var_os("SOLIDDRAFT_SUBASSEMBLY_PROOF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/subassembly-test"));
    std::fs::create_dir_all(&dir).unwrap();
    h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
}

/// B's underside, picked with A hidden for the moment so nothing stands in front of it: the
/// pick names the nested occurrence and the part's own face, whatever the click selects.
fn pick_nested_bottom(h: &mut App<'_>) -> soliddraft_assembly::Selection {
    let a = path("2");
    h.state_mut().state.assembly.session.as_mut().unwrap().doc.set_hidden(&[a], true).unwrap();
    view(h, StandardView::Bottom);
    let at = screen(h, Vec3::new(5.0, 7.0, 22.0));
    click_at(h, at);
    settle(h);
    let picked = session(h).picked.clone().expect("the click names a face");
    let s = h.state_mut().state.assembly.session.as_mut().unwrap();
    assert!(s.doc.undo(), "showing A again is the hide's own undo");
    settle(h);
    picked
}

/// The FaceRef the root's Zab mate holds for B's underside.
fn zab_b(h: &App<'_>) -> (OccurrencePath, FaceRef) {
    let s = session(h);
    let zab = s.doc.assembly().known_mates().find(|m| m.name() == "Zab").unwrap();
    let Mate::Distance { b, .. } = zab else { panic!("Zab is a distance") };
    (b.path.clone(), b.face.clone())
}

fn check_pick(h: &App<'_>, picked: &soliddraft_assembly::Selection, pin: soliddraft_document::DocumentId) {
    assert_eq!(picked.path, path("16/4"), "the nested B, never C or F");
    let face: FaceRef = serde_json::from_str(picked.face.as_deref().expect("a face")).unwrap();
    let (mate_path, mate_face) = zab_b(h);
    assert_eq!(mate_path, picked.path);
    assert_eq!(
        (face.feature, face.generator, face.ordinal),
        (mate_face.feature, mate_face.generator, mate_face.ordinal)
    );
    let s = session(h);
    let row = s.doc.table().find(&picked.path).unwrap();
    assert_eq!(s.doc.store().get(s.doc.table().definition(row)).key.document, pin, "the pin's document");
}

#[test]
fn form_pick_and_move_the_floating_child_in_the_window() {
    let instance = egui_wgpu::wgpu::Instance::default();
    let adapters = pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all()));
    if adapters.is_empty() {
        assert!(std::env::var_os("SOLIDDRAFT_REQUIRE_GPU").is_none(), "required GPU unavailable");
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("soliddraft-subassembly-panel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("work/parts")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut h: App<'static> = Harness::builder()
        .with_size(vec2(1600.0, 1000.0))
        .with_pixels_per_point(1.0)
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| SolidDraftApp::new(cc, soliddraft_app::new_document(), Automation::default()));
    h.run_steps(3);

    // The flat root T0 of the design: A on the origin, B on A, C on B, F free.
    let work = dir.join("work");
    let (base, pin, root, pack) = (
        work.join("parts/base.dwprt"),
        work.join("parts/pin.dwprt"),
        work.join("root.dwasm"),
        work.join("modules/pack.dwasm"),
    );
    {
        let st = &mut h.state_mut().state;
        part(st, &base, 40.0, 30.0, 10.0);
        tool(st, "make_assembly", json!({}));
        tool(st, "assembly_save", json!({ "path": root }));
        tool(st, "new_part", json!({}));
        part(st, &pin, 20.0, 10.0, 6.0);
        tool(st, "show_assembly", json!({}));
        for (t, q) in [([10, 10, 22], [0, 0, 0, 1]), ([10, 10, 31], [0, 0, 0, 1]), ([60, -20, 7], [0, 0, 1, 0])] {
            tool(st, "insert_component", json!({ "paths": [pin], "placement": { "translation": t, "rotation": q } }));
        }
        tool(st, "assembly_mate_origin", json!({ "name": "O", "component": 2, "align_axes": true }));
        let side = |c: u64, n: [i32; 3]| json!({ "component": c, "feature": "Boss-Extrude1", "generator": "side", "normal": n });
        let cap = |c: u64, g: &str| json!({ "component": c, "feature": "Boss-Extrude1", "generator": g });
        for (name, a, b, d, alignment) in [
            ("Xab", side(2, [1, 0, 0]), side(4, [1, 0, 0]), "0 mm", "aligned"),
            ("Yab", side(2, [0, 1, 0]), side(4, [0, 1, 0]), "0 mm", "aligned"),
            ("Zab", cap(2, "end_cap"), cap(4, "start_cap"), "12 mm", "opposed"),
            ("Xbc", side(4, [1, 0, 0]), side(5, [1, 0, 0]), "0 mm", "aligned"),
            ("Ybc", side(4, [0, 1, 0]), side(5, [0, 1, 0]), "0 mm", "aligned"),
            ("Zbc", cap(4, "end_cap"), cap(5, "start_cap"), "3 mm", "opposed"),
        ] {
            tool(
                st,
                "assembly_mate_distance",
                json!({ "name": name, "a": a, "b": b, "distance": d, "alignment": alignment }),
            );
        }
        tool(st, "assembly_set_value", json!({ "name": "gap", "expression": "12 mm" }));
        tool(st, "assembly_drive_mate", json!({ "mate": "Zab", "parameter": "distance", "expression": "gap" }));
        tool(st, "assembly_save", json!({ "path": root }));
    }
    show(&mut h);
    let pin_id = soliddraft_document::identify_file(&pin).unwrap().id;
    let root_id = session(&h).doc.id();

    // Choose B, C and F in the tree, in that order, and form them from the context menu.
    tree_row(&h, "pin<4>").click();
    h.run_steps(4);
    for name in ["pin<5>", "pin<6>"] {
        tree_row(&h, name).click_modifiers(Modifiers::COMMAND);
        h.run_steps(4);
    }
    assert_eq!(session(&h).chosen, [path("4"), path("5"), path("6")]);
    tree_row(&h, "pin<4>").click_secondary();
    h.run_steps(4);
    button(&mut h, "Form subassembly…");
    assert!(h.state().state.assembly.form.open);
    type_into(&mut h, "Subassembly file", "modules/pack.dwasm", |st| st.assembly.form.file.clear());
    button(&mut h, "Apply");
    settle(&mut h);
    assert!(h.state().state.assembly.form.error.is_none(), "{:?}", h.state().state.assembly.form.error);
    assert!(!h.state().state.assembly.form.open);
    assert!(h.state().state.assembly.is_active(), "the root stays active");
    assert!(pack.is_file());
    assert_eq!(session(&h).doc.undo_label(), Some("Form subassembly"));
    assert!(session(&h).expanded.contains(&path("16")), "the new sub-assembly opens in the tree");
    for name in ["pack<16>", "pin<4>", "pin<5>", "pin<6>"] {
        tree_row(&h, name);
    }
    view(&mut h, StandardView::Isometric);
    close(drawn(&h, "16/4"), [10.0, 10.0, 22.0]);
    close(drawn(&h, "16/5"), [10.0, 10.0, 31.0]);
    close(drawn(&h, "16/6"), [60.0, -20.0, 7.0]);
    snapshot(&mut h, "1-formed");

    // A nested face is picked by its own path and the part's persistent name.
    let picked = pick_nested_bottom(&mut h);
    check_pick(&h, &picked, pin_id);
    assert_eq!(session(&h).doc.id(), root_id);
    {
        let st = &mut h.state_mut().state;
        tool(st, "assembly_save", json!({ "path": root }));
    }

    // The two-file Update: the pin, then the new file, then the root, each by its Update.
    {
        let st = &mut h.state_mut().state;
        tool(st, "open", json!({ "path": pin }));
        tool(st, "edit_extrude", json!({ "feature": "Boss-Extrude1", "depth": 8, "flip": false }));
        tool(st, "save", json!({ "path": pin }));
        tool(st, "open", json!({ "path": pack }));
    }
    show(&mut h);
    button(&mut h, "Update");
    settle(&mut h);
    close(drawn(&h, "5"), [0.0, 0.0, 11.0]);
    h.state_mut().state.push(AppAction::Assembly(AssemblyAction::Save(None)));
    settle(&mut h);
    tool(&mut h.state_mut().state, "open", json!({ "path": root }));
    show(&mut h);
    button(&mut h, "Update");
    settle(&mut h);
    close(drawn(&h, "16/5"), [10.0, 10.0, 33.0]);
    snapshot(&mut h, "2-updated");
    let (child_bytes, pin_bytes) = (sha(&pack), sha(&pin));

    // The root's value drives the nested parts through the Mate panel; F stays.
    button(&mut h, "Mate");
    type_into(&mut h, "Value name", "gap", |st| st.assembly.mate.value_name.clear());
    type_into(&mut h, "Value expression", "8 mm", |st| st.assembly.mate.value_expression.clear());
    button(&mut h, "Set value");
    settle(&mut h);
    close(drawn(&h, "16/4"), [10.0, 10.0, 18.0]);
    close(drawn(&h, "16/5"), [10.0, 10.0, 29.0]);
    close(drawn(&h, "16/6"), [60.0, -20.0, 7.0]);
    snapshot(&mut h, "3-gap-8");
    type_into(&mut h, "Value expression", "12 mm", |st| st.assembly.mate.value_expression.clear());
    button(&mut h, "Set value");
    settle(&mut h);
    close(drawn(&h, "16/4"), [10.0, 10.0, 22.0]);
    close(drawn(&h, "16/5"), [10.0, 10.0, 33.0]);
    button(&mut h, "Mate");

    // The free nested F moves 2 mm along x through its placement fields; nothing else moves.
    let mates = session(&h).doc.assembly().mates.clone();
    let chevron = tree_row(&h, "pack<16>").rect();
    click_at(&mut h, egui::pos2(chevron.left() + 30.0, chevron.center().y));
    assert!(session(&h).expanded.contains(&path("16")), "the reopened root's sub-assembly opens in the tree");
    tree_row(&h, "pin<6>").click();
    h.run_steps(6);
    assert_eq!(session(&h).selected, Some(path("16/6")));
    // As a user types a number: click into the field, select what it holds, type, Enter.
    h.get_by_label("Placement X").click();
    h.run_steps(3);
    h.key_press_modifiers(Modifiers::COMMAND, egui::Key::A);
    h.run_steps(2);
    h.get_by_label("Placement X").type_text("52");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(6);
    settle(&mut h);
    assert_eq!(world(&h, "16/6"), [62.0, -20.0, 7.0]);
    let s = session(&h);
    let row = s.doc.table().find(&path("16/6")).unwrap();
    let bounds = s.doc.table().bounds(row);
    assert_eq!((bounds.min.to_array(), bounds.max.to_array()), ([52.0, -25.0, 7.0], [72.0, -15.0, 15.0]));
    let turn = session(&h).doc.table().world(row).rotation;
    for (axis, wanted) in
        [(glam::DVec3::X, -glam::DVec3::X), (glam::DVec3::Y, -glam::DVec3::Y), (glam::DVec3::Z, glam::DVec3::Z)]
    {
        assert!((turn * axis).distance(wanted) <= 1e-12, "F keeps its half turn: {turn}");
    }
    assert_eq!((world(&h, "16/4"), world(&h, "16/5")), ([10.0, 10.0, 22.0], [10.0, 10.0, 33.0]));
    assert_eq!(session(&h).doc.assembly().mates, mates, "no mate is made");
    h.state_mut().state.assembly.session.as_mut().unwrap().doc.solver();
    assert_eq!(h.state_mut().state.assembly.session.as_mut().unwrap().doc.solver().remaining_dof, 6);
    close(drawn(&h, "16/6"), [62.0, -20.0, 7.0]);
    snapshot(&mut h, "4-moved");
    h.state_mut().state.push(AppAction::Assembly(AssemblyAction::Undo));
    settle(&mut h);
    assert_eq!(world(&h, "16/6"), [60.0, -20.0, 7.0]);

    // Saved and reopened, the nested B picks the same again; neither file below was written.
    h.state_mut().state.push(AppAction::Assembly(AssemblyAction::Save(None)));
    settle(&mut h);
    tool(&mut h.state_mut().state, "open", json!({ "path": root }));
    show(&mut h);
    let again = pick_nested_bottom(&mut h);
    check_pick(&h, &again, pin_id);
    assert_eq!(again.face, picked.face, "the same persistent name");
    assert_eq!((sha(&pack), sha(&pin)), (child_bytes, pin_bytes), "the root's edits wrote no other file");
    snapshot(&mut h, "5-reopened");
    drop(h);
    let _ = std::fs::remove_dir_all(dir);
}
