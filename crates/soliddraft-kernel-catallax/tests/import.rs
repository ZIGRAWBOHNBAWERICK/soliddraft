//! A1: the independent V9 contract on the real adapter and exact fixture bytes.

use soliddraft_kernel_api::{Approval, Decision, Disposition, ImportOptions, ImportStepRequest, Kernel, OpTag};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::ImportStepFeature;

const BLOCK: &[u8] = include_bytes!("../../../ci/fixtures/import/block-seam-4um.step");
const WIDE: &[u8] = include_bytes!("../../../ci/fixtures/import/block-seam-20um.step");
const TWO: &[u8] = include_bytes!("../../../ci/fixtures/import/two-blocks-5um.step");

#[test]
fn reviewed_import_round_trips_the_kernel_question_and_builds_one_exact_body() {
    let kernel = CatallaxKernel::new();
    let options = ImportOptions::default();
    let preview = kernel.propose_import(BLOCK, &options).unwrap();
    assert_eq!(preview.source_sha256, "1a602addaa4b534ef9427427d5231ba689a811a11159a5f9e7ebc17b3365c32b");
    assert_eq!(preview.proposals.len(), 1);
    let proposal = &preview.proposals[0];
    assert_eq!(proposal.id, "g0");
    assert_eq!(proposal.question, "repair:1a602addaa4b:g0");
    assert_eq!(proposal.kind, "seam_close");
    assert_eq!(proposal.distance_min_mm, "0.004");
    assert_eq!(proposal.distance_max_mm, "0.004");
    assert_eq!(proposal.tolerance_mm, "0.01");
    assert_eq!(proposal.disposition, Disposition::Auto);
    assert!(proposal.entities.iter().any(|e| e == "#176"));
    let feature = ImportStepFeature::new("block-seam-4um.step", BLOCK.to_vec()).declined(&proposal.question);
    let decisions = feature.kernel_decisions().unwrap();
    assert_eq!(decisions, [Decision::Decline { id: "g0".into() }]);
    let request = ImportStepRequest {
        name: &feature.source_name,
        source: &feature.source,
        sha256: &feature.sha256,
        options: &options,
        decisions: &decisions,
        record: None,
    };
    let declined = kernel.import_step(&OpTag::new("declined"), &request).unwrap();
    assert_eq!(declined.left_open, ["#178"]);
    assert!(declined.bodies.is_empty());
    let accepted =
        kernel.import_step(&OpTag::new("imported"), &ImportStepRequest { decisions: &[], ..request }).unwrap();
    assert!(accepted.left_open.is_empty());
    assert_eq!(accepted.bodies.len(), 1);
    let body = accepted.bodies[0].result.body;
    let topology = kernel.topology(body).unwrap();
    assert_eq!(topology.faces.len(), 6);
    assert_eq!(topology.edges.len(), 12);
    assert!((kernel.mass_properties(body).unwrap().volume - 10004.0).abs() <= 1e-6);
    let mut saved = accepted.record.clone();
    kernel
        .import_step(&OpTag::new("reopened"), &ImportStepRequest { decisions: &[], record: Some(&saved), ..request })
        .unwrap();
    assert_eq!(saved.format, soliddraft_kernel_api::REPAIR_FORMAT);

    // A part saved before face tolerances: no such field, and the older format.
    let mut older = serde_json::to_value(&saved).unwrap();
    older["format"] = soliddraft_kernel_api::LEGACY_REPAIR_FORMAT.into();
    older.as_object_mut().unwrap().remove("face_tolerances");
    let mut older: soliddraft_kernel_api::RepairRecord = serde_json::from_value(older).unwrap();
    let reopened = kernel
        .import_step(&OpTag::new("older"), &ImportStepRequest { decisions: &[], record: Some(&older), ..request })
        .unwrap();
    assert_eq!(reopened.record, saved, "the import is redone and certified, not relabelled");
    older.source.sha256 = "0".repeat(64);
    let stale = kernel
        .import_step(&OpTag::new("stale"), &ImportStepRequest { decisions: &[], record: Some(&older), ..request })
        .unwrap_err();
    assert!(stale.to_string().contains("does not match"), "{stale}");

    saved.source.sha256 = "0".repeat(64);
    let mismatch = kernel
        .import_step(&OpTag::new("tampered"), &ImportStepRequest { decisions: &[], record: Some(&saved), ..request })
        .unwrap_err();
    assert!(mismatch.to_string().contains("does not match"), "{mismatch}");
    kernel.release(body).unwrap();
    let two = ImportStepFeature::new("two-blocks-5um.step", TWO.to_vec());
    let refused = kernel
        .import_step(
            &OpTag::new("two"),
            &ImportStepRequest {
                name: &two.source_name,
                source: &two.source,
                sha256: &two.sha256,
                options: &options,
                decisions: &[],
                record: None,
            },
        )
        .unwrap_err();
    assert!(refused.to_string().contains("2 solids"), "{refused}");
    assert!(refused.to_string().contains("a part holds one body"), "{refused}");
}

#[test]
fn an_approved_seam_saves_its_limit_and_approver_and_verifies_on_reopen() {
    let kernel = CatallaxKernel::new();
    let options = ImportOptions::default();
    let proposals = kernel.propose_import(WIDE, &options).unwrap().proposals;
    let [proposal] = proposals.as_slice() else { panic!("one seam") };
    assert_eq!(proposal.disposition, Disposition::Ask);
    assert_eq!(proposal.tolerance_mm, "0.02");

    let unanswered = ImportStepFeature::new("wide.step", WIDE.to_vec());
    let feature = unanswered.accepted(&proposal.question, "0.03", "Ana: lead reviewer");
    let decisions = feature.kernel_decisions().unwrap();
    let request = ImportStepRequest {
        name: &feature.source_name,
        source: &feature.source,
        sha256: &feature.sha256,
        options: &options,
        decisions: &decisions,
        record: None,
    };
    let imported = kernel.import_step(&OpTag::new("approved"), &request).unwrap();
    let body = imported.bodies[0].result.body;
    assert!((kernel.mass_properties(body).unwrap().volume - 10020.0).abs() <= 1e-6);
    let record = imported.record;
    assert_eq!(record.format, soliddraft_kernel_api::REPAIR_FORMAT);
    assert_eq!(record.changes[0].approval, Approval::User);
    assert_eq!(record.changes[0].approver.as_deref(), Some("Ana: lead reviewer"));
    assert_eq!(record.changes[0].tolerance_mm, "0.03");
    assert_eq!(record.changes[0].max_movement_mm, "0.02");
    assert_eq!(record.face_tolerances.len(), 4);
    for face in &record.face_tolerances {
        assert_eq!(face.deviation_upper_mm, "0.02", "attained, apart from the approved limit");
        assert_eq!(face.approved_tolerance_mm, "0.03");
    }

    let reopened =
        kernel.import_step(&OpTag::new("reopened"), &ImportStepRequest { record: Some(&record), ..request }).unwrap();
    assert_eq!(reopened.record, record);
    let mut forged = record.clone();
    forged.face_tolerances[0].deviation_upper_mm = "0".into();
    let refused =
        kernel.import_step(&OpTag::new("forged"), &ImportStepRequest { record: Some(&forged), ..request }).unwrap_err();
    assert!(refused.to_string().contains("without an unverified repair"), "{refused}");

    let below = unanswered.accepted(&proposal.question, "0.01", "Ana").kernel_decisions().unwrap();
    let refused =
        kernel.import_step(&OpTag::new("below"), &ImportStepRequest { decisions: &below, ..request }).unwrap_err();
    assert!(refused.to_string().contains("at least the measured gap"), "{refused}");
}
