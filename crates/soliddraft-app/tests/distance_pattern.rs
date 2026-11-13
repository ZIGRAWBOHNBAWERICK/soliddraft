//! Exact distance features and source-feature patterns through real application commands.
use serde_json::{Value, json};
use soliddraft_app::{
    app_tools::AppTools,
    document::{Command, Document, ExtrudeParams, ProfileKind},
    state::AppState,
};
use soliddraft_assistant::{Caller, ToolOutput, tools::call_checked};

fn blank(w: f64, h: f64, d: f64) -> AppState {
    let mut doc = Document::new("Distance and pattern", soliddraft_app::kernel());
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams {
            profile: ProfileKind::Rectangle { width: w, height: h },
            depth: d,
            ..Default::default()
        },
        via_assistant: false,
    })
    .unwrap();
    AppState::new(doc)
}
fn call(s: &mut AppState, name: &str, args: Value) -> ToolOutput {
    call_checked(&mut AppTools { state: s }, name, args, &Caller::Mcp { client: "distance-pattern-test".into() })
}
fn accept(s: &mut AppState, name: &str, args: Value) {
    let out = call(s, name, args);
    assert!(!out.is_error, "{name}: {}", out.content);
}
fn volume(s: &AppState, expected: f64) {
    let actual = s.doc.kernel().mass_properties(s.doc.display_body().unwrap()).unwrap().volume;
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}
fn plane_limits(s: &AppState, axis: soliddraft_kernel_api::Vec3) -> (f64, f64) {
    let planes = s.doc.kernel().face_carriers(s.doc.display_body().unwrap()).unwrap();
    let values = planes
        .iter()
        .filter(|f| f.normal == Some(axis) || f.normal == Some(-axis))
        .map(|f| f.on_plane.expect("planar carrier").dot(axis))
        .collect::<Vec<_>>();
    (*values.iter().min_by(|a, b| a.total_cmp(b)).unwrap(), *values.iter().max_by(|a, b| a.total_cmp(b)).unwrap())
}
fn reopen(s: &mut AppState, file: &str) {
    let dir = std::env::temp_dir().join(format!("soliddraft-distance-pattern-{}-{file}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let part = dir.join("part.dwprt");
    accept(s, "save", json!({"path":part}));
    accept(s, "open", json!({"path":part}));
    let step = dir.join("part.step");
    accept(s, "export_step", json!({"path":step}));
    assert!(std::fs::read_to_string(step).unwrap().contains("MANIFOLD_SOLID_BREP"));
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn offset_edits_undo_reopens_and_refuses_without_changing_the_accepted_part() {
    let mut s = blank(10.0, 8.0, 6.0);
    accept(&mut s, "offset", json!({"distance":-1}));
    volume(&s, 192.0);
    let reference = {
        let d = &s.doc;
        let t = d.kernel().topology(d.display_body().unwrap()).unwrap();
        d.any_face_ref(t.faces.iter().find(|f| f.normal == Some(soliddraft_kernel_api::Vec3::Z)).unwrap().id).unwrap()
    };
    accept(&mut s, "edit_offset", json!({"feature":"Offset1","distance":-0.5}));
    volume(&s, 315.0);
    s.doc.undo().unwrap();
    volume(&s, 192.0);
    s.doc.redo().unwrap();
    volume(&s, 315.0);
    reopen(&mut s, "offset");
    volume(&s, 315.0);
    assert!(s.doc.model().resolve_face(&reference, s.doc.display_body().unwrap()).is_ok());
    let body = s.doc.display_body();
    let undo = s.doc.undo_depth();
    let count = s.doc.features.len();
    for distance in [0.0, -10.0] {
        let out = call(&mut s, "edit_offset", json!({"feature":"Offset1","distance":distance}));
        assert!(out.is_error, "{}", out.content);
        assert!(!out.content.contains("catallax"), "{}", out.content);
        assert_eq!(s.doc.display_body(), body);
        assert_eq!(s.doc.undo_depth(), undo);
        assert_eq!(s.doc.features.len(), count);
    }
}
#[test]
fn thicken_sides_and_persistent_rims_survive_edit_and_reopen() {
    let mut s = blank(10.0, 8.0, 6.0);
    accept(&mut s, "thicken", json!({"faces":["Boss-Extrude1 · End cap"],"thickness":2,"side":"front"}));
    volume(&s, 160.0);
    let references = {
        let d = &s.doc;
        let t = d.kernel().topology(d.display_body().unwrap()).unwrap();
        t.faces.iter().map(|f| d.any_face_ref(f.id).expect("every thickened face has a name")).collect::<Vec<_>>()
    };
    assert!(references.iter().any(|f| f.generator == soliddraft_model::Generator::ThickenRim));
    accept(&mut s, "edit_thicken", json!({"feature":"Thicken1","thickness":1,"side":"front"}));
    volume(&s, 80.0);
    assert_eq!(plane_limits(&s, soliddraft_kernel_api::Vec3::Z), (6.0, 7.0));
    reopen(&mut s, "thicken");
    volume(&s, 80.0);
    for r in references {
        assert!(s.doc.model().resolve_face(&r, s.doc.display_body().unwrap()).is_ok(), "{}", r.label);
    }
    accept(&mut s, "edit_thicken", json!({"feature":"Thicken1","thickness":1,"side":"back"}));
    assert_eq!(plane_limits(&s, soliddraft_kernel_api::Vec3::Z), (5.0, 6.0));
    let body = s.doc.display_body();
    let undo = s.doc.undo_depth();
    for args in
        [json!({"faces":[],"thickness":2,"side":"front"}), json!({"faces":["missing"],"thickness":2,"side":"front"})]
    {
        assert!(call(&mut s, "thicken", args).is_error);
        assert_eq!(s.doc.display_body(), body);
        assert_eq!(s.doc.undo_depth(), undo);
    }
}
fn cut(s: &mut AppState, center: [f64; 3], name: &str) {
    let center = [center[0], center[1], 2.0];
    accept(
        s,
        "sketch",
        json!({"plane":"Boss-Extrude1 · End cap","shapes":[{"shape":"rectangle","center":center,"width":2,"height":2}]}),
    );
    accept(s, "cut", json!({"sketch":name,"depth":2,"through_all":true,"flip":false}));
}
#[test]
fn feature_pattern_repeats_only_selected_cut_through_edit_undo_and_reopen() {
    let mut s = blank(30.0, 10.0, 2.0);
    cut(&mut s, [-10.0, 0.0, 0.0], "Sketch1");
    volume(&s, 592.0);
    cut(&mut s, [12.0, 0.0, 0.0], "Sketch2");
    volume(&s, 584.0);
    accept(&mut s, "linear_pattern", json!({"source":"Cut-Extrude1","direction":[1,0,0],"spacing":6,"count":3}));
    volume(&s, 568.0);
    let reference = {
        let d = &s.doc;
        let t = d.kernel().topology(d.display_body().unwrap()).unwrap();
        t.faces
            .iter()
            .filter_map(|f| d.any_face_ref(f.id))
            .find(|r| matches!(r.generator, soliddraft_model::Generator::Copy { instance: 1 }))
            .unwrap()
    };
    assert_eq!(plane_limits(&s, soliddraft_kernel_api::Vec3::X), (-15.0, 15.0));
    accept(&mut s, "edit_pattern", json!({"feature":"LPattern1","count":4,"step":6}));
    volume(&s, 560.0);
    s.doc.undo().unwrap();
    volume(&s, 568.0);
    s.doc.redo().unwrap();
    volume(&s, 560.0);
    reopen(&mut s, "pattern");
    volume(&s, 560.0);
    assert!(s.doc.model().resolve_face(&reference, s.doc.display_body().unwrap()).is_ok());
    let body = s.doc.display_body();
    let undo = s.doc.undo_depth();
    let out = call(&mut s, "linear_pattern", json!({"source":"LPattern1","direction":[1,0,0],"spacing":6,"count":3}));
    assert!(out.is_error);
    assert_eq!(s.doc.display_body(), body);
    assert_eq!(s.doc.undo_depth(), undo);
}
#[test]
fn circular_cut_uses_quarter_turns_and_preserves_the_part_on_exact_parameter_refusal() {
    let mut s = blank(20.0, 20.0, 2.0);
    cut(&mut s, [6.0, 0.0, 0.0], "Sketch1");
    volume(&s, 792.0);
    let body = s.doc.display_body();
    let undo = s.doc.undo_depth();
    let args =
        |angle| json!({"source":"Cut-Extrude1","axis_origin":[0,0,0],"axis_direction":[0,0,1],"count":4,"angle":angle});
    let out = call(&mut s, "circular_pattern", args(45));
    assert!(out.is_error && out.content.contains("cannot be saved exactly"), "{}", out.content);
    assert_eq!(s.doc.display_body(), body);
    assert_eq!(s.doc.undo_depth(), undo);
    accept(&mut s, "circular_pattern", args(90));
    volume(&s, 768.0);
    reopen(&mut s, "ring");
    volume(&s, 768.0);
}

#[path = "support/ui.rs"]
mod ribbon;
use egui_kittest::kittest::{NodeT, Queryable};
use soliddraft_app::state::{AppAction, EditorParams};

fn field(app: &mut ribbon::App, label: &str, value: &str) {
    app.get_by_label(label).click();
    app.run_steps(2);
    app.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    app.get_by_label(label).type_text(value);
    app.run_steps(3);
    app.key_press(egui::Key::Enter);
    app.run_steps(4);
}
#[test]
fn distance_commands_render_in_both_frames_and_hide_pins_during_sketches() {
    let case = ribbon::case("distance-features");
    for frame in ["classic", "edit_suite"] {
        std::fs::write(case.path("config/settings.json"), format!(r#"{{"frame_layout":"{frame}"}}"#)).unwrap();
        let pins = soliddraft_app::command_line::BarSettings {
            tools: ["offset", "thicken", "linear_pattern", "circular_pattern"].map(str::to_owned).to_vec(),
            ..Default::default()
        };
        soliddraft_app::settings::save_command_line(&pins).unwrap();
        let mut app = ribbon::app_at(blank(10.0, 8.0, 6.0).doc, ribbon::EXPANDED_WINDOW, 1.0);
        case.run(&mut app, |app| {
            app.get_by_label("Offset on the command bar").click();
            app.run_steps(5);
            field(app, "Offset value", "-1");
            assert!(app.state().state.editor.as_ref().unwrap().preview.result.is_some());
            app.get_by_label("OK").click();
            app.run_steps(10);
            volume(&app.state().state, 192.0);
            let id = app.state().state.doc.features.last().unwrap().id;
            app.state_mut().state.push(AppAction::EditFeature(id));
            app.run_steps(4);
            field(app, "Offset value", "0");
            let editor = app.state().state.editor.as_ref().unwrap();
            assert!(editor.error.as_ref().is_some_and(|e| e.contains("nonzero")), "{:?}", editor.error);
            assert!(editor.preview.result.is_none());
            case.frame(app, &format!("{frame}-offset-refused"));
            field(app, "Offset value", "-0.5");
            app.get_by_label("OK").click();
            app.run_steps(10);
            volume(&app.state().state, 315.0);
            case.frame(app, &format!("{frame}-offset"));
            app.state_mut().state.push(AppAction::StartSketchOn(soliddraft_app::document::SketchPlane::Reference {
                plane: soliddraft_app::document::PlaneKind::Top,
                offset: 0.0,
            }));
            app.run_steps(10);
            assert!(app.state().state.in_sketch());
            for label in ["Offset", "Thicken", "Linear", "Circular"] {
                assert!(
                    app.query_all_by_label(&format!("{label} on the command bar")).next().is_none(),
                    "{frame}: {label}"
                );
            }
            assert_eq!(app.state().state.command_line.settings.tools, pins.tools);
            ribbon::run_command(app, "exit_sketch");
            for label in ["Offset", "Thicken", "Linear", "Circular"] {
                app.get_by_label(&format!("{label} on the command bar"));
            }
        });
    }
}
#[test]
fn thicken_collects_a_viewport_face_and_pattern_picker_repeats_a_cut() {
    let case = ribbon::case("thicken-pattern-controls");
    for frame in ["classic", "edit_suite"] {
        std::fs::write(case.path("config/settings.json"), format!(r#"{{"frame_layout":"{frame}"}}"#)).unwrap();
        let mut app = ribbon::app_at(blank(10.0, 8.0, 6.0).doc, ribbon::EXPANDED_WINDOW, 1.0);
        case.run(&mut app, |app| {
            ribbon::run_command(app, "zoom_fit");
            ribbon::run_command(app, "thicken");
            let screen = {
                let s = &app.state().state;
                s.viewport.camera.project(glam::Vec3::new(0.0, 0.0, 6.0), s.viewport.rect).unwrap()
            };
            ribbon::pick_at(app, screen);
            app.run_steps(6);
            let editor = app.state().state.editor.as_ref().unwrap();
            let EditorParams::Thicken(s) = &editor.params else { panic!("Thicken editor") };
            assert_eq!(s.faces.len(), 1);
            assert!(editor.preview.result.is_some(), "{:?}", editor.error);
            field(app, "Thickness value", "1");
            app.get_by_label("Thicken side").click();
            app.run_steps(2);
            app.get_by_label("Back").click();
            app.run_steps(4);
            assert_eq!(app.get_by_label("Thicken side").accesskit_node().value().as_deref(), Some("Back"));
            app.get_by_label("OK").click();
            app.run_steps(10);
            volume(&app.state().state, 80.0);
            case.frame(app, &format!("{frame}-thicken-back"));
        });
        drop(app);
        let mut s = blank(20.0, 20.0, 2.0);
        cut(&mut s, [6.0, 0.0, 0.0], "Sketch1");
        let source = s.doc.features.last().unwrap().id;
        s.doc.selection = Some(soliddraft_app::document::Selection::Feature(source));
        let mut app = ribbon::app_at(s.doc, ribbon::EXPANDED_WINDOW, 1.0);
        case.run(&mut app, |app| {
            ribbon::run_command(app, "circular_pattern");
            let EditorParams::Pattern(p) = &app.state().state.editor.as_ref().unwrap().params else {
                panic!("Pattern editor")
            };
            assert_eq!(p.source, Some(soliddraft_model::FeatureId(source as u32)));
            assert_eq!(app.get_by_label("Pattern source").accesskit_node().value().as_deref(), Some("Cut-Extrude1"));
            case.frame(app, &format!("{frame}-selected-feature-pattern"));
            app.get_by_label("OK").click();
            app.run_steps(10);
            volume(&app.state().state, 768.0);
            case.frame(app, &format!("{frame}-patterned-cuts"));
        });
    }
}
