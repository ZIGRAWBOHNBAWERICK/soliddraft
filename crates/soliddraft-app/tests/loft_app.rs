//! Loft through real application commands and the rendered profile editor.

use serde_json::{Value, json};
use soliddraft_app::command_tools::{CommandTools, ProposalBook, Surface};
use soliddraft_app::document::{Document, FeatureKind, Selection};
use soliddraft_assistant::{Caller, ToolHost};
use soliddraft_kernel_api::LoftShape;

fn call(doc: &mut Document, name: &str, input: Value) -> Value {
    let mut book = ProposalBook::default();
    let result = CommandTools { doc, book: &mut book, surface: Surface::Mcp, drawing: None }.call(
        name,
        input,
        &Caller::Mcp { client: "loft test".into() },
    );
    assert!(!result.is_error, "{name}: {}", result.content);
    serde_json::from_str(&result.content).unwrap()
}

fn profiles() -> Document {
    let mut doc = Document::new("Loft", soliddraft_app::kernel());
    for (size, z) in [(2.0, 0), (4.0, 3), (2.0, 6)] {
        rectangle(&mut doc, z, 0.0, size, size);
    }
    doc
}

fn rectangle(doc: &mut Document, z: i32, x: f64, width: f64, height: f64) {
    call(
        doc,
        "sketch",
        json!({"plane":"top","offset":z,"shapes":[{"shape":"rectangle","center":[x,0,z],"width":width,"height":height}]}),
    );
}

fn volume(doc: &Document) -> f64 {
    doc.kernel().mass_properties(doc.display_body().expect("a solid")).unwrap().volume
}

fn assert_loft(doc: &Document, shape: LoftShape, profile_count: usize, expected_volume: f64) {
    let FeatureKind::Loft(loft) = &doc.features[3].kind else { panic!("expected Loft1") };
    assert_eq!(loft.shape, shape);
    assert_eq!(loft.profiles.len(), profile_count);
    let actual = volume(doc);
    assert!((actual - expected_volume).abs() < 1e-7, "expected volume {expected_volume}, got {actual}");
}

#[test]
fn loft_edit_cache_undo_reopen_and_refusal_preserve_the_exact_part() {
    let mut doc = Document::new("Loft", soliddraft_app::kernel());
    // The first two rectangles form a prism. Widening the last at one edge curves only one wall.
    rectangle(&mut doc, 0, 1.0, 2.0, 2.0);
    rectangle(&mut doc, 7, 1.0, 2.0, 2.0);
    rectangle(&mut doc, 16, 17.0 / 16.0, 17.0 / 8.0, 2.0);
    call(&mut doc, "loft", json!({"profiles":["Sketch1","Sketch2"],"shape":"ruled"}));
    assert_loft(&doc, LoftShape::Ruled, 2, 28.0);
    let first = doc.display_body().unwrap();
    for name in ["loft", "edit_loft"] {
        let before = doc.model().to_bytes().unwrap();
        let mut book = ProposalBook::default();
        let mut input = json!({"profiles":["Sketch1"],"shape":"ruled"});
        if name == "edit_loft" {
            input["feature"] = json!("Loft1");
        }
        let result = CommandTools { doc: &mut doc, book: &mut book, surface: Surface::Mcp, drawing: None }.call(
            name,
            input,
            &Caller::Mcp { client: "loft profile count".into() },
        );
        assert!(result.is_error, "{name}: {}", result.content);
        assert!(result.content.contains("Loft needs at least two profiles in order"), "{}", result.content);
        assert_eq!(before, doc.model().to_bytes().unwrap());
        assert_eq!(doc.display_body(), Some(first));
    }
    call(&mut doc, "edit_loft", json!({"feature":"Loft1","profiles":["Sketch1","Sketch2","Sketch3"],"shape":"smooth"}));
    // Integrate the section area (2 * width) against z, using natural cubics through the three
    // stations for width and z. The ruled result is 65.125 and dropping Sketch2 gives 66.
    let smooth_volume = 64.90625;
    assert_loft(&doc, LoftShape::Smooth, 3, smooth_volume);
    call(&mut doc, "undo", json!({}));
    assert_eq!(doc.display_body(), Some(first));
    assert_loft(&doc, LoftShape::Ruled, 2, 28.0);
    call(&mut doc, "redo", json!({}));
    assert_loft(&doc, LoftShape::Smooth, 3, smooth_volume);
    let before = doc.model().to_bytes().unwrap();
    let body = doc.display_body();
    let mut book = ProposalBook::default();
    let result = CommandTools { doc: &mut doc, book: &mut book, surface: Surface::Mcp, drawing: None }.call(
        "edit_loft",
        json!({"feature":"Loft1","profiles":["Sketch1","Sketch1"],"shape":"ruled"}),
        &Caller::Mcp { client: "loft refusal".into() },
    );
    assert!(result.is_error);
    assert!(result.content.contains("Loft profiles must enclose a solid"), "{}", result.content);
    assert_eq!(before, doc.model().to_bytes().unwrap());
    assert_eq!(body, doc.display_body());
    let path = std::env::temp_dir().join(format!("loft-{}.dwprt", soliddraft_document::DocumentId::new()));
    call(&mut doc, "save", json!({"path":path}));
    let mut reopened = Document::new("Reopened", soliddraft_app::kernel());
    call(&mut reopened, "open", json!({"path":path}));
    assert_loft(&reopened, LoftShape::Smooth, 3, smooth_volume);
    assert_eq!(doc.features[3].kind, reopened.features[3].kind);
    assert!(
        reopened
            .kernel()
            .export_step(reopened.display_body().unwrap(), &Default::default())
            .unwrap()
            .text
            .contains("MANIFOLD_SOLID_BREP")
    );
    std::fs::remove_file(path).unwrap();

    // Editing a source after a successful loft must miss its cached profile versions.
    let sketch_id = doc.features[1].id;
    let FeatureKind::Sketch(mut sketch) = doc.features[1].kind.clone() else { panic!("sketch") };
    sketch.plane = soliddraft_app::document::SketchPlane::Reference {
        plane: soliddraft_app::document::PlaneKind::Top,
        offset: 8.0,
    };
    doc.apply(soliddraft_app::document::Command::EditFeature { id: sketch_id, kind: FeatureKind::Sketch(sketch) })
        .unwrap();
    assert_ne!(body, doc.display_body());
    // With Sketch2 at z = 8, the same integral changes to 64.75 (ruled: 65).
    assert_loft(&doc, LoftShape::Smooth, 3, 64.75);
}

#[test]
fn loft_uses_an_exact_face_boundary_and_joins_to_the_existing_part() {
    let mut doc = profiles();
    call(&mut doc, "extrude", json!({"sketch":"Sketch1","depth":1,"flip":false}));
    let top = doc
        .faces
        .iter()
        .find(|f| f.centroid.z > 0.99 && f.surface == soliddraft_kernel_api::SurfaceKind::Plane)
        .unwrap()
        .id;
    let top = doc.any_face_ref(top).unwrap().label;
    call(&mut doc, "loft", json!({"profiles":[top,"Sketch2"],"shape":"ruled"}));
    assert!((volume(&doc) - (4.0 + 56.0 / 3.0)).abs() < 1e-7, "{}", volume(&doc));
    assert!(doc.faces.iter().all(|f| doc.any_face_ref(f.id).is_some()));
}

#[path = "support/ui.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

fn open_profiles(app: &mut ribbon::App) {
    use egui_kittest::kittest::Queryable;

    let bar = egui::PanelState::load(&app.ctx, egui::Id::new("options_bar")).unwrap().outer_rect;
    let (_, rect) = ribbon::buttons_in(app, bar).into_iter().find(|(name, _)| name.starts_with("More (")).unwrap();
    ribbon::click_at(app, rect.center());
    ux_tools::settle(app);
    assert!(bar.contains_rect(app.get_by_label("OK").rect()), "finish stays on the bar above the profile list");
}

#[test]
fn warmed_loft_completion_disappears_when_a_sketch_opens_in_edit_suite() {
    use egui::Key;
    use egui_kittest::kittest::Queryable;

    let mut case = ux_tools::Case::new("loft-completion");
    let loft = format!("{:<24}{}", "loft", "Loft");
    {
        let layout = "edit_suite";
        let mut doc = ribbon::sketch();
        doc.selection = Some(Selection::Feature(doc.features[0].id));
        let mut app = ux_tools::launch(doc, ux_tools::STANDARD);

        ux_tools::settle(&mut app);
        app.get_by_label("Command line").click();
        app.run_steps(2);
        app.get_by_label("Command line").type_text("lo");
        app.run_steps(2);
        app.key_press(Key::Tab);
        app.run_steps(4);
        assert!(app.query_by_label(&loft).is_some(), "Loft is offered before sketch entry");
        let history = app.state().state.command_line.history.clone();
        case.frame(&mut app, &format!("{layout:?}-model-completions"));
        ribbon::run_command(&mut app, "edit_feature");
        ux_tools::settle(&mut app);
        case.frame(&mut app, &format!("{layout:?}-sketch-completions"));
        assert!(app.state().state.in_sketch());
        assert_eq!(app.state().state.command_line.input, "lo");
        assert_eq!(app.state().state.command_line.history, history);
        assert!(app.query_by_label(&loft).is_none(), "cached Loft must be hidden in {layout:?}");
        app.get_by_label("Command line").click();
        app.run_steps(2);
        for _ in 0..12 {
            app.key_press(Key::Tab);
            app.run_steps(2);
            assert!(app.state().state.command_line.input.starts_with("look_"));
            assert!(app.query_by_label(&loft).is_none());
        }
        assert_eq!(app.state().state.command_line.history, history);
    }
}

#[test]
fn loft_registry_command_opens_the_real_editor_builds_and_shows_a_plain_refusal() {
    use egui_kittest::kittest::Queryable;
    use soliddraft_app::state::EditorParams;
    let mut case = ux_tools::Case::new("loft");
    {
        let layout = "edit_suite";
        let mut doc = profiles();
        doc.selection = Some(Selection::Feature(doc.features[0].id));
        let mut app = ux_tools::launch(doc, ux_tools::STANDARD);

        app.state_mut()
            .state
            .viewport
            .camera
            .fit_framed(Some((glam::Vec3::new(-2.0, -2.0, 0.0), glam::Vec3::new(2.0, 2.0, 6.0))));
        ux_tools::settle(&mut app);
        ribbon::run_command(&mut app, "loft");
        ux_tools::settle(&mut app);
        assert!(matches!(app.state().state.editor.as_ref().unwrap().params, EditorParams::Loft(_)));
        assert!(app.query_by_label("Loft needs at least two profiles in order").is_some());
        app.get_by_label("OK").click();
        ux_tools::settle(&mut app);
        assert!(app.state().state.editor.is_some());
        assert_eq!(app.state().state.doc.features.len(), 3);
        case.frame(&mut app, &format!("{layout:?}-refused"));
        open_profiles(&mut app);
        app.query_by_label("Add profile").unwrap().click();
        ux_tools::settle(&mut app);
        // The open menu follows the tree row with the same sketch name.
        app.get_all_by_label("Sketch2").last().unwrap().click();
        ux_tools::settle(&mut app);
        assert!(app.state().state.editor.as_ref().unwrap().error.is_none());
        assert!(app.state().state.editor.as_ref().unwrap().preview.result.is_some());
        case.frame(&mut app, &format!("{layout:?}-preview"));
        app.get_by_label("OK").click();
        ux_tools::settle(&mut app);
        ux_tools::rebuilt(&mut app);
        assert!(matches!(app.state().state.doc.features[3].kind, FeatureKind::Loft(_)));
        assert!((volume(&app.state().state.doc) - 28.0).abs() < 1e-7);
        case.frame(&mut app, &format!("{layout:?}-built"));
        app.state_mut().state.push(soliddraft_app::state::AppAction::StartSketchOn(
            soliddraft_app::document::SketchPlane::Reference {
                plane: soliddraft_app::document::PlaneKind::Top,
                offset: 9.0,
            },
        ));
        ux_tools::settle(&mut app);
        let loft = soliddraft_app::commands::get("loft").unwrap();
        assert!(app.state().state.in_sketch());
        assert!(!(loft.shown)(&app.state().state));
        assert!(loft.action_now(&app.state().state).is_none());
    }
}

#[test]
fn existing_loft_can_preview_and_commit_profile_and_mode_edits() {
    use egui_kittest::kittest::Queryable;
    use soliddraft_app::state::EditorParams;
    let mut case = ux_tools::Case::new("loft-edit");
    {
        let layout = "edit_suite";
        let mut doc = profiles();
        call(&mut doc, "loft", json!({"profiles":["Sketch1","Sketch2"],"shape":"ruled"}));
        let id = doc.features.last().unwrap().id;
        doc.selection = Some(Selection::Feature(id));
        let mut app = ux_tools::launch(doc, ux_tools::STANDARD);

        app.state_mut()
            .state
            .viewport
            .camera
            .fit_framed(Some((glam::Vec3::new(-2.0, -2.0, 0.0), glam::Vec3::new(2.0, 2.0, 6.0))));
        ux_tools::settle(&mut app);
        assert_eq!(app.state().state.doc.rollback, None);
        ribbon::run_command(&mut app, "edit_feature");
        ux_tools::settle(&mut app);
        assert_eq!(app.state().state.editor.as_ref().unwrap().target, Some(id));
        assert_eq!(app.state().state.doc.rollback, Some(3));
        open_profiles(&mut app);
        app.get_all_by_label("Remove").last().unwrap().click();
        ux_tools::settle(&mut app);
        app.get_by_label("Add profile").click();
        ux_tools::settle(&mut app);
        app.get_all_by_label("Sketch3").last().unwrap().click();
        ux_tools::settle(&mut app);

        app.get_by_value("Ruled").click();
        ux_tools::settle(&mut app);

        app.get_by_label("Smooth").click();
        ux_tools::settle(&mut app);
        let editor = app.state().state.editor.as_ref().unwrap();
        assert!(matches!(&editor.params, EditorParams::Loft(l)
            if l.profiles.len() == 2 && l.shape == soliddraft_kernel_api::LoftShape::Smooth));
        assert!(editor.error.is_none(), "existing Loft edit preview: {:?}", editor.error);
        assert!(editor.preview.result.is_some());
        assert_eq!(app.state().state.doc.rollback, Some(3));
        assert!(matches!(&app.state().state.doc.feature(id).unwrap().kind, FeatureKind::Loft(l)
            if l.profiles.len() == 2 && l.shape == soliddraft_kernel_api::LoftShape::Ruled));
        case.frame(&mut app, &format!("{layout:?}-edited-preview"));
        app.get_by_label("OK").click();
        ux_tools::settle(&mut app);
        ux_tools::rebuilt(&mut app);
        assert!(app.state().state.editor.is_none());
        assert_eq!(app.state().state.doc.rollback, None);
        assert!(matches!(&app.state().state.doc.feature(id).unwrap().kind, FeatureKind::Loft(l)
            if l.profiles.len() == 2 && l.shape == soliddraft_kernel_api::LoftShape::Smooth));
        assert!((volume(&app.state().state.doc) - 24.0).abs() < 1e-7);
        case.frame(&mut app, &format!("{layout:?}-edited-committed"));
    }
}

#[test]
fn three_section_smooth_loft_previews_and_commits_without_changing_the_allowance() {
    use egui_kittest::kittest::Queryable;
    use soliddraft_app::state::EditorParams;

    let mut case = ux_tools::Case::new("loft-smooth-three-section");
    let mut doc = Document::new("Loft", soliddraft_app::kernel());
    // Widen only one edge of the middle rectangle to limit spline face integration.
    rectangle(&mut doc, 0, 1.0, 2.0, 2.0);
    rectangle(&mut doc, 3, 1.125, 2.25, 2.0);
    rectangle(&mut doc, 6, 1.0, 2.0, 2.0);
    doc.selection = Some(Selection::Feature(doc.features[0].id));
    let mut app = ux_tools::launch(doc, ux_tools::STANDARD);
    app.state_mut()
        .state
        .viewport
        .camera
        .fit_framed(Some((glam::Vec3::new(0.0, -1.0, 0.0), glam::Vec3::new(2.25, 1.0, 6.0))));
    ux_tools::settle(&mut app);
    case.run(&mut app, |case, app| {
        ribbon::run_command(app, "loft");
        ux_tools::settle(app);
        open_profiles(app);
        for label in ["Sketch2", "Sketch3"] {
            app.get_by_label("Add profile").click();
            ux_tools::settle(app);
            app.get_all_by_label(label).last().unwrap().click();
            ux_tools::settle(app);
        }
        ribbon::close_options(app);
        app.get_by_value("Ruled").click();
        ux_tools::settle(app);
        app.get_by_label("Smooth").click();
        ux_tools::settle(app);
        case.frame(app, "preview");
        let editor = app.state().state.editor.as_ref().unwrap();
        assert!(matches!(&editor.params, EditorParams::Loft(l)
            if l.profiles.len() == 3 && l.shape == soliddraft_kernel_api::LoftShape::Smooth));
        assert!(editor.error.is_none(), "three-section Smooth preview: {:?}", editor.error);
        assert!(editor.preview.result.is_some());
        assert_eq!(app.state().state.doc.features.len(), 3);
        app.get_by_label("OK").click();
        ux_tools::settle(app);
        ux_tools::rebuilt(app);
        assert!(app.state().state.editor.is_none());
        assert!(matches!(&app.state().state.doc.features[3].kind, FeatureKind::Loft(l)
            if l.profiles.len() == 3 && l.shape == soliddraft_kernel_api::LoftShape::Smooth));
        let measured = volume(&app.state().state.doc);
        // The natural cubic bulge averages 5/8 of its peak. Ruled gives 25.5 and omitting Sketch2 gives 24.
        let expected = 24.0 + 6.0 * 2.0 * 0.25 * (5.0 / 8.0);
        assert!((measured - expected).abs() < 1e-7, "{measured}");
        assert!(!app.state().state.doc.proof_state().failed);
        case.note(format!("three-section Smooth volume {measured} mm3"));
        case.frame(app, "committed");
    });
}
