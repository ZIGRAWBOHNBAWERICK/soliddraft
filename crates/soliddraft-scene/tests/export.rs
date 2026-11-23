//! URDF and OpenUSD export with joints, read back through this crate's own importers:
//! a grounded shaft, a second shaft on a limited Hinge, a third on a free Hinge geared to
//! the second, and a loose block no mate holds.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{DQuat, DVec3};
use soliddraft_assembly::{
    Alignment, AssemblyDocument, ComponentId, EndpointSelector, FaceSelector, GeneratorChoice, KernelSource,
    OccurrencePath, OpenOptions, Placement, Step,
};
use soliddraft_document::{CacheOnlyStore, DocRef, identify_file};
use soliddraft_kernel_api::Kernel;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, ProfileSource, SketchFeature,
    SketchPlane,
};
use soliddraft_scene::export::{self, Articulation};
use soliddraft_scene::{JointKind, Options, Scene, read};

fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("soliddraft-export-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn part(path: &Path, circle: bool) {
    let mut doc = Document::new("Part", kernel());
    let mut sketch = Sketch::new();
    if circle {
        sketch.circle_at(P2::new(0.0, 0.0), 10.0).unwrap();
    } else {
        sketch.rectangle(P2::new(0.0, 0.0), 10.0, 10.0).unwrap();
    }
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
            end: EndCondition::Blind { depth: 10.0 },
            reverse: false,
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

fn cylinder(component: ComponentId) -> EndpointSelector {
    EndpointSelector {
        component,
        face: FaceSelector::Generator {
            feature: "Boss-Extrude1".into(),
            generator: GeneratorChoice::Side,
            normal: None,
        },
    }
}

fn datum(component: ComponentId, plane: &str) -> EndpointSelector {
    EndpointSelector { component, face: FaceSelector::Plane(plane.into()) }
}

fn options() -> OpenOptions {
    OpenOptions {
        display: Some(Arc::new(KernelSource::new(kernel()))),
        revisions: Some(Arc::new(CacheOnlyStore::new())),
        ..OpenOptions::default()
    }
}

fn rig(dir: &Path) -> AssemblyDocument {
    let (shaft, block) = (dir.join("shaft.dwprt"), dir.join("block.dwprt"));
    part(&shaft, true);
    part(&block, false);
    let file = dir.join("rig.dwasm");
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&file)).unwrap();
    let a = doc.insert(reference(&file, &shaft), Placement::IDENTITY).unwrap();
    let b = doc
        .insert(reference(&file, &shaft), Placement::new(DQuat::from_rotation_z(0.2), DVec3::new(2.0, 1.0, 20.0)))
        .unwrap();
    let c = doc.insert(reference(&file, &shaft), Placement::translation(DVec3::new(0.0, 0.0, 40.0))).unwrap();
    doc.insert(reference(&file, &block), Placement::translation(DVec3::new(50.0, 0.0, 0.0))).unwrap();
    doc.finish_loading();
    doc.mate_origin("O", a, true, false).unwrap();
    let planes = |x: ComponentId| [datum(a, "Top plane"), datum(x, "Top plane")];
    let [pa, pb] = planes(b);
    doc.mate_hinge_limited(
        "Lid",
        [&cylinder(a), &cylinder(b)],
        [&pa, &pb],
        [Alignment::Aligned; 2],
        Some([-30.0, 45.0]),
        false,
    )
    .unwrap();
    let [pa, pc] = planes(c);
    doc.mate_hinge("Spin", [&cylinder(a), &cylinder(c)], [&pa, &pc], Alignment::Aligned, Alignment::Aligned, false)
        .unwrap();
    doc.mate_gear("G", ["Lid", "Spin"], [1.0, 2.0], false, false).unwrap();
    doc.finish_loading();
    doc.save(None).unwrap();
    doc
}

fn shafts(dir: &Path, name: &str) -> (AssemblyDocument, [ComponentId; 2]) {
    let shaft = dir.join("shaft.dwprt");
    part(&shaft, true);
    let file = dir.join("hinge.dwasm");
    let mut doc = AssemblyDocument::new_with(name, options());
    doc.save(Some(&file)).unwrap();
    let a = doc.insert(reference(&file, &shaft), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference(&file, &shaft), Placement::IDENTITY).unwrap();
    doc.finish_loading();
    (doc, [a, b])
}

fn component(id: ComponentId) -> OccurrencePath {
    OccurrencePath::root().child(Step::component(id))
}

fn check_joints(a: &Articulation) {
    let kinds: Vec<JointKind> = a.joints.iter().map(|j| j.kind).collect();
    assert_eq!(a.links.len(), 5, "base_link and four parts");
    assert_eq!(
        kinds.iter().filter(|k| **k == JointKind::Fixed).count(),
        2,
        "the grounded shaft and the loose block: {kinds:?}"
    );
    let lid = a.joints.iter().find(|j| j.mates.contains(&"Lid".into())).unwrap();
    assert_eq!((lid.kind, lid.limits), (JointKind::Revolute, Some([-30.0, 45.0])));
    assert!((DVec3::from_array(lid.axis).z.abs() - 1.0).abs() < 1e-9, "{:?}", lid.axis);
    let spin = a.joints.iter().find(|j| j.mates.contains(&"Spin".into())).unwrap();
    assert_eq!(spin.kind, JointKind::Continuous);
    let (leader, k) = spin.mimic.clone().expect("the gear makes Spin follow Lid");
    assert_eq!(leader, lid.name);
    // A 1:2 gear, opposite by default: Spin turns twice as far the other way.
    assert!((k + 2.0).abs() < 1e-12, "{k}");
    assert!(a.notes.iter().any(|n| n.contains("fixed where it stands")), "{:?}", a.notes);
}

/// The joints the importer read, by kind, with each moving joint's axis in the world.
fn read_back(scene: &Scene) -> Vec<(JointKind, Option<[f64; 2]>, DVec3, bool)> {
    let mut out: Vec<_> = scene
        .joints
        .iter()
        .map(|j| {
            let world = scene.world(j.parent) * j.frame;
            (j.kind, j.limits, world.transform_vector3(j.axis).normalize(), j.mimic.is_some())
        })
        .collect();
    out.sort_by_key(|(k, ..)| format!("{k:?}"));
    out
}

fn check_read(scene: &Scene, what: &str) {
    let joints = read_back(scene);
    let moving: Vec<_> = joints.iter().filter(|(k, ..)| *k != JointKind::Fixed).collect();
    assert_eq!(moving.len(), 2, "{what}: {joints:?}");
    for (kind, limits, axis, _) in &moving {
        assert!((axis.z.abs() - 1.0).abs() < 1e-6, "{what}: a joint turns about Z, not {axis}");
        if *kind == JointKind::Revolute {
            let [lo, hi] = limits.unwrap();
            assert!((lo + 30.0).abs() < 1e-6 && (hi - 45.0).abs() < 1e-6, "{what}: {lo} {hi}");
        }
    }
    assert!(joints.iter().any(|j| j.3), "{what}: the mimic joint comes back");
    assert_eq!(scene.nodes.iter().filter(|n| n.body.is_some()).count(), 4, "{what}: four bodies");
    // Each body's mesh sits where the assembly placed it: the shafts on the axis, the
    // block 50 mm along X, every top at 10 mm.
    let mut centres: Vec<(f64, f64)> = scene
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.body.is_some())
        .map(|(i, n)| {
            let m = &scene.meshes[n.body.unwrap().mesh];
            let w = scene.world(i);
            let pts: Vec<DVec3> = m.positions.iter().map(|p| w.transform_point3(*p)).collect();
            let (lo, hi) = pts.iter().fold((f64::MAX, f64::MIN), |(l, h), p| (l.min(p.x), h.max(p.x)));
            ((lo + hi) * 0.5, pts.iter().map(|p| p.z).fold(f64::MIN, f64::max))
        })
        .collect();
    centres.sort_by(|a, b| a.0.total_cmp(&b.0));
    for ((x, top), want) in centres.iter().zip([0.0, 0.0, 0.0, 50.0]) {
        assert!((x - want).abs() < 1e-3 && (top - 10.0).abs() < 1e-3, "{what}: {centres:?}");
    }
}

#[test]
fn urdf_and_usd_carry_the_hinges_the_gear_and_the_placements() {
    let dir = folder("robot");
    let mut doc = rig(&dir);
    let articulation = export::articulate(&mut doc).unwrap();
    check_joints(&articulation);

    let urdf = dir.join("out/rig.urdf");
    std::fs::create_dir_all(urdf.parent().unwrap()).unwrap();
    let (_, files) = export::export(&mut doc, &urdf).unwrap();
    assert_eq!(files.last(), Some(&urdf));
    assert!(files.iter().any(|f| f.extension().is_some_and(|e| e == "stl")));
    check_read(&read(&urdf, &Options::default()).unwrap(), "URDF");

    let usd = dir.join("out/rig.usda");
    export::export(&mut doc, &usd).unwrap();
    check_read(&read(&usd, &Options::default()).unwrap(), "USD");

    // The export changes nothing in the assembly, and refuses another extension.
    assert!(!doc.is_dirty());
    assert!(export::export(&mut doc, &dir.join("out/rig.step")).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn export_uses_saved_placements_while_exploded_or_dragging() {
    let dir = folder("accepted");
    let (mut doc, [a, b]) = shafts(&dir, "Rig");
    doc.mate_origin("Ground", a, true, false).unwrap();
    let loose = component(b);
    doc.add_explode_step(vec![loose.clone()], DVec3::X, 75.0).unwrap();
    doc.save(None).unwrap();
    let mut doc = AssemblyDocument::open(dir.join("hinge.dwasm"), options()).unwrap();
    doc.finish_loading();
    let row = doc.table().find(&loose).unwrap();
    let paths = [dir.join("rig.urdf"), dir.join("rig.usda")];
    let files = |doc: &mut AssemblyDocument| paths.each_ref().map(|p| export::export_files(doc, p).unwrap().1);
    let accepted = files(&mut doc);

    doc.set_exploded(true).unwrap();
    assert_eq!(doc.table().world(row).translation, DVec3::X * 75.0);
    let exploded = files(&mut doc);
    assert!(doc.is_exploded());
    assert_eq!(doc.table().world(row).translation, DVec3::X * 75.0);

    doc.begin_drag(loose).unwrap();
    let later = DVec3::new(30.0, 20.0, 0.0);
    doc.preview_drag(Placement::translation(later)).unwrap();
    assert_eq!(doc.table().world(row).translation, later);
    let preview = files(&mut doc);
    assert!(doc.dragging());
    assert_eq!(doc.table().world(row).translation, later);
    doc.commit_drag().unwrap();
    assert_ne!(files(&mut doc), accepted, "export includes the drag after committing it");
    std::fs::remove_dir_all(dir).unwrap();
    for (format, (expected, (exploded, preview))) in
        ["URDF", "USD"].into_iter().zip(accepted.iter().zip(exploded.iter().zip(&preview)))
    {
        assert!(expected == exploded, "{format} changed with the exploded view");
        assert!(expected == preview, "{format} changed during a drag preview");
    }
}
