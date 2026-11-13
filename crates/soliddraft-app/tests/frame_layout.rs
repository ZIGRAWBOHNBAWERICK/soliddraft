//! Both internal frame choices run the real app without a user-facing layout toggle.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, Rect, pos2, vec2};
use egui_kittest::kittest::Queryable;
use soliddraft_app::settings::FrameLayout;
use soliddraft_app::state::RibbonTab;
use soliddraft_app::theme::{self, Mode};

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
            assert_eq!(app.ctx.global_style().visuals.panel_fill, palette.panel);
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

#[test]
fn title_bar_workspaces_select_the_palette_without_a_duplicate_strip() {
    let case = ribbon::case("title-bar-workspaces");
    for mode in [Mode::Light, Mode::Dark] {
        for (width, scale) in [(1440.0, 1.0), (1280.0, 1.0), (960.0, 1.0), (1280.0, 2.0)] {
            let mut app = ribbon::app_at(ribbon::plate(), vec2(width, 900.0), scale);
            app.state_mut().state.frame_layout = FrameLayout::EditSuite;
            theme::set_mode(&app.ctx, mode);
            app.run_steps(8);
            case.run(&mut app, |app| {
                let title = Rect::from_min_max(pos2(0.0, 0.0), pos2(width, theme::MENU_HEIGHT));
                let mut previous = 0.0;
                for tab in RibbonTab::ALL {
                    let label = format!("{} tab", tab.label());
                    assert_eq!(app.query_all_by_label(&label).count(), 1, "one {label}");
                    let control = ribbon::control(app, &label, title);
                    assert!(title.contains_rect(control.rect), "{label} fits in the title bar");
                    assert!(control.rect.left() > previous, "workspaces keep their order: {mode:?} width={width} scale={scale} tab={tab:?} previous_right={previous} current={:?}", control.rect);
                    previous = control.rect.right();
                    ribbon::click_at(app, control.rect.center());
                    app.run_steps(8);
                    assert_eq!(app.state().state.ribbon_tab, tab);
                    assert!(ribbon::control(app, &label, title).selected);
                }
                let view = app.get_by_label("View tab").rect();
                let undo = app.get_by_label("Undo").rect();
                assert!(!undo.intersects(view), "quick access leaves room for the switch");
                let sketch = app.get_by_label("Sketch tab").rect();
                ribbon::click_at(app, sketch.center());
                app.run_steps(8);
                let palette = Rect::from_min_max(pos2(0.0, theme::MENU_HEIGHT), pos2(250.0, 850.0));
                assert!(
                    ribbon::control(app, "Start sketch", palette).rect.top() < theme::MENU_HEIGHT + 60.0,
                    "the palette begins directly under the title bar"
                );
                ribbon::control(app, "Smart Dimension", palette);
                assert_eq!(RibbonTab::ALL.map(|tab| tab.id()), ["solid", "sketch", "assembly", "inspect", "view"]);
                if width < 1280.0 {
                    let menu = app.get_by_label("Menu").rect();
                    assert!(!menu.intersects(sketch));
                    ribbon::click_at(app, menu.center());
                    app.run_steps(4);
                    assert!(app.query_all_by_label("File").next().is_some(), "compact menus remain reachable");
                    app.key_press(Key::Escape);
                    app.run_steps(4);
                }
                case.frame(app, &format!("title-{mode:?}-{width}-{scale}x"));
            });
        }
    }
}
