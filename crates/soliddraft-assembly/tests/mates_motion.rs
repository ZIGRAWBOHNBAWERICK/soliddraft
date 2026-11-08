//! Bounded mates through real part loading, transactions, saving and reopening.
use glam::{DQuat, DVec3};
use soliddraft_assembly::{
    Alignment, AssemblyDocument, ComponentId, EndpointSelector, FaceSelector, GeneratorChoice, KernelSource, MateEdit,
    OpenOptions, Placement, RefusalCode,
};
use soliddraft_document::{CacheOnlyStore, DocRef, identify_file};
use soliddraft_kernel_api::Kernel;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, ProfileSource, SketchFeature,
    SketchPlane,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
fn kernel() -> Arc<dyn Kernel> {
    Arc::new(CatallaxKernel::new())
}
fn options() -> OpenOptions {
    OpenOptions {
        display: Some(Arc::new(KernelSource::new(kernel()))),
        revisions: Some(Arc::new(CacheOnlyStore::new())),
        check_freshness: true,
        ..OpenOptions::default()
    }
}
#[derive(Clone, Copy)]
enum Profile {
    Block,
    Circle,
    Trapezoid,
}
fn part(path: &Path, profile: Profile) {
    let mut doc = Document::new("Part", kernel());
    let mut sketch = Sketch::new();
    match profile {
        Profile::Circle => {
            sketch.circle_at(P2::new(0.0, 0.0), 10.0).unwrap();
        }
        Profile::Block => {
            sketch.rectangle(P2::new(0.0, 0.0), 40.0, 30.0).unwrap();
        }
        Profile::Trapezoid => {
            sketch
                .polyline_loop(&[
                    P2::new(-20.0, -15.0),
                    P2::new(20.0, -15.0),
                    P2::new(10.0, 15.0),
                    P2::new(-10.0, 15.0),
                ])
                .unwrap();
        }
    }
    solve(&mut sketch, None, true);
    let kind = FeatureKind::Sketch(SketchFeature {
        plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
        sketch,
    });
    let sketch = doc
        .execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
        .unwrap()
        .created
        .unwrap();
    doc.execute(Command::AddFeature {
        kind: FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: sketch },
            operation: ExtrudeOp::Boss,
            end: EndCondition::Blind { depth: 10.0 },
            reverse: false,
        }),
        name: None,
        position: None,
        via_assistant: false,
    })
    .unwrap();
    doc.save(path).unwrap();
}
fn datum(component: ComponentId, plane: &str) -> EndpointSelector {
    EndpointSelector { component, face: FaceSelector::Plane(plane.into()) }
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
struct Rig {
    dir: PathBuf,
    file: PathBuf,
    doc: AssemblyDocument,
    a: ComponentId,
    b: ComponentId,
}
fn rig(name: &str, profile: Profile) -> Rig {
    let dir = std::env::temp_dir().join(format!("soliddraft-motion-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("part.dwprt");
    part(&source, profile);
    let file = dir.join("rig.dwasm");
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&file)).unwrap();
    let identity = identify_file(&source).unwrap();
    let mut reference = DocRef::new(identity.id, identity.kind, "", identity.revision);
    reference.set_path(&file, &source);
    let a = doc.insert(reference.clone(), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference, Placement::new(DQuat::from_rotation_z(0.4), DVec3::new(3.0, 4.0, 20.0))).unwrap();
    doc.finish_loading();
    doc.mate_origin("O", a, true, false).unwrap();
    Rig { dir, file, doc, a, b }
}
fn bounded(doc: &mut AssemblyDocument, name: &str) {
    let status = doc.solver();
    assert!(status.diagnostics.is_empty(), "{status:?}");
    let p = status.bounded.iter().find(|p| p.mate == name).unwrap_or_else(|| panic!("{name}: {status:?}"));
    assert_eq!(p.certainty, "bounded");
    assert!(p.residual_mm <= 1e-9 && p.residual_angle <= 1e-9);
}
#[test]
fn angle_with_rotation_locked_slot_replays_the_application_request() {
    use soliddraft_assembly::SlotInput;
    let dir = std::env::temp_dir().join(format!("soliddraft-angle-slot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("rig.dwasm");
    let mut doc = AssemblyDocument::new_with("Rig", options());
    doc.save(Some(&file)).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mates-angle-slot");
    let mut parts = Vec::new();
    for (name, at) in
        [("plate.dwprt", Placement::IDENTITY), ("pin.dwprt", Placement::translation(DVec3::new(40.0, 7.0, 30.0)))]
    {
        let path = dir.join(name);
        std::fs::copy(source.join(name), &path).unwrap();
        let id = identify_file(&path).unwrap();
        let mut reference = DocRef::new(id.id, id.kind, "", id.revision);
        reference.set_path(&file, &path);
        parts.push(doc.insert(reference, at).unwrap());
    }
    doc.finish_loading();
    let (a, b) = (parts[0], parts[1]);
    doc.mate_origin("O", a, true, false).unwrap();
    doc.mate_angle("A", &datum(a, "Right plane"), &datum(b, "Right plane"), &datum(a, "Top plane"), 30.0, false)
        .unwrap();
    let slot = EndpointSelector {
        component: a,
        face: FaceSelector::Generator {
            feature: "Cut-Extrude1".into(),
            generator: GeneratorChoice::Side,
            normal: Some([0.0, 1.0, 0.0]),
        },
    };
    doc.mate_slot("SL", &slot, &cylinder(b), Alignment::Aligned, &SlotInput::Free, true, false).unwrap();
    bounded(&mut doc, "SL");
    assert_eq!(doc.assembly().component(a).unwrap().placement, Placement::IDENTITY);
    doc.save(None).unwrap();
    let mut reopened = AssemblyDocument::open(&file, options()).unwrap();
    reopened.finish_loading();
    bounded(&mut reopened, "SL");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn signed_angle_edits_and_reopens_with_a_bound_on_the_stored_pose() {
    let mut r = rig("angle", Profile::Block);
    let a = datum(r.a, "Right plane");
    let b = datum(r.b, "Right plane");
    let reference = datum(r.a, "Top plane");
    r.doc.mate_angle("A", &a, &b, &reference, -30.0, false).unwrap();
    bounded(&mut r.doc, "A");
    r.doc.edit_mates(&[("A".into(), MateEdit::Degrees(45.0))]).unwrap();
    bounded(&mut r.doc, "A");
    r.doc.save(None).unwrap();
    let bytes = std::fs::read(&r.file).unwrap();
    let mut reopened = AssemblyDocument::open(&r.file, options()).unwrap();
    reopened.finish_loading();
    bounded(&mut reopened, "A");
    reopened.save(None).unwrap();
    assert_eq!(bytes, std::fs::read(&r.file).unwrap());
    std::fs::remove_dir_all(r.dir).unwrap();
}
#[test]
fn limit_angle_refuses_invalid_ranges_without_changing_truth() {
    let mut r = rig("limit", Profile::Block);
    let a = datum(r.a, "Right plane");
    let b = datum(r.b, "Right plane");
    let reference = datum(r.a, "Top plane");
    r.doc.mate_limit_angle("L", &a, &b, &reference, [-15.0, 15.0], false).unwrap();
    bounded(&mut r.doc, "L");
    let before = serde_json::to_vec(r.doc.assembly()).unwrap();
    let err = r.doc.edit_mates(&[("L".into(), MateEdit::AngleLimits([90.0, -90.0]))]).unwrap_err();
    assert_eq!(err.code, RefusalCode::InvalidValue);
    assert_eq!(before, serde_json::to_vec(r.doc.assembly()).unwrap());
    std::fs::remove_dir_all(r.dir).unwrap();
}
#[test]
fn rotated_cylinders_have_axis_distances_and_curved_tangency() {
    let mut r = rig("curved", Profile::Circle);
    let a = cylinder(r.a);
    let b = cylinder(r.b);
    r.doc.mate_distance("D", &a, &b, "15 mm", Alignment::Aligned, false).unwrap();
    bounded(&mut r.doc, "D");
    r.doc.delete_mate("D").unwrap();
    r.doc.mate_tangent("T", &a, &b, Alignment::Opposed, false).unwrap();
    bounded(&mut r.doc, "T");
    let at = r.doc.assembly().component(r.b).unwrap().placement.translation;
    assert!((DVec3::new(at.x, at.y, 0.0).length() - 10.0).abs() < 1e-8);
    std::fs::remove_dir_all(r.dir).unwrap();
}
#[test]
fn a_rotated_distance_has_a_bounded_measurement() {
    let mut r = rig("plane", Profile::Block);
    let a = datum(r.a, "Right plane");
    let b = datum(r.b, "Right plane");
    let reference = datum(r.a, "Top plane");
    r.doc.mate_angle("A", &a, &b, &reference, 30.0, false).unwrap();
    let a = datum(r.a, "Top plane");
    let b = datum(r.b, "Top plane");
    r.doc.mate_distance("D", &a, &b, "5 mm", Alignment::Aligned, false).unwrap();
    bounded(&mut r.doc, "D");
    let measurement = r.doc.measure_planes(&a, &b).unwrap();
    assert!(measurement.lo <= 5.0 + 1e-9 && measurement.hi >= 5.0 - 1e-9);
    std::fs::remove_dir_all(r.dir).unwrap();
}
#[test]
fn nonparallel_width_centres_real_sloping_faces_and_names_the_other_modes() {
    use soliddraft_assembly::WidthInput;
    let mut r = rig("nonparallel", Profile::Trapezoid);
    let side = |component, line| EndpointSelector {
        component,
        face: FaceSelector::Label(format!("Boss-Extrude1 · Side of line {line}")),
    };
    let (g0, g1, t0, t1) = (side(r.a, 2), side(r.a, 4), side(r.b, 2), side(r.b, 4));
    for mode in [
        WidthInput::Free,
        WidthInput::Dimension { distance: "2 mm".into(), flipped: false },
        WidthInput::Percent { percent: 25.0, flipped: false },
    ] {
        let error = r.doc.mate_width("W", [&g0, &g1], [&t0, &t1], Alignment::Aligned, &mode, false).unwrap_err();
        assert!(error.message.contains("Width between nonparallel faces supports Centered for now"), "{error}");
    }
    r.doc.mate_width("W", [&g0, &g1], [&t0, &t1], Alignment::Aligned, &WidthInput::Centered, false).unwrap();
    bounded(&mut r.doc, "W");
    r.doc.mate_linear_coupler("K", &g0, &t0, [None, None], [1.0, 2.0], false, false).unwrap();
    bounded(&mut r.doc, "K");
    std::fs::remove_dir_all(r.dir).unwrap();
}
#[test]
fn point_point_distance_uses_persistent_vertices() {
    let mut r = rig("point-distance", Profile::Block);
    let vertex = |component| EndpointSelector {
        component,
        face: FaceSelector::Vertex(vec![
            "Boss-Extrude1 · End cap".into(),
            "Boss-Extrude1 · Side of line 2".into(),
            "Boss-Extrude1 · Side of line 3".into(),
        ]),
    };
    r.doc.mate_distance("D", &vertex(r.a), &vertex(r.b), "12 mm", Alignment::Aligned, false).unwrap();
    bounded(&mut r.doc, "D");
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn zero_point_distance_accepts_a_strict_fully_constrained_assembly() {
    let mut r = rig("zero-point-distance", Profile::Block);
    for plane in ["Right plane", "Top plane"] {
        r.doc.mate_parallel(plane, &datum(r.a, plane), &datum(r.b, plane), Alignment::Aligned, false).unwrap();
    }
    let vertex = |component| EndpointSelector {
        component,
        face: FaceSelector::Vertex(vec![
            "Boss-Extrude1 · End cap".into(),
            "Boss-Extrude1 · Side of line 2".into(),
            "Boss-Extrude1 · Side of line 3".into(),
        ]),
    };
    r.doc.mate_distance("D", &vertex(r.a), &vertex(r.b), "0 mm", Alignment::Aligned, true).unwrap();
    assert_eq!(r.doc.solver().remaining_dof, 0);
    bounded(&mut r.doc, "D");
    r.doc.save(None).unwrap();
    let mut reopened = AssemblyDocument::open(&r.file, options()).unwrap();
    reopened.finish_loading();
    assert_eq!(reopened.solver().remaining_dof, 0);
    let update = reopened.rebuild();
    assert!(update.mate_refusal.is_none(), "{update:?}");
    reopened.finish_loading();
    assert_eq!(reopened.solver().remaining_dof, 0);
    bounded(&mut reopened, "D");
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn zero_axis_distance_leaves_only_the_axial_translation() {
    let mut r = rig("zero-axis-distance", Profile::Circle);
    for plane in ["Right plane", "Top plane"] {
        r.doc.mate_parallel(plane, &datum(r.a, plane), &datum(r.b, plane), Alignment::Aligned, false).unwrap();
    }
    r.doc.mate_distance("D", &cylinder(r.a), &cylinder(r.b), "0 mm", Alignment::Aligned, false).unwrap();
    let status = r.doc.solver();
    assert_eq!(status.remaining_dof, 1, "{status:?}");
    assert_eq!(status.free.iter().map(|f| f.motion.as_str()).collect::<Vec<_>>(), ["translate_z"]);
    r.doc
        .mate_distance("Axial", &datum(r.a, "Top plane"), &datum(r.b, "Top plane"), "5 mm", Alignment::Aligned, true)
        .unwrap();
    r.doc.save(None).unwrap();
    let mut reopened = AssemblyDocument::open(&r.file, options()).unwrap();
    reopened.finish_loading();
    assert_eq!(reopened.solver().remaining_dof, 0);
    let update = reopened.rebuild();
    assert!(update.mate_refusal.is_none(), "{update:?}");
    reopened.finish_loading();
    assert_eq!(reopened.solver().remaining_dof, 0);
    bounded(&mut reopened, "D");
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn pre_b5_axis_aligned_assembly_keeps_exact_proofs_and_archive_bytes() {
    let dir = std::env::temp_dir().join(format!("soliddraft-motion-{}-exact-fixture", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let files: [(&str, &[u8]); 3] = [
        ("a.dwprt", include_bytes!("fixtures/mates-axis-aligned/a.dwprt")),
        ("b.dwprt", include_bytes!("fixtures/mates-axis-aligned/b.dwprt")),
        ("rig.dwasm", include_bytes!("fixtures/mates-axis-aligned/rig.dwasm")),
    ];
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    let mut doc = AssemblyDocument::open(dir.join("rig.dwasm"), options()).unwrap();
    doc.finish_loading();
    assert!(doc.solver().bounded.is_empty());
    for (_, measurement) in doc.mate_measurements() {
        if let Some(measurement) = measurement.unwrap() {
            assert_eq!(measurement.certainty, soliddraft_kernel_api::Certainty::Exact);
            assert!(!measurement.rational.is_empty());
        }
    }
    let report = doc.rebuild();
    assert!(report.refusal.is_none() && report.mate_refusal.is_none(), "{report:?}");
    doc.finish_loading();
    doc.save(None).unwrap();
    assert_eq!(std::fs::read(dir.join("rig.dwasm")).unwrap(), files[2].1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn orthogonal_width_keeps_its_bisector_choice_while_the_part_turns() {
    let mut r = rig("orthogonal-width", Profile::Block);
    let (g0, g1, t0, t1) =
        (datum(r.a, "Right plane"), datum(r.a, "Top plane"), datum(r.b, "Right plane"), datum(r.b, "Top plane"));
    let width = r
        .doc
        .mate_width("W", [&g0, &g1], [&t0, &t1], Alignment::Aligned, &soliddraft_assembly::WidthInput::Centered, false)
        .unwrap();
    bounded(&mut r.doc, "W");
    let measurements = r.doc.mate_measurements();
    assert!(measurements.into_iter().find(|(id, _)| *id == width).unwrap().1.unwrap().is_none());
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn drag_previews_stop_at_distance_limits_and_release_is_one_undo_step() {
    let mut r = rig("drag-distance", Profile::Block);
    r.doc
        .mate_limit_distance(
            "L",
            &datum(r.a, "Top plane"),
            &datum(r.b, "Top plane"),
            ["0 mm", "10 mm"],
            Alignment::Aligned,
            false,
        )
        .unwrap();
    let path = soliddraft_assembly::OccurrencePath::root().child(soliddraft_assembly::Step::component(r.b));
    let before = r.doc.assembly().clone();
    let depth = r.doc.undo_depth();
    r.doc.begin_drag(path.clone()).unwrap();
    for z in [3.0, 5.0, 40.0] {
        r.doc.preview_drag(Placement::translation(DVec3::new(4.0, 7.0, z))).unwrap();
        assert_eq!(&before, r.doc.assembly());
        assert_eq!(depth, r.doc.undo_depth());
    }
    let row = r.doc.table().find(&path).unwrap();
    assert_eq!(r.doc.table().local(row).translation, DVec3::new(4.0, 7.0, 10.0));
    r.doc.commit_drag().unwrap();
    assert_eq!(r.doc.undo_depth(), depth + 1);
    assert_eq!(r.doc.assembly().component(r.b).unwrap().placement.translation.z, 10.0);
    assert!(r.doc.undo());
    assert_eq!(&before, r.doc.assembly());
    let redo = r.doc.redo_depth();
    r.doc.begin_drag(path.clone()).unwrap();
    r.doc.preview_drag(Placement::translation(DVec3::new(9.0, 5.0, -20.0))).unwrap();
    r.doc.cancel_drag();
    assert_eq!(&before, r.doc.assembly());
    assert_eq!(r.doc.redo_depth(), redo);
    assert_eq!(r.doc.table().local(row), before.component(r.b).unwrap().placement);
    assert!(r.doc.redo());
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn hinge_stops_hold_during_rotation_and_invalid_updates_keep_the_preview() {
    let mut r = rig("drag-hinge", Profile::Circle);
    r.doc
        .mate_hinge(
            "H",
            [&cylinder(r.a), &cylinder(r.b)],
            [&datum(r.a, "Top plane"), &datum(r.b, "Top plane")],
            Alignment::Aligned,
            Alignment::Aligned,
            false,
        )
        .unwrap();
    r.doc.edit_mates(&[("H".into(), MateEdit::AngleLimits([-30.0, 30.0]))]).unwrap();
    let path = soliddraft_assembly::OccurrencePath::root().child(soliddraft_assembly::Step::component(r.b));
    let depth = r.doc.undo_depth();
    r.doc.begin_drag(path.clone()).unwrap();
    for degrees in [15.0_f64, 80.0, -70.0] {
        r.doc
            .preview_drag(Placement::new(DQuat::from_rotation_z(degrees.to_radians()), DVec3::new(20.0, 10.0, 9.0)))
            .unwrap();
        let row = r.doc.table().find(&path).unwrap();
        let pose = r.doc.table().local(row);
        let expected = degrees.clamp(-30.0, 30.0).to_radians();
        assert!(pose.rotation.angle_between(DQuat::from_rotation_z(expected)) < 1e-7, "{pose:?}");
        assert!(pose.translation.length() < 1e-8);
    }
    let row = r.doc.table().find(&path).unwrap();
    let shown = r.doc.table().local(row);
    assert_eq!(
        r.doc.preview_drag(Placement { rotation: DQuat::IDENTITY, translation: DVec3::NAN }).unwrap_err().code,
        RefusalCode::InvalidValue
    );
    assert_eq!(shown, r.doc.table().local(row));
    r.doc.commit_drag().unwrap();
    assert_eq!(r.doc.undo_depth(), depth + 1);
    bounded(&mut r.doc, "H");
    r.doc.save(None).unwrap();
    let mut reopened = AssemblyDocument::open(&r.file, options()).unwrap();
    reopened.finish_loading();
    bounded(&mut reopened, "H");
    assert_eq!(r.doc.assembly(), reopened.assembly());
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn limit_angle_drag_and_saving_a_preview_keep_accepted_truth() {
    let mut r = rig("drag-angle", Profile::Block);
    r.doc
        .mate_limit_angle(
            "A",
            &datum(r.a, "Right plane"),
            &datum(r.b, "Right plane"),
            &datum(r.a, "Top plane"),
            [-10.0, 15.0],
            false,
        )
        .unwrap();
    let before = r.doc.assembly().clone();
    let path = soliddraft_assembly::OccurrencePath::root().child(soliddraft_assembly::Step::component(r.b));
    r.doc.begin_drag(path.clone()).unwrap();
    r.doc.preview_drag(Placement::new(DQuat::from_rotation_z(70_f64.to_radians()), DVec3::new(6.0, 7.0, 8.0))).unwrap();
    let row = r.doc.table().find(&path).unwrap();
    assert!(r.doc.table().local(row).rotation.angle_between(DQuat::from_rotation_z(15_f64.to_radians())) < 1e-7);
    r.doc.save(None).unwrap();
    let mut reopened = AssemblyDocument::open(&r.file, options()).unwrap();
    reopened.finish_loading();
    assert_eq!(reopened.assembly(), &before);
    r.doc.cancel_drag();
    assert_eq!(r.doc.table().local(row), before.component(r.b).unwrap().placement);
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn dragging_either_end_of_a_floating_lock_moves_the_whole_group() {
    let mut r = rig("drag-floating", Profile::Block);
    r.doc.delete_mate("O").unwrap();
    r.doc.mate_lock("Lock", r.a, r.b, false).unwrap();
    let before = r
        .doc
        .assembly()
        .component(r.a)
        .unwrap()
        .placement
        .inverse()
        .compose(&r.doc.assembly().component(r.b).unwrap().placement);
    let path = soliddraft_assembly::OccurrencePath::root().child(soliddraft_assembly::Step::component(r.b));
    let target = Placement::new(DQuat::from_rotation_z(0.7), DVec3::new(20.0, 25.0, 30.0));
    r.doc.begin_drag(path).unwrap();
    r.doc.preview_drag(target).unwrap();
    r.doc.commit_drag().unwrap();
    let a = r.doc.assembly().component(r.a).unwrap().placement;
    let b = r.doc.assembly().component(r.b).unwrap().placement;
    assert!(b.translation.distance(target.translation) < 1e-8);
    assert!(b.rotation.angle_between(target.rotation) < 1e-7);
    let relative = a.inverse().compose(&b);
    assert!(relative.translation.distance(before.translation) < 1e-8);
    assert!(relative.rotation.angle_between(before.rotation) < 1e-7);
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn ordinary_edits_never_capture_a_drag_preview_in_history() {
    for edit in ["hide", "suppress", "move", "move_by"] {
        for cancel_explicitly in [false, true] {
            let mut r = rig(&format!("drag-interleave-{edit}-{cancel_explicitly}"), Profile::Block);
            let path = soliddraft_assembly::OccurrencePath::root().child(soliddraft_assembly::Step::component(r.b));
            let before = r.doc.assembly().clone();
            r.doc.begin_drag(path.clone()).unwrap();
            r.doc.preview_drag(Placement::translation(DVec3::new(10.0, 0.0, 0.0))).unwrap();
            match edit {
                "hide" => r.doc.set_hidden(std::slice::from_ref(&path), true).unwrap(),
                "suppress" => r.doc.set_suppressed(std::slice::from_ref(&path), true).unwrap(),
                "move" => r.doc.move_to(&[(path.clone(), Placement::translation(DVec3::new(20.0, 0.0, 0.0)))]).unwrap(),
                "move_by" => r.doc.move_by(std::slice::from_ref(&path), &Placement::translation(DVec3::X)).unwrap(),
                _ => unreachable!(),
            }
            if cancel_explicitly {
                r.doc.cancel_drag();
            }
            assert!(r.doc.undo());
            assert_eq!(&before, r.doc.assembly());
            let row = r.doc.table().find(&path).unwrap();
            let accepted = r.doc.assembly().component(r.b).unwrap().placement;
            assert_eq!(r.doc.table().local(row), accepted, "{edit}, cancel {cancel_explicitly}");
            assert_eq!(r.doc.table().world(row), accepted);
            assert!(r.doc.redo());
            let accepted = r.doc.assembly().component(r.b).unwrap().placement;
            assert_eq!(r.doc.table().local(row), accepted);
            assert_eq!(r.doc.table().world(row), accepted);
            if edit == "move_by" {
                assert_eq!(accepted.translation, before.component(r.b).unwrap().placement.translation + DVec3::X);
            }
            std::fs::remove_dir_all(r.dir).unwrap();
        }
    }
}
