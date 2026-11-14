//! V2 G1: a real viewport pick, proven readout, Track this, and an interactive edit;
//! V2 batch 2 G1: a hovered midpoint, a picked pair, its tracked row, an edit and reopen.

#[path = "support/measure_window.rs"]
mod window;

use egui::{Key, Modifiers, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use glam::Vec3;
use soliddraft_app::{
    app::{Automation, SolidDraftApp},
    document::{Command, Document, ExtrudeParams, ProfileKind, Selection},
    measure, theme,
};
use std::sync::Arc;
use window::{App, button, click_at, screen_of, settle, snapshot};

#[test]
fn pick_measure_track_and_edit() {
    let instance = egui_wgpu::wgpu::Instance::default();
    let adapters = pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all()));
    if adapters.is_empty() {
        assert!(std::env::var_os("SOLIDDRAFT_REQUIRE_GPU").is_none(), "required GPU unavailable");
        eprintln!("no GPU adapter; skipping");
        return;
    }
    let config = std::env::temp_dir().join(format!("soliddraft-measure-panel-{}", std::process::id()));
    std::fs::create_dir_all(&config).unwrap();
    soliddraft_app::file::set_config_dir(Some(config.clone()));
    let mut doc = Document::new("Measured block", Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new()));
    let id = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams {
                profile: ProfileKind::Rectangle { width: 40.0, height: 30.0 },
                depth: 10.0,
                ..Default::default()
            },
            via_assistant: false,
        })
        .unwrap()
        .unwrap();
    let mut h = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| SolidDraftApp::new(cc, doc, Automation::default()));
    settle(&mut h);
    button(&mut h, "Inspect tab");
    button(&mut h, "Measure");
    assert!(h.state().state.measure.open);
    h.key_press(Key::Z);
    settle(&mut h);
    // Pick a visible vertical corner through the actual id-buffer path.
    let world = Vec3::new(20.0, -15.0, 5.0);
    let pos = screen_of(&h, world);
    click_at(&mut h, pos);
    settle(&mut h);
    assert!(matches!(h.state().state.doc.selection, Some(Selection::Edge(_))), "{:?}", h.state().state.doc.selection);
    h.get_by_label("10 mm");
    h.get_by_label("exact");
    h.get_by_label("Measurement check").focus();
    h.run_steps(2);
    h.get_by_label("Measurement check").type_text("< 12 mm");
    h.run_steps(2);
    button(&mut h, "Track this");
    settle(&mut h);
    h.get_by_label("measure_1: 10 mm");
    h.get_by_label("Check: pass");
    snapshot(&mut h, "tracked");
    let feature = h.state().state.doc.feature(id).unwrap().name.clone();
    button(&mut h, &feature);
    settle(&mut h);
    h.get_by_label("Depth value").click();
    h.run_steps(2);
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_label("Depth value").type_text("15");
    h.run_steps(2);
    h.key_press(Key::Enter);
    settle(&mut h);
    h.get_by_label("measure_1: 15 mm");
    h.get_by_label("Check: fail");
    assert_eq!(measure::panel::row_color("fail"), theme::error());
    snapshot(&mut h, "failed-check");
    let saved = config.join("tracked.dwprt");
    h.state_mut().state.push(soliddraft_app::state::AppAction::SaveAs(Some(saved.clone())));
    settle(&mut h);
    assert!(saved.is_file());
    let recent: Vec<std::path::PathBuf> = serde_json::from_slice(
        &std::fs::read(config.join("recent.json")).expect("the active measurement case owns its recent-file list"),
    )
    .unwrap();
    assert!(recent.contains(&saved));
}

/// V2 batch 2 G1.
#[test]
fn hover_pick_pair_track_edit_and_reopen_in_window() {
    use soliddraft_app::state::AppAction;
    let mut doc = Document::new("Paired block", Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new()));
    let id = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams {
                profile: ProfileKind::Rectangle { width: 40.0, height: 30.0 },
                depth: 10.0,
                ..Default::default()
            },
            via_assistant: false,
        })
        .unwrap()
        .unwrap();
    let Some(mut h) = window::window(doc, "measure-pairs") else { return };
    window::button(&mut h, "Inspect tab");
    window::button(&mut h, "Measure");
    h.key_press(Key::Z);
    window::settle(&mut h);
    let midpoint = "Midpoint (0, -15, 0) mm";
    // Half a millimetre along E0 from its midpoint: the label is the proof, not the ray.
    let near_m0 = |h: &App<'_>| window::screen_of(h, Vec3::new(0.5, -15.0, 0.0));
    let at = near_m0(&h);
    window::hover(&mut h, at);
    assert!(window::shows(&h, midpoint), "no proven midpoint target near E0");
    window::snapshot(&mut h, "pair-hover-midpoint");
    window::click_at(&mut h, at);
    window::settle(&mut h);
    let m2 = window::screen_of(&h, Vec3::new(20.0, 0.0, 10.0));
    window::click_at(&mut h, m2);
    window::settle(&mut h);
    h.get_by_label("26.93 mm");
    window::snapshot(&mut h, "pair-midpoints");
    window::button(&mut h, "Minimum distance");
    window::settle(&mut h);
    h.get_by_label("10 mm");
    // E1, away from its midpoint, replaces the second pick.
    let e1 = window::screen_of(&h, Vec3::new(-10.0, 15.0, 10.0));
    window::click_at(&mut h, e1);
    window::settle(&mut h);
    h.get_by_label("31.62 mm");
    h.get_by_label("bounded");
    for (field, text) in [("Tracked value name", "diagonal_gap"), ("Measurement check", "< 32 mm")] {
        h.get_by_label(field).focus();
        h.run_steps(2);
        h.get_by_label(field).type_text(text);
        h.run_steps(2);
    }
    window::button(&mut h, "Track this");
    window::settle(&mut h);
    h.get_by_label("diagonal_gap: 31.62 mm");
    h.get_by_label("Check: pass");

    let edit_depth = |h: &mut App<'_>, depth: &str| {
        let feature = h.state().state.doc.feature(id).unwrap().name.clone();
        window::button(h, &feature);
        window::settle(h);
        h.get_by_label("Depth value").click();
        h.run_steps(2);
        h.key_press_modifiers(Modifiers::COMMAND, Key::A);
        h.get_by_label("Depth value").type_text(depth);
        h.run_steps(2);
        h.key_press(Key::Enter);
        window::settle(h);
    };
    edit_depth(&mut h, "15");
    h.get_by_label("diagonal_gap: 33.54 mm");
    h.get_by_label("Check: fail");
    window::snapshot(&mut h, "pair-changed-row");
    h.state_mut().state.push(AppAction::Undo);
    window::settle(&mut h);
    h.get_by_label("diagonal_gap: 31.62 mm");
    h.state_mut().state.push(AppAction::Redo);
    window::settle(&mut h);
    h.get_by_label("diagonal_gap: 33.54 mm");

    let path = std::env::temp_dir().join(format!("soliddraft-measure-pairs-{}.dwprt", std::process::id()));
    h.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    window::settle(&mut h);
    h.state_mut().state.push(AppAction::Open(Some(path)));
    window::settle(&mut h);
    h.get_by_label("diagonal_gap: 33.54 mm");
    edit_depth(&mut h, "10");
    h.get_by_label("diagonal_gap: 31.62 mm");
    h.get_by_label("Check: pass");

    // A zoom changes the screen, not the proven point.
    let centre = h.state().state.viewport.rect.center();
    window::hover(&mut h, centre);
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, 1.0),
        phase: egui::TouchPhase::Move,
        modifiers: Modifiers::NONE,
    });
    window::settle(&mut h);
    if !h.state().state.measure.open {
        window::button(&mut h, "Measure");
    }
    let at = near_m0(&h);
    window::hover(&mut h, at);
    assert!(window::shows(&h, midpoint), "the midpoint moved with the camera");
}

/// The real-use pass picked the walls of two holes. The panel printed the kernel query,
/// its status and its error code, such as `parallel_face_to_face failed (status 0):
/// measure.nonplanar-face: face #94 is not planar`. An unfinished pair says only Coming soon.
#[test]
fn an_unfinished_pair_says_only_coming_soon() {
    use egui_kittest::kittest::{NodeT, by};
    use serde_json::json;
    use soliddraft_app::{app_tools::AppTools, command_tools::find_entity, measure::hover::Pick};
    use soliddraft_assistant::{Caller, tools::call_checked};

    // Every text on screen: egui gives a label's words as its value.
    fn texts(h: &App<'_>) -> Vec<String> {
        h.query_all(by())
            .flat_map(|n| {
                let node = n.accesskit_node();
                [node.label(), node.value()]
            })
            .flatten()
            .collect()
    }

    let doc = Document::new("Two holes", Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new()));
    let Some(mut h) = window::window(doc, "measure-unfinished") else { return };
    let caller = Caller::Mcp { client: "test".into() };
    for (tool, arguments) in [
        (
            "sketch",
            json!({"plane": "top", "shapes": [{"shape": "rectangle", "center": [0, 0, 0], "width": 40, "height": 30}]}),
        ),
        ("extrude", json!({"sketch": "Sketch1", "depth": 10, "flip": false})),
        ("hole", json!({"face": "Boss-Extrude1 · End cap", "points": [[-10, 0, 10]], "diameter": 6})),
        ("hole", json!({"face": "Boss-Extrude1 · End cap", "points": [[10, 0, 10]], "diameter": 6})),
    ] {
        let out = call_checked(&mut AppTools { state: &mut h.state_mut().state }, tool, arguments, &caller);
        assert!(!out.is_error, "{tool}: {}", out.content);
        window::settle(&mut h);
    }
    window::button(&mut h, "Inspect tab");
    window::button(&mut h, "Measure");
    let coming_soon = |h: &App<'_>| texts(h).iter().filter(|t| *t == "Coming soon").count();
    let before = coming_soon(&h);
    for name in ["Hole1 · Hole 1 wall", "Hole2 · Hole 1 wall"] {
        let selection = find_entity(&h.state().state.doc, name).unwrap();
        h.state_mut().state.measure.pick(Pick { selection, midpoint: false });
    }
    window::settle(&mut h);
    for quantity in ["Minimum distance", "Direction angle"] {
        window::button(&mut h, quantity);
        window::settle(&mut h);
        window::snapshot(&mut h, &format!("unfinished-{}", quantity.to_lowercase().replace(' ', "-")));
        let leaked: Vec<_> = texts(&h)
            .into_iter()
            .filter(|t| {
                ["measure.", "(status", "FaceId", "face #", "_face"].iter().any(|internal| t.contains(internal))
            })
            .collect();
        assert!(leaked.is_empty(), "{quantity}: the panel shows {leaked:?}");
        assert_eq!(coming_soon(&h), before + 1, "{quantity}: the panel does not say Coming soon");
    }
}
