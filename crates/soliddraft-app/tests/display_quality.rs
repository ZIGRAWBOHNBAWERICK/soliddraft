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

/// The refusal Catallax's preview mesher returns for the built flange, recorded in the
/// preview's operation span. Catallax task cx-preview-mesher-split-boundary.
const SPLIT_BOUNDARY: (&str, &str) = ("mesh.precision-or-budget", "a boundary constraint was split or overlapped");

/// Builds the unsnapped flange and shows it the way the viewport asks first, through the
/// early preview. The flange's union must build: it needs about 55M work units, so under
/// the former 20M default it ran out of work and only the plate was shown. Catallax's
/// mesher refuses an early preview of the flanged body with [`SPLIT_BOUNDARY`], and the
/// screen shows the certified mesh and edges instead, which must not be empty.
fn shows_the_certified_flange(via_reference_plane: bool) {
    let catallax = Arc::new(soliddraft_kernel_catallax::CatallaxKernel::with_operation_log());
    let part = unsnapped_flange(via_reference_plane, catallax.clone());
    for feature in part.features() {
        assert_eq!(part.status(feature.id), Some(&soliddraft_model::FeatureStatus::Ok), "{}", feature.name);
    }
    let (kernel, body) = (part.kernel(), part.body().unwrap());
    let params = soliddraft_kernel_api::TessellationParams { chord_height: 0.1, ..Default::default() };
    catallax.take_diagnostics().unwrap();
    let shown = kernel.preview(body, &params).unwrap();
    let calls = catallax.take_diagnostics().unwrap();
    let refusals: Vec<_> = calls["calls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|call| call["operation"] == "preview")
        .flat_map(|call| call["operation_log"]["operations"].as_array().cloned().unwrap_or_default())
        .filter_map(|operation| {
            let refusal = operation.get("refusal")?.as_object()?;
            Some((refusal["code"].as_str()?.to_owned(), refusal["detail"].as_str()?.to_owned()))
        })
        .collect();
    assert_eq!(
        refusals,
        [(SPLIT_BOUNDARY.0.to_owned(), SPLIT_BOUNDARY.1.to_owned())],
        "the preview fell back for another reason"
    );
    assert!(!shown.unproved, "a refused preview must show the certified mesh");
    assert!(!shown.indices.is_empty());
    assert_eq!(shown, kernel.tessellate(body, &params).unwrap());
    let edges = kernel.preview_edges(body, &params).unwrap();
    assert!(!edges.is_empty());
    assert_eq!(edges, kernel.tessellate_edges(body, &params).unwrap());
}

#[test]
fn an_exact_face_attachment_shows_the_certified_flange_when_its_preview_is_refused() {
    shows_the_certified_flange(false);
}

#[test]
fn a_refused_preview_shows_the_certified_mesh() {
    // A separate reference plane still uses the rounded sketch frame.
    shows_the_certified_flange(true);
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
