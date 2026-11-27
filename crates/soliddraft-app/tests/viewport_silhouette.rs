//! Apparent contours are drawn from retained display meshes, not recomputed during orbit.

#[path = "support/viewport_acceptance.rs"]
mod support;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use egui::vec2;
use image::RgbaImage;
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Command, Document, ExtrudeParams, PlaneKind, ProfileKind};
use soliddraft_app::viewport::render::DisplayMode;
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_kernel_catallax::CatallaxKernel;
use support::{App, Case, launch_for_frames, orbit, pointer_to_chrome, run_command, settle, viewport};

#[derive(Default)]
struct SilhouetteLog(Mutex<Vec<String>>);

impl log::Log for SilhouetteLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target() == "soliddraft_app::viewport::silhouette"
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0.lock().unwrap().push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

impl SilhouetteLog {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

#[test]
fn orbit_reuses_silhouette_inputs_for_curved_parts() {
    static LOG: SilhouetteLog = SilhouetteLog(Mutex::new(Vec::new()));
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Debug);
    for (name, scenario) in [
        ("cylinder", None),
        ("torus", Some((include_str!("../../../ci/scenarios/torus-revolve.json"), 2))),
        ("filleted-loft", Some((include_str!("../../../ci/scenarios/loft-edge-fillet.json"), 4))),
    ] {
        LOG.take();
        let mut case = Case::gpu(&format!("silhouette-{name}"));
        let kernel = Arc::new(CatallaxKernel::with_operation_log());
        let mut doc = Document::new(name, kernel.clone());
        if scenario.is_none() {
            doc.apply(Command::AddExtrude {
                params: ExtrudeParams {
                    profile: ProfileKind::Circle { diameter: 50.0 },
                    plane: PlaneKind::Top,
                    depth: 75.0,
                    ..Default::default()
                },
                via_assistant: false,
            })
            .unwrap();
        }
        let mut app = launch_for_frames(doc, vec2(1080.0, 700.0), 1.0);
        if let Some((source, steps)) = scenario {
            build_fixture(&mut app, source, steps);
        }
        let state = &mut app.state_mut().state;
        state.doc.selection = None;
        state.viewport.references.show_origin_planes = false;
        state.viewport.references.show_grid = false;
        state.viewport.references.show_floor = false;
        state.viewport.camera.orthographic = name == "cylinder";
        run_command(&mut app, "zoom_fit");
        settle(&mut app);
        if name == "filleted-loft" {
            // Zoom Fit uses a 1 mm minimum radius; this fixture is smaller.
            app.state_mut().state.viewport.camera.zoom(3.0, None, 1.0);
            settle(&mut app);
        }
        let start = Instant::now();
        while app.state().state.doc.display_quality().pending() {
            assert!(start.elapsed() < support::render_steps::HANG_GUARD, "{name}: display did not settle");
            app.step();
            std::thread::yield_now();
        }
        pointer_to_chrome(&mut app);
        app.state_mut().state.viewport.display = DisplayMode::ShadedWithEdges;
        app.step();
        case.frame(&mut app, "initial");
        let setup = LOG.take();
        assert!(!setup.is_empty(), "{name}: no curved triangles reached the renderer");
        case.note(format!("initial input log: {setup:?}"));
        let setup = kernel.take_diagnostics().unwrap();
        assert!(!setup["calls"].as_array().unwrap().is_empty(), "{name}: kernel call log was not active");

        orbit(&mut app, vec2(68.0, -32.0));
        settle(&mut app);
        pointer_to_chrome(&mut app);
        case.frame(&mut app, "orbited");
        let calls = kernel.take_diagnostics().unwrap();
        let calls: Vec<_> = calls["calls"].as_array().unwrap().iter().map(|call| &call["operation"]).collect();
        let cpu = LOG.take();
        case.note(format!("orbit calls: kernel={calls:?}, CPU silhouette inputs={cpu:?}"));
        assert!(calls.is_empty(), "{name}: orbit called the kernel: {calls:?}");
        assert!(cpu.is_empty(), "{name}: orbit rebuilt silhouette inputs on the CPU");

        // Remove topological strokes only for this comparison. Both captures still use
        // the same faces, camera and depth buffer path. An edges-only renderer cannot
        // pass, even if its topological boundaries happen to outline these models.
        let doc = &mut app.state_mut().state.doc;
        let body = doc.display_body().unwrap();
        Arc::make_mut(doc.meshes.get_mut(&body).unwrap()).edges.clear();
        doc.mesh_version += 1;
        app.step();
        let contour = case.frame(&mut app, "contours-only");
        app.state_mut().state.viewport.display = DisplayMode::Shaded;
        app.step();
        let shaded = case.frame(&mut app, "shaded");
        let pixels = changed_pixels(&app, &contour, &shaded);
        case.note(format!("pixels changed by the contour pass: {pixels}"));
        assert!(pixels > 40, "{name}: expected visible contours away from the viewport controls, got {pixels} pixels");
    }
}

fn build_fixture(app: &mut App, source: &str, steps: usize) {
    let scenario: serde_json::Value = serde_json::from_str(source).unwrap();
    let caller = Caller::Mcp { client: "viewport-silhouette".into() };
    for step in &scenario["steps"].as_array().unwrap()[..steps] {
        let name = step["tool"].as_str().unwrap();
        let out =
            call_checked(&mut AppTools { state: &mut app.state_mut().state }, name, step["arguments"].clone(), &caller);
        assert!(!out.is_error, "{name}: {}", out.content);
    }
}

fn changed_pixels(app: &App, contour: &RgbaImage, shaded: &RgbaImage) -> usize {
    let rect = viewport(app).shrink(80.0);
    let mut pixels = 0;
    for y in rect.top().ceil() as u32..rect.bottom() as u32 {
        for x in rect.left().ceil() as u32..rect.right() as u32 {
            let a = contour.get_pixel(x, y).0;
            let b = shaded.get_pixel(x, y).0;
            let distance: u16 = (0..3).map(|i| u16::from(a[i].abs_diff(b[i]))).sum();
            // Silhouette quads can straddle the edge of the dark viewport background.
            // Count partially covered samples without treating rounding noise as ink.
            pixels += usize::from(distance > 12);
        }
    }
    pixels
}
