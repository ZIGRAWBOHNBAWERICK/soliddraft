//! Application startup, the geometry backend and sample documents.

use crate::document::{Command, Document, ExtrudeParams, PlaneKind, ProfileKind};
use soliddraft_kernel_api::Kernel;
use soliddraft_tessellation::{Advanced, Quality};
use std::sync::Arc;

/// The Catallax backend used by both the desktop and browser application.
pub fn kernel() -> Arc<dyn Kernel> {
    Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new())
}

/// Hands the drawing library the sheet templates, sheet formats and drafting styles
/// File locations lists, so they join the built-in ones. Call once at startup, before
/// the first drawing.
pub fn load_reference_files() {
    use soliddraft_document::LocationKind;
    let locations = crate::settings::file_locations();
    let mut templates = locations.files(LocationKind::DrawingTemplates);
    templates.extend(locations.files(LocationKind::SheetFormats));
    soliddraft_drawing::library::use_files(templates, locations.files(LocationKind::DraftingStyles));
    for problem in &soliddraft_drawing::library::library().problems {
        log::warn!("File locations: {problem}");
    }
}

/// An empty part.
pub fn new_document() -> Document {
    Document::new("Part1", kernel())
}

/// The sample part used by `--demo` and the screenshots: a base plate, a flange and a boss.
pub fn demo_document() -> Document {
    let mut doc = new_document();
    populate_demo(&mut doc);
    doc
}

/// Adds the sample features to a document, retaining its Catallax diagnostics.
pub fn populate_demo(doc: &mut Document) {
    for params in [
        ExtrudeParams {
            sketch: None,
            profile: ProfileKind::Rectangle { width: 120.0, height: 90.0 },
            plane: PlaneKind::Top,
            depth: 10.0,
            flip: false,
            end: Default::default(),
            draft: None,
            ..Default::default()
        },
        ExtrudeParams {
            sketch: None,
            profile: ProfileKind::Rectangle { width: 120.0, height: 60.0 },
            plane: PlaneKind::Front,
            depth: 10.0,
            flip: true,
            end: Default::default(),
            draft: None,
            ..Default::default()
        },
        ExtrudeParams {
            sketch: None,
            profile: ProfileKind::Circle { diameter: 24.0 },
            plane: PlaneKind::Top,
            depth: 24.0,
            flip: false,
            end: Default::default(),
            draft: None,
            ..Default::default()
        },
    ] {
        doc.apply(Command::AddExtrude { params, via_assistant: false }).ok();
    }
    doc.selection = None;
    doc.dirty = false;
}

/// One body tessellated to well over 10,000 triangles, for the frame-time benchmark.
pub fn bench_document() -> Document {
    let mut doc = new_document();
    let edges = Advanced { chord_height: 0.0, max_edge_length: 2.0, ..Advanced::default() };
    doc.set_viewport_quality(Quality::Custom(edges)).expect("valid advanced values");
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams {
            sketch: None,
            profile: ProfileKind::Rectangle { width: 100.0, height: 60.0 },
            plane: PlaneKind::Top,
            depth: 40.0,
            flip: false,
            end: Default::default(),
            draft: None,
            ..Default::default()
        },
        via_assistant: false,
    })
    .ok();
    doc.selection = None;
    doc
}
