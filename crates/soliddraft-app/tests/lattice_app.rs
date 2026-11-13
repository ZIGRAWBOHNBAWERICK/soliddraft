//! The human lattice route uses the same saved feature as the agent verbs.

use egui::{Key, Modifiers, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Command, Document, ExtrudeParams, FeatureKind, ProfileKind};
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;

#[path = "support/ribbon.rs"]
mod ribbon;

type App = Harness<'static, SolidDraftApp>;

fn button(app: &mut App, label: &str) {
    app.get_all_by_role_and_label(egui::accesskit::Role::Button, label)
        .next()
        .unwrap_or_else(|| panic!("missing button {label}"))
        .click();
    app.run_steps(4);
}

fn field(app: &mut App, label: &str, text: &str) {
    app.get_by_label(label).click();
    app.run_steps(2);
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.get_by_label(label).type_text(text);
    app.key_press(Key::Enter);
    app.run_steps(4);
}

#[test]
fn ribbon_and_editor_create_a_terminal_unproved_lattice() {
    let instance = egui_wgpu::wgpu::Instance::default();
    assert!(
        !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty(),
        "lattice click acceptance requires a renderer"
    );
    let temporary = std::env::temp_dir().join(format!("soliddraft-lattice-{}", std::process::id()));
    std::fs::create_dir_all(&temporary).unwrap();
    soliddraft_app::file::set_config_dir(Some(temporary.join("config")));
    let mut doc = Document::new("Lattice UI", soliddraft_app::kernel());
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams {
            profile: ProfileKind::Rectangle { width: 25.0, height: 15.0 },
            depth: 10.0,
            ..Default::default()
        },
        via_assistant: false,
    })
    .unwrap();
    let mut doc = Some(doc);
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_render_every_step(true)
        .wgpu()
        .build_eframe(move |cc| SolidDraftApp::new(cc, doc.take().unwrap(), Automation::default()));
    app.run_steps(5);
    button(&mut app, "Lattice");
    assert!(app.state().state.editor.is_some());
    // Seed other values so typing proves each control writes its parameter.
    let soliddraft_app::state::EditorParams::Lattice(l) = &mut app.state_mut().state.editor.as_mut().unwrap().params
    else {
        panic!("lattice editor")
    };
    l.cell_size_mm = Some(6.0);
    l.radius_mm = Some(2.0.into());
    app.run_steps(2);
    field(&mut app, "Cell size value", "5");
    field(&mut app, "Beam radius (mm), controls density value", "1");
    assert!(app.query_by_label("unproved preview").is_some());
    button(&mut app, "OK (Enter)");
    app.run_steps(12);
    let doc = &app.state().state.doc;
    let last = doc.features.last().unwrap();
    assert_eq!(last.name, "Lattice1");
    let FeatureKind::Lattice(l) = last.kind else { panic!("last feature is lattice") };
    assert_eq!(l.cell_size_mm, Some(5.0));
    assert_eq!(l.radius_mm, Some(1.0.into()));
    assert_eq!(last.status, soliddraft_app::document::FeatureStatus::Ok);
    assert!(last.summary().contains("unproved preview"));
    let image = std::path::PathBuf::from(
        std::env::var_os("LATTICE_UI_EVIDENCE").unwrap_or_else(|| temporary.join("lattice.png").into_os_string()),
    );
    app.render().unwrap().save(image).unwrap();
}

#[test]
fn ribbon_and_editor_create_a_terminal_unproved_gyroid() {
    let instance = egui_wgpu::wgpu::Instance::default();
    assert!(
        !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty(),
        "gyroid click acceptance requires a renderer"
    );
    let temporary = std::env::temp_dir().join(format!("soliddraft-gyroid-{}", std::process::id()));
    std::fs::create_dir_all(&temporary).unwrap();
    soliddraft_app::file::set_config_dir(Some(temporary.join("config")));
    let mut doc = Some(Document::new("Gyroid UI", soliddraft_app::kernel()));
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_render_every_step(true)
        .wgpu()
        .build_eframe(move |cc| SolidDraftApp::new(cc, doc.take().unwrap(), Automation::default()));
    // The body of G0, made by the verbs the scenario uses.
    for (tool, arguments) in [
        (
            "sketch",
            json!({"plane":"top", "offset":-5, "shapes":[
                {"shape":"rectangle", "center":[0,0,-5], "width":20, "height":20}
            ]}),
        ),
        ("extrude", json!({"sketch":"Sketch1", "depth":10, "flip":false})),
    ] {
        let caller = Caller::Mcp { client: "test".into() };
        let out = call_checked(&mut AppTools { state: &mut app.state_mut().state }, tool, arguments, &caller);
        assert!(!out.is_error, "{tool}: {}", out.content);
    }
    app.run_steps(5);
    button(&mut app, "Gyroid");
    assert!(app.state().state.editor.is_some());
    // Seed other values so typing proves each control writes its parameter.
    let soliddraft_app::state::EditorParams::Gyroid(g) = &mut app.state_mut().state.editor.as_mut().unwrap().params
    else {
        panic!("gyroid editor")
    };
    g.period_mm = 20.0;
    g.level = 0.5.into();
    app.run_steps(2);
    field(&mut app, "Period value", "10");
    field(&mut app, "Network level (dimensionless) value", "0");
    assert!(app.query_by_label("unproved preview").is_some());
    button(&mut app, "OK (Enter)");
    app.run_steps(12);
    let last = app.state().state.doc.features.last().unwrap();
    assert_eq!(last.name, "Gyroid1");
    let FeatureKind::Gyroid(g) = last.kind else { panic!("last feature is gyroid") };
    assert_eq!((g.period_mm, g.level), (10.0, soliddraft_kernel_api::Grade::Constant(0.0)));
    assert_eq!(last.status, soliddraft_app::document::FeatureStatus::Ok);
    assert!(last.summary().contains("unproved preview"));

    button(&mut app, "Gyroid1");
    button(&mut app, "Edit feature");
    field(&mut app, "Network level (dimensionless) value", "0.5");
    button(&mut app, "OK (Enter)");
    app.run_steps(12);
    let last = app.state().state.doc.features.last().unwrap();
    assert_eq!(last.name, "Gyroid1");
    let FeatureKind::Gyroid(g) = last.kind else { panic!("last feature is gyroid") };
    assert_eq!((g.period_mm, g.level), (10.0, soliddraft_kernel_api::Grade::Constant(0.5)));
    assert_eq!(last.status, soliddraft_app::document::FeatureStatus::Ok);
    assert!(app.query_by_label("unproved preview").is_some(), "the viewport keeps its label");
    let image = std::path::PathBuf::from(
        std::env::var_os("GYROID_UI_EVIDENCE").unwrap_or_else(|| temporary.join("gyroid.png").into_os_string()),
    );
    app.render().unwrap().save(image).unwrap();
}

#[test]
fn ridealong_edit_suite_exposes_all_lattice_cells() {
    let temporary = std::env::temp_dir().join(format!("soliddraft-cells-{}", std::process::id()));
    soliddraft_app::file::set_config_dir(Some(temporary.join("config")));
    let mut doc = Document::new("Lattice cells", soliddraft_app::kernel());
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams {
            profile: ProfileKind::Rectangle { width: 25.0, height: 15.0 },
            depth: 10.0,
            ..Default::default()
        },
        via_assistant: false,
    })
    .unwrap();
    let mut doc = Some(doc);
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_render_every_step(true)
        .wgpu()
        .build_eframe(move |cc| SolidDraftApp::new(cc, doc.take().unwrap(), Automation::default()));
    app.state_mut().state.frame_layout = soliddraft_app::settings::FrameLayout::EditSuite;
    app.run_steps(5);
    ribbon::run_command(&mut app, "lattice");
    for choice in [
        soliddraft_kernel_api::LatticeCell::BodyCentred,
        soliddraft_kernel_api::LatticeCell::FaceCentred,
        soliddraft_kernel_api::LatticeCell::Octet,
    ] {
        app.get_by_label("Cell type").click();
        app.run_steps(3);
        app.get_by_label(choice.label()).click();
        app.run_steps(3);
        let soliddraft_app::state::EditorParams::Lattice(lattice) = &app.state().state.editor.as_ref().unwrap().params
        else {
            panic!("lattice editor")
        };
        assert_eq!(lattice.cell, Some(choice));
    }
    let folder = std::env::var_os("SOLIDDRAFT_TEST_OUT").map(std::path::PathBuf::from).unwrap_or(temporary);
    std::fs::create_dir_all(&folder).unwrap();
    app.render().unwrap().save(folder.join("ridealong-lattice-editor.png")).unwrap();
    ribbon::run_command(&mut app, "confirm");
    app.run_steps(30);
    let start = std::time::Instant::now();
    while app.state().state.doc.features.last().unwrap().status == soliddraft_app::document::FeatureStatus::Pending {
        assert!(start.elapsed().as_secs() < 60, "lattice proof did not finish");
        std::thread::sleep(std::time::Duration::from_millis(10));
        app.run_steps(1);
    }
    let FeatureKind::Lattice(lattice) = &app.state().state.doc.features.last().unwrap().kind else { panic!("lattice") };
    assert_eq!(lattice.cell, Some(soliddraft_kernel_api::LatticeCell::Octet));
    assert_eq!(app.state().state.doc.features.last().unwrap().status, soliddraft_app::document::FeatureStatus::Ok);
    app.render().unwrap().save(folder.join("ridealong-octet.png")).unwrap();
}
