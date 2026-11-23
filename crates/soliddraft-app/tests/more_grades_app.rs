//! Grades beyond a linear one along a part axis, through the window: the Lattice and
//! Gyroid editors offer a distance, radial or point grade only when the kernel builds
//! it, so at the pinned Catallax, which builds none of them, the editors look as they
//! did. A kernel that accepts them shows each one; the form then saves the grade and a
//! face picked in the viewport, and Catallax's refusal reaches the user by its code.
//!
//! Requires a renderer: a run without one fails.

mod gated;

use std::sync::Arc;

use egui::{Key, Modifiers, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use gated::Gated;
use serde_json::{Value, json};
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Command, Document, FeatureKind, FeatureStatus};
use soliddraft_app::state::{AppAction, EditorParams};
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use soliddraft_kernel_api::{Grade, GradeFaces, GradeKind, GradedField, Kernel};

type App = Harness<'static, SolidDraftApp>;

const NEW_GRADES: [&str; 3] = ["Distance from faces", "Radial from an axis", "From a point"];

fn button(app: &mut App, label: &str) {
    app.get_all_by_role_and_label(egui::accesskit::Role::Button, label)
        .next()
        .unwrap_or_else(|| panic!("missing button {label}"))
        .click();
    app.run_steps(4);
}

fn field(app: &mut App, label: &str, text: &str) {
    if app.query_by_label(label).is_none() && !app.state().state.form_bar.expanded {
        app.get_by_label_contains("More (").click();
        app.run_steps(8);
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

fn renderer(kernel: Arc<dyn Kernel>, name: &str) -> App {
    let instance = egui_wgpu::wgpu::Instance::default();
    assert!(
        !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty(),
        "grade acceptance requires a renderer"
    );
    let temporary = std::env::temp_dir().join(format!("soliddraft-more-grades-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&temporary).unwrap();
    soliddraft_app::file::set_config_dir(Some(temporary.join("config")));
    let mut doc = Some(Document::new("Grades", kernel));
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
    // Zoom to fit, so the small region fills the viewport.
    app.key_press(Key::Z);
    app.run_steps(30);
}

fn last(app: &App) -> (FeatureKind, FeatureStatus) {
    let f = app.state().state.doc.features.last().unwrap();
    (f.kind.clone(), f.status.clone())
}

/// Opens the editor for `command` and shows every row of its form.
fn open(app: &mut App, command: &str) {
    button(app, command);
    app.get_by_label_contains("More (").click();
    app.run_steps(8);
}

fn shown(app: &App, label: &str) -> bool {
    app.query_all_by_label(label).next().is_some()
}

#[test]
fn new_grades_stay_hidden_while_the_pinned_catallax_refuses_them() {
    let kernel = soliddraft_app::kernel();
    for (field, code) in
        [(GradedField::LatticeRadius, "lattice.unsupported-grade"), (GradedField::GyroidLevel, "gyroid.unsupported")]
    {
        let refused = kernel.field_grade(field, GradeKind::Distance).unwrap_err().to_string();
        assert!(refused.contains(code), "{refused}");
        for kind in [GradeKind::Radial, GradeKind::Point] {
            assert!(kernel.field_grade(field, kind).is_err(), "{kind:?}");
        }
    }
    let mut app = renderer(kernel, "hidden");
    region(&mut app);
    for command in ["Lattice", "Gyroid"] {
        open(&mut app, command);
        assert!(shown(&app, "Linear along an axis"), "{command}: the linear grade stays");
        for label in NEW_GRADES {
            assert!(!shown(&app, label), "{command}: {label} is hidden while Catallax refuses it");
        }
        button(&mut app, "Cancel");
    }
}

#[test]
fn a_kernel_that_builds_them_shows_each_grade_and_catallax_answers_the_build() {
    let gated = Arc::new(Gated::default());
    gated.rules.lock().unwrap().accept_grades = vec![GradeKind::Distance, GradeKind::Radial, GradeKind::Point];
    let mut app = renderer(gated.clone(), "accepted");
    region(&mut app);
    open(&mut app, "Lattice");
    field(&mut app, "Cell size value", "100");
    // More than two kinds: the grade is a list, each new kind in it.
    app.get_by_label("Beam radius grade").click();
    app.run_steps(4);
    for label in NEW_GRADES {
        assert!(shown(&app, label), "{label} is offered once the kernel builds it");
    }
    app.get_by_label("Distance from faces").click();
    app.run_steps(4);
    button(&mut app, "Picked faces");
    for (name, value) in [("near", "2"), ("far", "1"), ("span", "5")] {
        field(&mut app, &format!("Beam radius {name} value"), value);
    }
    // OK with no face picked keeps the form open and says why.
    button(&mut app, "OK");
    let refusal = app.state().state.editor.as_ref().and_then(|e| e.refusal().map(str::to_owned));
    assert!(refusal.as_deref().is_some_and(|why| why.contains("at least one face")), "{refusal:?}");
    // Fold the sheet away and click the region's top face in the window, as a user picks it.
    app.get_by_label_contains("More (").click();
    app.run_steps(8);
    click_model(&mut app, glam::Vec3::new(1.0, 1.0, 30.0));
    let editor = app.state().state.editor.as_ref().expect("the lattice form is open");
    let EditorParams::Lattice(held) = editor.params.clone() else { panic!("the lattice form") };
    let Some(Grade::Distance(grade)) = &held.radius_mm else { panic!("a distance grade: {:?}", held.radius_mm) };
    assert_eq!((grade.near, grade.far, grade.span), (2.0, 1.0, 5.0));
    let GradeFaces::Picked(faces) = &grade.faces else { panic!("picked faces") };
    assert_eq!(faces.len(), 1, "the clicked face");

    // The kernel offered the grade; Catallax builds it no further. OK keeps the form open
    // with Catallax's refusal by its code, the grade and its face as entered, and the
    // tree gains no failed row.
    button(&mut app, "OK");
    app.run_steps(12);
    let editor = app.state().state.editor.as_ref().expect("the refused form stays open");
    let refusal = editor.refusal().unwrap_or_default().to_owned();
    assert!(refusal.contains("lattice.unsupported-grade"), "{refusal}");
    assert_eq!(editor.params, EditorParams::Lattice(held.clone()));
    assert_eq!(app.state().state.doc.features.len(), 2, "no failed lattice joins the tree");
    button(&mut app, "Cancel");

    // A part that holds the grade, as a later Catallax builds it, saves and reopens with
    // the grade and its face unchanged; here Catallax names its refusal on the feature.
    app.state_mut().state.doc.apply(Command::AddFeature(FeatureKind::Lattice(held.clone()))).unwrap_err();
    app.run_steps(8);
    let path = std::env::temp_dir().join(format!("soliddraft-more-grades-{}.dwprt", std::process::id()));
    tool(&mut app, "save", json!({"path":path})).unwrap();
    tool(&mut app, "open", json!({"path":path})).unwrap();
    app.run_steps(12);
    std::fs::remove_file(&path).unwrap();
    let (reopened, status) = last(&app);
    assert_eq!(reopened, FeatureKind::Lattice(held), "the grade and its face reopen unchanged");
    let FeatureStatus::Error(message) = status else { panic!("{status:?}") };
    assert!(message.contains("lattice.unsupported-grade"), "{message}");

    // Editing the feature shows the grade it holds and the kernel's refusal of it.
    gated.rules.lock().unwrap().accept_grades.clear();
    let id = app.state().state.doc.features.last().unwrap().id;
    app.state_mut().state.push(AppAction::EditFeature(id));
    app.run_steps(8);
    if !app.state().state.form_bar.expanded {
        app.get_by_label_contains("More (").click();
        app.run_steps(8);
    }
    assert!(app.query_by_label_contains("does not build this grade yet").is_some(), "the refusal is shown");
}
