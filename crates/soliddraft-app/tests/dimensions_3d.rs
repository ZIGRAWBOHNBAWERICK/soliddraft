//! Model-space annotation placement through native pointer input and the real renderer.
use egui::{Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::kittest::Queryable;
use soliddraft_app::document::{Command, Document, FeatureId, FeatureKind, Selection, ViewItem};
use soliddraft_app::state::AppAction;
use soliddraft_app::viewport::camera::StandardView;
use soliddraft_app::viewport::dimensions::{DimensionTarget, ShownDimension};
use soliddraft_model::view::AnnotationKey;

#[path = "support/ribbon.rs"]
mod ribbon;
use ribbon::App;

fn frame(app: &mut App, name: &str) {
    app.hover_at(egui::pos2(10.0, 10.0));
    app.run_steps(3);
    let dir = std::env::var_os("SOLIDDRAFT_DIMENSION_PROOF_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("soliddraft-dimensions-3d"));
    std::fs::create_dir_all(&dir).unwrap();
    app.render().expect("native renderer").save(dir.join(format!("{name}.png"))).unwrap();
}

fn dimension(app: &App, key: &AnnotationKey) -> ShownDimension {
    app.state().state.viewport.dimensions.iter().find(|d| d.key == *key).expect("shown dimension").clone()
}

fn drag(app: &mut App, from: Pos2, to: Pos2) {
    app.hover_at(from);
    app.run_steps(40);
    app.event(egui::Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::default(),
    });
    app.step();
    for i in 1..=12 {
        app.hover_at(from + (to - from) * (i as f32 / 12.0));
        app.step();
    }
    app.event(egui::Event::PointerButton {
        pos: to,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::default(),
    });
    app.run_steps(5);
}

#[test]
fn feature_drag_is_model_space_undoable_and_keeps_value_editing() {
    let _case = ribbon::case("dimensions-3d-feature");
    let mut app = ribbon::app(ribbon::plate());
    app.key_press(Key::Z);
    app.run_steps(30);
    let name = app.state().state.doc.features[0].name.clone();
    let row = app.get_by_label(&name).rect().center();
    ribbon::double_click_at(&mut app, row);
    app.run_steps(5);
    let original = app.state().state.viewport.dimensions.iter().find(|d| d.tip.ends_with("Depth")).unwrap().clone();
    let before_depth = app.state().state.doc.undo_depth();
    let camera = app.state().state.viewport.camera.view();
    let geometry = app.state().state.doc.model().geometry_hash();
    frame(&mut app, "feature-before-drag");
    let to = original.rect.center() + vec2(95.0, -65.0);
    drag(&mut app, original.rect.center(), to);
    let moved = dimension(&app, &original.key);
    assert!(moved.rect.center().distance(to) < 2.0, "label follows pointer: {moved:?}");
    assert!(moved.position.distance(original.position) > 1.0);
    assert_eq!(app.state().state.viewport.camera.view(), camera, "annotation drag never orbits");
    assert_eq!(app.state().state.doc.undo_depth(), before_depth + 1, "one gesture, one history entry");
    assert_eq!(app.state().state.doc.model().geometry_hash(), geometry, "placement never changes geometry");
    frame(&mut app, "feature-after-drag");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(8);
    assert!(dimension(&app, &original.key).position.distance(original.position) < 0.001);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(8);
    assert!(dimension(&app, &original.key).position.distance(moved.position) < 0.001);
    for (i, view) in [StandardView::Isometric, StandardView::Front, StandardView::Right].into_iter().enumerate() {
        app.state_mut().state.push(AppAction::SetView(view));
        app.run_steps(40);
        let d = dimension(&app, &original.key);
        let viewport = &app.state().state.viewport;
        let expected = viewport.camera.project(moved.position, viewport.rect).unwrap();
        assert!(d.rect.center().distance(expected) < 1.0, "model anchor at {view:?}");
        assert!(d.rect.is_finite() && d.rect.height() >= 15.0 && d.rect.width() >= 20.0);
        assert_eq!(d.text, "20");
        assert!((d.rect.size() - moved.rect.size()).length() < 0.01, "viewer-facing text stays upright, unscaled");
        for label in &app.state().state.viewport.dimensions {
            assert!(
                ribbon::painted_in(&app, label.rect).iter().any(|p| p.text == label.text && p.rect == p.full),
                "the complete value is painted at {view:?}: {label:?}"
            );
        }
        frame(&mut app, &format!("feature-angle-{i}"));
    }
    let at = dimension(&app, &original.key).rect.center();
    ribbon::double_click_at(&mut app, at);
    assert!(app.state().state.dimension_edit.is_some(), "moved value still edits in place");
    app.key_press_modifiers(Modifiers::COMMAND, Key::A);
    app.get_by_label("Depth dimension value").type_text("25");
    app.key_press(Key::Enter);
    app.run_steps(12);
    ribbon::until(&mut app, "the edited depth's proof", |a| !a.state().state.doc.proof_pending());
    assert_eq!(dimension(&app, &original.key).text, "25");
    assert!(dimension(&app, &original.key).position.distance(moved.position) < 0.001);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(12);
    ribbon::until(&mut app, "undo of the value edit", |a| !a.state().state.doc.proof_pending());
    // Geometry undo retains its existing feature-selection behavior.
    app.get_by_label("Solid body").click();
    app.run_steps(5);
    assert_eq!(dimension(&app, &original.key).text, "20");
    assert!(
        dimension(&app, &original.key).position.distance(moved.position) < 0.001,
        "undoing a subsequent geometry edit leaves the earlier placement intact"
    );
    let bytes = app.state().state.doc.model().to_bytes().unwrap();
    let reopened = soliddraft_model::Document::from_bytes(&bytes, soliddraft_app::kernel()).unwrap();
    let mut reopened = ribbon::app(Document::from_model(reopened));
    reopened.state_mut().state.push(AppAction::Select(Some(Selection::Body)));
    reopened.run_steps(6);
    assert!(
        dimension(&reopened, &original.key).position.distance(original.position) < 0.001,
        "the current part format has no 3D placement storage; reopening uses default placement"
    );
}

#[test]
fn every_visible_sketch_dimension_drags_without_selecting_the_body() {
    let _case = ribbon::case("dimensions-3d-sketch");
    let mut doc = ribbon::sketch();
    let id = doc.features[0].id;
    doc.selection = None;
    doc.set_view_choice(ViewItem::Feature(id), Some(true));
    let mut app = ribbon::app(doc);
    app.key_press(Key::Z);
    app.run_steps(30);
    let dims = app.state().state.viewport.dimensions.clone();
    assert_eq!(dims.len(), 2, "both dimensions on a visible unconsumed sketch");
    frame(&mut app, "sketch-before-drag");
    for d in &dims {
        assert!(matches!(d.target, DimensionTarget::Sketch { sketch, .. } if sketch == id));
        let from = dimension(&app, &d.key).rect.center();
        let to = from + vec2(75.0, -45.0);
        drag(&mut app, from, to);
        assert!(dimension(&app, &d.key).rect.center().distance(to) < 2.0);
        assert_ne!(app.state().state.doc.selection, Some(Selection::Body));
    }
    frame(&mut app, "sketch-after-drag");
    let moved = dimension(&app, &dims[1].key);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert!(dimension(&app, &dims[1].key).position.distance(dims[1].position) < 0.001);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert!(dimension(&app, &dims[1].key).position.distance(moved.position) < 0.001);
    app.state_mut().state.doc.set_view_choice(ViewItem::Feature(id), Some(false));
    app.run_steps(4);
    assert!(app.state().state.viewport.dimensions.is_empty(), "hidden sketch has no annotations");
    app.state_mut().state.doc.set_view_choice(ViewItem::Feature(id), Some(true));
    app.run_steps(4);
    assert!(dimension(&app, &dims[1].key).position.distance(moved.position) < 0.001);
    let at = dimension(&app, &dims[0].key).rect.center();
    ribbon::double_click_at(&mut app, at);
    assert_eq!(app.state().state.sketch.as_ref().expect("sketch value editor").feature, id);
}

#[test]
fn escape_cancels_a_dimension_drag_without_orbiting_or_recording_history() {
    let _case = ribbon::case("dimensions-3d-cancel");
    let mut doc = ribbon::plate();
    doc.selection = Some(Selection::Body);
    let mut app = ribbon::app(doc);
    app.key_press(Key::Z);
    app.run_steps(30);
    let label = app.state().state.viewport.dimensions.iter().find(|d| d.tip.ends_with("Depth")).unwrap().clone();
    let depth = app.state().state.doc.undo_depth();
    let camera = app.state().state.viewport.camera.view();
    let from = label.rect.center();
    app.hover_at(from);
    app.run_steps(40);
    app.event(egui::Event::PointerButton {
        pos: from,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    app.step();
    app.hover_at(from + vec2(90.0, -60.0));
    app.step();
    assert!(dimension(&app, &label.key).position.distance(label.position) > 1.0);
    app.key_press(Key::Escape);
    app.step();
    app.hover_at(from + vec2(120.0, -80.0));
    app.run_steps(3);
    app.event(egui::Event::PointerButton {
        pos: from + vec2(120.0, -80.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    app.run_steps(5);
    assert_eq!(app.state().state.doc.selection, Some(Selection::Body));
    assert!(dimension(&app, &label.key).position.distance(label.position) < 0.001);
    assert_eq!(app.state().state.doc.undo_depth(), depth);
    assert_eq!(app.state().state.viewport.camera.view(), camera);
}

fn multi_value_document(hole: bool) -> (Document, FeatureId, usize) {
    let mut doc = ribbon::plate();
    let kind = if hole {
        let top = doc.faces.iter().find(|face| face.label == "End cap").unwrap().id;
        FeatureKind::Hole(soliddraft_model::HoleFeature {
            face: doc.face_ref(top).unwrap(),
            points: vec![soliddraft_sketch::P2::ZERO],
            diameter: 6.0,
            depth: soliddraft_kernel_api::HoleDepth::Blind { depth: 8.0 },
            tip: soliddraft_kernel_api::HoleTip::Cone { angle_deg: 90.0 },
            head: Some(soliddraft_kernel_api::HoleHead::Counterbore { diameter: 10.0, depth: 3.0 }),
        })
    } else {
        FeatureKind::Pattern(soliddraft_model::PatternFeature {
            source: None,
            layout: soliddraft_model::PatternSpec::Linear {
                direction: soliddraft_kernel_api::Vec3::X,
                spacing: 80.0,
                count: 3,
            },
        })
    };
    let feature = doc.apply(Command::AddFeature(kind)).unwrap().unwrap();
    doc.selection = Some(Selection::Body);
    (doc, feature, if hole { 4 } else { 2 })
}

#[test]
fn multi_value_defaults_stay_separate_and_each_centre_grabs_its_own_label() {
    let _case = ribbon::case("dimensions-3d-multi-value");
    for hole in [true, false] {
        let (doc, feature, count) = multi_value_document(hole);
        let name = if hole { "hole" } else { "pattern" };
        let mut app = ribbon::app(doc);
        app.key_press(Key::Z);
        app.run_steps(40);
        for orthographic in [true, false] {
            app.state_mut().state.viewport.camera.orthographic = orthographic;
            for (view_name, view) in [
                ("top", Some(StandardView::Top)),
                ("bottom", Some(StandardView::Bottom)),
                ("front", Some(StandardView::Front)),
                ("right", Some(StandardView::Right)),
                ("isometric", Some(StandardView::Isometric)),
                ("oblique", None),
            ] {
                if let Some(view) = view {
                    app.state_mut().state.push(AppAction::SetView(view));
                } else {
                    app.state_mut().state.viewport.camera.turn_to("Oblique", 25.0, 17.0);
                }
                app.run_steps(40);
                let labels: Vec<_> = app
                    .state()
                    .state
                    .viewport
                    .dimensions
                    .iter()
                    .filter(|d| d.target == DimensionTarget::Feature(feature))
                    .cloned()
                    .collect();
                assert_eq!(labels.len(), count, "{name} exposes every value");
                for (i, label) in labels.iter().enumerate() {
                    assert!(
                        app.state().state.viewport.rect.contains_rect(label.rect),
                        "{name} {view_name} orthographic={orthographic}: {label:?}"
                    );
                    for other in &labels[i + 1..] {
                        assert!(
                            !label.rect.expand(2.0).intersects(other.rect),
                            "{name} {view_name} orthographic={orthographic}: {:?} overlaps {:?}",
                            label.key,
                            other.key
                        );
                    }
                    assert!(
                        ribbon::painted_in(&app, label.rect).iter().any(|p| p.text == label.text && p.rect == p.full),
                        "the complete {name} value is readable in {view_name}"
                    );
                }
                frame(
                    &mut app,
                    &format!("multi-{name}-{view_name}-{}", if orthographic { "ortho" } else { "perspective" }),
                );
                for label in &labels {
                    let camera = app.state().state.viewport.camera.view();
                    let undo = app.state().state.doc.undo_depth();
                    let to = label.rect.center() + vec2(100.0, -20.0);
                    drag(&mut app, label.rect.center(), to);
                    let moved = dimension(&app, &label.key);
                    assert!(
                        moved.rect.center().distance(to) < 2.0,
                        "{name} {view_name}: its centre grabs {:?}, not another value",
                        label.key
                    );
                    assert_eq!(app.state().state.viewport.camera.view(), camera);
                    assert_eq!(app.state().state.doc.undo_depth(), undo + 1);
                    for other in labels.iter().filter(|other| other.key != label.key) {
                        assert!(
                            dimension(&app, &other.key).position.distance(other.position) < 0.001,
                            "grabbing {:?} must not move {:?}",
                            label.key,
                            other.key
                        );
                    }
                    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
                    app.run_steps(5);
                    assert!(
                        dimension(&app, &label.key).rect.center().distance(label.rect.center()) < 0.01,
                        "undo returns to the current view's default stack"
                    );
                }
            }
        }
        let label = app
            .state()
            .state
            .viewport
            .dimensions
            .iter()
            .find(|d| d.target == DimensionTarget::Feature(feature))
            .unwrap()
            .clone();
        drag(&mut app, label.rect.center(), label.rect.center() + vec2(100.0, -20.0));
        let authored = dimension(&app, &label.key).position;
        app.state_mut().state.push(AppAction::SetView(StandardView::Top));
        app.run_steps(40);
        assert!(
            dimension(&app, &label.key).position.distance(authored) < 0.001,
            "a manually moved label never follows automatic camera layout"
        );
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        app.run_steps(5);
        app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        app.run_steps(5);
        assert!(
            dimension(&app, &label.key).position.distance(authored) < 0.001,
            "redo restores the authored model-space anchor even in another view"
        );
    }
}

#[test]
fn two_closed_rectangles_keep_front_view_dimensions_readable_and_draggable() {
    let _case = ribbon::case("dimensions-3d-two-rectangles");
    let mut doc = ribbon::blank();
    let sketch = doc
        .apply(Command::AddSketch {
            plane: soliddraft_app::document::SketchPlane::Reference {
                plane: soliddraft_app::document::PlaneKind::Top,
                offset: 0.0,
            },
        })
        .unwrap()
        .unwrap();
    let mut geometry = soliddraft_sketch::Sketch::new();
    geometry.rectangle(soliddraft_sketch::P2::new(0.0, -60.0), 60.0, 40.0).unwrap();
    geometry.rectangle(soliddraft_sketch::P2::new(0.0, 60.0), 80.0, 40.0).unwrap();
    doc.apply(Command::EditSketch { id: sketch, sketch: geometry }).unwrap();
    doc.set_view_choice(ViewItem::Feature(sketch), Some(true));
    doc.selection = None;
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::SetView(StandardView::Front));
    app.run_steps(40);
    for orthographic in [true, false] {
        app.state_mut().state.viewport.camera.orthographic = orthographic;
        app.run_steps(5);
        let labels = app.state().state.viewport.dimensions.clone();
        let mut values: Vec<_> = labels.iter().map(|d| d.text.as_str()).collect();
        values.sort_unstable();
        assert_eq!(values, ["40", "40", "60", "80"]);
        for (i, label) in labels.iter().enumerate() {
            assert!(matches!(label.target, DimensionTarget::Sketch { sketch: id, .. } if id == sketch));
            assert!(ribbon::painted_in(&app, label.rect).iter().any(|p| p.text == label.text && p.rect == p.full));
            for other in &labels[i + 1..] {
                assert!(
                    !label.rect.expand(1.0).intersects(other.rect),
                    "orthographic={orthographic}: {} covers {}",
                    label.text,
                    other.text
                );
            }
        }
        frame(&mut app, if orthographic { "two-rectangles-front-ortho" } else { "two-rectangles-front-perspective" });
        for label in &labels {
            let to = label.rect.center() + vec2(100.0, -35.0);
            drag(&mut app, label.rect.center(), to);
            assert!(
                dimension(&app, &label.key).rect.center().distance(to) < 2.0,
                "the {} centre grabs its own constraint",
                label.text
            );
            app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            app.run_steps(5);
            assert!(dimension(&app, &label.key).rect.center().distance(label.rect.center()) < 0.01);
        }
    }
    let labels = app.state().state.viewport.dimensions.clone();
    let first = labels.iter().find(|d| d.text == "60").unwrap();
    let second = labels.iter().find(|d| d.text == "80").unwrap();
    drag(&mut app, second.rect.center(), first.rect.center());
    let moved = dimension(&app, &second.key);
    assert!(
        moved.rect.center().distance(first.rect.center()) < 2.0,
        "authored placement wins over default collision avoidance"
    );
    let default = dimension(&app, &first.key);
    assert!(!default.rect.intersects(moved.rect), "the automatic value clears the authored value");
    frame(&mut app, "two-rectangles-authored-priority");
    app.state_mut().state.push(AppAction::SetView(StandardView::Top));
    app.run_steps(40);
    assert!(dimension(&app, &second.key).position.distance(moved.position) < 0.001);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(5);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(5);
    assert!(
        dimension(&app, &second.key).position.distance(moved.position) < 0.001,
        "redo retains the authored sketch anchor after a view change"
    );
}

#[test]
fn authored_labels_take_priority_across_separate_visible_sketches() {
    let _case = ribbon::case("dimensions-3d-cross-sketch-priority");
    let mut doc = ribbon::blank();
    let mut sketches = Vec::new();
    for (v, width) in [(-60.0, 60.0), (60.0, 80.0)] {
        let id = doc
            .apply(Command::AddSketch {
                plane: soliddraft_app::document::SketchPlane::Reference {
                    plane: soliddraft_app::document::PlaneKind::Top,
                    offset: 0.0,
                },
            })
            .unwrap()
            .unwrap();
        let mut geometry = soliddraft_sketch::Sketch::new();
        geometry.rectangle(soliddraft_sketch::P2::new(0.0, v), width, 40.0).unwrap();
        doc.apply(Command::EditSketch { id, sketch: geometry }).unwrap();
        doc.set_view_choice(ViewItem::Feature(id), Some(true));
        sketches.push(id);
    }
    doc.selection = None;
    let mut app = ribbon::app(doc);
    app.state_mut().state.push(AppAction::SetView(StandardView::Front));
    app.run_steps(40);
    for orthographic in [true, false] {
        app.state_mut().state.viewport.camera.orthographic = orthographic;
        app.run_steps(5);
        let before = app.state().state.viewport.dimensions.clone();
        let first = before.iter().find(|d| d.text == "60").unwrap();
        let second = before.iter().find(|d| d.text == "80").unwrap();
        assert!(matches!(first.target, DimensionTarget::Sketch { sketch, .. } if sketch == sketches[0]));
        assert!(matches!(second.target, DimensionTarget::Sketch { sketch, .. } if sketch == sketches[1]));
        drag(&mut app, second.rect.center(), first.rect.center());
        let authored = dimension(&app, &second.key);
        assert!(
            authored.rect.center().distance(first.rect.center()) < 2.0,
            "later sketch's authored value stays where the user put it"
        );
        let default = dimension(&app, &first.key);
        assert!(
            !default.rect.expand(1.0).intersects(authored.rect),
            "earlier sketch's default clears the later sketch's authored label"
        );
        let shown = app.state().state.viewport.dimensions.clone();
        let mut values: Vec<_> = shown.iter().map(|d| d.text.as_str()).collect();
        values.sort_unstable();
        assert_eq!(values, ["40", "40", "60", "80"]);
        for (i, label) in shown.iter().enumerate() {
            assert!(ribbon::painted_in(&app, label.rect).iter().any(|p| p.text == label.text && p.rect == p.full));
            for other in &shown[i + 1..] {
                assert!(
                    !label.rect.expand(1.0).intersects(other.rect),
                    "separate sketches keep every default clear of authored labels"
                );
            }
        }
        frame(&mut app, if orthographic { "cross-sketch-front-ortho" } else { "cross-sketch-front-perspective" });
        for label in [&default, &authored] {
            let to = dimension(&app, &label.key).rect.center() + vec2(100.0, -35.0);
            let from = dimension(&app, &label.key).rect.center();
            drag(&mut app, from, to);
            assert!(
                dimension(&app, &label.key).rect.center().distance(to) < 2.0,
                "each separate sketch value remains grabbable at its own centre"
            );
            app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            app.run_steps(5);
            assert!(dimension(&app, &second.key).position.distance(authored.position) < 0.001);
        }
        app.state_mut().state.push(AppAction::SetView(StandardView::Top));
        app.run_steps(40);
        assert!(dimension(&app, &second.key).position.distance(authored.position) < 0.001);
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        app.run_steps(5);
        app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
        app.run_steps(5);
        assert!(
            dimension(&app, &second.key).position.distance(authored.position) < 0.001,
            "cross-sketch collision layout never changes authored-anchor history"
        );
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        app.run_steps(5);
        app.state_mut().state.push(AppAction::SetView(StandardView::Front));
        app.run_steps(40);
    }
}
