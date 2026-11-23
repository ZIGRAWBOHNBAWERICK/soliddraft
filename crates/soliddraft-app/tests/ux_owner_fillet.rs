//! The owner's fillet in the real app: the pocket rim's six edges picked in the viewport,
//! previewed and built at R2. R5 is refused in plain words that name the face it reaches.
//! The user can apply the suggested radius, save, reopen and use a face in a later feature.
//! Frames land under
//! `target/ribbon-acceptance/ux-owner-fillet/`.

use std::time::{Duration, Instant};

use egui_kittest::kittest::{By, NodeT, Queryable};
use glam::Vec3;
use serde_json::json;
use soliddraft_app::app_tools::AppTools;
use soliddraft_app::document::{Document, FeatureKind, FeatureStatus};
use soliddraft_app::state::{AppState, EditorParams};
use soliddraft_assistant::{Caller, tools::call_checked};

#[path = "support/ui.rs"]
mod ribbon;

use ribbon::App;

const HANG_GUARD: Duration = Duration::from_secs(900);

/// The owner's part, as `fillet_acceptance` builds it, and the midpoints of its pocket
/// rim's six edges.
fn owner_part() -> (Document, Vec<Vec3>) {
    let mut state = AppState::new(soliddraft_app::new_document());
    let caller = Caller::Mcp { client: "ux-owner-fillet".into() };
    for (tool, arguments) in [
        (
            "sketch_geometry",
            json!({"plane":"front","operations":[
                {"tool":"add","construction":false,"kind":{"type":"line","a":{"u":-40,"v":0},"b":{"u":40,"v":0}}},
                {"tool":"add","construction":false,"kind":{"type":"line","a":{"u":40,"v":0},"b":{"u":40,"v":35}}},
                {"tool":"add","construction":false,"kind":{"type":"arc","center":{"u":15,"v":35},"start":{"u":40,"v":35},"end":{"u":15,"v":60}}},
                {"tool":"add","construction":false,"kind":{"type":"line","a":{"u":15,"v":60},"b":{"u":-15,"v":60}}},
                {"tool":"add","construction":false,"kind":{"type":"arc","center":{"u":-15,"v":35},"start":{"u":-15,"v":60},"end":{"u":-40,"v":35}}},
                {"tool":"add","construction":false,"kind":{"type":"line","a":{"u":-40,"v":35},"b":{"u":-40,"v":0}}}]}),
        ),
        ("extrude", json!({"sketch":"Sketch1","depth":76,"flip":false})),
        (
            "sketch",
            json!({"plane":"top","offset":60,"shapes":[{"shape":"rectangle","center":[15,-38,60],"width":34,"height":22.67}]}),
        ),
        ("cut", json!({"sketch":"Sketch2","through_all":false,"depth":35,"flip":false})),
    ] {
        let out = call_checked(&mut AppTools { state: &mut state }, tool, arguments, &caller);
        assert!(!out.is_error, "{tool}: {}", out.content);
    }
    let doc = std::mem::replace(&mut state.doc, soliddraft_app::new_document());
    let model = doc.model();
    let topology = model.kernel().topology(model.body().unwrap()).unwrap();
    let feature = |face| {
        let named = doc.named_face(face).unwrap();
        doc.feature(named.feature).unwrap().name.clone()
    };
    let walls: Vec<_> = topology
        .faces
        .iter()
        .filter(|f| feature(f.id) == "Cut-Extrude1" && f.bbox.max.z - f.bbox.min.z > 1.0)
        .map(|f| f.id)
        .collect();
    let rim: Vec<Vec3> = topology
        .edges
        .iter()
        .filter(|e| {
            e.faces.iter().filter(|f| walls.contains(f)).count() == 1
                && e.faces.iter().any(|f| feature(*f) == "Boss-Extrude1")
        })
        .map(|e| Vec3::new(e.midpoint.x as f32, e.midpoint.y as f32, e.midpoint.z as f32))
        .collect();
    assert_eq!(rim.len(), 6);
    (doc, rim)
}

fn settle(app: &mut App) {
    let start = Instant::now();
    while app.state().state.doc.proof_pending() {
        assert!(start.elapsed() < HANG_GUARD, "still rebuilding");
        app.step();
    }
    app.run_steps(20);
}

/// Every text the frame shows: accessibility labels and values, and painted text.
fn texts(app: &App) -> Vec<String> {
    let accessible = app.query_all(By::new()).flat_map(|node| {
        let node = node.accesskit_node();
        [node.label(), node.value()]
    });
    accessible.flatten().chain(ribbon::painted(app).into_iter().map(|p| p.text)).collect()
}

fn project(app: &App, world: Vec3) -> egui::Pos2 {
    let state = &app.state().state;
    state.viewport.camera.project(world, state.viewport.rect).expect("in front of the camera")
}

fn held(app: &App) -> usize {
    app.state().state.editor.as_ref().expect("the Fillet panel is open").edges().len()
}

/// Opens Fillet and picks the rim's edges in the viewport, as the owner did.
fn pick_rim(app: &mut App, rim: &[Vec3]) {
    ribbon::switch_workspace(app, "solid");
    ribbon::run_command(app, "fillet");
    app.run_steps(4);
    for (i, edge) in rim.iter().enumerate() {
        let at = project(app, *edge);
        ribbon::pick_at(app, at);
        app.run_steps(2);
        assert_eq!(held(app), i + 1, "the rim edge at {edge} picks");
    }
}

/// Types a radius and waits for the panel to answer it, with a preview or a refusal.
fn radius(app: &mut App, r: f64) {
    ribbon::type_field(app, "Radius value", &r.to_string());
    ribbon::until(app, &format!("the R{r} answer"), |a| {
        a.state().state.editor.as_ref().is_some_and(|e| {
            matches!(&e.params, EditorParams::Fillet(f) if f.radius == r)
                && (e.preview.result.is_some() || e.error.is_some())
        })
    });
    settle(app);
}

fn accept_radius(app: &mut App) -> f64 {
    let shown = texts(app);
    let label = shown
        .iter()
        .find(|t| t.starts_with("Apply ") && t.ends_with(" mm radius"))
        .unwrap_or_else(|| panic!("the refusal offers one radius to apply: {shown:?}"));
    let offered: f64 = label.trim_start_matches("Apply ").trim_end_matches(" mm radius").parse().unwrap();
    assert!((3.9..=4.0).contains(&offered), "the rounded wall limits the radius to 4 mm: {offered}");
    let button = ribbon::button_rect(app, label, ribbon::window(app));
    ribbon::click_at(app, button.center());
    settle(app);
    assert!(app.state().state.editor.is_none(), "the accepted proposal closes the editor");
    offered
}

fn fillet_radius(doc: &Document, id: u64) -> f64 {
    let feature = doc.feature(id).expect("the fillet is in the tree");
    assert_eq!(feature.status, FeatureStatus::Ok);
    assert_eq!(doc.faces.iter().filter(|face| face.feature == id).count(), 6, "one round per picked edge");
    match &feature.kind {
        FeatureKind::Fillet(params) => params.radius,
        kind => panic!("expected a fillet, got {kind:?}"),
    }
}

#[test]
fn the_owner_rims_his_pocket_at_r2_and_reads_why_r5_does_not_fit() {
    let case = ribbon::case("ux-owner-fillet");
    let (doc, rim) = owner_part();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        ribbon::switch_workspace(app, "view");
        ribbon::run_command(app, "view_isometric");
        ribbon::run_command(app, "zoom_fit");
        settle(app);
        case.frame(app, "01-owner-part");

        pick_rim(app, &rim);
        radius(app, 5.0);
        let doc = &app.state().state.doc;
        assert_eq!(doc.features.len(), 4, "the refused creation kept the base intact");
        let offered = accept_radius(app);
        let doc = &app.state().state.doc;
        let fillet = doc.features.last().unwrap();
        assert_eq!(fillet_radius(doc, fillet.id), offered);
        assert!(fillet.via_assistant, "the applied change came from an assistant proposal");
        app.state_mut().state.doc.undo().unwrap();
        settle(app);

        pick_rim(app, &rim);
        radius(app, 2.0);
        let editor = app.state().state.editor.as_ref().unwrap();
        assert!(editor.preview.result.is_some(), "R2 previews the rounded rim: {:?}", editor.error);
        case.frame(app, "02-rim-r2-preview");
        let rect = ribbon::button_rect(app, "OK", ribbon::window(app));
        ribbon::click_at(app, rect.center());
        settle(app);
        let doc = &app.state().state.doc;
        let fillet = doc.features.last().unwrap();
        assert!(matches!(fillet.kind, FeatureKind::Fillet(_)) && fillet.status == FeatureStatus::Ok);
        let id = fillet.id;
        let kept = doc.model().part_json();
        let depth = doc.undo_depth();
        case.frame(app, "03-rim-r2-built");

        // R5 is past the 4 mm the rounded side's thin wall leaves: the panel says which
        // face the fillet would reach and the radius that fits, and the part stays.
        ribbon::run_command(app, "edit_feature");
        settle(app);
        radius(app, 5.0);
        let confirm = ribbon::button_rect(app, "OK", ribbon::window(app));
        ribbon::click_at(app, confirm.center());
        settle(app);
        let shown = texts(app);
        let why = shown
            .iter()
            .find(|t| t.starts_with("This fillet runs into "))
            .unwrap_or_else(|| panic!("the panel names the face R5 reaches: {shown:?}"));
        assert!(why.contains("Try a radius under"), "{why}");
        assert!(why.contains("Boss-Extrude1 ·"), "{why}");
        assert_eq!(app.state().state.doc.undo_depth(), depth);
        let original = app.state().state.doc.feature(id).unwrap();
        assert!(matches!(&original.kind, FeatureKind::Fillet(params) if params.radius == 2.0));
        case.frame(app, "04-rim-r5-refused");

        let offered = accept_radius(app);
        let doc = &app.state().state.doc;
        assert_eq!(fillet_radius(doc, id), offered);
        assert_eq!(doc.undo_depth(), depth + 1);
        assert_eq!(doc.features.len(), 5);
        app.state_mut().state.doc.undo().unwrap();
        assert_eq!(app.state().state.doc.model().part_json(), kept);
        app.state_mut().state.doc.redo().unwrap();
        settle(app);
        assert_eq!(fillet_radius(&app.state().state.doc, id), offered);
        case.frame(app, "05-rim-corrected");

        let path = case.path("accepted-radius.dwprt");
        let caller = Caller::Mcp { client: "ux-owner-fillet".into() };
        let tool = |app: &mut App, tool, args| {
            let out = call_checked(&mut AppTools { state: &mut app.state_mut().state }, tool, args, &caller);
            assert!(!out.is_error, "{tool}: {}", out.content);
        };
        tool(app, "save", json!({"path": path}));
        tool(app, "new_part", json!({}));
        tool(app, "open", json!({"path": path}));
        let doc = &app.state().state.doc;
        assert_eq!(fillet_radius(doc, id), offered);
        let cut = doc.features.iter().find(|f| f.name == "Cut-Extrude1").unwrap();
        let floor = doc
            .faces
            .iter()
            .filter(|f| f.feature == cut.id && f.surface == soliddraft_kernel_api::SurfaceKind::Plane)
            .min_by(|a, b| a.centroid.z.total_cmp(&b.centroid.z))
            .expect("the reopened pocket has its planar floor");
        let source = format!("{} · {}", cut.name, floor.label);
        tool(app, "reference_plane", json!({"plane": source, "offset": 2}));
        let doc = &app.state().state.doc;
        let plane = doc.features.last().unwrap();
        assert!(matches!(plane.kind, FeatureKind::ReferencePlane(_)));
        assert_eq!(plane.status, FeatureStatus::Ok);
        assert_eq!(fillet_radius(doc, id), offered);
    });
}
