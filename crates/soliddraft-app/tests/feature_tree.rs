//! The feature tree through the app, with pointer events and software Vulkan. The first
//! icons of the left strip choose the side area's pane, and the tree carries everything
//! the History strip at the bottom once did: selection, editing, the context actions,
//! reordering, the rollback bar, proof marks and visible keyboard focus.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2, Rect, pos2, vec2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::document::{Command, Document, ExtrudeParams, FeatureStatus, PlaneKind, Selection, SketchPlane};
use soliddraft_app::panels::{Pane, Side};
use soliddraft_app::state::AppAction;
use soliddraft_app::theme::{self, Mode};

/// The side area while it is shown.
fn pane(app: &App) -> Rect {
    egui::PanelState::load(&app.ctx, Side::Tree.panel_id()).expect("the side area is laid out").outer_rect
}

/// A feature's row in the tree.
fn row(app: &App, name: &str) -> Rect {
    ribbon::control(app, name, pane(app)).rect
}

/// The one node called `label` in the side area.
fn in_pane<'a>(app: &'a App, label: &'a str) -> egui_kittest::Node<'a> {
    let side = pane(app);
    let mut found = app.query_all_by_label(label).filter(|n| side.intersects(n.rect()));
    let node = found.next().unwrap_or_else(|| panic!("no {label} in the side area"));
    assert!(found.next().is_none(), "one {label} in the side area");
    node
}

fn rollback_bar(app: &App) -> Rect {
    app.get_by_label("Rollback bar").rect()
}

fn names(app: &App) -> Vec<String> {
    app.state().state.doc.features.iter().map(|f| f.name.clone()).collect()
}

fn id_of(app: &App, name: &str) -> u64 {
    app.state().state.doc.features.iter().find(|f| f.name == name).unwrap().id
}

fn right_click(app: &mut App, pos: Pos2) {
    app.hover_at(pos);
    app.run_steps(3);
    for pressed in [true, false] {
        app.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        });
        app.step();
    }
    app.run_steps(4);
}

/// Right-clicks a tree row and runs a command from its menu.
fn menu(app: &mut App, name: &str, id: &str) {
    right_click(app, row(app, name).center());
    let label = soliddraft_app::commands::get(id).unwrap().label_in(&app.state().state);
    // The popup follows same-named buttons in the accessibility tree.
    let point =
        app.query_all_by_label(&label).last().unwrap_or_else(|| panic!("missing menu command {label}")).rect().center();
    ribbon::click_at(app, point);
    app.run_steps(8);
}

fn drag(app: &mut App, from: Pos2, to: Pos2) {
    app.hover_at(from);
    app.run_steps(2);
    app.drag_at(from);
    app.run_steps(2);
    for step in 1..=10 {
        app.hover_at(from + (to - from) * (step as f32 / 10.0));
        app.run_steps(2);
    }
    app.drop_at(to);
    app.run_steps(8);
}

fn sketches() -> Document {
    let mut doc = ribbon::sketch();
    for plane in [PlaneKind::Front, PlaneKind::Right] {
        doc.apply(Command::AddSketch { plane: SketchPlane::Reference { plane, offset: 0.0 } }).unwrap();
    }
    doc
}

#[test]
fn strip_icons_show_the_tree_or_values_and_hide_the_side_area() {
    let case = ribbon::case("tree-pane-icons");
    let mut app = ribbon::app(ribbon::plate());
    case.run(&mut app, |app| {
        let bar = ribbon::left_bar(app);
        let tree = ribbon::control(app, "Feature tree", bar);
        let history = ribbon::control(app, "History", bar);
        let values = ribbon::control(app, "Values", bar);
        let props = ribbon::control(app, "Properties", bar);
        assert!(tree.rect.bottom() <= history.rect.top(), "Tree sits above History on the rail");
        assert!(history.rect.bottom() <= values.rect.top(), "History sits above Values on the rail");
        assert!(values.rect.bottom() <= props.rect.top(), "Values sits above Props on the rail");
        assert!(app.state().state.pane_shown(Pane::Tree), "a part opens on its feature tree");
        assert!(tree.selected && !values.selected);
        assert!(
            egui::PanelState::load(&app.ctx, egui::Id::new("feature_timeline")).is_none(),
            "no legacy bottom history strip is laid out"
        );
        assert!(pane(app).contains_rect(row(app, "Boss-Extrude1")), "the tree lists the feature");
        // The pointer rests on the icon before any click: egui hides a tooltip right after one.
        assert_eq!(ribbon::tooltip(app, tree.rect), "Feature tree (F9)");
        case.frame(app, "tree");

        let with_pane = app.state().state.viewport.rect.width();
        ribbon::click_at(app, values.rect.center());
        app.run_steps(12);
        assert!(app.state().state.pane_shown(Pane::Values));
        assert!(app.state().state.pane_shown(Pane::Tree), "D1 stacks Values with Tree in the same column");
        assert!(app.query_all_by_label("New global name").next().is_some(), "the Values pane shows");
        assert!(ribbon::control(app, "Values", bar).selected);
        assert!(ribbon::control(app, "Feature tree", bar).selected);
        assert!(
            (app.state().state.viewport.rect.width() - with_pane).abs() < 1.0,
            "all stacked panes share one side area"
        );
        case.frame(app, "values");

        // Toggle off all open panes via Clear -> side area disappears completely.
        let clear = ribbon::control(app, "Clear panels", bar);
        ribbon::click_at(app, clear.rect.center());
        app.run_steps(12);
        assert!(!app.state().state.tree_shown(), "Clear hides the side area");
        assert!(!ribbon::control(app, "Values", bar).selected);
        assert!(!ribbon::control(app, "Feature tree", bar).selected);
        let wide = app.state().state.viewport.rect.width();
        assert!(wide > with_pane + 200.0);
        case.frame(app, "hidden");

        // Toggle Values on alone, verify F9 hides and restores it, and persists across reopen.
        ribbon::click_at(app, values.rect.center());
        app.run_steps(12);
        assert!(app.state().state.pane_shown(Pane::Values));
        assert!(!app.state().state.pane_shown(Pane::Tree));
        app.key_press(Key::F9);
        app.run_steps(12);
        assert!((app.state().state.viewport.rect.width() - wide).abs() < 1.0);
        app.key_press(Key::F9);
        app.run_steps(12);
        assert!(app.state().state.pane_shown(Pane::Values), "F9 brings back the active pane set");
    });
    drop(app);
    let reopened = ribbon::app(ribbon::plate());
    assert!(reopened.state().state.pane_shown(Pane::Values), "the next run keeps the chosen pane");
}

#[test]
fn absorbed_sketches_nest_under_their_feature_and_rows_select_rename_and_edit() {
    let case = ribbon::case("tree-rows");
    let mut doc = ribbon::sketch();
    let sketch = doc.features[0].id;
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(sketch), ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        let side = pane(app);
        assert!(
            !app.query_all_by_label("Sketch1").any(|n| side.intersects(n.rect())),
            "the sketch is folded into its extrude"
        );
        let extrude = row(app, "Boss-Extrude1");
        ribbon::click_at(app, pos2(extrude.left() + 10.0, extrude.center().y));
        app.run_steps(4);
        let nested = row(app, "Sketch1");
        assert!(nested.top() >= extrude.bottom() - 1.0, "the sketch nests under the extrude");
        for name in ["Sketch1", "Boss-Extrude1"] {
            ribbon::click_at(app, row(app, name).center());
            app.run_steps(4);
            assert_eq!(app.state().state.doc.selection, Some(Selection::Feature(id_of(app, name))));
        }
        case.frame(app, "absorbed-sketch");
        app.key_press(Key::F2);
        app.run_steps(4);
        assert!(app.state().state.rename.is_some(), "F2 renames in the tree");
        assert!(app.query_all_by_label("Feature name").next().is_some());
        app.key_press(Key::Escape);
        app.run_steps(4);
        assert!(app.state().state.rename.is_none());
        menu(app, "Boss-Extrude1", "edit_feature");
        let editor = app.state().state.editor.as_ref().expect("the feature editor opens");
        assert_eq!(editor.target, Some(id_of(app, "Boss-Extrude1")));
        assert!(matches!(editor.params, soliddraft_app::state::EditorParams::Extrude(_)));
        assert!(app.query_all_by_label("Depth value").next().is_some(), "the form is on the options bar");
    });
}

#[test]
fn f2_on_a_hidden_tree_brings_the_tree_forward_to_rename() {
    let case = ribbon::case("tree-f2-hidden");
    let mut app = ribbon::app(ribbon::plate());
    case.run(&mut app, |app| {
        ribbon::show_pane(app, Pane::Values);
        app.key_press(Key::F9);
        app.run_steps(12);
        assert!(!app.state().state.tree_shown());
        let feature = app.state().state.doc.features[0].id;
        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(feature))));
        app.run_steps(4);
        app.key_press(Key::F2);
        app.run_steps(8);
        assert!(app.state().state.pane_shown(Pane::Tree));
        assert!(app.query_all_by_label("Feature name").next().is_some(), "the name is typed in its row");
        app.key_press_modifiers(Modifiers::COMMAND, Key::A);
        app.event(egui::Event::Text("Plate".into()));
        app.key_press(Key::Enter);
        app.run_steps(8);
        assert_eq!(names(app), ["Plate"]);
    });
}

#[test]
fn context_actions_rename_suppress_roll_back_edit_and_delete() {
    let case = ribbon::case("tree-actions");
    let mut app = ribbon::app(sketches());
    case.run(&mut app, |app| {
        menu(app, "Sketch2", "rename");
        app.key_press_modifiers(Modifiers::COMMAND, Key::A);
        app.event(egui::Event::Text("Construction".into()));
        app.key_press(Key::Enter);
        app.run_steps(8);
        assert_eq!(names(app), ["Sketch1", "Construction", "Sketch3"]);
        menu(app, "Construction", "suppress");
        assert!(app.state().state.doc.features[1].suppressed);
        menu(app, "Construction", "suppress");
        assert!(!app.state().state.doc.features[1].suppressed);
        menu(app, "Sketch1", "rollback_here");
        assert_eq!(app.state().state.doc.rollback, Some(1));
        app.run_steps(60);
        case.frame(app, "rolled-back");
        ribbon::double_click_at(app, rollback_bar(app).center());
        app.run_steps(8);
        assert_eq!(app.state().state.doc.rollback, None, "a double-click restores every feature");
        menu(app, "Sketch1", "edit_feature");
        assert_eq!(app.state().state.sketch.as_ref().unwrap().feature, app.state().state.doc.features[0].id);
        let later = ribbon::painted_in(app, row(app, "Construction"))
            .into_iter()
            .find(|p| p.text == "Construction")
            .expect("the later row is painted");
        assert_eq!(later.color, theme::muted(), "features after the open sketch read dimmed");
        app.state_mut().state.push(AppAction::ExitSketch);
        app.run_steps(8);
        menu(app, "Sketch3", "delete_feature");
        assert_eq!(names(app), ["Sketch1", "Construction"]);
        case.frame(app, "after-actions");
    });
}

#[test]
fn row_and_rollback_bar_drags_change_order_and_rollback() {
    let case = ribbon::case("tree-drag");
    let mut app = ribbon::app(sketches());
    case.run(&mut app, |app| {
        let from = row(app, "Sketch3").center();
        let to = row(app, "Sketch1").center();
        drag(app, from, to);
        assert_eq!(names(app), ["Sketch3", "Sketch1", "Sketch2"]);
        // Between Sketch3 and Sketch1: only Sketch3 builds.
        let (first, second) = (row(app, "Sketch3"), row(app, "Sketch1"));
        drag(app, rollback_bar(app).center(), pos2(first.center().x, (first.center().y + second.center().y) / 2.0));
        assert_eq!(app.state().state.doc.rollback, Some(1));
        case.frame(app, "rolled-back");
        let top = row(app, "Sketch3");
        drag(app, rollback_bar(app).center(), pos2(top.center().x, top.top() + 2.0));
        assert_eq!(app.state().state.doc.rollback, Some(0));
        let last = row(app, "Sketch2");
        drag(app, rollback_bar(app).center(), pos2(last.center().x, last.bottom() + 6.0));
        assert_eq!(app.state().state.doc.rollback, None);
    });
}

#[test]
fn a_nested_sketch_drags_out_of_its_feature_to_a_legal_place() {
    use soliddraft_app::document::CutParams;
    let case = ribbon::case("tree-nested-drag");
    let mut doc = ribbon::sketch();
    let sketch = doc.features[0].id;
    let extrude = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams { sketch: Some(sketch), ..Default::default() },
            via_assistant: false,
        })
        .unwrap()
        .unwrap();
    let hole = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut circle = soliddraft_sketch::Sketch::new();
    circle.circle_at(soliddraft_sketch::P2::new(10.0, 10.0), 8.0).unwrap();
    doc.apply(Command::EditSketch { id: hole, sketch: circle }).unwrap();
    let cut = doc
        .apply(Command::AddCut {
            params: CutParams { sketch: hole, depth: 20.0, through_all: true, flip: true, up_to: None, draft: None },
        })
        .unwrap()
        .unwrap();
    let order = |app: &App| app.state().state.doc.features.iter().map(|f| f.id).collect::<Vec<_>>();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        assert_eq!(order(app), [sketch, extrude, hole, cut]);
        let name = |app: &App, id| app.state().state.doc.feature(id).unwrap().name.clone();
        let (cut_name, hole_name, extrude_name) = (name(app, cut), name(app, hole), name(app, extrude));
        let cut_row = row(app, &cut_name);
        ribbon::click_at(app, pos2(cut_row.left() + 10.0, cut_row.center().y));
        app.run_steps(4);
        let nested = row(app, &hole_name);
        assert!(nested.top() >= cut_row.bottom() - 1.0, "the sketch nests under its cut");
        case.frame(app, "nested");
        // The circle sketch reads nothing, so it may leave its cut for the top of the part.
        drag(app, nested.center(), row(app, &extrude_name).center());
        assert_eq!(order(app), [hole, sketch, extrude, cut], "the sketch moved above the extrude's block");
        assert!(
            app.state()
                .state
                .doc
                .features
                .iter()
                .all(|f| !matches!(f.status, FeatureStatus::Error(_) | FeatureStatus::NeedsRepair(_))),
            "every feature still builds"
        );
        case.frame(app, "moved");
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        app.run_steps(8);
        assert_eq!(order(app), [sketch, extrude, hole, cut], "undo restores the order");
    });
}

#[test]
fn a_sketch_between_a_feature_and_its_sketch_drops_below_the_feature() {
    let case = ribbon::case("tree-interleaved-drag");
    let mut doc = ribbon::sketch();
    let profile = doc.features[0].id;
    let loose = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Front, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let extrude = doc
        .apply(Command::AddExtrude {
            params: ExtrudeParams { sketch: Some(profile), ..Default::default() },
            via_assistant: false,
        })
        .unwrap()
        .unwrap();
    let order = |app: &App| app.state().state.doc.features.iter().map(|f| f.id).collect::<Vec<_>>();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        assert_eq!(order(app), [profile, loose, extrude]);
        let name = |app: &App, id| app.state().state.doc.feature(id).unwrap().name.clone();
        let (loose_name, extrude_name) = (name(app, loose), name(app, extrude));
        let (from, to) = (row(app, &loose_name), row(app, &extrude_name));
        assert!(from.bottom() <= to.top() + 1.0, "the loose sketch's row comes before the extrude's");
        // The loose sketch sits inside the extrude's block in the feature list, yet nothing
        // ties it there: dropped on the extrude, it moves below it.
        drag(app, from.center(), to.center());
        assert_eq!(order(app), [profile, extrude, loose]);
        case.frame(app, "moved");
        // A sketch dropped on its own feature's row stays where it is.
        let to = row(app, &extrude_name);
        ribbon::click_at(app, pos2(to.left() + 10.0, to.center().y));
        app.run_steps(4);
        let nested = row(app, &name(app, profile));
        drag(app, nested.center(), row(app, &extrude_name).center());
        assert_eq!(order(app), [profile, extrude, loose], "a drop on its own feature changes nothing");
    });
}

#[test]
fn rows_mark_only_unfinished_features_with_their_reason() {
    let case = ribbon::case("tree-proof");
    let mut app = ribbon::app(ribbon::plate());
    ribbon::until(&mut app, "the plate proof", |app| !app.state().state.doc.proof_pending());
    app.run_steps(8);
    case.run(&mut app, |app| {
        // A built feature has no mark.
        assert!(app.query_all_by_label_contains("Boss-Extrude1 status:").next().is_none());
        let id = app.state().state.doc.features[0].id;
        app.state_mut().state.push(AppAction::EditFeature(id));
        app.run_steps(8);
        let badge = app.get_by_label("Boss-Extrude1 status: Working").rect();
        let feature = row(app, "Boss-Extrude1");
        assert!(feature.contains_rect(badge), "the mark sits in the feature's row");
        let summary =
            ribbon::painted_in(app, feature).into_iter().filter(|p| p.text != "Boss-Extrude1").collect::<Vec<_>>();
        assert!(summary.iter().all(|p| p.rect.right() < badge.left()), "the mark sits right of the summary");
        app.hover_at(badge.center());
        app.run_steps(90);
        let text = ribbon::painted(app).into_iter().map(|p| p.text).collect::<Vec<_>>().join("\n");
        assert!(text.contains("Working: Finish or cancel this feature edit."), "hover gives the reason: {text}");
        case.frame(app, "working");
        app.key_press(Key::Escape);
        app.run_steps(8);
        assert!(app.query_all_by_label_contains("Boss-Extrude1 status:").next().is_none());
    });
}

#[test]
fn keyboard_focus_is_visible_and_distinct_from_selection() {
    let case = ribbon::case("tree-keyboard-focus");
    for mode in Mode::ALL {
        let mut app = ribbon::app(ribbon::plate());
        theme::set_mode(&app.ctx, mode);
        app.run_steps(8);
        let selected = row(&app, "Boss-Extrude1").center();
        ribbon::click_at(&mut app, selected);
        let view = app.state().state.viewport.rect;
        app.hover_at(view.center());
        app.run_steps(8);
        case.run(&mut app, |app| {
            for (label, name) in
                [("Boss-Extrude1", "selected-row"), ("Rollback bar", "rollback"), ("Boss-Extrude1", "unselected-row")]
            {
                if name == "unselected-row" {
                    app.state_mut().state.push(AppAction::Select(None));
                    app.run_steps(4);
                }
                in_pane(app, label).focus();
                app.run_steps(2);
                app.key_press(Key::Tab);
                app.run_steps(2);
                assert!(!in_pane(app, label).is_focused(), "Tab leaves {name}");
                let rect = in_pane(app, label).rect();
                let selection = app.state().state.doc.selection;
                let before = app.render().expect("the unfocused tree renders");
                app.key_press_modifiers(Modifiers::SHIFT, Key::Tab);
                app.run_steps(2);
                assert!(in_pane(app, label).is_focused(), "Shift+Tab returns to {name}");
                assert_eq!(app.state().state.doc.selection, selection, "focus does not select {name}");
                let focused = app.render().expect("the focused tree renders");
                focused.save(case.path(&format!("{}-{name}-focused.png", mode.label().to_lowercase()))).unwrap();
                // Rows ring their inset fill; the bar rings its own rect, left of its line.
                let at = if name == "rollback" {
                    pos2(rect.left() + 1.0, rect.center().y)
                } else {
                    pos2(rect.left() + 5.0, rect.center().y)
                };
                let old = before.get_pixel(at.x as u32, at.y as u32).0;
                let new = focused.get_pixel(at.x as u32, at.y as u32).0;
                let luminance = |pixel: [u8; 4]| -> f64 {
                    pixel[..3]
                        .iter()
                        .zip([0.2126, 0.7152, 0.0722])
                        .map(|(c, w)| {
                            let v = f64::from(*c) / 255.0;
                            w * if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
                        })
                        .sum()
                };
                let (a, b) = (luminance(old), luminance(new));
                let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(contrast >= 3.0, "{} {name} has no distinct focus paint: {contrast:.2}:1", mode.label());
                if name.ends_with("row") {
                    assert_eq!(ribbon::control(app, label, pane(app)).selected, name == "selected-row");
                }
            }
        });
    }
}

#[test]
fn long_trees_scroll_to_the_selection_and_an_empty_tree_has_a_hint() {
    let case = ribbon::case("tree-scroll");
    let mut app = ribbon::app(ribbon::blank());
    assert!(ribbon::painted_in(&app, pane(&app)).iter().any(|p| p.text == "No features yet"));
    for _ in 0..30 {
        app.state_mut()
            .state
            .doc
            .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
            .unwrap();
    }
    app.set_size(vec2(1280.0, 800.0));
    app.run_steps(8);
    case.run(&mut app, |app| {
        let last = id_of(app, "Sketch30");
        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(last))));
        app.run_steps(30);
        assert!(pane(app).contains_rect(row(app, "Sketch30")), "the selected feature scrolls into view");
        assert!(app.state().state.viewport.rect.width() >= 300.0);
        case.frame(app, "1280-scrolled");
    });
}

/// A second click on the selected row renames it, like a file manager, but only a slow
/// one. A double-click on a row that was already selected used to start the rename on its
/// first click and put the second click into the text field, so the dimensions never
/// showed.
#[test]
fn a_double_click_on_the_selected_row_shows_its_dimensions_and_never_renames() {
    let case = ribbon::case("tree-double-click-selected");
    let mut app = ribbon::app(ribbon::plate());
    ribbon::until(&mut app, "the plate proof", |app| !app.state().state.doc.proof_pending());
    app.run_steps(8);
    case.run(&mut app, |app| {
        let id = id_of(app, "Boss-Extrude1");
        let at = row(app, "Boss-Extrude1").center();
        ribbon::click_at(app, at);
        // Half a second: the next click is no double-click.
        app.run_steps(30);
        assert_eq!(app.state().state.doc.selection, Some(Selection::Feature(id)));

        ribbon::click_at(app, at);
        assert!(app.state().state.rename.is_none(), "the rename waits out the double-click time");
        app.run_steps(30);
        assert!(app.state().state.rename.is_some(), "a slow second click renames the row");
        case.frame(app, "slow-second-click-renames");
        app.key_press(Key::Escape);
        app.run_steps(30);
        assert!(app.state().state.rename.is_none());

        // Escape also cleared the selection. Select the row again without the pointer.
        app.state_mut().state.push(AppAction::Select(Some(Selection::Feature(id))));
        app.run_steps(30);
        assert_eq!(app.state().state.doc.selection, Some(Selection::Feature(id)));
        ribbon::double_click_at(app, at);
        app.run_steps(30);
        assert!(app.state().state.rename.is_none(), "a double-click on the selected row does not rename it");
        assert!(app.query_all_by_label("Feature name").next().is_none(), "no rename field is open");
        assert_eq!(
            app.state().state.doc.selection,
            Some(Selection::Feature(id)),
            "the double-click shows the feature's own dimensions"
        );
        assert_eq!(app.state().state.viewport.dimensions_of, Some(id));
        case.frame(app, "double-click-shows-dimensions");
    });
}
