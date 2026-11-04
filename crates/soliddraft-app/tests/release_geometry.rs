//! Release geometry through the same commands, saved parts and display used by the app.

use serde_json::{Value, json};
use soliddraft_app::{app_tools::AppTools, document::FeatureStatus, state::AppState};
use soliddraft_assistant::{Caller, tools::call_checked};
use soliddraft_exchange::{Format, Report};
use soliddraft_kernel_api::{CurveKind, EdgeInfo, SurfaceKind};

fn fresh() -> AppState {
    static CONFIG: std::sync::Once = std::sync::Once::new();
    CONFIG.call_once(|| {
        let dir = std::env::temp_dir().join(format!("soliddraft-release-config-{}", std::process::id()));
        soliddraft_app::file::set_config_dir(Some(dir));
    });
    AppState::new(soliddraft_app::new_document())
}

fn accepted(state: &mut AppState, tool: &str, arguments: Value) {
    let out =
        call_checked(&mut AppTools { state }, tool, arguments, &Caller::Mcp { client: "release-geometry".into() });
    assert!(!out.is_error, "{tool}: {}", out.content);
    assert!(state.doc.features.iter().all(|f| f.status == FeatureStatus::Ok), "{tool}: failed feature");
}

fn sketch(state: &mut AppState, plane: &str, shape: Value) {
    accepted(state, "sketch", json!({"plane":plane,"shapes":[shape]}));
}

fn block() -> AppState {
    let mut state = fresh();
    sketch(&mut state, "top", json!({"shape":"rectangle","center":[0,0,0],"width":40,"height":30}));
    accepted(&mut state, "extrude", json!({"sketch":"Sketch1","depth":12,"flip":false}));
    state
}

fn edges(state: &AppState, select: impl Fn(&EdgeInfo) -> bool) -> Vec<String> {
    let doc = &state.doc;
    doc.kernel()
        .topology(doc.display_body().unwrap())
        .unwrap()
        .edges
        .iter()
        .filter(|e| select(e))
        .map(|edge| {
            let named = doc.named_edge(edge.id).expect("the edge has a persistent name");
            format!("{} · {}", doc.feature(named.feature).unwrap().name, named.label)
        })
        .collect()
}

fn round(state: &mut AppState, selected: Vec<String>, radius: f64) {
    assert!(!selected.is_empty());
    eprintln!("round {selected:?} at {radius}");
    accepted(state, "fillet", json!({"edges":selected,"radius":radius}));
}

fn refused(state: &mut AppState, selected: Vec<String>, radius: f64, expected: &str) {
    assert!(!selected.is_empty(), "{expected}: no edge selected");
    let before = state.doc.model().part_json();
    let body = state.doc.display_body();
    let out = call_checked(
        &mut AppTools { state },
        "fillet",
        json!({"edges":selected,"radius":radius}),
        &Caller::Mcp { client: "release-refusal".into() },
    );
    assert!(out.is_error && out.content.contains(expected), "{expected}: {}", out.content);
    assert_eq!(state.doc.model().part_json(), before);
    assert_eq!(state.doc.display_body(), body);
}

fn displayed(state: &AppState) {
    let doc = &state.doc;
    let body = doc.display_body().unwrap();
    assert!(doc.kernel().validate(body).unwrap().valid);
    let mesh = doc.kernel().tessellate(body, &Default::default()).unwrap();
    assert!(!mesh.indices.is_empty());
    let topology = doc.kernel().topology(body).unwrap();
    for face in &topology.faces {
        assert!(doc.named_face(face.id).is_some(), "unnamed face: {face:?}");
    }
}

fn export(state: &AppState, dir: &std::path::Path, name: &str) {
    let output = state.doc.model().export(Format::Step).unwrap();
    let Report::Step(report) = output.report else { panic!("STEP report") };
    assert!(report.within_tolerance, "{report:?}");
    assert_eq!(report.solids, 1);
    std::fs::write(dir.join(format!("{name}.step")), output.bytes).unwrap();
}

/// What a stage of the release loop must keep or change: the measured volume, the
/// topology counts and the persistent names of the displayed faces.
#[derive(Clone, Debug)]
struct Fingerprint {
    volume: f64,
    faces: usize,
    edges: usize,
    names: std::collections::BTreeSet<String>,
}

/// Volume agreement for the same part rebuilt by the kernel, as in the closed-form checks above.
const VOLUME_TOLERANCE: f64 = 1e-6;

impl Fingerprint {
    fn of(state: &AppState) -> Fingerprint {
        let doc = &state.doc;
        let body = doc.display_body().unwrap();
        let topology = doc.kernel().topology(body).unwrap();
        let names = topology
            .faces
            .iter()
            .map(|face| {
                let named = doc.named_face(face.id).expect("the face has a persistent name");
                // The interface names a face without a tree feature after the body, as
                // "Body · Face N". Every face of a released part has a feature that made it.
                let feature = doc
                    .feature(named.feature)
                    .unwrap_or_else(|| panic!("face {:?} has no feature and shows as Body · {}", face.id, named.label));
                format!("{} · {}", feature.name, named.label)
            })
            .collect();
        Fingerprint {
            volume: doc.kernel().mass_properties(body).unwrap().volume,
            faces: topology.faces.len(),
            edges: topology.edges.len(),
            names,
        }
    }

    fn assert_matches(&self, expected: &Fingerprint, stage: &str) {
        assert_eq!(
            (self.faces, self.edges, &self.names),
            (expected.faces, expected.edges, &expected.names),
            "{stage} changed the topology or face names"
        );
        assert!(
            (self.volume - expected.volume).abs() <= VOLUME_TOLERANCE,
            "{stage} volume {} != {}",
            self.volume,
            expected.volume
        );
    }
}

/// Which way the edit must move the volume. A larger convex round or a thinner wall removes
/// material; a larger concave round adds it. A smaller round does the reverse.
#[derive(Clone, Copy, Debug)]
enum Volume {
    Less,
    More,
}

fn exercise(state: AppState, name: &str, tool: &str, edit: Value, volume: Volume) {
    let proof = std::env::var_os("SOLIDDRAFT_RELEASE_PROOF_DIR");
    let dir = proof
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-release-{}", std::process::id())))
        .join(name);
    exercise_in(state, &dir, tool, edit, volume);
    if proof.is_none() {
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn exercise_in(mut state: AppState, dir: &std::path::Path, tool: &str, edit: Value, volume: Volume) {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("part.dwprt");
    accepted(&mut state, "save", json!({"path":path}));
    displayed(&state);
    export(&state, dir, "build");
    let built = Fingerprint::of(&state);
    accepted(&mut state, tool, edit);
    displayed(&state);
    export(&state, dir, "edit");
    let edited = Fingerprint::of(&state);
    eprintln!("{tool}: {built:?} -> {edited:?}");
    let change = edited.volume - built.volume;
    let moved = match volume {
        Volume::Less => change < -VOLUME_TOLERANCE,
        Volume::More => change > VOLUME_TOLERANCE,
    };
    assert!(moved, "{tool} did not change the volume as expected ({volume:?}): {} -> {}", built.volume, edited.volume);
    accepted(&mut state, "undo", json!({}));
    displayed(&state);
    Fingerprint::of(&state).assert_matches(&built, "undo");
    accepted(&mut state, "redo", json!({}));
    Fingerprint::of(&state).assert_matches(&edited, "redo");
    let saved = Fingerprint::of(&state);
    accepted(&mut state, "save", json!({"path":path}));
    accepted(&mut state, "open", json!({"path":path}));
    displayed(&state);
    Fingerprint::of(&state).assert_matches(&saved, "reopen");
    export(&state, dir, "reopen");
}

#[test]
fn an_edit_that_leaves_the_part_unchanged_fails_the_release_check() {
    let mut state = block();
    let top = edges(&state, |e| (e.midpoint.z - 12.0).abs() < 1e-6 && (e.midpoint.y - 15.0).abs() < 1e-6);
    round(&mut state, top, 1.0);
    let dir = std::env::temp_dir().join(format!("soliddraft-release-noop-{}", std::process::id()));
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        exercise_in(state, &dir, "edit_fillet", json!({"feature":"Fillet1","radius":1}), Volume::Less)
    }));
    std::fs::remove_dir_all(&dir).unwrap();
    let message = failed.expect_err("a no-op edit passed the release check");
    let message = message.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(message.contains("edit_fillet did not change the volume"), "{message}");
}

#[test]
fn crossing_cylinder_loop_builds_displays_edits_and_exports() {
    let mut state = fresh();
    sketch(&mut state, "top", json!({"shape":"circle","center":[0,0,0],"diameter":20}));
    accepted(&mut state, "extrude", json!({"sketch":"Sketch1","depth":40,"flip":false}));
    accepted(
        &mut state,
        "sketch",
        json!({"plane":"front","offset":-20,"shapes":[{"shape":"circle","center":[8,20,20],"diameter":8}]}),
    );
    accepted(&mut state, "extrude", json!({"sketch":"Sketch2","depth":40,"flip":false}));
    let selected = edges(&state, |e| e.curve == CurveKind::Other);
    round(&mut state, selected, 0.5);
    exercise(state, "loop", "edit_fillet", json!({"feature":"Fillet1","radius":0.75}), Volume::More);
}

#[test]
fn chamfered_cylinder_rim_rounds_between_curved_supports() {
    let mut state = fresh();
    sketch(&mut state, "top", json!({"shape":"circle","center":[0,0,0],"diameter":20}));
    accepted(&mut state, "extrude", json!({"sketch":"Sketch1","depth":20,"flip":false}));
    let rim = edges(&state, |e| e.curve == CurveKind::Circle && (e.midpoint.z - 20.0).abs() < 1e-6);
    accepted(&mut state, "chamfer", json!({"edges":rim,"distance":3}));
    let rim = edges(&state, |e| e.curve == CurveKind::Circle && (e.midpoint.z - 20.0).abs() < 1e-6);
    round(&mut state, rim, 0.5);
    exercise(state, "curved-rim", "edit_fillet", json!({"feature":"Fillet1","radius":0.75}), Volume::Less);
}

#[test]
fn pocket_strips_close_with_concave_spherical_corners() {
    let mut state = block();
    accepted(
        &mut state,
        "sketch",
        json!({"plane":"top","offset":12,"shapes":[{"shape":"rectangle","center":[0,0,12],"width":20,"height":16}]}),
    );
    accepted(&mut state, "cut", json!({"sketch":"Sketch2","depth":8,"through_all":false,"flip":false}));
    let selected = edges(&state, |e| {
        e.curve == CurveKind::Line
            && e.midpoint.x.abs() <= 10.0
            && e.midpoint.y.abs() <= 8.0
            && e.midpoint.z <= 8.0 + 1e-6
    });
    round(&mut state, selected, 1.0);
    let topology = state.doc.kernel().topology(state.doc.display_body().unwrap()).unwrap();
    assert_eq!(topology.faces.iter().filter(|f| f.surface == SurfaceKind::Sphere).count(), 4);
    exercise(state, "pocket-corners", "edit_fillet", json!({"feature":"Fillet1","radius":0.75}), Volume::Less);
}

#[test]
fn later_top_round_rolls_over_upright_rounds_at_smaller_and_equal_radii() {
    let mut state = block();
    let selected = edges(&state, |e| (e.midpoint.z - 6.0).abs() < 1e-6 && e.midpoint.x > 0.0 && e.midpoint.y > 0.0);
    assert_eq!(selected.len(), 1);
    round(&mut state, selected, 2.0);
    let selected = edges(&state, |e| {
        e.curve == CurveKind::Line && (e.midpoint.z - 12.0).abs() < 1e-6 && (e.midpoint.y - 15.0).abs() < 1e-6
    });
    assert_eq!(selected.len(), 1);
    round(&mut state, selected, 1.0);
    exercise(state, "fillet-over-fillet", "edit_fillet", json!({"feature":"Fillet2","radius":2}), Volume::Less);
}

#[test]
fn bore_mouth_and_blind_floor_round_on_the_material_side() {
    for blind in [false, true] {
        let mut state = block();
        let mut hole = json!({"face":"Boss-Extrude1 · End cap","points":[[0,0,12]],"diameter":6});
        if blind {
            hole["depth"] = json!(8);
        }
        accepted(&mut state, "hole", hole);
        let z = if blind { 4.0 } else { 12.0 };
        let selected = edges(&state, |e| e.curve == CurveKind::Circle && (e.midpoint.z - z).abs() < 1e-6);
        round(&mut state, selected, 1.0);
        // Independent integrals of the R1 torus strips on the R3 bore.
        let pi = std::f64::consts::PI;
        let expected =
            if blind { 14400.0 - 203.0 * pi / 3.0 - pi * pi } else { 14400.0 - 347.0 * pi / 3.0 + 2.0 * pi * pi };
        let volume = state.doc.kernel().mass_properties(state.doc.display_body().unwrap()).unwrap().volume;
        assert!((volume - expected).abs() < 1e-6, "{volume} != {expected}");
        exercise(
            state,
            if blind { "blind-floor" } else { "bore-mouth" },
            "edit_fillet",
            json!({"feature":"Fillet1","radius":0.75}),
            if blind { Volume::Less } else { Volume::More },
        );
    }
}

#[test]
fn through_bore_shell_builds_displays_edits_and_exports() {
    let mut state = block();
    accepted(&mut state, "hole", json!({"face":"Boss-Extrude1 · End cap","points":[[0,0,12]],"diameter":6}));
    accepted(&mut state, "shell", json!({"open_faces":["Boss-Extrude1 · End cap"],"thickness":2}));
    exercise(state, "bored-shell", "edit_shell", json!({"feature":"Shell1","thickness":1.5}), Volume::Less);
}

#[test]
fn nonconvex_shell_builds_displays_edits_and_exports() {
    let mut state = fresh();
    let vertices = [[0, 0], [8, 0], [8, 3], [3, 3], [3, 8], [0, 8]];
    let operations: Vec<_> = (0..vertices.len()).map(|i| {
        let a=vertices[i]; let b=vertices[(i+1)%vertices.len()];
        json!({"tool":"add","kind":{"type":"line","a":{"u":a[0],"v":a[1]},"b":{"u":b[0],"v":b[1]}},"construction":false})
    }).collect();
    accepted(&mut state, "sketch_geometry", json!({"plane":"top","operations":operations}));
    accepted(&mut state, "extrude", json!({"sketch":"Sketch1","depth":4,"flip":false}));
    accepted(&mut state, "shell", json!({"open_faces":["Boss-Extrude1 · End cap"],"thickness":1}));
    let mass = state.doc.kernel().mass_properties(state.doc.display_body().unwrap()).unwrap();
    assert!((mass.volume - (120.0 + 0.75 * std::f64::consts::PI)).abs() < 1e-6, "{mass:?}");
    // The wall turns round the reentrant edge on a cylinder the shell made, not a fillet.
    let names = Fingerprint::of(&state).names;
    assert!(names.contains("Shell1 · Inner round"), "{names:?}");
    exercise(state, "nonconvex-shell", "edit_shell", json!({"feature":"Shell1","thickness":0.75}), Volume::Less);
}

#[test]
fn tangent_shadow_and_ambiguous_equal_crossing_preserve_the_part() {
    for (equal, expected) in [(false, "fillet.radius-too-large"), (true, "recipe.ambiguous-reference")] {
        let mut state = fresh();
        accepted(
            &mut state,
            "sketch",
            json!({"plane":"top","offset":0,"shapes":[{
            "shape":"circle","center":[if equal {0} else {8},0,0],"diameter":if equal {20} else {8}}]}),
        );
        accepted(&mut state, "extrude", json!({"sketch":"Sketch1","depth":40,"flip":false}));
        accepted(
            &mut state,
            "sketch",
            json!({"plane":"front","offset":-20,"shapes":[{
            "shape":"circle","center":[0,20,20],"diameter":20}]}),
        );
        if equal {
            accepted(&mut state, "extrude", json!({"sketch":"Sketch2","depth":40,"flip":false}));
        } else {
            accepted(&mut state, "cut", json!({"sketch":"Sketch2","depth":40,"through_all":false,"flip":true}));
        }
        let selected = edges(&state, |e| e.curve == CurveKind::Other || (equal && e.curve == CurveKind::Ellipse));
        refused(&mut state, selected, 1.0, expected);
    }
}

#[test]
fn unequal_radius_mitre_reports_the_typed_corner_refusal() {
    let mut state = block();
    let top = edges(&state, |e| (e.midpoint.z - 12.0).abs() < 1e-6);
    round(&mut state, top, 2.0);
    let uprights = edges(&state, |e| e.curve == CurveKind::Line && e.bbox.max.z - e.bbox.min.z > 5.0);
    refused(&mut state, uprights, 1.0, "fillet.unsupported-corner");
}

#[path = "support/ribbon.rs"]
mod ribbon;

fn wait_for(app: &mut ribbon::App, ready: impl Fn(&ribbon::App) -> bool) {
    let started = std::time::Instant::now();
    while !ready(app) {
        assert!(started.elapsed().as_secs() < 120, "the editor did not settle");
        app.run_steps(2);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn rendered_real_use_rounds_an_earlier_round_then_shells_an_l_tray() {
    use egui::Key;
    use egui_kittest::kittest::Queryable;
    let dir = std::env::var_os("SOLIDDRAFT_RELEASE_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-release-{}", std::process::id())))
        .join("real-use");
    std::fs::create_dir_all(&dir).unwrap();
    let mut state = block();
    let selected = edges(&state, |e| (e.midpoint.z - 6.0).abs() < 1e-6);
    round(&mut state, selected, 2.0);
    let mut app = ribbon::app(state.doc);
    app.key_press(Key::Z);
    app.run_steps(30);
    ribbon::click_tab(&mut app, "Solid");
    ribbon::click_ribbon(&mut app, "Fillet");
    ribbon::click_world(&mut app, glam::Vec3::new(0.0, 0.0, 12.0));
    ribbon::type_field(&mut app, "Radius value", "3");
    wait_for(&mut app, |a| a.state().state.editor.as_ref().is_some_and(|e| e.error.is_some()));
    let error = app.state().state.editor.as_ref().unwrap().error.as_ref().unwrap();
    assert!(error.contains("fillet.radius-too-large"), "{error}");
    app.render().unwrap().save(dir.join("radius-refused.png")).unwrap();
    ribbon::type_field(&mut app, "Radius value", "2");
    wait_for(&mut app, |a| {
        a.state().state.editor.as_ref().is_some_and(|e| e.preview.result.is_some() && e.error.is_none())
    });
    app.render().unwrap().save(dir.join("equal-radius-preview.png")).unwrap();
    let window = ribbon::window(&app);
    ribbon::click_button(&mut app, "OK (Enter)", window);
    app.run_steps(30);
    assert!(app.state().state.editor.is_none());
    displayed(&app.state().state);
    app.render().unwrap().save(dir.join("round-committed.png")).unwrap();
    drop(app);

    let mut state = fresh();
    let vertices = [[0, 0], [8, 0], [8, 3], [3, 3], [3, 8], [0, 8]];
    let operations: Vec<_> = (0..vertices.len()).map(|i| {
        let a=vertices[i]; let b=vertices[(i+1)%vertices.len()];
        json!({"tool":"add","kind":{"type":"line","a":{"u":a[0],"v":a[1]},"b":{"u":b[0],"v":b[1]}},"construction":false})
    }).collect();
    accepted(&mut state, "sketch_geometry", json!({"plane":"top","operations":operations}));
    accepted(&mut state, "extrude", json!({"sketch":"Sketch1","depth":4,"flip":false}));
    let mut app = ribbon::app(state.doc);
    app.key_press(Key::Z);
    app.run_steps(30);
    ribbon::click_tab(&mut app, "Solid");
    ribbon::click_ribbon(&mut app, "Shell");
    ribbon::click_world(&mut app, glam::Vec3::new(1.5, 1.5, 4.0));
    ribbon::type_field(&mut app, "Wall thickness value", "1");
    wait_for(&mut app, |a| {
        a.state().state.editor.as_ref().is_some_and(|e| e.preview.result.is_some() && e.error.is_none())
    });
    app.render().unwrap().save(dir.join("l-shell-preview.png")).unwrap();
    let window = ribbon::window(&app);
    ribbon::click_button(&mut app, "OK (Enter)", window);
    app.run_steps(30);
    assert!(app.state().state.editor.is_none());
    displayed(&app.state().state);
    let volume =
        app.state().state.doc.kernel().mass_properties(app.state().state.doc.display_body().unwrap()).unwrap().volume;
    assert!((volume - (120.0 + 0.75 * std::f64::consts::PI)).abs() < 1e-6);
    app.render().unwrap().save(dir.join("l-shell-committed.png")).unwrap();
    assert!(app.get_all_by_label("Shell1").next().is_some());
}
