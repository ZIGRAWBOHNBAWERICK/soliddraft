//! Sheet exports write the saved Catallax line work.

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::command_tools::board_bracket;
use soliddraft_app::drawing::part::PartContext;
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use soliddraft_drawing::{Command, Drawing, apply_with};

mod gated;

/// The hang guard every test shares: a wait this long means stuck, not slow.
const HANG: std::time::Duration = std::time::Duration::from_secs(600);

fn call(state: &mut AppState, name: &str, arguments: Value) -> Value {
    let caller = Caller::Mcp { client: "test".into() };
    let out = call_checked(&mut AppTools { state }, name, arguments, &caller);
    assert!(!out.is_error, "{name}: {}", out.content);
    serde_json::from_str(&out.content).unwrap()
}

#[test]
fn rebuild_names_a_broken_out_cut_that_no_longer_reaches_the_part() {
    use soliddraft_app::document::Document;
    use soliddraft_drawing::{Boundary, BrokenOut};
    use soliddraft_kernel_api::Point2;
    use soliddraft_kernel_api::drawing::StandardView;
    use std::sync::Arc;

    let kernel = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new());
    let mut state = AppState::new(Document::new("Plate", kernel));
    call(
        &mut state,
        "sketch",
        json!({"plane":"top","shapes":[{"shape":"rectangle","center":[0,0,0],"width":40,"height":30}]}),
    );
    call(&mut state, "extrude", json!({"sketch":"Sketch1","depth":10,"flip":false}));
    call(&mut state, "make_drawing", json!({"template":"iso-a3"}));
    call(&mut state, "drawing_auto_update", json!({"on":false}));
    let ctx = PartContext::new(&state.doc).unwrap();
    let session = state.drawing.session.as_mut().unwrap();
    let view = session
        .sheet()
        .views
        .iter()
        .find(|view| {
            soliddraft_drawing::project::spec_in(&session.doc.drawing, 0, view).is_some_and(|spec| {
                spec.direction == soliddraft_kernel_api::drawing::ViewSpec::standard(StandardView::Top).direction
            })
        })
        .unwrap();
    let name = soliddraft_drawing::project::view_label_in(&session.doc.drawing, session.sheet(), view);
    let view = view.id;
    let command = Command::AddBrokenOut {
        view,
        broken_out: BrokenOut {
            boundary: Boundary::rectangle(Point2::new(-4.0, -4.0), Point2::new(4.0, 4.0)),
            depth: 7.0,
        },
    };
    session.doc.apply(command, Some(&ctx)).unwrap();
    call(&mut state, "drawing_show_part", json!({}));
    call(&mut state, "edit_extrude", json!({"feature":"Boss-Extrude1","depth":5,"flip":false}));

    let caller = Caller::Mcp { client: "test".into() };
    let out = call_checked(&mut AppTools { state: &mut state }, "drawing_rebuild", json!({}), &caller);
    assert!(!out.is_error, "{}", out.content);
    let reply: Value = serde_json::from_str(&out.content).unwrap();
    let cause = "the cutting plane is behind the whole body";
    assert!(reply["done"].as_str().is_some_and(|message| message.contains(&name) && message.contains(cause)));
    assert_eq!(reply["drawing"]["up_to_date"], false);
    let refusal = &reply["drawing"]["refused_views"][0];
    assert_eq!(refusal["name"], name);
    let refused_view = reply["drawing"]["sheets"][0]["views"]
        .as_array()
        .unwrap()
        .iter()
        .find(|view| view["id"] == refusal["id"])
        .unwrap();
    assert_eq!(refused_view["up_to_date"], false);
    assert!(refusal["cause"].as_str().is_some_and(|message| message.contains(cause)));
    assert!(state.drawing.session.as_ref().unwrap().has_issues());
    let status = soliddraft_app::drawing::status(&state).unwrap();
    assert!(status.headline.0.contains(&name) && status.headline.0.contains(cause));

    let session = state.drawing.session.as_mut().unwrap();
    let error = session.doc.apply(Command::SetViewExcluded { view, excluded: true }, None).unwrap_err();
    assert!(error.contains(&name) && error.contains(cause), "{error}");
    assert!(!session.doc.drawing.view(view).unwrap().excluded_from_update);
    let report = call(&mut state, "drawing_rebuild", json!({}));
    assert!(report["done"].as_str().is_some_and(|message| message.contains(&name) && message.contains(cause)));

    use soliddraft_app::drawing::{DrawingSession, document::DrawingDoc};
    use soliddraft_drawing::file::{FileMeta, from_bytes, to_bytes};
    let drawing = &state.drawing.session.as_ref().unwrap().doc.drawing;
    let bytes = to_bytes(drawing, &FileMeta::new("Catallax", drawing.meta.units)).unwrap();
    let (reopened, _) = from_bytes(&bytes).unwrap();
    state.drawing.session = Some(DrawingSession::new(DrawingDoc::new(reopened)));
    let session = state.drawing.session.as_ref().unwrap();
    assert!(session.has_issues());
    assert!(!session.is_up_to_date());
    assert!(soliddraft_app::drawing::status(&state).unwrap().headline.0.contains("needs a rebuild"));
}

/// How many views Catallax drew as hidden-line views, in the calls `kernel` logged since
/// it was last asked. A view may say it is proved only when its projection came from
/// one: a preview must never be labelled as proved.
fn certified_views(kernel: &soliddraft_kernel_catallax::CatallaxKernel) -> usize {
    let calls = kernel.take_diagnostics().unwrap();
    calls["calls"].as_array().unwrap().iter().filter(|c| exact_view(c)).count()
}

fn finish_viewport_work(state: &mut AppState) {
    let projection = soliddraft_tessellation::Projection::Orthographic { pixels_per_mm: 1.0 };
    let started = std::time::Instant::now();
    while state.doc.update_display(&projection, 0.016) && state.doc.display_quality().pending() {
        assert!(started.elapsed() < HANG, "the viewport worker did not answer");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(!state.doc.display_quality().pending(), "the viewport worker stopped with pending work");
}

fn assert_no_drawing_queries(kernel: &soliddraft_kernel_catallax::CatallaxKernel) {
    let log = kernel.take_diagnostics().unwrap();
    for call in log["calls"].as_array().unwrap() {
        assert!(
            !matches!(
                call["operation"].as_str(),
                Some(
                    "view_curves"
                        | "section_view"
                        | "silhouette_curves"
                        | "section_curves"
                        | "edge_curve"
                        | "face_loops"
                        | "lines_and_arcs"
                        | "preview"
                        | "preview_edges"
                        | "tessellate"
                        | "tessellate_edges"
                )
            ),
            "export ran a drawing query: {call}"
        );
    }
}

/// A top-level view call whose operation log holds Catallax's hidden-line view.
fn exact_view(call: &Value) -> bool {
    call["operation"] == "view_curves"
        && call["nested"] == false
        && call["operation_log"]["operations"]
            .as_array()
            .is_some_and(|log| log.iter().any(|e| e["operation"] == "view_curves" && e["status"] == "answered"))
}

#[test]
fn drawings_never_pair_requested_definitions_with_a_retained_body() {
    use soliddraft_app::document::{Command as PartCommand, Document, ExtrudeParams, FilletParams, ProfileKind};
    use soliddraft_app::drag::Dimension;
    use soliddraft_app::state::DragStep;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    let settle = |state: &mut AppState| {
        let deadline = Instant::now() + HANG;
        while state.doc.proof_pending() {
            state.doc.poll_proof();
            assert!(Instant::now() < deadline, "the part proof did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
        soliddraft_app::drawing::follow_part(state);
    };
    let rejected_rebuild = |state: &mut AppState| {
        let caller = Caller::Mcp { client: "test".into() };
        let out = call_checked(&mut AppTools { state }, "drawing_rebuild", json!({}), &caller);
        assert!(out.is_error, "an unproved part must not become the drawing's source");
    };
    let dir = std::env::temp_dir().join(format!("soliddraft-drawing-revision-proof-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let kernel = Arc::new(gated::Gated::default());
    kernel.rules.lock().unwrap().refuse_fillet_radii.push(50.0);
    let mut doc = Document::new("Proof part", kernel);
    let mut params = ExtrudeParams {
        profile: ProfileKind::Rectangle { width: 40.0, height: 30.0 },
        depth: 10.0,
        ..Default::default()
    };
    let block = doc.apply(PartCommand::AddExtrude { params: params.clone(), via_assistant: false }).unwrap().unwrap();
    let mut state = AppState::new(doc);
    call(&mut state, "make_drawing", json!({ "template": "iso-a3" }));
    soliddraft_app::drawing::follow_part(&mut state);
    let part = PartContext::new(&state.doc).unwrap();
    let mut certified = state.drawing.session.as_ref().unwrap().doc.drawing.clone();
    certified.settings.automatic_update = true;
    apply_with(&mut certified, &Command::UpdateFromModel, &part.part()).unwrap();
    state.drawing.session.as_mut().unwrap().doc.drawing = certified.clone();
    call(&mut state, "drawing_export_svg", json!({ "path": dir.join("proved.svg") }));
    assert!(dir.join("proved.svg").is_file());

    state.drag_dimension(DragStep::Start { feature: block, dimension: Dimension::Depth });
    state.drag_dimension(DragStep::Move(14.0));
    soliddraft_app::drawing::follow_part(&mut state);
    rejected_rebuild(&mut state);
    assert_eq!(state.drawing.session.as_ref().unwrap().doc.drawing, certified);
    state.drag_dimension(DragStep::Cancel);

    params.depth = 14.0;
    state.doc.apply_interactive(PartCommand::EditExtrude { id: block, params }).unwrap();
    assert!(state.doc.proof_pending());
    assert_ne!(state.doc.model().semantic_hash().as_str(), certified.model.pinned.as_str());
    assert!(PartContext::new(&state.doc).is_none());
    soliddraft_app::drawing::follow_part(&mut state);
    rejected_rebuild(&mut state);
    let session = state.drawing.session.as_ref().unwrap();
    assert_eq!(session.doc.drawing, certified);
    assert_eq!(session.doc.seen_digest, certified.model.pinned);
    let pending_path = dir.join("pending-pinned.svg");
    call(&mut state, "drawing_export_svg", json!({ "path": pending_path }));
    assert_eq!(std::fs::read(pending_path).unwrap(), std::fs::read(dir.join("proved.svg")).unwrap());

    settle(&mut state);
    let part = PartContext::new(&state.doc).unwrap();
    assert_eq!(part.digest.as_str(), state.doc.model().semantic_hash().as_str());
    assert_eq!(part.kernel.mass_properties(part.body).unwrap().volume, 16_800.0);
    assert_eq!(state.drawing.session.as_ref().unwrap().doc.drawing.model.pinned, part.digest);
    let edge = part.topology.edges.iter().find(|edge| (edge.midpoint.z - 14.0).abs() < 1e-9).unwrap().id;
    let reference = state.doc.edge_ref(edge).unwrap();
    let fillet = state
        .doc
        .apply(PartCommand::AddFillet { params: FilletParams { edges: vec![reference.clone()], radius: 1.0 } })
        .unwrap()
        .unwrap();
    soliddraft_app::drawing::follow_part(&mut state);
    let before_failure = state.drawing.session.as_ref().unwrap().doc.drawing.clone();
    let retained_body = state.doc.display_body();
    state
        .doc
        .apply_interactive(PartCommand::EditFillet {
            id: fillet,
            params: FilletParams { edges: vec![reference], radius: 50.0 },
        })
        .unwrap();
    settle(&mut state);
    assert!(state.doc.proof_state().failed);
    assert_eq!(state.doc.display_body(), retained_body);
    assert!(
        matches!(&state.doc.feature(fillet).unwrap().status, soliddraft_app::document::FeatureStatus::Error(message) if message.contains("the scenario refuses a radius of 50"))
    );
    assert!(PartContext::new(&state.doc).is_none());
    rejected_rebuild(&mut state);
    let session = state.drawing.session.as_ref().unwrap();
    assert_eq!(session.doc.drawing, before_failure);
    assert_eq!(session.doc.seen_digest, before_failure.model.pinned);
    std::fs::remove_dir_all(dir).unwrap();
}

fn assert_rod_circle(path: &std::path::Path, drawing: &Drawing) {
    use kurbo::{BezPath, PathEl, Rect, Shape};
    use quick_xml::events::Event;

    let (view, radius) = drawing.sheets[0]
        .views
        .iter()
        .find_map(|view| {
            let spec = soliddraft_drawing::project::view_spec(drawing, view.id).ok()?;
            (spec.direction == soliddraft_kernel_api::Vec3::Z)
                .then(|| (view, soliddraft_drawing::project::scale_in(drawing, 0, view)))
        })
        .expect("the rod has a top view");
    let svg = std::fs::read_to_string(path).unwrap();
    let mut xml = quick_xml::Reader::from_str(&svg);
    let mut centre = None;
    let mut rim = None::<Rect>;
    loop {
        match xml.read_event().unwrap() {
            Event::Start(el) if el.name().as_ref() == b"svg" => {
                let view_box = el.try_get_attribute("viewBox").unwrap().unwrap();
                let values: Vec<f64> = std::str::from_utf8(&view_box.value)
                    .unwrap()
                    .split_whitespace()
                    .map(|v| v.parse().unwrap())
                    .collect();
                centre = Some(kurbo::Point::new(view.position.u, values[3] - view.position.v));
            }
            Event::Empty(el) if el.name().as_ref() == b"path" => {
                let Some(accuracy) = el.try_get_attribute("data-accuracy").unwrap() else { continue };
                if accuracy.value.as_ref() == b"polygon" {
                    continue;
                }
                let d = el.try_get_attribute("d").unwrap().unwrap();
                let path = BezPath::from_svg(std::str::from_utf8(&d.value).unwrap()).unwrap();
                let bbox = path.bounding_box();
                let c = centre.expect("the sheet has a viewBox");
                let target = Rect::new(c.x - radius, c.y - radius, c.x + radius, c.y + radius).inflate(0.005, 0.005);
                if path.elements().iter().any(|el| matches!(el, PathEl::CurveTo(..)))
                    && target.contains(bbox.origin())
                    && target.contains(kurbo::Point::new(bbox.x1, bbox.y1))
                {
                    rim = Some(rim.map_or(bbox, |other| other.union(bbox)));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let bounds = rim.expect("the 2 mm rod rim is drawn with curved paths");
    let c = centre.unwrap();
    for (got, want) in
        [(bounds.x0, c.x - radius), (bounds.x1, c.x + radius), (bounds.y0, c.y - radius), (bounds.y1, c.y + radius)]
    {
        assert!((got - want).abs() < 0.005, "rod rim {bounds:?} at {c:?}, radius {radius}");
    }
}

#[test]
fn a_rod_cut_exports_the_saved_catallax_views() {
    use soliddraft_app::document::Document;
    use soliddraft_kernel_catallax::CatallaxKernel;
    use std::sync::Arc;

    let dir = std::env::temp_dir().join(format!("soliddraft-drawing-quality-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let kernel = Arc::new(CatallaxKernel::with_operation_log());
    let mut state = AppState::new(Document::new("Tilted rod cut", kernel.clone()));
    let scenario: Value = serde_json::from_str(include_str!("../../../ci/scenarios/rod-cut-28deg.json")).unwrap();
    for step in scenario["steps"].as_array().unwrap() {
        let tool = step["tool"].as_str().unwrap();
        if tool == "save" {
            break;
        }
        call(&mut state, tool, step["arguments"].clone());
    }
    finish_viewport_work(&mut state);
    kernel.take_diagnostics().unwrap();
    call(&mut state, "make_drawing", json!({ "template": "iso-a3" }));
    let shown = state.drawing.session.as_ref().unwrap().doc.drawing.clone();
    let views = shown.sheets[0].views.len();
    assert_eq!(views, 4);
    assert!(shown.sheets[0].views.iter().all(|v| v.geometry.catallax && !v.geometry.unproved));
    assert_eq!(certified_views(&kernel), views);
    for (tool, extension) in
        [("drawing_export_svg", "svg"), ("drawing_export_pdf", "pdf"), ("drawing_export_dxf", "dxf")]
    {
        let path = dir.join(format!("rod.{extension}"));
        let caller = Caller::Mcp { client: "test".into() };
        let out = call_checked(&mut AppTools { state: &mut state }, tool, json!({ "path": path }), &caller);
        assert!(!out.is_error, "{tool}: {}", out.content);
        let result: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(result["views"], json!(views), "{tool}: {result}");
        assert_eq!(result["method"], "catallax");
        assert!(path.is_file());
    }
    assert_rod_circle(&dir.join("rod.svg"), &shown);
    assert_eq!(state.drawing.session.as_ref().unwrap().doc.drawing, shown);

    assert_no_drawing_queries(&kernel);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_legacy_rod_cache_refuses_export_until_explicit_rebuild() {
    const SENTINEL_BOX: [f64; 4] = [-12345.5, -12345.5, 12345.5, 12345.5];
    use soliddraft_app::document::Document;
    use soliddraft_document::manifest::{self, Entry, Package};
    use soliddraft_kernel_catallax::CatallaxKernel;
    use std::sync::Arc;

    let dir = std::env::temp_dir().join(format!("soliddraft-drawing-old-quality-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut state = AppState::new(Document::new("Old rod cache", Arc::new(CatallaxKernel::new())));
    let scenario: Value = serde_json::from_str(include_str!("../../../ci/scenarios/rod-cut-28deg.json")).unwrap();
    for step in scenario["steps"].as_array().unwrap() {
        let tool = step["tool"].as_str().unwrap();
        if tool == "save" {
            break;
        }
        call(&mut state, tool, step["arguments"].clone());
    }
    call(&mut state, "save", json!({ "path": dir.join("rod.dwprt") }));
    call(&mut state, "make_drawing", json!({ "template": "iso-a3" }));
    let path = dir.join("old.dwdrw");
    call(&mut state, "drawing_save", json!({ "path": path }));
    let bytes = std::fs::read(&path).unwrap();
    let mut package = Package::from_bytes(&bytes).unwrap();
    let mut entries = Vec::new();
    for name in package.names().into_iter().filter(|name| name != manifest::MANIFEST) {
        let mut bytes = package.read(&name).unwrap().unwrap();
        if name.starts_with("cache/views/") {
            let mut geometry: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(geometry.as_object_mut().unwrap().remove("catallax"), Some(json!(true)));
            // A sentinel no projection makes: a view still carrying it was not drawn again.
            geometry["bbox"] = json!(SENTINEL_BOX);
            geometry["curves"] = json!([]);
            geometry.as_object_mut().unwrap().remove("unproved");
            bytes = serde_json::to_vec(&geometry).unwrap();
        }
        entries.push(Entry::new(name, bytes));
    }
    std::fs::write(&path, manifest::write_package(package.manifest().unwrap(), &entries).unwrap()).unwrap();
    drop(state);

    let kernel = Arc::new(CatallaxKernel::with_operation_log());
    let mut reopened = AppState::new(Document::new("Part1", kernel.clone()));
    call(&mut reopened, "open", json!({ "path": path }));
    finish_viewport_work(&mut reopened);
    let shown = reopened.drawing.session.as_ref().unwrap().doc.drawing.clone();
    assert_eq!(shown.sheets[0].views.len(), 4);
    assert!(shown.sheets[0].views.iter().all(|v| v.geometry.bbox == SENTINEL_BOX));
    assert!(shown.sheets[0].views.iter().all(|v| !v.geometry.unproved && !v.geometry.coarse));
    assert_eq!(certified_views(&kernel), 0, "opening drew no view");
    assert_eq!(shown.model.pinned, PartContext::new(&reopened.doc).unwrap().digest);
    let session = reopened.drawing.session.as_ref().unwrap();
    assert!(!session.is_up_to_date());
    assert_eq!(session.doc.stale_views().len(), 4);
    soliddraft_app::drawing::follow_part(&mut reopened);
    assert_eq!(reopened.drawing.session.as_ref().unwrap().doc.drawing, shown);

    kernel.take_diagnostics().unwrap();
    let caller = Caller::Mcp { client: "test".into() };
    for (tool, extension) in
        [("drawing_export_svg", "svg"), ("drawing_export_pdf", "pdf"), ("drawing_export_dxf", "dxf")]
    {
        let path = dir.join(format!("legacy.{extension}"));
        let out = call_checked(&mut AppTools { state: &mut reopened }, tool, json!({ "path": path }), &caller);
        assert!(out.is_error && out.content.contains("Redraw it once"), "{tool}: {}", out.content);
        assert!(!path.exists());
    }
    assert_no_drawing_queries(&kernel);
    assert_eq!(reopened.drawing.session.as_ref().unwrap().doc.drawing, shown);

    let view = shown.sheets[0].views[0].id;
    let session = reopened.drawing.session.as_mut().unwrap();
    session.doc.apply(Command::SetViewExcluded { view, excluded: true }, None).unwrap();
    let path = dir.join("frozen.svg");
    let out =
        call_checked(&mut AppTools { state: &mut reopened }, "drawing_export_svg", json!({ "path": path }), &caller);
    assert!(out.is_error && out.content.contains("Include the view in automatic update, then rebuild."));
    assert!(!path.exists());
    assert_no_drawing_queries(&kernel);
    reopened
        .drawing
        .session
        .as_mut()
        .unwrap()
        .doc
        .apply(Command::SetViewExcluded { view, excluded: false }, None)
        .unwrap();
    call(&mut reopened, "drawing_rebuild", json!({}));
    assert_eq!(certified_views(&kernel), 4);
    let path = dir.join("reopened.svg");
    call(&mut reopened, "drawing_export_svg", json!({ "path": path }));
    assert_no_drawing_queries(&kernel);
    assert_rod_circle(&path, &reopened.drawing.session.as_ref().unwrap().doc.drawing);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn single_sheet_exports_check_only_the_written_views() {
    use soliddraft_app::drawing::{DrawingSession, document::DrawingDoc};
    use soliddraft_drawing::file::{FileMeta, from_bytes, to_bytes};
    use soliddraft_drawing::{Sheet, ViewKind, library};

    let dir = std::env::temp_dir().join(format!("soliddraft-drawing-sheet-scope-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut state = AppState::new(board_bracket());
    call(&mut state, "make_drawing", json!({ "template": "iso-a3" }));
    let session = state.drawing.session.as_mut().unwrap();
    let drawing = &mut session.doc.drawing;
    let parent = drawing.sheets[0].views.iter().find_map(|v| v.parent()).unwrap();
    let pos = drawing.sheets[0].views.iter().position(|v| v.id == parent).unwrap();
    let parent = drawing.sheets[0].views.remove(pos);
    let mut unrelated = parent.clone();
    unrelated.id = drawing.next_view_id();
    unrelated.geometry.catallax = false;
    unrelated.kind = ViewKind::Section {
        parent: parent.id,
        line: vec![],
        flip: false,
        label: "A".into(),
        model_line: vec![],
        hatch: None,
    };
    let mut other = Sheet::from_template("Other", &library::template("iso-a3").template());
    other.views.extend([parent, unrelated]);
    drawing.sheets.push(other);

    call(&mut state, "drawing_export_svg", json!({ "path": dir.join("active.svg") }));
    call(&mut state, "drawing_export_dxf", json!({ "path": dir.join("active.dxf") }));
    let caller = Caller::Mcp { client: "test".into() };
    let pdf = dir.join("all.pdf");
    let refused =
        call_checked(&mut AppTools { state: &mut state }, "drawing_export_pdf", json!({ "path": pdf }), &caller);
    assert!(refused.is_error, "{}", refused.content);
    assert!(!pdf.exists());

    let session = state.drawing.session.as_mut().unwrap();
    let old = soliddraft_drawing::ContentHash::of_bytes(b"older part");
    session.doc.drawing.sheets[1].views.pop();
    let frozen = &mut session.doc.drawing.sheets[1].views[0];
    frozen.excluded_from_update = true;
    frozen.geometry.source_digest = old.as_str().into();
    let drawing = &session.doc.drawing;
    let pinned = drawing.model.pinned.clone();
    let bytes = to_bytes(drawing, &FileMeta::new("Catallax", drawing.meta.units)).unwrap();
    let (reopened, _) = from_bytes(&bytes).unwrap();
    let mut session = DrawingSession::new(DrawingDoc::new(reopened));
    session.sheet = 1;
    assert!(session.is_up_to_date());
    state.drawing.session = Some(session);
    for (tool, extension) in [("drawing_export_svg", "svg"), ("drawing_export_dxf", "dxf")] {
        let path = dir.join(format!("frozen.{extension}"));
        let report = call(&mut state, tool, json!({ "path": path }));
        assert_eq!(report["pinned_revision"], pinned.short(), "{report}");
        assert_eq!(report["revision"], old.as_str(), "{report}");
        assert!(path.is_file());
    }
    let report = call(&mut state, "drawing_export_pdf", json!({ "path": pdf }));
    let mut revisions = [old, pinned];
    revisions.sort();
    assert_eq!(report["revision"], Value::Null, "{report}");
    assert_eq!(report["view_revisions"], json!(revisions), "{report}");

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pdf_and_dxf_exports_report_the_single_approximate_hatch() {
    use soliddraft_app::drawing::{DrawingSession, document::DrawingDoc};
    use soliddraft_drawing::Polygon;
    use soliddraft_kernel_api::Point2;

    let dir = std::env::temp_dir().join(format!("soliddraft-fitted-hatch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut drawing = soliddraft_drawing::fixture::bracket_a3();
    for view in &mut drawing.sheets[0].views {
        view.geometry.catallax = true;
        view.geometry.curves.clear();
    }
    drawing.sheets[0].views[0].geometry.hatch.push(Polygon {
        outer: vec![Point2::new(-5.0, -5.0), Point2::new(5.0, -5.0), Point2::new(5.0, 5.0), Point2::new(-5.0, 5.0)],
        approximate: true,
        ..Default::default()
    });
    let mut state = AppState::new(board_bracket());
    state.drawing.session = Some(DrawingSession::new(DrawingDoc::new(drawing)));
    for (tool, extension) in [("drawing_export_pdf", "pdf"), ("drawing_export_dxf", "dxf")] {
        let path = dir.join(format!("hatch.{extension}"));
        let report = call(&mut state, tool, json!({ "path": path }));
        assert_eq!(report["accuracy"]["approximate"], 1, "{tool}: {report}");
        assert!(path.is_file());
    }
    std::fs::remove_dir_all(dir).unwrap();
}
