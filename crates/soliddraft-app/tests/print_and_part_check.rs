//! Release checks through the real command queue, report windows and export buttons.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use soliddraft_app::{app_tools::AppTools, document::Command, state::AppAction};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_exchange::{Format, Report};
use std::time::{Duration, Instant};

fn counted_plate() -> soliddraft_app::document::Document {
    let kernel = std::sync::Arc::new(soliddraft_kernel_api::counting::Counting::new(soliddraft_app::kernel()));
    let mut doc = soliddraft_app::document::Document::new("Checked part", kernel);
    doc.apply(Command::AddExtrude { params: Default::default(), via_assistant: false }).unwrap();
    doc
}

fn click(app: &mut ribbon::App, label: &str) {
    app.get_all_by_role_and_label(Role::Button, label).next().unwrap_or_else(|| panic!("missing {label}")).click();
    app.run_steps(4);
}

fn settle(app: &mut ribbon::App) {
    let until = Instant::now() + Duration::from_secs(120);
    loop {
        app.run_steps(2);
        let state = &app.state().state;
        if !state.doc.proof_pending()
            && state.print_check.as_ref().is_none_or(|dialog| dialog.result.is_some())
            && state.part_check.as_ref().is_none_or(|inspection| inspection.result.is_some())
        {
            break;
        }
        assert!(Instant::now() < until, "check did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    app.run_steps(4);
}

#[test]
fn inspect_check_reports_independent_coverage_and_invalidates_on_undo() {
    let case = ribbon::case("independent-part-check");
    let mut app = ribbon::app(counted_plate());
    let before = app.state().state.doc.model().part_json();
    let undo = app.state().state.doc.undo_depth();
    click(&mut app, "Inspect tab");
    click(&mut app, "Check");
    settle(&mut app);
    let report = app.state().state.part_check.as_ref().unwrap().result.as_ref().unwrap().as_ref().unwrap();
    assert!(report.steps.iter().any(|step| step.boundary_built && step.field_built));
    assert!(report.steps.iter().all(|step| step.reference || step.unconfirmed.is_empty()));
    assert!(app.query_by_label("Coverage: 1 of 1 solid steps confirmed independently (100%).").is_some());
    assert!(
        app.query_by_label(
            "This checks construction coverage. Surface agreement and print accuracy are not checked here."
        )
        .is_some()
    );
    assert_eq!(app.state().state.doc.model().part_json(), before);
    assert_eq!(app.state().state.doc.undo_depth(), undo);
    case.frame(&mut app, "confirmed");
    app.state_mut().state.push(AppAction::Undo);
    app.run_steps(5);
    assert!(app.query_by_label("The part changed. This report belongs to an earlier revision.").is_some());
}

#[test]
fn mesh_exports_show_the_native_certificate_before_writing() {
    let case = ribbon::case("print-check-export");
    let mut app = ribbon::app(counted_plate());
    for format in [Format::Stl, Format::ThreeMf, Format::Obj] {
        let path = case.path(&format!("part.{}", format.extension()));
        let previous = b"previous file";
        std::fs::write(&path, previous).unwrap();
        app.state_mut().state.push(AppAction::Export { format, path: Some(path.clone()) });
        settle(&mut app);
        let output = app.state().state.print_check.as_ref().unwrap().result.as_ref().unwrap().as_ref().unwrap();
        let Report::Mesh(report) = &output.report else { panic!("mesh report") };
        let check = report.print_check.as_ref().unwrap();
        assert_eq!(check.route, "brep");
        assert!(check.certified_accuracy_mm <= check.requested_accuracy_mm);
        assert_eq!(check.stages.len(), 6);
        assert!(app.query_by_label("Print route: boundary model").is_some());
        assert_eq!(std::fs::read(&path).unwrap(), previous);
        let expected = output.bytes.clone();
        case.frame(&mut app, format.extension());
        click(&mut app, "Save checked export");
        assert!(app.state().state.print_check.is_none());
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        if format == Format::Stl {
            assert!(expected.starts_with(b"CXPRINT1"));
        }
        if format == Format::Obj {
            let text = String::from_utf8(expected).unwrap();
            assert_eq!(text.lines().filter(|line| line.starts_with("g face_")).count(), 6);
        }
    }
}

fn tool(app: &mut ribbon::App, name: &str, input: serde_json::Value) {
    let out = call_checked(
        &mut AppTools { state: &mut app.state_mut().state },
        name,
        input,
        &Caller::Mcp { client: "print-check".into() },
    );
    assert!(!out.is_error, "{name}: {}", out.content);
    app.run_steps(4);
}

#[test]
fn a_refused_accuracy_offers_a_labelled_coarser_choice_and_cancel_preserves_the_file() {
    let case = ribbon::case("print-check-coarser");
    let mut app = ribbon::app(ribbon::blank());
    tool(
        &mut app,
        "sketch",
        json!({"plane":"top","offset":99900.1,"shapes":[{"shape":"rectangle","center":[0,0,99900.1],"width":6,"height":6}]}),
    );
    tool(&mut app, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    let mut export_settings = app.state().state.doc.model().settings().clone();
    export_settings.export.chord_height = 0.00001;
    app.state_mut().state.doc.set_settings(export_settings).unwrap();
    let path = case.path("coarser.stl");
    std::fs::write(&path, b"previous file").unwrap();
    let settings = app.state().state.doc.model().settings().clone();
    app.state_mut().state.push(AppAction::Export { format: Format::Stl, path: Some(path.clone()) });
    settle(&mut app);
    assert!(app.state().state.print_check.as_ref().unwrap().result.as_ref().unwrap().is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"previous file");
    case.frame(&mut app, "refused");
    for _ in 0..10 {
        let dialog = app.state().state.print_check.as_ref().unwrap();
        if dialog.result.as_ref().unwrap().is_ok() {
            break;
        }
        let mm = dialog.accuracy_mm * 2.0;
        click(&mut app, &format!("Try coarser accuracy ({mm} mm)"));
        settle(&mut app);
    }
    let dialog = app.state().state.print_check.as_ref().unwrap();
    assert!(dialog.result.as_ref().unwrap().is_ok(), "{:?}", dialog.result);
    assert!(dialog.accuracy_mm > dialog.requested_mm);
    assert!(
        app.query_by_label(&format!(
            "Coarser choice: {} mm. This allows more deviation from the part.",
            dialog.accuracy_mm
        ))
        .is_some()
    );
    let expected = dialog.result.as_ref().unwrap().as_ref().unwrap().bytes.clone();
    case.frame(&mut app, "coarser-confirmed");
    assert_eq!(app.state().state.doc.model().settings(), &settings);
    click(&mut app, "Save checked export");
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    app.state_mut().state.push(AppAction::Export { format: Format::Stl, path: Some(path.clone()) });
    settle(&mut app);
    assert!(app.state().state.print_check.as_ref().unwrap().result.as_ref().unwrap().is_err());
    click(&mut app, "Cancel");
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    assert!(app.state().state.print_check.is_none());
}

#[test]
fn changing_the_part_prevents_saving_an_earlier_checked_export() {
    let case = ribbon::case("print-check-stale");
    let mut app = ribbon::app(ribbon::plate());
    let path = case.path("stale.stl");
    app.state_mut().state.push(AppAction::Export { format: Format::Stl, path: Some(path.clone()) });
    settle(&mut app);
    app.state_mut().state.push(AppAction::Command(Command::Rollback(Some(0))));
    app.run_steps(4);
    assert!(app.query_by_label("The part changed. Close this report and export again.").is_some());
    assert!(app.query_all_by_role_and_label(Role::Button, "Save checked export").next().is_none());
    assert!(!path.exists());
}

#[test]
fn an_unconfirmed_field_step_reports_partial_coverage_without_claiming_agreement() {
    let case = ribbon::case("part-check-unconfirmed");
    let mut app = ribbon::app(ribbon::plate());
    tool(&mut app, "lattice", json!({"cell":"cubic","cell_size":100,"radius":1,"accuracy":0.5,"min_detail":0.1}));
    ribbon::run_command(&mut app, "check");
    settle(&mut app);
    let report = app.state().state.part_check.as_ref().unwrap().result.as_ref().unwrap().as_ref().unwrap();
    assert!(report.steps.iter().any(|step| step.boundary_built && step.field_built));
    let field = report.steps.iter().find(|step| step.operation == "lattice").unwrap();
    assert!(!field.boundary_built);
    assert!(field.field_built);
    assert!(field.unconfirmed.iter().any(|reason| reason.contains("Boundary model")));
    assert!(app.query_by_label("Coverage: 1 of 2 solid steps confirmed independently (50%).").is_some());
    case.frame(&mut app, "partial-coverage");
}

#[test]
fn a_field_export_reviews_its_native_route_and_verdict_before_saving() {
    let case = ribbon::case("print-check-field");
    let mut app = ribbon::app(ribbon::blank());
    tool(
        &mut app,
        "sketch",
        json!({"plane":"top","offset":20,"shapes":[{"shape":"rectangle","center":[0,0,20],"width":6,"height":6}]}),
    );
    tool(&mut app, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    tool(&mut app, "lattice", json!({"cell":"cubic","cell_size":100,"radius":1,"accuracy":0.5,"min_detail":0.1}));
    let path = case.path("field.stl");
    app.state_mut().state.push(AppAction::Export { format: Format::Stl, path: Some(path.clone()) });
    settle(&mut app);
    let dialog = app.state().state.print_check.as_ref().unwrap();
    let output = dialog.result.as_ref().unwrap().as_ref().unwrap();
    let Report::FieldMesh(report) = &output.report else { panic!("field report") };
    assert_eq!(report.route, "field");
    assert_eq!(dialog.requested_mm, 0.5);
    assert!(report.closed && report.connected && report.non_self_intersecting);
    assert!(app.query_by_label("Print route: distance field").is_some());
    assert!(!path.exists());
    let expected = output.bytes.clone();
    case.frame(&mut app, "certified-field");
    click(&mut app, "Save checked export");
    assert_eq!(std::fs::read(path).unwrap(), expected);
}
