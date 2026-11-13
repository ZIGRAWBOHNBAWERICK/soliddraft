//! Display controls and grid placements through the native sketch editor.

#[path = "support/ui.rs"]
mod ribbon;

use egui::{Key, Modifiers, Pos2};
use egui_kittest::kittest::Queryable;
use ribbon::App;
use soliddraft_app::{
    commands,
    document::{Command, ExtrudeParams, FeatureId, PlaneKind, SketchPlane},
    sketch::{ConstraintKind, EntityKind, P2, PointRef, Sketch, solve},
    sketcher::Frame,
    state::{AppAction, SketchSel, SketchSession},
    viewport::grid::{GridStyle, Spacing},
};
use std::{path::PathBuf, sync::Mutex};

static SERIAL: Mutex<()> = Mutex::new(());

fn p(u: f64, v: f64) -> P2 {
    P2::new(u, v)
}
fn session(app: &App) -> &SketchSession {
    app.state().state.sketch.as_ref().unwrap()
}
fn screen(app: &App, p: P2) -> Pos2 {
    let st = &app.state().state;
    Frame { plane: session(app).plane, camera: &st.viewport.camera, rect: st.viewport.rect }.screen(p).unwrap()
}
fn pick(app: &mut App, p: P2) {
    ribbon::close_options(app);
    let at = screen(app, p);
    assert!(app.state().state.viewport.rect.contains(at), "{p:?} projects outside the viewport: {at:?}");
    ribbon::click_at(app, at);
}
fn text_click(app: &mut App, label: &str) {
    ribbon::open_options(app);
    let control = ribbon::painted(app)
        .into_iter()
        .find(|c| c.text == label && c.rect.contains_rect(c.full))
        .unwrap_or_else(|| panic!("visible control {label}"));
    ribbon::click_at(app, control.rect.center());
}
fn property_text(app: &mut App, label: &str) {
    ribbon::click_option(app, label);
}
fn palette(app: &mut App, query: &str, id: &str) {
    app.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::P);
    app.run_steps(3);
    app.event(egui::Event::Text(query.into()));
    app.run_steps(3);
    assert_eq!(app.state().state.palette.as_ref().unwrap().results(&app.state().state)[0].command.id, id);
    app.key_press(Key::Enter);
    app.run_steps(5);
    assert!(app.state().state.palette.is_none());
}
fn launch(sketch: Sketch, name: &str) -> (App, FeatureId, PathBuf) {
    let dir = std::env::var_os("SOLIDDRAFT_DISPLAY_GRID_PROOF_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("soliddraft-display-grid-{}", std::process::id())))
        .join(name);
    std::fs::create_dir_all(dir.join("config")).unwrap();
    soliddraft_app::file::set_config_dir(Some(dir.join("config")));
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Front, offset: 7.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    let mut app = ribbon::app_at(doc, ribbon::EXPANDED_WINDOW, 1.0);
    for id in ["sketch_shading", "sketch_dimensions", "sketch_grid_snap"] {
        assert!(!commands::get(id).unwrap().is_enabled(&app.state().state));
    }
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(30);
    let st = &mut app.state_mut().state;
    st.viewport.camera.zoom(0.1, None, st.viewport.rect.aspect_ratio());
    app.run_steps(6);
    (app, id, dir)
}
fn reopen(app: &mut App, id: FeatureId, dir: &std::path::Path, expected: &Sketch) {
    let path = dir.join("reopen.sdraft");
    for action in [AppAction::ExitSketch, AppAction::SaveAs(Some(path.clone())), AppAction::Open(Some(path))] {
        app.state_mut().state.push(action);
        app.run_steps(10);
    }
    assert_eq!(&app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, expected);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(20);
    assert_eq!(&session(app).sketch, expected);
    assert!(session(app).show_regions && session(app).show_dimensions);
    assert!(!session(app).grid_snap, "display choices are session-local");
}
fn last_point(app: &App) -> P2 {
    let EntityKind::Point { p } = session(app).sketch.drawn().last().unwrap().kind else { panic!("a point") };
    p
}
fn near(actual: P2, expected: P2) {
    assert!((actual - expected).len() < 0.02, "{actual:?} expected {expected:?}");
}

#[test]
fn contour_and_dimension_switches_change_raster_and_picking_without_history() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let circle = sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 10.0 }, false);
    let radius =
        sketch.constrain(ConstraintKind::Radius { entity: circle, value: 10.0, diameter: false, label: p(16.0, 15.0) });
    sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(circle, 0), b: PointRef::ORIGIN });
    solve::solve(&mut sketch, None, true);
    let (mut app, id, dir) = launch(sketch, "visibility");
    let original = session(&app).sketch.clone();
    let revision = app.state().state.doc.revision();
    let sample = screen(&app, p(-4.0, -4.0));
    let pixel = |image: &image::RgbaImage| image.get_pixel(sample.x as u32, sample.y as u32).0;
    let filled = app.render().unwrap();
    filled.save(dir.join("shading-on.png")).unwrap();
    ribbon::run_command(&mut app, "sketch_shading");
    assert!(!session(&app).show_regions);
    let unfilled = app.render().unwrap();
    unfilled.save(dir.join("shading-off.png")).unwrap();
    assert_ne!(pixel(&filled), pixel(&unfilled), "the closed region fill disappears");
    assert!(ribbon::painted(&app).iter().any(|c| c.text == "R10"));
    let label = screen(&app, p(16.0, 15.0));
    ribbon::click_at(&mut app, label);
    assert_eq!(session(&app).selection, vec![SketchSel::Constraint(radius)]);
    text_click(&mut app, "Sketch display");
    property_text(&mut app, "Sketch dimensions");
    assert!(!session(&app).show_dimensions);
    assert!(session(&app).selection.is_empty());
    assert!(!ribbon::painted(&app).iter().any(|c| c.text == "R10"));
    ribbon::double_click_at(&mut app, label);
    app.run_steps(5);
    assert!(session(&app).dim_edit.is_none());
    assert!(session(&app).selection.is_empty());
    palette(&mut app, "Sketch dimensions", "sketch_dimensions");
    assert!(session(&app).show_dimensions);
    assert!(ribbon::painted(&app).iter().any(|c| c.text == "R10"));
    palette(&mut app, "Contour shading", "sketch_shading");
    assert!(session(&app).show_regions);
    assert_eq!(session(&app).sketch, original);
    assert_eq!(app.state().state.doc.revision(), revision);
    assert_eq!(session(&app).sketch.regions().unwrap().len(), 1);
    reopen(&mut app, id, &dir, &original);
}

#[test]
fn grid_placement_uses_fixed_and_adaptive_spacing_with_cancellation_and_history() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    sketch.add(EntityKind::Circle { center: P2::ZERO, radius: 2.0 }, true);
    let (mut app, id, dir) = launch(sketch, "placement");
    app.state_mut().state.viewport.grid_style = GridStyle { follow_zoom: false, fixed_mm: 2.5, ..Default::default() };
    app.run_steps(4);
    ribbon::run_command(&mut app, "sketch_point");
    text_click(&mut app, "Auto relations");
    ribbon::run_command(&mut app, "sketch_grid_snap");
    assert!(session(&app).grid_snap);
    let before = session(&app).sketch.clone();
    let target = p(13.1, -9.1);
    app.hover_at(screen(&app, target));
    app.run_steps(4);
    assert_eq!(session(&app).sketch, before, "preview is read-only");
    app.render().unwrap().save(dir.join("grid-preview.png")).unwrap();
    let painted = ribbon::painted(&app);
    assert!(
        painted.iter().any(|c| c.text == "12.50, -10"),
        "viewport text: {:?}",
        painted
            .iter()
            .filter(|c| app.state().state.viewport.rect.contains_rect(c.rect))
            .map(|c| &c.text)
            .collect::<Vec<_>>()
    );
    pick(&mut app, target);
    near(last_point(&app), p(12.5, -10.0));
    let after = session(&app).sketch.clone();
    assert_eq!(after.constraints, before.constraints, "grid snap creates no relation");
    app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, before);
    app.key_press_modifiers(Modifiers::COMMAND, Key::Y);
    app.run_steps(6);
    assert_eq!(session(&app).sketch, after);

    // Hiding the grid changes its painting, not its spacing or snap preference.
    app.state_mut().state.viewport.references.show_grid = false;
    app.run_steps(4);
    pick(&mut app, p(-13.1, -9.1));
    near(last_point(&app), p(-12.5, -10.0));
    let revision = app.state().state.doc.revision();
    ribbon::run_command(&mut app, "sketch_rectangle");
    pick(&mut app, p(-8.9, 12.1));
    assert_eq!(session(&app).pending.len(), 1);
    app.key_press(Key::Escape);
    app.run_steps(4);
    assert!(session(&app).pending.is_empty());
    assert_eq!(app.state().state.doc.revision(), revision);
    assert!(session(&app).grid_snap);

    ribbon::run_command(&mut app, "sketch_point");
    app.state_mut().state.viewport.references.show_grid = true;
    app.state_mut().state.viewport.grid_style.follow_zoom = true;
    let mut steps = Vec::new();
    for (factor, target) in [(2.0, p(7.3, 8.6)), (0.25, p(-18.6, 18.1))] {
        let st = &mut app.state_mut().state;
        st.viewport.camera.zoom(factor, None, st.viewport.rect.aspect_ratio());
        app.run_steps(6);
        let view = &app.state().state.viewport;
        let spacing = Spacing::for_scale(view.camera.mm_per_point(view.rect.height()), &view.grid_style).unwrap();
        assert!(ribbon::painted(&app).iter().any(|c| c.text == spacing.label()));
        steps.push(spacing.minor());
        pick(&mut app, target);
        near(last_point(&app), spacing.snap(target));
    }
    assert_ne!(steps[0], steps[1]);
    palette(&mut app, "Snap to grid", "sketch_grid_snap");
    assert!(!session(&app).grid_snap);
    pick(&mut app, p(13.1, -9.1));
    near(last_point(&app), p(13.1, -9.1));
    let saved = session(&app).sketch.clone();
    reopen(&mut app, id, &dir, &saved);
}

#[test]
fn geometry_and_quick_snaps_override_grid_and_closed_sketch_visibility_is_retained() {
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut sketch = Sketch::new();
    let seed = sketch.add(EntityKind::Point { p: p(1.29, 0.79) }, true);
    let (mut app, id, dir) = launch(sketch, "precedence");
    app.state_mut().state.viewport.grid_style = GridStyle { follow_zoom: false, fixed_mm: 5.0, ..Default::default() };
    app.run_steps(4);
    ribbon::run_command(&mut app, "sketch_point");
    text_click(&mut app, "Snap to grid");
    assert!(session(&app).auto_relations);
    assert_eq!(session(&app).sketch.point(PointRef::new(seed, 0)), Some(p(1.29, 0.79)));
    app.hover_at(screen(&app, p(1.29, 0.79)));
    app.run_steps(4);
    assert_eq!(session(&app).snaps, vec![soliddraft_app::state::Snap::Point(PointRef::new(seed, 0))]);
    pick(&mut app, p(1.29, 0.79));
    near(last_point(&app), p(1.29, 0.79));
    assert!(session(&app).sketch.constraints.iter().any(|c| matches!(c.kind,
        ConstraintKind::Coincident { a, b } if a.entity == seed || b.entity == seed)));
    text_click(&mut app, "Quick snap");
    text_click(&mut app, "No snap");
    pick(&mut app, p(13.1, -9.1));
    near(last_point(&app), p(13.1, -9.1));
    assert!(session(&app).quick_snap.is_none());
    pick(&mut app, p(23.1, -9.1));
    near(last_point(&app), p(25.0, -10.0));
    let original = session(&app).sketch.clone();
    app.state_mut().state.push(AppAction::ExitSketch);
    app.run_steps(10);
    app.state_mut().state.push(AppAction::Select(None));
    app.run_steps(4);
    let name = app.state().state.doc.feature(id).unwrap().name.clone();
    // The existing eye control operates independently of sketch editing.
    let show = app.get_by_label(&format!("Show {name}")).rect();
    ribbon::click_at(&mut app, show.center());
    assert!(soliddraft_app::tree_visibility::visible(&app.state().state).sketches.contains(&id));
    let shown = app.render().unwrap();
    shown.save(dir.join("closed-sketch-shown.png")).unwrap();
    let hide = app.get_by_label(&format!("Hide {name}")).rect();
    ribbon::click_at(&mut app, hide.center());
    assert!(!soliddraft_app::tree_visibility::visible(&app.state().state).sketches.contains(&id));
    let hidden = app.render().unwrap();
    hidden.save(dir.join("closed-sketch-hidden.png")).unwrap();
    let viewport = app.state().state.viewport.rect;
    let changed = (viewport.top() as u32..viewport.bottom() as u32)
        .flat_map(|y| (viewport.left() as u32..viewport.right() as u32).map(move |x| (x, y)))
        .filter(|(x, y)| shown.get_pixel(*x, *y) != hidden.get_pixel(*x, *y))
        .count();
    assert!(changed > 10, "the eye changes passive sketch pixels: {changed}");
    assert_eq!(app.state().state.doc.feature(id).unwrap().sketch().unwrap().sketch, original);
    app.state_mut().state.push(AppAction::EditSketch(id));
    app.run_steps(20);
    reopen(&mut app, id, &dir, &original);
}

#[test]
fn rounded_endpoints_form_a_closed_profile_at_the_kernel_boundary() {
    let spacing =
        Spacing::for_scale(0.1, &GridStyle { follow_zoom: false, fixed_mm: 2.5, ..Default::default() }).unwrap();
    let mut sketch = Sketch::new();
    let raw = [p(-6.1, -4.1), p(9.1, -4.2), p(9.2, 6.1), p(-6.2, 6.2)];
    let corners = raw.map(|p| spacing.snap(p));
    for i in 0..4 {
        sketch.add(EntityKind::Line { a: corners[i], b: corners[(i + 1) % 4] }, false);
    }
    sketch.add(EntityKind::Point { p: spacing.snap(p(1.1, 1.1)) }, false);
    assert_eq!(sketch.regions().unwrap().len(), 1, "a reference point does not create a profile");
    let mut doc = ribbon::blank();
    let id = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Front, offset: 7.0 } })
        .unwrap()
        .unwrap();
    doc.apply(Command::EditSketch { id, sketch }).unwrap();
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(id), depth: 4.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let volume = doc.model().kernel().mass_properties(doc.model().body().unwrap()).unwrap().volume;
    assert!((volume - 600.0).abs() < 1e-5, "{volume}");
    let mut reopened =
        soliddraft_model::Document::from_bytes(&doc.model().to_bytes().unwrap(), soliddraft_app::kernel()).unwrap();
    reopened.rebuild();
    let volume = reopened.kernel().mass_properties(reopened.body().unwrap()).unwrap().volume;
    assert!((volume - 600.0).abs() < 1e-5, "{volume}");
}
