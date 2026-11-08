//! Hole Alignment reads internal cylinders at pinned revisions and never moves parts.

use glam::{DQuat, DVec3};
use soliddraft_assembly::{
    AssemblyDocument, CheckCode, CheckScope, HoleAlignmentRequest, HoleOutcome, KernelSource, OpenOptions, Placement,
};
use soliddraft_document::{CacheOnlyStore, DocRef, identify_file};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, ProfileSource, SketchFeature,
    SketchPlane,
};
use std::sync::Arc;

#[test]
fn aligned_offset_far_and_unsupported_holes_on_pinned_parts() {
    let dir = std::env::temp_dir().join(format!("soliddraft-hole-alignment-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let kernel = Arc::new(CatallaxKernel::new());
    let path = dir.join("plate.dwprt");
    let mut part = Document::new("Plate", kernel.clone());
    for operation in [ExtrudeOp::Boss, ExtrudeOp::Cut] {
        let mut sketch = Sketch::new();
        if operation == ExtrudeOp::Boss {
            sketch.rectangle(P2::new(0.0, 0.0), 40.0, 30.0).unwrap();
        } else {
            sketch.circle_at(P2::new(2.0, 3.0), 4.0).unwrap();
        }
        solve(&mut sketch, None, true);
        let kind = FeatureKind::Sketch(SketchFeature {
            plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
            sketch,
        });
        let s = part
            .execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false })
            .unwrap()
            .created
            .unwrap();
        let kind = FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: s },
            operation,
            end: EndCondition::Blind { depth: 5.0 },
            reverse: false,
        });
        part.execute(Command::AddFeature { kind, name: None, position: None, via_assistant: false }).unwrap();
    }
    part.save(&path).unwrap();
    let identity = identify_file(&path).unwrap();
    let mut reference = DocRef::new(identity.id, identity.kind, "", identity.revision);
    let root = dir.join("rig.dwasm");
    reference.set_path(&root, &path);
    let options = OpenOptions {
        display: Some(Arc::new(KernelSource::new(kernel))),
        revisions: Some(Arc::new(CacheOnlyStore::new())),
        ..OpenOptions::default()
    };
    let mut doc = AssemblyDocument::new_with("Rig", options);
    doc.save(Some(&root)).unwrap();
    let a = doc.insert(reference.clone(), Placement::IDENTITY).unwrap();
    let b = doc.insert(reference.clone(), Placement::translation(DVec3::new(0.25, 0.0, 10.0))).unwrap();
    let c = doc.insert(reference.clone(), Placement::translation(DVec3::new(0.0, 0.0, 20.0))).unwrap();
    doc.finish_loading();
    let before = doc.assembly().clone();
    let revision = doc.revision();
    let request = HoleAlignmentRequest { max_deviation_mm: 0.25, ..Default::default() };
    let report = doc.check_hole_alignment(&request).unwrap();
    assert!(report.complete, "{report:#?}");
    assert_eq!((report.parts, report.holes.len(), report.aligned, report.misaligned), (3, 3, 1, 2));
    assert_eq!(report.results[0].outcome, HoleOutcome::Misaligned);
    assert_eq!(report.results[0].deviation_squared, "1/16");
    assert_eq!(report.results[0].deviation_mm, 0.25);
    assert!(report.holes.iter().all(|h| h.revision == reference.pinned));
    assert_eq!(doc.assembly(), &before);
    assert_eq!(doc.revision(), revision);
    let b_path = b.to_string().parse().unwrap();
    doc.set_hidden(&[b_path], true).unwrap();
    assert_eq!(doc.check_hole_alignment(&request).unwrap().parts, 2);
    let include_hidden = HoleAlignmentRequest { include_hidden: true, ..request.clone() };
    assert_eq!(doc.check_hole_alignment(&include_hidden).unwrap().parts, 3);
    let b_path = b.to_string().parse().unwrap();
    doc.set_suppressed(&[b_path], true).unwrap();
    assert_eq!(doc.check_hole_alignment(&include_hidden).unwrap().parts, 2);
    let b_path = b.to_string().parse().unwrap();
    doc.set_suppressed(&[b_path], false).unwrap();
    let b_path = b.to_string().parse().unwrap();
    doc.set_hidden(&[b_path], false).unwrap();
    let smaller = HoleAlignmentRequest { max_deviation_mm: f64::from_bits(0.25_f64.to_bits() - 1), ..request.clone() };
    let report = doc.check_hole_alignment(&smaller).unwrap();
    assert_eq!((report.aligned, report.misaligned, report.beyond), (1, 0, 2));
    let selected = HoleAlignmentRequest {
        scope: CheckScope::Occurrences(vec![a.to_string().parse().unwrap(), b.to_string().parse().unwrap()]),
        ..request.clone()
    };
    assert_eq!(doc.check_hole_alignment(&selected).unwrap().misaligned, 1);
    let one = HoleAlignmentRequest {
        scope: CheckScope::Occurrences(vec![c.to_string().parse().unwrap()]),
        ..request.clone()
    };
    assert_eq!(doc.check_hole_alignment(&one).unwrap_err().code, CheckCode::TooFewParts);
    for invalid in [f64::NAN, f64::INFINITY, -1.0] {
        assert_eq!(
            doc.check_hole_alignment(&HoleAlignmentRequest { max_deviation_mm: invalid, ..request.clone() })
                .unwrap_err()
                .code,
            CheckCode::Invalid
        );
    }
    doc.insert(reference.clone(), Placement::new(DQuat::from_rotation_x(std::f64::consts::FRAC_PI_2), DVec3::ZERO))
        .unwrap();
    let tilted = Placement::new(DQuat::from_rotation_x(0.1), DVec3::ZERO);
    doc.insert(reference, tilted).unwrap();
    let report = doc.check_hole_alignment(&request).unwrap();
    assert!(!report.complete);
    assert_eq!(report.nonparallel, 3);
    assert_eq!(report.skipped.len(), 1);
    assert!(report.skipped[0].message.contains("placed axis"));
    std::fs::remove_dir_all(dir).unwrap();
}
