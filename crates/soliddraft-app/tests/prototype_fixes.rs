//! Regression checks for the six L25 fixes, through the real app and renderer.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Pos2, pos2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::{
    chat::ChatState,
    sketch::{ConstraintKind, P2, SolveStatus},
    sketcher::Frame,
    viewport::scenes::Scene,
};

fn click(app: &mut App, label: &str) {
    let rect = app.get_all_by_label(label).find(|n| ribbon::window(app).contains_rect(n.rect())).unwrap().rect();
    ribbon::click_at(app, rect.center());
    app.run_steps(8);
}

fn start_sketch(app: &mut App) {
    click(app, "Start sketch");
    click(app, "Top plane");
    assert!(app.state().state.in_sketch());
}

fn screen(app: &App, p: P2) -> Pos2 {
    let state = &app.state().state;
    Frame { plane: state.sketch.as_ref().unwrap().plane, camera: &state.viewport.camera, rect: state.viewport.rect }
        .screen(p)
        .unwrap()
}

fn type_line(app: &mut App, text: &str) {
    click(app, "Command line");
    app.get_by_label("Command line").type_text(text);
    app.run_steps(8);
}

fn near_line(app: &mut App, target: [u8; 3]) -> bool {
    let line = app.state().state.sketch.as_ref().unwrap().sketch.entities.iter().find(|e| e.is_line()).unwrap();
    let (a, b) = (line.point(0).unwrap(), line.point(1).unwrap());
    let at = screen(app, P2::new(a.u * 0.65 + b.u * 0.35, a.v * 0.65 + b.v * 0.35));
    app.hover_at(pos2(1430.0, 890.0));
    app.run_steps(8);
    let image = app.render().unwrap();
    (-3..=3).any(|dx| {
        (-3..=3).any(|dy| {
            let p = image.get_pixel((at.x as i32 + dx) as u32, (at.y as i32 + dy) as u32).0;
            p[..3].iter().zip(target).all(|(a, b)| a.abs_diff(b) <= 12)
        })
    })
}

#[test]
fn individual_relations_are_on_the_palette() {
    let case = ribbon::case("prototype-relations");
    let mut app = ribbon::app(ribbon::blank());
    start_sketch(&mut app);
    case.run(&mut app, |app| {
        let palette = ribbon::palette_region(app);
        let buttons = ribbon::buttons_in(app, palette);
        let missing: Vec<_> = [
            "Horizontal",
            "Vertical",
            "Coincident",
            "Parallel",
            "Perpendicular",
            "Tangent",
            "Equal",
            "Concentric",
            "Midpoint",
            "Symmetric",
            "Fix",
        ]
        .into_iter()
        .filter(|name| !buttons.iter().any(|(label, rect)| label == name && palette.contains_rect(*rect)))
        .collect();
        assert!(missing.is_empty(), "relations missing from the open palette: {missing:?}");
    });
    case.frame(&mut app, "relations");
}

#[test]
fn defined_sketch_geometry_paints_above_origin_axes() {
    let case = ribbon::case("prototype-axis-colour");
    let mut app = ribbon::app(ribbon::blank());
    start_sketch(&mut app);
    click(&mut app, "Line");
    for p in [P2::ZERO, P2::new(20.0, 30.0)] {
        let at = screen(&app, p);
        ribbon::click_at(&mut app, at);
    }
    app.key_press(Key::Escape);
    app.run_steps(8);
    assert!(near_line(&mut app, [29, 79, 224]), "under defined line is blue");
    click(&mut app, "Smart Dimension");
    let at = screen(&app, P2::new(10.0, 15.0));
    ribbon::click_at(&mut app, at);
    let at = screen(&app, P2::new(25.0, 15.0));
    ribbon::click_at(&mut app, at);
    app.key_press(Key::Enter);
    app.run_steps(8);
    app.key_press(Key::Escape);
    app.run_steps(8);
    let at = screen(&app, P2::new(7.0, 10.5));
    ribbon::click_at(&mut app, at);
    ribbon::run_command(&mut app, "sketch_vertical");
    let at = screen(&app, P2::new(-40.0, -20.0));
    ribbon::click_at(&mut app, at);
    case.run(&mut app, |app| {
        let state = &app.state().state;
        let sketch = state.sketch.as_ref().unwrap();
        assert_eq!(sketch.sketch.status, SolveStatus::FullyDefined);
        assert!(sketch.sketch.constraints.iter().any(|c| matches!(c.kind, ConstraintKind::Vertical { .. })));
        assert!(state.viewport.references.always_show_origin_axes);
        assert!(near_line(app, [11, 13, 17]), "fully defined line on the Y axis is black with axes on");
    });
    case.frame(&mut app, "defined-with-axes");
}

#[test]
fn typing_a_prefix_shows_a_clickable_suggestion_and_tab_completes() {
    let case = ribbon::case("prototype-type-ahead");
    let mut app = ribbon::app(ribbon::plate());
    type_line(&mut app, "fil");
    case.run(&mut app, |app| {
        let input = app.get_by_label("Command line").rect();
        let found = ribbon::buttons_in(app, ribbon::window(app)).into_iter().find(|(label, rect)| {
            label.contains("fillet")
                && label.contains("Fillet")
                && rect.left() >= input.left() - 8.0
                && rect.bottom() <= input.top()
        });
        let (_, suggestion) = found.expect("typing fil offers Fillet above the prompt without Tab");
        assert_eq!(app.state().state.command_line.input, "fil", "suggestions leave the typed text alone");
        ribbon::click_at(app, suggestion.center());
        assert_eq!(app.state().state.command_line.input, "fillet");
        app.key_press(Key::Escape);
        app.run_steps(4);
        type_line(app, "fil");
        app.key_press(Key::Tab);
        app.run_steps(8);
        assert_eq!(app.state().state.command_line.input, "fillet");
    });
    case.frame(&mut app, "completed");
}

#[test]
fn a_fresh_plain_white_view_has_only_origin_axes() {
    let case = ribbon::case("prototype-fresh-view");
    let mut app = ribbon::app(ribbon::blank());
    case.run(&mut app, |app| {
        let viewport = &app.state().state.viewport;
        assert_eq!(viewport.scene, Some(Scene::White));
        assert!(viewport.references.always_show_origin_axes);
        assert!(!viewport.references.show_grid, "grid is off in a fresh view");
        assert!(!viewport.references.show_origin_planes, "origin planes are off in a fresh view");
    });
    case.frame(&mut app, "fresh-white");
}

#[test]
fn the_default_command_bar_pins_smart_dimension() {
    let case = ribbon::case("prototype-default-pins");
    let mut app = ribbon::app(ribbon::blank());
    case.run(&mut app, |app| {
        assert_eq!(
            app.state().state.command_line.settings.tools,
            ["sketch_select", "smart_dimension", "measure", "normal_to", "zoom_fit"]
        );
        let dimension = app.get_by_label("Smart dim on the command bar").rect();
        ribbon::click_at(app, dimension.center());
        assert!(
            app.state()
                .state
                .editor
                .as_ref()
                .is_some_and(|e| matches!(e.params, soliddraft_app::state::EditorParams::PickPlane))
        );
    });
    case.frame(&mut app, "default-pins");
}

#[test]
fn the_offline_assistant_points_to_the_provider_route() {
    let case = ribbon::case("prototype-offline-route");
    let mut app = ribbon::app(ribbon::blank());
    app.state_mut().state.chat = ChatState::offline(soliddraft_assistant::NoProvider::NoKey.to_string());
    type_line(&mut app, "ai");
    app.key_press(Key::Enter);
    app.run_steps(8);
    case.run(&mut app, |app| {
        let text = ribbon::painted(app).into_iter().map(|p| p.text).collect::<Vec<_>>().join(" ");
        assert!(text.contains("Tools > Assistant > Provider"), "offline message gives the current route: {text}");
        assert!(!text.contains("Assistant tab"));
    });
    case.frame(&mut app, "offline-provider-route");
}

#[test]
fn saved_view_and_command_bar_choices_survive_restart() {
    let case = ribbon::case("prototype-saved-choices");
    let path = case.path("config/settings.json");
    let saved = r#"{"view":{"scene":"white","show_grid":true,"show_origin_planes":true,"always_show_origin_axes":false},"command_line":{"tools":["cancel","zoom_fit"]}}"#;
    std::fs::write(&path, saved).unwrap();
    let app = ribbon::app(ribbon::blank());
    let shown = app.state().state.viewport.references;
    assert!(shown.show_grid && shown.show_origin_planes);
    assert!(!shown.always_show_origin_axes);
    assert_eq!(app.state().state.command_line.settings.tools, ["cancel", "zoom_fit"]);
    assert_eq!(std::fs::read_to_string(path).unwrap(), saved);
}
