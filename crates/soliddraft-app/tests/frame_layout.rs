//! Both internal frame choices run the real app without a user-facing layout toggle.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers};
use egui_kittest::kittest::Queryable;
use soliddraft_app::settings::FrameLayout;
use soliddraft_app::theme;

#[test]
fn both_frame_settings_open_the_workspace_and_command_palette() {
    let case = ribbon::case("frame-layout");
    let config = case.path("config");
    for (setting, layout) in [("classic", FrameLayout::Classic), ("edit_suite", FrameLayout::EditSuite)] {
        std::fs::write(config.join("settings.json"), format!(r#"{{"frame_layout":"{setting}"}}"#)).unwrap();
        let mut app = ribbon::app(ribbon::plate());
        case.run(&mut app, |app| {
            assert_eq!(app.state().state.frame_layout, layout);
            let palette = match layout {
                FrameLayout::Classic => &theme::LIGHT,
                FrameLayout::EditSuite => &theme::edit_suite::PALETTE,
            };
            assert_eq!(theme::palette(), palette, "{setting}: startup installs the frame's tokens");
            assert_eq!(app.ctx.style().visuals.panel_fill, palette.panel);
            assert!(app.state().state.viewport.rect.is_positive());
            assert!(app.query_all_by_label("Extrude").next().is_some());
            assert!(app.query_all_by_label("Edit Suite").next().is_none(), "the switch stays internal");
            for tab in ["Solid tab", "Sketch tab", "Assembly tab", "Inspect tab", "View tab"] {
                assert!(app.query_all_by_label(tab).next().is_some(), "{setting}: {tab} remains available");
            }
            for tab in ["Surface tab", "Point cloud tab"] {
                assert!(app.query_all_by_label(tab).next().is_none(), "{setting}: {tab} is removed");
            }
            let painted = ribbon::painted(app);
            for removed in ["Surface", "Point cloud", "Search commands and features"] {
                assert!(!painted.iter().any(|p| p.text == removed), "{setting}: {removed} is not painted");
            }
            case.frame(app, setting);
            app.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
            app.run_steps(4);
            assert!(app.state().state.palette.is_some(), "the shared shortcut works in {setting}");
            app.key_press(Key::Escape);
            app.run_steps(4);
            assert!(app.state().state.palette.is_none());
        });
    }
}
