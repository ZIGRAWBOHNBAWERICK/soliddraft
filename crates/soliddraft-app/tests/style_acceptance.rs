//! Layout and control checks against the approved Edit Suite prototype.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Rect, pos2, vec2};
use egui_kittest::kittest::Queryable;
use soliddraft_app::theme;

fn title() -> Rect {
    Rect::from_min_max(pos2(0.0, 0.0), pos2(1440.0, theme::MENU_HEIGHT))
}

fn click(app: &mut ribbon::App, label: &str) {
    let rect = app.get_by_label(label).rect();
    ribbon::click_at(app, rect.center());
    app.run_steps(8);
}

#[test]
fn open_sketch_replaces_workspaces_with_its_name_and_plane() {
    let case = ribbon::case("style-sketch-banner");
    let mut app = ribbon::app(ribbon::plate());
    click(&mut app, "Start sketch");
    click(&mut app, "Top plane");
    assert!(app.state().state.in_sketch());
    case.frame(&mut app, "sketch");
    case.run(&mut app, |app| {
        let text = ribbon::painted_in(app, title());
        assert!(
            text.iter().any(|t| t.text.contains("on the Top plane")),
            "the title identifies the open sketch and plane: {text:?}"
        );
        for tab in ["Solid tab", "Sketch tab", "Assembly tab", "Inspect tab", "View tab"] {
            assert!(app.query_all_by_label(tab).next().is_none(), "{tab} is replaced by the sketch banner");
        }
        click(app, "Close sketch");
        assert!(app.query_all_by_label("Solid tab").next().is_some());
    });
}

#[test]
fn document_tabs_live_in_the_title_and_switch_documents() {
    use soliddraft_app::{assembly::AssemblyAction, drawing::DrawingAction, state::AppAction};
    let case = ribbon::case("style-document-tabs");
    let mut app = ribbon::app(ribbon::plate());
    case.frame(&mut app, "part");
    case.run(&mut app, |app| {
        let part = ribbon::control(app, "Ribbon acceptance document tab", title());
        let name = ribbon::painted_in(app, part.rect).into_iter().find(|t| t.text == "Ribbon acceptance").unwrap();
        assert_eq!(name.rows, ["Ribbon acceptance"], "a normal document title fits without shortening");
        assert!(part.rect.left() > app.get_by_label("View tab").rect().right());
        assert_eq!(app.state().state.viewport.rect.top(), soliddraft_app::options_bar::HEIGHT + theme::MENU_HEIGHT);
        app.state_mut().state.push(AppAction::Drawing(DrawingAction::MakeDrawing));
        app.run_steps(8);
        app.state_mut().state.push(AppAction::Drawing(DrawingAction::CreateDrawing));
        app.run_steps(16);
        assert!(app.state().state.drawing.is_active());
        click(app, "Ribbon acceptance document tab");
        assert!(!app.state().state.drawing.is_active());
        app.state_mut().state.push(AppAction::Assembly(AssemblyAction::New));
        app.run_steps(12);
        assert!(app.state().state.assembly.is_active());
        let tabs = ribbon::buttons_in(app, title())
            .into_iter()
            .filter(|(name, _)| name.ends_with(" document tab"))
            .collect::<Vec<_>>();
        assert_eq!(tabs.len(), 3, "all open documents stay reachable");
        click(app, "Ribbon acceptance document tab");
        assert!(!app.state().state.assembly.is_active());
        let drawing = tabs.iter().find(|(name, _)| name.contains("Drawing")).unwrap().0.clone();
        click(app, &drawing);
        assert!(app.state().state.drawing.is_active());
        case.frame(app, "drawing");
        app.set_size(vec2(960.0, 700.0));
        app.run_steps(8);
        let narrow_tabs = ribbon::buttons_in(app, ribbon::window(app))
            .into_iter()
            .filter(|(name, _)| name.ends_with(" document tab"))
            .collect::<Vec<_>>();
        assert_eq!(narrow_tabs.len(), 3);
        for (_, rect) in narrow_tabs {
            assert!(ribbon::window(app).contains_rect(rect), "document tabs fit the narrow title");
        }
    });
}

#[test]
fn controls_use_plain_icons_unboxed_menus_and_grey_workspaces() {
    let case = ribbon::case("style-controls");
    let mut app = ribbon::app(ribbon::plate());
    app.hover_at(pos2(1400.0, 500.0));
    app.run_steps(8);
    case.frame(&mut app, "controls");
    case.run(&mut app, |app| {
        let solid = app.get_by_label("Solid tab").rect();
        let image = app.render().unwrap();
        assert_eq!(
            image.get_pixel((solid.left() + 3.0) as u32, solid.center().y as u32).0,
            [0x4a, 0x4e, 0x55, 255],
            "the selected workspace is grey"
        );
        let options = Rect::from_min_max(pos2(250.0, theme::MENU_HEIGHT), pos2(1440.0, 80.0));
        for label in ["Isometric", "Fit", "Front", "Top", "Right", "Normal to"] {
            let button = ribbon::control(app, label, options);
            assert!(button.rect.width() <= 30.0, "{label} is an icon control: {:?}", button.rect);
            assert!(!ribbon::painted_in(app, options).iter().any(|t| t.text == label));
        }
        let extrude = app.get_by_label("Extrude").rect();
        let icon = Rect::from_min_size(pos2(extrude.left() + 5.0, extrude.center().y - 8.0), vec2(16.0, 16.0));
        let coloured = image
            .enumerate_pixels()
            .filter(|(x, y, p)| {
                icon.contains(pos2(*x as f32, *y as f32))
                    && p.0[..3].iter().max().unwrap() - p.0[..3].iter().min().unwrap() > 20
            })
            .count();
        assert_eq!(coloured, 0, "palette icons use neutral ink");
        for (label, view) in [("Front", "Front"), ("Top", "Top"), ("Right", "Right")] {
            click(app, label);
            assert_eq!(app.state().state.viewport.camera.view_name, view, "the icon runs its view command");
        }
        let file = app.get_by_label("File").rect();
        assert_eq!(
            image.get_pixel((file.left() + 3.0) as u32, (file.top() + 3.0) as u32).0,
            image.get_pixel(400, 2).0,
            "menus have no resting box"
        );
        click(app, "File");
        assert!(app.query_all_by_label_contains("New part").next().is_some());
    });
}
