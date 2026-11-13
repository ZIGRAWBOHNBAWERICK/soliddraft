//! Edit Suite palette acceptance through the real app, pointer and renderer.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, Rect, pos2, vec2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::commands;
use soliddraft_app::document::{PlaneKind, Reference, Selection};
use soliddraft_app::settings::FrameLayout;
use soliddraft_app::state::{RibbonTab, Tool};
use soliddraft_app::theme::{self, Mode};

fn palette(app: &App) -> Rect {
    Rect::from_min_max(pos2(0.0, theme::MENU_HEIGHT), pos2(250.0, ribbon::window(app).bottom() - 32.0))
}

fn click(app: &mut App, label: &str) {
    let control = ribbon::control(app, label, palette(app));
    assert!(control.enabled, "{label} is enabled");
    ribbon::click_at(app, control.rect.center());
    app.run_steps(8);
}

fn start_sketch(app: &mut App) {
    app.state_mut().state.doc.selection = Some(Selection::Reference(Reference::Plane(PlaneKind::Top)));
    click(app, "Start sketch");
    assert!(app.state().state.in_sketch());
}

#[test]
fn registry_tools_are_named_in_two_columns_and_run_from_the_palette() {
    let case = ribbon::case("tool-palette-registry");
    let mut app = ribbon::app_at(ribbon::blank(), vec2(1440.0, 1800.0), 1.0);
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.run_steps(8);
    case.run(&mut app, |app| {
        let start = ribbon::control(app, "Start sketch", palette(app));
        assert!(start.rect.width() > 220.0, "the starter spans both columns");
        start_sketch(app);
        let bounds = palette(app);
        let mut columns = Vec::new();
        for (_, items) in commands::ribbon(RibbonTab::Sketch) {
            for (command, _) in items {
                if !(command.shown)(&app.state().state) {
                    continue;
                }
                let label = if command.id == "smart_dimension" {
                    "Smart Dimension".to_owned()
                } else {
                    command.label_in(&app.state().state)
                };
                let control = ribbon::control(app, &label, bounds);
                assert!(bounds.contains_rect(control.rect), "{label} is inside the palette");
                assert_eq!(control.enabled, command.is_enabled(&app.state().state), "{label} follows the registry");
                columns.push(control.rect.left().round() as i32);
                let text = ribbon::painted_in(app, control.rect)
                    .into_iter()
                    .find(|p| p.text == label)
                    .expect("the full tool name is painted");
                assert!(control.rect.expand(1.0).contains_rect(text.full), "{label} wraps inside its tile");
            }
        }
        columns.sort();
        columns.dedup();
        assert_eq!(columns.len(), 2, "exactly two tile columns");
        let painted = ribbon::painted_in(app, bounds);
        for forbidden in ["Modify", "Transform"] {
            assert!(!painted.iter().any(|p| p.text == forbidden));
        }
        for (label, tool) in
            [("Move entities", Tool::Move), ("Copy entities", Tool::Copy), ("Smart Dimension", Tool::Dimension)]
        {
            click(app, label);
            assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, tool);
            assert!(ribbon::control(app, label, bounds).selected);
        }
        click(app, "Relations");
        assert!(app.state().state.sketch.as_ref().unwrap().relations_open);
        assert!(app.query_all_by_label("Show all relations").next().is_some(), "the Relations menu is visible");
        case.frame(app, "relations");
        app.key_press(Key::Escape);
        app.run_steps(8);
        assert!(!app.state().state.sketch.as_ref().unwrap().relations_open);
        assert!(app.query_all_by_label("Show all relations").next().is_none());
        app.set_size(vec2(1440.0, 900.0));
        app.run_steps(8);
        case.frame(app, "sketch-palette");
    });
}

#[test]
fn palette_tiles_pin_command_chips_by_id() {
    let case = ribbon::case("tool-palette-chips");
    let mut app = ribbon::app(ribbon::plate());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.run_steps(8);
    case.run(&mut app, |app| {
        let shell = ribbon::control(app, "Shell", palette(app)).rect;
        let chip = app.get_by_label("Normal to on the command bar").rect();
        let (from, to) = (shell.center(), chip.right_center() - vec2(3.0, 0.0));
        app.hover_at(from);
        app.run_steps(2);
        app.drag_at(from);
        app.run_steps(2);
        for step in 1..=6 {
            app.hover_at(from + (to - from) * (step as f32 / 6.0));
            app.run_steps(2);
        }
        app.drop_at(to);
        app.run_steps(8);
        assert_eq!(app.state().state.command_line.settings.tools.iter().filter(|id| *id == "shell").count(), 1);
        let chip = app.get_by_label("Shell on the command bar").rect();
        ribbon::click_at(app, chip.center());
        app.run_steps(8);
        assert!(matches!(
            app.state().state.editor.as_ref().map(|e| &e.params),
            Some(soliddraft_app::state::EditorParams::Shell(_))
        ));
        case.frame(app, "pinned-shell");
    });
}

#[test]
fn short_windows_scroll_without_collapsing_tools_in_both_themes() {
    let case = ribbon::case("tool-palette-short");
    for mode in [Mode::Light, Mode::Dark] {
        for scale in [1.0, 2.0] {
            let mut app = ribbon::app_at(ribbon::blank(), vec2(960.0, 600.0), scale);
            theme::set_mode(&app.ctx, mode);
            app.state_mut().state.frame_layout = FrameLayout::EditSuite;
            app.run_steps(8);
            case.run(&mut app, |app| {
                start_sketch(app);
                assert!(app.state().state.viewport.rect.width() >= 300.0);
                case.frame(app, &format!("{mode:?}-{scale}x-top"));
                let mut seen = std::collections::BTreeSet::new();
                for _ in 0..20 {
                    for text in ribbon::painted_in(app, palette(app)) {
                        if text.rect.contains_rect(text.full) {
                            seen.insert(text.text);
                        }
                    }
                    app.hover_at(pos2(125.0, 300.0));
                    app.event(egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: vec2(0.0, -120.0),
                        phase: egui::TouchPhase::Move,
                        modifiers: Modifiers::NONE,
                    });
                    app.run_steps(12);
                }
                for (_, items) in commands::ribbon(RibbonTab::Sketch) {
                    for (command, _) in items {
                        if (command.shown)(&app.state().state) {
                            let label = if command.id == "smart_dimension" {
                                "Smart Dimension".to_owned()
                            } else {
                                command.label_in(&app.state().state)
                            };
                            assert!(seen.contains(&label), "{label} can be read at {scale}x in {mode:?}");
                        }
                    }
                }
                assert!(!egui::Popup::is_any_open(&app.ctx));
                case.frame(app, &format!("{mode:?}-{scale}x"));
            });
        }
    }
}
