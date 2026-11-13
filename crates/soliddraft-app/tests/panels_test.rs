//! Optional panes keep their width and per-document visibility across runs.
//! The session overlays history without resizing the viewport.

use egui::{Key, Modifiers, Rect, pos2, vec2};
use egui_kittest::Harness;
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::document::Selection;
use soliddraft_app::drawing::DrawingAction;
use soliddraft_app::panels::{Layout, Side};
use soliddraft_app::state::AppAction;
use soliddraft_app::theme;
use std::path::PathBuf;

fn gpu_available() -> bool {
    let _ = env_logger::try_init();
    let instance = egui_wgpu::wgpu::Instance::default();
    !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty()
}

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/panels-test");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[path = "support/render_steps.rs"]
mod render_steps;

type App<'a> = Harness<'a, SolidDraftApp>;

fn harness() -> App<'static> {
    Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .renderer(render_steps::renderer())
        .build_eframe(|cc| SolidDraftApp::new(cc, soliddraft_app::demo_document(), Automation::default()))
}

fn save(harness: &mut App<'_>, name: &str) {
    let image = harness.render().expect("render");
    let path = out_dir().join(format!("{name}.png"));
    image.save(&path).expect("save png");
    eprintln!("wrote {}", path.display());
}

/// Lets the slide animation and the one-frame layout lag settle.
fn settle(harness: &mut App<'_>) {
    harness.run_steps(30);
}

fn layout(harness: &App<'_>) -> Layout {
    harness.state().state.layout
}

fn viewport_rect(harness: &App<'_>) -> Rect {
    harness.state().state.viewport.rect
}

/// Drags the inner edge of a shown side area by `dx` logical pixels.
fn drag_edge(harness: &mut App<'_>, side: Side, dx: f32) {
    let edge_x = match side {
        Side::Tree => viewport_rect(harness).left() - 1.0,
    };
    // Well above the tree/properties split, so the panel edge is the only handle there.
    let y = viewport_rect(harness).top() + 80.0;
    render_steps::rendered(harness, |harness| {
        harness.hover_at(pos2(edge_x, y));
        harness.run_steps(2);
        harness.drag_at(pos2(edge_x, y));
        harness.run_steps(2);
        for step in 1..=4 {
            harness.hover_at(pos2(edge_x + dx * step as f32 / 4.0, y));
            harness.run_steps(2);
        }
        harness.drop_at(pos2(edge_x + dx, y));
        harness.step();
    });
    harness.run_steps(29);
}

/// The GPU harnesses share one adapter; one test at a time.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|e| e.into_inner())
}

/// A config directory of this test's own, so the settings file starts empty and
/// the developer's real one is left alone.
fn config_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-panels-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.clone()));
    dir
}

#[test]
fn side_areas_toggle_resize_and_come_back_next_run() {
    if !gpu_available() {
        return;
    }
    let _gpu = gpu_lock();
    let dir = config_dir("part");
    let mut harness = harness();
    settle(&mut harness);
    assert_eq!(layout(&harness), Layout::default());
    assert!(!harness.state().state.tree_shown());
    let wide = viewport_rect(&harness);
    assert!((wide.left() - theme::edit_suite::PALETTE_WIDTH).abs() < 2.0);
    assert!(wide.right() >= 1439.0);
    let history = egui::PanelState::load(&harness.ctx, egui::Id::new("feature_timeline")).unwrap().outer_rect;
    let command = egui::PanelState::load(&harness.ctx, soliddraft_app::command_line::bar_id()).unwrap().outer_rect;
    assert!(wide.bottom() <= history.top());
    assert!(history.bottom() <= command.top() + 1.0);
    save(&mut harness, "p01-default");

    harness.key_press(Key::F9);
    settle(&mut harness);
    assert!(harness.state().state.tree_shown());
    let full = viewport_rect(&harness);
    assert!((full.left() - wide.left() - Side::Tree.default_width()).abs() < 2.0);
    let plate = harness.state().state.doc.features[0].id;
    harness.state_mut().state.doc.selection = Some(Selection::Feature(plate));
    harness.key_press(Key::F9);
    settle(&mut harness);
    assert!(!layout(&harness).suite.part);
    assert_eq!(viewport_rect(&harness), wide);
    save(&mut harness, "p02-tree-hidden");
    harness.key_press(Key::F9);
    settle(&mut harness);
    assert!(layout(&harness).suite.part);
    assert_eq!(viewport_rect(&harness), full);
    assert_eq!(harness.state().state.doc.selection, Some(Selection::Feature(plate)));

    harness.key_press_modifiers(Modifiers::SHIFT, Key::F9);
    settle(&mut harness);
    assert!(harness.state().state.command_line.session_open);
    assert_eq!(viewport_rect(&harness), full, "the session overlays history");
    save(&mut harness, "p03-session-open");
    harness.key_press_modifiers(Modifiers::SHIFT, Key::F9);
    settle(&mut harness);
    assert!(!harness.state().state.command_line.session_open);

    let tree_before = layout(&harness).tree.width;
    drag_edge(&mut harness, Side::Tree, 120.0);
    let tree_after = layout(&harness).tree.width;
    assert!((tree_after - (tree_before + 120.0)).abs() < 3.0);
    assert!((viewport_rect(&harness).left() - wide.left() - tree_after).abs() < 2.0);
    drag_edge(&mut harness, Side::Tree, 2000.0);
    let cap = layout(&harness).max_width(Side::Tree, 1440.0);
    assert!((layout(&harness).tree.width - cap).abs() < 3.0);
    assert!(viewport_rect(&harness).width() >= soliddraft_app::panels::VIEWPORT_MIN_WIDTH - 2.0);
    let back = tree_after - layout(&harness).tree.width;
    drag_edge(&mut harness, Side::Tree, back);
    assert!((layout(&harness).tree.width - tree_after).abs() < 3.0);
    save(&mut harness, "p04-resized");

    harness.key_press(Key::F9);
    settle(&mut harness);
    let chosen = layout(&harness);
    assert!(!chosen.suite.part);
    assert_eq!(soliddraft_app::settings::layout(), chosen);
    drop(harness);
    let mut again = self::harness();
    settle(&mut again);
    assert_eq!(layout(&again), chosen);
    assert_eq!(viewport_rect(&again), wide);
    save(&mut again, "p05-next-run");
    again.state_mut().state.push(AppAction::ResetLayout);
    settle(&mut again);
    assert_eq!(layout(&again), Layout::default());
    assert_eq!(viewport_rect(&again), wide);
    assert_eq!(soliddraft_app::settings::layout(), Layout::default());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_drawing_workspace_toggles_the_same_areas() {
    if !gpu_available() {
        eprintln!("skipped: no GPU adapter");
        return;
    }
    let _gpu = gpu_lock();
    let dir = config_dir("drawing");
    let mut harness = harness();
    settle(&mut harness);
    harness.state_mut().state.push(AppAction::Drawing(DrawingAction::MakeDrawing));
    settle(&mut harness);
    harness.state_mut().state.push(AppAction::Drawing(DrawingAction::CreateDrawing));
    settle(&mut harness);
    assert!(harness.state().state.drawing.is_active(), "the drawing tab is open and active");

    harness.key_press(Key::F9);
    settle(&mut harness);
    assert!(!layout(&harness).suite.drawing);
    assert!(harness.state().state.drawing.is_active(), "hiding the tree keeps the drawing active");
    save(&mut harness, "p06-drawing-tree-hidden");
    harness.key_press(Key::F9);
    settle(&mut harness);
    assert!(layout(&harness).suite.drawing);
    assert!(harness.state().state.drawing.is_active());

    harness.key_press_modifiers(Modifiers::SHIFT, Key::F9);
    settle(&mut harness);
    assert!(harness.state().state.command_line.session_open, "Shift+F9 opens the session in the drawing too");
    assert!(harness.state().state.drawing.is_active());
    harness.key_press_modifiers(Modifiers::SHIFT, Key::F9);
    settle(&mut harness);
    assert!(!harness.state().state.command_line.session_open);

    // Back on the part, the drawing tab is still there.
    harness.state_mut().state.push(AppAction::Drawing(DrawingAction::ShowPart));
    settle(&mut harness);
    assert!(!harness.state().state.drawing.is_active());
    assert!(harness.state().state.drawing.session.is_some());
    let _ = std::fs::remove_dir_all(dir);
}
