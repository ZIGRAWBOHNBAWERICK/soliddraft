//! Editing a part in context, as a user does it in the window: Edit part in context from
//! the assembly tree opens the plate where the assembly places it, the base dimmed around
//! it; a click on the base's top face picks it, a sketch drawn there and a peg extruded
//! from it follow the base; Save writes the part and the assembly; Exit returns to the
//! assembly. Moving the base and editing again updates the reference as one undo step;
//! the part reopened alone keeps it, takes a later feature, and a deleted base marks it
//! broken without moving the peg. Frames go to `target/edit-in-context/`; the test skips
//! itself when no GPU adapter is available.

use egui::{PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use glam::DVec3;
use soliddraft_app::app::{Automation, SolidDraftApp};
use soliddraft_app::assembly::{AssemblyAction, AssemblySession};
use soliddraft_app::document::{Command, ExtrudeParams, PlaneKind, ProfileKind};
use soliddraft_app::file;
use soliddraft_app::state::{AppAction, AppState};
use soliddraft_assembly::{OccurrencePath, Placement};
use soliddraft_model::sketch::{EntityKind, P2};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const HANG: Duration = Duration::from_secs(600);

type App<'a> = Harness<'a, SolidDraftApp>;

fn gpu_available() -> bool {
    let _ = env_logger::try_init();
    let instance = egui_wgpu::wgpu::Instance::default();
    !pollster::block_on(instance.enumerate_adapters(egui_wgpu::wgpu::Backends::all())).is_empty()
}

fn out_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/edit-in-context");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn save_frame(harness: &mut App<'_>, name: &str) {
    let image = harness.render().expect("render");
    let path = out_dir().join(format!("{name}.png"));
    image.save(&path).expect("save png");
    eprintln!("wrote {}", path.display());
}

/// A block `w` by `d` on the top plane, `h` tall, centred on the origin, saved at `path`.
fn part(path: &Path, w: f64, d: f64, h: f64) -> PathBuf {
    let mut doc = soliddraft_app::new_document();
    let params = ExtrudeParams {
        profile: ProfileKind::Rectangle { width: w, height: d },
        plane: PlaneKind::Top,
        depth: h,
        ..Default::default()
    };
    doc.apply(Command::AddExtrude { params, via_assistant: false }).unwrap();
    file::save(&mut doc, path).unwrap();
    path.to_path_buf()
}

fn session(state: &AppState) -> &AssemblySession {
    state.assembly.session.as_ref().expect("an open assembly")
}

fn path_named(state: &AppState, prefix: &str) -> OccurrencePath {
    let s = session(state);
    let row = s.components().into_iter().find(|r| s.doc.label(*r).starts_with(prefix)).expect("the component");
    s.doc.table().path(row)
}

/// The open part's body box along z, once built.
fn heights(state: &AppState) -> Option<(f64, f64)> {
    let m = state.doc.model();
    let b = m.kernel().topology(m.body()?).ok()?.bbox;
    Some((b.min.z, b.max.z))
}

/// Steps until `done` holds.
fn until(harness: &mut App<'_>, what: &str, done: impl Fn(&AppState) -> bool) {
    let started = Instant::now();
    while !done(&harness.state().state) {
        if started.elapsed() > HANG {
            let st = &harness.state().state;
            let statuses: Vec<String> =
                st.doc.model().statuses().iter().map(|(id, s)| format!("{id}: {s:?}")).collect();
            panic!(
                "waited too long for {what}: body {:?}, features {statuses:?}, note {:?}",
                heights(st),
                st.note.as_ref().map(|n| &n.text)
            );
        }
        harness.run_steps(2);
    }
    harness.run_steps(4);
}

/// The kernel pads the box of a body with curved faces by 0.01 mm; the sketch plane is
/// checked exactly through the reference.
const BOX_PAD: f64 = 0.011;

fn near(h: Option<(f64, f64)>, want: (f64, f64)) -> bool {
    h.is_some_and(|(lo, hi)| (lo - want.0).abs() <= BOX_PAD && (hi - want.1).abs() <= BOX_PAD)
}

/// The z of the in-context reference's plane in the open part.
fn reference_z(state: &AppState) -> Option<f64> {
    soliddraft_model::context::references(state.doc.model().features()).first().map(|(_, c)| c.origin[2])
}

/// Every accessible node's label and value: a `ui.label`'s text is its value.
fn texts(harness: &App<'_>) -> Vec<String> {
    harness
        .query_all(egui_kittest::kittest::by())
        .flat_map(|n| [n.accesskit_node().label(), n.accesskit_node().value()])
        .flatten()
        .collect()
}

fn right_click(harness: &mut App<'_>, label: &str) {
    let at = harness.get_by_label(label).rect().center();
    harness.hover_at(at);
    harness.run_steps(2);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(2);
    }
}

/// A primary click at `at`, after the pick readback for that pixel came back.
fn click_at(harness: &mut App<'_>, at: egui::Pos2) {
    harness.hover_at(at);
    harness.run_steps(12);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.run_steps(3);
    }
}

#[test]
fn a_part_edited_in_its_assembly_takes_and_follows_a_face_of_another_component() {
    if !gpu_available() {
        eprintln!("skipping: no GPU adapter");
        return;
    }
    let dir = std::env::temp_dir().join(format!("soliddraft-edit-in-context-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("parts")).unwrap();
    file::set_config_dir(Some(dir.join("config")));
    let base = part(&dir.join("parts/base.dwprt"), 100.0, 100.0, 10.0);
    let plate = part(&dir.join("parts/plate.dwprt"), 40.0, 40.0, 5.0);
    let rig = dir.join("rig.dwasm");
    let mut harness: App<'static> = Harness::builder()
        .with_size(vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .with_render_every_step(true)
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| SolidDraftApp::new(cc, soliddraft_app::demo_document(), Automation::default()));
    harness.run_steps(3);

    // An assembly of the base and, 30 mm up, the plate; saved.
    harness.state_mut().state.run_command(soliddraft_app::commands::get("new_assembly").unwrap());
    harness.run_steps(3);
    let parts = vec![base.clone(), plate.clone()];
    harness.state_mut().state.push(AppAction::Assembly(AssemblyAction::Insert(Some(parts))));
    until(&mut harness, "the parts to load", |st| {
        st.assembly.session.as_ref().is_some_and(|s| s.components().len() == 2 && !s.doc.is_loading())
    });
    let plate_path = path_named(&harness.state().state, "plate");
    let base_path = path_named(&harness.state().state, "base");
    let up = Placement::translation(DVec3::new(0.0, 0.0, 30.0));
    harness.state_mut().state.push(AppAction::Assembly(AssemblyAction::Place(plate_path.clone(), up)));
    harness.run_steps(6);
    harness.state_mut().state.push(AppAction::Assembly(AssemblyAction::Save(Some(rig.clone()))));
    harness.run_steps(4);
    assert!(rig.is_file());
    // The same files serve the browser scenario.
    let fixture = out_dir().join("fixture");
    std::fs::create_dir_all(&fixture).unwrap();
    for f in [&base, &plate, &rig] {
        std::fs::copy(f, fixture.join(f.file_name().unwrap())).unwrap();
    }
    save_frame(&mut harness, "1-assembly");

    // Edit part in context from the tree.
    let plate_label = harness.state().state.assembly.session.as_ref().map(|s| {
        let r = s.doc.table().find(&plate_path).unwrap();
        s.doc.label(r)
    });
    let plate_label = plate_label.unwrap();
    right_click(&mut harness, &plate_label);
    harness.get_by_label("Edit part in context").click();
    until(&mut harness, "the dimmed assembly", |st| {
        soliddraft_app::in_context::shown(st)["context"].as_u64().is_some_and(|n| n > 0)
    });
    {
        let st = &harness.state().state;
        assert!(!st.assembly.is_active(), "the part's own tab is in front");
        assert_eq!(st.doc.path.as_deref(), Some(plate.as_path()));
        assert_eq!(st.assembly.in_context.as_ref().map(|c| c.path.clone()), Some(plate_path.clone()));
    }
    let shown = texts(&harness);
    assert!(shown.iter().any(|t| t.starts_with("Editing plate<")), "{shown:?}");
    assert!(harness.query_by_label("Exit in context").is_some());
    assert!(harness.query_by_label("Solid Bodies (1)").is_some(), "the tree shows the part's features");
    assert!(harness.query_by_label("Mates (0)").is_none());
    save_frame(&mut harness, "2-editing-in-context");

    // Pick the base's top face where the plate does not cover it: the corner nearest the eye.
    let (rect, camera) = {
        let st = &harness.state().state;
        (st.viewport.rect, st.viewport.camera.clone())
    };
    let eye = camera.eye();
    let corner = [(-42.0, -42.0), (42.0, -42.0), (-42.0, 42.0), (42.0, 42.0)]
        .map(|(x, y)| glam::Vec3::new(x, y, -20.0))
        .into_iter()
        .min_by(|a, b| a.distance(eye).total_cmp(&b.distance(eye)))
        .unwrap();
    let at = camera.project(corner, rect).expect("the corner is in view");
    click_at(&mut harness, at);
    let picked = harness.state().state.assembly.in_context.as_ref().and_then(|c| c.picked.clone());
    let picked = picked.expect("the click picked a face of the base");
    assert!(picked.component.starts_with("base<"), "{picked:?}");
    save_frame(&mut harness, "3-picked-base-face");

    // Sketch on it: a circle, then a peg 25 mm up through the plate.
    harness.state_mut().state.push(AppAction::StartSketch);
    until(&mut harness, "the sketch", |st| st.sketch.is_some());
    // The circle goes in as the sketcher puts geometry in: an edit of the open sketch.
    let (sketch, mut drawn) = {
        let s = harness.state().state.sketch.as_ref().unwrap();
        (s.feature, s.sketch.clone())
    };
    drawn.add(EntityKind::Circle { center: P2::ZERO, radius: 5.0 }, false);
    harness.state_mut().state.push(AppAction::Command(Command::EditSketch { id: sketch, sketch: drawn }));
    harness.run_steps(4);
    harness.state_mut().state.push(AppAction::ExitSketch);
    harness.run_steps(6);
    let params = ExtrudeParams { sketch: Some(sketch), depth: 25.0, ..Default::default() };
    harness.state_mut().state.push(AppAction::Command(Command::AddExtrude { params, via_assistant: false }));
    until(&mut harness, "the peg", |st| near(heights(st), (-20.0, 5.0)));
    let reference = {
        let st = &harness.state().state;
        let refs = soliddraft_model::context::references(st.doc.model().features());
        assert_eq!(refs.len(), 1);
        refs[0].1.clone()
    };
    assert_eq!(
        (reference.face.as_str(), reference.component.as_str()),
        (picked.face.as_str(), base_path.to_string().as_str())
    );
    assert_eq!(reference.own, plate_path.to_string());
    assert_eq!(reference.origin[2], -20.0);
    let marked = |st: &AppState| {
        st.doc.features.iter().filter_map(|f| soliddraft_app::in_context::tree_note(&f.kind)).collect::<Vec<_>>()
    };
    let notes = marked(&harness.state().state);
    assert!(notes.len() == 1 && notes[0].starts_with("in context: base<"), "{notes:?}");
    save_frame(&mut harness, "4-peg");

    // Undo and redo the peg in the part.
    harness.state_mut().state.push(AppAction::Undo);
    until(&mut harness, "the undo", |st| near(heights(st), (0.0, 5.0)));
    harness.state_mut().state.push(AppAction::Redo);
    until(&mut harness, "the redo", |st| near(heights(st), (-20.0, 5.0)));

    // Save writes the part and the assembly.
    harness.state_mut().state.push(AppAction::Save);
    harness.run_steps(8);
    {
        let st = &harness.state().state;
        assert!(!st.doc.dirty);
        assert!(!session(st).doc.is_dirty(), "the assembly is saved too");
        let note = st.note.as_ref().map(|n| n.text.clone()).unwrap_or_default();
        assert!(note.contains("plate.dwprt") && note.contains("rig.dwasm"), "{note}");
    }
    let saved = soliddraft_model::Document::open(
        &plate,
        std::sync::Arc::new(soliddraft_kernel_catallax::CatallaxKernel::new()),
    )
    .unwrap();
    assert_eq!(soliddraft_model::context::references(saved.features()).len(), 1);

    // Exit returns to the assembly.
    harness.get_by_label("Exit in context").click();
    harness.run_steps(8);
    {
        let st = &harness.state().state;
        assert!(st.assembly.is_active());
        assert!(st.assembly.in_context.is_none());
    }
    assert!(harness.query_by_label("Components (2)").is_some());
    save_frame(&mut harness, "5-back-in-assembly");

    // Move the base up 5 mm, then edit the plate again through the ribbon's Edit part.
    let lift = Placement::translation(DVec3::new(0.0, 0.0, 5.0));
    harness.state_mut().state.push(AppAction::Assembly(AssemblyAction::Place(base_path.clone(), lift)));
    harness.run_steps(6);
    harness.state_mut().state.push(AppAction::Assembly(AssemblyAction::Select(Some(plate_path.clone()))));
    harness.run_steps(2);
    harness.state_mut().state.run_command(soliddraft_app::commands::get("assembly_edit_part").unwrap());
    // The open plate followed the base as soon as it moved; Edit part shows it so.
    until(&mut harness, "the reference to follow the base", |st| near(heights(st), (-15.0, 10.0)));
    assert_eq!(reference_z(&harness.state().state), Some(-15.0));
    assert_eq!(harness.state().state.doc.undo_label().as_deref(), Some("Update in-context references"));
    save_frame(&mut harness, "6-followed-the-base");
    harness.state_mut().state.push(AppAction::Undo);
    until(&mut harness, "the update to undo", |st| near(heights(st), (-20.0, 5.0)));
    assert_eq!(reference_z(&harness.state().state), Some(-20.0));
    harness.state_mut().state.push(AppAction::Redo);
    until(&mut harness, "the update to redo", |st| near(heights(st), (-15.0, 10.0)));
    harness.state_mut().state.push(AppAction::Save);
    harness.run_steps(8);
    harness.get_by_label("Exit in context").click();
    harness.run_steps(8);

    // The plate opened alone keeps the reference and takes a later feature.
    harness.state_mut().state.push(AppAction::Open(Some(plate.clone())));
    until(&mut harness, "the plate", |st| st.doc.path.as_deref() == Some(plate.as_path()) && heights(st).is_some());
    harness.state_mut().state.push(AppAction::Assembly(AssemblyAction::ShowPart));
    harness.run_steps(6);
    assert!(near(heights(&harness.state().state), (-15.0, 10.0)));
    assert!(harness.state().state.assembly.in_context.is_none());
    assert!(harness.query_by_label("Exit in context").is_none(), "a part on its own shows no assembly");
    let later = ExtrudeParams {
        profile: ProfileKind::Rectangle { width: 10.0, height: 10.0 },
        plane: PlaneKind::Top,
        depth: 12.0,
        ..Default::default()
    };
    harness.state_mut().state.push(AppAction::Command(Command::AddExtrude { params: later, via_assistant: false }));
    until(&mut harness, "the later feature", |st| near(heights(st), (-15.0, 12.0)));
    save_frame(&mut harness, "7-later-feature");

    // Deleting the base breaks the reference; the peg stays where it was.
    {
        let st = &mut harness.state_mut().state;
        let s = st.assembly.session.as_mut().unwrap();
        let placed_at = |id| OccurrencePath::root().child(soliddraft_assembly::Step::component(id));
        let base_id = s.doc.assembly().components.iter().map(|c| c.id).find(|id| placed_at(*id) == base_path).unwrap();
        s.doc.delete(&[base_id]).unwrap();
        st.assembly.context_check = true;
    }
    until(&mut harness, "the broken reference", |st| {
        soliddraft_model::context::references(st.doc.model().features()).iter().any(|(_, c)| c.broken.is_some())
    });
    until(&mut harness, "the broken note", |st| {
        st.note.as_ref().is_some_and(|n| n.text.starts_with("In-context reference broken"))
    });
    assert!(near(heights(&harness.state().state), (-15.0, 12.0)));
    assert_eq!(reference_z(&harness.state().state), Some(-15.0), "a broken reference keeps its plane");
    let notes = marked(&harness.state().state);
    assert!(notes.len() == 1 && notes[0].starts_with("in-context reference broken"), "{notes:?}");
    save_frame(&mut harness, "8-broken-reference");
    drop(harness);
    let _ = std::fs::remove_dir_all(dir);
}
