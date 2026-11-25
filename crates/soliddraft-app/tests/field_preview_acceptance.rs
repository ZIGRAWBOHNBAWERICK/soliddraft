//! Roadmap 4.1, field preview and first display, held at the real app on the pinned
//! Catallax kernel.
//!
//! The app draws a field preview only for the field-only features, lattices and gyroids;
//! the boundary model previews everything else (checkpoint 2, Q32). A feature shows at
//! once: its first display is the screen's mesh, never a certified one it would wait for.
//! The proved and bounded lights are gone; a working mark and a refused message stay
//! (Q31). Done when a preview never stands in for an export: the viewport's mesh and an
//! approved export come from separate routes, and an export the kernel refuses writes
//! nothing even while the screen shows the preview.
//!
//! The kernel is wrapped in [`Counting`], which logs the mesh route of every request:
//! the screen's display, a certified mesh, the print route of a mesh export or a field's
//! certified STL. Catallax's own log also records certified work nested inside a
//! display request. [`Gated`] holds a rebuild so its working state can be seen.

mod gated;
#[path = "support/ui.rs"]
mod ribbon;

use egui_kittest::kittest::{NodeT, Queryable};
use gated::Gated;
use serde_json::json;
use soliddraft_app::document::{Command, Document, ExtrudeParams, FeatureId, FeatureStatus, ProfileKind};
use soliddraft_app::{app_tools::AppTools, state::AppAction};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_exchange::{Format, Report};
use soliddraft_kernel_api::BodyId;
use soliddraft_kernel_api::counting::{Counting, MeshRoute};
use soliddraft_kernel_catallax::{APP_REQUEST_WORK, CatallaxKernel};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

const HANG: Duration = Duration::from_secs(600);
const UNPROVED: &str = "unproved preview";

fn counted() -> (Arc<Counting>, Document) {
    let kernel = Arc::new(Counting::new(soliddraft_app::startup::kernel()));
    (kernel.clone(), Document::new("Field preview", kernel))
}

fn observed_display() -> (Arc<CatallaxKernel>, Arc<Counting>, Document) {
    let catallax = Arc::new(CatallaxKernel::with_diagnostics(APP_REQUEST_WORK));
    let kernel = Arc::new(Counting::new(catallax.clone()));
    let doc = Document::new("First display", kernel.clone());
    (catallax, kernel, doc)
}

fn assert_first_display_used_no_certified_mesh(catallax: &CatallaxKernel, feature: &str) {
    let diagnostics = catallax.take_diagnostics().unwrap();
    let calls = diagnostics["calls"].as_array().unwrap();
    let requests: Vec<_> = calls.iter().map(|call| call["operation"].as_str().unwrap()).collect();
    let operations: Vec<_> = calls
        .iter()
        .flat_map(|call| call["operation_log"]["operations"].as_array().unwrap())
        .map(|op| op["operation"].as_str().unwrap())
        .collect();
    assert!(requests.contains(&"preview"), "{feature}: Catallax did not observe the preview");
    assert!(
        !requests.iter().chain(&operations).any(|op| matches!(*op, "tessellate" | "tessellate_edges")),
        "{feature}: Catallax asked for a certified mesh: requests {requests:?}, operations {operations:?}"
    );
}

fn tool(app: &mut ribbon::App, name: &str, input: serde_json::Value) {
    let out = call_checked(
        &mut AppTools { state: &mut app.state_mut().state },
        name,
        input,
        &Caller::Mcp { client: "field-preview".into() },
    );
    assert!(!out.is_error, "{name}: {}", out.content);
}

/// A 20 by 20 by 10 block centred on the origin, the body of the gyroid G0.
fn block(app: &mut ribbon::App) {
    tool(
        app,
        "sketch",
        json!({"plane":"top","offset":-5,"shapes":[{"shape":"rectangle","center":[0,0,-5],"width":20,"height":20}]}),
    );
    tool(app, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
}

/// Frames until no build is owed and the Print check window, if open, has its outcome.
fn settle(app: &mut ribbon::App) {
    let until = Instant::now() + HANG;
    loop {
        app.run_steps(2);
        let state = &app.state().state;
        if !state.doc.proof_pending() && state.print_check.as_ref().is_none_or(|dialog| dialog.result.is_some()) {
            break;
        }
        assert!(Instant::now() < until, "the part did not settle");
        std::thread::sleep(Duration::from_millis(10));
    }
    app.run_steps(4);
}

fn shown(app: &ribbon::App) -> (BodyId, Arc<soliddraft_app::document::DisplayMesh>) {
    let doc = &app.state().state.doc;
    let body = doc.display_body().expect("the part has a body");
    let mesh = doc.meshes.get(&body).expect("the body is drawn").clone();
    assert!(!mesh.indices.is_empty(), "the display mesh has triangles");
    (body, mesh)
}

/// The routes a body's requests took, in order.
fn routes_of(log: &[(BodyId, MeshRoute)], body: BodyId) -> Vec<MeshRoute> {
    log.iter().filter(|(b, _)| *b == body).map(|(_, r)| *r).collect()
}

fn marks(app: &ribbon::App) -> Vec<String> {
    app.query_all_by_label_contains(" status: ")
        .filter_map(|node| node.accesskit_node().label().map(|label| label.to_string()))
        .collect()
}

#[test]
fn boundary_model_features_show_at_once_from_their_own_faces() {
    let case = ribbon::case("field-preview-brep");
    let (catallax, kernel, doc) = observed_display();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        tool(
            app,
            "sketch",
            json!({"plane":"top","shapes":[{"shape":"rectangle","center":[9,6,0],"width":6,"height":12}]}),
        );
        kernel.take_routes();
        let steps = [
            ("revolve", json!({"sketch":"Sketch1","axis_origin":[0,0,0],"axis_direction":[0,1,0],"angle_deg":360})),
            ("chamfer", json!({"edges":["Revolve1 · Edge 1","Revolve1 · Edge 3"],"distance":1})),
        ];
        for (verb, input) in steps {
            catallax.take_diagnostics().unwrap();
            tool(app, verb, input);
            // Before any frame: the feature is built and drawn, nothing is owed.
            assert!(!app.state().state.doc.proof_pending(), "{verb} owes no build");
            let (body, mesh) = shown(app);
            let routes = routes_of(&kernel.take_routes(), body);
            assert!(!routes.is_empty() && routes.iter().all(|r| *r == MeshRoute::Display), "{verb}: {routes:?}");
            assert_first_display_used_no_certified_mesh(&catallax, verb);
            // Drawn from the boundary model: one pick per face, every face drawn.
            let faces = app.state().state.doc.kernel().topology(body).unwrap().faces.len();
            let drawn: BTreeSet<u32> = mesh.face_picks.iter().copied().filter(|p| *p != 0).collect();
            assert_eq!(drawn.len(), faces, "{verb}: every face of the boundary model is drawn and pickable");
            app.run_steps(4);
            assert!(app.query_by_label(UNPROVED).is_none(), "{verb}: a boundary-model part is not labelled");
            assert!(marks(app).is_empty(), "{verb}: a built feature has no mark: {:?}", marks(app));
            let last = app.state().state.doc.features.last().unwrap();
            assert_eq!(last.status, FeatureStatus::Ok);
            assert!(!last.summary().contains(UNPROVED), "{verb}: {}", last.summary());
        }
        case.frame(app, "chamfered-revolve");
    });
}

#[test]
fn lattices_and_gyroids_show_at_once_from_their_field_sample() {
    let case = ribbon::case("field-preview-field");
    let lattice = |cell: &str| json!({"cell":cell,"cell_size":10,"radius":1,"accuracy":0.5,"min_detail":0.5});
    let gyroid = json!({"kind":"network","period":10,"level":0,"accuracy":0.5,"min_detail":0.5});
    let cases = [
        ("cubic", "lattice", lattice("cubic"), None),
        ("body-centred", "lattice", lattice("body_centred"), None),
        ("face-centred", "lattice", lattice("face_centred"), None),
        ("octet", "lattice", lattice("octet"), None),
        (
            "graded-lattice",
            "lattice",
            lattice("cubic"),
            Some(json!({"feature":"Lattice1","axis":"z","from":-5,"to":5,"start":0.5,"end":1})),
        ),
        ("gyroid", "gyroid", gyroid.clone(), None),
        (
            "graded-gyroid",
            "gyroid",
            gyroid,
            Some(json!({"feature":"Gyroid1","axis":"z","from":-5,"to":5,"start":0,"end":0.2})),
        ),
    ];
    for (name, verb, input, grade) in cases {
        let (catallax, kernel, doc) = observed_display();
        let mut app = ribbon::app(doc);
        case.run(&mut app, |app| {
            block(app);
            let (before, _) = shown(app);
            catallax.take_diagnostics().unwrap();
            tool(app, verb, input);
            if let Some(grade) = grade {
                tool(app, "grade_field", grade);
            }
            assert!(!app.state().state.doc.proof_pending(), "{name} owes no build");
            let (body, mesh) = shown(app);
            assert_ne!(body, before, "{name} replaces the block on screen");
            let routes = routes_of(&kernel.take_routes(), body);
            assert!(!routes.is_empty() && routes.iter().all(|r| *r == MeshRoute::Display), "{name}: {routes:?}");
            assert_first_display_used_no_certified_mesh(&catallax, name);
            // The screen draws the field's own sample, which has no faces to pick.
            let sample = app.state().state.doc.kernel().preview(body, &Default::default()).unwrap();
            let drawn: Vec<[f32; 3]> = mesh.positions.iter().map(|p| p.to_array()).collect();
            let expected: Vec<[f32; 3]> = sample.indices.iter().map(|i| sample.positions[*i as usize]).collect();
            assert_eq!(drawn, expected, "{name} is drawn from its field sample");
            app.run_steps(4);
            assert!(app.query_by_label(UNPROVED).is_some(), "{name}: the viewport labels the field preview");
            assert!(marks(app).is_empty(), "{name}: a built field has no mark: {:?}", marks(app));
            let last = app.state().state.doc.features.last().unwrap();
            assert_eq!(last.status, FeatureStatus::Ok, "{name}");
            assert_eq!(last.summary(), "unproved preview · ±0.50 mm", "{name}");
            case.frame(app, name);
        });
    }
}

#[test]
fn an_export_is_made_by_its_own_route_never_from_the_preview() {
    let case = ribbon::case("field-preview-export");
    // A boundary-model part: every mesh format runs the print route, and the file is the
    // same whatever the viewport quality.
    let (kernel, doc) = counted();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        block(app);
        let (body, _) = shown(app);
        kernel.take_routes();
        for format in [Format::Stl, Format::ThreeMf, Format::Obj] {
            let out = app.state().state.export_part(format).unwrap();
            let Report::Mesh(report) = &out.report else { panic!("{format:?} mesh report") };
            assert_eq!(report.print_check.as_ref().map(|check| check.route.as_str()), Some("brep"));
            assert_eq!(routes_of(&kernel.take_routes_here(), body), [MeshRoute::Print], "{format:?}");
        }
        let at_default = app.state().state.export_part(Format::Stl).unwrap().bytes;
        assert!(at_default.starts_with(b"CXPRINT1"));
        let mut settings = app.state().state.doc.model().settings().clone();
        settings.viewport = soliddraft_tessellation::Quality::Slider { value: 1 };
        app.state_mut().state.doc.set_settings(settings).unwrap();
        settle(app);
        kernel.take_routes();
        let at_coarse = app.state().state.export_part(Format::Stl).unwrap().bytes;
        assert_eq!(at_coarse, at_default, "the viewport quality never reaches the file");
        assert_eq!(routes_of(&kernel.take_routes_here(), body), [MeshRoute::Print]);
    });

    // A lattice: only its certified field STL, marked as the field route, and every other
    // format refused before any mesh is made.
    let (kernel, doc) = counted();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        tool(
            app,
            "sketch",
            json!({"plane":"top","offset":20,"shapes":[{"shape":"rectangle","center":[0,0,20],"width":6,"height":6}]}),
        );
        tool(app, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
        tool(app, "lattice", json!({"cell":"cubic","cell_size":100,"radius":1,"accuracy":0.5,"min_detail":0.1}));
        let (body, _) = shown(app);
        kernel.take_routes();
        for format in [Format::Step, Format::Iges, Format::ThreeMf, Format::Obj] {
            let error = app.state().state.export_part(format).unwrap_err();
            assert_eq!(error, "field parts export as meshes; use labelled STL", "{format:?}");
        }
        assert!(kernel.take_routes_here().is_empty(), "a refused format makes no mesh");
        let out = app.state().state.export_part(Format::Stl).unwrap();
        let Report::FieldMesh(report) = &out.report else { panic!("field report") };
        assert_eq!(report.route, "field");
        assert!(out.bytes.starts_with(b"CXPRINT1") && out.bytes[8] == 2, "the file is the certified field mesh");
        assert_eq!(routes_of(&kernel.take_routes_here(), body), [MeshRoute::FieldExport]);
        // Through the window: reviewed as the distance field before anything is written.
        let path = case.path("lattice.stl");
        app.state_mut().state.push(AppAction::Export { format: Format::Stl, path: Some(path.clone()) });
        settle(app);
        assert!(app.query_by_label("Print route: distance field").is_some());
        assert!(!path.exists());
        // The window checks on a thread of its own; the screen may refine meanwhile.
        let routes = routes_of(&kernel.take_routes(), body);
        assert_eq!(routes.iter().filter(|r| **r != MeshRoute::Display).collect::<Vec<_>>(), [&MeshRoute::FieldExport]);
        case.frame(app, "lattice-print-check");
    });
}

#[test]
fn a_preview_on_screen_does_not_make_an_export_the_kernel_refuses() {
    let case = ribbon::case("field-preview-refused-export");
    let (kernel, doc) = counted();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        tool(
            app,
            "sketch",
            json!({"plane":"top","offset":-5,"shapes":[{"shape":"rectangle","center":[0,0,-5],"width":20,"height":20}]}),
        );
        tool(app, "extrude", json!({"sketch":"Sketch1","depth":20,"flip":false}));
        // Level 1/2 in this box leaves a separate corner piece; at 2 mm the certified
        // route cannot settle the boundary within the minimum detail.
        tool(app, "gyroid", json!({"kind":"network","period":10,"level":0.5,"accuracy":2,"min_detail":2}));
        app.run_steps(4);
        let (body, _) = shown(app);
        assert_eq!(app.state().state.doc.features.last().unwrap().status, FeatureStatus::Ok);
        assert!(app.query_by_label(UNPROVED).is_some(), "the screen shows the gyroid's preview");
        kernel.take_routes();
        let path = case.path("gyroid.stl");
        std::fs::write(&path, b"previous file").unwrap();
        app.state_mut().state.push(AppAction::Export { format: Format::Stl, path: Some(path.clone()) });
        settle(app);
        let dialog = app.state().state.print_check.as_ref().unwrap();
        let error = dialog.result.as_ref().unwrap().as_ref().unwrap_err();
        assert!(error.contains("field.print-unresolved"), "{error}");
        assert!(app.query_by_label("Print route: distance field").is_some());
        assert!(
            app.query_by_label(
                "Export refused: The requested accuracy could not be certified within the calculation limits."
            )
            .is_some()
        );
        assert!(app.query_by_label("Save checked export").is_none(), "a refused export offers no Save");
        assert_eq!(std::fs::read(&path).unwrap(), b"previous file", "nothing is written");
        // The window checks on a thread of its own; the screen may refine meanwhile.
        let routes = routes_of(&kernel.take_routes(), body);
        assert_eq!(
            routes.iter().filter(|r| **r != MeshRoute::Display).collect::<Vec<_>>(),
            [&MeshRoute::FieldExport],
            "the export asks the field, not the screen's mesh"
        );
        case.frame(app, "refused-export");
    });
}

#[test]
fn a_field_whose_preview_cannot_be_sampled_is_refused_by_name() {
    let case = ribbon::case("field-preview-unsampled");
    let (_kernel, doc) = counted();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        block(app);
        let (before, _) = shown(app);
        let out = call_checked(
            &mut AppTools { state: &mut app.state_mut().state },
            "lattice",
            json!({"cell":"cubic","cell_size":10,"radius":0.2,"accuracy":0.5,"min_detail":1}),
            &Caller::Mcp { client: "field-preview".into() },
        );
        assert!(out.is_error && out.content.contains("field.print-budget"), "{}", out.content);
        app.run_steps(8);
        let doc = &app.state().state.doc;
        assert!(doc.features.iter().all(|f| f.name != "Lattice1"), "the lattice is not kept without a display");
        assert_eq!(shown(app).0, before, "the block stays on screen");
        assert!(app.query_by_label(UNPROVED).is_none());
        case.frame(app, "unsampled");
    });
}

fn release(kernel: &Gated) {
    let mut rules = kernel.rules.lock().unwrap();
    rules.hold = None;
    kernel.changed.notify_all();
}

#[test]
fn a_field_waiting_on_an_upstream_edit_is_marked_working_and_exports_nothing() {
    let case = ribbon::case("field-preview-working");
    let kernel = Arc::new(Gated::default());
    let mut doc = Document::new("Working field", kernel.clone());
    let params = |depth| ExtrudeParams {
        profile: ProfileKind::Rectangle { width: 20.0, height: 20.0 },
        depth,
        ..Default::default()
    };
    let block: FeatureId =
        doc.apply(Command::AddExtrude { params: params(10.0), via_assistant: false }).unwrap().unwrap();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        tool(app, "gyroid", json!({"kind":"network","period":10,"level":0,"accuracy":0.5,"min_detail":0.5}));
        app.run_steps(4);
        let (built, _) = shown(app);
        assert!(app.query_by_label(UNPROVED).is_some());

        kernel.rules.lock().unwrap().hold = Some(12.0);
        app.state_mut().state.doc.apply_interactive(Command::EditExtrude { id: block, params: params(12.0) }).unwrap();
        {
            let rules = kernel.changed.wait_while(kernel.rules.lock().unwrap(), |r| r.holding.is_none());
            drop(rules);
        }
        app.run_steps(8);
        assert!(app.state().state.doc.proof_pending());
        let gyroid = app.state().state.doc.features.last().unwrap();
        assert_eq!(gyroid.status, FeatureStatus::Pending);
        assert!(app.query_by_label("Gyroid1 status: Working").is_some(), "marks: {:?}", marks(app));
        // The screen keeps the gyroid it last built, still labelled a preview, and nothing
        // is exported while the edit works.
        let doc = &app.state().state.doc;
        assert_eq!(doc.meshes.keys().copied().collect::<Vec<_>>(), [built], "the built gyroid stays on screen");
        assert!(doc.provisional_mesh().is_none(), "no construction stands in for the field");
        assert!(app.query_by_label(UNPROVED).is_some());
        assert_eq!(
            app.state().state.export_part(Format::Stl).unwrap_err(),
            "The current edit is still working. Export waits for it."
        );
        case.frame(app, "working");

        release(&kernel);
        settle(app);
        let gyroid = app.state().state.doc.features.last().unwrap();
        assert_eq!(gyroid.status, FeatureStatus::Ok);
        assert!(marks(app).is_empty(), "marks: {:?}", marks(app));
        assert_ne!(shown(app).0, built, "the rebuilt gyroid replaces the earlier one");
        assert!(app.query_by_label(UNPROVED).is_some());
        case.frame(app, "rebuilt");
    });
}

#[test]
fn a_refused_field_is_marked_with_its_reason_and_exports_nothing() {
    let case = ribbon::case("field-preview-refused");
    let (kernel, doc) = counted();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        block(app);
        tool(app, "gyroid", json!({"kind":"network","period":10,"level":0,"accuracy":0.5,"min_detail":0.5}));
        app.run_steps(4);
        // A level graded past the range Catallax certifies keeps the edit and is refused.
        tool(app, "grade_field", json!({"feature":"Gyroid1","axis":"z","from":-5,"to":5,"start":0,"end":0.9}));
        app.run_steps(8);
        let gyroid = app.state().state.doc.features.last().unwrap();
        let FeatureStatus::Error(reason) = &gyroid.status else { panic!("refused: {:?}", gyroid.status) };
        assert!(reason.contains("gyroid.grade-range-unsupported"), "{reason}");
        assert!(app.query_by_label("Gyroid1 status: Refused").is_some(), "marks: {:?}", marks(app));
        assert!(app.query_by_label(UNPROVED).is_none(), "a refused field shows no field preview label");
        assert_eq!(gyroid.summary(), "±0.50 mm", "a refused field's row does not claim a preview");
        kernel.take_routes();
        let error = app.state().state.export_part(Format::Stl).unwrap_err();
        assert!(
            error.starts_with("Export needs every feature built. Gyroid1: ")
                && error.contains("gyroid.grade-range-unsupported"),
            "{error}"
        );
        assert!(
            kernel.take_routes_here().iter().all(|(_, route)| *route == MeshRoute::Display),
            "a refused field makes no export mesh"
        );
        case.frame(app, "refused-grade");
    });
}
