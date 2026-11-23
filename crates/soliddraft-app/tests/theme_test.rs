//! Clear Bench appearance settings through the real app and wgpu pipeline.
//! Bracket frames are written under the temporary directory for review.
//!
//! The mode is process-wide (`theme::set_mode`), so switches run light then dark
//! serially in one test.
//!
//! Skips itself when no GPU adapter is available (plain CI runners).

use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use soliddraft_app::app::{Automation, SolidDraftApp};

use soliddraft_app::state::AppAction;
use soliddraft_app::theme::{self, Mode};
use std::path::PathBuf;

fn gpu_available() -> bool {
    let _ = env_logger::try_init();
    let instance = egui_wgpu::wgpu::Instance::default();
    !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty()
}

fn out_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-theme-frames-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_bracket_in_light_and_dark() {
    if !gpu_available() {
        return;
    }
    // Choosing a theme saves it, so the test keeps its settings away from the user's.
    let config = std::env::temp_dir().join(format!("soliddraft-theme-test-{}", std::process::id()));
    std::fs::create_dir_all(&config).unwrap();
    soliddraft_app::file::set_config_dir(Some(config.clone()));
    let mut harness = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            SolidDraftApp::new(cc, soliddraft_app::command_tools::board_bracket(), Automation::default())
        });
    assert_eq!(theme::mode(), Mode::Light, "light is the default mode");
    harness.key_press(Key::I);
    harness.run_steps(30);
    harness.key_press(Key::Z);
    harness.run_steps(30);

    for mode in Mode::ALL {
        harness.state_mut().state.push(AppAction::SetTheme(mode));
        harness.run_steps(6);
        assert_eq!(theme::mode(), mode, "the appearance preference is retained");
        assert_eq!(theme::active_theme_id(), "clear_bench");
        assert_eq!(theme::palette(), &theme::clear_bench::PALETTE);
        let style = harness.ctx.global_style();
        assert!(!style.visuals.dark_mode, "Clear Bench has daylight chrome in either mode");
        assert_eq!(style.visuals.panel_fill, egui::Color32::WHITE);
        assert_eq!(style.visuals.widgets.inactive.corner_radius, egui::CornerRadius::same(6));
        let image = harness.render().expect("Clear Bench renders");
        // The tool strip's own ground shows in the gap between two of its cards.
        let start = harness.get_by_label("Start sketch").rect();
        let gap = (start.right() + 2.0) as u32;
        assert_eq!(
            image.get_pixel(gap, start.center().y as u32).0,
            [0xf5, 0xf7, 0xfa, 255],
            "the strip paints its daylight ground"
        );
        assert_eq!(
            image.get_pixel((start.left() + 6.0) as u32, start.center().y as u32).0,
            [0x1d, 0x52, 0xd6, 255],
            "Start sketch paints solid cobalt"
        );
        let path = out_dir().join(format!("clear-bench-{}.png", mode.label().to_lowercase()));
        image.save(&path).expect("save Clear Bench png");
        eprintln!("wrote {}", path.display());
        harness.get_by_label("Start sketch").focus();
        harness.run_steps(2);
        harness.key_press(Key::Tab);
        harness.run_steps(2);
        harness.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
        harness.run_steps(2);
        assert!(harness.get_by_label("Start sketch").is_focused(), "keyboard navigation returns to Start sketch");
        let rect = harness.get_by_label("Start sketch").rect();
        let focused = harness.render().expect("focused Start sketch renders");
        focused.save(out_dir().join(format!("focus-{}.png", mode.label().to_lowercase()))).unwrap();
        let ring = focused.get_pixel((rect.left() + 1.0) as u32, rect.center().y as u32).0;
        let fill = focused.get_pixel((rect.left() + 8.0) as u32, rect.center().y as u32).0;
        let luminance = |pixel: [u8; 4]| -> f64 {
            pixel[..3]
                .iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(c, w)| {
                    let v = f64::from(*c) / 255.0;
                    w * if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
                })
                .sum()
        };
        let (a, b) = (luminance(ring), luminance(fill));
        let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
        eprintln!("{} keyboard focus contrast: {contrast:.2}:1", mode.label());
        assert!(contrast >= 3.0, "{}: keyboard focus contrast is {contrast:.2}:1 ({ring:?} / {fill:?})", mode.label());
    }

    harness.run_steps(6);
    assert_eq!(theme::palette(), &theme::clear_bench::PALETTE);
    harness.state_mut().state.push(AppAction::SetTheme(Mode::Light));
    harness.run_steps(2);
    assert_eq!(theme::palette(), &theme::clear_bench::PALETTE);
    assert!(!harness.ctx.global_style().visuals.dark_mode);
    let saved = std::fs::read_to_string(config.join("settings.json")).expect("the choice was saved");
    assert!(saved.contains("\"theme\": \"light\""), "the last choice is in the file: {saved}");
    let _ = std::fs::remove_dir_all(config);
}
