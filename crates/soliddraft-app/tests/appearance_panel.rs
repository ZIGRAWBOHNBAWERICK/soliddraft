//! Part colours and transparency, through the properties panel and the real wgpu
//! pipeline: the sample bracket goes blue from a swatch, transparent from the
//! checkbox, more transparent from a slider drag that applies once when the pointer
//! lets go, and with its part see-through an opaque red boss shows through the front
//! wall. Frames are written to `target/exit-test/appearance-*.png` for review.
//!
//! Skips itself when no GPU adapter is available (plain CI runners).

use egui::{Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::document::Selection;
use soliddraft_app::state::AppAction;
use soliddraft_app::viewport::camera::StandardView;
use soliddraft_materials::{Appearance, Srgb};
use soliddraft_model::AppearanceTarget;
use std::path::PathBuf;

fn gpu_available() -> bool {
    let _ = env_logger::try_init();
    let instance = egui_wgpu::wgpu::Instance::default();
    !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty()
}

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/exit-test");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Mean colour of a 5 by 5 pixel patch.
fn colour(image: &image::RgbaImage, at: Pos2) -> [f64; 3] {
    let mut sum = [0.0; 3];
    for y in at.y as u32 - 2..=at.y as u32 + 2 {
        for x in at.x as u32 - 2..=at.x as u32 + 2 {
            let p = image.get_pixel(x, y);
            for c in 0..3 {
                sum[c] += p[c] as f64 / 25.0;
            }
        }
    }
    sum
}

fn pointer(app: &mut Harness<'_, SolidDraftApp>, pos: Pos2, pressed: bool) {
    app.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::default(),
    });
    app.step();
}

#[test]
fn a_part_takes_a_colour_and_turns_see_through() {
    if !gpu_available() {
        return;
    }
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            SolidDraftApp::new(cc, soliddraft_app::command_tools::board_bracket(), Automation::default())
        });
    app.run_steps(10);
    let part = || AppearanceTarget::Part;
    let blue = Srgb::new(21, 101, 192);

    // The Appearance command opens the panel on the body.
    app.state_mut().state.push(AppAction::ShowAppearance);
    app.run_steps(6);
    assert_eq!(app.state().state.doc.selection, Some(Selection::Body));
    assert!(!app.state().state.doc.has_own_appearance(&part()), "the bracket starts in the default grey");

    // A swatch applies at once, as one undo step, keeping the default finish.
    let depth = app.state().state.doc.model().undo_depth();
    app.get_by_label("Blue").click();
    app.run_steps(4);
    let doc = &app.state().state.doc;
    let shown = doc.shown_appearance(None);
    assert_eq!((shown.base_color, shown.opacity, shown.roughness), (blue, 1.0, 0.45));
    assert_eq!(doc.undo_label(), Some("set appearance Blue on the part"));
    assert_eq!(doc.model().undo_depth(), depth + 1);
    assert!(doc.meshes.values().all(|m| m.appearances[0].base_color[3] == 1.0));

    // The checkbox makes it 60% transparent.
    app.get_by_label("Transparent").click();
    app.run_steps(4);
    let doc = &app.state().state.doc;
    let shown = doc.shown_appearance(None);
    assert_eq!((shown.base_color, shown.opacity), (blue, 0.4));
    assert_eq!(doc.undo_label(), Some("set appearance Blue, 60% transparent on the part"));
    assert_eq!(doc.model().undo_depth(), depth + 2);

    // Dragging the slider shows each value without applying it; letting go applies
    // the last one as one step.
    let slider = app.get_by_label("Transparency").rect();
    let from = slider.center();
    pointer(&mut app, from, true);
    for step in 1..=8 {
        app.hover_at(from + vec2(step as f32 * 30.0, 0.0));
        app.step();
    }
    {
        let doc = &app.state().state.doc;
        assert!(doc.appearance_preview().is_some(), "the drag previews");
        assert_eq!(doc.model().undo_depth(), depth + 2, "nothing applied mid-drag");
        assert!(doc.shown_appearance(None).opacity < 0.4, "the preview is more transparent");
    }
    pointer(&mut app, from + vec2(240.0, 0.0), false);
    app.run_steps(4);
    let doc = &app.state().state.doc;
    assert!(doc.appearance_preview().is_none());
    assert_eq!(doc.model().undo_depth(), depth + 3, "one step for the whole drag");
    assert!((doc.shown_appearance(None).opacity - 0.1).abs() < 1e-6, "the slider stops at 90%");

    // Undo walks back through the slider, the checkbox and the swatch.
    app.state_mut().state.doc.undo().unwrap();
    assert_eq!(app.state().state.doc.shown_appearance(None).opacity, 0.4);
    app.state_mut().state.doc.undo().unwrap();
    assert_eq!(app.state().state.doc.shown_appearance(None).opacity, 1.0);
    app.state_mut().state.doc.redo().unwrap();

    // From the front, the front wall hides the boss behind it until the part is
    // see-through; an opaque red boss then shows through the blue wall.
    {
        let doc = &mut app.state_mut().state.doc;
        let boss = doc.features.iter().find(|f| f.name == "Boss-Extrude3").expect("the boss").id;
        let red = Appearance::new("red", "Red", Srgb::new(196, 48, 40));
        let target = AppearanceTarget::Feature { feature: soliddraft_model::FeatureId(boss as u32) };
        doc.set_appearance(target, Some(red)).unwrap();
    }
    app.state_mut().state.push(AppAction::Select(None));
    app.state_mut().state.push(AppAction::SetView(StandardView::Front));
    app.run_steps(60);
    let rect = app.state().state.viewport.rect;
    let behind = app.state().state.viewport.camera.project(glam::Vec3::new(85.0, 43.0, 22.0), rect).expect("in view");
    let image = app.render().expect("render");
    image.save(out_dir().join("appearance-see-through.png")).unwrap();
    let [r, g, _] = colour(&image, behind);
    assert!(r > g + 25.0, "the red boss shows through the transparent wall: {:?}", colour(&image, behind));

    let opaque = Appearance::new("opaque-blue", "Opaque blue", blue);
    app.state_mut().state.doc.set_appearance(part(), Some(opaque)).unwrap();
    app.run_steps(4);
    let image = app.render().expect("render");
    image.save(out_dir().join("appearance-opaque.png")).unwrap();
    let [r, g, _] = colour(&image, behind);
    assert!(r < g, "an opaque wall hides the boss: {:?}", colour(&image, behind));

    // The command opens on a selected face and colours only that face.
    let face = app.state().state.doc.faces[0].id;
    app.state_mut().state.push(AppAction::Select(Some(Selection::Face(face))));
    app.state_mut().state.push(AppAction::ShowAppearance);
    app.run_steps(6);
    assert_eq!(app.state().state.doc.selection, Some(Selection::Face(face)));
    app.get_by_label("Green").click();
    app.run_steps(4);
    let doc = &app.state().state.doc;
    assert_eq!(doc.shown_appearance(Some(face)).base_color, Srgb::new(46, 125, 50));
    assert_eq!(doc.shown_appearance(None).base_color, blue, "the rest of the part stays blue");
    app.get_by_label("Use the part's appearance").click();
    app.run_steps(4);
    assert_eq!(app.state().state.doc.shown_appearance(Some(face)).base_color, blue);
}
