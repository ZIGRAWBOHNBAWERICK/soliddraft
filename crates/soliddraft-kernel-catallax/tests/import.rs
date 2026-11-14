//! A1: the independent V9 contract on the real adapter and exact fixture bytes.

use soliddraft_kernel_api::{
    Approval, Decision, Disposition, ImportOptions, ImportStepRequest, Kernel, OpTag, RepairRecord,
};
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::ImportStepFeature;

const BLOCK: &[u8] = include_bytes!("../../../ci/fixtures/import/block-seam-4um.step");
const WIDE: &[u8] = include_bytes!("../../../ci/fixtures/import/block-seam-20um.step");
const TWO: &[u8] = include_bytes!("../../../ci/fixtures/import/two-blocks-5um.step");

#[test]
fn source_incidence_certificates_round_trip_and_tampering_is_refused() {
    let source = include_bytes!("fixtures/planar-source-incidence.step");
    let feature = ImportStepFeature::new("incidence.step", source.to_vec());
    let kernel = CatallaxKernel::new();
    let options = ImportOptions::default();
    kernel.propose_import(source, &options).unwrap();
    let request = ImportStepRequest {
        source_solid: None,
        name: &feature.source_name,
        source,
        sha256: &feature.sha256,
        options: &options,
        decisions: &[],
        record: None,
    };
    let imported = kernel.import_step(&OpTag::new("incidence"), &request).unwrap();
    assert_eq!(imported.record.format, catallax_import::INCIDENCE_REPAIR_FORMAT);
    assert_eq!(imported.record.source_incidence.len(), 1);
    let saved: RepairRecord = serde_json::from_slice(&serde_json::to_vec(&imported.record).unwrap()).unwrap();
    assert_eq!(saved, imported.record);
    let reopened =
        kernel.import_step(&OpTag::new("reopened"), &ImportStepRequest { record: Some(&saved), ..request }).unwrap();
    assert_eq!(reopened.record, saved);
    let mut forged = saved.clone();
    forged.source_incidence[0].source_to_result_upper_mm = "0".into();
    assert!(
        kernel.import_step(&OpTag::new("forged"), &ImportStepRequest { record: Some(&forged), ..request }).is_err()
    );
}

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
        source_solid: None,
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
                source_solid: None,
                name: &two.source_name,
                source: &two.source,
                sha256: &two.sha256,
                options: &options,
                decisions: &[],
                record: None,
            },
        )
        .unwrap_err();
    assert!(refused.to_string().contains("Choose one source solid for each imported body."), "{refused}");
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
        source_solid: None,
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

fn exported_source(mut step: serde_json::Value) -> Vec<u8> {
    step["id"] = serde_json::json!("source");
    step["pose"] = serde_json::json!({"rotation":["1","0","0","0"],"translation":["0","0","0"]});
    let recipe = catallax_recipe::Recipe::parse(
        &serde_json::json!({"format":catallax_recipe::FORMAT_V5,"name":"Import test","steps":[step],"result":"source"})
            .to_string(),
    )
    .unwrap();
    let app = catallax_app::build_incremental_with_files(
        &recipe,
        None,
        &Default::default(),
        &catallax_mesh::MeshOptions::default(),
        &mut catallax_numbers::Decider::default(),
    )
    .unwrap();
    let built = app.checkpoint.built();
    let mut bytes = Vec::new();
    catallax_step::write_step(
        &built.model,
        &[built.result],
        &Default::default(),
        &mut catallax_numbers::Decider::default(),
        &mut bytes,
    )
    .unwrap();
    bytes
}

fn round_trip_source(name: &str, source: &[u8], volume: f64) -> RepairRecord {
    let kernel = CatallaxKernel::new();
    let feature = ImportStepFeature::new(name, source.to_vec());
    let options = ImportOptions::default();
    kernel.propose_import(source, &options).unwrap();
    let request = ImportStepRequest {
        source_solid: None,
        name,
        source,
        sha256: &feature.sha256,
        options: &options,
        decisions: &[],
        record: None,
    };
    let imported = kernel.import_step(&OpTag::new("wider"), &request).unwrap();
    assert_eq!(imported.bodies.len(), 1);
    let body = imported.bodies[0].result.body;
    assert!((kernel.mass_properties(body).unwrap().volume - volume).abs() < 1e-5, "{name}");
    assert!(!kernel.tessellate(body, &Default::default()).unwrap().indices.is_empty());
    let record = imported.record;
    let reopened =
        kernel.import_step(&OpTag::new("reopened"), &ImportStepRequest { record: Some(&record), ..request }).unwrap();
    assert_eq!(reopened.record, record);
    assert!((kernel.mass_properties(reopened.bodies[0].result.body).unwrap().volume - volume).abs() < 1e-5);
    record
}

#[test]
fn spheres_tori_and_non_unit_cones_import_display_and_replay() {
    use serde_json::json;
    let pi = std::f64::consts::PI;
    for (name, step, volume) in [
        ("sphere", json!({"op":"sphere","radius":"2"}), 32.0 * pi / 3.0),
        ("torus", json!({"op":"torus","major":"5","minor":"1"}), 10.0 * pi * pi),
        ("cone", json!({"op":"cone","base_radius":"4","top_radius":"1","height":"5"}), 35.0 * pi),
        ("widening cone", json!({"op":"cone","base_radius":"1","top_radius":"4","height":"5"}), 35.0 * pi),
    ] {
        round_trip_source(name, &exported_source(step), volume);
    }
}

#[test]
fn binary_noise_cleanup_keeps_its_measured_bounds_on_reopen() {
    let source = exported_source(serde_json::json!({"op":"cylinder","radius":"2","height":"5"}));
    let text = String::from_utf8(source).unwrap();
    let noisy = text
        .replace("(2.,0.,0.)", "(2.,-7.3478807948841188E-16,0.)")
        .replace("(2.,0.,5.)", "(2.,-7.3478807948841188E-16,5.)");
    assert_ne!(text, noisy);
    let record = round_trip_source("noisy.step", noisy.as_bytes(), 20.0 * std::f64::consts::PI);
    assert!(record.derivative.is_some());
    assert_eq!(record.face_tolerances.len(), 3);
    assert!(record.face_tolerances.iter().all(|f| f.deviation_upper_mm.parse::<f64>().unwrap() < 1e-9));
}

#[test]
fn a_certified_spline_patch_imports_as_a_solid_and_replays() {
    let source = exported_source(serde_json::json!({"op":"block","size":["2","3","4"]}));
    let text = String::from_utf8(source).unwrap();
    let file = catallax_import::read_step(text.as_bytes()).unwrap();
    let face = &file.solids[0].faces[0];
    let mut corners: Vec<_> = face.loops[0]
        .edges
        .iter()
        .map(|edge| if edge.forward { edge.start.point.clone() } else { edge.end.point.clone() })
        .collect();
    if !face.same_sense {
        corners.swap(1, 3);
    }
    let mut points = String::new();
    for (id, point) in [0, 3, 1, 2].map(|i| &corners[i]).into_iter().enumerate() {
        let p = point.clone().map(|x| x.to_decimal_bounds(30).0);
        points.push_str(&format!("#{}=CARTESIAN_POINT('',({},{},{}));\n", 900001 + id, p[0], p[1], p[2]));
    }
    let face_line = text.lines().find(|line| line.starts_with(&format!("{}=", face.name))).unwrap();
    let surface_id = face_line.split(',').nth(2).unwrap();
    let line = text.lines().find(|line| line.starts_with(&format!("{surface_id}="))).unwrap();
    let spline = text.replace(line, &format!("{surface_id}=B_SPLINE_SURFACE_WITH_KNOTS('',1,1,((#900001,#900002),(#900003,#900004)),.UNSPECIFIED.,.F.,.F.,.F.,(2,2),(2,2),(0.,1.),(0.,1.),.UNSPECIFIED.);"))
        .replace("DATA;", &format!("DATA;\n{points}"));
    round_trip_source("spline.step", spline.as_bytes(), 24.0);
}
