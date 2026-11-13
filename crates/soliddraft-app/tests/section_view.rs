//! Section inspection through Inspect and navigation on the real application.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{accesskit::Role, pos2};
use egui_kittest::kittest::Queryable;
use soliddraft_app::{document::DisplayMesh, section::SectionPlane};

fn separated_bodies() -> soliddraft_app::document::Document {
    use soliddraft_app::document::{Command, Document, ExtrudeParams, PlaneKind, SketchPlane};
    use soliddraft_sketch::{P2, Sketch};

    let mut doc = Document::new("Section both bodies", soliddraft_app::kernel());
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
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        click(app, "Inspect tab");
        click(app, "Section");
        ready(app);
        app.get_by_label("Face plane").click();
        app.run_steps(3);
        app.get_by_label(&label).click();
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
    app.event(egui::Event::PointerButton {
        pos: pos2(from.x + dx, from.y),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Default::default(),
    });
    app.run_steps(4);
    ready(app);
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
