//! Shell conversion, analytic tray geometry, export and unsupported-domain coverage.
//! The rendered application click path is tested separately in shell_tool.rs.

#[path = "../src/shell.rs"]
mod shell;

use catallax_recipe::{FaceRef, Operation, StepId};
use serde_json::json;
use soliddraft_kernel_api::{BodyId, FaceId, KernelError};

fn top() -> FaceRef {
    serde_json::from_value(json!({"face":{"step":"blank","role":"end"}})).unwrap()
}

#[test]
fn tray_edit_preserves_open_face_origin_and_exact_wall_parameter() {
    let selected = FaceId::from_raw(17);
    for thickness in [2.0, 1.5] {
        let mut resolved = Vec::new();
        let op = shell::operation(StepId::new("blank").unwrap(), &[selected], thickness, |face| {
            resolved.push(face);
            Ok(top())
        })
        .unwrap();
        assert_eq!(resolved, vec![selected]);
        let saved = serde_json::to_value(&op).unwrap();
        let restored: Operation = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), saved);
        let Operation::Shell { body, open, thickness: wall } = op else { panic!("expected shell") };
        assert_eq!(body, StepId::new("blank").unwrap());
        assert_eq!(serde_json::to_value(open).unwrap(), json!([top()]));
        assert_eq!(wall, catallax_recipe::Number::parse(&thickness.to_string()).unwrap());
    }
}

#[test]
fn invalid_wall_is_rejected_before_face_resolution() {
    for wall in [0.0, -0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let result = shell::operation(StepId::new("blank").unwrap(), &[FaceId::from_raw(1)], wall, |_| {
            panic!("invalid thickness must not resolve faces")
        });
        assert!(matches!(result, Err(KernelError::InvalidInput(_))));
    }
}

#[test]
fn foreign_or_lost_face_refuses_the_whole_operation() {
    let body = BodyId::from_raw(1);
    let face = FaceId::from_raw(99);
    let result = shell::operation(StepId::new("blank").unwrap(), &[face], 2.0, |face| {
        Err(KernelError::UnknownFace { body, face })
    });
    assert!(matches!(result, Err(KernelError::UnknownFace { body: b, face: f }) if b == body && f == face));
}

#[test]
fn all_faces_request_is_one_unfiltered_reference_and_stays_apart_from_closed() {
    let body = StepId::new("blank").unwrap();
    let Operation::Shell { open, .. } = shell::all_faces(body.clone(), 2.0).unwrap() else { panic!("expected shell") };
    assert_eq!(serde_json::to_value(open).unwrap(), json!([{}]));
    let Operation::Shell { open, .. } = shell::operation(body.clone(), &[], 2.0, |_| panic!("no faces")).unwrap()
    else {
        panic!("expected shell")
    };
    assert_eq!(serde_json::to_value(open).unwrap(), json!([]));
    assert!(matches!(shell::all_faces(body, 0.0), Err(KernelError::InvalidInput(_))));
}

#[test]
fn no_open_faces_preserves_closed_shell_request() {
    let op =
        shell::operation(StepId::new("blank").unwrap(), &[], 0.1, |_| panic!("closed shell has no selected faces"))
            .unwrap();
    let Operation::Shell { open, thickness, .. } = op else { panic!("expected shell") };
    assert!(open.is_empty());
    assert_eq!(thickness, catallax_recipe::Number::parse("0.1").unwrap());
}

use soliddraft_kernel_api::{Extrude, FaceRole, Kernel, OpTag, Plane, Point2, Profile, StepOptions, Vec3};
use soliddraft_kernel_catallax::CatallaxKernel;

fn extruded(k: &CatallaxKernel, profile: Profile, height: f64) -> soliddraft_kernel_api::BodyId {
    let sheet = k.create_body_from_profile(&OpTag::new("blank"), &profile).unwrap().body;
    let body = k.extrude(&OpTag::new("blank"), sheet, &Extrude { distance: height, direction: None }).unwrap().body;
    k.release(sheet).unwrap();
    body
}

#[test]
fn analytic_tray_has_inward_walls_edits_and_exports_without_changing_blank() {
    let k = CatallaxKernel::new();
    let blank = extruded(&k, Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 40.0, 30.0), 12.0);
    let topology = k.topology(blank).unwrap();
    let top = topology.faces.iter().find(|face| face.normal == Some(Vec3::Z)).unwrap().id;
    let history = k.history(blank).unwrap();
    for thickness in [2.0, 1.5] {
        let tray = k.shell(&OpTag::new("tray"), blank, &[top], thickness).unwrap();
        assert!(k.validate(tray.body).unwrap().valid);
        let expected = 40.0 * 30.0 * 12.0 - (40.0 - 2.0 * thickness) * (30.0 - 2.0 * thickness) * (12.0 - thickness);
        assert!((k.mass_properties(tray.body).unwrap().volume - expected).abs() < 1e-6);
        let walls: Vec<_> =
            tray.record.face_roles.iter().filter(|o| matches!(o.role, FaceRole::Offset { .. })).collect();
        assert_eq!(walls.len(), 5, "four inner walls and an inner floor");
        let result = k.topology(tray.body).unwrap();
        for wall in walls {
            let FaceRole::Offset { face } = wall.role else { unreachable!() };
            let source = topology.face(face).unwrap();
            let inner = result.face(wall.face).unwrap();
            assert_eq!(inner.normal, source.normal.map(|n| -n), "inner wall must face the cavity");
        }
        let step = k.export_step(tray.body, &StepOptions::default()).unwrap();
        assert!(step.text.starts_with("ISO-10303-21;"));
        assert!(step.report.within_tolerance);
        k.release(tray.body).unwrap();
    }
    assert_eq!(k.history(blank).unwrap(), history);
    assert!((k.mass_properties(blank).unwrap().volume - 14400.0).abs() < 1e-6);
}

#[test]
fn unsupported_nonconvex_shell_refuses_without_mutating_input() {
    use soliddraft_kernel_api::{Loop, Segment};
    let k = CatallaxKernel::new();
    let vertices = [[0.0, 0.0], [30.0, 0.0], [30.0, 10.0], [10.0, 10.0], [10.0, 30.0], [0.0, 30.0]];
    let outer = Loop(
        (0..vertices.len())
            .map(|i| {
                let a = vertices[i];
                let b = vertices[(i + 1) % vertices.len()];
                Segment::Line { from: Point2::new(a[0], a[1]), to: Point2::new(b[0], b[1]) }
            })
            .collect(),
    );
    let blank = extruded(&k, Profile { plane: Plane::XY, outer, holes: vec![] }, 12.0);
    let history = k.history(blank).unwrap();
    let before = k.mass_properties(blank).unwrap();
    let error = k.shell(&OpTag::new("unsupported"), blank, &[], 2.0).unwrap_err();
    assert!(
        matches!(&error, KernelError::Failed { op: soliddraft_kernel_api::OpKind::Shell, message, .. } if message.starts_with("distance.unsupported-envelope:")),
        "{error:?}"
    );
    assert_eq!(k.history(blank).unwrap(), history);
    assert_eq!(k.mass_properties(blank).unwrap(), before);
    assert!(k.validate(blank).unwrap().valid);
}

#[test]
fn bored_tray_edits_preserve_inner_walls_and_export() {
    let k = CatallaxKernel::new();
    let blank = extruded(&k, Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 40.0, 30.0), 12.0);
    let bore = soliddraft_kernel_api::contract::cylinder(&k, Vec3::ZERO, 3.0, 12.0);
    let bored = k.boolean(&OpTag::new("bore"), soliddraft_kernel_api::BooleanOp::Subtract, blank, bore).unwrap().body;
    let topology = k.topology(bored).unwrap();
    let top = topology.faces.iter().find(|face| face.normal == Some(Vec3::Z)).unwrap().id;
    let before = k.mass_properties(bored).unwrap();
    let mut names = None;
    for thickness in [2.0, 1.5] {
        let tray = k.shell(&OpTag::new("tray"), bored, &[top], thickness).unwrap();
        assert!(k.validate(tray.body).unwrap().valid);
        let expected = 14400.0
            - std::f64::consts::PI * 9.0 * 12.0
            - ((40.0 - 2.0 * thickness) * (30.0 - 2.0 * thickness) - std::f64::consts::PI * (3.0 + thickness).powi(2))
                * (12.0 - thickness);
        assert!((k.mass_properties(tray.body).unwrap().volume - expected).abs() < 1e-6);
        let inner: Vec<_> = tray
            .record
            .face_roles
            .iter()
            .filter_map(|face| if let FaceRole::Offset { face } = face.role { Some(face) } else { None })
            .collect();
        assert_eq!(inner.len(), 6, "floor, four walls and expanded bore wall");
        if let Some(before) = &names {
            assert_eq!(&inner, before);
        }
        names = Some(inner);
        let step = k.export_step(tray.body, &StepOptions::default()).unwrap();
        assert!(step.report.within_tolerance);
        assert_eq!(step.report.solids, 1);
        k.release(tray.body).unwrap();
    }
    assert_eq!(k.mass_properties(bored).unwrap(), before);
}

fn last_question(k: &CatallaxKernel) -> serde_json::Value {
    k.take_diagnostics().unwrap()["questions"].as_array().unwrap().last().unwrap()["problem"].clone()
}

#[test]
fn all_faces_after_a_boolean_and_a_finish_saves_version_five_and_leaves_inputs_alone() {
    let k = CatallaxKernel::with_operation_log();
    let blank = extruded(&k, Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 40.0, 30.0), 12.0);
    let bore = soliddraft_kernel_api::contract::cylinder(&k, Vec3::ZERO, 3.0, 12.0);
    let bored = k.boolean(&OpTag::new("bore"), soliddraft_kernel_api::BooleanOp::Subtract, blank, bore).unwrap().body;
    let topology = k.topology(bored).unwrap();
    let side = topology.edges.iter().find(|e| e.curve == soliddraft_kernel_api::CurveKind::Line).unwrap().id;
    let rounded = k.fillet(&OpTag::new("round"), bored, &[side], 1.0).unwrap().body;
    for body in [bored, rounded] {
        let before = k.mass_properties(body).unwrap();
        k.shell_all_faces(body, 2.0).unwrap();
        let recipe = last_question(&k);
        assert_eq!(recipe["format"], "catallax-recipe/5");
        let shell = recipe["steps"].as_array().unwrap().last().unwrap().clone();
        assert_eq!(shell["open"], json!([{}]), "not a list of surviving origins");
        catallax_recipe::Recipe::parse(&recipe.to_string()).expect("the saved request reads back");
        assert_eq!(k.mass_properties(body).unwrap(), before);
    }
}

#[test]
fn saved_older_versions_stay_as_they_were_and_only_version_five_opens_all_faces() {
    let recipe = |format: &str, open: serde_json::Value| {
        json!({"format":format, "name":"tray", "result":"tray", "steps":[
            {"id":"blank", "op":"block", "size":["40","30","12"], "pose":{
                "rotation":["1","0","0","0"], "translation":["0","0","0"]}},
            {"id":"tray", "op":"shell", "body":"blank", "open":open, "thickness":"2"}
        ]})
        .to_string()
    };
    let named = json!([{"face":{"step":"blank","role":"end"}}]);
    for format in ["catallax-recipe/3", "catallax-recipe/4", "catallax-recipe/5"] {
        let read = catallax_recipe::Recipe::parse(&recipe(format, named.clone())).unwrap();
        assert_eq!(read.format, format, "parsing never upgrades a saved version");
        catallax_recipe::Recipe::parse(&recipe(format, json!([]))).unwrap();
    }
    catallax_recipe::Recipe::parse(&recipe("catallax-recipe/5", json!([{}]))).unwrap();
    for format in ["catallax-recipe/3", "catallax-recipe/4"] {
        assert!(catallax_recipe::Recipe::parse(&recipe(format, json!([{}]))).is_err(), "{format}");
    }
}

#[test]
fn opening_all_faces_by_name_is_a_plain_refusal() {
    let k = CatallaxKernel::new();
    let blank = extruded(&k, Profile::rectangle(Plane::XY, Point2::new(0.0, 0.0), 40.0, 30.0), 12.0);
    let faces: Vec<_> = k.topology(blank).unwrap().faces.iter().map(|f| f.id).collect();
    let refusal = k.shell(&OpTag::new("tray"), blank, &faces, 2.0).unwrap_err().to_string();
    assert!(refusal.contains("leave nothing of the part"), "{refusal}");
    for internal in ["Catallax", "BodyKind", "kernel"] {
        assert!(!refusal.contains(internal), "{refusal}");
    }
}
