//! Section inspection through the ribbon and navigation bar on the real application.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{accesskit::Role, pos2};
use egui_kittest::kittest::{NodeT, Queryable};
use soliddraft_app::{document::DisplayMesh, section::SectionPlane};

fn ready(app: &mut ribbon::App) {
    ribbon::until(app, "the section is tessellated", |app| {
        let state = &app.state().state;
        state.section.mesh(&state.doc).is_some()
    });
    app.run_steps(3);
}

fn volume(mesh: &DisplayMesh) -> f64 {
    mesh.indices
        .chunks_exact(3)
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

#[test]
fn inspect_and_navigation_section_drag_flip_face_and_close() {
    let case = ribbon::case("section-view");
    let mut app = ribbon::app(ribbon::plate());
    case.run(&mut app, |app| {
        let before = app.state().state.doc.model().to_bytes().unwrap();
        let body = app.state().state.doc.display_body();
        let meshes = app.state().state.doc.meshes.clone();
        let dirty = app.state().state.doc.dirty;
        let export_before = app.state().state.export_part(soliddraft_exchange::Format::Stl).unwrap().bytes;

        app.get_by_label("Inspect").click();
        app.run_steps(4);
        let section = ribbon::reveal_ribbon_command(app, "section");
        ribbon::click_at(app, section.rect.center());
        ready(app);
        assert!(app.state().state.section.enabled);
        let mesh = app.state().state.section.mesh(&app.state().state.doc).unwrap();
        assert!((volume(&mesh) - 24_000.0).abs() < 0.1);
        case.frame(app, "inspect-half-with-cap");

        // Drag the actual offset control, then wait for the displayed body to catch up.
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
        app.hover_at(pos2(from.x + 20.0, from.y));
        app.run_steps(2);
        app.event(egui::Event::PointerButton {
            pos: pos2(from.x + 20.0, from.y),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        });
        app.run_steps(4);
        ready(app);
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
