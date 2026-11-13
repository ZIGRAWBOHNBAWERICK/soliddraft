//! Edit Suite forms, pane access and sketch completion through the real app.

#[path = "support/ribbon.rs"]
mod ribbon;
#[path = "support/ux_tools.rs"]
mod ux_tools;

use egui::{Key, Rect, vec2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::{
    document::{FeatureKind, PlaneKind, Reference, Selection},
    settings::FrameLayout,
    state::{AppAction, EditorParams},
};
use ux_tools::{Case, Look, STANDARD, launch, settle};

fn bar(app: &App) -> Rect {
    egui::PanelState::load(&app.ctx, egui::Id::new("options_bar")).unwrap().outer_rect
}

fn suite(app: &mut App) {
    app.state_mut().state.frame_layout = FrameLayout::EditSuite;
    settle(app);
}

fn click_bar(app: &mut App, label: &str) {
    let rect = ribbon::control(app, label, bar(app)).rect;
    ribbon::click_at(app, rect.center());
    settle(app);
}

fn more(app: &mut App) {
    let (name, _) = ribbon::buttons_in(app, bar(app)).into_iter().find(|(s, _)| s.starts_with("More (")).unwrap();
    click_bar(app, &name);
}

fn click(app: &mut App, label: &str) {
    let rect = app.get_by_label(label).rect();
    ribbon::click_at(app, rect.center());
    settle(app);
}

#[test]
fn feature_fields_and_finish_actions_stay_on_the_bar_and_spill_into_the_sheet() {
    let mut case = Case::new("feature-forms");
    for (id, sketch, sheet) in [
        ("extrude", false, false),
        ("revolve", true, false),
        ("sweep", true, true),
        ("loft", true, true),
        ("cut", true, false),
        ("fillet", false, false),
        ("chamfer", false, false),
        ("hole", false, true),
        ("linear_pattern", false, true),
        ("circular_pattern", false, true),
        ("reference_plane", false, false),
        ("mirror", false, false),
        ("draft", false, false),
        ("shell", false, false),
        ("offset", false, false),
        ("thicken", false, false),
        ("lattice", false, true),
        ("gyroid", false, true),
    ] {
        let mut doc = if sketch { ribbon::sketch() } else { ribbon::plate() };
        if id == "cut" {
            doc.apply(soliddraft_app::document::Command::AddExtrude {
                params: soliddraft_app::document::ExtrudeParams::default(),
                via_assistant: false,
            })
            .unwrap();
        }
        let mut app = launch(doc, STANDARD);
        suite(&mut app);
        case.run(&mut app, |case, app| {
            if sketch {
                let id = app.state().state.doc.features[0].id;
                app.state_mut().state.doc.selection = Some(Selection::Feature(id));
            }
            ribbon::run_command(app, id);
            settle(app);
            assert!(app.state().state.editor.is_some(), "{id} opens a form");
            let title = app.state().state.editor.as_ref().unwrap().title();
            assert!(app.query_all_by_label(&format!("Active tool: {title}")).next().is_some());
            for label in ["OK", "Cancel"] {
                let button = ribbon::control(app, label, bar(app));
                assert!(bar(app).contains_rect(button.rect), "{id}: {label} stays on the bar");
            }
            assert!(!ribbon::painted(app).iter().any(|p| p.text == "Properties"));
            let has_sheet = ribbon::buttons_in(app, bar(app)).iter().any(|(s, _)| s.starts_with("More ("));
            assert_eq!(has_sheet, sheet, "{id}: only extra fields need a sheet at 1440 px");
            case.frame(app, id);
            if sheet {
                more(app);
                assert!(app.state().state.form_bar.expanded);
                assert!(bar(app).contains_rect(app.get_by_label("OK").rect()));
                case.frame(app, &format!("{id}-sheet"));
            }
            let before = ribbon::fingerprint(app);
            click_bar(app, "Cancel");
            assert!(app.state().state.editor.is_none());
            assert_eq!(ribbon::fingerprint(app).features, before.features);
        });
    }
}

#[test]
fn typed_feature_edits_commit_once_and_refused_sizes_survive_resizing() {
    let mut case = Case::new("feature-form-edits");
    let mut app = launch(ribbon::plate(), STANDARD);
    suite(&mut app);
    case.run(&mut app, |case, app| {
        let feature = app.state().state.doc.features[0].id;
        app.state_mut().state.push(AppAction::EditFeature(feature));
        settle(app);
        let before = ribbon::fingerprint(app);
        ribbon::type_field(app, "Depth value", "24");
        settle(app);
        if app.state().state.editor.is_some() {
            click_bar(app, "OK");
        }
        assert!(app.state().state.editor.is_none());
        assert!(matches!(&app.state().state.doc.features[0].kind, FeatureKind::Extrude(p) if p.depth == 24.0));
        assert_eq!(ribbon::fingerprint(app).undo_depth, before.undo_depth + 1);
        ribbon::run_command(app, "undo");
        settle(app);
        assert!(matches!(&app.state().state.doc.features[0].kind, FeatureKind::Extrude(p) if p.depth == 20.0));

        ribbon::run_command(app, "fillet");
        settle(app);
        let at = app.get_by_label("Radius value").rect();
        ribbon::click_at(app, at.center());
        app.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
        app.get_by_label("Radius value").type_text("-2");
        settle(app);
        assert!(app.state().state.editor.as_ref().unwrap().feedback.size.is_some());
        assert!(
            matches!(&app.state().state.editor.as_ref().unwrap().params, EditorParams::Fillet(p) if p.radius > 0.0)
        );
        case.frame(app, "refused-radius");
        app.set_size(vec2(640.0, 600.0));
        settle(app);
        more(app);
        assert!(app.get_by_label("Radius value").value().is_some_and(|v| v.contains("-2")));
        assert!(bar(app).contains_rect(app.get_by_label("OK").rect()));
        case.frame(app, "refused-radius-small");
        click_bar(app, "Cancel");
    });
}

#[test]
fn hole_positions_and_blind_depth_are_in_the_sheet_without_moving_ok() {
    let mut case = Case::new("feature-form-hole");
    let mut app = launch(ribbon::plate(), STANDARD);
    suite(&mut app);
    case.run(&mut app, |case, app| {
        ribbon::run_command(app, "hole");
        settle(app);
        more(app);
        click(app, "Add point");
        assert!(
            matches!(&app.state().state.editor.as_ref().unwrap().params, EditorParams::Hole(p) if p.points.len() == 2)
        );
        assert!(bar(app).contains_rect(app.get_by_label("OK").rect()));
        case.frame(app, "positions");
        click_bar(app, "Cancel");
        assert!(app.state().state.editor.is_none());
    });
}

#[test]
fn tree_and_values_are_one_optional_pane_and_sketch_closes_from_the_bar() {
    let mut case = Case::new("edit-suite-panes-sketch");
    let mut app = launch(ribbon::blank(), Look { size: vec2(960.0, 600.0), pixels_per_point: 2.0, ..STANDARD });
    suite(&mut app);
    case.run(&mut app, |case, app| {
        assert!(!app.state().state.tree_shown(), "part starts with the pane hidden");
        assert!(!ribbon::painted(app).iter().any(|p| p.text == "Properties"));
        ribbon::run_command(app, "configuration_grid");
        settle(app);
        assert!(app.state().state.tree_shown());
        assert!(app.query_all_by_label("New global name").next().is_some(), "Values content is active");
        assert!(!ribbon::painted(app).iter().any(|p| p.text == "Properties"));
        case.frame(app, "values");
        app.key_press(Key::F9);
        settle(app);
        assert!(!app.state().state.tree_shown());
        app.state_mut().state.doc.selection = Some(Selection::Reference(Reference::Plane(PlaneKind::Top)));
        ribbon::run_command(app, "sketch");
        settle(app);
        assert!(app.state().state.in_sketch());
        assert!(bar(app).contains_rect(app.get_by_label("Close sketch").rect()));
        let palette = egui::PanelState::load(&app.ctx, egui::Id::new("tool_palette")).unwrap().outer_rect;
        assert!(!ribbon::painted_in(app, palette).iter().any(|p| p.text == "Exit"));
        let actions = ribbon::buttons_in(app, bar(app));
        assert!(!actions.iter().any(|(label, _)| matches!(label.as_str(), "OK" | "Cancel")));
        ribbon::run_command(app, "sketch_line");
        let canvas = app.state().state.viewport.rect;
        for at in [canvas.center() + vec2(-40.0, 40.0), canvas.center() + vec2(40.0, 40.0)] {
            assert!(canvas.contains(at));
            ribbon::click_at(app, at);
        }
        settle(app);
        assert!(app.state().state.sketch.as_ref().unwrap().sketch.drawn().count() > 0);
        let status = ribbon::painted_in(app, bar(app)).into_iter().find(|p| p.text == "Under defined").unwrap();
        assert_eq!(status.color, soliddraft_app::theme::edit_suite::UNDER_DEFINED_LABEL);
        let image = case.frame(app, "close-sketch");
        let close = app.get_by_label("Close sketch").rect();
        let ppp = app.ctx.pixels_per_point();
        assert_eq!(
            image.get_pixel(((close.left() + 6.0) * ppp) as u32, (close.center().y * ppp) as u32).0,
            soliddraft_app::theme::edit_suite::DANGER.to_array(),
            "Close sketch keeps the approved red fill on charcoal chrome"
        );
        click_bar(app, "Close sketch");
        assert!(!app.state().state.in_sketch());
        assert!(app.state().state.editor.is_none());
        assert_eq!(app.state().state.doc.features.len(), 1, "closing saves the sketch without adding a feature");
        let chip = app.get_by_label("Sketch1 in history").rect();
        assert!(ribbon::painted_in(app, chip).iter().any(|p| p.text == "(-) Sketch1"));
        assert_eq!(app.state().state.doc.features[0].name, "Sketch1", "the prefix is display only");
        case.frame(app, "closed-sketch");
        app.state_mut().state.frame_layout = FrameLayout::Classic;
        settle(app);
        assert!(ribbon::painted(app).iter().any(|p| p.text == "Properties"), "classic keeps its dock");
    });
}

#[test]
fn face_and_edge_selection_show_the_dimensions_of_every_built_feature() {
    use soliddraft_app::document::{Command, ExtrudeParams, ProfileKind};
    use soliddraft_app::viewport::dimensions::DimensionTarget;
    let mut case = Case::new("edit-suite-all-dimensions");
    let mut doc = ribbon::plate();
    let first = doc.features[0].id;
    let second = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams {
                profile: ProfileKind::Rectangle { width: 30.0, height: 20.0 },
                depth: 30.0,
                ..Default::default()
            },
            via_assistant: false,
        })
        .unwrap()
        .unwrap();
    let mut app = launch(doc, STANDARD);
    suite(&mut app);
    case.run(&mut app, |case, app| {
        app.state_mut().state.doc.selection = None;
        settle(app);
        ribbon::click_world(app, glam::Vec3::new(0.0, 0.0, 30.0));
        settle(app);
        assert!(matches!(app.state().state.doc.selection, Some(Selection::Face(_))));
        for feature in [first, second] {
            assert!(
                app.state().state.viewport.dimensions.iter().any(|d| d.target == DimensionTarget::Feature(feature)),
                "dimensions include feature {feature}"
            );
        }
        case.frame(app, "face-all-dimensions");
        let edge = app.state().state.doc.edges.iter().find(|e| (e.midpoint.z - 30.0).abs() < 1e-9).unwrap().clone();
        ribbon::click_world(app, edge.midpoint);
        settle(app);
        assert_eq!(app.state().state.doc.selection, Some(Selection::Edge(edge.id)));
        for feature in [first, second] {
            assert!(
                app.state().state.viewport.dimensions.iter().any(|d| d.target == DimensionTarget::Feature(feature))
            );
        }
        case.frame(app, "edge-all-dimensions");
    });
}

#[test]
fn plane_picker_and_repair_use_the_same_bar_and_sheet() {
    use soliddraft_app::{
        document::{RefKind, RepairItem},
        state::FeatureEditor,
    };
    let mut case = Case::new("feature-form-picks-repair");
    let mut app = launch(ribbon::blank(), STANDARD);
    suite(&mut app);
    case.run(&mut app, |case, app| {
        ribbon::run_command(app, "sketch");
        settle(app);
        assert!(matches!(app.state().state.editor.as_ref().map(|e| &e.params), Some(EditorParams::PickPlane)));
        assert!(app.query_all_by_label("OK").next().is_none());
        assert_eq!(
            app.get_all_by_role_and_label(egui::accesskit::Role::Button, "Top plane").count(),
            1,
            "plane selection has one set of controls on the options bar"
        );
        case.frame(app, "plane-picker");
        click_bar(app, "Cancel");
        app.state_mut().state.editor = Some(FeatureEditor::new(
            None,
            EditorParams::Repair {
                items: vec![
                    RepairItem { index: 0, kind: RefKind::Edge, label: "Missing edge".into() },
                    RepairItem { index: 1, kind: RefKind::Face, label: "Missing face".into() },
                ],
                chosen: vec![None, None],
                active: 0,
            },
            None,
        ));
        settle(app);
        click_bar(app, "Next");
        assert!(matches!(
            app.state().state.editor.as_ref().map(|e| &e.params),
            Some(EditorParams::Repair { active: 1, .. })
        ));
        more(app);
        case.frame(app, "repair-list");
        click_bar(app, "Cancel");
        assert!(app.state().state.editor.is_none());
    });
}

#[test]
fn assembly_and_drawing_panes_start_open_and_keep_part_visibility_separate() {
    use soliddraft_app::{assembly::AssemblyAction, drawing::DrawingAction};
    let mut case = Case::new("edit-suite-document-panes");
    let mut app = launch(ribbon::plate(), STANDARD);
    suite(&mut app);
    case.run(&mut app, |case, app| {
        assert!(!app.state().state.tree_shown());
        app.state_mut().state.push(AppAction::Assembly(AssemblyAction::New));
        settle(app);
        assert!(app.state().state.assembly.is_active());
        assert!(app.state().state.tree_shown());
        assert!(!ribbon::painted(app).iter().any(|p| p.text == "Properties"));
        case.frame(app, "assembly-pane");
        let part = app.state().state.doc.name.clone();
        ribbon::document_tab(app, &part);
        settle(app);
        assert!(!app.state().state.assembly.is_active());
        assert!(!app.state().state.tree_shown(), "part keeps its hidden pane");
        app.state_mut().state.push(AppAction::Drawing(DrawingAction::MakeDrawing));
        settle(app);
        app.state_mut().state.push(AppAction::Drawing(DrawingAction::CreateDrawing));
        settle(app);
        assert!(app.state().state.drawing.is_active());
        assert!(app.state().state.tree_shown());
        assert!(!ribbon::painted(app).iter().any(|p| p.text == "Properties"));
        case.frame(app, "drawing-pane");
    });
}
