//! The grid and the floor from the viewport's Scene menu: "Show grid" takes away the
//! modelling grid and the scene's own screen grid, "Show floor" the shadow the part casts
//! on the ground, and both choices outlast a restart. Read from the real renderer's frames.

#[path = "support/viewport_acceptance.rs"]
mod support;

use egui::accesskit::Role;
use egui::{Rect, vec2};
use egui_kittest::kittest::Queryable;
use support::*;

const SIZE: egui::Vec2 = vec2(1280.0, 800.0);

/// The Night grid scene, with the origin planes and axes out of the way of the backdrop.
const SEED: &str = r#"{"view":{"scene":"night","show_origin_planes":false,"always_show_origin_axes":false}}"#;

/// The viewport's own Scene button, not a ribbon control of the same name.
fn scene_button(app: &App) -> Rect {
    let view = viewport(app);
    let found: Vec<Rect> = app
        .get_all_by_role_and_label(Role::Button, "Scene")
        .map(|n| n.rect())
        .filter(|r| view.contains_rect(*r))
        .collect();
    assert_eq!(found.len(), 1, "one Scene button over the view: {found:?}");
    found[0]
}

fn toggle_scene_menu(app: &mut App) {
    let at = scene_button(app).center();
    click(app, at);
}

fn tick(app: &mut App, label: &str) {
    app.get_by_role_and_label(Role::CheckBox, label).hover();
    app.run_steps(4);
    app.get_by_role_and_label(Role::CheckBox, label).click();
    settle(app);
}

/// The brightest and darkest red along a row near the top of the view, left of the view
/// cube. The scene's gradient is vertical, so only a grid makes the row vary.
fn row_spread(image: &image::RgbaImage, app: &App) -> u8 {
    let view = viewport(app);
    let ppp = app.ctx.pixels_per_point();
    let y = ((view.top() + view.height() * 0.06) * ppp) as u32;
    let reds: Vec<u8> = ((view.left() * ppp) as u32 + 8..((view.left() + view.width() * 0.5) * ppp) as u32)
        .map(|x| image.get_pixel(x, y).0[0])
        .collect();
    reds.iter().max().unwrap() - reds.iter().min().unwrap()
}

/// How many pixels of the view differ between two frames by more than a rounding step.
fn changed(a: &image::RgbaImage, b: &image::RgbaImage, app: &App) -> usize {
    let view = viewport(app);
    let ppp = app.ctx.pixels_per_point();
    let mut count = 0;
    for y in (view.top() * ppp) as u32..(view.bottom() * ppp) as u32 {
        for x in (view.left() * ppp) as u32..(view.right() * ppp) as u32 {
            let [p, q] = [a.get_pixel(x, y).0, b.get_pixel(x, y).0];
            if (0..3).any(|c| p[c].abs_diff(q[c]) > 6) {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn the_scene_menu_hides_the_grid_and_the_floor_and_keeps_the_choice() {
    let mut case = Case::gpu("scene-grid-floor");
    case.seed_settings(SEED);
    let mut app = launch_for_frames(box_part(), SIZE, 1.0);
    let references = app.state().state.viewport.references;
    assert!(references.show_grid && references.show_floor, "both show on a file that never set them");

    pointer_to_chrome(&mut app);
    let shown = case.frame(&mut app, "1-grid-and-floor-shown");
    let spread = row_spread(&shown, &app);
    case.note(format!("row spread with the grid shown: {spread}"));
    assert!(spread > 6, "the scene's grid crosses the row: {spread}");

    toggle_scene_menu(&mut app);
    assert!(checked(&app, "Show grid") && checked(&app, "Show floor"), "the menu says both show");
    case.frame(&mut app, "2-scene-menu");

    tick(&mut app, "Show grid");
    assert!(!checked(&app, "Show grid"), "the grid box took the click");
    toggle_scene_menu(&mut app);
    pointer_to_chrome(&mut app);
    let no_grid = case.frame(&mut app, "3-grid-hidden");
    let spread = row_spread(&no_grid, &app);
    assert!(spread <= 3, "no grid of either kind is left on the row: {spread}");

    toggle_scene_menu(&mut app);
    tick(&mut app, "Show floor");
    assert!(!checked(&app, "Show floor"), "the floor box took the click");
    case.frame(&mut app, "4-scene-menu-both-off");
    toggle_scene_menu(&mut app);
    pointer_to_chrome(&mut app);
    let bare = case.frame(&mut app, "5-grid-and-floor-hidden");
    let shadow = changed(&no_grid, &bare, &app);
    case.note(format!("{shadow} pixels of floor shadow went"));
    assert!(shadow > 500, "the floor's shadow went: {shadow} pixels changed");

    let saved = case.settings_json();
    assert_eq!(saved["view"]["show_grid"], false);
    assert_eq!(saved["view"]["show_floor"], false);
    assert_eq!(saved["view"]["scene"], "night", "the scene itself stays");
    drop(app);

    // A restart keeps both hidden, and Preferences agrees with the menu.
    let mut app = launch_for_frames(box_part(), SIZE, 1.0);
    let references = app.state().state.viewport.references;
    assert!(!references.show_grid && !references.show_floor, "both stay hidden after a restart");
    open_grid_and_planes(&mut app);
    assert!(!checked(&app, "Show grid") && !checked(&app, "Show floor"), "Preferences shows the same choices");
    close_preferences(&mut app);
    let restarted = case.frame(&mut app, "6-restarted");
    assert!(row_spread(&restarted, &app) <= 3, "no grid after a restart");

    // The command line brings them back.
    run_command(&mut app, "grid");
    run_command(&mut app, "floor");
    pointer_to_chrome(&mut app);
    let back = case.frame(&mut app, "7-shown-again");
    assert!(row_spread(&back, &app) > 6, "the scene's grid is back");
    assert!(changed(&back, &restarted, &app) > 500, "the grid and the floor are back");
    let saved = case.settings_json();
    assert_eq!((saved["view"]["show_grid"].clone(), saved["view"]["show_floor"].clone()), (true.into(), true.into()));
}
