//! Section inspection through Inspect and navigation on the real application.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{accesskit::Role, pos2};
use egui_kittest::kittest::Queryable;
use glam::Vec3;
use soliddraft_app::{document::DisplayMesh, section::SectionPlane};
use soliddraft_kernel_api::counting::{Counting, Work};
use std::{collections::BTreeMap, sync::Arc};

fn separated_bodies() -> soliddraft_app::document::Document {
    use soliddraft_app::document::{Command, Document, ExtrudeParams, PlaneKind, SketchPlane};
    use soliddraft_sketch::{P2, Sketch};

    let mut doc = Document::new("Section both bodies", soliddraft_app::startup::kernel());
    for (x, depth) in [(-30.0, 10.0), (30.0, 20.0)] {
        if x > 0.0 {
            doc.new_body();
        }
        let id = doc
            .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
            .unwrap()
            .unwrap();
        let mut sketch = Sketch::new();
        sketch.rectangle(P2::new(x, 0.0), 20.0, 10.0).unwrap();
        doc.apply(Command::EditSketch { id, sketch }).unwrap();
        doc.apply(Command::AddExtrude {
            params: ExtrudeParams { sketch: Some(id), depth, ..Default::default() },
            via_assistant: false,
        })
        .unwrap();
    }
    doc
}

#[test]
fn section_keeps_every_body_when_the_active_body_changes() {
    let case = ribbon::case("section-multibody");
    std::fs::write(case.path("config/settings.json"), r#"{"frame_layout":"edit_suite"}"#).unwrap();
    let doc = separated_bodies();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        let before = app.state().state.doc.model().to_bytes().unwrap();
        let meshes = app.state().state.doc.meshes.clone();
        click(app, "Inspect tab");
        click(app, "Section");
        ready(app);
        let section = |app: &ribbon::App| app.state().state.section.mesh(&app.state().state.doc).unwrap();
        assert!((volume(&section(app)) - 4000.0).abs() < 0.1, "both bodies contribute to the section");
        let (lo, hi) = section(app).bounds().unwrap();
        assert!(lo.x < -30.0 && hi.x > 30.0, "the section spans both separated bodies");

        app.state_mut().state.doc.select_body(0);
        app.run_steps(3);
        ready(app);
        assert!((volume(&section(app)) - 4000.0).abs() < 0.1, "selection does not change the section");
        app.state_mut().state.doc.new_body();
        app.run_steps(3);
        ready(app);
        assert!((volume(&section(app)) - 4000.0).abs() < 0.1, "an empty active body does not hide the section");
        assert!(soliddraft_app::commands::get("section").unwrap().is_enabled(&app.state().state));
        drag_offset(app, -20.0);
        let offset = app.state().state.section.offset;
        assert!(offset > 0.0 && offset < 10.0);
        let kept = volume(&section(app));
        assert!((kept - 400.0 * offset).abs() < 0.1, "both bodies are cut at the same plane");
        click(app, "Flip side");
        ready(app);
        assert!((kept + volume(&section(app)) - 6000.0).abs() < 0.1, "both complementary cuts stay visible");
        case.frame(app, "both-bodies-sectioned");
        let body_feature = app.state().state.doc.features.iter().find(|f| !f.is_sketch()).unwrap().id;
        let item = soliddraft_app::document::ViewItem::Feature(body_feature);
        app.state_mut().state.doc.set_view_choice(item, Some(false));
        app.run_steps(4);
        assert!(app.state().state.section.mesh(&app.state().state.doc).is_none(), "hidden solids stay hidden");
        app.state_mut().state.doc.set_view_choice(item, None);
        app.run_steps(3);
        ready(app);
        click(app, "Close section");
        assert_eq!(app.state().state.doc.model().to_bytes().unwrap(), before);
        assert_eq!(app.state().state.doc.meshes, meshes);
    });
}

#[test]
fn a_face_section_keeps_its_plane_when_another_or_empty_body_is_active() {
    let case = ribbon::case("section-face-multibody");
    std::fs::write(case.path("config/settings.json"), r#"{"frame_layout":"edit_suite"}"#).unwrap();
    let mut doc = separated_bodies();
    doc.select_body(0);
    let face = doc.faces.iter().find(|f| f.centroid.x < 0.0 && (f.centroid.z - 10.0).abs() < 1e-5).unwrap();
    let (face, label) = (face.id, face.label.clone());
    let entry = doc.faces.iter().filter(|f| f.label == label).position(|f| f.id == face).unwrap();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        click(app, "Inspect tab");
        click(app, "Section");
        ready(app);
        app.get_by_label("Face plane").click();
        app.run_steps(3);
        app.query_all_by_label(&label).nth(entry).unwrap().click();
        app.run_steps(3);
        ready(app);
        assert_eq!(app.state().state.section.plane, SectionPlane::Face(face));
        let version = app.state().state.doc.mesh_version;
        let mesh = app.state().state.section.mesh(&app.state().state.doc).unwrap();
        app.state_mut().state.doc.select_body(1);
        app.run_steps(3);
        assert_eq!(app.state().state.doc.mesh_version, version);
        assert!(app.state().state.doc.face_plane(face).is_some(), "the selected face still belongs to the scene");
        assert_eq!(app.state().state.section.mesh(&app.state().state.doc).as_deref(), Some(mesh.as_ref()));
        app.state_mut().state.doc.new_body();
        app.run_steps(3);
        assert!(app.state().state.doc.face_plane(face).is_some(), "an empty active body does not lose the face");
        assert_eq!(app.state().state.section.mesh(&app.state().state.doc).as_deref(), Some(mesh.as_ref()));
        case.frame(app, "face-section-with-empty-active-body");
    });
}

fn ready(app: &mut ribbon::App) {
    let start = std::time::Instant::now();
    loop {
        let state = &app.state().state;
        if state.section.mesh(&state.doc).is_some() {
            break;
        }
        assert!(state.section.enabled, "the section stayed open while its mesh was requested");
        assert!(
            app.query_by_label("Could not cut this part on that plane. Try another plane or offset.").is_none(),
            "the requested cut succeeded"
        );
        assert!(
            start.elapsed() < ribbon::render_steps::HANG_GUARD,
            "section did not settle: enabled={} plane={:?} offset={} flip={} display={} proved={}",
            state.section.enabled,
            state.section.plane,
            state.section.offset,
            state.section.flip,
            state.doc.mesh_version,
            state.doc.proof_state().current_proved_display()
        );
        app.step();
    }
    app.run_steps(3);
}

fn volume(mesh: &DisplayMesh) -> f64 {
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
            f64::from(a.dot(b.cross(c))) / 6.0
        })
        .sum::<f64>()
        .abs()
}

fn click(app: &mut ribbon::App, label: &str) {
    app.get_by_role_and_label(Role::Button, label).click();
    app.run_steps(4);
}

fn drag_offset(app: &mut ribbon::App, dx: f32) {
    drag_offset_with(app, dx, |_| ());
    ready(app);
}

fn drag_offset_with<R>(app: &mut ribbon::App, dx: f32, held: impl FnOnce(&mut ribbon::App) -> R) -> R {
    let offset = app.get_by_label("Section offset").rect();
    let from = offset.center();
    app.hover_at(from);
    app.run_steps(2);
    app.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    });
    app.run_steps(2);
    app.hover_at(pos2(from.x + dx, from.y));
    app.run_steps(2);
    let held = held(app);
    app.event(egui::Event::PointerButton {
        pos: pos2(from.x + dx, from.y),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    app.run_steps(4);
    held
}

#[test]
fn inspect_and_navigation_section_drag_flip_face_and_close() {
    let case = ribbon::case("section-view");
    std::fs::write(case.path("config/settings.json"), r#"{"frame_layout":"edit_suite"}"#).unwrap();
    let mut app = ribbon::app(ribbon::plate());
    case.run(&mut app, |app| {
        let before = app.state().state.doc.model().to_bytes().unwrap();
        let body = app.state().state.doc.display_body();
        let meshes = app.state().state.doc.meshes.clone();
        let dirty = app.state().state.doc.dirty;
        let export_before = app.state().state.export_part(soliddraft_exchange::Format::Stl).unwrap().bytes;

        click(app, "Inspect tab");
        click(app, "Section");
        ready(app);
        assert!(app.state().state.section.enabled);
        let mesh = app.state().state.section.mesh(&app.state().state.doc).unwrap();
        assert!((volume(&mesh) - 24_000.0).abs() < 0.1);
        assert_eq!(app.state().state.export_part(soliddraft_exchange::Format::Stl).unwrap().bytes, export_before);
        assert_eq!(app.state().state.doc.model().to_bytes().unwrap(), before);
        case.frame(app, "inspect-half-with-cap");

        // Drag the actual offset control, then wait for the displayed body to catch up.
        drag_offset(app, 20.0);
        let offset = app.state().state.section.offset;
        assert!(offset > 10.0 && offset < 20.0, "the pointer moved the cut plane: {offset}");
        let first = volume(&app.state().state.section.mesh(&app.state().state.doc).unwrap());
        click(app, "Flip side");
        ready(app);
        let second = volume(&app.state().state.section.mesh(&app.state().state.doc).unwrap());
        assert!((first + second - 48_000.0).abs() < 0.1);
        assert!((first - second).abs() > 100.0);

        for (label, plane) in [("YZ", SectionPlane::YZ), ("ZX", SectionPlane::ZX), ("XY", SectionPlane::XY)] {
            app.get_by_label(label).click();
            app.run_steps(3);
            ready(app);
            assert_eq!(app.state().state.section.plane, plane);
            assert!((volume(&app.state().state.section.mesh(&app.state().state.doc).unwrap()) - 24_000.0).abs() < 0.1);
        }
        app.get_by_label("Face plane").click();
        app.run_steps(3);
        let face = app.state().state.doc.faces.iter().find(|f| (f.centroid.z - 20.0).abs() < 1e-5).unwrap();
        let (id, label) = (face.id, face.label.clone());
        app.get_by_label(&label).click();
        app.run_steps(3);
        ready(app);
        assert_eq!(app.state().state.section.plane, SectionPlane::Face(id));
        assert_eq!(app.state().state.section.offset, 0.0);
        assert!(app.state().state.section.mesh(&app.state().state.doc).unwrap().indices.is_empty());
        assert!(app.query_by_label("Nothing of the part is on this side of the plane").is_some());
        assert!(app.state().state.section.enabled);
        case.frame(app, "empty-face-side");
        click(app, "Flip side");
        ready(app);
        assert!((volume(&app.state().state.section.mesh(&app.state().state.doc).unwrap()) - 48_000.0).abs() < 0.1);
        drag_offset(app, -20.0);
        let moved = volume(&app.state().state.section.mesh(&app.state().state.doc).unwrap());
        assert!(moved > 0.0 && moved < 48_000.0, "the face plane still moves after an empty cut");
        click(app, "Close section");
        assert!(!app.state().state.section.enabled);
        assert!(app.state().state.section.mesh(&app.state().state.doc).is_none());

        click(app, "Section view");
        ready(app);
        assert!(app.state().state.section.enabled);
        case.frame(app, "navigation-section");
        click(app, "Section view");
        assert!(!app.state().state.section.enabled);
        let state = &app.state().state;
        assert_eq!(state.doc.model().to_bytes().unwrap(), before);
        assert_eq!(state.doc.display_body(), body);
        assert_eq!(state.doc.meshes, meshes);
        assert_eq!(state.doc.dirty, dirty);
        assert_eq!(state.export_part(soliddraft_exchange::Format::Stl).unwrap().bytes, export_before);
    });
}

#[test]
fn turning_off_the_preview_tint_keeps_the_section_cap_fill() {
    let case = ribbon::case("section-cap-tint");
    let mut app = ribbon::app(ribbon::plate());
    case.run(&mut app, |app| {
        click(app, "Inspect tab");
        click(app, "Section");
        ready(app);
        let view = app.state().state.viewport.rect;
        app.hover_at(pos2(view.center().x, 4.0));
        app.run_steps(4);
        let viewport = |path: std::path::PathBuf| {
            let image = image::open(path).unwrap().to_rgba8();
            image::imageops::crop_imm(
                &image,
                view.left() as u32,
                view.top() as u32,
                view.width() as u32,
                view.height() as u32,
            )
            .to_image()
        };
        let tinted = viewport(case.frame(app, "cap-with-preview-tint"));
        app.state_mut().state.graphics.tint_preview = false;
        app.run_steps(4);
        let plain = viewport(case.frame(app, "cap-without-preview-tint"));
        let changed = tinted.pixels().zip(plain.pixels()).filter(|(a, b)| a != b).count();
        assert_eq!(changed, 0, "the preview tint setting changed {changed} pixels of the section view");
    });
}

fn fixture(source: &str, steps: usize, kernel: Arc<Counting>) -> soliddraft_app::document::Document {
    use soliddraft_app::{app_tools::AppTools, document::Document, state::AppState};
    use soliddraft_assistant::{Caller, tools::call_checked};

    let scenario: serde_json::Value = serde_json::from_str(source).unwrap();
    let name = scenario["name"].as_str().unwrap();
    let mut state = AppState::new(Document::new(name, kernel));
    let caller = Caller::Mcp { client: "section acceptance".into() };
    for step in scenario["steps"].as_array().unwrap().iter().take(steps) {
        let tool = step["tool"].as_str().unwrap();
        let out = call_checked(&mut AppTools { state: &mut state }, tool, step["arguments"].clone(), &caller);
        assert!(!out.is_error, "{name}: {tool}: {}", out.content);
    }
    state.doc
}

type Segment = [Vec3; 2];

fn mesh_section(mesh: &DisplayMesh, normal: Vec3, offset: f32) -> Vec<Segment> {
    // Intersect the certified mesh's triangles independently of the stencil pass.
    mesh.indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|triangle| {
            let points = triangle.map(|v| mesh.positions[v as usize]);
            let signed = points.map(|p| p.dot(normal) - offset);
            let mut crossing = [Vec3::ZERO; 2];
            let mut n = 0;
            for a in 0..3 {
                let b = (a + 1) % 3;
                if (signed[a] > 0.0) != (signed[b] > 0.0) {
                    crossing[n] = points[a].lerp(points[b], signed[a] / (signed[a] - signed[b]));
                    n += 1;
                }
            }
            (n == 2 && crossing[0] != crossing[1]).then_some(crossing)
        })
        .collect()
}

fn kernel_cap_boundary(mesh: &DisplayMesh) -> Vec<Segment> {
    let mut edges = BTreeMap::new();
    for t in mesh.indices.as_chunks::<3>().0 {
        let ids = t.map(|i| i as usize);
        if !ids.iter().all(|&i| mesh.face_features[i] == soliddraft_app::document::PREVIEW_MADE) {
            continue;
        }
        for i in 0..3 {
            let line = [mesh.positions[ids[i]], mesh.positions[ids[(i + 1) % 3]]];
            let mut key = line.map(|p| p.to_array().map(|v| if v == 0.0 { 0 } else { v.to_bits() }));
            key.sort_unstable();
            if edges.remove(&key).is_none() {
                edges.insert(key, line);
            }
        }
    }
    edges.into_values().collect()
}

fn outline_error(a: &[Segment], b: &[Segment]) -> f32 {
    a.iter()
        .flat_map(|&[p, q]| [p, (p + q) * 0.5, q])
        .map(|p| {
            b.iter()
                .map(|&[a, b]| {
                    let d = b - a;
                    let t = if d == Vec3::ZERO { 0.0 } else { ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0) };
                    p.distance_squared(a + d * t)
                })
                .fold(f32::INFINITY, f32::min)
        })
        .fold(0.0, f32::max)
        .sqrt()
}

fn assert_drawn_cap(app: &mut ribbon::App, case: &ribbon::Case, frame: &str, outline: &[Segment]) -> egui::Pos2 {
    let view = &app.state().state.viewport;
    let rect = view.rect;
    let segments: Vec<_> = outline.iter().map(|s| s.map(|p| view.camera.project(p, rect).unwrap())).collect();
    let vertices = || segments.iter().flatten();
    let min = vertices().fold(rect.max, |a, b| pos2(a.x.min(b.x), a.y.min(b.y)));
    let max = vertices().fold(rect.min, |a, b| pos2(a.x.max(b.x), a.y.max(b.y)));
    let image = image::open(case.frame(app, frame)).unwrap().to_rgba8();
    let x0 = (min.x.floor() - 6.0).max(rect.left()) as u32;
    let x1 = (max.x.ceil() + 6.0).min(rect.right()) as u32;
    let y0 = (min.y.floor() - 6.0).max(rect.top()) as u32;
    let y1 = (max.y.ceil() + 6.0).min(rect.bottom()) as u32;
    let mut mask = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
    for y in y0..y1 {
        let sy = y as f32 + 0.5;
        let mut xs: Vec<_> = segments
            .iter()
            .filter_map(|&[a, b]| ((a.y > sy) != (b.y > sy)).then(|| a.x + (b.x - a.x) * (sy - a.y) / (b.y - a.y)))
            .collect();
        xs.sort_unstable_by(f32::total_cmp);
        assert_eq!(xs.len() % 2, 0, "{frame}: the section has an open contour at y={sy}");
        let mut next = 0;
        for x in x0..x1 {
            while next < xs.len() && xs[next] < x as f32 + 0.5 {
                next += 1;
            }
            mask.push(next % 2 != 0);
        }
    }
    let at = |x: u32, y: u32| mask[((y - y0) * (x1 - x0) + x - x0) as usize];
    let mut inside = 0;
    let mut outside = 0;
    let mut bad = 0;
    let mut cap_point = None;
    let mut first_mismatch = None;
    for y in y0 + 2..y1 - 2 {
        for x in x0 + 2..x1 - 2 {
            let expected = at(x, y);
            // Ignore the two-pixel rasterization band around an outline.
            if !(y - 2..=y + 2).all(|j| (x - 2..=x + 2).all(|i| at(i, j) == expected)) {
                continue;
            }
            let [r, g, b, _] = image.get_pixel(x, y).0;
            let amber = r.saturating_sub(g) > 12 && g.saturating_sub(b) > 20;
            inside += usize::from(expected);
            if expected {
                cap_point.get_or_insert(pos2(x as f32 + 0.5, y as f32 + 0.5));
            }
            outside += usize::from(!expected);
            if amber != expected {
                first_mismatch.get_or_insert((x, y, expected, [r, g, b]));
                bad += 1;
            }
        }
    }
    assert!(inside > 1000 && outside > 100, "{frame}: exercised {inside} inside and {outside} outside pixels");
    assert_eq!(bad, 0, "{frame}: {bad} cap pixels differ away from the outline; first: {first_mismatch:?}");
    cap_point.unwrap()
}

#[test]
fn drag_stencils_closed_caps_on_p1() {
    check_drag("p1", include_str!("../../../ci/scenarios/canonical-p1.json"), 34, SectionPlane::XY, Vec3::Z, 2.5, None);
}

#[test]
fn drag_stencils_closed_caps_on_p5() {
    check_drag(
        "p5",
        include_str!("../../../ci/scenarios/canonical-p5.json"),
        4,
        SectionPlane::ZX,
        Vec3::Y,
        12.5,
        Some("intersect.spline-plane"),
    );
}

#[test]
fn drag_stencils_closed_caps_on_a_filleted_loft() {
    check_drag(
        "filleted-loft",
        include_str!("../../../ci/scenarios/loft-edge-fillet-small-warp.json"),
        4,
        SectionPlane::XY,
        Vec3::Z,
        1.0,
        Some("intersect.unsupported-pair"),
    );
}

fn check_drag(
    name: &str,
    source: &str,
    steps: usize,
    plane: SectionPlane,
    normal: Vec3,
    target: f32,
    refusal: Option<&str>,
) {
    use soliddraft_app::viewport::{look::Preset, render::DisplayMode};

    let case = ribbon::case(&format!("section-stencil-{name}"));
    let observed = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_operation_log());
    let kernel = Arc::new(Counting::new(observed.clone()));
    let doc = fixture(source, steps, kernel.clone());
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        let state = &mut app.state_mut().state;
        let bounds = state.doc.bounds();
        let (lo, hi) = bounds.unwrap();
        state.doc.selection = None;
        for axis in soliddraft_app::document::AxisKind::ALL {
            state.doc.set_view_choice(soliddraft_app::document::ViewItem::Axis(axis), Some(false));
        }
        for plane in soliddraft_app::document::PlaneKind::ALL {
            state.doc.set_view_choice(soliddraft_app::document::ViewItem::Plane(plane), Some(false));
        }
        state.viewport.references.show_grid = false;
        state.viewport.display = DisplayMode::Shaded;
        state.viewport.preset = Preset::Fast;
        let camera = &mut state.viewport.camera;
        camera.orthographic = true;
        camera.look_from("Section normal", normal);
        camera.fit(bounds);
        camera.tick(10.0);
        // Leave room on the left for the section controls.
        camera.target -= camera.right() * (hi - lo).length() * 0.18;
        state.section.enabled = true;
        state.section.plane = plane;
        let start = -1.0;
        state.section.offset = f64::from(start);
        ribbon::until(app, "display refinement", |app| !app.state().state.doc.display_quality().pending());
        let part_bytes = app.state().state.doc.model().to_bytes().unwrap();
        ready(app);
        kernel.take();
        observed.take_diagnostics().unwrap();

        let source_outline = drag_offset_with(app, (target - start) * 4.0, |app| {
            let state = &app.state().state;
            let offset = state.section.offset;
            assert!((offset - f64::from(target)).abs() < 0.1, "{name}: drag stopped at {offset}, wanted {target}");
            let outline: Vec<_> =
                state.doc.meshes.values().flat_map(|m| mesh_section(m, normal, offset as f32)).collect();
            assert!(!outline.is_empty(), "{name}: the drag crosses the solid");
            assert_drawn_cap(app, &case, "held", &outline);
            assert!(
                !kernel.take().iter().any(|w| matches!(w, Work::Operation(tag) if tag.starts_with("viewport_section"))),
                "{name}: a kernel cut ran before pointer release"
            );
            outline
        });
        if let Some(code) = refusal {
            let notice = "The kernel could not cut this plane. Showing a display-only cap.";
            ribbon::until(app, notice, |app| {
                app.query_by_label(notice).is_some() || app.state().state.section.mesh(&app.state().state.doc).is_some()
            });
            assert_eq!(app.query_all_by_label(notice).count(), 1, "{name}: expected the retained cap notice ({code})");
            let evidence = observed.take_diagnostics().unwrap();
            let calls = evidence["calls"].as_array().unwrap();
            let cuts: Vec<_> = calls.iter().filter(|call| call["operation"] == "boolean").collect();
            let [cut] = cuts.as_slice() else { panic!("{name}: expected one section Boolean: {cuts:?}") };
            assert_eq!(cut["status"], "refused");
            let error = cut["error"].as_str().unwrap();
            assert!(error.contains(&format!("{code}:")), "{name}: expected {code}, got {error}");
            eprintln!("{name}: exact cut refused with {code}; display-only cap retained");
            let point = assert_drawn_cap(app, &case, "refused", &source_outline);
            ribbon::pick_at(app, point);
            let state = &app.state().state;
            let pixel = point - state.viewport.rect.min;
            assert_eq!(state.viewport.pick_sample().texel(pixel.x as u32, pixel.y as u32), Some([0, 0]));
            assert!(state.viewport.hover_point.is_none() && state.doc.selection.is_none());
            assert!(state.section.mesh(&state.doc).is_none());
            assert_eq!(app.query_all_by_label(notice).count(), 1, "{name}: one persistent refusal notice");
        } else {
            ready(app);
            let state = &app.state().state;
            let exact = state.section.mesh(&state.doc).unwrap();
            let exact_outline = kernel_cap_boundary(&exact);
            assert!(!exact_outline.is_empty(), "{name}: the released mesh has a cap");
            let jump =
                outline_error(&source_outline, &exact_outline).max(outline_error(&exact_outline, &source_outline));
            let chord = state.doc.tess_params.chord_height;
            eprintln!("{name}: cap outline jump {jump:.6} mm; chord bound {chord:.6} mm");
            assert!(jump as f64 <= chord, "{name}: cap outline moves {jump} mm (chord bound {chord} mm)");
            assert_drawn_cap(app, &case, "released", &exact_outline);
        }
        assert_eq!(app.state().state.doc.model().to_bytes().unwrap(), part_bytes);
        let log = kernel.take();
        assert_eq!(
            log.iter().filter(|w| matches!(w, Work::Operation(tag) if tag == "viewport_section")).count(),
            1,
            "{name}: release must run one kernel cut without retrying: {log:?}"
        );
    });
}

#[test]
fn live_section_clips_apparent_contours() {
    use soliddraft_app::document::{Command, ExtrudeParams, ProfileKind};
    use soliddraft_app::viewport::{look::Preset, render::DisplayMode};

    let case = ribbon::case("section-clipped-silhouette");
    let mut doc = ribbon::blank();
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { profile: ProfileKind::Circle { diameter: 40.0 }, depth: 30.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        ribbon::until(app, "display refinement", |app| !app.state().state.doc.display_quality().pending());
        let state = &mut app.state_mut().state;
        // Isolate apparent contours from topological strokes.
        for mesh in state.doc.meshes.values_mut() {
            Arc::make_mut(mesh).edges.clear();
        }
        state.doc.mesh_version += 1;
        state.doc.selection = None;
        state.viewport.preset = Preset::Fast;
        state.viewport.camera.orthographic = true;
        state.viewport.camera.look_from("Cylinder side", Vec3::Y);
        state.viewport.camera.fit(state.doc.bounds());
        state.viewport.camera.tick(10.0);

        let count_contour_pixels = |app: &mut ribbon::App, label: &str| {
            let frame = |app: &mut ribbon::App, mode, suffix| {
                app.state_mut().state.viewport.display = mode;
                app.step();
                image::open(case.frame(app, &format!("{label}-{suffix}"))).unwrap().to_rgba8()
            };
            let contours = frame(app, DisplayMode::ShadedWithEdges, "contours");
            let shaded = frame(app, DisplayMode::Shaded, "shaded");
            let area = app.state().state.viewport.rect.shrink(100.0);
            (area.top().ceil() as u32..area.bottom() as u32)
                .map(|y| {
                    (area.left().ceil() as u32..area.right() as u32)
                        .filter(|&x| contours.get_pixel(x, y) != shaded.get_pixel(x, y))
                        .count()
                })
                .sum::<usize>()
        };
        assert!(count_contour_pixels(app, "whole") > 40, "the cylinder shows its apparent contours");

        let state = &mut app.state_mut().state;
        state.section.enabled = true;
        state.section.offset = -5.0;
        ready(app);
        drag_offset_with(app, -20.0, |app| {
            assert!(app.query_by_label("Previewing section").is_some());
            assert_eq!(count_contour_pixels(app, "clipped"), 0, "removed triangles contribute no contour strokes");
        });
        ready(app);
    });
}
