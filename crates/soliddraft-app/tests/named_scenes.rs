//! Scene commands, persistence, classic isolation and pixels on both renderer tiers.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, pos2};
use egui_kittest::{Harness, kittest::Queryable};
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::settings;
use soliddraft_app::viewport::look::Tier;
use soliddraft_app::viewport::render::ViewportRenderer;
use soliddraft_app::viewport::scenes::Scene;

fn type_line(app: &mut ribbon::App, text: &str) {
    app.get_by_label("Command line").click();
    app.run_steps(2);
    app.get_by_label("Command line").type_text(text);
    app.run_steps(2);
    app.key_press(Key::Enter);
    app.run_steps(4);
}

fn pixel(app: &ribbon::App, image: &image::RgbaImage, uv: [f32; 2]) -> [u8; 3] {
    let rect = app.state().state.viewport.rect;
    let point = rect.min + egui::vec2(rect.width() * uv[0], rect.height() * uv[1]);
    let ppp = app.ctx.pixels_per_point();
    let [r, g, b, _] = image.get_pixel((point.x * ppp) as u32, (point.y * ppp) as u32).0;
    [r, g, b]
}

fn near(actual: [u8; 3], expected: [u8; 3]) {
    assert!(actual.into_iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 7), "{actual:?} != {expected:?}");
}

#[test]
fn scene_commands_persist_and_render_on_both_tiers() {
    let case = ribbon::case("named-scenes");
    let config = case.path("config").join("settings.json");
    for tier in [Tier::Full, Tier::Basic] {
        std::fs::write(
            &config,
            r#"{"frame_layout":"edit_suite","view":{"show_grid":false,"show_origin_planes":false,"future":"keep"}}"#,
        )
        .unwrap();
        let mut app = Harness::builder()
            .with_size(ribbon::WINDOW)
            .with_pixels_per_point(1.0)
            .renderer(ribbon::render_steps::renderer())
            .build_eframe(move |cc| {
                let app = SolidDraftApp::new(cc, ribbon::blank(), Automation::default());
                let rs = cc.wgpu_render_state.as_ref().unwrap();
                rs.renderer.write().callback_resources.insert(ViewportRenderer::new(
                    &rs.device,
                    &rs.queue,
                    rs.target_format,
                    tier,
                ));
                app
            });
        app.run_steps(10);
        case.run(&mut app, |app| {
            assert_eq!(app.state().state.viewport.scene, Some(Scene::White));
            for scene in Scene::ALL {
                type_line(app, &format!("scene {}", scene.id()));
                assert_eq!(app.state().state.viewport.scene, Some(scene));
                assert_eq!(settings::view().scene, scene);
                let image = app.render().expect("scene renders");
                image.save(case.path(&format!("{tier:?}-{}.png", scene.id()))).unwrap();
                match scene {
                    Scene::White => {
                        near(pixel(app, &image, [0.18, 0.18]), [255; 3]);
                        near(pixel(app, &image, [0.82, 0.82]), [255; 3]);
                    }
                    Scene::Studio => near(pixel(app, &image, [0.18, 0.46]), [221, 226, 233]),
                    Scene::Faded => near(pixel(app, &image, [0.18, 0.52]), [221, 227, 237]),
                    Scene::Spot => {
                        near(pixel(app, &image, [0.5, 0.46]), [255; 3]);
                        let centre = pixel(app, &image, [0.5, 0.46]);
                        let side = pixel(app, &image, [0.95, 0.46]);
                        assert!(centre[0] > side[0] + 10, "spotlight varies across a row");
                    }
                    Scene::Graphite => near(pixel(app, &image, [0.18, 0.55]), [42, 47, 54]),
                    Scene::Beige => near(pixel(app, &image, [0.18, 0.5]), [236, 227, 209]),
                    Scene::Lightbox => {
                        let above = pixel(app, &image, [0.18, 0.575]);
                        let below = pixel(app, &image, [0.18, 0.60]);
                        assert!(above[0] > below[0] + 20, "lightbox has its horizon");
                    }
                    Scene::Night | Scene::Blueprint => {
                        let spacing = if scene == Scene::Night { 32.0 } else { 28.0 };
                        let rect = app.state().state.viewport.rect;
                        let y = 0.3;
                        let line = pixel(app, &image, [spacing * 3.0 / rect.width(), y]);
                        let cell = pixel(app, &image, [spacing * 3.5 / rect.width(), y]);
                        assert!(line[0] > cell[0] + 3, "screen grid is visible: {line:?} {cell:?}");
                    }
                }
            }
            let before = std::fs::read(&config).unwrap();
            type_line(app, "scene missing");
            assert_eq!(app.state().state.viewport.scene, Some(Scene::Lightbox));
            assert_eq!(std::fs::read(&config).unwrap(), before, "invalid names write nothing");
            type_line(app, "scene Plain white");
            assert_eq!(app.state().state.viewport.scene, Some(Scene::White));
            let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
            assert_eq!(saved["view"]["future"], "keep");
            assert_eq!(saved["view"]["show_grid"], false);
            settings::save_default_preset(soliddraft_app::viewport::look::Preset::Fast).unwrap();
            assert_eq!(settings::view().scene, Scene::White);
        });
        drop(app);
        settings::save_scene(Scene::Graphite).unwrap();
        let mut reopened = ribbon::app(ribbon::blank());
        assert_eq!(reopened.state().state.viewport.scene, Some(Scene::Graphite));
        case.frame(&mut reopened, &format!("{tier:?}-reopened"));
    }
}

#[test]
fn classic_ignores_saved_scenes_and_scene_commands() {
    let case = ribbon::case("classic-scenes");
    let config = case.path("config").join("settings.json");
    std::fs::write(
        &config,
        r#"{"frame_layout":"classic","view":{"scene":"graphite","show_grid":false,"show_origin_planes":false}}"#,
    )
    .unwrap();
    let mut app = ribbon::app(ribbon::blank());
    case.run(&mut app, |app| {
        assert_eq!(app.state().state.viewport.scene, None);
        let before = app.render().unwrap();
        type_line(app, "scene white");
        assert_eq!(app.state().state.viewport.scene, None);
        assert_eq!(settings::view().scene, Scene::Graphite);
        app.hover_at(pos2(10.0, 10.0));
        app.run_steps(4);
        let after = app.render().unwrap();
        near(pixel(app, &before, [0.18, 0.3]), pixel(app, &after, [0.18, 0.3]));
    });
}

#[test]
fn bottom_navigation_gallery_and_favourites_work_and_reopen() {
    use soliddraft_app::viewport::render::DisplayMode;
    let case = ribbon::case("bottom-navigation");
    let config = case.path("config").join("settings.json");
    std::fs::write(
        &config,
        r#"{"frame_layout":"edit_suite","view":{"show_grid":false,"show_origin_planes":false,"future":"keep"}}"#,
    )
    .unwrap();
    let mut app = ribbon::app(ribbon::plate());
    let viewport = app.state().state.viewport.rect;
    case.frame(&mut app, "navigation-initial");
    for label in ["Orbit", "Pan", "Zoom", "Zoom to fit", "Display style", "Scene", "Origin axes", "All dimensions"] {
        let rect = ribbon::button_rect(&app, label, ribbon::window(&app));
        assert!(viewport.contains_rect(rect), "{label} stays in the viewport");
        assert!(rect.top() > viewport.bottom() - 60.0, "{label} is on the bottom bar");
    }
    assert!(app.query_all_by_label("Isometric view (I)").next().is_none(), "the top HUD is replaced");
    let undo = app.state().state.doc.undo_depth();
    app.get_by_label("Scene").click();
    app.run_steps(4);
    for scene in &Scene::ALL[..5] {
        assert!(app.query_all_by_label(scene.label()).next().is_some());
    }
    assert!(app.query_all_by_label("Night grid").next().is_none());
    case.frame(&mut app, "five-favourites");
    app.get_by_label("Manage favourites").click();
    app.run_steps(4);
    for scene in Scene::ALL {
        assert!(app.query_all_by_label(scene.label()).next().is_some());
    }
    app.get_by_label("Night grid").click();
    app.run_steps(4);
    app.get_by_label("Done").click();
    app.run_steps(4);
    app.get_by_label("Scene").click();
    app.run_steps(4);
    let preview = ribbon::button_rect(&app, "Night grid", ribbon::window(&app));
    let image = app.render().unwrap();
    image.save(case.path("six-favourites.png")).unwrap();
    let grid = image.get_pixel((preview.left() + 8.0) as u32, (preview.top() + 10.0) as u32).0;
    let cell = image.get_pixel((preview.left() + 12.0) as u32, (preview.top() + 10.0) as u32).0;
    assert!(grid[0] > cell[0], "the Night thumbnail shows its grid");
    app.get_by_label("Night grid").click();
    app.run_steps(4);
    assert_eq!(app.state().state.viewport.scene, Some(Scene::Night));
    assert_eq!(settings::view().scene, Scene::Night);
    assert!(settings::scene_favourites().contains(&Scene::Night));
    app.get_by_label("Display style").click();
    app.run_steps(4);
    app.get_by_label("Wireframe").click();
    app.run_steps(4);
    assert_eq!(app.state().state.viewport.display, DisplayMode::Wireframe);
    app.get_by_label("Origin axes").click();
    app.run_steps(4);
    assert!(settings::view().references.always_show_origin_axes);
    app.get_by_label("All dimensions").click();
    app.run_steps(4);
    assert!(app.state().state.viewport.all_dimensions);
    assert!(!app.state().state.viewport.dimensions.is_empty());
    for label in ["Orbit", "Pan", "Zoom"] {
        let rect = ribbon::button_rect(&app, label, ribbon::window(&app));
        let before = app.state().state.viewport.camera.clone();
        app.hover_at(rect.center());
        app.run_steps(2);
        app.event(egui::Event::PointerButton {
            pos: rect.center(),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        app.step();
        app.hover_at(rect.center() + egui::vec2(18.0, -12.0));
        app.run_steps(3);
        app.event(egui::Event::PointerButton {
            pos: rect.center() + egui::vec2(18.0, -12.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        app.run_steps(3);
        let after = &app.state().state.viewport.camera;
        assert!(
            before.yaw != after.yaw || before.target != after.target || before.distance != after.distance,
            "{label} changes the camera"
        );
    }
    app.state_mut().state.viewport.camera.distance = 10000.0;
    app.get_by_label("Zoom to fit").click();
    app.run_steps(6);
    assert!(app.state().state.viewport.camera.distance < 10000.0);
    assert_eq!(app.state().state.doc.undo_depth(), undo, "navigation does not edit the model");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(saved["view"]["future"], "keep");
    drop(app);
    let mut app = ribbon::app(ribbon::plate());
    assert_eq!(app.state().state.viewport.scene, Some(Scene::Night));
    app.get_by_label("Scene").click();
    app.run_steps(4);
    assert!(app.query_all_by_label("Night grid").next().is_some());
    app.get_by_label("Manage favourites").click();
    app.run_steps(4);
    for scene in settings::scene_favourites() {
        app.get_by_label(scene.label()).click();
        app.run_steps(2);
    }
    app.get_by_label("Done").click();
    app.run_steps(4);
    app.get_by_label("Scene").click();
    app.run_steps(4);
    assert!(app.query_all_by_label("Choose scenes in Manage favourites.").next().is_some());
    assert!(settings::scene_favourites().is_empty(), "an empty choice persists without restoring defaults");
    case.frame(&mut app, "empty-gallery");
}

#[test]
fn sketch_definition_colours_keep_contrast_on_every_scene() {
    use soliddraft_app::{
        document::{Command, PlaneKind, SketchPlane},
        sketch::{ConstraintKind, EntityKind, P2, PointRef, Sketch},
        sketcher::Frame,
        state::AppAction,
    };
    let case = ribbon::case("scene-sketch-contrast");
    std::fs::write(
        case.path("config").join("settings.json"),
        r#"{"frame_layout":"edit_suite","view":{"show_grid":false,"show_origin_planes":false}}"#,
    )
    .unwrap();
    let mut sketch = Sketch::new();
    let blue = sketch.add(EntityKind::Line { a: P2::new(-35.0, 20.0), b: P2::new(35.0, 28.0) }, false);
    let a = P2::new(-35.0, -20.0);
    let b = P2::new(35.0, -28.0);
    let black = sketch.add(EntityKind::Line { a, b }, false);
    sketch.constrain(ConstraintKind::Fix { point: PointRef::new(black, 0), at: a });
    sketch.constrain(ConstraintKind::Fix { point: PointRef::new(black, 1), at: b });
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(25);
    {
        let session = app.state_mut().state.sketch.as_mut().unwrap();
        session.show_relations = false;
        session.show_regions = false;
        session.selection.clear();
    }
    app.get_by_label("All dimensions").click();
    app.run_steps(4);
    assert!(!app.state().state.sketch.as_ref().unwrap().show_dimensions);
    app.get_by_label("All dimensions").click();
    app.run_steps(4);
    assert!(app.state().state.sketch.as_ref().unwrap().show_dimensions);
    app.get_by_label("Scene").click();
    app.run_steps(4);
    app.get_by_label("Manage favourites").click();
    app.run_steps(4);
    app.key_press(Key::Escape);
    app.run_steps(4);
    assert!(app.state().state.sketch.is_some(), "Escape closes scene management before leaving the sketch");
    assert!(app.query_all_by_label("Manage favourites").next().is_none());
    let before = app.state().state.sketch.as_ref().unwrap().sketch.clone();
    assert!(before.free.contains(&blue));
    assert!(!before.free.contains(&black));
    let luminance = |rgb: [u8; 3]| -> f64 {
        rgb.into_iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(c, w)| {
                let v = f64::from(c) / 255.0;
                w * if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            })
            .sum()
    };
    for scene in Scene::ALL {
        app.state_mut().state.push(AppAction::SetScene(scene));
        app.run_steps(4);
        let state = &app.state().state;
        let frame = Frame {
            plane: state.sketch.as_ref().unwrap().plane,
            camera: &state.viewport.camera,
            rect: state.viewport.rect,
        };
        let sample = frame.screen(P2::new(40.0, 35.0)).unwrap();
        let centers = [frame.screen(P2::new(0.0, 24.0)).unwrap(), frame.screen(P2::new(0.0, -24.0)).unwrap()];
        let image = app.render().unwrap();
        image.save(case.path(&format!("sketch-{}.png", scene.id()))).unwrap();
        let p = image.get_pixel(sample.x.round() as u32, sample.y.round() as u32).0;
        let background = [p[0], p[1], p[2]];
        if scene.is_dark() {
            assert_eq!(background, [0xf1, 0xf3, 0xf7], "{} has an opaque light sheet", scene.label());
        }
        for (center, expected) in centers.into_iter().zip([[0x1d, 0x4f, 0xe0], [0x0b, 0x0d, 0x11]]) {
            let pixels = (-3..=3).flat_map(|dy| (-3..=3).map(move |dx| (dx, dy)));
            let ink = pixels
                .map(|(dx, dy)| {
                    let p =
                        image.get_pixel((center.x.round() as i32 + dx) as u32, (center.y.round() as i32 + dy) as u32).0;
                    [p[0], p[1], p[2]]
                })
                .min_by_key(|p| p.iter().zip(expected).map(|(a, b)| u32::from(a.abs_diff(b))).sum::<u32>())
                .unwrap();
            assert!(
                ink.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 6),
                "{}: {ink:?} keeps {expected:?}",
                scene.label()
            );
            let (a, b) = (luminance(ink), luminance(background));
            let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(contrast >= 4.5, "{}: {ink:?} against {background:?} is {contrast}:1", scene.label());
            eprintln!("{} {ink:?}: {contrast:.2}:1", scene.label());
        }
        image.save(case.path(&format!("sketch-{}.png", scene.id()))).unwrap();
        assert_eq!(
            app.state().state.sketch.as_ref().unwrap().sketch,
            before,
            "scene changes do not solve or edit geometry"
        );
    }
}

#[test]
fn navigation_and_gallery_fit_short_and_scaled_windows() {
    let case = ribbon::case("navigation-window-sizes");
    std::fs::write(
        case.path("config").join("settings.json"),
        r#"{"frame_layout":"edit_suite","view":{"show_grid":false,"show_origin_planes":false}}"#,
    )
    .unwrap();
    for (width, height, scale) in [(960.0, 480.0, 1.0), (1440.0, 900.0, 1.5)] {
        let mut app = ribbon::app_at(ribbon::plate(), egui::vec2(width, height), scale);
        let viewport = app.state().state.viewport.rect;
        for label in ["Orbit", "Pan", "Zoom", "Zoom to fit", "Display style", "Scene", "Origin axes", "All dimensions"]
        {
            let rect = ribbon::button_rect(&app, label, ribbon::window(&app));
            assert!(viewport.contains_rect(rect), "{width} at {scale}: {label} stays inside the viewport");
        }
        app.get_by_label("Scene").click();
        app.run_steps(4);
        let manage = ribbon::button_rect(&app, "Manage favourites", ribbon::window(&app));
        assert!(ribbon::window(&app).contains_rect(manage));
        case.frame(&mut app, &format!("gallery-{width}-{scale}"));
        app.key_press(Key::Escape);
        app.run_steps(4);
        assert!(app.query_all_by_label("Manage favourites").next().is_none());
    }
}
