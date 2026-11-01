//! Exact material points on the concave draft and the skew corner fillet. The application has
//! no command that classifies a point, so this takes the native recipe the adapter sends to
//! Catallax and asks both of its evaluators: the B-rep and the independently compiled field.
//! Every expected side comes from the designers' hand derivations, not from a kernel answer.

use catallax_boolean::{Containment, classify_point, face_contains};
use catallax_features::{Built, evaluate};
use catallax_field::{FaceState, Location, Plan, Workspace};
use catallax_numbers::{Decider, Outcome, Rational, with_operation_budget};
use catallax_recipe::{ElementId, Origin, Recipe, Role, StepId};
use catallax_shapes::RPoint3;
use serde_json::Value;
use soliddraft_kernel_api::*;
use soliddraft_kernel_catallax::CatallaxKernel;

/// One application request's allowance.
const WORK: u64 = 20_000_000;

fn extruded(k: &CatallaxKernel, corners: &[(f64, f64)], depth: f64) -> BodyId {
    let point = |i: usize| Point2::new(corners[i % corners.len()].0, corners[i % corners.len()].1);
    let segments = (0..corners.len()).map(|i| Segment::Line { from: point(i), to: point(i + 1) }).collect();
    let profile = Profile { plane: Plane::XY, outer: Loop(segments), holes: Vec::new() };
    let sheet = k.create_body_from_profile(&OpTag::new("section"), &profile).unwrap().body;
    let body = k.extrude(&OpTag::new("base"), sheet, &Extrude { distance: depth, direction: None }).unwrap().body;
    k.release(sheet).unwrap();
    body
}

/// The native recipe of the operation the adapter ran last, and its extrusion's step id.
fn sent_recipe(k: &CatallaxKernel) -> (Recipe, String) {
    let evidence = k.take_diagnostics().unwrap();
    let questions = evidence["questions"].as_array().expect("the adapter records its questions");
    let question: &Value = questions.last().expect("the adapter asked Catallax");
    let steps = question["problem"]["steps"].as_array().unwrap();
    let extrusion = steps.iter().find(|s| s["op"] == "extrude").expect("the extrusion");
    (Recipe::parse(&question["problem"].to_string()).unwrap(), extrusion["id"].as_str().unwrap().to_owned())
}

fn point(p: [&str; 3]) -> RPoint3 {
    let [x, y, z] = p.map(|v| Rational::parse(v).unwrap());
    RPoint3::new(x, y, z)
}

/// Both evaluators of one recipe.
struct Evaluators {
    built: Built,
    plan: Plan,
    storage: Workspace,
}

impl Evaluators {
    fn of(recipe: &Recipe) -> Self {
        let (built, work) = with_operation_budget(WORK, || evaluate(recipe, &mut Decider::default()));
        assert!(!work.exhausted, "{work:?}");
        let plan = Plan::compile_checked(recipe, &mut Decider::default()).unwrap();
        Self { built: built.unwrap(), plan, storage: Workspace::default() }
    }

    fn expect(&mut self, stage: &str, p: [&str; 3], expected: Location) {
        let at = point(p);
        let (field, work) =
            with_operation_budget(WORK, || self.plan.proven_checked(&at, &mut self.storage, &mut Decider::default()));
        let field = field.unwrap_or_else(|e| panic!("{stage} {p:?}: {e:?} {work:?}"));
        assert_eq!(field.location, expected, "{stage} field {p:?}");
        let built = &mut self.built;
        let (native, work) = with_operation_budget(WORK, || {
            classify_point(&mut built.model, built.result, &at, &mut Decider::default())
        });
        assert!(!work.exhausted, "{stage} {p:?}");
        let matched = match expected {
            Location::Inside => matches!(native, Outcome::Proven(Containment::Inside)),
            Location::Outside => matches!(native, Outcome::Proven(Containment::Outside)),
            _ => matches!(native, Outcome::ProvenCoincident(_)),
        };
        assert!(matched, "{stage} native {p:?}: {native:?}");
    }

    /// Whether `p` is on the face with this origin, in both evaluators.
    fn on_face(&mut self, origin: &Origin, p: [&str; 3], expected: bool) {
        let at = point(p);
        let model = &mut self.built.model;
        let id = model.faces_of_body(self.built.result).into_iter().find(|id| model.face_origins(*id).contains(origin));
        let native = face_contains(model, id.expect("the face"), &at, &mut Decider::default()).unwrap();
        assert_eq!(native, expected, "{origin:?} {p:?}");
        let selected = self.plan.select(origin).unwrap();
        let state = self.plan.face_query(selected, &at, &mut self.storage).unwrap().state;
        assert_eq!(state, if expected { FaceState::OnFace } else { FaceState::NotOnFace }, "{origin:?} {p:?}");
    }
}

/// The bracket drafted by `angle` on the outer right, inner and left walls.
fn drafted_bracket(angle: f64) -> (Recipe, String) {
    let k = CatallaxKernel::with_diagnostics(WORK);
    let corners = [(-20.0, -15.0), (20.0, -15.0), (20.0, -5.0), (0.0, -5.0), (0.0, 15.0), (-20.0, 15.0)];
    let base = extruded(&k, &corners, 20.0);
    let faces = k.topology(base).unwrap().faces;
    let wall = |x: f64, y: f64| {
        let at = Vec3::new(x, y, 10.0);
        faces.iter().find(|f| f.centroid.distance(at) < 1e-9).expect("the wall").id
    };
    let walls = [wall(20.0, -10.0), wall(0.0, 5.0), wall(-20.0, 0.0)];
    let params = Draft { pull: Vec3::Z, neutral_origin: Vec3::ZERO, neutral_normal: Vec3::Z, angle_deg: angle };
    k.draft(&OpTag::new("draft"), base, &walls, &params).unwrap();
    sent_recipe(&k)
}

#[test]
fn the_concave_draft_keeps_both_arms_and_its_edit_moves_the_outer_and_inner_walls() {
    use Location::{Boundary, Inside, Outside};
    let rows: [([&str; 3], Location, Location); 8] = [
        (["-10", "-10", "10"], Inside, Inside),
        (["-10", "5", "10"], Inside, Inside),
        (["-10", "-5", "10"], Inside, Inside),
        (["5", "5", "10"], Outside, Outside),
        (["21", "-10", "10"], Outside, Outside),
        (["19", "-10", "19"], Inside, Outside),
        (["-1", "0", "19"], Inside, Outside),
        (["-10", "-10", "20"], Boundary, Boundary),
    ];
    let (three, extrusion) = drafted_bracket(3.0);
    let mut at_three = Evaluators::of(&three);
    let mut at_five = Evaluators::of(&drafted_bracket(5.0).0);
    for (p, a3, a5) in rows {
        at_three.expect("a3", p, a3);
        at_five.expect("a5", p, a5);
    }

    // A face is its finite trim, not its infinite support. Segment l0s2 is the notch line.
    let origin = |role| Origin { step: StepId::new(&extrusion).unwrap(), role };
    let end = origin(Role::End);
    let notch = origin(Role::Side { segment: ElementId::new("l0s2").unwrap(), path: None });
    at_three.on_face(&end, ["-10", "5", "20"], true);
    at_three.on_face(&end, ["5", "5", "20"], false);
    at_three.on_face(&notch, ["10", "-5", "10"], true);
    at_three.on_face(&notch, ["-10", "-5", "10"], false);
}

/// The triangular prism with all nine edges filleted at `radius`.
fn rounded_prism(radius: f64) -> Recipe {
    let k = CatallaxKernel::with_diagnostics(WORK);
    let base = extruded(&k, &[(0.0, 0.0), (8.0, 0.0), (0.0, 6.0)], 10.0);
    let edges: Vec<_> = k.topology(base).unwrap().edges.iter().map(|e| e.id).collect();
    assert_eq!(edges.len(), 9);
    k.fillet(&OpTag::new("round"), base, &edges, radius).unwrap();
    sent_recipe(&k).0
}

#[test]
fn the_skew_corner_material_follows_the_radius_edit() {
    use Location::{Boundary, Inside, Outside};
    let rows: [([&str; 3], Location, Location); 5] = [
        (["2", "2", "5"], Inside, Inside),
        (["8", "0", "0"], Outside, Outside),
        (["11/2", "1/2", "1/2"], Inside, Inside),
        (["29/5", "2/5", "1/5"], Outside, Inside),
        (["13/3", "1/3", "2/3"], Inside, Inside),
    ];
    let mut r1 = Evaluators::of(&rounded_prism(1.0));
    let mut half = Evaluators::of(&rounded_prism(0.5));
    for (p, at_one, at_half) in rows {
        r1.expect("r1", p, at_one);
        half.expect("r1/2", p, at_half);
    }
    // In the sector of the sphere at (5, 1, 1), where a dot-product octant would say otherwise.
    r1.expect("r1", ["17/3", "1/3", "2/3"], Boundary);
}
