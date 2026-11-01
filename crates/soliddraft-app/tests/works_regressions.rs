//! Release blockers exercised through the native HTTP endpoint and real app frames.

use egui::vec2;
use egui_kittest::Harness;
use kurbo::Shape;
use serde_json::{Value, json};
use soliddraft_app::app::{Automation, SolidDraftApp};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod gated;
#[path = "support/render_steps.rs"]
mod render_steps;

type App = Harness<'static, SolidDraftApp>;

static NATIVE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn native_lock() -> std::sync::MutexGuard<'static, ()> {
    NATIVE.lock().unwrap_or_else(|e| e.into_inner())
}

fn directory(name: &str) -> PathBuf {
    let root = std::env::var_os("SOLIDDRAFT_WORKS_PROOF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-works-{}", std::process::id())));
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("work")).unwrap();
    dir
}

fn app() -> App {
    app_with(soliddraft_app::new_document())
}

fn app_with(doc: soliddraft_app::document::Document) -> App {
    soliddraft_app::file::set_config_dir(Some(directory("config")));
    eprintln!("Catallax revision: {}", soliddraft_kernel_catallax::CATALLAX_REVISION);
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .renderer(render_steps::renderer())
        .build_eframe(|cc| SolidDraftApp::new(cc, doc, Automation::default()));
    app.run_steps(3);
    app.state_mut().state.start_mcp(0, None).unwrap();
    app
}

fn request(app: &mut App, name: &str, arguments: Value) -> (Value, Duration) {
    let addr = app.state().state.chat.mcp.as_ref().unwrap().server.addr();
    let message = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":name,"arguments":arguments}});
    let started = Instant::now();
    let client = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(180))).unwrap();
        let body = message.to_string();
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });
    let mut longest_frame = Duration::ZERO;
    while !client.is_finished() {
        assert!(started.elapsed() < Duration::from_secs(180), "{name} never replied");
        let frame = Instant::now();
        app.run_steps(1);
        longest_frame = longest_frame.max(frame.elapsed());
        std::thread::sleep(Duration::from_millis(10));
    }
    let response = client.join().unwrap();
    eprintln!("{name}: elapsed={:?}, longest frame={longest_frame:?}", started.elapsed());
    assert!(response.starts_with("HTTP/1.1 200"), "{name}: {response}");
    let reply: Value = serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    (reply["result"].clone(), longest_frame)
}

fn call(app: &mut App, name: &str, arguments: Value) -> Value {
    request(app, name, arguments).0
}

fn tool(app: &mut App, name: &str, arguments: Value) -> Value {
    let reply = call(app, name, arguments);
    assert_eq!(reply["isError"], false, "{name}: {reply}");
    serde_json::from_str(reply["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn replay(app: &mut App, fixture: &str, count: usize, dir: &std::path::Path) {
    let bundle = serde_json::to_string(dir).unwrap();
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../ci/scenarios/{fixture}.json")),
    )
    .unwrap()
    .replace("{bundle}", &bundle[1..bundle.len() - 1]);
    let scenario: Value = serde_json::from_str(&source).unwrap();
    for (i, step) in scenario["steps"].as_array().unwrap().iter().take(count).enumerate() {
        let name = step["tool"].as_str().unwrap();
        let reply = call(app, name, step["arguments"].clone());
        let refused = step["refused"].is_string();
        assert_eq!(reply["isError"], refused, "{fixture} step {} {name}: {reply}", i + 1);
    }
}

fn basic_curved_shape(shape: &str, count: usize) {
    let _native = native_lock();
    let dir = directory(shape);
    let mut app = app();
    replay(&mut app, shape, count, &dir);
    let mut calls = Vec::new();
    for (name, arguments) in [
        ("save", json!({"path":dir.join("part.dwprt")})),
        ("open", json!({"path":dir.join("part.dwprt")})),
        ("export_step", json!({"path":dir.join("part.step")})),
        ("make_drawing", json!({"template":"iso-a3"})),
        ("drawing_save", json!({"path":dir.join("drawing.dwdrw")})),
        ("open", json!({"path":dir.join("drawing.dwdrw")})),
    ] {
        let started = Instant::now();
        let (reply, longest) = request(&mut app, name, arguments);
        let elapsed = started.elapsed();
        calls.push(json!({"tool":name,"elapsed_ms":elapsed.as_secs_f64()*1000.0,
            "longest_update_ms":longest.as_secs_f64()*1000.0,"reply":reply}));
        std::fs::write(dir.join("calls.json"), serde_json::to_vec_pretty(&calls).unwrap()).unwrap();
        assert_eq!(reply["isError"], false, "{shape} {name}: {reply}");
        if name == "make_drawing" {
            assert!(elapsed < Duration::from_secs(100), "{shape} drawing exceeded 100 seconds: {elapsed:?}");
        }
    }
    let session = app.state().state.drawing.session.as_ref().unwrap();
    assert!(app.state().state.drawing.is_active(), "{shape} drawing is not active");
    assert!(!session.sheet().views.is_empty(), "{shape} drawing has no views");
    for view in &session.sheet().views {
        assert!(!view.geometry.curves.is_empty(), "{shape} drawing view has no curves");
        if shape != "rod-cut-28deg" {
            let frame = soliddraft_drawing::project::view_spec(&session.doc.drawing, view.id).unwrap().frame().unwrap();
            let scale = soliddraft_drawing::project::view_scale(&session.doc.drawing, view.id).unwrap();
            let extent = |axis: soliddraft_kernel_api::Vec3| {
                let radius =
                    if shape == "sphere-revolve" { 1.0 } else { 3.0 * (1.0 - axis.y * axis.y).max(0.0).sqrt() + 1.0 };
                2.0 * radius * scale
            };
            let bounds = view
                .geometry
                .curves
                .iter()
                .filter(|curve| curve.visible)
                .map(|curve| curve.path.bounding_box())
                .reduce(|a, b| a.union(b))
                .expect("drawing view has visible paths");
            let (width, height) = (bounds.width(), bounds.height());
            let tolerance = 0.005 * scale + 0.0002;
            assert!(
                (width - extent(frame.right)).abs() <= tolerance,
                "{shape} view {:?}: width {width}, expected {}",
                view.id,
                extent(frame.right)
            );
            assert!(
                (height - extent(frame.up)).abs() <= tolerance,
                "{shape} view {:?}: height {height}, expected {}",
                view.id,
                extent(frame.up)
            );
        }
    }
    assert!(std::fs::metadata(dir.join("part.step")).unwrap().len() > 0);
    app.run_steps(3);
    app.render().unwrap().save(dir.join("drawing.png")).unwrap();
}

#[test]
fn sphere_revolve_has_full_drawing_views_and_file_roundtrips() {
    basic_curved_shape("sphere-revolve", 2);
}

#[test]
fn torus_revolve_has_full_drawing_views_and_file_roundtrips() {
    basic_curved_shape("torus-revolve", 2);
}

#[test]
fn angled_rod_has_drawing_views_and_file_roundtrips() {
    basic_curved_shape("rod-cut-28deg", 4);
}

#[test]
fn native_drawing_switches_keep_the_tab_and_undo_target() {
    let _native = native_lock();
    let dir = directory("tabs");
    let mut app = app();
    tool(
        &mut app,
        "sketch",
        json!({"plane":"top","shapes":[{"shape":"rectangle","center":[0,0,0],"width":40,"height":30}]}),
    );
    tool(&mut app, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    tool(&mut app, "save", json!({"path":dir.join("plate.dwprt")}));
    tool(&mut app, "make_drawing", json!({"template":"iso-a3"}));
    app.run_steps(3);
    let summary = tool(&mut app, "model_summary", json!({}));
    app.render().unwrap().save(dir.join("drawing-requested.png")).unwrap();
    assert_eq!(summary["drawing"]["active"], true, "{summary}");
    tool(&mut app, "drawing_model_items", json!({}));
    let part_depth = app.state().state.doc.undo_depth();
    let undone = tool(&mut app, "undo", json!({}));
    assert_eq!(undone["document"], "drawing");
    assert_eq!(app.state().state.doc.undo_depth(), part_depth);
    tool(&mut app, "drawing_show_part", json!({}));
    app.run_steps(3);
    assert!(!app.state().state.drawing.is_active());
    tool(&mut app, "show_drawing", json!({}));
    app.run_steps(3);
    assert!(app.state().state.drawing.is_active());
    tool(&mut app, "drawing_model_items", json!({}));
    let drawing_depth = app.state().state.drawing.session.as_ref().unwrap().doc.undo_depth();
    assert!(drawing_depth > 0);
    app.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    app.run_steps(3);
    assert_eq!(app.state().state.doc.undo_depth(), part_depth);
    assert_eq!(app.state().state.drawing.session.as_ref().unwrap().doc.undo_depth(), drawing_depth - 1);
    app.render().unwrap().save(dir.join("drawing.png")).unwrap();
}

#[test]
fn native_assembly_switches_keep_the_tab_and_undo_target() {
    let _native = native_lock();
    let dir = directory("assembly");
    let mut app = app();
    replay(&mut app, "mate-distance", 21, &dir);
    app.run_steps(3);
    app.render().unwrap().save(dir.join("assembly-requested.png")).unwrap();
    let active = app.state().state.assembly.is_active();
    let before = app.state().state.doc.undo_depth();
    let undone = tool(&mut app, "undo", json!({}));
    eprintln!(
        "assembly active before Undo: {active}; Undo document: {}; gap: {}",
        undone["document"], undone["assembly"]["values"][0]["value"]
    );
    assert_eq!(undone["document"], "assembly", "{undone}");
    assert_eq!(undone["assembly"]["values"][0]["value"], 5.0, "{undone}");
    assert_eq!(app.state().state.doc.undo_depth(), before);
    assert!(active);
    tool(&mut app, "redo", json!({}));
    tool(&mut app, "assembly_show_part", json!({}));
    app.run_steps(3);
    assert!(!app.state().state.assembly.is_active());
    tool(&mut app, "show_assembly", json!({}));
    app.run_steps(3);
    assert!(app.state().state.assembly.is_active());
    app.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    app.run_steps(3);
    assert_eq!(app.state().state.doc.undo_depth(), before);
    let summary = tool(&mut app, "model_summary", json!({}));
    assert_eq!(summary["assembly"]["values"][0]["value"], 5.0, "{summary}");
    tool(&mut app, "redo", json!({}));
    app.render().unwrap().save(dir.join("assembly.png")).unwrap();
}

#[test]
fn bracket_depth_edit_keeps_its_thickness_dimension() {
    let _native = native_lock();
    let dir = directory("bracket");
    let mut app = app();
    replay(&mut app, "bracket", 32, &dir);
    tool(&mut app, "make_drawing", json!({"template":"iso-a3"}));
    tool(&mut app, "drawing_model_items", json!({}));
    let thickness = app.state().state.drawing.session.as_ref().unwrap().sheet().annotations.iter().find(|a| {
        matches!(&a.body, soliddraft_drawing::AnnotationKind::Dimension(d) if matches!(d.value, soliddraft_drawing::DimValue::Measured {value} if (value - 10.0).abs() < 1e-6))
    }).expect("the bracket thickness is dimensioned").id;
    tool(&mut app, "drawing_show_part", json!({}));
    tool(&mut app, "edit_extrude", json!({"feature":"Boss-Extrude1","depth":12,"flip":false}));
    app.run_steps(3);
    let check = |app: &App| {
        let drawing = app.state().state.drawing.session.as_ref().unwrap();
        let a = drawing.sheet().annotations.iter().find(|a| a.id == thickness).unwrap();
        assert!(!matches!(a.state, soliddraft_drawing::AnnotationState::Dangling { .. }), "{a:?}");
        let soliddraft_drawing::AnnotationKind::Dimension(d) = &a.body else { panic!() };
        assert!(
            matches!(d.value, soliddraft_drawing::DimValue::Measured {value} if (value - 12.0).abs() < 1e-6),
            "{d:?}"
        );
    };
    check(&app);
    tool(&mut app, "save", json!({"path":dir.join("bracket-140.dwprt")}));
    let path = dir.join("bracket.dwdrw");
    tool(&mut app, "drawing_save", json!({"path":path}));
    tool(&mut app, "open", json!({"path":path}));
    app.run_steps(3);
    check(&app);
    app.render().unwrap().save(dir.join("bracket-depth-12.png")).unwrap();
}

#[test]
fn rounded_prism_drawing_replies_without_blocking_native_frames() {
    let _native = native_lock();
    let dir = directory("rounded");
    let mut app = app();
    tool(
        &mut app,
        "sketch_geometry",
        json!({"plane":"top","operations":[
            {"tool":"add","kind":{"type":"line","a":{"u":0,"v":0},"b":{"u":8,"v":0}},"construction":false},
            {"tool":"add","kind":{"type":"line","a":{"u":8,"v":0},"b":{"u":0,"v":6}},"construction":false},
            {"tool":"add","kind":{"type":"line","a":{"u":0,"v":6},"b":{"u":0,"v":0}},"construction":false}
        ]}),
    );
    tool(&mut app, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    let edges: Vec<_> = (1..=9).map(|i| format!("Boss-Extrude1 · Edge {i}")).collect();
    tool(&mut app, "fillet", json!({"edges":edges,"radius":1}));
    tool(&mut app, "save", json!({"path":dir.join("rounded.dwprt")}));
    tool(&mut app, "export_step", json!({"path":dir.join("rounded.step")}));
    let started = Instant::now();
    let (result, longest_frame) = request(&mut app, "make_drawing", json!({"template":"iso-a3"}));
    let drawing_elapsed = started.elapsed();
    assert!(longest_frame < Duration::from_secs(2), "drawing blocked a native frame for {longest_frame:?}");
    assert!(drawing_elapsed < Duration::from_secs(30), "drawing did not return a bounded result");
    assert!(result["isError"].is_boolean(), "a result or plain refusal reaches the caller: {result}");
    let started = Instant::now();
    let summary = tool(&mut app, "model_summary", json!({}));
    let summary_elapsed = started.elapsed();
    assert!(summary_elapsed < Duration::from_secs(2), "summary stayed blocked after drawing");
    assert_eq!(summary["edit_state"]["failed"], false);
    let features = summary["features"].as_array().unwrap();
    assert_eq!(features.len(), 3);
    assert!(features.iter().all(|feature| feature["status"] == "ok"));
    if result["isError"] == false {
        assert_eq!(summary["drawing"]["active"], true);
    }
    eprintln!("rounded prism drawing isError: {}", result["isError"]);
    for (name, value) in [
        ("drawing-response", result),
        ("model-summary", summary),
        (
            "timings",
            json!({"drawing_ms":drawing_elapsed.as_secs_f64()*1000.0,"longest_frame_ms":longest_frame.as_secs_f64()*1000.0,"summary_ms":summary_elapsed.as_secs_f64()*1000.0}),
        ),
    ] {
        std::fs::write(dir.join(format!("{name}.json")), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    app.render().unwrap().save(dir.join("rounded-drawing.png")).unwrap();
}

#[test]
fn slow_drawing_work_does_not_run_on_the_native_frame() {
    let _native = native_lock();
    let kernel = std::sync::Arc::new(gated::Gated::default());
    let mut app = app_with(soliddraft_app::document::Document::new("Slow drawing", kernel.clone()));
    tool(
        &mut app,
        "sketch",
        json!({"plane":"top","shapes":[{"shape":"rectangle","center":[0,0,0],"width":40,"height":30}]}),
    );
    tool(&mut app, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    kernel.rules.lock().unwrap().drawing_delay = Duration::from_secs(3);
    let (reply, longest_frame) = request(&mut app, "make_drawing", json!({"template":"iso-a3"}));
    assert_eq!(reply["isError"], false, "{reply}");
    assert!(longest_frame < Duration::from_secs(2), "drawing blocked a native frame for {longest_frame:?}");
}

#[test]
fn drawing_over_budget_returns_a_plain_refusal_and_keeps_the_part() {
    let _native = native_lock();
    let kernel = std::sync::Arc::new(gated::Gated::default());
    let mut app = app_with(soliddraft_app::document::Document::new("Drawing refusal", kernel.clone()));
    tool(
        &mut app,
        "sketch",
        json!({"plane":"top","shapes":[{"shape":"rectangle","center":[0,0,0],"width":40,"height":30}]}),
    );
    tool(&mut app, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    let before = (app.state().state.doc.model().id(), app.state().state.doc.undo_depth());
    kernel.rules.lock().unwrap().drawing_delay = Duration::from_secs(121);
    let started = Instant::now();
    let (reply, longest_frame) = request(&mut app, "make_drawing", json!({"template":"iso-a3"}));
    assert!(started.elapsed() < Duration::from_secs(120), "the caller waited for the HTTP timeout");
    assert!(longest_frame < Duration::from_secs(2), "drawing blocked a native frame for {longest_frame:?}");
    assert_eq!(reply["isError"], true, "{reply}");
    assert!(reply["content"][0]["text"].as_str().unwrap().contains("100-second time limit"), "{reply}");
    assert!(app.state().state.drawing.session.is_none());
    assert_eq!((app.state().state.doc.model().id(), app.state().state.doc.undo_depth()), before);
    let started = Instant::now();
    tool(&mut app, "model_summary", json!({}));
    assert!(started.elapsed() < Duration::from_secs(2), "summary stayed blocked after the refusal");
}
