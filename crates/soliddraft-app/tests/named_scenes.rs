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
            r#"{"frame_layout":"edit_suite","view":{"show_grid":false,"show_origin_planes":false,"always_show_origin_axes":false,"future":"keep"}}"#,
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
            assert!(!app.state().state.viewport.references.always_show_origin_axes, "sample the backdrop without axes");
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
            assert_eq!(saved["view"]["always_show_origin_axes"], false);
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
