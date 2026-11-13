//! OBJ through the installed CLI, including the export settings saved in a part.

use std::process::Command as Process;
use std::sync::Arc;

use serde_json::Value;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{P2, Sketch, solve::solve};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, ProfileSource, SketchFeature,
    SketchPlane,
};

#[test]
fn obj_from_recipe_and_saved_part_records_the_mesh_and_export_tolerance() {
    let dir = std::env::temp_dir().join(format!("soliddraft-cli-obj-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let recipe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes/box.json");
    let recipe_obj = dir.join("recipe.obj");
    let out = Process::new(env!("CARGO_BIN_EXE_soliddraft"))
        .args(["export", recipe.to_str().unwrap(), "--out", recipe_obj.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["format"], "obj");
    assert_eq!(report["report"]["chord_height"], 0.01);
    assert_eq!(report["report"]["triangles"], 12);
    assert_eq!(report["report"]["closed"], true);
    let unavailable = report["report"]["unavailable_controls"].as_array().unwrap();
    assert!(!unavailable.is_empty());
    let plain = Process::new(env!("CARGO_BIN_EXE_soliddraft"))
        .args(["export", recipe.to_str().unwrap(), "--out", recipe_obj.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(plain.status.success(), "{}", String::from_utf8_lossy(&plain.stderr));
    let summary = String::from_utf8(plain.stdout).unwrap();
    for control in unavailable {
        assert!(summary.contains(control.as_str().unwrap()), "{summary}");
    }
    let text = std::fs::read_to_string(&recipe_obj).unwrap();
    assert_eq!(text.lines().filter(|l| l.starts_with("f ")).count(), 12);
    assert_eq!(text.lines().filter(|l| l.starts_with("g face_")).count(), 6);

    let mut doc = Document::new("Pin with a name", Arc::new(CatallaxKernel::new()));
    let mut sketch = Sketch::new();
    sketch.circle_at(P2::new(0.0, 0.0), 20.0).unwrap();
    solve(&mut sketch, None, true);
    let s = doc
        .execute(Command::AddFeature {
            kind: FeatureKind::Sketch(SketchFeature {
                plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
                sketch,
            }),
            name: None,
            position: None,
            via_assistant: false,
        })
        .unwrap()
        .created
        .unwrap();
    doc.execute(Command::AddFeature {
        kind: FeatureKind::Extrude(ExtrudeFeature {
            profile: ProfileSource::Sketch { feature: s },
            operation: ExtrudeOp::Boss,
            end: EndCondition::Blind { depth: 30.0 },
            reverse: false,
        }),
        name: None,
        position: None,
        via_assistant: false,
    })
    .unwrap();
    let mut settings = doc.settings().clone();
    settings.export.chord_height = 0.005;
    doc.set_settings(settings).unwrap();
    let part = dir.join("pin.dwprt");
    doc.save(&part).unwrap();
    let obj = dir.join("pin.OBJ");
    let out = Process::new(env!("CARGO_BIN_EXE_soliddraft"))
        .args(["export", part.to_str().unwrap(), "--out", obj.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["report"]["chord_height"], 0.005);
    assert_eq!(report["report"]["closed"], true);
    let text = std::fs::read_to_string(&obj).unwrap();
    assert!(text.lines().any(|l| l == "o Pin with a name"));
    assert!(text.lines().any(|l| l == "# chord_height_mm 0.005"));
    assert_eq!(
        text.lines().filter(|l| l.starts_with("f ")).count() as u64,
        report["report"]["triangles"].as_u64().unwrap()
    );
    assert_eq!(
        text.lines().filter(|l| l.starts_with("v ")).count() as u64,
        report["report"]["vertices"].as_u64().unwrap()
    );
    assert_eq!(
        text.lines().filter(|l| l.starts_with("vn ")).count() as u64,
        report["report"]["vertices"].as_u64().unwrap()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_allowance_above_the_application_limit_is_refused_before_the_part_opens() {
    let dir = std::env::temp_dir().join(format!("soliddraft-cli-limit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let part = dir.join("never-opened.dwprt");
    let output = dir.join("part.stl");
    let out = Process::new(env!("CARGO_BIN_EXE_soliddraft"))
        .args(["export", part.to_str().unwrap(), "--out", output.to_str().unwrap(), "--max-work", "20000001"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("20M application limit"), "{error}");
    assert!(!error.contains("never-opened"), "{error}");
    assert!(!output.exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_iges_refusal_leaves_the_whole_batch_untouched() {
    let dir = std::env::temp_dir().join(format!("soliddraft-cli-iges-refusal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let recipe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes/fillet.json");
    let step = dir.join("previous.step");
    let iges = dir.join("previous.iges");
    let sentinel = b"previous export";
    std::fs::write(&step, sentinel).unwrap();
    std::fs::write(&iges, sentinel).unwrap();
    let out = Process::new(env!("CARGO_BIN_EXE_soliddraft"))
        .arg("export")
        .arg(recipe)
        .arg("--out")
        .arg(&step)
        .arg("--out")
        .arg(&iges)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("iges."), "{error}");
    assert_eq!(std::fs::read(step).unwrap(), sentinel);
    assert_eq!(std::fs::read(iges).unwrap(), sentinel);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn step_from_recipe_writes_proof_reports_with_operation_work() {
    let dir =
        std::env::temp_dir().join(format!("soliddraft-cli-step-proof-{}", soliddraft_document::DocumentId::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let recipe = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes/box.json");
    let path = dir.join("box.step");
    let out = Process::new(env!("CARGO_BIN_EXE_soliddraft"))
        .arg("export")
        .arg(recipe)
        .arg("--out")
        .arg(&path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let proof: Value =
        serde_json::from_slice(&std::fs::read(soliddraft_exchange::proof::sidecar_path(&path, "json")).unwrap())
            .unwrap();
    let features = proof["features"].as_array().unwrap();
    assert!(!features.is_empty());
    for feature in features {
        assert_eq!(feature["answer_kind"], "proved");
        assert!(feature["work_used"].as_u64().is_some(), "{feature}");
    }
    assert!(features.iter().any(|feature| feature["work_used"].as_u64().unwrap() > 0));
    assert!(std::fs::read(&path).unwrap().starts_with(b"ISO-10303-21"));
    assert!(
        std::fs::read_to_string(soliddraft_exchange::proof::sidecar_path(&path, "txt")).unwrap().contains("Proved")
    );
    std::fs::remove_dir_all(dir).unwrap();
}
