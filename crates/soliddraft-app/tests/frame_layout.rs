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
            assert_eq!(app.state().state.layout.tree.shown, layout == FrameLayout::Classic);
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
                let options = egui::PanelState::load(&app.ctx, egui::Id::new("options_bar"))
                    .expect("the options bar is laid out").outer_rect;
                let palette = egui::PanelState::load(&app.ctx, egui::Id::new("tool_palette"))
                    .expect("the palette is laid out").outer_rect;
                assert_eq!(options.top(), title.bottom(), "the options bar follows the title");
                assert_eq!(palette.top(), options.bottom(), "the palette follows options without a duplicate tab row");
                assert!(
                    ribbon::control(app, "Start sketch", palette).rect.top() < palette.top() + 10.0,
                    "Start sketch begins the palette"
                );
                ribbon::control(app, "Smart Dimension", palette);
                assert_eq!(RibbonTab::ALL.map(|tab| tab.id()), ["solid", "sketch", "assembly", "inspect", "view"]);
                if width < 1280.0 {
                    let menu = app.get_by_label("Menu").rect();
                    assert!(!menu.intersects(sketch));
                    ribbon::click_at(app, menu.center());
                    app.run_steps(4);
                    let file = app.get_by_label_contains("File").rect();
                    ribbon::click_at(app, file.center());
                    app.run_steps(4);
                    assert!(app.query_all_by_label_contains("New part").next().is_some(), "compact File commands remain reachable");
                    app.key_press(Key::Escape);
                    app.run_steps(4);
                    app.key_press(Key::Escape);
                    app.run_steps(4);
                }
                case.frame(app, &format!("title-{mode:?}-{width}-{scale}x"));
            });
        }
    }
}

fn suite_palette(app: &ribbon::App) -> Rect {
    egui::PanelState::load(&app.ctx, egui::Id::new("tool_palette")).expect("the palette is open").outer_rect
}

fn click_suite(app: &mut ribbon::App, label: &str) {
    let control = ribbon::control(app, label, suite_palette(app));
    assert!(control.enabled, "{label} is enabled");
    ribbon::click_at(app, control.rect.center());
    app.run_steps(8);
}

#[test]
fn assembly_workspaces_route_named_tools_and_keep_the_part_camera() {
    use soliddraft_app::assembly::{AssemblyAction, Workspace};
    use soliddraft_app::state::AppAction;
    use soliddraft_app::viewport::render::DisplayMode;
    let case = ribbon::case("assembly-workspaces");
    let mut app = ribbon::app(ribbon::plate());
    ribbon::until(&mut app, "the plate proof", |app| !app.state().state.doc.proof_pending());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.run_steps(8);
    app.state_mut().state.push(AppAction::Assembly(AssemblyAction::New));
    app.run_steps(12);
    case.run(&mut app, |app| {
        assert!(app.state().state.assembly.is_active());
        assert!(app.state().state.tree_shown(), "assembly tree starts open");
        assert!(!ribbon::painted(app).iter().any(|text| text.text == "History"), "the part timeline is absent");
        assert!(app.query_all_by_label("Properties").next().is_none(), "assembly editors live on the options bar");
        assert_eq!(
            app.state().state.workspaces().iter().map(|tab| tab.id()).collect::<Vec<_>>(),
            ["assembly", "inspect", "view"]
        );
        for absent in ["Solid tab", "Sketch tab"] {
            assert!(app.query_all_by_label(absent).next().is_none());
        }
        click_suite(app, "Mate");
        assert!(app.state().state.assembly.mate.open);
        assert!(app.query_all_by_label("Mate value").next().is_some(), "the Mate fields are on the bar");
        let confirm = app.get_by_label("OK").rect();
        ribbon::click_at(app, confirm.center());
        app.run_steps(8);
        assert!(
            app.state().state.assembly.mate.error.is_some(),
            "Mate validates missing picks through the existing action"
        );
        ribbon::type_command(app, "ai");
        assert!(app.state().state.command_line.session_open, "Mate leaves the command line's Enter key alone");
        app.state_mut().state.command_line.close_session();
        app.run_steps(8);
        let inspect = app.get_by_label("Inspect tab").rect();
        ribbon::click_at(app, inspect.center());
        app.run_steps(8);
        click_suite(app, "Interference");
        assert!(app.state().state.assembly.interference.open);
        let part_camera = app.state().state.viewport.camera.direction();
        let view = app.get_by_label("View tab").rect();
        ribbon::click_at(app, view.center());
        app.run_steps(8);
        click_suite(app, "Graphite");
        assert_eq!(app.state().state.active_viewport().scene, Some(soliddraft_app::viewport::scenes::Scene::Graphite));
        assert_eq!(app.state().state.viewport.scene, Some(soliddraft_app::viewport::scenes::Scene::Graphite));
        click_suite(app, "Front");
        assert_eq!(app.state().state.assembly.session.as_ref().unwrap().viewport.camera.view_name, "Front");
        assert_eq!(app.state().state.viewport.camera.direction(), part_camera);
        click_suite(app, "Wireframe");
        assert_eq!(app.state().state.assembly.session.as_ref().unwrap().viewport.display, DisplayMode::Wireframe);
        assert_ne!(app.state().state.viewport.display, DisplayMode::Wireframe);
        assert_eq!(app.state().state.assembly.workspace, Workspace::View);
        case.frame(app, "assembly-view");
        app.state_mut().state.push(AppAction::Assembly(AssemblyAction::ShowPart));
        app.run_steps(8);
        assert_eq!(app.state().state.workspaces().len(), 5);
        app.state_mut().state.push(AppAction::Assembly(AssemblyAction::Show));
        app.run_steps(8);
        assert!(ribbon::control(app, "View tab", ribbon::window(app)).selected);
        app.set_size(vec2(960.0, 700.0));
        app.run_steps(8);
        assert!(app.state().state.active_viewport().rect.width() >= 300.0);
        let assembly_tab = app.get_by_label("Assembly tab").rect();
        ribbon::click_at(app, assembly_tab.center());
        app.run_steps(8);
        click_suite(app, "Mate");
        let bar = egui::PanelState::load(&app.ctx, egui::Id::new("options_bar")).unwrap().outer_rect;
        assert!(bar.contains_rect(app.get_by_label("OK").rect()), "Mate actions fit at 960 points");
        assert!(bar.contains_rect(app.get_by_label("Cancel").rect()));
        case.frame(app, "assembly-mate-960");
    });
}

#[test]
fn drawing_has_four_workspaces_and_asks_for_help_on_the_ai_line() {
    use soliddraft_app::drawing::DrawingAction;
    use soliddraft_app::state::AppAction;
    let case = ribbon::case("drawing-workspaces");
    let mut app = ribbon::app(ribbon::plate());
    ribbon::until(&mut app, "the plate proof", |app| !app.state().state.doc.proof_pending());
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    app.state_mut().state.push(AppAction::Drawing(DrawingAction::MakeDrawing));
    app.run_steps(8);
    app.state_mut().state.push(AppAction::Drawing(DrawingAction::CreateDrawing));
    app.run_steps(16);
    case.run(&mut app, |app| {
        assert!(app.state().state.drawing.is_active());
        assert!(app.state().state.tree_shown(), "drawing tree starts open");
        assert!(!ribbon::painted(app).iter().any(|text| text.text == "History"), "the part timeline is absent");
        assert!(app.query_all_by_label("Properties").next().is_none(), "drawing editors live on the options bar");
        assert_eq!(
            app.state().state.workspaces().iter().map(|tab| tab.id()).collect::<Vec<_>>(),
            ["views", "annotate", "sheet", "view"]
        );
        assert!(app.query_all_by_label("Assistant tab").next().is_none());
        for (workspace, command) in
            [("Views", "Standard views"), ("Annotate", "Smart dimension"), ("Sheet", "Add sheet"), ("View", "Fit")]
        {
            let tab = app.get_by_label(&format!("{workspace} tab")).rect();
            ribbon::click_at(app, tab.center());
            app.run_steps(8);
            ribbon::control(app, command, suite_palette(app));
            assert!(!ribbon::painted_in(app, suite_palette(app)).iter().any(|text| text.text == "Assistant"));
        }
        let views = app.get_by_label("Views tab").rect();
        ribbon::click_at(app, views.center());
        app.run_steps(8);
        case.frame(app, "drawing-views");
        let sheet = app.get_by_label("Sheet tab").rect();
        ribbon::click_at(app, sheet.center());
        app.run_steps(8);
        let before = app.state().state.drawing.session.as_ref().unwrap().doc.drawing.sheets.len();
        click_suite(app, "Add sheet");
        assert_eq!(app.state().state.drawing.session.as_ref().unwrap().doc.drawing.sheets.len(), before + 1);
        for id in ["drawing_check_sheet", "drawing_propose_dimensions", "drawing_find_issues", "drawing_explain"] {
            let command = soliddraft_app::commands::get(id).unwrap();
            assert!(command.is_enabled(&app.state().state));
            ribbon::type_command(app, command.label);
            app.run_steps(8);
            assert!(app.state().state.command_line.session_open);
            assert!(
                app.state().state.chat.messages.iter().any(|message| message.role == soliddraft_app::state::Role::User)
            );
            app.state_mut().state.command_line.close_session();
            app.run_steps(8);
        }
        case.frame(app, "drawing-sheet");
        app.set_size(vec2(960.0, 700.0));
        app.run_steps(8);
        assert!(app.state().state.drawing.session.as_ref().unwrap().canvas_rect.width() >= 300.0);
        let bar = egui::PanelState::load(&app.ctx, egui::Id::new("options_bar")).unwrap().outer_rect;
        let properties = app
            .query_all_by_label("Sheet properties")
            .find(|control| bar.contains_rect(control.rect()))
            .expect("sheet properties is on the bar")
            .rect();
        ribbon::click_at(app, properties.center());
        app.run_steps(8);
        assert!(app.query_all_by_label("Sheet name field").next().is_some(), "the sheet editor opens in the new frame");
        case.frame(app, "drawing-properties-960");
    });
}
