//! Body targeting, Combine history and compatibility with existing part files.

use soliddraft_kernel_api::BooleanOp;
use soliddraft_kernel_catallax::CatallaxKernel;
use soliddraft_model::{
    BuiltinShape, Command, DatumPlane, Document, EndCondition, ExtrudeFeature, ExtrudeOp, FeatureId, FeatureKind,
    FeatureStatus, ProfileSource,
};
use std::sync::Arc;

fn block(width: f64, depth: f64) -> FeatureKind {
    FeatureKind::Extrude(ExtrudeFeature {
        profile: ProfileSource::Shape {
            datum: DatumPlane::Top,
            shape: BuiltinShape::Rectangle { width, height: 10.0 },
        },
        operation: ExtrudeOp::Boss,
        end: EndCondition::Blind { depth },
        reverse: false,
        options: Default::default(),
    })
}

fn add(doc: &mut Document, body: u32, kind: FeatureKind) -> FeatureId {
    doc.execute(Command::AddFeatureInBody { body, kind, via_assistant: false, position: None })
        .unwrap()
        .created
        .unwrap()
}

fn volume(doc: &Document, key: u32) -> f64 {
    let body = doc.live_bodies().into_iter().find(|(body, _)| *body == key).unwrap().1;
    doc.kernel().mass_properties(body).unwrap().volume
}

fn pair() -> (Document, FeatureId, FeatureId) {
    let mut doc = Document::new("Two bodies", Arc::new(CatallaxKernel::new()));
    let a = add(&mut doc, 0, block(10.0, 10.0));
    let b = add(&mut doc, 1, block(20.0, 5.0));
    (doc, a, b)
}

#[test]
fn body_edits_rollback_and_suppression_leave_other_bodies_unchanged() {
    let (mut doc, a, b) = pair();
    let first = doc.body_after(a).unwrap();
    assert_eq!(doc.live_bodies().len(), 2);
    doc.execute(Command::EditFeature { id: b, kind: block(20.0, 6.0) }).unwrap();
    assert_eq!(doc.body_after(a), Some(first));
    assert!((volume(&doc, 0) - 1000.0).abs() < 1e-6);
    assert!((volume(&doc, 1) - 1200.0).abs() < 1e-6);
    assert!(doc.body_before(b).is_none(), "a new body starts without another body's solid");
    doc.execute(Command::RollTo { before: Some(b) }).unwrap();
    assert_eq!(doc.live_bodies(), vec![(0, first)]);
    doc.undo().unwrap();
    doc.execute(Command::SetSuppressed { id: b, suppressed: true }).unwrap();
    assert_eq!(doc.live_bodies(), vec![(0, first)]);
    doc.undo().unwrap();
    assert_eq!(doc.live_bodies().len(), 2);
}

#[test]
fn every_combine_operation_consumes_only_its_tool_and_survives_history_and_files() {
    for (operation, expected) in
        [(BooleanOp::Union, 1500.0), (BooleanOp::Subtract, 500.0), (BooleanOp::Intersect, 500.0)]
    {
        let (mut doc, _, tool) = pair();
        let other = add(&mut doc, 2, block(3.0, 2.0));
        let untouched = doc.body_after(other).unwrap();
        let before = doc.live_bodies();
        let combine = add(&mut doc, 0, FeatureKind::Combine { operation, tool: 1 });
        assert_eq!(doc.status(combine), Some(&FeatureStatus::Ok));
        let contributing = doc.body_features(0);
        assert!(contributing.contains(&tool));
        assert!(contributing.contains(&combine));
        assert!(!contributing.contains(&other));
        assert_eq!(doc.live_bodies().len(), 2);
        assert!(doc.live_bodies().contains(&(2, untouched)));
        assert!((volume(&doc, 0) - expected).abs() < 1e-6);
        doc.undo().unwrap();
        assert_eq!(doc.live_bodies(), before);
        doc.redo().unwrap();
        assert!((volume(&doc, 0) - expected).abs() < 1e-6);
        assert_eq!(doc.truth()["format"], "soliddraft-part/3");
        assert_eq!(doc.manifest().format, "soliddraft-part/3");
        let hash = doc.geometry_hash();
        let back = Document::from_bytes(&doc.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
        assert_eq!(back.geometry_hash(), hash);
        assert_eq!(back.live_bodies().len(), 2);
        assert!((volume(&back, 0) - expected).abs() < 1e-6);
        // Changing the tool invalidates the Combine build, even though its target is unchanged.
        doc.execute(Command::EditFeature { id: tool, kind: block(20.0, 6.0) }).unwrap();
        let expected = if operation == BooleanOp::Union {
            1600.0
        } else if operation == BooleanOp::Subtract {
            400.0
        } else {
            600.0
        };
        assert!((volume(&doc, 0) - expected).abs() < 1e-6);
    }
}

#[test]
fn old_single_body_part_json_and_archive_keep_their_truth_and_shape() {
    let legacy = serde_json::json!({
        "format":"soliddraft-part/2", "name":"Legacy part", "units":"mm", "next_feature":2,
        "features":[{"id":1,"name":"Boss-Extrude1","kind":{"type":"extrude",
            "profile":{"from":"shape","datum":"top","shape":{"type":"rectangle","width":10.0,"height":10.0}},
            "operation":"boss","end":{"type":"blind","depth":10.0}}}]
    });
    let doc = Document::from_part_json(&legacy.to_string(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(doc.features()[0].body, 0);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&doc.part_json()).unwrap(), legacy);
    assert!((volume(&doc, 0) - 1000.0).abs() < 1e-6);
    let hash = doc.semantic_hash();
    let back = Document::from_bytes(&doc.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(back.semantic_hash(), hash);
    assert_eq!(back.part_json(), doc.part_json());
    assert_eq!(back.features()[0].body, 0);
}

#[test]
fn deleted_and_undone_body_ids_are_not_reused_after_reopening() {
    let (mut doc, _, tool) = pair();
    doc.execute(Command::DeleteFeature { id: tool }).unwrap();
    assert_eq!(doc.next_body(), 2);
    let back = Document::from_bytes(&doc.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(back.next_body(), 2);
    let (mut doc, _, _) = pair();
    doc.undo().unwrap();
    assert_eq!(doc.next_body(), 2);
}

#[test]
fn deferred_deletion_and_combination_keep_the_previous_body_membership_until_rebuild() {
    let (mut doc, _, tool) = pair();
    let before = doc.live_bodies();
    doc.execute_deferred(Command::DeleteFeature { id: tool }).unwrap();
    assert_eq!(doc.live_bodies(), before);
    doc.undo_deferred().unwrap();
    doc.rebuild();
    assert_eq!(doc.live_bodies(), before);
    doc.execute_deferred(Command::AddFeatureInBody {
        body: 0,
        kind: FeatureKind::Combine { operation: BooleanOp::Union, tool: 1 },
        via_assistant: false,
        position: None,
    })
    .unwrap();
    assert_eq!(doc.live_bodies(), before);
    doc.rebuild();
    assert_eq!(doc.live_bodies().len(), 1);
}

#[test]
fn body_materials_and_appearances_survive_rebuild_and_reopen() {
    use soliddraft_document::ConfigScope;
    use soliddraft_materials::{Appearance, Material, Srgb};
    use soliddraft_model::AppearanceTarget;

    let (mut doc, _, second) = pair();
    let unstyled = doc.geometry_hash();
    let steel_colour = Appearance::new("steel", "Steel", Srgb::new(100, 110, 120));
    let brass_colour = Appearance::new("brass", "Brass", Srgb::new(200, 170, 90));
    let mut steel = Material::new("steel", "Steel");
    steel.density_kg_m3 = Some(7850.0);
    steel.default_appearance = Some(steel_colour.id.clone());
    let mut brass = Material::new("brass", "Brass");
    brass.density_kg_m3 = Some(8500.0);
    brass.damping_ratio = Some(0.03);
    brass.default_appearance = Some(brass_colour.id.clone());
    doc.execute(Command::SetMaterial {
        material: Some(steel),
        default_appearance: Some(steel_colour.clone()),
        scope: ConfigScope::All,
    })
    .unwrap();
    doc.execute(Command::SetBodyMaterial {
        body: 1,
        material: Some(brass),
        default_appearance: Some(brass_colour.clone()),
        scope: ConfigScope::All,
    })
    .unwrap();

    let faces = |doc: &Document| {
        let body = doc.body_after(second).unwrap();
        doc.kernel().topology(body).unwrap().faces.into_iter().map(|face| face.id).collect::<Vec<_>>()
    };
    let paint = |target, appearance| Command::SetAppearance { target, appearance, scope: ConfigScope::All };
    let part = Appearance::new("part", "Part paint", Srgb::WHITE);
    let body = Appearance::new("body", "Body paint", Srgb::BLACK);
    let face = Appearance::new("face", "Face paint", Srgb::new(255, 0, 0));
    let ids = faces(&doc);
    let face_target = AppearanceTarget::Face { face: doc.face_ref(ids[0]).unwrap() };
    let body_target = AppearanceTarget::Body { body: 1 };

    assert_eq!(doc.appearance_table(None).of(ids[0]), &brass_colour);
    doc.execute(paint(AppearanceTarget::Part, Some(part.clone()))).unwrap();
    assert_eq!(doc.appearance_table(None).of(ids[0]), &part);
    doc.execute(paint(body_target.clone(), Some(body.clone()))).unwrap();
    doc.execute(paint(face_target.clone(), Some(face.clone()))).unwrap();
    let table = doc.appearance_table(None);
    assert_eq!(table.of(ids[0]), &face);
    assert_eq!(table.of(ids[1]), &body);
    assert_eq!(table.of_body(0), &part);
    assert_eq!(doc.geometry_hash(), unstyled);
    assert!((doc.mass(None).unwrap().mass_kg - 0.01635).abs() < 1e-9);

    let mut back = Document::from_bytes(&doc.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(back.manifest().format, "soliddraft-part/5");
    assert_eq!(back.part_json(), doc.part_json());
    assert_eq!(back.body_material(1, None).unwrap().damping_ratio, Some(0.03));
    let table = back.appearance_table(None);
    assert_eq!(table.of_body(1), &body);
    assert_eq!(faces(&back).iter().filter(|&&id| table.of(id) == &face).count(), 1);

    let previous = doc.part_json().replace("soliddraft-part/5", "soliddraft-part/4");
    let migrated = Document::from_part_json(&previous, Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(migrated.part_json(), doc.part_json());
    assert_eq!(migrated.appearance_table(None).of_body(1), &body);

    back.execute(paint(face_target, None)).unwrap();
    back.execute(paint(body_target, None)).unwrap();
    back.execute(paint(AppearanceTarget::Part, None)).unwrap();
    assert_eq!(back.appearance_table(None).of_body(1), &brass_colour);
    back.execute(Command::EditFeature { id: second, kind: block(20.0, 6.0) }).unwrap();
    assert_eq!(back.appearance_table(None).of_body(0), &steel_colour);
    assert_eq!(back.appearance_table(None).of_body(1), &brass_colour);
    assert!((back.mass(None).unwrap().mass_kg - 0.01805).abs() < 1e-9);
}

#[test]
fn display_states_follow_the_configuration_and_keep_their_view_through_a_file() {
    use soliddraft_document::{
        ConfigScope,
        display_state::{DisplayMode, DisplayScope, DisplayState},
    };
    use soliddraft_materials::{Appearance, Srgb};
    use soliddraft_model::AppearanceTarget;

    let (mut doc, _, second) = pair();
    doc.execute(Command::AddConfiguration { name: "Service".into() }).unwrap();
    let service = doc.named_configurations().next().unwrap().0.clone();
    let mut states = doc.materials().display_states.clone();
    let blue = Appearance::new("blue", "Blue", Srgb::new(21, 101, 192));
    let red = Appearance::new("red", "Red", Srgb::new(198, 40, 40));
    let target = AppearanceTarget::Body { body: 1 };
    doc.execute(Command::SetAppearance {
        target: target.clone(),
        appearance: Some(blue.clone()),
        scope: ConfigScope::All,
    })
    .unwrap();

    let inspection =
        DisplayState::new("Inspection".into(), DisplayScope::Only(vec![None]), DisplayMode::ShadedWithEdges);
    let inspection_id = inspection.id.clone();
    states.put(inspection).unwrap();
    states.select(None, Some(inspection_id)).unwrap();
    let mut working: DisplayState<soliddraft_model::PartDisplay> =
        DisplayState::new("Working".into(), DisplayScope::Only(vec![Some(service.clone())]), DisplayMode::Shaded);
    working.view.paint(target, Some(red.clone()));
    working.view.hidden.insert(0);
    let working_id = working.id.clone();
    states.put(working).unwrap();
    assert!(states.select(None, Some(working_id.clone())).is_err());
    states.select(Some(&service), Some(working_id.clone())).unwrap();

    let shape = doc.geometry_hash();
    doc.execute(Command::SetDisplayStates { states: Box::new(states) }).unwrap();
    assert_eq!(doc.geometry_hash(), shape);
    assert_eq!(doc.appearance_table(None).of_body(1), &blue);
    doc.execute(Command::ActivateConfiguration { name: "Service".into() }).unwrap();
    assert_eq!(doc.appearance_table(Some(&service)).of_body(1), &red);

    let mut back = Document::from_bytes(&doc.to_bytes().unwrap(), Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(back.part_json(), doc.part_json());
    assert_eq!(back.active_configuration(), Some(&service));
    let selected = back.materials().display_states.selected(back.active_configuration()).unwrap();
    assert_eq!(selected.id, working_id);
    assert_eq!(selected.mode, DisplayMode::Shaded);
    assert_eq!(selected.view.hidden.iter().copied().collect::<Vec<_>>(), vec![0]);
    assert_eq!(back.appearance_table(back.active_configuration()).of_body(1), &red);

    back.execute(Command::ActivateConfiguration { name: "Default".into() }).unwrap();
    assert_eq!(back.appearance_table(None).of_body(1), &blue);
    assert!(back.materials().display_states.selected(None).unwrap().view.hidden.is_empty());
    back.execute(Command::EditFeature { id: second, kind: block(20.0, 7.0) }).unwrap();
    back.execute(Command::ActivateConfiguration { name: "Service".into() }).unwrap();
    assert_eq!(back.appearance_table(back.active_configuration()).of_body(1), &red);
}

#[test]
fn configured_formulas_materials_and_display_states_share_one_part_file() {
    use soliddraft_document::{
        ConfigScope,
        display_state::{DisplayMode, DisplayScope, DisplayState},
    };
    use soliddraft_materials::{Appearance, Material, Srgb};
    use soliddraft_model::{AppearanceTarget, values::Input};

    let (mut doc, _, second) = pair();
    doc.execute(Command::SetValue { name: "Stock".into(), expression: "=SUM(2 mm,3 mm)".into() }).unwrap();
    doc.execute(Command::SetValue { name: "TotalMass".into(), expression: "=Mass".into() }).unwrap();
    doc.execute(Command::Drive { input: Input::Depth { feature: second }, expression: "=Stock".into() }).unwrap();
    doc.execute(Command::AddConfiguration { name: "Service".into() }).unwrap();
    doc.execute(Command::SetConfigurationValue {
        configuration: "Service".into(),
        name: "Stock".into(),
        expression: "=ROUND(7.4 mm,0)".into(),
    })
    .unwrap();
    doc.execute(Command::ActivateConfiguration { name: "Service".into() }).unwrap();
    let service = doc.active_configuration().unwrap().clone();

    let mut steel = Material::new("steel", "Steel");
    steel.density_kg_m3 = Some(7850.0);
    doc.execute(Command::SetMaterial { material: Some(steel), default_appearance: None, scope: ConfigScope::All })
        .unwrap();
    let mut brass = Material::new("brass", "Brass");
    brass.density_kg_m3 = Some(8500.0);
    doc.execute(Command::SetBodyMaterial {
        body: 1,
        material: Some(brass),
        default_appearance: None,
        scope: ConfigScope::All,
    })
    .unwrap();
    let blue = Appearance::new("blue", "Blue", Srgb::new(21, 101, 192));
    let red = Appearance::new("red", "Red", Srgb::new(198, 40, 40));
    let target = AppearanceTarget::Body { body: 1 };
    doc.execute(Command::SetAppearance {
        target: target.clone(),
        appearance: Some(blue.clone()),
        scope: ConfigScope::All,
    })
    .unwrap();
    let mut state: DisplayState<soliddraft_model::PartDisplay> =
        DisplayState::new("Working".into(), DisplayScope::Only(vec![Some(service.clone())]), DisplayMode::Shaded);
    state.view.paint(target, Some(red.clone()));
    state.view.hidden.insert(0);
    let state_id = state.id.clone();
    let mut states = doc.materials().display_states.clone();
    states.put(state).unwrap();
    states.select(Some(&service), Some(state_id.clone())).unwrap();
    doc.execute(Command::SetDisplayStates { states: Box::new(states) }).unwrap();

    let bytes = doc.to_bytes().unwrap();
    let mut reopened = Document::from_bytes(&bytes, Arc::new(CatallaxKernel::new())).unwrap();
    assert_eq!(reopened.manifest().format, "soliddraft-part/5");
    assert_eq!(reopened.id(), doc.id());
    assert_eq!(reopened.part_json(), doc.part_json());
    assert_eq!(reopened.semantic_hash(), doc.semantic_hash());
    assert_eq!(reopened.geometry_hash(), doc.geometry_hash());
    assert_eq!(reopened.active_configuration(), Some(&service));
    assert_eq!(reopened.body_material(1, Some(&service)).unwrap().name, "Brass");
    assert_eq!(reopened.appearance_table(Some(&service)).of_body(1), &red);
    assert_eq!(reopened.materials().display_states.selected(Some(&service)).unwrap().id, state_id);
    assert_eq!(reopened.values().driver(&Input::Depth { feature: second }), Some("=Stock"));
    assert!((volume(&reopened, 1) - 1400.0).abs() < 1e-6);
    let table = reopened.values_table();
    let mass = table.globals.iter().find(|row| row.name == "TotalMass").unwrap().values[table.active].unwrap();
    assert!((mass - 0.01975).abs() < 1e-9);
    assert_eq!(mass, reopened.mass(Some(&service)).unwrap().mass_kg);

    reopened.execute(Command::ActivateConfiguration { name: "Default".into() }).unwrap();
    assert!((volume(&reopened, 1) - 1000.0).abs() < 1e-6);
    assert_eq!(reopened.appearance_table(None).of_body(1), &blue);
}
