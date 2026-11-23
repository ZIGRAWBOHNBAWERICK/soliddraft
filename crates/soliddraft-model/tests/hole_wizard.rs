//! End-to-end Hole Wizard model tests on Catallax:
//! - ISO M6 counterbore 4-hole pattern driven by an editable position sketch (with
//!   dimensions and sketch linear pattern), edit positions later, change standard to
//!   ANSI Inch 1/4", save (soliddraft-part/7), reopen, rebuild, and add a later feature on top.
//! - Through-all tapped hole (ISO M6 & ANSI Inch 1/4-20 UNC) with cosmetic thread.
//! - Slot and Counterbore Slot geometry on Catallax.
//! - End conditions: Blind, Through All, Up To Next, Up To Vertex, Up To Surface,
//!   Offset From Surface, Flip direction, Head clearance, Near-side countersink.
//! - Non-planar face refusal by name.
//! - Coming-soon refusals by name (Tapered tap, Countersink slot, Under-head CSK, Far-side CSK).
//! - Backward-compatible old-file reader for soliddraft-part/2 legacy holes.

use std::sync::Arc;

use soliddraft_kernel_api::{FaceId, HoleDepth, HoleTip, SurfaceKind, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::file::{FORMAT, HOLE_WIZARD_FORMAT};
use soliddraft_model::hole_wizard::{
    CalloutParam, CalloutTolerance, CountersinkOption, HoleEndCondition, HoleFit, HoleOptions, HoleStandard, HoleType,
    HoleWizardSpec, ToleranceMode,
};
use soliddraft_model::sketch::{
    ConstraintKind, EntityId, EntityKind, P2, PointRef, Sketch, SolveStatus,
    solve::solve,
    tools::{Operation, PatternDirection, PatternSpec},
};
use soliddraft_model::{
    BlendFeature, Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureId, FeatureKind,
    FeatureStatus, HoleFeature, ProfileSource, SketchFeature, SketchPlane,
};

fn doc() -> Document {
    Document::new("HoleWizardPart", Arc::new(CatallaxKernel::new()))
}

fn add(doc: &mut Document, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
        .unwrap()
        .created
        .unwrap()
}

fn plate(doc: &mut Document, width: f64, height: f64, thickness: f64) -> FeatureId {
    let mut s = Sketch::new();
    s.rectangle(P2::ZERO, width, height).unwrap();
    solve(&mut s, None, true);
    assert_eq!(s.status, SolveStatus::FullyDefined);
    let sketch = add(
        doc,
        FeatureKind::Sketch(SketchFeature {
            plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
            sketch: s,
        }),
    );
    add(
        doc,
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: sketch },
            operation: ExtrudeOp::Boss,
            end: EndCondition::Blind { depth: thickness },
            reverse: false,
        }),
    )
}

fn face_facing(doc: &Document, normal: Vec3, z_approx: f64) -> FaceId {
    let topology = doc.kernel().topology(doc.body().unwrap()).unwrap();
    topology
        .faces
        .iter()
        .find(|f| f.normal.is_some_and(|n| n.dot(normal) > 1.0 - 1e-6) && (f.centroid.z - z_approx).abs() < 1e-4)
        .expect("expected planar face")
        .id
}

fn volume(doc: &Document) -> f64 {
    doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume
}

fn all_ok(doc: &Document) {
    for (id, status) in doc.statuses() {
        assert!(status.is_ok(), "{id}: {status:?}");
    }
}

fn hole_from_spec(
    face: soliddraft_model::FaceRef,
    sketch: Option<FeatureId>,
    points: Vec<P2>,
    spec: HoleWizardSpec,
) -> HoleFeature {
    let resolved = spec.resolve_defaults();
    let depth = spec.legacy_depth();
    HoleFeature {
        face,
        points,
        diameter: resolved.diameter,
        depth,
        tip: resolved.tip,
        head: resolved.head,
        thread: resolved.thread,
        sketch,
        wizard: Some(spec),
    }
}

#[test]
fn iso_m6_four_hole_counterbore_edit_positions_switch_to_ansi_inch_save_reopen_and_later_feature() {
    let mut d = doc();
    plate(&mut d, 120.0, 80.0, 15.0);
    let plate_vol = volume(&d);
    assert!((plate_vol - 120.0 * 80.0 * 15.0).abs() < 1e-3);

    let top = d.face_ref(face_facing(&d, Vec3::Z, 15.0)).unwrap();

    // Build a 2x2 position sketch using a fixed seed point + retained linear pattern.
    let mut pos_sketch = Sketch::new();
    let seed: EntityId = pos_sketch.add(EntityKind::Point { p: P2::new(-30.0, -20.0) }, false);
    pos_sketch.constrain(ConstraintKind::Fix { point: PointRef::new(seed, 0), at: P2::new(-30.0, -20.0) });
    pos_sketch
        .apply_operation(&Operation::Pattern {
            entities: vec![seed],
            spec: PatternSpec::Linear {
                step: P2::new(60.0, 0.0),
                count: 2,
                second: Some(PatternDirection { step: P2::new(0.0, 40.0), count: 2 }),
            },
            skipped: vec![],
        })
        .unwrap();
    solve(&mut pos_sketch, None, true);
    assert_eq!(
        pos_sketch
            .entities
            .iter()
            .filter(|e| e.id != soliddraft_model::sketch::ORIGIN && matches!(e.kind, EntityKind::Point { .. }))
            .count(),
        4,
        "2x2 linear pattern yields 4 hole points"
    );

    let sketch_id = add(
        &mut d,
        FeatureKind::Sketch(SketchFeature { plane: SketchPlane::Face { face: top.clone() }, sketch: pos_sketch }),
    );

    // ISO M6 Socket Head Cap Screw counterbore, Through All.
    let mut spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::Counterbore);
    spec.size = "M6".into();
    spec.fit = HoleFit::Normal;
    spec.end_condition = HoleEndCondition::ThroughAll;
    spec.set_tolerance(CalloutTolerance {
        param: CalloutParam::CounterboreDepth,
        mode: ToleranceMode::Bilateral { upper: 0.1, lower: -0.05 },
        precision: 2,
    });
    let hole_id = add(&mut d, FeatureKind::Hole(hole_from_spec(top.clone(), Some(sketch_id), vec![], spec.clone())));
    all_ok(&d);

    // Check exact removed volume for 4x ISO M6 SHCS counterbore (Ø6.6 thru 15mm, Ø11.0 x 6.4mm cbore).
    let expected_removed_iso = 4.0
        * (std::f64::consts::PI * (3.3_f64).powi(2) * (15.0 - 6.4) + std::f64::consts::PI * (5.5_f64).powi(2) * 6.4);
    let vol_iso = volume(&d);
    assert!(
        ((plate_vol - vol_iso) - expected_removed_iso).abs() < 1e-2,
        "ISO M6 4x cbore removed {}, expected {expected_removed_iso}",
        plate_vol - vol_iso
    );

    // Edit positions later by editing the position sketch: change pattern to 3x2 (6 holes)!
    let FeatureKind::Sketch(mut edited_sf) = d.feature(sketch_id).unwrap().kind.clone() else { panic!() };
    let pat_id = edited_sf.sketch.patterns[0].id;
    edited_sf
        .sketch
        .apply_operation(&Operation::EditPattern {
            group: pat_id,
            spec: PatternSpec::Linear {
                step: P2::new(30.0, 0.0),
                count: 3,
                second: Some(PatternDirection { step: P2::new(0.0, 40.0), count: 2 }),
            },
            skipped: vec![],
        })
        .unwrap();
    solve(&mut edited_sf.sketch, None, true);
    d.execute(Command::EditFeature { id: sketch_id, kind: FeatureKind::Sketch(edited_sf) }).unwrap();
    all_ok(&d);
    let vol_6holes = volume(&d);
    assert!(
        ((plate_vol - vol_6holes) - 1.5 * expected_removed_iso).abs() < 1e-2,
        "editing position sketch to 6 holes updates geometry immediately"
    );

    // Change standard to ANSI Inch 1/4" SHCS Counterbore!
    spec.set_standard(HoleStandard::AnsiInch);
    assert_eq!(spec.size, "1/4");
    let inch_hole = hole_from_spec(top.clone(), Some(sketch_id), vec![], spec);
    let inch_diam = inch_hole.diameter;
    let Some(soliddraft_kernel_api::HoleHead::Counterbore { diameter: inch_cb_d, depth: inch_cb_h }) = inch_hole.head
    else {
        panic!("expected counterbore head");
    };
    d.execute(Command::EditFeature { id: hole_id, kind: FeatureKind::Hole(inch_hole) }).unwrap();
    all_ok(&d);

    let expected_removed_inch = 6.0
        * (std::f64::consts::PI * (inch_diam / 2.0).powi(2) * (15.0 - inch_cb_h)
            + std::f64::consts::PI * (inch_cb_d / 2.0).powi(2) * inch_cb_h);
    let vol_inch = volume(&d);
    assert!(
        ((plate_vol - vol_inch) - expected_removed_inch).abs() < 1e-2,
        "ANSI Inch 1/4 6x cbore removed {}, expected {expected_removed_inch}",
        plate_vol - vol_inch
    );

    // Save: must bump format to soliddraft-part/7.
    assert_eq!(d.truth()["format"], HOLE_WIZARD_FORMAT);
    assert_eq!(d.manifest().format, HOLE_WIZARD_FORMAT);
    let saved_bytes = d.to_bytes().unwrap();
    let saved_hash = d.geometry_hash();

    // Reopen in a fresh kernel and rebuild.
    let mut reopened = Document::from_bytes(&saved_bytes, Arc::new(CatallaxKernel::new())).unwrap();
    all_ok(&reopened);
    assert_eq!(reopened.geometry_hash(), saved_hash);
    assert!((volume(&reopened) - vol_inch).abs() < 1e-4);

    // Add a later feature on top (fillet an outer vertical edge of the plate).
    let topo = reopened.kernel().topology(reopened.body().unwrap()).unwrap();
    let vertical_edge = topo
        .edges
        .iter()
        .find(|e| (e.bbox.max.z - e.bbox.min.z - 15.0).abs() < 1e-4 && (e.bbox.max.x - e.bbox.min.x).abs() < 1e-4)
        .expect("outer vertical edge")
        .id;
    let edge_ref = reopened.edge_ref(vertical_edge).unwrap();
    let fillet = add(
        &mut reopened,
        FeatureKind::Fillet(BlendFeature { edges: vec![edge_ref], size: 2.0, other_distance: None }),
    );
    assert_eq!(reopened.status(fillet), Some(&FeatureStatus::Ok));
    assert!(volume(&reopened) < vol_inch);
}

#[test]
fn through_all_tapped_holes_with_cosmetic_thread_iso_and_ansi_inch() {
    let mut d = doc();
    plate(&mut d, 100.0, 60.0, 12.0);
    let top = d.face_ref(face_facing(&d, Vec3::Z, 12.0)).unwrap();

    // 1. ISO M6 Through-All Straight Tap with cosmetic thread.
    let mut s1 = Sketch::new();
    s1.add(EntityKind::Point { p: P2::new(-25.0, 0.0) }, false);
    s1.add(EntityKind::Point { p: P2::new(-5.0, 0.0) }, false);
    let sk1 =
        add(&mut d, FeatureKind::Sketch(SketchFeature { plane: SketchPlane::Face { face: top.clone() }, sketch: s1 }));

    let mut iso_tap = HoleWizardSpec::new(HoleStandard::Iso, HoleType::StraightTap);
    iso_tap.size = "M6".into();
    iso_tap.end_condition = HoleEndCondition::ThroughAll;
    iso_tap.options.cosmetic_thread = true;
    add(&mut d, FeatureKind::Hole(hole_from_spec(top.clone(), Some(sk1), vec![], iso_tap)));

    // 2. ANSI Inch 1/4-20 UNC Through-All Straight Tap with cosmetic thread.
    let mut s2 = Sketch::new();
    s2.add(EntityKind::Point { p: P2::new(25.0, 0.0) }, false);
    let sk2 =
        add(&mut d, FeatureKind::Sketch(SketchFeature { plane: SketchPlane::Face { face: top.clone() }, sketch: s2 }));

    let mut inch_tap = HoleWizardSpec::new(HoleStandard::AnsiInch, HoleType::StraightTap);
    inch_tap.size = "1/4-20".into();
    inch_tap.end_condition = HoleEndCondition::ThroughAll;
    inch_tap.options.cosmetic_thread = true;
    add(&mut d, FeatureKind::Hole(hole_from_spec(top, Some(sk2), vec![], inch_tap)));

    all_ok(&d);

    let walls = d.threaded_walls();
    assert_eq!(walls.len(), 3, "2 ISO M6 + 1 ANSI Inch 1/4-20 walls");
    let callouts: Vec<String> = d.pmi_rows().into_iter().filter(|r| r.feature.is_some()).map(|r| r.callout()).collect();
    assert_eq!(callouts, vec!["2\u{d7} M6x1-6H THRU", "1/4-20 UNC-2B THRU"]);

    // Round-trip save & reopen preserves both cosmetic threads.
    let reopened = Document::from_bytes(&d.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    all_ok(&reopened);
    assert_eq!(reopened.threaded_walls().len(), 3);
    let reopened_callouts: Vec<String> =
        reopened.pmi_rows().into_iter().filter(|r| r.feature.is_some()).map(|r| r.callout()).collect();
    assert_eq!(reopened_callouts, callouts);
}

#[test]
fn slots_and_counterbore_slots_build_and_round_trip() {
    let mut d = doc();
    plate(&mut d, 100.0, 60.0, 12.0);
    let before = volume(&d);
    let top = d.face_ref(face_facing(&d, Vec3::Z, 12.0)).unwrap();

    // Simple through slot: M6 normal (width 6.6), length 12.0.
    let mut slot_spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::Slot);
    slot_spec.size = "M6".into();
    slot_spec.slot_length = Some(12.0);
    slot_spec.end_condition = HoleEndCondition::ThroughAll;
    add(&mut d, FeatureKind::Hole(hole_from_spec(top.clone(), None, vec![P2::new(-25.0, 0.0)], slot_spec)));
    all_ok(&d);

    let slot_area = 12.0 * 6.6 + std::f64::consts::PI * (3.3_f64).powi(2);
    let after_slot = volume(&d);
    assert!(((before - after_slot) - slot_area * 12.0).abs() < 1e-2);

    // Counterbore slot: M6 SHCS (width 6.6, cbore 11.0 x 6.4, length 12.0) + 1.0 mm head clearance.
    let mut cb_slot_spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::CounterboreSlot);
    cb_slot_spec.size = "M6".into();
    cb_slot_spec.slot_length = Some(12.0);
    cb_slot_spec.end_condition = HoleEndCondition::ThroughAll;
    cb_slot_spec.options.head_clearance = Some(1.0);
    add(&mut d, FeatureKind::Hole(hole_from_spec(top, None, vec![P2::new(25.0, 0.0)], cb_slot_spec)));
    all_ok(&d);

    let cb_area = 12.0 * 11.0 + std::f64::consts::PI * (5.5_f64).powi(2);
    let expected_cb_slot = cb_area * 7.4 + slot_area * (12.0 - 7.4);
    assert!(((after_slot - volume(&d)) - expected_cb_slot).abs() < 1e-2);

    let reopened = Document::from_bytes(&d.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    all_ok(&reopened);
    assert!((volume(&reopened) - volume(&d)).abs() < 1e-4);
}

#[test]
fn end_conditions_up_to_surface_offset_up_to_vertex_up_to_next_and_near_side_csk() {
    let mut d = doc();
    plate(&mut d, 120.0, 80.0, 20.0);
    let top = d.face_ref(face_facing(&d, Vec3::Z, 20.0)).unwrap();
    let bottom = d.face_ref(face_facing(&d, -Vec3::Z, 0.0)).unwrap();

    // 1. Offset from surface: 5 mm short of bottom => 15 mm deep blind hole.
    let mut offset_spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::Simple);
    offset_spec.size = "M6".into();
    offset_spec.end_condition =
        HoleEndCondition::OffsetFromSurface { face: bottom.clone(), offset: 5.0, reverse_offset: false };
    offset_spec.options.near_side_csk = Some(CountersinkOption { diameter: 8.6, angle_deg: 90.0 });
    let mut h_off = hole_from_spec(top.clone(), None, vec![P2::new(-35.0, 0.0)], offset_spec);
    h_off.tip = HoleTip::Flat;
    add(&mut d, FeatureKind::Hole(h_off));

    // 2. Up to surface (bottom face, 20 mm deep).
    let mut surf_spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::Simple);
    surf_spec.size = "M6".into();
    surf_spec.end_condition = HoleEndCondition::UpToSurface { face: bottom };
    let mut h_surf = hole_from_spec(top.clone(), None, vec![P2::new(0.0, 0.0)], surf_spec);
    h_surf.tip = HoleTip::Flat;
    add(&mut d, FeatureKind::Hole(h_surf));

    // 3. Up to next (simple flat hole).
    let mut next_spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::Simple);
    next_spec.size = "M6".into();
    next_spec.end_condition = HoleEndCondition::UpToNext;
    let mut h_next = hole_from_spec(top.clone(), None, vec![P2::new(35.0, 0.0)], next_spec);
    h_next.tip = HoleTip::Flat;
    add(&mut d, FeatureKind::Hole(h_next));

    // 4. Up to vertex (a bottom corner vertex at z = 0 => 20 mm deep).
    let topo = d.kernel().topology(d.body().unwrap()).unwrap();
    let v_id = topo.vertices.iter().find(|v| v.position.z.abs() < 1e-6).unwrap().id;
    let v_ref = d.vertex_ref(v_id).unwrap();
    let mut vtx_spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::Simple);
    vtx_spec.size = "M6".into();
    vtx_spec.end_condition = HoleEndCondition::UpToVertex { vertex: v_ref };
    let mut h_vtx = hole_from_spec(top, None, vec![P2::new(0.0, 20.0)], vtx_spec);
    h_vtx.tip = HoleTip::Flat;
    add(&mut d, FeatureKind::Hole(h_vtx));

    all_ok(&d);
}

#[test]
fn non_planar_face_and_coming_soon_options_refuse_by_name() {
    let mut d = doc();
    // Build a cylinder so we have a curved cylindrical face.
    let mut s = Sketch::new();
    s.circle_at(P2::ZERO, 40.0).unwrap();
    solve(&mut s, None, true);
    let sk = add(
        &mut d,
        FeatureKind::Sketch(SketchFeature {
            plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
            sketch: s,
        }),
    );
    add(
        &mut d,
        FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: sk },
            operation: ExtrudeOp::Boss,
            end: EndCondition::Blind { depth: 20.0 },
            reverse: false,
        }),
    );
    let topo = d.kernel().topology(d.body().unwrap()).unwrap();
    let cyl_face = topo.faces.iter().find(|f| f.surface == SurfaceKind::Cylinder).unwrap().id;
    let cyl_ref = d.face_ref(cyl_face).unwrap();
    let top_ref = d.face_ref(face_facing(&d, Vec3::Z, 20.0)).unwrap();

    // Non-planar face refuses by name.
    let spec = HoleWizardSpec::new(HoleStandard::Iso, HoleType::Simple);
    let bad_face = add(&mut d, FeatureKind::Hole(hole_from_spec(cyl_ref, None, vec![P2::ZERO], spec)));
    match d.status(bad_face) {
        Some(FeatureStatus::Failed(msg)) => {
            assert!(msg.contains("planar"), "expected planar refusal, got: {msg}");
        }
        other => panic!("expected Failed on non-planar face, got {other:?}"),
    }

    // Coming soon refusals by name.
    let cases = [
        (HoleWizardSpec::new(HoleStandard::AnsiInch, HoleType::TaperedTap), "Tapered (pipe) tap"),
        (HoleWizardSpec::new(HoleStandard::Iso, HoleType::CountersinkSlot), "Countersink slot"),
        (
            HoleWizardSpec {
                options: HoleOptions {
                    under_head_csk: Some(CountersinkOption { diameter: 8.0, angle_deg: 90.0 }),
                    ..Default::default()
                },
                ..HoleWizardSpec::new(HoleStandard::Iso, HoleType::Counterbore)
            },
            "Under head countersink",
        ),
        (
            HoleWizardSpec {
                options: HoleOptions {
                    far_side_csk: Some(CountersinkOption { diameter: 8.0, angle_deg: 90.0 }),
                    ..Default::default()
                },
                ..HoleWizardSpec::new(HoleStandard::Iso, HoleType::Simple)
            },
            "Far side countersink",
        ),
    ];
    for (spec, expected_name) in cases {
        let id = add(&mut d, FeatureKind::Hole(hole_from_spec(top_ref.clone(), None, vec![P2::ZERO], spec)));
        match d.status(id) {
            Some(FeatureStatus::Failed(msg)) => {
                assert!(msg.contains(expected_name) && msg.contains("coming soon"), "{msg}");
            }
            other => panic!("expected coming-soon failure for {expected_name}, got {other:?}"),
        }
    }
}

#[test]
fn old_v2_hole_file_opens_and_rebuilds_identically() {
    let mut d = doc();
    plate(&mut d, 60.0, 40.0, 10.0);
    let top = d.face_ref(face_facing(&d, Vec3::Z, 10.0)).unwrap();
    // Legacy hole without wizard or sketch writes soliddraft-part/2!
    add(
        &mut d,
        FeatureKind::Hole(HoleFeature {
            face: top,
            points: vec![P2::new(0.0, 0.0)],
            diameter: 6.6,
            depth: HoleDepth::Through,
            tip: HoleTip::Flat,
            head: Some(soliddraft_kernel_api::HoleHead::Counterbore { diameter: 11.0, depth: 6.4 }),
            thread: None,
            sketch: None,
            wizard: None,
        }),
    );
    all_ok(&d);
    assert_eq!(d.truth()["format"], FORMAT, "legacy hole keeps soliddraft-part/2");
    let v2_json = d.part_json();
    let parsed: serde_json::Value = serde_json::from_str(&v2_json).unwrap();
    let hole_json = parsed["features"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"]["type"] == "hole")
        .expect("hole feature in json");
    assert!(hole_json["kind"].get("wizard").is_none());
    assert!(hole_json["kind"].get("sketch").is_none());
    let reopened = Document::from_part_json(&v2_json, Arc::new(CatallaxKernel::new())).unwrap();
    all_ok(&reopened);
    assert_eq!(reopened.part_json(), v2_json);
    assert!((volume(&reopened) - volume(&d)).abs() < 1e-4);
}
