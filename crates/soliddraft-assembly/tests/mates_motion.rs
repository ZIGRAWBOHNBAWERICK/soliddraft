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

#[test]
fn limited_hinge_aligns_an_exactly_reversed_axis_and_corrects_reversed_drag_targets() {
    for (index, limits) in [[-30.0, 30.0], [-180.0, 180.0], [0.0, 0.0]].into_iter().enumerate() {
        let mut r = rig(&format!("hinge-antipodal-{index}"), Profile::Circle);
        let path = soliddraft_assembly::OccurrencePath::root().child(soliddraft_assembly::Step::component(r.b));
        let reversed = Placement { rotation: DQuat::from_xyzw(0.0, 1.0, 0.0, 0.0), translation: DVec3::ZERO };
        r.doc.move_to(&[(path.clone(), reversed)]).unwrap();
        r.doc
            .mate_hinge(
                "Unlimited",
                [&cylinder(r.a), &cylinder(r.b)],
                [&datum(r.a, "Top plane"), &datum(r.b, "Top plane")],
                Alignment::Aligned,
                Alignment::Aligned,
                false,
            )
            .unwrap();
        assert!(r.doc.assembly().component(r.b).unwrap().placement.rotation.angle_between(DQuat::IDENTITY) < 1e-7);
        assert!(r.doc.undo());
        assert_eq!(r.doc.assembly().component(r.b).unwrap().placement, reversed);
        r.doc
            .mate_hinge_limited(
                "H",
                [&cylinder(r.a), &cylinder(r.b)],
                [&datum(r.a, "Top plane"), &datum(r.b, "Top plane")],
                [Alignment::Aligned; 2],
                Some(limits),
                false,
            )
            .unwrap();
        bounded(&mut r.doc, "H");
        let accepted = r.doc.assembly().clone();
        for rotation in [reversed.rotation, DQuat::from_rotation_y(std::f64::consts::PI - 0.1)] {
            r.doc.begin_drag(path.clone()).unwrap();
            r.doc.preview_drag(Placement::new(rotation, DVec3::ZERO)).unwrap();
            r.doc.commit_drag().unwrap();
            bounded(&mut r.doc, "H");
            assert!(r.doc.assembly().component(r.a).unwrap().placement.rotation.angle_between(DQuat::IDENTITY) < 1e-7);
        }
        let at = r.doc.assembly().component(r.b).unwrap().placement;
        r.doc.begin_drag(path).unwrap();
        r.doc.preview_drag(at).unwrap();
        r.doc.commit_drag().unwrap();
        assert_eq!(r.doc.assembly().component(r.b).unwrap().placement, at);
        assert_eq!(r.doc.assembly().mates, accepted.mates);
        std::fs::remove_dir_all(r.dir).unwrap();
    }
}

#[test]
fn limited_hinge_can_move_its_first_axis_when_the_second_is_held() {
    for indirect in [false, true] {
        let mut r = rig(&format!("hinge-fixed-second-{indirect}"), Profile::Circle);
        r.doc.delete_mate("O").unwrap();
        if indirect {
            let reference = r.doc.assembly().sources[0].document.clone();
            let ground = r.doc.insert(reference, Placement::IDENTITY).unwrap();
            r.doc.finish_loading();
            r.doc.mate_origin("Ground", ground, true, false).unwrap();
            r.doc.mate_lock("Held", ground, r.b, false).unwrap();
        } else {
            r.doc.mate_origin("Ground", r.b, true, false).unwrap();
        }
        let held = r.doc.assembly().component(r.b).unwrap().placement;
        let path = soliddraft_assembly::OccurrencePath::root().child(soliddraft_assembly::Step::component(r.a));
        r.doc
            .move_to(&[(path, Placement { rotation: DQuat::from_xyzw(0.0, 1.0, 0.0, 0.0), translation: DVec3::ZERO })])
            .unwrap();
        r.doc
            .mate_hinge_limited(
                "H",
                [&cylinder(r.a), &cylinder(r.b)],
                [&datum(r.a, "Top plane"), &datum(r.b, "Top plane")],
                [Alignment::Aligned; 2],
                Some([-30.0, 30.0]),
                false,
            )
            .unwrap();
        bounded(&mut r.doc, "H");
        let a = r.doc.assembly().component(r.a).unwrap().placement;
        let b = r.doc.assembly().component(r.b).unwrap().placement;
        assert!((a.rotation * DVec3::Z - b.rotation * DVec3::Z).length() < 1e-8);
        assert!(a.translation.distance(b.translation) < 1e-8);
        assert!(b.translation.distance(held.translation) < 1e-8);
        assert!(b.rotation.angle_between(held.rotation) < 1e-7);
        std::fs::remove_dir_all(r.dir).unwrap();
    }
}

#[test]
fn unrestricted_hinge_stops_preserve_non_axial_plane_alignment() {
    let mut r = rig("hinge-nonaxial-antipodal", Profile::Circle);
    let path = soliddraft_assembly::OccurrencePath::root().child(soliddraft_assembly::Step::component(r.b));
    r.doc
        .move_to(&[(path, Placement { rotation: DQuat::from_xyzw(1.0, 0.0, 0.0, 0.0), translation: DVec3::ZERO })])
        .unwrap();
    r.doc
        .mate_hinge(
            "Unlimited",
            [&cylinder(r.a), &cylinder(r.b)],
            [&datum(r.a, "Right plane"), &datum(r.b, "Right plane")],
            Alignment::Aligned,
            Alignment::Aligned,
            false,
        )
        .unwrap();
    assert!(r.doc.undo());
    r.doc
        .mate_hinge_limited(
            "H",
            [&cylinder(r.a), &cylinder(r.b)],
            [&datum(r.a, "Right plane"), &datum(r.b, "Right plane")],
            [Alignment::Aligned; 2],
            Some([-180.0, 180.0]),
            false,
        )
        .unwrap();
    bounded(&mut r.doc, "H");
    assert!(r.doc.assembly().component(r.b).unwrap().placement.rotation.angle_between(DQuat::IDENTITY) < 1e-7);
    std::fs::remove_dir_all(r.dir).unwrap();
}

fn occurrence(c: ComponentId) -> soliddraft_assembly::OccurrencePath {
    soliddraft_assembly::OccurrencePath::root().child(soliddraft_assembly::Step::component(c))
}
fn hinge(doc: &mut AssemblyDocument, name: &str, a: ComponentId, b: ComponentId) {
    doc.mate_hinge(
        name,
        [&cylinder(a), &cylinder(b)],
        [&datum(a, "Top plane"), &datum(b, "Top plane")],
        Alignment::Aligned,
        Alignment::Aligned,
        false,
    )
    .unwrap();
}
fn move_under_mates(doc: &mut AssemblyDocument, component: ComponentId, target: Placement) {
    doc.begin_drag(occurrence(component)).unwrap();
    doc.preview_drag(target).unwrap();
    doc.commit_drag().unwrap();
}
#[test]
fn gear_is_driven_from_either_joint_and_preserves_its_phase_after_reopen() {
    let mut r = rig("gear-motion", Profile::Circle);
    let reference = r.doc.assembly().sources[0].document.clone();
    let c = r.doc.insert(reference, Placement::IDENTITY).unwrap();
    r.doc.finish_loading();
    hinge(&mut r.doc, "H1", r.a, r.b);
    hinge(&mut r.doc, "H2", r.a, c);
    let initial = r.doc.assembly().component(r.b).unwrap().placement.rotation.to_euler(glam::EulerRot::XYZ).2;
    r.doc.mate_gear("G", ["H1", "H2"], [2.0, 1.0], false, false).unwrap();
    move_under_mates(
        &mut r.doc,
        r.b,
        Placement::new(DQuat::from_rotation_z(initial + std::f64::consts::FRAC_PI_2), DVec3::ZERO),
    );
    assert!(
        r.doc
            .assembly()
            .component(c)
            .unwrap()
            .placement
            .rotation
            .angle_between(DQuat::from_rotation_z(-std::f64::consts::FRAC_PI_4))
            < 1e-7
    );
    move_under_mates(&mut r.doc, c, Placement::new(DQuat::from_rotation_z(-std::f64::consts::FRAC_PI_2), DVec3::ZERO));
    assert!(
        r.doc
            .assembly()
            .component(r.b)
            .unwrap()
            .placement
            .rotation
            .angle_between(DQuat::from_rotation_z(initial + std::f64::consts::PI))
            < 1e-7
    );
    bounded(&mut r.doc, "G");
    let before = r.doc.assembly().clone();
    assert!(r.doc.delete_mate("H1").is_err());
    assert_eq!(&before, r.doc.assembly());
    r.doc.save(None).unwrap();
    let mut reopened = AssemblyDocument::open(&r.file, options()).unwrap();
    reopened.finish_loading();
    bounded(&mut reopened, "G");
    assert_eq!(reopened.assembly(), &before);
    std::fs::remove_dir_all(r.dir).unwrap();
}
#[test]
fn screw_tracks_multiple_turns_and_can_be_driven_by_axial_travel() {
    let mut r = rig("screw-motion", Profile::Circle);
    r.doc.mate_concentric("C", &cylinder(r.a), &cylinder(r.b), Alignment::Aligned, false, false).unwrap();
    let initial = r.doc.assembly().component(r.b).unwrap().placement;
    let initial_angle = initial.rotation.to_euler(glam::EulerRot::XYZ).2;
    r.doc.mate_screw("S", "C", 8.0, false, false).unwrap();
    let depth = r.doc.undo_depth();
    r.doc.begin_drag(occurrence(r.b)).unwrap();
    for quarter in 1..=10 {
        let shown = r.doc.table().local(r.doc.table().find(&occurrence(r.b)).unwrap());
        let target = Placement::new(
            DQuat::from_rotation_z(initial_angle + quarter as f64 * std::f64::consts::FRAC_PI_2),
            shown.translation,
        );
        r.doc.preview_drag(target).unwrap();
        let shown = r.doc.table().local(r.doc.table().find(&occurrence(r.b)).unwrap());
        assert!(
            (shown.translation.z - (initial.translation.z + quarter as f64 * 2.0)).abs() < 1e-8,
            "quarter {quarter}: {shown:?}"
        );
    }
    r.doc.commit_drag().unwrap();
    assert_eq!(r.doc.undo_depth(), depth + 1);
    bounded(&mut r.doc, "S");
    r.doc.save(None).unwrap();
    let mut reopened = AssemblyDocument::open(&r.file, options()).unwrap();
    reopened.finish_loading();
    bounded(&mut reopened, "S");
    let before = reopened.assembly().component(r.b).unwrap().placement;
    move_under_mates(&mut reopened, r.b, Placement::new(before.rotation, before.translation + DVec3::Z * 2.0));
    let after = reopened.assembly().component(r.b).unwrap().placement;
    assert!((after.translation.z - before.translation.z - 2.0).abs() < 1e-8);
    assert!(after.rotation.angle_between(before.rotation * DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2)) < 1e-7);
    std::fs::remove_dir_all(r.dir).unwrap();
}
#[test]
fn rack_and_pinion_moves_in_both_directions_and_reverse_changes_sign() {
    let mut r = rig("rack-motion", Profile::Circle);
    let reference = r.doc.assembly().sources[0].document.clone();
    let rack = r.doc.insert(reference, Placement::IDENTITY).unwrap();
    r.doc.finish_loading();
    hinge(&mut r.doc, "H", r.a, r.b);
    r.doc
        .mate_parallel("PX", &datum(r.a, "Right plane"), &datum(rack, "Right plane"), Alignment::Aligned, false)
        .unwrap();
    r.doc.mate_parallel("P", &datum(r.a, "Top plane"), &datum(rack, "Top plane"), Alignment::Aligned, false).unwrap();
    let initial = r.doc.assembly().component(r.b).unwrap().placement.rotation.to_euler(glam::EulerRot::XYZ).2;
    r.doc.mate_rack_and_pinion("R", "H", &datum(rack, "Right plane"), Some(r.a), 40.0, true, false).unwrap();
    move_under_mates(
        &mut r.doc,
        r.b,
        Placement::new(DQuat::from_rotation_z(initial + std::f64::consts::FRAC_PI_2), DVec3::ZERO),
    );
    assert!((r.doc.assembly().component(rack).unwrap().placement.translation.x + 10.0).abs() < 1e-8);
    let old = r.doc.assembly().component(rack).unwrap().placement;
    move_under_mates(&mut r.doc, rack, Placement::new(old.rotation, DVec3::new(-20.0, 0.0, 0.0)));
    assert!(
        r.doc
            .assembly()
            .component(r.b)
            .unwrap()
            .placement
            .rotation
            .angle_between(DQuat::from_rotation_z(initial + std::f64::consts::PI))
            < 1e-7
    );
    bounded(&mut r.doc, "R");
    std::fs::remove_dir_all(r.dir).unwrap();
}
#[test]
fn linear_coupler_can_be_dragged_from_either_end_and_stops_at_a_limit() {
    let mut r = rig("linear-motion", Profile::Block);
    let reference = r.doc.assembly().sources[0].document.clone();
    let c = r.doc.insert(reference, Placement::IDENTITY).unwrap();
    r.doc.finish_loading();
    r.doc.move_to(&[(occurrence(r.b), Placement::translation(DVec3::new(3.0, 4.0, 20.0)))]).unwrap();
    r.doc
        .mate_linear_coupler(
            "L",
            &datum(r.b, "Right plane"),
            &datum(c, "Right plane"),
            [Some(r.a), Some(r.a)],
            [1.0, 2.0],
            false,
            false,
        )
        .unwrap();
    let b = r.doc.assembly().component(r.b).unwrap().placement;
    move_under_mates(&mut r.doc, c, Placement::translation(DVec3::new(10.0, 0.0, 0.0)));
    assert!((r.doc.assembly().component(r.b).unwrap().placement.translation.x - (b.translation.x + 5.0)).abs() < 1e-8);
    r.doc
        .mate_limit_distance(
            "Stop",
            &datum(r.a, "Right plane"),
            &datum(c, "Right plane"),
            ["0 mm", "12 mm"],
            Alignment::Aligned,
            false,
        )
        .unwrap();
    move_under_mates(&mut r.doc, c, Placement::translation(DVec3::new(100.0, 0.0, 0.0)));
    assert!((r.doc.assembly().component(c).unwrap().placement.translation.x - 12.0).abs() < 1e-8);
    let at_stop = r.doc.assembly().component(r.b).unwrap().placement.translation.x;
    let stop = r.doc.assembly().component(c).unwrap().placement.translation.x;
    r.doc.edit_mates(&[("L".into(), MateEdit::Reverse(true))]).unwrap();
    move_under_mates(&mut r.doc, c, Placement::IDENTITY);
    assert!((r.doc.assembly().component(r.b).unwrap().placement.translation.x - at_stop - stop / 2.0).abs() < 1e-8);
    assert!(r.doc.solver().bounded.is_empty());
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn mechanical_settings_reject_invalid_values_and_capture_the_current_phase() {
    let mut r = rig("mechanical-settings", Profile::Circle);
    r.doc.mate_concentric("C", &cylinder(r.a), &cylinder(r.b), Alignment::Aligned, false, false).unwrap();
    let before = r.doc.assembly().clone();
    assert_eq!(r.doc.mate_screw("Bad", "C", 0.0, false, false).unwrap_err().code, RefusalCode::InvalidValue);
    assert_eq!(r.doc.assembly(), &before);
    r.doc.mate_screw("S", "C", 8.0, false, false).unwrap();
    let pose = r.doc.assembly().component(r.b).unwrap().placement;
    r.doc.edit_mates(&[("S".into(), MateEdit::Reverse(true)), ("S".into(), MateEdit::Pitch(16.0))]).unwrap();
    assert_eq!(r.doc.assembly().component(r.b).unwrap().placement, pose);
    move_under_mates(
        &mut r.doc,
        r.b,
        Placement::new(pose.rotation * DQuat::from_rotation_z(std::f64::consts::FRAC_PI_2), pose.translation),
    );
    assert!((r.doc.assembly().component(r.b).unwrap().placement.translation.z - pose.translation.z + 4.0).abs() < 1e-8);
    let before = r.doc.assembly().clone();
    assert_eq!(
        r.doc.edit_mates(&[("S".into(), MateEdit::Pitch(f64::NAN))]).unwrap_err().code,
        RefusalCode::InvalidValue
    );
    assert!(r.doc.edit_mates(&[("C".into(), MateEdit::LockRotation(true))]).is_err());
    assert_eq!(r.doc.assembly(), &before);
    bounded(&mut r.doc, "S");
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn an_exact_plane_group_stays_canonical_after_a_free_spin_drag() {
    let mut r = rig("drag-exact-free-spin", Profile::Block);
    let reference = r.doc.assembly().sources[0].document.clone();
    let c = r.doc.insert(reference, Placement::IDENTITY).unwrap();
    r.doc.finish_loading();
    r.doc
        .mate_distance("D", &datum(r.b, "Top plane"), &datum(c, "Top plane"), "0.2 mm", Alignment::Aligned, false)
        .unwrap();
    for z in [0.1, 0.01, 0.3, 1.234, 0.6789, -0.1, 100.123] {
        move_under_mates(&mut r.doc, r.b, Placement::new(DQuat::from_rotation_z(0.7), DVec3::new(3.0, 4.0, z)));
        assert!(r.doc.solver().bounded.is_empty());
        let before = r.doc.assembly().clone();
        let depth = r.doc.undo_depth();
        r.doc.edit_mates(&[]).unwrap();
        assert_eq!(r.doc.assembly(), &before, "a free-spin drag at {z} must retain the original exact solve");
        assert_eq!(r.doc.undo_depth(), depth);
    }
    std::fs::remove_dir_all(r.dir).unwrap();
}

#[test]
fn nested_screw_winding_belongs_to_the_parent_occurrence() {
    let mut r = rig("nested-screw-motion", Profile::Circle);
    r.doc.mate_concentric("C", &cylinder(r.a), &cylinder(r.b), Alignment::Aligned, false, false).unwrap();
    r.doc.mate_screw("S", "C", 8.0, false, false).unwrap();
    let initial = r.doc.assembly().component(r.b).unwrap().placement;
    let initial_angle = initial.rotation.to_euler(glam::EulerRot::XYZ).2;
    r.doc.save(None).unwrap();
    let child_bytes = std::fs::read(&r.file).unwrap();
    let parent_file = r.dir.join("parent.dwasm");
    let mut parent = AssemblyDocument::new_with("Parent", options());
    parent.save(Some(&parent_file)).unwrap();
    let identity = identify_file(&r.file).unwrap();
    let mut reference = DocRef::new(identity.id, identity.kind, "", identity.revision);
    reference.set_path(&parent_file, &r.file);
    let child = parent.insert(reference.clone(), Placement::IDENTITY).unwrap();
    let other = parent.insert(reference, Placement::new(DQuat::IDENTITY, DVec3::X * 100.0)).unwrap();
    parent.finish_loading();
    let path = occurrence(child).child(soliddraft_assembly::Step::component(r.b));
    let depth = parent.undo_depth();
    parent.begin_drag(path.clone()).unwrap();
    for quarter in 1..=10 {
        let shown = parent.table().local(parent.table().find(&path).unwrap());
        parent
            .preview_drag(Placement::new(
                DQuat::from_rotation_z(initial_angle + quarter as f64 * std::f64::consts::FRAC_PI_2),
                shown.translation,
            ))
            .unwrap();
        let shown = parent.table().local(parent.table().find(&path).unwrap());
        assert!(
            (shown.translation.z - (initial.translation.z + quarter as f64 * 2.0)).abs() < 1e-8,
            "quarter {quarter}: {shown:?}"
        );
    }
    parent.commit_drag().unwrap();
    assert_eq!(parent.undo_depth(), depth + 1);
    let other_path = occurrence(other).child(soliddraft_assembly::Step::component(r.b));
    assert_eq!(parent.table().local(parent.table().find(&other_path).unwrap()), initial);
    let accepted = parent.assembly().clone();
    assert!(parent.undo());
    assert!(parent.redo());
    assert_eq!(parent.assembly(), &accepted);
    parent.save(None).unwrap();
    let mut reopened = AssemblyDocument::open(&parent_file, options()).unwrap();
    reopened.finish_loading();
    assert_eq!(reopened.assembly(), &accepted);
    let before = reopened.table().local(reopened.table().find(&path).unwrap());
    reopened.begin_drag(path.clone()).unwrap();
    reopened
        .preview_drag(Placement::new(
            DQuat::from_rotation_z(initial_angle + 11.0 * std::f64::consts::FRAC_PI_2),
            before.translation,
        ))
        .unwrap();
    reopened.commit_drag().unwrap();
    let after = reopened.table().local(reopened.table().find(&path).unwrap());
    assert!((after.translation.z - initial.translation.z - 22.0).abs() < 1e-8);
    reopened.save(None).unwrap();
    let grandparent_file = r.dir.join("grandparent.dwasm");
    let mut grandparent = AssemblyDocument::new_with("Grandparent", options());
    grandparent.save(Some(&grandparent_file)).unwrap();
    let identity = identify_file(&parent_file).unwrap();
    let mut reference = DocRef::new(identity.id, identity.kind, "", identity.revision);
    reference.set_path(&grandparent_file, &parent_file);
    let parent_id = grandparent.insert(reference, Placement::IDENTITY).unwrap();
    grandparent.finish_loading();
    let nested = occurrence(parent_id)
        .child(soliddraft_assembly::Step::component(child))
        .child(soliddraft_assembly::Step::component(r.b));
    let before = grandparent.table().local(grandparent.table().find(&nested).unwrap());
    grandparent.begin_drag(nested.clone()).unwrap();
    grandparent
        .preview_drag(Placement::new(
            DQuat::from_rotation_z(initial_angle + 12.0 * std::f64::consts::FRAC_PI_2),
            before.translation,
        ))
        .unwrap();
    grandparent.commit_drag().unwrap();
    let after = grandparent.table().local(grandparent.table().find(&nested).unwrap());
    assert!((after.translation.z - initial.translation.z - 24.0).abs() < 1e-8);
    grandparent.save(None).unwrap();
    assert_eq!(std::fs::read(&r.file).unwrap(), child_bytes);
    std::fs::remove_dir_all(r.dir).unwrap();
}

fn wait_motion(doc: &mut AssemblyDocument) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while doc.drag_pending() {
        doc.poll_drag();
        assert!(std::time::Instant::now() < deadline, "collision worker timed out");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    if let Some(report) = doc.drag_report() {
        assert!(report.error.is_none(), "{report:?}");
    }
}

#[test]
fn guarded_drag_stops_before_contact_and_cannot_tunnel_to_a_clear_endpoint() {
    for (name, target_z, clearance) in
        [("collision", 5.0, None), ("tunnel", -30.0, None), ("dynamic", -30.0, Some(2.0))]
    {
        let mut r = rig(&format!("guard-{name}"), Profile::Block);
        let path = occurrence(r.b);
        let before = r.doc.assembly().clone();
        let depth = r.doc.undo_depth();
        r.doc.begin_guarded_drag(path.clone(), clearance).unwrap();
        r.doc.preview_drag(Placement::translation(DVec3::new(3.0, 4.0, target_z))).unwrap();
        assert!(r.doc.drag_pending());
        wait_motion(&mut r.doc);
        assert_eq!(&before, r.doc.assembly());
        assert_eq!(depth, r.doc.undo_depth());
        let row = r.doc.table().find(&path).unwrap();
        let z = r.doc.table().world(row).translation.z;
        let stop = 10.0 + clearance.unwrap_or(0.0);
        assert!(z > stop && z < stop + 0.03, "{name}: stopped at {z}");
        let report = r.doc.drag_report().unwrap();
        assert_eq!(report.tier, "certified_mesh");
        assert!(report.stopped && report.faces.len() == 2, "{report:?}");
        assert!(report.distance.unwrap().lo >= clearance.unwrap_or(0.0));
        r.doc.save(None).unwrap();
        let reopened = AssemblyDocument::open(&r.file, options()).unwrap();
        assert_eq!(&before, reopened.assembly(), "saving must not publish the preview");
        r.doc.commit_drag().unwrap();
        wait_motion(&mut r.doc);
        assert!(!r.doc.dragging());
        assert_eq!(r.doc.undo_depth(), depth + 1);
        assert!(r.doc.undo());
        assert_eq!(&before, r.doc.assembly());
        assert!(r.doc.redo());
        assert_eq!(r.doc.assembly().component(r.b).unwrap().placement.translation.z, z);
        std::fs::remove_dir_all(r.dir).unwrap();
    }
}

#[test]
fn guarded_release_waits_for_latest_wish_and_cancel_discards_inflight_work() {
    let mut r = rig("guard-lifecycle", Profile::Block);
    let path = occurrence(r.b);
    let before = r.doc.assembly().clone();
    let depth = r.doc.undo_depth();
    r.doc.begin_guarded_drag(path.clone(), None).unwrap();
    for z in [25.0, 30.0, 40.0] {
        r.doc.preview_drag(Placement::translation(DVec3::new(3.0, 4.0, z))).unwrap();
    }
    r.doc.commit_drag().unwrap();
    wait_motion(&mut r.doc);
    assert!(!r.doc.dragging());
    assert_eq!(r.doc.assembly().component(r.b).unwrap().placement.translation.z, 40.0);
    assert_eq!(r.doc.undo_depth(), depth + 1);
    r.doc.undo();
    r.doc.begin_guarded_drag(path.clone(), None).unwrap();
    r.doc.preview_drag(Placement::translation(DVec3::new(3.0, 4.0, -30.0))).unwrap();
    r.doc.cancel_drag();
    r.doc.poll_drag();
    assert!(!r.doc.drag_pending());
    assert_eq!(&before, r.doc.assembly());
    r.doc.begin_guarded_drag(path, None).unwrap();
    r.doc.preview_drag(Placement::translation(DVec3::new(3.0, 4.0, 30.0))).unwrap();
    r.doc.rename(r.b, Some("Renamed during check".into())).unwrap();
    r.doc.poll_drag();
    assert!(!r.doc.drag_pending());
    assert_eq!(r.doc.assembly().component(r.b).unwrap().placement, before.component(r.b).unwrap().placement);
    std::fs::remove_dir_all(r.dir).unwrap();
}
