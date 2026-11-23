//! Part colours and transparency, through the properties panel and the real wgpu
//! pipeline: the sample bracket goes blue from a swatch, transparent from the
//! checkbox, more transparent from a slider drag that applies once when the pointer
//! lets go, and with its part see-through an opaque red boss shows through the front
//! wall. Frames are written to `target/exit-test/appearance-*.png` for review.
//!
//! Skips itself when no GPU adapter is available (plain CI runners).

use egui::{Modifiers, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::document::Selection;
use soliddraft_app::state::AppAction;
use soliddraft_app::viewport::camera::StandardView;
use soliddraft_materials::{Appearance, Srgb};
use soliddraft_model::AppearanceTarget;
use std::path::PathBuf;

fn gpu_available() -> bool {
    let _ = env_logger::try_init();
    let instance = egui_wgpu::wgpu::Instance::default();
    !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty()
}

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/exit-test");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Mean colour of a 5 by 5 pixel patch.
fn colour(image: &image::RgbaImage, at: Pos2) -> [f64; 3] {
    let mut sum = [0.0; 3];
    for y in at.y as u32 - 2..=at.y as u32 + 2 {
        for x in at.x as u32 - 2..=at.x as u32 + 2 {
            let p = image.get_pixel(x, y);
            for c in 0..3 {
                sum[c] += p[c] as f64 / 25.0;
            }
        }
    }
    sum
}

fn pointer(app: &mut Harness<'_, SolidDraftApp>, pos: Pos2, pressed: bool) {
    app.event(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Modifiers::default(),
    });
    app.step();
}

#[test]
fn a_part_takes_a_colour_and_turns_see_through() {
    if !gpu_available() {
        return;
    }
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            SolidDraftApp::new(cc, soliddraft_app::command_tools::board_bracket(), Automation::default())
        });
    app.run_steps(10);
    let part = || AppearanceTarget::Part;
    let blue = Srgb::new(21, 101, 192);

    // The Appearance command opens the panel on the body.
    app.state_mut().state.push(AppAction::ShowAppearance);
    app.run_steps(6);
    assert_eq!(app.state().state.doc.selection, Some(Selection::Body));
    assert!(!app.state().state.doc.has_own_appearance(&part()), "the bracket starts in the default grey");

    // A swatch applies at once, as one undo step, keeping the default finish.
    let depth = app.state().state.doc.model().undo_depth();
    app.get_by_label("Blue").click();
    app.run_steps(4);
    let doc = &app.state().state.doc;
    let shown = doc.shown_appearance(None);
    assert_eq!((shown.base_color, shown.opacity, shown.roughness), (blue, 1.0, 0.45));
    assert_eq!(doc.undo_label(), Some("set appearance Blue on the part"));
    assert_eq!(doc.model().undo_depth(), depth + 1);
    assert!(doc.meshes.values().all(|m| m.appearances[0].base_color[3] == 1.0));

    // The checkbox makes it 60% transparent.
    app.get_by_label("Transparent").click();
    app.run_steps(4);
    let doc = &app.state().state.doc;
    let shown = doc.shown_appearance(None);
    assert_eq!((shown.base_color, shown.opacity), (blue, 0.4));
    assert_eq!(doc.undo_label(), Some("set appearance Blue, 60% transparent on the part"));
    assert_eq!(doc.model().undo_depth(), depth + 2);

    // Dragging the slider shows each value without applying it; letting go applies
    // the last one as one step.
    // The slider, not the number box beside it that shares its label.
    let slider = app.get_by_role_and_label(egui::accesskit::Role::Slider, "Transparency").rect();
    let from = slider.center();
    pointer(&mut app, from, true);
    for step in 1..=8 {
        app.hover_at(from + vec2(step as f32 * 30.0, 0.0));
        app.step();
    }
    {
        let doc = &app.state().state.doc;
        assert!(doc.appearance_preview().is_some(), "the drag previews");
        assert_eq!(doc.model().undo_depth(), depth + 2, "nothing applied mid-drag");
        assert!(doc.shown_appearance(None).opacity < 0.4, "the preview is more transparent");
    }
    pointer(&mut app, from + vec2(240.0, 0.0), false);
    app.run_steps(4);
    let doc = &app.state().state.doc;
    assert!(doc.appearance_preview().is_none());
    assert_eq!(doc.model().undo_depth(), depth + 3, "one step for the whole drag");
    assert!((doc.shown_appearance(None).opacity - 0.1).abs() < 1e-6, "the slider stops at 90%");

    // Undo walks back through the slider, the checkbox and the swatch.
    app.state_mut().state.doc.undo().unwrap();
    assert_eq!(app.state().state.doc.shown_appearance(None).opacity, 0.4);
    app.state_mut().state.doc.undo().unwrap();
    assert_eq!(app.state().state.doc.shown_appearance(None).opacity, 1.0);
    app.state_mut().state.doc.redo().unwrap();

    // From the front, the front wall hides the boss behind it until the part is
    // see-through; an opaque red boss then shows through the blue wall.
    {
        let doc = &mut app.state_mut().state.doc;
        let boss = doc.features.iter().find(|f| f.name == "Boss-Extrude3").expect("the boss").id;
        let red = Appearance::new("red", "Red", Srgb::new(196, 48, 40));
        let target = AppearanceTarget::Feature { feature: soliddraft_model::FeatureId(boss as u32) };
        doc.set_appearance(target, Some(red)).unwrap();
    }
    app.state_mut().state.push(AppAction::Select(None));
    app.state_mut().state.push(AppAction::SetView(StandardView::Front));
    app.run_steps(60);
    let rect = app.state().state.viewport.rect;
    let behind = app.state().state.viewport.camera.project(glam::Vec3::new(85.0, 43.0, 22.0), rect).expect("in view");
    let image = app.render().expect("render");
    image.save(out_dir().join("appearance-see-through.png")).unwrap();
    let [r, g, _] = colour(&image, behind);
    assert!(r > g + 25.0, "the red boss shows through the transparent wall: {:?}", colour(&image, behind));

    let opaque = Appearance::new("opaque-blue", "Opaque blue", blue);
    app.state_mut().state.doc.set_appearance(part(), Some(opaque)).unwrap();
    app.run_steps(4);
    let image = app.render().expect("render");
    image.save(out_dir().join("appearance-opaque.png")).unwrap();
    let [r, g, _] = colour(&image, behind);
    assert!(r < g, "an opaque wall hides the boss: {:?}", colour(&image, behind));

    // The command opens on a selected face and colours only that face.
    let face = app.state().state.doc.faces[0].id;
    app.state_mut().state.push(AppAction::Select(Some(Selection::Face(face))));
    app.state_mut().state.push(AppAction::ShowAppearance);
    app.run_steps(6);
    assert_eq!(app.state().state.doc.selection, Some(Selection::Face(face)));
    app.get_by_label("Green").click();
    app.run_steps(4);
    let doc = &app.state().state.doc;
    assert_eq!(doc.shown_appearance(Some(face)).base_color, Srgb::new(46, 125, 50));
    assert_eq!(doc.shown_appearance(None).base_color, blue, "the rest of the part stays blue");
    app.get_by_label("Use the part's appearance").click();
    app.run_steps(4);
    assert_eq!(app.state().state.doc.shown_appearance(Some(face)).base_color, blue);
}

/// Undo in the middle of a transparency drag ends the drag's preview for good: letting go
/// applies nothing, and Redo brings back only what was applied, never the preview.
#[test]
fn undo_during_a_transparency_drag_drops_the_preview() {
    if !gpu_available() {
        return;
    }
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            SolidDraftApp::new(cc, soliddraft_app::command_tools::board_bracket(), Automation::default())
        });
    app.run_steps(10);
    app.state_mut().state.push(AppAction::ShowAppearance);
    app.run_steps(6);
    app.get_by_label("Blue").click();
    app.run_steps(4);
    app.get_by_label("Transparent").click();
    app.run_steps(4);
    let depth = app.state().state.doc.model().undo_depth();
    assert_eq!(app.state().state.doc.shown_appearance(None).opacity, 0.4);

    let from = app.get_by_role_and_label(egui::accesskit::Role::Slider, "Transparency").rect().center();
    pointer(&mut app, from, true);
    for step in 1..=8 {
        app.hover_at(from + vec2(step as f32 * 30.0, 0.0));
        app.step();
    }
    assert!(app.state().state.doc.appearance_preview().is_some(), "the drag previews");

    // Undo while the pointer is still down takes back the Transparent step.
    app.state_mut().state.push(AppAction::Undo);
    app.step();
    let opaque = |app: &Harness<'_, SolidDraftApp>| {
        let doc = &app.state().state.doc;
        (doc.appearance_preview().is_none(), doc.shown_appearance(None).opacity)
    };
    assert_eq!(opaque(&app), (true, 1.0), "undo drops the preview and shows the opaque blue");

    pointer(&mut app, from + vec2(240.0, 0.0), false);
    app.run_steps(4);
    assert_eq!(app.state().state.doc.model().undo_depth(), depth - 1, "letting go applies nothing");
    assert_eq!(opaque(&app), (true, 1.0));

    // Redo restores the applied 60%, and the dropped preview stays dropped.
    app.state_mut().state.push(AppAction::Redo);
    app.run_steps(4);
    let doc = &app.state().state.doc;
    assert!(doc.appearance_preview().is_none(), "redo does not revive the preview");
    assert_eq!(doc.shown_appearance(None).opacity, 0.4);
    assert_eq!(doc.model().undo_depth(), depth);
    assert!(doc.meshes.values().all(|m| (m.appearances[0].base_color[3] - 0.4).abs() < 1e-6), "the mesh shows 60%");
    app.run_steps(4);
    let doc = &app.state().state.doc;
    assert_eq!(doc.model().undo_depth(), depth, "nothing is applied after the redo");
    assert_eq!(doc.undo_label(), Some("set appearance Blue, 60% transparent on the part"));
}

/// From one transparency to another: after Undo in the middle of a drag, the still-held
/// slider never makes a new preview, so letting go applies nothing and Redo still has
/// the step it undid.
#[test]
fn a_held_slider_after_undo_applies_nothing() {
    if !gpu_available() {
        return;
    }
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            SolidDraftApp::new(cc, soliddraft_app::command_tools::board_bracket(), Automation::default())
        });
    app.run_steps(10);
    app.state_mut().state.push(AppAction::ShowAppearance);
    app.run_steps(6);
    app.get_by_label("Transparent").click();
    app.run_steps(4);
    assert_eq!(app.state().state.doc.shown_appearance(None).opacity, 0.4, "60% transparent");
    let slider = app.get_by_role_and_label(egui::accesskit::Role::Slider, "Transparency").rect();

    // A first drag applies another transparency.
    pointer(&mut app, slider.center(), true);
    app.hover_at(slider.center() + vec2(15.0, 0.0));
    app.step();
    pointer(&mut app, slider.center() + vec2(15.0, 0.0), false);
    app.run_steps(4);
    let committed = app.state().state.doc.shown_appearance(None).opacity;
    assert!(committed != 0.4 && committed > 0.1, "the first drag applied {committed}");
    let depth = app.state().state.doc.model().undo_depth();

    // A second drag toward 90%, and Undo while it is held.
    pointer(&mut app, slider.center(), true);
    for step in 1..=6 {
        app.hover_at(slider.center() + vec2(step as f32 * 40.0, 0.0));
        app.step();
    }
    app.state_mut().state.push(AppAction::Undo);
    app.step();
    let shown = |app: &Harness<'_, SolidDraftApp>| {
        let doc = &app.state().state.doc;
        (doc.appearance_preview().is_none(), doc.shown_appearance(None).opacity, doc.model().undo_depth())
    };
    assert_eq!(shown(&app), (true, 0.4, depth - 1), "undo is back at 60%");

    // Held still, then moved, the slider shows nothing new.
    let held = slider.center() + vec2(240.0, 0.0);
    app.run_steps(4);
    app.hover_at(held - vec2(20.0, 0.0));
    app.step();
    app.hover_at(held);
    app.run_steps(4);
    assert_eq!(shown(&app), (true, 0.4, depth - 1), "a held slider makes no preview after undo");

    // Letting go applies nothing, so Redo still brings back the first drag's value.
    pointer(&mut app, held, false);
    app.run_steps(4);
    assert_eq!(shown(&app), (true, 0.4, depth - 1), "letting go applies nothing");
    app.state_mut().state.push(AppAction::Redo);
    app.run_steps(4);
    assert_eq!(shown(&app), (true, committed, depth), "redo restores the first drag");

    // The next gesture works as usual.
    pointer(&mut app, slider.center(), true);
    app.hover_at(slider.center() + vec2(60.0, 0.0));
    app.step();
    assert!(app.state().state.doc.appearance_preview().is_some(), "a new drag previews again");
    pointer(&mut app, slider.center() + vec2(60.0, 0.0), false);
    app.run_steps(4);
    assert_eq!(app.state().state.doc.model().undo_depth(), depth + 1);
}

#[test]
fn named_state_renders_two_body_appearances_and_reopens_with_visibility() {
    use soliddraft_app::document::{Command, Document, ExtrudeParams, PlaneKind, SketchPlane};
    use soliddraft_document::display_state::{DisplayMode, DisplayScope, DisplayState};
    use soliddraft_materials::{Catalog, MaterialId};
    use soliddraft_sketch::{P2, Sketch};

    fn block(doc: &mut Document, x: f64, depth: f64) {
        let id = doc
            .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
            .unwrap()
            .unwrap();
        let mut sketch = Sketch::new();
        sketch.rectangle(P2::new(x, 0.0), 20.0, 10.0).unwrap();
        doc.apply(Command::EditSketch { id, sketch }).unwrap();
        doc.apply(Command::AddExtrude {
            params: ExtrudeParams { sketch: Some(id), depth, ..Default::default() },
            via_assistant: false,
        })
        .unwrap();
    }

    let mut doc = Document::new("Painted parts", soliddraft_app::kernel());
    block(&mut doc, -18.0, 10.0);
    doc.new_body();
    block(&mut doc, 18.0, 20.0);
    let catalog = Catalog::built_in();
    let steel = catalog.material(&MaterialId("steel-generic".into())).unwrap().1;
    doc.set_material(Some(steel.clone()), catalog.default_appearance_of(steel).cloned()).unwrap();
    assert!((doc.model().mass(None).unwrap().mass_kg - 0.0474).abs() < 1e-9);

    let blue = Appearance::new("face-blue", "Blue", Srgb::new(21, 101, 192));
    let red = Appearance::new("body-red", "Red", Srgb::new(198, 40, 40));
    let right = doc.model().live_bodies().into_iter().find(|(key, _)| *key == 1).unwrap().1;
    let face = doc
        .kernel()
        .topology(right)
        .unwrap()
        .faces
        .into_iter()
        .find(|face| face.normal.is_some_and(|n| n.dot(soliddraft_kernel_api::Vec3::Z) > 0.9999))
        .unwrap()
        .id;
    let face = doc.model().face_ref(face).unwrap();
    doc.set_appearance(AppearanceTarget::Body { body: 0 }, Some(red)).unwrap();
    doc.set_appearance(AppearanceTarget::Face { face }, Some(blue)).unwrap();
    doc.apply(Command::AddConfiguration { name: "Service".into() }).unwrap();
    let config = doc.model().named_configurations().next().unwrap().0.clone();
    let inspection = DisplayState::new("Inspection".into(), DisplayScope::All, DisplayMode::Shaded);
    let mut working: DisplayState<soliddraft_model::PartDisplay> =
        DisplayState::new("Working".into(), DisplayScope::Only(vec![Some(config.clone())]), DisplayMode::Shaded);
    working.view.hidden.insert(0);
    let mut states = doc.display_states().clone();
    let inspection_id = inspection.id.clone();
    let working_id = working.id.clone();
    states.put(inspection).unwrap();
    states.put(working).unwrap();
    states.select(None, Some(inspection_id.clone())).unwrap();
    states.select(Some(&config), Some(working_id.clone())).unwrap();
    doc.change_display_states(states).unwrap();
    let bytes = doc.model().to_bytes().unwrap();
    std::fs::write(out_dir().join("display-states.dwprt"), &bytes).unwrap();

    doc.apply(Command::ActivateConfiguration { name: "Service".into() }).unwrap();
    std::fs::write(out_dir().join("display-states-service.dwprt"), doc.model().to_bytes().unwrap()).unwrap();

    let back = soliddraft_model::Document::from_bytes(&bytes, soliddraft_app::kernel()).unwrap();
    let mut doc = Document::from_model(back);
    assert_eq!(doc.display_state().unwrap().id, inspection_id);
    doc.new_body();
    block(&mut doc, 50.0, 5.0);
    assert_eq!(doc.model().live_bodies().len(), 3);

    if !gpu_available() {
        return;
    }
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| SolidDraftApp::new(cc, doc, Automation::default()));
    for action in [AppAction::Select(None), AppAction::SetView(StandardView::Top), AppAction::ZoomFit] {
        app.state_mut().state.push(action);
    }
    app.run_steps(80);
    let points = |app: &Harness<'_, SolidDraftApp>| {
        let v = &app.state().state.viewport;
        [(-18.0, 10.0), (18.0, 20.0)].map(|(x, z)| v.camera.project(glam::Vec3::new(x, 2.5, z), v.rect).unwrap())
    };
    let [left, right] = points(&app);
    let image = app.render().unwrap();
    image.save(out_dir().join("display-state-inspection.png")).unwrap();
    let [red, _, blue] = colour(&image, left);
    assert!(red > blue + 30.0, "the first body's red appearance is drawn");
    let [red, _, blue] = colour(&image, right);
    assert!(blue > red + 30.0, "the second body's blue face is drawn");

    app.state_mut().state.doc.apply(Command::ActivateConfiguration { name: "Service".into() }).unwrap();
    app.run_steps(30);
    assert_eq!(app.state().state.doc.display_state().unwrap().id, working_id);
    assert_eq!(app.state().state.doc.hidden_display_bodies().len(), 1);
    let [left, right] = points(&app);
    let image = app.render().unwrap();
    image.save(out_dir().join("display-state-working.png")).unwrap();
    let [red, _, blue] = colour(&image, left);
    assert!(red < blue + 15.0, "the hidden red body leaves the viewport");
    let [red, _, blue] = colour(&image, right);
    assert!(blue > red + 30.0, "the blue face remains visible");
}

#[test]
fn assembly_colour_replaces_and_reveals_the_part_finish_after_reopen() {
    use glam::DVec3;
    use soliddraft_app::assembly::AssemblyAction;
    use soliddraft_assembly::{Assembly, DisplayMesh, OccurrencePath, Placement, RefCopy, Step};
    use soliddraft_document::{ContentHash, DocKind, DocRef, DocumentId};

    if !gpu_available() {
        return;
    }
    let part = DocumentId::new();
    let revision = ContentHash::of_bytes(b"painted blue block");
    let mut mesh = DisplayMesh::cuboid(DVec3::ZERO, DVec3::splat(10.0));
    for face in &mut mesh.faces {
        face.style = Some([21, 101, 192, 0, 128]);
    }
    let mut assembly = Assembly::new("Painted assembly");
    let source = assembly.add_source(DocRef::new(part, DocKind::Part, "Block.dwprt", revision.clone()));
    let first = assembly.add_component(source, Placement::IDENTITY);
    assembly.add_component(source, Placement::translation(DVec3::new(22.0, 0.0, 0.0)));
    let id = DocumentId::new();
    let copies = [RefCopy::part(part, revision, &mesh, "test")];
    let file = out_dir().join("appearance-assembly.dwasm");
    std::fs::write(&file, assembly.to_bytes(id, None, &copies).unwrap()).unwrap();

    let adapter = std::cell::RefCell::new(None);
    let mut app = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| {
            *adapter.borrow_mut() = cc.wgpu_render_state.clone();
            SolidDraftApp::new(
                cc,
                soliddraft_app::document::Document::new("Part", soliddraft_app::kernel()),
                Automation::default(),
            )
        });
    let path = OccurrencePath::root().child(Step::component(first));
    let open = |app: &mut Harness<'_, SolidDraftApp>| {
        app.state_mut().state.push(AppAction::Assembly(AssemblyAction::Open(file.clone())));
        let start = std::time::Instant::now();
        loop {
            app.run_steps(1);
            let assembly = &app.state().state.assembly;
            if assembly.opening.is_none()
                && assembly.session.as_ref().is_some_and(|s| s.doc.id() == id && !s.doc.is_loading())
            {
                break;
            }
            assert!(start.elapsed().as_secs() < 20, "the saved assembly opened");
        }
        for action in [AppAction::SetView(StandardView::Top), AppAction::Assembly(AssemblyAction::ZoomFit)] {
            app.state_mut().state.push(action);
        }
        app.run_steps(70);
    };
    let sample = |app: &mut Harness<'_, SolidDraftApp>, note: &str| {
        let view = &app.state().state.assembly.session.as_ref().unwrap().viewport;
        let points = [5.0, 27.0].map(|x| view.camera.project(glam::Vec3::new(x, 5.0, 10.0), view.rect).unwrap());
        let image = app.render().unwrap();
        image.save(out_dir().join(format!("appearance-assembly-{note}.png"))).unwrap();
        points.map(|point| colour(&image, point))
    };

    open(&mut app);
    let pixels = sample(&mut app, "part-finish");
    let mut draws = 0;
    if let Some(adapter) = &*adapter.borrow() {
        let renderer = adapter.renderer.read();
        if let Some(viewport) = renderer.callback_resources.get::<soliddraft_app::viewport::render::ViewportRenderer>()
        {
            draws = viewport.assembly_stats().draws;
        }
    }
    assert!(
        pixels.iter().all(|c| c[2] > c[0] + 30.0),
        "expected two blue components, got {pixels:?} from {draws} draws"
    );
    let red = Appearance::new("override-red", "Red", Srgb::new(210, 33, 22));
    app.state_mut().state.push(AppAction::Assembly(AssemblyAction::SetColour(path.clone(), Some(red))));
    app.run_steps(10);
    let [a, b] = sample(&mut app, "component-override");
    assert!(a[0] > a[2] + 30.0 && b[2] > b[0] + 30.0);

    app.state_mut().state.push(AppAction::Assembly(AssemblyAction::Save(Some(file.clone()))));
    app.run_steps(10);
    let reopened = soliddraft_assembly::AssemblyDocument::open(&file, Default::default()).unwrap();
    let row = reopened.table().find(&path).unwrap();
    assert_eq!(reopened.table().colour(row).unwrap().id.as_str(), "override-red");
    open(&mut app);
    app.state_mut().state.push(AppAction::Assembly(AssemblyAction::SetColour(path, None)));
    app.run_steps(10);
    assert!(sample(&mut app, "colour-removed").iter().all(|c| c[2] > c[0] + 30.0));
}
