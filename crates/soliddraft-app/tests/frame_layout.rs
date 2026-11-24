//! Old and current settings open the same Edit Suite frame.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Rect, pos2, vec2};
use egui_kittest::kittest::Queryable;

use soliddraft_app::left_bar;
use soliddraft_app::state::RibbonTab;
use soliddraft_app::theme::{self, Mode};

#[test]
fn old_and_current_frame_settings_open_edit_suite_and_the_command_palette() {
    let case = ribbon::case("frame-layout");
    let config = case.path("config");
    for setting in ["classic", "edit_suite", "unknown"] {
        std::fs::write(config.join("settings.json"), format!(r#"{{"frame_layout":"{setting}"}}"#)).unwrap();
        let mut app = ribbon::app(ribbon::plate());
        case.run(&mut app, |app| {
            assert!(app.state().state.tree_shown(), "{setting}: the part's tree starts shown");
            let palette = &theme::edit_suite::PALETTE;
            assert_eq!(theme::palette(), palette, "{setting}: startup installs the frame's tokens");
            assert_eq!(app.ctx.global_style().visuals.panel_fill, palette.panel);
            assert!(app.state().state.viewport.rect.is_positive());
            assert!(app.query_all_by_label("Extrude").next().is_some());
            assert!(app.query_all_by_label("Edit Suite").next().is_none(), "no layout switch is offered");
            for tab in ["Solid tab", "Sketch tab", "Inspect tab", "View tab"] {
                assert!(app.query_all_by_label(tab).next().is_some(), "{setting}: {tab} remains available");
            }
            for tab in ["Surface tab", "Point cloud tab", "Assembly tab"] {
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
fn left_bar_workspaces_select_the_palette_and_leave_the_title_bar() {
    let case = ribbon::case("left-bar-workspaces");
    for mode in [Mode::Light, Mode::Dark] {
        for (width, scale) in [(1440.0, 1.0), (1280.0, 1.0), (960.0, 1.0), (1280.0, 2.0)] {
            let mut app = ribbon::app_at(ribbon::plate(), vec2(width, 900.0), scale);

            theme::set_mode(&app.ctx, mode);
            app.run_steps(8);
            case.run(&mut app, |app| {
                let title = Rect::from_min_max(pos2(0.0, 0.0), pos2(width, theme::MENU_HEIGHT));
                let palette = suite_palette(app);
                let bar = ribbon::left_bar(app);
                assert_eq!(palette.left(), 0.0, "the tool strip spans the window");
                assert_eq!(palette.top(), title.bottom(), "the tool strip follows the title");
                assert_eq!(bar.left(), 0.0, "the rail is at the window's left edge");
                assert_eq!(bar.top(), palette.bottom(), "the rail starts below the tool strip");
                assert_eq!(bar.width(), left_bar::WIDTH);
                assert!(
                    soliddraft_app::options_bar::shown_rect(&app.ctx).is_none(),
                    "a part with nothing selected hides the options bar"
                );

                // The pointer rests on Inspect before any click: egui hides a tooltip
                // right after one.
                let inspect = ribbon::control(app, "Inspect tab", palette).rect;
                assert_eq!(ribbon::tooltip(app, inspect), "Inspect", "the tab is named on hover");

                let mut previous = palette.left();
                for tab in RibbonTab::ALL.into_iter().filter(|tab| *tab != RibbonTab::Assembly) {
                    let label = format!("{} tab", tab.label());
                    assert_eq!(app.query_all_by_label(&label).count(), 1, "one {label}");
                    let control = ribbon::control(app, &label, palette);
                    assert!(palette.contains_rect(control.rect), "{label} is in the tool strip");
                    assert!(!title.intersects(control.rect), "{label} left the title bar");
                    assert!(!bar.intersects(control.rect), "{label} left the rail");
                    assert!(
                        control.rect.left() >= previous,
                        "workspaces keep their order: {mode:?} width={width} scale={scale} tab={tab:?}"
                    );
                    previous = control.rect.right();
                    ribbon::click_at(app, control.rect.center());
                    app.run_steps(8);
                    assert_eq!(app.state().state.ribbon_tab, tab);
                    assert!(ribbon::control(app, &label, palette).selected);
                    for other in
                        RibbonTab::ALL.into_iter().filter(|other| *other != tab && *other != RibbonTab::Assembly)
                    {
                        let other = format!("{} tab", other.label());
                        assert!(!ribbon::control(app, &other, palette).selected, "only {label} is marked");
                    }
                }
                let sketch = ribbon::control(app, "Sketch tab", palette).rect;
                ribbon::click_at(app, sketch.center());
                app.run_steps(8);
                let palette = suite_palette(app);
                assert!(
                    ribbon::control(app, "Start sketch", palette).rect.left() < palette.left() + 24.0,
                    "Start sketch begins the strip"
                );
                ribbon::control(app, "Smart Dimension", palette);
                assert_eq!(RibbonTab::ALL.map(|tab| tab.id()), ["solid", "sketch", "assembly", "inspect", "view"]);
                // The one File button stays in the title bar at every width.
                let file = app.get_by_label("File").rect();
                assert!(title.contains_rect(file), "File is in the title bar");
                ribbon::click_at(app, file.center());
                app.run_steps(4);
                assert!(app.query_all_by_label_contains("New part").next().is_some(), "File commands remain reachable");
                app.key_press(Key::Escape);
                app.run_steps(4);
                case.frame(app, &format!("left-bar-{mode:?}-{width}-{scale}x"));
            });
        }
    }
}

#[test]
fn a_locked_left_bar_icon_says_why() {
    use soliddraft_app::document::{PlaneKind, SketchPlane};
    use soliddraft_app::state::AppAction;
    let case = ribbon::case("left-bar-sketch-lock");
    let mut app = ribbon::app(ribbon::plate());
    app.state_mut().state.push(AppAction::StartSketchOn(SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 }));
    case.run(&mut app, |app| {
        ribbon::until(app, "the sketch opens", |app| app.state().state.in_sketch());
        app.run_steps(8);
        let bar = suite_palette(app);
        assert!(ribbon::control(app, "Sketch tab", bar).selected);
        let solid = ribbon::control(app, "Solid tab", bar);
        assert!(!solid.enabled);
        assert_eq!(ribbon::tooltip(app, solid.rect), "Close sketch to change workspace");
        assert_eq!(ribbon::tooltip(app, ribbon::control(app, "Sketch tab", bar).rect), "Sketch");
        case.frame(app, "sketch-locked");
    });
}

fn suite_palette(app: &ribbon::App) -> Rect {
    egui::PanelState::load(&app.ctx, egui::Id::new("tool_palette")).expect("the palette is open").outer_rect
}

/// Clicks a tool of the strip, in its group's More menu when the strip has no room for it.
fn click_suite(app: &mut ribbon::App, label: &str) {
    ribbon::click_button(app, label, suite_palette(app));
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

    app.run_steps(8);
    app.state_mut().state.push(AppAction::Assembly(AssemblyAction::New));
    app.run_steps(12);
    case.run(&mut app, |app| {
        assert!(app.state().state.assembly.is_active());
        assert!(app.state().state.tree_shown(), "assembly tree starts open");
        assert!(
            egui::PanelState::load(&app.ctx, egui::Id::new("feature_timeline")).is_none(),
            "no history strip is laid out"
        );
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
        let inspect = ribbon::control(app, "Inspect tab", suite_palette(app)).rect;
        ribbon::click_at(app, inspect.center());
        app.run_steps(8);
        click_suite(app, "Interference");
        assert!(app.state().state.assembly.interference.open);
        let part_camera = app.state().state.viewport.camera.direction();
        let view = ribbon::control(app, "View tab", suite_palette(app)).rect;
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
        assert_eq!(app.state().state.workspaces().len(), 4);
        app.state_mut().state.push(AppAction::Assembly(AssemblyAction::Show));
        app.run_steps(8);
        assert!(ribbon::control(app, "View tab", suite_palette(app)).selected);
        app.set_size(vec2(960.0, 700.0));
        app.run_steps(8);
        assert!(app.state().state.active_viewport().rect.width() >= 300.0);
        // A 960 point window gives the bar its actions' room with the side column closed.
        ribbon::close_tree(app);
        let assembly_tab = ribbon::control(app, "Assembly tab", suite_palette(app)).rect;
        ribbon::click_at(app, assembly_tab.center());
        app.run_steps(8);
        click_suite(app, "Mate");
        let bar = soliddraft_app::options_bar::shown_rect(&app.ctx).expect("the options bar is drawn");
        assert!(bar.contains_rect(app.get_by_label("OK").rect()), "Mate actions fit at 960 points");
        assert!(bar.contains_rect(app.get_by_label("Cancel").rect()));
        let viewport = app.state().state.active_viewport().rect;
        let hint = ribbon::painted(app)
            .into_iter()
            .find(|text| text.text.starts_with("Insert component, or drop"))
            .expect("an empty assembly explains how to add parts");
        assert!(viewport.contains_rect(hint.full), "the whole empty assembly hint fits the viewport");
        case.frame(app, "assembly-empty-960");
        let more = app
            .query_all_by_role(egui::accesskit::Role::Button)
            .find(|n| ribbon::label_of(n).starts_with("More ("))
            .expect("the Mate form has a More sheet at 960 points")
            .rect();
        ribbon::click_at(app, more.center());
        app.run_steps(8);
        let value = app.get_by_label("Mate value").rect();
        assert!(ribbon::window(app).contains_rect(value), "overflow fields remain on screen");
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

    app.state_mut().state.push(AppAction::Drawing(DrawingAction::MakeDrawing));
    app.run_steps(8);
    app.state_mut().state.push(AppAction::Drawing(DrawingAction::CreateDrawing));
    app.run_steps(16);
    app.state_mut().state.drawing.session.as_mut().unwrap().doc.drawing.meta.designer = "SolidDraft".into();
    case.run(&mut app, |app| {
        assert!(app.state().state.drawing.is_active());
        assert!(app.state().state.tree_shown(), "drawing tree starts open");
        assert!(
            egui::PanelState::load(&app.ctx, egui::Id::new("feature_timeline")).is_none(),
            "no history strip is laid out"
        );
        assert!(app.query_all_by_label("Properties").next().is_none(), "drawing editors live on the options bar");
        assert_eq!(
            app.state().state.workspaces().iter().map(|tab| tab.id()).collect::<Vec<_>>(),
            ["views", "annotate", "sheet", "view"]
        );
        assert!(app.query_all_by_label("Assistant tab").next().is_none());
        for (workspace, command) in
            [("Views", "Standard views"), ("Annotate", "Smart dimension"), ("Sheet", "Add sheet"), ("View", "Fit")]
        {
            let tab = ribbon::control(app, &format!("{workspace} tab"), suite_palette(app)).rect;
            ribbon::click_at(app, tab.center());
            app.run_steps(8);
            ribbon::control(app, command, suite_palette(app));
            assert!(!ribbon::painted_in(app, suite_palette(app)).iter().any(|text| text.text == "Assistant"));
        }
        let views = ribbon::control(app, "Views tab", suite_palette(app)).rect;
        ribbon::click_at(app, views.center());
        app.run_steps(8);
        case.frame(app, "drawing-views");
        let sheet = ribbon::control(app, "Sheet tab", suite_palette(app)).rect;
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
        // A 960 point window gives the bar its sheet controls' room with the side column closed.
        ribbon::close_tree(app);
        let bar = soliddraft_app::options_bar::shown_rect(&app.ctx).expect("the options bar is drawn");
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

#[test]
fn serialized_classic_tree_choice_migrates_and_later_suite_choices_survive() {
    let case = ribbon::case("classic-saved-pane");
    let config = case.path("config/settings.json");
    for shown in [true, false] {
        let mut legacy = soliddraft_app::panels::Layout::default();
        legacy.tree.shown = shown;
        legacy.tree.width = 333.0;
        legacy.suite.part = !shown;
        std::fs::write(
            &config,
            serde_json::json!({
                "frame_layout": "classic", "layout": legacy, "future": {"keep": 7}
            })
            .to_string(),
        )
        .unwrap();
        let mut app = ribbon::app(ribbon::plate());
        case.run(&mut app, |app| {
            assert_eq!(
                app.state().state.tree_shown(),
                shown,
                "the active Classic choice wins over its inactive Suite choice"
            );
            assert_eq!(app.state().state.layout.tree.width, 333.0);
            ribbon::run_command(app, "toggle_tree");
            app.run_steps(30);
            assert_eq!(app.state().state.tree_shown(), !shown);
            let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
            assert_eq!(saved["frame_layout"], "edit_suite", "a layout save completes migration");
            assert_eq!(saved["future"]["keep"], 7);
        });
        drop(app);
        let mut reopened = ribbon::app(ribbon::plate());
        case.run(&mut reopened, |app| {
            assert_eq!(app.state().state.tree_shown(), !shown, "the new-frame choice survives another launch");
            assert_eq!(app.state().state.layout.tree.width, 333.0);
        });
    }
}

/// A greyed-out menu entry says what it does, or why it is off, on hover. egui shows plain
/// hover text on enabled widgets only, so a disabled command used to explain nothing.
#[test]
fn a_disabled_menu_entry_explains_itself_on_hover() {
    let case = ribbon::case("frame-menu-disabled-tip");
    let mut app = ribbon::app(ribbon::blank());
    case.run(&mut app, |app| {
        let command = soliddraft_app::commands::get("make_assembly").expect("the command");
        assert!(!command.is_enabled(&app.state().state), "a part with no body makes no assembly");
        let tip = command.tip(&app.state().state);
        let area = ribbon::window(app);
        ribbon::click_button(app, "File", area);
        app.run_steps(8);
        let entry = ribbon::menu_entry(app, "File", "Make assembly from part");
        let enabled = ribbon::menu_entry(app, "File", "New part");
        // egui holds tooltips back right after a click: rest elsewhere, then on the entry.
        app.hover_at(enabled.center());
        app.run_steps(30);
        app.hover_at(entry.center());
        app.run_steps(90);
        let painted: Vec<String> = ribbon::painted(app).into_iter().map(|p| p.text).collect();
        assert!(painted.iter().any(|text| text.contains(&tip)), "the disabled entry shows {tip:?}: {painted:?}");
        case.frame(app, "disabled-entry-tip");
    });
}

/// The options sheet takes a click on Close options in the first frame after it opens.
/// Its first pass only measures it, invisible and deaf to clicks; a slow software
/// renderer can run no other frame before the next click, which the sheet then lost.
#[test]
fn the_options_sheet_takes_a_click_in_the_frame_after_it_opens() {
    let mut app = ribbon::app(ribbon::plate());
    let feature = app.state().state.doc.features.iter().find(|f| f.kind.noun() == "extrude").expect("the plate").id;
    app.state_mut()
        .state
        .push(soliddraft_app::state::AppAction::Select(Some(soliddraft_app::document::Selection::Feature(feature))));
    app.run_steps(6);
    // One frame for the press and one for the release, as the browser runs the app on
    // each pointer event. A page's own synthetic events bring no pointer move.
    let click_in_one_frame = |app: &mut ribbon::App, at: egui::Pos2| {
        for pressed in [true, false] {
            app.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            app.step();
        }
    };
    let details = app.get_by_label("Details").rect().center();
    click_in_one_frame(&mut app, details);
    assert!(app.state().state.options_details, "Details opens the sheet");
    let close = app.get_by_label("Close options").rect().center();
    click_in_one_frame(&mut app, close);
    assert!(!app.state().state.options_details, "Close options closes the sheet on the frame after it opened");
}

/// The walk-over's frame fixes. The File menu has no theme pair that changed only the
/// grid, and names the browser's feedback form only in the browser. The options sheet
/// has one close control. Fit frames the part above the navigation bar.
#[test]
fn view_insert_options_and_fit_after_the_walkover() {
    let case = ribbon::case("frame-walkover");
    let mut app = ribbon::app(ribbon::plate());
    case.run(&mut app, |app| {
        let area = ribbon::window(app);
        ribbon::click_button(app, "File", area);
        app.run_steps(6);
        let painted: Vec<String> = ribbon::painted(app).into_iter().map(|p| p.text).collect();
        assert!(painted.iter().any(|t| t == "New part"), "File lists New part: {painted:?}");
        for gone in ["Light theme", "Dark theme"] {
            assert!(!painted.iter().any(|t| t == gone), "File lists no {gone}: {painted:?}");
        }
        assert!(app.query_all_by_label("Give feedback…").next().is_none(), "the desktop has no feedback form");
        case.frame(app, "file-menu");
        app.key_press(Key::Escape);
        app.run_steps(4);

        // One close control on the options sheet: the selection's Clear X is not there.
        let face = app.state().state.doc.faces.iter().find(|f| f.label == "End cap").expect("the top").id;
        app.state_mut()
            .state
            .push(soliddraft_app::state::AppAction::Select(Some(soliddraft_app::document::Selection::Face(face))));
        app.run_steps(6);
        ribbon::open_options(app);
        assert_eq!(app.query_all_by_label("Close options").count(), 1);
        assert!(app.query_all_by_label("Clear selection (Esc)").next().is_none(), "no second close control");
        case.frame(app, "options-one-close");
        ribbon::close_options(app);
        app.state_mut().state.push(soliddraft_app::state::AppAction::Select(None));
        app.run_steps(4);

        // Fit keeps the part clear of the pills and the navigation bar at the bottom.
        app.key_press(Key::Z);
        app.run_steps(60);
        let state = &app.state().state;
        let (lo, hi) = state.doc.bounds().unwrap();
        let view = state.viewport.rect;
        let mut lowest = f32::MIN;
        for x in [lo.x, hi.x] {
            for y in [lo.y, hi.y] {
                for z in [lo.z, hi.z] {
                    let p = state.viewport.camera.project(glam::Vec3::new(x, y, z), view).unwrap();
                    lowest = lowest.max(p.y);
                }
            }
        }
        assert!(lowest < view.bottom() - 84.0, "the part's lowest corner {lowest} clears the bar band of {view:?}");
        case.frame(app, "fit-above-the-bar");
    });
}

/// A feature tree row takes a click as the browser delivers it, one frame for the press
/// and one for the release with no pointer move. The options bar draws the Details
/// button for the new selection in the next frame, and that button then takes one click
/// delivered the same way. The browser checks wait for that frame before clicking
/// Details: a click sent before it lands where Details is not drawn yet.
#[test]
fn a_tree_row_and_then_details_take_clicks_in_the_frames_they_are_drawn() {
    let mut app = ribbon::app(ribbon::plate());
    app.run_steps(6);
    let feature =
        app.state().state.doc.features.iter().find(|f| f.kind.noun() == "extrude").expect("the plate").clone();
    let click_in_one_frame = |app: &mut ribbon::App, at: egui::Pos2| {
        for pressed in [true, false] {
            app.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            app.step();
        }
    };
    let row = app.get_all_by_label(&feature.name).next().expect("the plate's tree row").rect().center();
    click_in_one_frame(&mut app, row);
    assert_eq!(
        app.state().state.doc.selection,
        Some(soliddraft_app::document::Selection::Feature(feature.id)),
        "the row's click selects its feature"
    );
    // The browser checks prompt this frame with a pointer move before they click Details.
    app.step();
    let details = app.get_by_label("Details").rect().center();
    click_in_one_frame(&mut app, details);
    assert!(app.state().state.options_details, "Details opens the sheet");
}
