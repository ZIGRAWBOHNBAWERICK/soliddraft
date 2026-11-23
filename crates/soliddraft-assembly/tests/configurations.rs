//! Configurations and exploded views of real parts, as a user meets them: build, change
//! one configuration, switch, save, reopen, and keep working on top.
//!
//! Block A is 40 x 30 x 14, block B 20 x 10 x 6, both centred in plan and rising along +Z.
//! The mates put B on A's top, a driven gap above it.

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
    let dir = std::env::temp_dir().join(format!("soliddraft-configurations-{}-{name}", std::process::id()));
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

fn new_block(path: &Path, w: f64, d: f64, h: f64) {
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
            options: Default::default(),
        }),
    );
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

struct Rig {
    root: PathBuf,
    c_file: PathBuf,
    doc: AssemblyDocument,
    a: ComponentId,
    b: ComponentId,
}

/// A at the origin, B on its top a driven gap above, both fully mated, saved.
fn rig(name: &str) -> Rig {
    let dir = folder(name);
    let (a_file, b_file, c_file, root) =
        (dir.join("a.dwprt"), dir.join("b.dwprt"), dir.join("c.dwprt"), dir.join("rig.dwasm"));
    new_block(&a_file, 40.0, 30.0, 14.0);
    new_block(&b_file, 20.0, 10.0, 6.0);
    new_block(&c_file, 8.0, 8.0, 8.0);
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&root)).unwrap();
    let a = doc.insert(reference(&root, &a_file), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference(&root, &b_file), Placement::translation(DVec3::new(10.0, 10.0, 30.0))).unwrap();
    doc.finish_loading();
    doc.mate_origin("O", a, true, false).unwrap();
    for (name, n) in [("X", [1.0, 0.0, 0.0]), ("Y", [0.0, 1.0, 0.0])] {
        let side = GeneratorChoice::Side;
        doc.mate_coincident(name, &face(a, side, Some(n)), &face(b, side, Some(n)), Alignment::Aligned, false).unwrap();
    }
    let (top, bottom) = (face(a, GeneratorChoice::EndCap, None), face(b, GeneratorChoice::StartCap, None));
    doc.mate_distance("Z", &top, &bottom, "5 mm", Alignment::Opposed, false).unwrap();
    doc.set_value("gap", "12 mm").unwrap();
    doc.drive_mate("Z", "gap", false).unwrap();
    doc.save(None).unwrap();
    Rig { root, c_file, doc, a, b }
}

fn z_of(doc: &AssemblyDocument, c: ComponentId) -> f64 {
    doc.assembly().component(c).unwrap().placement.translation.z
}

fn shown_z(doc: &AssemblyDocument, c: ComponentId) -> f64 {
    let row = doc.table().find(&component_path(c)).unwrap();
    doc.table().world(row).translation.z
}

#[test]
fn each_configuration_keeps_its_values_placements_and_suppression() {
    let mut r = rig("values");
    assert!((z_of(&r.doc, r.b) - 26.0).abs() < 1e-9, "14 mm top plus the 12 mm gap");

    // Build: an Open configuration with a wide gap, and A's twin suppressed in it.
    let open = r.doc.add_configuration("Open").unwrap();
    assert_eq!(r.doc.assembly().active_configuration().unwrap().name, "Default");
    r.doc.activate_configuration(open).unwrap();
    r.doc.set_value("gap", "30 mm").unwrap();
    assert!((z_of(&r.doc, r.b) - 44.0).abs() < 1e-9);
    let c = r.doc.insert(reference(&r.root, &r.c_file), Placement::translation(DVec3::new(60.0, 0.0, 0.0))).unwrap();
    r.doc.finish_loading();
    r.doc.set_suppressed(&[component_path(c)], true).unwrap();

    // Switch back: Default keeps its own gap; C, added in Open, keeps its state.
    let default = r.doc.configuration_id("Default").unwrap();
    r.doc.activate_configuration(default).unwrap();
    assert!((z_of(&r.doc, r.b) - 26.0).abs() < 1e-9);
    assert!((shown_z(&r.doc, r.b) - 26.0).abs() < 1e-9, "the table follows the truth");
    assert_eq!(
        r.doc.assembly().values.expression(r.doc.assembly().values.id_of("gap").unwrap()).unwrap().text(),
        "12 mm"
    );
    let row = r.doc.table().find(&component_path(c)).unwrap();
    assert!(r.doc.table().flags(row).has(soliddraft_assembly::Flags::SUPPRESSED));
    r.doc.set_suppressed(&[component_path(c)], false).unwrap();
    assert_eq!(r.doc.solver().remaining_dof, 6, "C floats in Default; A and B stay solved");

    // Undo and redo walk the switch.
    r.doc.undo();
    r.doc.undo();
    assert_eq!(r.doc.assembly().active_configuration().unwrap().name, "Open");
    assert!((z_of(&r.doc, r.b) - 44.0).abs() < 1e-9);
    r.doc.redo();
    assert_eq!(r.doc.assembly().active_configuration().unwrap().name, "Default");

    // Save and reopen: the active configuration and the other's state come back.
    let geometry = r.doc.assembly().geometry_hash();
    r.doc.save(None).unwrap();
    let mut back = AssemblyDocument::open(&r.root, options()).unwrap();
    back.finish_loading();
    assert_eq!(back.assembly(), r.doc.assembly());
    assert_eq!(back.assembly().geometry_hash(), geometry);
    let open = back.configuration_id("Open").unwrap();
    back.activate_configuration(open).unwrap();
    assert!((z_of(&back, r.b) - 44.0).abs() < 1e-9);
    assert_ne!(back.assembly().geometry_hash(), geometry, "another configuration is another shape");
    assert_eq!(back.solver().remaining_dof, 0, "Open's stored placements still satisfy its mates");

    // A later feature on top: in Open, the gap is edited again and the mates solve.
    back.set_value("gap", "20 mm").unwrap();
    assert!((z_of(&back, r.b) - 34.0).abs() < 1e-9);
    back.save(None).unwrap();
    let back = AssemblyDocument::open(&r.root, options()).unwrap();
    assert_eq!(back.assembly().active_configuration().unwrap().name, "Open");
    assert!((z_of(&back, r.b) - 34.0).abs() < 1e-9);
    let _ = r.a;
}

#[test]
fn configurations_refuse_what_they_cannot_hold() {
    let mut r = rig("refusals");
    assert!(r.doc.add_configuration("Default").is_err(), "Default is the state the assembly has now");
    let open = r.doc.add_configuration("Open").unwrap();
    assert!(r.doc.add_configuration("Open").is_err());
    let default = r.doc.configuration_id("Default").unwrap();
    assert!(r.doc.delete_configuration(default).is_err(), "the active one stays");
    let depth = r.doc.undo_depth();
    assert!(r.doc.activate_configuration(9999).is_err());
    assert_eq!(r.doc.undo_depth(), depth, "a refusal adds no step");
    r.doc.rename_configuration(open, "Wide").unwrap();
    r.doc.delete_configuration(open).unwrap();
    assert!(r.doc.assembly().configurations.is_empty());
}

#[test]
fn an_exploded_view_moves_what_is_shown_and_never_the_placements() {
    let mut r = rig("explode");
    let truth = r.doc.assembly().semantic_hash();
    let b = component_path(r.b);
    let step = r.doc.add_explode_step(vec![b.clone()], DVec3::Z, 50.0).unwrap();
    assert!(!r.doc.is_exploded());
    r.doc.set_exploded(true).unwrap();
    assert!((shown_z(&r.doc, r.b) - 76.0).abs() < 1e-9, "26 assembled, 50 up");
    assert!((z_of(&r.doc, r.b) - 26.0).abs() < 1e-9, "the placement stays");

    // Editing the step while exploded shows the new distance.
    r.doc.edit_explode_step(step, None, Some(80.0)).unwrap();
    assert!(r.doc.is_exploded());
    assert!((shown_z(&r.doc, r.b) - 106.0).abs() < 1e-9);

    // Any other edit collapses the view first.
    r.doc.set_hidden(&[component_path(r.a)], true).unwrap();
    assert!(!r.doc.is_exploded());
    assert!((shown_z(&r.doc, r.b) - 26.0).abs() < 1e-9);
    r.doc.undo();

    // The worlds a drawing reads come without showing anything.
    let worlds = r.doc.exploded_worlds();
    let (_, w) = worlds.iter().find(|(p, _)| *p == b).unwrap();
    assert!((w.translation.z - 106.0).abs() < 1e-9);
    assert!(!r.doc.is_exploded());

    // Save and reopen: the steps come back, assembled.
    r.doc.save(None).unwrap();
    assert_ne!(r.doc.assembly().semantic_hash(), truth);
    let mut back = AssemblyDocument::open(&r.root, options()).unwrap();
    back.finish_loading();
    assert_eq!(back.assembly().explode.len(), 1);
    assert!(!back.is_exploded());
    back.set_exploded(true).unwrap();
    assert!((shown_z(&back, r.b) - 106.0).abs() < 1e-9);

    // A later feature on top: auto explode replaces the steps; B moves away from A.
    let ids = back.auto_explode(40.0).unwrap();
    assert_eq!(ids.len(), 1, "one step for every component but the one in the middle");
    back.set_exploded(true).unwrap();
    // A sits nearest the middle and stays; B leaves along the line from the middle.
    assert_eq!(shown_z(&back, r.a), 0.0);
    assert!(shown_z(&back, r.b) - 26.0 > 20.0, "{}", shown_z(&back, r.b));
    back.set_exploded(false).unwrap();
    assert!((shown_z(&back, r.b) - 26.0).abs() < 1e-9);
    back.undo();
    assert_eq!(back.assembly().explode.len(), 1);
    assert_eq!(back.assembly().explode[0].id, step);
}

#[test]
fn each_configuration_has_its_own_exploded_view() {
    let mut r = rig("explode-configurations");
    r.doc.add_explode_step(vec![component_path(r.b)], DVec3::Z, 50.0).unwrap();
    let open = r.doc.add_configuration("Open").unwrap();
    r.doc.activate_configuration(open).unwrap();
    assert_eq!(r.doc.assembly().explode.len(), 1, "a new configuration starts with the view it was made from");
    let first = r.doc.assembly().explode[0].id;
    r.doc.delete_explode_step(first).unwrap();
    assert!(r.doc.set_exploded(true).is_err(), "nothing to explode in Open now");
    let default = r.doc.configuration_id("Default").unwrap();
    r.doc.activate_configuration(default).unwrap();
    assert_eq!(r.doc.assembly().explode.len(), 1);
    r.doc.set_exploded(true).unwrap();

    // Deleting a component takes it out of every configuration's view, and undo brings it back.
    r.doc.set_exploded(false).unwrap();
    let held = r.doc.delete(&[r.b]);
    assert!(held.is_err(), "B's mates hold it");
    let paths: Vec<OccurrencePath> = vec![component_path(r.a)];
    r.doc.add_explode_step(paths, DVec3::X, 10.0).unwrap();
    assert_eq!(r.doc.assembly().explode.len(), 2);
}
