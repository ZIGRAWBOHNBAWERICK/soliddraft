//! Build and reopen Catallax's distance, radial and point grades through both field editors.
//! Requires a renderer.

use egui::{Key, Modifiers, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::{Value, json};
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Document, FeatureKind, FeatureStatus};
use soliddraft_app::state::EditorParams;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use soliddraft_kernel_api::{Grade, GradeFaces, GradeKind};

type App = Harness<'static, SolidDraftApp>;

const NEW_GRADES: [GradeKind; 3] = [GradeKind::Distance, GradeKind::Radial, GradeKind::Point];

#[test]
fn modelling_guide_names_the_grades_the_pinned_editors_offer() {
    use soliddraft_kernel_api::GradedField;

    let guide = include_str!("../../../docs/modelling.md");
    let (_, grades) = guide.split_once("## Graded fields\n").unwrap();
    let grades = grades.split("\n## ").next().unwrap();
    let support_notes = include_str!("../../../docs/needs-catallax.md");
    let kernel = soliddraft_app::startup::kernel();
    for field in [GradedField::LatticeRadius, GradedField::GyroidLevel] {
        for kind in NEW_GRADES {
            if kernel.field_grade(field, kind).is_ok() {
                for doc in [grades, support_notes] {
                    assert!(doc.contains(kind.label()), "{field:?} offers {}", kind.label());
                }
            }
        }
    }
}

fn button(app: &mut App, label: &str) {
    app.get_all_by_role_and_label(egui::accesskit::Role::Button, label)
        .next()
        .unwrap_or_else(|| panic!("missing button {label}"))
        .click();
    app.run_steps(4);
}

/// Unfolds the feature panel's folded sections, where the grade fields are.
fn unfold(app: &mut App) {
    use egui_kittest::kittest::NodeT;
    for _ in 0..16 {
        let Some(label) = app.query_all_by_label_contains("Unfold ").next().and_then(|n| n.accesskit_node().label())
        else {
            break;
        };
        app.get_by_label(&label).scroll_to_me();
        app.run_steps(24);
        app.get_by_label(&label).click();
        app.run_steps(6);
    }
}

fn field(app: &mut App, label: &str, text: &str) {
    if app.query_by_label(label).is_none() {
        unfold(app);
    }
    app.get_by_label(label).scroll_to_me();
    app.run_steps(24);
    app.get_by_label(label).click();
    app.run_steps(2);
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.get_by_label(label).type_text(text);
    app.key_press(Key::Enter);
    app.run_steps(4);
}

fn click_model(app: &mut App, point: glam::Vec3) {
    let screen = {
        let state = &app.state().state;
        state.viewport.camera.project(point, state.viewport.rect).unwrap()
    };
    app.hover_at(screen);
    app.run_steps(4);
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos: screen,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        app.step();
    }
    app.run_steps(4);
}

fn tool(app: &mut App, name: &str, arguments: Value) -> Result<Value, String> {
    let caller = Caller::Mcp { client: "test".into() };
    let out = call_checked(&mut AppTools { state: &mut app.state_mut().state }, name, arguments, &caller);
    if out.is_error { Err(out.content) } else { Ok(serde_json::from_str(&out.content).unwrap()) }
}

fn renderer() -> App {
    let instance = egui_wgpu::wgpu::Instance::default();
    assert!(
        !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty(),
        "grade acceptance requires a renderer"
    );
    let temporary = std::env::temp_dir().join(format!("soliddraft-more-grades-{}", std::process::id()));
    std::fs::create_dir_all(&temporary).unwrap();
    soliddraft_app::file::set_config_dir(Some(temporary.join("config")));
    let mut doc = Some(Document::new("Grades", soliddraft_app::startup::kernel()));
    Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_render_every_step(true)
        .wgpu()
        .build_eframe(move |cc| SolidDraftApp::new(cc, doc.take().unwrap(), Automation::default()))
}

/// A 6 mm square at z = 20, extruded 10 mm: the region the field fills.
fn region(app: &mut App) {
    let shape = json!({"shape":"rectangle", "center":[0,0,20], "width":6.0, "height":6.0});
    tool(app, "sketch", json!({"plane":"top", "offset":20.0, "shapes":[shape]})).unwrap();
    tool(app, "extrude", json!({"sketch":"Sketch1", "depth":10.0, "flip":false})).unwrap();
    app.run_steps(30);
}

fn last(app: &App) -> (FeatureKind, FeatureStatus) {
    let f = app.state().state.doc.features.last().unwrap();
    (f.kind.clone(), f.status.clone())
}

/// Opens the editor for `command` and shows every row of its form.
fn open(app: &mut App, command: &str) {
    button(app, command);
    unfold(app);
}

fn shown(app: &App, label: &str) -> bool {
    app.query_all_by_label(label).next().is_some()
}

fn refusal(app: &App) -> Option<&str> {
    app.state().state.editor.as_ref().and_then(|editor| editor.refusal())
}

fn assert_picked_distance(app: &App) {
    let editor = app.state().state.editor.as_ref().expect("the lattice form is open");
    let EditorParams::Lattice(held) = &editor.params else { panic!("the lattice form") };
    let Some(Grade::Distance(grade)) = &held.radius_mm else { panic!("a distance grade: {:?}", held.radius_mm) };
    assert_eq!((grade.near, grade.far, grade.span), (2.0, 1.0, 5.0));
    let GradeFaces::Picked(faces) = &grade.faces else { panic!("picked faces") };
    assert_eq!(faces.len(), 1, "the clicked face");
}

#[test]
fn catallax_grades_build_and_reopen_from_both_field_editors() {
    let mut app = renderer();
    region(&mut app);
    let dir = std::env::temp_dir().join(format!("soliddraft-more-grades-{}", std::process::id()));
    let source = dir.join("region.dwprt");
    tool(&mut app, "save", json!({"path": source})).unwrap();

    for (command, label, near, far) in [("Lattice", "Beam radius", "2", "1"), ("Gyroid", "Network level", "0", "0.1")] {
        for kind in NEW_GRADES {
            tool(&mut app, "open", json!({"path": source})).unwrap();
            app.run_steps(12);
            assert_eq!(app.state().state.doc.features.len(), 2);
            if command == "Lattice" && kind == GradeKind::Distance {
                app.key_press(Key::Z);
                app.run_steps(30);
            }
            open(&mut app, command);
            if command == "Lattice" {
                field(&mut app, "Cell size value", "100");
            }
            app.get_by_label(&format!("{label} grade")).click();
            app.run_steps(4);
            for offered in GradeKind::ALL {
                assert!(shown(&app, offered.label()), "{command}: {} is offered", offered.label());
            }
            app.get_by_label(kind.label()).click();
            app.run_steps(4);

            let (first, last_value) = if kind == GradeKind::Distance { ("near", "far") } else { ("start", "end") };
            field(&mut app, &format!("{label} {first} value"), near);
            field(&mut app, &format!("{label} {last_value} value"), far);
            if command == "Lattice" && kind == GradeKind::Distance {
                button(&mut app, "Picked faces");
                field(&mut app, "Beam radius span value", "5");
                button(&mut app, "OK");
                assert!(refusal(&app).is_some_and(|why| why.contains("at least one face")), "{:?}", refusal(&app));
                unfold(&mut app);
                click_model(&mut app, glam::Vec3::new(1.0, 1.0, 30.0));
                assert_picked_distance(&app);

                field(&mut app, "Beam radius span value", "1");
                let invalid = app.state().state.editor.as_ref().unwrap().params.clone();
                button(&mut app, "OK");
                assert!(
                    refusal(&app).is_some_and(|why| why.contains("lattice.grade-range-unsupported")),
                    "{:?}",
                    refusal(&app)
                );
                assert_eq!(app.state().state.editor.as_ref().unwrap().params, invalid);
                assert_eq!(app.state().state.doc.features.len(), 2);
                field(&mut app, "Beam radius span value", "5");
                assert_picked_distance(&app);
            }

            let editor = app.state().state.editor.as_ref().unwrap();
            let (expected, grade) = match &editor.params {
                EditorParams::Lattice(l) => (FeatureKind::Lattice(l.clone()), l.radius_mm.as_ref().unwrap()),
                EditorParams::Gyroid(g) => (FeatureKind::Gyroid(g.clone()), &g.level),
                _ => panic!("{command} form"),
            };
            assert_eq!(grade.kind(), kind);
            assert_eq!(grade.endpoints(), [near.parse::<f64>().unwrap(), far.parse::<f64>().unwrap()]);
            button(&mut app, "OK");
            assert!(app.state().state.editor.is_none(), "{command} {kind:?}: {:?}", refusal(&app));
            let built = last(&app);
            assert_eq!(built, (expected, FeatureStatus::Ok), "{command} {kind:?}");

            let path = dir.join(format!("{command}-{}.dwprt", kind.name()));
            tool(&mut app, "save", json!({"path": path})).unwrap();
            tool(&mut app, "open", json!({"path": path})).unwrap();
            app.run_steps(12);
            assert_eq!(last(&app), built, "{command} {kind:?} reopened");
            std::fs::remove_file(path).unwrap();
        }
    }
    std::fs::remove_file(source).unwrap();
}
