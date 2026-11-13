//! Edit Suite palette acceptance through the real app, pointer and renderer.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, Rect, pos2, vec2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::commands::{self, KeyChord, ShortcutPreset, Shortcuts};
use soliddraft_app::document::{PlaneKind, Reference, Selection, ViewItem};
use soliddraft_app::settings::FrameLayout;
use soliddraft_app::state::{RibbonTab, Tool};
use soliddraft_app::theme::{self, Mode};

fn palette(app: &App) -> Rect {
    egui::PanelState::load(&app.ctx, egui::Id::new("tool_palette")).expect("the palette is laid out").outer_rect
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
fn a_sketch_tool_asks_for_a_plane_then_arms_itself() {
    let case = ribbon::case("tool-palette-plane-pick");
    let mut app = ribbon::app(ribbon::blank());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.state_mut().state.viewport.references.show_origin_planes = false;
    app.run_steps(8);
    case.run(&mut app, |app| {
        app.get_by_label("Sketch tab").click();
        app.run_steps(8);
        click(app, "Line");
        assert!(!app.state().state.in_sketch());
        assert!(matches!(
            app.state().state.editor.as_ref().map(|e| &e.params),
            Some(soliddraft_app::state::EditorParams::PickPlane)
        ));
        case.frame(app, "choose-plane");
        let plane = ribbon::control(app, "Top plane", plane_bar(app));
        ribbon::click_at(app, plane.rect.center());
        app.run_steps(8);
        assert!(app.state().state.in_sketch());
        assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Line);
        assert!(app.state().state.editor.is_none());
        let center = app.state().state.viewport.rect.center();
        ribbon::click_at(app, center - vec2(50.0, 30.0));
        ribbon::click_at(app, center + vec2(50.0, 30.0));
        app.run_steps(8);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().sketch.drawn().count(), 1);
        case.frame(app, "line-armed");
    });
}

fn plane_bar(app: &App) -> Rect {
    Rect::from_min_max(
        pos2(0.0, theme::MENU_HEIGHT + theme::RIBBON_TABS_HEIGHT),
        pos2(ribbon::window(app).right(), theme::MENU_HEIGHT + theme::RIBBON_TABS_HEIGHT + 44.0),
    )
}

#[test]
fn plane_picking_is_temporary_and_cancel_drops_the_pending_tool() {
    let case = ribbon::case("tool-palette-plane-cancel");
    let mut app = ribbon::app(ribbon::blank());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.state_mut().state.viewport.references.show_origin_planes = false;
    for plane in PlaneKind::ALL {
        app.state_mut().state.doc.set_view_choice(ViewItem::Plane(plane), Some(false));
    }
    app.run_steps(8);
    case.run(&mut app, |app| {
        app.get_by_label("Sketch tab").click();
        app.run_steps(8);
        click(app, "Line");
        assert!(soliddraft_app::tree_visibility::visible(&app.state().state).planes.into_iter().all(|p| p));
        app.key_press(Key::Escape);
        app.run_steps(8);
        assert!(app.state().state.editor.is_none());
        assert!(!app.state().state.in_sketch());
        assert!(app.state().state.doc.features.is_empty());
        assert_eq!(soliddraft_app::tree_visibility::visible(&app.state().state).planes, [false; 3]);
        click(app, "Start sketch");
        let plane = ribbon::control(app, "Front plane", plane_bar(app));
        ribbon::click_at(app, plane.rect.center());
        app.run_steps(8);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Select);
        assert!(!app.state().state.viewport.references.show_origin_planes);
        case.frame(app, "front-sketch");
    });
}

#[test]
fn sketch_tools_accept_viewport_planes_and_flat_faces() {
    let case = ribbon::case("tool-palette-viewport-plane");
    let mut app = ribbon::app(ribbon::blank());
    assert!(!commands::get("sketch_circle").unwrap().is_enabled(&app.state().state));
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.state_mut().state.viewport.references.show_origin_planes = false;
    app.run_steps(8);
    case.run(&mut app, |app| {
        app.get_by_label("Sketch tab").click();
        app.run_steps(8);
        click(app, "Circle");
        ribbon::click_world(app, glam::Vec3::new(0.0, -40.0, 40.0));
        app.run_steps(8);
        let session = app.state().state.sketch.as_ref().expect("the viewport plane starts a sketch");
        assert_eq!(session.tool, Tool::Circle);
        assert!(matches!(
            app.state().state.doc.feature(session.feature).unwrap().sketch().unwrap().plane,
            soliddraft_app::document::SketchPlane::Reference { plane: PlaneKind::Right, .. }
        ));
        case.frame(app, "right-plane");
    });
    let mut app = ribbon::app(ribbon::plate());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.state_mut().state.doc.selection = None;
    app.run_steps(8);
    case.run(&mut app, |app| {
        app.get_by_label("Sketch tab").click();
        app.run_steps(8);
        click(app, "Circle");
        ribbon::click_world(app, glam::Vec3::new(10.0, 5.0, 20.0));
        app.run_steps(8);
        let session = app.state().state.sketch.as_ref().expect("the flat face starts a sketch");
        assert_eq!(session.tool, Tool::Circle);
        assert!(matches!(
            app.state().state.doc.feature(session.feature).unwrap().sketch().unwrap().plane,
            soliddraft_app::document::SketchPlane::Face { .. }
        ));
        case.frame(app, "flat-face");
    });
}

#[test]
fn sketch_workspace_locks_and_smart_dimension_leads_the_palette() {
    let case = ribbon::case("tool-palette-sketch-takeover");
    let mut app = ribbon::app(ribbon::blank());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.run_steps(8);
    case.run(&mut app, |app| {
        start_sketch(app);
        let dimension = ribbon::control(app, "Smart Dimension", palette(app)).rect;
        let line = ribbon::control(app, "Line", palette(app)).rect;
        assert!(dimension.width() > 220.0);
        assert!(dimension.bottom() < line.top());
        for tab in ["Solid", "Assembly", "Inspect", "View"] {
            let control = ribbon::tab(app, tab);
            assert!(!control.enabled, "{tab} is locked during the sketch");
            ribbon::click_at(app, control.rect.center());
            app.run_steps(8);
            assert_eq!(app.state().state.ribbon_tab, RibbonTab::Sketch);
        }
        let painted = ribbon::painted_in(app, ribbon::ribbon_region(app));
        assert!(painted.iter().any(|p| p.text.contains("Sketch1") && p.text.contains("Top plane")));
        assert!(app.query_all_by_label("Extrude").next().is_none());
        case.frame(app, "locked-workspace");
        click(app, "Exit");
        assert!(!app.state().state.in_sketch());
        assert!(ribbon::tab(app, "Solid").enabled);
    });
}

#[test]
fn sketch_keys_follow_every_preset_and_keep_custom_bindings() {
    let case = ribbon::case("tool-palette-sketch-keys");
    let mut app = ribbon::app(ribbon::blank());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.run_steps(8);
    case.run(&mut app, |app| {
        start_sketch(app);
        for preset in ShortcutPreset::ALL {
            app.state_mut().state.shortcuts = Shortcuts::for_preset(preset);
            for (key, tool) in [
                (Key::L, Tool::Line),
                (Key::R, Tool::Rectangle),
                (Key::C, Tool::Circle),
                (Key::A, Tool::Arc),
                (Key::D, Tool::Dimension),
                (Key::T, Tool::Trim),
                (Key::O, Tool::Offset),
                (Key::M, Tool::Move),
            ] {
                app.key_press(key);
                app.run_steps(8);
                assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, tool, "{preset:?}: {key:?}");
            }
        }
        app.state_mut().state.shortcuts.assign("sketch_move", KeyChord::plain(Key::B)).unwrap();
        app.key_press(Key::L);
        app.run_steps(8);
        app.key_press(Key::M);
        app.run_steps(8);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Line);
        app.key_press(Key::B);
        app.run_steps(8);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Move);
        let input = ribbon::input_rect(app);
        ribbon::click_at(app, input.center());
        app.key_press(Key::C);
        app.run_steps(8);
        assert_eq!(app.state().state.sketch.as_ref().unwrap().tool, Tool::Move, "typing keeps the tool");
    });
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
                if !(command.shown)(&app.state().state) || matches!(command.id, "sketch" | "exit_sketch") {
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
                        if (command.shown)(&app.state().state) && !matches!(command.id, "sketch" | "exit_sketch") {
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
