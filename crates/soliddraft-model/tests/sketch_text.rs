//! Font profiles retain counters and regenerate real Catallax solids.
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::sketch::{EntityKind, Sketch, SketchText, TextFont};
use soliddraft_model::{
    Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureKind, FeatureStatus, ProfileSource,
    SketchFeature, SketchPlane,
};
use std::sync::Arc;

#[test]
fn text_counters_extrude_and_edits_rebuild_after_reopen() {
    for font in [TextFont::InterRegular, TextFont::InterMedium] {
        let mut sketch = Sketch::new();
        let text_id = sketch
            .add(EntityKind::Text { text: SketchText { content: "o".into(), font, ..Default::default() } }, false);
        let regions = sketch.regions().unwrap();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions.iter().map(|region| region.holes.len()).sum::<usize>(), 1);
        let area: f64 = regions.iter().map(|region| region.area()).sum();
        let mut doc = Document::new("Sketch text", Arc::new(CatallaxKernel::new()));
        let sketch_id = doc
            .execute(Command::AddFeature {
                kind: FeatureKind::Sketch(SketchFeature {
                    plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
                    sketch: sketch.clone(),
                }),
                name: None,
                position: None,
                via_assistant: false,
            })
            .unwrap()
            .created
            .unwrap();
        let extrude_id = doc
            .execute(Command::AddFeature {
                kind: FeatureKind::Extrude(ExtrudeFeature {
                    profile: ProfileSource::Sketch { feature: sketch_id },
                    operation: ExtrudeOp::Boss,
                    end: EndCondition::Blind { depth: 2.0 },
                    reverse: false,
                }),
                name: None,
                position: None,
                via_assistant: false,
            })
            .unwrap()
            .created
            .unwrap();
        assert_eq!(doc.status(extrude_id), Some(&FeatureStatus::Ok));
        let actual = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
        assert!((actual - area * 2.0).abs() < 1e-5, "{font:?}: {actual} != {}", area * 2.0);
        let EntityKind::Text { text } = &mut sketch.entity_mut(text_id).unwrap().kind else { unreachable!() };
        text.content = "B".into();
        text.height = 15.0;
        text.position.u = 30.0;
        let area = sketch.regions().unwrap()[0].area();
        doc.execute(Command::EditFeature {
            id: sketch_id,
            kind: FeatureKind::Sketch(SketchFeature {
                plane: SketchPlane::Datum { datum: DatumPlane::Top, offset: 0.0 },
                sketch,
            }),
        })
        .unwrap();
        assert_eq!(doc.status(extrude_id), Some(&FeatureStatus::Ok));
        let actual = doc.kernel().mass_properties(doc.body().unwrap()).unwrap().volume;
        assert!((actual - area * 2.0).abs() < 1e-5);
        let reopened = Document::from_part_json(&doc.part_json(), Arc::new(CatallaxKernel::new())).unwrap();
        assert_eq!(reopened.status(extrude_id), Some(&FeatureStatus::Ok));
        let actual = reopened.kernel().mass_properties(reopened.body().unwrap()).unwrap().volume;
        assert!((actual - area * 2.0).abs() < 1e-5);
    }
}

#[test]
fn disconnected_text_boss_reports_the_release_limit_before_kernel_calls() {
    let kernel = Arc::new(CatallaxKernel::with_operation_log());
    let mut doc = Document::new("Disconnected text", kernel.clone());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Text { text: SketchText { content: "oae".into(), ..Default::default() } }, false);
    let notice = sketch.text_extrusion_notice(sketch.regions().unwrap().len());
    let sketch_id = doc
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
    let boss = doc
        .execute(Command::AddFeature {
            kind: FeatureKind::Extrude(ExtrudeFeature {
                profile: ProfileSource::Sketch { feature: sketch_id },
                operation: ExtrudeOp::Boss,
                end: EndCondition::Blind { depth: 2.0 },
                reverse: false,
            }),
            name: None,
            position: None,
            via_assistant: false,
        })
        .unwrap()
        .created
        .unwrap();
    if let Some(notice) = notice {
        assert_eq!(doc.status(boss), Some(&FeatureStatus::Failed(notice.into())));
        assert!(kernel.take_diagnostics().unwrap()["calls"].as_array().unwrap().is_empty());
        assert!(doc.body().is_none());
    } else {
        assert_eq!(doc.status(boss), Some(&FeatureStatus::Ok));
    }
}

/// Direct kernel calls deliberately bypass the release guard for the kernel lane.
#[test]
#[ignore = "manual reproduction: requires disjoint polynomial union in Catallax"]
fn oae_disconnected_kernel_reproduction() {
    use soliddraft_kernel_api::{BooleanOp, Extrude, Kernel, OpTag, Plane, Vec3};
    let kernel = CatallaxKernel::with_operation_log();
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Text { text: SketchText { content: "oae".into(), ..Default::default() } }, false);
    let profiles: Vec<_> = sketch.regions().unwrap().iter().map(|region| region.to_profile(Plane::XY)).collect();
    let output = std::env::var_os("SOLIDDRAFT_TEXT_RECIPE_DIR").map(std::path::PathBuf::from);
    let write = |name: &str, value: serde_json::Value| {
        if let Some(path) = &output {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join(name), serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        }
    };
    write("profiles.json", serde_json::to_value(&profiles).unwrap());
    let mut body = None;
    for (i, profile) in profiles.iter().enumerate() {
        let tag = |step: &str| OpTag::new(format!("2.r{i}.{step}"));
        let sheet = kernel.create_body_from_profile(&tag("profile"), profile).unwrap();
        let tool = kernel.extrude(
            &tag("sweep"),
            sheet.body,
            &Extrude { distance: 2.0, direction: Some(Vec3::new(0.0, 0.0, 1.0)) },
        );
        kernel.release(sheet.body).unwrap();
        let tool = tool.unwrap();
        write(&format!("body-{i}.json"), kernel.body_recipe(tool.body).unwrap());
        let next = match body {
            None => tool.body,
            Some(previous) => {
                let result = kernel.boolean(&tag("union"), BooleanOp::Union, previous, tool.body);
                kernel.release(tool.body).unwrap();
                write("diagnostics.json", kernel.take_diagnostics().unwrap());
                if let Err(error) = &result {
                    write("error.json", serde_json::json!({"error":error.to_string()}));
                }
                result.unwrap().body
            }
        };
        if let Some(previous) = body {
            kernel.release(previous).unwrap();
        }
        body = Some(next);
    }
}
