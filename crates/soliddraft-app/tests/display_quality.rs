//! The part view's display levels on the Catallax kernel: a rebuild shows the standard
//! level at once, the worker makes the others, the camera moves between them, and a
//! quality change swaps the mesh when its replacement is ready without a rebuild or a
//! new revision.

use soliddraft_kernel_api::Kernel;
use std::sync::Arc;
use std::time::{Duration, Instant};

use soliddraft_app::document::{Command, Document, ExtrudeParams, PlaneKind, ProfileKind};
use soliddraft_tessellation::{Level, Projection, Quality};

/// The hang guard every test shares: a wait this long means stuck, not slow.
const HANG: Duration = Duration::from_secs(600);

/// A disc 200 mm across and 50 mm thick: large enough that the chord height, not the
/// 15 degree angle, decides the standard and fine levels.
fn disc() -> Document {
    let mut doc = soliddraft_app::document::Document::new(
        "Part1",
        std::sync::Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new()),
    );
    let params = ExtrudeParams {
        sketch: None,
        profile: ProfileKind::Circle { diameter: 200.0 },
        plane: PlaneKind::Top,
        depth: 50.0,
        flip: false,
        end: Default::default(),
        draft: None,
        ..Default::default()
    };
    doc.apply(Command::AddExtrude { params, via_assistant: false }).unwrap();
    doc
}

fn triangles(doc: &Document) -> usize {
    doc.meshes.values().map(|m| m.indices.len() / 3).sum()
}

/// Frames at `projection` until the worker has nothing left to make.
fn settle(doc: &mut Document, projection: &Projection) {
    let start = Instant::now();
    while doc.update_display(projection, 0.016) && doc.display_quality().pending() {
        assert!(start.elapsed() < HANG, "the worker did not answer");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_camera_moves_the_part_between_levels() {
    let mut doc = disc();
    let q = doc.display_quality();
    assert_eq!(q.shown(), Some(Level::Standard));
    assert_eq!(doc.tess_params, *q.levels().params(Level::Standard));
    let standard = triangles(&doc);
    let faces = doc.faces.len();

    // At about a pixel per millimetre the standard level's 0.57 mm is under a pixel.
    let middle = Projection::Orthographic { pixels_per_mm: 1.0 };
    settle(&mut doc, &middle);
    assert_eq!(doc.display_quality().shown(), Some(Level::Standard));

    // Close up the fine level is shown, from the worker, with the same faces.
    let version = doc.mesh_version;
    assert!(doc.update_display(&Projection::Orthographic { pixels_per_mm: 10.0 }, 0.016));
    assert_eq!(doc.display_quality().shown(), Some(Level::Fine));
    assert!(doc.mesh_version > version);
    assert!(triangles(&doc) > standard, "{} vs {standard}", triangles(&doc));
    assert_eq!(doc.faces.len(), faces);
    assert_eq!(doc.tess_params, *doc.display_quality().levels().params(Level::Fine));

    // Far away it steps down one level at a time, after two quiet seconds each.
    let far = Projection::Orthographic { pixels_per_mm: 0.1 };
    doc.update_display(&far, 1.0);
    assert_eq!(doc.display_quality().shown(), Some(Level::Fine), "not at once");
    for _ in 0..8 {
        doc.update_display(&far, 1.0);
    }
    assert_eq!(doc.display_quality().shown(), Some(Level::Coarse));
    assert!(triangles(&doc) <= standard);

    // A rebuild keeps the level the camera asked for and makes it at once.
    let params = ExtrudeParams {
        sketch: None,
        profile: ProfileKind::Circle { diameter: 40.0 },
        plane: PlaneKind::Top,
        depth: 80.0,
        flip: false,
        end: Default::default(),
        draft: None,
        ..Default::default()
    };
    doc.apply(Command::AddExtrude { params, via_assistant: false }).unwrap();
    assert_eq!(doc.display_quality().shown(), Some(Level::Coarse));
    assert_eq!(doc.tess_params, *doc.display_quality().levels().params(Level::Coarse));
}

#[test]
fn a_quality_change_swaps_the_mesh_without_a_rebuild() {
    let mut doc = disc();
    // Both settings' standard levels stay between the thresholds here.
    let middle = Projection::Orthographic { pixels_per_mm: 1.5 };
    settle(&mut doc, &middle);
    let (before, revision, version) = (triangles(&doc), doc.model().semantic_hash(), doc.mesh_version);
    // As if just saved.
    doc.dirty = false;

    doc.set_viewport_quality(Quality::Slider { value: 10 }).unwrap();
    assert!(doc.dirty, "the setting is saved with the part");
    assert_eq!(doc.mesh_version, version, "the old mesh stays until the new one is ready");
    assert_eq!(doc.model().settings().viewport, Quality::Slider { value: 10 });
    settle(&mut doc, &middle);
    assert!(doc.mesh_version > version);
    assert!(triangles(&doc) > before, "{} vs {before}", triangles(&doc));
    assert_eq!(doc.model().semantic_hash(), revision, "no new revision");
    assert_eq!(doc.tess_params, *doc.display_quality().levels().params(doc.display_quality().shown().unwrap()));

    // The same value again changes nothing.
    doc.dirty = false;
    doc.set_viewport_quality(Quality::Slider { value: 10 }).unwrap();
    assert!(!doc.dirty);
    assert!(doc.set_viewport_quality(Quality::Slider { value: 11 }).is_err());

    // Saved and reopened, the part keeps the setting and shows it at once.
    let bytes = doc.model().to_bytes().unwrap();
    let reopened = soliddraft_model::Document::from_bytes(&bytes, doc.kernel().clone()).unwrap();
    let reopened = Document::from_model(reopened);
    assert_eq!(reopened.display_quality().levels(), doc.display_quality().levels());
    assert_eq!(triangles(&reopened), triangles(&doc));
}

#[test]
fn graphics_presets_remesh_curves_without_editing_the_saved_part() {
    use soliddraft_app::graphics::{CurvedSurfaces, Graphics, GraphicsDevice, GraphicsPreset};

    let mut doc = disc();
    let projection = Projection::Orthographic { pixels_per_mm: 1.0 };
    let revision = doc.revision();
    let bytes = doc.model().to_bytes().unwrap();
    let mut graphics = Graphics::default();
    let device = GraphicsDevice::default();
    let mut results = Vec::new();
    for (preset, expected, angle) in [
        (GraphicsPreset::Performance, CurvedSurfaces::Coarse, 15.0),
        (GraphicsPreset::Quality, CurvedSurfaces::Fine, 10.0),
    ] {
        graphics.choose_preset(preset, &device);
        assert_eq!(graphics.curved_surfaces, Some(expected));
        doc.set_curved_surfaces(graphics.curved_surfaces);
        settle(&mut doc, &projection);
        let params = doc.tess_params;
        assert_eq!(params.curve_angle_deg, angle);
        assert_eq!(params.surface_angle_deg, angle);
        results.push((params.chord_height, triangles(&doc)));
    }
    assert!(results[1].0 < results[0].0, "{results:?}");
    assert!(results[1].1 > results[0].1, "{results:?}");
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.model().to_bytes().unwrap(), bytes);

    graphics.choose_preset(GraphicsPreset::Custom, &device);
    assert_eq!(graphics.curved_surfaces, Some(CurvedSurfaces::Fine));
    graphics.curved_surfaces = Some(CurvedSurfaces::Ultra);
    let saved = doc.model().to_bytes().unwrap();
    let reopened = soliddraft_model::Document::from_bytes(&saved, doc.kernel().clone()).unwrap();
    let mut reopened = Document::from_model(reopened);
    reopened.set_curved_surfaces(graphics.curved_surfaces);
    settle(&mut reopened, &projection);
    assert_eq!(reopened.tess_params.surface_angle_deg, 7.5);

    let id = reopened.features.last().unwrap().id;
    let mut edited = match &reopened.feature(id).unwrap().kind {
        soliddraft_app::document::FeatureKind::Extrude(params) => params.clone(),
        _ => panic!("the reopened part should have its extrusion"),
    };
    edited.depth = 65.0;
    reopened.apply(Command::EditExtrude { id, params: edited.clone() }).unwrap();
    assert_eq!(reopened.tess_params.surface_angle_deg, 7.5);
    edited.profile = ProfileKind::Circle { diameter: 40.0 };
    edited.depth = 90.0;
    reopened.apply(Command::AddExtrude { params: edited, via_assistant: false }).unwrap();
    assert_eq!(reopened.tess_params.surface_angle_deg, 7.5);
}

#[test]
fn an_unchanged_body_keeps_its_mesh_and_undo_shows_the_earlier_one_again() {
    use soliddraft_kernel_api::counting::{Counting, Work};

    let kernel =
        std::sync::Arc::new(Counting::new(std::sync::Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new())));
    let mut doc = Document::new("Part1", kernel.clone());
    let params = ExtrudeParams {
        sketch: None,
        profile: ProfileKind::Circle { diameter: 200.0 },
        plane: PlaneKind::Top,
        depth: 50.0,
        flip: false,
        end: Default::default(),
        draft: None,
        ..Default::default()
    };
    let id = doc.apply(Command::AddExtrude { params: params.clone(), via_assistant: false }).unwrap().unwrap();
    let middle = Projection::Orthographic { pixels_per_mm: 1.0 };
    settle(&mut doc, &middle);
    let disc = doc.display_body().unwrap();
    let disc_triangles = triangles(&doc);
    kernel.take();

    // A rename builds and tessellates nothing: the body and its mesh stay.
    doc.apply(Command::Rename { id, name: "Disc".into() }).unwrap();
    assert_eq!(kernel.take(), [], "the rename built or tessellated something");
    assert_eq!(doc.display_body(), Some(disc));
    assert_eq!(triangles(&doc), disc_triangles);

    // A deeper disc is a new body: it is built and tessellated, and nothing else is.
    doc.apply(Command::EditExtrude { id, params: ExtrudeParams { depth: 60.0, ..params } }).unwrap();
    let deeper = doc.display_body().unwrap();
    assert_ne!(deeper, disc);
    let work = kernel.take();
    assert!(work.iter().any(|w| matches!(w, Work::Operation(_))), "{work:?}");
    assert!(work.iter().all(|w| !matches!(w, Work::Mesh(b) if *b != deeper)), "{work:?}");
    settle(&mut doc, &middle);
    kernel.take();

    // Undo shows the first disc again with its kept body and meshes, and redo the
    // deeper one: nothing is built or tessellated, now or by the worker.
    doc.undo().unwrap();
    assert_eq!(doc.display_body(), Some(disc));
    assert_eq!(triangles(&doc), disc_triangles);
    settle(&mut doc, &middle);
    assert_eq!(kernel.take(), [], "undo built or tessellated something");
    doc.redo().unwrap();
    assert_eq!(doc.display_body(), Some(deeper));
    settle(&mut doc, &middle);
    assert_eq!(kernel.take(), [], "redo built or tessellated something");
    assert!(!doc.faces.is_empty() && doc.pick_of_face(doc.faces[0].id).is_some(), "the kept picks are in place");
}

#[test]
fn datum_sketch_edits_keep_the_solid_mesh_picks_and_gpu_upload() {
    use soliddraft_app::document::{Selection, SketchPlane};

    let mut doc = disc();
    settle(&mut doc, &Projection::Orthographic { pixels_per_mm: 1.0 });
    let body = doc.display_body().unwrap();
    let mesh = doc.meshes[&body].clone();
    let faces = doc.faces.clone();
    let edges = doc.edges.clone();
    let selected = faces[0].id;
    let pick = doc.pick_of_face(selected);
    doc.selection = Some(Selection::Face(selected));
    let upload = doc.mesh_version;
    let sketch = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let unchanged = |doc: &Document| {
        assert_eq!(doc.display_body(), Some(body));
        assert!(Arc::ptr_eq(&doc.meshes[&body], &mesh), "a sketch replaced the solid's display mesh");
        assert_eq!(doc.faces, faces);
        assert_eq!(doc.edges, edges);
        assert_eq!(doc.pick_of_face(selected), pick);
        assert_eq!(doc.selection, Some(Selection::Face(selected)));
        assert_eq!(doc.mesh_version, upload, "a sketch invalidated the solid's GPU upload");
    };
    unchanged(&doc);
    let mut geometry = doc.sketch_of(sketch).unwrap().sketch.clone();
    geometry.circle_at(soliddraft_sketch::P2::new(0.0, 0.0), 5.0).unwrap();
    doc.apply(Command::EditSketch { id: sketch, sketch: geometry }).unwrap();
    unchanged(&doc);
    doc.undo().unwrap();
    unchanged(&doc);
    doc.redo().unwrap();
    unchanged(&doc);
}

#[test]
fn a_late_answer_for_another_quality_does_not_replace_the_cached_levels() {
    let mut doc = disc();
    let close = Projection::Orthographic { pixels_per_mm: 10.0 };
    let far = Projection::Orthographic { pixels_per_mm: 0.1 };
    let auto = doc.model().settings().viewport;

    // Every level of the current quality is cached, and the fine one is shown.
    settle(&mut doc, &close);
    doc.update_display(&close, 0.016);
    assert_eq!(doc.display_quality().shown(), Some(Level::Fine));
    let fine = triangles(&doc);

    // Ask for another quality, then return to the cached one before the worker's answer
    // is taken: the frame loop only takes answers in `update_display`.
    doc.set_viewport_quality(Quality::Slider { value: 10 }).unwrap();
    doc.set_viewport_quality(auto).unwrap();
    assert!(doc.display_quality().pending(), "the other quality's request is still out");
    settle(&mut doc, &close);

    // Step down to the coarse level and back up: the fine mesh comes from the cache and
    // must still be the one made for this quality.
    for _ in 0..9 {
        doc.update_display(&far, 1.0);
    }
    assert_eq!(doc.display_quality().shown(), Some(Level::Coarse));
    settle(&mut doc, &close);
    doc.update_display(&close, 0.016);
    assert_eq!(doc.display_quality().shown(), Some(Level::Fine));
    assert_eq!(triangles(&doc), fine, "the fine level shows another quality's mesh");
}

/// The near-coincident flange with its sketch constraints removed, built on `kernel`.
fn unsnapped_flange(via_reference_plane: bool, kernel: Arc<dyn Kernel>) -> soliddraft_model::Document {
    let bytes = include_bytes!("data/near-coincident-flange.dwprt");
    let part = soliddraft_model::Document::from_bytes(bytes, soliddraft_app::kernel()).unwrap();
    let mut source = part.truth();
    for feature in source["features"].as_array_mut().unwrap() {
        let kind = &mut feature["kind"];
        if kind["type"] == "sketch" {
            kind["sketch"]["constraints"] = serde_json::json!([]);
        }
    }
    if via_reference_plane {
        let id = source["next_feature"].as_u64().unwrap();
        source["next_feature"] = serde_json::json!(id + 1);
        let features = source["features"].as_array_mut().unwrap();
        let index = features.iter().position(|f| f["kind"]["plane"]["on"] == "face").unwrap();
        let mut plane = features[index]["kind"]["plane"].clone();
        plane["type"] = serde_json::json!("reference_plane");
        features[index]["kind"]["plane"] = serde_json::json!({"on":"feature", "feature":id});
        features.insert(index, serde_json::json!({"id":id, "name":"ReferencePlane1", "kind":plane}));
    }
    soliddraft_model::Document::from_part_json(&source.to_string(), kernel).unwrap()
}

/// The early preview's refusals, recorded in the operation spans of the `preview` calls
/// since the diagnostics were last taken.
fn preview_refusals(catallax: &soliddraft_kernel_catallax::CatallaxKernel) -> Vec<(String, String)> {
    let calls = catallax.take_diagnostics().unwrap();
    calls["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|call| call["operation"] == "preview")
        .flat_map(|call| call["operation_log"]["operations"].as_array().cloned().unwrap_or_default())
        .filter_map(|operation| {
            let refusal = operation.get("refusal")?.as_object()?;
            Some((refusal["code"].as_str()?.to_owned(), refusal["detail"].as_str()?.to_owned()))
        })
        .collect()
}

/// Builds the unsnapped flange and shows it the way the viewport asks first, through the
/// early preview. The flange's union must build: it needs about 55M work units, so under
/// the former 20M default it ran out of work and only the plate was shown. Catallax's
/// mesher refused an early preview of the flanged body ("a boundary constraint was split
/// or overlapped") until Catallax PR 650; it now samples one, with edges.
fn shows_the_early_flange(via_reference_plane: bool) {
    let catallax = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_operation_log());
    let part = unsnapped_flange(via_reference_plane, catallax.clone());
    for feature in part.features() {
        assert_eq!(part.status(feature.id), Some(&soliddraft_model::FeatureStatus::Ok), "{}", feature.name);
    }
    let (kernel, body) = (part.kernel(), part.body().unwrap());
    let params = soliddraft_kernel_api::TessellationParams { chord_height: 0.1, ..Default::default() };
    catallax.take_diagnostics().unwrap();
    let shown = kernel.preview(body, &params).unwrap();
    assert_eq!(preview_refusals(&catallax), [], "the early preview was refused");
    assert!(shown.unproved, "the flange shows its early preview");
    assert!(!shown.indices.is_empty());
    assert!(!kernel.preview_edges(body, &params).unwrap().is_empty());
}

#[test]
fn an_exact_face_attachment_shows_the_early_flange() {
    shows_the_early_flange(false);
}

#[test]
fn a_reference_plane_flange_shows_its_early_preview() {
    // A separate reference plane still uses the rounded sketch frame.
    shows_the_early_flange(true);
}

#[test]
fn a_snapped_flange_can_use_an_early_display() {
    let bytes = include_bytes!("data/near-coincident-flange.dwprt");
    let part = soliddraft_model::Document::from_bytes(bytes, soliddraft_app::kernel()).unwrap();
    let (kernel, body) = (part.kernel(), part.body().unwrap());
    let params = soliddraft_kernel_api::TessellationParams { chord_height: 0.1, ..Default::default() };
    let shown = kernel.preview(body, &params).unwrap();
    assert!(shown.unproved, "the saved snap relations remove the accidental tilt");
    assert!(!shown.indices.is_empty());
    assert!(!kernel.preview_edges(body, &params).unwrap().is_empty());
}

/// Two discs side by side, each its own body.
fn two_discs() -> Document {
    let mut doc = disc();
    doc.new_body();
    let params = ExtrudeParams {
        profile: ProfileKind::Circle { diameter: 120.0 },
        plane: PlaneKind::Top,
        depth: 30.0,
        ..Default::default()
    };
    doc.apply(Command::AddExtrude { params, via_assistant: false }).unwrap();
    assert_eq!(doc.meshes.len(), 2);
    doc
}

/// Polls the background proof until the document has adopted it.
fn wait_proof(doc: &mut Document) {
    let start = Instant::now();
    while doc.proof_pending() {
        assert!(start.elapsed() < HANG, "the background proof did not answer");
        doc.poll_proof();
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_proof_adopted_after_a_curve_change_meshes_every_body_at_the_new_setting() {
    use soliddraft_app::graphics::CurvedSurfaces;

    let deepen = |doc: &mut Document| {
        let id = doc.features.last().unwrap().id;
        let mut params = match &doc.feature(id).unwrap().kind {
            soliddraft_app::document::FeatureKind::Extrude(params) => params.clone(),
            _ => panic!("the second disc is an extrusion"),
        };
        params.depth = 45.0;
        doc.apply_interactive(Command::EditExtrude { id, params }).unwrap();
    };
    let triangles_by_body = |doc: &Document| {
        let mut counts: Vec<usize> = doc.meshes.values().map(|mesh| mesh.indices.len() / 3).collect();
        counts.sort_unstable();
        counts
    };

    // The same edit made with Ultra already chosen.
    let mut expected = two_discs();
    expected.set_curved_surfaces(Some(CurvedSurfaces::Ultra));
    deepen(&mut expected);
    wait_proof(&mut expected);

    // Ultra chosen while the edit's proof runs.
    let mut doc = two_discs();
    deepen(&mut doc);
    doc.set_curved_surfaces(Some(CurvedSurfaces::Ultra));
    wait_proof(&mut doc);

    assert_eq!(doc.meshes.len(), 2);
    assert_eq!(doc.tess_params, expected.tess_params, "the adopted bodies use the current setting");
    assert_eq!(triangles_by_body(&doc), triangles_by_body(&expected), "every body is meshed at Ultra");
}

/// Every named face with its pick number, by feature and name. Each face's pick must be
/// drawn by exactly one mesh on screen, the mesh of the body that holds the face.
fn picks_by_name(doc: &Document) -> Vec<((u64, String), u32)> {
    let mut named = Vec::new();
    for face in &doc.faces {
        let pick = doc.pick_of_face(face.id).expect("every named face has a pick");
        let drawn: Vec<_> =
            doc.meshes.iter().filter(|(_, mesh)| mesh.face_picks.contains(&pick)).map(|(body, _)| *body).collect();
        assert_eq!(drawn.len(), 1, "{} is drawn by {} meshes", face.label, drawn.len());
        let holds = doc.kernel().topology(drawn[0]).unwrap().faces.iter().any(|f| f.id == face.id);
        assert!(holds, "{} is picked on another body's mesh", face.label);
        named.push(((face.feature, face.label.clone()), pick));
    }
    named.sort();
    named
}

#[test]
fn a_two_body_part_publishes_meshes_names_and_picks_together() {
    use soliddraft_app::graphics::CurvedSurfaces;

    let mut doc = two_discs();
    let first = picks_by_name(&doc);

    // A rename changes no geometry: both bodies keep their meshes and their GPU upload.
    let (meshes, upload) = (doc.meshes.clone(), doc.mesh_version);
    let id = doc.features.last().unwrap().id;
    doc.apply(Command::Rename { id, name: "Small disc".into() }).unwrap();
    assert_eq!(doc.mesh_version, upload, "a rename uploaded the meshes again");
    assert!(meshes.iter().all(|(body, mesh)| Arc::ptr_eq(&doc.meshes[body], mesh)), "a rename remeshed a body");

    // The second disc deepened, with Ultra chosen while its proof runs.
    let mut params = match &doc.feature(id).unwrap().kind {
        soliddraft_app::document::FeatureKind::Extrude(params) => params.clone(),
        _ => panic!("the second disc is an extrusion"),
    };
    params.depth = 45.0;
    doc.apply_interactive(Command::EditExtrude { id, params }).unwrap();
    doc.set_curved_surfaces(Some(CurvedSurfaces::Ultra));
    wait_proof(&mut doc);
    let edited = picks_by_name(&doc);
    assert_eq!(edited.len(), first.len());

    // The same part opened fresh at that setting numbers its picks the same way.
    let bytes = doc.model().to_bytes().unwrap();
    let mut reopened =
        Document::from_model(soliddraft_model::Document::from_bytes(&bytes, doc.kernel().clone()).unwrap());
    reopened.set_curved_surfaces(Some(CurvedSurfaces::Ultra));
    wait_proof(&mut reopened);
    assert_eq!(reopened.tess_params, doc.tess_params);
    assert_eq!(picks_by_name(&reopened), edited);

    // Undo shows the first discs again, each face under its first pick.
    doc.undo().unwrap();
    wait_proof(&mut doc);
    assert_eq!(picks_by_name(&doc), first);
}
