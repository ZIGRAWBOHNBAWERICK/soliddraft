//! Merge 2 of the ribbon layout: task groups and truthful tabs. Acceptance cases R1, R2
//! and R3 of the ribbon test design, on the real app, the real wgpu pipeline and the real
//! kernel, with the gestures a user makes.
//!
//! Requires a renderer: a run without one fails.
//!
//! Run them one at a time, away from the other GPU targets:
//! `cargo test -p soliddraft-app --test ribbon_workflow -- --test-threads=1`.

#[path = "support/ribbon.rs"]
mod ribbon;

use egui::{Key, Modifiers, Rect, pos2};
use egui_kittest::kittest::Queryable;
use ribbon::{App, TOLERANCE};
use soliddraft_app::document::{Document, FeatureKind, Selection};
use soliddraft_app::state::{EditorParams, Tool};
use soliddraft_app::viewport::camera::StandardView;

const TABS: [&str; 5] = ["Solid", "Sketch", "Assembly", "Inspect", "View"];

const SOLID: [(&str, &[&str]); 6] = [
    ("Sketch", &["Sketch"]),
    ("Create", &["Extrude", "Revolve", "Sweep", "Loft"]),
    ("Remove", &["Cut", "Hole", "Slot"]),
    ("Modify", &["Fillet", "Chamfer", "Shell", "Draft", "Lattice"]),
    ("Pattern", &["Linear", "Circular", "Mirror"]),
    ("Reference", &["Plane", "Axis", "Point"]),
];

const SKETCH: [(&str, &[&str]); 8] = [
    ("Sketch", &["Sketch", "Exit"]),
    ("Draw", &["Line", "Circle", "Rectangle", "Arc", "Slot", "Point"]),
    ("Modify", &["Polygon", "Trim", "Extend", "Offset", "Mirror entities", "Linear pattern", "Circular pattern"]),
    ("Transform", &["Move entities", "Copy entities", "Rotate entities", "Scale entities"]),
    ("Constrain", &["Smart dim", "Construction", "Relations"]),
    ("Display", &["Contour shading", "Sketch dimensions", "Snap to grid"]),
    ("Use sketch", &["Extrude", "Revolve", "Cut"]),
    ("View", &["Normal to"]),
];

/// What needs a body, and what it says without one.
const NEEDS_A_BODY: [(&str, &str); 9] = [
    ("Fillet", "Fillet needs a body"),
    ("Chamfer", "Chamfer needs a body"),
    ("Hole", "Hole needs a body"),
    ("Shell", "Shell needs a body"),
    ("Draft", "Draft needs a body"),
    ("Lattice", "Lattice needs a body"),
    ("Linear", "Pattern needs a body"),
    ("Circular", "Pattern needs a body"),
    ("Mirror", "Mirror needs a body"),
];

/// What has no action yet, whatever the part holds, and what it says.
const NOT_YET: [(&str, &str); 3] = [("Slot", "Coming soon"), ("Axis", "Coming soon"), ("Point", "Coming soon")];

fn settle(app: &mut App) {
    app.run_steps(30);
}

fn viewport(app: &App) -> Rect {
    app.state().state.viewport.rect
}

fn same_rect(a: Rect, b: Rect) -> bool {
    (a.min - b.min).length() <= TOLERANCE && (a.max - b.max).length() <= TOLERANCE
}

fn selected_tabs(app: &App) -> Vec<&'static str> {
    TABS.into_iter().filter(|name| ribbon::tab(app, name).selected).collect()
}

/// A left-column region, for the panels' own buttons.
fn left_column(app: &App) -> Rect {
    let view = viewport(app);
    Rect::from_min_max(pos2(0.0, view.top()), pos2(view.left(), view.bottom()))
}

/// The properties sit below the named tree, left of the viewport, and nothing is right of it.
fn properties_below(app: &App, tree: &str) {
    let view = viewport(app);
    let tree = ribbon::title_rect(app, tree);
    let properties = ribbon::title_rect(app, "Properties");
    assert!(tree.right() <= view.left() + TOLERANCE, "the tree is left of the viewport: {tree:?} {view:?}");
    assert!(properties.right() <= view.left() + TOLERANCE, "so are the properties: {properties:?} {view:?}");
    assert!(properties.top() > tree.bottom(), "the properties are below the tree: {properties:?} {tree:?}");
    assert!(view.right() >= ribbon::window(app).right() - TOLERANCE, "nothing is right of the viewport: {view:?}");
}

/// Changing tabs preserves the part, selection and any open editor.
fn tabs_preserve_the_part(app: &mut App) {
    let before = ribbon::fingerprint(app);
    let captions = ribbon::caption_names(app);
    let window = ribbon::window(app);
    let tab = ribbon::tab(app, "Solid").rect;
    let strip = Rect::from_min_max(pos2(window.left(), tab.top()), pos2(window.right(), tab.bottom()));
    let painted = ribbon::painted_in(app, strip);
    for name in ["Surface", "Point cloud"] {
        let label = format!("{name} tab");
        assert!(app.query_by_label(label.as_str()).is_none(), "{name} is absent from the controls");
        assert!(!painted.iter().any(|p| p.text == name), "{name} is not painted");
    }
    for name in TABS {
        assert!(ribbon::tab(app, name).enabled, "{name} is enabled");
        ribbon::click_tab(app, name);
        assert_eq!(selected_tabs(app), [name], "clicking {name} selects only that tab");
        assert_eq!(ribbon::fingerprint(app), before, "browsing {name} changes no part or editor state");
    }
    ribbon::click_tab(app, "Solid");
    assert_eq!(selected_tabs(app), ["Solid"]);
    assert_eq!(ribbon::fingerprint(app), before, "returning to Solid changes no part or editor state");
    assert_eq!(ribbon::caption_names(app), captions, "returning to Solid restores its ribbon");
}

/// The volume of the shown body from its tessellation. A box's triangles sum to it exactly,
/// and this leaves the kernel's mass cache for the Mass props button to fill.
fn mesh_volume(doc: &Document) -> f64 {
    let body = doc.display_body().expect("a body to measure");
    let mesh = &doc.meshes[&body];
    let point = |i: u32| mesh.positions[i as usize].as_dvec3();
    mesh.indices.as_chunks::<3>().0.iter().map(|t| point(t[0]).dot(point(t[1]).cross(point(t[2]))) / 6.0).sum()
}

fn plate_is_whole(doc: &Document) {
    assert_eq!(doc.features.len(), 2, "one sketch and one extrude");
    assert!(doc.features[0].is_sketch());
    assert!(matches!(doc.features[1].kind, FeatureKind::Extrude(_)));
    assert_eq!(doc.error_count(), 0, "no rebuild errors");
    let volume = mesh_volume(doc);
    assert!((volume - 48000.0).abs() <= 1e-6, "volume {volume}");
}

fn settled_camera_is(app: &App, view: StandardView) {
    let camera = &app.state().state.viewport.camera;
    let (yaw, pitch) = view.angles();
    assert!((camera.yaw - yaw.to_radians()).abs() < 1e-3, "{view:?} yaw {}", camera.yaw);
    assert!((camera.pitch - pitch.to_radians()).abs() < 1e-3, "{view:?} pitch {}", camera.pitch);
}

#[test]
fn solid_groups_and_tabs_are_visible_and_honest() {
    let case = ribbon::case("r1");
    let mut blank = ribbon::app(ribbon::blank());
    case.run(&mut blank, |app| {
        settle(app);

        // One tab per name, Solid open, and its six groups.
        for name in TABS {
            let tab = ribbon::tab(app, name);
            let shown: Vec<_> = ribbon::painted_in(app, tab.rect).into_iter().filter(|p| p.text == name).collect();
            assert_eq!(shown.len(), 1, "the tab {name} is painted once: {shown:?}");
        }
        for gone in ["Features tab", "Evaluate tab"] {
            assert!(app.query_by_label(gone).is_none(), "{gone} is gone");
        }
        assert_eq!(selected_tabs(app), ["Solid"]);
        case.frame(app, "solid-blank");
        ribbon::groups_in_order(app, &SOLID);
        let captions = ribbon::caption_names(app);
        for old in ["Boss", "Cut", "Finish", "Edit", "Drawing", "Assistant"] {
            assert!(!captions.iter().any(|c| c == old), "the group {old} is gone: {captions:?}");
        }

        // What a blank part offers, and why the rest is off.
        for label in ["Extrude", "Sketch"] {
            assert!(ribbon::ribbon_button(app, label).enabled, "{label} is enabled on a blank part");
        }
        for (label, says) in NEEDS_A_BODY.iter().chain(&NOT_YET) {
            let button = ribbon::ribbon_button(app, label);
            assert!(!button.enabled, "{label} is disabled on a blank part");
            let said = ribbon::tooltip(app, button.rect);
            assert_eq!(said, *says, "{label}");
        }
        for (label, says) in
            [("Revolve", "sketch"), ("Sweep", "polygon sketch"), ("Cut", "Cut needs a body and a sketch")]
        {
            let button = ribbon::ribbon_button(app, label);
            assert!(!button.enabled, "{label} is disabled without its sketch");
            let said = ribbon::tooltip(app, button.rect);
            assert!(said.contains(says), "{label} says {says}: {said:?}");
        }

        // Browse the available tabs without running their commands.
        tabs_preserve_the_part(app);
    });

    let mut plate = ribbon::app(ribbon::plate());
    case.run(&mut plate, |app| {
        app.key_press(Key::Z);
        settle(app);

        // With a face selected, then with a Fillet collecting.
        ribbon::click_world(app, glam::Vec3::new(0.0, 0.0, 20.0));
        assert!(matches!(app.state().state.doc.selection, Some(Selection::Face(_))), "the pick selects the top face");
        tabs_preserve_the_part(app);
        ribbon::click_ribbon(app, "Fillet");
        settle(app);
        let edge = app.state().state.doc.edges.iter().find(|e| (e.midpoint.z - 20.0).abs() < 1e-9).unwrap().midpoint;
        ribbon::click_world(app, edge);
        settle(app);
        let collected = matches!(
            app.state().state.editor.as_ref().map(|e| &e.params),
            Some(EditorParams::Fillet(f)) if !f.edges.is_empty()
        );
        assert!(collected, "the Fillet collects the picked edge");
        tabs_preserve_the_part(app);
        let cancel = ribbon::button_rect(app, "Cancel (Esc)", left_column(app));
        ribbon::click_at(app, cancel.center());
        settle(app);
        assert!(app.state().state.editor.is_none(), "Cancel closes the editor");

        // What the body switches on, and what stays off.
        for (label, _) in NEEDS_A_BODY {
            assert!(ribbon::ribbon_button(app, label).enabled, "{label} is enabled on a plate");
        }
        let before = ribbon::fingerprint(app);
        for (label, says) in NOT_YET {
            let button = ribbon::ribbon_button(app, label);
            assert!(!button.enabled, "{label} stays disabled on a plate");
            let said = ribbon::tooltip(app, button.rect);
            assert!(said.contains(says), "{label} says {says}: {said:?}");
            ribbon::click_at(app, button.rect.center());
            app.run_steps(6);
            assert_eq!(ribbon::fingerprint(app), before, "clicking {label} changes nothing");
        }
    });
}

#[test]
fn a_sketch_extrudes_through_the_new_categories_and_undoes() {
    let case = ribbon::case("r2");
    let mut app = ribbon::app_at(ribbon::sketch(), ribbon::EXPANDED_WINDOW, 1.0);
    case.run(&mut app, |app| {
        settle(app);
        let (sketch, name) = {
            let feature = &app.state().state.doc.features[0];
            (feature.id, feature.name.clone())
        };

        // Into the sketch from the tree.
        let row = ribbon::button_rect(app, &name, left_column(app));
        ribbon::double_click_at(app, row.center());
        settle(app);
        assert!(app.state().state.sketch.is_some(), "the double click opens the sketch");
        assert_eq!(selected_tabs(app), ["Sketch"]);
        case.frame(app, "sketch-session");
        ribbon::groups_in_order(app, &SKETCH);
        ribbon::click_ribbon(app, "Rectangle");
        assert_eq!(app.state().state.sketch.as_ref().map(|s| s.tool), Some(Tool::Rectangle));
        ribbon::click_ribbon(app, "Line");
        assert_eq!(app.state().state.sketch.as_ref().map(|s| s.tool), Some(Tool::Line));

        // Extrude from Use sketch ends the sketch and opens the editor in Solid.
        let before = viewport(app);
        let extrude = ribbon::reveal_ribbon_command(app, "extrude");
        assert!(extrude.enabled, "Extrude is available for the sketch profile");
        ribbon::click_at(app, extrude.rect.center());
        settle(app);
        let state = &app.state().state;
        assert!(state.sketch.is_none(), "the sketch session ended");
        let extruding = matches!(
            state.editor.as_ref().map(|e| &e.params),
            Some(EditorParams::Extrude(p)) if p.sketch == Some(sketch)
        );
        assert!(extruding, "an Extrude of the fixture's sketch is open");
        assert_eq!(selected_tabs(app), ["Solid"]);
        let tree = ribbon::title_rect(app, "Tree");
        let ok = ribbon::button_rect(app, "OK (Enter)", left_column(app));
        assert!(ok.top() > tree.bottom(), "the editor's controls are below the tree: {ok:?} {tree:?}");
        assert!(same_rect(viewport(app), before), "the editor leaves the viewport: {:?} {before:?}", viewport(app));
        properties_below(app, "Tree");

        // Depth and OK.
        ribbon::type_field(app, "Depth value", "20");
        let ok = ribbon::button_rect(app, "OK (Enter)", left_column(app));
        ribbon::click_at(app, ok.center());
        ribbon::until(app, "the extrude", |a| {
            let state = &a.state().state;
            state.editor.is_none() && state.doc.features.len() == 2 && !state.doc.proof_pending()
        });
        settle(app);
        let state = &app.state().state;
        plate_is_whole(&state.doc);
        assert!(state.sketch.is_none() && state.editor.is_none());
        assert_eq!(state.commands.recent.first().map(String::as_str), Some("extrude"));

        // Undo and redo, by key and by the quick-access buttons.
        let extrude = app.state().state.doc.features[1].id;
        app.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        settle(app);
        let doc = &app.state().state.doc;
        assert_eq!(doc.features.len(), 1, "only the sketch remains");
        assert!(doc.display_body().is_none(), "no solid is shown");
        app.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        ribbon::until(app, "the redo", |a| {
            let doc = &a.state().state.doc;
            doc.features.len() == 2 && doc.display_body().is_some() && !doc.proof_pending()
        });
        settle(app);
        plate_is_whole(&app.state().state.doc);
        ribbon::click_quick_access(app, "Undo");
        settle(app);
        assert_eq!(app.state().state.doc.features.len(), 1, "quick-access Undo reaches the same stack");
        ribbon::click_quick_access(app, "Redo");
        ribbon::until(app, "the redo", |a| {
            let doc = &a.state().state.doc;
            doc.features.len() == 2 && doc.display_body().is_some() && !doc.proof_pending()
        });
        settle(app);
        plate_is_whole(&app.state().state.doc);

        // The view commands.
        ribbon::click_tab(app, "View");
        ribbon::click_ribbon(app, "Top");
        settle(app);
        settled_camera_is(app, StandardView::Top);
        ribbon::click_ribbon(app, "Isometric");
        settle(app);
        settled_camera_is(app, StandardView::Isometric);

        // Mass props of the selected extrude.
        let row = ribbon::button_rect(app, &app.state().state.doc.features[1].name.clone(), left_column(app));
        ribbon::click_at(app, row.center());
        settle(app);
        assert_eq!(app.state().state.doc.selection, Some(Selection::Feature(extrude)));
        ribbon::click_tab(app, "Inspect");
        case.frame(app, "inspect");
        let body = app.state().state.doc.display_body().unwrap();
        assert!(app.state().state.doc.kernel().cached_mass_properties(body).is_none(), "nothing has measured it yet");
        ribbon::click_ribbon(app, "Mass props");
        ribbon::until(app, "the mass properties", |a| {
            a.state().state.doc.kernel().cached_mass_properties(body).is_some()
        });
        settle(app);
        let state = &app.state().state;
        assert_eq!(state.doc.selection, Some(Selection::Feature(extrude)));
        let volume = state.doc.kernel().cached_mass_properties(body).unwrap().volume;
        assert!((volume - 48000.0).abs() <= 1e-6, "volume {volume}");
        assert!(app.query_by_label("48000 mm³").is_some(), "the property panel shows it");
    });
}

#[test]
fn assembly_is_discoverable_without_applying_part_commands_to_it() {
    let case = ribbon::case("r3");
    let mut app = ribbon::app(ribbon::blank());
    case.run(&mut app, |app| {
        settle(app);
        let start = ribbon::fingerprint(app);

        // The entry shows the commands and creates nothing.
        ribbon::click_tab(app, "Assembly");
        settle(app);
        assert_eq!(ribbon::fingerprint(app), start, "the entry changes nothing");
        assert_eq!(selected_tabs(app), ["Assembly"]);
        assert!(app.state().state.assembly.session.is_none(), "no assembly appears by itself");
        case.frame(app, "assembly-on-part");
        assert!(ribbon::caption_names(app).iter().any(|c| c == "Component"));
        let insert = ribbon::ribbon_button(app, "Insert component");
        assert!(!insert.enabled, "Insert component needs an assembly");
        let said = ribbon::tooltip(app, insert.rect);
        assert!(said.contains("assembly"), "Insert component says what it needs: {said:?}");
        ribbon::click_at(app, insert.rect.center());
        settle(app);
        assert_eq!(ribbon::fingerprint(app), start, "the disabled button does nothing");
        properties_below(app, "Tree");

        // A real assembly switches it on.
        ribbon::menu(app, "File", "New assembly");
        settle(app);
        assert!(app.state().state.assembly.is_active(), "the assembly is the active document");
        assert!(ribbon::status_lines(app).iter().any(|l| l == "0 components"));
        settle(app);
        assert!(ribbon::ribbon_button(app, "Insert component").enabled, "Insert component is on in an assembly");
        for label in ["Save assembly", "Fit"] {
            assert!(ribbon::ribbon_button(app, label).enabled, "{label} is reachable");
        }
        properties_below(app, "Assembly tree");

        // Back to the part, as it was.
        ribbon::document_tab(app, "Ribbon acceptance");
        settle(app);
        assert!(!app.state().state.assembly.is_active());
        ribbon::click_tab(app, "Solid");
        settle(app);
        assert_eq!(selected_tabs(app), ["Solid"]);
        assert_eq!(
            ribbon::fingerprint(app).without_history(),
            start.clone().without_history(),
            "the part is untouched"
        );
        for (label, _) in NEEDS_A_BODY {
            assert!(!ribbon::ribbon_button(app, label).enabled, "{label} is off without a body");
        }
        properties_below(app, "Tree");
    });
}

#[test]
fn reference_plane_editor_creates_a_plane_and_attaches_a_sketch() {
    let case = ribbon::case("reference-plane");
    let mut app = ribbon::app(ribbon::blank());
    case.run(&mut app, |app| {
        settle(app);
        assert!(ribbon::ribbon_button(app, "Plane").enabled);
        ribbon::click_ribbon(app, "Plane");
        settle(app);
        assert!(matches!(app.state().state.editor.as_ref().map(|e| &e.params), Some(EditorParams::ReferencePlane(_))));
        ribbon::type_field(app, "Offset value", "-6");
        let ok = ribbon::button_rect(app, "OK (Enter)", left_column(app));
        ribbon::click_at(app, ok.center());
        ribbon::until(app, "the reference plane", |a| {
            a.state().state.editor.is_none() && a.state().state.doc.features.len() == 1
        });
        settle(app);
        let plane = app.state().state.doc.features[0].clone();
        assert_eq!(plane.name, "Plane1");
        assert_eq!(plane.plane.unwrap().origin.z, -6.0);
        case.frame(app, "reference-plane");
        let eye = app.get_by_label("Hide Plane1").rect();
        ribbon::click_at(app, eye.center());
        settle(app);
        assert_eq!(
            app.state().state.doc.view_choice(soliddraft_app::document::ViewItem::Feature(plane.id)),
            Some(false)
        );
        let eye = app.get_by_label("Show Plane1").rect();
        ribbon::click_at(app, eye.center());
        settle(app);
        let row = ribbon::button_rect(app, "Plane1", left_column(app));
        ribbon::click_at(app, row.center());
        ribbon::click_ribbon(app, "Sketch");
        ribbon::until(app, "the attached sketch", |a| a.state().state.sketch.is_some());
        let session = app.state().state.sketch.as_ref().unwrap();
        assert_eq!(session.plane.origin.z, -6.0);
        case.frame(app, "sketch-on-plane");
    });
}

#[test]
fn reference_plane_on_a_translated_part_is_visible() {
    use soliddraft_app::document::{Command, ExtrudeParams, PlaneKind, SketchPlane};
    let case = ribbon::case("reference-plane-translated");
    let mut doc = ribbon::blank();
    let sketch = doc
        .apply(Command::AddSketch { plane: SketchPlane::Reference { plane: PlaneKind::Top, offset: 0.0 } })
        .unwrap()
        .unwrap();
    let mut drawing = soliddraft_sketch::Sketch::new();
    drawing.rectangle(soliddraft_sketch::P2::new(1005.0, 1005.0), 10.0, 10.0).unwrap();
    doc.apply(Command::EditSketch { id: sketch, sketch: drawing }).unwrap();
    doc.apply(Command::AddExtrude {
        params: ExtrudeParams { sketch: Some(sketch), depth: 5.0, ..Default::default() },
        via_assistant: false,
    })
    .unwrap();
    let mut app = ribbon::app(doc);
    case.run(&mut app, |app| {
        settle(app);
        app.key_press(Key::Z);
        settle(app);
        ribbon::click_ribbon(app, "Plane");
        settle(app);
        ribbon::type_field(app, "Offset value", "-6");
        let ok = ribbon::button_rect(app, "OK (Enter)", left_column(app));
        ribbon::click_at(app, ok.center());
        ribbon::until(app, "the translated part's plane", |a| {
            a.state().state.editor.is_none() && a.state().state.doc.features.len() == 3
        });
        settle(app);
        case.frame(app, "translated-reference-plane");
        assert!(
            ribbon::painted_in(app, viewport(app)).iter().any(|p| p.text == "Plane1"),
            "the shown plane is labelled in the viewport"
        );
    });
}
