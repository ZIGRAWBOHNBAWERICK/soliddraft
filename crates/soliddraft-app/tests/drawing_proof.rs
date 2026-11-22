//! An exported drawing never comes from a preview. The drawing tab shows views drawn
//! from a display mesh and marked unproved, which the drawing writers refuse. Exporting
//! draws the views again from Catallax's hidden-line views, on a copy, and leaves the
//! drawing on screen as it was.

use serde_json::{Value, json};
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::command_tools::board_bracket;
use soliddraft_app::drawing::part::PartContext;
use soliddraft_app::state::AppState;
use soliddraft_assistant::Caller;
use soliddraft_assistant::tools::call_checked;
use soliddraft_drawing::display::layout_sheet;
use soliddraft_drawing::export::{self, Format};
use soliddraft_drawing::{Command, Drawing, DrawingError, apply_with};

mod gated;

/// The hang guard every test shares: a wait this long means stuck, not slow.
const HANG: std::time::Duration = std::time::Duration::from_secs(600);

fn call(state: &mut AppState, name: &str, arguments: Value) {
    let caller = Caller::Mcp { client: "test".into() };
    let out = call_checked(&mut AppTools { state }, name, arguments, &caller);
    assert!(!out.is_error, "{name}: {}", out.content);
}

fn previews(drawing: &Drawing) -> Vec<bool> {
    drawing.sheets.iter().flat_map(|s| &s.views).map(|v| v.geometry.unproved).collect()
}

/// How many of a drawing's views say they are proved.
fn proved_views(drawing: &Drawing) -> usize {
    previews(drawing).into_iter().filter(|preview| !preview).count()
}

/// How many views Catallax drew as hidden-line views, in the calls `kernel` logged since
/// it was last asked. A view may say it is proved only when its projection came from
/// one: a preview must never be labelled as proved.
fn certified_views(kernel: &soliddraft_kernel_catallax::CatallaxKernel) -> usize {
    let calls = kernel.take_diagnostics().unwrap();
    calls["calls"].as_array().unwrap().iter().filter(|c| exact_view(c)).count()
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
fn an_exported_drawing_never_comes_from_a_preview() {
    let dir = std::env::temp_dir().join(format!("soliddraft-drawing-proof-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut state = AppState::new(board_bracket());
    call(&mut state, "make_drawing", json!({ "template": "iso-a3" }));

    // The screen shows previews, and no writer takes them as they are.
    let shown = state.drawing.session.as_ref().unwrap().doc.drawing.clone();
    assert!(!previews(&shown).is_empty() && previews(&shown).iter().all(|p| *p), "{:?}", previews(&shown));
    let list = layout_sheet(&shown, 0);
    for format in [Format::Pdf, Format::Dxf, Format::Svg] {
        assert!(matches!(export::bytes(&list, format, "Part1"), Err(DrawingError::Unproved)), "{format:?}");
    }

    // The same views drawn by Catallax are proven and can be written.
    let part = PartContext::new(&state.doc).unwrap();
    let mut certified = shown.clone();
    apply_with(&mut certified, &Command::UpdateFromModel, &part.certified_part()).unwrap();
    assert!(previews(&certified).iter().all(|p| !p), "{:?}", previews(&certified));
    assert!(export::bytes(&layout_sheet(&certified, 0), Format::Svg, "Part1").is_ok());

    // So exporting succeeds only by drawing the views again through Catallax, and the
    // drawing on screen keeps its previews.
    for (tool, file) in
        [("drawing_export_pdf", "sheet.pdf"), ("drawing_export_dxf", "sheet.dxf"), ("drawing_export_svg", "sheet.svg")]
    {
        call(&mut state, tool, json!({ "path": dir.join(file) }));
        assert!(dir.join(file).is_file(), "{tool} wrote no {file}");
    }
    assert_eq!(state.drawing.session.as_ref().unwrap().doc.drawing, shown);
    let _ = std::fs::remove_dir_all(&dir);
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
    apply_with(&mut certified, &Command::UpdateFromModel, &part.certified_part()).unwrap();
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
    let caller = Caller::Mcp { client: "test".into() };
    let refused = call_checked(
        &mut AppTools { state: &mut state },
        "drawing_export_svg",
        json!({ "path": pending_path }),
        &caller,
    );
    assert!(refused.is_error);
    assert!(!pending_path.exists());

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
fn a_certified_coarse_rod_cut_is_redrawn_at_fine_quality_for_export() {
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
    kernel.take_diagnostics().unwrap();
    call(&mut state, "make_drawing", json!({ "template": "iso-a3" }));
    let shown = state.drawing.session.as_ref().unwrap().doc.drawing.clone();
    let views = shown.sheets[0].views.len();
    assert_eq!(views, 4);
    // The screen draws at coarse quality from a display mesh. A view says it is proved
    // only when Catallax drew it.
    assert!(shown.sheets[0].views.iter().all(|v| v.geometry.coarse));
    assert_eq!(certified_views(&kernel), proved_views(&shown), "{:?}", previews(&shown));

    let part = PartContext::new(&state.doc).unwrap();
    let mut fine = shown.clone();
    apply_with(&mut fine, &Command::UpdateFromModel, &part.certified_part()).unwrap();
    assert_eq!(certified_views(&kernel), views, "Catallax drew each fine view");
    assert!(fine.sheets[0].views.iter().all(|v| !v.geometry.unproved && !v.geometry.coarse));
    for (tool, extension) in
        [("drawing_export_svg", "svg"), ("drawing_export_pdf", "pdf"), ("drawing_export_dxf", "dxf")]
    {
        let path = dir.join(format!("rod.{extension}"));
        let caller = Caller::Mcp { client: "test".into() };
        let out = call_checked(&mut AppTools { state: &mut state }, tool, json!({ "path": path }), &caller);
        assert!(!out.is_error, "{tool}: {}", out.content);
        let result: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(result["views"], json!(views), "{tool}: {result}");
        assert_eq!(result["redrawn_from_certified_mesh"], 0);
        assert_eq!(result["method"], "catallax");
        assert!(path.is_file());
    }
    assert_rod_circle(&dir.join("rod.svg"), &shown);
    assert_eq!(state.drawing.session.as_ref().unwrap().doc.drawing, shown);

    state.drawing.session.as_mut().unwrap().doc.drawing = fine;
    kernel.take_diagnostics().unwrap();
    call(&mut state, "drawing_export_svg", json!({ "path": dir.join("fine.svg") }));
    assert_eq!(certified_views(&kernel), views);
    assert_rod_circle(&dir.join("fine.svg"), &shown);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_saved_rod_cache_without_quality_is_regenerated_after_reopening() {
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
            assert_eq!(geometry.as_object_mut().unwrap().remove("coarse"), Some(json!(true)));
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
    let shown = reopened.drawing.session.as_ref().unwrap().doc.drawing.clone();
    assert_eq!(shown.sheets[0].views.len(), 4);
    // Opening shows the cached views as they were saved, sentinel and labels included: it
    // draws nothing and claims no proof the cache did not record. A cache of unknown
    // quality is drawn again only when certified line work is asked for, below.
    assert!(shown.sheets[0].views.iter().all(|v| v.geometry.bbox == SENTINEL_BOX));
    assert!(shown.sheets[0].views.iter().all(|v| !v.geometry.unproved && !v.geometry.coarse));
    assert_eq!(certified_views(&kernel), 0, "opening drew no view");
    assert_eq!(shown.model.pinned, PartContext::new(&reopened.doc).unwrap().digest);
    soliddraft_app::drawing::follow_part(&mut reopened);
    assert_eq!(reopened.drawing.session.as_ref().unwrap().doc.drawing, shown);

    let mut exported = Vec::new();
    for (tool, extension) in
        [("drawing_export_svg", "svg"), ("drawing_export_pdf", "pdf"), ("drawing_export_dxf", "dxf")]
    {
        kernel.take_diagnostics().unwrap();
        let path = dir.join(format!("reopened.{extension}"));
        let caller = Caller::Mcp { client: "test".into() };
        let out = call_checked(&mut AppTools { state: &mut reopened }, tool, json!({ "path": path }), &caller);
        assert!(!out.is_error, "{tool}: {}", out.content);
        let result: Value = serde_json::from_str(&out.content).unwrap();
        let calls = kernel.take_diagnostics().unwrap();
        let fine_requests = calls["calls"].as_array().unwrap().iter().filter(|c| exact_view(c)).count();
        assert_eq!(result["redrawn_from_certified_mesh"], 0);
        assert_eq!(result["method"], "catallax");
        exported.push((result["views"].clone(), fine_requests));
        assert!(path.is_file());
    }
    assert_eq!(reopened.drawing.session.as_ref().unwrap().doc.drawing, shown);
    assert_rod_circle(&dir.join("reopened.svg"), &shown);
    std::fs::remove_dir_all(dir).unwrap();
    assert!(exported.iter().all(|(views, fine)| *views == json!(4) && *fine == 4), "{exported:?}");
}

#[test]
fn single_sheet_exports_ignore_other_views_but_read_cross_sheet_parents() {
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
    std::fs::remove_dir_all(dir).unwrap();
}
