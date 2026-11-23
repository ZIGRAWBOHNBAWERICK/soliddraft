//! Configurations chosen per placement, as a user meets them: a part placed twice in two
//! of its configurations, and a sub-assembly placed in one of its own. Build, change,
//! save, reopen, and keep working on top.
//!
//! Block A is 40 x 30, 14 mm tall in its Default configuration and 28 mm in Tall. Block
//! B is 20 x 10 x 6. Both are centred in plan and rise along +Z.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::DVec3;
use soliddraft_assembly::{
    Alignment, AssemblyDocument, ComponentId, EndpointSelector, FaceSelector, GeneratorChoice, KernelSource,
    OccurrencePath, OpenOptions, Placement, component_path,
};
use soliddraft_document::{CacheOnlyStore, DocRef, identify_file};
use soliddraft_kernel_api::Kernel;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, ProfileSource, SketchFeature,
    SketchPlane,
};

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-component-configurations-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn options() -> OpenOptions {
    OpenOptions {
        display: Some(Arc::new(KernelSource::new(kernel()))),
        revisions: Some(Arc::new(CacheOnlyStore::new())),
        check_freshness: true,
        ..OpenOptions::default()
    }
}

fn block(w: f64, d: f64, h: f64) -> Document {
    let mut doc = Document::new("Block", kernel());
    let mut sketch = Sketch::new();
    sketch.rectangle(P2::new(0.0, 0.0), w, d).unwrap();
    solve(&mut sketch, None, true);
    let plane = SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 };
    let add = |doc: &mut Document, kind| {
        doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
            .unwrap()
            .created
            .unwrap()
    };
    let s = add(&mut doc, FeatureKind::Sketch(SketchFeature { plane, sketch }));
    add(
        &mut doc,
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: s },
            operation: ExtrudeOp::Boss,
            end: EndCondition::Blind { depth: h },
            reverse: false,
        }),
    );
    doc
}

/// Block A: 14 mm tall in Default, 28 mm in Tall, saved in Default.
fn new_block_a(path: &Path) {
    let mut doc = block(40.0, 30.0, 14.0);
    let table = doc.values_table();
    let depth = table.groups.iter().flat_map(|g| &g.rows).map(|r| r.name.clone()).find(|n| n.ends_with(".Depth"));
    let depth = depth.expect("the extrusion's depth is a parameter");
    doc.execute(Command::AddConfiguration { name: "Tall".into() }).unwrap();
    let tall = Command::SetValueCell { configuration: "Tall".into(), name: depth, expression: Some("28".into()) };
    doc.execute(tall).unwrap();
    doc.save(path).unwrap();
}

fn reference(from: &Path, to: &Path) -> DocRef {
    let i = identify_file(to).unwrap();
    let mut r = DocRef::new(i.id, i.kind, "", i.revision);
    r.set_path(from, to);
    r
}

fn face(component: ComponentId, generator: GeneratorChoice, normal: Option<[f64; 3]>) -> EndpointSelector {
    EndpointSelector { component, face: FaceSelector::Generator { feature: "Boss-Extrude1".into(), generator, normal } }
}

fn z_of(doc: &AssemblyDocument, c: ComponentId) -> f64 {
    doc.assembly().component(c).unwrap().placement.translation.z
}

/// How tall the occurrence at `path` is shown.
fn height(doc: &AssemblyDocument, path: &OccurrencePath) -> f64 {
    let row = doc.table().find(path).unwrap();
    let b = doc.store().get(doc.table().definition(row)).bounds;
    b.max.z - b.min.z
}

#[test]
fn each_placement_shows_the_part_configuration_it_names() {
    let dir = folder("parts");
    let (a_file, b_file, root) = (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("rig.dwasm"));
    new_block_a(&a_file);
    block(20.0, 10.0, 6.0).save(&b_file).unwrap();
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&root)).unwrap();
    let a = doc.insert(reference(&root, &a_file), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference(&root, &b_file), Placement::translation(DVec3::new(10.0, 10.0, 30.0))).unwrap();
    let twin = doc.insert(reference(&root, &a_file), Placement::translation(DVec3::new(80.0, 0.0, 0.0))).unwrap();
    doc.finish_loading();
    doc.mate_origin("O", a, true, false).unwrap();
    for (name, n) in [("X", [1.0, 0.0, 0.0]), ("Y", [0.0, 1.0, 0.0])] {
        let side = GeneratorChoice::Side;
        doc.mate_coincident(name, &face(a, side, Some(n)), &face(b, side, Some(n)), Alignment::Aligned, false).unwrap();
    }
    let (top, bottom) = (face(a, GeneratorChoice::EndCap, None), face(b, GeneratorChoice::StartCap, None));
    doc.mate_distance("Z", &top, &bottom, "5 mm", Alignment::Opposed, false).unwrap();
    assert!((z_of(&doc, b) - 19.0).abs() < 1e-9, "B sits 5 mm above A's 14 mm top");
    assert_eq!(doc.configurations_for(a).unwrap(), ["Default", "Tall"]);
    assert_eq!(doc.configuration_of(a).unwrap(), None);

    // A name the part does not have is refused, with the ones it has, and nothing changes.
    let (depth, revision) = (doc.undo_depth(), doc.assembly().semantic_hash());
    let refused = doc.set_configuration(a, Some("Short")).unwrap_err().to_string();
    assert!(refused.contains("Default, Tall"), "{refused}");
    assert_eq!((doc.undo_depth(), doc.assembly().semantic_hash()), (depth, revision));

    // Tall: A shows 28 mm, and the mate reads the exact Tall body, so B rises with it.
    doc.set_configuration(a, Some("Tall")).unwrap();
    doc.finish_loading();
    assert_eq!(doc.undo_depth(), depth + 1, "one undo step");
    assert_eq!(doc.configuration_of(a).unwrap().as_deref(), Some("Tall"));
    assert!((height(&doc, &component_path(a)) - 28.0).abs() < 1e-6);
    assert!((height(&doc, &component_path(twin)) - 14.0).abs() < 1e-6, "the other placement keeps its own");
    assert!((z_of(&doc, b) - 33.0).abs() < 1e-9, "B sits 5 mm above A's 28 mm top");
    assert_eq!(doc.solver().remaining_dof, 6, "A and B solved; the twin floats");

    // Undo and redo walk the change.
    doc.undo();
    doc.finish_loading();
    assert_eq!(doc.configuration_of(a).unwrap(), None);
    assert!((z_of(&doc, b) - 19.0).abs() < 1e-9);
    assert!((height(&doc, &component_path(a)) - 14.0).abs() < 1e-6);
    doc.redo();
    doc.finish_loading();
    assert!((z_of(&doc, b) - 33.0).abs() < 1e-9);

    // Save and reopen: the choice comes back, and the display copies of both shapes.
    doc.save(None).unwrap();
    let mut back = AssemblyDocument::open(&root, options()).unwrap();
    back.finish_loading();
    assert_eq!(back.assembly(), doc.assembly());
    assert_eq!(back.configuration_of(a).unwrap().as_deref(), Some("Tall"));
    assert!((height(&back, &component_path(a)) - 28.0).abs() < 1e-6);
    assert!((height(&back, &component_path(twin)) - 14.0).abs() < 1e-6);
    assert_eq!(back.solver().remaining_dof, 6);

    // A later feature on top: the twin goes Tall as well and a mate stacks it on A.
    back.set_configuration(twin, Some("Tall")).unwrap();
    back.finish_loading();
    assert!((height(&back, &component_path(twin)) - 28.0).abs() < 1e-6);
    let twin_bottom = face(twin, GeneratorChoice::StartCap, None);
    back.mate_distance("T", &face(a, GeneratorChoice::EndCap, None), &twin_bottom, "0 mm", Alignment::Opposed, false)
        .unwrap();
    assert!((z_of(&back, twin) - 28.0).abs() < 1e-9, "the twin stands on A's Tall top");

    // Back to the saved configuration.
    back.set_configuration(a, None).unwrap();
    back.finish_loading();
    assert!((z_of(&back, b) - 19.0).abs() < 1e-9);
    assert!((z_of(&back, twin) - 14.0).abs() < 1e-9);
}

#[test]
fn a_sub_assembly_is_placed_in_the_configuration_the_assembly_above_names() {
    let dir = folder("sub");
    let (b_file, sub_file, root) = (dir.join("b.dwprt"), dir.join("sub.dwasm"), dir.join("top.dwasm"));
    block(20.0, 10.0, 6.0).save(&b_file).unwrap();

    // The sub-assembly: B at its origin in Default, 50 mm up in Raised; saved in Default.
    let mut sub = AssemblyDocument::new_with("Sub", options());
    sub.save(Some(&sub_file)).unwrap();
    let inner = sub.insert(reference(&sub_file, &b_file), Placement::IDENTITY).unwrap();
    sub.finish_loading();
    let raised = sub.add_configuration("Raised").unwrap();
    sub.activate_configuration(raised).unwrap();
    sub.move_to(&[(component_path(inner), Placement::translation(DVec3::new(0.0, 0.0, 50.0)))]).unwrap();
    let default = sub.configuration_id("Default").unwrap();
    sub.activate_configuration(default).unwrap();
    sub.save(None).unwrap();

    let mut top = AssemblyDocument::new_with("Top", options());
    top.save(Some(&root)).unwrap();
    let low = top.insert(reference(&root, &sub_file), Placement::IDENTITY).unwrap();
    let high = top.insert(reference(&root, &sub_file), Placement::translation(DVec3::new(100.0, 0.0, 0.0))).unwrap();
    top.finish_loading();
    assert_eq!(top.configurations_for(high).unwrap(), ["Default", "Raised"]);
    let inside = |c: ComponentId| component_path(c).child(component_path(inner).steps()[0]);
    let shown_z = |doc: &AssemblyDocument, c: ComponentId| {
        let row = doc.table().find(&inside(c)).unwrap();
        doc.table().world(row).translation.z
    };
    assert!(shown_z(&top, high).abs() < 1e-9);
    assert!(top.set_configuration(high, Some("Lowered")).unwrap_err().to_string().contains("Default, Raised"));

    top.set_configuration(high, Some("Raised")).unwrap();
    top.finish_loading();
    assert!((shown_z(&top, high) - 50.0).abs() < 1e-9, "Raised puts B 50 mm up");
    assert!(shown_z(&top, low).abs() < 1e-9, "the other placement keeps the saved configuration");

    // Save and reopen: the file keeps one copy of the sub-assembly, and both placements.
    top.save(None).unwrap();
    let mut back = AssemblyDocument::open(&root, options()).unwrap();
    back.finish_loading();
    assert_eq!(back.configuration_of(high).unwrap().as_deref(), Some("Raised"));
    assert!((shown_z(&back, high) - 50.0).abs() < 1e-9);
    assert!(shown_z(&back, low).abs() < 1e-9);

    // A later change on top: the low placement takes Raised too, then undo puts it back.
    back.set_configuration(low, Some("Raised")).unwrap();
    back.finish_loading();
    assert!((shown_z(&back, low) - 50.0).abs() < 1e-9);
    back.undo();
    back.finish_loading();
    assert!(shown_z(&back, low).abs() < 1e-9);
}
