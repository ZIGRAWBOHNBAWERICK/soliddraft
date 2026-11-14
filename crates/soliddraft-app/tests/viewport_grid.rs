//! The modelling grid: minor and major lines on the ground before any body exists, a
//! spacing that follows the zoom and says what it is, and a grid that moves to the open
//! sketch's plane and back, all read from the real renderer's frames.
//!
//! The design is `sdc-d-ux-viewport`, merge B.

#[path = "support/viewport_acceptance.rs"]
mod support;

use egui::{PointerButton, pos2, vec2};
use glam::Vec3;
use soliddraft_app::document::Selection;
use soliddraft_app::sketch::{Entity, EntityKind};
use soliddraft_app::viewport::look::Preset;
use support::*;

const SIZE: egui::Vec2 = vec2(1440.0, 900.0);

/// Up and across, which takes a view facing any of the planes here off its normal. From
/// above, 20 mm of height then moves the picture by 4.5 and 13.5 mm, so a lattice of 10 or
/// 20 mm at z = 0 does not land on one at z = 20.
const OBLIQUE: egui::Vec2 = vec2(40.0, -75.0);

/// Top view, orthographic, through the real controls.
fn to_top_orthographic(app: &mut App) {
    click_button(app, "Top view (T)");
    click_menu(app, "View", "Orthographic");
    assert_eq!(app.state().state.viewport.camera.view_name, "Top");
    assert!(app.state().state.viewport.camera.orthographic);
}

/// Points per millimetre on the ground, from the camera's own projection of two world points.
fn points_per_mm(app: &App) -> f32 {
    let state = &app.state().state;
    let (camera, rect) = (&state.viewport.camera, state.viewport.rect);
    let a = camera.project(Vec3::ZERO, rect).unwrap();
    let b = camera.project(Vec3::X, rect).unwrap();
    (b - a).length()
}

fn lattice_on_ground(step: f32) -> Lattice {
    Lattice { origin: Vec3::ZERO, u: Vec3::X, v: Vec3::Y, step }
}

/// The open sketch's plane as a lattice, from the app's own sketch session.
fn sketch_lattice(app: &App, step: f32) -> Lattice {
    let plane = app.state().state.sketch.as_ref().expect("a sketch is open").plane;
    let v = |p: soliddraft_kernel_api::Vec3| Vec3::new(p.x as f32, p.y as f32, p.z as f32);
    Lattice { origin: v(plane.origin), u: v(plane.x_dir), v: v(plane.y_dir()), step }
}

/// Orbits with the selected preset's gesture until the view is well off the plane's normal.
fn orbit_off_plane(app: &mut App, normal: Vec3) {
    orbit(app, OBLIQUE);
    let facing = app.state().state.viewport.camera.direction().dot(normal).abs();
    assert!((0.2..0.95).contains(&facing), "the view is oblique to {normal}: {facing}");
}

#[test]
fn empty_part_has_minor_and_major_ground_grid() {
    let mut case = Case::gpu("empty_part_grid");
    let mut app = launch(empty());
    let before = model(&app);

    // Before anything else, in the view a new part opens in.
    let (minor, major) = readout(&app).expect("a readout before any sketch");
    case.note(format!("first readout: minor {minor} mm, major {major} mm"));
    let (on, off) = grid_frames(&mut case, &mut app, "first-view");
    assert_lattice(&on, &off, &app, lattice_on_ground(minor), "the first view");

    to_top_orthographic(&mut app);
    let (on, off) = grid_frames(&mut case, &mut app, "top");
    let (x, y, spacing) = assert_top_view_grid(&app, (&on, &off), "Top orthographic");
    case.note(format!(
        "top: readout {spacing:?}, minors {} and {} points, contrast {}/{} and {}/{}, {} and {} majors",
        x.minor,
        y.minor,
        x.minor_contrast,
        x.major_contrast,
        y.minor_contrast,
        y.major_contrast,
        x.majors.len(),
        y.majors.len(),
    ));
    assert!(x.majors.len() >= 2 && y.majors.len() >= 2, "two major lines in each family at the fitted scale");
    assert_eq!(model(&app), before, "the grid does not touch the model");
}

#[test]
fn the_grid_shows_in_every_look() {
    for (name, look, preset) in [
        ("fast", "fast", Preset::Fast),
        ("studio", "studio", Preset::Studio),
        ("realistic", "realistic", Preset::Realistic),
    ] {
        let mut case = Case::gpu(&format!("grid_in_look_{name}"));
        seed_gpu_settings(&case, &format!(r#"{{"view":{{"preset":"{look}"}}}}"#));
        let mut app = launch_for_frames(empty(), SIZE, 1.0);
        assert_eq!(app.state().state.viewport.preset, preset);
        let (minor, _) = readout(&app).expect("a readout");
        let (on, off) = grid_frames(&mut case, &mut app, "first-view");
        assert_lattice(&on, &off, &app, lattice_on_ground(minor), name);
    }
}

#[test]
fn grid_visibility_is_a_persistent_display_preference() {
    let mut case = Case::gpu("grid_preference");
    seed_gpu_settings(
        &case,
        r#"{"view":{"preset":"fast","show_origin_planes":false,"always_show_origin_axes":false},"navigation":{"preset_id":"onshape","future":"keep"}}"#,
    );
    let navigation = case.settings_json()["navigation"].clone();
    let others = |case: &Case| {
        let saved = case.settings_json();
        assert_eq!(saved["navigation"], navigation, "the navigation section stays");
        assert_eq!(saved["view"]["show_origin_planes"], false, "Show origin planes stays");
        assert_eq!(saved["view"]["always_show_origin_axes"], false, "Always show origin axes stays");
        assert_eq!(saved["view"]["preset"], "fast", "the look stays");
    };

    let mut app = launch_for_frames(empty(), SIZE, 1.0);
    let before = model(&app);
    assert_eq!(app.state().state.viewport.preset, Preset::Fast, "the look with no shadow ground");
    to_top_orthographic(&mut app);
    let view = camera(&app);
    open_grid_and_planes(&mut app);
    assert!(checked(&app, SHOW_GRID), "checked on a file that never set it");
    close_preferences(&mut app);
    assert!(readout(&app).is_some(), "a readout with the grid on");
    let on = case.frame(&mut app, "checked");

    // Off: no readout. Both families are only in the frame with the grid on, so neither
    // is left in the frame without it.
    set_preference(&mut app, SHOW_GRID, false);
    assert_eq!(readout(&app), None, "no readout with the grid off");
    let off = case.frame(&mut app, "unchecked");
    others(&case);
    set_preference(&mut app, SHOW_GRID, true);
    assert!(readout(&app).is_some(), "the readout is back");
    let (x, y, _) = assert_top_view_grid(&app, (&on, &off), "the grid on and off");
    case.note(format!("{} and {} lines came and went", x.lines.len(), y.lines.len()));
    let again = case.frame(&mut app, "rechecked");
    assert_top_view_grid(&app, (&again, &off), "the grid checked again");
    assert_eq!((model(&app), camera(&app)), (before, view), "the choice touches neither the model nor the view");
    others(&case);
    drop(app);

    // A restart with it on keeps it on.
    let mut app = launch_for_frames(empty(), SIZE, 1.0);
    to_top_orthographic(&mut app);
    open_grid_and_planes(&mut app);
    assert!(checked(&app, SHOW_GRID), "still checked after a restart");
    close_preferences(&mut app);
    assert!(readout(&app).is_some(), "the readout after a restart");
    set_preference(&mut app, SHOW_GRID, false);
    drop(app);

    // A restart with it off keeps it off.
    let mut app = launch_for_frames(empty(), SIZE, 1.0);
    to_top_orthographic(&mut app);
    open_grid_and_planes(&mut app);
    assert!(!checked(&app, SHOW_GRID), "still unchecked after a restart");
    close_preferences(&mut app);
    assert_eq!(readout(&app), None, "no readout after a restart with the grid off");
    let off = case.frame(&mut app, "unchecked-restarted");
    set_preference(&mut app, SHOW_GRID, true);
    let on = case.frame(&mut app, "checked-restarted");
    assert_top_view_grid(&app, (&on, &off), "after a restart with the grid off");
    others(&case);
}

fn assert_zoom_grid(
    case: &mut Case,
    app: &App,
    frames: (&image::RgbaImage, &image::RgbaImage),
    when: &str,
) -> (Family, Family, (f32, f32)) {
    let (x, y, spacing) = assert_top_view_grid(app, frames, when);
    case.note(format!("{when}: readout {spacing:?}, measured {} and {} points", x.minor, y.minor));
    (x, y, spacing)
}

/// The zoom sweep, with the numbers the design holds it to at any size and scale.
fn sweep(case_name: &str, size: egui::Vec2, ppp: f32) {
    let mut case = Case::gpu(case_name);
    seed_gpu_settings(&case, r#"{"view":{}}"#);
    let mut app = launch_for_frames(empty(), size, ppp);
    let before = model(&app);
    to_top_orthographic(&mut app);

    let start_scale = points_per_mm(&app);
    let start = readout(&app).expect("a readout");
    let (on, off) = grid_frames(&mut case, &mut app, "start");
    let (start_family, _, _) = assert_zoom_grid(&mut case, &app, (&on, &off), "the start");
    let centre = viewport(&app).center();
    let phase = |app: &App, family: &Family| {
        family.lines.iter().map(|l| (l.at - centre.x).abs()).fold(f32::MAX, f32::min) * points_per_mm(app).signum()
    };
    let start_phase = phase(&app, &start_family);

    for (direction, label) in [(1.0_f32, "in"), (-1.0, "out")] {
        let mut seen = vec![start.0];
        let mut wheeled = 0.0;
        let mut sample = 0;
        loop {
            let scale = points_per_mm(&app);
            wheel(&mut app, direction * 300.0);
            wheeled += direction * 300.0;
            sample += 1;
            let moved = points_per_mm(&app);
            let (on, off) = grid_frames(&mut case, &mut app, &format!("zoom-{label}-{sample}"));
            let when = format!("zoom {label} {sample}, {moved} points per mm");
            let (x, y, spacing) = assert_zoom_grid(&mut case, &app, (&on, &off), &when);
            for family in [&x, &y] {
                assert!((8.0..=80.0).contains(&family.minor), "{when}: minor {} points", family.minor);
            }
            case.note(format!("{when}: readout {spacing:?}, measured {} points", x.minor));
            if !seen.contains(&spacing.0) {
                seen.push(spacing.0);
            }
            let span = (moved / start_scale).max(start_scale / moved);
            if span >= 100.0 || moved == scale {
                break;
            }
        }
        assert!(seen.len() >= 2, "zooming {label} must change the spacing, saw {seen:?}");

        // Back the same way: the same scale, the same spacing, the same phase.
        wheel(&mut app, -wheeled);
        let back = points_per_mm(&app);
        assert!((back / start_scale - 1.0).abs() < 0.03, "back at {back}, started at {start_scale}");
        assert_eq!(readout(&app), Some(start), "the spacing after zooming {label} and back");
        let (on, off) = grid_frames(&mut case, &mut app, &format!("back-from-{label}"));
        let (family, _, _) = assert_zoom_grid(&mut case, &app, (&on, &off), "back at the start");
        assert!((phase(&app, &family) - start_phase).abs() <= 2.0, "the phase after zooming {label} and back");
    }
    assert_eq!(model(&app), before, "zooming does not touch the model");
}

#[test]
fn grid_adapts_to_zoom_and_reports_the_rendered_spacing() {
    sweep("grid_zoom", SIZE, 1.0);
}

#[test]
fn grid_density_is_in_points_at_two_pixels_per_point() {
    sweep("grid_zoom_2x", SIZE, 2.0);
}

#[test]
fn grid_density_holds_in_a_small_window() {
    sweep("grid_zoom_small", vec2(960.0, 640.0), 1.0);
}

#[test]
fn compact_window_readout_matches_the_rendered_grid_through_zoom() {
    let mut case = Case::gpu("grid_readout_compact");
    seed_gpu_settings(&case, r#"{"view":{}}"#);
    let mut app = launch_for_frames(empty(), vec2(960.0, 640.0), 1.0);
    to_top_orthographic(&mut app);

    for sample in 0..=8 {
        let when = format!("compact zoom out {sample}");
        let (on, off) = grid_frames(&mut case, &mut app, &when);
        assert_zoom_grid(&mut case, &app, (&on, &off), &when);
        wheel(&mut app, -300.0);
    }
}

#[test]
fn a_two_times_screen_keeps_the_same_world_spacing() {
    let spacing = |name: &str, ppp| {
        let _case = Case::gpu(name);
        let mut app = launch_for_frames(empty(), SIZE, ppp);
        to_top_orthographic(&mut app);
        readout(&app).expect("a readout")
    };
    assert_eq!(spacing("grid_scale_1x", 1.0), spacing("grid_scale_2x", 2.0));
}

#[test]
fn a_spacing_step_is_stable_and_a_pan_keeps_the_spacing() {
    let mut case = Case::gpu("grid_step_and_pan");
    let mut app = launch_for_frames(empty(), SIZE, 1.0);
    to_top_orthographic(&mut app);

    // Walk in 1 percent steps until the spacing changes. 1 percent of zoom is 4 points of wheel.
    let mut previous = readout(&app).expect("a readout");
    let mut steps = 0;
    let (below, above) = loop {
        wheel(&mut app, 4.0);
        let now = readout(&app).expect("a readout");
        steps += 1;
        assert!(steps < 200, "no spacing step in a factor of 7");
        if now != previous {
            break (previous, now);
        }
        previous = now;
    };
    case.note(format!("spacing step after {steps} steps: {below:?} to {above:?}"));

    // At rest on either side of it, the spacing holds with no repaint from a document edit.
    let rest = |app: &mut App, expected, side: &str| {
        for frame in 0..30 {
            app.step();
            assert_eq!(readout(app), Some(expected), "{side}, frame {frame}");
        }
    };
    rest(&mut app, above, "after the step");
    wheel(&mut app, -4.0);
    rest(&mut app, below, "back before the step");
    wheel(&mut app, 4.0);
    rest(&mut app, above, "over the step again");
    let (on, off) = grid_frames(&mut case, &mut app, "over-the-step");
    assert_top_view_grid(&app, (&on, &off), "over the step");

    // A pan moves the lines with the world and leaves their spacing alone.
    let target = app.state().state.viewport.camera.target;
    let centre = viewport(&app).center();
    drag(&mut app, PointerButton::Middle, centre, centre + vec2(73.0, 41.0));
    assert_ne!(app.state().state.viewport.camera.target, target, "the drag panned");
    assert_eq!(readout(&app), Some(above), "a pan keeps the spacing");
    let (on, off) = grid_frames(&mut case, &mut app, "panned");
    assert_top_view_grid(&app, (&on, &off), "after the pan");
}

#[test]
fn grid_follows_each_reference_sketch_and_restores_ground() {
    let mut case = Case::gpu("grid_on_sketches");
    // The dark scene has an opaque sketch sheet; sample the grid over a light scene.
    case.seed_settings(r##"{"view":{"scene":"white","always_show_origin_axes":false,"grid":{"color":"#30343a"}},"navigation":{"preset_id":"fusion"}}"##);
    let mut app = launch(sketches());

    // Each with the normal the plane gives it, and one offset from its reference.
    let rows =
        [("Sketch1", Vec3::Y, 0.0), ("Sketch2", Vec3::Z, 0.0), ("Sketch3", Vec3::X, 0.0), ("Sketch4", Vec3::Z, 25.0)];
    for (row, normal, offset) in rows {
        double_click_label(&mut app, row);
        let open = app.state().state.sketch.as_ref().unwrap_or_else(|| panic!("{row} opened"));
        let plane = open.plane;
        let actual = Vec3::new(plane.normal.x as f32, plane.normal.y as f32, plane.normal.z as f32);
        assert!(actual.cross(normal).length() < 1e-4, "{row}: normal {actual}");
        let along = Vec3::new(plane.origin.x as f32, plane.origin.y as f32, plane.origin.z as f32).dot(normal);
        assert!((along.abs() - offset).abs() < 1e-4, "{row}: at {along}");

        let (minor, _) = readout(&app).expect("a readout in the sketch");
        let (on, off) = grid_frames(&mut case, &mut app, &format!("{row}-normal"));
        assert_lattice(&on, &off, &app, sketch_lattice(&app, minor), &format!("{row}, facing the plane"));

        // An oblique view proves the lines lie in the plane: a ground grid would not fit.
        orbit_off_plane(&mut app, normal);
        wheel(&mut app, -500.0);
        let (minor, _) = readout(&app).expect("a readout in the sketch");
        let (on, off) = grid_frames(&mut case, &mut app, &format!("{row}-oblique"));
        assert_lattice(&on, &off, &app, sketch_lattice(&app, minor), &format!("{row}, oblique"));

        click_button(&mut app, "Exit");
        assert!(app.state().state.sketch.is_none(), "{row} closed");
        click_empty(&mut app);
    }

    assert_eq!(app.state().state.doc.selection, None);
    click_button(&mut app, "Isometric view (I)");
    let (minor, _) = readout(&app).expect("a readout back on the ground");
    let (on, off) = grid_frames(&mut case, &mut app, "ground-again");
    assert_lattice(&on, &off, &app, lattice_on_ground(minor), "back on the ground");
    assert_axes_hidden(&on, &app, "back on the ground");
}

#[test]
fn face_sketch_grid_is_on_the_face_not_world_zero() {
    let mut case = Case::gpu("grid_on_a_face");
    // The dark scene has an opaque sketch sheet; sample the grid over a light scene.
    case.seed_settings(r##"{"view":{"scene":"white","always_show_origin_axes":false,"grid":{"color":"#30343a"}},"navigation":{"preset_id":"fusion"}}"##);
    let mut app = launch(box_part());

    let top = {
        let state = &app.state().state;
        state.viewport.camera.project(Vec3::new(0.0, 0.0, 20.0), state.viewport.rect).expect("the top is on screen")
    };
    pick(&mut app, top);
    let selection = app.state().state.doc.selection;
    assert!(matches!(selection, Some(Selection::Face(_))), "the top face is selected: {selection:?}");
    click_button(&mut app, "Sketch");
    let session = app.state().state.sketch.as_ref().expect("a sketch on the face");
    assert!((session.plane.origin.z - 20.0).abs() < 1e-6, "the sketch is at z = 20");

    // Leave the model clear of the sampled grid crossings in the narrower canvas.
    orbit_off_plane(&mut app, Vec3::Z);
    while readout(&app).expect("a readout").0 < 20.0 {
        wheel(&mut app, -300.0);
    }
    let (minor, _) = readout(&app).expect("a readout");
    case.note(format!("oblique view, minor {minor} mm"));
    let (on, off) = grid_frames(&mut case, &mut app, "oblique");
    let on_face = sketch_lattice(&app, minor);
    assert_lattice(&on, &off, &app, on_face, "the face's height");
    let on_ground = Lattice { origin: on_face.origin - Vec3::Z * 20.0, ..on_face };

    // Six crossings clear of the box and of the frame's own axes.
    let crossings: Vec<(f32, f32)> = (2..5).flat_map(|i| [(i as f32, 3.0), (i as f32, -3.0)]).collect();
    let at_face = crossings.iter().filter(|(i, j)| crossing_at(&on, &off, &app, on_face, *i, *j)).count();
    let at_ground = crossings.iter().filter(|(i, j)| crossing_at(&on, &off, &app, on_ground, *i, *j)).count();
    case.note(format!("of {} crossings, {at_face} at z = 20 and {at_ground} at z = 0", crossings.len()));
    assert_eq!(at_face, crossings.len(), "every crossing is at the face's height");
    assert_eq!(at_ground, 0, "no crossing is at z = 0");

    click_button(&mut app, "Exit");
    assert!(app.state().state.sketch.is_none());
    click_empty(&mut app);
    click_button(&mut app, "Isometric view (I)");
    let (minor, _) = readout(&app).expect("a readout");
    let (on, off) = grid_frames(&mut case, &mut app, "ground-again");
    // The box stands on part of the ground, so only the clear part can fit.
    let (lines, _, _) = lattice_fit(&on, &off, &app, lattice_on_ground(minor));
    assert!(lines >= 0.5, "the ground grid is back: {lines}");
}

#[test]
fn grid_off_keeps_sketch_drawing_and_a_line_lands_where_it_was_clicked() {
    let mut case = Case::gpu("grid_and_sketch_drawing");
    let mut app = launch(sketches());
    double_click_label(&mut app, "Sketch2");
    set_display(&mut app, |d| d.show_grid = false);
    assert_eq!(readout(&app), None, "no readout with the grid off");
    let entities = app.state().state.sketch.as_ref().unwrap().sketch.entities.len();

    run_command(&mut app, "sketch_line");
    let rect = viewport(&app);
    let (a, b) = (pos2(rect.center().x + 150.0, rect.center().y + 120.0), rect.center() + vec2(230.0, 170.0));
    let on_plane = |app: &App, p| {
        let state = &app.state().state;
        let (from, along) = state.viewport.camera.ray(p, state.viewport.rect);
        let t = -from.z / along.z;
        from + along * t
    };
    let (want_a, want_b) = (on_plane(&app, a), on_plane(&app, b));
    click(&mut app, a);
    click(&mut app, b);
    right_click(&mut app, b);
    let session = app.state().state.sketch.as_ref().expect("the sketch stays open");
    assert!(session.sketch.entities.len() > entities, "the line was drawn with the grid off");
    let is_line = |e: &&Entity| matches!(e.kind, EntityKind::Line { .. });
    let line = session.sketch.entities.iter().rev().find(is_line).expect("the new line");
    let points = line.points();
    let frame = sketch_lattice(&app, 1.0);
    let found = |want: Vec3| {
        let (u, v) = ((want - frame.origin).dot(frame.u), (want - frame.origin).dot(frame.v));
        let near = points.iter().map(|p| (p.u as f32 - u).hypot(p.v as f32 - v)).fold(f32::MAX, f32::min);
        assert!(near < 0.5, "an endpoint at {want}, nearest {near}: {points:?}");
    };
    found(want_a);
    found(want_b);
    case.note("line endpoints landed on the clicked plane points");
}

/// Strength, colour and spacing are the user's: each is drawn as chosen, and a restart
/// reads it back from the settings.
#[test]
fn the_grid_style_is_drawn_as_chosen_and_kept() {
    let mut case = Case::gpu("grid_style");
    let top = |case: &mut Case, name: &str| {
        let mut app = launch_for_frames(empty(), SIZE, 1.0);
        to_top_orthographic(&mut app);
        let (on, off) = grid_frames(case, &mut app, name);
        let (x, y, spacing) = assert_top_view_grid(&app, (&on, &off), name);
        case.note(format!(
            "{name}: readout {spacing:?}, contrast {}/{} and {}/{}",
            x.minor_contrast, x.major_contrast, y.minor_contrast, y.major_contrast
        ));
        (app, on, off, x, spacing)
    };

    seed_gpu_settings(&case, r#"{"view":{"preset":"fast"}}"#);
    let (_, _, _, light, _) = top(&mut case, "default");
    // The dark theme draws the default in light ink; it reads as a grid there too.
    seed_gpu_settings(&case, r#"{"appearance":{"theme":"dark"},"view":{"preset":"fast"}}"#);
    let (app, _, _, dark, _) = top(&mut case, "default-dark");
    assert!(dark.minor_contrast >= 4.0, "{dark:?}");
    drop(app);
    seed_gpu_settings(&case, r#"{"view":{"preset":"fast","grid":{"strength":1.0}}}"#);
    let (app, _, _, full, _) = top(&mut case, "full-strength");
    assert_eq!(app.state().state.viewport.grid_style.strength, 1.0, "the start read the strength");
    assert!(
        full.major_contrast > light.major_contrast * 2.0 && full.minor_contrast > light.minor_contrast * 2.0,
        "full strength draws darker lines: {light:?} then {full:?}"
    );
    drop(app);

    seed_gpu_settings(&case, r##"{"view":{"preset":"fast","grid":{"strength":1.0,"color":"#d42020"}}}"##);
    let (app, on, off, red, _) = top(&mut case, "red");
    let line = red.majors.first().expect("a major line").at;
    let rect = viewport(&app);
    let point = |y: f32| (line.round() as u32, (rect.top() + y) as u32);
    let (x, y) = point(rect.height() / 3.0);
    let on = on.get_pixel(x, y).0.map(i32::from);
    let off = off.get_pixel(x, y).0.map(i32::from);
    // Red ink darkens green and blue far more than red.
    let (dr, dg, db) = (off[0] - on[0], off[1] - on[1], off[2] - on[2]);
    assert!(dg > 30 && db > 30 && dg > dr * 2 && db > dr * 2, "a red line: {on:?} over {off:?}");
    drop(app);

    seed_gpu_settings(
        &case,
        r#"{"view":{"preset":"fast","grid":{"follow_zoom":false,"fixed_mm":10,"major_every":4}}}"#,
    );
    let (mut app, on, off, _, spacing) = top(&mut case, "fixed-10mm");
    assert_eq!(spacing, (10.0, 40.0), "the readout names the fixed spacing");
    assert_lattice(&on, &off, &app, lattice_on_ground(10.0), "a fixed 10 mm lattice");
    wheel(&mut app, 300.0);
    assert_eq!(readout(&app), Some((10.0, 40.0)), "a fixed spacing stays when zooming");
}

fn seed_gpu_settings(case: &Case, json: &str) {
    let mut settings: serde_json::Value = serde_json::from_str(json).unwrap();
    settings["view"]["scene"] = serde_json::json!("white");
    if settings["view"]["grid"]["color"].is_null() {
        settings["view"]["grid"]["color"] = serde_json::json!("#0c1420");
    }
    settings["view"]["always_show_origin_axes"] = serde_json::json!(false);
    if settings.get("navigation").is_none() {
        settings["navigation"] = serde_json::json!({"preset_id":"fusion"});
    }
    case.seed_settings(&settings.to_string());
}

#[test]
fn automatic_grid_ink_is_readable_on_light_and_dark_scenes() {
    for scene in ["white", "graphite"] {
        let mut case = Case::gpu(&format!("automatic_grid_{scene}"));
        case.seed_settings(
            &serde_json::json!({
                "view": {"scene": scene, "show_origin_planes": false, "always_show_origin_axes": false},
                "navigation": {"preset_id": "fusion"}
            })
            .to_string(),
        );
        let mut app = launch(empty());
        assert!(app.state().state.viewport.grid_style.color.is_none());
        to_top_orthographic(&mut app);
        let (on, off) = grid_frames(&mut case, &mut app, "automatic-ink");
        assert_top_view_grid(&app, (&on, &off), scene);
    }
}
