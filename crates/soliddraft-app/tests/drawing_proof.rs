//! An exported drawing never comes from a preview. The drawing tab shows views drawn
//! from Catallax's unproved preview mesh, which the drawing writers refuse. Exporting
//! draws the views again from the certified mesh, on a copy, and leaves the drawing on
//! screen as it was.

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

    // The same views drawn from the certified mesh are proven and can be written.
    let part = PartContext::new(&state.doc).unwrap();
    let mut certified = shown.clone();
    apply_with(&mut certified, &Command::UpdateFromModel, &part.certified_part()).unwrap();
    assert!(previews(&certified).iter().all(|p| !p), "{:?}", previews(&certified));
    assert!(export::bytes(&layout_sheet(&certified, 0), Format::Svg, "Part1").is_ok());

    // So exporting succeeds only by drawing the views again from the certified mesh,
    // and the drawing on screen keeps its previews.
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
    use soliddraft_kernel_catallax::CatallaxKernel;
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
    let mut doc = Document::new("Proof part", Arc::new(CatallaxKernel::new()));
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
    let proved_svg = std::fs::read(dir.join("proved.svg")).unwrap();

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
    // Already-certified views remain deliberately pinned; export does not recertify
    // them against the pending definition or silently substitute another body.
    call(&mut state, "drawing_export_svg", json!({ "path": dir.join("pending-pinned.svg") }));
    assert_eq!(std::fs::read(dir.join("pending-pinned.svg")).unwrap(), proved_svg);

    settle(&mut state);
    let part = PartContext::new(&state.doc).unwrap();
    assert_eq!(part.digest.as_str(), state.doc.model().semantic_hash().as_str());
    assert_eq!(part.kernel.mass_properties(part.body).unwrap().volume, 16_800.0);
    assert_eq!(state.drawing.session.as_ref().unwrap().doc.drawing.model.pinned, part.digest);
    let edge = part.topology.edges.iter().find(|edge| (edge.midpoint.z - 14.0).abs() < 1e-9).unwrap().id;
    let reference = state.doc.edge_ref(edge).unwrap();
    let fillet = state
        .doc
        .apply(PartCommand::AddFillet {
            params: FilletParams { edges: vec![reference.clone()], radius: 1.0, overflow: None, laws: Vec::new() },
        })
        .unwrap()
        .unwrap();
    soliddraft_app::drawing::follow_part(&mut state);
    let before_failure = state.drawing.session.as_ref().unwrap().doc.drawing.clone();
    state
        .doc
        .apply_interactive(PartCommand::EditFillet {
            id: fillet,
            params: FilletParams { edges: vec![reference], radius: 50.0, overflow: None, laws: Vec::new() },
        })
        .unwrap();
    settle(&mut state);
    assert!(state.doc.proof_state().failed);
    assert!(PartContext::new(&state.doc).is_none());
    rejected_rebuild(&mut state);
    let session = state.drawing.session.as_ref().unwrap();
    assert_eq!(session.doc.drawing, before_failure);
    assert_eq!(session.doc.seen_digest, before_failure.model.pinned);
    std::fs::remove_dir_all(dir).unwrap();
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
    call(&mut state, "make_drawing", json!({ "template": "iso-a3" }));
    let shown = state.drawing.session.as_ref().unwrap().doc.drawing.clone();
    let views = shown.sheets[0].views.len();
    assert_eq!(views, 4);
    // Catallax refuses this body's preview, so the screen uses a proved coarse mesh.
    assert!(shown.sheets[0].views.iter().all(|v| !v.geometry.unproved && v.geometry.coarse));

    let part = PartContext::new(&state.doc).unwrap();
    let mut fine = shown.clone();
    apply_with(&mut fine, &Command::UpdateFromModel, &part.certified_part()).unwrap();
    assert!(fine.sheets[0].views.iter().all(|v| !v.geometry.unproved && !v.geometry.coarse));
    let fine_svg = export::bytes(&layout_sheet(&fine, 0), Format::Svg, &fine.meta.title).unwrap();

    for (tool, extension) in
        [("drawing_export_svg", "svg"), ("drawing_export_pdf", "pdf"), ("drawing_export_dxf", "dxf")]
    {
        let path = dir.join(format!("rod.{extension}"));
        let caller = Caller::Mcp { client: "test".into() };
        let out = call_checked(&mut AppTools { state: &mut state }, tool, json!({ "path": path }), &caller);
        assert!(!out.is_error, "{tool}: {}", out.content);
        let result: Value = serde_json::from_str(&out.content).unwrap();
        assert_eq!(result["redrawn_from_certified_mesh"], json!(views), "{tool}: {result}");
        assert!(path.is_file());
    }
    assert_eq!(std::fs::read(dir.join("rod.svg")).unwrap(), fine_svg);
    assert_eq!(state.drawing.session.as_ref().unwrap().doc.drawing, shown);

    state.drawing.session.as_mut().unwrap().doc.drawing = fine;
    let calls = kernel.take_diagnostics().unwrap();
    assert!(calls["calls"].as_array().unwrap().iter().any(|c| c["operation"] == "view_curves"));
    call(&mut state, "drawing_export_svg", json!({ "path": dir.join("fine.svg") }));
    let calls = kernel.take_diagnostics().unwrap();
    assert!(
        !calls["calls"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| matches!(c["operation"].as_str(), Some("view_curves" | "tessellate")))
    );
    assert_eq!(std::fs::read(dir.join("fine.svg")).unwrap(), fine_svg);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_saved_rod_cache_without_quality_is_regenerated_after_reopening() {
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

    // Generate the cache format written before quality was stored separately from proof.
    let bytes = std::fs::read(&path).unwrap();
    let mut package = Package::from_bytes(&bytes).unwrap();
    let mut entries = Vec::new();
    for name in package.names().into_iter().filter(|name| name != manifest::MANIFEST) {
        let mut bytes = package.read(&name).unwrap().unwrap();
        if name.starts_with("cache/views/") {
            let mut geometry: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(geometry.as_object_mut().unwrap().remove("coarse"), Some(json!(true)));
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
    assert!(shown.sheets[0].views.iter().all(|v| !v.geometry.unproved));
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
        let fine_requests = calls["calls"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["operation"] == "tessellate" && c["log_scope"] == "view_curves")
            .count();
        eprintln!("{tool}: redrawn={}, fine tessellations={fine_requests}", result["redrawn_from_certified_mesh"]);
        exported.push((result["redrawn_from_certified_mesh"].clone(), fine_requests));
        assert!(path.is_file());
    }
    assert_eq!(reopened.drawing.session.as_ref().unwrap().doc.drawing, shown);
    std::fs::remove_dir_all(dir).unwrap();
    assert!(exported.iter().all(|(redrawn, fine)| *redrawn == json!(4) && *fine == 4), "{exported:?}");
}
