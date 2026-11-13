//! Dimension placement and reference annotations through native sketch input.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2, pos2, vec2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::{
    document::{Command, FeatureId, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, DistanceKind, EntityKind, P2, PointRef, Sketch, SolveStatus, solve},
    sketcher::Frame,
    state::{AppAction, SketchSel, SketchSession},
};
use std::sync::Mutex;

static SERIAL: Mutex<()> = Mutex::new(());

fn dir() -> std::path::PathBuf {
    std::env::var_os("SOLIDDRAFT_DRIVEN_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-diagnostics-{}", std::process::id())))
}

fn launch(sketch: Sketch) -> (App, FeatureId) {
    launch_at(ribbon::blank(), sketch, 0.0)
}

fn launch_at(mut doc: soliddraft_app::document::Document, mut sketch: Sketch, offset: f64) -> (App, FeatureId) {
    std::fs::create_dir_all(dir().join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir().join("config")));
    solve::solve(&mut sketch, None, true);
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    (app, id)
}

fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}

fn control(app: &mut App, label: &str) {
    ribbon::click_option(app, label);
}

fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap()
}

fn frame(app: &mut App, name: &str) {
    app.render().unwrap().save(dir().join(format!("{name}.png"))).unwrap();
}

fn circle() -> (Sketch, u32) {
    let mut sketch = Sketch::new();
    let entity = sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 5.0 }, false);
    sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(entity, 0), b: PointRef::new(0, 0) });
    let dim =
        sketch.constrain(ConstraintKind::Radius { entity, value: 10.0, diameter: true, label: P2::new(9.0, 7.0) });
    (sketch, dim)
}

fn shared_field(app: &mut App, label: &str, text: &str) {
    app.get_by_label(label).focus();
    app.run_steps(2);
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.get_by_label(label).type_text(text);
    app.run_steps(3);
}

#[test]
fn linked_dimensions_share_edits_unlink_cancel_undo_and_reopen() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let mut ids = Vec::new();
    for (x, radius, y) in [(-12.0, 5.0, -9.0), (12.0, 7.0, -11.0)] {
        let entity = sketch.add(EntityKind::Circle { center: P2::new(x, 0.0), radius }, false);
        ids.push(sketch.constrain(ConstraintKind::Radius {
            entity,
            value: radius * 2.0,
            diameter: true,
            label: P2::new(x, y),
        }));
    }
    let (mut app, feature) = launch(sketch);
    app.run_steps(60);
    let read = |app: &App| {
        ids.iter().map(|id| session(app).sketch.constraint(*id).unwrap().value().unwrap()).collect::<Vec<_>>()
    };
    let label = |app: &App, id| screen(app, session(app).sketch.constraint(id).unwrap().label().unwrap());
    let at = label(&app, ids[0]);
    ribbon::click_at(&mut app, at);
    let at = label(&app, ids[1]);
    app.hover_at(at);
    app.run_steps(2);
    app.event(egui::Event::ModifiersChanged(Modifiers::CTRL));
    app.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::CTRL,
    });
    app.step();
    app.event(egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::CTRL,
    });
    app.step();
    app.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    app.run_steps(3);
    assert_eq!(session(&app).selection, ids.iter().copied().map(SketchSel::Constraint).collect::<Vec<_>>());
    let history = app.state().state.doc.model().undo_depth();
    control(&mut app, "Link values");
    assert!(
        ribbon::painted(&app)
            .iter()
            .any(|p| p.text.contains("Default bindings and values") && p.rect.contains_rect(p.full))
    );
    shared_field(&mut app, "Shared value name", "Span");
    frame(&mut app, "link-values-new-group");
    control(&mut app, "Link selected");
    app.run_steps(10);
    assert!(session(&app).link_values.is_none());
    assert_eq!(read(&app), [10.0, 10.0]);
    assert_eq!(app.state().state.doc.model().undo_depth(), history + 1);
    let linked = session(&app).sketch.clone();
    app.run_steps(30);
    let at = label(&app, ids[1]);
    ribbon::double_click_at(&mut app, at);
    app.run_steps(6);
    assert_eq!(app.get_by_label("Shared value name").value().as_deref(), Some("Span"));
    shared_field(&mut app, "Shared numeric value (mm)", "20");
    control(&mut app, "Link selected");
    app.run_steps(10);
    assert_eq!(read(&app), [20.0, 20.0]);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(10);
    assert_eq!(session(&app).sketch, linked);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(10);
    assert_eq!(read(&app), [20.0, 20.0]);
    app.set_size(vec2(960.0, 600.0));
    soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
    app.run_steps(60);
    let at = label(&app, ids[1]);
    ribbon::double_click_at(&mut app, at);
    app.run_steps(6);
    frame(&mut app, "link-values-member-small-dark");
    let before = (app.state().state.doc.model().part_json(), app.state().state.doc.model().undo_depth());
    control(&mut app, "Link selected");
    app.run_steps(6);
    assert!(session(&app).link_values.is_none());
    assert_eq!((app.state().state.doc.model().part_json(), app.state().state.doc.model().undo_depth()), before);
    app.run_steps(30);
    let at = label(&app, ids[1]);
    ribbon::double_click_at(&mut app, at);
    app.run_steps(6);
    app.key_press(Key::Escape);
    app.run_steps(6);
    assert!(session(&app).link_values.is_none());
    assert_eq!((app.state().state.doc.model().part_json(), app.state().state.doc.model().undo_depth()), before);
    app.run_steps(30);
    let at = label(&app, ids[1]);
    ribbon::double_click_at(&mut app, at);
    app.run_steps(6);
    shared_field(&mut app, "Shared numeric value (mm)", "not a number");
    control(&mut app, "Unlink selected");
    app.run_steps(10);
    assert!(session(&app).link_values.is_none(), "unlink discards an unused numeric draft");
    assert_eq!(read(&app), [20.0, 20.0]);
    app.run_steps(30);
    let at = label(&app, ids[0]);
    ribbon::double_click_at(&mut app, at);
    app.run_steps(6);
    shared_field(&mut app, "Shared numeric value (mm)", "16");
    control(&mut app, "Link selected");
    app.run_steps(10);
    assert_eq!(read(&app), [16.0, 20.0]);
    let saved = session(&app).sketch.clone();
    let path = dir().join("linked-dimensions.dwprt");
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(8);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(12);
    ribbon::document_tab(&mut app, "Values");
    app.run_steps(6);
    shared_field(&mut app, "New global name", "Other");
    app.key_press(Key::Enter);
    app.run_steps(4);
    shared_field(&mut app, "New global formula", "24 mm");
    app.key_press(Key::Enter);
    app.run_steps(8);
    ribbon::document_tab(&mut app, "Tree");
    app.state_mut().state.push(AppAction::EditSketch(feature));
    app.run_steps(60);
    assert_eq!(session(&app).sketch, saved);
    let at = label(&app, ids[0]);
    ribbon::double_click_at(&mut app, at);
    app.run_steps(6);
    assert_eq!(app.get_by_label("Shared value name").value().as_deref(), Some("Span"));
    assert_eq!(app.get_by_label("Shared numeric value (mm)").value().as_deref(), Some("16"));
    frame(&mut app, "link-values-reopened");
    app.key_press(Key::Escape);
    app.run_steps(30);
    let at = label(&app, ids[1]);
    ribbon::click_at(&mut app, at);
    control(&mut app, "Link values");
    let choosing = (app.state().state.doc.model().part_json(), app.state().state.doc.model().undo_depth());
    app.key_press(Key::Tab);
    app.run_steps(2);
    app.key_press(Key::Enter);
    app.run_steps(3);
    assert!(session(&app).link_values.as_ref().unwrap().existing_open);
    app.key_press(Key::ArrowDown);
    app.run_steps(3);
    assert_eq!(session(&app).link_values.as_ref().unwrap().existing_index, 1);
    app.key_press(Key::ArrowUp);
    app.run_steps(3);
    assert_eq!(session(&app).link_values.as_ref().unwrap().existing_index, 0);
    frame(&mut app, "link-values-inline-names");
    app.key_press(Key::Escape);
    app.run_steps(3);
    assert!(!session(&app).link_values.as_ref().unwrap().existing_open, "Escape closes only the list");
    assert_eq!((app.state().state.doc.model().part_json(), app.state().state.doc.model().undo_depth()), choosing);
    app.key_press(Key::Enter);
    app.run_steps(3);
    assert!(session(&app).link_values.as_ref().unwrap().existing_open);
    app.key_press(Key::ArrowDown);
    app.run_steps(3);
    app.key_press(Key::Enter);
    app.run_steps(3);
    assert_eq!(app.get_by_label("Shared value name").value().as_deref(), Some("Other"));
    assert_eq!(app.get_by_label("Shared numeric value (mm)").value().as_deref(), Some("24"));
    assert!(!session(&app).link_values.as_ref().unwrap().existing_open);
    assert_eq!((app.state().state.doc.model().part_json(), app.state().state.doc.model().undo_depth()), choosing);
    control(&mut app, "Choose existing value");
    control(&mut app, "Span");
    assert_eq!(app.get_by_label("Shared numeric value (mm)").value().as_deref(), Some("16"));
    control(&mut app, "Link selected");
    app.run_steps(10);
    assert_eq!(read(&app), [16.0, 16.0]);
    frame(&mut app, "link-values-joined-existing");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn compact_angle_literals_can_be_edited_from_a_linked_member() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let a = sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
    let b = sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 10.0) }, false);
    let dimension = sketch.constrain(ConstraintKind::Angle { a, b, value: 45.0, label: P2::new(9.0, 7.0) });
    let (mut app, feature) = launch(sketch);
    app.state_mut().state.doc.apply(Command::SetValue { name: "Bend".into(), expression: "45deg".into() }).unwrap();
    app.state_mut()
        .state
        .doc
        .apply(Command::LinkSketchValues {
            id: feature,
            constraints: vec![dimension],
            name: "Bend".into(),
            value: None,
        })
        .unwrap();
    app.run_steps(10);
    let at = screen(&app, P2::new(9.0, 7.0));
    ribbon::double_click_at(&mut app, at);
    app.run_steps(6);
    frame(&mut app, "link-values-angle-literal");
    shared_field(&mut app, "Shared numeric value (deg)", "60");
    control(&mut app, "Link selected");
    app.run_steps(8);
    assert_eq!(session(&app).sketch.constraint(dimension).unwrap().value(), Some(60.0));
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn the_values_tab_refuses_non_positive_sketch_lengths_without_mutation() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (sketch, dimension) = circle();
    let (mut app, feature) = launch(sketch);
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(12);
    ribbon::document_tab(&mut app, "Values");
    app.run_steps(6);
    let before = (app.state().state.doc.model().part_json(), app.state().state.doc.model().undo_depth());
    for text in ["-10", "0"] {
        app.get_by_label("Default Sketch1.D1").focus();
        app.run_steps(2);
        app.key_press_modifiers(Modifiers::COMMAND, Key::A);
        app.get_by_label("Default Sketch1.D1").type_text(text);
        app.run_steps(2);
        app.key_press(Key::Enter);
        app.run_steps(8);
        assert!(
            app.state()
                .state
                .values
                .error(&soliddraft_app::values_tab::Field::Cell("Sketch1.D1".into(), 0))
                .is_some_and(|e| e.contains("greater than 0"))
        );
        frame(&mut app, "negative-dimension-values-cell");
        assert_eq!(
            app.state()
                .state
                .doc
                .feature(feature)
                .unwrap()
                .sketch()
                .unwrap()
                .sketch
                .constraint(dimension)
                .unwrap()
                .value(),
            Some(10.0),
            "a negative diameter must be refused, not solved by absolute magnitude"
        );
        assert_eq!((app.state().state.doc.model().part_json(), app.state().state.doc.model().undo_depth()), before);
    }
    app.get_by_label("Default Sketch1.D1").focus();
    app.run_steps(2);
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.get_by_label("Default Sketch1.D1").type_text("12");
    app.run_steps(2);
    app.key_press(Key::Enter);
    app.run_steps(8);
    assert_eq!(
        app.state().state.doc.feature(feature).unwrap().sketch().unwrap().sketch.constraint(dimension).unwrap().value(),
        Some(12.0)
    );
    assert!(app.state().state.values.error(&soliddraft_app::values_tab::Field::Cell("Sketch1.D1".into(), 0)).is_none());
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn link_values_is_an_actionable_selected_dimension_command() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (sketch, id) = circle();
    let (mut app, _) = launch(sketch);
    let at = screen(&app, session(&app).sketch.constraint(id).unwrap().label().unwrap());
    ribbon::click_at(&mut app, at);
    assert_eq!(session(&app).selection, [SketchSel::Constraint(id)]);
    frame(&mut app, "link-values-selected-baseline");
    control(&mut app, "Link values");
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "Shared value name"));
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn accepting_a_measured_dimension_without_typing_keeps_its_full_precision() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 6.0) }, false);
    let (mut app, feature) = launch(sketch);
    let before = session(&app).sketch.entities.clone();
    app.key_press(Key::D);
    app.run_steps(6);
    for at in [P2::new(5.0, 3.0), P2::new(3.0, 4.0)] {
        let at = screen(&app, at);
        ribbon::click_at(&mut app, at);
    }
    let id = session(&app).sketch.constraints.iter().find(|c| c.is_dimension()).unwrap().id;
    let value = session(&app).sketch.constraint(id).unwrap().value();
    assert_eq!(session(&app).sketch.entities, before);
    assert!(session(&app).dim_edit.is_some());
    frame(&mut app, "dimension-precision-before-accept");
    let history = app.state().state.doc.model().undo_depth();
    app.key_press(Key::Enter);
    app.run_steps(10);
    assert!(session(&app).dim_edit.is_none());
    assert_eq!(
        session(&app).sketch.constraint(id).unwrap().value(),
        value,
        "Enter without typing must not round the stored dimension"
    );
    assert_eq!(session(&app).sketch.entities, before);
    assert_eq!(app.state().state.doc.model().undo_depth(), history);
    app.key_press(Key::Escape);
    app.run_steps(30);
    for command in [false, true] {
        if command {
            app.state_mut().state.push(AppAction::EditDimension(
                soliddraft_app::viewport::dimensions::DimensionTarget::Sketch { sketch: feature, constraint: id },
            ));
            app.run_steps(6);
        } else {
            let at = screen(&app, session(&app).sketch.constraint(id).unwrap().label().unwrap());
            ribbon::click_at(&mut app, at);
            ribbon::click_at(&mut app, at);
        }
        let edit = session(&app).dim_edit.as_ref().expect("existing dimension opens for editing");
        assert_eq!(edit.text, soliddraft_app::sketch::fmt_value(value.unwrap()));
        if command {
            let away = app.state().state.viewport.rect.center() + vec2(0.0, -100.0);
            ribbon::click_at(&mut app, away);
        } else {
            let ok = ribbon::painted(&app).into_iter().find(|p| p.text == "OK").unwrap().rect.center();
            ribbon::click_at(&mut app, ok);
        }
        app.run_steps(10);
        assert!(session(&app).dim_edit.is_none());
        assert_eq!(app.state().state.doc.model().undo_depth(), history);
        assert_eq!(session(&app).sketch.constraint(id).unwrap().value(), value);
        assert_eq!(session(&app).sketch.entities, before);
    }
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn smart_distance_placement_selects_axis_or_aligned_without_moving_geometry() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for (line, swap) in [(false, false), (false, true), (true, false)] {
        for (kind, label, value) in [
            (DistanceKind::Aligned, P2::new(-2.0, 1.0), 136.0_f64.sqrt()),
            (DistanceKind::Horizontal, P2::new(0.0, -5.0), 10.0),
            (DistanceKind::Vertical, P2::new(8.0, 0.0), 6.0),
        ] {
            let mut sketch = Sketch::new();
            let (a, b) = (P2::new(-5.0, -3.0), P2::new(5.0, 3.0));
            if line {
                sketch.add(EntityKind::Line { a, b }, false);
            } else {
                sketch.add(EntityKind::Point { p: a }, false);
                sketch.add(EntityKind::Point { p: b }, false);
            }
            let (mut app, _) = launch(sketch);
            if line && kind == DistanceKind::Vertical {
                app.set_size(vec2(960.0, 600.0));
                soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
                app.run_steps(60);
            }
            let before = session(&app).sketch.entities.clone();
            app.key_press(Key::D);
            app.run_steps(6);
            let picks = if line {
                vec![a.lerp(b, 0.6)]
            } else if swap {
                vec![b, a]
            } else {
                vec![a, b]
            };
            for at in picks {
                let at = screen(&app, at);
                ribbon::click_at(&mut app, at);
            }
            let at = screen(&app, label);
            ribbon::click_at(&mut app, at);
            let dimension =
                session(&app).sketch.constraints.iter().find(|c| c.is_dimension()).cloned().expect("distance placed");
            let ConstraintKind::Distance { kind: actual, .. } = dimension.kind else {
                panic!("distance family");
            };
            assert_eq!(actual, kind);
            assert!((dimension.value().unwrap() - value).abs() < 1e-9);
            assert_eq!(session(&app).sketch.entities, before);
            assert_eq!(session(&app).dim_edit.as_ref().unwrap().constraint, dimension.id);
            frame(&mut app, &format!("distance-{kind:?}-line-{line}-swap-{swap}"));
            drop(app);
            soliddraft_app::file::set_config_dir(None);
        }
    }
}

#[test]
fn smart_radius_defaults_preserve_arc_and_circle_geometry() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for arc in [false, true] {
        let mut sketch = Sketch::new();
        sketch.add(
            if arc {
                EntityKind::Arc { center: P2::ZERO, start: P2::new(5.0, 0.0), end: P2::new(0.0, 5.0) }
            } else {
                EntityKind::Circle { center: P2::ZERO, radius: 5.0 }
            },
            false,
        );
        let (mut app, _) = launch(sketch);
        let before = session(&app).sketch.entities.clone();
        app.key_press(Key::D);
        app.run_steps(6);
        let picks = if arc {
            [P2::new(5.0 / 2.0_f64.sqrt(), 5.0 / 2.0_f64.sqrt()), P2::new(6.0, 2.0)]
        } else {
            [P2::new(-5.0, 0.0), P2::new(7.0, 3.0)]
        };
        for at in picks {
            let at = screen(&app, at);
            ribbon::click_at(&mut app, at);
        }
        let dimension = session(&app)
            .sketch
            .constraints
            .iter()
            .find(|c| c.is_dimension())
            .cloned()
            .expect("circular dimension placed");
        let ConstraintKind::Radius { diameter, .. } = dimension.kind else {
            panic!("circular dimension family");
        };
        assert_eq!(diameter, !arc);
        assert_eq!(dimension.value(), Some(if arc { 5.0 } else { 10.0 }));
        assert_eq!(session(&app).sketch.entities, before);
        assert_eq!(session(&app).dim_edit.as_ref().unwrap().constraint, dimension.id);
        frame(&mut app, &format!("circular-dimension-arc-{arc}"));
        drop(app);
        soliddraft_app::file::set_config_dir(None);
    }
}

#[test]
fn the_values_tab_drives_a_sketch_dimension_from_a_global_and_reopens_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (sketch, dimension) = circle();
    let (mut app, feature) = launch(sketch);
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(12);
    ribbon::document_tab(&mut app, "Values");
    app.run_steps(6);
    for (label, text) in [
        ("New global name", "Span"),
        ("New global formula", "12"),
        ("Default Sketch1.D1", "=Span"),
        ("Default Span", "18"),
    ] {
        app.get_by_label(label).focus();
        app.run_steps(2);
        app.key_press_modifiers(Modifiers::COMMAND, Key::A);
        app.get_by_label(label).type_text(text);
        app.run_steps(2);
        app.key_press(Key::Enter);
        app.run_steps(8);
    }
    let s = app.state().state.doc.feature(feature).unwrap().sketch().unwrap();
    assert_eq!(s.sketch.constraint(dimension).unwrap().value(), Some(18.0));
    assert!(matches!(s.sketch.entities[1].kind, EntityKind::Circle { radius: 9.0, .. }));
    assert_eq!(app.get_by_label("Default Sketch1.D1").value().as_deref(), Some("=Span"));
    frame(&mut app, "global-sketch-dimension");
    let path = dir().join("global-sketch-dimension.dwprt");
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(8);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(12);
    ribbon::document_tab(&mut app, "Values");
    app.run_steps(6);
    assert_eq!(app.get_by_label("Default Sketch1.D1").value().as_deref(), Some("=Span"));
    assert_eq!(app.get_by_label("Default Span").value().as_deref(), Some("18"));
    let s = app.state().state.doc.feature(feature).unwrap().sketch().unwrap();
    assert_eq!(s.sketch.constraint(dimension).unwrap().value(), Some(18.0));
    assert!(matches!(s.sketch.entities[1].kind, EntityKind::Circle { radius: 9.0, .. }));
    frame(&mut app, "global-sketch-dimension-reopened");
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn smart_angle_defaults_preserve_geometry_in_every_quadrant_and_pick_order() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for degrees in [315.0_f64, 45.0, 135.0, 225.0] {
        for swap in [false, true] {
            let mut sketch = Sketch::new();
            let a = sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
            let direction = P2::from_angle(degrees.to_radians());
            let b = sketch.add(EntityKind::Line { a: P2::ZERO, b: direction.scale(10.0) }, false);
            let expected = ConstraintKind::Angle {
                a: if swap { b } else { a },
                b: if swap { a } else { b },
                value: 0.0,
                label: P2::ZERO,
            };
            let measurement =
                soliddraft_app::sketch::Constraint { id: 0, kind: expected }.measured_value(&sketch).unwrap();
            let (mut app, _) = launch(sketch);
            let before = session(&app).sketch.entities.clone();
            app.key_press(Key::D);
            app.run_steps(6);
            let picks = if swap {
                [direction.scale(5.0), P2::new(5.0, 0.0)]
            } else {
                [P2::new(5.0, 0.0), direction.scale(5.0)]
            };
            for at in picks {
                let at = screen(&app, at);
                ribbon::click_at(&mut app, at);
            }
            let label = app.state().state.viewport.rect.center() + vec2(0.0, 80.0);
            ribbon::click_at(&mut app, label);
            let dimension = session(&app)
                .sketch
                .constraints
                .iter()
                .find(|c| matches!(c.kind, ConstraintKind::Angle { .. }))
                .cloned()
                .expect("angle dimension placed");
            frame(&mut app, &format!("angle-{degrees}-swap-{swap}"));
            assert!(
                (dimension.value().unwrap() - measurement).abs() < 1e-9,
                "{degrees}, swap {swap}: default must measure existing geometry"
            );
            assert_eq!(
                session(&app).sketch.entities,
                before,
                "{degrees}, swap {swap}: placing a measured angle must not move geometry"
            );
            drop(app);
            soliddraft_app::file::set_config_dir(None);
        }
    }
}

#[test]
fn perpendicular_defaults_use_supporting_lines_without_moving_geometry() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let p = P2::new;
    for reference in [false, true] {
        for (name, parallel, reverse, swap, point) in [
            ("point-after", false, false, false, p(20.0, 5.0)),
            ("point-before", false, false, true, p(-20.0, 5.0)),
            ("point-interior", false, false, false, p(5.0, 5.0)),
            ("parallel-a-b", true, false, false, p(20.0, 5.0)),
            ("parallel-b-a", true, false, true, p(20.0, 5.0)),
            ("parallel-reversed", true, true, false, p(20.0, 5.0)),
        ] {
            let mut sketch = Sketch::new();
            sketch.add(EntityKind::Line { a: p(0.0, 0.0), b: p(10.0, 0.0) }, false);
            if parallel {
                let (a, b) = if reverse { (p(30.0, 5.0), p(20.0, 5.0)) } else { (p(20.0, 5.0), p(30.0, 5.0)) };
                sketch.add(EntityKind::Line { a, b }, false);
            } else {
                sketch.add(EntityKind::Point { p: point }, false);
            }
            let (mut app, _) = launch(sketch);
            let before = session(&app).sketch.entities.clone();
            app.key_press(Key::D);
            app.run_steps(6);
            if reference {
                control(&mut app, "Reference dimensions");
            }
            let a = if parallel { p(25.0, 5.0) } else { point };
            let b = p(5.0, 0.0);
            let picks = if swap { [b, a] } else { [a, b] };
            for at in picks {
                let at = screen(&app, at);
                ribbon::click_at(&mut app, at);
            }
            let label = app.state().state.viewport.rect.center() + vec2(0.0, 80.0);
            ribbon::click_at(&mut app, label);
            let dimension = session(&app)
                .sketch
                .constraints
                .iter()
                .find(|c| matches!(c.geometry_kind(), ConstraintKind::PointLineDistance { .. }))
                .cloned()
                .unwrap_or_else(|| panic!("{name}, reference {reference}: perpendicular dimension placed"));
            frame(&mut app, &format!("perpendicular-{name}-{}", if reference { "reference" } else { "driving" }));
            assert_eq!(
                dimension.display_value(&session(&app).sketch),
                Some(5.0),
                "{name}: placement must measure the supporting line"
            );
            assert_eq!(dimension.is_reference(), reference);
            assert_eq!(
                session(&app).sketch.entities,
                before,
                "{name}: placing a measured value must not move geometry"
            );
            drop(app);
            soliddraft_app::file::set_config_dir(None);
        }
    }
}

#[test]
fn reference_creation_is_read_only_updates_with_driving_edits_and_survives_reopen() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (sketch, driving) = circle();
    let (mut app, id) = launch(sketch);
    let entities = session(&app).sketch.entities.clone();
    app.key_press(Key::D);
    app.run_steps(6);
    control(&mut app, "Reference dimensions");
    for p in [P2::new(-5.0, 0.0), P2::new(-9.0, -7.0)] {
        let at = screen(&app, p);
        ribbon::click_at(&mut app, at);
    }
    let reference = session(&app)
        .sketch
        .constraints
        .iter()
        .find(|c| c.is_reference())
        .expect("reference annotation created")
        .clone();
    assert!(session(&app).dim_edit.is_none());
    assert_eq!(session(&app).sketch.status, SolveStatus::FullyDefined);
    assert_eq!(session(&app).sketch.entities, entities);
    app.key_press(Key::Escape);
    app.run_steps(8);
    let at = screen(&app, reference.label().unwrap());
    app.run_steps(30);
    ribbon::double_click_at(&mut app, at);
    assert!(session(&app).dim_edit.is_none(), "a reference label must not open a numeric editor");
    assert!(session(&app).message.as_ref().is_some_and(|m| m.0.contains("read-only")));
    app.run_steps(30);
    let at = screen(&app, session(&app).sketch.constraint(driving).unwrap().label().unwrap());
    ribbon::double_click_at(&mut app, at);
    frame(&mut app, "reference-driving-editor");
    assert!(
        session(&app).dim_edit.is_some(),
        "tool {:?}, hover {:?}, selected {:?}, click {at:?}",
        session(&app).tool,
        session(&app).hover,
        session(&app).selection
    );
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.event(egui::Event::Text("20".into()));
    app.key_press(Key::Enter);
    app.run_steps(12);
    assert!(session(&app).dim_edit.is_none());
    assert_eq!(session(&app).sketch.constraint(reference.id).unwrap().display_value(&session(&app).sketch), Some(20.0));
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "(Ø20)"));
    app.set_size(vec2(960.0, 600.0));
    soliddraft_app::theme::set_mode(&app.ctx, soliddraft_app::theme::Mode::Dark);
    app.run_steps(12);
    frame(&mut app, "reference-created-small-dark");
    let before = session(&app).sketch.clone();
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(6);
    let path = dir().join("reference.dwprt");
    app.state_mut().state.push(AppAction::SaveAs(Some(path.clone())));
    app.run_steps(6);
    app.state_mut().state.push(AppAction::Open(Some(path)));
    app.run_steps(10);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(60);
    assert_eq!(session(&app).sketch, before);
    assert!(!session(&app).reference_dimensions, "the creation choice is session-local");
    let at = screen(&app, reference.label().unwrap());
    ribbon::click_at(&mut app, at);
    assert_eq!(
        session(&app).selection,
        [SketchSel::Constraint(reference.id)],
        "the reopened reference label is the delete target"
    );
    control(&mut app, "Reference dimension");
    control(&mut app, "Reference dimension");
    assert_eq!(session(&app).sketch, before);
    frame(&mut app, "reference-reopened");
    app.key_press(Key::Delete);
    app.run_steps(10);
    assert!(session(&app).sketch.constraint(reference.id).is_none());
    assert_eq!(session(&app).sketch.entities, before.entities);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(10);
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn reference_label_drag_moves_only_the_annotation_and_undo_restores_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (sketch, dim) = circle();
    let (mut app, _) = launch(sketch);
    let at = screen(&app, session(&app).sketch.constraint(dim).unwrap().label().unwrap());
    ribbon::click_at(&mut app, at);
    control(&mut app, "Reference dimension");
    let before = session(&app).sketch.clone();
    let from = screen(&app, before.constraint(dim).unwrap().label().unwrap());
    let to = from + vec2(-30.0, 25.0);
    app.hover_at(from);
    app.run_steps(4);
    app.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.step();
    for t in [0.25, 0.5, 0.75, 1.0] {
        app.hover_at(from + (to - from) * t);
        app.run_steps(3);
    }
    app.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(10);
    assert_eq!(session(&app).sketch.entities, before.entities);
    let label = session(&app).sketch.constraint(dim).unwrap().label().unwrap();
    assert_ne!(label, before.constraint(dim).unwrap().label().unwrap());
    assert!(screen(&app, label).distance(to) < 2.0);
    frame(&mut app, "reference-label-moved");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn reference_values_follow_a_live_point_drag_without_becoming_equations() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let line = sketch.add(EntityKind::Line { a: P2::ZERO, b: P2::new(10.0, 0.0) }, false);
    let id = sketch.constrain(
        ConstraintKind::Distance {
            a: PointRef::new(line, 0),
            b: PointRef::new(line, 1),
            kind: soliddraft_app::sketch::DistanceKind::Aligned,
            value: 999.0,
            label: P2::new(5.0, -5.0),
        }
        .as_reference()
        .unwrap(),
    );
    let (mut app, _) = launch(sketch);
    app.run_steps(60);
    let from = screen(&app, P2::new(10.0, 0.0));
    let to = screen(&app, P2::new(14.0, 3.0));
    app.hover_at(from);
    app.run_steps(4);
    app.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.step();
    for t in [0.25, 0.5, 0.75, 1.0] {
        app.hover_at(from + (to - from) * t);
        app.run_steps(3);
    }
    let measured = session(&app).sketch.constraint(id).unwrap().display_value(&session(&app).sketch).unwrap();

    assert!(
        (measured - 205.0_f64.sqrt()).abs() < 1e-5,
        "measurement {measured}, points {:?}, drag {:?}, from {from:?}, to {to:?}",
        session(&app).sketch.entities,
        session(&app).drag
    );
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "(14.32)"));
    assert_eq!(session(&app).sketch.constraint(id).unwrap().value(), None);
    frame(&mut app, "reference-live-point-drag");
    app.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(10);
    assert!((session(&app).sketch.point(PointRef::new(line, 1)).unwrap().dist(P2::new(14.0, 3.0))) < 1e-5);
    assert_eq!(session(&app).sketch.status, SolveStatus::UnderDefined);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}

#[test]
fn reference_mode_releases_only_the_dimension_and_keeps_its_identity() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (sketch, dim) = circle();
    let (mut app, _) = launch(sketch);
    assert_eq!(session(&app).sketch.status, SolveStatus::FullyDefined);
    let before = session(&app).sketch.clone();
    let at = screen(&app, before.constraint(dim).unwrap().label().unwrap());
    ribbon::click_at(&mut app, at);
    assert_eq!(session(&app).selection, [SketchSel::Constraint(dim)]);
    control(&mut app, "Reference dimension");
    assert_eq!(session(&app).sketch.status, SolveStatus::UnderDefined);
    assert_eq!(session(&app).sketch.entities, before.entities);
    assert_eq!(session(&app).sketch.constraint(dim).unwrap().label(), before.constraint(dim).unwrap().label());
    assert!(ribbon::painted(&app).iter().any(|p| p.text == "(Ø10)"));
    frame(&mut app, "reference-converted");
    control(&mut app, "Reference dimension");
    assert_eq!(session(&app).sketch.status, SolveStatus::FullyDefined);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert_eq!(session(&app).sketch.status, SolveStatus::UnderDefined);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    assert_eq!(session(&app).sketch, before);
    drop(app);
    soliddraft_app::file::set_config_dir(None);
}
